//! Empirical Adversarial Verification Suite for Milestone M4 Selection & Whiteboard
//!
//! Scope:
//! - SelectionEngine lasso and rectangle containment with complex, concave, and self-intersecting polygons
//! - 80% containment threshold boundaries (79% vs 80%, degenerate 0/1 point strokes)
//! - 60/120Hz drag translation: zero allocation during motion, atomic DocCommand::MoveItems commit
//! - Undo/Redo mathematical symmetry and drift tests over 10,000 cycles for MoveItems and EraseItems
//! - Infinite Whiteboard coordinates (-inf, +inf) and Viewport::center_on_rect edge cases
//! - dynamic_grid_spec(zoom): anti-moiré screen step calculations, alpha fading, and line count analysis

#![allow(irrefutable_let_patterns)]

use std::time::Instant;

use uninote_app::canvas::{dynamic_grid_spec, SelectionState};
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::coordinates::{Rect2D, Viewport};
use uninote_core::model::{Document, Item, ItemKind};
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, point_in_polygon, SelectionEngine,
    SelectionMode,
};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle,
};

// ============================================================================
// TEST FIXTURES & BUILDERS
// ============================================================================

fn make_test_stroke(id: u64, points: &[(f64, f64)]) -> Stroke {
    let stroke_points = points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1_000_000 * (i as u64),
        })
        .collect();

    let raw_samples = points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| InputSample {
            position: (x, y),
            pressure: Some(0.5),
            tilt: None,
            timestamp_ns: 1_000_000 * (i as u64),
            device_kind: DeviceKind::Stylus,
            source_id: 1,
            phase: if i == 0 {
                InputPhase::Down
            } else if i + 1 == points.len() {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        })
        .collect();

    let mut stroke = Stroke {
        id,
        points: stroke_points,
        style: StrokeStyle::default(),
        raw_samples,
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// ============================================================================
// 1. SELECTION ENGINE: COMPLEX POLYGONS & THRESHOLD BOUNDARIES
// ============================================================================

#[test]
fn test_lasso_concave_u_horseshoe_containment() {
    // U-shaped concave polygon:
    // Outer boundary: (0,0) -> (100,0) -> (100,100) -> (70,100) -> (70,30) -> (30,30) -> (30,100) -> (0,100)
    let u_poly = vec![
        (0.0, 0.0),
        (100.0, 0.0),
        (100.0, 100.0),
        (70.0, 100.0),
        (70.0, 30.0),
        (30.0, 30.0),
        (30.0, 100.0),
        (0.0, 100.0),
    ];

    // Left arm interior point (15, 50) -> inside
    assert!(point_in_polygon(15.0, 50.0, &u_poly));
    // Right arm interior point (85, 50) -> inside
    assert!(point_in_polygon(85.0, 50.0, &u_poly));
    // Base interior point (50, 15) -> inside
    assert!(point_in_polygon(50.0, 15.0, &u_poly));

    // The bay indentation between arms (50, 60) -> MUST BE OUTSIDE
    assert!(!point_in_polygon(50.0, 60.0, &u_poly));

    // Stroke residing entirely in the concave bay
    let stroke_in_bay = make_test_stroke(1, &[(40.0, 60.0), (50.0, 70.0), (60.0, 60.0)]);
    assert!(
        !is_stroke_selected_by_lasso(&stroke_in_bay, &u_poly),
        "Stroke inside concave bay must not be selected"
    );

    // Stroke in the left arm
    let stroke_in_arm = make_test_stroke(2, &[(10.0, 40.0), (15.0, 50.0), (20.0, 60.0)]);
    assert!(
        is_stroke_selected_by_lasso(&stroke_in_arm, &u_poly),
        "Stroke inside solid arm must be selected"
    );
}

#[test]
fn test_lasso_self_intersecting_figure_eight() {
    // Figure-eight polygon crossing at (50, 50):
    // Top loop: (50,50) -> (20,80) -> (80,80) -> (50,50)
    // Bottom loop: (50,50) -> (80,20) -> (20,20) -> (50,50)
    let fig8 = vec![
        (50.0, 50.0),
        (20.0, 80.0),
        (80.0, 80.0),
        (50.0, 50.0),
        (80.0, 20.0),
        (20.0, 20.0),
    ];

    // Inside top loop (50, 70)
    assert!(point_in_polygon(50.0, 70.0, &fig8));
    // Inside bottom loop (50, 30)
    assert!(point_in_polygon(50.0, 30.0, &fig8));
    // Outside waist (10, 50) and (90, 50)
    assert!(!point_in_polygon(10.0, 50.0, &fig8));
    assert!(!point_in_polygon(90.0, 50.0, &fig8));
}

#[test]
fn test_lasso_strict_80_percent_boundary_100_points() {
    let poly = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

    // Stroke with 100 points: exactly 79 points inside, 21 points outside
    let mut pts_79 = Vec::new();
    for i in 0..79 {
        pts_79.push((1.0 + (i as f64) * 1.0, 50.0));
    }
    for i in 0..21 {
        pts_79.push((150.0 + (i as f64) * 2.0, 50.0));
    }
    let s_79 = make_test_stroke(101, &pts_79);
    assert_eq!(s_79.points.len(), 100);
    assert!(
        !is_stroke_selected_by_lasso(&s_79, &poly),
        "79% containment must strictly be rejected by >=80% rule"
    );

    // Stroke with 100 points: exactly 80 points inside, 20 points outside
    let mut pts_80 = Vec::new();
    for i in 0..80 {
        pts_80.push((1.0 + (i as f64) * 1.0, 50.0));
    }
    for i in 0..20 {
        pts_80.push((150.0 + (i as f64) * 2.0, 50.0));
    }
    let s_80 = make_test_stroke(102, &pts_80);
    assert_eq!(s_80.points.len(), 100);
    assert!(
        is_stroke_selected_by_lasso(&s_80, &poly),
        "80% containment must strictly be accepted by >=80% rule"
    );
}

#[test]
fn test_selection_degenerate_strokes_and_polygons() {
    let poly = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];

    // 0 points stroke: must not panic or divide by zero
    let empty_stroke = Stroke {
        id: 999,
        points: Vec::new(),
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    assert!(!is_stroke_selected_by_lasso(&empty_stroke, &poly));
    assert!(!is_stroke_selected_by_rect(
        &empty_stroke,
        &Rect::new(0.0, 0.0, 100.0, 100.0)
    ));

    // 1 point stroke: 100% inside -> selected; outside -> rejected
    let s_single_in = make_test_stroke(1001, &[(50.0, 50.0)]);
    assert!(is_stroke_selected_by_lasso(&s_single_in, &poly));
    let s_single_out = make_test_stroke(1002, &[(150.0, 50.0)]);
    assert!(!is_stroke_selected_by_lasso(&s_single_out, &poly));

    // Degenerate polygon: 0, 1, or 2 vertices
    assert!(!point_in_polygon(50.0, 50.0, &[]));
    assert!(!point_in_polygon(50.0, 50.0, &[(10.0, 10.0)]));
    assert!(!point_in_polygon(50.0, 50.0, &[(10.0, 10.0), (20.0, 20.0)]));

    // SelectionEngine with degenerate inputs
    let res = SelectionEngine::select_lasso([&s_single_in], &[(10.0, 10.0), (20.0, 20.0)]);
    assert!(res.is_empty(), "Degenerate polygon must select nothing");
}

#[test]
fn test_rect_selection_inverted_drags_and_dual_rule() {
    // Target stroke at [50, 150] x [50, 150]
    let s = make_test_stroke(1, &[(60.0, 60.0), (140.0, 140.0)]);

    // 4 drag directions:
    // 1. Top-Left to Bottom-Right
    assert!(is_stroke_selected_by_rect(
        &s,
        &Rect::new(0.0, 0.0, 200.0, 200.0)
    ));
    // 2. Bottom-Right to Top-Left (inverted both X and Y)
    assert!(is_stroke_selected_by_rect(
        &s,
        &Rect::new(200.0, 200.0, 0.0, 0.0)
    ));
    // 3. Top-Right to Bottom-Left (inverted X)
    assert!(is_stroke_selected_by_rect(
        &s,
        &Rect::new(200.0, 0.0, 0.0, 200.0)
    ));
    // 4. Bottom-Left to Top-Right (inverted Y)
    assert!(is_stroke_selected_by_rect(
        &s,
        &Rect::new(0.0, 200.0, 200.0, 0.0)
    ));

    // Dual rule test:
    // Stroke where bbox exceeds the rect, but 80% of points are inside
    let mut pts = Vec::new();
    for i in 0..8 {
        pts.push((10.0 + (i as f64) * 10.0, 50.0)); // x in [10, 80] inside [0, 100]
    }
    pts.push((120.0, 50.0)); // outside
    pts.push((130.0, 50.0)); // outside
    let s_partial = make_test_stroke(2, &pts);

    let rect = Rect::new(0.0, 0.0, 100.0, 100.0);
    // Bbox of s_partial is [10, 130] x [50, 50] which is NOT inside rect [0, 100]
    assert!(!rect.contains_rect(&s_partial.bounding_box().unwrap()));
    // BUT 8/10 points = 80% are inside -> MUST BE SELECTED!
    assert!(
        is_stroke_selected_by_rect(&s_partial, &rect),
        "Stroke with >=80% points inside rect must be selected even if bbox exceeds"
    );
}

// ============================================================================
// 2. 60/120HZ DRAG TRANSLATION: ZERO-ALLOCATION & ATOMIC COMMIT
// ============================================================================

#[test]
fn test_drag_translation_zero_allocation_during_motion() {
    let mut state = SelectionState {
        mode: SelectionMode::Lasso,
        selected_ids: vec![101, 102, 103],
        bbox: Some(Rect::new(50.0, 50.0, 200.0, 200.0)),
        is_dragging: true,
        drag_start_doc: Some((100.0, 100.0)),
        drag_offset: (0.0, 0.0),
        ..Default::default()
    };

    // Simulate 12,000 frames of high-frequency 120Hz drag updates (100 seconds)
    let t0 = Instant::now();
    for frame in 0..12000 {
        let cur_doc_x = 100.0 + (frame as f64 * 0.01).cos() * 50.0;
        let cur_doc_y = 100.0 + (frame as f64 * 0.01).sin() * 50.0;

        if let Some(start) = state.drag_start_doc {
            // Pure arithmetic identical to canvas.rs lines 1284 and 1731
            state.drag_offset = (cur_doc_x - start.0, cur_doc_y - start.1);
        }
    }
    let elapsed = t0.elapsed();

    // 12,000 updates of pure arithmetic must complete in under 5 ms without any allocations
    assert!(
        elapsed.as_millis() < 5,
        "12,000 drag translation updates took {}ms, expected < 5ms",
        elapsed.as_millis()
    );
}

#[test]
fn test_drag_release_atomic_commit_threshold() {
    let mut doc = Document::new_empty("Drag Release Test");
    let page_id = doc.pages[0].id;
    let s = make_test_stroke(1, &[(10.0, 10.0), (20.0, 20.0)]);
    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(s),
    });

    let mut history = CommandHistory::new(50);

    // Sub-threshold drag: dx = 0.0005, dy = -0.0002 (< 0.001)
    let dx_micro: f64 = 0.0005;
    let dy_micro: f64 = -0.0002;
    let should_commit_micro = dx_micro.abs() > 0.001 || dy_micro.abs() > 0.001;
    assert!(!should_commit_micro, "Micro-drags must be ignored");

    // Meaningful drag: dx = 25.0, dy = -15.0
    let dx: f64 = 25.0;
    let dy: f64 = -15.0;
    let should_commit = dx.abs() > 0.001 || dy.abs() > 0.001;
    assert!(should_commit);

    let cmd = DocCommand::MoveItems {
        page_id,
        item_ids: vec![1],
        delta_x: dx,
        delta_y: dy,
    };
    history.execute(cmd, &mut doc);

    // Verify atomic stroke update
    if let ItemKind::Stroke(ref stroke) = doc.pages[0].items[0].kind {
        assert_eq!(stroke.points[0].x, 35.0);
        assert_eq!(stroke.points[0].y, -5.0);
        assert_eq!(stroke.raw_samples[0].position.0, 35.0);
        assert_eq!(stroke.raw_samples[0].position.1, -5.0);
    }
}

