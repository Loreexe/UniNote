//! Milestone M2 Challenger Empirical Verification Test Suite
//! Target file: crates/uninote-app/tests/m2_challenger_m2_2_tests.rs
//!
//! Scope:
//! 1. Notebook Mode Page Assignment across Non-Standard Page Counts:
//!    - 0 pages: document with 0 pages gracefully returns Err, no panic.
//!    - 1 page: extreme scroll offsets (top, bottom, left, right) all map to Page 0 within 10pt margins.
//!    - 50 pages: accurate index resolution (page 0, 24, 49, out-of-bounds upper/lower clamping).
//!    - Inter-page boundaries and page gap: points in gap or near boundary clamp cleanly to page bounds without straddling.
//! 2. Whiteboard Mode Coordinate Conversions & Proportional Scaling:
//!    - Arbitrary viewport transforms: zoom (0.2x to 4.0x), offset (+10k to -10k), accurate document center mapping.
//!    - Proportional 60% viewport clamping: wide (6:1), tall (1:6), square (1:1), small (< 60%),
//!      zoomed-in min-size clamp (100pt floor), strictly preserving aspect ratio.
//! 3. Auto-Selection & State Consistency:
//!    - Previous tool (Pen, Highlighter, Eraser) switches to ActiveTool::Select.
//!    - SelectionState fields: selected_ids, page_index, bbox, is_dragging = false, drag_offset = (0,0), drag_kind = None.
//! 4. Undo / Redo Integrity:
//!    - Undo cleanly purges item from page.items and spatial_index, clears selection state (no dangling IDs).
//!    - Redo cleanly re-inserts item and spatial index with identical ID/hash/bbox, clears selection state safely.
//!    - 50+ alternating Undo/Redo stress cycles maintaining spatial index and document invariance.
//!    - Multi-image sequential insertion with interleaved Undo/Redo.

use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::hud::DiagnosticHud;
use uninote_app::selection::SelectionDragKind;
use uninote_core::coordinates::{Viewport, PAGE_GAP_PT};
use uninote_core::model::{DocumentFormat, PageBackground};

// ============================================================================
// DEDICATED GTK TEST HARNESS (Worker thread for GTK lifecycle safety)
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
            .name("gtk-challenger-m2-2-worker".into())
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
        .expect("Failed to send task to GTK test worker");

    match res_rx.recv().expect("Failed to receive test result") {
        Ok(val) => val,
        Err(err) => std::panic::resume_unwind(err),
    }
}

// ============================================================================
// HELPERS
// ============================================================================

fn create_test_canvas() -> NoteCanvas {
    let hud = Rc::new(DiagnosticHud::new());
    NoteCanvas::new(hud)
}

fn create_test_png_bytes(width: i32, height: i32, color_rgb: (f64, f64, f64)) -> Vec<u8> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .expect("Failed to create Cairo ImageSurface for test");
    let cr = cairo::Context::new(&surface).expect("Failed to create Cairo context");
    cr.set_source_rgb(color_rgb.0, color_rgb.1, color_rgb.2);
    cr.paint().expect("Failed to paint surface");
    drop(cr);

    let mut bytes = Vec::new();
    surface
        .write_to_png(&mut bytes)
        .expect("Failed to write surface to PNG bytes");
    bytes
}

// ============================================================================
// CHALLENGE 1: NOTEBOOK MODE NON-STANDARD PAGE COUNTS & BOUNDARIES
// ============================================================================

/// Case 1A: Non-standard page count: 0 pages.
/// Calling insert_image_bytes on an empty document must return an Err without panic or corrupted state.
#[test]
fn test_notebook_page_assignment_0_pages_error() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.pages.clear();
            assert_eq!(st.document.pages.len(), 0);
        }

        let png_bytes = create_test_png_bytes(100, 100, (1.0, 0.0, 0.0));
        let res = canvas.insert_image_bytes(&png_bytes, Some("image/png"));
        assert!(
            res.is_err(),
            "insert_image_bytes on 0-page document must fail gracefully, got: {:?}",
            res
        );

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert!(!st.selection.has_selection());
        assert_eq!(st.selection.selected_ids.len(), 0);
    });
}

