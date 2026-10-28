use rusqlite::{params, Connection, OpenFlags};
use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Instant;
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, ImageItem, Item, PageBackground};
use uninote_storage::backup::export_portable_copy;
use uninote_storage::database::{DocumentStore, PdfPageStorageMeta};
use uninote_storage::schema::SCHEMA_VERSION;
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

// =========================================================================
// TEST FIXTURES & DETERMINISTIC GENERATORS
// =========================================================================

/// Genera un buffer deterministico pseudo-casuale usando LCG per evitare dipendenze esterne.
fn generate_deterministic_payload(seed: u64, size_bytes: usize) -> Vec<u8> {
    let mut state = seed;
    let mut buffer = Vec::with_capacity(size_bytes);
    for _ in 0..size_bytes {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        buffer.push((state >> 32) as u8);
    }
    buffer
}

/// Crea un tratto sintetico con N punti campionati.
fn make_test_stroke(
    id: u64,
    z_index: u64,
    point_count: usize,
    offset_x: f64,
    offset_y: f64,
) -> Item {
    let mut stroke = Stroke::new(id, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    for i in 0..point_count {
        let t = i as f64;
        stroke.points.push(StrokePoint {
            x: offset_x + t * 3.5,
            y: offset_y + (t * 0.2).sin() * 15.0,
            pressure: 0.4 + 0.3 * (t * 0.1).cos().abs(),
            width: 2.0,
            timestamp_ns: (i * 10_000_000) as u64,
        });
    }
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

/// Crea un ImageItem sintetico referenziante un determinato hash.
fn make_test_image(
    id: u64,
    z_index: u64,
    page_id: Uuid,
    asset_hash: &str,
    bbox: Rect,
    orig_w: u32,
    orig_h: u32,
) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        asset_hash.to_string(),
        bbox,
        z_index as i64,
        1.0,
        orig_w,
        orig_h,
    );
    Item::new_image(id, z_index, img)
}

// =========================================================================
// 1. HIGH-VOLUME ASSET DEDUPLICATION (1,000 CALLS)
// =========================================================================

#[test]
fn test_adversarial_high_volume_asset_deduplication_1000_calls() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path();

    let store = DocumentStore::open_or_create(path).expect("open_or_create");
    let payload_64k = generate_deterministic_payload(0x1337_CAFE, 64 * 1024);
    let asset_hash = "dedup_sha256_adversarial_64k_target";

    // 1. Prima scrittura iniziale: inserimento dell'asset
    let start_initial = Instant::now();
    store
        .save_asset(asset_hash, "image/png", &payload_64k)
        .expect("initial save_asset");
    let initial_save_elapsed = start_initial.elapsed();

    // Rileva lo stato post-primo inserimento
    let initial_created_at: i64 = store
        .connection()
        .query_row(
            "SELECT created_at FROM assets WHERE hash = ?1;",
            params![asset_hash],
            |r| r.get(0),
        )
        .expect("read initial created_at");

    let initial_rowid: i64 = store
        .connection()
        .query_row(
            "SELECT rowid FROM assets WHERE hash = ?1;",
            params![asset_hash],
            |r| r.get(0),
        )
        .expect("read initial rowid");

    let initial_page_count: i64 = store
        .connection()
        .query_row("PRAGMA page_count;", [], |r| r.get(0))
        .expect("read page_count");

    // 2. Esecuzione di 1.000 chiamate consecutive con medesimo hash e payload
    let start_1000 = Instant::now();
    for i in 0..1000 {
        store
            .save_asset(asset_hash, "image/png", &payload_64k)
            .unwrap_or_else(|e| panic!("save_asset iteration {} failed: {}", i, e));
    }
    let elapsed_1000 = start_1000.elapsed();

    // 3. Verifiche empiriche di zero write amplification
    let post_row_count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0))
        .expect("count rows in assets");
    assert_eq!(
        post_row_count, 1,
        "Deduplication MUST result in exactly 1 row in assets, found: {}",
        post_row_count
    );

    let post_created_at: i64 = store
        .connection()
        .query_row(
            "SELECT created_at FROM assets WHERE hash = ?1;",
            params![asset_hash],
            |r| r.get(0),
        )
        .expect("read post created_at");
    assert_eq!(
        initial_created_at, post_created_at,
        "created_at timestamp must be immutable on duplicate save_asset"
    );

    let post_rowid: i64 = store
        .connection()
        .query_row(
            "SELECT rowid FROM assets WHERE hash = ?1;",
            params![asset_hash],
            |r| r.get(0),
        )
        .expect("read post rowid");
    assert_eq!(
        initial_rowid, post_rowid,
        "rowid must remain identical on duplicate save_asset"
    );

    let post_page_count: i64 = store
        .connection()
        .query_row("PRAGMA page_count;", [], |r| r.get(0))
        .expect("read post page_count");
    // Se ci fosse write amplification di 1000 * 64KB = 64MB, il page_count aumenterebbe di ~16,000 pagine.
    // Con INSERT OR IGNORE, il page_count rimane invariato o cresce al massimo di una pagina contabile.
    assert!(
        post_page_count <= initial_page_count + 1,
        "Zero write amplification violated: initial pages={}, post pages={}",
        initial_page_count,
        post_page_count
    );

    // 4. Verifica di integrità del dato caricato
    let loaded_bytes = store
        .load_asset(asset_hash)
        .expect("load_asset")
        .expect("asset must exist");
    assert_eq!(
        loaded_bytes, payload_64k,
        "Loaded payload must match original 64KB byte-for-byte"
    );

    println!(
        "[EMPIRICAL] 1000 deduplicated save_asset calls completed in {:?}, avg {:.3} µs/call (initial call: {:?})",
        elapsed_1000,
        (elapsed_1000.as_micros() as f64) / 1000.0,
        initial_save_elapsed
    );
}

