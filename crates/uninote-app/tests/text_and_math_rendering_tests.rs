use gtk4::cairo::{Context, Format, ImageSurface};
use uninote_app::markdown_renderer::{draw_text_item, parse_text_blocks, ParsedBlock};
use uninote_app::math_renderer::{render_latex_to_surface, render_latex_to_svg};
use uninote_core::model::{TextItem, TextLayer};
use uninote_stroke::Rect;
use uuid::Uuid;

#[test]
fn test_render_latex_to_svg_and_surface() {
    let formula = r"\frac{a}{b} + \sqrt{x}";
    let svg = render_latex_to_svg(formula).expect("LaTeX to SVG should succeed");
    assert!(svg.contains("<svg"), "SVG output must contain <svg tag");

    let surf = render_latex_to_surface(formula).expect("LaTeX to surface should succeed");
    assert!(surf.width() > 0, "Surface width must be positive");
    assert!(surf.height() > 0, "Surface height must be positive");
}

#[test]
fn test_markdown_parser_blocks() {
    let md = "# Titolo Principale\n## Sottotitolo\n- Primo punto\n* Secondo punto\n$$E = mc^2$$\nTesto normale con **grassetto**";
    let blocks = parse_text_blocks(md, 14.0);
    assert!(blocks.len() >= 5);

    let mut found_heading = false;
    let mut found_math = false;
    let mut found_bullet = false;

    for b in blocks {
        match b {
            ParsedBlock::Markup {
                is_heading, text, ..
            } => {
                if is_heading {
                    found_heading = true;
                }
                if text.contains('•') {
                    found_bullet = true;
                }
            }
            ParsedBlock::MathDisplay { latex } => {
                if latex.contains("E = mc^2") {
                    found_math = true;
                }
            }
        }
    }

    assert!(found_heading, "Heading block must be found");
    assert!(found_math, "Math display block must be found");
    assert!(found_bullet, "Bullet points must be formatted");
}

#[test]
fn test_draw_text_item_cairo_no_crash() {
    let surface = ImageSurface::create(Format::ARgb32, 400, 300).expect("ImageSurface create");
    let cr = Context::new(&surface).expect("Cairo Context create");

    let page_id = Uuid::new_v4();
    let underlay_item = TextItem::new(
        page_id,
        r"\int_0^1 x^2 dx = \frac{1}{3}",
        Rect::new(20.0, 20.0, 180.0, 80.0),
        TextLayer::Underlay,
    );

    draw_text_item(&cr, &underlay_item, true);
    draw_text_item(&cr, &underlay_item, false);

    let normal_item = TextItem::new(
        page_id,
        "# Nota di Laboratorio\n- Misurazione 1\n$$V = I R$$",
        Rect::new(10.0, 10.0, 250.0, 180.0),
        TextLayer::Normal,
    );

    draw_text_item(&cr, &normal_item, true);
    draw_text_item(&cr, &normal_item, false);
}
