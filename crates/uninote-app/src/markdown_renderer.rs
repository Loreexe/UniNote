//! Parser e renderer tipografico per caselle di testo in stile Obsidian.
//!
//! Converte la sintassi Markdown (titoli, grassetto, corsivo, elenchi, codice)
//! in Pango Markup e individua i blocchi matematici LaTeX (`$$...$$` e `$inline$`)
//! per il disegno vettoriale composito su contesti Cairo.

use crate::math_renderer::render_latex_to_surface;
use gtk4::cairo;
use gtk4::pango;
use std::borrow::Cow;
use uninote_core::model::{TextItem, TextLayer};

/// Rappresentazione di un blocco di contenuto all'interno di una casella di testo.
#[derive(Debug, Clone)]
pub enum ParsedBlock {
    /// Paragrafo o titolo tipografico formattato con Pango Markup.
    Markup {
        text: String,
        font_size: f64,
        is_heading: bool,
    },
    /// Formula matematica a blocco centrato (`$$ ... $$`).
    MathDisplay { latex: String },
}

/// Esegue l'escape dei caratteri speciali XML per generare Pango Markup valido.
pub fn escape_pango(text: &str) -> Cow<'_, str> {
    if !text.contains('&') && !text.contains('<') && !text.contains('>') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    Cow::Owned(out)
}

/// Converte una riga di testo Markdown semplice in Pango Markup con stili inline.
pub fn markdown_line_to_pango(line: &str) -> String {
    let escaped = escape_pango(line);
    let mut s = escaped.into_owned();

    // Grassetto: **testo** -> <b>testo</b>
    while let Some(start) = s.find("**") {
        if let Some(end) = s[start + 2..].find("**") {
            let actual_end = start + 2 + end;
            let inner = &s[start + 2..actual_end];
            s.replace_range(start..actual_end + 2, &format!("<b>{inner}</b>"));
        } else {
            break;
        }
    }

    // Corsivo: *testo* -> <i>testo</i>
    while let Some(start) = s.find('*') {
        if let Some(end) = s[start + 1..].find('*') {
            let actual_end = start + 1 + end;
            let inner = &s[start + 1..actual_end];
            s.replace_range(start..actual_end + 1, &format!("<i>{inner}</i>"));
        } else {
            break;
        }
    }

    // Codice inline: `codice` -> <tt>codice</tt>
    while let Some(start) = s.find('`') {
        if let Some(end) = s[start + 1..].find('`') {
            let actual_end = start + 1 + end;
            let inner = &s[start + 1..actual_end];
            s.replace_range(start..actual_end + 1, &format!("<tt>{inner}</tt>"));
        } else {
            break;
        }
    }

    s
}

/// Scompone il testo grezzo di una casella in blocchi tipografici e formule matematiche.
pub fn parse_text_blocks(raw_text: &str, base_font_size: f64) -> Vec<ParsedBlock> {
    let mut blocks = Vec::new();
    let lines: Vec<&str> = raw_text.lines().collect();
    let mut idx = 0;

    while idx < lines.len() {
        let line = lines[idx].trim();

        // 1. Blocco Display Math: $$ ... $$
        if let Some(stripped_start) = line.strip_prefix("$$") {
            if let Some(inner) = stripped_start.strip_suffix("$$") {
                // Formula su singola riga: $$formula$$
                let latex = inner.trim();
                if !latex.is_empty() {
                    blocks.push(ParsedBlock::MathDisplay {
                        latex: latex.to_string(),
                    });
                }
                idx += 1;
                continue;
            } else {
                // Formula multilinea: raccoglie fino al closing $$
                let mut math_lines = Vec::new();
                let first = stripped_start.trim();
                if !first.is_empty() {
                    math_lines.push(first);
                }
                idx += 1;
                while idx < lines.len() {
                    let next_line = lines[idx].trim();
                    if let Some(inner) = next_line.strip_suffix("$$") {
                        let last = inner.trim();
                        if !last.is_empty() {
                            math_lines.push(last);
                        }
                        idx += 1;
                        break;
                    } else {
                        math_lines.push(next_line);
                        idx += 1;
                    }
                }
                let full_latex = math_lines.join(" ");
                if !full_latex.is_empty() {
                    blocks.push(ParsedBlock::MathDisplay { latex: full_latex });
                }
                continue;
            }
        }

        // 2. Titoli Markdown: #, ##, ###
        if let Some(title) = line.strip_prefix("### ") {
            blocks.push(ParsedBlock::Markup {
                text: format!("<b>{}</b>", markdown_line_to_pango(title)),
                font_size: base_font_size * 1.18,
                is_heading: true,
            });
            idx += 1;
            continue;
        }
        if let Some(title) = line.strip_prefix("## ") {
            blocks.push(ParsedBlock::Markup {
                text: format!("<b>{}</b>", markdown_line_to_pango(title)),
                font_size: base_font_size * 1.35,
                is_heading: true,
            });
            idx += 1;
            continue;
        }
        if let Some(title) = line.strip_prefix("# ") {
            blocks.push(ParsedBlock::Markup {
                text: format!("<b>{}</b>", markdown_line_to_pango(title)),
                font_size: base_font_size * 1.6,
                is_heading: true,
            });
            idx += 1;
            continue;
        }

        // 3. Elenchi puntati: - elemento o * elemento
        if line.starts_with("- ") || line.starts_with("* ") {
            let bullet_line = format!("• {}", &line[2..]);
            blocks.push(ParsedBlock::Markup {
                text: markdown_line_to_pango(&bullet_line),
                font_size: base_font_size,
                is_heading: false,
            });
            idx += 1;
            continue;
        }

        // 4. Paragrafo ordinario
        if !line.is_empty() {
            blocks.push(ParsedBlock::Markup {
                text: markdown_line_to_pango(line),
                font_size: base_font_size,
                is_heading: false,
            });
        }
        idx += 1;
    }

    blocks
}

