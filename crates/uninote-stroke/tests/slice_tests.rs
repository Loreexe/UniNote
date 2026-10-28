//! Comprehensive Unit and Integration Test Suite for Circular Eraser Bézier Slicing in uninote-stroke.

use uninote_stroke::geometry::CubicBezierSegment;
use uninote_stroke::slice::{
    extract_bezier_subsegment, find_bezier_circle_intersections, reconstruct_sub_stroke,
    slice_stroke_with_circle, subdivide_cubic_bezier,
};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

fn create_sample_points(coords: &[(f64, f64)], base_w: f64) -> Vec<StrokePoint> {
    coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: base_w,
            timestamp_ns: 1_000_000 * (i as u64 + 1),
        })
        .collect()
}

fn create_test_stroke(id: u64, coords: &[(f64, f64)], base_w: f64) -> Stroke {
    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width: base_w,
        min_width_factor: 0.5,
        max_width_factor: 1.5,
    };
    let points = create_sample_points(coords, base_w);
    let mut raw_samples = Vec::new();
    for (i, p) in points.iter().enumerate() {
        let phase = if i == 0 {
            InputPhase::Down
        } else if i == points.len() - 1 {
            InputPhase::Up
        } else {
            InputPhase::Move
        };
        raw_samples.push(InputSample::new(
            (p.x, p.y),
            Some(p.pressure),
            p.timestamp_ns,
            DeviceKind::Stylus,
            phase,
        ));
    }
    let mut stroke = Stroke {
        id,
        style,
        points,
        raw_samples,
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// =========================================================================
// 1. Tests for de Casteljau Subdivision & Subsegment Extraction
// =========================================================================

#[test]
fn test_subdivide_cubic_bezier_midpoint_exactness() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (20.0, 60.0),
        cp2: (80.0, 60.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 4.0,
    };

    let (left, right) = subdivide_cubic_bezier(&seg, 0.5);

    // Endpoint of left must match startpoint of right and match seg.evaluate(0.5)
    let mid_pt = seg.evaluate(0.5);
    assert!((left.p1.0 - mid_pt.0).abs() < 1e-9);
    assert!((left.p1.1 - mid_pt.1).abs() < 1e-9);
    assert!((right.p0.0 - mid_pt.0).abs() < 1e-9);
    assert!((right.p0.1 - mid_pt.1).abs() < 1e-9);

    // Left start must be p0, right end must be p1
    assert!((left.p0.0 - seg.p0.0).abs() < 1e-9);
    assert!((left.p0.1 - seg.p0.1).abs() < 1e-9);
    assert!((right.p1.0 - seg.p1.0).abs() < 1e-9);
    assert!((right.p1.1 - seg.p1.1).abs() < 1e-9);

    // Width at junction must equal seg.evaluate_width(0.5)
    let mid_w = seg.evaluate_width(0.5);
    assert!((left.width1 - mid_w).abs() < 1e-9);
    assert!((right.width0 - mid_w).abs() < 1e-9);

    // Check intermediate evaluations: left(u) == seg(0.5 * u)
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let p_left = left.evaluate(u);
        let p_orig = seg.evaluate(u * 0.5);
        assert!((p_left.0 - p_orig.0).abs() < 1e-7);
        assert!((p_left.1 - p_orig.1).abs() < 1e-7);
    }
}

#[test]
fn test_subdivide_cubic_bezier_boundary_parameters() {
    let seg = CubicBezierSegment {
        p0: (10.0, 10.0),
        cp1: (20.0, 30.0),
        cp2: (40.0, 50.0),
        p1: (60.0, 70.0),
        width0: 1.0,
        width1: 3.0,
    };

    // t = 0.0: left is degenerate point, right is original curve
    let (left_0, right_0) = subdivide_cubic_bezier(&seg, 0.0);
    assert!((left_0.p0.0 - seg.p0.0).abs() < 1e-9);
    assert!((left_0.p1.0 - seg.p0.0).abs() < 1e-9);
    assert!((right_0.p0.0 - seg.p0.0).abs() < 1e-9);
    assert!((right_0.p1.0 - seg.p1.0).abs() < 1e-9);

    // t = 1.0: left is original curve, right is degenerate point
    let (left_1, right_1) = subdivide_cubic_bezier(&seg, 1.0);
    assert!((left_1.p0.0 - seg.p0.0).abs() < 1e-9);
    assert!((left_1.p1.0 - seg.p1.0).abs() < 1e-9);
    assert!((right_1.p0.0 - seg.p1.0).abs() < 1e-9);
    assert!((right_1.p1.0 - seg.p1.0).abs() < 1e-9);
}

