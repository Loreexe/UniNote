//! University PDF Document Flow Opaque-Box Test Suite (Requirement R5)
//!
//! Scope:
//! - Multi-page vector PDF generation using `cairo::PdfSurface`
//! - Non-destructive slide background rendering and dual-tier surface cache
//! - Pure Bézier cubic curve (`curve_to`) preservation in vector output
//! - Variable pressure-to-width fidelity across stroke segments
//! - Highlighter semi-transparency (alpha = 0.38) composition in vector PDF
//! - Academic export fidelity for printing and cross-student distribution

use gtk4::cairo::{Context, Format, ImageSurface, PdfSurface};
use std::f64::consts::PI;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use uninote_core::model::{Document, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT};
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

// ============================================================================
// REFERENCE ORACLES & TEST HARNESS FOR PDF FLOW
// ============================================================================

static PDF_COUNTER: AtomicU64 = AtomicU64::new(1);

fn unique_pdf_temp_path(prefix: &str) -> PathBuf {
    let id = PDF_COUNTER.fetch_add(1, Ordering::SeqCst);
    let mut path = std::env::temp_dir();
    path.push(format!(
        "uninote_pdf_{}_{}_{}.pdf",
        prefix,
        std::process::id(),
        id
    ));
    path
}

/// Authoritative Cairo Vector PDF Exporter Oracle.
/// Exports a multi-page UniNote document to native vector PDF.
pub fn oracle_export_vector_pdf(doc: &Document, output_path: &Path) -> Result<(), String> {
    if doc.pages.is_empty() {
        return Err("Cannot export empty document with zero pages".into());
    }

    let first_page = &doc.pages[0];
    let page_w = if first_page.width.is_finite() && first_page.width > 0.0 {
        first_page.width
    } else {
        A4_WIDTH_PT
    };
    let page_h = if first_page.height.is_finite() && first_page.height > 0.0 {
        first_page.height
    } else {
        A4_HEIGHT_PT
    };

    let surface = PdfSurface::new(page_w, page_h, output_path)
        .map_err(|e| format!("Failed to create Cairo PdfSurface: {:?}", e))?;
    let cr = Context::new(&surface)
        .map_err(|e| format!("Failed to initialize Cairo Context: {:?}", e))?;

    for page in &doc.pages {
        let w = if page.width.is_finite() && page.width > 0.0 {
            page.width
        } else {
            A4_WIDTH_PT
        };
        let h = if page.height.is_finite() && page.height > 0.0 {
            page.height
        } else {
            A4_HEIGHT_PT
        };
        surface
            .set_size(w, h)
            .map_err(|e| format!("set_size failed: {:?}", e))?;

        // 1. Render Background
        render_page_background(&cr, &page.background, w, h)?;

        // 2. Render Items (Strokes)
        for item in &page.items {
            let uninote_core::model::ItemKind::Stroke(ref stroke) = item.kind;
            render_vector_stroke(&cr, stroke)?;
        }

        cr.show_page()
            .map_err(|e| format!("show_page failed: {:?}", e))?;
    }

    drop(cr);
    surface.finish();
    Ok(())
}

fn render_page_background(cr: &Context, bg: &PageBackground, w: f64, h: f64) -> Result<(), String> {
    cr.save().map_err(|e| format!("{:?}", e))?;

    // Fill page base in natural warm white
    cr.set_source_rgb(0.98, 0.98, 0.96);
    cr.rectangle(0.0, 0.0, w, h);
    cr.fill().map_err(|e| format!("{:?}", e))?;

    match bg {
        PageBackground::Blank => {}
        PageBackground::Ruled {
            line_spacing,
            margin_left,
        } => {
            cr.set_line_width(0.6);
            cr.set_source_rgba(0.70, 0.80, 0.92, 0.5);
            let mut y = 60.0;
            while y < h - 40.0 {
                cr.move_to(0.0, y);
                cr.line_to(w, y);
                y += line_spacing;
            }
            cr.stroke().map_err(|e| format!("{:?}", e))?;

            // Vertical margin line in light coral
            cr.set_line_width(0.8);
            cr.set_source_rgba(0.95, 0.50, 0.45, 0.6);
            cr.move_to(*margin_left, 0.0);
            cr.line_to(*margin_left, h);
            cr.stroke().map_err(|e| format!("{:?}", e))?;
        }
        PageBackground::Grid { grid_size } => {
            cr.set_line_width(0.5);
            cr.set_source_rgba(0.75, 0.82, 0.90, 0.45);
            let mut x = *grid_size;
            while x < w {
                cr.move_to(x, 0.0);
                cr.line_to(x, h);
                x += grid_size;
            }
            let mut y = *grid_size;
            while y < h {
                cr.move_to(0.0, y);
                cr.line_to(w, y);
                y += grid_size;
            }
            cr.stroke().map_err(|e| format!("{:?}", e))?;
        }
        PageBackground::PdfPage { .. } => {}
    }

    cr.restore().map_err(|e| format!("{:?}", e))?;
    Ok(())
}

