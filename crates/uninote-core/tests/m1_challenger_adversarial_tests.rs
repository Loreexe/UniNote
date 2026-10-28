//! Adversarial Stress & Chaos Test Suite for UniNote Phase 4D Milestone M1
//!
//! Authored by: challenger_m1_1 (Empirical Challenger)
//! Scope: crates/uninote-core (ImageItem, Spatial Indexing, Undo/Redo Engine)
//!
//! Rigorous empirical stress-testing:
//! 1. Extreme image bounding boxes: negative coordinates, astronomical coordinates,
//!    zero dimensions, non-finite values (NaN, +Inf, -Inf).
//! 2. Mathematical reversibility of proportional & non-proportional scaling
//!    over 1000 transform iterations with epsilon drift tracking.
//! 3. Spatial indexing stress on 10,000 mixed items (strokes and images), bulk loading
//!    vs dynamic inserts/removals, and strict (z_index, item_id) Painter's Algorithm ordering.
//! 4. Undo/Redo stress: 500 consecutive operations (AddImage, MoveItems, TransformItems)
//!    with 100% roundtrip state recovery and memory/time profiling.

use std::collections::HashSet;
use std::time::Instant;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, ImageItem, Item, ItemId};
use uninote_core::spatial::{rect_to_aabb, PageSpatialIndex};
use uninote_stroke::types::{Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

/// Helper: Deterministic PRNG (Xorshift64) to avoid external rand dependencies in core.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xdeadbeefcafebabe } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + self.next_f64() * (max - min)
    }

    fn range_u64(&mut self, min: u64, max: u64) -> u64 {
        min + (self.next_u64() % (max - min + 1))
    }
}

/// Helper: Create a realistic stroke item.
fn create_test_stroke(
    id: ItemId,
    z_index: u64,
    min_x: f64,
    min_y: f64,
    width: f64,
    height: f64,
) -> Item {
    let mut stroke = Stroke::new(
        id,
        StrokeStyle::default(),
        uninote_stroke::types::PipelineMode::CatmullRomSpline,
    );
    let n_pts = 8;
    for i in 0..n_pts {
        let t = i as f64 / (n_pts - 1) as f64;
        let x = min_x + t * width;
        let y = min_y + (t * std::f64::consts::PI).sin() * height;
        stroke.points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: (i as u64 + 1) * 1000,
        });
    }
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

/// Helper: Create an ImageItem item.
#[allow(clippy::too_many_arguments)]
fn create_test_image(
    id: ItemId,
    page_id: Uuid,
    z_index: u64,
    min_x: f64,
    min_y: f64,
    width: f64,
    height: f64,
    hash: &str,
) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        hash.to_string(),
        Rect::new(min_x, min_y, min_x + width, min_y + height),
        z_index as i64,
        1.0,
        width.max(1.0).round() as u32,
        height.max(1.0).round() as u32,
    );
    Item::new_image(id, z_index, img)
}

// =========================================================================
// AREA 1: Extreme Image Bounding Boxes & Non-Finite Safeguards
// =========================================================================

#[test]
fn test_adversarial_negative_and_astronomical_bounding_boxes() {
    let page_id = Uuid::new_v4();

    // 1. Fully negative bounding box
    let neg_img = ImageItem::new(
        page_id,
        "neg_hash",
        Rect::new(-50000.0, -40000.0, -10000.0, -20000.0),
        400,
        200,
    );
    assert_eq!(neg_img.width(), 40000.0);
    assert_eq!(neg_img.height(), 20000.0);
    assert!((neg_img.aspect_ratio() - 2.0).abs() < 1e-9);

    // Spatial indexing of negative coordinates
    let item_neg = Item::new_image(1, 10, neg_img.clone());
    let mut index = PageSpatialIndex::empty();
    index.insert(&item_neg);
    assert_eq!(index.len(), 1);

    // Negative query envelope
    let hits = index.locate_in_envelope(-60000.0, -50000.0, -5000.0, -10000.0);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item_id, 1);

    // 2. Astronomical coordinates (> 1e12 clamping limit in spatial.rs)
    let astro_rect = Rect::new(1e15, 2e15, 1e15 + 500.0, 2e15 + 300.0);
    let astro_aabb = rect_to_aabb(&astro_rect);
    // Verified clamping to clamp_limit (1e12)
    assert_eq!(astro_aabb.lower(), [1e12, 1e12]);
    assert_eq!(astro_aabb.upper(), [1e12, 1e12]);

    let astro_img = ImageItem::new(page_id, "astro_hash", astro_rect, 500, 300);
    let item_astro = Item::new_image(2, 20, astro_img);
    index.insert(&item_astro);
    assert_eq!(index.len(), 2);

    // 3. Negative-to-positive straddling coordinates
    let straddle_img = ImageItem::new(
        page_id,
        "straddle_hash",
        Rect::new(-1000.0, -1000.0, 1000.0, 1000.0),
        2000,
        2000,
    );
    assert_eq!(straddle_img.width(), 2000.0);
    assert_eq!(straddle_img.height(), 2000.0);
    assert!((straddle_img.aspect_ratio() - 1.0).abs() < 1e-9);
}

