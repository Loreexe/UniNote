//! Adversarial Stress & Empirical Challenge Suite for Milestone 1 (Clipboard Engine)
//!
//! Stress dimensions tested:
//! 1. Cross-page and cross-format pasting:
//!    - Test 1A: Copying from Notebook A4 page 0 and pasting into page 3 with explicit deselect.
//!    - Test 1B: Copying from Notebook A4 page 0 and pasting into page 3 without deselect (viewport scrolled).
//!    - Test 1C: Copying from Notebook A4 and pasting into Whiteboard infinite negative coordinates.
//!    - Test 1D: Copying from Notebook A4 and pasting into Whiteboard infinite positive coordinates.
//!    - Test 1E: Copying from Whiteboard and pasting into Notebook A4 (centering on page).
//! 2. Auto-activation of selection post-paste:
//!    - Test 2A: Verification of st.selection state immediately after paste (has_selection, selected_ids, bbox, tool).
//!    - Test 2B: Dragging newly pasted items and verifying item coordinate shifts & spatial index sync.
//!    - Test 2C: Undo and Redo symmetry of drag on newly pasted items.
//! 3. GdkClipboard serialization stress:
//!    - Test 3A: Extreme coordinates roundtrip (1e20, -1e20, 1e-10, 0.0, -0.0).
//!    - Test 3B: NaN and Infinity behavior in serialize_clipboard.
//!    - Test 3C: Malformed JSON payload injection resilience in deserialize_clipboard.
//!    - Test 3D: Resilient handling of malformed/corrupted clipboard data during paste_from_clipboard.

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::clipboard::{
    clear_clipboard, deserialize_clipboard, serialize_clipboard, set_clipboard_data, ClipboardData,
};
use uninote_app::hud::DiagnosticHud;
use uninote_core::command::DocCommand;
use uninote_core::coordinates::Viewport;
use uninote_core::model::{Document, DocumentFormat, Item, ItemKind, Page, PageBackground};
use uninote_stroke::types::{Rect, Stroke, StrokePoint, StrokeStyle};

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
            .name("gtk-clip-adv-test-worker".into())
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

fn create_test_canvas() -> NoteCanvas {
    let hud = Rc::new(DiagnosticHud::new());
    NoteCanvas::new(hud)
}

// ============================================================================
// STRESS TEST 1: CROSS-PAGE AND CROSS-FORMAT PASTING
// ============================================================================

/// Test 1A: Copia da Pagina 0 e incolla su Pagina 3 dopo deselezione esplicita.
#[test]
fn test_1a_cross_page_paste_0_to_3_with_deselect() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Crea 4 pagine A4 (0, 1, 2, 3)
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            for i in 1..4 {
                st.document
                    .pages
                    .push(Page::new_a4(i, PageBackground::Grid { grid_size: 14.17 }));
            }
            assert_eq!(st.document.pages.len(), 4);
        }

        let stroke = create_test_stroke(100, 100.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p0 = &mut st.document.pages[0];
            let item = Item {
                id: 100,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            p0.spatial_index.insert(&item);
            p0.items.push(item);
            st.next_stroke_id = 101;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![100];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(
            canvas.copy_selected(),
            "Copy selected on page 0 must succeed"
        );

        // Scorre a Pagina 3 e deseleziona
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p3_rect = Viewport::page_rect(3);
            st.viewport.offset_y = -p3_rect.y * st.viewport.zoom + 40.0;
            st.selection.clear();
        }

        assert!(
            canvas.paste_from_clipboard(),
            "Paste onto page 3 must succeed"
        );

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();

            assert_eq!(st.document.pages[0].items.len(), 1, "Page 0 has 1 item");
            assert_eq!(st.document.pages[3].items.len(), 1, "Page 3 has 1 item");
            assert_eq!(st.document.pages[3].spatial_index.len(), 1);

            let p3_item = &st.document.pages[3].items[0];
            let ItemKind::Stroke(ref s) = p3_item.kind;
            let bbox = s.bounding_box().unwrap();
            let p3_rect = Viewport::page_rect(3);

            assert!(
                bbox.min_y >= p3_rect.y - 100.0 && bbox.max_y <= p3_rect.y + p3_rect.height + 100.0,
                "Pasted stroke coordinates ({:?}) must be within Page 3 vertical span [{:.1}, {:.1}]",
                bbox,
                p3_rect.y,
                p3_rect.y + p3_rect.height
            );
            assert_eq!(st.selection.page_index, Some(3));
        }
    });
}