#[test]
fn test_adversarial_interleaved_multi_asset_deduplication_stress() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let store = DocumentStore::open_or_create(temp_file.path()).expect("open store");

    const NUM_DISTINCT_ASSETS: usize = 10;
    const CALLS_PER_ASSET: usize = 100; // 10 * 100 = 1000 total calls

    let mut payloads = Vec::with_capacity(NUM_DISTINCT_ASSETS);
    let mut hashes = Vec::with_capacity(NUM_DISTINCT_ASSETS);

    for i in 0..NUM_DISTINCT_ASSETS {
        let size = 8 * 1024 + i * 2048; // Dimensione variabile tra 8KB e ~26KB
        let data = generate_deterministic_payload(i as u64 + 100, size);
        let hash = format!("multi_asset_hash_interleaved_{:03}", i);
        payloads.push(data);
        hashes.push(hash);
    }

    // 1000 chiamate interleaved (round-robin)
    let start = Instant::now();
    for round in 0..CALLS_PER_ASSET {
        for (i, hash) in hashes.iter().enumerate().take(NUM_DISTINCT_ASSETS) {
            let mime = if i % 2 == 0 {
                "image/png"
            } else {
                "image/jpeg"
            };
            store
                .save_asset(hash, mime, &payloads[i])
                .unwrap_or_else(|e| panic!("Round {} asset {} failed: {}", round, i, e));
        }
    }
    let elapsed = start.elapsed();

    // Verifiche: esattamente 10 righe nella tabella assets
    let total_assets: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0))
        .expect("count assets");
    assert_eq!(
        total_assets, NUM_DISTINCT_ASSETS as i64,
        "Expected exactly {} assets, found {}",
        NUM_DISTINCT_ASSETS, total_assets
    );

    // Ogni asset caricato deve essere byte-for-byte identico
    for i in 0..NUM_DISTINCT_ASSETS {
        let loaded = store
            .load_asset(&hashes[i])
            .expect("load_asset")
            .unwrap_or_else(|| panic!("Asset {} not found", hashes[i]));
        assert_eq!(loaded, payloads[i]);
    }

    println!(
        "[EMPIRICAL] Interleaved 1000 calls across 10 distinct assets completed in {:?}, avg {:.3} µs/call",
        elapsed,
        (elapsed.as_micros() as f64) / 1000.0
    );
}

// =========================================================================
// 2. ZERO BLOB REWRITES EMPIRICAL PROOF (1MB ASSET, 100 CONSECUTIVE SAVES)
// =========================================================================

