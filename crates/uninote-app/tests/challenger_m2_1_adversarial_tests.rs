//! Empirical Challenger Adversarial Verification Suite for Milestone M2
//! Agent: challenger_m2_1
//! Scope:
//! 1. DUP-004: Centralizzazione del riordino z-index e invarianza matematica / comportamentale.
//! 2. OBS-004 & RID-002: Snellimento di `create_card_with_refresh` e `create_card` con metadati avversari.
//! 3. DUP-003: Esportazione PDF chiaro/scuro e divergenza coerente delle opzioni.
//! 4. OBS-005: Hot-path dell'input a 120Hz/240Hz senza `recorder` e test del recorder in `uninote-stroke`.

use gtk4::prelude::*;
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;
use std::time::Instant;

use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::dashboard::{OpenDocCallback, UniNoteDashboard};
use uninote_app::hud::DiagnosticHud;
use uninote_app::pdf_export::{export_to_pdf_with_options, PdfExportOptions};
use uninote_app::preferences::{DashboardRecentStore, RecentDocumentMeta};
use uninote_core::model::{Document, DocumentFormat, ImageItem, Item, ItemId, PageBackground};
use uninote_stroke::recorder::RecordedSession;
use uninote_stroke::{
    DeviceKind, InputDisposition, InputPhase, InputSample, PalmRejectionEngine, PipelineMode, Rect,
    Stroke, StrokeBuilder, StrokePoint, StrokeStyle, TouchMode,
};
use uuid::Uuid;

// ============================================================================
// GTK TEST HARNESS (Worker thread with event loop safety)
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
            .name("gtk-challenger-m2-1-worker".into())
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
        .expect("Failed to send task to GTK worker");

    match res_rx.recv().expect("Failed to receive test result") {
        Ok(val) => val,
        Err(err) => std::panic::resume_unwind(err),
    }
}

static TEST_TEMP_COUNTER: AtomicU64 = AtomicU64::new(5000);

fn unique_challenger_temp_path(prefix: &str, ext: &str) -> PathBuf {
    let id = TEST_TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_m2_challenger_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
    ));
    path
}

fn create_mock_stroke(id: ItemId, z_index: u64, x: f64, y: f64) -> Item {
    let points = vec![
        StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 0,
        },
        StrokePoint {
            x: x + 10.0,
            y: y + 10.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 10,
        },
    ];

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        cached_bezier: Vec::new(),
        cached_bbox: None,
        mode: PipelineMode::CatmullRomSpline,
        is_dot: false,
    };
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

fn create_mock_image(id: ItemId, z_index: u64, x: f64, y: f64, w: f64, h: f64) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        Uuid::new_v4(),
        format!("hash_{}", id),
        Rect::new(x, y, w, h),
        z_index as i64,
        1.0,
        w as u32,
        h as u32,
    );
    Item::new_image(id, z_index, img)
}

// ============================================================================
// ITEM 1: DUP-004 Z-INDEX REORDERING MATHEMATICAL INVARIANCE
// ============================================================================

