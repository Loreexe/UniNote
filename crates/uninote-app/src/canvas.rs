use crate::hud::DiagnosticHud;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use uninote_core::{
    CommandHistory, DocCommand, Document, DocumentFormat, ItemId, ItemKind, PageBackground,
    Viewport,
};
use uninote_storage::{DocumentStore, SaveStatus, StorageWorker};
use uninote_stroke::selection::{SelectionEngine, SelectionMode};
use uninote_stroke::{
    DeviceKind, InputDisposition, InputPhase, InputSample, PalmRejectionEngine, PipelineMode,
    RecordedSession, Rect, Stroke, StrokeBuilder, StrokeStyle, TouchMode,
};

/// Specifica dinamica procedurale della griglia Cairo in base al livello di zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DynamicGridSpec {
    pub step_pt: f64,
    pub alpha: f64,
    pub major_step_pt: f64,
}

/// Calcola i parametri ottimali della griglia procedurale in base al fattore di ingrandimento dello schermo.
pub fn dynamic_grid_spec(zoom: f64) -> DynamicGridSpec {
    let clamped_zoom = if zoom.is_finite() {
        zoom.clamp(0.25, 5.0)
    } else {
        1.0
    };
    let base_step = 20.0;

    if clamped_zoom < 0.5 {
        let t = (clamped_zoom - 0.25) / 0.25; // 0.0 a 0.25, 1.0 a 0.5
        DynamicGridSpec {
            step_pt: base_step * 2.0, // 40.0 pt
            alpha: 0.15 + 0.15 * t,
            major_step_pt: base_step * 8.0,
        }
    } else if clamped_zoom <= 2.0 {
        DynamicGridSpec {
            step_pt: base_step, // 20.0 pt
            alpha: 0.25,
            major_step_pt: base_step * 5.0, // 100.0 pt
        }
    } else {
        let t = ((clamped_zoom - 2.0) / 3.0).min(1.0); // 0.0 a 2.0, 1.0 a 5.0
        DynamicGridSpec {
            step_pt: base_step / 2.0, // 10.0 pt
            alpha: 0.15 + 0.20 * t,
            major_step_pt: base_step,
        }
    }
}

/// Strumento di disegno/modifica attualmente selezionato.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveTool {
    Pen,
    Eraser,
    Highlighter,
    Text,
    Line,
    Shapes,
    Select,
}

/// Stato della selezione interattiva su canvas (Lazo libero e Rettangolo diagonale).
#[derive(Debug, Clone)]
pub struct SelectionState {
    /// Modalità attiva (Lasso a mano libera o Rettangolo diagonale).
    pub mode: SelectionMode,
    /// Indice della pagina a cui appartengono gli elementi selezionati.
    pub page_index: Option<usize>,
    /// Identificatori degli elementi selezionati (Item::id).
    pub selected_ids: Vec<ItemId>,
    /// Bounding box complessivo dei tratti selezionati in coordinate documento.
    pub bbox: Option<Rect>,
    /// Traccia poligonale del lazo in corso di disegno (coordinate documento).
    pub active_lasso: Vec<(f64, f64)>,
    /// Punto iniziale del rettangolo di selezione in corso (coordinate documento).
    pub active_rect_start: Option<(f64, f64)>,
    /// Punto corrente del rettangolo di selezione in corso (coordinate documento).
    pub active_rect_current: Option<(f64, f64)>,
    /// Flag che indica se è in corso un'operazione di trascinamento (drag translation).
    pub is_dragging: bool,
    /// Punto iniziale del drag in coordinate documento.
    pub drag_start_doc: Option<(f64, f64)>,
    /// Offset corrente di trascinamento (delta_x, delta_y) in coordinate documento.
    pub drag_offset: (f64, f64),
}

impl Default for SelectionState {
    fn default() -> Self {
        Self {
            mode: SelectionMode::Lasso,
            page_index: None,
            selected_ids: Vec::new(),
            bbox: None,
            active_lasso: Vec::new(),
            active_rect_start: None,
            active_rect_current: None,
            is_dragging: false,
            drag_start_doc: None,
            drag_offset: (0.0, 0.0),
        }
    }
}

impl SelectionState {
    pub fn has_selection(&self) -> bool {
        !self.selected_ids.is_empty() && self.bbox.is_some()
    }

    pub fn is_selecting(&self) -> bool {
        !self.active_lasso.is_empty() || self.active_rect_start.is_some()
    }

    pub fn clear(&mut self) {
        self.page_index = None;
        self.selected_ids.clear();
        self.bbox = None;
        self.active_lasso.clear();
        self.active_rect_start = None;
        self.active_rect_current = None;
        self.is_dragging = false;
        self.drag_start_doc = None;
        self.drag_offset = (0.0, 0.0);
    }

    pub fn hit_test_handle_or_bbox(&self, doc_pt: (f64, f64), tolerance: f64) -> bool {
        if let Some(ref bbox) = self.bbox {
            let current_bbox = bbox.translate(self.drag_offset.0, self.drag_offset.1);
            let expanded = current_bbox.expand(tolerance);
            expanded.contains_point(doc_pt.0, doc_pt.1)
        } else {
            false
        }
    }
}

/// Stato interno condiviso del canvas di UniNote.
pub struct CanvasState {
    pub document: Document,
    pub history: CommandHistory,
    pub storage_worker: Option<StorageWorker>,
    pub file_path: Option<PathBuf>,
    pub save_status: SaveStatus,
    pub viewport: Viewport,
    pub active_tool: ActiveTool,
    pub selection: SelectionState,
    pub active_builder: Option<StrokeBuilder>,
    pub active_device: Option<DeviceKind>,
    pub palm_engine: PalmRejectionEngine,
    pub pipeline_mode: PipelineMode,
    pub current_style: StrokeStyle,
    pub next_stroke_id: u64,
    pub recorder: Option<RecordedSession>,
    pub last_sample_time: Option<Instant>,
    pub last_stylus_time: Option<Instant>,
    pub mouse_drag_start: Option<(f64, f64)>,
    pub mouse_pan_start: Option<(f64, f64)>,
    pub tip_pressure_threshold: f64,
    pub pdf_cache: std::cell::RefCell<crate::pdf_cache::PdfCacheManager>,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            document: Document::new_empty("Quaderno Senza Titolo"),
            history: CommandHistory::new(100),
            storage_worker: None,
            file_path: None,
            save_status: SaveStatus::Saved(0),
            viewport: Viewport::default(),
            active_tool: ActiveTool::Pen,
            selection: SelectionState::default(),
            active_builder: None,
            active_device: None,
            palm_engine: PalmRejectionEngine::new(TouchMode::GesturesOnly),
            pipeline_mode: PipelineMode::CatmullRomSpline,
            current_style: StrokeStyle::default(),
            next_stroke_id: 1,
            recorder: None,
            last_sample_time: None,
            last_stylus_time: None,
            mouse_drag_start: None,
            mouse_pan_start: None,
            tip_pressure_threshold: 0.06,
            pdf_cache: std::cell::RefCell::new(crate::pdf_cache::PdfCacheManager::default_120hz()),
        }
    }

    pub fn undo(&mut self) -> bool {
        let res = self.history.undo(&mut self.document);
        if res {
            self.persist_changes();
            self.selection.clear();
        }
        res
    }

    pub fn redo(&mut self) -> bool {
        let res = self.history.redo(&mut self.document);
        if res {
            self.persist_changes();
            self.selection.clear();
        }
        res
    }

    pub fn add_page(&mut self, page: uninote_core::Page) {
        self.history
            .execute(DocCommand::AddPage { page }, &mut self.document);
        self.persist_changes();
    }

    #[allow(dead_code)]
    pub fn clear_current_page(&mut self) {
        if let Some(page) = self.document.pages.first_mut() {
            let erased = page.items.clone();
            if !erased.is_empty() {
                let page_id = page.id;
                self.history.execute(
                    DocCommand::EraseItems {
                        page_id,
                        erased_items: erased,
                    },
                    &mut self.document,
                );
                self.persist_changes();
            }
        }
        self.active_builder = None;
        self.active_device = None;
        self.mouse_drag_start = None;
    }

    pub fn mark_saved(&mut self) {
        let rev = self.history.current_revision;
        self.history.mark_saved(rev);
        self.save_status = SaveStatus::Saved(rev);
    }

    /// Invia la revisione corrente al worker di storage asincrono (se collegato).
    pub fn persist_changes(&mut self) {
        let rev = self.history.current_revision;
        self.save_status = SaveStatus::Saving(rev);
        if let Some(ref worker) = self.storage_worker {
            worker.save_revision(self.document.clone(), rev);
        }
    }

    #[allow(dead_code)]
    pub fn save_status(&self) -> SaveStatus {
        self.save_status.clone()
    }
}

/// Adatta il colore a video in base al tema scuro/chiaro della pagina senza mutare il dato memorizzato.
pub fn adapt_ink_color(color: (f64, f64, f64, f64), is_dark_sheet: bool) -> (f64, f64, f64, f64) {
    let (r, g, b, a) = color;
    // Calcolo luminanza percettiva standard Rec. 709
    let lum = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let chroma = max_c - min_c;

    if is_dark_sheet {
        if lum < 0.35 {
            if chroma < 0.12 {
                // Inchiostro scuro neutro (nero, grafite) -> Bianco ghiaccio (#ECEFF4)
                (0.925, 0.937, 0.957, a)
            } else {
                // Inchiostro scuro cromatico (navy, bordeaux, verde bosco) -> Tone lift pastello
                let lift = ((0.75 - lum) / (1.0 - lum).max(0.01)).clamp(0.0, 0.85);
                (
                    (r + (1.0 - r) * lift).clamp(0.0, 1.0),
                    (g + (1.0 - g) * lift).clamp(0.0, 1.0),
                    (b + (1.0 - b) * lift).clamp(0.0, 1.0),
                    a,
                )
            }
        } else {
            // Inchiostri già vividi (ciano, menta, oro, evidenziatori fluorescenti)
            (r, g, b, a)
        }
    } else {
        // Sul foglio bianco chiaro: se un tratto era bianco (#FFFFFF / off-white), inverti a grafite scura
        if lum > 0.85 && chroma < 0.12 {
            (0.090, 0.129, 0.149, a) // #172126
        } else {
            (r, g, b, a)
        }
    }
}

pub struct NoteCanvas {
    area: DrawingArea,
    state: Rc<RefCell<CanvasState>>,
    hud: Rc<DiagnosticHud>,
    app_start: Instant,
}

impl NoteCanvas {
    pub fn new(hud: Rc<DiagnosticHud>) -> Self {
        let area = DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .build();

        let area_weak = area.downgrade();
        libadwaita::StyleManager::default().connect_dark_notify(move |_| {
            if let Some(a) = area_weak.upgrade() {
                a.queue_draw();
            }
        });

        let state = Rc::new(RefCell::new(CanvasState::new()));
        let app_start = Instant::now();

        let canvas = Self {
            area,
            state,
            hud,
            app_start,
        };
        canvas.setup_drawing();
        canvas.setup_capture_controller();
        canvas.setup_stylus_controller();
        canvas.setup_mouse_controller();
        canvas.setup_scroll_and_zoom_controllers();
        canvas
    }

