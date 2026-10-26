//! Dedicated Test Suite for Milestone M3 (UX Impostazioni & Scheda GitHub)
//!
//! Scope:
//! - Switch "Opzioni Avanzate": stato predefinito disattivato, espansione dei gruppi
//!   tecnici di calibrazione fine e sincronizzazione tra le schede che lo replicano.
//! - Preset base di sensibilità dello stilo (Morbida/Media/Dura) e coerenza con la
//!   curva di pressione tecnica `PressureCurve`.
//! - Scheda GitHub: badge di versione, licenza GPL-3.0, collegamenti ufficiali e
//!   pulsante "Verifica Aggiornamenti".
//! - Retrocompatibilità del modello impostazioni con JSON preesistenti (campi opzionali).

#![allow(clippy::field_reassign_with_default)]

use gtk4::prelude::*;
use libadwaita::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use uninote_app::preferences::{FullAppSettingsModel, PressureCurve, StylusSensitivity};
use uninote_app::settings::UniNoteSettingsWindow;

// ============================================================================
// GTK TEST HARNESS (worker thread dedicato con main context GTK4)
// ============================================================================

struct GtkTestHarness {
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

static GTK_HARNESS: OnceLock<GtkTestHarness> = OnceLock::new();

/// Estrae un messaggio leggibile dal payload di un panic catturato.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else {
        "panic senza payload testuale".to_string()
    }
}

fn run_on_gtk_thread<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let harness = GTK_HARNESS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        thread::Builder::new()
            .name("gtk-settings-ux-test-worker".into())
            .spawn(move || {
                let _ = gtk4::init();
                while let Ok(task) = rx.recv() {
                    task();
                }
            })
            .expect("Failed to spawn GTK test worker thread");
        GtkTestHarness { sender: tx }
    });

    let (res_tx, res_rx) = mpsc::channel();
    harness
        .sender
        .send(Box::new(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            // Il panic avviene su un thread worker: libtest non ne cattura l'output,
            // quindi lo rendiamo esplicito su stderr e su file per la diagnostica CI.
            if let Err(payload) = &res {
                let message = panic_message(payload.as_ref());
                eprintln!("GTK-TEST-PANIC: {message}");
                let _ = std::fs::write("/tmp/uninote_gtk_test_panic.log", &message);
            }
            let _ = res_tx.send(res);
        }))
        .expect("GTK test worker thread is dead");

    match res_rx
        .recv()
        .expect("GTK test worker did not return a result")
    {
        Ok(val) => val,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

// ============================================================================
// TREE INSPECTION HELPERS
// ============================================================================

/// Raccoglie ricorsivamente l'intero albero di widget a partire dalla radice.
fn collect_widgets(root: &gtk4::Widget) -> Vec<gtk4::Widget> {
    let mut collected = Vec::new();
    let mut pending = vec![root.clone()];

    while let Some(widget) = pending.pop() {
        collected.push(widget.clone());
        let mut child = widget.first_child();
        while let Some(current) = child {
            pending.push(current.clone());
            child = current.next_sibling();
        }
    }

    collected
}

/// Normalizza il markup di un titolo Adwaita confrontabile tra versioni di libadwaita.
/// Alcune versioni restituiscono il titolo grezzo (`&amp;`), altre quello già decodificato (`&`).
fn normalized_title(title: &str) -> String {
    title
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

/// Cerca un `PreferencesGroup` tramite il proprio titolo.
fn find_group_by_title(root: &gtk4::Widget, title: &str) -> Option<libadwaita::PreferencesGroup> {
    let target = normalized_title(title);
    collect_widgets(root).into_iter().find_map(|widget| {
        widget
            .clone()
            .downcast::<libadwaita::PreferencesGroup>()
            .ok()
            .filter(|group| normalized_title(group.title().as_str()) == target)
    })
}

/// Cerca tutte le righe `SwitchRow` con il titolo indicato.
fn find_switch_rows_by_title(root: &gtk4::Widget, title: &str) -> Vec<libadwaita::SwitchRow> {
    let target = normalized_title(title);
    collect_widgets(root)
        .into_iter()
        .filter_map(|widget| {
            widget
                .clone()
                .downcast::<libadwaita::SwitchRow>()
                .ok()
                .filter(|row| normalized_title(row.title().as_str()) == target)
        })
        .collect()
}

/// Cerca tutte le righe `ActionRow` con il titolo indicato.
fn find_action_rows_by_title(root: &gtk4::Widget, title: &str) -> Vec<libadwaita::ActionRow> {
    let target = normalized_title(title);
    collect_widgets(root)
        .into_iter()
        .filter_map(|widget| {
            widget
                .clone()
                .downcast::<libadwaita::ActionRow>()
                .ok()
                .filter(|row| normalized_title(row.title().as_str()) == target)
        })
        .collect()
}

/// Legge la proprietà GObject "visible" del widget (indipendente dalla realizzazione).
fn visible_property(widget: &gtk4::Widget) -> bool {
    widget.property::<bool>("visible")
}

/// Converte un widget in widget generico per l'ispezione delle proprietà.
fn as_widget<W: IsA<gtk4::Widget>>(widget: &W) -> gtk4::Widget {
    widget.clone().upcast::<gtk4::Widget>()
}

/// Verifica la presenza di una stringa nel testo di una `Label` dell'albero.
fn tree_contains_label(root: &gtk4::Widget, needle: &str) -> bool {
    collect_widgets(root).into_iter().any(|widget| {
        widget
            .clone()
            .downcast::<gtk4::Label>()
            .map(|label| label.text().as_str() == needle)
            .unwrap_or(false)
    })
}

// ============================================================================
// TIER 1: SWITCH "OPZIONI AVANZATE" E ESPANSIONE DEI PARAMETRI TECNICI
// ============================================================================

#[test]
fn test_m3_advanced_switch_present_and_disabled_by_default() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings.clone());
        let root: gtk4::Widget = win.window().clone().upcast();

        let switches = find_switch_rows_by_title(&root, "Opzioni Avanzate");
        assert_eq!(
            switches.len(),
            2,
            "Lo switch 'Opzioni Avanzate' deve essere presente in testa alla scheda Hardware e alla scheda Strumenti"
        );

        for switch in &switches {
            assert!(
                !switch.is_active(),
                "Lo switch 'Opzioni Avanzate' deve essere disattivato per impostazione predefinita"
            );
        }

        assert!(
            !settings.borrow().advanced_options_enabled,
            "Il modello deve conservare la vista avanzata disattivata per impostazione predefinita"
        );
    });
}

