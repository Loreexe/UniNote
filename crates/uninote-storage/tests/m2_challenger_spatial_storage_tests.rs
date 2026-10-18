//! Adversarial empirical challenge test suite for Milestone M2 (Spatial Index Persistence & SQLite Isolation).
//!
//! Verifies:
//! 1. Serde skip isolation: `spatial_index` is strictly absent from raw SQLite tables (`pages`, `items`, `data_json`).
//! 2. Multi-page document SQLite WAL roundtrip with automatic rehydration via `page.rebuild_spatial_index()`.
//! 3. 100-page heterogeneous document SQLite persistence stress test.

use rusqlite::Connection;
use std::time::Instant;
use tempfile::tempdir;
use uninote_core::model::{Document, Item, ItemKind, Page, PageBackground};
use uninote_storage::DocumentStore;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn make_test_stroke(id: u64, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Stroke {
    let p0 = StrokePoint {
        x: min_x,
        y: min_y,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1000,
    };
    let p1 = StrokePoint {
        x: max_x,
        y: max_y,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 2000,
    };
    let mut stroke = Stroke {
        id,
        points: vec![p0, p1],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

#[test]
fn test_sqlite_raw_storage_strictly_omits_spatial_index() {
    let dir = tempdir().expect("tempdir creation");
    let db_path = dir.path().join("spatial_isolation_test.uninote");

    let mut doc = Document::new_notebook("Spatial Serde Isolation Doc");
    for i in 0..15 {
        let stroke = make_test_stroke(
            i as u64 + 1,
            i as f64 * 20.0,
            i as f64 * 20.0,
            i as f64 * 20.0 + 30.0,
            i as f64 * 20.0 + 20.0,
        );
        doc.pages[0].items.push(Item {
            id: i as u64 + 1,
            z_index: i as u64 + 1,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // Warm spatial index
    doc.pages[0].rebuild_spatial_index();
    assert_eq!(doc.pages[0].spatial_index.len(), 15);

    // Save document to SQLite
    let mut store = DocumentStore::open_or_create(&db_path).expect("DocumentStore::open_or_create");
    store.save_document(&doc).expect("save_document");

    // Open raw connection to inspect SQLite tables
    let raw_conn = Connection::open(&db_path).expect("raw rusqlite connection");

    // 1. Inspect table 'pages' schema: confirm no spatial_index column exists
    let mut stmt = raw_conn
        .prepare("PRAGMA table_info(pages)")
        .expect("pragma table_info");
    let mut page_cols = Vec::new();
    let mut rows = stmt.query([]).expect("query pragma");
    while let Some(row) = rows.next().expect("next row") {
        let col_name: String = row.get(1).expect("col name");
        page_cols.push(col_name);
    }
    assert!(
        !page_cols.contains(&"spatial_index".to_string()),
        "Table 'pages' contains forbidden 'spatial_index' column: {:?}",
        page_cols
    );
    assert!(
        !page_cols.contains(&"tree".to_string()),
        "Table 'pages' contains forbidden 'tree' column: {:?}",
        page_cols
    );

    // 2. Inspect table 'items': check data_json column
    let mut item_stmt = raw_conn
        .prepare("SELECT id, bounding_box, data_json FROM items")
        .expect("prepare select");
    let mut item_rows = item_stmt.query([]).expect("query items");

    let mut count = 0;
    while let Some(row) = item_rows.next().expect("next item row") {
        let _id: i64 = row.get(0).expect("id");
        let bbox: String = row.get(1).expect("bounding_box");
        let data_json: String = row.get(2).expect("data_json");

        assert!(
            !data_json.contains("spatial_index"),
            "Item data_json contaminated with 'spatial_index': {}",
            data_json
        );
        assert!(
            !data_json.contains("\"tree\""),
            "Item data_json contaminated with 'tree': {}",
            data_json
        );
        assert!(
            bbox.contains("min_x") && bbox.contains("max_x"),
            "Bounding box column does not contain valid Rect JSON: {}",
            bbox
        );
        count += 1;
    }
    assert_eq!(count, 15);

    // 3. Load document back through DocumentStore
    let loaded_doc = store.load_document().expect("load_document");
    assert_eq!(loaded_doc.pages.len(), 1);
    assert_eq!(loaded_doc.pages[0].items.len(), 15);

    // DocumentStore::load_document invokes page.rebuild_spatial_index(), so it must be fully rehydrated
    assert_eq!(loaded_doc.pages[0].spatial_index.len(), 15);

    // Verify querying rehydrated index
    let matches = loaded_doc.pages[0]
        .spatial_index
        .locate_in_envelope(-10.0, -10.0, 500.0, 500.0);
    assert_eq!(matches.len(), 15);
}

#[test]
fn test_sqlite_100_page_heterogeneous_document_persistence_and_rehydration() {
    let dir = tempdir().expect("tempdir creation");
    let db_path = dir.path().join("heterogeneous_100_pages.uninote");

    let mut doc = Document::new_notebook("100-Page SQLite Stress Test");

    // Page 0
    doc.pages[0].background = PageBackground::Blank;

    // Create 99 more pages (total 100 pages)
    for i in 1..100 {
        if i < 50 {
            doc.add_page(PageBackground::Ruled {
                line_spacing: 22.68,
                margin_left: 40.0,
            });
        } else if i < 80 {
            doc.pages.push(Page::new_whiteboard(PageBackground::Grid {
                grid_size: 14.17,
            }));
        } else {
            doc.pages.push(Page::new_pdf_page(
                i,
                format!("/home/cachylore/docs/lecture_{}.pdf", i),
                i % 5,
                800.0,
                600.0,
            ));
        }
    }

    let mut global_item_id = 1u64;
    // Populate each page with 10 strokes (total 1,000 strokes)
    for page in &mut doc.pages {
        for s in 0..10 {
            let x = (s * 30) as f64;
            let y = (s * 20) as f64;
            let stroke = make_test_stroke(global_item_id, x, y, x + 25.0, y + 15.0);
            page.items.push(Item {
                id: global_item_id,
                z_index: s as u64 + 1,
                kind: ItemKind::Stroke(stroke),
            });
            global_item_id += 1;
        }
        page.rebuild_spatial_index();
        assert_eq!(page.spatial_index.len(), 10);
    }

    // Save 100-page document
    let start_save = Instant::now();
    let mut store = DocumentStore::open_or_create(&db_path).expect("DocumentStore::open_or_create");
    store.save_document(&doc).expect("save_document");
    let save_duration = start_save.elapsed();
    println!(
        "Saved 100-page document with 1,000 strokes in {:?}",
        save_duration
    );

    // Load 100-page document back
    let start_load = Instant::now();
    let loaded_doc = store.load_document().expect("load_document");
    let load_duration = start_load.elapsed();
    println!(
        "Loaded 100-page document and rehydrated 100 spatial indices in {:?}",
        load_duration
    );

    assert_eq!(loaded_doc.pages.len(), 100);
    for (idx, page) in loaded_doc.pages.iter().enumerate() {
        assert_eq!(page.items.len(), 10, "Page {} item count mismatch", idx);
        assert_eq!(
            page.spatial_index.len(),
            10,
            "Page {} rehydrated spatial_index length mismatch",
            idx
        );

        // Test spatial index query on loaded page
        let q = page
            .spatial_index
            .locate_in_envelope(-5.0, -5.0, 500.0, 500.0);
        assert_eq!(q.len(), 10, "Page {} query result count mismatch", idx);
    }
}
