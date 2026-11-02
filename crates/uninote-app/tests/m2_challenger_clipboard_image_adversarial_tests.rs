//! Empirical Challenger Adversarial Test Suite for Milestone M2 (Clipboard Input & Image Insertion)
//! Target: crates/uninote-app/tests/m2_challenger_clipboard_image_adversarial_tests.rs
//!
//! Stress-tests:
//! 1. Extreme image dimensions (1x1 px, 16000x16000 px, ratios 1:100, 100:1).
//! 2. Corrupted bytes, fuzz payloads, random binary noise, truncated headers.
//! 3. High-concurrency / rapid-fire insertions and deduplication under load.
//! 4. Coordinate extremes: viewports at infinity, huge coordinates, zero zoom, extreme zoom (0.01x, 100x).
//! 5. Multi-page Notebook boundary mapping and margin clamping.
//! 6. ImageCacheManager LRU churn, capacity boundaries, and cache coherence.
//! 7. Clipboard synchronization thread-safety and fallback resilience.

use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;

use gtk4::glib;
use uninote_app::canvas::NoteCanvas;
use uninote_app::hud::DiagnosticHud;
use uninote_app::image_cache::ImageCacheManager;
use uninote_core::coordinates::Viewport;
use uninote_core::model::{DocumentFormat, ItemKind, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT};
use uninote_storage::DocumentStore;

// ============================================================================
// GTK TEST HARNESS (Ensures thread-safe GTK lifecycle across tests)
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
            .name("gtk-challenger-m2-worker".into())
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

fn create_test_canvas() -> NoteCanvas {
    let hud = Rc::new(DiagnosticHud::new());
    NoteCanvas::new(hud)
}

fn unique_test_db(prefix: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let unique = format!(
        "uninote_m2_challenger_{}_{}_{}.uninote",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    );
    path.push(unique);
    path
}

fn create_test_png(width: i32, height: i32, color_rgb: (f64, f64, f64)) -> Vec<u8> {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height)
        .expect("Failed to create Cairo surface");
    let cr = cairo::Context::new(&surface).expect("Failed to create Cairo context");
    cr.set_source_rgb(color_rgb.0, color_rgb.1, color_rgb.2);
    cr.paint().expect("Failed to paint surface");
    drop(cr);

    let mut bytes = Vec::new();
    surface
        .write_to_png(&mut bytes)
        .expect("Failed to encode PNG bytes");
    bytes
}

fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (-((crc & 1) as i32)) as u32;
            crc = (crc >> 1) ^ (0xEDB88320 & mask);
        }
    }
    !crc
}

// ============================================================================
// 1. EXTREME IMAGE DIMENSIONS: 1x1, 1:100, 100:1, and 16000x16000
// ============================================================================

