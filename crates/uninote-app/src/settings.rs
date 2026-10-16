use crate::preferences::{BarrelButtonAction, FullAppSettingsModel, PressureCurve, ThemeMode};
use gtk4::cairo::Context;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, Box as GtkBox, Button, DrawingArea, Entry, GestureStylus, Orientation,
    ProgressBar, Scale, SpinButton, Switch,
};
use libadwaita::prelude::*;
use libadwaita::{ActionRow, ComboRow, PreferencesGroup, PreferencesPage, PreferencesWindow};
use std::cell::RefCell;
use std::rc::Rc;

pub type SettingsChangedCallback = Rc<RefCell<Option<Box<dyn Fn(&FullAppSettingsModel)>>>>;

/// Finestra preferenze nativa Adwaita / DMS multi-scheda.
pub struct UniNoteSettingsWindow {
    window: PreferencesWindow,
    settings: Rc<RefCell<FullAppSettingsModel>>,
    on_changed: SettingsChangedCallback,
}

impl Default for UniNoteSettingsWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl UniNoteSettingsWindow {
    /// Crea una nuova finestra impostazioni caricando le preferenze salvate o i default.
    pub fn new() -> Self {
        Self::with_settings(Rc::new(RefCell::new(FullAppSettingsModel::load())))
    }

    /// Crea la finestra impostazioni associata al modello di configurazione fornito.
    pub fn with_settings(settings: Rc<RefCell<FullAppSettingsModel>>) -> Self {
        let window = PreferencesWindow::builder()
            .title("Impostazioni UniNote")
            .default_width(780)
            .default_height(640)
            .css_classes(["uninote-settings-window"])
            .build();

        let on_changed = Rc::new(RefCell::new(None));

        let page_hardware = Self::build_hardware_page(&settings, &on_changed);
        let page_appearance = Self::build_appearance_page(&settings, &on_changed);
        let page_tools = Self::build_tools_page(&settings, &on_changed);
        let page_shortcuts = Self::build_shortcuts_page(&settings, &on_changed);

        window.add(&page_hardware);
        window.add(&page_appearance);
        window.add(&page_tools);
        window.add(&page_shortcuts);

        Self {
            window,
            settings,
            on_changed,
        }
    }

