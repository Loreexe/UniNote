use std::time::Instant;
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, point_in_polygon, SelectionEngine,
    SelectionMode,
};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle,
};

fn make_stroke(id: u64, coords: &[(f64, f64)]) -> Stroke {
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

    let raw_samples = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| InputSample {
            position: (x, y),
            pressure: Some(0.5),
            tilt: None,
            timestamp_ns: 1_000_000 * (i as u64),
            device_kind: DeviceKind::Stylus,
            source_id: 1,
            phase: if i == 0 {
                InputPhase::Down
            } else if i + 1 == coords.len() {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        })
        .collect();

    Stroke {
        id,
        points,
        raw_samples,
        style: StrokeStyle::default(),
        is_dot: false,
        mode: PipelineMode::Baseline,
    }
}

// =========================================================================
// 1. RECT EXTENSIONS & STROKE TRANSLATION
// =========================================================================

#[test]
fn test_rect_contains_point_and_normalization() {
    let r = Rect::new(10.0, 20.0, 100.0, 80.0);
    // Interior point
    assert!(r.contains_point(50.0, 50.0));
    // Boundary points
    assert!(r.contains_point(10.0, 20.0));
    assert!(r.contains_point(100.0, 80.0));
    assert!(r.contains_point(10.0, 50.0));
    assert!(r.contains_point(100.0, 50.0));
    // Outside points
    assert!(!r.contains_point(9.9, 50.0));
    assert!(!r.contains_point(100.1, 50.0));
    assert!(!r.contains_point(50.0, 19.9));
    assert!(!r.contains_point(50.0, 80.1));

    // Inverted/denormalized rectangle
    let r_inv = Rect::new(100.0, 80.0, 10.0, 20.0);
    assert!(r_inv.contains_point(50.0, 50.0));
    assert!(r_inv.contains_point(10.0, 20.0));
    assert!(!r_inv.contains_point(5.0, 50.0));
}

#[test]
fn test_rect_contains_rect() {
    let parent = Rect::new(0.0, 0.0, 200.0, 200.0);
    let child_inner = Rect::new(50.0, 50.0, 150.0, 150.0);
    let child_edge = Rect::new(0.0, 0.0, 200.0, 200.0);
    let child_crossing = Rect::new(-10.0, 50.0, 150.0, 150.0);
    let child_disjoint = Rect::new(250.0, 250.0, 300.0, 300.0);

    assert!(parent.contains_rect(&child_inner));
    assert!(parent.contains_rect(&child_edge));
    assert!(!parent.contains_rect(&child_crossing));
    assert!(!parent.contains_rect(&child_disjoint));

    // Inverted child rect
    let inverted_child = Rect::new(150.0, 150.0, 50.0, 50.0);
    assert!(parent.contains_rect(&inverted_child));
}

#[test]
fn test_rect_intersects_rect() {
    let r1 = Rect::new(10.0, 10.0, 100.0, 100.0);
    let r2 = Rect::new(50.0, 50.0, 150.0, 150.0); // partial overlap
    let r3 = Rect::new(100.0, 10.0, 200.0, 100.0); // touching on edge
    let r4 = Rect::new(101.0, 10.0, 200.0, 100.0); // disjoint

    assert!(r1.intersects_rect(&r2));
    assert!(r2.intersects_rect(&r1));
    assert!(r1.intersects_rect(&r3));
    assert!(!r1.intersects_rect(&r4));
}

#[test]
fn test_rect_translate() {
    let r = Rect::new(10.0, 20.0, 50.0, 60.0);
    let translated = r.translate(15.0, -5.0);

    assert_eq!(translated.min_x, 25.0);
    assert_eq!(translated.min_y, 15.0);
    assert_eq!(translated.max_x, 65.0);
    assert_eq!(translated.max_y, 55.0);
}

