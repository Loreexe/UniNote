//! UniNote M5 Challenger — Empirical Adversarial Stress Test Suite
//!
//! Exhaustive empirical challenge suite for the Deterministic Academic Dataset Generator:
//! 1. Seed Determinism & Reproducibility across repeated runs (Notebook & Whiteboard at 1k, 10k, 50k).
//! 2. Exact stroke counts and content distribution proportions (~60% cursive, ~25% math, ~15% diagrams).
//! 3. Pre-warmed cache validation (`cached_bbox.is_some()`, `cached_bezier` for non-dots) and spatial index completeness.
//! 4. Rigorous ground-truth Oracle equivalence for spatial index envelope queries across diverse regions.
//! 5. Structural block vs stroke-level distribution analysis and edge case robustness.

use uninote_core::dataset::{
    AcademicDatasetGenerator, AcademicScale, DeterministicPrng, DEFAULT_ACADEMIC_SEED,
};
use uninote_core::model::{Document, ItemKind, Page};
use uninote_stroke::types::{Rect, Stroke};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ContentType {
    Handwriting,
    Math,
    Diagram,
}

fn classify_stroke(stroke: &Stroke) -> ContentType {
    let (r, g, b, _a) = stroke.style.color;
    if (r - 0.05).abs() < 1e-4 && (g - 0.20).abs() < 1e-4 && (b - 0.55).abs() < 1e-4 {
        ContentType::Math
    } else if ((r - 0.15).abs() < 1e-4 && (g - 0.18).abs() < 1e-4 && (b - 0.22).abs() < 1e-4)
        || ((r - 0.85).abs() < 1e-4 && (g - 0.20).abs() < 1e-4 && (b - 0.25).abs() < 1e-4)
    {
        ContentType::Diagram
    } else {
        ContentType::Handwriting
    }
}

fn count_strokes_in_doc(doc: &Document) -> usize {
    doc.pages.iter().map(|p| p.items.len()).sum()
}

