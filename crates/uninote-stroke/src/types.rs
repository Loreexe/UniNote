use crate::geometry::CubicBezierSegment;
use serde::{Deserialize, Serialize};

/// Identifica la tipologia di dispositivo che ha generato il campione di input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DeviceKind {
    /// Penna stilo attiva o passiva con digitizer.
    Stylus,
    /// Dito o palmo rilevato dal sensore capacitivo touchscreen.
    Touch,
    /// Mouse tradizionale o trackpad generico.
    Mouse,
    /// Sorgente non identificata.
    Unknown,
}

/// Fase temporale del ciclo di contatto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InputPhase {
    /// Contatto iniziale (down / tip down).
    Down,
    /// Movimento con contatto mantenuto (move / drag).
    Move,
    /// Rilascio del contatto (up / tip up).
    Up,
    /// Cancellazione forzata della sequenza (es. gesture del compositor, perdita di focus).
    Cancel,
}

/// Singolo campione grezzo proveniente dal sottosistema di input.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InputSample {
    /// Coordinate in pixel nel sistema di riferimento del canvas/documento.
    pub position: (f64, f64),
    /// Pressione normalizzata in intervallo [0.0, 1.0]. `None` se il dispositivo non supporta la pressione.
    pub pressure: Option<f64>,
    /// Inclinazione (tilt X, tilt Y) in radianti se supportata.
    pub tilt: Option<(f64, f64)>,
    /// Timestamp monotonico ad alta precisione in nanosecondi.
    pub timestamp_ns: u64,
    /// Tipologia di dispositivo sorgente.
    pub device_kind: DeviceKind,
    /// Identificatore univoco dello slot touch o della sorgente hardware.
    pub source_id: u64,
    /// Fase dell'evento.
    pub phase: InputPhase,
}

impl InputSample {
    pub fn new(
        position: (f64, f64),
        pressure: Option<f64>,
        timestamp_ns: u64,
        device_kind: DeviceKind,
        phase: InputPhase,
    ) -> Self {
        Self {
            position,
            pressure,
            tilt: None,
            timestamp_ns,
            device_kind,
            source_id: 0,
            phase,
        }
    }
}

/// Punto geometrico derivato con spessore e pressione calcolati.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokePoint {
    pub x: f64,
    pub y: f64,
    pub pressure: f64,
    pub width: f64,
    pub timestamp_ns: u64,
}

/// Rettangolo di ingombro 2D delimitatore (AABB).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Rect {
    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self {
            min_x,
            min_y,
            max_x,
            max_y,
        }
    }

    pub fn from_point(x: f64, y: f64, radius: f64) -> Self {
        Self {
            min_x: x - radius,
            min_y: y - radius,
            max_x: x + radius,
            max_y: y + radius,
        }
    }

    pub fn expand(&self, margin: f64) -> Self {
        Self {
            min_x: self.min_x - margin,
            min_y: self.min_y - margin,
            max_x: self.max_x + margin,
            max_y: self.max_y + margin,
        }
    }

    pub fn union(&self, other: &Rect) -> Self {
        Self {
            min_x: self.min_x.min(other.min_x),
            min_y: self.min_y.min(other.min_y),
            max_x: self.max_x.max(other.max_x),
            max_y: self.max_y.max(other.max_y),
        }
    }

    pub fn width(&self) -> f64 {
        (self.max_x - self.min_x).max(0.0)
    }

    pub fn height(&self) -> f64 {
        (self.max_y - self.min_y).max(0.0)
    }

    /// Restituisce una copia normalizzata del rettangolo tale che min_x <= max_x e min_y <= max_y.
    pub fn normalized(&self) -> Self {
        Self {
            min_x: self.min_x.min(self.max_x),
            min_y: self.min_y.min(self.max_y),
            max_x: self.min_x.max(self.max_x),
            max_y: self.min_y.max(self.max_y),
        }
    }

    /// Verifica se un punto 2D (px, py) cade all'interno del rettangolo (inclusi i bordi).
    pub fn contains_point(&self, px: f64, py: f64) -> bool {
        let norm = self.normalized();
        px >= norm.min_x && px <= norm.max_x && py >= norm.min_y && py <= norm.max_y
    }

    /// Verifica se un altro rettangolo è interamente contenuto in questo rettangolo.
    pub fn contains_rect(&self, other: &Rect) -> bool {
        let s = self.normalized();
        let o = other.normalized();
        o.min_x >= s.min_x && o.max_x <= s.max_x && o.min_y >= s.min_y && o.max_y <= s.max_y
    }

    /// Verifica se due rettangoli si intersecano o si sovrappongono.
    pub fn intersects_rect(&self, other: &Rect) -> bool {
        let s = self.normalized();
        let o = other.normalized();
        s.min_x <= o.max_x && s.max_x >= o.min_x && s.min_y <= o.max_y && s.max_y >= o.min_y
    }

    /// Trasla il rettangolo di un vettore di spostamento (dx, dy).
    pub fn translate(&self, dx: f64, dy: f64) -> Self {
        Self {
            min_x: self.min_x + dx,
            min_y: self.min_y + dy,
            max_x: self.max_x + dx,
            max_y: self.max_y + dy,
        }
    }
}

