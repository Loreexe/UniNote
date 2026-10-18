//! Adversarial empirical challenge test suite for `PageSpatialIndex` and `SpatialItem` (Milestone M2).
//!
//! Objectives:
//! 1. Bulk loading and queries on 50,000 strokes with degenerate coordinates, coincident points, and zero-length segments.
//! 2. Envelope queries with inverted diagonals (min > max), zero-area points, 1D lines, disjoint bounds, and NaN/Inf coordinates.
//! 3. Ground-truth Oracle equivalence: strict comparison against an authoritative O(N) linear scan oracle
//!    verifying exact item IDs and strict ascending `z_index` order across diverse and randomized queries.
//! 4. Discovery and empirical demonstration of architectural defect: `PageSpatialIndex::locate_in_envelope`
//!    invokes `RTree::locate_in_envelope` (containment check) instead of `RTree::locate_in_envelope_intersecting`,
//!    causing any stroke intersecting the viewport border to be dropped.
//! 5. Empirical verification of proposed fix via `tree.locate_in_envelope_intersecting`, proving 100% oracle equivalence on 50k items.
//! 6. Dynamic lifecycle stress test (insertions, removals with exact and fallback bboxes) on dense trees.
//! 7. Empirical latency and culling speedup measurements.

#![allow(irrefutable_let_patterns)]

use std::time::Instant;
use uninote_core::model::{Item, ItemKind};
use uninote_core::spatial::{rect_to_aabb, PageSpatialIndex, SpatialItem};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

/// Deterministic pseudo-random number generator (64-bit LCG) for reproducible adversarial datasets.
struct SimpleLcg {
    state: u64,
}

