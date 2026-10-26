//! Adversarial Challenger Test Suite for Milestone M3
//!
//! Scope:
//! 1. Scaling commit via DocCommand::TransformItems: multiple items selected, scale factor applied,
//!    undo restores exact original vertices and bounding box, redo reapplies scale.
//! 2. Stress test repeated undo/redo cycles (150+ cycles) with spatial index queries
//!    (`page.spatial_index.locate_in_envelope` matching `page.items`).
//! 3. Segment Eraser stress testing: slice strokes repeatedly with varying radius, verify all
//!    fragments are correctly indexed in R-Tree, undo all slices sequentially and verify exact
//!    original stroke restoration, followed by sequential redo.
//! 4. Concurrency, numerical stability, zero-drift verification (no NaN/Inf, zero division guards).

#![allow(irrefutable_let_patterns)]

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{EraserMode, NoteCanvas, SelectionState};
use uninote_app::hud::DiagnosticHud;
use uninote_app::selection::HandleId;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, Item, ItemId, ItemKind};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// GTK TEST HARNESS (For GTK widget tests)
// ============================================================================

struct GtkTestHarness {
    sender: mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

static GTK_HARNESS: OnceLock<GtkTestHarness> = OnceLock::new();

fn run_on_gtk_thread<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let harness = GTK_HARNESS.get_or_init(|| {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send>>();
        thread::Builder::new()
            .name("gtk-challenger-m3-worker".into())
            .spawn(move || {
                let _ = gtk4::init();
                while let Ok(task) = rx.recv() {
                    task();
                }
            })
            .expect("Failed to spawn GTK test worker thread");
        GtkTestHarness { sender: tx }
    });

    let (res_tx, res_rx) = mpsc::channel();
    harness
        .sender
        .send(Box::new(move || {
            let res = f();
            let _ = res_tx.send(res);
        }))
        .expect("Send task to GTK worker");
    res_rx.recv().expect("Receive task result from GTK worker")
}

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

fn make_test_stroke(id: u64, points: &[(f64, f64)], base_width: f64) -> Stroke {
    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width,
        ..Default::default()
    };
    let mut stroke = Stroke::new(id, style, PipelineMode::CatmullRomSpline);
    for (i, &(x, y)) in points.iter().enumerate() {
        stroke.points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: base_width,
            timestamp_ns: i as u64 * 1000,
        });
    }
    stroke.recompute_cache();
    stroke
}

fn make_test_item(stroke: Stroke, z_index: u64) -> Item {
    Item {
        id: stroke.id,
        z_index,
        kind: ItemKind::Stroke(stroke),
    }
}

// ============================================================================
// TEST 1: SCALING COMMIT VIA DOCCOMMAND::TRANSFORMITEMS (MULTI-ITEM UNIFORM)
// ============================================================================

