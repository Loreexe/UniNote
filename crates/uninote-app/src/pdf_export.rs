//! Motore di esportazione PDF vettoriale e gestione flusso documentale universitario per UniNote.
//!
//! Genera documenti PDF multi-pagina conformi agli standard tipografici accademici:
//! - Fedeltà vettoriale pura: nessun raster dei tratti; interpolazione Catmull-Rom in curve di Bézier cubiche (`cr.curve_to`).
//! - Preservazione degli spessori variabili da pressione e trasparenze native dell'evidenziatore (`alpha = 0.38`).
//! - Re-iniezione non distruttiva degli sfondi PDF originali tramite `poppler::PopplerPage::render_for_printing`.
//! - Sfondi accademici vettoriali (Bianco caldo naturale, Righe universitarie 8mm, Quadretti matematici 5mm).

use cairo::{Context, LineCap, LineJoin, PdfMetadata, PdfSurface, PdfVersion};
use std::collections::HashMap;
use std::f64::consts::PI;
use std::path::Path;
use uninote_core::coordinates::Rect2D;
use uninote_core::model::{
    Document, DocumentFormat, ItemKind, Page, PageBackground, A4_HEIGHT_PT, A4_WIDTH_PT,
};

/// Colore di base per fogli accademici chiari: Bianco caldo naturale (#FAF9F6).
const WARM_WHITE: (f64, f64, f64) = (0.985, 0.980, 0.965);

/// Opzioni di configurazione per l'esportazione tipografica in PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PdfExportOptions {
    /// Se true, esporta su sfondo scuro Deep Slate (#172126) con inchiostri e griglie adattati;
    /// se false, esporta su sfondo chiaro naturale accademico (#FAF9F6).
    pub is_dark: bool,
}

/// Esporta un documento UniNote in un file PDF vettoriale multi-pagina ad alta fedeltà con opzioni tematiche.
///
/// Restituisce `Ok(())` al completamento, oppure un errore esplicito se il documento
/// è privo di pagine o si verificano errori I/O o Cairo.
pub fn export_to_pdf_with_options(
    document: &Document,
    output_path: &Path,
    options: &PdfExportOptions,
) -> Result<(), Box<dyn std::error::Error>> {
    if document.pages.is_empty() {
        return Err("Impossibile esportare un documento privo di pagine".into());
    }

    // 1. Inizializzazione della superficie PDF con le dimensioni della prima pagina
    let first_page = &document.pages[0];
    let (first_w, first_h, _, _) = calculate_page_geometry(document, first_page);

    let surface = PdfSurface::new(first_w, first_h, output_path)
        .map_err(|e| format!("Errore creazione cairo::PdfSurface: {:?}", e))?;
    surface
        .restrict(PdfVersion::_1_5)
        .map_err(|e| format!("Errore impostazione versione PDF: {:?}", e))?;

    if !document.title.trim().is_empty() {
        surface
            .set_metadata(PdfMetadata::Title, &document.title)
            .map_err(|e| format!("Errore metadati titolo: {:?}", e))?;
    }
    surface
        .set_metadata(
            PdfMetadata::Creator,
            concat!("UniNote Academic Suite (v", env!("CARGO_PKG_VERSION"), ")"),
        )
        .map_err(|e| format!("Errore metadati creator: {:?}", e))?;

    // Cache locale per i documenti Poppler aperti durante l'esportazione
    let mut poppler_cache: HashMap<String, poppler::PopplerDocument> = HashMap::new();

    // 2. Rendering sequenziale di ciascuna pagina
    for page in &document.pages {
        let (page_w, page_h, shift_x, shift_y) = calculate_page_geometry(document, page);
        surface
            .set_size(page_w, page_h)
            .map_err(|e| format!("Errore surface.set_size: {:?}", e))?;

        let cr = Context::new(&surface)
            .map_err(|e| format!("Errore inizializzazione cairo::Context: {:?}", e))?;

        // 2.1. Rendering dello sfondo
        render_background(
            &cr,
            page,
            page_w,
            page_h,
            options.is_dark,
            &mut poppler_cache,
        )?;

        // 2.2. Rendering vettoriale dei tratti dello studente
        render_strokes(&cr, &page.items, shift_x, shift_y, options.is_dark)?;

        cr.show_page()
            .map_err(|e| format!("Errore cr.show_page: {:?}", e))?;
    }

    surface.finish();
    Ok(())
}

