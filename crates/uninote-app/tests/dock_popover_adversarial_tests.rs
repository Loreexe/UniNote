//! Adversarial Empirical Test Suite for UniNote Sub-Phase 2A Milestone M4:
//! Pen Dock & DMS Popover System.
//!
//! Stress-tests:
//! 1. `AppPreferences` with extreme inputs (negative dock_y, huge dock_y, 0 stroke_width, NaN, Inf).
//! 2. Empty slots vector recovery, 100+ slots scaling, index sanitization.
//! 3. Corrupted JSON, non-UTF8 bytes, unwriteable directory resilience.
//! 4. Invalid hex colors and markup injection.
//! 5. Empirical mathematical proof of Dock drag clamping boundary failure (panic in interval `(dock_h + 120, dock_h + 132)`).
//! 6. Demonstration of robust docking clamping formula.

#![allow(dead_code, unused_variables)]

#[path = "../src/preferences.rs"]
pub mod preferences;

use preferences::{AppPreferences, PenSlotConfig};
use std::fs;
use std::path::PathBuf;

fn make_adversarial_temp_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("uninote_m4_adversarial_{}_{}", tag, nonce));
    fs::create_dir_all(&dir).unwrap();
    dir
}

// =========================================================================
// 1. Extreme numeric inputs: dock_y, stroke_width
// =========================================================================

#[test]
fn test_extreme_dock_y_inputs_load_and_gui_contract() {
    let temp_dir = make_adversarial_temp_dir("dock_y_extreme");
    let file_path = temp_dir.join("extreme_dock_y.json");

    // Extreme negative dock_y
    let json_negative = r#"{
        "dock_y": -9999.0,
        "stroke_width": 2.8,
        "slots": [],
        "active_slot_index": 0
    }"#;
    fs::write(&file_path, json_negative).unwrap();
    let loaded_neg = AppPreferences::load_from_path(&file_path).expect("load negative dock_y");
    assert_eq!(loaded_neg.dock_y, -9999.0);

    // Verify window.rs initial dock clamping contract: .clamp(72.0, 600.0)
    let clamped_neg = loaded_neg.dock_y.clamp(72.0, 600.0);
    assert_eq!(clamped_neg, 72.0, "Negative dock_y must clamp to min 72.0");

    // Extreme positive dock_y
    let json_huge = r#"{
        "dock_y": 99999.0,
        "stroke_width": 2.8,
        "slots": [],
        "active_slot_index": 0
    }"#;
    fs::write(&file_path, json_huge).unwrap();
    let loaded_huge = AppPreferences::load_from_path(&file_path).expect("load huge dock_y");
    assert_eq!(loaded_huge.dock_y, 99999.0);

    let clamped_huge = loaded_huge.dock_y.clamp(72.0, 600.0);
    assert_eq!(clamped_huge, 600.0, "Huge dock_y must clamp to max 600.0");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_extreme_stroke_width_inputs() {
    let temp_dir = make_adversarial_temp_dir("stroke_extreme");
    let file_path = temp_dir.join("stroke_w.json");

    // stroke_width = 0.0
    let json_zero = r#"{
        "dock_y": 190.0,
        "stroke_width": 0.0,
        "slots": [],
        "active_slot_index": 0
    }"#;
    fs::write(&file_path, json_zero).unwrap();
    let loaded_zero = AppPreferences::load_from_path(&file_path).expect("load zero stroke_width");
    assert_eq!(loaded_zero.stroke_width, 0.0);

    // stroke_width negative
    let json_neg = r#"{
        "dock_y": 190.0,
        "stroke_width": -10.5,
        "slots": [],
        "active_slot_index": 0
    }"#;
    fs::write(&file_path, json_neg).unwrap();
    let loaded_neg =
        AppPreferences::load_from_path(&file_path).expect("load negative stroke_width");
    assert_eq!(loaded_neg.stroke_width, -10.5);

    // stroke_width huge
    let json_huge = r#"{
        "dock_y": 190.0,
        "stroke_width": 999.0,
        "slots": [],
        "active_slot_index": 0
    }"#;
    fs::write(&file_path, json_huge).unwrap();
    let loaded_huge = AppPreferences::load_from_path(&file_path).expect("load huge stroke_width");
    assert_eq!(loaded_huge.stroke_width, 999.0);

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 2. NaN and Infinity handling
// =========================================================================