#[test]
fn test_adv_extreme_dimensions_1x1_and_ratios() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // 1.1: 1x1 pixel PNG (boundary lower limit)
        let png_1x1 = create_test_png(1, 1, (1.0, 0.0, 0.0));
        let id_1x1 = canvas
            .insert_image_bytes(&png_1x1, Some("image/png"))
            .expect("1x1 PNG must be inserted successfully");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_1x1)
                .unwrap();
            if let ItemKind::Image(ref img) = item.kind {
                assert_eq!(img.original_width, 1);
                assert_eq!(img.original_height, 1);
                assert!((img.aspect_ratio() - 1.0).abs() < 1e-4);
                let bbox = item.bounding_box().unwrap();
                assert!((bbox.width() - 1.0).abs() < 1e-3);
                assert!((bbox.height() - 1.0).abs() < 1e-3);
            } else {
                panic!("Expected ImageItem");
            }
        }

        // 1.2: Extreme vertical ratio 1:100 (10x1000 px)
        let png_1_to_100 = create_test_png(10, 1000, (0.0, 1.0, 0.0));
        let id_v = canvas
            .insert_image_bytes(&png_1_to_100, Some("image/png"))
            .expect("1:100 PNG must be inserted successfully");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_v)
                .unwrap();
            if let ItemKind::Image(ref img) = item.kind {
                assert_eq!(img.original_width, 10);
                assert_eq!(img.original_height, 1000);
                let expected_ratio = 10.0 / 1000.0; // 0.01
                assert!((img.aspect_ratio() - expected_ratio).abs() < 1e-4);
                let bbox = item.bounding_box().unwrap();
                let actual_ratio = bbox.width() / bbox.height();
                assert!(
                    (actual_ratio - expected_ratio).abs() < 1e-3,
                    "Aspect ratio must be preserved after clamping: got {actual_ratio}, expected {expected_ratio}"
                );
                // Must fit within A4 limits
                assert!(bbox.width() <= A4_WIDTH_PT - 20.0 + 1e-3);
                assert!(bbox.height() <= A4_HEIGHT_PT - 20.0 + 1e-3);
            }
        }

        // 1.3: Extreme horizontal ratio 100:1 (1000x10 px)
        let png_100_to_1 = create_test_png(1000, 10, (0.0, 0.0, 1.0));
        let id_h = canvas
            .insert_image_bytes(&png_100_to_1, Some("image/png"))
            .expect("100:1 PNG must be inserted successfully");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_h)
                .unwrap();
            if let ItemKind::Image(ref img) = item.kind {
                assert_eq!(img.original_width, 1000);
                assert_eq!(img.original_height, 10);
                let expected_ratio = 1000.0 / 10.0; // 100.0
                assert!((img.aspect_ratio() - expected_ratio).abs() < 1e-4);
                let bbox = item.bounding_box().unwrap();
                let actual_ratio = bbox.width() / bbox.height();
                assert!(
                    (actual_ratio - expected_ratio).abs() < 1e-2,
                    "Aspect ratio must be preserved: got {actual_ratio}, expected {expected_ratio}"
                );
                assert!(bbox.width() <= A4_WIDTH_PT - 20.0 + 1e-3);
                assert!(bbox.height() <= A4_HEIGHT_PT - 20.0 + 1e-3);
            }
        }

        // 1.4: Extreme ratios in Whiteboard mode
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Whiteboard;
        }

        let id_wb_v = canvas
            .insert_image_bytes(&png_1_to_100, Some("image/png"))
            .expect("1:100 PNG on Whiteboard");
        let id_wb_h = canvas
            .insert_image_bytes(&png_100_to_1, Some("image/png"))
            .expect("100:1 PNG on Whiteboard");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item_v = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_wb_v)
                .unwrap();
            let item_h = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_wb_h)
                .unwrap();

            let bbox_v = item_v.bounding_box().unwrap();
            let bbox_h = item_h.bounding_box().unwrap();

            assert!((bbox_v.width() / bbox_v.height() - 0.01).abs() < 1e-3);
            assert!((bbox_h.width() / bbox_h.height() - 100.0).abs() < 1e-2);
        }
    });
}

#[test]
fn test_adv_extreme_dimension_16000x16000_or_graceful_rejection() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // Construct a synthetic PNG buffer with IHDR claiming 16000 x 16000 dimensions
        let mut png_16k = Vec::new();
        png_16k.extend_from_slice(b"\x89PNG\r\n\x1a\n");

        let mut ihdr_data = Vec::new();
        ihdr_data.extend_from_slice(&16000u32.to_be_bytes()); // width = 16000
        ihdr_data.extend_from_slice(&16000u32.to_be_bytes()); // height = 16000
        ihdr_data.push(8); // bit depth = 8
        ihdr_data.push(0); // color type = grayscale
        ihdr_data.push(0); // compression
        ihdr_data.push(0); // filter
        ihdr_data.push(0); // interlace

        let ihdr_len = (ihdr_data.len() as u32).to_be_bytes();
        png_16k.extend_from_slice(&ihdr_len);
        png_16k.extend_from_slice(b"IHDR");
        png_16k.extend_from_slice(&ihdr_data);

        let mut crc_buf = Vec::new();
        crc_buf.extend_from_slice(b"IHDR");
        crc_buf.extend_from_slice(&ihdr_data);
        let ihdr_crc = compute_crc32(&crc_buf);
        png_16k.extend_from_slice(&ihdr_crc.to_be_bytes());

        // IEND chunk
        png_16k.extend_from_slice(&0u32.to_be_bytes());
        png_16k.extend_from_slice(b"IEND");
        png_16k.extend_from_slice(&0xAE426082u32.to_be_bytes());

        // Attempt insertion of 16000x16000 synthetic PNG (missing IDAT)
        let res = canvas.insert_image_bytes(&png_16k, Some("image/png"));
        match res {
            Ok(item_id) => {
                let st_rc = canvas.state();
                let st = st_rc.borrow();
                let item = st.document.pages[0]
                    .items
                    .iter()
                    .find(|it| it.id == item_id)
                    .unwrap();
                let bbox = item.bounding_box().unwrap();
                assert!(bbox.width().is_finite());
                assert!(bbox.height().is_finite());
                assert!(bbox.width() <= A4_WIDTH_PT);
                assert!(bbox.height() <= A4_HEIGHT_PT);
            }
            Err(err) => {
                assert!(!err.is_empty(), "Error message must be non-empty");
            }
        }

        // Test a large renderable PNG: 4000x3000 px via Cairo
        let large_png = create_test_png(4000, 3000, (0.5, 0.2, 0.7));
        let id_large = canvas
            .insert_image_bytes(&large_png, Some("image/png"))
            .expect("4000x3000 PNG must insert successfully");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_large)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.width() <= A4_WIDTH_PT - 20.0 + 1e-3);
            assert!(bbox.height() <= A4_HEIGHT_PT - 20.0 + 1e-3);
            assert!((bbox.width() / bbox.height() - (4000.0 / 3000.0)).abs() < 1e-3);
        }
    });
}

