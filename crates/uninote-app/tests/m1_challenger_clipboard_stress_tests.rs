//! Empirical Challenger Stress Tests for Milestone 1 (Selection Clipboard Engine)
//!
//! Stress Tests:
//! 1. Multi-cycle cascading offset: 10 consecutive pastes on the same page produce cascading +25pt offsets (+25pt, +50pt, +75pt, ...).
//! 2. Undo/Redo symmetry: 100+ repeated undo/redo cycles on DocCommand::PasteItems maintain exact invariance: `page.items.len() == page.spatial_index.len()`.
//! 3. Unique ItemId allocation: verify that 1000 pasted items never collide with existing stroke IDs and save cleanly to SQLite WAL without primary key violation.
//! 4. Eraser compatibility: verify that pasted strokes can be immediately erased by eraser without residual artifacts in spatial index.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::clipboard::clear_clipboard;
use uninote_app::hud::DiagnosticHud;
use uninote_core::model::{Item, ItemKind};
use uninote_storage::DocumentStore;
use uninote_stroke::types::{Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// DEDICATED GTK TEST HARNESS
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
            .name("gtk-challenger-m1-worker".into())
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
            x: x + 30.0,
            y: y + 30.0,
            pressure: 0.6,
            width: 2.5,
            timestamp_ns: 2000,
        },
        StrokePoint {
            x: x + 60.0,
            y: y + 15.0,
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

fn unique_test_db(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "uninote_m1_challenger_{}_{}_{}.uninote",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    path.push(unique);
    path
}

// ============================================================================
// STRESS TEST 1: MULTI-CYCLE CASCADING OFFSET (10 CONSECUTIVE PASTES)
// ============================================================================

#[test]
fn test_stress_cascading_offset_10_cycles() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Inserisci un gruppo di 2 tratti originali
        let stroke1 = create_test_stroke(100, 50.0, 50.0);
        let stroke2 = create_test_stroke(101, 120.0, 80.0);
        let orig_bbox1 = stroke1.bounding_box().unwrap();
        let orig_bbox2 = stroke2.bounding_box().unwrap();
        let union_orig_bbox = orig_bbox1.union(&orig_bbox2);

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];

            let it1 = Item {
                id: 100,
                z_index: 1,
                kind: ItemKind::Stroke(stroke1),
            };
            let it2 = Item {
                id: 101,
                z_index: 2,
                kind: ItemKind::Stroke(stroke2),
            };
            page.spatial_index.insert(&it1);
            page.spatial_index.insert(&it2);
            page.items.push(it1);
            page.items.push(it2);
            st.next_stroke_id = 102;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![100, 101];
            st.selection.bbox = Some(union_orig_bbox);
        }

        assert!(canvas.copy_selected(), "Initial copy must succeed");

        // Esegui esattamente 10 paste consecutivi sulla stessa vista/pagina
        for cycle in 1..=10 {
            assert!(
                canvas.paste_from_clipboard(),
                "Paste #{cycle} failed to execute"
            );

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];

            // Numero atteso di elementi: 2 originali + cycle * 2
            let expected_count = 2 + cycle * 2;
            assert_eq!(
                page.items.len(),
                expected_count,
                "Cycle #{cycle}: items count mismatch"
            );
            assert_eq!(
                page.spatial_index.len(),
                expected_count,
                "Cycle #{cycle}: spatial index count mismatch"
            );

            // Verifica l'offset esatto dell'ultimo batch di tratti incollati
            let expected_offset = (cycle as f64) * 25.0;

            // Gli ultimi due elementi sono stati appena incollati
            let pasted_it1 = &page.items[expected_count - 2];
            let pasted_it2 = &page.items[expected_count - 1];

            let ItemKind::Stroke(ref s1) = pasted_it1.kind;
            let ItemKind::Stroke(ref s2) = pasted_it2.kind;

            let b1 = s1.bounding_box().unwrap();
            let b2 = s2.bounding_box().unwrap();

            let dx1 = b1.min_x - orig_bbox1.min_x;
            let dy1 = b1.min_y - orig_bbox1.min_y;
            assert!(
                (dx1 - expected_offset).abs() < 1e-3,
                "Cycle #{cycle} stroke1 X offset expected {expected_offset}, got {dx1}"
            );
            assert!(
                (dy1 - expected_offset).abs() < 1e-3,
                "Cycle #{cycle} stroke1 Y offset expected {expected_offset}, got {dy1}"
            );

            let dx2 = b2.min_x - orig_bbox2.min_x;
            let dy2 = b2.min_y - orig_bbox2.min_y;
            assert!(
                (dx2 - expected_offset).abs() < 1e-3,
                "Cycle #{cycle} stroke2 X offset expected {expected_offset}, got {dx2}"
            );
            assert!(
                (dy2 - expected_offset).abs() < 1e-3,
                "Cycle #{cycle} stroke2 Y offset expected {expected_offset}, got {dy2}"
            );

            // Verifica che la selezione attiva contenga ESATTAMENTE i 2 nuovi ID
            assert_eq!(
                st.selection.selected_ids,
                vec![pasted_it1.id, pasted_it2.id],
                "Cycle #{cycle} selection IDs must match newly pasted items"
            );
            assert_eq!(
                st.active_tool,
                ActiveTool::Select,
                "Cycle #{cycle} active tool must be Select"
            );
        }
    });
}

