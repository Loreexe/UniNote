use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uninote_core::model::DocumentFormat;

/// Modalità operativa dello strumento gomma (Tratto Intero o Per Segmento analitico).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum EraserMode {
    #[default]
    WholeStroke,
    Segment,
}

// ============================================================================
// 1. PEN DOCK PREFERENCES (AppPreferences)
// ============================================================================

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
    #[serde(default = "default_eraser_mode")]
    pub eraser_mode: EraserMode,
    #[serde(default = "default_eraser_radius")]
    pub eraser_radius: f64,
}

fn default_eraser_mode() -> EraserMode {
    EraserMode::WholeStroke
}

fn default_eraser_radius() -> f64 {
    16.0
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
            eraser_mode: EraserMode::WholeStroke,
            eraser_radius: 16.0,
        }
    }
}

static TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

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

    /// Carica e deserializza le preferenze da un percorso specifico sanitizzando gli slot.
    pub fn load_from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let mut prefs: Self = serde_json::from_str(&content)?;
        if prefs.slots.is_empty() {
            prefs.slots = Self::default().slots;
        }
        if prefs.active_slot_index >= prefs.slots.len() {
            prefs.active_slot_index = 0;
        }
        if !prefs.eraser_radius.is_finite() {
            prefs.eraser_radius = 16.0;
        } else {
            prefs.eraser_radius = prefs.eraser_radius.clamp(4.0, 64.0);
        }
        Ok(prefs)
    }

    /// Rimuove uno slot penna all'indice specificato se sono presenti almeno 2 slot.
    /// Restituisce `true` se lo slot è stato rimosso, `false` se l'operazione non è consentita (es. solo 1 slot rimasto).
    pub fn remove_slot(&mut self, index: usize) -> bool {
        if self.slots.len() <= 1 || index >= self.slots.len() {
            return false;
        }
        self.slots.remove(index);
        if self.active_slot_index >= self.slots.len() {
            self.active_slot_index = self.slots.len() - 1;
        } else if self.active_slot_index > index {
            self.active_slot_index -= 1;
        }
        true
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
        let tmp_id = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp_path = PathBuf::from(format!(
            "{}.tmp.{}.{}",
            path.display(),
            std::process::id(),
            tmp_id
        ));

        std::fs::write(&tmp_path, json)?;
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }

        Ok(())
    }
}

// ============================================================================
// 2. DASHBOARD & RECENT DOCUMENTS STORE (DashboardRecentStore)
// ============================================================================

/// Metadati di un documento recente per la Dashboard accademica.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecentDocumentMeta {
    pub file_path: String,
    pub title: String,
    pub format: DocumentFormat,
    pub last_modified_timestamp: u64,
    pub page_count: usize,
    pub thumbnail_path: Option<String>,
}

/// Gestore e archivio persistente dei documenti recenti della Dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardRecentStore {
    pub default_notes_dir: String,
    pub recent_files: Vec<RecentDocumentMeta>,
}

impl Default for DashboardRecentStore {
    fn default() -> Self {
        let default_dir = glib::user_special_dir(glib::UserDirectory::Documents)
            .map(|p| p.join("UniNote"))
            .unwrap_or_else(|| PathBuf::from("~/Documents/UniNote"))
            .to_string_lossy()
            .to_string();

        Self {
            default_notes_dir: default_dir,
            recent_files: Vec::new(),
        }
    }
}

impl DashboardRecentStore {
    pub const MAX_RECENT_ITEMS: usize = 30;

    pub fn new() -> Self {
        Self::default()
    }

    /// Restituisce il percorso standard del file di configurazione `$XDG_CONFIG_HOME/uninote/recent_documents.json`.
    pub fn config_path() -> PathBuf {
        glib::user_config_dir()
            .join("uninote")
            .join("recent_documents.json")
    }

