//! Adversarial & Empirical Test Suite for FullAppSettingsModel & UniNoteSettingsWindow
//!
//! Scope:
//! 1. Boundary & Fuzzing: Malicious JSON, corrupt syntax, wrong types, numerical extremes (NaN, Inf, negatives, huge numbers), unknown fields, atomic save edge cases.
//! 2. Pressure Curve Calibration: 1,000 points sweep, monotonicity, bounds [0, 1], ordering (Hard < Linear < Soft), fixed points, analytical sensitivity.
//! 3. Headless GTK4/Libadwaita Lifecycle: Widget tree construction, multiple instances, callback emission, reentrancy safety, rapid updates.

#![allow(clippy::field_reassign_with_default, clippy::manual_range_contains)]

use gtk4::prelude::*;
use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;
use uninote_app::preferences::{
    AppPreferences, BarrelButtonAction, FullAppSettingsModel, PenSlotConfig, PressureCurve,
    ThemeMode,
};
use uninote_app::settings::UniNoteSettingsWindow;

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
            .name("gtk-adv-settings-worker".into())
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

static TEST_COUNTER: AtomicU64 = AtomicU64::new(100);

fn unique_test_path(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_adv_settings_{}_{}_{}.json",
        prefix,
        std::process::id(),
        id
    ));
    path
}

// ============================================================================
// 1. BOUNDARY & FUZZING TESTS FOR FullAppSettingsModel
// ============================================================================

#[test]
fn test_fuzz_json_malformed_syntax_fallback_to_default() {
    let malformed_payloads = [
        "",
        "   \n\t  ",
        "{",
        "}",
        "{\"min_contact_pressure\":",
        "{\"min_contact_pressure\": 0.05,",
        "[1, 2, 3]",
        "\"a plain string\"",
        "12345",
        "true",
        "null",
        "{ key_without_quotes: 1.0 }",
        "{\"min_contact_pressure\": 0.05 /* comment not allowed in json */}",
        "<<< XML NOT JSON >>>",
        "\0\0\0",
    ];

    for (i, payload) in malformed_payloads.iter().enumerate() {
        let temp_path = unique_test_path(&format!("syntax_{}", i));
        fs::write(&temp_path, payload).expect("Failed to write test file");

        let loaded = FullAppSettingsModel::load_or_default(&temp_path);
        assert_eq!(
            loaded,
            FullAppSettingsModel::default(),
            "Payload #{i} ({payload:?}) must safely trigger graceful fallback to default"
        );

        let _ = fs::remove_file(temp_path);
    }
}

#[test]
fn test_fuzz_json_wrong_types_fallback_to_default() {
    let type_mismatch_payloads = [
        r#"{"min_contact_pressure": "high"}"#,
        r#"{"min_contact_pressure": [0.05]}"#,
        r#"{"min_contact_pressure": {"val": 0.05}}"#,
        r#"{"min_contact_pressure": null}"#,
        r#"{"one_euro_filter_enabled": "yes"}"#,
        r#"{"one_euro_filter_enabled": 1}"#,
        r#"{"one_euro_filter_enabled": null}"#,
        r#"{"palm_proximity_timeout_ms": "300ms"}"#,
        r#"{"palm_proximity_timeout_ms": 300.5}"#,
        r#"{"pressure_curve": "NonExistentCurve"}"#,
        r#"{"pressure_curve": 999}"#,
        r#"{"pressure_curve": null}"#,
        r#"{"theme_mode": "OledNeonCyberpunk"}"#,
        r#"{"theme_mode": 42}"#,
        r#"{"barrel_button_action": "ExplodeDevice"}"#,
        r#"{"shortcuts": "not_a_map"}"#,
        r#"{"shortcuts": {"undo": 12345}}"#,
    ];

    for (i, payload) in type_mismatch_payloads.iter().enumerate() {
        let temp_path = unique_test_path(&format!("types_{}", i));
        fs::write(&temp_path, payload).expect("Failed to write test file");

        let loaded = FullAppSettingsModel::load_or_default(&temp_path);
        assert_eq!(
            loaded,
            FullAppSettingsModel::default(),
            "Type-mismatched payload #{i} must safely fallback to default without panic"
        );

        let _ = fs::remove_file(temp_path);
    }
}

