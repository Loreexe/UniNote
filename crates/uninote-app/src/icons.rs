use gtk4::cairo;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::f64::consts::PI;

/// Tipi di icone vettoriali supportate dall'interfaccia UniNote DMS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconKind {
    AppLogo,
    Undo,
    Redo,
    ChevronDown,
    PenTool,
    Eraser,
    Highlighter,
    Type,
    Line,
    Shapes,
    Export,
    Settings,
    ChevronLeft,
    ChevronRight,
    Page,
    Add,
    Minus,
    PenLine,
    Select,
    MoreVertical,
    Edit,
    Copy,
    Trash,
    Folder,
    FolderPlus,
    FolderOpen,
}

/// Crea una `DrawingArea` GTK4 che disegna l'icona vettoriale indicata con Cairo.
/// Se `custom_color` è `None`, l'icona eredita dinamicamente il colore CSS del widget.
pub fn create_icon(kind: IconKind, size: i32, custom_color: Option<gdk::RGBA>) -> DrawingArea {
    let area = DrawingArea::builder()
        .content_width(size)
        .content_height(size)
        .valign(gtk4::Align::Center)
        .halign(gtk4::Align::Center)
        .build();

    area.set_draw_func(move |widget, cr, width, height| {
        let rgba = custom_color.unwrap_or_else(|| widget.color());

        if kind == IconKind::AppLogo {
            draw_app_logo(cr, width as f64, height as f64);
            return;
        }

        // Coordinate space 24x24 standard Lucide
        let w = width as f64;
        let h = height as f64;
        let scale = (w.min(h) / 24.0).max(0.01);
        let pad_x = (w - 24.0 * scale) / 2.0;
        let pad_y = (h - 24.0 * scale) / 2.0;

        cr.save().expect("cairo save");
        cr.translate(pad_x, pad_y);
        cr.scale(scale, scale);

        cr.set_source_rgba(
            rgba.red() as f64,
            rgba.green() as f64,
            rgba.blue() as f64,
            rgba.alpha() as f64,
        );
        cr.set_line_width(2.0);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);

        match kind {
            IconKind::AppLogo => {} // gestito sopra
            IconKind::Undo => draw_undo(cr),
            IconKind::Redo => draw_redo(cr),
            IconKind::ChevronDown => draw_chevron_down(cr),
            IconKind::PenTool => draw_pen_tool(cr),
            IconKind::Eraser => draw_eraser(cr),
            IconKind::Highlighter => draw_highlighter(cr),
            IconKind::Type => draw_type(cr),
            IconKind::Line => draw_line(cr),
            IconKind::Shapes => draw_shapes(cr),
            IconKind::Export => draw_export(cr),
            IconKind::Settings => draw_settings(cr),
            IconKind::ChevronLeft => draw_chevron_left(cr),
            IconKind::ChevronRight => draw_chevron_right(cr),
            IconKind::Page => draw_page(cr),
            IconKind::Add => draw_add(cr),
            IconKind::Minus => draw_minus(cr),
            IconKind::PenLine => draw_pen_line(cr),
            IconKind::Select => draw_select(cr),
            IconKind::MoreVertical => draw_more_vertical(cr),
            IconKind::Edit => draw_edit(cr),
            IconKind::Copy => draw_copy(cr),
            IconKind::Trash => draw_trash(cr),
            IconKind::Folder => draw_folder(cr),
            IconKind::FolderPlus => draw_folder_plus(cr),
            IconKind::FolderOpen => draw_folder_open(cr),
        }

        cr.restore().expect("cairo restore");
    });

    area
}

fn draw_undo(cr: &cairo::Context) {
    // Arrow head
    cr.move_to(9.0, 14.0);
    cr.line_to(4.0, 9.0);
    cr.line_to(9.0, 4.0);
    let _ = cr.stroke();

    // Hook
    cr.move_to(4.0, 9.0);
    cr.line_to(14.5, 9.0);
    cr.arc(14.5, 14.5, 5.5, -PI / 2.0, PI / 2.0);
    cr.line_to(11.0, 20.0);
    let _ = cr.stroke();
}

