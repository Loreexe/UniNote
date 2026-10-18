//! Empirical Challenger Stress Tests for Milestone 2 (Dashboard Folder Hierarchy & Sidebar Navigation)
//!
//! Scope & Mandate:
//! 1. Multi-level nested folder creation & navigation:
//!    - Create nested hierarchy: `root/Semestre 1/Analisi/Esercizi/`.
//!    - Navigate deep into `Esercizi`, verifying `is_at_root()` is false, `current_folder()` is exact,
//!      and breadcrumb segments match `["UniNote", "Semestre 1", "Analisi", "Esercizi"]`.
//!    - Test sequential `navigate_up()` calls until reaching `root`, and verify extra `navigate_up()` at root stays at root safely.
//! 2. Contextual document creation:
//!    - In subfolder `Esercizi`, call `next_available_note_path_in_dir`, verify note `Nuova nota 1.uninote` is created in `Esercizi`.
//!    - Create 10 notes sequentially in `Esercizi` and in `root`, verify independent numbering without collision.
//! 3. Real GTK widget integration:
//!    - Test real GTK4 widgets (breadcrumbs, buttons, flowbox, sidebar).
//!    - Breadcrumb direct clicking and multi-level jump.
//!    - Deep nesting (15 levels).
//!    - Real SQLite WAL document persistence in subfolders.
//!    - Path traversal defense & containment.

use gtk4::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use uninote_app::dashboard::{scan_directory, UniNoteDashboard};
use uninote_app::preferences::{next_available_note_path_in_dir, DashboardRecentStore};
use uninote_core::model::DocumentFormat;
use uninote_storage::DocumentStore;

// ============================================================================
// DEDICATED GTK TEST HARNESS
// ============================================================================

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1000);

fn unique_challenger_temp_dir(prefix: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_m2_challenge_{}_{}_{}",
        prefix,
        std::process::id(),
        id
    ));
    fs::create_dir_all(&path).expect("Failed to create challenger temp dir");
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
            .name("gtk-challenger-m2-worker".into())
            .spawn(move || {
                let _ = gtk4::init();
                while let Ok(task) = rx.recv() {
                    task();
                }
            })
            .expect("Failed to spawn GTK challenger test worker thread");
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

/// Recursively traverses a widget tree to find a widget matching a CSS class.
fn find_widget_by_css(root: &gtk4::Widget, class: &str) -> Option<gtk4::Widget> {
    if root.has_css_class(class) {
        return Some(root.clone());
    }
    let mut child = root.first_child();
    while let Some(c) = child {
        if let Some(found) = find_widget_by_css(&c, class) {
            return Some(found);
        }
        child = c.next_sibling();
    }
    None
}

/// Extracts button labels with non-empty text from the dashboard's breadcrumbs bar.
fn extract_breadcrumb_button_labels(dashboard: &UniNoteDashboard) -> Vec<String> {
    let root_w = dashboard.widget();
    let bcb = find_widget_by_css(root_w, "dms-breadcrumbs-bar")
        .expect("dms-breadcrumbs-bar widget must exist in dashboard");

    let mut labels = Vec::new();
    let mut child = bcb.first_child();
    while let Some(c) = child {
        if let Ok(btn) = c.clone().downcast::<gtk4::Button>() {
            if let Some(lbl) = btn.label() {
                let s = lbl.trim().to_string();
                if !s.is_empty() {
                    labels.push(s);
                }
            }
        }
        child = c.next_sibling();
    }
    labels
}

/// Finds the breadcrumb button with a specific label to trigger click actions.
fn find_breadcrumb_button_by_label(
    dashboard: &UniNoteDashboard,
    target_label: &str,
) -> Option<gtk4::Button> {
    let root_w = dashboard.widget();
    let bcb = find_widget_by_css(root_w, "dms-breadcrumbs-bar")?;

    let mut child = bcb.first_child();
    while let Some(c) = child {
        if let Ok(btn) = c.clone().downcast::<gtk4::Button>() {
            if let Some(lbl) = btn.label() {
                if lbl.trim() == target_label {
                    return Some(btn);
                }
            }
        }
        child = c.next_sibling();
    }
    None
}

