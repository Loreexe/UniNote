use crate::canvas::{ActiveTool, NoteCanvas};
use crate::hud::DiagnosticHud;
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Box as GtkBox, Button, DropDown, EventControllerKey, FileDialog, FileFilter, MenuButton,
    Orientation, Overlay, Popover, ToggleButton,
};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow, HeaderBar, WindowTitle};
use std::rc::Rc;
use uninote_core::{PageBackground, Viewport};
use uninote_stroke::{PipelineMode, RecordedSession, TouchMode};

pub struct UniNoteWindow {
    window: ApplicationWindow,
}

impl UniNoteWindow {
    pub fn new(app: &Application) -> Self {
        let window = ApplicationWindow::builder()
            .application(app)
            .title("UniNote — Quaderno Universitario")
            .default_width(1280)
            .default_height(850)
            .build();

        let header_bar = HeaderBar::new();
        let title = WindowTitle::new("UniNote", "Quaderno Universitario A4");
        header_bar.set_title_widget(Some(&title));

        let hud = Rc::new(DiagnosticHud::new());
        let canvas = Rc::new(NoteCanvas::new(hud.clone()));

        // --- 1. STRUMENTI PRINCIPALI: Penna vs Gomma ---
        let btn_pen = ToggleButton::builder()
            .icon_name("document-edit-symbolic")
            .tooltip_text("Strumento Penna (Scrittura fluida naturale)")
            .active(true)
            .build();

        let btn_eraser = ToggleButton::builder()
            .icon_name("edit-clear-symbolic")
            .tooltip_text("Strumento Gomma (Cancella tratto intero al tocco)")
            .active(false)
            .build();

        let canvas_tool_pen = canvas.clone();
        let btn_eraser_ref = btn_eraser.clone();
        btn_pen.connect_toggled(move |btn| {
            if btn.is_active() {
                btn_eraser_ref.set_active(false);
                canvas_tool_pen.set_active_tool(ActiveTool::Pen);
            }
        });

        let canvas_tool_eraser = canvas.clone();
        let btn_pen_ref = btn_pen.clone();
        btn_eraser.connect_toggled(move |btn| {
            if btn.is_active() {
                btn_pen_ref.set_active(false);
                canvas_tool_eraser.set_active_tool(ActiveTool::Eraser);
            }
        });

        let tools_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["linked"])
            .build();
        tools_box.append(&btn_pen);
        tools_box.append(&btn_eraser);
        header_bar.pack_start(&tools_box);

        // --- 2. PALETTE COLORI RAPIDA ---
        let colors = [
            ("Nero", (0.12, 0.12, 0.14, 1.0)),
            ("Blu", (0.10, 0.35, 0.75, 1.0)),
            ("Rosso", (0.75, 0.12, 0.15, 1.0)),
            ("Verde", (0.15, 0.55, 0.25, 1.0)),
        ];

