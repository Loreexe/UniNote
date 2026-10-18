//! UniNote M5 — Challenger Empirical Adversarial Stress Testing Suite
//!
//! Independent stress testing under authoritative requirements R1-R6:
//! 1. Adversarial Viewport Navigation & Cairo Stress:
//!    - Continuous pan & zoom with extreme zoom levels (0.01x up to 100.0x) on 10k and 50k strokes.
//!    - Rendering time stays < 8.0 ms under culling, appropriate SLA under extreme zoom-out, no panics.
//! 2. Memory Leak Extended Stress:
//!    - 2,000 continuous frames of randomized pan/zoom on 10,000 strokes.
//!    - Track exact thread heap allocations and assert net leak == 0 bytes.
//! 3. Adversarial Eraser Under 1,000 Queries:
//!    - 1,000 random points across dense clusters, empty margins, and degenerate parameters.
//!    - Assert latency < 0.5 ms for every single event and spatial index consistency.
//! 4. Adversarial Selection Under 1,000 Queries:
//!    - Degenerate lasso polygons (self-intersecting, microscopic, collinear, giant, non-finite).
//!    - Assert latency < 2.0 ms and 100% agreement with linear reference oracle.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::fs;
use std::time::Instant;

use cairo::{Context, Format, ImageSurface};
use uninote_app::canvas::{CanvasState, NoteCanvas};
use uninote_app::selection::AppSelectionEngine;
use uninote_core::coordinates::Viewport;
use uninote_core::dataset::{
    AcademicDatasetGenerator, AcademicScale, DeterministicPrng, DEFAULT_ACADEMIC_SEED,
};
use uninote_core::model::{Document, DocumentFormat, ItemKind};
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, SelectionEngine,
};
use uninote_stroke::types::{PipelineMode, Rect};

// ============================================================================
// 1. DUAL ALLOCATION TRACKER (EXACT LIVE HEAP BYTES & OS STATM)
// ============================================================================

struct CountingAllocator;

