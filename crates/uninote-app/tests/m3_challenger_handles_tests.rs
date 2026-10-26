//! Adversarial Verification Test Suite for Milestone M3
//! (Selection Handles, Screen-Space Hit Testing, Scaling Geometry & Zoom Invariance)
//!
//! Tests:
//! 1. Screen-space hit testing at extreme zoom levels: 0.05x, 0.1x, 0.5x, 1.0x, 5.0x, 20.0x.
//!    Verifies 15px tolerance radius strictly invariant in screen space.
//! 2. Hit test priority: Corner vs Edge Pill vs Grip vs Body vs None.
//!    Verifies strict precedence and zero interior dead zones.
//! 3. Decluttering threshold: < 32px screen suppression of edge pills while corners remain active.
//! 4. Anti-singularity & degenerate shapes: w=0, h=0, point bbox, dragging past anchor (negative scale),
//!    minimum scale clamp (0.05), minimum dimension clamp (5.0 pt).
//! 5. Aspect ratio locking precision: bit-for-bit (sx == sy) under diagonal projection for all corners.
//! 6. Full interactive CanvasState drag lifecycle and command reversibility (TransformItems Undo/Redo).

#![allow(irrefutable_let_patterns)]

use std::f64::consts::PI;

use uninote_app::canvas::{CanvasState, NoteCanvas, SelectionState};
use uninote_app::selection::{
    compute_scale_factors, HandleId, SelectionDragKind, SelectionHitTestResult,
};
use uninote_core::coordinates::Viewport;
use uninote_core::model::{Item, ItemKind};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// 1. SCREEN-SPACE HIT TESTING AT EXTREME ZOOM LEVELS
// ============================================================================

#[test]
fn test_adv_screenspace_hit_radius_invariance_across_extreme_zooms() {
    let sel = SelectionState {
        bbox: Some(Rect::new(200.0, 300.0, 600.0, 800.0)),
        ..Default::default()
    };

    let extreme_zooms = [0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 20.0];
    let test_viewports = [(0.0, 0.0), (350.0, -120.0), (-1500.0, 2400.0)];

    type CornerProbe = (HandleId, (f64, f64), (f64, f64));

    for &zoom in &extreme_zooms {
        for &(pan_x, pan_y) in &test_viewports {
            let vp = Viewport::new(pan_x, pan_y, zoom);

            // Test all 4 corners
            let corners: [CornerProbe; 4] = [
                (HandleId::TopLeft, (200.0, 300.0), (-1.0, -1.0)),
                (HandleId::TopRight, (600.0, 300.0), (1.0, -1.0)),
                (HandleId::BottomRight, (600.0, 800.0), (1.0, 1.0)),
                (HandleId::BottomLeft, (200.0, 800.0), (-1.0, 1.0)),
            ];

            for (handle_id, corner_doc, outward_dir) in corners {
                let corner_screen = vp.document_to_screen(corner_doc);

                // 1. Exact corner hit (r = 0.0)
                let hit_center = sel.hit_test(corner_doc, &vp);
                assert_eq!(
                    hit_center,
                    SelectionHitTestResult::Handle(handle_id),
                    "Exact corner must hit handle at zoom {}",
                    zoom
                );

                // 2. Radial probe in 16 directions around corner at r = 10.0 px and r = 14.9 px
                for step in 0..16 {
                    let angle = (step as f64) * (2.0 * PI / 16.0);
                    let (dx, dy) = (angle.cos(), angle.sin());

                    // Inside tolerance: r = 10.0 px
                    let pt_screen_10 = (corner_screen.0 + dx * 10.0, corner_screen.1 + dy * 10.0);
                    let pt_doc_10 = vp.screen_to_document(pt_screen_10);
                    let hit_10 = sel.hit_test(pt_doc_10, &vp);
                    assert_eq!(
                        hit_10,
                        SelectionHitTestResult::Handle(handle_id),
                        "Probe at r=10px must hit corner handle {:?} at zoom {} (angle {})",
                        handle_id,
                        zoom,
                        angle
                    );

                    // Just inside tolerance: r = 14.9 px
                    let pt_screen_14_9 = (corner_screen.0 + dx * 14.9, corner_screen.1 + dy * 14.9);
                    let pt_doc_14_9 = vp.screen_to_document(pt_screen_14_9);
                    let hit_14_9 = sel.hit_test(pt_doc_14_9, &vp);
                    assert_eq!(
                        hit_14_9,
                        SelectionHitTestResult::Handle(handle_id),
                        "Probe at r=14.9px must hit corner handle {:?} at zoom {} (angle {})",
                        handle_id,
                        zoom,
                        angle
                    );
                }

                // 3. Exact boundary: r = 15.000 px
                let pt_screen_15 = (corner_screen.0 + 15.0, corner_screen.1);
                let pt_doc_15 = vp.screen_to_document(pt_screen_15);
                let hit_15 = sel.hit_test(pt_doc_15, &vp);
                assert_eq!(
                    hit_15,
                    SelectionHitTestResult::Handle(handle_id),
                    "Boundary r=15.0px must hit corner handle {:?} at zoom {}",
                    handle_id,
                    zoom
                );

                // 4. Outside tolerance: probe outwards from corner (away from bbox) at r = 15.5 px and r = 25.0 px
                let (norm_ox, norm_oy) = {
                    let len =
                        (outward_dir.0 * outward_dir.0 + outward_dir.1 * outward_dir.1).sqrt();
                    (outward_dir.0 / len, outward_dir.1 / len)
                };

                let pt_screen_15_5 = (
                    corner_screen.0 + norm_ox * 15.5,
                    corner_screen.1 + norm_oy * 15.5,
                );
                let pt_doc_15_5 = vp.screen_to_document(pt_screen_15_5);
                let hit_15_5 = sel.hit_test(pt_doc_15_5, &vp);
                assert_ne!(
                    hit_15_5,
                    SelectionHitTestResult::Handle(handle_id),
                    "Probe at r=15.5px outwards must not hit corner handle {:?} at zoom {}",
                    handle_id,
                    zoom
                );

                // Far outside: r = 35.0 px outwards
                let pt_screen_35 = (
                    corner_screen.0 + norm_ox * 35.0,
                    corner_screen.1 + norm_oy * 35.0,
                );
                let pt_doc_35 = vp.screen_to_document(pt_screen_35);
                let hit_35 = sel.hit_test(pt_doc_35, &vp);
                assert_eq!(
                    hit_35,
                    SelectionHitTestResult::None,
                    "Probe at r=35px outwards must be None at zoom {}",
                    zoom
                );
            }
        }
    }
}