#[test]
fn test_scaling_commit_multi_item_uniform_undo_redo_exactness() {
    let mut doc = Document::new_notebook("Transform Multi-Item Test");
    let page_id = doc.pages[0].id;

    // Create 5 distinct strokes with different geometries
    let s1 = make_test_stroke(101, &[(10.0, 10.0), (30.0, 10.0), (20.0, 40.0)], 2.0);
    let s2 = make_test_stroke(102, &[(50.0, 50.0), (90.0, 50.0)], 2.0);
    let s3 = make_test_stroke(103, &[(100.0, 100.0), (120.0, 150.0), (140.0, 100.0)], 3.0);
    let s4 = make_test_stroke(104, &[(200.0, 20.0), (250.0, 80.0)], 1.5);
    let s5 = make_test_stroke(105, &[(300.0, 300.0), (350.0, 350.0), (300.0, 400.0)], 2.5);

    let original_strokes = [s1.clone(), s2.clone(), s3.clone(), s4.clone(), s5.clone()];
    let item_ids = vec![101, 102, 103, 104, 105];

    let page = &mut doc.pages[0];
    for (idx, stroke) in original_strokes.iter().enumerate() {
        let it = make_test_item(stroke.clone(), idx as u64);
        page.items.push(it.clone());
        page.spatial_index.insert(&it);
    }

    assert_eq!(page.items.len(), 5);
    assert_eq!(page.spatial_index.len(), 5);

    let mut history = CommandHistory::new(100);

    // Apply uniform scaling (sx = 2.0, sy = 2.0) with anchor origin at (50.0, 50.0)
    let origin_x = 50.0;
    let origin_y = 50.0;
    let scale_x = 2.0;
    let scale_y = 2.0;

    let cmd = DocCommand::TransformItems {
        page_id,
        item_ids: item_ids.clone(),
        origin_x,
        origin_y,
        scale_x,
        scale_y,
        rotation_rad: 0.0,
    };
    history.execute(cmd, &mut doc);

    // Verify all 5 items have their vertices transformed correctly
    let page = &doc.pages[0];
    for (idx, orig_s) in original_strokes.iter().enumerate() {
        let ItemKind::Stroke(ref transformed) = page.items[idx].kind;
        assert_eq!(transformed.points.len(), orig_s.points.len());
        for (p_trans, p_orig) in transformed.points.iter().zip(orig_s.points.iter()) {
            let exp_x = origin_x + (p_orig.x - origin_x) * scale_x;
            let exp_y = origin_y + (p_orig.y - origin_y) * scale_y;
            assert!(
                (p_trans.x - exp_x).abs() < 1e-10,
                "Point X mismatch on stroke {}: got {}, expected {}",
                orig_s.id,
                p_trans.x,
                exp_x
            );
            assert!(
                (p_trans.y - exp_y).abs() < 1e-10,
                "Point Y mismatch on stroke {}: got {}, expected {}",
                orig_s.id,
                p_trans.y,
                exp_y
            );
        }

        // Verify spatial index finds the transformed item at new coordinates
        let bbox = transformed.bounding_box().expect("Bbox exists");
        let hits = page
            .spatial_index
            .locate_in_envelope(bbox.min_x, bbox.min_y, bbox.max_x, bbox.max_y);
        assert!(
            hits.iter().any(|h| h.item_id == orig_s.id),
            "Spatial index must find transformed item {}",
            orig_s.id
        );
    }

    // Perform UNDO: verify exact restoration of original vertices and bounding boxes
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    for (idx, orig_s) in original_strokes.iter().enumerate() {
        let ItemKind::Stroke(ref reverted) = page.items[idx].kind;
        for (p_rev, p_orig) in reverted.points.iter().zip(orig_s.points.iter()) {
            assert!(
                (p_rev.x - p_orig.x).abs() < 1e-10,
                "Undo must restore exact vertex X for stroke {}",
                orig_s.id
            );
            assert!(
                (p_rev.y - p_orig.y).abs() < 1e-10,
                "Undo must restore exact vertex Y for stroke {}",
                orig_s.id
            );
            assert!(
                (p_rev.width - p_orig.width).abs() < 1e-10,
                "Undo must restore exact point width for uniform scale"
            );
        }

        let orig_bbox = orig_s.bounding_box().unwrap();
        let rev_bbox = reverted.bounding_box().unwrap();
        assert!(
            (orig_bbox.min_x - rev_bbox.min_x).abs() < 1e-9
                && (orig_bbox.min_y - rev_bbox.min_y).abs() < 1e-9
                && (orig_bbox.max_x - rev_bbox.max_x).abs() < 1e-9
                && (orig_bbox.max_y - rev_bbox.max_y).abs() < 1e-9,
            "Undo must restore exact bounding box"
        );
    }
    assert_eq!(page.spatial_index.len(), 5);

    // Perform REDO: verify scale is reapplied
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    for (idx, orig_s) in original_strokes.iter().enumerate() {
        let ItemKind::Stroke(ref redone) = page.items[idx].kind;
        let exp_x = origin_x + (orig_s.points[0].x - origin_x) * scale_x;
        let exp_y = origin_y + (orig_s.points[0].y - origin_y) * scale_y;
        assert!((redone.points[0].x - exp_x).abs() < 1e-10);
        assert!((redone.points[0].y - exp_y).abs() < 1e-10);
    }
    assert_eq!(page.spatial_index.len(), 5);
}

// ============================================================================
// TEST 2: ANISOTROPIC SCALING & MATHEMATICAL WIDTH DRIFT VERIFICATION
// ============================================================================

#[test]
fn test_scaling_anisotropic_vertices_restore_and_width_drift_documented() {
    let mut doc = Document::new_notebook("Anisotropic Scaling Test");
    let page_id = doc.pages[0].id;

    // Stroke from (100, 100) to (200, 100)
    let s = make_test_stroke(201, &[(100.0, 100.0), (200.0, 100.0)], 2.0);
    let orig_width = s.points[0].width;
    let orig_base_width = s.style.base_width;

    let item = make_test_item(s.clone(), 0);
    doc.pages[0].items.push(item.clone());
    doc.pages[0].spatial_index.insert(&item);

    let mut history = CommandHistory::new(100);

    // Anisotropic scale: sx = 2.0, sy = 1.0 (e.g. edge handle RightMiddle drag)
    let origin_x = 100.0;
    let origin_y = 100.0;
    let scale_x = 2.0;
    let scale_y = 1.0;

    let cmd = DocCommand::TransformItems {
        page_id,
        item_ids: vec![201],
        origin_x,
        origin_y,
        scale_x,
        scale_y,
        rotation_rad: 0.0,
    };
    history.execute(cmd, &mut doc);

    // Verify vertices scaled horizontally but not vertically
    let page = &doc.pages[0];
    let ItemKind::Stroke(ref transformed) = page.items[0].kind;
    assert_eq!(transformed.points[0].x, 100.0);
    assert_eq!(transformed.points[1].x, 300.0);
    assert_eq!(transformed.points[0].y, 100.0);
    assert_eq!(transformed.points[1].y, 100.0);

    // Undo anisotropic scaling
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    let ItemKind::Stroke(ref reverted) = page.items[0].kind;

    // 1. Vertices (coordinates x, y) RESTORE EXACTLY:
    assert!((reverted.points[0].x - 100.0).abs() < 1e-10);
    assert!((reverted.points[1].x - 200.0).abs() < 1e-10);
    assert!((reverted.points[0].y - 100.0).abs() < 1e-10);
    assert!((reverted.points[1].y - 100.0).abs() < 1e-10);

    // 2. Stroke width drift analysis:
    // Forward RMS = sqrt((2^2 + 1^2)/2) = sqrt(2.5) ≈ 1.5811388
    // Inverse RMS = sqrt((0.5^2 + 1^2)/2) = sqrt(0.625) ≈ 0.7905694
    // Product = sqrt(2.5 * 0.625) = sqrt(1.5625) = 1.25.
    // The width drifts by exactly factor 1.25 on a single non-uniform scale undo cycle.
    let expected_drift_factor = 1.25;
    let measured_width_ratio = reverted.points[0].width / orig_width;
    let measured_base_width_ratio = reverted.style.base_width / orig_base_width;

    assert!(
        (measured_width_ratio - expected_drift_factor).abs() < 1e-6,
        "Measured width ratio {} must match theoretical RMS product 1.25",
        measured_width_ratio
    );
    assert!(
        (measured_base_width_ratio - expected_drift_factor).abs() < 1e-6,
        "Measured base_width ratio {} must match theoretical RMS product 1.25",
        measured_base_width_ratio
    );

    // Spatial index must remain synchronized despite the width change
    assert_eq!(page.spatial_index.len(), 1);
    let hits = page
        .spatial_index
        .locate_in_envelope(90.0, 90.0, 210.0, 110.0);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item_id, 201);
}

