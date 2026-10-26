//! Integration and Unit Test Suite for Milestone M3 (Canvas Handles & Segment Eraser)
//!
//! Scope:
//! 1. Handle Coordinate Placement for all 8 handles against known AABB.
//! 2. Anchor Point Calculations for all 8 handles (opposite corner / edge).
//! 3. Screen-Space Hit Testing Priority (corners over pills, pills over body, body over none).
//! 4. Screen-Space Hit Testing Zoom Invariance and Decluttering (< 32px screen threshold).
//! 5. Corner Scaling Geometry with Aspect-Ratio-Locked and Free Scale (Shift modifier).
//! 6. Edge Scaling Geometry (1D normal scaling: Top/Bottom scales Y, Left/Right scales X).
//! 7. Minimum Size Clamping and Anti-Singularity Guards (factor >= 0.05, size >= 5.0 pt).
//! 8. DocCommand::TransformItems Commit, Undo/Redo Reversibility, and Spatial Index Invariance.
//! 9. Segment Eraser Stroke Slicing and DocCommand::SliceStroke Undo/Redo Reversibility.
//! 10. EraserMode Enum and Canvas Toggle Integration.

#![allow(irrefutable_let_patterns)]

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{CanvasState, EraserMode, NoteCanvas, SelectionState};
use uninote_app::hud::DiagnosticHud;
use uninote_app::selection::{HandleId, SelectionHitTestResult};
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::coordinates::Viewport;
use uninote_core::model::{Document, Item, ItemKind};
use uninote_storage::SaveStatus;
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// DEDICATED GTK TEST HARNESS (For GTK widget tests)
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
            .name("gtk-handles-test-worker".into())
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
// TEST 1: HANDLE IDENTIFIER PROPERTIES & SYMMETRY
// ============================================================================

#[test]
fn test_handle_id_properties_and_symmetry() {
    assert_eq!(HandleId::ALL.len(), 8);

    // 4 corners
    assert!(HandleId::TopLeft.is_corner());
    assert!(HandleId::TopRight.is_corner());
    assert!(HandleId::BottomRight.is_corner());
    assert!(HandleId::BottomLeft.is_corner());

    // 4 edge pills
    assert!(!HandleId::TopMiddle.is_corner());
    assert!(!HandleId::BottomMiddle.is_corner());
    assert!(!HandleId::LeftMiddle.is_corner());
    assert!(!HandleId::RightMiddle.is_corner());

    // Involution symmetry: opposite(opposite(h)) == h
    for handle in HandleId::ALL {
        assert_eq!(
            handle.opposite().opposite(),
            handle,
            "Opposite must be an involution for {:?}",
            handle
        );
    }

    // Opposite pairs match expected geometry
    assert_eq!(HandleId::TopLeft.opposite(), HandleId::BottomRight);
    assert_eq!(HandleId::TopRight.opposite(), HandleId::BottomLeft);
    assert_eq!(HandleId::TopMiddle.opposite(), HandleId::BottomMiddle);
    assert_eq!(HandleId::LeftMiddle.opposite(), HandleId::RightMiddle);
}

// ============================================================================
// TEST 2: HANDLE COORDINATE PLACEMENT AGAINST KNOWN AABB
// ============================================================================

#[test]
fn test_handle_coordinate_placement_against_known_aabb() {
    let bbox = Rect::new(100.0, 200.0, 300.0, 400.0);

    assert_eq!(HandleId::TopLeft.position(&bbox), (100.0, 200.0));
    assert_eq!(HandleId::TopMiddle.position(&bbox), (200.0, 200.0));
    assert_eq!(HandleId::TopRight.position(&bbox), (300.0, 200.0));
    assert_eq!(HandleId::RightMiddle.position(&bbox), (300.0, 300.0));
    assert_eq!(HandleId::BottomRight.position(&bbox), (300.0, 400.0));
    assert_eq!(HandleId::BottomMiddle.position(&bbox), (200.0, 400.0));
    assert_eq!(HandleId::BottomLeft.position(&bbox), (100.0, 400.0));
    assert_eq!(HandleId::LeftMiddle.position(&bbox), (100.0, 300.0));
}