#[test]
fn test_adversarial_zero_dimension_and_degenerate_bounding_boxes() {
    let page_id = Uuid::new_v4();

    // 1. Zero width, zero height point
    let point_img = ImageItem::new(
        page_id,
        "point_hash",
        Rect::new(100.0, 200.0, 100.0, 200.0),
        0,
        0,
    );
    assert_eq!(point_img.width(), 0.0);
    assert_eq!(point_img.height(), 0.0);
    assert_eq!(point_img.aspect_ratio(), 1.0); // Safe fallback to 1.0

    // Spatial indexing of zero-area point
    let item_point = Item::new_image(10, 5, point_img);
    let mut index = PageSpatialIndex::empty();
    index.insert(&item_point);
    assert_eq!(index.len(), 1);

    // Locate envelope containing point
    let hits = index.locate_in_envelope(99.0, 199.0, 101.0, 201.0);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item_id, 10);

    // Locate envelope disjoint
    let disjoint_hits = index.locate_in_envelope(101.0, 201.0, 110.0, 210.0);
    assert_eq!(disjoint_hits.len(), 0);

    // 2. Inverted bounds: min > max (should be normalized by Rect)
    let inverted_rect = Rect::new(300.0, 400.0, 100.0, 200.0);
    let norm = inverted_rect.normalized();
    assert_eq!(norm.min_x, 100.0);
    assert_eq!(norm.min_y, 200.0);
    assert_eq!(norm.max_x, 300.0);
    assert_eq!(norm.max_y, 400.0);

    let norm_aabb = rect_to_aabb(&inverted_rect);
    assert_eq!(norm_aabb.lower(), [100.0, 200.0]);
    assert_eq!(norm_aabb.upper(), [300.0, 400.0]);
}

#[test]
fn test_adversarial_non_finite_inputs_no_panic_or_corruption() {
    let page_id = Uuid::new_v4();
    let mut img = ImageItem::new(
        page_id,
        "nan_hash",
        Rect::new(10.0, 20.0, 110.0, 120.0),
        100,
        100,
    );

    // 1. NaN, Inf, -Inf in translate() must be silent no-ops
    img.translate(f64::NAN, 5.0);
    assert_eq!(img.bbox.min_x, 10.0);
    assert_eq!(img.bbox.min_y, 20.0);

    img.translate(5.0, f64::INFINITY);
    assert_eq!(img.bbox.min_x, 10.0);
    assert_eq!(img.bbox.min_y, 20.0);

    img.translate(f64::NEG_INFINITY, f64::NAN);
    assert_eq!(img.bbox.min_x, 10.0);
    assert_eq!(img.bbox.min_y, 20.0);

    // 2. NaN, Inf in scale() must be silent no-ops
    img.scale(f64::NAN, 1.0, 0.0, 0.0);
    assert_eq!(img.bbox.width(), 100.0);

    img.scale(1.0, f64::INFINITY, 0.0, 0.0);
    assert_eq!(img.bbox.height(), 100.0);

    img.scale(1.0, 1.0, f64::NAN, 0.0);
    assert_eq!(img.bbox.min_x, 10.0);

    // 3. Opacity clamping with NaN and non-finite
    img.set_opacity(f64::NAN);
    assert_eq!(img.opacity, 1.0); // Non-finite NaN is no-op

    img.set_opacity(f64::INFINITY);
    assert_eq!(img.opacity, 1.0); // Non-finite Inf is no-op

    img.set_opacity(f64::NEG_INFINITY);
    assert_eq!(img.opacity, 1.0); // Non-finite -Inf is no-op

    // Valid finite out-of-range values clamp properly
    img.set_opacity(1.5);
    assert_eq!(img.opacity, 1.0);

    img.set_opacity(-0.5);
    assert_eq!(img.opacity, 0.0);

    // 4. rect_to_aabb with NaN
    let nan_rect = Rect::new(f64::NAN, f64::NAN, f64::NAN, f64::NAN);
    let nan_aabb = rect_to_aabb(&nan_rect);
    assert_eq!(nan_aabb.lower(), [0.0, 0.0]);
    assert_eq!(nan_aabb.upper(), [0.0, 0.0]);

    // 5. Spatial index query with NaN envelope coordinates
    let item = Item::new_image(99, 1, img);
    let mut index = PageSpatialIndex::empty();
    index.insert(&item);

    let nan_query = index.locate_in_envelope(f64::NAN, 0.0, 200.0, 200.0);
    assert_eq!(nan_query.len(), 1); // Should clamp NaN to envelope limits safely
}

// =========================================================================
// AREA 2: Scaling Mathematical Reversibility & Epsilon Drift
// =========================================================================

#[test]
fn test_proportional_scaling_reversibility_1000_iterations() {
    let page_id = Uuid::new_v4();
    let initial_rect = Rect::new(123.456, 789.012, 523.456, 1189.012); // w=400, h=400
    let mut img = ImageItem::new(page_id, "prop_drift", initial_rect, 400, 400);

    let origin_x = 123.456;
    let origin_y = 789.012;
    let scale_factor = 1.00314159265;
    let inv_factor = 1.0 / scale_factor;

    let start_time = Instant::now();
    for i in 0..1000 {
        // Alternating scale up and down
        if i % 2 == 0 {
            img.scale_proportional(scale_factor, origin_x, origin_y);
        } else {
            img.scale_proportional(inv_factor, origin_x, origin_y);
        }
    }
    let elapsed = start_time.elapsed();
    println!("1000 proportional scale iterations elapsed: {:?}", elapsed);

    // After 1000 iterations (500 up, 500 down), check epsilon drift
    let drift_x = (img.bbox.min_x - initial_rect.min_x).abs();
    let drift_y = (img.bbox.min_y - initial_rect.min_y).abs();
    let drift_w = (img.bbox.width() - initial_rect.width()).abs();
    let drift_h = (img.bbox.height() - initial_rect.height()).abs();
    let drift_aspect = (img.aspect_ratio() - 1.0).abs();

    println!(
        "Proportional drift: dx={:e}, dy={:e}, dw={:e}, dh={:e}, d_aspect={:e}",
        drift_x, drift_y, drift_w, drift_h, drift_aspect
    );

    // Floating-point drift across 1,000 steps must be strictly bounded under 1e-8
    assert!(drift_x < 1e-8, "X drift exceeded: {:e}", drift_x);
    assert!(drift_y < 1e-8, "Y drift exceeded: {:e}", drift_y);
    assert!(drift_w < 1e-8, "Width drift exceeded: {:e}", drift_w);
    assert!(drift_h < 1e-8, "Height drift exceeded: {:e}", drift_h);
    assert!(
        drift_aspect < 1e-12,
        "Aspect ratio drift exceeded: {:e}",
        drift_aspect
    );
}