/// Stile visivo del tratto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrokeStyle {
    /// Colore RGBA in intervallo [0.0, 1.0].
    pub color: (f64, f64, f64, f64),
    /// Spessore base di riferimento in pixel.
    pub base_width: f64,
    /// Fattore moltiplicativo di larghezza a pressione minima.
    pub min_width_factor: f64,
    /// Fattore moltiplicativo di larghezza a pressione massima.
    pub max_width_factor: f64,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        Self {
            color: (0.1, 0.1, 0.12, 1.0), // Inchiostro scuro quasi nero
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        }
    }
}

/// Modalità della pipeline di elaborazione e rendering del tratto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum PipelineMode {
    /// Spline Catmull-Rom naturale (predefinita): curve C1 fluide e naturali, zero lag su coordinate stilo.
    #[default]
    CatmullRomSpline,
    /// One Euro Filter ricalibrato + Spline Catmull-Rom: filtraggio rumore per mano tremolante o scrittura lenta.
    OneEuroFiltered,
    /// Baseline diretta senza smoothing e senza filtri (poligonale pura da specifica).
    Baseline,
}

/// Rappresentazione di un tratto vettoriale completato o in corso.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub style: StrokeStyle,
    pub points: Vec<StrokePoint>,
    pub raw_samples: Vec<InputSample>,
    pub is_dot: bool,
    pub mode: PipelineMode,
    /// Bounding box AABB 2D in-memory pre-calcolato.
    /// Escluso dalla serializzazione JSON/database: zero alterazioni di schema SQLite.
    #[serde(skip, default)]
    pub cached_bbox: Option<Rect>,
    /// Segmenti di Bézier cubici continui C1 pre-calcolati per rendering zero-allocation.
    /// Esclusi dalla serializzazione JSON/database: zero alterazioni di schema SQLite.
    #[serde(skip, default)]
    pub cached_bezier: Vec<CubicBezierSegment>,
}

impl PartialEq for Stroke {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.style == other.style
            && self.points == other.points
            && self.raw_samples == other.raw_samples
            && self.is_dot == other.is_dot
            && self.mode == other.mode
    }
}

impl Default for Stroke {
    fn default() -> Self {
        Self::new(0, StrokeStyle::default(), PipelineMode::default())
    }
}

impl Stroke {
    pub fn new(id: u64, style: StrokeStyle, mode: PipelineMode) -> Self {
        Self {
            id,
            style,
            points: Vec::new(),
            raw_samples: Vec::new(),
            is_dot: false,
            mode,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        }
    }

    /// Costruttore di convenienza con punti noti che pre-popola immediatamente la cache geometrica.
    pub fn with_points(id: u64, style: StrokeStyle, points: Vec<StrokePoint>) -> Self {
        let mut stroke = Self {
            id,
            style,
            points,
            raw_samples: Vec::new(),
            is_dot: false,
            mode: PipelineMode::default(),
            cached_bbox: None,
            cached_bezier: Vec::new(),
        };
        stroke.recompute_cache();
        stroke
    }

    /// Costruttore con punti e modalità specifica che pre-popola immediatamente la cache geometrica.
    pub fn with_points_and_mode(
        id: u64,
        style: StrokeStyle,
        mode: PipelineMode,
        points: Vec<StrokePoint>,
    ) -> Self {
        let mut stroke = Self {
            id,
            style,
            points,
            raw_samples: Vec::new(),
            is_dot: false,
            mode,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        };
        stroke.recompute_cache();
        stroke
    }

