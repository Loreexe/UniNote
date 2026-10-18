//! Milestone M3: SQLite WAL Persistence Optimization Verification Suite
//!
//! Authoritative references:
//! - DISPATCH.md (Worker M3 objective: TransactionBehavior::Immediate, prepare_cached, buffer reuse,
//!   COUNT(*) pre-allocation in load_document, ensure_cache rehydration, spatial_index bulk load,
//!   backward compatibility verification, and stress benchmarks).
//! - PROJECT.md (Features 10, 11, 12: High-throughput SQLite WAL persistence, 0 DB pollution, Schema V1/V2 integrity).
//! - .uninote-wiki/RULES.md (Code quality, SemVer, integrity).

use rusqlite::Connection;
use std::time::Instant;
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, Item, ItemKind, PageBackground};
use uninote_storage::{DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

fn create_test_stroke(id: u64, x: f64, y: f64, point_count: usize) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let offset = i as f64 * 3.5;
        points.push(StrokePoint {
            x: x + offset,
            y: y + offset * 0.8,
            pressure: 0.5 + ((i % 5) as f64 * 0.08),
            width: 2.5,
            timestamp_ns: 1_000_000 + (i as u64 * 8_000),
        });
    }

    Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples: Vec::new(),
        is_dot: point_count == 1,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ============================================================================
// 1. RAPID WAL SAVE & LOAD ON 10,000 STROKES (PERFORMANCE & INTEGRITY)
// ============================================================================

#[test]
fn test_m3_wal_rapid_save_and_load_10_000_strokes() {
    let temp_file = NamedTempFile::new().expect("Temp file creation failed");
    let file_path = temp_file.path();

    let total_strokes = 10_000;
    let mut doc = Document::new_notebook("M3 High Volume Benchmark");
    doc.pages[0].background = PageBackground::Grid { grid_size: 14.17 };

    for i in 0..total_strokes {
        let x = (i % 80) as f64 * 6.0 + 40.0;
        let y = (i / 80) as f64 * 6.0 + 50.0;
        let stroke = create_test_stroke(i as u64, x, y, 6);
        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // Measure Save Duration
    let save_start = Instant::now();
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store
            .save_document(&doc)
            .expect("Save 10,000 strokes document");
    }
    let save_duration = save_start.elapsed();
    println!(
        "M3 Benchmark: Saved 10,000 strokes in {:.2?}",
        save_duration
    );

    // Release mode target: < 70 ms (target < 20 ms). Debug mode upper bound: < 1500 ms.
    #[cfg(not(debug_assertions))]
    assert!(
        save_duration.as_millis() < 70,
        "Save 10k strokes in release mode took {} ms (expected < 70 ms)",
        save_duration.as_millis()
    );

    // Measure Load Duration
    let load_start = Instant::now();
    let loaded = {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        assert!(store.verify_integrity().expect("Integrity verification"));
        store.load_document().expect("Load 10,000 strokes")
    };
    let load_duration = load_start.elapsed();
    println!(
        "M3 Benchmark: Loaded 10,000 strokes in {:.2?}",
        load_duration
    );

    #[cfg(not(debug_assertions))]
    assert!(
        load_duration.as_millis() < 40,
        "Load 10k strokes in release mode took {} ms (expected < 40 ms)",
        load_duration.as_millis()
    );

    assert_eq!(loaded.title, "M3 High Volume Benchmark");
    assert_eq!(loaded.format, DocumentFormat::Notebook);
    assert_eq!(loaded.pages.len(), 1);
    assert_eq!(loaded.pages[0].items.len(), total_strokes);

    // Sample boundary and intermediate items
    let check_indices = [0, 100, 2500, 5000, 7500, 9999];
    for &idx in &check_indices {
        let orig = &doc.pages[0].items[idx];
        let reloaded = &loaded.pages[0].items[idx];
        assert_eq!(orig.id, reloaded.id);
        assert_eq!(orig.z_index, reloaded.z_index);

        let ItemKind::Stroke(ref orig_s) = orig.kind;
        let ItemKind::Stroke(ref reloaded_s) = reloaded.kind;
        assert_eq!(orig_s.id, reloaded_s.id);
        assert_eq!(orig_s.points.len(), reloaded_s.points.len());
        assert_eq!(orig_s.mode, reloaded_s.mode);
    }
}

// ============================================================================
// 2. CACHE REHYDRATION (ensure_cache & PageSpatialIndex) ON LOAD
// ============================================================================

