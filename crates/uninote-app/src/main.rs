mod canvas;
mod hud;
mod window;

use libadwaita::prelude::*;
use libadwaita::Application;
use window::UniNoteWindow;

const APP_ID: &str = "org.uninote.UniNote";

fn main() -> glib::ExitCode {
    // Inizializzazione Libadwaita
    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(|app| {
        let win = UniNoteWindow::new(app);
        win.present();
    });

    app.run()
}
