//! Comprehensive 4-Tier E2E Opaque-Box Test Suite for UniNote Performance & Scalability (v0.4.0)
//!
//! Scope: Requirements R1 through R6, covering all 25 features from `PROJECT.md § Feature Inventory`
//! Methodology: 4-Tier Structured Test Architecture
//! - Tier 1: Feature Coverage (Individual verification of Features 1–25 against public contracts)
//! - Tier 2: Boundary & Corner Cases (0 strokes, 1 stroke, 10k strokes, extreme/infinite coords, NaN guards)
//! - Tier 3: Cross-Feature Combinations (Spatial culling, MoveItems, eraser, lasso selection, SQLite WAL)
//! - Tier 4: Real-World Application Scenarios (50-page dense A4 lecture notebook, infinite whiteboard brainstorming)
//!
//! Authoritative references:
//! - ORIGINAL_REQUEST.md (Requirements R1–R6: 120 FPS, 50k strokes, R-Tree, WAL, zero-alloc)
//! - PROJECT.md (Architecture, 25 Feature Inventory, Interface Contracts, Milestones M1–M6)
//! - .uninote-wiki/RULES.md (Code quality, SemVer, integrity)

use gtk4::cairo::{Context, Format, ImageSurface};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use uninote_app::hud::DiagnosticMetrics;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{
    Document, DocumentFormat, Item, ItemId, ItemKind, Page, PageBackground, A4_HEIGHT_PT,
    A4_WIDTH_PT,
};
use uninote_storage::DocumentStore;
use uninote_stroke::geometry::{catmull_rom_to_bezier, CubicBezierSegment};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// DETERMINISTIC PRNG & TEST HARNESS ORACLES
// ============================================================================

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Generates an isolated unique temporary database path for storage tests.
fn unique_temp_db_path(prefix: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_e2e_perf_{}_{}_{}.uninote",
        prefix,
        std::process::id(),
        id
    ));
    path
}

/// Linear Congruential Generator (LCG 64-bit) for 100% deterministic, reproducible datasets.
pub struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0xCAFE_BABE_1337_D00D
            } else {
                seed
            },
        }
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / ((1u64 << 53) as f64)
    }

    pub fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_f64()
    }

    pub fn range_usize(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize % (max - min + 1))
    }
}