fn draw_redo(cr: &cairo::Context) {
    // Arrow head
    cr.move_to(15.0, 14.0);
    cr.line_to(20.0, 9.0);
    cr.line_to(15.0, 4.0);
    let _ = cr.stroke();

    // Hook
    cr.move_to(20.0, 9.0);
    cr.line_to(9.5, 9.0);
    cr.arc_negative(9.5, 14.5, 5.5, -PI / 2.0, PI / 2.0);
    cr.line_to(13.0, 20.0);
    let _ = cr.stroke();
}

fn draw_chevron_down(cr: &cairo::Context) {
    cr.move_to(6.0, 9.0);
    cr.line_to(12.0, 15.0);
    cr.line_to(18.0, 9.0);
    let _ = cr.stroke();
}

fn draw_chevron_left(cr: &cairo::Context) {
    cr.move_to(15.0, 18.0);
    cr.line_to(9.0, 12.0);
    cr.line_to(15.0, 6.0);
    let _ = cr.stroke();
}

fn draw_chevron_right(cr: &cairo::Context) {
    cr.move_to(9.0, 18.0);
    cr.line_to(15.0, 12.0);
    cr.line_to(9.0, 6.0);
    let _ = cr.stroke();
}

fn draw_pen_tool(cr: &cairo::Context) {
    // Nib
    cr.move_to(12.0, 19.0);
    cr.line_to(19.0, 12.0);
    let _ = cr.stroke();

    cr.move_to(18.0, 13.0);
    cr.line_to(17.3, 8.3);
    cr.curve_to(16.5, 5.5, 15.0, 5.0, 14.6, 5.0);
    cr.line_to(5.4, 2.1);
    cr.curve_to(4.0, 1.8, 3.0, 2.5, 2.6, 3.4);
    cr.line_to(1.8, 4.2);
    cr.curve_to(1.3, 5.0, 1.3, 6.0, 1.8, 7.0);
    cr.line_to(4.7, 16.2);
    cr.curve_to(5.5, 18.0, 7.0, 18.8, 8.0, 19.0);
    cr.line_to(13.0, 18.0);
    let _ = cr.stroke();

    cr.arc(11.0, 11.0, 2.0, 0.0, 2.0 * PI);
    let _ = cr.stroke();
    cr.move_to(11.0, 13.0);
    cr.line_to(18.0, 20.0);
    let _ = cr.stroke();
}

fn draw_pen_line(cr: &cairo::Context) {
    // Pen body angled 45 deg
    cr.move_to(21.2, 6.8);
    cr.line_to(17.2, 2.8);
    cr.line_to(3.8, 16.2);
    cr.line_to(2.5, 20.5);
    cr.line_to(6.9, 19.2);
    cr.close_path();
    let _ = cr.stroke();

    // Line under pen
    cr.move_to(13.0, 21.0);
    cr.line_to(21.0, 21.0);
    let _ = cr.stroke();
}

fn draw_eraser(cr: &cairo::Context) {
    cr.move_to(21.0, 21.0);
    cr.line_to(8.0, 21.0);
    cr.line_to(2.6, 15.6);
    cr.line_to(12.6, 5.6);
    cr.line_to(18.6, 11.6);
    cr.line_to(9.2, 21.0);
    let _ = cr.stroke();

    // Divider line
    cr.move_to(6.8, 11.4);
    cr.line_to(12.8, 17.4);
    let _ = cr.stroke();
}

fn draw_highlighter(cr: &cairo::Context) {
    // 1. Fusto principale inclinato a 45°
    cr.move_to(19.0, 4.0);
    cr.line_to(20.5, 5.5);
    cr.curve_to(21.2, 6.2, 21.2, 7.3, 20.5, 8.0);
    cr.line_to(14.5, 14.0);
    cr.line_to(10.0, 9.5);
    cr.line_to(16.0, 3.5);
    cr.curve_to(16.7, 2.8, 17.8, 2.8, 18.5, 3.5);
    cr.close_path();
    let _ = cr.stroke();

    // Collarino di battuta del fusto
    cr.move_to(11.5, 11.0);
    cr.line_to(13.0, 12.5);
    let _ = cr.stroke();

    // 2. Punta a Scalpello asimmetrica (Chisel-Tip) appoggiata alla linea base
    cr.move_to(10.0, 9.5);
    cr.line_to(4.5, 15.0);
    cr.line_to(4.5, 19.5); // Taglio verticale d'attacco dello scalpello
    cr.line_to(11.5, 19.5); // Taglio piatto orizzontale di contatto
    cr.line_to(14.5, 14.0);
    cr.close_path();
    let _ = cr.stroke();

    // 3. Tratto orizzontale d'inchiostro evidenziatore sulla carta
    cr.move_to(13.5, 20.5);
    cr.line_to(21.5, 20.5);
    let _ = cr.stroke();
}

