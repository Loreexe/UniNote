use crate::coordinates::Rect2D;
use crate::spatial::PageSpatialIndex;
use serde::{Deserialize, Serialize};
use uninote_stroke::{Rect, Stroke};
use uuid::Uuid;

/// Dimensioni standard foglio A4 in punti tipografici (1 pt = 1/72 pollice).
/// 210 mm x 297 mm = 595.28 pt x 841.89 pt.
pub const A4_WIDTH_PT: f64 = 595.28;
pub const A4_HEIGHT_PT: f64 = 841.89;

/// Formato del documento UniNote.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum DocumentFormat {
    /// Quaderno a pagine discrete A4 a scorrimento verticale continuo.
    #[default]
    Notebook,
    /// Lavagna infinita bidirezionale a coordinate libere (-inf, +inf).
    Whiteboard,
}

/// Sfondo della pagina con parametri grafici.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageBackground {
    /// Foglio bianco caldo naturale.
    Blank,
    /// Foglio a righe orizzontali (interlinea tipica 8 mm ≈ 22.68 pt).
    Ruled { line_spacing: f64, margin_left: f64 },
    /// Foglio a quadretti (quadratura tipica 5 mm ≈ 14.17 pt).
    Grid { grid_size: f64 },
    /// Pagina PDF importata come sfondo vettoriale non distruttivo.
    PdfPage {
        source_path: String,
        page_index: usize,
        original_width: f64,
        original_height: f64,
    },
}

impl Default for PageBackground {
    fn default() -> Self {
        Self::Grid { grid_size: 14.17 } // Quadretti 5 mm standard universitario
    }
}

/// Pagina del documento UniNote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Page {
    pub id: Uuid,
    pub index: usize,
    pub width: f64,
    pub height: f64,
    pub background: PageBackground,
    pub items: Vec<Item>,
    #[serde(skip, default)]
    pub spatial_index: PageSpatialIndex,
}

impl Default for Page {
    fn default() -> Self {
        Self::new_a4(0, PageBackground::default())
    }
}

impl Page {
    pub fn new(index: usize, background: PageBackground) -> Self {
        Self::new_a4(index, background)
    }

    pub fn new_a4(index: usize, background: PageBackground) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            width: A4_WIDTH_PT,
            height: A4_HEIGHT_PT,
            background,
            items: Vec::new(),
            spatial_index: PageSpatialIndex::empty(),
        }
    }

    pub fn new_whiteboard(background: PageBackground) -> Self {
        Self {
            id: Uuid::new_v4(),
            index: 0,
            width: f64::INFINITY,
            height: f64::INFINITY,
            background,
            items: Vec::new(),
            spatial_index: PageSpatialIndex::empty(),
        }
    }

    /// Crea una nuova pagina a partire da una pagina PDF con dimensioni native.
    pub fn new_pdf_page(
        index: usize,
        source_path: impl Into<String>,
        page_index: usize,
        width: f64,
        height: f64,
    ) -> Self {
        let source_path = source_path.into();
        Self {
            id: Uuid::new_v4(),
            index,
            width,
            height,
            background: PageBackground::PdfPage {
                source_path,
                page_index,
                original_width: width,
                original_height: height,
            },
            items: Vec::new(),
            spatial_index: PageSpatialIndex::empty(),
        }
    }

    /// Inserisce un nuovo elemento nella pagina e nel relativo indice spaziale R-Tree.
    pub fn add_item(&mut self, item: Item) {
        self.spatial_index.insert(&item);
        self.items.push(item);
    }

    /// Rimuove un elemento dalla pagina tramite il suo ItemId, aggiornando l'indice spaziale.
    pub fn remove_item(&mut self, id: ItemId) -> Option<Item> {
        if let Some(pos) = self.items.iter().position(|it| it.id == id) {
            let item = self.items.remove(pos);
            self.spatial_index.remove_item(&item);
            Some(item)
        } else {
            None
        }
    }

    /// Trova un riferimento all'elemento con l'ID specificato.
    pub fn item(&self, id: ItemId) -> Option<&Item> {
        self.items.iter().find(|it| it.id == id)
    }

    /// Trova un riferimento mutabile all'elemento con l'ID specificato.
    pub fn item_mut(&mut self, id: ItemId) -> Option<&mut Item> {
        self.items.iter_mut().find(|it| it.id == id)
    }

    /// Itera su tutti i tratti presenti nella pagina.
    pub fn strokes(&self) -> impl Iterator<Item = &Stroke> {
        self.items.iter().filter_map(|it| it.as_stroke())
    }

    /// Itera su tutte le immagini presenti nella pagina.
    pub fn images(&self) -> impl Iterator<Item = &ImageItem> {
        self.items.iter().filter_map(|it| it.as_image())
    }

    /// Ricostruisce l'indice spaziale 2D R-Tree a partire da tutti gli elementi della pagina.
    pub fn rebuild_spatial_index(&mut self) {
        self.spatial_index = PageSpatialIndex::bulk_load(&self.items);
    }
}

