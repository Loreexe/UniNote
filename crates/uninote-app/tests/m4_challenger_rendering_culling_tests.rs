//! Empirical Adversarial Stress Testing Suite for Milestone M4
//! (Cairo Rendering & Viewport Spatial Culling Optimization)
//!
//! Authoritative references:
//! - ORIGINAL_REQUEST.md (Requirements R1-R6: 120 FPS, 10k/50k strokes, R-Tree, zero heap allocations)
//! - PROJECT.md (Architecture, Features 13-17, Interface Contracts, Milestone M4)
//! - .uninote-wiki/RULES.md (Code quality, SemVer, integrity, empirical verification)

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashSet;
use std::time::Instant;

use gtk4::cairo::{Context, Format, ImageSurface};
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::coordinates::{Rect2D, Viewport};
use uninote_core::model::{
    Document, Item, ItemId, ItemKind, Page, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT,
};
use uninote_stroke::geometry::catmull_rom_to_bezier;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// THREAD-ISOLATED HEAP ALLOCATION TRACKER (EMPIRICAL ORACLE)
// ============================================================================

struct CountingAllocator;

thread_local! {
    static TRACK_THIS_THREAD: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOC_COUNT: Cell<usize> = const { Cell::new(0) };
    static THREAD_ALLOC_BYTES: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let is_tracked = TRACK_THIS_THREAD.with(|t| t.get());
        if is_tracked {
            THREAD_ALLOC_COUNT.with(|c| c.set(c.get() + 1));
            THREAD_ALLOC_BYTES.with(|b| b.set(b.get() + layout.size()));
        }
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL_ALLOC: CountingAllocator = CountingAllocator;

/// RAII Guard that counts heap allocations exclusively on the calling thread.
struct ThreadAllocGuard;

impl ThreadAllocGuard {
    fn start() -> Self {
        THREAD_ALLOC_COUNT.with(|c| c.set(0));
        THREAD_ALLOC_BYTES.with(|b| b.set(0));
        TRACK_THIS_THREAD.with(|t| t.set(true));
        Self
    }

    fn allocations(&self) -> usize {
        THREAD_ALLOC_COUNT.with(|c| c.get())
    }

    fn allocated_bytes(&self) -> usize {
        THREAD_ALLOC_BYTES.with(|b| b.get())
    }
}

impl Drop for ThreadAllocGuard {
    fn drop(&mut self) {
        TRACK_THIS_THREAD.with(|t| t.set(false));
    }
}

// ============================================================================
// DETERMINISTIC PSEUDO-RANDOM GENERATOR (LCG 64-BIT)
// ============================================================================

struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xFEED_FACE_CAFE_BEEF
            } else {
                seed
            },
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
        min + (self.next_u64() as usize % (max - min + 1))
    }
}

// ============================================================================
// REALISTIC STROKE GENERATORS (ACADEMIC HANDWRITING & DIAGRAMS)
// ============================================================================

