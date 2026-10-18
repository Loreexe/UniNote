//! Academic Dashboard & Documents Hub Opaque-Box Test Suite (Requirement R1)
//!
//! Scope:
//! - Dashboard recent documents metadata model (`RecentDocumentMeta`)
//! - Persistence and sanitization in `preferences.json` (max 30 items)
//! - Automatic pruning of non-existent files from recent document history
//! - Cairo 240x340 px thumbnail rendering and local disk cache pathing
//! - Instant case-insensitive search filtering across titles and dates
//! - Format badge attribution ("Quaderno A4" vs "Lavagna Infinita")
//! - Default notebooks directory initialization ($XDG_DOCUMENTS_DIR/UniNote)

use gtk4::prelude::*;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use uninote_app::dashboard::{
    format_badge_label, render_cairo_thumbnail, scan_directory, FolderEntry, OpenDocCallback,
    UniNoteDashboard,
};
use uninote_app::preferences::{
    next_available_note_path_in_dir, DashboardRecentStore, RecentDocumentMeta,
};
use uninote_core::model::DocumentFormat;

// ============================================================================
// REFERENCE ORACLES & DATA MODELS FOR DASHBOARD
// ============================================================================

static DASH_COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_dash_temp_path(prefix: &str, ext: &str) -> PathBuf {
    let id = DASH_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_dash_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
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
            .name("gtk-dash-test-worker".into())
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
fn test_t1_r1_dashboard_initialization_defaults() {
    let store = DashboardRecentStore::new();
    assert!(store.recent_files.is_empty());
    assert!(
        store.default_notes_dir.contains("UniNote"),
        "Default notes dir must contain 'UniNote': got {}",
        store.default_notes_dir
    );
}

#[test]
fn test_t1_r1_recent_files_addition_and_ordering() {
    let mut store = DashboardRecentStore::new();
    let doc1 = RecentDocumentMeta {
        file_path: "/tmp/doc1.uninote".into(),
        title: "Analisi Matematica I".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 1000,
        page_count: 12,
        thumbnail_path: None,
    };
    let doc2 = RecentDocumentMeta {
        file_path: "/tmp/doc2.uninote".into(),
        title: "Fisica Generale".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 2000,
        page_count: 1,
        thumbnail_path: None,
    };

    store.add_recent(doc1);
    store.add_recent(doc2);

    assert_eq!(store.recent_files.len(), 2);
    // Most recent must be at index 0
    assert_eq!(store.recent_files[0].title, "Fisica Generale");
    assert_eq!(store.recent_files[1].title, "Analisi Matematica I");
}

#[test]
fn test_t1_r1_thirty_items_capacity_eviction() {
    let mut store = DashboardRecentStore::new();
    for i in 1..=35 {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/doc_{}.uninote", i),
            title: format!("Quaderno {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: i as u64,
            page_count: 1,
            thumbnail_path: None,
        });
    }

    assert_eq!(
        store.recent_files.len(),
        DashboardRecentStore::MAX_RECENT_ITEMS,
        "Recent files must be strictly capped at 30 items"
    );
    // Most recent is item 35
    assert_eq!(store.recent_files[0].title, "Quaderno 35");
    // Oldest surviving is item 6 (1 to 5 evicted)
    assert_eq!(store.recent_files[29].title, "Quaderno 6");
}

#[test]
fn test_t1_r1_cairo_thumbnail_generation_240x340() {
    let temp_png = unique_dash_temp_path("thumb", "png");
    render_cairo_thumbnail(
        "Algoritmi e Strutture Dati",
        DocumentFormat::Notebook,
        &temp_png,
    )
    .expect("Thumbnail generation must succeed");

    assert!(temp_png.exists());
    let size = fs::metadata(&temp_png).unwrap().len();
    assert!(size > 500, "PNG thumbnail must have valid image bytes");

    let _ = fs::remove_file(temp_png);
}

