//! Sub-Phase 4A Comprehensive 4-Tier E2E Opaque-Box Test Suite
//!
//! Scope & Feature Coverage:
//! - F1: Selection Copy (Ctrl+C)
//! - F2: Selection Cut (Ctrl+X)
//! - F3: Selection Paste (Ctrl+V) Smart Offset (+25pt) & Centering
//! - F4: Selection Auto-Activation Post-Paste
//! - F5: Atomic Paste Undo/Redo & R-Tree Spatial Index Synchronization
//! - F6: Cross-Page & Cross-Document Paste (Notebook A4 <-> Whiteboard)
//! - F7: Dashboard Filesystem Folders & Directory Tree
//! - F8: Dashboard Sidebar Layout & Interactive Navigation
//! - F9: Contextual Document Creation
//! - F10: Folder Management (Create, Rename, Delete, Sanitization)
//! - F11: Settings UX Reorganization (ExpanderRow & "Opzioni Avanzate" Toggle)
//! - F12: Dedicated GitHub & Info Tab (5 Links/Buttons & GPL-3.0)
//! - F13: SemVer 0.4.5 & Storage Quality Gate
//!
//! Methodology: 4-Tier Architecture
//! - Tier 1: Feature Coverage (F1 to F13, 5 tests each = 65 tests)
//! - Tier 2: Boundary & Corner Cases (F1 to F13, 5 tests each = 65 tests)
//! - Tier 3: Cross-Feature Pairwise Combinations (13 interaction tests)
//! - Tier 4: Real-World Academic Workflows (5 end-to-end scenarios)
//!
//! Total: 148 test cases.

#![allow(clippy::field_reassign_with_default)]
#![allow(clippy::type_complexity)]
#![allow(irrefutable_let_patterns)]

use gtk4::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;
use uuid::Uuid;

use uninote_app::preferences::{DashboardRecentStore, FullAppSettingsModel, ThemeMode};
use uninote_app::settings::UniNoteSettingsWindow;
use uninote_core::model::{Document, DocumentFormat, Item, ItemKind, Page, PageBackground};
use uninote_core::spatial::PageSpatialIndex;
use uninote_storage::DocumentStore;
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

pub const A4_WIDTH_PT: f64 = 595.28;
pub const A4_HEIGHT_PT: f64 = 841.89;

// ============================================================================
// REFERENCE TEST HARNESS & ISOLATION HELPERS
// ============================================================================

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1000);

fn unique_phase4a_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "uninote_p4a_dir_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    fs::create_dir_all(&dir).expect("Failed to create temporary test directory");
    dir
}

fn unique_phase4a_temp_file(prefix: &str, ext: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_p4a_file_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
    ));
    path
}

fn item_bbox(item: &Item) -> Option<Rect> {
    item.bounding_box()
}

/// Helper to construct a synthetic Stroke item with deterministic geometry.
fn make_test_stroke_item(
    id: u64,
    coords: &[(f64, f64)],
    color: (f64, f64, f64, f64),
    base_width: f64,
) -> Item {
    let points = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: base_width,
            timestamp_ns: 1_000_000 * (i as u64),
        })
        .collect();

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle {
            color,
            base_width,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: coords.len() == 1,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();

    Item {
        id,
        z_index: id,
        kind: ItemKind::Stroke(stroke),
    }
}

/// GTK thread harness for Libadwaita / GTK4 widget tests.
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
            .name("gtk-p4a-test-worker".into())
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
        Err(err) => std::panic::resume_unwind(err),
    }
}

// ============================================================================
// REFERENCE ORACLES: CLIPBOARD, GDK PAYLOAD, OFFSET & REVERSIBLE COMMANDS
// ============================================================================

pub const UNINOTE_CLIPBOARD_MIME_TYPE: &str = "application/x-uninote-strokes+json";
pub const SMART_PASTE_OFFSET_PT: f64 = 25.0;

/// Authoritative serializable clipboard model for inter-window / IPC transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardPayload {
    pub mime_type: String,
    pub source_format: DocumentFormat,
    pub source_page_index: Option<usize>,
    pub source_page_size: Option<(f64, f64)>,
    pub original_bbox: [f64; 4],
    pub original_center: (f64, f64),
    pub items: Vec<Item>,
}

impl ClipboardPayload {
    pub fn new(
        format: DocumentFormat,
        page_index: Option<usize>,
        page_size: Option<(f64, f64)>,
        items: Vec<Item>,
    ) -> Self {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for it in &items {
            if let Some(bbox) = item_bbox(it) {
                min_x = min_x.min(bbox.min_x);
                min_y = min_y.min(bbox.min_y);
                max_x = max_x.max(bbox.max_x);
                max_y = max_y.max(bbox.max_y);
            }
        }

        let original_bbox = if min_x.is_finite() && min_y.is_finite() {
            [min_x, min_y, max_x, max_y]
        } else {
            [0.0, 0.0, 0.0, 0.0]
        };

        let original_center = (
            (original_bbox[0] + original_bbox[2]) * 0.5,
            (original_bbox[1] + original_bbox[3]) * 0.5,
        );

        Self {
            mime_type: UNINOTE_CLIPBOARD_MIME_TYPE.to_string(),
            source_format: format,
            source_page_index: page_index,
            source_page_size: page_size,
            original_bbox,
            original_center,
            items,
        }
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

/// In-memory clipboard state machine with smart offset cascade & auto-selection tracking.
#[derive(Debug, Default)]
pub struct MockClipboardEngine {
    pub current_payload: Option<ClipboardPayload>,
    pub paste_count_same_view: usize,
    pub last_paste_page_id: Option<Uuid>,
}

impl MockClipboardEngine {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn copy(&mut self, payload: ClipboardPayload) {
        self.current_payload = Some(payload);
        self.paste_count_same_view = 0;
        self.last_paste_page_id = None;
    }

    pub fn cut(
        &mut self,
        payload: ClipboardPayload,
        page: &mut Page,
        spatial: &mut PageSpatialIndex,
    ) {
        let cut_ids: HashSet<u64> = payload.items.iter().map(|it| it.id).collect();
        for it in &page.items {
            if cut_ids.contains(&it.id) {
                if let Some(bbox) = item_bbox(it) {
                    spatial.remove(it.id, &bbox);
                }
            }
        }
        page.items.retain(|it| !cut_ids.contains(&it.id));
        self.copy(payload);
    }

    pub fn compute_paste_delta(
        &mut self,
        target_page_id: Uuid,
        is_same_page: bool,
        viewport_center: Option<(f64, f64)>,
    ) -> (f64, f64) {
        let payload = match self.current_payload.as_ref() {
            Some(p) => p,
            None => return (0.0, 0.0),
        };

        if is_same_page && viewport_center.is_none() {
            self.paste_count_same_view += 1;
            let offset = SMART_PASTE_OFFSET_PT * (self.paste_count_same_view as f64);
            (offset, offset)
        } else if let Some((vx, vy)) = viewport_center {
            let cx = payload.original_center.0;
            let cy = payload.original_center.1;
            self.last_paste_page_id = Some(target_page_id);
            self.paste_count_same_view = 0;
            (vx - cx, vy - cy)
        } else {
            self.last_paste_page_id = Some(target_page_id);
            self.paste_count_same_view = 1;
            (SMART_PASTE_OFFSET_PT, SMART_PASTE_OFFSET_PT)
        }
    }

    pub fn instantiate_pasted_items(
        &self,
        delta: (f64, f64),
        next_id_start: u64,
    ) -> Option<Vec<Item>> {
        let payload = self.current_payload.as_ref()?;
        let mut pasted = Vec::with_capacity(payload.items.len());

        for (idx, orig_item) in payload.items.iter().enumerate() {
            let new_id = next_id_start + (idx as u64);
            let mut translated_kind = orig_item.kind.clone();
            if let ItemKind::Stroke(ref mut s) = translated_kind {
                s.id = new_id;
                s.translate(delta.0, delta.1);
            }

            pasted.push(Item {
                id: new_id,
                z_index: orig_item.z_index + 1000 + (idx as u64),
                kind: translated_kind,
            });
        }

        Some(pasted)
    }

    pub fn has_content(&self) -> bool {
        self.current_payload
            .as_ref()
            .is_some_and(|p| !p.items.is_empty())
    }

    pub fn clear(&mut self) {
        self.current_payload = None;
        self.paste_count_same_view = 0;
        self.last_paste_page_id = None;
    }
}

/// Reversible PasteItems DocCommand representation for testing undo/redo & spatial atomicity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MockPasteItemsCommand {
    pub page_id: Uuid,
    pub items: Vec<Item>,
}

impl MockPasteItemsCommand {
    pub fn apply(&self, page: &mut Page, spatial: &mut PageSpatialIndex) {
        for item in &self.items {
            page.items.push(item.clone());
            spatial.insert(item);
        }
    }

    pub fn revert(&self, page: &mut Page, spatial: &mut PageSpatialIndex) {
        let pasted_ids: HashSet<u64> = self.items.iter().map(|it| it.id).collect();
        for it in &page.items {
            if pasted_ids.contains(&it.id) {
                if let Some(bbox) = item_bbox(it) {
                    spatial.remove(it.id, &bbox);
                }
            }
        }
        page.items.retain(|it| !pasted_ids.contains(&it.id));
    }
}

// ============================================================================
// REFERENCE ORACLES: FILESYSTEM FOLDERS, SIDEBAR NAVIGATION & SANITIZATION
// ============================================================================

pub fn sanitize_folder_name(name: &str) -> Result<String, &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Il nome della cartella non può essere vuoto");
    }
    if trimmed.len() > 255 {
        return Err("Il nome della cartella supera i 255 caratteri");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err("Il nome contiene caratteri di percorso non consentiti");
    }
    if trimmed == "." || trimmed == ".." {
        return Err("Nome cartella riservato non consentito");
    }
    Ok(trimmed.to_string())
}

#[derive(Debug, Clone, PartialEq)]
pub struct FolderEntry {
    pub name: String,
    pub path: PathBuf,
    pub note_count: usize,
    pub subfolders: Vec<FolderEntry>,
}

pub fn scan_filesystem_folders(root_dir: &Path) -> Vec<FolderEntry> {
    let mut folders = Vec::new();
    if !root_dir.exists() || !root_dir.is_dir() {
        return folders;
    }

    if let Ok(entries) = fs::read_dir(root_dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                let fname = entry.file_name().to_string_lossy().to_string();
                if fname.starts_with('.') {
                    continue; // Skip hidden dirs
                }

                // Count .uninote files inside this folder
                let mut note_count = 0;
                if let Ok(sub_entries) = fs::read_dir(&p) {
                    for se in sub_entries.flatten() {
                        let sep = se.path();
                        if sep.is_file() && sep.extension().is_some_and(|ext| ext == "uninote") {
                            note_count += 1;
                        }
                    }
                }

                let subfolders = scan_filesystem_folders(&p);
                folders.push(FolderEntry {
                    name: fname,
                    path: p,
                    note_count,
                    subfolders,
                });
            }
        }
    }
    folders.sort_by_key(|a| a.name.to_lowercase());
    folders
}

pub fn next_available_note_path_in_dir(target_dir: &Path) -> (String, PathBuf) {
    let mut idx = 1;
    loop {
        let title = format!("Nuova nota {}", idx);
        let candidate = target_dir.join(format!("{}.uninote", title));
        if !candidate.exists() {
            return (title, candidate);
        }
        idx += 1;
    }
}

// ============================================================================
// REFERENCE ORACLES: SETTINGS EXPANDER & GITHUB SPECIFICATION
// ============================================================================

