//! Challenger M3-2: SQLite Database Schema Cleanliness & Backward Compatibility Suite
//!
//! Objectives:
//! 1. Empirical verification of SQLite schema cleanliness:
//!    - PRAGMA user_version == 2
//!    - PRAGMA application_id == 0x554E494E ("UNIN")
//!    - Strict table column definitions (no cached_bbox, cached_bezier, spatial_index columns)
//!    - Zero cache pollution in raw data_json payloads and raw database file bytes
//! 2. Backward compatibility verification:
//!    - Legacy UniNote 0.1 (Schema V1, no background_extra, no document_format)
//!    - Legacy UniNote 0.2 / 0.3 (Schema V2 without spatial index)
//!    - Legacy unversioned V0 database
//!    - Automatic in-memory cache rehydration (ensure_cache and PageSpatialIndex bulk load)
//! 3. Edge-case and degraded stroke handling:
//!    - 0-point, 1-point, 2-point, extreme coordinates, and Whiteboard format

use rusqlite::Connection;
use std::fs;
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, Item, ItemKind, PageBackground};
use uninote_storage::{DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

fn create_sample_stroke(
    id: u64,
    start_x: f64,
    start_y: f64,
    point_count: usize,
    mode: PipelineMode,
) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let step = i as f64 * 4.0;
        points.push(StrokePoint {
            x: start_x + step,
            y: start_y + step * 0.5 + (i as f64 * 0.2).sin() * 5.0,
            pressure: 0.3 + ((i % 7) as f64 * 0.1),
            width: 2.0 + ((i % 3) as f64 * 0.5),
            timestamp_ns: 10_000 + (i as u64 * 5_000),
        });
    }

    let is_dot = point_count == 1;
    Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples: Vec::new(),
        is_dot,
        mode,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ============================================================================
// 1. MASTER SCHEMA PURITY & USER VERSION VERIFICATION
// ============================================================================

