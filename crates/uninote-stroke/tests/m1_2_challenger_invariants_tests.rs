//! Mathematical Invariants & Property-Based Stress Test Harness for Stroke Slicing
//!
//! Verifies the 5 core mathematical invariants required by Challenger M1_2:
//! 1. Arc length of surviving sub-strokes <= original stroke arc length (+ eps).
//! 2. Every point in surviving strokes is at distance >= R - eps from eraser center.
//! 3. Point timestamps are strictly monotonic.
//! 4. Widths and pressures remain within [min_width, max_width] and [0.0, 1.0].
//! 5. No NaN or infinite floats in points, control points, or bounding boxes.

use std::f64::consts::PI;
use uninote_stroke::geometry::{distance, CubicBezierSegment};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

/// Computes the exact arc length of a sequence of CubicBezierSegments via fine numerical integration.
fn compute_spline_arc_length(segments: &[CubicBezierSegment], samples_per_seg: usize) -> f64 {
    let mut total_len = 0.0;
    for seg in segments {
        let n = samples_per_seg.max(20);
        let mut prev_pt = seg.evaluate(0.0);
        for step in 1..=n {
            let t = (step as f64) / (n as f64);
            let pt = seg.evaluate(t);
            total_len += distance(prev_pt, pt);
            prev_pt = pt;
        }
    }
    total_len
}

