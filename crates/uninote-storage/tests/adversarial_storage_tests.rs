use std::time::Instant;
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, Item, ItemKind, PageBackground};
use uninote_storage::{DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn create_sample_stroke(id: u64, x: f64, y: f64, num_points: usize) -> Stroke {
    let mut points = Vec::with_capacity(num_points);
    for i in 0..num_points {
        let offset = i as f64 * 5.0;
        points.push(StrokePoint {
            x: x + offset,
            y: y + offset,
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 1_000_000 + (i as u64 * 10_000),
        });
    }

    Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples: Vec::new(),
        is_dot: num_points == 1,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// 1. LEGACY & CORRUPT METADATA ADVERSARIAL TESTS
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_legacy_db_missing_document_format() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let conn = rusqlite::Connection::open(temp_file.path()).expect("Open raw SQLite");

    // Crea schema base identico a pre-0.2.0 (senza document_format)
    conn.execute_batch(
        "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE pages (id TEXT PRIMARY KEY, page_index INTEGER NOT NULL, width REAL NOT NULL, height REAL NOT NULL, background_type TEXT NOT NULL, background_spacing REAL NOT NULL);
         CREATE TABLE items (id INTEGER PRIMARY KEY, page_id TEXT NOT NULL, z_index INTEGER NOT NULL, item_type TEXT NOT NULL, bounding_box TEXT NOT NULL, data_json TEXT NOT NULL);
         INSERT INTO metadata (key, value) VALUES ('doc_id', 'a0000000-0000-0000-0000-000000000001');
         INSERT INTO metadata (key, value) VALUES ('title', 'Appunti Pre-0.2.0');
         INSERT INTO metadata (key, value) VALUES ('version', '1');
         INSERT INTO pages VALUES ('b0000000-0000-0000-0000-000000000001', 0, 595.28, 841.89, 'ruled', 22.68);"
    ).expect("Insert legacy schema");
    drop(conn);

    let store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");
    assert!(store.verify_integrity().expect("Integrity"));

    let loaded = store.load_document().expect("Load legacy document");
    assert_eq!(
        loaded.format,
        DocumentFormat::Notebook,
        "Missing document_format MUST fallback gracefully to Notebook"
    );
    assert_eq!(loaded.title, "Appunti Pre-0.2.0");
    assert_eq!(loaded.pages.len(), 1);
    assert_eq!(loaded.pages[0].width, 595.28);
}

#[test]
fn test_adversarial_legacy_db_corrupt_document_format_values() {
    let corrupt_values = [
        "",                                    // Empty string
        "unknown",                             // Unknown format
        "canvas",                              // Legacy or third-party format
        "infinite_canvas",                     // Similar keyword
        "WHITEBOARD",                          // Uppercase variant
        "Whiteboard",                          // Capitalized variant
        "whiteboard ",                         // Trailing space
        " notebook",                           // Leading space
        "null",                                // Literal string null
        "12345",                               // Numeric string
        "true",                                // Boolean string
        "{\"format\":\"whiteboard\"}",         // JSON string
        "'; DROP TABLE metadata; --",          // SQL injection attempt
        "corrupted_format_value_!@#$%^&*()_+", // Special chars
        "\0whiteboard",                        // Embedded null byte prefix
    ];

    for (idx, bad_val) in corrupt_values.iter().enumerate() {
        let temp_file = NamedTempFile::new().expect("Temp file");
        let conn = rusqlite::Connection::open(temp_file.path()).expect("Open raw SQLite");

        conn.execute_batch(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE pages (id TEXT PRIMARY KEY, page_index INTEGER NOT NULL, width REAL NOT NULL, height REAL NOT NULL, background_type TEXT NOT NULL, background_spacing REAL NOT NULL);
             CREATE TABLE items (id INTEGER PRIMARY KEY, page_id TEXT NOT NULL, z_index INTEGER NOT NULL, item_type TEXT NOT NULL, bounding_box TEXT NOT NULL, data_json TEXT NOT NULL);
             INSERT INTO metadata (key, value) VALUES ('title', 'Corrupt Format Test');
             INSERT INTO pages VALUES ('c0000000-0000-0000-0000-000000000001', 0, 595.28, 841.89, 'blank', 0.0);"
        ).expect("Init tables");

        conn.execute(
            "INSERT INTO metadata (key, value) VALUES ('document_format', ?1)",
            [bad_val],
        )
        .expect("Insert corrupt format");
        drop(conn);

        let store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");
        let loaded = store
            .load_document()
            .unwrap_or_else(|e| panic!("Failed on corrupt value '{bad_val}' (test #{idx}): {e}"));

        assert_eq!(
            loaded.format,
            DocumentFormat::Notebook,
            "Corrupt value '{bad_val}' must fallback to Notebook (test #{idx})"
        );
    }
}

#[test]
fn test_adversarial_legacy_db_type_mismatch_in_metadata() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let conn = rusqlite::Connection::open(temp_file.path()).expect("Open raw SQLite");

    // In SQLite, una colonna TEXT può contenere interi, float o BLOB (tipo dinamico)
    conn.execute_batch(
        "CREATE TABLE metadata (key TEXT PRIMARY KEY, value NOT NULL);
         CREATE TABLE pages (id TEXT PRIMARY KEY, page_index INTEGER NOT NULL, width REAL NOT NULL, height REAL NOT NULL, background_type TEXT NOT NULL, background_spacing REAL NOT NULL);
         CREATE TABLE items (id INTEGER PRIMARY KEY, page_id TEXT NOT NULL, z_index INTEGER NOT NULL, item_type TEXT NOT NULL, bounding_box TEXT NOT NULL, data_json TEXT NOT NULL);
         INSERT INTO metadata (key, value) VALUES ('doc_id', 'd0000000-0000-0000-0000-000000000001');
         INSERT INTO metadata (key, value) VALUES ('document_format', 999999); -- Integer invece di TEXT!
         INSERT INTO pages VALUES ('e0000000-0000-0000-0000-000000000001', 0, 595.28, 841.89, 'blank', 0.0);"
    ).expect("Init tables with type mismatch");
    drop(conn);

    let store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");
    let loaded = store
        .load_document()
        .expect("Load document with integer document_format");

    assert_eq!(
        loaded.format,
        DocumentFormat::Notebook,
        "Type mismatch in document_format must fallback gracefully to Notebook"
    );
}