// ============================================================================
// TEST 3: BOUNDING BOX HANDLES: 1D EDGES, CORNERS & ANTI-SINGULARITY BOUNDS
// ============================================================================

#[test]
fn test_scaling_handles_all_edges_and_corners_anti_singularity() {
    let bbox = Rect::new(100.0, 200.0, 300.0, 400.0); // 200x200

    // 1. Verify all 4 Edge handles enforce 1D scaling strictly:
    // LeftMiddle: changes X, Y locked at 1.0
    let (_anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::LeftMiddle, &bbox, (50.0, 300.0), false);
    assert!((sx - 1.25).abs() < 1e-4);
    assert_eq!(sy, 1.0, "LeftMiddle must strictly preserve Y dimension");

    // RightMiddle: changes X, Y locked at 1.0
    let (_anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::RightMiddle, &bbox, (350.0, 300.0), false);
    assert!((sx - 1.25).abs() < 1e-4);
    assert_eq!(sy, 1.0, "RightMiddle must strictly preserve Y dimension");

    // TopMiddle: changes Y, X locked at 1.0
    let (_anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::TopMiddle, &bbox, (200.0, 150.0), false);
    assert_eq!(sx, 1.0, "TopMiddle must strictly preserve X dimension");
    assert!((sy - 1.25).abs() < 1e-4);

    // BottomMiddle: changes Y, X locked at 1.0
    let (_anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::BottomMiddle, &bbox, (200.0, 450.0), false);
    assert_eq!(sx, 1.0, "BottomMiddle must strictly preserve X dimension");
    assert!((sy - 1.25).abs() < 1e-4);

    // 2. Corner handles: locked aspect ratio vs free scaling
    // Locked aspect: BottomRight corner dragged to (400, 400)
    let (_anchor, (sx_lock, sy_lock)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (400.0, 400.0), true);
    assert_eq!(
        sx_lock, sy_lock,
        "Corner scaling with lock_aspect must maintain sx == sy"
    );

    // Free scaling: BottomRight dragged to (400, 300)
    let (_anchor, (sx_free, sy_free)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (400.0, 300.0), false);
    assert!((sx_free - 1.5).abs() < 1e-4);
    assert!((sy_free - 0.5).abs() < 1e-4);

    // 3. Anti-singularity bounds:
    // Drag BottomRight handle to (-1000, -1000) (deep negative inversion attempt)
    let (_anchor, (sx_inv, sy_inv)) = SelectionState::compute_scale_factors(
        HandleId::BottomRight,
        &bbox,
        (-1000.0, -1000.0),
        true,
    );
    assert!(
        sx_inv >= 0.05,
        "Scale factor must clamp to >= 0.05 on negative drag"
    );
    assert!(
        sy_inv >= 0.05,
        "Scale factor must clamp to >= 0.05 on negative drag"
    );
    assert!(
        200.0 * sx_inv >= 5.0,
        "Resulting width must be >= 5.0 pt minimum dimension"
    );
    assert!(
        200.0 * sy_inv >= 5.0,
        "Resulting height must be >= 5.0 pt minimum dimension"
    );

    // Drag exactly onto the opposite anchor point (100.0, 200.0) -> zero vector
    let (_anchor, (sx_zero, sy_zero)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (100.0, 200.0), true);
    assert!(!sx_zero.is_nan() && !sx_zero.is_infinite());
    assert!(!sy_zero.is_nan() && !sy_zero.is_infinite());
    assert!(sx_zero >= 0.05 && 200.0 * sx_zero >= 5.0);

    // Zero-dimension degenerate initial bbox (point stroke)
    let zero_bbox = Rect::new(50.0, 50.0, 50.0, 50.0);
    let (_anchor, (sx_pt, sy_pt)) = SelectionState::compute_scale_factors(
        HandleId::BottomRight,
        &zero_bbox,
        (100.0, 100.0),
        true,
    );
    assert!(!sx_pt.is_nan() && !sx_pt.is_infinite());
    assert!(!sy_pt.is_nan() && !sy_pt.is_infinite());
    assert!(sx_pt > 0.0 && sy_pt > 0.0);
}