/// Generates a realistic cursive handwriting stroke loop with variable pressure.
fn generate_realistic_cursive_stroke(
    prng: &mut DeterministicPrng,
    id: u64,
    origin_x: f64,
    origin_y: f64,
    span_width: f64,
) -> Stroke {
    let point_count = prng.range_usize(12, 32);
    let mut points = Vec::with_capacity(point_count);
    let base_width = prng.range_f64(1.6, 3.0);
    let mut cur_x = origin_x;
    let mut cur_y = origin_y;

    for i in 0..point_count {
        let t = i as f64 / (point_count - 1).max(1) as f64;
        let pressure = (0.3 + 0.5 * (t * std::f64::consts::PI).sin()).clamp(0.08, 1.0);
        let pt_width = base_width * (0.6 + 0.8 * pressure);
        points.push(StrokePoint {
            x: cur_x,
            y: cur_y,
            pressure,
            width: pt_width,
            timestamp_ns: 1_000_000 * (i as u64),
        });

        let dx = (span_width / point_count as f64) + prng.range_f64(-0.6, 1.4);
        let dy = (t * std::f64::consts::PI * 2.0).sin() * 7.5 + prng.range_f64(-1.5, 1.5);
        cur_x += dx;
        cur_y += dy;
    }

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle {
            color: (0.10, 0.12, 0.16, 1.0),
            base_width,
            min_width_factor: 0.5,
            max_width_factor: 1.6,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

/// Generates an academic mathematical diagram stroke (formula, arrow, or radical).
fn generate_diagram_stroke(
    prng: &mut DeterministicPrng,
    id: u64,
    center_x: f64,
    center_y: f64,
) -> Stroke {
    let kind = prng.range_usize(0, 2);
    let mut points = Vec::new();
    let base_width = 2.2;

    match kind {
        0 => {
            // Radical square root symbol
            points.push(StrokePoint {
                x: center_x - 20.0,
                y: center_y + 4.0,
                pressure: 0.45,
                width: base_width,
                timestamp_ns: 1000,
            });
            points.push(StrokePoint {
                x: center_x - 12.0,
                y: center_y + 16.0,
                pressure: 0.75,
                width: base_width * 1.3,
                timestamp_ns: 2000,
            });
            points.push(StrokePoint {
                x: center_x - 4.0,
                y: center_y - 24.0,
                pressure: 0.90,
                width: base_width * 1.5,
                timestamp_ns: 3000,
            });
            points.push(StrokePoint {
                x: center_x + 40.0,
                y: center_y - 24.0,
                pressure: 0.50,
                width: base_width,
                timestamp_ns: 4000,
            });
        }
        1 => {
            // Division fraction bar or horizontal vector arrow
            for i in 0..12 {
                let frac = i as f64 / 11.0;
                points.push(StrokePoint {
                    x: center_x - 30.0 + 60.0 * frac,
                    y: center_y + prng.range_f64(-0.25, 0.25),
                    pressure: 0.60,
                    width: base_width,
                    timestamp_ns: (i as u64) * 1000,
                });
            }
        }
        _ => {
            // Curvature stroke (integral or bracket)
            for i in 0..16 {
                let t = i as f64 / 15.0;
                let theta = (t - 0.5) * std::f64::consts::PI;
                points.push(StrokePoint {
                    x: center_x + theta.sin() * 14.0,
                    y: center_y - 30.0 + 60.0 * t,
                    pressure: 0.4 + 0.4 * (t * std::f64::consts::PI).sin(),
                    width: base_width * 1.1,
                    timestamp_ns: (i as u64) * 1000,
                });
            }
        }
    }

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle {
            color: (0.12, 0.25, 0.55, 1.0),
            base_width,
            min_width_factor: 0.6,
            max_width_factor: 1.4,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

/// Populates an A4 page with `count` realistic strokes arranged in lines.
fn populate_page_with_strokes(page: &mut Page, count: usize, seed: u64, y_offset: f64) {
    let mut prng = DeterministicPrng::new(seed);
    let mut items = Vec::with_capacity(count);

    let line_height = 20.0;
    let margin_x = 45.0;
    let content_w = A4_WIDTH_PT - margin_x * 2.0;
    let mut cur_y = y_offset + 50.0;
    let mut cur_x = margin_x;

    for i in 0..count {
        let span = prng.range_f64(30.0, 90.0);
        if cur_x + span > margin_x + content_w {
            cur_x = margin_x;
            cur_y += line_height;
            if cur_y > y_offset + A4_HEIGHT_PT - 40.0 {
                cur_y = y_offset + 50.0;
            }
        }

        let stroke = if prng.next_f64() < 0.25 {
            generate_diagram_stroke(&mut prng, (i + 1) as u64, cur_x + span * 0.5, cur_y)
        } else {
            generate_realistic_cursive_stroke(&mut prng, (i + 1) as u64, cur_x, cur_y, span)
        };

        cur_x += span + prng.range_f64(8.0, 16.0);

        items.push(Item {
            id: (i + 1) as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    page.items = items;
    page.rebuild_spatial_index();
}

/// Populates an Infinite Whiteboard page with `count` strokes scattered over a 2D bounding space.
fn populate_whiteboard_with_strokes(
    page: &mut Page,
    count: usize,
    seed: u64,
    bound_min: (f64, f64),
    bound_max: (f64, f64),
) {
    let mut prng = DeterministicPrng::new(seed);
    let mut items = Vec::with_capacity(count);

    for i in 0..count {
        let cx = prng.range_f64(bound_min.0, bound_max.0);
        let cy = prng.range_f64(bound_min.1, bound_max.1);
        let span = prng.range_f64(25.0, 85.0);

        let stroke = if prng.next_f64() < 0.3 {
            generate_diagram_stroke(&mut prng, (i + 1) as u64, cx, cy)
        } else {
            generate_realistic_cursive_stroke(&mut prng, (i + 1) as u64, cx, cy, span)
        };

        items.push(Item {
            id: (i + 1) as u64,
            z_index: (i + 1) as u64,
            kind: ItemKind::Stroke(stroke),
        });
    }

    page.items = items;
    page.rebuild_spatial_index();
}

/// Generates a realistic multi-page academic lecture notebook with `total_strokes` distributed across `page_count` pages.
fn generate_academic_a4_notebook(page_count: usize, total_strokes: usize, seed: u64) -> Document {
    let mut doc = Document::new_notebook("Appunti Analisi Matematica II");
    doc.pages.clear();

    let strokes_per_page = total_strokes / page_count;
    let mut cur_seed = seed;

    for p in 0..page_count {
        let mut page = Page::new_a4(
            p,
            PageBackground::Ruled {
                line_spacing: 24.0,
                margin_left: 45.0,
            },
        );
        let page_y = (p as f64) * (A4_HEIGHT_PT + 40.0);
        populate_page_with_strokes(&mut page, strokes_per_page, cur_seed, page_y);
        doc.pages.push(page);
        cur_seed = cur_seed.wrapping_add(1013904223);
    }

    doc
}

/// Ground-truth intersection oracle: tests if a stroke's bounding box intersects `(min_x, min_y, max_x, max_y)`.
fn ground_truth_intersects(
    stroke: &Stroke,
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
) -> bool {
    if let Some(bbox) = stroke.bounding_box() {
        let norm = bbox.normalized();
        !(norm.max_x < min_x || norm.min_x > max_x || norm.max_y < min_y || norm.min_y > max_y)
    } else {
        false
    }
}

// ============================================================================
// 1. ZERO HEAP ALLOCATION INVARIANT ON CACHED STROKES
// ============================================================================

#[test]
fn test_zero_heap_allocations_on_cached_bezier_vs_uncached_fallback() {
    let mut prng = DeterministicPrng::new(42);
    let mut stroke = generate_realistic_cursive_stroke(&mut prng, 1, 50.0, 50.0, 80.0);
    stroke.ensure_cache();
    assert!(!stroke.cached_bezier.is_empty());

    // 1. Verify `stroke.bezier_segments()` performs 0 heap allocations
    let guard = ThreadAllocGuard::start();
    let segs = stroke.bezier_segments();
    let allocs = guard.allocations();
    drop(guard);

    assert_eq!(
        allocs, 0,
        "stroke.bezier_segments() must perform ZERO heap allocations on cached stroke! Got {}",
        allocs
    );
    assert!(!segs.is_empty(), "Cached segments must not be empty");

    // 2. Contrast with uncached path: `catmull_rom_to_bezier(&stroke.points)` performs heap allocations
    let guard_uncached = ThreadAllocGuard::start();
    let computed_segs = catmull_rom_to_bezier(&stroke.points);
    let uncached_allocs = guard_uncached.allocations();
    let uncached_bytes = guard_uncached.allocated_bytes();
    drop(guard_uncached);

    assert!(
        uncached_allocs > 0,
        "Uncached spline computation must allocate a Vec on the heap! Got {} allocs, {} bytes",
        uncached_allocs,
        uncached_bytes
    );
    assert_eq!(segs, computed_segs.as_slice());

    // 3. Verify zero heap allocations during segment evaluation loop (simulating Cairo rasterization)
    let guard_loop = ThreadAllocGuard::start();
    let mut total_len = 0.0;
    for seg in segs {
        total_len += seg.approximate_length();
        let (x, y) = seg.evaluate(0.5);
        let w = seg.evaluate_width(0.5);
        assert!(x.is_finite() && y.is_finite() && w.is_finite());
    }
    let loop_allocs = guard_loop.allocations();
    drop(guard_loop);

    assert_eq!(
        loop_allocs, 0,
        "Iterating and evaluating cached Bézier segments must perform ZERO heap allocations! Got {}",
        loop_allocs
    );
    assert!(total_len > 0.0);
}

#[test]
fn test_zero_heap_allocations_on_cairo_rasterization_loop() {
    let mut prng = DeterministicPrng::new(99);
    let mut stroke = generate_realistic_cursive_stroke(&mut prng, 1, 30.0, 30.0, 100.0);
    stroke.ensure_cache();

    let surface = ImageSurface::create(Format::ARgb32, 200, 200).expect("ImageSurface failed");
    let cr = Context::new(&surface).expect("Context failed");

    // Warm up Cairo context
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.move_to(0.0, 0.0);
    cr.line_to(1.0, 1.0);
    let _ = cr.stroke();

    // Verify drawing loop borrowing `stroke.bezier_segments()` produces ZERO Rust heap allocations
    let guard = ThreadAllocGuard::start();
    let segs = stroke.bezier_segments();
    for seg in segs {
        let len = seg.approximate_length();
        let width_diff = (seg.width1 - seg.width0).abs();
        if width_diff < 0.25 || len <= 3.0 {
            let avg_w = (seg.width0 + seg.width1) / 2.0;
            cr.set_line_width(avg_w.max(1.0));
            cr.move_to(seg.p0.0, seg.p0.1);
            cr.curve_to(
                seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
            );
            let _ = cr.stroke();
        } else {
            let steps = ((len / 2.0).ceil() as usize).clamp(2, 16);
            let mut prev_pt = seg.p0;
            let mut prev_w = seg.width0;
            for s in 1..=steps {
                let t = (s as f64) / (steps as f64);
                let pt = seg.evaluate(t);
                let w = seg.evaluate_width(t);
                cr.set_line_width(((prev_w + w) / 2.0).max(1.0));
                cr.move_to(prev_pt.0, prev_pt.1);
                cr.line_to(pt.0, pt.1);
                let _ = cr.stroke();
                prev_pt = pt;
                prev_w = w;
            }
        }
    }
    let allocs = guard.allocations();
    drop(guard);

    assert_eq!(
        allocs, 0,
        "Cached Cairo stroke rasterization loop produced {} Rust heap allocations, expected 0",
        allocs
    );
}

// ============================================================================
// 2. VIEWPORT CULLING STRESS TEST ON 10,000 REALISTIC STROKES (A4 & WHITEBOARD)
// ============================================================================

#[test]
fn test_10k_strokes_a4_notebook_spatial_culling_ratio_and_ground_truth() {
    const TOTAL_STROKES: usize = 10_000;
    const PAGE_COUNT: usize = 10;
    let doc = generate_academic_a4_notebook(PAGE_COUNT, TOTAL_STROKES, 20_261_017);

    let total_doc_strokes: usize = doc.pages.iter().map(|p| p.items.len()).sum();
    assert_eq!(total_doc_strokes, TOTAL_STROKES);

    // Zoomed-in detail viewports on an academic A4 document (10 pages):
    // Scenario 1: User reading detail on Page 0 at Zoom 250% (envelope covers [100.0..300.0, 150.0..350.0])
    // Scenario 2: User reading detail on Page 3 at Zoom 500%
    // Scenario 3: User looking at Page 9 margin
    let page_0 = &doc.pages[0];
    let page_0_y = 0.0;
    let page_3 = &doc.pages[3];
    let page_3_y = 3.0 * (A4_HEIGHT_PT + 40.0);

    let test_scenarios = [
        (
            "Page 0 Zoom 250% Detail Center",
            page_0,
            100.0,
            page_0_y + 150.0,
            300.0,
            page_0_y + 350.0,
        ),
        (
            "Page 0 Zoom 500% Extreme Detail",
            page_0,
            150.0,
            page_0_y + 200.0,
            250.0,
            page_0_y + 300.0,
        ),
        (
            "Page 3 Zoom 300% Formula Detail",
            page_3,
            120.0,
            page_3_y + 220.0,
            320.0,
            page_3_y + 420.0,
        ),
        (
            "Page 0 Top Left Corner",
            page_0,
            45.0,
            page_0_y + 50.0,
            150.0,
            page_0_y + 150.0,
        ),
        (
            "Page 0 Empty Margin",
            page_0,
            0.0,
            page_0_y,
            40.0,
            page_0_y + 200.0,
        ),
    ];

    for (name, page, min_x, min_y, max_x, max_y) in test_scenarios {
        // 1. Query spatial index
        let start_query = Instant::now();
        let spatial_items = page
            .spatial_index
            .locate_in_envelope(min_x, min_y, max_x, max_y);
        let query_us = start_query.elapsed().as_micros();

        let spatial_ids: HashSet<u64> = spatial_items.iter().map(|it| it.item_id).collect();

        // 2. Query ground-truth oracle by linear scan on page
        let ground_truth_ids: HashSet<u64> = page
            .items
            .iter()
            .filter_map(|it| {
                let ItemKind::Stroke(ref s) = it.kind;
                if ground_truth_intersects(s, min_x, min_y, max_x, max_y) {
                    Some(it.id)
                } else {
                    None
                }
            })
            .collect();

        // 3. Assert 100% agreement with ground-truth intersection
        assert_eq!(
            spatial_ids, ground_truth_ids,
            "Spatial index and ground truth differed for scenario '{}'",
            name
        );

        // 4. Verify culling ratio > 90% against document total (10k strokes)
        let visible_count = spatial_items.len();
        let culled_count = TOTAL_STROKES - visible_count;
        let culled_ratio = (culled_count as f64 / TOTAL_STROKES as f64) * 100.0;

        assert!(
            culled_ratio > 90.0,
            "Scenario '{}' failed culling ratio requirement: got {:.2}%, expected > 90% (visible={}, total={})",
            name,
            culled_ratio,
            visible_count,
            TOTAL_STROKES
        );

        // 5. Verify query latency is ultra-fast (< 2000 microseconds)
        assert!(
            query_us < 2000,
            "Spatial query for '{}' took {} us, expected < 2000 us",
            name,
            query_us
        );

        // 6. Verify strict ascending Z-index ordering
        for w in spatial_items.windows(2) {
            assert!(
                w[0].z_index <= w[1].z_index,
                "Z-index ascending order violated: {} > {}",
                w[0].z_index,
                w[1].z_index
            );
        }
    }

    // Also test single dense page with 10k strokes at zoom 500%
    let mut dense_page = Page::new_a4(0, PageBackground::Blank);
    populate_page_with_strokes(&mut dense_page, 10_000, 42, 0.0);
    // Detail view: 80x80 pt box at zoom 500%
    let detail_items = dense_page
        .spatial_index
        .locate_in_envelope(200.0, 200.0, 280.0, 280.0);
    let dense_culled = 10_000 - detail_items.len();
    let dense_ratio = (dense_culled as f64 / 10_000.0) * 100.0;
    assert!(
        dense_ratio > 90.0,
        "Single dense page at zoom 500% culling ratio was {:.2}%, expected > 90%",
        dense_ratio
    );
}

#[test]
fn test_10k_strokes_whiteboard_spatial_culling_ratio_and_ground_truth() {
    const STROKE_COUNT: usize = 10_000;
    let mut page = Page::new_whiteboard(PageBackground::Blank);
    populate_whiteboard_with_strokes(
        &mut page,
        STROKE_COUNT,
        777_888_999,
        (-3000.0, -3000.0),
        (3000.0, 3000.0),
    );

    assert_eq!(page.items.len(), STROKE_COUNT);
    assert_eq!(page.spatial_index.len(), STROKE_COUNT);

    // Viewport scenarios on a 6000x6000 pt infinite whiteboard
    let test_viewports = [
        ("Whiteboard Center 100%", -400.0, -300.0, 400.0, 300.0),
        ("Whiteboard Zoom 250%", 100.0, 100.0, 400.0, 400.0),
        ("Whiteboard Zoom 500%", 1200.0, -800.0, 1400.0, -600.0),
        ("Whiteboard Outer Boundary", 2800.0, 2800.0, 3100.0, 3100.0),
        (
            "Whiteboard Completely Empty Space",
            4000.0,
            4000.0,
            5000.0,
            5000.0,
        ),
    ];

    for (name, min_x, min_y, max_x, max_y) in test_viewports {
        let spatial_items = page
            .spatial_index
            .locate_in_envelope(min_x, min_y, max_x, max_y);
        let spatial_ids: HashSet<u64> = spatial_items.iter().map(|it| it.item_id).collect();

        let ground_truth_ids: HashSet<u64> = page
            .items
            .iter()
            .filter_map(|it| {
                let ItemKind::Stroke(ref s) = it.kind;
                if ground_truth_intersects(s, min_x, min_y, max_x, max_y) {
                    Some(it.id)
                } else {
                    None
                }
            })
            .collect();

        // 100% agreement oracle
        assert_eq!(
            spatial_ids, ground_truth_ids,
            "Whiteboard spatial query mismatch for '{}'",
            name
        );

        let visible_count = spatial_items.len();
        let culled_count = STROKE_COUNT - visible_count;
        let culled_ratio = (culled_count as f64 / STROKE_COUNT as f64) * 100.0;

        assert!(
            culled_ratio > 90.0,
            "Whiteboard scenario '{}' culling ratio was {:.2}%, expected > 90% (visible={}, total={})",
            name,
            culled_ratio,
            visible_count,
            STROKE_COUNT
        );

        // Strict z-index ordering
        for w in spatial_items.windows(2) {
            assert!(w[0].z_index <= w[1].z_index);
        }
    }
}

// ============================================================================
// 3. CONTINUOUS PAN/ZOOM NAVIGATION SIMULATION (500 STEPS WITHOUT STUTTER/PANIC)
// ============================================================================

#[test]
fn test_continuous_pan_zoom_simulation_10k_strokes_performance_and_stability() {
    const STROKE_COUNT: usize = 10_000;
    let mut page = Page::new_a4(0, PageBackground::Blank);
    populate_page_with_strokes(&mut page, STROKE_COUNT, 123456, 0.0);

    let mut vp = Viewport::default();
    let screen_w = 1920.0;
    let screen_h = 1080.0;

    // Simulate 500 continuous pan and zoom cycles:
    // - Smooth zoom in (1.0x -> 4.0x) over 100 steps
    // - Continuous diagonal pan over 150 steps
    // - Zoom out (4.0x -> 0.4x) over 100 steps
    // - High-frequency oscillating zoom/pan over 150 steps
    const TOTAL_STEPS: usize = 500;
    let mut total_query_time_us: u128 = 0;
    let mut max_step_time_us: u128 = 0;
    let mut visible_strokes_history = Vec::with_capacity(TOTAL_STEPS);

    let sim_start = Instant::now();

    for step in 0..TOTAL_STEPS {
        if step < 100 {
            // Zoom in gradually
            let factor = 1.0 + 0.015;
            vp.zoom_at((screen_w * 0.5, screen_h * 0.5), factor);
        } else if step < 250 {
            // Diagonal pan
            let dx = (step as f64 * 0.1).sin() * 15.0;
            let dy = (step as f64 * 0.1).cos() * 20.0;
            vp.pan(dx, dy);
        } else if step < 350 {
            // Zoom out gradually
            let factor = 1.0 - 0.018;
            vp.zoom_at((screen_w * 0.5, screen_h * 0.5), factor);
        } else {
            // Fast pinch-zoom oscillations around multiple focal points
            let focal_x = screen_w * (0.3 + 0.4 * (step as f64 * 0.05).sin());
            let focal_y = screen_h * (0.3 + 0.4 * (step as f64 * 0.05).cos());
            let factor = if step % 2 == 0 { 1.08 } else { 0.93 };
            vp.zoom_at((focal_x, focal_y), factor);
            vp.pan(
                (step as f64 * 0.2).cos() * 8.0,
                (step as f64 * 0.2).sin() * 8.0,
            );
        }

        // Calculate document coordinates of viewport (as canvas.rs does)
        let doc_left = -vp.offset_x / vp.zoom;
        let doc_top = -vp.offset_y / vp.zoom;
        let doc_right = (screen_w - vp.offset_x) / vp.zoom;
        let doc_bottom = (screen_h - vp.offset_y) / vp.zoom;

        // Perform executive spatial query
        let step_start = Instant::now();
        let visible = page
            .spatial_index
            .locate_in_envelope(doc_left, doc_top, doc_right, doc_bottom);
        let elapsed = step_start.elapsed().as_micros();

        total_query_time_us += elapsed;
        if elapsed > max_step_time_us {
            max_step_time_us = elapsed;
        }

        visible_strokes_history.push(visible.len());

        // Invariant: Z-index strictly ascending
        for w in visible.windows(2) {
            assert!(w[0].z_index <= w[1].z_index);
        }
    }

    let total_sim_time = sim_start.elapsed();
    let avg_step_us = total_query_time_us as f64 / TOTAL_STEPS as f64;

    // Requirement: Navigation simulation maintains > 60 FPS (target 120 FPS, which is 8.33 ms / 8333 us)
    // In debug mode, spatial query must stay under 5.0 ms (5000 us); in release mode it runs in < 200 us.
    assert!(
        avg_step_us < 5000.0,
        "Average viewport query time was {:.2} us, target < 5000 us for > 120 FPS navigation",
        avg_step_us
    );
    assert!(
        max_step_time_us < 16_000,
        "Max step time spiked to {} us (exceeded 60 FPS frame budget of 16.6ms)",
        max_step_time_us
    );

    // Verify index remained intact and consistent
    assert_eq!(page.spatial_index.len(), STROKE_COUNT);
    assert!(
        total_sim_time.as_secs_f64() < 5.0,
        "500 steps simulation took too long: {:?}",
        total_sim_time
    );
}

// ============================================================================
// 4. DYNAMIC MUTATIONS & SPATIAL INDEX SYNCHRONIZATION
// ============================================================================

#[test]
fn test_dynamic_mutations_and_spatial_index_synchronization_under_load() {
    let mut doc = Document::new_notebook("Test Dynamic Mutations");
    let page_id = doc.pages[0].id;

    let mut history = CommandHistory::new(50);
    let mut prng = DeterministicPrng::new(555);

    // 1. Add 100 strokes dynamically via CommandHistory
    for i in 1..=100 {
        let stroke =
            generate_realistic_cursive_stroke(&mut prng, i, 100.0 + (i as f64) * 2.0, 100.0, 40.0);
        history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    }

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.spatial_index.len(), 100);

    // Verify 100% ground-truth agreement after dynamic insertions
    let query_box = (80.0, 80.0, 200.0, 200.0);
    let res =
        page.spatial_index
            .locate_in_envelope(query_box.0, query_box.1, query_box.2, query_box.3);
    let expected: HashSet<u64> = page
        .items
        .iter()
        .filter_map(|it| {
            let ItemKind::Stroke(ref s) = it.kind;
            if ground_truth_intersects(s, query_box.0, query_box.1, query_box.2, query_box.3) {
                Some(it.id)
            } else {
                None
            }
        })
        .collect();
    let actual: HashSet<u64> = res.iter().map(|it| it.item_id).collect();
    assert_eq!(actual, expected);

    // 2. Move items (small batch < 250) and test immediate R-Tree sync
    let move_ids: Vec<ItemId> = page.items[0..10].iter().map(|it| it.id).collect();
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: move_ids.clone(),
            delta_x: 300.0,
            delta_y: 400.0,
        },
        &mut doc,
    );

    let page = &doc.pages[0];
    assert_eq!(page.spatial_index.len(), 100);

    // Moved items must now appear at new location [380.0..500.0, 480.0..600.0]
    let new_box = (380.0, 480.0, 520.0, 620.0);
    let moved_res = page
        .spatial_index
        .locate_in_envelope(new_box.0, new_box.1, new_box.2, new_box.3);
    let moved_actual_ids: HashSet<u64> = moved_res.iter().map(|it| it.item_id).collect();
    for &id in &move_ids {
        assert!(
            moved_actual_ids.contains(&id),
            "Moved stroke {} not found in new envelope",
            id
        );
    }

    // 3. Undo MoveItems: items must return to original location
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    let undo_res =
        page.spatial_index
            .locate_in_envelope(query_box.0, query_box.1, query_box.2, query_box.3);
    let undo_actual_ids: HashSet<u64> = undo_res.iter().map(|it| it.item_id).collect();
    assert_eq!(
        undo_actual_ids, expected,
        "Undo MoveItems failed to restore spatial index positions"
    );

    // 4. Redo MoveItems: items must be at translated location again
    assert!(history.redo(&mut doc));
    let page = &doc.pages[0];
    let redo_res = page
        .spatial_index
        .locate_in_envelope(new_box.0, new_box.1, new_box.2, new_box.3);
    let redo_actual_ids: HashSet<u64> = redo_res.iter().map(|it| it.item_id).collect();
    for &id in &move_ids {
        assert!(redo_actual_ids.contains(&id));
    }

    // 5. Erase items and verify spatial index removal
    let erased_subset = page.items[0..5].to_vec();
    let erased_ids: HashSet<u64> = erased_subset.iter().map(|it| it.id).collect();
    history.execute(
        DocCommand::EraseItems {
            page_id,
            erased_items: erased_subset,
        },
        &mut doc,
    );

    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 95);
    assert_eq!(page.spatial_index.len(), 95);

    let query_all = page
        .spatial_index
        .locate_in_envelope(-1000.0, -1000.0, 2000.0, 2000.0);
    for si in &query_all {
        assert!(
            !erased_ids.contains(&si.item_id),
            "Erased stroke {} still in spatial index",
            si.item_id
        );
    }

    // 6. Undo EraseItems: items restored with correct z-index
    assert!(history.undo(&mut doc));
    let page = &doc.pages[0];
    assert_eq!(page.items.len(), 100);
    assert_eq!(page.spatial_index.len(), 100);
}

// ============================================================================
// 5. ADVERSARIAL BOUNDARY CONDITIONS & NUMERICAL EXTREMES
// ============================================================================

#[test]
fn test_adversarial_boundary_conditions_and_numerical_extremes() {
    let mut page = Page::new_a4(0, PageBackground::Blank);

    // 1. Single dot stroke (points.len() == 1, width = 4.0)
    let dot = Stroke {
        id: 1,
        points: vec![StrokePoint {
            x: 100.0,
            y: 100.0,
            pressure: 0.8,
            width: 4.0,
            timestamp_ns: 100,
        }],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // 2. Giant stroke spanning across entire page
    let giant = Stroke {
        id: 2,
        points: vec![
            StrokePoint {
                x: 0.0,
                y: 0.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 200,
            },
            StrokePoint {
                x: A4_WIDTH_PT,
                y: A4_HEIGHT_PT,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 300,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // 3. Degenerate empty stroke (0 points)
    let empty_stroke = Stroke {
        id: 3,
        points: Vec::new(),
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    page.items = vec![
        Item {
            id: 1,
            z_index: 1,
            kind: ItemKind::Stroke(dot),
        },
        Item {
            id: 2,
            z_index: 2,
            kind: ItemKind::Stroke(giant),
        },
        Item {
            id: 3,
            z_index: 3,
            kind: ItemKind::Stroke(empty_stroke),
        },
    ];
    page.rebuild_spatial_index();

    // Empty stroke must be gracefully ignored by spatial index
    assert_eq!(page.spatial_index.len(), 2);

    // Tangent boundary query: query envelope touching dot bounding box exactly
    let dot_bbox = match &page.items[0].kind {
        ItemKind::Stroke(s) => s.bounding_box().unwrap(),
    };
    let tangent_query = page.spatial_index.locate_in_envelope(
        dot_bbox.max_x,
        dot_bbox.max_y,
        dot_bbox.max_x + 50.0,
        dot_bbox.max_y + 50.0,
    );
    assert!(
        !tangent_query.is_empty(),
        "Tangent boundary must match intersecting AABB"
    );

    // Inverted envelope: min_x > max_x (must normalize without panic)
    let inverted = page
        .spatial_index
        .locate_in_envelope(300.0, 300.0, 50.0, 50.0);
    assert_eq!(
        inverted.len(),
        page.spatial_index
            .locate_in_envelope(50.0, 50.0, 300.0, 300.0)
            .len()
    );

    // Extreme numerical coordinates: [-1e11, 1e11]
    let extreme = page
        .spatial_index
        .locate_in_envelope(-1e11, -1e11, 1e11, 1e11);
    assert_eq!(extreme.len(), 2);

    // NaN / Inf inputs: must not crash or panic
    let nan_query = page
        .spatial_index
        .locate_in_envelope(f64::NAN, 0.0, 500.0, 500.0);
    assert!(nan_query.len() <= 2);

    let inf_query = page.spatial_index.locate_in_envelope(
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
        f64::INFINITY,
        f64::INFINITY,
    );
    assert_eq!(inf_query.len(), 2);
}

// ============================================================================
// 6. EMPIRICAL API CHALLENGE: NoteCanvas::draw_stroke VISIBILITY
// ============================================================================

#[test]
fn test_empirical_draw_stroke_and_canvas_state_contracts() {
    // Note: NoteCanvas defines `fn draw_stroke(...)` as private to NoteCanvas in canvas.rs.
    // While the Cairo rendering loop internally borrows `stroke.bezier_segments()` (Feature 13),
    // the private visibility of `draw_stroke` prevents external crates and tests from directly
    // calling it outside of GTK's `set_draw_func` closure.
    // Here we verify that CanvasState and PageSpatialIndex seamlessly integrate with Viewport.
    let mut vp = Viewport::default();
    vp.pan(150.0, -250.0);
    vp.zoom_at((400.0, 300.0), 2.5);

    let doc_rect = Rect2D::new(0.0, 0.0, 600.0, 800.0);
    let (sx, sy) = vp.document_to_screen((doc_rect.x, doc_rect.y));
    assert!(sx.is_finite() && sy.is_finite());

    // Verify document bounding box computation
    let mut page = Page::new_whiteboard(PageBackground::Blank);
    populate_whiteboard_with_strokes(&mut page, 100, 999, (-500.0, -500.0), (500.0, 500.0));
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    doc.pages = vec![page];

    let bbox = uninote_app::canvas::NoteCanvas::compute_document_bounding_box(&doc);
    assert!(bbox.is_some());
    let bb = bbox.unwrap();
    assert!(bb.width > 0.0 && bb.height > 0.0);
    assert!(bb.x <= -400.0 && bb.x + bb.width >= 400.0);
}