#[test]
fn test_adversarial_empty_metadata_table() {
    let store = DocumentStore::open_in_memory().expect("Open in memory");
    // Tabella metadata esiste ma ha 0 righe (es. pulizia errata o transazione interrotta)
    let loaded = store
        .load_document()
        .expect("Load document with empty metadata");

    assert_eq!(loaded.format, DocumentFormat::Notebook);
    assert_eq!(loaded.title, "Senza titolo");
    assert_eq!(loaded.version, 1);
    assert!(!loaded.pages.is_empty(), "Must synthesize default page");
}

#[test]
fn test_adversarial_extra_metadata_keys() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");

    let doc = Document::new_notebook("Document with Extra Metadata");
    store.save_document(&doc).expect("Initial save");

    // Inietta 30 chiavi arbitrarie / custom in metadata
    for i in 0..30 {
        store
            .connection()
            .execute(
                "INSERT INTO metadata (key, value) VALUES (?1, ?2)",
                rusqlite::params![format!("custom_key_{i}"), format!("custom_value_{i}")],
            )
            .expect("Insert custom metadata");
    }

    let loaded = store.load_document().expect("Load with extra metadata");
    assert_eq!(loaded.title, "Document with Extra Metadata");
    assert_eq!(loaded.format, DocumentFormat::Notebook);

    // Salva nuovamente per verificare che le extra metadata non causino conflitti o crash
    store.save_document(&loaded).expect("Resave document");
    assert!(store.verify_integrity().expect("Integrity"));
}

#[test]
fn test_adversarial_corrupt_items_data_json() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let conn = rusqlite::Connection::open(temp_file.path()).expect("Open raw SQLite");

    conn.execute_batch(
        "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
         CREATE TABLE pages (id TEXT PRIMARY KEY, page_index INTEGER NOT NULL, width REAL NOT NULL, height REAL NOT NULL, background_type TEXT NOT NULL, background_spacing REAL NOT NULL);
         CREATE TABLE items (id INTEGER PRIMARY KEY, page_id TEXT NOT NULL, z_index INTEGER NOT NULL, item_type TEXT NOT NULL, bounding_box TEXT NOT NULL, data_json TEXT NOT NULL);
         INSERT INTO metadata (key, value) VALUES ('doc_id', 'f0000000-0000-0000-0000-000000000001');
         INSERT INTO pages VALUES ('p0000000-0000-0000-0000-000000000001', 0, 595.28, 841.89, 'blank', 0.0);
         -- Inserisce 1 tratto valido e 2 tratti con JSON corrotto o tipo sconosciuto
         INSERT INTO items VALUES (1, 'p0000000-0000-0000-0000-000000000001', 0, 'stroke', '{}', 'INVALID_JSON_CORRUPTED_DATA');
         INSERT INTO items VALUES (2, 'p0000000-0000-0000-0000-000000000001', 1, 'unknown_type', '{}', '{}');"
    ).expect("Insert corrupt item");
    drop(conn);

    let store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");
    let loaded = store
        .load_document()
        .expect("Load document with corrupt items");

    // Gli elementi corrotti o non riconosciuti devono essere scartati senza far fallire il caricamento dell'intero documento
    assert_eq!(loaded.pages[0].items.len(), 0);
}