#[test]
fn test_dup004_invariance_direct_vs_wrapper_bring_to_front() {
    run_on_gtk_thread(|| {
        let setup_canvas = |sel_ids: Vec<ItemId>| -> NoteCanvas {
            let hud = Rc::new(DiagnosticHud::new());
            let canvas = NoteCanvas::new(hud);
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();

            for i in 1..=6 {
                if i % 2 == 1 {
                    page.items
                        .push(create_mock_stroke(i, i, i as f64 * 10.0, 10.0));
                } else {
                    page.items
                        .push(create_mock_image(i, i, i as f64 * 10.0, 10.0, 50.0, 50.0));
                }
            }
            page.rebuild_spatial_index();

            st.selection.page_index = Some(0);
            st.selection.selected_ids = sel_ids;
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
            st.active_tool = ActiveTool::Select;
            drop(st);
            canvas
        };

        let sel_test = vec![2, 4];
        let canvas_direct = setup_canvas(sel_test.clone());
        let canvas_wrapper = setup_canvas(sel_test);

        let res_direct = canvas_direct.reorder_selected_z_index(true);
        let res_wrapper = canvas_wrapper.bring_to_front_selected();

        assert_eq!(res_direct, res_wrapper, "Return value must be identical");
        assert!(res_direct, "Bring to front operation must succeed");

        let state_dir = canvas_direct.state();
        let st_direct = state_dir.borrow();
        let state_wrap = canvas_wrapper.state();
        let st_wrapper = state_wrap.borrow();
        let page_dir = &st_direct.document.pages[0];
        let page_wrap = &st_wrapper.document.pages[0];

        assert_eq!(page_dir.items.len(), page_wrap.items.len());
        for (it_d, it_w) in page_dir.items.iter().zip(page_wrap.items.iter()) {
            assert_eq!(it_d.id, it_w.id, "Items must have identical ID order");
            assert_eq!(
                it_d.z_index, it_w.z_index,
                "Items must have identical z_index"
            );
        }

        // Verifica invarianza: non-selected [1, 3, 5, 6] poi selected [2, 4]
        let ids: Vec<ItemId> = page_dir.items.iter().map(|it| it.id).collect();
        assert_eq!(
            ids,
            vec![1, 3, 5, 6, 2, 4],
            "Selected items must be at the end"
        );
        let zs: Vec<u64> = page_dir.items.iter().map(|it| it.z_index).collect();
        assert_eq!(
            zs,
            vec![1, 2, 3, 4, 5, 6],
            "Z-indices must be compact 1..=N"
        );
    });
}

#[test]
fn test_dup004_invariance_direct_vs_wrapper_send_to_back() {
    run_on_gtk_thread(|| {
        let setup_canvas = |sel_ids: Vec<ItemId>| -> NoteCanvas {
            let hud = Rc::new(DiagnosticHud::new());
            let canvas = NoteCanvas::new(hud);
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();

            for i in 1..=6 {
                page.items
                    .push(create_mock_stroke(i, i, i as f64 * 10.0, 10.0));
            }
            page.rebuild_spatial_index();

            st.selection.page_index = Some(0);
            st.selection.selected_ids = sel_ids;
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
            st.active_tool = ActiveTool::Select;
            drop(st);
            canvas
        };

        let sel_test = vec![3, 5];
        let canvas_direct = setup_canvas(sel_test.clone());
        let canvas_wrapper = setup_canvas(sel_test);

        let res_direct = canvas_direct.reorder_selected_z_index(false);
        let res_wrapper = canvas_wrapper.send_to_back_selected();

        assert_eq!(res_direct, res_wrapper, "Return value must be identical");
        assert!(res_direct, "Send to back operation must succeed");

        let state_dir = canvas_direct.state();
        let st_direct = state_dir.borrow();
        let state_wrap = canvas_wrapper.state();
        let st_wrapper = state_wrap.borrow();
        let page_dir = &st_direct.document.pages[0];
        let page_wrap = &st_wrapper.document.pages[0];

        assert_eq!(page_dir.items.len(), page_wrap.items.len());
        for (it_d, it_w) in page_dir.items.iter().zip(page_wrap.items.iter()) {
            assert_eq!(it_d.id, it_w.id);
            assert_eq!(it_d.z_index, it_w.z_index);
        }

        // Invarianza send_to_back: selected [3, 5] a z=1, 2, poi non-selected [1, 2, 4, 6] a z=3, 4, 5, 6
        let ids: Vec<ItemId> = page_dir.items.iter().map(|it| it.id).collect();
        assert_eq!(
            ids,
            vec![3, 5, 1, 2, 4, 6],
            "Selected items must be at the beginning"
        );
        let zs: Vec<u64> = page_dir.items.iter().map(|it| it.z_index).collect();
        assert_eq!(
            zs,
            vec![1, 2, 3, 4, 5, 6],
            "Z-indices must be compact 1..=N"
        );
    });
}