    // =========================================================================
    // SCHEDA 1: HARDWARE MPP & DIGITIZER
    // =========================================================================
    fn build_hardware_page(
        settings: &Rc<RefCell<FullAppSettingsModel>>,
        on_changed: &SettingsChangedCallback,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Hardware MPP")
            .icon_name("input-tablet-symbolic")
            .build();

        // Gruppo 1: Calibrazione Pressione e Contatto
        let grp_pressure = PreferencesGroup::builder()
            .title("Calibrazione Digitizer Microsoft Pen Protocol (MPP)")
            .description(
                "Regola la soglia di contatto fisico con il vetro e la curva di risposta dinamica.",
            )
            .build();

        // 1.1 Soglia Minima di Contatto
        let row_threshold = ActionRow::builder()
            .title("Soglia Minima di Contatto")
            .subtitle("Pressione minima per avviare il tratto inchiostro ed evitare spike in aria.")
            .build();

        let initial_thresh = settings.borrow().min_contact_pressure;
        let adj_thresh = Adjustment::new(initial_thresh, 0.01, 0.50, 0.005, 0.05, 0.0);
        let scale_thresh = Scale::builder()
            .adjustment(&adj_thresh)
            .digits(3)
            .draw_value(true)
            .value_pos(gtk4::PositionType::Right)
            .width_request(180)
            .build();

        let s_thresh = settings.clone();
        let cb_thresh = on_changed.clone();
        scale_thresh.connect_value_changed(move |scale| {
            let val = scale.value();
            s_thresh.borrow_mut().min_contact_pressure = val;
            let _ = s_thresh.borrow().save();
            if let Some(ref cb) = *cb_thresh.borrow() {
                cb(&s_thresh.borrow());
            }
        });
        row_threshold.add_suffix(&scale_thresh);
        grp_pressure.add(&row_threshold);

        // 1.2 Barra di Prova Live Pressione
        let row_live = ActionRow::builder()
            .title("Test Pressione dal Vivo")
            .subtitle("Premi con lo stilo su quest'area per verificare la risposta del digitizer.")
            .build();

        let progress_live = ProgressBar::builder()
            .fraction(0.0)
            .show_text(true)
            .text("0.000")
            .width_request(180)
            .css_classes(["dms-pressure-bar"])
            .build();

        let live_area = DrawingArea::builder()
            .width_request(180)
            .height_request(28)
            .css_classes(["dms-pressure-test-area"])
            .build();

        let p_bar = progress_live.clone();
        let s_curve_for_live = settings.clone();
        live_area.set_draw_func(move |_, cr: &Context, w, h| {
            cr.set_source_rgb(0.090, 0.129, 0.149);
            cr.rectangle(0.0, 0.0, w as f64, h as f64);
            let _ = cr.fill();
        });

        let stylus_ctrl = GestureStylus::new();

        let p_bar_down = p_bar.clone();
        let s_curve_down = s_curve_for_live.clone();
        stylus_ctrl.connect_down(move |gesture, _x, _y| {
            let raw_p = gesture
                .axis(gdk::AxisUse::Pressure)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let curve = s_curve_down.borrow().pressure_curve;
            let calibrated = curve.apply(raw_p);
            p_bar_down.set_fraction(calibrated);
            p_bar_down.set_text(Some(&format!("{:.3}", calibrated)));
        });

        let p_bar_motion = p_bar.clone();
        let s_curve_motion = s_curve_for_live.clone();
        stylus_ctrl.connect_motion(move |gesture, _x, _y| {
            let raw_p = gesture
                .axis(gdk::AxisUse::Pressure)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let curve = s_curve_motion.borrow().pressure_curve;
            let calibrated = curve.apply(raw_p);
            p_bar_motion.set_fraction(calibrated);
            p_bar_motion.set_text(Some(&format!("{:.3}", calibrated)));
        });

        let p_bar_up = p_bar.clone();
        stylus_ctrl.connect_up(move |_gesture, _x, _y| {
            p_bar_up.set_fraction(0.0);
            p_bar_up.set_text(Some("0.000"));
        });

        live_area.add_controller(stylus_ctrl);

        let live_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();
        live_box.append(&live_area);
        live_box.append(&progress_live);
        row_live.add_suffix(&live_box);
        grp_pressure.add(&row_live);

        // 1.3 Curva di Sensibilità
        let curve_strings = [
            "Lineare (Naturale)",
            "Morbida (Alta Sensibilità)",
            "Dura (Tratto Marcato)",
        ];
        let row_curve = ComboRow::builder()
            .title("Curva di Risposta alla Pressione")
            .subtitle("Adatta l'erogazione dello spessore alla forza della tua mano.")
            .model(&gtk4::StringList::new(&curve_strings))
            .build();

        let initial_curve_idx = match settings.borrow().pressure_curve {
            PressureCurve::Linear => 0,
            PressureCurve::Soft => 1,
            PressureCurve::Hard => 2,
        };
        row_curve.set_selected(initial_curve_idx);

        let s_curve = settings.clone();
        let cb_curve = on_changed.clone();
        row_curve.connect_selected_notify(move |combo| {
            let curve = match combo.selected() {
                1 => PressureCurve::Soft,
                2 => PressureCurve::Hard,
                _ => PressureCurve::Linear,
            };
            s_curve.borrow_mut().pressure_curve = curve;
            let _ = s_curve.borrow().save();
            if let Some(ref cb) = *cb_curve.borrow() {
                cb(&s_curve.borrow());
            }
        });
        grp_pressure.add(&row_curve);
        page.add(&grp_pressure);

        // Gruppo 2: Filtri Anti-Hover & Anti-Blip
        let grp_filters = PreferencesGroup::builder()
            .title("Filtri Hardware e Stabilizzazione")
            .description("Protezione da campioni spuri capacitivi ed eliminazione spike.")
            .build();

        // 2.1 Soglia Anti-Hover & Anti-Blip
        let row_blip = ActionRow::builder()
            .title("Soglia Filtro Anti-Blip")
            .subtitle(
                "Scarta campioni capacitivi transitori privi di reale pressione (default 0.065).",
            )
            .build();

        let initial_blip = settings.borrow().anti_hover_blip_threshold;
        let adj_blip = Adjustment::new(initial_blip, 0.02, 0.20, 0.005, 0.02, 0.0);
        let scale_blip = Scale::builder()
            .adjustment(&adj_blip)
            .digits(3)
            .draw_value(true)
            .value_pos(gtk4::PositionType::Right)
            .width_request(180)
            .build();

        let s_blip = settings.clone();
        let cb_blip = on_changed.clone();
        scale_blip.connect_value_changed(move |scale| {
            let val = scale.value();
            s_blip.borrow_mut().anti_hover_blip_threshold = val;
            let _ = s_blip.borrow().save();
            if let Some(ref cb) = *cb_blip.borrow() {
                cb(&s_blip.borrow());
            }
        });
        row_blip.add_suffix(&scale_blip);
        grp_filters.add(&row_blip);

        // 2.2 One Euro Filter (Toggle)
        let row_one_euro = ActionRow::builder()
            .title("Filtro Adattivo 1€ (One Euro Filter)")
            .subtitle("Smoothing dinamico adattivo per azzerare jitter e tremolio.")
            .build();

        let switch_oe = Switch::builder()
            .active(settings.borrow().one_euro_filter_enabled)
            .valign(Align::Center)
            .build();

        let s_oe = settings.clone();
        let cb_oe = on_changed.clone();
        switch_oe.connect_active_notify(move |switch| {
            let active = switch.is_active();
            s_oe.borrow_mut().one_euro_filter_enabled = active;
            let _ = s_oe.borrow().save();
            if let Some(ref cb) = *cb_oe.borrow() {
                cb(&s_oe.borrow());
            }
        });
        row_one_euro.add_suffix(&switch_oe);
        row_one_euro.set_activatable_widget(Some(&switch_oe));
        grp_filters.add(&row_one_euro);

        // 2.3 Cutoff Frequenza 1€
        let adj_oe_cutoff = Adjustment::new(
            settings.borrow().one_euro_cutoff_hz,
            0.1,
            10.0,
            0.1,
            1.0,
            0.0,
        );
        let row_oe_cutoff = ActionRow::builder()
            .title("Cutoff Frequenza 1€ (Hz)")
            .subtitle("Frequenza di taglio per il filtraggio del rumore ad alta velocità.")
            .build();

        let spin_oe = SpinButton::builder()
            .adjustment(&adj_oe_cutoff)
            .digits(2)
            .climb_rate(0.1)
            .valign(Align::Center)
            .build();

        let s_cutoff = settings.clone();
        let cb_cutoff = on_changed.clone();
        spin_oe.connect_value_changed(move |spin| {
            let val = spin.value();
            s_cutoff.borrow_mut().one_euro_cutoff_hz = val;
            let _ = s_cutoff.borrow().save();
            if let Some(ref cb) = *cb_cutoff.borrow() {
                cb(&s_cutoff.borrow());
            }
        });
        row_oe_cutoff.add_suffix(&spin_oe);
        grp_filters.add(&row_oe_cutoff);
        page.add(&grp_filters);

        // Gruppo 3: Riconoscimento Palmo (Palm Rejection)
        let grp_palm = PreferencesGroup::builder()
            .title("Protezione Palmo (Palm Rejection)")
            .description("Parametri temporali della macchina a stati anti-palmo su touchscreen.")
            .build();

        let adj_palm_timeout = Adjustment::new(
            settings.borrow().palm_proximity_timeout_ms as f64,
            50.0,
            2000.0,
            50.0,
            100.0,
            0.0,
        );
        let row_palm_timeout = ActionRow::builder()
            .title("Timeout Auto-Scadenza Prossimità (ms)")
            .subtitle(
                "Durata massima dell'inibizione touch dopo il distacco stilo (default 300ms).",
            )
            .build();

        let spin_pt = SpinButton::builder()
            .adjustment(&adj_palm_timeout)
            .digits(0)
            .valign(Align::Center)
            .build();

        let s_pt = settings.clone();
        let cb_pt = on_changed.clone();
        spin_pt.connect_value_changed(move |spin| {
            let val = spin.value() as u64;
            s_pt.borrow_mut().palm_proximity_timeout_ms = val;
            let _ = s_pt.borrow().save();
            if let Some(ref cb) = *cb_pt.borrow() {
                cb(&s_pt.borrow());
            }
        });
        row_palm_timeout.add_suffix(&spin_pt);
        grp_palm.add(&row_palm_timeout);

        let adj_cooldown = Adjustment::new(
            settings.borrow().palm_cooldown_ms as f64,
            50.0,
            1000.0,
            25.0,
            50.0,
            0.0,
        );
        let row_cooldown = ActionRow::builder()
            .title("Durata Cooldown Palmo (ms)")
            .subtitle(
                "Intervallo di guardia prima di riabilitare il tocco multitouch (default 200ms).",
            )
            .build();

        let spin_cd = SpinButton::builder()
            .adjustment(&adj_cooldown)
            .digits(0)
            .valign(Align::Center)
            .build();

        let s_cd = settings.clone();
        let cb_cd = on_changed.clone();
        spin_cd.connect_value_changed(move |spin| {
            let val = spin.value() as u64;
            s_cd.borrow_mut().palm_cooldown_ms = val;
            let _ = s_cd.borrow().save();
            if let Some(ref cb) = *cb_cd.borrow() {
                cb(&s_cd.borrow());
            }
        });
        row_cooldown.add_suffix(&spin_cd);
        grp_palm.add(&row_cooldown);

        // 3.3 HUD Telemetria
        let row_hud = ActionRow::builder()
            .title("Overlay Telemetria Diagnostica (F12)")
            .subtitle("Mostra frequenza Hz, tempo rendering e statistiche hardware a schermo.")
            .build();

        let switch_hud = Switch::builder()
            .active(settings.borrow().hud_telemetry_enabled)
            .valign(Align::Center)
            .build();

        let s_hud = settings.clone();
        let cb_hud = on_changed.clone();
        switch_hud.connect_active_notify(move |switch| {
            let val = switch.is_active();
            s_hud.borrow_mut().hud_telemetry_enabled = val;
            let _ = s_hud.borrow().save();
            if let Some(ref cb) = *cb_hud.borrow() {
                cb(&s_hud.borrow());
            }
        });
        row_hud.add_suffix(&switch_hud);
        row_hud.set_activatable_widget(Some(&switch_hud));
        grp_palm.add(&row_hud);
        page.add(&grp_palm);

        page
    }