// ============================================================================
// 2. HIT TEST PRIORITY AND ZERO DEAD ZONES
// ============================================================================

#[test]
fn test_adv_hit_test_strict_precedence_and_zero_interior_dead_zones() {
    let bbox = Rect::new(100.0, 100.0, 500.0, 500.0);
    let sel = SelectionState {
        bbox: Some(bbox),
        ..Default::default()
    };
    let vp = Viewport::new(0.0, 0.0, 1.0); // 1:1 screen mapping

    // --- A. Strict Precedence: Corner over Edge Pill ---
    // At small boxes, corner 15px radius and edge pill 14x10 zone could overlap.
    // Corner is checked first, so corner must strictly take precedence.
    let small_bbox = Rect::new(100.0, 100.0, 136.0, 136.0); // 36x36 px (> 32px so pills active)
    let small_sel = SelectionState {
        bbox: Some(small_bbox),
        ..Default::default()
    };
    // TopLeft corner is at (100, 100). TopMiddle is at (118, 100).
    // Test point at (110, 100):
    // Distance to TopLeft is 10px (<= 15px).
    // Distance to TopMiddle is 8px (<= 14px PILL_HIT_X).
    // Must resolve to TopLeft because corner has absolute priority.
    let hit_overlap = small_sel.hit_test((110.0, 100.0), &vp);
    assert_eq!(
        hit_overlap,
        SelectionHitTestResult::Handle(HandleId::TopLeft),
        "Corner handle must take precedence over edge pill in overlap region"
    );

    // --- B. Edge Pill Extents and Precedence over Body ---
    // Midpoints on original 400x400 bbox:
    // TopMiddle at (300.0, 100.0)
    assert_eq!(
        sel.hit_test((300.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );
    // At edge of pill hit zone: dx = 13.9, dy = 9.9
    assert_eq!(
        sel.hit_test((313.9, 109.9), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );
    assert_eq!(
        sel.hit_test((286.1, 90.1), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );

    // RightMiddle at (500.0, 300.0) - vertical pill: dx <= 10, dy <= 14
    assert_eq!(
        sel.hit_test((500.0, 300.0), &vp),
        SelectionHitTestResult::Handle(HandleId::RightMiddle)
    );
    assert_eq!(
        sel.hit_test((509.9, 313.9), &vp),
        SelectionHitTestResult::Handle(HandleId::RightMiddle)
    );

    // BottomMiddle at (300.0, 500.0)
    assert_eq!(
        sel.hit_test((300.0, 500.0), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomMiddle)
    );

    // LeftMiddle at (100.0, 300.0)
    assert_eq!(
        sel.hit_test((100.0, 300.0), &vp),
        SelectionHitTestResult::Handle(HandleId::LeftMiddle)
    );

    // --- C. Center Grip and Body Precedence ---
    // Center point (300, 300) hits Grip -> Body
    assert_eq!(
        sel.hit_test((300.0, 300.0), &vp),
        SelectionHitTestResult::Body
    );
    // 14.5 px from center -> Body
    assert_eq!(
        sel.hit_test((314.5, 300.0), &vp),
        SelectionHitTestResult::Body
    );

    // General interior point (away from handles and grip): (200.0, 200.0) -> Body
    assert_eq!(
        sel.hit_test((200.0, 200.0), &vp),
        SelectionHitTestResult::Body
    );

    // --- D. Zero Dead Zones: Dense Interior Raster Probe ---
    // Every single interior point must return Handle or Body. Zero interior points may return None.
    for y in (105..=495).step_by(10) {
        for x in (105..=495).step_by(10) {
            let hit = sel.hit_test((x as f64, y as f64), &vp);
            assert_ne!(
                hit,
                SelectionHitTestResult::None,
                "Interior point ({}, {}) resulted in dead zone (None)",
                x,
                y
            );
        }
    }

    // --- E. Exterior Boundaries: Strict None ---
    // Points far outside bounding box and handles
    let exterior_points = [
        (50.0, 50.0),
        (550.0, 50.0),
        (550.0, 550.0),
        (50.0, 550.0),
        (300.0, 50.0),
        (300.0, 550.0),
        (50.0, 300.0),
        (550.0, 300.0),
    ];
    for &(ex, ey) in &exterior_points {
        assert_eq!(
            sel.hit_test((ex, ey), &vp),
            SelectionHitTestResult::None,
            "Exterior point ({}, {}) must return None",
            ex,
            ey
        );
    }
}

// ============================================================================
// 3. DECLUTTERING UNDER 32PX SCREEN THRESHOLD
// ============================================================================

#[test]
fn test_adv_decluttering_boundary_and_zoom_transition() {
    let vp = Viewport::new(0.0, 0.0, 1.0);

    // Case 1: width < 32px (31.99px), height = 100px
    let sel_narrow = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 131.99, 200.0)),
        ..Default::default()
    };
    // Edge pills must be suppressed!
    // TopMiddle is at (115.995, 100.0), distance to corners is ~16.0 px (> 15 px).
    // Since pills are decluttered, contact on top edge falls through to Body!
    let hit_narrow_mid = sel_narrow.hit_test((115.995, 100.0), &vp);
    assert_ne!(
        hit_narrow_mid,
        SelectionHitTestResult::Handle(HandleId::TopMiddle),
        "Edge pill TopMiddle must be suppressed when width < 32px"
    );
    assert_eq!(
        hit_narrow_mid,
        SelectionHitTestResult::Body,
        "Decluttered edge contact must fall through to Body"
    );
    // Corners must still be active!
    assert_eq!(
        sel_narrow.hit_test((100.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );
    assert_eq!(
        sel_narrow.hit_test((131.99, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopRight)
    );

    // Case 2: height < 32px (31.99px), width = 100px
    let sel_short = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 200.0, 131.99)),
        ..Default::default()
    };
    // LeftMiddle is at (100.0, 115.995). Pills suppressed!
    let hit_short_mid = sel_short.hit_test((100.0, 115.995), &vp);
    assert_ne!(
        hit_short_mid,
        SelectionHitTestResult::Handle(HandleId::LeftMiddle),
        "Edge pill LeftMiddle must be suppressed when height < 32px"
    );
    assert_eq!(
        hit_short_mid,
        SelectionHitTestResult::Body,
        "Decluttered edge contact must fall through to Body"
    );
    // Corners still active
    assert_eq!(
        sel_short.hit_test((200.0, 131.99), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomRight)
    );

    // Case 3: Exactly at threshold width = 32.0px, height = 32.0px
    let sel_threshold = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 132.0, 132.0)),
        ..Default::default()
    };
    // Midpoints: TopMiddle at (116.0, 100.0). Distance to TopLeft is 16px (> 15px).
    // Pills MUST be active!
    assert_eq!(
        sel_threshold.hit_test((116.0, 100.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle),
        "Edge pill must be active when dimensions >= 32px"
    );
    assert_eq!(
        sel_threshold.hit_test((116.0, 132.0), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomMiddle)
    );
    assert_eq!(
        sel_threshold.hit_test((100.0, 116.0), &vp),
        SelectionHitTestResult::Handle(HandleId::LeftMiddle)
    );
    assert_eq!(
        sel_threshold.hit_test((132.0, 116.0), &vp),
        SelectionHitTestResult::Handle(HandleId::RightMiddle)
    );

    // Case 4: Dynamic zoom transition
    // A document box of 50.0 x 50.0 pt
    let sel_zoom = SelectionState {
        bbox: Some(Rect::new(100.0, 100.0, 150.0, 150.0)),
        ..Default::default()
    };
    // At zoom 1.0 -> screen size 50x50 px >= 32px -> Pills ACTIVE
    let vp_10 = Viewport::new(0.0, 0.0, 1.0);
    assert_eq!(
        sel_zoom.hit_test((125.0, 100.0), &vp_10),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );

    // Zoom out to 0.5 -> screen size 25x25 px < 32px -> Pills SUPPRESSED
    let vp_05 = Viewport::new(0.0, 0.0, 0.5);
    let hit_zoom_out = sel_zoom.hit_test((125.0, 100.0), &vp_05);
    assert_ne!(
        hit_zoom_out,
        SelectionHitTestResult::Handle(HandleId::TopMiddle),
        "Zooming out to 25px screen size must suppress edge pills"
    );

    // Zoom in to 2.0 -> screen size 100x100 px >= 32px -> Pills ACTIVE
    let vp_20 = Viewport::new(0.0, 0.0, 2.0);
    assert_eq!(
        sel_zoom.hit_test((125.0, 100.0), &vp_20),
        SelectionHitTestResult::Handle(HandleId::TopMiddle),
        "Zooming in to 100px screen size must reactivate edge pills"
    );
}

