use crate::canvas::{ActiveTool, EraserMode, NoteCanvas};
use crate::hud::DiagnosticHud;
use crate::icons::{create_icon, IconKind};
use crate::preferences::{AppPreferences, PenSlotConfig};
use gtk4::cairo;
use gtk4::gdk;
use gtk4::gio;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, ColorDialog, DrawingArea, DropDown, EventControllerKey,
    FileDialog, GestureClick, GestureDrag, GestureLongPress, Label, Orientation, Overlay, Popover,
    PositionType, Scale,
};
use libadwaita::prelude::*;
use libadwaita::{Application, ApplicationWindow};
use std::cell::{Cell, RefCell};
use std::f64::consts::PI;
use std::rc::Rc;
use uninote_core::{DocumentFormat, PageBackground};
use uninote_stroke::{PipelineMode, TouchMode};

pub struct UniNoteWindow {
    window: ApplicationWindow,
    canvas: Rc<NoteCanvas>,
    notebook_dock_box: GtkBox,
    whiteboard_dock_box: GtkBox,
    stack: gtk4::Stack,
    dashboard: Rc<crate::dashboard::UniNoteDashboard>,
}

impl UniNoteWindow {
    pub fn new(app: &Application) -> Self {
        // 1. Inizializzazione Stile CSS personalizzato DMS / Material You
        let css_provider = gtk4::CssProvider::new();
        css_provider.load_from_data(include_str!("style.css"));
        if let Some(display) = gdk::Display::default() {
            gtk4::style_context_add_provider_for_display(
                &display,
                &css_provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }

        // Inizializza sincronizzazione clipboard di sistema (GdkClipboard IPC)
        crate::clipboard::init_system_clipboard_sync();

        // Finestra principale senza decorazioni rigide di sistema
        let window = ApplicationWindow::builder()
            .application(app)
            .title("UniNote — Hub Accademico")
            .default_width(1320)
            .default_height(860)
            .css_classes(["uninote-window"])
            .build();

        let hud = Rc::new(DiagnosticHud::new());
        hud.widget().set_visible(false); // Nascosto di default, attivabile con F12 o da Settings

        let canvas = Rc::new(NoteCanvas::new(hud.clone()));
        let prefs = Rc::new(RefCell::new(AppPreferences::load()));

        let dashboard = Rc::new(crate::dashboard::UniNoteDashboard::new());
        let stack = gtk4::Stack::builder()
            .transition_type(gtk4::StackTransitionType::Crossfade)
            .transition_duration(150)
            .build();

        // Applica preferenze persistite al canvas all'avvio
        let initial_stroke_w = prefs.borrow().stroke_width;
        canvas.set_pen_width(initial_stroke_w);
        let initial_eraser_mode = prefs.borrow().eraser_mode;
        canvas.set_eraser_mode(initial_eraser_mode);
        let initial_eraser_radius = prefs.borrow().eraser_radius;
        canvas.set_eraser_radius(initial_eraser_radius);

        let initial_slot_idx = prefs.borrow().active_slot_index;
        let initial_hex = if let Some(slot) = prefs.borrow().slots.get(initial_slot_idx) {
            canvas.set_pen_color(slot.rgba);
            slot.hex.clone()
        } else {
            "#6DD2FF".to_string()
        };

        // Contenitore Overlay a tutto schermo (Full-Bleed)
        let overlay = Overlay::builder().child(canvas.widget()).build();

        // =========================================================================
        // 1. BRAND BADGE IN ALTO A SINISTRA (x: 22, y: 16)
        // =========================================================================
        let badge_btn = Button::builder()
            .valign(Align::Start)
            .halign(Align::Start)
            .margin_start(22)
            .margin_top(16)
            .tooltip_text("Torna alla Dashboard")
            .css_classes(["dms-brand-badge"])
            .build();

        let badge_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();

        let app_logo = create_icon(IconKind::AppLogo, 35, None);
        badge_box.append(&app_logo);

        let title_label = Label::builder()
            .label("UniNote")
            .valign(Align::Center)
            .css_classes(["brand-title"])
            .build();
        badge_box.append(&title_label);

        let dot_label = Label::builder()
            .label("•")
            .valign(Align::Center)
            .css_classes(["brand-dot"])
            .build();
        badge_box.append(&dot_label);

        let ver_label = Label::builder()
            .label(concat!("v", env!("CARGO_PKG_VERSION")))
            .valign(Align::Center)
            .css_classes(["brand-ver"])
            .build();
        badge_box.append(&ver_label);

        badge_btn.set_child(Some(&badge_box));

        let win_nav = window.clone();
        let c_nav = canvas.clone();
        let d_nav = dashboard.clone();
        let s_nav = stack.clone();
        badge_btn.connect_clicked(move |_| {
            let doc_title = c_nav.document_title();
            let file_path = c_nav.file_path();
            let is_auto_named = doc_title.starts_with("Nuova nota");

            if is_auto_named {
                let win_for_dlg = win_nav.clone();
                let c_for_dlg = c_nav.clone();
                let d_for_dlg = d_nav.clone();
                let s_for_dlg = s_nav.clone();
                let path_to_remove = file_path.clone();

                let dlg = gtk4::Window::builder()
                    .title("Salvare il documento? — UniNote")
                    .transient_for(&win_for_dlg)
                    .modal(true)
                    .default_width(440)
                    .default_height(220)
                    .css_classes(["uninote-dialog", "dms-dialog"])
                    .build();

                let box_content = GtkBox::builder()
                    .orientation(Orientation::Vertical)
                    .spacing(16)
                    .margin_start(24)
                    .margin_end(24)
                    .margin_top(20)
                    .margin_bottom(20)
                    .build();

                let lbl_head = Label::builder()
                    .label("Salvare il documento?")
                    .xalign(0.0)
                    .css_classes(["title-2", "brand-title"])
                    .build();

                let lbl_body = Label::builder()
                    .label(format!(
                        "Il documento \"{}\" non ha un nome personalizzato.\nVuoi conservarlo nella cartella delle note?",
                        doc_title
                    ))
                    .xalign(0.0)
                    .wrap(true)
                    .css_classes(["dim-label"])
                    .build();

                let act_box = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(12)
                    .halign(Align::End)
                    .margin_top(12)
                    .build();

                let btn_discard = Button::builder()
                    .label("Elimina")
                    .css_classes(["destructive-action", "dms-action-btn"])
                    .build();

                let btn_save = Button::builder()
                    .label("Conserva")
                    .css_classes(["suggested-action", "dms-btn-primary"])
                    .build();

                act_box.append(&btn_discard);
                act_box.append(&btn_save);

                box_content.append(&lbl_head);
                box_content.append(&lbl_body);
                box_content.append(&act_box);
                dlg.set_child(Some(&box_content));

                let dlg_d = dlg.clone();
                let path_d = path_to_remove.clone();
                let d_d = d_for_dlg.clone();
                let s_d = s_for_dlg.clone();
                let w_d = win_for_dlg.clone();
                btn_discard.connect_clicked(move |_| {
                    if let Some(ref p) = path_d {
                        let _ = std::fs::remove_file(p);
                        d_d.store().borrow_mut().sanitize_existing_files();
                        let _ = d_d.store().borrow().save();
                    }
                    dlg_d.close();
                    d_d.refresh();
                    w_d.set_title(Some("UniNote — Hub Accademico"));
                    s_d.set_visible_child_name("dashboard");
                });

                let dlg_s = dlg.clone();
                let c_s = c_for_dlg.clone();
                let d_s = d_for_dlg.clone();
                let s_s = s_for_dlg.clone();
                let w_s = win_for_dlg.clone();
                btn_save.connect_clicked(move |_| {
                    c_s.persist_changes();
                    dlg_s.close();
                    d_s.refresh();
                    w_s.set_title(Some("UniNote — Hub Accademico"));
                    s_s.set_visible_child_name("dashboard");
                });

                dlg.present();
            } else {
                c_nav.persist_changes();
                d_nav.refresh();
                win_nav.set_title(Some("UniNote — Hub Accademico"));
                s_nav.set_visible_child_name("dashboard");
            }
        });

        overlay.add_overlay(&badge_btn);

        // =========================================================================
        // 2. ISOLA CENTRALE STRUMENTI (Top-Center, y: 18, larghezza 447px)
        // =========================================================================
        let island_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .valign(Align::Start)
            .halign(Align::Center)
            .margin_top(18)
            .css_classes(["dms-pill"])
            .build();

        // --- Undo / Redo ---
        let btn_undo = Button::builder()
            .child(&create_icon(IconKind::Undo, 16, None))
            .tooltip_text("Annulla (Ctrl+Z)")
            .css_classes(["dms-tool-btn"])
            .build();
        let c_undo = canvas.clone();
        btn_undo.connect_clicked(move |_| {
            c_undo.undo();
        });
        island_box.append(&btn_undo);

        let btn_redo = Button::builder()
            .child(&create_icon(IconKind::Redo, 16, None))
            .tooltip_text("Ripeti (Ctrl+Y)")
            .css_classes(["dms-tool-btn"])
            .build();
        let c_redo = canvas.clone();
        btn_redo.connect_clicked(move |_| {
            c_redo.redo();
        });
        island_box.append(&btn_redo);

        // Separatore 1
        let sep1 = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        island_box.append(&sep1);

        // --- Chip Selettore Spessore (Calibro 2.8 ▾) ---
        let thick_chip_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();

        let caliber_dot = Label::builder()
            .use_markup(true)
            .label(format!(
                "<span foreground='{}' font_desc='11'>●</span>",
                initial_hex
            ))
            .build();
        thick_chip_box.append(&caliber_dot);

        let caliber_val = Label::builder()
            .label(format!("{:.1}", initial_stroke_w))
            .css_classes(["thick-value"])
            .build();
        thick_chip_box.append(&caliber_val);

        let caliber_down = create_icon(IconKind::ChevronDown, 10, None);
        thick_chip_box.append(&caliber_down);

        let btn_thick_chip = Button::builder()
            .child(&thick_chip_box)
            .tooltip_text("Regola Spessore Tratto")
            .css_classes(["dms-thick-chip"])
            .build();

        let thick_popover = Popover::builder().css_classes(["dms-popover"]).build();
        thick_popover.set_parent(&btn_thick_chip);

        let thick_pop_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .margin_start(10)
            .margin_end(10)
            .margin_top(8)
            .margin_bottom(8)
            .build();

        let thick_pop_title = Label::builder()
            .label("Calibro Tratto (pt)")
            .xalign(0.0)
            .css_classes(["brand-title"])
            .build();
        thick_pop_box.append(&thick_pop_title);

        // Anteprima Cairo live del tratto
        let stroke_preview = DrawingArea::builder()
            .content_height(40)
            .content_width(220)
            .css_classes(["dms-stroke-preview-area"])
            .build();

        let thick_scale = Scale::with_range(Orientation::Horizontal, 0.25, 12.0, 0.05);
        thick_scale.set_value(initial_stroke_w);
        thick_scale.set_width_request(220);
        thick_scale.set_css_classes(&["dms-pill-scale"]);

        let canvas_prev = canvas.clone();
        let scale_prev = thick_scale.clone();
        stroke_preview.set_draw_func(move |_area, cr, width, height| {
            let w = width as f64;
            let h = height as f64;
            let stroke_w = scale_prev.value();
            let (r, g, b, a) = canvas_prev.state().borrow().current_style.color;

            cr.set_source_rgba(r, g, b, a);
            cr.set_line_cap(cairo::LineCap::Round);
            cr.set_line_width(stroke_w);
            let pad_x = 28.0;
            let y = h / 2.0;
            cr.move_to(pad_x, y);
            cr.line_to(w - pad_x, y);
            let _ = cr.stroke();
        });
        thick_pop_box.append(&stroke_preview);
        thick_pop_box.append(&thick_scale);

        // 6 Chip semantici di preset rapido
        let presets_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .build();

        let preset_chips_data: [(&str, f64); 6] = [
            ("Micro", 0.3),
            ("Fine", 1.0),
            ("Normale", 2.0),
            ("Medio", 2.8),
            ("Spesso", 4.0),
            ("Calligrafico", 6.0),
        ];

        let preset_buttons: Rc<RefCell<Vec<(Button, f64)>>> = Rc::new(RefCell::new(Vec::new()));

        for (name, preset_val) in preset_chips_data {
            let p_btn = Button::builder()
                .label(name)
                .tooltip_text(format!("{:.1} pt", preset_val))
                .css_classes(["dms-preset-chip"])
                .build();
            if (initial_stroke_w - preset_val).abs() < 0.15 {
                p_btn.add_css_class("active");
            }
            let s_ref = thick_scale.clone();
            p_btn.connect_clicked(move |_| {
                s_ref.set_value(preset_val);
            });
            presets_box.append(&p_btn);
            preset_buttons.borrow_mut().push((p_btn, preset_val));
        }
        thick_pop_box.append(&presets_box);

        let c_scale = canvas.clone();
        let val_lbl_clone = caliber_val.clone();
        let stroke_prev_clone = stroke_preview.clone();
        let preset_btns_clone = preset_buttons.clone();
        let prefs_scale = prefs.clone();

        thick_scale.connect_value_changed(move |scale| {
            let val = scale.value();
            c_scale.set_pen_width(val);
            val_lbl_clone.set_label(&format!("{:.1}", val));
            stroke_prev_clone.queue_draw();

            for (btn, preset_val) in preset_btns_clone.borrow().iter() {
                if (val - preset_val).abs() < 0.15 {
                    btn.add_css_class("active");
                } else {
                    btn.remove_css_class("active");
                }
            }

            prefs_scale.borrow_mut().stroke_width = val;
            let _ = prefs_scale.borrow().save();
        });

        thick_popover.set_child(Some(&thick_pop_box));

        let t_pop_ref = thick_popover.clone();
        btn_thick_chip.connect_clicked(move |_| {
            t_pop_ref.popup();
        });
        island_box.append(&btn_thick_chip);

        // Separatore 2
        let sep2 = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        island_box.append(&sep2);

        // --- Gruppo Strumenti di Disegno ---
        let tools: [(IconKind, ActiveTool, &str); 7] = [
            (IconKind::PenTool, ActiveTool::Pen, "Penna Stilografica"),
            (
                IconKind::Eraser,
                ActiveTool::Eraser,
                "Gomma (Tratto Intero)",
            ),
            (
                IconKind::Highlighter,
                ActiveTool::Highlighter,
                "Evidenziatore Fluido",
            ),
            (
                IconKind::Select,
                ActiveTool::Select,
                "Selezione (Lazo / Rettangolo)",
            ),
            (
                IconKind::Type,
                ActiveTool::Text,
                "Testo & Formule (Markdown/LaTeX)",
            ),
            (IconKind::Line, ActiveTool::Line, "Linea Dritta"),
            (IconKind::Shapes, ActiveTool::Shapes, "Riconoscimento Forme"),
        ];

        let tool_buttons: Rc<RefCell<Vec<Button>>> = Rc::new(RefCell::new(Vec::new()));
        let eraser_popover = Popover::builder().css_classes(["dms-popover"]).build();
        let mut eraser_btn_opt: Option<Button> = None;

        for (i, (icon_kind, tool_kind, tooltip)) in tools.into_iter().enumerate() {
            let initial_tip = if tool_kind == ActiveTool::Eraser {
                match initial_eraser_mode {
                    EraserMode::WholeStroke => "Gomma (Tratto Intero)",
                    EraserMode::Segment => "Gomma (Per Segmento)",
                }
            } else {
                tooltip
            };

            let btn = Button::builder()
                .child(&create_icon(icon_kind, 16, None))
                .tooltip_text(initial_tip)
                .css_classes(["dms-tool-btn"])
                .build();

            if tool_kind == ActiveTool::Eraser {
                eraser_btn_opt = Some(btn.clone());
            }

            if i == 0 {
                btn.add_css_class("active");
            }

            let c_tool = canvas.clone();
            let all_tools = tool_buttons.clone();
            let btn_self = btn.clone();
            let pop_click = eraser_popover.clone();

            btn.connect_clicked(move |_| {
                let is_already_active = btn_self.has_css_class("active");
                for b in all_tools.borrow().iter() {
                    b.remove_css_class("active");
                }
                btn_self.add_css_class("active");
                if tool_kind == ActiveTool::Select && is_already_active {
                    let cur_mode = c_tool.selection_mode();
                    let new_mode = match cur_mode {
                        uninote_stroke::selection::SelectionMode::Lasso => {
                            uninote_stroke::selection::SelectionMode::Rectangle
                        }
                        uninote_stroke::selection::SelectionMode::Rectangle => {
                            uninote_stroke::selection::SelectionMode::Lasso
                        }
                    };
                    c_tool.set_selection_mode(new_mode);
                } else if tool_kind == ActiveTool::Eraser && is_already_active {
                    pop_click.popup();
                } else {
                    c_tool.set_active_tool(tool_kind);
                }
            });

            island_box.append(&btn);
            tool_buttons.borrow_mut().push(btn);
        }

        let eraser_btn = eraser_btn_opt.expect("eraser button must exist");
        eraser_popover.set_parent(&eraser_btn);

        // Controller gesture click destro e long-press su eraser_btn
        let right_click = GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        let pop_rc = eraser_popover.clone();
        let tools_rc = tool_buttons.clone();
        let btn_rc = eraser_btn.clone();
        let canvas_rc = canvas.clone();
        right_click.connect_pressed(move |gesture, _n, _x, _y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            for b in tools_rc.borrow().iter() {
                b.remove_css_class("active");
            }
            btn_rc.add_css_class("active");
            canvas_rc.set_active_tool(ActiveTool::Eraser);
            pop_rc.popup();
        });
        eraser_btn.add_controller(right_click);

        let long_press = GestureLongPress::new();
        long_press.set_touch_only(false);
        let pop_lp = eraser_popover.clone();
        let tools_lp = tool_buttons.clone();
        let btn_lp = eraser_btn.clone();
        let canvas_lp = canvas.clone();
        long_press.connect_pressed(move |_gesture, _x, _y| {
            for b in tools_lp.borrow().iter() {
                b.remove_css_class("active");
            }
            btn_lp.add_css_class("active");
            canvas_lp.set_active_tool(ActiveTool::Eraser);
            pop_lp.popup();
        });
        eraser_btn.add_controller(long_press);

        // Popolamento contenuti del Popover Gomma
        let eraser_pop_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .margin_start(10)
            .margin_end(10)
            .margin_top(8)
            .margin_bottom(8)
            .build();

        // Header con titolo e valore numerico raggio
        let title_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .build();
        let lbl_eraser_title = Label::builder()
            .label("Strumento Gomma")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["brand-title"])
            .build();
        title_box.append(&lbl_eraser_title);

