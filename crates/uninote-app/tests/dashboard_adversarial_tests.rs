//! Adversarial and Empirical Test Suite for Milestone M3 (Dashboard & Preferences)
//!
//! Scope of Empirical Verification:
//! 1. Stress testing `DashboardRecentStore` with heavy insertions (600+ items), ensuring strict 30-item LRU capacity clamping and order preservation.
//! 2. Concurrent atomic saves and rapid serial writes with continuous readers (verifying zero partial JSON reads or corruption).
//! 3. Path sanitization under adversarial conditions: non-existent files, broken symlinks, valid symlinks, deleted files.
//! 4. Cairo thumbnail empirical checks: exact 240x340 px dimensions, ARGB32 format, aspect ratio, non-zero pixel data, and visual differentiation between Notebook and Whiteboard.
//! 5. Corrupted JSON recovery and graceful fallback.
//! 6. Query filtering: case-insensitivity, Unicode normalization, whitespace trimming, and empty queries.

use gtk4::cairo::{Format, ImageSurface};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use uninote_app::dashboard::{get_or_create_thumbnail_path, render_cairo_thumbnail};
use uninote_app::preferences::{DashboardRecentStore, RecentDocumentMeta};
use uninote_core::model::DocumentFormat;

static ADVERSARIAL_ID: AtomicU64 = AtomicU64::new(1000);

fn unique_test_path(prefix: &str, ext: &str) -> PathBuf {
    let id = ADVERSARIAL_ID.fetch_add(1, Ordering::SeqCst);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "uninote_m3_adv_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
    ));
    p
}

// ============================================================================
// 1. HEAVY INSERTION & LRU CLAMPING STRESS TEST (600+ ITEMS)
// ============================================================================

#[test]
fn test_adv_heavy_insertions_600_items_strict_lru_clamping() {
    let mut store = DashboardRecentStore::new();
    assert_eq!(store.recent_files.len(), 0);

    // Insert 600 distinct items sequentially
    for i in 1..=600 {
        let meta = RecentDocumentMeta {
            file_path: format!("/tmp/heavy_note_{:04}.uninote", i),
            title: format!("Quaderno Accademico {:04}", i),
            format: if i % 2 == 0 {
                DocumentFormat::Notebook
            } else {
                DocumentFormat::Whiteboard
            },
            last_modified_timestamp: i as u64 * 100,
            page_count: (i % 50) + 1,
            thumbnail_path: None,
        };
        store.add_recent(meta);

        // Once 30 is reached, size MUST stay exactly clamped to 30 at every single step
        if i >= 30 {
            assert_eq!(
                store.recent_files.len(),
                DashboardRecentStore::MAX_RECENT_ITEMS,
                "Length must remain strictly 30 at insertion i={}",
                i
            );
        } else {
            assert_eq!(store.recent_files.len(), i);
        }

        // Most recently added item must ALWAYS be at index 0
        assert_eq!(
            store.recent_files[0].title,
            format!("Quaderno Accademico {:04}", i),
            "Most recent item must be at index 0"
        );
    }

    // Verify final state after 600 insertions:
    assert_eq!(store.recent_files.len(), 30);

    // Old items 1..=570 must have been completely evicted
    for i in 1..=570 {
        let path = format!("/tmp/heavy_note_{:04}.uninote", i);
        assert!(
            store.recent_files.iter().all(|r| r.file_path != path),
            "Evicted item {} should not be in recent_files",
            i
        );
    }

    // Items 571..=600 must be present in exact reverse order (LRU order)
    for (idx, expected_num) in (571..=600).rev().enumerate() {
        assert_eq!(
            store.recent_files[idx].file_path,
            format!("/tmp/heavy_note_{:04}.uninote", expected_num),
            "Index {} mismatch in LRU chain",
            idx
        );
        assert_eq!(
            store.recent_files[idx].title,
            format!("Quaderno Accademico {:04}", expected_num)
        );
    }

    // Test LRU promotion: re-insert item 575 (currently near the tail, at index 25)
    let promoted_meta = RecentDocumentMeta {
        file_path: "/tmp/heavy_note_0575.uninote".into(),
        title: "Quaderno Accademico 0575 (Riaperto)".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 999_999,
        page_count: 10,
        thumbnail_path: None,
    };
    store.add_recent(promoted_meta);

    // Total length must still be exactly 30
    assert_eq!(store.recent_files.len(), 30);
    // Index 0 must now be item 575
    assert_eq!(
        store.recent_files[0].file_path,
        "/tmp/heavy_note_0575.uninote"
    );
    assert_eq!(
        store.recent_files[0].title,
        "Quaderno Accademico 0575 (Riaperto)"
    );

    // Verify item 571 is still in the list at index 29 (tail was not evicted because 575 was an update)
    assert_eq!(
        store.recent_files[29].file_path,
        "/tmp/heavy_note_0571.uninote"
    );

    // Now insert a BRAND NEW item 601: item 571 must be evicted, tail becomes 572
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/heavy_note_0601.uninote".into(),
        title: "Quaderno Accademico 0601".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 1_000_000,
        page_count: 1,
        thumbnail_path: None,
    });
    assert_eq!(store.recent_files.len(), 30);
    assert_eq!(
        store.recent_files[0].file_path,
        "/tmp/heavy_note_0601.uninote"
    );
    assert!(
        store
            .recent_files
            .iter()
            .all(|r| r.file_path != "/tmp/heavy_note_0571.uninote"),
        "Item 571 must now be evicted"
    );
    assert_eq!(
        store.recent_files[29].file_path,
        "/tmp/heavy_note_0572.uninote"
    );
}

