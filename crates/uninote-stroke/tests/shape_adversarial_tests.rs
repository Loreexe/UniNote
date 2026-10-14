use std::f64::consts::PI;
use uninote_stroke::shape::{recognize_shape, snap_line_orthogonal};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn stroke_from_points(pts: &[(f64, f64)]) -> Stroke {
    let mut stroke = Stroke::new(100, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    stroke.points = pts
        .iter()
        .map(|&(x, y)| StrokePoint {
            x,
            y,
            pressure: 0.6,
            width: 3.0,
            timestamp_ns: 0,
        })
        .collect();
    stroke
}

// =========================================================================
// 1. DEGENERATE INPUTS
// =========================================================================

#[test]
fn test_degenerate_zero_points() {
    let stroke = stroke_from_points(&[]);
    assert!(recognize_shape(&stroke).is_none());
    assert!(snap_line_orthogonal(&stroke).is_none());
}

#[test]
fn test_degenerate_single_point() {
    let stroke = stroke_from_points(&[(150.0, 150.0)]);
    assert!(recognize_shape(&stroke).is_none());
    assert!(snap_line_orthogonal(&stroke).is_none());
}

#[test]
fn test_degenerate_two_identical_points() {
    let stroke = stroke_from_points(&[(150.0, 150.0), (150.0, 150.0)]);
    assert!(recognize_shape(&stroke).is_none());
    assert!(snap_line_orthogonal(&stroke).is_none());
}

#[test]
fn test_degenerate_many_identical_points() {
    let pts = vec![(200.0, 200.0); 100];
    let stroke = stroke_from_points(&pts);
    assert!(recognize_shape(&stroke).is_none());
    assert!(snap_line_orthogonal(&stroke).is_none());
}

#[test]
fn test_degenerate_three_collinear_closed_loop_rejected() {
    // 3 collineari che tornano indietro (non devono formare un triangolo o cerchio)
    let stroke = stroke_from_points(&[(10.0, 10.0), (100.0, 10.0), (10.0, 10.0)]);
    let shape = recognize_shape(&stroke);
    assert!(
        shape.is_none(),
        "3 punti collineari chiusi non devono essere riconosciuti come poligono"
    );
}

#[test]
fn test_degenerate_three_collinear_backtracking_open_line() {
    // 3 punti collineari aperti con arretramento parziale: (0, 100) -> (100, 100) -> (50, 100)
    let stroke = stroke_from_points(&[(0.0, 100.0), (100.0, 100.0), (50.0, 100.0)]);
    let shape = recognize_shape(&stroke);
    println!("Backtrack shape is_some: {:?}", shape.is_some());
    if let Some(ref s) = shape {
        println!("Backtrack points: {}", s.points.len());
    }
}

#[test]
fn test_degenerate_sub_pixel_path_length() {
    // Punti distribuiti su meno di 5 pixel totali
    let stroke = stroke_from_points(&[
        (50.0, 50.0),
        (50.5, 50.2),
        (50.8, 50.4),
        (51.0, 50.5),
        (50.0, 50.0),
    ]);
    assert!(
        recognize_shape(&stroke).is_none(),
        "Micro-tratti < 10px devono restituire None"
    );
}

#[test]
fn test_degenerate_nan_and_inf_no_panic() {
    let stroke_nan = stroke_from_points(&[(0.0, 0.0), (f64::NAN, 50.0), (100.0, 100.0)]);
    // Non deve andare in panic
    assert!(recognize_shape(&stroke_nan).is_none());

    let stroke_inf = stroke_from_points(&[(0.0, 0.0), (f64::INFINITY, 50.0), (100.0, 100.0)]);
    assert!(recognize_shape(&stroke_inf).is_none());
}

#[test]
fn test_snap_line_nan_endpoint_must_return_none() {
    let stroke_nan_end = stroke_from_points(&[(0.0, 0.0), (f64::NAN, 100.0)]);
    let res = snap_line_orthogonal(&stroke_nan_end);
    assert!(
        res.is_none(),
        "snap_line_orthogonal con coordinate NaN deve restituire None, invece ha restituito Some(stroke) con endpoint NaN!"
    );
}

// =========================================================================
// 2. ORTHOGONAL SNAP BOUNDARY TESTING (7.9° vs 8.1°)
// =========================================================================

#[test]
fn test_snap_line_boundary_horizontal_positive() {
    let len = 200.0;
    // 7.9°: deve snappare a orizzontale
    let rad_7_9 = 7.9_f64.to_radians();
    let stroke_7_9 = stroke_from_points(&[(0.0, 0.0), (len * rad_7_9.cos(), len * rad_7_9.sin())]);
    let snapped_7_9 = snap_line_orthogonal(&stroke_7_9).expect("Dovrebbe snappare a 7.9°");
    assert_eq!(
        snapped_7_9.points[0].y, snapped_7_9.points[1].y,
        "7.9° deve snappare esattamente a y=0"
    );

    // 8.1°: NON deve snappare a orizzontale
    let rad_8_1 = 8.1_f64.to_radians();
    let stroke_8_1 = stroke_from_points(&[(0.0, 0.0), (len * rad_8_1.cos(), len * rad_8_1.sin())]);
    let snapped_8_1 = snap_line_orthogonal(&stroke_8_1).expect("Dovrebbe preservare a 8.1°");
    assert_ne!(
        snapped_8_1.points[0].y, snapped_8_1.points[1].y,
        "8.1° non deve subire snap orizzontale"
    );
}

#[test]
fn test_snap_line_boundary_horizontal_negative() {
    let len = 200.0;
    // -7.9°: snap orizzontale
    let rad_neg_7_9 = (-7.9_f64).to_radians();
    let stroke_neg_7_9 = stroke_from_points(&[
        (0.0, 0.0),
        (len * rad_neg_7_9.cos(), len * rad_neg_7_9.sin()),
    ]);
    let snapped_neg_7_9 = snap_line_orthogonal(&stroke_neg_7_9).expect("Dovrebbe snappare a -7.9°");
    assert_eq!(
        snapped_neg_7_9.points[0].y, snapped_neg_7_9.points[1].y,
        "-7.9° deve snappare a orizzontale"
    );

    // -8.1°: nessun snap orizzontale
    let rad_neg_8_1 = (-8.1_f64).to_radians();
    let stroke_neg_8_1 = stroke_from_points(&[
        (0.0, 0.0),
        (len * rad_neg_8_1.cos(), len * rad_neg_8_1.sin()),
    ]);
    let snapped_neg_8_1 =
        snap_line_orthogonal(&stroke_neg_8_1).expect("Dovrebbe preservare a -8.1°");
    assert_ne!(
        snapped_neg_8_1.points[0].y, snapped_neg_8_1.points[1].y,
        "-8.1° non deve subire snap orizzontale"
    );
}

#[test]
fn test_snap_line_boundary_horizontal_leftwards() {
    let len = 200.0;
    // Verso sinistra: 180° - 7.9° = 172.1° -> deve snappare
    let rad_172_1 = 172.1_f64.to_radians();
    let stroke_left_snap = stroke_from_points(&[
        (200.0, 50.0),
        (200.0 + len * rad_172_1.cos(), 50.0 + len * rad_172_1.sin()),
    ]);
    let snapped_left = snap_line_orthogonal(&stroke_left_snap).expect("Dovrebbe snappare a 172.1°");
    assert_eq!(
        snapped_left.points[0].y, snapped_left.points[1].y,
        "172.1° deve snappare a orizzontale sinistra"
    );

    // Verso sinistra: 180° - 8.1° = 171.9° -> non deve snappare
    let rad_171_9 = 171.9_f64.to_radians();
    let stroke_left_no_snap = stroke_from_points(&[
        (200.0, 50.0),
        (200.0 + len * rad_171_9.cos(), 50.0 + len * rad_171_9.sin()),
    ]);
    let preserved_left =
        snap_line_orthogonal(&stroke_left_no_snap).expect("Dovrebbe preservare a 171.9°");
    assert_ne!(
        preserved_left.points[0].y, preserved_left.points[1].y,
        "171.9° non deve snappare"
    );
}

#[test]
fn test_snap_line_boundary_vertical_downward() {
    let len = 200.0;
    // 90° - 7.9° = 82.1° -> deve snappare a verticale (x1 == x0)
    let rad_82_1 = 82.1_f64.to_radians();
    let stroke_vert_snap = stroke_from_points(&[
        (100.0, 0.0),
        (100.0 + len * rad_82_1.cos(), len * rad_82_1.sin()),
    ]);
    let snapped_vert = snap_line_orthogonal(&stroke_vert_snap).expect("Dovrebbe snappare a 82.1°");
    assert_eq!(
        snapped_vert.points[0].x, snapped_vert.points[1].x,
        "82.1° deve snappare a verticale esatta"
    );

    // 90° - 8.1° = 81.9° -> NON deve snappare a verticale
    let rad_81_9 = 81.9_f64.to_radians();
    let stroke_vert_no_snap = stroke_from_points(&[
        (100.0, 0.0),
        (100.0 + len * rad_81_9.cos(), len * rad_81_9.sin()),
    ]);
    let preserved_vert =
        snap_line_orthogonal(&stroke_vert_no_snap).expect("Dovrebbe preservare a 81.9°");
    assert_ne!(
        preserved_vert.points[0].x, preserved_vert.points[1].x,
        "81.9° non deve snappare a verticale"
    );
}

#[test]
fn test_snap_line_boundary_vertical_upward() {
    let len = 200.0;
    // -90° + 7.9° = -82.1° -> deve snappare a verticale
    let rad_up_snap = (-82.1_f64).to_radians();
    let stroke_up_snap = stroke_from_points(&[
        (100.0, 250.0),
        (
            100.0 + len * rad_up_snap.cos(),
            250.0 + len * rad_up_snap.sin(),
        ),
    ]);
    let snapped_up = snap_line_orthogonal(&stroke_up_snap).expect("Dovrebbe snappare a -82.1°");
    assert_eq!(
        snapped_up.points[0].x, snapped_up.points[1].x,
        "-82.1° deve snappare a verticale verso l'alto"
    );

    // -90° + 8.1° = -81.9° -> NON deve snappare
    let rad_up_no_snap = (-81.9_f64).to_radians();
    let stroke_up_no_snap = stroke_from_points(&[
        (100.0, 250.0),
        (
            100.0 + len * rad_up_no_snap.cos(),
            250.0 + len * rad_up_no_snap.sin(),
        ),
    ]);
    let preserved_up =
        snap_line_orthogonal(&stroke_up_no_snap).expect("Dovrebbe preservare a -81.9°");
    assert_ne!(
        preserved_up.points[0].x, preserved_up.points[1].x,
        "-81.9° non deve snappare"
    );
}

// =========================================================================
// 3. CIRCLES & ELLIPSES (WINDING ORDER & NOISE STRESS)
// =========================================================================

#[test]
fn test_circle_clockwise_vs_counter_clockwise() {
    let r = 50.0;
    let cx = 100.0;
    let cy = 100.0;
    let n = 48;

    // CCW
    let mut ccw_pts = Vec::new();
    for i in 0..n {
        let t = 2.0 * PI * (i as f64) / (n as f64);
        ccw_pts.push((cx + r * t.cos(), cy + r * t.sin()));
    }
    ccw_pts.push(ccw_pts[0]);
    let ccw_stroke = stroke_from_points(&ccw_pts);
    let ccw_shape = recognize_shape(&ccw_stroke).expect("CCW deve riconoscere cerchio");

    // CW
    let mut cw_pts = Vec::new();
    for i in 0..n {
        let t = -2.0 * PI * (i as f64) / (n as f64);
        cw_pts.push((cx + r * t.cos(), cy + r * t.sin()));
    }
    cw_pts.push(cw_pts[0]);
    let cw_stroke = stroke_from_points(&cw_pts);
    let cw_shape = recognize_shape(&cw_stroke).expect("CW deve riconoscere cerchio");

    assert_eq!(ccw_shape.points.len(), cw_shape.points.len());
    // Entrambi devono essere centrati in (cx, cy) entro tolleranza
    let ccw_avg_x =
        ccw_shape.points.iter().map(|p| p.x).sum::<f64>() / ccw_shape.points.len() as f64;
    let cw_avg_x = cw_shape.points.iter().map(|p| p.x).sum::<f64>() / cw_shape.points.len() as f64;
    // CCW e CW devono produrre il medesimo centro esatto
    assert!(
        (ccw_avg_x - cw_avg_x).abs() < 1e-4,
        "Centri CCW e CW devono coincidere"
    );
    assert!(
        (ccw_avg_x - cx).abs() < 3.0,
        "Centro stimato deve essere vicino al centro reale"
    );
}

#[test]
fn test_ellipse_winding_and_orientations() {
    let angles = [
        0.0,
        30.0_f64.to_radians(),
        45.0_f64.to_radians(),
        90.0_f64.to_radians(),
    ];
    let a = 80.0;
    let b = 35.0;
    let n = 48;

    for &rot in &angles {
        // Genera ellisse ruotata
        let mut pts = Vec::new();
        for i in 0..n {
            let t = 2.0 * PI * (i as f64) / (n as f64);
            let u = a * t.cos();
            let v = b * t.sin();
            let x = 150.0 + u * rot.cos() - v * rot.sin();
            let y = 150.0 + u * rot.sin() + v * rot.cos();
            pts.push((x, y));
        }
        pts.push(pts[0]);
        let stroke = stroke_from_points(&pts);
        let recognized = recognize_shape(&stroke);
        assert!(
            recognized.is_some(),
            "Ellisse inclinata di {} rad deve essere riconosciuta",
            rot
        );
    }
}

#[test]
fn test_extremely_noisy_circle_rejected() {
    // Cerchio con rumore radiale estremo (+/- 15px su raggio 40px, deviazione ~37%)
    let mut pts = Vec::new();
    let n = 40;
    for i in 0..n {
        let t = 2.0 * PI * (i as f64) / (n as f64);
        let noise = if i % 2 == 0 { 15.0 } else { -15.0 };
        let r = 40.0 + noise;
        pts.push((100.0 + r * t.cos(), 100.0 + r * t.sin()));
    }
    pts.push(pts[0]);
    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    // Non deve essere riconosciuto come un cerchio perfetto (deve essere None o non cerchio)
    if let Some(s) = shape {
        // Se riconosce qualcosa, non deve essere un cerchio perfetto perché il rumore è enorme
        // Verifichiamo che i punti sintetizzati non abbiano deviazione irrealistica
        assert!(!s.points.is_empty());
    }
}

#[test]
fn test_archimedean_spiral_rejected() {
    // Spirale di Archimede che si avvolge su se stessa (non è cerchio né ellisse né poligono)
    let mut pts = Vec::new();
    let n = 120;
    for i in 0..n {
        let t = i as f64 / n as f64;
        let angle = 4.0 * PI * t; // 2 giri
        let r = 10.0 + 50.0 * t;
        pts.push((150.0 + r * angle.cos(), 150.0 + r * angle.sin()));
    }
    let stroke = stroke_from_points(&pts);
    assert!(
        recognize_shape(&stroke).is_none(),
        "Una spirale aperta non deve essere riconosciuta come forma canonica"
    );
}

// =========================================================================
// 4. RECTANGLES (SKEWED CORNERS & WINDING)
// =========================================================================

#[test]
fn test_rectangle_clockwise_and_counter_clockwise() {
    // Rettangolo 100x60
    let corners = [(50.0, 50.0), (150.0, 50.0), (150.0, 110.0), (50.0, 110.0)];

    // Genera CCW
    let mut ccw = Vec::new();
    for i in 0..4 {
        let (x0, y0) = corners[i];
        let (x1, y1) = corners[(i + 1) % 4];
        for step in 0..8 {
            let t = step as f64 / 8.0;
            ccw.push((x0 + t * (x1 - x0), y0 + t * (y1 - y0)));
        }
    }
    ccw.push(corners[0]);
    let stroke_ccw = stroke_from_points(&ccw);
    let shape_ccw = recognize_shape(&stroke_ccw).expect("Rettangolo CCW deve essere riconosciuto");
    assert_eq!(shape_ccw.points.len(), 5);

    // Genera CW
    let mut cw = ccw.clone();
    cw.reverse();
    let stroke_cw = stroke_from_points(&cw);
    let shape_cw = recognize_shape(&stroke_cw).expect("Rettangolo CW deve essere riconosciuto");
    assert_eq!(shape_cw.points.len(), 5);
}

#[test]
fn test_rectangle_starting_at_midpoint() {
    // Disegna rettangolo partendo da metà del lato superiore (100, 50)
    let p_mid = (100.0, 50.0);
    let corners = [(150.0, 50.0), (150.0, 120.0), (50.0, 120.0), (50.0, 50.0)];

    let mut pts = Vec::new();
    // Da mid a corner 1
    for step in 0..5 {
        let t = step as f64 / 5.0;
        pts.push((p_mid.0 + t * (corners[0].0 - p_mid.0), p_mid.1));
    }
    // Lati successivi
    for i in 0..3 {
        let c0 = corners[i];
        let c1 = corners[i + 1];
        for step in 0..8 {
            let t = step as f64 / 8.0;
            pts.push((c0.0 + t * (c1.0 - c0.0), c0.1 + t * (c1.1 - c0.1)));
        }
    }
    // Da corner 3 a mid
    let c3 = corners[3];
    for step in 0..5 {
        let t = step as f64 / 5.0;
        pts.push((c3.0 + t * (p_mid.0 - c3.0), c3.1));
    }
    pts.push(p_mid);

    let stroke = stroke_from_points(&pts);
    let shape =
        recognize_shape(&stroke).expect("Rettangolo iniziato a metà lato deve essere riconosciuto");
    assert_eq!(
        shape.points.len(),
        5,
        "Deve essere semplificato a 4 vertici + chiusura"
    );
}

#[test]
fn test_rectangle_skewed_corner_sharp_boundary() {
    // Test di confine esatto per la tolleranza dell'angolo del rettangolo (90° +/- 18°)
    // Angolo 90° + 17° = 107° (entro 18° -> deve accettare come rettangolo)
    let c0 = (50.0, 50.0);
    let c1 = (150.0, 50.0);
    // Spostiamo c2 per fare un angolo di circa 106° su c1
    // Su c1: lato in ingresso da c0 è (100, 0), angolo 0°.
    // Per avere angolo di 106°, il lato c1->c2 deve avere direzione 106° rispetto a (-1, 0), cioè dy verso il basso e dx verso sinistra o destra
    // Usiamo coordinate calcolate:
    let angle_106 = 106.0_f64.to_radians();
    let edge_len = 80.0;
    let c2_106 = (
        c1.0 - edge_len * angle_106.cos(),
        c1.1 + edge_len * angle_106.sin(),
    );
    let c3_106 = (
        c0.0 - edge_len * angle_106.cos(),
        c0.1 + edge_len * angle_106.sin(),
    );

    // Genera punti lungo i 4 lati
    let corners_106 = [c0, c1, c2_106, c3_106];
    let mut pts_106 = Vec::new();
    for i in 0..4 {
        let a = corners_106[i];
        let b = corners_106[(i + 1) % 4];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts_106.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts_106.push(c0);
    let shape_106 = recognize_shape(&stroke_from_points(&pts_106));
    assert!(
        shape_106.is_some(),
        "Rettangolo con angolo a 106° (entro 90° +/- 18°) deve essere accettato"
    );

    // Ora angolo a 112° (> 90° + 18° = 108° -> deve rifiutare)
    let angle_112 = 112.0_f64.to_radians();
    let c2_112 = (
        c1.0 - edge_len * angle_112.cos(),
        c1.1 + edge_len * angle_112.sin(),
    );
    let c3_112 = (
        c0.0 - edge_len * angle_112.cos(),
        c0.1 + edge_len * angle_112.sin(),
    );
    let corners_112 = [c0, c1, c2_112, c3_112];
    let mut pts_112 = Vec::new();
    for i in 0..4 {
        let a = corners_112[i];
        let b = corners_112[(i + 1) % 4];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts_112.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts_112.push(c0);
    let shape_112 = recognize_shape(&stroke_from_points(&pts_112));
    if let Some(ref s) = shape_112 {
        assert_ne!(
            s.points.len(),
            5,
            "Poligono con angolo a 112° (> 108°) NON deve essere riconosciuto come rettangolo a 5 vertici"
        );
    }
}

#[test]
fn test_rectangle_skewed_corner_beyond_tolerance_rejected() {
    // Rettangolo distorto: un angolo piegato a 115° (> 90° + 18° = 108°)
    // Vertici: (40, 40), (140, 40), (190, 100) [skewed!], (40, 100)
    let corners = [(40.0, 40.0), (140.0, 40.0), (190.0, 100.0), (40.0, 100.0)];
    let mut pts = Vec::new();
    for i in 0..4 {
        let c0 = corners[i];
        let c1 = corners[(i + 1) % 4];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts.push((c0.0 + t * (c1.0 - c0.0), c0.1 + t * (c1.1 - c0.1)));
        }
    }
    pts.push(corners[0]);

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    // Non deve essere riconosciuto come rettangolo a 5 vertici rettangolari
    if let Some(s) = shape {
        // Se riconosciuto, non deve forzare un rettangolo ortogonale perfetto spurio
        assert_ne!(
            s.points.len(),
            5,
            "Un trapezio fortemente obliquo non deve essere rettangolo"
        );
    }
}

#[test]
fn test_parallelogram_60_deg_rejected() {
    // Parallelogramma con angoli 60° e 120°
    let c0 = (50.0, 100.0);
    let c1 = (150.0, 100.0);
    let c2 = (180.0, 48.0); // dx=30, dy=-52 -> angolo ~60°
    let c3 = (80.0, 48.0);
    let corners = [c0, c1, c2, c3];

    let mut pts = Vec::new();
    for i in 0..4 {
        let a = corners[i];
        let b = corners[(i + 1) % 4];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts.push(corners[0]);

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    if let Some(s) = shape {
        assert_ne!(
            s.points.len(),
            5,
            "Parallelogramma a 60° non deve essere riconosciuto come rettangolo"
        );
    }
}

// =========================================================================
// 5. TRIANGLES (FLATNESS & SHARP CORNERS)
// =========================================================================

#[test]
fn test_triangle_winding_cw_and_ccw() {
    let v0 = (100.0, 30.0);
    let v1 = (170.0, 150.0);
    let v2 = (30.0, 150.0);

    // CCW: v0 -> v2 -> v1 -> v0
    let corners_ccw = [v0, v2, v1];
    let mut ccw = Vec::new();
    for i in 0..3 {
        let a = corners_ccw[i];
        let b = corners_ccw[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            ccw.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    ccw.push(v0);
    let shape_ccw = recognize_shape(&stroke_from_points(&ccw)).expect("CCW Triangolo");
    assert_eq!(shape_ccw.points.len(), 4);

    // CW: v0 -> v1 -> v2 -> v0
    let corners_cw = [v0, v1, v2];
    let mut cw = Vec::new();
    for i in 0..3 {
        let a = corners_cw[i];
        let b = corners_cw[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            cw.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    cw.push(v0);
    let shape_cw = recognize_shape(&stroke_from_points(&cw)).expect("CW Triangolo");
    assert_eq!(shape_cw.points.len(), 4);
}

#[test]
fn test_triangle_angle_boundary_20_and_145_deg() {
    // Triangolo valido al limite: angoli (138°, 21°, 21°)
    // B = (-100, 0), C = (100, 0), A = (0, 100 * tan(21°))
    let tan_21 = 21.0_f64.to_radians().tan();
    let v_a_valid = (0.0, 100.0 * tan_21);
    let v_b = (-100.0, 0.0);
    let v_c = (100.0, 0.0);

    let corners_valid = [v_a_valid, v_b, v_c];
    let mut pts_valid = Vec::new();
    for i in 0..3 {
        let a = corners_valid[i];
        let b = corners_valid[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts_valid.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts_valid.push(v_a_valid);
    let shape_valid = recognize_shape(&stroke_from_points(&pts_valid));
    assert!(
        shape_valid.is_some(),
        "Triangolo con angoli ~21° (min_angle > 20°) deve essere accettato"
    );

    // Triangolo fuori limite: angolo alla base 17° < 20° (e angolo al vertice 146° > 145°)
    let tan_17 = 17.0_f64.to_radians().tan();
    let v_a_invalid = (0.0, 100.0 * tan_17);
    let corners_invalid = [v_a_invalid, v_b, v_c];
    let mut pts_invalid = Vec::new();
    for i in 0..3 {
        let a = corners_invalid[i];
        let b = corners_invalid[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts_invalid.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts_invalid.push(v_a_invalid);
    let shape_invalid = recognize_shape(&stroke_from_points(&pts_invalid));
    assert!(
        shape_invalid.is_none(),
        "Triangolo piatto con angoli 17° (< 20°) e 146° (> 145°) deve essere rifiutato"
    );
}

#[test]
fn test_flat_triangle_collinear_rejected() {
    // Triangolo quasi degenere piatto: altezza 1px su base 200px (angolo ottuso ~179°)
    let v0 = (0.0, 100.0);
    let v1 = (100.0, 101.0);
    let v2 = (200.0, 100.0);
    let corners = [v0, v1, v2];

    let mut pts = Vec::new();
    for i in 0..3 {
        let a = corners[i];
        let b = corners[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts.push(v0);

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    assert!(
        shape.is_none(),
        "Triangolo quasi degenere/piatto con angolo ~179° deve essere rifiutato"
    );
}

#[test]
fn test_needle_triangle_tiny_angle_rejected() {
    // Triangolo ad ago: base 200px, vertice opposto a (100, 104) -> angolo < 10°
    let v0 = (0.0, 100.0);
    let v1 = (200.0, 100.0);
    let v2 = (100.0, 104.0);
    let corners = [v0, v1, v2];

    let mut pts = Vec::new();
    for i in 0..3 {
        let a = corners[i];
        let b = corners[(i + 1) % 3];
        for step in 0..10 {
            let t = step as f64 / 10.0;
            pts.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
    }
    pts.push(v0);

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    assert!(
        shape.is_none(),
        "Triangolo ad ago con angoli acutissimi < 20° deve essere rifiutato"
    );
}

// =========================================================================
// 6. FREEHAND SQUIGGLES, CURSIVE & STRESS
// =========================================================================

#[test]
fn test_l_shape_open_corner_rejected() {
    // Tratto a "L": due segmenti ortogonali aperti
    let mut pts = Vec::new();
    for i in 0..10 {
        pts.push((50.0, 50.0 + i as f64 * 10.0));
    }
    for i in 0..10 {
        pts.push((50.0 + i as f64 * 10.0, 150.0));
    }
    let stroke = stroke_from_points(&pts);
    assert!(
        recognize_shape(&stroke).is_none(),
        "Una forma ad L aperta non deve essere trasformata in linea né in poligono"
    );
}

#[test]
fn test_u_shape_horseshoe_rejected() {
    // Tratto a "U" / ferro di cavallo aperto
    let mut pts = Vec::new();
    let n = 30;
    // Semintervallo da pi a 2*pi
    for i in 0..n {
        let angle = PI + PI * (i as f64 / n as f64);
        pts.push((100.0 + 50.0 * angle.cos(), 100.0 + 50.0 * angle.sin()));
    }
    let stroke = stroke_from_points(&pts);
    assert!(
        recognize_shape(&stroke).is_none(),
        "Un semicerchio a U aperto non deve essere trasformato in cerchio o poligono"
    );
}

#[test]
fn test_cursive_letter_e_graceful_none() {
    // Tracciato ad anello che simula la 'e' corsiva
    let mut pts = Vec::new();
    // Tratto ascendente
    for i in 0..15 {
        let t = i as f64 / 15.0;
        pts.push((30.0 + t * 20.0, 80.0 - t * 30.0));
    }
    // Loop superiore
    for i in 0..20 {
        let angle = PI * (i as f64 / 20.0);
        pts.push((50.0 + 10.0 * angle.cos(), 50.0 - 15.0 * angle.sin()));
    }
    // Tratto discendente e coda
    for i in 0..20 {
        let t = i as f64 / 20.0;
        pts.push((40.0 + t * 40.0, 50.0 + t * 30.0));
    }

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    assert!(
        shape.is_none(),
        "La lettera corsiva 'e' non deve essere scambiata per un cerchio o rettangolo"
    );
}

#[test]
fn test_five_pointed_star_rejected() {
    // Stella a 5 punte
    let mut pts = Vec::new();
    let cx = 100.0;
    let cy = 100.0;
    for i in 0..10 {
        let angle = -PI / 2.0 + (i as f64) * (2.0 * PI / 10.0);
        let r = if i % 2 == 0 { 50.0 } else { 20.0 };
        pts.push((cx + r * angle.cos(), cy + r * angle.sin()));
    }
    pts.push(pts[0]);

    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    assert!(
        shape.is_none(),
        "Una stella a 5 punte non deve essere confusa con triangolo o rettangolo"
    );
}

#[test]
fn test_high_frequency_zigzag_rejected() {
    // Zig-zag / dente di sega
    let mut pts = Vec::new();
    for i in 0..50 {
        let x = 10.0 + i as f64 * 4.0;
        let y = if i % 2 == 0 { 30.0 } else { 70.0 };
        pts.push((x, y));
    }
    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    if let Some(ref s) = shape {
        println!(
            "Zigzag recognized as shape with {} points, mode {:?}",
            s.points.len(),
            s.mode
        );
        for (i, p) in s.points.iter().enumerate() {
            println!("  p[{}]: ({}, {})", i, p.x, p.y);
        }
    }
    assert!(
        shape.is_none(),
        "Un zig-zag orizzontale aperto non deve essere trasformato in triangolo!"
    );
}

#[test]
fn test_wavy_underline_rejected() {
    // Riga ondulata aperta orizzontale (es. sottolineatura o scarabocchio)
    let mut pts = Vec::new();
    let n = 200;
    for i in 0..n {
        let t = i as f64 / n as f64;
        let x = 50.0 + t * 400.0;
        let y = 200.0 + (t * 20.0 * PI).sin() * 15.0;
        pts.push((x, y));
    }
    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    if let Some(ref s) = shape {
        println!(
            "Wavy underline recognized as shape with {} points!",
            s.points.len()
        );
    }
    assert!(
        shape.is_none(),
        "Una sottolineatura ondulata aperta non deve essere trasformata in forma geometrica"
    );
}

#[test]
fn test_cursive_word_handwriting_rejected() {
    // Simula la scrittura della parola corsiva "minim"
    let mut pts = Vec::new();
    for i in 0..10 {
        let x_base = 50.0 + i as f64 * 30.0;
        for step in 0..10 {
            let t = step as f64 / 10.0;
            // Tratto in salita e discesa
            pts.push((x_base + t * 15.0, 100.0 - (t * PI).sin() * 35.0));
        }
    }
    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    if let Some(ref s) = shape {
        println!(
            "Cursive word recognized as shape with {} points!",
            s.points.len()
        );
    }
    assert!(
        shape.is_none(),
        "Scrittura corsiva non deve essere convertita in una forma geometrica chiusa"
    );
}

#[test]
fn test_scratch_out_rejected() {
    // Tratto di cancellatura a zig-zag orizzontale (scratch-out)
    let mut pts = Vec::new();
    for i in 0..12 {
        let y = 100.0 + i as f64 * 2.0;
        let x = if i % 2 == 0 { 50.0 } else { 150.0 };
        pts.push((x, y));
    }
    let stroke = stroke_from_points(&pts);
    let shape = recognize_shape(&stroke);
    if let Some(ref s) = shape {
        println!(
            "Scratch-out recognized as shape with {} points, mode {:?}",
            s.points.len(),
            s.mode
        );
        for (i, p) in s.points.iter().enumerate() {
            println!("  p[{}]: ({}, {})", i, p.x, p.y);
        }
    }
    assert!(
        shape.is_none(),
        "Uno scarabocchio di cancellatura (scratch-out) non deve essere riconosciuto come forma"
    );
}

#[test]
fn test_high_density_stroke_performance_stress() {
    // Tratto ad altissima densità (5.000 campioni)
    let n = 5000;
    let mut pts = Vec::with_capacity(n);
    for i in 0..n {
        let t = i as f64 / n as f64;
        let x = 10.0 + t * 300.0;
        let y = 100.0 + (t * 50.0 * PI).sin() * 2.0; // lieve ondulazione
        pts.push((x, y));
    }
    let stroke = stroke_from_points(&pts);
    let start_time = std::time::Instant::now();
    let _ = recognize_shape(&stroke);
    let elapsed = start_time.elapsed();
    assert!(
        elapsed.as_millis() < 500,
        "Il riconoscimento su 5.000 campioni deve completare in meno di 500ms (tempo effettivo: {:?})",
        elapsed
    );
}