/// Helper to build a test stroke with specified points, base width, and strictly monotonic timestamps.
fn make_test_stroke(
    id: u64,
    coords: &[(f64, f64)],
    base_w: f64,
    start_ts_ns: u64,
    dt_ns: u64,
) -> Stroke {
    let style = StrokeStyle {
        color: (0.2, 0.4, 0.8, 1.0),
        base_width: base_w,
        min_width_factor: 0.5,
        max_width_factor: 1.5,
    };
    let points: Vec<StrokePoint> = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let p_norm = 0.1 + 0.8 * ((i as f64) / (coords.len().max(2) as f64));
            let factor = 0.5 + (1.5 - 0.5) * p_norm;
            StrokePoint {
                x,
                y,
                pressure: p_norm,
                width: base_w * factor,
                timestamp_ns: start_ts_ns + (i as u64) * dt_ns,
            }
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

/// Comprehensive validator checking all 5 invariants on surviving sub-strokes.
fn assert_all_invariants(
    original: &Stroke,
    sub_strokes: &[Stroke],
    eraser_center: (f64, f64),
    eraser_radius: f64,
) {
    let min_width = original.style.base_width * original.style.min_width_factor;
    let max_width = original.style.base_width * original.style.max_width_factor;

    // --- Invariant 1: Arc length of surviving sub-strokes <= original stroke arc length ---
    let orig_arc_len = compute_spline_arc_length(&original.cached_bezier, 50);
    let mut total_sub_arc_len = 0.0;
    for sub in sub_strokes {
        let sub_len = compute_spline_arc_length(&sub.cached_bezier, 50);
        total_sub_arc_len += sub_len;
    }

    // Numerical tolerance of 0.25 px for discretization/subdivision rounding
    assert!(
        total_sub_arc_len <= orig_arc_len + 0.25,
        "Invariant 1 Violated (Arc Length): sum(surviving)={:.4} > original={:.4}",
        total_sub_arc_len,
        orig_arc_len
    );

    // --- Invariant 2: Every point in surviving strokes is at distance >= R - epsilon from eraser center ---
    let eps = 0.55; // Root-finding tolerance in slice.rs is 0.5 px
    for (sub_idx, sub) in sub_strokes.iter().enumerate() {
        // Check discrete stroke points
        for (pt_idx, p) in sub.points.iter().enumerate() {
            let dist = distance((p.x, p.y), eraser_center);
            assert!(
                dist >= eraser_radius - eps,
                "Invariant 2 Violated (Discrete Point): sub[{}] point[{}] at ({:.3}, {:.3}) dist={:.4} < R-eps={:.4} (R={:.3})",
                sub_idx,
                pt_idx,
                p.x,
                p.y,
                dist,
                eraser_radius - eps,
                eraser_radius
            );
        }

        // Check densely sampled points along cached Bézier segments
        for (seg_idx, seg) in sub.cached_bezier.iter().enumerate() {
            for step in 0..=20 {
                let u = (step as f64) / 20.0;
                let pt = seg.evaluate(u);
                let dist = distance(pt, eraser_center);
                assert!(
                    dist >= eraser_radius - eps,
                    "Invariant 2 Violated (Bezier Sample): sub[{}] seg[{}] u={:.2} pt=({:.3}, {:.3}) dist={:.4} < R-eps={:.4} (R={:.3})",
                    sub_idx,
                    seg_idx,
                    u,
                    pt.0,
                    pt.1,
                    dist,
                    eraser_radius - eps,
                    eraser_radius
                );
            }
        }
    }

    // --- Invariant 3: Point timestamps are strictly monotonic ---
    for (sub_idx, sub) in sub_strokes.iter().enumerate() {
        for w in sub.points.windows(2) {
            assert!(
                w[1].timestamp_ns > w[0].timestamp_ns,
                "Invariant 3 Violated (Timestamps Not Strictly Monotonic): sub[{}] has ts0={} and ts1={}",
                sub_idx,
                w[0].timestamp_ns,
                w[1].timestamp_ns
            );
        }
    }

    // --- Invariant 4: Widths and pressures remain within [min_width, max_width] and [0.0, 1.0] ---
    for (sub_idx, sub) in sub_strokes.iter().enumerate() {
        for (pt_idx, p) in sub.points.iter().enumerate() {
            assert!(
                p.pressure >= 0.0 && p.pressure <= 1.0,
                "Invariant 4 Violated (Pressure out of bounds): sub[{}] pt[{}] pressure={}",
                sub_idx,
                pt_idx,
                p.pressure
            );
            assert!(
                p.width >= min_width - 1e-6 && p.width <= max_width + 1e-6,
                "Invariant 4 Violated (Width out of bounds): sub[{}] pt[{}] width={} not in [{:.3}, {:.3}]",
                sub_idx,
                pt_idx,
                p.width,
                min_width,
                max_width
            );
        }

        for (seg_idx, seg) in sub.cached_bezier.iter().enumerate() {
            assert!(
                seg.width0 >= min_width - 1e-6 && seg.width0 <= max_width + 1e-6,
                "Invariant 4 Violated (Bezier width0): sub[{}] seg[{}] width0={} not in [{:.3}, {:.3}]",
                sub_idx,
                seg_idx,
                seg.width0,
                min_width,
                max_width
            );
            assert!(
                seg.width1 >= min_width - 1e-6 && seg.width1 <= max_width + 1e-6,
                "Invariant 4 Violated (Bezier width1): sub[{}] seg[{}] width1={} not in [{:.3}, {:.3}]",
                sub_idx,
                seg_idx,
                seg.width1,
                min_width,
                max_width
            );
        }
    }

    // --- Invariant 5: No NaN or infinite floats in points or control points ---
    for (sub_idx, sub) in sub_strokes.iter().enumerate() {
        for (pt_idx, p) in sub.points.iter().enumerate() {
            assert!(
                p.x.is_finite() && p.y.is_finite() && p.pressure.is_finite() && p.width.is_finite(),
                "Invariant 5 Violated (Non-finite float in point): sub[{}] pt[{}]",
                sub_idx,
                pt_idx
            );
        }
        for (seg_idx, seg) in sub.cached_bezier.iter().enumerate() {
            assert!(
                seg.p0.0.is_finite()
                    && seg.p0.1.is_finite()
                    && seg.cp1.0.is_finite()
                    && seg.cp1.1.is_finite()
                    && seg.cp2.0.is_finite()
                    && seg.cp2.1.is_finite()
                    && seg.p1.0.is_finite()
                    && seg.p1.1.is_finite()
                    && seg.width0.is_finite()
                    && seg.width1.is_finite(),
                "Invariant 5 Violated (Non-finite float in bezier segment): sub[{}] seg[{}]",
                sub_idx,
                seg_idx
            );
        }
        if let Some(bbox) = &sub.cached_bbox {
            assert!(
                bbox.min_x.is_finite()
                    && bbox.max_x.is_finite()
                    && bbox.min_y.is_finite()
                    && bbox.max_y.is_finite(),
                "Invariant 5 Violated (Non-finite bbox): sub[{}]",
                sub_idx
            );
        }
    }
}

// =========================================================================
// 1. Comprehensive Generator Suites for Invariant Stress
// =========================================================================

#[test]
fn test_invariants_straight_lines_all_orientations() {
    let angles_deg = [
        0.0, 15.0, 30.0, 45.0, 60.0, 90.0, 120.0, 135.0, 180.0, 225.0, 270.0,
    ];
    for &angle in &angles_deg {
        let rad = angle * PI / 180.0;
        let mut coords = Vec::new();
        for i in 0..10 {
            let d = (i as f64) * 20.0;
            coords.push((100.0 + d * rad.cos(), 100.0 + d * rad.sin()));
        }
        let stroke = make_test_stroke(1, &coords, 3.0, 1_000_000, 8_000_000);

        // Test cuts at various positions along the line
        let cut_fractions = [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
        for &frac in &cut_fractions {
            let cx = 100.0 + (180.0 * frac) * rad.cos();
            let cy = 100.0 + (180.0 * frac) * rad.sin();
            for &r in &[5.0, 15.0, 30.0, 50.0] {
                let sub_strokes = stroke.slice_with_circle((cx, cy), r);
                assert_all_invariants(&stroke, &sub_strokes, (cx, cy), r);
            }
        }
    }
}

#[test]
fn test_invariants_sine_wave_varying_radii() {
    let mut coords = Vec::new();
    for i in 0..50 {
        let x = (i as f64) * 10.0;
        let y = 100.0 + 40.0 * ((i as f64) * 0.25).sin();
        coords.push((x, y));
    }
    let stroke = make_test_stroke(2, &coords, 2.5, 1_000_000, 8_000_000);

    // Sweep eraser across the sine wave at various crests, troughs, and zero-crossings
    let test_points = [
        (50.0, 100.0),
        (100.0, 140.0),
        (150.0, 100.0),
        (200.0, 60.0),
        (250.0, 100.0),
        (300.0, 140.0),
    ];
    for (cx, cy) in test_points {
        for &r in &[10.0, 25.0, 45.0, 70.0] {
            let sub_strokes = stroke.slice_with_circle((cx, cy), r);
            assert_all_invariants(&stroke, &sub_strokes, (cx, cy), r);
        }
    }
}

#[test]
fn test_invariants_spiral_high_curvature() {
    let mut coords = Vec::new();
    for i in 0..60 {
        let theta = (i as f64) * 0.2;
        let r = 10.0 + 3.0 * theta;
        let x = 200.0 + r * theta.cos();
        let y = 200.0 + r * theta.sin();
        coords.push((x, y));
    }
    let stroke = make_test_stroke(3, &coords, 2.0, 1_000_000, 4_000_000);

    // Eraser placed at spiral center, middle winding, and outer edge
    let centers = [
        (200.0, 200.0),
        (220.0, 200.0),
        (160.0, 180.0),
        (240.0, 240.0),
    ];
    for (cx, cy) in centers {
        for &r in &[15.0, 30.0, 50.0] {
            let sub_strokes = stroke.slice_with_circle((cx, cy), r);
            assert_all_invariants(&stroke, &sub_strokes, (cx, cy), r);
        }
    }
}

#[test]
fn test_invariants_zigzag_sharp_turns() {
    let mut coords = Vec::new();
    for i in 0..20 {
        let x = (i as f64) * 20.0;
        let y = if i % 2 == 0 { 50.0 } else { 150.0 };
        coords.push((x, y));
    }
    let stroke = make_test_stroke(4, &coords, 3.5, 1_000_000, 8_000_000);

    for &(cx, cy) in coords.iter().take(19) {
        for &r in &[10.0, 25.0, 50.0] {
            let sub_strokes = stroke.slice_with_circle((cx, cy), r);
            assert_all_invariants(&stroke, &sub_strokes, (cx, cy), r);
        }
    }
}

#[test]
fn test_invariants_dense_sampling_100_points() {
    // Highly dense handwritten-style stroke with 100 points
    let mut coords = Vec::new();
    for i in 0..100 {
        let t = i as f64 * 0.05;
        let x = 50.0 + 30.0 * t + 10.0 * (t * 2.0).cos();
        let y = 50.0 + 15.0 * t + 15.0 * (t * 3.0).sin();
        coords.push((x, y));
    }
    let stroke = make_test_stroke(5, &coords, 2.0, 10_000_000, 4_000_000);

    for step in 0..10 {
        let idx = step * 10;
        let center = coords[idx];
        let sub_strokes = stroke.slice_with_circle(center, 20.0);
        assert_all_invariants(&stroke, &sub_strokes, center, 20.0);
    }
}

#[test]
fn test_invariants_short_2_point_and_3_point_strokes() {
    let coords2 = [(50.0, 50.0), (150.0, 50.0)];
    let s2 = make_test_stroke(6, &coords2, 2.0, 1_000_000, 8_000_000);

    let coords3 = [(50.0, 50.0), (100.0, 100.0), (150.0, 50.0)];
    let s3 = make_test_stroke(7, &coords3, 2.0, 1_000_000, 8_000_000);

    for s in &[s2, s3] {
        // Cut middle
        let res_mid = s.slice_with_circle((100.0, 50.0), 15.0);
        assert_all_invariants(s, &res_mid, (100.0, 50.0), 15.0);

        // Cut start
        let res_start = s.slice_with_circle((50.0, 50.0), 15.0);
        assert_all_invariants(s, &res_start, (50.0, 50.0), 15.0);

        // Cut end
        let res_end = s.slice_with_circle((150.0, 50.0), 15.0);
        assert_all_invariants(s, &res_end, (150.0, 50.0), 15.0);
    }
}

// =========================================================================
// 2. Stress Test for Cascaded & Multi-Cycle Slicing
// =========================================================================

#[test]
fn test_invariants_cascaded_multi_slice_monotonic_arc_length() {
    let mut coords = Vec::new();
    for i in 0..30 {
        coords.push(((i as f64) * 15.0, 100.0 + 30.0 * ((i as f64) * 0.3).sin()));
    }
    let orig = make_test_stroke(10, &coords, 3.0, 1_000_000, 8_000_000);
    let orig_arc_len = compute_spline_arc_length(&orig.cached_bezier, 50);

    let mut current_strokes = vec![orig.clone()];
    let cuts = [
        (80.0, 100.0, 15.0),
        (200.0, 100.0, 18.0),
        (320.0, 100.0, 15.0),
        (140.0, 100.0, 12.0),
    ];

    let mut prev_total_len = orig_arc_len;
    for (cx, cy, r) in cuts {
        let mut next_gen = Vec::new();
        for s in &current_strokes {
            let res = s.slice_with_circle((cx, cy), r);
            assert_all_invariants(s, &res, (cx, cy), r);
            next_gen.extend(res);
        }
        current_strokes = next_gen;

        // Verify total surviving arc length decreases monotonically
        let current_total_len: f64 = current_strokes
            .iter()
            .map(|s| compute_spline_arc_length(&s.cached_bezier, 50))
            .sum();

        assert!(
            current_total_len <= prev_total_len + 0.1,
            "Cascaded arc length increased: prev={:.4}, current={:.4}",
            prev_total_len,
            current_total_len
        );
        prev_total_len = current_total_len;
    }
}

// =========================================================================
// 3. Pathological Edge Cases & Invariant Robustness
// =========================================================================

#[test]
fn test_invariants_tangent_grazing_circle() {
    // Parabolic arc with apex at (100, 50)
    let coords = [
        (0.0, 100.0),
        (50.0, 60.0),
        (100.0, 50.0),
        (150.0, 60.0),
        (200.0, 100.0),
    ];
    let stroke = make_test_stroke(20, &coords, 2.0, 1_000_000, 8_000_000);

    // Tangent circle touching apex from outside (center at (100, 30), radius 20.0 -> distance is exactly 20.0)
    let res = stroke.slice_with_circle((100.0, 30.0), 20.0);
    // Should preserve stroke intact without splitting into dust
    assert_all_invariants(&stroke, &res, (100.0, 30.0), 20.0);
    assert_eq!(res.len(), 1);
}

#[test]
fn test_invariants_pathological_floats_and_zero_radius() {
    let coords = [(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)];
    let stroke = make_test_stroke(30, &coords, 2.0, 1_000_000, 8_000_000);

    // Zero radius -> stroke untouched
    let res_zero = stroke.slice_with_circle((50.0, 50.0), 0.0);
    assert_eq!(res_zero.len(), 1);
    assert_eq!(res_zero[0].points.len(), 3);

    // Negative radius -> stroke untouched
    let res_neg = stroke.slice_with_circle((50.0, 50.0), -15.0);
    assert_eq!(res_neg.len(), 1);

    // NaN / Inf inputs -> stroke untouched
    let res_nan = stroke.slice_with_circle((f64::NAN, 50.0), 10.0);
    assert_eq!(res_nan.len(), 1);

    let res_inf = stroke.slice_with_circle((50.0, 50.0), f64::INFINITY);
    assert_eq!(res_inf.len(), 1);
}

// =========================================================================
// 4. Large-Scale Randomized Fuzzing Suite (500 Cuts)
// =========================================================================

struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u32(&mut self) -> u32 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        (self.state >> 32) as u32
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u32() as f64) / (u32::MAX as f64)
    }

    fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + self.next_f64() * (max - min)
    }
}

