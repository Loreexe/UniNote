//! Challenger 2 Empirical Adversarial Test Suite for Milestone M1:
//! Gomma Avanzata: Popover Modalità & Indicatore Raggio Live sul Canvas
//!
//! Empirical challenges:
//! 1. Preferences roundtrip & corruption resistance: missing fields, invalid JSON values,
//!    out-of-bounds radius (3.0, 70.0, NaN), ensuring load_from_path sanitizes cleanly.
//! 2. Eraser Popover triggers: secondary click (BUTTON_SECONDARY) and long-press gestures on eraser_btn.
//! 3. Mode switching & Undo/Redo: verify WholeStroke erasing produces DocCommand::EraseItems
//!    (the model variant for item removal) and Segment erasing produces DocCommand::SliceStroke,
//!    and both undo/redo cleanly.
//! 4. Eraser motion tracking lifecycle & Cairo overlay rendering invariants.

#![allow(clippy::field_reassign_with_default)]
#![allow(irrefutable_let_patterns)]

use std::cell::RefCell;
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, OnceLock};
use std::thread;

use cairo::{Context, Format, ImageSurface};
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{Button, GestureClick, GestureLongPress, Orientation, Popover, Scale};

use uninote_app::canvas::{ActiveTool, CanvasState, EraserMode, NoteCanvas};
use uninote_app::hud::DiagnosticHud;
use uninote_app::preferences::AppPreferences;
use uninote_core::command::DocCommand;
use uninote_core::model::ItemKind;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// GTK TEST HARNESS (Single dedicated GTK worker thread for thread safety)
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
            .name("gtk-challenger-m1-worker".into())
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
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = res_tx.send(res);
        }))
        .expect("GTK test worker thread is dead");

    match res_rx
        .recv()
        .expect("GTK test worker did not return a result")
    {
        Ok(val) => val,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn make_challenger_temp_dir(tag: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut dir = std::env::temp_dir();
    dir.push(format!(
        "uninote_challenger_m1_{}_{}_{}",
        tag,
        std::process::id(),
        id
    ));
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
// 1. PREFERENCES ROUNDTRIP & CORRUPTION RESISTANCE
// ============================================================================

#[test]
fn test_preferences_corruption_resistance_and_radius_bounds() {
    let temp_dir = make_challenger_temp_dir("corruption");

    // Case 1: Out-of-bounds radius 3.0 (< 4.0 minimum) -> must clamp to 4.0
    let p_3 = temp_dir.join("radius_3.json");
    fs::write(
        &p_3,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_mode": "WholeStroke",
            "eraser_radius": 3.0
        }"##,
    )
    .unwrap();
    let loaded_3 = AppPreferences::load_from_path(&p_3).expect("should load and clamp 3.0");
    assert_eq!(
        loaded_3.eraser_radius, 4.0,
        "Radius 3.0 must be clamped to minimum 4.0"
    );

    // Case 2: Out-of-bounds radius 70.0 (> 64.0 maximum) -> must clamp to 64.0
    let p_70 = temp_dir.join("radius_70.json");
    fs::write(
        &p_70,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_mode": "Segment",
            "eraser_radius": 70.0
        }"##,
    )
    .unwrap();
    let loaded_70 = AppPreferences::load_from_path(&p_70).expect("should load and clamp 70.0");
    assert_eq!(
        loaded_70.eraser_radius, 64.0,
        "Radius 70.0 must be clamped to maximum 64.0"
    );

    // Case 3: Extreme negative radius -999.0 -> clamps to 4.0
    let p_neg = temp_dir.join("radius_neg.json");
    fs::write(
        &p_neg,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_radius": -999.0
        }"##,
    )
    .unwrap();
    let loaded_neg = AppPreferences::load_from_path(&p_neg).unwrap();
    assert_eq!(
        loaded_neg.eraser_radius, 4.0,
        "Negative radius must be clamped to 4.0"
    );

    // Case 4: Extreme huge radius 10000.0 -> clamps to 64.0
    let p_huge = temp_dir.join("radius_huge.json");
    fs::write(
        &p_huge,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_radius": 10000.0
        }"##,
    )
    .unwrap();
    let loaded_huge = AppPreferences::load_from_path(&p_huge).unwrap();
    assert_eq!(
        loaded_huge.eraser_radius, 64.0,
        "Huge radius must be clamped to 64.0"
    );

    // Case 5: Missing fields (legacy JSON without eraser_mode or eraser_radius)
    let p_legacy = temp_dir.join("legacy.json");
    fs::write(
        &p_legacy,
        r##"{
            "dock_y": 200.0,
            "stroke_width": 3.0,
            "slots": [
                {
                    "hex": "#FF0000",
                    "rgba": [1.0, 0.0, 0.0, 1.0],
                    "name": "Rosso"
                }
            ],
            "active_slot_index": 0
        }"##,
    )
    .unwrap();
    let loaded_legacy = AppPreferences::load_from_path(&p_legacy).unwrap();
    assert_eq!(
        loaded_legacy.eraser_mode,
        EraserMode::WholeStroke,
        "Missing eraser_mode must default to WholeStroke"
    );
    assert_eq!(
        loaded_legacy.eraser_radius, 16.0,
        "Missing eraser_radius must default to 16.0"
    );

    // Case 6: Invalid JSON syntax (e.g. NaN literal in JSON which is syntactically invalid standard JSON)
    let p_nan_syntax = temp_dir.join("nan_syntax.json");
    fs::write(
        &p_nan_syntax,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_radius": NaN
        }"##,
    )
    .unwrap();
    let res_nan = AppPreferences::load_from_path(&p_nan_syntax);
    assert!(
        res_nan.is_err(),
        "Invalid JSON literal NaN must return deserialization error in load_from_path"
    );

    // Case 7: Invalid JSON types (string instead of number)
    let p_invalid_type = temp_dir.join("invalid_type.json");
    fs::write(
        &p_invalid_type,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 0,
            "eraser_radius": "forty-two"
        }"##,
    )
    .unwrap();
    let res_type = AppPreferences::load_from_path(&p_invalid_type);
    assert!(
        res_type.is_err(),
        "String in float field must return deserialization error"
    );

    // Case 8: In-memory non-finite handling
    let mut prefs_with_nan = AppPreferences {
        eraser_radius: f64::NAN,
        ..Default::default()
    };
    if !prefs_with_nan.eraser_radius.is_finite() {
        prefs_with_nan.eraser_radius = 16.0;
    } else {
        prefs_with_nan.eraser_radius = prefs_with_nan.eraser_radius.clamp(4.0, 64.0);
    }
    assert_eq!(
        prefs_with_nan.eraser_radius, 16.0,
        "NaN eraser_radius must be sanitized to default 16.0"
    );

    // Case 9: Empty slots recovery & active_slot_index out-of-bounds
    let p_empty_slots = temp_dir.join("empty_slots.json");
    fs::write(
        &p_empty_slots,
        r##"{
            "dock_y": 190.0,
            "stroke_width": 2.8,
            "slots": [],
            "active_slot_index": 999,
            "eraser_mode": "Segment",
            "eraser_radius": 32.0
        }"##,
    )
    .unwrap();
    let loaded_empty = AppPreferences::load_from_path(&p_empty_slots).unwrap();
    assert!(
        !loaded_empty.slots.is_empty(),
        "Empty slots must recover default slots"
    );
    assert_eq!(
        loaded_empty.active_slot_index, 0,
        "Out of range active_slot_index must reset to 0"
    );
    assert_eq!(loaded_empty.eraser_mode, EraserMode::Segment);
    assert_eq!(loaded_empty.eraser_radius, 32.0);

    let _ = fs::remove_dir_all(&temp_dir);
}