// ============================================================================
// 2. CORRUPTED BYTES, FUZZ PAYLOADS, RANDOM BINARY NOISE, TRUNCATED HEADERS
// ============================================================================

#[test]
fn test_adv_fuzz_corrupted_truncated_and_malformed_bytes() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        let mut fuzz_cases: Vec<(&'static str, Vec<u8>)> = Vec::new();

        // 1. Empty and single byte buffers
        fuzz_cases.push(("empty", Vec::new()));
        fuzz_cases.push(("single_zero", vec![0x00]));
        fuzz_cases.push(("single_png_magic", vec![0x89]));
        fuzz_cases.push(("single_jpeg_magic", vec![0xFF]));
        fuzz_cases.push(("single_ascii", vec![0x41]));

        // 2. Truncated PNG headers
        fuzz_cases.push(("png_sig_2bytes", vec![0x89, b'P']));
        fuzz_cases.push(("png_sig_4bytes", vec![0x89, b'P', b'N', b'G']));
        fuzz_cases.push((
            "png_sig_7bytes",
            vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A],
        ));
        fuzz_cases.push(("png_sig_8bytes_no_chunks", b"\x89PNG\r\n\x1a\n".to_vec()));
        fuzz_cases.push(("png_sig_truncated_ihdr_len", {
            let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
            v.extend_from_slice(&[0x00, 0x00]);
            v
        }));
        fuzz_cases.push(("png_sig_corrupt_ihdr_tag", {
            let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D, b'X', b'X', b'X', b'X']);
            v
        }));

        // 3. PNG with zero dimensions in IHDR
        fuzz_cases.push(("png_zero_width", {
            let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
            v.extend_from_slice(b"IHDR");
            v.extend_from_slice(&0u32.to_be_bytes()); // w = 0
            v.extend_from_slice(&100u32.to_be_bytes()); // h = 100
            v.extend_from_slice(&[8, 2, 0, 0, 0]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // dummy crc
            v
        }));
        fuzz_cases.push(("png_zero_height", {
            let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x0D]);
            v.extend_from_slice(b"IHDR");
            v.extend_from_slice(&100u32.to_be_bytes()); // w = 100
            v.extend_from_slice(&0u32.to_be_bytes()); // h = 0
            v.extend_from_slice(&[8, 2, 0, 0, 0]);
            v.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // dummy crc
            v
        }));

        // 4. Truncated JPEG variants
        fuzz_cases.push(("jpeg_2bytes", vec![0xFF, 0xD8]));
        fuzz_cases.push(("jpeg_3bytes", vec![0xFF, 0xD8, 0xFF]));
        fuzz_cases.push(("jpeg_app0_truncated", vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00]));
        fuzz_cases.push((
            "jpeg_app0_no_jfif",
            vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, 0xAA, 0xBB],
        ));

        // 5. Truncated WebP variants
        fuzz_cases.push(("webp_riff_only", b"RIFF".to_vec()));
        fuzz_cases.push(("webp_riff_short", b"RIFF\x04\x00\x00\x00".to_vec()));
        fuzz_cases.push((
            "webp_riff_webp_no_vp8",
            b"RIFF\x0c\x00\x00\x00WEBP".to_vec(),
        ));

        // 6. Text, HTML, JSON, Unicode
        fuzz_cases.push((
            "html_doc",
            b"<!DOCTYPE html><html><body><h1>Not an image</h1></body></html>".to_vec(),
        ));
        fuzz_cases.push((
            "json_payload",
            b"{\"name\": \"image.png\", \"width\": 800}".to_vec(),
        ));
        fuzz_cases.push(("unicode_emojis", "🖼️📸🎨✨💥".as_bytes().to_vec()));
        fuzz_cases.push((
            "sql_injection_attempt",
            b"' OR 1=1; DROP TABLE assets; --".to_vec(),
        ));

        // 7. Uniform blocks of zeros and 0xFF
        fuzz_cases.push(("zeros_1k", vec![0x00; 1024]));
        fuzz_cases.push(("zeros_64k", vec![0x00; 65536]));
        fuzz_cases.push(("ff_1k", vec![0xFF; 1024]));
        fuzz_cases.push(("ff_64k", vec![0xFF; 65536]));

        // 8. Pseudorandom deterministic noise (PRNG)
        for seed in [13u64, 42, 1337, 99999] {
            let mut state = seed;
            let mut noise = Vec::with_capacity(512);
            for _ in 0..512 {
                state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                noise.push((state >> 33) as u8);
            }
            fuzz_cases.push(("prng_noise", noise));
        }

        for (desc, payload) in fuzz_cases {
            let res1 = canvas.insert_image_bytes(&payload, None);
            assert!(
                res1.is_err(),
                "Fuzz case '{desc}' with None mime must return Err, got Ok"
            );

            let res2 = canvas.insert_image_bytes(&payload, Some("image/png"));
            assert!(
                res2.is_err(),
                "Fuzz case '{desc}' with image/png mime must return Err, got Ok"
            );

            let res3 = canvas.insert_image_bytes(&payload, Some("image/jpeg"));
            assert!(
                res3.is_err(),
                "Fuzz case '{desc}' with image/jpeg mime must return Err, got Ok"
            );

            // Verify canvas remains completely clean and uncorrupted
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            assert!(
                st.document.pages[0].items.is_empty(),
                "Document must remain pristine after rejected fuzz case '{desc}'"
            );
            assert_eq!(
                st.document.pages[0].spatial_index.len(),
                0,
                "Spatial index must remain empty after rejected fuzz case '{desc}'"
            );
            assert!(
                !st.selection.has_selection(),
                "Selection must remain cleared after rejected fuzz case '{desc}'"
            );
        }
    });
}