/// Test 1B: Copia da Pagina 0 e incolla su Pagina 3 SENZA deselezione (scrolling puro).
/// QUESTO TEST EVIDENZIA IL BUG ARCHITETTURALE: st.selection.page_index sovrascrive la viewport visibile!
#[test]
fn test_1b_cross_page_paste_0_to_3_without_deselect_highlights_bug() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            for i in 1..4 {
                st.document
                    .pages
                    .push(Page::new_a4(i, PageBackground::Grid { grid_size: 14.17 }));
            }
        }

        let stroke = create_test_stroke(101, 100.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p0 = &mut st.document.pages[0];
            let item = Item {
                id: 101,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            p0.spatial_index.insert(&item);
            p0.items.push(item);
            st.next_stroke_id = 102;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![101];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // L'utente naviga scorrendo con il mouse/touch (offset_y cambia) senza cliccare deseleziona
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p3_rect = Viewport::page_rect(3);
            st.viewport.offset_y = -p3_rect.y * st.viewport.zoom + 40.0;
            // st.selection NON viene cancellato: selezione attiva rimane Some(0)
        }

        assert!(canvas.paste_from_clipboard());

        let state_rc = canvas.state();
        let st = state_rc.borrow();

        // Osservazione empirica: l'elemento incollato finisce in Page 0 o Page 3?
        let p0_count = st.document.pages[0].items.len();
        let p3_count = st.document.pages[3].items.len();
        let sel_page = st.selection.page_index;

        eprintln!(
            "EMPIRICAL OBSERVATION (Test 1B): p0_count = {}, p3_count = {}, sel_page = {:?}",
            p0_count, p3_count, sel_page
        );

        // L'elemento incollato deve finire sulla pagina visibile (Page 3), non su Page 0
        assert_eq!(
            p0_count, 1,
            "Page 0 must retain exactly its original 1 stroke"
        );
        assert_eq!(p3_count, 1, "Page 3 must receive the newly pasted stroke");
        assert_eq!(
            sel_page,
            Some(3),
            "Active selection must be updated to target Page 3"
        );
    });
}

/// Test 1C: Copia da Notebook A4 e incolla su Lavagna Infinita a coordinate negative estreme.
#[test]
fn test_1c_notebook_to_whiteboard_negative_infinite_coordinates() {
    run_on_gtk_thread(|| {
        clear_clipboard();

        // 1. Copia da Notebook A4
        let nb_canvas = create_test_canvas();
        let stroke_nb = create_test_stroke(110, 200.0, 300.0);
        let nb_bbox = stroke_nb.bounding_box().unwrap();

        {
            let state_rc = nb_canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 110,
                z_index: 1,
                kind: ItemKind::Stroke(stroke_nb),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![110];
            st.selection.bbox = Some(nb_bbox);
        }
        assert!(nb_canvas.copy_selected());

        // 2. Apri Whiteboard e posiziona il viewport su coordinate fortemente negative
        let wb_canvas = create_test_canvas();
        {
            let state_rc = wb_canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
            // Imposta viewport per inquadrare centro attorno a (-5000.0, -3000.0)
            st.viewport.offset_x = 5400.0;
            st.viewport.offset_y = 3300.0;
            st.viewport.zoom = 1.0;
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

            let item = &st.document.pages[0].items[0];
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().expect("Bbox must exist");

            // Verifica che il bounding box sia posizionato nelle coordinate negative
            assert!(
                bbox.min_x < -4000.0 && bbox.max_x < -4000.0,
                "Stroke X coords must be deeply negative, got {:?}",
                bbox
            );
            assert!(
                bbox.min_y < -2000.0 && bbox.max_y < -2000.0,
                "Stroke Y coords must be deeply negative, got {:?}",
                bbox
            );

            // Verifica che l'R-Tree indicizzi correttamente l'elemento a coordinate negative
            let located = st.document.pages[0].spatial_index.locate_in_envelope(
                bbox.min_x - 10.0,
                bbox.min_y - 10.0,
                bbox.max_x + 10.0,
                bbox.max_y + 10.0,
            );
            assert_eq!(
                located.len(),
                1,
                "Spatial index must locate item in negative coordinates"
            );
            assert_eq!(located[0].item_id, item.id);
        }
    });
}

