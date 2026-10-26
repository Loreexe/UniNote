//! Empirical Adversarial Test Suite for Milestone M1 — Stroke Slicing
//!
//! Stress tests extreme corner cases, mathematical pathologies, and geometric invariants:
//! 1. Zero, negative, NaN/Inf circle coordinates and radii.
//! 2. Tangent grazing where distance == radius exactly (boundary contacts, endpoint contacts, vertex grazing).
//! 3. Collinear 4-point cubic Béziers (uniform, non-uniform clustered, and backtracking).
//! 4. High-curvature loops, cusps (vanishing derivative), and self-intersections.
//! 5. Repeated recursive slicing (deep subdivision trees, sequential eraser drags, serialization stability).
//! 6. Numerical precision and scale extremes (sub-pixel, large coordinates, micro-radii).

use uninote_stroke::geometry::CubicBezierSegment;
use uninote_stroke::slice::{bezier_derivative, find_bezier_circle_intersections};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

fn make_stroke(id: u64, coords: &[(f64, f64)], base_w: f64) -> Stroke {
    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width: base_w,
        min_width_factor: 0.5,
        max_width_factor: 1.5,
    };
    let points: Vec<StrokePoint> = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: base_w,
            timestamp_ns: 1_000_000 * (i as u64 + 1),
        })
        .collect();

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
// Category 1: Extreme Input Validation & Pathological Values (NaN, Inf, Zero, Neg)
// =========================================================================

#[test]
fn test_adversarial_nan_and_inf_handling() {
    let stroke = make_stroke(100, &[(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)], 2.0);

    let pathological_queries = vec![
        // (center_x, center_y, radius)
        (f64::NAN, 50.0, 10.0),
        (50.0, f64::NAN, 10.0),
        (50.0, 50.0, f64::NAN),
        (f64::INFINITY, 50.0, 10.0),
        (50.0, f64::INFINITY, 10.0),
        (50.0, 50.0, f64::INFINITY),
        (f64::NEG_INFINITY, 50.0, 10.0),
        (50.0, f64::NEG_INFINITY, 10.0),
        (50.0, 50.0, f64::NEG_INFINITY),
        (f64::NAN, f64::NAN, f64::NAN),
        // Zero and negative radii
        (50.0, 50.0, 0.0),
        (50.0, 50.0, -0.0),
        (50.0, 50.0, -10.0),
        (50.0, 50.0, -f64::MIN_POSITIVE),
    ];

    for (cx, cy, r) in pathological_queries {
        // Direct root finding must return empty without panic
        if let Some(first_seg) = stroke.cached_bezier.first() {
            let roots = find_bezier_circle_intersections(first_seg, (cx, cy), r);
            assert!(
                roots.is_empty(),
                "find_bezier_circle_intersections returned roots for pathological input: ({}, {}, {})",
                cx, cy, r
            );
        }

        // Full stroke slice must return original intact without modification or panic
        let result = stroke.slice_with_circle((cx, cy), r);
        assert_eq!(
            result.len(),
            1,
            "slice_with_circle failed to preserve original stroke on input: ({}, {}, {})",
            cx,
            cy,
            r
        );
        assert_eq!(result[0].id, stroke.id);
        assert_eq!(result[0].points.len(), stroke.points.len());
    }
}