// ============================================================================
// 4. ANTI-SINGULARITY AND DEGENERATE SHAPES
// ============================================================================

#[test]
fn test_adv_anti_singularity_and_degenerate_boxes() {
    // 1. Degenerate width = 0.0 (vertical line bbox)
    let v_line_bbox = Rect::new(150.0, 100.0, 150.0, 300.0);
    for handle in HandleId::ALL {
        for &lock in &[true, false] {
            let (anchor, (sx, sy)) =
                compute_scale_factors(handle, &v_line_bbox, (200.0, 400.0), lock);
            assert!(
                sx.is_finite(),
                "sx must be finite for {:?} (lock={})",
                handle,
                lock
            );
            assert!(
                sy.is_finite(),
                "sy must be finite for {:?} (lock={})",
                handle,
                lock
            );
            assert!(sx >= 0.05, "sx must be >= 0.05 for {:?}", handle);
            assert!(sy >= 0.05, "sy must be >= 0.05 for {:?}", handle);
            assert!(anchor.0.is_finite() && anchor.1.is_finite());
        }
    }

    // 2. Degenerate height = 0.0 (horizontal line bbox)
    let h_line_bbox = Rect::new(100.0, 250.0, 400.0, 250.0);
    for handle in HandleId::ALL {
        for &lock in &[true, false] {
            let (anchor, (sx, sy)) =
                compute_scale_factors(handle, &h_line_bbox, (500.0, 300.0), lock);
            assert!(sx.is_finite());
            assert!(sy.is_finite());
            assert!(sx >= 0.05);
            assert!(sy >= 0.05);
            assert!(anchor.0.is_finite() && anchor.1.is_finite());
        }
    }

    // 3. Degenerate point bbox (width = 0.0, height = 0.0)
    let point_bbox = Rect::new(200.0, 200.0, 200.0, 200.0);
    for handle in HandleId::ALL {
        for &lock in &[true, false] {
            let (_anchor, (sx, sy)) =
                compute_scale_factors(handle, &point_bbox, (250.0, 250.0), lock);
            assert!(sx.is_finite());
            assert!(sy.is_finite());
            assert!(sx >= 0.05);
            assert!(sy >= 0.05);
        }
    }

    // 4. Dragging handle past opposite anchor (inversion / negative scale clamping)
    let bbox = Rect::new(100.0, 100.0, 300.0, 300.0); // 200x200 pt, center (200, 200)

    // Corner TopLeft dragged past anchor BottomRight (300, 300) to (1000, 1000)
    let (_anchor, (sx_tl_inv, sy_tl_inv)) =
        compute_scale_factors(HandleId::TopLeft, &bbox, (1000.0, 1000.0), true);
    assert!(
        sx_tl_inv >= 0.05,
        "TopLeft inverted must clamp to >= 0.05: got {}",
        sx_tl_inv
    );
    assert!(
        sy_tl_inv >= 0.05,
        "TopLeft inverted must clamp to >= 0.05: got {}",
        sy_tl_inv
    );

    // Corner BottomRight dragged past anchor TopLeft (100, 100) to (-500, -500)
    let (_anchor, (sx_br_inv, sy_br_inv)) =
        compute_scale_factors(HandleId::BottomRight, &bbox, (-500.0, -500.0), false);
    assert!(
        sx_br_inv >= 0.05,
        "BottomRight inverted must clamp to >= 0.05: got {}",
        sx_br_inv
    );
    assert!(
        sy_br_inv >= 0.05,
        "BottomRight inverted must clamp to >= 0.05: got {}",
        sy_br_inv
    );

    // Corner TopRight dragged past anchor BottomLeft (100, 300) to (-200, 800)
    let (_anchor, (sx_tr_inv, sy_tr_inv)) =
        compute_scale_factors(HandleId::TopRight, &bbox, (-200.0, 800.0), false);
    assert!(sx_tr_inv >= 0.05);
    assert!(sy_tr_inv >= 0.05);

    // Corner BottomLeft dragged past anchor TopRight (300, 100) to (800, -200)
    let (_anchor, (sx_bl_inv, sy_bl_inv)) =
        compute_scale_factors(HandleId::BottomLeft, &bbox, (800.0, -200.0), false);
    assert!(sx_bl_inv >= 0.05);
    assert!(sy_bl_inv >= 0.05);

    // Edge LeftMiddle dragged past anchor RightMiddle (300, 200) to (500, 200)
    let (_anchor, (sx_lm_inv, sy_lm_inv)) =
        compute_scale_factors(HandleId::LeftMiddle, &bbox, (500.0, 200.0), false);
    assert!(sx_lm_inv >= 0.05);
    assert_eq!(sy_lm_inv, 1.0);

    // Edge TopMiddle dragged past anchor BottomMiddle (200, 300) to (200, 600)
    let (_anchor, (sx_tm_inv, sy_tm_inv)) =
        compute_scale_factors(HandleId::TopMiddle, &bbox, (200.0, 600.0), false);
    assert_eq!(sx_tm_inv, 1.0);
    assert!(sy_tm_inv >= 0.05);

    // 5. Minimum dimension clamp (5.0 pt):
    // Box with 10.0 pt width: scale 0.1x would produce 1.0 pt, clamped to 5.0 pt (scale = 0.5)
    let small_box = Rect::new(50.0, 50.0, 60.0, 60.0); // 10x10 pt
    let (_anchor, (sx_dim, sy_dim)) =
        compute_scale_factors(HandleId::BottomRight, &small_box, (51.0, 51.0), true);
    assert!(
        10.0 * sx_dim >= 5.0 - 1e-6,
        "Scaled width must remain >= 5.0 pt: got {}",
        10.0 * sx_dim
    );
    assert!(
        10.0 * sy_dim >= 5.0 - 1e-6,
        "Scaled height must remain >= 5.0 pt: got {}",
        10.0 * sy_dim
    );

    // Box with 1.0 pt width: clamped to 5.0 pt (scale = 5.0)
    let tiny_box = Rect::new(50.0, 50.0, 51.0, 51.0); // 1x1 pt
    let (_anchor, (sx_tiny, sy_tiny)) =
        compute_scale_factors(HandleId::BottomRight, &tiny_box, (50.5, 50.5), true);
    assert!(
        1.0 * sx_tiny >= 5.0 - 1e-6,
        "Scaled tiny width must remain >= 5.0 pt: got {}",
        1.0 * sx_tiny
    );
    assert!(
        1.0 * sy_tiny >= 5.0 - 1e-6,
        "Scaled tiny height must remain >= 5.0 pt: got {}",
        1.0 * sy_tiny
    );
}

