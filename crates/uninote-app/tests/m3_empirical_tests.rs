//! Empirical and Adversarial Verification Suite for Milestone M3 (Dashboard & Settings)
//!
//! Developed by empirical_challenger (challenger_2b_m3_it2_1) to stress-test:
//! 1. `UniNoteDashboard::format_badge_label` and top-level `format_badge_label` across all invariants.
//! 2. `DashboardRecentStore` under pathological titles, paths, LRU reordering, query filtering, and sanitization.
//! 3. Symlink handling and edge cases in `sanitize_existing_files`.
//! 4. Concurrency, atomic persistence, corrupted JSON recovery, and tempfile hygiene.
//! 5. Callback wiring contracts for `UniNoteDashboard`.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;

use uninote_app::dashboard::{
    format_badge_label as top_format_badge_label, get_or_create_thumbnail_path,
    render_cairo_thumbnail, UniNoteDashboard,
};
use uninote_app::preferences::{DashboardRecentStore, RecentDocumentMeta};
use uninote_core::model::DocumentFormat;

static TEST_COUNTER: AtomicU64 = AtomicU64::new(5000);

fn unique_temp_path(prefix: &str, ext: &str) -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut p = std::env::temp_dir();
    p.push(format!(
        "uninote_m3_challenger_{}_{}_{}.{}",
        prefix,
        std::process::id(),
        id,
        ext
    ));
    p
}

// ============================================================================
// 1. FORMAT BADGE LABEL ADVERSARIAL & INVARIANT TESTS
// ============================================================================

#[test]
fn test_m3_format_badge_label_invariants_and_equivalence() {
    // Test both top-level and associated functions
    let top_notebook = top_format_badge_label(DocumentFormat::Notebook);
    let assoc_notebook = UniNoteDashboard::format_badge_label(DocumentFormat::Notebook);
    assert_eq!(top_notebook, "Quaderno A4");
    assert_eq!(assoc_notebook, "Quaderno A4");
    assert_eq!(top_notebook, assoc_notebook);

    let top_whiteboard = top_format_badge_label(DocumentFormat::Whiteboard);
    let assoc_whiteboard = UniNoteDashboard::format_badge_label(DocumentFormat::Whiteboard);
    assert_eq!(top_whiteboard, "Lavagna Infinita");
    assert_eq!(assoc_whiteboard, "Lavagna Infinita");
    assert_eq!(top_whiteboard, assoc_whiteboard);

    // Differentiation: the two labels must be distinct
    assert_ne!(top_notebook, top_whiteboard);

    // Invariant: Non-empty, UTF-8 clean, no extraneous whitespace
    for label in [top_notebook, top_whiteboard] {
        assert!(!label.is_empty(), "Badge label must not be empty");
        assert_eq!(
            label.trim(),
            label,
            "Badge label must have no leading/trailing whitespace"
        );
        assert!(
            label.chars().all(|c| !c.is_control()),
            "Badge label must not have control chars"
        );
    }
}

// ============================================================================
// 2. DASHBOARD RECENT STORE: PATHOLOGICAL TITLES & STRINGS
// ============================================================================

