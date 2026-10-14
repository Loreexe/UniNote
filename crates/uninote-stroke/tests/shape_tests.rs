use std::f64::consts::PI;
use uninote_stroke::shape::{recognize_shape, snap_line_orthogonal};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn create_stroke_from_coords(coords: &[(f64, f64)]) -> Stroke {
    let mut stroke = Stroke::new(1, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    stroke.points = coords
        .iter()
        .map(|&(x, y)| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 0,
        })
        .collect();
    stroke
}

#[test]
fn test_snap_line_orthogonal_horizontal() {
    // Linea quasi orizzontale (inclinazione ~1.1° <= 8°)
    let stroke = create_stroke_from_coords(&[(10.0, 100.0), (100.0, 101.0), (200.0, 104.0)]);
    let snapped = snap_line_orthogonal(&stroke).expect("Dovrebbe rettificare la linea");

    assert_eq!(snapped.points.len(), 2);
    assert_eq!(snapped.mode, PipelineMode::Baseline);
    assert_eq!(snapped.points[0].x, 10.0);
    assert_eq!(snapped.points[0].y, 100.0);
    assert_eq!(snapped.points[1].x, 200.0);
    // y dell'estremo finale deve essere snappata a 100.0 esatti
    assert_eq!(
        snapped.points[1].y, 100.0,
        "La linea deve essere perfettamente orizzontale"
    );
}

#[test]
fn test_snap_line_orthogonal_vertical() {
    // Linea quasi verticale (inclinazione ~89° <= 8° da 90°)
    let stroke = create_stroke_from_coords(&[(150.0, 20.0), (151.0, 150.0), (153.0, 300.0)]);
    let snapped = snap_line_orthogonal(&stroke).expect("Dovrebbe rettificare la linea");

    assert_eq!(snapped.points.len(), 2);
    assert_eq!(snapped.mode, PipelineMode::Baseline);
    assert_eq!(snapped.points[0].x, 150.0);
    assert_eq!(snapped.points[0].y, 20.0);
    // x dell'estremo finale deve essere snappata a 150.0 esatti
    assert_eq!(
        snapped.points[1].x, 150.0,
        "La linea deve essere perfettamente verticale"
    );
    assert_eq!(snapped.points[1].y, 300.0);
}

#[test]
fn test_snap_line_diagonal_preserves_endpoints() {
    // Linea diagonale a 45°: non deve subire snap orizzontale o verticale
    let stroke = create_stroke_from_coords(&[(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)]);
    let snapped = snap_line_orthogonal(&stroke).expect("Dovrebbe produrre linea retta a 2 punti");

    assert_eq!(snapped.points.len(), 2);
    assert_eq!(snapped.mode, PipelineMode::Baseline);
    assert_eq!(snapped.points[0].x, 0.0);
    assert_eq!(snapped.points[0].y, 0.0);
    assert_eq!(snapped.points[1].x, 100.0);
    assert_eq!(snapped.points[1].y, 100.0);
}

#[test]
fn test_recognize_shape_straight_line() {
    // Tratto aperto teso con piccolo jitter manuale
    let mut coords = Vec::new();
    for i in 0..=20 {
        let t = i as f64 / 20.0;
        let x = 10.0 + t * 150.0;
        let y = 50.0 + (i % 3) as f64 * 0.5; // jitter <= 1.0px
        coords.push((x, y));
    }
    let stroke = create_stroke_from_coords(&coords);
    let shape = recognize_shape(&stroke).expect("Dovrebbe riconoscere una linea retta");

    assert_eq!(shape.points.len(), 2);
    assert_eq!(shape.mode, PipelineMode::Baseline);
    assert_eq!(shape.points[0].y, shape.points[1].y);
}

#[test]
fn test_recognize_shape_circle() {
    // Cerchio di raggio 40.0 centrato in (100, 100) con lieve rumore
    let mut coords = Vec::new();
    let num_pts = 40;
    for i in 0..num_pts {
        let angle = 2.0 * PI * (i as f64) / (num_pts as f64);
        let jitter = if i % 2 == 0 { 0.5 } else { -0.5 };
        let r = 40.0 + jitter;
        coords.push((100.0 + r * angle.cos(), 100.0 + r * angle.sin()));
    }
    // Chiudi il cerchio
    coords.push(coords[0]);

    let stroke = create_stroke_from_coords(&coords);
    let shape = recognize_shape(&stroke).expect("Dovrebbe riconoscere un cerchio");

    assert_eq!(shape.mode, PipelineMode::Baseline);
    assert!(
        shape.points.len() >= 30,
        "Il cerchio deve avere loop liscio"
    );
    // Verifica chiusura
    let first = &shape.points[0];
    let last = shape.points.last().unwrap();
    assert!((first.x - last.x).abs() < 1e-4 && (first.y - last.y).abs() < 1e-4);

    // Verifica che i punti siano equidistanti dal centro (~100, 100)
    for p in &shape.points {
        let dist = ((p.x - 100.0).powi(2) + (p.y - 100.0).powi(2)).sqrt();
        assert!(
            (dist - 40.0).abs() < 3.0,
            "Punto fuori tolleranza raggio: {}",
            dist
        );
    }
}