fn draw_type(cr: &cairo::Context) {
    // Elegant 'A' for text formatting (Markdown + LaTeX)
    cr.move_to(4.5, 19.5);
    cr.line_to(12.0, 4.5);
    cr.line_to(19.5, 19.5);
    let _ = cr.stroke();

    cr.move_to(7.2, 14.2);
    cr.line_to(16.8, 14.2);
    let _ = cr.stroke();
}

fn draw_line(cr: &cairo::Context) {
    cr.move_to(20.0, 4.0);
    cr.line_to(4.0, 20.0);
    let _ = cr.stroke();
}

fn draw_shapes(cr: &cairo::Context) {
    // Triangle
    cr.move_to(8.0, 9.5);
    cr.line_to(12.0, 3.5);
    cr.line_to(16.0, 9.5);
    cr.close_path();
    let _ = cr.stroke();

    // Rectangle
    cr.rectangle(3.5, 13.5, 6.0, 6.0);
    let _ = cr.stroke();

    // Circle
    cr.arc(17.5, 16.5, 3.5, 0.0, 2.0 * PI);
    let _ = cr.stroke();
}

fn draw_export(cr: &cairo::Context) {
    // Tray
    cr.move_to(4.0, 12.0);
    cr.line_to(4.0, 20.0);
    cr.line_to(20.0, 20.0);
    cr.line_to(20.0, 12.0);
    let _ = cr.stroke();

    // Arrow up
    cr.move_to(12.0, 15.0);
    cr.line_to(12.0, 3.5);
    let _ = cr.stroke();

    cr.move_to(7.5, 8.0);
    cr.line_to(12.0, 3.5);
    cr.line_to(16.5, 8.0);
    let _ = cr.stroke();
}

fn draw_settings(cr: &cairo::Context) {
    let cx = 12.0;
    let cy = 12.0;
    let r_root = 6.8;
    let r_tip = 9.8;
    let a_tip = 0.20; // Semiamplitude dente testa (~11.5°)
    let a_root = 0.34; // Semiamplitude dente base (~19.5°)

    // Profilo ingranaggio meccanico solido a 6 denti sagomati
    for i in 0..6 {
        let theta = (i as f64) * PI / 3.0;
        let a_tip_start = theta - a_tip;
        let a_tip_end = theta + a_tip;
        let a_root_start = theta - a_root;
        let a_root_end = theta + a_root;
        let next_theta = ((i + 1) as f64) * PI / 3.0;
        let a_next_root_start = next_theta - a_root;

        if i == 0 {
            cr.move_to(
                cx + r_root * a_root_start.cos(),
                cy + r_root * a_root_start.sin(),
            );
        }

        // Salita trapezoidale fianco dente
        cr.line_to(
            cx + r_tip * a_tip_start.cos(),
            cy + r_tip * a_tip_start.sin(),
        );
        // Cresta esterna dente
        cr.arc(cx, cy, r_tip, a_tip_start, a_tip_end);
        // Discesa trapezoidale fianco dente
        cr.line_to(
            cx + r_root * a_root_end.cos(),
            cy + r_root * a_root_end.sin(),
        );
        // Fondo gola concentrica tra denti
        cr.arc(cx, cy, r_root, a_root_end, a_next_root_start);
    }
    cr.close_path();
    let _ = cr.stroke();

    // Foro circolare centrale di precisione (Bore Hole)
    cr.arc(cx, cy, 3.0, 0.0, 2.0 * PI);
    let _ = cr.stroke();
}

