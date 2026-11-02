//! Milestone M2: Clipboard Input & Image Insertion Test Suite
//! Target file: crates/uninote-app/tests/m2_clipboard_tests.rs
//!
//! Scope:
//! 1. Unit tests for `NoteCanvas::insert_image_bytes`:
//!    - Valid PNG image bytes (dimensions, SHA-256 hash, aspect ratio, spatial index, undo/redo).
//!    - Valid JPEG image bytes (magic byte decoding, dimensions, SHA-256 hash).
//!    - Corrupted/invalid image bytes (empty, truncated headers, random garbage) gracefully returning Err.
//!    - Duplicate asset insertion (content-addressed SHA-256 deduplication in SQLite, distinct ItemIds).
//! 2. Viewport Placement & Smart Sizing:
//!    - Whiteboard mode: exact viewport centering and 60% viewport clamping for large images.
//!    - Notebook mode: page index resolution via `find_page_index_at_y`, margin clamping (10pt margin),
//!      and proportional scaling to prevent A4 page overflow.
//! 3. Auto-Selection & Tool State:
//!    - Automatic tool switch to `ActiveTool::Select`.
//!    - `SelectionState` populated with inserted `ItemId`, matching `bbox`, and `is_dragging = false`.
//! 4. SQLite Asset Persistence Verification:
//!    - Asset row saved to `assets` table with matching hash, MIME type, data bytes, and valid timestamp.
//!    - Document reloaded via `DocumentStore::load_document` preserves `ImageItem` and spatial indexing.
//! 5. Headless GTK lifecycle handling:
//!    - Dedicated GTK worker thread via `OnceLock<GtkTestHarness>` and `run_on_gtk_thread`.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use gtk4::glib;
use uninote_app::canvas::{ActiveTool, NoteCanvas};
use uninote_app::hud::DiagnosticHud;
use uninote_core::coordinates::Viewport;
use uninote_core::model::{DocumentFormat, ItemKind, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT};
use uninote_storage::DocumentStore;

// ============================================================================
// DEDICATED GTK TEST HARNESS (Ensures thread-safe GTK lifecycle across tests)
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
            .name("gtk-m2-clipboard-test-worker".into())
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
// TEST HELPERS
// ============================================================================

/// Helper per costruire un NoteCanvas su quaderno vuoto in memoria.
fn create_test_canvas() -> NoteCanvas {
    let hud = Rc::new(DiagnosticHud::new());
    NoteCanvas::new(hud)
}

/// Helper per generare un percorso file temporaneo univoco.
fn unique_test_db(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "uninote_m2_{}_{}_{}.uninote",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    path.push(unique);
    path
}

/// Helper per generare byte PNG validi di dimensioni arbitrarie tramite Cairo.
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