// ============================================================================
// 5. ASPECT RATIO LOCKING PRECISION UNDER DIAGONAL PROJECTION
// ============================================================================

#[test]
fn test_adv_aspect_ratio_locking_precision_and_diagonal_projection() {
    // Non-square bounding box: 300 pt width, 150 pt height (aspect ratio = 2.0)
    let bbox = Rect::new(100.0, 100.0, 400.0, 250.0);

    // --- A. Bit-for-bit Equality across all 4 Corners ---
    let corners = [
        HandleId::TopLeft,
        HandleId::TopRight,
        HandleId::BottomRight,
        HandleId::BottomLeft,
    ];

    let arbitrary_cursor_positions = [
        (450.0, 300.0),
        (600.0, 150.0),
        (-200.0, 500.0),
        (350.0, -100.0),
        (100.0, 100.0),
        (0.0, 0.0),
        (1234.56, 789.10),
    ];

    for &corner in &corners {
        for &cursor in &arbitrary_cursor_positions {
            let (_anchor, (sx, sy)) = compute_scale_factors(corner, &bbox, cursor, true);
            assert_eq!(
                sx.to_bits(),
                sy.to_bits(),
                "Aspect ratio locked must guarantee sx == sy bit-for-bit on {:?} at cursor {:?}",
                corner,
                cursor
            );
        }
    }

    // --- B. Theoretical Diagonal Projection Verification for BottomRight ---
    // Anchor O = (100.0, 100.0), Handle P0 = (400.0, 250.0)
    // v0 = (300.0, 150.0). |v0|^2 = 300^2 + 150^2 = 90000 + 22500 = 112500
    // Test vector v = (450.0, 30.0) -> Cursor = (100 + 450, 100 + 30) = (550.0, 130.0)
    // dot = 450 * 300 + 30 * 150 = 135000 + 4500 = 139500
    // expected uniform scale = 139500 / 112500 = 1.24
    let (_anchor, (sx, sy)) =
        compute_scale_factors(HandleId::BottomRight, &bbox, (550.0, 130.0), true);
    assert_eq!(sx, sy);
    assert!(
        (sx - 1.24).abs() < 1e-12,
        "Expected diagonal projection scale 1.24: got {}",
        sx
    );

    // --- C. Orthogonal Displacement (Perpendicular to Diagonal) ---
    // v_perp = (-150.0, 300.0) since (-150)*300 + 300*150 = -45000 + 45000 = 0
    // Cursor = (100 - 150, 100 + 300) = (-50.0, 400.0)
    // dot = 0 -> scale <= 0 -> clamped to MIN_SCALE (0.05) or MIN_DIMENSION (5.0 / 150 = 0.033 -> 0.05)
    let (_anchor, (sx_perp, sy_perp)) =
        compute_scale_factors(HandleId::BottomRight, &bbox, (-50.0, 400.0), true);
    assert_eq!(sx_perp, sy_perp);
    assert!(
        (sx_perp - 0.05).abs() < 1e-12,
        "Perpendicular cursor must clamp to MIN_SCALE 0.05: got {}",
        sx_perp
    );

    // --- D. Decoupled Free Scaling (lock_aspect = false) ---
    // For same cursor (550.0, 130.0):
    // vx = 450.0, v0x = 300.0 -> sx = 450 / 300 = 1.5
    // vy = 30.0, v0y = 150.0 -> sy = 30 / 150 = 0.2
    let (_anchor, (sx_free, sy_free)) =
        compute_scale_factors(HandleId::BottomRight, &bbox, (550.0, 130.0), false);
    assert_ne!(sx_free, sy_free);
    assert!(
        (sx_free - 1.5).abs() < 1e-12,
        "Expected free scale_x 1.5: got {}",
        sx_free
    );
    assert!(
        (sy_free - 0.2).abs() < 1e-12,
        "Expected free scale_y 0.2: got {}",
        sy_free
    );

    // --- E. Preservation of Final Aspect Ratio across Scale ---
    let new_w = 300.0 * sx;
    let new_h = 150.0 * sy;
    let aspect_ratio = new_w / new_h;
    assert!(
        (aspect_ratio - 2.0).abs() < 1e-12,
        "Aspect ratio must remain exactly 2.0: got {}",
        aspect_ratio
    );
}