#[test]
fn test_nan_and_infinity_in_json_fails_gracefully_to_default() {
    let temp_dir = make_adversarial_temp_dir("nan_inf");

    // Standard JSON rejects bare NaN
    let file_nan = temp_dir.join("nan.json");
    fs::write(
        &file_nan,
        r#"{"dock_y": NaN, "stroke_width": 2.8, "slots": [], "active_slot_index": 0}"#,
    )
    .unwrap();
    assert!(
        AppPreferences::load_from_path(&file_nan).is_err(),
        "Bare NaN in JSON must return Err"
    );

    // Standard JSON rejects bare Infinity
    let file_inf = temp_dir.join("inf.json");
    fs::write(
        &file_inf,
        r#"{"dock_y": 190.0, "stroke_width": Infinity, "slots": [], "active_slot_index": 0}"#,
    )
    .unwrap();
    assert!(
        AppPreferences::load_from_path(&file_inf).is_err(),
        "Bare Infinity in JSON must return Err"
    );

    // JSON null for numeric field
    let file_null = temp_dir.join("null.json");
    fs::write(
        &file_null,
        r#"{"dock_y": null, "stroke_width": 2.8, "slots": [], "active_slot_index": 0}"#,
    )
    .unwrap();
    assert!(
        AppPreferences::load_from_path(&file_null).is_err(),
        "null in numeric field must return Err"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_in_memory_nan_or_inf_save_never_panics() {
    let temp_dir = make_adversarial_temp_dir("save_nan");
    let file_path = temp_dir.join("save_nan.json");

    let nan_prefs = AppPreferences {
        dock_y: f64::NAN,
        stroke_width: f64::INFINITY,
        slots: vec![],
        active_slot_index: 0,
    };

    let save_res = nan_prefs.save_to_path(&file_path);
    // Observe serde_json behavior: does it write null, and how does load handle it?
    assert!(
        save_res.is_ok(),
        "serde_json serializes NaN/Infinity as null without panicking"
    );
    let written = fs::read_to_string(&file_path).unwrap();
    assert!(
        written.contains("null"),
        "serde_json writes null for NaN/Inf"
    );

    // Deserializing null back into f64 must fail gracefully in load_from_path
    let reload_res = AppPreferences::load_from_path(&file_path);
    assert!(
        reload_res.is_err(),
        "Loading null for f64 field must return Err"
    );

    // And AppPreferences::load() must recover gracefully to Default
    // (Simulate load() behavior)
    let fallback = reload_res.unwrap_or_default();
    assert_eq!(
        fallback.dock_y, 190.0,
        "Fallback must produce Default dock_y"
    );
    assert_eq!(
        fallback.stroke_width, 2.8,
        "Fallback must produce Default stroke_width"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 3. Empty slots and Index sanitization
// =========================================================================

#[test]
fn test_empty_slots_and_out_of_bounds_indices_sanitization() {
    let temp_dir = make_adversarial_temp_dir("slots_sanitize");
    let file_path = temp_dir.join("empty_slots.json");

    // Empty slots with active_slot_index = 42
    let json = r#"{
        "dock_y": 190.0,
        "stroke_width": 2.8,
        "slots": [],
        "active_slot_index": 42
    }"#;
    fs::write(&file_path, json).unwrap();

    let loaded = AppPreferences::load_from_path(&file_path).unwrap();
    assert_eq!(
        loaded.slots.len(),
        5,
        "Empty slots must be repopulated with 5 defaults"
    );
    assert_eq!(
        loaded.active_slot_index, 0,
        "Out of bounds index must reset to 0"
    );

    // 1 custom slot with index = 1 (out of bounds)
    let json_single = r##"{
        "dock_y": 190.0,
        "stroke_width": 2.8,
        "slots": [
            {"hex": "#112233", "rgba": [0.1, 0.2, 0.3, 1.0], "name": "Custom 1"}
        ],
        "active_slot_index": 1
    }"##;
    fs::write(&file_path, json_single).unwrap();

    let loaded_single = AppPreferences::load_from_path(&file_path).unwrap();
    assert_eq!(loaded_single.slots.len(), 1);
    assert_eq!(
        loaded_single.active_slot_index, 0,
        "Index 1 with 1 slot must reset to 0"
    );

    // 1 custom slot with usize::MAX
    let json_max = format!(
        r##"{{
        "dock_y": 190.0,
        "stroke_width": 2.8,
        "slots": [
            {{"hex": "#112233", "rgba": [0.1, 0.2, 0.3, 1.0], "name": "Custom 1"}}
        ],
        "active_slot_index": {}
    }}"##,
        usize::MAX
    );
    fs::write(&file_path, json_max).unwrap();

    let loaded_max = AppPreferences::load_from_path(&file_path).unwrap();
    assert_eq!(
        loaded_max.active_slot_index, 0,
        "usize::MAX must reset to 0"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 4. Massive slots vector scaling (100+ slots)
// =========================================================================

#[test]
fn test_massive_slots_vector_150_slots_roundtrip() {
    let temp_dir = make_adversarial_temp_dir("massive_slots");
    let file_path = temp_dir.join("150_slots.json");

    let mut prefs = AppPreferences {
        dock_y: 200.0,
        stroke_width: 3.5,
        slots: Vec::with_capacity(150),
        active_slot_index: 149,
    };

    for i in 0..150 {
        let r = (i as f64) / 150.0;
        prefs.slots.push(PenSlotConfig::new(
            format!("#{:02X}88FF", (r * 255.0) as u8),
            (r, 0.5, 1.0, 1.0),
            format!("Slot Color {}", i),
        ));
    }

    let t0 = std::time::Instant::now();
    prefs.save_to_path(&file_path).expect("save 150 slots");
    let save_duration = t0.elapsed();
    assert!(
        save_duration.as_millis() < 50,
        "Saving 150 slots must take < 50ms"
    );

    let t1 = std::time::Instant::now();
    let loaded = AppPreferences::load_from_path(&file_path).expect("load 150 slots");
    let load_duration = t1.elapsed();
    assert!(
        load_duration.as_millis() < 50,
        "Loading 150 slots must take < 50ms"
    );

    assert_eq!(loaded.slots.len(), 150);
    assert_eq!(loaded.active_slot_index, 149);
    assert_eq!(loaded.slots[149].name, "Slot Color 149");

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 5. Corrupted files, non-UTF8 bytes, unwriteable directory
// =========================================================================

#[test]
fn test_corrupted_json_syntax_returns_err() {
    let temp_dir = make_adversarial_temp_dir("corrupt_syntax");
    let file_path = temp_dir.join("corrupt.json");

    let syntax_errors = [
        "",
        "{",
        "{ \"dock_y\": }",
        "{\"slots\": [}",
        "random unformatted text content",
        "{\"dock_y\": 190.0, \"stroke_width\": 2.8, \"slots\": [{\"hex\": \"#FFF\", \"rgba\": [1,2,3]}], \"active_slot_index\": 0}", // rgba tuple missing alpha
    ];

    for (idx, err_content) in syntax_errors.iter().enumerate() {
        fs::write(&file_path, err_content).unwrap();
        let res = AppPreferences::load_from_path(&file_path);
        assert!(
            res.is_err(),
            "Case {} must return Err: {:?}",
            idx,
            err_content
        );
    }

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_non_utf8_binary_bytes_returns_err() {
    let temp_dir = make_adversarial_temp_dir("non_utf8");
    let file_path = temp_dir.join("binary.json");

    let bad_bytes: [u8; 8] = [0xFF, 0xFE, 0xFD, 0xFC, 0x80, 0x81, 0x82, 0x00];
    fs::write(&file_path, bad_bytes).unwrap();

    let res = AppPreferences::load_from_path(&file_path);
    assert!(res.is_err(), "Non-UTF8 file must return Err");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_unwriteable_directory_save_safety() {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = make_adversarial_temp_dir("readonly_dir");
    let ro_dir = temp_dir.join("ro_subdir");
    fs::create_dir_all(&ro_dir).unwrap();

    // Set read-only permissions (r-xr-xr-x)
    let _ = fs::set_permissions(&ro_dir, Permissions::from_mode(0o555));

    let file_path = ro_dir.join("prefs.json");
    let prefs = AppPreferences::default();

    let res = prefs.save_to_path(&file_path);
    // On unix, writing to read-only directory fails with PermissionDenied
    // Even if running as root, the call must return a Result and never panic.
    if let Err(e) = res {
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    }

    // Reset permissions so cleanup can delete it
    let _ = fs::set_permissions(&ro_dir, Permissions::from_mode(0o755));
    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_saving_to_directory_path_returns_err_no_panic() {
    let temp_dir = make_adversarial_temp_dir("save_to_dir");
    let dir_path = temp_dir.join("a_directory");
    fs::create_dir_all(&dir_path).unwrap();

    let prefs = AppPreferences::default();
    let res = prefs.save_to_path(&dir_path);
    assert!(
        res.is_err(),
        "Writing to an existing directory path must return Err"
    );

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 6. Invalid hex colors and markup injection
// =========================================================================

#[test]
fn test_invalid_hex_colors_and_markup_resilience() {
    let temp_dir = make_adversarial_temp_dir("hex_adversarial");
    let file_path = temp_dir.join("adversarial_colors.json");

    let adversarial_hexes = [
        "",
        "not_a_color",
        "#12",
        "#GGGGGG",
        "#123456789ABCDEF",
        "<span foreground='red'>pwned</span>",
        "\" onmouseover=\"alert(1)\"",
        "&amp; < > ' \"",
    ];

    let mut prefs = AppPreferences::default();
    prefs.slots.clear();
    for (i, &hex) in adversarial_hexes.iter().enumerate() {
        prefs.slots.push(PenSlotConfig::new(
            hex,
            (0.1, 0.2, 0.3, 1.0),
            format!("Adversarial Slot {}", i),
        ));
    }

    prefs
        .save_to_path(&file_path)
        .expect("save adversarial hexes");
    let loaded = AppPreferences::load_from_path(&file_path).expect("load adversarial hexes");
    assert_eq!(loaded.slots.len(), adversarial_hexes.len());

    for (i, &hex) in adversarial_hexes.iter().enumerate() {
        assert_eq!(loaded.slots[i].hex, hex);
    }

    let _ = fs::remove_dir_all(temp_dir);
}

// =========================================================================
// 7. EMPIRICAL VULNERABILITY TEST: Dock Drag Clamping Boundary Panic
// =========================================================================

/// In `crates/uninote-app/src/window.rs`, the drag update controller computes:
/// ```rust
/// let max_y = if win_h > dock_h + 120.0 {
///     win_h - dock_h - 60.0
/// } else {
///     700.0
/// };
/// let new_y = (init_y + offset_y).clamp(72.0, max_y);
/// ```
///
/// Mathematical Vulnerability Proof:
/// Whenever `dock_h + 120.0 < win_h < dock_h + 132.0`:
/// 1. `win_h > dock_h + 120.0` evaluates to TRUE.
/// 2. `max_y = win_h - dock_h - 60.0 < 72.0`.
/// 3. In `f64::clamp(72.0, max_y)`, `min` (72.0) > `max` (< 72.0).
/// 4. Rust's `f64::clamp` asserts `min <= max` and PANICS immediately.
#[test]
fn test_dock_drag_clamping_boundary_panic_reproduction() {
    let dock_h = 300.0f64;

    // Critical window height: dock_h + 125.0 = 425.0
    // dock_h + 120.0 = 420.0 (win_h > dock_h + 120.0 is TRUE)
    // max_y = 425.0 - 300.0 - 60.0 = 65.0
    // min = 72.0, max = 65.0 => min > max!
    let win_h = 425.0f64;
    let init_y = 72.0f64;
    let offset_y = 10.0f64;

    let panic_result = std::panic::catch_unwind(|| {
        let max_y = if win_h > dock_h + 120.0 {
            win_h - dock_h - 60.0
        } else {
            700.0
        };
        (init_y + offset_y).clamp(72.0, max_y)
    });

    assert!(
        panic_result.is_err(),
        "CRITICAL DEFECT: Current clamp logic MUST panic when dock_h + 120 < win_h < dock_h + 132!"
    );
}

/// Sweeps across the entire vulnerability interval `(dock_h + 120.0, dock_h + 132.0)`
/// to prove that every single value in this 12-pixel window causes a panic.
#[test]
fn test_dock_drag_clamping_sweep_vulnerability_window() {
    let dock_h = 250.0f64;

    // Vulnerability range: (370.0, 382.0)
    for delta in 1..12 {
        let win_h = dock_h + 120.0 + (delta as f64);
        let panic_result = std::panic::catch_unwind(|| {
            let max_y = if win_h > dock_h + 120.0 {
                win_h - dock_h - 60.0
            } else {
                700.0
            };
            72.0f64.clamp(72.0, max_y)
        });

        assert!(
            panic_result.is_err(),
            "win_h = {} must trigger clamp panic (min 72.0 > max {})",
            win_h,
            win_h - dock_h - 60.0
        );
    }
}

/// When `win_h <= dock_h + 120.0`, the fallback `max_y = 700.0` allows the dock
/// to be dragged completely offscreen if window height is small (e.g. 400px).
#[test]
fn test_dock_drag_clamping_offscreen_defect() {
    let dock_h = 300.0f64;
    let win_h = 400.0f64; // Small window (e.g. portrait or split view)

    let max_y = if win_h > dock_h + 120.0 {
        win_h - dock_h - 60.0
    } else {
        700.0 // Hardcoded fallback
    };

    assert_eq!(max_y, 700.0);
    let dragged_y = 650.0f64.clamp(72.0, max_y);
    assert_eq!(dragged_y, 650.0);

    // Dock bottom would be at 650 + 300 = 950px, while window height is only 400px!
    // The dock is 550px below the bottom edge of the window.
    assert!(
        dragged_y > win_h,
        "Dock top (650) exceeds total window height (400)"
    );
}

/// Demonstrates the mathematically sound, robust clamping formula:
/// `let max_y = (win_h - dock_h - 60.0).max(72.0);`
/// `let new_y = (init_y + offset_y).clamp(72.0, max_y);`
#[test]
fn test_robust_dock_drag_clamping_never_panics_and_bounds_dock() {
    let test_dock_heights: [f64; 4] = [100.0, 200.0, 300.0, 500.0];
    let test_win_heights: [f64; 8] = [200.0, 350.0, 420.0, 425.0, 500.0, 768.0, 1080.0, 1440.0];
    let test_offsets: [f64; 6] = [-500.0, -100.0, 0.0, 50.0, 200.0, 1000.0];

    for &dock_h in &test_dock_heights {
        for &win_h in &test_win_heights {
            for &offset_y in &test_offsets {
                let init_y = 190.0f64;

                // Robust clamping formula:
                // 1. max_y is guaranteed to be >= 72.0
                // 2. Therefore min (72.0) <= max (max_y) is ALWAYS true
                // 3. When window is too small, dock is safely pinned at top (72.0)
                let max_y = f64::max(win_h - dock_h - 60.0, 72.0);
                let new_y = (init_y + offset_y).clamp(72.0, max_y);

                assert!(new_y >= 72.0, "new_y must be >= 72.0");
                assert!(new_y <= max_y, "new_y must be <= max_y");
                assert!(new_y.is_finite(), "new_y must be finite");
            }
        }
    }
}
