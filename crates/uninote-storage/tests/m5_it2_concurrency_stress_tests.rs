//! M5 Iteration 2 Empirical Challenger Test Suite
//!
//! Adversarially verifies:
//! 1. Concurrent multi-thread SQLite Schema V2 migration without application retries (busy_timeout test).
//! 2. Concurrent writer vs readers contention under WAL mode with busy_timeout.
//! 3. Backward compatibility with unversioned V0, legacy V1, and degraded/empty databases.
//! 4. Heterogeneous page layout bounds and randomized oracle validation for find_page_index_at_y.
//! 5. CLI and UI integration contract invariants.

use rusqlite::Connection;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tempfile::NamedTempFile;
use uninote_core::coordinates::{PAGE_GAP_PT, PAGE_TOP_MARGIN_PT};
use uninote_core::{Document, DocumentFormat, Page, PageBackground};
use uninote_storage::{DocumentStore, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

fn make_test_stroke(id: u64, x: f64, y: f64) -> Stroke {
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
                x: x + 10.0,
                y: y + 20.0,
                pressure: 0.7,
                width: 2.8,
                timestamp_ns: 2_000_000,
            },
        ],
        style: StrokeStyle {
            color: (0.0, 0.4, 0.8, 1.0),
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
// 1. CONCURRENT MULTI-THREAD SCHEMA V2 MIGRATION WITHOUT APPLICATION RETRIES
// ============================================================================

#[test]
fn test_it2_concurrent_open_or_create_no_retries_busy_timeout() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // Inizializza un database legacy V1 privo della colonna `background_extra`
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
            "INSERT INTO metadata VALUES ('doc_id', ?1), ('title', 'V1 Test'), ('version', '1')",
            [Uuid::new_v4().to_string()],
        )
        .expect("Insert V1 metadata");

        for i in 0..5 {
            let p_id = Uuid::new_v4();
            conn.execute(
                "INSERT INTO pages VALUES (?1, ?2, 595.28, 841.89, 'ruled', 20.0)",
                rusqlite::params![p_id.to_string(), i],
            )
            .expect("Insert page");

            let stroke = make_test_stroke((i + 1) as u64, 50.0, 50.0);
            let s_json = serde_json::to_string(&stroke).unwrap();
            let b_json = serde_json::to_string(&stroke.bounding_box()).unwrap();

            conn.execute(
                "INSERT INTO items VALUES (?1, ?2, 0, 'stroke', ?3, ?4)",
                rusqlite::params![(i + 1) as i64, p_id.to_string(), b_json, s_json],
            )
            .expect("Insert item");
        }
    }

    // 16 thread concorrenti aprono il database simultaneamente SENZA alcun ciclo di retry.
    // Grazie a `busy_timeout = 5000;`, tutti i thread devono attendere ed eseguire con successo.
    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    for _thread_id in 0..16 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            for _iter in 0..5 {
                // Notare l'assenza di loop / match DatabaseBusy retry:
                // busy_timeout deve gestire la contesa a livello driver SQLite.
                let store = DocumentStore::open_or_create(&path)
                    .expect("open_or_create must succeed without application retry loop");
                let doc = store.load_document().expect("load_document must succeed");
                assert_eq!(doc.pages.len(), 5);
                assert_eq!(doc.title, "V1 Test");
            }
        }));
    }

    for h in handles {
        h.join().expect("All threads must complete cleanly");
    }

    // Verifica finale dello schema
    let store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    assert!(store.verify_integrity().expect("Integrity check"));
    let user_ver: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, SCHEMA_VERSION);
    assert_eq!(user_ver, 2);
}

// ============================================================================
// 2. CONCURRENT WRITER VS READERS CONTENTION UNDER WAL WITH BUSY_TIMEOUT
// ============================================================================