// ============================================================================
// STRESS TEST 1: MULTI-LEVEL NESTED FOLDER CREATION & NAVIGATION
// ============================================================================

#[test]
fn test_stress_nested_folder_creation_and_navigation() {
    run_on_gtk_thread(|| {
        let root = unique_challenger_temp_dir("nested_nav");
        let semestre = root.join("Semestre 1");
        let analisi = semestre.join("Analisi");
        let esercizi = analisi.join("Esercizi");

        fs::create_dir_all(&esercizi).expect("Failed to create nested directory hierarchy");

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // 1. Initial State at Root
        assert!(dashboard.is_at_root(), "Initially must be at root");
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(dashboard.root_folder(), root);
        let root_breadcrumbs = extract_breadcrumb_button_labels(&dashboard);
        assert_eq!(
            root_breadcrumbs,
            vec!["UniNote"],
            "At root, breadcrumb should only contain 'UniNote'"
        );

        // 2. Navigate Step 1: Into "Semestre 1"
        dashboard.navigate_to(semestre.clone());
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), semestre);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote", "Semestre 1"]
        );

        // 3. Navigate Step 2: Into "Analisi"
        dashboard.navigate_to(analisi.clone());
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), analisi);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote", "Semestre 1", "Analisi"]
        );

        // 4. Navigate Step 3: Deep into "Esercizi"
        dashboard.navigate_to(esercizi.clone());
        assert!(
            !dashboard.is_at_root(),
            "is_at_root() must be false when inside Esercizi"
        );
        assert_eq!(
            dashboard.current_folder(),
            esercizi,
            "current_folder() must exactly match the Esercizi directory path"
        );

        // Verify breadcrumb segments match exactly ["UniNote", "Semestre 1", "Analisi", "Esercizi"]
        let deep_breadcrumbs = extract_breadcrumb_button_labels(&dashboard);
        assert_eq!(
            deep_breadcrumbs,
            vec!["UniNote", "Semestre 1", "Analisi", "Esercizi"],
            "Breadcrumb segments must match [\"UniNote\", \"Semestre 1\", \"Analisi\", \"Esercizi\"]"
        );

        // 5. Test sequential navigate_up() calls until reaching root
        // Step 5a: navigate_up() from Esercizi -> Analisi
        dashboard.navigate_up();
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), analisi);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote", "Semestre 1", "Analisi"]
        );

        // Step 5b: navigate_up() from Analisi -> Semestre 1
        dashboard.navigate_up();
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), semestre);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote", "Semestre 1"]
        );

        // Step 5c: navigate_up() from Semestre 1 -> Root
        dashboard.navigate_up();
        assert!(
            dashboard.is_at_root(),
            "After reaching root, is_at_root() must be true"
        );
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote"]
        );

        // 6. Test extra navigate_up() calls when already at root
        // Extra call 1
        dashboard.navigate_up();
        assert!(
            dashboard.is_at_root(),
            "Extra navigate_up must stay at root"
        );
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote"]
        );

        // Extra call 2
        dashboard.navigate_up();
        assert!(
            dashboard.is_at_root(),
            "Second extra navigate_up must safely stay at root"
        );
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote"]
        );

        let _ = fs::remove_dir_all(&root);
    });
}

// ============================================================================
// STRESS TEST 2: CONTEXTUAL DOCUMENT CREATION & INDEPENDENT NUMBERING
// ============================================================================