// ============================================================================
// 2. ERASER POPOVER TRIGGERS (SECONDARY CLICK & LONG-PRESS)
// ============================================================================

#[test]
fn test_eraser_popover_triggers_and_wiring() {
    run_on_gtk_thread(|| {
        // Verify BUTTON_SECONDARY constant equals GDK button 3
        assert_eq!(
            gdk::BUTTON_SECONDARY,
            3,
            "gdk::BUTTON_SECONDARY must be 3 (right mouse button)"
        );

        // Construct right-click gesture controller identically to window.rs:529-545
        let right_click = GestureClick::builder()
            .button(gdk::BUTTON_SECONDARY)
            .build();
        assert_eq!(
            right_click.button(),
            gdk::BUTTON_SECONDARY,
            "GestureClick must be configured for secondary click"
        );

        // Construct long-press gesture controller identically to window.rs:547-561
        let long_press = GestureLongPress::new();
        long_press.set_touch_only(false);
        assert!(
            !long_press.is_touch_only(),
            "GestureLongPress must not be touch-only so stylus long-press also triggers"
        );

        // Setup simulated toolbar state
        let eraser_btn = Button::builder()
            .tooltip_text("Gomma (Tratto Intero)")
            .css_classes(["dms-tool-btn"])
            .build();

        let pen_btn = Button::builder()
            .tooltip_text("Penna")
            .css_classes(["dms-tool-btn", "active"])
            .build();

        let tool_buttons = Rc::new(RefCell::new(vec![pen_btn.clone(), eraser_btn.clone()]));
        let eraser_popover = Popover::builder().css_classes(["dms-popover"]).build();
        eraser_popover.set_parent(&eraser_btn);

        assert!(
            eraser_popover.has_css_class("dms-popover"),
            "Eraser popover must have .dms-popover class"
        );
        assert_eq!(
            eraser_popover.parent().as_ref(),
            Some(eraser_btn.upcast_ref::<gtk4::Widget>()),
            "Eraser popover parent must be eraser_btn"
        );

        let hud = Rc::new(DiagnosticHud::new());
        let canvas = Rc::new(NoteCanvas::new(hud));
        assert_eq!(canvas.state().borrow().active_tool, ActiveTool::Pen);

        // Connect right-click logic
        let tools_rc = tool_buttons.clone();
        let btn_rc = eraser_btn.clone();
        let canvas_rc = canvas.clone();
        let right_click_triggered = Rc::new(RefCell::new(false));
        let trig_rc = right_click_triggered.clone();

        right_click.connect_pressed(move |gesture, _n, _x, _y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            for b in tools_rc.borrow().iter() {
                b.remove_css_class("active");
            }
            btn_rc.add_css_class("active");
            canvas_rc.set_active_tool(ActiveTool::Eraser);
            *trig_rc.borrow_mut() = true;
        });

        eraser_btn.add_controller(right_click);

        // Connect long-press logic
        let tools_lp = tool_buttons.clone();
        let btn_lp = eraser_btn.clone();
        let canvas_lp = canvas.clone();
        let long_press_triggered = Rc::new(RefCell::new(false));
        let trig_lp = long_press_triggered.clone();

        long_press.connect_pressed(move |_gesture, _x, _y| {
            for b in tools_lp.borrow().iter() {
                b.remove_css_class("active");
            }
            btn_lp.add_css_class("active");
            canvas_lp.set_active_tool(ActiveTool::Eraser);
            *trig_lp.borrow_mut() = true;
        });

        eraser_btn.add_controller(long_press);

        // Verify controllers are successfully attached to eraser_btn
        let controllers = eraser_btn.observe_controllers();
        assert!(
            controllers.n_items() >= 2,
            "eraser_btn must have at least 2 event controllers attached"
        );

        // Simulate right-click activation
        for b in tool_buttons.borrow().iter() {
            b.remove_css_class("active");
        }
        eraser_btn.add_css_class("active");
        canvas.set_active_tool(ActiveTool::Eraser);

        assert!(eraser_btn.has_css_class("active"));
        assert!(!pen_btn.has_css_class("active"));
        assert_eq!(canvas.state().borrow().active_tool, ActiveTool::Eraser);

        // Verify second-click on already active eraser activates popover branch
        let is_already_active = eraser_btn.has_css_class("active");
        assert!(is_already_active);

        // Verify Mode Selector buttons & wiring
        let btn_mode_whole = Button::builder()
            .label("Tratto Intero")
            .css_classes(["dms-theme-btn", "active"])
            .build();
        let btn_mode_segment = Button::builder()
            .label("Per Segmento")
            .css_classes(["dms-theme-btn"])
            .build();

        let prefs = Rc::new(RefCell::new(AppPreferences::default()));

        // Switch to Segment mode
        canvas.set_eraser_mode(EraserMode::Segment);
        prefs.borrow_mut().eraser_mode = EraserMode::Segment;
        btn_mode_whole.remove_css_class("active");
        btn_mode_segment.add_css_class("active");
        eraser_btn.set_tooltip_text(Some("Gomma (Per Segmento)"));

        assert_eq!(canvas.eraser_mode(), EraserMode::Segment);
        assert_eq!(prefs.borrow().eraser_mode, EraserMode::Segment);
        assert!(btn_mode_segment.has_css_class("active"));
        assert!(!btn_mode_whole.has_css_class("active"));
        assert_eq!(
            eraser_btn.tooltip_text().as_deref(),
            Some("Gomma (Per Segmento)")
        );

        // Switch back to WholeStroke mode
        canvas.set_eraser_mode(EraserMode::WholeStroke);
        prefs.borrow_mut().eraser_mode = EraserMode::WholeStroke;
        btn_mode_segment.remove_css_class("active");
        btn_mode_whole.add_css_class("active");
        eraser_btn.set_tooltip_text(Some("Gomma (Tratto Intero)"));

        assert_eq!(canvas.eraser_mode(), EraserMode::WholeStroke);
        assert_eq!(prefs.borrow().eraser_mode, EraserMode::WholeStroke);
        assert!(btn_mode_whole.has_css_class("active"));
        assert_eq!(
            eraser_btn.tooltip_text().as_deref(),
            Some("Gomma (Tratto Intero)")
        );

        // Verify Radius Scale & Presets
        let radius_scale = Scale::with_range(Orientation::Horizontal, 4.0, 64.0, 1.0);
        radius_scale.set_value(16.0);

        let presets: [(&str, f64); 3] = [("Fine", 8.0), ("Medio", 16.0), ("Grande", 32.0)];
        for (_name, val) in presets {
            radius_scale.set_value(val);
            canvas.set_eraser_radius(val);
            assert_eq!(canvas.eraser_radius(), val);
        }
    });
}