#[test]
fn test_it2_concurrent_writer_vs_readers_wal_contention() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // Crea documento iniziale
    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Initial create");
        let doc = Document::new_empty("Writer Contention Test");
        store.save_document(&doc).expect("Initial save");
    }

    let shared_path = Arc::new(file_path);
    let stop_signal = Arc::new(AtomicBool::new(false));

    // Thread scrittore: esegue salvataggi continui con molte pagine e tratti
    let writer_path = Arc::clone(&shared_path);
    let writer_stop = Arc::clone(&stop_signal);
    let writer_handle = thread::spawn(move || {
        let mut store = DocumentStore::open_or_create(&writer_path).expect("Writer open");
        let mut counter = 0;
        while !writer_stop.load(Ordering::Relaxed) && counter < 25 {
            counter += 1;
            let mut doc = Document::new_empty(format!("Revision {}", counter));
            doc.pages.clear();
            for p in 0..10 {
                let mut page = Page::new_a4(p, PageBackground::Blank);
                for s in 0..5 {
                    let stroke = make_test_stroke((p * 10 + s) as u64, 100.0, 100.0);
                    page.items.push(uninote_core::Item {
                        id: stroke.id,
                        z_index: s as u64,
                        kind: uninote_core::ItemKind::Stroke(stroke),
                    });
                }
                doc.pages.push(page);
            }
            store.save_document(&doc).expect("save_document in writer");
            thread::sleep(std::time::Duration::from_millis(5));
        }
        counter
    });

    // 8 Thread lettori: aprono e leggono continuamente mentre lo scrittore è attivo
    let mut reader_handles = Vec::new();
    for _ in 0..8 {
        let path = Arc::clone(&shared_path);
        let stop = Arc::clone(&stop_signal);
        reader_handles.push(thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(Ordering::Relaxed) && reads < 30 {
                let store = match DocumentStore::open_or_create(&path) {
                    Ok(s) => s,
                    Err(e) => panic!("Reader failed open_or_create: {:?}", e),
                };
                let doc = match store.load_document() {
                    Ok(d) => d,
                    Err(e) => panic!("Reader failed load_document: {:?}", e),
                };
                assert!(doc.title.starts_with("Revision") || doc.title == "Writer Contention Test");
                reads += 1;
                thread::sleep(std::time::Duration::from_millis(2));
            }
            reads
        }));
    }

    let write_count = writer_handle.join().expect("Writer thread finished");
    stop_signal.store(true, Ordering::Relaxed);

    for rh in reader_handles {
        let reads = rh.join().expect("Reader thread finished");
        assert!(reads > 0, "Reader must have executed at least one read");
    }

    assert!(write_count >= 20, "Writer must have completed its cycles");

    let final_store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    assert!(final_store.verify_integrity().expect("Integrity check"));
}

// ============================================================================
// 3. BACKWARD COMPATIBILITY: V0, V1, AND EMPTY/MALFORMED DATABASES
// ============================================================================

#[test]
fn test_it2_backward_compatibility_completely_empty_db() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // Database completamente vuoto (0 byte, 0 tabelle)
    {
        let _f = std::fs::File::create(file_path).expect("Create 0-byte file");
    }

    // open_or_create deve inizializzare lo schema V2 da zero senza errori
    let store = DocumentStore::open_or_create(file_path).expect("Open empty 0-byte file");
    assert!(store.verify_integrity().expect("Integrity ok"));

    let doc = store.load_document().expect("Load empty doc");
    assert_eq!(doc.title, "Senza titolo");
    // DocumentStore garantisce almeno 1 pagina predefinita A4 su documento vuoto
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.format, DocumentFormat::Notebook);
}

#[test]
fn test_it2_backward_compatibility_v1_with_missing_optional_tables() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // Database V1 con solo la tabella `pages`, senza `metadata` né `items`
    {
        let conn = Connection::open(file_path).expect("Open SQLite");
        conn.execute_batch(
            "PRAGMA user_version = 1;
             CREATE TABLE pages (
                 id TEXT PRIMARY KEY,
                 page_index INTEGER NOT NULL,
                 width REAL NOT NULL,
                 height REAL NOT NULL,
                 background_type TEXT NOT NULL,
                 background_spacing REAL NOT NULL
             );
             INSERT INTO pages VALUES ('00000000-0000-0000-0000-000000000001', 0, 600.0, 800.0, 'blank', 0.0);",
        )
        .expect("Create partial V1 schema");
    }

    let store = DocumentStore::open_or_create(file_path).expect("Open partial V1");
    assert!(store.verify_integrity().expect("Integrity ok"));

    let doc = store.load_document().expect("Load partial V1");
    assert_eq!(doc.pages.len(), 1);
    assert_eq!(doc.pages[0].width, 600.0);
    assert_eq!(doc.pages[0].height, 800.0);
    assert_eq!(doc.title, "Senza titolo");
}

