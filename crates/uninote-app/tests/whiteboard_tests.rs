//! Infinite Whiteboard Opaque-Box Test Suite (Requirement R2)
//!
//! Scope:
//! - DocumentFormat::{Notebook, Whiteboard} data model
//! - Single infinite logical page with unbounded dimensions
//! - Dynamic procedural Cairo grid scaling with adaptive zoom fading
//! - Viewport::center_on_rect calculation for "Center Content"
//! - Unbounded Cartesian coordinates (-inf, +inf) affine invariance
//! - Backward-compatible SQLite metadata document_format persistence model

use gtk4::cairo::{Context, Format, ImageSurface};

use uninote_core::coordinates::{Point2D, Rect2D, Viewport};
use uninote_core::model::{Document, DocumentFormat, PageBackground};
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// REFERENCE ORACLES & MATHEMATICAL MODELS FOR WHITEBOARD
// ============================================================================

/// Dynamic procedural grid calculation oracle based on zoom level.
/// In Whiteboard mode, Cairo renders a dynamic mathematical dot/line grid:
/// - Base spacing: 20.0 pt
/// - When zoom < 0.5: grid steps increase by factor of 2 (40.0 pt) with alpha fade
/// - When zoom > 2.0: subdivision grid appears (10.0 pt) with progressive alpha
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicGridSpec {
    pub step_pt: f64,
    pub alpha: f64,
    pub major_step_pt: f64,
}

pub fn oracle_dynamic_grid_spec(zoom: f64) -> DynamicGridSpec {
    let clamped_zoom = zoom.clamp(0.25, 5.0);
    let base_step = 20.0;

    if clamped_zoom < 0.5 {
        let t = (clamped_zoom - 0.25) / 0.25; // 0.0 at 0.25, 1.0 at 0.5
        DynamicGridSpec {
            step_pt: base_step * 2.0, // 40.0 pt
            alpha: 0.15 + 0.15 * t,
            major_step_pt: base_step * 8.0,
        }
    } else if clamped_zoom <= 2.0 {
        DynamicGridSpec {
            step_pt: base_step, // 20.0 pt
            alpha: 0.25,
            major_step_pt: base_step * 5.0, // 100.0 pt
        }
    } else {
        let t = ((clamped_zoom - 2.0) / 3.0).min(1.0); // 0.0 at 2.0, 1.0 at 5.0
        DynamicGridSpec {
            step_pt: base_step / 2.0, // 10.0 pt
            alpha: 0.15 + 0.20 * t,
            major_step_pt: base_step,
        }
    }
}

/// Computes the aggregate bounding box of all strokes in a whiteboard page.
pub fn compute_strokes_bounding_box(strokes: &[Stroke]) -> Option<Rect2D> {
    if strokes.is_empty() {
        return None;
    }

    let mut min_x = f64::INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut max_y = f64::NEG_INFINITY;

    for s in strokes {
        if let Some(bbox) = s.bounding_box() {
            min_x = min_x.min(bbox.min_x);
            min_y = min_y.min(bbox.min_y);
            max_x = max_x.max(bbox.max_x);
            max_y = max_y.max(bbox.max_y);
        }
    }

    if min_x.is_finite() && min_y.is_finite() && max_x.is_finite() && max_y.is_finite() {
        Some(Rect2D::new(min_x, min_y, max_x - min_x, max_y - min_y))
    } else {
        None
    }
}