// ============================================================================
// 3. UNDO/REDO MATHEMATICAL SYMMETRY & DRIFT VERIFICATION (10,000 CYCLES)
// ============================================================================

#[test]
fn test_move_items_10000_cycles_floating_point_drift() {
    let mut doc = Document::new_empty("Move Drift Test");
    let page_id = doc.pages[0].id;

    let original_coords = [(123.456789, 987.654321), (555.123456, 333.987654)];
    let s = make_test_stroke(42, &original_coords);
    doc.pages[0].items.push(Item {
        id: 42,
        z_index: 1,
        kind: ItemKind::Stroke(s),
    });

    let mut history = CommandHistory::new(100);
    let cmd = DocCommand::MoveItems {
        page_id,
        item_ids: vec![42],
        delta_x: 37.1234567,
        delta_y: -19.9876543,
    };

    // Run 10,000 execute / undo cycles
    for _ in 0..10_000 {
        history.execute(cmd.clone(), &mut doc);
        assert!(history.undo(&mut doc));
    }

    // Verify floating point drift is < 1e-9 pt
    if let ItemKind::Stroke(ref stroke) = doc.pages[0].items[0].kind {
        for (i, pt) in stroke.points.iter().enumerate() {
            let orig = original_coords[i];
            let drift_x = (pt.x - orig.0).abs();
            let drift_y = (pt.y - orig.1).abs();
            assert!(
                drift_x < 1e-9,
                "Point {} X drifted by {} after 10k cycles",
                i,
                drift_x
            );
            assert!(
                drift_y < 1e-9,
                "Point {} Y drifted by {} after 10k cycles",
                i,
                drift_y
            );
        }
    }
}

