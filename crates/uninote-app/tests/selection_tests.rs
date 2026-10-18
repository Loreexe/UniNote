//! Selection Engine Opaque-Box Test Suite (Requirement R4)
//!
//! Scope:
//! - Freehand Lasso & Diagonal Rectangle selection
//! - Ray-Casting (Even-Odd) point-in-polygon containment
//! - 80% point containment threshold rule
//! - Interactive drag translation (delta_x, delta_y)
//! - Atomic DocCommand::MoveItems undo/redo reversibility
//! - Batch deletion of selected items

#![allow(irrefutable_let_patterns)]

use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

use uninote_core::model::{Document, Item, ItemKind, Page, PageBackground};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// REFERENCE ORACLES & MATHEMATICAL MODELS FOR SELECTION
// ============================================================================

/// 2D Point representation for selection geometry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

impl Point2D {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

/// Authoritative Ray-Casting algorithm (Even-Odd rule) for testing point-in-polygon.
/// A ray is cast horizontally from `point` towards positive infinity (+X).
/// Each intersection with a polygon edge toggles the inside/outside state.
pub fn ray_cast_point_in_polygon(point: Point2D, polygon: &[Point2D]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let n = polygon.len();
    let mut j = n - 1;

    for i in 0..n {
        let pi = polygon[i];
        let pj = polygon[j];

        // Check if point Y is between vertex Ys (one above/equal, one below)
        let y_between = (pi.y > point.y) != (pj.y > point.y);
        if y_between {
            // Compute X intersection of the edge with the horizontal ray at point.y
            let x_intersect = (pj.x - pi.x) * (point.y - pi.y) / (pj.y - pi.y) + pi.x;
            if point.x < x_intersect {
                inside = !inside;
            }
        }
        j = i;
    }

    inside
}

/// Evaluates whether a stroke is selected by a freehand lasso polygon.
/// Authoritative rule: at least 80% (>= 0.80) of the stroke's sampled points must lie inside the polygon.
pub fn is_stroke_selected_by_lasso(stroke: &Stroke, polygon: &[Point2D]) -> bool {
    if stroke.points.is_empty() || polygon.len() < 3 {
        return false;
    }

    let mut inside_count = 0;
    for pt in &stroke.points {
        if ray_cast_point_in_polygon(Point2D::new(pt.x, pt.y), polygon) {
            inside_count += 1;
        }
    }

    let fraction = inside_count as f64 / stroke.points.len() as f64;
    fraction >= 0.80
}

/// Evaluates whether a stroke is selected by a selection rectangle.
/// Authoritative rule: the stroke's bounding box must be completely contained in the normalized rectangle.
pub fn is_stroke_selected_by_rect(stroke: &Stroke, rect: &Rect) -> bool {
    let bbox = match stroke.bounding_box() {
        Some(b) => b,
        None => return false,
    };

    let norm_min_x = rect.min_x.min(rect.max_x);
    let norm_max_x = rect.min_x.max(rect.max_x);
    let norm_min_y = rect.min_y.min(rect.max_y);
    let norm_max_y = rect.min_y.max(rect.max_y);

    bbox.min_x >= norm_min_x
        && bbox.max_x <= norm_max_x
        && bbox.min_y >= norm_min_y
        && bbox.max_y <= norm_max_y
}

/// Translates a stroke by (delta_x, delta_y).
pub fn translate_stroke(stroke: &mut Stroke, delta_x: f64, delta_y: f64) {
    for pt in &mut stroke.points {
        pt.x += delta_x;
        pt.y += delta_y;
    }
}

/// Reversible MoveItems command representation for testing atomicity and undo/redo fidelity.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MoveItemsCommand {
    pub page_id: Uuid,
    pub item_ids: Vec<u64>,
    pub delta_x: f64,
    pub delta_y: f64,
}

impl MoveItemsCommand {
    pub fn new(page_id: Uuid, item_ids: Vec<u64>, delta_x: f64, delta_y: f64) -> Self {
        Self {
            page_id,
            item_ids,
            delta_x,
            delta_y,
        }
    }

