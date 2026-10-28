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

/// Segmento di curva di Bézier cubica definito da punto iniziale, due punti di controllo e punto finale,
/// con spessori iniziale e finale associati.
#[derive(Debug, Clone, PartialEq)]
pub struct CubicBezierSegment {
    pub p0: (f64, f64),
    pub cp1: (f64, f64),
    pub cp2: (f64, f64),
    pub p1: (f64, f64),
    pub width0: f64,
    pub width1: f64,
}

impl CubicBezierSegment {
    /// Trasla il segmento di Bézier di un offset (dx, dy) in-place a zero ri-calcoli spline.
    #[inline]
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.p0.0 += dx;
        self.p0.1 += dy;
        self.cp1.0 += dx;
        self.cp1.1 += dy;
        self.cp2.0 += dx;
        self.cp2.1 += dy;
        self.p1.0 += dx;
        self.p1.1 += dy;
    }

    /// Scala il segmento di Bézier rispetto a un'origine (origin_x, origin_y) con fattori (sx, sy).
    /// Lo spessore del tratto viene scalato isotropicamente con la media quadratica dei fattori.
    #[inline]
    pub fn scale(&mut self, sx: f64, sy: f64, origin_x: f64, origin_y: f64) {
        let scale_pt = |p: (f64, f64)| {
            (
                origin_x + (p.0 - origin_x) * sx,
                origin_y + (p.1 - origin_y) * sy,
            )
        };
        self.p0 = scale_pt(self.p0);
        self.cp1 = scale_pt(self.cp1);
        self.cp2 = scale_pt(self.cp2);
        self.p1 = scale_pt(self.p1);

        let sw = ((sx * sx + sy * sy) / 2.0).sqrt();
        self.width0 *= sw;
        self.width1 *= sw;
    }

    /// Ruota il segmento di Bézier di un angolo (in radianti) rispetto a un centro (origin_x, origin_y).
    #[inline]
    pub fn rotate(&mut self, angle_rad: f64, origin_x: f64, origin_y: f64) {
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();
        let rotate_pt = |p: (f64, f64)| {
            let dx = p.0 - origin_x;
            let dy = p.1 - origin_y;
            (
                origin_x + dx * cos_a - dy * sin_a,
                origin_y + dx * sin_a + dy * cos_a,
            )
        };
        self.p0 = rotate_pt(self.p0);
        self.cp1 = rotate_pt(self.cp1);
        self.cp2 = rotate_pt(self.cp2);
        self.p1 = rotate_pt(self.p1);
    }

    /// Valuta la coordinata 2D (x, y) lungo la curva per il parametro t in [0.0, 1.0].
    pub fn evaluate(&self, t: f64) -> (f64, f64) {
        let t = t.clamp(0.0, 1.0);
        let u = 1.0 - t;
        let tt = t * t;
        let uu = u * u;
        let uuu = uu * u;
        let ttt = tt * t;

        let x = uuu * self.p0.0
            + 3.0 * uu * t * self.cp1.0
            + 3.0 * u * tt * self.cp2.0
            + ttt * self.p1.0;

        let y = uuu * self.p0.1
            + 3.0 * uu * t * self.cp1.1
            + 3.0 * u * tt * self.cp2.1
            + ttt * self.p1.1;

        (x, y)
    }

    /// Valuta lo spessore interpolato linearmente lungo la curva per il parametro t in [0.0, 1.0].
    pub fn evaluate_width(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        self.width0 + t * (self.width1 - self.width0)
    }

    /// Stima la lunghezza d'arco del segmento di Bézier cubico combinando corda e poligono di controllo.
    pub fn approximate_length(&self) -> f64 {
        let chord = distance(self.p0, self.p1);
        let net = distance(self.p0, self.cp1)
            + distance(self.cp1, self.cp2)
            + distance(self.cp2, self.p1);
        (chord + net) / 2.0
    }

    /// Verifica se tutte le coordinate e gli spessori del segmento sono valori numerici finiti (non NaN né Inf).
    #[inline]
    pub fn is_finite(&self) -> bool {
        self.p0.0.is_finite()
            && self.p0.1.is_finite()
            && self.cp1.0.is_finite()
            && self.cp1.1.is_finite()
            && self.cp2.0.is_finite()
            && self.cp2.1.is_finite()
            && self.p1.0.is_finite()
            && self.p1.1.is_finite()
            && self.width0.is_finite()
            && self.width1.is_finite()
    }
}