// ============================================================================
// STRESS TEST 2: UNDO / REDO SYMMETRY (120+ REPEATED CYCLES INVARIANCE)
// ============================================================================

#[test]
fn test_stress_undo_redo_symmetry_120_cycles() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Crea 3 tratti
        let s1 = create_test_stroke(201, 100.0, 100.0);
        let s2 = create_test_stroke(202, 150.0, 150.0);
        let s3 = create_test_stroke(203, 200.0, 200.0);
        let bbox = s1
            .bounding_box()
            .unwrap()
            .union(&s2.bounding_box().unwrap())
            .union(&s3.bounding_box().unwrap());

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];

            let it1 = Item {
                id: 201,
                z_index: 1,
                kind: ItemKind::Stroke(s1),
            };
            let it2 = Item {
                id: 202,
                z_index: 2,
                kind: ItemKind::Stroke(s2),
            };
            let it3 = Item {
                id: 203,
                z_index: 3,
                kind: ItemKind::Stroke(s3),
            };
            page.spatial_index.insert(&it1);
            page.spatial_index.insert(&it2);
            page.spatial_index.insert(&it3);
            page.items.push(it1);
            page.items.push(it2);
            page.items.push(it3);
            st.next_stroke_id = 204;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![201, 202, 203];
            st.selection.bbox = Some(bbox);
        }

        assert!(canvas.copy_selected());
        assert!(canvas.paste_from_clipboard());

        // Stato post-paste: 6 elementi sia in items che in spatial_index
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 6);
            assert_eq!(st.document.pages[0].spatial_index.len(), 6);
        }

        // 125 cicli completi di Undo seguito da Redo
        for cycle in 1..=125 {
            // Esegui UNDO
            assert!(canvas.undo(), "Cycle #{cycle}: Undo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                let page = &st.document.pages[0];
                assert_eq!(
                    page.items.len(),
                    3,
                    "Cycle #{cycle} (after undo): items.len must be 3"
                );
                assert_eq!(
                    page.spatial_index.len(),
                    3,
                    "Cycle #{cycle} (after undo): spatial_index.len must be 3"
                );
                assert_eq!(
                    page.items.len(),
                    page.spatial_index.len(),
                    "Cycle #{cycle} (after undo): exact invariance violated"
                );

                // Verifica che i 3 elementi rimasti siano esattamente gli originali
                let ids: Vec<u64> = page.items.iter().map(|it| it.id).collect();
                assert_eq!(ids, vec![201, 202, 203]);
            }

            // Esegui REDO
            assert!(canvas.redo(), "Cycle #{cycle}: Redo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                let page = &st.document.pages[0];
                assert_eq!(
                    page.items.len(),
                    6,
                    "Cycle #{cycle} (after redo): items.len must be 6"
                );
                assert_eq!(
                    page.spatial_index.len(),
                    6,
                    "Cycle #{cycle} (after redo): spatial_index.len must be 6"
                );
                assert_eq!(
                    page.items.len(),
                    page.spatial_index.len(),
                    "Cycle #{cycle} (after redo): exact invariance violated"
                );

                // Verifica ordinamento z_index
                for w in page.items.windows(2) {
                    assert!(
                        w[0].z_index < w[1].z_index,
                        "Cycle #{cycle} (after redo): z_index strictly ascending"
                    );
                }
            }
        }
    });
}

// ============================================================================
// STRESS TEST 3: 1000 PASTED ITEMS ALLOCATION & SQLITE WAL PERSISTENCE
// ============================================================================