#[test]
fn test_anisotropic_scaling_reversibility_1000_iterations() {
    let page_id = Uuid::new_v4();
    let initial_rect = Rect::new(200.0, 150.0, 600.0, 450.0); // w=400, h=300, aspect=4/3
    let mut img = ImageItem::new(page_id, "aniso_drift", initial_rect, 400, 300);

    let origin_x = 400.0; // Center origin
    let origin_y = 300.0;
    let sx = 1.0025;
    let sy = 0.9975;
    let inv_sx = 1.0 / sx;
    let inv_sy = 1.0 / sy;

    for i in 0..1000 {
        if i % 2 == 0 {
            img.scale(sx, sy, origin_x, origin_y);
        } else {
            img.scale(inv_sx, inv_sy, origin_x, origin_y);
        }
    }

    let drift_min_x = (img.bbox.min_x - initial_rect.min_x).abs();
    let drift_min_y = (img.bbox.min_y - initial_rect.min_y).abs();
    let drift_max_x = (img.bbox.max_x - initial_rect.max_x).abs();
    let drift_max_y = (img.bbox.max_y - initial_rect.max_y).abs();

    println!(
        "Anisotropic drift: d_min_x={:e}, d_min_y={:e}, d_max_x={:e}, d_max_y={:e}",
        drift_min_x, drift_min_y, drift_max_x, drift_max_y
    );

    assert!(
        drift_min_x < 1e-8,
        "min_x drift exceeded: {:e}",
        drift_min_x
    );
    assert!(
        drift_min_y < 1e-8,
        "min_y drift exceeded: {:e}",
        drift_min_y
    );
    assert!(
        drift_max_x < 1e-8,
        "max_x drift exceeded: {:e}",
        drift_max_x
    );
    assert!(
        drift_max_y < 1e-8,
        "max_y drift exceeded: {:e}",
        drift_max_y
    );
}

#[test]
fn test_reflection_scaling_reversibility_500_cycles() {
    let page_id = Uuid::new_v4();
    let initial_rect = Rect::new(100.0, 150.0, 300.0, 250.0);
    let mut img = ImageItem::new(page_id, "reflection_reversibility", initial_rect, 200, 100);

    let origin_x = 200.0;
    let origin_y = 200.0;

    // Flip with negative scale factors and reverse
    for _ in 0..500 {
        img.scale(-1.25, -1.5, origin_x, origin_y);
        img.scale(-1.0 / 1.25, -1.0 / 1.5, origin_x, origin_y);
    }

    let drift_x = (img.bbox.min_x - initial_rect.min_x).abs();
    let drift_y = (img.bbox.min_y - initial_rect.min_y).abs();
    let drift_w = (img.bbox.width() - initial_rect.width()).abs();
    let drift_h = (img.bbox.height() - initial_rect.height()).abs();

    println!(
        "Reflection reversibility drift: dx={:e}, dy={:e}, dw={:e}, dh={:e}",
        drift_x, drift_y, drift_w, drift_h
    );

    assert!(drift_x < 1e-8, "X reflection drift exceeded: {:e}", drift_x);
    assert!(drift_y < 1e-8, "Y reflection drift exceeded: {:e}", drift_y);
    assert!(
        drift_w < 1e-8,
        "Width reflection drift exceeded: {:e}",
        drift_w
    );
    assert!(
        drift_h < 1e-8,
        "Height reflection drift exceeded: {:e}",
        drift_h
    );
}

// =========================================================================
// AREA 3: Spatial Indexing Stress (10,000 Mixed Items) & Ordering Invariant
// =========================================================================