#[test]
fn test_adversarial_zero_blob_rewrites_and_latency_100_saves() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path().to_path_buf();

    // 1. Genera un asset da 1 MB (1.048.576 byte)
    const ONE_MB: usize = 1024 * 1024;
    let one_mb_bytes = generate_deterministic_payload(0xDEAD_BEEF_0001, ONE_MB);
    let asset_hash = "sha256_large_1mb_asset_empirical_proof";

    let mut doc = Document::new_empty("Zero BLOB Rewrite Stress Test");
    let page_id = doc.pages[0].id;

    // 2. Salva l'asset una tantum
    {
        let store = DocumentStore::open_or_create(&path).expect("open store");
        store
            .save_asset(asset_hash, "image/png", &one_mb_bytes)
            .expect("save 1MB asset");
    }

    // 3. Aggiunge ImageItem al documento
    let img_item = make_test_image(
        1,
        1,
        page_id,
        asset_hash,
        Rect::new(20.0, 20.0, 400.0, 300.0),
        1920,
        1080,
    );
    doc.pages[0].items.push(img_item);

    // Salva lo stato base del documento
    {
        let mut store = DocumentStore::open_or_create(&path).expect("open store");
        store.save_document(&doc).expect("initial save_document");
    }

    // Ispeziona created_at e rowid dell'asset
    let (initial_created_at, initial_rowid): (i64, i64) = {
        let store = DocumentStore::open_or_create(&path).expect("open store");
        store
            .connection()
            .query_row(
                "SELECT created_at, rowid FROM assets WHERE hash = ?1;",
                params![asset_hash],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("read asset initial meta")
    };

    let mut latencies_ms: Vec<f64> = Vec::with_capacity(100);

    // 4. Esegui 100 salvataggi consecutivi aggiungendo ogni volta un nuovo tratto
    let mut store = DocumentStore::open_or_create(&path).expect("open store");

    for i in 1..=100 {
        // Aggiungi un nuovo tratto con 40 punti
        let stroke_item = make_test_stroke(
            (i + 1) as u64,
            (i + 1) as u64,
            40,
            (i as f64) * 2.0,
            (i as f64) * 3.0,
        );
        doc.pages[0].items.push(stroke_item);

        let t0 = Instant::now();
        store.save_document(&doc).expect("save_document");
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        latencies_ms.push(elapsed_ms);

        // Verifica rigida: la latenza del salvataggio deve restare < 50 ms
        assert!(
            elapsed_ms < 50.0,
            "Save latency exceeded 50ms at iteration {}: {:.2} ms",
            i,
            elapsed_ms
        );

        // Controllo periodico (ogni 20 salvataggi) dei metadati SQLite dell'asset
        if i % 20 == 0 {
            let (curr_created_at, curr_rowid): (i64, i64) = store
                .connection()
                .query_row(
                    "SELECT created_at, rowid FROM assets WHERE hash = ?1;",
                    params![asset_hash],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .expect("check asset meta");

            assert_eq!(
                initial_created_at, curr_created_at,
                "Iteration {}: created_at must never change (zero BLOB rewrites invariant)",
                i
            );
            assert_eq!(
                initial_rowid, curr_rowid,
                "Iteration {}: rowid must remain unchanged",
                i
            );
        }
    }

    // 5. Statistiche empiriche di latenza
    latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min_latency = latencies_ms[0];
    let max_latency = latencies_ms[latencies_ms.len() - 1];
    let median_latency = latencies_ms[latencies_ms.len() / 2];
    let p95_latency = latencies_ms[(latencies_ms.len() as f64 * 0.95) as usize];
    let p99_latency = latencies_ms[(latencies_ms.len() as f64 * 0.99) as usize];
    let avg_latency = latencies_ms.iter().sum::<f64>() / latencies_ms.len() as f64;

    println!(
        "[EMPIRICAL ZERO BLOB REWRITE] 100 consecutive saves with 1MB asset: \
        min={:.2}ms, avg={:.2}ms, median(p50)={:.2}ms, p95={:.2}ms, p99={:.2}ms, max={:.2}ms",
        min_latency, avg_latency, median_latency, p95_latency, p99_latency, max_latency
    );

    assert!(
        p99_latency < 50.0,
        "P99 latency must be strictly < 50ms, got {:.2}ms",
        p99_latency
    );

    // 6. Dimensione su disco: Se il BLOB da 1MB fosse riscritto ad ogni salvataggio,
    // il file WAL avrebbe accumulato oltre 100 MB di dati.
    let wal_path = path.with_extension("uninote-wal");
    let wal_size = if wal_path.exists() {
        fs::metadata(&wal_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };
    let main_db_size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let total_disk_size = main_db_size + wal_size;

    println!(
        "[EMPIRICAL] Total DB+WAL disk size after 100 saves: {:.2} MB (main: {:.2} MB, wal: {:.2} MB)",
        total_disk_size as f64 / (1024.0 * 1024.0),
        main_db_size as f64 / (1024.0 * 1024.0),
        wal_size as f64 / (1024.0 * 1024.0)
    );

    // La dimensione totale combinata DB + WAL deve essere decisamente inferiore a 30 MB
    // (a riprova che 100 MB di BLOB non sono mai stati generati né riscritti in WAL).
    assert!(
        total_disk_size < 30 * 1024 * 1024,
        "Disk size ({:.2} MB) indicates BLOB rewrites occurred during stroke saves!",
        total_disk_size as f64 / (1024.0 * 1024.0)
    );

    // 7. Ricarica del documento e dell'asset: verifica integrità completa
    let loaded_doc = store.load_document().expect("load_document");
    assert_eq!(loaded_doc.pages[0].items.len(), 101); // 1 Image + 100 Strokes
    assert_eq!(loaded_doc.pages[0].images().count(), 1);
    assert_eq!(loaded_doc.pages[0].strokes().count(), 100);

    let loaded_asset = store
        .load_asset(asset_hash)
        .expect("load_asset")
        .expect("asset must exist");
    assert_eq!(loaded_asset.len(), ONE_MB);
    assert_eq!(loaded_asset, one_mb_bytes);
}

// =========================================================================
// 3. BACKUP PORTABILITY TEST (sqlite3_backup VIA export_portable_copy)
// =========================================================================

#[test]
fn test_adversarial_backup_portability_sqlite3_backup() {
    let src_file = NamedTempFile::new().expect("create src temp file");
    let dst_file = NamedTempFile::new().expect("create dst temp file");
    let src_path = src_file.path();
    let dst_path = dst_file.path();

    // 1. Popola il database sorgente con molteplici asset di dimensioni varie e tratti su 3 pagine
    let asset_specs: Vec<(&str, &str, usize, u64)> = vec![
        ("thumb_16k", "image/png", 16 * 1024, 0x1111),
        ("medium_256k", "image/jpeg", 256 * 1024, 0x2222),
        ("large_1_5m", "image/png", 1536 * 1024, 0x3333),
        ("vector_64k", "image/webp", 64 * 1024, 0x4444),
        ("icon_2k", "image/png", 2 * 1024, 0x5555),
    ];

    let mut payloads: std::collections::HashMap<String, Vec<u8>> = std::collections::HashMap::new();

    let mut doc = Document::new_empty("Multi-Page Backup Portability Source");
    let page0_id = doc.pages[0].id;
    let page1_id = doc.add_page(PageBackground::Ruled {
        line_spacing: 24.0,
        margin_left: 40.0,
    });
    let page2_id = doc.add_page(PageBackground::Grid { grid_size: 20.0 });

    {
        let mut store = DocumentStore::open_or_create(src_path).expect("open src store");

        // Salva gli asset
        for (hash, mime, size, seed) in &asset_specs {
            let data = generate_deterministic_payload(*seed, *size);
            store.save_asset(hash, mime, &data).expect("save asset");
            payloads.insert(hash.to_string(), data);
        }

        // Distribuisci tratti e immagini sulle 3 pagine:
        // Pagina 0: 10 tratti + 2 immagini
        for i in 0..10 {
            doc.pages[0]
                .items
                .push(make_test_stroke(i + 1, i + 1, 20, (i as f64) * 10.0, 50.0));
        }
        doc.pages[0].items.push(make_test_image(
            101,
            11,
            page0_id,
            "thumb_16k",
            Rect::new(10.0, 10.0, 100.0, 100.0),
            200,
            200,
        ));
        doc.pages[0].items.push(make_test_image(
            102,
            12,
            page0_id,
            "large_1_5m",
            Rect::new(150.0, 50.0, 500.0, 400.0),
            1920,
            1080,
        ));

        // Pagina 1: 8 tratti + 2 immagini (inclusa una referenza deduplicata a "thumb_16k")
        for i in 0..8 {
            doc.pages[1]
                .items
                .push(make_test_stroke(i + 201, i + 1, 25, (i as f64) * 8.0, 80.0));
        }
        doc.pages[1].items.push(make_test_image(
            301,
            9,
            page1_id,
            "thumb_16k", // Deduplicata!
            Rect::new(20.0, 20.0, 80.0, 80.0),
            200,
            200,
        ));
        doc.pages[1].items.push(make_test_image(
            302,
            10,
            page1_id,
            "medium_256k",
            Rect::new(120.0, 120.0, 320.0, 240.0),
            800,
            600,
        ));

        // Pagina 2: 7 tratti + 2 immagini
        for i in 0..7 {
            doc.pages[2].items.push(make_test_stroke(
                i + 401,
                i + 1,
                30,
                (i as f64) * 15.0,
                120.0,
            ));
        }
        doc.pages[2].items.push(make_test_image(
            501,
            8,
            page2_id,
            "vector_64k",
            Rect::new(30.0, 30.0, 200.0, 150.0),
            640,
            480,
        ));
        doc.pages[2].items.push(make_test_image(
            502,
            9,
            page2_id,
            "icon_2k",
            Rect::new(250.0, 50.0, 64.0, 64.0),
            64,
            64,
        ));

        // Salva documento nel database sorgente
        store.save_document(&doc).expect("save_document on src");

        // 2. Esegui export_portable_copy usando la connessione viva (in WAL mode attiva)
        let t_backup = Instant::now();
        export_portable_copy(store.connection(), dst_path).expect("export_portable_copy");
        let elapsed_backup = t_backup.elapsed();
        println!(
            "[EMPIRICAL] export_portable_copy completed in {:?} for multi-megabyte database",
            elapsed_backup
        );
    } // Chiudi lo store sorgente

    // 3. Verifica isolamento: Rimuovi i file sorgente dal disco per garantire zero dipendenze
    let _ = fs::remove_file(src_path);
    let _ = fs::remove_file(src_path.with_extension("uninote-wal"));
    let _ = fs::remove_file(src_path.with_extension("uninote-shm"));

    // 4. Apri la copia esportata in una connessione completamente fresca
    let fresh_store = DocumentStore::open_or_create(dst_path).expect("open dst copy");

    // Verifica PRAGMA integrity_check
    let integrity_ok = fresh_store.verify_integrity().expect("integrity_check");
    assert!(
        integrity_ok,
        "Destination backup database must pass PRAGMA integrity_check"
    );

    // Verifica SCHEMA_VERSION == 3
    let dst_ver: u32 = fresh_store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("user_version");
    assert_eq!(dst_ver, 3);

    // 5. Verifica che ogni singolo asset sia identico byte-per-byte
    for (hash, _, _, _) in &asset_specs {
        let loaded = fresh_store
            .load_asset(hash)
            .expect("load_asset")
            .unwrap_or_else(|| panic!("Asset {} missing in backup destination", hash));
        let expected = payloads.get(*hash).unwrap();
        assert_eq!(
            loaded.len(),
            expected.len(),
            "Asset {} size mismatch in backup",
            hash
        );
        assert_eq!(
            &loaded, expected,
            "Asset {} byte content corrupted in backup",
            hash
        );
    }

    // 6. Ricarica il documento e confronta la struttura completa
    let loaded_doc = fresh_store.load_document().expect("load_document");
    assert_eq!(loaded_doc.pages.len(), 3);
    assert_eq!(loaded_doc.title, "Multi-Page Backup Portability Source");

    // Verifica pagina 0
    assert_eq!(loaded_doc.pages[0].strokes().count(), 10);
    assert_eq!(loaded_doc.pages[0].images().count(), 2);

    // Verifica pagina 1
    assert_eq!(loaded_doc.pages[1].strokes().count(), 8);
    assert_eq!(loaded_doc.pages[1].images().count(), 2);

    // Verifica pagina 2
    assert_eq!(loaded_doc.pages[2].strokes().count(), 7);
    assert_eq!(loaded_doc.pages[2].images().count(), 2);

    // Verifica che gli indici spaziali R-Tree delle pagine siano stati reidratati
    assert_eq!(loaded_doc.pages[0].spatial_index.len(), 12);
    assert_eq!(loaded_doc.pages[1].spatial_index.len(), 10);
    assert_eq!(loaded_doc.pages[2].spatial_index.len(), 9);

    // 7. Modifica e risalvataggio sulla copia portabile: deve essere pienamente funzionante
    let mut writable_store = fresh_store;
    let mut mod_doc = loaded_doc;
    mod_doc.pages[0]
        .items
        .push(make_test_stroke(9999, 99, 10, 50.0, 50.0));
    writable_store
        .save_document(&mod_doc)
        .expect("save on backup copy");

    let post_mod_doc = writable_store
        .load_document()
        .expect("reload modified backup");
    assert_eq!(post_mod_doc.pages[0].items.len(), 13);
}

// =========================================================================
// 4. V1 AND V2 LEGACY DATABASE MIGRATION STRESS
// =========================================================================

/// Helper che crea un database legacy V1 sintetico sul filesystem (user_version = 1, senza background_extra né assets).
fn create_synthetic_legacy_v1_database(path: &Path) {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .expect("create legacy v1");

    conn.execute_batch(
        "PRAGMA application_id = 0x554E494E;
         PRAGMA user_version = 1;
         PRAGMA journal_mode = WAL;

         CREATE TABLE metadata (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );

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
    .expect("setup v1 tables");

    // Inserisci metadati V1
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('doc_id', 'a1b2c3d4-0000-0000-0000-111122223333')",
        [],
    )
    .expect("insert v1 doc_id");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('title', 'Legacy V1 Academic Notebook')",
        [],
    )
    .expect("insert v1 title");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('version', '1')",
        [],
    )
    .expect("insert v1 version");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('document_format', 'notebook')",
        [],
    )
    .expect("insert v1 format");

    // Popola 3 pagine V1 con sfondi classici
    let page_uuids = [
        "10000000-0000-0000-0000-000000000001",
        "10000000-0000-0000-0000-000000000002",
        "10000000-0000-0000-0000-000000000003",
    ];

    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
         VALUES (?1, 0, 595.28, 841.89, 'blank', 0.0)",
        [page_uuids[0]],
    )
    .expect("insert p0");

    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
         VALUES (?1, 1, 595.28, 841.89, 'ruled', 25.0)",
        [page_uuids[1]],
    )
    .expect("insert p1");

    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
         VALUES (?1, 2, 595.28, 841.89, 'grid', 20.0)",
        [page_uuids[2]],
    )
    .expect("insert p2");

    // Popola 30 tratti realistici distribuiti sulle 3 pagine
    let mut stroke_id = 1;
    for p_uuid in &page_uuids {
        for _ in 0..10 {
            let stroke_json = format!(
                r#"{{
                    "id": {stroke_id},
                    "style": {{
                        "color": [0.0, 0.2, 0.8, 1.0],
                        "base_width": 2.5,
                        "min_width_factor": 0.5,
                        "max_width_factor": 1.5
                    }},
                    "points": [
                        {{"x": 10.0, "y": 20.0, "pressure": 0.5, "width": 2.5, "timestamp_ns": 100}},
                        {{"x": 30.0, "y": 45.0, "pressure": 0.6, "width": 2.7, "timestamp_ns": 200}},
                        {{"x": 70.0, "y": 90.0, "pressure": 0.7, "width": 2.8, "timestamp_ns": 300}}
                    ],
                    "raw_samples": [],
                    "is_dot": false,
                    "mode": "CatmullRomSpline"
                }}"#
            );
            conn.execute(
                "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
                 VALUES (?1, ?2, ?3, 'stroke', '{\"min_x\": 8.0, \"min_y\": 18.0, \"max_x\": 72.0, \"max_y\": 92.0}', ?4)",
                params![stroke_id, p_uuid, stroke_id, stroke_json],
            )
            .expect("insert v1 stroke");
            stroke_id += 1;
        }
    }
}

