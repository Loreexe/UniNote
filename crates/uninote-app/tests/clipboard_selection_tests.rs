//! Selection Clipboard Engine Integration & Unit Tests (Milestone M1)
//!
//! Scope:
//! - Selection Copy (Ctrl+C) preserving geometry and bounding box
//! - Selection Cut (Ctrl+X) with atomic deletion via DocCommand::EraseItems
//! - Selection Paste (Ctrl+V) with smart +25pt offset on same view
//! - Multiple consecutive pastes cascading (+25pt, +50pt, +75pt)
//! - Selection Auto-Activation post-paste (dashed overlay, bbox, active tool)
//! - Symmetric Undo / Redo fidelity for DocCommand::PasteItems (Document.items and PageSpatialIndex)
//! - Cross-page paste (Notebook Page 0 to Page 1)
//! - Cross-format paste (Notebook to Whiteboard and Whiteboard to Notebook)
//! - SQLite WAL storage roundtrip without primary key collisions or spatial index loss
//! - Serialization fidelity for GdkClipboard JSON payload

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::clipboard::{
    clear_clipboard, deserialize_clipboard, serialize_clipboard, ClipboardData,
};
use uninote_app::hud::DiagnosticHud;
use uninote_core::coordinates::Viewport;
use uninote_core::model::{Document, DocumentFormat, Item, ItemId, ItemKind, Page, PageBackground};
use uninote_storage::DocumentStore;
use uninote_stroke::types::{Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// DEDICATED GTK TEST HARNESS (Ensures thread-safe GTK lifecycle across tests)
// ============================================================================

struct GtkTestHarness {
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

static GTK_HARNESS: OnceLock<GtkTestHarness> = OnceLock::new();

fn run_on_gtk_thread<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let harness = GTK_HARNESS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        thread::Builder::new()
            .name("gtk-clipboard-test-worker".into())
            .spawn(move || {
                let _ = gtk4::init();
                while let Ok(task) = rx.recv() {
                    task();
                }
            })
            .expect("Failed to spawn GTK test worker thread");
        GtkTestHarness { sender: tx }
    });

    let (res_tx, res_rx) = mpsc::channel();
    harness
        .sender
        .send(Box::new(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = res_tx.send(res);
        }))
        .expect("Failed to send task to GTK test worker");

    match res_rx.recv().expect("Failed to receive test result") {
        Ok(val) => val,
        Err(err) => std::panic::resume_unwind(err),
    }
}

/// Helper per creare uno Stroke di test con bounding box e cache Bézier precalcolati.
fn create_test_stroke(id: u64, x: f64, y: f64) -> Stroke {
    let points = vec![
        StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1000,
        },
        StrokePoint {
            x: x + 20.0,
            y: y + 20.0,
            pressure: 0.6,
            width: 2.5,
            timestamp_ns: 2000,
        },
        StrokePoint {
            x: x + 40.0,
            y: y + 10.0,
            pressure: 0.7,
            width: 2.2,
            timestamp_ns: 3000,
        },
    ];
    Stroke::with_points(id, StrokeStyle::default(), points)
}

/// Helper per costruire un NoteCanvas su quaderno vuoto in memoria.
fn create_test_canvas() -> NoteCanvas {
    let hud = Rc::new(DiagnosticHud::new());
    NoteCanvas::new(hud)
}

/// Helper per generare un percorso file temporaneo univoco.
fn unique_test_db(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "uninote_clip_{}_{}_{}.uninote",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    path.push(unique);
    path
}

// ============================================================================
// TEST 1: COPY + PASTE CON SMART OFFSET +25 PT
// ============================================================================