/// Test 1D: Copia da Notebook A4 e incolla su Lavagna Infinita a coordinate positive estreme (+10000, +20000).
#[test]
fn test_1d_notebook_to_whiteboard_positive_infinite_coordinates() {
    run_on_gtk_thread(|| {
        clear_clipboard();

        let nb_canvas = create_test_canvas();
        let stroke_nb = create_test_stroke(120, 150.0, 200.0);
        let nb_bbox = stroke_nb.bounding_box().unwrap();

        {
            let state_rc = nb_canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 120,
                z_index: 1,
                kind: ItemKind::Stroke(stroke_nb),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![120];
            st.selection.bbox = Some(nb_bbox);
        }
        assert!(nb_canvas.copy_selected());

        let wb_canvas = create_test_canvas();
        {
            let state_rc = wb_canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
            // Viewport a coordinate fortemente positive
            st.viewport.offset_x = -9600.0;
            st.viewport.offset_y = -19700.0;
            st.viewport.zoom = 1.0;
        }

        assert!(wb_canvas.paste_from_clipboard());

        {
            let state_rc = wb_canvas.state();
            let st = state_rc.borrow();
            let item = &st.document.pages[0].items[0];
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().expect("Bbox must exist");

            assert!(
                bbox.min_x > 9000.0 && bbox.max_x > 9000.0,
                "Stroke X coords must be deeply positive, got {:?}",
                bbox
            );
            assert!(
                bbox.min_y > 19000.0 && bbox.max_y > 19000.0,
                "Stroke Y coords must be deeply positive, got {:?}",
                bbox
            );

            let located = st.document.pages[0].spatial_index.locate_in_envelope(
                bbox.min_x - 10.0,
                bbox.min_y - 10.0,
                bbox.max_x + 10.0,
                bbox.max_y + 10.0,
            );
            assert_eq!(located.len(), 1);
            assert_eq!(located[0].item_id, item.id);
        }
    });
}

/// Test 1E: Copia da Whiteboard con coordinate lontane e incolla su Notebook A4 (centratura su foglio).
#[test]
fn test_1e_whiteboard_to_notebook_centering() {
    run_on_gtk_thread(|| {
        clear_clipboard();

        // 1. Whiteboard con elemento a (50000.0, -80000.0)
        let wb_canvas = create_test_canvas();
        let stroke_wb = create_test_stroke(130, 50000.0, -80000.0);
        let wb_bbox = stroke_wb.bounding_box().unwrap();

        {
            let state_rc = wb_canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 130,
                z_index: 1,
                kind: ItemKind::Stroke(stroke_wb),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![130];
            st.selection.bbox = Some(wb_bbox);
        }
        assert!(wb_canvas.copy_selected());

        // 2. Notebook A4 centrato normalmente
        let nb_canvas = create_test_canvas();
        nb_canvas.center_page_1();

        assert!(
            nb_canvas.paste_from_clipboard(),
            "Paste into Notebook must succeed"
        );

        {
            let state_rc = nb_canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.format, DocumentFormat::Notebook);
            assert_eq!(st.document.pages[0].items.len(), 1);

            let item = &st.document.pages[0].items[0];
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().expect("Bbox must exist");

            // Verifica che l'elemento incollato sia atterrato all'interno del foglio A4 (larghezza ~595pt, altezza ~842pt)
            assert!(
                bbox.min_x >= 0.0 && bbox.max_x <= 595.28 + 200.0,
                "Pasted stroke X should be within visible A4 viewport horizontal range, got {:?}",
                bbox
            );
            assert!(
                bbox.min_y >= 0.0 && bbox.max_y <= 841.89 + 200.0,
                "Pasted stroke Y should be within visible A4 viewport vertical range, got {:?}",
                bbox
            );

            // Verifica auto-selezione
            assert!(st.selection.has_selection());
            assert_eq!(st.selection.page_index, Some(0));
            assert_eq!(st.selection.selected_ids, vec![item.id]);
        }
    });
}

