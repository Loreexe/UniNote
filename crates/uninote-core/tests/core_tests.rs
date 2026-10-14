use uninote_core::{
    CommandHistory, DocCommand, Document, PageBackground, Viewport, A4_HEIGHT_PT, A4_WIDTH_PT,
    PAGE_GAP_PT, PAGE_TOP_MARGIN_PT,
};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

#[test]
fn test_screen_to_document_and_back_roundtrip() {
    let vp = Viewport::new(100.0, 50.0, 1.5);

    let screen_pt = (250.0, 350.0);
    let doc_pt = vp.screen_to_document(screen_pt);
    let roundtrip = vp.document_to_screen(doc_pt);

    assert!((roundtrip.0 - screen_pt.0).abs() < 1e-6);
    assert!((roundtrip.1 - screen_pt.1).abs() < 1e-6);
}

#[test]
fn test_pointer_centered_zoom_invariance() {
    let mut vp = Viewport::new(120.0, 80.0, 1.0);
    let pivot = (300.0, 400.0);

    let doc_before = vp.screen_to_document(pivot);

    // Zoom 2x centrato sul pivot
    vp.zoom_at(pivot, 2.0);
    assert!((vp.zoom - 2.0).abs() < 1e-6);

    let doc_after = vp.screen_to_document(pivot);

    // Il punto del documento sotto il puntatore deve essere identico
    assert!((doc_before.0 - doc_after.0).abs() < 1e-6);
    assert!((doc_before.1 - doc_after.1).abs() < 1e-6);
}

#[test]
fn test_page_rect_and_page_index_lookup() {
    let p0 = Viewport::page_rect(0);
    assert_eq!(p0.x, 0.0);
    assert_eq!(p0.y, PAGE_TOP_MARGIN_PT);
    assert_eq!(p0.width, A4_WIDTH_PT);
    assert_eq!(p0.height, A4_HEIGHT_PT);

    let p1 = Viewport::page_rect(1);
    assert_eq!(p1.y, PAGE_TOP_MARGIN_PT + A4_HEIGHT_PT + PAGE_GAP_PT);

    // Coordinate Y appartenenti alla pagina 0
    assert_eq!(
        Viewport::find_page_index_at_y(PAGE_TOP_MARGIN_PT + 100.0, 3),
        0
    );
    // Coordinate Y appartenenti alla pagina 1
    assert_eq!(Viewport::find_page_index_at_y(p1.y + 100.0, 3), 1);
}

#[test]
fn test_command_history_undo_redo_monotonic_revisions() {
    let mut doc = Document::new_empty("Quaderno Test");
    let mut history = CommandHistory::new(50);
    let page_id = doc.pages[0].id;

    assert_eq!(history.current_revision, 0);
    assert!(!history.can_undo());
    assert!(!history.can_redo());
    assert!(!history.is_dirty());

    // 1. Aggiungi stroke
    let stroke = Stroke {
        id: 1,
        points: vec![
            StrokePoint {
                x: 10.0,
                y: 10.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1000,
            },
            StrokePoint {
                x: 20.0,
                y: 20.0,
                pressure: 0.6,
                width: 2.2,
                timestamp_ns: 2000,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
    };

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke.clone(),
        },
        &mut doc,
    );

    assert_eq!(history.current_revision, 1);
    assert!(history.can_undo());
    assert!(!history.can_redo());
    assert!(history.is_dirty());
    assert_eq!(doc.pages[0].items.len(), 1);

    // 2. Undo
    assert!(history.undo(&mut doc));
    assert_eq!(history.current_revision, 2);
    assert!(!history.can_undo());
    assert!(history.can_redo());
    assert_eq!(doc.pages[0].items.len(), 0);

    // 3. Redo
    assert!(history.redo(&mut doc));
    assert_eq!(history.current_revision, 3);
    assert!(history.can_undo());
    assert!(!history.can_redo());
    assert_eq!(doc.pages[0].items.len(), 1);

    // 4. Erase
    let item_to_erase = doc.pages[0].items[0].clone();
    history.execute(
        DocCommand::EraseItems {
            page_id,
            erased_items: vec![item_to_erase],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 0);

    // 5. Undo Erase (ripristino)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 1);
}

#[test]
fn test_add_page_command() {
    let mut doc = Document::new_empty("Pagine Test");
    let mut history = CommandHistory::new(50);
    assert_eq!(doc.pages.len(), 1);

    let new_page = uninote_core::Page::new_a4(1, PageBackground::Blank);
    history.execute(DocCommand::AddPage { page: new_page }, &mut doc);
    assert_eq!(doc.pages.len(), 2);

    history.undo(&mut doc);
    assert_eq!(doc.pages.len(), 1);

    history.redo(&mut doc);
    assert_eq!(doc.pages.len(), 2);
}