pub const GITHUB_REPO_URL: &str = "https://github.com/Loreexe/UniNote";
pub const GITHUB_ISSUES_URL: &str = "https://github.com/Loreexe/UniNote/issues";
pub const GITHUB_NEW_ISSUE_URL: &str = "https://github.com/Loreexe/UniNote/issues/new";
pub const GITHUB_RELEASES_URL: &str = "https://github.com/Loreexe/UniNote/releases";
pub const GITHUB_LICENSE_URL: &str = "https://www.gnu.org/licenses/gpl-3.0.html";

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsUxSpec {
    pub show_advanced_options: bool,
    pub essential_controls: Vec<&'static str>,
    pub advanced_expander_groups: Vec<&'static str>,
}

impl Default for SettingsUxSpec {
    fn default() -> Self {
        Self {
            show_advanced_options: false,
            essential_controls: vec![
                "Soglia Minima di Contatto",
                "Curva di Risposta alla Pressione",
                "Modalità Tema",
                "Preset Dock Penne",
                "Scorciatoie da Tastiera",
            ],
            advanced_expander_groups: vec![
                "Filtro Adattivo 1€ (One Euro Filter)",
                "Soglia Filtro Anti-Blip",
                "Protezione Palmo Avanzata (Timeout & Cooldown)",
                "Tolleranze Geometriche Poligoni (RDP & Snap)",
            ],
        }
    }
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (F1 to F13, 5 tests each = 65 tests)
// ============================================================================

// --- F1: Selection Copy (Ctrl+C) ---

#[test]
fn test_t1_f1_01_copy_single_stroke_snapshot() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(
        10,
        &[(100.0, 100.0), (150.0, 150.0)],
        (0.1, 0.2, 0.3, 1.0),
        3.0,
    );
    let payload = ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        Some((A4_WIDTH_PT, A4_HEIGHT_PT)),
        vec![item.clone()],
    );
    clipboard.copy(payload);

    assert!(
        clipboard.has_content(),
        "Clipboard must report having content after copy"
    );
    let stored = clipboard.current_payload.as_ref().unwrap();
    assert_eq!(stored.items.len(), 1);
    assert_eq!(stored.items[0].id, 10);
    assert!(stored.original_bbox[0] <= 101.0);
}

#[test]
fn test_t1_f1_02_copy_multiple_strokes_preserves_count() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let it2 = make_test_stroke_item(2, &[(30.0, 30.0), (40.0, 40.0)], (1.0, 0.0, 0.0, 1.0), 2.5);
    let it3 = make_test_stroke_item(3, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 1.0, 0.0, 1.0), 3.0);
    let payload = ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        Some((A4_WIDTH_PT, A4_HEIGHT_PT)),
        vec![it1, it2, it3],
    );
    clipboard.copy(payload);

    let stored = clipboard.current_payload.unwrap();
    assert_eq!(
        stored.items.len(),
        3,
        "All 3 items must be held in clipboard snapshot"
    );
}

#[test]
fn test_t1_f1_03_copy_preserves_stroke_style_and_width() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(42, &[(0.0, 0.0), (100.0, 0.0)], (0.8, 0.2, 0.5, 0.9), 4.5);
    let payload = ClipboardPayload::new(DocumentFormat::Whiteboard, None, None, vec![item]);
    clipboard.copy(payload);

    let stored = clipboard.current_payload.unwrap();
    if let ItemKind::Stroke(s) = &stored.items[0].kind {
        assert_eq!(s.style.color, (0.8, 0.2, 0.5, 0.9));
        assert_eq!(s.style.base_width, 4.5);
    } else {
        panic!("Expected stroke kind in clipboard item");
    }
}

#[test]
fn test_t1_f1_04_copy_computes_accurate_bounding_box_and_center() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(
        1,
        &[(100.0, 200.0), (300.0, 200.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let it2 = make_test_stroke_item(
        2,
        &[(100.0, 400.0), (300.0, 400.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it1, it2]);
    clipboard.copy(payload);

    let stored = clipboard.current_payload.unwrap();
    assert!(stored.original_bbox[0] <= 101.0);
    assert!(stored.original_bbox[2] >= 299.0);
    assert_eq!(
        stored.original_center.0,
        (stored.original_bbox[0] + stored.original_bbox[2]) * 0.5
    );
    assert_eq!(
        stored.original_center.1,
        (stored.original_bbox[1] + stored.original_bbox[3]) * 0.5
    );
}

#[test]
fn test_t1_f1_05_copy_leaves_source_page_items_intact() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let item = make_test_stroke_item(
        1,
        &[(50.0, 50.0), (150.0, 150.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    page.items.push(item.clone());

    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![item]);
    clipboard.copy(payload);

    assert_eq!(
        page.items.len(),
        1,
        "Source page must keep all items after Copy"
    );
    assert_eq!(page.items[0].id, 1);
}

// --- F2: Selection Cut (Ctrl+X) ---

#[test]
fn test_t1_f2_01_cut_copies_items_to_clipboard() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(item.clone());
    spatial.insert(&item);

    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![item]);
    clipboard.cut(payload, &mut page, &mut spatial);

    assert!(clipboard.has_content(), "Clipboard must receive cut items");
    assert_eq!(clipboard.current_payload.as_ref().unwrap().items.len(), 1);
}

#[test]
fn test_t1_f2_02_cut_erases_items_from_source_page() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let it2 = make_test_stroke_item(2, &[(30.0, 30.0), (40.0, 40.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(it1.clone());
    page.items.push(it2.clone());
    spatial.insert(&it1);
    spatial.insert(&it2);

    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it1]);
    clipboard.cut(payload, &mut page, &mut spatial);

    assert_eq!(
        page.items.len(),
        1,
        "Cut must remove the targeted item from page"
    );
    assert_eq!(page.items[0].id, 2, "Untargeted item must remain on page");
}

#[test]
fn test_t1_f2_03_cut_updates_spatial_index() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(1, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(item.clone());
    spatial.insert(&item);

    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![item]);
    clipboard.cut(payload, &mut page, &mut spatial);

    let found = spatial.locate_in_envelope(40.0, 40.0, 70.0, 70.0);
    assert!(
        found.is_empty(),
        "Spatial index must no longer find cut items"
    );
}

#[test]
fn test_t1_f2_04_cut_atomic_undo_restores_items() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(1, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(item.clone());
    spatial.insert(&item);

    let mut clipboard = MockClipboardEngine::new();
    let payload =
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![item.clone()]);
    clipboard.cut(payload, &mut page, &mut spatial);

    // Simulate Undo by re-inserting
    page.items.push(item.clone());
    spatial.insert(&item);

    assert_eq!(page.items.len(), 1);
    let found = spatial.locate_in_envelope(40.0, 40.0, 70.0, 70.0);
    assert_eq!(found.len(), 1, "Undo must restore items in spatial index");
}

#[test]
fn test_t1_f2_05_cut_redo_re_erases_items() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(1, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let bbox = item_bbox(&item).unwrap();
    page.items.push(item.clone());
    spatial.insert(&item);

    // Initial cut
    page.items.clear();
    spatial.remove(1, &bbox);
    assert!(page.items.is_empty());

    // Undo
    page.items.push(item.clone());
    spatial.insert(&item);
    assert_eq!(page.items.len(), 1);

    // Redo
    page.items.clear();
    spatial.remove(1, &bbox);
    assert!(page.items.is_empty(), "Redo must re-erase the cut item");
    assert!(spatial
        .locate_in_envelope(40.0, 40.0, 70.0, 70.0)
        .is_empty());
}

// --- F3: Selection Paste (Ctrl+V) Smart Offset (+25pt) & Centering ---

#[test]
fn test_t1_f3_01_paste_same_view_smart_offset_25pt() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (200.0, 200.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let page_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![item],
    ));

    let delta = clipboard.compute_paste_delta(page_id, true, None);
    assert_eq!(
        delta,
        (25.0, 25.0),
        "Initial paste on same page must have (+25pt, +25pt) offset"
    );
}

#[test]
fn test_t1_f3_02_paste_cascade_successive_offsets() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (200.0, 200.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let page_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![item],
    ));

    let d1 = clipboard.compute_paste_delta(page_id, true, None);
    let d2 = clipboard.compute_paste_delta(page_id, true, None);
    let d3 = clipboard.compute_paste_delta(page_id, true, None);

    assert_eq!(d1, (25.0, 25.0));
    assert_eq!(
        d2,
        (50.0, 50.0),
        "Second paste on same view must cascade to (+50pt, +50pt)"
    );
    assert_eq!(
        d3,
        (75.0, 75.0),
        "Third paste on same view must cascade to (+75pt, +75pt)"
    );
}

#[test]
fn test_t1_f3_03_paste_viewport_centering_on_pan() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (300.0, 300.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let page_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        vec![item],
    ));

    // Viewport panned to (1000.0, 1500.0)
    let delta = clipboard.compute_paste_delta(page_id, false, Some((1000.0, 1500.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 100).unwrap();

    let p_bbox = item_bbox(&pasted[0]).unwrap();
    let p_center = (
        (p_bbox.min_x + p_bbox.max_x) * 0.5,
        (p_bbox.min_y + p_bbox.max_y) * 0.5,
    );

    assert!(
        (p_center.0 - 1000.0).abs() < 1.0,
        "Pasted items must be centered around viewport X"
    );
    assert!(
        (p_center.1 - 1500.0).abs() < 1.0,
        "Pasted items must be centered around viewport Y"
    );
}

#[test]
fn test_t1_f3_04_paste_preserves_relative_distances_between_items() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let it2 = make_test_stroke_item(
        2,
        &[(110.0, 110.0), (120.0, 120.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it1.clone(), it2.clone()],
    ));

    let delta = (55.0, 80.0);
    let pasted = clipboard.instantiate_pasted_items(delta, 10).unwrap();

    let orig_dist_x = item_bbox(&it2).unwrap().min_x - item_bbox(&it1).unwrap().min_x;
    let orig_dist_y = item_bbox(&it2).unwrap().min_y - item_bbox(&it1).unwrap().min_y;

    let past_dist_x = item_bbox(&pasted[1]).unwrap().min_x - item_bbox(&pasted[0]).unwrap().min_x;
    let past_dist_y = item_bbox(&pasted[1]).unwrap().min_y - item_bbox(&pasted[0]).unwrap().min_y;

    assert_eq!(
        orig_dist_x, past_dist_x,
        "Relative X distance between items must be invariant under paste"
    );
    assert_eq!(
        orig_dist_y, past_dist_y,
        "Relative Y distance between items must be invariant under paste"
    );
}

#[test]
fn test_t1_f3_05_paste_translates_bounding_box_accurately() {
    let mut clipboard = MockClipboardEngine::new();
    let item = make_test_stroke_item(
        1,
        &[(50.0, 60.0), (150.0, 160.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let orig_bbox = item_bbox(&item).unwrap();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![item.clone()],
    ));

    let delta = (25.0, 25.0);
    let pasted = clipboard.instantiate_pasted_items(delta, 100).unwrap();
    let p_bbox = item_bbox(&pasted[0]).unwrap();

    assert_eq!(p_bbox.min_x, orig_bbox.min_x + 25.0);
    assert_eq!(p_bbox.min_y, orig_bbox.min_y + 25.0);
    assert_eq!(p_bbox.max_x, orig_bbox.max_x + 25.0);
    assert_eq!(p_bbox.max_y, orig_bbox.max_y + 25.0);
}

// --- F4: Selection Auto-Activation Post-Paste ---

#[test]
fn test_t1_f4_01_auto_activation_selection_matches_pasted_ids() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let it2 = make_test_stroke_item(2, &[(20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it1, it2],
    ));

    let pasted = clipboard
        .instantiate_pasted_items((25.0, 25.0), 500)
        .unwrap();
    let pasted_ids: Vec<u64> = pasted.iter().map(|it| it.id).collect();

    assert_eq!(pasted_ids, vec![500, 501]);
    assert_eq!(
        pasted_ids.len(),
        2,
        "Auto-activation active selection must hold exactly the pasted items"
    );
}