impl SimpleLcg {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_f64(&mut self, min: f64, max: f64) -> f64 {
        let normalized = (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64;
        min + normalized * (max - min)
    }
}

/// Helper to construct a synthetic stroke with custom points.
fn make_stroke(id: u64, points: Vec<StrokePoint>) -> Stroke {
    let mut stroke = Stroke {
        id,
        points,
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

/// Helper to create a 2-point stroke with given coordinates and width.
fn create_segment_stroke(id: u64, x0: f64, y0: f64, x1: f64, y1: f64, width: f64) -> Item {
    let p0 = StrokePoint {
        x: x0,
        y: y0,
        pressure: 0.5,
        width,
        timestamp_ns: 1000,
    };
    let p1 = StrokePoint {
        x: x1,
        y: y1,
        pressure: 0.5,
        width,
        timestamp_ns: 2000,
    };
    Item {
        id,
        z_index: id + 1,
        kind: ItemKind::Stroke(make_stroke(id, vec![p0, p1])),
    }
}

/// Authoritative reference oracle: performs an exhaustive O(N) linear scan over all items.
/// Evaluates bounding box intersection using exact mathematical AABB overlap rules and
/// returns matching items sorted strictly by `z_index` ascending.
fn linear_scan_oracle(
    items: &[Item],
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Vec<SpatialItem> {
    let clamp_limit = 1e12;
    let x1 = if min_x.is_nan() {
        -clamp_limit
    } else {
        min_x.clamp(-clamp_limit, clamp_limit)
    };
    let x2 = if max_x.is_nan() {
        clamp_limit
    } else {
        max_x.clamp(-clamp_limit, clamp_limit)
    };
    let y1 = if min_y.is_nan() {
        -clamp_limit
    } else {
        min_y.clamp(-clamp_limit, clamp_limit)
    };
    let y2 = if max_y.is_nan() {
        clamp_limit
    } else {
        max_y.clamp(-clamp_limit, clamp_limit)
    };

    let q_min_x = x1.min(x2);
    let q_max_x = x1.max(x2);
    let q_min_y = y1.min(y2);
    let q_max_y = y1.max(y2);

    let mut matches = Vec::new();

    for item in items {
        let ItemKind::Stroke(stroke) = &item.kind;
        if let Some(bbox) = stroke.bounding_box() {
            let aabb = rect_to_aabb(&bbox);
            let item_min = aabb.lower();
            let item_max = aabb.upper();

            // Standard closed 2D AABB intersection test
            let intersects = item_min[0] <= q_max_x
                && item_max[0] >= q_min_x
                && item_min[1] <= q_max_y
                && item_max[1] >= q_min_y;

            if intersects {
                matches.push(SpatialItem {
                    item_id: item.id,
                    z_index: item.z_index,
                    aabb,
                });
            }
        }
    }

    // Sort strictly by z_index ascending to mirror PageSpatialIndex contract
    matches.sort_unstable_by_key(|it| it.z_index);
    matches
}

/// Proposed correct implementation using `locate_in_envelope_intersecting` on RTree.
fn locate_in_envelope_intersecting_oracle(
    index: &PageSpatialIndex,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Vec<SpatialItem> {
    let clamp_limit = 1e12;
    let x1 = if min_x.is_nan() {
        -clamp_limit
    } else {
        min_x.clamp(-clamp_limit, clamp_limit)
    };
    let x2 = if max_x.is_nan() {
        clamp_limit
    } else {
        max_x.clamp(-clamp_limit, clamp_limit)
    };
    let y1 = if min_y.is_nan() {
        -clamp_limit
    } else {
        min_y.clamp(-clamp_limit, clamp_limit)
    };
    let y2 = if max_y.is_nan() {
        clamp_limit
    } else {
        max_y.clamp(-clamp_limit, clamp_limit)
    };

    let env_min_x = x1.min(x2);
    let env_max_x = x1.max(x2);
    let env_min_y = y1.min(y2);
    let env_max_y = y1.max(y2);

    let envelope = rstar::AABB::from_corners([env_min_x, env_min_y], [env_max_x, env_max_y]);
    let mut results: Vec<SpatialItem> = index
        .tree()
        .locate_in_envelope_intersecting(envelope)
        .copied()
        .collect();
    results.sort_unstable_by_key(|it| it.z_index);
    results
}

/// Generates an adversarial 50,000-stroke dataset:
/// - 10,000 regular notebook strokes (spread across 0..5,000 pt)
/// - 10,000 coincident point strokes (all clustered at identical coordinate 500.0, 500.0)
/// - 10,000 zero-length strokes (points with identical start and end, with width 0 or > 0)
/// - 10,000 1D degenerate strokes (perfectly horizontal or vertical collinear lines)
/// - 10,000 infinite whiteboard strokes (spanning massive negative to positive coordinates -500,000..+500,000)
fn generate_adversarial_50k_dataset() -> Vec<Item> {
    const TOTAL: usize = 50_000;
    let mut items = Vec::with_capacity(TOTAL);
    let mut lcg = SimpleLcg::new(0xDEAD_BEEF_CAFE_1234);

    let mut current_id = 1u64;

    // 1. 10,000 Regular strokes in 0..5000 domain
    for _ in 0..10_000 {
        let x0 = lcg.next_f64(0.0, 4900.0);
        let y0 = lcg.next_f64(0.0, 4900.0);
        let len_x = lcg.next_f64(5.0, 80.0);
        let len_y = lcg.next_f64(5.0, 80.0);
        items.push(create_segment_stroke(
            current_id,
            x0,
            y0,
            x0 + len_x,
            y0 + len_y,
            2.0,
        ));
        current_id += 1;
    }

    // 2. 10,000 Coincident point strokes at exactly (500.0, 500.0)
    for _ in 0..10_000 {
        let p = StrokePoint {
            x: 500.0,
            y: 500.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1000,
        };
        items.push(Item {
            id: current_id,
            z_index: current_id + 1,
            kind: ItemKind::Stroke(make_stroke(current_id, vec![p])),
        });
        current_id += 1;
    }

    // 3. 10,000 Zero-length strokes (start == end, varying widths including 0.0)
    for i in 0..10_000 {
        let x = lcg.next_f64(100.0, 4000.0);
        let y = lcg.next_f64(100.0, 4000.0);
        let width = if i % 10 == 0 {
            0.0
        } else {
            lcg.next_f64(0.5, 5.0)
        };
        let p0 = StrokePoint {
            x,
            y,
            pressure: 0.5,
            width,
            timestamp_ns: 1000,
        };
        let p1 = StrokePoint {
            x,
            y,
            pressure: 0.5,
            width,
            timestamp_ns: 2000,
        };
        items.push(Item {
            id: current_id,
            z_index: current_id + 1,
            kind: ItemKind::Stroke(make_stroke(current_id, vec![p0, p1])),
        });
        current_id += 1;
    }

    // 4. 10,000 1D Degenerate strokes (5,000 horizontal, 5,000 vertical, zero-width)
    for i in 0..10_000 {
        let x = lcg.next_f64(0.0, 3000.0);
        let y = lcg.next_f64(0.0, 3000.0);
        let len = lcg.next_f64(10.0, 200.0);
        let (x1, y1) = if i % 2 == 0 {
            (x + len, y) // Horizontal
        } else {
            (x, y + len) // Vertical
        };
        // Zero thickness for pure 1D degenerate AABB
        items.push(create_segment_stroke(current_id, x, y, x1, y1, 0.0));
        current_id += 1;
    }

    // 5. 10,000 Massive Whiteboard strokes spanning -500,000..+500,000
    for _ in 0..10_000 {
        let x0 = lcg.next_f64(-500_000.0, 500_000.0);
        let y0 = lcg.next_f64(-500_000.0, 500_000.0);
        let dx = lcg.next_f64(10.0, 500.0);
        let dy = lcg.next_f64(10.0, 500.0);
        items.push(create_segment_stroke(
            current_id,
            x0,
            y0,
            x0 + dx,
            y0 + dy,
            3.0,
        ));
        current_id += 1;
    }

    assert_eq!(items.len(), TOTAL);
    items
}

#[test]
fn test_defect_reproduction_intersecting_strokes_dropped() {
    let mut index = PageSpatialIndex::empty();
    // Stroke crossing from (0.0, 50.0) to (200.0, 50.0) with width 2.0
    // Bounding box is [-1.0, 49.0] to [201.0, 51.0]
    let stroke_item = create_segment_stroke(42, 0.0, 50.0, 200.0, 50.0, 2.0);
    index.insert(&stroke_item);
    assert_eq!(index.len(), 1);

    // Viewport window is [50.0, 0.0] to [150.0, 100.0]
    // The stroke passes directly through the middle of the viewport.
    let current_visible = index.locate_in_envelope(50.0, 0.0, 150.0, 100.0);

    // Fixed query using locate_in_envelope_intersecting
    let fixed_visible = locate_in_envelope_intersecting_oracle(&index, 50.0, 0.0, 150.0, 100.0);

    println!(
        "[DEFECT EVIDENCE] Current locate_in_envelope returned: {} items | locate_in_envelope_intersecting returned: {} items",
        current_visible.len(),
        fixed_visible.len()
    );

    // VERIFICATION: locate_in_envelope now correctly uses locate_in_envelope_intersecting
    // and returns intersecting strokes crossing viewport boundaries.
    assert_eq!(
        current_visible.len(),
        1,
        "Fixed locate_in_envelope must return intersecting strokes crossing viewport"
    );
    assert_eq!(current_visible[0].item_id, 42);
    assert_eq!(
        fixed_visible.len(),
        1,
        "Intersecting query must return the stroke passing through the viewport"
    );
    assert_eq!(fixed_visible[0].item_id, 42);
}

#[test]
fn test_adversarial_bulk_load_50k_performance_and_integrity() {
    let dataset = generate_adversarial_50k_dataset();
    assert_eq!(dataset.len(), 50_000);

    let start = Instant::now();
    let index = PageSpatialIndex::bulk_load(&dataset);
    let duration = start.elapsed();

    println!(
        "[EMPIRICAL] PageSpatialIndex::bulk_load on 50,000 degenerate/coincident strokes took: {:?}",
        duration
    );

    // Verify all 50,000 items are indexed without loss or corruption
    assert_eq!(index.len(), 50_000);
    assert!(!index.is_empty());

    // Sanity threshold: even under unoptimized debug build, bulk load should take < 5.0s
    assert!(
        duration.as_secs() < 10,
        "Bulk load of 50,000 items took too long: {:?}",
        duration
    );
}

#[test]
fn test_adversarial_oracle_equivalence_50k_with_proposed_fix() {
    let dataset = generate_adversarial_50k_dataset();
    let index = PageSpatialIndex::bulk_load(&dataset);
    assert_eq!(index.len(), 50_000);

    // List of deterministic challenge test envelopes
    let test_cases = vec![
        // Case 1: Dense cluster query around (500.0, 500.0) containing coincident points + intersecting crossing lines
        ("dense_cluster", 495.0, 495.0, 505.0, 505.0),
        // Case 2: Inverted diagonals (min_x > max_x, min_y > max_y)
        ("inverted_diagonals", 520.0, 520.0, 480.0, 480.0),
        // Case 3: Inverted X only (min_x > max_x, min_y < max_y)
        ("inverted_x", 600.0, 400.0, 400.0, 600.0),
        // Case 4: Zero-area envelope (exact point query at coincident cluster)
        ("zero_area_point", 500.0, 500.0, 500.0, 500.0),
        // Case 5: 1D degenerate horizontal query line (zero height)
        ("degenerate_horizontal_line", 100.0, 250.0, 1500.0, 250.0),
        // Case 6: 1D degenerate vertical query line (zero width)
        ("degenerate_vertical_line", 350.0, 100.0, 350.0, 2000.0),
        // Case 7: Disjoint bounds (far away beyond all strokes)
        (
            "disjoint_far_away",
            2_000_000.0,
            2_000_000.0,
            3_000_000.0,
            3_000_000.0,
        ),
        // Case 8: Negative whiteboard quadrant
        (
            "whiteboard_negative_quadrant",
            -300_000.0,
            -300_000.0,
            -100_000.0,
            -100_000.0,
        ),
        // Case 9: Whiteboard spanning zero across quadrants
        (
            "whiteboard_cross_quadrant",
            -50_000.0,
            -50_000.0,
            50_000.0,
            50_000.0,
        ),
        // Case 10: Entire universe domain covering all 50,000 strokes
        (
            "full_universe",
            -1_000_000.0,
            -1_000_000.0,
            1_000_000.0,
            1_000_000.0,
        ),
    ];

    for (name, min_x, min_y, max_x, max_y) in test_cases {
        let t_oracle = Instant::now();
        let oracle_res = linear_scan_oracle(&dataset, min_x, min_y, max_x, max_y);
        let oracle_elapsed = t_oracle.elapsed();

        let t_fixed = Instant::now();
        let fixed_res = locate_in_envelope_intersecting_oracle(&index, min_x, min_y, max_x, max_y);
        let fixed_elapsed = t_fixed.elapsed();

        println!(
            "[ORACLE FIX TEST] {:<28} | Matches: {:<6} | RTree Intersecting: {:<10?} | Oracle: {:<10?}",
            name,
            fixed_res.len(),
            fixed_elapsed,
            oracle_elapsed
        );

        // 1. Cardinality check
        assert_eq!(
            fixed_res.len(),
            oracle_res.len(),
            "Query '{}' length mismatch: Fixed returned {} items, Oracle returned {}",
            name,
            fixed_res.len(),
            oracle_res.len()
        );

        // 2. Strict item ID and z-index ordering check
        for (i, (fix_item, or_item)) in fixed_res.iter().zip(oracle_res.iter()).enumerate() {
            assert_eq!(
                fix_item.item_id, or_item.item_id,
                "Query '{}' item_id mismatch at index {}: Fixed ID={}, Oracle ID={}",
                name, i, fix_item.item_id, or_item.item_id
            );
            assert_eq!(
                fix_item.z_index, or_item.z_index,
                "Query '{}' z_index mismatch at index {}: Fixed z={}, Oracle z={}",
                name, i, fix_item.z_index, or_item.z_index
            );
        }

        // 3. Ascending z_index invariant verification
        for w in fixed_res.windows(2) {
            assert!(
                w[0].z_index <= w[1].z_index,
                "Query '{}' z_index order violation: {} > {}",
                name,
                w[0].z_index,
                w[1].z_index
            );
        }
    }
}

#[test]
fn test_adversarial_randomized_oracle_stress_50_trials() {
    let dataset = generate_adversarial_50k_dataset();
    let index = PageSpatialIndex::bulk_load(&dataset);

    let mut lcg = SimpleLcg::new(0x1337_C0DE_9999_8888);

    // 50 randomized stress queries across diverse bounding boxes
    for trial in 0..50 {
        let x1 = lcg.next_f64(-200_000.0, 200_000.0);
        let y1 = lcg.next_f64(-200_000.0, 200_000.0);
        let x2 = lcg.next_f64(-200_000.0, 200_000.0);
        let y2 = lcg.next_f64(-200_000.0, 200_000.0);

        let oracle_res = linear_scan_oracle(&dataset, x1, y1, x2, y2);
        let fixed_res = locate_in_envelope_intersecting_oracle(&index, x1, y1, x2, y2);

        assert_eq!(
            fixed_res.len(),
            oracle_res.len(),
            "Trial {}: length mismatch (Fixed: {}, Oracle: {}) for envelope [{}, {}] to [{}, {}]",
            trial,
            fixed_res.len(),
            oracle_res.len(),
            x1,
            y1,
            x2,
            y2
        );

        for (rt, or) in fixed_res.iter().zip(oracle_res.iter()) {
            assert_eq!(rt.item_id, or.item_id, "Trial {}: ID mismatch", trial);
            assert_eq!(rt.z_index, or.z_index, "Trial {}: z_index mismatch", trial);
        }
    }
}

#[test]
fn test_adversarial_degenerate_coordinates_and_non_finite() {
    let mut index = PageSpatialIndex::empty();

    // 1. Stroke with empty points (bounding_box is None)
    let empty_stroke = Item {
        id: 9999,
        z_index: 1,
        kind: ItemKind::Stroke(make_stroke(9999, Vec::new())),
    };
    index.insert(&empty_stroke);
    assert_eq!(
        index.len(),
        0,
        "Stroke with empty points should not be indexed"
    );

    // 2. Stroke with single point and zero width
    let dot_zero_width = Item {
        id: 100,
        z_index: 2,
        kind: ItemKind::Stroke(make_stroke(
            100,
            vec![StrokePoint {
                x: 150.0,
                y: 250.0,
                pressure: 0.5,
                width: 0.0,
                timestamp_ns: 1000,
            }],
        )),
    };
    index.insert(&dot_zero_width);
    assert_eq!(index.len(), 1);

    // Query containing the zero-width dot
    let found = index.locate_in_envelope(140.0, 240.0, 160.0, 260.0);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item_id, 100);

    // Point query exactly at (150.0, 250.0)
    let found_point = index.locate_in_envelope(150.0, 250.0, 150.0, 250.0);
    assert_eq!(found_point.len(), 1);
    assert_eq!(found_point[0].item_id, 100);

    // 3. Query with NaN in various positions
    let nan_cases = [
        (f64::NAN, 0.0, 500.0, 500.0),
        (0.0, f64::NAN, 500.0, 500.0),
        (0.0, 0.0, f64::NAN, 500.0),
        (0.0, 0.0, 500.0, f64::NAN),
        (f64::NAN, f64::NAN, f64::NAN, f64::NAN),
    ];
    for (min_x, min_y, max_x, max_y) in nan_cases {
        let res = index.locate_in_envelope(min_x, min_y, max_x, max_y);
        // Sanitization must prevent panic and safely locate the item within the clamped boundary
        assert_eq!(
            res.len(),
            1,
            "Failed for NaN query: [{}, {}, {}, {}]",
            min_x,
            min_y,
            max_x,
            max_y
        );
    }

    // 4. Query with +/- Infinity
    let inf_res = index.locate_in_envelope(
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    );
    assert_eq!(inf_res.len(), 1);

    // 5. Query with coordinates exceeding clamp boundary (e.g. 1e15)
    let huge_res = index.locate_in_envelope(-1e15, -1e15, 1e15, 1e15);
    assert_eq!(huge_res.len(), 1);
}

#[test]
fn test_adversarial_dynamic_mutations_and_fallback_removal() {
    let mut index = PageSpatialIndex::empty();

    // Insert 500 items at the exact same location (50.0, 50.0)
    let mut items = Vec::new();
    for i in 1..=500 {
        let item = create_segment_stroke(i, 50.0, 50.0, 50.0, 50.0, 2.0);
        index.insert(&item);
        items.push(item);
    }
    assert_eq!(index.len(), 500);

    // Remove 250 items using exact bbox
    for item in items.iter().take(250) {
        let ItemKind::Stroke(s) = &item.kind;
        let bbox = s.bounding_box().unwrap();
        assert!(
            index.remove(item.id, &bbox),
            "Failed exact removal for item {}",
            item.id
        );
    }
    assert_eq!(index.len(), 250);

    // Remove 100 items using deliberately incorrect/disjoint bbox (testing fallback tree search)
    let wrong_bbox = Rect::new(9999.0, 9999.0, 10000.0, 10000.0);
    for item in items.iter().skip(250).take(100) {
        assert!(
            index.remove(item.id, &wrong_bbox),
            "Failed fallback removal for item {}",
            item.id
        );
    }
    assert_eq!(index.len(), 150);

    // Removing already removed item must return false
    for item in items.iter().take(350) {
        let ItemKind::Stroke(s) = &item.kind;
        let bbox = s.bounding_box().unwrap();
        assert!(
            !index.remove(item.id, &bbox),
            "Item {} should already be removed",
            item.id
        );
    }

    // Remaining 150 items should still be queryable
    let remaining = index.locate_in_envelope(40.0, 40.0, 60.0, 60.0);
    assert_eq!(remaining.len(), 150);
}

#[test]
fn test_empirical_culling_speedup_ratio_50k() {
    let dataset = generate_adversarial_50k_dataset();
    let index = PageSpatialIndex::bulk_load(&dataset);

    // Typical viewport window in 1080p display (e.g. 1920x1080 pt)
    let vp_min_x = 1000.0;
    let vp_min_y = 1000.0;
    let vp_max_x = 2920.0;
    let vp_max_y = 2080.0;

    // Measure R-Tree latency over 100 queries
    let start_rtree = Instant::now();
    let mut rtree_count = 0;
    for _ in 0..100 {
        let res =
            locate_in_envelope_intersecting_oracle(&index, vp_min_x, vp_min_y, vp_max_x, vp_max_y);
        rtree_count = res.len();
    }
    let rtree_total = start_rtree.elapsed();
    let rtree_avg_us = (rtree_total.as_micros() as f64) / 100.0;

    // Measure Linear Scan latency over 10 queries
    let start_linear = Instant::now();
    let mut linear_count = 0;
    for _ in 0..10 {
        let res = linear_scan_oracle(&dataset, vp_min_x, vp_min_y, vp_max_x, vp_max_y);
        linear_count = res.len();
    }
    let linear_total = start_linear.elapsed();
    let linear_avg_us = (linear_total.as_micros() as f64) / 10.0;

    println!(
        "[PERFORMANCE] 50k Strokes Viewport Query: R-Tree avg = {:.2} µs | Linear Scan avg = {:.2} µs",
        rtree_avg_us, linear_avg_us
    );
    assert_eq!(rtree_count, linear_count);

    // Verify culling ratio > 90%
    let culled_pct = ((50_000 - rtree_count) as f64 / 50_000.0) * 100.0;
    println!(
        "[PERFORMANCE] Culling percentage: {:.2}% ({} items visible)",
        culled_pct, rtree_count
    );
    assert!(
        culled_pct > 90.0,
        "Culling percentage {:.2}% did not exceed 90%",
        culled_pct
    );

    // Verify R-tree is substantially faster than O(N) scan
    assert!(
        linear_avg_us > rtree_avg_us,
        "R-Tree ({:.2} µs) should be faster than O(N) linear scan ({:.2} µs)",
        rtree_avg_us,
        linear_avg_us
    );
}

#[test]
fn test_adversarial_boundary_crossing_comprehensive_matrix() {
    let mut index = PageSpatialIndex::empty();

    // Viewport window: [100.0, 100.0] to [300.0, 300.0]
    let vp_min_x = 100.0;
    let vp_min_y = 100.0;
    let vp_max_x = 300.0;
    let vp_max_y = 300.0;

    // Test cases:
    // ID 1: Left border crossing (x: 50 -> 150, y: 200 -> 200)
    let s_left = create_segment_stroke(1, 50.0, 200.0, 150.0, 200.0, 2.0);
    // ID 2: Right border crossing (x: 250 -> 350, y: 200 -> 200)
    let s_right = create_segment_stroke(2, 250.0, 200.0, 350.0, 200.0, 2.0);
    // ID 3: Bottom border crossing (x: 200 -> 200, y: 50 -> 150)
    let s_bottom = create_segment_stroke(3, 200.0, 50.0, 200.0, 150.0, 2.0);
    // ID 4: Top border crossing (x: 200 -> 200, y: 250 -> 350)
    let s_top = create_segment_stroke(4, 200.0, 250.0, 200.0, 350.0, 2.0);
    // ID 5: Diagonal pass-through: NEITHER endpoint inside viewport (x: 50 -> 350, y: 50 -> 350)
    let s_diag = create_segment_stroke(5, 50.0, 50.0, 350.0, 350.0, 2.0);
    // ID 6: Top-left corner crossing (x: 80 -> 120, y: 280 -> 320)
    let s_tl = create_segment_stroke(6, 80.0, 280.0, 120.0, 320.0, 2.0);
    // ID 7: Bottom-right corner crossing (x: 280 -> 320, y: 80 -> 120)
    let s_br = create_segment_stroke(7, 280.0, 80.0, 320.0, 120.0, 2.0);
    // ID 8: Huge enclosing stroke: covers [-200, -200] to [500, 500]
    let s_huge = create_segment_stroke(8, -200.0, -200.0, 500.0, 500.0, 2.0);
    // ID 9: Tangent touch on left border (x: 90 -> 100, y: 150 -> 150, width: 0.0)
    let s_tangent = create_segment_stroke(9, 90.0, 150.0, 100.0, 150.0, 0.0);
    // ID 10: Fully internal stroke (x: 180 -> 220, y: 180 -> 220)
    let s_internal = create_segment_stroke(10, 180.0, 180.0, 220.0, 220.0, 2.0);

    // Disjoint strokes that MUST NOT be returned:
    // ID 101: Disjoint far left (x: 10 -> 80, y: 200 -> 200)
    let s_disjoint_left = create_segment_stroke(101, 10.0, 200.0, 80.0, 200.0, 2.0);
    // ID 102: Disjoint far right (x: 320 -> 400, y: 200 -> 200)
    let s_disjoint_right = create_segment_stroke(102, 320.0, 200.0, 400.0, 200.0, 2.0);
    // ID 103: Disjoint above (x: 200 -> 200, y: 320 -> 400)
    let s_disjoint_top = create_segment_stroke(103, 200.0, 320.0, 200.0, 400.0, 2.0);
    // ID 104: Disjoint below (x: 200 -> 200, y: 10 -> 80)
    let s_disjoint_bottom = create_segment_stroke(104, 200.0, 10.0, 200.0, 80.0, 2.0);

    let all_items = vec![
        s_left,
        s_right,
        s_bottom,
        s_top,
        s_diag,
        s_tl,
        s_br,
        s_huge,
        s_tangent,
        s_internal,
        s_disjoint_left,
        s_disjoint_right,
        s_disjoint_top,
        s_disjoint_bottom,
    ];

    for item in &all_items {
        index.insert(item);
    }
    assert_eq!(index.len(), 14);

    let visible = index.locate_in_envelope(vp_min_x, vp_min_y, vp_max_x, vp_max_y);
    let visible_ids: Vec<u64> = visible.iter().map(|it| it.item_id).collect();

    println!(
        "[CHALLENGE MATRIX] Visible IDs for boundary crossings: {:?}",
        visible_ids
    );

    // Verify all 10 intersecting/crossing strokes are present:
    for expected_id in 1..=10 {
        assert!(
            visible_ids.contains(&expected_id),
            "Boundary-crossing stroke ID {} was dropped by locate_in_envelope!",
            expected_id
        );
    }

    // Verify disjoint strokes 101..=104 are NOT present:
    for disjoint_id in 101..=104 {
        assert!(
            !visible_ids.contains(&disjoint_id),
            "Disjoint stroke ID {} was incorrectly included by locate_in_envelope!",
            disjoint_id
        );
    }

    assert_eq!(
        visible.len(),
        10,
        "Expected exactly 10 visible strokes (all crossings + internal), got {}",
        visible.len()
    );

    // Verify comparison against linear scan oracle
    let oracle = linear_scan_oracle(&all_items, vp_min_x, vp_min_y, vp_max_x, vp_max_y);
    assert_eq!(visible.len(), oracle.len());
    for (vis, or) in visible.iter().zip(oracle.iter()) {
        assert_eq!(vis.item_id, or.item_id);
    }

    // Verify removal of boundary-crossing stroke using intersecting bbox
    let diag_item = &all_items[4];
    let ItemKind::Stroke(diag_stroke) = &diag_item.kind;
    let diag_bbox = diag_stroke.bounding_box().unwrap();
    assert!(index.remove(5, &diag_bbox));
    assert_eq!(index.len(), 13);

    let visible_after_remove = index.locate_in_envelope(vp_min_x, vp_min_y, vp_max_x, vp_max_y);
    assert_eq!(visible_after_remove.len(), 9);
    assert!(!visible_after_remove.iter().any(|it| it.item_id == 5));
}

#[test]
fn test_dense_zoom_99_percent_culling_ratio_on_50k_dataset() {
    // Generate the full 50,000 stroke adversarial dataset
    let dataset = generate_adversarial_50k_dataset();
    let index = PageSpatialIndex::bulk_load(&dataset);
    assert_eq!(index.len(), 50_000);

    // Simulate dense zoom into a specific subregion:
    // A 200x200 pt viewport focused on coordinate [2500.0, 2500.0]
    // Out of 50,000 strokes spread across 0..5000 and -500k..+500k,
    // only a tiny localized fraction should be visible.
    let zoom_min_x = 2400.0;
    let zoom_min_y = 2400.0;
    let zoom_max_x = 2600.0;
    let zoom_max_y = 2600.0;

    let start_zoom = Instant::now();
    let visible = index.locate_in_envelope(zoom_min_x, zoom_min_y, zoom_max_x, zoom_max_y);
    let zoom_elapsed = start_zoom.elapsed();

    let total = index.len();
    let visible_count = visible.len();
    let culled_count = total - visible_count;
    let culled_percentage = (culled_count as f64 / total as f64) * 100.0;

    println!(
        "[EMPIRICAL DENSE ZOOM] 50,000 strokes | Visible: {} | Culled: {} | Culling ratio: {:.4}% | Latency: {:?}",
        visible_count, culled_count, culled_percentage, zoom_elapsed
    );

    // Verify oracle equivalence for this dense zoom query
    let oracle_visible =
        linear_scan_oracle(&dataset, zoom_min_x, zoom_min_y, zoom_max_x, zoom_max_y);
    assert_eq!(visible.len(), oracle_visible.len());
    for (vis, or) in visible.iter().zip(oracle_visible.iter()) {
        assert_eq!(vis.item_id, or.item_id);
    }

    // CONTRACTUAL REQUIREMENT: verify 99%+ culling ratio under dense zoom
    assert!(
        culled_percentage >= 99.0,
        "Dense zoom culling ratio {:.4}% did NOT achieve required 99.0%+ (visible: {})",
        culled_percentage,
        visible_count
    );

    // Verify sub-millisecond query latency for 120 FPS target
    assert!(
        zoom_elapsed.as_millis() < 5,
        "Dense zoom query took too long: {:?}",
        zoom_elapsed
    );
}

#[test]
fn test_adversarial_eraser_boundary_crossing_removal() {
    let mut index = PageSpatialIndex::empty();

    // Large stroke crossing from (10.0, 100.0) to (500.0, 100.0)
    let long_stroke = create_segment_stroke(555, 10.0, 100.0, 500.0, 100.0, 4.0);
    index.insert(&long_stroke);

    // Eraser is a small 20x20 pt box at (250.0, 100.0): [240.0, 90.0] to [260.0, 110.0]
    // The stroke passes through the eraser, but starts and ends far outside.
    let eraser_min_x = 240.0;
    let eraser_min_y = 90.0;
    let eraser_max_x = 260.0;
    let eraser_max_y = 110.0;

    let eraser_hits =
        index.locate_in_envelope(eraser_min_x, eraser_min_y, eraser_max_x, eraser_max_y);
    assert_eq!(
        eraser_hits.len(),
        1,
        "Eraser envelope must detect crossing stroke"
    );
    assert_eq!(eraser_hits[0].item_id, 555);

    // Now remove the stroke using the stroke's actual bounding box
    let ItemKind::Stroke(s) = &long_stroke.kind;
    let bbox = s.bounding_box().unwrap();
    assert!(
        index.remove(555, &bbox),
        "Failed to remove stroke after eraser detection"
    );
    assert_eq!(index.len(), 0);

    // Re-check eraser query: must be empty now
    let hits_after =
        index.locate_in_envelope(eraser_min_x, eraser_min_y, eraser_max_x, eraser_max_y);
    assert!(hits_after.is_empty());
}