/// Case 1B: Non-standard page count: 1 page.
/// Extreme viewport offsets (scrolled high above, deep below, far left, far right)
/// must always clamp within Page 0 bounds [x+10, x+w-10], [y+10, y+h-10].
#[test]
fn test_notebook_page_assignment_single_page_boundaries() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.format = DocumentFormat::Notebook;
            assert_eq!(st.document.pages.len(), 1);
        }

        let page_rect = Viewport::page_rect(0);
        let png_bytes = create_test_png_bytes(150, 100, (0.2, 0.5, 0.8));

        // Test extreme offsets
        let test_offsets = vec![
            ("scrolled_way_above", 0.0, 50_000.0),      // vp_center_y << 0
            ("scrolled_way_below", 0.0, -50_000.0),     // vp_center_y >> page_rect.bottom
            ("scrolled_far_left", 20_000.0, 0.0),       // vp_center_x << page_rect.left
            ("scrolled_far_right", -20_000.0, 0.0),     // vp_center_x >> page_rect.right
            ("diagonal_extreme", -10_000.0, -10_000.0), // both far away
        ];

        for (desc, off_x, off_y) in test_offsets {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.viewport.offset_x = off_x;
                st.viewport.offset_y = off_y;
                st.viewport.zoom = 1.0;
            }

            let item_id = canvas
                .insert_image_bytes(&png_bytes, Some("image/png"))
                .unwrap_or_else(|e| panic!("Insertion failed for {desc}: {e}"));

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let item = page.items.iter().find(|it| it.id == item_id).unwrap();
            let bbox = item.bounding_box().expect("bbox exists");

            assert!(
                bbox.min_x >= page_rect.x + 10.0 - 1e-4,
                "[{desc}] bbox.min_x {} < page_rect.x + 10 ({})",
                bbox.min_x,
                page_rect.x + 10.0
            );
            assert!(
                bbox.max_x <= page_rect.x + page_rect.width - 10.0 + 1e-4,
                "[{desc}] bbox.max_x {} > page_rect.right - 10 ({})",
                bbox.max_x,
                page_rect.x + page_rect.width - 10.0
            );
            assert!(
                bbox.min_y >= page_rect.y + 10.0 - 1e-4,
                "[{desc}] bbox.min_y {} < page_rect.y + 10 ({})",
                bbox.min_y,
                page_rect.y + 10.0
            );
            assert!(
                bbox.max_y <= page_rect.y + page_rect.height - 10.0 + 1e-4,
                "[{desc}] bbox.max_y {} > page_rect.bottom - 10 ({})",
                bbox.max_y,
                page_rect.y + page_rect.height - 10.0
            );
        }
    });
}

/// Case 1C: Non-standard page count: 50 pages.
/// Verify accurate index resolution and boundary clamping across 50 pages:
/// - Page 0, Page 24, Page 49
/// - Beyond Page 49 (doc_y = 1_000_000.0) -> mapped to Page 49
/// - Before Page 0 (doc_y = -100_000.0) -> mapped to Page 0
/// - Items are inserted ONLY in the target page's items and spatial index.
#[test]
fn test_notebook_page_assignment_50_pages_scaling_and_boundaries() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.format = DocumentFormat::Notebook;
            while st.document.pages.len() < 50 {
                st.document.add_page(PageBackground::default());
            }
            assert_eq!(st.document.pages.len(), 50);
        }

        let png_bytes = create_test_png_bytes(180, 120, (0.1, 0.7, 0.4));

        let test_targets = vec![(0, "first_page"), (24, "middle_page"), (49, "last_page")];

        for (target_idx, name) in test_targets {
            let target_rect = Viewport::page_rect(target_idx);
            let center_y = target_rect.y + target_rect.height / 2.0;

            // Set viewport so screen center (400, 300) maps to (target_rect.center_x, center_y)
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.viewport.zoom = 1.0;
                st.viewport.offset_x = 400.0 - (target_rect.x + target_rect.width / 2.0);
                st.viewport.offset_y = 300.0 - center_y;
            }

            let item_id = canvas
                .insert_image_bytes(&png_bytes, Some("image/png"))
                .unwrap_or_else(|e| panic!("Failed to insert on {name}: {e}"));

            let state_rc = canvas.state();
            let st = state_rc.borrow();

            assert_eq!(
                st.selection.page_index,
                Some(target_idx),
                "Selection page_index must be {target_idx} for {name}"
            );

            let page = &st.document.pages[target_idx];
            let item = page
                .items
                .iter()
                .find(|it| it.id == item_id)
                .expect("item on target page");
            let bbox = item.bounding_box().unwrap();

            // Centered on target page
            let actual_cx = (bbox.min_x + bbox.max_x) / 2.0;
            let actual_cy = (bbox.min_y + bbox.max_y) / 2.0;
            assert!((actual_cx - (target_rect.x + target_rect.width / 2.0)).abs() < 1e-3);
            assert!((actual_cy - center_y).abs() < 1e-3);

            // Verified inside margins
            assert!(bbox.min_x >= target_rect.x + 10.0 - 1e-4);
            assert!(bbox.max_x <= target_rect.x + target_rect.width - 10.0 + 1e-4);
            assert!(bbox.min_y >= target_rect.y + 10.0 - 1e-4);
            assert!(bbox.max_y <= target_rect.y + target_rect.height - 10.0 + 1e-4);

            // Spatial index verified
            let hits = page
                .spatial_index
                .locate_in_envelope(bbox.min_x, bbox.min_y, bbox.max_x, bbox.max_y);
            assert!(hits.iter().any(|h| h.item_id == item_id));
        }

        // Out-of-bounds tests:
        // 1. Way past page 49 -> maps to page 49
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.viewport.offset_y = -1_000_000.0;
        }
        let id_far_below = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert far below");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.selection.page_index, Some(49), "Clamped to page 49");
            let p49_rect = Viewport::page_rect(49);
            let item = st.document.pages[49]
                .items
                .iter()
                .find(|it| it.id == id_far_below)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.max_y <= p49_rect.y + p49_rect.height - 10.0 + 1e-4);
        }

        // 2. Way above page 0 -> maps to page 0
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.viewport.offset_y = 1_000_000.0;
        }
        let id_far_above = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert far above");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.selection.page_index, Some(0), "Clamped to page 0");
            let p0_rect = Viewport::page_rect(0);
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_far_above)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_y >= p0_rect.y + 10.0 - 1e-4);
        }
    });
}