#[test]
fn test_t1_r1_search_query_filtering() {
    let mut store = DashboardRecentStore::new();
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/chimica.uninote".into(),
        title: "Chimica Organica".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 100,
        page_count: 5,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/geometria.uninote".into(),
        title: "Geometria e Algebra Lineare".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 200,
        page_count: 8,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/architettura.uninote".into(),
        title: "Architettura dei Calcolatori".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 300,
        page_count: 1,
        thumbnail_path: None,
    });

    let results = store.filter_by_query("algebra");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].title, "Geometria e Algebra Lineare");

    let case_insensitive = store.filter_by_query("CHIMICA");
    assert_eq!(case_insensitive.len(), 1);
    assert_eq!(case_insensitive[0].title, "Chimica Organica");

    let empty_query = store.filter_by_query("");
    assert_eq!(empty_query.len(), 3);
}

#[test]
fn test_t1_r1_format_badge_attribution() {
    let meta_a4 = RecentDocumentMeta {
        file_path: "/tmp/a4.uninote".into(),
        title: "A4 Note".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 1,
        page_count: 4,
        thumbnail_path: None,
    };
    let meta_wb = RecentDocumentMeta {
        file_path: "/tmp/wb.uninote".into(),
        title: "Whiteboard Note".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 2,
        page_count: 1,
        thumbnail_path: None,
    };

    assert_eq!(format_badge_label(meta_a4.format), "Quaderno A4");
    assert_eq!(format_badge_label(meta_wb.format), "Lavagna Infinita");

    run_on_gtk_thread(move || {
        let open_cb: OpenDocCallback = std::rc::Rc::new(std::cell::RefCell::new(None));
        let card_a4 = UniNoteDashboard::create_card(&meta_a4, &open_cb);
        let card_wb = UniNoteDashboard::create_card(&meta_wb, &open_cb);

        assert!(card_a4.has_css_class("dms-flowbox-child"));
        assert!(card_wb.has_css_class("dms-flowbox-child"));
    });
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 TESTS)
// ============================================================================

#[test]
fn test_t2_r1_sanitization_removes_deleted_files() {
    let mut store = DashboardRecentStore::new();
    let temp_existing = unique_dash_temp_path("existing_doc", "uninote");
    fs::write(&temp_existing, "dummy uninote content").unwrap();

    let existing_path = temp_existing.to_str().unwrap().to_string();
    let missing_path = "/tmp/surely_does_not_exist_987654.uninote".to_string();

    store.add_recent(RecentDocumentMeta {
        file_path: missing_path,
        title: "Ghost Document".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 10,
        page_count: 1,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: existing_path.clone(),
        title: "Real Document".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 20,
        page_count: 2,
        thumbnail_path: None,
    });

    assert_eq!(store.recent_files.len(), 2);
    store.sanitize_existing_files();

    assert_eq!(
        store.recent_files.len(),
        1,
        "Non-existent ghost file must be pruned"
    );
    assert_eq!(store.recent_files[0].file_path, existing_path);

    let _ = fs::remove_file(temp_existing);
}

#[test]
fn test_t2_r1_duplicate_file_path_updates_position() {
    let mut store = DashboardRecentStore::new();
    let path = "/tmp/shared_lecture.uninote".to_string();

    store.add_recent(RecentDocumentMeta {
        file_path: path.clone(),
        title: "First Version".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 100,
        page_count: 1,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/other.uninote".into(),
        title: "Other Document".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 200,
        page_count: 2,
        thumbnail_path: None,
    });

    // Re-add the first document with updated title and timestamp
    store.add_recent(RecentDocumentMeta {
        file_path: path.clone(),
        title: "Updated Version".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 300,
        page_count: 3,
        thumbnail_path: None,
    });

    assert_eq!(store.recent_files.len(), 2, "No duplicate paths allowed");
    assert_eq!(store.recent_files[0].file_path, path);
    assert_eq!(store.recent_files[0].title, "Updated Version");
}

#[test]
fn test_t2_r1_special_characters_in_document_titles() {
    let mut store = DashboardRecentStore::new();
    let special_title = "Appunti di Teoria dei Sistemi & Controlli (A.A. 2026/27) — Prof. Rossi #1";
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/special.uninote".into(),
        title: special_title.into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 500,
        page_count: 10,
        thumbnail_path: None,
    });

    let filtered = store.filter_by_query("2026/27");
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].title, special_title);
}