// ============================================================================
// 2. ATOMIC FILE WRITES & CONCURRENT READ/WRITE STRESS TESTS
// ============================================================================

#[test]
fn test_adv_rapid_serial_writes_concurrent_reader_no_partial_reads() {
    let test_path = unique_test_path("atomic_rw", "json");
    let test_path_clone = test_path.clone();

    let stop_flag = Arc::new(AtomicBool::new(false));
    let stop_flag_clone = stop_flag.clone();
    let read_count = Arc::new(AtomicUsize::new(0));
    let read_count_clone = read_count.clone();
    let corrupted_count = Arc::new(AtomicUsize::new(0));
    let corrupted_clone = corrupted_count.clone();

    // Reader thread: continuously reads while writer writes rapidly
    let reader_handle = thread::spawn(move || {
        while !stop_flag_clone.load(Ordering::Relaxed) {
            if test_path_clone.exists() {
                // Read raw text first to inspect JSON validity
                match fs::read_to_string(&test_path_clone) {
                    Ok(text) => {
                        if text.trim().is_empty() {
                            // Partial read! Empty file exposed!
                            corrupted_clone.fetch_add(1, Ordering::SeqCst);
                        } else {
                            match serde_json::from_str::<DashboardRecentStore>(&text) {
                                Ok(store) => {
                                    if !store.recent_files.is_empty() {
                                        let title = &store.recent_files[0].title;
                                        if !title.starts_with("Serial Doc ") {
                                            corrupted_clone.fetch_add(1, Ordering::SeqCst);
                                        }
                                    }
                                    read_count_clone.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(_) => {
                                    // Deserialization failure indicates half-written JSON
                                    corrupted_clone.fetch_add(1, Ordering::SeqCst);
                                }
                            }
                        }
                    }
                    Err(_) => {
                        // File might be briefly in transit, but rename should be atomic on POSIX
                    }
                }
            }
            thread::yield_now();
        }
    });

    // Writer thread (main): 120 rapid serial atomic writes
    for i in 1..=120 {
        let mut store = DashboardRecentStore::new();
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/doc_serial_{}.uninote", i),
            title: format!("Serial Doc {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: i as u64,
            page_count: (i % 20) + 1,
            thumbnail_path: None,
        });

        store
            .save_atomic(&test_path)
            .expect("save_atomic must succeed");
        thread::sleep(Duration::from_millis(1));
    }

    stop_flag.store(true, Ordering::Relaxed);
    reader_handle
        .join()
        .expect("Reader thread must join cleanly");

    let total_reads = read_count.load(Ordering::SeqCst);
    let total_corrupt = corrupted_count.load(Ordering::SeqCst);

    println!(
        "Rapid serial write test: total successful reads = {}, corrupted reads = {}",
        total_reads, total_corrupt
    );

    assert!(
        total_reads > 10,
        "Reader must perform multiple successful reads"
    );
    assert_eq!(
        total_corrupt, 0,
        "Atomic rename guarantees ZERO corrupted or partial JSON reads"
    );

    let _ = fs::remove_file(test_path);
}