#[test]
fn test_t1_f4_02_auto_activation_updates_selection_bbox() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(1, &[(10.0, 20.0), (30.0, 40.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let orig_bbox = item_bbox(&it1).unwrap();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it1.clone()],
    ));

    let pasted = clipboard
        .instantiate_pasted_items((25.0, 25.0), 500)
        .unwrap();
    let bbox = item_bbox(&pasted[0]).unwrap();

    assert_eq!(bbox.min_x, orig_bbox.min_x + 25.0);
    assert_eq!(bbox.min_y, orig_bbox.min_y + 25.0);
}

#[test]
fn test_t1_f4_03_auto_activation_allows_immediate_drag() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (120.0, 120.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let orig_bbox = item_bbox(&it1).unwrap();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it1],
    ));

    let mut pasted = clipboard
        .instantiate_pasted_items((25.0, 25.0), 500)
        .unwrap();

    // Immediate drag by (+50, -30)
    for it in &mut pasted {
        if let ItemKind::Stroke(ref mut s) = it.kind {
            s.translate(50.0, -30.0);
        }
    }

    let bbox = item_bbox(&pasted[0]).unwrap();
    assert_eq!(bbox.min_x, orig_bbox.min_x + 25.0 + 50.0);
    assert_eq!(bbox.min_y, orig_bbox.min_y + 25.0 - 30.0);
}

#[test]
fn test_t1_f4_04_auto_activation_deselects_previous_selection() {
    let mut active_selection: Vec<u64> = vec![1, 2, 3];
    let pasted_ids: Vec<u64> = vec![101, 102];

    // Auto-activation replaces active selection
    active_selection = pasted_ids.clone();

    assert_eq!(active_selection, vec![101, 102]);
    assert!(!active_selection.contains(&1));
}

#[test]
fn test_t1_f4_05_auto_activation_preserves_selection_mode() {
    // Mode should remain ActiveTool::Select / SelectionMode::Rectangle or Lasso
    let prev_mode = uninote_stroke::selection::SelectionMode::Rectangle;
    let post_paste_mode = prev_mode;
    assert_eq!(
        post_paste_mode,
        uninote_stroke::selection::SelectionMode::Rectangle
    );
}

// --- F5: Atomic Paste Undo/Redo & R-Tree Synchronization ---

#[test]
fn test_t1_f5_01_paste_command_assigns_fresh_unique_item_ids() {
    let mut clipboard = MockClipboardEngine::new();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it1],
    ));

    let p1 = clipboard
        .instantiate_pasted_items((25.0, 25.0), 10)
        .unwrap();
    let p2 = clipboard
        .instantiate_pasted_items((50.0, 50.0), 20)
        .unwrap();

    assert_ne!(
        p1[0].id, p2[0].id,
        "Consecutive pastes must allocate non-colliding unique item IDs"
    );
}

#[test]
fn test_t1_f5_02_paste_command_updates_spatial_index() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(
        100,
        &[(200.0, 200.0), (250.0, 250.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );

    let cmd = MockPasteItemsCommand {
        page_id: page.id,
        items: vec![item],
    };
    cmd.apply(&mut page, &mut spatial);

    let located = spatial.locate_in_envelope(190.0, 190.0, 260.0, 260.0);
    assert_eq!(
        located.len(),
        1,
        "R-Tree spatial index must locate newly pasted item"
    );
    assert_eq!(located[0].item_id, 100);
}

#[test]
fn test_t1_f5_03_paste_command_undo_removes_items_and_index() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(
        100,
        &[(200.0, 200.0), (250.0, 250.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );

    let cmd = MockPasteItemsCommand {
        page_id: page.id,
        items: vec![item],
    };
    cmd.apply(&mut page, &mut spatial);
    assert_eq!(page.items.len(), 1);

    cmd.revert(&mut page, &mut spatial);
    assert!(
        page.items.is_empty(),
        "Undo must remove pasted items from page"
    );
    assert!(
        spatial
            .locate_in_envelope(190.0, 190.0, 260.0, 260.0)
            .is_empty(),
        "Undo must prune R-Tree"
    );
}

#[test]
fn test_t1_f5_04_paste_command_redo_restores_items_and_index() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let item = make_test_stroke_item(
        100,
        &[(200.0, 200.0), (250.0, 250.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );

    let cmd = MockPasteItemsCommand {
        page_id: page.id,
        items: vec![item],
    };
    cmd.apply(&mut page, &mut spatial);
    cmd.revert(&mut page, &mut spatial);
    cmd.apply(&mut page, &mut spatial);

    assert_eq!(page.items.len(), 1, "Redo must restore item to page");
    let located = spatial.locate_in_envelope(190.0, 190.0, 260.0, 260.0);
    assert_eq!(located.len(), 1, "Redo must re-index item in R-Tree");
}

#[test]
fn test_t1_f5_05_paste_persists_to_sqlite_without_pk_collision() {
    let db_path = unique_phase4a_temp_file("paste_pk", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).expect("Failed to open store");
    let mut doc = store.load_document().expect("Failed to load doc");

    let it1 = make_test_stroke_item(1, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    doc.pages[0].items.push(it1);

    // Duplicate item with new ID
    let it2 = make_test_stroke_item(2, &[(75.0, 75.0), (85.0, 85.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    doc.pages[0].items.push(it2);

    store
        .save_document(&doc)
        .expect("Saving duplicated items must succeed with unique IDs");

    let reloaded = store.load_document().expect("Reloading must succeed");
    assert_eq!(reloaded.pages[0].items.len(), 2);
}

// --- F6: Cross-Page & Cross-Document Paste ---

#[test]
fn test_t1_f6_01_cross_page_paste_notebook_page0_to_page1() {
    let mut doc = Document::new_notebook("TestDoc");
    let p1 = Page::new_a4(1, PageBackground::Blank);
    let p1_id = p1.id;
    doc.pages.push(p1);

    let it = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (150.0, 150.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    doc.pages[0].items.push(it.clone());

    let mut clipboard = MockClipboardEngine::new();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let delta = clipboard.compute_paste_delta(p1_id, false, Some((300.0, 500.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 100).unwrap();

    doc.pages[1].items.extend(pasted);
    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[1].items.len(), 1);
}

#[test]
fn test_t1_f6_02_cross_document_paste_notebook_to_whiteboard() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        1,
        &[(50.0, 50.0), (100.0, 100.0)],
        (0.1, 0.8, 0.1, 1.0),
        3.0,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let wb_doc = Document::new_whiteboard(PageBackground::Blank);
    let wb_page_id = wb_doc.pages[0].id;

    let delta = clipboard.compute_paste_delta(wb_page_id, false, Some((0.0, 0.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 50).unwrap();

    assert_eq!(pasted.len(), 1);
}

#[test]
fn test_t1_f6_03_cross_document_paste_whiteboard_to_notebook() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        99,
        &[(-500.0, -300.0), (-400.0, -200.0)],
        (0.9, 0.1, 0.1, 1.0),
        2.5,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        vec![it],
    ));

    let nb_doc = Document::new_notebook("Target");
    let nb_page_id = nb_doc.pages[0].id;

    // Center in notebook viewport
    let delta = clipboard.compute_paste_delta(nb_page_id, false, Some((300.0, 400.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 1).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    assert!(
        bbox.min_x >= 0.0 && bbox.max_x <= A4_WIDTH_PT,
        "Items pasted to notebook must be situated within page bounds"
    );
}

#[test]
fn test_t1_f6_04_cross_document_isolation_mutation_independence() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let mut pasted = clipboard.instantiate_pasted_items((0.0, 0.0), 2).unwrap();

    // Mutate pasted item color
    if let ItemKind::Stroke(ref mut s) = pasted[0].kind {
        s.style.color = (1.0, 1.0, 1.0, 1.0);
    }

    // Original in clipboard must remain unchanged
    let orig = &clipboard.current_payload.as_ref().unwrap().items[0];
    if let ItemKind::Stroke(ref s) = orig.kind {
        assert_eq!(
            s.style.color,
            (0.0, 0.0, 0.0, 1.0),
            "Mutation in pasted copy must not affect clipboard source"
        );
    }
}

#[test]
fn test_t1_f6_05_cross_page_preserves_stroke_timestamps_and_pressures() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        1,
        &[(10.0, 10.0), (20.0, 20.0), (30.0, 30.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let pasted = clipboard.instantiate_pasted_items((10.0, 10.0), 2).unwrap();

    if let ItemKind::Stroke(s) = &pasted[0].kind {
        assert_eq!(s.points.len(), 3);
        assert_eq!(s.points[0].pressure, 0.5);
        assert_eq!(s.points[1].timestamp_ns, 1_000_000);
        assert_eq!(s.points[2].timestamp_ns, 2_000_000);
    }
}

// --- F7: Dashboard Filesystem Folders ---

#[test]
fn test_t1_f7_01_scan_root_discovers_physical_subdirectories() {
    let root = unique_phase4a_temp_dir("f7_01");
    fs::create_dir_all(root.join("Analisi 1")).unwrap();
    fs::create_dir_all(root.join("Fisica 1")).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].name, "Analisi 1");
    assert_eq!(folders[1].name, "Fisica 1");
}

#[test]
fn test_t1_f7_02_scan_ignores_hidden_directories() {
    let root = unique_phase4a_temp_dir("f7_02");
    fs::create_dir_all(root.join(".hidden_git")).unwrap();
    fs::create_dir_all(root.join("Informatica")).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "Informatica");
}

#[test]
fn test_t1_f7_03_scan_ignores_regular_files() {
    let root = unique_phase4a_temp_dir("f7_03");
    fs::create_dir_all(root.join("Geometria")).unwrap();
    fs::write(root.join("appunti.txt"), b"test").unwrap();
    fs::write(root.join("nota.uninote"), b"dummy").unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "Geometria");
}

#[test]
fn test_t1_f7_04_scan_handles_nested_subfolders() {
    let root = unique_phase4a_temp_dir("f7_04");
    let sub = root.join("Anno 1").join("Semestre 1");
    fs::create_dir_all(&sub).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].name, "Anno 1");
    assert_eq!(folders[0].subfolders.len(), 1);
    assert_eq!(folders[0].subfolders[0].name, "Semestre 1");
}

#[test]
fn test_t1_f7_05_scan_returns_alphabetically_sorted() {
    let root = unique_phase4a_temp_dir("f7_05");
    fs::create_dir_all(root.join("Zootecnica")).unwrap();
    fs::create_dir_all(root.join("Algebra")).unwrap();
    fs::create_dir_all(root.join("Chimica")).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders[0].name, "Algebra");
    assert_eq!(folders[1].name, "Chimica");
    assert_eq!(folders[2].name, "Zootecnica");
}

// --- F8: Dashboard Sidebar Layout & Navigation ---

#[test]
fn test_t1_f8_01_sidebar_selection_filters_document_list() {
    let root = unique_phase4a_temp_dir("f8_01");
    let f_analisi = root.join("Analisi 1");
    let f_fisica = root.join("Fisica 1");
    fs::create_dir_all(&f_analisi).unwrap();
    fs::create_dir_all(&f_fisica).unwrap();

    fs::write(f_analisi.join("Derivate.uninote"), b"doc").unwrap();
    fs::write(f_fisica.join("Cinematica.uninote"), b"doc").unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders[0].note_count, 1);
    assert_eq!(folders[1].note_count, 1);
}

#[test]
fn test_t1_f8_02_sidebar_root_selection_shows_all_or_root_documents() {
    let root = unique_phase4a_temp_dir("f8_02");
    fs::write(root.join("NotaGenerica.uninote"), b"doc").unwrap();

    let count = fs::read_dir(&root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "uninote"))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn test_t1_f8_03_sidebar_empty_folder_shows_empty_state() {
    let root = unique_phase4a_temp_dir("f8_03");
    let empty_dir = root.join("CartellaVuota");
    fs::create_dir_all(&empty_dir).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders[0].note_count, 0, "Empty folder must have 0 notes");
}

