use crate::types::{PipelineMode, Stroke, StrokePoint};
use std::f64::consts::PI;

/// Punto 2D per calcoli geometrici interni.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Point2D {
    x: f64,
    y: f64,
}

impl Point2D {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    fn dist(self, other: Point2D) -> f64 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt()
    }
}

/// Calcola la distanza perpendicolare da un punto a un segmento di retta [a, b].
fn dist_to_segment(p: Point2D, a: Point2D, b: Point2D) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len_sq = dx * dx + dy * dy;
    if len_sq < 1e-8 {
        return p.dist(a);
    }
    let t = (((p.x - a.x) * dx + (p.y - a.y) * dy) / len_sq).clamp(0.0, 1.0);
    let proj = Point2D::new(a.x + t * dx, a.y + t * dy);
    p.dist(proj)
}

/// Calcola la distanza perpendicolare da un punto alla retta infinita passante per a e b.
fn dist_to_line(p: Point2D, a: Point2D, b: Point2D) -> f64 {
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-8 {
        return p.dist(a);
    }
    ((dy * p.x - dx * p.y + b.x * a.y - b.y * a.x).abs()) / len
}

/// Calcola l'angolo interno in radianti (intervallo [0, PI]) tra i segmenti (p_prev -> p_curr) e (p_next -> p_curr).
fn corner_angle(p_prev: Point2D, p_curr: Point2D, p_next: Point2D) -> f64 {
    let v1x = p_prev.x - p_curr.x;
    let v1y = p_prev.y - p_curr.y;
    let v2x = p_next.x - p_curr.x;
    let v2y = p_next.y - p_curr.y;
    let len1 = (v1x * v1x + v1y * v1y).sqrt();
    let len2 = (v2x * v2x + v2y * v2y).sqrt();
    if len1 < 1e-8 || len2 < 1e-8 {
        return 0.0;
    }
    let dot = (v1x * v2x + v1y * v2y) / (len1 * len2);
    dot.clamp(-1.0, 1.0).acos()
}

/// Semplificazione Ramer-Douglas-Peucker ricorsiva per una catena aperta di punti.
fn rdp_chain(points: &[Point2D], epsilon: f64, out: &mut Vec<Point2D>) {
    if points.len() < 2 {
        return;
    }
    if points.len() == 2 {
        out.push(points[0]);
        return;
    }

    let start = points[0];
    let end = *points.last().unwrap();
    let mut max_dist = 0.0;
    let mut max_idx = 0;

    for (i, p) in points.iter().enumerate().skip(1).take(points.len() - 2) {
        let d = dist_to_segment(*p, start, end);
        if d > max_dist {
            max_dist = d;
            max_idx = i;
        }
    }

    if max_dist > epsilon {
        rdp_chain(&points[..=max_idx], epsilon, out);
        rdp_chain(&points[max_idx..], epsilon, out);
    } else {
        out.push(start);
    }
}

/// Semplifica un tracciato chiuso dividendolo in due catene rispetto al punto più distante.
fn rdp_closed(points: &[Point2D], epsilon: f64) -> Vec<Point2D> {
    if points.len() < 4 {
        return points.to_vec();
    }

    let p0 = points[0];
    // Trova l'antipodo (punto a distanza massima dall'inizio)
    let mut far_idx = 1;
    let mut max_d = 0.0;
    for (i, p) in points.iter().enumerate().skip(1) {
        let d = p0.dist(*p);
        if d > max_d {
            max_d = d;
            far_idx = i;
        }
    }

    if far_idx < 2 || far_idx >= points.len() - 2 {
        far_idx = points.len() / 2;
    }

    let mut chain1 = Vec::new();
    rdp_chain(&points[..=far_idx], epsilon, &mut chain1);

    let mut chain2_input = points[far_idx..].to_vec();
    chain2_input.push(points[0]);
    let mut chain2 = Vec::new();
    rdp_chain(&chain2_input, epsilon, &mut chain2);

    let mut result = chain1;
    result.extend(chain2);

    // Rimuovi vertici duplicati o consecutivi troppo ravvicinati
    clean_vertices(&result, epsilon * 0.8)
}

