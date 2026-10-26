use crate::model::{A4_HEIGHT_PT, A4_WIDTH_PT};
use serde::{Deserialize, Serialize};

/// Spaziatura verticale tra le pagine A4 consecutive in punti tipografici.
pub const PAGE_GAP_PT: f64 = 30.0;
/// Margine superiore iniziale del documento prima della prima pagina.
pub const PAGE_TOP_MARGIN_PT: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Point2D {
    pub x: f64,
    pub y: f64,
}

impl Point2D {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn distance_to(&self, other: &Point2D) -> f64 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        (dx * dx + dy * dy).sqrt()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Rect2D {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect2D {
    pub fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn contains(&self, pt: Point2D) -> bool {
        pt.x >= self.x
            && pt.x <= self.x + self.width
            && pt.y >= self.y
            && pt.y <= self.y + self.height
    }

    pub fn intersects(&self, other: &Rect2D) -> bool {
        self.x < other.x + other.width
            && self.x + self.width > other.x
            && self.y < other.y + other.height
            && self.y + self.height > other.y
    }

    pub fn expand(&self, margin: f64) -> Self {
        Self {
            x: self.x - margin,
            y: self.y - margin,
            width: self.width + margin * 2.0,
            height: self.height + margin * 2.0,
        }
    }
}

/// Gestore della trasformazione tra coordinate dello schermo (pixel del widget) e coordinate del documento (pt).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub offset_x: f64,
    pub offset_y: f64,
    pub zoom: f64,
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            offset_x: 60.0,
            offset_y: PAGE_TOP_MARGIN_PT,
            zoom: 1.0,
        }
    }
}

pub const MIN_ZOOM: f64 = 0.25;
pub const MAX_ZOOM: f64 = 10.0;

impl Viewport {
    pub fn new(offset_x: f64, offset_y: f64, zoom: f64) -> Self {
        Self {
            offset_x,
            offset_y,
            zoom: zoom.clamp(MIN_ZOOM, MAX_ZOOM),
        }
    }

    /// Converte una coordinata schermo (pixel del widget) in coordinata del documento (pt).
    pub fn screen_to_document(&self, screen_pt: (f64, f64)) -> (f64, f64) {
        let doc_x = (screen_pt.0 - self.offset_x) / self.zoom;
        let doc_y = (screen_pt.1 - self.offset_y) / self.zoom;
        (doc_x, doc_y)
    }

    /// Converte una coordinata documento (pt) in coordinata schermo (pixel del widget).
    pub fn document_to_screen(&self, doc_pt: (f64, f64)) -> (f64, f64) {
        let screen_x = doc_pt.0 * self.zoom + self.offset_x;
        let screen_y = doc_pt.1 * self.zoom + self.offset_y;
        (screen_x, screen_y)
    }

    /// Applica una traslazione di pan.
    pub fn pan(&mut self, dx: f64, dy: f64) {
        self.offset_x += dx;
        self.offset_y += dy;
    }

    /// Esegue lo zoom centrato sul puntatore dello schermo.
    /// Mantiene invariate le coordinate documento presenti sotto (pivot_sx, pivot_sy).
    pub fn zoom_at(&mut self, pivot_screen: (f64, f64), zoom_multiplier: f64) {
        let (doc_x, doc_y) = self.screen_to_document(pivot_screen);
        let new_zoom = (self.zoom * zoom_multiplier).clamp(MIN_ZOOM, MAX_ZOOM);

        self.offset_x = pivot_screen.0 - doc_x * new_zoom;
        self.offset_y = pivot_screen.1 - doc_y * new_zoom;
        self.zoom = new_zoom;
    }

    /// Reimposta lo zoom a 100% (1.0), centrando orizzontalmente il foglio A4 nella finestra.
    pub fn reset_zoom_and_center(&mut self, window_width: f64) {
        self.zoom = 1.0;
        let page_w = A4_WIDTH_PT;
        self.offset_x = ((window_width - page_w) / 2.0).max(20.0);
    }

    /// Centra e scala il viewport per inquadrare un bounding box (es. "Centra Contenuto" per Lavagna).
    pub fn center_on_rect(&mut self, rect: Rect2D, win_w: f64, win_h: f64) {
        if !rect.x.is_finite()
            || !rect.y.is_finite()
            || !rect.width.is_finite()
            || !rect.height.is_finite()
            || rect.width <= 0.0
            || rect.height <= 0.0
            || !win_w.is_finite()
            || !win_h.is_finite()
            || win_w <= 0.0
            || win_h <= 0.0
        {
            self.offset_x = if win_w.is_finite() && win_w > 0.0 {
                win_w / 2.0
            } else {
                0.0
            };
            self.offset_y = if win_h.is_finite() && win_h > 0.0 {
                win_h / 2.0
            } else {
                0.0
            };
            self.zoom = 1.0;
            return;
        }

        let padding = 60.0;
        let avail_w = (win_w - padding * 2.0).max(100.0);
        let avail_h = (win_h - padding * 2.0).max(100.0);
        let scale_x = avail_w / rect.width;
        let scale_y = avail_h / rect.height;
        self.zoom = scale_x.min(scale_y).clamp(0.25, 3.0);

        let center_x = rect.x + rect.width / 2.0;
        let center_y = rect.y + rect.height / 2.0;
        self.offset_x = win_w / 2.0 - center_x * self.zoom;
        self.offset_y = win_h / 2.0 - center_y * self.zoom;
    }

    /// Calcola il rettangolo globale della pagina dato il suo indice sequenziale.
    pub fn page_rect(page_index: usize) -> Rect2D {
        let top = PAGE_TOP_MARGIN_PT + (page_index as f64) * (A4_HEIGHT_PT + PAGE_GAP_PT);
        Rect2D::new(0.0, top, A4_WIDTH_PT, A4_HEIGHT_PT)
    }

    /// Individua l'indice della pagina associata a una data coordinata Y del documento.
    pub fn find_page_index_at_y(doc_y: f64, total_pages: usize) -> usize {
        if total_pages <= 1 {
            return 0;
        }

        let stride = A4_HEIGHT_PT + PAGE_GAP_PT;
        let adjusted_y = (doc_y - PAGE_TOP_MARGIN_PT).max(0.0);
        let index = (adjusted_y / stride).floor() as usize;
        index.min(total_pages - 1)
    }
}