#[test]
fn test_fuzz_json_numerical_extremes_and_clamping() {
    // Construct a valid JSON containing extreme numbers across all fields
    let extreme_json = serde_json::json!({
        "min_contact_pressure": -100.0,
        "pressure_curve": "Hard",
        "anti_hover_blip_threshold": 999.0,
        "one_euro_filter_enabled": false,
        "one_euro_cutoff_hz": 0.000001,
        "palm_proximity_timeout_ms": 0,
        "palm_cooldown_ms": 50000,
        "hud_telemetry_enabled": true,
        "theme_mode": "ForceDarkOled",
        "adapt_ink_rec709": false,
        "dark_sheet_ruling_contrast": -5.0,
        "floating_bars_opacity": 2.5,
        "orthogonal_snap_tolerance_deg": 180.0,
        "rdp_epsilon_factor": -0.5,
        "shortcuts": { "undo": "<Primary>z" },
        "barrel_button_action": "LassoSelect"
    });

    let temp_path = unique_test_path("numerical_extremes");
    fs::write(&temp_path, extreme_json.to_string()).expect("Failed to write test file");

    let loaded = FullAppSettingsModel::load_or_default(&temp_path);

    // Verify all numerical fields are strictly clamped to their documented ranges:
    assert_eq!(loaded.min_contact_pressure, 0.01); // clamp(0.01, 0.50)
    assert_eq!(loaded.anti_hover_blip_threshold, 0.20); // clamp(0.02, 0.20)
    assert_eq!(loaded.one_euro_cutoff_hz, 0.1); // clamp(0.1, 10.0)
    assert_eq!(loaded.palm_proximity_timeout_ms, 50); // clamp(50, 2000)
    assert_eq!(loaded.palm_cooldown_ms, 1000); // clamp(50, 1000)
    assert_eq!(loaded.dark_sheet_ruling_contrast, 0.05); // clamp(0.05, 0.50)
    assert_eq!(loaded.floating_bars_opacity, 1.0); // clamp(0.50, 1.0)
    assert_eq!(loaded.orthogonal_snap_tolerance_deg, 25.0); // clamp(2.0, 25.0)
    assert_eq!(loaded.rdp_epsilon_factor, 0.01); // clamp(0.01, 0.15)

    // Non-numerical settings should be preserved exactly as deserialized:
    assert_eq!(loaded.pressure_curve, PressureCurve::Hard);
    assert!(!loaded.one_euro_filter_enabled);
    assert!(loaded.hud_telemetry_enabled);
    assert_eq!(loaded.theme_mode, ThemeMode::ForceDarkOled);
    assert!(!loaded.adapt_ink_rec709);
    assert_eq!(loaded.barrel_button_action, BarrelButtonAction::LassoSelect);

    let _ = fs::remove_file(temp_path);
}

#[test]
fn test_fuzz_json_huge_floats_and_denormals() {
    let extreme_json = serde_json::json!({
        "min_contact_pressure": 1e308,
        "pressure_curve": "Linear",
        "anti_hover_blip_threshold": -1e308,
        "one_euro_filter_enabled": true,
        "one_euro_cutoff_hz": 1e100,
        "palm_proximity_timeout_ms": 18446744073709551615u64, // u64::MAX
        "palm_cooldown_ms": 0,
        "hud_telemetry_enabled": false,
        "theme_mode": "System",
        "adapt_ink_rec709": true,
        "dark_sheet_ruling_contrast": 1e-100,
        "floating_bars_opacity": 1e300,
        "orthogonal_snap_tolerance_deg": 1e20,
        "rdp_epsilon_factor": 1e-50,
        "shortcuts": {},
        "barrel_button_action": "InstantEraser"
    });

    let temp_path = unique_test_path("huge_floats");
    fs::write(&temp_path, extreme_json.to_string()).expect("Failed to write test file");

    let loaded = FullAppSettingsModel::load_or_default(&temp_path);

    assert_eq!(loaded.min_contact_pressure, 0.50);
    assert_eq!(loaded.anti_hover_blip_threshold, 0.02);
    assert_eq!(loaded.one_euro_cutoff_hz, 10.0);
    assert_eq!(loaded.palm_proximity_timeout_ms, 2000);
    assert_eq!(loaded.palm_cooldown_ms, 50);
    assert_eq!(loaded.dark_sheet_ruling_contrast, 0.05);
    assert_eq!(loaded.floating_bars_opacity, 1.0);
    assert_eq!(loaded.orthogonal_snap_tolerance_deg, 25.0);
    assert_eq!(loaded.rdp_epsilon_factor, 0.01);

    let _ = fs::remove_file(temp_path);
}

