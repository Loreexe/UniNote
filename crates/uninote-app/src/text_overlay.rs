//! Overlay e Popover di editing per Formule LaTeX di Fondo e Caselle di Testo.
//!
//! Fornisce popover contestuali ancorati al punto di tocco/clic con anteprima
//! LaTeX istantanea compilata da `latex-rust` e editor Markdown in stile Obsidian.

use crate::canvas::CanvasState;
use crate::icons::{create_icon, IconKind};
use crate::math_renderer::render_latex_to_surface;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, DrawingArea, Entry, Orientation, Popover, ScrolledWindow,
    TextView,
};
use std::cell::RefCell;
use std::rc::Rc;
use uninote_core::command::DocCommand;
use uninote_core::coordinates::Viewport;
use uninote_core::model::{DocumentFormat, TextItem, TextLayer};
use uninote_stroke::Rect;

/// Mostra il popover rapido di inserimento o modifica per Formule LaTeX di Fondo.
pub fn show_math_underlay_popover(
    area: &DrawingArea,
    state: Rc<RefCell<CanvasState>>,
    screen_pos: (f64, f64),
    doc_pos: (f64, f64),
    existing_item: Option<(u64, TextItem)>,
) {
    let popover = Popover::new();
    popover.set_parent(area);
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.add_css_class("uninote-math-popover");

    let pointing_rect = gdk::Rectangle::new(screen_pos.0 as i32, screen_pos.1 as i32, 1, 1);
    popover.set_pointing_to(Some(&pointing_rect));

    let main_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .width_request(340)
        .build();

    // Intestazione con icona
    let header_box = GtkBox::new(Orientation::Horizontal, 8);
    let icon = create_icon(IconKind::Math, 18, None);
    let title_label = gtk4::Label::builder()
        .label(if existing_item.is_some() {
            "Modifica Formula LaTeX di Fondo"
        } else {
            "Nuova Formula LaTeX di Fondo"
        })
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["title-4"])
        .build();
    header_box.append(&icon);
    header_box.append(&title_label);
    main_box.append(&header_box);

    // Campo di input LaTeX
    let entry = Entry::builder()
        .placeholder_text(r"Es. \int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}")
        .hexpand(true)
        .css_classes(["dms-entry"])
        .build();

    if let Some((_, ref item)) = existing_item {
        entry.set_text(&item.raw_text);
    }
    main_box.append(&entry);

    // Barra dei simboli rapidi (chips)
    let chips_box = GtkBox::new(Orientation::Horizontal, 6);
    let chips = [
        (r"\frac{}{}", r"\frac{a}{b}"),
        (r"\sqrt{}", r"\sqrt{x}"),
        (r"\int", r"\int_{0}^{1}"),
        (r"\sum", r"\sum_{i=1}^{n}"),
        (r"\alpha", r"\alpha"),
        (r"\to", r"\to"),
        (r"\pm", r"\pm"),
    ];

    for (label, snippet) in chips {
        let chip_btn = Button::builder()
            .label(label)
            .css_classes(["dms-chip-btn"])
            .build();
        let entry_clone = entry.clone();
        let snippet_str = snippet.to_string();
        chip_btn.connect_clicked(move |_| {
            let cur = entry_clone.text();
            let pos = entry_clone.position();
            let mut new_text = String::with_capacity(cur.len() + snippet_str.len());
            let byte_pos = cur
                .char_indices()
                .nth(pos as usize)
                .map(|(idx, _)| idx)
                .unwrap_or(cur.len());
            new_text.push_str(&cur[..byte_pos]);
            new_text.push_str(&snippet_str);
            new_text.push_str(&cur[byte_pos..]);
            entry_clone.set_text(&new_text);
            entry_clone.set_position((pos as usize + snippet_str.len()) as i32);
        });
        chips_box.append(&chip_btn);
    }
    main_box.append(&chips_box);

    // Area di anteprima live formula
    let preview_area = DrawingArea::builder()
        .height_request(54)
        .hexpand(true)
        .css_classes(["math-preview-area"])
        .build();

    let cur_latex = Rc::new(RefCell::new(entry.text().to_string()));
    let preview_cur_latex = cur_latex.clone();

    preview_area.set_draw_func(move |_area, cr, width, height| {
        let is_dark = libadwaita::StyleManager::default().is_dark();
        // Sfondo dell'anteprima
        if is_dark {
            cr.set_source_rgba(0.12, 0.16, 0.18, 1.0);
        } else {
            cr.set_source_rgba(0.94, 0.96, 0.98, 1.0);
        }
        let _ = cr.paint();

        let text = preview_cur_latex.borrow().clone();
        let clean = text.trim();
        if clean.is_empty() {
            return;
        }

        if let Some(surf) = render_latex_to_surface(clean) {
            let tex_w = surf.width() as f64;
            let tex_h = surf.height() as f64;
            if tex_w > 0.0 && tex_h > 0.0 {
                let max_w = (width as f64 - 16.0).max(10.0);
                let max_h = (height as f64 - 12.0).max(10.0);
                let scale = (max_w / tex_w).min(max_h / tex_h).min(1.5);
                let target_w = tex_w * scale;
                let target_h = tex_h * scale;

                let draw_x = (width as f64 - target_w) * 0.5;
                let draw_y = (height as f64 - target_h) * 0.5;

                let _ = cr.save();
                cr.translate(draw_x, draw_y);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(&surf, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            }
        } else {
            // Sintassi non valida o incompleta
            cr.set_source_rgba(0.9, 0.3, 0.3, 0.85);
            cr.move_to(12.0, (height as f64) * 0.5 + 4.0);
            let layout = pangocairo::functions::create_layout(cr);
            layout.set_text("Formula non valida");
            pangocairo::functions::show_layout(cr, &layout);
        }
    });

    let prev_clone = preview_area.clone();
    let cur_latex_notify = cur_latex.clone();
    entry.connect_text_notify(move |ent| {
        *cur_latex_notify.borrow_mut() = ent.text().to_string();
        prev_clone.queue_draw();
    });
    main_box.append(&preview_area);

    // Barra azioni: Conferma ed Annulla
    let actions_box = GtkBox::new(Orientation::Horizontal, 8);
    actions_box.set_halign(Align::End);

    let cancel_btn = Button::builder()
        .label("Annulla")
        .css_classes(["dms-btn-flat"])
        .build();
    let pop_cancel = popover.clone();
    cancel_btn.connect_clicked(move |_| {
        pop_cancel.popdown();
    });

    let confirm_btn = Button::builder()
        .label("Conferma Formula")
        .css_classes(["suggested-action", "dms-pill-btn"])
        .build();

    let pop_confirm = popover.clone();
    let area_clone = area.clone();
    let state_confirm = state.clone();
    let entry_confirm = entry.clone();
    let existing_confirm = existing_item.clone();

    let do_confirm = move || {
        let latex = entry_confirm.text().trim().to_string();
        if latex.is_empty() {
            pop_confirm.popdown();
            return;
        }

        // Calcola dimensioni naturali della formula compilata
        let (nat_w, nat_h) = if let Some(surf) = render_latex_to_surface(&latex) {
            (
                (surf.width() as f64).max(60.0),
                (surf.height() as f64).max(30.0),
            )
        } else {
            (160.0, 48.0)
        };

        let mut st_guard = state_confirm.borrow_mut();
        let st = &mut *st_guard;
        if st.document.pages.is_empty() {
            pop_confirm.popdown();
            return;
        }

        let (target_page_idx, target_page_id) = match st.document.format {
            DocumentFormat::Whiteboard => (0, st.document.pages[0].id),
            DocumentFormat::Notebook => {
                let p_idx = Viewport::find_page_index_at_y(doc_pos.1, st.document.pages.len())
                    .min(st.document.pages.len().saturating_sub(1));
                (p_idx, st.document.pages[p_idx].id)
            }
        };
        let _ = target_page_idx;

        if let Some((item_id, prev_item)) = existing_confirm.clone() {
            let new_bbox = Rect::new(
                prev_item.bbox.min_x,
                prev_item.bbox.min_y,
                prev_item.bbox.min_x + nat_w,
                prev_item.bbox.min_y + nat_h,
            );
            let cmd = DocCommand::EditText {
                page_id: target_page_id,
                item_id,
                old_text: prev_item.raw_text,
                new_text: latex,
                old_bbox: prev_item.bbox,
                new_bbox,
            };
            st.history.execute(cmd, &mut st.document);
        } else {
            let item_id = st.alloc_item_id();
            let bbox = Rect::new(doc_pos.0, doc_pos.1, doc_pos.0 + nat_w, doc_pos.1 + nat_h);
            let text_item = TextItem::new(target_page_id, latex, bbox, TextLayer::Underlay);
            let cmd = DocCommand::add_text(target_page_id, item_id, 0, text_item);
            st.history.execute(cmd, &mut st.document);
        }

        drop(st_guard);
        area_clone.queue_draw();
        pop_confirm.popdown();
    };

    let do_confirm_rc = Rc::new(do_confirm);
    let do_c1 = do_confirm_rc.clone();
    confirm_btn.connect_clicked(move |_| {
        do_c1();
    });

    let do_c2 = do_confirm_rc.clone();
    entry.connect_activate(move |_| {
        do_c2();
    });

    actions_box.append(&cancel_btn);
    actions_box.append(&confirm_btn);
    main_box.append(&actions_box);

    popover.set_child(Some(&main_box));
    popover.popup();
    entry.grab_focus();
}

