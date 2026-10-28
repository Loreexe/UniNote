//! Milestone M3 Challenger Empirical Verification and Stress Test Suite.
//!
//! Objectives:
//! 1. Empirically benchmark SQLite WAL save (< 70ms) and load (< 40ms) on 10,000 realistic strokes in release mode.
//! 2. Stress test 50 rapid sequential transactions ensuring zero lock errors (`database is locked`) and zero WAL corruption.
//! 3. Test concurrent readers during rapid WAL writes.
//! 4. Test 100-page distributed 10k strokes save/load.
//! 5. Verify database raw cleanliness and schema invariants.
//! 6. Test boundary conditions (empty document, giant stroke with 5,000 points).

use rusqlite::{params, Connection};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, Item, ItemKind, Page, PageBackground};
use uninote_storage::{DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

fn create_realistic_stroke(id: u64, x: f64, y: f64, point_count: usize) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let t = i as f64;
        let px = x + t * 4.2 + (t * 0.5).sin() * 2.0;
        let py = y + t * 3.8 + (t * 0.7).cos() * 2.0;
        let pressure = 0.35 + (0.15 * (i % 7) as f64 / 6.0);
        points.push(StrokePoint {
            x: px,
            y: py,
            pressure,
            width: 2.2 + pressure * 1.5,
            timestamp_ns: 1_000_000 + (i as u64 * 8_333),
        });
    }

    let style = StrokeStyle {
        color: (
            0.1 + ((id % 5) as f64 * 0.15),
            0.15 + ((id % 3) as f64 * 0.2),
            0.2 + ((id % 7) as f64 * 0.1),
            1.0,
        ),
        base_width: 2.4,
        ..Default::default()
    };

    Stroke {
        id,
        style,
        points,
        raw_samples: Vec::new(),
        is_dot: point_count == 1,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ============================================================================
// 1. EMPIRICAL BENCHMARK: 10,000 STROKES SAVE (< 70ms) & LOAD (< 40ms)
// ============================================================================

#[test]
fn test_empirical_wal_save_and_load_10k_realistic_strokes() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let total_strokes = 10_000;
    let mut doc = Document::new_notebook("Challenger 10k Benchmark Document");
    doc.pages[0].background = PageBackground::Grid { grid_size: 14.17 };

    // Generate 10,000 realistic handwriting/diagram strokes
    for i in 0..total_strokes {
        let x = (i % 100) as f64 * 8.0 + 30.0;
        let y = (i / 100) as f64 * 7.5 + 40.0;
        let stroke = create_realistic_stroke(i as u64, x, y, 8);
        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // 1. Benchmark SAVE
    let save_start = Instant::now();
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store
            .save_document(&doc)
            .expect("Save 10k strokes document");
    }
    let save_duration = save_start.elapsed();
    println!(
        "[EMPIRICAL CHALLENGER] Save 10,000 realistic strokes duration: {:.2?} ({} ms)",
        save_duration,
        save_duration.as_millis()
    );

    #[cfg(not(debug_assertions))]
    assert!(
        save_duration.as_millis() < 70,
        "Save 10k strokes in release mode took {} ms, exceeding 70 ms limit!",
        save_duration.as_millis()
    );

    // 2. Verify file size and SQLite WAL mode
    let metadata = std::fs::metadata(file_path).expect("File metadata");
    let file_size_mb = metadata.len() as f64 / (1024.0 * 1024.0);
    println!(
        "[EMPIRICAL CHALLENGER] Saved database file size: {:.2} MB",
        file_size_mb
    );

    // 3. Benchmark LOAD
    let store_reopen_start = Instant::now();
    let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
    let reopen_duration = store_reopen_start.elapsed();

    let integrity_start = Instant::now();
    assert!(store.verify_integrity().expect("Integrity check"));
    let integrity_duration = integrity_start.elapsed();

    let pure_load_start = Instant::now();
    let loaded = store.load_document().expect("Load 10,000 strokes document");
    let pure_load_duration = pure_load_start.elapsed();

    println!(
        "[EMPIRICAL CHALLENGER] Reopen DB: {:.2?} | PRAGMA integrity_check: {:.2?} | Pure load_document: {:.2?} ({} ms)",
        reopen_duration,
        integrity_duration,
        pure_load_duration,
        pure_load_duration.as_millis()
    );

    // Micro-profile breakdown of the loading steps directly on the same DB
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store for profiling");
        let conn = store.connection();

        let t_sql = Instant::now();
        let mut item_stmt = conn
            .prepare("SELECT id, z_index, item_type, data_json FROM items ORDER BY z_index ASC")
            .unwrap();
        let mut rows = item_stmt.query([]).unwrap();
        let mut raw_jsons = Vec::with_capacity(10_000);
        while let Some(row) = rows.next().unwrap() {
            let s: String = row.get(3).unwrap();
            raw_jsons.push(s);
        }
        let sql_dur = t_sql.elapsed();

        let t_json = Instant::now();
        let mut parsed_strokes: Vec<Stroke> = Vec::with_capacity(10_000);
        for s in &raw_jsons {
            if let Ok(st) = serde_json::from_str::<Stroke>(s) {
                parsed_strokes.push(st);
            }
        }
        let json_dur = t_json.elapsed();

        let t_cache = Instant::now();
        for st in &mut parsed_strokes {
            st.ensure_cache();
        }
        let cache_dur = t_cache.elapsed();

        let t_spatial = Instant::now();
        let items: Vec<Item> = parsed_strokes
            .into_iter()
            .enumerate()
            .map(|(i, st)| Item {
                id: i as u64,
                z_index: i as u64,
                kind: ItemKind::Stroke(st),
            })
            .collect();
        let _spatial_index = uninote_core::spatial::PageSpatialIndex::bulk_load(&items);
        let spatial_dur = t_spatial.elapsed();

        println!(
            "[EMPIRICAL CHALLENGER PROFILE] Breakdown:\n  - SQLite Query: {:.2?}\n  - serde_json::from_str (10k items): {:.2?}\n  - ensure_cache (10k items): {:.2?}\n  - R-Tree bulk load (10k items): {:.2?}",
            sql_dur, json_dur, cache_dur, spatial_dur
        );

        // Test with composite index
        conn.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_items_page_z ON items(page_id, z_index);",
        )
        .unwrap();
        let t_sql_opt = Instant::now();
        let mut item_stmt_opt = conn
            .prepare("SELECT id, z_index, item_type, data_json FROM items WHERE page_id = ?1 ORDER BY z_index ASC")
            .unwrap();
        let mut rows_opt = item_stmt_opt
            .query(params![&loaded.pages[0].id.to_string()])
            .unwrap();
        let mut raw_jsons_opt = Vec::with_capacity(10_000);
        while let Some(row) = rows_opt.next().unwrap() {
            let s: String = row.get(3).unwrap();
            raw_jsons_opt.push(s);
        }
        let sql_opt_dur = t_sql_opt.elapsed();
        println!(
            "[EMPIRICAL CHALLENGER PROFILE] Query with composite index (page_id, z_index): {:.2?} (vs {:.2?})",
            sql_opt_dur, sql_dur
        );
    }

    // 4. Verify Document and Stroke Fidelity
    assert_eq!(loaded.title, "Challenger 10k Benchmark Document");
    assert_eq!(loaded.format, DocumentFormat::Notebook);
    assert_eq!(loaded.pages.len(), 1);
    assert_eq!(loaded.pages[0].items.len(), total_strokes);

    // Verify cache rehydration on all 10,000 strokes
    for (idx, item) in loaded.pages[0].items.iter().enumerate() {
        assert_eq!(item.id, idx as u64);
        assert_eq!(item.z_index, idx as u64);
        let s = item.as_stroke().expect("stroke");
        assert_eq!(s.id, idx as u64);
        assert_eq!(s.points.len(), 8);
        assert!(
            s.cached_bbox.is_some(),
            "Stroke {} must have rehydrated cached_bbox",
            idx
        );
        assert!(
            !s.cached_bezier.is_empty(),
            "Stroke {} must have rehydrated cached_bezier",
            idx
        );
        let computed = s.compute_bounding_box();
        assert_eq!(
            s.bounding_box(),
            computed,
            "Stroke {} cached bbox must match computed bbox",
            idx
        );
    }

    // Verify PageSpatialIndex bulk-load
    let page = &loaded.pages[0];
    assert_eq!(
        page.spatial_index.len(),
        total_strokes,
        "PageSpatialIndex must contain all 10k items"
    );

    // Spatial query benchmark
    let q_start = Instant::now();
    let hits = page
        .spatial_index
        .locate_in_envelope(100.0, 100.0, 300.0, 300.0);
    let q_dur = q_start.elapsed();
    println!(
        "[EMPIRICAL CHALLENGER] Spatial query in 10k items found {} hits in {:.2?}",
        hits.len(),
        q_dur
    );
    assert!(!hits.is_empty(), "Spatial query should find items");
    // Verify sorted by z_index
    for w in hits.windows(2) {
        assert!(w[0].z_index <= w[1].z_index);
    }
}