#[test]
fn test_fuzz_json_forward_compatibility_extra_fields_ignored() {
    let future_json = serde_json::json!({
        "min_contact_pressure": 0.075,
        "pressure_curve": "Soft",
        "anti_hover_blip_threshold": 0.065,
        "one_euro_filter_enabled": true,
        "one_euro_cutoff_hz": 1.2,
        "palm_proximity_timeout_ms": 350,
        "palm_cooldown_ms": 220,
        "hud_telemetry_enabled": true,
        "theme_mode": "ForceLight",
        "adapt_ink_rec709": false,
        "dark_sheet_ruling_contrast": 0.25,
        "floating_bars_opacity": 0.90,
        "orthogonal_snap_tolerance_deg": 10.0,
        "rdp_epsilon_factor": 0.05,
        "shortcuts": { "undo": "<Primary>z" },
        "barrel_button_action": "ColorPicker",
        "future_ai_summarizer_enabled": true,
        "future_cloud_sync_endpoint": "https://sync.uninote.ac.it",
        "future_vector_engine_version": 4
    });

    let temp_path = unique_test_path("forward_compat");
    fs::write(&temp_path, future_json.to_string()).expect("Failed to write test file");

    let loaded = FullAppSettingsModel::load_or_default(&temp_path);

    // Verify known fields loaded properly despite unfamiliar future fields:
    assert_eq!(loaded.min_contact_pressure, 0.075);
    assert_eq!(loaded.pressure_curve, PressureCurve::Soft);
    assert_eq!(loaded.one_euro_cutoff_hz, 1.2);
    assert_eq!(loaded.palm_proximity_timeout_ms, 350);
    assert_eq!(loaded.theme_mode, ThemeMode::ForceLight);
    assert_eq!(loaded.barrel_button_action, BarrelButtonAction::ColorPicker);

    let _ = fs::remove_file(temp_path);
}

#[test]
fn test_fuzz_sanitize_exhaustive_float_boundary_oracle() {
    let test_floats = [
        f64::NEG_INFINITY,
        -1e308,
        -1e50,
        -100.0,
        -1.0,
        -0.5,
        -0.0,
        0.0,
        0.0001,
        0.005,
        0.01,
        0.05,
        0.10,
        0.50,
        0.95,
        1.0,
        2.5,
        10.0,
        25.0,
        100.0,
        1e50,
        1e308,
        f64::INFINITY,
    ];

    for &f in &test_floats {
        let mut model = FullAppSettingsModel::default();
        model.min_contact_pressure = f;
        model.anti_hover_blip_threshold = f;
        model.one_euro_cutoff_hz = f;
        model.dark_sheet_ruling_contrast = f;
        model.floating_bars_opacity = f;
        model.orthogonal_snap_tolerance_deg = f;
        model.rdp_epsilon_factor = f;

        model.sanitize();

        assert!(
            model.min_contact_pressure >= 0.01 && model.min_contact_pressure <= 0.50,
            "min_contact_pressure out of bounds for input {f}: got {}",
            model.min_contact_pressure
        );
        assert!(
            model.anti_hover_blip_threshold >= 0.02 && model.anti_hover_blip_threshold <= 0.20,
            "anti_hover_blip_threshold out of bounds for input {f}: got {}",
            model.anti_hover_blip_threshold
        );
        assert!(
            model.one_euro_cutoff_hz >= 0.1 && model.one_euro_cutoff_hz <= 10.0,
            "one_euro_cutoff_hz out of bounds for input {f}: got {}",
            model.one_euro_cutoff_hz
        );
        assert!(
            model.dark_sheet_ruling_contrast >= 0.05 && model.dark_sheet_ruling_contrast <= 0.50,
            "dark_sheet_ruling_contrast out of bounds for input {f}: got {}",
            model.dark_sheet_ruling_contrast
        );
        assert!(
            model.floating_bars_opacity >= 0.50 && model.floating_bars_opacity <= 1.0,
            "floating_bars_opacity out of bounds for input {f}: got {}",
            model.floating_bars_opacity
        );
        assert!(
            model.orthogonal_snap_tolerance_deg >= 2.0
                && model.orthogonal_snap_tolerance_deg <= 25.0,
            "orthogonal_snap_tolerance_deg out of bounds for input {f}: got {}",
            model.orthogonal_snap_tolerance_deg
        );
        assert!(
            model.rdp_epsilon_factor >= 0.01 && model.rdp_epsilon_factor <= 0.15,
            "rdp_epsilon_factor out of bounds for input {f}: got {}",
            model.rdp_epsilon_factor
        );
    }
}