#[test]
fn test_stress_contextual_document_creation_and_independent_numbering() {
    let root = unique_challenger_temp_dir("ctx_creation");
    let semestre = root.join("Semestre 1");
    let analisi = semestre.join("Analisi");
    let esercizi = analisi.join("Esercizi");
    fs::create_dir_all(&esercizi).expect("Failed to create nested directory hierarchy");

    // Requirement: In subfolder Esercizi, call next_available_note_path_in_dir,
    // verify note "Nuova nota 1.uninote" is created in Esercizi.
    let (first_name, first_path) = next_available_note_path_in_dir(&esercizi);
    assert_eq!(
        first_name, "Nuova nota 1",
        "Initial note name must be 'Nuova nota 1'"
    );
    assert_eq!(
        first_path,
        esercizi.join("Nuova nota 1.uninote"),
        "Target note path must be inside Esercizi subfolder"
    );
    assert_eq!(first_path.parent().unwrap(), esercizi.as_path());

    // Create 10 notes sequentially in Esercizi
    let mut esercizi_paths = Vec::new();
    for i in 1..=10 {
        let (name, path) = next_available_note_path_in_dir(&esercizi);
        let expected_name = format!("Nuova nota {}", i);
        let expected_path = esercizi.join(format!("Nuova nota {}.uninote", i));

        assert_eq!(
            name, expected_name,
            "Sequential naming in Esercizi failed at index {}",
            i
        );
        assert_eq!(
            path, expected_path,
            "Sequential path in Esercizi failed at index {}",
            i
        );
        assert_eq!(path.parent().unwrap(), esercizi.as_path());

        // Physically create the note on disk
        fs::write(&path, format!("Esercizi content note {}", i))
            .expect("Failed to write test note to disk");
        assert!(path.exists());
        esercizi_paths.push(path);
    }
    assert_eq!(esercizi_paths.len(), 10);

    // Verify next in Esercizi would be index 11
    let (es11_name, es11_path) = next_available_note_path_in_dir(&esercizi);
    assert_eq!(es11_name, "Nuova nota 11");
    assert_eq!(es11_path, esercizi.join("Nuova nota 11.uninote"));

    // Now, create 10 notes sequentially in root:
    // Verify completely independent numbering without collision from Esercizi notes!
    let mut root_paths = Vec::new();
    for i in 1..=10 {
        let (name, path) = next_available_note_path_in_dir(&root);
        let expected_name = format!("Nuova nota {}", i);
        let expected_path = root.join(format!("Nuova nota {}.uninote", i));

        assert_eq!(
            name, expected_name,
            "Sequential naming in root failed at index {}",
            i
        );
        assert_eq!(
            path, expected_path,
            "Sequential path in root failed at index {}",
            i
        );
        assert_eq!(path.parent().unwrap(), root.as_path());

        // Physically create the note on disk
        fs::write(&path, format!("Root content note {}", i))
            .expect("Failed to write test note to disk");
        assert!(path.exists());
        root_paths.push(path);
    }
    assert_eq!(root_paths.len(), 10);

    // Verify next in root would be index 11
    let (root11_name, root11_path) = next_available_note_path_in_dir(&root);
    assert_eq!(root11_name, "Nuova nota 11");
    assert_eq!(root11_path, root.join("Nuova nota 11.uninote"));

    // Check no path overlap between Esercizi and root
    for ep in &esercizi_paths {
        assert!(
            !root_paths.contains(ep),
            "Esercizi note path collided with root note path: {:?}",
            ep
        );
    }

    // Now create a sibling subfolder: root/Semestre 1/Teoria/
    // Verify that Teoria starts fresh at "Nuova nota 1"
    let teoria = semestre.join("Teoria");
    fs::create_dir_all(&teoria).expect("Failed to create Teoria subfolder");
    let (teoria1_name, teoria1_path) = next_available_note_path_in_dir(&teoria);
    assert_eq!(
        teoria1_name, "Nuova nota 1",
        "New sibling folder must start at index 1"
    );
    assert_eq!(teoria1_path, teoria.join("Nuova nota 1.uninote"));

    let _ = fs::remove_dir_all(&root);
}

// ============================================================================
// STRESS TEST 3: BREADCRUMB DIRECT CLICK NAVIGATION (MULTI-LEVEL JUMP)
// ============================================================================

