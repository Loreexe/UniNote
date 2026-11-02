//! Test suite per la selezione, manipolazione e interazione con ImageItem nel Canvas.
//! Verifica che le immagini possano essere:
//! 1. Riselezionate tramite rettangolo (inclusione e intersezione);
//! 2. Riselezionate tramite lazo (inclusione e attraversamento);
//! 3. Selezionate assieme ai tratti con unificazione del bounding box;
//! 4. Selezionate tramite click/tap diretto sull'immagine;
//! 5. Traslate e trasformate a seguito della riselezione.

use uuid::Uuid;

use uninote_app::canvas::{CanvasState, NoteCanvas};
use uninote_app::selection::{AppSelectionEngine, SelectionDragKind};
use uninote_core::model::{Document, ImageItem, Item, ItemKind, Page, PageBackground};
use uninote_stroke::types::{Rect, Stroke, StrokePoint, StrokeStyle};

fn create_sample_image(page_id: Uuid, id: u64, bbox: Rect) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        "sample_hash_12345".to_string(),
        bbox,
        1,
        1.0,
        bbox.width() as u32,
        bbox.height() as u32,
    );
    Item {
        id,
        z_index: 1,
        kind: ItemKind::Image(img),
    }
}

fn create_sample_stroke(id: u64, coords: &[(f64, f64)]) -> Item {
    let points = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 1_000_000 * (i as u64),
        })
        .collect();

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        cached_bezier: Vec::new(),
        cached_bbox: None,
        mode: uninote_stroke::types::PipelineMode::CatmullRomSpline,
        is_dot: false,
    };
    stroke.ensure_cache();
    Item {
        id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke),
    }
}

#[test]
fn test_image_selection_by_enclosing_rectangle() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let img_item = create_sample_image(page_id, 42, Rect::new(100.0, 100.0, 300.0, 250.0));
    page.items.push(img_item);
    page.rebuild_spatial_index();

    let query_rect = Rect::new(50.0, 50.0, 350.0, 300.0);
    let (selected_ids, bbox) = AppSelectionEngine::select_rect(&page, &query_rect);

    assert_eq!(selected_ids, vec![42]);
    let b = bbox.expect("Bounding box must be present");
    assert!((b.min_x - 100.0).abs() < 1e-4);
    assert!((b.min_y - 100.0).abs() < 1e-4);
    assert!((b.max_x - 300.0).abs() < 1e-4);
    assert!((b.max_y - 250.0).abs() < 1e-4);
}

#[test]
fn test_image_selection_by_intersecting_rectangle() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let img_item = create_sample_image(page_id, 99, Rect::new(100.0, 100.0, 300.0, 250.0));
    page.items.push(img_item);
    page.rebuild_spatial_index();

    // Rettangolo che tocca solo un angolo dell'immagine
    let query_rect = Rect::new(80.0, 80.0, 150.0, 150.0);
    let (selected_ids, bbox) = AppSelectionEngine::select_rect(&page, &query_rect);

    assert_eq!(selected_ids, vec![99]);
    assert!(bbox.is_some());
}

#[test]
fn test_image_selection_outside_rectangle() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let img_item = create_sample_image(page_id, 10, Rect::new(100.0, 100.0, 200.0, 200.0));
    page.items.push(img_item);
    page.rebuild_spatial_index();

    let query_rect = Rect::new(300.0, 300.0, 400.0, 400.0);
    let (selected_ids, bbox) = AppSelectionEngine::select_rect(&page, &query_rect);

    assert!(selected_ids.is_empty());
    assert!(bbox.is_none());
}

#[test]
fn test_image_selection_by_enclosing_lasso() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let img_item = create_sample_image(page_id, 77, Rect::new(100.0, 100.0, 200.0, 200.0));
    page.items.push(img_item);
    page.rebuild_spatial_index();

    let poly = vec![
        (50.0, 50.0),
        (250.0, 50.0),
        (250.0, 250.0),
        (50.0, 250.0),
        (50.0, 50.0),
    ];
    let (selected_ids, bbox) = AppSelectionEngine::select_lasso(&page, &poly);

    assert_eq!(selected_ids, vec![77]);
    assert!(bbox.is_some());
}