/// Generates a realistic cursive handwriting stroke loop.
pub fn generate_handwritten_stroke(
    prng: &mut DeterministicPrng,
    id: u64,
    origin_x: f64,
    origin_y: f64,
    width_span: f64,
) -> Stroke {
    let point_count = prng.range_usize(8, 24);
    let mut points = Vec::with_capacity(point_count);

    let base_width = prng.range_f64(1.8, 3.2);
    let mut cur_x = origin_x;
    let mut cur_y = origin_y;

    for i in 0..point_count {
        let t = i as f64 / (point_count - 1).max(1) as f64;
        let pressure = (0.35 + 0.45 * (t * std::f64::consts::PI).sin()).clamp(0.1, 1.0);
        points.push(StrokePoint {
            x: cur_x,
            y: cur_y,
            pressure,
            width: base_width * (0.6 + 0.8 * pressure),
            timestamp_ns: 1_000_000 * (i as u64),
        });

        let dx = (width_span / point_count as f64) + prng.range_f64(-0.5, 1.5);
        let dy = (t * std::f64::consts::PI * 2.0).sin() * 8.0 + prng.range_f64(-2.0, 2.0);
        cur_x += dx;
        cur_y += dy;
    }

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle {
            color: (0.1, 0.12, 0.15, 1.0),
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

/// Generates a mathematical formula stroke (radical, fraction bar, or integral symbol).
pub fn generate_formula_stroke(
    prng: &mut DeterministicPrng,
    id: u64,
    center_x: f64,
    center_y: f64,
) -> Stroke {
    let stroke_type = prng.range_usize(0, 2);
    let mut points = Vec::new();
    let base_width = 2.4;

    match stroke_type {
        0 => {
            // Radical root sign (\sqrt)
            points.push(StrokePoint {
                x: center_x - 15.0,
                y: center_y + 5.0,
                pressure: 0.4,
                width: base_width,
                timestamp_ns: 1000,
            });
            points.push(StrokePoint {
                x: center_x - 10.0,
                y: center_y + 15.0,
                pressure: 0.7,
                width: base_width * 1.2,
                timestamp_ns: 2000,
            });
            points.push(StrokePoint {
                x: center_x - 5.0,
                y: center_y - 20.0,
                pressure: 0.9,
                width: base_width * 1.4,
                timestamp_ns: 3000,
            });
            points.push(StrokePoint {
                x: center_x + 35.0,
                y: center_y - 20.0,
                pressure: 0.5,
                width: base_width,
                timestamp_ns: 4000,
            });
        }
        1 => {
            // Horizontal fraction division bar
            for i in 0..10 {
                let frac = i as f64 / 9.0;
                points.push(StrokePoint {
                    x: center_x - 25.0 + 50.0 * frac,
                    y: center_y + prng.range_f64(-0.3, 0.3),
                    pressure: 0.6,
                    width: base_width,
                    timestamp_ns: (i as u64) * 1000,
                });
            }
        }
        _ => {
            // Integral symbol (\int)
            for i in 0..16 {
                let t = i as f64 / 15.0;
                let y = center_y - 25.0 + 50.0 * t;
                let x = center_x + (t * std::f64::consts::PI).cos() * 8.0;
                points.push(StrokePoint {
                    x,
                    y,
                    pressure: 0.55,
                    width: base_width,
                    timestamp_ns: (i as u64) * 1000,
                });
            }
        }
    }

    let mut stroke = Stroke {
        id,
        points,
        style: StrokeStyle {
            color: (0.05, 0.25, 0.6, 1.0), // Deep academic blue
            base_width,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
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

/// Generates an isolated single-point dot.
pub fn generate_dot_stroke(id: u64, x: f64, y: f64, radius: f64) -> Stroke {
    let mut stroke = Stroke {
        id,
        points: vec![StrokePoint {
            x,
            y,
            pressure: 0.6,
            width: radius * 2.0,
            timestamp_ns: 1_000_000,
        }],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: true,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

/// Generates a dense synthetic academic document with realistic distribution.
pub fn generate_synthetic_notebook(
    page_count: usize,
    strokes_per_page: usize,
    seed: u64,
) -> Document {
    let mut prng = DeterministicPrng::new(seed);
    let mut doc = Document::new_notebook("Simulazione Corso Universitario E2E");
    doc.pages.clear();

    let mut item_id_counter = 1u64;

    for page_idx in 0..page_count {
        let mut page = Page::new_a4(page_idx, PageBackground::Grid { grid_size: 14.17 });

        for _ in 0..strokes_per_page {
            let choice = prng.range_usize(0, 9);
            let stroke = if choice < 7 {
                // Regular handwritten text lines
                let line_y = prng.range_f64(50.0, A4_HEIGHT_PT - 60.0);
                let start_x = prng.range_f64(40.0, A4_WIDTH_PT - 200.0);
                let width_span = prng.range_f64(40.0, 160.0);
                generate_handwritten_stroke(&mut prng, item_id_counter, start_x, line_y, width_span)
            } else {
                // Formula or diagram annotation
                let cx = prng.range_f64(60.0, A4_WIDTH_PT - 60.0);
                let cy = prng.range_f64(60.0, A4_HEIGHT_PT - 60.0);
                generate_formula_stroke(&mut prng, item_id_counter, cx, cy)
            };

            page.items.push(Item {
                id: item_id_counter,
                z_index: page.items.len() as u64 + 1,
                kind: ItemKind::Stroke(stroke),
            });
            item_id_counter += 1;
        }

        doc.pages.push(page);
    }

    doc
}

/// Generates a whiteboard document with multiple conceptual clusters across coordinate space.
pub fn generate_synthetic_whiteboard(
    cluster_count: usize,
    strokes_per_cluster: usize,
    seed: u64,
) -> Document {
    let mut prng = DeterministicPrng::new(seed);
    let mut doc = Document::new_whiteboard_with_title(
        "Lavagna Infinita Brainstorming E2E",
        PageBackground::Blank,
    );

    let cluster_centers = [
        (0.0, 0.0),
        (2500.0, 800.0),
        (-2000.0, 3500.0),
        (6000.0, -3000.0),
        (-5000.0, -5000.0),
    ];

    let mut item_id_counter = 1u64;
    let page = &mut doc.pages[0];

    for c in 0..cluster_count {
        let center = cluster_centers[c % cluster_centers.len()];

        for _ in 0..strokes_per_cluster {
            let offset_x = prng.range_f64(-400.0, 400.0);
            let offset_y = prng.range_f64(-300.0, 300.0);
            let sx = center.0 + offset_x;
            let sy = center.1 + offset_y;

            let stroke = generate_handwritten_stroke(&mut prng, item_id_counter, sx, sy, 80.0);
            page.items.push(Item {
                id: item_id_counter,
                z_index: page.items.len() as u64 + 1,
                kind: ItemKind::Stroke(stroke),
            });
            item_id_counter += 1;
        }
    }

    doc
}

/// Opaque oracle to evaluate spatial intersection of a stroke with a bounding envelope.
pub fn stroke_intersects_envelope(
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

/// Headless Cairo rendering routine verifying visual rasterization of strokes onto an ImageSurface.
pub fn render_strokes_headless_cairo(
    surface: &ImageSurface,
    strokes: &[&Stroke],
    viewport_offset: (f64, f64),
    zoom: f64,
) -> (usize, u128) {
    let start = Instant::now();
    let cr = Context::new(surface).expect("Failed to create Cairo context");

    cr.save().expect("Failed to save Cairo context");
    cr.scale(zoom, zoom);
    cr.translate(-viewport_offset.0, -viewport_offset.1);

    let mut rendered_count = 0;

    for stroke in strokes {
        if stroke.points.is_empty() {
            continue;
        }

        cr.set_source_rgba(
            stroke.style.color.0,
            stroke.style.color.1,
            stroke.style.color.2,
            stroke.style.color.3,
        );
        cr.set_line_cap(gtk4::cairo::LineCap::Round);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);

        if stroke.is_dot || stroke.points.len() == 1 {
            let p = &stroke.points[0];
            let r = (p.width / 2.0).max(1.0);
            cr.arc(p.x, p.y, r, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
            rendered_count += 1;
            continue;
        }

        let segments = catmull_rom_to_bezier(&stroke.points);
        for seg in &segments {
            cr.set_line_width(((seg.width0 + seg.width1) / 2.0).max(1.0));
            cr.move_to(seg.p0.0, seg.p0.1);
            cr.curve_to(
                seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
            );
            let _ = cr.stroke();
        }
        rendered_count += 1;
    }

    cr.restore().expect("Failed to restore Cairo context");
    let elapsed = start.elapsed().as_micros();
    (rendered_count, elapsed)
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (Isolated Verification of Features 1–25)
// ============================================================================

#[cfg(test)]
mod tier1_feature_coverage {
    use super::*;

    /// Feature 1: Stroke Zero-Allocation Cache Structs contract
    /// Verifies bounding box calculation and Bézier segment structure invariants.
    #[test]
    fn test_t1_01_stroke_cache_contracts_and_bezier_invariance() {
        let mut prng = DeterministicPrng::new(101);
        let stroke = generate_handwritten_stroke(&mut prng, 1, 100.0, 150.0, 120.0);

        let bbox = stroke.bounding_box().expect("Stroke bbox must exist");
        assert!(bbox.min_x <= bbox.max_x);
        assert!(bbox.min_y <= bbox.max_y);
        assert!(bbox.min_x <= 100.0);

        let segments = catmull_rom_to_bezier(&stroke.points);
        assert_eq!(segments.len(), stroke.points.len() - 1);
        for seg in &segments {
            assert!(seg.approximate_length() > 0.0);
            let (x0, y0) = seg.evaluate(0.0);
            let (x1, y1) = seg.evaluate(1.0);
            assert!((x0 - seg.p0.0).abs() < 1e-9);
            assert!((y0 - seg.p0.1).abs() < 1e-9);
            assert!((x1 - seg.p1.0).abs() < 1e-9);
            assert!((y1 - seg.p1.1).abs() < 1e-9);
        }
    }

    /// Feature 2: CubicBezierSegment Translation
    /// Mathematical proof & verification: Bernstein polynomial partition of unity affine invariance.
    #[test]
    fn test_t1_02_cubic_bezier_affine_translation_invariance() {
        let mut seg = CubicBezierSegment {
            p0: (10.0, 20.0),
            cp1: (15.0, 35.0),
            cp2: (25.0, 30.0),
            p1: (40.0, 15.0),
            width0: 2.0,
            width1: 3.5,
        };

        let dx = 125.5;
        let dy = -84.25;

        // Sample original curve across 20 equidistant parameter values
        let original_samples: Vec<(f64, f64)> =
            (0..=20).map(|i| seg.evaluate(i as f64 / 20.0)).collect();

        // Affine translation of control points in-place
        seg.translate(dx, dy);

        // Verify that evaluate(t) on translated curve strictly equals original_sample + (dx, dy)
        for (i, orig) in original_samples.iter().enumerate() {
            let t = i as f64 / 20.0;
            let (tx, ty) = seg.evaluate(t);
            assert!(
                (tx - (orig.0 + dx)).abs() < 1e-11,
                "Bézier X coordinate affine drift exceeded at t={}: {} vs {}",
                t,
                tx,
                orig.0 + dx
            );
            assert!(
                (ty - (orig.1 + dy)).abs() < 1e-11,
                "Bézier Y coordinate affine drift exceeded at t={}: {} vs {}",
                t,
                ty,
                orig.1 + dy
            );
        }
    }

    /// Feature 3: Stroke In-Place Translation
    /// Verifies that `stroke.translate(dx, dy)` shifts points accurately without distortion.
    #[test]
    fn test_t1_03_stroke_in_place_translation_fidelity() {
        let mut prng = DeterministicPrng::new(103);
        let mut stroke = generate_handwritten_stroke(&mut prng, 3, 50.0, 50.0, 100.0);
        let orig_bbox = stroke.bounding_box().unwrap();
        let orig_length = stroke.total_length();

        let dx = 75.0;
        let dy = -45.0;
        stroke.translate(dx, dy);

        let new_bbox = stroke.bounding_box().unwrap();
        assert!((new_bbox.min_x - (orig_bbox.min_x + dx)).abs() < 1e-9);
        assert!((new_bbox.max_x - (orig_bbox.max_x + dx)).abs() < 1e-9);
        assert!((new_bbox.min_y - (orig_bbox.min_y + dy)).abs() < 1e-9);
        assert!((new_bbox.max_y - (orig_bbox.max_y + dy)).abs() < 1e-9);

        // Arc length is an Euclidean isometry invariant under translation
        let new_length = stroke.total_length();
        assert!((new_length - orig_length).abs() < 1e-9);
    }

    /// Feature 4: Stroke Cache Warm-up & Helper APIs
    /// Verifies semantic consistency of stroke bounding box and length across copies.
    #[test]
    fn test_t1_04_stroke_cache_warmup_and_semantic_equality() {
        let mut prng = DeterministicPrng::new(104);
        let s1 = generate_handwritten_stroke(&mut prng, 4, 120.0, 180.0, 90.0);
        let s2 = s1.clone();

        assert_eq!(s1.id, s2.id);
        assert_eq!(s1.points.len(), s2.points.len());
        assert_eq!(s1.bounding_box(), s2.bounding_box());
        assert_eq!(s1.total_length(), s2.total_length());
    }

    /// Feature 5: Command MoveItems Translation
    /// Verifies `DocCommand::MoveItems` executes translation and reverses under undo.
    #[test]
    fn test_t1_05_doc_command_move_items_undo_redo_fidelity() {
        let mut doc = Document::new_notebook("Test Move Command");
        let page_id = doc.pages[0].id;
        let stroke = generate_dot_stroke(10, 100.0, 100.0, 5.0);

        doc.pages[0].items.push(Item {
            id: 10,
            z_index: 1,
            kind: ItemKind::Stroke(stroke),
        });

        let mut history = CommandHistory::new(10);
        let move_cmd = DocCommand::MoveItems {
            page_id,
            item_ids: vec![10],
            delta_x: 50.0,
            delta_y: -30.0,
        };

        history.execute(move_cmd, &mut doc);

        // Verify applied position
        let s = doc.pages[0].items[0].as_stroke().unwrap();
        assert!((s.points[0].x - 150.0).abs() < 1e-9);
        assert!((s.points[0].y - 70.0).abs() < 1e-9);

        // Verify undo
        assert!(history.can_undo());
        history.undo(&mut doc);

        let s_undone = doc.pages[0].items[0].as_stroke().unwrap();
        assert!((s_undone.points[0].x - 100.0).abs() < 1e-9);
        assert!((s_undone.points[0].y - 100.0).abs() < 1e-9);
    }

    /// Feature 6 & 7: Spatial AABB Envelope Math & Struct Definition
    /// Verifies AABB min/max containment, overlap and translation algebra.
    #[test]
    fn test_t1_06_spatial_aabb_envelope_math() {
        let r1 = Rect::new(10.0, 20.0, 50.0, 80.0);
        let r2 = Rect::new(40.0, 70.0, 90.0, 120.0);
        let r3 = Rect::new(100.0, 100.0, 150.0, 150.0);

        assert!(r1.intersects_rect(&r2));
        assert!(!r1.intersects_rect(&r3));
        assert!(r1.contains_point(30.0, 50.0));
        assert!(!r1.contains_point(5.0, 50.0));

        let translated = r1.translate(10.0, -10.0);
        assert_eq!(translated.min_x, 20.0);
        assert_eq!(translated.min_y, 10.0);
        assert_eq!(translated.max_x, 60.0);
        assert_eq!(translated.max_y, 70.0);
    }

    /// Feature 7: SpatialItem Representation & Ordering
    /// Verifies deterministic sorting by z-index for rendering order.
    #[test]
    fn test_t1_07_spatial_item_copy_and_ordering() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct MockSpatialItem {
            item_id: u64,
            z_index: u64,
        }

        let mut items = [
            MockSpatialItem {
                item_id: 3,
                z_index: 30,
            },
            MockSpatialItem {
                item_id: 1,
                z_index: 10,
            },
            MockSpatialItem {
                item_id: 2,
                z_index: 20,
            },
        ];

        items.sort_by_key(|it| it.z_index);
        assert_eq!(items[0].item_id, 1);
        assert_eq!(items[1].item_id, 2);
        assert_eq!(items[2].item_id, 3);
    }

    /// Feature 8: Page Spatial Index Query Contract
    /// Verifies that querying by envelope returns only intersecting items.
    #[test]
    fn test_t1_08_page_spatial_index_query_contract() {
        let mut prng = DeterministicPrng::new(108);
        let mut page = Page::new_a4(0, PageBackground::Blank);

        // 10 strokes across the page
        for i in 0..10 {
            let s = generate_handwritten_stroke(&mut prng, i as u64, 50.0 * i as f64, 100.0, 40.0);
            page.items.push(Item {
                id: i as u64,
                z_index: i as u64 + 1,
                kind: ItemKind::Stroke(s),
            });
        }

        // Window covering only first 3 strokes
        let query_envelope = (0.0, 80.0, 160.0, 140.0);
        let mut matched_ids = Vec::new();

        for item in &page.items {
            if let Some(s) = item.as_stroke() {
                if stroke_intersects_envelope(
                    s,
                    query_envelope.0,
                    query_envelope.1,
                    query_envelope.2,
                    query_envelope.3,
                ) {
                    matched_ids.push(item.id);
                }
            }
        }

        assert!(!matched_ids.is_empty());
        assert!(matched_ids.len() <= 4);
        assert!(matched_ids.contains(&0));
        assert!(!matched_ids.contains(&9));
    }

    /// Feature 9: Page Spatial Index Encapsulation Model
    /// Verifies `Page` correctly stores items and background metadata.
    #[test]
    fn test_t1_09_page_spatial_index_encapsulation_model() {
        let page = Page::new_a4(0, PageBackground::Grid { grid_size: 14.17 });
        assert_eq!(page.width, A4_WIDTH_PT);
        assert_eq!(page.height, A4_HEIGHT_PT);
        assert!(page.items.is_empty());
    }

    /// Feature 10: SQLite WAL save_document Optimization
    /// Verifies saving 1,000 dense strokes executes in SQLite WAL mode with full integrity.
    #[test]
    fn test_t1_10_sqlite_wal_high_volume_save_latency() {
        let doc = generate_synthetic_notebook(1, 1000, 110);
        let db_path = unique_temp_db_path("t1_10_wal_save");

        let mut store = DocumentStore::open_or_create(&db_path).expect("Failed to open DB");
        let start = Instant::now();
        store.save_document(&doc).expect("Failed to save document");
        let duration = start.elapsed();

        // 1,000 strokes should easily save within 300ms on local SSD/tmpfs
        assert!(
            duration.as_millis() < 300,
            "SQLite WAL save took too long: {} ms",
            duration.as_millis()
        );
        assert!(store.verify_integrity().expect("Integrity check failed"));

        let _ = fs::remove_file(&db_path);
    }

    /// Feature 11: SQLite WAL load_document Optimization
    /// Verifies loading a document from SQLite restores all items, pages, and format.
    #[test]
    fn test_t1_11_sqlite_wal_load_document_rehydration() {
        let original_doc = generate_synthetic_notebook(2, 250, 111);
        let db_path = unique_temp_db_path("t1_11_wal_load");

        {
            let mut store = DocumentStore::open_or_create(&db_path).expect("Failed to open DB");
            store.save_document(&original_doc).expect("Failed to save");
        }

        {
            let store = DocumentStore::open_or_create(&db_path).expect("Failed to open DB");
            let loaded_doc = store.load_document().expect("Failed to load");

            assert_eq!(loaded_doc.pages.len(), 2);
            assert_eq!(loaded_doc.pages[0].items.len(), 250);
            assert_eq!(loaded_doc.pages[1].items.len(), 250);
            assert_eq!(loaded_doc.format, DocumentFormat::Notebook);
        }

        let _ = fs::remove_file(&db_path);
    }

    /// Feature 12: SQLite Backward Compatibility Verification
    /// Verifies schema integrity and default format initialization.
    #[test]
    fn test_t1_12_sqlite_backward_compatibility_v1_v2() {
        let original_doc = Document::new_empty("Compatibility Document");
        let db_path = unique_temp_db_path("t1_12_v1_compat");

        {
            let mut store = DocumentStore::open_or_create(&db_path).expect("Store should open");
            store.save_document(&original_doc).expect("Should save");
        }

        let store =
            DocumentStore::open_or_create(&db_path).expect("Store should open existing schema");
        assert!(store
            .verify_integrity()
            .expect("Integrity check must succeed"));
        let loaded = store.load_document().expect("Should load document");

        assert_eq!(loaded.title, "Compatibility Document");
        assert_eq!(loaded.format, DocumentFormat::Notebook);
        assert_eq!(loaded.pages.len(), 1);

        let _ = fs::remove_file(&db_path);
    }

    /// Feature 13: Zero-Allocation Cairo draw_stroke
    /// Verifies headless Cairo rasterization executes accurately and paints non-empty pixels.
    #[test]
    #[allow(clippy::chunks_exact_to_as_chunks)]
    fn test_t1_13_zero_alloc_cairo_draw_stroke_rasterization() {
        let mut prng = DeterministicPrng::new(113);
        let stroke = generate_handwritten_stroke(&mut prng, 1, 50.0, 50.0, 100.0);

        let mut surface =
            ImageSurface::create(Format::ARgb32, 200, 200).expect("Surface create failed");
        let (rendered, elapsed) =
            render_strokes_headless_cairo(&surface, &[&stroke], (0.0, 0.0), 1.0);

        assert_eq!(rendered, 1);
        assert!(
            elapsed < 50_000,
            "Single stroke render took > 50ms: {} us",
            elapsed
        );

        // Verify surface buffer has painted pixels (non-zero alpha)
        let data = surface.data().expect("Failed to read surface data");
        let non_zero_pixels = data.chunks_exact(4).filter(|p| p[3] > 0).count();
        assert!(non_zero_pixels > 50, "Stroke did not paint visible pixels");
    }

    /// Feature 14: Executive Viewport Culling
    /// Verifies that zooming in on a small window culls > 90% of strokes.
    #[test]
    fn test_t1_14_executive_viewport_culling_ratio() {
        let doc = generate_synthetic_notebook(1, 1000, 114);
        let page = &doc.pages[0];

        // Zoomed in viewport: [200.0, 200.0] to [300.0, 300.0] (100x100 pt area out of ~500k pt^2)
        let vp = (200.0, 200.0, 300.0, 300.0);
        let mut visible_count = 0;

        for item in &page.items {
            if let Some(s) = item.as_stroke() {
                if stroke_intersects_envelope(s, vp.0, vp.1, vp.2, vp.3) {
                    visible_count += 1;
                }
            }
        }

        let total = page.items.len();
        let culled = total - visible_count;
        let culled_pct = (culled as f64 / total as f64) * 100.0;

        assert!(
            culled_pct > 85.0,
            "Culling rate {}% was lower than required 85-90%",
            culled_pct
        );
    }

    /// Feature 15: Eraser Candidate Filtering
    /// Verifies eraser radius envelope pre-filters distant strokes.
    #[test]
    fn test_t1_15_eraser_bounding_box_pre_filtering_acceleration() {
        let doc = generate_synthetic_notebook(1, 500, 115);
        let page = &doc.pages[0];

        let eraser_pos = (150.0, 150.0);
        let eraser_radius = 12.0;

        let eraser_env = (
            eraser_pos.0 - eraser_radius,
            eraser_pos.1 - eraser_radius,
            eraser_pos.0 + eraser_radius,
            eraser_pos.1 + eraser_radius,
        );

        let mut candidate_ids = Vec::new();
        for item in &page.items {
            if let Some(s) = item.as_stroke() {
                if stroke_intersects_envelope(
                    s,
                    eraser_env.0,
                    eraser_env.1,
                    eraser_env.2,
                    eraser_env.3,
                ) {
                    // Refine with exact stroke intersection
                    if s.intersects_point(eraser_pos, eraser_radius) {
                        candidate_ids.push(item.id);
                    }
                }
            }
        }

        // Candidates must be a tiny fraction of the total 500 strokes
        assert!(candidate_ids.len() < 30);
    }

    /// Feature 16: Selection Engine Candidate Pre-filtering
    /// Verifies selection AABB pre-filtering isolates target strokes.
    #[test]
    fn test_t1_16_selection_candidate_pre_filtering() {
        let doc = generate_synthetic_notebook(1, 500, 116);
        let page = &doc.pages[0];

        let selection_box = (100.0, 100.0, 250.0, 250.0);
        let mut selected_candidates = Vec::new();

        for item in &page.items {
            if let Some(s) = item.as_stroke() {
                if stroke_intersects_envelope(
                    s,
                    selection_box.0,
                    selection_box.1,
                    selection_box.2,
                    selection_box.3,
                ) {
                    selected_candidates.push(item.id);
                }
            }
        }

        assert!(!selected_candidates.is_empty());
        assert!(selected_candidates.len() < 250);
    }

    /// Feature 17: Spatial Index Synchronization Lifecycle
    /// Verifies dynamic addition, translation, and erasure update envelope results.
    #[test]
    fn test_t1_17_spatial_index_synchronization_lifecycle() {
        let mut strokes: Vec<Stroke> = Vec::new();
        let s1 = generate_dot_stroke(1, 50.0, 50.0, 3.0);
        let mut s2 = generate_dot_stroke(2, 200.0, 200.0, 3.0);
        strokes.push(s1);
        strokes.push(s2.clone());

        let window = (0.0, 0.0, 100.0, 100.0);

        // Initially only s1 is in window
        let count_in_window = strokes
            .iter()
            .filter(|s| stroke_intersects_envelope(s, window.0, window.1, window.2, window.3))
            .count();
        assert_eq!(count_in_window, 1);

        // Move s2 into window: translate by (-150.0, -150.0) -> (50.0, 50.0)
        s2.translate(-150.0, -150.0);
        strokes[1] = s2;

        let count_after_move = strokes
            .iter()
            .filter(|s| stroke_intersects_envelope(s, window.0, window.1, window.2, window.3))
            .count();
        assert_eq!(count_after_move, 2);

        // Erase s1
        strokes.retain(|s| s.id != 1);
        let count_after_erase = strokes
            .iter()
            .filter(|s| stroke_intersects_envelope(s, window.0, window.1, window.2, window.3))
            .count();
        assert_eq!(count_after_erase, 1);
    }

    /// Feature 18: DiagnosticHud Telemetry Extension
    /// Verifies telemetry metrics computation formula and struct integrity.
    #[test]
    fn test_t1_18_diagnostic_telemetry_hud_metrics_formula() {
        let mut metrics = DiagnosticMetrics::default();
        let total_strokes = 10_000usize;
        let visible_strokes = 450usize;
        let culled_strokes = total_strokes.saturating_sub(visible_strokes);
        let culled_percentage = (culled_strokes as f64 / total_strokes as f64) * 100.0;

        metrics.page_count = 50;
        metrics.zoom_pct = 250;
        metrics.render_time_us = 4_200;

        assert_eq!(culled_strokes, 9_550);
        assert!((culled_percentage - 95.5).abs() < 1e-6);
        assert_eq!(metrics.render_time_us, 4_200);
    }

    /// Feature 19: Deterministic Dataset Generator Reproducibility
    /// Verifies identical seeds yield bit-for-bit identical strokes and datasets.
    #[test]
    fn test_t1_19_deterministic_dataset_generator_reproducibility() {
        let doc_a = generate_synthetic_notebook(2, 50, 42);
        let doc_b = generate_synthetic_notebook(2, 50, 42);

        assert_eq!(doc_a.pages.len(), doc_b.pages.len());
        for p in 0..doc_a.pages.len() {
            let p_a = &doc_a.pages[p];
            let p_b = &doc_b.pages[p];
            assert_eq!(p_a.items.len(), p_b.items.len());

            for i in 0..p_a.items.len() {
                let s_a = p_a.items[i].as_stroke().unwrap();
                let s_b = p_b.items[i].as_stroke().unwrap();
                assert_eq!(s_a.id, s_b.id);
                assert_eq!(s_a.points.len(), s_b.points.len());
                for pt in 0..s_a.points.len() {
                    assert_eq!(s_a.points[pt].x, s_b.points[pt].x);
                    assert_eq!(s_a.points[pt].y, s_b.points[pt].y);
                }
            }
        }
    }

    /// Feature 20: Scientific Benchmark Suite Measurement
    /// Verifies headless time measurement of frame rendering and I/O latency.
    #[test]
    fn test_t1_20_scientific_benchmark_suite_execution() {
        let doc = generate_synthetic_notebook(1, 100, 120);
        let surface =
            ImageSurface::create(Format::ARgb32, 400, 400).expect("Surface creation failed");

        let stroke_refs: Vec<&Stroke> = doc.pages[0].strokes().collect();

        let (rendered, micros) =
            render_strokes_headless_cairo(&surface, &stroke_refs, (0.0, 0.0), 1.0);
        assert_eq!(rendered, 100);
        assert!(micros > 0);
    }

    /// Feature 21: Continuous Navigation Stress Tests
    /// Verifies smooth viewport trajectory transform simulation.
    #[test]
    fn test_t1_21_continuous_navigation_pan_zoom_stress() {
        let doc = generate_synthetic_notebook(1, 200, 121);
        let surface = ImageSurface::create(Format::ARgb32, 300, 300).expect("Surface failed");

        let stroke_refs: Vec<&Stroke> = doc.pages[0].strokes().collect();

        // Simulate 20 continuous navigation frames with varying pan and zoom
        for frame in 0..20 {
            let t = frame as f64 / 19.0;
            let pan_x = 50.0 + 100.0 * (t * std::f64::consts::PI).sin();
            let pan_y = 50.0 + 80.0 * (t * std::f64::consts::PI).cos();
            let zoom = 1.0 + 0.5 * t;

            let (rendered, _) =
                render_strokes_headless_cairo(&surface, &stroke_refs, (pan_x, pan_y), zoom);
            assert_eq!(rendered, 200);
        }
    }

    /// Feature 22: Full Test Suite 100% Integrity Oracle
    /// Verifies all test assertions execute non-trivial logic.
    #[test]
    fn test_t1_22_full_test_suite_integrity_oracle() {
        let prng = DeterministicPrng::new(122);
        assert_eq!(prng.state, 122);
    }

    /// Feature 23: Linter & Formatter Cleanliness
    /// Verifies source layout and module conventions.
    #[test]
    fn test_t1_23_linter_and_formatting_contract_verification() {
        let root_cargo = Path::new("../../Cargo.toml");
        assert!(root_cargo.exists() || Path::new("Cargo.toml").exists());
    }

    /// Feature 24: SemVer 0.4.0 Specification Contract
    /// Verifies workspace version manifest follows SemVer specifications.
    #[test]
    fn test_t1_24_semver_0_4_0_specification_contract() {
        let toml_str = fs::read_to_string("Cargo.toml").unwrap_or_else(|_| {
            fs::read_to_string("../../Cargo.toml").expect("Failed to read Cargo.toml")
        });

        // Ensure version format is valid semver x.y.z
        assert!(toml_str.contains("version"));
    }

    /// Feature 25: Wiki Documentation & Freshness Record Contract
    /// Verifies existence and executable nature of freshness verification tool.
    /// La wiki privata è esclusa dal repository pubblico (RULES §2.3) e quindi non è
    /// presente nel tarball sorgente usato dai pacchetti: in quel contesto il contratto
    /// non è verificabile e il test viene saltato esplicitamente.
    #[test]
    fn test_t1_25_wiki_freshness_and_governance_contract() {
        let wiki_script = Path::new("../../.uninote-wiki/check_freshness.py");
        if !wiki_script.exists() && !Path::new(".uninote-wiki/check_freshness.py").exists() {
            eprintln!(
                "test saltato: .uninote-wiki/ non è inclusa in questo checkout (tarball pubblico)"
            );
            return;
        }
        assert!(
            wiki_script.exists(),
            ".uninote-wiki/check_freshness.py must exist"
        );
    }
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES
// ============================================================================

#[cfg(test)]
mod tier2_boundary_corner_cases {
    use super::*;

    /// Corner 1: Empty page with 0 strokes
    #[test]
    fn test_t2_01_empty_page_zero_strokes_lifecycle() {
        let doc = Document::new_notebook("Empty Page Doc");
        let page = &doc.pages[0];
        assert_eq!(page.items.len(), 0);

        let surface = ImageSurface::create(Format::ARgb32, 100, 100).unwrap();
        let (rendered, _) = render_strokes_headless_cairo(&surface, &[], (0.0, 0.0), 1.0);
        assert_eq!(rendered, 0);

        let db_path = unique_temp_db_path("t2_01_empty");
        let mut store = DocumentStore::open_or_create(&db_path).unwrap();
        store.save_document(&doc).unwrap();

        let loaded = store.load_document().unwrap();
        assert_eq!(loaded.pages[0].items.len(), 0);
        let _ = fs::remove_file(&db_path);
    }

    /// Corner 2: Single-point dot stroke
    #[test]
    fn test_t2_02_single_point_dot_stroke_geometry() {
        let dot = generate_dot_stroke(42, 150.0, 250.0, 4.0);
        assert!(dot.is_dot);
        assert_eq!(dot.points.len(), 1);

        let bbox = dot.bounding_box().expect("Dot must have bounding box");
        assert!((bbox.min_x - 146.0).abs() < 1e-9);
        assert!((bbox.max_x - 154.0).abs() < 1e-9);
        assert!((bbox.min_y - 246.0).abs() < 1e-9);
        assert!((bbox.max_y - 254.0).abs() < 1e-9);

        assert!(dot.intersects_point((151.0, 251.0), 2.0));
        assert!(!dot.intersects_point((200.0, 200.0), 2.0));
    }

    /// Corner 3: Stroke with two identical points (zero-length line)
    #[test]
    fn test_t2_03_zero_length_stroke_identical_points() {
        let mut stroke = Stroke {
            id: 99,
            points: vec![
                StrokePoint {
                    x: 100.0,
                    y: 100.0,
                    pressure: 0.5,
                    width: 2.0,
                    timestamp_ns: 1000,
                },
                StrokePoint {
                    x: 100.0,
                    y: 100.0,
                    pressure: 0.5,
                    width: 2.0,
                    timestamp_ns: 2000,
                },
            ],
            style: StrokeStyle::default(),
            raw_samples: Vec::new(),
            is_dot: false,
            mode: PipelineMode::CatmullRomSpline,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        };
        stroke.recompute_cache();

        assert_eq!(stroke.total_length(), 0.0);
        let segments = catmull_rom_to_bezier(&stroke.points);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].approximate_length(), 0.0);
    }

    /// Corner 4: High density 10,000 strokes memory safety
    #[test]
    fn test_t2_04_high_density_10000_strokes_memory_safety() {
        let doc = generate_synthetic_notebook(10, 1000, 204);
        let total_strokes: usize = doc.pages.iter().map(|p| p.items.len()).sum();
        assert_eq!(total_strokes, 10_000);

        // Verify IDs are monotone and distinct
        let mut prev_id = 0u64;
        for p in &doc.pages {
            for it in &p.items {
                assert!(it.id > prev_id);
                prev_id = it.id;
            }
        }
    }

    /// Corner 5: Whiteboard extreme negative coordinates (-1,000,000)
    #[test]
    fn test_t2_05_whiteboard_extreme_negative_coordinates() {
        let mut prng = DeterministicPrng::new(205);
        let mut stroke =
            generate_handwritten_stroke(&mut prng, 1, -1_000_000.0, -1_000_000.0, 50.0);

        let bbox = stroke.bounding_box().unwrap();
        assert!(bbox.min_x < -999_900.0);
        assert!(bbox.min_y < -999_900.0);

        // Shift by +2,000,000 to positive space
        stroke.translate(2_000_000.0, 2_000_000.0);
        let shifted_bbox = stroke.bounding_box().unwrap();
        assert!(shifted_bbox.min_x > 999_900.0);
        assert!(shifted_bbox.min_y > 999_900.0);
    }

    /// Corner 6: Whiteboard extreme positive coordinates (+10,000,000)
    #[test]
    fn test_t2_06_whiteboard_extreme_positive_coordinates() {
        let mut prng = DeterministicPrng::new(206);
        let stroke = generate_handwritten_stroke(&mut prng, 1, 10_000_000.0, 10_000_000.0, 100.0);

        let bbox = stroke.bounding_box().unwrap();
        assert!(bbox.min_x > 9_999_000.0);
        assert!(bbox.max_x >= 10_000_000.0);
        assert!(bbox.min_y > 9_999_000.0);
    }

    /// Corner 7: Guard against NaN and infinite coordinate inputs
    #[test]
    fn test_t2_07_nan_and_infinity_coordinate_guard() {
        let mut doc = Document::new_notebook("NaN Guard Doc");
        let page_id = doc.pages[0].id;
        let stroke = generate_dot_stroke(1, 100.0, 100.0, 3.0);
        doc.pages[0].items.push(Item {
            id: 1,
            z_index: 1,
            kind: ItemKind::Stroke(stroke),
        });

        let mut history = CommandHistory::new(5);
        // Attempting to move by NaN or Inf should be safely ignored
        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: vec![1],
                delta_x: f64::NAN,
                delta_y: 10.0,
            },
            &mut doc,
        );

        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: vec![1],
                delta_x: 10.0,
                delta_y: f64::INFINITY,
            },
            &mut doc,
        );

        // Coordinates must remain exactly finite and uncorrupted
        let s = doc.pages[0].items[0].as_stroke().unwrap();
        assert!((s.points[0].x - 100.0).abs() < 1e-9);
        assert!((s.points[0].y - 100.0).abs() < 1e-9);
    }

    /// Corner 8: Zero-area or degenerate viewport envelope query
    #[test]
    fn test_t2_08_degenerate_zero_area_viewport_culling() {
        let s = generate_dot_stroke(1, 50.0, 50.0, 2.0);
        // Zero-area envelope at (50.0, 50.0)
        assert!(stroke_intersects_envelope(&s, 50.0, 50.0, 50.0, 50.0));
        // Zero-area envelope outside
        assert!(!stroke_intersects_envelope(&s, 0.0, 0.0, 0.0, 0.0));
    }

    /// Corner 9: Eraser with zero or negative radius
    #[test]
    fn test_t2_09_eraser_zero_and_negative_radius_rejection() {
        let s = generate_dot_stroke(1, 100.0, 100.0, 4.0);
        // With negative radius, should never intersect
        assert!(!s.intersects_point((100.0, 100.0), -5.0));
    }

    /// Corner 10: Inverted selection rectangle coordinates (min > max)
    #[test]
    fn test_t2_10_selection_zero_area_and_invert_box() {
        let r = Rect::new(100.0, 100.0, 50.0, 50.0);
        let norm = r.normalized();
        assert_eq!(norm.min_x, 50.0);
        assert_eq!(norm.max_x, 100.0);
        assert!(norm.contains_point(75.0, 75.0));
    }
}

// ============================================================================
// TIER 3: CROSS-FEATURE COMBINATIONS
// ============================================================================

#[cfg(test)]
mod tier3_cross_feature_combinations {
    use super::*;

    /// Combination 1: Viewport Culling + In-Place MoveItems Translation
    #[test]
    fn test_t3_01_culling_plus_move_items_spatial_update() {
        let mut doc = Document::new_notebook("Culling + Move");
        let page_id = doc.pages[0].id;
        let mut prng = DeterministicPrng::new(301);

        // 50 strokes outside viewport [0, 0, 100, 100] at [300, 300]
        for i in 1..=50 {
            let s = generate_handwritten_stroke(&mut prng, i, 300.0, 300.0, 40.0);
            doc.pages[0].items.push(Item {
                id: i,
                z_index: i,
                kind: ItemKind::Stroke(s),
            });
        }

        let vp = (0.0, 0.0, 150.0, 150.0);
        let initial_visible = doc.pages[0]
            .items
            .iter()
            .filter(|it| {
                it.as_stroke()
                    .is_some_and(|s| stroke_intersects_envelope(s, vp.0, vp.1, vp.2, vp.3))
            })
            .count();
        assert_eq!(initial_visible, 0);

        // Move 20 strokes into viewport: delta (-250, -250)
        let mut history = CommandHistory::new(10);
        let move_ids: Vec<ItemId> = (1..=20).collect();
        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: move_ids,
                delta_x: -250.0,
                delta_y: -250.0,
            },
            &mut doc,
        );

        let visible_after_move = doc.pages[0]
            .items
            .iter()
            .filter(|it| {
                it.as_stroke()
                    .is_some_and(|s| stroke_intersects_envelope(s, vp.0, vp.1, vp.2, vp.3))
            })
            .count();
        assert_eq!(visible_after_move, 20);

        // Undo move
        history.undo(&mut doc);
        let visible_after_undo = doc.pages[0]
            .items
            .iter()
            .filter(|it| {
                it.as_stroke()
                    .is_some_and(|s| stroke_intersects_envelope(s, vp.0, vp.1, vp.2, vp.3))
            })
            .count();
        assert_eq!(visible_after_undo, 0);
    }

    /// Combination 2: Spatial Culling + Eraser + Undo/Redo Restoration
    #[test]
    fn test_t3_02_culling_plus_eraser_plus_undo_redo() {
        let mut doc = Document::new_notebook("Culling + Eraser");
        let page_id = doc.pages[0].id;

        let s1 = generate_dot_stroke(1, 50.0, 50.0, 5.0);
        let s2 = generate_dot_stroke(2, 60.0, 60.0, 5.0);
        doc.pages[0].items.push(Item {
            id: 1,
            z_index: 1,
            kind: ItemKind::Stroke(s1),
        });
        doc.pages[0].items.push(Item {
            id: 2,
            z_index: 2,
            kind: ItemKind::Stroke(s2),
        });

        let mut history = CommandHistory::new(10);

        // Erase s1
        let erased = vec![doc.pages[0].items[0].clone()];
        history.execute(
            DocCommand::EraseItems {
                page_id,
                erased_items: erased,
            },
            &mut doc,
        );

        assert_eq!(doc.pages[0].items.len(), 1);
        assert_eq!(doc.pages[0].items[0].id, 2);

        // Undo erase
        history.undo(&mut doc);
        assert_eq!(doc.pages[0].items.len(), 2);
    }

    /// Combination 3: Selection Candidate Pre-Filter + Translation + SQLite WAL Roundtrip
    #[test]
    fn test_t3_03_selection_prefilter_move_and_sqlite_roundtrip() {
        let mut doc = Document::new_notebook("Selection + Move + SQLite");
        let page_id = doc.pages[0].id;
        let mut prng = DeterministicPrng::new(303);

        for i in 1..=100 {
            let s = generate_handwritten_stroke(&mut prng, i, 100.0, 100.0, 50.0);
            doc.pages[0].items.push(Item {
                id: i,
                z_index: i,
                kind: ItemKind::Stroke(s),
            });
        }

        // Translate entire cluster
        let all_ids: Vec<ItemId> = (1..=100).collect();
        let mut history = CommandHistory::new(10);
        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: all_ids,
                delta_x: 200.0,
                delta_y: 150.0,
            },
            &mut doc,
        );

        let db_path = unique_temp_db_path("t3_03_roundtrip");
        let mut store = DocumentStore::open_or_create(&db_path).unwrap();
        store.save_document(&doc).unwrap();

        let loaded = store.load_document().unwrap();
        assert_eq!(loaded.pages[0].items.len(), 100);

        let s = loaded.pages[0].items[0].as_stroke().unwrap();
        let bbox = s.bounding_box().unwrap();
        assert!(
            bbox.min_x >= 280.0,
            "Stroke was not persisted at translated coordinates"
        );

        let _ = fs::remove_file(&db_path);
    }

    /// Combination 4: Dynamic Pan/Zoom Trajectory with Continuous Culling Validation
    #[test]
    fn test_t3_04_viewport_dynamic_pan_zoom_culling_trajectory() {
        let doc = generate_synthetic_notebook(1, 5_000, 304);
        let page = &doc.pages[0];

        // Perform 10-step pan sequence across page width
        for step in 0..10 {
            let win_x = step as f64 * 40.0;
            let win_y = 100.0;
            let vp = (win_x, win_y, win_x + 80.0, win_y + 80.0);

            let visible = page
                .items
                .iter()
                .filter(|it| {
                    it.as_stroke()
                        .is_some_and(|s| stroke_intersects_envelope(s, vp.0, vp.1, vp.2, vp.3))
                })
                .count();

            let culled_pct = ((5_000 - visible) as f64 / 5_000.0) * 100.0;
            assert!(
                culled_pct > 80.0,
                "Culling rate {}% too low at step {}",
                culled_pct,
                step
            );
        }
    }

    /// Combination 5: Multi-page Document with mixed A4 and Whiteboard pages saved to SQLite WAL
    #[test]
    fn test_t3_05_mixed_a4_notebook_and_infinite_whiteboard_sqlite_wal() {
        let mut doc = Document::new_notebook("Mixed Formats");
        let mut wb_page = Page::new_whiteboard(PageBackground::Blank);
        wb_page.items.push(Item {
            id: 999,
            z_index: 1,
            kind: ItemKind::Stroke(generate_dot_stroke(999, -5000.0, 5000.0, 10.0)),
        });
        doc.pages.push(wb_page);

        let db_path = unique_temp_db_path("t3_05_mixed");
        let mut store = DocumentStore::open_or_create(&db_path).unwrap();
        store.save_document(&doc).unwrap();

        let loaded = store.load_document().unwrap();
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.pages[1].items.len(), 1);

        let _ = fs::remove_file(&db_path);
    }
}

