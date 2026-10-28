use crate::model::{Document, ImageItem, Item, ItemId, ItemKind, Page, PageBackground};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uninote_stroke::Stroke;
use uuid::Uuid;

/// Comando reversibile sul documento UniNote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocCommand {
    /// Aggiunta di un nuovo tratto a una pagina.
    AddStroke { page_id: Uuid, stroke: Stroke },
    /// Aggiunta di una nuova immagine a una pagina.
    AddImage { page_id: Uuid, item: Item },
    /// Cancellazione di uno o più elementi con preservazione per Undo.
    EraseItems {
        page_id: Uuid,
        erased_items: Vec<Item>,
    },
    /// Aggiunta di una nuova pagina in coda.
    AddPage { page: Page },
    /// Modifica del template di sfondo di una pagina.
    ChangeBackground {
        page_id: Uuid,
        old_bg: PageBackground,
        new_bg: PageBackground,
    },
    /// Spostamento relativo di uno o più elementi su una pagina.
    MoveItems {
        page_id: Uuid,
        item_ids: Vec<ItemId>,
        delta_x: f64,
        delta_y: f64,
    },
    /// Incolla di uno o più elementi su una pagina con preservazione R-Tree e z-index.
    PasteItems { page_id: Uuid, items: Vec<Item> },
    /// Trasformazione affine (scala e/o rotazione) di uno o più elementi attorno a un perno.
    TransformItems {
        page_id: Uuid,
        item_ids: Vec<ItemId>,
        origin_x: f64,
        origin_y: f64,
        scale_x: f64,
        scale_y: f64,
        rotation_rad: f64,
    },
    /// Modifica di stile (colore e/o spessore base) di elementi con snapshot pre/post per Undo.
    ModifyItemsStyle {
        page_id: Uuid,
        items_style: Vec<(
            ItemId,
            uninote_stroke::StrokeStyle,
            uninote_stroke::StrokeStyle,
        )>,
    },
    /// Sostituzione atomica di un tratto originale con N frammenti (taglio analitico con gomma a segmento).
    SliceStroke {
        page_id: Uuid,
        original_stroke: Item,
        new_strokes: Vec<Item>,
    },
}

impl DocCommand {
    /// Costruisce un comando di aggiunta immagine incapsulando direttamente l'Item.
    pub fn add_image(page_id: Uuid, item_id: ItemId, z_index: u64, mut image: ImageItem) -> Self {
        image.page_id = page_id;
        image.z_index = z_index as i64;
        Self::AddImage {
            page_id,
            item: Item::new_image(item_id, z_index, image),
        }
    }
}

/// Gestore della cronologia dei comandi per Undo / Redo e tracciamento delle revisioni monotone.
#[derive(Debug)]
pub struct CommandHistory {
    undo_stack: Vec<DocCommand>,
    redo_stack: Vec<DocCommand>,
    pub current_revision: u64,
    pub saved_revision: u64,
    pub max_depth: usize,
    next_item_id: u64,
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::new(100)
    }
}

impl CommandHistory {
    pub fn new(max_depth: usize) -> Self {
        Self::with_next_id(max_depth, 1)
    }

    pub fn with_next_id(max_depth: usize, next_id: u64) -> Self {
        Self {
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            current_revision: 0,
            saved_revision: 0,
            max_depth,
            next_item_id: next_id.max(1),
        }
    }

    pub fn next_item_id(&self) -> u64 {
        self.next_item_id
    }

    pub fn set_next_item_id(&mut self, next_id: u64) {
        self.next_item_id = next_id.max(1);
    }

    pub fn alloc_item_id(&mut self) -> u64 {
        let id = self.next_item_id;
        self.next_item_id += 1;
        id
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }

    pub fn is_dirty(&self) -> bool {
        self.current_revision != self.saved_revision
    }

    pub fn mark_saved(&mut self, revision: u64) {
        self.saved_revision = revision;
    }

    /// Esegue un comando sul documento, aggiornando la cronologia e la revisione.
    pub fn execute(&mut self, cmd: DocCommand, doc: &mut Document) {
        self.apply_command(&cmd, doc);
        self.undo_stack.push(cmd);
        self.redo_stack.clear();
        self.current_revision += 1;

        if self.undo_stack.len() > self.max_depth {
            self.undo_stack.remove(0);
        }
    }