// ---------------------------------------------------------------------------
// 2. WHITEBOARD EXTREME COORDINATES & BOUNDARY TESTS
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_whiteboard_extreme_coordinates() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    assert_eq!(doc.format, DocumentFormat::Whiteboard);
    assert!(doc.pages[0].width.is_infinite());
    assert!(doc.pages[0].height.is_infinite());

    // Coordinate estreme (negative e positive molto grandi)
    let stroke_far_negative = create_sample_stroke(1, -2_000_000.0, -1_500_000.0, 5);
    let stroke_near_origin = create_sample_stroke(2, 0.0, 0.0, 3);
    let stroke_far_positive = create_sample_stroke(3, 3_500_000.0, 4_000_000.0, 10);
    let stroke_dot = create_sample_stroke(4, -500.0, 500.0, 1);

    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 0,
        kind: ItemKind::Stroke(stroke_far_negative),
    });
    doc.pages[0].items.push(Item {
        id: 2,
        z_index: 1,
        kind: ItemKind::Stroke(stroke_near_origin),
    });
    doc.pages[0].items.push(Item {
        id: 3,
        z_index: 2,
        kind: ItemKind::Stroke(stroke_far_positive),
    });
    doc.pages[0].items.push(Item {
        id: 4,
        z_index: 3,
        kind: ItemKind::Stroke(stroke_dot),
    });

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save whiteboard");
        assert!(store.verify_integrity().expect("Integrity"));
    }

    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load whiteboard");

        assert_eq!(loaded.format, DocumentFormat::Whiteboard);
        assert_eq!(loaded.pages.len(), 1);
        let p0 = &loaded.pages[0];
        assert!(p0.width.is_infinite(), "Width must remain infinite");
        assert!(p0.height.is_infinite(), "Height must remain infinite");
        assert_eq!(p0.items.len(), 4);

        // Verifica fedeltà coordinate estreme
        let s1 = p0.items[0].as_stroke().expect("stroke");
        assert_eq!(s1.points[0].x, -2_000_000.0);
        assert_eq!(s1.points[0].y, -1_500_000.0);

        let s3 = p0.items[2].as_stroke().expect("stroke");
        assert_eq!(s3.points[0].x, 3_500_000.0);
        assert_eq!(s3.points[0].y, 4_000_000.0);

        let s4 = p0.items[3].as_stroke().expect("stroke");
        assert!(s4.is_dot);
    }
}

#[test]
fn test_adversarial_special_and_extreme_titles() {
    let long_title = "A".repeat(10_000);
    let adversarial_titles = [
        "",                                                  // Titolo vuoto
        "UniNote 🎓 📐 🦀 — Analisi Matematica II (120 Hz)", // Emoji, caratteri greci e simboli
        "数学解析 / 高等数学 / التفاضل والتكامل",            // CJK & Arabo
        "Robert'); DROP TABLE metadata;--",                  // Little Bobby Tables (SQL injection)
        "\"'\\;\n\t\r<>&`",                                  // Caratteri di controllo ed escape
        long_title.as_str(),                                 // Titolo lunghissimo da 10k caratteri
    ];

    for (idx, title) in adversarial_titles.iter().enumerate() {
        let temp_file = NamedTempFile::new().expect("Temp file");
        let mut store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");

        let doc = Document::new_notebook(*title);
        store
            .save_document(&doc)
            .unwrap_or_else(|e| panic!("Failed saving title #{idx}: {e}"));

        assert!(
            store.verify_integrity().expect("Integrity check"),
            "Integrity failed for title #{idx}"
        );

        let loaded = store
            .load_document()
            .unwrap_or_else(|e| panic!("Failed loading title #{idx}: {e}"));

        assert_eq!(
            loaded.title, *title,
            "Title fidelity mismatch on title #{idx}"
        );
    }
}