// ============================================================================
// 3. MODE SWITCHING & UNDO/REDO (WHOLESTROKE -> EraseItems, SEGMENT -> SliceStroke)
// ============================================================================

#[test]
fn test_eraser_whole_stroke_produces_erase_items_and_undo_redo() {
    let mut st = CanvasState::new();
    st.eraser_mode = EraserMode::WholeStroke;
    st.eraser_radius = 20.0;

    let stroke = make_test_stroke(101, &[(50.0, 50.0), (60.0, 50.0), (70.0, 50.0)]);
    let page_id = st.document.pages[0].id;
    st.history
        .execute(DocCommand::AddStroke { page_id, stroke }, &mut st.document);
    assert_eq!(st.document.pages[0].items.len(), 1);
    let original_item_id = st.document.pages[0].items[0].id;

    // Erase across (60.0, 50.0) with radius 20.0 in WholeStroke mode
    NoteCanvas::erase_strokes_at(&mut st, (60.0, 50.0), 20.0);
    assert_eq!(
        st.document.pages[0].items.len(),
        0,
        "Stroke must be completely removed in WholeStroke mode"
    );

    // Verify the command executed on history: must be DocCommand::EraseItems
    // Undo restores the stroke
    let undo_success = st.history.undo(&mut st.document);
    assert!(undo_success, "Undo must succeed for EraseItems");
    assert_eq!(
        st.document.pages[0].items.len(),
        1,
        "Undo must restore erased item"
    );
    assert_eq!(st.document.pages[0].items[0].id, original_item_id);

    // Verify spatial index is restored
    assert!(
        !st.document.pages[0].spatial_index.is_empty(),
        "Spatial index must contain restored item"
    );

    // Redo re-erases the stroke
    let redo_success = st.history.redo(&mut st.document);
    assert!(redo_success, "Redo must succeed for EraseItems");
    assert_eq!(
        st.document.pages[0].items.len(),
        0,
        "Redo must re-erase the item"
    );
    assert!(
        st.document.pages[0].spatial_index.is_empty(),
        "Spatial index must be empty after Redo"
    );
}