/// Helper che crea un database legacy V2 sintetico sul filesystem (user_version = 2, con background_extra per PDF, ma senza assets).
fn create_synthetic_legacy_v2_database(path: &Path) {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
    )
    .expect("create legacy v2");

    conn.execute_batch(
        "PRAGMA application_id = 0x554E494E;
         PRAGMA user_version = 2;
         PRAGMA journal_mode = WAL;

         CREATE TABLE metadata (
             key TEXT PRIMARY KEY,
             value TEXT NOT NULL
         );

         CREATE TABLE pages (
             id TEXT PRIMARY KEY,
             page_index INTEGER NOT NULL,
             width REAL NOT NULL,
             height REAL NOT NULL,
             background_type TEXT NOT NULL,
             background_spacing REAL NOT NULL,
             background_extra TEXT NOT NULL DEFAULT ''
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
    .expect("setup v2 tables");

    // Inserisci metadati V2
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('doc_id', 'b2c3d4e5-0000-0000-0000-222233334444')",
        [],
    )
    .expect("insert v2 doc_id");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('title', 'Legacy V2 PDF Document')",
        [],
    )
    .expect("insert v2 title");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('version', '2')",
        [],
    )
    .expect("insert v2 version");
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('document_format', 'notebook')",
        [],
    )
    .expect("insert v2 format");

    // Popola 2 pagine: una con PDF background_extra JSON, una ruled
    let p0_uuid = "20000000-0000-0000-0000-000000000001";
    let p1_uuid = "20000000-0000-0000-0000-000000000002";

    let pdf_meta = PdfPageStorageMeta {
        source_path: "/home/user/Documents/dispense.pdf".to_string(),
        page_index: 0,
        original_width: 595.28,
        original_height: 841.89,
    };
    let pdf_meta_json = serde_json::to_string(&pdf_meta).unwrap();

    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
         VALUES (?1, 0, 595.28, 841.89, 'pdf_page', 0.0, ?2)",
        params![p0_uuid, pdf_meta_json],
    )
    .expect("insert v2 pdf page");

    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
         VALUES (?1, 1, 595.28, 841.89, 'ruled', 25.0, '')",
        params![p1_uuid],
    )
    .expect("insert v2 ruled page");

    // Popola 15 tratti
    for i in 1..=15 {
        let p_uuid = if i <= 10 { p0_uuid } else { p1_uuid };
        let stroke_json = format!(
            r#"{{
                "id": {i},
                "style": {{
                    "color": [0.9, 0.1, 0.1, 1.0],
                    "base_width": 2.0,
                    "min_width_factor": 0.5,
                    "max_width_factor": 1.5
                }},
                "points": [
                    {{"x": 50.0, "y": 60.0, "pressure": 0.5, "width": 2.0, "timestamp_ns": 100}},
                    {{"x": 100.0, "y": 120.0, "pressure": 0.6, "width": 2.2, "timestamp_ns": 200}}
                ],
                "raw_samples": [],
                "is_dot": false,
                "mode": "CatmullRomSpline"
            }}"#
        );
        conn.execute(
            "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
             VALUES (?1, ?2, ?3, 'stroke', '{\"min_x\": 48.0, \"min_y\": 58.0, \"max_x\": 102.0, \"max_y\": 122.0}', ?4)",
            params![i, p_uuid, i, stroke_json],
        )
        .expect("insert v2 stroke");
    }
}

