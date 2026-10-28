#![allow(irrefutable_let_patterns)]

use uninote_core::{
    CommandHistory, DocCommand, Document, ItemKind, PageBackground, Rect2D, Viewport,
};
use uninote_stroke::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

#[test]
fn test_viewport_center_on_rect_nan_in_position_coords() {
    let mut vp = Viewport::new(100.0, 100.0, 1.0);
    let win_w = 1920.0;
    let win_h = 1080.0;

    // rect.x is NaN
    vp.center_on_rect(Rect2D::new(f64::NAN, 10.0, 200.0, 200.0), win_w, win_h);
    println!(
        "vp after NaN x: offset_x={}, offset_y={}, zoom={}",
        vp.offset_x, vp.offset_y, vp.zoom
    );
    assert!(
        vp.offset_x.is_finite(),
        "offset_x should be finite when rect.x is NaN, but got {}",
        vp.offset_x
    );
    assert!(
        vp.offset_y.is_finite(),
        "offset_y should be finite when rect.x is NaN, but got {}",
        vp.offset_y
    );
    assert!(vp.zoom.is_finite(), "zoom should be finite");

    // rect.y is NaN
    vp.center_on_rect(Rect2D::new(10.0, f64::NAN, 200.0, 200.0), win_w, win_h);
    assert!(
        vp.offset_x.is_finite(),
        "offset_x should be finite when rect.y is NaN, but got {}",
        vp.offset_x
    );
    assert!(
        vp.offset_y.is_finite(),
        "offset_y should be finite when rect.y is NaN, but got {}",
        vp.offset_y
    );

    // rect.x is INFINITY
    vp.center_on_rect(Rect2D::new(f64::INFINITY, 10.0, 200.0, 200.0), win_w, win_h);
    assert!(
        vp.offset_x.is_finite(),
        "offset_x should be finite when rect.x is INFINITY, but got {}",
        vp.offset_x
    );
    assert!(
        vp.offset_y.is_finite(),
        "offset_y should be finite when rect.x is INFINITY, but got {}",
        vp.offset_y
    );

    // rect.y is NEG_INFINITY
    vp.center_on_rect(
        Rect2D::new(10.0, f64::NEG_INFINITY, 200.0, 200.0),
        win_w,
        win_h,
    );
    assert!(
        vp.offset_x.is_finite(),
        "offset_x should be finite when rect.y is NEG_INFINITY, but got {}",
        vp.offset_x
    );
    assert!(
        vp.offset_y.is_finite(),
        "offset_y should be finite when rect.y is NEG_INFINITY, but got {}",
        vp.offset_y
    );
}

#[test]
fn test_viewport_center_on_rect_infinite_window_dimensions() {
    let mut vp = Viewport::new(100.0, 100.0, 1.0);
    let rect = Rect2D::new(0.0, 0.0, 200.0, 200.0);

    // win_w is INFINITY
    vp.center_on_rect(rect, f64::INFINITY, 1080.0);
    println!(
        "vp after win_w=INFINITY: offset_x={}, offset_y={}, zoom={}",
        vp.offset_x, vp.offset_y, vp.zoom
    );
    assert!(
        vp.offset_x.is_finite(),
        "offset_x must be finite even if win_w is INFINITY, but got {}",
        vp.offset_x
    );

    // win_h is INFINITY
    vp.center_on_rect(rect, 1920.0, f64::INFINITY);
    println!(
        "vp after win_h=INFINITY: offset_x={}, offset_y={}, zoom={}",
        vp.offset_x, vp.offset_y, vp.zoom
    );
    assert!(
        vp.offset_y.is_finite(),
        "offset_y must be finite even if win_h is INFINITY, but got {}",
        vp.offset_y
    );
}

#[test]
fn test_viewport_center_on_rect_extreme_coordinates() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);
    let win_w = 1920.0;
    let win_h = 1080.0;

    // Extreme coordinates: +/- 10^9
    let extreme_rect = Rect2D::new(1e9, -1e9, 500.0, 500.0);
    vp.center_on_rect(extreme_rect, win_w, win_h);

    let center_doc = (
        extreme_rect.x + extreme_rect.width / 2.0,
        extreme_rect.y + extreme_rect.height / 2.0,
    );
    let center_screen = vp.document_to_screen(center_doc);

    assert!(
        (center_screen.0 - win_w / 2.0).abs() < 1e-4,
        "center_x projection must match win_w/2"
    );
    assert!(
        (center_screen.1 - win_h / 2.0).abs() < 1e-4,
        "center_y projection must match win_h/2"
    );
    assert!(vp.zoom >= 0.25 && vp.zoom <= 3.0);
}