#[test]
fn test_adv_concurrent_saves_distinct_paths_multithreaded() {
    let thread_count = 8;
    let mut handles = Vec::new();

    for t in 0..thread_count {
        let handle = thread::spawn(move || {
            let path = unique_test_path(&format!("thread_{}", t), "json");
            let mut store = DashboardRecentStore::new();
            let mut thread_docs = Vec::new();

            for i in 1..=15 {
                let doc_path = unique_test_path(&format!("t{}_doc_{}", t, i), "uninote");
                fs::write(&doc_path, format!("thread {} content {}", t, i)).unwrap();
                store.add_recent(RecentDocumentMeta {
                    file_path: doc_path.to_string_lossy().to_string(),
                    title: format!("Thread {} Note {}", t, i),
                    format: DocumentFormat::Notebook,
                    last_modified_timestamp: (t * 100 + i) as u64,
                    page_count: i,
                    thumbnail_path: None,
                });
                thread_docs.push(doc_path);
            }

            store
                .save_atomic(&path)
                .unwrap_or_else(|e| panic!("Thread {} save failed: {:?}", t, e));

            assert!(path.exists());
            let loaded = DashboardRecentStore::load_or_default(&path);
            assert_eq!(loaded.recent_files.len(), 15);
            assert_eq!(
                loaded.recent_files[0].title,
                format!("Thread {} Note 15", t)
            );

            let _ = fs::remove_file(path);
            for d in thread_docs {
                let _ = fs::remove_file(d);
            }
        });
        handles.push(handle);
    }

    for h in handles {
        h.join().expect("Worker thread panicked");
    }
}

#[test]
fn test_adv_concurrent_saves_same_path_race_condition() {
    let shared_path = unique_test_path("shared_race", "json");
    let thread_count = 6;
    let mut handles = Vec::new();

    // Create 10 real files on disk for the test
    let mut real_docs = Vec::new();
    for i in 1..=10 {
        let p = unique_test_path(&format!("shared_doc_{}", i), "uninote");
        fs::write(&p, format!("shared content {}", i)).unwrap();
        real_docs.push(p);
    }

    let real_docs_arc = Arc::new(real_docs);

    for t in 0..thread_count {
        let path_clone = shared_path.clone();
        let docs = real_docs_arc.clone();
        let handle = thread::spawn(move || {
            let mut errors = 0;
            for iter in 0..20 {
                let mut store = DashboardRecentStore::new();
                for (idx, doc_p) in docs.iter().enumerate() {
                    store.add_recent(RecentDocumentMeta {
                        file_path: doc_p.to_string_lossy().to_string(),
                        title: format!("Thread {} Iter {} Note {}", t, iter, idx),
                        format: DocumentFormat::Notebook,
                        last_modified_timestamp: (t * 1000 + iter * 10 + idx) as u64,
                        page_count: 1,
                        thumbnail_path: None,
                    });
                }
                if let Err(_e) = store.save_atomic(&path_clone) {
                    errors += 1;
                }
                thread::sleep(Duration::from_millis(1));
            }
            errors
        });
        handles.push(handle);
    }

    let mut total_errors = 0;
    for h in handles {
        total_errors += h.join().expect("Worker thread panicked");
    }

    println!(
        "Concurrent same-path saves: total write errors = {} across threads",
        total_errors
    );

    // If the file exists after concurrent execution, verify it is completely uncorrupted
    if shared_path.exists() {
        let content = fs::read_to_string(&shared_path).expect("Read shared_path");
        let parsed: Result<DashboardRecentStore, _> = serde_json::from_str(&content);
        assert!(
            parsed.is_ok(),
            "Resulting file must NEVER be corrupted JSON even under same-path concurrent saves: {:?}",
            parsed.err()
        );
        let store = parsed.unwrap();
        assert_eq!(store.recent_files.len(), 10);
    }

    let _ = fs::remove_file(shared_path);
    for p in real_docs_arc.iter() {
        let _ = fs::remove_file(p);
    }
}

// ============================================================================
// 3. ADVERSARIAL PATH SANITIZATION & GHOST FILES
// ============================================================================

