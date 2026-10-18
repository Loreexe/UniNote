//! Empirical Adversarial Stress Test Suite for Milestone M4 (Challenger M4-2)
//!
//! Scope:
//! 1. Eraser Latency Benchmark on 10,000 strokes across 500 simulated touch/motion events (< 0.5 ms latency target < 0.05 ms).
//! 2. Selection Pre-Filtering Benchmark on AppSelectionEngine::select_rect and select_lasso on 10,000 strokes (< 2.0 ms latency, 100% agreement with linear reference oracle).
//! 3. Command Lifecycle & Undo/Redo Consistency Stress Test: 1,000 randomized interleaved cycles of AddStroke, MoveItems (small & bulk), EraseItems, Undo, Redo.
//!    Invariant: `page.spatial_index.len() == page.items.len()` and spatial query completeness after every single operation.

use std::collections::HashSet;
use std::time::Instant;
use uuid::Uuid;

use uninote_app::selection::AppSelectionEngine;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, Item, ItemId, ItemKind};
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, SelectionEngine,
};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// DETERMINISTIC PSEUDO-RANDOM NUMBER GENERATOR (LCG)
// ============================================================================

struct LcgPrng {
    state: u64,
}

impl LcgPrng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0x853c49e6748fea9b } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_f64()
    }

    fn range_usize(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + ((self.next_u64() >> 32) as usize % (max - min + 1))
    }
}

// ============================================================================
// SYNTHETIC STROKE & DOCUMENT GENERATORS
// ============================================================================

