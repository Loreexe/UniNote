//! Modulo Core di UniNote: modello del documento, pagine, coordinate e comandi.

pub mod command;
pub mod coordinates;
pub mod model;

pub use command::{CommandHistory, DocCommand};
pub use coordinates::{Point2D, Rect2D, Viewport, PAGE_GAP_PT, PAGE_TOP_MARGIN_PT};
pub use model::{
    Document, DocumentFormat, Item, ItemId, ItemKind, Page, PageBackground, A4_HEIGHT_PT,
    A4_WIDTH_PT,
};