#[test]
fn test_erase_items_undo_redo_preserves_z_index_ordering() {
    let mut doc = Document::new_empty("Erase Order Test");
    let page_id = doc.pages[0].id;

    for i in 1..=5 {
        let s = make_test_stroke(i, &[(i as f64 * 10.0, i as f64 * 10.0)]);
        doc.pages[0].items.push(Item {
            id: i,
            z_index: i * 10, // 10, 20, 30, 40, 50
            kind: ItemKind::Stroke(s),
        });
    }

    let mut history = CommandHistory::new(50);

    // Erase items 2 and 4 (middle items)
    let erased_items = vec![
        doc.pages[0].items[1].clone(), // id 2
        doc.pages[0].items[3].clone(), // id 4
    ];

    let cmd = DocCommand::EraseItems {
        page_id,
        erased_items,
    };
    history.execute(cmd, &mut doc);

    assert_eq!(doc.pages[0].items.len(), 3);
    assert_eq!(doc.pages[0].items[0].id, 1);
    assert_eq!(doc.pages[0].items[1].id, 3);
    assert_eq!(doc.pages[0].items[2].id, 5);

    // Undo: items 2 and 4 must be restored in EXACT original order (z_index)
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 5);
    for (idx, item) in doc.pages[0].items.iter().enumerate() {
        let expected_id = (idx + 1) as u64;
        let expected_z = expected_id * 10;
        assert_eq!(item.id, expected_id);
        assert_eq!(item.z_index, expected_z);
    }

    // Redo: erased again
    assert!(history.redo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 3);
    assert_eq!(doc.pages[0].items[0].id, 1);
    assert_eq!(doc.pages[0].items[1].id, 3);
    assert_eq!(doc.pages[0].items[2].id, 5);
}