#[test]
fn test_challenger_sqlite_schema_master_purity_and_user_version() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    // Create and initialize store
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        let doc = Document::new_notebook("Schema Master Purity Test");
        store.save_document(&doc).expect("Save initial document");
    }

    // Direct low-level SQLite inspection
    let conn = Connection::open(file_path).expect("Open raw SQLite connection");

    // 1. Check user_version and application_id
    let user_version: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(
        user_version, SCHEMA_VERSION,
        "PRAGMA user_version must be exactly SCHEMA_VERSION (3)"
    );
    assert_eq!(user_version, 3, "SCHEMA_VERSION must strictly equal 3");

    let app_id: u32 = conn
        .query_row("PRAGMA application_id;", [], |r| r.get(0))
        .expect("PRAGMA application_id");
    assert_eq!(
        app_id, APPLICATION_ID,
        "PRAGMA application_id must be 0x554E494E"
    );

    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
        .expect("PRAGMA journal_mode");
    assert_eq!(
        journal_mode.to_lowercase(),
        "wal",
        "SQLite journal_mode must be WAL"
    );

    // 2. Inspect sqlite_master / sqlite_schema
    let mut stmt = conn
        .prepare(
            "SELECT type, name, tbl_name, sql FROM sqlite_master WHERE type IN ('table', 'index', 'view', 'trigger') ORDER BY name ASC",
        )
        .expect("Query sqlite_master");

    let schema_objects: Vec<(String, String, String, Option<String>)> = stmt
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .expect("Map schema objects")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect schema objects");

    let mut tables = Vec::new();
    let mut indices = Vec::new();
    let mut views = Vec::new();
    let mut triggers = Vec::new();

    for (obj_type, name, _tbl, _sql) in &schema_objects {
        // Assert no schema object contains cache keywords
        assert!(
            !name.contains("cache"),
            "Schema object name '{name}' must not contain 'cache'"
        );
        assert!(
            !name.contains("spatial"),
            "Schema object name '{name}' must not contain 'spatial'"
        );
        assert!(
            !name.contains("bezier"),
            "Schema object name '{name}' must not contain 'bezier'"
        );

        match obj_type.as_str() {
            "table" => tables.push(name.clone()),
            "index" => indices.push(name.clone()),
            "view" => views.push(name.clone()),
            "trigger" => triggers.push(name.clone()),
            _ => {}
        }
    }

    // Tables must be exactly: items, metadata, pages (ignoring sqlite_sequence if any)
    tables.retain(|t| !t.starts_with("sqlite_"));
    let mut expected_tables = vec![
        "assets".to_string(),
        "items".to_string(),
        "metadata".to_string(),
        "pages".to_string(),
    ];
    expected_tables.sort();
    tables.sort();
    assert_eq!(
        tables, expected_tables,
        "Tables must be exactly assets, items, metadata, pages"
    );

    // Views and triggers must be empty
    assert!(views.is_empty(), "No views should exist in the schema");
    assert!(
        triggers.is_empty(),
        "No triggers should exist in the schema"
    );

    // Index on items(page_id) must exist
    assert!(
        indices.iter().any(|idx| idx == "idx_items_page"),
        "Index 'idx_items_page' must exist"
    );

    // 3. Inspect columns of 'items' table
    let mut items_info_stmt = conn
        .prepare("PRAGMA table_info(items);")
        .expect("PRAGMA table_info(items)");
    let items_cols: Vec<(i64, String, String, i64, Option<String>, i64)> = items_info_stmt
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .expect("Map items table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect items table_info");

    let items_col_names: Vec<String> = items_cols.iter().map(|c| c.1.clone()).collect();
    let expected_items_cols = vec![
        "id".to_string(),
        "page_id".to_string(),
        "z_index".to_string(),
        "item_type".to_string(),
        "bounding_box".to_string(),
        "data_json".to_string(),
    ];
    assert_eq!(
        items_col_names, expected_items_cols,
        "Columns in 'items' table must strictly match canonical schema without cache columns"
    );

    // 4. Inspect columns of 'pages' table
    let mut pages_info_stmt = conn
        .prepare("PRAGMA table_info(pages);")
        .expect("PRAGMA table_info(pages)");
    let pages_cols: Vec<String> = pages_info_stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("Map pages table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect pages table_info");

    let expected_pages_cols = vec![
        "id".to_string(),
        "page_index".to_string(),
        "width".to_string(),
        "height".to_string(),
        "background_type".to_string(),
        "background_spacing".to_string(),
        "background_extra".to_string(),
    ];
    assert_eq!(
        pages_cols, expected_pages_cols,
        "Columns in 'pages' table must strictly match canonical V2 schema"
    );

    // 5. Inspect columns of 'metadata' table
    let mut meta_info_stmt = conn
        .prepare("PRAGMA table_info(metadata);")
        .expect("PRAGMA table_info(metadata)");
    let meta_cols: Vec<String> = meta_info_stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("Map metadata table_info")
        .collect::<Result<Vec<_>, _>>()
        .expect("Collect metadata table_info");

    let expected_meta_cols = vec!["key".to_string(), "value".to_string()];
    assert_eq!(
        meta_cols, expected_meta_cols,
        "Columns in 'metadata' table must strictly match canonical schema"
    );
}

// ============================================================================
// 2. BYTE-LEVEL & JSON ZERO CACHE LEAKAGE (STRESS TEST)
// ============================================================================