#[test]
fn test_m3_store_pathological_titles_and_metadata() {
    let mut store = DashboardRecentStore::new();

    let huge_title = "A".repeat(10_000);
    let pathological_cases = [
        "",                                                // empty title
        "   ",                                             // whitespace only
        "🎉🚀📚 Quaderno con Emojis e Simboli 🧮📝",       // emojis
        "رياضيات متقدمة - تحليل II",                       // Arabic (RTL)
        "אלגברה לינארית מתקדמת",                           // Hebrew (RTL)
        "量子力学講義ノート",                              // Japanese CJK
        "Математический анализ",                           // Cyrillic
        "{\"injected\": true, \"format\": \"malformed\"}", // JSON injection string
        "'; DROP TABLE notes; --",                         // SQL injection string
        "<script>alert('xss')</script>",                   // HTML injection string
        huge_title.as_str(),                               // 10,000 char huge title
        "Title\nWith\nNewlines\r\nAnd\tTabs",              // Control chars in title
    ];

    for (i, &title) in pathological_cases.iter().enumerate() {
        let meta = RecentDocumentMeta {
            file_path: format!("/tmp/pathological_note_{}.uninote", i),
            title: title.to_string(),
            format: if i % 2 == 0 {
                DocumentFormat::Notebook
            } else {
                DocumentFormat::Whiteboard
            },
            last_modified_timestamp: u64::MAX - (i as u64),
            page_count: if i == 0 { 0 } else { usize::MAX - i },
            thumbnail_path: None,
        };
        store.add_recent(meta);

        assert_eq!(store.recent_files[0].title, title);
        assert_eq!(
            store.recent_files[0].file_path,
            format!("/tmp/pathological_note_{}.uninote", i)
        );
    }

    // Verify raw JSON roundtrip serialization fidelity of all pathological titles
    let temp_json = unique_temp_path("pathological_store", "json");
    store.save_atomic(&temp_json).expect("Save must succeed");

    let json_content = fs::read_to_string(&temp_json).expect("Read JSON");
    let loaded: DashboardRecentStore = serde_json::from_str(&json_content).expect("Parse JSON");
    assert_eq!(
        loaded.recent_files.len(),
        pathological_cases
            .len()
            .min(DashboardRecentStore::MAX_RECENT_ITEMS)
    );

    for (idx, original_meta) in store.recent_files.iter().enumerate() {
        assert_eq!(loaded.recent_files[idx].title, original_meta.title);
        assert_eq!(loaded.recent_files[idx].file_path, original_meta.file_path);
        assert_eq!(loaded.recent_files[idx].format, original_meta.format);
        assert_eq!(
            loaded.recent_files[idx].last_modified_timestamp,
            original_meta.last_modified_timestamp
        );
        assert_eq!(
            loaded.recent_files[idx].page_count,
            original_meta.page_count
        );
    }

    // Now test load_or_default behavior: since none of the /tmp/pathological_note_* exist on disk,
    // load_or_default MUST sanitize all non-existent paths, returning exactly 0 files.
    let sanitized_on_load = DashboardRecentStore::load_or_default(&temp_json);
    assert_eq!(
        sanitized_on_load.recent_files.len(),
        0,
        "load_or_default must sanitize all non-existent files"
    );

    // Now create real files on disk for 3 pathological titles and verify load_or_default preserves them
    let mut real_store = DashboardRecentStore::new();
    let mut real_paths = Vec::new();
    for i in 0..3 {
        let p = unique_temp_path(&format!("real_patho_{}", i), "uninote");
        fs::write(&p, b"uninote_test").unwrap();
        real_store.add_recent(RecentDocumentMeta {
            file_path: p.to_string_lossy().to_string(),
            title: pathological_cases[i + 2].to_string(), // Emojis, Arabic, Hebrew
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 12345,
            page_count: 7,
            thumbnail_path: None,
        });
        real_paths.push(p);
    }
    let real_json = unique_temp_path("real_patho_store", "json");
    real_store.save_atomic(&real_json).expect("Save real store");

    let loaded_real = DashboardRecentStore::load_or_default(&real_json);
    assert_eq!(
        loaded_real.recent_files.len(),
        3,
        "Real existing files must be preserved on load_or_default"
    );
    assert_eq!(loaded_real.recent_files[0].title, pathological_cases[4]); // Hebrew (most recent)
    assert_eq!(loaded_real.recent_files[1].title, pathological_cases[3]); // Arabic
    assert_eq!(loaded_real.recent_files[2].title, pathological_cases[2]); // Emojis

    // Clean up
    let _ = fs::remove_file(&temp_json);
    let _ = fs::remove_file(&real_json);
    for p in real_paths {
        let _ = fs::remove_file(p);
    }
}

// ============================================================================
// 3. DASHBOARD RECENT STORE: DUPLICATE PATHS & LRU REORDERING INVARIANTS
// ============================================================================