        let color_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["linked"])
            .build();

        for (name, rgba) in colors {
            let btn_c = Button::builder().tooltip_text(name).label(name).build();
            let canvas_c = canvas.clone();
            btn_c.connect_clicked(move |_| {
                canvas_c.set_pen_color(rgba);
            });
            color_box.append(&btn_c);
        }
        header_bar.pack_start(&color_box);

        // --- 3. SPESSORE PENNA ---
        let width_strings = ["Fine (1.8 pt)", "Medio (2.8 pt)", "Spesso (4.5 pt)"];
        let dropdown_width = DropDown::from_strings(&width_strings);
        dropdown_width.set_selected(1);
        dropdown_width.set_tooltip_text(Some("Spessore base del tratto della penna"));
        let canvas_w = canvas.clone();
        dropdown_width.connect_selected_notify(move |dd| {
            let w = match dd.selected() {
                0 => 1.8,
                1 => 2.8,
                _ => 4.5,
            };
            canvas_w.set_pen_width(w);
        });
        header_bar.pack_start(&dropdown_width);

        // --- 4. UNDO & REDO ---
        let btn_undo = Button::builder()
            .icon_name("edit-undo-symbolic")
            .tooltip_text("Annulla (Ctrl+Z)")
            .build();
        let canvas_undo = canvas.clone();
        btn_undo.connect_clicked(move |_| {
            canvas_undo.undo();
        });

        let btn_redo = Button::builder()
            .icon_name("edit-redo-symbolic")
            .tooltip_text("Ripristina (Ctrl+Y / Ctrl+Shift+Z)")
            .build();
        let canvas_redo = canvas.clone();
        btn_redo.connect_clicked(move |_| {
            canvas_redo.redo();
        });

        let undo_redo_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["linked"])
            .build();
        undo_redo_box.append(&btn_undo);
        undo_redo_box.append(&btn_redo);
        header_bar.pack_start(&undo_redo_box);

        // --- 5. AGGIUNGI PAGINA A4 (+) ---
        let btn_add_page = Button::builder()
            .icon_name("list-add-symbolic")
            .tooltip_text("Aggiungi nuova pagina A4 in coda")
            .build();
        let canvas_add_page = canvas.clone();
        btn_add_page.connect_clicked(move |_| {
            canvas_add_page.add_page(PageBackground::default());
        });
        header_bar.pack_start(&btn_add_page);

        // --- 6. SFONDO PAGINA (Quadretti, Righe, Bianco) ---
        let bg_strings = [
            "Quadretti 5mm (Default)",
            "Righe Universitarie 8mm",
            "Foglio Bianco",
        ];
        let dropdown_bg = DropDown::from_strings(&bg_strings);
        dropdown_bg.set_selected(0);
        dropdown_bg.set_tooltip_text(Some("Template di sfondo per le pagine del quaderno"));
        let canvas_bg = canvas.clone();
        dropdown_bg.connect_selected_notify(move |dd| {
            let new_bg = match dd.selected() {
                1 => PageBackground::Ruled {
                    line_spacing: 22.68,
                    margin_left: 35.0,
                },
                2 => PageBackground::Blank,
                _ => PageBackground::Grid { grid_size: 14.17 },
            };
            let state_rc = canvas_bg.state();
            let mut st = state_rc.borrow_mut();
            if let Some(p) = st.document.pages.first_mut() {
                p.background = new_bg;
            }
            drop(st);
            canvas_bg.widget().queue_draw();
        });
        header_bar.pack_start(&dropdown_bg);

        // --- 7. CONTROLLI ZOOM ( - / 100% / + ) ---
        let btn_zoom_out = Button::builder()
            .icon_name("zoom-out-symbolic")
            .tooltip_text("Riduci zoom")
            .build();
        let canvas_z_out = canvas.clone();
        btn_zoom_out.connect_clicked(move |_| {
            canvas_z_out.zoom_out();
        });

        let btn_zoom_reset = Button::builder()
            .label("100%")
            .tooltip_text("Reimposta zoom a 100% e centra il foglio A4")
            .build();
        let canvas_z_reset = canvas.clone();
        btn_zoom_reset.connect_clicked(move |_| {
            canvas_z_reset.reset_zoom();
        });

        let btn_zoom_in = Button::builder()
            .icon_name("zoom-in-symbolic")
            .tooltip_text("Aumenta zoom")
            .build();
        let canvas_z_in = canvas.clone();
        btn_zoom_in.connect_clicked(move |_| {
            canvas_z_in.zoom_in();
        });

        let zoom_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .css_classes(["linked"])
            .build();
        zoom_box.append(&btn_zoom_out);
        zoom_box.append(&btn_zoom_reset);
        zoom_box.append(&btn_zoom_in);
        header_bar.pack_start(&zoom_box);

        // --- 8. MENU PRIMARIO (Nuovo, Apri, Salva, Salva con nome, Copia Portabile) ---
        let menu_btn = MenuButton::builder()
            .icon_name("open-menu-symbolic")
            .tooltip_text("Menu Documento e Operazioni")
            .build();

        let popover = Popover::new();
        let popover_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .margin_top(6)
            .margin_bottom(6)
            .margin_start(6)
            .margin_end(6)
            .build();

        // Pulsante Nuovo
        let btn_new = Button::builder()
            .label("Nuovo Quaderno")
            .icon_name("document-new-symbolic")
            .build();
        let canvas_new = canvas.clone();
        let popover_new = popover.clone();
        btn_new.connect_clicked(move |_| {
            let state_rc = canvas_new.state();
            let mut st = state_rc.borrow_mut();
            st.document = uninote_core::Document::new_empty("Nuovo Quaderno");
            st.history = uninote_core::CommandHistory::default();
            st.storage_worker = None;
            st.file_path = None;
            st.viewport = Viewport::default();
            drop(st);
            canvas_new.widget().queue_draw();
            popover_new.popdown();
        });
        popover_box.append(&btn_new);

        // Pulsante Apri...
        let btn_open = Button::builder()
            .label("Apri Quaderno...")
            .icon_name("document-open-symbolic")
            .build();
        let canvas_open = canvas.clone();
        let window_open = window.clone();
        let popover_open = popover.clone();
        btn_open.connect_clicked(move |_| {
            popover_open.popdown();
            let dialog = FileDialog::builder()
                .title("Apri Quaderno UniNote")
                .modal(true)
                .build();

            let filter = FileFilter::new();
            filter.set_name(Some("File UniNote (*.uninote)"));
            filter.add_pattern("*.uninote");
            let filters = gio::ListStore::new::<FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let canvas_ref = canvas_open.clone();
            dialog.open(Some(&window_open), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let _ = canvas_ref.open_file(path);
                    }
                }
            });
        });
        popover_box.append(&btn_open);

        // Pulsante Salva con nome...
        let btn_save_as = Button::builder()
            .label("Salva con nome...")
            .icon_name("document-save-as-symbolic")
            .build();
        let canvas_save_as = canvas.clone();
        let window_save_as = window.clone();
        let popover_save_as = popover.clone();
        btn_save_as.connect_clicked(move |_| {
            popover_save_as.popdown();
            let dialog = FileDialog::builder()
                .title("Salva Quaderno UniNote")
                .initial_name("quaderno.uninote")
                .modal(true)
                .build();

            let canvas_ref = canvas_save_as.clone();
            dialog.save(Some(&window_save_as), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let _ = canvas_ref.save_as(path);
                    }
                }
            });
        });
        popover_box.append(&btn_save_as);

        // Pulsante Esporta copia portabile...
        let btn_backup = Button::builder()
            .label("Esporta copia portabile...")
            .tooltip_text("Esporta un database SQLite autoconsistente consolidando il file WAL")
            .icon_name("folder-download-symbolic")
            .build();
        let canvas_backup = canvas.clone();
        let window_backup = window.clone();
        let popover_backup = popover.clone();
        btn_backup.connect_clicked(move |_| {
            popover_backup.popdown();
            let dialog = FileDialog::builder()
                .title("Esporta copia portabile (Online Backup)")
                .initial_name("quaderno_backup.uninote")
                .modal(true)
                .build();

            let canvas_ref = canvas_backup.clone();
            dialog.save(Some(&window_backup), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let _ = canvas_ref.export_portable_copy(path);
                    }
                }
            });
        });
        popover_box.append(&btn_backup);

        // Pulsante Pulisci pagina
        let btn_clear = Button::builder()
            .label("Pulisci pagina corrente")
            .icon_name("edit-clear-all-symbolic")
            .build();
        let canvas_clear = canvas.clone();
        let popover_clear = popover.clone();
        btn_clear.connect_clicked(move |_| {
            canvas_clear.state().borrow_mut().clear_current_page();
            canvas_clear.widget().queue_draw();
            popover_clear.popdown();
        });
        popover_box.append(&btn_clear);

        popover.set_child(Some(&popover_box));
        menu_btn.set_popover(Some(&popover));
        header_bar.pack_end(&menu_btn);

        // --- 9. SELETTORE SOGLIA & PIPELINE (Consolidati dalla Fase 0) ---
        let thresh_strings = [
            "Soglia: 0.06 (Default Anti-Hover)",
            "Soglia: 0.08 (Alta MPP)",
            "Soglia: 0.12 (Massima)",
            "Soglia: 0.04 (Media)",
            "Soglia: 0.02 (Sensibile)",
        ];
        let dropdown_thresh = DropDown::from_strings(&thresh_strings);
        dropdown_thresh.set_selected(0);
        let canvas_thresh = canvas.clone();
        let hud_thresh = hud.clone();
        dropdown_thresh.connect_selected_notify(move |dd| {
            let new_thresh = match dd.selected() {
                0 => 0.06,
                1 => 0.08,
                2 => 0.12,
                3 => 0.04,
                _ => 0.02,
            };
            canvas_thresh.state().borrow_mut().tip_pressure_threshold = new_thresh;
            hud_thresh.update_metrics(|m| m.tip_threshold = new_thresh);
            hud_thresh.update_view();
        });
        header_bar.pack_end(&dropdown_thresh);

        let mode_strings = [
            "Spline Naturale (Default)",
            "One Euro + Spline",
            "Baseline (Off)",
        ];
        let dropdown_mode = DropDown::from_strings(&mode_strings);
        dropdown_mode.set_selected(0);
        let canvas_smooth = canvas.clone();
        let hud_smooth = hud.clone();
        dropdown_mode.connect_selected_notify(move |dd| {
            let new_mode = match dd.selected() {
                0 => PipelineMode::CatmullRomSpline,
                1 => PipelineMode::OneEuroFiltered,
                _ => PipelineMode::Baseline,
            };
            canvas_smooth.state().borrow_mut().pipeline_mode = new_mode;
            hud_smooth.update_metrics(|m| m.pipeline_mode = new_mode);
            hud_smooth.update_view();
            canvas_smooth.widget().queue_draw();
        });
        header_bar.pack_end(&dropdown_mode);

        // --- 10. TOUCH OFF / GESTI ---
        let toggle_touch = ToggleButton::builder()
            .label("Touch: OFF")
            .active(false)
            .tooltip_text("Protezione palmo: Touch OFF vs Gesti a due dita")
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
        header_bar.pack_end(&toggle_touch);

        // --- 11. REGISTRAZIONE INPUT & HUD TOGGLE ---
        let toggle_rec = ToggleButton::builder()
            .label("● Rec")
            .active(false)
            .tooltip_text("Registra i campioni per verifiche e benchmark")
            .build();
        let canvas_rec = canvas.clone();
        let hud_rec = hud.clone();
        toggle_rec.connect_toggled(move |btn| {
            let is_recording = btn.is_active();
            let state_rc = canvas_rec.state();
            let mut st = state_rc.borrow_mut();

            if is_recording {
                btn.set_label("■ Stop");
                st.recorder = Some(RecordedSession::new("uninote_session"));
                hud_rec.update_metrics(|m| m.is_recording = true);
            } else {
                btn.set_label("● Rec");
                hud_rec.update_metrics(|m| m.is_recording = false);
                if let Some(rec) = st.recorder.take() {
                    if let Ok(json) = rec.to_json() {
                        let _ = std::fs::write("uninote_session.json", json);
                    }
                }
            }
            hud_rec.update_view();
        });
        header_bar.pack_end(&toggle_rec);

        let toggle_hud = ToggleButton::builder()
            .label("HUD")
            .active(true)
            .tooltip_text("Mostra o nascondi il pannello diagnostico")
            .build();
        let hud_widget = hud.widget().clone();
        toggle_hud.connect_toggled(move |btn| {
            hud_widget.set_visible(btn.is_active());
        });
        header_bar.pack_end(&toggle_hud);

        // --- 12. SCORCIATOIE DA TASTIERA GLOBALI (Ctrl+Z, Ctrl+Y, Ctrl+S, Ctrl+O, Ctrl+N) ---
        let key_controller = EventControllerKey::new();
        let canvas_keys = canvas.clone();
        let window_keys = window.clone();
        key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, modifier| {
            let is_ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let is_shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);

            if is_ctrl {
                match keyval {
                    gdk::Key::z | gdk::Key::Z => {
                        if is_shift {
                            canvas_keys.redo();
                        } else {
                            canvas_keys.undo();
                        }
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::y | gdk::Key::Y => {
                        canvas_keys.redo();
                        return glib::Propagation::Stop;
                    }
                    gdk::Key::s | gdk::Key::S => {
                        let state_rc = canvas_keys.state();
                        let st = state_rc.borrow();
                        if let Some(ref path) = st.file_path {
                            let path_clone = path.clone();
                            drop(st);
                            let _ = canvas_keys.save_as(path_clone);
                        } else {
                            drop(st);
                            let dialog = FileDialog::builder()
                                .title("Salva Quaderno UniNote")
                                .initial_name("quaderno.uninote")
                                .modal(true)
                                .build();
                            let c_ref = canvas_keys.clone();
                            dialog.save(Some(&window_keys), gio::Cancellable::NONE, move |res| {
                                if let Ok(file) = res {
                                    if let Some(path) = file.path() {
                                        let _ = c_ref.save_as(path);
                                    }
                                }
                            });
                        }
                        return glib::Propagation::Stop;
                    }
                    _ => {}
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);

        // Contenitore principale con Overlay
        let overlay = Overlay::builder().child(canvas.widget()).build();
        overlay.add_overlay(hud.widget());

        let main_box = GtkBox::builder().orientation(Orientation::Vertical).build();
        main_box.append(&header_bar);
        main_box.append(&overlay);

        window.set_content(Some(&main_box));

        Self { window }
    }

    pub fn present(&self) {
        self.window.present();
    }
}