// ============================================================================
// TEST 3: ANCHOR POINT CALCULATIONS FOR ALL 8 HANDLES
// ============================================================================

#[test]
fn test_anchor_point_calculation_for_all_handles() {
    let bbox = Rect::new(100.0, 200.0, 300.0, 400.0);

    // Anchor of corner is opposite corner
    assert_eq!(HandleId::TopLeft.anchor_point(&bbox), (300.0, 400.0));
    assert_eq!(HandleId::TopRight.anchor_point(&bbox), (100.0, 400.0));
    assert_eq!(HandleId::BottomRight.anchor_point(&bbox), (100.0, 200.0));
    assert_eq!(HandleId::BottomLeft.anchor_point(&bbox), (300.0, 200.0));

    // Anchor of edge is opposite edge midpoint
    assert_eq!(HandleId::TopMiddle.anchor_point(&bbox), (200.0, 400.0));
    assert_eq!(HandleId::BottomMiddle.anchor_point(&bbox), (200.0, 200.0));
    assert_eq!(HandleId::LeftMiddle.anchor_point(&bbox), (300.0, 300.0));
    assert_eq!(HandleId::RightMiddle.anchor_point(&bbox), (100.0, 300.0));
}

// ============================================================================
// TEST 4: SCREEN-SPACE HIT TESTING PRIORITY AND RADII
// ============================================================================