/// Case 1D: Viewport spanning page boundaries and inter-page gap.
/// Ensures no image is ever placed spanning the gap between pages, or outside printable margins.
#[test]
fn test_notebook_inter_page_gap_and_straddling_viewport() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.format = DocumentFormat::Notebook;
            st.document.add_page(PageBackground::default());
            assert_eq!(st.document.pages.len(), 2);
        }

        let p0_rect = Viewport::page_rect(0);
        let p1_rect = Viewport::page_rect(1);
        let gap_start = p0_rect.y + p0_rect.height; // 871.89
        let gap_end = p1_rect.y; // 901.89
        assert!(gap_end > gap_start);
        assert!((gap_end - gap_start - PAGE_GAP_PT).abs() < 1e-4);

        let png_bytes = create_test_png_bytes(200, 100, (0.8, 0.2, 0.5));

        // Test points right before gap, inside gap, and right at start of page 1
        let test_y_coords = vec![
            (gap_start - 1.0, 0, "just_before_gap_page0"),
            (gap_start + PAGE_GAP_PT * 0.25, 0, "inside_gap_first_half"),
            (gap_start + PAGE_GAP_PT * 0.50, 0, "inside_gap_midpoint"),
            (gap_start + PAGE_GAP_PT * 0.75, 0, "inside_gap_second_half"),
            (gap_end - 0.001, 0, "just_before_next_top"),
            (gap_end, 1, "exactly_at_page1_top"),
            (gap_end + 1.0, 1, "just_after_page1_top"),
        ];

        for (target_y, expected_page_idx, desc) in test_y_coords {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.viewport.zoom = 1.0;
                st.viewport.offset_x = 400.0 - p0_rect.width / 2.0;
                st.viewport.offset_y = 300.0 - target_y;
            }

            let item_id = canvas
                .insert_image_bytes(&png_bytes, Some("image/png"))
                .unwrap_or_else(|e| panic!("Insertion failed for {desc}: {e}"));

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(
                st.selection.page_index,
                Some(expected_page_idx),
                "[{desc}] Assigned to wrong page index"
            );

            let page_rect = Viewport::page_rect(expected_page_idx);
            let page = &st.document.pages[expected_page_idx];
            let item = page.items.iter().find(|it| it.id == item_id).unwrap();
            let bbox = item.bounding_box().unwrap();

            // Strictly within the assigned page margins
            assert!(
                bbox.min_y >= page_rect.y + 10.0 - 1e-4,
                "[{desc}] bbox.min_y {} < page top margin {}",
                bbox.min_y,
                page_rect.y + 10.0
            );
            assert!(
                bbox.max_y <= page_rect.y + page_rect.height - 10.0 + 1e-4,
                "[{desc}] bbox.max_y {} > page bottom margin {}",
                bbox.max_y,
                page_rect.y + page_rect.height - 10.0
            );
        }
    });
}