    pub fn widget(&self) -> &DrawingArea {
        &self.area
    }

    pub fn state(&self) -> Rc<RefCell<CanvasState>> {
        self.state.clone()
    }

    pub fn set_active_tool(&self, tool: ActiveTool) {
        let mut st = self.state.borrow_mut();
        st.active_tool = tool;
        if tool != ActiveTool::Select {
            st.selection.clear();
        }
        let tool_name = match tool {
            ActiveTool::Pen => "Penna",
            ActiveTool::Eraser => "Gomma (Tratto Intero)",
            ActiveTool::Highlighter => "Evidenziatore",
            ActiveTool::Text => "Testo",
            ActiveTool::Line => "Linea Dritta",
            ActiveTool::Shapes => "Forme",
            ActiveTool::Select => "Selezione (Lazo / Rettangolo)",
        };
        self.hud
            .update_metrics(|m| m.current_tool = tool_name.to_string());
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn selection_mode(&self) -> SelectionMode {
        self.state.borrow().selection.mode
    }

    pub fn set_selection_mode(&self, mode: SelectionMode) {
        let mut st = self.state.borrow_mut();
        st.selection.mode = mode;
        self.area.queue_draw();
    }

    pub fn clear_selection(&self) {
        let mut st = self.state.borrow_mut();
        st.selection.clear();
        drop(st);
        self.area.queue_draw();
    }

    pub fn delete_selected(&self) -> bool {
        let mut st = self.state.borrow_mut();
        let deleted = Self::delete_selected_items(&mut st);
        drop(st);
        if deleted {
            self.area.queue_draw();
        }
        deleted
    }

    pub fn set_pen_color(&self, color: (f64, f64, f64, f64)) {
        let mut st = self.state.borrow_mut();
        st.current_style.color = color;
    }

    pub fn set_pen_width(&self, width: f64) {
        let mut st = self.state.borrow_mut();
        st.current_style.base_width = width;
    }

    pub fn undo(&self) -> bool {
        let mut st = self.state.borrow_mut();
        let res = st.undo();
        if res {
            drop(st);
            self.area.queue_draw();
        }
        res
    }

    pub fn redo(&self) -> bool {
        let mut st = self.state.borrow_mut();
        let res = st.redo();
        if res {
            drop(st);
            self.area.queue_draw();
        }
        res
    }

    /// Calcola il bounding box complessivo (AABB) di tutti i tratti del documento.
    pub fn compute_document_bounding_box(
        doc: &Document,
    ) -> Option<uninote_core::coordinates::Rect2D> {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        let mut has_strokes = false;

        for page in &doc.pages {
            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if let Some(bbox) = s.bounding_box() {
                    min_x = min_x.min(bbox.min_x);
                    min_y = min_y.min(bbox.min_y);
                    max_x = max_x.max(bbox.max_x);
                    max_y = max_y.max(bbox.max_y);
                    has_strokes = true;
                }
            }
        }

        if has_strokes
            && min_x.is_finite()
            && min_y.is_finite()
            && max_x.is_finite()
            && max_y.is_finite()
        {
            Some(uninote_core::coordinates::Rect2D::new(
                min_x,
                min_y,
                max_x - min_x,
                max_y - min_y,
            ))
        } else {
            None
        }
    }