/// Helper che restituisce un payload binario JPEG valido deterministico (16x16 pixel).
fn sample_valid_jpeg_bytes() -> &'static [u8] {
    &[
        255, 216, 255, 224, 0, 16, 74, 70, 73, 70, 0, 1, 1, 0, 0, 1, 0, 1, 0, 0, 255, 219, 0, 67,
        0, 8, 6, 6, 7, 6, 5, 8, 7, 7, 7, 9, 9, 8, 10, 12, 20, 13, 12, 11, 11, 12, 25, 18, 19, 15,
        20, 29, 26, 31, 30, 29, 26, 28, 28, 32, 36, 46, 39, 32, 34, 44, 35, 28, 28, 40, 55, 41, 44,
        48, 49, 52, 52, 52, 31, 39, 57, 61, 56, 50, 60, 46, 51, 52, 50, 255, 219, 0, 67, 1, 9, 9,
        9, 12, 11, 12, 24, 13, 13, 24, 50, 33, 28, 33, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
        50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50,
        50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 50, 255, 192, 0, 17, 8, 0, 16,
        0, 16, 3, 1, 34, 0, 2, 17, 1, 3, 17, 1, 255, 196, 0, 31, 0, 0, 1, 5, 1, 1, 1, 1, 1, 1, 0,
        0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 255, 196, 0, 181, 16, 0, 2, 1, 3,
        3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 125, 1, 2, 3, 0, 4, 17, 5, 18, 33, 49, 65, 6, 19, 81, 97,
        7, 34, 113, 20, 50, 129, 145, 161, 8, 35, 66, 177, 193, 21, 82, 209, 240, 36, 51, 98, 114,
        130, 9, 10, 22, 23, 24, 25, 26, 37, 38, 39, 40, 41, 42, 52, 53, 54, 55, 56, 57, 58, 67, 68,
        69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102, 103, 104, 105,
        106, 115, 116, 117, 118, 119, 120, 121, 122, 131, 132, 133, 134, 135, 136, 137, 138, 146,
        147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167, 168, 169, 170, 178,
        179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199, 200, 201, 202, 210,
        211, 212, 213, 214, 215, 216, 217, 218, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234,
        241, 242, 243, 244, 245, 246, 247, 248, 249, 250, 255, 196, 0, 31, 1, 0, 3, 1, 1, 1, 1, 1,
        1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 255, 196, 0, 181, 17, 0,
        2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 119, 0, 1, 2, 3, 17, 4, 5, 33, 49, 6, 18, 65, 81,
        7, 97, 113, 19, 34, 50, 129, 8, 20, 66, 145, 161, 177, 193, 9, 35, 51, 82, 240, 21, 98,
        114, 209, 10, 22, 36, 52, 225, 37, 241, 23, 24, 25, 26, 38, 39, 40, 41, 42, 53, 54, 55, 56,
        57, 58, 67, 68, 69, 70, 71, 72, 73, 74, 83, 84, 85, 86, 87, 88, 89, 90, 99, 100, 101, 102,
        103, 104, 105, 106, 115, 116, 117, 118, 119, 120, 121, 122, 130, 131, 132, 133, 134, 135,
        136, 137, 138, 146, 147, 148, 149, 150, 151, 152, 153, 154, 162, 163, 164, 165, 166, 167,
        168, 169, 170, 178, 179, 180, 181, 182, 183, 184, 185, 186, 194, 195, 196, 197, 198, 199,
        200, 201, 202, 210, 211, 212, 213, 214, 215, 216, 217, 218, 226, 227, 228, 229, 230, 231,
        232, 233, 234, 242, 243, 244, 245, 246, 247, 248, 249, 250, 255, 218, 0, 12, 3, 1, 0, 2,
        17, 3, 17, 0, 63, 0, 226, 232, 162, 138, 249, 147, 247, 19, 255, 217,
    ]
}

// ============================================================================
// TEST 1: INSERT VALID PNG BYTES
// ============================================================================

#[test]
fn test_insert_valid_png_bytes() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png_bytes = create_test_png_bytes(240, 160, (0.2, 0.4, 0.8));
        let expected_hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &png_bytes)
            .expect("compute SHA-256")
            .to_string();

        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert_image_bytes with valid PNG should succeed");

        assert!(item_id > 0, "Item ID must be positive");

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        assert_eq!(st.document.pages.len(), 1, "Should have 1 page");
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 1, "Page should contain exactly 1 item");
        assert_eq!(
            page.spatial_index.len(),
            1,
            "Spatial index should contain 1 item"
        );

        let item = &page.items[0];
        assert_eq!(item.id, item_id);
        match &item.kind {
            ItemKind::Image(img) => {
                assert_eq!(
                    img.asset_hash, expected_hash,
                    "Asset hash must match SHA-256"
                );
                assert_eq!(img.original_width, 240, "Original width must match source");
                assert_eq!(
                    img.original_height, 160,
                    "Original height must match source"
                );
                assert_eq!(img.page_id, page.id, "Page ID must match owning page");
                assert!(
                    (img.aspect_ratio() - 1.5).abs() < 1e-4,
                    "Aspect ratio must be 240/160 = 1.5"
                );
                assert_eq!(img.z_index, 1, "z-index should be initialized");
            }
            _ => panic!("Inserted item must be ItemKind::Image"),
        }

        // Test spatial index query
        let img_bbox = item.bounding_box().expect("Bbox must exist");
        let hits = page.spatial_index.locate_in_envelope(
            img_bbox.min_x,
            img_bbox.min_y,
            img_bbox.max_x,
            img_bbox.max_y,
        );
        let hit_ids: Vec<u64> = hits.iter().map(|it| it.item_id).collect();
        assert_eq!(
            hit_ids,
            vec![item_id],
            "Spatial index query must hit inserted image"
        );
    });
}

// ============================================================================
// TEST 2: INSERT VALID JPEG BYTES WITH FORMAT AUTO-DETECTION
// ============================================================================