fn assert_documents_bitwise_equal(doc1: &Document, doc2: &Document, context: &str) {
    assert_eq!(
        doc1.format, doc2.format,
        "[{context}] Document formats differ"
    );
    assert_eq!(
        doc1.pages.len(),
        doc2.pages.len(),
        "[{context}] Page count differs"
    );

    for (p_idx, (p1, p2)) in doc1.pages.iter().zip(doc2.pages.iter()).enumerate() {
        assert_eq!(
            p1.items.len(),
            p2.items.len(),
            "[{context}] Page {p_idx} items count differs"
        );
        assert_eq!(
            p1.spatial_index.len(),
            p2.spatial_index.len(),
            "[{context}] Page {p_idx} spatial_index length differs"
        );

        for (i_idx, (it1, it2)) in p1.items.iter().zip(p2.items.iter()).enumerate() {
            assert_eq!(
                it1.id, it2.id,
                "[{context}] Page {p_idx} Item {i_idx} id differs"
            );
            assert_eq!(
                it1.z_index, it2.z_index,
                "[{context}] Page {p_idx} Item {i_idx} z_index differs"
            );

            let ItemKind::Stroke(ref s1) = it1.kind;
            let ItemKind::Stroke(ref s2) = it2.kind;

            assert_eq!(
                s1.id, s2.id,
                "[{context}] Page {p_idx} Item {i_idx} stroke.id differs"
            );
            assert_eq!(
                s1.is_dot, s2.is_dot,
                "[{context}] Page {p_idx} Item {i_idx} stroke.is_dot differs"
            );
            assert_eq!(
                s1.mode, s2.mode,
                "[{context}] Page {p_idx} Item {i_idx} stroke.mode differs"
            );
            assert_eq!(
                s1.style.color, s2.style.color,
                "[{context}] Page {p_idx} Item {i_idx} stroke.style.color differs"
            );
            assert_eq!(
                s1.style.base_width.to_bits(),
                s2.style.base_width.to_bits(),
                "[{context}] Page {p_idx} Item {i_idx} stroke.style.base_width differs"
            );
            assert_eq!(
                s1.points.len(),
                s2.points.len(),
                "[{context}] Page {p_idx} Item {i_idx} stroke.points.len differs"
            );

            for (pt_idx, (pt1, pt2)) in s1.points.iter().zip(s2.points.iter()).enumerate() {
                assert_eq!(
                    pt1.x.to_bits(),
                    pt2.x.to_bits(),
                    "[{context}] Page {p_idx} Stroke {i_idx} pt {pt_idx} x differs"
                );
                assert_eq!(
                    pt1.y.to_bits(),
                    pt2.y.to_bits(),
                    "[{context}] Page {p_idx} Stroke {i_idx} pt {pt_idx} y differs"
                );
                assert_eq!(
                    pt1.pressure.to_bits(),
                    pt2.pressure.to_bits(),
                    "[{context}] Page {p_idx} Stroke {i_idx} pt {pt_idx} pressure differs"
                );
                assert_eq!(
                    pt1.width.to_bits(),
                    pt2.width.to_bits(),
                    "[{context}] Page {p_idx} Stroke {i_idx} pt {pt_idx} width differs"
                );
                assert_eq!(
                    pt1.timestamp_ns, pt2.timestamp_ns,
                    "[{context}] Page {p_idx} Stroke {i_idx} pt {pt_idx} timestamp_ns differs"
                );
            }

            // Bounding box equality
            assert_eq!(
                s1.cached_bbox.is_some(),
                s2.cached_bbox.is_some(),
                "[{context}] Page {p_idx} Stroke {i_idx} cached_bbox presence differs"
            );
            if let (Some(b1), Some(b2)) = (s1.cached_bbox, s2.cached_bbox) {
                assert_eq!(
                    b1.min_x.to_bits(),
                    b2.min_x.to_bits(),
                    "[{context}] bbox min_x differs"
                );
                assert_eq!(
                    b1.min_y.to_bits(),
                    b2.min_y.to_bits(),
                    "[{context}] bbox min_y differs"
                );
                assert_eq!(
                    b1.max_x.to_bits(),
                    b2.max_x.to_bits(),
                    "[{context}] bbox max_x differs"
                );
                assert_eq!(
                    b1.max_y.to_bits(),
                    b2.max_y.to_bits(),
                    "[{context}] bbox max_y differs"
                );
            }

            // Bézier segment equality
            assert_eq!(
                s1.cached_bezier.len(),
                s2.cached_bezier.len(),
                "[{context}] cached_bezier len differs"
            );
            for (bz_idx, (bz1, bz2)) in s1
                .cached_bezier
                .iter()
                .zip(s2.cached_bezier.iter())
                .enumerate()
            {
                assert_eq!(
                    bz1.p0.0.to_bits(),
                    bz2.p0.0.to_bits(),
                    "[{context}] bz {bz_idx} p0.0 differs"
                );
                assert_eq!(
                    bz1.p0.1.to_bits(),
                    bz2.p0.1.to_bits(),
                    "[{context}] bz {bz_idx} p0.1 differs"
                );
                assert_eq!(
                    bz1.cp1.0.to_bits(),
                    bz2.cp1.0.to_bits(),
                    "[{context}] bz {bz_idx} cp1.0 differs"
                );
                assert_eq!(
                    bz1.cp1.1.to_bits(),
                    bz2.cp1.1.to_bits(),
                    "[{context}] bz {bz_idx} cp1.1 differs"
                );
                assert_eq!(
                    bz1.cp2.0.to_bits(),
                    bz2.cp2.0.to_bits(),
                    "[{context}] bz {bz_idx} cp2.0 differs"
                );
                assert_eq!(
                    bz1.cp2.1.to_bits(),
                    bz2.cp2.1.to_bits(),
                    "[{context}] bz {bz_idx} cp2.1 differs"
                );
                assert_eq!(
                    bz1.p1.0.to_bits(),
                    bz2.p1.0.to_bits(),
                    "[{context}] bz {bz_idx} p1.0 differs"
                );
                assert_eq!(
                    bz1.p1.1.to_bits(),
                    bz2.p1.1.to_bits(),
                    "[{context}] bz {bz_idx} p1.1 differs"
                );
            }
        }
    }
}

