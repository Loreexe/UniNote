//! Motore di rendering matematico vettoriale per UniNote.
//!
//! Basato su `latex-rust` e i font matematici OpenType STIX Two Math incorporati.
//! Genera standard vettoriali SVG, superfici `cairo::ImageSurface` e texture `gdk::Texture` GTK4,
//! memorizzate in una cache locale per garantire latenza zero e assenza di allocazioni
//! nel loop di disegno Cairo a 120 FPS.

use gtk4::cairo::{self, Format, ImageSurface};
use gtk4::gdk::{self, prelude::*};
use gtk4::glib;
use latex_rust::{latex_to_svg, MathFont, SvgOptions};
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

static MATH_FONT: OnceLock<Option<MathFont>> = OnceLock::new();

/// Restituisce un riferimento al font matematico STIX Two Math inizializzato globalmente.
fn get_math_font() -> Option<&'static MathFont> {
    MATH_FONT
        .get_or_init(|| match MathFont::stix_two_math() {
            Ok(font) => Some(font),
            Err(err) => {
                eprintln!("UniNote: impossibile caricare il font STIX Two Math: {err:?}");
                None
            }
        })
        .as_ref()
}

thread_local! {
    /// Cache in-memory delle texture matematiche GTK4 compilate.
    static TEXTURE_CACHE: RefCell<HashMap<String, gdk::Texture>> = RefCell::new(HashMap::new());
    /// Cache in-memory delle superfici raster Cairo ImageSurface compilate.
    static SURFACE_CACHE: RefCell<HashMap<String, ImageSurface>> = RefCell::new(HashMap::new());
    /// Cache in-memory del codice SVG vettoriale per esportazione PDF.
    static SVG_CACHE: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Compila un'espressione LaTeX in codice vettoriale SVG.
pub fn render_latex_to_svg(latex: &str) -> Result<String, String> {
    let clean = latex.trim();
    if clean.is_empty() {
        return Err("Formula LaTeX vuota".to_string());
    }

    if let Some(cached) = SVG_CACHE.with(|c| c.borrow().get(clean).cloned()) {
        return Ok(cached);
    }

    let font = get_math_font()
        .ok_or_else(|| "Font STIX Two Math non disponibile nel sistema".to_string())?;
    let options = SvgOptions::new();

    match latex_to_svg(clean, font, &options) {
        Ok(svg) => {
            SVG_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                if map.len() > 600 {
                    map.clear();
                }
                map.insert(clean.to_string(), svg.clone());
            });
            Ok(svg)
        }
        Err(err) => Err(format!("Errore sintassi formula LaTeX: {err:?}")),
    }
}

/// Compila un'espressione LaTeX in una texture hardware-accelerata GTK4.
///
/// Restituisce `Some(gdk::Texture)` se la compilazione e la rasterizzazione hanno successo,
/// oppure `None` in caso di formula vuota o errore di parsing.
pub fn render_latex_to_texture(latex: &str) -> Option<gdk::Texture> {
    let clean = latex.trim();
    if clean.is_empty() {
        return None;
    }

    if let Some(tex) = TEXTURE_CACHE.with(|c| c.borrow().get(clean).cloned()) {
        return Some(tex);
    }

    let svg = match render_latex_to_svg(clean) {
        Ok(s) => s,
        Err(_) => return None,
    };

    let bytes = glib::Bytes::from(svg.as_bytes());
    match gdk::Texture::from_bytes(&bytes) {
        Ok(texture) => {
            TEXTURE_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                if map.len() > 600 {
                    map.clear();
                }
                map.insert(clean.to_string(), texture.clone());
            });
            Some(texture)
        }
        Err(err) => {
            eprintln!("UniNote: errore creazione texture GTK4 da SVG LaTeX: {err}");
            None
        }
    }
}

/// Compila un'espressione LaTeX in una superficie raster `cairo::ImageSurface` con caching LRU.
pub fn render_latex_to_surface(latex: &str) -> Option<ImageSurface> {
    let clean = latex.trim();
    if clean.is_empty() {
        return None;
    }

    if let Some(surf) = SURFACE_CACHE.with(|c| c.borrow().get(clean).cloned()) {
        return Some(surf);
    }

    let tex = render_latex_to_texture(clean)?;
    let w = tex.width();
    let h = tex.height();
    if w <= 0 || h <= 0 {
        return None;
    }

    let stride = Format::ARgb32.stride_for_width(w as u32).ok()?;
    let mut data = vec![0u8; (stride * h) as usize];
    tex.download(&mut data, stride as usize);

    match ImageSurface::create_for_data(data, Format::ARgb32, w, h, stride) {
        Ok(surface) => {
            SURFACE_CACHE.with(|c| {
                let mut map = c.borrow_mut();
                if map.len() > 600 {
                    map.clear();
                }
                map.insert(clean.to_string(), surface.clone());
            });
            Some(surface)
        }
        Err(err) => {
            eprintln!("UniNote: errore creazione ImageSurface da Texture: {err}");
            None
        }
    }
}

/// Disegna una formula LaTeX su un contesto Cairo alle coordinate `(x, y)`.
///
/// Restituisce la dimensione `(larghezza, altezza)` della formula renderizzata.
pub fn draw_latex(cr: &cairo::Context, latex: &str, x: f64, y: f64) -> Option<(f64, f64)> {
    let surface = render_latex_to_surface(latex)?;
    let w = surface.width() as f64;
    let h = surface.height() as f64;

    let _ = cr.save();
    let _ = cr.set_source_surface(&surface, x, y);
    let _ = cr.paint();
    let _ = cr.restore();

    Some((w, h))
}

/// Svuota la cache delle texture matematiche (ad es. al cambio di documento o per liberare RAM).
pub fn clear_math_cache() {
    TEXTURE_CACHE.with(|c| c.borrow_mut().clear());
    SURFACE_CACHE.with(|c| c.borrow_mut().clear());
    SVG_CACHE.with(|c| c.borrow_mut().clear());
}