thread_local! {
    static TRACK_THIS_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_LIVE_BYTES: Cell<isize> = const { Cell::new(0) };
    static THREAD_TOTAL_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() && TRACK_THIS_THREAD.with(|t| t.get()) {
            THREAD_LIVE_BYTES.with(|b| b.set(b.get() + layout.size() as isize));
            THREAD_TOTAL_ALLOCS.with(|c| c.set(c.get() + 1));
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        if TRACK_THIS_THREAD.with(|t| t.get()) {
            THREAD_LIVE_BYTES.with(|b| b.set(b.get() - layout.size() as isize));
        }
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL_ALLOC: CountingAllocator = CountingAllocator;

struct AllocTrackerGuard;

impl AllocTrackerGuard {
    fn start() -> Self {
        THREAD_LIVE_BYTES.with(|b| b.set(0));
        THREAD_TOTAL_ALLOCS.with(|c| c.set(0));
        TRACK_THIS_THREAD.with(|t| t.set(true));
        Self
    }

    fn live_bytes(&self) -> isize {
        THREAD_LIVE_BYTES.with(|b| b.get())
    }

    #[allow(dead_code)]
    fn total_allocations(&self) -> usize {
        THREAD_TOTAL_ALLOCS.with(|c| c.get())
    }
}

impl Drop for AllocTrackerGuard {
    fn drop(&mut self) {
        TRACK_THIS_THREAD.with(|t| t.set(false));
    }
}

fn get_process_rss_kb() -> usize {
    if let Ok(statm) = fs::read_to_string("/proc/self/statm") {
        if let Some(pages_str) = statm.split_whitespace().nth(1) {
            if let Ok(pages) = pages_str.parse::<usize>() {
                return pages * 4; // 4096 bytes per page
            }
        }
    }
    0
}

// ============================================================================
// HELPER: HEADLESS DOCUMENT RASTERIZER WITH SPATIAL CULLING
// ============================================================================

fn render_document_headless(
    cr: &Context,
    doc: &Document,
    vp: &Viewport,
    width: f64,
    height: f64,
) -> (usize, usize) {
    let mut total_strokes = 0;
    let mut rendered_strokes = 0;

    let doc_left = -vp.offset_x / vp.zoom;
    let doc_top = -vp.offset_y / vp.zoom;
    let doc_right = (width - vp.offset_x) / vp.zoom;
    let doc_bottom = (height - vp.offset_y) / vp.zoom;

    match doc.format {
        DocumentFormat::Notebook => {
            let _ = cr.save();
            cr.scale(vp.zoom, vp.zoom);

            for page in &doc.pages {
                total_strokes += page.items.len();
                let doc_rect = doc
                    .page_rect(page.index)
                    .unwrap_or_else(|| Viewport::page_rect(page.index));
                let (sx, sy) = vp.document_to_screen((doc_rect.x, doc_rect.y));
                let sw = doc_rect.width * vp.zoom;
                let sh = doc_rect.height * vp.zoom;

                // Culling di pagina: scarta pagine completamente fuori dal viewport
                if sx + sw < 0.0 || sx > width || sy + sh < 0.0 || sy > height {
                    continue;
                }

                let page_doc_left = (doc_left - doc_rect.x).max(0.0);
                let page_doc_top = (doc_top - doc_rect.y).max(0.0);
                let page_doc_right = (doc_right - doc_rect.x).min(doc_rect.width);
                let page_doc_bottom = (doc_bottom - doc_rect.y).min(doc_rect.height);

                if !page.spatial_index.is_empty() {
                    if page_doc_left <= page_doc_right && page_doc_top <= page_doc_bottom {
                        let visible_items = page.spatial_index.locate_in_envelope(
                            page_doc_left,
                            page_doc_top,
                            page_doc_right,
                            page_doc_bottom,
                        );
                        let _ = cr.save();
                        cr.translate(doc_rect.x, doc_rect.y);
                        for si in &visible_items {
                            let item_opt = page
                                .items
                                .binary_search_by_key(&(si.z_index, si.item_id), |it| {
                                    (it.z_index, it.id)
                                })
                                .ok()
                                .map(|idx| &page.items[idx])
                                .or_else(|| page.items.iter().find(|it| it.id == si.item_id));

                            if let Some(item) = item_opt {
                                let ItemKind::Stroke(ref s) = item.kind;
                                NoteCanvas::draw_stroke(
                                    cr,
                                    s,
                                    PipelineMode::CatmullRomSpline,
                                    false,
                                );
                                rendered_strokes += 1;
                            }
                        }
                        let _ = cr.restore();
                    }
                } else {
                    let _ = cr.save();
                    cr.translate(doc_rect.x, doc_rect.y);
                    for item in &page.items {
                        let ItemKind::Stroke(ref s) = item.kind;
                        NoteCanvas::draw_stroke(cr, s, PipelineMode::CatmullRomSpline, false);
                        rendered_strokes += 1;
                    }
                    let _ = cr.restore();
                }
            }
            let _ = cr.restore();
        }
        DocumentFormat::Whiteboard => {
            let _ = cr.save();
            cr.translate(vp.offset_x, vp.offset_y);
            cr.scale(vp.zoom, vp.zoom);

            for page in &doc.pages {
                total_strokes += page.items.len();
                if !page.spatial_index.is_empty() {
                    let visible_items = page
                        .spatial_index
                        .locate_in_envelope(doc_left, doc_top, doc_right, doc_bottom);
                    for si in &visible_items {
                        let item_opt = page
                            .items
                            .binary_search_by_key(&(si.z_index, si.item_id), |it| {
                                (it.z_index, it.id)
                            })
                            .ok()
                            .map(|idx| &page.items[idx])
                            .or_else(|| page.items.iter().find(|it| it.id == si.item_id));

                        if let Some(item) = item_opt {
                            let ItemKind::Stroke(ref s) = item.kind;
                            NoteCanvas::draw_stroke(cr, s, PipelineMode::CatmullRomSpline, false);
                            rendered_strokes += 1;
                        }
                    }
                } else {
                    for item in &page.items {
                        let ItemKind::Stroke(ref s) = item.kind;
                        NoteCanvas::draw_stroke(cr, s, PipelineMode::CatmullRomSpline, false);
                        rendered_strokes += 1;
                    }
                }
            }
            let _ = cr.restore();
        }
    }

    (rendered_strokes, total_strokes)
}

// ============================================================================
// 2. STRESS TEST SUITE
// ============================================================================

/// 1. Adversarial Viewport Navigation & Cairo Stress under extreme zoom levels (0.01x to 100.0x) on 10k and 50k strokes.
#[test]
fn stress_01_adversarial_extreme_zoom_and_pan_10k_and_50k() {
    let surface =
        ImageSurface::create(Format::ARgb32, 1920, 1080).expect("Surface creation failed");
    let cr = Context::new(&surface).expect("Cairo context creation failed");

    // Part A: 10,000 strokes on Whiteboard and Notebook
    {
        let mut gen = AcademicDatasetGenerator::new();
        let doc_wb = gen.generate_whiteboard(AcademicScale::Medium10k);
        assert_eq!(doc_wb.pages[0].items.len(), 10_000);

        let zoom_levels = [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 50.0, 100.0];

        for &zoom in &zoom_levels {
            let vp = Viewport {
                offset_x: 960.0,
                offset_y: 540.0,
                zoom,
            };

            // Warmup 2 frames for steady-state Cairo rasterization
            for _ in 0..2 {
                render_document_headless(&cr, &doc_wb, &vp, 1920.0, 1080.0);
            }

            const MEASURE_SAMPLES: usize = 5;
            let mut sample_ms = Vec::with_capacity(MEASURE_SAMPLES);
            let mut rendered = 0;
            let mut total = 0;

            for _ in 0..MEASURE_SAMPLES {
                let t0 = Instant::now();
                let (r, t) = render_document_headless(&cr, &doc_wb, &vp, 1920.0, 1080.0);
                sample_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
                rendered = r;
                total = t;
            }

            let elapsed_ms = sample_ms.iter().sum::<f64>() / MEASURE_SAMPLES as f64;
            let culled_pct = if total > 0 {
                ((total - rendered) as f64 / total as f64) * 100.0
            } else {
                0.0
            };

            println!(
                "[STRESS 01 - 10k WB] Zoom {:.2}x: Rendered {}/{} ({:.1}% culled) in {:.3} ms",
                zoom, rendered, total, culled_pct, elapsed_ms
            );

            // Assertions by zoom category:
            if zoom >= 1.0 {
                // At zoom >= 1.0, culling must be effective and render time must be fast (< 8.0 ms in release)
                assert!(
                    culled_pct > 80.0,
                    "Zoom {:.2}x: Expected > 80% culled, got {:.1}%",
                    zoom,
                    culled_pct
                );
                let budget_ms = if cfg!(debug_assertions) { 25.0 } else { 8.0 };
                assert!(
                    elapsed_ms < budget_ms,
                    "Zoom {:.2}x: Render time {:.3} ms exceeded budget {:.1} ms",
                    zoom,
                    elapsed_ms,
                    budget_ms
                );
            }
            if zoom >= 50.0 {
                // Extreme zoom in: almost everything is culled
                assert!(
                    culled_pct > 99.0,
                    "Zoom {:.2}x: Expected > 99% culled, got {:.1}%",
                    zoom,
                    culled_pct
                );
                let budget_ms = if cfg!(debug_assertions) { 10.0 } else { 3.0 };
                assert!(
                    elapsed_ms < budget_ms,
                    "Zoom {:.2}x: Render time {:.3} ms exceeded budget {:.1} ms",
                    zoom,
                    elapsed_ms,
                    budget_ms
                );
            }
            if zoom <= 0.01 {
                // Extreme zoom out: all strokes visible, no panic, completes within reasonable SLA
                let sla_ms = if cfg!(debug_assertions) { 300.0 } else { 120.0 };
                assert!(
                    elapsed_ms < sla_ms,
                    "Zoom {:.2}x: Render time {:.3} ms exceeded extreme zoom SLA {:.1} ms",
                    zoom,
                    elapsed_ms,
                    sla_ms
                );
            }
        }
    }

    // Part B: 50,000 strokes on Whiteboard under continuous pan/zoom across extreme envelopes
    {
        let mut gen = AcademicDatasetGenerator::new();
        let doc_50k = gen.generate_whiteboard(AcademicScale::Massive50k);
        assert_eq!(doc_50k.pages[0].items.len(), 50_000);

        let mut vp = Viewport {
            zoom: 100.0,
            ..Default::default()
        };
        let t_in = Instant::now();
        let (rendered_in, total_in) = render_document_headless(&cr, &doc_50k, &vp, 1920.0, 1080.0);
        let elapsed_in_ms = t_in.elapsed().as_secs_f64() * 1000.0;
        let culled_in_pct = ((total_in - rendered_in) as f64 / total_in as f64) * 100.0;

        println!(
            "[STRESS 01 - 50k WB] Extreme Zoom In (100.0x): Rendered {}/{} ({:.2}% culled) in {:.3} ms",
            rendered_in, total_in, culled_in_pct, elapsed_in_ms
        );
        assert!(
            culled_in_pct > 99.5,
            "50k at 100x zoom should cull > 99.5% of strokes"
        );
        let budget_in_ms = if cfg!(debug_assertions) { 10.0 } else { 3.0 };
        assert!(
            elapsed_in_ms < budget_in_ms,
            "50k at 100x zoom exceeded SLA of {:.1} ms",
            budget_in_ms
        );

        // 2. Normal zoom test (1.0x) with culling SLA < 8.0 ms
        vp.zoom = 1.0;
        let t_norm = Instant::now();
        let (rendered_norm, total_norm) =
            render_document_headless(&cr, &doc_50k, &vp, 1920.0, 1080.0);
        let elapsed_norm_ms = t_norm.elapsed().as_secs_f64() * 1000.0;
        let culled_norm_pct = ((total_norm - rendered_norm) as f64 / total_norm as f64) * 100.0;

        println!(
            "[STRESS 01 - 50k WB] Normal Zoom (1.0x): Rendered {}/{} ({:.2}% culled) in {:.3} ms",
            rendered_norm, total_norm, culled_norm_pct, elapsed_norm_ms
        );
        assert!(
            culled_norm_pct > 90.0,
            "50k at 1.0x zoom should cull > 90% of strokes"
        );
        let budget_norm_ms = if cfg!(debug_assertions) { 25.0 } else { 8.0 };
        assert!(
            elapsed_norm_ms < budget_norm_ms,
            "50k at 1.0x zoom exceeded SLA of {:.1} ms",
            budget_norm_ms
        );

        // 3. Extreme zoom-out test (0.01x) centered at canvas origin where all 50k strokes are visible
        vp.offset_x = 960.0;
        vp.offset_y = 540.0;
        vp.zoom = 0.01;
        let t_out = Instant::now();
        let (rendered_out, total_out) =
            render_document_headless(&cr, &doc_50k, &vp, 1920.0, 1080.0);
        let elapsed_out_ms = t_out.elapsed().as_secs_f64() * 1000.0;

        println!(
            "[STRESS 01 - 50k WB] Extreme Zoom Out (0.01x): Rendered {}/{} in {:.2} ms",
            rendered_out, total_out, elapsed_out_ms
        );
        assert_eq!(
            rendered_out, 50_000,
            "All 50,000 strokes must be visible at 0.01x zoom"
        );
        let budget_out_ms = if cfg!(debug_assertions) { 800.0 } else { 350.0 };
        assert!(
            elapsed_out_ms < budget_out_ms,
            "50k at 0.01x zoom exceeded SLA of {:.1} ms",
            budget_out_ms
        );

        // 4. Adversarial continuous pan with extreme coordinate offsets
        let offsets = [
            (-50_000.0, -50_000.0),
            (50_000.0, 50_000.0),
            (-100_000.0, 100_000.0),
            (0.0, 0.0),
        ];
        for (ox, oy) in offsets {
            vp.offset_x = ox;
            vp.offset_y = oy;
            vp.zoom = 2.5;
            let (r, t) = render_document_headless(&cr, &doc_50k, &vp, 1920.0, 1080.0);
            assert_eq!(t, 50_000);
            assert!(r <= t);
        }
    }
}

/// 2. Memory Leak Extended Stress: 2,000 continuous frames of randomized pan/zoom on 10,000 strokes.
///
/// Asserts net leaked bytes == 0.
#[test]
fn stress_02_memory_leak_2000_frames_randomized_pan_zoom() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Medium10k);
    let surface =
        ImageSurface::create(Format::ARgb32, 1920, 1080).expect("Surface creation failed");
    let cr = Context::new(&surface).expect("Cairo context creation failed");
    let mut vp = Viewport::default();

    // Warmup 50 frames to establish steady state
    for _ in 0..50 {
        vp.pan(2.0, 2.0);
        render_document_headless(&cr, &doc, &vp, 1920.0, 1080.0);
    }

    let initial_rss_kb = get_process_rss_kb();
    let tracker = AllocTrackerGuard::start();
    let baseline_live = tracker.live_bytes();

    let mut prng = DeterministicPrng::new(0xDEAD_BEEF_CAFE_0042);
    const NUM_FRAMES: usize = 2000;

    // Run 2,000 randomized navigation frames
    for _step in 0..NUM_FRAMES {
        let dx = prng.range_f64(-30.0, 30.0);
        let dy = prng.range_f64(-30.0, 30.0);
        vp.pan(dx, dy);

        let zoom_mult = prng.range_f64(0.97, 1.03);
        vp.zoom_at((960.0, 540.0), zoom_mult);

        // Clamp zoom within reasonable bounds to prevent zero or overflow
        vp.zoom = vp.zoom.clamp(0.05, 50.0);

        render_document_headless(&cr, &doc, &vp, 1920.0, 1080.0);
    }

    let end_live = tracker.live_bytes();
    let net_leaked_bytes = end_live - baseline_live;
    drop(tracker);

    let final_rss_kb = get_process_rss_kb();
    println!(
        "[STRESS 02] 2,000 Frames Memory Leak Stress: Net Leak: {} bytes (RSS: {} KB -> {} KB)",
        net_leaked_bytes, initial_rss_kb, final_rss_kb
    );

    assert_eq!(
        net_leaked_bytes, 0,
        "Memory leak detected! Leaked {} bytes across 2,000 frames",
        net_leaked_bytes
    );
}