#[test]
fn test_adversarial_v1_legacy_database_migration_to_v3() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path();

    // 1. Crea database sintetico V1
    create_synthetic_legacy_v1_database(path);

    // Verifica prima dell'apertura: user_version == 1 e tabella assets NON presente
    {
        let conn = Connection::open(path).expect("open raw");
        let v: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 1);
        let assets_exists: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='assets';",
                [],
                |r| r.get::<_, i64>(0),
            )
            .map(|c| c == 1)
            .unwrap();
        assert!(!assets_exists, "V1 database must not have assets table");
    }

    // 2. Apri tramite DocumentStore::open_or_create (deve innescare la migrazione atomica a V3)
    let mut store = DocumentStore::open_or_create(path).expect("open_or_create must migrate V1");

    // 3. Verifiche dello schema post-migrazione
    let post_version: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("get user_version");
    assert_eq!(
        post_version, SCHEMA_VERSION,
        "user_version must be upgraded to 3"
    );

    // Verifica presenza colonna background_extra su pages
    {
        let mut stmt = store
            .connection()
            .prepare("PRAGMA table_info(pages);")
            .expect("table_info pages");
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .expect("query cols")
            .filter_map(Result::ok)
            .collect();
        assert!(
            cols.contains(&"background_extra".to_string()),
            "pages must have background_extra"
        );
    }

    // Verifica presenza tabella assets
    let assets_exists: bool = store
        .connection()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='assets';",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c == 1)
        .expect("query assets");
    assert!(
        assets_exists,
        "assets table must be created by V3 migration"
    );

    // 4. Verifica zero data loss: tutte le 3 pagine e i 30 tratti V1 sono intatti
    let doc = store.load_document().expect("load_document");
    assert_eq!(doc.title, "Legacy V1 Academic Notebook");
    assert_eq!(doc.format, DocumentFormat::Notebook);
    assert_eq!(doc.pages.len(), 3);
    assert_eq!(doc.pages[0].background, PageBackground::Blank);
    assert!(matches!(
        doc.pages[1].background,
        PageBackground::Ruled { .. }
    ));
    assert!(matches!(
        doc.pages[2].background,
        PageBackground::Grid { .. }
    ));

    let total_strokes: usize = doc.pages.iter().map(|p| p.strokes().count()).sum();
    assert_eq!(total_strokes, 30, "All 30 V1 strokes must be preserved");

    // 5. Verifica che il database migrato possa ora accogliere nuovi asset e immagini
    let asset_hash = "v1_migrated_new_asset_hash";
    let asset_data = vec![1, 2, 3, 4, 5, 6, 7, 8];
    store
        .save_asset(asset_hash, "image/png", &asset_data)
        .expect("save_asset on migrated V1 DB");
    let loaded_asset = store
        .load_asset(asset_hash)
        .expect("load_asset")
        .expect("asset exists");
    assert_eq!(loaded_asset, asset_data);

    // Aggiungi un ImageItem alla prima pagina e salva il documento
    let mut updated_doc = doc;
    let page0_id = updated_doc.pages[0].id;
    updated_doc.pages[0].items.push(make_test_image(
        1001,
        31,
        page0_id,
        asset_hash,
        Rect::new(50.0, 50.0, 150.0, 150.0),
        300,
        300,
    ));
    store
        .save_document(&updated_doc)
        .expect("save_document with ImageItem");

    // Ricarica e conferma
    let reloaded = store.load_document().expect("reloaded migrated doc");
    assert_eq!(reloaded.pages[0].images().count(), 1);
    assert_eq!(reloaded.pages[0].strokes().count(), 10);
    assert!(store.verify_integrity().expect("integrity_check"));
}