#[test]
fn test_challenger_database_byte_level_and_json_zero_cache_leakage() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path().to_path_buf();

    // Create a document with 100 heterogeneous strokes with pre-warmed cache
    let mut doc = Document::new_notebook("Zero Cache Pollution Test");
    let page = &mut doc.pages[0];

    for i in 0..100 {
        let mode = match i % 3 {
            0 => PipelineMode::CatmullRomSpline,
            1 => PipelineMode::OneEuroFiltered,
            _ => PipelineMode::Baseline,
        };
        let pt_count = match i % 4 {
            0 => 1,  // Dot
            1 => 2,  // Segment
            2 => 15, // Medium curve
            _ => 60, // Long complex spline
        };

        let mut stroke =
            create_sample_stroke(i as u64, (i * 10) as f64, (i * 12) as f64, pt_count, mode);

        // Explicitly warm up the in-memory cache before saving
        stroke.ensure_cache();
        assert!(
            stroke.cached_bbox.is_some(),
            "Stroke {i} must have cached_bbox warmed up"
        );
        if pt_count >= 2 && mode != PipelineMode::Baseline {
            assert!(
                !stroke.cached_bezier.is_empty(),
                "Stroke {i} must have cached_bezier warmed up"
            );
        }

        page.items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // Populate spatial index in memory
    page.rebuild_spatial_index();
    assert_eq!(page.spatial_index.len(), 100);

    // Save document to SQLite
    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Open store");
        store.save_document(&doc).expect("Save document");
    }

    // A. Query raw SQLite rows
    let conn = Connection::open(&file_path).expect("Open raw SQLite connection");
    let mut stmt = conn
        .prepare("SELECT id, bounding_box, data_json FROM items ORDER BY id ASC")
        .expect("Prepare query");

    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let bbox_str: String = row.get(1)?;
            let data_json: String = row.get(2)?;
            Ok((id, bbox_str, data_json))
        })
        .expect("Query rows");

    let forbidden_keys = [
        "cached_bbox",
        "cached_bezier",
        "spatial_index",
        "spatial_item",
        "spatial",
    ];

    let mut inspected_count = 0;
    for row in rows {
        let (id, bbox_str, data_json) = row.expect("Row read");
        inspected_count += 1;

        // Verify bounding_box column
        for key in &forbidden_keys {
            assert!(
                !bbox_str.contains(key),
                "Item {id}: bounding_box column contains forbidden key '{key}': {bbox_str}"
            );
        }
        let parsed_bbox: Result<Rect, _> = serde_json::from_str(&bbox_str);
        assert!(
            parsed_bbox.is_ok(),
            "Item {id}: bounding_box must parse to Rect"
        );

        // Verify data_json column string content
        for key in &forbidden_keys {
            assert!(
                !data_json.contains(key),
                "Item {id}: data_json column contains forbidden key '{key}': {data_json}"
            );
        }

        // Verify data_json as parsed JSON Object map
        let val: serde_json::Value =
            serde_json::from_str(&data_json).expect("Parse data_json as serde_json::Value");
        let obj = val.as_object().expect("data_json must be a JSON object");

        // Allowed canonical Stroke fields
        let allowed_keys = ["id", "style", "points", "raw_samples", "is_dot", "mode"];
        for key in obj.keys() {
            assert!(
                allowed_keys.contains(&key.as_str()),
                "Item {id}: data_json contains unexpected field '{key}'"
            );
        }
        for key in &forbidden_keys {
            assert!(
                !obj.contains_key(*key),
                "Item {id}: data_json map explicitly contains forbidden key '{key}'"
            );
        }
    }
    assert_eq!(inspected_count, 100);

    // B. Direct raw binary inspection of the entire .uninote file on disk
    let raw_bytes = fs::read(&file_path).expect("Read raw SQLite file bytes");
    assert!(!raw_bytes.is_empty(), "SQLite file must not be empty");

    let forbidden_byte_patterns: Vec<&[u8]> = vec![
        b"cached_bbox",
        b"cached_bezier",
        b"spatial_index",
        b"spatial_item",
    ];

    for pattern in forbidden_byte_patterns {
        let pattern_str = std::str::from_utf8(pattern).unwrap();
        let found = raw_bytes
            .windows(pattern.len())
            .any(|window| window == pattern);
        assert!(
            !found,
            "Raw database file contains forbidden byte sequence '{pattern_str}'"
        );
    }
}

// ============================================================================
// 3. BACKWARD COMPATIBILITY: SYNTHETIC V1 MIGRATION & CACHE REHYDRATION
// ============================================================================

