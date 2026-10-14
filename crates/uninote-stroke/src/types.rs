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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Stroke {
    pub id: u64,
    pub style: StrokeStyle,
    pub points: Vec<StrokePoint>,
    pub raw_samples: Vec<InputSample>,
    pub is_dot: bool,
    pub mode: PipelineMode,
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
        }
    }

    pub fn bounding_box(&self) -> Option<Rect> {
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
}