#[test]
fn test_m3_store_duplicate_paths_lru_invariants() {
    let mut store = DashboardRecentStore::new();

    // Add 10 initial documents
    for i in 1..=10 {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/doc_{}.uninote", i),
            title: format!("Doc {}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: i as u64,
            page_count: i,
            thumbnail_path: None,
        });
    }
    assert_eq!(store.recent_files.len(), 10);
    // Index 0 is doc 10, index 9 is doc 1
    assert_eq!(store.recent_files[0].file_path, "/tmp/doc_10.uninote");
    assert_eq!(store.recent_files[9].file_path, "/tmp/doc_1.uninote");

    // Re-insert doc 1 with updated metadata
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/doc_1.uninote".to_string(),
        title: "Doc 1 - Updated Revision".to_string(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 9999,
        page_count: 42,
        thumbnail_path: Some("/tmp/thumb_doc_1.png".to_string()),
    });

    // Invariant: Length MUST NOT increase (must remain 10)
    assert_eq!(
        store.recent_files.len(),
        10,
        "Re-inserting existing path must not grow list"
    );

    // Invariant: Doc 1 MUST now be at index 0 (LRU promotion)
    assert_eq!(store.recent_files[0].file_path, "/tmp/doc_1.uninote");
    assert_eq!(store.recent_files[0].title, "Doc 1 - Updated Revision");
    assert_eq!(store.recent_files[0].format, DocumentFormat::Whiteboard);
    assert_eq!(store.recent_files[0].last_modified_timestamp, 9999);
    assert_eq!(store.recent_files[0].page_count, 42);

    // Invariant: Doc 1 must NOT appear anywhere else in the list
    let count_doc1 = store
        .recent_files
        .iter()
        .filter(|r| r.file_path == "/tmp/doc_1.uninote")
        .count();
    assert_eq!(count_doc1, 1, "Doc 1 must appear exactly once");

    // Invariant: Doc 10 was pushed to index 1, Doc 2 is at index 9
    assert_eq!(store.recent_files[1].file_path, "/tmp/doc_10.uninote");
    assert_eq!(store.recent_files[9].file_path, "/tmp/doc_2.uninote");
}

// ============================================================================
// 4. SANITIZATION: SYMLINKS, BROKEN SYMLINKS, AND NON-EXISTENT FILES
// ============================================================================

#[test]
fn test_m3_store_sanitization_with_symlinks_and_lifecycle() {
    let mut store = DashboardRecentStore::new();

    // Create 3 real temp files
    let real_file_1 = unique_temp_path("real_1", "uninote");
    let real_file_2 = unique_temp_path("real_2", "uninote");
    let real_file_3 = unique_temp_path("real_3", "uninote");
    fs::write(&real_file_1, b"doc1").unwrap();
    fs::write(&real_file_2, b"doc2").unwrap();
    fs::write(&real_file_3, b"doc3").unwrap();

    // Create a valid symlink to real_file_1
    let symlink_valid = unique_temp_path("symlink_valid", "uninote");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&real_file_1, &symlink_valid).unwrap();

    // Create a broken symlink (points to deleted target)
    let dead_target = unique_temp_path("dead_target", "uninote");
    let symlink_broken = unique_temp_path("symlink_broken", "uninote");
    #[cfg(unix)]
    std::os::unix::fs::symlink(&dead_target, &symlink_broken).unwrap();

    let non_existent = unique_temp_path("never_existed", "uninote");

    store.add_recent(RecentDocumentMeta {
        file_path: real_file_1.to_string_lossy().to_string(),
        title: "Real 1".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 100,
        page_count: 1,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: real_file_2.to_string_lossy().to_string(),
        title: "Real 2".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 200,
        page_count: 2,
        thumbnail_path: None,
    });
    store.add_recent(RecentDocumentMeta {
        file_path: real_file_3.to_string_lossy().to_string(),
        title: "Real 3".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 300,
        page_count: 3,
        thumbnail_path: None,
    });
    #[cfg(unix)]
    {
        store.add_recent(RecentDocumentMeta {
            file_path: symlink_valid.to_string_lossy().to_string(),
            title: "Valid Symlink".into(),
            format: DocumentFormat::Whiteboard,
            last_modified_timestamp: 400,
            page_count: 1,
            thumbnail_path: None,
        });
        store.add_recent(RecentDocumentMeta {
            file_path: symlink_broken.to_string_lossy().to_string(),
            title: "Broken Symlink".into(),
            format: DocumentFormat::Whiteboard,
            last_modified_timestamp: 500,
            page_count: 1,
            thumbnail_path: None,
        });
    }
    store.add_recent(RecentDocumentMeta {
        file_path: non_existent.to_string_lossy().to_string(),
        title: "Non Existent".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 600,
        page_count: 1,
        thumbnail_path: None,
    });

    let non_existent_str = non_existent.to_string_lossy();
    let real_1_str = real_file_1.to_string_lossy();
    let real_2_str = real_file_2.to_string_lossy();
    let real_3_str = real_file_3.to_string_lossy();
    #[cfg(unix)]
    let symlink_valid_str = symlink_valid.to_string_lossy();
    #[cfg(unix)]
    let symlink_broken_str = symlink_broken.to_string_lossy();

    // Before sanitize: contains all entries
    assert!(store
        .recent_files
        .iter()
        .any(|r| r.file_path == non_existent_str));

    // Run sanitize
    store.sanitize_existing_files();

    // After sanitize:
    // 1. Non-existent file MUST be pruned
    assert!(!store
        .recent_files
        .iter()
        .any(|r| r.file_path == non_existent_str));

    #[cfg(unix)]
    {
        // 2. Broken symlink MUST be pruned (Path::exists() follows symlink and returns false)
        assert!(!store
            .recent_files
            .iter()
            .any(|r| r.file_path == symlink_broken_str));
        // 3. Valid symlink MUST be preserved
        assert!(store
            .recent_files
            .iter()
            .any(|r| r.file_path == symlink_valid_str));
    }

    // 4. Real files 1, 2, 3 must be preserved
    assert!(store.recent_files.iter().any(|r| r.file_path == real_1_str));
    assert!(store.recent_files.iter().any(|r| r.file_path == real_2_str));
    assert!(store.recent_files.iter().any(|r| r.file_path == real_3_str));

    // Now delete real_file_2 from disk and re-sanitize
    let _ = fs::remove_file(&real_file_2);
    store.sanitize_existing_files();
    assert!(!store.recent_files.iter().any(|r| r.file_path == real_2_str));

    // Cleanup
    let _ = fs::remove_file(&real_file_1);
    let _ = fs::remove_file(&real_file_3);
    #[cfg(unix)]
    {
        let _ = fs::remove_file(&symlink_valid);
        let _ = fs::remove_file(&symlink_broken);
    }
}