    pub fn execute(&self, doc: &mut Document) {
        if let Some(page) = doc.get_page_mut(self.page_id) {
            for item in &mut page.items {
                if self.item_ids.contains(&item.id) {
                    if let ItemKind::Stroke(ref mut s) = item.kind {
                        translate_stroke(s, self.delta_x, self.delta_y);
                    }
                }
            }
        }
    }

    pub fn undo(&self, doc: &mut Document) {
        if let Some(page) = doc.get_page_mut(self.page_id) {
            for item in &mut page.items {
                if self.item_ids.contains(&item.id) {
                    if let ItemKind::Stroke(ref mut s) = item.kind {
                        translate_stroke(s, -self.delta_x, -self.delta_y);
                    }
                }
            }
        }
    }
}

/// Helper to create a test stroke with specified points.
fn make_stroke(id: u64, coords: &[(f64, f64)]) -> Stroke {
    let points = coords
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 1_000_000 * (i as u64),
        })
        .collect();

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

// ============================================================================
// TIER 1: FEATURE COVERAGE (>=5 TESTS)
// ============================================================================

#[test]
fn test_t1_r4_ray_casting_convex_polygon_containment() {
    // Square polygon from (10, 10) to (100, 100)
    let poly = vec![
        Point2D::new(10.0, 10.0),
        Point2D::new(100.0, 10.0),
        Point2D::new(100.0, 100.0),
        Point2D::new(10.0, 100.0),
    ];

    // Center point must be inside
    assert!(ray_cast_point_in_polygon(Point2D::new(50.0, 50.0), &poly));
    // Outside points
    assert!(!ray_cast_point_in_polygon(Point2D::new(5.0, 50.0), &poly));
    assert!(!ray_cast_point_in_polygon(Point2D::new(105.0, 50.0), &poly));
    assert!(!ray_cast_point_in_polygon(Point2D::new(50.0, 5.0), &poly));
    assert!(!ray_cast_point_in_polygon(Point2D::new(50.0, 105.0), &poly));
}

#[test]
fn test_t1_r4_lasso_eighty_percent_threshold_acceptance() {
    let poly = vec![
        Point2D::new(0.0, 0.0),
        Point2D::new(100.0, 0.0),
        Point2D::new(100.0, 100.0),
        Point2D::new(0.0, 100.0),
    ];

    // Stroke with 10 points: 8 points inside (0..80 in x) and 2 points outside (110, 120 in x)
    // 8 / 10 = 80% -> must be selected!
    let mut coords = Vec::new();
    for i in 0..8 {
        coords.push((10.0 + (i as f64) * 10.0, 50.0));
    }
    coords.push((110.0, 50.0));
    coords.push((120.0, 50.0));

    let stroke = make_stroke(1, &coords);
    assert_eq!(stroke.points.len(), 10);
    assert!(
        is_stroke_selected_by_lasso(&stroke, &poly),
        "Stroke with exactly 80% points inside must be selected"
    );
}

#[test]
fn test_t1_r4_rectangle_selection_complete_containment() {
    let rect = Rect::new(50.0, 50.0, 200.0, 200.0);

    // Stroke inside rect
    let stroke_inside = make_stroke(1, &[(60.0, 60.0), (100.0, 100.0), (180.0, 180.0)]);
    assert!(is_stroke_selected_by_rect(&stroke_inside, &rect));

    // Stroke crossing rect border
    let stroke_crossing = make_stroke(2, &[(40.0, 60.0), (100.0, 100.0), (180.0, 180.0)]);
    assert!(!is_stroke_selected_by_rect(&stroke_crossing, &rect));
}

#[test]
fn test_t1_r4_stroke_translation_point_preservation() {
    let mut stroke = make_stroke(1, &[(10.0, 20.0), (30.0, 40.0), (50.0, 60.0)]);
    let (dx, dy) = (15.5, -7.25);
    translate_stroke(&mut stroke, dx, dy);

    assert_eq!(stroke.points[0].x, 25.5);
    assert_eq!(stroke.points[0].y, 12.75);
    assert_eq!(stroke.points[1].x, 45.5);
    assert_eq!(stroke.points[1].y, 32.75);
    assert_eq!(stroke.points[2].x, 65.5);
    assert_eq!(stroke.points[2].y, 52.75);
}