#[test]
fn test_challenger_backward_compatibility_v1_synthetic_to_v2_migration() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let legacy_doc_id = Uuid::new_v4();
    let legacy_page_id_1 = Uuid::new_v4();
    let legacy_page_id_2 = Uuid::new_v4();

    // 1. Manually craft a Schema V1 database (as produced by UniNote 0.1.0)
    // In V1: user_version = 1, pages table does NOT have background_extra, metadata has no document_format.
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite for V1 synthesis");
        conn.execute_batch(
            "PRAGMA application_id = 0x554E494E;
             PRAGMA user_version = 1;
             PRAGMA journal_mode = DELETE;

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
        .expect("Create raw V1 schema");

        // Insert metadata without document_format
        conn.execute(
            "INSERT INTO metadata (key, value) VALUES ('doc_id', ?1), ('title', 'Legacy 0.1 UniNote Notebook'), ('version', '1')",
            [legacy_doc_id.to_string()],
        )
        .expect("Insert V1 metadata");

        // Insert 2 pages
        conn.execute(
            "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
             VALUES (?1, 0, 595.28, 841.89, 'ruled', 22.68)",
            [legacy_page_id_1.to_string()],
        )
        .expect("Insert V1 page 1");

        conn.execute(
            "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
             VALUES (?1, 1, 595.28, 841.89, 'grid', 14.17)",
            [legacy_page_id_2.to_string()],
        )
        .expect("Insert V1 page 2");

        // Insert 10 legacy strokes across the two pages
        for i in 0..10 {
            let page_target = if i < 6 {
                legacy_page_id_1
            } else {
                legacy_page_id_2
            };
            let stroke = create_sample_stroke(
                i as u64,
                50.0 + (i as f64 * 20.0),
                80.0 + (i as f64 * 15.0),
                8,
                PipelineMode::CatmullRomSpline,
            );
            let stroke_json = serde_json::to_string(&stroke).unwrap();
            let bbox_json = serde_json::to_string(&stroke.compute_bounding_box()).unwrap();

            conn.execute(
                "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
                 VALUES (?1, ?2, ?3, 'stroke', ?4, ?5)",
                rusqlite::params![
                    i as i64,
                    page_target.to_string(),
                    i as i64,
                    bbox_json,
                    stroke_json
                ],
            )
            .expect("Insert legacy item");
        }
    }

    // 2. Open via DocumentStore. This MUST trigger seamless V1 -> V2 schema migration!
    let store = DocumentStore::open_or_create(file_path).expect("Open store on legacy V1 file");
    assert!(store.verify_integrity().expect("Integrity check on V1"));

    // Check PRAGMA user_version is now 2
    let migrated_user_version: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("Check migrated user_version");
    assert_eq!(
        migrated_user_version, 3,
        "Migrated database user_version must be 3"
    );

    // Verify background_extra column was dynamically added to pages table
    {
        let mut check_col_stmt = store
            .connection()
            .prepare("PRAGMA table_info(pages);")
            .expect("Prepare table_info(pages)");
        let pages_cols: Vec<String> = check_col_stmt
            .query_map([], |r| r.get::<_, String>(1))
            .expect("Query cols")
            .collect::<Result<Vec<_>, _>>()
            .expect("Collect cols");
        assert!(
            pages_cols.contains(&"background_extra".to_string()),
            "pages table must now contain 'background_extra'"
        );
    }

    // 3. Load the document and check complete cache rehydration
    let loaded_doc = store.load_document().expect("Load migrated document");
    assert_eq!(loaded_doc.id, legacy_doc_id);
    assert_eq!(loaded_doc.title, "Legacy 0.1 UniNote Notebook");
    assert_eq!(
        loaded_doc.format,
        DocumentFormat::Notebook,
        "Legacy document without document_format must default to Notebook"
    );
    assert_eq!(loaded_doc.pages.len(), 2);
    assert_eq!(loaded_doc.pages[0].items.len(), 6);
    assert_eq!(loaded_doc.pages[1].items.len(), 4);

    // Verify in-memory cache rehydration for every stroke on both pages
    for page in &loaded_doc.pages {
        for item in &page.items {
            let s = item.as_stroke().expect("stroke");
            assert!(
                s.cached_bbox.is_some(),
                "Stroke {} must have rehydrated cached_bbox",
                s.id
            );
            assert!(
                !s.cached_bezier.is_empty(),
                "Stroke {} must have rehydrated cached_bezier",
                s.id
            );
            assert_eq!(
                s.cached_bbox,
                s.compute_bounding_box(),
                "Rehydrated cached_bbox must match compute_bounding_box exactly"
            );
        }

        // Verify PageSpatialIndex was bulk-loaded
        assert_eq!(
            page.spatial_index.len(),
            page.items.len(),
            "PageSpatialIndex must contain all items for page {}",
            page.index
        );
        let all_spatial_items = page
            .spatial_index
            .locate_in_envelope(-1000.0, -1000.0, 5000.0, 5000.0);
        assert_eq!(
            all_spatial_items.len(),
            page.items.len(),
            "Spatial query over large envelope must return all page items"
        );
    }

    drop(store);

    // 4. Save modifications back and re-open to ensure roundtrip stability
    {
        let mut store =
            DocumentStore::open_or_create(file_path).expect("Reopen store for modification");
        let mut doc_to_save = loaded_doc;
        doc_to_save.title = "Updated V1->V2 Migrated Doc".to_string();

        let new_stroke = create_sample_stroke(999, 100.0, 100.0, 5, PipelineMode::CatmullRomSpline);
        doc_to_save.pages[0].items.push(Item {
            id: 999,
            z_index: 999,
            kind: ItemKind::Stroke(new_stroke),
        });

        store
            .save_document(&doc_to_save)
            .expect("Save updated document");
    }

    // Re-verify after resaving
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen resaved store");
        let reloaded = store.load_document().expect("Reload resaved doc");
        assert_eq!(reloaded.title, "Updated V1->V2 Migrated Doc");
        assert_eq!(reloaded.pages[0].items.len(), 7);
        assert_eq!(reloaded.pages[0].spatial_index.len(), 7);
    }
}