#[test]
fn test_adversarial_v2_legacy_database_migration_to_v3() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path();

    // 1. Crea database sintetico V2
    create_synthetic_legacy_v2_database(path);

    // Verifica prima dell'apertura: user_version == 2 e tabella assets assente
    {
        let conn = Connection::open(path).expect("open raw");
        let v: u32 = conn
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .unwrap();
        assert_eq!(v, 2);
    }

    // 2. Apri con DocumentStore::open_or_create
    let store = DocumentStore::open_or_create(path).expect("open_or_create migrate V2");

    // 3. Verifiche V3
    let post_version: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("user_version");
    assert_eq!(post_version, 3);

    let assets_exists: bool = store
        .connection()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='assets';",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c == 1)
        .expect("query assets");
    assert!(assets_exists);

    // 4. Verifica zero data loss: PDF background preservato al 100%
    let doc = store.load_document().expect("load_document");
    assert_eq!(doc.title, "Legacy V2 PDF Document");
    assert_eq!(doc.pages.len(), 2);

    match &doc.pages[0].background {
        PageBackground::PdfPage {
            source_path,
            page_index,
            original_width,
            original_height,
        } => {
            assert_eq!(source_path, "/home/user/Documents/dispense.pdf");
            assert_eq!(*page_index, 0);
            assert_eq!(*original_width, 595.28);
            assert_eq!(*original_height, 841.89);
        }
        other => panic!("Page 0 background must be PdfPage, got: {:?}", other),
    }

    let total_strokes: usize = doc.pages.iter().map(|p| p.strokes().count()).sum();
    assert_eq!(total_strokes, 15, "All 15 V2 strokes must be preserved");

    // 5. Salva un asset e verifica integrità
    store
        .save_asset("v2_asset_proof", "image/png", &[9, 9, 9])
        .expect("save_asset");
    assert_eq!(
        store.load_asset("v2_asset_proof").unwrap().unwrap(),
        vec![9, 9, 9]
    );
    assert!(store.verify_integrity().expect("integrity_check"));
}