#[test]
fn test_copy_paste_offset_25pt() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // 1. Inserisci un tratto sulla pagina 0
        let stroke = create_test_stroke(1, 100.0, 150.0);
        let orig_bbox = stroke.bounding_box().expect("Bounding box must exist");

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 1,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 2;

            // Seleziona il tratto
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![1];
            st.selection.bbox = Some(orig_bbox);
        }

        // 2. Copia negli appunti
        assert!(canvas.copy_selected(), "copy_selected must succeed");

        // 3. Incolla dagli appunti
        assert!(
            canvas.paste_from_clipboard(),
            "paste_from_clipboard must succeed"
        );

        // 4. Verifica elementi risultanti
        let state_rc = canvas.state();
        let st = state_rc.borrow();
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 2, "Page must contain exactly 2 items");
        assert_eq!(
            page.spatial_index.len(),
            2,
            "Spatial index must contain exactly 2 items"
        );

        let orig_item = &page.items[0];
        let pasted_item = &page.items[1];
        assert_eq!(orig_item.id, 1);
        assert!(
            pasted_item.id > 1,
            "Pasted item must have a new strictly unique ItemId"
        );
        assert!(
            pasted_item.z_index > orig_item.z_index,
            "Pasted item z_index must be above original"
        );

        let ItemKind::Stroke(ref pasted_stroke) = pasted_item.kind;
        let pasted_bbox = pasted_stroke
            .bounding_box()
            .expect("Pasted bbox must exist");

        let dx = pasted_bbox.min_x - orig_bbox.min_x;
        let dy = pasted_bbox.min_y - orig_bbox.min_y;
        assert!(
            (dx - 25.0).abs() < 1e-3,
            "Pasted item X offset must be +25.0pt, got {dx}"
        );
        assert!(
            (dy - 25.0).abs() < 1e-3,
            "Pasted item Y offset must be +25.0pt, got {dy}"
        );
    });
}

// ============================================================================
// TEST 2: MULTIPLE CONSECUTIVE PASTES CASCADING (+25pt, +50pt, +75pt)
// ============================================================================

#[test]
fn test_multiple_consecutive_pastes_cascading() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(10, 80.0, 120.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 10,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 11;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![10];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // Esegui 3 incolla consecutivi
        assert!(canvas.paste_from_clipboard(), "Paste 1 must succeed");
        assert!(canvas.paste_from_clipboard(), "Paste 2 must succeed");
        assert!(canvas.paste_from_clipboard(), "Paste 3 must succeed");

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        let page = &st.document.pages[0];
        assert_eq!(
            page.items.len(),
            4,
            "Page must contain original + 3 pasted items"
        );
        assert_eq!(page.spatial_index.len(), 4);

        let expected_offsets = [25.0, 50.0, 75.0];
        for (i, &expected_delta) in expected_offsets.iter().enumerate() {
            let item = &page.items[i + 1];
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().unwrap();

            let dx = bbox.min_x - orig_bbox.min_x;
            let dy = bbox.min_y - orig_bbox.min_y;
            assert!(
                (dx - expected_delta).abs() < 1e-3,
                "Paste #{} X offset expected +{}pt, got {}",
                i + 1,
                expected_delta,
                dx
            );
            assert!(
                (dy - expected_delta).abs() < 1e-3,
                "Paste #{} Y offset expected +{}pt, got {}",
                i + 1,
                expected_delta,
                dy
            );
        }
    });
}

// ============================================================================
// TEST 3: CUT + PASTE
// ============================================================================

#[test]
fn test_cut_and_paste() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(20, 150.0, 200.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 20,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 21;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![20];
            st.selection.bbox = Some(orig_bbox);
        }

        // Taglia selezione
        assert!(canvas.cut_selected(), "cut_selected must succeed");

        // Verifica cancellazione dalla pagina
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 0, "Page must be empty after cut_selected");
            assert_eq!(page.spatial_index.len(), 0);
            assert!(
                !st.selection.has_selection(),
                "Selection must be cleared after cut"
            );
        }

        // Incolla selezione
        assert!(
            canvas.paste_from_clipboard(),
            "paste_from_clipboard after cut must succeed"
        );

        // Verifica ripristino dell'elemento incollato
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 1, "Page must contain 1 pasted item");
            assert_eq!(page.spatial_index.len(), 1);

            let pasted_item = &page.items[0];
            assert!(
                pasted_item.id > 20,
                "Pasted item must have newly allocated ID"
            );

            // Auto-selezione deve essere attiva
            assert!(st.selection.has_selection(), "Pasted item must be selected");
            assert_eq!(st.selection.selected_ids, vec![pasted_item.id]);
        }
    });
}

// ============================================================================
// TEST 4: UNDO / REDO SYMMETRY (Both in Document.items and PageSpatialIndex)
// ============================================================================