#[test]
fn test_m3_advanced_toggle_expands_technical_calibration_groups() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings.clone());
        let root: gtk4::Widget = win.window().clone().upcast();

        const ADVANCED_GROUP_TITLES: [&str; 5] = [
            "Calibrazione Digitizer MPP Avanzata",
            "Filtro Adattivo 1€ (One Euro Filter)",
            "Filtri Anti-Hover e Anti-Blip",
            // I titoli dei gruppi Adwaita sono interpretati come markup: la sequenza
            // "&amp;" corrisponde all'ampersand visualizzato all'utente.
            "Protezione Palmo Avanzata (Timeout &amp; Cooldown)",
            "Diagnostica e Telemetria",
        ];

        // I gruppi tecnici devono essere collassati di default
        for title in ADVANCED_GROUP_TITLES {
            let group = find_group_by_title(&root, title)
                .unwrap_or_else(|| panic!("Gruppo avanzato '{title}' non trovato"));
            assert!(
                !visible_property(&as_widget(&group)),
                "Il gruppo '{title}' deve essere nascosto finché 'Opzioni Avanzate' non viene attivato"
            );
        }

        // Il gruppo base di configurazione rapida resta sempre visibile
        let quick_group = find_group_by_title(&root, "Configurazione Rapida")
            .expect("Gruppo base 'Configurazione Rapida' non trovato");
        assert!(
            visible_property(&as_widget(&quick_group)),
            "La vista base deve restare sempre visibile"
        );

        // Attivazione dello switch: i gruppi tecnici devono comparire
        let switches = find_switch_rows_by_title(&root, "Opzioni Avanzate");
        switches[0].set_active(true);

        assert!(
            settings.borrow().advanced_options_enabled,
            "L'attivazione dello switch deve essere persistita nel modello"
        );

        for title in ADVANCED_GROUP_TITLES {
            let group = find_group_by_title(&root, title)
                .unwrap_or_else(|| panic!("Gruppo avanzato '{title}' non trovato"));
            assert!(
                visible_property(&as_widget(&group)),
                "Il gruppo '{title}' deve comparire dopo l'attivazione di 'Opzioni Avanzate'"
            );
        }
    });
}