fn draw_page(cr: &cairo::Context) {
    // Page outline with folded corner
    cr.move_to(4.0, 20.0);
    cr.line_to(4.0, 4.0);
    cr.line_to(14.0, 4.0);
    cr.line_to(20.0, 10.0);
    cr.line_to(20.0, 20.0);
    cr.close_path();
    let _ = cr.stroke();

    // Corner flap
    cr.move_to(14.0, 4.0);
    cr.line_to(14.0, 10.0);
    cr.line_to(20.0, 10.0);
    let _ = cr.stroke();

    // Text lines
    cr.move_to(8.0, 14.0);
    cr.line_to(16.0, 14.0);
    cr.move_to(8.0, 17.0);
    cr.line_to(13.0, 17.0);
    let _ = cr.stroke();
}

fn draw_add(cr: &cairo::Context) {
    cr.move_to(12.0, 5.0);
    cr.line_to(12.0, 19.0);
    cr.move_to(5.0, 12.0);
    cr.line_to(19.0, 12.0);
    let _ = cr.stroke();
}

fn draw_minus(cr: &cairo::Context) {
    cr.move_to(5.0, 12.0);
    cr.line_to(19.0, 12.0);
    let _ = cr.stroke();
}

fn draw_select(cr: &cairo::Context) {
    // Icona lazo di selezione (tratteggio superiore con nodo e coda)
    let _ = cr.save();
    cr.set_dash(&[3.5, 2.5], 0.0);
    cr.move_to(10.0, 16.0);
    cr.curve_to(5.0, 15.0, 4.0, 7.0, 12.0, 6.0);
    cr.curve_to(20.0, 5.0, 21.0, 13.0, 14.0, 16.0);
    let _ = cr.stroke();
    let _ = cr.restore();

    // Nodo dell'asola
    cr.move_to(10.0, 16.0);
    cr.curve_to(11.0, 18.0, 13.0, 18.0, 14.0, 16.0);
    cr.curve_to(15.0, 14.5, 9.0, 14.5, 10.0, 16.0);
    let _ = cr.stroke();

    // Coda della fune
    cr.move_to(12.0, 17.5);
    cr.curve_to(12.0, 20.0, 15.0, 21.0, 18.0, 20.5);
    let _ = cr.stroke();
}

fn draw_more_vertical(cr: &cairo::Context) {
    // 3 pallini verticali centrati su x = 12.0, y = 5.0, 12.0, 19.0
    for &y in &[5.0, 12.0, 19.0] {
        cr.arc(12.0, y, 1.8, 0.0, 2.0 * PI);
        let _ = cr.fill();
    }
}

fn draw_edit(cr: &cairo::Context) {
    // Matita inclinata (stile Lucide Pencil)
    cr.move_to(17.0, 3.0);
    cr.line_to(21.0, 7.0);
    cr.line_to(7.0, 21.0);
    cr.line_to(3.0, 21.0);
    cr.line_to(3.0, 17.0);
    cr.close_path();
    let _ = cr.stroke();

    cr.move_to(14.0, 6.0);
    cr.line_to(18.0, 10.0);
    let _ = cr.stroke();
}

fn draw_copy(cr: &cairo::Context) {
    // Foglio posteriore
    cr.move_to(8.0, 4.0);
    cr.line_to(18.0, 4.0);
    cr.line_to(20.0, 6.0);
    cr.line_to(20.0, 16.0);
    let _ = cr.stroke();

    // Foglio anteriore principale
    let x = 4.0;
    let y = 8.0;
    let w = 12.0;
    let h = 12.0;
    let r = 2.0;
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
    let _ = cr.stroke();
}

fn draw_trash(cr: &cairo::Context) {
    // Coperchio orizzontale
    cr.move_to(3.0, 6.0);
    cr.line_to(21.0, 6.0);
    let _ = cr.stroke();

    // Manico superiore
    cr.move_to(8.0, 6.0);
    cr.line_to(8.0, 4.0);
    cr.line_to(16.0, 4.0);
    cr.line_to(16.0, 6.0);
    let _ = cr.stroke();

    // Secchio
    cr.move_to(5.0, 6.0);
    cr.line_to(6.0, 19.0);
    cr.arc(8.0, 19.0, 2.0, PI, PI / 2.0);
    cr.line_to(16.0, 21.0);
    cr.arc(16.0, 19.0, 2.0, PI / 2.0, 0.0);
    cr.line_to(19.0, 6.0);
    let _ = cr.stroke();

    // Scanalature verticali
    cr.move_to(10.0, 10.0);
    cr.line_to(10.0, 17.0);
    cr.move_to(14.0, 10.0);
    cr.line_to(14.0, 17.0);
    let _ = cr.stroke();
}