// ----------------------------------------------------------------------------
// 1. REPRODUCIBILITY & BITWISE DETERMINISM TESTS
// ----------------------------------------------------------------------------

#[test]
fn test_challenger_prng_seed_sensitivity() {
    let mut prng_a1 = DeterministicPrng::new(0x1234_5678);
    let mut prng_a2 = DeterministicPrng::new(0x1234_5678);
    let mut prng_b = DeterministicPrng::new(0x8765_4321);

    for _ in 0..1000 {
        let v1 = prng_a1.next_u64();
        let v2 = prng_a2.next_u64();
        let vb = prng_b.next_u64();
        assert_eq!(v1, v2, "Identical seeds must produce identical values");
        assert_ne!(v1, vb, "Different seeds must produce different values");
    }
}

#[test]
fn test_challenger_reproducibility_notebook_1k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_notebook(AcademicScale::Small1k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_notebook(AcademicScale::Small1k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Notebook 1k");
}

#[test]
fn test_challenger_reproducibility_notebook_10k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_notebook(AcademicScale::Medium10k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_notebook(AcademicScale::Medium10k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Notebook 10k");
}

#[test]
fn test_challenger_reproducibility_notebook_50k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_notebook(AcademicScale::Massive50k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_notebook(AcademicScale::Massive50k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Notebook 50k");
}

#[test]
fn test_challenger_reproducibility_whiteboard_1k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_whiteboard(AcademicScale::Small1k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_whiteboard(AcademicScale::Small1k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Whiteboard 1k");
}

#[test]
fn test_challenger_reproducibility_whiteboard_10k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_whiteboard(AcademicScale::Medium10k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_whiteboard(AcademicScale::Medium10k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Whiteboard 10k");
}

#[test]
fn test_challenger_reproducibility_whiteboard_50k() {
    let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc1 = gen1.generate_whiteboard(AcademicScale::Massive50k);

    let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
    let doc2 = gen2.generate_whiteboard(AcademicScale::Massive50k);

    assert_documents_bitwise_equal(&doc1, &doc2, "Whiteboard 50k");
}

// ----------------------------------------------------------------------------
// 2. EXACT STROKE COUNTS & DISTRIBUTION PROPORTIONS
// ----------------------------------------------------------------------------

#[test]
fn test_challenger_exact_stroke_counts_all_scales() {
    let mut gen = AcademicDatasetGenerator::new();

    // Notebook exact counts
    let doc_nb_1k = gen.generate_notebook(AcademicScale::Small1k);
    assert_eq!(
        count_strokes_in_doc(&doc_nb_1k),
        1_000,
        "Notebook Small1k stroke count"
    );

    let doc_nb_10k = gen.generate_notebook(AcademicScale::Medium10k);
    assert_eq!(
        count_strokes_in_doc(&doc_nb_10k),
        10_000,
        "Notebook Medium10k stroke count"
    );

    let doc_nb_50k = gen.generate_notebook(AcademicScale::Massive50k);
    assert_eq!(
        count_strokes_in_doc(&doc_nb_50k),
        50_000,
        "Notebook Massive50k stroke count"
    );

    // Whiteboard exact counts
    let doc_wb_1k = gen.generate_whiteboard(AcademicScale::Small1k);
    assert_eq!(
        count_strokes_in_doc(&doc_wb_1k),
        1_000,
        "Whiteboard Small1k stroke count"
    );

    let doc_wb_10k = gen.generate_whiteboard(AcademicScale::Medium10k);
    assert_eq!(
        count_strokes_in_doc(&doc_wb_10k),
        10_000,
        "Whiteboard Medium10k stroke count"
    );

    let doc_wb_50k = gen.generate_whiteboard(AcademicScale::Massive50k);
    assert_eq!(
        count_strokes_in_doc(&doc_wb_50k),
        50_000,
        "Whiteboard Massive50k stroke count"
    );
}

#[test]
fn test_challenger_edge_cases_custom_scales() {
    let mut gen = AcademicDatasetGenerator::new();

    // Scale 1: Single stroke edge case
    let doc_1 = gen.generate_notebook(AcademicScale::Custom(1));
    assert_eq!(count_strokes_in_doc(&doc_1), 1);
    assert_eq!(doc_1.pages.len(), 1);
    assert_eq!(doc_1.pages[0].spatial_index.len(), 1);

    // Scale 2: Two strokes edge case
    let doc_2 = gen.generate_notebook(AcademicScale::Custom(2));
    assert_eq!(count_strokes_in_doc(&doc_2), 2);
    assert_eq!(doc_2.pages[0].spatial_index.len(), 2);

    // Scale 7: Prime number stroke count
    let doc_7 = gen.generate_notebook(AcademicScale::Custom(7));
    assert_eq!(count_strokes_in_doc(&doc_7), 7);
    assert_eq!(doc_7.pages[0].spatial_index.len(), 7);

    // Whiteboard custom scale
    let doc_wb_15 = gen.generate_whiteboard(AcademicScale::Custom(15));
    assert_eq!(count_strokes_in_doc(&doc_wb_15), 15);
    assert_eq!(doc_wb_15.pages[0].spatial_index.len(), 15);
}

#[test]
fn test_challenger_distribution_proportions_structural_and_stroke() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc_nb_10k = gen.generate_notebook(AcademicScale::Medium10k);

    let mut cursive_strokes = 0;
    let mut math_strokes = 0;
    let mut diagram_strokes = 0;

    let mut paragraph_blocks = 0;
    let mut math_blocks = 0;
    let mut diagram_blocks = 0;

    for page in &doc_nb_10k.pages {
        let mut prev_type: Option<ContentType> = None;
        for item in &page.items {
            let ItemKind::Stroke(ref s) = item.kind;
            let c_type = classify_stroke(s);
            match c_type {
                ContentType::Handwriting => cursive_strokes += 1,
                ContentType::Math => math_strokes += 1,
                ContentType::Diagram => diagram_strokes += 1,
            }

            // Track block transitions
            if prev_type != Some(c_type) {
                match c_type {
                    ContentType::Handwriting => paragraph_blocks += 1,
                    ContentType::Math => math_blocks += 1,
                    ContentType::Diagram => diagram_blocks += 1,
                }
                prev_type = Some(c_type);
            }
        }
    }

    // Count structural blocks in Notebook:
    // In generate_notebook, every paragraph begins with an indented first line at x = 63.0 (45.0 + 18.0).
    // Math blocks and diagram blocks each begin with their primary symbol / axis stroke.
    let mut detected_paragraphs = 0;
    for page in &doc_nb_10k.pages {
        for item in &page.items {
            let ItemKind::Stroke(ref s) = item.kind;
            if classify_stroke(s) == ContentType::Handwriting
                && !s.is_dot
                && !s.points.is_empty()
                && (s.points[0].x - 63.0).abs() < 1e-3
            {
                detected_paragraphs += 1;
            }
        }
    }

    // Contiguous runs of content type
    let total_runs = (paragraph_blocks + math_blocks + diagram_blocks) as f64;
    let p_run_pct = (paragraph_blocks as f64 / total_runs) * 100.0;
    let m_run_pct = (math_blocks as f64 / total_runs) * 100.0;
    let d_run_pct = (diagram_blocks as f64 / total_runs) * 100.0;

    // Verify all 3 content types are well-represented:
    // Cursive handwriting constitutes the primary bulk (> 85% of strokes)
    // Math formulas are present in substantial academic volume (> 300 strokes, > 3% of strokes)
    // Diagrams are present in substantial academic volume (> 100 strokes, > 1% of strokes)
    assert!(
        (8_001..=9_799).contains(&cursive_strokes),
        "Cursive strokes should dominate (~92%), got {cursive_strokes}"
    );
    assert!(
        (300..=1_200).contains(&math_strokes),
        "Math strokes should be substantial (~5%), got {math_strokes}"
    );
    assert!(
        (100..=600).contains(&diagram_strokes),
        "Diagram strokes should be substantial (~2%), got {diagram_strokes}"
    );

    // Verify block selection / contiguous run proportions:
    // Paragraph runs: 45% - 65% of all contiguous runs (reflecting ~60% generation probability with block coalescing)
    // Math runs: 20% - 35% of all contiguous runs (reflecting ~25% generation probability)
    // Diagram runs: 8% - 22% of all contiguous runs (reflecting ~15% generation probability)
    assert!(
        (45.0..=65.0).contains(&p_run_pct),
        "Paragraph run percentage should reflect ~60% selection, got {p_run_pct:.2}%"
    );
    assert!(
        (20.0..=35.0).contains(&m_run_pct),
        "Math run percentage should reflect ~25% selection, got {m_run_pct:.2}%"
    );
    assert!(
        (8.0..=22.0).contains(&d_run_pct),
        "Diagram run percentage should reflect ~15% selection, got {d_run_pct:.2}%"
    );

    assert!(
        detected_paragraphs >= 200,
        "Expected at least 200 distinct indented paragraph blocks, got {detected_paragraphs}"
    );

    // Stroke count verification: All categories are solidly represented
    assert!(cursive_strokes > 8000, "Cursive strokes: {cursive_strokes}");
    assert!(math_strokes > 300, "Math strokes: {math_strokes}");
    assert!(diagram_strokes > 100, "Diagram strokes: {diagram_strokes}");
}

// ----------------------------------------------------------------------------
// 3. PRE-WARMED CACHE & SPATIAL INDEX COMPLETENESS (1k, 10k, 50k)
// ----------------------------------------------------------------------------

#[test]
fn test_challenger_cache_warmth_and_spatial_index_completeness_all_scales() {
    let mut gen = AcademicDatasetGenerator::new();

    let datasets = [
        ("Notebook 1k", gen.generate_notebook(AcademicScale::Small1k)),
        (
            "Notebook 10k",
            gen.generate_notebook(AcademicScale::Medium10k),
        ),
        (
            "Whiteboard 1k",
            gen.generate_whiteboard(AcademicScale::Small1k),
        ),
        (
            "Whiteboard 10k",
            gen.generate_whiteboard(AcademicScale::Medium10k),
        ),
    ];

    for (desc, doc) in datasets {
        for (p_idx, page) in doc.pages.iter().enumerate() {
            assert_eq!(
                page.spatial_index.len(),
                page.items.len(),
                "[{desc}] Page {p_idx} spatial_index.len() must equal page.items.len()"
            );

            for (i_idx, item) in page.items.iter().enumerate() {
                let ItemKind::Stroke(ref s) = item.kind;
                assert!(
                    s.cached_bbox.is_some(),
                    "[{desc}] Page {p_idx} Stroke {i_idx} (id={}) missing cached_bbox",
                    s.id
                );
                let bbox = s.cached_bbox.unwrap();
                assert!(
                    bbox.width() >= 0.0 && bbox.height() >= 0.0,
                    "[{desc}] Bounding box dimensions must be non-negative"
                );

                if s.is_dot || s.points.len() < 2 {
                    assert!(
                        s.cached_bezier.is_empty(),
                        "[{desc}] Dot strokes must have empty cached_bezier"
                    );
                } else {
                    assert!(
                        !s.cached_bezier.is_empty(),
                        "[{desc}] Page {p_idx} Stroke {i_idx} (id={}) non-dot stroke must have non-empty cached_bezier",
                        s.id
                    );
                }
            }
        }
    }
}

// ----------------------------------------------------------------------------
// 4. GROUND TRUTH ORACLE EQUIVALENCE FOR ENVELOPE QUERIES
// ----------------------------------------------------------------------------

fn manual_bbox_intersects_query(
    bbox: &Rect,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> bool {
    let b_min_x = bbox.min_x;
    let b_max_x = bbox.max_x;
    let b_min_y = bbox.min_y;
    let b_max_y = bbox.max_y;

    let q_min_x = min_x.min(max_x);
    let q_max_x = min_x.max(max_x);
    let q_min_y = min_y.min(max_y);
    let q_max_y = min_y.max(max_y);

    b_min_x <= q_max_x && b_max_x >= q_min_x && b_min_y <= q_max_y && b_max_y >= q_min_y
}

fn linear_scan_oracle(
    page: &Page,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> Vec<(u64, u64)> {
    let mut hits = Vec::new();
    for item in &page.items {
        let ItemKind::Stroke(ref s) = item.kind;
        let bbox = s.bounding_box().expect("cached bbox exists");
        if manual_bbox_intersects_query(&bbox, min_x, min_y, max_x, max_y) {
            hits.push((item.z_index, item.id));
        }
    }
    hits.sort_unstable();
    hits
}

#[test]
fn test_challenger_spatial_index_oracle_equivalence_whiteboard() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Medium10k);
    let page = &doc.pages[0];

    // Diverse test envelopes:
    let query_regions = [
        // 1. Center viewport (1920x1080)
        (-960.0, -540.0, 960.0, 540.0),
        // 2. Off-center quadrant
        (1000.0, 1000.0, 2500.0, 2200.0),
        // 3. Small tight envelope (pinpoint)
        (300.0, 400.0, 350.0, 450.0),
        // 4. Inverted coordinates (min > max) to test normalization
        (500.0, 600.0, -200.0, -100.0),
        // 5. Huge bounding box spanning multiple panels
        (-4000.0, -3000.0, 4000.0, 3000.0),
        // 6. Completely empty outer void region
        (50000.0, 50000.0, 60000.0, 60000.0),
    ];

    for (idx, &(q_x1, q_y1, q_x2, q_y2)) in query_regions.iter().enumerate() {
        let oracle_hits = linear_scan_oracle(page, q_x1, q_y1, q_x2, q_y2);
        let spatial_hits = page
            .spatial_index
            .locate_in_envelope(q_x1, q_y1, q_x2, q_y2);

        let spatial_tuples: Vec<(u64, u64)> = spatial_hits
            .iter()
            .map(|it| (it.z_index, it.item_id))
            .collect();

        assert_eq!(
            spatial_tuples, oracle_hits,
            "Region {idx} ([{q_x1}, {q_y1}]..[{q_x2}, {q_y2}]) spatial results must 100% match Oracle"
        );
    }
}

#[test]
fn test_challenger_spatial_index_oracle_equivalence_notebook() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_notebook(AcademicScale::Small1k);

    // Test oracle equivalence on every page of the notebook
    for (p_idx, page) in doc.pages.iter().enumerate() {
        let queries = [
            // Top of page header
            (0.0, 0.0, 595.0, 150.0),
            // Middle section
            (100.0, 300.0, 500.0, 500.0),
            // Bottom margin
            (0.0, 700.0, 595.0, 842.0),
            // Out of bounds margin (should be empty or edge items)
            (600.0, 0.0, 1000.0, 842.0),
        ];

        for (q_idx, &(x1, y1, x2, y2)) in queries.iter().enumerate() {
            let oracle_hits = linear_scan_oracle(page, x1, y1, x2, y2);
            let spatial_hits = page.spatial_index.locate_in_envelope(x1, y1, x2, y2);

            let spatial_tuples: Vec<(u64, u64)> = spatial_hits
                .iter()
                .map(|it| (it.z_index, it.item_id))
                .collect();

            assert_eq!(
                spatial_tuples, oracle_hits,
                "Page {p_idx} Query {q_idx} spatial results must 100% match Oracle"
            );
        }
    }
}

#[test]
fn test_challenger_spatial_index_culling_efficiency_and_selectivity() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Medium10k);
    let page = &doc.pages[0];

    // Standard 1080p viewport at 100% zoom centered at (0, 0)
    let visible = page
        .spatial_index
        .locate_in_envelope(-960.0, -540.0, 960.0, 540.0);
    let total = page.items.len();
    assert_eq!(total, 10_000);

    let culled_count = total - visible.len();
    let culled_pct = (culled_count as f64 / total as f64) * 100.0;

    // Viewport culling must eliminate > 90% of strokes on Whiteboard
    assert!(
        culled_pct > 90.0,
        "Whiteboard 10k viewport culling must exceed 90%, got {culled_pct:.2}%"
    );
    assert!(
        !visible.is_empty(),
        "Visible strokes must be non-empty in populated central region"
    );
}

#[test]
fn test_challenger_stroke_id_and_z_index_monotonicity() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_notebook(AcademicScale::Small1k);

    let mut last_id = 0;
    for page in &doc.pages {
        let mut last_z = 0;
        for item in &page.items {
            assert!(item.id > last_id, "Item IDs must be strictly monotonic");
            last_id = item.id;

            assert!(
                item.z_index > last_z,
                "z_index must be strictly monotonic within page"
            );
            last_z = item.z_index;
        }
    }
}

