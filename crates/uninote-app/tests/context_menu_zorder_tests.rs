use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use uninote_app::canvas::NoteCanvas;
use uninote_app::hud::DiagnosticHud;
use uninote_core::{ImageItem, Item, ItemId};
use uninote_stroke::{Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

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
            .name("gtk-context-menu-test-worker".into())
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
        mode: uninote_stroke::PipelineMode::CatmullRomSpline,
        is_dot: false,
    };
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

fn create_mock_image(id: ItemId, z_index: u64, x: f64, y: f64, w: f64, h: f64) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        Uuid::new_v4(),
        "dummy_hash_for_test".to_string(),
        Rect::new(x, y, w, h),
        z_index as i64,
        1.0,
        w as u32,
        h as u32,
    );
    Item::new_image(id, z_index, img)
}

#[test]
fn test_zorder_bring_to_front_selected() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        let item1 = create_mock_stroke(101, 1, 10.0, 10.0);
        let item2 = create_mock_stroke(102, 2, 20.0, 20.0);
        let item3 = create_mock_image(103, 3, 50.0, 50.0, 100.0, 100.0);

        let state = canvas.state();
        {
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            page.spatial_index = uninote_core::PageSpatialIndex::empty();

            page.items.push(item1.clone());
            page.items.push(item2.clone());
            page.items.push(item3.clone());
            page.rebuild_spatial_index();

            // Seleziona item1 (che ha z_index = 1, il più basso)
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![101];
            st.selection.bbox = item1.bounding_box();
            st.active_tool = uninote_app::canvas::ActiveTool::Select;
        }

        // Esegui "Sposta sopra"
        let ok = canvas.bring_to_front_selected();
        assert!(ok, "bring_to_front_selected deve avere successo");

        {
            let st = state.borrow();
            let page = &st.document.pages[0];
            // L'ultimo elemento in page.items deve essere item1 (id 101) con il z_index più alto
            let last_item = page.items.last().expect("last item");
            assert_eq!(last_item.id, 101, "item1 deve essere ora l'ultimo in lista");
            assert!(
                last_item.z_index > page.items[0].z_index,
                "item1 deve avere z_index superiore a quello degli altri elementi"
            );
        }
    });
}

#[test]
fn test_zorder_send_to_back_selected() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        let item1 = create_mock_stroke(201, 1, 10.0, 10.0);
        let item2 = create_mock_stroke(202, 2, 20.0, 20.0);
        let item3 = create_mock_image(203, 3, 50.0, 50.0, 100.0, 100.0);

        let state = canvas.state();
        {
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            page.spatial_index = uninote_core::PageSpatialIndex::empty();

            page.items.push(item1.clone());
            page.items.push(item2.clone());
            page.items.push(item3.clone());
            page.rebuild_spatial_index();

            // Seleziona item3 (che ha z_index = 3, il più alto)
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![203];
            st.selection.bbox = item3.bounding_box();
            st.active_tool = uninote_app::canvas::ActiveTool::Select;
        }

        // Esegui "Sposta sotto"
        let ok = canvas.send_to_back_selected();
        assert!(ok, "send_to_back_selected deve avere successo");

        {
            let st = state.borrow();
            let page = &st.document.pages[0];
            // Il primo elemento in page.items deve essere item3 (id 203) con il z_index più basso
            let first_item = page.items.first().expect("first item");
            assert_eq!(
                first_item.id, 203,
                "item3 deve essere ora il primo in lista"
            );
            assert!(
                first_item.z_index < page.items[1].z_index,
                "item3 deve avere z_index inferiore a quello degli altri elementi"
            );
        }
    });
}