#[test]
fn test_t1_r4_atomic_move_command_undo_redo_fidelity() {
    let mut doc = Document::new_empty("Selection Move Test");
    let page_id = doc.pages[0].id;

    let stroke1 = make_stroke(101, &[(10.0, 10.0), (20.0, 20.0)]);
    let stroke2 = make_stroke(102, &[(100.0, 100.0), (150.0, 150.0)]);

    doc.pages[0].items.push(Item {
        id: 101,
        z_index: 1,
        kind: ItemKind::Stroke(stroke1),
    });
    doc.pages[0].items.push(Item {
        id: 102,
        z_index: 2,
        kind: ItemKind::Stroke(stroke2),
    });

    // Execute Move of item 101 only by (+50, +30)
    let cmd = MoveItemsCommand::new(page_id, vec![101], 50.0, 30.0);
    cmd.execute(&mut doc);

    if let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 60.0);
        assert_eq!(s.points[0].y, 40.0);
    }
    // Item 102 must remain untouched
    if let ItemKind::Stroke(ref s) = doc.pages[0].items[1].kind {
        assert_eq!(s.points[0].x, 100.0);
        assert_eq!(s.points[0].y, 100.0);
    }

    // Undo Move
    cmd.undo(&mut doc);
    if let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 10.0);
        assert_eq!(s.points[0].y, 10.0);
    }
}

#[test]
fn test_t1_r4_batch_delete_selected_items() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    for i in 1..=5 {
        page.items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(make_stroke(i, &[(i as f64 * 10.0, 50.0)])),
        });
    }
    assert_eq!(page.items.len(), 5);

    // Batch delete items 2 and 4
    let selected_ids = [2, 4];
    page.items.retain(|item| !selected_ids.contains(&item.id));

    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].id, 1);
    assert_eq!(page.items[1].id, 3);
    assert_eq!(page.items[2].id, 5);
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 TESTS)
// ============================================================================

#[test]
fn test_t2_r4_lasso_below_eighty_percent_rejected() {
    let poly = vec![
        Point2D::new(0.0, 0.0),
        Point2D::new(100.0, 0.0),
        Point2D::new(100.0, 100.0),
        Point2D::new(0.0, 100.0),
    ];

    // Stroke with 10 points: 7 inside (0..70 in x), 3 outside (110, 120, 130)
    // 7 / 10 = 70% < 80% -> must NOT be selected!
    let mut coords = Vec::new();
    for i in 0..7 {
        coords.push((10.0 + (i as f64) * 10.0, 50.0));
    }
    coords.push((110.0, 50.0));
    coords.push((120.0, 50.0));
    coords.push((130.0, 50.0));

    let stroke = make_stroke(1, &coords);
    assert!(
        !is_stroke_selected_by_lasso(&stroke, &poly),
        "Stroke with 70% points inside must not be selected"
    );
}

#[test]
fn test_t2_r4_degenerate_lasso_polygon_zero_or_two_vertices() {
    let empty_poly: Vec<Point2D> = Vec::new();
    let two_poly = vec![Point2D::new(0.0, 0.0), Point2D::new(10.0, 10.0)];
    let stroke = make_stroke(1, &[(5.0, 5.0)]);

    assert!(!is_stroke_selected_by_lasso(&stroke, &empty_poly));
    assert!(!is_stroke_selected_by_lasso(&stroke, &two_poly));
}

#[test]
fn test_t2_r4_rectangle_inverted_drag_normalization() {
    // User drags from (200, 200) to (50, 50) -> min > max
    let inverted_rect = Rect::new(200.0, 200.0, 50.0, 50.0);
    let stroke = make_stroke(1, &[(100.0, 100.0), (120.0, 120.0)]);

    assert!(
        is_stroke_selected_by_rect(&stroke, &inverted_rect),
        "Inverted rectangle drag coordinates must be normalized to correctly select stroke"
    );
}

#[test]
fn test_t2_r4_zero_delta_translation_is_identity() {
    let original = make_stroke(1, &[(12.34, 56.78), (90.12, 34.56)]);
    let mut stroke = original.clone();
    translate_stroke(&mut stroke, 0.0, 0.0);

    assert_eq!(stroke.points[0].x, original.points[0].x);
    assert_eq!(stroke.points[0].y, original.points[0].y);
    assert_eq!(stroke.points[1].x, original.points[1].x);
    assert_eq!(stroke.points[1].y, original.points[1].y);
}