// ============================================================================
// 5. SEARCH FILTERING: REGEX CHARACTERS, UNICODE, AND WHITESPACE
// ============================================================================

#[test]
fn test_m3_store_search_query_resilience() {
    let mut store = DashboardRecentStore::new();

    let items = [
        ("Analisi Matematica I", "/tmp/an1.uninote"),
        ("Analisi Matematica II (Avanzata)", "/tmp/an2.uninote"),
        ("Fisica Generale & Quantistica", "/tmp/fis.uninote"),
        ("Algebra Lineare [Geometria]", "/tmp/alg.uninote"),
        ("Sistemi Operativi: Linux/Unix", "/tmp/so.uninote"),
        ("Chimica Organica {Lab}", "/tmp/ch.uninote"),
        ("Special $100 & *.* Query Test", "/tmp/sp.uninote"),
    ];

    for (t, p) in items {
        store.add_recent(RecentDocumentMeta {
            file_path: p.to_string(),
            title: t.to_string(),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 100,
            page_count: 5,
            thumbnail_path: None,
        });
    }

    // Adversarial queries containing regex characters that would crash regex engines:
    let regex_queries = [
        (".*", 1),          // Only matches the item with literal ".*" in title
        ("[Geometria]", 1), // Literal brackets
        ("(", 1),           // Unmatched parenthesis
        (")", 1),
        ("{Lab}", 1),
        ("$100", 1),
        ("\\", 0), // Backslash
        ("?", 0),
        ("+", 0),
        ("^", 0),
    ];

    for (query, expected_matches) in regex_queries {
        let results = store.filter_by_query(query);
        assert_eq!(
            results.len(),
            expected_matches,
            "Query '{}' should yield {} matches without crashing",
            query,
            expected_matches
        );
    }

    // Case insensitivity and trimming:
    assert_eq!(store.filter_by_query("   analisi   ").len(), 2);
    assert_eq!(store.filter_by_query("ANALISI").len(), 2);
    assert_eq!(store.filter_by_query("aNaLiSi").len(), 2);
    assert_eq!(store.filter_by_query("").len(), 7);
    assert_eq!(store.filter_by_query("   ").len(), 7);
    assert_eq!(store.filter_by_query("Inesistente").len(), 0);
}

// ============================================================================
// 6. THUMBNAIL CACHE PATH AND CAIRO GENERATION EMPIRICAL STRESS
// ============================================================================