/// Disegna un `TextItem` (sia formula di fondo che casella di testo) sul contesto Cairo fornito.
pub fn draw_text_item(cr: &cairo::Context, item: &TextItem, is_dark: bool) {
    let bbox = item.bbox.normalized();
    let width = bbox.width();
    let height = bbox.height();
    if width <= 0.0 || height <= 0.0 {
        return;
    }

    let _ = cr.save();

    // Disegno sfondo e bordo per caselle normali se presenti
    if item.layer == TextLayer::Normal {
        if let Some(fill) = item.background_fill {
            cr.set_source_rgba(fill[0], fill[1], fill[2], fill[3]);
            let r = 8.0f64.min(width * 0.1).min(height * 0.1);
            draw_rounded_rect(cr, bbox.min_x, bbox.min_y, width, height, r);
            let _ = cr.fill();
        }
        if let Some(border) = item.border_color {
            cr.set_source_rgba(border[0], border[1], border[2], border[3]);
            cr.set_line_width(1.0);
            let r = 8.0f64.min(width * 0.1).min(height * 0.1);
            draw_rounded_rect(cr, bbox.min_x, bbox.min_y, width, height, r);
            let _ = cr.stroke();
        }
    }

    // Se è una formula LaTeX pura sul layer di fondo
    if item.layer == TextLayer::Underlay {
        let trimmed = item.raw_text.trim();
        let latex_content =
            if trimmed.starts_with("$$") && trimmed.ends_with("$$") && trimmed.len() > 2 {
                &trimmed[2..trimmed.len() - 2]
            } else {
                trimmed
            };

        if let Some(surf) = render_latex_to_surface(latex_content) {
            let tex_w = surf.width() as f64;
            let tex_h = surf.height() as f64;
            if tex_w > 0.0 && tex_h > 0.0 {
                // Centra la formula all'interno del bounding box
                let target_w = tex_w.min(width);
                let scale = target_w / tex_w;
                let target_h = tex_h * scale;

                let draw_x = bbox.min_x + (width - target_w) * 0.5;
                let draw_y = bbox.min_y + (height - target_h) * 0.5;

                let _ = cr.save();
                cr.translate(draw_x, draw_y);
                cr.scale(scale, scale);
                let _ = cr.set_source_surface(&surf, 0.0, 0.0);
                let _ = cr.paint();
                let _ = cr.restore();
            }
            let _ = cr.restore();
            return;
        }
    }

    // Rendering tipografico a blocchi per caselle di testo o fallback
    let blocks = parse_text_blocks(&item.raw_text, item.font_size);
    let mut cur_y = bbox.min_y + 8.0;
    let margin_x = bbox.min_x + 8.0;
    let max_text_w = (width - 16.0).max(20.0);

    let default_color = if is_dark {
        [0.92, 0.92, 0.94, 1.0]
    } else {
        [0.12, 0.12, 0.14, 1.0]
    };
    let color = if item.text_color[3] > 0.0 {
        item.text_color
    } else {
        default_color
    };

    for block in blocks {
        match block {
            ParsedBlock::MathDisplay { latex } => {
                if let Some(surf) = render_latex_to_surface(&latex) {
                    let tex_w = surf.width() as f64;
                    let tex_h = surf.height() as f64;
                    if tex_w > 0.0 && tex_h > 0.0 {
                        let target_w = tex_w.min(max_text_w);
                        let scale = target_w / tex_w;
                        let target_h = tex_h * scale;
                        let draw_x = margin_x + (max_text_w - target_w) * 0.5;

                        let _ = cr.save();
                        cr.translate(draw_x, cur_y);
                        cr.scale(scale, scale);
                        let _ = cr.set_source_surface(&surf, 0.0, 0.0);
                        let _ = cr.paint();
                        let _ = cr.restore();

                        cur_y += target_h + 8.0;
                    }
                }
            }
            ParsedBlock::Markup {
                text,
                font_size,
                is_heading,
            } => {
                let layout = pangocairo::functions::create_layout(cr);
                let mut desc = pango::FontDescription::new();
                desc.set_family("sans-serif");
                desc.set_size((font_size * pango::SCALE as f64) as i32);
                if is_heading {
                    desc.set_weight(pango::Weight::Bold);
                }
                layout.set_font_description(Some(&desc));
                layout.set_markup(&text);
                layout.set_width((max_text_w * pango::SCALE as f64) as i32);
                layout.set_wrap(pango::WrapMode::WordChar);

                cr.set_source_rgba(color[0], color[1], color[2], color[3]);
                cr.move_to(margin_x, cur_y);
                pangocairo::functions::show_layout(cr, &layout);

                let (_, h) = layout.pixel_size();
                cur_y += (h as f64) + 6.0;
            }
        }
    }

    let _ = cr.restore();
}

/// Helper per tracciare un rettangolo arrotondato su Cairo.
fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi * 0.5, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi * 0.5);
    cr.arc(x + r, y + h - r, r, pi * 0.5, pi);
    cr.arc(x + r, y + r, r, pi, pi * 1.5);
    cr.close_path();
}