    /// Inquadra e centra tutti i contenuti disegnati nella lavagna o nel quaderno.
    /// Calcola l'AABB di tutti i tratti e adatta zoom e pan del viewport.
    pub fn center_content(&self) {
        let mut st = self.state.borrow_mut();
        let win_w = self.area.width() as f64;
        let win_h = self.area.height() as f64;

        let bbox = Self::compute_document_bounding_box(&st.document);
        if let Some(rect) = bbox {
            st.viewport.center_on_rect(rect, win_w, win_h);
        } else {
            // Se non ci sono tratti, reimposta la vista al centro (0, 0)
            if st.document.format == DocumentFormat::Whiteboard {
                st.viewport.center_on_rect(
                    uninote_core::coordinates::Rect2D::new(0.0, 0.0, 0.0, 0.0),
                    win_w,
                    win_h,
                );
            } else {
                st.viewport.reset_zoom_and_center(win_w);
            }
        }

        let pct = (st.viewport.zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn add_page(&self, bg: PageBackground) {
        let mut st = self.state.borrow_mut();
        if st.document.format == DocumentFormat::Whiteboard {
            return;
        }
        let idx = st.document.pages.len();
        let new_page = uninote_core::Page::new_a4(idx, bg);
        st.add_page(new_page);
        let count = st.document.pages.len();
        let page_rect = Viewport::page_rect(idx);
        st.viewport.offset_y = -page_rect.y * st.viewport.zoom + 40.0;
        drop(st);
        self.hud.update_metrics(|m| m.page_count = count);
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn zoom_in(&self) {
        let mut st = self.state.borrow_mut();
        let w = self.area.width() as f64;
        let h = self.area.height() as f64;
        st.viewport.zoom_at((w / 2.0, h / 2.0), 1.25);
        let pct = (st.viewport.zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn zoom_out(&self) {
        let mut st = self.state.borrow_mut();
        let w = self.area.width() as f64;
        let h = self.area.height() as f64;
        st.viewport.zoom_at((w / 2.0, h / 2.0), 0.8);
        let pct = (st.viewport.zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn reset_zoom(&self) {
        let mut st = self.state.borrow_mut();
        let w = self.area.width() as f64;
        let h = self.area.height() as f64;
        if st.document.format == DocumentFormat::Whiteboard {
            let bbox = Self::compute_document_bounding_box(&st.document);
            if let Some(rect) = bbox {
                st.viewport.center_on_rect(rect, w, h);
            } else {
                st.viewport.center_on_rect(
                    uninote_core::coordinates::Rect2D::new(0.0, 0.0, 0.0, 0.0),
                    w,
                    h,
                );
            }
        } else {
            st.viewport.reset_zoom_and_center(w);
        }
        let pct = (st.viewport.zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.area.queue_draw();
    }

    pub fn total_pages(&self) -> usize {
        let st = self.state.borrow();
        if st.document.format == DocumentFormat::Whiteboard {
            return 1;
        }
        st.document.pages.len().max(1)
    }

    pub fn current_page_index(&self) -> usize {
        let st = self.state.borrow();
        if st.document.format == DocumentFormat::Whiteboard {
            return 0;
        }
        let center_screen_y = (self.area.height() as f64) / 2.0;
        let (_doc_x, doc_y) = st.viewport.screen_to_document((0.0, center_screen_y));
        for (i, _) in st.document.pages.iter().enumerate() {
            let rect = Viewport::page_rect(i);
            if doc_y >= rect.y && doc_y <= rect.y + rect.height {
                return i;
            }
        }
        0
    }

    pub fn scroll_to_page(&self, page_idx: usize) {
        let mut st = self.state.borrow_mut();
        if st.document.format == DocumentFormat::Whiteboard {
            return;
        }
        let count = st.document.pages.len();
        if page_idx < count {
            let page_rect = Viewport::page_rect(page_idx);
            st.viewport.offset_y = -page_rect.y * st.viewport.zoom + 40.0;
            drop(st);
            self.area.queue_draw();
        }
    }

    pub fn set_current_page_background(&self, bg: PageBackground) {
        let mut st = self.state.borrow_mut();
        let cur_idx = if st.document.format == DocumentFormat::Whiteboard {
            0
        } else {
            let center_screen_y = (self.area.height() as f64) / 2.0;
            let (_doc_x, doc_y) = st.viewport.screen_to_document((0.0, center_screen_y));
            let mut found = 0;
            for (i, _) in st.document.pages.iter().enumerate() {
                let rect = Viewport::page_rect(i);
                if doc_y >= rect.y && doc_y <= rect.y + rect.height {
                    found = i;
                    break;
                }
            }
            found
        };
        if let Some(page) = st.document.pages.get_mut(cur_idx) {
            page.background = bg;
            st.persist_changes();
        }
        drop(st);
        self.area.queue_draw();
    }

    pub fn document_format(&self) -> DocumentFormat {
        self.state.borrow().document.format
    }

    pub fn document_title(&self) -> String {
        self.state.borrow().document.title.clone()
    }

    pub fn file_path(&self) -> Option<PathBuf> {
        self.state.borrow().file_path.clone()
    }

    pub fn persist_changes(&self) {
        self.state.borrow_mut().persist_changes();
    }

    pub fn new_document(&self) {
        let mut st = self.state.borrow_mut();
        st.document = Document::new_empty("Nuovo Quaderno");
        st.history = CommandHistory::default();
        st.storage_worker = None;
        st.file_path = None;
        st.active_builder = None;
        st.active_device = None;
        st.active_tool = ActiveTool::Pen;
        st.mouse_drag_start = None;
        st.mouse_pan_start = None;
        st.last_sample_time = None;
        st.last_stylus_time = None;
        st.palm_engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
        st.viewport = Viewport::default();
        drop(st);

        self.hud.update_metrics(|m| {
            m.current_tool = "Penna".into();
            m.page_count = 1;
            m.zoom_pct = 100;
            m.touch_mode = TouchMode::GesturesOnly;
        });
        self.hud.update_view();
        self.area.queue_draw();
    }

    /// Crea un nuovo documento con titolo, formato e sfondo specificati.
    pub fn create_document(&self, title: &str, format: DocumentFormat, bg: PageBackground) {
        let mut st = self.state.borrow_mut();
        st.document = match format {
            DocumentFormat::Notebook => {
                let mut doc = Document::new_notebook(title);
                if let Some(first_page) = doc.pages.first_mut() {
                    first_page.background = bg;
                }
                doc
            }
            DocumentFormat::Whiteboard => Document::new_whiteboard_with_title(title, bg),
        };
        st.history = CommandHistory::default();
        st.storage_worker = None;
        st.file_path = None;
        st.active_builder = None;
        st.active_device = None;
        st.active_tool = ActiveTool::Pen;
        st.mouse_drag_start = None;
        st.mouse_pan_start = None;
        st.last_sample_time = None;
        st.last_stylus_time = None;
        st.palm_engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
        st.viewport = Viewport::default();
        drop(st);

        self.hud.update_metrics(|m| {
            m.current_tool = "Penna".into();
            m.page_count = 1;
            m.zoom_pct = 100;
            m.touch_mode = TouchMode::GesturesOnly;
        });
        self.hud.update_view();
        self.area.queue_draw();
    }

    #[allow(dead_code)]
    pub fn open_file(&self, path: PathBuf) -> Result<(), String> {
        let store = DocumentStore::open_or_create(&path).map_err(|e| e.to_string())?;
        let doc = store.load_document().map_err(|e| e.to_string())?;
        let mut st = self.state.borrow_mut();
        st.document = doc;
        st.history = CommandHistory::new(100);
        st.file_path = Some(path.clone());

        // Inizializza worker di storage asincrono per questo file
        if let Ok(worker) = StorageWorker::spawn(path, move |_status| {
            glib::idle_add_once(move || {
                // Notifica asincrona di cambio stato salvataggio
            });
        }) {
            st.storage_worker = Some(worker);
        }

        let page_count = st.document.pages.len();
        drop(st);
        self.hud.update_metrics(|m| m.page_count = page_count);
        self.hud.update_view();
        self.area.queue_draw();
        Ok(())
    }

    pub fn save_as(&self, path: PathBuf) -> Result<(), String> {
        let mut st = self.state.borrow_mut();
        let mut store = DocumentStore::open_or_create(&path).map_err(|e| e.to_string())?;
        store
            .save_document(&st.document)
            .map_err(|e| e.to_string())?;
        st.file_path = Some(path.clone());

        if let Ok(worker) = StorageWorker::spawn(path, move |_status| {
            glib::idle_add_once(move || {
                // Notifica asincrona di cambio stato salvataggio
            });
        }) {
            st.storage_worker = Some(worker);
        }
        st.mark_saved();
        self.area.queue_draw();
        Ok(())
    }

    #[allow(dead_code)]
    pub fn export_portable_copy(&self, target_path: PathBuf) -> Result<(), String> {
        let st = self.state.borrow();
        if let Some(ref path) = st.file_path {
            let store = DocumentStore::open_or_create(path).map_err(|e| e.to_string())?;
            uninote_storage::export_portable_copy(store.connection(), &target_path)
                .map_err(|e| e.to_string())?;
            Ok(())
        } else {
            let mut store =
                DocumentStore::open_or_create(&target_path).map_err(|e| e.to_string())?;
            store
                .save_document(&st.document)
                .map_err(|e| e.to_string())?;
            Ok(())
        }
    }

    /// Esporta il documento corrente in un file PDF vettoriale ad alta definizione.
    pub fn export_pdf(
        &self,
        target_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let st = self.state.borrow();
        crate::pdf_export::export_to_pdf(&st.document, target_path)
    }

    /// Carica un nuovo documento sostituendo lo stato corrente e aggiornando l'HUD e la vista.
    pub fn load_document(&self, doc: uninote_core::Document) {
        let mut st = self.state.borrow_mut();
        st.document = doc;
        st.history = CommandHistory::new(100);
        st.storage_worker = None;
        st.file_path = None;
        st.active_builder = None;
        st.active_device = None;
        st.active_tool = ActiveTool::Pen;
        st.mouse_drag_start = None;
        st.mouse_pan_start = None;
        st.last_sample_time = None;
        st.last_stylus_time = None;
        st.palm_engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
        st.viewport = Viewport::default();
        st.selection.clear();
        drop(st);

        let count = self.total_pages();
        self.hud.update_metrics(|m| {
            m.current_tool = "Penna".into();
            m.page_count = count;
            m.zoom_pct = 100;
            m.touch_mode = TouchMode::GesturesOnly;
        });
        self.hud.update_view();
        self.area.queue_draw();
    }

    /// Crea un nuovo quaderno a partire da un file PDF esterno.
    pub fn create_from_pdf(
        &self,
        pdf_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let doc = crate::pdf_export::import_from_pdf(pdf_path)?;
        self.load_document(doc);
        Ok(())
    }

    /// Accoda le pagine di un file PDF in fondo al documento corrente.
    pub fn append_pdf(
        &self,
        pdf_path: &std::path::Path,
    ) -> Result<usize, Box<dyn std::error::Error>> {
        let mut st = self.state.borrow_mut();
        let count = crate::pdf_export::append_pdf_pages(&mut st.document, pdf_path)?;
        drop(st);
        let total = self.total_pages();
        self.hud.update_metrics(|m| {
            m.page_count = total;
        });
        self.hud.update_view();
        self.area.queue_draw();
        Ok(count)
    }

    fn setup_drawing(&self) {
        let state_clone = self.state.clone();
        let hud_clone = self.hud.clone();

        self.area.set_draw_func(move |_area, cr, width, height| {
            let start_render = Instant::now();
            let st = state_clone.borrow();
            let vp = st.viewport;
            let current_mode = st.pipeline_mode;
            let is_dark = libadwaita::StyleManager::default().is_dark();

            match st.document.format {
                DocumentFormat::Notebook => {
                    // 1. Sfondo desktop canvas per quaderno A4
                    if is_dark {
                        cr.set_source_rgb(0.039, 0.082, 0.102); // #0A151A
                    } else {
                        cr.set_source_rgb(0.910, 0.925, 0.937); // #E8ECEF
                    }
                    let _ = cr.paint();

                    // 2. Rendering di ciascuna pagina A4
                    for page in &st.document.pages {
                        let doc_rect = st
                            .document
                            .page_rect(page.index)
                            .unwrap_or_else(|| Viewport::page_rect(page.index));
                        let (sx, sy) = vp.document_to_screen((doc_rect.x, doc_rect.y));
                        let sw = doc_rect.width * vp.zoom;
                        let sh = doc_rect.height * vp.zoom;

                        // Culling viewport: scarta pagine completamente fuori dallo schermo
                        if sx + sw < 0.0 || sx > width as f64 || sy + sh < 0.0 || sy > height as f64
                        {
                            continue;
                        }

                        // Ombra ad elevazione per foglio A4
                        if is_dark {
                            cr.set_source_rgba(0.0, 0.0, 0.0, 0.65);
                        } else {
                            cr.set_source_rgba(0.0, 0.0, 0.0, 0.14);
                        }
                        cr.rectangle(sx - 2.0, sy + 4.0, sw + 8.0, sh + 8.0);
                        let _ = cr.fill();

                        // Corpo foglio A4
                        if is_dark {
                            cr.set_source_rgb(0.090, 0.129, 0.149); // #172126 Deep Slate
                        } else {
                            cr.set_source_rgb(0.985, 0.980, 0.965); // #FAF9F6 Natural White
                        }
                        cr.rectangle(sx, sy, sw, sh);
                        let _ = cr.fill();

                        // Bordo perimetrale
                        if is_dark {
                            cr.set_source_rgba(0.243, 0.282, 0.306, 0.80); // #3E484E
                        } else {
                            cr.set_source_rgba(0.0, 0.0, 0.0, 0.09);
                        }
                        cr.set_line_width(1.0);
                        cr.rectangle(sx, sy, sw, sh);
                        let _ = cr.stroke();

                        // Ritaglio (Clip) sul perimetro del foglio
                        let _ = cr.save();
                        cr.rectangle(sx, sy, sw, sh);
                        cr.clip();

                        // Disegno del pattern di sfondo della pagina
                        match &page.background {
                            PageBackground::Blank => {}
                            PageBackground::Ruled {
                                line_spacing,
                                margin_left,
                            } => {
                                let spacing_px = line_spacing * vp.zoom;
                                let margin_px = margin_left * vp.zoom;

                                if is_dark {
                                    cr.set_source_rgba(1.0, 0.706, 0.670, 0.35);
                                // #FFB4AB Corallo DMS
                                } else {
                                    cr.set_source_rgba(0.88, 0.35, 0.35, 0.35);
                                }
                                cr.set_line_width(1.0);
                                cr.move_to(sx + margin_px, sy);
                                cr.line_to(sx + margin_px, sy + sh);
                                let _ = cr.stroke();

                                if is_dark {
                                    cr.set_source_rgba(0.427, 0.824, 1.0, 0.18);
                                } else {
                                    cr.set_source_rgba(0.35, 0.50, 0.70, 0.35);
                                }
                                cr.set_line_width(0.8);
                                let mut line_y = sy + spacing_px * 2.0;
                                while line_y < sy + sh {
                                    cr.move_to(sx, line_y);
                                    cr.line_to(sx + sw, line_y);
                                    line_y += spacing_px;
                                }
                                let _ = cr.stroke();
                            }
                            PageBackground::Grid { grid_size } => {
                                let grid_px = grid_size * vp.zoom;
                                if is_dark {
                                    cr.set_source_rgba(0.427, 0.824, 1.0, 0.14);
                                } else {
                                    cr.set_source_rgba(0.45, 0.60, 0.80, 0.28);
                                }
                                cr.set_line_width(0.7);

                                let mut gx = sx + grid_px;
                                while gx < sx + sw {
                                    cr.move_to(gx, sy);
                                    cr.line_to(gx, sy + sh);
                                    gx += grid_px;
                                }

                                let mut gy = sy + grid_px;
                                while gy < sy + sh {
                                    cr.move_to(sx, gy);
                                    cr.line_to(sx + sw, gy);
                                    gy += grid_px;
                                }
                                let _ = cr.stroke();
                            }
                            PageBackground::PdfPage {
                                source_path,
                                page_index,
                                original_width,
                                original_height,
                            } => {
                                let path = std::path::Path::new(source_path);
                                let rendered = if let Ok(surface) = st
                                    .pdf_cache
                                    .borrow_mut()
                                    .get_or_render_raster(path, *page_index)
                                {
                                    let _ = cr.save();
                                    cr.translate(sx, sy);
                                    let surf_w = surface.width() as f64;
                                    let surf_h = surface.height() as f64;
                                    if surf_w > 0.0 && surf_h > 0.0 {
                                        cr.scale(sw / surf_w, sh / surf_h);
                                        let _ = cr.set_source_surface(&surface, 0.0, 0.0);
                                        let pattern = cr.source();
                                        pattern.set_filter(gtk4::cairo::Filter::Bilinear);
                                        let _ = cr.paint();
                                    }
                                    let _ = cr.restore();
                                    true
                                } else {
                                    false
                                };

                                if !rendered {
                                    // Segnaposto elegante DMS per PDF non ancora caricato o file assente
                                    cr.set_source_rgba(0.50, 0.54, 0.62, 0.20);
                                    cr.set_line_width(1.0);
                                    cr.rectangle(sx + 12.0, sy + 12.0, sw - 24.0, sh - 24.0);
                                    let _ = cr.stroke();

                                    cr.set_source_rgba(0.50, 0.54, 0.62, 0.75);
                                    cr.set_font_size(12.0);
                                    let label = format!(
                                        "PDF Slide {} ({:.0} × {:.0} pt)",
                                        page_index + 1,
                                        original_width,
                                        original_height
                                    );
                                    cr.move_to(sx + 24.0, sy + 40.0);
                                    let _ = cr.show_text(&label);

                                    if !path.exists() {
                                        cr.set_source_rgba(0.95, 0.35, 0.35, 0.85);
                                        cr.set_font_size(11.0);
                                        cr.move_to(sx + 24.0, sy + 62.0);
                                        let warn = format!("File non trovato: {}", source_path);
                                        let _ = cr.show_text(&warn);
                                    }
                                }
                            }
                        }

                        // Tratti della pagina
                        let _ = cr.save();
                        cr.translate(vp.offset_x, vp.offset_y);
                        cr.scale(vp.zoom, vp.zoom);

                        for item in &page.items {
                            let ItemKind::Stroke(stroke) = &item.kind;
                            let is_selected_drag = st.selection.is_dragging
                                && st.selection.selected_ids.contains(&item.id);
                            if is_selected_drag {
                                let _ = cr.save();
                                cr.translate(
                                    st.selection.drag_offset.0,
                                    st.selection.drag_offset.1,
                                );
                                Self::draw_stroke(cr, stroke, current_mode, is_dark);
                                let _ = cr.restore();
                            } else {
                                Self::draw_stroke(cr, stroke, current_mode, is_dark);
                            }
                        }
                        let _ = cr.restore();

                        let _ = cr.restore(); // Fine clip pagina

                        if page.index + 1 < st.document.pages.len() {
                            cr.set_source_rgba(0.50, 0.54, 0.62, 0.70);
                            cr.set_font_size(11.0);
                            let label = format!("— Pagina {} —", page.index + 1);
                            cr.move_to(sx + (sw / 2.0) - 40.0, sy + sh + 20.0);
                            let _ = cr.show_text(&label);
                        }
                    }
                }
                DocumentFormat::Whiteboard => {
                    // Rendering procedurale della Lavagna Infinita
                    let spec = dynamic_grid_spec(vp.zoom);
                    let bg = st
                        .document
                        .pages
                        .first()
                        .map(|p| &p.background)
                        .unwrap_or(&PageBackground::Grid { grid_size: 14.17 });

                    Self::draw_whiteboard_background(
                        cr,
                        width as f64,
                        height as f64,
                        &vp,
                        spec,
                        bg,
                        is_dark,
                    );

                    // Rendering vettoriale dei tratti su tutta la superficie senza clipping
                    let _ = cr.save();
                    cr.translate(vp.offset_x, vp.offset_y);
                    cr.scale(vp.zoom, vp.zoom);

                    let vis_rect = uninote_core::coordinates::Rect2D::new(
                        -vp.offset_x / vp.zoom,
                        -vp.offset_y / vp.zoom,
                        width as f64 / vp.zoom,
                        height as f64 / vp.zoom,
                    );

                    for page in &st.document.pages {
                        for item in &page.items {
                            let ItemKind::Stroke(stroke) = &item.kind;
                            if let Some(bbox) = stroke.bounding_box() {
                                let stroke_rect = uninote_core::coordinates::Rect2D::new(
                                    bbox.min_x,
                                    bbox.min_y,
                                    bbox.max_x - bbox.min_x,
                                    bbox.max_y - bbox.min_y,
                                );
                                if !vis_rect.intersects(&stroke_rect) {
                                    continue;
                                }
                            }
                            let is_selected_drag = st.selection.is_dragging
                                && st.selection.selected_ids.contains(&item.id);
                            if is_selected_drag {
                                let _ = cr.save();
                                cr.translate(
                                    st.selection.drag_offset.0,
                                    st.selection.drag_offset.1,
                                );
                                Self::draw_stroke(cr, stroke, current_mode, is_dark);
                                let _ = cr.restore();
                            } else {
                                Self::draw_stroke(cr, stroke, current_mode, is_dark);
                            }
                        }
                    }
                    let _ = cr.restore();
                }
            }

            // 3. Disegno del tratto live attivo in corso di scrittura (se presente)
            if let Some(ref builder) = st.active_builder {
                let _ = cr.save();
                cr.translate(vp.offset_x, vp.offset_y);
                cr.scale(vp.zoom, vp.zoom);
                Self::draw_stroke(cr, &builder.stroke, current_mode, is_dark);
                let _ = cr.restore();
            }

            // 4. Disegno overlay di selezione (Lasso trail, rubber band, bbox tratteggiato #6DD2FF con maniglia)
            Self::draw_selection_overlay(cr, &st.selection, &vp, is_dark);

            // 4. Aggiornamento metriche di rendering HUD
            let render_duration = start_render.elapsed().as_micros() as u64;
            hud_clone.record_frame_tick();
            hud_clone.update_metrics(|m| {
                m.render_time_us = render_duration;
            });
            hud_clone.update_view();
        });
    }

    fn draw_stroke(cr: &gtk4::cairo::Context, stroke: &Stroke, mode: PipelineMode, is_dark: bool) {
        if stroke.points.is_empty() {
            return;
        }

        let adapted = adapt_ink_color(stroke.style.color, is_dark);
        let is_semi_transparent = adapted.3 < 0.999;
        if is_semi_transparent {
            cr.push_group_with_content(gtk4::cairo::Content::ColorAlpha);
            cr.set_source_rgb(adapted.0, adapted.1, adapted.2);
        } else {
            cr.set_source_rgba(adapted.0, adapted.1, adapted.2, adapted.3);
        }
        cr.set_line_cap(gtk4::cairo::LineCap::Round);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);

        if stroke.is_dot || stroke.points.len() == 1 {
            let p = &stroke.points[0];
            let radius = (p.width / 2.0).max(1.5);
            cr.arc(p.x, p.y, radius, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
            if is_semi_transparent {
                let _ = cr.pop_group_to_source();
                let _ = cr.paint_with_alpha(adapted.3);
            }
            return;
        }

        match mode {
            PipelineMode::Baseline => {
                for w in stroke.points.windows(2) {
                    let p0 = &w[0];
                    let p1 = &w[1];
                    let avg_width = (p0.width + p1.width) / 2.0;
                    cr.set_line_width(avg_width.max(1.0));
                    cr.move_to(p0.x, p0.y);
                    cr.line_to(p1.x, p1.y);
                    let _ = cr.stroke();
                }
            }
            PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered => {
                let segments = uninote_stroke::geometry::catmull_rom_to_bezier(&stroke.points);
                for seg in &segments {
                    let len = seg.approximate_length();
                    let width_diff = (seg.width1 - seg.width0).abs();

                    if width_diff < 0.25 || len <= 3.0 {
                        let avg_w = (seg.width0 + seg.width1) / 2.0;
                        cr.set_line_width(avg_w.max(1.0));
                        cr.move_to(seg.p0.0, seg.p0.1);
                        cr.curve_to(
                            seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
                        );
                        let _ = cr.stroke();
                    } else {
                        let steps = ((len / 2.0).ceil() as usize).clamp(2, 16);
                        let mut prev_pt = seg.p0;
                        let mut prev_w = seg.width0;
                        for s in 1..=steps {
                            let t = (s as f64) / (steps as f64);
                            let pt = seg.evaluate(t);
                            let w = seg.evaluate_width(t);
                            cr.set_line_width(((prev_w + w) / 2.0).max(1.0));
                            cr.move_to(prev_pt.0, prev_pt.1);
                            cr.line_to(pt.0, pt.1);
                            let _ = cr.stroke();
                            prev_pt = pt;
                            prev_w = w;
                        }
                    }
                }
            }
        }

        if is_semi_transparent {
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(adapted.3);
        }
    }

    fn setup_capture_controller(&self) {
        let legacy = gtk4::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let state_clone = self.state.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        legacy.connect_event(move |_controller, event| {
            let is_stylus = event.device_tool().is_some()
                || event
                    .device()
                    .is_some_and(|d| d.source() == gdk::InputSource::Pen);
            if is_stylus {
                return glib::Propagation::Proceed;
            }

            let event_type = event.event_type();
            let is_touch = matches!(
                event_type,
                gdk::EventType::TouchBegin
                    | gdk::EventType::TouchUpdate
                    | gdk::EventType::TouchEnd
                    | gdk::EventType::TouchCancel
            );

            if is_touch {
                let is_down = event_type == gdk::EventType::TouchBegin;
                let is_up = event_type == gdk::EventType::TouchEnd
                    || event_type == gdk::EventType::TouchCancel;
                let slot = if let Some(te) = event.downcast_ref::<gdk::TouchEvent>() {
                    use std::hash::{Hash, Hasher};
                    let mut hasher = std::collections::hash_map::DefaultHasher::new();
                    te.event_sequence().hash(&mut hasher);
                    hasher.finish()
                } else {
                    0
                };
                let now_ns = app_start.elapsed().as_nanos() as u64;

                let mut st = state_clone.borrow_mut();
                let suppress = st
                    .palm_engine
                    .should_suppress_raw_touch(is_down, slot, now_ns);
                if is_up {
                    st.palm_engine.on_touch_release(slot);
                }
                let suppressed_count = st.palm_engine.suppressed_touch_events;
                let palm_state = st.palm_engine.current_state();
                drop(st);

                hud_clone.update_metrics(|m| {
                    m.touch_suppressed = suppressed_count;
                    m.palm_state = palm_state;
                });

                if suppress {
                    return glib::Propagation::Stop;
                }
                return glib::Propagation::Proceed;
            }

            if event.is_pointer_emulated() {
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });

        self.area.add_controller(legacy);
    }

    fn setup_stylus_controller(&self) {
        let stylus = gtk4::GestureStylus::new();
        let state_clone = self.state.clone();
        let area_clone = self.area.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        // Down
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        stylus.connect_down(move |gesture, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let p_val = pressure.unwrap_or(0.0);
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st = state.borrow_mut();
            st.last_stylus_time = Some(Instant::now());
            st.active_device = Some(DeviceKind::Stylus);

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Selezione (Stylus Down)
            if st.active_tool == ActiveTool::Select {
                let hit = st
                    .selection
                    .hit_test_handle_or_bbox((doc_x, doc_y), 12.0 / st.viewport.zoom);
                if hit {
                    st.selection.is_dragging = true;
                    st.selection.drag_start_doc = Some((doc_x, doc_y));
                    st.selection.drag_offset = (0.0, 0.0);
                } else {
                    st.selection.clear();
                    match st.selection.mode {
                        SelectionMode::Lasso => {
                            st.selection.active_lasso.push((doc_x, doc_y));
                        }
                        SelectionMode::Rectangle => {
                            st.selection.active_rect_start = Some((doc_x, doc_y));
                            st.selection.active_rect_current = Some((doc_x, doc_y));
                        }
                    }
                }
                drop(st);
                area.queue_draw();
                return;
            }

            // Gestione Strumento Gomma (a schermo o tramite gomma hardware / pulsante laterale penna)
            let is_eraser_hardware = gesture
                .device_tool()
                .is_some_and(|t| t.tool_type() == gdk::DeviceToolType::Eraser)
                || gesture.current_button() == 2
                || gesture.current_button() == 3
                || gesture.current_event().is_some_and(|ev| {
                    let m = ev.modifier_state();
                    m.contains(gdk::ModifierType::BUTTON2_MASK)
                        || m.contains(gdk::ModifierType::BUTTON3_MASK)
                });
            let is_eraser = st.active_tool == ActiveTool::Eraser || is_eraser_hardware;

            if is_eraser {
                if let Some(prev_builder) = st.active_builder.take() {
                    let stroke = prev_builder.finish();
                    Self::commit_finished_stroke(&mut st, stroke);
                }
                let radius = 16.0 / st.viewport.zoom;
                Self::erase_strokes_at(&mut st, (doc_x, doc_y), radius);
                drop(st);
                area.queue_draw();
                return;
            }

            if let Some(prev_builder) = st.active_builder.take() {
                let stroke = prev_builder.finish();
                Self::commit_finished_stroke(&mut st, stroke);
            }

            if p_val <= st.tip_pressure_threshold {
                let palm_state = st.palm_engine.current_state();
                drop(st);
                hud.update_metrics(|m| {
                    m.live_pressure = pressure;
                    m.palm_state = palm_state;
                });
                return;
            }

            let sample = InputSample::new(
                (doc_x, doc_y),
                pressure,
                now_ns,
                DeviceKind::Stylus,
                InputPhase::Down,
            );

            if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                let id = st.next_stroke_id;
                st.next_stroke_id += 1;
                let mut style = st.current_style.clone();
                if st.active_tool == ActiveTool::Highlighter {
                    style.base_width = 14.0;
                    style.color.3 = 0.38;
                }
                let mut builder = StrokeBuilder::new(id, style, st.pipeline_mode);
                builder.process_sample(sample.clone());
                st.active_builder = Some(builder);

                if let Some(ref mut rec) = st.recorder {
                    rec.record_sample(sample);
                }

                let palm_state = st.palm_engine.current_state();
                drop(st);

                hud.update_metrics(|m| {
                    m.device_name = "Stilo Digitizer (Attivo)".to_string();
                    m.live_pressure = pressure;
                    m.palm_state = palm_state;
                    m.samples_received += 1;
                });
            }

            area.queue_draw();
        });

        // Motion
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        stylus.connect_motion(move |gesture, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let now = Instant::now();
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st_guard = state.borrow_mut();
            let st = &mut *st_guard;
            st.last_stylus_time = Some(now);

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Selezione (Stylus Motion)
            if st.active_tool == ActiveTool::Select {
                if st.selection.is_dragging {
                    if let Some(start) = st.selection.drag_start_doc {
                        st.selection.drag_offset = (doc_x - start.0, doc_y - start.1);
                    }
                    drop(st_guard);
                    area.queue_draw();
                    return;
                } else if st.selection.is_selecting() {
                    match st.selection.mode {
                        SelectionMode::Lasso => {
                            st.selection.active_lasso.push((doc_x, doc_y));
                        }
                        SelectionMode::Rectangle => {
                            st.selection.active_rect_current = Some((doc_x, doc_y));
                        }
                    }
                    drop(st_guard);
                    area.queue_draw();
                    return;
                }
                drop(st_guard);
                return;
            }

            // Gestione Strumento Gomma (a schermo o tramite gomma hardware / pulsante laterale penna)
            let is_eraser_hardware = gesture
                .device_tool()
                .is_some_and(|t| t.tool_type() == gdk::DeviceToolType::Eraser)
                || gesture.current_button() == 2
                || gesture.current_button() == 3
                || gesture.current_event().is_some_and(|ev| {
                    let m = ev.modifier_state();
                    m.contains(gdk::ModifierType::BUTTON2_MASK)
                        || m.contains(gdk::ModifierType::BUTTON3_MASK)
                });
            let is_eraser = st.active_tool == ActiveTool::Eraser || is_eraser_hardware;

            if is_eraser {
                if let Some(prev_builder) = st.active_builder.take() {
                    let stroke = prev_builder.finish();
                    Self::commit_finished_stroke(st, stroke);
                }
                let radius = 16.0 / st.viewport.zoom;
                Self::erase_strokes_at(st, (doc_x, doc_y), radius);
                drop(st_guard);
                area.queue_draw();
                return;
            }

            let has_button1 = gesture.current_event().is_some_and(|ev| {
                ev.modifier_state()
                    .contains(gdk::ModifierType::BUTTON1_MASK)
            });
            let p_val = pressure.unwrap_or(0.0);
            let thresh = st.tip_pressure_threshold;

            if st.active_builder.is_none() {
                if has_button1 && p_val > thresh {
                    let sample = InputSample::new(
                        (doc_x, doc_y),
                        pressure,
                        now_ns,
                        DeviceKind::Stylus,
                        InputPhase::Down,
                    );
                    if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                        let id = st.next_stroke_id;
                        st.next_stroke_id += 1;
                        let mut style = st.current_style.clone();
                        if st.active_tool == ActiveTool::Highlighter {
                            style.base_width = 14.0;
                            style.color.3 = 0.38;
                        }
                        let mut builder = StrokeBuilder::new(id, style, st.pipeline_mode);
                        builder.process_sample(sample.clone());
                        st.active_builder = Some(builder);
                        st.active_device = Some(DeviceKind::Stylus);
                        st.last_sample_time = Some(now);
                    }
                }
                let palm_state = st.palm_engine.current_state();
                drop(st_guard);
                hud.update_metrics(|m| {
                    m.live_pressure = pressure;
                    m.palm_state = palm_state;
                });
                area.queue_draw();
                return;
            }

            let (dist_from_last, speed_px_s) = if let Some(ref builder) = st.active_builder {
                if let Some(last_pt) = builder.stroke.points.last() {
                    let dist = ((last_pt.x - doc_x).powi(2) + (last_pt.y - doc_y).powi(2)).sqrt();
                    let dt = st
                        .last_sample_time
                        .map_or(0.008, |t| now.duration_since(t).as_secs_f64());
                    let speed = if dt > 0.0001 { dist / dt } else { 0.0 };
                    (dist, speed)
                } else {
                    (0.0, 0.0)
                }
            } else {
                (0.0, 0.0)
            };

            let is_kinematic_transit = speed_px_s > 300.0 && p_val < 0.12;
            let is_leap = dist_from_last > 18.0 && p_val < 0.10;
            let is_lifted = !has_button1
                || pressure.is_none()
                || p_val <= thresh
                || is_kinematic_transit
                || is_leap;

            if is_lifted {
                if let Some(b) = st.active_builder.take() {
                    let stroke = b.finish();
                    Self::commit_finished_stroke(st, stroke);
                }
                st.active_device = None;
                st.last_sample_time = None;
                let palm_state = st.palm_engine.current_state();
                drop(st_guard);
                hud.update_metrics(|m| {
                    m.live_pressure = pressure;
                    m.palm_state = palm_state;
                });
                area.queue_draw();
                return;
            }

            if let Some(ref mut builder) = st.active_builder {
                if let Some(last_time) = st.last_sample_time {
                    let dt = now.duration_since(last_time).as_secs_f64();
                    if dt > 0.0001 {
                        let hz = 1.0 / dt;
                        hud.update_metrics(|m| {
                            m.sample_rate_hz = m.sample_rate_hz * 0.85 + hz * 0.15;
                        });
                    }
                }
                st.last_sample_time = Some(now);

                let sample = InputSample::new(
                    (doc_x, doc_y),
                    pressure,
                    now_ns,
                    DeviceKind::Stylus,
                    InputPhase::Move,
                );
                builder.process_sample(sample.clone());

                if let Some(ref mut rec) = st.recorder {
                    rec.record_sample(sample);
                }

                let (received, dropped) = builder.samples_stats();
                let palm_state = st.palm_engine.current_state();
                drop(st_guard);

                hud.update_metrics(|m| {
                    m.live_pressure = pressure;
                    m.samples_received = received;
                    m.samples_dropped = dropped;
                    m.palm_state = palm_state;
                });
            }

            area.queue_draw();
        });

        // Up
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        stylus.connect_up(move |gesture, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st = state.borrow_mut();
            st.last_stylus_time = Some(Instant::now());
            st.active_device = None;

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Selezione (Stylus Up)
            if st.active_tool == ActiveTool::Select {
                if st.selection.is_dragging {
                    let (dx, dy) = st.selection.drag_offset;
                    st.selection.is_dragging = false;
                    st.selection.drag_start_doc = None;
                    st.selection.drag_offset = (0.0, 0.0);

                    if (dx.abs() > 0.001 || dy.abs() > 0.001)
                        && !st.selection.selected_ids.is_empty()
                    {
                        if let Some(page_idx) = st.selection.page_index {
                            if let Some(page) = st.document.pages.get(page_idx) {
                                let page_id = page.id;
                                let item_ids = st.selection.selected_ids.clone();
                                let cmd = DocCommand::MoveItems {
                                    page_id,
                                    item_ids,
                                    delta_x: dx,
                                    delta_y: dy,
                                };
                                let st = &mut *st;
                                st.history.execute(cmd, &mut st.document);
                                st.persist_changes();
                                if let Some(bbox) = st.selection.bbox.as_mut() {
                                    *bbox = bbox.translate(dx, dy);
                                }
                            }
                        }
                    }
                } else if st.selection.is_selecting() {
                    Self::finalize_selection_gesture(&mut st);
                }
                drop(st);
                area.queue_draw();
                return;
            }

            let sample = InputSample::new(
                (doc_x, doc_y),
                pressure,
                now_ns,
                DeviceKind::Stylus,
                InputPhase::Up,
            );
            st.palm_engine.evaluate_sample(&sample);

            if let Some(mut builder) = st.active_builder.take() {
                builder.process_sample(sample.clone());
                let stroke = builder.finish();
                Self::commit_finished_stroke(&mut st, stroke);

                if let Some(ref mut rec) = st.recorder {
                    rec.record_sample(sample);
                }
            }

            st.last_sample_time = None;
            let palm_state = st.palm_engine.current_state();
            drop(st);

            hud.update_metrics(|m| {
                m.live_pressure = None;
                m.palm_state = palm_state;
            });

            area.queue_draw();
        });

        // Proximity
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        stylus.connect_proximity(move |_gesture, _x, _y| {
            let now_ns = app_start.elapsed().as_nanos() as u64;
            let mut st = state.borrow_mut();
            st.last_stylus_time = Some(Instant::now());
            st.active_device = None;
            st.palm_engine.on_pen_proximity(true, now_ns);

            let closed_stroke = if let Some(builder) = st.active_builder.take() {
                let stroke = builder.finish();
                Self::commit_finished_stroke(&mut st, stroke);
                true
            } else {
                false
            };

            let palm_state = st.palm_engine.current_state();
            drop(st);

            if closed_stroke {
                area.queue_draw();
            }

            hud.update_metrics(|m| {
                m.palm_state = palm_state;
            });
        });

        self.area.add_controller(stylus);
    }

    fn setup_mouse_controller(&self) {
        let drag = gtk4::GestureDrag::new();
        drag.set_button(0);
        let state_clone = self.state.clone();
        let area_clone = self.area.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        // Drag Begin
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        drag.connect_drag_begin(move |gesture, start_x, start_y| {
            // Ignora categoricamente tocchi dal touchscreen (la mano/palmo non deve mai disegnare tratti)
            if gesture.current_sequence().is_some() {
                gesture.set_state(gtk4::EventSequenceState::Denied);
                return;
            }
            if let Some(event) = gesture.current_event() {
                if event
                    .device()
                    .is_some_and(|d| d.source() == gdk::InputSource::Touchscreen)
                {
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                    return;
                }
            }

            let current_btn = gesture.current_button();
            let mut st = state.borrow_mut();

            if let Some(last_pen) = st.last_stylus_time {
                if Instant::now().duration_since(last_pen).as_millis() < 400 {
                    return;
                }
            }

            if current_btn == gdk::BUTTON_SECONDARY || current_btn == gdk::BUTTON_MIDDLE {
                st.mouse_pan_start = Some((start_x, start_y));
                return;
            }

            if current_btn == gdk::BUTTON_PRIMARY {
                st.active_device = Some(DeviceKind::Mouse);
                st.mouse_drag_start = Some((start_x, start_y));
                let (doc_x, doc_y) = st.viewport.screen_to_document((start_x, start_y));
                let now_ns = app_start.elapsed().as_nanos() as u64;

                // Gestione Strumento Selezione (Mouse Drag Begin)
                if st.active_tool == ActiveTool::Select {
                    let hit = st
                        .selection
                        .hit_test_handle_or_bbox((doc_x, doc_y), 12.0 / st.viewport.zoom);
                    if hit {
                        st.selection.is_dragging = true;
                        st.selection.drag_start_doc = Some((doc_x, doc_y));
                        st.selection.drag_offset = (0.0, 0.0);
                    } else {
                        st.selection.clear();
                        match st.selection.mode {
                            SelectionMode::Lasso => {
                                st.selection.active_lasso.push((doc_x, doc_y));
                            }
                            SelectionMode::Rectangle => {
                                st.selection.active_rect_start = Some((doc_x, doc_y));
                                st.selection.active_rect_current = Some((doc_x, doc_y));
                            }
                        }
                    }
                    drop(st);
                    area.queue_draw();
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    let radius = 12.0 / st.viewport.zoom;
                    Self::erase_strokes_at(&mut st, (doc_x, doc_y), radius);
                    drop(st);
                    area.queue_draw();
                    return;
                }

                let sample = InputSample::new(
                    (doc_x, doc_y),
                    None,
                    now_ns,
                    DeviceKind::Mouse,
                    InputPhase::Down,
                );

                if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                    let id = st.next_stroke_id;
                    st.next_stroke_id += 1;
                    let mut style = st.current_style.clone();
                    if st.active_tool == ActiveTool::Highlighter {
                        style.base_width = 14.0;
                        style.color.3 = 0.38;
                    }
                    let mut builder = StrokeBuilder::new(id, style, st.pipeline_mode);
                    builder.process_sample(sample.clone());
                    st.active_builder = Some(builder);

                    if let Some(ref mut rec) = st.recorder {
                        rec.record_sample(sample);
                    }

                    drop(st);
                    hud.update_metrics(|m| {
                        m.device_name = "Mouse / Touchpad".to_string();
                        m.samples_received += 1;
                    });
                }
                area.queue_draw();
            }
        });

        // Drag Update
        let state = state_clone.clone();
        let area = area_clone.clone();
        drag.connect_drag_update(move |gesture, offset_x, offset_y| {
            if gesture.current_sequence().is_some() {
                return;
            }
            if let Some(event) = gesture.current_event() {
                if event
                    .device()
                    .is_some_and(|d| d.source() == gdk::InputSource::Touchscreen)
                {
                    return;
                }
            }

            let current_btn = gesture.current_button();
            let mut st_guard = state.borrow_mut();
            let st = &mut *st_guard;

            if current_btn == gdk::BUTTON_SECONDARY || current_btn == gdk::BUTTON_MIDDLE {
                if let Some((sx, sy)) = st.mouse_pan_start {
                    st.viewport.offset_x += offset_x;
                    st.viewport.offset_y += offset_y;
                    st.mouse_pan_start = Some((sx + offset_x, sy + offset_y));
                    drop(st_guard);
                    area.queue_draw();
                }
                return;
            }

            if st.active_device != Some(DeviceKind::Mouse) {
                return;
            }

            if let Some((start_x, start_y)) = st.mouse_drag_start {
                let cur_screen_x = start_x + offset_x;
                let cur_screen_y = start_y + offset_y;
                let (doc_x, doc_y) = st.viewport.screen_to_document((cur_screen_x, cur_screen_y));
                let now_ns = app_start.elapsed().as_nanos() as u64;

                // Gestione Strumento Selezione (Mouse Drag Update)
                if st.active_tool == ActiveTool::Select {
                    if st.selection.is_dragging {
                        if let Some(start) = st.selection.drag_start_doc {
                            st.selection.drag_offset = (doc_x - start.0, doc_y - start.1);
                        }
                        drop(st_guard);
                        area.queue_draw();
                        return;
                    } else if st.selection.is_selecting() {
                        match st.selection.mode {
                            SelectionMode::Lasso => {
                                st.selection.active_lasso.push((doc_x, doc_y));
                            }
                            SelectionMode::Rectangle => {
                                st.selection.active_rect_current = Some((doc_x, doc_y));
                            }
                        }
                        drop(st_guard);
                        area.queue_draw();
                        return;
                    }
                    drop(st_guard);
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    let radius = 12.0 / st.viewport.zoom;
                    Self::erase_strokes_at(st, (doc_x, doc_y), radius);
                    drop(st_guard);
                    area.queue_draw();
                    return;
                }

                if let Some(ref mut builder) = st.active_builder {
                    let sample = InputSample::new(
                        (doc_x, doc_y),
                        None,
                        now_ns,
                        DeviceKind::Mouse,
                        InputPhase::Move,
                    );
                    builder.process_sample(sample.clone());

                    if let Some(ref mut rec) = st.recorder {
                        rec.record_sample(sample);
                    }
                }
            }

            area.queue_draw();
        });

        // Drag End
        let state = state_clone.clone();
        let area = area_clone.clone();
        drag.connect_drag_end(move |gesture, offset_x, offset_y| {
            if gesture.current_sequence().is_some() {
                return;
            }
            if let Some(event) = gesture.current_event() {
                if event
                    .device()
                    .is_some_and(|d| d.source() == gdk::InputSource::Touchscreen)
                {
                    return;
                }
            }

            let mut st = state.borrow_mut();
            st.mouse_pan_start = None;

            if st.active_device != Some(DeviceKind::Mouse) {
                st.mouse_drag_start = None;
                return;
            }
            st.active_device = None;

            if let Some((start_x, start_y)) = st.mouse_drag_start.take() {
                let cur_x = start_x + offset_x;
                let cur_y = start_y + offset_y;
                let (doc_x, doc_y) = st.viewport.screen_to_document((cur_x, cur_y));
                let now_ns = app_start.elapsed().as_nanos() as u64;

                // Gestione Strumento Selezione (Mouse Drag End)
                if st.active_tool == ActiveTool::Select {
                    if st.selection.is_dragging {
                        let (dx, dy) = st.selection.drag_offset;
                        st.selection.is_dragging = false;
                        st.selection.drag_start_doc = None;
                        st.selection.drag_offset = (0.0, 0.0);

                        if (dx.abs() > 0.001 || dy.abs() > 0.001)
                            && !st.selection.selected_ids.is_empty()
                        {
                            if let Some(page_idx) = st.selection.page_index {
                                if let Some(page) = st.document.pages.get(page_idx) {
                                    let page_id = page.id;
                                    let item_ids = st.selection.selected_ids.clone();
                                    let cmd = DocCommand::MoveItems {
                                        page_id,
                                        item_ids,
                                        delta_x: dx,
                                        delta_y: dy,
                                    };
                                    let st = &mut *st;
                                    st.history.execute(cmd, &mut st.document);
                                    st.persist_changes();
                                    if let Some(bbox) = st.selection.bbox.as_mut() {
                                        *bbox = bbox.translate(dx, dy);
                                    }
                                }
                            }
                        }
                    } else if st.selection.is_selecting() {
                        Self::finalize_selection_gesture(&mut st);
                    }
                    drop(st);
                    area.queue_draw();
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    let radius = 12.0 / st.viewport.zoom;
                    Self::erase_strokes_at(&mut st, (doc_x, doc_y), radius);
                    drop(st);
                    area.queue_draw();
                    return;
                }

                let sample = InputSample::new(
                    (doc_x, doc_y),
                    None,
                    now_ns,
                    DeviceKind::Mouse,
                    InputPhase::Up,
                );
                st.palm_engine.evaluate_sample(&sample);

                if let Some(mut builder) = st.active_builder.take() {
                    builder.process_sample(sample.clone());
                    let stroke = builder.finish();
                    Self::commit_finished_stroke(&mut st, stroke);

                    if let Some(ref mut rec) = st.recorder {
                        rec.record_sample(sample);
                    }
                }
            }

            area.queue_draw();
        });

        self.area.add_controller(drag);
    }

    fn setup_scroll_and_zoom_controllers(&self) {
        let scroll = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
        );
        let state = self.state.clone();
        let area = self.area.clone();
        let hud = self.hud.clone();

        scroll.connect_scroll(move |controller, dx, dy| {
            let mut st = state.borrow_mut();
            let is_ctrl = controller
                .current_event_state()
                .contains(gdk::ModifierType::CONTROL_MASK);

            if is_ctrl {
                let w = area.width() as f64;
                let h = area.height() as f64;
                let factor = if dy < 0.0 { 1.15 } else { 0.87 };
                st.viewport.zoom_at((w / 2.0, h / 2.0), factor);
                let pct = (st.viewport.zoom * 100.0).round() as usize;
                drop(st);
                hud.update_metrics(|m| m.zoom_pct = pct);
                hud.update_view();
            } else {
                let (pan_x, pan_y) = match controller.unit() {
                    gdk::ScrollUnit::Surface => (-dx, -dy),
                    gdk::ScrollUnit::Wheel => (-dx * 25.0, -dy * 25.0),
                    _ => (-dx, -dy),
                };
                st.viewport.pan(pan_x, pan_y);
                drop(st);
            }

            area.queue_draw();
            glib::Propagation::Stop
        });
        self.area.add_controller(scroll);

        let zoom = gtk4::GestureZoom::new();
        let state = self.state.clone();
        let area = self.area.clone();
        let hud = self.hud.clone();

        let last_scale = Rc::new(std::cell::Cell::new(1.0));
        let last_center = Rc::new(std::cell::Cell::new(None));

        let ls_begin = last_scale.clone();
        let lc_begin = last_center.clone();
        zoom.connect_begin(move |gesture, _seq| {
            ls_begin.set(1.0);
            lc_begin.set(gesture.bounding_box_center());
        });

        // Pan a due dita indipendente dal pinch-to-zoom: calcolato sull'evento update
        let state_update = state.clone();
        let area_update = area.clone();
        let lc_update = last_center.clone();
        zoom.connect_update(move |gesture, _seq| {
            if let Some(current_center) = gesture.bounding_box_center() {
                if let Some(prev_center) = lc_update.get() {
                    let dx = current_center.0 - prev_center.0;
                    let dy = current_center.1 - prev_center.1;
                    if dx.abs() > 0.01 || dy.abs() > 0.01 {
                        let mut st = state_update.borrow_mut();
                        st.viewport.pan(dx, dy);
                        drop(st);
                        area_update.queue_draw();
                    }
                }
                lc_update.set(Some(current_center));
            }
        });

        // Zoom scalare (pinch): scala il canvas centrato sul bounding_box_center
        let ls_scale = last_scale.clone();
        let state_scale = state.clone();
        let area_scale = area.clone();
        let hud_scale = hud.clone();
        zoom.connect_scale_changed(move |gesture, scale| {
            let prev_scale = ls_scale.get();
            ls_scale.set(scale);
            let factor = if prev_scale > 0.001 {
                scale / prev_scale
            } else {
                1.0
            };

            let w = area_scale.width() as f64;
            let h = area_scale.height() as f64;
            let default_center = (w / 2.0, h / 2.0);
            let current_center = gesture.bounding_box_center().unwrap_or(default_center);

            if (factor - 1.0).abs() > 0.001 && factor > 0.5 && factor < 2.0 {
                let mut st = state_scale.borrow_mut();
                st.viewport.zoom_at(current_center, factor);
                let pct = (st.viewport.zoom * 100.0).round() as usize;
                drop(st);

                hud_scale.update_metrics(|m| m.zoom_pct = pct);
                hud_scale.update_view();
                area_scale.queue_draw();
            }
        });

        let lc_end = last_center.clone();
        let ls_end = last_scale.clone();
        zoom.connect_end(move |_gesture, _seq| {
            lc_end.set(None);
            ls_end.set(1.0);
        });
        self.area.add_controller(zoom);
    }

    /// Cancella i tratti che intersecano la coordinata del documento indicata.
    fn erase_strokes_at(st: &mut CanvasState, doc_pt: (f64, f64), radius: f64) {
        if st.document.pages.is_empty() {
            return;
        }

        let page_idx = if st.document.format == DocumentFormat::Whiteboard {
            0
        } else {
            Viewport::find_page_index_at_y(doc_pt.1, st.document.pages.len())
        };
        if let Some(page) = st.document.pages.get_mut(page_idx) {
            let page_id = page.id;
            let mut erased = Vec::new();

            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if s.intersects_point(doc_pt, radius) {
                    erased.push(item.clone());
                }
            }

            if !erased.is_empty() {
                st.history.execute(
                    DocCommand::EraseItems {
                        page_id,
                        erased_items: erased,
                    },
                    &mut st.document,
                );
                st.persist_changes();
            }
        }
    }

    /// Salva un tratto appena completato nella pagina corrispondente del documento.
    fn commit_finished_stroke(st: &mut CanvasState, mut stroke: Stroke) {
        if stroke.points.is_empty() || st.document.pages.is_empty() {
            return;
        }

        if st.active_tool == ActiveTool::Line {
            if let Some(snapped) = uninote_stroke::shape::snap_line_orthogonal(&stroke) {
                stroke = snapped;
            }
        } else if st.active_tool == ActiveTool::Shapes {
            if let Some(recognized) = uninote_stroke::shape::recognize_shape(&stroke) {
                stroke = recognized;
            }
        }

        let first_pt = &stroke.points[0];
        let page_idx = if st.document.format == DocumentFormat::Whiteboard {
            0
        } else {
            st.document.find_page_index_at_y(first_pt.y).unwrap_or(0)
        };
        let page_id = st.document.pages[page_idx].id;

        st.history
            .execute(DocCommand::AddStroke { page_id, stroke }, &mut st.document);
        st.persist_changes();
    }

    fn draw_whiteboard_background(
        cr: &gtk4::cairo::Context,
        width: f64,
        height: f64,
        vp: &Viewport,
        spec: DynamicGridSpec,
        bg: &PageBackground,
        is_dark: bool,
    ) {
        if !width.is_finite() || !height.is_finite() || width <= 0.0 || height <= 0.0 {
            return;
        }

        // 1. Desktop background flood fill
        if is_dark {
            cr.set_source_rgb(0.039, 0.082, 0.102); // #0A151A Dark OLED
        } else {
            cr.set_source_rgb(0.975, 0.978, 0.980); // #F8F9FA Natural Crisp White
        }
        let _ = cr.paint();

        // Palette cromatiche DMS per la griglia
        let (grid_r, grid_g, grid_b) = if is_dark {
            (0.475, 0.851, 0.647) // #79D8A5 Verde menta DMS
        } else {
            (0.350, 0.520, 0.650) // Ciano-ardesia soft DMS
        };

        let doc_left = -vp.offset_x / vp.zoom;
        let doc_top = -vp.offset_y / vp.zoom;
        let doc_right = (width - vp.offset_x) / vp.zoom;
        let doc_bottom = (height - vp.offset_y) / vp.zoom;

        let start_col = (doc_left / spec.step_pt).floor() as i64;
        let end_col = (doc_right / spec.step_pt).ceil() as i64;
        let start_row = (doc_top / spec.step_pt).floor() as i64;
        let end_row = (doc_bottom / spec.step_pt).ceil() as i64;

        match bg {
            PageBackground::Blank => {
                // Sfondo vuoto pulito senza artefatti o griglia
            }
            PageBackground::PdfPage { .. } => {
                // Sfondo per PDF
            }
            PageBackground::Grid { .. } => {
                // Linee di griglia minori
                cr.set_source_rgba(grid_r, grid_g, grid_b, spec.alpha * 0.45);
                cr.set_line_width(0.7);

                for col in start_col..=end_col {
                    let sx = (col as f64 * spec.step_pt) * vp.zoom + vp.offset_x;
                    cr.move_to(sx, 0.0);
                    cr.line_to(sx, height);
                }
                for row in start_row..=end_row {
                    let sy = (row as f64 * spec.step_pt) * vp.zoom + vp.offset_y;
                    cr.move_to(0.0, sy);
                    cr.line_to(width, sy);
                }
                let _ = cr.stroke();

                // Linee di griglia maggiori (major_step_pt)
                let major_step = spec.major_step_pt.max(spec.step_pt);
                let major_start_col = (doc_left / major_step).floor() as i64;
                let major_end_col = (doc_right / major_step).ceil() as i64;
                let major_start_row = (doc_top / major_step).floor() as i64;
                let major_end_row = (doc_bottom / major_step).ceil() as i64;

                cr.set_source_rgba(grid_r, grid_g, grid_b, (spec.alpha * 0.9).min(0.50));
                cr.set_line_width(1.0);

                for col in major_start_col..=major_end_col {
                    let sx = (col as f64 * major_step) * vp.zoom + vp.offset_x;
                    cr.move_to(sx, 0.0);
                    cr.line_to(sx, height);
                }
                for row in major_start_row..=major_end_row {
                    let sy = (row as f64 * major_step) * vp.zoom + vp.offset_y;
                    cr.move_to(0.0, sy);
                    cr.line_to(width, sy);
                }
                let _ = cr.stroke();

                // Assi cartesiani principali di origine X = 0 e Y = 0 (se visibili a schermo)
                let origin_x = vp.offset_x;
                let origin_y = vp.offset_y;

                if origin_x >= 0.0 && origin_x <= width {
                    if is_dark {
                        cr.set_source_rgba(0.427, 0.824, 1.0, 0.45); // Ciano elettrico DMS per asse Y
                    } else {
                        cr.set_source_rgba(0.20, 0.45, 0.70, 0.45);
                    }
                    cr.set_line_width(1.2);
                    cr.move_to(origin_x, 0.0);
                    cr.line_to(origin_x, height);
                    let _ = cr.stroke();
                }

                if origin_y >= 0.0 && origin_y <= height {
                    if is_dark {
                        cr.set_source_rgba(0.427, 0.824, 1.0, 0.45); // Ciano elettrico DMS per asse X
                    } else {
                        cr.set_source_rgba(0.20, 0.45, 0.70, 0.45);
                    }
                    cr.set_line_width(1.2);
                    cr.move_to(0.0, origin_y);
                    cr.line_to(width, origin_y);
                    let _ = cr.stroke();
                }
            }
            PageBackground::Ruled { line_spacing, .. } => {
                let spacing = if *line_spacing > 5.0 {
                    *line_spacing
                } else {
                    spec.step_pt
                };
                let r_start = (doc_top / spacing).floor() as i64;
                let r_end = (doc_bottom / spacing).ceil() as i64;

                cr.set_source_rgba(grid_r, grid_g, grid_b, spec.alpha * 0.55);
                cr.set_line_width(0.8);

                for row in r_start..=r_end {
                    let sy = (row as f64 * spacing) * vp.zoom + vp.offset_y;
                    cr.move_to(0.0, sy);
                    cr.line_to(width, sy);
                }
                let _ = cr.stroke();

                // Margine verticale di riferimento all'asse X = 0 (se visibile)
                let origin_x = vp.offset_x;
                if origin_x >= 0.0 && origin_x <= width {
                    if is_dark {
                        cr.set_source_rgba(1.0, 0.706, 0.670, 0.40); // Corallo DMS
                    } else {
                        cr.set_source_rgba(0.88, 0.35, 0.35, 0.40);
                    }
                    cr.set_line_width(1.0);
                    cr.move_to(origin_x, 0.0);
                    cr.line_to(origin_x, height);
                    let _ = cr.stroke();
                }
            }
        }
    }

    /// Conclude il gesto di selezione poligonale o rettangolare e rileva i tratti selezionati.
    fn finalize_selection_gesture(st: &mut CanvasState) {
        if st.document.pages.is_empty() {
            st.selection.clear();
            return;
        }

        match st.selection.mode {
            SelectionMode::Lasso => {
                let poly = std::mem::take(&mut st.selection.active_lasso);
                if poly.len() < 3 {
                    st.selection.clear();
                    return;
                }

                let min_y = poly.iter().map(|p| p.1).fold(f64::INFINITY, f64::min);
                let max_y = poly.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max);
                let center_y = (min_y + max_y) / 2.0;

                let page_idx = if st.document.format == DocumentFormat::Whiteboard {
                    0
                } else {
                    Viewport::find_page_index_at_y(center_y, st.document.pages.len())
                };
                let page = &st.document.pages[page_idx];

                let strokes: Vec<&Stroke> = page
                    .items
                    .iter()
                    .map(|it| match &it.kind {
                        ItemKind::Stroke(s) => s,
                    })
                    .collect();

                let selected_stroke_ids =
                    SelectionEngine::select_lasso(strokes.iter().copied(), &poly);
                let stroke_id_set: std::collections::HashSet<u64> =
                    selected_stroke_ids.into_iter().collect();

                let mut selected_item_ids = Vec::new();
                let mut selected_strokes = Vec::new();
                for it in &page.items {
                    let ItemKind::Stroke(ref s) = it.kind;
                    if stroke_id_set.contains(&s.id) {
                        selected_item_ids.push(it.id);
                        selected_strokes.push(s);
                    }
                }

                if !selected_item_ids.is_empty() {
                    let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = selected_item_ids;
                    st.selection.bbox = bbox;
                } else {
                    st.selection.clear();
                }
            }
            SelectionMode::Rectangle => {
                let start = st.selection.active_rect_start.take();
                let current = st.selection.active_rect_current.take();
                let (p1, p2) = match (start, current) {
                    (Some(a), Some(b)) => (a, b),
                    _ => {
                        st.selection.clear();
                        return;
                    }
                };

                let rect = Rect::new(p1.0, p1.1, p2.0, p2.1).normalized();
                if rect.width() < 2.0 && rect.height() < 2.0 {
                    st.selection.clear();
                    return;
                }

                let center_y = (rect.min_y + rect.max_y) / 2.0;
                let page_idx = if st.document.format == DocumentFormat::Whiteboard {
                    0
                } else {
                    Viewport::find_page_index_at_y(center_y, st.document.pages.len())
                };
                let page = &st.document.pages[page_idx];

                let strokes: Vec<&Stroke> = page
                    .items
                    .iter()
                    .map(|it| match &it.kind {
                        ItemKind::Stroke(s) => s,
                    })
                    .collect();

                let selected_stroke_ids =
                    SelectionEngine::select_rect(strokes.iter().copied(), rect);
                let stroke_id_set: std::collections::HashSet<u64> =
                    selected_stroke_ids.into_iter().collect();

                let mut selected_item_ids = Vec::new();
                let mut selected_strokes = Vec::new();
                for it in &page.items {
                    let ItemKind::Stroke(ref s) = it.kind;
                    if stroke_id_set.contains(&s.id) {
                        selected_item_ids.push(it.id);
                        selected_strokes.push(s);
                    }
                }

                if !selected_item_ids.is_empty() {
                    let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = selected_item_ids;
                    st.selection.bbox = bbox;
                } else {
                    st.selection.clear();
                }
            }
        }
    }