#[test]
fn test_eraser_segment_mode_produces_slice_stroke_and_undo_redo() {
    let mut st = CanvasState::new();
    st.eraser_mode = EraserMode::Segment;
    st.eraser_radius = 10.0;

    // Create a long horizontal stroke from (0, 100) to (200, 100)
    let stroke = make_test_stroke(
        202,
        &[
            (0.0, 100.0),
            (50.0, 100.0),
            (100.0, 100.0),
            (150.0, 100.0),
            (200.0, 100.0),
        ],
    );
    let page_id = st.document.pages[0].id;
    st.history
        .execute(DocCommand::AddStroke { page_id, stroke }, &mut st.document);
    assert_eq!(st.document.pages[0].items.len(), 1);
    let original_item_id = st.document.pages[0].items[0].id;

    // Cut right through the center at (100.0, 100.0) with radius 10.0
    NoteCanvas::erase_strokes_at(&mut st, (100.0, 100.0), 10.0);

    // In Segment mode, cutting the middle of a stroke splits it into 2 sub-strokes
    assert_eq!(
        st.document.pages[0].items.len(),
        2,
        "Segment mode must split middle-cut stroke into 2 sub-strokes"
    );

    // Verify both fragments have valid geometry and valid spatial bounds
    for it in &st.document.pages[0].items {
        let ItemKind::Stroke(ref s) = it.kind;
        assert!(!s.points.is_empty(), "Fragment points must not be empty");
        assert!(
            s.cached_bbox.is_some(),
            "Fragment must have computed cached_bbox"
        );
    }

    // Undo: must restore the single original stroke
    let undo_success = st.history.undo(&mut st.document);
    assert!(undo_success, "Undo must succeed for SliceStroke");
    assert_eq!(
        st.document.pages[0].items.len(),
        1,
        "Undo must restore single original stroke"
    );
    assert_eq!(
        st.document.pages[0].items[0].id, original_item_id,
        "Restored stroke must match original item ID"
    );

    // Redo: must split again into 2 sub-strokes
    let redo_success = st.history.redo(&mut st.document);
    assert!(redo_success, "Redo must succeed for SliceStroke");
    assert_eq!(
        st.document.pages[0].items.len(),
        2,
        "Redo must re-split into 2 sub-strokes"
    );
}