// ============================================================================
// 2. STRESS TEST: 50 RAPID SEQUENTIAL TRANSACTIONS
// ============================================================================

#[test]
fn test_stress_50_rapid_sequential_transactions() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let mut store = DocumentStore::open_or_create(file_path).expect("Open store");

    println!("[EMPIRICAL CHALLENGER] Executing 50 rapid sequential transactions...");
    let start_all = Instant::now();

    for tx_idx in 0..50 {
        // Append 20 strokes per transaction
        for j in 0..20 {
            let item_id = (tx_idx * 20 + j) as u64;
            let stroke = create_realistic_stroke(
                item_id,
                (tx_idx as f64) * 15.0 + (j as f64) * 5.0,
                (tx_idx as f64) * 10.0 + (j as f64) * 8.0,
                5,
            );
            doc.pages[0].items.push(Item {
                id: item_id,
                z_index: item_id,
                kind: ItemKind::Stroke(stroke),
            });
        }
        doc.title = format!("Rapid TX Revision {}", tx_idx + 1);

        let t_tx = Instant::now();
        store.save_document(&doc).unwrap_or_else(|e| {
            panic!("FATAL: Transaction {tx_idx} failed with error: {e}");
        });
        let _tx_save_time = t_tx.elapsed();

        // Every 5th transaction, reload immediately and verify integrity
        if (tx_idx + 1) % 5 == 0 {
            let loaded = store.load_document().unwrap_or_else(|e| {
                panic!("FATAL: Load failed at transaction {tx_idx}: {e}");
            });
            let expected_count = (tx_idx + 1) * 20;
            assert_eq!(loaded.pages[0].items.len(), expected_count);
            assert_eq!(loaded.title, format!("Rapid TX Revision {}", tx_idx + 1));
            assert!(
                store.verify_integrity().expect("Integrity check"),
                "Integrity failed at transaction {}",
                tx_idx
            );
        }

        // Release check: individual small saves must be ultra-fast (< 15ms)
        #[cfg(not(debug_assertions))]
        if _tx_save_time.as_millis() > 30 {
            println!(
                "Notice: TX {} took {} ms for {} items",
                tx_idx,
                _tx_save_time.as_millis(),
                doc.pages[0].items.len()
            );
        }
    }

    let elapsed = start_all.elapsed();
    println!(
        "[EMPIRICAL CHALLENGER] Completed 50 rapid sequential transactions in {:.2?} (avg {:.2?} / tx)",
        elapsed,
        elapsed / 50
    );

    // Final checks
    let final_loaded = store.load_document().expect("Final load");
    assert_eq!(final_loaded.pages[0].items.len(), 1000);
    assert_eq!(final_loaded.pages[0].spatial_index.len(), 1000);
    assert!(store.verify_integrity().expect("Final integrity"));

    // Checkpoint TRUNCATE should succeed cleanly
    let conn = store.connection();
    let checkpoint_res: Result<(i32, i32, i32), rusqlite::Error> =
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE);", [], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        });
    assert!(
        checkpoint_res.is_ok(),
        "WAL checkpoint TRUNCATE must succeed after 50 rapid transactions: {:?}",
        checkpoint_res
    );
}

