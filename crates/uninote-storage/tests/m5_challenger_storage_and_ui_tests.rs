//! M5 Challenger Adversarial Test Suite: SQLite V2 Migration, Heterogeneous Layouts & UI Wiring
//!
//! Empirical validation for:
//! 1. Concurrent multi-thread SQLite Schema V2 migration and WAL mode stability.
//! 2. Backward compatibility with V0 / unversioned databases.
//! 3. Repeated rapid save/load cycles with mixed page backgrounds.
//! 4. `PdfPageStorageMeta` adversarial paths (Unicode, RTL, long paths, special characters).
//! 5. `PdfPageStorageMeta` extreme and boundary dimensions.
//! 6. 1,000 heterogeneous pages layout mathematical continuity and exact Y-boundary lookup.
//! 7. Whiteboard layout infinite bounds behavior.
//! 8. Graceful fallback on corrupted `background_extra` JSON.
//! 9. Static AST/source verification of UI wiring and CLI arguments.

use rusqlite::Connection;
use std::sync::Arc;
use std::thread;
use tempfile::NamedTempFile;
use uninote_core::coordinates::{PAGE_GAP_PT, PAGE_TOP_MARGIN_PT};
use uninote_core::{Document, DocumentFormat, Page, PageBackground};
use uninote_storage::{DocumentStore, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

fn make_dummy_stroke(id: u64, x: f64, y: f64) -> Stroke {
    Stroke {
        id,
        points: vec![
            StrokePoint {
                x,
                y,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1_000_000,
            },
            StrokePoint {
                x: x + 15.0,
                y: y + 25.0,
                pressure: 0.8,
                width: 3.2,
                timestamp_ns: 2_000_000,
            },
        ],
        style: StrokeStyle {
            color: (0.1, 0.5, 0.9, 1.0),
            base_width: 2.0,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ============================================================================
// 1. CONCURRENT MULTI-THREAD SCHEMA V2 MIGRATION & WAL STABILITY
// ============================================================================

#[test]
fn test_concurrent_multi_thread_schema_v2_migration() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // 1. Prepara un database V1 reale
    {
        let conn = Connection::open(&file_path).expect("Open SQLite V1");
        conn.execute_batch(
            "PRAGMA application_id = 0x554E494E;
             PRAGMA user_version = 1;
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE pages (
                 id TEXT PRIMARY KEY,
                 page_index INTEGER NOT NULL,
                 width REAL NOT NULL,
                 height REAL NOT NULL,
                 background_type TEXT NOT NULL,
                 background_spacing REAL NOT NULL
             );
             CREATE TABLE items (
                 id INTEGER PRIMARY KEY,
                 page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 z_index INTEGER NOT NULL,
                 item_type TEXT NOT NULL,
                 bounding_box TEXT NOT NULL,
                 data_json TEXT NOT NULL
             );
             CREATE INDEX idx_items_page ON items(page_id);",
        )
        .expect("Create V1 schema");

        conn.execute(
            "INSERT INTO metadata VALUES ('doc_id', ?1), ('title', 'Concurrent V1'), ('version', '1')",
            [Uuid::new_v4().to_string()],
        )
        .expect("Insert V1 metadata");

        for i in 0..10 {
            let p_id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO pages VALUES (?1, ?2, 595.28, 841.89, 'grid', 14.17)",
                rusqlite::params![p_id.to_string(), i],
            )
            .expect("Insert page");

            let stroke = make_dummy_stroke((i + 1) as u64, 100.0, 200.0);
            let s_json = serde_json::to_string(&stroke).unwrap();
            let b_json = serde_json::to_string(&stroke.bounding_box()).unwrap();

            conn.execute(
                "INSERT INTO items VALUES (?1, ?2, 0, 'stroke', ?3, ?4)",
                rusqlite::params![(i + 1) as i64, p_id.to_string(), b_json, s_json],
            )
            .expect("Insert item");
        }
    }

    // 2. Lancia 8 thread concorrenti che tentano tutti simultaneamente di aprire il db,
    // innescando initialize_schema() concorrente sotto WAL.
    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    for thread_idx in 0..8 {
        let path_clone = Arc::clone(&shared_path);
        let h = thread::spawn(move || {
            for iter in 0..5 {
                let mut attempts = 0;
                let store = loop {
                    match DocumentStore::open_or_create(&path_clone) {
                        Ok(s) => break s,
                        Err(rusqlite::Error::SqliteFailure(err, _))
                            if err.code == rusqlite::ErrorCode::DatabaseBusy =>
                        {
                            attempts += 1;
                            if attempts > 100 {
                                panic!(
                                    "Thread {} iter {} exhausted retries due to database lock",
                                    thread_idx, iter
                                );
                            }
                            thread::sleep(std::time::Duration::from_millis(10));
                        }
                        Err(e) => panic!(
                            "Thread {} iter {} unexpected error: {:?}",
                            thread_idx, iter, e
                        ),
                    }
                };

                assert!(store.verify_integrity().expect("Integrity check in thread"));
                let doc = store.load_document().expect("Load document in thread");
                assert_eq!(doc.pages.len(), 10);
                assert_eq!(doc.title, "Concurrent V1");
            }
        });
        handles.push(h);
    }

    for h in handles {
        h.join().expect("Thread joined successfully");
    }

    // 3. Verifica finale
    let store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    let user_ver: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, SCHEMA_VERSION);
    assert_eq!(user_ver, 2);
}

// ============================================================================
// 2. BACKWARD COMPATIBILITY WITH V0 / UNVERSIONED LEGACY DATABASES
// ============================================================================

#[test]
fn test_migration_from_unversioned_raw_v0_database() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // Database grezzo con user_version = 0 (nessuna versione dichiarata)
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite");
        conn.execute_batch(
            "PRAGMA user_version = 0;
             CREATE TABLE pages (
                 id TEXT PRIMARY KEY,
                 page_index INTEGER NOT NULL,
                 width REAL NOT NULL,
                 height REAL NOT NULL,
                 background_type TEXT NOT NULL,
                 background_spacing REAL NOT NULL
             );
             CREATE TABLE items (
                 id INTEGER PRIMARY KEY,
                 page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 z_index INTEGER NOT NULL,
                 item_type TEXT NOT NULL,
                 bounding_box TEXT NOT NULL,
                 data_json TEXT NOT NULL
             );",
        )
        .expect("Create raw V0 schema");

        let p_id = Uuid::new_v4();
        conn.execute(
            "INSERT INTO pages VALUES (?1, 0, 595.28, 841.89, 'blank', 0.0)",
            [p_id.to_string()],
        )
        .expect("Insert page");
    }

    // open_or_create deve aggiornare senza errori a V2
    let store = DocumentStore::open_or_create(file_path).expect("Migrate V0 db");
    assert!(store.verify_integrity().expect("Integrity ok"));

    let user_ver: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, 2);

    let doc = store.load_document().expect("Load migrated V0 doc");
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].background, PageBackground::Blank);
}

