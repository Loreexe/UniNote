//! Test suite per la Milestone M4 (Sotto-Fase 4D):
//! Rendering Cairo 120 Hz, Miniature Dashboard 240x340 px ed Esportazione PDF vettoriale.

use std::fs;
use std::sync::mpsc;
use std::sync::OnceLock;
use std::thread;
use uuid::Uuid;

use gtk4::glib;
use uninote_app::dashboard::render_document_thumbnail;
use uninote_app::image_cache::get_global_image_cache;
use uninote_app::pdf_export::{export_to_pdf, export_to_pdf_with_store, PdfExportOptions};
use uninote_core::model::{
    Document, ImageItem, Item, ItemKind, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT,
};
use uninote_storage::DocumentStore;
use uninote_stroke::types::Rect;

// ============================================================================
// DEDICATED GTK TEST HARNESS
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
            .name("gtk-m4-test-worker".into())
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
        .expect("Failed to send task to GTK worker");

    match res_rx.recv().expect("Failed to receive test result") {
        Ok(val) => val,
        Err(err) => std::panic::resume_unwind(err),
    }
}

fn create_test_png(w: i32, h: i32, r: f64, g: f64, b: f64) -> (Vec<u8>, String) {
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, w, h).expect("surface");
    let cr = cairo::Context::new(&surface).expect("cr");
    cr.set_source_rgb(r, g, b);
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    cr.fill().expect("fill");
    drop(cr);

    let mut buf = Vec::new();
    surface.write_to_png(&mut buf).expect("write png");
    let hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, &buf)
        .unwrap()
        .to_string();
    (buf, hash)
}

// ============================================================================
// TESTS
// ============================================================================