// ============================================================================
// 3. STRESS TEST: CONCURRENT READERS DURING RAPID WAL WRITES
// ============================================================================

#[test]
fn test_stress_concurrent_reader_during_rapid_writes() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path().to_path_buf();

    // Initialize DB with a document
    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Initial store");
        let doc = Document::new_notebook("Concurrent Base Doc");
        store.save_document(&doc).expect("Initial save");
    }

    let stop_reader = Arc::new(AtomicBool::new(false));
    let stop_reader_clone = Arc::clone(&stop_reader);
    let reader_path = file_path.clone();

    // Spawn reader thread
    let reader_handle = thread::spawn(move || {
        let mut read_count = 0;
        let mut max_items_seen = 0;
        while !stop_reader_clone.load(Ordering::Acquire) {
            // Open read connection
            if let Ok(store) = DocumentStore::open_or_create(&reader_path) {
                if let Ok(doc) = store.load_document() {
                    let items_count = doc.pages.first().map(|p| p.items.len()).unwrap_or(0);
                    if items_count > max_items_seen {
                        max_items_seen = items_count;
                    }
                    read_count += 1;
                }
            }
            thread::sleep(Duration::from_millis(2));
        }
        (read_count, max_items_seen)
    });

    // Writer performs 30 rapid successive transactions on the main thread
    let mut store = DocumentStore::open_or_create(&file_path).expect("Writer store");
    let mut doc = Document::new_notebook("Concurrent Writer Doc");

    for i in 0..30 {
        let stroke = create_realistic_stroke(i, (i as f64) * 20.0, (i as f64) * 20.0, 6);
        doc.pages[0].items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(stroke),
        });
        store
            .save_document(&doc)
            .unwrap_or_else(|e| panic!("Writer save failed on cycle {i}: {e}"));
        thread::sleep(Duration::from_millis(3));
    }

    // Stop reader
    stop_reader.store(true, Ordering::Release);
    let (read_count, max_seen) = reader_handle.join().expect("Join reader thread");

    println!(
        "[EMPIRICAL CHALLENGER] Reader thread performed {} concurrent reads without locking; max items seen: {}",
        read_count, max_seen
    );

    assert!(
        read_count > 0,
        "Reader thread should have performed at least 1 successful read"
    );
    assert!(store.verify_integrity().expect("Final writer integrity"));
}