// ============================================================================
// 3. HIGH-CONCURRENCY / RAPID-FIRE INSERTIONS & DEDUPLICATION UNDER LOAD
// ============================================================================

#[test]
fn test_adv_high_concurrency_rapid_fire_and_dedup_load() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let db_path = unique_test_db("stress_dedup");
        canvas
            .save_as(db_path.clone())
            .expect("save_as to attach db");

        const NUM_UNIQUE_IMAGES: usize = 30;
        const DUPLICATE_COPIES: usize = 3;
        const TOTAL_INSERTIONS: usize = NUM_UNIQUE_IMAGES * DUPLICATE_COPIES; // 90 insertions

        // Generate 30 distinct small PNG images
        let mut unique_pngs = Vec::with_capacity(NUM_UNIQUE_IMAGES);
        for i in 0..NUM_UNIQUE_IMAGES {
            let r = (i as f64 * 7.0) % 1.0;
            let g = (i as f64 * 13.0) % 1.0;
            let b = (i as f64 * 23.0) % 1.0;
            let w = 20 + (i as i32 % 30);
            let h = 20 + ((i as i32 * 3) % 30);
            let bytes = create_test_png(w, h, (r, g, b));
            let hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &bytes)
                .unwrap()
                .to_string();
            unique_pngs.push((bytes, hash));
        }

        let start_time = std::time::Instant::now();
        let mut inserted_ids = Vec::with_capacity(TOTAL_INSERTIONS);

        // Perform 90 rapid consecutive insertions in interleaved order
        for copy_idx in 0..DUPLICATE_COPIES {
            for (i, (bytes, _hash)) in unique_pngs.iter().enumerate() {
                let item_id = canvas
                    .insert_image_bytes(bytes, Some("image/png"))
                    .unwrap_or_else(|e| {
                        panic!("Insertion failed at copy {copy_idx}, image {i}: {e}")
                    });
                inserted_ids.push(item_id);
            }
        }

        let elapsed = start_time.elapsed();
        println!(
            "[Challenger M2-1] {TOTAL_INSERTIONS} rapid image insertions completed in {:.3}s ({:.2} ms/insert)",
            elapsed.as_secs_f64(),
            elapsed.as_secs_f64() * 1000.0 / TOTAL_INSERTIONS as f64
        );

        // Verify all 90 ItemIds are unique
        let mut sorted_ids = inserted_ids.clone();
        sorted_ids.sort();
        sorted_ids.dedup();
        assert_eq!(
            sorted_ids.len(),
            TOTAL_INSERTIONS,
            "Every insertion must yield a strictly distinct ItemId"
        );

        // Verify document state
        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let page = &st.document.pages[0];
            assert_eq!(page.items.len(), TOTAL_INSERTIONS);
            assert_eq!(page.spatial_index.len(), TOTAL_INSERTIONS);
            let z_indices: Vec<i64> = page.items.iter().map(|it| it.z_index as i64).collect();
            for w in z_indices.windows(2) {
                assert!(w[1] > w[0], "z_index must strictly increase");
            }
            assert_eq!(
                st.selection.selected_ids,
                vec![*inserted_ids.last().unwrap()],
                "Auto-selection must target the most recently inserted item"
            );
        }

        // Verify SQLite assets table: exactly NUM_UNIQUE_IMAGES rows
        let store = DocumentStore::open_or_create(&db_path).expect("open store");
        let distinct_asset_count: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0))
            .expect("query assets count");
        assert_eq!(
            distinct_asset_count as usize, NUM_UNIQUE_IMAGES,
            "SQLite assets table must contain exactly {NUM_UNIQUE_IMAGES} deduplicated rows, got {distinct_asset_count}"
        );

        // Verify all 90 items can be sequentially undone
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            for i in 0..TOTAL_INSERTIONS {
                let undone = st.undo();
                assert!(undone, "Undo must succeed at step {i}");
            }
            assert_eq!(st.document.pages[0].items.len(), 0);
            assert_eq!(st.document.pages[0].spatial_index.len(), 0);
            assert!(!st.selection.has_selection());

            // Redo all 90 items
            for i in 0..TOTAL_INSERTIONS {
                let redone = st.redo();
                assert!(redone, "Redo must succeed at step {i}");
            }
            assert_eq!(st.document.pages[0].items.len(), TOTAL_INSERTIONS);
            assert_eq!(st.document.pages[0].spatial_index.len(), TOTAL_INSERTIONS);
        }

        let _ = fs::remove_file(db_path);
    });
}