#[test]
fn test_m4_thumbnail_notebook_with_image() {
    run_on_gtk_thread(|| {
        let (png_bytes, hash) = create_test_png(60, 40, 0.1, 0.7, 0.9);
        {
            let cache_rc = get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            let _ = cache.insert_from_bytes(&hash, &png_bytes);
        }

        let mut doc = Document::new_notebook("Thumbnail Notebook Test");
        let page_id = doc.pages[0].id;
        let img = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            hash.clone(),
            Rect::new(50.0, 50.0, 200.0, 150.0),
            1,
            1.0,
            60,
            40,
        );
        doc.pages[0].items.push(Item {
            id: 1,
            z_index: 1,
            kind: ItemKind::Image(img),
        });

        let out_dir = std::env::temp_dir().join(format!("uninote_test_thumb_{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&out_dir);
        let out_png = out_dir.join("thumb.png");

        let res = render_document_thumbnail(&doc, &out_png);
        assert!(res.is_ok(), "Rendering thumbnail must succeed: {:?}", res);
        assert!(out_png.exists(), "Thumbnail file must be generated");

        let meta = fs::metadata(&out_png).expect("meta");
        assert!(meta.len() > 500, "Thumbnail file size must be non-trivial");

        let mut f = fs::File::open(&out_png).expect("open");
        let surf = cairo::ImageSurface::create_from_png(&mut f).expect("valid png");
        assert_eq!(surf.width(), 240);
        assert_eq!(surf.height(), 340);

        let _ = fs::remove_dir_all(out_dir);
    });
}

#[test]
fn test_m4_thumbnail_whiteboard_with_image() {
    run_on_gtk_thread(|| {
        let (png_bytes, hash) = create_test_png(80, 80, 0.9, 0.2, 0.4);
        {
            let cache_rc = get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            let _ = cache.insert_from_bytes(&hash, &png_bytes);
        }

        let mut doc = Document::new_whiteboard(PageBackground::Blank);
        let page_id = doc.pages[0].id;
        let img = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            hash.clone(),
            Rect::new(-50.0, -50.0, 50.0, 50.0),
            1,
            1.0,
            80,
            80,
        );
        doc.pages[0].items.push(Item {
            id: 2,
            z_index: 1,
            kind: ItemKind::Image(img),
        });

        let out_dir =
            std::env::temp_dir().join(format!("uninote_test_wb_thumb_{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&out_dir);
        let out_png = out_dir.join("wb_thumb.png");

        let res = render_document_thumbnail(&doc, &out_png);
        assert!(res.is_ok(), "Rendering whiteboard thumbnail must succeed");
        assert!(out_png.exists());

        let _ = fs::remove_dir_all(out_dir);
    });
}

#[test]
fn test_m4_export_pdf_with_image_poppler_verification() {
    run_on_gtk_thread(|| {
        let (png_bytes, hash) = create_test_png(120, 90, 0.2, 0.8, 0.3);
        {
            let cache_rc = get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            let _ = cache.insert_from_bytes(&hash, &png_bytes);
        }

        let mut doc = Document::new_notebook("PDF Image Export Test");
        let page_id = doc.pages[0].id;
        let img = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            hash.clone(),
            Rect::new(80.0, 100.0, 300.0, 260.0),
            1,
            1.0,
            120,
            90,
        );
        doc.pages[0].items.push(Item {
            id: 10,
            z_index: 1,
            kind: ItemKind::Image(img),
        });

        let out_dir = std::env::temp_dir().join(format!("uninote_test_pdf_{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&out_dir);
        let out_pdf = out_dir.join("test_export.pdf");

        let export_res = export_to_pdf(&doc, &out_pdf);
        assert!(
            export_res.is_ok(),
            "Export to PDF must succeed: {:?}",
            export_res
        );
        assert!(out_pdf.exists(), "PDF file must exist");

        // Poppler verification
        let pdf_path_str = out_pdf.to_str().unwrap();
        let poppler_doc = poppler::PopplerDocument::new_from_file(pdf_path_str, None)
            .expect("Poppler must open generated PDF cleanly");
        assert_eq!(poppler_doc.get_n_pages(), 1, "Must contain exactly 1 page");

        let page0 = poppler_doc.get_page(0).expect("Page 0");
        let (pw, ph) = page0.get_size();
        assert!((pw - A4_WIDTH_PT).abs() < 2.0);
        assert!((ph - A4_HEIGHT_PT).abs() < 2.0);

        let _ = fs::remove_dir_all(out_dir);
    });
}

#[test]
fn test_m4_export_pdf_with_store_offline_reload() {
    run_on_gtk_thread(|| {
        let (png_bytes, hash) = create_test_png(50, 50, 0.8, 0.4, 0.1);
        let temp_dir = std::env::temp_dir().join(format!("uninote_test_store_{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&temp_dir);
        let db_path = temp_dir.join("note.uninote");

        // Salva l'asset nel database SQLite
        {
            let store = DocumentStore::open_or_create(&db_path).expect("open store");
            store
                .save_asset(&hash, "image/png", &png_bytes)
                .expect("save asset");
        }

        // Svuota intenzionalmente la cache in memoria per simulare un avvio a freddo
        {
            let cache_rc = get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            cache.clear();
            assert!(!cache.contains(&hash));
        }

        let mut doc = Document::new_notebook("Offline Reload PDF Export");
        let page_id = doc.pages[0].id;
        let img = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            hash.clone(),
            Rect::new(100.0, 100.0, 250.0, 250.0),
            1,
            0.9,
            50,
            50,
        );
        doc.pages[0].items.push(Item {
            id: 25,
            z_index: 1,
            kind: ItemKind::Image(img),
        });

        let out_pdf = temp_dir.join("offline_export.pdf");
        let opts = PdfExportOptions::default();

        let res = export_to_pdf_with_store(&doc, &out_pdf, &opts, Some(&db_path));
        assert!(
            res.is_ok(),
            "Export with store path must succeed and load asset: {:?}",
            res
        );
        assert!(out_pdf.exists());

        // Verifica che durante l'esportazione l'asset sia stato ricaricato nella cache
        {
            let cache_rc = get_global_image_cache();
            let cache = cache_rc.borrow();
            assert!(
                cache.contains(&hash),
                "Asset must now be populated in LRU cache"
            );
        }

        let _ = fs::remove_dir_all(temp_dir);
    });
}

#[test]
fn test_m4_export_pdf_dark_theme_with_image() {
    run_on_gtk_thread(|| {
        let (png_bytes, hash) = create_test_png(70, 70, 0.3, 0.6, 0.9);
        {
            let cache_rc = get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            let _ = cache.insert_from_bytes(&hash, &png_bytes);
        }

        let mut doc = Document::new_notebook("Dark PDF Export");
        let page_id = doc.pages[0].id;
        let img = ImageItem::with_id(
            Uuid::new_v4(),
            page_id,
            hash.clone(),
            Rect::new(50.0, 50.0, 200.0, 200.0),
            1,
            1.0,
            70,
            70,
        );
        doc.pages[0].items.push(Item {
            id: 30,
            z_index: 1,
            kind: ItemKind::Image(img),
        });

        let out_dir =
            std::env::temp_dir().join(format!("uninote_test_dark_pdf_{}", Uuid::new_v4()));
        let _ = fs::create_dir_all(&out_dir);
        let out_pdf = out_dir.join("dark_export.pdf");

        let dark_opts = PdfExportOptions { is_dark: true };

        let res = export_to_pdf_with_store(&doc, &out_pdf, &dark_opts, None);
        assert!(
            res.is_ok(),
            "Dark PDF export with image must succeed: {:?}",
            res
        );
        assert!(out_pdf.exists());

        let poppler_doc = poppler::PopplerDocument::new_from_file(out_pdf.to_str().unwrap(), None)
            .expect("Poppler opens dark PDF");
        assert_eq!(poppler_doc.get_n_pages(), 1);

        let _ = fs::remove_dir_all(out_dir);
    });
}