#[test]
fn test_viewport_zoom_clamping_limits() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);
    let win_w = 1000.0;
    let win_h = 1000.0;

    // Huge rect (10^9 x 10^9) -> scale would be 1000/10^9 = 10^-6 -> must clamp to min 0.25
    vp.center_on_rect(Rect2D::new(0.0, 0.0, 1e9, 1e9), win_w, win_h);
    assert_eq!(vp.zoom, 0.25);

    // Tiny rect (0.001 x 0.001) -> scale would be 1000/0.001 = 10^6 -> must clamp to max 3.0
    vp.center_on_rect(Rect2D::new(0.0, 0.0, 0.001, 0.001), win_w, win_h);
    assert_eq!(vp.zoom, 3.0);
}

#[test]
fn test_viewport_projection_accuracy() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);
    let win_w = 1440.0;
    let win_h = 900.0;

    // Test a variety of rect sizes and positions
    let rects = [
        Rect2D::new(0.0, 0.0, 100.0, 100.0),
        Rect2D::new(-500.0, 200.0, 800.0, 600.0),
        Rect2D::new(1234.56, -789.12, 345.67, 890.12),
        Rect2D::new(-10000.0, -10000.0, 5000.0, 15000.0),
    ];

    for r in rects {
        vp.center_on_rect(r, win_w, win_h);
        let center_doc = (r.x + r.width / 2.0, r.y + r.height / 2.0);
        let center_screen = vp.document_to_screen(center_doc);

        assert!(
            (center_screen.0 - win_w / 2.0).abs() < 1e-5,
            "X projection mismatch for rect {:?}: got {}, expected {}",
            r,
            center_screen.0,
            win_w / 2.0
        );
        assert!(
            (center_screen.1 - win_h / 2.0).abs() < 1e-5,
            "Y projection mismatch for rect {:?}: got {}, expected {}",
            r,
            center_screen.1,
            win_h / 2.0
        );
    }
}

#[test]
fn test_move_items_1000_undo_redo_cycles_drift() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(100);

    let test_cases = [
        (
            100.123456789,
            200.987654321,
            33.33333333333333,
            -77.77777777777777,
        ),
        (0.1, 0.2, 0.3, 0.4),
        (1e6, -1e6, 0.0001, -0.0001),
        (
            std::f64::consts::PI,
            std::f64::consts::E,
            std::f64::consts::SQRT_2,
            std::f64::consts::LN_2,
        ),
    ];

    for (case_idx, &(orig_x, orig_y, dx, dy)) in test_cases.iter().enumerate() {
        let sample = InputSample::new(
            (orig_x, orig_y),
            Some(0.5),
            1_000_000,
            DeviceKind::Stylus,
            InputPhase::Down,
        );
        let stroke = Stroke {
            id: (case_idx + 1) as u64,
            points: vec![StrokePoint {
                x: orig_x,
                y: orig_y,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1_000_000,
            }],
            style: StrokeStyle::default(),
            raw_samples: vec![sample],
            is_dot: false,
            mode: PipelineMode::Baseline,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        };

        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
        let item_id = doc.pages[0].items.last().unwrap().id;

        // Execute MoveItems
        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: vec![item_id],
                delta_x: dx,
                delta_y: dy,
            },
            &mut doc,
        );

        // Run 1000 undo/redo cycles
        for cycle in 0..1000 {
            let undone = history.undo(&mut doc);
            assert!(
                undone,
                "Undo failed at cycle {} for case {}",
                cycle, case_idx
            );

            let redone = history.redo(&mut doc);
            assert!(
                redone,
                "Redo failed at cycle {} for case {}",
                cycle, case_idx
            );
        }

        // Return to undone state
        history.undo(&mut doc);

        let item = doc.pages[0]
            .items
            .iter()
            .find(|it| it.id == item_id)
            .unwrap();
        let ItemKind::Stroke(ref s) = item.kind else {
            panic!("expected stroke");
        };
        let diff_x = (s.points[0].x - orig_x).abs();
        let diff_y = (s.points[0].y - orig_y).abs();
        let raw_diff_x = (s.raw_samples[0].position.0 - orig_x).abs();
        let raw_diff_y = (s.raw_samples[0].position.1 - orig_y).abs();

        println!(
            "Case {}: 1000 Undo/Redo cycles drift: diff_x = {:e}, diff_y = {:e}",
            case_idx, diff_x, diff_y
        );

        assert!(
            diff_x < 1e-12,
            "Drift detected on stroke point X in case {}: diff = {:e}",
            case_idx,
            diff_x
        );
        assert!(
            diff_y < 1e-12,
            "Drift detected on stroke point Y in case {}: diff = {:e}",
            case_idx,
            diff_y
        );
        assert!(
            raw_diff_x < 1e-12,
            "Drift detected on raw sample X in case {}: diff = {:e}",
            case_idx,
            raw_diff_x
        );
        assert!(
            raw_diff_y < 1e-12,
            "Drift detected on raw sample Y in case {}: diff = {:e}",
            case_idx,
            raw_diff_y
        );
    }
}

