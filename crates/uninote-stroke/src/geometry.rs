use crate::types::{StrokePoint, StrokeStyle};

/// Calcola lo spessore del tratto dato un valore di pressione e lo stile corrente.
pub fn compute_width(pressure: Option<f64>, style: &StrokeStyle) -> f64 {
    match pressure {
        Some(p) => {
            // Curva esponenziale/potenza dolce per sensibilità naturale alla pressione
            let clamped_p = p.clamp(0.0, 1.0);
            let factor = style.min_width_factor
                + (style.max_width_factor - style.min_width_factor) * clamped_p.powf(1.15);
            style.base_width * factor
        }
        None => {
            // Dispositivi privi di pressione (mouse, trackpad, stilo capacitivo passivo)
            style.base_width
        }
    }
}

/// Calcola la distanza euclidea tra due coordinate 2D.
pub fn distance(p1: (f64, f64), p2: (f64, f64)) -> f64 {
    let dx = p2.0 - p1.0;
    let dy = p2.1 - p1.1;
    (dx * dx + dy * dy).sqrt()
}

/// Verifica se una sequenza di punti rappresenta un punto isolato (dot/click rapido).
pub fn is_discrete_dot(points: &[StrokePoint], max_travel_px: f64) -> bool {
    if points.is_empty() {
        return false;
    }
    if points.len() == 1 {
        return true;
    }

    let first = &points[0];
    for p in &points[1..] {
        if distance((first.x, first.y), (p.x, p.y)) > max_travel_px {
            return false;
        }
    }
    true
}