#[test]
fn test_insert_valid_jpeg_bytes_auto_detection() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let jpeg_bytes = sample_valid_jpeg_bytes();
        let expected_hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, jpeg_bytes)
            .expect("compute SHA-256")
            .to_string();

        // Pass None as mime_type to verify automatic format detection
        let item_id = canvas
            .insert_image_bytes(jpeg_bytes, None)
            .expect("insert_image_bytes with valid JPEG and None mime should succeed");

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 1);

        let item = &page.items[0];
        assert_eq!(item.id, item_id);
        match &item.kind {
            ItemKind::Image(img) => {
                assert_eq!(img.asset_hash, expected_hash);
                assert_eq!(img.original_width, 16);
                assert_eq!(img.original_height, 16);
                assert!((img.aspect_ratio() - 1.0).abs() < 1e-4);
            }
            _ => panic!("Expected ItemKind::Image"),
        }
    });
}

// ============================================================================
// TEST 3: CORRUPTED IMAGE BYTES REJECTED DEFENSIVELY
// ============================================================================

#[test]
fn test_insert_corrupted_image_bytes_graceful_error() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        let corrupted_cases: Vec<(&'static str, Vec<u8>)> = vec![
            ("empty_bytes", Vec::new()),
            (
                "plain_text",
                b"Hello, this is just plain text, not an image!".to_vec(),
            ),
            (
                "truncated_png",
                vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A],
            ),
            ("truncated_jpeg", vec![0xFF, 0xD8, 0xFF, 0xE0]),
            (
                "random_binary",
                vec![0x00, 0x01, 0x02, 0x03, 0xDE, 0xAD, 0xBE, 0xEF],
            ),
        ];

        for (desc, bytes) in corrupted_cases {
            let res = canvas.insert_image_bytes(&bytes, None);
            assert!(
                res.is_err(),
                "Corrupted case '{}' must return Err, got {:?}",
                desc,
                res
            );

            // Verify document and spatial index remain pristine
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert!(
                st.document.pages[0].items.is_empty(),
                "Case '{}' must not insert any item into page.items",
                desc
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                0,
                "Case '{}' must not insert into spatial index",
                desc
            );
            assert!(
                !st.selection.has_selection(),
                "Case '{}' must not alter selection",
                desc
            );
        }
    });
}

// ============================================================================
// TEST 4: DUPLICATE ASSET INSERTION DEDUPLICATION CHECK
// ============================================================================

#[test]
fn test_duplicate_asset_deduplication() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let db_path = unique_test_db("dedup");
        canvas
            .save_as(db_path.clone())
            .expect("save_as to init storage");

        let png_bytes = create_test_png_bytes(100, 100, (1.0, 0.0, 0.0));
        let expected_hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &png_bytes)
            .unwrap()
            .to_string();

        // 1. Insert image first time
        let id1 = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("first insertion");

        // 2. Insert the exact same image bytes a second time
        let id2 = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("second insertion");

        assert_ne!(
            id1, id2,
            "Each insertion must produce a distinct unique ItemId"
        );

        let state_rc = canvas.state();
        let st = state_rc.borrow();
        let page = &st.document.pages[0];
        assert_eq!(page.items.len(), 2, "Page must contain both image items");

        let item1 = page.items.iter().find(|it| it.id == id1).unwrap();
        let item2 = page.items.iter().find(|it| it.id == id2).unwrap();

        match (&item1.kind, &item2.kind) {
            (ItemKind::Image(img1), ItemKind::Image(img2)) => {
                assert_eq!(img1.asset_hash, expected_hash);
                assert_eq!(img2.asset_hash, expected_hash);
                assert_eq!(
                    img1.asset_hash, img2.asset_hash,
                    "Both items share identical hash"
                );
            }
            _ => panic!("Both items must be ItemKind::Image"),
        }
        drop(st);

        // 3. Verify SQLite assets table: exactly ONE row with this hash!
        let store = DocumentStore::open_or_create(&db_path).expect("open store");
        let count: i64 = store
            .connection()
            .query_row(
                "SELECT count(*) FROM assets WHERE hash = ?1;",
                [&expected_hash],
                |r| r.get(0),
            )
            .expect("query assets count");
        assert_eq!(
            count, 1,
            "SQLite assets table must contain exactly 1 row for duplicate hash (content-addressed dedup)"
        );

        let _ = fs::remove_file(db_path);
    });
}

// ============================================================================
// TEST 5: WHITEBOARD VIEWPORT CENTERING & 60% SCALE CLAMPING
// ============================================================================