// =========================================================================
// 5. WAL CONCURRENCY & MULTI-THREADING ADVERSARIAL STRESS
// =========================================================================

#[test]
fn test_adversarial_wal_concurrency_reader_during_rapid_asset_writes() {
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path().to_path_buf();

    // Inizializza schema
    {
        let _ = DocumentStore::open_or_create(&path).expect("open_or_create");
    }

    let stop_signal = Arc::new(AtomicBool::new(false));
    let stop_signal_clone = stop_signal.clone();
    let reader_ready = Arc::new(AtomicBool::new(false));
    let reader_ready_clone = reader_ready.clone();
    let reader_path = path.clone();

    // Thread Lettore concorrente: legge continuamente asset e documenti senza bloccare
    let reader_handle = thread::spawn(move || {
        let reader_store = DocumentStore::open_or_create(&reader_path).expect("reader open");
        let mut read_cycles = 0;
        reader_ready_clone.store(true, Ordering::Release);
        while !stop_signal_clone.load(Ordering::Acquire) {
            // Esegui query sul database
            let count: Result<i64, _> =
                reader_store
                    .connection()
                    .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0));
            assert!(
                count.is_ok(),
                "Reader must not fail with SQLITE_BUSY: {:?}",
                count.err()
            );

            // Carica un asset casuale se disponibile
            let _ = reader_store.load_asset("asset_concurrency_010");
            let _ = reader_store.load_document();

            read_cycles += 1;
            thread::yield_now();
        }
        read_cycles
    });

    // Attendi che il lettore sia inizializzato e in esecuzione
    while !reader_ready.load(Ordering::Acquire) {
        thread::yield_now();
    }

    // Thread Scrittore (principale): esegue 200 salvataggi rapidi di asset e revisioni di documenti
    let mut writer_store = DocumentStore::open_or_create(&path).expect("writer open");
    let test_data = generate_deterministic_payload(0x9999, 16 * 1024);

    let mut doc = Document::new_empty("WAL Concurrency Test");

    for i in 0..200 {
        let hash = format!("asset_concurrency_{:03}", i);
        writer_store
            .save_asset(&hash, "image/png", &test_data)
            .unwrap_or_else(|e| panic!("Writer failed on asset {}: {}", i, e));

        if i % 10 == 0 {
            doc.pages[0]
                .items
                .push(make_test_stroke(i + 1, i + 1, 10, i as f64, i as f64));
            writer_store
                .save_document(&doc)
                .unwrap_or_else(|e| panic!("Writer failed on doc save {}: {}", i, e));
        }
        thread::yield_now();
    }

    // Ferma il lettore
    stop_signal.store(true, Ordering::Relaxed);
    let total_reads = reader_handle.join().expect("reader thread join");

    println!(
        "[EMPIRICAL WAL CONCURRENCY] Reader executed {} non-blocking read cycles during 200 writes",
        total_reads
    );
    assert!(
        total_reads > 10,
        "Reader must have executed multiple cycles"
    );

    // Verifica integrità finale
    assert!(writer_store.verify_integrity().expect("integrity_check"));
}