// ============================================================================
// 4. BACKWARD COMPATIBILITY: UNVERSIONED V0 DATABASE RECOVERY
// ============================================================================

#[test]
fn test_challenger_backward_compatibility_unversioned_v0_database() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    // Create an unversioned V0 database (user_version = 0, no background_extra, no application_id)
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite for V0");
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
                 page_id TEXT NOT NULL,
                 z_index INTEGER NOT NULL,
                 item_type TEXT NOT NULL,
                 bounding_box TEXT NOT NULL,
                 data_json TEXT NOT NULL
             );
             INSERT INTO metadata VALUES ('title', 'Raw V0 Database');
             INSERT INTO pages VALUES ('33333333-3333-3333-3333-333333333333', 0, 595.28, 841.89, 'blank', 0.0);",
        )
        .expect("Create V0 tables");
    }

    // Open via DocumentStore
    let store = DocumentStore::open_or_create(file_path).expect("Open V0 database");
    assert!(store.verify_integrity().expect("Integrity check on V0"));

    let user_version: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("Check user_version");
    assert_eq!(
        user_version, 3,
        "V0 database must be migrated to user_version 3"
    );

    let doc = store.load_document().expect("Load migrated V0 doc");
    assert_eq!(doc.title, "Raw V0 Database");
    assert_eq!(doc.format, DocumentFormat::Notebook);
    assert_eq!(doc.pages.len(), 1);
    assert!(doc.pages[0].items.is_empty());
}

// ============================================================================
// 5. DEGRADED STROKE VARIANTS & REHYDRATION EDGE CASES
// ============================================================================