// ============================================================================
// TEST 4: STRESS TEST: 150 REPEATED UNDO/REDO CYCLES WITH SPATIAL INDEX PARITY
// ============================================================================

#[test]
fn test_stress_150_undo_redo_cycles_spatial_index_parity() {
    let mut doc = Document::new_notebook("Stress 150 Cycles");
    let page_id = doc.pages[0].id;

    // Create 10 strokes across the page
    let mut original_items = Vec::new();
    for i in 0..10 {
        let x0 = 50.0 + i as f64 * 40.0;
        let y0 = 100.0 + (i % 3) as f64 * 50.0;
        let s = make_test_stroke(
            1000 + i,
            &[(x0, y0), (x0 + 30.0, y0 + 10.0), (x0 + 15.0, y0 + 30.0)],
            2.0,
        );
        let it = make_test_item(s, i);
        original_items.push(it);
    }

    let page = &mut doc.pages[0];
    for it in &original_items {
        page.items.push(it.clone());
        page.spatial_index.insert(it);
    }

    assert_eq!(page.items.len(), 10);
    assert_eq!(page.spatial_index.len(), 10);

    let mut history = CommandHistory::new(200);

    // Execute uniform TransformItems on all 10 strokes
    let item_ids: Vec<ItemId> = (1000..1010).collect();
    let origin_x = 200.0;
    let origin_y = 200.0;
    let scale = 1.5;

    let cmd = DocCommand::TransformItems {
        page_id,
        item_ids,
        origin_x,
        origin_y,
        scale_x: scale,
        scale_y: scale,
        rotation_rad: 0.0,
    };
    history.execute(cmd, &mut doc);

    // Run 150 consecutive Undo / Redo cycles
    const CYCLES: usize = 150;
    for cycle in 1..=CYCLES {
        // UNDO
        assert!(history.undo(&mut doc), "Undo failed at cycle {}", cycle);
        let page = &doc.pages[0];
        assert_eq!(
            page.spatial_index.len(),
            page.items.len(),
            "Spatial index count mismatch on undo at cycle {}",
            cycle
        );

        // Check spatial index query at milestones
        if cycle == 1 || cycle == 50 || cycle == 100 || cycle == CYCLES {
            let all_hits = page.spatial_index.locate_in_envelope(-1e5, -1e5, 1e5, 1e5);
            assert_eq!(all_hits.len(), 10);
            for (idx, hit) in all_hits.iter().enumerate() {
                assert_eq!(hit.item_id, 1000 + idx as u64);
                assert_eq!(hit.z_index, idx as u64);
            }
        }

        // REDO
        assert!(history.redo(&mut doc), "Redo failed at cycle {}", cycle);
        let page = &doc.pages[0];
        assert_eq!(
            page.spatial_index.len(),
            page.items.len(),
            "Spatial index count mismatch on redo at cycle {}",
            cycle
        );

        if cycle == 1 || cycle == 50 || cycle == 100 || cycle == CYCLES {
            let all_hits = page.spatial_index.locate_in_envelope(-1e5, -1e5, 1e5, 1e5);
            assert_eq!(all_hits.len(), 10);
        }
    }

    // Finally undo to verify zero-drift of coordinates after 150 cycles
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    for (i, orig_it) in original_items.iter().enumerate() {
        let ItemKind::Stroke(ref orig_s) = orig_it.kind;
        let ItemKind::Stroke(ref cur_s) = page.items[i].kind;

        for (p_cur, p_orig) in cur_s.points.iter().zip(orig_s.points.iter()) {
            assert!(
                (p_cur.x - p_orig.x).abs() < 1e-9,
                "Drift on X after 150 cycles on item {}: delta = {}",
                orig_it.id,
                (p_cur.x - p_orig.x).abs()
            );
            assert!(
                (p_cur.y - p_orig.y).abs() < 1e-9,
                "Drift on Y after 150 cycles on item {}: delta = {}",
                orig_it.id,
                (p_cur.y - p_orig.y).abs()
            );
        }
    }
}

// ============================================================================
// TEST 5: STRESS TEST: MIXED COMMAND CHAIN (TRANSFORM, MOVE, SLICE, ERASE)
// ============================================================================

