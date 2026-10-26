//! Modulo per il taglio geometrico analitico dei tratti vettoriali tramite gomma circolare.
//!
//! Implementa l'algoritmo esatto di intersezione tra cerchio e spline di Bézier cubiche,
//! la suddivisione tramite de Casteljau e la ricostruzione fedele dei sotto-tratti risultanti
//! con interpolazione di spessori, pressioni, timestamp e cache geometrica a zero allocazioni.

use crate::geometry::CubicBezierSegment;
use crate::types::{DeviceKind, InputPhase, InputSample, Stroke, StrokePoint};

/// Calcola la derivata vettoriale B'(t) di un segmento di Bézier cubico per t in [0.0, 1.0].
///
/// B'(t) = 3(1-t)^2 (cp1 - p0) + 6(1-t)t (cp2 - cp1) + 3t^2 (p1 - cp2)
#[inline]
pub fn bezier_derivative(seg: &CubicBezierSegment, t: f64) -> (f64, f64) {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;
    let c0 = 3.0 * u * u;
    let c1 = 6.0 * u * t;
    let c2 = 3.0 * t * t;

    let dx =
        c0 * (seg.cp1.0 - seg.p0.0) + c1 * (seg.cp2.0 - seg.cp1.0) + c2 * (seg.p1.0 - seg.cp2.0);
    let dy =
        c0 * (seg.cp1.1 - seg.p0.1) + c1 * (seg.cp2.1 - seg.cp1.1) + c2 * (seg.p1.1 - seg.cp2.1);

    (dx, dy)
}

/// Suddivide esattamente un segmento di Bézier cubico in due sotto-segmenti nel punto parametrico t in [0.0, 1.0]
/// utilizzando l'algoritmo numericamente stabile di de Casteljau.
pub fn subdivide_cubic_bezier(
    seg: &CubicBezierSegment,
    t: f64,
) -> (CubicBezierSegment, CubicBezierSegment) {
    let t = t.clamp(0.0, 1.0);
    let u = 1.0 - t;

    let lerp = |p1: (f64, f64), p2: (f64, f64)| -> (f64, f64) {
        (u * p1.0 + t * p2.0, u * p1.1 + t * p2.1)
    };

    let q0 = lerp(seg.p0, seg.cp1);
    let q1 = lerp(seg.cp1, seg.cp2);
    let q2 = lerp(seg.cp2, seg.p1);

    let r0 = lerp(q0, q1);
    let r1 = lerp(q1, q2);

    let s0 = lerp(r0, r1);

    let w_mid = seg.evaluate_width(t);

    let left = CubicBezierSegment {
        p0: seg.p0,
        cp1: q0,
        cp2: r0,
        p1: s0,
        width0: seg.width0,
        width1: w_mid,
    };
    let right = CubicBezierSegment {
        p0: s0,
        cp1: r1,
        cp2: q2,
        p1: seg.p1,
        width0: w_mid,
        width1: seg.width1,
    };

    (left, right)
}

/// Estrae un sotto-segmento di Bézier cubico nell'intervallo [t_start, t_end] in [0.0, 1.0].
pub fn extract_bezier_subsegment(
    seg: &CubicBezierSegment,
    t_start: f64,
    t_end: f64,
) -> CubicBezierSegment {
    let t_start = t_start.clamp(0.0, 1.0);
    let t_end = t_end.clamp(0.0, 1.0);

    if t_end <= t_start + 1e-9 {
        let pt = seg.evaluate(t_start);
        let w = seg.evaluate_width(t_start);
        return CubicBezierSegment {
            p0: pt,
            cp1: pt,
            cp2: pt,
            p1: pt,
            width0: w,
            width1: w,
        };
    }

    if t_start <= 1e-9 && t_end >= 1.0 - 1e-9 {
        return seg.clone();
    }

    if t_start <= 1e-9 {
        let (left, _) = subdivide_cubic_bezier(seg, t_end);
        return left;
    }

    if t_end >= 1.0 - 1e-9 {
        let (_, right) = subdivide_cubic_bezier(seg, t_start);
        return right;
    }

    // Caso generale: suddividi a t_start e prendi la parte destra, poi suddividi a t_end relativo
    let (_, right) = subdivide_cubic_bezier(seg, t_start);
    let rem = 1.0 - t_start;
    let t_sub = if rem > 1e-9 {
        ((t_end - t_start) / rem).clamp(0.0, 1.0)
    } else {
        1.0
    };
    let (sub, _) = subdivide_cubic_bezier(&right, t_sub);
    sub
}