#[test]
fn test_m3_advanced_switch_synchronizes_across_pages() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings.clone());
        let root: gtk4::Widget = win.window().clone().upcast();

        let switches = find_switch_rows_by_title(&root, "Opzioni Avanzate");
        switches[0].set_active(true);

        for switch in &switches {
            assert!(
                switch.is_active(),
                "Tutti gli switch 'Opzioni Avanzate' devono riflettere lo stesso stato"
            );
        }

        // Disattivazione propagata a tutte le occorrenze
        switches[1].set_active(false);
        for switch in &switches {
            assert!(
                !switch.is_active(),
                "La disattivazione deve propagarsi a tutti gli switch 'Opzioni Avanzate'"
            );
        }

        assert!(
            !settings.borrow().advanced_options_enabled,
            "Il modello deve riflettere la disattivazione della vista avanzata"
        );
    });
}

#[test]
fn test_m3_advanced_state_survives_persistence_roundtrip() {
    let dir = std::env::temp_dir();
    let path = dir.join(format!(
        "uninote_settings_ux_advanced_{}_{}.json",
        std::process::id(),
        AtomicU64::new(1).fetch_add(1, Ordering::SeqCst)
    ));

    let mut model = FullAppSettingsModel::default();
    model.advanced_options_enabled = true;
    model
        .save_atomic(&path)
        .expect("Il salvataggio atomico deve riuscire");

    let loaded = FullAppSettingsModel::load_or_default(&path);
    assert!(
        loaded.advanced_options_enabled,
        "Lo stato della vista avanzata deve essere persistito"
    );

    let _ = std::fs::remove_file(&path);
}

// ============================================================================
// TIER 2: PRESET DI SENSIBILITÀ DELLO STILO
// ============================================================================

#[test]
fn test_m3_stylus_sensitivity_presets_default_and_mapping() {
    let model = FullAppSettingsModel::default();
    assert_eq!(
        model.stylus_sensitivity,
        StylusSensitivity::Medium,
        "Il preset predefinito deve essere 'Media'"
    );
    assert_eq!(
        model.pressure_curve,
        PressureCurve::Linear,
        "Il preset 'Media' deve corrispondere alla curva lineare"
    );

    assert_eq!(StylusSensitivity::Soft.curve(), PressureCurve::Soft);
    assert_eq!(StylusSensitivity::Medium.curve(), PressureCurve::Linear);
    assert_eq!(StylusSensitivity::Hard.curve(), PressureCurve::Hard);

    assert_eq!(StylusSensitivity::from_index(0), StylusSensitivity::Soft);
    assert_eq!(StylusSensitivity::from_index(1), StylusSensitivity::Medium);
    assert_eq!(StylusSensitivity::from_index(2), StylusSensitivity::Hard);
    assert_eq!(StylusSensitivity::from_index(99), StylusSensitivity::Medium);

    assert_eq!(
        StylusSensitivity::LABELS,
        ["Morbida", "Media", "Dura"],
        "Le etichette dei preset devono essere quelle previste dal design"
    );
}

