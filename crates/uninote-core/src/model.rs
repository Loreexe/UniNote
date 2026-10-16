use crate::coordinates::Rect2D;
use serde::{Deserialize, Serialize};
use uninote_stroke::Stroke;
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
}

impl Page {
    pub fn new_a4(index: usize, background: PageBackground) -> Self {
        Self {
            id: Uuid::new_v4(),
            index,
            width: A4_WIDTH_PT,
            height: A4_HEIGHT_PT,
            background,
            items: Vec::new(),
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
        }
    }
}

/// Identificatore univoco di un elemento grafico.
pub type ItemId = u64;

/// Elemento grafico presente su una pagina.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: ItemId,
    pub z_index: u64,
    pub kind: ItemKind,
}

/// Tipologia di contenuto di un Item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ItemKind {
    Stroke(Stroke),
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
}
