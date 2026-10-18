use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use uninote_stroke::geometry::CubicBezierSegment;
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Stroke, StrokePoint, StrokeStyle,
};

thread_local! {
    static TRACKING_ACTIVE: Cell<bool> = const { Cell::new(false) };
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
    static THREAD_BYTES: Cell<usize> = const { Cell::new(0) };
}

struct TrackingAlloc;

unsafe impl GlobalAlloc for TrackingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        TRACKING_ACTIVE.with(|active| {
            if active.get() {
                THREAD_ALLOCS.with(|c| c.set(c.get() + 1));
                THREAD_BYTES.with(|b| b.set(b.get() + layout.size()));
            }
        });
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAlloc = TrackingAlloc;

fn track_allocations<R, F: FnOnce() -> R>(f: F) -> (R, usize, usize) {
    THREAD_ALLOCS.with(|c| c.set(0));
    THREAD_BYTES.with(|b| b.set(0));
    TRACKING_ACTIVE.with(|a| a.set(true));
    let result = f();
    TRACKING_ACTIVE.with(|a| a.set(false));
    let count = THREAD_ALLOCS.with(|c| c.get());
    let bytes = THREAD_BYTES.with(|b| b.get());
    (result, count, bytes)
}

fn generate_stroke(point_count: usize, mode: PipelineMode) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    let mut raw_samples = Vec::with_capacity(point_count);

    for i in 0..point_count {
        let t = i as f64;
        let x = 100.0 + t * 2.5;
        let y = 200.0 + (t * 0.1).sin() * 50.0 + (t * 0.02).cos() * 20.0;
        let pressure = 0.2 + 0.6 * ((t * 0.05).sin().abs());
        let width = 2.0 + pressure * 2.5;

        points.push(StrokePoint {
            x,
            y,
            pressure,
            width,
            timestamp_ns: (i * 5_000_000) as u64,
        });

        raw_samples.push(InputSample::new(
            (x, y),
            Some(pressure),
            (i * 5_000_000) as u64,
            DeviceKind::Stylus,
            if i == 0 {
                InputPhase::Down
            } else if i == point_count - 1 {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        ));
    }

    let mut stroke = Stroke {
        id: 101,
        style: StrokeStyle::default(),
        points,
        raw_samples,
        is_dot: point_count == 1,
        mode,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// =========================================================================
// 1. SANITY CHECK: ALLOCATION TRACKER SENSITIVITY
// =========================================================================

#[test]
fn test_sanity_tracker_detects_allocations() {
    let (v, allocs, bytes) = track_allocations(|| {
        let mut data = Vec::new();
        for i in 0..100 {
            data.push(i);
        }
        data
    });
    assert_eq!(v.len(), 100);
    assert!(
        allocs > 0,
        "Tracking allocator must detect heap allocations (got 0)"
    );
    assert!(
        bytes > 0,
        "Tracking allocator must detect bytes allocated (got 0)"
    );
}

// =========================================================================
// 2. ZERO ALLOCATION TRANSLATION TESTS
// =========================================================================

#[test]
fn test_stroke_translate_zero_allocations_10_points() {
    let mut stroke = generate_stroke(10, PipelineMode::CatmullRomSpline);
    assert!(stroke.cached_bbox.is_some());
    assert_eq!(stroke.cached_bezier.len(), 9);

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..100 {
            stroke.translate(12.5, -8.75);
        }
    });

    assert_eq!(
        allocs, 0,
        "Stroke::translate on 10 points must perform 0 allocations (got {})",
        allocs
    );
    assert_eq!(
        bytes, 0,
        "Stroke::translate on 10 points must allocate 0 bytes (got {})",
        bytes
    );
}