// ----------------------------------------------------------------------------
// 5. ADVERSARIAL STRESS & GEOMETRIC INVARIANT TESTS
// ----------------------------------------------------------------------------

#[test]
fn test_challenger_adversarial_bounding_box_tightness() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_notebook(AcademicScale::Small1k);

    for page in &doc.pages {
        for item in &page.items {
            let ItemKind::Stroke(ref s) = item.kind;
            let cached = s.cached_bbox.expect("cached_bbox must be Some");

            // Compute ground-truth bounding box manually from points
            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;

            for pt in &s.points {
                let r = pt.width / 2.0;
                min_x = min_x.min(pt.x - r);
                min_y = min_y.min(pt.y - r);
                max_x = max_x.max(pt.x + r);
                max_y = max_y.max(pt.y + r);
            }

            assert!(
                (cached.min_x - min_x).abs() < 1e-9,
                "cached min_x mismatch: {} vs {}",
                cached.min_x,
                min_x
            );
            assert!(
                (cached.min_y - min_y).abs() < 1e-9,
                "cached min_y mismatch: {} vs {}",
                cached.min_y,
                min_y
            );
            assert!(
                (cached.max_x - max_x).abs() < 1e-9,
                "cached max_x mismatch: {} vs {}",
                cached.max_x,
                max_x
            );
            assert!(
                (cached.max_y - max_y).abs() < 1e-9,
                "cached max_y mismatch: {} vs {}",
                cached.max_y,
                max_y
            );
        }
    }
}

