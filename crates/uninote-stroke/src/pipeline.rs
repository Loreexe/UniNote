use crate::filter::{OneEuroConfig, OneEuroPointFilter, OneEuroScalar};
use crate::geometry::{compute_width, is_discrete_dot};
pub use crate::types::PipelineMode;
use crate::types::{DeviceKind, InputPhase, InputSample, Rect, Stroke, StrokePoint, StrokeStyle};

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
            min_cutoff: 3.0,
            beta: 0.02,
            d_cutoff: 1.0,
        });

        Self {
            stroke: Stroke::new(id, style, mode),
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
                    PipelineMode::Baseline | PipelineMode::CatmullRomSpline => sample.position,
                    PipelineMode::OneEuroFiltered => self.point_filter.filter(
                        sample.position.0,
                        sample.position.1,
                        sample.timestamp_ns,
                    ),
                };

                let effective_pressure = match sample.pressure {
                    Some(p) => match self.mode {
                        PipelineMode::Baseline => p,
                        PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered => {
                            let t_s = (sample.timestamp_ns as f64) / 1_000_000_000.0;
                            self.pressure_filter.filter(p, t_s)
                        }
                    },
                    None => {
                        if sample.device_kind == DeviceKind::Stylus {
                            0.05
                        } else {
                            0.5
                        }
                    }
                };

                let width = compute_width(sample.pressure, &self.stroke.style);
                let pt = StrokePoint {
                    x,
                    y,
                    pressure: effective_pressure,
                    width,
                    timestamp_ns: sample.timestamp_ns,
                };

                let pt_box = Rect::from_point(pt.x, pt.y, pt.width / 2.0);
                self.stroke.cached_bbox = Some(match self.stroke.cached_bbox {
                    Some(existing) => existing.union(&pt_box),
                    None => pt_box,
                });
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

                // Controllo stacco punta: per uno stilo, se la pressione hardware manca (hover)
                // oppure è sotto la soglia di contatto (4.5%), la punta ha staccato dal vetro:
                // scarta il campione per impedire sbavature, code aeree e spike di transizione MPP.
                if sample.device_kind == DeviceKind::Stylus {
                    match sample.pressure {
                        Some(raw_p) if raw_p <= 0.045 => {
                            self.samples_dropped += 1;
                            return None;
                        }
                        None => {
                            self.samples_dropped += 1;
                            return None;
                        }
                        _ => {}
                    }
                } else if let Some(raw_p) = sample.pressure {
                    if raw_p <= 0.015 {
                        self.samples_dropped += 1;
                        return None;
                    }
                }

                let (x, y) = match self.mode {
                    PipelineMode::Baseline | PipelineMode::CatmullRomSpline => sample.position,
                    PipelineMode::OneEuroFiltered => self.point_filter.filter(
                        sample.position.0,
                        sample.position.1,
                        sample.timestamp_ns,
                    ),
                };

                let effective_pressure = match sample.pressure {
                    Some(p) => match self.mode {
                        PipelineMode::Baseline => p,
                        PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered => {
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

                let pt_box = Rect::from_point(pt.x, pt.y, pt.width / 2.0);
                self.stroke.cached_bbox = Some(match self.stroke.cached_bbox {
                    Some(existing) => existing.union(&pt_box),
                    None => pt_box,
                });
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
        // Rifilatura delle code aeree (Tail Trimming): se gli ultimi campioni registrati durante lo stacco
        // avevano pressione debole di hover/rilascio (<= 0.05), vengono rimossi per eliminare qualsiasi sbavatura terminale.
        while self.stroke.points.len() > 1 {
            if let Some(last) = self.stroke.points.last() {
                if last.pressure <= 0.05 {
                    self.stroke.points.pop();
                    continue;
                }
            }
            break;
        }

        self.stroke.is_dot = is_discrete_dot(&self.stroke.points, 2.5);
        self.stroke.recompute_cache();
        self.stroke
    }

    pub fn samples_stats(&self) -> (u64, u64) {
        (self.samples_received, self.samples_dropped)
    }
}