#[test]
fn test_adversarial_degenerate_zero_length_stroke() {
    let empty_stroke = Stroke {
        id: 999,
        style: StrokeStyle {
            color: (0.0, 0.0, 0.0, 1.0),
            base_width: 2.0,
            min_width_factor: 1.0,
            max_width_factor: 1.0,
        },
        points: Vec::new(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    let res = empty_stroke.slice_with_circle((10.0, 10.0), 5.0);
    assert!(res.is_empty());
}

// =========================================================================
// Category 2: Tangent Grazing (Distance == Radius Exactly)
// =========================================================================

#[test]
fn test_adversarial_tangent_grazing_straight_line() {
    // Horizontal line at y = 50 from x = 0 to 100
    let stroke = make_stroke(101, &[(0.0, 50.0), (50.0, 50.0), (100.0, 50.0)], 2.0);

    // Circle centered at (50.0, 0.0) with radius 50.0
    // Tangent point is exactly at (50.0, 50.0). Distance = 50.0 == radius.
    // The curve does not penetrate into the circle interior (y < 50).
    let result = stroke.slice_with_circle((50.0, 0.0), 50.0);

    // Tangent contact must NOT shatter the stroke into pieces.
    // It should either return the single intact stroke or one continuous surviving stroke.
    assert_eq!(
        result.len(),
        1,
        "Grazing tangent at single point split the stroke into {} pieces!",
        result.len()
    );

    let total_len: f64 = result[0]
        .cached_bezier
        .iter()
        .map(|s| s.approximate_length())
        .sum();
    assert!(
        (total_len - 100.0).abs() < 2.0,
        "Grazing tangent altered total length significantly: {}",
        total_len
    );
}

#[test]
fn test_adversarial_tangent_grazing_endpoint() {
    // Line starting at (50.0, 50.0) extending to (150.0, 50.0)
    let stroke = make_stroke(102, &[(50.0, 50.0), (100.0, 50.0), (150.0, 50.0)], 2.0);

    // Circle centered at (50.0, 0.0) with radius 50.0.
    // Start point (50, 50) is exactly on circle boundary, rest extends away.
    let result = stroke.slice_with_circle((50.0, 0.0), 50.0);
    assert_eq!(result.len(), 1);
    let len: f64 = result[0]
        .cached_bezier
        .iter()
        .map(|s| s.approximate_length())
        .sum();
    assert!((len - 100.0).abs() < 2.0);
}

#[test]
fn test_adversarial_tangent_grazing_parabolic_curve_vertex() {
    // Parabolic curve with vertex at (50, 30)
    let coords = vec![
        (0.0, 80.0),
        (25.0, 45.0),
        (50.0, 30.0),
        (75.0, 45.0),
        (100.0, 80.0),
    ];
    let stroke = make_stroke(103, &coords, 2.0);

    // Circle below vertex: center (50, 10), radius 20.
    // Boundary touches (50, 30) exactly.
    let result = stroke.slice_with_circle((50.0, 10.0), 20.0);

    assert_eq!(
        result.len(),
        1,
        "Grazing curved vertex split stroke into {} pieces",
        result.len()
    );
}

#[test]
fn test_adversarial_curve_entirely_inside_tangent_to_boundary() {
    // Small segment entirely inside circle (50, 50) R = 50
    // spanning from (30, 80) to (50, 100) where (50, 100) is on boundary
    let stroke = make_stroke(104, &[(30.0, 80.0), (40.0, 90.0), (50.0, 100.0)], 2.0);

    // Slicing with circle (50, 50) R = 50: since it's entirely inside or on the boundary,
    // it must be completely erased!
    let result = stroke.slice_with_circle((50.0, 50.0), 50.0);
    assert!(
        result.is_empty(),
        "Curve entirely inside circle touching boundary was not erased, got {} strokes",
        result.len()
    );
}

// =========================================================================
// Category 3: Collinear 4-Point Cubic Béziers (Degenerate & Non-Uniform)
// =========================================================================

#[test]
fn test_adversarial_collinear_uniform_cubic() {
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (33.3333333333, 0.0),
        cp2: (66.6666666667, 0.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    let roots = find_bezier_circle_intersections(&seg, (50.0, 0.0), 20.0);
    assert_eq!(
        roots.len(),
        2,
        "Expected 2 roots for uniform collinear segment, got {:?}",
        roots
    );
    assert!((roots[0] - 0.3).abs() < 0.01);
    assert!((roots[1] - 0.7).abs() < 0.01);
}

#[test]
fn test_adversarial_collinear_clustered_control_points() {
    // Highly non-linear parameterization along a straight line:
    // cp1 and cp2 are packed tightly near p0
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (0.001, 0.0),
        cp2: (0.002, 0.0),
        p1: (100.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle at (50, 0) with radius 20 -> intersects line at X = 30 and X = 70
    let roots = find_bezier_circle_intersections(&seg, (50.0, 0.0), 20.0);
    assert_eq!(
        roots.len(),
        2,
        "Failed to find roots for clustered collinear Bézier: {:?}",
        roots
    );

    for r in &roots {
        let pt = seg.evaluate(*r);
        let dist = (pt.0 - 50.0).abs();
        assert!(
            (dist - 20.0).abs() < 0.5,
            "Root evaluation at t={} gave X={}, expected dist to center 20.0",
            r,
            pt.0
        );
    }
}

#[test]
fn test_adversarial_closed_loop_teardrop_bezier() {
    // Teardrop Bézier where p0 == p1 == (0, 0), looping out to (0, 75)
    let seg = CubicBezierSegment {
        p0: (0.0, 0.0),
        cp1: (10.0, 100.0),
        cp2: (-10.0, 100.0),
        p1: (0.0, 0.0),
        width0: 2.0,
        width1: 2.0,
    };

    // Circle at (0.0, 50.0) with radius 10.0 -> circle is around Y in [40, 60]
    // The curve reaches Y = 75 at t = 0.5. It crosses Y = 40 and Y = 60 on the way up,
    // and crosses them again on the way down.
    let roots = find_bezier_circle_intersections(&seg, (0.0, 50.0), 10.0);
    assert!(
        roots.len() >= 2,
        "Closed loop where p0 == p1 failed to find circle intersections (found {:?})",
        roots
    );

    for r in &roots {
        let pt = seg.evaluate(*r);
        let d = pt.0.hypot(pt.1 - 50.0);
        assert!(
            (d - 10.0).abs() < 0.5,
            "Root t={} pt={:?} not on circle (dist={})",
            r,
            pt,
            d
        );
    }
}

// =========================================================================
// Category 4: High-Curvature Loops, Cusps & Self-Intersections
// =========================================================================

#[test]
fn test_adversarial_cusp_vanishing_derivative() {
    // Exact mathematical cubic cusp:
    // X(t) = 300 * (t - 0.5)^2
    // Y(t) = 300 * (t - 0.5)^3
    // At t = 0.5: X'(0.5) = 0, Y'(0.5) = 0 => B'(0.5) = (0.0, 0.0)
    let seg = CubicBezierSegment {
        p0: (75.0, -37.5),
        cp1: (-25.0, 37.5),
        cp2: (-25.0, -37.5),
        p1: (75.0, 37.5),
        width0: 2.0,
        width1: 2.0,
    };

    // Verify derivative at t = 0.5 is exactly zero
    let deriv = bezier_derivative(&seg, 0.5);
    assert!(
        deriv.0.hypot(deriv.1) < 1e-9,
        "Test precondition failed: derivative at cusp is not zero: {:?}",
        deriv
    );

    // Cusp vertex is at B(0.5) = (0.0, 0.0)
    let cusp_pt = seg.evaluate(0.5);
    assert!(cusp_pt.0.hypot(cusp_pt.1) < 1e-9);

    // Eraser circle centered at cusp (0.0, 0.0) with radius 15.0
    let roots = find_bezier_circle_intersections(&seg, (0.0, 0.0), 15.0);
    assert_eq!(
        roots.len(),
        2,
        "Expected 2 roots entering and exiting cusp circle, got {:?}",
        roots
    );

    // Verify all found roots lie on the circle
    for r in &roots {
        let pt = seg.evaluate(*r);
        let d = pt.0.hypot(pt.1);
        assert!(
            (d - 15.0).abs() < 0.5,
            "Root {:?} not on circle (dist={})",
            pt,
            d
        );
    }
}

#[test]
fn test_adversarial_self_intersecting_figure_eight_stroke() {
    // Multi-point stroke that loops back and crosses itself in a figure-8
    let coords = vec![
        (50.0, 50.0),
        (80.0, 80.0),
        (50.0, 100.0),
        (20.0, 80.0),
        (50.0, 50.0), // crossing point
        (80.0, 20.0),
        (50.0, 0.0),
        (20.0, 20.0),
        (50.0, 50.0), // end at crossing point
    ];
    let stroke = make_stroke(105, &coords, 2.0);

    // Eraser placed right at the self-intersection crossing point (50, 50)
    let result = stroke.slice_with_circle((50.0, 50.0), 10.0);

    // The eraser at the crossing point cuts both loops, leaving isolated surviving lobes
    assert!(
        !result.is_empty(),
        "Cutting self-intersection should leave outer loop lobes, got empty"
    );

    for s in &result {
        assert!(s.points.len() >= 2);
        assert!(!s.cached_bezier.is_empty());
        assert!(s.cached_bbox.is_some());
    }
}

// =========================================================================
// Category 5: Repeated Recursive Slicing & Serialization Invariance
// =========================================================================

#[test]
fn test_adversarial_recursive_binary_tree_slicing() {
    // Long 600px line from (0, 0) to (600, 0)
    let initial_stroke = make_stroke(
        200,
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

    let mut current_strokes = vec![initial_stroke];

    // Perform 3 successive slicing cuts at 300, 150, 450
    let cuts = vec![
        (300.0, 0.0, 20.0), // Cuts middle: leaves [0, 280] and [320, 600]
        (150.0, 0.0, 20.0), // Cuts first half: leaves [0, 130], [170, 280], [320, 600]
        (450.0, 0.0, 20.0), // Cuts second half
    ];

    for (cx, cy, r) in cuts {
        let mut next_gen = Vec::new();
        for s in current_strokes {
            let res = s.slice_with_circle((cx, cy), r);
            next_gen.extend(res);
        }
        current_strokes = next_gen;
    }

    assert_eq!(
        current_strokes.len(),
        4,
        "Expected 4 sub-strokes after 3 binary cuts, got {}",
        current_strokes.len()
    );

    // Verify all 4 sub-strokes have unique IDs, monotonic timestamps, valid caches
    let mut ids = std::collections::HashSet::new();
    for s in &current_strokes {
        assert!(ids.insert(s.id), "Duplicate stroke ID: {}", s.id);
        assert!(s.points.len() >= 2);
        assert!(!s.cached_bezier.is_empty());
        assert!(s.cached_bbox.is_some());

        // Monotonic timestamps
        for w in s.points.windows(2) {
            assert!(
                w[1].timestamp_ns >= w[0].timestamp_ns,
                "Timestamps non-monotonic in sub-stroke: {} -> {}",
                w[0].timestamp_ns,
                w[1].timestamp_ns
            );
        }
    }
}

#[test]
fn test_adversarial_sequential_eraser_sweep_simulation() {
    // Continuous 500px stroke
    let stroke = make_stroke(
        300,
        &[
            (0.0, 100.0),
            (100.0, 100.0),
            (200.0, 100.0),
            (300.0, 100.0),
            (400.0, 100.0),
            (500.0, 100.0),
        ],
        2.0,
    );

    // Simulate an eraser dragging across the stroke in 20 small overlapping steps from x=150 to x=245
    let mut strokes = vec![stroke];
    for step in 0..20 {
        let x = 150.0 + (step as f64) * 5.0; // max x = 150 + 19 * 5 = 245
        let mut next_strokes = Vec::new();
        for s in strokes {
            let cut = s.slice_with_circle((x, 100.0), 10.0);
            next_strokes.extend(cut);
        }
        strokes = next_strokes;
    }

    // Should leave 2 surviving sub-strokes: left side [0, 140] and right side [255, 500]
    assert_eq!(
        strokes.len(),
        2,
        "Eraser drag sweep should leave 2 clean fragments, got {}",
        strokes.len()
    );

    let left = &strokes[0];
    let right = &strokes[1];

    // Check centerline endpoints (points) rather than bbox (which includes base_width/2)
    let left_end_x = left.points.last().unwrap().x;
    let right_start_x = right.points.first().unwrap().x;

    assert!(
        left_end_x <= 140.5,
        "Left fragment centerline leaked into eraser channel: end_x={}",
        left_end_x
    );
    assert!(
        right_start_x >= 254.5,
        "Right fragment centerline leaked into eraser channel: start_x={}",
        right_start_x
    );
}

#[test]
fn test_adversarial_cache_recomputation_after_slicing() {
    // Test that sub-strokes can undergo SQLite serialization-style cache reconstruction
    // (cached_bezier and cached_bbox wiped and recomputed via recompute_cache)
    let stroke = make_stroke(
        400,
        &[
            (0.0, 0.0),
            (50.0, 30.0),
            (100.0, 0.0),
            (150.0, 30.0),
            (200.0, 0.0),
        ],
        2.0,
    );

    let fragments = stroke.slice_with_circle((100.0, 0.0), 15.0);
    assert_eq!(fragments.len(), 2);

    for mut fragment in fragments {
        let original_bbox = fragment.cached_bbox;

        // Wipe cache as happens during SQLite serialization
        fragment.cached_bbox = None;
        fragment.cached_bezier.clear();

        // Recompute
        fragment.recompute_cache();

        assert!(
            fragment.cached_bbox.is_some(),
            "recompute_cache failed on sliced sub-stroke"
        );
        assert!(
            !fragment.cached_bezier.is_empty(),
            "recompute_cache produced 0 Béziers on sliced sub-stroke"
        );

        // Bounding box should be essentially identical
        let new_bbox = fragment.cached_bbox.unwrap();
        let old_bbox = original_bbox.unwrap();
        assert!(
            (new_bbox.min_x - old_bbox.min_x).abs() < 5.0,
            "Recomputed bbox diverged on min_x: {} vs {}",
            new_bbox.min_x,
            old_bbox.min_x
        );
    }
}

// =========================================================================
// Category 6: Extreme Coordinate Scales & Micro/Macro Boundaries
// =========================================================================

#[test]
fn test_adversarial_micro_and_macro_coordinates() {
    // Macro scale coordinates (100,000 pt whiteboard)
    let macro_stroke = make_stroke(
        500,
        &[
            (100_000.0, 100_000.0),
            (100_500.0, 100_000.0),
            (101_000.0, 100_000.0),
        ],
        10.0,
    );

    let res = macro_stroke.slice_with_circle((100_500.0, 100_000.0), 100.0);
    assert_eq!(res.len(), 2, "Macro coordinate stroke slice failed");

    // Micro radius (smaller than dust threshold 1.5px)
    let normal_stroke = make_stroke(501, &[(0.0, 0.0), (50.0, 0.0), (100.0, 0.0)], 2.0);

    // Slicing with micro radius 0.001: cut hole is tiny (< 0.002 px)
    // Both sides are huge, so it should split into 2 sub-strokes or remain 1
    let micro_res = normal_stroke.slice_with_circle((50.0, 0.0), 0.001);
    assert!(
        !micro_res.is_empty(),
        "Micro radius slice wiped the stroke!"
    );

    // Huge radius (1,000,000 pt) completely engulfing the stroke
    let macro_erase = normal_stroke.slice_with_circle((50.0, 0.0), 1_000_000.0);
    assert!(
        macro_erase.is_empty(),
        "Huge radius failed to erase stroke completely"
    );
}