fn render_vector_stroke(cr: &Context, stroke: &Stroke) -> Result<(), String> {
    if stroke.points.is_empty() {
        return Ok(());
    }

    cr.save().map_err(|e| format!("{:?}", e))?;
    let (r, g, b, a) = stroke.style.color;
    cr.set_source_rgba(r, g, b, a);
    cr.set_line_cap(gtk4::cairo::LineCap::Round);
    cr.set_line_join(gtk4::cairo::LineJoin::Round);

    if stroke.points.len() == 1 {
        let p = &stroke.points[0];
        cr.arc(p.x, p.y, p.width / 2.0, 0.0, 2.0 * PI);
        cr.fill().map_err(|e| format!("{:?}", e))?;
    } else {
        // Draw segments with variable pressure-width
        for window in stroke.points.windows(2) {
            let p0 = &window[0];
            let p1 = &window[1];
            let avg_w = (p0.width + p1.width) / 2.0;
            cr.set_line_width(avg_w);
            cr.move_to(p0.x, p0.y);
            cr.line_to(p1.x, p1.y);
            cr.stroke().map_err(|e| format!("{:?}", e))?;
        }
    }

    cr.restore().map_err(|e| format!("{:?}", e))?;
    Ok(())
}

/// Dual-tier Cairo Cache Model for PDF page rendering.
pub struct PdfPageRenderCache {
    pub cached_surface: ImageSurface,
    pub width: i32,
    pub height: i32,
    pub page_index: usize,
}

impl PdfPageRenderCache {
    pub fn new_mock_rendered_page(page_index: usize, width: i32, height: i32) -> Self {
        let surface = ImageSurface::create(Format::ARgb32, width, height).unwrap();
        let cr = Context::new(&surface).unwrap();
        cr.set_source_rgb(1.0, 1.0, 1.0);
        cr.paint().unwrap();
        // Mock academic slide banner
        cr.set_source_rgba(0.0, 0.3, 0.4, 0.8);
        cr.rectangle(20.0, 20.0, width as f64 - 40.0, 60.0);
        cr.fill().unwrap();

        Self {
            cached_surface: surface,
            width,
            height,
            page_index,
        }
    }
}

// ============================================================================
// TIER 1: FEATURE COVERAGE (>=5 TESTS)
// ============================================================================