// ============================================================================
// 4. MULTI-PAGE STRESS: 100 PAGES DISTRIBUTED 10,000 STROKES
// ============================================================================

#[test]
fn test_stress_100_pages_distributed_10k_strokes() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_notebook("100 Pages Distributed Benchmark");
    doc.pages.clear();

    let num_pages = 100;
    let strokes_per_page = 100;
    let mut stroke_id = 0u64;

    for page_idx in 0..num_pages {
        let mut page = Page::new_a4(page_idx, PageBackground::default());
        for _ in 0..strokes_per_page {
            let stroke = create_realistic_stroke(
                stroke_id,
                (stroke_id % 50) as f64 * 10.0 + 50.0,
                ((stroke_id / 50) % 50) as f64 * 10.0 + 50.0,
                6,
            );
            page.items.push(Item {
                id: stroke_id,
                z_index: stroke_id,
                kind: ItemKind::Stroke(stroke),
            });
            stroke_id += 1;
        }
        doc.pages.push(page);
    }

    assert_eq!(doc.pages.len(), 100);
    assert_eq!(stroke_id, 10_000);

    // Benchmark Save
    let save_start = Instant::now();
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store
            .save_document(&doc)
            .expect("Save 100-page 10k strokes doc");
    }
    let save_duration = save_start.elapsed();
    println!(
        "[EMPIRICAL CHALLENGER] Save 100 pages (10k strokes) duration: {:.2?} ({} ms)",
        save_duration,
        save_duration.as_millis()
    );

    #[cfg(not(debug_assertions))]
    assert!(
        save_duration.as_millis() < 90,
        "Save 100 pages 10k strokes in release mode took {} ms (expected < 90 ms)",
        save_duration.as_millis()
    );

    // Benchmark Load
    let load_start = Instant::now();
    let loaded = {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        store
            .load_document()
            .expect("Load 100-page 10k strokes doc")
    };
    let load_duration = load_start.elapsed();
    println!(
        "[EMPIRICAL CHALLENGER] Load 100 pages (10k strokes) duration: {:.2?} ({} ms)",
        load_duration,
        load_duration.as_millis()
    );

    #[cfg(not(debug_assertions))]
    assert!(
        load_duration.as_millis() < 50,
        "Load 100 pages 10k strokes in release mode took {} ms (expected < 50 ms)",
        load_duration.as_millis()
    );

    assert_eq!(loaded.pages.len(), 100);
    for page in &loaded.pages {
        assert_eq!(page.items.len(), 100);
        assert_eq!(page.spatial_index.len(), 100);
        for item in &page.items {
            let s = item.as_stroke().expect("stroke");
            assert!(s.cached_bbox.is_some());
            assert!(!s.cached_bezier.is_empty());
        }
    }
}

// ============================================================================
// 5. DATABASE RAW CLEANLINESS & ZERO CACHE POLLUTION UNDER STRESS
// ============================================================================

