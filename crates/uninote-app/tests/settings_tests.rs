//! Dedicated Settings Window Opaque-Box Test Suite (Requirement R3)
//!
//! Scope:
//! - Multi-tab `AdwPreferencesWindow` settings data model
//! - Tab 1 (Hardware MPP): pressure threshold (0.05), sensitivity curves, anti-hover/blip (0.065), 1€ filter, palm timeout (300ms)
//! - Tab 2 (Appearance DMS): Wayland desktop sync, dark OLED (#0A151A), light theme, Rec. 709 ink adaptation
//! - Tab 3 (Tools & Shapes): orthogonal line snap tolerance (8.0 deg), RDP polygon tolerances, pen presets
//! - Tab 4 (Shortcuts & Stylus): configurable shortcuts map (Ctrl+Z, Ctrl+Y, Ctrl+S, Ctrl+O, Ctrl+N, Ctrl+H, F12, Ctrl+0)
//! - Atomic JSON persistence via PID-indexed temporary file and graceful fallback on corrupted config

#![allow(clippy::field_reassign_with_default)]

use gtk4::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use uninote_app::preferences::{
    AppPreferences, BarrelButtonAction, FullAppSettingsModel, PenSlotConfig, PressureCurve,
    ThemeMode,
};
use uninote_app::settings::UniNoteSettingsWindow;

// ============================================================================
// REFERENCE ORACLES & DATA MODELS FOR SETTINGS WINDOW
// ============================================================================

static SETTINGS_COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_settings_temp_path(prefix: &str) -> PathBuf {
    let id = SETTINGS_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_settings_{}_{}_{}.json",
        prefix,
        std::process::id(),
        id
    ));
    path
}