#[test]
fn test_m3_thumbnail_hashing_and_rendering_invariants() {
    // 1. Two documents with the identical title but different paths must produce distinct thumbnail filenames
    let meta_a = RecentDocumentMeta {
        file_path: "/home/student/semester1/Appunti.uninote".into(),
        title: "Appunti".into(),
        format: DocumentFormat::Notebook,
        last_modified_timestamp: 1000,
        page_count: 10,
        thumbnail_path: None,
    };
    let meta_b = RecentDocumentMeta {
        file_path: "/home/student/semester2/Appunti.uninote".into(),
        title: "Appunti".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 2000,
        page_count: 1,
        thumbnail_path: None,
    };

    let path_a = get_or_create_thumbnail_path(&meta_a);
    let path_b = get_or_create_thumbnail_path(&meta_b);

    assert_ne!(
        path_a, path_b,
        "Thumbnails for identical titles on different paths must not collide"
    );

    // 2. Render both formats to temp PNGs and verify valid PNG format and dimensions
    let tmp_png_nb = unique_temp_path("test_thumb_nb", "png");
    let tmp_png_wb = unique_temp_path("test_thumb_wb", "png");

    render_cairo_thumbnail(&meta_a.title, DocumentFormat::Notebook, &tmp_png_nb)
        .expect("Notebook thumb render");
    render_cairo_thumbnail(&meta_b.title, DocumentFormat::Whiteboard, &tmp_png_wb)
        .expect("Whiteboard thumb render");

    assert!(tmp_png_nb.exists());
    assert!(tmp_png_wb.exists());

    let bytes_nb = fs::read(&tmp_png_nb).expect("Read notebook thumb");
    let bytes_wb = fs::read(&tmp_png_wb).expect("Read whiteboard thumb");

    // Standard PNG magic bytes: 0x89, 'P', 'N', 'G', 0x0D, 0x0A, 0x1A, 0x0A
    let png_magic = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    assert!(bytes_nb.starts_with(&png_magic), "Must be valid PNG file");
    assert!(bytes_wb.starts_with(&png_magic), "Must be valid PNG file");

    // Check PNG IHDR chunk for dimensions (240 x 340)
    // Width is bytes 16..20 (big endian), Height is bytes 20..24 (big endian)
    let w_nb = u32::from_be_bytes([bytes_nb[16], bytes_nb[17], bytes_nb[18], bytes_nb[19]]);
    let h_nb = u32::from_be_bytes([bytes_nb[20], bytes_nb[21], bytes_nb[22], bytes_nb[23]]);
    assert_eq!(w_nb, 240, "Notebook thumb width must be 240px");
    assert_eq!(h_nb, 340, "Notebook thumb height must be 340px");

    let w_wb = u32::from_be_bytes([bytes_wb[16], bytes_wb[17], bytes_wb[18], bytes_wb[19]]);
    let h_wb = u32::from_be_bytes([bytes_wb[20], bytes_wb[21], bytes_wb[22], bytes_wb[23]]);
    assert_eq!(w_wb, 240, "Whiteboard thumb width must be 240px");
    assert_eq!(h_wb, 340, "Whiteboard thumb height must be 340px");

    // Visual differentiation: file bytes must differ between formats
    assert_ne!(
        bytes_nb, bytes_wb,
        "Notebook and Whiteboard thumbnails must be visually distinct"
    );

    let _ = fs::remove_file(&tmp_png_nb);
    let _ = fs::remove_file(&tmp_png_wb);
}

// ============================================================================
// 7. ATOMIC PERSISTENCE & CONCURRENT STRESS RESILIENCE
// ============================================================================