    /// Carica o crea l'archivio con graceful fallback ai valori predefiniti.
    pub fn load_or_default(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(mut store) = serde_json::from_str::<Self>(&content) {
                store.sanitize_existing_files();
                return store;
            }
        }
        Self::default()
    }

    /// Carica l'archivio dal percorso predefinito.
    pub fn load() -> Self {
        Self::load_or_default(&Self::config_path())
    }

    /// Salva atomicamente l'archivio dei recenti nel percorso indicato.
    pub fn save_atomic(&self, path: &Path) -> Result<(), std::io::Error> {
        if path.is_dir() {
            return Err(std::io::Error::other("target path is a directory"));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp_id = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp_path = PathBuf::from(format!(
            "{}.tmp.{}.{}",
            path.display(),
            std::process::id(),
            tmp_id
        ));

        std::fs::write(&tmp_path, json)?;
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        Ok(())
    }

    /// Salva l'archivio nel percorso standard.
    pub fn save(&self) -> Result<(), std::io::Error> {
        self.save_atomic(&Self::config_path())
    }

    /// Aggiunge o aggiorna un documento recente in cima all'elenco rispettando il limite di 30 elementi.
    pub fn add_recent(&mut self, meta: RecentDocumentMeta) {
        self.recent_files.retain(|r| r.file_path != meta.file_path);
        self.recent_files.insert(0, meta);
        if self.recent_files.len() > Self::MAX_RECENT_ITEMS {
            self.recent_files.truncate(Self::MAX_RECENT_ITEMS);
        }
    }

    /// Rimuove dalla cronologia i file eliminati o non più presenti su disco.
    pub fn sanitize_existing_files(&mut self) {
        self.recent_files
            .retain(|r| Path::new(&r.file_path).exists());
    }

    /// Alias per la sanitizzazione.
    pub fn sanitize_recents(&mut self) {
        self.sanitize_existing_files();
    }

    /// Filtra i documenti recenti in base a una stringa di ricerca (case-insensitive su titolo).
    pub fn filter_by_query(&self, query: &str) -> Vec<RecentDocumentMeta> {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return self.recent_files.clone();
        }
        self.recent_files
            .iter()
            .filter(|r| r.title.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    /// Assicura che la directory predefinita per gli appunti esista su disco, creandola se necessario.
    pub fn ensure_default_notes_dir(&self) -> std::io::Result<()> {
        let p = Path::new(&self.default_notes_dir);
        if !p.exists() {
            std::fs::create_dir_all(p)?;
        }
        Ok(())
    }

    /// Trova il prossimo nome e percorso disponibile per "Nuova nota n.uninote" nella directory predefinita.
    pub fn next_available_note_path(&self) -> (String, PathBuf) {
        next_available_note_path_in_dir(Path::new(&self.default_notes_dir))
    }

    /// Rimuove un file specifico dall'elenco dei recenti.
    pub fn remove_recent(&mut self, file_path: &str) {
        self.recent_files.retain(|r| r.file_path != file_path);
    }

    /// Rinomina un documento recente sia nel percorso che nel titolo.
    pub fn rename_recent(&mut self, old_path: &str, new_path: &str, new_title: &str) {
        if let Some(doc) = self
            .recent_files
            .iter_mut()
            .find(|r| r.file_path == old_path)
        {
            doc.file_path = new_path.to_string();
            doc.title = new_title.to_string();
            doc.last_modified_timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
        }
    }

    /// Aggiorna i percorsi dei documenti memorizzati quando una cartella viene rinominata.
    pub fn update_folder_rename(&mut self, old_dir: &Path, new_dir: &Path) {
        for doc in &mut self.recent_files {
            let doc_p = PathBuf::from(&doc.file_path);
            if let Ok(rel) = doc_p.strip_prefix(old_dir) {
                doc.file_path = new_dir.join(rel).to_string_lossy().to_string();
            }
        }
    }

    /// Rimuove dall'archivio dei recenti tutti i documenti situati sotto la directory indicata.
    pub fn remove_recents_under(&mut self, dir: &Path) {
        self.recent_files.retain(|doc| {
            let doc_p = Path::new(&doc.file_path);
            !doc_p.starts_with(dir)
        });
    }
}

/// Trova il prossimo nome e percorso disponibile per "Nuova nota n.uninote" all'interno di una cartella specifica.
pub fn next_available_note_path_in_dir(dir: &Path) -> (String, PathBuf) {
    let mut idx = 1;
    loop {
        let title = format!("Nuova nota {}", idx);
        let candidate = dir.join(format!("{}.uninote", title));
        if !candidate.exists() {
            return (title, candidate);
        }
        idx += 1;
    }
}

// ============================================================================
// 3. FULL SETTINGS MODEL (FullAppSettingsModel)
// ============================================================================

/// Curva di risposta alla pressione dello stilo MPP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PressureCurve {
    #[default]
    Linear,
    Soft,
    Hard,
}

impl PressureCurve {
    /// Mappa la pressione grezza [0.0, 1.0] sulla risposta calibrata.
    pub fn apply(&self, raw: f64) -> f64 {
        let p = raw.clamp(0.0, 1.0);
        match self {
            Self::Linear => p,
            Self::Soft => p.sqrt(),
            Self::Hard => p.powi(2),
        }
    }
}