    /// Elimina in blocco gli elementi attualmente selezionati tramite DocCommand::EraseItems.
    fn delete_selected_items(st: &mut CanvasState) -> bool {
        if !st.selection.has_selection() {
            return false;
        }

        let page_idx = match st.selection.page_index {
            Some(idx) => idx,
            None => return false,
        };

        let page = match st.document.pages.get_mut(page_idx) {
            Some(p) => p,
            None => return false,
        };

        let page_id = page.id;
        let id_set: std::collections::HashSet<ItemId> =
            st.selection.selected_ids.iter().copied().collect();
        let erased: Vec<uninote_core::model::Item> = page
            .items
            .iter()
            .filter(|it| id_set.contains(&it.id))
            .cloned()
            .collect();

        if !erased.is_empty() {
            st.history.execute(
                DocCommand::EraseItems {
                    page_id,
                    erased_items: erased,
                },
                &mut st.document,
            );
            st.persist_changes();
            st.selection.clear();
            true
        } else {
            false
        }
    }

    /// Disegna l'overlay visivo della selezione (traccia lazo, rubber band, bbox tratteggiato #6DD2FF con maniglia).
    fn draw_selection_overlay(
        cr: &gtk4::cairo::Context,
        selection: &SelectionState,
        vp: &Viewport,
        is_dark: bool,
    ) {
        let cyan = (0.427, 0.824, 1.0); // #6DD2FF UniNote Electric Cyan

        // 1. Traccia attiva durante il disegno del gesto
        if selection.is_selecting() {
            match selection.mode {
                SelectionMode::Lasso => {
                    if selection.active_lasso.len() >= 2 {
                        let _ = cr.save();
                        cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.85);
                        cr.set_line_width(1.5);
                        cr.set_dash(&[5.0, 4.0], 0.0);

                        let first = vp.document_to_screen(selection.active_lasso[0]);
                        cr.move_to(first.0, first.1);
                        for &pt in &selection.active_lasso[1..] {
                            let (sx, sy) = vp.document_to_screen(pt);
                            cr.line_to(sx, sy);
                        }
                        let _ = cr.stroke_preserve();

                        cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.06);
                        let _ = cr.fill();
                        let _ = cr.restore();
                    }
                }
                SelectionMode::Rectangle => {
                    if let (Some(start), Some(curr)) =
                        (selection.active_rect_start, selection.active_rect_current)
                    {
                        let r = Rect::new(start.0, start.1, curr.0, curr.1).normalized();
                        let (s_min_x, s_min_y) = vp.document_to_screen((r.min_x, r.min_y));
                        let (s_max_x, s_max_y) = vp.document_to_screen((r.max_x, r.max_y));
                        let sw = s_max_x - s_min_x;
                        let sh = s_max_y - s_min_y;

                        let _ = cr.save();
                        cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.08);
                        cr.rectangle(s_min_x, s_min_y, sw, sh);
                        let _ = cr.fill();

                        cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.90);
                        cr.set_line_width(1.5);
                        cr.set_dash(&[6.0, 4.0], 0.0);
                        cr.rectangle(s_min_x, s_min_y, sw, sh);
                        let _ = cr.stroke();
                        let _ = cr.restore();
                    }
                }
            }
        }

        // 2. Bounding box tratteggiato #6DD2FF con maniglia centrale
        if let Some(ref base_bbox) = selection.bbox {
            let (dx, dy) = if selection.is_dragging {
                selection.drag_offset
            } else {
                (0.0, 0.0)
            };
            let live_bbox = base_bbox.translate(dx, dy);

            let (s_min_x, s_min_y) = vp.document_to_screen((live_bbox.min_x, live_bbox.min_y));
            let (s_max_x, s_max_y) = vp.document_to_screen((live_bbox.max_x, live_bbox.max_y));
            let sw = (s_max_x - s_min_x).max(1.0);
            let sh = (s_max_y - s_min_y).max(1.0);

            let _ = cr.save();
            cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.05);
            cr.rectangle(s_min_x, s_min_y, sw, sh);
            let _ = cr.fill();

            cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.95);
            cr.set_line_width(1.8);
            cr.set_dash(&[6.0, 4.0], 0.0);
            cr.rectangle(s_min_x, s_min_y, sw, sh);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);

            // Maniglia centrale
            let center_x = s_min_x + sw / 2.0;
            let center_y = s_min_y + sh / 2.0;
            let handle_radius = 8.0;

            cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
            cr.arc(
                center_x,
                center_y + 1.0,
                handle_radius + 1.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();

            cr.set_source_rgb(cyan.0, cyan.1, cyan.2);
            cr.arc(
                center_x,
                center_y,
                handle_radius,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();

            if is_dark {
                cr.set_source_rgb(0.039, 0.082, 0.102);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            cr.set_line_width(2.0);
            cr.arc(
                center_x,
                center_y,
                handle_radius,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.stroke();

            cr.arc(center_x, center_y, 2.5, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();

            let _ = cr.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_adapt_ink_color_dark_neutral_on_dark_sheet() {
        let black = (0.0, 0.0, 0.0, 1.0);
        let adapted = adapt_ink_color(black, true);
        assert_eq!(adapted, (0.925, 0.937, 0.957, 1.0));

        let dark_gray = (0.15, 0.15, 0.15, 0.85);
        let adapted_gray = adapt_ink_color(dark_gray, true);
        assert_eq!(adapted_gray, (0.925, 0.937, 0.957, 0.85));
    }

    #[test]
    fn test_adapt_ink_color_dark_saturated_tone_lift_on_dark_sheet() {
        // Dark navy: (0.0, 0.0, 0.5, 1.0) -> lum ~ 0.0361 < 0.35, chroma = 0.5 >= 0.12
        let navy = (0.0, 0.0, 0.5, 1.0);
        let adapted = adapt_ink_color(navy, true);
        let adapted_lum = 0.2126 * adapted.0 + 0.7152 * adapted.1 + 0.0722 * adapted.2;
        assert!(
            adapted_lum > 0.65,
            "Adapted navy luminance must be lifted to pastel contrast: got {}",
            adapted_lum
        );
        assert!(
            adapted.2 > adapted.0 && adapted.2 > adapted.1,
            "Blue hue must be preserved"
        );
        assert_eq!(adapted.3, 1.0);

        // Dark maroon: (0.4, 0.0, 0.0, 0.9) -> lum ~ 0.085 < 0.35, chroma = 0.4 >= 0.12
        let maroon = (0.4, 0.0, 0.0, 0.9);
        let adapted_maroon = adapt_ink_color(maroon, true);
        let maroon_lum =
            0.2126 * adapted_maroon.0 + 0.7152 * adapted_maroon.1 + 0.0722 * adapted_maroon.2;
        assert!(
            maroon_lum > 0.65,
            "Maroon luminance must be lifted: got {}",
            maroon_lum
        );
        assert!(
            adapted_maroon.0 > adapted_maroon.1 && adapted_maroon.0 > adapted_maroon.2,
            "Red hue preserved"
        );
        assert_eq!(adapted_maroon.3, 0.9);
    }

    #[test]
    fn test_adapt_ink_color_vibrant_preserved_on_dark_sheet() {
        // Cyan: lum ~ 0.644 >= 0.35
        let cyan = (0.0, 0.8, 1.0, 1.0);
        assert_eq!(adapt_ink_color(cyan, true), cyan);

        // Yellow: lum ~ 0.928 >= 0.35
        let yellow = (1.0, 1.0, 0.0, 0.75);
        assert_eq!(adapt_ink_color(yellow, true), yellow);

        // White: lum 1.0 >= 0.35
        let white = (1.0, 1.0, 1.0, 1.0);
        assert_eq!(adapt_ink_color(white, true), white);
    }

    #[test]
    fn test_adapt_ink_color_pure_white_on_light_sheet() {
        // Pure white: lum 1.0 > 0.85, chroma 0.0 < 0.12 -> mapped to graphite #172126
        let white = (1.0, 1.0, 1.0, 1.0);
        let adapted = adapt_ink_color(white, false);
        assert_eq!(adapted, (0.090, 0.129, 0.149, 1.0));

        let off_white = (0.95, 0.95, 0.95, 0.6);
        let adapted_off = adapt_ink_color(off_white, false);
        assert_eq!(adapted_off, (0.090, 0.129, 0.149, 0.6));
    }

    #[test]
    fn test_adapt_ink_color_standard_colors_on_light_sheet_preserved() {
        // Black on light sheet
        let black = (0.0, 0.0, 0.0, 1.0);
        assert_eq!(adapt_ink_color(black, false), black);

        // Navy on light sheet
        let navy = (0.0, 0.0, 0.5, 1.0);
        assert_eq!(adapt_ink_color(navy, false), navy);

        // Cyan on light sheet
        let cyan = (0.0, 0.8, 1.0, 1.0);
        assert_eq!(adapt_ink_color(cyan, false), cyan);

        // Pale saturated yellow (lum > 0.85, but chroma >= 0.12)
        let pale_yellow = (1.0, 1.0, 0.5, 1.0);
        assert_eq!(adapt_ink_color(pale_yellow, false), pale_yellow);
    }

    #[test]
    fn test_adapt_ink_color_alpha_strictly_preserved() {
        for alpha in [0.0, 0.15, 0.5, 0.85, 1.0] {
            let col = (0.0, 0.0, 0.0, alpha);
            assert_eq!(adapt_ink_color(col, true).3, alpha);
            let white = (1.0, 1.0, 1.0, alpha);
            assert_eq!(adapt_ink_color(white, false).3, alpha);
        }
    }
}