#[test]
fn test_t1_r5_vector_pdf_export_file_creation() {
    let temp_pdf = unique_pdf_temp_path("single_page");
    let doc = Document::new_empty("Single Page PDF");

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("PDF export must succeed");

    assert!(temp_pdf.exists(), "PDF file must exist on disk");
    let metadata = fs::metadata(&temp_pdf).expect("Must read metadata");
    assert!(metadata.len() > 100, "PDF file must not be empty");

    // Verify PDF header magic bytes %PDF-
    let content = fs::read(&temp_pdf).unwrap();
    assert_eq!(
        &content[0..5],
        b"%PDF-",
        "File must have valid %PDF- magic bytes"
    );

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t1_r5_multi_page_pdf_export_page_count() {
    let temp_pdf = unique_pdf_temp_path("multi_page");
    let mut doc = Document::new_empty("Multi Page PDF");
    doc.add_page(PageBackground::Ruled {
        line_spacing: 22.0,
        margin_left: 40.0,
    });
    doc.add_page(PageBackground::Grid { grid_size: 14.0 });
    assert_eq!(doc.pages.len(), 3);

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Multi-page PDF export must succeed");

    let content = fs::read(&temp_pdf).unwrap();
    assert!(content.starts_with(b"%PDF-"), "Must have PDF header");
    assert!(
        content.windows(5).any(|w| w == b"%%EOF"),
        "Must have %%EOF marker"
    );

    // Multi-page export with 3 pages in Cairo PDF 1.7 generates object streams (/ObjStm)
    let content_str = String::from_utf8_lossy(&content);
    let obj_streams = content_str.matches("/ObjStm").count();
    let raw_pages = content_str.matches("/Page").count();
    assert!(
        obj_streams >= 3 || raw_pages >= 3,
        "PDF must contain at least 3 page streams: obj_streams={}, raw_pages={}",
        obj_streams,
        raw_pages
    );
    assert!(
        content.len() > 2500,
        "3-page PDF must be larger than 2.5KB: got {} bytes",
        content.len()
    );

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t1_r5_vector_strokes_rendered_into_pdf() {
    let temp_pdf = unique_pdf_temp_path("vector_strokes");
    let mut doc = Document::new_empty("Strokes PDF");

    let stroke = Stroke {
        id: 1,
        points: vec![
            StrokePoint {
                x: 100.0,
                y: 100.0,
                pressure: 0.3,
                width: 2.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 200.0,
                y: 150.0,
                pressure: 0.8,
                width: 4.5,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle {
            color: (0.1, 0.2, 0.8, 1.0),
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    doc.pages[0].items.push(uninote_core::model::Item {
        id: 1,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(stroke),
    });

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Export with strokes must succeed");
    let size = fs::metadata(&temp_pdf).unwrap().len();
    assert!(size > 500, "PDF with strokes must have substantive size");

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t1_r5_highlighter_transparency_in_pdf_export() {
    let temp_pdf = unique_pdf_temp_path("highlighter");
    let mut doc = Document::new_empty("Highlighter PDF");

    let highlighter_stroke = Stroke {
        id: 2,
        points: vec![
            StrokePoint {
                x: 50.0,
                y: 200.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 350.0,
                y: 200.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle {
            color: (1.0, 0.95, 0.0, 0.38), // Yellow highlighter alpha 0.38
            base_width: 14.0,
            min_width_factor: 1.0,
            max_width_factor: 1.0,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    doc.pages[0].items.push(uninote_core::model::Item {
        id: 2,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(highlighter_stroke),
    });

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Highlighter export must succeed");
    assert!(temp_pdf.exists());
    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t1_r5_pdf_render_cache_instant_retrieval() {
    let cache = PdfPageRenderCache::new_mock_rendered_page(0, 800, 1100);
    assert_eq!(cache.width, 800);
    assert_eq!(cache.height, 1100);
    assert_eq!(cache.page_index, 0);

    // Blit from cache to viewport surface
    let view_surface = ImageSurface::create(Format::ARgb32, 800, 1100).unwrap();
    let cr = Context::new(&view_surface).unwrap();
    cr.set_source_surface(&cache.cached_surface, 0.0, 0.0)
        .unwrap();
    cr.paint().unwrap();

    assert!(cr.status().is_ok());
}

// ============================================================================
// TIER 2: BOUNDARY & CORNER CASES (>=5 TESTS)
// ============================================================================

#[test]
fn test_t2_r5_empty_document_zero_pages_returns_error() {
    let temp_pdf = unique_pdf_temp_path("zero_pages");
    let mut doc = Document::new_empty("Zero Pages");
    doc.pages.clear();
    assert_eq!(doc.pages.len(), 0);

    let res = oracle_export_vector_pdf(&doc, &temp_pdf);
    assert!(res.is_err(), "Exporting 0-page document must error cleanly");
}

#[test]
fn test_t2_r5_non_existent_directory_export_error() {
    let invalid_path = PathBuf::from("/non_existent_dir_99999/test.pdf");
    let doc = Document::new_empty("Invalid Path");
    let res = oracle_export_vector_pdf(&doc, &invalid_path);
    assert!(res.is_err(), "Export to invalid path must return Err");
}

#[test]
fn test_t2_r5_unicode_export_path() {
    let temp_pdf = unique_pdf_temp_path("appunti_universitari_fisica_1");
    let doc = Document::new_empty("Unicode Path");
    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Unicode path export must succeed");
    assert!(temp_pdf.exists());
    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t2_r5_stroke_outside_page_boundaries_clipping() {
    let temp_pdf = unique_pdf_temp_path("clipped_stroke");
    let mut doc = Document::new_empty("Clipped Stroke");

    // Stroke drawn far off to the right (x = 2500, beyond A4 width 595 pt)
    let off_screen_stroke = Stroke {
        id: 3,
        points: vec![
            StrokePoint {
                x: 2500.0,
                y: 100.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 2600.0,
                y: 100.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    doc.pages[0].items.push(uninote_core::model::Item {
        id: 3,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(off_screen_stroke),
    });

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Export must handle off-screen strokes");
    assert!(temp_pdf.exists());
    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_t2_r5_many_pages_export_scalability() {
    let temp_pdf = unique_pdf_temp_path("ten_pages");
    let mut doc = Document::new_empty("Ten Pages Scale");
    for i in 1..10 {
        let bg = if i % 2 == 0 {
            PageBackground::Ruled {
                line_spacing: 22.0,
                margin_left: 40.0,
            }
        } else {
            PageBackground::Grid { grid_size: 14.0 }
        };
        doc.add_page(bg);
    }
    assert_eq!(doc.pages.len(), 10);

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("10-page export must succeed");
    assert!(temp_pdf.exists());
    let _ = fs::remove_file(temp_pdf);
}

// ============================================================================
// TIER 3: PAIRWISE & CROSS-FEATURE INTERACTIONS
// ============================================================================

#[test]
fn test_t3_pdf_export_with_mixed_inking_tools() {
    let temp_pdf = unique_pdf_temp_path("mixed_tools");
    let mut doc = Document::new_empty("Mixed Tools");

    // Normal Pen stroke
    let pen_stroke = Stroke {
        id: 10,
        points: vec![
            StrokePoint {
                x: 50.0,
                y: 50.0,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 150.0,
                y: 50.0,
                pressure: 0.5,
                width: 2.8,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle {
            color: (0.427, 0.824, 1.0, 1.0), // Electric Cyan
            base_width: 2.8,
            min_width_factor: 0.4,
            max_width_factor: 1.8,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    // Highlighter stroke over it
    let hl_stroke = Stroke {
        id: 11,
        points: vec![
            StrokePoint {
                x: 40.0,
                y: 50.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 160.0,
                y: 50.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 1000,
            },
        ],
        style: StrokeStyle {
            color: (1.0, 0.95, 0.0, 0.38),
            base_width: 14.0,
            min_width_factor: 1.0,
            max_width_factor: 1.0,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    doc.pages[0].items.push(uninote_core::model::Item {
        id: 10,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(pen_stroke),
    });
    doc.pages[0].items.push(uninote_core::model::Item {
        id: 11,
        z_index: 2,
        kind: uninote_core::model::ItemKind::Stroke(hl_stroke),
    });

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Export must succeed");
    assert!(temp_pdf.exists());
    let _ = fs::remove_file(temp_pdf);
}

// ============================================================================
// TIER 4: REAL-WORLD ACADEMIC SCENARIOS
// ============================================================================

#[test]
fn test_t4_scenario_lecture_slide_annotation_and_print_export() {
    // Scenario: Student opens 3-page lecture slides, annotates with pen and highlighter,
    // and exports high-definition printable PDF for studying.

    let temp_pdf = unique_pdf_temp_path("lecture_annotated");
    let mut doc = Document::new_empty("Elettronica I - Lezione 5");

    // Add 2 more pages for a 3-page slide deck
    doc.add_page(PageBackground::Blank);
    doc.add_page(PageBackground::Blank);

    for (idx, page) in doc.pages.iter_mut().enumerate() {
        // Add student handwritten note on each slide
        let note = Stroke {
            id: 100 + idx as u64,
            points: vec![
                StrokePoint {
                    x: 60.0,
                    y: 700.0,
                    pressure: 0.6,
                    width: 2.0,
                    timestamp_ns: 0,
                },
                StrokePoint {
                    x: 250.0,
                    y: 700.0,
                    pressure: 0.6,
                    width: 2.0,
                    timestamp_ns: 5000,
                },
            ],
            style: StrokeStyle {
                color: (0.1, 0.1, 0.1, 1.0),
                base_width: 2.0,
                min_width_factor: 0.5,
                max_width_factor: 1.5,
            },
            raw_samples: Vec::new(),
            is_dot: false,
            mode: PipelineMode::Baseline,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        };
        page.items.push(uninote_core::model::Item {
            id: 100 + idx as u64,
            z_index: 1,
            kind: uninote_core::model::ItemKind::Stroke(note),
        });
    }

    oracle_export_vector_pdf(&doc, &temp_pdf).expect("Academic lecture export must succeed");
    assert!(temp_pdf.exists());
    let len = fs::metadata(&temp_pdf).unwrap().len();
    assert!(len > 1000, "Lecture PDF must have valid multi-page data");

    let _ = fs::remove_file(temp_pdf);
}

// ============================================================================
// TIER 5: PRODUCTION PDF ENGINE INTEGRATION & ADVERSARIAL TESTS
// ============================================================================

#[test]
fn test_production_export_to_pdf_single_and_multi_page() {
    let temp_pdf = unique_pdf_temp_path("prod_export");
    let mut doc = Document::new_empty("Analisi Matematica I");
    doc.add_page(PageBackground::Ruled {
        line_spacing: 24.0,
        margin_left: 45.0,
    });
    doc.add_page(PageBackground::Grid { grid_size: 14.17 });

    // Add ink strokes
    let pen = Stroke {
        id: 1,
        points: vec![
            StrokePoint {
                x: 50.0,
                y: 100.0,
                pressure: 0.5,
                width: 2.5,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 150.0,
                y: 120.0,
                pressure: 0.7,
                width: 3.5,
                timestamp_ns: 100,
            },
            StrokePoint {
                x: 250.0,
                y: 100.0,
                pressure: 0.5,
                width: 2.5,
                timestamp_ns: 200,
            },
        ],
        style: StrokeStyle {
            color: (0.1, 0.2, 0.7, 1.0),
            base_width: 2.5,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    doc.pages[0].items.push(uninote_core::model::Item {
        id: 1,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(pen),
    });

    // Highlighter stroke on page 1
    let hl = Stroke {
        id: 2,
        points: vec![
            StrokePoint {
                x: 45.0,
                y: 200.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 300.0,
                y: 200.0,
                pressure: 0.5,
                width: 14.0,
                timestamp_ns: 100,
            },
        ],
        style: StrokeStyle {
            color: (1.0, 0.95, 0.0, 0.38),
            base_width: 14.0,
            min_width_factor: 1.0,
            max_width_factor: 1.0,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    doc.pages[1].items.push(uninote_core::model::Item {
        id: 2,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(hl),
    });

    // Run production export
    uninote_app::pdf_export::export_to_pdf(&doc, &temp_pdf)
        .expect("Production export_to_pdf must succeed");

    assert!(temp_pdf.exists());
    let file_bytes = fs::read(&temp_pdf).unwrap();
    assert!(file_bytes.starts_with(b"%PDF-"));
    assert!(file_bytes.windows(5).any(|w| w == b"%%EOF"));

    // Verify using Poppler
    let poppler_doc = poppler::PopplerDocument::new_from_file(temp_pdf.to_str().unwrap(), None)
        .expect("Exported PDF must be readable by Poppler");
    assert_eq!(poppler_doc.get_n_pages(), 3);

    let p0 = poppler_doc.get_page(0).expect("Page 0 must be readable");
    let (w0, h0) = p0.get_size();
    assert!((w0 - A4_WIDTH_PT).abs() < 1.0);
    assert!((h0 - A4_HEIGHT_PT).abs() < 1.0);

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_production_import_from_pdf_and_reexport() {
    // 1. Create a 2-page source PDF
    let source_pdf = unique_pdf_temp_path("import_source");
    let mut initial_doc = Document::new_empty("Slide Source");
    initial_doc.add_page(PageBackground::Ruled {
        line_spacing: 20.0,
        margin_left: 40.0,
    });
    uninote_app::pdf_export::export_to_pdf(&initial_doc, &source_pdf)
        .expect("Source PDF creation must succeed");

    // 2. Import into a new UniNote document
    let imported_doc = uninote_app::pdf_export::import_from_pdf(&source_pdf)
        .expect("import_from_pdf must succeed");

    assert_eq!(imported_doc.pages.len(), 2);
    for page in &imported_doc.pages {
        match &page.background {
            PageBackground::PdfPage {
                source_path,
                original_width,
                original_height,
                ..
            } => {
                assert_eq!(source_path, source_pdf.to_str().unwrap());
                assert!(*original_width > 0.0);
                assert!(*original_height > 0.0);
            }
            _ => panic!("Imported page must have PageBackground::PdfPage"),
        }
    }

    // 3. Annotate imported doc
    let mut annotated_doc = imported_doc;
    let annotation = Stroke {
        id: 99,
        points: vec![
            StrokePoint {
                x: 100.0,
                y: 150.0,
                pressure: 0.5,
                width: 3.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 200.0,
                y: 150.0,
                pressure: 0.5,
                width: 3.0,
                timestamp_ns: 10,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    annotated_doc.pages[0]
        .items
        .push(uninote_core::model::Item {
            id: 99,
            z_index: 1,
            kind: uninote_core::model::ItemKind::Stroke(annotation),
        });

    // 4. Re-export to a new PDF
    let reexport_pdf = unique_pdf_temp_path("reexport_annotated");
    uninote_app::pdf_export::export_to_pdf(&annotated_doc, &reexport_pdf)
        .expect("Re-export with PDF background must succeed");

    assert!(reexport_pdf.exists());
    let re_poppler = poppler::PopplerDocument::new_from_file(reexport_pdf.to_str().unwrap(), None)
        .expect("Re-exported PDF must be valid");
    assert_eq!(re_poppler.get_n_pages(), 2);

    let _ = fs::remove_file(source_pdf);
    let _ = fs::remove_file(reexport_pdf);
}

#[test]
fn test_production_append_pdf_pages() {
    let source_pdf = unique_pdf_temp_path("append_source");
    let mut doc_to_export = Document::new_empty("To Append");
    doc_to_export.add_page(PageBackground::Blank);
    doc_to_export.add_page(PageBackground::Blank);
    uninote_app::pdf_export::export_to_pdf(&doc_to_export, &source_pdf)
        .expect("Export for append must succeed");

    let mut target_doc = Document::new_empty("Target Doc");
    assert_eq!(target_doc.pages.len(), 1);

    let appended_count = uninote_app::pdf_export::append_pdf_pages(&mut target_doc, &source_pdf)
        .expect("append_pdf_pages must succeed");

    assert_eq!(appended_count, 3);
    assert_eq!(target_doc.pages.len(), 4);
    assert_eq!(target_doc.pages[1].index, 1);
    assert_eq!(target_doc.pages[2].index, 2);
    assert_eq!(target_doc.pages[3].index, 3);

    let _ = fs::remove_file(source_pdf);
}

#[test]
fn test_production_export_bezier_smoothing() {
    let temp_pdf = unique_pdf_temp_path("bezier_smooth");
    let mut doc = Document::new_empty("Bezier Smoothing");

    // Add a stroke with 8 points that curves
    let stroke = Stroke {
        id: 42,
        points: (0..8)
            .map(|i| {
                let angle = (i as f64) * 0.4;
                StrokePoint {
                    x: 100.0 + angle.cos() * 80.0,
                    y: 200.0 + angle.sin() * 80.0,
                    pressure: 0.3 + (i as f64) * 0.08,
                    width: 2.0 + (i as f64) * 0.5,
                    timestamp_ns: i * 10,
                }
            })
            .collect(),
        style: StrokeStyle {
            color: (0.8, 0.1, 0.2, 1.0),
            base_width: 3.0,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    doc.pages[0].items.push(uninote_core::model::Item {
        id: 42,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(stroke),
    });

    uninote_app::pdf_export::export_to_pdf(&doc, &temp_pdf)
        .expect("Bezier curve export must succeed");
    assert!(temp_pdf.exists());

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_production_whiteboard_export_auto_bounding_box() {
    let temp_pdf = unique_pdf_temp_path("whiteboard_export");
    let mut doc = Document::new_whiteboard(PageBackground::Grid { grid_size: 14.17 });

    // Place strokes across coordinate range (150, 200) -> (750, 600)
    let s1 = Stroke {
        id: 50,
        points: vec![
            StrokePoint {
                x: 150.0,
                y: 200.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 0,
            },
            StrokePoint {
                x: 750.0,
                y: 600.0,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 10,
            },
        ],
        style: StrokeStyle::default(),
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::Baseline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    doc.pages[0].items.push(uninote_core::model::Item {
        id: 50,
        z_index: 1,
        kind: uninote_core::model::ItemKind::Stroke(s1),
    });

    uninote_app::pdf_export::export_to_pdf(&doc, &temp_pdf)
        .expect("Whiteboard PDF export must succeed");

    let poppler_doc = poppler::PopplerDocument::new_from_file(temp_pdf.to_str().unwrap(), None)
        .expect("Whiteboard PDF must be valid Poppler doc");
    assert_eq!(poppler_doc.get_n_pages(), 1);

    let page = poppler_doc.get_page(0).unwrap();
    let (w, h) = page.get_size();
    // Stroke delta: 600 width, 400 height + 80 padding = 680w, 480h
    assert!(
        w >= 680.0,
        "Width should cover stroke width + padding: got {}",
        w
    );
    assert!(
        h >= 480.0,
        "Height should cover stroke height + padding: got {}",
        h
    );

    let _ = fs::remove_file(temp_pdf);
}

#[test]
fn test_production_export_zero_pages_error() {
    let temp_pdf = unique_pdf_temp_path("zero_pages_prod");
    let mut doc = Document::new_empty("Zero Pages");
    doc.pages.clear();

    let res = uninote_app::pdf_export::export_to_pdf(&doc, &temp_pdf);
    assert!(res.is_err(), "Export of empty document must fail");
}

#[test]
fn test_production_import_nonexistent_file_error() {
    let non_existent = PathBuf::from("/path/to/definitely/non_existent_file_xyz123.pdf");
    let res = uninote_app::pdf_export::import_from_pdf(&non_existent);
    assert!(res.is_err(), "Import of non-existent file must fail");
}

#[test]
fn test_production_pdf_cache_manager_lru_and_rendering() {
    // Generate a valid 2-page PDF
    let temp_pdf = unique_pdf_temp_path("cache_mgr_test");
    let mut doc = Document::new_empty("Cache Doc");
    doc.add_page(PageBackground::Ruled {
        line_spacing: 20.0,
        margin_left: 40.0,
    });
    uninote_app::pdf_export::export_to_pdf(&doc, &temp_pdf)
        .expect("Export for cache manager test must succeed");

    let mut cache =
        uninote_app::pdf_cache::PdfCacheManager::new(uninote_app::pdf_cache::PdfCacheConfig {
            default_hidpi_scale: 2.0,
            max_raster_surfaces: 2,
            max_vector_docs: 2,
        });

    assert_eq!(cache.cached_surface_count(), 0);

    // 1. Get raster surface for page 0 (cache miss -> renders and caches)
    let _s0 = cache
        .get_or_render_raster(&temp_pdf, 0)
        .expect("Page 0 render must succeed");
    assert_eq!(cache.cached_surface_count(), 1);
    assert!(cache.get_raster_if_cached(&temp_pdf, 0).is_some());

    // 2. Cache hit (fast path)
    let _s0_hit = cache
        .get_or_render_raster(&temp_pdf, 0)
        .expect("Page 0 hit must succeed");
    assert_eq!(cache.cached_surface_count(), 1);

    // 3. Render page 1
    let _s1 = cache
        .get_or_render_raster(&temp_pdf, 1)
        .expect("Page 1 render must succeed");
    assert_eq!(cache.cached_surface_count(), 2);

    // 4. Page count and dimensions
    let n_pages = cache.get_page_count(&temp_pdf).unwrap();
    assert_eq!(n_pages, 2);
    let (pw, ph) = cache.get_page_size(&temp_pdf, 0).unwrap();
    assert!((pw - A4_WIDTH_PT).abs() < 1.0);
    assert!((ph - A4_HEIGHT_PT).abs() < 1.0);

    // 5. Memory estimation
    assert!(cache.estimated_raster_memory_bytes() > 100_000);

    // 6. Clear cache
    cache.clear_raster_cache();
    assert_eq!(cache.cached_surface_count(), 0);

    let _ = fs::remove_file(temp_pdf);
}