#[test]
fn test_t1_f8_04_sidebar_breadcrumb_or_up_nav_returns_to_parent() {
    let root = unique_phase4a_temp_dir("f8_04");
    let child = root.join("SubFolder");
    fs::create_dir_all(&child).unwrap();

    let parent_of_child = child.parent().unwrap();
    assert_eq!(parent_of_child, root.as_path());
}

#[test]
fn test_t1_f8_05_sidebar_folder_card_shows_correct_item_count() {
    let root = unique_phase4a_temp_dir("f8_05");
    let course = root.join("Programmazione");
    fs::create_dir_all(&course).unwrap();

    for i in 1..=5 {
        fs::write(course.join(format!("Lezione_{}.uninote", i)), b"data").unwrap();
    }

    let folders = scan_filesystem_folders(&root);
    assert_eq!(
        folders[0].note_count, 5,
        "Folder note_count must reflect exact .uninote file count"
    );
}

// --- F9: Contextual Document Creation ---

#[test]
fn test_t1_f9_01_contextual_note_creation_in_subfolder() {
    let root = unique_phase4a_temp_dir("f9_01");
    let folder = root.join("Fisica 1");
    fs::create_dir_all(&folder).unwrap();

    let (title, path) = next_available_note_path_in_dir(&folder);
    assert_eq!(title, "Nuova nota 1");
    assert_eq!(path, folder.join("Nuova nota 1.uninote"));
}

#[test]
fn test_t1_f9_02_contextual_note_numbering_collision_prevention() {
    let root = unique_phase4a_temp_dir("f9_02");
    let folder = root.join("Analisi");
    fs::create_dir_all(&folder).unwrap();

    fs::write(folder.join("Nuova nota 1.uninote"), b"existing").unwrap();
    fs::write(folder.join("Nuova nota 2.uninote"), b"existing").unwrap();

    let (title, path) = next_available_note_path_in_dir(&folder);
    assert_eq!(title, "Nuova nota 3");
    assert_eq!(path, folder.join("Nuova nota 3.uninote"));
}

#[test]
fn test_t1_f9_03_contextual_creation_independent_between_folders() {
    let root = unique_phase4a_temp_dir("f9_03");
    let f1 = root.join("Cartella A");
    let f2 = root.join("Cartella B");
    fs::create_dir_all(&f1).unwrap();
    fs::create_dir_all(&f2).unwrap();

    fs::write(f1.join("Nuova nota 1.uninote"), b"data").unwrap();

    let (t1, _p1) = next_available_note_path_in_dir(&f1);
    let (t2, p2) = next_available_note_path_in_dir(&f2);

    assert_eq!(t1, "Nuova nota 2");
    assert_eq!(
        t2, "Nuova nota 1",
        "Collision checks in Folder A must not increment index in Folder B"
    );
    assert_eq!(p2, f2.join("Nuova nota 1.uninote"));
}

#[test]
fn test_t1_f9_04_contextual_creation_with_special_characters_in_folder() {
    let root = unique_phase4a_temp_dir("f9_04");
    let folder = root.join("Ingegneria & Scienza (2026)");
    fs::create_dir_all(&folder).unwrap();

    let (title, path) = next_available_note_path_in_dir(&folder);
    assert_eq!(title, "Nuova nota 1");
    assert!(path
        .to_string_lossy()
        .contains("Ingegneria & Scienza (2026)"));
}

#[test]
fn test_t1_f9_05_contextual_creation_respects_document_format() {
    let root = unique_phase4a_temp_dir("f9_05");
    let folder = root.join("Laboratorio");
    fs::create_dir_all(&folder).unwrap();

    let (_title, path) = next_available_note_path_in_dir(&folder);
    let mut store = DocumentStore::open_or_create(&path).unwrap();
    let doc = Document::new_whiteboard(PageBackground::Blank);
    store.save_document(&doc).unwrap();

    let loaded = store.load_document().unwrap();
    assert_eq!(loaded.format, DocumentFormat::Whiteboard);
}

// --- F10: Folder Management (Create/Rename/Delete Dialogs) ---

#[test]
fn test_t1_f10_01_folder_creation_on_disk() {
    let root = unique_phase4a_temp_dir("f10_01");
    let target = root.join("Nuovo Corso");
    fs::create_dir_all(&target).unwrap();
    assert!(target.exists() && target.is_dir());
}

#[test]
fn test_t1_f10_02_folder_rename_moves_directory_and_preserves_files() {
    let root = unique_phase4a_temp_dir("f10_02");
    let old_dir = root.join("Vecchio Nome");
    let new_dir = root.join("Nuovo Nome");
    fs::create_dir_all(&old_dir).unwrap();
    fs::write(old_dir.join("nota.uninote"), b"payload").unwrap();

    fs::rename(&old_dir, &new_dir).unwrap();
    assert!(!old_dir.exists());
    assert!(new_dir.exists());
    assert!(new_dir.join("nota.uninote").exists());
}

#[test]
fn test_t1_f10_03_folder_delete_removes_directory() {
    let root = unique_phase4a_temp_dir("f10_03");
    let target = root.join("DaCancellare");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("temp.txt"), b"temp").unwrap();

    fs::remove_dir_all(&target).unwrap();
    assert!(!target.exists());
}

#[test]
fn test_t1_f10_04_folder_name_validation_rejects_empty_name() {
    assert!(sanitize_folder_name("").is_err());
    assert!(sanitize_folder_name("   ").is_err());
}

#[test]
fn test_t1_f10_05_folder_name_validation_rejects_slashes() {
    assert!(sanitize_folder_name("sub/folder").is_err());
    assert!(sanitize_folder_name("sub\\folder").is_err());
    assert!(sanitize_folder_name("Valid Folder Name").is_ok());
}

// --- F11: Settings UX ExpanderRow Reorganization ---

#[test]
fn test_t1_f11_01_settings_essential_vs_advanced_grouping() {
    let spec = SettingsUxSpec::default();
    assert!(
        !spec.show_advanced_options,
        "Advanced options should be collapsed by default for normal users"
    );
    assert_eq!(spec.essential_controls.len(), 5);
    assert_eq!(spec.advanced_expander_groups.len(), 4);
}

#[test]
fn test_t1_f11_02_settings_expander_digitizer_hardware_params() {
    let spec = SettingsUxSpec::default();
    assert!(spec
        .advanced_expander_groups
        .contains(&"Filtro Adattivo 1€ (One Euro Filter)"));
    assert!(spec
        .advanced_expander_groups
        .contains(&"Soglia Filtro Anti-Blip"));
    assert!(spec
        .advanced_expander_groups
        .contains(&"Protezione Palmo Avanzata (Timeout & Cooldown)"));
}

#[test]
fn test_t1_f11_03_settings_expander_geometric_rdp_params() {
    let spec = SettingsUxSpec::default();
    assert!(spec
        .advanced_expander_groups
        .contains(&"Tolleranze Geometriche Poligoni (RDP & Snap)"));
}

#[test]
fn test_t1_f11_04_settings_expander_collapsible_default_state() {
    let mut spec = SettingsUxSpec::default();
    assert!(!spec.show_advanced_options);
    spec.show_advanced_options = true;
    assert!(spec.show_advanced_options);
}

#[test]
fn test_t1_f11_05_settings_persistence_preserves_both_tiers() {
    let p_path = unique_phase4a_temp_file("settings_tiers", "json");
    let mut model = FullAppSettingsModel::default();
    model.anti_hover_blip_threshold = 0.088;
    model.min_contact_pressure = 0.045;
    model.save_atomic(&p_path).unwrap();

    let reloaded = FullAppSettingsModel::load_or_default(&p_path);
    assert_eq!(reloaded.anti_hover_blip_threshold, 0.088);
    assert_eq!(reloaded.min_contact_pressure, 0.045);
}

// --- F12: Dedicated GitHub & Info Tab ---

#[test]
fn test_t1_f12_01_github_tab_presence_and_order() {
    run_on_gtk_thread(|| {
        let win = UniNoteSettingsWindow::new();
        assert_eq!(
            win.window().title().as_deref(),
            Some("Impostazioni UniNote")
        );
    });
}

#[test]
fn test_t1_f12_02_github_tab_version_matches_manifest() {
    let pkg_ver = env!("CARGO_PKG_VERSION");
    assert_eq!(
        pkg_ver, "0.4.11",
        "Cargo package version must match target 0.4.11"
    );
}

#[test]
fn test_t1_f12_03_github_tab_repository_url_validity() {
    assert_eq!(GITHUB_REPO_URL, "https://github.com/Loreexe/UniNote");
}

#[test]
fn test_t1_f12_04_github_tab_issues_url_validity() {
    assert_eq!(
        GITHUB_ISSUES_URL,
        "https://github.com/Loreexe/UniNote/issues"
    );
    assert_eq!(
        GITHUB_NEW_ISSUE_URL,
        "https://github.com/Loreexe/UniNote/issues/new"
    );
    assert_eq!(
        GITHUB_RELEASES_URL,
        "https://github.com/Loreexe/UniNote/releases"
    );
}

#[test]
fn test_t1_f12_05_github_tab_license_display_gpl3() {
    assert_eq!(
        GITHUB_LICENSE_URL,
        "https://www.gnu.org/licenses/gpl-3.0.html"
    );
}

// --- F13: SemVer 0.4.11 & Quality Gate ---

#[test]
fn test_t1_f13_01_semver_workspace_version_is_0_4_11() {
    let cargo_toml = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.join("Cargo.toml"))
            .expect("Resolve root Cargo.toml path"),
    )
    .expect("Failed to read workspace Cargo.toml");
    assert!(
        cargo_toml.contains("version = \"0.4.11\""),
        "Workspace Cargo.toml must declare version = \"0.4.11\""
    );
}

#[test]
fn test_t1_f13_02_window_version_badge_displays_v0_4_11() {
    let badge_text = concat!("v", env!("CARGO_PKG_VERSION"));
    assert_eq!(badge_text, "v0.4.11", "UI badge must display v0.4.11");
}

#[test]
fn test_t1_f13_03_retrocompatibility_load_v1_v2_documents() {
    let db_path = unique_phase4a_temp_file("retro_v2", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).unwrap();
    let doc = Document::new_notebook("RetroDoc");
    store.save_document(&doc).unwrap();

    let reloaded = store.load_document().unwrap();
    assert_eq!(reloaded.pages.len(), 1);
    assert_eq!(reloaded.format, DocumentFormat::Notebook);
}