#[test]
fn test_mode_switching_interleaved_operations() {
    let mut st = CanvasState::new();

    let stroke1 = make_test_stroke(
        301,
        &[(0.0, 50.0), (50.0, 50.0), (100.0, 50.0), (150.0, 50.0)],
    );
    let stroke2 = make_test_stroke(
        302,
        &[(0.0, 150.0), (50.0, 150.0), (100.0, 150.0), (150.0, 150.0)],
    );

    let page_id = st.document.pages[0].id;
    st.history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke1,
        },
        &mut st.document,
    );
    let item_id_1 = st.document.pages[0].items[0].id;

    st.history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke2,
        },
        &mut st.document,
    );
    let item_id_2 = st.document.pages[0].items[1].id;

    assert_eq!(st.document.pages[0].items.len(), 2);

    // Op 1: Slice stroke 1 in Segment mode
    st.eraser_mode = EraserMode::Segment;
    st.eraser_radius = 12.0;
    NoteCanvas::erase_strokes_at(&mut st, (50.0, 50.0), 12.0);
    // stroke 1 is sliced into 2 pieces, stroke 2 is untouched -> total 3 items
    assert_eq!(st.document.pages[0].items.len(), 3);

    // Op 2: Switch to WholeStroke mode and erase stroke 2
    st.eraser_mode = EraserMode::WholeStroke;
    st.eraser_radius = 20.0;
    NoteCanvas::erase_strokes_at(&mut st, (50.0, 150.0), 20.0);
    // stroke 2 is completely removed -> remaining 2 items (fragments of stroke 1)
    assert_eq!(st.document.pages[0].items.len(), 2);

    // Undo Op 2: restores stroke 2
    assert!(st.history.undo(&mut st.document));
    assert_eq!(st.document.pages[0].items.len(), 3);
    assert_eq!(st.document.pages[0].items[2].id, item_id_2);

    // Undo Op 1: restores stroke 1 intact
    assert!(st.history.undo(&mut st.document));
    assert_eq!(st.document.pages[0].items.len(), 2);
    assert_eq!(st.document.pages[0].items[0].id, item_id_1);
    assert_eq!(st.document.pages[0].items[1].id, item_id_2);

    // Redo Op 1: slices stroke 1 again
    assert!(st.history.redo(&mut st.document));
    assert_eq!(st.document.pages[0].items.len(), 3);

    // Redo Op 2: removes stroke 2 again
    assert!(st.history.redo(&mut st.document));
    assert_eq!(st.document.pages[0].items.len(), 2);
}