#[test]
fn test_move_items_edge_cases() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(100);

    // 1. MoveItems with empty item_ids should be a no-op, not crash
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![],
            delta_x: 10.0,
            delta_y: 20.0,
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 0);
    assert!(history.undo(&mut doc));
    assert!(history.redo(&mut doc));

    // 2. MoveItems with non-existent item_ids
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![999999],
            delta_x: 10.0,
            delta_y: 20.0,
        },
        &mut doc,
    );
    assert!(history.undo(&mut doc));
    assert!(history.redo(&mut doc));

    // 3. MoveItems with multiple items and different stroke styles
    let s1 = Stroke {
        id: 1,
        points: vec![StrokePoint {
            x: 0.0,
            y: 0.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1,
        }],
        style: StrokeStyle::default(),
        raw_samples: vec![],
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    let s2 = Stroke {
        id: 2,
        points: vec![
            StrokePoint {
                x: 10.0,
                y: 10.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1,
            },
            StrokePoint {
                x: 20.0,
                y: 20.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 2,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: vec![],
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s1,
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s2,
        },
        &mut doc,
    );

    let id1 = doc.pages[0].items[0].id;
    let id2 = doc.pages[0].items[1].id;

    // Move both items
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![id1, id2],
            delta_x: 5.0,
            delta_y: -5.0,
        },
        &mut doc,
    );

    let it0 = &doc.pages[0].items[0];
    let it1 = &doc.pages[0].items[1];
    let ItemKind::Stroke(ref s0) = it0.kind else {
        panic!("expected stroke");
    };
    assert_eq!(s0.points[0].x, 5.0);
    assert_eq!(s0.points[0].y, -5.0);

    let ItemKind::Stroke(ref s1_item) = it1.kind else {
        panic!("expected stroke");
    };
    assert_eq!(s1_item.points[0].x, 15.0);
    assert_eq!(s1_item.points[0].y, 5.0);
    assert_eq!(s1_item.points[1].x, 25.0);
    assert_eq!(s1_item.points[1].y, 15.0);

    // Undo moves both back
    assert!(history.undo(&mut doc));
    let it0_undone = &doc.pages[0].items[0];
    let it1_undone = &doc.pages[0].items[1];
    let ItemKind::Stroke(ref s0_undone) = it0_undone.kind else {
        panic!("expected stroke");
    };
    assert_eq!(s0_undone.points[0].x, 0.0);
    assert_eq!(s0_undone.points[0].y, 0.0);

    let ItemKind::Stroke(ref s1_undone) = it1_undone.kind else {
        panic!("expected stroke");
    };
    assert_eq!(s1_undone.points[0].x, 10.0);
    assert_eq!(s1_undone.points[0].y, 10.0);
    assert_eq!(s1_undone.points[1].x, 20.0);
    assert_eq!(s1_undone.points[1].y, 20.0);
}

#[test]
fn test_move_items_nan_delta_corrupts_coordinates() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(100);

    let stroke = Stroke {
        id: 1,
        points: vec![StrokePoint {
            x: 10.0,
            y: 10.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1,
        }],
        style: StrokeStyle::default(),
        raw_samples: vec![],
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    let item_id = doc.pages[0].items[0].id;

    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![item_id],
            delta_x: f64::NAN,
            delta_y: 10.0,
        },
        &mut doc,
    );

    let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind else {
        panic!("expected stroke");
    };
    println!("Stroke point X after NaN MoveItems: {}", s.points[0].x);
    assert!(
        s.points[0].x.is_finite(),
        "Stroke point X should remain finite even with NaN delta, but got {}",
        s.points[0].x
    );
}
