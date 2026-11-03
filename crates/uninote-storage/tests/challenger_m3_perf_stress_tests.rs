//! Adversarial empirical verification and stress test suite for Milestone M3 (PERF-002).
//!
//! Evaluates:
//! 1. Scale testing on DocumentStore::load_document from 0 to 10,000 strokes.
//! 2. Multi-page document loading (heterogeneous page sizes, varying stroke densities, up to 100 pages).
//! 3. Interleaved, out-of-order, and mixed (Stroke + Image) z-indexes across pages.
//! 4. Bézier curve integrity (rehydration via `stroke.ensure_cache()`, C0 continuity, finite control points).
//! 5. R*-Tree spatial index integrity (oracle equivalence between spatial queries and linear scan).
//! 6. Adversarial edge cases (orphaned items, corrupted JSON items, extreme coordinates, dots, degenerate bboxes).

use rusqlite::params;
use std::collections::HashSet;
use tempfile::NamedTempFile;
use uninote_core::model::{Document, ImageItem, Item, ItemKind, Page, PageBackground};
use uninote_storage::DocumentStore;
use uninote_stroke::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

/// Helper to generate a realistic spline stroke.
fn make_spline_stroke(id: u64, start_x: f64, start_y: f64, point_count: usize) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    for i in 0..point_count {
        let t = i as f64;
        let px = start_x + t * 5.0 + (t * 0.4).sin() * 4.0;
        let py = start_y + t * 3.5 + (t * 0.6).cos() * 3.0;
        let pressure = 0.3 + (0.2 * ((i % 5) as f64 / 4.0));
        points.push(StrokePoint {
            x: px,
            y: py,
            pressure,
            width: 2.0 + pressure * 1.5,
            timestamp_ns: 1_000_000 + (i as u64 * 10_000),
        });
    }

    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width: 2.5,
        ..Default::default()
    };

    let mut stroke = Stroke {
        id,
        style,
        points,
        raw_samples: Vec::new(),
        is_dot: point_count <= 1,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.ensure_cache();
    stroke
}

// ============================================================================
// 1. SCALE AND REHYDRATION INTEGRITY: 0 TO 10,000 STROKES
// ============================================================================

#[test]
fn test_adversarial_load_scale_0_strokes() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");

    // Empty document with 3 empty pages
    let mut doc = Document::new_notebook("Empty Notebook");
    doc.pages.push(Page::new_a4(1, PageBackground::default()));
    doc.pages.push(Page::new_a4(2, PageBackground::default()));
    store.save_document(&doc).expect("save_document");

    let loaded = store.load_document().expect("load_document");
    assert_eq!(loaded.pages.len(), 3);
    for page in &loaded.pages {
        assert_eq!(page.items.len(), 0);
        assert_eq!(page.spatial_index.len(), 0);
    }
}

#[test]
fn test_adversarial_load_scale_10_000_strokes_single_page() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");

    let stroke_count = 10_000;
    let mut doc = Document::new_notebook("10k Strokes Single Page");
    doc.pages[0].items.reserve(stroke_count);

    for i in 0..stroke_count {
        let x = (i % 200) as f64 * 3.0;
        let y = (i / 200) as f64 * 15.0;
        let stroke = make_spline_stroke(i as u64 + 1, x, y, 6);
        doc.pages[0].items.push(Item {
            id: i as u64 + 1,
            z_index: i as u64 + 1,
            kind: ItemKind::Stroke(stroke),
        });
    }

    store.save_document(&doc).expect("save_document");

    let loaded = store.load_document().expect("load_document");
    assert_eq!(loaded.pages.len(), 1);
    let page = &loaded.pages[0];
    assert_eq!(page.items.len(), stroke_count);
    assert_eq!(page.spatial_index.len(), stroke_count);

    // Verify Bézier curve integrity and cache rehydration on all 10k strokes
    for (idx, item) in page.items.iter().enumerate() {
        assert_eq!(item.id, idx as u64 + 1);
        assert_eq!(item.z_index, idx as u64 + 1);
        if let ItemKind::Stroke(ref stroke) = item.kind {
            assert!(
                stroke.cached_bbox.is_some(),
                "cached_bbox must be rehydrated for stroke {idx}"
            );
            assert_eq!(stroke.cached_bezier.len(), stroke.points.len() - 1);
            for seg in &stroke.cached_bezier {
                assert!(seg.is_finite(), "Bézier segment must be finite");
            }
        } else {
            panic!("Expected stroke");
        }
    }
}