#[test]
fn test_undo_redo_symmetry() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(30, 200.0, 250.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 30,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 31;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![30];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        // Stato post-paste: 2 elementi
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 2);
            assert_eq!(st.document.pages[0].spatial_index.len(), 2);
        }

        // Esegui UNDO
        assert!(canvas.undo(), "Undo must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(
                st.document.pages[0].items.len(),
                1,
                "After undo, 1 item in items"
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                1,
                "After undo, 1 item in spatial index"
            );
            assert_eq!(st.document.pages[0].items[0].id, 30);
        }

        // Esegui REDO
        assert!(canvas.redo(), "Redo must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(
                st.document.pages[0].items.len(),
                2,
                "After redo, 2 items in items"
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                2,
                "After redo, 2 items in spatial index"
            );
        }

        // 20 cicli alternati di Undo e Redo per verificare invarianza rigorosa
        for cycle in 1..=20 {
            assert!(canvas.undo(), "Cycle {cycle} Undo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                assert_eq!(st.document.pages[0].items.len(), 1);
                assert_eq!(st.document.pages[0].spatial_index.len(), 1);
            }
            assert!(canvas.redo(), "Cycle {cycle} Redo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                assert_eq!(st.document.pages[0].items.len(), 2);
                assert_eq!(st.document.pages[0].spatial_index.len(), 2);
            }
        }
    });
}

// ============================================================================
// TEST 5: AUTO-SELECTION ON PASTE
// ============================================================================

#[test]
fn test_auto_selection_on_paste() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke1 = create_test_stroke(40, 100.0, 100.0);
        let stroke2 = create_test_stroke(41, 150.0, 150.0);
        let union_bbox = stroke1
            .bounding_box()
            .unwrap()
            .union(&stroke2.bounding_box().unwrap());

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let it1 = Item {
                id: 40,
                z_index: 1,
                kind: ItemKind::Stroke(stroke1),
            };
            let it2 = Item {
                id: 41,
                z_index: 2,
                kind: ItemKind::Stroke(stroke2),
            };
            page.spatial_index.insert(&it1);
            page.spatial_index.insert(&it2);
            page.items.push(it1);
            page.items.push(it2);
            st.next_stroke_id = 42;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![40, 41];
            st.selection.bbox = Some(union_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert!(st.selection.has_selection(), "Selection must be active");
        assert_eq!(st.selection.page_index, Some(0));
        assert_eq!(
            st.selection.selected_ids.len(),
            2,
            "Both pasted strokes must be selected"
        );
        assert!(
            !st.selection.selected_ids.contains(&40),
            "Old ID 40 must not be in selection"
        );
        assert!(
            !st.selection.selected_ids.contains(&41),
            "Old ID 41 must not be in selection"
        );
        assert_eq!(st.active_tool, ActiveTool::Select);

        let sel_bbox = st.selection.bbox.expect("Selection bbox must be Some");
        assert!(
            (sel_bbox.min_x - (union_bbox.min_x + 25.0)).abs() < 1e-3,
            "Selection bbox must match pasted translation"
        );
    });
}

// ============================================================================
// TEST 6: CROSS-PAGE PASTE (Notebook Page 0 to Page 1)
// ============================================================================

#[test]
fn test_cross_page_paste() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Aggiungi una seconda pagina A4
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document
                .pages
                .push(Page::new_a4(1, PageBackground::Grid { grid_size: 14.17 }));
        }

        let stroke = create_test_stroke(50, 100.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page0 = &mut st.document.pages[0];
            let item = Item {
                id: 50,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page0.spatial_index.insert(&item);
            page0.items.push(item);
            st.next_stroke_id = 51;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![50];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // Naviga verso Pagina 1 e azzera selezione attiva
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p1_rect = Viewport::page_rect(1);
            st.viewport.offset_y = -p1_rect.y * st.viewport.zoom + 40.0;
            st.selection.clear();
        }

        // Incolla su Pagina 1
        assert!(
            canvas.paste_from_clipboard(),
            "Cross-page paste must succeed"
        );

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert_eq!(
            st.document.pages[0].items.len(),
            1,
            "Page 0 still has 1 item"
        );
        assert_eq!(
            st.document.pages[1].items.len(),
            1,
            "Page 1 now has 1 pasted item"
        );
        assert_eq!(st.document.pages[1].spatial_index.len(), 1);

        let p1_item = &st.document.pages[1].items[0];
        let ItemKind::Stroke(ref s) = p1_item.kind;
        let p1_bbox = s.bounding_box().unwrap();

        // Le coordinate della Pagina 1 partono dopo la Pagina 0
        let p1_rect = Viewport::page_rect(1);
        assert!(
            p1_bbox.min_y >= p1_rect.y - 100.0,
            "Pasted stroke on Page 1 must have document coordinates within Page 1 bounds"
        );
        assert_eq!(st.selection.page_index, Some(1));
    });
}

// ============================================================================
// TEST 7: CROSS-FORMAT PASTE (Notebook to Whiteboard & Whiteboard to Notebook)
// ============================================================================