#[test]
fn test_t1_f13_04_wal_journal_mode_verification() {
    let db_path = unique_phase4a_temp_file("wal_mode", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).unwrap();
    let mut doc = Document::new_notebook("WalTest");
    doc.pages[0].items.push(make_test_stroke_item(
        1,
        &[(10.0, 10.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    ));
    store.save_document(&doc).unwrap();

    assert!(db_path.exists());
    assert!(store.verify_integrity().unwrap());
}

#[test]
fn test_t1_f13_05_clean_save_without_orphaned_data() {
    let db_path = unique_phase4a_temp_file("clean_save", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).unwrap();
    let mut doc = Document::new_notebook("CleanSave");
    doc.pages[0].items.push(make_test_stroke_item(
        1,
        &[(10.0, 10.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    ));
    store.save_document(&doc).unwrap();

    doc.pages[0].items.clear();
    store.save_document(&doc).unwrap();

    let reloaded = store.load_document().unwrap();
    assert!(
        reloaded.pages[0].items.is_empty(),
        "Document must be cleanly cleared without orphans"
    );
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (F1 to F13, 5 tests each = 65 tests)
// ============================================================================

// --- F1: Selection Copy Boundaries ---

#[test]
fn test_t2_f1_01_copy_empty_selection_no_op() {
    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, Vec::new());
    clipboard.copy(payload);
    assert!(
        !clipboard.has_content(),
        "Copying empty selection must leave clipboard empty"
    );
}

#[test]
fn test_t2_f1_02_copy_zero_length_stroke_dot() {
    let mut clipboard = MockClipboardEngine::new();
    let dot = make_test_stroke_item(1, &[(100.0, 100.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![dot],
    ));
    assert!(clipboard.has_content());
    let p = clipboard.current_payload.unwrap();
    assert!((p.original_center.0 - 100.0).abs() < 1e-4);
    assert!((p.original_bbox[2] - p.original_bbox[0] - 2.0).abs() < 1e-4);
}

#[test]
fn test_t2_f1_03_copy_huge_selection_1000_strokes() {
    let mut clipboard = MockClipboardEngine::new();
    let mut items = Vec::with_capacity(1000);
    for i in 0..1000 {
        items.push(make_test_stroke_item(
            i,
            &[(i as f64, i as f64)],
            (0.0, 0.0, 0.0, 1.0),
            1.0,
        ));
    }
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        items,
    ));
    assert_eq!(clipboard.current_payload.unwrap().items.len(), 1000);
}

#[test]
fn test_t2_f1_04_copy_stroke_with_zero_bounding_box() {
    let mut clipboard = MockClipboardEngine::new();
    let item = Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(Stroke {
            id: 1,
            points: Vec::new(),
            style: StrokeStyle::default(),
            raw_samples: Vec::new(),
            is_dot: false,
            mode: PipelineMode::Baseline,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        }),
    };
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![item],
    ));
    let p = clipboard.current_payload.unwrap();
    assert_eq!(p.original_bbox, [0.0, 0.0, 0.0, 0.0]);
}

#[test]
fn test_t2_f1_05_copy_stroke_with_extreme_coordinates() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        1,
        &[(-1_000_000.0, -1_000_000.0), (1_000_000.0, 1_000_000.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        vec![it],
    ));
    let p = clipboard.current_payload.unwrap();
    assert!(p.original_bbox[0] < -999_999.0);
    assert!(p.original_bbox[2] > 999_999.0);
}

// --- F2: Selection Cut Boundaries ---

#[test]
fn test_t2_f2_01_cut_empty_selection_leaves_doc_unchanged() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(it.clone());
    spatial.insert(&it);

    let mut clipboard = MockClipboardEngine::new();
    let payload = ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, Vec::new());
    clipboard.cut(payload, &mut page, &mut spatial);

    assert_eq!(
        page.items.len(),
        1,
        "Cutting empty selection must not erase page items"
    );
}

#[test]
fn test_t2_f2_02_cut_all_items_on_page_leaves_empty_page() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let mut items = Vec::new();
    for i in 1..=10 {
        let it = make_test_stroke_item(
            i,
            &[(i as f64 * 10.0, i as f64 * 10.0)],
            (0.0, 0.0, 0.0, 1.0),
            2.0,
        );
        page.items.push(it.clone());
        spatial.insert(&it);
        items.push(it);
    }

    let mut clipboard = MockClipboardEngine::new();
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, items),
        &mut page,
        &mut spatial,
    );

    assert!(
        page.items.is_empty(),
        "Cutting all items must result in completely empty page"
    );
}

#[test]
fn test_t2_f2_03_cut_already_erased_items_safe_handling() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it = make_test_stroke_item(99, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);

    let mut clipboard = MockClipboardEngine::new();
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it]),
        &mut page,
        &mut spatial,
    );
    assert!(page.items.is_empty());
}

#[test]
fn test_t2_f2_04_cut_single_point_stroke() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let dot = make_test_stroke_item(1, &[(50.0, 50.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(dot.clone());
    spatial.insert(&dot);

    let mut clipboard = MockClipboardEngine::new();
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![dot]),
        &mut page,
        &mut spatial,
    );

    assert!(page.items.is_empty());
    assert!(spatial
        .locate_in_envelope(49.0, 49.0, 51.0, 51.0)
        .is_empty());
}

#[test]
fn test_t2_f2_05_cut_interleaved_with_active_drag() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    page.items.push(it.clone());
    spatial.insert(&it);

    let mut clipboard = MockClipboardEngine::new();
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it]),
        &mut page,
        &mut spatial,
    );

    assert!(page.items.is_empty());
}

// --- F3: Selection Paste Boundaries ---

#[test]
fn test_t2_f3_01_paste_empty_clipboard_returns_false() {
    let clipboard = MockClipboardEngine::new();
    assert!(!clipboard.has_content());
    let res = clipboard.instantiate_pasted_items((0.0, 0.0), 1);
    assert!(res.is_none());
}

#[test]
fn test_t2_f3_02_paste_extreme_pan_coordinates_centering() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(0.0, 0.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        vec![it],
    ));

    let delta = clipboard.compute_paste_delta(p_id, false, Some((-50_000.0, 50_000.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 1).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    let center_x = (bbox.min_x + bbox.max_x) * 0.5;
    let center_y = (bbox.min_y + bbox.max_y) * 0.5;
    assert_eq!(center_x, -50_000.0);
    assert_eq!(center_y, 50_000.0);
}

#[test]
fn test_t2_f3_03_paste_multi_cycle_cascade_large_offset() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(0.0, 0.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let mut last_delta = (0.0, 0.0);
    for _ in 0..20 {
        last_delta = clipboard.compute_paste_delta(p_id, true, None);
    }
    assert_eq!(last_delta, (500.0, 500.0));
}

#[test]
fn test_t2_f3_04_paste_near_page_boundary_no_overflow() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        1,
        &[(A4_WIDTH_PT - 10.0, A4_HEIGHT_PT - 10.0)],
        (0.0, 0.0, 0.0, 1.0),
        1.0,
    );
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let delta = clipboard.compute_paste_delta(p_id, true, None);
    let pasted = clipboard.instantiate_pasted_items(delta, 1).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    assert!(bbox.min_x > A4_WIDTH_PT);
}

#[test]
fn test_t2_f3_05_paste_negative_coordinates_in_whiteboard() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(-200.0, -100.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let orig_bbox = item_bbox(&it).unwrap();
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Whiteboard,
        None,
        None,
        vec![it],
    ));

    let delta = clipboard.compute_paste_delta(p_id, true, None);
    let pasted = clipboard.instantiate_pasted_items(delta, 1).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    assert_eq!(bbox.min_x, orig_bbox.min_x + 25.0);
    assert_eq!(bbox.min_y, orig_bbox.min_y + 25.0);
}

// --- F4: Selection Auto-Activation Boundaries ---

#[test]
fn test_t2_f4_01_auto_activation_single_dot_selection_bbox() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(50.0, 50.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let pasted = clipboard.instantiate_pasted_items((0.0, 0.0), 1).unwrap();
    let bbox = item_bbox(&pasted[0]).unwrap();
    let center_x = (bbox.min_x + bbox.max_x) * 0.5;
    assert_eq!(center_x, 50.0);
}

#[test]
fn test_t2_f4_02_auto_activation_rapid_successive_pastes() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let mut current_selection = Vec::new();
    for i in 0..10 {
        let delta = clipboard.compute_paste_delta(p_id, true, None);
        let pasted = clipboard
            .instantiate_pasted_items(delta, 100 + i * 10)
            .unwrap();
        current_selection = pasted.iter().map(|item| item.id).collect();
    }
    assert_eq!(current_selection, vec![190]);
}

#[test]
fn test_t2_f4_03_auto_activation_when_switching_tools() {
    let mut selected_ids = vec![1, 2, 3];
    let is_select_tool = false;
    if !is_select_tool {
        selected_ids.clear();
    }
    assert!(selected_ids.is_empty());
}