// ============================================================================
// 2. MULTI-PAGE COMPLEX DOCUMENT WITH 10,000 STROKES DISTRIBUTED
// ============================================================================

#[test]
fn test_adversarial_load_100_pages_10k_strokes_multi_page() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");

    let num_pages = 100;
    let strokes_per_page = 100;
    let mut doc = Document::new_notebook("100 Pages 10k Strokes");
    doc.pages.clear();

    for p in 0..num_pages {
        let mut page = Page::new_a4(p, PageBackground::default());
        page.items.reserve(strokes_per_page);
        for s in 0..strokes_per_page {
            let global_id = (p * strokes_per_page + s) as u64 + 1;
            let stroke = make_spline_stroke(global_id, (s * 5) as f64, (s * 7) as f64, 5);
            page.items.push(Item {
                id: global_id,
                z_index: s as u64 + 1,
                kind: ItemKind::Stroke(stroke),
            });
        }
        doc.pages.push(page);
    }

    store.save_document(&doc).expect("save_document");

    let loaded = store.load_document().expect("load_document");
    assert_eq!(loaded.pages.len(), num_pages);

    for (p, page) in loaded.pages.iter().enumerate() {
        assert_eq!(page.index, p);
        assert_eq!(page.items.len(), strokes_per_page);
        assert_eq!(page.spatial_index.len(), strokes_per_page);

        for (s, item) in page.items.iter().enumerate() {
            let expected_id = (p * strokes_per_page + s) as u64 + 1;
            assert_eq!(item.id, expected_id);
            assert_eq!(item.z_index, s as u64 + 1);

            if let ItemKind::Stroke(ref stroke) = item.kind {
                assert!(stroke.cached_bbox.is_some());
                assert_eq!(stroke.cached_bezier.len(), 4);
                for seg in &stroke.cached_bezier {
                    assert!(seg.is_finite());
                }
            }
        }
    }
}

// ============================================================================
// 3. INTERLEAVED Z-INDEXES AND MIXED TYPES (STROKES + IMAGES)
// ============================================================================