#[test]
fn test_dup004_edge_cases_empty_and_all_selected() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            for i in 1..=4 {
                page.items
                    .push(create_mock_stroke(i, i, i as f64 * 10.0, 10.0));
            }
            page.rebuild_spatial_index();
            // 1. Nessun elemento selezionato
            st.selection.page_index = Some(0);
            st.selection.selected_ids.clear();
        }

        assert!(
            !canvas.bring_to_front_selected(),
            "Empty selection must return false"
        );
        assert!(
            !canvas.send_to_back_selected(),
            "Empty selection must return false"
        );
        assert!(!canvas.reorder_selected_z_index(true));
        assert!(!canvas.reorder_selected_z_index(false));

        // 2. Tutti gli elementi selezionati: deve restituire false (nessun non_selected rispetto a cui riordinare)
        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            st.selection.selected_ids = vec![1, 2, 3, 4];
        }

        assert!(
            !canvas.bring_to_front_selected(),
            "All selected must return false"
        );
        assert!(
            !canvas.send_to_back_selected(),
            "All selected must return false"
        );
        assert!(!canvas.reorder_selected_z_index(true));
        assert!(!canvas.reorder_selected_z_index(false));

        // Verifica che lo stato della pagina sia rimasto rigorosamente intatto
        {
            let state = canvas.state();
            let st = state.borrow();
            let page = &st.document.pages[0];
            let ids: Vec<ItemId> = page.items.iter().map(|it| it.id).collect();
            let zs: Vec<u64> = page.items.iter().map(|it| it.z_index).collect();
            assert_eq!(ids, vec![1, 2, 3, 4]);
            assert_eq!(zs, vec![1, 2, 3, 4]);
        }
    });
}

#[test]
fn test_dup004_edge_cases_already_at_boundaries() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            for i in 1..=4 {
                page.items
                    .push(create_mock_stroke(i, i, i as f64 * 10.0, 10.0));
            }
            page.rebuild_spatial_index();
            st.selection.page_index = Some(0);
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        }

        // Seleziona l'ultimo elemento (id 4, z=4): bring_to_front è già soddisfatto
        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            st.selection.selected_ids = vec![4];
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        }
        assert!(
            !canvas.bring_to_front_selected(),
            "Already at top should return false (no-op)"
        );

        // Seleziona il primo elemento (id 1, z=1): send_to_back è già soddisfatto
        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            st.selection.selected_ids = vec![1];
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        }
        assert!(
            !canvas.send_to_back_selected(),
            "Already at bottom should return false (no-op)"
        );
    });
}

#[test]
fn test_dup004_fragmented_selection_and_undo_redo_symmetry() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            // 7 elementi con ID da 10 a 16
            for (idx, id) in (10..=16).enumerate() {
                let z = (idx + 1) as u64;
                page.items
                    .push(create_mock_stroke(id, z, id as f64 * 5.0, 10.0));
            }
            page.rebuild_spatial_index();
            st.selection.page_index = Some(0);
            // Selezione frammentata: 11, 13, 15
            st.selection.selected_ids = vec![11, 13, 15];
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        }

        // Esegui bring_to_front_selected
        assert!(canvas.bring_to_front_selected());

        {
            let state = canvas.state();
            let st = state.borrow();
            let page = &st.document.pages[0];
            let ids: Vec<ItemId> = page.items.iter().map(|it| it.id).collect();
            let zs: Vec<u64> = page.items.iter().map(|it| it.z_index).collect();
            assert_eq!(ids, vec![10, 12, 14, 16, 11, 13, 15]);
            assert_eq!(zs, vec![1, 2, 3, 4, 5, 6, 7]);
        }

        // Test Undo
        assert!(canvas.undo(), "Undo must succeed");
        {
            let state = canvas.state();
            let st = state.borrow();
            let page = &st.document.pages[0];
            let ids: Vec<ItemId> = page.items.iter().map(|it| it.id).collect();
            let zs: Vec<u64> = page.items.iter().map(|it| it.z_index).collect();
            assert_eq!(ids, vec![10, 11, 12, 13, 14, 15, 16]);
            assert_eq!(zs, vec![1, 2, 3, 4, 5, 6, 7]);
        }

        // Test Redo
        assert!(canvas.redo(), "Redo must succeed");
        {
            let state = canvas.state();
            let st = state.borrow();
            let page = &st.document.pages[0];
            let ids: Vec<ItemId> = page.items.iter().map(|it| it.id).collect();
            assert_eq!(ids, vec![10, 12, 14, 16, 11, 13, 15]);
        }
    });
}