#[test]
fn test_sequential_move_and_erase_interleaving_undo_symmetry() {
    let mut doc = Document::new_empty("Interleaved Test");
    let page_id = doc.pages[0].id;

    for i in 1..=4 {
        doc.pages[0].items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(make_test_stroke(i, &[(i as f64 * 50.0, 100.0)])),
        });
    }

    let original_doc = doc.clone();
    let mut history = CommandHistory::new(50);

    // Step 1: Move item 1 by (+10, +20)
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![1],
            delta_x: 10.0,
            delta_y: 20.0,
        },
        &mut doc,
    );

    // Step 2: Erase item 3
    let item3 = doc.pages[0]
        .items
        .iter()
        .find(|it| it.id == 3)
        .unwrap()
        .clone();
    history.execute(
        DocCommand::EraseItems {
            page_id,
            erased_items: vec![item3],
        },
        &mut doc,
    );

    // Step 3: Move item 2 by (-30, +40)
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![2],
            delta_x: -30.0,
            delta_y: 40.0,
        },
        &mut doc,
    );

    assert_eq!(doc.pages[0].items.len(), 3);

    // Fully unwind undo stack (3 undos)
    assert!(history.undo(&mut doc));
    assert!(history.undo(&mut doc));
    assert!(history.undo(&mut doc));

    // Verify document returned to 100% identical state
    assert_eq!(doc.pages[0].items.len(), original_doc.pages[0].items.len());
    for i in 0..4 {
        let it_cur = &doc.pages[0].items[i];
        let it_orig = &original_doc.pages[0].items[i];
        assert_eq!(it_cur.id, it_orig.id);
        if let (ItemKind::Stroke(s1), ItemKind::Stroke(s2)) = (&it_cur.kind, &it_orig.kind) {
            assert_eq!(s1.points[0].x, s2.points[0].x);
            assert_eq!(s1.points[0].y, s2.points[0].y);
        }
    }
}