#[test]
fn test_m3_stylus_sensitivity_keeps_curve_in_sync_both_ways() {
    let mut model = FullAppSettingsModel::default();

    model.set_stylus_sensitivity(StylusSensitivity::Soft);
    assert_eq!(model.pressure_curve, PressureCurve::Soft);

    model.set_stylus_sensitivity(StylusSensitivity::Hard);
    assert_eq!(model.pressure_curve, PressureCurve::Hard);

    model.set_pressure_curve(PressureCurve::Linear);
    assert_eq!(model.stylus_sensitivity, StylusSensitivity::Medium);

    model.set_pressure_curve(PressureCurve::Hard);
    assert_eq!(model.stylus_sensitivity, StylusSensitivity::Hard);
}

#[test]
fn test_m3_stylus_sensitivity_response_ordering_is_monotonic() {
    let raw = 0.4;
    let soft = StylusSensitivity::Soft.apply(raw);
    let medium = StylusSensitivity::Medium.apply(raw);
    let hard = StylusSensitivity::Hard.apply(raw);

    assert!(
        soft > medium && medium > hard,
        "La risposta dei preset deve essere monotona decrescente: {soft} > {medium} > {hard}"
    );
}

// ============================================================================
// TIER 3: SCHEDA GITHUB
// ============================================================================

#[test]
fn test_m3_settings_window_exposes_five_preference_pages() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings);
        assert_eq!(
            win.settings_page_titles(),
            vec![
                "Aspetto DMS",
                "GitHub",
                "Hardware MPP",
                "Scorciatoie & Stylus",
                "Strumenti",
            ],
            "La finestra impostazioni deve esporre cinque schede (Hardware, Aspetto, Strumenti, Scorciatoie, GitHub)"
        );
    });
}

#[test]
fn test_m3_github_tab_official_links_and_license() {
    assert_eq!(
        uninote_app::GITHUB_REPO_URL,
        "https://github.com/Loreexe/UniNote"
    );
    assert_eq!(
        uninote_app::GITHUB_NEW_ISSUE_URL,
        "https://github.com/Loreexe/UniNote/issues/new"
    );
    assert_eq!(
        uninote_app::GITHUB_RELEASES_URL,
        "https://github.com/Loreexe/UniNote/releases"
    );
    assert_eq!(
        uninote_app::GPL_3_0_LICENSE_URL,
        "https://www.gnu.org/licenses/gpl-3.0.html"
    );
}

#[test]
fn test_m3_github_tab_rows_are_present_in_widget_tree() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings);
        let root: gtk4::Widget = win.window().clone().upcast();

        for title in [
            "Repository Ufficiale",
            "Segnala un Bug o Feedback",
            "Note di Rilascio / Changelog",
            "Licenza Open Source",
            "Verifica Aggiornamenti",
        ] {
            let rows = find_action_rows_by_title(&root, title);
            assert_eq!(
                rows.len(),
                1,
                "La riga '{title}' deve essere presente nella scheda GitHub"
            );
        }

        let license_row = find_action_rows_by_title(&root, "Licenza Open Source")
            .pop()
            .expect("Riga licenza presente");
        assert!(
            license_row
                .subtitle()
                .map(|text| text.as_str().to_string())
                .unwrap_or_default()
                .contains("GNU General Public License v3.0"),
            "La licenza GPL-3.0 deve essere dichiarata esplicitamente"
        );
    });
}

#[test]
fn test_m3_github_tab_shows_current_version_badge() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings);
        let root: gtk4::Widget = win.window().clone().upcast();

        assert_eq!(uninote_app::APP_VERSION, "0.4.7");
        assert_eq!(uninote_app::APP_VERSION_LABEL, "v0.4.7");
        assert!(
            tree_contains_label(&root, "v0.4.7"),
            "Il badge di versione v0.4.7 deve essere presente nella scheda GitHub"
        );
    });
}