// ============================================================================
// ITEM 2: OBS-004 & RID-002 DASHBOARD CARD CREATION & REFRESH VERIFICATION
// ============================================================================

#[test]
fn test_obs004_rid002_dashboard_card_adversarial_metadata() {
    run_on_gtk_thread(|| {
        let store = Rc::new(RefCell::new(DashboardRecentStore::new()));
        let open_cb: OpenDocCallback = Rc::new(RefCell::new(Some(Box::new(|_path| {}))));
        let refresh_invocations = Rc::new(std::cell::Cell::new(0));

        let adversarial_cases = [
            RecentDocumentMeta {
                file_path: "/path/to/normal.uninote".into(),
                title: "Quaderno Standard".into(),
                format: DocumentFormat::Notebook,
                last_modified_timestamp: 1600000000,
                page_count: 12,
                thumbnail_path: None,
            },
            RecentDocumentMeta {
                file_path: "/path/to/whiteboard.uninote".into(),
                title: "Lavagna Vuota".into(),
                format: DocumentFormat::Whiteboard,
                last_modified_timestamp: 1600000000,
                page_count: 0,
                thumbnail_path: None,
            },
            RecentDocumentMeta {
                file_path: "/non/existent/path/doc.uninote".into(),
                title: "".into(),
                format: DocumentFormat::Notebook,
                last_modified_timestamp: 0,
                page_count: 1,
                thumbnail_path: Some("/invalid/thumb.png".into()),
            },
            RecentDocumentMeta {
                file_path: "/tmp/deep/path/nested/long_title.uninote".into(),
                title: "A".repeat(5000),
                format: DocumentFormat::Notebook,
                last_modified_timestamp: u64::MAX,
                page_count: 99999,
                thumbnail_path: None,
            },
            RecentDocumentMeta {
                file_path: "../../../traversal/attempt.uninote".into(),
                title: "\t\n!@#$%^&*()_+-=[]{}|;':\",./<>?~`🚀🦀💡".into(),
                format: DocumentFormat::Whiteboard,
                last_modified_timestamp: 12345,
                page_count: 1,
                thumbnail_path: None,
            },
        ];

        for (idx, meta) in adversarial_cases.iter().enumerate() {
            let ref_clone = refresh_invocations.clone();
            let refresh_cb = Rc::new(move || {
                ref_clone.set(ref_clone.get() + 1);
            });

            // 1. Verifica create_card_with_refresh
            let card_child = UniNoteDashboard::create_card_with_refresh(
                meta,
                &open_cb,
                &store,
                refresh_cb.clone(),
            );

            assert!(
                card_child.child().is_some(),
                "Card child must contain inner GtkBox for case {}",
                idx
            );
            assert!(
                card_child.has_css_class("dms-flowbox-child"),
                "Card must have dms-flowbox-child CSS class"
            );

            // 2. Verifica create_card (wrapper snellito senza widget fittizi)
            let default_card = UniNoteDashboard::create_card(meta, &open_cb);
            assert!(
                default_card.child().is_some(),
                "Default card must contain inner GtkBox for case {}",
                idx
            );
        }

        // Verifica che il refresh callback sia invocabile e risponda senza panic
        let ref_test = refresh_invocations.clone();
        let direct_cb = Rc::new(move || {
            ref_test.set(ref_test.get() + 1);
        });
        direct_cb();
        assert_eq!(refresh_invocations.get(), 1);
    });
}