#[test]
fn test_challenger_edge_cases_empty_and_degraded_strokes() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page = &mut doc.pages[0];

    // Stroke 1: 0 points (degenerate / corrupted stroke)
    let s0 = Stroke {
        id: 100,
        style: StrokeStyle::default(),
        points: Vec::new(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // Stroke 2: 1 point dot (single touch tap)
    let s1 = Stroke {
        id: 101,
        style: StrokeStyle::default(),
        points: vec![StrokePoint {
            x: 50.0,
            y: 75.0,
            pressure: 0.8,
            width: 3.0,
            timestamp_ns: 500,
        }],
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // Stroke 3: 2 points straight line
    let s2 = Stroke {
        id: 102,
        style: StrokeStyle::default(),
        points: vec![
            StrokePoint {
                x: 100.0,
                y: 100.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 600,
            },
            StrokePoint {
                x: 200.0,
                y: 200.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 700,
            },
        ],
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // Stroke 4: Extreme coordinates (Whiteboard scale: -100,000 to +100,000)
    let s3 = Stroke {
        id: 103,
        style: StrokeStyle::default(),
        points: vec![
            StrokePoint {
                x: -100_000.0,
                y: -100_000.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 800,
            },
            StrokePoint {
                x: 100_000.0,
                y: 100_000.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 900,
            },
        ],
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::OneEuroFiltered,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    page.items.push(Item {
        id: 100,
        z_index: 0,
        kind: ItemKind::Stroke(s0),
    });
    page.items.push(Item {
        id: 101,
        z_index: 1,
        kind: ItemKind::Stroke(s1),
    });
    page.items.push(Item {
        id: 102,
        z_index: 2,
        kind: ItemKind::Stroke(s2),
    });
    page.items.push(Item {
        id: 103,
        z_index: 3,
        kind: ItemKind::Stroke(s3),
    });

    // Save
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store
            .save_document(&doc)
            .expect("Save document with edge cases");
    }

    // Load back and verify rehydration does not panic or produce NaN
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load edge cases doc");
        assert_eq!(loaded.format, DocumentFormat::Whiteboard);
        assert_eq!(loaded.pages[0].items.len(), 4);

        // s0 (0 points)
        let stroke0 = loaded.pages[0].items[0].as_stroke().expect("stroke");
        assert_eq!(stroke0.points.len(), 0);
        assert!(stroke0.cached_bezier.is_empty());

        // s1 (1 point)
        let stroke1 = loaded.pages[0].items[1].as_stroke().expect("stroke");
        assert_eq!(stroke1.points.len(), 1);
        assert!(stroke1.cached_bbox.is_some());
        assert!(stroke1.cached_bezier.is_empty()); // No bezier segments for 1 point

        // s2 (2 points Baseline)
        let stroke2 = loaded.pages[0].items[2].as_stroke().expect("stroke");
        assert_eq!(stroke2.points.len(), 2);
        assert!(stroke2.cached_bbox.is_some());
        assert!(
            stroke2.cached_bezier.is_empty(),
            "Baseline mode must have empty cached_bezier"
        );

        // s3 (extreme coordinates OneEuroFiltered)
        let stroke3 = loaded.pages[0].items[3].as_stroke().expect("stroke");
        assert_eq!(stroke3.points.len(), 2);
        assert_eq!(
            stroke3.cached_bezier.len(),
            1,
            "OneEuroFiltered 2 points must produce 1 Bezier segment"
        );
        let bbox = stroke3.cached_bbox.expect("Must have cached_bbox");
        assert!(bbox.min_x <= -100_000.0);
        assert!(bbox.max_x >= 100_000.0);

        // Spatial query on extreme coordinates
        let hits = loaded.pages[0]
            .spatial_index
            .locate_in_envelope(-200_000.0, -200_000.0, 200_000.0, 200_000.0);
        assert!(
            hits.iter().any(|h| h.item_id == 103),
            "Must locate extreme item in large envelope"
        );
    }
}

// ============================================================================
// 6. MULTIPLE RE-SAVE CYCLES IDEMPOTENCE & ZERO DRIFT
// ============================================================================

#[test]
fn test_challenger_multiple_save_cycles_idempotence_and_zero_drift() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_notebook("Idempotence Test");
    for i in 0..20 {
        let stroke = create_sample_stroke(
            i as u64,
            10.0 * i as f64,
            20.0 * i as f64,
            5,
            PipelineMode::CatmullRomSpline,
        );
        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // 15 repeated open-save-load cycles
    for cycle in 0..15 {
        let mut store = DocumentStore::open_or_create(file_path)
            .unwrap_or_else(|e| panic!("Cycle {cycle} open failed: {e}"));

        // Mutate title and 1 stroke
        doc.title = format!("Idempotence Test Cycle {cycle}");
        let s = doc.pages[0].items[0].as_stroke_mut().expect("stroke");
        s.points[0].x += 1.0;
        s.ensure_cache();

        store
            .save_document(&doc)
            .unwrap_or_else(|e| panic!("Cycle {cycle} save failed: {e}"));

        let loaded = store
            .load_document()
            .unwrap_or_else(|e| panic!("Cycle {cycle} load failed: {e}"));

        assert_eq!(loaded.title, format!("Idempotence Test Cycle {cycle}"));
        assert_eq!(loaded.pages[0].items.len(), 20);

        // Check user_version remains 3
        let uver: u32 = store
            .connection()
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .expect("PRAGMA user_version");
        assert_eq!(
            uver, 3,
            "Cycle {cycle}: user_version must remain strictly 3"
        );

        assert!(
            store.verify_integrity().expect("Integrity check"),
            "Cycle {cycle}: database integrity check failed"
        );
    }
}

// ============================================================================
// 7. BACKWARD COMPATIBILITY: EXACT 0.2 & 0.3 DATABASE REHYDRATION
// ============================================================================

#[test]
fn test_challenger_backward_compatibility_v2_synthetic_0_2_and_0_3_documents() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path();

    let doc_id = Uuid::new_v4();
    let page_id_pdf = Uuid::new_v4();
    let page_id_wb = Uuid::new_v4();

    // 1. Manually synthesize a V2 database as generated by UniNote 0.2.0 / 0.3.0
    // In V2: user_version = 2, background_extra exists, document_format in metadata.
    // However, data_json does not have any cached_bbox or cached_bezier fields.
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite for 0.2/0.3");
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
        .expect("Create raw V2 schema");

        conn.execute(
            "INSERT INTO metadata VALUES ('doc_id', ?1), ('title', 'UniNote 0.2/0.3 Document'), ('version', '2'), ('document_format', 'whiteboard')",
            [doc_id.to_string()],
        )
        .expect("Insert V2 metadata");

        // Page 1: PDF page background
        let pdf_meta_json = r#"{"source_path":"/tmp/test_lecture.pdf","page_index":0,"original_width":595.28,"original_height":841.89}"#;
        conn.execute(
            "INSERT INTO pages VALUES (?1, 0, 595.28, 841.89, 'pdf_page', 0.0, ?2)",
            rusqlite::params![page_id_pdf.to_string(), pdf_meta_json],
        )
        .expect("Insert PDF page");

        // Page 2: Whiteboard page with infinite bounds
        conn.execute(
            "INSERT INTO pages VALUES (?1, 1, 1e308, 1e308, 'blank', 0.0, '')",
            rusqlite::params![page_id_wb.to_string()],
        )
        .expect("Insert Whiteboard page");

        // Insert strokes serialized in UniNote 0.2/0.3 format (without cached fields)
        for i in 0..30 {
            let page_target = if i < 15 { page_id_pdf } else { page_id_wb };
            let raw_json = format!(
                r#"{{"id":{},"style":{{"color":[0.1,0.2,0.8,1.0],"base_width":2.5,"min_width_factor":0.4,"max_width_factor":1.8}},"points":[{{"x":{},"y":{},"pressure":0.5,"width":2.5,"timestamp_ns":1000}},{{"x":{},"y":{},"pressure":0.6,"width":2.5,"timestamp_ns":2000}},{{"x":{},"y":{},"pressure":0.7,"width":2.5,"timestamp_ns":3000}}],"raw_samples":[],"is_dot":false,"mode":"CatmullRomSpline"}}"#,
                i,
                (i * 10) as f64,
                (i * 10) as f64,
                (i * 10 + 20) as f64,
                (i * 10 + 20) as f64,
                (i * 10 + 40) as f64,
                (i * 10 + 40) as f64,
            );
            let bbox_json = format!(
                r#"{{"min_x":{},"min_y":{},"max_x":{},"max_y":{}}}"#,
                (i * 10) as f64,
                (i * 10) as f64,
                (i * 10 + 40) as f64,
                (i * 10 + 40) as f64,
            );

            conn.execute(
                "INSERT INTO items VALUES (?1, ?2, ?3, 'stroke', ?4, ?5)",
                rusqlite::params![i, page_target.to_string(), i, bbox_json, raw_json],
            )
            .expect("Insert 0.2/0.3 item");
        }
    }

    // 2. Open via DocumentStore in UniNote 0.4.0
    let store = DocumentStore::open_or_create(file_path).expect("Open 0.2/0.3 document");
    assert!(store.verify_integrity().expect("Integrity check"));

    let doc = store.load_document().expect("Load 0.2/0.3 document");
    assert_eq!(doc.id, doc_id);
    assert_eq!(doc.title, "UniNote 0.2/0.3 Document");
    assert_eq!(doc.format, DocumentFormat::Whiteboard);
    assert_eq!(doc.pages.len(), 2);

    // Verify PDF background on page 0
    match &doc.pages[0].background {
        PageBackground::PdfPage {
            source_path,
            page_index,
            original_width,
            original_height,
        } => {
            assert_eq!(source_path, "/tmp/test_lecture.pdf");
            assert_eq!(*page_index, 0);
            assert_eq!(*original_width, 595.28);
            assert_eq!(*original_height, 841.89);
        }
        other => panic!("Expected PdfPage background, got {other:?}"),
    }

    // Verify cache rehydration for all 30 strokes
    for page in &doc.pages {
        assert_eq!(page.items.len(), 15);
        assert_eq!(
            page.spatial_index.len(),
            15,
            "Spatial index must be automatically rebuilt with 15 items"
        );

        for item in &page.items {
            let s = item.as_stroke().expect("stroke");
            assert!(
                s.cached_bbox.is_some(),
                "Stroke {} must have rehydrated cached_bbox",
                s.id
            );
            assert_eq!(
                s.cached_bezier.len(),
                2,
                "Stroke {} (3 points) must have exactly 2 rehydrated Bezier segments",
                s.id
            );
        }
    }

    // 3. Re-save document and ensure zero schema drift
    drop(store);
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        store.save_document(&doc).expect("Save document");
    }

    let conn = Connection::open(file_path).expect("Open raw sqlite");
    let uver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(
        uver, 3,
        "user_version must remain 3 after re-saving 0.2/0.3 doc"
    );
}