/// Pulisce i vertici rimuovendo quelli adiacenti troppo vicini o collineari.
fn clean_vertices(verts: &[Point2D], min_dist: f64) -> Vec<Point2D> {
    if verts.len() <= 3 {
        return verts.to_vec();
    }

    let mut out: Vec<Point2D> = Vec::new();
    for &v in verts {
        if let Some(&last) = out.last() {
            if last.dist(v) < min_dist {
                continue;
            }
        }
        out.push(v);
    }

    if out.len() > 2 {
        if let (Some(&first), Some(&last)) = (out.first(), out.last()) {
            if first.dist(last) < min_dist {
                out.pop();
            }
        }
    }

    // Rimuovi vertici collineari intermedi (angolo ~180°)
    let mut i = 0;
    while out.len() > 3 && i < out.len() {
        let n = out.len();
        let prev = out[(i + n - 1) % n];
        let curr = out[i];
        let next = out[(i + 1) % n];
        let angle = corner_angle(prev, curr, next);
        // Se l'angolo interno è vicino a 180° (deviazione < 18°), il punto non è un angolo netto
        if (PI - angle).abs() < 18.0_f64.to_radians() {
            out.remove(i);
        } else {
            i += 1;
        }
    }

    out
}

/// Calcola la distanza media dei campioni del tratto dai lati del poligono definito dai vertici.
fn mean_dist_to_polygon(pts: &[Point2D], verts: &[Point2D]) -> f64 {
    if pts.is_empty() || verts.len() < 2 {
        return 0.0;
    }
    let n = verts.len();
    let mut total_d = 0.0;
    for &p in pts {
        let mut min_edge_d = f64::MAX;
        for i in 0..n {
            let a = verts[i];
            let b = verts[(i + 1) % n];
            let d = dist_to_segment(p, a, b);
            if d < min_edge_d {
                min_edge_d = d;
            }
        }
        total_d += min_edge_d;
    }
    total_d / pts.len() as f64
}

/// Rettifica un tratto aperto in una linea retta a due punti, applicando lo snap ortogonale
/// (orizzontale se entro 8° da 0° o 180°, verticale se entro 8° da 90° o 270°).
pub fn snap_line_orthogonal(stroke: &Stroke) -> Option<Stroke> {
    if stroke.points.len() < 2 {
        return None;
    }

    if stroke.points.iter().any(|p| {
        !p.x.is_finite() || !p.y.is_finite() || !p.width.is_finite() || !p.pressure.is_finite()
    }) {
        return None;
    }

    let p0 = stroke.points.first()?;
    let p_last = stroke.points.last()?;
    let dx = p_last.x - p0.x;
    let dy = p_last.y - p0.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if !dist.is_finite() || dist < 1.0 {
        return None;
    }

    let theta = dy.atan2(dx);
    let delta = 8.0_f64.to_radians(); // ~0.1396 rad (8°)

    // Orizzontale: angolo vicino a 0, PI o -PI
    let is_horizontal = theta.abs() <= delta || (PI - theta.abs()).abs() <= delta;
    // Verticale: angolo vicino a PI/2 o -PI/2
    let is_vertical = (theta.abs() - std::f64::consts::FRAC_PI_2).abs() <= delta;

    let mut start_pt = p0.clone();
    let mut end_pt = p_last.clone();

    if is_horizontal {
        end_pt.y = start_pt.y;
    } else if is_vertical {
        end_pt.x = start_pt.x;
    }

    let avg_width = stroke.points.iter().map(|p| p.width).sum::<f64>() / stroke.points.len() as f64;
    let avg_pressure =
        stroke.points.iter().map(|p| p.pressure).sum::<f64>() / stroke.points.len() as f64;

    start_pt.width = avg_width;
    start_pt.pressure = avg_pressure;
    end_pt.width = avg_width;
    end_pt.pressure = avg_pressure;

    let mut new_stroke = Stroke::new(stroke.id, stroke.style.clone(), PipelineMode::Baseline);
    new_stroke.is_dot = false;
    new_stroke.points = vec![start_pt, end_pt];
    Some(new_stroke)
}