// ============================================================================
// 4. COORDINATE EXTREMES: INFINITY, NAN, HUGE OFFSETS, EXTREME ZOOM (0.01x, 100x)
// ============================================================================

#[test]
fn test_adv_coordinate_extremes_and_viewport_boundaries() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png = create_test_png(100, 100, (0.4, 0.4, 0.4));

        // 4.1: Viewport offset at positive and negative infinity
        let inf_offsets = [
            (f64::INFINITY, 0.0),
            (0.0, f64::INFINITY),
            (f64::NEG_INFINITY, 0.0),
            (0.0, f64::NEG_INFINITY),
            (f64::NAN, 0.0),
            (0.0, f64::NAN),
        ];

        for (ox, oy) in inf_offsets {
            {
                let st_rc = canvas.state();
                let mut st = st_rc.borrow_mut();
                st.viewport.offset_x = ox;
                st.viewport.offset_y = oy;
                st.viewport.zoom = 1.0;
            }
            let res = canvas.insert_image_bytes(&png, Some("image/png"));
            assert!(
                res.is_err(),
                "Non-finite viewport offset ({ox}, {oy}) must be safely rejected"
            );
        }

        // 4.2: Zoom at zero, NaN, or negative
        let invalid_zooms = [0.0, -1.0, -100.0, f64::NAN];
        for z in invalid_zooms {
            {
                let st_rc = canvas.state();
                let mut st = st_rc.borrow_mut();
                st.viewport.offset_x = 0.0;
                st.viewport.offset_y = 0.0;
                st.viewport.zoom = z;
            }
            let res = canvas.insert_image_bytes(&png, Some("image/png"));
            if let Ok(id) = res {
                let st_rc = canvas.state();
                let st = st_rc.borrow();
                let item = st.document.pages[0]
                    .items
                    .iter()
                    .find(|it| it.id == id)
                    .unwrap();
                let bbox = item.bounding_box().unwrap();
                assert!(bbox.min_x.is_finite());
                assert!(bbox.min_y.is_finite());
                assert!(bbox.max_x.is_finite());
                assert!(bbox.max_y.is_finite());
            }
        }

        // 4.3: Extreme zoom out: zoom = 0.01 (100x zoom-out)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Whiteboard;
            st.viewport.offset_x = 0.0;
            st.viewport.offset_y = 0.0;
            st.viewport.zoom = 0.01;
        }

        let id_z001 = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insertion at zoom 0.01 should succeed on Whiteboard");
        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_z001)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x.is_finite());
            assert!(bbox.max_x.is_finite());
            assert!(bbox.width() > 0.0);
            assert!(bbox.height() > 0.0);
        }

        // 4.4: Extreme zoom in: zoom = 100.0 (100x magnification)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Whiteboard;
            st.viewport.offset_x = 0.0;
            st.viewport.offset_y = 0.0;
            st.viewport.zoom = 100.0;
        }

        let id_z100 = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insertion at zoom 100.0 should succeed on Whiteboard");
        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_z100)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x.is_finite());
            assert!(bbox.max_x.is_finite());
            assert!(bbox.width() <= 100.0 + 1e-3);
            assert!(bbox.height() <= 100.0 + 1e-3);
        }

        // 4.5: Huge positive coordinates (offset_x = 1e12, offset_y = 1e12)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Whiteboard;
            st.viewport.offset_x = 1e12;
            st.viewport.offset_y = 1e12;
            st.viewport.zoom = 1.0;
        }

        let id_huge_pos = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insertion at huge positive offset should succeed");
        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_huge_pos)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x.is_finite());
            assert!(bbox.max_x.is_finite());
            assert!(bbox.width() > 0.0);
        }

        // 4.6: Huge negative coordinates (offset_x = -1e12, offset_y = -1e12)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Whiteboard;
            st.viewport.offset_x = -1e12;
            st.viewport.offset_y = -1e12;
            st.viewport.zoom = 1.0;
        }

        let id_huge_neg = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insertion at huge negative offset should succeed");
        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_huge_neg)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x.is_finite());
            assert!(bbox.max_x.is_finite());
            assert!(bbox.width() > 0.0);
        }
    });
}