#[test]
fn test_stress_mixed_command_chain_undo_redo_spatial_index() {
    let mut doc = Document::new_notebook("Mixed Command Chain Test");
    let page_id = doc.pages[0].id;

    // Create 4 strokes
    let mut items = Vec::new();
    for i in 0..4 {
        let s = make_test_stroke(
            3000 + i,
            &[
                (100.0 * (i + 1) as f64, 100.0),
                (100.0 * (i + 1) as f64 + 50.0, 100.0),
            ],
            2.0,
        );
        let it = make_test_item(s, i);
        items.push(it);
    }

    let page = &mut doc.pages[0];
    for it in &items {
        page.items.push(it.clone());
        page.spatial_index.insert(it);
    }

    let mut history = CommandHistory::new(100);

    // Command 1: TransformItems (scale 2 items by 1.5x)
    history.execute(
        DocCommand::TransformItems {
            page_id,
            item_ids: vec![3000, 3001],
            origin_x: 100.0,
            origin_y: 100.0,
            scale_x: 1.5,
            scale_y: 1.5,
            rotation_rad: 0.0,
        },
        &mut doc,
    );

    // Command 2: MoveItems (translate item 3002 by dx=20, dy=-30)
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![3002],
            delta_x: 20.0,
            delta_y: -30.0,
        },
        &mut doc,
    );

    // Command 3: SliceStroke on item 3003
    let ItemKind::Stroke(ref s3003) = doc.pages[0].items[3].kind;
    let frags = s3003.slice_with_circle((425.0, 100.0), 10.0);
    assert_eq!(frags.len(), 2);
    let frag_items = vec![
        make_test_item(frags[0].clone(), 3),
        make_test_item(frags[1].clone(), 3),
    ];
    let orig_item_3003 = doc.pages[0].items[3].clone();
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: orig_item_3003,
            new_strokes: frag_items,
        },
        &mut doc,
    );

    // At step 3: items count is 3000, 3001, 3002, frag1, frag2 = 5 items
    assert_eq!(doc.pages[0].items.len(), 5);
    assert_eq!(doc.pages[0].spatial_index.len(), 5);

    // Command 4: EraseItems (erase item 3000)
    let erased = vec![doc.pages[0].items[0].clone()];
    history.execute(
        DocCommand::EraseItems {
            page_id,
            erased_items: erased,
        },
        &mut doc,
    );

    // At step 4: 4 items remaining
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Sequential Undo all 4 commands
    // Undo 4 (restore erased 3000)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 5);
    assert_eq!(doc.pages[0].spatial_index.len(), 5);

    // Undo 3 (restore sliced 3003)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Undo 2 (revert MoveItems)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Undo 1 (revert TransformItems)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Redo all 4 commands forward
    assert!(history.redo(&mut doc)); // Transform
    assert_eq!(doc.pages[0].items.len(), 4);
    assert!(history.redo(&mut doc)); // Move
    assert_eq!(doc.pages[0].items.len(), 4);
    assert!(history.redo(&mut doc)); // Slice
    assert_eq!(doc.pages[0].items.len(), 5);
    assert!(history.redo(&mut doc)); // Erase
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);
}

// ============================================================================
// TEST 6: SEGMENT ERASER: VARYING RADII CUTS & SPATIAL INDEX SYNCHRONIZATION
// ============================================================================

#[test]
fn test_segment_eraser_varying_radii_and_spatial_index_synchronization() {
    let mut doc = Document::new_notebook("Segment Eraser Radii Test");
    let page_id = doc.pages[0].id;

    // Stroke along y=50 from x=0 to x=200 with 21 points
    let mut pts = Vec::new();
    for i in 0..=20 {
        pts.push((i as f64 * 10.0, 50.0));
    }
    let orig_stroke = make_test_stroke(4000, &pts, 2.0);
    let orig_item = make_test_item(orig_stroke.clone(), 0);

    doc.pages[0].items.push(orig_item.clone());
    doc.pages[0].spatial_index.insert(&orig_item);

    let mut history = CommandHistory::new(100);

    // 1. Center cut with radius 15.0 at (100.0, 50.0): divides into 2 fragments
    let frags = orig_stroke.slice_with_circle((100.0, 50.0), 15.0);
    assert_eq!(frags.len(), 2, "Center cut must produce 2 fragments");

    let frag_items = vec![
        make_test_item(frags[0].clone(), 0),
        make_test_item(frags[1].clone(), 0),
    ];

    let cmd1 = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item.clone(),
        new_strokes: frag_items.clone(),
    };
    history.execute(cmd1, &mut doc);

    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // Spatial query on left fragment
    let left_hits = doc.pages[0]
        .spatial_index
        .locate_in_envelope(0.0, 40.0, 85.0, 60.0);
    assert_eq!(left_hits.len(), 1);

    // Spatial query on removed center
    let center_hits = doc.pages[0]
        .spatial_index
        .locate_in_envelope(95.0, 45.0, 105.0, 55.0);
    assert_eq!(
        center_hits.len(),
        0,
        "Erased center must have zero spatial index hits"
    );

    // 2. End cut: trim the end of right fragment (from 115 to 200) with circle at (195.0, 50.0), radius 10.0
    let right_stroke = &frags[1];
    let end_frags = right_stroke.slice_with_circle((195.0, 50.0), 10.0);
    assert_eq!(
        end_frags.len(),
        1,
        "End trim must produce 1 shortened fragment"
    );

    let end_frag_items = vec![make_test_item(end_frags[0].clone(), 0)];
    let cmd2 = DocCommand::SliceStroke {
        page_id,
        original_stroke: frag_items[1].clone(),
        new_strokes: end_frag_items,
    };
    history.execute(cmd2, &mut doc);

    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // 3. Macro erase: swallow the left fragment completely with radius 500.0
    let left_stroke = &frags[0];
    let macro_frags = left_stroke.slice_with_circle((50.0, 50.0), 500.0);
    assert_eq!(macro_frags.len(), 0, "Macro erase must produce 0 fragments");

    let cmd3 = DocCommand::SliceStroke {
        page_id,
        original_stroke: frag_items[0].clone(),
        new_strokes: vec![],
    };
    history.execute(cmd3, &mut doc);

    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[0].spatial_index.len(), 1);

    // Undo 3: restores left fragment
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // Undo 2: restores untrimmed right fragment
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // Undo 1: restores exact original stroke
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[0].spatial_index.len(), 1);

    let ItemKind::Stroke(ref restored) = doc.pages[0].items[0].kind;
    assert_eq!(restored.id, orig_stroke.id);
    assert_eq!(restored.points.len(), orig_stroke.points.len());
    for (p_res, p_orig) in restored.points.iter().zip(orig_stroke.points.iter()) {
        assert_eq!(p_res.x, p_orig.x);
        assert_eq!(p_res.y, p_orig.y);
    }
}