    // =========================================================================
    // SCHEDA 2: ASPETTO, TEMA E DMS
    // =========================================================================
    fn build_appearance_page(
        settings: &Rc<RefCell<FullAppSettingsModel>>,
        on_changed: &SettingsChangedCallback,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Aspetto DMS")
            .icon_name("preferences-desktop-theme-symbolic")
            .build();

        let grp_theme = PreferencesGroup::builder()
            .title("Design System DMS e Modalità Tema")
            .description(
                "Personalizza la tavolozza colori e l'adattamento ottico al tema di sistema.",
            )
            .build();

        let theme_strings = [
            "Sincronizza con Desktop Wayland/Niri",
            "Forza Tema Scuro OLED (#0A151A)",
            "Forza Tema Chiaro Naturale",
        ];
        let row_theme = ComboRow::builder()
            .title("Modalità Tema")
            .subtitle("Seleziona il tema dell'interfaccia e dei fogli accademici.")
            .model(&gtk4::StringList::new(&theme_strings))
            .build();

        let initial_theme_idx = match settings.borrow().theme_mode {
            ThemeMode::System => 0,
            ThemeMode::ForceDarkOled => 1,
            ThemeMode::ForceLight => 2,
        };
        row_theme.set_selected(initial_theme_idx);

        let s_theme = settings.clone();
        let cb_theme = on_changed.clone();
        row_theme.connect_selected_notify(move |combo| {
            let mode = match combo.selected() {
                1 => ThemeMode::ForceDarkOled,
                2 => ThemeMode::ForceLight,
                _ => ThemeMode::System,
            };
            s_theme.borrow_mut().theme_mode = mode;
            let _ = s_theme.borrow().save();
            if let Some(ref cb) = *cb_theme.borrow() {
                cb(&s_theme.borrow());
            }
        });
        grp_theme.add(&row_theme);

        // Adattamento Inchiostri Rec. 709
        let row_rec709 = ActionRow::builder()
            .title("Adattamento Percettivo Inchiostri (Rec. 709)")
            .subtitle("Eleva automaticamente la luminosità dei tratti scuri quando si scrive su fogli scuri.")
            .build();

        let switch_rec = Switch::builder()
            .active(settings.borrow().adapt_ink_rec709)
            .valign(Align::Center)
            .build();

        let s_rec = settings.clone();
        let cb_rec = on_changed.clone();
        switch_rec.connect_active_notify(move |switch| {
            let active = switch.is_active();
            s_rec.borrow_mut().adapt_ink_rec709 = active;
            let _ = s_rec.borrow().save();
            if let Some(ref cb) = *cb_rec.borrow() {
                cb(&s_rec.borrow());
            }
        });
        row_rec709.add_suffix(&switch_rec);
        row_rec709.set_activatable_widget(Some(&switch_rec));
        grp_theme.add(&row_rec709);

        // Contrasto Rigature Fogli Scuri
        let row_contrast = ActionRow::builder()
            .title("Contrasto Rigature e Griglie su Fogli Scuri")
            .subtitle("Calibrazione visiva dell'intensità di righe e quadretti su sfondo scuro.")
            .build();

        let adj_contrast = Adjustment::new(
            settings.borrow().dark_sheet_ruling_contrast,
            0.05,
            0.50,
            0.01,
            0.05,
            0.0,
        );
        let scale_contrast = Scale::builder()
            .adjustment(&adj_contrast)
            .digits(2)
            .draw_value(true)
            .value_pos(gtk4::PositionType::Right)
            .width_request(180)
            .build();

        let s_contrast = settings.clone();
        let cb_contrast = on_changed.clone();
        scale_contrast.connect_value_changed(move |scale| {
            let val = scale.value();
            s_contrast.borrow_mut().dark_sheet_ruling_contrast = val;
            let _ = s_contrast.borrow().save();
            if let Some(ref cb) = *cb_contrast.borrow() {
                cb(&s_contrast.borrow());
            }
        });
        row_contrast.add_suffix(&scale_contrast);
        grp_theme.add(&row_contrast);

        // Opacità Barre Flottanti DMS
        let row_opacity = ActionRow::builder()
            .title("Opacità Barre Flottanti DMS")
            .subtitle("Trasparenza ed elevazione delle isole degli strumenti a schermo.")
            .build();

        let adj_opacity = Adjustment::new(
            settings.borrow().floating_bars_opacity,
            0.50,
            1.00,
            0.01,
            0.05,
            0.0,
        );
        let scale_opacity = Scale::builder()
            .adjustment(&adj_opacity)
            .digits(2)
            .draw_value(true)
            .value_pos(gtk4::PositionType::Right)
            .width_request(180)
            .build();

        let s_opacity = settings.clone();
        let cb_opacity = on_changed.clone();
        scale_opacity.connect_value_changed(move |scale| {
            let val = scale.value();
            s_opacity.borrow_mut().floating_bars_opacity = val;
            let _ = s_opacity.borrow().save();
            if let Some(ref cb) = *cb_opacity.borrow() {
                cb(&s_opacity.borrow());
            }
        });
        row_opacity.add_suffix(&scale_opacity);
        grp_theme.add(&row_opacity);
        page.add(&grp_theme);

        page
    }