#[test]
fn test_whiteboard_viewport_centering_and_scaling() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // Configure Whiteboard format and custom viewport
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

        // Subcase A: Small image (100x80 px), smaller than 60% of viewport
        let small_png = create_test_png_bytes(100, 80, (0.0, 1.0, 0.0));
        let id_small = canvas
            .insert_image_bytes(&small_png, Some("image/png"))
            .expect("insert small png");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let item = page.items.iter().find(|it| it.id == id_small).unwrap();
            let bbox = item.bounding_box().unwrap();

            // Native size preserved
            assert!(
                (bbox.width() - 100.0).abs() < 1e-3,
                "Small image width should be 100pt"
            );
            assert!(
                (bbox.height() - 80.0).abs() < 1e-3,
                "Small image height should be 80pt"
            );

            // Centered at viewport center: area default is min 800x600, center is (400, 300)
            let center_x = (bbox.min_x + bbox.max_x) / 2.0;
            let center_y = (bbox.min_y + bbox.max_y) / 2.0;
            assert!((center_x - 400.0).abs() < 1e-3, "Centered at vp X = 400");
            assert!((center_y - 300.0).abs() < 1e-3, "Centered at vp Y = 300");
        }

        // Subcase B: Huge image (2000x1000 px), exceeding 60% of viewport
        // Viewport visible size is 800 x 600 at zoom 1.0. 60% limits are 480 x 360.
        let huge_png = create_test_png_bytes(2000, 1000, (1.0, 0.5, 0.0));
        let id_huge = canvas
            .insert_image_bytes(&huge_png, Some("image/png"))
            .expect("insert huge png");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page = &st.document.pages[0];
            let item = page.items.iter().find(|it| it.id == id_huge).unwrap();
            let bbox = item.bounding_box().unwrap();

            assert!(
                bbox.width() <= 480.0 + 1e-3,
                "Huge image width {} must not exceed 60% viewport (480pt)",
                bbox.width()
            );
            assert!(
                bbox.height() <= 360.0 + 1e-3,
                "Huge image height {} must not exceed 60% viewport (360pt)",
                bbox.height()
            );

            // Proportional aspect ratio strictly maintained: 2000 / 1000 = 2.0
            let ratio = bbox.width() / bbox.height();
            assert!(
                (ratio - 2.0).abs() < 1e-3,
                "Aspect ratio must be preserved: got {}, expected 2.0",
                ratio
            );

            // Centered at viewport center (400, 300)
            let center_x = (bbox.min_x + bbox.max_x) / 2.0;
            let center_y = (bbox.min_y + bbox.max_y) / 2.0;
            assert!((center_x - 400.0).abs() < 1e-3);
            assert!((center_y - 300.0).abs() < 1e-3);
        }
    });
}

// ============================================================================
// TEST 6: NOTEBOOK PAGE RESOLUTION & MARGIN CLAMPING
// ============================================================================

