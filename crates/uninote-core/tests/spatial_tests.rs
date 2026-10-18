//! Test suite for 2D Spatial Indexing R-Tree Engine in `uninote-core` (Milestone M2).
//!
//! Verifies:
//! - Bulk loading 10,000 items with OMT algorithm performance and accuracy
//! - Envelope query with > 90% culling ratio and strict ascending z-index ordering
//! - Dynamic insert, remove and lifecycle synchronization
//! - `Page.spatial_index` Serde skip serialization and rehydration
//! - `Document::rebuild_spatial_indices` across multi-page documents
//! - Degenerate and boundary coordinates (inverted, negative, infinite, NaN)

use std::time::Instant;
use uninote_core::model::{Document, Item, ItemKind, Page, PageBackground};
use uninote_core::spatial::{rect_to_aabb, PageSpatialIndex, SpatialItem};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

/// Helper to generate a stroke with known bounding box and point count.
fn create_test_stroke(id: u64, min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Stroke {
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
fn test_spatial_index_empty_and_default() {
    let empty_index = PageSpatialIndex::empty();
    assert_eq!(empty_index.len(), 0);
    assert!(empty_index.is_empty());

    let default_index = PageSpatialIndex::default();
    assert_eq!(default_index.len(), 0);
    assert!(default_index.is_empty());
    assert_eq!(empty_index, default_index);

    // Query on empty index returns empty vector
    let matches = empty_index.locate_in_envelope(0.0, 0.0, 500.0, 500.0);
    assert!(matches.is_empty());

    // Remove on empty index returns false
    let mut mut_index = empty_index;
    let dummy_rect = Rect::new(0.0, 0.0, 10.0, 10.0);
    assert!(!mut_index.remove(99, &dummy_rect));
}

#[test]
fn test_spatial_item_rtree_object_and_copy() {
    let rect = Rect::new(10.0, 20.0, 50.0, 80.0);
    let item = SpatialItem::from_rect(42, 100, &rect);

    assert_eq!(item.item_id, 42);
    assert_eq!(item.z_index, 100);

    // Test Copy semantics
    let item_copied = item;
    assert_eq!(item, item_copied);

    // Test non-finite value protection
    let nan_rect = Rect::new(f64::NAN, 10.0, 50.0, f64::INFINITY);
    let item_nan = SpatialItem::from_rect(7, 1, &nan_rect);
    assert_eq!(item_nan.item_id, 7);
    assert!(item_nan.aabb.lower()[0].is_finite());
    assert!(item_nan.aabb.lower()[1].is_finite());
}

#[test]
fn test_bulk_load_10000_items_performance_and_accuracy() {
    const N: usize = 10_000;
    let mut items = Vec::with_capacity(N);

    // Create 10,000 distinct items distributed uniformly in a 10,000 x 10,000 pt domain
    for i in 0..N {
        let x = (i % 100) as f64 * 100.0;
        let y = (i / 100) as f64 * 100.0;
        let stroke = create_test_stroke(i as u64, x, y, x + 30.0, y + 20.0);
        items.push(Item {
            id: i as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    let start = Instant::now();
    let index = PageSpatialIndex::bulk_load(&items);
    let elapsed = start.elapsed();

    // Verify all 10,000 items are indexed
    assert_eq!(index.len(), N);
    assert!(!index.is_empty());

    // Bulk loading 10,000 2D items with rstar OMT is highly optimized (typically 1-4 ms)
    println!("Bulk load of {} items completed in {:?}", N, elapsed);
    // Conservative threshold for unoptimized debug builds
    assert!(
        elapsed.as_millis() < 500,
        "Bulk load took too long: {} ms",
        elapsed.as_millis()
    );
}

#[test]
fn test_locate_in_envelope_culling_ratio_greater_than_90_percent() {
    const N: usize = 10_000;
    let mut items = Vec::with_capacity(N);

    // 10,000 items uniformly spread across a 100x100 grid (0.0 to 10,000.0 in X and Y)
    for i in 0..N {
        let gx = (i % 100) as f64;
        let gy = (i / 100) as f64;
        let x = gx * 100.0;
        let y = gy * 100.0;
        let stroke = create_test_stroke(i as u64, x, y, x + 25.0, y + 25.0);
        items.push(Item {
            id: i as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    let index = PageSpatialIndex::bulk_load(&items);

    // Query a viewport window of 500x500 pt located at [2000.0, 2000.0] to [2500.0, 2500.0]
    let query_min_x = 2000.0;
    let query_min_y = 2000.0;
    let query_max_x = 2500.0;
    let query_max_y = 2500.0;

    let visible = index.locate_in_envelope(query_min_x, query_min_y, query_max_x, query_max_y);

    let total = index.len();
    let culled_count = total - visible.len();
    let culled_pct = (culled_count as f64 / total as f64) * 100.0;

    println!(
        "Visible: {}, Culled: {} ({:.2}%)",
        visible.len(),
        culled_count,
        culled_pct
    );

    // Contractual requirement: culling ratio must exceed 90%
    assert!(
        culled_pct > 90.0,
        "Culling percentage {:.2}% was lower than required 90%",
        culled_pct
    );
    assert!(!visible.is_empty(), "Visible elements should not be empty");

    // Geometric assertion: every returned item must truly intersect the envelope
    for item in &visible {
        let lower = item.aabb.lower();
        let upper = item.aabb.upper();
        assert!(
            upper[0] >= query_min_x
                && lower[0] <= query_max_x
                && upper[1] >= query_min_y
                && lower[1] <= query_max_y,
            "Item {} with AABB {:?} does not intersect query envelope",
            item.item_id,
            item.aabb
        );
    }
}

#[test]
fn test_locate_in_envelope_z_index_ascending_ordering() {
    // Create items with intentionally disordered z-indexes
    let z_indices = [50u64, 12, 99, 1, 45, 8, 77, 23, 6, 100, 3];
    let mut items = Vec::new();

    for (idx, &z) in z_indices.iter().enumerate() {
        // Place all items overlapping within [10.0, 10.0] to [50.0, 50.0]
        let stroke = create_test_stroke(idx as u64, 10.0, 10.0, 40.0, 40.0);
        items.push(Item {
            id: idx as u64,
            z_index: z,
            kind: ItemKind::Stroke(stroke),
        });
    }

    let index = PageSpatialIndex::bulk_load(&items);
    let results = index.locate_in_envelope(0.0, 0.0, 100.0, 100.0);

    assert_eq!(results.len(), z_indices.len());

    // Verify strict ascending z-index ordering for natural Cairo drawing order
    for w in results.windows(2) {
        assert!(
            w[0].z_index <= w[1].z_index,
            "Spatial index results are not sorted by z_index ascending: {} > {}",
            w[0].z_index,
            w[1].z_index
        );
    }
}

#[test]
fn test_dynamic_insert_and_remove_lifecycle() {
    let mut index = PageSpatialIndex::empty();

    let s1 = create_test_stroke(1, 10.0, 10.0, 30.0, 30.0);
    let s2 = create_test_stroke(2, 50.0, 50.0, 70.0, 70.0);
    let s3 = create_test_stroke(3, 100.0, 100.0, 150.0, 150.0);

    let item1 = Item {
        id: 1,
        z_index: 10,
        kind: ItemKind::Stroke(s1),
    };
    let item2 = Item {
        id: 2,
        z_index: 20,
        kind: ItemKind::Stroke(s2),
    };
    let item3 = Item {
        id: 3,
        z_index: 30,
        kind: ItemKind::Stroke(s3),
    };

    index.insert(&item1);
    index.insert(&item2);
    index.insert(&item3);
    assert_eq!(index.len(), 3);

    // Query encompassing items 1 and 2
    let res1_2 = index.locate_in_envelope(0.0, 0.0, 80.0, 80.0);
    assert_eq!(res1_2.len(), 2);
    assert_eq!(res1_2[0].item_id, 1);
    assert_eq!(res1_2[1].item_id, 2);

    // Remove item 2
    let bbox2 = Rect::new(50.0, 50.0, 70.0, 70.0);
    assert!(index.remove(2, &bbox2));
    assert_eq!(index.len(), 2);

    // Query again: item 2 should be gone
    let res_after_remove = index.locate_in_envelope(0.0, 0.0, 80.0, 80.0);
    assert_eq!(res_after_remove.len(), 1);
    assert_eq!(res_after_remove[0].item_id, 1);

    // Removing already removed item returns false
    assert!(!index.remove(2, &bbox2));

    // Clear
    index.clear();
    assert_eq!(index.len(), 0);
    assert!(index.is_empty());
}

#[test]
fn test_page_serde_skip_spatial_index() {
    let mut page = Page::new_a4(0, PageBackground::default());
    let stroke = create_test_stroke(101, 50.0, 50.0, 120.0, 80.0);
    page.items.push(Item {
        id: 101,
        z_index: 1,
        kind: ItemKind::Stroke(stroke),
    });

    page.rebuild_spatial_index();
    assert_eq!(page.spatial_index.len(), 1);

    // Serialize Page to JSON
    let json_str = serde_json::to_string(&page).expect("Page serialization should succeed");

    // The field `spatial_index` must be skipped in JSON
    assert!(
        !json_str.contains("spatial_index"),
        "spatial_index should be excluded from serialized JSON but was found: {}",
        json_str
    );

    // Deserialize Page back from JSON
    let mut deserialized_page: Page =
        serde_json::from_str(&json_str).expect("Page deserialization should succeed");

    // Deserialized page starts with empty spatial_index due to #[serde(default)]
    assert_eq!(deserialized_page.spatial_index.len(), 0);
    assert!(deserialized_page.spatial_index.is_empty());

    // Calling rebuild_spatial_index rehydrates the spatial index
    deserialized_page.rebuild_spatial_index();
    assert_eq!(deserialized_page.spatial_index.len(), 1);
    let located = deserialized_page
        .spatial_index
        .locate_in_envelope(40.0, 40.0, 130.0, 90.0);
    assert_eq!(located.len(), 1);
    assert_eq!(located[0].item_id, 101);
}

#[test]
fn test_document_rebuild_spatial_indices() {
    let mut doc = Document::new_notebook("Test Document Rebuild");
    let stroke1 = create_test_stroke(1, 10.0, 10.0, 30.0, 30.0);
    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(stroke1),
    });

    let p2_id = doc.add_page(PageBackground::Blank);
    let p2 = doc.get_page_mut(p2_id).unwrap();
    let stroke2 = create_test_stroke(2, 50.0, 50.0, 90.0, 90.0);
    let stroke3 = create_test_stroke(3, 100.0, 100.0, 140.0, 140.0);
    p2.items.push(Item {
        id: 2,
        z_index: 1,
        kind: ItemKind::Stroke(stroke2),
    });
    p2.items.push(Item {
        id: 3,
        z_index: 2,
        kind: ItemKind::Stroke(stroke3),
    });

    // Rebuild all pages
    doc.rebuild_spatial_indices();

    assert_eq!(doc.pages[0].spatial_index.len(), 1);
    assert_eq!(doc.pages[1].spatial_index.len(), 2);
}

#[test]
fn test_envelope_query_with_inverted_or_infinite_coordinates() {
    let mut index = PageSpatialIndex::empty();
    let stroke = create_test_stroke(1, 100.0, 100.0, 200.0, 200.0);
    let item = Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(stroke),
    };
    index.insert(&item);

    // Inverted coordinates (min > max): should be automatically normalized
    let inverted = index.locate_in_envelope(250.0, 250.0, 50.0, 50.0);
    assert_eq!(inverted.len(), 1);
    assert_eq!(inverted[0].item_id, 1);

    // Infinite coordinates: should be safely clamped without panic
    let infinite = index.locate_in_envelope(
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    );
    assert_eq!(infinite.len(), 1);

    // NaN coordinates: handled without panic
    let nan_query = index.locate_in_envelope(f64::NAN, 50.0, 300.0, 300.0);
    assert_eq!(nan_query.len(), 1);
}

#[test]
fn test_locate_in_envelope_tie_break_by_item_id() {
    let mut items = Vec::new();
    // 5 items with identical z_index = 42, but unordered item_ids
    let item_ids = [99u64, 12, 105, 3, 50];
    for &id in &item_ids {
        let stroke = create_test_stroke(id, 10.0, 10.0, 20.0, 20.0);
        items.push(Item {
            id,
            z_index: 42,
            kind: ItemKind::Stroke(stroke),
        });
    }

    let index = PageSpatialIndex::bulk_load(&items);
    let results = index.locate_in_envelope(0.0, 0.0, 100.0, 100.0);

    assert_eq!(results.len(), 5);
    // Verifies deterministic ordering by item_id ascending when z_index is coincident
    for w in results.windows(2) {
        assert_eq!(w[0].z_index, 42);
        assert_eq!(w[1].z_index, 42);
        assert!(
            w[0].item_id < w[1].item_id,
            "Tie-break ordering failed: {} >= {}",
            w[0].item_id,
            w[1].item_id
        );
    }
}

#[test]
fn test_rect_to_aabb_astronomical_clamping() {
    // Coordinate exceeding normal domain (e.g. 1e20)
    let huge_rect = Rect::new(-1e20, -1e20, 1e20, 1e20);
    let aabb = rect_to_aabb(&huge_rect);
    assert_eq!(aabb.lower(), [-1e12, -1e12]);
    assert_eq!(aabb.upper(), [1e12, 1e12]);

    // Finite non-overflowing area
    let dx = aabb.upper()[0] - aabb.lower()[0];
    let dy = aabb.upper()[1] - aabb.lower()[1];
    let area = dx * dy;
    assert!(area.is_finite());
}

#[test]
fn test_locate_in_envelope_intersecting_vs_contained() {
    let mut index = PageSpatialIndex::empty();
    // Stroke spanning [0, 50] to [200, 50], bbox [-1, 49, 201, 51]
    let stroke = create_test_stroke(77, 0.0, 50.0, 200.0, 50.0);
    let item = Item {
        id: 77,
        z_index: 1,
        kind: ItemKind::Stroke(stroke),
    };
    index.insert(&item);

    // Viewport [50, 0] to [150, 100] intersects the stroke but does NOT strictly contain it
    let results = index.locate_in_envelope(50.0, 0.0, 150.0, 100.0);
    assert_eq!(
        results.len(),
        1,
        "Stroke intersecting the viewport boundary must be returned by locate_in_envelope"
    );
    assert_eq!(results[0].item_id, 77);
}
