use crate::filter::{OneEuroConfig, OneEuroPointFilter, OneEuroScalar};
use crate::geometry::{compute_width, is_discrete_dot};
use crate::types::{InputPhase, InputSample, Stroke, StrokePoint, StrokeStyle};

/// Modalità della pipeline di elaborazione del tratto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PipelineMode {
    /// Baseline diretta senza smoothing e senza lag algoritmico.
    Baseline,
    /// Tratto filtrato con One Euro Filter disaccoppiato su posizione e pressione.
    OneEuroFiltered,
}

/// Costruttore di un singolo tratto a partire dal flusso di campioni di input.
pub struct StrokeBuilder {
    pub stroke: Stroke,
    mode: PipelineMode,
    point_filter: OneEuroPointFilter,
    pressure_filter: OneEuroScalar,
    samples_received: u64,
    samples_dropped: u64,
}

impl StrokeBuilder {
    pub fn new(id: u64, style: StrokeStyle, mode: PipelineMode) -> Self {
        let point_filter = OneEuroPointFilter::new(OneEuroConfig::default());
        let pressure_filter = OneEuroScalar::new(OneEuroConfig {
            min_cutoff: 2.0,
            beta: 0.01,
            d_cutoff: 1.0,
        });

        Self {
            stroke: Stroke::new(id, style),
            mode,
            point_filter,
            pressure_filter,
            samples_received: 0,
            samples_dropped: 0,
        }
    }

    /// Riceve un nuovo campione di input ed emette il punto geometrico corrispondente se valido.
    pub fn process_sample(&mut self, sample: InputSample) -> Option<StrokePoint> {
        self.samples_received += 1;
        self.stroke.raw_samples.push(sample.clone());

        match sample.phase {
            InputPhase::Down => {
                self.point_filter.reset();
                self.pressure_filter.reset();

                let (x, y) = match self.mode {
                    PipelineMode::Baseline => sample.position,
                    PipelineMode::OneEuroFiltered => self.point_filter.filter(
                        sample.position.0,
                        sample.position.1,
                        sample.timestamp_ns,
                    ),
                };

                let effective_pressure = match sample.pressure {
                    Some(p) => match self.mode {
                        PipelineMode::Baseline => p,
                        PipelineMode::OneEuroFiltered => {
                            let t_s = (sample.timestamp_ns as f64) / 1_000_000_000.0;
                            self.pressure_filter.filter(p, t_s)
                        }
                    },
                    None => 0.5,
                };

                let width = compute_width(sample.pressure, &self.stroke.style);
                let pt = StrokePoint {
                    x,
                    y,
                    pressure: effective_pressure,
                    width,
                    timestamp_ns: sample.timestamp_ns,
                };

                self.stroke.points.push(pt.clone());
                Some(pt)
            }

            InputPhase::Move => {
                // Controllo duplicati esatti consecutivi: scarta solo se posizione E pressione sono invariate
                if let Some(last_raw) = self
                    .stroke
                    .raw_samples
                    .get(self.stroke.raw_samples.len().saturating_sub(2))
                {
                    if (last_raw.position.0 - sample.position.0).abs() < 1e-4
                        && (last_raw.position.1 - sample.position.1).abs() < 1e-4
                        && last_raw.pressure == sample.pressure
                    {
                        self.samples_dropped += 1;
                        return None;
                    }
                }

                let (x, y) = match self.mode {
                    PipelineMode::Baseline => sample.position,
                    PipelineMode::OneEuroFiltered => self.point_filter.filter(
                        sample.position.0,
                        sample.position.1,
                        sample.timestamp_ns,
                    ),
                };

                let effective_pressure = match sample.pressure {
                    Some(p) => match self.mode {
                        PipelineMode::Baseline => p,
                        PipelineMode::OneEuroFiltered => {
                            let t_s = (sample.timestamp_ns as f64) / 1_000_000_000.0;
                            self.pressure_filter.filter(p, t_s)
                        }
                    },
                    None => 0.5,
                };

                let width = compute_width(sample.pressure, &self.stroke.style);
                let pt = StrokePoint {
                    x,
                    y,
                    pressure: effective_pressure,
                    width,
                    timestamp_ns: sample.timestamp_ns,
                };

                self.stroke.points.push(pt.clone());
                Some(pt)
            }

            InputPhase::Up => {
                // Finalizzazione del tratto
                self.stroke.is_dot = is_discrete_dot(&self.stroke.points, 2.5);
                None
            }

            InputPhase::Cancel => {
                self.stroke.points.clear();
                None
            }
        }
    }

    /// Conclude il tratto e restituisce la struttura Stroke completa.
    pub fn finish(mut self) -> Stroke {
        self.stroke.is_dot = is_discrete_dot(&self.stroke.points, 2.5);
        self.stroke
    }

    pub fn samples_stats(&self) -> (u64, u64) {
        (self.samples_received, self.samples_dropped)
    }
}