// ============================================================================
// 3. REPEATED RAPID SAVE/LOAD CYCLES WITH MIXED BACKGROUNDS
// ============================================================================

#[test]
fn test_idempotent_repeated_save_and_reopen_cycles() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_empty("Stress Save Cycles");
    doc.pages.clear();
    doc.pages.push(Page::new_a4(0, PageBackground::Blank));
    doc.pages.push(Page::new_a4(
        1,
        PageBackground::Ruled {
            line_spacing: 22.0,
            margin_left: 40.0,
        },
    ));
    doc.add_pdf_page("/tmp/test.pdf", 0, 960.0, 540.0);

    for cycle in 0..30 {
        // Salva
        {
            let mut store = DocumentStore::open_or_create(file_path).expect("Open store cycle");
            doc.title = format!("Cycle {}", cycle);
            store.save_document(&doc).expect("Save doc cycle");
        }

        // Riapri e verifica
        {
            let store = DocumentStore::open_or_create(file_path).expect("Reopen store cycle");
            let reloaded = store.load_document().expect("Load doc cycle");
            assert_eq!(reloaded.title, format!("Cycle {}", cycle));
            assert_eq!(reloaded.pages.len(), 3);
            match &reloaded.pages[2].background {
                PageBackground::PdfPage {
                    source_path,
                    page_index,
                    ..
                } => {
                    assert_eq!(source_path, "/tmp/test.pdf");
                    assert_eq!(*page_index, 0);
                }
                _ => panic!("Expected PdfPage at index 2"),
            }
        }
    }
}