    // =========================================================================
    // SCHEDA 3: STRUMENTI & RICONOSCIMENTO FORME
    // =========================================================================
    fn build_tools_page(
        settings: &Rc<RefCell<FullAppSettingsModel>>,
        on_changed: &SettingsChangedCallback,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Strumenti")
            .icon_name("draw-polygon-symbolic")
            .build();

        let grp_shapes = PreferencesGroup::builder()
            .title("Riconoscimento Geometrico Forme")
            .description(
                "Tolleranze per la sintesi vettoriale di linee rette, rettangoli e poligoni.",
            )
            .build();

        // 3.1 Snap Ortogonale Linee
        let adj_ortho = Adjustment::new(
            settings.borrow().orthogonal_snap_tolerance_deg,
            2.0,
            25.0,
            1.0,
            5.0,
            0.0,
        );
        let row_ortho = ActionRow::builder()
            .title("Tolleranza Snap Ortogonale (°)")
            .subtitle("Angolo massimo per raddrizzare automaticamente linee quasi orizzontali o verticali.")
            .build();

        let spin_ortho = SpinButton::builder()
            .adjustment(&adj_ortho)
            .digits(1)
            .valign(Align::Center)
            .build();

        let s_ortho = settings.clone();
        let cb_ortho = on_changed.clone();
        spin_ortho.connect_value_changed(move |spin| {
            let val = spin.value();
            s_ortho.borrow_mut().orthogonal_snap_tolerance_deg = val;
            let _ = s_ortho.borrow().save();
            if let Some(ref cb) = *cb_ortho.borrow() {
                cb(&s_ortho.borrow());
            }
        });
        row_ortho.add_suffix(&spin_ortho);
        grp_shapes.add(&row_ortho);

        // 3.2 Tolleranza Ramer-Douglas-Peucker
        let adj_rdp = Adjustment::new(
            settings.borrow().rdp_epsilon_factor,
            0.01,
            0.15,
            0.005,
            0.02,
            0.0,
        );
        let row_rdp = ActionRow::builder()
            .title("Fattore Epsilon RDP Poligoni")
            .subtitle(
                "Sensibilità della semplificazione poligonale scalata sulla diagonale del tratto.",
            )
            .build();

        let spin_rdp = SpinButton::builder()
            .adjustment(&adj_rdp)
            .digits(3)
            .climb_rate(0.005)
            .valign(Align::Center)
            .build();

        let s_rdp = settings.clone();
        let cb_rdp = on_changed.clone();
        spin_rdp.connect_value_changed(move |spin| {
            let val = spin.value();
            s_rdp.borrow_mut().rdp_epsilon_factor = val;
            let _ = s_rdp.borrow().save();
            if let Some(ref cb) = *cb_rdp.borrow() {
                cb(&s_rdp.borrow());
            }
        });
        row_rdp.add_suffix(&spin_rdp);
        grp_shapes.add(&row_rdp);
        page.add(&grp_shapes);

        // Gruppo Preset Dock Penne
        let grp_dock = PreferencesGroup::builder()
            .title("Preset e Palette Dock Penne")
            .description("Gestione e ripristino rapido della tavolozza colori predefinita.")
            .build();

        let row_reset_colors = ActionRow::builder()
            .title("Colori Inchiostro di Default")
            .subtitle(
                "Ripristina i 5 colori accademici ufficiali (Ciano, Bianco, Corallo, Menta, Oro).",
            )
            .build();

        let btn_reset_colors = Button::builder()
            .label("Ripristina Predefiniti")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();

        let s_colors = settings.clone();
        let cb_colors = on_changed.clone();
        btn_reset_colors.connect_clicked(move |_| {
            // Ripristina genuinamente i 5 slot di default in AppPreferences e persiste su preferences.json
            let mut prefs = crate::preferences::AppPreferences::load();
            prefs.slots = crate::preferences::AppPreferences::default().slots;
            prefs.active_slot_index = 0;
            let _ = prefs.save();

            // Salva contestualmente le impostazioni generali e notifica i callback
            let _ = s_colors.borrow().save();
            if let Some(ref cb) = *cb_colors.borrow() {
                cb(&s_colors.borrow());
            }
        });
        row_reset_colors.add_suffix(&btn_reset_colors);
        grp_dock.add(&row_reset_colors);
        page.add(&grp_dock);

        page
    }