/// Trova tutti i valori parametrici t in [0.0, 1.0] in cui il segmento di Bézier cubico
/// interseca la circonferenza di centro (center.0, center.1) e raggio `radius`.
pub fn find_bezier_circle_intersections(
    seg: &CubicBezierSegment,
    center: (f64, f64),
    radius: f64,
) -> Vec<f64> {
    if !radius.is_finite() || radius <= 0.0 || !center.0.is_finite() || !center.1.is_finite() {
        return Vec::new();
    }

    // 1. Pre-filtro AABB rapido sui 4 punti di controllo
    let min_x = seg.p0.0.min(seg.cp1.0).min(seg.cp2.0).min(seg.p1.0);
    let max_x = seg.p0.0.max(seg.cp1.0).max(seg.cp2.0).max(seg.p1.0);
    let min_y = seg.p0.1.min(seg.cp1.1).min(seg.cp2.1).min(seg.p1.1);
    let max_y = seg.p0.1.max(seg.cp1.1).max(seg.cp2.1).max(seg.p1.1);

    if max_x < center.0 - radius
        || min_x > center.0 + radius
        || max_y < center.1 - radius
        || min_y > center.1 + radius
    {
        return Vec::new();
    }

    let mut roots = Vec::new();
    let query = CircleQuery { center, radius };
    find_intersections_recursive(seg, seg, (0.0, 1.0), query, 0, &mut roots);

    roots.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    roots.dedup_by(|a, b| (*a - *b).abs() < 1e-5);
    roots
}

#[derive(Clone, Copy)]
struct CircleQuery {
    center: (f64, f64),
    radius: f64,
}

fn dist_sq(p: (f64, f64), c: (f64, f64)) -> f64 {
    let dx = p.0 - c.0;
    let dy = p.1 - c.1;
    dx * dx + dy * dy
}

