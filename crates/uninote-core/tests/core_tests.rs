#![allow(irrefutable_let_patterns)]

use uninote_core::{
    CommandHistory, DocCommand, Document, DocumentFormat, Page, PageBackground, Rect2D, Viewport,
    A4_HEIGHT_PT, A4_WIDTH_PT, PAGE_GAP_PT, PAGE_TOP_MARGIN_PT,
};
use uninote_stroke::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

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

#[test]
fn test_document_format_enum_and_defaults() {
    assert_eq!(DocumentFormat::default(), DocumentFormat::Notebook);

    // Serde JSON roundtrip
    let notebook_json = serde_json::to_string(&DocumentFormat::Notebook).unwrap();
    let de_nb: DocumentFormat = serde_json::from_str(&notebook_json).unwrap();
    assert_eq!(de_nb, DocumentFormat::Notebook);

    let wb_json = serde_json::to_string(&DocumentFormat::Whiteboard).unwrap();
    let de_wb: DocumentFormat = serde_json::from_str(&wb_json).unwrap();
    assert_eq!(de_wb, DocumentFormat::Whiteboard);

    // Backward compatibility: Document serialized without "format" field defaults to Notebook
    let legacy_json = r#"{
        "id": "a0000000-0000-0000-0000-000000000001",
        "title": "Legacy Notebook",
        "version": 1,
        "pages": []
    }"#;
    let legacy_doc: Document = serde_json::from_str(legacy_json).unwrap();
    assert_eq!(legacy_doc.format, DocumentFormat::Notebook);
    assert_eq!(legacy_doc.title, "Legacy Notebook");
}

#[test]
fn test_whiteboard_creation_and_infinite_dimensions() {
    let page = Page::new_whiteboard(PageBackground::Blank);
    assert_eq!(page.index, 0);
    assert!(page.width.is_infinite());
    assert!(page.height.is_infinite());
    assert_eq!(page.background, PageBackground::Blank);
    assert!(page.items.is_empty());

    let wb_doc = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
    assert_eq!(wb_doc.format, DocumentFormat::Whiteboard);
    assert_eq!(wb_doc.pages.len(), 1);
    assert!(wb_doc.pages[0].width.is_infinite());
    assert!(wb_doc.pages[0].height.is_infinite());

    let nb_doc = Document::new_notebook("Test Notebook");
    assert_eq!(nb_doc.format, DocumentFormat::Notebook);
    assert_eq!(nb_doc.pages.len(), 1);
    assert_eq!(nb_doc.pages[0].width, A4_WIDTH_PT);
    assert_eq!(nb_doc.pages[0].height, A4_HEIGHT_PT);
}

#[test]
fn test_viewport_center_on_rect() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);

    // Standard bounding box: center is at (300.0, 350.0)
    let rect = Rect2D::new(100.0, 200.0, 400.0, 300.0);
    let win_w = 1920.0;
    let win_h = 1080.0;
    vp.center_on_rect(rect, win_w, win_h);

    // Center of rect should map directly to center of window
    let center_doc = (rect.x + rect.width / 2.0, rect.y + rect.height / 2.0);
    let center_screen = vp.document_to_screen(center_doc);
    assert!((center_screen.0 - win_w / 2.0).abs() < 1e-6);
    assert!((center_screen.1 - win_h / 2.0).abs() < 1e-6);
    assert!(vp.zoom >= 0.25 && vp.zoom <= 3.0);

    // Degenerate inputs must not panic or cause NaN/Inf
    vp.center_on_rect(Rect2D::new(0.0, 0.0, 0.0, 0.0), win_w, win_h);
    assert_eq!(vp.zoom, 1.0);
    assert!((vp.offset_x - win_w / 2.0).abs() < 1e-6);

    vp.center_on_rect(Rect2D::new(10.0, 10.0, -100.0, 50.0), win_w, win_h);
    assert_eq!(vp.zoom, 1.0);

    vp.center_on_rect(Rect2D::new(0.0, 0.0, f64::NAN, 100.0), win_w, win_h);
    assert_eq!(vp.zoom, 1.0);

    vp.center_on_rect(Rect2D::new(0.0, 0.0, f64::INFINITY, 100.0), win_w, win_h);
    assert_eq!(vp.zoom, 1.0);
}