#[test]
fn test_adversarial_item_id_boundary_values() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("Open store");

    let boundary_ids = [
        0,
        1,
        i64::MAX as u64 - 1,
        i64::MAX as u64,
        (i64::MAX as u64) + 1,
        u64::MAX - 1,
        u64::MAX,
    ];

    let mut doc = Document::new_notebook("Boundary IDs Document");
    for (idx, &id) in boundary_ids.iter().enumerate() {
        let stroke = create_sample_stroke(id, idx as f64 * 10.0, idx as f64 * 10.0, 2);
        doc.pages[0].items.push(Item {
            id,
            z_index: idx as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    store.save_document(&doc).expect("Save boundary IDs");
    assert!(store.verify_integrity().expect("Integrity check"));

    let loaded = store.load_document().expect("Load boundary IDs");
    assert_eq!(loaded.pages[0].items.len(), boundary_ids.len());

    for (idx, &expected_id) in boundary_ids.iter().enumerate() {
        let item = &loaded.pages[0].items[idx];
        assert_eq!(
            item.id, expected_id,
            "ItemId mismatch at index {idx}: expected {expected_id}, got {}",
            item.id
        );
        let s = item.as_stroke().expect("stroke");
        assert_eq!(
            s.id, expected_id,
            "Stroke ID mismatch at index {idx}: expected {expected_id}, got {}",
            s.id
        );
    }
}

// ---------------------------------------------------------------------------
// 3. STRESS TEST: 10,000 STROKES & MASS TRANSACTION INTEGRITY
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_stress_10_000_strokes() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Grid { grid_size: 14.17 });
    let total_strokes = 10_000;

    println!("Generating {total_strokes} strokes...");
    let gen_start = Instant::now();
    for i in 0..total_strokes {
        let x = (i % 100) as f64 * 30.0 - 1500.0;
        let y = (i / 100) as f64 * 30.0 - 1500.0;
        let stroke = create_sample_stroke(i as u64, x, y, 4);
        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }
    println!(
        "Generated {total_strokes} strokes in {:.2?}",
        gen_start.elapsed()
    );

    // Salvataggio su disco SQLite
    let save_start = Instant::now();
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store
            .save_document(&doc)
            .expect("Save 10,000 strokes document");
    }
    let save_duration = save_start.elapsed();
    println!("Saved {total_strokes} strokes to SQLite in {save_duration:.2?}");

    // Verifica dimensione file
    let metadata = std::fs::metadata(file_path).expect("File metadata");
    println!(
        "Database file size on disk: {:.2} MB",
        metadata.len() as f64 / (1024.0 * 1024.0)
    );

    // Ricaricamento da disco
    let load_start = Instant::now();
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        assert!(store
            .verify_integrity()
            .expect("Integrity check on 10k strokes"));

        let loaded = store.load_document().expect("Load 10,000 strokes");
        let load_duration = load_start.elapsed();
        println!("Loaded {total_strokes} strokes from SQLite in {load_duration:.2?}");

        assert_eq!(loaded.format, DocumentFormat::Whiteboard);
        assert_eq!(loaded.pages[0].items.len(), total_strokes);

        // Verifica a campione primo, mediano e ultimo elemento
        let sample_indices = [0, 5000, 9999];
        for &idx in &sample_indices {
            let item = &loaded.pages[0].items[idx];
            assert_eq!(item.id, idx as u64);
            let s = item.as_stroke().expect("stroke");
            assert_eq!(s.id, idx as u64);
            assert_eq!(s.points.len(), 4);
        }
    }
}