#[test]
fn test_stress_1000_pasted_items_unique_ids_and_sqlite_wal() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // 1. Inizializza la pagina con 10 tratti base (ID da 1 a 10)
        let orig_count: usize = 10;
        let mut initial_ids = Vec::new();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];

            for i in 1..=orig_count {
                let id = i as u64;
                let stroke = create_test_stroke(id, 50.0 + (i as f64) * 10.0, 50.0);
                let item = Item {
                    id,
                    z_index: id,
                    kind: ItemKind::Stroke(stroke),
                };
                page.spatial_index.insert(&item);
                page.items.push(item);
                initial_ids.push(id);
            }
            let union_b = page
                .items
                .iter()
                .filter_map(|it| match &it.kind {
                    ItemKind::Stroke(s) => s.bounding_box(),
                })
                .fold(
                    None,
                    |acc: Option<uninote_stroke::types::Rect>, b| match acc {
                        Some(prev) => Some(prev.union(&b)),
                        None => Some(b),
                    },
                );

            st.next_stroke_id = (orig_count as u64) + 1;
            st.selection.page_index = Some(0);
            st.selection.selected_ids = initial_ids.clone();
            st.selection.bbox = union_b;
        }

        assert!(canvas.copy_selected(), "Copy of 10 items must succeed");

        // 2. Esegui 100 paste di 10 elementi ciascuno = 1000 nuovi elementi incollati!
        let paste_iterations: usize = 100;
        for paste_num in 1..=paste_iterations {
            let ok = canvas.paste_from_clipboard();
            assert!(ok, "Paste iteration #{paste_num} must succeed");
        }

        let total_expected: usize = orig_count + paste_iterations * 10; // 10 + 1000 = 1010
        assert_eq!(total_expected, 1010);

        // 3. Verifica unicità assoluta di tutti i 1010 ItemId
        let state_rc = canvas.state();
        let st = state_rc.borrow();
        let page = &st.document.pages[0];

        assert_eq!(page.items.len(), total_expected);
        assert_eq!(page.spatial_index.len(), total_expected);

        let mut seen_ids = HashSet::with_capacity(total_expected);
        let mut max_id: u64 = 0;
        for item in &page.items {
            assert!(
                seen_ids.insert(item.id),
                "FATAL: Duplicate ItemId detected: {}",
                item.id
            );
            max_id = max_id.max(item.id);
        }
        assert_eq!(
            seen_ids.len(),
            total_expected,
            "All 1010 item IDs must be strictly unique"
        );
        assert!(
            max_id >= total_expected as u64,
            "Max ID must be >= {total_expected}"
        );

        // 4. Salva in un file SQLite reale in modalità WAL
        let db_path = unique_test_db("1000_pasted");
        {
            let mut store = DocumentStore::open_or_create(&db_path)
                .expect("Failed to open or create SQLite DocumentStore");
            store
                .save_document(&st.document)
                .expect("Failed to save 1010 items to SQLite WAL: Primary key collision?");
        }

        // 5. Riapri da disco e verifica integrità completa del caricamento
        {
            let store = DocumentStore::open_or_create(&db_path)
                .expect("Failed to reopen SQLite DocumentStore");
            let loaded_doc = store
                .load_document()
                .expect("Failed to load document with 1010 items from SQLite WAL");

            let loaded_page = &loaded_doc.pages[0];
            assert_eq!(
                loaded_page.items.len(),
                total_expected,
                "Loaded items count must be exactly 1010"
            );
            assert_eq!(
                loaded_page.spatial_index.len(),
                total_expected,
                "Loaded spatial index count must be exactly 1010"
            );

            // Verifica che tutti i 1010 ID siano preservati
            let loaded_ids: HashSet<u64> = loaded_page.items.iter().map(|it| it.id).collect();
            assert_eq!(
                loaded_ids, seen_ids,
                "Loaded item IDs must match original set exactly"
            );
        }

        // Cleanup del file temporaneo
        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(format!("{}-wal", db_path.display()));
        let _ = fs::remove_file(format!("{}-shm", db_path.display()));
    });
}

// ============================================================================
// STRESS TEST 4: ERASER COMPATIBILITY ON PASTED STROKES
// ============================================================================