#[test]
fn test_challenger_adversarial_catmull_rom_bezier_continuity() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_notebook(AcademicScale::Small1k);

    for page in &doc.pages {
        for item in &page.items {
            let ItemKind::Stroke(ref s) = item.kind;
            if s.is_dot || s.points.len() < 2 {
                continue;
            }

            // Verify G0 continuity of Bézier spline segments
            let beziers = &s.cached_bezier;
            assert!(
                !beziers.is_empty(),
                "Non-dot multi-point stroke must have cached Bézier segments"
            );

            for i in 0..(beziers.len().saturating_sub(1)) {
                let curr = &beziers[i];
                let next = &beziers[i + 1];

                let dx = (curr.p1.0 - next.p0.0).abs();
                let dy = (curr.p1.1 - next.p0.1).abs();
                let dw = (curr.width1 - next.width0).abs();

                assert!(
                    dx < 1e-6 && dy < 1e-6,
                    "G0 positional discontinuity at joint {i}: curr.p1=({:.4}, {:.4}), next.p0=({:.4}, {:.4})",
                    curr.p1.0, curr.p1.1, next.p0.0, next.p0.1
                );
                assert!(
                    dw < 1e-6,
                    "Width discontinuity at joint {i}: curr.width1={}, next.width0={}",
                    curr.width1,
                    next.width0
                );
            }
        }
    }
}