fn draw_folder(cr: &cairo::Context) {
    // Lucide folder vector geometry (24x24)
    cr.move_to(4.0, 20.0);
    cr.arc(20.0, 18.0, 2.0, PI / 2.0, 0.0);
    cr.line_to(22.0, 8.0);
    cr.arc(20.0, 8.0, 2.0, 0.0, 3.0 * PI / 2.0);
    cr.line_to(12.0, 6.0);
    cr.line_to(10.0, 4.0);
    cr.line_to(4.0, 4.0);
    cr.arc(4.0, 6.0, 2.0, 3.0 * PI / 2.0, PI);
    cr.line_to(2.0, 18.0);
    cr.arc(4.0, 18.0, 2.0, PI, PI / 2.0);
    cr.close_path();
    let _ = cr.stroke();
}

fn draw_folder_plus(cr: &cairo::Context) {
    draw_folder(cr);
    cr.move_to(12.0, 10.0);
    cr.line_to(12.0, 16.0);
    let _ = cr.stroke();
    cr.move_to(9.0, 13.0);
    cr.line_to(15.0, 13.0);
    let _ = cr.stroke();
}

fn draw_folder_open(cr: &cairo::Context) {
    // Parte posteriore della cartella con tab
    cr.move_to(2.0, 11.0);
    cr.line_to(2.0, 6.0);
    cr.arc(4.0, 6.0, 2.0, 3.0 * PI / 2.0, PI);
    cr.line_to(10.0, 4.0);
    cr.line_to(12.0, 6.0);
    cr.line_to(20.0, 6.0);
    cr.arc(20.0, 8.0, 2.0, 0.0, 3.0 * PI / 2.0);
    cr.line_to(22.0, 11.0);
    let _ = cr.stroke();

    // Falda anteriore aperta in prospettiva
    cr.move_to(2.0, 12.0);
    cr.line_to(5.0, 20.0);
    cr.line_to(20.5, 20.0);
    cr.line_to(22.5, 12.0);
    cr.close_path();
    let _ = cr.stroke();
}

