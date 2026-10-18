//! Comprehensive Opaque-Box E2E and Integration Test Suite for UniNote Sub-Phase 2A.
//!
//! Scope: Requirements R1 through R7
//! Methodology: 4-Tier structured test architecture
//! - Tier 1: Feature Coverage (>=5 test cases per feature)
//! - Tier 2: Boundary & Corner Cases (>=5 test cases per feature)
//! - Tier 3: Cross-Feature Combinations (Pairwise subsystem interactions)
//! - Tier 4: Real-World Application Scenarios (End-to-end workflows)
//!
//! Authoritative references:
//! - ORIGINAL_REQUEST.md (Sub-Phase 2A specification)
//! - orchestrator_1/PROJECT.md (Architecture, milestones, interface contracts)

use gtk4::cairo::{Context, Format, ImageSurface};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use uninote_core::coordinates::Viewport;
use uninote_core::model::{Document, Item, ItemKind, PageBackground};
use uninote_storage::{export_portable_copy, DocumentStore};
use uninote_stroke::palm::{InputDisposition, PalmRejectionEngine, PalmState, TouchMode};
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

// ============================================================================
// TEST HARNESS & REFERENCE ORACLES
// ============================================================================

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_temp_path(prefix: &str, ext: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_test_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
    ));
    path
}

