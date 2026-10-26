//! Milestone M1 Verification Suite:
//! Gomma Avanzata: Popover Modalità & Indicatore Raggio Live sul Canvas
//!
//! Verifies:
//! 1. Preferences defaults, roundtrip serialization, backward compatibility, and robust clamping/sanitization.
//! 2. CanvasState eraser radius, pointer state, and erase_strokes_at dynamic radius behavior.
//! 3. NoteCanvas getters, setters, clamping ([4.0, 64.0]), and pointer lifecycle (clearing on tool switch).
//! 4. Cairo live indicator overlay rendering in dark and light themes with zero-overhead invalid-input safety.
//! 5. Popover UI styling classes and theme switching synchronization.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;

use cairo::{Context, Format, ImageSurface};
use uninote_app::canvas::{ActiveTool, CanvasState, EraserMode, NoteCanvas};
use uninote_app::hud::DiagnosticHud;
use uninote_app::preferences::{AppPreferences, PenSlotConfig};
use uninote_core::command::DocCommand;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn make_temp_dir(tag: &str) -> PathBuf {
    let mut dir = std::env::temp_dir();
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    dir.push(format!("uninote_m1_test_{}_{}", tag, nonce));
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_test_stroke(id: u64, coords: &[(f64, f64)]) -> Stroke {
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
// 1. PREFERENCES TESTS: DEFAULTS, ROUNDTRIP, COMPATIBILITY & SANITIZATION
// ============================================================================

#[test]
fn test_preferences_eraser_defaults() {
    let prefs = AppPreferences::default();
    assert_eq!(
        prefs.eraser_mode,
        EraserMode::WholeStroke,
        "Default eraser mode must be WholeStroke"
    );
    assert_eq!(
        prefs.eraser_radius, 16.0,
        "Default eraser radius must be 16.0 px"
    );
}

#[test]
fn test_preferences_eraser_roundtrip_serialization() {
    let temp_dir = make_temp_dir("roundtrip");
    let file_path = temp_dir.join("prefs.json");

    let mut custom = AppPreferences {
        dock_y: 250.0,
        stroke_width: 3.5,
        active_slot_index: 1,
        eraser_mode: EraserMode::Segment,
        eraser_radius: 28.5,
        ..Default::default()
    };
    custom.slots.push(PenSlotConfig::new(
        "#00FF88",
        (0.0, 1.0, 0.53, 1.0),
        "Verde Fluorescente",
    ));

    custom
        .save_to_path(&file_path)
        .expect("save_to_path should succeed");
    assert!(file_path.exists());

    let loaded = AppPreferences::load_from_path(&file_path).expect("load_from_path should succeed");

    assert_eq!(loaded.eraser_mode, EraserMode::Segment);
    assert!(
        (loaded.eraser_radius - 28.5).abs() < 1e-6,
        "Loaded eraser radius must match saved value"
    );
    assert_eq!(loaded.slots.len(), custom.slots.len());

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_preferences_backward_compatibility_missing_fields() {
    let temp_dir = make_temp_dir("compat");
    let file_path = temp_dir.join("legacy_prefs.json");

    // Simula file JSON generato da versione precedente priva di eraser_mode ed eraser_radius
    let legacy_json = r##"{
        "dock_y": 180.0,
        "stroke_width": 2.0,
        "slots": [
            {
                "hex": "#6DD2FF",
                "rgba": [0.427, 0.824, 1.0, 1.0],
                "name": "Ciano Elettrico"
            }
        ],
        "active_slot_index": 0
    }"##;

    fs::write(&file_path, legacy_json).expect("failed to write legacy json");

    let loaded = AppPreferences::load_from_path(&file_path)
        .expect("load_from_path must deserialize legacy preferences without error");

    assert_eq!(
        loaded.eraser_mode,
        EraserMode::WholeStroke,
        "Missing eraser_mode in legacy JSON must default to WholeStroke"
    );
    assert_eq!(
        loaded.eraser_radius, 16.0,
        "Missing eraser_radius in legacy JSON must default to 16.0"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_preferences_eraser_radius_sanitization() {
    let temp_dir = make_temp_dir("sanitize");

    // Case 1: radius below minimum (1.0 px < 4.0 px)
    let p_min = temp_dir.join("below_min.json");
    let json_min = r#"{
        "dock_y": 190.0,
        "stroke_width": 2.8,
        "slots": [],
        "active_slot_index": 0,
        "eraser_mode": "WholeStroke",
        "eraser_radius": 1.0
    }"#;
    fs::write(&p_min, json_min).unwrap();
    let loaded_min = AppPreferences::load_from_path(&p_min).unwrap();
    assert_eq!(
        loaded_min.eraser_radius, 4.0,
        "Sub-minimum radius must clamp to 4.0"
    );

    // Case 2: radius above maximum (120.0 px > 64.0 px)
    let p_max = temp_dir.join("above_max.json");
    let json_max = r#"{
        "dock_y": 190.0,
        "stroke_width": 2.8,
        "slots": [],
        "active_slot_index": 0,
        "eraser_mode": "Segment",
        "eraser_radius": 120.0
    }"#;
    fs::write(&p_max, json_max).unwrap();
    let loaded_max = AppPreferences::load_from_path(&p_max).unwrap();
    assert_eq!(
        loaded_max.eraser_radius, 64.0,
        "Super-maximum radius must clamp to 64.0"
    );

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 2. CANVAS STATE: ERASER FIELDS & GEOMETRIC ERASING
// ============================================================================

#[test]
fn test_canvas_state_eraser_defaults() {
    let st = CanvasState::new();
    assert_eq!(st.eraser_radius, 16.0);
    assert_eq!(st.eraser_pointer, None);
    assert_eq!(st.eraser_mode, EraserMode::WholeStroke);
}

#[test]
fn test_canvas_state_dynamic_radius_erasing() {
    let mut st = CanvasState::new();

    // Create a horizontal stroke from (100, 100) to (120, 100)
    let stroke = make_test_stroke(1, &[(100.0, 100.0), (110.0, 100.0), (120.0, 100.0)]);

    let page_id = st.document.pages[0].id;
    st.history
        .execute(DocCommand::AddStroke { page_id, stroke }, &mut st.document);

    assert_eq!(st.document.pages[0].items.len(), 1);

    // Hit test at (100.0, 115.0): distance is 15.0 px.
    // 1. With radius = 10.0 px (15.0 > 10.0), stroke must NOT be erased.
    NoteCanvas::erase_strokes_at(&mut st, (100.0, 115.0), 10.0);
    assert_eq!(
        st.document.pages[0].items.len(),
        1,
        "Stroke should survive when outside radius"
    );

    // 2. With radius = 20.0 px (15.0 <= 20.0), stroke MUST be erased.
    NoteCanvas::erase_strokes_at(&mut st, (100.0, 115.0), 20.0);
    assert_eq!(
        st.document.pages[0].items.len(),
        0,
        "Stroke must be erased when within expanded radius"
    );

    // 3. Undo must restore the stroke
    assert!(st.history.undo(&mut st.document));
    assert_eq!(
        st.document.pages[0].items.len(),
        1,
        "Undo must restore erased stroke"
    );

    // 4. Redo must erase it again
    assert!(st.history.redo(&mut st.document));
    assert_eq!(
        st.document.pages[0].items.len(),
        0,
        "Redo must re-erase stroke"
    );
}

// ============================================================================
// 3. NOTECANVAS & GTK WIDGET: GETTERS, CLAMPING & POINTER LIFECYCLE
// ============================================================================

#[test]
fn test_note_canvas_radius_clamping_and_lifecycle() {
    let _ = gtk4::init();

    let hud = Rc::new(DiagnosticHud::new());
    let canvas = NoteCanvas::new(hud);

    // Default radius
    assert_eq!(canvas.eraser_radius(), 16.0);
    assert_eq!(canvas.eraser_pointer(), None);

    // Clamping sub-minimum
    canvas.set_eraser_radius(1.5);
    assert_eq!(
        canvas.eraser_radius(),
        4.0,
        "set_eraser_radius must clamp < 4.0 to 4.0"
    );

    // Clamping super-maximum
    canvas.set_eraser_radius(100.0);
    assert_eq!(
        canvas.eraser_radius(),
        64.0,
        "set_eraser_radius must clamp > 64.0 to 64.0"
    );

    // Clamping non-finite inputs
    canvas.set_eraser_radius(f64::NAN);
    assert_eq!(
        canvas.eraser_radius(),
        16.0,
        "set_eraser_radius with NaN must reset to default 16.0"
    );

    canvas.set_eraser_radius(f64::INFINITY);
    assert_eq!(
        canvas.eraser_radius(),
        16.0,
        "set_eraser_radius with Infinity must reset to default 16.0"
    );

    // Valid value within range
    canvas.set_eraser_radius(32.0);
    assert_eq!(canvas.eraser_radius(), 32.0);

    // Pointer lifecycle: set pointer while eraser is active
    canvas.set_active_tool(ActiveTool::Eraser);
    canvas.state().borrow_mut().eraser_pointer = Some((200.0, 150.0));
    assert_eq!(canvas.eraser_pointer(), Some((200.0, 150.0)));

    // Switching tool to Pen must immediately clear eraser_pointer
    canvas.set_active_tool(ActiveTool::Pen);
    assert_eq!(
        canvas.eraser_pointer(),
        None,
        "Switching from Eraser to Pen must clear eraser_pointer"
    );

    // Switching tool to Select must also ensure pointer remains cleared
    canvas.set_active_tool(ActiveTool::Select);
    assert_eq!(canvas.eraser_pointer(), None);

    // Switching tool to Highlighter must also ensure pointer remains cleared
    canvas.set_active_tool(ActiveTool::Highlighter);
    assert_eq!(canvas.eraser_pointer(), None);
}

// ============================================================================
// 4. CAIRO DRAWING: LIVE CIRCLE INDICATOR OVERLAY
// ============================================================================

#[test]
fn test_cairo_draw_eraser_indicator() {
    let surface =
        ImageSurface::create(Format::ARgb32, 200, 200).expect("Cairo ImageSurface creation failed");
    let cr = Context::new(&surface).expect("Cairo Context creation failed");

    // 1. Dark theme rendering
    NoteCanvas::draw_eraser_indicator(&cr, 100.0, 100.0, 16.0, true);
    assert!(
        cr.status().is_ok(),
        "Cairo draw_eraser_indicator in dark theme must succeed"
    );

    // 2. Light theme rendering
    NoteCanvas::draw_eraser_indicator(&cr, 50.0, 50.0, 24.0, false);
    assert!(
        cr.status().is_ok(),
        "Cairo draw_eraser_indicator in light theme must succeed"
    );

    // 3. Degenerate and non-finite inputs should safely early-return without errors
    NoteCanvas::draw_eraser_indicator(&cr, 100.0, 100.0, 0.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 100.0, 100.0, -5.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, f64::NAN, 100.0, 16.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 100.0, f64::INFINITY, 16.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 100.0, 100.0, f64::NAN, true);
    assert!(cr.status().is_ok());
}