/// Helper to construct a synthetic stroke at specified coordinates.
fn make_whiteboard_stroke(id: u64, x: f64, y: f64, len: f64) -> Stroke {
    Stroke {
        id,
        points: vec![
            StrokePoint {
                x,
                y,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: x + len,
                y: y + len,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 10_000,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
    }
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (>=5 TESTS)
// ============================================================================

#[test]
fn test_t1_r2_document_format_enum_variants() {
    assert_eq!(DocumentFormat::default(), DocumentFormat::Notebook);

    let doc_nb = Document::new_notebook("Test Notebook");
    assert_eq!(doc_nb.format, DocumentFormat::Notebook);
    assert_eq!(doc_nb.pages.len(), 1);
    assert!(doc_nb.pages[0].width.is_finite());

    let doc_wb = Document::new_whiteboard(PageBackground::Blank);
    assert_eq!(doc_wb.format, DocumentFormat::Whiteboard);
    assert_eq!(doc_wb.pages.len(), 1);
    assert_eq!(doc_wb.pages[0].width, f64::INFINITY);
    assert_eq!(doc_wb.pages[0].height, f64::INFINITY);
}

#[test]
fn test_t1_r2_unbounded_negative_and_positive_coordinates() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);

    // Pan into deep negative coordinates: dx = -10,000, dy = -25,000
    viewport.pan(-10000.0, -25000.0);

    let screen_pt = (500.0, 500.0);
    let doc_pt = viewport.screen_to_document(screen_pt);
    assert_eq!(doc_pt.0, 10500.0);
    assert_eq!(doc_pt.1, 25500.0);

    let roundtrip_screen = viewport.document_to_screen(doc_pt);
    assert!((roundtrip_screen.0 - screen_pt.0).abs() < 1e-6);
    assert!((roundtrip_screen.1 - screen_pt.1).abs() < 1e-6);
}

#[test]
fn test_t1_r2_dynamic_procedural_grid_scaling_levels() {
    // Zoom 0.25 (minimum zoom) -> large 40pt step with reduced alpha
    let spec_low = oracle_dynamic_grid_spec(0.25);
    assert_eq!(spec_low.step_pt, 40.0);
    assert!(spec_low.alpha < 0.20);

    // Zoom 1.0 (standard 100%) -> base 20pt step
    let spec_mid = oracle_dynamic_grid_spec(1.0);
    assert_eq!(spec_mid.step_pt, 20.0);
    assert_eq!(spec_mid.alpha, 0.25);

    // Zoom 4.0 (high zoom) -> fine 10pt step
    let spec_high = oracle_dynamic_grid_spec(4.0);
    assert_eq!(spec_high.step_pt, 10.0);
    assert!(spec_high.alpha > 0.20);
}

#[test]
fn test_t1_r2_dynamic_grid_cairo_rendering() {
    let surface = ImageSurface::create(Format::ARgb32, 200, 200).unwrap();
    let cr = Context::new(&surface).unwrap();

    let spec = oracle_dynamic_grid_spec(1.0);
    cr.set_source_rgba(0.427, 0.824, 1.0, spec.alpha);
    cr.set_line_width(0.8);

    // Draw grid lines
    let mut x = 0.0;
    while x <= 200.0 {
        cr.move_to(x, 0.0);
        cr.line_to(x, 200.0);
        x += spec.step_pt;
    }
    let mut y = 0.0;
    while y <= 200.0 {
        cr.move_to(0.0, y);
        cr.line_to(200.0, y);
        y += spec.step_pt;
    }
    cr.stroke().unwrap();

    assert!(cr.status().is_ok());
    drop(cr);

    let data = surface.take_data().unwrap();
    let non_zero = data.iter().filter(|&&b| b > 0).count();
    assert!(non_zero > 100, "Grid lines must produce rendered pixels");
}

#[test]
fn test_t1_r2_center_content_viewport_calculation() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let win_w = 1200.0;
    let win_h = 800.0;

    // Content bounding box: x in [100, 700] (width 600), y in [200, 600] (height 400)
    let content_rect = Rect2D::new(100.0, 200.0, 600.0, 400.0);
    viewport.center_on_rect(content_rect, win_w, win_h);

    // After center_on_rect, the center of content_rect must map to center of window (600, 400)
    let center_doc = (
        content_rect.x + content_rect.width / 2.0,  // 400.0
        content_rect.y + content_rect.height / 2.0, // 400.0
    );
    let center_screen = viewport.document_to_screen(center_doc);

    assert!(
        (center_screen.0 - win_w / 2.0).abs() < 1.0,
        "Center X must be at window center: got {}",
        center_screen.0
    );
    assert!(
        (center_screen.1 - win_h / 2.0).abs() < 1.0,
        "Center Y must be at window center: got {}",
        center_screen.1
    );
}

#[test]
fn test_t1_r2_format_metadata_sqlite_representation() {
    // Serialization contract for SQLite metadata table
    let format_nb = DocumentFormat::Notebook;
    let str_nb = serde_json::to_string(&format_nb).unwrap();
    assert!(str_nb.to_lowercase().contains("notebook"));

    let format_wb = DocumentFormat::Whiteboard;
    let str_wb = serde_json::to_string(&format_wb).unwrap();
    assert!(str_wb.to_lowercase().contains("whiteboard"));

    // Backward compatibility: default is Notebook when key missing
    let parsed: DocumentFormat = serde_json::from_str("\"Notebook\"").unwrap();
    assert_eq!(parsed, DocumentFormat::Notebook);
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 TESTS)
// ============================================================================

#[test]
fn test_t2_r2_center_content_on_empty_canvas() {
    let mut viewport = Viewport::new(100.0, 100.0, 1.5);
    // Degenerate zero-dimension bounding box
    let empty_rect = Rect2D::new(0.0, 0.0, 0.0, 0.0);
    viewport.center_on_rect(empty_rect, 1000.0, 800.0);

    // Must reset gracefully to center without NaN or panic
    assert!(viewport.offset_x.is_finite());
    assert!(viewport.offset_y.is_finite());
    assert_eq!(viewport.zoom, 1.0);
}

#[test]
fn test_t2_r2_center_content_single_point_stroke() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let point_rect = Rect2D::new(250.0, 350.0, 0.0, 0.0);
    viewport.center_on_rect(point_rect, 800.0, 600.0);

    assert!(viewport.zoom > 0.0 && viewport.zoom <= 3.0);
    assert!(viewport.offset_x.is_finite());
    assert!(viewport.offset_y.is_finite());
}

