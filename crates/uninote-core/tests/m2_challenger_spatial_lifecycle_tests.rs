//! Adversarial empirical challenge test suite for Milestone M2 (Spatial Index Lifecycle).
//!
//! Stress-tests:
//! 1. 10,000 dynamic insert and remove cycles (sequential, interleaved churn, memory & tree integrity).
//! 2. Multi-page document rebuilding with 100 heterogeneous pages (A4, Whiteboard, PDF, empty, dots, degenerate).
//! 3. Serde skip isolation on `Page` and `Document` via JSON serialization and deserialization.
//! 4. Adversarial geometry: coincident elements, stale bbox fallbacks, extreme coordinates, NaN/Inf robustness.

use std::time::Instant;
use uninote_core::model::{Document, Item, ItemKind, Page, PageBackground};
use uninote_core::spatial::PageSpatialIndex;
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

/// Helper to generate a stroke with deterministic coordinates and style.
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

/// Helper to generate a dot stroke (single point).
fn make_dot_stroke(id: u64, x: f64, y: f64) -> Stroke {
    let p0 = StrokePoint {
        x,
        y,
        pressure: 0.6,
        width: 3.0,
        timestamp_ns: 1000,
    };
    let mut stroke = Stroke {
        id,
        points: vec![p0],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

#[test]
fn test_dynamic_lifecycle_10000_sequential_insert_remove_cycles() {
    const N: usize = 10_000;
    let mut index = PageSpatialIndex::empty();

    // 1. Sequentially insert 10,000 items distributed across a 10,000 x 10,000 pt domain
    let start_insert = Instant::now();
    for i in 0..N {
        let x = (i % 100) as f64 * 100.0;
        let y = (i / 100) as f64 * 100.0;
        let stroke = make_test_stroke(i as u64, x, y, x + 30.0, y + 20.0);
        let item = Item {
            id: i as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        };
        index.insert(&item);
    }
    let insert_duration = start_insert.elapsed();
    println!("Inserted 10,000 items in {:?}", insert_duration);

    assert_eq!(index.len(), N);
    assert!(!index.is_empty());

    // 2. Remove 5,000 even-numbered items (i % 2 == 0)
    let start_remove_half = Instant::now();
    for i in (0..N).step_by(2) {
        let x = (i % 100) as f64 * 100.0;
        let y = (i / 100) as f64 * 100.0;
        let bbox = Rect::new(x, y, x + 30.0, y + 20.0);
        let removed = index.remove(i as u64, &bbox);
        assert!(removed, "Failed to remove existing even item {}", i);
    }
    let remove_half_duration = start_remove_half.elapsed();
    println!("Removed 5,000 even items in {:?}", remove_half_duration);

    assert_eq!(index.len(), 5_000);

    // 3. Verify integrity:
    // a) Even items must not exist (remove returns false)
    for i in (0..N).step_by(2) {
        let x = (i % 100) as f64 * 100.0;
        let y = (i / 100) as f64 * 100.0;
        let bbox = Rect::new(x, y, x + 30.0, y + 20.0);
        assert!(
            !index.remove(i as u64, &bbox),
            "Even item {} was supposed to be gone",
            i
        );
    }

    // b) Odd items must all still be findable in the tree with bounds enclosing their full bounding box
    let sample_query = index.locate_in_envelope(-10.0, -10.0, 11_000.0, 11_000.0);
    assert_eq!(sample_query.len(), 5_000);
    for item in &sample_query {
        assert_eq!(
            item.item_id % 2,
            1,
            "Found even item {} that was previously deleted",
            item.item_id
        );
    }

    // 4. Remove all remaining 5,000 odd items
    let start_remove_rest = Instant::now();
    for i in (1..N).step_by(2) {
        let x = (i % 100) as f64 * 100.0;
        let y = (i / 100) as f64 * 100.0;
        let bbox = Rect::new(x, y, x + 30.0, y + 20.0);
        let removed = index.remove(i as u64, &bbox);
        assert!(removed, "Failed to remove existing odd item {}", i);
    }
    let remove_rest_duration = start_remove_rest.elapsed();
    println!("Removed 5,000 odd items in {:?}", remove_rest_duration);

    // 5. Assert tree is completely empty
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());

    let final_query = index.locate_in_envelope(0.0, 0.0, 10_000.0, 10_000.0);
    assert!(final_query.is_empty());
}

#[test]
fn test_dynamic_lifecycle_10000_interleaved_churn_cycles() {
    const TOTAL_CYCLES: usize = 10_000;
    const WINDOW_SIZE: usize = 200;
    let mut index = PageSpatialIndex::empty();

    let start_churn = Instant::now();

    for i in 0..TOTAL_CYCLES {
        // Deterministic pseudo-random position
        let x = ((i * 37) % 5000) as f64;
        let y = ((i * 73) % 5000) as f64;
        let stroke = make_test_stroke(i as u64, x, y, x + 25.0, y + 25.0);
        let item = Item {
            id: i as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        };
        index.insert(&item);

        // Once window fills up, remove oldest active item
        if i >= WINDOW_SIZE {
            let old_id = (i - WINDOW_SIZE) as u64;
            let old_x = ((old_id as usize * 37) % 5000) as f64;
            let old_y = ((old_id as usize * 73) % 5000) as f64;
            let old_bbox = Rect::new(old_x, old_y, old_x + 25.0, old_y + 25.0);
            let removed = index.remove(old_id, &old_bbox);
            assert!(
                removed,
                "Failed to remove window-evicted item {} at step {}",
                old_id, i
            );
        }

        // Periodic checkpoint validation every 1,000 steps
        if i % 1000 == 0 && i >= WINDOW_SIZE {
            assert_eq!(index.len(), WINDOW_SIZE);
            // Verify latest inserted item is findable
            let loc = index.locate_in_envelope(x - 5.0, y - 5.0, x + 30.0, y + 30.0);
            assert!(
                loc.iter().any(|it| it.item_id == i as u64),
                "Latest inserted item {} not found in envelope query",
                i
            );
        }
    }

    let churn_duration = start_churn.elapsed();
    println!(
        "Completed 10,000 interleaved insert/remove churn cycles in {:?}",
        churn_duration
    );
    assert_eq!(index.len(), WINDOW_SIZE);

    // Drain remaining window items
    for id in (TOTAL_CYCLES - WINDOW_SIZE)..TOTAL_CYCLES {
        let x = ((id * 37) % 5000) as f64;
        let y = ((id * 73) % 5000) as f64;
        let bbox = Rect::new(x, y, x + 25.0, y + 25.0);
        assert!(index.remove(id as u64, &bbox));
    }

    assert_eq!(index.len(), 0);
    assert!(index.is_empty());

    // Post-churn re-insertion check: verify tree structure did not degrade
    for i in 0..500 {
        let stroke = make_test_stroke(
            i as u64,
            i as f64 * 5.0,
            i as f64 * 5.0,
            i as f64 * 5.0 + 10.0,
            i as f64 * 5.0 + 10.0,
        );
        let item = Item {
            id: i as u64,
            z_index: i as u64 + 1,
            kind: ItemKind::Stroke(stroke),
        };
        index.insert(&item);
    }
    assert_eq!(index.len(), 500);
    let all = index.locate_in_envelope(-10.0, -10.0, 3000.0, 3000.0);
    assert_eq!(all.len(), 500);
}

#[test]
fn test_adversarial_remove_stale_or_perturbed_bbox_fallback() {
    let mut index = PageSpatialIndex::empty();

    let stroke1 = make_test_stroke(42, 100.0, 100.0, 150.0, 150.0);
    let item1 = Item {
        id: 42,
        z_index: 10,
        kind: ItemKind::Stroke(stroke1),
    };
    index.insert(&item1);
    assert_eq!(index.len(), 1);

    // 1. Attempt removal with a perturbed / disjoint bounding box (e.g. translated by +500 pt)
    // The envelope query in remove will miss, but the linear fallback must find and remove it!
    let stale_bbox = Rect::new(600.0, 600.0, 650.0, 650.0);
    let removed = index.remove(42, &stale_bbox);
    assert!(
        removed,
        "Linear fallback failed to remove item with stale/perturbed bounding box"
    );
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());

    // 2. Removal on non-existent item ID returns false
    assert!(!index.remove(999, &stale_bbox));

    // 3. Removal on empty index returns false
    assert!(!index.remove(42, &stale_bbox));
}