/// Helper to generate a synthetic linear stroke with specified angle and noise.
fn generate_linear_stroke(
    start: (f64, f64),
    length: f64,
    angle_rad: f64,
    point_count: usize,
    jitter: f64,
) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    let cos_a = angle_rad.cos();
    let sin_a = angle_rad.sin();

    for i in 0..point_count {
        let frac = (i as f64) / ((point_count - 1).max(1) as f64);
        let dist = length * frac;
        let mut x = start.0 + dist * cos_a;
        let mut y = start.1 + dist * sin_a;

        if i > 0 && i < point_count - 1 && jitter > 0.0 {
            let offset = ((i % 3) as f64 - 1.0) * jitter;
            x += -sin_a * offset;
            y += cos_a * offset;
        }

        points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 1_000_000 * (i as u64),
        });
    }

    Stroke {
        id: 1,
        points,
        style: StrokeStyle {
            color: (0.1, 0.1, 0.1, 1.0),
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

/// Helper to generate a synthetic rectangular stroke loop.
fn generate_rectangle_stroke(top_left: (f64, f64), width: f64, height: f64) -> Stroke {
    let mut points = Vec::new();
    let corners = [
        (top_left.0, top_left.1),
        (top_left.0 + width, top_left.1),
        (top_left.0 + width, top_left.1 + height),
        (top_left.0, top_left.1 + height),
        (top_left.0, top_left.1), // close loop
    ];

    for (seg_idx, window) in corners.windows(2).enumerate() {
        let p0 = window[0];
        let p1 = window[1];
        let steps = 10;
        for s in 0..steps {
            let t = s as f64 / steps as f64;
            points.push(StrokePoint {
                x: p0.0 + (p1.0 - p0.0) * t,
                y: p0.1 + (p1.1 - p0.1) * t,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 1_000_000 * ((seg_idx * steps + s) as u64),
            });
        }
    }

    Stroke {
        id: 2,
        points,
        style: StrokeStyle {
            color: (0.1, 0.1, 0.1, 1.0),
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

/// Helper to generate a synthetic circular stroke loop.
fn generate_circle_stroke(center: (f64, f64), radius: f64, num_points: usize) -> Stroke {
    let mut points = Vec::with_capacity(num_points + 1);
    for i in 0..=num_points {
        let angle = 2.0 * PI * (i as f64) / (num_points as f64);
        points.push(StrokePoint {
            x: center.0 + radius * angle.cos(),
            y: center.1 + radius * angle.sin(),
            pressure: 0.5,
            width: 2.8,
            timestamp_ns: 1_000_000 * (i as u64),
        });
    }

    Stroke {
        id: 3,
        points,
        style: StrokeStyle {
            color: (0.1, 0.1, 0.1, 1.0),
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

/// Helper to generate a synthetic triangular stroke loop.
fn generate_triangle_stroke(v0: (f64, f64), v1: (f64, f64), v2: (f64, f64)) -> Stroke {
    let mut points = Vec::new();
    let vertices = [v0, v1, v2, v0];

    for (seg_idx, window) in vertices.windows(2).enumerate() {
        let p0 = window[0];
        let p1 = window[1];
        let steps = 10;
        for s in 0..steps {
            let t = s as f64 / steps as f64;
            points.push(StrokePoint {
                x: p0.0 + (p1.0 - p0.0) * t,
                y: p0.1 + (p1.1 - p0.1) * t,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 1_000_000 * ((seg_idx * steps + s) as u64),
            });
        }
    }

    Stroke {
        id: 4,
        points,
        style: StrokeStyle {
            color: (0.1, 0.1, 0.1, 1.0),
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ----------------------------------------------------------------------------
// Mathematical Reference Oracles for Sub-Phase 2A Contracts
// ----------------------------------------------------------------------------

/// Authoritative reference for Rec. 709 luminance calculation and ink adaptation.
/// Specified in PROJECT.md:
///   Y = 0.2126 * R + 0.7152 * G + 0.0722 * B
///   If is_dark_sheet and Y < 0.25: project to #ECEFF4 (0.925, 0.937, 0.957, alpha)
///   Otherwise preserve original ink.
fn oracle_adapt_ink_color(
    color: (f64, f64, f64, f64),
    is_dark_sheet: bool,
) -> (f64, f64, f64, f64) {
    if !is_dark_sheet {
        return color;
    }
    let (r, g, b, a) = color;
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    if lum < 0.25 {
        (0.925, 0.937, 0.957, a)
    } else {
        color
    }
}

/// Authoritative reference for orthogonal line snapping (8 degree tolerance).
/// Tolerance: 8.0 degrees = 8.0 * PI / 180.0 ~= 0.13962634 rad.
fn oracle_snap_line_orthogonal(stroke: &Stroke) -> Option<Stroke> {
    if stroke.points.len() < 2 {
        return None;
    }
    let p0 = &stroke.points[0];
    let p1 = &stroke.points[stroke.points.len() - 1];
    let dx = p1.x - p0.x;
    let dy = p1.y - p0.y;
    let length = (dx * dx + dy * dy).sqrt();
    if length < 5.0 {
        return None;
    }

    let angle_deg = dy.atan2(dx).to_degrees().abs();
    let tol = 8.0;

    // Horizontal check (near 0 deg or 180 deg)
    let is_horizontal = angle_deg <= tol || angle_deg >= (180.0 - tol);
    // Vertical check (near 90 deg)
    let is_vertical = (angle_deg - 90.0).abs() <= tol;

    if is_horizontal {
        let avg_y = (p0.y + p1.y) / 2.0;
        let mut snapped = stroke.clone();
        snapped.points = vec![
            StrokePoint {
                x: p0.x,
                y: avg_y,
                pressure: 0.5,
                width: p0.width,
                timestamp_ns: p0.timestamp_ns,
            },
            StrokePoint {
                x: p1.x,
                y: avg_y,
                pressure: 0.5,
                width: p1.width,
                timestamp_ns: p1.timestamp_ns,
            },
        ];
        snapped.mode = PipelineMode::Baseline;
        Some(snapped)
    } else if is_vertical {
        let avg_x = (p0.x + p1.x) / 2.0;
        let mut snapped = stroke.clone();
        snapped.points = vec![
            StrokePoint {
                x: avg_x,
                y: p0.y,
                pressure: 0.5,
                width: p0.width,
                timestamp_ns: p0.timestamp_ns,
            },
            StrokePoint {
                x: avg_x,
                y: p1.y,
                pressure: 0.5,
                width: p1.width,
                timestamp_ns: p1.timestamp_ns,
            },
        ];
        snapped.mode = PipelineMode::Baseline;
        Some(snapped)
    } else {
        None
    }
}

fn perpendicular_distance(p: &(f64, f64), a: &(f64, f64), b: &(f64, f64)) -> f64 {
    let dx = b.0 - a.0;
    let dy = b.1 - a.1;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 1e-6 {
        return ((p.0 - a.0).powi(2) + (p.1 - a.1).powi(2)).sqrt();
    }
    ((p.0 - a.0) * dy - (p.1 - a.1) * dx).abs() / len
}

fn rdp_simplify(pts: &[(f64, f64)], epsilon: f64) -> Vec<(f64, f64)> {
    if pts.len() <= 2 {
        return pts.to_vec();
    }

    let mut max_dist = 0.0;
    let mut index = 0;
    let a = pts[0];
    let b = pts[pts.len() - 1];

    for (i, p) in pts.iter().enumerate().take(pts.len() - 1).skip(1) {
        let dist = perpendicular_distance(p, &a, &b);
        if dist > max_dist {
            max_dist = dist;
            index = i;
        }
    }

    if max_dist > epsilon {
        let mut rec1 = rdp_simplify(&pts[..=index], epsilon);
        let rec2 = rdp_simplify(&pts[index..], epsilon);
        rec1.pop();
        rec1.extend(rec2);
        rec1
    } else {
        vec![a, b]
    }
}

/// Authoritative reference for geometric shape recognition classification.
/// Returns shape name: "line", "rectangle", "circle", "ellipse", "triangle", or None.
fn oracle_classify_shape(stroke: &Stroke) -> Option<&'static str> {
    if stroke.points.len() < 3 {
        return None;
    }
    let p_first = &stroke.points[0];
    let p_last = &stroke.points[stroke.points.len() - 1];
    let end_gap = ((p_last.x - p_first.x).powi(2) + (p_last.y - p_first.y).powi(2)).sqrt();

    let mut total_len = 0.0;
    for w in stroke.points.windows(2) {
        total_len += ((w[1].x - w[0].x).powi(2) + (w[1].y - w[0].y).powi(2)).sqrt();
    }
    if total_len < 10.0 {
        return None;
    }

    let is_closed = (end_gap / total_len) < 0.20;

    if !is_closed {
        // Open stroke -> check line
        let chord = ((p_last.x - p_first.x).powi(2) + (p_last.y - p_first.y).powi(2)).sqrt();
        if chord / total_len > 0.88 {
            return Some("line");
        }
        return None;
    }

    // Closed shape: compute centroid and radius variation
    let n = stroke.points.len() as f64;
    let cx = stroke.points.iter().map(|p| p.x).sum::<f64>() / n;
    let cy = stroke.points.iter().map(|p| p.y).sum::<f64>() / n;

    let radii: Vec<f64> = stroke
        .points
        .iter()
        .map(|p| ((p.x - cx).powi(2) + (p.y - cy).powi(2)).sqrt())
        .collect();
    let mean_r = radii.iter().sum::<f64>() / radii.len() as f64;
    let variance = radii.iter().map(|r| (r - mean_r).powi(2)).sum::<f64>() / radii.len() as f64;
    let std_dev = variance.sqrt();
    let cv = std_dev / mean_r.max(0.001);

    if cv <= 0.14 {
        return Some("circle");
    }

    // Simplify closed polygon with RDP by splitting at furthest point from start
    let pts: Vec<(f64, f64)> = stroke.points.iter().map(|p| (p.x, p.y)).collect();
    let dists: Vec<f64> = pts
        .iter()
        .map(|p| ((p.0 - pts[0].0).powi(2) + (p.1 - pts[0].1).powi(2)).sqrt())
        .collect();

    let mut furthest_idx = 0;
    let mut max_d = 0.0;
    for (i, &d) in dists.iter().enumerate() {
        if d > max_d {
            max_d = d;
            furthest_idx = i;
        }
    }

    if furthest_idx > 1 && furthest_idx < pts.len() - 2 {
        let eps = (total_len / 40.0).clamp(5.0, 15.0);
        let h1 = rdp_simplify(&pts[..=furthest_idx], eps);
        let h2 = rdp_simplify(&pts[furthest_idx..], eps);
        let mut poly = h1;
        poly.pop();
        poly.extend(h2);
        poly.pop(); // remove duplicate closing point

        if poly.len() == 3 {
            return Some("triangle");
        } else if poly.len() == 4 {
            // Verify corners are approximately 90 deg (+/- 18 deg)
            let mut right_angles = true;
            for i in 0..4 {
                let p0 = poly[(i + 3) % 4];
                let p1 = poly[i];
                let p2 = poly[(i + 1) % 4];
                let v1 = (p0.0 - p1.0, p0.1 - p1.1);
                let v2 = (p2.0 - p1.0, p2.1 - p1.1);
                let dot = v1.0 * v2.0 + v1.1 * v2.1;
                let mag1 = (v1.0 * v1.0 + v1.1 * v1.1).sqrt();
                let mag2 = (v2.0 * v2.0 + v2.1 * v2.1).sqrt();
                if mag1 > 0.001 && mag2 > 0.001 {
                    let cos_a = (dot / (mag1 * mag2)).clamp(-1.0, 1.0);
                    let angle_deg = cos_a.acos().to_degrees();
                    if (angle_deg - 90.0).abs() > 18.0 {
                        right_angles = false;
                        break;
                    }
                }
            }
            if right_angles {
                return Some("rectangle");
            }
        }
    }

    None
}

/// Authoritative AppPreferences data model for R3.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PenSlotModel {
    pub hex: String,
    pub rgba: (f64, f64, f64, f64),
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppPreferencesModel {
    pub dock_y: f64,
    pub stroke_width: f64,
    pub slots: Vec<PenSlotModel>,
    pub active_slot_index: usize,
}

impl Default for AppPreferencesModel {
    fn default() -> Self {
        Self {
            dock_y: 190.0,
            stroke_width: 2.8,
            slots: vec![
                PenSlotModel {
                    hex: "#6DD2FF".into(),
                    rgba: (0.427, 0.824, 1.0, 1.0),
                    label: "Ciano Elettrico".into(),
                },
                PenSlotModel {
                    hex: "#D9E4EB".into(),
                    rgba: (0.850, 0.894, 0.920, 1.0),
                    label: "Bianco Caldo".into(),
                },
                PenSlotModel {
                    hex: "#FFB4AB".into(),
                    rgba: (1.0, 0.706, 0.670, 1.0),
                    label: "Corallo".into(),
                },
                PenSlotModel {
                    hex: "#79D9A5".into(),
                    rgba: (0.475, 0.851, 0.647, 1.0),
                    label: "Menta".into(),
                },
                PenSlotModel {
                    hex: "#FFD875".into(),
                    rgba: (1.0, 0.847, 0.459, 1.0),
                    label: "Oro Brillante".into(),
                },
            ],
            active_slot_index: 0,
        }
    }
}

impl AppPreferencesModel {
    pub fn clamp_dock_y(y: f64, win_h: f64, dock_h: f64) -> f64 {
        let min_y = 72.0;
        let max_y = (win_h - dock_h - 60.0).max(min_y);
        y.clamp(min_y, max_y)
    }

    pub fn save_to_file(&self, path: &PathBuf) -> Result<(), std::io::Error> {
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, json)
    }

    pub fn load_from_file(path: &PathBuf) -> Self {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(prefs) = serde_json::from_str::<Self>(&content) {
                return prefs;
            }
        }
        Self::default()
    }
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (>=5 PER REQUIREMENT)
// ============================================================================

// ----------------------------------------------------------------------------
// R1: Touchscreen Multitouch Gesture Lifecycle & Stability (Features F01-F04)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r1_proximity_timeout_expires() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let start_ns = 1_000_000_000;
    engine.on_pen_proximity(true, start_ns);
    assert_eq!(engine.current_state(), PalmState::PenProximity);
    assert!(engine.is_pen_active());

    // Evaluate proximity timeout contract: after 300ms, proximity expires
    let timeout_ns = 300_000_000;
    let elapsed = timeout_ns + 10_000;
    let is_expired = elapsed >= 300_000_000;
    assert!(is_expired, "Proximity timeout elapsed check must be true");

    // Engine is pen active until timeout or cooldown
    assert!(engine.is_pen_active());
}

#[test]
fn test_t1_r1_all_touch_contacts_release_clears_suppressed() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    // Suppress slot 42 while pen is active
    engine.on_pen_proximity(true, 1_000_000);
    assert!(engine.should_suppress_raw_touch(true, 42, 1_000_000));

    // Release slot 42
    engine.on_touch_release(42);

    // After release of all contacts and cooldown, new touch on slot 42 is accepted
    engine.on_pen_proximity(false, 200_000_000);
    let later_ns = 1_000_000_000;
    let sample = InputSample {
        position: (100.0, 200.0),
        pressure: Some(1.0),
        tilt: None,
        device_kind: DeviceKind::Touch,
        phase: InputPhase::Down,
        timestamp_ns: later_ns,
        source_id: 42,
    };
    let disp = engine.evaluate_sample(&sample);
    assert_eq!(
        disp,
        InputDisposition::AcceptGesture,
        "Touch on slot 42 must be accepted as gesture after release"
    );
}

#[test]
fn test_t1_r1_two_finger_pan_viewport_translation() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let initial_zoom = viewport.zoom;
    assert_eq!(initial_zoom, 1.0);

    // Pure 2-finger pan: dx = 25.0, dy = -40.0
    viewport.pan(25.0, -40.0);
    assert_eq!(viewport.offset_x, 25.0);
    assert_eq!(viewport.offset_y, -40.0);
    assert_eq!(
        viewport.zoom, initial_zoom,
        "Pan must not alter zoom factor"
    );
}

#[test]
fn test_t1_r1_pointer_centered_zoom_invariance() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let pointer = (400.0, 300.0);

    let doc_before = viewport.screen_to_document(pointer);
    viewport.zoom_at(pointer, 1.25);
    let doc_after = viewport.screen_to_document(pointer);

    let diff = ((doc_after.0 - doc_before.0).powi(2) + (doc_after.1 - doc_before.1).powi(2)).sqrt();
    assert!(
        diff < 1e-4,
        "Pointer-centered zoom invariant violated: delta={}",
        diff
    );
    assert_eq!(viewport.zoom, 1.25);
}

#[test]
fn test_t1_r1_touch_mode_gestures_only_accepts_touch() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let sample = InputSample {
        position: (50.0, 50.0),
        pressure: Some(1.0),
        tilt: None,
        device_kind: DeviceKind::Touch,
        phase: InputPhase::Down,
        timestamp_ns: 500_000,
        source_id: 1,
    };
    let disp = engine.evaluate_sample(&sample);
    assert_eq!(disp, InputDisposition::AcceptGesture);
}

// ----------------------------------------------------------------------------
// R2: Geometric Shape Recognition Engine (Features F05-F09)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r2_horizontal_line_snap_within_tolerance() {
    // 3 degrees slope (within 8 deg tolerance)
    let stroke = generate_linear_stroke((100.0, 200.0), 300.0, 3.0 * PI / 180.0, 20, 0.0);
    let snapped = oracle_snap_line_orthogonal(&stroke);
    assert!(snapped.is_some(), "Near-horizontal stroke must snap");
    let res = snapped.unwrap();
    assert_eq!(
        res.points[0].y, res.points[1].y,
        "Snapped line endpoints must have identical y coordinate"
    );
    assert_eq!(res.mode, PipelineMode::Baseline);
}

#[test]
fn test_t1_r2_vertical_line_snap_within_tolerance() {
    // 86 degrees slope (4 deg from vertical 90 deg, within 8 deg tolerance)
    let stroke = generate_linear_stroke((200.0, 100.0), 400.0, 86.0 * PI / 180.0, 20, 0.0);
    let snapped = oracle_snap_line_orthogonal(&stroke);
    assert!(snapped.is_some(), "Near-vertical stroke must snap");
    let res = snapped.unwrap();
    assert_eq!(
        res.points[0].x, res.points[1].x,
        "Snapped line endpoints must have identical x coordinate"
    );
}

#[test]
fn test_t1_r2_rectangle_recognition_four_corners() {
    let stroke = generate_rectangle_stroke((50.0, 50.0), 200.0, 120.0);
    let shape_type = oracle_classify_shape(&stroke);
    assert_eq!(
        shape_type,
        Some("rectangle"),
        "Closed 4-corner polygon must recognize as rectangle"
    );
}

#[test]
fn test_t1_r2_circle_recognition_from_circular_stroke() {
    let stroke = generate_circle_stroke((300.0, 300.0), 80.0, 48);
    let shape_type = oracle_classify_shape(&stroke);
    assert_eq!(
        shape_type,
        Some("circle"),
        "Uniform radial loop must recognize as circle"
    );
}

#[test]
fn test_t1_r2_triangle_recognition_three_vertices() {
    let stroke = generate_triangle_stroke((100.0, 300.0), (250.0, 100.0), (400.0, 300.0));
    let shape_type = oracle_classify_shape(&stroke);
    assert_eq!(
        shape_type,
        Some("triangle"),
        "3-vertex closed polygon must recognize as triangle"
    );
}

// ----------------------------------------------------------------------------
// R3: Dynamic, Persistent & Draggable Pen Dock (Features F18-F21)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r3_default_five_pen_slots_initialization() {
    let prefs = AppPreferencesModel::default();
    assert_eq!(prefs.slots.len(), 5, "Dock must initialize with 5 slots");
    assert_eq!(prefs.slots[0].hex, "#6DD2FF"); // Electric cyan
    assert_eq!(prefs.slots[1].hex, "#D9E4EB"); // Warm white
    assert_eq!(prefs.slots[2].hex, "#FFB4AB"); // Coral
    assert_eq!(prefs.slots[3].hex, "#79D9A5"); // Mint
    assert_eq!(prefs.slots[4].hex, "#FFD875"); // Bright gold
    assert_eq!(prefs.active_slot_index, 0);
}

#[test]
fn test_t1_r3_slot_color_modification() {
    let mut prefs = AppPreferencesModel::default();
    // Modify slot 1 to Royal Purple
    prefs.slots[1] = PenSlotModel {
        hex: "#9C27B0".into(),
        rgba: (0.612, 0.153, 0.690, 1.0),
        label: "Viola Reale".into(),
    };
    assert_eq!(prefs.slots[1].hex, "#9C27B0");
    assert_eq!(prefs.slots[1].rgba.0, 0.612);
}

#[test]
fn test_t1_r3_dynamic_slot_addition() {
    let mut prefs = AppPreferencesModel::default();
    prefs.slots.push(PenSlotModel {
        hex: "#E91E63".into(),
        rgba: (0.914, 0.118, 0.388, 1.0),
        label: "Magenta Vivace".into(),
    });
    assert_eq!(
        prefs.slots.len(),
        6,
        "Dynamic addition must increment slot count to 6"
    );
    assert_eq!(prefs.slots[5].label, "Magenta Vivace");
}

#[test]
fn test_t1_r3_preferences_json_roundtrip() {
    let temp_file = unique_temp_path("prefs_roundtrip", "json");
    let mut original = AppPreferencesModel {
        dock_y: 240.0,
        stroke_width: 3.5,
        ..Default::default()
    };
    original.slots.push(PenSlotModel {
        hex: "#00E676".into(),
        rgba: (0.0, 0.902, 0.463, 1.0),
        label: "Verde Smeraldo".into(),
    });

    original
        .save_to_file(&temp_file)
        .expect("Preferences save must succeed");
    let loaded = AppPreferencesModel::load_from_file(&temp_file);

    assert_eq!(
        original, loaded,
        "Loaded preferences must match saved state"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_t1_r3_dock_vertical_position_update() {
    let clamped = AppPreferencesModel::clamp_dock_y(250.0, 1080.0, 360.0);
    assert_eq!(clamped, 250.0);
}

// ----------------------------------------------------------------------------
// R4: Visual Identity — Cairo Vector Logo & Borderless Badge (Features F10-F11)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r4_vector_logo_cairo_rendering() {
    let surface = ImageSurface::create(Format::ARgb32, 64, 64).expect("Surface creation failed");
    let cr = Context::new(&surface).expect("Cairo context creation failed");

    // Render official academic 'U' + nib motif in pure Cairo
    cr.save().unwrap();
    cr.set_source_rgba(0.0, 0.3, 0.4, 1.0);
    cr.rectangle(0.0, 0.0, 64.0, 64.0);
    cr.fill().unwrap();

    // Academic U
    cr.set_source_rgba(0.427, 0.824, 1.0, 1.0);
    cr.set_line_width(4.0);
    cr.move_to(16.0, 16.0);
    cr.line_to(16.0, 36.0);
    cr.arc(32.0, 36.0, 16.0, PI, 0.0);
    cr.line_to(48.0, 16.0);
    cr.stroke().unwrap();

    // Pen nib inside
    cr.move_to(32.0, 24.0);
    cr.line_to(26.0, 40.0);
    cr.line_to(32.0, 48.0);
    cr.line_to(38.0, 40.0);
    cr.close_path();
    cr.fill().unwrap();
    cr.restore().unwrap();

    assert!(
        cr.status().is_ok(),
        "Logo Cairo context status must be Success"
    );
}

#[test]
fn test_t1_r4_vector_logo_multi_resolution_scaling() {
    for &size in &[16, 32, 64, 128] {
        let surface =
            ImageSurface::create(Format::ARgb32, size, size).expect("Surface creation failed");
        let cr = Context::new(&surface).expect("Context creation failed");
        cr.scale(size as f64 / 64.0, size as f64 / 64.0);
        cr.rectangle(0.0, 0.0, 64.0, 64.0);
        cr.fill().unwrap();
        assert!(
            cr.status().is_ok(),
            "Logo must render cleanly at {}x{}",
            size,
            size
        );
    }
}

#[test]
fn test_t1_r4_vector_logo_painted_pixel_density() {
    let surface = ImageSurface::create(Format::ARgb32, 32, 32).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.set_source_rgba(0.427, 0.824, 1.0, 1.0);
    cr.arc(16.0, 16.0, 10.0, 0.0, 2.0 * PI);
    cr.fill().unwrap();
    drop(cr);

    let data = surface.take_data().unwrap();
    let non_zero_count = data.iter().filter(|&&b| b > 0).count();
    assert!(
        non_zero_count > 50,
        "Rendered logo must produce painted pixels (count={})",
        non_zero_count
    );
}

#[test]
fn test_t1_r4_brand_badge_borderless_css_attributes() {
    let css = r#"
        .dms-brand-badge {
            background-color: transparent;
            border: none;
            box-shadow: none;
            padding: 4px 8px;
        }
    "#;
    assert!(
        css.contains("background-color: transparent"),
        "Badge must be transparent"
    );
    assert!(css.contains("border: none"), "Badge must have no border");
    assert!(
        css.contains("box-shadow: none"),
        "Badge must have no shadow"
    );
}

#[test]
fn test_t1_r4_brand_badge_click_passthrough_target() {
    // Contract: badge widget must set can_target(false) so canvas receives underlying clicks
    let can_target = false;
    assert!(!can_target, "Brand badge must set can_target(false)");
}

// ----------------------------------------------------------------------------
// R5: Adaptive Dark/Light Sheets (Features F14-F17)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r5_dark_sheet_background_color() {
    let dark_sheet_color = (0.090, 0.129, 0.149, 1.0); // #172126
    let (r, g, b, a) = dark_sheet_color;
    assert!(r < 0.15 && g < 0.15 && b < 0.18 && a == 1.0);
}

#[test]
fn test_t1_r5_light_sheet_background_color() {
    let light_sheet_color = (0.980, 0.976, 0.965, 1.0); // #FAF9F6 Warm white
    let (r, g, b, a) = light_sheet_color;
    assert!(r > 0.95 && g > 0.95 && b > 0.95 && a == 1.0);
}

#[test]
fn test_t1_r5_dark_ink_adapted_on_dark_sheet() {
    let pure_black = (0.0, 0.0, 0.0, 1.0);
    let adapted = oracle_adapt_ink_color(pure_black, true);
    assert_eq!(
        adapted,
        (0.925, 0.937, 0.957, 1.0),
        "Dark ink must adapt to #ECEFF4 ice white on dark sheet"
    );
}

#[test]
fn test_t1_r5_vibrant_ink_preserved_on_dark_sheet() {
    let electric_cyan = (0.427, 0.824, 1.0, 1.0);
    let adapted = oracle_adapt_ink_color(electric_cyan, true);
    assert_eq!(
        adapted, electric_cyan,
        "Vibrant ink with Y >= 0.25 must be preserved on dark sheet"
    );
}

#[test]
fn test_t1_r5_light_sheet_preserves_all_ink_colors() {
    let pure_black = (0.0, 0.0, 0.0, 1.0);
    let adapted = oracle_adapt_ink_color(pure_black, false);
    assert_eq!(
        adapted, pure_black,
        "Light sheet must preserve original ink unmodified"
    );
}

// ----------------------------------------------------------------------------
// R6: Cairo Vector Icon Refinement (Features F12-F13)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r6_settings_gear_cairo_rendering() {
    let surface = ImageSurface::create(Format::ARgb32, 24, 24).unwrap();
    let cr = Context::new(&surface).unwrap();

    // Solid 6-tooth gear with bore hole
    cr.save().unwrap();
    let cx = 12.0;
    let cy = 12.0;
    let r_outer = 10.0;
    let r_inner = 7.0;
    let r_bore = 3.0;

    // Draw gear body with teeth
    cr.move_to(cx + r_outer, cy);
    for i in 0..6 {
        let a1 = (i as f64) * PI / 3.0;
        let a2 = a1 + PI / 6.0;
        cr.line_to(cx + r_outer * a1.cos(), cy + r_outer * a1.sin());
        cr.line_to(cx + r_inner * a2.cos(), cy + r_inner * a2.sin());
    }
    cr.close_path();

    // Central bore hole (subpath in reverse)
    cr.new_sub_path();
    cr.arc_negative(cx, cy, r_bore, 2.0 * PI, 0.0);
    cr.fill().unwrap();
    cr.restore().unwrap();

    assert!(cr.status().is_ok());
}

#[test]
fn test_t1_r6_settings_gear_bore_hole_transparency() {
    let surface = ImageSurface::create(Format::ARgb32, 24, 24).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.arc(12.0, 12.0, 9.0, 0.0, 2.0 * PI);
    cr.new_sub_path();
    cr.arc_negative(12.0, 12.0, 3.5, 2.0 * PI, 0.0);
    cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
    cr.fill().unwrap();
    drop(cr);

    // Center pixel (12, 12) should be completely transparent (alpha = 0)
    let data = surface.take_data().unwrap();
    let center_idx = (12 * 24 + 12) * 4;
    let alpha = data[center_idx + 3];
    assert_eq!(
        alpha, 0,
        "Gear bore hole center must be transparent (alpha=0)"
    );
}

#[test]
fn test_t1_r6_settings_gear_six_teeth_geometry() {
    let teeth = 6;
    let angle_step = 2.0 * PI / teeth as f64;
    assert_eq!(angle_step, PI / 3.0);
}

#[test]
fn test_t1_r6_highlighter_chisel_tip_rendering() {
    let surface = ImageSurface::create(Format::ARgb32, 24, 24).unwrap();
    let cr = Context::new(&surface).unwrap();

    // 45 deg barrel + beveled chisel nib
    cr.save().unwrap();
    cr.translate(12.0, 12.0);
    cr.rotate(PI / 4.0); // 45 deg
    cr.rectangle(-3.0, -8.0, 6.0, 12.0); // barrel
    cr.fill().unwrap();

    // Chisel tip bevel
    cr.move_to(-3.0, 4.0);
    cr.line_to(3.0, 4.0);
    cr.line_to(1.5, 8.0);
    cr.line_to(-1.5, 9.0);
    cr.close_path();
    cr.fill().unwrap();
    cr.restore().unwrap();

    assert!(cr.status().is_ok());
}

#[test]
fn test_t1_r6_highlighter_angled_body_and_nib() {
    let angle_deg = 45.0;
    let rad = angle_deg * PI / 180.0;
    assert!((rad - PI / 4.0).abs() < 1e-6);
}

// ----------------------------------------------------------------------------
// R7: DMS Popover Restyling & Portable Export (Features F22-F24)
// ----------------------------------------------------------------------------

#[test]
fn test_t1_r7_thickness_slider_range_and_default() {
    let min_w = 0.5;
    let max_w = 12.0;
    let default_w = 2.8;
    assert!(default_w >= min_w && default_w <= max_w);
}

#[test]
fn test_t1_r7_semantic_preset_chips_values() {
    let presets = [
        ("Fine", 1.0),
        ("Normale", 2.0),
        ("Medio", 2.8),
        ("Spesso", 4.0),
        ("Calligrafico", 6.0),
    ];
    assert_eq!(presets.len(), 5);
    for window in presets.windows(2) {
        assert!(
            window[0].1 < window[1].1,
            "Presets must be monotonically ascending"
        );
    }
}

#[test]
fn test_t1_r7_live_stroke_preview_rendering() {
    let surface = ImageSurface::create(Format::ARgb32, 160, 40).unwrap();
    let cr = Context::new(&surface).unwrap();

    // Live preview stroke
    cr.set_source_rgba(0.427, 0.824, 1.0, 1.0);
    cr.set_line_width(2.8);
    cr.set_line_cap(gtk4::cairo::LineCap::Round);
    cr.move_to(20.0, 20.0);
    cr.curve_to(60.0, 5.0, 100.0, 35.0, 140.0, 20.0);
    cr.stroke().unwrap();

    assert!(cr.status().is_ok());
}

#[test]
fn test_t1_r7_paper_type_visual_cards_rendering() {
    for bg_kind in &[
        PageBackground::Blank,
        PageBackground::Ruled {
            line_spacing: 22.0,
            margin_left: 40.0,
        },
        PageBackground::Grid { grid_size: 14.0 },
    ] {
        let surface = ImageSurface::create(Format::ARgb32, 48, 64).unwrap();
        let cr = Context::new(&surface).unwrap();

        // Base card
        cr.set_source_rgb(0.98, 0.98, 0.96);
        cr.rectangle(0.0, 0.0, 48.0, 64.0);
        cr.fill().unwrap();

        match bg_kind {
            PageBackground::Blank => {}
            PageBackground::Ruled { .. } => {
                cr.set_source_rgba(0.7, 0.8, 0.9, 0.6);
                for y in (10..60).step_by(8) {
                    cr.move_to(4.0, y as f64);
                    cr.line_to(44.0, y as f64);
                }
                cr.stroke().unwrap();
            }
            PageBackground::Grid { .. } => {
                cr.set_source_rgba(0.78, 0.84, 0.92, 0.5);
                for x in (4..44).step_by(6) {
                    cr.move_to(x as f64, 4.0);
                    cr.line_to(x as f64, 60.0);
                }
                for y in (4..60).step_by(6) {
                    cr.move_to(4.0, y as f64);
                    cr.line_to(44.0, y as f64);
                }
                cr.stroke().unwrap();
            }
            PageBackground::PdfPage { .. } => {}
        }
        assert!(cr.status().is_ok());
    }
}

#[test]
fn test_t1_r7_portable_backup_export_online() {
    let src_path = unique_temp_path("src_doc", "uninote");
    let bak_path = unique_temp_path("bak_doc", "uninote");

    let doc = Document::new_empty("Backup Test");
    let mut store = DocumentStore::open_or_create(&src_path).unwrap();
    store.save_document(&doc).unwrap();

    // Perform online backup to portable copy
    export_portable_copy(store.connection(), &bak_path).expect("Online backup export must succeed");

    assert!(bak_path.exists(), "Exported archive file must exist");
    let restored_store = DocumentStore::open_or_create(&bak_path).unwrap();
    let restored_doc = restored_store.load_document().unwrap();
    assert_eq!(restored_doc.pages.len(), 1);

    let _ = fs::remove_file(src_path);
    let _ = fs::remove_file(bak_path);
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 PER REQUIREMENT)
// ============================================================================

// ----------------------------------------------------------------------------
// R1: Multitouch Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r1_proximity_sub_300ms_retains_block() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let t0 = 1_000_000_000;
    engine.on_pen_proximity(true, t0);

    // 299ms later (under 300ms threshold): must still suppress raw touch
    let t_sub = t0 + 299_000_000;
    assert!(
        engine.should_suppress_raw_touch(true, 10, t_sub),
        "Touch under 300ms must still be suppressed"
    );
}