/// Disegna il logo ufficiale UniNote: Monogramma 'U' accademico fuso con un pennino stilografico di precisione.
fn draw_app_logo(cr: &cairo::Context, w: f64, h: f64) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    cr.save().expect("cairo save");

    let s = w.min(h);
    let r = s * (8.5 / 36.0); // Raggio squircle proporzionale

    // 1. Sfondo Squircle con gradiente oceanico DMS (#004D65 -> #00222E)
    draw_rounded_rect(cr, 0.0, 0.0, s, s, r);
    let pat = cairo::LinearGradient::new(0.0, 0.0, s, s);
    pat.add_color_stop_rgb(0.0, 0.0, 0.302, 0.396); // #004D65
    pat.add_color_stop_rgb(1.0, 0.0, 0.133, 0.180); // #00222E
    cr.set_source(&pat).expect("gradient pattern");
    let _ = cr.fill_preserve();

    // Bordo interno a luce radente ciano ultra-sottile (1px)
    cr.set_source_rgba(0.427, 0.824, 1.0, 0.22);
    cr.set_line_width((s / 36.0).max(0.75));
    let _ = cr.stroke();

    // Griglia geometrica 36x36 per gli elementi del monogramma
    let is = s / 36.0;
    cr.translate((s - 36.0 * is) / 2.0, (s - 36.0 * is) / 2.0);
    cr.scale(is, is);

    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    // 2. Braccio sinistro e conca inferiore della 'U' (Bianco Ghiaccio #ECEFF4)
    cr.set_source_rgb(0.925, 0.937, 0.957);
    cr.set_line_width(3.6);
    cr.move_to(10.5, 9.5);
    cr.line_to(10.5, 20.0);
    cr.curve_to(10.5, 25.5, 14.5, 28.5, 18.0, 28.5);
    let _ = cr.stroke();

    // 3. Pennino Stilografico angolato (Braccio destro della 'U' in Ciano #6DD2FF)
    // Orientato verso il centro in basso dello squircle (x=18.0, y ≈ 28.5)
    cr.move_to(23.5, 9.5);
    cr.line_to(27.5, 9.5);
    cr.line_to(28.0, 17.5); // Spalla esterna del pennino
    cr.line_to(18.0, 28.5); // Punta del pennino orientata al centro inferiore
    cr.line_to(20.5, 19.5); // Spalla interna del pennino
    cr.close_path();

    cr.set_source_rgba(0.427, 0.824, 1.0, 0.25);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(0.427, 0.824, 1.0); // Ciano Elettrico UniNote
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    // Fessura d'inchiostro (Ink Slit) e Foro di sfiato (Breather Hole)
    cr.move_to(18.0, 28.5);
    cr.line_to(23.0, 19.0);
    cr.set_line_width(1.4);
    let _ = cr.stroke();

    cr.arc(23.0, 19.0, 1.2, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.925, 0.937, 0.957);
    let _ = cr.fill();

    // 4. Accento Verde Menta DMS (#79D9A5) all'apice del pennino
    cr.arc(17.2, 29.8, 1.4, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.475, 0.851, 0.647);
    let _ = cr.fill();

    cr.restore().expect("cairo restore");
}

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::cairo::{Format, ImageSurface};

    #[test]
    fn test_draw_app_logo_renders_properly() {
        let surface = ImageSurface::create(Format::ARgb32, 64, 64).expect("surface create");
        let cr = cairo::Context::new(&surface).expect("cairo context");
        draw_app_logo(&cr, 64.0, 64.0);
        assert!(cr.status().is_ok());

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let painted_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(painted_pixels > 100, "Official app logo must paint pixels");
    }

    #[test]
    fn test_draw_app_logo_scales() {
        for &size in &[16.0, 32.0, 35.0, 48.0, 128.0] {
            let surface = ImageSurface::create(Format::ARgb32, size as i32, size as i32)
                .expect("surface create");
            let cr = cairo::Context::new(&surface).expect("cairo context");
            draw_app_logo(&cr, size, size);
            assert!(
                cr.status().is_ok(),
                "Logo must render cleanly at scale {}",
                size
            );
        }
    }

    #[test]
    fn test_draw_app_logo_zero_or_negative_dimensions_no_panic() {
        let surface = ImageSurface::create(Format::ARgb32, 10, 10).expect("surface create");
        let cr = cairo::Context::new(&surface).expect("cairo context");
        draw_app_logo(&cr, 0.0, 0.0);
        draw_app_logo(&cr, -5.0, 10.0);
        draw_app_logo(&cr, 10.0, -5.0);
        assert!(cr.status().is_ok());
    }

    #[test]
    fn test_draw_settings_gear_profile() {
        let surface = ImageSurface::create(Format::ARgb32, 24, 24).expect("surface create");
        let cr = cairo::Context::new(&surface).expect("cairo context");
        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        draw_settings(&cr);
        assert!(cr.status().is_ok());

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let painted_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(painted_pixels > 50, "Settings gear must paint pixels");
    }

    #[test]
    fn test_draw_highlighter_chisel_profile() {
        let surface = ImageSurface::create(Format::ARgb32, 24, 24).expect("surface create");
        let cr = cairo::Context::new(&surface).expect("cairo context");
        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        draw_highlighter(&cr);
        assert!(cr.status().is_ok());

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let painted_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(painted_pixels > 50, "Highlighter must paint pixels");
    }

    #[test]
    fn test_draw_folder_icons_render_properly() {
        for (name, draw_fn) in [
            ("folder", draw_folder as fn(&cairo::Context)),
            ("folder_plus", draw_folder_plus),
            ("folder_open", draw_folder_open),
        ] {
            let surface = ImageSurface::create(Format::ARgb32, 24, 24).expect("surface create");
            let cr = cairo::Context::new(&surface).expect("cairo context");
            cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
            cr.set_line_width(2.0);
            draw_fn(&cr);
            assert!(cr.status().is_ok(), "Cairo status error for {}", name);

            drop(cr);
            let data = surface.take_data().expect("take_data");
            let painted_pixels = data.iter().filter(|&&b| b > 0).count();
            assert!(
                painted_pixels > 30,
                "Icon {} must paint pixels: got {}",
                name,
                painted_pixels
            );
        }
    }
}