// ============================================================================
// 5. NOTEBOOK MULTI-PAGE BOUNDARIES & EMPTY PAGES HANDLING
// ============================================================================

#[test]
fn test_adv_notebook_multi_page_boundaries_and_empty_pages() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();

        // 5.1: Notebook with 10 pages
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.format = DocumentFormat::Notebook;
            for _ in 0..9 {
                st.document.add_page(PageBackground::default());
            }
            assert_eq!(st.document.pages.len(), 10);
        }

        let png = create_test_png(120, 80, (0.1, 0.7, 0.3));

        // Test insertion way above page 0 (Y = -50,000 pt)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.viewport = Viewport {
                offset_x: 0.0,
                offset_y: 50300.0, // vp_center_y = 300 - 50300 = -50000
                zoom: 1.0,
            };
        }
        let id_top = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insert above top should map to page 0");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            assert_eq!(st.selection.page_index, Some(0));
            let page0_rect = Viewport::page_rect(0);
            let item = st.document.pages[0]
                .items
                .iter()
                .find(|it| it.id == id_top)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x >= page0_rect.x + 10.0 - 1e-3);
            assert!(bbox.min_y >= page0_rect.y + 10.0 - 1e-3);
            assert!(bbox.max_x <= page0_rect.x + page0_rect.width - 10.0 + 1e-3);
            assert!(bbox.max_y <= page0_rect.y + page0_rect.height - 10.0 + 1e-3);
        }

        // Test insertion way below page 9 (Y = +500,000 pt)
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.viewport = Viewport {
                offset_x: 0.0,
                offset_y: -499700.0, // vp_center_y = 300 - (-499700) = 500000
                zoom: 1.0,
            };
        }
        let id_bottom = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("Insert below bottom should map to page 9");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            assert_eq!(st.selection.page_index, Some(9));
            let page9_rect = Viewport::page_rect(9);
            let item = st.document.pages[9]
                .items
                .iter()
                .find(|it| it.id == id_bottom)
                .unwrap();
            let bbox = item.bounding_box().unwrap();
            assert!(bbox.min_x >= page9_rect.x + 10.0 - 1e-3);
            assert!(bbox.min_y >= page9_rect.y + 10.0 - 1e-3);
            assert!(bbox.max_x <= page9_rect.x + page9_rect.width - 10.0 + 1e-3);
            assert!(bbox.max_y <= page9_rect.y + page9_rect.height - 10.0 + 1e-3);
        }

        // 5.2: Document with 0 pages gracefully returns Err
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            st.document.pages.clear();
        }
        let res_no_pages = canvas.insert_image_bytes(&png, Some("image/png"));
        assert!(
            res_no_pages.is_err(),
            "Document with zero pages must return Err without panicking"
        );
    });
}