// ============================================================================
// STRESS TEST 2: AUTO-ACTIVATION POST-PASTE & SELECTION DRAGGING
// ============================================================================

/// Test 2A: Verifica immediata auto-attivazione e conformità SelectionState dopo paste.
#[test]
fn test_2a_auto_activation_immediate_state_verification() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke1 = create_test_stroke(200, 50.0, 60.0);
        let stroke2 = create_test_stroke(201, 80.0, 100.0);
        let union_bbox = stroke1
            .bounding_box()
            .unwrap()
            .union(&stroke2.bounding_box().unwrap());

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let it1 = Item {
                id: 200,
                z_index: 1,
                kind: ItemKind::Stroke(stroke1),
            };
            let it2 = Item {
                id: 201,
                z_index: 2,
                kind: ItemKind::Stroke(stroke2),
            };
            page.spatial_index.insert(&it1);
            page.spatial_index.insert(&it2);
            page.items.push(it1);
            page.items.push(it2);
            st.next_stroke_id = 202;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![200, 201];
            st.selection.bbox = Some(union_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        let state_rc = canvas.state();
        let st = state_rc.borrow();

        // Verifiche mandatory:
        assert!(
            st.selection.has_selection(),
            "Selection must be active immediately after paste"
        );
        assert_eq!(
            st.selection.selected_ids.len(),
            2,
            "Both pasted items must be selected"
        );
        assert_eq!(
            st.active_tool,
            ActiveTool::Select,
            "Active tool must be Select"
        );
        assert!(
            !st.selection.is_dragging,
            "is_dragging must be false initially"
        );
        assert_eq!(
            st.selection.drag_offset,
            (0.0, 0.0),
            "drag_offset must be zero"
        );

        let p_items = &st.document.pages[0].items;
        assert_eq!(p_items.len(), 4, "Total items must be 4");

        let new_id1 = p_items[2].id;
        let new_id2 = p_items[3].id;
        assert_eq!(st.selection.selected_ids, vec![new_id1, new_id2]);

        let sel_bbox = st.selection.bbox.expect("Selection bbox must be Some");
        assert!(
            (sel_bbox.min_x - (union_bbox.min_x + 25.0)).abs() < 1e-3,
            "Selection bbox must match +25pt translated bounding box"
        );
    });
}