fn find_intersections_recursive(
    original: &CubicBezierSegment,
    sub_seg: &CubicBezierSegment,
    t_range: (f64, f64),
    query: CircleQuery,
    depth: usize,
    roots: &mut Vec<f64>,
) {
    let center = query.center;
    let radius = query.radius;
    let (t_min, t_max) = t_range;
    let r_sq = radius * radius;

    // Controllo AABB del sub-segmento
    let min_x = sub_seg
        .p0
        .0
        .min(sub_seg.cp1.0)
        .min(sub_seg.cp2.0)
        .min(sub_seg.p1.0);
    let max_x = sub_seg
        .p0
        .0
        .max(sub_seg.cp1.0)
        .max(sub_seg.cp2.0)
        .max(sub_seg.p1.0);
    let min_y = sub_seg
        .p0
        .1
        .min(sub_seg.cp1.1)
        .min(sub_seg.cp2.1)
        .min(sub_seg.p1.1);
    let max_y = sub_seg
        .p0
        .1
        .max(sub_seg.cp1.1)
        .max(sub_seg.cp2.1)
        .max(sub_seg.p1.1);

    if max_x < center.0 - radius
        || min_x > center.0 + radius
        || max_y < center.1 - radius
        || min_y > center.1 + radius
    {
        return;
    }

    let d0 = dist_sq(sub_seg.p0, center);
    let d1 = dist_sq(sub_seg.cp1, center);
    let d2 = dist_sq(sub_seg.cp2, center);
    let d3 = dist_sq(sub_seg.p1, center);

    // Se tutti i 4 punti di controllo sono strettamente interni al cerchio:
    // per convessità del disco, l'intero tratto è interno => zero intersezioni con la circonferenza.
    if d0 < r_sq && d1 < r_sq && d2 < r_sq && d3 < r_sq {
        return;
    }

    // Se tutti i 4 punti di controllo sono esterni e la minima distanza dell'AABB dal centro > radius:
    // l'intero tratto è strettamente esterno => zero intersezioni.
    let dx_closest = if center.0 < min_x {
        min_x - center.0
    } else if center.0 > max_x {
        center.0 - max_x
    } else {
        0.0
    };
    let dy_closest = if center.1 < min_y {
        min_y - center.1
    } else if center.1 > max_y {
        center.1 - max_y
    } else {
        0.0
    };
    if dx_closest * dx_closest + dy_closest * dy_closest > r_sq {
        return;
    }

    // Test di planarità della corda
    let chord_dx = sub_seg.p1.0 - sub_seg.p0.0;
    let chord_dy = sub_seg.p1.1 - sub_seg.p0.1;
    let chord_len_sq = chord_dx * chord_dx + chord_dy * chord_dy;

    if chord_len_sq < 1e-12 {
        let cp1_dist_sq =
            (sub_seg.cp1.0 - sub_seg.p0.0).powi(2) + (sub_seg.cp1.1 - sub_seg.p0.1).powi(2);
        let cp2_dist_sq =
            (sub_seg.cp2.0 - sub_seg.p0.0).powi(2) + (sub_seg.cp2.1 - sub_seg.p0.1).powi(2);
        if (cp1_dist_sq > 1e-12 || cp2_dist_sq > 1e-12) && depth < 12 {
            let (left, right) = subdivide_cubic_bezier(sub_seg, 0.5);
            let t_mid = (t_min + t_max) / 2.0;
            find_intersections_recursive(original, &left, (t_min, t_mid), query, depth + 1, roots);
            find_intersections_recursive(original, &right, (t_mid, t_max), query, depth + 1, roots);
            return;
        }

        let d = d0.sqrt();
        if (d - radius).abs() < 0.5 {
            roots.push(((t_min + t_max) / 2.0).clamp(0.0, 1.0));
        }
        return;
    }

    // Deviazione perpendicolare di cp1 e cp2 rispetto alla corda
    let d_cp1 = (chord_dy * (sub_seg.cp1.0 - sub_seg.p0.0)
        - chord_dx * (sub_seg.cp1.1 - sub_seg.p0.1))
        .abs();
    let d_cp2 = (chord_dy * (sub_seg.cp2.0 - sub_seg.p0.0)
        - chord_dx * (sub_seg.cp2.1 - sub_seg.p0.1))
        .abs();

    // Soglia deviazione < 0.25 px o profondità massima raggiunta
    let max_dev_sq = d_cp1.max(d_cp2).powi(2);

    // Uniformità della velocità parametrica lungo la corda
    let t_proj1 = (chord_dx * (sub_seg.cp1.0 - sub_seg.p0.0)
        + chord_dy * (sub_seg.cp1.1 - sub_seg.p0.1))
        / chord_len_sq;
    let t_proj2 = (chord_dx * (sub_seg.cp2.0 - sub_seg.p0.0)
        + chord_dy * (sub_seg.cp2.1 - sub_seg.p0.1))
        / chord_len_sq;
    let is_param_uniform = (t_proj1 - 1.0 / 3.0).abs() < 0.15 && (t_proj2 - 2.0 / 3.0).abs() < 0.15;

    let is_flat = max_dev_sq <= 0.0625 * chord_len_sq && is_param_uniform;

    if is_flat || depth >= 12 {
        // Risoluzione esatta quadratica dell'intersezione cerchio-corda
        // p(u) = p0 + u * V,  ||p(u) - C||^2 = R^2
        let delta_x = sub_seg.p0.0 - center.0;
        let delta_y = sub_seg.p0.1 - center.1;

        let a = chord_len_sq;
        let b = 2.0 * (delta_x * chord_dx + delta_y * chord_dy);
        let c = delta_x * delta_x + delta_y * delta_y - r_sq;

        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let sqrt_disc = disc.sqrt();
            let mut u1 = (-b - sqrt_disc) / (2.0 * a);
            let mut u2 = (-b + sqrt_disc) / (2.0 * a);
            if u1 > u2 {
                std::mem::swap(&mut u1, &mut u2);
            }

            let u_mid = (u1 + u2) / 2.0;

            for (idx, u) in [u1, u2].iter().enumerate() {
                if (-0.05..=1.05).contains(u) {
                    let u_clamped = u.clamp(0.0, 1.0);
                    let mut t_cand = t_min + u_clamped * (t_max - t_min);

                    // Range di clamping per evitare che una radice collassi nel bacino dell'altra
                    let (t_bound_min, t_bound_max) = if (u2 - u1).abs() > 1e-4 {
                        let t_split = (t_min + u_mid * (t_max - t_min)).clamp(t_min, t_max);
                        if idx == 0 {
                            (t_min.max(0.0), t_split)
                        } else {
                            (t_split, t_max.min(1.0))
                        }
                    } else {
                        (t_min.max(0.0), t_max.min(1.0))
                    };

                    // Rifinitura con 3 iterazioni di Newton-Raphson sulla curva originale
                    for _ in 0..3 {
                        let pt = original.evaluate(t_cand);
                        let qx = pt.0 - center.0;
                        let qy = pt.1 - center.1;
                        let f = qx * qx + qy * qy - r_sq;
                        let b_prime = bezier_derivative(original, t_cand);
                        let f_prime = 2.0 * (qx * b_prime.0 + qy * b_prime.1);

                        if f_prime.abs() > 1e-9 {
                            t_cand = (t_cand - f / f_prime).clamp(t_bound_min, t_bound_max);
                        }
                    }

                    let pt_final = original.evaluate(t_cand);
                    let d_final =
                        ((pt_final.0 - center.0).powi(2) + (pt_final.1 - center.1).powi(2)).sqrt();
                    if (d_final - radius).abs() < 0.5 {
                        roots.push(t_cand.clamp(0.0, 1.0));
                    }
                }
            }
        }
        return;
    }

    // Suddivisione ricorsiva a metà
    let (left, right) = subdivide_cubic_bezier(sub_seg, 0.5);
    let t_mid = (t_min + t_max) / 2.0;
    find_intersections_recursive(original, &left, (t_min, t_mid), query, depth + 1, roots);
    find_intersections_recursive(original, &right, (t_mid, t_max), query, depth + 1, roots);
}