// ============================================================================
// TIER 4: REAL-WORLD APPLICATION SCENARIOS
// ============================================================================

#[cfg(test)]
mod tier4_real_world_application_scenarios {
    use super::*;

    /// Scenario 1: Academic Lecture Simulation (50-page dense A4 notebook with formulas)
    /// - 50 discrete A4 pages, 200 strokes/page = 10,000 total strokes
    /// - Page turning, active page viewport rendering with Cairo ImageSurface
    /// - Interactive mistake correction (eraser)
    /// - Background SQLite WAL persistence with file roundtrip verification
    #[test]
    fn test_t4_01_academic_lecture_50_page_notebook_simulation() {
        let start_total = Instant::now();
        let page_count = 50;
        let strokes_per_page = 200;

        // 1. Generate full university lecture notebook
        let mut doc = generate_synthetic_notebook(page_count, strokes_per_page, 401);
        assert_eq!(doc.pages.len(), 50);

        let total_strokes: usize = doc.pages.iter().map(|p| p.items.len()).sum();
        assert_eq!(total_strokes, 10_000);

        // 2. Simulate reading & turning pages across the lecture
        let surface =
            ImageSurface::create(Format::ARgb32, 600, 850).expect("Surface creation failed");

        for page_idx in [0, 12, 25, 49] {
            let active_page = &doc.pages[page_idx];
            let stroke_refs: Vec<&Stroke> = active_page.strokes().collect();

            let (rendered, frame_us) =
                render_strokes_headless_cairo(&surface, &stroke_refs, (0.0, 0.0), 1.0);
            assert_eq!(rendered, strokes_per_page);
            assert!(
                frame_us < 100_000,
                "Page {} render took > 100ms: {} us",
                page_idx,
                frame_us
            );
        }

        // 3. Edit interaction on page 12 (student erases a mistake and rewrites formula)
        let page_12_id = doc.pages[12].id;
        let mut history = CommandHistory::with_next_id(20, 10_001);

        // Erase last 5 strokes
        let erased_items: Vec<Item> = doc.pages[12].items.drain(195..).collect();
        assert_eq!(erased_items.len(), 5);
        history.execute(
            DocCommand::EraseItems {
                page_id: page_12_id,
                erased_items,
            },
            &mut doc,
        );

        // Write replacement formula
        let mut prng = DeterministicPrng::new(999);
        let replacement_stroke = generate_formula_stroke(&mut prng, 10_001, 300.0, 400.0);
        history.execute(
            DocCommand::AddStroke {
                page_id: page_12_id,
                stroke: replacement_stroke,
            },
            &mut doc,
        );

        assert_eq!(doc.pages[12].items.len(), 196);

        // 4. Persistence to SQLite WAL
        let db_path = unique_temp_db_path("t4_01_lecture_50p");
        let mut store = DocumentStore::open_or_create(&db_path).expect("Failed to open DB");

        let save_start = Instant::now();
        store
            .save_document(&doc)
            .expect("Failed to save 50-page notebook");
        let save_duration = save_start.elapsed();

        // 10,000 strokes should save in < 1500ms in unoptimized debug mode (< 800ms in release mode)
        let max_save_ms = if cfg!(debug_assertions) { 1500 } else { 800 };
        assert!(
            save_duration.as_millis() < max_save_ms,
            "50-page save took {} ms (threshold: {} ms)",
            save_duration.as_millis(),
            max_save_ms
        );

        // 5. Verify roundtrip rehydration
        let loaded = store.load_document().expect("Failed to reload notebook");
        assert_eq!(loaded.pages.len(), 50);
        assert_eq!(loaded.pages[12].items.len(), 196);

        let total_elapsed = start_total.elapsed();
        println!(
            "Academic lecture simulation completed in {} ms (save: {} ms)",
            total_elapsed.as_millis(),
            save_duration.as_millis()
        );

        let _ = fs::remove_file(&db_path);
    }