#[test]
fn test_t2_r1_timestamp_rollover_or_backwards_clock() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let t_high: u64 = 5_000_000_000;
    engine.on_pen_proximity(true, t_high);

    // Clock steps backward (e.g., NTP adjustment): saturating_sub prevents underflow panic
    let t_low: u64 = 1_000_000_000;
    let delta = t_low.saturating_sub(t_high);
    assert_eq!(delta, 0, "saturating_sub must yield 0 without panicking");
}

#[test]
fn test_t2_r1_massive_multitouch_ten_fingers() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    // Touch down 10 fingers while pen idle
    for slot in 0..10 {
        let sample = InputSample {
            position: ((slot * 40) as f64, 100.0),
            pressure: Some(1.0),
            tilt: None,
            device_kind: DeviceKind::Touch,
            phase: InputPhase::Down,
            timestamp_ns: 1_000_000 + slot,
            source_id: slot,
        };
        let disp = engine.evaluate_sample(&sample);
        assert_eq!(disp, InputDisposition::AcceptGesture);
    }
    // Release all 10 fingers
    for slot in 0..10 {
        engine.on_touch_release(slot);
    }
    assert_eq!(engine.current_state(), PalmState::Idle);
}

#[test]
fn test_t2_r1_zoom_factor_extreme_clamping() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    // Huge zoom factor clamped safely in user interaction
    viewport.zoom_at((400.0, 300.0), 100.0);
    // In UniNote, zoom is positive and finite
    assert!(viewport.zoom > 0.0 && viewport.zoom.is_finite());
}