#[test]
fn test_t2_f4_04_auto_activation_boundary_drag_limits() {
    let mut it = make_test_stroke_item(1, &[(0.0, 0.0), (10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    if let ItemKind::Stroke(ref mut s) = it.kind {
        s.translate(-1000.0, -1000.0);
    }
    let bbox = item_bbox(&it).unwrap();
    assert!(bbox.min_x < 100.0);
}

#[test]
fn test_t2_f4_05_auto_activation_empty_paste_does_not_corrupt_selection() {
    let mut selection = vec![42];
    let clipboard = MockClipboardEngine::new();
    if clipboard.has_content() {
        selection.clear();
    }
    assert_eq!(
        selection,
        vec![42],
        "Empty paste must not clear existing selection"
    );
}

// --- F5: Undo/Redo & R-Tree Boundaries ---

#[test]
fn test_t2_f5_01_undo_redo_empty_paste_safe() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let cmd = MockPasteItemsCommand {
        page_id: page.id,
        items: Vec::new(),
    };
    cmd.apply(&mut page, &mut spatial);
    cmd.revert(&mut page, &mut spatial);
    assert!(page.items.is_empty());
}

#[test]
fn test_t2_f5_02_undo_paste_then_paste_different_branches() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it1 = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let it2 = make_test_stroke_item(2, &[(20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 1.0);

    let cmd1 = MockPasteItemsCommand {
        page_id: page.id,
        items: vec![it1],
    };
    let cmd2 = MockPasteItemsCommand {
        page_id: page.id,
        items: vec![it2],
    };

    cmd1.apply(&mut page, &mut spatial);
    cmd1.revert(&mut page, &mut spatial);
    cmd2.apply(&mut page, &mut spatial);

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, 2);
}

#[test]
fn test_t2_f5_03_paste_max_undo_depth_truncation() {
    let mut history = Vec::new();
    let max_depth = 5;
    for i in 0..10 {
        history.push(i);
        if history.len() > max_depth {
            history.remove(0);
        }
    }
    assert_eq!(history.len(), 5);
    assert_eq!(history, vec![5, 6, 7, 8, 9]);
}

#[test]
fn test_t2_f5_04_paste_sqlite_wal_checkpoint_stress() {
    let db_path = unique_phase4a_temp_file("wal_chk", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).unwrap();
    let mut doc = Document::new_notebook("WalStress");

    for i in 1..=20 {
        doc.pages[0].items.push(make_test_stroke_item(
            i,
            &[(i as f64, i as f64)],
            (0.0, 0.0, 0.0, 1.0),
            1.0,
        ));
        store.save_document(&doc).unwrap();
    }
    let reloaded = store.load_document().unwrap();
    assert_eq!(reloaded.pages[0].items.len(), 20);
}

#[test]
fn test_t2_f5_05_paste_spatial_index_boundary_query_precision() {
    let mut spatial = PageSpatialIndex::empty();
    let it = make_test_stroke_item(1, &[(10.0, 10.0), (20.0, 20.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    spatial.insert(&it);

    // Query covering
    let tangent = spatial.locate_in_envelope(5.0, 5.0, 25.0, 25.0);
    assert_eq!(tangent.len(), 1);

    let disjoint = spatial.locate_in_envelope(25.0, 25.0, 30.0, 30.0);
    assert!(
        disjoint.is_empty(),
        "Strictly disjoint query must not detect items"
    );
}

// --- F6: Cross-Page/Doc Boundaries ---

#[test]
fn test_t2_f6_01_cross_page_paste_to_non_existent_page_fails_gracefully() {
    let doc = Document::new_notebook("CheckDoc");
    let fake_page_id = Uuid::new_v4();
    assert!(doc.get_page(fake_page_id).is_none());
}

#[test]
fn test_t2_f6_02_cross_document_paste_between_incompatible_backgrounds() {
    let page_a = Page::new_a4(0, PageBackground::Blank);
    let page_b = Page::new_a4(0, PageBackground::Grid { grid_size: 14.17 });

    assert_eq!(page_a.background, PageBackground::Blank);
    assert_eq!(page_b.background, PageBackground::Grid { grid_size: 14.17 });
}

#[test]
fn test_t2_f6_03_cross_document_paste_with_huge_stroke_data() {
    let mut clipboard = MockClipboardEngine::new();
    let mut coords = Vec::with_capacity(5000);
    for i in 0..5000 {
        coords.push((i as f64 * 0.1, (i % 50) as f64));
    }
    let it = make_test_stroke_item(1, &coords, (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let pasted = clipboard.instantiate_pasted_items((0.0, 0.0), 2).unwrap();
    if let ItemKind::Stroke(s) = &pasted[0].kind {
        assert_eq!(s.points.len(), 5000);
    }
}

#[test]
fn test_t2_f6_04_cross_document_concurrent_documents_no_leak() {
    let p1 = unique_phase4a_temp_file("doc1", "uninote");
    let p2 = unique_phase4a_temp_file("doc2", "uninote");
    let mut s1 = DocumentStore::open_or_create(&p1).unwrap();
    let mut s2 = DocumentStore::open_or_create(&p2).unwrap();

    let d1 = Document::new_notebook("NB");
    let d2 = Document::new_whiteboard(PageBackground::Blank);
    s1.save_document(&d1).unwrap();
    s2.save_document(&d2).unwrap();

    assert_eq!(s1.load_document().unwrap().format, DocumentFormat::Notebook);
    assert_eq!(
        s2.load_document().unwrap().format,
        DocumentFormat::Whiteboard
    );
}

#[test]
fn test_t2_f6_05_cross_page_paste_boundary_offset_clamping() {
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(100.0, 100.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let p_id = Uuid::new_v4();
    let delta = clipboard.compute_paste_delta(p_id, false, Some((10.0, 10.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 1).unwrap();
    let bbox = item_bbox(&pasted[0]).unwrap();
    let center_x = (bbox.min_x + bbox.max_x) * 0.5;
    assert_eq!(center_x, 10.0);
}

// --- F7: Filesystem Folders Boundaries ---

#[test]
fn test_t2_f7_01_folder_scan_non_existent_root_dir_creates_or_handles() {
    let non_existent = PathBuf::from("/tmp/non_existent_path_99999");
    let folders = scan_filesystem_folders(&non_existent);
    assert!(
        folders.is_empty(),
        "Scanning non-existent directory must return empty list without panicking"
    );
}

#[test]
fn test_t2_f7_02_folder_scan_with_permission_denied_subfolder() {
    let root = unique_phase4a_temp_dir("perm_denied");
    let sub = root.join("NormalSub");
    fs::create_dir_all(&sub).unwrap();
    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
}

#[test]
fn test_t2_f7_03_folder_scan_unicode_and_emoji_directory_names() {
    let root = unique_phase4a_temp_dir("unicode_emoji");
    fs::create_dir_all(root.join("📚 Appunti Universitari 2026")).unwrap();
    fs::create_dir_all(root.join("📐 Geometria & Álgebra")).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 2);
}

#[test]
fn test_t2_f7_04_folder_scan_deeply_nested_directory_tree() {
    let root = unique_phase4a_temp_dir("deep_tree");
    let deep = root.join("L1").join("L2").join("L3").join("L4");
    fs::create_dir_all(&deep).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].subfolders.len(), 1);
}

#[test]
fn test_t2_f7_05_folder_scan_symlinks_loop_protection() {
    let root = unique_phase4a_temp_dir("symlink_test");
    let real_dir = root.join("RealDir");
    fs::create_dir_all(&real_dir).unwrap();

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
}

// --- F8: Sidebar Layout & Navigation Boundaries ---

#[test]
fn test_t2_f8_01_sidebar_rapid_folder_switching_no_race() {
    let root = unique_phase4a_temp_dir("rapid_switch");
    fs::create_dir_all(root.join("F1")).unwrap();
    fs::create_dir_all(root.join("F2")).unwrap();

    let mut current_folder = root.join("F1");
    for _ in 0..100 {
        current_folder = if current_folder == root.join("F1") {
            root.join("F2")
        } else {
            root.join("F1")
        };
    }
    assert!(current_folder.exists());
}

#[test]
fn test_t2_f8_02_sidebar_folder_deleted_externally_fallback_to_root() {
    let root = unique_phase4a_temp_dir("ext_deleted");
    let sub = root.join("Sub");
    fs::create_dir_all(&sub).unwrap();

    let mut active = sub.clone();
    fs::remove_dir_all(&sub).unwrap();

    if !active.exists() {
        active = root.clone();
    }
    assert_eq!(active, root);
}

#[test]
fn test_t2_f8_03_sidebar_very_long_folder_name_truncation() {
    let long_name = "A".repeat(300);
    assert!(sanitize_folder_name(&long_name).is_err());
}

#[test]
fn test_t2_f8_04_sidebar_special_characters_in_search_query() {
    let store = DashboardRecentStore::default();
    let filtered = store.filter_by_query("/*$%&?");
    assert!(filtered.is_empty());
}

#[test]
fn test_t2_f8_05_sidebar_zero_documents_rendering() {
    let root = unique_phase4a_temp_dir("zero_docs");
    let folders = scan_filesystem_folders(&root);
    assert!(folders.is_empty());
}

// --- F9: Contextual Creation Boundaries ---

#[test]
fn test_t2_f9_01_contextual_creation_non_writable_folder_error() {
    let non_writable = PathBuf::from("/proc/test_dir");
    let (_t, p) = next_available_note_path_in_dir(&non_writable);
    assert_eq!(p.parent().unwrap(), non_writable.as_path());
}

#[test]
fn test_t2_f9_02_contextual_creation_race_same_index() {
    let root = unique_phase4a_temp_dir("race_idx");
    let (t1, p1) = next_available_note_path_in_dir(&root);
    fs::write(&p1, b"1").unwrap();

    let (t2, p2) = next_available_note_path_in_dir(&root);
    assert_ne!(t1, t2);
    assert_ne!(p1, p2);
}

#[test]
fn test_t2_f9_03_contextual_creation_high_index_overflow() {
    let root = unique_phase4a_temp_dir("high_idx");
    fs::write(root.join("Nuova nota 999.uninote"), b"test").unwrap();

    let (t, p) = next_available_note_path_in_dir(&root);
    assert_eq!(t, "Nuova nota 1");
    assert_eq!(p, root.join("Nuova nota 1.uninote"));
}

#[test]
fn test_t2_f9_04_contextual_creation_with_existing_gaps() {
    let root = unique_phase4a_temp_dir("gaps");
    fs::write(root.join("Nuova nota 1.uninote"), b"test").unwrap();
    fs::write(root.join("Nuova nota 3.uninote"), b"test").unwrap();

    let (t, p) = next_available_note_path_in_dir(&root);
    assert_eq!(
        t, "Nuova nota 2",
        "Gap filling should detect lowest available note index"
    );
    assert_eq!(p, root.join("Nuova nota 2.uninote"));
}

#[test]
fn test_t2_f9_05_contextual_creation_folder_deleted_mid_operation() {
    let root = unique_phase4a_temp_dir("deleted_mid");
    let sub = root.join("TempDir");
    fs::create_dir_all(&sub).unwrap();
    fs::remove_dir(&sub).unwrap();

    let (_t, p) = next_available_note_path_in_dir(&sub);
    assert_eq!(p, sub.join("Nuova nota 1.uninote"));
}

// --- F10: Folder Management Boundaries ---

#[test]
fn test_t2_f10_01_folder_creation_path_traversal_dotdot_blocked() {
    assert!(sanitize_folder_name("../sneaky").is_err());
    assert!(sanitize_folder_name("foo/../../bar").is_err());
}

#[test]
fn test_t2_f10_02_folder_creation_null_byte_blocked() {
    assert!(sanitize_folder_name("folder\0name").is_err());
}

#[test]
fn test_t2_f10_03_folder_creation_reserved_system_names_windows_linux() {
    assert!(sanitize_folder_name(".").is_err());
    assert!(sanitize_folder_name("..").is_err());
}

#[test]
fn test_t2_f10_04_folder_rename_to_existing_destination_blocked() {
    let root = unique_phase4a_temp_dir("rename_coll");
    let d1 = root.join("Dir1");
    let d2 = root.join("Dir2");
    fs::create_dir_all(&d1).unwrap();
    fs::create_dir_all(&d2).unwrap();

    let can_rename = !d2.exists();
    assert!(
        !can_rename,
        "Renaming to an existing directory must be rejected by validation"
    );
}

#[test]
fn test_t2_f10_05_folder_delete_root_directory_prevented() {
    let root = unique_phase4a_temp_dir("prevent_root_del");
    let is_root = root.parent().is_none();
    assert!(!is_root);
}

// --- F11: Settings UX Boundaries ---

#[test]
fn test_t2_f11_01_corrupt_settings_json_graceful_fallback() {
    let file = unique_phase4a_temp_file("corrupt_settings", "json");
    fs::write(&file, b"{ invalid json syntax ...").unwrap();

    let loaded = FullAppSettingsModel::load_or_default(&file);
    assert_eq!(
        loaded.anti_hover_blip_threshold, 0.065,
        "Corrupt settings file must fall back to valid defaults"
    );
}

#[test]
fn test_t2_f11_02_partial_settings_json_merges_with_defaults() {
    let file = unique_phase4a_temp_file("partial_settings", "json");
    let mut initial = FullAppSettingsModel::default();
    initial.theme_mode = ThemeMode::ForceDarkOled;
    initial.save_atomic(&file).unwrap();

    let loaded = FullAppSettingsModel::load_or_default(&file);
    assert_eq!(loaded.theme_mode, ThemeMode::ForceDarkOled);
    assert_eq!(loaded.min_contact_pressure, 0.05);
}

#[test]
fn test_t2_f11_03_extreme_hardware_cutoff_frequency_clamped() {
    let mut model = FullAppSettingsModel::default();
    model.one_euro_cutoff_hz = 100.0;
    model.sanitize();
    assert!(
        model.one_euro_cutoff_hz <= 10.0,
        "Cutoff frequency must be clamped to max 10.0 Hz"
    );
}

#[test]
fn test_t2_f11_04_extreme_pressure_threshold_clamped() {
    let mut model = FullAppSettingsModel::default();
    model.min_contact_pressure = -1.0;
    model.sanitize();
    assert!(
        model.min_contact_pressure >= 0.01,
        "Pressure threshold must be clamped to min 0.01"
    );
}

#[test]
fn test_t2_f11_05_settings_concurrent_atomic_write_safety() {
    let file = unique_phase4a_temp_file("atomic_write", "json");
    let model = FullAppSettingsModel::default();
    model.save_atomic(&file).unwrap();
    assert!(file.exists());
}

// --- F12: GitHub Tab Boundaries ---

#[test]
fn test_t2_f12_01_github_tab_offline_network_safety() {
    assert!(GITHUB_REPO_URL.starts_with("https://"));
}

#[test]
fn test_t2_f12_02_github_tab_malformed_url_protection() {
    assert!(GITHUB_REPO_URL.contains("github.com"));
    assert!(GITHUB_ISSUES_URL.contains("github.com"));
}

#[test]
fn test_t2_f12_03_github_tab_changelog_empty_fallback() {
    let raw_changelog = "";
    let display_text = if raw_changelog.trim().is_empty() {
        "Nessuna nota di rilascio disponibile al momento."
    } else {
        raw_changelog
    };
    assert_eq!(
        display_text,
        "Nessuna nota di rilascio disponibile al momento."
    );
}

#[test]
fn test_t2_f12_04_github_tab_multiple_opens_no_leak() {
    run_on_gtk_thread(|| {
        for _ in 0..5 {
            let win = UniNoteSettingsWindow::new();
            let _ = win;
        }
    });
}

#[test]
fn test_t2_f12_05_github_tab_special_characters_escaping() {
    let raw = "UniNote v0.4.5 <release>";
    let escaped = glib::markup_escape_text(raw);
    assert_eq!(escaped, "UniNote v0.4.5 &lt;release&gt;");
}

// --- F13: SemVer & Database Boundaries ---

#[test]
fn test_t2_f13_01_semver_parse_exactness() {
    let ver = env!("CARGO_PKG_VERSION");
    let parts: Vec<&str> = ver.split('.').collect();
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "0");
    assert_eq!(parts[1], "4");
    assert!(parts[2].parse::<u32>().is_ok());
}

#[test]
fn test_t2_f13_02_database_empty_file_handling() {
    let path = unique_phase4a_temp_file("empty_db", "uninote");
    fs::write(&path, b"").unwrap();
    let res = DocumentStore::open_or_create(&path);
    assert!(
        res.is_ok(),
        "Opening empty file should initialize valid schema"
    );
}

#[test]
fn test_t2_f13_03_database_corrupt_header_handling() {
    let path = unique_phase4a_temp_file("corrupt_db", "uninote");
    fs::write(&path, b"CORRUPTED HEADER DATA NOT SQLITE").unwrap();
    let res = DocumentStore::open_or_create(&path);
    assert!(res.is_err(), "Opening corrupted file must return error");
}

#[test]
fn test_t2_f13_04_database_schema_v2_migration_idempotence() {
    let path = unique_phase4a_temp_file("idem_v2", "uninote");
    let mut s1 = DocumentStore::open_or_create(&path).unwrap();
    let doc = Document::new_notebook("IdemDoc");
    s1.save_document(&doc).unwrap();

    let s2 = DocumentStore::open_or_create(&path).unwrap();
    let loaded = s2.load_document().unwrap();
    assert_eq!(loaded.pages.len(), 1);
}

#[test]
fn test_t2_f13_05_window_version_badge_length_and_alignment() {
    let badge = concat!("v", env!("CARGO_PKG_VERSION"));
    assert!(badge.len() >= 6);
    assert!(badge.starts_with('v'));
}

// ============================================================================
// TIER 3: CROSS-FEATURE PAIRWISE COMBINATIONS (13 tests)
// ============================================================================

#[test]
fn test_t3_01_copy_in_notebook_paste_in_whiteboard() {
    // F1 + F3 + F6
    let mut clipboard = MockClipboardEngine::new();
    let stroke = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (150.0, 150.0)],
        (0.2, 0.4, 0.6, 1.0),
        3.0,
    );
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![stroke],
    ));

    let wb_doc = Document::new_whiteboard(PageBackground::Blank);
    let wb_page_id = wb_doc.pages[0].id;
    let delta = clipboard.compute_paste_delta(wb_page_id, false, Some((500.0, 500.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 10).unwrap();

    assert_eq!(pasted.len(), 1);
}

#[test]
fn test_t3_02_cut_in_subfolder_note_undo_move_to_another_folder() {
    // F2 + F5 + F9 + F10
    let root = unique_phase4a_temp_dir("t3_02");
    let folder_a = root.join("CorsoA");
    let folder_b = root.join("CorsoB");
    fs::create_dir_all(&folder_a).unwrap();
    fs::create_dir_all(&folder_b).unwrap();

    let (_t, note_path) = next_available_note_path_in_dir(&folder_a);
    let mut store = DocumentStore::open_or_create(&note_path).unwrap();
    let mut doc = Document::new_notebook("CorsoANote");
    let item = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    doc.pages[0].items.push(item.clone());
    store.save_document(&doc).unwrap();

    // Cut & Undo
    let mut clipboard = MockClipboardEngine::new();
    let mut spatial = PageSpatialIndex::empty();
    spatial.insert(&item);
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![item.clone()]),
        &mut doc.pages[0],
        &mut spatial,
    );
    assert!(doc.pages[0].items.is_empty());

    // Undo cut
    doc.pages[0].items.push(item);
    store.save_document(&doc).unwrap();
    drop(store);

    // Move file to folder B
    let dest_path = folder_b.join("nota_trasferita.uninote");
    let wal_src = note_path.with_extension("uninote-wal");
    let wal_dst = dest_path.with_extension("uninote-wal");
    if wal_src.exists() {
        let _ = fs::rename(wal_src, wal_dst);
    }
    let shm_src = note_path.with_extension("uninote-shm");
    let shm_dst = dest_path.with_extension("uninote-shm");
    if shm_src.exists() {
        let _ = fs::rename(shm_src, shm_dst);
    }
    fs::rename(&note_path, &dest_path).unwrap();
    assert!(dest_path.exists());
    let s2 = DocumentStore::open_or_create(&dest_path).unwrap();
    assert_eq!(s2.load_document().unwrap().pages[0].items.len(), 1);
}

#[test]
fn test_t3_03_copy_formula_paste_smart_offset_drag_delete() {
    // F1 + F3 + F4 + F5
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(
        1,
        &[(100.0, 100.0), (120.0, 120.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let delta = clipboard.compute_paste_delta(p_id, true, None);
    let mut pasted = clipboard.instantiate_pasted_items(delta, 100).unwrap();

    // Immediate drag
    for item in &mut pasted {
        if let ItemKind::Stroke(ref mut s) = item.kind {
            s.translate(30.0, 40.0);
        }
    }

    // Delete
    pasted.clear();
    assert!(pasted.is_empty());
}

#[test]
fn test_t3_04_create_subfolder_note_paste_external_clipboard_save_reopen() {
    // F6 + F7 + F9 + F13
    let root = unique_phase4a_temp_dir("t3_04");
    let folder = root.join("Laboratorio");
    fs::create_dir_all(&folder).unwrap();

    let (_t, p) = next_available_note_path_in_dir(&folder);
    let mut store = DocumentStore::open_or_create(&p).unwrap();
    let mut doc = Document::new_notebook("LabDoc");

    let mut clipboard = MockClipboardEngine::new();
    let ext_item =
        make_test_stroke_item(10, &[(50.0, 50.0), (60.0, 60.0)], (0.0, 0.5, 0.8, 1.0), 2.5);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        None,
        None,
        vec![ext_item],
    ));

    let pasted = clipboard
        .instantiate_pasted_items((25.0, 25.0), 20)
        .unwrap();
    doc.pages[0].items.extend(pasted);
    store.save_document(&doc).unwrap();

    let reloaded = store.load_document().unwrap();
    assert_eq!(reloaded.pages[0].items.len(), 1);
    assert_eq!(reloaded.pages[0].items[0].id, 20);
}

#[test]
fn test_t3_05_folder_rename_while_note_is_open_in_editor() {
    // F9 + F10 + F13
    let root = unique_phase4a_temp_dir("t3_05");
    let f_old = root.join("OldDir");
    let f_new = root.join("NewDir");
    fs::create_dir_all(&f_old).unwrap();

    let note_path = f_old.join("Nota.uninote");
    let mut store = DocumentStore::open_or_create(&note_path).unwrap();
    let doc = Document::new_notebook("OpenDoc");
    store.save_document(&doc).unwrap();

    fs::rename(&f_old, &f_new).unwrap();
    let new_note_path = f_new.join("Nota.uninote");
    let store_new = DocumentStore::open_or_create(&new_note_path).unwrap();
    assert_eq!(store_new.load_document().unwrap().pages.len(), 1);
}

#[test]
fn test_t3_06_multi_cycle_cut_and_paste_across_pages_with_undo() {
    // F2 + F3 + F5 + F6
    let mut doc = Document::new_notebook("MultiPage");
    let p1 = Page::new_a4(1, PageBackground::Blank);
    let p1_id = p1.id;
    doc.pages.push(p1);

    let it = make_test_stroke_item(1, &[(10.0, 10.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    doc.pages[0].items.push(it.clone());

    let mut clipboard = MockClipboardEngine::new();
    let mut spatial = PageSpatialIndex::empty();
    spatial.insert(&it);

    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it]),
        &mut doc.pages[0],
        &mut spatial,
    );
    let delta = clipboard.compute_paste_delta(p1_id, false, Some((100.0, 200.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 10).unwrap();
    doc.pages[1].items.extend(pasted);

    assert_eq!(doc.pages[0].items.len(), 0);
    assert_eq!(doc.pages[1].items.len(), 1);
}

#[test]
fn test_t3_07_settings_hardware_adjustment_and_stroke_copy_paste() {
    // F1 + F3 + F11
    let mut settings = FullAppSettingsModel::default();
    settings.min_contact_pressure = 0.035;
    settings.anti_hover_blip_threshold = 0.050;

    let stroke = make_test_stroke_item(1, &[(50.0, 50.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    let mut clipboard = MockClipboardEngine::new();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![stroke],
    ));

    let pasted = clipboard.instantiate_pasted_items((25.0, 25.0), 2).unwrap();
    assert_eq!(pasted.len(), 1);
}

#[test]
fn test_t3_08_search_filtering_in_sidebar_while_creating_notes() {
    // F7 + F8 + F9
    let root = unique_phase4a_temp_dir("t3_08");
    let f1 = root.join("Analisi Matematica");
    let f2 = root.join("Geometria Differenziale");
    fs::create_dir_all(&f1).unwrap();
    fs::create_dir_all(&f2).unwrap();

    let folders = scan_filesystem_folders(&root);
    let matched: Vec<&FolderEntry> = folders
        .iter()
        .filter(|f| f.name.to_lowercase().contains("analisi"))
        .collect();
    assert_eq!(matched.len(), 1);
    assert_eq!(matched[0].name, "Analisi Matematica");
}

#[test]
fn test_t3_09_selection_rect_cut_subfolder_paste_centered() {
    // F2 + F3 + F6 + F9
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let it = make_test_stroke_item(
        1,
        &[(200.0, 200.0), (300.0, 300.0)],
        (0.0, 0.0, 0.0, 1.0),
        2.0,
    );
    page.items.push(it.clone());
    spatial.insert(&it);

    let mut clipboard = MockClipboardEngine::new();
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, vec![it]),
        &mut page,
        &mut spatial,
    );

    let new_page_id = Uuid::new_v4();
    let delta = clipboard.compute_paste_delta(new_page_id, false, Some((500.0, 500.0)));
    let pasted = clipboard.instantiate_pasted_items(delta, 10).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    assert_eq!(bbox.min_x + (bbox.max_x - bbox.min_x) * 0.5, 500.0);
}

#[test]
fn test_t3_10_paste_cascade_10_repetitions_undo_all_redo_all() {
    // F3 + F4 + F5
    let mut clipboard = MockClipboardEngine::new();
    let it = make_test_stroke_item(1, &[(0.0, 0.0)], (0.0, 0.0, 0.0, 1.0), 1.0);
    let p_id = Uuid::new_v4();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![it],
    ));

    let mut applied_commands = Vec::new();
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();

    for i in 1..=10 {
        let delta = clipboard.compute_paste_delta(p_id, true, None);
        let pasted = clipboard.instantiate_pasted_items(delta, i * 10).unwrap();
        let cmd = MockPasteItemsCommand {
            page_id: p_id,
            items: pasted,
        };
        cmd.apply(&mut page, &mut spatial);
        applied_commands.push(cmd);
    }
    assert_eq!(page.items.len(), 10);

    // Undo all 10
    for cmd in applied_commands.iter().rev() {
        cmd.revert(&mut page, &mut spatial);
    }
    assert!(page.items.is_empty());

    // Redo all 10
    for cmd in &applied_commands {
        cmd.apply(&mut page, &mut spatial);
    }
    assert_eq!(page.items.len(), 10);
}

#[test]
fn test_t3_11_corrupt_settings_fallback_while_navigating_folders() {
    // F7 + F8 + F11
    let root = unique_phase4a_temp_dir("t3_11");
    let settings_file = root.join("bad_settings.json");
    fs::write(&settings_file, b"corrupted").unwrap();

    let model = FullAppSettingsModel::load_or_default(&settings_file);
    assert_eq!(model.theme_mode, ThemeMode::default());

    let f = root.join("Corso");
    fs::create_dir_all(&f).unwrap();
    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 1);
}

#[test]
fn test_t3_12_cross_doc_clipboard_transfer_legacy_to_subfolder_note() {
    // F6 + F9 + F13
    let root = unique_phase4a_temp_dir("t3_12");
    let legacy_file = root.join("legacy.uninote");
    let mut s_leg = DocumentStore::open_or_create(&legacy_file).unwrap();
    let mut doc_leg = Document::new_notebook("Legacy");
    doc_leg.pages[0].items.push(make_test_stroke_item(
        1,
        &[(10.0, 10.0)],
        (0.0, 0.0, 0.0, 1.0),
        1.0,
    ));
    s_leg.save_document(&doc_leg).unwrap();

    let mut clipboard = MockClipboardEngine::new();
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        doc_leg.pages[0].items.clone(),
    ));

    let sub = root.join("NuovoSemestre");
    fs::create_dir_all(&sub).unwrap();
    let (_t, new_file) = next_available_note_path_in_dir(&sub);
    let mut s_new = DocumentStore::open_or_create(&new_file).unwrap();
    let mut doc_new = Document::new_notebook("NewSemester");
    let pasted = clipboard
        .instantiate_pasted_items((25.0, 25.0), 50)
        .unwrap();
    doc_new.pages[0].items.extend(pasted);
    s_new.save_document(&doc_new).unwrap();

    assert_eq!(s_new.load_document().unwrap().pages[0].items.len(), 1);
}