/// Esporta un documento UniNote in un file PDF vettoriale multi-pagina ad alta fedeltà (default chiaro accademico).
pub fn export_to_pdf(
    document: &Document,
    output_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    export_to_pdf_with_options(document, output_path, &PdfExportOptions::default())
}

/// Crea un nuovo `Document` importando tutte le pagine di un file PDF esterno.
pub fn import_from_pdf(pdf_path: &Path) -> Result<Document, Box<dyn std::error::Error>> {
    let path_str = pdf_path
        .to_str()
        .ok_or_else(|| "Percorso PDF non valido (UTF-8 non conforme)".to_string())?;

    let doc = poppler::PopplerDocument::new_from_file(path_str, None)
        .map_err(|e| format!("Impossibile aprire il file PDF tramite Poppler: {:?}", e))?;

    let n_pages = doc.get_n_pages();
    if n_pages == 0 {
        return Err("Il file PDF contiene 0 pagine".into());
    }

    let title = pdf_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Documento PDF")
        .to_string();

    let mut pages = Vec::with_capacity(n_pages);
    for i in 0..n_pages {
        let page = doc
            .get_page(i)
            .ok_or_else(|| format!("Impossibile leggere la pagina {} del PDF", i))?;
        let (w, h) = page.get_size();
        let width = if w > 0.0 { w } else { A4_WIDTH_PT };
        let height = if h > 0.0 { h } else { A4_HEIGHT_PT };

        pages.push(Page::new_pdf_page(i, path_str, i, width, height));
    }

    Ok(Document {
        id: uuid::Uuid::new_v4(),
        title,
        version: 1,
        format: DocumentFormat::Notebook,
        pages,
    })
}

/// Accoda le pagine di un file PDF a un documento esistente, restituendo il numero di pagine aggiunte.
pub fn append_pdf_pages(
    document: &mut Document,
    pdf_path: &Path,
) -> Result<usize, Box<dyn std::error::Error>> {
    let path_str = pdf_path
        .to_str()
        .ok_or_else(|| "Percorso PDF non valido (UTF-8 non conforme)".to_string())?;

    let doc = poppler::PopplerDocument::new_from_file(path_str, None)
        .map_err(|e| format!("Impossibile aprire il file PDF tramite Poppler: {:?}", e))?;

    let n_pages = doc.get_n_pages();
    if n_pages == 0 {
        return Ok(0);
    }

    let start_idx = document.pages.len();
    for i in 0..n_pages {
        let page = doc
            .get_page(i)
            .ok_or_else(|| format!("Impossibile leggere la pagina {} del PDF", i))?;
        let (w, h) = page.get_size();
        let width = if w > 0.0 { w } else { A4_WIDTH_PT };
        let height = if h > 0.0 { h } else { A4_HEIGHT_PT };

        document.pages.push(Page::new_pdf_page(
            start_idx + i,
            path_str,
            i,
            width,
            height,
        ));
    }

    Ok(n_pages)
}

/// Calcola le dimensioni geometriche di esportazione di una pagina e i relativi shift di coordinate.
fn calculate_page_geometry(document: &Document, page: &Page) -> (f64, f64, f64, f64) {
    if document.format == DocumentFormat::Notebook
        && page.width.is_finite()
        && page.width > 0.0
        && page.height.is_finite()
        && page.height > 0.0
    {
        let page_rect = document.page_rect(page.index).unwrap_or(Rect2D::new(
            0.0,
            0.0,
            page.width,
            page.height,
        ));
        return (page.width, page.height, -page_rect.x, -page_rect.y);
    }

    // Modalità Lavagna Infinita: calcola il bounding box effettivo di tutti i tratti
    if document.format == DocumentFormat::Whiteboard
        || page.width.is_infinite()
        || page.height.is_infinite()
    {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;

        for item in &page.items {
            if let ItemKind::Stroke(ref s) = item.kind {
                for p in &s.points {
                    min_x = min_x.min(p.x);
                    min_y = min_y.min(p.y);
                    max_x = max_x.max(p.x);
                    max_y = max_y.max(p.y);
                }
            } else if let ItemKind::Image(ref img) = item.kind {
                min_x = min_x.min(img.bbox.min_x);
                min_y = min_y.min(img.bbox.min_y);
                max_x = max_x.max(img.bbox.max_x);
                max_y = max_y.max(img.bbox.max_y);
            }
        }

        if min_x.is_finite() && max_x.is_finite() && min_y.is_finite() && max_y.is_finite() {
            let margin = 40.0;
            let shift_x = margin - min_x;
            let shift_y = margin - min_y;
            let w = (max_x - min_x + margin * 2.0).max(A4_WIDTH_PT);
            let h = (max_y - min_y + margin * 2.0).max(A4_HEIGHT_PT);
            return (w, h, shift_x, shift_y);
        }
    }

    (A4_WIDTH_PT, A4_HEIGHT_PT, 0.0, 0.0)
}