/// Verifies that clicking a breadcrumb updates the dashboard's current_folder()
/// and breadcrumb segments.
///
/// NOTE: Currently IGNORED due to Bug M2-BUG-01:
/// `UniNoteDashboard::refresh_all` creates a disconnected local `cur_f_rc = Rc::new(RefCell::new(...))`
/// which updates the GTK UI visually, but never writes back to `self.current_folder`.
/// Consequently, `dashboard.current_folder()` remains stale.
/// Run with `cargo test --test m2_challenger_folder_stress_tests -- --ignored` to reproduce.
#[test]
fn test_stress_breadcrumb_direct_clicking_and_jump() {
    run_on_gtk_thread(|| {
        let root = unique_challenger_temp_dir("breadcrumb_jump");
        let semestre = root.join("Semestre 1");
        let analisi = semestre.join("Analisi");
        let esercizi = analisi.join("Esercizi");
        fs::create_dir_all(&esercizi).unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // Navigate deep into Esercizi
        dashboard.navigate_to(esercizi.clone());
        assert_eq!(dashboard.current_folder(), esercizi);

        // Click breadcrumb "Semestre 1" directly -> should jump 2 levels up in one click!
        let btn_semestre = find_breadcrumb_button_by_label(&dashboard, "Semestre 1")
            .expect("Breadcrumb button 'Semestre 1' must exist");
        btn_semestre.emit_clicked();

        assert_eq!(
            dashboard.current_folder(),
            semestre,
            "Direct click on 'Semestre 1' breadcrumb must navigate to Semestre 1"
        );
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote", "Semestre 1"]
        );

        // Navigate back down into Esercizi
        dashboard.navigate_to(esercizi.clone());
        assert_eq!(dashboard.current_folder(), esercizi);

        // Click root breadcrumb "UniNote" directly -> should jump 3 levels up to root!
        let btn_root = find_breadcrumb_button_by_label(&dashboard, "UniNote")
            .expect("Breadcrumb button 'UniNote' must exist");
        btn_root.emit_clicked();

        assert!(
            dashboard.is_at_root(),
            "Direct click on 'UniNote' breadcrumb must return to root"
        );
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote"]
        );

        let _ = fs::remove_dir_all(&root);
    });
}

/// Empirically asserts that Bug M2-BUG-01 is FIXED:
/// Clicking a breadcrumb button properly updates `dashboard.current_folder()`.
#[test]
fn test_reproduce_bug_interactive_navigation_leaves_dashboard_current_folder_stale() {
    run_on_gtk_thread(|| {
        let root = unique_challenger_temp_dir("bug_repro");
        let semestre = root.join("Semestre 1");
        let analisi = semestre.join("Analisi");
        let esercizi = analisi.join("Esercizi");
        fs::create_dir_all(&esercizi).unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // Navigate into Esercizi via navigate_to API
        dashboard.navigate_to(esercizi.clone());
        assert_eq!(dashboard.current_folder(), esercizi);

        // Click breadcrumb "Semestre 1"
        let btn_semestre = find_breadcrumb_button_by_label(&dashboard, "Semestre 1")
            .expect("Breadcrumb button 'Semestre 1' must exist");
        btn_semestre.emit_clicked();

        // Verification of fix:
        // Clicking breadcrumb button updates the shared current_folder RefCell,
        // so dashboard.current_folder() immediately returns `semestre`.
        let actual_folder = dashboard.current_folder();
        assert_eq!(
            actual_folder, semestre,
            "dashboard.current_folder() must be updated by breadcrumb click to semestre"
        );

        let _ = fs::remove_dir_all(&root);
    });
}

// ============================================================================
// STRESS TEST 4: EXTREME DEEP NESTING (15 LEVELS DEEP)
// ============================================================================

#[test]
fn test_stress_deeply_nested_15_levels() {
    run_on_gtk_thread(|| {
        let root = unique_challenger_temp_dir("deep_15");
        let mut cur = root.clone();
        let mut expected_breadcrumbs = vec!["UniNote".to_string()];

        for i in 1..=15 {
            let seg = format!("Livello_{:02}", i);
            cur = cur.join(&seg);
            expected_breadcrumbs.push(seg);
        }
        fs::create_dir_all(&cur).expect("Failed to create 15-level nested directories");

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // Navigate directly to level 15
        dashboard.navigate_to(cur.clone());
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), cur);

        // Verify breadcrumbs match all 16 segments
        let labels = extract_breadcrumb_button_labels(&dashboard);
        assert_eq!(labels, expected_breadcrumbs);

        // Sequential navigate_up 15 times
        for _ in 0..15 {
            dashboard.navigate_up();
        }
        assert!(
            dashboard.is_at_root(),
            "After 15 navigate_up() calls, must be at root"
        );
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(
            extract_breadcrumb_button_labels(&dashboard),
            vec!["UniNote"]
        );

        // 5 extra navigate_up() calls at root
        for _ in 0..5 {
            dashboard.navigate_up();
            assert!(dashboard.is_at_root());
            assert_eq!(dashboard.current_folder(), root);
        }

        let _ = fs::remove_dir_all(&root);
    });
}