#[test]
fn test_coincident_and_overlapping_elements_z_index_stability() {
    let mut index = PageSpatialIndex::empty();
    const NUM_COINCIDENT: usize = 300;

    // Insert 300 items at the EXACT same geometric coordinates with reverse z-indices
    for i in 0..NUM_COINCIDENT {
        let stroke = make_test_stroke(i as u64, 50.0, 50.0, 120.0, 120.0);
        let item = Item {
            id: i as u64,
            z_index: (NUM_COINCIDENT - i) as u64 * 10,
            kind: ItemKind::Stroke(stroke),
        };
        index.insert(&item);
    }

    assert_eq!(index.len(), NUM_COINCIDENT);

    // Query the exact overlapping envelope
    let res = index.locate_in_envelope(40.0, 40.0, 130.0, 130.0);
    assert_eq!(res.len(), NUM_COINCIDENT);

    // Verify strict ascending z-index ordering
    for w in res.windows(2) {
        assert!(
            w[0].z_index <= w[1].z_index,
            "Z-index ordering violation: {} > {}",
            w[0].z_index,
            w[1].z_index
        );
    }

    // Remove single specific item in the middle (id: 150)
    let bbox = Rect::new(50.0, 50.0, 120.0, 120.0);
    assert!(index.remove(150, &bbox));
    assert_eq!(index.len(), NUM_COINCIDENT - 1);

    // Verify item 150 is gone, while items 149 and 151 remain
    let res_after = index.locate_in_envelope(40.0, 40.0, 130.0, 130.0);
    assert_eq!(res_after.len(), NUM_COINCIDENT - 1);
    assert!(!res_after.iter().any(|it| it.item_id == 150));
    assert!(res_after.iter().any(|it| it.item_id == 149));
    assert!(res_after.iter().any(|it| it.item_id == 151));

    // Drain all remaining
    for i in 0..NUM_COINCIDENT {
        if i != 150 {
            assert!(index.remove(i as u64, &bbox));
        }
    }
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());
}