/// Renderizza lo sfondo della pagina (vettoriale Poppler, righe, quadretti, tema chiaro o scuro).
fn render_background(
    cr: &Context,
    page: &Page,
    w: f64,
    h: f64,
    is_dark: bool,
    poppler_cache: &mut HashMap<String, poppler::PopplerDocument>,
) -> Result<(), Box<dyn std::error::Error>> {
    cr.save()?;

    match &page.background {
        PageBackground::PdfPage {
            source_path,
            page_index,
            ..
        } => {
            // Per le slide e dispense PDF, riempimento base bianco ottico puro (#FFFFFF)
            cr.set_source_rgb(1.0, 1.0, 1.0);
            cr.rectangle(0.0, 0.0, w, h);
            cr.fill()?;

            let doc = match poppler_cache.entry(source_path.clone()) {
                std::collections::hash_map::Entry::Occupied(e) => Some(e.into_mut()),
                std::collections::hash_map::Entry::Vacant(e) => {
                    poppler::PopplerDocument::new_from_file(source_path, None)
                        .ok()
                        .map(|d| e.insert(d))
                }
            };

            if let Some(doc) = doc {
                if let Some(pdf_page) = doc.get_page(*page_index) {
                    let (pw, ph) = pdf_page.get_size();
                    if pw > 0.0 && ph > 0.0 && ((pw - w).abs() > 0.1 || (ph - h).abs() > 0.1) {
                        cr.scale(w / pw, h / ph);
                    }
                    pdf_page.render_for_printing(cr);
                }
            } else {
                // Segnaposto se il file PDF originale non è reperibile su disco
                cr.set_line_width(1.0);
                cr.set_source_rgba(0.90, 0.40, 0.40, 0.5);
                cr.rectangle(10.0, 10.0, w - 20.0, h - 20.0);
                cr.stroke()?;
            }
        }
        _ => {
            if is_dark {
                // Tema scuro DMS Deep Slate (#172126)
                cr.set_source_rgb(0.090, 0.129, 0.149);
                cr.rectangle(0.0, 0.0, w, h);
                cr.fill()?;

                match &page.background {
                    PageBackground::Blank => {}
                    PageBackground::Ruled {
                        line_spacing,
                        margin_left,
                    } => {
                        // Righe orizzontali DMS in tema scuro
                        cr.set_line_width(0.8);
                        cr.set_source_rgba(0.427, 0.824, 1.0, 0.18);
                        let mut y = 60.0;
                        while y < h - 40.0 {
                            cr.move_to(0.0, y);
                            cr.line_to(w, y);
                            y += line_spacing;
                        }
                        cr.stroke()?;

                        // Margine sinistro Corallo DMS
                        cr.set_line_width(1.0);
                        cr.set_source_rgba(1.0, 0.706, 0.670, 0.35);
                        cr.move_to(*margin_left, 0.0);
                        cr.line_to(*margin_left, h);
                        cr.stroke()?;
                    }
                    PageBackground::Grid { grid_size } => {
                        // Quadrettatura DMS in tema scuro
                        cr.set_line_width(0.7);
                        cr.set_source_rgba(0.427, 0.824, 1.0, 0.14);
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
                        cr.stroke()?;
                    }
                    PageBackground::PdfPage { .. } => unreachable!(),
                }
            } else {
                // Tema chiaro accademico: Bianco naturale (#FAF9F6)
                cr.set_source_rgb(WARM_WHITE.0, WARM_WHITE.1, WARM_WHITE.2);
                cr.rectangle(0.0, 0.0, w, h);
                cr.fill()?;

                match &page.background {
                    PageBackground::Blank => {}
                    PageBackground::Ruled {
                        line_spacing,
                        margin_left,
                    } => {
                        // Righe orizzontali universitarie azzurro tenue
                        cr.set_line_width(0.8);
                        cr.set_source_rgba(0.35, 0.50, 0.70, 0.35);
                        let mut y = 60.0;
                        while y < h - 40.0 {
                            cr.move_to(0.0, y);
                            cr.line_to(w, y);
                            y += line_spacing;
                        }
                        cr.stroke()?;

                        // Margine verticale sinistro accademico
                        cr.set_line_width(1.0);
                        cr.set_source_rgba(0.88, 0.35, 0.35, 0.35);
                        cr.move_to(*margin_left, 0.0);
                        cr.line_to(*margin_left, h);
                        cr.stroke()?;
                    }
                    PageBackground::Grid { grid_size } => {
                        // Quadrettatura matematica ortogonale
                        cr.set_line_width(0.7);
                        cr.set_source_rgba(0.45, 0.60, 0.80, 0.28);
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
                        cr.stroke()?;
                    }
                    PageBackground::PdfPage { .. } => unreachable!(),
                }
            }
        }
    }

    cr.restore()?;
    Ok(())
}