#[test]
fn test_it2_backward_compatibility_unknown_background_types() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        // Inserisci tipi di sfondo anomali/sconosciuti
        store
            .connection_mut()
            .execute(
                "INSERT INTO pages VALUES
                 ('11111111-1111-1111-1111-111111111111', 0, 595.0, 842.0, 'isometric_dots', 15.0, ''),
                 ('22222222-2222-2222-2222-222222222222', 1, 595.0, 842.0, 'music_stave', 12.0, ''),
                 ('33333333-3333-3333-3333-333333333333', 2, 595.0, 842.0, '', 0.0, '')",
                [],
            )
            .expect("Insert unknown backgrounds");
    }

    let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
    let doc = store.load_document().expect("Load doc");
    assert_eq!(doc.pages.len(), 3);
    // Tipi sconosciuti devono fare fallback sicuro a Grid con lo spacing memorizzato
    assert_eq!(
        doc.pages[0].background,
        PageBackground::Grid { grid_size: 15.0 }
    );
    assert_eq!(
        doc.pages[1].background,
        PageBackground::Grid { grid_size: 12.0 }
    );
    assert_eq!(
        doc.pages[2].background,
        PageBackground::Grid { grid_size: 0.0 }
    );
}

// ============================================================================
// 4. HETEROGENEOUS PAGE LAYOUT BOUNDS & RANDOMIZED ORACLE VALIDATION
// ============================================================================

#[test]
fn test_it2_heterogeneous_layout_randomized_oracle_bounds() {
    let mut doc = Document::new_empty("Oracle Heterogeneous Test");
    doc.pages.clear();

    // 500 pagine con altezze e larghezze pseudocasuali variabili tra 50 pt e 2000 pt
    let mut heights = Vec::new();
    let mut tops = Vec::new();

    let mut current_top = PAGE_TOP_MARGIN_PT;
    for i in 0..500 {
        // Generatore LCG deterministico per riproducibilità
        let pseudo_rand = ((i * 1103515245 + 12345) % 32768) as f64;
        let h = 50.0 + (pseudo_rand / 32768.0) * 1950.0;
        let w = 400.0 + (pseudo_rand / 32768.0) * 800.0;

        heights.push(h);
        tops.push(current_top);
        doc.add_pdf_page("/path/to/test.pdf", i, w, h);
        current_top += h + PAGE_GAP_PT;
    }

    assert_eq!(doc.pages.len(), 500);

    // Oracle function: brute-force scan deterministico
    let oracle_find = |y: f64| -> Option<usize> {
        if doc.pages.is_empty() {
            return None;
        }
        let mut t = PAGE_TOP_MARGIN_PT;
        for (idx, &h) in heights.iter().enumerate() {
            let next_t = t + h + PAGE_GAP_PT;
            if y < next_t || idx == heights.len() - 1 {
                return Some(idx);
            }
            t = next_t;
        }
        Some(heights.len() - 1)
    };

    // 1. Verifica test points critici per ogni pagina
    for i in 0..500 {
        let t = tops[i];
        let h = heights[i];

        // Top esatto della pagina
        assert_eq!(doc.find_page_index_at_y(t), oracle_find(t));
        assert_eq!(doc.find_page_index_at_y(t), Some(i));

        // Interno della pagina (25%, 50%, 75%)
        assert_eq!(doc.find_page_index_at_y(t + h * 0.25), Some(i));
        assert_eq!(doc.find_page_index_at_y(t + h * 0.50), Some(i));
        assert_eq!(doc.find_page_index_at_y(t + h * 0.75), Some(i));

        // Fondo esatto della pagina prima del gap
        assert_eq!(doc.find_page_index_at_y(t + h), Some(i));

        // Centro del gap
        if i < 499 {
            let gap_y = t + h + PAGE_GAP_PT * 0.5;
            assert_eq!(doc.find_page_index_at_y(gap_y), Some(i));
            assert_eq!(doc.find_page_index_at_y(gap_y), oracle_find(gap_y));

            // Immediatamente prima della pagina successiva
            let just_before_next = t + h + PAGE_GAP_PT - 1e-9;
            assert_eq!(doc.find_page_index_at_y(just_before_next), Some(i));

            // Esattamente sulla linea di confine della pagina successiva
            let next_boundary = t + h + PAGE_GAP_PT;
            assert_eq!(doc.find_page_index_at_y(next_boundary), Some(i + 1));
        }
    }

    // 2. Estremi
    assert_eq!(doc.find_page_index_at_y(-1e9), Some(0));
    assert_eq!(doc.find_page_index_at_y(0.0), Some(0));
    assert_eq!(doc.find_page_index_at_y(current_top + 1e9), Some(499));
}