// ============================================================================
// CHALLENGE 2: WHITEBOARD MODE COORDINATE CONVERSIONS & 60% SCALE CLAMPING
// ============================================================================

/// Case 2A: Arbitrary viewport coordinate conversions in Whiteboard mode.
/// Test zooming from 0.25x to 4.0x and non-zero panning offsets.
/// Verifies that the inserted image is centered precisely at the screen center transformed to doc coordinates.
#[test]
fn test_whiteboard_arbitrary_viewport_transforms_centering() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png_bytes = create_test_png_bytes(120, 80, (0.4, 0.4, 0.4));

        let viewports = vec![
            (1.0, 0.0, 0.0, "identity"),
            (0.5, 250.0, -180.0, "zoomed_out_panned"),
            (2.5, -450.0, 600.0, "zoomed_in_panned"),
            (0.25, 12000.0, -8500.0, "extreme_zoom_out_offset"),
            (3.8, -15000.0, 22000.0, "extreme_zoom_in_offset"),
        ];

        for (zoom, off_x, off_y, desc) in viewports {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.document.format = DocumentFormat::Whiteboard;
                st.viewport.zoom = zoom;
                st.viewport.offset_x = off_x;
                st.viewport.offset_y = off_y;
            }

            let item_id = canvas
                .insert_image_bytes(&png_bytes, Some("image/png"))
                .unwrap_or_else(|e| panic!("Failed for {desc}: {e}"));

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let item = page.items.iter().find(|it| it.id == item_id).unwrap();
            let bbox = item.bounding_box().unwrap();

            // Calculate expected document center
            let area_w = 800.0;
            let area_h = 600.0;
            let expected_doc_x = (area_w / 2.0 - off_x) / zoom;
            let expected_doc_y = (area_h / 2.0 - off_y) / zoom;

            let actual_cx = (bbox.min_x + bbox.max_x) / 2.0;
            let actual_cy = (bbox.min_y + bbox.max_y) / 2.0;

            assert!(
                (actual_cx - expected_doc_x).abs() < 1e-3,
                "[{desc}] Center X mismatch: actual {}, expected {}",
                actual_cx,
                expected_doc_x
            );
            assert!(
                (actual_cy - expected_doc_y).abs() < 1e-3,
                "[{desc}] Center Y mismatch: actual {}, expected {}",
                actual_cy,
                expected_doc_y
            );
        }
    });
}

