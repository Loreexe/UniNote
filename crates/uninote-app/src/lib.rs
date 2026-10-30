#![allow(clippy::new_without_default)]

pub mod canvas;
pub mod clipboard;
pub mod cloud;
pub mod dashboard;
pub mod hud;
pub mod icons;
pub mod pdf_cache;
pub mod pdf_export;
pub mod preferences;
pub mod selection;
pub mod settings;
pub mod window;

pub use cloud::{
    CloudServiceType, CloudSyncSettings, ConnectCloudDialog, GoogleDriveAccount, NextcloudAccount,
};

pub use clipboard::{
    clear_clipboard, deserialize_clipboard, get_app_clipboard, get_clipboard_data,
    init_system_clipboard_sync, serialize_clipboard, set_clipboard_data, ClipboardData,
    APP_CLIPBOARD, MIME_UNINOTE_SELECTION,
};

pub use dashboard::{
    get_or_create_thumbnail_path, render_cairo_thumbnail, thumbnail_cache_dir,
    CreateDocumentDialog, UniNoteDashboard,
};
pub use pdf_cache::{PdfCacheConfig, PdfCacheError, PdfCacheManager, PdfPageKey};
pub use pdf_export::{
    append_pdf_pages, export_to_pdf, export_to_pdf_with_options, import_from_pdf, PdfExportOptions,
};
pub use preferences::{
    AppPreferences, BarrelButtonAction, DashboardRecentStore, FullAppSettingsModel, PenSlotConfig,
    PressureCurve, RecentDocumentMeta, StylusSensitivity, ThemeMode,
};
pub use selection::AppSelectionEngine;
pub use settings::{
    UniNoteSettingsWindow, APP_VERSION, APP_VERSION_LABEL, GITHUB_NEW_ISSUE_URL,
    GITHUB_RELEASES_URL, GITHUB_REPO_URL, GPL_3_0_LICENSE_URL,
};
pub use window::UniNoteWindow;

/// Inizializza il provider CSS globale per tutte le finestre dell'applicazione.
pub fn init_application_style() {
    let css_provider = gtk4::CssProvider::new();
    css_provider.load_from_data(include_str!("style.css"));
    if let Some(display) = gtk4::gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

/// Crea o presenta la finestra principale con Dashboard dell'applicazione UniNote.
pub fn main_open_dashboard(app: &libadwaita::Application) {
    let win = UniNoteWindow::new(app);
    win.present();
}
