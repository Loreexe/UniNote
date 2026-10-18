use std::f64::consts::{PI, SQRT_2};
use uninote_stroke::geometry::{
    catmull_rom_to_bezier, sample_catmull_rom_spline, CubicBezierSegment,
};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

/// Helper: genera un punto StrokePoint
fn make_point(x: f64, y: f64, width: f64) -> StrokePoint {
    StrokePoint {
        x,
        y,
        pressure: 0.5,
        width,
        timestamp_ns: 0,
    }
}

/// Helper: genera un tratto con N punti distribuiti su una sinusoide smorzata
fn generate_curved_stroke(id: u64, n: usize, mode: PipelineMode) -> Stroke {
    let mut points = Vec::with_capacity(n);
    let mut raw_samples = Vec::with_capacity(n);

    for i in 0..n {
        let t = i as f64;
        let x = 100.0 + t * 4.0;
        let y = 200.0 + (t * 0.15).sin() * 40.0 + (t * 0.03).cos() * 15.0;
        let pressure = 0.2 + 0.6 * ((t * 0.08).sin().abs());
        let width = 1.5 + pressure * 2.5;

        points.push(StrokePoint {
            x,
            y,
            pressure,
            width,
            timestamp_ns: (i * 5_000_000) as u64,
        });

        raw_samples.push(InputSample::new(
            (x, y),
            Some(pressure),
            (i * 5_000_000) as u64,
            DeviceKind::Stylus,
            if i == 0 {
                InputPhase::Down
            } else if i == n - 1 {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        ));
    }

    let is_dot = n == 1;
    let mut stroke = Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples,
        is_dot,
        mode,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

/// Helper: genera un tratto con N punti a coordinate diadiche esatte (potenze di 2)
fn generate_dyadic_stroke(id: u64, n: usize) -> Stroke {
    let mut points = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64;
        // 12.5 = 25/2, 6.25 = 25/4: frazioni diadiche esatte in IEEE-754
        let x = 64.0 + t * 12.5;
        let y = 128.0 + t * 6.25;
        points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: (i * 1000) as u64,
        });
    }

    let is_dot = n == 1;
    let mut stroke = Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples: Vec::new(),
        is_dot,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// =========================================================================
// SUITE 1: 50,000 CONSECUTIVE IN-PLACE TRANSLATION CYCLES (FORWARD & BACK)
// =========================================================================