#[test]
fn test_stroke_translate() {
    let mut stroke = make_stroke(10, &[(10.0, 20.0), (30.0, 40.0)]);
    assert_eq!(stroke.points[0].x, 10.0);
    assert_eq!(stroke.raw_samples[0].position.0, 10.0);

    stroke.translate(12.5, -7.5);

    assert_eq!(stroke.points[0].x, 22.5);
    assert_eq!(stroke.points[0].y, 12.5);
    assert_eq!(stroke.points[1].x, 42.5);
    assert_eq!(stroke.points[1].y, 32.5);

    assert_eq!(stroke.raw_samples[0].position.0, 22.5);
    assert_eq!(stroke.raw_samples[0].position.1, 12.5);
    assert_eq!(stroke.raw_samples[1].position.0, 42.5);
    assert_eq!(stroke.raw_samples[1].position.1, 32.5);
}

// =========================================================================
// 2. RAY-CASTING POINT-IN-POLYGON
// =========================================================================

#[test]
fn test_point_in_polygon_convex_and_concave() {
    // Convex square
    let square = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    assert!(point_in_polygon(50.0, 50.0, &square));
    assert!(!point_in_polygon(-10.0, 50.0, &square));
    assert!(!point_in_polygon(110.0, 50.0, &square));

    // Concave L-shape
    let l_shape = [
        (0.0, 0.0),
        (50.0, 0.0),
        (50.0, 50.0),
        (100.0, 50.0),
        (100.0, 100.0),
        (0.0, 100.0),
    ];
    // Point inside bottom-left
    assert!(point_in_polygon(25.0, 25.0, &l_shape));
    // Point inside bottom-right
    assert!(point_in_polygon(25.0, 75.0, &l_shape));
    // Point inside cutout bay (60, 20) -> must be outside!
    assert!(!point_in_polygon(60.0, 20.0, &l_shape));

    // Degenerate polygons (< 3 points)
    assert!(!point_in_polygon(10.0, 10.0, &[]));
    assert!(!point_in_polygon(10.0, 10.0, &[(0.0, 0.0), (10.0, 10.0)]));
}

// =========================================================================
// 3. SELECTION ENGINE (LASSO & RECTANGLE)
// =========================================================================

