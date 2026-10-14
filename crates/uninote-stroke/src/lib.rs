pub mod filter;
pub mod geometry;
pub mod palm;
pub mod pipeline;
pub mod recorder;
pub mod shape;
pub mod types;

pub use filter::{OneEuroConfig, OneEuroPointFilter, OneEuroScalar};
pub use geometry::{
    catmull_rom_to_bezier, compute_width, distance, is_discrete_dot, sample_catmull_rom_spline,
    CubicBezierSegment,
};
pub use palm::{InputDisposition, PalmRejectionEngine, PalmState, TouchMode, PROXIMITY_TIMEOUT_NS};
pub use pipeline::StrokeBuilder;
pub use recorder::RecordedSession;
pub use shape::{recognize_shape, snap_line_orthogonal};
pub use types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle,
};