#[test]
fn test_invariants_randomized_fuzzing_500_cuts() {
    let mut rng = SimpleRng::new(0xDEADBEEF_CAFE1234);

    for run in 0..500 {
        let num_points = 3 + (rng.next_u32() % 15) as usize;
        let mut coords = Vec::with_capacity(num_points);
        let mut cur_x = rng.range_f64(50.0, 200.0);
        let mut cur_y = rng.range_f64(50.0, 200.0);
        coords.push((cur_x, cur_y));

        for _ in 1..num_points {
            cur_x += rng.range_f64(-30.0, 30.0);
            cur_y += rng.range_f64(-30.0, 30.0);
            coords.push((cur_x, cur_y));
        }

        let base_w = rng.range_f64(1.0, 6.0);
        let stroke = make_test_stroke(1000 + run as u64, &coords, base_w, 1_000_000, 8_000_000);

        // Pick a random cut center near the stroke
        let pick_pt = &coords[rng.next_u32() as usize % coords.len()];
        let cx = pick_pt.0 + rng.range_f64(-15.0, 15.0);
        let cy = pick_pt.1 + rng.range_f64(-15.0, 15.0);
        let r = rng.range_f64(3.0, 40.0);

        let sub_strokes = stroke.slice_with_circle((cx, cy), r);
        assert_all_invariants(&stroke, &sub_strokes, (cx, cy), r);
    }
}