    /// Annulla l'ultimo comando eseguito (Undo).
    pub fn undo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.undo_stack.pop() {
            self.revert_command(&cmd, doc);
            self.redo_stack.push(cmd);
            self.current_revision += 1;
            true
        } else {
            false
        }
    }

    /// Ripristina l'ultimo comando annullato (Redo).
    pub fn redo(&mut self, doc: &mut Document) -> bool {
        if let Some(cmd) = self.redo_stack.pop() {
            self.apply_command(&cmd, doc);
            self.undo_stack.push(cmd);
            self.current_revision += 1;
            true
        } else {
            false
        }
    }

    fn apply_command(&mut self, cmd: &DocCommand, doc: &mut Document) {
        match cmd {
            DocCommand::AddStroke { page_id, stroke } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let item_id = self.next_item_id;
                    self.next_item_id += 1;
                    let z_index = page.items.len() as u64 + 1;
                    let item = Item {
                        id: item_id,
                        z_index,
                        kind: ItemKind::Stroke(stroke.clone()),
                    };
                    page.spatial_index.insert(&item);
                    page.items.push(item);
                }
            }
            DocCommand::AddImage { page_id, item } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    page.spatial_index.insert(item);
                    page.items.push(item.clone());
                }
            }
            DocCommand::EraseItems {
                page_id,
                erased_items,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let ids_to_remove: std::collections::HashSet<u64> =
                        erased_items.iter().map(|it| it.id).collect();
                    for it in erased_items {
                        page.spatial_index.remove_item(it);
                    }
                    page.items.retain(|it| !ids_to_remove.contains(&it.id));
                }
            }
            DocCommand::AddPage { page } => {
                doc.pages.push(page.clone());
            }
            DocCommand::ChangeBackground {
                page_id, new_bg, ..
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    page.background = new_bg.clone();
                }
            }
            DocCommand::MoveItems {
                page_id,
                item_ids,
                delta_x,
                delta_y,
            } => {
                if !delta_x.is_finite() || !delta_y.is_finite() || item_ids.is_empty() {
                    return;
                }
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let id_set: HashSet<ItemId> = item_ids.iter().copied().collect();
                    const REBUILD_THRESHOLD: usize = 250;
                    if item_ids.len() > REBUILD_THRESHOLD {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                item.translate(*delta_x, *delta_y);
                            }
                        }
                        page.rebuild_spatial_index();
                    } else {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                if let Some(old_bbox) = item.bounding_box() {
                                    page.spatial_index.remove(item.id, &old_bbox);
                                }
                                item.translate(*delta_x, *delta_y);
                                page.spatial_index.insert(item);
                            }
                        }
                    }
                }
            }
            DocCommand::PasteItems { page_id, items } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    for item in items {
                        page.spatial_index.insert(item);
                        page.items.push(item.clone());
                    }
                    page.items.sort_by_key(|it| it.z_index);
                }
            }
            DocCommand::TransformItems {
                page_id,
                item_ids,
                origin_x,
                origin_y,
                scale_x,
                scale_y,
                rotation_rad,
            } => {
                if !origin_x.is_finite()
                    || !origin_y.is_finite()
                    || !scale_x.is_finite()
                    || !scale_y.is_finite()
                    || !rotation_rad.is_finite()
                    || item_ids.is_empty()
                {
                    return;
                }
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let id_set: HashSet<ItemId> = item_ids.iter().copied().collect();
                    for item in &mut page.items {
                        if id_set.contains(&item.id) {
                            if let Some(old_bbox) = item.bounding_box() {
                                page.spatial_index.remove(item.id, &old_bbox);
                            }
                            match &mut item.kind {
                                ItemKind::Stroke(stroke) => {
                                    if (*scale_x - 1.0).abs() > 1e-9
                                        || (*scale_y - 1.0).abs() > 1e-9
                                    {
                                        stroke.scale(*scale_x, *scale_y, *origin_x, *origin_y);
                                    }
                                    if rotation_rad.abs() > 1e-9 {
                                        stroke.rotate(*rotation_rad, *origin_x, *origin_y);
                                    }
                                }
                                ItemKind::Image(image) => {
                                    if (*scale_x - 1.0).abs() > 1e-9
                                        || (*scale_y - 1.0).abs() > 1e-9
                                    {
                                        image.scale(*scale_x, *scale_y, *origin_x, *origin_y);
                                    }
                                }
                            }
                            page.spatial_index.insert(item);
                        }
                    }
                }
            }
            DocCommand::ModifyItemsStyle {
                page_id,
                items_style,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    for (id, _old_style, new_style) in items_style {
                        if let Some(item) = page.items.iter_mut().find(|it| it.id == *id) {
                            let item_id = item.id;
                            if let Some(stroke) = item.as_stroke_mut() {
                                if let Some(old_bbox) = stroke.bounding_box() {
                                    page.spatial_index.remove(item_id, &old_bbox);
                                }
                                stroke.style = new_style.clone();
                                stroke.recompute_cache();
                                page.spatial_index.insert(item);
                            }
                        }
                    }
                }
            }
            DocCommand::SliceStroke {
                page_id,
                original_stroke,
                new_strokes,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    // 1. Rimuove original_stroke dall'indice spaziale
                    page.spatial_index.remove_item(original_stroke);

                    // 2. Rimuove original_stroke da page.items preservando la posizione di inserimento
                    let orig_pos = page.items.iter().position(|it| it.id == original_stroke.id);
                    if let Some(pos) = orig_pos {
                        page.items.remove(pos);
                        // 3. Inserisce i nuovi frammenti nella pagina e nell'indice spaziale
                        for (idx, new_item) in new_strokes.iter().enumerate() {
                            page.spatial_index.insert(new_item);
                            page.items.insert(pos + idx, new_item.clone());
                        }
                    } else {
                        // Fallback difensivo nel caso improbabile in cui l'ID non sia all'indice previsto
                        page.items.retain(|it| it.id != original_stroke.id);
                        for new_item in new_strokes {
                            page.spatial_index.insert(new_item);
                            page.items.push(new_item.clone());
                        }
                        page.items.sort_by_key(|it| it.z_index);
                    }
                }
            }
        }
    }

    fn revert_command(&mut self, cmd: &DocCommand, doc: &mut Document) {
        match cmd {
            DocCommand::AddStroke { page_id, stroke } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    // Rimuove l'ultimo stroke corrispondente inserito
                    if let Some(pos) = page.items.iter().rposition(|it| match &it.kind {
                        ItemKind::Stroke(s) => s.id == stroke.id,
                        _ => false,
                    }) {
                        let item = page.items.remove(pos);
                        page.spatial_index.remove_item(&item);
                    }
                }
            }
            DocCommand::AddImage { page_id, item } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    page.spatial_index.remove_item(item);
                    page.items.retain(|it| it.id != item.id);
                }
            }
            DocCommand::EraseItems {
                page_id,
                erased_items,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    for item in erased_items {
                        page.spatial_index.insert(item);
                        page.items.push(item.clone());
                    }
                    // Ripristina l'ordine corretto tramite z_index
                    page.items.sort_by_key(|it| it.z_index);
                }
            }
            DocCommand::AddPage { page } => {
                doc.pages.retain(|p| p.id != page.id);
            }
            DocCommand::ChangeBackground {
                page_id, old_bg, ..
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    page.background = old_bg.clone();
                }
            }
            DocCommand::MoveItems {
                page_id,
                item_ids,
                delta_x,
                delta_y,
            } => {
                if !delta_x.is_finite() || !delta_y.is_finite() || item_ids.is_empty() {
                    return;
                }
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let id_set: HashSet<ItemId> = item_ids.iter().copied().collect();
                    const REBUILD_THRESHOLD: usize = 250;
                    if item_ids.len() > REBUILD_THRESHOLD {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                item.translate(-*delta_x, -*delta_y);
                            }
                        }
                        page.rebuild_spatial_index();
                    } else {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                if let Some(cur_bbox) = item.bounding_box() {
                                    page.spatial_index.remove(item.id, &cur_bbox);
                                }
                                item.translate(-*delta_x, -*delta_y);
                                page.spatial_index.insert(item);
                            }
                        }
                    }
                }
            }
            DocCommand::PasteItems { page_id, items } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let ids_to_remove: HashSet<u64> = items.iter().map(|it| it.id).collect();
                    for it in items {
                        page.spatial_index.remove_item(it);
                    }
                    page.items.retain(|it| !ids_to_remove.contains(&it.id));
                }
            }
            DocCommand::TransformItems {
                page_id,
                item_ids,
                origin_x,
                origin_y,
                scale_x,
                scale_y,
                rotation_rad,
            } => {
                if !origin_x.is_finite()
                    || !origin_y.is_finite()
                    || !scale_x.is_finite()
                    || !scale_y.is_finite()
                    || !rotation_rad.is_finite()
                    || item_ids.is_empty()
                    || scale_x.abs() < 1e-9
                    || scale_y.abs() < 1e-9
                {
                    return;
                }
                let inv_sx = 1.0 / scale_x;
                let inv_sy = 1.0 / scale_y;
                let inv_rot = -rotation_rad;

                if let Some(page) = doc.get_page_mut(*page_id) {
                    let id_set: HashSet<ItemId> = item_ids.iter().copied().collect();
                    for item in &mut page.items {
                        if id_set.contains(&item.id) {
                            if let Some(cur_bbox) = item.bounding_box() {
                                page.spatial_index.remove(item.id, &cur_bbox);
                            }
                            match &mut item.kind {
                                ItemKind::Stroke(stroke) => {
                                    // Inverti rotazione prima e poi scala
                                    if inv_rot.abs() > 1e-9 {
                                        stroke.rotate(inv_rot, *origin_x, *origin_y);
                                    }
                                    if (inv_sx - 1.0).abs() > 1e-9 || (inv_sy - 1.0).abs() > 1e-9 {
                                        stroke.scale(inv_sx, inv_sy, *origin_x, *origin_y);
                                    }
                                }
                                ItemKind::Image(image) => {
                                    if (inv_sx - 1.0).abs() > 1e-9 || (inv_sy - 1.0).abs() > 1e-9 {
                                        image.scale(inv_sx, inv_sy, *origin_x, *origin_y);
                                    }
                                }
                            }
                            page.spatial_index.insert(item);
                        }
                    }
                }
            }
            DocCommand::ModifyItemsStyle {
                page_id,
                items_style,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    for (id, old_style, _new_style) in items_style {
                        if let Some(item) = page.items.iter_mut().find(|it| it.id == *id) {
                            let item_id = item.id;
                            if let Some(stroke) = item.as_stroke_mut() {
                                if let Some(cur_bbox) = stroke.bounding_box() {
                                    page.spatial_index.remove(item_id, &cur_bbox);
                                }
                                stroke.style = old_style.clone();
                                stroke.recompute_cache();
                                page.spatial_index.insert(item);
                            }
                        }
                    }
                }
            }
            DocCommand::SliceStroke {
                page_id,
                original_stroke,
                new_strokes,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    // 1. Rimuove tutti i frammenti new_strokes dall'indice spaziale
                    for new_item in new_strokes {
                        page.spatial_index.remove_item(new_item);
                    }

                    // 2. Rimuove i frammenti da page.items, individuando la posizione del primo per il reinserimento in-place
                    let restore_pos = if !new_strokes.is_empty() {
                        page.items.iter().position(|it| it.id == new_strokes[0].id)
                    } else {
                        None
                    };

                    let new_ids: HashSet<u64> = new_strokes.iter().map(|it| it.id).collect();
                    page.items.retain(|it| !new_ids.contains(&it.id));

                    // 3. Ripristina original_stroke in page.items
                    if let Some(pos) = restore_pos {
                        page.items
                            .insert(pos.min(page.items.len()), original_stroke.clone());
                    } else {
                        // Caso cancellazione completa (new_strokes vuoto o non trovato): riordina per z_index
                        page.items.push(original_stroke.clone());
                        page.items.sort_by_key(|it| it.z_index);
                    }

                    // 4. Ripristina original_stroke nell'indice spaziale
                    page.spatial_index.insert(original_stroke);
                }
            }
        }
    }
}