// ============================================================================
// TEST 7: SEGMENT ERASER: RECURSIVE SLICING & SEQUENTIAL RESTORATION
// ============================================================================

#[test]
fn test_segment_eraser_recursive_slicing_and_sequential_restoration() {
    let mut doc = Document::new_notebook("Recursive Slicing Test");
    let page_id = doc.pages[0].id;

    // Create a 101-point horizontal line stroke: (0, 100) to (1000, 100)
    let mut pts = Vec::new();
    for i in 0..=100 {
        pts.push((i as f64 * 10.0, 100.0));
    }
    let orig_stroke = make_test_stroke(5000, &pts, 2.0);
    let orig_item = make_test_item(orig_stroke.clone(), 0);

    doc.pages[0].items.push(orig_item.clone());
    doc.pages[0].spatial_index.insert(&orig_item);

    let mut history = CommandHistory::new(100);
    let mut current_id = 5001;

    // Step 1: Slice at x=500, radius 15 -> splits into 2 fragments (F1: ~[0, 485], F2: ~[515, 1000])
    let frags1 = orig_stroke.slice_with_circle((500.0, 100.0), 15.0);
    assert_eq!(frags1.len(), 2);
    let f1_item = make_test_item(
        {
            let mut s = frags1[0].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    let f2_item = make_test_item(
        {
            let mut s = frags1[1].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: orig_item.clone(),
            new_strokes: vec![f1_item.clone(), f2_item.clone()],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // Step 2: Slice F1 at x=250, radius 15 -> splits into F1a, F1b
    let ItemKind::Stroke(ref s_f1) = f1_item.kind;
    let frags2 = s_f1.slice_with_circle((250.0, 100.0), 15.0);
    assert_eq!(frags2.len(), 2);
    let f1a_item = make_test_item(
        {
            let mut s = frags2[0].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    let f1b_item = make_test_item(
        {
            let mut s = frags2[1].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: f1_item.clone(),
            new_strokes: vec![f1a_item.clone(), f1b_item.clone()],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 3); // F1a, F1b, F2
    assert_eq!(doc.pages[0].spatial_index.len(), 3);

    // Step 3: Slice F2 at x=750, radius 15 -> splits into F2a, F2b
    let ItemKind::Stroke(ref s_f2) = f2_item.kind;
    let frags3 = s_f2.slice_with_circle((750.0, 100.0), 15.0);
    assert_eq!(frags3.len(), 2);
    let f2a_item = make_test_item(
        {
            let mut s = frags3[0].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    let f2b_item = make_test_item(
        {
            let mut s = frags3[1].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: f2_item.clone(),
            new_strokes: vec![f2a_item.clone(), f2b_item.clone()],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 4); // F1a, F1b, F2a, F2b
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Step 4: Slice F1a at x=125, radius 15 -> splits into F1a1, F1a2
    let ItemKind::Stroke(ref s_f1a) = f1a_item.kind;
    let frags4 = s_f1a.slice_with_circle((125.0, 100.0), 15.0);
    assert_eq!(frags4.len(), 2);
    let f1a1_item = make_test_item(
        {
            let mut s = frags4[0].clone();
            s.id = current_id;
            current_id += 1;
            s
        },
        0,
    );
    let f1a2_item = make_test_item(
        {
            let mut s = frags4[1].clone();
            s.id = current_id;
            s
        },
        0,
    );
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: f1a_item.clone(),
            new_strokes: vec![f1a1_item.clone(), f1a2_item.clone()],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 5); // F1a1, F1a2, F1b, F2a, F2b
    assert_eq!(doc.pages[0].spatial_index.len(), 5);

    // Step 5: Completely erase F2b with large circle (radius 200 at 880, 100) -> 0 fragments
    let ItemKind::Stroke(ref s_f2b) = f2b_item.kind;
    let frags5 = s_f2b.slice_with_circle((880.0, 100.0), 200.0);
    assert_eq!(frags5.len(), 0);
    history.execute(
        DocCommand::SliceStroke {
            page_id,
            original_stroke: f2b_item.clone(),
            new_strokes: vec![],
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].items.len(), 4); // F1a1, F1a2, F1b, F2a
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // SEQUENTIAL UNDO:
    // Undo Step 5: restores F2b
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 5);
    assert_eq!(doc.pages[0].spatial_index.len(), 5);

    // Undo Step 4: restores F1a
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    // Undo Step 3: restores F2
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 3);
    assert_eq!(doc.pages[0].spatial_index.len(), 3);

    // Undo Step 2: restores F1
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 2);
    assert_eq!(doc.pages[0].spatial_index.len(), 2);

    // Undo Step 1: restores exact original stroke S0
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[0].spatial_index.len(), 1);

    let ItemKind::Stroke(ref restored) = doc.pages[0].items[0].kind;
    assert_eq!(restored.id, orig_stroke.id);
    assert_eq!(restored.points.len(), 101);
    assert_eq!(restored.points[0].x, 0.0);
    assert_eq!(restored.points[100].x, 1000.0);

    // SEQUENTIAL REDO:
    for expected_count in [2, 3, 4, 5, 4] {
        assert!(history.redo(&mut doc));
        assert_eq!(doc.pages[0].items.len(), expected_count);
        assert_eq!(doc.pages[0].spatial_index.len(), expected_count);
    }
}

// ============================================================================
// TEST 8: MULTI-STROKE SIMULTANEOUS SLICE (CANVAS CROSS-SECTION SIMULATION)
// ============================================================================

#[test]
fn test_segment_eraser_multi_stroke_simultaneous_slice() {
    let mut doc = Document::new_notebook("Multi-Stroke Slice Test");
    let page_id = doc.pages[0].id;

    // Cross pattern: 2 horizontal and 2 vertical strokes intersecting near (100, 100)
    let h1 = make_test_stroke(6001, &[(50.0, 95.0), (150.0, 95.0)], 2.0);
    let h2 = make_test_stroke(6002, &[(50.0, 105.0), (150.0, 105.0)], 2.0);
    let v1 = make_test_stroke(6003, &[(95.0, 50.0), (95.0, 150.0)], 2.0);
    let v2 = make_test_stroke(6004, &[(105.0, 50.0), (105.0, 150.0)], 2.0);

    let items = vec![
        make_test_item(h1.clone(), 0),
        make_test_item(h2.clone(), 1),
        make_test_item(v1.clone(), 2),
        make_test_item(v2.clone(), 3),
    ];

    for it in &items {
        doc.pages[0].items.push(it.clone());
        doc.pages[0].spatial_index.insert(it);
    }
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);

    let mut history = CommandHistory::new(100);
    let mut alloc_id = 6010;

    // Eraser circle at (100.0, 100.0) with radius 15.0 intersects all 4 strokes!
    let erase_center = (100.0, 100.0);
    let erase_radius = 15.0;

    for it in &items {
        let ItemKind::Stroke(ref s) = it.kind;
        let frags = s.slice_with_circle(erase_center, erase_radius);
        assert_eq!(frags.len(), 2, "Each intersecting stroke must split into 2");

        let mut rep_items = Vec::new();
        for frag in frags {
            let mut frag_s = frag;
            frag_s.id = alloc_id;
            alloc_id += 1;
            rep_items.push(make_test_item(frag_s, it.z_index));
        }

        history.execute(
            DocCommand::SliceStroke {
                page_id,
                original_stroke: it.clone(),
                new_strokes: rep_items,
            },
            &mut doc,
        );
    }

    // 4 strokes each split into 2 => total 8 items
    assert_eq!(doc.pages[0].items.len(), 8);
    assert_eq!(doc.pages[0].spatial_index.len(), 8);

    // None of the original strokes are present
    for it in &items {
        assert!(!doc.pages[0].items.iter().any(|i| i.id == it.id));
    }

    // Sequentially undo all 4 slice commands
    for _ in 0..4 {
        assert!(history.undo(&mut doc));
    }

    // Exact 4 original strokes restored
    assert_eq!(doc.pages[0].items.len(), 4);
    assert_eq!(doc.pages[0].spatial_index.len(), 4);
    for it in &items {
        assert!(doc.pages[0].items.iter().any(|i| i.id == it.id));
    }
}

// ============================================================================
// TEST 9: NUMERICAL STABILITY, NAN/INF/ZERO-DIVISION DEFENSIVE GUARDS
// ============================================================================

#[test]
fn test_numerical_stability_nan_inf_zero_division_resilience() {
    let mut doc = Document::new_notebook("Numerical Stability Test");
    let page_id = doc.pages[0].id;

    let s = make_test_stroke(7001, &[(10.0, 10.0), (50.0, 50.0)], 2.0);
    let it = make_test_item(s.clone(), 0);
    doc.pages[0].items.push(it.clone());
    doc.pages[0].spatial_index.insert(&it);

    let mut history = CommandHistory::new(100);

    // 1. DocCommand::TransformItems with NaN / Inf inputs: must be safely ignored without panic
    let bad_cmds = vec![
        DocCommand::TransformItems {
            page_id,
            item_ids: vec![7001],
            origin_x: f64::NAN,
            origin_y: 10.0,
            scale_x: 2.0,
            scale_y: 2.0,
            rotation_rad: 0.0,
        },
        DocCommand::TransformItems {
            page_id,
            item_ids: vec![7001],
            origin_x: 10.0,
            origin_y: f64::INFINITY,
            scale_x: 2.0,
            scale_y: 2.0,
            rotation_rad: 0.0,
        },
        DocCommand::TransformItems {
            page_id,
            item_ids: vec![7001],
            origin_x: 10.0,
            origin_y: 10.0,
            scale_x: f64::NAN,
            scale_y: 2.0,
            rotation_rad: 0.0,
        },
        DocCommand::TransformItems {
            page_id,
            item_ids: vec![], // Empty item IDs
            origin_x: 10.0,
            origin_y: 10.0,
            scale_x: 2.0,
            scale_y: 2.0,
            rotation_rad: 0.0,
        },
    ];

    for bad_cmd in bad_cmds {
        history.execute(bad_cmd, &mut doc);
        // Geometry must remain unchanged
        let ItemKind::Stroke(ref cur_s) = doc.pages[0].items[0].kind;
        assert_eq!(cur_s.points[0].x, 10.0);
        assert_eq!(cur_s.points[1].x, 50.0);
        assert_eq!(doc.pages[0].spatial_index.len(), 1);
    }

    // 2. SelectionState::compute_scale_factors with NaN / Inf cursor
    let bbox = Rect::new(10.0, 10.0, 50.0, 50.0);
    let (_anchor, (sx_nan, sy_nan)) = SelectionState::compute_scale_factors(
        HandleId::BottomRight,
        &bbox,
        (f64::NAN, f64::INFINITY),
        true,
    );
    assert!(!sx_nan.is_nan() && !sx_nan.is_infinite());
    assert!(!sy_nan.is_nan() && !sy_nan.is_infinite());

    // 3. Extremely large coordinates (1e15, -1e15)
    let (_anchor, (sx_huge, sy_huge)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (1e15, 1e15), true);
    assert!(sx_huge.is_finite());
    assert!(sy_huge.is_finite());
}

// ============================================================================
// TEST 10: CONCURRENCY: MULTI-THREADED SPATIAL INDEX PARALLEL QUERIES
// ============================================================================

#[test]
fn test_concurrency_spatial_index_parallel_queries() {
    let mut doc = Document::new_notebook("Parallel Spatial Queries Test");

    // Populate with 100 strokes across a 1000x1000 coordinate space
    for i in 0..100 {
        let x = (i % 10) as f64 * 100.0;
        let y = (i / 10) as f64 * 100.0;
        let s = make_test_stroke(8000 + i as u64, &[(x, y), (x + 30.0, y + 30.0)], 2.0);
        let it = make_test_item(s, i as u64);
        doc.pages[0].items.push(it.clone());
        doc.pages[0].spatial_index.insert(&it);
    }

    assert_eq!(doc.pages[0].spatial_index.len(), 100);

    // Share spatial index across 8 parallel threads and perform concurrent queries
    let spatial_index = doc.pages[0].spatial_index.clone();
    let mut handles = Vec::new();

    for t in 0..8 {
        let index_clone = spatial_index.clone();
        let handle = thread::spawn(move || {
            let mut total_hits = 0;
            for i in 0..50 {
                let q_min_x = (t * 50 + i * 10) as f64;
                let q_min_y = 0.0;
                let q_max_x = q_min_x + 150.0;
                let q_max_y = 500.0;
                let hits = index_clone.locate_in_envelope(q_min_x, q_min_y, q_max_x, q_max_y);
                total_hits += hits.len();
            }
            total_hits
        });
        handles.push(handle);
    }

    for handle in handles {
        let hits = handle.join().expect("Worker thread panicked");
        assert!(hits > 0, "Concurrent queries must return hits");
    }
}

// ============================================================================
// TEST 11: CANVAS INTERACTIVE ERASER MODE & SELECTION INTEGRATION
// ============================================================================

#[test]
fn test_canvas_eraser_mode_and_selection_integration() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        // Verify default eraser mode is WholeStroke
        assert_eq!(canvas.eraser_mode(), EraserMode::WholeStroke);

        // Toggle to Segment
        canvas.set_eraser_mode(EraserMode::Segment);
        assert_eq!(canvas.eraser_mode(), EraserMode::Segment);

        // Perform undo on empty canvas: returns false gracefully
        assert!(!canvas.undo());
        assert!(!canvas.redo());

        // Toggle back
        canvas.set_eraser_mode(EraserMode::WholeStroke);
        assert_eq!(canvas.eraser_mode(), EraserMode::WholeStroke);
    });
}