#[test]
fn test_adversarial_concurrent_migration_stress_multi_thread() {
    // 8 thread concorrenti che tentano simultaneamente di aprire ed eseguire la migrazione
    // dello stesso file SQLite V1 legacy non migrato
    let temp_file = NamedTempFile::new().expect("create temp file");
    let path = temp_file.path().to_path_buf();
    create_synthetic_legacy_v1_database(&path);

    let mut handles = Vec::new();
    for thread_idx in 0..8 {
        let p = path.clone();
        let handle = thread::spawn(move || {
            let res = DocumentStore::open_or_create(&p);
            assert!(
                res.is_ok(),
                "Thread {} failed to open/migrate legacy database: {:?}",
                thread_idx,
                res.err()
            );
            let store = res.unwrap();
            let ver: u32 = store
                .connection()
                .query_row("PRAGMA user_version;", [], |r| r.get(0))
                .expect("query ver");
            assert_eq!(ver, 3, "Thread {} saw invalid version: {}", thread_idx, ver);
            let doc = store.load_document().expect("load_document");
            assert_eq!(doc.pages.len(), 3);
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("thread join");
    }

    // Verifica finale
    let store = DocumentStore::open_or_create(&path).expect("open final");
    assert!(store.verify_integrity().expect("integrity_check"));
    let assets_count: i64 = store
        .connection()
        .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0))
        .expect("count assets");
    assert_eq!(assets_count, 0);
}