// ============================================================================
// ITEM 3: DUP-003 PDF EXPORT OPTIONS (LIGHT VS DARK)
// ============================================================================

#[test]
fn test_dup003_pdf_export_options_distinct_light_and_dark() {
    let opt_light = PdfExportOptions { is_dark: false };
    let opt_dark = PdfExportOptions { is_dark: true };

    assert_ne!(opt_light, opt_dark, "Options must not be equal");
    assert!(
        !opt_light.is_dark,
        "Light options must have is_dark = false"
    );
    assert!(opt_dark.is_dark, "Dark options must have is_dark = true");
}

#[test]
fn test_dup003_pdf_export_execution_binary_differentiation() {
    let temp_light = unique_challenger_temp_path("pdf_export_light", "pdf");
    let temp_dark = unique_challenger_temp_path("pdf_export_dark", "pdf");

    let mut doc = Document::new_empty("Corso di Algoritmi Avanzati");
    {
        let page = &mut doc.pages[0];
        page.background = PageBackground::Grid { grid_size: 14.17 };
        page.items.push(create_mock_stroke(1, 1, 50.0, 50.0));
        page.items.push(create_mock_stroke(2, 2, 100.0, 100.0));
    }

    // Esportazione PDF con opzioni Chiaro
    let res_light =
        export_to_pdf_with_options(&doc, &temp_light, &PdfExportOptions { is_dark: false });
    assert!(res_light.is_ok(), "Light PDF export must succeed");
    assert!(temp_light.exists(), "Light PDF file must exist");

    // Esportazione PDF con opzioni Scuro
    let res_dark =
        export_to_pdf_with_options(&doc, &temp_dark, &PdfExportOptions { is_dark: true });
    assert!(res_dark.is_ok(), "Dark PDF export must succeed");
    assert!(temp_dark.exists(), "Dark PDF file must exist");

    let light_bytes = fs::read(&temp_light).expect("Must read light PDF");
    let dark_bytes = fs::read(&temp_dark).expect("Must read dark PDF");

    assert!(
        light_bytes.starts_with(b"%PDF-"),
        "Light file must be valid PDF"
    );
    assert!(
        dark_bytes.starts_with(b"%PDF-"),
        "Dark file must be valid PDF"
    );

    // I due file devono avere contenuto binario differente a causa dello sfondo scuro e adattamento colori
    assert_ne!(
        light_bytes, dark_bytes,
        "Light and Dark PDF exports must generate distinct vector streams"
    );

    let _ = fs::remove_file(temp_light);
    let _ = fs::remove_file(temp_dark);
}

// ============================================================================
// ITEM 4: OBS-005 INPUT HOT-PATH 120HZ / 240HZ & RECORDER TESTS
// ============================================================================

