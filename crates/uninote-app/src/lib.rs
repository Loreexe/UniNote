#![allow(clippy::new_without_default)]

pub mod canvas;
pub mod dashboard;
pub mod hud;
pub mod icons;
pub mod pdf_cache;
pub mod pdf_export;
pub mod preferences;
pub mod settings;
pub mod window;

pub use dashboard::{
    get_or_create_thumbnail_path, render_cairo_thumbnail, thumbnail_cache_dir,
    CreateDocumentDialog, UniNoteDashboard,
};
pub use pdf_cache::{PdfCacheConfig, PdfCacheError, PdfCacheManager, PdfPageKey};
pub use pdf_export::{append_pdf_pages, export_to_pdf, import_from_pdf};
pub use preferences::{
    AppPreferences, BarrelButtonAction, DashboardRecentStore, FullAppSettingsModel, PenSlotConfig,
    PressureCurve, RecentDocumentMeta, ThemeMode,
};
pub use settings::UniNoteSettingsWindow;
pub use window::UniNoteWindow;