/// 3. Adversarial Eraser Stress across 1,000 queries.
///
/// Tests random points across dense strokes, empty margins, and degenerate envelopes.
/// Asserts latency < 0.5 ms for every single event.
#[test]
fn stress_03_adversarial_eraser_1000_queries() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Medium10k);
    let mut st = CanvasState::new();
    st.document = doc;
    assert_eq!(st.document.pages[0].items.len(), 10_000);

    let mut prng = DeterministicPrng::new(DEFAULT_ACADEMIC_SEED + 777);
    const TOTAL_QUERIES: usize = 1000;
    let mut latencies_us = Vec::with_capacity(TOTAL_QUERIES);

    for i in 0..TOTAL_QUERIES {
        let (pt, r) = match i % 5 {
            0 => {
                // Dense central cluster hit
                (
                    (prng.range_f64(-400.0, 400.0), prng.range_f64(-300.0, 300.0)),
                    15.0,
                )
            }
            1 => {
                // Far whitespace miss
                (
                    (
                        prng.range_f64(-8000.0, -6000.0),
                        prng.range_f64(-8000.0, -6000.0),
                    ),
                    12.0,
                )
            }
            2 => {
                // Panel gutter boundary
                (
                    (prng.range_f64(500.0, 700.0), prng.range_f64(400.0, 550.0)),
                    10.0,
                )
            }
            3 => {
                // Large eraser radius wiping cluster
                (
                    (prng.range_f64(-200.0, 200.0), prng.range_f64(-200.0, 200.0)),
                    40.0,
                )
            }
            _ => {
                // Adversarial and boundary inputs
                match i % 4 {
                    0 => ((0.0, 0.0), 0.0),       // Zero radius
                    1 => ((100.0, 100.0), -5.0),  // Negative radius
                    2 => ((f64::NAN, 0.0), 10.0), // NaN coordinate
                    _ => ((1e12, 1e12), 10.0),    // Extreme coordinate
                }
            }
        };

        let t0 = Instant::now();
        NoteCanvas::erase_strokes_at(&mut st, pt, r);
        let elapsed_us = t0.elapsed().as_micros();
        latencies_us.push(elapsed_us);

        let elapsed_ms = elapsed_us as f64 / 1000.0;
        if elapsed_ms > 1.0 {
            println!(
                "Event {}: pt={:?}, r={:.1}, latency={:.4} ms, items_remaining={}",
                i,
                pt,
                r,
                elapsed_ms,
                st.document.pages[0].items.len()
            );
        }
        let max_event_budget_ms = if cfg!(debug_assertions) { 10.0 } else { 0.5 };
        assert!(
            elapsed_ms < max_event_budget_ms,
            "Event {}: Eraser latency {:.4} ms exceeded SLA budget of {:.2} ms",
            i,
            elapsed_ms,
            max_event_budget_ms
        );
    }

    latencies_us.sort();
    let avg_us = latencies_us.iter().sum::<u128>() as f64 / TOTAL_QUERIES as f64;
    let avg_ms = avg_us / 1000.0;
    let p99_ms = latencies_us[TOTAL_QUERIES * 99 / 100] as f64 / 1000.0;
    let max_ms = *latencies_us.last().unwrap() as f64 / 1000.0;

    println!(
        "[STRESS 03] Adversarial Eraser (1,000 queries): Avg {:.4} ms, P99 {:.4} ms, Max {:.4} ms",
        avg_ms, p99_ms, max_ms
    );

    let max_avg_ms = if cfg!(debug_assertions) { 0.5 } else { 0.2 };
    assert!(
        avg_ms < max_avg_ms,
        "Average eraser latency {:.4} ms exceeded budget {:.2} ms",
        avg_ms,
        max_avg_ms
    );
    assert_eq!(
        st.document.pages[0].spatial_index.len(),
        st.document.pages[0].items.len(),
        "Spatial index must remain strictly synchronized with items count"
    );
}