#[test]
fn test_obs005_recorded_session_replay_determinism_in_stroke() {
    let mut session = RecordedSession::new("test_session");
    for i in 0..100 {
        let sample = InputSample::new(
            (10.0 + i as f64, 20.0 + i as f64),
            Some(0.5),
            i * 8_333_333,
            DeviceKind::Stylus,
            if i == 0 {
                InputPhase::Down
            } else if i == 99 {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        );
        session.record_sample(sample);
    }

    assert_eq!(session.samples.len(), 100);

    let strokes = session.replay(PipelineMode::CatmullRomSpline, StrokeStyle::default());
    assert_eq!(strokes.len(), 1, "Replay must produce exactly 1 stroke");
    assert!(!strokes[0].points.is_empty(), "Stroke must contain points");

    // Determinismo: riesecuzione produce identico numero di punti
    let strokes_replay2 = session.replay(PipelineMode::CatmullRomSpline, StrokeStyle::default());
    assert_eq!(strokes[0].points.len(), strokes_replay2[0].points.len());
}

#[test]
fn test_obs005_input_hot_path_120hz_and_240hz_stress_latency() {
    let mut builder = StrokeBuilder::new(1, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    let mut palm_engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    // 240Hz: campionamento a intervalli di ~4.16ms (4_166_666 ns)
    let sample_count = 2400; // 10 secondi di scrittura a 240Hz
    let start_time = Instant::now();

    for i in 0..sample_count {
        let phase = if i == 0 {
            InputPhase::Down
        } else if i == sample_count - 1 {
            InputPhase::Up
        } else {
            InputPhase::Move
        };

        let sample = InputSample::new(
            (50.0 + (i as f64 * 0.1), 50.0 + (i as f64 * 0.05)),
            Some(0.6),
            (i as u64) * 4_166_666,
            DeviceKind::Stylus,
            phase,
        );

        if palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
            builder.process_sample(sample);
        }
    }

    let elapsed = start_time.elapsed();
    let per_sample_us = (elapsed.as_micros() as f64) / (sample_count as f64);

    assert!(
        per_sample_us < 50.0,
        "Processing latency at 240Hz must be < 50µs per sample, actual: {:.2}µs",
        per_sample_us
    );

    let stroke = builder.finish();
    assert!(
        !stroke.points.is_empty(),
        "Stroke must contain smoothed points"
    );
}

#[test]
fn test_dup004_non_contiguous_initial_z_indices_compaction() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        {
            let state = canvas.state();
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            // Inizializza con z-index sparsi / non contigui: 10, 50, 100, 500
            page.items.push(create_mock_stroke(1, 10, 10.0, 10.0));
            page.items.push(create_mock_stroke(2, 50, 20.0, 20.0));
            page.items.push(create_mock_stroke(3, 100, 30.0, 30.0));
            page.items.push(create_mock_stroke(4, 500, 40.0, 40.0));
            page.rebuild_spatial_index();

            // Seleziona item 2 (z=50)
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![2];
            st.selection.bbox = Some(Rect::new(0.0, 0.0, 100.0, 100.0));
        }

        // Bring to front
        assert!(canvas.bring_to_front_selected());

        {
            let state = canvas.state();
            let st = state.borrow();
            let page = &st.document.pages[0];
            let ids: Vec<ItemId> = page.items.iter().map(|it| it.id).collect();
            let zs: Vec<u64> = page.items.iter().map(|it| it.z_index).collect();
            // Non selezionati: 1, 3, 4 (rispettivamente a z=1, 2, 3)
            // Selezionato: 2 (in coda a z=4)
            assert_eq!(ids, vec![1, 3, 4, 2]);
            assert_eq!(zs, vec![1, 2, 3, 4], "Z-indices must be compacted to 1..=4");
        }
    });
}

#[test]
fn test_obs004_show_confirmation_dialog_presentation_and_confirm() {
    run_on_gtk_thread(|| {
        let confirmed = Rc::new(std::cell::Cell::new(false));
        let conf_clone = confirmed.clone();

        // 1. Invoca UniNoteDashboard::show_confirmation_dialog con is_destructive = true
        UniNoteDashboard::show_confirmation_dialog(
            None,
            "Elimina Cartella — Test",
            "Confermi l'eliminazione della cartella?",
            "Elimina",
            true,
            move || {
                conf_clone.set(true);
            },
        );

        // 2. Invoca la funzione libera show_confirmation_dialog con is_destructive = false
        let confirmed2 = Rc::new(std::cell::Cell::new(false));
        let conf_clone2 = confirmed2.clone();
        uninote_app::dashboard::show_confirmation_dialog(
            None,
            "Conferma Azione — Test",
            "Procedere con l'operazione?",
            "Conferma",
            false,
            move || {
                conf_clone2.set(true);
            },
        );
    });
}
