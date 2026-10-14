//! Unit and integration tests for UniNote Sub-Phase 2A Milestone M4:
//! AppPreferences and PenSlotConfig persistence and lifecycle.

#![allow(dead_code, unused_variables)]

#[path = "../src/preferences.rs"]
pub mod preferences;

use preferences::{AppPreferences, PenSlotConfig};
use std::fs;
use std::path::PathBuf;

fn make_temp_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("uninote_test_prefs_{}_{}", tag, nonce));
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn test_default_preferences_spec() {
    let prefs = AppPreferences::default();
    assert_eq!(prefs.dock_y, 190.0);
    assert_eq!(prefs.stroke_width, 2.8);
    assert_eq!(prefs.active_slot_index, 0);
    assert_eq!(prefs.slots.len(), 5);

    let expected_slots = [
        ("#6DD2FF", (0.427, 0.824, 1.0, 1.0), "Ciano Elettrico"),
        ("#D9E4EB", (0.850, 0.894, 0.920, 1.0), "Bianco Caldo"),
        ("#FFB4AB", (1.0, 0.706, 0.670, 1.0), "Corallo"),
        ("#79D9A5", (0.475, 0.851, 0.647, 1.0), "Menta"),
        ("#FFD875", (1.0, 0.847, 0.459, 1.0), "Oro Brillante"),
    ];

    for (i, (exp_hex, exp_rgba, exp_name)) in expected_slots.iter().enumerate() {
        let slot = &prefs.slots[i];
        assert_eq!(&slot.hex, exp_hex);
        assert_eq!(&slot.name, exp_name);
        assert!((slot.rgba.0 - exp_rgba.0).abs() < 1e-3);
        assert!((slot.rgba.1 - exp_rgba.1).abs() < 1e-3);
        assert!((slot.rgba.2 - exp_rgba.2).abs() < 1e-3);
        assert!((slot.rgba.3 - exp_rgba.3).abs() < 1e-3);
    }
}

#[test]
fn test_serialization_and_deserialization_roundtrip() {
    let original = AppPreferences::default();
    let serialized = serde_json::to_string_pretty(&original).expect("serialize default");
    assert!(serialized.contains("\"dock_y\": 190.0"));
    assert!(serialized.contains("\"stroke_width\": 2.8"));
    assert!(serialized.contains("Ciano Elettrico"));

    let deserialized: AppPreferences =
        serde_json::from_str(&serialized).expect("deserialize roundtrip");
    assert_eq!(original, deserialized);
}

#[test]
fn test_file_persistence_save_and_load() {
    let temp_dir = make_temp_dir("roundtrip");
    let file_path = temp_dir.join("prefs.json");

    let mut custom = AppPreferences {
        dock_y: 312.5,
        stroke_width: 4.0,
        active_slot_index: 2,
        ..Default::default()
    };
    custom.slots.push(PenSlotConfig::new(
        "#8844AA",
        (0.53, 0.27, 0.67, 1.0),
        "Viola Intenso",
    ));

    custom
        .save_to_path(&file_path)
        .expect("save_to_path should succeed");
    assert!(file_path.exists());

    let loaded = AppPreferences::load_from_path(&file_path).expect("load_from_path should succeed");
    assert_eq!(loaded, custom);
    assert_eq!(loaded.slots.len(), 6);
    assert_eq!(loaded.slots[5].name, "Viola Intenso");
    assert_eq!(loaded.active_slot_index, 2);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_corrupted_file_error_and_fallback() {
    let temp_dir = make_temp_dir("corrupt");
    let file_path = temp_dir.join("corrupt.json");

    fs::write(&file_path, "{ broken json content !@#$").expect("write broken");

    let result = AppPreferences::load_from_path(&file_path);
    assert!(result.is_err(), "broken json must produce an error");

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_sanitization_on_empty_slots_and_out_of_bounds_index() {
    let temp_dir = make_temp_dir("sanitize");
    let file_path = temp_dir.join("sanitize.json");

    let raw_json = r#"{
        "dock_y": 250.0,
        "stroke_width": 1.2,
        "slots": [],
        "active_slot_index": 99
    }"#;
    fs::write(&file_path, raw_json).unwrap();

    let loaded = AppPreferences::load_from_path(&file_path).unwrap();
    assert_eq!(
        loaded.slots.len(),
        5,
        "empty slots must recover to default 5 slots"
    );
    assert_eq!(
        loaded.active_slot_index, 0,
        "out-of-bounds index must reset to 0"
    );
    assert_eq!(loaded.dock_y, 250.0);
    assert_eq!(loaded.stroke_width, 1.2);

    let _ = fs::remove_dir_all(temp_dir);
}

#[test]
fn test_config_path_structure() {
    let path = AppPreferences::config_path();
    let path_str = path.to_string_lossy();
    assert!(path_str.ends_with("uninote/preferences.json"));
}