// ---------------------------------------------------------------------------
// 4. WAL MODE & RAPID CYCLING TRANSACTION INTEGRITY
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_wal_mode_and_rapid_transactions() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // 1. Verifica iniziale pragmas WAL su file reale
    {
        let store = DocumentStore::open_or_create(file_path).expect("Open store");
        let journal_mode: String = store
            .connection()
            .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
            .expect("PRAGMA journal_mode");
        assert_eq!(
            journal_mode.to_lowercase(),
            "wal",
            "SQLite must be initialized in WAL mode"
        );

        let synchronous: i32 = store
            .connection()
            .query_row("PRAGMA synchronous;", [], |r| r.get(0))
            .expect("PRAGMA synchronous");
        assert_eq!(synchronous, 1, "PRAGMA synchronous must be NORMAL (1)");

        let fk: i32 = store
            .connection()
            .query_row("PRAGMA foreign_keys;", [], |r| r.get(0))
            .expect("PRAGMA foreign_keys");
        assert_eq!(fk, 1, "PRAGMA foreign_keys must be ON (1)");

        let app_id: u32 = store
            .connection()
            .query_row("PRAGMA application_id;", [], |r| r.get(0))
            .expect("PRAGMA application_id");
        assert_eq!(app_id, APPLICATION_ID);

        let user_ver: u32 = store
            .connection()
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .expect("PRAGMA user_version");
        assert_eq!(user_ver, SCHEMA_VERSION);
    }

    // 2. Esecuzione di 50 cicli sequenziali rapidi di modifica e salvataggio
    let mut doc = Document::new_notebook("Rapid Cycle Document");
    let mut store = DocumentStore::open_or_create(file_path).expect("Reopen store");

    for cycle in 0..50 {
        let stroke = create_sample_stroke(cycle, cycle as f64 * 2.0, cycle as f64 * 3.0, 5);
        doc.pages[0].items.push(Item {
            id: cycle,
            z_index: cycle,
            kind: ItemKind::Stroke(stroke),
        });

        store
            .save_document(&doc)
            .unwrap_or_else(|e| panic!("Failed at save cycle {cycle}: {e}"));

        if cycle % 10 == 0 {
            assert!(
                store.verify_integrity().expect("Integrity check"),
                "Integrity check failed at cycle {cycle}"
            );
        }
    }

    assert!(store.verify_integrity().expect("Final integrity check"));

    // 3. Riapertura e verifica finale
    let loaded = store.load_document().expect("Final load");
    assert_eq!(loaded.pages[0].items.len(), 50);
}

// ---------------------------------------------------------------------------
// 5. ASYNC STORAGE WORKER & TRUNCATED DB ADVERSARIAL TESTS
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_async_worker_rapid_revisions() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;
    use uninote_storage::{SaveStatus, StorageWorker};

    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path().to_path_buf();

    let saved_count = Arc::new(AtomicUsize::new(0));
    let saved_count_clone = saved_count.clone();

    let worker = StorageWorker::spawn(file_path.clone(), move |status| {
        if let SaveStatus::Saved(_) = status {
            saved_count_clone.fetch_add(1, Ordering::SeqCst);
        }
    })
    .expect("Spawn StorageWorker");

    let doc = Document::new_whiteboard(PageBackground::Blank);

    // Invia 20 revisioni in rapidissima sequenza
    for rev in 1..=20 {
        worker.save_revision(doc.clone(), rev);
    }

    // Attendi completamento
    let mut attempts = 0;
    while saved_count.load(Ordering::SeqCst) < 20 && attempts < 100 {
        thread::sleep(Duration::from_millis(50));
        attempts += 1;
    }

    assert_eq!(
        saved_count.load(Ordering::SeqCst),
        20,
        "All 20 rapid revisions must be processed and saved"
    );

    // Test export backup concomitante
    let backup_file = NamedTempFile::new().expect("Backup temp file");
    let rx = worker.export_backup(backup_file.path().to_path_buf());
    let res = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("Backup timeout");
    assert!(res.is_ok(), "Async backup export must succeed");

    // Verifica integrità backup
    let store = DocumentStore::open_or_create(backup_file.path()).expect("Open backup");
    assert!(store.verify_integrity().expect("Integrity check on backup"));
    let loaded = store.load_document().expect("Load from backup");
    assert_eq!(loaded.format, DocumentFormat::Whiteboard);
}

#[test]
fn test_adversarial_truncated_or_corrupt_file_handling() {
    use std::io::Write;

    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // Scrive header corrotto / casuale non SQLite
    {
        let mut f = std::fs::File::create(file_path).expect("Create file");
        f.write_all(b"CORRUPTED_NON_SQLITE_HEADER_DATA_1234567890\n")
            .expect("Write corrupt header");
    }

    // Apertura deve restituire Err senza panic
    let result = DocumentStore::open_or_create(file_path);
    assert!(
        result.is_err(),
        "Opening a corrupt non-SQLite file must return Err, not panic"
    );
}