// =========================================================================
// 5. High-Curvature Loop, Dot Stroke & ID Uniqueness
// =========================================================================

#[test]
fn test_invariants_figure_eight_self_intersection() {
    let mut coords = Vec::new();
    for i in 0..40 {
        let t = (i as f64) * 2.0 * PI / 40.0;
        let x = 200.0 + 80.0 * t.sin();
        let y = 200.0 + 80.0 * t.sin() * t.cos();
        coords.push((x, y));
    }
    let stroke = make_test_stroke(50, &coords, 2.5, 1_000_000, 4_000_000);

    // Cut at the center crossing
    let res_center = stroke.slice_with_circle((200.0, 200.0), 20.0);
    assert_all_invariants(&stroke, &res_center, (200.0, 200.0), 20.0);

    // Cut one lobe
    let res_lobe = stroke.slice_with_circle((260.0, 200.0), 15.0);
    assert_all_invariants(&stroke, &res_lobe, (260.0, 200.0), 15.0);
}

#[test]
fn test_invariants_single_point_dot_stroke_and_empty() {
    let style = StrokeStyle::default();
    let dot_p = StrokePoint {
        x: 100.0,
        y: 100.0,
        pressure: 0.7,
        width: 4.0,
        timestamp_ns: 500_000,
    };
    let dot_stroke = Stroke {
        id: 60,
        style: style.clone(),
        points: vec![dot_p],
        raw_samples: vec![InputSample::new(
            (100.0, 100.0),
            Some(0.7),
            500_000,
            DeviceKind::Stylus,
            InputPhase::Down,
        )],
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // Erase inside effective radius (R + width/2 = 10 + 2 = 12)
    // Distance = 8 < 12 -> erased
    let res_erased = dot_stroke.slice_with_circle((105.0, 100.0), 10.0);
    assert!(
        res_erased.is_empty(),
        "Dot inside eraser should be completely erased"
    );

    // Distance = 15 > 12 -> untouched
    let res_intact = dot_stroke.slice_with_circle((115.0, 100.0), 10.0);
    assert_eq!(res_intact.len(), 1);
    assert_eq!(res_intact[0].id, 60);

    // Empty stroke -> empty
    let empty_stroke = Stroke {
        id: 61,
        style,
        points: Vec::new(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    let res_empty = empty_stroke.slice_with_circle((50.0, 50.0), 20.0);
    assert!(res_empty.is_empty());
}

#[test]
fn test_invariants_single_slice_id_uniqueness() {
    let coords = [
        (0.0, 0.0),
        (100.0, 0.0),
        (200.0, 0.0),
        (300.0, 0.0),
        (400.0, 0.0),
    ];
    let stroke = make_test_stroke(70, &coords, 2.0, 1_000_000, 8_000_000);

    // Cut into 2 fragments
    let res2 = stroke.slice_with_circle((200.0, 0.0), 20.0);
    assert_eq!(res2.len(), 2);
    assert_ne!(
        res2[0].id, res2[1].id,
        "Sub-stroke IDs must be unique within slice"
    );

    // Verify first keeps original ID
    assert_eq!(res2[0].id, 70);
}

#[test]
fn test_invariants_timestamp_resolution_limits() {
    // Test with various delta timestamps: 1ms (standard), 10us (fast digitizer), 1us (microsecond)
    for &dt_ns in &[1_000_000, 10_000, 1_000] {
        let coords = [(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (300.0, 0.0)];
        let stroke = make_test_stroke(80, &coords, 2.0, 1_000_000, dt_ns);

        let res = stroke.slice_with_circle((150.0, 0.0), 20.0);
        for sub in &res {
            for w in sub.points.windows(2) {
                assert!(
                    w[1].timestamp_ns > w[0].timestamp_ns,
                    "Timestamp not strictly monotonic for dt_ns={}: ts0={}, ts1={}",
                    dt_ns,
                    w[0].timestamp_ns,
                    w[1].timestamp_ns
                );
            }
        }
    }
}