/// Analizza un tratto completato e, se soddisfa le tolleranze geometriche, lo trasforma
/// in una forma geometrica canonica (linea con snap ortogonale, cerchio/ellisse, rettangolo o triangolo).
/// Restituisce `None` se il tratto è a mano libera e non corrisponde ad alcuna forma.
pub fn recognize_shape(stroke: &Stroke) -> Option<Stroke> {
    if stroke.points.len() < 2 {
        return None;
    }

    if stroke.points.iter().any(|p| {
        !p.x.is_finite() || !p.y.is_finite() || !p.width.is_finite() || !p.pressure.is_finite()
    }) {
        return None;
    }

    let pts: Vec<Point2D> = stroke
        .points
        .iter()
        .map(|p| Point2D::new(p.x, p.y))
        .collect();

    let p0 = *pts.first()?;
    let p_last = *pts.last()?;

    let mut path_len = 0.0;
    for w in pts.windows(2) {
        path_len += w[0].dist(w[1]);
    }
    if path_len < 10.0 {
        return None;
    }

    let end_dist = p0.dist(p_last);
    let closure_ratio = end_dist / path_len;

    let mut min_x = f64::MAX;
    let mut max_x = f64::MIN;
    let mut min_y = f64::MAX;
    let mut max_y = f64::MIN;
    for p in &pts {
        min_x = min_x.min(p.x);
        max_x = max_x.max(p.x);
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }
    let w = max_x - min_x;
    let h = max_y - min_y;
    let diag = (w * w + h * h).sqrt();

    let avg_width = stroke.points.iter().map(|p| p.width).sum::<f64>() / stroke.points.len() as f64;
    let avg_pressure =
        stroke.points.iter().map(|p| p.pressure).sum::<f64>() / stroke.points.len() as f64;

    // 1. Candidato Linea Retta (tratto aperto e ben teso)
    if closure_ratio >= 0.60 && end_dist >= 10.0 {
        let mut max_line_dev = 0.0;
        for &p in &pts {
            let d = dist_to_line(p, p0, p_last);
            if d > max_line_dev {
                max_line_dev = d;
            }
        }
        let allowed_dev = (0.14 * end_dist).max(12.0);
        if max_line_dev <= allowed_dev {
            return snap_line_orthogonal(stroke);
        }
    }

    // Se il tracciato non è chiuso, non può essere un poligono o ellisse
    let is_closed = end_dist <= 45.0 || (end_dist <= 0.32 * diag && closure_ratio <= 0.40);
    if !is_closed {
        return None;
    }

    // 2. Prova riconoscimento Cerchio o Ellisse (superfici a curvatura continua)
    let cx = pts.iter().map(|p| p.x).sum::<f64>() / pts.len() as f64;
    let cy = pts.iter().map(|p| p.y).sum::<f64>() / pts.len() as f64;

    let radii: Vec<f64> = pts
        .iter()
        .map(|p| ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt())
        .collect();
    let mean_r = radii.iter().sum::<f64>() / radii.len() as f64;
    if mean_r < 4.0 {
        return None;
    }

    let variance_r = radii.iter().map(|r| (r - mean_r).powi(2)).sum::<f64>() / radii.len() as f64;
    let std_dev_r = variance_r.sqrt();
    let cv_r = std_dev_r / mean_r;
    let aspect_ratio = w.max(h) / w.min(h).max(1.0);

    let max_r_dev = radii
        .iter()
        .map(|r| (r - mean_r).abs() / mean_r)
        .fold(0.0_f64, f64::max);

    // Cerchio: CV <= 0.16, aspect ratio <= 1.28 e deviazione radiale massima <= 0.20
    // (un quadrato ha max deviazione > 0.23 sui vertici)
    if cv_r <= 0.16 && aspect_ratio <= 1.28 && max_r_dev <= 0.20 {
        let radius = (w + h) / 4.0;
        let circle_points = synthesize_ellipse(cx, cy, radius, radius, 0.0, 48);
        let synthesized =
            create_stroke_from_points(stroke, &circle_points, avg_width, avg_pressure);
        return Some(synthesized);
    }

    // Ellisse generale: aspect ratio <= 4.2, CV moderato o fit su assi ruotati
    if aspect_ratio <= 4.2 && cv_r <= 0.35 {
        // Matrice di covarianza per determinare orientamento ed assi
        let n = pts.len() as f64;
        let cov_xx = pts.iter().map(|p| (p.x - cx).powi(2)).sum::<f64>() / n;
        let cov_yy = pts.iter().map(|p| (p.y - cy).powi(2)).sum::<f64>() / n;
        let cov_xy = pts.iter().map(|p| (p.x - cx) * (p.y - cy)).sum::<f64>() / n;

        let mut angle = 0.5 * (2.0 * cov_xy).atan2(cov_xx - cov_yy);
        let snap_tol = 12.0_f64.to_radians();
        if angle.abs() <= snap_tol || (PI - angle.abs()).abs() <= snap_tol {
            angle = 0.0;
        } else if (angle.abs() - std::f64::consts::FRAC_PI_2).abs() <= snap_tol {
            angle = std::f64::consts::FRAC_PI_2;
        }

        let cos_a = angle.cos();
        let sin_a = angle.sin();

        // Coordinate ruotate
        let mut max_u = 0.0_f64;
        let mut max_v = 0.0_f64;
        let mut sum_u2 = 0.0_f64;
        let mut sum_v2 = 0.0_f64;

        for p in &pts {
            let dx = p.x - cx;
            let dy = p.y - cy;
            let u = dx * cos_a + dy * sin_a;
            let v = -dx * sin_a + dy * cos_a;
            max_u = max_u.max(u.abs());
            max_v = max_v.max(v.abs());
            sum_u2 += u * u;
            sum_v2 += v * v;
        }

        let semi_a = (max_u + (2.0 * sum_u2 / n).sqrt()) / 2.0;
        let semi_b = (max_v + (2.0 * sum_v2 / n).sqrt()) / 2.0;

        if semi_a > 4.0 && semi_b > 4.0 {
            // Verifica errore radiale relativo
            let mut total_rel_err = 0.0;
            let mut max_rel_err = 0.0_f64;
            for p in &pts {
                let dx = p.x - cx;
                let dy = p.y - cy;
                let u = dx * cos_a + dy * sin_a;
                let v = -dx * sin_a + dy * cos_a;
                let rho = ((u / semi_a).powi(2) + (v / semi_b).powi(2)).sqrt();
                let err = (rho - 1.0).abs();
                total_rel_err += err;
                max_rel_err = max_rel_err.max(err);
            }
            let mean_rel_err = total_rel_err / n;

            // Su un rettangolo, max_rel_err supera 0.40 sui vertici
            if mean_rel_err <= 0.16 && max_rel_err <= 0.26 {
                let ellipse_points = synthesize_ellipse(cx, cy, semi_a, semi_b, angle, 48);
                let synthesized =
                    create_stroke_from_points(stroke, &ellipse_points, avg_width, avg_pressure);
                return Some(synthesized);
            }
        }
    }

    // 3. Prova riconoscimento Poligoni con RDP (Rettangolo e Triangolo)
    let epsilons = [0.030 * diag, 0.045 * diag, 0.060 * diag];

    for &eps in &epsilons {
        let verts = rdp_closed(&pts, eps);

        // --- RETTANGOLO: 4 vertici ---
        if verts.len() == 4 {
            let mut right_angles_ok = true;
            let target_angle = std::f64::consts::FRAC_PI_2; // 90°
            let angle_tol = 18.0_f64.to_radians(); // +/- 18°

            for i in 0..4 {
                let prev = verts[(i + 3) % 4];
                let curr = verts[i];
                let next = verts[(i + 1) % 4];
                let angle = corner_angle(prev, curr, next);
                if (angle - target_angle).abs() > angle_tol {
                    right_angles_ok = false;
                    break;
                }
            }

            if right_angles_ok {
                // Verifica aderenza: i punti del tratto devono cadere vicini ai bordi
                let mean_dev = mean_dist_to_polygon(&pts, &verts);
                let allowed_dev = (0.15 * w.min(h)).max(12.0);
                if mean_dev <= allowed_dev {
                    // Snap opzionale agli assi ortogonali se entro 12°
                    let edge0_dx = verts[1].x - verts[0].x;
                    let edge0_dy = verts[1].y - verts[0].y;
                    let edge_angle = edge0_dy.atan2(edge0_dx);
                    let snap_tol = 12.0_f64.to_radians();

                    let is_axis_aligned = edge_angle.abs() <= snap_tol
                        || (PI - edge_angle.abs()).abs() <= snap_tol
                        || (edge_angle.abs() - std::f64::consts::FRAC_PI_2).abs() <= snap_tol;

                    let rect_points = synthesize_perfect_rectangle(
                        &verts,
                        min_x,
                        min_y,
                        max_x,
                        max_y,
                        is_axis_aligned,
                    );

                    let synthesized =
                        create_stroke_from_points(stroke, &rect_points, avg_width, avg_pressure);
                    return Some(synthesized);
                }
            }
        }

        // --- TRIANGOLO: 3 vertici ---
        if verts.len() == 3 {
            let mut sharp_corners_ok = true;
            let min_angle = 20.0_f64.to_radians(); // > 20°
            let max_angle = 145.0_f64.to_radians();

            for i in 0..3 {
                let prev = verts[(i + 2) % 3];
                let curr = verts[i];
                let next = verts[(i + 1) % 3];
                let angle = corner_angle(prev, curr, next);
                if angle < min_angle || angle > max_angle {
                    sharp_corners_ok = false;
                    break;
                }
            }

            // Verifica non-collinearità (area > 5% del bounding box)
            let area = 0.5
                * ((verts[0].x * (verts[1].y - verts[2].y)
                    + verts[1].x * (verts[2].y - verts[0].y)
                    + verts[2].x * (verts[0].y - verts[1].y))
                    .abs());
            let bb_area = w * h;

            if sharp_corners_ok && area >= 0.05 * bb_area && bb_area > 20.0 {
                let mean_dev = mean_dist_to_polygon(&pts, &verts);
                let allowed_dev = (0.16 * w.min(h)).max(14.0);
                if mean_dev <= allowed_dev {
                    let tri_points = vec![verts[0], verts[1], verts[2], verts[0]];
                    let synthesized =
                        create_stroke_from_points(stroke, &tri_points, avg_width, avg_pressure);
                    return Some(synthesized);
                }
            }
        }
    }

    // Fallback: nessuna forma riconosciuta entro le tolleranze
    None
}