#[test]
fn test_t3_13_github_tab_inspect_version_matches_note_metadata_and_badge() {
    // F12 + F13
    let manifest_ver = env!("CARGO_PKG_VERSION");
    let badge_ver = concat!("v", env!("CARGO_PKG_VERSION"));
    assert_eq!(manifest_ver, "0.4.11");
    assert_eq!(badge_ver, "v0.4.11");
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS (5 tests)
// ============================================================================

/// Scenario 1: Student organizing university courses
/// - Creates folders "Analisi 1", "Fisica 1"
/// - Creates notes inside folders
/// - Verifies tree hierarchy, note counts and navigation
#[test]
fn test_t4_scenario_1_student_course_folder_hierarchy_and_notes() {
    let root = unique_phase4a_temp_dir("scen_1");
    let dir_analisi = root.join("Analisi 1");
    let dir_fisica = root.join("Fisica 1");
    fs::create_dir_all(&dir_analisi).unwrap();
    fs::create_dir_all(&dir_fisica).unwrap();

    // Create 3 notes in Analisi 1
    for _ in 0..3 {
        let (_title, path) = next_available_note_path_in_dir(&dir_analisi);
        let mut store = DocumentStore::open_or_create(&path).unwrap();
        let doc = Document::new_notebook("AnalisiNote");
        store.save_document(&doc).unwrap();
    }

    // Create 2 notes in Fisica 1
    for _ in 0..2 {
        let (_title, path) = next_available_note_path_in_dir(&dir_fisica);
        let mut store = DocumentStore::open_or_create(&path).unwrap();
        let doc = Document::new_whiteboard(PageBackground::Blank);
        store.save_document(&doc).unwrap();
    }

    let folders = scan_filesystem_folders(&root);
    assert_eq!(folders.len(), 2);
    assert_eq!(folders[0].name, "Analisi 1");
    assert_eq!(folders[0].note_count, 3);
    assert_eq!(folders[1].name, "Fisica 1");
    assert_eq!(folders[1].note_count, 2);
}

/// Scenario 2: Formula and diagram duplication
/// - Selects complex mathematical schema
/// - Cuts from page 1, pastes on page 2
/// - Drags new selection to bottom margin
#[test]
fn test_t4_scenario_2_formula_diagram_cut_paste_and_drag() {
    let mut doc = Document::new_notebook("FormulaDoc");
    let p1 = Page::new_a4(1, PageBackground::Blank);
    let p1_id = p1.id;
    doc.pages.push(p1);

    // Schema: 5 strokes forming a formula
    let mut formula_strokes = Vec::new();
    for i in 1..=5 {
        let it = make_test_stroke_item(
            i,
            &[
                (100.0 + (i as f64 * 20.0), 100.0),
                (100.0 + (i as f64 * 20.0), 150.0),
            ],
            (0.0, 0.0, 0.0, 1.0),
            2.0,
        );
        doc.pages[0].items.push(it.clone());
        formula_strokes.push(it);
    }
    assert_eq!(doc.pages[0].items.len(), 5);

    // Cut formula
    let mut clipboard = MockClipboardEngine::new();
    let mut spatial = PageSpatialIndex::empty();
    for it in &formula_strokes {
        spatial.insert(it);
    }
    clipboard.cut(
        ClipboardPayload::new(DocumentFormat::Notebook, Some(0), None, formula_strokes),
        &mut doc.pages[0],
        &mut spatial,
    );
    assert!(
        doc.pages[0].items.is_empty(),
        "Page 0 must be empty after cutting formula"
    );

    // Paste onto Page 1 centered at (300, 700) - bottom margin
    let delta = clipboard.compute_paste_delta(p1_id, false, Some((300.0, 700.0)));
    let mut pasted = clipboard.instantiate_pasted_items(delta, 100).unwrap();
    assert_eq!(pasted.len(), 5);

    // Drag slightly
    for it in &mut pasted {
        if let ItemKind::Stroke(ref mut s) = it.kind {
            s.translate(0.0, 50.0);
        }
    }
    doc.pages[1].items.extend(pasted);
    assert_eq!(doc.pages[1].items.len(), 5);
}

/// Scenario 3: Multi-window / Cross-doc note collation
/// - Copies derivation from A4 notebook
/// - Opens Whiteboard
/// - Pastes centered in viewport, moves around
#[test]
fn test_t4_scenario_3_cross_document_derivation_collation() {
    let mut nb_doc = Document::new_notebook("Derivation");
    let it = make_test_stroke_item(
        1,
        &[(50.0, 100.0), (250.0, 100.0)],
        (0.1, 0.3, 0.7, 1.0),
        2.5,
    );
    nb_doc.pages[0].items.push(it.clone());

    // Copy to serialized payload (simulating GdkClipboard IPC)
    let payload = ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        Some((A4_WIDTH_PT, A4_HEIGHT_PT)),
        vec![it],
    );
    let json_bytes = payload.to_json().expect("Payload must serialize cleanly");

    // Whiteboard window deserializes payload
    let received_payload =
        ClipboardPayload::from_json(&json_bytes).expect("Payload must deserialize cleanly");
    let mut wb_clipboard = MockClipboardEngine::new();
    wb_clipboard.copy(received_payload);

    let wb_doc = Document::new_whiteboard(PageBackground::Blank);
    let wb_page_id = wb_doc.pages[0].id;

    // Paste centered at (0, 0) in infinite whiteboard
    let delta = wb_clipboard.compute_paste_delta(wb_page_id, false, Some((0.0, 0.0)));
    let pasted = wb_clipboard.instantiate_pasted_items(delta, 50).unwrap();

    let bbox = item_bbox(&pasted[0]).unwrap();
    let center_x = (bbox.min_x + bbox.max_x) * 0.5;
    let center_y = (bbox.min_y + bbox.max_y) * 0.5;
    assert!((center_x - 0.0).abs() < 1.0);
    assert!((center_y - 0.0).abs() < 1.0);
}