/// Generates a realistic handwriting stroke with pre-populated geometric cache.
fn generate_test_stroke(prng: &mut LcgPrng, id: u64, origin_x: f64, origin_y: f64) -> Stroke {
    let point_count = prng.range_usize(8, 20);
    let base_width = prng.range_f64(1.5, 3.0);
    let mut points = Vec::with_capacity(point_count);

    let mut cur_x = origin_x;
    let mut cur_y = origin_y;

    for i in 0..point_count {
        let t = i as f64 / (point_count - 1).max(1) as f64;
        let pressure = (0.4 + 0.5 * (t * std::f64::consts::PI).sin()).clamp(0.1, 1.0);
        points.push(StrokePoint {
            x: cur_x,
            y: cur_y,
            pressure,
            width: base_width * pressure,
            timestamp_ns: (i as u64) * 5_000_000,
        });

        cur_x += prng.range_f64(1.0, 4.0);
        cur_y += (t * std::f64::consts::PI * 2.0).sin() * 4.0 + prng.range_f64(-1.0, 1.0);
    }

    let mut stroke = Stroke {
        id,
        style: StrokeStyle {
            color: (0.1, 0.12, 0.15, 1.0),
            base_width,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
        },
        points,
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

/// Generates a 10,000-stroke document on an A4 page with a fully built R-Tree spatial index.
fn generate_10k_stroke_document(seed: u64) -> (Document, Uuid) {
    let mut prng = LcgPrng::new(seed);
    let mut doc = Document::new_empty("M4 Challenger 10k Benchmark");
    let page_id = doc.pages[0].id;

    // Distribute 10,000 strokes in 200 lines of 50 strokes across A4 canvas (595 x 842 pt)
    let page = &mut doc.pages[0];
    page.items.reserve(10_000);

    for i in 1..=10_000 {
        let line_idx = (i - 1) / 50;
        let col_idx = (i - 1) % 50;

        let origin_x = 30.0 + (col_idx as f64) * 10.5 + prng.range_f64(-1.5, 1.5);
        let origin_y = 40.0 + (line_idx as f64) * 3.8 + prng.range_f64(-0.5, 0.5);

        let stroke = generate_test_stroke(&mut prng, i as u64, origin_x, origin_y);
        page.items.push(Item {
            id: i as u64,
            z_index: i as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    page.rebuild_spatial_index();
    assert_eq!(page.items.len(), 10_000);
    assert_eq!(page.spatial_index.len(), 10_000);

    (doc, page_id)
}

// ============================================================================
// ERASER ALGORITHM HARNESS (Exact CanvasState::erase_strokes_at logic)
// ============================================================================

/// Executes the exact R-Tree accelerated eraser routine as implemented in canvas.rs:2376.
fn execute_eraser_at(
    doc: &mut Document,
    history: &mut CommandHistory,
    page_id: Uuid,
    doc_pt: (f64, f64),
    radius: f64,
) -> usize {
    if !radius.is_finite() || radius <= 0.0 || !doc_pt.0.is_finite() || !doc_pt.1.is_finite() {
        return 0;
    }

    let page = match doc.get_page_mut(page_id) {
        Some(p) => p,
        None => return 0,
    };

    if page.spatial_index.is_empty() && !page.items.is_empty() {
        page.rebuild_spatial_index();
    }

    // 1. Envelope pre-filtering via R-Tree in O(log N + K)
    let min_x = doc_pt.0 - radius;
    let min_y = doc_pt.1 - radius;
    let max_x = doc_pt.0 + radius;
    let max_y = doc_pt.1 + radius;

    let candidates = page
        .spatial_index
        .locate_in_envelope(min_x, min_y, max_x, max_y);
    if candidates.is_empty() {
        return 0;
    }

    // 2. Candidate point-to-stroke distance testing
    let mut erased = Vec::new();
    if candidates.len() == 1 {
        let target_id = candidates[0].item_id;
        if let Some(item) = page.items.iter().find(|it| it.id == target_id) {
            let ItemKind::Stroke(ref s) = item.kind;
            if s.intersects_point(doc_pt, radius) {
                erased.push(item.clone());
            }
        }
    } else {
        let candidate_ids: HashSet<u64> = candidates.iter().map(|c| c.item_id).collect();
        for item in &page.items {
            if candidate_ids.contains(&item.id) {
                let ItemKind::Stroke(ref s) = item.kind;
                if s.intersects_point(doc_pt, radius) {
                    erased.push(item.clone());
                }
            }
        }
    }

    let erased_count = erased.len();
    if !erased.is_empty() {
        history.execute(
            DocCommand::EraseItems {
                page_id,
                erased_items: erased,
            },
            doc,
        );
    }

    erased_count
}

// ============================================================================
// 1. ERASER LATENCY BENCHMARK (< 0.5 ms per event, target < 0.05 ms)
// ============================================================================

#[test]
fn test_benchmark_eraser_latency_10k_strokes_500_events() {
    let (mut doc, page_id) = generate_10k_stroke_document(42);
    let mut history = CommandHistory::new(100);
    let mut prng = LcgPrng::new(999);

    let num_events = 500;
    let mut latencies_us = Vec::with_capacity(num_events);
    let mut whitespace_events = 0;
    let mut dense_events = 0;
    let mut total_erased = 0;

    for i in 0..num_events {
        let (doc_pt, radius) = if i % 2 == 0 {
            // Whitespace event: outside drawing area or in empty margin
            whitespace_events += 1;
            let x = if prng.next_f64() < 0.5 {
                prng.range_f64(0.0, 25.0) // Left margin
            } else {
                prng.range_f64(560.0, 595.0) // Right margin
            };
            let y = prng.range_f64(10.0, 830.0);
            ((x, y), 12.0)
        } else {
            // Dense intersection event: inside active handwritten area
            dense_events += 1;
            let x = prng.range_f64(50.0, 500.0);
            let y = prng.range_f64(50.0, 780.0);
            ((x, y), 14.0)
        };

        let start = Instant::now();
        let erased = execute_eraser_at(&mut doc, &mut history, page_id, doc_pt, radius);
        let elapsed = start.elapsed();
        let us = elapsed.as_micros() as f64;

        latencies_us.push(us);
        total_erased += erased;
    }

    latencies_us.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min_us = latencies_us[0];
    let max_us = latencies_us[latencies_us.len() - 1];
    let sum_us: f64 = latencies_us.iter().sum();
    let avg_us = sum_us / (num_events as f64);
    let p50_us = latencies_us[num_events * 50 / 100];
    let p95_us = latencies_us[num_events * 95 / 100];
    let p99_us = latencies_us[num_events * 99 / 100];

    let avg_ms = avg_us / 1000.0;
    let max_ms = max_us / 1000.0;
    let p99_ms = p99_us / 1000.0;

    println!(
        "--- ERASER LATENCY BENCHMARK (10,000 strokes, 500 events) ---
Total Events: {} (Whitespace: {}, Dense: {})
Total Erased Strokes: {}
Remaining Strokes: {}
Latency:
  Min: {:8.2} µs ({:.4} ms)
  P50: {:8.2} µs ({:.4} ms)
  P95: {:8.2} µs ({:.4} ms)
  P99: {:8.2} µs ({:.4} ms)
  Max: {:8.2} µs ({:.4} ms)
  Avg: {:8.2} µs ({:.4} ms)
------------------------------------------------------------",
        num_events,
        whitespace_events,
        dense_events,
        total_erased,
        doc.pages[0].items.len(),
        min_us,
        min_us / 1000.0,
        p50_us,
        p50_us / 1000.0,
        p95_us,
        p95_us / 1000.0,
        p99_us,
        p99_ms,
        max_us,
        max_ms,
        avg_us,
        avg_ms
    );

    // Assert strictly against Milestone M4 performance budgets (< 0.5 ms in release)
    let max_allowed_avg_ms = if cfg!(debug_assertions) { 3.0 } else { 0.5 };
    let max_allowed_p99_ms = if cfg!(debug_assertions) { 10.0 } else { 0.5 };

    assert!(
        avg_ms < max_allowed_avg_ms,
        "Average eraser latency ({:.4} ms) must be < {:.2} ms",
        avg_ms,
        max_allowed_avg_ms
    );
    assert!(
        p99_ms < max_allowed_p99_ms,
        "P99 eraser latency ({:.4} ms) must be < {:.2} ms",
        p99_ms,
        max_allowed_p99_ms
    );

    // In release mode, verify tight target compliance
    #[cfg(not(debug_assertions))]
    assert!(
        avg_ms < 0.20,
        "Release average eraser latency ({:.4} ms) exceeds 0.20 ms target",
        avg_ms
    );

    // Invariant: spatial index and items must be strictly synchronized
    let page = &doc.pages[0];
    assert_eq!(page.spatial_index.len(), page.items.len());
}

// ============================================================================
// 2. SELECTION ENGINE BENCHMARK & 100% LINEAR ORACLE AGREEMENT (< 2.0 ms)
// ============================================================================

#[test]
fn test_benchmark_selection_rect_10k_strokes_oracle_agreement() {
    let (doc, _) = generate_10k_stroke_document(101);
    let page = &doc.pages[0];

    // Varied rectangular queries: whitespace, small, medium, large, full page, degenerate
    let query_rectangles = vec![
        // Whitespace margins (0 strokes expected)
        Rect::new(0.0, 0.0, 25.0, 800.0),
        Rect::new(560.0, 0.0, 595.0, 800.0),
        Rect::new(0.0, 0.0, 595.0, 30.0),
        // Small detail boxes (1 to ~20 strokes)
        Rect::new(50.0, 50.0, 80.0, 70.0),
        Rect::new(120.0, 200.0, 160.0, 220.0),
        Rect::new(300.0, 400.0, 340.0, 420.0),
        Rect::new(450.0, 600.0, 490.0, 620.0),
        // Medium boxes (50 to ~300 strokes)
        Rect::new(50.0, 100.0, 250.0, 200.0),
        Rect::new(200.0, 300.0, 450.0, 450.0),
        Rect::new(100.0, 500.0, 400.0, 650.0),
        // Inverted rectangle coordinates (requires normalization)
        Rect::new(250.0, 200.0, 50.0, 100.0),
        // Degenerate rectangles (< 2x2 pt)
        Rect::new(100.0, 100.0, 101.0, 101.0),
        Rect::new(50.0, 50.0, 50.0, 50.0),
        // Large box (~1,000 to 3,000 strokes)
        Rect::new(40.0, 100.0, 500.0, 500.0),
        // Entire page (all 10,000 strokes)
        Rect::new(0.0, 0.0, 600.0, 850.0),
    ];

    println!("--- RECTANGULAR SELECTION BENCHMARK & ORACLE VERIFICATION (10,000 strokes) ---");

    for (idx, query_rect) in query_rectangles.iter().enumerate() {
        // 1. AppSelectionEngine (R-Tree pre-filtered)
        let start = Instant::now();
        let (accel_ids, accel_bbox) = AppSelectionEngine::select_rect(page, query_rect);
        let elapsed = start.elapsed();
        let accel_ms = elapsed.as_secs_f64() * 1000.0;

        // 2. Linear Reference Oracle (Linear O(N) scan over all 10,000 strokes)
        let norm_rect = query_rect.normalized();
        let mut oracle_ids = Vec::new();
        let mut oracle_strokes = Vec::new();

        if norm_rect.width() >= 2.0 || norm_rect.height() >= 2.0 {
            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_rect(s, &norm_rect) {
                    oracle_ids.push(item.id);
                    oracle_strokes.push(s);
                }
            }
        }
        let oracle_bbox = SelectionEngine::compute_selection_bounding_box(oracle_strokes);

        println!(
            "Query {:2}: Box [{:5.1},{:5.1} -> {:5.1},{:5.1}] | Selected: {:5} | Latency: {:6.3} ms",
            idx + 1,
            norm_rect.min_x,
            norm_rect.min_y,
            norm_rect.max_x,
            norm_rect.max_y,
            accel_ids.len(),
            accel_ms
        );

        // Verification: Latency MUST be strictly < 2.0 ms in release mode
        let max_allowed_ms = if cfg!(debug_assertions) { 15.0 } else { 2.0 };
        assert!(
            accel_ms < max_allowed_ms,
            "Query {}: selection latency ({:.3} ms) exceeded {:.1} ms threshold",
            idx + 1,
            accel_ms,
            max_allowed_ms
        );

        // Verification: 100% Agreement with Linear Oracle (zero false positives, zero false negatives)
        assert_eq!(
            accel_ids,
            oracle_ids,
            "Query {}: 100% agreement violation! Selected IDs differed from oracle",
            idx + 1
        );
        assert_eq!(
            accel_bbox,
            oracle_bbox,
            "Query {}: Bounding box differed from oracle",
            idx + 1
        );
    }
}

#[test]
fn test_benchmark_selection_lasso_10k_strokes_oracle_agreement() {
    let (doc, _) = generate_10k_stroke_document(202);
    let page = &doc.pages[0];

    // Varied lasso polygons: whitespace, triangle, pentagon, concave star, dense cluster
    let test_polygons: Vec<Vec<(f64, f64)>> = vec![
        // 1. Whitespace triangle (0 strokes)
        vec![(5.0, 10.0), (25.0, 10.0), (15.0, 50.0)],
        // 2. Small triangle (1 to 10 strokes)
        vec![(50.0, 50.0), (90.0, 50.0), (70.0, 75.0)],
        // 3. Convex quadrilateral around 1 line of text (~15 strokes)
        vec![(45.0, 80.0), (180.0, 80.0), (180.0, 95.0), (45.0, 95.0)],
        // 4. Convex pentagon enclosing moderate cluster (~40 strokes)
        vec![
            (100.0, 150.0),
            (220.0, 130.0),
            (250.0, 200.0),
            (180.0, 240.0),
            (90.0, 210.0),
        ],
        // 5. Concave polygon (C-shape or L-shape)
        vec![
            (100.0, 300.0),
            (300.0, 300.0),
            (300.0, 350.0),
            (160.0, 350.0),
            (160.0, 450.0),
            (100.0, 450.0),
        ],
        // 6. Concave star polygon
        vec![
            (250.0, 400.0),
            (270.0, 450.0),
            (320.0, 450.0),
            (280.0, 480.0),
            (300.0, 530.0),
            (250.0, 500.0),
            (200.0, 530.0),
            (220.0, 480.0),
            (180.0, 450.0),
            (230.0, 450.0),
        ],
        // 7. Degenerate polygons (< 3 vertices: 0 and 2 points)
        vec![],
        vec![(100.0, 100.0), (200.0, 200.0)],
        // 8. Large polygon enclosing dense multi-paragraph region (~900 strokes)
        vec![(40.0, 200.0), (320.0, 200.0), (330.0, 420.0), (50.0, 410.0)],
        // 9. Extreme half-page stress polygon (~3,600 strokes)
        vec![(40.0, 200.0), (500.0, 200.0), (520.0, 550.0), (50.0, 530.0)],
    ];

    println!("--- LASSO SELECTION BENCHMARK & ORACLE VERIFICATION (10,000 strokes) ---");

    for (idx, poly) in test_polygons.iter().enumerate() {
        // 1. AppSelectionEngine (R-Tree pre-filtered)
        let start = Instant::now();
        let (accel_ids, accel_bbox) = AppSelectionEngine::select_lasso(page, poly);
        let elapsed = start.elapsed();
        let accel_ms = elapsed.as_secs_f64() * 1000.0;

        // 2. Linear Reference Oracle (Linear O(N) ray-casting on all 10,000 strokes)
        let mut oracle_ids = Vec::new();
        let mut oracle_strokes = Vec::new();

        if poly.len() >= 3 {
            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_lasso(s, poly) {
                    oracle_ids.push(item.id);
                    oracle_strokes.push(s);
                }
            }
        }
        let oracle_bbox = SelectionEngine::compute_selection_bounding_box(oracle_strokes);

        println!(
            "Lasso {:2}: Vertices: {:2} | Selected: {:5} | Latency: {:6.3} ms",
            idx + 1,
            poly.len(),
            accel_ids.len(),
            accel_ms
        );

        // Verification: Latency MUST be strictly < 2.0 ms for queries 1..=8, and < 3.0 ms for extreme half-page query
        let max_allowed_ms = if idx == 9 {
            if cfg!(debug_assertions) {
                15.0
            } else {
                3.0
            }
        } else if cfg!(debug_assertions) {
            10.0
        } else {
            2.0
        };
        assert!(
            accel_ms < max_allowed_ms,
            "Lasso {}: selection latency ({:.3} ms) exceeded {:.1} ms threshold",
            idx + 1,
            accel_ms,
            max_allowed_ms
        );

        // Verification: 100% Agreement with Linear Oracle
        assert_eq!(
            accel_ids,
            oracle_ids,
            "Lasso {}: 100% agreement violation! Selected IDs differed from oracle",
            idx + 1
        );
        assert_eq!(
            accel_bbox,
            oracle_bbox,
            "Lasso {}: Bounding box differed from oracle",
            idx + 1
        );
    }
}

// ============================================================================
// 3. COMMAND LIFECYCLE & UNDO/REDO RANDOMIZED STRESS TEST (1,000 CYCLES)
// ============================================================================

#[test]
fn test_stress_command_history_1000_cycles_spatial_index_bijection_and_completeness() {
    let mut prng = LcgPrng::new(54321);
    let mut doc = Document::new_empty("Command Lifecycle Stress Test");
    let page_id = doc.pages[0].id;

    // Use a large history depth so commands can be deeply undone and redone
    let mut history = CommandHistory::new(300);

    // Initial pre-population with 60 strokes
    for i in 1..=60 {
        let stroke = generate_test_stroke(&mut prng, i, (i as f64) * 8.0, 100.0);
        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    }

    assert_eq!(doc.pages[0].items.len(), 60);
    assert_eq!(doc.pages[0].spatial_index.len(), 60);

    let total_cycles = 1000;
    let mut stroke_counter = 1000u64;

    let mut add_count = 0;
    let mut move_small_count = 0;
    let mut move_bulk_count = 0;
    let mut erase_count = 0;
    let mut undo_count = 0;
    let mut redo_count = 0;

    println!("--- EXECUTING 1,000 RANDOMIZED INTERLEAVED COMMAND CYCLES ---");

    for cycle in 1..=total_cycles {
        let op_choice = prng.range_usize(0, 5);

        match op_choice {
            0 => {
                // AddStroke (25% weight)
                stroke_counter += 1;
                let sx = prng.range_f64(30.0, 500.0);
                let sy = prng.range_f64(30.0, 800.0);
                let stroke = generate_test_stroke(&mut prng, stroke_counter, sx, sy);
                history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
                add_count += 1;
            }
            1 => {
                // MoveItems Small (1 to 10 items) (20% weight)
                let page = &doc.pages[0];
                if !page.items.is_empty() {
                    let move_len = prng.range_usize(1, 10).min(page.items.len());
                    let mut chosen_ids = Vec::with_capacity(move_len);
                    for _ in 0..move_len {
                        let idx = prng.range_usize(0, page.items.len() - 1);
                        chosen_ids.push(page.items[idx].id);
                    }
                    chosen_ids.dedup();

                    let delta_x = prng.range_f64(-15.0, 15.0);
                    let delta_y = prng.range_f64(-15.0, 15.0);

                    history.execute(
                        DocCommand::MoveItems {
                            page_id,
                            item_ids: chosen_ids,
                            delta_x,
                            delta_y,
                        },
                        &mut doc,
                    );
                    move_small_count += 1;
                }
            }
            2 => {
                // MoveItems Bulk (> 250 threshold triggering rebuild_spatial_index if possible)
                let page = &doc.pages[0];
                if page.items.len() > 10 {
                    // Pick either all items or a large fraction
                    let count_to_pick = if page.items.len() > 260 {
                        prng.range_usize(251, page.items.len())
                    } else {
                        page.items.len()
                    };

                    let chosen_ids: Vec<ItemId> = page
                        .items
                        .iter()
                        .take(count_to_pick)
                        .map(|it| it.id)
                        .collect();

                    let delta_x = prng.range_f64(-5.0, 5.0);
                    let delta_y = prng.range_f64(-5.0, 5.0);

                    history.execute(
                        DocCommand::MoveItems {
                            page_id,
                            item_ids: chosen_ids,
                            delta_x,
                            delta_y,
                        },
                        &mut doc,
                    );
                    move_bulk_count += 1;
                }
            }
            3 => {
                // EraseItems (1 to 15 items)
                let page = &doc.pages[0];
                if !page.items.is_empty() {
                    let erase_len = prng.range_usize(1, 15).min(page.items.len());
                    let mut erased = Vec::with_capacity(erase_len);
                    for _ in 0..erase_len {
                        let idx = prng.range_usize(0, page.items.len() - 1);
                        let it = page.items[idx].clone();
                        if !erased.iter().any(|e: &Item| e.id == it.id) {
                            erased.push(it);
                        }
                    }

                    if !erased.is_empty() {
                        history.execute(
                            DocCommand::EraseItems {
                                page_id,
                                erased_items: erased,
                            },
                            &mut doc,
                        );
                        erase_count += 1;
                    }
                }
            }
            4 => {
                // Undo
                if history.can_undo() {
                    let undone = history.undo(&mut doc);
                    assert!(undone);
                    undo_count += 1;
                }
            }
            5 => {
                // Redo
                if history.can_redo() {
                    let redone = history.redo(&mut doc);
                    assert!(redone);
                    redo_count += 1;
                }
            }
            _ => unreachable!(),
        }

        // ====================================================================
        // INVARIANT CHECKS (Enforced after EVERY single operation)
        // ====================================================================
        let page = &doc.pages[0];

        // Invariant 1: Exact bijection length equality
        assert_eq!(
            page.spatial_index.len(),
            page.items.len(),
            "Cycle {}: Invariant failed! spatial_index.len() ({}) != page.items.len() ({})",
            cycle,
            page.spatial_index.len(),
            page.items.len()
        );

        // Invariant 2: Spatial query completeness
        let spatial_hits = page.spatial_index.locate_in_envelope(-1e9, -1e9, 1e9, 1e9);

        assert_eq!(
            spatial_hits.len(),
            page.items.len(),
            "Cycle {}: locate_in_envelope returned {} items, expected {}",
            cycle,
            spatial_hits.len(),
            page.items.len()
        );

        let spatial_id_set: HashSet<u64> = spatial_hits.iter().map(|it| it.item_id).collect();
        let item_id_set: HashSet<u64> = page.items.iter().map(|it| it.id).collect();

        assert_eq!(
            spatial_id_set, item_id_set,
            "Cycle {}: Spatial index IDs do not match page.items IDs (missing or ghost items)!",
            cycle
        );

        // Spot-check individual bounding box queries for 5 random items
        if !page.items.is_empty() {
            for _ in 0..5 {
                let rand_idx = prng.range_usize(0, page.items.len() - 1);
                let item = &page.items[rand_idx];
                let ItemKind::Stroke(ref s) = item.kind;
                if let Some(bbox) = s.bounding_box() {
                    let query_result = page.spatial_index.locate_in_envelope(
                        bbox.min_x - 1.0,
                        bbox.min_y - 1.0,
                        bbox.max_x + 1.0,
                        bbox.max_y + 1.0,
                    );
                    assert!(
                        query_result.iter().any(|si| si.item_id == item.id),
                        "Cycle {}: Item {} not found within its own bounding box!",
                        cycle,
                        item.id
                    );
                }
            }
        }
    }

    println!(
        "--- 1,000 CYCLES COMPLETED SUCCESSFULLY ---
Operations Executed:
  AddStroke:        {:4}
  MoveItems (Small):{:4}
  MoveItems (Bulk): {:4}
  EraseItems:       {:4}
  Undo:             {:4}
  Redo:             {:4}
Final Page Items:   {:4}
Final Spatial Len:  {:4}
---------------------------------------------",
        add_count,
        move_small_count,
        move_bulk_count,
        erase_count,
        undo_count,
        redo_count,
        doc.pages[0].items.len(),
        doc.pages[0].spatial_index.len()
    );
}

// ============================================================================
// 4. ADVERSARIAL EDGE CASES & STABILITY TESTS
// ============================================================================

#[test]
fn test_adversarial_bulk_move_threshold_boundary_crossing() {
    let mut doc = Document::new_empty("Threshold Boundary Test");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);
    let mut prng = LcgPrng::new(777);

    // Populate exactly 300 strokes
    for i in 1..=300 {
        let stroke = generate_test_stroke(&mut prng, i, (i as f64) * 2.0, 100.0);
        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    }

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 300);
    assert_eq!(page.spatial_index.len(), 300);

    // Case 1: Exactly 249 items moved (individual remove/insert path)
    let ids_249: Vec<ItemId> = page.items.iter().take(249).map(|it| it.id).collect();
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: ids_249,
            delta_x: 10.0,
            delta_y: 10.0,
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);

    // Case 2: Exactly 250 items moved (exact threshold)
    let ids_250: Vec<ItemId> = doc.pages[0]
        .items
        .iter()
        .take(250)
        .map(|it| it.id)
        .collect();
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: ids_250,
            delta_x: -5.0,
            delta_y: -5.0,
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);

    // Case 3: Exactly 251 items moved (bulk rebuild path)
    let ids_251: Vec<ItemId> = doc.pages[0]
        .items
        .iter()
        .take(251)
        .map(|it| it.id)
        .collect();
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: ids_251,
            delta_x: 20.0,
            delta_y: 20.0,
        },
        &mut doc,
    );
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);

    // Undo all three moves sequentially
    history.undo(&mut doc); // Undoes 251 (bulk revert)
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);

    history.undo(&mut doc); // Undoes 250 (individual revert)
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);

    history.undo(&mut doc); // Undoes 249 (individual revert)
    assert_eq!(doc.pages[0].spatial_index.len(), 300);
    assert_eq!(doc.pages[0].items.len(), 300);
}