/// Identificatore univoco di un elemento grafico.
pub type ItemId = u64;

/// Rappresentazione di un'immagine raster posizionata su una pagina di UniNote.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageItem {
    pub id: Uuid,
    pub page_id: Uuid,
    pub asset_hash: String,
    pub bbox: Rect,
    pub z_index: i64,
    pub opacity: f64,
    pub original_width: u32,
    pub original_height: u32,
}

impl ImageItem {
    /// Costruisce una nuova istanza con ID autogenerato, opacità predefinita (1.0) e z_index (0).
    pub fn new(
        page_id: Uuid,
        asset_hash: impl Into<String>,
        bbox: Rect,
        original_width: u32,
        original_height: u32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            page_id,
            asset_hash: asset_hash.into(),
            bbox,
            z_index: 0,
            opacity: 1.0,
            original_width,
            original_height,
        }
    }

    /// Costruttore completo con tutti i parametri espliciti.
    #[allow(clippy::too_many_arguments)]
    pub fn with_id(
        id: Uuid,
        page_id: Uuid,
        asset_hash: impl Into<String>,
        bbox: Rect,
        z_index: i64,
        opacity: f64,
        original_width: u32,
        original_height: u32,
    ) -> Self {
        let opacity = if opacity.is_finite() {
            opacity.clamp(0.0, 1.0)
        } else {
            1.0
        };
        Self {
            id,
            page_id,
            asset_hash: asset_hash.into(),
            bbox,
            z_index,
            opacity,
            original_width,
            original_height,
        }
    }

    /// Larghezza corrente in punti del bounding box.
    #[inline]
    pub fn width(&self) -> f64 {
        self.bbox.width()
    }

    /// Altezza corrente in punti del bounding box.
    #[inline]
    pub fn height(&self) -> f64 {
        self.bbox.height()
    }

    /// Rapporto d'aspetto originale (larghezza / altezza).
    #[inline]
    pub fn aspect_ratio(&self) -> f64 {
        if self.original_width > 0 && self.original_height > 0 {
            self.original_width as f64 / self.original_height as f64
        } else {
            let norm = self.bbox.normalized();
            if norm.width() > 0.0 && norm.height() > 0.0 {
                let ratio = norm.width() / norm.height();
                if ratio > 0.0 && ratio.is_finite() {
                    ratio
                } else {
                    1.0
                }
            } else {
                1.0
            }
        }
    }

    /// Trasla il bounding box in-place con zero allocazioni heap.
    #[inline]
    pub fn translate(&mut self, dx: f64, dy: f64) {
        if !dx.is_finite() || !dy.is_finite() {
            return;
        }
        self.bbox = self.bbox.translate(dx, dy);
    }

    /// Scala il bounding box rispetto al perno (origin_x, origin_y).
    pub fn scale(&mut self, sx: f64, sy: f64, origin_x: f64, origin_y: f64) {
        if !sx.is_finite() || !sy.is_finite() || !origin_x.is_finite() || !origin_y.is_finite() {
            return;
        }
        let x1 = origin_x + (self.bbox.min_x - origin_x) * sx;
        let x2 = origin_x + (self.bbox.max_x - origin_x) * sx;
        let y1 = origin_y + (self.bbox.min_y - origin_y) * sy;
        let y2 = origin_y + (self.bbox.max_y - origin_y) * sy;
        self.bbox = Rect::new(x1.min(x2), y1.min(y2), x1.max(x2), y1.max(y2));
    }

    /// Scala proporzionale uniforme attorno al perno preservando il rapporto d'aspetto.
    #[inline]
    pub fn scale_proportional(&mut self, scale: f64, origin_x: f64, origin_y: f64) {
        self.scale(scale, scale, origin_x, origin_y);
    }

    /// Imposta l'opacità con saturazione sicura in [0.0, 1.0].
    #[inline]
    pub fn set_opacity(&mut self, opacity: f64) {
        if opacity.is_finite() {
            self.opacity = opacity.clamp(0.0, 1.0);
        }
    }
}