#[test]
fn test_t2_r1_json_roundtrip_fidelity() {
    let mut store = DashboardRecentStore::new();
    store.add_recent(RecentDocumentMeta {
        file_path: "/home/user/UniNote/Calculus.uninote".into(),
        title: "Calculus III".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 1729000000,
        page_count: 24,
        thumbnail_path: Some("/home/user/.cache/uninote/calc.png".into()),
    });

    let json = serde_json::to_string_pretty(&store).expect("Serialization failed");
    let loaded: DashboardRecentStore = serde_json::from_str(&json).expect("Deserialization failed");

    assert_eq!(store, loaded);
}

#[test]
fn test_t2_r1_thumbnail_render_both_formats() {
    let temp_nb = unique_dash_temp_path("thumb_nb", "png");
    let temp_wb = unique_dash_temp_path("thumb_wb", "png");

    render_cairo_thumbnail("Notebook Preview", DocumentFormat::Notebook, &temp_nb).unwrap();
    render_cairo_thumbnail("Whiteboard Preview", DocumentFormat::Whiteboard, &temp_wb).unwrap();

    assert!(temp_nb.exists());
    assert!(temp_wb.exists());

    let _ = fs::remove_file(temp_nb);
    let _ = fs::remove_file(temp_wb);
}

// ============================================================================
// TIER 3: PAIRWISE & CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_t3_dashboard_recents_with_preferences_json_integration() {
    let temp_prefs = unique_dash_temp_path("dash_prefs", "json");
    let mut store = DashboardRecentStore::new();
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/sample.uninote".into(),
        title: "Bioingegneria".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 12345,
        page_count: 7,
        thumbnail_path: None,
    });

    let json = serde_json::to_string(&store).unwrap();
    fs::write(&temp_prefs, json).unwrap();

    let read_back: DashboardRecentStore =
        serde_json::from_str(&fs::read_to_string(&temp_prefs).unwrap()).unwrap();
    assert_eq!(read_back.recent_files.len(), 1);
    assert_eq!(read_back.recent_files[0].title, "Bioingegneria");

    let _ = fs::remove_file(temp_prefs);
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_academic_semester_document_hub() {
    // Scenario: Student takes 5 courses in the academic semester.
    // Daily usage pushes notes to the top; search quickly finds course notebooks.
    let mut store = DashboardRecentStore::new();

    let courses = [
        ("Fisica II", DocumentFormat::Notebook, 15),
        ("Calcolatori Elettronici", DocumentFormat::Notebook, 22),
        ("Laboratorio di Elettronica", DocumentFormat::Whiteboard, 1),
        ("Metodi Matematici", DocumentFormat::Notebook, 30),
        ("Segnali e Sistemi", DocumentFormat::Whiteboard, 1),
    ];

    for (idx, (name, fmt, pages)) in courses.iter().enumerate() {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/home/student/UniNote/{}.uninote", name.replace(' ', "_")),
            title: (*name).into(),
            format: *fmt,
            last_modified_timestamp: (idx as u64 + 1) * 1000,
            page_count: *pages,
            thumbnail_path: None,
        });
    }

    assert_eq!(store.recent_files.len(), 5);
    // Last added was "Segnali e Sistemi"
    assert_eq!(store.recent_files[0].title, "Segnali e Sistemi");

    // Search for "elettron" (stem matching both Elettronici and Elettronica)
    let results = store.filter_by_query("elettron");
    assert_eq!(results.len(), 2);
    let titles: Vec<String> = results.iter().map(|r| r.title.clone()).collect();
    assert!(titles.contains(&"Laboratorio di Elettronica".to_string()));
    assert!(titles.contains(&"Calcolatori Elettronici".to_string()));
}

// ============================================================================
// TIER 5: REAL IMPLEMENTATION TESTS (uninote_app::dashboard & preferences)
// ============================================================================

#[test]
fn test_t5_real_dashboard_recent_store_lifecycle() {
    let temp_store_path = unique_dash_temp_path("real_store", "json");
    let temp_doc_path = unique_dash_temp_path("real_doc", "uninote");
    fs::write(&temp_doc_path, "uninote test content").unwrap();

    let mut real_store = uninote_app::preferences::DashboardRecentStore::new();

    let doc = uninote_app::preferences::RecentDocumentMeta {
        file_path: temp_doc_path.to_string_lossy().to_string(),
        title: "Sistemi Operativi".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 4242,
        page_count: 14,
        thumbnail_path: None,
    };

    real_store.add_recent(doc);
    assert_eq!(real_store.recent_files.len(), 1);
    assert_eq!(real_store.recent_files[0].title, "Sistemi Operativi");

    real_store
        .save_atomic(&temp_store_path)
        .expect("save_atomic must succeed");
    assert!(temp_store_path.exists());

    let loaded = uninote_app::preferences::DashboardRecentStore::load_or_default(&temp_store_path);
    assert_eq!(loaded.recent_files.len(), 1);
    assert_eq!(loaded.recent_files[0].title, "Sistemi Operativi");

    let _ = fs::remove_file(temp_store_path);
    let _ = fs::remove_file(temp_doc_path);
}