#[test]
fn test_doc_command_move_items_undo_redo() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let sample_1 = InputSample::new(
        (100.0, 150.0),
        Some(0.5),
        1_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    let sample_2 = InputSample::new(
        (120.0, 180.0),
        Some(0.7),
        2_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    );

    let stroke = Stroke {
        id: 42,
        points: vec![
            StrokePoint {
                x: 100.0,
                y: 150.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1_000_000,
            },
            StrokePoint {
                x: 120.0,
                y: 180.0,
                pressure: 0.7,
                width: 2.5,
                timestamp_ns: 2_000_000,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: vec![sample_1, sample_2],
        is_dot: false,
        mode: PipelineMode::Baseline,
    };

    history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    assert_eq!(doc.pages[0].items.len(), 1);
    let item_id = doc.pages[0].items[0].id;

    // 1. Execute MoveItems (+50.0, -30.0)
    let move_cmd = DocCommand::MoveItems {
        page_id,
        item_ids: vec![item_id],
        delta_x: 50.0,
        delta_y: -30.0,
    };
    history.execute(move_cmd, &mut doc);

    if let uninote_core::ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 150.0);
        assert_eq!(s.points[0].y, 120.0);
        assert_eq!(s.points[1].x, 170.0);
        assert_eq!(s.points[1].y, 150.0);
        assert_eq!(s.raw_samples[0].position, (150.0, 120.0));
        assert_eq!(s.raw_samples[1].position, (170.0, 150.0));
    } else {
        panic!("Expected Stroke item");
    }

    // 2. Undo MoveItems -> restores original coordinates
    assert!(history.undo(&mut doc));
    if let uninote_core::ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 100.0);
        assert_eq!(s.points[0].y, 150.0);
        assert_eq!(s.points[1].x, 120.0);
        assert_eq!(s.points[1].y, 180.0);
        assert_eq!(s.raw_samples[0].position, (100.0, 150.0));
        assert_eq!(s.raw_samples[1].position, (120.0, 180.0));
    } else {
        panic!("Expected Stroke item");
    }

    // 3. Redo MoveItems -> applies translation again
    assert!(history.redo(&mut doc));
    if let uninote_core::ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 150.0);
        assert_eq!(s.points[0].y, 120.0);
        assert_eq!(s.points[1].x, 170.0);
        assert_eq!(s.points[1].y, 150.0);
    } else {
        panic!("Expected Stroke item");
    }
}

#[test]
fn test_page_background_pdf_page_serde_roundtrip() {
    let bg = PageBackground::PdfPage {
        source_path: "/tmp/sample.pdf".to_string(),
        page_index: 2,
        original_width: 960.0,
        original_height: 540.0,
    };
    let json = serde_json::to_string(&bg).expect("Serializzazione JSON");
    let deserialized: PageBackground = serde_json::from_str(&json).expect("Deserializzazione JSON");
    assert_eq!(bg, deserialized);
}

#[test]
fn test_heterogeneous_page_dimensions_page_rect_and_y_lookup() {
    let mut doc = Document::new_empty("Slide Deck e Appunti");
    // Pagina 0: default A4 (595.28 x 841.89)
    assert_eq!(doc.pages[0].height, A4_HEIGHT_PT);

    // Pagina 1: Slide 16:9 (960.0 x 540.0)
    let p1_id = doc.add_pdf_page("/tmp/deck.pdf", 0, 960.0, 540.0);
    assert_eq!(doc.pages.len(), 2);
    assert_eq!(doc.pages[1].id, p1_id);
    assert_eq!(doc.pages[1].width, 960.0);
    assert_eq!(doc.pages[1].height, 540.0);

    // Pagina 2: Formato Letter (612.0 x 792.0)
    let p2_id = doc.add_pdf_page("/tmp/letter.pdf", 0, 612.0, 792.0);
    assert_eq!(doc.pages.len(), 3);
    assert_eq!(doc.pages[2].id, p2_id);

    // Verifica page_rect
    let r0 = doc.page_rect(0).unwrap();
    assert_eq!(r0.x, 0.0);
    assert_eq!(r0.y, PAGE_TOP_MARGIN_PT);
    assert_eq!(r0.width, A4_WIDTH_PT);
    assert_eq!(r0.height, A4_HEIGHT_PT);

    let r1 = doc.page_rect(1).unwrap();
    let expected_p1_top = PAGE_TOP_MARGIN_PT + A4_HEIGHT_PT + PAGE_GAP_PT;
    assert_eq!(r1.x, 0.0);
    assert_eq!(r1.y, expected_p1_top);
    assert_eq!(r1.width, 960.0);
    assert_eq!(r1.height, 540.0);

    let r2 = doc.page_rect(2).unwrap();
    let expected_p2_top = expected_p1_top + 540.0 + PAGE_GAP_PT;
    assert_eq!(r2.x, 0.0);
    assert_eq!(r2.y, expected_p2_top);
    assert_eq!(r2.width, 612.0);
    assert_eq!(r2.height, 792.0);

    // Fuori range
    assert!(doc.page_rect(3).is_none());

    // Verifica find_page_index_at_y
    assert_eq!(doc.find_page_index_at_y(0.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(expected_p1_top - 5.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(expected_p1_top + 10.0), Some(1));
    assert_eq!(doc.find_page_index_at_y(expected_p2_top + 20.0), Some(2));
    assert_eq!(doc.find_page_index_at_y(expected_p2_top + 99999.0), Some(2));
}