/// Ricostruisce un sotto-tratto valido e ben condizionato per l'intervallo di parametri globali
/// [theta_start, theta_end] con theta in [0.0, segments.len()].
///
/// Restituisce `None` se la lunghezza totale del sotto-tratto è inferiore a 1.5 px (scarto polvere visiva).
pub fn reconstruct_sub_stroke(
    original_stroke: &Stroke,
    segments: &[CubicBezierSegment],
    mut theta_start: f64,
    mut theta_end: f64,
    new_id: u64,
) -> Option<Stroke> {
    let m = segments.len();
    if m == 0 || original_stroke.points.is_empty() {
        return None;
    }

    if theta_end <= theta_start + 1e-6 {
        return None;
    }

    theta_start = theta_start.clamp(0.0, m as f64);
    theta_end = theta_end.clamp(0.0, m as f64);

    let mut i_a = (theta_start.floor() as usize).min(m - 1);
    let mut t_a = (theta_start - i_a as f64).clamp(0.0, 1.0);
    let mut i_b = (theta_end.floor() as usize).min(m - 1);
    let mut t_b = (theta_end - i_b as f64).clamp(0.0, 1.0);

    // Normalizzazione per evitare segmenti microscopici degeneri ai bordi
    if (1.0 - t_a) < 1e-5 && i_a < i_b && i_a < m - 1 {
        i_a += 1;
        t_a = 0.0;
    }
    if t_b < 1e-5 && i_b > i_a {
        i_b -= 1;
        t_b = 1.0;
    }

    if i_a > i_b {
        return None;
    }

    if i_a == i_b && t_b <= t_a + 1e-5 {
        return None;
    }

    // 1. Estrazione dei segmenti Bézier esatti via de Casteljau
    let mut sub_bezier = Vec::new();
    if i_a == i_b {
        let seg = extract_bezier_subsegment(&segments[i_a], t_a, t_b);
        sub_bezier.push(seg);
    } else {
        sub_bezier.push(extract_bezier_subsegment(&segments[i_a], t_a, 1.0));
        if i_a < i_b {
            sub_bezier.extend(segments[(i_a + 1)..i_b].iter().cloned());
        }
        sub_bezier.push(extract_bezier_subsegment(&segments[i_b], 0.0, t_b));
    }

    // 2. Controllo scarto polvere visiva (< 1.5 px)
    let total_len: f64 = sub_bezier.iter().map(|s| s.approximate_length()).sum();
    if total_len < 1.5 {
        return None;
    }

    // Helper per interpolare coordinate, spessore, pressione e timestamp
    let interpolate_point = |seg_idx: usize, t: f64| -> StrokePoint {
        let t = t.clamp(0.0, 1.0);
        let pt = segments[seg_idx].evaluate(t);
        let p_prev = &original_stroke.points[seg_idx];
        let p_next = &original_stroke.points[(seg_idx + 1).min(original_stroke.points.len() - 1)];

        let width = p_prev.width + t * (p_next.width - p_prev.width);
        let pressure = p_prev.pressure + t * (p_next.pressure - p_prev.pressure);
        let ts = if p_next.timestamp_ns >= p_prev.timestamp_ns {
            p_prev.timestamp_ns + (t * ((p_next.timestamp_ns - p_prev.timestamp_ns) as f64)) as u64
        } else {
            p_prev.timestamp_ns
        };

        StrokePoint {
            x: pt.0,
            y: pt.1,
            pressure,
            width,
            timestamp_ns: ts,
        }
    };

    let start_pt = if theta_start < 1e-6 {
        original_stroke.points[0].clone()
    } else {
        interpolate_point(i_a, t_a)
    };

    let end_pt = if (m as f64 - theta_end) < 1e-6 {
        original_stroke.points[original_stroke.points.len() - 1].clone()
    } else {
        interpolate_point(i_b, t_b)
    };

    let mut points = Vec::new();
    if i_a == i_b {
        // Se il tratto è interamente dentro un singolo segmento:
        // Controlliamo se è significativamente curvo (> 0.25 px)
        let s = &sub_bezier[0];
        let chord_dx = s.p1.0 - s.p0.0;
        let chord_dy = s.p1.1 - s.p0.1;
        let chord_len_sq = chord_dx * chord_dx + chord_dy * chord_dy;
        let is_curved = if chord_len_sq > 1e-6 {
            let d1 = (chord_dy * (s.cp1.0 - s.p0.0) - chord_dx * (s.cp1.1 - s.p0.1)).abs();
            let d2 = (chord_dy * (s.cp2.0 - s.p0.0) - chord_dx * (s.cp2.1 - s.p0.1)).abs();
            d1.max(d2).powi(2) > 0.0625 * chord_len_sq
        } else {
            false
        };

        if is_curved {
            // Inserisci punto mediano per preservare la curvatura su serializzazione e reload
            let mid_pt = interpolate_point(i_a, (t_a + t_b) / 2.0);
            points.push(start_pt);
            points.push(mid_pt);
            points.push(end_pt);

            let (b_left, b_right) = subdivide_cubic_bezier(&sub_bezier[0], 0.5);
            sub_bezier = vec![b_left, b_right];
        } else {
            points.push(start_pt);
            points.push(end_pt);
        }
    } else {
        points.push(start_pt);
        for k in 1..m {
            let theta_k = k as f64;
            if theta_k > theta_start + 1e-4 && theta_k < theta_end - 1e-4 {
                points.push(original_stroke.points[k].clone());
            }
        }
        points.push(end_pt);
    }

    // Deduplicazione punti coincidenti (distanza < 1e-3 px)
    let mut deduped_points: Vec<StrokePoint> = Vec::with_capacity(points.len());
    for p in points {
        if let Some(last) = deduped_points.last() {
            let d = ((p.x - last.x).powi(2) + (p.y - last.y).powi(2)).sqrt();
            if d < 1e-3 {
                continue;
            }
        }
        deduped_points.push(p);
    }

    if deduped_points.len() < 2 {
        return None;
    }

    // Filtraggio e/o sintesi dei raw_samples
    let start_ts = deduped_points.first().unwrap().timestamp_ns;
    let end_ts = deduped_points.last().unwrap().timestamp_ns;
    let mut raw_samples: Vec<InputSample> = original_stroke
        .raw_samples
        .iter()
        .filter(|s| s.timestamp_ns >= start_ts && s.timestamp_ns <= end_ts)
        .cloned()
        .collect();

    if raw_samples.len() < 2 {
        raw_samples.clear();
        let first_p = &deduped_points[0];
        let last_p = &deduped_points[deduped_points.len() - 1];
        raw_samples.push(InputSample::new(
            (first_p.x, first_p.y),
            Some(first_p.pressure),
            first_p.timestamp_ns,
            DeviceKind::Stylus,
            InputPhase::Down,
        ));
        raw_samples.push(InputSample::new(
            (last_p.x, last_p.y),
            Some(last_p.pressure),
            last_p.timestamp_ns,
            DeviceKind::Stylus,
            InputPhase::Up,
        ));
    } else {
        if let Some(first) = raw_samples.first_mut() {
            first.phase = InputPhase::Down;
        }
        if let Some(last) = raw_samples.last_mut() {
            last.phase = InputPhase::Up;
        }
    }

    let mut sub_stroke = Stroke {
        id: new_id,
        style: original_stroke.style.clone(),
        points: deduped_points,
        raw_samples,
        is_dot: false,
        mode: original_stroke.mode,
        cached_bbox: None,
        cached_bezier: sub_bezier,
    };
    sub_stroke.cached_bbox = sub_stroke.compute_bounding_box();

    Some(sub_stroke)
}