#[test]
fn test_stress_eraser_compatibility_on_pasted_strokes() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // 1. Crea un tratto sorgente a (100.0, 100.0)
        let stroke = create_test_stroke(300, 100.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];

            let item = Item {
                id: 300,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 301;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![300];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // 2. Incolla il tratto -> ottiene offset (+25.0, +25.0), quindi si trova a (125.0, 125.0)
        assert!(canvas.paste_from_clipboard());

        let pasted_id = {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 2);
            assert_eq!(page.spatial_index.len(), 2);
            page.items[1].id
        };
        assert_ne!(pasted_id, 300);

        // 3. Cancella il tratto incollato usando l'eraser sul punto (185.0, 140.0)
        // Il punto 2 del tratto incollato è a (160.0 + 25.0, 115.0 + 25.0) = (185.0, 140.0).
        // Il punto più vicino del tratto originale è a (160.0, 115.0) a distanza > 35pt,
        // garantendo isolamento geometrico perfetto con raggio 5.0pt.
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            NoteCanvas::erase_strokes_at(&mut st, (185.0, 140.0), 5.0);
        }

        // 4. Verifica che il tratto incollato sia stato rimosso IMMEDIATAMENTE
        // e che NESSUN artefatto rimanga nell'indice spaziale
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];

            assert_eq!(
                page.items.len(),
                1,
                "Page must have only 1 item remaining after erase"
            );
            assert_eq!(
                page.spatial_index.len(),
                1,
                "Spatial index must have only 1 item remaining after erase"
            );
            assert_eq!(
                page.items[0].id, 300,
                "Original stroke must remain untouched"
            );

            // Query spaziale sull'area esclusiva del tratto incollato (x > 165, y > 135): deve restituire 0 risultati
            let residual_exclusive = page
                .spatial_index
                .locate_in_envelope(165.0, 135.0, 195.0, 155.0);
            assert!(
                residual_exclusive.is_empty(),
                "No residual spatial items must exist in exclusive pasted area: found {:?}",
                residual_exclusive
            );

            // Verifica che il pasted_id non sia presente in NESSUNA query spaziale dell'intera pagina
            let all_spatial = page
                .spatial_index
                .locate_in_envelope(-1000.0, -1000.0, 10000.0, 10000.0);
            assert_eq!(all_spatial.len(), 1);
            assert_eq!(all_spatial[0].item_id, 300);

            // Query spaziale sull'inviluppo del tratto originale: deve restituire il tratto 300
            let orig_res = page
                .spatial_index
                .locate_in_envelope(95.0, 95.0, 165.0, 165.0);
            assert_eq!(
                orig_res.len(),
                1,
                "Original stroke must still be located in spatial index"
            );
            assert_eq!(orig_res[0].item_id, 300);
        }

        // 5. Test simmetria: UNDO della cancellazione ripristina il tratto incollato!
        assert!(canvas.undo(), "Undo erase must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(
                page.items.len(),
                2,
                "After undo of erase, items must be 2 again"
            );
            assert_eq!(
                page.spatial_index.len(),
                2,
                "After undo of erase, spatial_index must be 2 again"
            );

            // Il tratto incollato deve essere rintracciabile nell'indice spaziale
            let restored = page
                .spatial_index
                .locate_in_envelope(165.0, 135.0, 195.0, 155.0);
            assert_eq!(
                restored.len(),
                1,
                "Restored stroke must be in spatial index"
            );
            assert_eq!(restored[0].item_id, pasted_id);
        }

        // 6. Test simmetria: REDO della cancellazione rimuove di nuovo il tratto
        assert!(canvas.redo(), "Redo erase must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 1);
            assert_eq!(page.spatial_index.len(), 1);
            let residual = page
                .spatial_index
                .locate_in_envelope(165.0, 135.0, 195.0, 155.0);
            assert!(
                residual.is_empty(),
                "Redo erase leaves zero artifacts in exclusive area"
            );

            let all_spatial = page
                .spatial_index
                .locate_in_envelope(-1000.0, -1000.0, 10000.0, 10000.0);
            assert_eq!(all_spatial.len(), 1);
            assert_eq!(all_spatial[0].item_id, 300);
        }
    });
}

// ============================================================================
// ADVERSARIAL TEST 5: CASCADE RESET ON RE-COPY & PAN CENTERING BEHAVIOR
// ============================================================================