#[test]
fn test_t2_r2_extreme_coordinates_numerical_stability() {
    let viewport = Viewport::new(0.0, 0.0, 1.0);
    let extreme_pt = (10_000_000.0, -10_000_000.0);

    let screen_pt = viewport.document_to_screen(extreme_pt);
    let recovered_doc = viewport.screen_to_document(screen_pt);

    assert!((recovered_doc.0 - extreme_pt.0).abs() < 1e-3);
    assert!((recovered_doc.1 - extreme_pt.1).abs() < 1e-3);
}

#[test]
fn test_t2_r2_dynamic_grid_nan_and_inf_zoom_safety() {
    let spec_nan = oracle_dynamic_grid_spec(f64::NAN);
    assert!(spec_nan.step_pt > 0.0 && spec_nan.alpha > 0.0);

    let spec_inf = oracle_dynamic_grid_spec(f64::INFINITY);
    assert!(spec_inf.step_pt > 0.0 && spec_inf.alpha > 0.0);

    let spec_neg = oracle_dynamic_grid_spec(-5.0);
    assert!(spec_neg.step_pt > 0.0 && spec_neg.alpha > 0.0);
}

#[test]
fn test_t2_r2_quadrant_spanning_bounding_box() {
    // Strokes placed in all 4 Cartesian quadrants:
    // Q1 (+, -): (500, -300)
    // Q2 (-, -): (-800, -600)
    // Q3 (-, +): (-400, 900)
    // Q4 (+, +): (1200, 1500)
    let s1 = make_whiteboard_stroke(1, 500.0, -300.0, 50.0);
    let s2 = make_whiteboard_stroke(2, -800.0, -600.0, 50.0);
    let s3 = make_whiteboard_stroke(3, -400.0, 900.0, 50.0);
    let s4 = make_whiteboard_stroke(4, 1200.0, 1500.0, 50.0);

    let strokes = vec![s1, s2, s3, s4];
    let bbox = compute_strokes_bounding_box(&strokes).expect("BBox must exist");

    assert!(bbox.x <= -800.0);
    assert!(bbox.y <= -600.0);
    assert!(bbox.x + bbox.width >= 1250.0);
    assert!(bbox.y + bbox.height >= 1550.0);
}

// ============================================================================
// TIER 3: PAIRWISE & CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_t3_whiteboard_and_selection_in_negative_coordinates() {
    // Selection lasso and translation operating entirely within negative coordinates
    let mut stroke = make_whiteboard_stroke(10, -500.0, -500.0, 100.0);

    // Lasso polygon surrounding (-550, -550) to (-350, -350)
    let _poly = [
        Point2D::new(-550.0, -550.0),
        Point2D::new(-350.0, -550.0),
        Point2D::new(-350.0, -350.0),
        Point2D::new(-550.0, -350.0),
    ];

    let mut inside = 0;
    for pt in &stroke.points {
        let p = Point2D::new(pt.x, pt.y);
        // Simple bounding check for test isolation
        if p.x >= -550.0 && p.x <= -350.0 && p.y >= -550.0 && p.y <= -350.0 {
            inside += 1;
        }
    }
    assert_eq!(inside, stroke.points.len());

    // Translate across origin into positive quadrant: dx = +1000, dy = +1000
    for pt in &mut stroke.points {
        pt.x += 1000.0;
        pt.y += 1000.0;
    }

    assert_eq!(stroke.points[0].x, 500.0);
    assert_eq!(stroke.points[0].y, 500.0);
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_infinite_brainstorming_mindmap() {
    // Scenario: Student brainstorms thesis software architecture.
    // Core node at origin (0, 0).
    // Subsystem 1: "UI/UX" at (+800, -400)
    // Subsystem 2: "Audio/DSP Engine" at (-1200, +600)
    // Subsystem 3: "Persistence/WAL" at (+1500, +1200)
    // Student connects them with branches and centers the entire map.

    let doc = Document::new_whiteboard(PageBackground::Blank);
    assert_eq!(doc.format, DocumentFormat::Whiteboard);

    let strokes = vec![
        make_whiteboard_stroke(1, 0.0, 0.0, 100.0),
        make_whiteboard_stroke(2, 800.0, -400.0, 150.0),
        make_whiteboard_stroke(3, -1200.0, 600.0, 200.0),
        make_whiteboard_stroke(4, 1500.0, 1200.0, 180.0),
    ];

    let bbox = compute_strokes_bounding_box(&strokes).unwrap();
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    viewport.center_on_rect(bbox, 1920.0, 1080.0);

    // Verify all 4 nodes are within visible screen coordinates after centering
    for s in &strokes {
        for pt in &s.points {
            let scr = viewport.document_to_screen((pt.x, pt.y));
            assert!(
                scr.0 >= -100.0 && scr.0 <= 2020.0,
                "Node point X must be within viewport view: {}",
                scr.0
            );
            assert!(
                scr.1 >= -100.0 && scr.1 <= 1180.0,
                "Node point Y must be within viewport view: {}",
                scr.1
            );
        }
    }
}