#[test]
fn test_m3_github_tab_update_check_button_is_activatable() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings);
        let root: gtk4::Widget = win.window().clone().upcast();

        let row = find_action_rows_by_title(&root, "Verifica Aggiornamenti")
            .pop()
            .expect("Riga di verifica aggiornamenti presente");

        let has_button = collect_widgets(&row.upcast::<gtk4::Widget>())
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk4::Button>().ok())
            .any(|button| {
                button
                    .label()
                    .map(|text| text.as_str() == "Verifica Aggiornamenti")
                    .unwrap_or(false)
            });

        assert!(
            has_button,
            "Il pulsante 'Verifica Aggiornamenti' deve essere presente e attivabile"
        );
    });
}

// ============================================================================
// TIER 4: RETROCOMPATIBILITÀ E SANITIZZAZIONE DEL MODELLO
// ============================================================================

#[test]
fn test_m3_settings_model_loads_legacy_json_without_new_fields() {
    let path = std::env::temp_dir().join(format!(
        "uninote_settings_ux_legacy_{}_{}.json",
        std::process::id(),
        AtomicU64::new(1).fetch_add(1, Ordering::SeqCst)
    ));

    // JSON di una versione precedente, privo dei campi introdotti dalla vista avanzata
    let legacy = r#"{
        "min_contact_pressure": 0.06,
        "pressure_curve": "Soft",
        "anti_hover_blip_threshold": 0.07,
        "one_euro_filter_enabled": true,
        "one_euro_cutoff_hz": 1.2,
        "palm_proximity_timeout_ms": 350,
        "palm_cooldown_ms": 250,
        "hud_telemetry_enabled": false,
        "theme_mode": "System",
        "adapt_ink_rec709": true,
        "dark_sheet_ruling_contrast": 0.2,
        "floating_bars_opacity": 0.9,
        "orthogonal_snap_tolerance_deg": 8.0,
        "rdp_epsilon_factor": 0.04,
        "shortcuts": {},
        "barrel_button_action": "InstantEraser"
    }"#;

    std::fs::write(&path, legacy).expect("Scrittura JSON legacy");
    let loaded = FullAppSettingsModel::load_or_default(&path);

    assert_eq!(loaded.min_contact_pressure, 0.06);
    assert_eq!(loaded.pressure_curve, PressureCurve::Soft);
    assert_eq!(
        loaded.stylus_sensitivity,
        StylusSensitivity::Soft,
        "Il preset deve essere ricostruito in modo coerente con la curva salvata"
    );
    assert!(!loaded.advanced_options_enabled);
    assert!(loaded.palm_rejection_enabled);
    assert!(loaded.anti_hover_enabled);
    assert_eq!(loaded.one_euro_beta, 0.04);
    assert_eq!(loaded.stylus_lift_off_cutoff, 0.045);

    let _ = std::fs::remove_file(&path);
}

#[test]
fn test_m3_sanitize_clamps_new_advanced_fields() {
    let mut model = FullAppSettingsModel::default();

    model.one_euro_beta = f64::NAN;
    model.stylus_lift_off_cutoff = f64::INFINITY;
    model.sanitize();

    assert_eq!(model.one_euro_beta, 0.04);
    assert_eq!(model.stylus_lift_off_cutoff, 0.045);

    model.one_euro_beta = 5.0;
    model.stylus_lift_off_cutoff = -1.0;
    model.sanitize();

    assert_eq!(model.one_euro_beta, 1.0);
    assert_eq!(model.stylus_lift_off_cutoff, 0.01);
}

/// Trait di comodo per rendere leggibile l'uso del modello nei test.
pub trait UniNoteSettingsWindowExt {
    /// Restituisce i titoli delle schede presenti nella finestra preferenze.
    fn settings_page_titles(&self) -> Vec<String>;
}

impl UniNoteSettingsWindowExt for UniNoteSettingsWindow {
    fn settings_page_titles(&self) -> Vec<String> {
        let mut titles: Vec<String> = collect_widgets(&self.window().clone().upcast())
            .into_iter()
            .filter_map(|widget| {
                widget
                    .clone()
                    .downcast::<libadwaita::PreferencesPage>()
                    .ok()
                    .map(|page| page.title().as_str().to_string())
            })
            .filter(|title| !title.is_empty())
            .collect();
        titles.sort();
        titles
    }
}