#[test]
fn test_spatial_index_10000_mixed_items_stress_and_ordering() {
    let mut rng = SimpleRng::new(0x2026_1027_f4d0_a100);
    let total_items = 10_000;
    let page_id = Uuid::new_v4();

    println!("Generating 10,000 mixed items (5,000 strokes + 5,000 images)...");
    let mut items = Vec::with_capacity(total_items);

    for id in 1..=total_items as u64 {
        let z_index = rng.range_u64(1, 20_000);
        let min_x = rng.range_f64(-5000.0, 50000.0);
        let min_y = rng.range_f64(-5000.0, 50000.0);
        let width = rng.range_f64(10.0, 500.0);
        let height = rng.range_f64(10.0, 500.0);

        if id % 2 == 0 {
            // Stroke item
            let stroke_item = create_test_stroke(id, z_index, min_x, min_y, width, height);
            items.push(stroke_item);
        } else {
            // Image item
            let img_item = create_test_image(
                id,
                page_id,
                z_index,
                min_x,
                min_y,
                width,
                height,
                &format!("hash_{}", id),
            );
            items.push(img_item);
        }
    }
    assert_eq!(items.len(), 10_000);

    // 1. Bulk loading stress & timing
    let start_bulk = Instant::now();
    let bulk_index = PageSpatialIndex::bulk_load(&items);
    let bulk_time = start_bulk.elapsed();
    println!("Bulk loaded 10,000 items in {:?}", bulk_time);
    assert_eq!(bulk_index.len(), 10_000);

    // 2. Dynamic insert stress & timing
    let start_dyn = Instant::now();
    let mut dynamic_index = PageSpatialIndex::empty();
    for item in &items {
        dynamic_index.insert(item);
    }
    let dyn_time = start_dyn.elapsed();
    println!("Dynamically inserted 10,000 items in {:?}", dyn_time);
    assert_eq!(dynamic_index.len(), 10_000);

    // 3. Dynamic removal stress: remove 3,000 items (mix of strokes and images)
    let start_removal = Instant::now();
    let mut removed_ids = HashSet::new();
    for i in (0..total_items).step_by(3) {
        let it = &items[i];
        assert!(
            dynamic_index.remove_item(it),
            "Item {} should be removed",
            it.id
        );
        removed_ids.insert(it.id);
    }
    let removal_time = start_removal.elapsed();
    println!(
        "Removed {} items in {:?}; remaining len={}",
        removed_ids.len(),
        removal_time,
        dynamic_index.len()
    );
    assert_eq!(dynamic_index.len(), 10_000 - removed_ids.len());

    // Re-removing already removed items should return false and not corrupt
    for &id in removed_ids.iter().take(50) {
        let dummy_rect = Rect::new(0.0, 0.0, 1.0, 1.0);
        assert!(!dynamic_index.remove(id, &dummy_rect));
    }
    assert_eq!(dynamic_index.len(), 10_000 - removed_ids.len());

    // 4. Strict Ordering Invariant (z_index, item_id) Verification on both indices
    let test_envelopes = vec![
        (-1000.0, -1000.0, 5000.0, 5000.0),   // Corner envelope
        (10000.0, 10000.0, 15000.0, 15000.0), // Dense interior region
        (20000.0, 0.0, 20500.0, 50000.0),     // Narrow vertical strip
        (-6000.0, -6000.0, 60000.0, 60000.0), // Enclosing entire canvas
        (60000.0, 60000.0, 70000.0, 70000.0), // Completely empty whitespace
    ];

    for (min_x, min_y, max_x, max_y) in test_envelopes {
        let bulk_res = bulk_index.locate_in_envelope(min_x, min_y, max_x, max_y);
        let dyn_res = dynamic_index.locate_in_envelope(min_x, min_y, max_x, max_y);

        // Verify strictly ascending (z_index, item_id) order for bulk results
        for w in bulk_res.windows(2) {
            let a = &w[0];
            let b = &w[1];
            let ordered =
                a.z_index < b.z_index || (a.z_index == b.z_index && a.item_id <= b.item_id);
            assert!(
                ordered,
                "Bulk ordering violated: a=(z={}, id={}), b=(z={}, id={})",
                a.z_index, a.item_id, b.z_index, b.item_id
            );
        }

        // Verify strictly ascending (z_index, item_id) order for dynamic results
        for w in dyn_res.windows(2) {
            let a = &w[0];
            let b = &w[1];
            let ordered =
                a.z_index < b.z_index || (a.z_index == b.z_index && a.item_id <= b.item_id);
            assert!(
                ordered,
                "Dynamic ordering violated: a=(z={}, id={}), b=(z={}, id={})",
                a.z_index, a.item_id, b.z_index, b.item_id
            );
        }

        // Ground-truth verification against linear scan for bulk results
        let query_rect = Rect::new(min_x, min_y, max_x, max_y);
        let mut ground_truth: Vec<(u64, u64)> = items
            .iter()
            .filter_map(|it| {
                it.bounding_box().and_then(|bbox| {
                    if bbox.intersects_rect(&query_rect) {
                        Some((it.z_index, it.id))
                    } else {
                        None
                    }
                })
            })
            .collect();
        ground_truth.sort_unstable();

        let bulk_ids: Vec<(u64, u64)> =
            bulk_res.iter().map(|it| (it.z_index, it.item_id)).collect();
        assert_eq!(
            bulk_ids.len(),
            ground_truth.len(),
            "Mismatch in result count for envelope {:?}",
            query_rect
        );
        assert_eq!(
            bulk_ids, ground_truth,
            "Ground truth vs bulk_load index mismatch"
        );
    }
}

// =========================================================================
// AREA 4: Undo/Redo Stress (500 Consecutive Operations) & State Recovery
// =========================================================================