// ============================================================================
// 6. CANVAS SELECTION LIFECYCLE AND TRANSFORM REVERSIBILITY
// ============================================================================

#[test]
fn test_adv_canvas_selection_lifecycle_and_transform_reversibility() {
    let mut st = CanvasState::new();

    // Create a page and a sample stroke in document
    let style = StrokeStyle {
        color: (0.2, 0.4, 0.6, 1.0),
        base_width: 2.0,
        ..Default::default()
    };
    let mut stroke = Stroke::new(42, style, PipelineMode::CatmullRomSpline);
    stroke.points.push(StrokePoint {
        x: 100.0,
        y: 100.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1000,
    });
    stroke.points.push(StrokePoint {
        x: 300.0,
        y: 200.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 2000,
    });

    let item = Item {
        id: stroke.id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke.clone()),
    };
    st.document.pages[0].items.push(item.clone());
    st.document.pages[0].spatial_index.insert(&item);

    let bbox = Rect::new(100.0, 100.0, 300.0, 200.0);
    st.selection.page_index = Some(0);
    st.selection.selected_ids = vec![stroke.id];
    st.selection.bbox = Some(bbox);

    // 1. Contact Down on BottomRight corner handle at (300.0, 200.0)
    NoteCanvas::handle_selection_down(&mut st, (300.0, 200.0), false);
    assert!(st.selection.is_dragging);
    assert_eq!(
        st.selection.drag_kind,
        SelectionDragKind::ScaleHandle(HandleId::BottomRight)
    );
    assert_eq!(st.selection.scale_anchor, (100.0, 100.0));
    assert!(st.selection.lock_aspect_ratio);

    // 2. Motion Tick scaling to 2.0x: cursor moves to (500.0, 300.0)
    let redraw = NoteCanvas::handle_selection_motion(&mut st, (500.0, 300.0), false);
    assert!(redraw);
    assert_eq!(st.selection.scale_factors.0, 2.0);
    assert_eq!(st.selection.scale_factors.1, 2.0);

    // 3. Up: commit transform
    NoteCanvas::handle_selection_up(&mut st);
    assert!(!st.selection.is_dragging);
    assert_eq!(st.selection.drag_kind, SelectionDragKind::None);

    // Verify document stroke scaled
    let ItemKind::Stroke(ref scaled_stroke) = st.document.pages[0].items[0].kind else {
        panic!("Expected stroke");
    };
    assert_eq!(scaled_stroke.points[0].x, 100.0); // 100 + (100-100)*2 = 100
    assert_eq!(scaled_stroke.points[0].y, 100.0);
    assert_eq!(scaled_stroke.points[1].x, 500.0); // 100 + (300-100)*2 = 500
    assert_eq!(scaled_stroke.points[1].y, 300.0); // 100 + (200-100)*2 = 300

    // Verify selection bbox updated to new geometry
    let new_bbox = st.selection.bbox.unwrap();
    assert_eq!(new_bbox.min_x, 100.0);
    assert_eq!(new_bbox.min_y, 100.0);
    assert_eq!(new_bbox.max_x, 500.0);
    assert_eq!(new_bbox.max_y, 300.0);

    // 4. Undo: restores original geometry
    st.history.undo(&mut st.document);
    let ItemKind::Stroke(ref undone_stroke) = st.document.pages[0].items[0].kind else {
        panic!("Expected stroke");
    };
    assert!((undone_stroke.points[1].x - 300.0).abs() < 1e-4);
    assert!((undone_stroke.points[1].y - 200.0).abs() < 1e-4);

    // 5. Redo: re-applies scaled geometry
    st.history.redo(&mut st.document);
    let ItemKind::Stroke(ref redone_stroke) = st.document.pages[0].items[0].kind else {
        panic!("Expected stroke");
    };
    assert!((redone_stroke.points[1].x - 500.0).abs() < 1e-4);
    assert!((redone_stroke.points[1].y - 300.0).abs() < 1e-4);
}