#[test]
fn test_stroke_translate_zero_allocations_1000_points() {
    let mut stroke = generate_stroke(1_000, PipelineMode::CatmullRomSpline);
    assert!(stroke.cached_bbox.is_some());
    assert_eq!(stroke.cached_bezier.len(), 999);

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..500 {
            stroke.translate(5.0, 5.0);
            stroke.translate(-5.0, -5.0);
        }
    });

    assert_eq!(
        allocs, 0,
        "Stroke::translate on 1,000 points must perform 0 allocations (got {})",
        allocs
    );
    assert_eq!(
        bytes, 0,
        "Stroke::translate on 1,000 points must allocate 0 bytes (got {})",
        bytes
    );
}

#[test]
fn test_stroke_translate_zero_allocations_10000_points() {
    let mut stroke = generate_stroke(10_000, PipelineMode::CatmullRomSpline);
    assert!(stroke.cached_bbox.is_some());
    assert_eq!(stroke.cached_bezier.len(), 9_999);

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..1_000 {
            stroke.translate(1.25, -1.25);
            stroke.translate(-1.25, 1.25);
        }
    });

    assert_eq!(
        allocs, 0,
        "Stroke::translate on 10,000 points must perform 0 allocations across 2000 calls (got {})",
        allocs
    );
    assert_eq!(
        bytes, 0,
        "Stroke::translate on 10,000 points must allocate 0 bytes across 2000 calls (got {})",
        bytes
    );
}

#[test]
fn test_stroke_translate_zero_allocations_unwarmed_stroke() {
    let mut stroke = Stroke {
        id: 42,
        style: StrokeStyle::default(),
        points: vec![
            StrokePoint {
                x: 10.0,
                y: 10.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 20.0,
                y: 20.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1,
            },
        ],
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    let (_, allocs, bytes) = track_allocations(|| {
        stroke.translate(10.0, -10.0);
    });

    assert_eq!(allocs, 0);
    assert_eq!(bytes, 0);
}

#[test]
fn test_stroke_translate_zero_allocations_single_dot() {
    let mut stroke = Stroke {
        id: 7,
        style: StrokeStyle::default(),
        points: vec![StrokePoint {
            x: 50.0,
            y: 50.0,
            pressure: 0.5,
            width: 3.0,
            timestamp_ns: 0,
        }],
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..1_000 {
            stroke.translate(2.0, 3.0);
        }
    });

    assert_eq!(allocs, 0);
    assert_eq!(bytes, 0);
}

#[test]
fn test_stroke_translate_zero_allocations_baseline_mode() {
    let mut stroke = generate_stroke(50, PipelineMode::Baseline);
    assert!(stroke.cached_bezier.is_empty());

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..500 {
            stroke.translate(-15.0, 30.0);
        }
    });

    assert_eq!(allocs, 0);
    assert_eq!(bytes, 0);
}

#[test]
fn test_stroke_translate_zero_allocations_one_euro_mode() {
    let mut stroke = generate_stroke(50, PipelineMode::OneEuroFiltered);
    assert!(!stroke.cached_bezier.is_empty());

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..500 {
            stroke.translate(15.0, -30.0);
        }
    });

    assert_eq!(allocs, 0);
    assert_eq!(bytes, 0);
}

// =========================================================================
// 3. ZERO ALLOCATION BOUNDING_BOX INSPECTION
// =========================================================================

#[test]
fn test_stroke_bounding_box_zero_allocations_warmed() {
    let stroke = generate_stroke(1_000, PipelineMode::CatmullRomSpline);
    assert!(stroke.cached_bbox.is_some());

    let (bbox, allocs, bytes) = track_allocations(|| {
        let mut last_box = None;
        for _ in 0..10_000 {
            last_box = stroke.bounding_box();
        }
        last_box
    });

    assert!(bbox.is_some());
    assert_eq!(
        allocs, 0,
        "Stroke::bounding_box() with warmed cache must perform 0 allocations (got {})",
        allocs
    );
    assert_eq!(
        bytes, 0,
        "Stroke::bounding_box() with warmed cache must allocate 0 bytes (got {})",
        bytes
    );
}