// ============================================================================
// 4. PDF PAGE STORAGE META ADVERSARIAL PATHS
// ============================================================================

#[test]
fn test_pdf_page_storage_meta_adversarial_paths() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let adversarial_paths = [
        "/tmp/normal.pdf",
        "/home/utente/dispense/Appunti_Fisica_2026.pdf",
        "/var/run/path with spaces and 'quotes' and \"double\".pdf",
        "/tmp/; rm -rf /; touch /tmp/pwn.pdf",
        "/tmp/backslash\\path\\windows\\doc.pdf",
        "/home/studente/Università/Ingegneria_Del_Software/Slide_Lezione_01_📚_🎯.pdf",
        "/mnt/dati/עִבְרִית_arabic_العربية_japanese_日本語.pdf",
        "/tmp/\nnewlines\r\nand\ttabs\t.pdf",
        &format!("/tmp/{}.pdf", "a".repeat(4000)), // 4000 chars path
    ];

    let mut doc = Document::new_empty("Adversarial Paths Test");
    doc.pages.clear();

    for (i, &path) in adversarial_paths.iter().enumerate() {
        doc.add_pdf_page(path, i, 800.0, 600.0);
    }

    // Salva nel database
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save document");
    }

    // Ricarica e verifica corrispondenza byte-per-byte
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load document");
        assert_eq!(loaded.pages.len(), adversarial_paths.len());

        for (i, page) in loaded.pages.iter().enumerate() {
            match &page.background {
                PageBackground::PdfPage {
                    source_path,
                    page_index,
                    original_width,
                    original_height,
                } => {
                    assert_eq!(source_path, adversarial_paths[i]);
                    assert_eq!(*page_index, i);
                    assert_eq!(*original_width, 800.0);
                    assert_eq!(*original_height, 600.0);
                }
                _ => panic!("Expected PdfPage at index {}", i),
            }
        }
    }
}

// ============================================================================
// 5. PDF PAGE STORAGE META EXTREME AND BOUNDARY DIMENSIONS
// ============================================================================

#[test]
fn test_pdf_page_storage_meta_extreme_and_boundary_dimensions() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let test_cases = [
        (0.0, 0.0, 0usize),
        (0.00001, 0.00001, 1),
        (1_000_000.0, 2_000_000.0, 999_999),
        (1e8, 1e8, usize::MAX / 2),
    ];

    let mut doc = Document::new_empty("Extreme Dimensions");
    doc.pages.clear();

    for (i, &(w, h, idx)) in test_cases.iter().enumerate() {
        doc.add_pdf_page(format!("/tmp/doc_{}.pdf", i), idx, w, h);
    }

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save document");
    }

    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load document");
        assert_eq!(loaded.pages.len(), test_cases.len());

        for (i, page) in loaded.pages.iter().enumerate() {
            let (expected_w, expected_h, expected_idx) = test_cases[i];
            assert_eq!(page.width, expected_w);
            assert_eq!(page.height, expected_h);

            match &page.background {
                PageBackground::PdfPage {
                    page_index,
                    original_width,
                    original_height,
                    ..
                } => {
                    assert_eq!(*page_index, expected_idx);
                    assert_eq!(*original_width, expected_w);
                    assert_eq!(*original_height, expected_h);
                }
                _ => panic!("Expected PdfPage"),
            }
        }
    }
}

// ============================================================================
// 6. 1,000 HETEROGENEOUS PAGES LAYOUT PRECISION & EXACT BOUNDARIES
// ============================================================================