#[test]
fn test_adversarial_cascade_reset_and_pan_centering() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        let s = create_test_stroke(400, 100.0, 100.0);
        let orig_bbox = s.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let item = Item {
                id: 400,
                z_index: 1,
                kind: ItemKind::Stroke(s),
            };
            page.spatial_index.insert(&item);
            page.items.push(item);
            st.next_stroke_id = 401;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![400];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // 3 paste consecutivi (+25, +50, +75)
        for expected_offset in [25.0, 50.0, 75.0] {
            assert!(canvas.paste_from_clipboard());
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let last_item = page.items.last().unwrap();
            let ItemKind::Stroke(ref stroke) = last_item.kind;
            let bbox = stroke.bounding_box().unwrap();
            let dx = bbox.min_x - orig_bbox.min_x;
            assert!(
                (dx - expected_offset).abs() < 1e-3,
                "Expected offset {expected_offset}, got {dx}"
            );
        }

        // Ora riseleziona e ricopia: deve resettare il contatore di offset!
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![400];
            st.selection.bbox = Some(orig_bbox);
        }
        assert!(
            canvas.copy_selected(),
            "Re-copy must succeed and reset cascade"
        );

        // Il prossimo paste deve ripartire da +25pt esatti rispetto all'originale!
        assert!(canvas.paste_from_clipboard());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let last_item = page.items.last().unwrap();
            let ItemKind::Stroke(ref stroke) = last_item.kind;
            let bbox = stroke.bounding_box().unwrap();
            let dx = bbox.min_x - orig_bbox.min_x;
            assert!(
                (dx - 25.0).abs() < 1e-3,
                "After re-copy, offset must reset to +25.0pt, got {dx}"
            );
        }

        // Ora simula un pan consistente del viewport: sposta offset_x di 400 pt
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.viewport.offset_x += 400.0;
        }

        // Incolla dopo il pan: deve centrare il tratto nella viewport visibile corrente!
        assert!(canvas.paste_from_clipboard());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let last_item = page.items.last().unwrap();
            let ItemKind::Stroke(ref stroke) = last_item.kind;
            let bbox = stroke.bounding_box().unwrap();
            // Poiché ci siamo mossi di pan, l'offset non deve essere +50.0 fisso dall'originale,
            // bensì centrato rispetto alla nuova coordinata viewport
            let dx = bbox.min_x - orig_bbox.min_x;
            assert!(
                dx.abs() > 100.0,
                "Pasting after pan must not stick to local cascade: dx={dx}"
            );
        }
    });
}

// ============================================================================
// ADVERSARIAL TEST 6: RANDOM WALK UNDO / REDO INVARIANCE
// ============================================================================

#[test]
fn test_adversarial_random_walk_undo_redo_invariance() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // 1. Inserisci 5 tratti originali
        let mut orig_ids = Vec::new();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            for i in 1..=5 {
                let id = 500 + i;
                let s = create_test_stroke(id, 100.0 + (i as f64) * 20.0, 100.0);
                let it = Item {
                    id,
                    z_index: i,
                    kind: ItemKind::Stroke(s),
                };
                page.spatial_index.insert(&it);
                page.items.push(it);
                orig_ids.push(id);
            }

            let union_b = page
                .items
                .iter()
                .filter_map(|it| match &it.kind {
                    ItemKind::Stroke(s) => s.bounding_box(),
                })
                .fold(
                    None,
                    |acc: Option<uninote_stroke::types::Rect>, b| match acc {
                        Some(prev) => Some(prev.union(&b)),
                        None => Some(b),
                    },
                );

            st.next_stroke_id = 506;
            st.selection.page_index = Some(0);
            st.selection.selected_ids = orig_ids.clone();
            st.selection.bbox = union_b;
        }

        assert!(canvas.copy_selected());

        // 2. Esegui 15 paste consecutivi (5 * 15 = 75 nuovi elementi, totale 80)
        let total_pastes = 15;
        for _ in 0..total_pastes {
            assert!(canvas.paste_from_clipboard());
        }

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 5 + total_pastes * 5);
            assert_eq!(page.spatial_index.len(), 5 + total_pastes * 5);
        }

        // 3. Esegui una sequenza deterministica a salti di Undo e Redo (Random Walk pseudo-casuale)
        // e ad OGNI SINGOLO PASSO verifica l'invarianza matematica:
        // page.items.len() == page.spatial_index.len()
        let steps = [
            -3, 2, -5, 4, -8, 6, -5, 2, -3, 3, // una serie di passi avanti e indietro
        ];

        let mut current_pastes_applied = total_pastes;
        for &step in &steps {
            if step < 0 {
                let undo_count = (-step) as usize;
                for _ in 0..undo_count {
                    if current_pastes_applied > 0 {
                        assert!(canvas.undo(), "Undo must succeed during random walk");
                        current_pastes_applied -= 1;
                        let state_rc = canvas.state();
                        let st = state_rc.borrow();
                        let page = &st.document.pages[0];
                        let expected = 5 + current_pastes_applied * 5;
                        assert_eq!(
                            page.items.len(),
                            expected,
                            "Random walk undo items len mismatch"
                        );
                        assert_eq!(
                            page.spatial_index.len(),
                            expected,
                            "Random walk undo spatial index len mismatch"
                        );
                        assert_eq!(page.items.len(), page.spatial_index.len());
                    }
                }
            } else {
                let redo_count = step as usize;
                for _ in 0..redo_count {
                    if current_pastes_applied < total_pastes {
                        assert!(canvas.redo(), "Redo must succeed during random walk");
                        current_pastes_applied += 1;
                        let state_rc = canvas.state();
                        let st = state_rc.borrow();
                        let page = &st.document.pages[0];
                        let expected = 5 + current_pastes_applied * 5;
                        assert_eq!(
                            page.items.len(),
                            expected,
                            "Random walk redo items len mismatch"
                        );
                        assert_eq!(
                            page.spatial_index.len(),
                            expected,
                            "Random walk redo spatial index len mismatch"
                        );
                        assert_eq!(page.items.len(), page.spatial_index.len());
                    }
                }
            }
        }
    });
}