#[test]
fn test_t2_r4_concave_star_polygon_ray_casting() {
    // 5-point star polygon / concave shape
    let concave_poly = vec![
        Point2D::new(50.0, 0.0),
        Point2D::new(60.0, 35.0),
        Point2D::new(95.0, 35.0),
        Point2D::new(65.0, 55.0),
        Point2D::new(80.0, 90.0),
        Point2D::new(50.0, 70.0),
        Point2D::new(20.0, 90.0),
        Point2D::new(35.0, 55.0),
        Point2D::new(5.0, 35.0),
        Point2D::new(40.0, 35.0),
    ];

    // Center of star (50, 45) is inside
    assert!(ray_cast_point_in_polygon(
        Point2D::new(50.0, 45.0),
        &concave_poly
    ));
    // Bay indentation between legs (50, 80) is outside
    assert!(!ray_cast_point_in_polygon(
        Point2D::new(50.0, 80.0),
        &concave_poly
    ));
}

#[test]
fn test_t2_r4_high_density_selection_performance_stress() {
    // Generate 1,000 strokes with 10 points each
    let poly = vec![
        Point2D::new(100.0, 100.0),
        Point2D::new(500.0, 100.0),
        Point2D::new(500.0, 500.0),
        Point2D::new(100.0, 500.0),
    ];

    let mut strokes = Vec::with_capacity(1000);
    for i in 0..1000 {
        let x = (i % 50) as f64 * 15.0;
        let y = (i / 50) as f64 * 25.0;
        strokes.push(make_stroke(i as u64, &[(x, y), (x + 5.0, y + 5.0)]));
    }

    let start = Instant::now();
    let mut selected_count = 0;
    for stroke in &strokes {
        if is_stroke_selected_by_lasso(stroke, &poly) {
            selected_count += 1;
        }
    }
    let elapsed = start.elapsed();

    assert!(
        selected_count > 0,
        "Some strokes must fall within polygon bounds"
    );
    assert!(
        elapsed.as_millis() < 50,
        "Selection of 1,000 strokes must execute in < 50ms (took {}ms)",
        elapsed.as_millis()
    );
}

// ============================================================================
// TIER 3: PAIRWISE & CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_t3_selection_and_undo_stack_sequential_operations() {
    let mut doc = Document::new_empty("Sequential Move Test");
    let page_id = doc.pages[0].id;

    let s1 = make_stroke(1, &[(0.0, 0.0), (10.0, 10.0)]);
    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(s1),
    });

    let cmd1 = MoveItemsCommand::new(page_id, vec![1], 100.0, 0.0);
    cmd1.execute(&mut doc);

    let cmd2 = MoveItemsCommand::new(page_id, vec![1], 0.0, 200.0);
    cmd2.execute(&mut doc);

    if let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 100.0);
        assert_eq!(s.points[0].y, 200.0);
    }

    // Undo step 2
    cmd2.undo(&mut doc);
    if let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 100.0);
        assert_eq!(s.points[0].y, 0.0);
    }

    // Undo step 1
    cmd1.undo(&mut doc);
    if let ItemKind::Stroke(ref s) = doc.pages[0].items[0].kind {
        assert_eq!(s.points[0].x, 0.0);
        assert_eq!(s.points[0].y, 0.0);
    }
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_academic_formula_reorganization() {
    // Scenario: Student writes a multi-line physics derivation.
    // Line 1: F = m * a (y = 100)
    // Line 2: W = F * s (y = 150)
    // Student realizes an intermediate definition (a = dv/dt) is needed.
    // Student lasso-selects Line 2 and moves it down by 80 pt to make room.

    let mut doc = Document::new_empty("Physics Derivation");
    let page_id = doc.pages[0].id;

    let line1 = make_stroke(1, &[(50.0, 100.0), (150.0, 100.0)]);
    let line2 = make_stroke(2, &[(50.0, 150.0), (150.0, 150.0)]);

    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(line1),
    });
    doc.pages[0].items.push(Item {
        id: 2,
        z_index: 2,
        kind: ItemKind::Stroke(line2),
    });

    // Lasso around Line 2: box (40, 130) to (160, 170)
    let lasso = vec![
        Point2D::new(40.0, 130.0),
        Point2D::new(160.0, 130.0),
        Point2D::new(160.0, 170.0),
        Point2D::new(40.0, 170.0),
    ];

    let mut selected = Vec::new();
    for item in &doc.pages[0].items {
        if let ItemKind::Stroke(ref s) = item.kind {
            if is_stroke_selected_by_lasso(s, &lasso) {
                selected.push(item.id);
            }
        }
    }

    assert_eq!(selected, vec![2], "Only Line 2 must be selected by lasso");

    // Move Line 2 downwards by 80 pt
    let move_cmd = MoveItemsCommand::new(page_id, selected, 0.0, 80.0);
    move_cmd.execute(&mut doc);

    if let ItemKind::Stroke(ref s) = doc.pages[0].items[1].kind {
        assert_eq!(s.points[0].y, 230.0, "Line 2 moved down to y=230");
    }

    // Insert intermediate definition at y = 160
    let line_intermediate = make_stroke(3, &[(50.0, 160.0), (150.0, 160.0)]);
    doc.pages[0].items.push(Item {
        id: 3,
        z_index: 3,
        kind: ItemKind::Stroke(line_intermediate),
    });

    assert_eq!(doc.pages[0].items.len(), 3);
}