        let lbl_radius_val = Label::builder()
            .label(format!("{:.0} px", initial_eraser_radius))
            .xalign(1.0)
            .css_classes(["thick-value"])
            .build();
        title_box.append(&lbl_radius_val);
        eraser_pop_box.append(&title_box);

        // Selettore a pillola modalità (WholeStroke vs Segment)
        let mode_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .homogeneous(true)
            .css_classes(["dms-theme-slider"])
            .build();

        let btn_mode_whole = Button::builder()
            .label("Tratto Intero")
            .tooltip_text("Cancella l'intero tratto Bézier toccato")
            .css_classes(["dms-theme-btn"])
            .build();

        let btn_mode_segment = Button::builder()
            .label("Per Segmento")
            .tooltip_text("Taglia chirurgicamente il tratto nel punto toccato")
            .css_classes(["dms-theme-btn"])
            .build();

        let update_mode_ui = {
            let b_whole = btn_mode_whole.clone();
            let b_seg = btn_mode_segment.clone();
            let btn_eraser_ref = eraser_btn.clone();
            move |mode: EraserMode| {
                b_whole.remove_css_class("active");
                b_seg.remove_css_class("active");
                match mode {
                    EraserMode::WholeStroke => {
                        b_whole.add_css_class("active");
                        btn_eraser_ref.set_tooltip_text(Some("Gomma (Tratto Intero)"));
                    }
                    EraserMode::Segment => {
                        b_seg.add_css_class("active");
                        btn_eraser_ref.set_tooltip_text(Some("Gomma (Per Segmento)"));
                    }
                }
            }
        };
        update_mode_ui(initial_eraser_mode);