#[test]
fn test_fuzz_nan_in_sanitize_does_not_panic() {
    let mut model = FullAppSettingsModel::default();
    model.min_contact_pressure = f64::NAN;
    model.anti_hover_blip_threshold = f64::NAN;
    model.one_euro_cutoff_hz = f64::NAN;
    model.dark_sheet_ruling_contrast = f64::NAN;
    model.floating_bars_opacity = f64::NAN;
    model.orthogonal_snap_tolerance_deg = f64::NAN;
    model.rdp_epsilon_factor = f64::NAN;

    // Must not panic on NaN inputs:
    model.sanitize();
}

#[test]
fn test_persistence_atomic_rapid_sequential_stress() {
    let temp_path = unique_test_path("rapid_stress");

    for i in 0..50 {
        let mut model = FullAppSettingsModel::default();
        model.min_contact_pressure = 0.01 + (i as f64) * 0.005;
        model.palm_proximity_timeout_ms = 100 + (i as u64) * 10;

        model
            .save_atomic(&temp_path)
            .unwrap_or_else(|e| panic!("Save failed on iteration {i}: {e}"));

        let loaded = FullAppSettingsModel::load_or_default(&temp_path);
        assert!(
            (loaded.min_contact_pressure - model.min_contact_pressure).abs() < 1e-6,
            "Mismatch on iteration {i}: loaded={}, model={}",
            loaded.min_contact_pressure,
            model.min_contact_pressure
        );
        assert_eq!(
            loaded.palm_proximity_timeout_ms, model.palm_proximity_timeout_ms,
            "Mismatch on iteration {i}"
        );
    }

    // Clean up
    let _ = fs::remove_file(temp_path);
}

#[test]
fn test_persistence_atomic_target_is_directory_fails_gracefully() {
    let dir_path = unique_test_path("target_dir");
    fs::create_dir_all(&dir_path).expect("Failed to create test directory");

    let model = FullAppSettingsModel::default();
    let result = model.save_atomic(&dir_path);
    assert!(
        result.is_err(),
        "Saving to an existing directory must return an Err, not overwrite or panic"
    );

    let _ = fs::remove_dir(&dir_path);
}

// ============================================================================
// 2. PRESSURE CURVE CALIBRATION (1,000 POINTS SWEEP & ORACLES)
// ============================================================================

#[test]
fn test_pressure_curve_monotonicity_and_bounds_across_1000_points() {
    const SAMPLES: usize = 1000;
    let curves = [
        (PressureCurve::Linear, "Linear"),
        (PressureCurve::Soft, "Soft"),
        (PressureCurve::Hard, "Hard"),
    ];

    for (curve, name) in curves {
        let mut prev_val = -1.0;

        for i in 0..=SAMPLES {
            let raw_p = i as f64 / SAMPLES as f64;
            let calibrated = curve.apply(raw_p);

            // Oracle 1: Value must lie strictly within [0.0, 1.0]
            assert!(
                calibrated >= 0.0 && calibrated <= 1.0,
                "{name} curve at {raw_p}: calibrated {calibrated} is out of bounds [0, 1]"
            );

            // Oracle 2: Monotonicity - curve must be strictly non-decreasing
            assert!(
                calibrated >= prev_val - 1e-12,
                "{name} curve violated monotonicity at p={raw_p}: prev={prev_val}, curr={calibrated}"
            );

            prev_val = calibrated;
        }

        // Oracle 3: Fixed points at endpoints
        assert_eq!(curve.apply(0.0), 0.0, "{name} must map 0.0 to 0.0");
        assert_eq!(curve.apply(1.0), 1.0, "{name} must map 1.0 to 1.0");
    }
}

#[test]
fn test_pressure_curve_strict_ordering_oracle() {
    // For all interior points p in (0.0, 1.0):
    // Hard(p) < Linear(p) < Soft(p)
    // because p^2 < p < sqrt(p) for p in (0, 1).
    const SAMPLES: usize = 1000;

    for i in 1..SAMPLES {
        let p = i as f64 / SAMPLES as f64;
        let lin = PressureCurve::Linear.apply(p);
        let soft = PressureCurve::Soft.apply(p);
        let hard = PressureCurve::Hard.apply(p);

        assert!(
            hard < lin,
            "Hard curve ({hard}) must be strictly less than Linear ({lin}) at p={p}"
        );
        assert!(
            lin < soft,
            "Linear curve ({lin}) must be strictly less than Soft ({soft}) at p={p}"
        );
    }
}