#[test]
fn test_zorder_undo_redo_symmetry() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        let item1 = create_mock_stroke(301, 1, 10.0, 10.0);
        let item2 = create_mock_image(302, 2, 30.0, 30.0, 80.0, 80.0);

        let state = canvas.state();
        {
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            page.items.push(item1.clone());
            page.items.push(item2.clone());
            page.rebuild_spatial_index();

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![301];
            st.selection.bbox = item1.bounding_box();
        }

        // 1. Sposta sopra item1
        assert!(canvas.bring_to_front_selected());
        {
            let st = state.borrow();
            assert_eq!(st.document.pages[0].items[1].id, 301);
        }

        // 2. Undo: torna indietro
        assert!(canvas.undo());
        {
            let st = state.borrow();
            assert_eq!(st.document.pages[0].items[0].id, 301);
            assert_eq!(st.document.pages[0].items[1].id, 302);
            assert_eq!(st.document.pages[0].items[0].z_index, 1);
            assert_eq!(st.document.pages[0].items[1].z_index, 2);
        }

        // 3. Redo: riapplica
        assert!(canvas.redo());
        {
            let st = state.borrow();
            assert_eq!(st.document.pages[0].items[1].id, 301);
            assert_eq!(st.document.pages[0].items[0].id, 302);
        }
    });
}

#[test]
fn test_duplicate_selected_flow() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        let item1 = create_mock_image(401, 1, 100.0, 100.0, 150.0, 120.0);

        let state = canvas.state();
        {
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            page.items.push(item1.clone());
            page.rebuild_spatial_index();

            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![401];
            st.selection.bbox = item1.bounding_box();
            st.active_tool = uninote_app::canvas::ActiveTool::Select;
        }

        // Esegui "Duplica"
        let ok = canvas.duplicate_selected();
        assert!(ok, "duplicate_selected deve duplicare l'elemento");

        {
            let st = state.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), 2, "La pagina deve contenere 2 elementi");
            assert!(
                st.selection.has_selection(),
                "Il duplicato deve essere automaticamente selezionato"
            );
            assert_ne!(
                st.selection.selected_ids[0], 401,
                "La selezione deve puntare al nuovo ID duplicato"
            );
        }

        // Verifica che l'annullamento rimuova il duplicato
        assert!(canvas.undo());
        {
            let st = state.borrow();
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].items[0].id, 401);
        }
    });
}

#[test]
fn test_zorder_mixed_selection_strokes_and_images() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        let item1 = create_mock_stroke(501, 1, 10.0, 10.0);
        let item2 = create_mock_image(502, 2, 20.0, 20.0, 50.0, 50.0);
        let item3 = create_mock_stroke(503, 3, 30.0, 30.0);
        let item4 = create_mock_image(504, 4, 60.0, 60.0, 70.0, 70.0);

        let state = canvas.state();
        {
            let mut st = state.borrow_mut();
            let page = &mut st.document.pages[0];
            page.items.clear();
            page.items.push(item1.clone());
            page.items.push(item2.clone());
            page.items.push(item3.clone());
            page.items.push(item4.clone());
            page.rebuild_spatial_index();

            // Seleziona un gruppo misto: item1 (stroke) e item2 (image)
            st.selection.page_index = Some(0);
            st.selection.selected_ids = vec![501, 502];
            st.selection.bbox = Some(
                item1
                    .bounding_box()
                    .unwrap()
                    .union(&item2.bounding_box().unwrap()),
            );
            st.active_tool = uninote_app::canvas::ActiveTool::Select;
        }

        // Porta il gruppo misto sopra a tutti gli altri
        assert!(canvas.bring_to_front_selected());

        {
            let st = state.borrow();
            let page = &st.document.pages[0];
            // Gli ultimi due elementi devono essere 501 e 502, con 501 prima di 502 (ordine relativo preservato)
            assert_eq!(page.items[0].id, 503);
            assert_eq!(page.items[1].id, 504);
            assert_eq!(page.items[2].id, 501);
            assert_eq!(page.items[3].id, 502);
            assert!(page.items[2].z_index > page.items[1].z_index);
            assert!(page.items[3].z_index > page.items[2].z_index);
        }

        // Ora porta lo stesso gruppo sotto a tutti gli altri
        assert!(canvas.send_to_back_selected());

        {
            let st = state.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items[0].id, 501);
            assert_eq!(page.items[1].id, 502);
            assert_eq!(page.items[2].id, 503);
            assert_eq!(page.items[3].id, 504);
        }
    });
}