/// Test 2B: Trascina la selezione appena incollata e verifica la precisione geometrica e sincronizzazione spaziale.
#[test]
fn test_2b_drag_newly_pasted_selection_accuracy_and_spatial_sync() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(210, 100.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let it = Item {
                id: 210,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&it);
            page.items.push(it);
            st.next_stroke_id = 211;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![210];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        // 1. Simula Mouse Down / Stylus Down sulla selezione incollata
        let (pasted_id, pasted_bbox_before_drag) = {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let id = st.selection.selected_ids[0];
            let bbox = st.selection.bbox.unwrap();

            // Hit test al centro del bounding box della selezione incollata
            let center = (
                (bbox.min_x + bbox.max_x) / 2.0,
                (bbox.min_y + bbox.max_y) / 2.0,
            );
            assert!(
                st.selection.hit_test_handle_or_bbox(center, 12.0),
                "Hit test on pasted selection must succeed"
            );

            st.selection.is_dragging = true;
            st.selection.drag_start_doc = Some(center);
            st.selection.drag_offset = (0.0, 0.0);
            (id, bbox)
        };

        // 2. Simula Mouse Motion / Stylus Motion: sposta di (+60.0, -40.0)
        let drag_delta = (60.0, -40.0);
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            assert!(st.selection.is_dragging);
            st.selection.drag_offset = drag_delta;
        }

        // 3. Simula Mouse Up / Stylus Up: applica DocCommand::MoveItems
        {
            let state_rc = canvas.state();
            let mut st_guard = state_rc.borrow_mut();
            let st = &mut *st_guard;
            let (dx, dy) = st.selection.drag_offset;
            st.selection.is_dragging = false;
            st.selection.drag_start_doc = None;
            st.selection.drag_offset = (0.0, 0.0);

            let page_id = st.document.pages[0].id;
            let item_ids = st.selection.selected_ids.clone();
            let cmd = DocCommand::MoveItems {
                page_id,
                item_ids,
                delta_x: dx,
                delta_y: dy,
            };
            st.history.execute(cmd, &mut st.document);
            if let Some(bbox) = st.selection.bbox.as_mut() {
                *bbox = bbox.translate(dx, dy);
            }
        }

        // 4. Verifica precisione coordinate e sincronizzazione R-Tree
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let item = page
                .items
                .iter()
                .find(|it| it.id == pasted_id)
                .expect("Pasted item must exist");
            let ItemKind::Stroke(ref s) = item.kind;
            let cur_bbox = s.bounding_box().expect("Current bbox must exist");

            let expected_bbox = pasted_bbox_before_drag.translate(drag_delta.0, drag_delta.1);
            assert!(
                (cur_bbox.min_x - expected_bbox.min_x).abs() < 1e-3,
                "Stroke X translation mismatch: got {}, expected {}",
                cur_bbox.min_x,
                expected_bbox.min_x
            );
            assert!(
                (cur_bbox.min_y - expected_bbox.min_y).abs() < 1e-3,
                "Stroke Y translation mismatch: got {}, expected {}",
                cur_bbox.min_y,
                expected_bbox.min_y
            );

            // Verifica R-Tree: rintracciabile nella nuova posizione, NON nella vecchia
            let old_located = page.spatial_index.locate_in_envelope(
                pasted_bbox_before_drag.min_x,
                pasted_bbox_before_drag.min_y,
                pasted_bbox_before_drag.max_x,
                pasted_bbox_before_drag.max_y,
            );
            assert!(
                !old_located.iter().any(|si| si.item_id == pasted_id),
                "Pasted item MUST NOT be indexed at old location in R-Tree"
            );

            let new_located = page.spatial_index.locate_in_envelope(
                cur_bbox.min_x - 1.0,
                cur_bbox.min_y - 1.0,
                cur_bbox.max_x + 1.0,
                cur_bbox.max_y + 1.0,
            );
            assert!(
                new_located.iter().any(|si| si.item_id == pasted_id),
                "Pasted item MUST be indexed at new location in R-Tree"
            );
        }
    });
}

/// Test 2C: Undo e Redo dello spostamento (drag) sugli elementi incollati.
#[test]
fn test_2c_undo_redo_of_drag_on_pasted_items() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(220, 120.0, 140.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let it = Item {
                id: 220,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&it);
            page.items.push(it);
            st.next_stroke_id = 221;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![220];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        let pasted_id = {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            st.selection.selected_ids[0]
        };

        // Applica spostamento DocCommand::MoveItems di (+100.0, +100.0)
        {
            let state_rc = canvas.state();
            let mut st_guard = state_rc.borrow_mut();
            let st = &mut *st_guard;
            let page_id = st.document.pages[0].id;
            let cmd = DocCommand::MoveItems {
                page_id,
                item_ids: vec![pasted_id],
                delta_x: 100.0,
                delta_y: 100.0,
            };
            st.history.execute(cmd, &mut st.document);
        }

        // UNDO spostamento -> ritorna alle coordinate post-paste (+25pt da originale)
        assert!(canvas.undo(), "Undo drag must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == pasted_id)
                .unwrap();
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().unwrap();
            assert!((bbox.min_x - (orig_bbox.min_x + 25.0)).abs() < 1e-3);
            assert!((bbox.min_y - (orig_bbox.min_y + 25.0)).abs() < 1e-3);
        }

        // UNDO incolla -> rimuove l'elemento incollato
        assert!(canvas.undo(), "Undo paste must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].spatial_index.len(), 1);
            assert!(!st.document.pages[0]
                .items
                .iter()
                .any(|it| it.id == pasted_id));
        }

        // REDO incolla -> ricrea l'elemento incollato
        assert!(canvas.redo(), "Redo paste must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 2);
            assert_eq!(st.document.pages[0].spatial_index.len(), 2);
            assert!(st.document.pages[0]
                .items
                .iter()
                .any(|it| it.id == pasted_id));
        }

        // REDO spostamento -> risposta a (+125pt da originale)
        assert!(canvas.redo(), "Redo drag must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == pasted_id)
                .unwrap();
            let ItemKind::Stroke(ref s) = item.kind;
            let bbox = s.bounding_box().unwrap();
            assert!((bbox.min_x - (orig_bbox.min_x + 125.0)).abs() < 1e-3);
            assert!((bbox.min_y - (orig_bbox.min_y + 125.0)).abs() < 1e-3);
        }
    });
}