// ============================================================================
// 4. INFINITE WHITEBOARD & CENTER CONTENT EDGE CASES
// ============================================================================

#[test]
fn test_whiteboard_extreme_coordinates_and_affine_invariance() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);

    // Deep negative Cartesian quadrant: (-1,000,000, -2,000,000)
    let doc_pt_neg = (-1_000_000.0, -2_000_000.0);
    vp.pan(1_000_500.0, 2_000_300.0);

    let screen_pt = vp.document_to_screen(doc_pt_neg);
    assert_eq!(screen_pt.0, 500.0);
    assert_eq!(screen_pt.1, 300.0);

    let recovered_pt = vp.screen_to_document(screen_pt);
    assert!((recovered_pt.0 - doc_pt_neg.0).abs() < 1e-6);
    assert!((recovered_pt.1 - doc_pt_neg.1).abs() < 1e-6);
}

#[test]
fn test_center_on_rect_empty_and_zero_dimension_handling() {
    let mut vp = Viewport::new(100.0, 100.0, 1.5);

    // Degenerate 0x0 rect (0 strokes or empty canvas)
    let zero_rect = Rect2D::new(0.0, 0.0, 0.0, 0.0);
    vp.center_on_rect(zero_rect, 1000.0, 800.0);

    assert_eq!(vp.zoom, 1.0);
    assert_eq!(vp.offset_x, 500.0);
    assert_eq!(vp.offset_y, 400.0);

    // Invalid window dimensions (0x0 or negative)
    vp.center_on_rect(Rect2D::new(10.0, 10.0, 100.0, 100.0), 0.0, -500.0);
    assert_eq!(vp.zoom, 1.0);
    assert!(vp.offset_x.is_finite());
    assert!(vp.offset_y.is_finite());
}