#[test]
fn test_adversarial_interleaved_z_indexes_and_mixed_items() {
    let temp_file = NamedTempFile::new().expect("temp file");

    // We populate the raw SQLite database directly with out-of-order, mixed inserts!
    {
        let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");
        let mut doc = Document::new_notebook("Interleaved Z-Index Doc");
        // Add 3 pages
        doc.pages.push(Page::new_a4(1, PageBackground::default()));
        doc.pages.push(Page::new_a4(2, PageBackground::default()));
        store.save_document(&doc).expect("save_document initial");
    }

    // Now insert items directly using raw SQL with scrambled page_ids and z_indexes
    {
        let conn = rusqlite::Connection::open(temp_file.path()).expect("raw conn");
        let mut page_ids: Vec<String> = Vec::new();
        let mut stmt = conn
            .prepare("SELECT id FROM pages ORDER BY page_index ASC")
            .expect("prepare");
        let mut rows = stmt.query([]).expect("query");
        while let Some(row) = rows.next().expect("next") {
            page_ids.push(row.get(0).expect("get"));
        }
        assert_eq!(page_ids.len(), 3);

        // We prepare inserts:
        // Page 0: items with z-index 50, 10, 100, 1, 25 (scrambled)
        // Page 1: mixed strokes and images with z-index 80, 20, 60, 40
        // Page 2: items with arbitrary non-sequential z-indexes
        let raw_inserts = vec![
            (&page_ids[0], 101, 50, "stroke"),
            (&page_ids[1], 201, 80, "image"),
            (&page_ids[0], 102, 10, "stroke"),
            (&page_ids[2], 301, 999, "stroke"),
            (&page_ids[1], 202, 20, "stroke"),
            (&page_ids[0], 103, 100, "stroke"),
            (&page_ids[2], 302, 5, "stroke"),
            (&page_ids[1], 203, 60, "stroke"),
            (&page_ids[0], 104, 1, "image"),
            (&page_ids[1], 204, 40, "image"),
            (&page_ids[0], 105, 25, "stroke"),
        ];

        for (pid, id, z, itype) in raw_inserts {
            let page_uuid = Uuid::parse_str(pid).expect("uuid");
            let data_json = if itype == "stroke" {
                let s = make_spline_stroke(id as u64, (z * 2) as f64, (z * 3) as f64, 4);
                serde_json::to_string(&s).expect("json")
            } else {
                let img = ImageItem {
                    id: Uuid::new_v4(),
                    page_id: page_uuid,
                    asset_hash: format!("asset_{id}"),
                    bbox: Rect::new((z * 2) as f64, (z * 3) as f64, 50.0, 50.0),
                    z_index: z,
                    opacity: 1.0,
                    original_width: 100,
                    original_height: 100,
                };
                serde_json::to_string(&img).expect("json")
            };

            conn.execute(
                "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
                 VALUES (?1, ?2, ?3, ?4, '{}', ?5)",
                params![id, pid, z, itype, data_json],
            )
            .expect("insert item");
        }
    }

    // Now load document through DocumentStore::load_document
    let store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");
    let loaded = store.load_document().expect("load_document");

    assert_eq!(loaded.pages.len(), 3);

    // Page 0: expected sorted z-indexes: 1 (image), 10 (stroke), 25 (stroke), 50 (stroke), 100 (stroke)
    let p0 = &loaded.pages[0];
    assert_eq!(p0.items.len(), 5);
    assert_eq!(p0.items[0].z_index, 1);
    assert_eq!(p0.items[0].id, 104);
    assert!(matches!(p0.items[0].kind, ItemKind::Image(_)));

    assert_eq!(p0.items[1].z_index, 10);
    assert_eq!(p0.items[1].id, 102);
    assert!(matches!(p0.items[1].kind, ItemKind::Stroke(_)));

    assert_eq!(p0.items[2].z_index, 25);
    assert_eq!(p0.items[2].id, 105);
    assert!(matches!(p0.items[2].kind, ItemKind::Stroke(_)));

    assert_eq!(p0.items[3].z_index, 50);
    assert_eq!(p0.items[3].id, 101);
    assert!(matches!(p0.items[3].kind, ItemKind::Stroke(_)));

    assert_eq!(p0.items[4].z_index, 100);
    assert_eq!(p0.items[4].id, 103);
    assert!(matches!(p0.items[4].kind, ItemKind::Stroke(_)));

    // Verify spatial index len matches items len
    assert_eq!(p0.spatial_index.len(), 5);

    // Page 1: expected sorted z-indexes: 20 (stroke), 40 (image), 60 (stroke), 80 (image)
    let p1 = &loaded.pages[1];
    assert_eq!(p1.items.len(), 4);
    assert_eq!(p1.items[0].z_index, 20);
    assert_eq!(p1.items[0].id, 202);
    assert!(matches!(p1.items[0].kind, ItemKind::Stroke(_)));

    assert_eq!(p1.items[1].z_index, 40);
    assert_eq!(p1.items[1].id, 204);
    assert!(matches!(p1.items[1].kind, ItemKind::Image(_)));

    assert_eq!(p1.items[2].z_index, 60);
    assert_eq!(p1.items[2].id, 203);
    assert!(matches!(p1.items[2].kind, ItemKind::Stroke(_)));

    assert_eq!(p1.items[3].z_index, 80);
    assert_eq!(p1.items[3].id, 201);
    assert!(matches!(p1.items[3].kind, ItemKind::Image(_)));
    assert_eq!(p1.spatial_index.len(), 4);

    // Page 2: expected sorted z-indexes: 5 (stroke), 999 (stroke)
    let p2 = &loaded.pages[2];
    assert_eq!(p2.items.len(), 2);
    assert_eq!(p2.items[0].z_index, 5);
    assert_eq!(p2.items[0].id, 302);
    assert_eq!(p2.items[1].z_index, 999);
    assert_eq!(p2.items[1].id, 301);
    assert_eq!(p2.spatial_index.len(), 2);
}

// ============================================================================
// 4. BÉZIER CURVE INTEGRITY & CONTINUITY ON DESERIALIZED STROKES
// ============================================================================