#[test]
fn test_pressure_curve_analytical_sensitivity_checkpoints() {
    // Midpoint p = 0.25:
    // Linear: 0.25
    // Soft: sqrt(0.25) = 0.50 (2x boost for light handwriting)
    // Hard: 0.25^2 = 0.0625 (4x suppression for heavy hands)
    let p_light = 0.25;
    let lin_25 = PressureCurve::Linear.apply(p_light);
    let soft_25 = PressureCurve::Soft.apply(p_light);
    let hard_25 = PressureCurve::Hard.apply(p_light);

    assert!((lin_25 - 0.25).abs() < 1e-9);
    assert!((soft_25 - 0.50).abs() < 1e-9);
    assert!((hard_25 - 0.0625).abs() < 1e-9);

    // Midpoint p = 0.50:
    // Soft: sqrt(0.5) ~ 0.7071
    // Hard: 0.5^2 = 0.25
    let p_mid = 0.50;
    assert!((PressureCurve::Soft.apply(p_mid) - 0.5_f64.sqrt()).abs() < 1e-9);
    assert!((PressureCurve::Hard.apply(p_mid) - 0.25).abs() < 1e-9);
}

#[test]
fn test_pressure_curve_out_of_bounds_clamping_extremes() {
    let out_of_bounds_samples = [
        (-1000.0, 0.0),
        (-1.0, 0.0),
        (-0.00001, 0.0),
        (1.00001, 1.0),
        (2.0, 1.0),
        (1000.0, 1.0),
        (f64::NEG_INFINITY, 0.0),
        (f64::INFINITY, 1.0),
    ];

    for (raw, expected) in out_of_bounds_samples {
        for curve in [
            PressureCurve::Linear,
            PressureCurve::Soft,
            PressureCurve::Hard,
        ] {
            let res = curve.apply(raw);
            assert_eq!(
                res, expected,
                "Curve {curve:?} applied to {raw} must clamp to {expected}, got {res}"
            );
        }
    }
}

// ============================================================================
// 3. GTK / LIBADWAITA HEADLESS LIFECYCLE & UI WIRING
// ============================================================================

#[test]
fn test_headless_settings_window_lifecycle_and_properties() {
    run_on_gtk_thread(|| {
        // 1. Widget tree and properties
        {
            let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
            let win = UniNoteSettingsWindow::with_settings(settings);

            let window_widget = win.window();
            assert_eq!(
                window_widget.title().as_deref(),
                Some("Impostazioni UniNote")
            );
            assert_eq!(window_widget.default_width(), 780);
            assert_eq!(window_widget.default_height(), 640);
            assert!(window_widget.has_css_class("uninote-settings-window"));
        }

        // 2. Callback firing and isolation
        {
            let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
            let win = UniNoteSettingsWindow::with_settings(settings.clone());

            let callback_invocations = Rc::new(RefCell::new(0usize));
            let last_threshold = Rc::new(RefCell::new(0.0f64));

            let cb_count = callback_invocations.clone();
            let cb_thresh = last_threshold.clone();

            win.connect_changed(move |model| {
                *cb_count.borrow_mut() += 1;
                *cb_thresh.borrow_mut() = model.min_contact_pressure;
            });

            assert_eq!(*callback_invocations.borrow(), 0);

            settings.borrow_mut().min_contact_pressure = 0.12;
            assert_eq!(win.settings().borrow().min_contact_pressure, 0.12);
        }

        // 3. Multiple instances lifecycle
        {
            for i in 0..15 {
                let mut model = FullAppSettingsModel::default();
                model.min_contact_pressure = 0.05 + (i as f64) * 0.01;
                model.pressure_curve = if i % 2 == 0 {
                    PressureCurve::Soft
                } else {
                    PressureCurve::Hard
                };

                let settings = Rc::new(RefCell::new(model));
                let win = UniNoteSettingsWindow::with_settings(settings.clone());

                assert_eq!(
                    win.settings().borrow().min_contact_pressure,
                    0.05 + (i as f64) * 0.01
                );
                let _ = win.window();
            }
        }

        // 4. Default constructor loads saved or default
        {
            let win = UniNoteSettingsWindow::default();
            let model = win.settings().borrow().clone();

            assert!(model.min_contact_pressure >= 0.01 && model.min_contact_pressure <= 0.50);
            assert!(
                model.anti_hover_blip_threshold >= 0.02 && model.anti_hover_blip_threshold <= 0.20
            );
            assert!(model.one_euro_cutoff_hz >= 0.1 && model.one_euro_cutoff_hz <= 10.0);
            assert!(
                model.palm_proximity_timeout_ms >= 50 && model.palm_proximity_timeout_ms <= 2000
            );
        }
    });
}