/// Esegue il taglio geometrico analitico del tratto vettoriale con un cerchio gomma.
///
/// Restituisce:
/// - `Vec::new()` se il tratto è interamente cancellato o se i frammenti sono polvere (< 1.5 px).
/// - `vec![stroke.clone()]` se non c'è intersezione con la gomma.
/// - `vec![accorciato]` se viene tagliata un'estremità iniziale o finale.
/// - `vec![frammento_1, frammento_2, ...]` se il tratto viene suddiviso internamente.
pub fn slice_stroke_with_circle(stroke: &Stroke, center: (f64, f64), radius: f64) -> Vec<Stroke> {
    if !radius.is_finite() || radius <= 0.0 || !center.0.is_finite() || !center.1.is_finite() {
        return vec![stroke.clone()];
    }

    if stroke.points.is_empty() {
        return Vec::new();
    }

    // Gestione punto isolato / dot stroke
    if stroke.is_dot || stroke.points.len() == 1 {
        let p = &stroke.points[0];
        let d = ((p.x - center.0).powi(2) + (p.y - center.1).powi(2)).sqrt();
        let eff_r = radius + p.width / 2.0;
        if d <= eff_r {
            return Vec::new(); // Cancellato
        } else {
            return vec![stroke.clone()]; // Intatto
        }
    }

    // Pre-filtro AABB rapido O(1)
    if let Some(bbox) = stroke.bounding_box() {
        if center.0 < bbox.min_x - radius
            || center.0 > bbox.max_x + radius
            || center.1 < bbox.min_y - radius
            || center.1 > bbox.max_y + radius
        {
            return vec![stroke.clone()];
        }
    }

    // Recupero segmenti Bézier (dalla cache a zero allocazioni, o calcolati da punti)
    let segments = if !stroke.cached_bezier.is_empty() {
        stroke.cached_bezier.clone()
    } else {
        crate::geometry::catmull_rom_to_bezier(&stroke.points)
    };

    if segments.is_empty() {
        return vec![stroke.clone()];
    }

    let m = segments.len();
    let mut cut_thetas: Vec<f64> = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        let roots = find_bezier_circle_intersections(seg, center, radius);
        for t in roots {
            cut_thetas.push((i as f64) + t.clamp(0.0, 1.0));
        }
    }

    // Aggiungi estremi globali del tratto
    cut_thetas.push(0.0);
    cut_thetas.push(m as f64);
    cut_thetas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    cut_thetas.dedup_by(|a, b| (*a - *b).abs() < 1e-5);

    // Determinazione intervalli sopravvissuti (esterni al cerchio gomma)
    let mut surviving_intervals: Vec<(f64, f64)> = Vec::new();

    for w in cut_thetas.windows(2) {
        let t0 = w[0];
        let t1 = w[1];
        if t1 - t0 < 1e-5 {
            continue;
        }

        let t_mid = (t0 + t1) / 2.0;
        let seg_idx = (t_mid.floor() as usize).min(m - 1);
        let local_t = (t_mid - seg_idx as f64).clamp(0.0, 1.0);
        let pt = segments[seg_idx].evaluate(local_t);
        let dist = ((pt.0 - center.0).powi(2) + (pt.1 - center.1).powi(2)).sqrt();

        if dist > radius {
            if let Some(last) = surviving_intervals.last_mut() {
                if (last.1 - t0).abs() < 1e-5 {
                    last.1 = t1;
                    continue;
                }
            }
            surviving_intervals.push((t0, t1));
        }
    }

    // Esito globale
    if surviving_intervals.is_empty() {
        return Vec::new(); // Cancellazione completa
    }

    if surviving_intervals.len() == 1 {
        let (t_start, t_end) = surviving_intervals[0];
        if t_start < 1e-5 && (t_end - m as f64).abs() < 1e-5 {
            return vec![stroke.clone()]; // Nessuna intersezione effettiva
        }
    }

    // Ricostruzione sotto-tratti
    let mut result_strokes = Vec::with_capacity(surviving_intervals.len());
    for (sub_idx, (t_start, t_end)) in surviving_intervals.into_iter().enumerate() {
        let sub_id = if sub_idx == 0 {
            stroke.id
        } else {
            // Calcolo deterministico univoco per ciascun frammento successivo:
            // combina stroke.id, sub_idx e t_start per prevenire collisioni
            // anche in caso di affettamenti ricorsivi successivi sul frammento 0.
            let t_bits = t_start.to_bits();
            let mut h = stroke
                .id
                .wrapping_add((sub_idx as u64).wrapping_mul(0x9E3779B97F4A7C15));
            h ^= t_bits.wrapping_mul(0xBF58476D1CE4E5B9);
            h = (h ^ (h >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            h = (h ^ (h >> 27)).wrapping_mul(0x94D049BB133111EB);
            let candidate = h ^ (h >> 31);
            if candidate == stroke.id {
                candidate.wrapping_add(0x9E3779B97F4A7C15)
            } else {
                candidate
            }
        };

        if let Some(sub_stroke) = reconstruct_sub_stroke(stroke, &segments, t_start, t_end, sub_id)
        {
            result_strokes.push(sub_stroke);
        }
    }

    result_strokes
}