/// Preset di sensibilità dello stilo esposti nella vista base delle impostazioni.
///
/// I preset sono pensati per utenti non tecnici: ognuno configura internamente
/// la curva di pressione tecnica [`PressureCurve`] del digitizer MPP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum StylusSensitivity {
    /// Morbida: risposta ad alta sensibilità, tratto leggero anche con pressioni basse.
    Soft,
    /// Media: risposta naturale e bilanciata (predefinito).
    #[default]
    Medium,
    /// Dura: tratto marcato, privilegia le pressioni più intense.
    Hard,
}

impl StylusSensitivity {
    /// Etichette localizzate dei preset mostrati nella vista base.
    pub const LABELS: [&'static str; 3] = ["Morbida", "Media", "Dura"];

    /// Indice del preset nella vista base (0 = Morbida, 1 = Media, 2 = Dura).
    pub fn index(&self) -> u32 {
        match self {
            Self::Soft => 0,
            Self::Medium => 1,
            Self::Hard => 2,
        }
    }

    /// Ricostruisce il preset a partire dall'indice della vista base.
    pub fn from_index(index: u32) -> Self {
        match index {
            0 => Self::Soft,
            2 => Self::Hard,
            _ => Self::Medium,
        }
    }

    /// Curva di pressione tecnica configurata internamente dal preset.
    pub fn curve(&self) -> PressureCurve {
        match self {
            Self::Soft => PressureCurve::Soft,
            Self::Medium => PressureCurve::Linear,
            Self::Hard => PressureCurve::Hard,
        }
    }

    /// Preset base corrispondente a una curva di pressione tecnica.
    pub fn from_curve(curve: PressureCurve) -> Self {
        match curve {
            PressureCurve::Soft => Self::Soft,
            PressureCurve::Hard => Self::Hard,
            PressureCurve::Linear => Self::Medium,
        }
    }

    /// Applica la curva calibrata del preset alla pressione grezza [0.0, 1.0].
    pub fn apply(&self, raw: f64) -> f64 {
        self.curve().apply(raw)
    }
}

/// Modalità tema dell'interfaccia DMS / GTK4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    #[default]
    System,
    ForceDarkOled,
    ForceLight,
}

/// Azione associata al pulsante barilotto dello stilo MPP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum BarrelButtonAction {
    #[default]
    InstantEraser,
    LassoSelect,
    ColorPicker,
}

/// Modello completo delle preferenze strutturato secondo le 5 schede di `UniNoteSettingsWindow`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FullAppSettingsModel {
    // Vista avanzata (parametri tecnici di calibrazione fine)
    #[serde(default)]
    pub advanced_options_enabled: bool,

    // Tab 1: Hardware MPP
    pub min_contact_pressure: f64,
    pub pressure_curve: PressureCurve,
    /// Preset di sensibilità accessibile nella vista base (Morbida/Media/Dura).
    #[serde(default)]
    pub stylus_sensitivity: StylusSensitivity,
    /// Soglia di distacco stilo dal vetro (cutoff lift-off in pressione grezza).
    #[serde(default = "default_stylus_lift_off_cutoff")]
    pub stylus_lift_off_cutoff: f64,
    pub anti_hover_blip_threshold: f64,
    /// Filtro anti-hover MPP attivo (scarta i campioni in aria a pressione sub-soglia).
    #[serde(default = "default_true")]
    pub anti_hover_enabled: bool,
    /// Protezione palmo attiva su touchscreen.
    #[serde(default = "default_true")]
    pub palm_rejection_enabled: bool,
    pub one_euro_filter_enabled: bool,
    pub one_euro_cutoff_hz: f64,
    /// Coefficiente di velocità beta del filtro One Euro.
    #[serde(default = "default_one_euro_beta")]
    pub one_euro_beta: f64,
    pub palm_proximity_timeout_ms: u64,
    pub palm_cooldown_ms: u64,
    pub hud_telemetry_enabled: bool,

    // Tab 2: Appearance & DMS
    pub theme_mode: ThemeMode,
    pub adapt_ink_rec709: bool,
    pub dark_sheet_ruling_contrast: f64,
    pub floating_bars_opacity: f64,

    // Tab 3: Tools & Shapes
    pub orthogonal_snap_tolerance_deg: f64,
    pub rdp_epsilon_factor: f64,

    // Tab 4: Shortcuts & Buttons
    pub shortcuts: HashMap<String, String>,
    pub barrel_button_action: BarrelButtonAction,
}

/// Valore booleano predefinito condiviso per i nuovi campi opzionali del modello.
fn default_true() -> bool {
    true
}