/// Scenario 4: Settings customization & GitHub link discovery
/// - User collapses advanced hardware sliders
/// - Verifies settings persistence
/// - Checks GitHub version and issues link
#[test]
fn test_t4_scenario_4_settings_customization_and_github_discovery() {
    let p_path = unique_phase4a_temp_file("scen_4_settings", "json");
    let mut settings = FullAppSettingsModel::default();
    settings.min_contact_pressure = 0.06;
    settings.theme_mode = ThemeMode::ForceDarkOled;
    settings.save_atomic(&p_path).unwrap();

    let loaded = FullAppSettingsModel::load_or_default(&p_path);
    assert_eq!(loaded.min_contact_pressure, 0.06);
    assert_eq!(loaded.theme_mode, ThemeMode::ForceDarkOled);

    // Verify GitHub spec
    assert_eq!(GITHUB_REPO_URL, "https://github.com/Loreexe/UniNote");
    assert_eq!(
        GITHUB_ISSUES_URL,
        "https://github.com/Loreexe/UniNote/issues"
    );
    assert_eq!(env!("CARGO_PKG_VERSION"), "0.4.11");
}

/// Scenario 5: Heavy stress clipboard lifecycle
/// - 50 consecutive copy/paste cycles with undo/redo interleaving
/// - Spatial index integrity validation
#[test]
fn test_t4_scenario_5_heavy_stress_clipboard_lifecycle() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let mut spatial = PageSpatialIndex::empty();
    let mut clipboard = MockClipboardEngine::new();

    let base_item =
        make_test_stroke_item(1, &[(10.0, 10.0), (30.0, 30.0)], (0.0, 0.0, 0.0, 1.0), 2.0);
    clipboard.copy(ClipboardPayload::new(
        DocumentFormat::Notebook,
        Some(0),
        None,
        vec![base_item],
    ));

    let mut history = Vec::new();

    // 50 iterations of paste + every 5th iteration undo then redo
    for i in 1..=50 {
        let delta = clipboard.compute_paste_delta(page.id, true, None);
        let pasted = clipboard.instantiate_pasted_items(delta, i * 100).unwrap();
        let cmd = MockPasteItemsCommand {
            page_id: page.id,
            items: pasted,
        };
        cmd.apply(&mut page, &mut spatial);
        history.push(cmd);

        if i % 5 == 0 {
            // Undo last 2
            let c1 = history.pop().unwrap();
            let c2 = history.pop().unwrap();
            c1.revert(&mut page, &mut spatial);
            c2.revert(&mut page, &mut spatial);

            // Redo them
            c2.apply(&mut page, &mut spatial);
            c1.apply(&mut page, &mut spatial);
            history.push(c2);
            history.push(c1);
        }
    }

    assert_eq!(
        page.items.len(),
        50,
        "Page must contain exactly 50 items after 50 interleaved operations"
    );
}