#[test]
fn test_cross_format_paste() {
    run_on_gtk_thread(|| {
        clear_clipboard();

        // 1. Da Notebook a Whiteboard
        let nb_canvas = create_test_canvas();
        let stroke_nb = create_test_stroke(60, 200.0, 300.0);
        let nb_bbox = stroke_nb.bounding_box().unwrap();

        {
            let state_rc = nb_canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 60,
                z_index: 1,
                kind: ItemKind::Stroke(stroke_nb),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![60];
            st.selection.bbox = Some(nb_bbox);
        }

        assert!(nb_canvas.copy_selected());

        // Crea lavagna Whiteboard e incolla
        let wb_canvas = create_test_canvas();
        {
            let state_rc = wb_canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
        }

        assert!(
            wb_canvas.paste_from_clipboard(),
            "Paste into Whiteboard must succeed"
        );

        {
            let state_rc = wb_canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.format, DocumentFormat::Whiteboard);
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].spatial_index.len(), 1);
        }

        // 2. Da Whiteboard a Notebook
        let stroke_wb = create_test_stroke(70, 3000.0, 4000.0);
        let wb_bbox = stroke_wb.bounding_box().unwrap();
        {
            let state_rc = wb_canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 70,
                z_index: 2,
                kind: ItemKind::Stroke(stroke_wb),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![70];
            st.selection.bbox = Some(wb_bbox);
        }

        assert!(wb_canvas.copy_selected());

        let nb_canvas2 = create_test_canvas();
        assert!(
            nb_canvas2.paste_from_clipboard(),
            "Paste from Whiteboard to Notebook must succeed"
        );

        {
            let state_rc = nb_canvas2.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.format, DocumentFormat::Notebook);
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].spatial_index.len(), 1);
        }
    });
}

// ============================================================================
// TEST 8: SQLITE ROUNDTRIP PRESERVATION
// ============================================================================

#[test]
fn test_sqlite_roundtrip_preservation() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(80, 110.0, 160.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 80,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 81;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![80];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());
        assert!(canvas.paste_from_clipboard());

        let db_path = unique_test_db("roundtrip");

        // Salva documento con elementi duplicati
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let mut store = DocumentStore::open_or_create(&db_path).expect("open_or_create");
            store
                .save_document(&st.document)
                .expect("save_document must succeed with pasted items");
        }

        // Riapri da disco e verifica
        let store2 = DocumentStore::open_or_create(&db_path).expect("open_or_create");
        let loaded = store2.load_document().expect("load_document");

        assert_eq!(loaded.pages[0].items.len(), 3);
        assert_eq!(loaded.pages[0].spatial_index.len(), 3);

        let ids: Vec<ItemId> = loaded.pages[0].items.iter().map(|it| it.id).collect();
        let unique_ids: std::collections::HashSet<ItemId> = ids.iter().copied().collect();
        assert_eq!(
            ids.len(),
            unique_ids.len(),
            "All saved and reloaded item IDs must be distinct"
        );

        let _ = fs::remove_file(db_path);
    });
}

// ============================================================================
// TEST 9: CLIPBOARD DATA SERIALIZATION FIDELITY
// ============================================================================

#[test]
fn test_clipboard_serialization_fidelity() {
    let stroke = create_test_stroke(90, 150.0, 250.0);
    let bbox = stroke.bounding_box().unwrap();
    let center = (
        (bbox.min_x + bbox.max_x) / 2.0,
        (bbox.min_y + bbox.max_y) / 2.0,
    );

    let original = ClipboardData {
        source_format: DocumentFormat::Notebook,
        source_doc_id: None,
        source_page_index: Some(0),
        source_page_size: Some((595.28, 841.89)),
        original_bbox: bbox,
        original_center: center,
        items: vec![Item {
            id: 90,
            z_index: 1,
            kind: ItemKind::Stroke(stroke),
        }],
        paste_count: 2,
        source_viewport_offset: Some((60.0, 40.0)),
    };

    let serialized = serialize_clipboard(&original).expect("Serialization must succeed");
    assert!(serialized.contains("\"source_format\":\"Notebook\""));
    assert!(serialized.contains("\"paste_count\":2"));

    let deserialized: ClipboardData =
        deserialize_clipboard(&serialized).expect("Deserialization must succeed");

    assert_eq!(deserialized.source_format, original.source_format);
    assert_eq!(deserialized.source_page_index, original.source_page_index);
    assert_eq!(deserialized.original_bbox, original.original_bbox);
    assert_eq!(deserialized.original_center, original.original_center);
    assert_eq!(deserialized.items.len(), original.items.len());
    assert_eq!(deserialized.items[0].id, 90);
    assert_eq!(deserialized.paste_count, 2);
}