// ============================================================================
// ADVERSARIAL TEST 7: MULTI-PAGE CROSS-PASTE 1000 ITEMS & SQLITE WAL INTEGRITY
// ============================================================================

#[test]
fn test_adversarial_multi_page_cross_paste_1000_items_sqlite_wal() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Crea un documento Notebook con 3 pagine A4
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.pages.push(uninote_core::model::Page::new_a4(
                1,
                uninote_core::model::PageBackground::Blank,
            ));
            st.document.pages.push(uninote_core::model::Page::new_a4(
                2,
                uninote_core::model::PageBackground::Blank,
            ));

            // Pagina 0: inserisci 10 tratti (ID 1..=10)
            let page0 = &mut st.document.pages[0];
            for i in 1..=10 {
                let s = create_test_stroke(i, 50.0 + (i as f64) * 5.0, 50.0);
                let it = Item {
                    id: i,
                    z_index: i,
                    kind: ItemKind::Stroke(s),
                };
                page0.spatial_index.insert(&it);
                page0.items.push(it);
            }

            let union_b = page0
                .items
                .iter()
                .filter_map(|it| match &it.kind {
                    ItemKind::Stroke(s) => s.bounding_box(),
                })
                .fold(
                    None,
                    |acc: Option<uninote_stroke::types::Rect>, b| match acc {
                        Some(prev) => Some(prev.union(&b)),
                        None => Some(b),
                    },
                );

            st.next_stroke_id = 11;
            st.selection.page_index = Some(0);
            st.selection.selected_ids = (1..=10).collect();
            st.selection.bbox = union_b;
        }

        assert!(canvas.copy_selected());

        // Ora incolla 50 volte su Pagina 1 (50 * 10 = 500 tratti)
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p1_rect = uninote_core::coordinates::Viewport::page_rect(1);
            st.viewport.offset_y = -p1_rect.y * st.viewport.zoom + 50.0;
            st.selection.clear();
        }

        for p in 1..=50 {
            assert!(canvas.paste_from_clipboard(), "Paste #{p} on page 1");
        }

        // Ora incolla 50 volte su Pagina 2 (50 * 10 = 500 tratti)
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let p2_rect = uninote_core::coordinates::Viewport::page_rect(2);
            st.viewport.offset_y = -p2_rect.y * st.viewport.zoom + 50.0;
            st.selection.clear();
        }

        for p in 1..=50 {
            assert!(canvas.paste_from_clipboard(), "Paste #{p} on page 2");
        }

        // Verifica totali:
        // Pagina 0: 10
        // Pagina 1: 500
        // Pagina 2: 500
        // Totale: 1010 elementi
        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert_eq!(st.document.pages[0].items.len(), 10);
        assert_eq!(st.document.pages[0].spatial_index.len(), 10);
        assert_eq!(st.document.pages[1].items.len(), 500);
        assert_eq!(st.document.pages[1].spatial_index.len(), 500);
        assert_eq!(st.document.pages[2].items.len(), 500);
        assert_eq!(st.document.pages[2].spatial_index.len(), 500);

        // Controllo unicità GLOBALE degli ID su tutte le 3 pagine
        let mut global_ids = HashSet::new();
        for page in &st.document.pages {
            for item in &page.items {
                assert!(
                    global_ids.insert(item.id),
                    "Cross-page ID collision detected: {}",
                    item.id
                );
            }
        }
        assert_eq!(global_ids.len(), 1010);

        // Salvataggio nel database SQLite WAL multi-pagina
        let db_path = unique_test_db("multipage_1000");
        {
            let mut store = DocumentStore::open_or_create(&db_path)
                .expect("Failed to open SQLite DocumentStore");
            store
                .save_document(&st.document)
                .expect("Failed to save multi-page document with 1010 items to SQLite WAL");
        }

        // Ricaricamento e verifica fedeltà
        {
            let store = DocumentStore::open_or_create(&db_path).expect("reopen");
            let loaded = store.load_document().expect("load multi-page");
            assert_eq!(loaded.pages.len(), 3);
            assert_eq!(loaded.pages[0].items.len(), 10);
            assert_eq!(loaded.pages[0].spatial_index.len(), 10);
            assert_eq!(loaded.pages[1].items.len(), 500);
            assert_eq!(loaded.pages[1].spatial_index.len(), 500);
            assert_eq!(loaded.pages[2].items.len(), 500);
            assert_eq!(loaded.pages[2].spatial_index.len(), 500);

            let mut loaded_global_ids = HashSet::new();
            for page in &loaded.pages {
                for item in &page.items {
                    assert!(loaded_global_ids.insert(item.id));
                }
            }
            assert_eq!(loaded_global_ids, global_ids);
        }

        let _ = fs::remove_file(&db_path);
        let _ = fs::remove_file(format!("{}-wal", db_path.display()));
        let _ = fs::remove_file(format!("{}-shm", db_path.display()));
    });
}