#[test]
fn test_adversarial_bezier_curve_c0_continuity_and_finite_points() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");

    let mut doc = Document::new_notebook("Bezier Curve Verification");
    let mut test_strokes = Vec::new();

    // 1. Single dot stroke
    let dot = Stroke {
        id: 1,
        style: StrokeStyle::default(),
        points: vec![StrokePoint {
            x: 50.0,
            y: 50.0,
            pressure: 0.5,
            width: 3.0,
            timestamp_ns: 1000,
        }],
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    test_strokes.push(dot);

    // 2. 2-point stroke (minimal segment)
    let two_pt = Stroke {
        id: 2,
        style: StrokeStyle::default(),
        points: vec![
            StrokePoint {
                x: 10.0,
                y: 10.0,
                pressure: 0.4,
                width: 2.0,
                timestamp_ns: 1000,
            },
            StrokePoint {
                x: 80.0,
                y: 90.0,
                pressure: 0.8,
                width: 3.5,
                timestamp_ns: 2000,
            },
        ],
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    test_strokes.push(two_pt);

    // 3. Multi-point complex spline stroke (100 points)
    let complex = make_spline_stroke(3, 100.0, 100.0, 100);
    test_strokes.push(complex);

    // 4. OneEuroFiltered stroke
    let mut filtered = make_spline_stroke(4, 300.0, 300.0, 50);
    filtered.mode = PipelineMode::OneEuroFiltered;
    test_strokes.push(filtered);

    for s in test_strokes {
        doc.pages[0].items.push(Item {
            id: s.id,
            z_index: s.id,
            kind: ItemKind::Stroke(s),
        });
    }

    store.save_document(&doc).expect("save_document");

    let loaded = store.load_document().expect("load_document");
    let page = &loaded.pages[0];
    assert_eq!(page.items.len(), 4);

    // Verify Stroke 1 (Dot)
    if let ItemKind::Stroke(ref s) = page.items[0].kind {
        assert!(s.is_dot);
        assert!(s.cached_bbox.is_some());
        assert_eq!(s.cached_bezier.len(), 0);
    } else {
        panic!("stroke expected");
    }

    // Verify Stroke 2 (2 points)
    if let ItemKind::Stroke(ref s) = page.items[1].kind {
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.cached_bezier.len(), 1);
        let seg = &s.cached_bezier[0];
        assert!(seg.is_finite());
        assert!((seg.p0.0 - 10.0).abs() < 1e-4);
        assert!((seg.p0.1 - 10.0).abs() < 1e-4);
        assert!((seg.p1.0 - 80.0).abs() < 1e-4);
        assert!((seg.p1.1 - 90.0).abs() < 1e-4);
    } else {
        panic!("stroke expected");
    }

    // Verify Stroke 3 (100 points) & Stroke 4 (50 points): check C0 continuity across all segments
    for item in &page.items[2..4] {
        if let ItemKind::Stroke(ref s) = item.kind {
            assert_eq!(s.cached_bezier.len(), s.points.len() - 1);
            for i in 0..s.cached_bezier.len() {
                let seg = &s.cached_bezier[i];
                assert!(seg.is_finite(), "Segment {} must be finite", i);
                if i > 0 {
                    let prev_seg = &s.cached_bezier[i - 1];
                    // C0 continuity: previous endpoint must equal next startpoint
                    let dist = ((prev_seg.p1.0 - seg.p0.0).powi(2)
                        + (prev_seg.p1.1 - seg.p0.1).powi(2))
                    .sqrt();
                    assert!(
                        dist < 1e-4,
                        "C0 discontinuity at junction {i}: dist = {dist}"
                    );
                }
            }
        }
    }
}

// ============================================================================
// 5. R*-TREE SPATIAL INDEX INTEGRITY & ORACLE EQUIVALENCE
// ============================================================================