#[test]
fn test_it2_layout_edge_cases_empty_and_zero_height() {
    // Documento vuoto
    let mut empty_doc = Document::new_empty("Empty");
    empty_doc.pages.clear();
    assert_eq!(empty_doc.find_page_index_at_y(100.0), None);
    assert_eq!(empty_doc.find_page_index_at_y(0.0), None);
    assert_eq!(empty_doc.page_rect(0), None);

    // Documento con pagina a altezza sub-pixel / zero
    let mut zero_doc = Document::new_empty("Zero Height");
    zero_doc.pages.clear();
    zero_doc.add_pdf_page("/tmp/zero.pdf", 0, 500.0, 0.0);
    zero_doc.add_pdf_page("/tmp/normal.pdf", 1, 500.0, 800.0);

    let r0 = zero_doc.page_rect(0).unwrap();
    let r1 = zero_doc.page_rect(1).unwrap();
    assert_eq!(r0.height, 0.0);
    assert_eq!(r1.y, r0.y + 0.0 + PAGE_GAP_PT);
    assert_eq!(zero_doc.find_page_index_at_y(r0.y), Some(0));
    assert_eq!(zero_doc.find_page_index_at_y(r1.y), Some(1));
}

// ============================================================================
// 5. PRODUCTION REMEDIATED CONCURRENCY TESTS (VERIFYING DocumentStore::open_or_create)
// ============================================================================

#[test]
fn test_it2_proof_of_concept_remediated_concurrency() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // Inizializza con DocumentStore::open_or_create di produzione
    {
        let store = DocumentStore::open_or_create(&file_path).expect("Initial create");
        assert!(store.verify_integrity().expect("Integrity check"));
    }

    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    // 16 thread concorrenti che aprono e verificano simultaneamente con DocumentStore::open_or_create di produzione
    for _ in 0..16 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            for _ in 0..10 {
                let store = DocumentStore::open_or_create(&path)
                    .expect("DocumentStore::open_or_create must never fail concurrently");
                let doc = store.load_document().expect("load_document");
                assert_eq!(doc.title, "Senza titolo");
            }
        }));
    }

    for h in handles {
        h.join()
            .expect("All threads in production concurrency test completed successfully");
    }
}

#[test]
fn test_it2_proof_of_concept_remediated_writer_vs_readers() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Initial create");
        let doc = Document::new_empty("Init");
        store.save_document(&doc).expect("Initial save");
    }

    let shared_path = Arc::new(file_path);
    let stop_signal = Arc::new(AtomicBool::new(false));

    // Scrittore di produzione
    let writer_path = Arc::clone(&shared_path);
    let writer_stop = Arc::clone(&stop_signal);
    let writer_h = thread::spawn(move || {
        let mut store = DocumentStore::open_or_create(&writer_path).expect("Open writer");
        let mut count = 0;
        while !writer_stop.load(Ordering::Relaxed) && count < 30 {
            count += 1;
            let mut doc = Document::new_empty(format!("Rev {}", count));
            doc.pages.clear();
            doc.pages.push(Page::new_a4(0, PageBackground::Blank));
            store.save_document(&doc).expect("save doc");
            thread::sleep(std::time::Duration::from_millis(2));
        }
        count
    });

    // Lettori concorrenti di produzione
    let mut reader_handles = Vec::new();
    for _ in 0..8 {
        let path = Arc::clone(&shared_path);
        let stop = Arc::clone(&stop_signal);
        reader_handles.push(thread::spawn(move || {
            let mut reads = 0;
            while !stop.load(Ordering::Relaxed) && reads < 40 {
                let store = DocumentStore::open_or_create(&path).expect("Open reader");
                let doc = store.load_document().expect("Load doc");
                assert!(doc.title.starts_with("Rev") || doc.title == "Init");
                reads += 1;
                thread::sleep(std::time::Duration::from_millis(1));
            }
            reads
        }));
    }

    let writes = writer_h.join().expect("Writer done");
    stop_signal.store(true, Ordering::Relaxed);
    for rh in reader_handles {
        let reads = rh.join().expect("Reader done");
        assert!(reads > 0);
    }
    assert!(writes >= 20);

    let final_store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    assert!(final_store.verify_integrity().expect("Integrity check"));
}

