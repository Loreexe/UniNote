use crate::hud::DiagnosticHud;
use crate::icons::{create_icon, IconKind};
use crate::selection::{
    compute_scale_factors, HandleId, SelectionDragKind, SelectionHitTestResult,
};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;
use uninote_core::{
    CommandHistory, DocCommand, Document, DocumentFormat, Item, ItemId, ItemKind, PageBackground,
    Viewport,
};
use uninote_storage::{DocumentStore, SaveStatus, StorageWorker};
use uninote_stroke::selection::SelectionMode;
use uninote_stroke::{
    DeviceKind, InputDisposition, InputPhase, InputSample, PalmRejectionEngine, PipelineMode, Rect,
    Stroke, StrokeBuilder, StrokeStyle, TouchMode,
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
        zoom.clamp(0.25, 10.0)
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
    } else if clamped_zoom <= 5.0 {
        let t = ((clamped_zoom - 2.0) / 3.0).min(1.0); // 0.0 a 2.0, 1.0 a 5.0
        DynamicGridSpec {
            step_pt: base_step / 2.0, // 10.0 pt
            alpha: 0.15 + 0.20 * t,
            major_step_pt: base_step * 2.5,
        }
    } else {
        let t = ((clamped_zoom - 5.0) / 5.0).min(1.0); // 0.0 a 5.0, 1.0 a 10.0
        DynamicGridSpec {
            step_pt: base_step / 4.0, // 5.0 pt micro-grid
            alpha: 0.18 + 0.15 * t,
            major_step_pt: base_step * 1.25,
        }
    }
}

pub use crate::preferences::EraserMode;

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
    LaserPointer,
}

/// Singolo punto campionato nella scia volatile del puntatore laser con timestamp assoluto.
#[derive(Debug, Clone)]
pub struct LaserPoint {
    pub x: f64,
    pub y: f64,
    pub timestamp_ns: u64,
}

/// Buffer volatile della scia del puntatore laser con stato di disegno attivo.
#[derive(Debug, Clone, Default)]
pub struct LaserTrail {
    pub points: Vec<LaserPoint>,
    pub is_active: bool,
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
    /// Flag che indica se è in corso un'operazione di trascinamento (drag translation o scaling).
    pub is_dragging: bool,
    /// Tipologia di trascinamento attiva (None, Translate o ScaleHandle).
    pub drag_kind: SelectionDragKind,
    /// Punto iniziale del drag in coordinate documento.
    pub drag_start_doc: Option<(f64, f64)>,
    /// Offset corrente di trascinamento (delta_x, delta_y) in coordinate documento.
    pub drag_offset: (f64, f64),
    /// Punto perno opposto fisso per il ridimensionamento (in coordinate documento).
    pub scale_anchor: (f64, f64),
    /// Fattori correnti di scala (scale_x, scale_y) durante il drag della maniglia.
    pub scale_factors: (f64, f64),
    /// Indica se il rapporto d'aspetto deve essere bloccato (true di default per gli angoli).
    pub lock_aspect_ratio: bool,
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
            drag_kind: SelectionDragKind::None,
            drag_start_doc: None,
            drag_offset: (0.0, 0.0),
            scale_anchor: (0.0, 0.0),
            scale_factors: (1.0, 1.0),
            lock_aspect_ratio: true,
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
        self.drag_kind = SelectionDragKind::None;
        self.drag_start_doc = None;
        self.drag_offset = (0.0, 0.0);
        self.scale_anchor = (0.0, 0.0);
        self.scale_factors = (1.0, 1.0);
        self.lock_aspect_ratio = true;
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

    /// Esegue l'hit-test gerarchico a schermo:
    /// 1. 4 angoli entro un raggio di 15px in coordinate schermo.
    /// 2. 4 pillole mediane (se la dimensione a schermo è >= 32 px).
    /// 3. Grip centrale di traslazione e corpo interno del bounding box.
    /// 4. None.
    pub fn hit_test(&self, doc_pt: (f64, f64), vp: &Viewport) -> SelectionHitTestResult {
        let bbox = match self.bbox {
            Some(ref b) => b,
            None => return SelectionHitTestResult::None,
        };

        let screen_pt = vp.document_to_screen(doc_pt);
        let s_min = vp.document_to_screen((bbox.min_x, bbox.min_y));
        let s_max = vp.document_to_screen((bbox.max_x, bbox.max_y));
        let s_left = s_min.0.min(s_max.0);
        let s_right = s_min.0.max(s_max.0);
        let s_top = s_min.1.min(s_max.1);
        let s_bottom = s_min.1.max(s_max.1);
        let sw = s_right - s_left;
        let sh = s_bottom - s_top;

        // Raggi di aggancio tattili (pixel schermo)
        const CORNER_HIT_RADIUS_SQ: f64 = 15.0 * 15.0; // 15 px
        const PILL_HIT_X: f64 = 14.0;
        const PILL_HIT_Y: f64 = 10.0;

        // 1. Priorità assoluta: i 4 vertici d'angolo
        let corners = [
            (HandleId::TopLeft, s_left, s_top),
            (HandleId::TopRight, s_right, s_top),
            (HandleId::BottomRight, s_right, s_bottom),
            (HandleId::BottomLeft, s_left, s_bottom),
        ];
        for (id, cx, cy) in corners {
            let dx = screen_pt.0 - cx;
            let dy = screen_pt.1 - cy;
            if dx * dx + dy * dy <= CORNER_HIT_RADIUS_SQ {
                return SelectionHitTestResult::Handle(id);
            }
        }

        // 2. Se il box ha dimensioni sufficienti (>= 32 px a schermo), controlla i 4 punti mediani
        if sw >= 32.0 && sh >= 32.0 {
            let mid_x = (s_left + s_right) / 2.0;
            let mid_y = (s_top + s_bottom) / 2.0;

            // TopMiddle e BottomMiddle (pillola orizzontale)
            if (screen_pt.0 - mid_x).abs() <= PILL_HIT_X
                && (screen_pt.1 - s_top).abs() <= PILL_HIT_Y
            {
                return SelectionHitTestResult::Handle(HandleId::TopMiddle);
            }
            if (screen_pt.0 - mid_x).abs() <= PILL_HIT_X
                && (screen_pt.1 - s_bottom).abs() <= PILL_HIT_Y
            {
                return SelectionHitTestResult::Handle(HandleId::BottomMiddle);
            }

            // LeftMiddle e RightMiddle (pillola verticale)
            if (screen_pt.0 - s_left).abs() <= PILL_HIT_Y
                && (screen_pt.1 - mid_y).abs() <= PILL_HIT_X
            {
                return SelectionHitTestResult::Handle(HandleId::LeftMiddle);
            }
            if (screen_pt.0 - s_right).abs() <= PILL_HIT_Y
                && (screen_pt.1 - mid_y).abs() <= PILL_HIT_X
            {
                return SelectionHitTestResult::Handle(HandleId::RightMiddle);
            }
        }

        // 3. Controllo grip centrale di traslazione (raggio 15 px a schermo)
        let mid_x = (s_left + s_right) / 2.0;
        let mid_y = (s_top + s_bottom) / 2.0;
        let dx_grip = screen_pt.0 - mid_x;
        let dy_grip = screen_pt.1 - mid_y;
        if dx_grip * dx_grip + dy_grip * dy_grip <= 15.0 * 15.0 {
            return SelectionHitTestResult::Body;
        }

        // 4. Controllo corpo interno del bounding box con tolleranza di 8 pt doc
        let tolerance = 8.0 / vp.zoom;
        if bbox.expand(tolerance).contains_point(doc_pt.0, doc_pt.1) {
            return SelectionHitTestResult::Body;
        }

        // 5. Fuori da tutto
        SelectionHitTestResult::None
    }