#[test]
fn test_mixed_selection_strokes_and_images() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let stroke_item = create_sample_stroke(1, &[(10.0, 10.0), (30.0, 30.0)]);
    let img_item = create_sample_image(page_id, 2, Rect::new(100.0, 100.0, 200.0, 200.0));
    page.items.push(stroke_item);
    page.items.push(img_item);
    page.rebuild_spatial_index();

    let query_rect = Rect::new(0.0, 0.0, 250.0, 250.0);
    let (mut selected_ids, bbox) = AppSelectionEngine::select_rect(&page, &query_rect);
    selected_ids.sort();

    assert_eq!(selected_ids, vec![1, 2]);
    let b = bbox.expect("Unified bounding box");
    assert!(b.min_x <= 10.0);
    assert!(b.min_y <= 10.0);
    assert!(b.max_x >= 200.0);
    assert!(b.max_y >= 200.0);
}

#[test]
fn test_direct_image_click_selection_and_move() {
    let mut doc = Document::new_notebook("Direct Click Test");
    let page_id = doc.pages[0].id;
    let img_item = create_sample_image(page_id, 101, Rect::new(100.0, 100.0, 200.0, 200.0));
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // 1. Click diretto all'interno dell'immagine (150.0, 150.0)
    NoteCanvas::handle_selection_down(&mut st, (150.0, 150.0), false);

    assert_eq!(st.selection.selected_ids, vec![101]);
    assert!(st.selection.is_dragging);
    assert_eq!(st.selection.drag_kind, SelectionDragKind::Translate);
    let bbox = st.selection.bbox.expect("BBox must be set");
    assert_eq!(bbox, Rect::new(100.0, 100.0, 200.0, 200.0));

    // 2. Trascina l'immagine di (+30, +40)
    NoteCanvas::handle_selection_motion(&mut st, (180.0, 190.0), false);
    assert_eq!(st.selection.drag_offset, (30.0, 40.0));

    // 3. Rilascio (Up): esecuzione MoveItems
    NoteCanvas::handle_selection_up(&mut st);
    assert!(!st.selection.is_dragging);
    assert_eq!(st.selection.selected_ids, vec![101]); // Rimane selezionata!
    let new_bbox = st.selection.bbox.expect("Updated bbox");
    assert_eq!(new_bbox, Rect::new(130.0, 140.0, 230.0, 240.0));

    // Verifica nel documento che l'ImageItem sia stato effettivamente spostato
    if let ItemKind::Image(ref img) = st.document.pages[0].items[0].kind {
        assert_eq!(img.bbox, Rect::new(130.0, 140.0, 230.0, 240.0));
    } else {
        panic!("Item 0 must be Image");
    }

    // 4. Click nel vuoto (500.0, 500.0) deseleziona l'immagine
    NoteCanvas::handle_selection_down(&mut st, (500.0, 500.0), false);
    NoteCanvas::handle_selection_up(&mut st);
    assert!(st.selection.selected_ids.is_empty());
    assert!(st.selection.bbox.is_none());
}

#[test]
fn test_direct_image_click_handle_resize() {
    let mut doc = Document::new_notebook("Handle Resize Test");
    let page_id = doc.pages[0].id;
    let img_item = create_sample_image(page_id, 202, Rect::new(100.0, 100.0, 200.0, 200.0));
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // 1. Click per selezionare l'immagine
    NoteCanvas::handle_selection_down(&mut st, (150.0, 150.0), false);
    NoteCanvas::handle_selection_up(&mut st);
    assert_eq!(st.selection.selected_ids, vec![202]);

    // 2. Click sulla maniglia BottomRight (200.0, 200.0)
    NoteCanvas::handle_selection_down(&mut st, (200.0, 200.0), false);
    assert!(st.selection.is_dragging);
    match st.selection.drag_kind {
        SelectionDragKind::ScaleHandle(_) => {}
        _ => panic!("Expected ScaleHandle drag kind"),
    }

    // 3. Trascina la maniglia per ingrandire fino a (250.0, 250.0)
    NoteCanvas::handle_selection_motion(&mut st, (250.0, 250.0), false);

    // 4. Rilascio: trasformazione applicata
    NoteCanvas::handle_selection_up(&mut st);
    assert_eq!(st.selection.selected_ids, vec![202]);
    let new_bbox = st.selection.bbox.expect("Scaled bbox");
    assert!(new_bbox.width() > 100.0);
    assert!(new_bbox.height() > 100.0);

    // Verifica nel modello che l'immagine sia stata ridimensionata
    if let ItemKind::Image(ref img) = st.document.pages[0].items[0].kind {
        assert!(img.bbox.width() > 100.0);
        assert!(img.bbox.height() > 100.0);
    } else {
        panic!("Item 0 must be Image");
    }
}