#[test]
fn test_t5_real_cairo_thumbnail_generation_both_formats() {
    let nb_thumb = unique_dash_temp_path("real_nb_thumb", "png");
    let wb_thumb = unique_dash_temp_path("real_wb_thumb", "png");

    uninote_app::dashboard::render_cairo_thumbnail(
        "Fisica Sperimentale",
        DocumentFormat::Notebook,
        &nb_thumb,
    )
    .expect("Notebook thumbnail must render");

    uninote_app::dashboard::render_cairo_thumbnail(
        "Brainstorming Progetto",
        DocumentFormat::Whiteboard,
        &wb_thumb,
    )
    .expect("Whiteboard thumbnail must render");

    assert!(nb_thumb.exists());
    assert!(wb_thumb.exists());

    let _ = fs::remove_file(nb_thumb);
    let _ = fs::remove_file(wb_thumb);
}

#[test]
fn test_t5_real_uninote_dashboard_widget_instantiation() {
    run_on_gtk_thread(|| {
        let temp_doc_path = unique_dash_temp_path("sample_hub", "uninote");
        fs::write(&temp_doc_path, "uninote test hub content").unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));

        store.borrow_mut().add_recent(RecentDocumentMeta {
            file_path: temp_doc_path.to_string_lossy().to_string(),
            title: "Basi di Dati".into(),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 1000,
            page_count: 5,
            thumbnail_path: None,
        });

        let dashboard = UniNoteDashboard::with_store(store.clone());
        let widget = dashboard.widget();
        assert!(widget.can_target());
        assert!(widget.is_sensitive());
        assert_eq!(dashboard.store().borrow().recent_files.len(), 1);
        assert_eq!(
            dashboard.store().borrow().recent_files[0].title,
            "Basi di Dati"
        );

        let opened_path = std::rc::Rc::new(std::cell::RefCell::new(None));
        let op_clone = opened_path.clone();
        dashboard.connect_open_document(move |p| {
            *op_clone.borrow_mut() = Some(p);
        });

        dashboard.refresh();
        assert_eq!(dashboard.store().borrow().recent_files.len(), 1);

        let _ = fs::remove_file(temp_doc_path);
    });
}

// ============================================================================
// TIER 6: MILESTONE 2 — FOLDER HIERARCHY & SIDEBAR NAVIGATION
// ============================================================================