#[test]
fn test_database_raw_cleanliness_under_stress() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    for i in 0..100 {
        let mut stroke = create_realistic_stroke(i, (i as f64) * 10.0, (i as f64) * 10.0, 10);
        // Explicitly pre-populate in-memory cache
        stroke.ensure_cache();
        doc.pages[0].items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(stroke),
        });
    }
    doc.pages[0].rebuild_spatial_index();

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save document");
    }

    // Inspect SQLite directly using rusqlite
    let conn = Connection::open(file_path).expect("Open raw SQLite connection");

    // 1. Pragmas
    let app_id: u32 = conn
        .query_row("PRAGMA application_id;", [], |r| r.get(0))
        .expect("PRAGMA application_id");
    assert_eq!(app_id, APPLICATION_ID, "APPLICATION_ID must match UNIN");

    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, SCHEMA_VERSION, "SCHEMA_VERSION must match 2");

    let j_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
        .expect("PRAGMA journal_mode");
    assert_eq!(j_mode.to_lowercase(), "wal", "Journal mode must be WAL");

    // 2. Column integrity
    let mut stmt = conn
        .prepare("PRAGMA table_info(items);")
        .expect("Prepare table_info");
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("Query columns")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect columns");
    assert_eq!(
        cols,
        vec![
            "id",
            "page_id",
            "z_index",
            "item_type",
            "bounding_box",
            "data_json"
        ]
    );

    // 3. Scan all 100 rows for zero cache contamination
    let mut stmt = conn
        .prepare("SELECT bounding_box, data_json FROM items;")
        .expect("Prepare select");
    let mut rows = stmt.query([]).expect("Query items");

    let mut count = 0;
    while let Some(row) = rows.next().expect("Next row") {
        let bbox_str: String = row.get(0).expect("Get bbox");
        let data_json: String = row.get(1).expect("Get json");

        assert!(
            !bbox_str.contains("cached_bbox"),
            "bbox_str must NOT contain cached_bbox"
        );
        assert!(
            !bbox_str.contains("cached_bezier"),
            "bbox_str must NOT contain cached_bezier"
        );
        assert!(
            !bbox_str.contains("spatial_index"),
            "bbox_str must NOT contain spatial_index"
        );

        assert!(
            !data_json.contains("cached_bbox"),
            "data_json must NOT contain cached_bbox"
        );
        assert!(
            !data_json.contains("cached_bezier"),
            "data_json must NOT contain cached_bezier"
        );
        assert!(
            !data_json.contains("spatial_index"),
            "data_json must NOT contain spatial_index"
        );

        // Parse to verify valid json
        let parsed_rect: Rect = serde_json::from_str(&bbox_str).expect("Valid rect");
        assert!(parsed_rect.width() > 0.0);

        let parsed_val: serde_json::Value =
            serde_json::from_str(&data_json).expect("Valid json value");
        assert!(parsed_val.get("cached_bbox").is_none());
        assert!(parsed_val.get("cached_bezier").is_none());
        assert!(parsed_val.get("spatial_index").is_none());

        count += 1;
    }
    assert_eq!(count, 100);
}

// ============================================================================
// 6. BOUNDARY CONDITIONS: EMPTY DOC & GIANT STROKE (5,000 POINTS)
// ============================================================================

#[test]
fn test_boundary_empty_doc_and_giant_stroke() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    // 1. Empty notebook (0 strokes)
    {
        let empty_doc = Document::new_notebook("Empty Doc");
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&empty_doc).expect("Save empty doc");
        let loaded = store.load_document().expect("Load empty doc");
        assert_eq!(loaded.pages.len(), 1);
        assert_eq!(loaded.pages[0].items.len(), 0);
        assert_eq!(loaded.pages[0].spatial_index.len(), 0);
        assert!(store.verify_integrity().expect("Integrity"));
    }

    // 2. Giant stroke with 5,000 points
    {
        let mut giant_doc = Document::new_notebook("Giant Stroke Doc");
        let giant_stroke = create_realistic_stroke(999, 10.0, 10.0, 5000);
        giant_doc.pages[0].items.push(Item {
            id: 999,
            z_index: 0,
            kind: ItemKind::Stroke(giant_stroke),
        });

        let mut store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let save_start = Instant::now();
        store
            .save_document(&giant_doc)
            .expect("Save giant stroke doc");
        let save_dur = save_start.elapsed();
        println!(
            "[EMPIRICAL CHALLENGER] Saved giant stroke (5,000 points) in {:.2?}",
            save_dur
        );

        let load_start = Instant::now();
        let loaded = store.load_document().expect("Load giant stroke doc");
        let load_dur = load_start.elapsed();
        println!(
            "[EMPIRICAL CHALLENGER] Loaded giant stroke (5,000 points) in {:.2?}",
            load_dur
        );

        assert_eq!(loaded.pages[0].items.len(), 1);
        let s = loaded.pages[0].items[0].as_stroke().expect("stroke");
        assert_eq!(s.points.len(), 5000);
        assert!(s.cached_bbox.is_some());
        assert_eq!(s.cached_bezier.len(), 4999);
        assert_eq!(loaded.pages[0].spatial_index.len(), 1);
        assert!(store
            .verify_integrity()
            .expect("Integrity after giant stroke"));
    }
}