// ============================================================================
// ADVERSARIAL TEST 8: MASS ERASER & SPATIAL INTEGRITY ON PASTED ITEMS
// ============================================================================

#[test]
fn test_adversarial_mass_eraser_and_spatial_integrity() {
    run_on_gtk_thread(|| {
        clear_clipboard();
        let canvas = create_test_canvas();

        // Inserisci un tratto a (200.0, 200.0)
        let s = create_test_stroke(600, 200.0, 200.0);
        let orig_bbox = s.bounding_box().unwrap();

        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let page = &mut st.document.pages[0];
            let it = Item {
                id: 600,
                z_index: 1,
                kind: ItemKind::Stroke(s),
            };
            page.spatial_index.insert(&it);
            page.items.push(it);
            st.next_stroke_id = 601;

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![600];
            st.selection.bbox = Some(orig_bbox);
        }

        assert!(canvas.copy_selected());

        // Incolla 5 volte: creiamo 5 tratti disposti a cascata (+25, +50, +75, +100, +125)
        for _ in 1..=5 {
            assert!(canvas.paste_from_clipboard());
        }

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 6);
            assert_eq!(st.document.pages[0].spatial_index.len(), 6);
        }

        // Ora cancella con un raggio enorme (raggio 500 pt centrato su 250, 250)
        // che ingloba tutti i 6 tratti (l'originale e tutti i 5 incollati)
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            NoteCanvas::erase_strokes_at(&mut st, (250.0, 250.0), 500.0);
        }

        // Verifica che la pagina sia completamente vuota e l'indice spaziale a 0
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 0, "Mass erase must remove all items");
            assert_eq!(
                page.spatial_index.len(),
                0,
                "Spatial index must be empty after mass erase"
            );
            assert!(
                page.spatial_index.is_empty(),
                "spatial_index.is_empty() must be true"
            );
        }

        // Undo della cancellazione di massa: tutti i 6 tratti devono tornare!
        assert!(canvas.undo(), "Undo mass erase must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 6);
            assert_eq!(page.spatial_index.len(), 6);
        }

        // Redo della cancellazione di massa: tutti i 6 tratti devono sparire di nuovo!
        assert!(canvas.redo(), "Redo mass erase must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 0);
            assert_eq!(page.spatial_index.len(), 0);
        }
    });
}