/// Converte una sequenza di punti del tratto in una serie di segmenti di Bézier cubici
/// continui di classe C1 tramite interpolazione spline di Catmull-Rom con controllo rigoroso degli overshoot.
pub fn catmull_rom_to_bezier(points: &[StrokePoint]) -> Vec<CubicBezierSegment> {
    let n = points.len();
    if n < 2 {
        return Vec::new();
    }

    if n == 2 {
        let p0 = (points[0].x, points[0].y);
        let p1 = (points[1].x, points[1].y);
        let cp1 = (p0.0 + (p1.0 - p0.0) / 3.0, p0.1 + (p1.1 - p0.1) / 3.0);
        let cp2 = (p1.0 - (p1.0 - p0.0) / 3.0, p1.1 - (p1.1 - p0.1) / 3.0);
        return vec![CubicBezierSegment {
            p0,
            cp1,
            cp2,
            p1,
            width0: points[0].width,
            width1: points[1].width,
        }];
    }

    let mut segments = Vec::with_capacity(n - 1);

    for i in 0..(n - 1) {
        let p_i = (points[i].x, points[i].y);
        let p_ip1 = (points[i + 1].x, points[i + 1].y);

        let p_prev = if i == 0 {
            // Estrapolazione lineare speculare per il primo punto
            (2.0 * p_i.0 - p_ip1.0, 2.0 * p_i.1 - p_ip1.1)
        } else {
            (points[i - 1].x, points[i - 1].y)
        };

        let p_next = if i + 1 == n - 1 {
            // Estrapolazione lineare speculare per l'ultimo punto
            (2.0 * p_ip1.0 - p_i.0, 2.0 * p_ip1.1 - p_i.1)
        } else {
            (points[i + 2].x, points[i + 2].y)
        };

        // Vettori tangenti Catmull-Rom
        let mut v1 = ((p_ip1.0 - p_prev.0) / 6.0, (p_ip1.1 - p_prev.1) / 6.0);
        let mut v2 = ((p_next.0 - p_i.0) / 6.0, (p_next.1 - p_i.1) / 6.0);

        // Controllo overshoot: limita l'estensione dei bracci tangenti a 1/3 della corda
        let chord_len = distance(p_i, p_ip1);
        let max_handle = chord_len / 3.0;

        let len_v1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
        if len_v1 > max_handle && len_v1 > 1e-6 {
            let scale = max_handle / len_v1;
            v1.0 *= scale;
            v1.1 *= scale;
        }

        let len_v2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
        if len_v2 > max_handle && len_v2 > 1e-6 {
            let scale = max_handle / len_v2;
            v2.0 *= scale;
            v2.1 *= scale;
        }

        let cp1 = (p_i.0 + v1.0, p_i.1 + v1.1);
        let cp2 = (p_ip1.0 - v2.0, p_ip1.1 - v2.1);

        segments.push(CubicBezierSegment {
            p0: p_i,
            cp1,
            cp2,
            p1: p_ip1,
            width0: points[i].width,
            width1: points[i + 1].width,
        });
    }

    segments
}

/// Campiona la spline Catmull-Rom a intervalli regolari (massimo `max_step_px` pixel)
/// restituendo coordinate (x, y) e larghezza interpolata.
pub fn sample_catmull_rom_spline(points: &[StrokePoint], max_step_px: f64) -> Vec<(f64, f64, f64)> {
    let segments = catmull_rom_to_bezier(points);
    if segments.is_empty() {
        return points.iter().map(|p| (p.x, p.y, p.width)).collect();
    }

    let max_step = max_step_px.max(0.5);
    let mut sampled = Vec::new();

    for (idx, seg) in segments.iter().enumerate() {
        let len = seg.approximate_length();
        let steps = ((len / max_step).ceil() as usize).clamp(1, 32);

        // Per il primo segmento includiamo anche t = 0
        let start_step = if idx == 0 { 0 } else { 1 };
        for s in start_step..=steps {
            let t = (s as f64) / (steps as f64);
            let (x, y) = seg.evaluate(t);
            let w = seg.evaluate_width(t);
            sampled.push((x, y, w));
        }
    }

    sampled
}
