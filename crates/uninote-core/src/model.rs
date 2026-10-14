use serde::{Deserialize, Serialize};
use uninote_stroke::Stroke;
use uuid::Uuid;

/// Dimensioni standard foglio A4 in punti tipografici (1 pt = 1/72 pollice).
/// 210 mm x 297 mm = 595.28 pt x 841.89 pt.
pub const A4_WIDTH_PT: f64 = 595.28;
pub const A4_HEIGHT_PT: f64 = 841.89;

/// Sfondo della pagina con parametri grafici.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageBackground {
    /// Foglio bianco caldo naturale.
    Blank,
    /// Foglio a righe orizzontali (interlinea tipica 8 mm ≈ 22.68 pt).
    Ruled { line_spacing: f64, margin_left: f64 },
    /// Foglio a quadretti (quadratura tipica 5 mm ≈ 14.17 pt).
    Grid { grid_size: f64 },
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
}

/// Elemento grafico presente su una pagina.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Item {
    pub id: u64,
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
    pub pages: Vec<Page>,
}

impl Document {
    pub fn new_empty(title: impl Into<String>) -> Self {
        let mut doc = Self {
            id: Uuid::new_v4(),
            title: title.into(),
            version: 1,
            pages: Vec::new(),
        };
        // Aggiunge di default la prima pagina A4
        doc.pages.push(Page::new_a4(0, PageBackground::default()));
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
}