#[test]
fn test_adversarial_eraser_non_finite_and_empty_guard() {
    let mut doc = Document::new_empty("Eraser Guard Test");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(10);
    let mut prng = LcgPrng::new(888);

    for i in 1..=20 {
        let stroke = generate_test_stroke(&mut prng, i, (i as f64) * 10.0, 100.0);
        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    }
    assert_eq!(doc.pages[0].items.len(), 20);

    // Test non-finite, zero, and negative inputs to eraser
    assert_eq!(
        execute_eraser_at(&mut doc, &mut history, page_id, (f64::NAN, 100.0), 10.0),
        0
    );
    assert_eq!(
        execute_eraser_at(
            &mut doc,
            &mut history,
            page_id,
            (100.0, f64::INFINITY),
            10.0
        ),
        0
    );
    assert_eq!(
        execute_eraser_at(&mut doc, &mut history, page_id, (100.0, 100.0), 0.0),
        0
    );
    assert_eq!(
        execute_eraser_at(&mut doc, &mut history, page_id, (100.0, 100.0), -10.0),
        0
    );
    assert_eq!(
        execute_eraser_at(&mut doc, &mut history, page_id, (100.0, 100.0), f64::NAN),
        0
    );

    // Items must remain fully intact and synchronized
    assert_eq!(doc.pages[0].items.len(), 20);
    assert_eq!(doc.pages[0].spatial_index.len(), 20);
}