// ============================================================================
// 6. IMAGE CACHE LRU CHURN & CAPACITY BOUNDARY BEHAVIOR
// ============================================================================

#[test]
fn test_adv_image_cache_lru_stress_and_eviction() {
    run_on_gtk_thread(|| {
        let mut cache = ImageCacheManager::new(8); // limit 8 items
        assert!(!cache.contains("non_existent"));

        // Generate 20 distinct PNGs
        let mut entries = Vec::new();
        for i in 0..20 {
            let bytes = create_test_png(10, 10, (i as f64 * 0.05, 0.5, 0.5));
            let hash = format!("hash_{i:03}");
            let surf = cache
                .insert_from_png_bytes(&hash, &bytes)
                .expect("insert into cache");
            entries.push((hash, bytes, surf));
        }

        // Cache must have strictly clamped size <= 8
        let mut resident_count = 0;
        for (h, _, _) in &entries {
            if cache.contains(h) {
                resident_count += 1;
            }
        }
        assert_eq!(
            resident_count, 8,
            "Cache must retain exactly 8 entries (max_entries limit)"
        );

        // The earliest entries (hash_000 to hash_011) should have been evicted
        assert!(!cache.contains("hash_000"));
        assert!(!cache.contains("hash_005"));
        // The latest entries (hash_012 to hash_019) must be present
        assert!(cache.contains("hash_019"));
        assert!(cache.contains("hash_018"));

        // Touch hash_012 so its access_seq is updated to newest
        let surf_12 = cache.get("hash_012");
        assert!(surf_12.is_some());

        // Now insert a new one: hash_020
        let bytes_20 = create_test_png(10, 10, (0.9, 0.9, 0.9));
        cache
            .insert_from_png_bytes("hash_020", &bytes_20)
            .expect("insert 20");

        // Because hash_012 was touched, hash_013 should be evicted instead!
        assert!(
            cache.contains("hash_012"),
            "hash_012 was touched so must survive"
        );
        assert!(
            !cache.contains("hash_013"),
            "hash_013 was oldest so must be evicted"
        );
        assert!(cache.contains("hash_020"), "hash_020 was newly inserted");

        // Clear cache
        cache.clear();
        assert!(!cache.contains("hash_012"));
        assert!(!cache.contains("hash_020"));
    });
}

// ============================================================================
// 7. JPEG / WEBP ASSET RELOADING FROM SQLITE STORE INTO IMAGE CACHE
// ============================================================================

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