// ============================================================================
// STRESS TEST 3: GDKCLIPBOARD SERIALIZATION STRESS & GUARDS
// ============================================================================

/// Test 3A: Serializzazione e deserializzazione con coordinate estreme (1e20, -1e20, 1e-10, 0.0, -0.0).
#[test]
fn test_3a_extreme_coordinates_serialization_roundtrip() {
    let extreme_coords = [1e20, -1e20, 1e-10, -1e-10, 0.0, -0.0, 1e15, -1e15, 1e-25];

    for &val in &extreme_coords {
        let stroke = create_test_stroke(300, val, val);
        let bbox = stroke.bounding_box().unwrap_or(Rect {
            min_x: val,
            min_y: val,
            max_x: val + 10.0,
            max_y: val + 10.0,
        });

        let data = ClipboardData {
            source_format: DocumentFormat::Whiteboard,
            source_doc_id: None,
            source_page_index: None,
            source_page_size: None,
            original_bbox: bbox,
            original_center: (val, val),
            items: vec![Item {
                id: 300,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            }],
            paste_count: 5,
            source_viewport_offset: Some((val / 2.0, val / 2.0)),
        };

        let json = serialize_clipboard(&data).expect("Serialization of extreme coord must succeed");
        let deserialized = deserialize_clipboard(&json).expect("Deserialization must succeed");

        assert_eq!(deserialized.source_format, DocumentFormat::Whiteboard);
        assert_eq!(deserialized.items.len(), 1);
        assert_eq!(deserialized.paste_count, 5);

        let ItemKind::Stroke(des_stroke) = &deserialized.items[0].kind;
        assert_eq!(des_stroke.points.len(), 3);
        let diff_x = (des_stroke.points[0].x - val).abs();
        let rel_x = if val.abs() > 0.0 {
            diff_x / val.abs()
        } else {
            diff_x
        };
        assert!(
            rel_x < 1e-12,
            "Extreme coord mismatch for val {val}: got {}",
            des_stroke.points[0].x
        );
    }
}

/// Test 3B: Comportamento empirico con NaN e Infinity in serialize_clipboard e deserialize_clipboard.
/// Serde_json di default serializza NaN come `null`.
/// Quando deserialize_clipboard tenta di deserializzare `null` in un f64, fallisce con Err.
#[test]
fn test_3b_nan_and_infinity_serialization_guards() {
    // 1. NaN in original_center
    let stroke = create_test_stroke(310, 10.0, 10.0);
    let bbox = stroke.bounding_box().unwrap();
    let nan_data = ClipboardData {
        source_format: DocumentFormat::Notebook,
        source_doc_id: None,
        source_page_index: Some(0),
        source_page_size: Some((595.28, 841.89)),
        original_bbox: bbox,
        original_center: (f64::NAN, 10.0),
        items: vec![Item {
            id: 310,
            z_index: 1,
            kind: ItemKind::Stroke(stroke),
        }],
        paste_count: 0,
        source_viewport_offset: None,
    };

    let serialized =
        serialize_clipboard(&nan_data).expect("serde_json serializes NaN to null in JSON");
    assert!(
        serialized.contains("null"),
        "serde_json serializes f64::NAN as null: got {serialized}"
    );

    // Quando un altro processo o istanza tenta di deserializzare il payload con null:
    let res_deser = deserialize_clipboard(&serialized);
    assert!(
        res_deser.is_err(),
        "deserialize_clipboard MUST reject JSON with null in place of f64 float: {:?}",
        res_deser
    );
}