/// Coefficiente di velocità predefinito del filtro One Euro (1€).
fn default_one_euro_beta() -> f64 {
    0.04
}

/// Soglia di distacco stilo predefinita (cutoff lift-off in pressione grezza).
fn default_stylus_lift_off_cutoff() -> f64 {
    0.045
}

impl Default for FullAppSettingsModel {
    fn default() -> Self {
        let mut shortcuts = HashMap::new();
        for (id, _, _, def) in Self::SHORTCUT_SPECS {
            shortcuts.insert(id.to_string(), def.to_string());
        }

        Self {
            advanced_options_enabled: false,

            min_contact_pressure: 0.05,
            pressure_curve: PressureCurve::Linear,
            stylus_sensitivity: StylusSensitivity::Medium,
            stylus_lift_off_cutoff: 0.045,
            anti_hover_blip_threshold: 0.065,
            anti_hover_enabled: true,
            palm_rejection_enabled: true,
            one_euro_filter_enabled: true,
            one_euro_cutoff_hz: 1.0,
            one_euro_beta: 0.04,
            palm_proximity_timeout_ms: 300,
            palm_cooldown_ms: 200,
            hud_telemetry_enabled: false,

            theme_mode: ThemeMode::System,
            adapt_ink_rec709: true,
            dark_sheet_ruling_contrast: 0.18,
            floating_bars_opacity: 0.95,

            orthogonal_snap_tolerance_deg: 8.0,
            rdp_epsilon_factor: 0.04,

            shortcuts,
            barrel_button_action: BarrelButtonAction::InstantEraser,
        }
    }
}