#[test]
fn test_heterogeneous_page_layout_1000_pages_exact_boundaries() {
    let mut doc = Document::new_empty("1000 Pages Heterogeneous");
    doc.pages.clear();

    let page_specs = [
        (595.28, 841.89),  // A4
        (960.0, 540.0),    // 16:9 Slide
        (612.0, 792.0),    // US Letter
        (720.0, 540.0),    // 4:3 Slide
        (1190.55, 841.89), // A3 Landscape
    ];

    const TOTAL_PAGES: usize = 1000;
    for i in 0..TOTAL_PAGES {
        let (w, h) = page_specs[i % page_specs.len()];
        doc.add_pdf_page("/tmp/lecture.pdf", i, w, h);
    }

    assert_eq!(doc.pages.len(), TOTAL_PAGES);

    // 1. Verifica continuità spaziale esatta per tutte le 1000 pagine
    let mut expected_top = PAGE_TOP_MARGIN_PT;
    for i in 0..TOTAL_PAGES {
        let rect = doc
            .page_rect(i)
            .unwrap_or_else(|| panic!("page_rect failed at {}", i));
        let (w, h) = page_specs[i % page_specs.len()];

        assert_eq!(rect.x, 0.0);
        assert!(
            (rect.y - expected_top).abs() < 1e-6,
            "Y mismatch at page {}: got {}, expected {}",
            i,
            rect.y,
            expected_top
        );
        assert_eq!(rect.width, w);
        assert_eq!(rect.height, h);

        expected_top += h + PAGE_GAP_PT;
    }

    // Out-of-bounds page_rect
    assert!(doc.page_rect(TOTAL_PAGES).is_none());
    assert!(doc.page_rect(TOTAL_PAGES + 50).is_none());

    // 2. Verifica find_page_index_at_y per punti chiave
    // Punto sopra la prima pagina
    assert_eq!(doc.find_page_index_at_y(-500.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(0.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(PAGE_TOP_MARGIN_PT * 0.5), Some(0));

    // Punti interni e gap per un campione di 50 pagine
    for i in (0..TOTAL_PAGES).step_by(20) {
        let rect = doc.page_rect(i).unwrap();

        // Esattamente in cima alla pagina
        assert_eq!(doc.find_page_index_at_y(rect.y), Some(i));

        // A metà pagina
        assert_eq!(
            doc.find_page_index_at_y(rect.y + rect.height * 0.5),
            Some(i)
        );

        // Quasi al fondo della pagina
        assert_eq!(
            doc.find_page_index_at_y(rect.y + rect.height - 0.001),
            Some(i)
        );

        // Nel gap tra questa pagina e la successiva
        if i < TOTAL_PAGES - 1 {
            let gap_y = rect.y + rect.height + (PAGE_GAP_PT * 0.5);
            assert_eq!(doc.find_page_index_at_y(gap_y), Some(i));

            // Esattamente all'inizio della pagina successiva
            let next_top = rect.y + rect.height + PAGE_GAP_PT;
            assert_eq!(doc.find_page_index_at_y(next_top), Some(i + 1));
        }
    }

    // Punto oltre l'ultima pagina
    let last_rect = doc.page_rect(TOTAL_PAGES - 1).unwrap();
    let beyond_y = last_rect.y + last_rect.height + 10_000.0;
    assert_eq!(doc.find_page_index_at_y(beyond_y), Some(TOTAL_PAGES - 1));
}

// ============================================================================
// 7. WHITEBOARD LAYOUT INFINITE BOUNDS BEHAVIOR
// ============================================================================

#[test]
fn test_whiteboard_layout_infinite_dimensions() {
    let doc = Document::new_whiteboard(PageBackground::Blank);
    assert_eq!(doc.format, DocumentFormat::Whiteboard);
    assert_eq!(doc.pages.len(), 1);

    let rect = doc.page_rect(0).expect("Whiteboard page_rect");
    assert_eq!(rect.x, 0.0);
    assert_eq!(rect.y, PAGE_TOP_MARGIN_PT);
    assert!(rect.width.is_infinite());
    assert!(rect.height.is_infinite());

    // find_page_index_at_y restituisce sempre Some(0)
    assert_eq!(doc.find_page_index_at_y(-1e12), Some(0));
    assert_eq!(doc.find_page_index_at_y(0.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(1e12), Some(0));
}

// ============================================================================
// 8. GRACEFUL FALLBACK ON CORRUPTED BACKGROUND_EXTRA JSON
// ============================================================================

#[test]
fn test_corrupt_background_extra_schema_fallback() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        let p_id = Uuid::new_v4();

        // Inserisci un record con background_extra totalmente corrotto
        store
            .connection_mut()
            .execute(
                "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
                 VALUES (?1, 0, 1024.0, 768.0, 'pdf_page', 5.0, '{ corrupted json: true }')",
                [p_id.to_string()],
            )
            .expect("Insert corrupted page");
    }

    let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
    let doc = store
        .load_document()
        .expect("Load document with corrupt extra");

    assert_eq!(doc.pages.len(), 1);
    match &doc.pages[0].background {
        PageBackground::PdfPage {
            source_path,
            page_index,
            original_width,
            original_height,
        } => {
            // Fallback: path vuoto, index da background_spacing (5), dimensioni dalla pagina (1024, 768)
            assert_eq!(source_path, "");
            assert_eq!(*page_index, 5);
            assert_eq!(*original_width, 1024.0);
            assert_eq!(*original_height, 768.0);
        }
        _ => panic!("Expected graceful fallback to PdfPage"),
    }
}

// ============================================================================
// 9. STATIC AST / SOURCE VERIFICATION OF UI WIRING & CLI
// ============================================================================

#[test]
fn test_ui_wiring_and_cli_source_invariants() {
    let read_src = |rel_from_root: &str, rel_from_storage: &str| -> String {
        std::fs::read_to_string(rel_from_root)
            .or_else(|_| std::fs::read_to_string(rel_from_storage))
            .unwrap_or_else(|e| panic!("Could not read source file {}: {:?}", rel_from_root, e))
    };

    let dashboard_src = read_src(
        "crates/uninote-app/src/dashboard.rs",
        "../uninote-app/src/dashboard.rs",
    );
    let window_src = read_src(
        "crates/uninote-app/src/window.rs",
        "../uninote-app/src/window.rs",
    );
    let main_src = read_src(
        "crates/uninote-app/src/main.rs",
        "../uninote-app/src/main.rs",
    );

    // 1. Dashboard ha il pulsante "Nuovo da PDF..."
    assert!(
        dashboard_src.contains(".label(\"Nuovo da PDF...\")")
            || dashboard_src.contains("Nuovo da PDF..."),
        "dashboard.rs must define 'Nuovo da PDF...' button"
    );

    // 2. Dashboard ha il file filter per *.pdf
    assert!(
        dashboard_src.contains("*.pdf"),
        "dashboard.rs must filter for *.pdf"
    );
    assert!(
        dashboard_src.contains("application/pdf"),
        "dashboard.rs must filter for application/pdf MIME type"
    );

    // 3. Window ha il pulsante PDF nel popover di esportazione
    assert!(
        window_src.contains("card_pdf"),
        "window.rs must define card_pdf in export popover"
    );
    assert!(
        window_src.contains("export_pdf"),
        "window.rs must call export_pdf on canvas"
    );

    // 4. Window riconosce l'estensione PDF in open_file e chiama create_from_pdf
    assert!(
        window_src.contains("eq_ignore_ascii_case(\"pdf\")"),
        "window.rs must check for 'pdf' extension in open_file"
    );
    assert!(
        window_src.contains("create_from_pdf"),
        "window.rs must implement create_from_pdf"
    );

    // 5. Main collega dashboard.connect_import_pdf
    assert!(
        main_src.contains("dashboard.connect_import_pdf"),
        "main.rs must wire dashboard.connect_import_pdf"
    );

    // 6. Main supporta apertura CLI di file PDF
    assert!(
        main_src.contains("app.connect_open"),
        "main.rs must handle app.connect_open CLI arguments"
    );
}