#[test]
fn test_notebook_page_assignment_and_margin_clamping() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // 1. Setup Notebook with 3 pages
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.document.format = DocumentFormat::Notebook;
            st.document.add_page(PageBackground::default());
            st.document.add_page(PageBackground::default());
            assert_eq!(st.document.pages.len(), 3);
        }

        // Subcase A: Insertion onto Page 1 (scrolled down)
        // Page 0 Y: [30.0, 872.0], Page 1 Y: [902.0, 1744.0]
        // Target center at Y = 1200.0 (Page 1)
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.viewport = Viewport {
                offset_x: 400.0 - 297.5, // center horizontally on A4 (595 / 2 = 297.5)
                offset_y: -900.0,        // doc_y = (300 - (-900)) = 1200
                zoom: 1.0,
            };
        }

        let png_bytes = create_test_png_bytes(200, 150, (0.0, 0.0, 1.0));
        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert on page 1");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(
                st.selection.page_index,
                Some(1),
                "Selection page_index must be Page 1"
            );
            assert!(
                st.document.pages[0].items.is_empty(),
                "Page 0 must remain empty"
            );
            assert_eq!(
                st.document.pages[1].items.len(),
                1,
                "Page 1 must receive the image item"
            );
            assert!(
                st.document.pages[2].items.is_empty(),
                "Page 2 must remain empty"
            );

            let item = &st.document.pages[1].items[0];
            assert_eq!(item.id, item_id);
            let bbox = item.bounding_box().unwrap();
            let page1_rect = Viewport::page_rect(1);

            // Bbox must be strictly within page 1 boundaries including 10pt margin
            assert!(
                bbox.min_x >= page1_rect.x + 10.0,
                "bbox.min_x {} must be >= page_rect.x + 10 ({})",
                bbox.min_x,
                page1_rect.x + 10.0
            );
            assert!(
                bbox.max_x <= page1_rect.x + page1_rect.width - 10.0,
                "bbox.max_x {} must be <= page_rect.x + width - 10 ({})",
                bbox.max_x,
                page1_rect.x + page1_rect.width - 10.0
            );
            assert!(
                bbox.min_y >= page1_rect.y + 10.0,
                "bbox.min_y {} must be >= page1_rect.y + 10 ({})",
                bbox.min_y,
                page1_rect.y + 10.0
            );
            assert!(
                bbox.max_y <= page1_rect.y + page1_rect.height - 10.0,
                "bbox.max_y {} must be <= page_rect.y + height - 10 ({})",
                bbox.max_y,
                page1_rect.y + page1_rect.height - 10.0
            );
        }

        // Subcase B: Clamping when viewport center is near edge or off-page
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            // Scroll off to the left (X = -500) and top (Y = 10)
            st.viewport = Viewport {
                offset_x: 900.0, // doc_x = (400 - 900) = -500
                offset_y: 270.0, // doc_y = (300 - 270) = 30
                zoom: 1.0,
            };
        }

        let edge_png = create_test_png_bytes(100, 100, (1.0, 1.0, 0.0));
        let id_edge = canvas
            .insert_image_bytes(&edge_png, Some("image/png"))
            .expect("insert near edge");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let page0_rect = Viewport::page_rect(0);
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_edge)
                .unwrap();
            let bbox = item.bounding_box().unwrap();

            assert!(
                bbox.min_x >= page0_rect.x + 10.0,
                "Clamped X must not go off left edge"
            );
            assert!(
                bbox.min_y >= page0_rect.y + 10.0,
                "Clamped Y must not go off top edge"
            );
        }

        // Subcase C: Oversized image scaling inside A4 page limits
        let huge_a4_png = create_test_png_bytes(1200, 1600, (0.5, 0.5, 0.5));
        let id_huge_a4 = canvas
            .insert_image_bytes(&huge_a4_png, Some("image/png"))
            .expect("insert huge in notebook");

        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_huge_a4)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            let max_w = A4_WIDTH_PT - 20.0;
            let max_h = A4_HEIGHT_PT - 20.0;

            assert!(
                bbox.width() <= max_w + 1e-3,
                "Width {} must fit inside A4 margins ({})",
                bbox.width(),
                max_w
            );
            assert!(
                bbox.height() <= max_h + 1e-3,
                "Height {} must fit inside A4 margins ({})",
                bbox.height(),
                max_h
            );
            let ratio = bbox.width() / bbox.height();
            assert!((ratio - (1200.0 / 1600.0)).abs() < 1e-3, "Ratio preserved");
        }
    });
}

// ============================================================================
// TEST 7: AUTO-SELECTION & ACTIVE TOOL VERIFICATION
// ============================================================================

#[test]
fn test_auto_selection_and_active_tool_after_insertion() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // Initial state: tool is Pen, selection is empty
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            st.active_tool = ActiveTool::Pen;
            st.selection.clear();
        }

        let png_bytes = create_test_png_bytes(150, 100, (0.3, 0.6, 0.9));
        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert image");

        // Verify selection auto-activation
        let state_rc = canvas.state();
        let st = state_rc.borrow();

        assert_eq!(
            st.active_tool,
            ActiveTool::Select,
            "Active tool must automatically switch to ActiveTool::Select"
        );
        assert_eq!(
            st.selection.page_index,
            Some(0),
            "Selection page_index must be page 0"
        );
        assert_eq!(
            st.selection.selected_ids,
            vec![item_id],
            "Selected IDs must contain exactly the newly inserted item id"
        );
        assert!(st.selection.bbox.is_some(), "Selection bbox must be Some");

        let expected_bbox = st.document.pages[0].items[0]
            .bounding_box()
            .expect("image bbox");
        assert_eq!(
            st.selection.bbox.unwrap(),
            expected_bbox,
            "Selection bbox must match image bounding box exactly"
        );
        assert!(
            !st.selection.is_dragging,
            "Selection must not be in dragging state"
        );
        assert_eq!(
            st.selection.drag_offset,
            (0.0, 0.0),
            "Drag offset must be zero"
        );
    });
}

