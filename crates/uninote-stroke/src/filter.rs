use std::f64::consts::PI;

/// Parametri di configurazione per il filtro One Euro (1€ Filter).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OneEuroConfig {
    /// Frequenza minima di taglio in Hertz (fc_min).
    /// Valori più bassi riducono maggiormente il jitter a bassa velocità.
    pub min_cutoff: f64,
    /// Coefficiente di velocità (beta).
    /// Valori più alti aumentano la frequenza di taglio all'aumentare della velocità, riducendo la latenza nei tratti veloci.
    pub beta: f64,
    /// Frequenza di taglio per il calcolo della derivata (d_cutoff), tipicamente 1.0 Hz.
    pub d_cutoff: f64,
}

impl Default for OneEuroConfig {
    fn default() -> Self {
        Self {
            min_cutoff: 1.0,
            beta: 0.007,
            d_cutoff: 1.0,
        }
    }
}

/// Filtro passa-basso esponenziale del primo ordine a frequenza variabile.
#[derive(Debug, Clone)]
struct LowPassFilter {
    y: Option<f64>,
}

impl LowPassFilter {
    fn new() -> Self {
        Self { y: None }
    }

    fn filter(&mut self, value: f64, alpha: f64) -> f64 {
        let alpha = alpha.clamp(0.0, 1.0);
        let result = match self.y {
            Some(prev) => alpha * value + (1.0 - alpha) * prev,
            None => value,
        };
        self.y = Some(result);
        result
    }

    fn last_value(&self) -> Option<f64> {
        self.y
    }

    fn reset(&mut self) {
        self.y = None;
    }
}

fn compute_alpha(rate: f64, cutoff: f64) -> f64 {
    let tau = 1.0 / (2.0 * PI * cutoff);
    let te = 1.0 / rate;
    1.0 / (1.0 + tau / te)
}

/// Filtro One Euro per singoli valori scalari (es. pressione o singola coordinata).
#[derive(Debug, Clone)]
pub struct OneEuroScalar {
    config: OneEuroConfig,
    x_filter: LowPassFilter,
    dx_filter: LowPassFilter,
    last_time_s: Option<f64>,
}

impl OneEuroScalar {
    pub fn new(config: OneEuroConfig) -> Self {
        Self {
            config,
            x_filter: LowPassFilter::new(),
            dx_filter: LowPassFilter::new(),
            last_time_s: None,
        }
    }

    pub fn filter(&mut self, value: f64, timestamp_s: f64) -> f64 {
        if !value.is_finite() {
            return self.x_filter.last_value().unwrap_or(0.0);
        }

        let dt = match self.last_time_s {
            Some(prev_t) => {
                let diff = timestamp_s - prev_t;
                if diff <= 0.0 {
                    1e-3 // Fallback protetto a 1ms (1000 Hz) per campioni con timestamp coincidente
                } else {
                    diff.clamp(5e-4, 1.0) // Clamping tra 0.5ms (max 2000 Hz) e 1s per evitare spike
                }
            }
            None => {
                self.last_time_s = Some(timestamp_s);
                return self.x_filter.filter(value, 1.0);
            }
        };

        self.last_time_s = Some(timestamp_s);
        let rate = 1.0 / dt;

        // Calcolo della derivata filtrata
        let prev_x = self.x_filter.last_value().unwrap_or(value);
        let dx = (value - prev_x) * rate;
        let edx = self
            .dx_filter
            .filter(dx, compute_alpha(rate, self.config.d_cutoff));

        // Frequenza di taglio adattiva basata sulla velocità di variazione
        let cutoff = self.config.min_cutoff + self.config.beta * edx.abs();
        self.x_filter.filter(value, compute_alpha(rate, cutoff))
    }

    pub fn reset(&mut self) {
        self.x_filter.reset();
        self.dx_filter.reset();
        self.last_time_s = None;
    }
}

/// Filtro One Euro applicato a coordinate bidimensionali (X, Y).
#[derive(Debug, Clone)]
pub struct OneEuroPointFilter {
    x_filter: OneEuroScalar,
    y_filter: OneEuroScalar,
}

impl OneEuroPointFilter {
    pub fn new(config: OneEuroConfig) -> Self {
        Self {
            x_filter: OneEuroScalar::new(config),
            y_filter: OneEuroScalar::new(config),
        }
    }

    pub fn filter(&mut self, x: f64, y: f64, timestamp_ns: u64) -> (f64, f64) {
        let timestamp_s = (timestamp_ns as f64) / 1_000_000_000.0;
        let fx = self.x_filter.filter(x, timestamp_s);
        let fy = self.y_filter.filter(y, timestamp_s);
        (fx, fy)
    }

    pub fn reset(&mut self) {
        self.x_filter.reset();
        self.y_filter.reset();
    }
}