#[test]
fn test_m3_cache_rehydration_ensure_cache_and_spatial_index_on_load() {
    let temp_file = NamedTempFile::new().expect("Temp file creation failed");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    for i in 0..500 {
        let x = (i % 20) as f64 * 50.0;
        let y = (i / 20) as f64 * 50.0;
        let stroke = create_test_stroke(i as u64, x, y, 8);
        // Initially cold
        assert!(stroke.cached_bbox.is_none());
        assert!(stroke.cached_bezier.is_empty());

        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save doc");
    }

    // Load back and inspect cache rehydration
    let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
    let loaded = store.load_document().expect("Load doc");

    let page = &loaded.pages[0];
    assert_eq!(page.items.len(), 500);

    // Verify all strokes have rehydrated cached_bbox and cached_bezier
    for item in &page.items {
        let ItemKind::Stroke(ref stroke) = item.kind;
        assert!(
            stroke.cached_bbox.is_some(),
            "Stroke {} must have cached_bbox rehydrated via ensure_cache()",
            stroke.id
        );
        assert!(
            !stroke.cached_bezier.is_empty(),
            "Stroke {} must have cached_bezier rehydrated via ensure_cache()",
            stroke.id
        );
        assert_eq!(
            stroke.bounding_box(),
            stroke.compute_bounding_box(),
            "Cached bbox must equal computed bbox exactly"
        );
    }

    // Verify spatial index was bulk-loaded
    assert_eq!(
        page.spatial_index.len(),
        500,
        "PageSpatialIndex must contain all 500 items"
    );

    // Spatial query on a known subregion
    let query_items = page
        .spatial_index
        .locate_in_envelope(0.0, 0.0, 120.0, 120.0);
    assert!(
        !query_items.is_empty(),
        "Must find items in [0..120, 0..120]"
    );
    // Results must be sorted by z_index
    for w in query_items.windows(2) {
        assert!(
            w[0].z_index <= w[1].z_index,
            "Spatial query results must be sorted by ascending z_index"
        );
    }
}

// ============================================================================
// 3. DATABASE CLEANLINESS & ZERO CACHE POLLUTION
// ============================================================================

#[test]
fn test_m3_db_cleanliness_no_cache_pollution_and_schema_version_2() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_notebook("Cleanliness Test");
    let mut stroke = create_test_stroke(42, 100.0, 200.0, 12);
    // Warm up cache in memory before saving
    stroke.ensure_cache();
    assert!(stroke.cached_bbox.is_some());
    assert!(!stroke.cached_bezier.is_empty());

    doc.pages[0].items.push(Item {
        id: 42,
        z_index: 0,
        kind: ItemKind::Stroke(stroke),
    });
    doc.pages[0].rebuild_spatial_index();
    assert_eq!(doc.pages[0].spatial_index.len(), 1);

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save doc");
    }

    // Directly open raw SQLite and inspect table structure and row payloads
    let conn = Connection::open(file_path).expect("Open raw sqlite");

    // 1. Verify schema pragma contracts
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_version, SCHEMA_VERSION, "SCHEMA_VERSION must remain 2");

    let app_id: u32 = conn
        .query_row("PRAGMA application_id;", [], |r| r.get(0))
        .expect("PRAGMA application_id");
    assert_eq!(app_id, APPLICATION_ID);

    // 2. Verify column definitions of items table
    let mut pragma_stmt = conn
        .prepare("PRAGMA table_info(items);")
        .expect("PRAGMA table_info");
    let col_names: Vec<String> = pragma_stmt
        .query_map([], |row| row.get::<_, String>(1))
        .expect("Query col names")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect col names");

    let expected_cols = [
        "id",
        "page_id",
        "z_index",
        "item_type",
        "bounding_box",
        "data_json",
    ];
    assert_eq!(
        col_names, expected_cols,
        "Table items must have exactly the canonical columns, with no cache fields"
    );

    // 3. Inspect raw data_json and bounding_box
    let (bbox_str, data_json): (String, String) = conn
        .query_row(
            "SELECT bounding_box, data_json FROM items WHERE id = 42;",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("Query item 42");

    // bounding_box column is a clean Rect JSON without cache keywords
    assert!(!bbox_str.contains("cached_bbox"));
    assert!(!bbox_str.contains("cached_bezier"));
    assert!(!bbox_str.contains("spatial_index"));
    let parsed_bbox: Rect = serde_json::from_str(&bbox_str).expect("Parse bbox Rect");
    assert!(parsed_bbox.width() > 0.0);
    assert!(parsed_bbox.height() > 0.0);

    // data_json is clean Stroke JSON without cache fields (strictly skipped via serde)
    assert!(
        !data_json.contains("cached_bbox"),
        "Raw data_json must NOT contain 'cached_bbox'"
    );
    assert!(
        !data_json.contains("cached_bezier"),
        "Raw data_json must NOT contain 'cached_bezier'"
    );
    assert!(
        !data_json.contains("spatial_index"),
        "Raw data_json must NOT contain 'spatial_index'"
    );

    let json_val: serde_json::Value =
        serde_json::from_str(&data_json).expect("Parse data_json Value");
    assert!(json_val.get("cached_bbox").is_none());
    assert!(json_val.get("cached_bezier").is_none());
    assert!(json_val.get("spatial_index").is_none());
}

// ============================================================================
// 4. BACKWARD COMPATIBILITY: SCHEMA V1 & V2 UNMODIFIED ROUND-TRIP
// ============================================================================