/// Renderizza i tratti a mano libera e le annotazioni come curve di Bézier cubiche pure (`cr.curve_to`)
/// con proiezione percettiva del colore e raggruppamento alfa pulito per gli evidenziatori.
fn render_strokes(
    cr: &Context,
    items: &[uninote_core::model::Item],
    shift_x: f64,
    shift_y: f64,
    is_dark: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    cr.save()?;
    if shift_x != 0.0 || shift_y != 0.0 {
        cr.translate(shift_x, shift_y);
    }

    for item in items {
        let stroke = match &item.kind {
            ItemKind::Stroke(ref s) => s,
            _ => continue,
        };
        if stroke.points.is_empty() {
            continue;
        }

        cr.save()?;
        let adapted = crate::canvas::adapt_ink_color(stroke.style.color, is_dark);
        let is_semi_transparent = adapted.3 < 0.999;

        if is_semi_transparent {
            cr.push_group_with_content(cairo::Content::ColorAlpha);
            cr.set_source_rgb(adapted.0, adapted.1, adapted.2);
        } else {
            cr.set_source_rgba(adapted.0, adapted.1, adapted.2, adapted.3);
        }

        cr.set_line_cap(LineCap::Round);
        cr.set_line_join(LineJoin::Round);

        if stroke.points.len() == 1 || stroke.is_dot {
            // Punto singolo isolato (dot)
            let p = &stroke.points[0];
            let radius = if p.width > 0.0 {
                p.width / 2.0
            } else {
                stroke.style.base_width / 2.0
            };
            cr.arc(p.x, p.y, radius.max(0.5), 0.0, 2.0 * PI);
            cr.fill()?;
        } else {
            // Conversione Catmull-Rom -> spline Bézier cubiche continue C1
            let segments = uninote_stroke::geometry::catmull_rom_to_bezier(&stroke.points);
            if segments.is_empty() {
                // Fallback di congiunzione diretta per sequenze degenerate a 2 punti
                for window in stroke.points.windows(2) {
                    let p0 = &window[0];
                    let p1 = &window[1];
                    let avg_w = ((p0.width + p1.width) / 2.0).max(0.5);
                    cr.set_line_width(avg_w);
                    cr.move_to(p0.x, p0.y);
                    cr.line_to(p1.x, p1.y);
                    cr.stroke()?;
                }
            } else {
                for seg in segments {
                    let avg_w = ((seg.width0 + seg.width1) / 2.0).max(0.5);
                    cr.set_line_width(avg_w);
                    cr.move_to(seg.p0.0, seg.p0.1);
                    cr.curve_to(
                        seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
                    );
                    cr.stroke()?;
                }
            }
        }

        if is_semi_transparent {
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(adapted.3);
        }

        cr.restore()?;
    }

    cr.restore()?;
    Ok(())
}