    // =========================================================================
    // SCHEDA 4: SCORCIATOIE DA TASTIERA & GESTI
    // =========================================================================
    fn build_shortcuts_page(
        settings: &Rc<RefCell<FullAppSettingsModel>>,
        on_changed: &SettingsChangedCallback,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Scorciatoie & Stylus")
            .icon_name("input-keyboard-symbolic")
            .build();

        let grp_barrel = PreferencesGroup::builder()
            .title("Pulsanti Barilotto Stilo MPP")
            .description("Assegna la funzione rapida richiamabile premendo il pulsante laterale della penna.")
            .build();

        let barrel_strings = [
            "Gomma Istantanea al Tocco",
            "Strumento Selezione Lazo",
            "Selettore Colore Inchiostro",
        ];
        let row_barrel = ComboRow::builder()
            .title("Pulsante Barilotto Primario")
            .subtitle("Comportamento quando la punta tocca il vetro con il pulsante premuto.")
            .model(&gtk4::StringList::new(&barrel_strings))
            .build();

        let initial_barrel_idx = match settings.borrow().barrel_button_action {
            BarrelButtonAction::InstantEraser => 0,
            BarrelButtonAction::LassoSelect => 1,
            BarrelButtonAction::ColorPicker => 2,
        };
        row_barrel.set_selected(initial_barrel_idx);

        let s_barrel = settings.clone();
        let cb_barrel = on_changed.clone();
        row_barrel.connect_selected_notify(move |combo| {
            let action = match combo.selected() {
                1 => BarrelButtonAction::LassoSelect,
                2 => BarrelButtonAction::ColorPicker,
                _ => BarrelButtonAction::InstantEraser,
            };
            s_barrel.borrow_mut().barrel_button_action = action;
            let _ = s_barrel.borrow().save();
            if let Some(ref cb) = *cb_barrel.borrow() {
                cb(&s_barrel.borrow());
            }
        });
        grp_barrel.add(&row_barrel);
        page.add(&grp_barrel);

        let grp_keys = PreferencesGroup::builder()
            .title("Tabella Scorciatoie da Tastiera")
            .description(
                "Configurazione delle scorciatoie native attive durante lo studio e l'annotazione.",
            )
            .build();

        // Pulsante globale di ripristino scorciatoie
        let row_reset_all = ActionRow::builder()
            .title("Ripristina Tutte le Scorciatoie")
            .subtitle("Ripristina le combinazioni di tasti native predefinite della suite.")
            .build();

        let btn_reset_all = Button::builder()
            .label("Ripristina Default")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();

        let mut entry_refs = Vec::new();

        for &(action_id, desc, subtitle, default_val) in FullAppSettingsModel::SHORTCUT_SPECS {
            let current_val = settings.borrow().get_shortcut(action_id);

            let row = ActionRow::builder().title(desc).subtitle(subtitle).build();

            let entry = Entry::builder()
                .text(&current_val)
                .valign(Align::Center)
                .width_chars(16)
                .css_classes(["dms-entry", "dms-shortcut-entry"])
                .build();

            let s_entry = settings.clone();
            let cb_entry = on_changed.clone();
            let act_id = action_id.to_string();
            entry.connect_changed(move |e| {
                let text = e.text().to_string();
                s_entry.borrow_mut().set_shortcut(&act_id, &text);
                let _ = s_entry.borrow().save();
                if let Some(ref cb) = *cb_entry.borrow() {
                    cb(&s_entry.borrow());
                }
            });

            // Pulsante di ripristino per la singola riga
            let btn_reset_single = Button::builder()
                .label("↺")
                .tooltip_text("Ripristina combinazione predefinita")
                .css_classes(["dms-btn-icon-round", "flat"])
                .valign(Align::Center)
                .build();

            let s_single = settings.clone();
            let cb_single = on_changed.clone();
            let act_id_single = action_id.to_string();
            let entry_single = entry.clone();
            let def_val = default_val.to_string();
            btn_reset_single.connect_clicked(move |_| {
                s_single.borrow_mut().reset_shortcut(&act_id_single);
                entry_single.set_text(&def_val);
                let _ = s_single.borrow().save();
                if let Some(ref cb) = *cb_single.borrow() {
                    cb(&s_single.borrow());
                }
            });

            let box_suffix = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .valign(Align::Center)
                .build();
            box_suffix.append(&entry);
            box_suffix.append(&btn_reset_single);

            row.add_suffix(&box_suffix);
            grp_keys.add(&row);

            entry_refs.push((entry, default_val));
        }

        let s_all = settings.clone();
        let cb_all = on_changed.clone();
        btn_reset_all.connect_clicked(move |_| {
            s_all.borrow_mut().reset_all_shortcuts();
            let _ = s_all.borrow().save();
            for (entry, def) in &entry_refs {
                entry.set_text(def);
            }
            if let Some(ref cb) = *cb_all.borrow() {
                cb(&s_all.borrow());
            }
        });

        row_reset_all.add_suffix(&btn_reset_all);
        grp_keys.add(&row_reset_all);

        page.add(&grp_keys);
        page
    }

    /// Presenta la finestra a schermo.
    pub fn present(&self) {
        self.window.present();
    }

    /// Restituisce il widget AdwPreferencesWindow sottostante.
    pub fn window(&self) -> &PreferencesWindow {
        &self.window
    }

    /// Restituisce il riferimento condiviso al modello di impostazioni.
    pub fn settings(&self) -> Rc<RefCell<FullAppSettingsModel>> {
        self.settings.clone()
    }

    /// Connette un callback invocato ad ogni variazione dei parametri con persistenza atomica.
    pub fn connect_changed<F>(&self, callback: F)
    where
        F: Fn(&FullAppSettingsModel) + 'static,
    {
        *self.on_changed.borrow_mut() = Some(Box::new(callback));
    }
}