// ============================================================================
// TEST 8: SQLITE ASSET PERSISTENCE VERIFICATION
// ============================================================================

#[test]
fn test_sqlite_asset_persistence_verification() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let db_path = unique_test_db("sqlite_persist");

        // Attach SQLite database to canvas
        canvas
            .save_as(db_path.clone())
            .expect("save_as must succeed");

        let png_bytes = create_test_png_bytes(80, 60, (0.7, 0.1, 0.4));
        let expected_hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &png_bytes)
            .unwrap()
            .to_string();

        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert image");

        // 1. Verify directly in SQLite assets table
        let store = DocumentStore::open_or_create(&db_path).expect("open store");
        let (hash, mime_type, data): (String, String, Vec<u8>) = store
            .connection()
            .query_row(
                "SELECT hash, mime_type, data FROM assets WHERE hash = ?1;",
                [&expected_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .expect("Querying asset row from SQLite assets table must succeed");

        assert_eq!(
            hash, expected_hash,
            "Stored hash must match expected SHA-256"
        );
        assert_eq!(mime_type, "image/png", "MIME type must be image/png");
        assert_eq!(
            data, png_bytes,
            "Stored data BLOB must match inserted bytes byte-for-byte"
        );

        // Also test store.load_asset API
        let loaded_blob = store
            .load_asset(&expected_hash)
            .expect("load_asset")
            .expect("blob exists");
        assert_eq!(loaded_blob, png_bytes);

        // 2. Persist document and reload to ensure full document roundtrip
        canvas.save_as(db_path.clone()).expect("save_as with image");
        let store2 = DocumentStore::open_or_create(&db_path).expect("reopen store");
        let doc2 = store2.load_document().expect("load document with image");

        assert_eq!(doc2.pages[0].items.len(), 1);
        assert_eq!(doc2.pages[0].spatial_index.len(), 1);
        let item2 = &doc2.pages[0].items[0];
        assert_eq!(item2.id, item_id);
        match &item2.kind {
            ItemKind::Image(img2) => {
                assert_eq!(img2.asset_hash, expected_hash);
                assert_eq!(img2.original_width, 80);
                assert_eq!(img2.original_height, 60);
            }
            _ => panic!("Expected ItemKind::Image"),
        }

        let _ = fs::remove_file(db_path);
    });
}

// ============================================================================
// TEST 9: SYMMETRIC UNDO / REDO FIDELITY FOR IMAGE INSERTION
// ============================================================================

#[test]
fn test_undo_redo_image_insertion() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png_bytes = create_test_png_bytes(100, 100, (0.1, 0.9, 0.3));

        let item_id = canvas
            .insert_image_bytes(&png_bytes, Some("image/png"))
            .expect("insert image");

        // Verify presence before undo
        {
            let state_rc = canvas.state();
            let st = state_rc.borrow();
            assert_eq!(st.document.pages[0].items.len(), 1);
            assert_eq!(st.document.pages[0].spatial_index.len(), 1);
            assert_eq!(st.selection.selected_ids, vec![item_id]);
        }

        // Execute Undo
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let undone = st.undo();
            assert!(undone, "Undo must succeed");
            assert!(
                st.document.pages[0].items.is_empty(),
                "Page items must be empty after undo"
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                0,
                "Spatial index must be empty after undo"
            );
            assert!(
                !st.selection.has_selection(),
                "Selection must be cleared after undo"
            );
        }

        // Execute Redo
        {
            let state_rc = canvas.state();
            let mut st = state_rc.borrow_mut();
            let redone = st.redo();
            assert!(redone, "Redo must succeed");
            assert_eq!(
                st.document.pages[0].items.len(),
                1,
                "Page items must contain 1 item after redo"
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                1,
                "Spatial index must contain 1 item after redo"
            );
            assert_eq!(st.document.pages[0].items[0].id, item_id);
        }
    });
}

// ============================================================================
// TEST 10: CLIPBOARD PASTE FALLBACK CLEANLINESS
// ============================================================================

#[test]
fn test_paste_from_clipboard_fallback_to_image() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        // Clear internal stroke clipboard
        uninote_app::clipboard::clear_clipboard();

        // In headless testing without display or image on system clipboard,
        // paste_from_clipboard gracefully falls back to paste_image and returns false
        // without panicking or corrupting state.
        let pasted = canvas.paste_from_clipboard();
        assert!(
            !pasted,
            "Without clipboard data, paste_from_clipboard should return false cleanly"
        );
    });
}