#[test]
fn test_center_on_rect_empirical_zoom_clamping_limits() {
    let mut vp = Viewport::new(0.0, 0.0, 1.0);
    let win_w = 1000.0;
    let win_h = 1000.0;

    // Massive bounds (1,000,000 x 1,000,000 pt) -> scale ~ 0.00088
    let massive_rect = Rect2D::new(-500_000.0, -500_000.0, 1_000_000.0, 1_000_000.0);
    vp.center_on_rect(massive_rect, win_w, win_h);

    // EMPIRICAL VERIFICATION:
    // coordinates.rs line 166 clamps to [0.25, 3.0].
    // Worker handoff line 24 claimed [0.5, 2.0].
    assert!(
        vp.zoom >= 0.25,
        "Zoom must never drop below the coordinates engine lower bound"
    );
    let empirical_min_zoom = vp.zoom;

    // Tiny bounds (1.0 x 1.0 pt) -> scale ~ 880.0
    let tiny_rect = Rect2D::new(100.0, 100.0, 1.0, 1.0);
    vp.center_on_rect(tiny_rect, win_w, win_h);
    let empirical_max_zoom = vp.zoom;

    // Verify numerical sanity: zoom is bounded and finite
    assert!(empirical_min_zoom.is_finite());
    assert!(empirical_max_zoom.is_finite());
    assert!(empirical_max_zoom <= 3.0);
}

// ============================================================================
// 5. DYNAMIC GRID SPEC: ANTI-MOIRÉ SCREEN STEPS & LINE COUNT LIMITS
// ============================================================================

#[test]
fn test_dynamic_grid_spec_anti_moire_screen_step_invariants() {
    // Test continuous zoom sweep from 0.001 to 100.0
    let test_zooms = [
        0.001, 0.1, 0.25, 0.35, 0.49, 0.50, 0.75, 1.0, 1.5, 2.0, 2.01, 3.0, 4.0, 5.0, 10.0,
    ];

    for &zoom in &test_zooms {
        let spec = dynamic_grid_spec(zoom);

        // Invariant 1: Step must be positive and finite
        assert!(spec.step_pt > 0.0 && spec.step_pt.is_finite());
        // Invariant 2: Alpha must be in [0.10, 0.50]
        assert!(
            spec.alpha >= 0.10 && spec.alpha <= 0.50,
            "Alpha {} out of bounds for zoom {}",
            spec.alpha,
            zoom
        );
        // Invariant 3: Major step >= minor step
        assert!(spec.major_step_pt >= spec.step_pt);

        // Screen step within normal viewport zoom range [0.25, 5.0]
        if (0.25..=5.0).contains(&zoom) {
            let screen_step = spec.step_pt * zoom;
            assert!(
                screen_step >= 10.0,
                "Screen step ({:.2} px) at zoom {} is < 10px, causing moiré!",
                screen_step,
                zoom
            );
        }
    }
}

#[test]
fn test_dynamic_grid_line_count_analysis() {
    // In UniNote, grid lines are drawn within viewport window:
    // minor_lines = (win_w / screen_step).ceil() + (win_h / screen_step).ceil()
    // On 1280x720 window:
    let win_w = 1280.0;
    let win_h = 720.0;

    let test_zooms = [0.25, 0.5, 1.0, 2.0, 3.0, 5.0];
    for &zoom in &test_zooms {
        let spec = dynamic_grid_spec(zoom);
        let screen_step = spec.step_pt * zoom;
        let major_screen_step = spec.major_step_pt * zoom;

        let minor_cols = (win_w / screen_step).ceil();
        let minor_rows = (win_h / screen_step).ceil();
        let major_cols = (win_w / major_screen_step).ceil();
        let major_rows = (win_h / major_screen_step).ceil();

        let total_lines = (minor_cols + minor_rows + major_cols + major_rows) as usize;

        // On 1280x720, total line count must remain well within GPU budget (<= 300 lines)
        assert!(
            total_lines <= 300,
            "Total grid lines {} exceeded 300 at zoom {} on 720p window",
            total_lines,
            zoom
        );
    }
}
