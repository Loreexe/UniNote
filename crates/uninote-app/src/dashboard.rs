use crate::icons::{create_icon, IconKind};
use crate::preferences::{
    next_available_note_path_in_dir, DashboardRecentStore, RecentDocumentMeta,
};
use gtk4::cairo::{Context, Format, ImageSurface};
use gtk4::gdk;
use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DragSource, DropDown, DropTarget, Entry, FileDialog, FlowBox,
    FlowBoxChild, GestureClick, Label, ListBox, ListBoxRow, Orientation, Picture, ScrolledWindow,
    SearchEntry, Window,
};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use uninote_core::model::{DocumentFormat, PageBackground};

/// Cartella predefinita per la cache delle miniature Cairo.
pub fn thumbnail_cache_dir() -> PathBuf {
    glib::user_cache_dir().join("uninote").join("thumbnails")
}

/// Genera o restituisce il percorso della miniatura PNG associata al documento.
pub fn is_dark_theme_active() -> bool {
    if gtk4::is_initialized() {
        libadwaita::StyleManager::default().is_dark()
    } else {
        true
    }
}

pub fn get_or_create_thumbnail_path(meta: &RecentDocumentMeta) -> PathBuf {
    let cache_dir = thumbnail_cache_dir();
    let sanitized_title: String = meta
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    let file_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        meta.file_path.hash(&mut hasher);
        hasher.finish()
    };
    let is_dark = is_dark_theme_active();
    let theme_suffix = if is_dark { "dark" } else { "light" };
    let thumb_filename = format!("{}_{:x}_{}.png", sanitized_title, file_hash, theme_suffix);
    let thumb_path = cache_dir.join(thumb_filename);

    let needs_generation = if !thumb_path.exists() {
        true
    } else {
        let doc_mtime = fs::metadata(&meta.file_path)
            .and_then(|m| m.modified())
            .ok();
        let thumb_mtime = fs::metadata(&thumb_path).and_then(|m| m.modified()).ok();
        match (doc_mtime, thumb_mtime) {
            (Some(dm), Some(tm)) => dm > tm,
            _ => false,
        }
    };

    if needs_generation {
        let p = Path::new(&meta.file_path);
        let rendered_real = if p.exists() {
            if let Ok(store) = uninote_storage::DocumentStore::open_or_create(p) {
                if let Ok(doc) = store.load_document() {
                    render_document_thumbnail_with_store(&doc, &thumb_path, Some(&store)).is_ok()
                } else {
                    false
                }
            } else {
                false
            }
        } else {
            false
        };

        if !rendered_real {
            let _ = render_cairo_thumbnail(&meta.title, meta.format, &thumb_path);
        }
    }
    thumb_path
}

/// Rigenera immediatamente e in modo sincrono la miniatura per il documento corrente.
pub fn update_document_thumbnail_now(
    meta: &RecentDocumentMeta,
    doc: &uninote_core::Document,
    store: Option<&uninote_storage::DocumentStore>,
) -> PathBuf {
    let cache_dir = thumbnail_cache_dir();
    let sanitized_title: String = meta
        .title
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();

    let file_hash = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        meta.file_path.hash(&mut hasher);
        hasher.finish()
    };
    let is_dark = is_dark_theme_active();
    let theme_suffix = if is_dark { "dark" } else { "light" };
    let thumb_filename = format!("{}_{:x}_{}.png", sanitized_title, file_hash, theme_suffix);
    let thumb_path = cache_dir.join(thumb_filename);
    let _ = render_document_thumbnail_with_store(doc, &thumb_path, store);
    thumb_path
}

/// Restituisce l'etichetta testuale del badge DMS per il formato documento specificato.
pub fn format_badge_label(format: DocumentFormat) -> &'static str {
    match format {
        DocumentFormat::Notebook => "Quaderno A4",
        DocumentFormat::Whiteboard => "Lavagna Infinita",
    }
}

/// Mostra una finestra di dialogo modale di conferma per azioni distruttive o conferme DMS.
pub fn show_confirmation_dialog<F>(
    parent: Option<&Window>,
    title: &str,
    message: &str,
    confirm_label: &str,
    is_destructive: bool,
    on_confirm: F,
) where
    F: FnOnce() + 'static,
{
    let dlg = Window::builder()
        .title(title)
        .modal(true)
        .default_width(400)
        .default_height(160)
        .css_classes(["uninote-dialog", "dms-dialog"])
        .build();

    if let Some(p) = parent {
        dlg.set_transient_for(Some(p));
    }

    let dlg_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(14)
        .margin_start(20)
        .margin_end(20)
        .margin_top(20)
        .margin_bottom(20)
        .build();

    let lbl = Label::builder()
        .label(message)
        .wrap(true)
        .xalign(0.0)
        .css_classes(["dim-label"])
        .build();
    dlg_box.append(&lbl);

    let act_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(10)
        .halign(Align::End)
        .build();

    let btn_cancel = Button::builder()
        .label("Annulla")
        .css_classes(["dms-action-btn"])
        .build();

    let confirm_builder = Button::builder().label(confirm_label);
    let btn_confirm = if is_destructive {
        confirm_builder
            .css_classes(["destructive-action", "dms-btn-primary"])
            .build()
    } else {
        confirm_builder
            .css_classes(["suggested-action", "dms-btn-primary"])
            .build()
    };

    act_box.append(&btn_cancel);
    act_box.append(&btn_confirm);
    dlg_box.append(&act_box);
    dlg.set_child(Some(&dlg_box));

    let dlg_cancel = dlg.clone();
    btn_cancel.connect_clicked(move |_| {
        dlg_cancel.close();
    });

    let dlg_conf = dlg.clone();
    let on_confirm_cell = std::rc::Rc::new(std::cell::RefCell::new(Some(on_confirm)));
    btn_confirm.connect_clicked(move |_| {
        dlg_conf.close();
        if let Some(cb) = on_confirm_cell.borrow_mut().take() {
            cb();
        }
    });

    dlg.present();
}

/// Genera la miniatura per il documento renderizzando la prima pagina (o fallback a sfondo procedurale).
pub fn render_document_thumbnail(
    doc: &uninote_core::Document,
    output_png: &Path,
) -> Result<(), String> {
    render_document_thumbnail_with_store(doc, output_png, None)
}