/// Test 1.1: 50,000 cicli di traslazione su tratto con 1 punto (dot vs non-dot)
#[test]
fn test_50k_translation_cycles_1_point() {
    let step_x = 15.625; // 125 / 8
    let step_y = -8.375; // -67 / 8

    // Caso 1: is_dot = true
    let mut dot_stroke = Stroke {
        id: 1,
        style: StrokeStyle::default(),
        points: vec![make_point(100.0, 200.0, 4.0)],
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    dot_stroke.recompute_cache();
    let orig_dot_bbox = dot_stroke.cached_bbox.unwrap();

    for _ in 0..50_000 {
        dot_stroke.translate(step_x, step_y);
        dot_stroke.translate(-step_x, -step_y);
    }

    assert!(
        (dot_stroke.points[0].x - 100.0).abs() < 1e-14,
        "Dot point X drifted: {:e}",
        (dot_stroke.points[0].x - 100.0).abs()
    );
    assert!(
        (dot_stroke.points[0].y - 200.0).abs() < 1e-14,
        "Dot point Y drifted: {:e}",
        (dot_stroke.points[0].y - 200.0).abs()
    );

    let cur_bbox = dot_stroke.cached_bbox.unwrap();
    assert!((cur_bbox.min_x - orig_dot_bbox.min_x).abs() < 1e-14);
    assert!((cur_bbox.max_x - orig_dot_bbox.max_x).abs() < 1e-14);
    assert!((cur_bbox.min_y - orig_dot_bbox.min_y).abs() < 1e-14);
    assert!((cur_bbox.max_y - orig_dot_bbox.max_y).abs() < 1e-14);
    assert!(dot_stroke.cached_bezier.is_empty());

    // Caso 2: is_dot = false, 1 punto, CatmullRomSpline
    let mut single_stroke = Stroke {
        id: 2,
        style: StrokeStyle::default(),
        points: vec![make_point(300.0, 400.0, 3.0)],
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    single_stroke.recompute_cache();

    for _ in 0..50_000 {
        single_stroke.translate(step_x, step_y);
        single_stroke.translate(-step_x, -step_y);
    }

    assert!((single_stroke.points[0].x - 300.0).abs() < 1e-14);
    assert!((single_stroke.points[0].y - 400.0).abs() < 1e-14);
    assert!(single_stroke.cached_bezier.is_empty());
}

/// Test 1.2: 50,000 cicli di traslazione su tratto con 2 punti (singolo segmento Bézier)
#[test]
fn test_50k_translation_cycles_2_points() {
    // 1. Coordinate diadiche esatte
    let mut dyadic = generate_dyadic_stroke(201, 2);
    let orig_p0 = dyadic.points[0].clone();
    let orig_p1 = dyadic.points[1].clone();
    let orig_bbox = dyadic.cached_bbox.unwrap();
    let orig_bezier = dyadic.cached_bezier[0].clone();

    let step_x = 31.25;
    let step_y = -18.75;

    for _ in 0..50_000 {
        dyadic.translate(step_x, step_y);
        dyadic.translate(-step_x, -step_y);
    }

    assert!((dyadic.points[0].x - orig_p0.x).abs() < 1e-14);
    assert!((dyadic.points[0].y - orig_p0.y).abs() < 1e-14);
    assert!((dyadic.points[1].x - orig_p1.x).abs() < 1e-14);
    assert!((dyadic.points[1].y - orig_p1.y).abs() < 1e-14);

    let cur_bbox = dyadic.cached_bbox.unwrap();
    assert!((cur_bbox.min_x - orig_bbox.min_x).abs() < 1e-14);
    assert!((cur_bbox.max_x - orig_bbox.max_x).abs() < 1e-14);
    assert!((cur_bbox.min_y - orig_bbox.min_y).abs() < 1e-14);
    assert!((cur_bbox.max_y - orig_bbox.max_y).abs() < 1e-14);

    let cur_seg = &dyadic.cached_bezier[0];
    assert!((cur_seg.p0.0 - orig_bezier.p0.0).abs() < 1e-14);
    assert!((cur_seg.p0.1 - orig_bezier.p0.1).abs() < 1e-14);
    assert!((cur_seg.cp1.0 - orig_bezier.cp1.0).abs() < 1e-14);
    assert!((cur_seg.cp1.1 - orig_bezier.cp1.1).abs() < 1e-14);
    assert!((cur_seg.cp2.0 - orig_bezier.cp2.0).abs() < 1e-14);
    assert!((cur_seg.cp2.1 - orig_bezier.cp2.1).abs() < 1e-14);
    assert!((cur_seg.p1.0 - orig_bezier.p1.0).abs() < 1e-14);
    assert!((cur_seg.p1.1 - orig_bezier.p1.1).abs() < 1e-14);

    // 2. Coordinate trascendenti (offset irrazionali PI, SQRT_2)
    let mut curved = generate_curved_stroke(202, 2, PipelineMode::CatmullRomSpline);
    let orig_curved_bezier = curved.cached_bezier[0].clone();
    let trans_x = PI;
    let trans_y = -SQRT_2;

    for _ in 0..50_000 {
        curved.translate(trans_x, trans_y);
        curved.translate(-trans_x, -trans_y);
    }

    // Su 100.000 addizioni/sottrazioni con numeri trascendenti, la precisione IEEE-754 accumula <= 1e-11
    let cur_cseg = &curved.cached_bezier[0];
    assert!(
        (cur_cseg.p0.0 - orig_curved_bezier.p0.0).abs() < 1e-11,
        "Transcendental drift p0.0: {:e}",
        (cur_cseg.p0.0 - orig_curved_bezier.p0.0).abs()
    );
    assert!(
        (cur_cseg.cp1.0 - orig_curved_bezier.cp1.0).abs() < 1e-11,
        "Transcendental drift cp1.0: {:e}",
        (cur_cseg.cp1.0 - orig_curved_bezier.cp1.0).abs()
    );
    assert!(
        (cur_cseg.cp2.1 - orig_curved_bezier.cp2.1).abs() < 1e-11,
        "Transcendental drift cp2.1: {:e}",
        (cur_cseg.cp2.1 - orig_curved_bezier.cp2.1).abs()
    );
    assert!(
        (cur_cseg.p1.1 - orig_curved_bezier.p1.1).abs() < 1e-11,
        "Transcendental drift p1.1: {:e}",
        (cur_cseg.p1.1 - orig_curved_bezier.p1.1).abs()
    );
}

/// Test 1.3: 50,000 cicli di traslazione su tratto con 10 punti (tratto tipico scrittura)
#[test]
fn test_50k_translation_cycles_10_points() {
    let mut stroke = generate_curved_stroke(101, 10, PipelineMode::CatmullRomSpline);
    assert_eq!(stroke.cached_bezier.len(), 9);

    let orig_points = stroke.points.clone();
    let orig_bbox = stroke.cached_bbox.unwrap();
    let orig_bezier = stroke.cached_bezier.clone();

    let step_x = 12.375; // diadico
    let step_y = -9.125; // diadico

    for _ in 0..50_000 {
        stroke.translate(step_x, step_y);
        stroke.translate(-step_x, -step_y);
    }

    for (p_curr, p_orig) in stroke.points.iter().zip(orig_points.iter()) {
        assert!(
            (p_curr.x - p_orig.x).abs() < 1e-13,
            "10-point stroke X drift exceeded: {:e}",
            (p_curr.x - p_orig.x).abs()
        );
        assert!(
            (p_curr.y - p_orig.y).abs() < 1e-13,
            "10-point stroke Y drift exceeded: {:e}",
            (p_curr.y - p_orig.y).abs()
        );
    }

    let cur_bbox = stroke.cached_bbox.unwrap();
    assert!((cur_bbox.min_x - orig_bbox.min_x).abs() < 1e-13);
    assert!((cur_bbox.max_x - orig_bbox.max_x).abs() < 1e-13);
    assert!((cur_bbox.min_y - orig_bbox.min_y).abs() < 1e-13);
    assert!((cur_bbox.max_y - orig_bbox.max_y).abs() < 1e-13);

    for (b_curr, b_orig) in stroke.cached_bezier.iter().zip(orig_bezier.iter()) {
        assert!((b_curr.p0.0 - b_orig.p0.0).abs() < 1e-13);
        assert!((b_curr.p0.1 - b_orig.p0.1).abs() < 1e-13);
        assert!((b_curr.cp1.0 - b_orig.cp1.0).abs() < 1e-13);
        assert!((b_curr.cp1.1 - b_orig.cp1.1).abs() < 1e-13);
        assert!((b_curr.cp2.0 - b_orig.cp2.0).abs() < 1e-13);
        assert!((b_curr.cp2.1 - b_orig.cp2.1).abs() < 1e-13);
        assert!((b_curr.p1.0 - b_orig.p1.0).abs() < 1e-13);
        assert!((b_curr.p1.1 - b_orig.p1.1).abs() < 1e-13);
    }
}

/// Test 1.4: 50,000 cicli di traslazione su tratto denso con 1.000 punti (stress massivo)
#[test]
fn test_50k_translation_cycles_1000_points() {
    let mut stroke = generate_curved_stroke(1001, 1_000, PipelineMode::CatmullRomSpline);
    assert_eq!(stroke.points.len(), 1_000);
    assert_eq!(stroke.cached_bezier.len(), 999);

    let orig_p0 = stroke.points[0].clone();
    let orig_pmid = stroke.points[500].clone();
    let orig_pend = stroke.points[999].clone();
    let orig_bbox = stroke.cached_bbox.unwrap();
    let orig_b0 = stroke.cached_bezier[0].clone();
    let orig_bmid = stroke.cached_bezier[500].clone();
    let orig_bend = stroke.cached_bezier[998].clone();

    let step_x = 7.5;
    let step_y = -3.25;

    for _ in 0..50_000 {
        stroke.translate(step_x, step_y);
        stroke.translate(-step_x, -step_y);
    }

    // Verifica punti cardine
    assert!((stroke.points[0].x - orig_p0.x).abs() < 1e-13);
    assert!((stroke.points[0].y - orig_p0.y).abs() < 1e-13);
    assert!((stroke.points[500].x - orig_pmid.x).abs() < 1e-13);
    assert!((stroke.points[500].y - orig_pmid.y).abs() < 1e-13);
    assert!((stroke.points[999].x - orig_pend.x).abs() < 1e-13);
    assert!((stroke.points[999].y - orig_pend.y).abs() < 1e-13);

    // Verifica bounding box
    let cur_bbox = stroke.cached_bbox.unwrap();
    assert!((cur_bbox.min_x - orig_bbox.min_x).abs() < 1e-13);
    assert!((cur_bbox.max_x - orig_bbox.max_x).abs() < 1e-13);
    assert!((cur_bbox.min_y - orig_bbox.min_y).abs() < 1e-13);
    assert!((cur_bbox.max_y - orig_bbox.max_y).abs() < 1e-13);

    // Verifica segmenti Bézier cardine
    assert!((stroke.cached_bezier[0].cp1.0 - orig_b0.cp1.0).abs() < 1e-13);
    assert!((stroke.cached_bezier[0].cp2.1 - orig_b0.cp2.1).abs() < 1e-13);
    assert!((stroke.cached_bezier[500].cp1.0 - orig_bmid.cp1.0).abs() < 1e-13);
    assert!((stroke.cached_bezier[500].cp2.1 - orig_bmid.cp2.1).abs() < 1e-13);
    let diff = (stroke.cached_bezier[998].cp1.0 - orig_bend.cp1.0).abs();
    println!(
        "Drift at seg 998 (x ~ 4096) after 50,000 cycles: {:e}",
        diff
    );
    assert!(diff < 1e-11, "Drift at seg 998 exceeded 1e-11: {:e}", diff);
    assert!((stroke.cached_bezier[998].cp2.1 - orig_bend.cp2.1).abs() < 1e-11);
}

// =========================================================================
// SUITE 2: EXTREME POINTS, SUB-PIXEL & BOUNDARY GEOMETRY
// =========================================================================

/// Test 2.1: Coordinate sub-pixel ultra-precise (1e-6 px)
#[test]
fn test_extreme_sub_pixel_coordinates() {
    let pts = vec![
        make_point(1e-6, 1e-6, 1e-5),
        make_point(2e-6, 3e-6, 1e-5),
        make_point(4e-6, 7e-6, 1e-5),
        make_point(8e-6, 1.2e-5, 1e-5),
    ];
    let mut stroke = Stroke::with_points(301, StrokeStyle::default(), pts);

    assert!(stroke.cached_bbox.is_some());
    assert_eq!(stroke.cached_bezier.len(), 3);

    let bbox = stroke.cached_bbox.unwrap();
    assert!(bbox.min_x < 1e-6);
    assert!(bbox.max_x > 8e-6);
    assert!(!bbox.min_x.is_nan() && !bbox.min_x.is_infinite());

    for seg in &stroke.cached_bezier {
        assert!(!seg.p0.0.is_nan());
        assert!(!seg.cp1.0.is_nan());
        assert!(!seg.cp2.0.is_nan());
        assert!(!seg.p1.0.is_nan());
        let (eval_x, eval_y) = seg.evaluate(0.5);
        assert!(!eval_x.is_nan() && !eval_y.is_nan());
    }

    // Traslazione con offset sub-pixel
    stroke.translate(1e-7, -1e-7);
    let translated_bbox = stroke.cached_bbox.unwrap();
    assert!((translated_bbox.min_x - (bbox.min_x + 1e-7)).abs() < 1e-14);
}

/// Test 2.2: Coordinate astronomiche (+- 10^9)
#[test]
fn test_extreme_astronomical_coordinates() {
    let large_x = 1_000_000_000.0; // 10^9
    let large_y = -1_000_000_000.0; // -10^9

    let pts = vec![
        make_point(large_x, large_y, 2.0),
        make_point(large_x + 100.0, large_y + 150.0, 2.5),
        make_point(large_x + 250.0, large_y + 300.0, 3.0),
        make_point(large_x + 500.0, large_y + 600.0, 3.5),
    ];
    let mut stroke = Stroke::with_points(302, StrokeStyle::default(), pts);

    assert!(stroke.cached_bbox.is_some());
    assert_eq!(stroke.cached_bezier.len(), 3);

    let bbox = stroke.cached_bbox.unwrap();
    assert!((bbox.min_x - (large_x - 1.0)).abs() < 1e-6);
    assert!((bbox.min_y - (large_y - 1.0)).abs() < 1e-6);

    for seg in &stroke.cached_bezier {
        assert!(!seg.p0.0.is_nan() && !seg.p0.0.is_infinite());
        assert!(!seg.cp1.0.is_nan() && !seg.cp1.0.is_infinite());
        assert!(!seg.cp2.0.is_nan() && !seg.cp2.0.is_infinite());
        assert!(!seg.p1.0.is_nan() && !seg.p1.0.is_infinite());
    }

    // In-place translate con delta astronomico
    stroke.translate(-large_x, -large_y);
    assert!((stroke.points[0].x - 0.0).abs() < 1e-9);
    assert!((stroke.points[0].y - 0.0).abs() < 1e-9);

    let origin_bbox = stroke.cached_bbox.unwrap();
    assert!(origin_bbox.contains_point(0.0, 0.0));
}

/// Test 2.3: Zero-length segments e tratti con punti coincidenti
#[test]
fn test_zero_length_segments_and_coincident_points() {
    // 4 punti tutti esattamente identici
    let pts = vec![
        make_point(150.0, 250.0, 2.0),
        make_point(150.0, 250.0, 2.0),
        make_point(150.0, 250.0, 2.0),
        make_point(150.0, 250.0, 2.0),
    ];
    let stroke = Stroke::with_points(303, StrokeStyle::default(), pts);

    assert_eq!(stroke.cached_bezier.len(), 3);
    for seg in &stroke.cached_bezier {
        assert_eq!(seg.p0, (150.0, 250.0));
        assert_eq!(seg.cp1, (150.0, 250.0));
        assert_eq!(seg.cp2, (150.0, 250.0));
        assert_eq!(seg.p1, (150.0, 250.0));
        assert_eq!(seg.approximate_length(), 0.0);

        let (x, y) = seg.evaluate(0.5);
        assert_eq!(x, 150.0);
        assert_eq!(y, 250.0);
    }

    assert_eq!(stroke.total_length(), 0.0);

    // Tratto con punti duplicati alternati a punti distinti
    let pts_alt = vec![
        make_point(10.0, 10.0, 2.0),
        make_point(10.0, 10.0, 2.0),
        make_point(50.0, 60.0, 2.0),
        make_point(50.0, 60.0, 2.0),
    ];
    let stroke_alt = Stroke::with_points(304, StrokeStyle::default(), pts_alt);
    assert_eq!(stroke_alt.cached_bezier.len(), 3);

    for seg in &stroke_alt.cached_bezier {
        assert!(!seg.p0.0.is_nan());
        assert!(!seg.cp1.0.is_nan());
        assert!(!seg.cp2.0.is_nan());
        assert!(!seg.p1.0.is_nan());
    }
}

/// Test 2.4: Empty stroke e Single-point degenerate robustness
#[test]
fn test_empty_and_single_point_stroke_robustness() {
    // Empty stroke
    let mut empty = Stroke::new(401, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    assert_eq!(empty.bounding_box(), None);
    assert!(empty.cached_bezier.is_empty());
    assert_eq!(empty.total_length(), 0.0);
    assert!(!empty.intersects_point((10.0, 10.0), 5.0));

    // Traslazione di stroke vuoto: safe, zero panic
    empty.translate(100.0, -100.0);
    assert_eq!(empty.bounding_box(), None);
    assert!(empty.cached_bezier.is_empty());

    // Idempotenza di ensure_cache su stroke vuoto
    empty.ensure_cache();
    assert_eq!(empty.bounding_box(), None);
    assert!(empty.cached_bezier.is_empty());

    // Single point (non-dot)
    let mut single = Stroke::with_points(
        402,
        StrokeStyle::default(),
        vec![make_point(50.0, 50.0, 2.0)],
    );
    assert!(single.cached_bbox.is_some());
    assert!(single.cached_bezier.is_empty());

    single.translate(25.0, 25.0);
    assert_eq!(single.points[0].x, 75.0);
    assert_eq!(single.points[0].y, 75.0);
    assert!(single.bounding_box().unwrap().contains_point(75.0, 75.0));
    single.ensure_cache();
    assert!(single.cached_bezier.is_empty());
}

// =========================================================================
// SUITE 3: BERNSTEIN POLYNOMIAL PARTITION OF UNITY & SPLINE AFFINE INVARIANCE
// =========================================================================

/// Test 3.1: Partizione dell'unità dei polinomi di Bernstein cubici: sum(B_{i,3}(t)) == 1.0
#[test]
fn test_bernstein_polynomial_partition_of_unity() {
    // Campiona t da 0.0 a 1.0 a intervalli densi (10.000 campioni)
    let steps = 10_000;
    for s in 0..=steps {
        let t = s as f64 / steps as f64;
        let u = 1.0 - t;

        let b0 = u * u * u;
        let b1 = 3.0 * u * u * t;
        let b2 = 3.0 * u * t * t;
        let b3 = t * t * t;

        let sum = b0 + b1 + b2 + b3;
        assert!(
            (sum - 1.0).abs() < 1e-15,
            "Partition of unity violated at t={}: sum={}",
            t,
            sum
        );
    }
}

/// Test 3.2: Invarianza affine di CubicBezierSegment: seg.translate(dx, dy).evaluate(t) == seg.evaluate(t) + (dx, dy)
#[test]
fn test_cubic_bezier_affine_invariance_oracle() {
    let seg = CubicBezierSegment {
        p0: (12.34, 56.78),
        cp1: (23.45, 67.89),
        cp2: (45.67, 89.01),
        p1: (78.90, 12.34),
        width0: 2.0,
        width1: 4.5,
    };

    let dx = 137.891;
    let dy = -42.109;

    let mut translated_seg = seg.clone();
    translated_seg.translate(dx, dy);

    for s in 0..=1000 {
        let t = s as f64 / 1000.0;
        let orig_pt = seg.evaluate(t);
        let trans_pt = translated_seg.evaluate(t);

        let expected_x = orig_pt.0 + dx;
        let expected_y = orig_pt.1 + dy;

        assert!(
            (trans_pt.0 - expected_x).abs() < 1e-12,
            "Affine invariance violated along X at t={}: trans={}, expected={}",
            t,
            trans_pt.0,
            expected_x
        );
        assert!(
            (trans_pt.1 - expected_y).abs() < 1e-12,
            "Affine invariance violated along Y at t={}: trans={}, expected={}",
            t,
            trans_pt.1,
            expected_y
        );

        // Larghezza è invariante alla traslazione
        assert_eq!(seg.evaluate_width(t), translated_seg.evaluate_width(t));
    }
}

/// Test 3.3: Invarianza Catmull-Rom: In-place translation matches fresh Catmull-Rom re-evaluation
#[test]
fn test_catmull_rom_in_place_translation_equivalence_oracle() {
    let stroke = generate_curved_stroke(501, 50, PipelineMode::CatmullRomSpline);

    let dx = 88.5;
    let dy = -33.25;

    // Metodo A: In-place translate
    let mut stroke_inplace = stroke.clone();
    stroke_inplace.translate(dx, dy);

    // Metodo B: Ricalcolo spline integrale sui punti traslati
    let fresh_bezier = catmull_rom_to_bezier(&stroke_inplace.points);
    let fresh_bbox = stroke_inplace.compute_bounding_box().unwrap();

    // 1. Verifica identità geometrica dei punti di controllo Bézier
    assert_eq!(stroke_inplace.cached_bezier.len(), fresh_bezier.len());
    for (idx, (in_seg, fresh_seg)) in stroke_inplace
        .cached_bezier
        .iter()
        .zip(fresh_bezier.iter())
        .enumerate()
    {
        assert!(
            (in_seg.p0.0 - fresh_seg.p0.0).abs() < 1e-12,
            "Seg {} p0.0 mismatch: in={}, fresh={}",
            idx,
            in_seg.p0.0,
            fresh_seg.p0.0
        );
        assert!((in_seg.p0.1 - fresh_seg.p0.1).abs() < 1e-12);
        assert!(
            (in_seg.cp1.0 - fresh_seg.cp1.0).abs() < 1e-12,
            "Seg {} cp1.0 mismatch: in={}, fresh={}",
            idx,
            in_seg.cp1.0,
            fresh_seg.cp1.0
        );
        assert!((in_seg.cp1.1 - fresh_seg.cp1.1).abs() < 1e-12);
        assert!(
            (in_seg.cp2.0 - fresh_seg.cp2.0).abs() < 1e-12,
            "Seg {} cp2.0 mismatch: in={}, fresh={}",
            idx,
            in_seg.cp2.0,
            fresh_seg.cp2.0
        );
        assert!((in_seg.cp2.1 - fresh_seg.cp2.1).abs() < 1e-12);
        assert!((in_seg.p1.0 - fresh_seg.p1.0).abs() < 1e-12);
        assert!((in_seg.p1.1 - fresh_seg.p1.1).abs() < 1e-12);
    }

    // 2. Verifica identità bounding box
    let in_bbox = stroke_inplace.cached_bbox.unwrap();
    assert!((in_bbox.min_x - fresh_bbox.min_x).abs() < 1e-12);
    assert!((in_bbox.max_x - fresh_bbox.max_x).abs() < 1e-12);
    assert!((in_bbox.min_y - fresh_bbox.min_y).abs() < 1e-12);
    assert!((in_bbox.max_y - fresh_bbox.max_y).abs() < 1e-12);

    // 3. Campionamento denso lungo tutta la spline
    let sampled_inplace = sample_catmull_rom_spline(&stroke_inplace.points, 2.0);
    // Valutazione diretta dalle curve in cache
    let mut sampled_from_cache = Vec::new();
    for (idx, seg) in stroke_inplace.cached_bezier.iter().enumerate() {
        let len = seg.approximate_length();
        let steps = ((len / 2.0).ceil() as usize).clamp(1, 32);
        let start_step = if idx == 0 { 0 } else { 1 };
        for s in start_step..=steps {
            let t = (s as f64) / (steps as f64);
            let (x, y) = seg.evaluate(t);
            let w = seg.evaluate_width(t);
            sampled_from_cache.push((x, y, w));
        }
    }

    assert_eq!(sampled_inplace.len(), sampled_from_cache.len());
    for (idx, (p_spline, p_cache)) in sampled_inplace
        .iter()
        .zip(sampled_from_cache.iter())
        .enumerate()
    {
        assert!(
            (p_spline.0 - p_cache.0).abs() < 1e-12,
            "Sample {} X mismatch: spline={}, cache={}",
            idx,
            p_spline.0,
            p_cache.0
        );
        assert!(
            (p_spline.1 - p_cache.1).abs() < 1e-12,
            "Sample {} Y mismatch: spline={}, cache={}",
            idx,
            p_spline.1,
            p_cache.1
        );
        assert_eq!(p_spline.2, p_cache.2);
    }
}