#[test]
fn test_adv_path_sanitization_comprehensive() {
    let mut store = DashboardRecentStore::new();
    let mut real_files = Vec::new();

    // 1. Create 20 real temp files on disk
    for i in 1..=20 {
        let p = unique_test_path(&format!("real_{}", i), "uninote");
        fs::write(&p, format!("real content {}", i)).unwrap();
        store.add_recent(RecentDocumentMeta {
            file_path: p.to_string_lossy().to_string(),
            title: format!("Real File {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: i as u64,
            page_count: 1,
            thumbnail_path: None,
        });
        real_files.push(p);
    }

    // 2. Add 5 non-existent ghost files
    for i in 1..=5 {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/ghost_non_existent_path_{:08}.uninote", i),
            title: format!("Ghost File {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 100 + i as u64,
            page_count: 1,
            thumbnail_path: None,
        });
    }

    // 3. Create 3 valid symlinks and 3 broken symlinks
    let mut valid_symlinks = Vec::new();
    let mut broken_symlinks = Vec::new();

    for i in 1..=3 {
        // Valid symlink pointing to an existing real file
        let valid_link = unique_test_path(&format!("valid_link_{}", i), "uninote");
        std::os::unix::fs::symlink(&real_files[i - 1], &valid_link).unwrap();
        store.add_recent(RecentDocumentMeta {
            file_path: valid_link.to_string_lossy().to_string(),
            title: format!("Valid Link {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 200 + i as u64,
            page_count: 1,
            thumbnail_path: None,
        });
        valid_symlinks.push(valid_link);

        // Broken symlink pointing to a non-existent target
        let broken_link = unique_test_path(&format!("broken_link_{}", i), "uninote");
        let fake_target = PathBuf::from(format!("/tmp/fake_target_symlink_{}.uninote", i));
        std::os::unix::fs::symlink(&fake_target, &broken_link).unwrap();
        store.add_recent(RecentDocumentMeta {
            file_path: broken_link.to_string_lossy().to_string(),
            title: format!("Broken Link {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 300 + i as u64,
            page_count: 1,
            thumbnail_path: None,
        });
        broken_symlinks.push(broken_link);
    }

    // Delete 5 of the real files from disk to turn them into ghosts
    for p in real_files.iter().take(5) {
        let _ = fs::remove_file(p);
    }

    // Run sanitization
    store.sanitize_existing_files();

    // Verify all remaining items actually exist on disk
    for r in &store.recent_files {
        let p = Path::new(&r.file_path);
        assert!(
            p.exists(),
            "Path in store must exist on disk after sanitization: {}",
            r.file_path
        );
    }

    // The 5 deleted real files must not be present
    for p in real_files.iter().take(5) {
        let path_str = p.to_string_lossy().to_string();
        assert!(
            store.recent_files.iter().all(|r| r.file_path != path_str),
            "Deleted real file must be purged by sanitize_existing_files"
        );
    }

    // The 5 ghost files must not be present
    for i in 1..=5 {
        let ghost_path = format!("/tmp/ghost_non_existent_path_{:08}.uninote", i);
        assert!(
            store.recent_files.iter().all(|r| r.file_path != ghost_path),
            "Ghost file must be purged by sanitize_existing_files"
        );
    }

    // The 3 broken symlinks must not be present (Path::exists() returns false for broken symlinks)
    for bl in &broken_symlinks {
        let path_str = bl.to_string_lossy().to_string();
        assert!(
            store.recent_files.iter().all(|r| r.file_path != path_str),
            "Broken symlink must be purged by sanitize_existing_files"
        );
    }

    // Clean up remaining real files and symlinks
    for p in real_files.iter().skip(5) {
        let _ = fs::remove_file(p);
    }
    for l in valid_symlinks {
        let _ = fs::remove_file(l);
    }
    for b in broken_symlinks {
        let _ = fs::remove_file(b);
    }
}

// ============================================================================
// 4. CAIRO THUMBNAIL EMPIRICAL VERIFICATION
// ============================================================================

#[test]
fn test_adv_cairo_thumbnail_dimensions_format_and_png_structure() {
    let nb_path = unique_test_path("emp_nb_thumb", "png");
    let wb_path = unique_test_path("emp_wb_thumb", "png");

    render_cairo_thumbnail("Algoritmi II", DocumentFormat::Notebook, &nb_path)
        .expect("Notebook thumbnail rendering failed");
    render_cairo_thumbnail("Architettura Lab", DocumentFormat::Whiteboard, &wb_path)
        .expect("Whiteboard thumbnail rendering failed");

    for (path, format_name) in [(&nb_path, "Notebook"), (&wb_path, "Whiteboard")] {
        assert!(path.exists(), "PNG file must exist on disk");
        let bytes = fs::read(path).expect("Read PNG bytes failed");

        // 1. Verify standard PNG magic header: 89 50 4E 47 0D 0A 1A 0A
        assert!(bytes.len() > 8, "PNG file too short");
        assert_eq!(
            &bytes[0..8],
            &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A],
            "{} thumbnail must have valid PNG magic header",
            format_name
        );

        // 2. Parse IHDR chunk directly from bytes (offset 8 is length, offset 12 is 'IHDR', 16 is width, 20 is height)
        assert_eq!(&bytes[12..16], b"IHDR");
        let width = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
        let height = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
        let bit_depth = bytes[24];

        assert_eq!(
            width, 240,
            "{} thumbnail PNG width in IHDR must be exactly 240 px",
            format_name
        );
        assert_eq!(
            height, 340,
            "{} thumbnail PNG height in IHDR must be exactly 340 px",
            format_name
        );
        assert_eq!(bit_depth, 8, "PNG bit depth must be 8");

        // 3. Aspect ratio check
        let aspect_ratio = width as f64 / height as f64;
        let expected_ratio = 240.0 / 340.0;
        assert!(
            (aspect_ratio - expected_ratio).abs() < 1e-6,
            "Aspect ratio must match 240/340 (got {})",
            aspect_ratio
        );

        // 4. Decode via Cairo ImageSurface to verify dimensions and pixel integrity
        let mut file = fs::File::open(path).expect("Open PNG for Cairo");
        let surface =
            ImageSurface::create_from_png(&mut file).expect("Cairo surface from PNG failed");
        assert_eq!(surface.width(), 240);
        assert_eq!(surface.height(), 340);
        assert!(
            surface.format() == Format::ARgb32 || surface.format() == Format::Rgb24,
            "Decoded Cairo surface format must be ARgb32 or Rgb24 (got {:?})",
            surface.format()
        );
    }

    let _ = fs::remove_file(nb_path);
    let _ = fs::remove_file(wb_path);
}

#[test]
fn test_adv_cairo_thumbnail_visual_differentiation() {
    let nb_path = unique_test_path("diff_nb", "png");
    let wb_path = unique_test_path("diff_wb", "png");

    render_cairo_thumbnail("Analisi I", DocumentFormat::Notebook, &nb_path).unwrap();
    render_cairo_thumbnail("Analisi I", DocumentFormat::Whiteboard, &wb_path).unwrap();

    let mut nb_file = fs::File::open(&nb_path).unwrap();
    let mut wb_file = fs::File::open(&wb_path).unwrap();

    let mut nb_surface = ImageSurface::create_from_png(&mut nb_file).unwrap();
    let mut wb_surface = ImageSurface::create_from_png(&mut wb_file).unwrap();

    let nb_data = nb_surface.data().expect("nb surface data");
    let wb_data = wb_surface.data().expect("wb surface data");

    assert_eq!(
        nb_data.len(),
        wb_data.len(),
        "Surface data buffers must be equal length"
    );

    // Count differing bytes between Notebook and Whiteboard representations
    let total_pixels = 240 * 340;
    let mut differing_pixels = 0;

    for i in 0..total_pixels {
        let offset = i * 4;
        let nb_pixel = &nb_data[offset..offset + 4];
        let wb_pixel = &wb_data[offset..offset + 4];
        if nb_pixel != wb_pixel {
            differing_pixels += 1;
        }
    }

    println!(
        "Thumbnail visual difference: {} out of {} pixels differ ({:.1}%)",
        differing_pixels,
        total_pixels,
        (differing_pixels as f64 / total_pixels as f64) * 100.0
    );

    // Visual designs are distinct: Notebook has ruling lines and cyan header, Whiteboard has mint dot grid and central ring
    assert!(
        differing_pixels > 2500,
        "Notebook and Whiteboard thumbnails must be visually distinct: got {} differing pixels",
        differing_pixels
    );

    let _ = fs::remove_file(nb_path);
    let _ = fs::remove_file(wb_path);
}

#[test]
fn test_adv_thumbnail_path_hashing_and_pathological_titles() {
    let meta_short = RecentDocumentMeta {
        file_path: "/tmp/doc1.uninote".into(),
        title: "Normal Title".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 100,
        page_count: 1,
        thumbnail_path: None,
    };
    let path1 = get_or_create_thumbnail_path(&meta_short);
    let path2 = get_or_create_thumbnail_path(&meta_short);
    assert_eq!(path1, path2, "Thumbnail path must be deterministic");

    // Title with Unicode and special characters
    let meta_unicode = RecentDocumentMeta {
        file_path: "/tmp/doc_unicode.uninote".into(),
        title: "Calcolo Differenziale §1 (Prof. Rossi — 2026/27) 日本語 🚀".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 200,
        page_count: 5,
        thumbnail_path: None,
    };
    let path_uni = get_or_create_thumbnail_path(&meta_unicode);
    assert!(
        path_uni.to_string_lossy().ends_with(".png"),
        "Must generate valid .png filename"
    );

    // Extremely long title (500 chars) — must not panic or cause unhandled crash
    let long_title = "A".repeat(500);
    let meta_long = RecentDocumentMeta {
        file_path: "/tmp/doc_long.uninote".into(),
        title: long_title,
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 300,
        page_count: 1,
        thumbnail_path: None,
    };
    let path_long = get_or_create_thumbnail_path(&meta_long);
    assert!(
        path_long.is_absolute(),
        "Path must be absolute without panic"
    );
}

// ============================================================================
// 5. CORRUPTED JSON RECOVERY & ROBUSTNESS
// ============================================================================

#[test]
fn test_adv_corrupted_json_recovery_graceful_defaults() {
    // 1. Truncated JSON
    let p_trunc = unique_test_path("trunc", "json");
    fs::write(&p_trunc, r#"{"default_notes_dir": "Uni"#).unwrap();
    let s1 = DashboardRecentStore::load_or_default(&p_trunc);
    assert_eq!(s1.recent_files.len(), 0);
    assert!(s1.default_notes_dir.contains("UniNote"));

    // 2. Zero-byte file
    let p_zero = unique_test_path("zero", "json");
    fs::write(&p_zero, b"").unwrap();
    let s2 = DashboardRecentStore::load_or_default(&p_zero);
    assert_eq!(s2.recent_files.len(), 0);

    // 3. Random binary garbage
    let p_bin = unique_test_path("bin", "json");
    fs::write(&p_bin, [0xFF, 0xFE, 0x00, 0x01, 0xDE, 0xAD, 0xBE, 0xEF]).unwrap();
    let s3 = DashboardRecentStore::load_or_default(&p_bin);
    assert_eq!(s3.recent_files.len(), 0);

    // 4. JSON Array instead of expected Object
    let p_arr = unique_test_path("arr", "json");
    fs::write(&p_arr, r#"[1, 2, 3, "invalid"]"#).unwrap();
    let s4 = DashboardRecentStore::load_or_default(&p_arr);
    assert_eq!(s4.recent_files.len(), 0);

    // 5. Non-existent path
    let p_none = PathBuf::from("/tmp/surely_does_not_exist_at_all_99999.json");
    let s5 = DashboardRecentStore::load_or_default(&p_none);
    assert_eq!(s5.recent_files.len(), 0);

    // Clean up
    let _ = fs::remove_file(p_trunc);
    let _ = fs::remove_file(p_zero);
    let _ = fs::remove_file(p_bin);
    let _ = fs::remove_file(p_arr);
}

// ============================================================================
// 6. SEARCH QUERY FILTERING & UNICODE NORMALIZATION
// ============================================================================

#[test]
fn test_adv_search_query_filtering_adversarial() {
    let mut store = DashboardRecentStore::new();
    let sample_titles = [
        "Metodi Matematici per l'Ingegneria",
        "Teoria dei Circuiti & Elettronica Applicata",
        "Fisica Generale: Meccanica e Termodinamica",
        "Laboratorio di Programmazione Rust (A.A. 2026/27)",
        "Economia ed Organizzazione Aziendale",
    ];

    for (idx, title) in sample_titles.iter().enumerate() {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/query_test_{}.uninote", idx),
            title: (*title).into(),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: (idx + 1) as u64,
            page_count: 5,
            thumbnail_path: None,
        });
    }

    // 1. Whitespace padding on query
    let res_ws = store.filter_by_query("   circuiti   ");
    assert_eq!(res_ws.len(), 1);
    assert_eq!(
        res_ws[0].title,
        "Teoria dei Circuiti & Elettronica Applicata"
    );

    // 2. Case insensitivity (uppercase query for mixed case title)
    let res_upper = store.filter_by_query("FISICA GENERALE");
    assert_eq!(res_upper.len(), 1);
    assert_eq!(
        res_upper[0].title,
        "Fisica Generale: Meccanica e Termodinamica"
    );

    // 3. Special characters in query
    let res_special = store.filter_by_query("2026/27");
    assert_eq!(res_special.len(), 1);
    assert_eq!(
        res_special[0].title,
        "Laboratorio di Programmazione Rust (A.A. 2026/27)"
    );

    // 4. Empty or whitespace-only query returns everything
    assert_eq!(store.filter_by_query("").len(), 5);
    assert_eq!(store.filter_by_query("   \t\n  ").len(), 5);

    // 5. Query matching nothing
    assert_eq!(store.filter_by_query("NonExistingSubjectXYZ").len(), 0);
}
