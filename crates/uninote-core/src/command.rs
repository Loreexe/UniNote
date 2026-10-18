use crate::model::{Document, Item, ItemId, ItemKind, Page, PageBackground};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use uninote_stroke::Stroke;
use uuid::Uuid;

/// Comando reversibile sul documento UniNote.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DocCommand {
    /// Aggiunta di un nuovo tratto a una pagina.
    AddStroke { page_id: Uuid, stroke: Stroke },
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
            DocCommand::EraseItems {
                page_id,
                erased_items,
            } => {
                if let Some(page) = doc.get_page_mut(*page_id) {
                    let ids_to_remove: std::collections::HashSet<u64> =
                        erased_items.iter().map(|it| it.id).collect();
                    for it in erased_items {
                        let ItemKind::Stroke(s) = &it.kind;
                        if let Some(bbox) = s.bounding_box() {
                            page.spatial_index.remove(it.id, &bbox);
                        }
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
                                let ItemKind::Stroke(stroke) = &mut item.kind;
                                stroke.translate(*delta_x, *delta_y);
                            }
                        }
                        page.rebuild_spatial_index();
                    } else {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                let ItemKind::Stroke(stroke) = &mut item.kind;
                                if let Some(old_bbox) = stroke.bounding_box() {
                                    page.spatial_index.remove(item.id, &old_bbox);
                                }
                                stroke.translate(*delta_x, *delta_y);
                                page.spatial_index.insert(item);
                            }
                        }
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
                    }) {
                        let item = page.items.remove(pos);
                        let ItemKind::Stroke(s) = &item.kind;
                        if let Some(bbox) = s.bounding_box() {
                            page.spatial_index.remove(item.id, &bbox);
                        }
                    }
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
                                let ItemKind::Stroke(stroke) = &mut item.kind;
                                stroke.translate(-*delta_x, -*delta_y);
                            }
                        }
                        page.rebuild_spatial_index();
                    } else {
                        for item in &mut page.items {
                            if id_set.contains(&item.id) {
                                let ItemKind::Stroke(stroke) = &mut item.kind;
                                if let Some(cur_bbox) = stroke.bounding_box() {
                                    page.spatial_index.remove(item.id, &cur_bbox);
                                }
                                stroke.translate(-*delta_x, -*delta_y);
                                page.spatial_index.insert(item);
                            }
                        }
                    }
                }
            }
        }
    }
}