#[test]
fn test_extract_bezier_subsegment_interior_interval() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (0.0, 100.0),
        cp2: (100.0, 100.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 4.0,
    };

    let sub = extract_bezier_subsegment(&seg, 0.25, 0.75);

    // Endpoints of sub must match seg.evaluate(0.25) and seg.evaluate(0.75)
    let p_start = seg.evaluate(0.25);
    let p_end = seg.evaluate(0.75);
    assert!((sub.p0.0 - p_start.0).abs() < 1e-7);
    assert!((sub.p0.1 - p_start.1).abs() < 1e-7);
    assert!((sub.p1.0 - p_end.0).abs() < 1e-7);
    assert!((sub.p1.1 - p_end.1).abs() < 1e-7);

    // Intermediate points on sub(u) must match seg(0.25 + 0.5 * u)
    for step in 0..=10 {
        let u = step as f64 / 10.0;
        let p_sub = sub.evaluate(u);
        let p_orig = seg.evaluate(0.25 + 0.5 * u);
        assert!((p_sub.0 - p_orig.0).abs() < 1e-7);
        assert!((p_sub.1 - p_orig.1).abs() < 1e-7);
    }
}

// =========================================================================
// 2. Tests for find_bezier_circle_intersections
// =========================================================================

#[test]
fn test_find_bezier_circle_intersections_straight_line_two_roots() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (33.333, 0.0),
        cp2: (66.666, 0.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle at (50, 0) with radius 10: crosses at x = 40 (t = 0.4) and x = 60 (t = 0.6)
    let roots = find_bezier_circle_intersections(&seg, (50.0, 0.0), 10.0);
    assert_eq!(roots.len(), 2, "Expected exactly 2 roots");
    assert!(
        (roots[0] - 0.4).abs() < 1e-4,
        "Root 0 should be ~0.4, got {}",
        roots[0]
    );
    assert!(
        (roots[1] - 0.6).abs() < 1e-4,
        "Root 1 should be ~0.6, got {}",
        roots[1]
    );
}

#[test]
fn test_find_bezier_circle_intersections_endpoint_grazing() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (100.0 / 3.0, 0.0),
        cp2: (200.0 / 3.0, 0.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle centered at (0, 0) with radius 20: root at t = 0.2
    let roots = find_bezier_circle_intersections(&seg, (0.0, 0.0), 20.0);
    assert_eq!(roots.len(), 1);
    assert!((roots[0] - 0.2).abs() < 1e-4);
    assert!((seg.evaluate(roots[0]).0 - 20.0).abs() < 1e-4);
}

#[test]
fn test_find_bezier_circle_intersections_no_intersection_outside() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (30.0, 0.0),
        cp2: (70.0, 0.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle far away
    let roots = find_bezier_circle_intersections(&seg, (50.0, 50.0), 10.0);
    assert!(roots.is_empty(), "Should have no roots when outside");
}