/// Elemento grafico presente su una pagina.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub z_index: u64,
    pub kind: ItemKind,
}

impl Item {
    pub fn new_stroke(id: ItemId, z_index: u64, stroke: Stroke) -> Self {
        Self {
            id,
            z_index,
            kind: ItemKind::Stroke(stroke),
        }
    }

    pub fn new_image(id: ItemId, z_index: u64, image: ImageItem) -> Self {
        Self {
            id,
            z_index,
            kind: ItemKind::Image(image),
        }
    }

    /// Restituisce il bounding box 2D dell'elemento a prescindere dal tipo.
    #[inline]
    pub fn bounding_box(&self) -> Option<Rect> {
        self.kind.bounding_box()
    }

    /// Trasla l'elemento in-place.
    #[inline]
    pub fn translate(&mut self, dx: f64, dy: f64) {
        self.kind.translate(dx, dy);
    }

    /// Scala l'elemento rispetto al perno indicato.
    #[inline]
    pub fn scale(&mut self, sx: f64, sy: f64, origin_x: f64, origin_y: f64) {
        self.kind.scale(sx, sy, origin_x, origin_y);
    }

    #[inline]
    pub fn as_stroke(&self) -> Option<&Stroke> {
        self.kind.as_stroke()
    }

    #[inline]
    pub fn as_stroke_mut(&mut self) -> Option<&mut Stroke> {
        self.kind.as_stroke_mut()
    }

    #[inline]
    pub fn as_image(&self) -> Option<&ImageItem> {
        self.kind.as_image()
    }

    #[inline]
    pub fn as_image_mut(&mut self) -> Option<&mut ImageItem> {
        self.kind.as_image_mut()
    }

    #[inline]
    pub fn is_stroke(&self) -> bool {
        matches!(self.kind, ItemKind::Stroke(_))
    }

    #[inline]
    pub fn is_image(&self) -> bool {
        matches!(self.kind, ItemKind::Image(_))
    }
}

/// Tipologia di contenuto di un Item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ItemKind {
    Stroke(Stroke),
    Image(ImageItem),
}

impl ItemKind {
    /// Restituisce il bounding box 2D dell'elemento a prescindere dal tipo.
    pub fn bounding_box(&self) -> Option<Rect> {
        match self {
            ItemKind::Stroke(s) => s.bounding_box(),
            ItemKind::Image(img) => Some(img.bbox),
        }
    }

    /// Trasla il contenuto dell'elemento in-place.
    pub fn translate(&mut self, dx: f64, dy: f64) {
        match self {
            ItemKind::Stroke(s) => s.translate(dx, dy),
            ItemKind::Image(img) => img.translate(dx, dy),
        }
    }

    /// Scala il contenuto dell'elemento rispetto al perno indicato.
    pub fn scale(&mut self, sx: f64, sy: f64, origin_x: f64, origin_y: f64) {
        match self {
            ItemKind::Stroke(s) => s.scale(sx, sy, origin_x, origin_y),
            ItemKind::Image(img) => img.scale(sx, sy, origin_x, origin_y),
        }
    }