#[test]
fn test_image_selection_by_intersecting_lasso() {
    let mut page = Page::new(0, PageBackground::Blank);
    let page_id = page.id;
    let img_item = create_sample_image(page_id, 88, Rect::new(100.0, 100.0, 200.0, 200.0));
    page.items.push(img_item);
    page.rebuild_spatial_index();

    // Lazo che attraversa solo il lato sinistro dell'immagine senza racchiuderla tutta
    let poly = vec![
        (80.0, 80.0),
        (150.0, 80.0),
        (150.0, 220.0),
        (80.0, 220.0),
        (80.0, 80.0),
    ];
    let (selected_ids, bbox) = AppSelectionEngine::select_lasso(&page, &poly);

    assert_eq!(selected_ids, vec![88]);
    assert!(bbox.is_some());
}

#[test]
fn test_image_handle_scale_undo_redo_symmetry() {
    let mut doc = Document::new_notebook("Handle Scale Undo Redo Test");
    let page_id = doc.pages[0].id;
    let orig_bbox = Rect::new(100.0, 100.0, 200.0, 200.0);
    let img_item = create_sample_image(page_id, 303, orig_bbox);
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // 1. Seleziona l'immagine
    NoteCanvas::handle_selection_down(&mut st, (150.0, 150.0), false);
    NoteCanvas::handle_selection_up(&mut st);
    assert_eq!(st.selection.selected_ids, vec![303]);

    // 2. Drag maniglia BottomRight
    NoteCanvas::handle_selection_down(&mut st, (200.0, 200.0), false);
    NoteCanvas::handle_selection_motion(&mut st, (300.0, 300.0), false);
    NoteCanvas::handle_selection_up(&mut st);

    let scaled_bbox = st.selection.bbox.expect("Scaled bbox");
    assert!(scaled_bbox.width() > 150.0);

    // 3. Esegui Undo
    let undone = st.undo();
    assert!(undone, "Undo must succeed");
    if let ItemKind::Image(ref img) = st.document.pages[0].items[0].kind {
        assert!((img.bbox.min_x - orig_bbox.min_x).abs() < 1e-4);
        assert!((img.bbox.min_y - orig_bbox.min_y).abs() < 1e-4);
        assert!((img.bbox.max_x - orig_bbox.max_x).abs() < 1e-4);
        assert!((img.bbox.max_y - orig_bbox.max_y).abs() < 1e-4);
    } else {
        panic!("Item must be Image");
    }

    // 4. Esegui Redo
    let redone = st.redo();
    assert!(redone, "Redo must succeed");
    if let ItemKind::Image(ref img) = st.document.pages[0].items[0].kind {
        assert!((img.bbox.width() - scaled_bbox.width()).abs() < 1e-3);
    } else {
        panic!("Item must be Image");
    }
}