/// Mostra l'editor inline per Caselle di Testo tipografiche (Stile Obsidian).
pub fn show_text_box_popover(
    area: &DrawingArea,
    state: Rc<RefCell<CanvasState>>,
    screen_pos: (f64, f64),
    doc_pos: (f64, f64),
    existing_item: Option<(u64, TextItem)>,
) {
    let popover = Popover::new();
    popover.set_parent(area);
    popover.set_has_arrow(true);
    popover.set_autohide(true);
    popover.add_css_class("uninote-text-popover");

    let pointing_rect = gdk::Rectangle::new(screen_pos.0 as i32, screen_pos.1 as i32, 1, 1);
    popover.set_pointing_to(Some(&pointing_rect));

    let main_box = GtkBox::builder()
        .orientation(Orientation::Vertical)
        .spacing(10)
        .margin_start(14)
        .margin_end(14)
        .margin_top(14)
        .margin_bottom(14)
        .width_request(420)
        .build();

    // Intestazione con icona
    let header_box = GtkBox::new(Orientation::Horizontal, 8);
    let icon = create_icon(IconKind::Type, 18, None);
    let title_label = gtk4::Label::builder()
        .label(if existing_item.is_some() {
            "Modifica Casella di Testo"
        } else {
            "Nuova Casella di Testo (Markdown & LaTeX)"
        })
        .halign(Align::Start)
        .hexpand(true)
        .css_classes(["title-4"])
        .build();
    header_box.append(&icon);
    header_box.append(&title_label);
    main_box.append(&header_box);

    // Barra rapida Markdown / LaTeX
    let toolbar_box = GtkBox::new(Orientation::Horizontal, 6);
    let text_view = TextView::builder()
        .wrap_mode(gtk4::WrapMode::WordChar)
        .left_margin(8)
        .right_margin(8)
        .top_margin(8)
        .bottom_margin(8)
        .css_classes(["dms-text-editor-view"])
        .build();

    if let Some((_, ref item)) = existing_item {
        text_view.buffer().set_text(&item.raw_text);
    }

    let chips = [
        ("# Titolo", "# "),
        ("## Sottotitolo", "## "),
        ("Grassetto", "**testo**"),
        ("Corsivo", "*testo*"),
        ("Elenco", "- "),
        ("$$ Blocco Math", "$$\n\\int_{0}^{1} x dx\n$$"),
        ("$ Inline $", "$E = mc^2$"),
    ];

    for (label, snippet) in chips {
        let chip_btn = Button::builder()
            .label(label)
            .css_classes(["dms-chip-btn"])
            .build();
        let tv_clone = text_view.clone();
        let snippet_str = snippet.to_string();
        chip_btn.connect_clicked(move |_| {
            let buffer = tv_clone.buffer();
            let mut iter = buffer.iter_at_mark(&buffer.get_insert());
            buffer.insert(&mut iter, &snippet_str);
        });
        toolbar_box.append(&chip_btn);
    }
    main_box.append(&toolbar_box);

    // Area di testo scorrevole
    let scroll = ScrolledWindow::builder()
        .height_request(160)
        .hexpand(true)
        .child(&text_view)
        .css_classes(["dms-scrolled-editor"])
        .build();
    main_box.append(&scroll);

    // Barra azioni
    let actions_box = GtkBox::new(Orientation::Horizontal, 8);
    actions_box.set_halign(Align::End);

    let cancel_btn = Button::builder()
        .label("Annulla")
        .css_classes(["dms-btn-flat"])
        .build();
    let pop_cancel = popover.clone();
    cancel_btn.connect_clicked(move |_| {
        pop_cancel.popdown();
    });

    let confirm_btn = Button::builder()
        .label("Salva & Chiudi (Ctrl+Invio)")
        .css_classes(["suggested-action", "dms-pill-btn"])
        .build();

    let pop_confirm = popover.clone();
    let area_clone = area.clone();
    let state_confirm = state.clone();
    let tv_confirm = text_view.clone();
    let existing_confirm = existing_item.clone();

    let do_confirm = move || {
        let buffer = tv_confirm.buffer();
        let (start, end) = buffer.bounds();
        let raw_text = buffer.text(&start, &end, true).to_string();
        let clean = raw_text.trim();
        if clean.is_empty() {
            pop_confirm.popdown();
            return;
        }

        let mut st_guard = state_confirm.borrow_mut();
        let st = &mut *st_guard;
        if st.document.pages.is_empty() {
            pop_confirm.popdown();
            return;
        }

        let (target_page_idx, target_page_id) = match st.document.format {
            DocumentFormat::Whiteboard => (0, st.document.pages[0].id),
            DocumentFormat::Notebook => {
                let p_idx = Viewport::find_page_index_at_y(doc_pos.1, st.document.pages.len())
                    .min(st.document.pages.len().saturating_sub(1));
                (p_idx, st.document.pages[p_idx].id)
            }
        };
        let _ = target_page_idx;

        let width = if let Some((_, ref prev)) = existing_confirm {
            prev.bbox.width().max(200.0)
        } else {
            340.0
        };
        // Stima altezza iniziale dal numero di righe e formule
        let lines_count = clean.lines().count().max(1);
        let height = (lines_count as f64 * 24.0 + 40.0).max(64.0);

        if let Some((item_id, prev_item)) = existing_confirm.clone() {
            let new_bbox = Rect::new(
                prev_item.bbox.min_x,
                prev_item.bbox.min_y,
                prev_item.bbox.min_x + width,
                prev_item.bbox.min_y + height,
            );
            let cmd = DocCommand::EditText {
                page_id: target_page_id,
                item_id,
                old_text: prev_item.raw_text,
                new_text: raw_text,
                old_bbox: prev_item.bbox,
                new_bbox,
            };
            st.history.execute(cmd, &mut st.document);
        } else {
            let item_id = st.alloc_item_id();
            let z_index = (st.document.pages[target_page_idx].items.len() + 1) as u64;
            let bbox = Rect::new(doc_pos.0, doc_pos.1, doc_pos.0 + width, doc_pos.1 + height);
            let mut text_item = TextItem::new(target_page_id, raw_text, bbox, TextLayer::Normal);
            let is_dark = libadwaita::StyleManager::default().is_dark();
            if is_dark {
                text_item.background_fill = Some([0.13, 0.17, 0.20, 0.92]);
                text_item.border_color = Some([0.25, 0.32, 0.37, 1.0]);
            } else {
                text_item.background_fill = Some([0.98, 0.98, 0.99, 0.96]);
                text_item.border_color = Some([0.82, 0.85, 0.88, 1.0]);
            }
            let cmd = DocCommand::add_text(target_page_id, item_id, z_index, text_item);
            st.history.execute(cmd, &mut st.document);
        }

        drop(st_guard);
        area_clone.queue_draw();
        pop_confirm.popdown();
    };

    let do_confirm_rc = Rc::new(do_confirm);
    let do_c1 = do_confirm_rc.clone();
    confirm_btn.connect_clicked(move |_| {
        do_c1();
    });

    let do_c2 = do_confirm_rc.clone();
    let key_controller = gtk4::EventControllerKey::new();
    key_controller.connect_key_pressed(move |_, keyval, _, state| {
        if keyval == gdk::Key::Return && state.contains(gdk::ModifierType::CONTROL_MASK) {
            do_c2();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    text_view.add_controller(key_controller);

    actions_box.append(&cancel_btn);
    actions_box.append(&confirm_btn);
    main_box.append(&actions_box);

    popover.set_child(Some(&main_box));
    popover.popup();
    text_view.grab_focus();
}