/// Genera la miniatura per il documento con supporto opzionale al DocumentStore per caricamento asset.
pub fn render_document_thumbnail_with_store(
    doc: &uninote_core::Document,
    output_png: &Path,
    store: Option<&uninote_storage::DocumentStore>,
) -> Result<(), String> {
    let is_dark = is_dark_theme_active();
    let (w, h) = (240, 340);
    let surface = ImageSurface::create(Format::ARgb32, w, h)
        .map_err(|e| format!("Thumbnail surface error: {:?}", e))?;
    let cr = Context::new(&surface).map_err(|e| format!("Thumbnail context error: {:?}", e))?;

    // Sfondo perimetrale esterno del canvas miniatura conforme al tema
    if is_dark {
        cr.set_source_rgb(0.039, 0.082, 0.102); // Dark Deep Slate (#0A151A)
    } else {
        cr.set_source_rgb(0.910, 0.925, 0.937); // Light Canvas (#E8ECEF)
    }
    cr.rectangle(0.0, 0.0, w as f64, h as f64);
    cr.fill().map_err(|e| format!("{:?}", e))?;

    match doc.format {
        DocumentFormat::Notebook => {
            // Disegna il foglio A4 proporzionato fedele al tema attivo
            let pad = 12.0;
            let paper_w = w as f64 - pad * 2.0;
            let paper_h =
                paper_w * (uninote_core::model::A4_HEIGHT_PT / uninote_core::model::A4_WIDTH_PT);
            let paper_x = pad;
            let paper_y = (h as f64 - paper_h) / 2.0;

            // Ombra
            if is_dark {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.65);
            } else {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
            }
            cr.rectangle(paper_x - 1.0, paper_y + 2.0, paper_w + 2.0, paper_h + 2.0);
            let _ = cr.fill();

            // Corpo foglio A4 (Fedele al tema: Dark Deep Slate #172126 o Natural White #FAF9F6)
            if is_dark {
                cr.set_source_rgb(0.090, 0.129, 0.149);
            } else {
                cr.set_source_rgb(0.985, 0.980, 0.965);
            }
            cr.rectangle(paper_x, paper_y, paper_w, paper_h);
            let _ = cr.fill();

            // Bordo foglio
            if is_dark {
                cr.set_source_rgba(0.243, 0.282, 0.306, 0.80);
            } else {
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.12);
            }
            cr.set_line_width(0.8);
            cr.rectangle(paper_x, paper_y, paper_w, paper_h);
            let _ = cr.stroke();

            // Clip all'interno del foglio
            let _ = cr.save();
            cr.rectangle(paper_x, paper_y, paper_w, paper_h);
            cr.clip();

            if let Some(first_page) = doc.pages.first() {
                // Rigatura o quadretti tenui coordinati col tema
                match &first_page.background {
                    PageBackground::Blank => {}
                    PageBackground::Ruled {
                        line_spacing,
                        margin_left,
                    } => {
                        let scale = paper_w / uninote_core::model::A4_WIDTH_PT;
                        let spacing_px = line_spacing * scale;
                        let margin_px = margin_left * scale;

                        if is_dark {
                            cr.set_source_rgba(1.0, 0.706, 0.670, 0.35); // Corallo DMS
                        } else {
                            cr.set_source_rgba(0.88, 0.35, 0.35, 0.30);
                        }
                        cr.set_line_width(0.8);
                        cr.move_to(paper_x + margin_px, paper_y);
                        cr.line_to(paper_x + margin_px, paper_y + paper_h);
                        let _ = cr.stroke();

                        if is_dark {
                            cr.set_source_rgba(0.427, 0.824, 1.0, 0.18);
                        } else {
                            cr.set_source_rgba(0.35, 0.50, 0.70, 0.25);
                        }
                        cr.set_line_width(0.6);
                        let mut ly = paper_y + spacing_px * 2.0;
                        while ly < paper_y + paper_h {
                            cr.move_to(paper_x, ly);
                            cr.line_to(paper_x + paper_w, ly);
                            ly += spacing_px;
                        }
                        let _ = cr.stroke();
                    }
                    PageBackground::Grid { grid_size } => {
                        let scale = paper_w / uninote_core::model::A4_WIDTH_PT;
                        let grid_px = (grid_size * scale).max(3.0);
                        if is_dark {
                            cr.set_source_rgba(0.427, 0.824, 1.0, 0.14);
                        } else {
                            cr.set_source_rgba(0.45, 0.60, 0.80, 0.22);
                        }
                        cr.set_line_width(0.5);

                        let mut gx = paper_x + grid_px;
                        while gx < paper_x + paper_w {
                            cr.move_to(gx, paper_y);
                            cr.line_to(gx, paper_y + paper_h);
                            gx += grid_px;
                        }
                        let mut gy = paper_y + grid_px;
                        while gy < paper_y + paper_h {
                            cr.move_to(paper_x, gy);
                            cr.line_to(paper_x + paper_w, gy);
                            gy += grid_px;
                        }
                        let _ = cr.stroke();
                    }
                    _ => {}
                }

                // Renderizza elementi reali della pagina 1 (tratti e immagini con contrasto adattato)
                let scale = paper_w / uninote_core::model::A4_WIDTH_PT;
                let _ = cr.save();
                cr.translate(paper_x, paper_y);
                cr.scale(scale, scale);

                for item in &first_page.items {
                    match &item.kind {
                        uninote_core::ItemKind::Stroke(stroke) => {
                            if stroke.points.is_empty() {
                                continue;
                            }
                            let adapted =
                                crate::canvas::adapt_ink_color(stroke.style.color, is_dark);
                            cr.set_source_rgba(adapted.0, adapted.1, adapted.2, adapted.3);
                            cr.set_line_cap(gtk4::cairo::LineCap::Round);
                            cr.set_line_join(gtk4::cairo::LineJoin::Round);

                            if stroke.points.len() == 1 {
                                let p = &stroke.points[0];
                                cr.arc(
                                    p.x,
                                    p.y,
                                    (p.width / 2.0).max(1.0),
                                    0.0,
                                    2.0 * std::f64::consts::PI,
                                );
                                let _ = cr.fill();
                            } else {
                                for w in stroke.points.windows(2) {
                                    cr.set_line_width(w[0].width.max(1.2));
                                    cr.move_to(w[0].x, w[0].y);
                                    cr.line_to(w[1].x, w[1].y);
                                    let _ = cr.stroke();
                                }
                            }
                        }
                        uninote_core::ItemKind::Image(img) => {
                            let cache_rc = crate::image_cache::get_global_image_cache();
                            let mut cache = cache_rc.borrow_mut();
                            let surface_opt = if let Some(surf) = cache.get(&img.asset_hash) {
                                Some(surf)
                            } else if let Some(st) = store {
                                cache.get_or_load_from_store(&img.asset_hash, st)
                            } else {
                                None
                            };

                            if let Some(surface) = surface_opt {
                                let surf_w = surface.width() as f64;
                                let surf_h = surface.height() as f64;
                                if surf_w > 0.0 && surf_h > 0.0 {
                                    let dest_x = img.bbox.min_x;
                                    let dest_y = img.bbox.min_y;
                                    let dest_w = img.bbox.width();
                                    let dest_h = img.bbox.height();
                                    if dest_w > 0.0 && dest_h > 0.0 {
                                        let _ = cr.save();
                                        cr.translate(dest_x, dest_y);
                                        cr.scale(dest_w / surf_w, dest_h / surf_h);
                                        let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                                        let pattern = cr.source();
                                        pattern.set_filter(gtk4::cairo::Filter::Bilinear);
                                        let opacity = img.opacity.clamp(0.0, 1.0);
                                        if opacity < 0.999 {
                                            let _ = cr.paint_with_alpha(opacity);
                                        } else {
                                            let _ = cr.paint();
                                        }
                                        let _ = cr.restore();
                                    }
                                }
                            }
                        }
                    }
                }
                let _ = cr.restore();
            }

            let _ = cr.restore();
        }
        DocumentFormat::Whiteboard => {
            // Matrice di punti per la lavagna procedurale (griglia discreta di sfondo conforme al tema)
            if is_dark {
                cr.set_source_rgba(0.475, 0.851, 0.647, 0.12);
            } else {
                cr.set_source_rgba(0.20, 0.35, 0.50, 0.18);
            }
            let mut x = 24.0;
            while x < w as f64 - 20.0 {
                let mut y = 24.0;
                while y < h as f64 - 20.0 {
                    cr.arc(x, y, 1.2, 0.0, 2.0 * std::f64::consts::PI);
                    cr.fill().map_err(|e| format!("{:?}", e))?;
                    y += 24.0;
                }
                x += 24.0;
            }

            // Inquadratura dinamica automatica del contenuto reale (senza cerchi artificiali)
            if let Some(first_page) = doc.pages.first() {
                if !first_page.items.is_empty() {
                    let mut bbox_opt: Option<(f64, f64, f64, f64)> = None;
                    for item in &first_page.items {
                        if let Some(b) = item.bounding_box() {
                            if b.min_x.is_finite()
                                && b.min_y.is_finite()
                                && b.max_x.is_finite()
                                && b.max_y.is_finite()
                            {
                                bbox_opt = match bbox_opt {
                                    None => Some((b.min_x, b.min_y, b.max_x, b.max_y)),
                                    Some((x1, y1, x2, y2)) => Some((
                                        x1.min(b.min_x),
                                        y1.min(b.min_y),
                                        x2.max(b.max_x),
                                        y2.max(b.max_y),
                                    )),
                                };
                            }
                        }
                    }

                    let (trans_x, trans_y, scale) =
                        if let Some((min_x, min_y, max_x, max_y)) = bbox_opt {
                            let content_w = (max_x - min_x).max(1.0);
                            let content_h = (max_y - min_y).max(1.0);
                            let pad = 24.0;
                            let avail_w = (w as f64 - pad * 2.0).max(10.0);
                            let avail_h = (h as f64 - pad * 2.0).max(10.0);
                            let s = (avail_w / content_w).min(avail_h / content_h).min(1.0);
                            let cx = (min_x + max_x) / 2.0;
                            let cy = (min_y + max_y) / 2.0;
                            ((w as f64 / 2.0) - cx * s, (h as f64 / 2.0) - cy * s, s)
                        } else {
                            (w as f64 / 2.0, h as f64 / 2.0, 0.3)
                        };

                    let _ = cr.save();
                    cr.translate(trans_x, trans_y);
                    cr.scale(scale, scale);
                    for item in &first_page.items {
                        match &item.kind {
                            uninote_core::ItemKind::Stroke(stroke) => {
                                if stroke.points.is_empty() {
                                    continue;
                                }
                                let adapted =
                                    crate::canvas::adapt_ink_color(stroke.style.color, is_dark);
                                cr.set_source_rgba(adapted.0, adapted.1, adapted.2, adapted.3);
                                cr.set_line_cap(gtk4::cairo::LineCap::Round);
                                cr.set_line_join(gtk4::cairo::LineJoin::Round);

                                if stroke.points.len() == 1 {
                                    let p = &stroke.points[0];
                                    cr.arc(
                                        p.x,
                                        p.y,
                                        (p.width / 2.0).max(1.0),
                                        0.0,
                                        2.0 * std::f64::consts::PI,
                                    );
                                    let _ = cr.fill();
                                } else {
                                    for w in stroke.points.windows(2) {
                                        cr.set_line_width(w[0].width.max(1.5));
                                        cr.move_to(w[0].x, w[0].y);
                                        cr.line_to(w[1].x, w[1].y);
                                        let _ = cr.stroke();
                                    }
                                }
                            }
                            uninote_core::ItemKind::Image(img) => {
                                let cache_rc = crate::image_cache::get_global_image_cache();
                                let mut cache = cache_rc.borrow_mut();
                                let surface_opt = if let Some(surf) = cache.get(&img.asset_hash) {
                                    Some(surf)
                                } else if let Some(st) = store {
                                    cache.get_or_load_from_store(&img.asset_hash, st)
                                } else {
                                    None
                                };

                                if let Some(surface) = surface_opt {
                                    let surf_w = surface.width() as f64;
                                    let surf_h = surface.height() as f64;
                                    if surf_w > 0.0 && surf_h > 0.0 {
                                        let dest_x = img.bbox.min_x;
                                        let dest_y = img.bbox.min_y;
                                        let dest_w = img.bbox.width();
                                        let dest_h = img.bbox.height();
                                        if dest_w > 0.0 && dest_h > 0.0 {
                                            let _ = cr.save();
                                            cr.translate(dest_x, dest_y);
                                            cr.scale(dest_w / surf_w, dest_h / surf_h);
                                            let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                                            let pattern = cr.source();
                                            pattern.set_filter(gtk4::cairo::Filter::Bilinear);
                                            let opacity = img.opacity.clamp(0.0, 1.0);
                                            if opacity < 0.999 {
                                                let _ = cr.paint_with_alpha(opacity);
                                            } else {
                                                let _ = cr.paint();
                                            }
                                            let _ = cr.restore();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    let _ = cr.restore();
                }
            }
        }
    }

    if let Some(parent) = output_png.parent() {
        if !parent.exists() {
            let _ = fs::create_dir_all(parent);
        }
    }

    let mut file = fs::File::create(output_png).map_err(|e| e.to_string())?;
    surface
        .write_to_png(&mut file)
        .map_err(|e| format!("PNG write error: {:?}", e))?;

    Ok(())
}

/// Genera una miniatura visiva 240x340 px in Cairo per una card del quaderno o della lavagna (fallback rapido).
pub fn render_cairo_thumbnail(
    _title: &str,
    format: DocumentFormat,
    output_png: &Path,
) -> Result<(), String> {
    let doc = match format {
        DocumentFormat::Notebook => uninote_core::Document::new_notebook("Quaderno"),
        DocumentFormat::Whiteboard => uninote_core::Document::new_whiteboard(PageBackground::Blank),
    };
    render_document_thumbnail(&doc, output_png)
}

/// Rappresenta una cartella o sottocartella rilevata nel filesystem UniNote.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderEntry {
    pub path: PathBuf,
    pub name: String,
    pub item_count: usize,
    pub last_modified: u64,
}

/// Valida e sanitizza il nome di una cartella per prevenire attacchi di path traversal o caratteri illegali.
pub fn sanitize_folder_name(name: &str) -> Result<String, &'static str> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Il nome della cartella non può essere vuoto");
    }
    if trimmed.len() > 255 {
        return Err("Il nome della cartella supera i 255 caratteri");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains('\0') {
        return Err("Il nome contiene caratteri di percorso non consentiti");
    }
    if trimmed == "." || trimmed == ".." {
        return Err("Nome cartella riservato non consentito");
    }
    if trimmed
        .chars()
        .any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return Err("Il nome contiene caratteri non consentiti");
    }
    Ok(trimmed.to_string())
}

/// Verifica difensivamente che un percorso sia un sottopercorso sicuro all'interno della cartella radice.
pub fn is_safe_subpath(root: &Path, candidate: &Path) -> bool {
    let Ok(canonical_root) = root.canonicalize() else {
        return candidate.starts_with(root) && !candidate.to_string_lossy().contains("..");
    };
    let Ok(canonical_candidate) = candidate.canonicalize() else {
        if let Some(parent) = candidate.parent() {
            if let Ok(canon_parent) = parent.canonicalize() {
                return canon_parent.starts_with(&canonical_root);
            }
        }
        return false;
    };
    canonical_candidate.starts_with(&canonical_root)
}

/// Sposta in modo atomico e sicuro un file da un percorso sorgente a una cartella di destinazione,
/// gestendo eventuali collisioni di nome aggiungendo un suffisso progressivo ed eseguendo
/// fallback a copy+remove se i filesystem sono distinti.
pub fn move_document_to_folder(src_path: &Path, target_dir: &Path) -> Result<PathBuf, String> {
    if !src_path.exists() {
        return Err("Il file di origine non esiste".to_string());
    }
    if !target_dir.exists() {
        std::fs::create_dir_all(target_dir)
            .map_err(|e| format!("Impossibile creare la cartella di destinazione: {}", e))?;
    }

    let file_name = src_path
        .file_name()
        .ok_or_else(|| "Nome file non valido".to_string())?;

    let mut dst_path = target_dir.join(file_name);
    if dst_path == src_path {
        return Ok(dst_path);
    }

    if dst_path.exists() {
        let stem = src_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("documento");
        let ext = src_path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("uninote");

        let mut idx = 1;
        loop {
            idx += 1;
            let candidate = target_dir.join(format!("{} ({}).{}", stem, idx, ext));
            if !candidate.exists() {
                dst_path = candidate;
                break;
            }
        }
    }

    if std::fs::rename(src_path, &dst_path).is_err() {
        std::fs::copy(src_path, &dst_path)
            .map_err(|e| format!("Errore durante lo spostamento del file: {}", e))?;
        let _ = std::fs::remove_file(src_path);
    }

    Ok(dst_path)
}

/// Collega un controller `DropTarget` GTK4 a un contenitore cartella (nella sidebar o nel flowbox),
/// consentendo di ricevere file trascinati da card di documenti e spostarli istantaneamente.
pub fn attach_folder_drop_target(
    widget: &GtkBox,
    target_dir: PathBuf,
    store: &Rc<RefCell<DashboardRecentStore>>,
    refresh_cb: &Rc<dyn Fn()>,
) {
    let drop_target = DropTarget::new(glib::Type::STRING, gdk::DragAction::MOVE);
    let store_clone = store.clone();
    let ref_clone = refresh_cb.clone();
    let target_clone = target_dir;
    let widget_weak = widget.downgrade();
    drop_target.connect_enter(move |_, _, _| {
        if let Some(w) = widget_weak.upgrade() {
            w.add_css_class("dms-drop-active");
        }
        gdk::DragAction::MOVE
    });
    let widget_weak2 = widget.downgrade();
    drop_target.connect_leave(move |_| {
        if let Some(w) = widget_weak2.upgrade() {
            w.remove_css_class("dms-drop-active");
        }
    });
    let widget_weak3 = widget.downgrade();
    drop_target.connect_drop(move |_, val, _, _| {
        if let Some(w) = widget_weak3.upgrade() {
            w.remove_css_class("dms-drop-active");
        }
        if let Ok(src_path_str) = val.get::<String>() {
            let src = PathBuf::from(&src_path_str);
            if src.exists() && src.parent() != Some(&target_clone) {
                if let Ok(new_path) = move_document_to_folder(&src, &target_clone) {
                    let new_title = new_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("Documento");
                    store_clone.borrow_mut().move_or_rename_recent(
                        &src_path_str,
                        &new_path.to_string_lossy(),
                        new_title,
                    );
                    let _ = store_clone.borrow().save();
                    ref_clone();
                    return true;
                }
            }
        }
        false
    });
    widget.add_controller(drop_target);
}

/// Esegue la scansione di una directory del filesystem, enumerando sottocartelle e file di appunti UniNote.
pub fn scan_directory(dir: &Path) -> Result<(Vec<FolderEntry>, Vec<PathBuf>), std::io::Error> {
    if !dir.exists() {
        return Ok((Vec::new(), Vec::new()));
    }
    let mut folders = Vec::new();
    let mut files = Vec::new();

    let entries = fs::read_dir(dir)?;
    for entry in entries.flatten() {
        let p = entry.path();
        let fname = entry.file_name().to_string_lossy().to_string();
        if fname.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            let mut item_count = 0;
            if let Ok(sub_entries) = fs::read_dir(&p) {
                for se in sub_entries.flatten() {
                    let sp = se.path();
                    let s_ext = sp.extension().and_then(|e| e.to_str()).unwrap_or("");
                    if sp.is_file() && (s_ext == "uninote" || s_ext.eq_ignore_ascii_case("pdf")) {
                        item_count += 1;
                    }
                }
            }
            let last_modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);

            folders.push(FolderEntry {
                path: p,
                name: fname,
                item_count,
                last_modified,
            });
        } else if p.is_file() {
            let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("");
            if ext == "uninote" || ext.eq_ignore_ascii_case("pdf") {
                files.push(p);
            }
        }
    }

    folders.sort_by_key(|a| a.name.to_lowercase());
    files.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

    Ok((folders, files))
}

/// Dialogo modale per la creazione di una nuova cartella nel filesystem.
pub struct CreateFolderDialog;

impl CreateFolderDialog {
    pub fn show<F>(parent: Option<&impl IsA<Window>>, current_dir: PathBuf, on_created: F)
    where
        F: Fn(PathBuf) + 'static,
    {
        let window = Window::builder()
            .title("Nuova Cartella — UniNote")
            .modal(true)
            .default_width(460)
            .default_height(260)
            .css_classes(["uninote-dialog", "dms-dialog"])
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p.as_ref()));
        }

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(14)
            .margin_start(24)
            .margin_end(24)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        let lbl_title = Label::builder()
            .label("Crea Nuova Cartella")
            .xalign(0.0)
            .css_classes(["title-2", "brand-title"])
            .build();
        main_box.append(&lbl_title);

        let parent_name = current_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Radice".into());
        let lbl_location = Label::builder()
            .label(format!("Destinazione: {}", parent_name))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        main_box.append(&lbl_location);

        let entry_name = Entry::builder()
            .placeholder_text("Nome cartella (es. Analisi 1, Fisica)")
            .css_classes(["dms-entry"])
            .build();
        main_box.append(&entry_name);

        let lbl_error = Label::builder()
            .label("")
            .xalign(0.0)
            .css_classes(["dms-dialog-error"])
            .visible(false)
            .build();
        main_box.append(&lbl_error);

        let actions_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .halign(Align::End)
            .margin_top(12)
            .build();

        let btn_cancel = Button::builder()
            .label("Annulla")
            .css_classes(["dms-action-btn"])
            .build();

        let btn_create = Button::builder()
            .label("Crea Cartella")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .build();

        actions_box.append(&btn_cancel);
        actions_box.append(&btn_create);
        main_box.append(&actions_box);

        let win_cancel = window.clone();
        btn_cancel.connect_clicked(move |_| {
            win_cancel.close();
        });

        let win_create = window.clone();
        let cur_dir = current_dir.clone();
        let cb = Rc::new(on_created);
        let entry_for_create = entry_name.clone();
        let err_label = lbl_error.clone();

        let do_create = move || {
            let raw_text = entry_for_create.text().to_string();
            match sanitize_folder_name(&raw_text) {
                Err(err_msg) => {
                    err_label.set_label(err_msg);
                    err_label.set_visible(true);
                }
                Ok(clean_name) => {
                    let target = cur_dir.join(&clean_name);
                    if target.exists() {
                        err_label.set_label("Esiste già una cartella con questo nome");
                        err_label.set_visible(true);
                        return;
                    }
                    if let Err(e) = fs::create_dir_all(&target) {
                        err_label.set_label(&format!("Errore creazione: {}", e));
                        err_label.set_visible(true);
                        return;
                    }
                    win_create.close();
                    cb(target);
                }
            }
        };

        let do_create_click = do_create.clone();
        btn_create.connect_clicked(move |_| {
            do_create_click();
        });

        entry_name.connect_activate(move |_| {
            do_create();
        });

        window.set_child(Some(&main_box));
        window.present();
    }
}