    #[inline]
    pub fn as_stroke(&self) -> Option<&Stroke> {
        match self {
            ItemKind::Stroke(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_stroke_mut(&mut self) -> Option<&mut Stroke> {
        match self {
            ItemKind::Stroke(s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn as_image(&self) -> Option<&ImageItem> {
        match self {
            ItemKind::Image(img) => Some(img),
            _ => None,
        }
    }

    #[inline]
    pub fn as_image_mut(&mut self) -> Option<&mut ImageItem> {
        match self {
            ItemKind::Image(img) => Some(img),
            _ => None,
        }
    }
}

/// Documento completo UniNote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub title: String,
    pub version: u32,
    #[serde(default)]
    pub format: DocumentFormat,
    pub pages: Vec<Page>,
}

impl Document {
    pub fn new_empty(title: impl Into<String>) -> Self {
        Self::new_notebook(title)
    }

    pub fn new_notebook(title: impl Into<String>) -> Self {
        let mut doc = Self {
            id: Uuid::new_v4(),
            title: title.into(),
            version: 1,
            format: DocumentFormat::Notebook,
            pages: Vec::new(),
        };
        // Aggiunge di default la prima pagina A4
        doc.pages.push(Page::new_a4(0, PageBackground::default()));
        doc
    }

    pub fn new_whiteboard(background: PageBackground) -> Self {
        Self::new_whiteboard_with_title("Lavagna Senza Titolo", background)
    }

    pub fn new_whiteboard_with_title(title: impl Into<String>, background: PageBackground) -> Self {
        let mut doc = Self {
            id: Uuid::new_v4(),
            title: title.into(),
            version: 1,
            format: DocumentFormat::Whiteboard,
            pages: Vec::new(),
        };
        doc.pages.push(Page::new_whiteboard(background));
        doc
    }

    pub fn get_page(&self, page_id: Uuid) -> Option<&Page> {
        self.pages.iter().find(|p| p.id == page_id)
    }

    pub fn get_page_mut(&mut self, page_id: Uuid) -> Option<&mut Page> {
        self.pages.iter_mut().find(|p| p.id == page_id)
    }

    pub fn add_page(&mut self, background: PageBackground) -> Uuid {
        let index = self.pages.len();
        let page = Page::new_a4(index, background);
        let id = page.id;
        self.pages.push(page);
        id
    }

    /// Calcola il rettangolo globale di una pagina considerando altezze eterogenee.
    pub fn page_rect(&self, page_index: usize) -> Option<Rect2D> {
        if page_index >= self.pages.len() {
            return None;
        }
        let mut top = crate::coordinates::PAGE_TOP_MARGIN_PT;
        for i in 0..page_index {
            top += self.pages[i].height + crate::coordinates::PAGE_GAP_PT;
        }
        let page = &self.pages[page_index];
        Some(Rect2D::new(0.0, top, page.width, page.height))
    }

    /// Trova l'indice della pagina alla coordinata Y del documento con altezze eterogenee.
    pub fn find_page_index_at_y(&self, doc_y: f64) -> Option<usize> {
        if self.pages.is_empty() {
            return None;
        }
        let mut top = crate::coordinates::PAGE_TOP_MARGIN_PT;
        for (i, page) in self.pages.iter().enumerate() {
            let next_top = top + page.height + crate::coordinates::PAGE_GAP_PT;
            if doc_y < next_top || i == self.pages.len() - 1 {
                return Some(i);
            }
            top = next_top;
        }
        Some(self.pages.len() - 1)
    }

    /// Aggiunge una pagina PDF in coda al documento.
    pub fn add_pdf_page(
        &mut self,
        source_path: impl Into<String>,
        page_index: usize,
        width: f64,
        height: f64,
    ) -> Uuid {
        let index = self.pages.len();
        let page = Page::new_pdf_page(index, source_path, page_index, width, height);
        let id = page.id;
        self.pages.push(page);
        id
    }

    /// Restituisce il valore massimo di ItemId presente in tutte le pagine del documento,
    /// oppure 0 se il documento non contiene ancora elementi.
    pub fn max_item_id(&self) -> u64 {
        self.pages
            .iter()
            .flat_map(|p| p.items.iter().map(|item| item.id))
            .max()
            .unwrap_or(0)
    }

    /// Ricostruisce gli indici spaziali 2D R-Tree per tutte le pagine del documento.
    pub fn rebuild_spatial_indices(&mut self) {
        for page in &mut self.pages {
            page.rebuild_spatial_index();
        }
    }
}
