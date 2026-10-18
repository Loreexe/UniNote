use std::f64::consts::PI;
use uninote_stroke::geometry::{catmull_rom_to_bezier, CubicBezierSegment};
use uninote_stroke::pipeline::StrokeBuilder;
use uninote_stroke::shape::{recognize_shape, snap_line_orthogonal};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

fn sample_point(x: f64, y: f64, width: f64) -> StrokePoint {
    StrokePoint {
        x,
        y,
        pressure: 0.5,
        width,
        timestamp_ns: 0,
    }
}

fn make_test_curve() -> Vec<StrokePoint> {
    (0..25)
        .map(|i| {
            let t = i as f64;
            StrokePoint {
                x: 100.0 + t * 12.0,
                y: 150.0 + (t * 0.4).sin() * 50.0,
                pressure: 0.3 + (t / 50.0),
                width: 2.0 + (t / 15.0),
                timestamp_ns: (i * 1000) as u64,
            }
        })
        .collect()
}

fn make_dyadic_curve() -> Vec<StrokePoint> {
    (0..10)
        .map(|i| {
            let t = i as f64;
            StrokePoint {
                x: 100.0 + t * 12.0,
                y: 150.0 + t * 6.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: (i * 1000) as u64,
            }
        })
        .collect()
}

// =========================================================================
// 1. BOUNDING BOX CACHING & O(1) FALLBACK
// =========================================================================

#[test]
fn test_cached_bbox_matches_exact_computation() {
    let pts = make_test_curve();
    let mut stroke = Stroke::with_points(1, StrokeStyle::default(), pts.clone());

    assert!(stroke.cached_bbox.is_some());
    let cached = stroke.cached_bbox.unwrap();
    let computed = stroke.compute_bounding_box().unwrap();

    assert_eq!(cached, computed);
    assert_eq!(stroke.bounding_box(), Some(computed));

    // Fallback verification when cache is None
    stroke.cached_bbox = None;
    assert_eq!(stroke.bounding_box(), Some(computed));
}

#[test]
fn test_cached_bbox_empty_stroke() {
    let stroke = Stroke::new(1, StrokeStyle::default(), PipelineMode::default());
    assert_eq!(stroke.cached_bbox, None);
    assert_eq!(stroke.bounding_box(), None);
}

#[test]
fn test_cached_bbox_single_dot() {
    let mut stroke = Stroke::with_points(
        1,
        StrokeStyle::default(),
        vec![sample_point(50.0, 50.0, 3.0)],
    );
    stroke.is_dot = true;
    stroke.recompute_cache();

    let bbox = stroke.bounding_box().expect("Dot bbox must exist");
    assert!(bbox.contains_point(50.0, 50.0));
    assert_eq!(bbox.min_x, 50.0 - 1.5);
    assert_eq!(bbox.max_x, 50.0 + 1.5);
    assert_eq!(bbox.min_y, 50.0 - 1.5);
    assert_eq!(bbox.max_y, 50.0 + 1.5);
}

// =========================================================================
// 2. BEZIER SEGMENTS CACHING
// =========================================================================

#[test]
fn test_cached_bezier_matches_catmull_rom() {
    let pts = make_test_curve();
    let stroke = Stroke::with_points(1, StrokeStyle::default(), pts.clone());

    assert!(!stroke.cached_bezier.is_empty());
    assert_eq!(stroke.cached_bezier.len(), pts.len() - 1);

    let direct_bezier = catmull_rom_to_bezier(&pts);
    assert_eq!(stroke.cached_bezier, direct_bezier);
    assert_eq!(stroke.bezier_segments(), &direct_bezier[..]);
}

#[test]
fn test_cached_bezier_baseline_mode_is_empty() {
    let pts = make_test_curve();
    let stroke =
        Stroke::with_points_and_mode(1, StrokeStyle::default(), PipelineMode::Baseline, pts);
    assert!(stroke.cached_bezier.is_empty());
}