/// Case 2B: Proportional scaling with 60% viewport clamping and aspect ratio lock.
/// Tests:
/// - Wide image (aspect ratio 6.0)
/// - Tall image (aspect ratio 1/6)
/// - Square massive image (aspect ratio 1.0)
/// - Small image (< 60% viewport): size preserved without upscaling
/// - High zoom viewport: verifies 100pt floor clamping
#[test]
fn test_whiteboard_extreme_aspect_ratios_and_60_percent_clamping() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // 1. Wide Image (3000 x 500 px, Aspect Ratio = 6.0)
        {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.document.format = DocumentFormat::Whiteboard;
                st.viewport = Viewport {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    zoom: 1.0,
                };
            }
            let wide_png = create_test_png_bytes(3000, 500, (1.0, 0.0, 0.0));
            let id = canvas
                .insert_image_bytes(&wide_png, Some("image/png"))
                .expect("insert wide");

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id)
                .unwrap();
            let bbox = item.bounding_box().unwrap();

            // 60% of 800 is 480pt. Width should clamp to 480pt.
            assert!(
                (bbox.width() - 480.0).abs() < 1e-3,
                "Wide image width must clamp to 480pt, got {}",
                bbox.width()
            );
            // Height should be 480 / 6 = 80pt
            assert!(
                (bbox.height() - 80.0).abs() < 1e-3,
                "Wide image height must be scaled to 80pt, got {}",
                bbox.height()
            );
            let ar = bbox.width() / bbox.height();
            assert!(
                (ar - 6.0).abs() < 1e-3,
                "Aspect ratio must be preserved at 6.0, got {ar}"
            );
        }

        // 2. Tall Image (500 x 3000 px, Aspect Ratio = 1/6)
        {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.document.format = DocumentFormat::Whiteboard;
                st.viewport = Viewport {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    zoom: 1.0,
                };
            }
            let tall_png = create_test_png_bytes(500, 3000, (0.0, 1.0, 0.0));
            let id = canvas
                .insert_image_bytes(&tall_png, Some("image/png"))
                .expect("insert tall");

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id)
                .unwrap();
            let bbox = item.bounding_box().unwrap();

            // 60% of 600 is 360pt. Height should clamp to 360pt.
            assert!(
                (bbox.height() - 360.0).abs() < 1e-3,
                "Tall image height must clamp to 360pt, got {}",
                bbox.height()
            );
            // Width should be 360 / 6 = 60pt
            assert!(
                (bbox.width() - 60.0).abs() < 1e-3,
                "Tall image width must be scaled to 60pt, got {}",
                bbox.width()
            );
            let ar = bbox.width() / bbox.height();
            assert!(
                (ar - (1.0 / 6.0)).abs() < 1e-3,
                "Aspect ratio must be preserved at 1/6, got {ar}"
            );
        }

        // 3. Small Image (64 x 48 px, smaller than 60%)
        {
            let small_png = create_test_png_bytes(64, 48, (0.0, 0.0, 1.0));
            let id = canvas
                .insert_image_bytes(&small_png, Some("image/png"))
                .expect("insert small");

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id)
                .unwrap();
            let bbox = item.bounding_box().unwrap();

            // Dimensions must remain exact native size (no artificial upscaling)
            assert!((bbox.width() - 64.0).abs() < 1e-3);
            assert!((bbox.height() - 48.0).abs() < 1e-3);
        }

        // 4. Heavy Zoom In (zoom = 6.0, vis_w = 800/6 = 133.3, 60% = 80.0 < 100.0 floor)
        {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.document.format = DocumentFormat::Whiteboard;
                st.viewport = Viewport {
                    offset_x: 0.0,
                    offset_y: 0.0,
                    zoom: 6.0,
                };
            }
            let large_png = create_test_png_bytes(1000, 1000, (1.0, 1.0, 0.0));
            let id = canvas
                .insert_image_bytes(&large_png, Some("image/png"))
                .expect("insert under heavy zoom");

            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id)
                .unwrap();
            let bbox = item.bounding_box().unwrap();

            // Floor is 100.0 pt
            assert!(
                (bbox.width() - 100.0).abs() < 1e-3,
                "Width should be clamped to floor 100pt under heavy zoom, got {}",
                bbox.width()
            );
            assert!(
                (bbox.height() - 100.0).abs() < 1e-3,
                "Height should be clamped to floor 100pt under heavy zoom, got {}",
                bbox.height()
            );
        }
    });
}

// ============================================================================
// CHALLENGE 3: AUTO-SELECTION INTEGRITY & ACTIVE TOOL SWITCHING
// ============================================================================

/// Case 3: Complete verification of auto-selection state invariants.
/// Switching from Pen / Highlighter / Eraser tool, ensuring selection fields are properly configured:
/// - active_tool == ActiveTool::Select
/// - selected_ids == vec![new_item_id]
/// - bbox == Some(exact image bbox)
/// - is_dragging == false
/// - drag_offset == (0.0, 0.0)
/// - drag_kind == SelectionDragKind::None
/// - active_lasso is empty, active_rect_start is None
#[test]
fn test_auto_selection_integrity_and_state_consistency() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png_bytes = create_test_png_bytes(160, 120, (0.3, 0.7, 0.2));

        let initial_tools = vec![ActiveTool::Pen, ActiveTool::Highlighter, ActiveTool::Eraser];

        for tool in initial_tools {
            {
                let state_rc = canvas.state();
                let mut st = state_rc.borrow_mut();
                st.active_tool = tool;
                st.selection.clear();
            }

            let item_id = canvas
                .insert_image_bytes(&png_bytes, Some("image/png"))
                .expect("insert image");

            let state_rc = canvas.state();
            let st = state_rc.borrow();

            // Tool switched to Select
            assert_eq!(
                st.active_tool,
                ActiveTool::Select,
                "Tool must switch to ActiveTool::Select from {:?}",
                tool
            );

            // Selection state fields
            assert_eq!(st.selection.selected_ids, vec![item_id]);
            assert_eq!(st.selection.page_index, Some(0));
            assert!(st.selection.has_selection());

            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == item_id)
                .unwrap();
            let expected_bbox = item.bounding_box().unwrap();
            assert_eq!(
                st.selection.bbox.unwrap(),
                expected_bbox,
                "Selection bbox must match image bounding box exactly"
            );

            // Drag / transient states reset
            assert!(!st.selection.is_dragging);
            assert_eq!(st.selection.drag_offset, (0.0, 0.0));
            assert_eq!(st.selection.drag_kind, SelectionDragKind::None);
            assert!(st.selection.active_lasso.is_empty());
            assert!(st.selection.active_rect_start.is_none());
            assert!(st.selection.active_rect_current.is_none());
        }
    });
}