// ============================================================================
// 7. LAVAGNA INFINITA NEGATIVE COORDINATES SCALING & HIT TESTING
// ============================================================================

#[test]
fn test_adv_lavagna_infinita_negative_coordinates_scaling_and_hit_test() {
    let neg_bbox = Rect::new(-400.0, -600.0, -100.0, -200.0); // 300x400 pt in negative space
    let sel = SelectionState {
        bbox: Some(neg_bbox),
        ..Default::default()
    };
    let vp = Viewport::new(500.0, 700.0, 1.0); // Offset shifts into positive screen space

    // TopLeft corner is at (-400.0, -600.0) -> maps to screen (100.0, 100.0)
    assert_eq!(
        sel.hit_test((-400.0, -600.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopLeft)
    );
    // BottomRight corner is at (-100.0, -200.0) -> maps to screen (400.0, 500.0)
    assert_eq!(
        sel.hit_test((-100.0, -200.0), &vp),
        SelectionHitTestResult::Handle(HandleId::BottomRight)
    );
    // TopMiddle is at (-250.0, -600.0) -> maps to screen (250.0, 100.0)
    assert_eq!(
        sel.hit_test((-250.0, -600.0), &vp),
        SelectionHitTestResult::Handle(HandleId::TopMiddle)
    );

    // Diagonal scaling on negative coordinates:
    // Drag TopLeft from (-400, -600) to (-550, -800) (1.5x expansion from anchor BottomRight (-100, -200))
    let (anchor, (sx, sy)) =
        compute_scale_factors(HandleId::TopLeft, &neg_bbox, (-550.0, -800.0), true);
    assert_eq!(anchor, (-100.0, -200.0));
    assert_eq!(sx, sy);
    assert!((sx - 1.5).abs() < 1e-12);
}

// ============================================================================
// 8. NON-FINITE INPUTS RESILIENCE (NaN, +Inf, -Inf)
// ============================================================================

#[test]
fn test_adv_non_finite_inputs_resilience() {
    let bbox = Rect::new(100.0, 100.0, 300.0, 300.0);
    let sel = SelectionState {
        bbox: Some(bbox),
        ..Default::default()
    };
    let vp = Viewport::new(0.0, 0.0, 1.0);

    // Hit test with NaN coordinates must not panic, should return None
    let hit_nan = sel.hit_test((f64::NAN, f64::NAN), &vp);
    assert_eq!(hit_nan, SelectionHitTestResult::None);

    let hit_inf = sel.hit_test((f64::INFINITY, f64::NEG_INFINITY), &vp);
    assert_eq!(hit_inf, SelectionHitTestResult::None);

    // Scaling with NaN / Inf cursor points must not panic and must sanitize to finite scale factors
    let non_finite_points = [
        (f64::NAN, 200.0),
        (200.0, f64::NAN),
        (f64::NAN, f64::NAN),
        (f64::INFINITY, 200.0),
        (200.0, f64::NEG_INFINITY),
        (f64::INFINITY, f64::INFINITY),
    ];

    for &cursor in &non_finite_points {
        for handle in HandleId::ALL {
            for &lock in &[true, false] {
                let (anchor, (sx, sy)) = compute_scale_factors(handle, &bbox, cursor, lock);
                assert!(
                    sx.is_finite(),
                    "sx must be finite for cursor {:?} on {:?}",
                    cursor,
                    handle
                );
                assert!(
                    sy.is_finite(),
                    "sy must be finite for cursor {:?} on {:?}",
                    cursor,
                    handle
                );
                assert!(sx >= 0.05);
                assert!(sy >= 0.05);
                assert!(anchor.0.is_finite() && anchor.1.is_finite());
            }
        }
    }
}

// ============================================================================
// 9. CONSECUTIVE TRANSFORMS UNDO / REDO CHAIN
// ============================================================================

#[test]
fn test_adv_consecutive_transforms_undo_redo_chain() {
    let mut st = CanvasState::new();

    let style = StrokeStyle {
        color: (0.1, 0.2, 0.3, 1.0),
        base_width: 2.0,
        ..Default::default()
    };
    let mut stroke = Stroke::new(99, style, PipelineMode::CatmullRomSpline);
    stroke.points.push(StrokePoint {
        x: 100.0,
        y: 100.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1000,
    });
    stroke.points.push(StrokePoint {
        x: 200.0,
        y: 200.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 2000,
    });

    let item = Item {
        id: stroke.id,
        z_index: 0,
        kind: ItemKind::Stroke(stroke.clone()),
    };
    st.document.pages[0].items.push(item.clone());
    st.document.pages[0].spatial_index.insert(&item);

    st.selection.page_index = Some(0);
    st.selection.selected_ids = vec![stroke.id];
    st.selection.bbox = Some(Rect::new(100.0, 100.0, 200.0, 200.0));

    // Gesture 1: Scale 2.0x from BottomRight (anchor 100, 100) -> moves to (300, 300)
    NoteCanvas::handle_selection_down(&mut st, (200.0, 200.0), false);
    NoteCanvas::handle_selection_motion(&mut st, (300.0, 300.0), false);
    NoteCanvas::handle_selection_up(&mut st);

    let bbox_1 = st.selection.bbox.unwrap();
    assert_eq!(bbox_1.max_x, 300.0);
    assert_eq!(bbox_1.max_y, 300.0);

    // Gesture 2: Scale 0.5x from BottomRight (anchor 100, 100) -> moves back to (200, 200)
    NoteCanvas::handle_selection_down(&mut st, (300.0, 300.0), false);
    NoteCanvas::handle_selection_motion(&mut st, (200.0, 200.0), false);
    NoteCanvas::handle_selection_up(&mut st);

    let bbox_2 = st.selection.bbox.unwrap();
    assert_eq!(bbox_2.max_x, 200.0);
    assert_eq!(bbox_2.max_y, 200.0);

    // Undo Gesture 2
    st.history.undo(&mut st.document);
    let ItemKind::Stroke(ref s_undo1) = st.document.pages[0].items[0].kind else {
        panic!()
    };
    assert!((s_undo1.points[1].x - 300.0).abs() < 1e-4);

    // Undo Gesture 1
    st.history.undo(&mut st.document);
    let ItemKind::Stroke(ref s_undo2) = st.document.pages[0].items[0].kind else {
        panic!()
    };
    assert!((s_undo2.points[1].x - 200.0).abs() < 1e-4);

    // Redo Gesture 1
    st.history.redo(&mut st.document);
    let ItemKind::Stroke(ref s_redo1) = st.document.pages[0].items[0].kind else {
        panic!()
    };
    assert!((s_redo1.points[1].x - 300.0).abs() < 1e-4);

    // Redo Gesture 2
    st.history.redo(&mut st.document);
    let ItemKind::Stroke(ref s_redo2) = st.document.pages[0].items[0].kind else {
        panic!()
    };
    assert!((s_redo2.points[1].x - 200.0).abs() < 1e-4);
}