impl FullAppSettingsModel {
    /// Specifiche standard delle scorciatoie da tastiera (id, nome breve, descrizione, acceleratore predefinito).
    pub const SHORTCUT_SPECS: &'static [(
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    )] = &[
        (
            "undo",
            "Annulla Tratto / Spostamento",
            "Annulla l'ultimo tratto o spostamento effettuato",
            "<Primary>z",
        ),
        (
            "redo",
            "Ripeti Tratto",
            "Ripete l'ultima operazione annullata",
            "<Primary>y",
        ),
        (
            "save",
            "Salva Quaderno",
            "Salva le modifiche nel file corrente",
            "<Primary>s",
        ),
        (
            "open",
            "Apri File...",
            "Apre un documento esistente da disco",
            "<Primary>o",
        ),
        (
            "new",
            "Nuovo Documento",
            "Crea un nuovo quaderno A4 o lavagna",
            "<Primary>n",
        ),
        (
            "home",
            "Hub / Dashboard Documenti",
            "Mostra la dashboard dei quaderni",
            "<Primary>h",
        ),
        (
            "telemetry",
            "Overlay Telemetria Diagnostica",
            "Attiva o disattiva l'HUD diagnostico a schermo",
            "F12",
        ),
        (
            "zoom_reset",
            "Reimposta Zoom al 100%",
            "Riporta il livello di zoom al 100% (scala 1:1)",
            "<Primary>0",
        ),
        (
            "copy",
            "Copia Selezione",
            "Copia gli elementi selezionati negli appunti",
            "<Primary>c",
        ),
        (
            "cut",
            "Taglia Selezione",
            "Taglia gli elementi selezionati negli appunti",
            "<Primary>x",
        ),
        (
            "paste",
            "Incolla Selezione",
            "Incolla gli elementi dagli appunti",
            "<Primary>v",
        ),
    ];

    /// Restituisce la scorciatoia configurata per l'ID azione specificato o il default.
    pub fn get_shortcut(&self, id: &str) -> String {
        self.shortcuts
            .get(id)
            .filter(|s| !s.trim().is_empty())
            .cloned()
            .unwrap_or_else(|| {
                Self::SHORTCUT_SPECS
                    .iter()
                    .find(|(k, _, _, _)| *k == id)
                    .map(|(_, _, _, def)| def.to_string())
                    .unwrap_or_default()
            })
    }

    /// Imposta e memorizza la combinazione di tasti per l'ID azione specificato.
    pub fn set_shortcut(&mut self, id: &str, accel: &str) {
        self.shortcuts
            .insert(id.to_string(), accel.trim().to_string());
    }

    /// Reimposta la combinazione di tasti per una singola azione al suo valore predefinito.
    pub fn reset_shortcut(&mut self, id: &str) {
        if let Some((_, _, _, def)) = Self::SHORTCUT_SPECS.iter().find(|(k, _, _, _)| *k == id) {
            self.shortcuts.insert(id.to_string(), def.to_string());
        }
    }

    /// Reimposta tutte le scorciatoie da tastiera ai valori predefiniti della suite.
    pub fn reset_all_shortcuts(&mut self) {
        for (id, _, _, def) in Self::SHORTCUT_SPECS {
            self.shortcuts.insert(id.to_string(), def.to_string());
        }
    }

    /// Restituisce il percorso standard del file di configurazione `$XDG_CONFIG_HOME/uninote/settings.json`.
    pub fn config_path() -> PathBuf {
        glib::user_config_dir()
            .join("uninote")
            .join("settings.json")
    }

    /// Sanitizza tutti i campi numerici e garantisce la presenza di tutte le scorciatoie standard.
    pub fn sanitize(&mut self) {
        self.min_contact_pressure = self.min_contact_pressure.clamp(0.01, 0.50);
        self.anti_hover_blip_threshold = self.anti_hover_blip_threshold.clamp(0.02, 0.20);
        self.one_euro_cutoff_hz = self.one_euro_cutoff_hz.clamp(0.1, 10.0);
        self.palm_proximity_timeout_ms = self.palm_proximity_timeout_ms.clamp(50, 2000);
        self.palm_cooldown_ms = self.palm_cooldown_ms.clamp(50, 1000);
        self.dark_sheet_ruling_contrast = self.dark_sheet_ruling_contrast.clamp(0.05, 0.50);
        self.floating_bars_opacity = self.floating_bars_opacity.clamp(0.50, 1.0);
        self.orthogonal_snap_tolerance_deg = self.orthogonal_snap_tolerance_deg.clamp(2.0, 25.0);
        self.rdp_epsilon_factor = self.rdp_epsilon_factor.clamp(0.01, 0.15);

        // Campi introdotti dalla vista avanzata: NaN e infiniti vengono riportati ai default.
        self.one_euro_beta = if self.one_euro_beta.is_finite() {
            self.one_euro_beta.clamp(0.0, 1.0)
        } else {
            0.04
        };
        self.stylus_lift_off_cutoff = if self.stylus_lift_off_cutoff.is_finite() {
            self.stylus_lift_off_cutoff.clamp(0.01, 0.20)
        } else {
            0.045
        };

        // Il preset base è la rappresentazione accessibile della curva tecnica: i due
        // valori devono restare coerenti anche per i file salvati da versioni precedenti.
        self.stylus_sensitivity = StylusSensitivity::from_curve(self.pressure_curve);

        for (id, _, _, def) in Self::SHORTCUT_SPECS {
            self.shortcuts
                .entry(id.to_string())
                .or_insert_with(|| def.to_string());
        }
    }

    /// Imposta il preset di sensibilità base allineando internamente la curva di pressione.
    pub fn set_stylus_sensitivity(&mut self, sensitivity: StylusSensitivity) {
        self.stylus_sensitivity = sensitivity;
        self.pressure_curve = sensitivity.curve();
    }

    /// Imposta la curva di pressione tecnica allineando il preset di sensibilità base.
    pub fn set_pressure_curve(&mut self, curve: PressureCurve) {
        self.pressure_curve = curve;
        self.stylus_sensitivity = StylusSensitivity::from_curve(curve);
    }

    /// Salva le impostazioni atomicamente usando un file temporaneo indicizzato dal PID e da un contatore.
    pub fn save_atomic(&self, path: &Path) -> Result<(), std::io::Error> {
        if path.is_dir() {
            return Err(std::io::Error::other("target path is a directory"));
        }
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        let tmp_id = TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp_path = PathBuf::from(format!(
            "{}.tmp.{}.{}",
            path.display(),
            std::process::id(),
            tmp_id
        ));

        std::fs::write(&tmp_path, json)?;
        if let Err(e) = std::fs::rename(&tmp_path, path) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(e);
        }
        Ok(())
    }

    /// Carica le impostazioni dal percorso specificato con fallback automatico ai valori predefiniti.
    pub fn load_or_default(path: &Path) -> Self {
        if let Ok(content) = std::fs::read_to_string(path) {
            if let Ok(mut settings) = serde_json::from_str::<Self>(&content) {
                settings.sanitize();
                return settings;
            }
        }
        Self::default()
    }

    /// Carica le impostazioni dal percorso standard.
    pub fn load() -> Self {
        Self::load_or_default(&Self::config_path())
    }

    /// Salva le impostazioni nel percorso standard.
    pub fn save(&self) -> Result<(), std::io::Error> {
        self.save_atomic(&Self::config_path())
    }
}
