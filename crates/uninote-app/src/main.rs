use gtk4::gio;
use libadwaita::prelude::*;
use libadwaita::Application;
use uninote_app::preferences::{DashboardRecentStore, RecentDocumentMeta};
use uninote_app::UniNoteWindow;
use uninote_core::model::DocumentFormat;

const APP_ID: &str = "org.uninote.UniNote";

fn main() -> glib::ExitCode {
    // Inizializza e verifica la cartella predefinita per gli appunti ($XDG_DOCUMENTS_DIR/UniNote)
    let recent_store = DashboardRecentStore::load();
    let _ = recent_store.ensure_default_notes_dir();

    // Supporta sia l'avvio standard (Dashboard) che l'avvio con argomenti file (apertura diretta documento)
    let app = Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    app.connect_activate(|app| {
        gtk4::Window::set_default_icon_name(APP_ID);
        uninote_app::init_application_style();

        // Applica modalità tema salvata
        let settings = uninote_app::preferences::FullAppSettingsModel::load();
        let sm = libadwaita::StyleManager::default();
        match settings.theme_mode {
            uninote_app::preferences::ThemeMode::System => {
                sm.set_color_scheme(libadwaita::ColorScheme::Default)
            }
            uninote_app::preferences::ThemeMode::ForceLight => {
                sm.set_color_scheme(libadwaita::ColorScheme::ForceLight)
            }
            uninote_app::preferences::ThemeMode::ForceDarkOled => {
                sm.set_color_scheme(libadwaita::ColorScheme::ForceDark)
            }
        }

        // Presenta la finestra principale persistente (la quale include dashboard.connect_import_pdf internamente)
        let win = UniNoteWindow::new(app);
        win.present();
    });

    app.connect_open(|app, files, _hint| {
        gtk4::Window::set_default_icon_name(APP_ID);
        if let Some(file) = files.first() {
            if let Some(path) = file.path() {
                let mut store = DashboardRecentStore::load();
                let title = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Documento")
                    .to_string();

                let meta = RecentDocumentMeta {
                    file_path: path.to_string_lossy().to_string(),
                    title,
                    format: DocumentFormat::Notebook,
                    last_modified_timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    page_count: 1,
                    thumbnail_path: None,
                };
                store.add_recent(meta);
                let _ = store.save();

                let win = UniNoteWindow::open_file(app, &path);
                win.present();
                return;
            }
        }
        let win = UniNoteWindow::new(app);
        win.present();
    });

    app.run()
}