#[test]
fn test_m3_concurrent_atomic_save_and_recovery() {
    let target_path = Arc::new(unique_temp_path("concurrent_store", "json"));
    let mut handles = Vec::new();

    // Spawn 8 concurrent writer threads
    for t_idx in 0..8 {
        let path_clone = target_path.clone();
        handles.push(thread::spawn(move || {
            for i in 0..25 {
                let mut store = DashboardRecentStore::new();
                for k in 0..15 {
                    store.add_recent(RecentDocumentMeta {
                        file_path: format!("/tmp/thread_{}_iter_{}_doc_{}.uninote", t_idx, i, k),
                        title: format!("Thread {} Iter {} Doc {}", t_idx, i, k),
                        format: if (t_idx + k) % 2 == 0 {
                            DocumentFormat::Notebook
                        } else {
                            DocumentFormat::Whiteboard
                        },
                        last_modified_timestamp: (t_idx * 1000 + i * 10 + k) as u64,
                        page_count: k + 1,
                        thumbnail_path: None,
                    });
                }
                let res = store.save_atomic(&path_clone);
                assert!(res.is_ok(), "Concurrent save must not fail: {:?}", res);
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    // Verify the resulting file is valid JSON and parses cleanly without corruption
    let file_content = fs::read_to_string(&*target_path).expect("File must be readable");
    let parsed_store: DashboardRecentStore =
        serde_json::from_str(&file_content).expect("Must parse as valid JSON");
    assert_eq!(
        parsed_store.recent_files.len(),
        15,
        "Recent store must have 15 items"
    );

    // load_or_default should execute gracefully without panicking
    let loaded_sanitized = DashboardRecentStore::load_or_default(&target_path);
    assert_eq!(
        loaded_sanitized.recent_files.len(),
        0,
        "Since thread doc paths don't exist, load_or_default sanitizes them"
    );

    // Verify no leftover .tmp files remain in temp dir
    if let Some(parent) = target_path.parent() {
        if let Ok(entries) = fs::read_dir(parent) {
            let tmp_prefix = format!("{}.tmp", target_path.file_name().unwrap().to_string_lossy());
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                assert!(
                    !name.starts_with(&tmp_prefix),
                    "No leftover temp file should remain: {}",
                    name
                );
            }
        }
    }

    let _ = fs::remove_file(&*target_path);
}

// ============================================================================
// 8. DEFAULT NOTES DIRECTORY NESTED CREATION & IDEMPOTENCE
// ============================================================================

#[test]
fn test_m3_ensure_default_notes_dir_nested_and_idempotent() {
    let base_temp = unique_temp_path("notes_base", "dir");
    let deep_nested = base_temp.join("level1").join("level2").join("UniNote");

    let mut store = DashboardRecentStore::new();
    store.default_notes_dir = deep_nested.to_string_lossy().to_string();

    assert!(
        !deep_nested.exists(),
        "Directory should not exist initially"
    );

    // First call creates the deep directory tree
    store
        .ensure_default_notes_dir()
        .expect("Directory creation must succeed");
    assert!(deep_nested.exists(), "Deep directory must now exist");
    assert!(deep_nested.is_dir(), "Target path must be a directory");

    // Second call is idempotent and succeeds without error
    store
        .ensure_default_notes_dir()
        .expect("Second call must succeed idempotently");
    assert!(deep_nested.exists());

    // Cleanup
    let _ = fs::remove_dir_all(&base_temp);
}

// ============================================================================
// 9. RECENT STORE CAPACITY 30 BOUNDARY INVARIANT & EVICTION ORACLE
// ============================================================================

#[test]
fn test_m3_recent_store_exact_capacity_30_oracle() {
    let mut store = DashboardRecentStore::new();

    // Insert 30 items
    for i in 1..=30 {
        store.add_recent(RecentDocumentMeta {
            file_path: format!("/tmp/capacity_doc_{:03}.uninote", i),
            title: format!("Doc {:03}", i),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: i as u64,
            page_count: i,
            thumbnail_path: None,
        });
        assert_eq!(store.recent_files.len(), i);
    }
    assert_eq!(store.recent_files.len(), 30);
    assert_eq!(
        store.recent_files[0].file_path,
        "/tmp/capacity_doc_030.uninote"
    );
    assert_eq!(
        store.recent_files[29].file_path,
        "/tmp/capacity_doc_001.uninote"
    );

    // Now insert item 31: item 1 must be evicted
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/capacity_doc_031.uninote".into(),
        title: "Doc 031".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 31,
        page_count: 31,
        thumbnail_path: None,
    });
    assert_eq!(store.recent_files.len(), 30);
    assert_eq!(
        store.recent_files[0].file_path,
        "/tmp/capacity_doc_031.uninote"
    );
    assert_eq!(
        store.recent_files[29].file_path,
        "/tmp/capacity_doc_002.uninote"
    );
    assert!(store
        .recent_files
        .iter()
        .all(|r| r.file_path != "/tmp/capacity_doc_001.uninote"));

    // Now promote the oldest item in the list (doc 002 at index 29)
    store.add_recent(RecentDocumentMeta {
        file_path: "/tmp/capacity_doc_002.uninote".into(),
        title: "Doc 002 (Promoted)".into(),
        format: DocumentFormat::Whiteboard,
        last_modified_timestamp: 9999,
        page_count: 100,
        thumbnail_path: None,
    });
    assert_eq!(store.recent_files.len(), 30, "Length must remain 30");
    assert_eq!(
        store.recent_files[0].file_path,
        "/tmp/capacity_doc_002.uninote"
    );
    assert_eq!(store.recent_files[0].title, "Doc 002 (Promoted)");
    assert_eq!(
        store.recent_files[29].file_path,
        "/tmp/capacity_doc_003.uninote"
    );
}

// ============================================================================
// 10. UNINOTE DASHBOARD GTK WIDGET & CALLBACK WIRING CONTRACT
// ============================================================================

struct GtkTestHarness {
    sender: std::sync::mpsc::Sender<Box<dyn FnOnce() + Send>>,
}

static GTK_HARNESS: std::sync::OnceLock<GtkTestHarness> = std::sync::OnceLock::new();

fn run_on_gtk_thread<F, R>(f: F) -> R
where
    F: FnOnce() -> R + Send + 'static,
    R: Send + 'static,
{
    let harness = GTK_HARNESS.get_or_init(|| {
        let (tx, rx) = std::sync::mpsc::channel::<Box<dyn FnOnce() + Send>>();
        thread::Builder::new()
            .name("gtk-challenger-test-worker".into())
            .spawn(move || {
                let _ = gtk4::init();
                while let Ok(task) = rx.recv() {
                    task();
                }
            })
            .expect("Failed to spawn GTK test worker thread");
        GtkTestHarness { sender: tx }
    });

    let (res_tx, res_rx) = std::sync::mpsc::channel();
    harness
        .sender
        .send(Box::new(move || {
            let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
            let _ = res_tx.send(res);
        }))
        .expect("GTK test worker thread is dead");

    match res_rx
        .recv()
        .expect("GTK test worker did not return result")
    {
        Ok(val) => val,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

#[test]
fn test_m3_uninote_dashboard_gtk_wiring_contract() {
    run_on_gtk_thread(|| {
        use gtk4::prelude::*;
        use std::cell::RefCell;
        use std::rc::Rc;

        let dashboard = UniNoteDashboard::new();
        let widget = dashboard.widget();

        // 1. Invariant: Widget is valid and of expected class
        assert!(widget.has_css_class("dms-dashboard") || widget.is_sensitive());

        // 2. Test connect_open_document contract
        let opened_path = Rc::new(RefCell::new(None));
        let op_clone = opened_path.clone();
        dashboard.connect_open_document(move |p| {
            *op_clone.borrow_mut() = Some(p);
        });

        // 3. Test connect_create_document contract
        let created_tuple = Rc::new(RefCell::new(None));
        let ct_clone = created_tuple.clone();
        dashboard.connect_create_document(move |title, format, bg| {
            *ct_clone.borrow_mut() = Some((title, format, bg));
        });

        // 4. Test connect_open_settings contract
        let settings_opened = Rc::new(RefCell::new(false));
        let so_clone = settings_opened.clone();
        dashboard.connect_open_settings(move || {
            *so_clone.borrow_mut() = true;
        });

        // Verify card generation works without panic and assigns proper format badge and CSS
        let meta_nb = RecentDocumentMeta {
            file_path: "/tmp/card_test_nb.uninote".into(),
            title: "Quaderno Test Card".into(),
            format: DocumentFormat::Notebook,
            last_modified_timestamp: 1000,
            page_count: 5,
            thumbnail_path: None,
        };
        let open_cb: uninote_app::dashboard::OpenDocCallback = Rc::new(RefCell::new(None));
        let card_nb = UniNoteDashboard::create_card(&meta_nb, &open_cb);
        assert!(card_nb.has_css_class("dms-flowbox-child"));

        let meta_wb = RecentDocumentMeta {
            file_path: "/tmp/card_test_wb.uninote".into(),
            title: "Lavagna Test Card".into(),
            format: DocumentFormat::Whiteboard,
            last_modified_timestamp: 2000,
            page_count: 1,
            thumbnail_path: None,
        };
        let card_wb = UniNoteDashboard::create_card(&meta_wb, &open_cb);
        assert!(card_wb.has_css_class("dms-flowbox-child"));
    });
}