/// Dialogo modale per la scelta del formato alla creazione di un nuovo documento.
pub struct CreateDocumentDialog;

impl CreateDocumentDialog {
    pub fn show<F>(parent: Option<&impl IsA<Window>>, on_created: F)
    where
        F: Fn(String, DocumentFormat, PageBackground) + 'static,
    {
        let store = DashboardRecentStore::load();
        let default_dir = PathBuf::from(&store.default_notes_dir);
        Self::show_in_folder(parent, default_dir, on_created);
    }

    pub fn show_in_folder<F>(
        parent: Option<&impl IsA<Window>>,
        target_folder: PathBuf,
        on_created: F,
    ) where
        F: Fn(String, DocumentFormat, PageBackground) + 'static,
    {
        let window = Window::builder()
            .title("Nuovo Documento — UniNote")
            .modal(true)
            .default_width(520)
            .default_height(500)
            .css_classes(["uninote-dialog", "dms-dialog"])
            .build();

        if let Some(p) = parent {
            window.set_transient_for(Some(p.as_ref()));
        }

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(18)
            .margin_start(28)
            .margin_end(28)
            .margin_top(24)
            .margin_bottom(24)
            .build();

        let lbl_title = Label::builder()
            .label("Crea Nuovo Documento")
            .xalign(0.0)
            .css_classes(["title-1", "brand-title"])
            .build();
        main_box.append(&lbl_title);

        let folder_name = target_folder
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "UniNote".into());
        let lbl_subtitle = Label::builder()
            .label(format!(
                "Destinazione: {} — Scegli formato e layout",
                folder_name
            ))
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        main_box.append(&lbl_subtitle);

        // Nome Documento
        let lbl_name = Label::builder()
            .label("Titolo Documento:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_name);

        let entry_name = Entry::builder()
            .text("")
            .placeholder_text("Nuova nota (lascia vuoto per nome automatico)")
            .css_classes(["dms-entry"])
            .build();
        main_box.append(&entry_name);

        // Selezione Formato
        let lbl_format = Label::builder()
            .label("Formato di Studio:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_format);

        let format_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .homogeneous(true)
            .build();

        let btn_notebook = Button::builder()
            .label("Quaderno A4\nPagine discrete standard")
            .css_classes(["dms-format-card", "selected"])
            .build();

        let btn_whiteboard = Button::builder()
            .label("Lavagna Infinita\nSpazio libero (-∞, +∞)")
            .css_classes(["dms-format-card"])
            .build();

        format_box.append(&btn_notebook);
        format_box.append(&btn_whiteboard);
        main_box.append(&format_box);

        let selected_format = Rc::new(RefCell::new(DocumentFormat::Notebook));

        let sf_nb = selected_format.clone();
        let b_nb = btn_notebook.clone();
        let b_wb = btn_whiteboard.clone();
        btn_notebook.connect_clicked(move |_| {
            *sf_nb.borrow_mut() = DocumentFormat::Notebook;
            b_nb.add_css_class("selected");
            b_wb.remove_css_class("selected");
        });

        let sf_wb = selected_format.clone();
        let b_nb2 = btn_notebook.clone();
        let b_wb2 = btn_whiteboard.clone();
        btn_whiteboard.connect_clicked(move |_| {
            *sf_wb.borrow_mut() = DocumentFormat::Whiteboard;
            b_wb2.add_css_class("selected");
            b_nb2.remove_css_class("selected");
        });

        // Sfondo Pagina (valido per Quaderno A4)
        let lbl_bg = Label::builder()
            .label("Sfondo Pagina:")
            .xalign(0.0)
            .css_classes(["heading"])
            .build();
        main_box.append(&lbl_bg);

        let bg_options = [
            "Quadretti Matematici (5mm)",
            "Righe Universitarie (8mm)",
            "Bianco Naturale",
        ];
        let dropdown_bg = DropDown::from_strings(&bg_options);
        dropdown_bg.set_selected(0);
        dropdown_bg.add_css_class("dms-dropdown");
        main_box.append(&dropdown_bg);

        // Pulsanti azione
        let actions_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(12)
            .halign(Align::End)
            .margin_top(16)
            .build();

        let btn_cancel = Button::builder()
            .label("Annulla")
            .css_classes(["dms-action-btn"])
            .build();

        let btn_create = Button::builder()
            .label("Crea Documento")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .build();

        actions_box.append(&btn_cancel);
        actions_box.append(&btn_create);
        main_box.append(&actions_box);

        let win_cancel = window.clone();
        btn_cancel.connect_clicked(move |_| {
            win_cancel.close();
        });

        let win_create = window.clone();
        let callback = Rc::new(on_created);
        let folder_for_create = target_folder.clone();
        btn_create.connect_clicked(move |_| {
            let title = entry_name.text().to_string();
            let final_title = if title.trim().is_empty() {
                let (auto_name, _) = next_available_note_path_in_dir(&folder_for_create);
                auto_name
            } else {
                title.trim().to_string()
            };
            let format = *selected_format.borrow();
            let background = match dropdown_bg.selected() {
                1 => PageBackground::Ruled {
                    line_spacing: 22.68,
                    margin_left: 35.0,
                },
                2 => PageBackground::Blank,
                _ => PageBackground::Grid { grid_size: 14.17 },
            };

            win_create.close();
            callback(final_title, format, background);
        });

        window.set_child(Some(&main_box));
        window.present();
    }
}

pub type OpenDocCallback = Rc<RefCell<Option<Box<dyn Fn(PathBuf)>>>>;
pub type CreateDocCallback =
    Rc<RefCell<Option<Box<dyn Fn(String, DocumentFormat, PageBackground)>>>>;
pub type OpenSettingsCallback = Rc<RefCell<Option<Box<dyn Fn()>>>>;
pub type ImportPdfCallback = Rc<RefCell<Option<Box<dyn Fn(PathBuf)>>>>;

/// Componente Hub Documenti / Dashboard iniziale di UniNote.
pub struct UniNoteDashboard {
    container: GtkBox,
    flow_box: FlowBox,
    search_entry: SearchEntry,
    empty_box: GtkBox,
    folder_list_box: GtkBox,
    breadcrumbs_bar: GtkBox,
    btn_up: Button,
    store: Rc<RefCell<DashboardRecentStore>>,
    root_folder: PathBuf,
    current_folder: Rc<RefCell<PathBuf>>,
    on_open_document: OpenDocCallback,
    on_create_document: CreateDocCallback,
    on_open_settings: OpenSettingsCallback,
    on_import_pdf: ImportPdfCallback,
}

impl Default for UniNoteDashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl UniNoteDashboard {
    /// Inizializza una nuova Dashboard caricando l'archivio dei documenti recenti.
    pub fn new() -> Self {
        let store = Rc::new(RefCell::new(DashboardRecentStore::load()));
        let root = PathBuf::from(&store.borrow().default_notes_dir);
        Self::with_store_and_root(store, root)
    }

    /// Inizializza la Dashboard a partire da uno store condiviso.
    pub fn with_store(store: Rc<RefCell<DashboardRecentStore>>) -> Self {
        let root = PathBuf::from(&store.borrow().default_notes_dir);
        Self::with_store_and_root(store, root)
    }

    /// Inizializza la Dashboard a partire da uno store condiviso e da una cartella radice specifica.
    pub fn with_store_and_root(
        store: Rc<RefCell<DashboardRecentStore>>,
        root_folder: PathBuf,
    ) -> Self {
        if !root_folder.exists() {
            let _ = fs::create_dir_all(&root_folder);
        }

        let current_folder = Rc::new(RefCell::new(root_folder.clone()));

        // Contenitore principale orizzontale: Sidebar a sinistra + Area Contenuti a destra
        let main_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(0)
            .hexpand(true)
            .vexpand(true)
            .can_target(true)
            .css_classes(["dms-dashboard", "dms-dashboard-split"])
            .build();

        // -------------------------------------------------------------------------
        // 1. BARRA LATERALE (SIDEBAR ~260px)
        // -------------------------------------------------------------------------
        let sidebar_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .width_request(260)
            .margin_start(16)
            .margin_end(16)
            .margin_top(20)
            .margin_bottom(20)
            .css_classes(["dms-dashboard-sidebar"])
            .build();

        let sidebar_header_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();

        let lbl_sidebar_title = Label::builder()
            .label("Cartelle")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["title-3", "heading", "dms-sidebar-header"])
            .build();
        sidebar_header_box.append(&lbl_sidebar_title);
        sidebar_box.append(&sidebar_header_box);

        let btn_new_folder = Button::builder()
            .label("+ Nuova Cartella")
            .css_classes(["dms-btn-new-folder", "suggested-action", "dms-btn-primary"])
            .build();
        sidebar_box.append(&btn_new_folder);

        let folder_list_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .build();

        let folder_scroll = ScrolledWindow::builder()
            .child(&folder_list_box)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .vexpand(true)
            .build();
        sidebar_box.append(&folder_scroll);

        let btn_connect_cloud = Button::builder()
            .label("☁️ Gestisci Cloud...")
            .css_classes(["flat", "dms-btn-connect-cloud"])
            .margin_top(8)
            .margin_bottom(4)
            .build();
        sidebar_box.append(&btn_connect_cloud);

        main_box.append(&sidebar_box);

        // Separatore verticale DMS
        let vsep = GtkBox::builder()
            .css_classes(["dms-sidebar-separator"])
            .width_request(1)
            .build();
        main_box.append(&vsep);

        // -------------------------------------------------------------------------
        // 2. AREA CONTENUTI DESTRA
        // -------------------------------------------------------------------------
        let content_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(16)
            .hexpand(true)
            .vexpand(true)
            .margin_start(24)
            .margin_end(28)
            .margin_top(20)
            .margin_bottom(28)
            .build();

        // Top Bar
        let top_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(16)
            .valign(Align::Center)
            .build();

        let brand_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .valign(Align::Center)
            .css_classes(["dms-dashboard-brand"])
            .build();

        let logo_icon = create_icon(IconKind::AppLogo, 36, None);
        brand_box.append(&logo_icon);

        let brand_labels = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .valign(Align::Center)
            .build();

        let lbl_app_title = Label::builder()
            .label("UniNote")
            .xalign(0.0)
            .css_classes(["title-2", "brand-title"])
            .build();
        brand_labels.append(&lbl_app_title);

        let lbl_hub_subtitle = Label::builder()
            .label("Hub Documenti Accademico")
            .xalign(0.0)
            .css_classes(["dim-label"])
            .build();
        brand_labels.append(&lbl_hub_subtitle);
        brand_box.append(&brand_labels);
        top_bar.append(&brand_box);

        let spacer_left = GtkBox::builder().hexpand(true).build();
        top_bar.append(&spacer_left);

        let search_entry = SearchEntry::builder()
            .placeholder_text("Cerca quaderni e appunti...")
            .width_request(340)
            .css_classes(["dms-search-entry"])
            .build();
        top_bar.append(&search_entry);

        let spacer_right = GtkBox::builder().hexpand(true).build();
        top_bar.append(&spacer_right);

        let btn_new_doc = Button::builder()
            .label("+ Nuovo Documento")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_new_doc);