#[test]
fn test_mixed_selection_scale_handles() {
    let mut doc = Document::new_notebook("Mixed Selection Handle Test");
    let page_id = doc.pages[0].id;
    let stroke_item = create_sample_stroke(10, &[(100.0, 100.0), (150.0, 150.0)]);
    let img_item = create_sample_image(page_id, 20, Rect::new(150.0, 150.0, 250.0, 250.0));
    doc.pages[0].items.push(stroke_item);
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // Seleziona entrambi via rettangolo
    let (mut ids, bbox) = AppSelectionEngine::select_rect(
        &st.document.pages[0],
        &Rect::new(50.0, 50.0, 300.0, 300.0),
    );
    ids.sort();
    assert_eq!(ids, vec![10, 20]);
    st.selection.selected_ids = ids;
    st.selection.bbox = bbox;
    st.selection.page_index = Some(0);

    let base_bbox = bbox.expect("Unified bbox");

    // Ridimensiona con maniglia BottomRight
    NoteCanvas::handle_selection_down(&mut st, (base_bbox.max_x, base_bbox.max_y), false);
    NoteCanvas::handle_selection_motion(
        &mut st,
        (base_bbox.max_x + 50.0, base_bbox.max_y + 50.0),
        false,
    );
    NoteCanvas::handle_selection_up(&mut st);

    // Verifica che entrambi siano stati ridimensionati
    let updated_stroke = st.document.pages[0].items[0].as_stroke().unwrap();
    let updated_img = st.document.pages[0].items[1].as_image().unwrap();
    assert!(updated_stroke.bounding_box().unwrap().width() > 50.0);
    assert!(updated_img.bbox.width() > 100.0);
}

#[test]
fn test_direct_image_click_under_pan_and_zoom() {
    let mut doc = Document::new_notebook("Pan and Zoom Click Test");
    let page_id = doc.pages[0].id;
    let img_item = create_sample_image(page_id, 404, Rect::new(200.0, 200.0, 400.0, 350.0));
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // Viewport con zoom 2.5x e pan offset (150.0, -80.0)
    st.viewport.zoom = 2.5;
    st.viewport.offset_x = 150.0;
    st.viewport.offset_y = -80.0;

    // Coordinate schermo che corrispondono al centro dell'immagine (300.0, 275.0) in document space:
    // screen_x = 300.0 * 2.5 + 150.0 = 900.0
    // screen_y = 275.0 * 2.5 - 80.0 = 607.5
    let screen_pt = (900.0, 607.5);
    let doc_pt = st.viewport.screen_to_document(screen_pt);

    assert!((doc_pt.0 - 300.0).abs() < 1e-4);
    assert!((doc_pt.1 - 275.0).abs() < 1e-4);

    NoteCanvas::handle_selection_down(&mut st, doc_pt, false);
    assert_eq!(st.selection.selected_ids, vec![404]);
    assert!(st.selection.is_dragging);
}

#[test]
fn test_delete_selected_image_and_undo_redo() {
    let mut doc = Document::new_notebook("Delete Image Test");
    let page_id = doc.pages[0].id;
    let img_item = create_sample_image(page_id, 505, Rect::new(100.0, 100.0, 200.0, 200.0));
    doc.pages[0].items.push(img_item);
    doc.pages[0].rebuild_spatial_index();

    let mut st = CanvasState::new();
    st.document = doc;
    st.active_tool = uninote_app::canvas::ActiveTool::Select;

    // Seleziona l'immagine
    NoteCanvas::handle_selection_down(&mut st, (150.0, 150.0), false);
    NoteCanvas::handle_selection_up(&mut st);
    assert_eq!(st.selection.selected_ids, vec![505]);

    // Elimina
    let deleted = NoteCanvas::delete_selected_items(&mut st);
    assert!(deleted, "Image must be deleted");
    assert!(st.document.pages[0].items.is_empty());
    assert_eq!(st.document.pages[0].spatial_index.len(), 0);

    // Undo
    let undone = st.undo();
    assert!(undone, "Undo delete must succeed");
    assert_eq!(st.document.pages[0].items.len(), 1);
    assert_eq!(st.document.pages[0].spatial_index.len(), 1);
    assert_eq!(st.document.pages[0].items[0].id, 505);

    // Redo
    let redone = st.redo();
    assert!(redone, "Redo delete must succeed");
    assert!(st.document.pages[0].items.is_empty());
    assert_eq!(st.document.pages[0].spatial_index.len(), 0);
}