#[test]
fn test_selection_engine_lasso_threshold_boundaries() {
    let lasso = [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

    // Stroke 1: 10 points, exactly 8 inside (80%) -> ACCEPTED
    let mut pts_80 = Vec::new();
    for i in 0..8 {
        pts_80.push((10.0 + i as f64 * 10.0, 50.0));
    }
    pts_80.push((150.0, 50.0));
    pts_80.push((160.0, 50.0));
    let s1 = make_stroke(1, &pts_80);

    // Stroke 2: 10 points, 7 inside (70%) -> REJECTED
    let mut pts_70 = Vec::new();
    for i in 0..7 {
        pts_70.push((10.0 + i as f64 * 10.0, 50.0));
    }
    pts_70.push((140.0, 50.0));
    pts_70.push((150.0, 50.0));
    pts_70.push((160.0, 50.0));
    let s2 = make_stroke(2, &pts_70);

    // Stroke 3: 1 point dot inside (100%) -> ACCEPTED
    let s3 = make_stroke(3, &[(50.0, 50.0)]);

    // Stroke 4: 1 point dot outside (0%) -> REJECTED
    let s4 = make_stroke(4, &[(150.0, 50.0)]);

    let strokes = [&s1, &s2, &s3, &s4];
    let selected = SelectionEngine::select_lasso(strokes, &lasso);

    assert_eq!(selected, vec![1, 3]);
    assert!(is_stroke_selected_by_lasso(&s1, &lasso));
    assert!(!is_stroke_selected_by_lasso(&s2, &lasso));
    assert!(is_stroke_selected_by_lasso(&s3, &lasso));
    assert!(!is_stroke_selected_by_lasso(&s4, &lasso));
}

#[test]
fn test_selection_engine_rect_bounding_box_and_normalization() {
    // User drags inverted rectangle from (200, 200) to (50, 50)
    let rect_inverted = Rect::new(200.0, 200.0, 50.0, 50.0);

    // Stroke entirely enclosed
    let s1 = make_stroke(101, &[(60.0, 60.0), (100.0, 100.0), (140.0, 140.0)]);

    // Stroke with 85% points inside (e.g. 17 inside, 3 outside)
    let mut pts_85 = Vec::new();
    for i in 0..17 {
        pts_85.push((60.0 + i as f64 * 5.0, 100.0));
    }
    pts_85.push((210.0, 100.0));
    pts_85.push((220.0, 100.0));
    pts_85.push((230.0, 100.0));
    let s2 = make_stroke(102, &pts_85);

    // Stroke with only 40% points inside
    let mut pts_40 = Vec::new();
    for i in 0..4 {
        pts_40.push((60.0 + i as f64 * 5.0, 100.0));
    }
    for i in 0..6 {
        pts_40.push((210.0 + i as f64 * 5.0, 100.0));
    }
    let s3 = make_stroke(103, &pts_40);

    let strokes = [&s1, &s2, &s3];
    let selected = SelectionEngine::select_rect(strokes, rect_inverted);

    assert_eq!(selected, vec![101, 102]);
    assert!(is_stroke_selected_by_rect(&s1, &rect_inverted));
    assert!(is_stroke_selected_by_rect(&s2, &rect_inverted));
    assert!(!is_stroke_selected_by_rect(&s3, &rect_inverted));
}

#[test]
fn test_selection_engine_compute_bounding_box() {
    let s1 = make_stroke(1, &[(10.0, 20.0), (50.0, 40.0)]);
    let s2 = make_stroke(2, &[(100.0, 80.0), (150.0, 120.0)]);

    let bbox = SelectionEngine::compute_selection_bounding_box([&s1, &s2]).expect("Unified bbox");

    // bbox should encompass all points plus stroke width margin
    assert!(bbox.min_x <= 10.0);
    assert!(bbox.min_y <= 20.0);
    assert!(bbox.max_x >= 150.0);
    assert!(bbox.max_y >= 120.0);

    let empty_bbox = SelectionEngine::compute_selection_bounding_box(Vec::<&Stroke>::new());
    assert!(empty_bbox.is_none());
}

#[test]
fn test_selection_mode_serde() {
    let lasso = SelectionMode::Lasso;
    let json_lasso = serde_json::to_string(&lasso).unwrap();
    let de_lasso: SelectionMode = serde_json::from_str(&json_lasso).unwrap();
    assert_eq!(lasso, de_lasso);

    let rect = SelectionMode::Rectangle;
    let json_rect = serde_json::to_string(&rect).unwrap();
    let de_rect: SelectionMode = serde_json::from_str(&json_rect).unwrap();
    assert_eq!(rect, de_rect);
}

#[test]
fn test_selection_engine_high_density_performance_120hz() {
    // 1,000 strokes with 10 points each
    let poly = [
        (100.0, 100.0),
        (500.0, 100.0),
        (500.0, 500.0),
        (100.0, 500.0),
    ];

    let mut strokes = Vec::with_capacity(1000);
    for i in 0..1000 {
        let x = (i % 50) as f64 * 15.0;
        let y = (i / 50) as f64 * 25.0;
        strokes.push(make_stroke(i as u64, &[(x, y), (x + 5.0, y + 5.0)]));
    }

    let start = Instant::now();
    let stroke_refs: Vec<&Stroke> = strokes.iter().collect();
    let selected = SelectionEngine::select_lasso(stroke_refs, &poly);
    let elapsed = start.elapsed();

    assert!(!selected.is_empty());
    assert!(
        elapsed.as_millis() < 40,
        "Selection of 1,000 strokes must complete in < 40ms, took {}ms",
        elapsed.as_millis()
    );
}