    /// Ricalcola integralmente in-memory sia il bounding box che i segmenti di Bézier.
    pub fn recompute_cache(&mut self) {
        self.cached_bbox = self.compute_bounding_box();
        self.cached_bezier = if self.is_dot || self.points.len() < 2 {
            Vec::new()
        } else {
            match self.mode {
                PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered => {
                    crate::geometry::catmull_rom_to_bezier(&self.points)
                }
                PipelineMode::Baseline => Vec::new(),
            }
        };
    }

    /// Garantisce la presenza dei dati in cache ricalcolandoli se non ancora popolati.
    pub fn ensure_cache(&mut self) {
        if self.cached_bbox.is_none()
            || (self.cached_bezier.is_empty()
                && !self.is_dot
                && self.points.len() >= 2
                && matches!(
                    self.mode,
                    PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered
                ))
        {
            self.recompute_cache();
        }
    }

    /// Calcola il bounding box grezzo iterando sui punti (fallback di sicurezza O(N)).
    pub fn compute_bounding_box(&self) -> Option<Rect> {
        if self.points.is_empty() {
            return None;
        }

        let first = &self.points[0];
        let mut r = Rect::from_point(first.x, first.y, first.width / 2.0);

        for p in &self.points[1..] {
            let point_box = Rect::from_point(p.x, p.y, p.width / 2.0);
            r = r.union(&point_box);
        }

        Some(r)
    }

    /// Restituisce il bounding box precalcolato O(1). Se assente, esegue il fallback on-demand.
    #[inline]
    pub fn bounding_box(&self) -> Option<Rect> {
        if let Some(bbox) = self.cached_bbox {
            Some(bbox)
        } else {
            self.compute_bounding_box()
        }
    }

    /// Restituisce il riferimento ai segmenti di Bézier memorizzati nella cache.
    #[inline]
    pub fn bezier_segments(&self) -> &[CubicBezierSegment] {
        &self.cached_bezier
    }

    pub fn total_length(&self) -> f64 {
        if self.points.len() < 2 {
            return 0.0;
        }

        let mut len = 0.0;
        for w in self.points.windows(2) {
            let dx = w[1].x - w[0].x;
            let dy = w[1].y - w[0].y;
            len += (dx * dx + dy * dy).sqrt();
        }
        len
    }

    /// Verifica se un punto con raggio di tolleranza interseca il tratto (per gomma per tratto intero).
    pub fn intersects_point(&self, (px, py): (f64, f64), radius: f64) -> bool {
        if let Some(bbox) = self.bounding_box() {
            if px < bbox.min_x - radius
                || px > bbox.max_x + radius
                || py < bbox.min_y - radius
                || py > bbox.max_y + radius
            {
                return false;
            }
        } else {
            return false;
        }

        // 1. Controllo prossimità punti
        for p in &self.points {
            let dx = p.x - px;
            let dy = p.y - py;
            let eff_r = radius + p.width / 2.0;
            if dx * dx + dy * dy <= eff_r * eff_r {
                return true;
            }
        }

        // 2. Controllo prossimità segmenti
        for w in self.points.windows(2) {
            let p1 = &w[0];
            let p2 = &w[1];
            let l2 = (p2.x - p1.x).powi(2) + (p2.y - p1.y).powi(2);
            let dist_sq = if l2 < 1e-6 {
                (px - p1.x).powi(2) + (py - p1.y).powi(2)
            } else {
                let t = (((px - p1.x) * (p2.x - p1.x) + (py - p1.y) * (p2.y - p1.y)) / l2)
                    .clamp(0.0, 1.0);
                let proj_x = p1.x + t * (p2.x - p1.x);
                let proj_y = p1.y + t * (p2.y - p1.y);
                (px - proj_x).powi(2) + (py - proj_y).powi(2)
            };
            let eff_w = p1.width.max(p2.width) / 2.0 + radius;
            if dist_sq <= eff_w * eff_w {
                return true;
            }
        }

        false
    }

    /// Trasla tutti i punti, campioni grezzi, bounding box e curve Bézier in-place con zero allocazioni.
    pub fn translate(&mut self, dx: f64, dy: f64) {
        for p in &mut self.points {
            p.x += dx;
            p.y += dy;
        }
        for s in &mut self.raw_samples {
            s.position.0 += dx;
            s.position.1 += dy;
        }
        if let Some(ref mut bbox) = self.cached_bbox {
            *bbox = bbox.translate(dx, dy);
        }
        for seg in &mut self.cached_bezier {
            seg.translate(dx, dy);
        }
    }
}