#[test]
fn test_screenspace_hit_testing_priority_and_radii() {
    let mut sel = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 300.0, 300.0)),
        ..Default::default()
    };
    let vp = Viewport::new(0.0, 0.0, 1.0);

    // 1. Exact corner hit
    assert_eq!(
        sel.hit_test((100.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );
    assert_eq!(
        sel.hit_test((300.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopRight)
    );
    assert_eq!(
        sel.hit_test((300.0, 300.0), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomRight)
    );
    assert_eq!(
        sel.hit_test((100.0, 300.0), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomLeft)
    );

    // 2. Corner proximity within 15px radius (dx=10, dy=10 -> dist^2 = 200 <= 225)
    assert_eq!(
        sel.hit_test((110.0, 110.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );

    // 3. Edge pill hit (TopMiddle: x=200, y=100)
    assert_eq!(
        sel.hit_test((200.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );
    // Edge pill proximity (dx=12 <= 14, dy=7 <= 10)
    assert_eq!(
        sel.hit_test((212.0, 107.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );

    // 4. Body hit (inside bounding box, outside handles)
    assert_eq!(
        sel.hit_test((200.0, 200.0), &vp),
        SelectionHitTestResult::Body
    );

    // 5. Outside hit
    assert_eq!(
        sel.hit_test((50.0, 50.0), &vp),
        SelectionHitTestResult::None
    );
    assert_eq!(
        sel.hit_test((350.0, 350.0), &vp),
        SelectionHitTestResult::None
    );

    // 6. None when no selection bbox
    sel.bbox = None;
    assert_eq!(
        sel.hit_test((200.0, 200.0), &vp),
        SelectionHitTestResult::None
    );
}

// ============================================================================
// TEST 5: SCREEN-SPACE HIT TESTING ZOOM INVARIANCE
// ============================================================================

#[test]
fn test_screenspace_hit_testing_zoom_invariance() {
    let sel = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 300.0, 300.0)),
        ..Default::default()
    };

    // Viewport with zoom 2.0 and pan offset (50.0, 50.0)
    let vp = Viewport::new(50.0, 50.0, 2.0);

    // TopLeft corner at document (100.0, 100.0) maps to screen (250.0, 250.0)
    // A screen contact 10px away at (260.0, 250.0) should hit TopLeft handle
    let doc_contact = vp.screen_to_document((260.0, 250.0));
    assert_eq!(
        sel.hit_test(doc_contact, &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );

    // A screen contact 20px away at (270.0, 250.0) exceeds 15px radius and is not a corner hit
    let doc_miss = vp.screen_to_document((270.0, 250.0));
    assert_ne!(
        sel.hit_test(doc_miss, &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );
}

// ============================================================================
// TEST 6: SCREEN-SPACE HIT TESTING DECLUTTERING UNDER 32PX
// ============================================================================

#[test]
fn test_screenspace_hit_testing_decluttering_under_32px() {
    let sel = SelectionState {
        bbox: Some(Rect::new(0.0, 10.0, 100.0, 30.0)),
        ..Default::default()
    };
    let vp = Viewport::new(0.0, 0.0, 1.0);

    // Corners are still active at (0.0, 10.0)
    assert_eq!(
        sel.hit_test((0.0, 10.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );

    // Midpoints (at x=50.0, y=10.0) are > 15px from corners, so corners don't hit.
    // They must NOT hit edge pills because pills are decluttered (height 20px < 32px).
    // Instead, contact falls through to Body.
    assert_eq!(
        sel.hit_test((50.0, 10.0), &vp),
        SelectionHitTestResult::Body
    );
}

// ============================================================================
// TEST 7: CORNER SCALING GEOMETRY (ASPECT-RATIO LOCKED VS FREE)
// ============================================================================

#[test]
fn test_corner_scaling_geometry_locked_and_free() {
    let bbox = Rect::new(100.0, 100.0, 300.0, 300.0); // 200x200, anchor BottomRight is (100, 100)

    // 1. Proportional diagonal drag: (400, 400) -> 1.5x uniform scale
    let (anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (400.0, 400.0), true);
    assert_eq!(anchor, (100.0, 100.0));
    assert!((sx - 1.5).abs() < 1e-4);
    assert!((sy - 1.5).abs() < 1e-4);

    // 2. Off-diagonal drag with lock_aspect = true (orthogonal projection)
    // v = (300, 250), v0 = (200, 200) -> dot = 60000 + 50000 = 110000, len_sq = 80000 -> 1.375
    let (_anchor, (sx_locked, sy_locked)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (400.0, 350.0), true);
    assert_eq!(sx_locked, sy_locked);
    assert!((sx_locked - 1.375).abs() < 1e-4);

    // 3. Same off-diagonal drag with lock_aspect = false (Free scaling / Shift)
    // sx = 300 / 200 = 1.5, sy = 250 / 200 = 1.25
    let (_anchor, (sx_free, sy_free)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (400.0, 350.0), false);
    assert!((sx_free - 1.5).abs() < 1e-4);
    assert!((sy_free - 1.25).abs() < 1e-4);

    // 4. TopLeft corner drag: anchor is BottomRight (300.0, 300.0)
    // Moving TopLeft from (100, 100) to (0, 0) -> expands from 200 to 300 (1.5x)
    let (anchor_tl, (sx_tl, sy_tl)) =
        SelectionState::compute_scale_factors(HandleId::TopLeft, &bbox, (0.0, 0.0), true);
    assert_eq!(anchor_tl, (300.0, 300.0));
    assert!((sx_tl - 1.5).abs() < 1e-4);
    assert!((sy_tl - 1.5).abs() < 1e-4);

    // Moving TopLeft from (100, 100) to (-100, -100) -> expands from 200 to 400 (2.0x)
    let (anchor_tl2, (sx_tl2, sy_tl2)) =
        SelectionState::compute_scale_factors(HandleId::TopLeft, &bbox, (-100.0, -100.0), true);
    assert_eq!(anchor_tl2, (300.0, 300.0));
    assert!((sx_tl2 - 2.0).abs() < 1e-4);
    assert!((sy_tl2 - 2.0).abs() < 1e-4);
}

// ============================================================================
// TEST 8: EDGE SCALING GEOMETRY (ONE-DIMENSIONAL NORMAL SCALING)
// ============================================================================

#[test]
fn test_edge_scaling_one_dimensional() {
    let bbox = Rect::new(100.0, 100.0, 300.0, 300.0); // 200x200

    // TopMiddle: anchor is (200, 300). Dragging up to y = 50 -> dy = 250 -> sy = 1.25, sx = 1.0
    let (anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::TopMiddle, &bbox, (200.0, 50.0), true);
    assert_eq!(anchor, (200.0, 300.0));
    assert_eq!(sx, 1.0);
    assert!((sy - 1.25).abs() < 1e-4);

    // BottomMiddle: anchor is (200, 100). Dragging down to y = 400 -> dy = 300 -> sy = 1.5, sx = 1.0
    let (anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::BottomMiddle, &bbox, (200.0, 400.0), true);
    assert_eq!(anchor, (200.0, 100.0));
    assert_eq!(sx, 1.0);
    assert!((sy - 1.5).abs() < 1e-4);

    // LeftMiddle: anchor is (300, 200). Dragging left to x = 50 -> dx = 250 -> sx = 1.25, sy = 1.0
    let (anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::LeftMiddle, &bbox, (50.0, 200.0), true);
    assert_eq!(anchor, (300.0, 200.0));
    assert!((sx - 1.25).abs() < 1e-4);
    assert_eq!(sy, 1.0);

    // RightMiddle: anchor is (100, 200). Dragging right to x = 400 -> dx = 300 -> sx = 1.5, sy = 1.0
    let (anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::RightMiddle, &bbox, (400.0, 200.0), true);
    assert_eq!(anchor, (100.0, 200.0));
    assert!((sx - 1.5).abs() < 1e-4);
    assert_eq!(sy, 1.0);
}

// ============================================================================
// TEST 9: MINIMUM SIZE CLAMPING AND ANTI-SINGULARITY GUARDS
// ============================================================================

#[test]
fn test_scale_factors_minimum_size_clamp_and_anti_singularity() {
    let bbox = Rect::new(100.0, 100.0, 300.0, 300.0); // 200x200

    // Drag BottomRight handle all the way to anchor (100.0, 100.0) -> scale zero
    let (_anchor, (sx, sy)) =
        SelectionState::compute_scale_factors(HandleId::BottomRight, &bbox, (100.0, 100.0), true);
    assert!(sx >= 0.05, "Scale factor must clamp to >= 0.05: got {}", sx);
    assert!(sy >= 0.05, "Scale factor must clamp to >= 0.05: got {}", sy);
    assert!(
        200.0 * sx >= 5.0,
        "Box width must remain >= 5.0 pt: got {}",
        200.0 * sx
    );
    assert!(
        200.0 * sy >= 5.0,
        "Box height must remain >= 5.0 pt: got {}",
        200.0 * sy
    );

    // Drag BottomRight past anchor to (-100.0, -100.0) -> negative scale
    let (_anchor, (sx_neg, sy_neg)) = SelectionState::compute_scale_factors(
        HandleId::BottomRight,
        &bbox,
        (-100.0, -100.0),
        false,
    );
    assert!(sx_neg >= 0.05);
    assert!(sy_neg >= 0.05);

    // Singular / tiny initial bounding box does not panic or produce NaN
    let tiny_bbox = Rect::new(50.0, 50.0, 50.0, 50.0);
    let (_anchor, (sx_tiny, sy_tiny)) = SelectionState::compute_scale_factors(
        HandleId::BottomRight,
        &tiny_bbox,
        (60.0, 60.0),
        true,
    );
    assert!(!sx_tiny.is_nan() && !sx_tiny.is_infinite());
    assert!(!sy_tiny.is_nan() && !sy_tiny.is_infinite());
}

// ============================================================================
// TEST 10: DOCCOMMAND::TRANSFORMITEMS COMMIT & UNDO/REDO REVERSIBILITY
// ============================================================================

#[test]
fn test_transform_items_command_undo_redo_and_spatial_index() {
    let mut doc = Document::new_notebook("Test Doc");
    let page_id = doc.pages[0].id;

    // Create a sample stroke: triangle (100, 100) -> (200, 100) -> (150, 200)
    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width: 2.0,
        ..Default::default()
    };
    let mut stroke = Stroke::new(101, style, PipelineMode::CatmullRomSpline);
    stroke.points.push(StrokePoint {
        x: 100.0,
        y: 100.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1000,
    });
    stroke.points.push(StrokePoint {
        x: 200.0,
        y: 100.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 2000,
    });
    stroke.points.push(StrokePoint {
        x: 150.0,
        y: 200.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 3000,
    });

    let item = Item {
        id: stroke.id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke.clone()),
    };

    let page = &mut doc.pages[0];
    page.items.push(item.clone());
    page.spatial_index.insert(&item);

    let mut history = CommandHistory::new(100);

    // Apply TransformItems: origin at (100.0, 100.0), scale_x = 2.0, scale_y = 2.0
    let cmd = DocCommand::TransformItems {
        page_id,
        item_ids: vec![stroke.id],
        origin_x: 100.0,
        origin_y: 100.0,
        scale_x: 2.0,
        scale_y: 2.0,
        rotation_rad: 0.0,
    };
    history.execute(cmd, &mut doc);

    // Verify points transformed
    let page = &doc.pages[0];
    let ItemKind::Stroke(ref transformed_stroke) = page.items[0].kind else {
        panic!("Expected stroke");
    };
    assert_eq!(transformed_stroke.points[0].x, 100.0); // 100 + (100 - 100)*2 = 100
    assert_eq!(transformed_stroke.points[0].y, 100.0);
    assert_eq!(transformed_stroke.points[1].x, 300.0); // 100 + (200 - 100)*2 = 300
    assert_eq!(transformed_stroke.points[1].y, 100.0);
    assert_eq!(transformed_stroke.points[2].x, 200.0); // 100 + (150 - 100)*2 = 200
    assert_eq!(transformed_stroke.points[2].y, 300.0); // 100 + (200 - 100)*2 = 300

    // Verify spatial index query at new coordinates
    let hits = page
        .spatial_index
        .locate_in_envelope(290.0, 90.0, 310.0, 110.0);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].item_id, stroke.id);

    // Undo transformation
    history.undo(&mut doc);
    let page = &doc.pages[0];
    let ItemKind::Stroke(ref reverted_stroke) = page.items[0].kind else {
        panic!("Expected stroke");
    };
    assert!((reverted_stroke.points[1].x - 200.0).abs() < 1e-4);
    assert!((reverted_stroke.points[2].y - 200.0).abs() < 1e-4);

    // Verify spatial index query after undo
    let hits_old = page
        .spatial_index
        .locate_in_envelope(190.0, 90.0, 210.0, 110.0);
    assert_eq!(hits_old.len(), 1);
    assert_eq!(hits_old[0].item_id, stroke.id);

    // Redo transformation
    history.redo(&mut doc);
    let page = &doc.pages[0];
    let ItemKind::Stroke(ref redone_stroke) = page.items[0].kind else {
        panic!("Expected stroke");
    };
    assert!((redone_stroke.points[1].x - 300.0).abs() < 1e-4);
    assert!((redone_stroke.points[2].y - 300.0).abs() < 1e-4);
}

// ============================================================================
// TEST 11: SEGMENT ERASER SLICE & DOCCOMMAND::SLICESTROKE REVERSIBILITY
// ============================================================================

#[test]
fn test_segment_eraser_slice_stroke_and_reversibility() {
    let mut doc = Document::new_notebook("Test Doc");
    let page_id = doc.pages[0].id;

    // Horizontal line stroke from (0, 50) to (100, 50) with 11 equidistant points
    let style = StrokeStyle {
        color: (0.0, 0.0, 0.0, 1.0),
        base_width: 2.0,
        ..Default::default()
    };
    let mut stroke = Stroke::new(500, style, PipelineMode::CatmullRomSpline);
    for i in 0..=10 {
        let x = i as f64 * 10.0;
        stroke.points.push(StrokePoint {
            x,
            y: 50.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: i * 1000,
        });
    }

    let orig_item = Item {
        id: stroke.id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke.clone()),
    };
    doc.pages[0].items.push(orig_item.clone());
    doc.pages[0].spatial_index.insert(&orig_item);

    // Slice in the middle with circle at (50.0, 50.0), radius 12.0
    let fragments = stroke.slice_with_circle((50.0, 50.0), 12.0);
    assert_eq!(
        fragments.len(),
        2,
        "Slicing in the middle must produce exactly 2 fragments"
    );

    // Assign IDs to fragments
    let mut rep_items = Vec::new();
    for (id_alloc, mut frag) in (501..).zip(fragments) {
        frag.id = id_alloc;
        rep_items.push(Item {
            id: frag.id,
            z_index: 0,
            kind: ItemKind::Stroke(frag),
        });
    }

    let mut history = CommandHistory::new(100);
    let cmd = DocCommand::SliceStroke {
        page_id,
        original_stroke: orig_item.clone(),
        new_strokes: rep_items.clone(),
    };
    history.execute(cmd, &mut doc);

    // Verify original stroke replaced by the two fragments
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert!(page.items.iter().any(|it| it.id == 501));
    assert!(page.items.iter().any(|it| it.id == 502));
    assert!(!page.items.iter().any(|it| it.id == 500));

    // Verify spatial index contains both fragments and not original
    assert_eq!(page.spatial_index.len(), 2);

    // Undo slicing: original restored, fragments removed
    history.undo(&mut doc);
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].id, 500);
    assert_eq!(page.spatial_index.len(), 1);

    // Redo slicing: fragments re-applied
    history.redo(&mut doc);
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.spatial_index.len(), 2);
}

// ============================================================================
// TEST 12: ERASERMODE ENUM AND CANVAS TOGGLE INTEGRATION
// ============================================================================

#[test]
fn test_eraser_mode_toggle_and_canvas_integration() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        // Default eraser mode is WholeStroke
        assert_eq!(canvas.eraser_mode(), EraserMode::WholeStroke);

        // Toggle to Segment
        canvas.set_eraser_mode(EraserMode::Segment);
        assert_eq!(canvas.eraser_mode(), EraserMode::Segment);

        // Toggle back to WholeStroke
        canvas.set_eraser_mode(EraserMode::WholeStroke);
        assert_eq!(canvas.eraser_mode(), EraserMode::WholeStroke);
    });
}