#[test]
fn test_recognize_shape_ellipse() {
    // Ellisse con a=70.0, b=30.0 (aspect ratio ~2.33)
    let mut coords = Vec::new();
    let num_pts = 40;
    for i in 0..num_pts {
        let angle = 2.0 * PI * (i as f64) / (num_pts as f64);
        coords.push((150.0 + 70.0 * angle.cos(), 150.0 + 30.0 * angle.sin()));
    }
    coords.push(coords[0]);

    let stroke = create_stroke_from_coords(&coords);
    let shape = recognize_shape(&stroke).expect("Dovrebbe riconoscere un'ellisse");

    assert_eq!(shape.mode, PipelineMode::Baseline);
    let first = &shape.points[0];
    let last = shape.points.last().unwrap();
    assert!((first.x - last.x).abs() < 1e-4 && (first.y - last.y).abs() < 1e-4);
}

#[test]
fn test_recognize_shape_rectangle() {
    // Rettangolo asse-allineato con vertici (40, 40) a (160, 110)
    let mut coords = Vec::new();
    // Lato superiore: da (40, 40) a (160, 40)
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((40.0 + t * 120.0, 40.0 + (i % 2) as f64 * 0.4));
    }
    // Lato destro: da (160, 40) a (160, 110)
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((160.0 + (i % 2) as f64 * 0.4, 40.0 + t * 70.0));
    }
    // Lato inferiore: da (160, 110) a (40, 110)
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((160.0 - t * 120.0, 110.0 + (i % 2) as f64 * 0.4));
    }
    // Lato sinistro: da (40, 110) a (40, 40)
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((40.0 + (i % 2) as f64 * 0.4, 110.0 - t * 70.0));
    }
    coords.push((40.0, 40.0));

    let stroke = create_stroke_from_coords(&coords);
    let shape = recognize_shape(&stroke).expect("Dovrebbe riconoscere un rettangolo");

    assert_eq!(shape.mode, PipelineMode::Baseline);
    // Deve avere 5 punti (4 angoli + chiusura)
    assert_eq!(shape.points.len(), 5);
    // Primo e ultimo punto coincidenti
    assert_eq!(shape.points[0].x, shape.points[4].x);
    assert_eq!(shape.points[0].y, shape.points[4].y);
}

#[test]
fn test_recognize_shape_triangle() {
    // Triangolo con vertici in (100, 30), (170, 150), (30, 150)
    let mut coords = Vec::new();
    let v0 = (100.0, 30.0);
    let v1 = (170.0, 150.0);
    let v2 = (30.0, 150.0);

    // Lato v0 -> v1
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((v0.0 + t * (v1.0 - v0.0), v0.1 + t * (v1.1 - v0.1)));
    }
    // Lato v1 -> v2
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((v1.0 + t * (v2.0 - v1.0), v1.1 + t * (v2.1 - v1.1)));
    }
    // Lato v2 -> v0
    for i in 0..10 {
        let t = i as f64 / 10.0;
        coords.push((v2.0 + t * (v0.0 - v2.0), v2.1 + t * (v0.1 - v2.1)));
    }
    coords.push(v0);

    let stroke = create_stroke_from_coords(&coords);
    let shape = recognize_shape(&stroke).expect("Dovrebbe riconoscere un triangolo");

    assert_eq!(shape.mode, PipelineMode::Baseline);
    assert_eq!(shape.points.len(), 4, "3 vertici + chiusura");
    assert_eq!(shape.points[0].x, shape.points[3].x);
    assert_eq!(shape.points[0].y, shape.points[3].y);
}

#[test]
fn test_recognize_shape_fallback_freehand() {
    // Tratto a forma di 'S' / sinusoide aperta: non deve essere convertito in forma
    let mut coords = Vec::new();
    for i in 0..40 {
        let t = i as f64 / 40.0;
        coords.push((10.0 + t * 100.0, 50.0 + 30.0 * (t * 4.0 * PI).sin()));
    }
    let stroke = create_stroke_from_coords(&coords);
    assert!(
        recognize_shape(&stroke).is_none(),
        "Una curva complessa aperta non deve corrispondere ad alcuna forma"
    );
}

#[test]
fn test_recognize_shape_degenerate() {
    let empty_stroke = create_stroke_from_coords(&[]);
    assert!(recognize_shape(&empty_stroke).is_none());

    let single_pt = create_stroke_from_coords(&[(50.0, 50.0)]);
    assert!(recognize_shape(&single_pt).is_none());
}