#[test]
fn test_rebuild_spatial_indices_100_heterogeneous_pages() {
    let mut doc = Document::new_notebook("Heterogeneous 100-Page Challenge");
    let mut expected_counts = Vec::with_capacity(100);

    // Page 0 is created by default as A4
    doc.pages[0].background = PageBackground::Blank;

    // Create 99 more pages to reach 100 pages total
    for i in 1..100 {
        if i < 40 {
            // Pages 0..39 (40 pages): Notebook A4
            doc.add_page(PageBackground::Ruled {
                line_spacing: 22.68,
                margin_left: 40.0,
            });
        } else if i < 70 {
            // Pages 40..69 (30 pages): Infinite Whiteboard
            doc.pages.push(Page::new_whiteboard(PageBackground::Grid {
                grid_size: 14.17,
            }));
        } else if i < 90 {
            // Pages 70..89 (20 pages): PDF Pages
            doc.pages.push(Page::new_pdf_page(
                i,
                format!("/home/cachylore/docs/lecture_{}.pdf", i),
                i % 10,
                800.0,
                600.0,
            ));
        } else if i < 93 {
            // Pages 90..92 (3 pages): Empty pages
            doc.add_page(PageBackground::Blank);
        } else if i < 95 {
            // Pages 93..94 (2 pages): Single-dot pages
            doc.add_page(PageBackground::Blank);
        } else if i < 97 {
            // Pages 95..96 (2 pages): Degenerate strokes with 0 points (no bbox)
            doc.add_page(PageBackground::Blank);
        } else {
            // Pages 97..99 (3 pages): Dense pages (300 strokes each)
            doc.add_page(PageBackground::Grid { grid_size: 14.17 });
        }
    }

    assert_eq!(doc.pages.len(), 100);

    let mut global_item_id = 1u64;

    // Populate each page according to its category
    for (page_idx, page) in doc.pages.iter_mut().enumerate() {
        if page_idx < 40 {
            // 40 A4 pages: 50 strokes each
            for s in 0..50 {
                let x = (s % 10) as f64 * 50.0;
                let y = (s / 10) as f64 * 100.0;
                let stroke = make_test_stroke(global_item_id, x, y, x + 35.0, y + 25.0);
                page.items.push(Item {
                    id: global_item_id,
                    z_index: s as u64 + 1,
                    kind: ItemKind::Stroke(stroke),
                });
                global_item_id += 1;
            }
            expected_counts.push(50);
        } else if page_idx < 70 {
            // 30 Whiteboard pages: 30 strokes with negative and large coordinates
            for s in 0..30 {
                let x = -2000.0 + (s as f64 * 150.0);
                let y = -1500.0 + (s as f64 * 100.0);
                let stroke = make_test_stroke(global_item_id, x, y, x + 40.0, y + 30.0);
                page.items.push(Item {
                    id: global_item_id,
                    z_index: s as u64 + 1,
                    kind: ItemKind::Stroke(stroke),
                });
                global_item_id += 1;
            }
            expected_counts.push(30);
        } else if page_idx < 90 {
            // 20 PDF pages: 20 strokes each
            for s in 0..20 {
                let x = (s * 35) as f64;
                let y = (s * 25) as f64;
                let stroke = make_test_stroke(global_item_id, x, y, x + 30.0, y + 20.0);
                page.items.push(Item {
                    id: global_item_id,
                    z_index: s as u64 + 1,
                    kind: ItemKind::Stroke(stroke),
                });
                global_item_id += 1;
            }
            expected_counts.push(20);
        } else if page_idx < 93 {
            // 3 Empty pages: 0 strokes
            expected_counts.push(0);
        } else if page_idx < 95 {
            // 2 Single-dot pages: 1 dot stroke
            let stroke = make_dot_stroke(global_item_id, 250.0, 350.0);
            page.items.push(Item {
                id: global_item_id,
                z_index: 1,
                kind: ItemKind::Stroke(stroke),
            });
            global_item_id += 1;
            expected_counts.push(1);
        } else if page_idx < 97 {
            // 2 Degenerate pages: 2 strokes with empty points (bounding_box() returns None)
            for _ in 0..2 {
                let stroke = Stroke {
                    id: global_item_id,
                    points: Vec::new(),
                    style: StrokeStyle::default(),
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::Baseline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                page.items.push(Item {
                    id: global_item_id,
                    z_index: 1,
                    kind: ItemKind::Stroke(stroke),
                });
                global_item_id += 1;
            }
            // Because bounding_box() is None, spatial index excludes them
            expected_counts.push(0);
        } else {
            // 3 Dense pages: 300 strokes each
            for s in 0..300 {
                let x = (s % 20) as f64 * 25.0;
                let y = (s / 20) as f64 * 50.0;
                let stroke = make_test_stroke(global_item_id, x, y, x + 20.0, y + 20.0);
                page.items.push(Item {
                    id: global_item_id,
                    z_index: s as u64 + 1,
                    kind: ItemKind::Stroke(stroke),
                });
                global_item_id += 1;
            }
            expected_counts.push(300);
        }
    }

    // Verify all spatial indices are currently empty before rebuild
    for p in &doc.pages {
        assert_eq!(p.spatial_index.len(), 0);
        assert!(p.spatial_index.is_empty());
    }

    // Rebuild all 100 pages
    let start_rebuild = Instant::now();
    doc.rebuild_spatial_indices();
    let elapsed_rebuild = start_rebuild.elapsed();

    println!(
        "Rebuilt spatial indices for 100 heterogeneous pages in {:?}",
        elapsed_rebuild
    );
    assert!(
        elapsed_rebuild.as_millis() < 500,
        "Rebuilding 100 pages took too long: {} ms",
        elapsed_rebuild.as_millis()
    );

    // Verify every page's spatial_index.len() exactly matches expected_counts
    for (page_idx, page) in doc.pages.iter().enumerate() {
        assert_eq!(
            page.spatial_index.len(),
            expected_counts[page_idx],
            "Page {} index length mismatch: expected {}, got {}",
            page_idx,
            expected_counts[page_idx],
            page.spatial_index.len()
        );
    }

    // Spot-check Whiteboard query with negative coordinates
    let wb_page = &doc.pages[45];
    let wb_matches = wb_page
        .spatial_index
        .locate_in_envelope(-2500.0, -2000.0, 0.0, 0.0);
    assert!(
        !wb_matches.is_empty(),
        "Whiteboard negative coordinate query returned empty"
    );
    for w in wb_matches.windows(2) {
        assert!(w[0].z_index <= w[1].z_index);
    }
}

#[test]
fn test_serde_skip_isolation_page_and_document_json() {
    let mut doc = Document::new_notebook("Serde Skip Test Document");
    for s in 0..20 {
        let stroke = make_test_stroke(
            s as u64 + 1,
            s as f64 * 10.0,
            s as f64 * 10.0,
            s as f64 * 10.0 + 30.0,
            s as f64 * 10.0 + 20.0,
        );
        doc.pages[0].items.push(Item {
            id: s as u64 + 1,
            z_index: s as u64 + 1,
            kind: ItemKind::Stroke(stroke),
        });
    }

    // Populate spatial index on page 0
    doc.pages[0].rebuild_spatial_index();
    assert_eq!(doc.pages[0].spatial_index.len(), 20);

    // 1. Serialize single Page
    let page_json = serde_json::to_string(&doc.pages[0]).expect("Page JSON serialization failed");
    assert!(
        !page_json.contains("spatial_index"),
        "Serialized Page JSON contaminated with 'spatial_index'"
    );
    assert!(
        !page_json.contains("\"tree\""),
        "Serialized Page JSON contaminated with 'tree'"
    );

    // 2. Deserialize single Page
    let mut deserialized_page: Page =
        serde_json::from_str(&page_json).expect("Page JSON deserialization failed");
    // Under #[serde(skip, default)], deserialized spatial_index must be empty
    assert_eq!(deserialized_page.spatial_index.len(), 0);
    assert!(deserialized_page.spatial_index.is_empty());

    // Rehydrate
    deserialized_page.rebuild_spatial_index();
    assert_eq!(deserialized_page.spatial_index.len(), 20);

    // 3. Serialize Document
    let doc_json = serde_json::to_string(&doc).expect("Document JSON serialization failed");
    assert!(
        !doc_json.contains("spatial_index"),
        "Serialized Document JSON contaminated with 'spatial_index'"
    );

    // 4. Deserialize Document
    let mut deserialized_doc: Document =
        serde_json::from_str(&doc_json).expect("Document JSON deserialization failed");
    assert_eq!(deserialized_doc.pages[0].spatial_index.len(), 0);
    assert!(deserialized_doc.pages[0].spatial_index.is_empty());

    // Rebuild all
    deserialized_doc.rebuild_spatial_indices();
    assert_eq!(deserialized_doc.pages[0].spatial_index.len(), 20);
}

#[test]
fn test_adversarial_pathological_bounding_boxes() {
    let mut index = PageSpatialIndex::empty();

    // 1. Zero-dimensional dot stroke
    let dot = make_dot_stroke(1, 100.0, 100.0);
    let item_dot = Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(dot),
    };
    index.insert(&item_dot);
    assert_eq!(index.len(), 1);

    // Query enclosing the dot
    let q_dot = index.locate_in_envelope(95.0, 95.0, 105.0, 105.0);
    assert_eq!(q_dot.len(), 1);
    assert_eq!(q_dot[0].item_id, 1);

    // 2. Extreme astronomical coordinates
    let extreme_stroke = make_test_stroke(2, -1e8, -1e8, -1e8 + 50.0, -1e8 + 50.0);
    let item_extreme = Item {
        id: 2,
        z_index: 2,
        kind: ItemKind::Stroke(extreme_stroke),
    };
    index.insert(&item_extreme);
    assert_eq!(index.len(), 2);

    let q_extreme = index.locate_in_envelope(-1e8 - 10.0, -1e8 - 10.0, -1e8 + 100.0, -1e8 + 100.0);
    assert_eq!(q_extreme.len(), 1);
    assert_eq!(q_extreme[0].item_id, 2);

    // 3. Degenerate inverted query bounds
    let q_inverted = index.locate_in_envelope(500.0, 500.0, 50.0, 50.0);
    assert_eq!(q_inverted.len(), 1); // should find dot at 100,100
    assert_eq!(q_inverted[0].item_id, 1);

    // 4. NaN & Infinite query coordinates
    let q_nan = index.locate_in_envelope(f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    assert_eq!(q_nan.len(), 2); // clamped to [-1e12, 1e12], encloses both items
}

#[test]
fn test_boundary_intersecting_strokes_culling_semantics() {
    let mut index = PageSpatialIndex::empty();

    // A stroke spanning from (50.0, 50.0) to (150.0, 150.0)
    let stroke = make_test_stroke(1001, 50.0, 50.0, 150.0, 150.0);
    let item = Item {
        id: 1001,
        z_index: 1,
        kind: ItemKind::Stroke(stroke),
    };
    index.insert(&item);
    assert_eq!(index.len(), 1);

    // Viewport window [100.0, 100.0, 200.0, 200.0]
    // The stroke intersects this viewport between (100, 100) and (150, 150).
    let results = index.locate_in_envelope(100.0, 100.0, 200.0, 200.0);

    // Note: rstar::RTree::locate_in_envelope only returns elements FULLY CONTAINED within the envelope!
    // Because the stroke starts at 50.0 < 100.0, it is not fully contained.
    // For executive viewport culling (M4), Cairo rendering requires intersecting strokes to be drawn.
    // Let's document whether results.len() is 0 (contained-only) or 1 (intersecting).
    println!(
        "Query [100..200, 100..200] for stroke with AABB [49..151, 49..151]: returned {} items",
        results.len()
    );
    // Directly verifies the containment behavior of locate_in_envelope vs locate_in_envelope_intersecting
    let is_contained_only = results.is_empty();
    println!(
        "locate_in_envelope semantic is contained_only: {}",
        is_contained_only
    );
    assert_eq!(
        results.len(),
        1,
        "PageSpatialIndex::locate_in_envelope must return intersecting stroke"
    );

    // Verify that rstar's locate_in_envelope_intersecting correctly finds the stroke!
    let envelope = rstar::AABB::from_corners([100.0, 100.0], [200.0, 200.0]);
    let intersecting_count = index
        .tree()
        .locate_in_envelope_intersecting(envelope)
        .count();
    println!(
        "rstar locate_in_envelope_intersecting count: {}",
        intersecting_count
    );
    assert_eq!(
        intersecting_count, 1,
        "rstar locate_in_envelope_intersecting successfully finds intersecting stroke"
    );
}