    /// Helper che calcola i fattori di scala richiamando la logica geometrica authoritative di `selection.rs`.
    pub fn compute_scale_factors(
        handle: HandleId,
        bbox: &Rect,
        cur_doc_pt: (f64, f64),
        lock_aspect: bool,
    ) -> ((f64, f64), (f64, f64)) {
        compute_scale_factors(handle, bbox, cur_doc_pt, lock_aspect)
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
    pub eraser_mode: EraserMode,
    /// Raggio della gomma in pixel a schermo (default 16.0 px, clamp 4.0..=64.0).
    pub eraser_radius: f64,
    /// Posizione corrente a schermo (x, y) del puntatore della gomma per l'indicatore live.
    pub eraser_pointer: Option<(f64, f64)>,
    /// Ultima posizione documento (x, y) della gomma durante il trascinamento continuo (per interpolazione priva di salti).
    pub last_eraser_doc_pos: Option<(f64, f64)>,
    pub selection: SelectionState,
    pub active_builder: Option<StrokeBuilder>,
    pub active_device: Option<DeviceKind>,
    pub palm_engine: PalmRejectionEngine,
    pub pipeline_mode: PipelineMode,
    pub current_style: StrokeStyle,
    pub next_stroke_id: u64,
    pub last_sample_time: Option<Instant>,
    pub last_stylus_time: Option<Instant>,
    pub mouse_drag_start: Option<(f64, f64)>,
    pub mouse_pan_start: Option<(f64, f64)>,
    pub tip_pressure_threshold: f64,
    pub pdf_cache: std::cell::RefCell<crate::pdf_cache::PdfCacheManager>,
    pub laser_trail: LaserTrail,
    pub laser_tick_active: bool,
    pub laser_width: f64,
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
            eraser_mode: EraserMode::WholeStroke,
            eraser_radius: 16.0,
            eraser_pointer: None,
            last_eraser_doc_pos: None,
            selection: SelectionState::default(),
            active_builder: None,
            active_device: None,
            palm_engine: PalmRejectionEngine::new(TouchMode::GesturesOnly),
            pipeline_mode: PipelineMode::CatmullRomSpline,
            current_style: StrokeStyle::default(),
            next_stroke_id: 1,
            last_sample_time: None,
            last_stylus_time: None,
            mouse_drag_start: None,
            mouse_pan_start: None,
            tip_pressure_threshold: 0.06,
            pdf_cache: std::cell::RefCell::new(crate::pdf_cache::PdfCacheManager::default_120hz()),
            laser_trail: LaserTrail::default(),
            laser_tick_active: false,
            laser_width: 5.0,
        }
    }

    /// Alloca un nuovo ItemId strettamente univoco sincronizzando next_stroke_id, history e document.
    pub fn alloc_item_id(&mut self) -> u64 {
        let max_doc_id = self
            .document
            .pages
            .iter()
            .flat_map(|p| p.items.iter())
            .map(|it| it.id)
            .max()
            .unwrap_or(0);
        let id = self
            .next_stroke_id
            .max(self.history.next_item_id())
            .max(max_doc_id + 1);
        self.next_stroke_id = id + 1;
        self.history.set_next_item_id(id + 1);
        id
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

pub type ZoomCallback = Box<dyn Fn(f64)>;
pub type ToolCallback = Box<dyn Fn(ActiveTool)>;

#[derive(Clone)]
pub struct NoteCanvas {
    area: DrawingArea,
    state: Rc<RefCell<CanvasState>>,
    hud: Rc<DiagnosticHud>,
    app_start: Instant,
    zoom_callbacks: Rc<RefCell<Vec<ZoomCallback>>>,
    tool_callbacks: Rc<RefCell<Vec<ToolCallback>>>,
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
        let zoom_callbacks = Rc::new(RefCell::new(Vec::new()));
        let tool_callbacks = Rc::new(RefCell::new(Vec::new()));

        let canvas = Self {
            area,
            state,
            hud,
            app_start,
            zoom_callbacks,
            tool_callbacks,
        };
        canvas.setup_drawing();
        canvas.setup_capture_controller();
        canvas.setup_stylus_controller();
        canvas.setup_mouse_controller();
        canvas.setup_motion_controller();
        canvas.setup_scroll_and_zoom_controllers();
        canvas.setup_touch_long_press_controller();
        canvas
    }

    pub fn connect_zoom_changed<F: Fn(f64) + 'static>(&self, f: F) {
        self.zoom_callbacks.borrow_mut().push(Box::new(f));
    }

    fn notify_zoom_changed(&self, zoom: f64) {
        for cb in self.zoom_callbacks.borrow().iter() {
            cb(zoom);
        }
    }

    pub fn connect_tool_changed<F: Fn(ActiveTool) + 'static>(&self, f: F) {
        self.tool_callbacks.borrow_mut().push(Box::new(f));
    }

    pub fn notify_tool_changed(&self, tool: ActiveTool) {
        for cb in self.tool_callbacks.borrow().iter() {
            cb(tool);
        }
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
        if tool != ActiveTool::Eraser {
            st.eraser_pointer = None;
        }
        if tool != ActiveTool::LaserPointer {
            st.laser_trail.points.clear();
            st.laser_trail.is_active = false;
        }
        let tool_name = match tool {
            ActiveTool::Pen => "Penna",
            ActiveTool::Eraser => match st.eraser_mode {
                EraserMode::WholeStroke => "Gomma (Tratto Intero)",
                EraserMode::Segment => "Gomma (Per Segmento)",
            },
            ActiveTool::Highlighter => "Evidenziatore",
            ActiveTool::Text => "Testo",
            ActiveTool::Line => "Linea Dritta",
            ActiveTool::Shapes => "Forme",
            ActiveTool::Select => "Selezione (Lazo / Rettangolo)",
            ActiveTool::LaserPointer => "Puntatore Laser",
        };
        self.hud
            .update_metrics(|m| m.current_tool = tool_name.to_string());
        self.hud.update_view();
        drop(st);
        self.area.queue_draw();
        self.notify_tool_changed(tool);
    }

    pub fn active_tool(&self) -> ActiveTool {
        self.state.borrow().active_tool
    }

    pub fn laser_width(&self) -> f64 {
        self.state.borrow().laser_width
    }

    pub fn set_laser_width(&self, width: f64) {
        self.state.borrow_mut().laser_width = width.clamp(1.5, 24.0);
        self.area.queue_draw();
    }

    pub fn eraser_mode(&self) -> EraserMode {
        self.state.borrow().eraser_mode
    }

    pub fn set_eraser_mode(&self, mode: EraserMode) {
        let mut st = self.state.borrow_mut();
        st.eraser_mode = mode;
        let tool_name = match mode {
            EraserMode::WholeStroke => "Gomma (Tratto Intero)",
            EraserMode::Segment => "Gomma (Per Segmento)",
        };
        self.hud
            .update_metrics(|m| m.current_tool = tool_name.to_string());
        self.hud.update_view();
        self.area.queue_draw();
    }

    /// Restituisce il raggio corrente della gomma in pixel a schermo.
    pub fn eraser_radius(&self) -> f64 {
        self.state.borrow().eraser_radius
    }

    /// Imposta il raggio della gomma in pixel a schermo, con clamping rigido in [4.0, 64.0],
    /// e richiede il ridisegno del canvas per aggiornare immediatamente l'indicatore live.
    pub fn set_eraser_radius(&self, radius: f64) {
        let clamped = if radius.is_finite() {
            radius.clamp(4.0, 64.0)
        } else {
            16.0
        };
        self.state.borrow_mut().eraser_radius = clamped;
        self.area.queue_draw();
    }

    /// Restituisce la coordinata a schermo corrente del puntatore della gomma (se attiva).
    pub fn eraser_pointer(&self) -> Option<(f64, f64)> {
        self.state.borrow().eraser_pointer
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

    /// Centralizza il riordino z-index degli elementi selezionati.
    /// Se `to_front` è true, sposta la selezione sopra tutti gli altri elementi ("Sposta sopra").
    /// Se `to_front` è false, sposta la selezione sotto tutti gli altri elementi ("Sposta sotto").
    pub fn reorder_selected_z_index(&self, to_front: bool) -> bool {
        let mut st_guard = self.state.borrow_mut();
        let st = &mut *st_guard;
        if !st.selection.has_selection() {
            return false;
        }
        let page_idx = match st.selection.page_index {
            Some(idx) => idx,
            None => return false,
        };

        let sel_set: std::collections::HashSet<ItemId> =
            st.selection.selected_ids.iter().copied().collect();

        let page = match st.document.pages.get_mut(page_idx) {
            Some(p) => p,
            None => return false,
        };

        let mut non_selected: Vec<&Item> = Vec::new();
        let mut selected: Vec<&Item> = Vec::new();
        for item in &page.items {
            if sel_set.contains(&item.id) {
                selected.push(item);
            } else {
                non_selected.push(item);
            }
        }

        if selected.is_empty() || non_selected.is_empty() {
            return false;
        }

        let (first_group, second_group): (&[&Item], &[&Item]) = if to_front {
            (&non_selected[..], &selected[..])
        } else {
            (&selected[..], &non_selected[..])
        };

        let mut changes: Vec<(ItemId, u64, u64)> = Vec::new();
        let mut next_z: u64 = 1;
        for group in [first_group, second_group] {
            for it in group {
                if it.z_index != next_z {
                    changes.push((it.id, it.z_index, next_z));
                }
                next_z += 1;
            }
        }

        if changes.is_empty() {
            return false;
        }

        let page_id = page.id;
        let cmd = DocCommand::ReorderItemsZIndex { page_id, changes };
        st.history.execute(cmd, &mut st.document);
        st.persist_changes();
        drop(st_guard);
        self.area.queue_draw();
        true
    }

    /// Sposta gli elementi selezionati in cima a tutti gli altri elementi della pagina ("Sposta sopra").
    pub fn bring_to_front_selected(&self) -> bool {
        self.reorder_selected_z_index(true)
    }

    /// Sposta gli elementi selezionati sotto a tutti gli altri elementi della pagina ("Sposta sotto").
    pub fn send_to_back_selected(&self) -> bool {
        self.reorder_selected_z_index(false)
    }

    /// Duplica gli elementi selezionati creandone una copia con offset e auto-selezionandoli ("Duplica").
    pub fn duplicate_selected(&self) -> bool {
        if !self.copy_selected() {
            return false;
        }
        self.paste_from_clipboard()
    }

    /// Mostra il menu contestuale della selezione alle coordinate specificate.
    pub fn show_selection_context_menu(&self, screen_x: f64, screen_y: f64) {
        {
            let st = self.state.borrow();
            if !st.selection.has_selection() {
                return;
            }
        }

        let popover = gtk4::Popover::builder()
            .autohide(true)
            .has_arrow(true)
            .position(gtk4::PositionType::Bottom)
            .css_classes(["dms-popover", "dms-context-popover"])
            .build();
        popover.set_parent(&self.area);
        popover.set_pointing_to(Some(&gdk::Rectangle::new(
            screen_x as i32,
            screen_y as i32,
            1,
            1,
        )));

        let menu_box = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Vertical)
            .spacing(2)
            .margin_start(4)
            .margin_end(4)
            .margin_top(4)
            .margin_bottom(4)
            .build();

        // 1. Sposta sopra
        let item_up = Self::create_context_menu_button("Sposta sopra", IconKind::ChevronUp, false);
        let pop_up = popover.clone();
        let canvas_up = self.clone();
        item_up.connect_clicked(move |_| {
            pop_up.popdown();
            canvas_up.bring_to_front_selected();
        });
        menu_box.append(&item_up);

        // 2. Sposta sotto
        let item_down =
            Self::create_context_menu_button("Sposta sotto", IconKind::ChevronDown, false);
        let pop_down = popover.clone();
        let canvas_down = self.clone();
        item_down.connect_clicked(move |_| {
            pop_down.popdown();
            canvas_down.send_to_back_selected();
        });
        menu_box.append(&item_down);

        // Separatore
        let sep1 = gtk4::Separator::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .css_classes(["dms-menu-separator"])
            .build();
        menu_box.append(&sep1);

        // 3. Copia
        let item_copy = Self::create_context_menu_button("Copia", IconKind::Copy, false);
        let pop_copy = popover.clone();
        let canvas_copy = self.clone();
        item_copy.connect_clicked(move |_| {
            pop_copy.popdown();
            canvas_copy.copy_selected();
        });
        menu_box.append(&item_copy);

        // 4. Duplica
        let item_dup = Self::create_context_menu_button("Duplica", IconKind::Copy, false);
        let pop_dup = popover.clone();
        let canvas_dup = self.clone();
        item_dup.connect_clicked(move |_| {
            pop_dup.popdown();
            canvas_dup.duplicate_selected();
        });
        menu_box.append(&item_dup);

        // Separatore
        let sep2 = gtk4::Separator::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .css_classes(["dms-menu-separator"])
            .build();
        menu_box.append(&sep2);

        // 5. Elimina
        let item_del = Self::create_context_menu_button("Elimina", IconKind::Trash, true);
        let pop_del = popover.clone();
        let canvas_del = self.clone();
        item_del.connect_clicked(move |_| {
            pop_del.popdown();
            canvas_del.delete_selected();
        });
        menu_box.append(&item_del);

        popover.set_child(Some(&menu_box));

        let area_cleanup = self.area.clone();
        popover.connect_closed(move |p| {
            p.unparent();
            area_cleanup.queue_draw();
        });

        popover.popup();
    }

    fn create_context_menu_button(
        label_text: &str,
        icon: IconKind,
        is_destructive: bool,
    ) -> gtk4::Button {
        let hbox = gtk4::Box::builder()
            .orientation(gtk4::Orientation::Horizontal)
            .spacing(8)
            .build();
        hbox.append(&create_icon(icon, 16, None));
        let lbl = gtk4::Label::builder()
            .label(label_text)
            .xalign(0.0)
            .hexpand(true)
            .build();
        hbox.append(&lbl);

        let mut classes = vec!["dms-menu-item-btn"];
        if is_destructive {
            classes.push("destructive-action");
        }

        gtk4::Button::builder()
            .child(&hbox)
            .css_classes(classes)
            .halign(gtk4::Align::Fill)
            .build()
    }

    /// Copia gli elementi selezionati negli appunti (in-memory e GdkClipboard di sistema).
    pub fn copy_selected(&self) -> bool {
        let mut st = self.state.borrow_mut();
        if !st.selection.has_selection() {
            return false;
        }

        let page_idx = match st.selection.page_index {
            Some(idx) => idx,
            None => return false,
        };

        let bbox = match st.selection.bbox {
            Some(b) => b,
            None => return false,
        };

        let id_set: std::collections::HashSet<ItemId> =
            st.selection.selected_ids.iter().copied().collect();
        let doc_id = st.document.id;
        let doc_format = st.document.format;
        let vp_offset = (st.viewport.offset_x, st.viewport.offset_y);

        let page = match st.document.pages.get_mut(page_idx) {
            Some(p) => p,
            None => return false,
        };

        let page_size = (page.width, page.height);
        let mut copied_items: Vec<uninote_core::Item> = Vec::new();

        for item in &mut page.items {
            if id_set.contains(&item.id) {
                let mut it_clone = item.clone();
                if let ItemKind::Stroke(ref mut s) = it_clone.kind {
                    s.ensure_cache();
                }
                copied_items.push(it_clone);
            }
        }

        if copied_items.is_empty() {
            return false;
        }

        let center = (
            (bbox.min_x + bbox.max_x) / 2.0,
            (bbox.min_y + bbox.max_y) / 2.0,
        );

        let data = crate::clipboard::ClipboardData {
            source_format: doc_format,
            source_doc_id: Some(doc_id),
            source_page_index: Some(page_idx),
            source_page_size: Some(page_size),
            original_bbox: bbox,
            original_center: center,
            items: copied_items,
            paste_count: 0,
            source_viewport_offset: Some(vp_offset),
        };

        crate::clipboard::set_clipboard_data(data);
        true
    }

    /// Taglia gli elementi selezionati (copia negli appunti + cancellazione atomica via DocCommand::EraseItems).
    pub fn cut_selected(&self) -> bool {
        if !self.copy_selected() {
            return false;
        }
        let mut st = self.state.borrow_mut();
        let deleted = Self::delete_selected_items(&mut st);
        drop(st);
        if deleted {
            self.area.queue_draw();
        }
        deleted
    }

    /// Inserisce un'immagine da buffer di byte (PNG, JPEG, WebP) con calcolo SHA-256,
    /// posizionamento intelligente centrato nella viewport, salvataggio asset su SQLite e auto-selezione.
    pub fn insert_image_bytes(
        &self,
        bytes: &[u8],
        mime_type: Option<&str>,
    ) -> Result<ItemId, String> {
        if bytes.is_empty() {
            return Err("Cannot insert empty image bytes".to_string());
        }

        // 1. Riconoscimento del MIME type
        let detected_mime = if let Some(m) = mime_type {
            m.to_string()
        } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            "image/png".to_string()
        } else if bytes.starts_with(b"\xff\xd8\xff") {
            "image/jpeg".to_string()
        } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
            "image/webp".to_string()
        } else {
            "image/png".to_string()
        };

        // 2. Decodifica della texture / superficie per ricavare le dimensioni originali
        let (orig_w, orig_h, png_for_cache) = {
            let glib_bytes = glib::Bytes::from(bytes);
            if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
                let w = texture.width() as u32;
                let h = texture.height() as u32;
                let png = if detected_mime == "image/png" {
                    None
                } else {
                    Some(texture.save_to_png_bytes().to_vec())
                };
                (w, h, png)
            } else {
                // Fallback deterministico headless tramite Cairo PNG decoder
                let mut cursor = std::io::Cursor::new(bytes);
                let surf = cairo::ImageSurface::create_from_png(&mut cursor)
                    .map_err(|e| format!("Failed to decode image texture: {e:?}"))?;
                let w = surf.width() as u32;
                let h = surf.height() as u32;
                (w, h, None)
            }
        };

        if orig_w == 0 || orig_h == 0 {
            return Err("Decoded image has zero dimensions".to_string());
        }

        // 3. Calcolo hash crittografico SHA-256 per persistenza e deduplicazione
        let hash = glib::compute_checksum_for_data(glib::ChecksumType::Sha256, bytes)
            .map(|s| s.to_string())
            .ok_or_else(|| "Failed to compute SHA-256 checksum for image data".to_string())?;

        // Pre-popolamento ImageCacheManager globale per rendering a 120 Hz immediato
        {
            let cache_rc = crate::image_cache::get_global_image_cache();
            let mut cache = cache_rc.borrow_mut();
            if let Some(ref png_bytes) = png_for_cache {
                let _ = cache.insert_from_png_bytes(&hash, png_bytes);
            } else {
                let _ = cache.insert_from_bytes(&hash, bytes);
            }
        }

        // 4. Risoluzione centro viewport e calcolo bounding box (Whiteboard vs Notebook)
        let st_ref = self.state.borrow();
        if st_ref.document.pages.is_empty() {
            return Err("Document contains no pages".to_string());
        }

        let area_w = (self.area.width() as f64).max(800.0);
        let area_h = (self.area.height() as f64).max(600.0);
        let screen_center = (area_w / 2.0, area_h / 2.0);
        let (vp_center_x, vp_center_y) = st_ref.viewport.screen_to_document(screen_center);

        if !vp_center_x.is_finite() || !vp_center_y.is_finite() {
            return Err("Viewport center coordinate is not finite".to_string());
        }

        let (target_page_idx, target_page_id, bbox) = match st_ref.document.format {
            DocumentFormat::Whiteboard => {
                let vis_w = area_w / st_ref.viewport.zoom;
                let vis_h = area_h / st_ref.viewport.zoom;
                let max_w = (vis_w * 0.6).max(100.0);
                let max_h = (vis_h * 0.6).max(100.0);
                let mut w = orig_w as f64;
                let mut h = orig_h as f64;
                if w > max_w || h > max_h {
                    let scale = (max_w / w).min(max_h / h);
                    w *= scale;
                    h *= scale;
                }
                let page_id = st_ref.document.pages[0].id;
                let b = Rect::new(
                    vp_center_x - w / 2.0,
                    vp_center_y - h / 2.0,
                    vp_center_x + w / 2.0,
                    vp_center_y + h / 2.0,
                );
                (0, page_id, b)
            }
            DocumentFormat::Notebook => {
                let page_idx =
                    Viewport::find_page_index_at_y(vp_center_y, st_ref.document.pages.len())
                        .min(st_ref.document.pages.len().saturating_sub(1));
                let page = &st_ref.document.pages[page_idx];
                let page_id = page.id;
                let page_rect = Viewport::page_rect(page_idx);

                let max_w = (page_rect.width - 20.0).max(100.0);
                let max_h = (page_rect.height - 20.0).max(100.0);
                let mut w = orig_w as f64;
                let mut h = orig_h as f64;
                if w > max_w || h > max_h {
                    let scale = (max_w / w).min(max_h / h);
                    w *= scale;
                    h *= scale;
                }

                // Centratura orizzontale intelligente con clamping entro i margini
                let min_cx = page_rect.x + w / 2.0 + 10.0;
                let max_cx = page_rect.x + page_rect.width - w / 2.0 - 10.0;
                let cx = if min_cx <= max_cx {
                    if vp_center_x >= page_rect.x && vp_center_x <= page_rect.x + page_rect.width {
                        vp_center_x.clamp(min_cx, max_cx)
                    } else {
                        page_rect.x + page_rect.width / 2.0
                    }
                } else {
                    page_rect.x + page_rect.width / 2.0
                };

                // Centratura verticale intelligente clampata nel foglio A4
                let min_cy = page_rect.y + h / 2.0 + 10.0;
                let max_cy = page_rect.y + page_rect.height - h / 2.0 - 10.0;
                let cy = if min_cy <= max_cy {
                    vp_center_y.clamp(min_cy, max_cy)
                } else {
                    page_rect.y + page_rect.height / 2.0
                };

                let b = Rect::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
                (page_idx, page_id, b)
            }
        };
        drop(st_ref);

        // 5. Modifica atomica dello stato: allocazione nuovo ID, comando DocCommand::AddImage e salvataggio asset
        let mut st_guard = self.state.borrow_mut();
        let st = &mut *st_guard;

        let new_item_id = st.alloc_item_id();
        let target_page = match st.document.pages.get(target_page_idx) {
            Some(p) => p,
            None => return Err("Target page not found".to_string()),
        };
        let max_z = target_page
            .items
            .iter()
            .map(|it| it.z_index)
            .max()
            .unwrap_or(0);
        let new_z_index = max_z + 1;

        let image_item = uninote_core::ImageItem::with_id(
            uuid::Uuid::new_v4(),
            target_page_id,
            hash.clone(),
            bbox,
            new_z_index as i64,
            1.0,
            orig_w,
            orig_h,
        );

        let cmd = DocCommand::add_image(target_page_id, new_item_id, new_z_index, image_item);
        st.history.execute(cmd, &mut st.document);
        st.persist_changes();

        // Persistenza dell'asset su SQLite (sincrona immediata se file_path disponibile e/o asincrona via worker)
        if let Some(ref path) = st.file_path {
            if let Ok(store) = DocumentStore::open_or_create(path) {
                let _ = store.save_asset(&hash, &detected_mime, bytes);
            }
        }
        if let Some(ref worker) = st.storage_worker {
            worker.save_asset(hash.clone(), detected_mime.clone(), bytes.to_vec(), None);
        }

        // 6. Auto-attivazione della selezione sul nuovo elemento immagine
        st.selection.page_index = Some(target_page_idx);
        st.selection.selected_ids = vec![new_item_id];
        st.selection.bbox = Some(bbox);
        st.selection.is_dragging = false;
        st.selection.drag_kind = SelectionDragKind::None;
        st.selection.drag_offset = (0.0, 0.0);
        st.active_tool = ActiveTool::Select;

        drop(st_guard);
        self.hud
            .update_metrics(|m| m.current_tool = "Selezione (Lazo / Rettangolo)".into());
        self.hud.update_view();
        self.area.queue_draw();
        self.notify_tool_changed(ActiveTool::Select);

        Ok(new_item_id)
    }

    /// Interroga in modo asincrono la clipboard GDK di sistema per texture o immagini raster,
    /// serializza il payload in PNG ed esegue l'inserimento programmato.
    pub fn paste_image(&self) -> bool {
        let display = match gdk::Display::default() {
            Some(d) => d,
            None => return false,
        };
        let clipboard = display.clipboard();
        let formats = clipboard.formats();

        let has_image = formats.contains_type(gdk::Texture::static_type())
            || crate::clipboard::IMAGE_MIME_TYPES
                .iter()
                .any(|mime| formats.contain_mime_type(mime))
            || formats.mime_types().iter().any(|m| m.starts_with("image/"));

        if !has_image {
            return false;
        }

        let canvas_clone = self.clone();
        crate::clipboard::read_clipboard_image_async(move |payload_opt| {
            if let Some(payload) = payload_opt {
                if let Err(err) =
                    canvas_clone.insert_image_bytes(&payload.bytes, Some(&payload.mime_type))
                {
                    eprintln!("[UniNote NoteCanvas] Failed to insert pasted image: {err}");
                }
            }
        });

        true
    }

    /// Collega o aggiorna il worker di storage asincrono.
    pub fn set_storage_worker(&self, worker: StorageWorker) {
        self.state.borrow_mut().storage_worker = Some(worker);
    }

    /// Incolla gli elementi dagli appunti con posizionamento intelligente e auto-attivazione della selezione.
    pub fn paste_from_clipboard(&self) -> bool {
        // 1. Se la clipboard di sistema contiene un'immagine esterna o texture GDK, delega a paste_image
        if let Some(display) = gdk::Display::default() {
            let sys_cb = display.clipboard();
            let formats = sys_cb.formats();
            let has_uninote_strokes =
                formats.contain_mime_type(crate::clipboard::MIME_UNINOTE_SELECTION);
            let has_image = formats.contains_type(gdk::Texture::static_type())
                || crate::clipboard::IMAGE_MIME_TYPES
                    .iter()
                    .any(|mime| formats.contain_mime_type(mime))
                || formats.mime_types().iter().any(|m| m.starts_with("image/"));

            if has_image && !has_uninote_strokes {
                return self.paste_image();
            }
        }

        // 2. Recupero tratti/elementi UniNote dagli appunti interni
        let clipboard_opt = crate::clipboard::get_clipboard_data();
        let clipboard = match clipboard_opt {
            Some(cb) if !cb.items.is_empty() => cb,
            _ => return self.paste_image(),
        };

        let st = self.state.borrow();
        if st.document.pages.is_empty() {
            return false;
        }

        let area_w = (self.area.width() as f64).max(800.0);
        let area_h = (self.area.height() as f64).max(600.0);
        let screen_center = (area_w / 2.0, area_h / 2.0);
        let (vp_center_x, vp_center_y) = st.viewport.screen_to_document(screen_center);

        // NaN / Finite guard per prevenire corruzione dello stato o crash Cairo
        if !clipboard.original_center.0.is_finite()
            || !clipboard.original_center.1.is_finite()
            || !vp_center_x.is_finite()
            || !vp_center_y.is_finite()
        {
            return false;
        }

        let is_same_doc = clipboard.source_doc_id == Some(st.document.id);
        let pan_moved = if let Some(src_offset) = clipboard.source_viewport_offset {
            (st.viewport.offset_x - src_offset.0).abs() > 1.0
                || (st.viewport.offset_y - src_offset.1).abs() > 1.0
        } else {
            false
        };

        // 1. Risoluzione della pagina di destinazione
        let page_at_vp_center =
            Viewport::find_page_index_at_y(vp_center_y, st.document.pages.len());

        let target_page_idx = if st.document.format == DocumentFormat::Whiteboard {
            0
        } else if is_same_doc && !pan_moved {
            if let Some(sel_idx) = st.selection.page_index {
                let sel_rect = Viewport::page_rect(sel_idx);
                let vp_in_sel_page = vp_center_y >= sel_rect.y
                    && vp_center_y <= sel_rect.y + sel_rect.height + uninote_core::PAGE_GAP_PT;
                if vp_in_sel_page {
                    sel_idx.min(st.document.pages.len().saturating_sub(1))
                } else {
                    page_at_vp_center
                }
            } else {
                page_at_vp_center
            }
        } else {
            page_at_vp_center
        };

        let target_page_id = match st.document.pages.get(target_page_idx) {
            Some(p) => p.id,
            None => return false,
        };

        // 2. Calcolo dello spostamento (Smart Positioning)
        let is_same_format = st.document.format == clipboard.source_format;
        let is_same_page = clipboard.source_page_index == Some(target_page_idx);

        let count = clipboard.paste_count;
        let (dx, dy) = if is_same_doc && is_same_format && is_same_page && !pan_moved {
            // Stessa vista/pagina/documento: offset a cascata (+25.0 pt, +50.0 pt, +75.0 pt)
            let offset = ((count + 1) as f64) * 25.0;
            (offset, offset)
        } else {
            // Spostamento di pan, cambio pagina o cambio documento: centra nella viewport visibile corrente
            let base_dx = vp_center_x - clipboard.original_center.0;
            let base_dy = vp_center_y - clipboard.original_center.1;
            let cascade = (count as f64) * 25.0;
            (base_dx + cascade, base_dy + cascade)
        };

        if !dx.is_finite() || !dy.is_finite() {
            return false;
        }

        // Aggiorna il contatore di incolla
        drop(st);
        crate::clipboard::advance_paste_count();
        let mut st_guard = self.state.borrow_mut();
        let st = &mut *st_guard;

        // 3. Allocazione nuovi ItemId univoci, nuovo z_index e traslazione in-place
        let target_page = match st.document.pages.get(target_page_idx) {
            Some(p) => p,
            None => return false,
        };
        let max_z = target_page
            .items
            .iter()
            .map(|it| it.z_index)
            .max()
            .unwrap_or(0);

        let mut new_items: Vec<uninote_core::Item> = Vec::with_capacity(clipboard.items.len());
        let mut new_ids: Vec<ItemId> = Vec::with_capacity(clipboard.items.len());
        let mut union_bbox: Option<Rect> = None;

        for (idx, item) in clipboard.items.iter().enumerate() {
            let new_id = st.alloc_item_id();
            let mut item_clone = item.clone();
            item_clone.id = new_id;
            item_clone.z_index = max_z + (idx as u64) + 1;

            match &mut item_clone.kind {
                ItemKind::Stroke(ref mut stroke) => {
                    stroke.id = new_id;
                    stroke.translate(dx, dy);
                }
                ItemKind::Image(ref mut image) => {
                    image.page_id = target_page_id;
                    image.z_index = item_clone.z_index as i64;
                    image.translate(dx, dy);
                }
            }
            if let Some(b) = item_clone.bounding_box() {
                union_bbox = Some(match union_bbox {
                    Some(existing) => existing.union(&b),
                    None => b,
                });
            }

            new_ids.push(new_id);
            new_items.push(item_clone);
        }

        // 4. Esecuzione comando atomico DocCommand::PasteItems
        let cmd = DocCommand::PasteItems {
            page_id: target_page_id,
            items: new_items,
        };
        st.history.execute(cmd, &mut st.document);
        st.persist_changes();

        // 5. MANDATORY: Auto-attivazione della selezione sui tratti appena incollati!
        st.selection.page_index = Some(target_page_idx);
        st.selection.selected_ids = new_ids;
        st.selection.bbox = union_bbox;
        st.selection.is_dragging = false;
        st.selection.drag_offset = (0.0, 0.0);
        st.active_tool = ActiveTool::Select;

        drop(st_guard);
        self.hud
            .update_metrics(|m| m.current_tool = "Selezione (Lazo / Rettangolo)".into());
        self.hud.update_view();
        self.area.queue_draw();
        self.notify_tool_changed(ActiveTool::Select);
        true
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
                if let Some(bbox) = item.bounding_box() {
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
        let zoom = st.viewport.zoom;
        let pct = (zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.notify_zoom_changed(zoom);
        self.area.queue_draw();
    }

    pub fn zoom_out(&self) {
        let mut st = self.state.borrow_mut();
        let w = self.area.width() as f64;
        let h = self.area.height() as f64;
        st.viewport.zoom_at((w / 2.0, h / 2.0), 0.8);
        let zoom = st.viewport.zoom;
        let pct = (zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.notify_zoom_changed(zoom);
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
        let zoom = st.viewport.zoom;
        let pct = (zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.notify_zoom_changed(zoom);
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

    /// Centra orizzontalmente e posiziona la prima pagina (A4) del quaderno nel viewport.
    pub fn center_page_1(&self) {
        let mut st = self.state.borrow_mut();
        if st.document.format == DocumentFormat::Whiteboard {
            return;
        }
        let w = (self.area.width() as f64).max(800.0);
        st.viewport.reset_zoom_and_center(w);
        st.viewport.offset_y = 40.0;
        let zoom = st.viewport.zoom;
        let pct = (zoom * 100.0).round() as usize;
        drop(st);
        self.hud.update_metrics(|m| m.zoom_pct = pct);
        self.hud.update_view();
        self.notify_zoom_changed(zoom);
        self.area.queue_draw();
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

    /// Aggiorna la miniatura/copertina del documento su disco.
    pub fn update_thumbnail(&self) {
        let st = self.state.borrow();
        if let Some(ref path) = st.file_path {
            let meta = crate::preferences::RecentDocumentMeta {
                file_path: path.to_string_lossy().to_string(),
                title: st.document.title.clone(),
                format: st.document.format,
                last_modified_timestamp: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0),
                page_count: st.document.pages.len().max(1),
                thumbnail_path: None,
            };
            let _ = crate::dashboard::update_document_thumbnail_now(&meta, &st.document, None);
        }
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

    pub fn open_file(&self, path: PathBuf) -> Result<(), String> {
        let store = DocumentStore::open_or_create(&path).map_err(|e| e.to_string())?;
        let doc = store.load_document().map_err(|e| e.to_string())?;
        let max_id = doc.max_item_id();
        let mut st = self.state.borrow_mut();
        st.document = doc;
        st.history = CommandHistory::with_next_id(100, max_id + 1);
        st.next_stroke_id = max_id + 1;
        st.file_path = Some(path.clone());

        // Inizializza worker di storage asincrono per questo file
        if let Ok(worker) = StorageWorker::spawn(path, move |status| {
            if let uninote_storage::SaveStatus::Error(ref err) = status {
                eprintln!("[UniNote StorageWorker ERROR]: {}", err);
            }
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

        // Rigenera o aggiorna la miniatura copertina in cache
        let meta = crate::preferences::RecentDocumentMeta {
            file_path: path.to_string_lossy().to_string(),
            title: st.document.title.clone(),
            format: st.document.format,
            last_modified_timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0),
            page_count: st.document.pages.len().max(1),
            thumbnail_path: None,
        };
        let thumb_path = crate::dashboard::get_or_create_thumbnail_path(&meta);
        let _ = crate::dashboard::render_document_thumbnail(&st.document, &thumb_path);

        if let Ok(worker) = StorageWorker::spawn(path, move |status| {
            if let uninote_storage::SaveStatus::Error(ref err) = status {
                eprintln!("[UniNote StorageWorker ERROR]: {}", err);
            }
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

    /// Esporta il documento corrente in un file PDF vettoriale ad alta definizione con opzioni tematiche.
    pub fn export_pdf_with_options(
        &self,
        target_path: &std::path::Path,
        options: &crate::pdf_export::PdfExportOptions,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let st = self.state.borrow();
        crate::pdf_export::export_to_pdf_with_store(
            &st.document,
            target_path,
            options,
            st.file_path.as_deref(),
        )
    }

    /// Esporta il documento corrente in un file PDF vettoriale ad alta definizione.
    pub fn export_pdf(
        &self,
        target_path: &std::path::Path,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let st = self.state.borrow();
        crate::pdf_export::export_to_pdf_with_store(
            &st.document,
            target_path,
            &crate::pdf_export::PdfExportOptions::default(),
            st.file_path.as_deref(),
        )
    }

    /// Carica un nuovo documento sostituendo lo stato corrente e aggiornando l'HUD e la vista.
    pub fn load_document(&self, doc: uninote_core::Document) {
        let mut doc = doc;
        doc.rebuild_spatial_indices();
        let max_id = doc.max_item_id();
        let mut st = self.state.borrow_mut();
        st.document = doc;
        st.history = CommandHistory::with_next_id(100, max_id + 1);
        st.next_stroke_id = max_id + 1;
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
        let app_start = self.app_start;

        self.area.set_draw_func(move |_area, cr, width, height| {
            let start_render = Instant::now();
            let st = state_clone.borrow();
            let vp = st.viewport;
            let current_mode = st.pipeline_mode;
            let is_dark = libadwaita::StyleManager::default().is_dark();
            let mut visible_strokes = 0usize;

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

                        // Ombra morbida ad elevazione naturale multi-pass per foglio A4
                        let shadow_base_alpha = if is_dark { 0.20 } else { 0.05 };
                        // Pass 1: ampia e diffusa
                        cr.set_source_rgba(0.0, 0.0, 0.0, shadow_base_alpha * 0.7);
                        cr.rectangle(sx - 6.0, sy + 4.0, sw + 12.0, sh + 14.0);
                        let _ = cr.fill();
                        // Pass 2: media
                        cr.set_source_rgba(0.0, 0.0, 0.0, shadow_base_alpha * 1.3);
                        cr.rectangle(sx - 3.0, sy + 3.0, sw + 6.0, sh + 8.0);
                        let _ = cr.fill();
                        // Pass 3: contatto
                        cr.set_source_rgba(0.0, 0.0, 0.0, shadow_base_alpha * 2.2);
                        cr.rectangle(sx - 1.0, sy + 2.0, sw + 2.0, sh + 3.0);
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

                        // Tratti della pagina con Culling Spaziale R-Tree
                        let _ = cr.save();
                        cr.translate(vp.offset_x, vp.offset_y);
                        cr.scale(vp.zoom, vp.zoom);

                        let doc_left = -vp.offset_x / vp.zoom;
                        let doc_top = -vp.offset_y / vp.zoom;
                        let doc_right = (width as f64 - vp.offset_x) / vp.zoom;
                        let doc_bottom = (height as f64 - vp.offset_y) / vp.zoom;

                        let vis_min_x = doc_left.max(doc_rect.x);
                        let vis_min_y = doc_top.max(doc_rect.y);
                        let vis_max_x = doc_right.min(doc_rect.x + doc_rect.width);
                        let vis_max_y = doc_bottom.min(doc_rect.y + doc_rect.height);

                        if !page.spatial_index.is_empty() {
                            if vis_min_x <= vis_max_x && vis_min_y <= vis_max_y {
                                let visible_items = page
                                    .spatial_index
                                    .locate_in_envelope(vis_min_x, vis_min_y, vis_max_x, vis_max_y);
                                visible_strokes += visible_items.len();
                                for spatial_item in &visible_items {
                                    let item_opt = page
                                        .items
                                        .binary_search_by_key(
                                            &(spatial_item.z_index, spatial_item.item_id),
                                            |it| (it.z_index, it.id),
                                        )
                                        .ok()
                                        .map(|idx| &page.items[idx])
                                        .or_else(|| {
                                            page.items
                                                .iter()
                                                .find(|it| it.id == spatial_item.item_id)
                                        });

                                    if let Some(item) = item_opt {
                                        Self::draw_item_with_selection(
                                            cr,
                                            item,
                                            &st.selection,
                                            &st,
                                            current_mode,
                                            is_dark,
                                        );
                                    }
                                }

                                if st.selection.is_dragging
                                    && st.selection.page_index == Some(page.index)
                                {
                                    for &sel_id in &st.selection.selected_ids {
                                        if !visible_items.iter().any(|si| si.item_id == sel_id) {
                                            if let Some(item) =
                                                page.items.iter().find(|it| it.id == sel_id)
                                            {
                                                visible_strokes += 1;
                                                Self::draw_item_with_selection(
                                                    cr,
                                                    item,
                                                    &st.selection,
                                                    &st,
                                                    current_mode,
                                                    is_dark,
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            // Fallback sicuro su scansione lineare se l'indice non è ancora popolato
                            visible_strokes += page.items.len();
                            for item in &page.items {
                                Self::draw_item_with_selection(
                                    cr,
                                    item,
                                    &st.selection,
                                    &st,
                                    current_mode,
                                    is_dark,
                                );
                            }
                        }
                        let _ = cr.restore();

                        let _ = cr.restore(); // Fine clip pagina

                        if page.index + 1 < st.document.pages.len() {
                            cr.set_source_rgba(0.50, 0.54, 0.62, 0.70);
                            cr.set_font_size(11.0);
                            let label = format!("— Inizio Pagina {} —", page.index + 2);
                            let text_w = cr.text_extents(&label).map(|e| e.width()).unwrap_or(90.0);
                            cr.move_to(sx + (sw - text_w) / 2.0, sy + sh + 24.0);
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

                    // Rendering vettoriale dei tratti con Culling Spaziale R-Tree
                    let _ = cr.save();
                    cr.translate(vp.offset_x, vp.offset_y);
                    cr.scale(vp.zoom, vp.zoom);

                    let doc_left = -vp.offset_x / vp.zoom;
                    let doc_top = -vp.offset_y / vp.zoom;
                    let doc_right = (width as f64 - vp.offset_x) / vp.zoom;
                    let doc_bottom = (height as f64 - vp.offset_y) / vp.zoom;

                    for page in &st.document.pages {
                        if !page.spatial_index.is_empty() {
                            let visible_items = page
                                .spatial_index
                                .locate_in_envelope(doc_left, doc_top, doc_right, doc_bottom);
                            visible_strokes += visible_items.len();
                            for spatial_item in &visible_items {
                                let item_opt = page
                                    .items
                                    .binary_search_by_key(
                                        &(spatial_item.z_index, spatial_item.item_id),
                                        |it| (it.z_index, it.id),
                                    )
                                    .ok()
                                    .map(|idx| &page.items[idx])
                                    .or_else(|| {
                                        page.items.iter().find(|it| it.id == spatial_item.item_id)
                                    });

                                if let Some(item) = item_opt {
                                    Self::draw_item_with_selection(
                                        cr,
                                        item,
                                        &st.selection,
                                        &st,
                                        current_mode,
                                        is_dark,
                                    );
                                }
                            }

                            if st.selection.is_dragging {
                                for &sel_id in &st.selection.selected_ids {
                                    if !visible_items.iter().any(|si| si.item_id == sel_id) {
                                        if let Some(item) =
                                            page.items.iter().find(|it| it.id == sel_id)
                                        {
                                            visible_strokes += 1;
                                            Self::draw_item_with_selection(
                                                cr,
                                                item,
                                                &st.selection,
                                                &st,
                                                current_mode,
                                                is_dark,
                                            );
                                        }
                                    }
                                }
                            }
                        } else {
                            // Fallback sicuro su scansione lineare se l'indice è vuoto
                            for item in &page.items {
                                if let Some(bbox) = item.bounding_box() {
                                    let item_rect = uninote_core::coordinates::Rect2D::new(
                                        bbox.min_x,
                                        bbox.min_y,
                                        bbox.max_x - bbox.min_x,
                                        bbox.max_y - bbox.min_y,
                                    );
                                    let vis_rect = uninote_core::coordinates::Rect2D::new(
                                        doc_left,
                                        doc_top,
                                        doc_right - doc_left,
                                        doc_bottom - doc_top,
                                    );
                                    if !vis_rect.intersects(&item_rect) {
                                        continue;
                                    }
                                }
                                visible_strokes += 1;
                                Self::draw_item_with_selection(
                                    cr,
                                    item,
                                    &st.selection,
                                    &st,
                                    current_mode,
                                    is_dark,
                                );
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

            // 5. Disegno indicatore visivo live del raggio della gomma (overlay a schermo)
            if st.active_tool == ActiveTool::Eraser || st.eraser_pointer.is_some() {
                if let Some((px, py)) = st.eraser_pointer {
                    Self::draw_eraser_indicator(cr, px, py, st.eraser_radius, is_dark);
                }
            }

            // 6. Disegno scia effimera del puntatore laser (overlay con coordinate in spazio documento)
            if !st.laser_trail.points.is_empty() {
                let now_ns = app_start.elapsed().as_nanos() as u64;
                let _ = cr.save();
                cr.translate(vp.offset_x, vp.offset_y);
                cr.scale(vp.zoom, vp.zoom);
                Self::draw_laser_trail(cr, &st.laser_trail, now_ns, vp.zoom, st.laser_width);
                let _ = cr.restore();
            }

            // 4. Aggiornamento metriche di rendering HUD (zero-overhead quando nascosto)
            if hud_clone.is_visible() {
                let render_duration = start_render.elapsed().as_micros() as u64;
                let total_strokes: usize = st.document.pages.iter().map(|p| p.items.len()).sum();
                let culled_pct = if total_strokes > 0 {
                    let culled_count = total_strokes.saturating_sub(visible_strokes);
                    (culled_count as f64 / total_strokes as f64) * 100.0
                } else {
                    0.0
                };

                hud_clone.record_frame_tick();
                hud_clone.update_metrics(|m| {
                    m.visible_strokes = visible_strokes;
                    m.total_strokes = total_strokes;
                    m.culled_pct = culled_pct;
                    m.render_time_us = render_duration;
                });
                hud_clone.update_view();
            }
        });
    }

    #[inline]
    pub fn draw_item_with_selection(
        cr: &gtk4::cairo::Context,
        item: &uninote_core::Item,
        selection: &SelectionState,
        st: &CanvasState,
        mode: PipelineMode,
        is_dark: bool,
    ) {
        match &item.kind {
            ItemKind::Stroke(stroke) => {
                Self::draw_stroke_with_selection(cr, stroke, item.id, selection, mode, is_dark);
            }
            ItemKind::Image(image) => {
                Self::draw_image_with_selection(cr, image, item.id, selection, st);
            }
        }
    }

    #[inline]
    pub fn draw_image_with_selection(
        cr: &gtk4::cairo::Context,
        image: &uninote_core::ImageItem,
        item_id: u64,
        selection: &SelectionState,
        st: &CanvasState,
    ) {
        let is_selected_drag = selection.is_dragging && selection.selected_ids.contains(&item_id);
        if is_selected_drag {
            let _ = cr.save();
            match selection.drag_kind {
                SelectionDragKind::Translate => {
                    cr.translate(selection.drag_offset.0, selection.drag_offset.1);
                }
                SelectionDragKind::ScaleHandle(_) => {
                    let (ox, oy) = selection.scale_anchor;
                    let (sx, sy) = selection.scale_factors;
                    cr.translate(ox, oy);
                    cr.scale(sx, sy);
                    cr.translate(-ox, -oy);
                }
                SelectionDragKind::None => {}
            }
            Self::draw_image(cr, image, st);
            let _ = cr.restore();
        } else {
            Self::draw_image(cr, image, st);
        }
    }

    pub fn draw_image(
        cr: &gtk4::cairo::Context,
        image: &uninote_core::ImageItem,
        st: &CanvasState,
    ) {
        let cache_rc = crate::image_cache::get_global_image_cache();
        let mut cache = cache_rc.borrow_mut();
        let surface_opt = if let Some(surf) = cache.get(&image.asset_hash) {
            Some(surf)
        } else if let Some(ref path) = st.file_path {
            if let Ok(store) = uninote_storage::DocumentStore::open_or_create(path) {
                cache.get_or_load_from_store(&image.asset_hash, &store)
            } else {
                None
            }
        } else {
            None
        };

        let Some(surface) = surface_opt else {
            return;
        };

        let surf_w = surface.width() as f64;
        let surf_h = surface.height() as f64;
        if surf_w <= 0.0 || surf_h <= 0.0 {
            return;
        }

        let dest_x = image.bbox.min_x;
        let dest_y = image.bbox.min_y;
        let dest_w = image.bbox.max_x - image.bbox.min_x;
        let dest_h = image.bbox.max_y - image.bbox.min_y;

        if dest_w <= 0.0 || dest_h <= 0.0 {
            return;
        }

        let _ = cr.save();
        cr.translate(dest_x, dest_y);
        cr.scale(dest_w / surf_w, dest_h / surf_h);

        let _ = cr.set_source_surface(&surface, 0.0, 0.0);
        let pattern = cr.source();
        pattern.set_filter(gtk4::cairo::Filter::Bilinear);

        let opacity = image.opacity.clamp(0.0, 1.0);
        if opacity < 0.999 {
            let _ = cr.paint_with_alpha(opacity);
        } else {
            let _ = cr.paint();
        }

        let _ = cr.restore();
    }

    #[inline]
    pub fn draw_stroke_with_selection(
        cr: &gtk4::cairo::Context,
        stroke: &Stroke,
        item_id: u64,
        selection: &SelectionState,
        mode: PipelineMode,
        is_dark: bool,
    ) {
        let is_selected_drag = selection.is_dragging && selection.selected_ids.contains(&item_id);
        if is_selected_drag {
            let _ = cr.save();
            match selection.drag_kind {
                SelectionDragKind::Translate => {
                    cr.translate(selection.drag_offset.0, selection.drag_offset.1);
                }
                SelectionDragKind::ScaleHandle(_) => {
                    let (ox, oy) = selection.scale_anchor;
                    let (sx, sy) = selection.scale_factors;
                    cr.translate(ox, oy);
                    cr.scale(sx, sy);
                    cr.translate(-ox, -oy);
                }
                SelectionDragKind::None => {}
            }
            Self::draw_stroke(cr, stroke, mode, is_dark);
            let _ = cr.restore();
        } else {
            Self::draw_stroke(cr, stroke, mode, is_dark);
        }
    }

    pub fn draw_stroke(
        cr: &gtk4::cairo::Context,
        stroke: &Stroke,
        mode: PipelineMode,
        is_dark: bool,
    ) {
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
            let radius = (p.width / 2.0).max(0.05);
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
                    cr.set_line_width(avg_width.max(0.05));
                    cr.move_to(p0.x, p0.y);
                    cr.line_to(p1.x, p1.y);
                    let _ = cr.stroke();
                }
            }
            PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered => {
                let cached = stroke.bezier_segments();
                let fallback: Vec<uninote_stroke::geometry::CubicBezierSegment>;
                let segments: &[uninote_stroke::geometry::CubicBezierSegment] =
                    if !cached.is_empty() {
                        cached
                    } else {
                        fallback = uninote_stroke::geometry::catmull_rom_to_bezier(&stroke.points);
                        &fallback
                    };

                for seg in segments {
                    let len = seg.approximate_length();
                    let width_diff = (seg.width1 - seg.width0).abs();

                    if width_diff < 0.25 || len <= 3.0 {
                        let avg_w = (seg.width0 + seg.width1) / 2.0;
                        cr.set_line_width(avg_w.max(0.05));
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
                            cr.set_line_width(((prev_w + w) / 2.0).max(0.05));
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
        let canvas_clone = self.clone();

        // Down
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        let canvas_down = canvas_clone.clone();
        stylus.connect_down(move |gesture, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let p_val = pressure.unwrap_or(0.0);
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st = state.borrow_mut();
            st.last_stylus_time = Some(Instant::now());
            st.active_device = Some(DeviceKind::Stylus);

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Puntatore Laser (Stylus Down)
            if st.active_tool == ActiveTool::LaserPointer {
                st.laser_trail.points.clear();
                st.laser_trail.points.push(LaserPoint {
                    x: doc_x,
                    y: doc_y,
                    timestamp_ns: now_ns,
                });
                st.laser_trail.is_active = true;
                drop(st);
                canvas_down.ensure_laser_tick_active();
                area.queue_draw();
                return;
            }

            // Gestione Strumento Selezione (Stylus Down)
            if st.active_tool == ActiveTool::Select {
                let is_shift = gesture
                    .current_event()
                    .is_some_and(|ev| ev.modifier_state().contains(gdk::ModifierType::SHIFT_MASK));
                Self::handle_selection_down(&mut st, (doc_x, doc_y), is_shift);
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
                st.history.begin_batch();
                st.eraser_pointer = Some((x, y));
                st.last_eraser_doc_pos = Some((doc_x, doc_y));
                let radius = st.eraser_radius / st.viewport.zoom;
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
                builder.process_sample(sample);
                st.active_builder = Some(builder);

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
        let canvas_motion = canvas_clone.clone();
        stylus.connect_motion(move |gesture, x, y| {
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let now = Instant::now();
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st_guard = state.borrow_mut();
            let st = &mut *st_guard;
            st.last_stylus_time = Some(now);

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Puntatore Laser (Stylus Motion)
            if st.active_tool == ActiveTool::LaserPointer {
                if st.laser_trail.is_active {
                    let should_add = match st.laser_trail.points.last() {
                        Some(last) => {
                            let dx = doc_x - last.x;
                            let dy = doc_y - last.y;
                            dx * dx + dy * dy >= 1.44 // dist >= 1.2 pt
                        }
                        None => true,
                    };
                    if should_add {
                        st.laser_trail.points.push(LaserPoint {
                            x: doc_x,
                            y: doc_y,
                            timestamp_ns: now_ns,
                        });
                        drop(st_guard);
                        canvas_motion.ensure_laser_tick_active();
                        area.queue_draw();
                    } else {
                        drop(st_guard);
                    }
                } else {
                    drop(st_guard);
                }
                return;
            }

            // Gestione Strumento Selezione (Stylus Motion)
            if st.active_tool == ActiveTool::Select {
                let is_shift = gesture
                    .current_event()
                    .is_some_and(|ev| ev.modifier_state().contains(gdk::ModifierType::SHIFT_MASK));
                let redraw = Self::handle_selection_motion(st, (doc_x, doc_y), is_shift);
                drop(st_guard);
                if redraw {
                    area.queue_draw();
                }
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
                if !st.history.is_batch_active() {
                    st.history.begin_batch();
                }
                st.eraser_pointer = Some((x, y));
                let radius = st.eraser_radius / st.viewport.zoom;
                if let Some((prev_x, prev_y)) = st.last_eraser_doc_pos {
                    let dx = doc_x - prev_x;
                    let dy = doc_y - prev_y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    let step = (radius * 0.4).max(3.0);
                    if dist > step {
                        let steps = ((dist / step).ceil() as usize).min(32);
                        for i in 1..steps {
                            let t = (i as f64) / (steps as f64);
                            let inter_x = prev_x + t * dx;
                            let inter_y = prev_y + t * dy;
                            Self::erase_strokes_at(st, (inter_x, inter_y), radius);
                        }
                    }
                }
                st.last_eraser_doc_pos = Some((doc_x, doc_y));
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
                builder.process_sample(sample);

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
            st.last_eraser_doc_pos = None;
            if st.history.is_batch_active() {
                st.history.end_batch();
                st.persist_changes();
            }
            if st.active_tool != ActiveTool::Eraser && st.eraser_pointer.is_some() {
                st.eraser_pointer = None;
            }

            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            // Gestione Strumento Puntatore Laser (Stylus Up)
            if st.active_tool == ActiveTool::LaserPointer {
                st.laser_trail.is_active = false;
                drop(st);
                area.queue_draw();
                return;
            }

            // Gestione Strumento Selezione (Stylus Up)
            if st.active_tool == ActiveTool::Select {
                Self::handle_selection_up(&mut st);
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
                builder.process_sample(sample);
                let stroke = builder.finish();
                Self::commit_finished_stroke(&mut st, stroke);
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
            if st.history.is_batch_active() {
                st.history.end_batch();
                st.persist_changes();
            }
            if st.active_tool == ActiveTool::LaserPointer {
                st.laser_trail.is_active = false;
            }
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
        let canvas_clone = self.clone();

        // Drag Begin
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        let canvas_begin = canvas_clone.clone();
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

            if current_btn == gdk::BUTTON_SECONDARY {
                let (doc_x, doc_y) = st.viewport.screen_to_document((start_x, start_y));

                // 1. Clic destro all'interno della selezione corrente
                let hit_selection = if st.selection.has_selection() {
                    if let Some(bbox) = st.selection.bbox {
                        doc_x >= bbox.min_x - 8.0
                            && doc_x <= bbox.max_x + 8.0
                            && doc_y >= bbox.min_y - 8.0
                            && doc_y <= bbox.max_y + 8.0
                    } else {
                        false
                    }
                } else {
                    false
                };

                // 2. Clic destro sopra un'immagine (se non era già selezionata)
                let mut hit_image = false;
                if !hit_selection {
                    let page_idx = match st.document.format {
                        DocumentFormat::Whiteboard => 0,
                        DocumentFormat::Notebook => {
                            Viewport::find_page_index_at_y(doc_y, st.document.pages.len())
                        }
                    };
                    let mut found_image = None;
                    if let Some(page) = st.document.pages.get(page_idx) {
                        for it in page.items.iter().rev() {
                            if let ItemKind::Image(ref img) = it.kind {
                                if img.bbox.contains_point(doc_x, doc_y) {
                                    found_image = Some((it.id, it.bounding_box()));
                                    break;
                                }
                            }
                        }
                    }
                    if let Some((it_id, it_bbox)) = found_image {
                        st.selection.page_index = Some(page_idx);
                        st.selection.selected_ids = vec![it_id];
                        st.selection.bbox = it_bbox;
                        st.active_tool = ActiveTool::Select;
                        hit_image = true;
                    }
                }

                if hit_selection || hit_image {
                    st.mouse_pan_start = None;
                    drop(st);
                    area.queue_draw();
                    if hit_image {
                        canvas_begin.notify_tool_changed(ActiveTool::Select);
                    }
                    canvas_begin.show_selection_context_menu(start_x, start_y);
                    return;
                }

                st.mouse_pan_start = Some((start_x, start_y));
                return;
            }

            if current_btn == gdk::BUTTON_MIDDLE {
                st.mouse_pan_start = Some((start_x, start_y));
                return;
            }

            if current_btn == gdk::BUTTON_PRIMARY {
                st.active_device = Some(DeviceKind::Mouse);
                st.mouse_drag_start = Some((start_x, start_y));
                let (doc_x, doc_y) = st.viewport.screen_to_document((start_x, start_y));
                let now_ns = app_start.elapsed().as_nanos() as u64;

                // Gestione Strumento Puntatore Laser (Mouse Drag Begin)
                if st.active_tool == ActiveTool::LaserPointer {
                    st.laser_trail.points.clear();
                    st.laser_trail.points.push(LaserPoint {
                        x: doc_x,
                        y: doc_y,
                        timestamp_ns: now_ns,
                    });
                    st.laser_trail.is_active = true;
                    drop(st);
                    canvas_begin.ensure_laser_tick_active();
                    area.queue_draw();
                    return;
                }

                // Gestione Strumento Selezione (Mouse Drag Begin)
                if st.active_tool == ActiveTool::Select {
                    let is_shift = gesture.current_event().is_some_and(|ev| {
                        ev.modifier_state().contains(gdk::ModifierType::SHIFT_MASK)
                    });
                    Self::handle_selection_down(&mut st, (doc_x, doc_y), is_shift);
                    drop(st);
                    area.queue_draw();
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    st.history.begin_batch();
                    st.eraser_pointer = Some((start_x, start_y));
                    st.last_eraser_doc_pos = Some((doc_x, doc_y));
                    let radius = st.eraser_radius / st.viewport.zoom;
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
                    builder.process_sample(sample);
                    st.active_builder = Some(builder);

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
        let canvas_update = canvas_clone.clone();
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

                // Gestione Strumento Puntatore Laser (Mouse Drag Update)
                if st.active_tool == ActiveTool::LaserPointer {
                    if st.laser_trail.is_active {
                        let should_add = match st.laser_trail.points.last() {
                            Some(last) => {
                                let dx = doc_x - last.x;
                                let dy = doc_y - last.y;
                                dx * dx + dy * dy >= 1.44 // dist >= 1.2 pt
                            }
                            None => true,
                        };
                        if should_add {
                            st.laser_trail.points.push(LaserPoint {
                                x: doc_x,
                                y: doc_y,
                                timestamp_ns: now_ns,
                            });
                            drop(st_guard);
                            canvas_update.ensure_laser_tick_active();
                            area.queue_draw();
                        } else {
                            drop(st_guard);
                        }
                    } else {
                        drop(st_guard);
                    }
                    return;
                }

                // Gestione Strumento Selezione (Mouse Drag Update)
                if st.active_tool == ActiveTool::Select {
                    let is_shift = gesture.current_event().is_some_and(|ev| {
                        ev.modifier_state().contains(gdk::ModifierType::SHIFT_MASK)
                    });
                    let redraw = Self::handle_selection_motion(st, (doc_x, doc_y), is_shift);
                    drop(st_guard);
                    if redraw {
                        area.queue_draw();
                    }
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    if !st.history.is_batch_active() {
                        st.history.begin_batch();
                    }
                    st.eraser_pointer = Some((cur_screen_x, cur_screen_y));
                    let radius = st.eraser_radius / st.viewport.zoom;
                    if let Some((prev_x, prev_y)) = st.last_eraser_doc_pos {
                        let dx = doc_x - prev_x;
                        let dy = doc_y - prev_y;
                        let dist = (dx * dx + dy * dy).sqrt();
                        let step = (radius * 0.4).max(3.0);
                        if dist > step {
                            let steps = ((dist / step).ceil() as usize).min(32);
                            for i in 1..steps {
                                let t = (i as f64) / (steps as f64);
                                let inter_x = prev_x + t * dx;
                                let inter_y = prev_y + t * dy;
                                Self::erase_strokes_at(st, (inter_x, inter_y), radius);
                            }
                        }
                    }
                    st.last_eraser_doc_pos = Some((doc_x, doc_y));
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
                    builder.process_sample(sample);
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

                // Gestione Strumento Puntatore Laser (Mouse Drag End)
                if st.active_tool == ActiveTool::LaserPointer {
                    st.laser_trail.is_active = false;
                    drop(st);
                    area.queue_draw();
                    return;
                }

                // Gestione Strumento Selezione (Mouse Drag End)
                if st.active_tool == ActiveTool::Select {
                    Self::handle_selection_up(&mut st);
                    drop(st);
                    area.queue_draw();
                    return;
                }

                if st.active_tool == ActiveTool::Eraser {
                    st.eraser_pointer = Some((cur_x, cur_y));
                    st.last_eraser_doc_pos = None;
                    let radius = st.eraser_radius / st.viewport.zoom;
                    Self::erase_strokes_at(&mut st, (doc_x, doc_y), radius);
                    if st.history.is_batch_active() {
                        st.history.end_batch();
                        st.persist_changes();
                    }
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
                    builder.process_sample(sample);
                    let stroke = builder.finish();
                    Self::commit_finished_stroke(&mut st, stroke);
                }
            }

            area.queue_draw();
        });

        self.area.add_controller(drag);
    }

    fn setup_touch_long_press_controller(&self) {
        let long_press = gtk4::GestureLongPress::new();
        long_press.set_touch_only(false);
        let state = self.state.clone();
        let area = self.area.clone();
        let canvas_clone = self.clone();

        long_press.connect_pressed(move |_gesture, x, y| {
            let mut st = state.borrow_mut();
            let (doc_x, doc_y) = st.viewport.screen_to_document((x, y));

            let hit_selection = if st.selection.has_selection() {
                if let Some(bbox) = st.selection.bbox {
                    doc_x >= bbox.min_x - 8.0
                        && doc_x <= bbox.max_x + 8.0
                        && doc_y >= bbox.min_y - 8.0
                        && doc_y <= bbox.max_y + 8.0
                } else {
                    false
                }
            } else {
                false
            };

            let mut hit_image = false;
            if !hit_selection {
                let page_idx = match st.document.format {
                    DocumentFormat::Whiteboard => 0,
                    DocumentFormat::Notebook => {
                        Viewport::find_page_index_at_y(doc_y, st.document.pages.len())
                    }
                };
                let mut found_image = None;
                if let Some(page) = st.document.pages.get(page_idx) {
                    for it in page.items.iter().rev() {
                        if let ItemKind::Image(ref img) = it.kind {
                            if img.bbox.contains_point(doc_x, doc_y) {
                                found_image = Some((it.id, it.bounding_box()));
                                break;
                            }
                        }
                    }
                }
                if let Some((it_id, it_bbox)) = found_image {
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = vec![it_id];
                    st.selection.bbox = it_bbox;
                    st.active_tool = ActiveTool::Select;
                    hit_image = true;
                }
            }

            if hit_selection || hit_image {
                st.mouse_pan_start = None;
                drop(st);
                area.queue_draw();
                if hit_image {
                    canvas_clone.notify_tool_changed(ActiveTool::Select);
                }
                canvas_clone.show_selection_context_menu(x, y);
            }
        });

        self.area.add_controller(long_press);
    }

    fn setup_motion_controller(&self) {
        let motion = gtk4::EventControllerMotion::new();
        let state_clone = self.state.clone();
        let area_clone = self.area.clone();

        // 1. Motion: tracciamento continuo a schermo
        motion.connect_motion(move |_ctrl, x, y| {
            let mut st = state_clone.borrow_mut();

            // Aggiorna il cursore del mouse su maniglie/corpo della selezione
            if st.active_tool == ActiveTool::Select {
                let doc_pt = st.viewport.screen_to_document((x, y));
                let hit_res = st.selection.hit_test(doc_pt, &st.viewport);
                let cursor_name = match hit_res {
                    SelectionHitTestResult::Handle(h) => match h {
                        crate::selection::HandleId::TopLeft
                        | crate::selection::HandleId::BottomRight => Some("nwse-resize"),
                        crate::selection::HandleId::TopRight
                        | crate::selection::HandleId::BottomLeft => Some("nesw-resize"),
                        crate::selection::HandleId::TopMiddle
                        | crate::selection::HandleId::BottomMiddle => Some("ns-resize"),
                        crate::selection::HandleId::RightMiddle
                        | crate::selection::HandleId::LeftMiddle => Some("ew-resize"),
                    },
                    SelectionHitTestResult::Body => Some("grab"),
                    SelectionHitTestResult::None => None,
                };
                area_clone.set_cursor_from_name(cursor_name);
            } else {
                area_clone.set_cursor_from_name(None);
            }

            // ZERO-OVERHEAD FAST PATH:
            // Se lo strumento attivo non è la gomma, usciamo immediatamente.
            if st.active_tool != ActiveTool::Eraser {
                if st.eraser_pointer.is_some() {
                    st.eraser_pointer = None;
                    drop(st);
                    area_clone.queue_draw();
                }
                return;
            }

            // Aggiorna le coordinate a schermo del cerchio della gomma
            st.eraser_pointer = Some((x, y));
            drop(st);
            area_clone.queue_draw();
        });

        // 2. Leave: quando il puntatore esce dal DrawingArea, rimuovi il cerchio
        let state_clone2 = self.state.clone();
        let area_clone2 = self.area.clone();
        motion.connect_leave(move |_ctrl| {
            area_clone2.set_cursor_from_name(None);
            let mut st = state_clone2.borrow_mut();
            if st.eraser_pointer.is_some() {
                st.eraser_pointer = None;
                drop(st);
                area_clone2.queue_draw();
            }
        });

        // 3. Enter: quando il puntatore entra, se la gomma è attiva, mostra il cerchio
        let state_clone3 = self.state.clone();
        let area_clone3 = self.area.clone();
        motion.connect_enter(move |_ctrl, x, y| {
            let mut st = state_clone3.borrow_mut();
            if st.active_tool == ActiveTool::Eraser {
                st.eraser_pointer = Some((x, y));
                drop(st);
                area_clone3.queue_draw();
            }
        });

        self.area.add_controller(motion);
    }

    fn setup_scroll_and_zoom_controllers(&self) {
        let scroll = gtk4::EventControllerScroll::new(
            gtk4::EventControllerScrollFlags::BOTH_AXES | gtk4::EventControllerScrollFlags::KINETIC,
        );
        let state = self.state.clone();
        let area = self.area.clone();
        let hud = self.hud.clone();
        let zoom_callbacks_scroll = self.zoom_callbacks.clone();

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
                let zoom = st.viewport.zoom;
                let pct = (zoom * 100.0).round() as usize;
                drop(st);
                hud.update_metrics(|m| m.zoom_pct = pct);
                hud.update_view();
                for cb in zoom_callbacks_scroll.borrow().iter() {
                    cb(zoom);
                }
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
        let zoom_callbacks_pinch = self.zoom_callbacks.clone();

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

        // Zoom scalare (pinch): scala il canvas centrato sul bounding_box_center in tempo reale
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

            if (factor - 1.0).abs() > 0.0001 && factor > 0.5 && factor < 2.0 {
                let mut st = state_scale.borrow_mut();
                st.viewport.zoom_at(current_center, factor);
                let zoom = st.viewport.zoom;
                let pct = (zoom * 100.0).round() as usize;
                drop(st);

                hud_scale.update_metrics(|m| m.zoom_pct = pct);
                hud_scale.update_view();
                for cb in zoom_callbacks_pinch.borrow().iter() {
                    cb(zoom);
                }
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

    /// Cancella i tratti che intersecano la coordinata del documento indicata,
    /// accelerato tramite R*-Tree PageSpatialIndex per latenza < 0.5 ms su 10.000 tratti.
    pub fn erase_strokes_at(st: &mut CanvasState, doc_pt: (f64, f64), radius: f64) {
        if !radius.is_finite() || radius <= 0.0 || !doc_pt.0.is_finite() || !doc_pt.1.is_finite() {
            return;
        }
        if st.document.pages.is_empty() {
            return;
        }

        let page_idx = if st.document.format == DocumentFormat::Whiteboard {
            0
        } else {
            Viewport::find_page_index_at_y(doc_pt.1, st.document.pages.len())
        };
        if let Some(page) = st.document.pages.get_mut(page_idx) {
            // Auto-reidratazione difensiva se l'indice spaziale è vuoto ma ci sono elementi
            if page.spatial_index.is_empty() && !page.items.is_empty() {
                page.rebuild_spatial_index();
            }

            // 1. Pre-filtraggio R*-Tree dell'inviluppo della gomma in O(log N + K) con padding per spessore tratto
            let query_pad = 8.0;
            let min_x = doc_pt.0 - radius - query_pad;
            let min_y = doc_pt.1 - radius - query_pad;
            let max_x = doc_pt.0 + radius + query_pad;
            let max_y = doc_pt.1 + radius + query_pad;

            let candidates = page
                .spatial_index
                .locate_in_envelope(min_x, min_y, max_x, max_y);
            if candidates.is_empty() {
                return;
            }

            // 2. Controllo geometrico puntuale dettagliato ESCLUSIVAMENTE sui candidati (K << N)
            let page_id = page.id;

            match st.eraser_mode {
                EraserMode::WholeStroke => {
                    let mut erased = Vec::new();

                    if candidates.len() == 1 {
                        let target_id = candidates[0].item_id;
                        if let Some(item) = page.items.iter().find(|it| it.id == target_id) {
                            if let ItemKind::Stroke(ref s) = item.kind {
                                if s.intersects_point(doc_pt, radius) {
                                    erased.push(item.clone());
                                }
                            }
                        }
                    } else {
                        let candidate_ids: std::collections::HashSet<u64> =
                            candidates.iter().map(|c| c.item_id).collect();
                        for item in &page.items {
                            if candidate_ids.contains(&item.id) {
                                if let ItemKind::Stroke(ref s) = item.kind {
                                    if s.intersects_point(doc_pt, radius) {
                                        erased.push(item.clone());
                                    }
                                }
                            }
                        }
                    }

                    // 3. Esecuzione comando atomico EraseItems (aggiorna history, document e spatial_index)
                    if !erased.is_empty() {
                        st.history.execute(
                            DocCommand::EraseItems {
                                page_id,
                                erased_items: erased,
                            },
                            &mut st.document,
                        );
                        if !st.history.is_batch_active() {
                            st.persist_changes();
                        }
                    }
                }
                EraserMode::Segment => {
                    let candidate_ids: std::collections::HashSet<u64> =
                        candidates.iter().map(|c| c.item_id).collect();
                    let mut to_slice = Vec::new();

                    for item in &page.items {
                        if candidate_ids.contains(&item.id) {
                            if let ItemKind::Stroke(ref s) = item.kind {
                                let stroke_half_w =
                                    (s.points.iter().map(|p| p.width).fold(0.0_f64, f64::max)
                                        / 2.0)
                                        .max(s.style.base_width / 2.0);
                                let eff_cut_radius = radius + stroke_half_w;

                                // Verifica se il cerchio della gomma interseca la linea mediana (anche curvata)
                                // o include un estremo del tratto, prevenendo tagli accidentali da halo grazing.
                                let r_sq = radius * radius;
                                let any_pt_inside = s.points.iter().any(|p| {
                                    let dx = p.x - doc_pt.0;
                                    let dy = p.y - doc_pt.1;
                                    dx * dx + dy * dy <= r_sq
                                });

                                let hits_centerline = if any_pt_inside {
                                    true
                                } else {
                                    let segs = s.bezier_segments();
                                    if !segs.is_empty() {
                                        segs.iter().any(|seg| {
                                            !uninote_stroke::slice::find_bezier_circle_intersections(
                                                seg, doc_pt, radius,
                                            )
                                            .is_empty()
                                        })
                                    } else {
                                        s.intersects_centerline(doc_pt, radius)
                                    }
                                };

                                if hits_centerline {
                                    let fragments = s.slice_with_circle(doc_pt, eff_cut_radius);
                                    let is_unchanged = fragments.len() == 1
                                        && fragments[0].id == s.id
                                        && fragments[0].points.len() == s.points.len()
                                        && fragments[0].points.iter().zip(&s.points).all(
                                            |(p1, p2)| {
                                                (p1.x - p2.x).abs() < 1e-4
                                                    && (p1.y - p2.y).abs() < 1e-4
                                            },
                                        );
                                    if !is_unchanged {
                                        to_slice.push((item.clone(), fragments));
                                    }
                                }
                            }
                        }
                    }

                    if !to_slice.is_empty() {
                        for (orig_item, fragments) in to_slice {
                            let mut new_items = Vec::with_capacity(fragments.len());
                            for mut frag in fragments {
                                let new_id = st.alloc_item_id();
                                frag.id = new_id;
                                new_items.push(Item {
                                    id: new_id,
                                    z_index: orig_item.z_index,
                                    kind: ItemKind::Stroke(frag),
                                });
                            }
                            st.history.execute(
                                DocCommand::SliceStroke {
                                    page_id,
                                    original_stroke: orig_item,
                                    new_strokes: new_items,
                                },
                                &mut st.document,
                            );
                        }
                        if !st.history.is_batch_active() {
                            st.persist_changes();
                        }
                    }
                }
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

    /// Gestisce l'inizio del contatto (Down) per lo strumento di selezione.
    pub fn handle_selection_down(st: &mut CanvasState, doc_pt: (f64, f64), is_shift: bool) {
        let hit_res = st.selection.hit_test(doc_pt, &st.viewport);
        match hit_res {
            SelectionHitTestResult::Handle(handle_id) => {
                if let Some(ref bbox) = st.selection.bbox {
                    let (ox, oy) = handle_id.anchor_point(bbox);
                    st.selection.is_dragging = true;
                    st.selection.drag_kind = SelectionDragKind::ScaleHandle(handle_id);
                    st.selection.scale_anchor = (ox, oy);
                    st.selection.scale_factors = (1.0, 1.0);
                    st.selection.lock_aspect_ratio = !is_shift;
                    st.selection.drag_start_doc = Some(doc_pt);
                }
            }
            SelectionHitTestResult::Body => {
                st.selection.is_dragging = true;
                st.selection.drag_kind = SelectionDragKind::Translate;
                st.selection.drag_start_doc = Some(doc_pt);
                st.selection.drag_offset = (0.0, 0.0);
            }
            SelectionHitTestResult::None => {
                // Se il tocco/click avviene direttamente su un'immagine, selezionala al volo e prepara il trascinamento
                let mut clicked_image: Option<(usize, ItemId, Rect)> = None;
                if !st.document.pages.is_empty() {
                    let est_idx = if st.document.format == DocumentFormat::Whiteboard {
                        0
                    } else {
                        Viewport::find_page_index_at_y(doc_pt.1, st.document.pages.len())
                            .min(st.document.pages.len().saturating_sub(1))
                    };

                    let mut candidate_pages = vec![est_idx];
                    for idx in 0..st.document.pages.len() {
                        if idx != est_idx {
                            candidate_pages.push(idx);
                        }
                    }

                    for page_idx in candidate_pages {
                        if let Some(page) = st.document.pages.get(page_idx) {
                            for item in page.items.iter().rev() {
                                if let ItemKind::Image(ref img) = item.kind {
                                    if img.bbox.normalized().contains_point(doc_pt.0, doc_pt.1) {
                                        clicked_image =
                                            Some((page_idx, item.id, img.bbox.normalized()));
                                        break;
                                    }
                                }
                            }
                        }
                        if clicked_image.is_some() {
                            break;
                        }
                    }
                }

                if let Some((page_idx, img_id, img_bbox)) = clicked_image {
                    st.selection.clear();
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = vec![img_id];
                    st.selection.bbox = Some(img_bbox);
                    st.selection.is_dragging = true;
                    st.selection.drag_kind = SelectionDragKind::Translate;
                    st.selection.drag_start_doc = Some(doc_pt);
                    st.selection.drag_offset = (0.0, 0.0);
                } else {
                    st.selection.clear();
                    match st.selection.mode {
                        SelectionMode::Lasso => {
                            st.selection.active_lasso.push(doc_pt);
                        }
                        SelectionMode::Rectangle => {
                            st.selection.active_rect_start = Some(doc_pt);
                            st.selection.active_rect_current = Some(doc_pt);
                        }
                    }
                }
            }
        }
    }

    /// Gestisce il movimento del contatto (Motion / Drag Update) per lo strumento di selezione.
    pub fn handle_selection_motion(
        st: &mut CanvasState,
        doc_pt: (f64, f64),
        is_shift: bool,
    ) -> bool {
        if st.selection.is_dragging {
            match st.selection.drag_kind {
                SelectionDragKind::Translate => {
                    if let Some(start) = st.selection.drag_start_doc {
                        st.selection.drag_offset = (doc_pt.0 - start.0, doc_pt.1 - start.1);
                    }
                }
                SelectionDragKind::ScaleHandle(handle) => {
                    if let Some(ref bbox) = st.selection.bbox {
                        st.selection.lock_aspect_ratio = !is_shift;
                        let (anchor, factors) = SelectionState::compute_scale_factors(
                            handle,
                            bbox,
                            doc_pt,
                            st.selection.lock_aspect_ratio,
                        );
                        st.selection.scale_anchor = anchor;
                        st.selection.scale_factors = factors;
                    }
                }
                SelectionDragKind::None => {}
            }
            true
        } else if st.selection.is_selecting() {
            match st.selection.mode {
                SelectionMode::Lasso => {
                    st.selection.active_lasso.push(doc_pt);
                }
                SelectionMode::Rectangle => {
                    st.selection.active_rect_current = Some(doc_pt);
                }
            }
            true
        } else {
            false
        }
    }

    /// Gestisce il rilascio del contatto (Up / Drag End) per lo strumento di selezione.
    pub fn handle_selection_up(st: &mut CanvasState) {
        if st.selection.is_dragging {
            match st.selection.drag_kind {
                SelectionDragKind::Translate => {
                    let (dx, dy) = st.selection.drag_offset;
                    st.selection.is_dragging = false;
                    st.selection.drag_kind = SelectionDragKind::None;
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
                                st.history.execute(cmd, &mut st.document);
                                st.persist_changes();
                                if let Some(bbox) = st.selection.bbox.as_mut() {
                                    *bbox = bbox.translate(dx, dy);
                                }
                            }
                        }
                    }
                }
                SelectionDragKind::ScaleHandle(_handle) => {
                    let (ox, oy) = st.selection.scale_anchor;
                    let (sx, sy) = st.selection.scale_factors;
                    st.selection.is_dragging = false;
                    st.selection.drag_kind = SelectionDragKind::None;
                    st.selection.drag_start_doc = None;
                    st.selection.scale_factors = (1.0, 1.0);

                    if ((sx - 1.0).abs() > 0.001 || (sy - 1.0).abs() > 0.001)
                        && !st.selection.selected_ids.is_empty()
                    {
                        if let Some(page_idx) = st.selection.page_index {
                            if let Some(page) = st.document.pages.get(page_idx) {
                                let page_id = page.id;
                                let item_ids = st.selection.selected_ids.clone();
                                let cmd = DocCommand::TransformItems {
                                    page_id,
                                    item_ids,
                                    origin_x: ox,
                                    origin_y: oy,
                                    scale_x: sx,
                                    scale_y: sy,
                                    rotation_rad: 0.0,
                                };
                                st.history.execute(cmd, &mut st.document);
                                st.persist_changes();
                                if let Some(bbox) = st.selection.bbox.as_mut() {
                                    *bbox = Rect::new(
                                        ox + (bbox.min_x - ox) * sx,
                                        oy + (bbox.min_y - oy) * sy,
                                        ox + (bbox.max_x - ox) * sx,
                                        oy + (bbox.max_y - oy) * sy,
                                    )
                                    .normalized();
                                }
                            }
                        }
                    }
                }
                SelectionDragKind::None => {
                    st.selection.is_dragging = false;
                }
            }
        } else if st.selection.is_selecting() {
            Self::finalize_selection_gesture(st);
        }
    }

    /// Conclude il gesto di selezione poligonale o rettangolare e rileva i tratti selezionati.
    pub fn finalize_selection_gesture(st: &mut CanvasState) {
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

                let (selected_ids, bbox) =
                    crate::selection::AppSelectionEngine::select_lasso(page, &poly);
                if !selected_ids.is_empty() {
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = selected_ids;
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

                let (selected_ids, bbox) =
                    crate::selection::AppSelectionEngine::select_rect(page, &rect);
                if !selected_ids.is_empty() {
                    st.selection.page_index = Some(page_idx);
                    st.selection.selected_ids = selected_ids;
                    st.selection.bbox = bbox;
                } else {
                    st.selection.clear();
                }
            }
        }
    }

    /// Elimina in blocco gli elementi attualmente selezionati tramite DocCommand::EraseItems.
    pub fn delete_selected_items(st: &mut CanvasState) -> bool {
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
        let cyan = if is_dark {
            (0.427, 0.824, 1.0) // #6DD2FF UniNote Electric Cyan
        } else {
            (0.0, 0.40, 0.545) // #00668B WCAG AA 6.2:1 contrast su carta chiara
        };

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

        // 2. Bounding box e 8 maniglie perimetrali DMS
        if let Some(ref base_bbox) = selection.bbox {
            // Calcola l'AABB live trasformato durante il drag
            let live_bbox = match selection.drag_kind {
                SelectionDragKind::Translate => {
                    base_bbox.translate(selection.drag_offset.0, selection.drag_offset.1)
                }
                SelectionDragKind::ScaleHandle(_) => {
                    let (ox, oy) = selection.scale_anchor;
                    let (sx, sy) = selection.scale_factors;
                    Rect::new(
                        ox + (base_bbox.min_x - ox) * sx,
                        oy + (base_bbox.min_y - oy) * sy,
                        ox + (base_bbox.max_x - ox) * sx,
                        oy + (base_bbox.max_y - oy) * sy,
                    )
                    .normalized()
                }
                SelectionDragKind::None => *base_bbox,
            };

            let (s_min_x, s_min_y) = vp.document_to_screen((live_bbox.min_x, live_bbox.min_y));
            let (s_max_x, s_max_y) = vp.document_to_screen((live_bbox.max_x, live_bbox.max_y));
            let sw = (s_max_x - s_min_x).max(1.0);
            let sh = (s_max_y - s_min_y).max(1.0);

            let _ = cr.save();

            // 2.1 Riempimento ciano trasparente
            cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.05);
            cr.rectangle(s_min_x, s_min_y, sw, sh);
            let _ = cr.fill();

            // 2.2 Bordo tratteggiato Electric Cyan
            cr.set_source_rgba(cyan.0, cyan.1, cyan.2, 0.95);
            cr.set_line_width(1.8);
            cr.set_dash(&[6.0, 4.0], 0.0);
            cr.rectangle(s_min_x, s_min_y, sw, sh);
            let _ = cr.stroke();
            cr.set_dash(&[], 0.0);

            // 2.3 Grip centrale per la presa di spostamento
            let center_x = s_min_x + sw / 2.0;
            let center_y = s_min_y + sh / 2.0;
            let grip_r = 7.5;

            cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
            cr.arc(
                center_x,
                center_y + 1.0,
                grip_r + 1.0,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();

            cr.set_source_rgb(cyan.0, cyan.1, cyan.2);
            cr.arc(center_x, center_y, grip_r, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();

            if is_dark {
                cr.set_source_rgb(0.039, 0.082, 0.102);
            } else {
                cr.set_source_rgb(1.0, 1.0, 1.0);
            }
            cr.set_line_width(2.0);
            cr.arc(center_x, center_y, grip_r, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.stroke();

            cr.arc(center_x, center_y, 2.2, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();

            // 2.4 Le 4 Maniglie d'Angolo (Nodi Circolari DMS con Pinpoint)
            let corner_radius = 5.5;
            let corners = [
                (s_min_x, s_min_y),
                (s_max_x, s_min_y),
                (s_max_x, s_max_y),
                (s_min_x, s_max_y),
            ];

            for (cx, cy) in corners {
                // Ombra sottile
                cr.set_source_rgba(0.0, 0.0, 0.0, 0.30);
                cr.arc(
                    cx,
                    cy + 1.0,
                    corner_radius + 0.8,
                    0.0,
                    2.0 * std::f64::consts::PI,
                );
                let _ = cr.fill();

                // Riempimento a contrasto
                if is_dark {
                    cr.set_source_rgb(0.92, 0.95, 0.98);
                } else {
                    cr.set_source_rgb(1.0, 1.0, 1.0);
                }
                cr.arc(cx, cy, corner_radius, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();

                // Anello Electric Cyan
                cr.set_source_rgb(cyan.0, cyan.1, cyan.2);
                cr.set_line_width(1.8);
                cr.arc(cx, cy, corner_radius, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.stroke();

                // Punto centrale
                cr.arc(cx, cy, 1.5, 0.0, 2.0 * std::f64::consts::PI);
                let _ = cr.fill();
            }

            // 2.5 I 4 Punti Mediani (Pillole Arrotondate DMS) se lo spazio è >= 32 px
            if sw >= 32.0 && sh >= 32.0 {
                let mid_x = s_min_x + sw / 2.0;
                let mid_y = s_min_y + sh / 2.0;

                // Pillola orizzontale (Top e Bottom): 14x6 px
                draw_dms_pill(cr, mid_x, s_min_y, 14.0, 6.0, cyan, is_dark);
                draw_dms_pill(cr, mid_x, s_max_y, 14.0, 6.0, cyan, is_dark);

                // Pillola verticale (Left e Right): 6x14 px
                draw_dms_pill(cr, s_min_x, mid_y, 6.0, 14.0, cyan, is_dark);
                draw_dms_pill(cr, s_max_x, mid_y, 6.0, 14.0, cyan, is_dark);
            }

            let _ = cr.restore();
        }
    }

    /// Disegna l'indicatore circolare semitrasparente che segue il puntatore della gomma.
    pub fn draw_eraser_indicator(
        cr: &gtk4::cairo::Context,
        screen_x: f64,
        screen_y: f64,
        radius: f64,
        is_dark: bool,
    ) {
        if !radius.is_finite() || radius <= 0.0 || !screen_x.is_finite() || !screen_y.is_finite() {
            return;
        }

        let _ = cr.save();
        cr.new_path();
        cr.set_dash(&[], 0.0);
        cr.arc(screen_x, screen_y, radius, 0.0, 2.0 * std::f64::consts::PI);

        if is_dark {
            // Tema scuro: campitura ciano DMS delicata con perimetro netto brillante
            cr.set_source_rgba(0.427, 0.824, 1.0, 0.12);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.427, 0.824, 1.0, 0.85);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        } else {
            // Tema chiaro: campitura neutra ombreggiata con perimetro scuro ad alto contrasto
            cr.set_source_rgba(0.15, 0.18, 0.22, 0.08);
            let _ = cr.fill_preserve();
            cr.set_source_rgba(0.15, 0.18, 0.22, 0.70);
            cr.set_line_width(1.5);
            let _ = cr.stroke();
        }

        let _ = cr.restore();
    }

    /// Attiva il tick callback a 60/120 Hz solo fintanto che ci sono punti laser da dissolvere o lo strumento è attivo.
    pub fn ensure_laser_tick_active(&self) {
        let mut st = self.state.borrow_mut();
        if st.laser_tick_active {
            return;
        }
        st.laser_tick_active = true;

        let state_clone = self.state.clone();
        let app_start = self.app_start;
        let area = self.area.clone();

        self.area.add_tick_callback(move |_widget, _frame_clock| {
            let mut st = state_clone.borrow_mut();
            let now_ns = app_start.elapsed().as_nanos() as u64;
            const LIFESPAN_NS: u64 = 1_200_000_000;

            // Pruning dei punti laser la cui durata ha superato 1.2s
            st.laser_trail
                .points
                .retain(|p| now_ns.saturating_sub(p.timestamp_ns) < LIFESPAN_NS);

            if st.laser_trail.points.is_empty() && !st.laser_trail.is_active {
                st.laser_tick_active = false;
                area.queue_draw();
                return glib::ControlFlow::Break;
            }

            area.queue_draw();
            glib::ControlFlow::Continue
        });
    }

    /// Disegna la scia volatile del puntatore laser con effetto neon glow a passate separate e spline Catmull-Rom continua.
    pub fn draw_laser_trail(
        cr: &gtk4::cairo::Context,
        trail: &LaserTrail,
        now_ns: u64,
        zoom: f64,
        base_laser_width: f64,
    ) {
        if trail.points.is_empty() {
            return;
        }

        const LIFESPAN_NS: u64 = 1_200_000_000;
        let points = &trail.points;
        let n = points.len();

        let core_width = (base_laser_width / zoom).clamp(1.2, 32.0);

        // 4 layer continui di diffusione ottica (Point Spread Function coerente):
        // Nessun filo sottile staccato o gradino brusco; il fascio è un raggio laser
        // compatto, potente, rosso fuoco brillante al centro e morbidamente diffuso all'esterno.
        let layers = [
            (core_width * 1.85, 1.0, 0.02, 0.05, 0.18),
            (core_width * 1.40, 1.0, 0.02, 0.05, 0.38),
            (core_width, 1.0, 0.04, 0.06, 0.88),
            (core_width * 0.65, 1.0, 0.20, 0.16, 0.96),
        ];

        if n == 1 {
            let p = &points[0];
            let age_ns = now_ns.saturating_sub(p.timestamp_ns);
            if age_ns < LIFESPAN_NS {
                let alpha = (1.0 - (age_ns as f64 / LIFESPAN_NS as f64)).clamp(0.0, 1.0);
                let _ = cr.save();
                for &(w, r, g, b, alpha_mult) in &layers {
                    cr.set_source_rgba(r, g, b, alpha * alpha_mult);
                    cr.arc(p.x, p.y, w * 0.5, 0.0, 2.0 * std::f64::consts::PI);
                    let _ = cr.fill();
                }
                let _ = cr.restore();
            }
            return;
        }

        // Calcola in anticipo i punti di controllo di Bézier cubica centripeta per tutti i segmenti (continuità C^1 fluida)
        struct BezierSeg {
            p0: (f64, f64),
            cp1: (f64, f64),
            cp2: (f64, f64),
            p1: (f64, f64),
            alpha: f64,
        }

        let mut segs = Vec::with_capacity(n - 1);
        for i in 0..(n - 1) {
            let p_i = (points[i].x, points[i].y);
            let p_ip1 = (points[i + 1].x, points[i + 1].y);

            let age0 = now_ns.saturating_sub(points[i].timestamp_ns);
            let age1 = now_ns.saturating_sub(points[i + 1].timestamp_ns);
            let avg_age = (age0 + age1) / 2;

            if avg_age >= LIFESPAN_NS {
                continue;
            }

            let alpha = (1.0 - (avg_age as f64 / LIFESPAN_NS as f64)).clamp(0.0, 1.0);

            let p_prev = if i == 0 {
                (2.0 * p_i.0 - p_ip1.0, 2.0 * p_i.1 - p_ip1.1)
            } else {
                (points[i - 1].x, points[i - 1].y)
            };

            let p_next = if i + 1 == n - 1 {
                (2.0 * p_ip1.0 - p_i.0, 2.0 * p_ip1.1 - p_i.1)
            } else {
                (points[i + 2].x, points[i + 2].y)
            };

            let cp1 = (
                p_i.0 + (p_ip1.0 - p_prev.0) / 6.0,
                p_i.1 + (p_ip1.1 - p_prev.1) / 6.0,
            );
            let cp2 = (
                p_ip1.0 - (p_next.0 - p_i.0) / 6.0,
                p_ip1.1 - (p_next.1 - p_i.1) / 6.0,
            );

            segs.push(BezierSeg {
                p0: p_i,
                cp1,
                cp2,
                p1: p_ip1,
                alpha,
            });
        }

        if segs.is_empty() {
            return;
        }

        let first_p0 = segs[0].p0;
        let first_alpha = segs[0].alpha;
        let last_p1 = segs.last().unwrap().p1;
        let last_alpha = segs.last().unwrap().alpha;

        let _ = cr.save();
        cr.set_line_cap(gtk4::cairo::LineCap::Butt);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);

        for &(width, r, g, b, alpha_mult) in &layers {
            cr.set_line_width(width);
            for seg in &segs {
                if seg.alpha <= 0.01 {
                    continue;
                }
                cr.set_source_rgba(r, g, b, seg.alpha * alpha_mult);
                cr.move_to(seg.p0.0, seg.p0.1);
                cr.curve_to(
                    seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
                );
                let _ = cr.stroke();
            }

            // Cap arrotondato naturale SOLO alla punta attiva dello stilo/laser
            let cap_radius = width * 0.5;
            cr.set_source_rgba(r, g, b, last_alpha * alpha_mult);
            cr.arc(
                last_p1.0,
                last_p1.1,
                cap_radius,
                0.0,
                2.0 * std::f64::consts::PI,
            );
            let _ = cr.fill();

            // Cap arrotondato alla coda solo se la coda è ancora visibile (non svanita)
            if first_alpha > 0.05 {
                cr.set_source_rgba(r, g, b, first_alpha * alpha_mult);
                cr.arc(
                    first_p0.0,
                    first_p0.1,
                    cap_radius,
                    0.0,
                    2.0 * std::f64::consts::PI,
                );
                let _ = cr.fill();
            }
        }

        let _ = cr.restore();
    }
}

/// Helper per il disegno di pillole/squircle arrotondati con Cairo.
fn draw_dms_pill(
    cr: &gtk4::cairo::Context,
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
    cyan: (f64, f64, f64),
    is_dark: bool,
) {
    let r = (w.min(h)) / 2.0;
    let x = cx - w / 2.0;
    let y = cy - h / 2.0;

    // Ombra
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.25);
    draw_rounded_rect_path(cr, x, y + 1.0, w, h, r);
    let _ = cr.fill();

    // Riempimento
    if is_dark {
        cr.set_source_rgb(0.92, 0.95, 0.98);
    } else {
        cr.set_source_rgb(1.0, 1.0, 1.0);
    }
    draw_rounded_rect_path(cr, x, y, w, h, r);
    let _ = cr.fill();

    // Bordo Electric Cyan
    cr.set_source_rgb(cyan.0, cyan.1, cyan.2);
    cr.set_line_width(1.5);
    draw_rounded_rect_path(cr, x, y, w, h, r);
    let _ = cr.stroke();
}

fn draw_rounded_rect_path(cr: &gtk4::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
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