/// EMPIRICAL BUG REPRODUCTION:
/// Demonstrates that when `AddImage` is followed by operations targeting that item
/// (e.g. `MoveItems`), and then Undo + Redo is performed:
/// `AddImage` re-allocates a different `item_id` upon Redo, causing downstream
/// commands (`MoveItems`) in the Redo stack to fail silently because they cannot find the item.
#[test]
fn test_empirical_bug_add_image_redo_id_drift_and_command_desync() {
    let mut doc = Document::new_notebook("Reproducer Doc");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    // 1. Add Image
    let img = ImageItem::new(
        page_id,
        "hash_repro",
        Rect::new(10.0, 10.0, 110.0, 110.0),
        100,
        100,
    );
    let item_id = history.alloc_item_id();
    history.execute(DocCommand::add_image(page_id, item_id, 1, img), &mut doc);
    assert_eq!(doc.pages[0].items.len(), 1);
    let original_item_id = doc.pages[0].items[0].id;
    assert_eq!(original_item_id, item_id);
    println!(
        "Step 1 (AddImage): item allocated with id={}",
        original_item_id
    );

    // 2. Move that image
    let move_cmd = DocCommand::MoveItems {
        page_id,
        item_ids: vec![original_item_id],
        delta_x: 50.0,
        delta_y: 50.0,
    };
    history.execute(move_cmd, &mut doc);
    assert_eq!(
        doc.pages[0].items[0].bounding_box().unwrap().min_x,
        60.0,
        "Image should have moved by +50px"
    );

    // 3. Undo Move
    assert!(history.undo(&mut doc));
    assert_eq!(
        doc.pages[0].items[0].bounding_box().unwrap().min_x,
        10.0,
        "Image should be back at x=10 after undo move"
    );

    // 4. Undo AddImage
    assert!(history.undo(&mut doc));
    assert_eq!(
        doc.pages[0].items.len(),
        0,
        "Page should be empty after undo AddImage"
    );

    // 5. Redo AddImage
    assert!(history.redo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 1);
    let redone_item_id = doc.pages[0].items[0].id;
    println!(
        "Step 5 (Redo AddImage): redone item allocated with id={}",
        redone_item_id
    );

    // CRITICAL ASSERTION: The redone item MUST have the same item_id as originally allocated,
    // otherwise subsequent commands in the redo stack that reference original_item_id will fail!
    assert_eq!(
        redone_item_id, original_item_id,
        "Redone item got new id {} instead of original id {}",
        redone_item_id, original_item_id
    );

    // 6. Redo Move
    assert!(history.redo(&mut doc));
    let final_min_x = doc.pages[0].items[0].bounding_box().unwrap().min_x;
    println!("Step 6 (Redo MoveItems): final min_x={}", final_min_x);
    assert_eq!(
        final_min_x, 60.0,
        "MoveItems during Redo must succeed because redone item preserved item_id"
    );
}

#[test]
fn test_undo_redo_500_add_image_lifecycle() {
    let mut rng = SimpleRng::new(0x4444_5555_6666_7777);
    let mut doc = Document::new_notebook("UndoRedo 500 AddImage");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(1000);

    // Initial base state: 10 strokes
    for id in 1..=10 {
        let stroke = create_test_stroke(id, id, (id * 20) as f64, 50.0, 30.0, 30.0);
        doc.pages[0].items.push(stroke);
    }
    doc.pages[0].rebuild_spatial_index();
    history.set_next_item_id(11);

    let initial_count = doc.pages[0].items.len();
    assert_eq!(initial_count, 10);

    println!("Executing 500 consecutive AddImage operations...");
    let start_exec = Instant::now();
    for step in 0..500 {
        let img = ImageItem::new(
            page_id,
            format!("bulk_img_{}", step),
            Rect::new(
                rng.range_f64(0.0, 2000.0),
                rng.range_f64(0.0, 2000.0),
                rng.range_f64(2050.0, 2200.0),
                rng.range_f64(2050.0, 2200.0),
            ),
            800,
            600,
        );
        let item_id = history.alloc_item_id();
        let z_index = (step + 11) as u64;
        history.execute(
            DocCommand::add_image(page_id, item_id, z_index, img),
            &mut doc,
        );
    }
    let exec_time = start_exec.elapsed();
    println!("Executed 500 AddImage in {:?}", exec_time);
    assert_eq!(doc.pages[0].items.len(), 510);
    assert_eq!(doc.pages[0].spatial_index.len(), 510);

    // 1. UNDO ALL 500 AddImage OPERATIONS
    let start_undo = Instant::now();
    for i in 0..500 {
        assert!(history.undo(&mut doc), "Undo step {} failed", i + 1);
    }
    let undo_time = start_undo.elapsed();
    println!("Undid 500 AddImage in {:?}", undo_time);

    // Verify recovery to initial base state
    assert_eq!(doc.pages[0].items.len(), initial_count);
    assert_eq!(doc.pages[0].spatial_index.len(), initial_count);

    // 2. REDO ALL 500 AddImage OPERATIONS
    let start_redo = Instant::now();
    for i in 0..500 {
        assert!(history.redo(&mut doc), "Redo step {} failed", i + 1);
    }
    let redo_time = start_redo.elapsed();
    println!("Redid 500 AddImage in {:?}", redo_time);
    assert_eq!(doc.pages[0].items.len(), 510);
    assert_eq!(doc.pages[0].spatial_index.len(), 510);

    // 3. UNDO ALL 500 AddImage OPERATIONS AGAIN
    for _ in 0..500 {
        assert!(history.undo(&mut doc));
    }
    assert_eq!(doc.pages[0].items.len(), initial_count);
    assert_eq!(doc.pages[0].spatial_index.len(), initial_count);
}

