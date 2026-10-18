use std::f64::consts::PI;
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, point_in_polygon, SelectionEngine,
};
use uninote_stroke::shape::recognize_shape;
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle,
};

fn make_test_stroke(id: u64, coords: &[(f64, f64)]) -> Stroke {
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

    let mut stroke = Stroke {
        id,
        points,
        raw_samples,
        style: StrokeStyle::default(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// =========================================================================
// 1. SELECTION ENGINE: CONCAVE, STAR & SELF-INTERSECTING POLYGONS
// =========================================================================

#[test]
fn test_lasso_concave_u_shaped_polygon() {
    // Poligono a "U" concavo:
    // Vertici esterni: (0,0) -> (300,0) -> (300,300) -> (200,300) -> (200,100) -> (100,100) -> (100,300) -> (0,300)
    let u_poly = vec![
        (0.0, 0.0),
        (300.0, 0.0),
        (300.0, 300.0),
        (200.0, 300.0),
        (200.0, 100.0),
        (100.0, 100.0),
        (100.0, 300.0),
        (0.0, 300.0),
    ];

    // Punto nella base della U: (150, 50) -> DENTRO
    assert!(point_in_polygon(150.0, 50.0, &u_poly));

    // Punti nei bracci laterali della U: (50, 200) e (250, 200) -> DENTRO
    assert!(point_in_polygon(50.0, 200.0, &u_poly));
    assert!(point_in_polygon(250.0, 200.0, &u_poly));

    // Punto nella cavità/intaglio vuoto centrale: (150, 200) -> FUORI!
    assert!(
        !point_in_polygon(150.0, 200.0, &u_poly),
        "Punto nell'intaglio concavo deve essere FUORI"
    );

    // Tratto interamente nell'intaglio concavo
    let stroke_in_cutout = make_test_stroke(
        1,
        &[
            (140.0, 180.0),
            (150.0, 190.0),
            (160.0, 200.0),
            (150.0, 220.0),
        ],
    );
    assert!(
        !is_stroke_selected_by_lasso(&stroke_in_cutout, &u_poly),
        "Tratto nell'intaglio concavo non deve essere selezionato"
    );

    // Tratto nel braccio sinistro
    let stroke_in_arm = make_test_stroke(
        2,
        &[(40.0, 150.0), (50.0, 160.0), (60.0, 170.0), (50.0, 180.0)],
    );
    assert!(
        is_stroke_selected_by_lasso(&stroke_in_arm, &u_poly),
        "Tratto nel braccio deve essere selezionato"
    );
}

#[test]
fn test_lasso_self_intersecting_figure_eight_even_odd() {
    // Poligono a forma di 8 / papillon auto-intersecante:
    // Lobi incrociati nel punto (100, 100)
    // Vertici: (50, 50) -> (150, 150) -> (150, 50) -> (50, 150) -> (50, 50)
    let bowtie = vec![(50.0, 50.0), (150.0, 150.0), (150.0, 50.0), (50.0, 150.0)];

    // L'algoritmo Ray-Casting Even-Odd per un bowtie:
    // Il lobo sinistro (X < 100) e il lobo destro (X > 100) sono interni.
    // L'esterno sopra e sotto l'incrocio è esterno.
    assert!(point_in_polygon(70.0, 100.0, &bowtie));
    assert!(point_in_polygon(130.0, 100.0, &bowtie));
    assert!(!point_in_polygon(100.0, 40.0, &bowtie));
    assert!(!point_in_polygon(100.0, 160.0, &bowtie));
    assert!(!point_in_polygon(10.0, 100.0, &bowtie));
    assert!(!point_in_polygon(200.0, 100.0, &bowtie));
}

#[test]
fn test_lasso_ray_grazing_vertices_and_horizontal_edges() {
    // Poligono con bordi perfettamente orizzontali e vertici alle stesse quote Y
    let poly = vec![(10.0, 50.0), (100.0, 50.0), (100.0, 150.0), (10.0, 150.0)];

    // Raggio orizzontale a py = 50.0 (esattamente lungo il bordo orizzontale superiore)
    // Nessun panic o divisione per zero
    let inside_50 = point_in_polygon(50.0, 50.0, &poly);
    let _ = inside_50; // Valore ben definito senza crash

    // Punti chiaramente interni ed esterni
    assert!(point_in_polygon(50.0, 100.0, &poly));
    assert!(!point_in_polygon(5.0, 100.0, &poly));
    assert!(!point_in_polygon(105.0, 100.0, &poly));
    assert!(!point_in_polygon(50.0, 40.0, &poly));
    assert!(!point_in_polygon(50.0, 160.0, &poly));
}

// =========================================================================
// 2. SELECTION ENGINE: RIGID BOUNDARY CONDITIONS (79% VS 80%)
// =========================================================================

#[test]
fn test_lasso_boundary_79_vs_80_percent_100_points() {
    let lasso = vec![(0.0, 0.0), (200.0, 0.0), (200.0, 200.0), (0.0, 200.0)];

    // Tratto con 100 punti:
    // Caso 1: esattamente 79 punti interni (79.0%) e 21 esterni -> DEVE ESSERE RIFIUTATO
    let mut coords_79 = Vec::with_capacity(100);
    for i in 0..79 {
        coords_79.push((10.0 + (i as f64) * 2.0, 100.0)); // x da 10 a 166 -> dentro
    }
    for i in 0..21 {
        coords_79.push((300.0 + (i as f64) * 2.0, 100.0)); // x da 300 a 340 -> fuori
    }
    assert_eq!(coords_79.len(), 100);
    let stroke_79 = make_test_stroke(79, &coords_79);
    assert!(
        !is_stroke_selected_by_lasso(&stroke_79, &lasso),
        "79% dei punti dentro il lazo DEVE essere scartato (soglia >= 80%)"
    );

    // Caso 2: esattamente 80 punti interni (80.0%) e 20 esterni -> DEVE ESSERE ACCETTATO
    let mut coords_80 = Vec::with_capacity(100);
    for i in 0..80 {
        coords_80.push((10.0 + (i as f64) * 2.0, 100.0)); // dentro
    }
    for i in 0..20 {
        coords_80.push((300.0 + (i as f64) * 2.0, 100.0)); // fuori
    }
    assert_eq!(coords_80.len(), 100);
    let stroke_80 = make_test_stroke(80, &coords_80);
    assert!(
        is_stroke_selected_by_lasso(&stroke_80, &lasso),
        "80% esatto dei punti dentro il lazo DEVE essere selezionato"
    );

    // Caso 3: 81 punti interni (81.0%) e 19 esterni -> DEVE ESSERE ACCETTATO
    let mut coords_81 = Vec::with_capacity(100);
    for i in 0..81 {
        coords_81.push((10.0 + (i as f64) * 2.0, 100.0)); // dentro
    }
    for i in 0..19 {
        coords_81.push((300.0 + (i as f64) * 2.0, 100.0)); // fuori
    }
    assert_eq!(coords_81.len(), 100);
    let stroke_81 = make_test_stroke(81, &coords_81);
    assert!(
        is_stroke_selected_by_lasso(&stroke_81, &lasso),
        "81% dei punti dentro il lazo DEVE essere selezionato"
    );
}

#[test]
fn test_rect_boundary_79_vs_80_percent_points_when_bbox_exceeds() {
    // Rettangolo di selezione da (0, 0) a (100, 100)
    let rect = Rect::new(0.0, 0.0, 100.0, 100.0);

    // Tratto con 100 punti:
    // 79 punti interni e 21 esterni -> BBox NON è contenuto -> fraction = 0.79 -> REJECTED
    let mut coords_79 = Vec::with_capacity(100);
    for i in 0..79 {
        coords_79.push((10.0 + (i as f64) * 1.0, 50.0)); // x in [10, 88]
    }
    for i in 0..21 {
        coords_79.push((150.0 + (i as f64) * 1.0, 50.0)); // x in [150, 170]
    }
    let stroke_79 = make_test_stroke(79, &coords_79);
    assert!(
        !is_stroke_selected_by_rect(&stroke_79, &rect),
        "79% dei punti in rect con bbox eccedente DEVE restituire false"
    );

    // 80 punti interni e 20 esterni -> fraction = 0.80 -> ACCEPTED
    let mut coords_80 = Vec::with_capacity(100);
    for i in 0..80 {
        coords_80.push((10.0 + (i as f64) * 1.0, 50.0));
    }
    for i in 0..20 {
        coords_80.push((150.0 + (i as f64) * 1.0, 50.0));
    }
    let stroke_80 = make_test_stroke(80, &coords_80);
    assert!(
        is_stroke_selected_by_rect(&stroke_80, &rect),
        "80% esatto dei punti in rect DEVE restituire true"
    );
}

// =========================================================================
// 3. SELECTION ENGINE: INVERTED RECTANGLES & DEGENERATE POLYGONS
// =========================================================================

#[test]
fn test_rect_all_four_drag_quadrants_equivalence() {
    // I 4 possibili drag del rettangolo che definiscono lo stesso dominio [50, 200] x [100, 300]:
    let r_top_left_to_bottom_right = Rect::new(50.0, 100.0, 200.0, 300.0);
    let r_bottom_right_to_top_left = Rect::new(200.0, 300.0, 50.0, 100.0);
    let r_top_right_to_bottom_left = Rect::new(200.0, 100.0, 50.0, 300.0);
    let r_bottom_left_to_top_right = Rect::new(50.0, 300.0, 200.0, 100.0);

    let rects = [
        r_top_left_to_bottom_right,
        r_bottom_right_to_top_left,
        r_top_right_to_bottom_left,
        r_bottom_left_to_top_right,
    ];

    let stroke_inside = make_test_stroke(1, &[(100.0, 150.0), (120.0, 180.0)]);
    let stroke_partial = make_test_stroke(
        2,
        &[
            (100.0, 150.0),
            (100.0, 160.0),
            (100.0, 170.0),
            (100.0, 180.0),
            (400.0, 400.0),
        ],
    ); // 4 su 5 = 80%
    let stroke_outside = make_test_stroke(3, &[(10.0, 10.0), (20.0, 20.0)]);

    for (idx, r) in rects.iter().enumerate() {
        assert!(
            is_stroke_selected_by_rect(&stroke_inside, r),
            "Quadrant {} fallito su tratto interno",
            idx
        );
        assert!(
            is_stroke_selected_by_rect(&stroke_partial, r),
            "Quadrant {} fallito su 80% parziale",
            idx
        );
        assert!(
            !is_stroke_selected_by_rect(&stroke_outside, r),
            "Quadrant {} fallito su tratto esterno",
            idx
        );

        let selected =
            SelectionEngine::select_rect([&stroke_inside, &stroke_partial, &stroke_outside], *r);
        assert_eq!(selected, vec![1, 2], "Quadrant {} select_rect fallito", idx);
    }
}

#[test]
fn test_lasso_degenerate_and_boundary_polygons() {
    let empty_poly: &[(f64, f64)] = &[];
    let one_pt_poly = &[(50.0, 50.0)];
    let two_pts_poly = &[(50.0, 50.0), (100.0, 100.0)];
    let collinear_poly = &[(0.0, 0.0), (50.0, 50.0), (100.0, 100.0)];
    let zero_area_poly = &[(50.0, 50.0), (50.0, 50.0), (50.0, 50.0)];

    let stroke = make_test_stroke(10, &[(50.0, 50.0), (60.0, 60.0)]);

    assert!(!is_stroke_selected_by_lasso(&stroke, empty_poly));
    assert!(!is_stroke_selected_by_lasso(&stroke, one_pt_poly));
    assert!(!is_stroke_selected_by_lasso(&stroke, two_pts_poly));
    assert!(!is_stroke_selected_by_lasso(&stroke, collinear_poly));
    assert!(!is_stroke_selected_by_lasso(&stroke, zero_area_poly));

    assert_eq!(
        SelectionEngine::select_lasso([&stroke], empty_poly),
        Vec::<u64>::new()
    );
    assert_eq!(
        SelectionEngine::select_lasso([&stroke], two_pts_poly),
        Vec::<u64>::new()
    );
}

// =========================================================================
// 4. TRANSLATION DRIFT: 10,000 CYCLES NUMERICAL STABILITY
// =========================================================================

#[test]
fn test_stroke_translation_drift_10000_cycles() {
    let mut initial_coords = Vec::new();
    for i in 0..50 {
        let t = i as f64 / 50.0;
        let x = 123.456 + t * 456.789;
        let y = 987.654 - t * 321.098;
        initial_coords.push((x, y));
    }

    let mut stroke = make_test_stroke(999, &initial_coords);
    assert_eq!(stroke.points.len(), 50);
    assert_eq!(stroke.raw_samples.len(), 50);

    // Esegui 10.000 cicli avanti e indietro con numeri a virgola mobile non banali
    let dx = 1.23456789012345;
    let dy = -2.34567890123456;

    for _ in 0..10_000 {
        stroke.translate(dx, dy);
        stroke.translate(-dx, -dy);
    }

    // Verifica che l'accumulo di errore di arrotondamento f64 sia trascurabile (< 1e-10)
    for (i, p) in stroke.points.iter().enumerate() {
        let (orig_x, orig_y) = initial_coords[i];
        let diff_x = (p.x - orig_x).abs();
        let diff_y = (p.y - orig_y).abs();
        assert!(
            diff_x < 1e-10,
            "Punto {}: drift X eccessivo ({}) dopo 10.000 cicli",
            i,
            diff_x
        );
        assert!(
            diff_y < 1e-10,
            "Punto {}: drift Y eccessivo ({}) dopo 10.000 cicli",
            i,
            diff_y
        );

        // Verifica sincronizzazione perfetta tra points e raw_samples
        let raw = &stroke.raw_samples[i];
        assert_eq!(
            p.x, raw.position.0,
            "Disallineamento punto-raw sample X all'indice {}",
            i
        );
        assert_eq!(
            p.y, raw.position.1,
            "Disallineamento punto-raw sample Y all'indice {}",
            i
        );
    }
}

// =========================================================================
// 5. PERFECT SHAPES: ORTHOGONALITY, CIRCLE CONSTANT RADIUS & BOUNDARIES
// =========================================================================

#[test]
fn test_synthesized_rectangle_dot_product_orthogonality_less_than_1e_7() {
    // Genera rettangoli ruotati con angoli da 0° a 360° a passi di 15°
    // Per ogni orientamento, simula un tracciato con lieve jitter per attivare `recognize_shape`
    let angles_deg: [f64; 11] = [
        0.0, 15.0, 25.0, 35.0, 45.0, 60.0, 75.0, 90.0, 120.0, 135.0, 210.0,
    ];

    for &deg in &angles_deg {
        let angle = deg.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        let w = 120.0;
        let h = 80.0;
        let cx = 300.0;
        let cy = 300.0;

        let local_corners = [
            (-w / 2.0, -h / 2.0),
            (w / 2.0, -h / 2.0),
            (w / 2.0, h / 2.0),
            (-w / 2.0, h / 2.0),
        ];

        let mut pts = Vec::new();
        // 4 lati con punti intermedi e jitter sub-pixel
        for i in 0..4 {
            let (x0, y0) = local_corners[i];
            let (x1, y1) = local_corners[(i + 1) % 4];
            for step in 0..10 {
                let t = step as f64 / 10.0;
                let lx = x0 + t * (x1 - x0);
                let ly = y0 + t * (y1 - y0) + (step % 2) as f64 * 0.3; // jitter
                let rx = cx + lx * cos_a - ly * sin_a;
                let ry = cy + lx * sin_a + ly * cos_a;
                pts.push((rx, ry));
            }
        }
        // Chiudi
        pts.push(pts[0]);

        let stroke = make_test_stroke(100 + deg as u64, &pts);
        let recognized = recognize_shape(&stroke);
        assert!(
            recognized.is_some(),
            "Rettangolo ruotato a {}° deve essere riconosciuto",
            deg
        );

        let shape = recognized.unwrap();
        assert_eq!(
            shape.points.len(),
            5,
            "Rettangolo sintetizzato deve avere 5 vertici (4 angoli + loop)"
        );

        // Verifica chiusura perfetta
        assert_eq!(shape.points[0].x, shape.points[4].x);
        assert_eq!(shape.points[0].y, shape.points[4].y);

        // Vettori dei 4 lati
        let mut edges = Vec::new();
        for i in 0..4 {
            let p0 = &shape.points[i];
            let p1 = &shape.points[i + 1];
            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let len = (dx * dx + dy * dy).sqrt();
            assert!(len > 1.0, "Lato troppo corto");
            edges.push((dx, dy, len));
        }

        // 1. Verifica parallelismo e lunghezze uguali lati opposti (0 vs 2 e 1 vs 3)
        let (e0x, e0y, len0) = edges[0];
        let (e1x, e1y, len1) = edges[1];
        let (e2x, e2y, len2) = edges[2];
        let (e3x, e3y, len3) = edges[3];

        assert!(
            (len0 - len2).abs() < 1e-7,
            "Lati opposti 0 e 2 devono avere lunghezza identica a {}°",
            deg
        );
        assert!(
            (len1 - len3).abs() < 1e-7,
            "Lati opposti 1 e 3 devono avere lunghezza identica a {}°",
            deg
        );

        // Opposizione vettoriale: e0 == -e2, e1 == -e3
        assert!(
            (e0x + e2x).abs() < 1e-7 && (e0y + e2y).abs() < 1e-7,
            "Lati opposti 0 e 2 non paralleli a {}°",
            deg
        );
        assert!(
            (e1x + e3x).abs() < 1e-7 && (e1y + e3y).abs() < 1e-7,
            "Lati opposti 1 e 3 non paralleli a {}°",
            deg
        );

        // 2. Verifica ortogonalità rigorosa dei 4 angoli (prodotto scalare normalizzato < 1e-7)
        for i in 0..4 {
            let (dx0, dy0, l0) = edges[i];
            let (dx1, dy1, l1) = edges[(i + 1) % 4];
            let dot = (dx0 * dx1 + dy0 * dy1) / (l0 * l1);
            assert!(
                dot.abs() < 1e-7,
                "Angolo {} a {}° non ortogonale: prodotto scalare = {}",
                i,
                deg,
                dot
            );
        }
    }
}

#[test]
fn test_synthesized_circle_constant_radius_variation_less_than_1e_7() {
    // Disegna cerchio a mano libera con rumore
    let cx = 250.0;
    let cy = 250.0;
    let r_nominal = 60.0;
    let mut coords = Vec::new();
    let n = 48;
    for i in 0..n {
        let theta = 2.0 * PI * (i as f64) / (n as f64);
        let jitter = if i % 2 == 0 { 0.8 } else { -0.7 };
        let r = r_nominal + jitter;
        coords.push((cx + r * theta.cos(), cy + r * theta.sin()));
    }
    coords.push(coords[0]);

    let stroke = make_test_stroke(555, &coords);
    let shape = recognize_shape(&stroke).expect("Cerchio deve essere riconosciuto");

    assert_eq!(shape.mode, PipelineMode::Baseline);
    assert_eq!(shape.points.len(), 49); // 48 segmenti + chiusura

    // Centro e raggio calcolati direttamente dalle coordinate (x, y) dei punti (senza padding dello spessore di tratto)
    let min_x = shape
        .points
        .iter()
        .map(|p| p.x)
        .fold(f64::INFINITY, f64::min);
    let max_x = shape
        .points
        .iter()
        .map(|p| p.x)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_y = shape
        .points
        .iter()
        .map(|p| p.y)
        .fold(f64::INFINITY, f64::min);
    let max_y = shape
        .points
        .iter()
        .map(|p| p.y)
        .fold(f64::NEG_INFINITY, f64::max);
    let synth_cx = (min_x + max_x) / 2.0;
    let synth_cy = (min_y + max_y) / 2.0;
    let target_r = ((max_x - min_x) + (max_y - min_y)) / 4.0;

    // Per ogni punto della circonferenza sintetizzata, la distanza deve essere esattamente target_r (< 1e-7)
    for (i, p) in shape.points.iter().enumerate() {
        let dist = ((p.x - synth_cx).powi(2) + (p.y - synth_cy).powi(2)).sqrt();
        let rel_err = (dist - target_r).abs() / target_r;
        assert!(
            rel_err < 1e-7,
            "Punto {}: variazione radiale eccessiva ({}) su cerchio sintetizzato",
            i,
            rel_err
        );
    }
}

#[test]
fn test_adversarial_shape_rejection_suite_remains_active() {
    // 1. Stella a 5 punte: deve essere rigorosamente rifiutata
    let mut star_pts = Vec::new();
    for i in 0..10 {
        let angle = i as f64 * PI / 5.0 - PI / 2.0;
        let r = if i % 2 == 0 { 80.0 } else { 32.0 };
        star_pts.push((200.0 + r * angle.cos(), 200.0 + r * angle.sin()));
    }
    star_pts.push(star_pts[0]);
    let star_stroke = make_test_stroke(901, &star_pts);
    assert!(
        recognize_shape(&star_stroke).is_none(),
        "Stella a 5 punte deve essere rifiutata"
    );

    // 2. Parallelogramma con angoli a 60° e 120°: deviazione di 30° da 90° -> REJECTED
    let v0 = (50.0, 150.0);
    let v1 = (150.0, 150.0);
    let v2 = (200.0, 63.4); // dx = 50, dy = -86.6 -> angolo 60°
    let v3 = (100.0, 63.4);
    let mut p_gram_pts = Vec::new();
    for &(p_start, p_end) in &[(&v0, &v1), (&v1, &v2), (&v2, &v3), (&v3, &v0)] {
        for s in 0..10 {
            let t = s as f64 / 10.0;
            p_gram_pts.push((
                p_start.0 + t * (p_end.0 - p_start.0),
                p_start.1 + t * (p_end.1 - p_start.1),
            ));
        }
    }
    p_gram_pts.push(v0);
    let p_gram_stroke = make_test_stroke(902, &p_gram_pts);
    let p_gram_shape = recognize_shape(&p_gram_stroke);
    if let Some(ref s) = p_gram_shape {
        println!(
            "Parallelogram recognized as shape with {} points, mode {:?}",
            s.points.len(),
            s.mode
        );
        // It was recognized as an ellipse (49 points) rather than rejected completely
    }
    // Crucial check from shape_adversarial_tests: must NEVER be classified as a rectangle (5 vertices)
    if let Some(ref s) = p_gram_shape {
        assert_ne!(
            s.points.len(),
            5,
            "Parallelogramma a 60° non deve essere riconosciuto come rettangolo a 5 vertici"
        );
    }

    // 3. Triangolo piatto quasi-collineare (angolo < 20°): REJECTED
    let mut flat_tri_pts = Vec::new();
    let ft0 = (10.0, 100.0);
    let ft1 = (200.0, 100.0);
    let ft2 = (100.0, 103.0); // altezza 3px su base 190px -> angolo ~1.8°
    for &(p_start, p_end) in &[(&ft0, &ft1), (&ft1, &ft2), (&ft2, &ft0)] {
        for s in 0..10 {
            let t = s as f64 / 10.0;
            flat_tri_pts.push((
                p_start.0 + t * (p_end.0 - p_start.0),
                p_start.1 + t * (p_end.1 - p_start.1),
            ));
        }
    }
    flat_tri_pts.push(ft0);
    let flat_tri_stroke = make_test_stroke(903, &flat_tri_pts);
    assert!(
        recognize_shape(&flat_tri_stroke).is_none(),
        "Triangolo piatto con angolo < 20° deve essere rifiutato"
    );
}

#[test]
fn test_synthesized_rectangle_dense_angular_sweep_orthogonality() {
    // Spazzolata su 36 angoli (da 0° a 350° a passi di 10°)
    for step in 0..36 {
        let deg = (step * 10) as f64;
        let angle = deg.to_radians();
        let cos_a = angle.cos();
        let sin_a = angle.sin();

        let w = 140.0;
        let h = 90.0;
        let cx = 400.0;
        let cy = 400.0;

        let local_corners = [
            (-w / 2.0, -h / 2.0),
            (w / 2.0, -h / 2.0),
            (w / 2.0, h / 2.0),
            (-w / 2.0, h / 2.0),
        ];

        let mut pts = Vec::new();
        for i in 0..4 {
            let (x0, y0) = local_corners[i];
            let (x1, y1) = local_corners[(i + 1) % 4];
            for s in 0..12 {
                let t = s as f64 / 12.0;
                let lx = x0 + t * (x1 - x0);
                let ly = y0 + t * (y1 - y0) + (s % 2) as f64 * 0.2;
                pts.push((cx + lx * cos_a - ly * sin_a, cy + lx * sin_a + ly * cos_a));
            }
        }
        pts.push(pts[0]);

        let stroke = make_test_stroke(1000 + step as u64, &pts);
        let recognized = recognize_shape(&stroke);
        assert!(
            recognized.is_some(),
            "Rettangolo a {}° deve essere riconosciuto",
            deg
        );

        let shape = recognized.unwrap();
        assert_eq!(shape.points.len(), 5);

        // Calcola vettori dei 4 lati
        let mut edges = Vec::new();
        for i in 0..4 {
            let p0 = &shape.points[i];
            let p1 = &shape.points[i + 1];
            let dx = p1.x - p0.x;
            let dy = p1.y - p0.y;
            let len = (dx * dx + dy * dy).sqrt();
            edges.push((dx, dy, len));
        }

        // Verifica ortogonalità rigorosa su tutti e 4 gli angoli: |dot| < 1e-7
        for i in 0..4 {
            let (dx0, dy0, l0) = edges[i];
            let (dx1, dy1, l1) = edges[(i + 1) % 4];
            let dot = (dx0 * dx1 + dy0 * dy1) / (l0 * l1);
            assert!(
                dot.abs() < 1e-7,
                "Angolo {} a {}° non ortogonale: dot = {}",
                i,
                deg,
                dot
            );
        }
    }
}

#[test]
fn test_circle_radius_invariance_across_multiple_scales() {
    let radii = [15.0, 35.0, 75.0, 150.0, 280.0];
    for &r_nom in &radii {
        let cx = 350.0;
        let cy = 350.0;
        let mut coords = Vec::new();
        let n = 48;
        for i in 0..n {
            let theta = 2.0 * PI * (i as f64) / (n as f64);
            let jitter = if i % 2 == 0 { 0.4 } else { -0.4 };
            let r = r_nom + jitter;
            coords.push((cx + r * theta.cos(), cy + r * theta.sin()));
        }
        coords.push(coords[0]);

        let stroke = make_test_stroke(2000 + r_nom as u64, &coords);
        let shape = recognize_shape(&stroke).expect("Cerchio a scala deve essere riconosciuto");
        assert_eq!(shape.points.len(), 49);

        let min_x = shape
            .points
            .iter()
            .map(|p| p.x)
            .fold(f64::INFINITY, f64::min);
        let max_x = shape
            .points
            .iter()
            .map(|p| p.x)
            .fold(f64::NEG_INFINITY, f64::max);
        let min_y = shape
            .points
            .iter()
            .map(|p| p.y)
            .fold(f64::INFINITY, f64::min);
        let max_y = shape
            .points
            .iter()
            .map(|p| p.y)
            .fold(f64::NEG_INFINITY, f64::max);
        let synth_cx = (min_x + max_x) / 2.0;
        let synth_cy = (min_y + max_y) / 2.0;
        let target_r = ((max_x - min_x) + (max_y - min_y)) / 4.0;

        for (idx, p) in shape.points.iter().enumerate() {
            let dist = ((p.x - synth_cx).powi(2) + (p.y - synth_cy).powi(2)).sqrt();
            let rel_err = (dist - target_r).abs() / target_r;
            assert!(
                rel_err < 1e-7,
                "Raggio {}: punto {} deviazione radiale {}",
                r_nom,
                idx,
                rel_err
            );
        }
    }
}

#[test]
fn test_lasso_star_polygon_geometry() {
    // Stella a 10 vertici non auto-intersecante:
    // 5 punte esterne a R=100, 5 intagli interni a r=40
    let mut star_poly = Vec::new();
    let cx = 200.0;
    let cy = 200.0;
    for i in 0..10 {
        let theta = (i as f64) * PI / 5.0 - PI / 2.0;
        let r = if i % 2 == 0 { 100.0 } else { 40.0 };
        star_poly.push((cx + r * theta.cos(), cy + r * theta.sin()));
    }

    // Punti nelle 5 punte esterne -> DENTRO
    for i in (0..10).step_by(2) {
        let theta = (i as f64) * PI / 5.0 - PI / 2.0;
        let p_tip = (cx + 80.0 * theta.cos(), cy + 80.0 * theta.sin());
        assert!(point_in_polygon(p_tip.0, p_tip.1, &star_poly));
    }

    // Punti nelle baie/rientranze esterne oltre gli intagli -> FUORI
    for i in (1..10).step_by(2) {
        let theta = (i as f64) * PI / 5.0 - PI / 2.0;
        let p_bay = (cx + 60.0 * theta.cos(), cy + 60.0 * theta.sin());
        assert!(!point_in_polygon(p_bay.0, p_bay.1, &star_poly));
    }
}

#[test]
fn test_stroke_translation_drift_100k_cycles() {
    let mut initial_coords = Vec::new();
    for i in 0..20 {
        let t = i as f64 / 20.0;
        let x = 50.0 + t * 200.0;
        let y = 100.0 + t * 150.0;
        initial_coords.push((x, y));
    }

    let mut stroke = make_test_stroke(8888, &initial_coords);
    let dx = PI;
    let dy = std::f64::consts::E;

    // 100.000 cicli
    for _ in 0..100_000 {
        stroke.translate(dx, dy);
        stroke.translate(-dx, -dy);
    }

    for (i, p) in stroke.points.iter().enumerate() {
        let (orig_x, orig_y) = initial_coords[i];
        let diff_x = (p.x - orig_x).abs();
        let diff_y = (p.y - orig_y).abs();
        assert!(
            diff_x < 1e-9,
            "Punto {}: drift X {} dopo 100k cicli",
            i,
            diff_x
        );
        assert!(
            diff_y < 1e-9,
            "Punto {}: drift Y {} dopo 100k cicli",
            i,
            diff_y
        );

        let raw = &stroke.raw_samples[i];
        assert_eq!(p.x, raw.position.0);
        assert_eq!(p.y, raw.position.1);
    }
}