/// Genera punti parametrici per un cerchio o un'ellisse (eventualmente inclinata).
fn synthesize_ellipse(
    cx: f64,
    cy: f64,
    semi_a: f64,
    semi_b: f64,
    angle: f64,
    num_segments: usize,
) -> Vec<Point2D> {
    let mut points = Vec::with_capacity(num_segments + 1);
    let cos_a = angle.cos();
    let sin_a = angle.sin();

    for i in 0..=num_segments {
        let t = 2.0 * PI * (i as f64) / (num_segments as f64);
        let u = semi_a * t.cos();
        let v = semi_b * t.sin();
        let x = cx + u * cos_a - v * sin_a;
        let y = cy + u * sin_a + v * cos_a;
        points.push(Point2D::new(x, y));
    }
    points
}

/// Costruisce uno `Stroke` con `PipelineMode::Baseline` a partire da una sequenza di `Point2D`.
fn create_stroke_from_points(
    src_stroke: &Stroke,
    pts: &[Point2D],
    width: f64,
    pressure: f64,
) -> Stroke {
    let mut new_stroke = Stroke::new(
        src_stroke.id,
        src_stroke.style.clone(),
        PipelineMode::Baseline,
    );
    new_stroke.is_dot = false;
    new_stroke.points = pts
        .iter()
        .map(|p| StrokePoint {
            x: p.x,
            y: p.y,
            pressure,
            width,
            timestamp_ns: 0,
        })
        .collect();
    new_stroke
}