#[test]
fn test_undo_redo_500_moves_and_transforms_preexisting_items() {
    let mut rng = SimpleRng::new(0x1234_5678_9abc_def0);
    let mut doc = Document::new_notebook("UndoRedo 500 Transforms");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(1000);

    // Initial base state: 10 strokes and 10 images
    let mut initial_item_ids = Vec::new();
    for id in 1..=10 {
        let stroke = create_test_stroke(id, id, (id * 20) as f64, 50.0, 30.0, 30.0);
        doc.pages[0].items.push(stroke);
        initial_item_ids.push(id);
    }
    for id in 11..=20 {
        let img = create_test_image(
            id,
            page_id,
            id,
            (id * 20) as f64,
            150.0,
            40.0,
            40.0,
            &format!("base_img_{}", id),
        );
        doc.pages[0].items.push(img);
        initial_item_ids.push(id);
    }
    doc.pages[0].rebuild_spatial_index();

    let initial_items_snapshot = doc.pages[0].items.clone();
    let initial_spatial_len = doc.pages[0].spatial_index.len();

    println!("Executing 500 consecutive MoveItems and TransformItems on preexisting items...");
    let start_exec = Instant::now();
    let num_ops = 500;

    for _ in 0..num_ops {
        let op_type = rng.range_u64(0, 1);
        let count = rng.range_u64(1, 10) as usize;
        let mut chosen_ids = Vec::with_capacity(count);
        for _ in 0..count {
            let idx = rng.range_u64(0, 19) as usize;
            chosen_ids.push(doc.pages[0].items[idx].id);
        }
        chosen_ids.sort_unstable();
        chosen_ids.dedup();

        if op_type == 0 {
            // MoveItems
            let cmd = DocCommand::MoveItems {
                page_id,
                item_ids: chosen_ids,
                delta_x: rng.range_f64(-15.0, 15.0),
                delta_y: rng.range_f64(-15.0, 15.0),
            };
            history.execute(cmd, &mut doc);
        } else {
            // TransformItems
            let scale = rng.range_f64(0.9, 1.11);
            let cmd = DocCommand::TransformItems {
                page_id,
                item_ids: chosen_ids,
                origin_x: rng.range_f64(50.0, 300.0),
                origin_y: rng.range_f64(50.0, 300.0),
                scale_x: scale,
                scale_y: scale,
                rotation_rad: 0.0,
            };
            history.execute(cmd, &mut doc);
        }
    }
    let exec_time = start_exec.elapsed();
    println!("Executed 500 transform operations in {:?}", exec_time);

    let post_exec_items_snapshot = doc.pages[0].items.clone();
    let post_exec_spatial_len = doc.pages[0].spatial_index.len();

    // 1. UNDO ALL 500 OPERATIONS
    let start_undo = Instant::now();
    for i in 0..num_ops {
        assert!(history.undo(&mut doc), "Undo step {} failed", i + 1);
    }
    let undo_time = start_undo.elapsed();
    println!("Undid 500 transform operations in {:?}", undo_time);

    // Verify recovery to initial snapshot
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), initial_items_snapshot.len());
    assert_eq!(page.spatial_index.len(), initial_spatial_len);

    for (item, expected) in page.items.iter().zip(initial_items_snapshot.iter()) {
        assert_eq!(item.id, expected.id);
        assert_eq!(item.z_index, expected.z_index);
        let b1 = item.bounding_box().expect("bounding box");
        let b2 = expected.bounding_box().expect("expected bounding box");
        let diff_x = (b1.min_x - b2.min_x).abs();
        let diff_y = (b1.min_y - b2.min_y).abs();
        let diff_w = (b1.width() - b2.width()).abs();
        let diff_h = (b1.height() - b2.height()).abs();
        assert!(
            diff_x < 1e-7,
            "min_x drift after 500 pure undo: {:e}",
            diff_x
        );
        assert!(
            diff_y < 1e-7,
            "min_y drift after 500 pure undo: {:e}",
            diff_y
        );
        assert!(
            diff_w < 1e-7,
            "width drift after 500 pure undo: {:e}",
            diff_w
        );
        assert!(
            diff_h < 1e-7,
            "height drift after 500 pure undo: {:e}",
            diff_h
        );
    }

    // 2. REDO ALL 500 OPERATIONS
    let start_redo = Instant::now();
    for i in 0..num_ops {
        assert!(history.redo(&mut doc), "Redo step {} failed", i + 1);
    }
    let redo_time = start_redo.elapsed();
    println!("Redid 500 transform operations in {:?}", redo_time);

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), post_exec_items_snapshot.len());
    assert_eq!(page.spatial_index.len(), post_exec_spatial_len);

    for (item, expected) in page.items.iter().zip(post_exec_items_snapshot.iter()) {
        assert_eq!(item.id, expected.id);
        assert_eq!(item.z_index, expected.z_index);
        let b1 = item.bounding_box().expect("bounding box");
        let b2 = expected.bounding_box().expect("expected bounding box");
        let diff_x = (b1.min_x - b2.min_x).abs();
        let diff_y = (b1.min_y - b2.min_y).abs();
        assert!(
            diff_x < 1e-7,
            "min_x drift after 500 pure redo: {:e}",
            diff_x
        );
        assert!(
            diff_y < 1e-7,
            "min_y drift after 500 pure redo: {:e}",
            diff_y
        );
    }

    // 3. UNDO ALL 500 OPERATIONS AGAIN
    for _ in 0..num_ops {
        assert!(history.undo(&mut doc));
    }
    assert_eq!(doc.pages[0].items.len(), initial_items_snapshot.len());
    assert_eq!(doc.pages[0].spatial_index.len(), initial_spatial_len);
}