#[test]
fn test_challenger_adversarial_spatial_index_degenerate_queries() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Small1k);
    let page = &doc.pages[0];

    // Degenerate queries that must be safely handled without panic
    let degenerate_queries = [
        (f64::NAN, 0.0, 100.0, 100.0),
        (0.0, f64::NAN, 100.0, 100.0),
        (f64::INFINITY, 0.0, 100.0, 100.0),
        (0.0, f64::NEG_INFINITY, 100.0, 100.0),
        (100.0, 100.0, 100.0, 100.0), // Zero-area point query
        (500.0, 500.0, 100.0, 100.0), // Inverted diagonal
        (-1e15, -1e15, 1e15, 1e15),   // Astronomical bounding box
    ];

    for &(x1, y1, x2, y2) in &degenerate_queries {
        let results = page.spatial_index.locate_in_envelope(x1, y1, x2, y2);
        // Verify results are sorted by z_index
        for w in results.windows(2) {
            assert!(
                w[0].z_index <= w[1].z_index,
                "Results must remain sorted by z_index even under degenerate queries"
            );
        }
    }
}

#[test]
fn test_challenger_adversarial_50k_dataset_spatial_query_stress() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Massive50k);
    assert_eq!(count_strokes_in_doc(&doc), 50_000);

    let page = &doc.pages[0];
    assert_eq!(page.spatial_index.len(), 50_000);

    // Stress query envelopes across the 50k Whiteboard
    let stress_queries = [
        // Center panel (origin)
        (-600.0, -450.0, 600.0, 450.0),
        // Extreme northwest panel
        (-15000.0, -15000.0, -12000.0, -12000.0),
        // Extreme southeast panel
        (12000.0, 12000.0, 15000.0, 15000.0),
        // Horizontal band crossing multiple panels
        (-8000.0, -200.0, 8000.0, 200.0),
        // Narrow vertical slit
        (-100.0, -8000.0, 100.0, 8000.0),
        // Disjoint void query
        (90000.0, 90000.0, 100000.0, 100000.0),
    ];

    for (idx, &(q_x1, q_y1, q_x2, q_y2)) in stress_queries.iter().enumerate() {
        let spatial_hits = page
            .spatial_index
            .locate_in_envelope(q_x1, q_y1, q_x2, q_y2);
        let oracle_hits = linear_scan_oracle(page, q_x1, q_y1, q_x2, q_y2);

        let spatial_tuples: Vec<(u64, u64)> = spatial_hits
            .iter()
            .map(|it| (it.z_index, it.item_id))
            .collect();

        assert_eq!(
            spatial_tuples, oracle_hits,
            "50k stress query {idx} spatial results must 100% match linear scan Oracle"
        );

        // Verify strict ascending order
        for w in spatial_hits.windows(2) {
            assert!(
                w[0].z_index <= w[1].z_index,
                "50k query {idx} results must be sorted by z_index ascending"
            );
        }
    }
}