/// 4. Adversarial Selection Stress across 1,000 lasso queries and 500 rect queries.
///
/// Degenerate lasso polygons (self-intersecting, microscopic, collinear, giant, non-finite).
/// Degenerate rectangles (subpixel, inverted, giant, non-finite).
/// Asserts latency < 2.0 ms in release and 100% agreement with linear reference oracle.
#[test]
fn stress_04_adversarial_selection_degenerate_polygons_1000_queries() {
    let mut gen = AcademicDatasetGenerator::new();
    let doc = gen.generate_whiteboard(AcademicScale::Medium10k);
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 10_000);

    let mut prng = DeterministicPrng::new(DEFAULT_ACADEMIC_SEED + 888);
    const TOTAL_LASSO_QUERIES: usize = 1000;
    let mut lasso_latencies_ms = Vec::with_capacity(TOTAL_LASSO_QUERIES);

    // 4A: 1,000 Adversarial Lasso Queries
    for i in 0..TOTAL_LASSO_QUERIES {
        let cat = i % 10;
        let poly = match cat {
            0 => {
                // Self-intersecting figure-8 polygon
                let cx = prng.range_f64(-300.0, 300.0);
                let cy = prng.range_f64(-200.0, 200.0);
                vec![
                    (cx - 100.0, cy - 100.0),
                    (cx + 100.0, cy + 100.0),
                    (cx - 100.0, cy + 100.0),
                    (cx + 100.0, cy - 100.0),
                ]
            }
            1 => {
                // Microscopic polygon (side 0.0001 pt)
                let cx = prng.range_f64(-100.0, 100.0);
                let cy = prng.range_f64(-100.0, 100.0);
                vec![(cx, cy), (cx + 0.0001, cy), (cx + 0.00005, cy + 0.0001)]
            }
            2 => {
                // Collinear 3-point segment (zero area)
                let cx = prng.range_f64(-200.0, 200.0);
                let cy = prng.range_f64(-200.0, 200.0);
                vec![(cx, cy), (cx + 50.0, cy + 50.0), (cx + 100.0, cy + 100.0)]
            }
            3 => {
                // Huge polygon covering 2,500 x 2,500 pt area (~1,500 strokes)
                vec![
                    (-1250.0, -1250.0),
                    (1250.0, -1250.0),
                    (1250.0, 1250.0),
                    (-1250.0, 1250.0),
                ]
            }
            4 => {
                // Degenerate: less than 3 points
                if i % 2 == 0 {
                    vec![(10.0, 10.0), (20.0, 20.0)]
                } else {
                    vec![(10.0, 10.0)]
                }
            }
            5 => {
                // Degenerate: non-finite coordinates (NaN or Inf)
                vec![(f64::NAN, 0.0), (100.0, 100.0), (0.0, 100.0)]
            }
            6 => {
                // Concave star polygon over dense cluster
                let cx = prng.range_f64(-200.0, 200.0);
                let cy = prng.range_f64(-200.0, 200.0);
                vec![
                    (cx, cy - 120.0),
                    (cx + 35.0, cy - 35.0),
                    (cx + 120.0, cy - 35.0),
                    (cx + 55.0, cy + 20.0),
                    (cx + 80.0, cy + 100.0),
                    (cx, cy + 50.0),
                    (cx - 80.0, cy + 100.0),
                    (cx - 55.0, cy + 20.0),
                    (cx - 120.0, cy - 35.0),
                    (cx - 35.0, cy - 35.0),
                ]
            }
            7 => {
                // Far whitespace triangle
                let ox = prng.range_f64(-8000.0, -6000.0);
                let oy = prng.range_f64(-8000.0, -6000.0);
                vec![(ox, oy), (ox + 200.0, oy), (ox + 100.0, oy + 200.0)]
            }
            _ => {
                // Random convex/concave polygon (5-8 vertices)
                let num_v = prng.range_usize(4, 8);
                let cx = prng.range_f64(-600.0, 600.0);
                let cy = prng.range_f64(-500.0, 500.0);
                let mut v = Vec::with_capacity(num_v);
                for k in 0..num_v {
                    let angle = (k as f64 / num_v as f64) * 2.0 * std::f64::consts::PI;
                    let rad = prng.range_f64(40.0, 180.0);
                    v.push((cx + rad * angle.cos(), cy + rad * angle.sin()));
                }
                v
            }
        };

        let t0 = Instant::now();
        let (accel_ids, accel_bbox) = AppSelectionEngine::select_lasso(page, &poly);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        lasso_latencies_ms.push(elapsed_ms);

        // Ground-truth reference oracle
        let mut oracle_ids = Vec::new();
        let mut oracle_strokes = Vec::new();

        if poly.len() >= 3 && poly.iter().all(|&(x, y)| x.is_finite() && y.is_finite()) {
            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_lasso(s, &poly) {
                    oracle_ids.push(item.id);
                    oracle_strokes.push(s);
                }
            }
        }
        let oracle_bbox = SelectionEngine::compute_selection_bounding_box(oracle_strokes);

        assert_eq!(
            accel_ids, oracle_ids,
            "Lasso Query {}: Accelerated selected IDs mismatch with reference oracle!",
            i
        );
        assert_eq!(
            accel_bbox, oracle_bbox,
            "Lasso Query {}: Accelerated bounding box mismatch with reference oracle!",
            i
        );

        let max_budget_ms = if cfg!(debug_assertions) { 15.0 } else { 2.0 };
        assert!(
            elapsed_ms < max_budget_ms,
            "Lasso Query {}: Selection latency {:.3} ms exceeded budget {:.1} ms",
            i,
            elapsed_ms,
            max_budget_ms
        );
    }

    lasso_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg_ms = lasso_latencies_ms.iter().sum::<f64>() / TOTAL_LASSO_QUERIES as f64;
    let p99_ms = lasso_latencies_ms[TOTAL_LASSO_QUERIES * 99 / 100];
    let max_ms = *lasso_latencies_ms.last().unwrap();

    println!(
        "[STRESS 04A] Adversarial Lasso (1,000 queries): Avg {:.4} ms, P99 {:.4} ms, Max {:.4} ms",
        avg_ms, p99_ms, max_ms
    );

    let max_avg_ms = if cfg!(debug_assertions) { 2.0 } else { 0.5 };
    assert!(
        avg_ms < max_avg_ms,
        "Average lasso selection latency {:.4} ms exceeded budget {:.2} ms",
        avg_ms,
        max_avg_ms
    );

    // 4B: 500 Adversarial Rectangular Queries
    const TOTAL_RECT_QUERIES: usize = 500;
    let mut rect_latencies_ms = Vec::with_capacity(TOTAL_RECT_QUERIES);

    for j in 0..TOTAL_RECT_QUERIES {
        let rect = match j % 5 {
            0 => {
                // Microscopic subpixel rect (< 2.0 pt)
                let cx = prng.range_f64(-200.0, 200.0);
                let cy = prng.range_f64(-200.0, 200.0);
                Rect::new(cx, cy, 0.5, 0.5)
            }
            1 => {
                // Inverted negative rect
                let cx = prng.range_f64(-300.0, 300.0);
                let cy = prng.range_f64(-300.0, 300.0);
                Rect::new(cx, cy, -150.0, -120.0)
            }
            2 => {
                // Non-finite rect
                Rect::new(f64::NAN, 0.0, 100.0, 100.0)
            }
            3 => {
                // Huge rect across multiple panels
                Rect::new(-1500.0, -1500.0, 3000.0, 3000.0)
            }
            _ => {
                // Normal random cluster rect
                let cx = prng.range_f64(-500.0, 500.0);
                let cy = prng.range_f64(-500.0, 500.0);
                let w = prng.range_f64(30.0, 300.0);
                let h = prng.range_f64(30.0, 300.0);
                Rect::new(cx, cy, w, h)
            }
        };

        let t0 = Instant::now();
        let (accel_ids, accel_bbox) = AppSelectionEngine::select_rect(page, &rect);
        let elapsed_ms = t0.elapsed().as_secs_f64() * 1000.0;
        rect_latencies_ms.push(elapsed_ms);

        // Ground-truth reference oracle
        let norm_rect = rect.normalized();
        let mut oracle_ids = Vec::new();
        let mut oracle_strokes = Vec::new();

        let is_too_small = norm_rect.width() < 2.0 && norm_rect.height() < 2.0;
        if !is_too_small
            && norm_rect.min_x.is_finite()
            && norm_rect.min_y.is_finite()
            && norm_rect.max_x.is_finite()
            && norm_rect.max_y.is_finite()
        {
            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_rect(s, &norm_rect) {
                    oracle_ids.push(item.id);
                    oracle_strokes.push(s);
                }
            }
        }
        let oracle_bbox = SelectionEngine::compute_selection_bounding_box(oracle_strokes);

        assert_eq!(
            accel_ids, oracle_ids,
            "Rect Query {}: Accelerated selected IDs mismatch with reference oracle!",
            j
        );
        assert_eq!(
            accel_bbox, oracle_bbox,
            "Rect Query {}: Accelerated bounding box mismatch with reference oracle!",
            j
        );

        let max_budget_ms = if cfg!(debug_assertions) { 15.0 } else { 2.0 };
        assert!(
            elapsed_ms < max_budget_ms,
            "Rect Query {}: Selection latency {:.3} ms exceeded budget {:.1} ms",
            j,
            elapsed_ms,
            max_budget_ms
        );
    }

    rect_latencies_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let avg_rect_ms = rect_latencies_ms.iter().sum::<f64>() / TOTAL_RECT_QUERIES as f64;
    let p99_rect_ms = rect_latencies_ms[TOTAL_RECT_QUERIES * 99 / 100];
    let max_rect_ms = *rect_latencies_ms.last().unwrap();

    println!(
        "[STRESS 04B] Adversarial Rect (500 queries): Avg {:.4} ms, P99 {:.4} ms, Max {:.4} ms",
        avg_rect_ms, p99_rect_ms, max_rect_ms
    );
}