/// EMPIRICAL STRESS TEST: 500 interleaved AddImage, MoveItems, TransformItems.
/// Panics during the Redo phase because redone images receive new item IDs,
/// breaking referential integrity with subsequent Move/Transform commands.
#[test]
fn test_undo_redo_500_consecutive_operations_state_recovery() {
    let mut rng = SimpleRng::new(0x500_0123_4567_89ab);
    let mut doc = Document::new_notebook("UndoRedo 500 Stress");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(1000);

    // Initial base state: 10 strokes and 10 images
    let mut initial_item_ids = Vec::new();
    for id in 1..=10 {
        let stroke = create_test_stroke(id, id, (id * 20) as f64, 50.0, 30.0, 30.0);
        doc.pages[0].items.push(stroke);
        initial_item_ids.push(id);
    }
    for id in 11..=20 {
        let img = create_test_image(
            id,
            page_id,
            id,
            (id * 20) as f64,
            150.0,
            40.0,
            40.0,
            &format!("base_img_{}", id),
        );
        doc.pages[0].items.push(img);
        initial_item_ids.push(id);
    }
    doc.pages[0].rebuild_spatial_index();
    history.set_next_item_id(21);

    // Deep snapshot of initial state
    let initial_items_snapshot = doc.pages[0].items.clone();
    let initial_spatial_len = doc.pages[0].spatial_index.len();
    assert_eq!(initial_items_snapshot.len(), 20);
    assert_eq!(initial_spatial_len, 20);

    println!(
        "Executing 500 consecutive mutating operations (AddImage, MoveItems, TransformItems)..."
    );
    let start_exec = Instant::now();
    let num_ops = 500;

    for step in 0..num_ops {
        let op_type = rng.range_u64(0, 2);
        match op_type {
            0 => {
                // AddImage operation
                let img = ImageItem::new(
                    page_id,
                    format!("op_hash_{}", step),
                    Rect::new(
                        rng.range_f64(0.0, 1000.0),
                        rng.range_f64(0.0, 1000.0),
                        rng.range_f64(1050.0, 1200.0),
                        rng.range_f64(1050.0, 1200.0),
                    ),
                    800,
                    600,
                );
                let item_id = history.alloc_item_id();
                let z_index = doc.pages[0].items.len() as u64 + 1;
                let cmd = DocCommand::add_image(page_id, item_id, z_index, img);
                history.execute(cmd, &mut doc);
            }
            1 => {
                // MoveItems operation on random subset of items
                let page = &doc.pages[0];
                if !page.items.is_empty() {
                    let count = rng.range_u64(1, (page.items.len().min(15)) as u64) as usize;
                    let mut chosen_ids = Vec::with_capacity(count);
                    for _ in 0..count {
                        let idx = rng.range_u64(0, (page.items.len() - 1) as u64) as usize;
                        chosen_ids.push(page.items[idx].id);
                    }
                    chosen_ids.sort_unstable();
                    chosen_ids.dedup();

                    let cmd = DocCommand::MoveItems {
                        page_id,
                        item_ids: chosen_ids,
                        delta_x: rng.range_f64(-25.0, 25.0),
                        delta_y: rng.range_f64(-25.0, 25.0),
                    };
                    history.execute(cmd, &mut doc);
                }
            }
            2 => {
                // TransformItems operation (proportional or anisotropic scale)
                let page = &doc.pages[0];
                if !page.items.is_empty() {
                    let count = rng.range_u64(1, (page.items.len().min(10)) as u64) as usize;
                    let mut chosen_ids = Vec::with_capacity(count);
                    for _ in 0..count {
                        let idx = rng.range_u64(0, (page.items.len() - 1) as u64) as usize;
                        chosen_ids.push(page.items[idx].id);
                    }
                    chosen_ids.sort_unstable();
                    chosen_ids.dedup();

                    let scale = rng.range_f64(0.8, 1.25);
                    let cmd = DocCommand::TransformItems {
                        page_id,
                        item_ids: chosen_ids,
                        origin_x: rng.range_f64(0.0, 500.0),
                        origin_y: rng.range_f64(0.0, 500.0),
                        scale_x: scale,
                        scale_y: scale,
                        rotation_rad: 0.0,
                    };
                    history.execute(cmd, &mut doc);
                }
            }
            _ => unreachable!(),
        }
    }
    let exec_time = start_exec.elapsed();
    println!("Executed 500 operations in {:?}", exec_time);

    // Save post-execution snapshot
    let post_exec_items_snapshot = doc.pages[0].items.clone();
    let post_exec_spatial_len = doc.pages[0].spatial_index.len();

    // 1. UNDO ALL 500 OPERATIONS
    let start_undo = Instant::now();
    for i in 0..num_ops {
        let undone = history.undo(&mut doc);
        assert!(undone, "Undo step {} of {} failed", i + 1, num_ops);
    }
    let undo_time = start_undo.elapsed();
    println!("Undid 500 operations in {:?}", undo_time);

    // Verify cannot undo further
    assert!(
        !history.undo(&mut doc),
        "Undo should return false when stack empty"
    );

    // VERIFY EXACT STATE RECOVERY TO INITIAL SNAPSHOT
    let page = &doc.pages[0];
    assert_eq!(
        page.items.len(),
        initial_items_snapshot.len(),
        "Item count mismatch after full undo"
    );
    assert_eq!(
        page.spatial_index.len(),
        initial_spatial_len,
        "Spatial index size mismatch after full undo"
    );

    for (item, expected) in page.items.iter().zip(initial_items_snapshot.iter()) {
        assert_eq!(item.id, expected.id);
        assert_eq!(item.z_index, expected.z_index);
        let b1 = item.bounding_box().expect("item bounding box");
        let b2 = expected.bounding_box().expect("expected bounding box");
        let diff_x = (b1.min_x - b2.min_x).abs();
        let diff_y = (b1.min_y - b2.min_y).abs();
        let diff_w = (b1.width() - b2.width()).abs();
        let diff_h = (b1.height() - b2.height()).abs();
        assert!(diff_x < 1e-7, "min_x drift after 500 undo: {:e}", diff_x);
        assert!(diff_y < 1e-7, "min_y drift after 500 undo: {:e}", diff_y);
        assert!(diff_w < 1e-7, "width drift after 500 undo: {:e}", diff_w);
        assert!(diff_h < 1e-7, "height drift after 500 undo: {:e}", diff_h);
    }

    // 2. REDO ALL 500 OPERATIONS
    let start_redo = Instant::now();
    for i in 0..num_ops {
        let redone = history.redo(&mut doc);
        assert!(redone, "Redo step {} of {} failed", i + 1, num_ops);
    }
    let redo_time = start_redo.elapsed();
    println!("Redid 500 operations in {:?}", redo_time);

    // Verify cannot redo further
    assert!(
        !history.redo(&mut doc),
        "Redo should return false when stack empty"
    );

    // VERIFY STATE MATCHES POST-EXECUTION SNAPSHOT
    let page = &doc.pages[0];
    assert_eq!(
        page.items.len(),
        post_exec_items_snapshot.len(),
        "Item count mismatch after full redo"
    );
    assert_eq!(
        page.spatial_index.len(),
        post_exec_spatial_len,
        "Spatial index len mismatch after full redo"
    );

    for (item, expected) in page.items.iter().zip(post_exec_items_snapshot.iter()) {
        assert_eq!(item.id, expected.id);
        assert_eq!(item.z_index, expected.z_index);
        let b1 = item.bounding_box().expect("item bounding box");
        let b2 = expected.bounding_box().expect("expected bounding box");
        let diff_x = (b1.min_x - b2.min_x).abs();
        let diff_y = (b1.min_y - b2.min_y).abs();
        assert!(diff_x < 1e-7, "min_x drift after redo: {:e}", diff_x);
        assert!(diff_y < 1e-7, "min_y drift after redo: {:e}", diff_y);
    }

    // 3. UNDO ALL 500 OPERATIONS AGAIN
    for _ in 0..num_ops {
        assert!(history.undo(&mut doc));
    }
    assert_eq!(doc.pages[0].items.len(), initial_items_snapshot.len());
    assert_eq!(doc.pages[0].spatial_index.len(), initial_spatial_len);
}