        let c_mode_w = canvas.clone();
        let p_mode_w = prefs.clone();
        let u_mode_w = update_mode_ui.clone();
        btn_mode_whole.connect_clicked(move |_| {
            c_mode_w.set_eraser_mode(EraserMode::WholeStroke);
            u_mode_w(EraserMode::WholeStroke);
            p_mode_w.borrow_mut().eraser_mode = EraserMode::WholeStroke;
            let _ = p_mode_w.borrow().save();
        });

        let c_mode_s = canvas.clone();
        let p_mode_s = prefs.clone();
        let u_mode_s = update_mode_ui.clone();
        btn_mode_segment.connect_clicked(move |_| {
            c_mode_s.set_eraser_mode(EraserMode::Segment);
            u_mode_s(EraserMode::Segment);
            p_mode_s.borrow_mut().eraser_mode = EraserMode::Segment;
            let _ = p_mode_s.borrow().save();
        });

        mode_box.append(&btn_mode_whole);
        mode_box.append(&btn_mode_segment);
        eraser_pop_box.append(&mode_box);

        // Etichetta sezione raggio
        let lbl_radius_section = Label::builder()
            .label("RAGGIO D'AZIONE")
            .xalign(0.0)
            .css_classes(["dms-popover-section-title"])
            .build();
        eraser_pop_box.append(&lbl_radius_section);

        // Slider raggio (4.0..=64.0 px)
        let radius_scale = Scale::with_range(Orientation::Horizontal, 4.0, 64.0, 1.0);
        radius_scale.set_value(initial_eraser_radius);
        radius_scale.set_width_request(220);
        radius_scale.set_css_classes(&["dms-pill-scale"]);
        eraser_pop_box.append(&radius_scale);

        // Chip preset rapidi (Fine 8px, Medio 16px, Grande 32px)
        let presets_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .homogeneous(true)
            .build();

        let presets_data: [(&str, f64); 3] = [("Fine", 8.0), ("Medio", 16.0), ("Grande", 32.0)];

        let preset_buttons: Rc<RefCell<Vec<(Button, f64)>>> = Rc::new(RefCell::new(Vec::new()));

        for (name, preset_val) in presets_data {
            let p_btn = Button::builder()
                .label(name)
                .tooltip_text(format!("{:.0} px", preset_val))
                .css_classes(["dms-preset-chip"])
                .build();
            if (initial_eraser_radius - preset_val).abs() < 0.5 {
                p_btn.add_css_class("active");
            }
            let s_ref = radius_scale.clone();
            p_btn.connect_clicked(move |_| {
                s_ref.set_value(preset_val);
            });
            presets_box.append(&p_btn);
            preset_buttons.borrow_mut().push((p_btn, preset_val));
        }
        eraser_pop_box.append(&presets_box);

        let c_scale = canvas.clone();
        let val_lbl_clone = lbl_radius_val.clone();
        let preset_btns_clone = preset_buttons.clone();
        let prefs_scale = prefs.clone();

        radius_scale.connect_value_changed(move |scale| {
            let val = scale.value().round();
            c_scale.set_eraser_radius(val);
            val_lbl_clone.set_label(&format!("{:.0} px", val));

            for (btn, preset_val) in preset_btns_clone.borrow().iter() {
                if (val - preset_val).abs() < 0.5 {
                    btn.add_css_class("active");
                } else {
                    btn.remove_css_class("active");
                }
            }

            prefs_scale.borrow_mut().eraser_radius = val;
            let _ = prefs_scale.borrow().save();
        });

        eraser_popover.set_child(Some(&eraser_pop_box));

        overlay.add_overlay(&island_box);

        // =========================================================================
        // 3. AZIONI RAPIDE IN ALTO A DESTRA (x: 1216, y: 20)
        // =========================================================================
        let right_actions_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .valign(Align::Start)
            .halign(Align::End)
            .margin_end(22)
            .margin_top(20)
            .build();

        // Pulsante Esporta / Salva
        let btn_export = Button::builder()
            .child(&create_icon(IconKind::Export, 18, None))
            .tooltip_text("Esporta & Salvataggio (Ctrl+S)")
            .css_classes(["dms-action-btn"])
            .build();

        let export_popover = Popover::builder()
            .css_classes(["dms-popover", "dms-context-popover"])
            .build();
        export_popover.set_parent(&btn_export);

        let export_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let lbl_group1 = Label::builder()
            .label("Documento")
            .xalign(0.0)
            .css_classes(["dms-popover-section-title"])
            .build();
        export_box.append(&lbl_group1);

        let make_export_item = |icon_kind: IconKind, title: &str, badge_text: &str| -> Button {
            let row = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .valign(Align::Center)
                .build();

            let icon = create_icon(icon_kind, 16, None);
            let lbl = Label::builder()
                .label(title)
                .xalign(0.0)
                .hexpand(true)
                .build();
            let badge = Label::builder()
                .label(badge_text)
                .valign(Align::Center)
                .css_classes(["dms-shortcut-badge"])
                .build();

            row.append(&icon);
            row.append(&lbl);
            row.append(&badge);

            Button::builder()
                .child(&row)
                .css_classes(["dms-menu-item-btn", "dms-action-card"])
                .build()
        };

        // Salva (.uninote)
        let card_save = make_export_item(IconKind::Export, "Salva", "Ctrl+S");
        let c_save = canvas.clone();
        let win_save = window.clone();
        let exp_pop_ref = export_popover.clone();
        card_save.connect_clicked(move |_| {
            exp_pop_ref.popdown();
            let state_rc = c_save.state();
            let st = state_rc.borrow();
            if let Some(ref path) = st.file_path {
                let path_clone = path.clone();
                drop(st);
                let _ = c_save.save_as(path_clone);
            } else {
                drop(st);
                let dialog = FileDialog::builder()
                    .title("Salva Quaderno UniNote")
                    .initial_name("quaderno.uninote")
                    .modal(true)
                    .build();
                let c_ref = c_save.clone();
                dialog.save(Some(&win_save), gio::Cancellable::NONE, move |res| {
                    if let Ok(file) = res {
                        if let Some(path) = file.path() {
                            let _ = c_ref.save_as(path);
                        }
                    }
                });
            }
        });
        export_box.append(&card_save);

        // Nuovo Quaderno
        let card_new = make_export_item(IconKind::Add, "Nuovo quaderno", "Ctrl+N");
        let c_new = canvas.clone();
        let exp_pop_ref2 = export_popover.clone();
        card_new.connect_clicked(move |_| {
            exp_pop_ref2.popdown();
            c_new.new_document();
        });
        export_box.append(&card_new);

        // Separatore orizzontale
        let sep = gtk4::Separator::new(Orientation::Horizontal);
        sep.add_css_class("dms-menu-separator");
        export_box.append(&sep);

        // Group 2: Esporta
        let lbl_group2 = Label::builder()
            .label("Esporta")
            .xalign(0.0)
            .css_classes(["dms-popover-section-title"])
            .build();
        export_box.append(&lbl_group2);

