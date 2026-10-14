use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Configurazione per un singolo slot colore nel dock laterale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PenSlotConfig {
    pub hex: String,
    pub rgba: (f64, f64, f64, f64),
    pub name: String,
}

impl PenSlotConfig {
    pub fn new(
        hex: impl Into<String>,
        rgba: (f64, f64, f64, f64),
        name: impl Into<String>,
    ) -> Self {
        Self {
            hex: hex.into(),
            rgba,
            name: name.into(),
        }
    }
}

/// Preferenze dell'applicazione UniNote per dock laterale, calibri e palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPreferences {
    pub dock_y: f64,
    pub stroke_width: f64,
    pub slots: Vec<PenSlotConfig>,
    pub active_slot_index: usize,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            dock_y: 190.0,
            stroke_width: 2.8,
            active_slot_index: 0,
            slots: vec![
                PenSlotConfig::new("#6DD2FF", (0.427, 0.824, 1.0, 1.0), "Ciano Elettrico"),
                PenSlotConfig::new("#D9E4EB", (0.850, 0.894, 0.920, 1.0), "Bianco Caldo"),
                PenSlotConfig::new("#FFB4AB", (1.0, 0.706, 0.670, 1.0), "Corallo"),
                PenSlotConfig::new("#79D9A5", (0.475, 0.851, 0.647, 1.0), "Menta"),
                PenSlotConfig::new("#FFD875", (1.0, 0.847, 0.459, 1.0), "Oro Brillante"),
            ],
        }
    }
}

impl AppPreferences {
    /// Restituisce il percorso standard del file di configurazione `$XDG_CONFIG_HOME/uninote/preferences.json`.
    pub fn config_path() -> PathBuf {
        glib::user_config_dir()
            .join("uninote")
            .join("preferences.json")
    }

    /// Carica le preferenze dal file standard; se non trovato o in caso di errore, restituisce `Default`.
    pub fn load() -> Self {
        Self::load_from_path(&Self::config_path()).unwrap_or_default()
    }

    /// Carica e deserializza le preferenze da un percorso specifico.
    pub fn load_from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let mut prefs: Self = serde_json::from_str(&content)?;
        if prefs.slots.is_empty() {
            prefs.slots = Self::default().slots;
        }
        if prefs.active_slot_index >= prefs.slots.len() {
            prefs.active_slot_index = 0;
        }
        Ok(prefs)
    }

    /// Salva le preferenze nel file standard `$XDG_CONFIG_HOME/uninote/preferences.json`.
    pub fn save(&self) -> Result<(), std::io::Error> {
        self.save_to_path(&Self::config_path())
    }

    /// Salva le preferenze serializzate in formato JSON formattato su un percorso specifico in modo atomico.
    pub fn save_to_path(&self, path: &Path) -> Result<(), std::io::Error> {
        if path.is_dir() {
            return Err(std::io::Error::other("target path is a directory"));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp_path = PathBuf::from(format!("{}.tmp.{}", path.display(), std::process::id()));

        std::fs::write(&tmp_path, json)?;
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }

        Ok(())
    }
}
