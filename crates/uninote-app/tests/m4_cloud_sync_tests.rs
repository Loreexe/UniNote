use std::fs;
use std::path::PathBuf;
use uninote_app::cloud::{CloudServiceType, CloudSyncSettings};
use uninote_app::dashboard::is_safe_subpath;

fn unique_test_temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "uninote_test_cloud_{}_{}",
        std::process::id(),
        name
    ));
    let _ = fs::create_dir_all(&dir);
    dir
}

#[test]
fn test_cloud_sync_settings_defaults_and_serde() {
    let default_st = CloudSyncSettings::default();
    assert!(default_st.nextcloud.is_none());
    assert!(default_st.google_drive.is_none());

    let json = serde_json::to_string(&default_st).expect("serialization failed");
    let deserialized: CloudSyncSettings =
        serde_json::from_str(&json).expect("deserialization failed");
    assert_eq!(default_st, deserialized);
}

#[test]
fn test_cloud_service_type_variants() {
    let t1 = CloudServiceType::Nextcloud;
    let t2 = CloudServiceType::GoogleDrive;
    assert_ne!(t1, t2);

    let j1 = serde_json::to_string(&t1).unwrap();
    let d1: CloudServiceType = serde_json::from_str(&j1).unwrap();
    assert_eq!(t1, d1);
}

#[test]
fn test_cloud_directories_are_isolated_from_project_repo() {
    let nc_dir = CloudSyncSettings::nextcloud_local_dir();
    let gd_dir = CloudSyncSettings::gdrive_local_dir();

    // Invariante di isolamento: NON devono mai puntare al repository di codice UniNote
    let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    assert_ne!(nc_dir, current_dir);
    assert_ne!(gd_dir, current_dir);

    // Devono risiedere in ~/.local/share/uninote/cloud/
    assert!(nc_dir.to_string_lossy().contains("uninote"));
    assert!(nc_dir.to_string_lossy().contains("cloud"));
    assert!(nc_dir.to_string_lossy().ends_with("nextcloud"));

    assert!(gd_dir.to_string_lossy().contains("uninote"));
    assert!(gd_dir.to_string_lossy().contains("cloud"));
    assert!(gd_dir.to_string_lossy().ends_with("gdrive"));
}

#[test]
fn test_connect_and_disconnect_nextcloud() {
    let temp_dir = unique_test_temp_dir("nc");
    let config_path = temp_dir.join("cloud.json");

    let mut settings = CloudSyncSettings::default();
    assert!(settings.nextcloud.is_none());

    // Connessione Nextcloud
    let res = settings.connect_nextcloud("https://cloud.universita.it", "studente_uninote");
    assert!(res.is_ok());

    let nc = settings
        .nextcloud
        .as_ref()
        .expect("nextcloud should be set");
    assert_eq!(nc.server_url, "https://cloud.universita.it");
    assert_eq!(nc.username, "studente_uninote");
    assert_eq!(nc.remote_folder, "UniNote");
    assert!(nc.is_connected);

    // Salvataggio e ricaricamento
    settings.save_to_path(&config_path).expect("save failed");
    let loaded = CloudSyncSettings::load_from_path(&config_path);
    assert_eq!(settings, loaded);

    // Disconnessione
    let _ = settings.disconnect_nextcloud();
    assert!(settings.nextcloud.is_none());

    settings.save_to_path(&config_path).expect("save failed");
    let loaded_after = CloudSyncSettings::load_from_path(&config_path);
    assert!(loaded_after.nextcloud.is_none());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_connect_and_disconnect_google_drive() {
    let temp_dir = unique_test_temp_dir("gd");
    let config_path = temp_dir.join("cloud.json");

    let mut settings = CloudSyncSettings::default();
    assert!(settings.google_drive.is_none());

    // Connessione Google Drive
    let res = settings.connect_google_drive("studente@gmail.com");
    assert!(res.is_ok());

    let gd = settings
        .google_drive
        .as_ref()
        .expect("gdrive should be set");
    assert_eq!(gd.account_email, "studente@gmail.com");
    assert_eq!(gd.remote_folder, "UniNote");
    assert!(gd.is_connected);

    // Salvataggio e ricaricamento
    settings.save_to_path(&config_path).expect("save failed");
    let loaded = CloudSyncSettings::load_from_path(&config_path);
    assert_eq!(settings, loaded);

    // Disconnessione
    let _ = settings.disconnect_google_drive();
    assert!(settings.google_drive.is_none());

    settings.save_to_path(&config_path).expect("save failed");
    let loaded_after = CloudSyncSettings::load_from_path(&config_path);
    assert!(loaded_after.google_drive.is_none());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_safe_subpath_with_cloud_directory() {
    let temp_root = unique_test_temp_dir("safe_sub");
    let cloud_sub = temp_root.join("SottoCartellaCloud");
    let _ = fs::create_dir_all(&cloud_sub);

    let outside = temp_root
        .parent()
        .unwrap()
        .join(format!("outside_dir_{}", std::process::id()));
    let _ = fs::create_dir_all(&outside);

    assert!(is_safe_subpath(&temp_root, &cloud_sub));
    assert!(!is_safe_subpath(&temp_root, &outside));

    let _ = fs::remove_dir_all(&cloud_sub);
    let _ = fs::remove_dir_all(&outside);
    let _ = fs::remove_dir_all(&temp_root);
}