        // Documento PDF (.pdf) - Chiaro / Stampa
        let card_pdf = make_export_item(IconKind::Page, "Esporta in PDF (Chiaro)", ".pdf");
        let c_pdf = canvas.clone();
        let win_pdf = window.clone();
        let exp_pop_ref_pdf = export_popover.clone();
        card_pdf.connect_clicked(move |_| {
            exp_pop_ref_pdf.popdown();
            let doc_title = c_pdf.state().borrow().document.title.clone();
            let initial_file = if doc_title.trim().is_empty() {
                "quaderno.pdf".to_string()
            } else {
                format!("{}.pdf", doc_title)
            };

            let dialog = FileDialog::builder()
                .title("Esporta Documento PDF (Chiaro / Stampa)")
                .initial_name(&initial_file)
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documento PDF (*.pdf)"));
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let c_ref = c_pdf.clone();
            dialog.save(Some(&win_pdf), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let mut export_path = path;
                        if export_path.extension().is_none() {
                            export_path.set_extension("pdf");
                        }
                        let options = crate::pdf_export::PdfExportOptions { is_dark: false };
                        if let Err(e) = c_ref.export_pdf_with_options(&export_path, &options) {
                            eprintln!("Errore durante l'esportazione PDF (Chiaro): {}", e);
                        }
                    }
                }
            });
        });
        export_box.append(&card_pdf);

        // Documento PDF (.pdf) - Scuro / Come a schermo
        let card_pdf_dark = make_export_item(IconKind::Page, "Esporta in PDF (Scuro)", ".pdf");
        let c_pdf_dark = canvas.clone();
        let win_pdf_dark = window.clone();
        let exp_pop_ref_pdf_dark = export_popover.clone();
        card_pdf_dark.connect_clicked(move |_| {
            exp_pop_ref_pdf_dark.popdown();
            let doc_title = c_pdf_dark.state().borrow().document.title.clone();
            let initial_file = if doc_title.trim().is_empty() {
                "quaderno_scuro.pdf".to_string()
            } else {
                format!("{}_scuro.pdf", doc_title)
            };

            let dialog = FileDialog::builder()
                .title("Esporta Documento PDF (Scuro / Come a schermo)")
                .initial_name(&initial_file)
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documento PDF (*.pdf)"));
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let c_ref = c_pdf_dark.clone();
            dialog.save(Some(&win_pdf_dark), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let mut export_path = path;
                        if export_path.extension().is_none() {
                            export_path.set_extension("pdf");
                        }
                        let options = crate::pdf_export::PdfExportOptions { is_dark: true };
                        if let Err(e) = c_ref.export_pdf_with_options(&export_path, &options) {
                            eprintln!("Errore durante l'esportazione PDF (Scuro): {}", e);
                        }
                    }
                }
            });
        });
        export_box.append(&card_pdf_dark);

        // Pacchetto Portabile (.uninote)
        let card_portable = make_export_item(IconKind::Copy, "Copia portabile", ".uninote");
        let c_port = canvas.clone();
        let win_port = window.clone();
        let exp_pop_ref3 = export_popover.clone();
        card_portable.connect_clicked(move |_| {
            exp_pop_ref3.popdown();
            let dialog = FileDialog::builder()
                .title("Esporta Pacchetto Portabile UniNote")
                .initial_name("quaderno_portabile.uninote")
                .modal(true)
                .build();
            let c_ref = c_port.clone();
            dialog.save(Some(&win_port), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let _ = c_ref.export_portable_copy(path);
                    }
                }
            });
        });
        export_box.append(&card_portable);

        export_popover.set_child(Some(&export_box));
        let exp_ref = export_popover.clone();
        btn_export.connect_clicked(move |_| {
            exp_ref.popup();
        });
        right_actions_box.append(&btn_export);

        // Pulsante Impostazioni & Control Center DMS
        let btn_settings = Button::builder()
            .child(&create_icon(IconKind::Settings, 18, None))
            .tooltip_text("Pannello Hardware DMS & Telemetria")
            .css_classes(["dms-action-btn"])
            .build();

        let settings_popover = Popover::builder().css_classes(["dms-popover"]).build();
        settings_popover.set_parent(&btn_settings);

        let settings_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(10)
            .margin_start(12)
            .margin_end(12)
            .margin_top(10)
            .margin_bottom(10)
            .build();

        let lbl_settings_title = Label::builder()
            .label("Control Center UniNote DMS")
            .xalign(0.0)
            .css_classes(["brand-title"])
            .build();
        settings_box.append(&lbl_settings_title);

        // Toggle Overlay HUD Diagnostico
        let hud_ref_toggle = hud.clone();
        let btn_hud_toggle = Button::builder()
            .label("Overlay Telemetria HUD (F12)")
            .css_classes(["dms-tool-btn-neutral"])
            .build();
        btn_hud_toggle.connect_clicked(move |_| {
            let vis = hud_ref_toggle.widget().is_visible();
            hud_ref_toggle.widget().set_visible(!vis);
        });
        settings_box.append(&btn_hud_toggle);

        // Sezione Rifiuto Palmo
        let lbl_palm = Label::builder()
            .label("Riconoscimento Palmo:")
            .xalign(0.0)
            .build();
        settings_box.append(&lbl_palm);

        let touch_strings = [
            "Solo Gesti Pan/Zoom (Consigliato)",
            "Touch Totalmente Bloccato (OFF)",
        ];
        let dropdown_touch = DropDown::from_strings(&touch_strings);
        dropdown_touch.set_selected(0);
        let canvas_touch = canvas.clone();
        let hud_touch = hud.clone();
        dropdown_touch.connect_selected_notify(move |dd| {
            let mode = match dd.selected() {
                0 => TouchMode::GesturesOnly,
                _ => TouchMode::Off,
            };
            canvas_touch
                .state()
                .borrow_mut()
                .palm_engine
                .set_touch_mode(mode);
            hud_touch.update_metrics(|m| m.touch_mode = mode);
            hud_touch.update_view();
        });
        settings_box.append(&dropdown_touch);

        // Sezione Soglia Pressione Punta (Anti-Hover)
        let lbl_thresh = Label::builder()
            .label("Soglia Punta Anti-Hover:")
            .xalign(0.0)
            .build();
        settings_box.append(&lbl_thresh);

        let thresh_strings = [
            "0.065 (Ottimale MPP Prova 7)",
            "0.080 (Pressione Decisa)",
            "0.120 (Alta Resistenza)",
            "0.040 (Sensibile)",
            "0.020 (Ultra Sensibile)",
        ];
        let dropdown_thresh = DropDown::from_strings(&thresh_strings);
        dropdown_thresh.set_selected(0);
        let canvas_thresh = canvas.clone();
        let hud_thresh = hud.clone();
        dropdown_thresh.connect_selected_notify(move |dd| {
            let new_thresh = match dd.selected() {
                0 => 0.065,
                1 => 0.080,
                2 => 0.120,
                3 => 0.040,
                _ => 0.020,
            };
            canvas_thresh.state().borrow_mut().tip_pressure_threshold = new_thresh;
            hud_thresh.update_metrics(|m| m.tip_threshold = new_thresh);
            hud_thresh.update_view();
        });
        settings_box.append(&dropdown_thresh);

        // Sezione Pipeline Smoothing
        let lbl_mode = Label::builder()
            .label("Algoritmo Tratto:")
            .xalign(0.0)
            .build();
        settings_box.append(&lbl_mode);

        let mode_strings = [
            "Spline Catmull-Rom (Naturale)",
            "One Euro Filter + Spline",
            "Baseline Diretto",
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
        settings_box.append(&dropdown_mode);

        settings_popover.set_child(Some(&settings_box));
        let win_settings_ed = window.clone();
        btn_settings.connect_clicked(move |_| {
            let settings_win = crate::settings::UniNoteSettingsWindow::new();
            settings_win.set_transient_for(Some(&win_settings_ed));
            settings_win.set_modal(true);
            settings_win.present();
        });
        right_actions_box.append(&btn_settings);

        overlay.add_overlay(&right_actions_box);

        // =========================================================================
        // 4. DOCK PENNE A SINISTRA (x: 22, y: draggable, larghezza 54px)
        // =========================================================================
        let dock_initial_y = prefs.borrow().dock_y.clamp(72.0, 600.0);
        let left_dock = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .valign(Align::Start)
            .halign(Align::Start)
            .margin_start(22)
            .margin_top(dock_initial_y as i32)
            .css_classes(["dms-left-dock"])
            .build();

        // Maniglia dock interattiva con GestureDrag
        let handle_area = GtkBox::builder()
            .halign(Align::Center)
            .css_classes(["dms-dock-handle-area"])
            .build();

        let handle_bar = GtkBox::builder()
            .halign(Align::Center)
            .valign(Align::Center)
            .css_classes(["dms-dock-handle"])
            .build();
        handle_area.append(&handle_bar);

        let gesture_drag = GestureDrag::new();
        let start_top = Rc::new(Cell::new(0.0f64));
        let max_drag_y = Rc::new(Cell::new(72.0f64));

        let start_top_begin = start_top.clone();
        let max_drag_y_begin = max_drag_y.clone();
        let dock_begin = left_dock.clone();
        let win_begin = window.clone();
        gesture_drag.connect_drag_begin(move |_, _, _| {
            let init_y = dock_begin.margin_top() as f64;
            start_top_begin.set(init_y);
            let win_h = win_begin.height() as f64;
            let dock_h = dock_begin.height() as f64;
            let max_y = (win_h - dock_h - 60.0).max(72.0);
            max_drag_y_begin.set(max_y);
        });

        let start_top_update = start_top.clone();
        let max_drag_y_update = max_drag_y.clone();
        let dock_update = left_dock.clone();
        gesture_drag.connect_drag_update(move |_, _, offset_y| {
            let init_y = start_top_update.get();
            let max_y = max_drag_y_update.get();
            let new_y = (init_y + offset_y).clamp(72.0, max_y);
            dock_update.set_margin_top(new_y as i32);
        });

        let dock_end = left_dock.clone();
        let prefs_drag_end = prefs.clone();
        gesture_drag.connect_drag_end(move |_, _, _| {
            let final_y = dock_end.margin_top() as f64;
            prefs_drag_end.borrow_mut().dock_y = final_y;
            let _ = prefs_drag_end.borrow().save();
        });
        handle_area.add_controller(gesture_drag);
        left_dock.append(&handle_area);

        // Contenitore verticale per i bottoni degli slot di inchiostro
        let slots_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .build();
        left_dock.append(&slots_box);

        let slot_buttons: Rc<RefCell<Vec<Button>>> = Rc::new(RefCell::new(Vec::new()));

        let create_slot_btn = {
            let slot_buttons = slot_buttons.clone();
            let prefs = prefs.clone();
            let canvas = canvas.clone();
            let caliber_dot = caliber_dot.clone();
            let tool_buttons = tool_buttons.clone();
            let stroke_preview = stroke_preview.clone();
            let window = window.clone();
            let slots_box = slots_box.clone();

            Rc::new(move |slot_cfg: PenSlotConfig| -> Button {
                let rgba = slot_cfg.rgba;
                let pen_icon = create_icon(
                    IconKind::PenLine,
                    18,
                    Some(gdk::RGBA::new(
                        rgba.0 as f32,
                        rgba.1 as f32,
                        rgba.2 as f32,
                        rgba.3 as f32,
                    )),
                );

                let slot_btn = Button::builder()
                    .child(&pen_icon)
                    .tooltip_text(&slot_cfg.name)
                    .css_classes(["dms-pen-slot"])
                    .build();

                // Clic singolo: attiva lo slot
                let c_color = canvas.clone();
                let all_slots = slot_buttons.clone();
                let slot_self = slot_btn.clone();
                let cal_dot_ref = caliber_dot.clone();
                let tool_btns_ref = tool_buttons.clone();
                let stroke_prev_ref = stroke_preview.clone();
                let prefs_click = prefs.clone();

                slot_btn.connect_clicked(move |_| {
                    let mut active_idx = 0;
                    for (idx, s) in all_slots.borrow().iter().enumerate() {
                        if s == &slot_self {
                            s.add_css_class("active");
                            active_idx = idx;
                        } else {
                            s.remove_css_class("active");
                        }
                    }

                    prefs_click.borrow_mut().active_slot_index = active_idx;
                    let _ = prefs_click.borrow().save();

                    if let Some(cfg) = prefs_click.borrow().slots.get(active_idx) {
                        c_color.set_pen_color(cfg.rgba);
                        c_color.set_active_tool(ActiveTool::Pen);

                        let tb = tool_btns_ref.borrow();
                        for (idx, b) in tb.iter().enumerate() {
                            if idx == 0 {
                                b.add_css_class("active");
                            } else {
                                b.remove_css_class("active");
                            }
                        }

                        cal_dot_ref.set_markup(&format!(
                            "<span foreground='{}' font_desc='11'>●</span>",
                            cfg.hex
                        ));
                    }

                    stroke_prev_ref.queue_draw();
                });

                // Clic secondario e Long press per aprire il menu contestuale (Modifica Colore / Elimina Penna)
                let open_context_menu = {
                    let slot_self = slot_btn.clone();
                    let all_slots = slot_buttons.clone();
                    let prefs_edit = prefs.clone();
                    let c_color = canvas.clone();
                    let cal_dot_ref = caliber_dot.clone();
                    let stroke_prev_ref = stroke_preview.clone();
                    let win = window.clone();
                    let slots_box_ref = slots_box.clone();

                    Rc::new(move || {
                        let idx = all_slots.borrow().iter().position(|b| b == &slot_self);
                        if idx.is_none() {
                            return;
                        };

                        let popover = Popover::builder()
                            .autohide(true)
                            .has_arrow(true)
                            .position(PositionType::Right)
                            .css_classes(["dms-popover"])
                            .build();
                        popover.set_parent(&slot_self);

                        let menu_box = GtkBox::builder()
                            .orientation(Orientation::Vertical)
                            .spacing(4)
                            .margin_top(6)
                            .margin_bottom(6)
                            .margin_start(6)
                            .margin_end(6)
                            .build();

                        // 1. Modifica Colore
                        let edit_content = GtkBox::builder()
                            .orientation(Orientation::Horizontal)
                            .spacing(8)
                            .build();
                        edit_content.append(&create_icon(IconKind::Edit, 16, None));
                        edit_content.append(
                            &Label::builder()
                                .label("Modifica Colore...")
                                .xalign(0.0)
                                .build(),
                        );
                        let btn_edit = Button::builder()
                            .child(&edit_content)
                            .css_classes(["dms-menu-item-btn"])
                            .build();

                        let pop_edit = popover.clone();
                        let all_slots_edit = all_slots.clone();
                        let slot_self_edit = slot_self.clone();
                        let prefs_edit_cb = prefs_edit.clone();
                        let win_edit = win.clone();
                        let c_color_edit = c_color.clone();
                        let cal_dot_edit = cal_dot_ref.clone();
                        let stroke_prev_edit = stroke_prev_ref.clone();

                        btn_edit.connect_clicked(move |_| {
                            pop_edit.popdown();
                            let current_idx = all_slots_edit
                                .borrow()
                                .iter()
                                .position(|b| b == &slot_self_edit);
                            let Some(cur_slot_idx) = current_idx else {
                                return;
                            };

                            let current_rgba = prefs_edit_cb
                                .borrow()
                                .slots
                                .get(cur_slot_idx)
                                .map(|s| s.rgba)
                                .unwrap_or((0.0, 0.0, 0.0, 1.0));

                            let dialog = ColorDialog::builder()
                                .title("Modifica Colore Inchiostro")
                                .modal(true)
                                .build();

                            let initial = gdk::RGBA::new(
                                current_rgba.0 as f32,
                                current_rgba.1 as f32,
                                current_rgba.2 as f32,
                                current_rgba.3 as f32,
                            );

                            let slot_btn_inner = slot_self_edit.clone();
                            let prefs_cb = prefs_edit_cb.clone();
                            let c_color_cb = c_color_edit.clone();
                            let cal_dot_cb = cal_dot_edit.clone();
                            let stroke_prev_cb = stroke_prev_edit.clone();

                            dialog.choose_rgba(
                                Some(&win_edit),
                                Some(&initial),
                                gio::Cancellable::NONE,
                                move |res| {
                                    if let Ok(rgba) = res {
                                        let tuple = (
                                            rgba.red() as f64,
                                            rgba.green() as f64,
                                            rgba.blue() as f64,
                                            rgba.alpha() as f64,
                                        );
                                        let hex = format!(
                                            "#{:02X}{:02X}{:02X}",
                                            (tuple.0 * 255.0) as u8,
                                            (tuple.1 * 255.0) as u8,
                                            (tuple.2 * 255.0) as u8
                                        );
                                        let name = format!("Personalizzato {}", hex);

                                        if let Some(slot) =
                                            prefs_cb.borrow_mut().slots.get_mut(cur_slot_idx)
                                        {
                                            slot.hex = hex.clone();
                                            slot.rgba = tuple;
                                            slot.name = name.clone();
                                        }
                                        let _ = prefs_cb.borrow().save();

                                        let new_icon =
                                            create_icon(IconKind::PenLine, 18, Some(rgba));
                                        slot_btn_inner.set_child(Some(&new_icon));
                                        slot_btn_inner.set_tooltip_text(Some(&name));

                                        if prefs_cb.borrow().active_slot_index == cur_slot_idx {
                                            c_color_cb.set_pen_color(tuple);
                                            cal_dot_cb.set_markup(&format!(
                                                "<span foreground='{}' font_desc='11'>●</span>",
                                                hex
                                            ));
                                            stroke_prev_cb.queue_draw();
                                        }
                                    }
                                },
                            );
                        });
                        menu_box.append(&btn_edit);

                        // 2. Elimina Penna (disabilitato se rimane un solo slot)
                        let del_content = GtkBox::builder()
                            .orientation(Orientation::Horizontal)
                            .spacing(8)
                            .build();
                        del_content.append(&create_icon(IconKind::Trash, 16, None));
                        del_content
                            .append(&Label::builder().label("Elimina Penna").xalign(0.0).build());
                        let btn_del = Button::builder()
                            .child(&del_content)
                            .css_classes(["dms-menu-item-btn", "destructive-action"])
                            .build();

                        let can_delete = prefs_edit.borrow().slots.len() > 1;
                        if !can_delete {
                            btn_del.set_sensitive(false);
                            btn_del.set_tooltip_text(Some(
                                "Impossibile eliminare l'unica penna rimasta",
                            ));
                        }

                        let pop_del = popover.clone();
                        let all_slots_del = all_slots.clone();
                        let slot_self_del = slot_self.clone();
                        let prefs_del_cb = prefs_edit.clone();
                        let slots_box_del = slots_box_ref.clone();
                        let c_color_del = c_color.clone();
                        let cal_dot_del = cal_dot_ref.clone();
                        let stroke_prev_del = stroke_prev_ref.clone();

                        btn_del.connect_clicked(move |_| {
                            pop_del.popdown();
                            let current_idx = all_slots_del
                                .borrow()
                                .iter()
                                .position(|b| b == &slot_self_del);
                            let Some(cur_slot_idx) = current_idx else {
                                return;
                            };

                            if prefs_del_cb.borrow_mut().remove_slot(cur_slot_idx) {
                                let _ = prefs_del_cb.borrow().save();
                                slots_box_del.remove(&slot_self_del);
                                all_slots_del.borrow_mut().remove(cur_slot_idx);

                                let active_idx = prefs_del_cb.borrow().active_slot_index;
                                for (i, btn) in all_slots_del.borrow().iter().enumerate() {
                                    if i == active_idx {
                                        btn.add_css_class("active");
                                    } else {
                                        btn.remove_css_class("active");
                                    }
                                }

                                if let Some(cfg) = prefs_del_cb.borrow().slots.get(active_idx) {
                                    c_color_del.set_pen_color(cfg.rgba);
                                    cal_dot_del.set_markup(&format!(
                                        "<span foreground='{}' font_desc='11'>●</span>",
                                        cfg.hex
                                    ));
                                    stroke_prev_del.queue_draw();
                                }
                            }
                        });
                        menu_box.append(&btn_del);

                        popover.set_child(Some(&menu_box));
                        popover.popup();
                    })
                };

                let right_click = GestureClick::builder()
                    .button(gdk::BUTTON_SECONDARY)
                    .build();
                let editor_rc = open_context_menu.clone();
                right_click.connect_pressed(move |_, _, _, _| {
                    editor_rc();
                });
                slot_btn.add_controller(right_click);

                let long_press = GestureLongPress::new();
                long_press.set_touch_only(false);
                let editor_lp = open_context_menu.clone();
                long_press.connect_pressed(move |_, _, _| {
                    editor_lp();
                });
                slot_btn.add_controller(long_press);

                slot_btn
            })
        };

        // Renderizza slot iniziali da preferences
        let active_slot = prefs.borrow().active_slot_index;
        for (i, slot_cfg) in prefs.borrow().slots.iter().enumerate() {
            let btn = create_slot_btn(slot_cfg.clone());
            if i == active_slot {
                btn.add_css_class("active");
            }
            slots_box.append(&btn);
            slot_buttons.borrow_mut().push(btn);
        }

        // Tasto [+] Aggiungi colore
        let btn_add_color = Button::builder()
            .child(&create_icon(IconKind::Add, 14, None))
            .tooltip_text("Aggiungi Nuovo Colore Inchiostro")
            .css_classes(["dms-add-pen-btn"])
            .build();

        let win_color = window.clone();
        let prefs_add = prefs.clone();
        let slots_box_ref = slots_box.clone();
        let slot_buttons_ref = slot_buttons.clone();
        let create_slot_btn_ref = create_slot_btn.clone();
        let c_color_add = canvas.clone();
        let cal_dot_add = caliber_dot.clone();
        let tool_btns_add = tool_buttons.clone();
        let stroke_prev_add = stroke_preview.clone();

        btn_add_color.connect_clicked(move |_| {
            let dialog = ColorDialog::builder()
                .title("Aggiungi Nuovo Colore Inchiostro")
                .modal(true)
                .build();

            let prefs_cb = prefs_add.clone();
            let slots_box_cb = slots_box_ref.clone();
            let slot_buttons_cb = slot_buttons_ref.clone();
            let create_slot_btn_cb = create_slot_btn_ref.clone();
            let c_color_cb = c_color_add.clone();
            let cal_dot_cb = cal_dot_add.clone();
            let tool_btns_cb = tool_btns_add.clone();
            let stroke_prev_cb = stroke_prev_add.clone();

            dialog.choose_rgba(
                Some(&win_color),
                None::<&gdk::RGBA>,
                gio::Cancellable::NONE,
                move |res| {
                    if let Ok(rgba) = res {
                        let tuple = (
                            rgba.red() as f64,
                            rgba.green() as f64,
                            rgba.blue() as f64,
                            rgba.alpha() as f64,
                        );
                        let hex = format!(
                            "#{:02X}{:02X}{:02X}",
                            (tuple.0 * 255.0) as u8,
                            (tuple.1 * 255.0) as u8,
                            (tuple.2 * 255.0) as u8
                        );
                        let name = format!("Personalizzato {}", hex);
                        let slot_cfg = PenSlotConfig::new(hex.clone(), tuple, name);

                        let new_idx = prefs_cb.borrow().slots.len();
                        prefs_cb.borrow_mut().slots.push(slot_cfg.clone());
                        prefs_cb.borrow_mut().active_slot_index = new_idx;
                        let _ = prefs_cb.borrow().save();

                        let new_btn = create_slot_btn_cb(slot_cfg);
                        new_btn.add_css_class("active");

                        for s in slot_buttons_cb.borrow().iter() {
                            s.remove_css_class("active");
                        }

                        slots_box_cb.append(&new_btn);
                        slot_buttons_cb.borrow_mut().push(new_btn);

                        c_color_cb.set_pen_color(tuple);
                        c_color_cb.set_active_tool(ActiveTool::Pen);

                        let tb = tool_btns_cb.borrow();
                        for (idx, b) in tb.iter().enumerate() {
                            if idx == 0 {
                                b.add_css_class("active");
                            } else {
                                b.remove_css_class("active");
                            }
                        }

                        cal_dot_cb.set_markup(&format!(
                            "<span foreground='{}' font_desc='11'>●</span>",
                            hex
                        ));
                        stroke_prev_cb.queue_draw();
                    }
                },
            );
        });
        left_dock.append(&btn_add_color);

        overlay.add_overlay(&left_dock);

        // =========================================================================
        // 5. DOCK PAGINE E ZOOM INFERIORE (y: 794, larghezza 486px)
        // =========================================================================
        let bottom_dock = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .valign(Align::End)
            .halign(Align::Center)
            .margin_bottom(16)
            .css_classes(["dms-pill-dense"])
            .build();

        // Navigazione Pagine: ‹ Pagina X di Y ›
        let btn_prev_page = Button::builder()
            .child(&create_icon(IconKind::ChevronLeft, 14, None))
            .tooltip_text("Pagina Precedente")
            .css_classes(["dms-zoom-btn"])
            .build();

        let page_icon = create_icon(IconKind::Page, 16, None);

        let lbl_page = Label::builder()
            .label("Pagina 1 di 1")
            .css_classes(["dock-page-text"])
            .build();

        let btn_next_page = Button::builder()
            .child(&create_icon(IconKind::ChevronRight, 14, None))
            .tooltip_text("Pagina Successiva")
            .css_classes(["dms-zoom-btn"])
            .build();

        let c_page_nav = canvas.clone();
        let lbl_page_clone1 = lbl_page.clone();
        let update_page_lbl = {
            let c = c_page_nav.clone();
            let lbl = lbl_page_clone1.clone();
            move || {
                let cur = c.current_page_index() + 1;
                let tot = c.total_pages();
                lbl.set_label(&format!("Pagina {} di {}", cur, tot));
            }
        };

        let c_prev = canvas.clone();
        let upd_prev = update_page_lbl.clone();
        btn_prev_page.connect_clicked(move |_| {
            let cur = c_prev.current_page_index();
            if cur > 0 {
                c_prev.scroll_to_page(cur - 1);
                upd_prev();
            }
        });

        let c_next = canvas.clone();
        let upd_next = update_page_lbl.clone();
        btn_next_page.connect_clicked(move |_| {
            let cur = c_next.current_page_index();
            if cur + 1 < c_next.total_pages() {
                c_next.scroll_to_page(cur + 1);
                upd_next();
            }
        });

        let notebook_dock_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        notebook_dock_box.append(&btn_prev_page);
        notebook_dock_box.append(&page_icon);
        notebook_dock_box.append(&lbl_page);
        notebook_dock_box.append(&btn_next_page);

        // Separatore 1
        let div_dock1 = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        notebook_dock_box.append(&div_dock1);

        // Tasto [+ Pagina] (Aggiunge una nuova pagina A4)
        let btn_add_page = Button::builder()
            .label("+ Pagina")
            .tooltip_text("Aggiungi Nuova Pagina A4")
            .css_classes(["dms-btn-add-page"])
            .build();
        let c_add = canvas.clone();
        let upd_add = update_page_lbl.clone();
        btn_add_page.connect_clicked(move |_| {
            c_add.add_page(PageBackground::Grid { grid_size: 14.17 });
            upd_add();
        });
        notebook_dock_box.append(&btn_add_page);

        // Separatore 2
        let div_dock2 = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        notebook_dock_box.append(&div_dock2);
        bottom_dock.append(&notebook_dock_box);

        // In modalità Lavagna Infinita: Tasto "Torna al centro"
        let whiteboard_dock_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .build();
        let btn_recenter = Button::builder()
            .label("Torna al centro")
            .tooltip_text("Ripristina la vista all'origine della lavagna")
            .css_classes(["dms-btn-recenter"])
            .build();
        let c_recenter = canvas.clone();
        btn_recenter.connect_clicked(move |_| {
            c_recenter.reset_zoom();
        });
        whiteboard_dock_box.append(&btn_recenter);

        let div_dock_wb = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        whiteboard_dock_box.append(&div_dock_wb);
        bottom_dock.append(&whiteboard_dock_box);

        // Selettore Sfondo: Quadretti ▾
        let btn_grid_select = Button::builder()
            .label("Quadretti ▾")
            .tooltip_text("Scegli Sfondo Foglio")
            .css_classes(["dms-btn-grid-select"])
            .build();

        let grid_popover = Popover::builder().css_classes(["dms-popover"]).build();
        grid_popover.set_parent(&btn_grid_select);

        let grid_pop_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(6)
            .margin_start(8)
            .margin_end(8)
            .margin_top(6)
            .margin_bottom(6)
            .build();

        let grid_options: [(&str, &str, PageBackground, &str); 3] = [
            (
                "Quadretti",
                "Quadretti ▾",
                PageBackground::Grid { grid_size: 14.17 },
                "grid",
            ),
            (
                "Righe",
                "Righe ▾",
                PageBackground::Ruled {
                    line_spacing: 22.68,
                    margin_left: 56.7,
                },
                "ruled",
            ),
            ("Bianco", "Bianco ▾", PageBackground::Blank, "blank"),
        ];

        let lbl_grid_title = Label::builder()
            .label("Tipo di Foglio")
            .xalign(0.0)
            .css_classes(["brand-title"])
            .build();
        grid_pop_box.append(&lbl_grid_title);

        let cards_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .build();

        let paper_cards: Rc<RefCell<Vec<Button>>> = Rc::new(RefCell::new(Vec::new()));

        for (i, (card_title, label_text, bg_kind, kind_id)) in grid_options.into_iter().enumerate()
        {
            let card_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(6)
                .halign(Align::Center)
                .valign(Align::Center)
                .build();

            let preview_area = DrawingArea::builder()
                .content_width(68)
                .content_height(80)
                .build();

            let kind_str = kind_id.to_string();
            preview_area.set_draw_func(move |_area, cr, width, height| {
                let w = width as f64;
                let h = height as f64;

                let r = 4.0;
                cr.save().expect("cairo save");
                cr.new_sub_path();
                cr.arc(w - r, r, r, -PI / 2.0, 0.0);
                cr.arc(w - r, h - r, r, 0.0, PI / 2.0);
                cr.arc(r, h - r, r, PI / 2.0, PI);
                cr.arc(r, r, r, PI, 3.0 * PI / 2.0);
                cr.close_path();

                // Foglio bianco caldo naturale
                cr.set_source_rgb(0.98, 0.98, 0.97);
                let _ = cr.fill_preserve();

                // Bordo foglio
                cr.set_source_rgba(0.70, 0.75, 0.78, 0.8);
                cr.set_line_width(0.8);
                let _ = cr.stroke();

                // Pattern interno con clip
                cr.new_sub_path();
                cr.arc(w - r, r, r, -PI / 2.0, 0.0);
                cr.arc(w - r, h - r, r, 0.0, PI / 2.0);
                cr.arc(r, h - r, r, PI / 2.0, PI);
                cr.arc(r, r, r, PI, 3.0 * PI / 2.0);
                cr.close_path();
                cr.clip();

                match kind_str.as_str() {
                    "grid" => {
                        // Griglia miniaturizzata da 5mm
                        cr.set_source_rgba(0.68, 0.78, 0.85, 0.75);
                        cr.set_line_width(0.6);
                        let step = 7.0;
                        let mut x = step;
                        while x < w {
                            cr.move_to(x, 0.0);
                            cr.line_to(x, h);
                            x += step;
                        }
                        let mut y = step;
                        while y < h {
                            cr.move_to(0.0, y);
                            cr.line_to(w, y);
                            y += step;
                        }
                        let _ = cr.stroke();
                    }
                    "ruled" => {
                        // Righe miniaturizzate
                        cr.set_source_rgba(0.68, 0.78, 0.85, 0.75);
                        cr.set_line_width(0.6);
                        let step = 9.0;
                        let mut y = 14.0;
                        while y < h {
                            cr.move_to(0.0, y);
                            cr.line_to(w, y);
                            y += step;
                        }
                        let _ = cr.stroke();

                        // Margine sinistro rosso/corallo
                        cr.set_source_rgba(1.0, 0.45, 0.42, 0.85);
                        cr.set_line_width(0.8);
                        cr.move_to(16.0, 0.0);
                        cr.line_to(16.0, h);
                        let _ = cr.stroke();
                    }
                    "blank" => {
                        // Foglio bianco naturale
                        cr.set_source_rgba(0.80, 0.85, 0.88, 0.35);
                        cr.set_line_width(0.5);
                        cr.move_to(8.0, 12.0);
                        cr.line_to(w - 8.0, 12.0);
                        let _ = cr.stroke();
                    }
                    _ => {}
                }

                cr.restore().expect("cairo restore");
            });

            card_box.append(&preview_area);

            let title_lbl = Label::builder()
                .label(card_title)
                .css_classes(["dms-paper-card-title"])
                .build();
            card_box.append(&title_lbl);

            let card_btn = Button::builder()
                .child(&card_box)
                .tooltip_text(card_title)
                .css_classes(["dms-paper-card"])
                .build();

            if i == 0 {
                card_btn.add_css_class("active");
            }

            let c_bg = canvas.clone();
            let btn_grid_lbl = btn_grid_select.clone();
            let pop_close = grid_popover.clone();
            let all_cards = paper_cards.clone();
            let this_card = card_btn.clone();

            card_btn.connect_clicked(move |_| {
                for c in all_cards.borrow().iter() {
                    c.remove_css_class("active");
                }
                this_card.add_css_class("active");
                pop_close.popdown();
                c_bg.set_current_page_background(bg_kind.clone());
                btn_grid_lbl.set_label(label_text);
            });

            cards_row.append(&card_btn);
            paper_cards.borrow_mut().push(card_btn);
        }

        grid_pop_box.append(&cards_row);

        grid_popover.set_child(Some(&grid_pop_box));
        let grid_pop_ref = grid_popover.clone();
        btn_grid_select.connect_clicked(move |_| {
            grid_pop_ref.popup();
        });
        bottom_dock.append(&btn_grid_select);

        // Separatore 3
        let div_dock3 = GtkBox::builder().css_classes(["dms-dock-divider"]).build();
        bottom_dock.append(&div_dock3);

        // Segmento Zoom: − 100% +
        let zoom_segment = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(2)
            .css_classes(["dms-zoom-segment"])
            .build();

        let btn_zoom_out = Button::builder()
            .child(&create_icon(IconKind::Minus, 12, None))
            .tooltip_text("Rimpicciolisci")
            .css_classes(["dms-zoom-btn"])
            .build();

        let lbl_zoom = Label::builder()
            .label("100%")
            .tooltip_text("Doppio clic per ripristinare 100%")
            .css_classes(["zoom-value"])
            .build();

        let btn_zoom_in = Button::builder()
            .child(&create_icon(IconKind::Add, 12, None))
            .tooltip_text("Ingrandisci")
            .css_classes(["dms-zoom-btn"])
            .build();

        let c_z_out = canvas.clone();
        let lbl_z1 = lbl_zoom.clone();
        btn_zoom_out.connect_clicked(move |_| {
            c_z_out.zoom_out();
            let z = (c_z_out.state().borrow().viewport.zoom * 100.0).round() as usize;
            lbl_z1.set_label(&format!("{}%", z));
        });

        let c_z_in = canvas.clone();
        let lbl_z2 = lbl_zoom.clone();
        btn_zoom_in.connect_clicked(move |_| {
            c_z_in.zoom_in();
            let z = (c_z_in.state().borrow().viewport.zoom * 100.0).round() as usize;
            lbl_z2.set_label(&format!("{}%", z));
        });

        let c_z_reset = canvas.clone();
        let lbl_z3 = lbl_zoom.clone();
        let click_gesture = GestureClick::new();
        click_gesture.connect_pressed(move |_, _, _, _| {
            let is_wb = c_z_reset.state().borrow().document.format == DocumentFormat::Whiteboard;
            if is_wb {
                c_z_reset.center_content();
            } else {
                c_z_reset.reset_zoom();
            }
            let z = (c_z_reset.state().borrow().viewport.zoom * 100.0).round() as usize;
            lbl_z3.set_label(&format!("{}%", z));
        });
        lbl_zoom.add_controller(click_gesture);

        // Sincronizzazione fluida continua con GestureZoom / pinch touch
        let lbl_zoom_live = lbl_zoom.clone();
        canvas.connect_zoom_changed(move |zoom| {
            let pct = (zoom * 100.0).round() as usize;
            lbl_zoom_live.set_label(&format!("{}%", pct));
        });

        zoom_segment.append(&btn_zoom_out);
        zoom_segment.append(&lbl_zoom);
        zoom_segment.append(&btn_zoom_in);
        bottom_dock.append(&zoom_segment);

        overlay.add_overlay(&bottom_dock);

        // =========================================================================
        // 6. OVERLAY DIAGNOSTICO HUD (Posizionato in basso a destra, attivabile con F12)
        // =========================================================================
        let hud_widget = hud.widget();
        hud_widget.set_halign(Align::End);
        hud_widget.set_valign(Align::End);
        hud_widget.set_margin_end(20);
        hud_widget.set_margin_bottom(80);
        overlay.add_overlay(hud_widget);

        // =========================================================================
        // 7. SCORCIATOIE DA TASTIERA GLOBALI
        // =========================================================================
        let key_controller = EventControllerKey::new();
        let c_keys = canvas.clone();
        let win_keys = window.clone();
        let hud_keys = hud.clone();
        let lbl_zoom_sync = lbl_zoom.clone();
        key_controller.connect_key_pressed(move |_ctrl, keyval, _keycode, modifier| {
            let is_ctrl = modifier.contains(gdk::ModifierType::CONTROL_MASK);
            let is_shift = modifier.contains(gdk::ModifierType::SHIFT_MASK);

            if keyval == gdk::Key::F12 {
                let vis = hud_keys.widget().is_visible();
                let new_vis = !vis;
                hud_keys.widget().set_visible(new_vis);
                if new_vis {
                    hud_keys.update_view();
                    c_keys.widget().queue_draw();
                }
                return glib::Propagation::Stop;
            }

            // Cancellazione selezione con Delete o BackSpace
            if (keyval == gdk::Key::Delete || keyval == gdk::Key::BackSpace)
                && c_keys.delete_selected()
            {
                return glib::Propagation::Stop;
            }

            if is_ctrl {
                let is_editable_focused = gtk4::prelude::GtkWindowExt::focus(&win_keys)
                    .is_some_and(|w| w.is::<gtk4::Editable>());
                if !is_editable_focused {
                    match keyval {
                        gdk::Key::c | gdk::Key::C => {
                            if c_keys.copy_selected() {
                                return glib::Propagation::Stop;
                            }
                        }
                        gdk::Key::x | gdk::Key::X => {
                            if c_keys.cut_selected() {
                                return glib::Propagation::Stop;
                            }
                        }
                        gdk::Key::v | gdk::Key::V => {
                            if c_keys.paste_from_clipboard() {
                                return glib::Propagation::Stop;
                            }
                        }
                        gdk::Key::z | gdk::Key::Z => {
                            if is_shift {
                                c_keys.redo();
                            } else {
                                c_keys.undo();
                            }
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::y | gdk::Key::Y => {
                            c_keys.redo();
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::_0 | gdk::Key::KP_0 => {
                            let is_wb = c_keys.state().borrow().document.format
                                == DocumentFormat::Whiteboard;
                            if is_wb {
                                c_keys.center_content();
                            } else {
                                c_keys.reset_zoom();
                            }
                            let z =
                                (c_keys.state().borrow().viewport.zoom * 100.0).round() as usize;
                            lbl_zoom_sync.set_label(&format!("{}%", z));
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add => {
                            c_keys.zoom_in();
                            let z =
                                (c_keys.state().borrow().viewport.zoom * 100.0).round() as usize;
                            lbl_zoom_sync.set_label(&format!("{}%", z));
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::minus | gdk::Key::underscore | gdk::Key::KP_Subtract => {
                            c_keys.zoom_out();
                            let z =
                                (c_keys.state().borrow().viewport.zoom * 100.0).round() as usize;
                            lbl_zoom_sync.set_label(&format!("{}%", z));
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::s | gdk::Key::S => {
                            let state_rc = c_keys.state();
                            let st = state_rc.borrow();
                            if let Some(ref path) = st.file_path {
                                let path_clone = path.clone();
                                drop(st);
                                let _ = c_keys.save_as(path_clone);
                            } else {
                                drop(st);
                                let dialog = FileDialog::builder()
                                    .title("Salva Quaderno UniNote")
                                    .initial_name("quaderno.uninote")
                                    .modal(true)
                                    .build();
                                let c_ref = c_keys.clone();
                                dialog.save(Some(&win_keys), gio::Cancellable::NONE, move |res| {
                                    if let Ok(file) = res {
                                        if let Some(path) = file.path() {
                                            let _ = c_ref.save_as(path);
                                        }
                                    }
                                });
                            }
                            return glib::Propagation::Stop;
                        }
                        gdk::Key::n | gdk::Key::N => {
                            c_keys.new_document();
                            return glib::Propagation::Stop;
                        }
                        _ => {}
                    }
                }
            }
            glib::Propagation::Proceed
        });
        window.add_controller(key_controller);

        // =========================================================================
        // 8. SINCRONIZZAZIONE DINAMICA TEMA CHIARO/SCURO DMS (libadwaita StyleManager)
        // =========================================================================
        let win_init = window.clone();
        let p1_init = thick_popover.clone();
        let p2_init = export_popover.clone();
        let p3_init = settings_popover.clone();
        let p4_init = grid_popover.clone();
        let p5_init = eraser_popover.clone();
        let is_dark_init = libadwaita::StyleManager::default().is_dark();
        if !is_dark_init {
            win_init.add_css_class("light-theme");
            p1_init.add_css_class("light-theme");
            p2_init.add_css_class("light-theme");
            p3_init.add_css_class("light-theme");
            p4_init.add_css_class("light-theme");
            p5_init.add_css_class("light-theme");
        }

        let win_sm = window.clone();
        let p1_sm = thick_popover.clone();
        let p2_sm = export_popover.clone();
        let p3_sm = settings_popover.clone();
        let p4_sm = grid_popover.clone();
        let p5_sm = eraser_popover.clone();
        libadwaita::StyleManager::default().connect_dark_notify(move |sm| {
            let is_dark = sm.is_dark();
            if is_dark {
                win_sm.remove_css_class("light-theme");
                p1_sm.remove_css_class("light-theme");
                p2_sm.remove_css_class("light-theme");
                p3_sm.remove_css_class("light-theme");
                p4_sm.remove_css_class("light-theme");
                p5_sm.remove_css_class("light-theme");
            } else {
                win_sm.add_css_class("light-theme");
                p1_sm.add_css_class("light-theme");
                p2_sm.add_css_class("light-theme");
                p3_sm.add_css_class("light-theme");
                p4_sm.add_css_class("light-theme");
                p5_sm.add_css_class("light-theme");
            }
        });

        // Connessione dei segnali della Dashboard
        let c_open = canvas.clone();
        let s_open = stack.clone();
        let w_open = window.clone();
        let nb_open = notebook_dock_box.clone();
        let wb_open = whiteboard_dock_box.clone();
        dashboard.connect_open_document(move |path| {
            let is_pdf = path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(|ext| ext.eq_ignore_ascii_case("pdf"))
                .unwrap_or(false);

            if is_pdf {
                if let Err(e) = c_open.create_from_pdf(&path) {
                    eprintln!(
                        "Errore durante l'apertura del PDF {}: {}",
                        path.display(),
                        e
                    );
                    return;
                }
            } else {
                let _ = c_open.open_file(path);
            }
            let is_wb = c_open.document_format() == DocumentFormat::Whiteboard;
            nb_open.set_visible(!is_wb);
            wb_open.set_visible(is_wb);
            c_open.center_page_1();
            let title = c_open.document_title();
            w_open.set_title(Some(&format!("UniNote — {}", title)));
            s_open.set_visible_child_name("editor");
        });

        let c_create = canvas.clone();
        let s_create = stack.clone();
        let w_create = window.clone();
        let nb_create = notebook_dock_box.clone();
        let wb_create = whiteboard_dock_box.clone();
        let d_create = dashboard.clone();
        dashboard.connect_create_document(move |title, format, bg| {
            c_create.create_document(&title, format, bg);
            let is_wb = c_create.document_format() == DocumentFormat::Whiteboard;
            nb_create.set_visible(!is_wb);
            wb_create.set_visible(is_wb);
            c_create.center_page_1();

            let store = d_create.store();
            let active_dir = d_create.current_folder();
            if !active_dir.exists() {
                let _ = std::fs::create_dir_all(&active_dir);
            }
            let note_path = active_dir.join(format!("{}.uninote", title));

            if let Err(e) = c_create.save_as(note_path.clone()) {
                eprintln!("Attenzione: impossibile inizializzare autosave: {}", e);
            } else {
                let meta = crate::preferences::RecentDocumentMeta {
                    file_path: note_path.to_string_lossy().to_string(),
                    title: title.to_string(),
                    format,
                    last_modified_timestamp: std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    page_count: 1,
                    thumbnail_path: None,
                };
                store.borrow_mut().add_recent(meta);
                let _ = store.borrow().save();
            }

            w_create.set_title(Some(&format!("UniNote — {}", title)));
            s_create.set_visible_child_name("editor");
        });

        let c_import = canvas.clone();
        let s_import = stack.clone();
        let w_import = window.clone();
        let nb_import = notebook_dock_box.clone();
        let wb_import = whiteboard_dock_box.clone();
        dashboard.connect_import_pdf(move |path| match c_import.create_from_pdf(&path) {
            Ok(_) => {
                let is_wb = c_import.document_format() == DocumentFormat::Whiteboard;
                nb_import.set_visible(!is_wb);
                wb_import.set_visible(is_wb);
                c_import.center_page_1();
                let title = c_import.document_title();
                w_import.set_title(Some(&format!("UniNote — {}", title)));
                s_import.set_visible_child_name("editor");
            }
            Err(e) => {
                eprintln!(
                    "Errore durante l'importazione del PDF {}: {}",
                    path.display(),
                    e
                );
            }
        });

        let win_settings_dash = window.clone();
        dashboard.connect_open_settings(move || {
            let settings_win = crate::settings::UniNoteSettingsWindow::new();
            settings_win.set_transient_for(Some(&win_settings_dash));
            settings_win.set_modal(true);
            settings_win.present();
        });

        stack.add_named(dashboard.widget(), Some("dashboard"));
        stack.add_named(&overlay, Some("editor"));
        stack.set_visible_child_name("dashboard");

        window.set_content(Some(&stack));

        // Salvataggio garantito prima della chiusura della finestra
        let c_close = canvas.clone();
        window.connect_close_request(move |_| {
            c_close.persist_changes();
            glib::Propagation::Proceed
        });

        let win = Self {
            window,
            canvas,
            notebook_dock_box,
            whiteboard_dock_box,
            stack,
            dashboard,
        };
        win.sync_dock_mode();
        win
    }

    pub fn sync_dock_mode(&self) {
        let is_wb = self.canvas.document_format() == DocumentFormat::Whiteboard;
        self.notebook_dock_box.set_visible(!is_wb);
        self.whiteboard_dock_box.set_visible(is_wb);
    }

    pub fn canvas(&self) -> &Rc<NoteCanvas> {
        &self.canvas
    }

    pub fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    pub fn stack(&self) -> &gtk4::Stack {
        &self.stack
    }

    pub fn dashboard(&self) -> &Rc<crate::dashboard::UniNoteDashboard> {
        &self.dashboard
    }

    pub fn show_dashboard(&self) {
        self.canvas.persist_changes();
        self.dashboard.refresh();
        self.window.set_title(Some("UniNote — Hub Accademico"));
        self.stack.set_visible_child_name("dashboard");
    }

    pub fn show_editor(&self) {
        self.sync_dock_mode();
        let title = self.canvas.document_title();
        self.window.set_title(Some(&format!("UniNote — {}", title)));
        self.stack.set_visible_child_name("editor");
    }

    pub fn open_file(app: &Application, path: &std::path::Path) -> Self {
        let is_pdf = path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.eq_ignore_ascii_case("pdf"))
            .unwrap_or(false);

        if is_pdf {
            match Self::create_from_pdf(app, path) {
                Ok(win) => win,
                Err(e) => {
                    eprintln!(
                        "Errore durante l'apertura del PDF {}: {}",
                        path.display(),
                        e
                    );
                    Self::new(app)
                }
            }
        } else {
            let win = Self::new(app);
            let _ = win.canvas.open_file(path.to_path_buf());
            win.sync_dock_mode();
            win.canvas.center_page_1();
            win.show_editor();
            win
        }
    }

    pub fn create_document(
        app: &Application,
        title: &str,
        format: uninote_core::model::DocumentFormat,
        bg: uninote_core::model::PageBackground,
    ) -> Self {
        let win = Self::new(app);
        win.canvas.create_document(title, format, bg);
        win.sync_dock_mode();
        win.canvas.center_page_1();

        // Autosave: assegna file fisico predefinito nella cartella note e avvia StorageWorker
        let store = crate::preferences::DashboardRecentStore::load();
        let _ = store.ensure_default_notes_dir();
        let notes_dir = std::path::PathBuf::from(&store.default_notes_dir);
        let note_path = notes_dir.join(format!("{}.uninote", title));

        if let Err(e) = win.canvas.save_as(note_path.clone()) {
            eprintln!("Attenzione: impossibile inizializzare autosave: {}", e);
        } else {
            let mut store_update = crate::preferences::DashboardRecentStore::load();
            let meta = crate::preferences::RecentDocumentMeta {
                file_path: note_path.to_string_lossy().to_string(),
                title: title.to_string(),
                format,
                last_modified_timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                page_count: 1,
                thumbnail_path: None,
            };
            store_update.add_recent(meta);
            let _ = store_update.save();
        }

        win.show_editor();
        win
    }

    pub fn create_from_pdf(
        app: &Application,
        pdf_path: &std::path::Path,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let win = Self::new(app);
        win.canvas.create_from_pdf(pdf_path)?;
        win.sync_dock_mode();
        win.canvas.center_page_1();
        win.show_editor();
        Ok(win)
    }

    pub fn present(&self) {
        self.window.present();
    }
}