        let btn_import_pdf = Button::builder()
            .label("Nuovo da PDF...")
            .tooltip_text("Importa dispense o slide accademiche creando un quaderno")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_import_pdf);

        let btn_open_file = Button::builder()
            .label("Apri File...")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_open_file);

        let btn_settings = Button::builder()
            .child(&create_icon(IconKind::Settings, 18, None))
            .tooltip_text("Impostazioni UniNote")
            .css_classes(["dms-action-btn"])
            .valign(Align::Center)
            .build();
        top_bar.append(&btn_settings);

        content_box.append(&top_bar);

        // Barra dei Breadcrumbs con pulsante livello superiore
        let breadcrumbs_bar = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .css_classes(["dms-breadcrumbs-bar"])
            .build();

        let btn_up = Button::builder()
            .child(&create_icon(IconKind::ChevronLeft, 14, None))
            .tooltip_text("Livello superiore")
            .css_classes(["dms-btn-up", "dms-action-btn"])
            .valign(Align::Center)
            .build();

        content_box.append(&breadcrumbs_bar);

        // FlowBox per documenti e cartelle
        let flow_box = FlowBox::builder()
            .valign(Align::Start)
            .halign(Align::Start)
            .column_spacing(24)
            .row_spacing(24)
            .min_children_per_line(1)
            .max_children_per_line(6)
            .selection_mode(gtk4::SelectionMode::None)
            .homogeneous(false)
            .css_classes(["dms-dashboard-grid"])
            .build();

        let empty_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .halign(Align::Center)
            .valign(Align::Center)
            .vexpand(true)
            .margin_top(60)
            .margin_bottom(60)
            .build();

        let empty_title = Label::builder()
            .label("Nessun documento trovato")
            .css_classes(["title-2"])
            .build();
        let empty_desc = Label::builder()
            .label("Crea un nuovo quaderno A4 o una lavagna infinita per iniziare a scrivere.")
            .css_classes(["dim-label"])
            .build();

        empty_box.append(&empty_title);
        empty_box.append(&empty_desc);
        empty_box.set_visible(false);

        let grid_content_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .build();
        grid_content_box.append(&flow_box);
        grid_content_box.append(&empty_box);

        let content_scrolled = ScrolledWindow::builder()
            .child(&grid_content_box)
            .hscrollbar_policy(gtk4::PolicyType::Never)
            .vscrollbar_policy(gtk4::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        content_box.append(&content_scrolled);
        main_box.append(&content_box);

        let on_open_document = Rc::new(RefCell::new(None));
        let on_create_document = Rc::new(RefCell::new(None));
        let on_open_settings = Rc::new(RefCell::new(None));
        let on_import_pdf = Rc::new(RefCell::new(None));

        let dashboard = Self {
            container: main_box,
            flow_box,
            search_entry,
            empty_box,
            folder_list_box,
            breadcrumbs_bar,
            btn_up: btn_up.clone(),
            store,
            root_folder,
            current_folder,
            on_open_document,
            on_create_document,
            on_open_settings,
            on_import_pdf,
        };

        dashboard.wire_signals(
            btn_new_doc,
            btn_new_folder,
            btn_up,
            btn_import_pdf,
            btn_open_file,
            btn_settings,
            btn_connect_cloud,
        );
        dashboard.refresh();

        dashboard
    }

    /// Collega i segnali di ricerca, apertura file, importazione PDF, cartelle e creazione documento.
    #[allow(clippy::too_many_arguments)]
    fn wire_signals(
        &self,
        btn_new_doc: Button,
        btn_new_folder: Button,
        btn_up: Button,
        btn_import: Button,
        btn_open: Button,
        btn_settings: Button,
        btn_connect_cloud: Button,
    ) {
        let store_ref = self.store.clone();
        let flow_ref = self.flow_box.clone();
        let empty_ref = self.empty_box.clone();
        let open_cb = self.on_open_document.clone();
        let cur_folder = self.current_folder.clone();
        let root_folder = self.root_folder.clone();
        let folder_list_box = self.folder_list_box.clone();
        let breadcrumbs_bar = self.breadcrumbs_bar.clone();
        let btn_up_ref = btn_up.clone();
        let search_entry_for_search = self.search_entry.clone();

        let nav_cb = {
            let cur_f = cur_folder.clone();
            let fl = flow_ref.clone();
            let em = empty_ref.clone();
            let st = store_ref.clone();
            let se = search_entry_for_search.clone();
            let flb = folder_list_box.clone();
            let bcb = breadcrumbs_bar.clone();
            let b_up = btn_up_ref.clone();
            let root_f = root_folder.clone();
            let od = open_cb.clone();
            Rc::new(move |target: PathBuf| {
                let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
                let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();
                let is_valid = is_safe_subpath(&root_f, &target)
                    || target == root_f
                    || is_safe_subpath(&nc_dir, &target)
                    || target == nc_dir
                    || is_safe_subpath(&gd_dir, &target)
                    || target == gd_dir;
                if is_valid {
                    *cur_f.borrow_mut() = target;
                } else {
                    *cur_f.borrow_mut() = root_f.clone();
                }
                Self::refresh_all(
                    &st,
                    &cur_f,
                    &root_f,
                    &se.text(),
                    &flb,
                    &bcb,
                    &b_up,
                    &fl,
                    &em,
                    &od,
                );
            })
        };

        let refresh_cb = {
            let cur_f = cur_folder.clone();
            let fl = flow_ref;
            let em = empty_ref;
            let st = store_ref.clone();
            let se = search_entry_for_search;
            let flb = folder_list_box;
            let bcb = breadcrumbs_bar;
            let b_up = btn_up_ref;
            let root_f = root_folder.clone();
            let od = open_cb.clone();
            Rc::new(move || {
                Self::refresh_all(
                    &st,
                    &cur_f,
                    &root_f,
                    &se.text(),
                    &flb,
                    &bcb,
                    &b_up,
                    &fl,
                    &em,
                    &od,
                );
            })
        };

        // Ricerca istantanea
        let ref_cb_search = refresh_cb.clone();
        self.search_entry.connect_search_changed(move |_| {
            ref_cb_search();
        });

        // Pulsante Livello Superiore
        let cur_f_up = cur_folder.clone();
        let root_f_up = root_folder.clone();
        let nav_cb_up = nav_cb.clone();
        btn_up.connect_clicked(move |_| {
            let cur = cur_f_up.borrow().clone();
            let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
            let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();
            if cur == nc_dir || cur == gd_dir {
                nav_cb_up(root_f_up.clone());
            } else if cur != root_f_up {
                if let Some(parent) = cur.parent() {
                    if parent.starts_with(&root_f_up)
                        || parent == nc_dir
                        || is_safe_subpath(&nc_dir, parent)
                        || parent == gd_dir
                        || is_safe_subpath(&gd_dir, parent)
                    {
                        nav_cb_up(parent.to_path_buf());
                        return;
                    }
                }
                nav_cb_up(root_f_up.clone());
            }
        });

        // Pulsante Gestisci Cloud
        let btn_cloud_clone = btn_connect_cloud.clone();
        let ref_cb_cloud = refresh_cb.clone();
        btn_connect_cloud.connect_clicked(move |_| {
            let parent_win = btn_cloud_clone
                .root()
                .and_then(|r| r.downcast::<Window>().ok());
            let r_cb = ref_cb_cloud.clone();
            let dialog = crate::cloud::ConnectCloudDialog::new(parent_win.as_ref(), move || {
                r_cb();
            });
            dialog.present();
        });

        // Pulsante + Nuova Cartella
        let cur_f_mkdir = cur_folder.clone();
        let nav_cb_mkdir = nav_cb.clone();
        btn_new_folder.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let target_dir = cur_f_mkdir.borrow().clone();
            let nav = nav_cb_mkdir.clone();
            CreateFolderDialog::show(parent_win.as_ref(), target_dir, move |new_folder| {
                nav(new_folder);
            });
        });

        // Pulsante + Nuovo Documento
        let cur_f_doc = cur_folder;
        let create_cb = self.on_create_document.clone();
        btn_new_doc.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let target_dir = cur_f_doc.borrow().clone();
            let c_cb = create_cb.clone();
            CreateDocumentDialog::show_in_folder(
                parent_win.as_ref(),
                target_dir,
                move |title, format, bg| {
                    if let Some(ref cb) = *c_cb.borrow() {
                        cb(title, format, bg);
                    }
                },
            );
        });

        // Pulsante Nuovo da PDF...
        let import_cb = self.on_import_pdf.clone();
        let store_for_import = self.store.clone();
        let ref_import = refresh_cb.clone();
        btn_import.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let dialog = FileDialog::builder()
                .title("Seleziona File PDF da Importare")
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documenti PDF (*.pdf)"));
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let imp_cb = import_cb.clone();
            let st = store_for_import.clone();
            let do_ref = ref_import.clone();

            dialog.open(parent_win.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let path_clone = path.clone();
                        let title = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Documento PDF")
                            .to_string();

                        let meta = RecentDocumentMeta {
                            file_path: path.to_string_lossy().to_string(),
                            title,
                            format: DocumentFormat::Notebook,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: 1,
                            thumbnail_path: None,
                        };

                        st.borrow_mut().add_recent(meta);
                        let _ = st.borrow().save();
                        do_ref();

                        if let Some(ref cb) = *imp_cb.borrow() {
                            cb(path_clone);
                        }
                    }
                }
            });
        });

        // Pulsante Apri File...
        let open_doc_cb = self.on_open_document.clone();
        let store_for_open = self.store.clone();
        let ref_open = refresh_cb;
        btn_open.connect_clicked(move |btn| {
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let dialog = FileDialog::builder()
                .title("Apri Documento UniNote o PDF")
                .modal(true)
                .build();

            let filter = gtk4::FileFilter::new();
            filter.set_name(Some("Documenti UniNote e PDF (*.uninote, *.pdf)"));
            filter.add_pattern("*.uninote");
            filter.add_pattern("*.pdf");
            filter.add_mime_type("application/pdf");
            let filters = gio::ListStore::new::<gtk4::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));

            let od_cb = open_doc_cb.clone();
            let st = store_for_open.clone();
            let do_ref = ref_open.clone();
            dialog.open(parent_win.as_ref(), gio::Cancellable::NONE, move |res| {
                if let Ok(file) = res {
                    if let Some(path) = file.path() {
                        let path_clone = path.clone();
                        let title = path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("Documento")
                            .to_string();

                        let meta = RecentDocumentMeta {
                            file_path: path.to_string_lossy().to_string(),
                            title,
                            format: DocumentFormat::Notebook,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: 1,
                            thumbnail_path: None,
                        };

                        st.borrow_mut().add_recent(meta);
                        let _ = st.borrow().save();
                        do_ref();

                        if let Some(ref cb) = *od_cb.borrow() {
                            cb(path_clone);
                        }
                    }
                }
            });
        });

        // Pulsante Impostazioni
        let settings_cb = self.on_open_settings.clone();
        btn_settings.connect_clicked(move |_| {
            if let Some(ref cb) = *settings_cb.borrow() {
                cb();
            }
        });
    }

    /// Ricarica e sincronizza simultaneamente sidebar, breadcrumb e flowbox.
    #[allow(clippy::too_many_arguments)]
    fn refresh_all(
        store: &Rc<RefCell<DashboardRecentStore>>,
        current_folder: &Rc<RefCell<PathBuf>>,
        root_folder: &Path,
        query: &str,
        folder_list_box: &GtkBox,
        breadcrumbs_bar: &GtkBox,
        btn_up: &Button,
        flow_box: &FlowBox,
        empty_box: &GtkBox,
        open_cb: &OpenDocCallback,
    ) {
        store.borrow_mut().sanitize_existing_files();

        let cur_f_rc = current_folder.clone();
        let st_rc = store.clone();
        let fl_rc = flow_box.clone();
        let em_rc = empty_box.clone();
        let flb_rc = folder_list_box.clone();
        let bcb_rc = breadcrumbs_bar.clone();
        let b_up_rc = btn_up.clone();
        let od_rc = open_cb.clone();
        let root_f_rc = root_folder.to_path_buf();
        let q_string = query.to_string();

        let nav_cb: Rc<dyn Fn(PathBuf)> = {
            let cur_f = cur_f_rc.clone();
            let st = st_rc.clone();
            let fl = fl_rc.clone();
            let em = em_rc.clone();
            let flb = flb_rc.clone();
            let bcb = bcb_rc.clone();
            let b_up = b_up_rc.clone();
            let root_f = root_f_rc.clone();
            let od = od_rc.clone();
            let q = q_string.clone();
            Rc::new(move |target: PathBuf| {
                let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
                let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();
                let is_valid = is_safe_subpath(&root_f, &target)
                    || target == root_f
                    || is_safe_subpath(&nc_dir, &target)
                    || target == nc_dir
                    || is_safe_subpath(&gd_dir, &target)
                    || target == gd_dir;
                if is_valid {
                    *cur_f.borrow_mut() = target;
                } else {
                    *cur_f.borrow_mut() = root_f.clone();
                }
                Self::refresh_all(&st, &cur_f, &root_f, &q, &flb, &bcb, &b_up, &fl, &em, &od);
            })
        };

        let ref_cb: Rc<dyn Fn()> = {
            let cur_f = cur_f_rc;
            let st = st_rc;
            let fl = fl_rc;
            let em = em_rc;
            let flb = flb_rc;
            let bcb = bcb_rc;
            let b_up = b_up_rc;
            let root_f = root_f_rc;
            let od = od_rc;
            let q = q_string;
            Rc::new(move || {
                Self::refresh_all(&st, &cur_f, &root_f, &q, &flb, &bcb, &b_up, &fl, &em, &od);
            })
        };

        let cur_folder_borrow = current_folder.borrow();
        Self::populate_sidebar(
            folder_list_box,
            root_folder,
            &cur_folder_borrow,
            &nav_cb,
            store,
            &ref_cb,
        );

        Self::populate_breadcrumbs(
            breadcrumbs_bar,
            root_folder,
            &cur_folder_borrow,
            btn_up,
            &nav_cb,
        );

        Self::populate_flowbox(
            flow_box,
            empty_box,
            store,
            &cur_folder_borrow,
            root_folder,
            query,
            open_cb,
            &nav_cb,
            &ref_cb,
        );
    }

    /// Popola la barra laterale delle cartelle con "Tutti i Documenti" e le sottocartelle.
    fn populate_sidebar(
        folder_list_box: &GtkBox,
        root_folder: &Path,
        current_folder: &Path,
        nav_cb: &Rc<dyn Fn(PathBuf)>,
        store: &Rc<RefCell<DashboardRecentStore>>,
        refresh_cb: &Rc<dyn Fn()>,
    ) {
        while let Some(child) = folder_list_box.first_child() {
            folder_list_box.remove(&child);
        }

        let is_at_root = current_folder == root_folder;

        // 1. Voce Radice: "Tutti i Documenti"
        let root_item = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_start(8)
            .margin_end(8)
            .margin_top(2)
            .margin_bottom(2)
            .css_classes(if is_at_root {
                vec!["dms-sidebar-item", "active"]
            } else {
                vec!["dms-sidebar-item"]
            })
            .build();

        let root_icon = create_icon(
            if is_at_root {
                IconKind::FolderOpen
            } else {
                IconKind::Folder
            },
            16,
            None,
        );
        root_item.append(&root_icon);

        let root_label = Label::builder()
            .label("Tutti i Documenti")
            .xalign(0.0)
            .hexpand(true)
            .css_classes(["dms-sidebar-item-label"])
            .build();
        root_item.append(&root_label);

        let total_docs = store.borrow().recent_files.len();
        let root_badge = Label::builder()
            .label(format!("{}", total_docs))
            .css_classes(["dms-sidebar-item-badge"])
            .build();
        root_item.append(&root_badge);

        let gesture_root = GestureClick::new();
        let nav_root = nav_cb.clone();
        let rf = root_folder.to_path_buf();
        gesture_root.connect_pressed(move |_, _, _, _| {
            nav_root(rf.clone());
        });
        root_item.add_controller(gesture_root);
        attach_folder_drop_target(&root_item, root_folder.to_path_buf(), store, refresh_cb);
        folder_list_box.append(&root_item);

        // 2. Sottocartelle ricorsive
        let mut all_subfolders = Vec::new();
        Self::collect_subfolders_recursive(root_folder, 1, &mut all_subfolders);

        for (path, name, count, depth) in all_subfolders {
            let is_active = current_folder == path;
            let item_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(8)
                .margin_start(8 + (depth - 1) as i32 * 14)
                .margin_end(8)
                .margin_top(2)
                .margin_bottom(2)
                .css_classes(if is_active {
                    vec!["dms-sidebar-item", "active"]
                } else {
                    vec!["dms-sidebar-item"]
                })
                .build();

            let folder_icon = create_icon(
                if is_active {
                    IconKind::FolderOpen
                } else {
                    IconKind::Folder
                },
                16,
                None,
            );
            item_box.append(&folder_icon);

            let lbl = Label::builder()
                .label(&name)
                .xalign(0.0)
                .hexpand(true)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .css_classes(["dms-sidebar-item-label"])
                .build();
            item_box.append(&lbl);

            let badge = Label::builder()
                .label(format!("{}", count))
                .css_classes(["dms-sidebar-item-badge"])
                .build();
            item_box.append(&badge);

            let gesture = GestureClick::new();
            let nav = nav_cb.clone();
            let target_p = path.clone();
            gesture.connect_pressed(move |_, _, _, _| {
                nav(target_p.clone());
            });
            item_box.add_controller(gesture);
            attach_folder_drop_target(&item_box, path.clone(), store, refresh_cb);
            folder_list_box.append(&item_box);
        }

        // 3. Sezione Cloud
        let cloud_settings = crate::cloud::CloudSyncSettings::load();
        let has_cloud = cloud_settings.nextcloud.is_some() || cloud_settings.google_drive.is_some();

        if has_cloud {
            let cloud_hdr = Label::builder()
                .label("Servizi Cloud")
                .xalign(0.0)
                .margin_start(12)
                .margin_top(14)
                .margin_bottom(4)
                .css_classes(["dim-label", "caption", "dms-sidebar-section-header"])
                .build();
            folder_list_box.append(&cloud_hdr);

            if let Some(ref nc) = cloud_settings.nextcloud {
                let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
                let is_active = current_folder == nc_dir;
                let item_box = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(8)
                    .margin_start(8)
                    .margin_end(8)
                    .margin_top(2)
                    .margin_bottom(2)
                    .css_classes(if is_active {
                        vec!["dms-sidebar-item", "active"]
                    } else {
                        vec!["dms-sidebar-item"]
                    })
                    .build();

                let icon = create_icon(IconKind::Cloud, 16, None);
                item_box.append(&icon);

                let lbl = Label::builder()
                    .label(format!("Nextcloud ({})", nc.username))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .css_classes(["dms-sidebar-item-label"])
                    .build();
                item_box.append(&lbl);

                let count = if let Ok((_, docs)) = scan_directory(&nc_dir) {
                    docs.len()
                } else {
                    0
                };
                let badge = Label::builder()
                    .label(format!("{}", count))
                    .css_classes(["dms-sidebar-item-badge"])
                    .build();
                item_box.append(&badge);

                let gesture = GestureClick::new();
                let nav = nav_cb.clone();
                let target_p = nc_dir.clone();
                gesture.connect_pressed(move |_, _, _, _| {
                    nav(target_p.clone());
                });
                item_box.add_controller(gesture);
                attach_folder_drop_target(&item_box, nc_dir.clone(), store, refresh_cb);
                folder_list_box.append(&item_box);

                let mut nc_subfolders = Vec::new();
                Self::collect_subfolders_recursive(&nc_dir, 2, &mut nc_subfolders);
                for (path, name, count, depth) in nc_subfolders {
                    let is_sub_active = current_folder == path;
                    let sub_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(8)
                        .margin_start(8 + (depth - 1) as i32 * 14)
                        .margin_end(8)
                        .margin_top(2)
                        .margin_bottom(2)
                        .css_classes(if is_sub_active {
                            vec!["dms-sidebar-item", "active"]
                        } else {
                            vec!["dms-sidebar-item"]
                        })
                        .build();

                    sub_box.append(&create_icon(
                        if is_sub_active {
                            IconKind::FolderOpen
                        } else {
                            IconKind::Folder
                        },
                        16,
                        None,
                    ));

                    let sub_lbl = Label::builder()
                        .label(&name)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk4::pango::EllipsizeMode::End)
                        .css_classes(["dms-sidebar-item-label"])
                        .build();
                    sub_box.append(&sub_lbl);

                    let sub_badge = Label::builder()
                        .label(format!("{}", count))
                        .css_classes(["dms-sidebar-item-badge"])
                        .build();
                    sub_box.append(&sub_badge);

                    let g = GestureClick::new();
                    let n = nav_cb.clone();
                    let tp = path.clone();
                    g.connect_pressed(move |_, _, _, _| {
                        n(tp.clone());
                    });
                    sub_box.add_controller(g);
                    attach_folder_drop_target(&sub_box, path.clone(), store, refresh_cb);
                    folder_list_box.append(&sub_box);
                }
            }

            if let Some(ref gd) = cloud_settings.google_drive {
                let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();
                let is_active = current_folder == gd_dir;
                let item_box = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(8)
                    .margin_start(8)
                    .margin_end(8)
                    .margin_top(2)
                    .margin_bottom(2)
                    .css_classes(if is_active {
                        vec!["dms-sidebar-item", "active"]
                    } else {
                        vec!["dms-sidebar-item"]
                    })
                    .build();

                let icon = create_icon(IconKind::Cloud, 16, None);
                item_box.append(&icon);

                let lbl = Label::builder()
                    .label(format!("Google Drive ({})", gd.account_email))
                    .xalign(0.0)
                    .hexpand(true)
                    .ellipsize(gtk4::pango::EllipsizeMode::End)
                    .css_classes(["dms-sidebar-item-label"])
                    .build();
                item_box.append(&lbl);

                let count = if let Ok((_, docs)) = scan_directory(&gd_dir) {
                    docs.len()
                } else {
                    0
                };
                let badge = Label::builder()
                    .label(format!("{}", count))
                    .css_classes(["dms-sidebar-item-badge"])
                    .build();
                item_box.append(&badge);

                let gesture = GestureClick::new();
                let nav = nav_cb.clone();
                let target_p = gd_dir.clone();
                gesture.connect_pressed(move |_, _, _, _| {
                    nav(target_p.clone());
                });
                item_box.add_controller(gesture);
                attach_folder_drop_target(&item_box, gd_dir.clone(), store, refresh_cb);
                folder_list_box.append(&item_box);

                let mut gd_subfolders = Vec::new();
                Self::collect_subfolders_recursive(&gd_dir, 2, &mut gd_subfolders);
                for (path, name, count, depth) in gd_subfolders {
                    let is_sub_active = current_folder == path;
                    let sub_box = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(8)
                        .margin_start(8 + (depth - 1) as i32 * 14)
                        .margin_end(8)
                        .margin_top(2)
                        .margin_bottom(2)
                        .css_classes(if is_sub_active {
                            vec!["dms-sidebar-item", "active"]
                        } else {
                            vec!["dms-sidebar-item"]
                        })
                        .build();

                    sub_box.append(&create_icon(
                        if is_sub_active {
                            IconKind::FolderOpen
                        } else {
                            IconKind::Folder
                        },
                        16,
                        None,
                    ));

                    let sub_lbl = Label::builder()
                        .label(&name)
                        .xalign(0.0)
                        .hexpand(true)
                        .ellipsize(gtk4::pango::EllipsizeMode::End)
                        .css_classes(["dms-sidebar-item-label"])
                        .build();
                    sub_box.append(&sub_lbl);

                    let sub_badge = Label::builder()
                        .label(format!("{}", count))
                        .css_classes(["dms-sidebar-item-badge"])
                        .build();
                    sub_box.append(&sub_badge);

                    let g = GestureClick::new();
                    let n = nav_cb.clone();
                    let tp = path.clone();
                    g.connect_pressed(move |_, _, _, _| {
                        n(tp.clone());
                    });
                    sub_box.add_controller(g);
                    attach_folder_drop_target(&sub_box, path.clone(), store, refresh_cb);
                    folder_list_box.append(&sub_box);
                }
            }
        }
    }

    fn collect_subfolders_recursive(
        dir: &Path,
        depth: usize,
        out: &mut Vec<(PathBuf, String, usize, usize)>,
    ) {
        if let Ok((subfolders, _)) = scan_directory(dir) {
            for sub in subfolders {
                out.push((sub.path.clone(), sub.name, sub.item_count, depth));
                Self::collect_subfolders_recursive(&sub.path, depth + 1, out);
            }
        }
    }

    /// Popola la barra dei breadcrumb mostrando i segmenti cliccabili del percorso attivo.
    fn populate_breadcrumbs(
        breadcrumbs_bar: &GtkBox,
        root_folder: &Path,
        current_folder: &Path,
        btn_up: &Button,
        nav_cb: &Rc<dyn Fn(PathBuf)>,
    ) {
        while let Some(child) = breadcrumbs_bar.first_child() {
            breadcrumbs_bar.remove(&child);
        }

        let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
        let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();

        let is_at_root = current_folder == root_folder;
        let is_at_nc_root = current_folder == nc_dir;
        let is_at_gd_root = current_folder == gd_dir;

        let in_nc = current_folder == nc_dir || is_safe_subpath(&nc_dir, current_folder);
        let in_gd = current_folder == gd_dir || is_safe_subpath(&gd_dir, current_folder);

        let is_at_any_root = is_at_root || is_at_nc_root || is_at_gd_root;

        if !is_at_any_root {
            btn_up.set_visible(true);
            btn_up.set_sensitive(true);
            breadcrumbs_bar.append(btn_up);
        } else {
            btn_up.set_visible(false);
            btn_up.set_sensitive(false);
        }

        if in_nc {
            let btn_root = Button::builder()
                .label("☁️ Nextcloud")
                .css_classes(if is_at_nc_root {
                    vec!["dms-breadcrumb-btn", "active"]
                } else {
                    vec!["dms-breadcrumb-btn"]
                })
                .build();
            let nav_r = nav_cb.clone();
            let ncf = nc_dir.clone();
            btn_root.connect_clicked(move |_| {
                nav_r(ncf.clone());
            });
            breadcrumbs_bar.append(&btn_root);

            if !is_at_nc_root {
                if let Ok(rel) = current_folder.strip_prefix(&nc_dir) {
                    let mut accumulated = nc_dir;
                    let components: Vec<_> = rel.components().collect();
                    let num_components = components.len();

                    for (idx, comp) in components.iter().enumerate() {
                        let sep = Label::builder()
                            .label("/")
                            .css_classes(["dms-breadcrumb-sep"])
                            .build();
                        breadcrumbs_bar.append(&sep);

                        accumulated = accumulated.join(comp);
                        let is_last = idx == num_components - 1;
                        let comp_name = comp.as_os_str().to_string_lossy().to_string();

                        let btn_comp = Button::builder()
                            .label(&comp_name)
                            .css_classes(if is_last {
                                vec!["dms-breadcrumb-btn", "active"]
                            } else {
                                vec!["dms-breadcrumb-btn"]
                            })
                            .build();

                        let nav_c = nav_cb.clone();
                        let target_p = accumulated.clone();
                        btn_comp.connect_clicked(move |_| {
                            nav_c(target_p.clone());
                        });
                        breadcrumbs_bar.append(&btn_comp);
                    }
                }
            }
        } else if in_gd {
            let btn_root = Button::builder()
                .label("☁️ Google Drive")
                .css_classes(if is_at_gd_root {
                    vec!["dms-breadcrumb-btn", "active"]
                } else {
                    vec!["dms-breadcrumb-btn"]
                })
                .build();
            let nav_r = nav_cb.clone();
            let gdf = gd_dir.clone();
            btn_root.connect_clicked(move |_| {
                nav_r(gdf.clone());
            });
            breadcrumbs_bar.append(&btn_root);

            if !is_at_gd_root {
                if let Ok(rel) = current_folder.strip_prefix(&gd_dir) {
                    let mut accumulated = gd_dir;
                    let components: Vec<_> = rel.components().collect();
                    let num_components = components.len();

                    for (idx, comp) in components.iter().enumerate() {
                        let sep = Label::builder()
                            .label("/")
                            .css_classes(["dms-breadcrumb-sep"])
                            .build();
                        breadcrumbs_bar.append(&sep);

                        accumulated = accumulated.join(comp);
                        let is_last = idx == num_components - 1;
                        let comp_name = comp.as_os_str().to_string_lossy().to_string();

                        let btn_comp = Button::builder()
                            .label(&comp_name)
                            .css_classes(if is_last {
                                vec!["dms-breadcrumb-btn", "active"]
                            } else {
                                vec!["dms-breadcrumb-btn"]
                            })
                            .build();

                        let nav_c = nav_cb.clone();
                        let target_p = accumulated.clone();
                        btn_comp.connect_clicked(move |_| {
                            nav_c(target_p.clone());
                        });
                        breadcrumbs_bar.append(&btn_comp);
                    }
                }
            }
        } else {
            let btn_root = Button::builder()
                .label("UniNote")
                .css_classes(if is_at_root {
                    vec!["dms-breadcrumb-btn", "active"]
                } else {
                    vec!["dms-breadcrumb-btn"]
                })
                .build();
            let nav_r = nav_cb.clone();
            let rf = root_folder.to_path_buf();
            btn_root.connect_clicked(move |_| {
                nav_r(rf.clone());
            });
            breadcrumbs_bar.append(&btn_root);

            if !is_at_root {
                if let Ok(rel) = current_folder.strip_prefix(root_folder) {
                    let mut accumulated = root_folder.to_path_buf();
                    let components: Vec<_> = rel.components().collect();
                    let num_components = components.len();

                    for (idx, comp) in components.iter().enumerate() {
                        let sep = Label::builder()
                            .label("/")
                            .css_classes(["dms-breadcrumb-sep"])
                            .build();
                        breadcrumbs_bar.append(&sep);

                        accumulated = accumulated.join(comp);
                        let is_last = idx == num_components - 1;
                        let comp_name = comp.as_os_str().to_string_lossy().to_string();

                        let btn_comp = Button::builder()
                            .label(&comp_name)
                            .css_classes(if is_last {
                                vec!["dms-breadcrumb-btn", "active"]
                            } else {
                                vec!["dms-breadcrumb-btn"]
                            })
                            .build();

                        let nav_c = nav_cb.clone();
                        let target_p = accumulated.clone();
                        btn_comp.connect_clicked(move |_| {
                            nav_c(target_p.clone());
                        });
                        breadcrumbs_bar.append(&btn_comp);
                    }
                }
            }
        }
    }

    /// Popola la FlowBox con card cartella e card documenti secondo la cartella attiva o query di ricerca.
    #[allow(clippy::too_many_arguments)]
    fn populate_flowbox(
        flow_box: &FlowBox,
        empty_box: &GtkBox,
        store: &Rc<RefCell<DashboardRecentStore>>,
        current_folder: &Path,
        root_folder: &Path,
        query: &str,
        open_cb: &OpenDocCallback,
        nav_cb: &Rc<dyn Fn(PathBuf)>,
        refresh_cb: &Rc<dyn Fn()>,
    ) {
        while let Some(child) = flow_box.first_child() {
            flow_box.remove(&child);
        }

        let is_searching = !query.trim().is_empty();
        let (subfolders, files_on_disk) = scan_directory(current_folder).unwrap_or_default();
        let mut displayed_count = 0;

        if is_searching {
            let q = query.trim().to_lowercase();
            // Mostra sottocartelle corrispondenti
            for folder in subfolders {
                if folder.name.to_lowercase().contains(&q) {
                    let card = Self::create_folder_card(
                        &folder,
                        nav_cb.clone(),
                        refresh_cb.clone(),
                        store,
                    );
                    flow_box.append(&card);
                    displayed_count += 1;
                }
            }
            // Mostra documenti corrispondenti dai recenti
            let filtered_recents = store.borrow().filter_by_query(&q);
            for meta in filtered_recents {
                let mut resolved_meta = meta;
                let p = Path::new(&resolved_meta.file_path);
                if p.exists() {
                    if let Ok(doc_store) = uninote_storage::DocumentStore::open_or_create(p) {
                        if let Ok(doc) = doc_store.load_document() {
                            resolved_meta.format = doc.format;
                            resolved_meta.page_count = doc.pages.len();
                        }
                    }
                }
                let card = Self::create_card_with_refresh(
                    &resolved_meta,
                    open_cb,
                    store,
                    refresh_cb.clone(),
                );
                flow_box.append(&card);
                displayed_count += 1;
            }
        } else {
            // Vista normale della cartella corrente
            // 1. Mostra card per ogni sottocartella diretta
            for folder in subfolders {
                let card =
                    Self::create_folder_card(&folder, nav_cb.clone(), refresh_cb.clone(), store);
                flow_box.append(&card);
                displayed_count += 1;
            }

            // 2. Mostra card per ogni documento in current_folder
            let mut seen_paths = std::collections::HashSet::new();

            // File fisici su disco nella cartella corrente
            for file_path in files_on_disk {
                let path_str = file_path.to_string_lossy().to_string();
                seen_paths.insert(path_str.clone());

                let mut meta = store
                    .borrow()
                    .recent_files
                    .iter()
                    .find(|r| r.file_path == path_str)
                    .cloned()
                    .unwrap_or_else(|| {
                        let title = file_path
                            .file_stem()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "Nuova nota".to_string());
                        RecentDocumentMeta {
                            file_path: path_str,
                            title,
                            format: DocumentFormat::Notebook,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: 1,
                            thumbnail_path: None,
                        }
                    });

                // Se il file esiste su disco, sincronizziamo formato reale e page_count
                if let Ok(doc_store) = uninote_storage::DocumentStore::open_or_create(&file_path) {
                    if let Ok(doc) = doc_store.load_document() {
                        meta.format = doc.format;
                        meta.page_count = doc.pages.len();
                    }
                }

                let card =
                    Self::create_card_with_refresh(&meta, open_cb, store, refresh_cb.clone());
                flow_box.append(&card);
                displayed_count += 1;
            }

            // Se ci troviamo alla radice, includiamo eventuali file recenti esterni (non memorizzati dentro root_folder o sue sottocartelle)
            if current_folder == root_folder {
                for meta in &store.borrow().recent_files {
                    let p = Path::new(&meta.file_path);
                    if p.exists()
                        && !p.starts_with(root_folder)
                        && !seen_paths.contains(&meta.file_path)
                    {
                        seen_paths.insert(meta.file_path.clone());
                        let mut resolved_meta = meta.clone();
                        if let Ok(doc_store) = uninote_storage::DocumentStore::open_or_create(p) {
                            if let Ok(doc) = doc_store.load_document() {
                                resolved_meta.format = doc.format;
                                resolved_meta.page_count = doc.pages.len();
                            }
                        }
                        let card = Self::create_card_with_refresh(
                            &resolved_meta,
                            open_cb,
                            store,
                            refresh_cb.clone(),
                        );
                        flow_box.append(&card);
                        displayed_count += 1;
                    }
                }
            }
        }

        if displayed_count == 0 {
            empty_box.set_visible(true);
            flow_box.set_visible(false);
        } else {
            empty_box.set_visible(false);
            flow_box.set_visible(true);
        }
    }

    /// Restituisce l'etichetta testuale del badge DMS per il formato documento specificato.
    pub fn format_badge_label(format: DocumentFormat) -> &'static str {
        format_badge_label(format)
    }

    /// Mostra una finestra di dialogo modale di conferma per azioni distruttive o conferme DMS.
    pub fn show_confirmation_dialog<F>(
        parent: Option<&Window>,
        title: &str,
        message: &str,
        confirm_label: &str,
        is_destructive: bool,
        on_confirm: F,
    ) where
        F: FnOnce() + 'static,
    {
        show_confirmation_dialog(
            parent,
            title,
            message,
            confirm_label,
            is_destructive,
            on_confirm,
        )
    }

    /// Crea il widget grafico di una card per una cartella nella `GtkFlowBox`.
    pub fn create_folder_card(
        folder: &FolderEntry,
        nav_cb: Rc<dyn Fn(PathBuf)>,
        refresh_cb: Rc<dyn Fn()>,
        store: &Rc<RefCell<DashboardRecentStore>>,
    ) -> FlowBoxChild {
        let child = FlowBoxChild::new();
        child.add_css_class("dms-flowbox-child");

        let card_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .width_request(240)
            .css_classes(["dms-dashboard-card", "dms-folder-card"])
            .can_target(true)
            .build();

        // Area grafica / Icona Cartella a tutta larghezza ed altezza uniforme 260px
        let art_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .hexpand(true)
            .halign(Align::Fill)
            .valign(Align::Fill)
            .height_request(260)
            .css_classes(["dms-folder-art"])
            .build();

        let icon_folder = create_icon(IconKind::Folder, 72, None);
        icon_folder.set_halign(Align::Center);
        icon_folder.set_valign(Align::Center);
        icon_folder.set_vexpand(true);
        icon_folder.set_hexpand(true);
        art_box.append(&icon_folder);
        card_box.append(&art_box);

        // Dettagli cartella (titolo + menu contestuale 3 puntini)
        let info_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_start(10)
            .margin_end(10)
            .margin_bottom(10)
            .build();

        let title_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .valign(Align::Center)
            .build();

        let lbl_title = Label::builder()
            .label(&folder.name)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["dms-folder-title", "heading"])
            .build();
        title_row.append(&lbl_title);

        // Menu contestuale 3 puntini per la cartella
        let btn_menu = Button::builder()
            .child(&create_icon(IconKind::MoreVertical, 14, None))
            .tooltip_text("Opzioni cartella...")
            .css_classes(["dms-card-menu-btn"])
            .valign(Align::Center)
            .halign(Align::End)
            .build();

        let popover = gtk4::Popover::builder()
            .css_classes(["dms-popover", "dms-context-popover"])
            .build();

        let menu_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let btn_rename = Button::builder()
            .label("Rinomina")
            .css_classes(["dms-popover-item"])
            .halign(Align::Fill)
            .build();

        let btn_delete = Button::builder()
            .label("Elimina")
            .css_classes(["dms-popover-item", "destructive-action"])
            .halign(Align::Fill)
            .build();

        menu_box.append(&btn_rename);
        menu_box.append(&btn_delete);
        popover.set_child(Some(&menu_box));
        popover.set_parent(&btn_menu);

        let pop_clone = popover.clone();
        btn_menu.connect_clicked(move |_| {
            pop_clone.popup();
        });

        // Azione Rinomina Cartella
        let pop_for_ren = popover.clone();
        let folder_for_ren = folder.clone();
        let store_for_ren = store.clone();
        let ref_for_ren = refresh_cb.clone();
        btn_rename.connect_clicked(move |btn| {
            pop_for_ren.popdown();
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());

            let dlg = Window::builder()
                .title("Rinomina Cartella — UniNote")
                .modal(true)
                .default_width(380)
                .default_height(160)
                .css_classes(["uninote-dialog", "dms-dialog"])
                .build();

            if let Some(ref p) = parent_win {
                dlg.set_transient_for(Some(p));
            }

            let dlg_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(12)
                .margin_start(20)
                .margin_end(20)
                .margin_top(20)
                .margin_bottom(20)
                .build();

            let lbl = Label::builder()
                .label("Nuovo nome della cartella:")
                .xalign(0.0)
                .css_classes(["heading"])
                .build();
            dlg_box.append(&lbl);

            let entry = Entry::builder()
                .text(&folder_for_ren.name)
                .css_classes(["dms-entry"])
                .build();
            dlg_box.append(&entry);

            let lbl_err = Label::builder()
                .label("")
                .xalign(0.0)
                .css_classes(["dms-dialog-error"])
                .visible(false)
                .build();
            dlg_box.append(&lbl_err);

            let act_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .halign(Align::End)
                .build();

            let btn_cancel = Button::builder()
                .label("Annulla")
                .css_classes(["dms-action-btn"])
                .build();

            let btn_ok = Button::builder()
                .label("Salva")
                .css_classes(["suggested-action", "dms-btn-primary"])
                .build();

            act_box.append(&btn_cancel);
            act_box.append(&btn_ok);
            dlg_box.append(&act_box);
            dlg.set_child(Some(&dlg_box));

            let dlg_cancel = dlg.clone();
            btn_cancel.connect_clicked(move |_| {
                dlg_cancel.close();
            });

            let dlg_ok = dlg.clone();
            let f_old = folder_for_ren.clone();
            let st = store_for_ren.clone();
            let ref_cb = ref_for_ren.clone();
            let err_lbl = lbl_err.clone();
            let ent = entry.clone();

            let do_rename = move || {
                let raw_text = ent.text().to_string();
                match sanitize_folder_name(&raw_text) {
                    Err(e) => {
                        err_lbl.set_label(e);
                        err_lbl.set_visible(true);
                    }
                    Ok(clean_name) => {
                        if clean_name == f_old.name {
                            dlg_ok.close();
                            return;
                        }
                        if let Some(parent) = f_old.path.parent() {
                            let new_path = parent.join(&clean_name);
                            if new_path.exists() {
                                err_lbl.set_label("Esiste già una cartella con questo nome");
                                err_lbl.set_visible(true);
                                return;
                            }
                            if let Err(e) = std::fs::rename(&f_old.path, &new_path) {
                                err_lbl.set_label(&format!("Errore rinomina: {}", e));
                                err_lbl.set_visible(true);
                                return;
                            }
                            st.borrow_mut().update_folder_rename(&f_old.path, &new_path);
                            let _ = st.borrow().save();
                            dlg_ok.close();
                            ref_cb();
                        }
                    }
                }
            };

            let do_ren_btn = do_rename.clone();
            btn_ok.connect_clicked(move |_| {
                do_ren_btn();
            });
            entry.connect_activate(move |_| {
                do_rename();
            });

            dlg.present();
        });

        // Azione Elimina Cartella
        let pop_for_del = popover.clone();
        let folder_for_del = folder.clone();
        let store_for_del = store.clone();
        let ref_for_del = refresh_cb.clone();
        btn_delete.connect_clicked(move |btn| {
            pop_for_del.popdown();
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let f_del = folder_for_del.clone();
            let st_del = store_for_del.clone();
            let ref_cb = ref_for_del.clone();
            let msg = format!(
                "Sei sicuro di voler eliminare la cartella \"{}\" e tutti i file contenuti?",
                f_del.name
            );

            show_confirmation_dialog(
                parent_win.as_ref(),
                "Elimina Cartella — UniNote",
                &msg,
                "Elimina",
                true,
                move || {
                    let _ = std::fs::remove_dir_all(&f_del.path);
                    st_del.borrow_mut().remove_recents_under(&f_del.path);
                    let _ = st_del.borrow().save();
                    ref_cb();
                },
            );
        });

        title_row.append(&btn_menu);
        info_box.append(&title_row);

        // Badge conteggio elementi nella cartella
        let badge_text = if folder.item_count == 1 {
            "1 elemento".to_string()
        } else {
            format!("{} elementi", folder.item_count)
        };
        let lbl_badge = Label::builder()
            .label(&badge_text)
            .xalign(0.0)
            .css_classes(["dms-folder-badge", "dim-label"])
            .build();
        info_box.append(&lbl_badge);

        card_box.append(&info_box);

        // Cliccando sulla card (esclusi controlli interni come btn_menu) si naviga nella cartella!
        let gesture = GestureClick::new();
        let target_path = folder.path.clone();
        let nav = nav_cb.clone();
        gesture.connect_pressed(move |_, _, _, _| {
            nav(target_path.clone());
        });
        art_box.add_controller(gesture);

        // Aggiungiamo anche il gesture su lbl_title
        let gesture_lbl = GestureClick::new();
        let target_path_lbl = folder.path.clone();
        let nav_lbl = nav_cb;
        gesture_lbl.connect_pressed(move |_, _, _, _| {
            nav_lbl(target_path_lbl.clone());
        });
        lbl_title.add_controller(gesture_lbl);

        // Collega DropTarget per ricevere note trascinate col cursore
        attach_folder_drop_target(&card_box, folder.path.clone(), store, &refresh_cb);

        child.set_child(Some(&card_box));
        child
    }

    /// Mostra il dialogo Adwaita per spostare un documento tra cartelle e sottocartelle.
    pub fn open_move_dialog(
        meta: &RecentDocumentMeta,
        store: &Rc<RefCell<DashboardRecentStore>>,
        refresh_cb: Rc<dyn Fn()>,
        parent_win: Option<&Window>,
    ) {
        let src_path = PathBuf::from(&meta.file_path);
        let current_parent = src_path.parent().map(|p| p.to_path_buf());
        let root_dir = PathBuf::from(&store.borrow().default_notes_dir);

        let dlg = Window::builder()
            .title("Sposta Documento — UniNote")
            .modal(true)
            .default_width(460)
            .default_height(480)
            .css_classes(["uninote-dialog", "dms-dialog"])
            .build();

        if let Some(p) = parent_win {
            dlg.set_transient_for(Some(p));
        }

        let main_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(12)
            .margin_start(20)
            .margin_end(20)
            .margin_top(20)
            .margin_bottom(20)
            .build();

        let lbl_title = Label::builder()
            .label(format!("Sposta \"{}\"", meta.title))
            .css_classes(["heading"])
            .xalign(0.0)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .build();
        main_box.append(&lbl_title);

        let cur_dir_name = if current_parent.as_ref() == Some(&root_dir) {
            "Tutti i Documenti (Cartella principale)".to_string()
        } else {
            current_parent
                .as_ref()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("Sconosciuta")
                .to_string()
        };

        let lbl_cur = Label::builder()
            .label(format!("Posizione attuale: {}", cur_dir_name))
            .css_classes(["dim-label"])
            .xalign(0.0)
            .build();
        main_box.append(&lbl_cur);

        let lbl_select = Label::builder()
            .label("Seleziona cartella di destinazione:")
            .css_classes(["caption"])
            .xalign(0.0)
            .margin_top(4)
            .build();
        main_box.append(&lbl_select);

        // Raccoglie tutte le destinazioni possibili
        let mut dests: Vec<(PathBuf, String, usize, bool)> = Vec::new();

        // 1. Cartella Principale
        let is_root_curr = current_parent.as_ref() == Some(&root_dir);
        dests.push((
            root_dir.clone(),
            "Tutti i Documenti (Cartella principale)".to_string(),
            0,
            is_root_curr,
        ));

        // 2. Sottocartelle ricorsive
        let mut subs = Vec::new();
        Self::collect_subfolders_recursive(&root_dir, 1, &mut subs);
        for (p, name, _, depth) in subs {
            let is_curr = current_parent.as_ref() == Some(&p);
            dests.push((p, name, depth, is_curr));
        }

        // 3. Nextcloud (se configurato)
        let cloud_settings = crate::cloud::CloudSyncSettings::load();
        if let Some(ref nc) = cloud_settings.nextcloud {
            let nc_dir = crate::cloud::CloudSyncSettings::nextcloud_local_dir();
            if nc_dir.exists() {
                let is_curr = current_parent.as_ref() == Some(&nc_dir);
                dests.push((
                    nc_dir.clone(),
                    format!("Nextcloud ({})", nc.username),
                    0,
                    is_curr,
                ));
                let mut nc_subs = Vec::new();
                Self::collect_subfolders_recursive(&nc_dir, 1, &mut nc_subs);
                for (p, name, _, depth) in nc_subs {
                    let is_curr = current_parent.as_ref() == Some(&p);
                    dests.push((p, name, depth, is_curr));
                }
            }
        }

        // 4. Google Drive (se configurato)
        if let Some(ref gd) = cloud_settings.google_drive {
            let gd_dir = crate::cloud::CloudSyncSettings::gdrive_local_dir();
            if gd_dir.exists() {
                let is_curr = current_parent.as_ref() == Some(&gd_dir);
                dests.push((
                    gd_dir.clone(),
                    format!("Google Drive ({})", gd.account_email),
                    0,
                    is_curr,
                ));
                let mut gd_subs = Vec::new();
                Self::collect_subfolders_recursive(&gd_dir, 1, &mut gd_subs);
                for (p, name, _, depth) in gd_subs {
                    let is_curr = current_parent.as_ref() == Some(&p);
                    dests.push((p, name, depth, is_curr));
                }
            }
        }

        let list_box = ListBox::builder()
            .selection_mode(gtk4::SelectionMode::Single)
            .css_classes(["dms-folder-picker-list"])
            .build();

        let dest_paths: Rc<RefCell<Vec<PathBuf>>> =
            Rc::new(RefCell::new(dests.iter().map(|d| d.0.clone()).collect()));
        let dest_is_curr: Rc<RefCell<Vec<bool>>> =
            Rc::new(RefCell::new(dests.iter().map(|d| d.3).collect()));

        for (_p, name, depth, is_curr) in &dests {
            let row = ListBoxRow::new();
            row.add_css_class("dms-folder-picker-row");
            let hbox = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .margin_start(8 + (*depth as i32 * 14))
                .margin_end(8)
                .margin_top(6)
                .margin_bottom(6)
                .build();

            let icon = create_icon(
                if *is_curr {
                    IconKind::FolderOpen
                } else {
                    IconKind::Folder
                },
                16,
                None,
            );
            hbox.append(&icon);

            let lbl = Label::builder()
                .label(name)
                .hexpand(true)
                .xalign(0.0)
                .ellipsize(gtk4::pango::EllipsizeMode::End)
                .build();
            hbox.append(&lbl);

            if *is_curr {
                let badge = Label::builder()
                    .label("(attuale)")
                    .css_classes(["dim-label", "dms-folder-badge"])
                    .build();
                hbox.append(&badge);
            }

            row.set_child(Some(&hbox));
            list_box.append(&row);
        }

        let scroll = ScrolledWindow::builder()
            .child(&list_box)
            .vexpand(true)
            .min_content_height(200)
            .build();
        main_box.append(&scroll);

        // Barra per creazione al volo di una nuova cartella
        let new_folder_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .margin_top(4)
            .build();
        let new_folder_entry = Entry::builder()
            .placeholder_text("Nome nuova sottocartella...")
            .hexpand(true)
            .css_classes(["dms-entry"])
            .build();
        let btn_add_folder = Button::builder()
            .label("+ Nuova Cartella")
            .css_classes(["dms-action-btn"])
            .build();
        new_folder_box.append(&new_folder_entry);
        new_folder_box.append(&btn_add_folder);
        main_box.append(&new_folder_box);

        // Pulsanti inferiori
        let act_box = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(10)
            .halign(Align::End)
            .margin_top(8)
            .build();

        let btn_cancel = Button::builder()
            .label("Annulla")
            .css_classes(["dms-action-btn"])
            .build();

        let btn_move = Button::builder()
            .label("Sposta qui")
            .css_classes(["suggested-action", "dms-btn-primary"])
            .sensitive(false)
            .build();

        act_box.append(&btn_cancel);
        act_box.append(&btn_move);
        main_box.append(&act_box);

        dlg.set_child(Some(&main_box));

        let selected_target: Rc<RefCell<Option<PathBuf>>> = Rc::new(RefCell::new(None));

        let btn_move_ref = btn_move.clone();
        let dest_paths_ref = dest_paths.clone();
        let dest_is_curr_ref = dest_is_curr.clone();
        let sel_target_ref = selected_target.clone();

        list_box.connect_row_selected(move |_, row_opt| {
            if let Some(row) = row_opt {
                let idx = row.index() as usize;
                let paths = dest_paths_ref.borrow();
                let is_currs = dest_is_curr_ref.borrow();
                if idx < paths.len() {
                    let is_current = is_currs.get(idx).copied().unwrap_or(false);
                    *sel_target_ref.borrow_mut() = Some(paths[idx].clone());
                    btn_move_ref.set_sensitive(!is_current);
                } else {
                    btn_move_ref.set_sensitive(false);
                }
            } else {
                btn_move_ref.set_sensitive(false);
            }
        });

        let dlg_for_cancel = dlg.clone();
        btn_cancel.connect_clicked(move |_| {
            dlg_for_cancel.close();
        });

        // Esecuzione dello spostamento
        let dlg_for_move = dlg.clone();
        let meta_for_exec = meta.clone();
        let store_for_exec = store.clone();
        let ref_for_exec = refresh_cb;
        let sel_for_exec = selected_target.clone();

        let execute_move = move || {
            let target_opt = sel_for_exec.borrow().clone();
            if let Some(target_dir) = target_opt {
                let src = PathBuf::from(&meta_for_exec.file_path);
                if src.exists() && src.parent() != Some(&target_dir) {
                    if let Ok(new_path) = move_document_to_folder(&src, &target_dir) {
                        let new_title = new_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&meta_for_exec.title);
                        store_for_exec.borrow_mut().move_or_rename_recent(
                            &meta_for_exec.file_path,
                            &new_path.to_string_lossy(),
                            new_title,
                        );
                        let _ = store_for_exec.borrow().save();
                        ref_for_exec();
                    }
                }
            }
            dlg_for_move.close();
        };

        let exec_clone = execute_move.clone();
        btn_move.connect_clicked(move |_| {
            exec_clone();
        });

        let exec_double = execute_move;
        let dest_is_curr_double = dest_is_curr.clone();
        list_box.connect_row_activated(move |_, row| {
            let idx = row.index() as usize;
            let is_currs = dest_is_curr_double.borrow();
            if idx < is_currs.len() && !is_currs[idx] {
                exec_double();
            }
        });

        // Gestione creazione nuova cartella
        let list_box_for_add = list_box;
        let entry_for_add = new_folder_entry;
        let sel_target_for_add = selected_target;
        let root_for_add = root_dir;
        let dest_paths_for_add = dest_paths;
        let dest_is_curr_for_add = dest_is_curr;
        let btn_move_for_add = btn_move;

        let entry_input = entry_for_add.clone();
        let add_folder_action = move || {
            let raw_name = entry_input.text().trim().to_string();
            if let Ok(clean_name) = sanitize_folder_name(&raw_name) {
                let base_dir = sel_target_for_add
                    .borrow()
                    .clone()
                    .unwrap_or_else(|| root_for_add.clone());
                let new_dir = base_dir.join(&clean_name);
                if std::fs::create_dir_all(&new_dir).is_ok() {
                    let row = ListBoxRow::new();
                    row.add_css_class("dms-folder-picker-row");
                    let hbox = GtkBox::builder()
                        .orientation(Orientation::Horizontal)
                        .spacing(10)
                        .margin_start(16)
                        .margin_end(8)
                        .margin_top(6)
                        .margin_bottom(6)
                        .build();
                    let icon = create_icon(IconKind::Folder, 16, None);
                    hbox.append(&icon);
                    let lbl = Label::builder()
                        .label(&clean_name)
                        .hexpand(true)
                        .xalign(0.0)
                        .build();
                    hbox.append(&lbl);
                    let badge = Label::builder()
                        .label("(nuova)")
                        .css_classes(["dim-label", "dms-folder-badge"])
                        .build();
                    hbox.append(&badge);
                    row.set_child(Some(&hbox));

                    dest_paths_for_add.borrow_mut().push(new_dir.clone());
                    dest_is_curr_for_add.borrow_mut().push(false);
                    list_box_for_add.append(&row);
                    list_box_for_add.select_row(Some(&row));
                    *sel_target_for_add.borrow_mut() = Some(new_dir);
                    btn_move_for_add.set_sensitive(true);
                    entry_input.set_text("");
                }
            }
        };

        let add_btn_clone = add_folder_action.clone();
        btn_add_folder.connect_clicked(move |_| {
            add_btn_clone();
        });
        entry_for_add.connect_activate(move |_| {
            add_folder_action();
        });

        dlg.present();
    }

    /// Crea il widget grafico di una singola card quaderno conforme a DMS.
    pub fn create_card(meta: &RecentDocumentMeta, open_cb: &OpenDocCallback) -> FlowBoxChild {
        let store = Rc::new(RefCell::new(DashboardRecentStore::new()));
        Self::create_card_with_refresh(meta, open_cb, &store, Rc::new(|| {}))
    }

    /// Crea il widget grafico di una singola card quaderno conforme a DMS con callback di refresh completo.
    pub fn create_card_with_refresh(
        meta: &RecentDocumentMeta,
        open_cb: &OpenDocCallback,
        store: &Rc<RefCell<DashboardRecentStore>>,
        refresh_cb: Rc<dyn Fn()>,
    ) -> FlowBoxChild {
        let child = FlowBoxChild::new();
        child.add_css_class("dms-flowbox-child");

        let card_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(8)
            .width_request(240)
            .css_classes(["dms-dashboard-card"])
            .can_target(true)
            .build();

        // 1. Miniatura con altezza uniforme 260px (ContentFit::Cover scala la texture 240x340 impeccabilmente)
        let thumb_path = get_or_create_thumbnail_path(meta);
        let picture = Picture::for_filename(&thumb_path);
        picture.set_size_request(240, 260);
        picture.set_content_fit(gtk4::ContentFit::Cover);
        picture.add_css_class("dms-card-thumbnail");
        card_box.append(&picture);

        // Flag atomico per distinguere il click dall'avvio di un'operazione di trascinamento (drag)
        let is_dragging = Rc::new(std::cell::Cell::new(false));

        // DragSource per consentire di trascinare il documento verso una cartella
        let drag_src = DragSource::new();
        drag_src.set_actions(gdk::DragAction::MOVE);
        let fpath_drag = meta.file_path.clone();
        drag_src.connect_prepare(move |_, _, _| {
            Some(gdk::ContentProvider::for_value(&fpath_drag.to_value()))
        });

        let is_drag_start = is_dragging.clone();
        let pic_clone = picture.clone();
        let drag_src_clone = drag_src.clone();
        drag_src.connect_drag_begin(move |_, _| {
            is_drag_start.set(true);
            if let Some(paintable) = pic_clone.paintable() {
                drag_src_clone.set_icon(Some(&paintable), 40, 40);
            }
        });

        let is_drag_end = is_dragging.clone();
        drag_src.connect_drag_end(move |_, _, _| {
            let is_drag_reset = is_drag_end.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                is_drag_reset.set(false);
            });
        });

        let is_drag_cancel = is_dragging.clone();
        drag_src.connect_drag_cancel(move |_, _, _| {
            let is_drag_reset = is_drag_cancel.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(200), move || {
                is_drag_reset.set(false);
            });
            false
        });
        card_box.add_controller(drag_src);

        // 2. Dettagli testuali con riga titolo + menu 3 puntini
        let info_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_start(10)
            .margin_end(10)
            .margin_bottom(10)
            .build();

        let title_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(6)
            .valign(Align::Center)
            .build();

        let lbl_title = Label::builder()
            .label(&meta.title)
            .xalign(0.0)
            .hexpand(true)
            .ellipsize(gtk4::pango::EllipsizeMode::End)
            .css_classes(["dms-card-title", "heading"])
            .build();
        title_row.append(&lbl_title);

        // Pulsante Menu 3 puntini
        let btn_menu = Button::builder()
            .child(&create_icon(IconKind::MoreVertical, 14, None))
            .tooltip_text("Altre opzioni...")
            .css_classes(["dms-card-menu-btn"])
            .valign(Align::Center)
            .halign(Align::End)
            .build();

        let popover = gtk4::Popover::builder()
            .css_classes(["dms-popover", "dms-context-popover"])
            .build();

        let menu_box = GtkBox::builder()
            .orientation(Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        let make_menu_item =
            |icon_kind: IconKind, label_text: &str, is_destructive: bool| -> Button {
                let row = GtkBox::builder()
                    .orientation(Orientation::Horizontal)
                    .spacing(10)
                    .valign(Align::Center)
                    .build();

                let icon_color = if is_destructive {
                    Some(gtk4::gdk::RGBA::new(1.0, 0.42, 0.42, 1.0))
                } else {
                    None
                };
                let icon = create_icon(icon_kind, 16, icon_color);
                let lbl = Label::builder()
                    .label(label_text)
                    .xalign(0.0)
                    .hexpand(true)
                    .build();

                row.append(&icon);
                row.append(&lbl);

                let mut classes = vec!["dms-menu-item-btn"];
                if is_destructive {
                    classes.push("destructive-action");
                }

                Button::builder().child(&row).css_classes(classes).build()
            };

        let btn_rename = make_menu_item(IconKind::Edit, "Rinomina", false);
        let btn_duplicate = make_menu_item(IconKind::Copy, "Duplica", false);
        let btn_move = make_menu_item(IconKind::Folder, "Sposta in...", false);
        let btn_delete = make_menu_item(IconKind::Trash, "Elimina", true);

        menu_box.append(&btn_rename);
        menu_box.append(&btn_duplicate);
        menu_box.append(&btn_move);

        let sep = gtk4::Separator::new(Orientation::Horizontal);
        sep.add_css_class("dms-menu-separator");
        menu_box.append(&sep);

        menu_box.append(&btn_delete);
        popover.set_child(Some(&menu_box));
        popover.set_parent(&btn_menu);

        let pop_ref = popover.clone();
        btn_menu.connect_clicked(move |_| {
            pop_ref.popup();
        });

        // Azione Rinomina
        let pop_for_rename = popover.clone();
        let meta_for_rename = meta.clone();
        let store_for_rename = store.clone();
        let ref_rename = refresh_cb.clone();
        btn_rename.connect_clicked(move |btn| {
            pop_for_rename.popdown();
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());

            let dlg = Window::builder()
                .title("Rinomina Documento — UniNote")
                .modal(true)
                .default_width(420)
                .default_height(180)
                .css_classes(["uninote-dialog", "dms-dialog"])
                .build();

            if let Some(ref p) = parent_win {
                dlg.set_transient_for(Some(p));
            }

            let dlg_box = GtkBox::builder()
                .orientation(Orientation::Vertical)
                .spacing(14)
                .margin_start(20)
                .margin_end(20)
                .margin_top(20)
                .margin_bottom(20)
                .build();

            let lbl = Label::builder()
                .label("Nuovo nome del file:")
                .xalign(0.0)
                .css_classes(["heading"])
                .build();
            dlg_box.append(&lbl);

            let entry = Entry::builder()
                .text(&meta_for_rename.title)
                .css_classes(["dms-entry"])
                .build();
            dlg_box.append(&entry);

            let act_box = GtkBox::builder()
                .orientation(Orientation::Horizontal)
                .spacing(10)
                .halign(Align::End)
                .build();

            let btn_cancel = Button::builder()
                .label("Annulla")
                .css_classes(["dms-action-btn"])
                .build();

            let btn_ok = Button::builder()
                .label("Salva")
                .css_classes(["suggested-action", "dms-btn-primary"])
                .build();

            act_box.append(&btn_cancel);
            act_box.append(&btn_ok);
            dlg_box.append(&act_box);
            dlg.set_child(Some(&dlg_box));

            let dlg_cancel = dlg.clone();
            btn_cancel.connect_clicked(move |_| {
                dlg_cancel.close();
            });

            let dlg_ok = dlg.clone();
            let m_ren = meta_for_rename.clone();
            let st_ren = store_for_rename.clone();
            let ref_cb = ref_rename.clone();
            btn_ok.connect_clicked(move |_| {
                let new_title = entry.text().trim().to_string();
                if !new_title.is_empty() && new_title != m_ren.title {
                    let old_path = PathBuf::from(&m_ren.file_path);
                    if let Some(parent) = old_path.parent() {
                        let ext = old_path
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or("uninote");
                        let new_path = parent.join(format!("{}.{}", new_title, ext));
                        if std::fs::rename(&old_path, &new_path).is_ok() {
                            st_ren.borrow_mut().rename_recent(
                                &m_ren.file_path,
                                &new_path.to_string_lossy(),
                                &new_title,
                            );
                            let _ = st_ren.borrow().save();
                            ref_cb();
                        }
                    }
                }
                dlg_ok.close();
            });

            dlg.present();
        });

        // Azione Duplica
        let pop_for_dup = popover.clone();
        let meta_for_dup = meta.clone();
        let store_for_dup = store.clone();
        let ref_dup = refresh_cb.clone();
        btn_duplicate.connect_clicked(move |_| {
            pop_for_dup.popdown();
            let old_path = PathBuf::from(&meta_for_dup.file_path);
            if old_path.exists() {
                if let Some(parent) = old_path.parent() {
                    let ext = old_path
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("uninote");
                    let mut copy_idx = 1;
                    let mut new_title = format!("{} (Copia)", meta_for_dup.title);
                    let mut new_path = parent.join(format!("{}.{}", new_title, ext));
                    while new_path.exists() {
                        copy_idx += 1;
                        new_title = format!("{} (Copia {})", meta_for_dup.title, copy_idx);
                        new_path = parent.join(format!("{}.{}", new_title, ext));
                    }

                    if std::fs::copy(&old_path, &new_path).is_ok() {
                        let new_meta = RecentDocumentMeta {
                            file_path: new_path.to_string_lossy().to_string(),
                            title: new_title,
                            format: meta_for_dup.format,
                            last_modified_timestamp: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .map(|d| d.as_secs())
                                .unwrap_or(0),
                            page_count: meta_for_dup.page_count,
                            thumbnail_path: None,
                        };
                        store_for_dup.borrow_mut().add_recent(new_meta);
                        let _ = store_for_dup.borrow().save();
                        ref_dup();
                    }
                }
            }
        });

        // Azione Sposta in...
        let pop_for_move = popover.clone();
        let meta_for_move = meta.clone();
        let store_for_move = store.clone();
        let ref_move = refresh_cb.clone();
        btn_move.connect_clicked(move |btn| {
            pop_for_move.popdown();
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            Self::open_move_dialog(
                &meta_for_move,
                &store_for_move,
                ref_move.clone(),
                parent_win.as_ref(),
            );
        });

        // Azione Elimina
        let pop_for_del = popover.clone();
        let meta_for_del = meta.clone();
        let store_for_del = store.clone();
        let ref_del = refresh_cb;
        btn_delete.connect_clicked(move |btn| {
            pop_for_del.popdown();
            let parent_win = btn.root().and_then(|r| r.downcast::<Window>().ok());
            let m_del = meta_for_del.clone();
            let st_del = store_for_del.clone();
            let ref_cb = ref_del.clone();
            let msg = format!(
                "Vuoi davvero eliminare definitivamente \"{}\"?",
                m_del.title
            );

            show_confirmation_dialog(
                parent_win.as_ref(),
                "Elimina Documento — UniNote",
                &msg,
                "Elimina",
                true,
                move || {
                    let p = PathBuf::from(&m_del.file_path);
                    let _ = std::fs::remove_file(&p);
                    st_del.borrow_mut().remove_recent(&m_del.file_path);
                    let _ = st_del.borrow().save();
                    ref_cb();
                },
            );
        });

        title_row.append(&btn_menu);
        info_box.append(&title_row);

        let meta_row = GtkBox::builder()
            .orientation(Orientation::Horizontal)
            .spacing(8)
            .valign(Align::Center)
            .build();

        let badge_text = Self::format_badge_label(meta.format);

        let badge_class = match meta.format {
            DocumentFormat::Notebook => "badge-notebook",
            DocumentFormat::Whiteboard => "badge-whiteboard",
        };

        let lbl_badge = Label::builder()
            .label(badge_text)
            .css_classes(["dms-format-badge", badge_class])
            .build();
        meta_row.append(&lbl_badge);

        let pages_text = match meta.format {
            DocumentFormat::Notebook => format!("{} pag.", meta.page_count),
            DocumentFormat::Whiteboard => "Infinita".to_string(),
        };

        let lbl_pages = Label::builder()
            .label(&pages_text)
            .css_classes(["dim-label"])
            .build();
        meta_row.append(&lbl_pages);

        info_box.append(&meta_row);
        card_box.append(&info_box);

        // Interattività al click per aprire il documento (sulla Picture e sul titolo, evitando btn_menu).
        // Si usa connect_released controllando che non sia avvenuto un trascinamento (drag) per non
        // aprire il documento mentre l'utente intende spostare la card con il drag and drop.
        let gesture = GestureClick::new();
        let target_path = PathBuf::from(&meta.file_path);
        let cb_clone = open_cb.clone();
        let pop_picture = popover.clone();
        gesture.connect_pressed(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_SECONDARY {
                pop_picture.popup();
            }
        });
        let is_drag_rel = is_dragging.clone();
        gesture.connect_released(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_PRIMARY && !is_drag_rel.get() {
                if let Some(ref cb) = *cb_clone.borrow() {
                    cb(target_path.clone());
                }
            }
        });
        picture.add_controller(gesture);

        let gesture_lbl = GestureClick::new();
        let target_path_lbl = PathBuf::from(&meta.file_path);
        let cb_lbl = open_cb.clone();
        let is_drag_lbl = is_dragging;
        let pop_lbl = popover.clone();
        gesture_lbl.connect_pressed(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_SECONDARY {
                pop_lbl.popup();
            }
        });
        gesture_lbl.connect_released(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_PRIMARY && !is_drag_lbl.get() {
                if let Some(ref cb) = *cb_lbl.borrow() {
                    cb(target_path_lbl.clone());
                }
            }
        });
        lbl_title.add_controller(gesture_lbl);

        let gesture_info = GestureClick::new();
        let pop_info = popover.clone();
        gesture_info.connect_pressed(move |g, _, _, _| {
            if g.current_button() == gdk::BUTTON_SECONDARY {
                pop_info.popup();
            }
        });
        info_box.add_controller(gesture_info);

        child.set_child(Some(&card_box));
        child
    }

    /// Restituisce il widget GTK radice della Dashboard per l'inclusione nella gerarchia Adwaita.
    pub fn widget(&self) -> &gtk4::Widget {
        self.container.upcast_ref()
    }

    /// Restituisce il widget FlowBox interno per testing e ispezione dei figli.
    pub fn flow_box(&self) -> &FlowBox {
        &self.flow_box
    }

    /// Connette il callback invocato quando l'utente seleziona un documento da aprire.
    pub fn connect_open_document<F>(&self, callback: F)
    where
        F: Fn(PathBuf) + 'static,
    {
        *self.on_open_document.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente crea un nuovo documento.
    pub fn connect_create_document<F>(&self, callback: F)
    where
        F: Fn(String, DocumentFormat, PageBackground) + 'static,
    {
        *self.on_create_document.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente clicca sul pulsante impostazioni.
    pub fn connect_open_settings<F>(&self, callback: F)
    where
        F: Fn() + 'static,
    {
        *self.on_open_settings.borrow_mut() = Some(Box::new(callback));
    }

    /// Connette il callback invocato quando l'utente importa un file PDF.
    pub fn connect_import_pdf<F>(&self, callback: F)
    where
        F: Fn(PathBuf) + 'static,
    {
        *self.on_import_pdf.borrow_mut() = Some(Box::new(callback));
    }

    /// Restituisce il riferimento condiviso allo store dei documenti recenti.
    pub fn store(&self) -> Rc<RefCell<DashboardRecentStore>> {
        self.store.clone()
    }

    /// Restituisce il percorso della cartella attualmente visualizzata.
    pub fn current_folder(&self) -> PathBuf {
        self.current_folder.borrow().clone()
    }

    /// Restituisce il percorso della cartella radice di UniNote.
    pub fn root_folder(&self) -> PathBuf {
        self.root_folder.clone()
    }

    /// Indica se la dashboard si trova attualmente nella cartella radice.
    pub fn is_at_root(&self) -> bool {
        *self.current_folder.borrow() == self.root_folder
    }

    /// Naviga verso una cartella di destinazione all'interno di UniNote.
    pub fn navigate_to(&self, folder: PathBuf) {
        if is_safe_subpath(&self.root_folder, &folder) || folder == self.root_folder {
            *self.current_folder.borrow_mut() = folder;
        } else {
            *self.current_folder.borrow_mut() = self.root_folder.clone();
        }
        self.refresh();
    }

    /// Torna alla cartella genitore (o alla radice).
    pub fn navigate_up(&self) {
        let cur = self.current_folder.borrow().clone();
        if cur != self.root_folder {
            if let Some(parent) = cur.parent() {
                if parent.starts_with(&self.root_folder) {
                    *self.current_folder.borrow_mut() = parent.to_path_buf();
                    self.refresh();
                    return;
                }
            }
            *self.current_folder.borrow_mut() = self.root_folder.clone();
            self.refresh();
        }
    }

    /// Ricostruisce l'intera interfaccia (sidebar, breadcrumb e flowbox) per lo stato corrente.
    pub fn refresh(&self) {
        let query = self.search_entry.text().to_string();
        Self::refresh_all(
            &self.store,
            &self.current_folder,
            &self.root_folder,
            &query,
            &self.folder_list_box,
            &self.breadcrumbs_bar,
            &self.btn_up,
            &self.flow_box,
            &self.empty_box,
            &self.on_open_document,
        );
    }
}