struct GtkTestHarness {
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

static GTK_HARNESS: OnceLock<GtkTestHarness> = OnceLock::new();

fn run_on_gtk_thread<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let harness = GTK_HARNESS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        thread::Builder::new()
            .name("gtk-settings-test-worker".into())
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
// TIER 1: FEATURE COVERAGE (>=5 TESTS)
// ============================================================================

#[test]
fn test_t1_r3_hardware_mpp_default_parameters() {
    let s = FullAppSettingsModel::default();
    assert_eq!(s.min_contact_pressure, 0.05);
    assert_eq!(s.anti_hover_blip_threshold, 0.065);
    assert_eq!(s.palm_proximity_timeout_ms, 300);
    assert_eq!(s.palm_cooldown_ms, 200);
    assert!(s.one_euro_filter_enabled);
    assert_eq!(s.pressure_curve, PressureCurve::Linear);
    assert!(!s.hud_telemetry_enabled);
}

#[test]
fn test_t1_r3_pressure_curve_mapping_functions() {
    let p_mid = 0.5;

    let linear_val = PressureCurve::Linear.apply(p_mid);
    assert_eq!(linear_val, 0.5);

    let soft_val = PressureCurve::Soft.apply(p_mid);
    assert!(
        soft_val > linear_val,
        "Soft curve must elevate mid-pressures for light hands: got {}",
        soft_val
    );

    let hard_val = PressureCurve::Hard.apply(p_mid);
    assert!(
        hard_val < linear_val,
        "Hard curve must suppress mid-pressures for heavy hands: got {}",
        hard_val
    );
}

#[test]
fn test_t1_r3_appearance_theme_options() {
    let mut s = FullAppSettingsModel::default();
    assert_eq!(s.theme_mode, ThemeMode::System);
    assert!(s.adapt_ink_rec709);

    s.theme_mode = ThemeMode::ForceDarkOled;
    assert_eq!(s.theme_mode, ThemeMode::ForceDarkOled);

    s.theme_mode = ThemeMode::ForceLight;
    assert_eq!(s.theme_mode, ThemeMode::ForceLight);
}

#[test]
fn test_t1_r3_shortcuts_native_mapping() {
    let s = FullAppSettingsModel::default();
    assert_eq!(s.shortcuts.get("undo"), Some(&"<Primary>z".to_string()));
    assert_eq!(s.shortcuts.get("redo"), Some(&"<Primary>y".to_string()));
    assert_eq!(s.shortcuts.get("save"), Some(&"<Primary>s".to_string()));
    assert_eq!(s.shortcuts.get("open"), Some(&"<Primary>o".to_string()));
    assert_eq!(s.shortcuts.get("new"), Some(&"<Primary>n".to_string()));
    assert_eq!(s.shortcuts.get("home"), Some(&"<Primary>h".to_string()));
    assert_eq!(s.shortcuts.get("telemetry"), Some(&"F12".to_string()));
    assert_eq!(
        s.shortcuts.get("zoom_reset"),
        Some(&"<Primary>0".to_string())
    );
}

#[test]
fn test_t1_r3_atomic_persistence_save_and_load() {
    let temp_file = unique_settings_temp_path("save_load");
    let mut original = FullAppSettingsModel::default();
    original.min_contact_pressure = 0.08;
    original.set_pressure_curve(PressureCurve::Soft);
    original.theme_mode = ThemeMode::ForceDarkOled;
    original.barrel_button_action = BarrelButtonAction::LassoSelect;

    original
        .save_atomic(&temp_file)
        .expect("Atomic save must succeed");
    let loaded = FullAppSettingsModel::load_or_default(&temp_file);

    assert_eq!(original, loaded);
    let _ = fs::remove_file(temp_file);
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 TESTS)
// ============================================================================

#[test]
fn test_t2_r3_pressure_threshold_clamping_bounds() {
    let mut s = FullAppSettingsModel::default();
    s.min_contact_pressure = -0.50; // below minimum 0.01
    s.sanitize();
    assert_eq!(s.min_contact_pressure, 0.01);

    s.min_contact_pressure = 0.95; // above maximum 0.50
    s.sanitize();
    assert_eq!(s.min_contact_pressure, 0.50);
}

#[test]
fn test_t2_r3_palm_timeout_clamping_bounds() {
    let mut s = FullAppSettingsModel::default();
    s.palm_proximity_timeout_ms = 10; // below minimum 50ms
    s.sanitize();
    assert_eq!(s.palm_proximity_timeout_ms, 50);

    s.palm_proximity_timeout_ms = 5000; // above maximum 2000ms
    s.sanitize();
    assert_eq!(s.palm_proximity_timeout_ms, 2000);
}

#[test]
fn test_t2_r3_corrupt_json_fallback_to_default() {
    let temp_file = unique_settings_temp_path("corrupt");
    fs::write(&temp_file, "{ invalid json key: value broken ??? }").unwrap();

    let loaded = FullAppSettingsModel::load_or_default(&temp_file);
    assert_eq!(
        loaded,
        FullAppSettingsModel::default(),
        "Corrupted configuration file must safely revert to default settings"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_t2_r3_missing_settings_file_creates_default() {
    let missing_path = PathBuf::from("/tmp/uninote_missing_settings_8888.json");
    let loaded = FullAppSettingsModel::load_or_default(&missing_path);
    assert_eq!(loaded, FullAppSettingsModel::default());
}

#[test]
fn test_t2_r3_pressure_curve_edge_values() {
    for curve in &[
        PressureCurve::Linear,
        PressureCurve::Soft,
        PressureCurve::Hard,
    ] {
        assert_eq!(curve.apply(0.0), 0.0, "Zero pressure must yield zero");
        assert_eq!(curve.apply(1.0), 1.0, "Full pressure must yield full");
        // Out of bounds clamped
        assert_eq!(curve.apply(-0.5), 0.0);
        assert_eq!(curve.apply(1.5), 1.0);
    }
}

// ============================================================================
// TIER 3: PAIRWISE & CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_t3_settings_change_alters_stroke_width_calculation() {
    let s_default = FullAppSettingsModel::default();
    let mut s_soft = FullAppSettingsModel::default();
    s_soft.pressure_curve = PressureCurve::Soft;

    let raw_p = 0.25;
    let width_default = s_default.pressure_curve.apply(raw_p) * 2.8;
    let width_soft = s_soft.pressure_curve.apply(raw_p) * 2.8;

    assert!(
        width_soft > width_default,
        "Soft curve must produce wider strokes at light pressure"
    );
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_student_mpp_digitizer_calibration() {
    // Scenario: Student with light handwriting on HP Envy / Surface Pro
    // opens Settings Window:
    // - Lowers contact threshold to 0.03
    // - Sets sensitivity to Soft
    // - Enables Dark OLED theme for late-night library study
    // - Assigns stylus barrel button to LassoSelect for fast formula reorganization
    // - Verifies settings persist atomically across reboots

    let temp_file = unique_settings_temp_path("student_profile");
    let mut student_profile = FullAppSettingsModel::default();

    student_profile.min_contact_pressure = 0.03;
    student_profile.pressure_curve = PressureCurve::Soft;
    student_profile.theme_mode = ThemeMode::ForceDarkOled;
    student_profile.barrel_button_action = BarrelButtonAction::LassoSelect;
    student_profile.hud_telemetry_enabled = true;

    student_profile.save_atomic(&temp_file).unwrap();

    let loaded = FullAppSettingsModel::load_or_default(&temp_file);
    assert_eq!(loaded.min_contact_pressure, 0.03);
    assert_eq!(loaded.pressure_curve, PressureCurve::Soft);
    assert_eq!(loaded.theme_mode, ThemeMode::ForceDarkOled);
    assert_eq!(loaded.barrel_button_action, BarrelButtonAction::LassoSelect);
    assert!(loaded.hud_telemetry_enabled);

    let _ = fs::remove_file(temp_file);
}

// ============================================================================
// TIER 5: REAL IMPLEMENTATION TESTS (uninote_app::settings & preferences)
// ============================================================================

#[test]
fn test_t5_real_app_settings_model_roundtrip_and_clamping() {
    let temp_file = unique_settings_temp_path("real_settings");
    let mut model = uninote_app::preferences::FullAppSettingsModel::default();

    model.min_contact_pressure = -5.0; // Out of bounds -> clamp to 0.01
    model.anti_hover_blip_threshold = 99.0; // Out of bounds -> clamp to 0.20
    model.pressure_curve = uninote_app::preferences::PressureCurve::Hard;
    model.theme_mode = uninote_app::preferences::ThemeMode::ForceLight;
    model.barrel_button_action = uninote_app::preferences::BarrelButtonAction::ColorPicker;

    model
        .save_atomic(&temp_file)
        .expect("save_atomic must succeed");
    let loaded = uninote_app::preferences::FullAppSettingsModel::load_or_default(&temp_file);

    assert_eq!(loaded.min_contact_pressure, 0.01);
    assert_eq!(loaded.anti_hover_blip_threshold, 0.20);
    assert_eq!(
        loaded.pressure_curve,
        uninote_app::preferences::PressureCurve::Hard
    );
    assert_eq!(
        loaded.theme_mode,
        uninote_app::preferences::ThemeMode::ForceLight
    );
    assert_eq!(
        loaded.barrel_button_action,
        uninote_app::preferences::BarrelButtonAction::ColorPicker
    );

    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_t5_real_settings_window_widget_instantiation() {
    run_on_gtk_thread(|| {
        let settings = std::rc::Rc::new(std::cell::RefCell::new(FullAppSettingsModel::default()));

        let win = UniNoteSettingsWindow::with_settings(settings.clone());
        assert_eq!(win.settings().borrow().min_contact_pressure, 0.05);

        let changed = std::rc::Rc::new(std::cell::RefCell::new(false));
        let ch_clone = changed.clone();
        win.connect_changed(move |_s| {
            *ch_clone.borrow_mut() = true;
        });

        let window_widget = win.window();
        assert_eq!(
            window_widget.title().as_deref(),
            Some("Impostazioni UniNote"),
            "Window title must match DMS design"
        );
        assert_eq!(window_widget.default_width(), 780);
        assert_eq!(window_widget.default_height(), 640);
        assert!(window_widget.has_css_class("uninote-settings-window"));
        assert!(window_widget.can_target());
        assert!(window_widget.is_sensitive());
    });
}

#[test]
fn test_t5_pen_dock_reset_restores_default_slots() {
    let temp_dir = std::env::temp_dir().join(format!("uninote_test_dock_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let prefs_path = temp_dir.join("preferences.json");

    let mut custom_prefs = AppPreferences::default();
    custom_prefs.slots = vec![PenSlotConfig::new(
        "#FF0000",
        (1.0, 0.0, 0.0, 1.0),
        "Rosso Custom",
    )];
    custom_prefs.save_to_path(&prefs_path).unwrap();

    let loaded = AppPreferences::load_from_path(&prefs_path).unwrap();
    assert_eq!(loaded.slots.len(), 1);

    // Esegui la logica di ripristino
    let mut reset_prefs = loaded;
    reset_prefs.slots = AppPreferences::default().slots;
    reset_prefs.active_slot_index = 0;
    reset_prefs.save_to_path(&prefs_path).unwrap();

    let restored = AppPreferences::load_from_path(&prefs_path).unwrap();
    assert_eq!(restored.slots.len(), 5);
    assert_eq!(restored.slots[0].hex, "#6DD2FF");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_t5_shortcuts_model_mutation_and_persistence() {
    let temp_file = unique_settings_temp_path("shortcuts_persist");
    let mut model = FullAppSettingsModel::default();
    model.set_shortcut("undo", "<Control>z");
    model.set_shortcut("custom_action", "<Control><Shift>p");

    model.save_atomic(&temp_file).unwrap();
    let loaded = FullAppSettingsModel::load_or_default(&temp_file);

    assert_eq!(loaded.get_shortcut("undo"), "<Control>z");
    assert_eq!(loaded.get_shortcut("custom_action"), "<Control><Shift>p");

    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_t5_shortcuts_reset_helpers() {
    let mut model = FullAppSettingsModel::default();
    model.set_shortcut("undo", "CustomKey");
    assert_eq!(model.get_shortcut("undo"), "CustomKey");

    model.reset_shortcut("undo");
    assert_eq!(model.get_shortcut("undo"), "<Primary>z");

    model.set_shortcut("save", "CustomSave");
    model.set_shortcut("open", "CustomOpen");
    model.reset_all_shortcuts();
    assert_eq!(model.get_shortcut("save"), "<Primary>s");
    assert_eq!(model.get_shortcut("open"), "<Primary>o");
}