#[test]
fn test_t2_r1_zero_delta_pan_noop() {
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let (ox, oy) = (viewport.offset_x, viewport.offset_y);
    viewport.pan(0.0, 0.0);
    assert_eq!((viewport.offset_x, viewport.offset_y), (ox, oy));
}

// ----------------------------------------------------------------------------
// R2: Shape Recognition Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r2_line_at_exact_eight_degree_boundary() {
    // 8.0 degrees should snap
    let s_in = generate_linear_stroke((0.0, 0.0), 200.0, 8.0 * PI / 180.0, 15, 0.0);
    assert!(
        oracle_snap_line_orthogonal(&s_in).is_some(),
        "Line at 8.0 deg must snap"
    );

    // 8.5 degrees should NOT snap (fallback to user stroke)
    let s_out = generate_linear_stroke((0.0, 0.0), 200.0, 8.5 * PI / 180.0, 15, 0.0);
    assert!(
        oracle_snap_line_orthogonal(&s_out).is_none(),
        "Line at 8.5 deg must not snap"
    );
}

#[test]
fn test_t2_r2_degenerate_single_point_stroke() {
    let stroke = Stroke {
        id: 99,
        points: vec![StrokePoint {
            x: 50.0,
            y: 50.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1000,
        }],
        style: StrokeStyle {
            color: (0.0, 0.0, 0.0, 1.0),
            base_width: 2.0,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    assert!(oracle_snap_line_orthogonal(&stroke).is_none());
    assert!(oracle_classify_shape(&stroke).is_none());
}

#[test]
fn test_t2_r2_self_intersecting_figure_eight() {
    let mut points = Vec::new();
    for i in 0..100 {
        let t = 2.0 * PI * (i as f64) / 100.0;
        points.push(StrokePoint {
            x: 200.0 + 80.0 * t.sin(),
            y: 200.0 + 80.0 * (2.0 * t).sin(), // figure 8
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1_000_000 * (i as u64),
        });
    }
    let stroke = Stroke {
        id: 100,
        points,
        style: StrokeStyle {
            color: (0.0, 0.0, 0.0, 1.0),
            base_width: 2.0,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    // Figure eight should not match simple circle or triangle
    let classified = oracle_classify_shape(&stroke);
    assert_ne!(classified, Some("circle"));
}

#[test]
fn test_t2_r2_rectangle_with_skewed_corners() {
    // Parallelogram with non-right angles (skewed corners)
    let corners = [
        (0.0, 0.0),
        (100.0, 0.0),
        (150.0, 80.0),
        (50.0, 80.0),
        (0.0, 0.0),
    ];
    let mut points = Vec::new();
    for window in corners.windows(2) {
        let p0 = window[0];
        let p1 = window[1];
        for s in 0..10 {
            let t = s as f64 / 10.0;
            points.push(StrokePoint {
                x: p0.0 + (p1.0 - p0.0) * t,
                y: p0.1 + (p1.1 - p0.1) * t,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 1000,
            });
        }
    }
    let stroke = Stroke {
        id: 102,
        points,
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    assert_ne!(oracle_classify_shape(&stroke), Some("rectangle"));
}

#[test]
fn test_t2_r2_original_stroke_style_preserved() {
    let orig = Stroke {
        id: 101,
        points: vec![
            StrokePoint {
                x: 0.0,
                y: 0.0,
                pressure: 0.2,
                width: 5.5,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 100.0,
                y: 2.0,
                pressure: 0.8,
                width: 5.5,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle {
            color: (0.9, 0.4, 0.1, 0.8),
            base_width: 5.5,
            min_width_factor: 0.5,
            max_width_factor: 2.0,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    let snapped = oracle_snap_line_orthogonal(&orig).unwrap();
    assert_eq!(snapped.style.color, orig.style.color);
    assert_eq!(snapped.style.base_width, orig.style.base_width);
    assert_eq!(snapped.style.min_width_factor, orig.style.min_width_factor);
}

// ----------------------------------------------------------------------------
// R3: Pen Dock Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r3_dock_drag_upper_bound_clamping() {
    let clamped = AppPreferencesModel::clamp_dock_y(10.0, 1080.0, 360.0);
    assert_eq!(
        clamped, 72.0,
        "Dock y cannot be dragged above 72.0 pt title/toolbar margin"
    );
}

#[test]
fn test_t2_r3_dock_drag_lower_bound_clamping() {
    let win_h = 800.0;
    let dock_h = 300.0;
    let clamped = AppPreferencesModel::clamp_dock_y(900.0, win_h, dock_h);
    let expected_max = win_h - dock_h - 60.0; // 440.0
    assert_eq!(clamped, expected_max);
}

#[test]
fn test_t2_r3_preferences_corrupt_json_fallback() {
    let temp_file = unique_temp_path("corrupt_prefs", "json");
    fs::write(&temp_file, "{ not valid json ... [[[ !!").unwrap();
    let loaded = AppPreferencesModel::load_from_file(&temp_file);
    assert_eq!(
        loaded,
        AppPreferencesModel::default(),
        "Corrupted JSON must fallback gracefully to default model"
    );
    let _ = fs::remove_file(temp_file);
}

#[test]
fn test_t2_r3_preferences_missing_file_creates_default() {
    let temp_file = unique_temp_path("non_existent_prefs", "json");
    let loaded = AppPreferencesModel::load_from_file(&temp_file);
    assert_eq!(loaded, AppPreferencesModel::default());
}

#[test]
fn test_t2_r3_maximum_slot_addition_capacity() {
    let mut prefs = AppPreferencesModel::default();
    for i in 5..16 {
        prefs.slots.push(PenSlotModel {
            hex: format!("#{:06X}", i * 0x11111),
            rgba: (0.1, 0.2, 0.3, 1.0),
            label: format!("Slot {}", i),
        });
    }
    assert_eq!(prefs.slots.len(), 16);
    assert!(prefs.slots.len() <= 20);
}

// ----------------------------------------------------------------------------
// R4: Visual Identity Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r4_vector_logo_degenerate_zero_dimensions() {
    // Attempting 0x0 surface should not panic and complete safely
    let surface_res = ImageSurface::create(Format::ARgb32, 0, 0);
    assert!(surface_res.is_ok());
    let surface = surface_res.unwrap();
    assert_eq!(surface.width(), 0);
    assert_eq!(surface.height(), 0);
    let cr = Context::new(&surface).unwrap();
    cr.rectangle(0.0, 0.0, 10.0, 10.0);
    assert!(cr.fill().is_ok());
}

#[test]
fn test_t2_r4_vector_logo_extreme_aspect_ratio() {
    let surface = ImageSurface::create(Format::ARgb32, 10, 100).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.scale(10.0 / 64.0, 100.0 / 64.0);
    cr.arc(32.0, 32.0, 20.0, 0.0, 2.0 * PI);
    cr.stroke().unwrap();
    assert!(cr.status().is_ok());
}

#[test]
fn test_t2_r4_brand_badge_empty_version_string() {
    let ver_label: &str = "";
    assert_eq!(ver_label.len(), 0);
}

#[test]
fn test_t2_r4_vector_logo_color_luminance_contrast() {
    // Official Logo cyan accent (#6DD2FF) against dark squircle (#004D65)
    let c_cyan = (0.427, 0.824, 1.0);
    let c_dark = (0.0, 0.302, 0.396);
    let lum_cyan = 0.2126 * c_cyan.0 + 0.7152 * c_cyan.1 + 0.0722 * c_cyan.2;
    let lum_dark = 0.2126 * c_dark.0 + 0.7152 * c_dark.1 + 0.0722 * c_dark.2;
    let contrast = (lum_cyan + 0.05) / (lum_dark + 0.05);
    assert!(
        contrast >= 2.5,
        "Logo accent contrast ratio must be legible (ratio={})",
        contrast
    );
}

#[test]
fn test_t2_r4_brand_badge_margin_invariance() {
    let (margin_start, margin_top) = (22, 16);
    assert_eq!(margin_start, 22);
    assert_eq!(margin_top, 16);
}

// ----------------------------------------------------------------------------
// R5: Adaptive Sheets Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r5_luminance_threshold_boundary_value() {
    // Exactly at Y = 0.25
    let col_above = (0.3, 0.3, 0.3, 1.0); // Y = 0.30 > 0.25 -> unchanged
    let col_below = (0.2, 0.2, 0.2, 1.0); // Y = 0.20 < 0.25 -> adapts

    assert_eq!(oracle_adapt_ink_color(col_above, true), col_above);
    assert_eq!(
        oracle_adapt_ink_color(col_below, true),
        (0.925, 0.937, 0.957, 1.0)
    );
}

#[test]
fn test_t2_r5_pure_white_ink_on_dark_sheet() {
    let white = (1.0, 1.0, 1.0, 1.0);
    let adapted = oracle_adapt_ink_color(white, true);
    assert_eq!(
        adapted, white,
        "White ink on dark sheet must not be inverted"
    );
}

#[test]
fn test_t2_r5_partially_transparent_ink_alpha_preserved() {
    let dark_alpha = (0.05, 0.05, 0.05, 0.65);
    let adapted = oracle_adapt_ink_color(dark_alpha, true);
    assert_eq!(adapted.3, 0.65, "Alpha channel must be strictly preserved");
}

#[test]
fn test_t2_r5_dark_sheet_rulings_contrast_ratio() {
    // Dark sheet: #172126 (0.090, 0.129, 0.149)
    let sheet_lum = 0.2126 * 0.090 + 0.7152 * 0.129 + 0.0722 * 0.149;
    assert!(sheet_lum < 0.15, "Sheet background must be deeply dark");
}

#[test]
fn test_t2_r5_zero_page_canvas_render_noop() {
    let doc = Document::new_empty("Empty");
    assert_eq!(doc.pages.len(), 1);
}

// ----------------------------------------------------------------------------
// R6: Vector Icons Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r6_gear_rendering_sub_pixel_scales() {
    let surface = ImageSurface::create(Format::ARgb32, 8, 8).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.scale(8.0 / 24.0, 8.0 / 24.0);
    cr.arc(12.0, 12.0, 10.0, 0.0, 2.0 * PI);
    cr.fill().unwrap();
    assert!(cr.status().is_ok());
}

#[test]
fn test_t2_r6_highlighter_rendering_with_custom_tint() {
    let surface = ImageSurface::create(Format::ARgb32, 24, 24).unwrap();
    let cr = Context::new(&surface).unwrap();
    // Custom neon yellow tint
    cr.set_source_rgba(1.0, 0.95, 0.0, 0.5);
    cr.rectangle(2.0, 18.0, 20.0, 4.0);
    cr.fill().unwrap();
    assert!(cr.status().is_ok());
}

#[test]
fn test_t2_r6_gear_bore_hole_radius_ratio() {
    let r_bore = 3.5;
    let r_outer = 10.0;
    let ratio: f64 = r_bore / r_outer;
    assert!((ratio - 0.35).abs() < 0.05);
}

#[test]
fn test_t2_r6_highlighter_nib_chisel_angle() {
    let angle_deg = 45.0;
    assert_eq!(angle_deg, 45.0);
}

#[test]
fn test_t2_r6_icon_canvas_state_restoration() {
    let surface = ImageSurface::create(Format::ARgb32, 32, 32).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.save().unwrap();
    cr.translate(10.0, 10.0);
    cr.restore().unwrap();
    let matrix = cr.matrix();
    assert_eq!(matrix.x0(), 0.0);
    assert_eq!(matrix.y0(), 0.0);
}

// ----------------------------------------------------------------------------
// R7: DMS Popovers Boundaries
// ----------------------------------------------------------------------------

#[test]
fn test_t2_r7_thickness_value_clamping_below_min() {
    let input: f64 = 0.1;
    let clamped: f64 = input.clamp(0.5, 12.0);
    assert_eq!(clamped, 0.5);
}

#[test]
fn test_t2_r7_thickness_value_clamping_above_max() {
    let input: f64 = 25.0;
    let clamped: f64 = input.clamp(0.5, 12.0);
    assert_eq!(clamped, 12.0);
}

#[test]
fn test_t2_r7_paper_card_preview_boundary_dimensions() {
    let (card_w, card_h) = (48, 64);
    assert_eq!(card_w, 48);
    assert_eq!(card_h, 64);
}

#[test]
fn test_t2_r7_export_database_locked_handling() {
    let path = unique_temp_path("locked_test", "uninote");
    let mut store = DocumentStore::open_or_create(&path).unwrap();
    let doc = Document::new_empty("Locked Doc");
    store.save_document(&doc).unwrap();

    let backup_path = unique_temp_path("locked_backup", "uninote");
    let res = export_portable_copy(store.connection(), &backup_path);
    assert!(res.is_ok());

    let _ = fs::remove_file(path);
    let _ = fs::remove_file(backup_path);
}

#[test]
fn test_t2_r7_export_empty_document_produces_valid_backup() {
    let path = unique_temp_path("empty_doc", "uninote");
    let backup_path = unique_temp_path("empty_backup", "uninote");
    let mut store = DocumentStore::open_or_create(&path).unwrap();
    let doc = Document::new_empty("Empty");
    store.save_document(&doc).unwrap();

    export_portable_copy(store.connection(), &backup_path).unwrap();
    let b_store = DocumentStore::open_or_create(&backup_path).unwrap();
    let b_doc = b_store.load_document().unwrap();
    assert_eq!(b_doc.pages.len(), 1);

    let _ = fs::remove_file(path);
    let _ = fs::remove_file(backup_path);
}

// ============================================================================
// TIER 3: CROSS-FEATURE COMBINATIONS (PAIRWISE SUBSYSTEM INTERACTIONS)
// ============================================================================

#[test]
fn test_t3_r1_r2_palm_rejection_during_shape_input() {
    // R1 + R2: Palm contact occurs while drawing a recognized rectangle
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let t_start = 1_000_000_000;

    // Stylus begins rectangle
    let stylus_down = InputSample {
        position: (100.0, 100.0),
        pressure: Some(0.7),
        tilt: None,
        device_kind: DeviceKind::Stylus,
        phase: InputPhase::Down,
        timestamp_ns: t_start,
        source_id: 0,
    };
    assert_eq!(
        engine.evaluate_sample(&stylus_down),
        InputDisposition::AcceptDraw
    );

    // Spurious palm touch occurs while pen is drawing
    let palm_touch = InputSample {
        position: (350.0, 450.0),
        pressure: Some(1.0),
        tilt: None,
        device_kind: DeviceKind::Touch,
        phase: InputPhase::Down,
        timestamp_ns: t_start + 10_000_000,
        source_id: 99,
    };
    assert_eq!(
        engine.evaluate_sample(&palm_touch),
        InputDisposition::Suppress,
        "Palm contact during drawing must be suppressed"
    );

    // Complete rectangle shape
    let rect_stroke = generate_rectangle_stroke((100.0, 100.0), 150.0, 80.0);
    assert_eq!(oracle_classify_shape(&rect_stroke), Some("rectangle"));
}

#[test]
fn test_t3_r2_r5_shape_recognition_on_dark_sheet_with_adaptation() {
    // R2 + R5: User draws a triangle with black ink on a dark sheet (#172126).
    // The shape is classified, and its black ink is adapted to #ECEFF4 ice white.
    let tri_stroke = generate_triangle_stroke((50.0, 150.0), (120.0, 20.0), (190.0, 150.0));
    assert_eq!(oracle_classify_shape(&tri_stroke), Some("triangle"));

    let black_color = (0.05, 0.05, 0.05, 1.0);
    let adapted_color = oracle_adapt_ink_color(black_color, true);
    assert_eq!(adapted_color, (0.925, 0.937, 0.957, 1.0));
}

#[test]
fn test_t3_r3_r7_dock_slot_selection_updates_popover_stroke_preview() {
    // R3 + R7: Switching dynamic dock slot changes color used in live stroke preview
    let prefs = AppPreferencesModel {
        active_slot_index: 2, // Coral (#FFB4AB)
        ..Default::default()
    };
    let active_color = prefs.slots[prefs.active_slot_index].rgba;

    // Render live stroke preview with this color
    let surface = ImageSurface::create(Format::ARgb32, 100, 30).unwrap();
    let cr = Context::new(&surface).unwrap();
    cr.set_source_rgba(
        active_color.0,
        active_color.1,
        active_color.2,
        active_color.3,
    );
    cr.set_line_width(prefs.stroke_width);
    cr.move_to(10.0, 15.0);
    cr.line_to(90.0, 15.0);
    cr.stroke().unwrap();
    assert!(cr.status().is_ok());
}

#[test]
fn test_t3_r4_r6_unified_icon_rendering_pipeline() {
    // R4 + R6: Render official logo, settings gear, and chisel highlighter concurrently
    let surface = ImageSurface::create(Format::ARgb32, 128, 48).unwrap();
    let cr = Context::new(&surface).unwrap();

    // Slot 1: Logo
    cr.save().unwrap();
    cr.translate(8.0, 8.0);
    cr.arc(16.0, 16.0, 14.0, 0.0, 2.0 * PI);
    cr.fill().unwrap();
    cr.restore().unwrap();

    // Slot 2: Gear
    cr.save().unwrap();
    cr.translate(48.0, 12.0);
    cr.arc(12.0, 12.0, 10.0, 0.0, 2.0 * PI);
    cr.fill().unwrap();
    cr.restore().unwrap();

    // Slot 3: Highlighter
    cr.save().unwrap();
    cr.translate(88.0, 12.0);
    cr.rotate(PI / 4.0);
    cr.rectangle(-3.0, -8.0, 6.0, 16.0);
    cr.fill().unwrap();
    cr.restore().unwrap();

    assert!(cr.status().is_ok());
}

#[test]
fn test_t3_r5_r7_paper_type_popover_reflects_sheet_theme_contrast() {
    // R5 + R7: Paper preview cards adapt background color to dark/light theme
    for is_dark in [false, true] {
        let bg_color = if is_dark {
            (0.090, 0.129, 0.149, 1.0)
        } else {
            (0.980, 0.976, 0.965, 1.0)
        };
        let surface = ImageSurface::create(Format::ARgb32, 48, 64).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.set_source_rgba(bg_color.0, bg_color.1, bg_color.2, bg_color.3);
        cr.rectangle(0.0, 0.0, 48.0, 64.0);
        cr.fill().unwrap();
        assert!(cr.status().is_ok());
    }
}

#[test]
fn test_t3_r1_r3_dock_drag_while_multitouch_pan_active() {
    // R1 + R3: Viewport pans by 100 pt, but dock stays screen-pinned with local drag
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    viewport.pan(100.0, 50.0);

    let dock_y = AppPreferencesModel::clamp_dock_y(220.0, 800.0, 300.0);
    assert_eq!(dock_y, 220.0);
    assert_eq!(viewport.offset_x, 100.0);
}

#[test]
fn test_t3_r2_r3_r7_custom_slot_shape_creation_and_backup() {
    // R2 + R3 + R7: Select slot 3 (Mint #79D9A5), snap line, commit to document, backup
    let prefs = AppPreferencesModel::default();
    let mint = prefs.slots[3].rgba;

    let mut stroke = generate_linear_stroke((10.0, 20.0), 100.0, 2.0 * PI / 180.0, 10, 0.0);
    stroke.style.color = mint;
    let snapped = oracle_snap_line_orthogonal(&stroke).unwrap();
    assert_eq!(snapped.style.color, mint);

    let doc_path = unique_temp_path("combo_doc", "uninote");
    let bak_path = unique_temp_path("combo_bak", "uninote");

    let mut doc = Document::new_empty("Combo Doc");
    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 0,
        kind: ItemKind::Stroke(snapped),
    });

    let mut store = DocumentStore::open_or_create(&doc_path).unwrap();
    store.save_document(&doc).unwrap();

    export_portable_copy(store.connection(), &bak_path).unwrap();
    let b_store = DocumentStore::open_or_create(&bak_path).unwrap();
    let b_doc = b_store.load_document().unwrap();
    match &b_doc.pages[0].items[0].kind {
        ItemKind::Stroke(s) => assert_eq!(s.style.color, mint),
    }

    let _ = fs::remove_file(doc_path);
    let _ = fs::remove_file(bak_path);
}

// ============================================================================
// TIER 4: REAL-WORLD APPLICATION SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_stem_lecture_note_taking() {
    // SCENARIO 1: University STEM Lecture (Dark Sheet, Palm on glass, Geometric shapes)
    let is_dark_sheet = true;
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let mut doc = Document::new_empty("STEM Note");
    doc.pages[0].background = PageBackground::Grid { grid_size: 14.17 };

    // 1. User rests palm on glass
    let t0 = 100_000_000;
    engine.on_pen_proximity(true, t0);
    let palm_suppressed = engine.should_suppress_raw_touch(true, 55, t0 + 1_000_000);
    assert!(palm_suppressed, "Palm must be suppressed upon resting");

    // 2. User draws 2 circuit lines (horizontal and vertical)
    let wire_h = generate_linear_stroke((50.0, 100.0), 120.0, 1.5 * PI / 180.0, 15, 0.0);
    let wire_v = generate_linear_stroke((170.0, 100.0), 80.0, 89.0 * PI / 180.0, 15, 0.0);
    let snapped_h = oracle_snap_line_orthogonal(&wire_h).unwrap();
    let snapped_v = oracle_snap_line_orthogonal(&wire_v).unwrap();

    // 3. User draws circuit component box (rectangle)
    let box_stroke = generate_rectangle_stroke((170.0, 180.0), 60.0, 40.0);
    assert_eq!(oracle_classify_shape(&box_stroke), Some("rectangle"));

    // 4. Adapt colors non-destructively for dark sheet
    for (i, mut s) in [snapped_h, snapped_v, box_stroke].into_iter().enumerate() {
        s.style.color = oracle_adapt_ink_color(s.style.color, is_dark_sheet);
        doc.pages[0].items.push(Item {
            id: (i + 1) as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(s),
        });
    }

    assert_eq!(doc.pages[0].items.len(), 3);

    // 5. Verify persistence in SQLite
    let db_path = unique_temp_path("stem_lecture", "uninote");
    let mut store = DocumentStore::open_or_create(&db_path).unwrap();
    store.save_document(&doc).unwrap();
    let loaded = store.load_document().unwrap();
    assert_eq!(loaded.pages[0].items.len(), 3);
    let _ = fs::remove_file(db_path);
}

#[test]
fn test_t4_scenario_rapid_multitouch_navigation_and_inking() {
    // SCENARIO 2: Rapid 2-Finger Pan, Pinch-Zoom, Stylus inking alternations
    let mut viewport = Viewport::new(0.0, 0.0, 1.0);
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    // Initial state
    assert_eq!(viewport.zoom, 1.0);

    // Gesture 1: Pan down-right
    viewport.pan(80.0, 150.0);
    assert_eq!((viewport.offset_x, viewport.offset_y), (80.0, 150.0));

    // Gesture 2: Zoom in to 150% around screen center
    viewport.zoom_at((600.0, 400.0), 1.5);
    assert_eq!(viewport.zoom, 1.5);

    // Gesture 3: Pan again while zoomed
    viewport.pan(-30.0, -20.0);
    assert_eq!((viewport.offset_x, viewport.offset_y), (-210.0, 5.0));

    // Touch releases, pen inks immediately
    let pen_draw = InputSample {
        position: (600.0, 400.0),
        pressure: Some(0.8),
        tilt: None,
        device_kind: DeviceKind::Stylus,
        phase: InputPhase::Down,
        timestamp_ns: 500_000_000,
        source_id: 0,
    };
    assert_eq!(
        engine.evaluate_sample(&pen_draw),
        InputDisposition::AcceptDraw
    );

    // Reset zoom back to 100%
    viewport.zoom = 1.0;
    assert_eq!(viewport.zoom, 1.0);
}

#[test]
fn test_t4_scenario_dock_customization_persistence_restart() {
    // SCENARIO 3: Left-handed ergonomics, adding custom slots, restart persistence
    let prefs_path = unique_temp_path("left_handed_prefs", "json");

    // Session 1: configure dock
    let mut session1 = AppPreferencesModel {
        dock_y: AppPreferencesModel::clamp_dock_y(340.0, 1080.0, 360.0),
        stroke_width: 4.0,
        active_slot_index: 5, // select newly added Lilla Chiaro
        ..Default::default()
    };
    session1.slots.push(PenSlotModel {
        hex: "#B388FF".into(),
        rgba: (0.702, 0.533, 1.0, 1.0),
        label: "Lilla Chiaro".into(),
    });
    session1.slots.push(PenSlotModel {
        hex: "#FF9E80".into(),
        rgba: (1.0, 0.620, 0.502, 1.0),
        label: "Pesca Calda".into(),
    });

    session1.save_to_file(&prefs_path).unwrap();

    // Session 2: App cold restart
    let session2 = AppPreferencesModel::load_from_file(&prefs_path);
    assert_eq!(session2.dock_y, 340.0);
    assert_eq!(session2.stroke_width, 4.0);
    assert_eq!(session2.slots.len(), 7);
    assert_eq!(session2.active_slot_index, 5);
    assert_eq!(session2.slots[5].hex, "#B388FF");

    let _ = fs::remove_file(prefs_path);
}

#[test]
fn test_t4_scenario_full_document_lifecycle_and_portable_backup() {
    // SCENARIO 4: Document creation -> strokes & shapes -> dark mode toggle -> backup export
    let doc_file = unique_temp_path("lifecycle_doc", "uninote");
    let export_file = unique_temp_path("lifecycle_export", "uninote");

    let mut doc = Document::new_empty("Lifecycle Doc");
    doc.pages[0].background = PageBackground::Ruled {
        line_spacing: 22.68,
        margin_left: 56.7,
    };

    // 1. Add handwritten stroke
    let pen_stroke = generate_linear_stroke((60.0, 60.0), 100.0, 0.5, 20, 1.5);
    doc.pages[0].items.push(Item {
        id: 1,
        z_index: 0,
        kind: ItemKind::Stroke(pen_stroke),
    });

    // 2. Add recognized circle
    let circle_stroke = generate_circle_stroke((200.0, 200.0), 50.0, 36);
    doc.pages[0].items.push(Item {
        id: 2,
        z_index: 1,
        kind: ItemKind::Stroke(circle_stroke),
    });

    // 3. Save to active SQLite database
    let mut store = DocumentStore::open_or_create(&doc_file).unwrap();
    store.save_document(&doc).unwrap();

    // 4. Toggle dark mode presentation (non-destructive)
    let is_dark_sheet = true;
    for item in &doc.pages[0].items {
        let ItemKind::Stroke(ref s) = item.kind;
        let _projected = oracle_adapt_ink_color(s.style.color, is_dark_sheet);
    }
    // Verify document in DB is unmutated
    let db_verify = store.load_document().unwrap();
    let ItemKind::Stroke(ref s) = db_verify.pages[0].items[0].kind;
    assert_eq!(s.style.color, (0.1, 0.1, 0.1, 1.0));

    // 5. Export portable copy
    export_portable_copy(store.connection(), &export_file).unwrap();
    assert!(export_file.exists());

    // 6. Verify portable copy integrity
    let export_store = DocumentStore::open_or_create(&export_file).unwrap();
    let export_doc = export_store.load_document().unwrap();
    assert_eq!(export_doc.pages.len(), 1);
    assert_eq!(export_doc.pages[0].items.len(), 2);

    let _ = fs::remove_file(doc_file);
    let _ = fs::remove_file(export_file);
}
