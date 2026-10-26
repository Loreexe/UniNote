use uninote_app::preferences::{AppPreferences, PenSlotConfig};

#[test]
fn test_remove_slot_success() {
    let mut prefs = AppPreferences::default();
    let initial_count = prefs.slots.len();
    assert!(initial_count >= 2);

    let removed = prefs.remove_slot(1);
    assert!(removed);
    assert_eq!(prefs.slots.len(), initial_count - 1);
}

#[test]
fn test_remove_slot_refuses_when_single_slot_remains() {
    let mut prefs = AppPreferences {
        slots: vec![PenSlotConfig::new(
            "#6DD2FF",
            (0.427, 0.824, 1.0, 1.0),
            "Solo Uno",
        )],
        active_slot_index: 0,
        ..Default::default()
    };

    let removed = prefs.remove_slot(0);
    assert!(!removed);
    assert_eq!(prefs.slots.len(), 1);
    assert_eq!(prefs.active_slot_index, 0);
}

#[test]
fn test_remove_slot_adjusts_active_index_when_removing_active() {
    let mut prefs = AppPreferences {
        active_slot_index: 4,
        ..Default::default()
    };

    let removed = prefs.remove_slot(4);
    assert!(removed);
    assert_eq!(prefs.slots.len(), 4);
    assert_eq!(prefs.active_slot_index, 3); // Clamped to new last index
}

#[test]
fn test_remove_slot_adjusts_active_index_when_removing_earlier_slot() {
    let mut prefs = AppPreferences {
        active_slot_index: 3,
        ..Default::default()
    };

    let removed = prefs.remove_slot(1);
    assert!(removed);
    assert_eq!(prefs.slots.len(), 4);
    assert_eq!(prefs.active_slot_index, 2); // Shifted down by 1
}

#[test]
fn test_remove_slot_keeps_active_index_when_removing_later_slot() {
    let mut prefs = AppPreferences {
        active_slot_index: 1,
        ..Default::default()
    };

    let removed = prefs.remove_slot(3);
    assert!(removed);
    assert_eq!(prefs.slots.len(), 4);
    assert_eq!(prefs.active_slot_index, 1); // Unchanged
}

#[test]
fn test_remove_slot_out_of_bounds() {
    let mut prefs = AppPreferences::default();
    let count = prefs.slots.len();

    let removed = prefs.remove_slot(count + 10);
    assert!(!removed);
    assert_eq!(prefs.slots.len(), count);
}

#[test]
fn test_remove_slot_persistence_roundtrip() {
    let temp_dir =
        std::env::temp_dir().join(format!("uninote_test_slot_del_{}", std::process::id()));
    let _ = std::fs::create_dir_all(&temp_dir);
    let config_path = temp_dir.join("preferences.json");

    let mut prefs = AppPreferences::default();
    assert_eq!(prefs.slots.len(), 5);

    // Remove 2 slots
    assert!(prefs.remove_slot(0));
    assert!(prefs.remove_slot(1));
    assert_eq!(prefs.slots.len(), 3);

    prefs
        .save_to_path(&config_path)
        .expect("save should succeed");

    let loaded = AppPreferences::load_from_path(&config_path).expect("load should succeed");
    assert_eq!(loaded.slots.len(), 3);
    assert_eq!(loaded.slots[0].hex, prefs.slots[0].hex);

    let _ = std::fs::remove_dir_all(&temp_dir);
}
