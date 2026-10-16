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

/// Crea o presenta la finestra Dashboard dell'applicazione UniNote.
pub fn main_open_dashboard(app: &libadwaita::Application) {
    use gtk4::prelude::*;
    use libadwaita::prelude::*;
    use std::path::PathBuf;

    let dash_win = libadwaita::ApplicationWindow::builder()
        .application(app)
        .title("UniNote — Hub Accademico")
        .default_width(1180)
        .default_height(800)
        .css_classes(["uninote-window", "uninote-dashboard-window"])
        .build();

    let dashboard = UniNoteDashboard::new();
    dash_win.set_content(Some(dashboard.widget()));

    let dash_win_open = dash_win.clone();
    let app_open = app.clone();
    dashboard.connect_open_document(move |path: PathBuf| {
        let win = UniNoteWindow::open_file(&app_open, &path);
        win.present();
        dash_win_open.close();
    });

    let dash_win_create = dash_win.clone();
    let app_create = app.clone();
    dashboard.connect_create_document(move |title, format, bg| {
        let win = UniNoteWindow::create_document(&app_create, &title, format, bg);
        win.present();
        dash_win_create.close();
    });

    let dash_win_import = dash_win.clone();
    let app_import = app.clone();
    dashboard.connect_import_pdf(move |path: PathBuf| {
        match UniNoteWindow::create_from_pdf(&app_import, &path) {
            Ok(win) => {
                win.present();
                dash_win_import.close();
            }
            Err(e) => {
                eprintln!(
                    "Errore durante l'apertura del PDF {}: {}",
                    path.display(),
                    e
                );
            }
        }
    });

    dashboard.connect_open_settings(move || {
        let settings_win = UniNoteSettingsWindow::new();
        settings_win.present();
    });

    dash_win.present();
}