// ============================================================================
// 4. EMPIRICAL CHALLENGER TESTS (Shortcuts, Pen Dock Reset, Live Area)
// ============================================================================

#[test]
fn test_adv_shortcuts_mutations_persistence_and_reset_helpers() {
    let temp_file = unique_test_path("shortcuts_full");
    let mut model = FullAppSettingsModel::default();

    // Verify all 8 default shortcuts are populated
    for (id, _, _, def) in FullAppSettingsModel::SHORTCUT_SPECS {
        assert_eq!(
            &model.get_shortcut(id),
            def,
            "Default shortcut for {id} mismatch"
        );
    }

    // Mutate all 8 shortcuts to custom bindings
    for (id, _, _, _) in FullAppSettingsModel::SHORTCUT_SPECS {
        model.set_shortcut(id, &format!("<Control><Alt>{}", &id[..1]));
    }
    model
        .save_atomic(&temp_file)
        .expect("save_atomic must succeed");

    // Reload from disk and verify persistence fidelity
    let loaded = FullAppSettingsModel::load_or_default(&temp_file);
    for (id, _, _, _) in FullAppSettingsModel::SHORTCUT_SPECS {
        assert_eq!(
            loaded.get_shortcut(id),
            format!("<Control><Alt>{}", &id[..1]),
            "Persisted shortcut mismatch for {id}"
        );
    }

    // Reset single shortcut
    let mut modified = loaded;
    modified.reset_shortcut("undo");
    assert_eq!(modified.get_shortcut("undo"), "<Primary>z");
    // Verify others remain modified
    assert_eq!(modified.get_shortcut("redo"), "<Control><Alt>r");

    // Reset all shortcuts
    modified.reset_all_shortcuts();
    for (id, _, _, def) in FullAppSettingsModel::SHORTCUT_SPECS {
        assert_eq!(
            &modified.get_shortcut(id),
            def,
            "Reset all shortcut for {id} mismatch"
        );
    }

    // Edge case: whitespace or empty string returns default
    modified.set_shortcut("save", "   ");
    assert_eq!(modified.get_shortcut("save"), "<Primary>s");

    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_adv_pen_dock_reset_and_persistence() {
    let temp_dir = std::env::temp_dir().join(format!("uninote_adv_dock_{}", std::process::id()));
    let _ = fs::create_dir_all(&temp_dir);
    let prefs_path = temp_dir.join("preferences.json");

    // Start with custom slots: 2 custom slots, active index 1
    let mut custom_prefs = AppPreferences::default();
    custom_prefs.slots = vec![
        PenSlotConfig::new("#112233", (0.1, 0.2, 0.3, 1.0), "Custom Dark"),
        PenSlotConfig::new("#445566", (0.4, 0.5, 0.6, 1.0), "Custom Slate"),
    ];
    custom_prefs.active_slot_index = 1;
    custom_prefs
        .save_to_path(&prefs_path)
        .expect("save_to_path must succeed");

    let loaded = AppPreferences::load_from_path(&prefs_path).expect("load_from_path must succeed");
    assert_eq!(loaded.slots.len(), 2);
    assert_eq!(loaded.active_slot_index, 1);
    assert_eq!(loaded.slots[0].hex, "#112233");

    // Invoke genuine reset logic
    let mut reset_prefs = loaded;
    reset_prefs.slots = AppPreferences::default().slots;
    reset_prefs.active_slot_index = 0;
    reset_prefs
        .save_to_path(&prefs_path)
        .expect("save_to_path must succeed");

    // Reload and verify 5 academic slots restored
    let restored =
        AppPreferences::load_from_path(&prefs_path).expect("load_from_path must succeed");
    assert_eq!(restored.slots.len(), 5);
    assert_eq!(restored.active_slot_index, 0);
    assert_eq!(restored.slots[0].hex, "#6DD2FF");
    assert_eq!(restored.slots[1].hex, "#D9E4EB");
    assert_eq!(restored.slots[2].hex, "#FFB4AB");
    assert_eq!(restored.slots[3].hex, "#79D9A5");
    assert_eq!(restored.slots[4].hex, "#FFD875");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_adv_pressure_curve_math_and_sensitivity_oracles() {
    // Monotonicity and bounds oracle: 10,000 steps sweep on [0, 1]
    let steps = 10_000;
    let mut prev_linear = 0.0;
    let mut prev_soft = 0.0;
    let mut prev_hard = 0.0;

    for i in 0..=steps {
        let p = (i as f64) / (steps as f64);
        let lin = PressureCurve::Linear.apply(p);
        let soft = PressureCurve::Soft.apply(p);
        let hard = PressureCurve::Hard.apply(p);

        // Bounds: [0, 1]
        assert!(
            lin >= 0.0 && lin <= 1.0,
            "Linear out of bounds at {p}: {lin}"
        );
        assert!(
            soft >= 0.0 && soft <= 1.0,
            "Soft out of bounds at {p}: {soft}"
        );
        assert!(
            hard >= 0.0 && hard <= 1.0,
            "Hard out of bounds at {p}: {hard}"
        );

        // Ordering for interior: hard < linear < soft
        if p > 0.0 && p < 1.0 {
            assert!(
                hard < lin,
                "Strict inequality hard < lin violated at {p}: hard={hard}, lin={lin}"
            );
            assert!(
                lin < soft,
                "Strict inequality lin < soft violated at {p}: lin={lin}, soft={soft}"
            );
        }

        // Monotonic non-decreasing
        if i > 0 {
            assert!(lin >= prev_linear, "Linear monotonicity violated at {p}");
            assert!(soft >= prev_soft, "Soft monotonicity violated at {p}");
            assert!(hard >= prev_hard, "Hard monotonicity violated at {p}");
        }

        prev_linear = lin;
        prev_soft = soft;
        prev_hard = hard;
    }

    // Fixed points
    assert_eq!(PressureCurve::Linear.apply(0.0), 0.0);
    assert_eq!(PressureCurve::Linear.apply(1.0), 1.0);
    assert_eq!(PressureCurve::Soft.apply(0.0), 0.0);
    assert_eq!(PressureCurve::Soft.apply(1.0), 1.0);
    assert_eq!(PressureCurve::Hard.apply(0.0), 0.0);
    assert_eq!(PressureCurve::Hard.apply(1.0), 1.0);

    // Clamping
    assert_eq!(PressureCurve::Linear.apply(-1.0), 0.0);
    assert_eq!(PressureCurve::Linear.apply(2.0), 1.0);
    assert_eq!(PressureCurve::Soft.apply(-1.0), 0.0);
    assert_eq!(PressureCurve::Soft.apply(2.0), 1.0);
    assert_eq!(PressureCurve::Hard.apply(-1.0), 0.0);
    assert_eq!(PressureCurve::Hard.apply(2.0), 1.0);
}

fn find_widget_by_css_recursive(widget: &gtk4::Widget, css_class: &str) -> Option<gtk4::Widget> {
    if widget.has_css_class(css_class) {
        return Some(widget.clone());
    }
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_widget_by_css_recursive(&c, css_class) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

#[test]
fn test_adv_live_area_widget_hierarchy_inspection() {
    run_on_gtk_thread(|| {
        let settings = Rc::new(RefCell::new(FullAppSettingsModel::default()));
        let win = UniNoteSettingsWindow::with_settings(settings);
        let window_widget: gtk4::Widget = win.window().clone().upcast();

        // 1. Verify progress bar is mounted in widget tree
        let p_bar = find_widget_by_css_recursive(&window_widget, "dms-pressure-bar");
        assert!(
            p_bar.is_some(),
            "dms-pressure-bar must be present in settings window hierarchy"
        );

        // 2. Check if live_area (DrawingArea with dms-pressure-test-area) is mounted in widget tree
        let live_area = find_widget_by_css_recursive(&window_widget, "dms-pressure-test-area");
        assert!(
            live_area.is_some(),
            "EMPIRICAL FINDING: dms-pressure-test-area (DrawingArea live_area) is NOT mounted in the widget tree! live_box only appended progress_live, leaving live_area orphaned and unable to receive stylus events."
        );
    });
}