/// Sintetizza un rettangolo matematicamente perfetto al 100% (4 angoli a 90° esatti, lati opposti paralleli e identici).
/// Se quasi allineato agli assi, snappa all'AABB ortogonale; se ruotato, sintetizza un OBB perfetto.
fn synthesize_perfect_rectangle(
    verts: &[Point2D],
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
    is_axis_aligned: bool,
) -> Vec<Point2D> {
    if is_axis_aligned || verts.len() < 4 {
        return vec![
            Point2D::new(min_x, min_y),
            Point2D::new(max_x, min_y),
            Point2D::new(max_x, max_y),
            Point2D::new(min_x, max_y),
            Point2D::new(min_x, min_y),
        ];
    }

    // Centroide dei 4 vertici
    let cx = (verts[0].x + verts[1].x + verts[2].x + verts[3].x) / 4.0;
    let cy = (verts[0].y + verts[1].y + verts[2].y + verts[3].y) / 4.0;
    let center = Point2D::new(cx, cy);

    // Vettore per il primo asse (lati opposti 0->1 e 3->2)
    let u1x = (verts[1].x - verts[0].x) + (verts[2].x - verts[3].x);
    let u1y = (verts[1].y - verts[0].y) + (verts[2].y - verts[3].y);
    let len_u = (u1x * u1x + u1y * u1y).sqrt();

    // Vettore per il secondo asse (lati opposti 1->2 e 0->3)
    let v1x = (verts[2].x - verts[1].x) + (verts[3].x - verts[0].x);
    let v1y = (verts[2].y - verts[1].y) + (verts[3].y - verts[0].y);
    let len_v = (v1x * v1x + v1y * v1y).sqrt();

    if len_u < 1e-4 || len_v < 1e-4 {
        return vec![
            Point2D::new(min_x, min_y),
            Point2D::new(max_x, min_y),
            Point2D::new(max_x, max_y),
            Point2D::new(min_x, max_y),
            Point2D::new(min_x, min_y),
        ];
    }

    // Direzione unitaria per u
    let angle_u = u1y.atan2(u1x);
    let ux = angle_u.cos();
    let uy = angle_u.sin();

    // Vettore unitario ortogonale per v (ruotato di 90° esatti: (-uy, ux))
    let mut vx = -uy;
    let mut vy = ux;

    // Preserva il verso relativo a v1
    if v1x * vx + v1y * vy < 0.0 {
        vx = -vx;
        vy = -vy;
    }

    let half_w = len_u / 4.0;
    let half_h = len_v / 4.0;

    let p0 = Point2D::new(
        center.x - half_w * ux - half_h * vx,
        center.y - half_w * uy - half_h * vy,
    );
    let p1 = Point2D::new(
        center.x + half_w * ux - half_h * vx,
        center.y + half_w * uy - half_h * vy,
    );
    let p2 = Point2D::new(
        center.x + half_w * ux + half_h * vx,
        center.y + half_w * uy + half_h * vy,
    );
    let p3 = Point2D::new(
        center.x - half_w * ux + half_h * vx,
        center.y - half_w * uy + half_h * vy,
    );

    vec![p0, p1, p2, p3, p0]
}
