use gtk4::gio;
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow};
use std::path::PathBuf;
use uninote_app::dashboard::UniNoteDashboard;
use uninote_app::preferences::{DashboardRecentStore, RecentDocumentMeta};
use uninote_app::settings::UniNoteSettingsWindow;
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
        let dash_win = ApplicationWindow::builder()
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
    });

    app.connect_open(|app, files, _hint| {
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