#[test]
fn test_find_bezier_circle_intersections_completely_enclosing() {
    let seg = CubicBezierSegment {
        p0: (10.0, 10.0),
        cp1: (20.0, 20.0),
        cp2: (30.0, 20.0),
        p1: (40.0, 10.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Huge circle covering the entire curve
    let roots = find_bezier_circle_intersections(&seg, (25.0, 15.0), 200.0);
    assert!(
        roots.is_empty(),
        "Should have no boundary roots when curve is wholly inside"
    );
}

#[test]
fn test_find_bezier_circle_intersections_curved_arc() {
    // Semi-circle arc from (-50, 0) to (50, 0) passing through (0, 50)
    let seg = CubicBezierSegment {
        p0: (-50.0, 0.0),
        cp1: (-50.0, 55.228),
        cp2: (50.0, 55.228),
        p1: (50.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle centered at (0, 50) with radius 20: cuts the arc at two symmetric points
    let roots = find_bezier_circle_intersections(&seg, (0.0, 50.0), 20.0);
    assert_eq!(roots.len(), 2, "Expected 2 roots on curved arc");
    assert!(roots[0] < 0.5);
    assert!(roots[1] > 0.5);
    assert!(((1.0 - roots[1]) - roots[0]).abs() < 1e-3); // Symmetry
}

#[test]
fn test_find_bezier_circle_intersections_degenerate_inputs() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (10.0, 10.0),
        cp2: (20.0, 20.0),
        p1: (30.0, 30.0),
        width0: 2.0,
        width1: 2.0,
    };

    assert!(find_bezier_circle_intersections(&seg, (0.0, 0.0), 0.0).is_empty());
    assert!(find_bezier_circle_intersections(&seg, (0.0, 0.0), -10.0).is_empty());
    assert!(find_bezier_circle_intersections(&seg, (f64::NAN, 0.0), 10.0).is_empty());
    assert!(find_bezier_circle_intersections(&seg, (0.0, 0.0), f64::INFINITY).is_empty());
}

// =========================================================================
// 3. Tests for slice_stroke_with_circle Edge Cases
// =========================================================================

#[test]
fn test_slice_middle_split_two_sub_strokes() {
    // Horizontal line from (0, 0) to (100, 0)
    let stroke = create_test_stroke(42, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser at center (50, 0) with radius 10 => cuts out [40, 60]
    let fragments = stroke.slice_with_circle((50.0, 0.0), 10.0);
    assert_eq!(fragments.len(), 2, "Middle split must produce 2 strokes");

    let f1 = &fragments[0];
    let f2 = &fragments[1];

    // Fragment 1: (0, 0) to (40, 0)
    assert_eq!(f1.points.len(), 2);
    assert!((f1.points[0].x - 0.0).abs() < 0.1);
    assert!((f1.points[1].x - 40.0).abs() < 0.1);
    assert!((f1.total_length() - 40.0).abs() < 0.5);

    // Fragment 2: (60, 0) to (100, 0)
    assert_eq!(f2.points.len(), 2);
    assert!((f2.points[0].x - 60.0).abs() < 0.1);
    assert!((f2.points[1].x - 100.0).abs() < 0.1);
    assert!((f2.total_length() - 40.0).abs() < 0.5);

    // Check IDs
    assert_eq!(f1.id, 42);
    assert_ne!(f2.id, 42);

    // Both must have valid warmed-up caches
    assert!(f1.cached_bbox.is_some());
    assert!(!f1.cached_bezier.is_empty());
    assert!(f2.cached_bbox.is_some());
    assert!(!f2.cached_bezier.is_empty());
}

#[test]
fn test_slice_trim_start_of_stroke() {
    let stroke = create_test_stroke(101, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser at (0, 0) with radius 20 => cuts out [0, 20]
    let fragments = stroke.slice_with_circle((0.0, 0.0), 20.0);
    assert_eq!(fragments.len(), 1, "Trimming start must produce 1 stroke");

    let f = &fragments[0];
    assert!((f.points[0].x - 20.0).abs() < 0.2);
    assert!((f.points[1].x - 100.0).abs() < 0.1);
    assert!((f.total_length() - 80.0).abs() < 0.5);
    assert!(f.cached_bbox.is_some());
    assert!(!f.cached_bezier.is_empty());
}

#[test]
fn test_slice_trim_end_of_stroke() {
    let stroke = create_test_stroke(102, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser at (100, 0) with radius 20 => cuts out [80, 100]
    let fragments = stroke.slice_with_circle((100.0, 0.0), 20.0);
    assert_eq!(fragments.len(), 1, "Trimming end must produce 1 stroke");

    let f = &fragments[0];
    assert!((f.points[0].x - 0.0).abs() < 0.1);
    assert!((f.points[1].x - 80.0).abs() < 0.2);
    assert!((f.total_length() - 80.0).abs() < 0.5);
    assert!(f.cached_bbox.is_some());
    assert!(!f.cached_bezier.is_empty());
}

#[test]
fn test_slice_complete_erasure() {
    let stroke = create_test_stroke(103, &[(10.0, 10.0), (30.0, 10.0)], 2.0);

    // Eraser covering the entire stroke
    let fragments = stroke.slice_with_circle((20.0, 10.0), 50.0);
    assert!(
        fragments.is_empty(),
        "Completely erased stroke must return empty Vec"
    );
}

#[test]
fn test_slice_out_of_range_no_intersection() {
    let stroke = create_test_stroke(104, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser far away
    let fragments = stroke.slice_with_circle((50.0, 80.0), 10.0);
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].id, stroke.id);
    assert_eq!(fragments[0].points.len(), stroke.points.len());
    assert_eq!(fragments[0], stroke);
}

#[test]
fn test_slice_tangent_circle_outside_untouched() {
    let stroke = create_test_stroke(105, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser touching line at (50, 0) tangentially from y = 10, radius 10
    let fragments = stroke.slice_with_circle((50.0, 10.0), 10.0);
    assert_eq!(fragments.len(), 1);
    assert_eq!(fragments[0].id, stroke.id);
    assert_eq!(fragments[0], stroke);
}

#[test]
fn test_slice_single_point_dot_stroke() {
    let mut dot = Stroke {
        id: 200,
        is_dot: true,
        points: vec![StrokePoint {
            x: 50.0,
            y: 50.0,
            pressure: 0.5,
            width: 4.0,
            timestamp_ns: 1_000_000,
        }],
        ..Default::default()
    };
    dot.recompute_cache();

    // 1. Eraser at (50, 50) with radius 10 => covers dot => erased
    let erased = dot.slice_with_circle((50.0, 50.0), 10.0);
    assert!(erased.is_empty(), "Dot covered by eraser must be erased");

    // 2. Eraser at (100, 100) with radius 10 => outside => untouched
    let untouched = dot.slice_with_circle((100.0, 100.0), 10.0);
    assert_eq!(untouched.len(), 1);
    assert_eq!(untouched[0].id, dot.id);
}

#[test]
fn test_slice_discard_sub_pixel_dust() {
    // Stroke from (0, 0) to (100, 0)
    let stroke = create_test_stroke(300, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    // Eraser cuts from 1.0 to 100 => remaining start piece is (0, 0) to (1.0, 0), length = 1.0 px (< 1.5 px)
    // Circle centered at (50.5, 0) with radius 49.5:
    // Left boundary: 50.5 - 49.5 = 1.0. Right boundary: 50.5 + 49.5 = 100.0.
    let fragments = stroke.slice_with_circle((50.5, 0.0), 49.5);

    // The 1.0 px fragment must be discarded as sub-pixel dust (< 1.5 px)!
    assert!(
        fragments.is_empty(),
        "Fragment shorter than 1.5 px must be discarded as dust"
    );
}

#[test]
fn test_slice_multi_segment_curved_stroke_and_tangents() {
    // A 4-point smooth S-curve
    let stroke = create_test_stroke(
        400,
        &[(0.0, 0.0), (30.0, 40.0), (70.0, -40.0), (100.0, 0.0)],
        3.0,
    );

    // Slicing through the middle around (50, 0) with radius 15
    let fragments = stroke.slice_with_circle((50.0, 0.0), 15.0);
    assert_eq!(
        fragments.len(),
        2,
        "Curved stroke sliced in middle must produce 2 sub-strokes"
    );

    for f in &fragments {
        assert!(f.points.len() >= 2);
        assert!(f.total_length() >= 1.5);
        assert!(f.cached_bbox.is_some());
        assert!(!f.cached_bezier.is_empty());
    }

    // Check that endpoints of surviving pieces lie near the circle perimeter
    let f1_end = f1_last_point(&fragments[0]);
    let dist1 = ((f1_end.0 - 50.0).powi(2) + f1_end.1.powi(2)).sqrt();
    assert!(
        (dist1 - 15.0).abs() < 1.0,
        "Fragment 1 end should be at circle perimeter (~15.0), got {}",
        dist1
    );

    let f2_start = (fragments[1].points[0].x, fragments[1].points[0].y);
    let dist2 = ((f2_start.0 - 50.0).powi(2) + f2_start.1.powi(2)).sqrt();
    assert!(
        (dist2 - 15.0).abs() < 1.0,
        "Fragment 2 start should be at circle perimeter (~15.0), got {}",
        dist2
    );
}

fn f1_last_point(s: &Stroke) -> (f64, f64) {
    let p = s.points.last().unwrap();
    (p.x, p.y)
}

#[test]
fn test_slice_zigzag_multi_intersection() {
    // A stroke that loops in and out of a circle region:
    // (0, 50) -> (50, 50) -> (50, 0) -> (0, 0) -> (0, 50)
    let stroke = create_test_stroke(
        500,
        &[
            (0.0, 50.0),
            (50.0, 50.0),
            (50.0, 0.0),
            (0.0, 0.0),
            (0.0, 50.0),
        ],
        2.0,
    );

    // Slice with circle centered at (50, 25) with radius 15:
    // Cuts the vertical segment (50, 50) -> (50, 0) in the middle
    let fragments = stroke.slice_with_circle((50.0, 25.0), 15.0);
    assert_eq!(
        fragments.len(),
        2,
        "Cutting one leg of loop must split into 2 pieces"
    );
}

#[test]
fn test_slice_idempotent_and_repeated_slicing() {
    let stroke = create_test_stroke(600, &[(0.0, 0.0), (200.0, 0.0)], 2.0);

    // First cut at (50, 0), R = 10 => produces [0, 40] and [60, 200]
    let pieces1 = stroke.slice_with_circle((50.0, 0.0), 10.0);
    assert_eq!(pieces1.len(), 2);

    // Cut the second piece at (150, 0), R = 10 => produces [60, 140] and [160, 200]
    let pieces2 = pieces1[1].slice_with_circle((150.0, 0.0), 10.0);
    assert_eq!(pieces2.len(), 2);

    assert!((pieces2[0].points[0].x - 60.0).abs() < 0.2);
    assert!((pieces2[0].points.last().unwrap().x - 140.0).abs() < 0.2);
    assert!((pieces2[1].points[0].x - 160.0).abs() < 0.2);
    assert!((pieces2[1].points.last().unwrap().x - 200.0).abs() < 0.2);
}

#[test]
fn test_slice_preserves_interpolated_metadata() {
    let stroke = create_test_stroke(700, &[(0.0, 0.0), (100.0, 0.0)], 2.0);

    let fragments = stroke.slice_with_circle((50.0, 0.0), 10.0);
    assert_eq!(fragments.len(), 2);

    for f in &fragments {
        assert_eq!(f.style.color, stroke.style.color);
        assert_eq!(f.style.base_width, stroke.style.base_width);
        assert_eq!(f.mode, stroke.mode);

        // Check timestamp monotonicity
        let mut prev_ts = 0;
        for p in &f.points {
            assert!(p.timestamp_ns >= prev_ts);
            prev_ts = p.timestamp_ns;
            assert!(p.width > 0.0);
            assert!(p.pressure >= 0.0 && p.pressure <= 1.0);
        }

        // Check raw samples validity
        assert!(f.raw_samples.len() >= 2);
        assert_eq!(f.raw_samples.first().unwrap().phase, InputPhase::Down);
        assert_eq!(f.raw_samples.last().unwrap().phase, InputPhase::Up);
    }
}

#[test]
fn test_standalone_slice_stroke_with_circle_function() {
    let stroke = create_test_stroke(800, &[(0.0, 0.0), (100.0, 0.0)], 2.0);
    let frags = slice_stroke_with_circle(&stroke, (50.0, 0.0), 10.0);
    assert_eq!(frags.len(), 2);
    assert!((frags[0].points.last().unwrap().x - 40.0).abs() < 0.2);
    assert!((frags[1].points.first().unwrap().x - 60.0).abs() < 0.2);
}

#[test]
fn test_standalone_reconstruct_sub_stroke() {
    let stroke = create_test_stroke(900, &[(0.0, 0.0), (100.0, 0.0)], 2.0);
    let segs = stroke.bezier_segments();
    let sub = reconstruct_sub_stroke(&stroke, segs, 0.1, 0.9, 901);
    assert!(sub.is_some());
    let s = sub.unwrap();
    assert_eq!(s.id, 901);
    assert!((s.points.first().unwrap().x - 10.0).abs() < 0.2);
    assert!((s.points.last().unwrap().x - 90.0).abs() < 0.2);
    assert!((s.total_length() - 80.0).abs() < 0.5);
    assert!(s.cached_bbox.is_some());
    assert!(!s.cached_bezier.is_empty());
}

#[test]
fn test_reconstruct_sub_stroke_boundary_normalization_panic_guard() {
    let stroke = create_test_stroke(
        999,
        &[(0.0, 0.0), (50.0, 50.0), (100.0, 0.0), (150.0, 50.0)],
        2.0,
    );
    let segs = stroke.bezier_segments();
    // Interval at the very tail of segment 1 (theta in [1.999995, 1.999998])
    // Previously panicked with `slice index starts at 3 but ends at 1`.
    let res = reconstruct_sub_stroke(&stroke, segs, 1.999995, 1.999998, 1000);
    // Must safely return None without panicking on slice index
    assert!(res.is_none() || res.is_some());
}

#[test]
fn test_slice_closed_loop_teardrop_stroke() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (10.0, 100.0),
        cp2: (-10.0, 100.0),
        p1: (0.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };
    // Circle centered at (0, 50) with radius 10 intersects the teardrop loop
    let roots = find_bezier_circle_intersections(&seg, (0.0, 50.0), 10.0);
    assert!(
        roots.len() >= 2,
        "Closed loop where p0 == p1 must find circle intersections: {:?}",
        roots
    );
    for r in &roots {
        let pt = seg.evaluate(*r);
        let dist = pt.0.hypot(pt.1 - 50.0);
        assert!((dist - 10.0).abs() < 0.5);
    }
}

#[test]
fn test_slice_recursive_slicing_unique_ids() {
    let stroke = create_test_stroke(
        1000,
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (200.0, 0.0),
            (300.0, 0.0),
            (400.0, 0.0),
            (500.0, 0.0),
            (600.0, 0.0),
        ],
        2.0,
    );

    let mut current_strokes = vec![stroke];
    let cuts = [(300.0, 0.0, 20.0), (150.0, 0.0, 20.0), (450.0, 0.0, 20.0)];

    for (cx, cy, r) in cuts {
        let mut next_gen = Vec::new();
        for s in current_strokes {
            next_gen.extend(s.slice_with_circle((cx, cy), r));
        }
        current_strokes = next_gen;
    }

    assert_eq!(current_strokes.len(), 4);
    let mut ids = std::collections::HashSet::new();
    for s in &current_strokes {
        assert!(ids.insert(s.id), "Duplicate sub-stroke ID found: {}", s.id);
        assert!(s.points.len() >= 2);
        assert!(s.cached_bbox.is_some());
        assert!(!s.cached_bezier.is_empty());
    }
}

#[test]
fn test_slice_serde_json_roundtrip_and_cache_rehydration() {
    let stroke = create_test_stroke(2000, &[(0.0, 0.0), (100.0, 0.0)], 2.0);
    let fragments = stroke.slice_with_circle((50.0, 0.0), 10.0);
    assert_eq!(fragments.len(), 2);

    for frag in &fragments {
        let json = serde_json::to_string(frag).expect("Serialization to JSON must succeed");
        let mut deserialized: Stroke =
            serde_json::from_str(&json).expect("Deserialization from JSON must succeed");

        // Verify skipped cache fields are initially empty
        assert!(deserialized.cached_bbox.is_none());
        assert!(deserialized.cached_bezier.is_empty());

        // Rehydrate cache
        deserialized.recompute_cache();

        assert_eq!(deserialized.id, frag.id);
        assert_eq!(deserialized.points.len(), frag.points.len());
        assert_eq!(deserialized.bounding_box(), frag.bounding_box());
        assert_eq!(deserialized.cached_bezier.len(), frag.cached_bezier.len());
    }
}

#[test]
fn test_regression_slice_deduped_points_and_disaligned_bezier_no_panic() {
    // 1. Direct simulation of disalignment (len is 39 but cached_bezier index/len is 42)
    let mut coords = Vec::new();
    for i in 0..39 {
        coords.push((i as f64 * 10.0, 50.0));
    }
    let mut stroke = create_test_stroke(9901, &coords, 2.0);
    assert_eq!(stroke.points.len(), 39);
    assert_eq!(stroke.cached_bezier.len(), 38);

    // Artificially inject 42 Bézier segments (disaligned with 39 points)
    let mut disaligned_bezier = Vec::new();
    for i in 0..42 {
        disaligned_bezier.push(CubicBezierSegment {
            p0: (i as f64 * 10.0, 50.0),
            cp1: (i as f64 * 10.0 + 3.0, 50.0),
            cp2: (i as f64 * 10.0 + 7.0, 50.0),
            p1: ((i + 1) as f64 * 10.0, 50.0),
            width0: 2.0,
            width1: 2.0,
        });
    }
    stroke.cached_bezier = disaligned_bezier.clone();
    assert_eq!(stroke.cached_bezier.len(), 42);

    // slice_stroke_with_circle must detect disalignment, recalculate Bézier segments, and not panic
    let frags = stroke.slice_with_circle((200.0, 50.0), 15.0);
    assert_eq!(frags.len(), 2);
    for f in &frags {
        assert_eq!(f.cached_bezier.len(), f.points.len().saturating_sub(1));
    }

    // Direct call to reconstruct_sub_stroke with mismatched segments (m = 42) and stroke (len = 39)
    // Must not panic even when theta accesses seg_idx >= 39
    let sub = reconstruct_sub_stroke(&stroke, &disaligned_bezier, 38.5, 41.5, 9902);
    assert!(sub.is_some());
    let s = sub.unwrap();
    assert_eq!(s.cached_bezier.len(), s.points.len().saturating_sub(1));

    // 2. Scenario with consecutive points spaced < 1e-3 px that trigger deduplication during slicing
    let mut dedupe_coords = Vec::new();
    for i in 0..45 {
        dedupe_coords.push((i as f64 * 5.0, 100.0));
        // Add duplicate / nearly identical point (< 1e-3 px apart)
        if i == 10 || i == 20 || i == 30 {
            dedupe_coords.push((i as f64 * 5.0 + 1e-4, 100.0));
        }
    }
    let stroke2 = create_test_stroke(9903, &dedupe_coords, 2.0);
    // Repeated cutting across the deduplication regions
    let frags2 = stroke2.slice_with_circle((50.0, 100.0), 12.0);
    assert!(!frags2.is_empty());
    for f in &frags2 {
        assert_eq!(f.cached_bezier.len(), f.points.len().saturating_sub(1));
        // Re-slice the fragment repeatedly to simulate interactive eraser stroke
        let sub_frags = f.slice_with_circle((100.0, 100.0), 10.0);
        for sf in &sub_frags {
            assert_eq!(sf.cached_bezier.len(), sf.points.len().saturating_sub(1));
            let sub_frags2 = sf.slice_with_circle((150.0, 100.0), 8.0);
            for sf2 in &sub_frags2 {
                assert_eq!(sf2.cached_bezier.len(), sf2.points.len().saturating_sub(1));
            }
        }
    }
}

#[test]
fn test_slice_baseline_mode_preserves_empty_cached_bezier() {
    let coords: Vec<(f64, f64)> = (0..20).map(|i| (i as f64 * 10.0, 50.0)).collect();
    let pts: Vec<StrokePoint> = coords
        .into_iter()
        .map(|(x, y)| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 0,
        })
        .collect();
    let stroke =
        Stroke::with_points_and_mode(9950, StrokeStyle::default(), PipelineMode::Baseline, pts);
    assert!(stroke.cached_bezier.is_empty());

    let frags = stroke.slice_with_circle((100.0, 50.0), 15.0);
    assert!(!frags.is_empty());
    for f in &frags {
        assert_eq!(f.mode, PipelineMode::Baseline);
        assert!(
            f.cached_bezier.is_empty(),
            "Baseline sub-stroke must have empty cached_bezier"
        );
    }
}

#[test]
fn test_slice_recursive_scrubbing_self_intersecting_knot_no_panic() {
    // 1. Trefoil knot self-intersecting figure
    let mut coords = Vec::new();
    for i in 0..120 {
        let t = (i as f64) * 2.0 * std::f64::consts::PI / 120.0;
        let x = 300.0 + 90.0 * t.sin() + 45.0 * (2.0 * t).sin();
        let y = 300.0 + 90.0 * t.cos() - 45.0 * (2.0 * t).cos();
        coords.push((x, y));
    }
    let stroke = create_test_stroke(9960, &coords, 2.5);

    // Rapid back-and-forth scrubbing across the intersection core
    let mut current_strokes = vec![stroke];
    for step in 0..35 {
        let offset_x = ((step as f64) * 0.4).sin() * 25.0;
        let offset_y = ((step as f64) * 0.4).cos() * 20.0;
        let center = (300.0 + offset_x, 300.0 + offset_y);
        let radius = 14.0 + ((step % 5) as f64) * 2.0;

        let mut next_generation = Vec::new();
        for s in &current_strokes {
            let frags = s.slice_with_circle(center, radius);
            for f in frags {
                assert_eq!(f.cached_bezier.len(), f.points.len().saturating_sub(1));
                assert!(f.points.len() >= 2);
                assert!(f.cached_bbox.is_some());
                for p in &f.points {
                    assert!(p.x.is_finite() && p.y.is_finite());
                }
                next_generation.push(f);
            }
        }
        current_strokes = next_generation;
    }

    // 2. Figure-eight curve with deep scrub through the cross-point
    let mut fig8_coords = Vec::new();
    for i in 0..80 {
        let t = (i as f64) * 2.0 * std::f64::consts::PI / 80.0;
        let x = 200.0 + 70.0 * t.sin();
        let y = 200.0 + 70.0 * t.sin() * t.cos();
        fig8_coords.push((x, y));
    }
    let fig8 = create_test_stroke(9961, &fig8_coords, 3.0);
    let mut fig8_strokes = vec![fig8];
    for step in 0..25 {
        let center = (200.0 + ((step as f64) - 12.0) * 2.0, 200.0);
        let mut next_gen = Vec::new();
        for s in &fig8_strokes {
            let frags = s.slice_with_circle(center, 12.0);
            for f in frags {
                assert_eq!(f.cached_bezier.len(), f.points.len().saturating_sub(1));
                next_gen.push(f);
            }
        }
        fig8_strokes = next_gen;
    }
}

#[test]
fn test_reconstruct_sub_stroke_nan_inf_guards() {
    let stroke = create_test_stroke(9970, &[(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)], 2.0);
    let segs = stroke.bezier_segments();

    // NaN parameters must return None rather than generating NaN coordinates
    assert!(reconstruct_sub_stroke(&stroke, segs, f64::NAN, 1.0, 9971).is_none());
    assert!(reconstruct_sub_stroke(&stroke, segs, 0.0, f64::NAN, 9972).is_none());
    assert!(reconstruct_sub_stroke(&stroke, segs, f64::NAN, f64::NAN, 9973).is_none());

    // Infinite parameters must return None
    assert!(reconstruct_sub_stroke(&stroke, segs, f64::NEG_INFINITY, 1.0, 9974).is_none());
    assert!(reconstruct_sub_stroke(&stroke, segs, 0.0, f64::INFINITY, 9975).is_none());

    // Non-finite points inside original_stroke must return None
    let mut nan_stroke = stroke.clone();
    nan_stroke.points[1].x = f64::NAN;
    assert!(reconstruct_sub_stroke(&nan_stroke, segs, 0.0, 1.0, 9976).is_none());

    // slice_stroke_with_circle on non-finite points returns stroke untouched without panic
    let frags = nan_stroke.slice_with_circle((50.0, 50.0), 10.0);
    assert_eq!(frags.len(), 1);
}

#[test]
fn test_reconstruct_sub_stroke_disaligned_intermediate_continuity() {
    // Original stroke with 39 points
    let coords: Vec<(f64, f64)> = (0..39).map(|i| (i as f64 * 10.0, 50.0)).collect();
    let stroke = create_test_stroke(9980, &coords, 2.0);
    assert_eq!(stroke.points.len(), 39);

    // Mismatched segments: 45 segments curved along y = sin(x)
    let mut disaligned_segs = Vec::new();
    for i in 0..45 {
        let x0 = i as f64 * 10.0;
        let x1 = (i + 1) as f64 * 10.0;
        disaligned_segs.push(CubicBezierSegment {
            p0: (x0, 50.0 + (x0 * 0.1).sin() * 20.0),
            cp1: (x0 + 3.0, 50.0 + (x0 * 0.1).sin() * 20.0),
            cp2: (x1 - 3.0, 50.0 + (x1 * 0.1).sin() * 20.0),
            p1: (x1, 50.0 + (x1 * 0.1).sin() * 20.0),
            width0: 2.0,
            width1: 2.0,
        });
    }

    // Interval spanning intermediate points where seg_idx crosses 39 (e.g. 5.5 to 42.5)
    let sub = reconstruct_sub_stroke(&stroke, &disaligned_segs, 5.5, 42.5, 9981);
    assert!(sub.is_some());
    let s = sub.unwrap();

    // Verify cache alignment and strict coordinate continuity with disaligned_segs
    assert_eq!(s.cached_bezier.len(), s.points.len().saturating_sub(1));
    for p in &s.points {
        assert!(p.x.is_finite());
        assert!(p.y.is_finite());
    }

    // Verify that every point sits close to the curve (y is within [30.0, 70.0], not jumping)
    for p in &s.points {
        assert!(
            p.y >= 29.0 && p.y <= 71.0,
            "Point y={} must follow curve, no jump",
            p.y
        );
    }
}

#[test]
fn test_reconstruct_sub_stroke_closed_loop_single_segment_preserved() {
    // A single cubic Bezier segment forming a closed loop (p0 == p1)
    let loop_seg = CubicBezierSegment {
        p0: (100.0, 100.0),
        cp1: (100.0, 200.0),
        cp2: (200.0, 100.0),
        p1: (100.0, 100.0),
        width0: 2.0,
        width1: 2.0,
    };
    let stroke = create_test_stroke(9990, &[(100.0, 100.0), (100.0, 100.0)], 2.0);
    let segs = vec![loop_seg];

    // Reconstructing the loop within the single segment (0.0 to 1.0)
    let sub = reconstruct_sub_stroke(&stroke, &segs, 0.0, 1.0, 9991);
    assert!(
        sub.is_some(),
        "Large closed loop in single segment must not be discarded"
    );
    let s = sub.unwrap();
    assert!(
        s.points.len() >= 3,
        "Closed loop must include midpoint to preserve curvature"
    );
    assert_eq!(s.cached_bezier.len(), s.points.len().saturating_sub(1));
}

#[test]
fn test_slice_extreme_radius_boundaries() {
    let stroke = create_test_stroke(9995, &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)], 2.0);

    // 1. Tiny radius < 1e-6 (e.g. 1e-8)
    let tiny_frags = stroke.slice_with_circle((50.0, 0.0), 1e-8);
    // Sub-pixel dust should be discarded and original stroke either slightly trimmed or untouched
    assert!(!tiny_frags.is_empty());

    // 2. Huge radius > 1e6 (completely engulfs stroke)
    let huge_frags = stroke.slice_with_circle((50.0, 0.0), 1e7);
    assert!(
        huge_frags.is_empty(),
        "Huge eraser must erase entire stroke"
    );
}

#[test]
fn test_reconstruct_sub_stroke_with_nan_segments_returns_none() {
    let stroke = create_test_stroke(9996, &[(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)], 2.0);
    let mut nan_segs = stroke.bezier_segments().to_vec();
    nan_segs[0].cp1.0 = f64::NAN;

    assert!(reconstruct_sub_stroke(&stroke, &nan_segs, 0.0, 1.0, 9997).is_none());

    nan_segs[0].cp1.0 = 25.0;
    nan_segs[0].width0 = f64::INFINITY;
    assert!(reconstruct_sub_stroke(&stroke, &nan_segs, 0.0, 1.0, 9998).is_none());
}

#[test]
fn test_slice_stroke_with_corrupt_nan_cached_bezier_recovers() {
    let coords: Vec<(f64, f64)> = (0..10).map(|i| (i as f64 * 10.0, 50.0)).collect();
    let mut stroke = create_test_stroke(9999, &coords, 2.0);
    assert_eq!(stroke.cached_bezier.len(), 9);

    // Corrupt one control point with NaN
    stroke.cached_bezier[3].cp2.1 = f64::NAN;

    // slice_with_circle must reject the NaN cache, recompute via Catmull-Rom, and slice successfully
    let frags = stroke.slice_with_circle((50.0, 50.0), 10.0);
    assert_eq!(frags.len(), 2);
    for f in &frags {
        assert_eq!(f.cached_bezier.len(), f.points.len().saturating_sub(1));
        for seg in &f.cached_bezier {
            assert!(seg.is_finite());
        }
    }
}

#[test]
fn test_ensure_cache_repairs_disaligned_or_nan_cached_bezier() {
    let coords: Vec<(f64, f64)> = (0..20).map(|i| (i as f64 * 10.0, 50.0)).collect();
    let mut stroke = create_test_stroke(10000, &coords, 2.0);
    assert_eq!(stroke.cached_bezier.len(), 19);

    // 1. Truncate cached_bezier to simulate disalignment (e.g. 10 segments vs 20 points)
    stroke.cached_bezier.truncate(10);
    assert_eq!(stroke.cached_bezier.len(), 10);

    // ensure_cache must detect length disalignment and repair
    stroke.ensure_cache();
    assert_eq!(stroke.cached_bezier.len(), 19);

    // 2. Inject NaN into a segment
    stroke.cached_bezier[5].p1.0 = f64::NAN;
    stroke.ensure_cache();
    assert_eq!(stroke.cached_bezier.len(), 19);
    for seg in &stroke.cached_bezier {
        assert!(seg.is_finite());
    }
}