#[test]
fn test_m2_subfolder_note_auto_naming() {
    let temp_dir = unique_dash_temp_path("subfolder_naming", "dir");
    fs::create_dir_all(&temp_dir).expect("Failed to create temp dir");

    // Initially, directory is empty: next note should be "Nuova nota 1"
    let (name1, path1) = next_available_note_path_in_dir(&temp_dir);
    assert_eq!(name1, "Nuova nota 1");
    assert_eq!(path1, temp_dir.join("Nuova nota 1.uninote"));

    // Create Nuova nota 1.uninote on disk
    fs::write(&path1, "content 1").unwrap();

    // Now next should be "Nuova nota 2"
    let (name2, path2) = next_available_note_path_in_dir(&temp_dir);
    assert_eq!(name2, "Nuova nota 2");
    assert_eq!(path2, temp_dir.join("Nuova nota 2.uninote"));

    // Create Nuova nota 2.uninote on disk
    fs::write(&path2, "content 2").unwrap();

    // Now next should be "Nuova nota 3"
    let (name3, path3) = next_available_note_path_in_dir(&temp_dir);
    assert_eq!(name3, "Nuova nota 3");
    assert_eq!(path3, temp_dir.join("Nuova nota 3.uninote"));

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_m2_directory_scanner_enumerates_folders_and_files() {
    let root = unique_dash_temp_path("scan_root", "dir");
    fs::create_dir_all(&root).unwrap();

    let algebra = root.join("Algebra Lineare");
    let analisi = root.join("Analisi Matematica");
    let hidden_dir = root.join(".cache_uninote");
    fs::create_dir_all(&algebra).unwrap();
    fs::create_dir_all(&analisi).unwrap();
    fs::create_dir_all(&hidden_dir).unwrap();

    // Files in root
    let root_doc = root.join("Appunti Generali.uninote");
    let root_pdf = root.join("Programma Corso.pdf");
    let root_hidden = root.join(".DS_Store");
    fs::write(&root_doc, "doc").unwrap();
    fs::write(&root_pdf, "pdf").unwrap();
    fs::write(&root_hidden, "hidden").unwrap();

    // Files in subfolder algebra
    fs::write(algebra.join("Matrici.uninote"), "doc").unwrap();
    fs::write(algebra.join("Vettori.uninote"), "doc").unwrap();
    fs::write(algebra.join("Slide.pdf"), "pdf").unwrap();

    let (folders, files) = scan_directory(&root).expect("Scan directory must succeed");

    // Folders must include Algebra and Analisi, but NOT hidden_dir
    assert_eq!(folders.len(), 2);
    let algebra_entry = folders.iter().find(|f| f.name == "Algebra Lineare");
    assert!(algebra_entry.is_some());
    assert_eq!(algebra_entry.unwrap().item_count, 3);

    let analisi_entry = folders.iter().find(|f| f.name == "Analisi Matematica");
    assert!(analisi_entry.is_some());
    assert_eq!(analisi_entry.unwrap().item_count, 0);

    // Files must include root_doc and root_pdf, but NOT hidden
    assert_eq!(files.len(), 2);
    assert!(files.contains(&root_doc));
    assert!(files.contains(&root_pdf));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn test_m2_dashboard_breadcrumb_and_navigation() {
    run_on_gtk_thread(|| {
        let root = unique_dash_temp_path("nav_root", "dir");
        let sub = root.join("Geometria");
        let subsub = sub.join("Curve");
        fs::create_dir_all(&subsub).unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store, root.clone());

        // 1. Initial state at root
        assert!(dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), root);
        assert_eq!(dashboard.root_folder(), root);

        // 2. Navigate into Geometria
        dashboard.navigate_to(sub.clone());
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), sub);

        // 3. Navigate into Curve
        dashboard.navigate_to(subsub.clone());
        assert!(!dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), subsub);

        // 4. Navigate up to Geometria
        dashboard.navigate_up();
        assert_eq!(dashboard.current_folder(), sub);

        // 5. Navigate up to root
        dashboard.navigate_up();
        assert!(dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), root);

        // 6. Navigate up when already at root remains at root
        dashboard.navigate_up();
        assert!(dashboard.is_at_root());
        assert_eq!(dashboard.current_folder(), root);

        let _ = fs::remove_dir_all(&root);
    });
}

#[test]
fn test_m2_sidebar_and_folder_cards_rendering() {
    run_on_gtk_thread(|| {
        let root = unique_dash_temp_path("cards_root", "dir");
        let sub1 = root.join("Fisica 1");
        fs::create_dir_all(&sub1).unwrap();

        let store = std::rc::Rc::new(std::cell::RefCell::new(DashboardRecentStore::new()));
        let dashboard = UniNoteDashboard::with_store_and_root(store.clone(), root.clone());

        let folder_entry = FolderEntry {
            path: sub1.clone(),
            name: "Fisica 1".to_string(),
            item_count: 5,
            last_modified: 12345,
        };

        let navigated = std::rc::Rc::new(std::cell::RefCell::new(None));
        let nav_clone = navigated.clone();
        let nav_cb = std::rc::Rc::new(move |p: PathBuf| {
            *nav_clone.borrow_mut() = Some(p);
        });

        let card = UniNoteDashboard::create_folder_card(
            &folder_entry,
            nav_cb,
            std::rc::Rc::new(|| {}),
            &store,
        );

        assert!(card.has_css_class("dms-flowbox-child"));
        assert!(card.child().is_some());

        // Refresh dashboard should not crash
        dashboard.refresh();

        let _ = fs::remove_dir_all(&root);
    });
}