// =========================================================================
// 4. CUBIC BEZIER SEGMENT ZERO ALLOCATION TRANSLATION
// =========================================================================

#[test]
fn test_cubic_bezier_segment_translate_zero_allocations() {
    let mut seg = CubicBezierSegment {
        p0: (10.0, 20.0),
        cp1: (15.0, 25.0),
        cp2: (25.0, 35.0),
        p1: (30.0, 40.0),
        width0: 2.0,
        width1: 4.0,
    };

    let (_, allocs, bytes) = track_allocations(|| {
        for _ in 0..100_000 {
            seg.translate(1.0, -1.0);
        }
    });

    assert_eq!(
        allocs, 0,
        "CubicBezierSegment::translate must perform 0 allocations (got {})",
        allocs
    );
    assert_eq!(
        bytes, 0,
        "CubicBezierSegment::translate must allocate 0 bytes (got {})",
        bytes
    );
}

// =========================================================================
// 5. GEOMETRIC INVARIANCE ACROSS 10,000 TRANSLATION CYCLES
// =========================================================================

#[test]
fn test_stroke_translate_preserves_bezier_continuity_and_tangents_10k_cycles() {
    let mut stroke = generate_stroke(20, PipelineMode::CatmullRomSpline);
    let orig_bezier = stroke.cached_bezier.clone();
    let dx = std::f64::consts::PI;
    let dy = -std::f64::consts::E;

    for _ in 0..10_000 {
        stroke.translate(dx, dy);
        stroke.translate(-dx, -dy);
    }

    // Verify each segment matches original control points within IEEE-754 precision (< 1e-11)
    for (seg_curr, seg_orig) in stroke.cached_bezier.iter().zip(orig_bezier.iter()) {
        assert!((seg_curr.p0.0 - seg_orig.p0.0).abs() < 1e-11);
        assert!((seg_curr.p0.1 - seg_orig.p0.1).abs() < 1e-11);
        assert!((seg_curr.cp1.0 - seg_orig.cp1.0).abs() < 1e-11);
        assert!((seg_curr.cp1.1 - seg_orig.cp1.1).abs() < 1e-11);
        assert!((seg_curr.cp2.0 - seg_orig.cp2.0).abs() < 1e-11);
        assert!((seg_curr.cp2.1 - seg_orig.cp2.1).abs() < 1e-11);
        assert!((seg_curr.p1.0 - seg_orig.p1.0).abs() < 1e-11);
        assert!((seg_curr.p1.1 - seg_orig.p1.1).abs() < 1e-11);
        assert_eq!(seg_curr.width0, seg_orig.width0);
        assert_eq!(seg_curr.width1, seg_orig.width1);
    }

    // Verify C1 continuity between consecutive Bézier segments
    for i in 0..stroke.cached_bezier.len() - 1 {
        let seg_a = &stroke.cached_bezier[i];
        let seg_b = &stroke.cached_bezier[i + 1];

        // Shared endpoint: p1 of seg_a must match p0 of seg_b
        assert!((seg_a.p1.0 - seg_b.p0.0).abs() < 1e-13);
        assert!((seg_a.p1.1 - seg_b.p0.1).abs() < 1e-13);

        // Tangent continuity: (p1 - cp2) on A and (cp1 - p0) on B must be collinear
        let tan_a = (seg_a.p1.0 - seg_a.cp2.0, seg_a.p1.1 - seg_a.cp2.1);
        let tan_b = (seg_b.cp1.0 - seg_b.p0.0, seg_b.cp1.1 - seg_b.p0.1);
        let cross = tan_a.0 * tan_b.1 - tan_a.1 * tan_b.0;
        assert!(
            cross.abs() < 1e-9,
            "C1 Tangent continuity violated at segment boundary {}: cross={:e}",
            i,
            cross
        );
    }
}