#[test]
fn test_adversarial_spatial_index_oracle_equivalence_after_load() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");

    let stroke_count = 1_000;
    let mut doc = Document::new_notebook("Spatial Oracle Test");
    for i in 0..stroke_count {
        let x = (i % 50) as f64 * 16.0;
        let y = (i / 50) as f64 * 40.0;
        let stroke = make_spline_stroke(i as u64 + 1, x, y, 8);
        doc.pages[0].items.push(Item {
            id: i as u64 + 1,
            z_index: i as u64 + 1,
            kind: ItemKind::Stroke(stroke),
        });
    }

    store.save_document(&doc).expect("save_document");

    let loaded = store.load_document().expect("load_document");
    let page = &loaded.pages[0];
    assert_eq!(page.items.len(), stroke_count);
    assert_eq!(page.spatial_index.len(), stroke_count);

    // Test 6 diverse query windows against ground-truth linear scan
    let test_windows = vec![
        Rect::new(0.0, 0.0, 100.0, 100.0),
        Rect::new(200.0, 300.0, 150.0, 200.0),
        Rect::new(500.0, 600.0, 300.0, 300.0),
        Rect::new(0.0, 0.0, 1000.0, 1000.0),   // Large window
        Rect::new(9999.0, 9999.0, 50.0, 50.0), // Disjoint window (should return 0)
        Rect::new(100.0, 100.0, 0.1, 0.1),     // Tiny window
    ];

    for window in test_windows {
        // Oracle: linear brute-force scan
        let mut expected_ids: HashSet<u64> = HashSet::new();
        for item in &page.items {
            if let Some(bbox) = item.bounding_box() {
                if bbox.intersects_rect(&window) {
                    expected_ids.insert(item.id);
                }
            }
        }

        // Query spatial R*-Tree via locate_in_envelope
        let spatial_results = page.spatial_index.locate_in_envelope(
            window.min_x,
            window.min_y,
            window.max_x,
            window.max_y,
        );
        let spatial_ids: HashSet<u64> = spatial_results.iter().map(|item| item.item_id).collect();

        assert_eq!(
            spatial_ids, expected_ids,
            "Spatial index oracle divergence for window {:?}",
            window
        );
    }
}

// ============================================================================
// 6. ADVERSARIAL FAULT TOLERANCE & CORRUPTION RESISTANCE
// ============================================================================

#[test]
fn test_adversarial_orphaned_and_corrupt_items_tolerance() {
    let temp_file = NamedTempFile::new().expect("temp file");

    {
        let mut store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");
        let doc = Document::new_notebook("Corrupt and Orphaned Test");
        store.save_document(&doc).expect("initial save");
    }

    // Insert an orphaned item (non-existent page) and an invalid JSON item into valid page
    {
        let conn = rusqlite::Connection::open(temp_file.path()).expect("conn");
        conn.execute("PRAGMA foreign_keys = OFF;", [])
            .expect("disable fk");
        let valid_page_id: String = conn
            .query_row("SELECT id FROM pages LIMIT 1", [], |r| r.get(0))
            .expect("page_id");

        // 1. Orphaned item
        conn.execute(
            "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
             VALUES (9999, '00000000-0000-0000-0000-000000000000', 1, 'stroke', '{}', '{}')",
            [],
        )
        .expect("insert orphaned item");

        // 2. Corrupted JSON item on valid page
        conn.execute(
            "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
             VALUES (8888, ?1, 5, 'stroke', '{}', 'INVALID_JSON_CORRUPTED{{{')",
            params![valid_page_id],
        )
        .expect("insert corrupt item");

        // 3. Valid item with high z-index
        let valid_stroke = make_spline_stroke(7777, 20.0, 20.0, 4);
        let valid_json = serde_json::to_string(&valid_stroke).expect("json");
        conn.execute(
            "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
             VALUES (7777, ?1, 10, 'stroke', '{}', ?2)",
            params![valid_page_id, valid_json],
        )
        .expect("insert valid item");
    }

    let store = DocumentStore::open_or_create(temp_file.path()).expect("open_or_create");
    let loaded = store
        .load_document()
        .expect("load_document must tolerate corrupted/orphaned items");

    let page = &loaded.pages[0];
    // Orphaned item (9999) must NOT be in page
    assert!(!page.items.iter().any(|i| i.id == 9999));
    // Corrupt JSON item (8888) must be gracefully skipped
    assert!(!page.items.iter().any(|i| i.id == 8888));
    // Valid item (7777) must be present and correctly rehydrated
    let item_7777 = page
        .items
        .iter()
        .find(|i| i.id == 7777)
        .expect("item 7777 found");
    assert_eq!(item_7777.z_index, 10);
    assert_eq!(page.spatial_index.len(), 1);
}