// ============================================================================
// 8. LARGE-SCALE BINARY SCAN (1,000 STROKES RAW DISK SCAN)
// ============================================================================

#[test]
fn test_challenger_raw_database_hex_and_utf8_scan_zero_leakage_on_1k_strokes() {
    let temp_file = NamedTempFile::new().expect("Create temp file");
    let file_path = temp_file.path().to_path_buf();

    let mut doc = Document::new_notebook("1k Strokes Disk Scan");
    for i in 0..1_000 {
        let mut stroke = create_sample_stroke(
            i as u64,
            (i % 50) as f64 * 10.0,
            (i / 50) as f64 * 15.0,
            8,
            PipelineMode::CatmullRomSpline,
        );
        stroke.ensure_cache();
        doc.pages[0].items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }
    doc.pages[0].rebuild_spatial_index();

    {
        let mut store = DocumentStore::open_or_create(&file_path).expect("Open store");
        store.save_document(&doc).expect("Save 1k strokes");
    }

    // Read full raw database file from disk
    let file_bytes = fs::read(&file_path).expect("Read database file bytes");

    let forbidden_literals = [
        "cached_bbox",
        "cached_bezier",
        "spatial_index",
        "spatial_item",
        "PageSpatialIndex",
    ];

    for literal in forbidden_literals {
        let needle = literal.as_bytes();
        let found = file_bytes.windows(needle.len()).any(|w| w == needle);
        assert!(
            !found,
            "CRITICAL BUG: Forbidden literal '{literal}' detected in SQLite database file bytes!"
        );
    }
}