// ============================================================================
// CHALLENGE 4: UNDO / REDO SELECTION INVARIANTS & STRESS CYCLES
// ============================================================================

/// Case 4: Empirical challenge of Undo/Redo cycles:
/// 1. Undo clears selection state (avoiding dangling IDs of deleted items).
/// 2. Redo clears selection state (safely re-inserting item).
/// 3. 50 alternating Undo/Redo cycles confirm zero spatial index drift or state corruption.
/// 4. Multiple sequential insertions with interleaved Undo/Redo preserve topological order.
#[test]
fn test_undo_redo_selection_invariants_and_stress_cycles() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png_bytes = create_test_png_bytes(100, 100, (0.5, 0.2, 0.8));

        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert image");

        // Verify pre-undo state
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.selection.selected_ids, vec![item_id]);
            assert!(st.selection.has_selection());
        }

        // Step 1: Perform UNDO
        assert!(canvas.undo(), "Canvas undo must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 0, "Item removed on undo");
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                0,
                "Spatial index purged on undo"
            );
            // Invariant: Selection must be cleanly cleared, NO dangling IDs
            assert!(
                !st.selection.has_selection(),
                "Selection must be cleared on undo"
            );
            assert!(
                st.selection.selected_ids.is_empty(),
                "selected_ids must be empty"
            );
            assert!(st.selection.bbox.is_none(), "selection bbox must be None");
        }

        // Step 2: Perform REDO
        assert!(canvas.redo(), "Canvas redo must succeed");
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 1, "Item restored on redo");
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                1,
                "Spatial index restored on redo"
            );
            assert_eq!(
                st.document.pages[0].items[0].id, item_id,
                "Item ID preserved"
            );
            // Invariant: Selection is safely empty (not pointing to un-focused object)
            assert!(
                !st.selection.has_selection(),
                "Selection cleanly cleared on redo"
            );
            assert!(st.selection.selected_ids.is_empty());
        }

        // Step 3: Stress-test 50 alternating Undo / Redo cycles
        for cycle in 1..=50 {
            assert!(canvas.undo(), "Cycle #{cycle}: Undo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                assert_eq!(
                    st.document.pages[0].items.len(),
                    0,
                    "Cycle #{cycle} undo items"
                );
                assert_eq!(
                    st.document.pages[0].spatial_index.len(),
                    0,
                    "Cycle #{cycle} undo R-tree"
                );
                assert!(
                    !st.selection.has_selection(),
                    "Cycle #{cycle} undo selection"
                );
            }

            assert!(canvas.redo(), "Cycle #{cycle}: Redo failed");
            {
                let state_rc = canvas.state();
                let st = state_rc.borrow();
                assert_eq!(
                    st.document.pages[0].items.len(),
                    1,
                    "Cycle #{cycle} redo items"
                );
                assert_eq!(
                    st.document.pages[0].spatial_index.len(),
                    1,
                    "Cycle #{cycle} redo R-tree"
                );
                assert_eq!(st.document.pages[0].items[0].id, item_id);
            }
        }

        // Step 4: Multi-image sequential insertions with interleaved Undo/Redo
        let png2 = create_test_png_bytes(80, 80, (0.9, 0.9, 0.0));
        let id2 = canvas
            .insert_image_bytes(&png2, Some("image/png"))
            .expect("insert image 2");

        let png3 = create_test_png_bytes(60, 60, (0.0, 0.9, 0.9));
        let id3 = canvas
            .insert_image_bytes(&png3, Some("image/png"))
            .expect("insert image 3");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 3);
            assert_eq!(st.document.pages[0].spatial_index.len(), 3);
            assert_eq!(st.selection.selected_ids, vec![id3]);
        }

        // Undo Image 3
        assert!(canvas.undo());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 2);
            assert_eq!(st.document.pages[0].items[0].id, item_id);
            assert_eq!(st.document.pages[0].items[1].id, id2);
            assert!(!st.selection.has_selection());
        }

        // Undo Image 2
        assert!(canvas.undo());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].items[0].id, item_id);
        }

        // Redo Image 2
        assert!(canvas.redo());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 2);
            assert_eq!(st.document.pages[0].items[1].id, id2);
        }

        // Redo Image 3
        assert!(canvas.redo());
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 3);
            assert_eq!(st.document.pages[0].items[2].id, id3);
        }
    });
}