#[test]
fn test_adversarial_extreme_aspect_ratios_and_with_id_safeguards() {
    let page_id = Uuid::new_v4();

    // 1. Extreme Aspect Ratios: Zero Dimensions, Negative BBox, Astronomical Coordinates
    let test_cases = vec![
        // (orig_w, orig_h, rect, expected_condition)
        (0, 0, Rect::new(0.0, 0.0, 0.0, 0.0)), // Zero dims + zero bbox
        (0, 0, Rect::new(100.0, 200.0, 100.0, 200.0)), // Zero dims + point bbox
        (0, 0, Rect::new(100.0, 100.0, 300.0, 100.0)), // Zero dims + zero height bbox
        (0, 0, Rect::new(100.0, 100.0, 100.0, 300.0)), // Zero dims + zero width bbox
        (0, 100, Rect::new(0.0, 0.0, 200.0, 100.0)), // Zero orig_w + valid bbox
        (100, 0, Rect::new(0.0, 0.0, 200.0, 100.0)), // Zero orig_h + valid bbox
        (0, 0, Rect::new(-50000.0, -40000.0, -10000.0, -20000.0)), // Fully negative bbox
        (0, 0, Rect::new(300.0, 400.0, 100.0, 200.0)), // Inverted bbox (min > max)
        (0, 0, Rect::new(1e15, 2e15, 1e15 + 800.0, 2e15 + 400.0)), // Astronomical coords bbox
        (0, 0, Rect::new(1e250, 1e250, 1e250 + 1e-100, 1e250 + 1e250)), // Extreme ratio potential overflow/underflow
        (u32::MAX, 1, Rect::new(0.0, 0.0, 10.0, 10.0)), // Astronomical orig dimensions
        (1, u32::MAX, Rect::new(0.0, 0.0, 10.0, 10.0)), // Astronomical orig dimensions
        (0, 0, Rect::new(f64::NAN, 0.0, 100.0, 100.0)), // NaN in bbox
        (0, 0, Rect::new(0.0, f64::INFINITY, 100.0, 100.0)), // Infinity in bbox
    ];

    for (i, (w, h, rect)) in test_cases.into_iter().enumerate() {
        let item = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            format!("aspect_case_{}", i),
            rect,
            1,
            1.0,
            w,
            h,
        );
        let ratio = item.aspect_ratio();
        assert!(
            ratio > 0.0 && ratio.is_finite(),
            "Case {}: aspect_ratio() returned non-positive or non-finite: {} for w={}, h={}, rect={:?}",
            i, ratio, w, h, rect
        );
    }

    // 2. Extreme `with_id` Opacity values: NaN, Infinity, Negative, Out-of-bounds
    let opacity_cases = vec![
        (f64::NAN, 1.0, "NaN should fallback to 1.0"),
        (f64::INFINITY, 1.0, "+Inf should fallback to 1.0"),
        (f64::NEG_INFINITY, 1.0, "-Inf should fallback to 1.0"),
        (-0.0, 0.0, "-0.0 should clamp to 0.0"),
        (-100.0, 0.0, "Negative opacity should clamp to 0.0"),
        (100.0, 1.0, "Excessive opacity should clamp to 1.0"),
        (0.0, 0.0, "0.0 should remain 0.0"),
        (1.0, 1.0, "1.0 should remain 1.0"),
        (
            0.654,
            0.654,
            "Valid opacity in [0, 1] should be preserved exactly",
        ),
        (
            f64::MIN_POSITIVE,
            f64::MIN_POSITIVE,
            "Subnormal/min positive preserved",
        ),
    ];

    for (input_op, expected_op, msg) in opacity_cases {
        let item = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            "opacity_test",
            Rect::new(0.0, 0.0, 100.0, 100.0),
            1,
            input_op,
            100,
            100,
        );
        assert!(
            (item.opacity - expected_op).abs() < 1e-9,
            "Failed {}: got {}, expected {}",
            msg,
            item.opacity,
            expected_op
        );
        assert!(
            item.opacity >= 0.0 && item.opacity <= 1.0 && item.opacity.is_finite(),
            "Opacity {} invariant violated",
            item.opacity
        );
    }
}