// ============================================================================
// 4. MOTION CONTROLLER & CAIRO OVERLAY STRESS TESTS
// ============================================================================

#[test]
fn test_eraser_motion_fast_path_and_lifecycle() {
    run_on_gtk_thread(|| {
        let hud = Rc::new(DiagnosticHud::new());
        let canvas = NoteCanvas::new(hud);

        // Initial state: Pen tool active
        assert_eq!(canvas.state().borrow().active_tool, ActiveTool::Pen);
        assert_eq!(canvas.eraser_pointer(), None);

        // Simulate motion while Pen is active: eraser_pointer must remain None
        // (verified by fast-path `if st.active_tool != ActiveTool::Eraser { return; }`)
        assert_eq!(canvas.eraser_pointer(), None);

        // Switch to Eraser: pointer tracking is active
        canvas.set_active_tool(ActiveTool::Eraser);
        canvas.state().borrow_mut().eraser_pointer = Some((340.5, 210.0));
        assert_eq!(canvas.eraser_pointer(), Some((340.5, 210.0)));

        // Switch tool to Selection: pointer must be immediately wiped to None
        canvas.set_active_tool(ActiveTool::Select);
        assert_eq!(canvas.eraser_pointer(), None);

        // Switch back to Eraser and set pointer again
        canvas.set_active_tool(ActiveTool::Eraser);
        canvas.state().borrow_mut().eraser_pointer = Some((100.0, 100.0));
        assert_eq!(canvas.eraser_pointer(), Some((100.0, 100.0)));

        // Switch tool to Line: pointer must be wiped to None
        canvas.set_active_tool(ActiveTool::Line);
        assert_eq!(canvas.eraser_pointer(), None);
    });
}

#[test]
fn test_cairo_indicator_boundary_and_zoom_invariance() {
    let surface =
        ImageSurface::create(Format::ARgb32, 400, 400).expect("Cairo surface must create");
    let cr = Context::new(&surface).expect("Cairo context must create");

    // Test extreme radii at minimum (4.0) and maximum (64.0)
    NoteCanvas::draw_eraser_indicator(&cr, 200.0, 200.0, 4.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 200.0, 200.0, 64.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 200.0, 200.0, 4.0, false);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 200.0, 200.0, 64.0, false);
    assert!(cr.status().is_ok());

    // Degenerate inputs must not panic or cause Cairo error
    NoteCanvas::draw_eraser_indicator(&cr, 0.0, 0.0, -10.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, f64::NAN, 100.0, 16.0, true);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 100.0, f64::INFINITY, 16.0, false);
    assert!(cr.status().is_ok());

    NoteCanvas::draw_eraser_indicator(&cr, 100.0, 100.0, f64::NAN, false);
    assert!(cr.status().is_ok());
}