#[test]
fn test_adv_jpeg_reload_from_sqlite_store_behavior() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let db_path = unique_test_db("jpeg_reload");
        canvas
            .save_as(db_path.clone())
            .expect("save_as to attach db");

        let jpeg_bytes = sample_valid_jpeg_bytes();
        let expected_hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, jpeg_bytes)
            .unwrap()
            .to_string();

        let _id = canvas
            .insert_image_bytes(jpeg_bytes, Some("image/jpeg"))
            .expect("JPEG insertion must succeed");

        // 1. In-memory cache contains entry right after insertion (pre-populated by insert_image_bytes)
        let cache_rc = uninote_app::image_cache::get_global_image_cache();
        assert!(
            cache_rc.borrow().contains(&expected_hash),
            "Cache must be pre-populated after insertion"
        );

        // 2. Clear cache to simulate app restart or LRU eviction
        cache_rc.borrow_mut().clear();
        assert!(!cache_rc.borrow().contains(&expected_hash));

        // 3. Document store has the asset saved
        let store = DocumentStore::open_or_create(&db_path).expect("open store");
        let loaded_bytes = store
            .load_asset(&expected_hash)
            .expect("load asset")
            .expect("asset present");
        assert_eq!(loaded_bytes, jpeg_bytes);

        // 4. Test what get_or_load_from_store does with raw JPEG bytes
        let surf_opt = cache_rc
            .borrow_mut()
            .get_or_load_from_store(&expected_hash, &store);
        println!(
            "[Challenger M2-1] get_or_load_from_store for JPEG asset: returned Some? {}",
            surf_opt.is_some()
        );

        let _ = fs::remove_file(db_path);
    });
}

// ============================================================================
// 8. CONCURRENT THREAD-SAFETY OF IN-MEMORY CLIPBOARD
// ============================================================================

#[test]
fn test_adv_concurrent_clipboard_access_no_deadlock() {
    use uninote_app::clipboard::{
        advance_paste_count, clear_clipboard, get_clipboard_data, set_clipboard_data, ClipboardData,
    };
    use uninote_core::DocumentFormat;
    use uninote_stroke::Rect;

    let sample_data = ClipboardData {
        source_format: DocumentFormat::Notebook,
        source_doc_id: None,
        source_page_index: Some(0),
        source_page_size: Some((595.0, 842.0)),
        original_bbox: Rect::new(10.0, 10.0, 100.0, 100.0),
        original_center: (55.0, 55.0),
        items: Vec::new(),
        paste_count: 0,
        source_viewport_offset: None,
    };

    let handles: Vec<_> = (0..16)
        .map(|thread_idx| {
            let data_clone = sample_data.clone();
            thread::spawn(move || {
                for i in 0..100 {
                    if (thread_idx + i) % 3 == 0 {
                        set_clipboard_data(data_clone.clone());
                    } else if (thread_idx + i) % 3 == 1 {
                        let _ = get_clipboard_data();
                        let _ = advance_paste_count();
                    } else {
                        clear_clipboard();
                    }
                }
            })
        })
        .collect();

    for h in handles {
        h.join()
            .expect("Worker thread panicked during clipboard stress");
    }

    // Clean up
    clear_clipboard();
}

// ============================================================================
// 9. SELECTION STATE ISOLATION ON NEW IMAGE INSERTION
// ============================================================================

#[test]
fn test_adv_selection_state_isolation_on_new_insertion() {
    run_on_gtk_thread(|| {
        let canvas = create_test_canvas();
        let png = create_test_png(50, 50, (0.2, 0.4, 0.6));

        // Insert first image
        let id1 = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("insert 1");
        {
            let st_rc = canvas.state();
            let mut st = st_rc.borrow_mut();
            // Simulate that multiple items were selected and dragging was active
            st.selection.selected_ids = vec![id1, 9999, 8888];
            st.selection.is_dragging = true;
            st.selection.drag_offset = (15.0, -25.0);
        }

        // Insert second image
        let id2 = canvas
            .insert_image_bytes(&png, Some("image/png"))
            .expect("insert 2");

        {
            let st_rc = canvas.state();
            let st = st_rc.borrow();
            // Must strictly isolate id2 only!
            assert_eq!(
                st.selection.selected_ids,
                vec![id2],
                "Selection must be cleared and solely contain the newly inserted item id"
            );
            assert!(
                !st.selection.is_dragging,
                "Dragging flag must be reset to false"
            );
            assert_eq!(
                st.selection.drag_offset,
                (0.0, 0.0),
                "Drag offset must be reset to zero"
            );
            assert_eq!(
                st.active_tool,
                uninote_app::canvas::ActiveTool::Select,
                "Active tool must be ActiveTool::Select"
            );
        }
    });
}
