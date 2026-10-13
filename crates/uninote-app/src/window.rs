use crate::canvas::NoteCanvas;
use crate::hud::DiagnosticHud;
use gtk4::prelude::*;
use gtk4::{Button, Overlay, ToggleButton};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, HeaderBar, WindowTitle};
use std::rc::Rc;
use uninote_stroke::{PipelineMode, RecordedSession, TouchMode};

pub struct UniNoteWindow {
    window: ApplicationWindow,
}

impl UniNoteWindow {
    pub fn new(app: &Application) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("UniNote — Fase 0")
            .default_width(1200)
            .default_height(800)
            .build();

        let header_bar = HeaderBar::new();
        let title = WindowTitle::new("UniNote", "Fase 0 — Scrittura & Diagnostica");
        header_bar.set_title_widget(Some(&title));

        let hud = Rc::new(DiagnosticHud::new());
        let canvas = Rc::new(NoteCanvas::new(hud.clone()));

        // Pulsante Cancella foglio
        let btn_clear = Button::builder()
            .tooltip_text("Cancella tutti i tratti sul foglio")
            .icon_name("edit-clear-all-symbolic")
            .build();
        let canvas_clear = canvas.clone();
        btn_clear.connect_clicked(move |_| {
            canvas_clear.state().borrow_mut().clear();
            canvas_clear.widget().queue_draw();
        });
        header_bar.pack_start(&btn_clear);

        // Selettore Pipeline: Baseline vs One Euro
        let toggle_smoothing = ToggleButton::builder()
            .label("Smoothing: One Euro")
            .active(true)
            .tooltip_text(
                "Attiva o disattiva il filtro One Euro (Baseline = nessun filtro e nessun lag)",
            )
            .build();
        let canvas_smooth = canvas.clone();
        let hud_smooth = hud.clone();
        toggle_smoothing.connect_toggled(move |btn| {
            let is_filtered = btn.is_active();
            let new_mode = if is_filtered {
                btn.set_label("Smoothing: One Euro");
                PipelineMode::OneEuroFiltered
            } else {
                btn.set_label("Smoothing: Baseline (Off)");
                PipelineMode::Baseline
            };

            canvas_smooth.state().borrow_mut().pipeline_mode = new_mode;
            hud_smooth.update_metrics(|m| m.pipeline_mode = new_mode);
            hud_smooth.update_view();
        });
        header_bar.pack_start(&toggle_smoothing);

        // Selettore Touch: Off vs Gesti
        let toggle_touch = ToggleButton::builder()
            .label("Touch: OFF")
            .active(false)
            .tooltip_text(
                "Disabilita totalmente il touchscreen sul foglio per massima protezione dal palmo",
            )
            .build();
        let canvas_touch = canvas.clone();
        let hud_touch = hud.clone();
        toggle_touch.connect_toggled(move |btn| {
            let is_gestures = btn.is_active();
            let new_mode = if is_gestures {
                btn.set_label("Touch: Gesti");
                TouchMode::GesturesOnly
            } else {
                btn.set_label("Touch: OFF");
                TouchMode::Off
            };

            canvas_touch
                .state()
                .borrow_mut()
                .palm_engine
                .set_touch_mode(new_mode);
            hud_touch.update_metrics(|m| m.touch_mode = new_mode);
            hud_touch.update_view();
        });
        header_bar.pack_start(&toggle_touch);

        // Selettore Registrazione Input (REC)
        let toggle_rec = ToggleButton::builder()
            .label("● Registra")
            .active(false)
            .tooltip_text("Registra i campioni hardware su file per test deterministici")
            .build();
        let canvas_rec = canvas.clone();
        let hud_rec = hud.clone();
        toggle_rec.connect_toggled(move |btn| {
            let is_recording = btn.is_active();
            let state_rc = canvas_rec.state();
            let mut st = state_rc.borrow_mut();

            if is_recording {
                btn.set_label("■ STOP Rec");
                st.recorder = Some(RecordedSession::new("uninote_live_recording"));
                hud_rec.update_metrics(|m| m.is_recording = true);
            } else {
                btn.set_label("● Registra");
                hud_rec.update_metrics(|m| m.is_recording = false);
                if let Some(rec) = st.recorder.take() {
                    if let Ok(json) = rec.to_json() {
                        let path = "uninote_session_dump.json";
                        if let Err(e) = std::fs::write(path, json) {
                            eprintln!("Errore durante il salvataggio della registrazione: {}", e);
                        } else {
                            println!("Sessione salvata con successo su {}", path);
                        }
                    }
                }
            }
            hud_rec.update_view();
        });
        header_bar.pack_end(&toggle_rec);

        // Selettore visibilità HUD
        let toggle_hud = ToggleButton::builder()
            .label("HUD")
            .active(true)
            .tooltip_text("Mostra o nascondi il pannello diagnostico a schermo")
            .build();
        let hud_widget = hud.widget().clone();
        toggle_hud.connect_toggled(move |btn| {
            hud_widget.set_visible(btn.is_active());
        });
        header_bar.pack_end(&toggle_hud);

        // Contenitore principale con Overlay
        let overlay = Overlay::builder().child(canvas.widget()).build();
        overlay.add_overlay(hud.widget());

        let main_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .build();
        main_box.append(&header_bar);
        main_box.append(&overlay);

        window.set_content(Some(&main_box));

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