#[test]
fn test_ensure_cache_repopulates_unwarmed_stroke() {
    let pts = make_test_curve();
    let mut stroke = Stroke {
        id: 1,
        style: StrokeStyle::default(),
        points: pts.clone(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    assert!(stroke.cached_bbox.is_none());
    assert!(stroke.cached_bezier.is_empty());

    stroke.ensure_cache();

    assert!(stroke.cached_bbox.is_some());
    assert!(!stroke.cached_bezier.is_empty());
    assert_eq!(stroke.cached_bezier, catmull_rom_to_bezier(&pts));
}

// =========================================================================
// 3. CUBIC BEZIER SEGMENT TRANSLATION
// =========================================================================

#[test]
fn test_cubic_bezier_segment_translate() {
    let mut seg = CubicBezierSegment {
        p0: (10.0, 20.0),
        cp1: (15.0, 25.0),
        cp2: (25.0, 35.0),
        p1: (30.0, 40.0),
        width0: 2.0,
        width1: 4.0,
    };

    seg.translate(5.0, -10.0);

    assert_eq!(seg.p0, (15.0, 10.0));
    assert_eq!(seg.cp1, (20.0, 15.0));
    assert_eq!(seg.cp2, (30.0, 25.0));
    assert_eq!(seg.p1, (35.0, 30.0));
    assert_eq!(seg.width0, 2.0);
    assert_eq!(seg.width1, 4.0);
}

// =========================================================================
// 4. IN-PLACE TRANSLATION SYMMETRY & 10,000 CYCLE DRIFT IMMUNITY
// =========================================================================

#[test]
fn test_stroke_translate_in_place_bezier_and_bbox_invariance_10k_cycles() {
    let pts = make_test_curve();
    let mut stroke = Stroke::with_points(1, StrokeStyle::default(), pts.clone());

    // Verify 1 translation step is numerically identical to recomputing
    let dx = 42.5;
    let dy = -18.75;
    stroke.translate(dx, dy);

    let recomputed_bbox = stroke.compute_bounding_box().unwrap();
    let recomputed_bezier = catmull_rom_to_bezier(&stroke.points);

    assert!(
        (stroke.cached_bbox.unwrap().min_x - recomputed_bbox.min_x).abs() < 1e-12,
        "BBox min_x must match recomputed"
    );
    assert!(
        (stroke.cached_bbox.unwrap().max_x - recomputed_bbox.max_x).abs() < 1e-12,
        "BBox max_x must match recomputed"
    );

    for (cached_seg, recomp_seg) in stroke.cached_bezier.iter().zip(recomputed_bezier.iter()) {
        assert!((cached_seg.p0.0 - recomp_seg.p0.0).abs() < 1e-12);
        assert!((cached_seg.p0.1 - recomp_seg.p0.1).abs() < 1e-12);
        assert!((cached_seg.cp1.0 - recomp_seg.cp1.0).abs() < 1e-12);
        assert!((cached_seg.cp1.1 - recomp_seg.cp1.1).abs() < 1e-12);
        assert!((cached_seg.cp2.0 - recomp_seg.cp2.0).abs() < 1e-12);
        assert!((cached_seg.cp2.1 - recomp_seg.cp2.1).abs() < 1e-12);
        assert!((cached_seg.p1.0 - recomp_seg.p1.0).abs() < 1e-12);
        assert!((cached_seg.p1.1 - recomp_seg.p1.1).abs() < 1e-12);
    }

    // Run 10,000 back-and-forth cycles with dyadic offsets on exact dyadic stroke
    let dyadic_pts = make_dyadic_curve();
    let mut dyadic_stroke = Stroke::with_points(2, StrokeStyle::default(), dyadic_pts);
    let orig_dyadic_points = dyadic_stroke.points.clone();
    let orig_dyadic_bbox = dyadic_stroke.cached_bbox.unwrap();
    let orig_dyadic_bezier = dyadic_stroke.cached_bezier.clone();

    let step_x = 12.5;
    let step_y = -7.25;

    for _ in 0..10_000 {
        dyadic_stroke.translate(step_x, step_y);
        dyadic_stroke.translate(-step_x, -step_y);
    }

    // Verify coordinate drift is strictly < 1e-14
    for (p_curr, p_orig) in dyadic_stroke.points.iter().zip(orig_dyadic_points.iter()) {
        assert!(
            (p_curr.x - p_orig.x).abs() < 1e-14,
            "Point X drift exceeded 1e-14: diff={:e}",
            (p_curr.x - p_orig.x).abs()
        );
        assert!(
            (p_curr.y - p_orig.y).abs() < 1e-14,
            "Point Y drift exceeded 1e-14: diff={:e}",
            (p_curr.y - p_orig.y).abs()
        );
    }

    let final_bbox = dyadic_stroke.cached_bbox.unwrap();
    assert!((final_bbox.min_x - orig_dyadic_bbox.min_x).abs() < 1e-14);
    assert!((final_bbox.max_x - orig_dyadic_bbox.max_x).abs() < 1e-14);
    assert!((final_bbox.min_y - orig_dyadic_bbox.min_y).abs() < 1e-14);
    assert!((final_bbox.max_y - orig_dyadic_bbox.max_y).abs() < 1e-14);

    for (b_curr, b_orig) in dyadic_stroke
        .cached_bezier
        .iter()
        .zip(orig_dyadic_bezier.iter())
    {
        assert!((b_curr.p0.0 - b_orig.p0.0).abs() < 1e-14);
        assert!((b_curr.p0.1 - b_orig.p0.1).abs() < 1e-14);
        assert!((b_curr.cp1.0 - b_orig.cp1.0).abs() < 1e-14);
        assert!((b_curr.cp1.1 - b_orig.cp1.1).abs() < 1e-14);
        assert!((b_curr.cp2.0 - b_orig.cp2.0).abs() < 1e-14);
        assert!((b_curr.cp2.1 - b_orig.cp2.1).abs() < 1e-14);
        assert!((b_curr.p1.0 - b_orig.p1.0).abs() < 1e-14);
        assert!((b_curr.p1.1 - b_orig.p1.1).abs() < 1e-14);
    }

    // Also run 10,000 cycles on transcendental sine curve to verify IEEE-754 precision bound (< 1e-12)
    let mut sine_stroke = Stroke::with_points(3, StrokeStyle::default(), pts);
    let orig_sine_bezier = sine_stroke.cached_bezier.clone();
    for _ in 0..10_000 {
        sine_stroke.translate(step_x, step_y);
        sine_stroke.translate(-step_x, -step_y);
    }
    for (b_curr, b_orig) in sine_stroke
        .cached_bezier
        .iter()
        .zip(orig_sine_bezier.iter())
    {
        assert!((b_curr.p0.0 - b_orig.p0.0).abs() < 1e-12);
        assert!((b_curr.p0.1 - b_orig.p0.1).abs() < 1e-12);
        assert!((b_curr.cp1.0 - b_orig.cp1.0).abs() < 1e-12);
        assert!((b_curr.cp1.1 - b_orig.cp1.1).abs() < 1e-12);
        assert!((b_curr.cp2.0 - b_orig.cp2.0).abs() < 1e-12);
        assert!((b_curr.cp2.1 - b_orig.cp2.1).abs() < 1e-12);
        assert!((b_curr.p1.0 - b_orig.p1.0).abs() < 1e-12);
        assert!((b_curr.p1.1 - b_orig.p1.1).abs() < 1e-12);
    }
}

// =========================================================================
// 5. PIPELINE & BUILDER CACHE POPULATION
// =========================================================================

#[test]
fn test_stroke_builder_finish_populates_cache() {
    let mut builder =
        StrokeBuilder::new(42, StrokeStyle::default(), PipelineMode::CatmullRomSpline);

    builder.process_sample(InputSample::new(
        (10.0, 10.0),
        Some(0.4),
        1000,
        DeviceKind::Stylus,
        InputPhase::Down,
    ));
    builder.process_sample(InputSample::new(
        (50.0, 60.0),
        Some(0.6),
        2000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));
    builder.process_sample(InputSample::new(
        (100.0, 120.0),
        Some(0.8),
        3000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));
    builder.process_sample(InputSample::new(
        (150.0, 180.0),
        Some(0.7),
        4000,
        DeviceKind::Stylus,
        InputPhase::Up,
    ));

    let stroke = builder.finish();

    assert!(
        stroke.cached_bbox.is_some(),
        "cached_bbox must be populated on finish"
    );
    assert!(
        !stroke.cached_bezier.is_empty(),
        "cached_bezier must be populated on finish"
    );
    assert_eq!(stroke.cached_bezier.len(), stroke.points.len() - 1);
    assert_eq!(stroke.bounding_box(), stroke.compute_bounding_box());
}

#[test]
fn test_stroke_builder_incremental_bbox_during_down_and_move() {
    let mut builder = StrokeBuilder::new(99, StrokeStyle::default(), PipelineMode::Baseline);

    assert!(builder.stroke.cached_bbox.is_none());

    builder.process_sample(InputSample::new(
        (20.0, 30.0),
        Some(0.5),
        1000,
        DeviceKind::Stylus,
        InputPhase::Down,
    ));

    assert!(builder.stroke.cached_bbox.is_some());
    let b1 = builder.stroke.cached_bbox.unwrap();
    assert!(b1.contains_point(20.0, 30.0));

    builder.process_sample(InputSample::new(
        (200.0, 300.0),
        Some(0.5),
        2000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));

    let b2 = builder.stroke.cached_bbox.unwrap();
    assert!(b2.contains_point(20.0, 30.0));
    assert!(b2.contains_point(200.0, 300.0));
    assert!(b2.max_x >= 200.0);
    assert!(b2.max_y >= 300.0);
}

// =========================================================================
// 6. SHAPE RECOGNITION CACHE POPULATION
// =========================================================================

#[test]
fn test_shape_recognition_populates_cache() {
    // 1. Line snap
    let line_pts = vec![
        sample_point(10.0, 100.0, 2.5),
        sample_point(100.0, 101.0, 2.5),
        sample_point(200.0, 103.0, 2.5),
    ];
    let line_stroke = Stroke::with_points(1, StrokeStyle::default(), line_pts);
    let snapped = snap_line_orthogonal(&line_stroke).expect("Must snap horizontal line");

    assert!(
        snapped.cached_bbox.is_some(),
        "Snapped line must have cached_bbox"
    );
    assert_eq!(snapped.bounding_box(), snapped.compute_bounding_box());

    // 2. Circle shape recognition
    let mut circle_pts = Vec::new();
    let center = (200.0, 200.0);
    let radius = 60.0;
    for i in 0..=48 {
        let theta = 2.0 * PI * (i as f64 / 48.0);
        circle_pts.push(sample_point(
            center.0 + radius * theta.cos(),
            center.1 + radius * theta.sin(),
            3.0,
        ));
    }
    let circle_stroke = Stroke::with_points(2, StrokeStyle::default(), circle_pts);
    let recognized = recognize_shape(&circle_stroke).expect("Must recognize circle");

    assert!(
        recognized.cached_bbox.is_some(),
        "Recognized shape must have cached_bbox"
    );
    assert_eq!(recognized.bounding_box(), recognized.compute_bounding_box());
}

// =========================================================================
// 7. SERDE SKIP & CUSTOM PARTIALEQ
// =========================================================================

#[test]
fn test_serde_skip_preserves_clean_json_and_sqlite_schema() {
    let pts = vec![
        sample_point(100.0, 150.0, 2.5),
        sample_point(112.5, 168.75, 2.75),
        sample_point(125.0, 187.5, 3.0),
        sample_point(137.5, 195.25, 3.25),
        sample_point(150.0, 200.0, 3.5),
    ];
    let stroke = Stroke::with_points(777, StrokeStyle::default(), pts);

    assert!(stroke.cached_bbox.is_some());
    assert!(!stroke.cached_bezier.is_empty());

    let json_str = serde_json::to_string(&stroke).expect("Serialization must succeed");

    // Verify cache fields are completely absent from JSON
    assert!(
        !json_str.contains("cached_bbox"),
        "JSON must not contain 'cached_bbox'"
    );
    assert!(
        !json_str.contains("cached_bezier"),
        "JSON must not contain 'cached_bezier'"
    );

    // Verify parse to JSON Value
    let v: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    assert!(v.get("cached_bbox").is_none());
    assert!(v.get("cached_bezier").is_none());

    // Deserialize back
    let de_stroke: Stroke = serde_json::from_str(&json_str).expect("Deserialization must succeed");

    // Serde skip leaves cache fields unpopulated
    assert!(de_stroke.cached_bbox.is_none());
    assert!(de_stroke.cached_bezier.is_empty());

    // Semantic PartialEq must be true despite cache field divergence!
    assert_eq!(
        stroke, de_stroke,
        "Stroke and deserialized Stroke must be equal under semantic PartialEq"
    );

    // After ensure_cache, cached fields are restored identically
    let mut warmed = de_stroke;
    warmed.ensure_cache();
    assert_eq!(warmed.cached_bbox, stroke.cached_bbox);
    assert_eq!(warmed.cached_bezier, stroke.cached_bezier);
}
