use crate::icons::{create_icon, IconKind};
use crate::preferences::{
    BarrelButtonAction, FullAppSettingsModel, PressureCurve, StylusSensitivity, ThemeMode,
};
use gtk4::cairo::Context;
use gtk4::gdk;
use gtk4::gio;
use gtk4::gio::prelude::FileExtManual;
use gtk4::prelude::*;
use gtk4::{
    Adjustment, Align, Box as GtkBox, Button, DrawingArea, Entry, GestureStylus, Label,
    Orientation, ProgressBar, Scale, SpinButton, Switch,
};
use libadwaita::prelude::*;
use libadwaita::{
    ActionRow, ComboRow, PreferencesGroup, PreferencesPage, PreferencesWindow, SwitchRow,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub type SettingsChangedCallback = Rc<RefCell<Option<Box<dyn Fn(&FullAppSettingsModel)>>>>;

/// Versione corrente dell'applicazione mostrata nella scheda GitHub.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Etichetta versione con prefisso `v` (usata nel badge del titolo e nella scheda GitHub).
pub const APP_VERSION_LABEL: &str = concat!("v", env!("CARGO_PKG_VERSION"));

/// URL del repository ufficiale UniNote.
pub const GITHUB_REPO_URL: &str = "https://github.com/Loreexe/UniNote";
/// URL per l'apertura di una nuova segnalazione (bug o feedback).
pub const GITHUB_NEW_ISSUE_URL: &str = "https://github.com/Loreexe/UniNote/issues/new";
/// URL delle note di rilascio ufficiali.
pub const GITHUB_RELEASES_URL: &str = "https://github.com/Loreexe/UniNote/releases";
/// URL del testo integrale della licenza GNU General Public License v3.0.
pub const GPL_3_0_LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";
/// Endpoint pubblico interrogato dal pulsante "Verifica Aggiornamenti".
const GITHUB_LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/Loreexe/UniNote/releases/latest";

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

// ============================================================================
// STATO CONDIVISO DELLA VISTA AVANZATA
// ============================================================================

/// Callback di visibilità registrato dai gruppi tecnici della vista avanzata.
type AdvancedVisibilityHook = Rc<dyn Fn(bool)>;

/// Stato condiviso della modalità "Opzioni Avanzate" tra le schede della finestra.
///
/// Lo switch "Opzioni Avanzate" è replicato in testa alle schede che espongono
/// parametri tecnici: tutte le occorrenze riflettono e aggiornano lo stesso stato.
#[derive(Clone)]
struct AdvancedState {
    enabled: Rc<Cell<bool>>,
    syncing: Rc<Cell<bool>>,
    switches: Rc<RefCell<Vec<SwitchRow>>>,
    hooks: Rc<RefCell<Vec<AdvancedVisibilityHook>>>,
}

impl AdvancedState {
    /// Crea lo stato inizializzandolo dal valore persistito nel modello.
    fn new(initial: bool) -> Self {
        Self {
            enabled: Rc::new(Cell::new(initial)),
            syncing: Rc::new(Cell::new(false)),
            switches: Rc::new(RefCell::new(Vec::new())),
            hooks: Rc::new(RefCell::new(Vec::new())),
        }
    }

    /// Restituisce `true` se la vista avanzata è attiva.
    fn is_enabled(&self) -> bool {
        self.enabled.get()
    }

    /// Restituisce `true` se lo stato è in fase di propagazione (per evitare cicli).
    fn is_syncing(&self) -> bool {
        self.syncing.get()
    }

    /// Registra uno switch "Opzioni Avanzate" da tenere allineato allo stato.
    fn register_switch(&self, row: SwitchRow) {
        self.switches.borrow_mut().push(row);
    }

    /// Registra un callback di visibilità per i gruppi tecnici.
    fn register_visibility_hook(&self, hook: AdvancedVisibilityHook) {
        self.hooks.borrow_mut().push(hook);
    }

    /// Propaga il nuovo stato a tutti gli switch registrati e ai gruppi tecnici.
    fn set_enabled(&self, value: bool) {
        self.enabled.set(value);

        let switches: Vec<SwitchRow> = self.switches.borrow().clone();
        self.syncing.set(true);
        for row in switches {
            row.set_active(value);
        }
        self.syncing.set(false);

        let hooks: Vec<AdvancedVisibilityHook> = self.hooks.borrow().clone();
        for hook in hooks {
            hook(value);
        }
    }
}

// ============================================================================
// FUNZIONI DI SUPPORTO
// ============================================================================

/// Traduce una curva di pressione tecnica nell'indice del selettore avanzato.
fn curve_index(curve: PressureCurve) -> u32 {
    match curve {
        PressureCurve::Linear => 0,
        PressureCurve::Soft => 1,
        PressureCurve::Hard => 2,
    }
}

/// Traduce l'indice del selettore avanzato in una curva di pressione tecnica.
fn curve_from_index(index: u32) -> PressureCurve {
    match index {
        1 => PressureCurve::Soft,
        2 => PressureCurve::Hard,
        _ => PressureCurve::Linear,
    }
}

/// Crea un pulsante collegamento che apre un URI esterno con il gestore predefinito.
fn link_button(label: &str, uri: &str) -> Button {
    let button = Button::builder()
        .label(label)
        .valign(Align::Center)
        .css_classes(["dms-action-btn"])
        .build();

    let uri = uri.to_string();
    button.connect_clicked(move |_| {
        if let Err(err) = gio::AppInfo::launch_default_for_uri(&uri, None::<&gio::AppLaunchContext>)
        {
            // Nessun gestore predefinito per lo schema URI: la pagina resta comunque
            // consultabile incollando l'indirizzo riportato nel sottotitolo della riga.
            eprintln!("UniNote: impossibile aprire l'URI '{uri}': {err}");
        }
    });

    button
}

/// Crea una riga `AdwActionRow` con pulsante collegamento verso un URI esterno.
fn link_row(title: &str, subtitle: &str, uri: &str, button_label: &str) -> ActionRow {
    let row = ActionRow::builder().title(title).subtitle(subtitle).build();
    let button = link_button(button_label, uri);
    row.add_suffix(&button);
    row.set_activatable_widget(Some(&button));
    row
}

/// Costruisce lo switch "Opzioni Avanzate" replicato in testa alle schede tecniche.
fn build_advanced_switch(
    settings: &Rc<RefCell<FullAppSettingsModel>>,
    on_changed: &SettingsChangedCallback,
    advanced: &AdvancedState,
) -> SwitchRow {
    let row = SwitchRow::builder()
        .title("Opzioni Avanzate")
        .subtitle(
            "Mostra i parametri tecnici di calibrazione fine del digitizer (soglie, filtri 1€, anti-hover, palm rejection).",
        )
        .active(advanced.is_enabled())
        .build();

    let adv_state = advanced.clone();
    let s_adv = settings.clone();
    let cb_adv = on_changed.clone();
    row.connect_active_notify(move |switch_row| {
        if adv_state.is_syncing() {
            return;
        }
        let active = switch_row.is_active();
        adv_state.set_enabled(active);
        s_adv.borrow_mut().advanced_options_enabled = active;
        let _ = s_adv.borrow().save();
        if let Some(ref cb) = *cb_adv.borrow() {
            cb(&s_adv.borrow());
        }
    });

    advanced.register_switch(row.clone());
    row
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
        let advanced = AdvancedState::new(settings.borrow().advanced_options_enabled);

        let page_hardware = Self::build_hardware_page(&settings, &on_changed, &advanced);
        let page_appearance = Self::build_appearance_page(&settings, &on_changed);
        let page_tools = Self::build_tools_page(&settings, &on_changed, &advanced);
        let page_shortcuts = Self::build_shortcuts_page(&settings, &on_changed);
        let page_github = Self::build_github_page();

        window.add(&page_hardware);
        window.add(&page_appearance);
        window.add(&page_tools);
        window.add(&page_shortcuts);
        window.add(&page_github);

        if !libadwaita::StyleManager::default().is_dark() {
            window.add_css_class("light-theme");
        }

        let win_theme_sync = window.clone();
        libadwaita::StyleManager::default().connect_dark_notify(move |sm| {
            if sm.is_dark() {
                win_theme_sync.remove_css_class("light-theme");
            } else {
                win_theme_sync.add_css_class("light-theme");
            }
        });

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
        advanced: &AdvancedState,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Hardware MPP")
            .icon_name("input-tablet-symbolic")
            .build();

        // ---------------------------------------------------------------------
        // GRUPPO 0: VISTA BASE + SWITCH "OPZIONI AVANZATE"
        // ---------------------------------------------------------------------
        let grp_quick = PreferencesGroup::builder()
            .title("Configurazione Rapida")
            .description(
                "Preset intuitivi per la scrittura a mano. Attiva 'Opzioni Avanzate' per i parametri tecnici di calibrazione fine.",
            )
            .build();

        let row_advanced = build_advanced_switch(settings, on_changed, advanced);
        grp_quick.add(&row_advanced);

        // 1.1 Preset di Sensibilità dello Stilo (Morbida / Media / Dura)
        let row_sensitivity = ComboRow::builder()
            .title("Sensibilità dello Stilo")
            .subtitle("Scegli un preset immediato: la curva di pressione viene calibrata automaticamente.")
            .model(&gtk4::StringList::new(&StylusSensitivity::LABELS))
            .build();

        row_sensitivity.set_selected(settings.borrow().stylus_sensitivity.index());

        // 1.2 Curva di Pressione Tecnica (vista avanzata, allineata al preset base)
        let row_curve = ComboRow::builder()
            .title("Curva di Risposta alla Pressione")
            .subtitle("Curva tecnica grezza applicata alla pressione grezza del digitizer.")
            .model(&gtk4::StringList::new(&[
                "Lineare (Naturale)",
                "Morbida (Alta Sensibilità)",
                "Dura (Tratto Marcato)",
            ]))
            .build();

        row_curve.set_selected(curve_index(settings.borrow().pressure_curve));

        // Sincronizzazione bidirezionale tra preset base e curva tecnica
        let sync_guard = Rc::new(Cell::new(false));

        let curve_row = row_curve.clone();
        let s_sens = settings.clone();
        let cb_sens = on_changed.clone();
        let guard_sens = sync_guard.clone();
        row_sensitivity.connect_selected_notify(move |combo| {
            if guard_sens.get() {
                return;
            }
            guard_sens.set(true);

            let sensitivity = StylusSensitivity::from_index(combo.selected());
            s_sens.borrow_mut().set_stylus_sensitivity(sensitivity);
            curve_row.set_selected(curve_index(sensitivity.curve()));

            let _ = s_sens.borrow().save();
            if let Some(ref cb) = *cb_sens.borrow() {
                cb(&s_sens.borrow());
            }

            guard_sens.set(false);
        });

        let sens_row = row_sensitivity.clone();
        let s_curve = settings.clone();
        let cb_curve = on_changed.clone();
        let guard_curve = sync_guard.clone();
        row_curve.connect_selected_notify(move |combo| {
            if guard_curve.get() {
                return;
            }
            guard_curve.set(true);

            let curve = curve_from_index(combo.selected());
            s_curve.borrow_mut().set_pressure_curve(curve);
            sens_row.set_selected(StylusSensitivity::from_curve(curve).index());

            let _ = s_curve.borrow().save();
            if let Some(ref cb) = *cb_curve.borrow() {
                cb(&s_curve.borrow());
            }

            guard_curve.set(false);
        });

        grp_quick.add(&row_sensitivity);

        // 1.3 Barra di Prova Live Pressione
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
        let s_sens_down = s_curve_for_live.clone();
        stylus_ctrl.connect_down(move |gesture, _x, _y| {
            let raw_p = gesture
                .axis(gdk::AxisUse::Pressure)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let sensitivity = s_sens_down.borrow().stylus_sensitivity;
            let calibrated = sensitivity.apply(raw_p);
            p_bar_down.set_fraction(calibrated);
            p_bar_down.set_text(Some(&format!("{:.3}", calibrated)));
        });

        let p_bar_motion = p_bar.clone();
        let s_sens_motion = s_curve_for_live.clone();
        stylus_ctrl.connect_motion(move |gesture, _x, _y| {
            let raw_p = gesture
                .axis(gdk::AxisUse::Pressure)
                .unwrap_or(0.0)
                .clamp(0.0, 1.0);
            let sensitivity = s_sens_motion.borrow().stylus_sensitivity;
            let calibrated = sensitivity.apply(raw_p);
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
        grp_quick.add(&row_live);

        // 1.4 Protezione Palmo (toggle accessibile nella vista base)
        let row_palm_toggle = SwitchRow::builder()
            .title("Protezione Palmo (Palm Rejection)")
            .subtitle("Ignora il contatto della mano sul touchscreen mentre scrivi con lo stilo.")
            .active(settings.borrow().palm_rejection_enabled)
            .build();

        let s_palm_toggle = settings.clone();
        let cb_palm_toggle = on_changed.clone();
        row_palm_toggle.connect_active_notify(move |switch_row| {
            let active = switch_row.is_active();
            s_palm_toggle.borrow_mut().palm_rejection_enabled = active;
            let _ = s_palm_toggle.borrow().save();
            if let Some(ref cb) = *cb_palm_toggle.borrow() {
                cb(&s_palm_toggle.borrow());
            }
        });
        grp_quick.add(&row_palm_toggle);

        page.add(&grp_quick);

        // ---------------------------------------------------------------------
        // GRUPPO 1 (AVANZATO): CALIBRAZIONE DIGITIZER
        // ---------------------------------------------------------------------
        let grp_digitizer = PreferencesGroup::builder()
            .title("Calibrazione Digitizer MPP Avanzata")
            .description("Soglie numeriche di contatto, cutoff di distacco stilo e curva grezza.")
            .build();

        // Soglia numerica di pressione di contatto
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
        grp_digitizer.add(&row_threshold);

        // Cutoff di distacco stilo dal vetro
        let adj_lift_off = Adjustment::new(
            settings.borrow().stylus_lift_off_cutoff,
            0.01,
            0.20,
            0.005,
            0.01,
            0.0,
        );
        let row_lift_off = ActionRow::builder()
            .title("Cutoff di Distacco Stilo")
            .subtitle("Pressione sotto la quale il contatto viene considerato staccato dal vetro.")
            .build();

        let spin_lift_off = SpinButton::builder()
            .adjustment(&adj_lift_off)
            .digits(3)
            .climb_rate(0.005)
            .valign(Align::Center)
            .build();

        let s_lift_off = settings.clone();
        let cb_lift_off = on_changed.clone();
        spin_lift_off.connect_value_changed(move |spin| {
            let val = spin.value();
            s_lift_off.borrow_mut().stylus_lift_off_cutoff = val;
            let _ = s_lift_off.borrow().save();
            if let Some(ref cb) = *cb_lift_off.borrow() {
                cb(&s_lift_off.borrow());
            }
        });
        row_lift_off.add_suffix(&spin_lift_off);
        grp_digitizer.add(&row_lift_off);

        // Curva di pressione tecnica
        grp_digitizer.add(&row_curve);
        page.add(&grp_digitizer);

        // ---------------------------------------------------------------------
        // GRUPPO 2 (AVANZATO): FILTRO ADATTIVO 1 EURO
        // ---------------------------------------------------------------------
        let grp_one_euro = PreferencesGroup::builder()
            .title("Filtro Adattivo 1€ (One Euro Filter)")
            .description("Smoothing dinamico adattivo per azzerare jitter e tremolio.")
            .build();

        let row_one_euro = SwitchRow::builder()
            .title("Filtro Adattivo 1€ (One Euro Filter)")
            .subtitle("Smoothing dinamico adattivo per azzerare jitter e tremolio.")
            .active(settings.borrow().one_euro_filter_enabled)
            .build();

        let s_oe = settings.clone();
        let cb_oe = on_changed.clone();
        row_one_euro.connect_active_notify(move |switch_row| {
            let active = switch_row.is_active();
            s_oe.borrow_mut().one_euro_filter_enabled = active;
            let _ = s_oe.borrow().save();
            if let Some(ref cb) = *cb_oe.borrow() {
                cb(&s_oe.borrow());
            }
        });
        grp_one_euro.add(&row_one_euro);

        // Cutoff Frequenza 1€
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
        grp_one_euro.add(&row_oe_cutoff);

        // Coefficiente di velocità beta
        let adj_oe_beta =
            Adjustment::new(settings.borrow().one_euro_beta, 0.0, 1.0, 0.005, 0.01, 0.0);
        let row_oe_beta = ActionRow::builder()
            .title("Coefficiente di Velocità β (1€)")
            .subtitle(
                "Adattamento del cutoff alla velocità del tratto: valori alti riducono il ritardo.",
            )
            .build();

        let spin_beta = SpinButton::builder()
            .adjustment(&adj_oe_beta)
            .digits(3)
            .climb_rate(0.005)
            .valign(Align::Center)
            .build();

        let s_beta = settings.clone();
        let cb_beta = on_changed.clone();
        spin_beta.connect_value_changed(move |spin| {
            let val = spin.value();
            s_beta.borrow_mut().one_euro_beta = val;
            let _ = s_beta.borrow().save();
            if let Some(ref cb) = *cb_beta.borrow() {
                cb(&s_beta.borrow());
            }
        });
        row_oe_beta.add_suffix(&spin_beta);
        grp_one_euro.add(&row_oe_beta);
        page.add(&grp_one_euro);

        // ---------------------------------------------------------------------
        // GRUPPO 3 (AVANZATO): FILTRI ANTI-HOVER E ANTI-BLIP
        // ---------------------------------------------------------------------
        let grp_filters = PreferencesGroup::builder()
            .title("Filtri Anti-Hover e Anti-Blip")
            .description("Protezione da campioni spuri capacitivi ed eliminazione spike.")
            .build();

        let row_anti_hover = SwitchRow::builder()
            .title("Filtro Anti-Hover MPP")
            .subtitle(
                "Scarta i campioni in aria a pressione sub-soglia durante l'hover dello stilo.",
            )
            .active(settings.borrow().anti_hover_enabled)
            .build();

        let s_ah = settings.clone();
        let cb_ah = on_changed.clone();
        row_anti_hover.connect_active_notify(move |switch_row| {
            let active = switch_row.is_active();
            s_ah.borrow_mut().anti_hover_enabled = active;
            let _ = s_ah.borrow().save();
            if let Some(ref cb) = *cb_ah.borrow() {
                cb(&s_ah.borrow());
            }
        });
        grp_filters.add(&row_anti_hover);

        // Soglia Anti-Blip
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
        page.add(&grp_filters);

        // ---------------------------------------------------------------------
        // GRUPPO 4 (AVANZATO): PROTEZIONE PALMO AVANZATA
        // ---------------------------------------------------------------------
        let grp_palm = PreferencesGroup::builder()
            .title("Protezione Palmo Avanzata (Timeout &amp; Cooldown)")
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
        page.add(&grp_palm);

        // ---------------------------------------------------------------------
        // GRUPPO 5 (AVANZATO): TELEMETRIA DIAGNOSTICA
        // ---------------------------------------------------------------------
        let grp_diagnostics = PreferencesGroup::builder()
            .title("Diagnostica e Telemetria")
            .description("Overlay di diagnostica a schermo per il monitoraggio del digitizer.")
            .build();

        let row_hud = SwitchRow::builder()
            .title("Overlay Telemetria Diagnostica (F12)")
            .subtitle("Mostra frequenza Hz, tempo rendering e statistiche hardware a schermo.")
            .active(settings.borrow().hud_telemetry_enabled)
            .build();

        let s_hud = settings.clone();
        let cb_hud = on_changed.clone();
        row_hud.connect_active_notify(move |switch_row| {
            let val = switch_row.is_active();
            s_hud.borrow_mut().hud_telemetry_enabled = val;
            let _ = s_hud.borrow().save();
            if let Some(ref cb) = *cb_hud.borrow() {
                cb(&s_hud.borrow());
            }
        });
        grp_diagnostics.add(&row_hud);
        page.add(&grp_diagnostics);

        // ---------------------------------------------------------------------
        // VISIBILITÀ DEI GRUPPI TECNICI (espansa solo al toggle)
        // ---------------------------------------------------------------------
        let advanced_groups = [
            grp_digitizer.clone(),
            grp_one_euro.clone(),
            grp_filters.clone(),
            grp_palm.clone(),
            grp_diagnostics.clone(),
        ];

        for group in &advanced_groups {
            group.set_visible(advanced.is_enabled());
        }

        advanced.register_visibility_hook(Rc::new(move |visible| {
            for group in &advanced_groups {
                group.set_visible(visible);
            }
        }));

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

        let row_theme = ActionRow::builder()
            .title("Modalità Tema")
            .subtitle("Seleziona il tema dell'interfaccia: Sistema, Chiaro o Scuro.")
            .build();

        let slider_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(4)
            .valign(Align::Center)
            .css_classes(["dms-theme-slider"])
            .build();

        let btn_system = Button::builder()
            .icon_name("display-symbolic")
            .tooltip_text("Sincronizza con Sistema (Computer)")
            .css_classes(["dms-theme-btn"])
            .build();

        let btn_light = Button::builder()
            .icon_name("weather-clear-symbolic")
            .tooltip_text("Forza Tema Chiaro Naturale (Sole)")
            .css_classes(["dms-theme-btn"])
            .build();

        let btn_dark = Button::builder()
            .icon_name("weather-clear-night-symbolic")
            .tooltip_text("Forza Tema Scuro OLED (Luna)")
            .css_classes(["dms-theme-btn"])
            .build();

        let update_pill_classes = {
            let b_sys = btn_system.clone();
            let b_light = btn_light.clone();
            let b_dark = btn_dark.clone();
            move |mode: ThemeMode| {
                b_sys.remove_css_class("active");
                b_light.remove_css_class("active");
                b_dark.remove_css_class("active");
                match mode {
                    ThemeMode::System => b_sys.add_css_class("active"),
                    ThemeMode::ForceLight => b_light.add_css_class("active"),
                    ThemeMode::ForceDarkOled => b_dark.add_css_class("active"),
                }
            }
        };

        let initial_mode = settings.borrow().theme_mode;
        update_pill_classes(initial_mode);

        let apply_theme = {
            let s = settings.clone();
            let cb = on_changed.clone();
            let upd = update_pill_classes.clone();
            move |mode: ThemeMode| {
                s.borrow_mut().theme_mode = mode;
                let _ = s.borrow().save();
                upd(mode);

                let sm = libadwaita::StyleManager::default();
                match mode {
                    ThemeMode::System => sm.set_color_scheme(libadwaita::ColorScheme::Default),
                    ThemeMode::ForceLight => {
                        sm.set_color_scheme(libadwaita::ColorScheme::ForceLight)
                    }
                    ThemeMode::ForceDarkOled => {
                        sm.set_color_scheme(libadwaita::ColorScheme::ForceDark)
                    }
                }

                if let Some(ref callback) = *cb.borrow() {
                    callback(&s.borrow());
                }
            }
        };

        let apply_sys = apply_theme.clone();
        btn_system.connect_clicked(move |_| {
            apply_sys(ThemeMode::System);
        });

        let apply_lt = apply_theme.clone();
        btn_light.connect_clicked(move |_| {
            apply_lt(ThemeMode::ForceLight);
        });

        let apply_dk = apply_theme.clone();
        btn_dark.connect_clicked(move |_| {
            apply_dk(ThemeMode::ForceDarkOled);
        });

        slider_box.append(&btn_system);
        slider_box.append(&btn_light);
        slider_box.append(&btn_dark);

        row_theme.add_suffix(&slider_box);
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
        advanced: &AdvancedState,
    ) -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("Strumenti")
            .icon_name("draw-polygon-symbolic")
            .build();

        let grp_switch = PreferencesGroup::builder()
            .title("Configurazione Rapida")
            .description(
                "Attiva 'Opzioni Avanzate' per mostrare le tolleranze geometriche di calibrazione.",
            )
            .build();

        let row_advanced = build_advanced_switch(settings, on_changed, advanced);
        grp_switch.add(&row_advanced);
        page.add(&grp_switch);

        let grp_shapes = PreferencesGroup::builder()
            .title("Tolleranze Geometriche Poligoni (RDP &amp; Snap)")
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

        // Il gruppo delle tolleranze geometriche è visibile solo in modalità avanzata
        grp_shapes.set_visible(advanced.is_enabled());
        let shapes_group = grp_shapes.clone();
        advanced.register_visibility_hook(Rc::new(move |visible| {
            shapes_group.set_visible(visible);
        }));

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

    // =========================================================================
    // SCHEDA 5: GITHUB, LICENZA E AGGIORNAMENTI
    // =========================================================================
    fn build_github_page() -> PreferencesPage {
        let page = PreferencesPage::builder()
            .title("GitHub")
            .icon_name("applications-internet-symbolic")
            .build();

        // ---------------------------------------------------------------------
        // GRUPPO 1: BADGE PROGETTO
        // ---------------------------------------------------------------------
        let grp_project = PreferencesGroup::builder()
            .title("UniNote")
            .description(
                "Suite accademica Linux nativa per appunti a mano libera con stilo (MPP/ELAN).",
            )
            .build();

        let row_badge = ActionRow::builder()
            .title("UniNote")
            .subtitle(format!("Versione {}", APP_VERSION_LABEL))
            .build();

        let app_logo = create_icon(IconKind::AppLogo, 40, None);
        row_badge.add_prefix(&app_logo);

        let ver_label = Label::builder()
            .label(APP_VERSION_LABEL)
            .valign(Align::Center)
            .css_classes(["dms-thick-chip"])
            .build();
        row_badge.add_suffix(&ver_label);
        grp_project.add(&row_badge);

        let row_license = link_row(
            "Licenza Open Source",
            "GNU General Public License v3.0 (GPL-3.0)",
            GPL_3_0_LICENSE_URL,
            "Apri",
        );
        grp_project.add(&row_license);
        page.add(&grp_project);

        // ---------------------------------------------------------------------
        // GRUPPO 2: COLLEGAMENTI UFFICIALI
        // ---------------------------------------------------------------------
        let grp_links = PreferencesGroup::builder()
            .title("Collegamenti Ufficiali")
            .description(
                "Risorse del progetto su GitHub: sorgenti, segnalazioni e note di rilascio.",
            )
            .build();

        let row_repo = link_row(
            "Repository Ufficiale",
            GITHUB_REPO_URL,
            GITHUB_REPO_URL,
            "Apri",
        );
        grp_links.add(&row_repo);

        let row_issue = link_row(
            "Segnala un Bug o Feedback",
            GITHUB_NEW_ISSUE_URL,
            GITHUB_NEW_ISSUE_URL,
            "Segnala",
        );
        grp_links.add(&row_issue);

        let row_releases = link_row(
            "Note di Rilascio / Changelog",
            GITHUB_RELEASES_URL,
            GITHUB_RELEASES_URL,
            "Apri",
        );
        grp_links.add(&row_releases);
        page.add(&grp_links);

        // ---------------------------------------------------------------------
        // GRUPPO 3: VERIFICA AGGIORNAMENTI
        // ---------------------------------------------------------------------
        let grp_updates = PreferencesGroup::builder()
            .title("Aggiornamenti")
            .description("Verifica in modo sicuro l'ultima release pubblicata su GitHub.")
            .build();

        let row_check = ActionRow::builder()
            .title("Verifica Aggiornamenti")
            .subtitle(format!(
                "Interroga l'API pubblica di GitHub e confronta il risultato con la versione installata ({}).",
                APP_VERSION_LABEL
            ))
            .build();

        let btn_check = Button::builder()
            .label("Verifica Aggiornamenti")
            .valign(Align::Center)
            .css_classes(["dms-action-btn"])
            .build();

        row_check.add_suffix(&btn_check);
        grp_updates.add(&row_check);

        let row_status = ActionRow::builder()
            .title("Stato della Verifica")
            .subtitle("Nessuna verifica eseguita.")
            .build();
        grp_updates.add(&row_status);

        let status_row = row_status.clone();
        btn_check.connect_clicked(move |_| {
            status_row.set_subtitle("Verifica in corso…");

            let file = gio::File::for_uri(GITHUB_LATEST_RELEASE_API);
            let status_row = status_row.clone();
            file.load_contents_async(None::<&gio::Cancellable>, move |result| {
                let message = match result {
                    Ok((contents, _etag)) => {
                        let body = String::from_utf8_lossy(&contents).to_string();
                        match serde_json::from_str::<serde_json::Value>(&body) {
                            Ok(json) => {
                                let tag = json
                                    .get("tag_name")
                                    .and_then(|value| value.as_str())
                                    .unwrap_or_default()
                                    .trim_start_matches('v')
                                    .to_string();

                                if tag.is_empty() {
                                    "Risposta inattesa dal server GitHub: apri le Note di Rilascio per la verifica manuale.".to_string()
                                } else if tag == APP_VERSION {
                                    format!(
                                        "Sei aggiornato: l'ultima release pubblicata è v{}.",
                                        tag
                                    )
                                } else {
                                    format!(
                                        "Nuova release disponibile: v{} (versione installata: v{}).",
                                        tag, APP_VERSION
                                    )
                                }
                            }
                            Err(err) => format!(
                                "Risposta di GitHub non interpretabile ({}). Apri le Note di Rilascio.",
                                err
                            ),
                        }
                    }
                    Err(err) => format!(
                        "Impossibile contattare GitHub ({}). Apri le Note di Rilascio per la verifica manuale.",
                        err
                    ),
                };

                status_row.set_subtitle(&message);
            });
        });

        page.add(&grp_updates);

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

    /// Imposta la finestra genitore per il rendering modale/transiente.
    pub fn set_transient_for<P: glib::object::IsA<gtk4::Window>>(&self, parent: Option<&P>) {
        self.window.set_transient_for(parent);
    }

    /// Imposta se la finestra è modale.
    pub fn set_modal(&self, modal: bool) {
        self.window.set_modal(modal);
    }

    /// Connette un callback invocato ad ogni variazione dei parametri con persistenza atomica.
    pub fn connect_changed<F>(&self, callback: F)
    where
        F: Fn(&FullAppSettingsModel) + 'static,
    {
        *self.on_changed.borrow_mut() = Some(Box::new(callback));
    }
}