// ============================================================================
// TEST 13: SEGMENT ERASER HALO GRAZING & EMPTY MOTION NO MUTATION OR UNDO
// ============================================================================

#[test]
fn test_segment_eraser_halo_grazing_and_empty_motion_no_mutation_or_undo() {
    let mut st = CanvasState::new();
    st.document = Document::new_notebook("Test Notebook");
    st.eraser_mode = EraserMode::Segment;

    // Create a horizontal thick stroke from (0.0, 50.0) to (100.0, 50.0) with width 20.0.
    // Centerline is at y=50.0; outer visual halo reaches y = 50.0 +/- 10.0 => [40.0, 60.0].
    let style = StrokeStyle {
        color: (0.0, 0.0, 0.0, 1.0),
        base_width: 20.0,
        ..Default::default()
    };
    let mut stroke = Stroke::new(100, style, PipelineMode::CatmullRomSpline);
    for i in 0..=10 {
        let x = i as f64 * 10.0;
        stroke.points.push(StrokePoint {
            x,
            y: 50.0,
            pressure: 0.5,
            width: 20.0,
            timestamp_ns: i * 1000,
        });
    }

    let orig_item = Item {
        id: stroke.id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke.clone()),
    };

    st.document.pages[0].items.push(orig_item.clone());
    st.document.pages[0].spatial_index.insert(&orig_item);

    // Initial state: no undo steps, save status is Saved(0)
    assert!(!st.history.can_undo());
    assert_eq!(st.save_status, SaveStatus::Saved(0));

    // 1. Move segment eraser in completely empty space at (500.0, 500.0)
    NoteCanvas::erase_strokes_at(&mut st, (500.0, 500.0), 10.0);

    // Verify: no mutation, no undo step created, no persist_changes triggered
    assert!(
        !st.history.can_undo(),
        "Moving eraser in empty space must not create undo steps"
    );
    assert_eq!(
        st.save_status,
        SaveStatus::Saved(0),
        "Moving eraser in empty space must not call persist_changes()"
    );
    assert_eq!(st.document.pages[0].items.len(), 1);
    assert_eq!(st.document.pages[0].items[0].id, 100);

    // 2. Graze the outer halo of the thick stroke without cutting the centerline.
    // Centerline is at y=50.0. Outer halo boundary is at y=60.0.
    // Eraser at (50.0, 65.0) with radius 8.0:
    // Distance to centerline = 15.0 > 8.0 (circle does NOT intersect centerline).
    // Distance to visual ink = 15.0 - 10.0 = 5.0 <= 8.0 (circle overlaps outer halo: intersects_point is true!).
    {
        let page = &st.document.pages[0];
        let ItemKind::Stroke(ref s) = page.items[0].kind else {
            panic!("Expected stroke");
        };
        assert!(
            s.intersects_point((50.0, 65.0), 8.0),
            "Outer halo must be touched by circle"
        );
        let frags = s.slice_with_circle((50.0, 65.0), 8.0);
        assert_eq!(
            frags.len(),
            1,
            "Centerline is untouched so slice_with_circle returns unchanged stroke"
        );
        assert_eq!(frags[0].id, 100);
    }

    // Run erase_strokes_at at grazing position
    NoteCanvas::erase_strokes_at(&mut st, (50.0, 65.0), 8.0);

    // Verify: stroke was NOT sliced or mutated, NO undo steps created, persist_changes NOT called
    assert!(
        !st.history.can_undo(),
        "Grazing outer halo must not create undo steps"
    );
    assert_eq!(
        st.save_status,
        SaveStatus::Saved(0),
        "Grazing outer halo must not call persist_changes()"
    );
    {
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, 100);
        let ItemKind::Stroke(ref s_after) = page.items[0].kind else {
            panic!("Expected stroke");
        };
        assert_eq!(s_after.points.len(), 11);
        assert_eq!(s_after.id, 100);
    }

    // 3. Now cut across the centerline at (50.0, 50.0) with radius 5.0
    NoteCanvas::erase_strokes_at(&mut st, (50.0, 50.0), 5.0);

    // Verify stroke is now sliced into 2 fragments, 1 undo step created, persist_changes called
    assert!(
        st.history.can_undo(),
        "Actual centerline cut must create an undo step"
    );
    assert_eq!(
        st.save_status,
        SaveStatus::Saving(1),
        "Actual centerline cut must invoke persist_changes()"
    );
    {
        let page = &st.document.pages[0];
        assert_eq!(
            page.items.len(),
            2,
            "Stroke should now be cut into 2 fragments"
        );
        assert!(
            !page.items.iter().any(|it| it.id == 100),
            "Original stroke ID 100 should be replaced by fragment IDs"
        );
    }

    // 4. Verify undo restores original stroke with original ID 100
    assert!(st.undo());
    {
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, 100);
    }

    // Undo stack is now exhausted (only 1 undo step was recorded)
    assert!(
        !st.undo(),
        "Only 1 undo step should have been recorded for the actual cut"
    );

    // 5. Verify redo re-applies the slicing
    assert!(st.redo());
    {
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 2);
    }
}