#[test]
fn test_m3_backward_compatibility_v1_and_v2_unmodified_schema() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // 1. Manually synthesize a pre-0.2.0 Schema V1 database
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite");
        conn.execute_batch(
            "CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE pages (id TEXT PRIMARY KEY, page_index INTEGER NOT NULL, width REAL NOT NULL, height REAL NOT NULL, background_type TEXT NOT NULL, background_spacing REAL NOT NULL);
             CREATE TABLE items (id INTEGER PRIMARY KEY, page_id TEXT NOT NULL, z_index INTEGER NOT NULL, item_type TEXT NOT NULL, bounding_box TEXT NOT NULL, data_json TEXT NOT NULL);
             INSERT INTO metadata VALUES ('doc_id', '11111111-1111-1111-1111-111111111111');
             INSERT INTO metadata VALUES ('title', 'Legacy V1 Test Notebook');
             INSERT INTO metadata VALUES ('version', '1');
             INSERT INTO pages VALUES ('22222222-2222-2222-2222-222222222222', 0, 595.28, 841.89, 'ruled', 22.68);
             INSERT INTO items VALUES (1, '22222222-2222-2222-2222-222222222222', 0, 'stroke',
                '{\"min_x\":10.0,\"min_y\":20.0,\"max_x\":50.0,\"max_y\":60.0}',
                '{\"id\":1,\"style\":{\"color\":[0.1,0.1,0.12,1.0],\"base_width\":2.8,\"min_width_factor\":0.4,\"max_width_factor\":1.8},\"points\":[{\"x\":10.0,\"y\":20.0,\"pressure\":0.5,\"width\":2.5,\"timestamp_ns\":100},{\"x\":30.0,\"y\":40.0,\"pressure\":0.5,\"width\":2.5,\"timestamp_ns\":200},{\"x\":50.0,\"y\":60.0,\"pressure\":0.5,\"width\":2.5,\"timestamp_ns\":300}],\"raw_samples\":[],\"is_dot\":false,\"mode\":\"CatmullRomSpline\"}'
             );"
        ).expect("Batch insert legacy V1 schema");
    }

    // 2. Open via DocumentStore and load document
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open legacy store");
        assert!(store.verify_integrity().expect("Integrity check"));

        let loaded = store.load_document().expect("Load legacy document");
        assert_eq!(loaded.title, "Legacy V1 Test Notebook");
        assert_eq!(loaded.format, DocumentFormat::Notebook);
        assert_eq!(loaded.pages.len(), 1);
        assert_eq!(loaded.pages[0].items.len(), 1);

        let ItemKind::Stroke(ref s) = loaded.pages[0].items[0].kind;
        assert_eq!(s.id, 1);
        assert_eq!(s.points.len(), 3);
        // Automatic cache rehydration on load
        assert!(s.cached_bbox.is_some());
        assert_eq!(s.cached_bezier.len(), 2);
        assert_eq!(loaded.pages[0].spatial_index.len(), 1);

        // Modify and save back
        let mut modified = loaded.clone();
        modified.title = "Migrated and Saved V2 Doc".to_string();
        let stroke2 = create_test_stroke(2, 100.0, 100.0, 5);
        modified.pages[0].items.push(Item {
            id: 2,
            z_index: 1,
            kind: ItemKind::Stroke(stroke2),
        });

        store.save_document(&modified).expect("Save modified doc");
    }

    // 3. Re-open and verify round-trip persistence fidelity
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let reloaded = store.load_document().expect("Reload doc");
        assert_eq!(reloaded.title, "Migrated and Saved V2 Doc");
        assert_eq!(reloaded.pages[0].items.len(), 2);
        assert_eq!(reloaded.pages[0].spatial_index.len(), 2);
    }
}

// ============================================================================
// 5. RAPID CYCLING TRANSACTION STRESS TEST
// ============================================================================

#[test]
fn test_m3_rapid_cycling_transaction_stress() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let mut store = DocumentStore::open_or_create(file_path).expect("Open store");

    // Perform 30 rapid successive save and load cycles
    for cycle in 0..30 {
        let stroke = create_test_stroke(cycle, cycle as f64 * 10.0, cycle as f64 * 15.0, 4);
        doc.pages[0].items.push(Item {
            id: cycle,
            z_index: cycle,
            kind: ItemKind::Stroke(stroke),
        });

        store
            .save_document(&doc)
            .unwrap_or_else(|e| panic!("Save failed on cycle {cycle}: {e}"));

        let loaded = store
            .load_document()
            .unwrap_or_else(|e| panic!("Load failed on cycle {cycle}: {e}"));
        assert_eq!(loaded.pages[0].items.len(), (cycle as usize) + 1);
        assert_eq!(loaded.pages[0].spatial_index.len(), (cycle as usize) + 1);
    }

    assert!(store
        .verify_integrity()
        .expect("Final integrity verification"));
}