/// Test 3E: Iniezione di NaN in-memory negli appunti e verifica vulnerabilità in paste_from_clipboard.
/// RILEVA LA MANCANZA DI NAN GUARDS IN paste_from_clipboard():
/// Se APP_CLIPBOARD contiene original_center = (NaN, 10.0), paste_from_clipboard() calcola dx = NaN,
/// trasla tutti i punti in NaN, corrompe l'AABB e inserisce il tratto corrotto nella pagina!
#[test]
fn test_3e_nan_guard_vulnerability_in_paste_from_clipboard() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let stroke = create_test_stroke(320, 100.0, 100.0);
        let bbox = stroke.bounding_box().unwrap();

        // Inietta dati con NaN in original_center negli appunti in memoria
        // con source_format: Whiteboard per forzare il branch di centratura viewport
        let nan_data = ClipboardData {
            source_format: DocumentFormat::Whiteboard,
            source_doc_id: None,
            source_page_index: None,
            source_page_size: None,
            original_bbox: bbox,
            original_center: (f64::NAN, 100.0),
            items: vec![Item {
                id: 320,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            }],
            paste_count: 0,
            source_viewport_offset: None,
        };
        set_clipboard_data(nan_data);

        // Tenta di incollare con NaN in original_center
        let pasted = canvas.paste_from_clipboard();
        eprintln!(
            "EMPIRICAL OBSERVATION (Test 3E): paste_from_clipboard with NaN returned: {pasted}"
        );

        assert!(
            !pasted,
            "paste_from_clipboard MUST return false when original_center contains NaN"
        );

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert_eq!(
            st.document.pages[0].items.len(),
            0,
            "Page 0 must not receive corrupted items on NaN rejection"
        );
    });
}

/// Test 3C: Resilienza a injection di payload JSON malevolo o malformato.
#[test]
fn test_3c_malformed_payload_injection_resilience() {
    let malformed_payloads = [
        "",
        "   ",
        "null",
        "true",
        "42",
        "\"a string\"",
        "{}",
        "{\"items\": []}",
        "{\"source_format\": \"InvalidFormat\"}",
        "{\"source_format\": \"Notebook\", \"items\": \"not an array\"}",
        "{\"items\": [{\"id\": \"not_a_number\"}]}",
        "{\"original_center\": [\"NaN\", 0]}",
        "{\"original_bbox\": null}",
        "{\"paste_count\": -10}",
        r#"{"source_format": "Notebook", "items": [{"id": 1, "z_index": 1, "kind": {"Stroke": {"points": "corrupted"}}}]}"#,
        "\0\0\0\0",
        "{\"source_format\": \"Notebook\", \"original_bbox\": {\"min_x\": 0, \"min_y\": 0, \"max_x\": 10, \"max_y\": 10}, \"original_center\": [5, 5], \"items\": []", // Tronco
    ];

    for (idx, payload) in malformed_payloads.iter().enumerate() {
        let result = deserialize_clipboard(payload);
        assert!(
            result.is_err(),
            "Payload #{} ({:?}) must be safely rejected with Err without panic",
            idx,
            payload
        );
    }
}

/// Test 3D: Resilienza di paste_from_clipboard se APP_CLIPBOARD è vuoto o contiene items vuoti.
#[test]
fn test_3d_empty_or_cleared_clipboard_safety() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // 1. Incolla con appunti vuoti
        assert!(
            !canvas.paste_from_clipboard(),
            "paste_from_clipboard on empty clipboard must return false gracefully"
        );

        // 2. Incolla con items vuoti
        let empty_items_data = ClipboardData {
            source_format: DocumentFormat::Notebook,
            source_doc_id: None,
            source_page_index: Some(0),
            source_page_size: Some((595.28, 841.89)),
            original_bbox: Rect {
                min_x: 0.0,
                min_y: 0.0,
                max_x: 0.0,
                max_y: 0.0,
            },
            original_center: (0.0, 0.0),
            items: Vec::new(),
            paste_count: 0,
            source_viewport_offset: None,
        };
        set_clipboard_data(empty_items_data);

        assert!(
            !canvas.paste_from_clipboard(),
            "paste_from_clipboard on zero-items data must return false gracefully"
        );
    });
}