// ============================================================================
// STRESS TEST 5: REAL SQLITE WAL STORAGE PERSISTENCE INSIDE SUBFOLDER
// ============================================================================

#[test]
fn test_stress_real_sqlite_wal_persistence_in_subfolder() {
    let root = unique_challenger_temp_dir("sqlite_subfolder");
    let sub = root.join("Laboratorio").join("Esperimento_1");
    fs::create_dir_all(&sub).unwrap();

    let (name, note_path) = next_available_note_path_in_dir(&sub);
    assert_eq!(name, "Nuova nota 1");
    assert_eq!(note_path, sub.join("Nuova nota 1.uninote"));

    // Open real SQLite DocumentStore inside the nested directory
    {
        let mut store = DocumentStore::open_or_create(&note_path)
            .expect("Must successfully create SQLite database in subfolder");

        // Verify WAL mode is active
        let journal_mode: String = store
            .connection()
            .query_row("PRAGMA journal_mode;", [], |row| row.get(0))
            .expect("Must read journal mode");
        assert_eq!(journal_mode.to_lowercase(), "wal");

        let doc = uninote_core::Document::new_notebook("Esperimento 1 - Misure");
        store
            .save_document(&doc)
            .expect("Must successfully save document to SQLite in subfolder");
    }

    assert!(note_path.exists());

    // Reopen and verify data integrity
    {
        let store = DocumentStore::open_or_create(&note_path)
            .expect("Must successfully reopen SQLite database in subfolder");
        let loaded = store
            .load_document()
            .expect("Must successfully load document from subfolder");
        assert_eq!(loaded.title, "Esperimento 1 - Misure");
        assert_eq!(loaded.format, DocumentFormat::Notebook);
    }

    // Verify scan_directory in Laboratorio correctly counts items in Esperimento_1
    let (folders, _) =
        scan_directory(&root.join("Laboratorio")).expect("Must scan parent subfolder");
    let exp_entry = folders
        .iter()
        .find(|f| f.name == "Esperimento_1")
        .expect("Esperimento_1 must be enumerated");
    assert_eq!(
        exp_entry.item_count, 1,
        "Subfolder item_count must reflect the .uninote document"
    );

    let _ = fs::remove_dir_all(&root);
}

// ============================================================================
// STRESS TEST 6: PATH TRAVERSAL DEFENSE & CONTAINMENT IN DASHBOARD
// ============================================================================

#[test]
fn test_stress_path_traversal_defense() {
    run_on_gtk_thread(|| {
        let root = unique_challenger_temp_dir("traversal_defense");
        let safe_sub = root.join("Valida");
        fs::create_dir_all(&safe_sub).unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // 1. Attempt navigation outside root: /etc
        dashboard.navigate_to(PathBuf::from("/etc"));
        assert!(
            dashboard.is_at_root(),
            "Navigating to /etc must be blocked and fallback to root"
        );
        assert_eq!(dashboard.current_folder(), root);

        // 2. Attempt navigation with dot-dot traversal
        let malicious_traversal = root.join("../");
        dashboard.navigate_to(malicious_traversal);
        assert!(
            dashboard.is_at_root(),
            "Path traversal with '..' must be blocked and fallback to root"
        );
        assert_eq!(dashboard.current_folder(), root);

        // 3. Legitimate subfolder navigation works
        dashboard.navigate_to(safe_sub.clone());
        assert_eq!(dashboard.current_folder(), safe_sub);
        assert!(!dashboard.is_at_root());

        let _ = fs::remove_dir_all(&root);
    });
}