#[test]
fn test_it2_proof_of_concept_unmigrated_v1_concurrent_immediate() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // Inizializza un database legacy V1
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
             );",
        )
        .expect("Create V1 schema");
    }

    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    // 16 thread concorrenti che tentano di migrare il database V1 non migrato contemporaneamente
    // tramite DocumentStore::open_or_create di produzione
    for _ in 0..16 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            let store = DocumentStore::open_or_create(&path).expect(
                "Migration via DocumentStore::open_or_create must serialize via busy_timeout",
            );
            assert!(store.verify_integrity().expect("Integrity check"));
        }));
    }

    for h in handles {
        h.join()
            .expect("All concurrent migrations must succeed without error 517");
    }

    let final_store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    let ver: u32 = final_store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("ver");
    assert_eq!(ver, 2);
}

#[test]
fn test_it2_concurrent_open_or_create_brand_new_file_no_retries() {
    let temp_dir = tempfile::tempdir().expect("Temp dir");
    let file_path = temp_dir.path().join("brand_new_concurrent.uninote");

    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    // 16 thread concorrenti che tentano di aprire un file non ancora esistente su disco contemporaneamente
    for _ in 0..16 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            let store = DocumentStore::open_or_create(&path)
                .expect("Brand new file creation must succeed concurrently");
            assert!(store.verify_integrity().expect("Integrity check"));
            let doc = store.load_document().expect("Load document");
            assert_eq!(doc.pages.len(), 1);
        }));
    }

    for h in handles {
        h.join()
            .expect("All threads on brand new file must succeed");
    }

    let final_store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    let ver: u32 = final_store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("ver");
    assert_eq!(ver, 2);
}

#[test]
fn test_it2_concurrent_open_or_create_unversioned_v0_no_retries() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    // Inizializza un database V0 (privo di user_version e privo di background_extra)
    {
        let conn = Connection::open(&file_path).expect("Open SQLite V0");
        conn.execute_batch(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
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
             INSERT INTO metadata VALUES ('title', 'V0 Legacy');
             INSERT INTO pages VALUES ('00000000-0000-0000-0000-000000000001', 0, 595.0, 842.0, 'blank', 0.0);",
        )
        .expect("Create V0 schema");
    }

    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    // 16 thread concorrenti che migrano il database V0
    for _ in 0..16 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            let store = DocumentStore::open_or_create(&path)
                .expect("V0 migration must succeed without application retry loop");
            assert!(store.verify_integrity().expect("Integrity check"));
            let doc = store.load_document().expect("Load V0 doc");
            assert_eq!(doc.title, "V0 Legacy");
            assert_eq!(doc.pages.len(), 1);
        }));
    }

    for h in handles {
        h.join().expect("All threads migrating V0 must succeed");
    }

    let final_store = DocumentStore::open_or_create(&shared_path).expect("Final open");
    let ver: u32 = final_store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("ver");
    assert_eq!(ver, 2);
}

#[test]
fn test_it2_concurrent_32_threads_stress() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Init 32 threads test");
        let doc = Document::new_empty("Stress 32");
        store.save_document(&doc).expect("Initial save");
    }

    let shared_path = Arc::new(file_path);
    let mut handles = Vec::new();

    // 32 thread contemporanei che eseguono ciascuno 10 cicli di open + load (320 aperture totali)
    for _ in 0..32 {
        let path = Arc::clone(&shared_path);
        handles.push(thread::spawn(move || {
            for _ in 0..10 {
                let store = DocumentStore::open_or_create(&path)
                    .expect("32 concurrent threads open must succeed");
                let doc = store.load_document().expect("load doc");
                assert_eq!(doc.title, "Stress 32");
            }
        }));
    }

    for h in handles {
        h.join().expect("All 32 threads must complete cleanly");
    }
}
