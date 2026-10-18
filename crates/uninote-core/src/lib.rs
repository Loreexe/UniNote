//! Modulo Core di UniNote: modello del documento, pagine, coordinate e comandi.

pub mod command;
pub mod coordinates;
pub mod dataset;
pub mod model;
pub mod spatial;

pub use command::{CommandHistory, DocCommand};
pub use coordinates::{Point2D, Rect2D, Viewport, PAGE_GAP_PT, PAGE_TOP_MARGIN_PT};
pub use dataset::{
    AcademicDatasetGenerator, AcademicScale, DeterministicPrng, DEFAULT_ACADEMIC_SEED,
};
pub use model::{
    Document, DocumentFormat, Item, ItemId, ItemKind, Page, PageBackground, A4_HEIGHT_PT,
    A4_WIDTH_PT,
};
pub use spatial::{PageSpatialIndex, SpatialItem};