/// Verification of AppSelectionEngine with R-Tree spatial index pre-filtering (Rect).
#[test]
fn test_app_selection_engine_spatial_rect_acceleration() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    for i in 1..=100 {
        let x = (i as f64) * 5.0;
        let s = make_stroke(i, &[(x, 100.0), (x + 3.0, 100.0)]);
        page.items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(s),
        });
    }

    // Populate spatial index
    page.rebuild_spatial_index();
    assert_eq!(page.spatial_index.len(), 100);

    // Select items between x = 45.0 and x = 65.0
    let query_rect = Rect::new(45.0, 90.0, 65.0, 110.0);
    let (selected_ids, bbox) =
        uninote_app::selection::AppSelectionEngine::select_rect(&page, &query_rect);

    assert!(!selected_ids.is_empty());
    for id in &selected_ids {
        let x = (*id as f64) * 5.0;
        assert!((45.0..=65.0).contains(&x));
    }
    assert!(bbox.is_some());
}

/// Verification of AppSelectionEngine with R-Tree spatial index pre-filtering (Lasso).
#[test]
fn test_app_selection_engine_spatial_lasso_acceleration() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    for i in 1..=50 {
        let y = (i as f64) * 10.0;
        let s = make_stroke(i, &[(100.0, y), (120.0, y)]);
        page.items.push(Item {
            id: i,
            z_index: i,
            kind: ItemKind::Stroke(s),
        });
    }

    page.rebuild_spatial_index();

    // Lasso polygon surrounding strokes i = 10..=15 (y from 95 to 155)
    let poly = vec![(80.0, 95.0), (140.0, 95.0), (140.0, 155.0), (80.0, 155.0)];
    let (selected_ids, bbox) =
        uninote_app::selection::AppSelectionEngine::select_lasso(&page, &poly);

    assert_eq!(selected_ids.len(), 6);
    assert_eq!(selected_ids, vec![10, 11, 12, 13, 14, 15]);
    assert!(bbox.is_some());
}

/// Verification of AppSelectionEngine fallback when spatial index is empty.
#[test]
fn test_app_selection_engine_fallback_on_empty_spatial_index() {
    let mut page = Page::new_a4(0, PageBackground::Blank);
    let s = make_stroke(1, &[(50.0, 50.0), (60.0, 50.0)]);
    page.items.push(Item {
        id: 1,
        z_index: 1,
        kind: ItemKind::Stroke(s),
    });

    // Spatial index deliberately empty
    assert!(page.spatial_index.is_empty());

    let query_rect = Rect::new(40.0, 40.0, 70.0, 60.0);
    let (selected_ids, bbox) =
        uninote_app::selection::AppSelectionEngine::select_rect(&page, &query_rect);

    assert_eq!(selected_ids, vec![1]);
    assert!(bbox.is_some());
}