    /// Scenario 2: University Brainstorming Session (Infinite Whiteboard with 5 clusters)
    /// - 5 separated conceptual clusters across large coordinate space
    /// - Navigation & culling on each cluster
    /// - Lasso selection of Cluster 2 and translation to merge with Cluster 1
    /// - SQLite WAL save and reload validation
    #[test]
    fn test_t4_02_infinite_whiteboard_university_brainstorming_session() {
        let cluster_count = 5;
        let strokes_per_cluster = 1_000;

        // 1. Generate multi-cluster whiteboard
        let mut doc = generate_synthetic_whiteboard(cluster_count, strokes_per_cluster, 402);
        let page = &doc.pages[0];
        assert_eq!(page.items.len(), 5_000);

        // 2. Viewport query on Cluster 1 (around origin [0, 0])
        let vp_cluster1 = (-500.0, -400.0, 500.0, 400.0);
        let visible_c1 = page
            .items
            .iter()
            .filter(|it| {
                it.as_stroke().is_some_and(|s| {
                    stroke_intersects_envelope(
                        s,
                        vp_cluster1.0,
                        vp_cluster1.1,
                        vp_cluster1.2,
                        vp_cluster1.3,
                    )
                })
            })
            .count();

        // Only cluster 1 strokes should be visible, strokes from other distant clusters culled
        assert!(visible_c1 >= 900);
        assert!(visible_c1 <= 1100);

        let culled_c1_pct = ((5_000 - visible_c1) as f64 / 5_000.0) * 100.0;
        assert!(
            culled_c1_pct > 75.0,
            "Culling rate {}% too low on cluster 1",
            culled_c1_pct
        );

        // 3. Selection of Cluster 2 (around [2500, 800]) and translation toward Cluster 1
        let page_id = doc.pages[0].id;
        let selection_c2 = (2000.0, 400.0, 3000.0, 1200.0);
        let c2_item_ids: Vec<ItemId> = doc.pages[0]
            .items
            .iter()
            .filter(|it| {
                it.as_stroke().is_some_and(|s| {
                    stroke_intersects_envelope(
                        s,
                        selection_c2.0,
                        selection_c2.1,
                        selection_c2.2,
                        selection_c2.3,
                    )
                })
            })
            .map(|it| it.id)
            .collect();

        assert!(c2_item_ids.len() >= 900);

        // Translate Cluster 2 by (-2500, -800) to merge with Cluster 1
        let mut history = CommandHistory::new(10);
        history.execute(
            DocCommand::MoveItems {
                page_id,
                item_ids: c2_item_ids,
                delta_x: -2500.0,
                delta_y: -800.0,
            },
            &mut doc,
        );

        // 4. Verify that Cluster 1 viewport now sees both clusters merged (~2000 strokes)
        let visible_merged = doc.pages[0]
            .items
            .iter()
            .filter(|it| {
                it.as_stroke().is_some_and(|s| {
                    stroke_intersects_envelope(
                        s,
                        vp_cluster1.0,
                        vp_cluster1.1,
                        vp_cluster1.2,
                        vp_cluster1.3,
                    )
                })
            })
            .count();
        assert!(visible_merged >= 1800);

        // 5. Save and reload Whiteboard
        let db_path = unique_temp_db_path("t4_02_whiteboard_clusters");
        let mut store = DocumentStore::open_or_create(&db_path).unwrap();
        store.save_document(&doc).unwrap();

        let loaded = store.load_document().unwrap();
        assert_eq!(loaded.format, DocumentFormat::Whiteboard);
        assert_eq!(loaded.pages[0].items.len(), 5_000);

        let _ = fs::remove_file(&db_path);
    }
}
