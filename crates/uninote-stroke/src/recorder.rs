use crate::pipeline::{PipelineMode, StrokeBuilder};
use crate::types::{InputPhase, InputSample, Stroke, StrokeStyle};
use serde::{Deserialize, Serialize};

/// Sessione registrata di campioni di input grezzi per riproduzione e test deterministici.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordedSession {
    pub name: String,
    pub samples: Vec<InputSample>,
}

impl RecordedSession {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            samples: Vec::new(),
        }
    }

    pub fn record_sample(&mut self, sample: InputSample) {
        self.samples.push(sample);
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json_str: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_str)
    }

    /// Riproduce l'intera sessione attraverso la pipeline con una modalità specificata.
    pub fn replay(&self, mode: PipelineMode, style: StrokeStyle) -> Vec<Stroke> {
        let mut strokes = Vec::new();
        let mut current_builder: Option<StrokeBuilder> = None;
        let mut stroke_id = 1u64;

        for sample in &self.samples {
            match sample.phase {
                InputPhase::Down => {
                    let mut builder = StrokeBuilder::new(stroke_id, style.clone(), mode);
                    stroke_id += 1;
                    builder.process_sample(sample.clone());
                    current_builder = Some(builder);
                }
                InputPhase::Move => {
                    if let Some(ref mut builder) = current_builder {
                        builder.process_sample(sample.clone());
                    }
                }
                InputPhase::Up => {
                    if let Some(mut builder) = current_builder.take() {
                        builder.process_sample(sample.clone());
                        strokes.push(builder.finish());
                    }
                }
                InputPhase::Cancel => {
                    current_builder = None;
                }
            }
        }

        if let Some(builder) = current_builder {
            strokes.push(builder.finish());
        }

        strokes
    }
}
