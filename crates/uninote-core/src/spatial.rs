//! Motore di indicizzazione spaziale 2D basato su R*-tree per UniNote.
//!
//! Fornisce strutture leggere ad alte prestazioni per accelerare culling del viewport (120 FPS),
//! cancellazione con gomma (O(log N + K)) e pre-filtraggio per strumenti di selezione.

use crate::model::{Item, ItemKind};
use rstar::{RTree, RTreeObject, AABB};
use serde::{Deserialize, Serialize};
use uninote_stroke::Rect;

/// Converte un `Rect` di `uninote_stroke` in un `AABB<[f64; 2]>` di `rstar`.
/// Gestisce la normalizzazione e protegge da valori non finiti (NaN, Inf).
#[inline]
pub fn rect_to_aabb(rect: &Rect) -> AABB<[f64; 2]> {
    let norm = rect.normalized();
    let clamp_limit = 1e12;
    let min_x = if norm.min_x.is_finite() {
        norm.min_x.clamp(-clamp_limit, clamp_limit)
    } else {
        0.0
    };
    let min_y = if norm.min_y.is_finite() {
        norm.min_y.clamp(-clamp_limit, clamp_limit)
    } else {
        0.0
    };
    let max_x = if norm.max_x.is_finite() {
        norm.max_x.clamp(-clamp_limit, clamp_limit)
    } else {
        0.0
    };
    let max_y = if norm.max_y.is_finite() {
        norm.max_y.clamp(-clamp_limit, clamp_limit)
    } else {
        0.0
    };
    AABB::from_corners([min_x, min_y], [max_x, max_y])
}

/// Elemento leggero memorizzato nelle foglie dell'albero R*.
/// Occupa solo 48 byte ed implementa Copy per massimizzare la cache locality
/// e azzerare l'overhead di allocazione heap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SpatialItem {
    pub item_id: u64,
    pub z_index: u64,
    pub aabb: AABB<[f64; 2]>,
}

impl SpatialItem {
    /// Costruisce una nuova istanza di `SpatialItem` a partire dall'AABB specificato.
    #[inline]
    pub fn new(item_id: u64, z_index: u64, aabb: AABB<[f64; 2]>) -> Self {
        Self {
            item_id,
            z_index,
            aabb,
        }
    }

    /// Costruisce un `SpatialItem` a partire da un `Rect` delimitatore.
    #[inline]
    pub fn from_rect(item_id: u64, z_index: u64, bbox: &Rect) -> Self {
        Self {
            item_id,
            z_index,
            aabb: rect_to_aabb(bbox),
        }
    }
}

impl RTreeObject for SpatialItem {
    type Envelope = AABB<[f64; 2]>;

    #[inline]
    fn envelope(&self) -> Self::Envelope {
        self.aabb
    }
}

/// Indice spaziale 2D R*-Tree a livello di singola pagina per culling ed interrogazioni rapide.
#[derive(Debug, Clone)]
pub struct PageSpatialIndex {
    tree: RTree<SpatialItem>,
}

impl PartialEq for PageSpatialIndex {
    fn eq(&self, other: &Self) -> bool {
        if self.len() != other.len() {
            return false;
        }
        let mut self_items: Vec<SpatialItem> = self.tree.iter().copied().collect();
        let mut other_items: Vec<SpatialItem> = other.tree.iter().copied().collect();
        self_items.sort_unstable_by_key(|it| (it.item_id, it.z_index));
        other_items.sort_unstable_by_key(|it| (it.item_id, it.z_index));
        self_items == other_items
    }
}

impl Default for PageSpatialIndex {
    fn default() -> Self {
        Self::empty()
    }
}

impl PageSpatialIndex {
    /// Crea un indice spaziale vuoto.
    pub fn empty() -> Self {
        Self { tree: RTree::new() }
    }

    /// Costruisce l'indice bilanciato ottimale tramite algoritmo OMT (Overlap-Minimizing Top-Down)
    /// su tutti gli elementi della pagina aventi un bounding box calcolabile.
    pub fn bulk_load(items: &[Item]) -> Self {
        let mut entries = Vec::with_capacity(items.len());
        for item in items {
            match &item.kind {
                ItemKind::Stroke(stroke) => {
                    if let Some(bbox) = stroke.bounding_box() {
                        entries.push(SpatialItem::from_rect(item.id, item.z_index, &bbox));
                    }
                }
            }
        }
        Self {
            tree: RTree::bulk_load(entries),
        }
    }

    /// Inserisce un elemento nell'indice spaziale in tempo O(log N).
    pub fn insert(&mut self, item: &Item) {
        match &item.kind {
            ItemKind::Stroke(stroke) => {
                if let Some(bbox) = stroke.bounding_box() {
                    let entry = SpatialItem::from_rect(item.id, item.z_index, &bbox);
                    self.tree.insert(entry);
                }
            }
        }
    }

    /// Rimuove un elemento dall'indice spaziale identificato da `item_id` e dal suo `bbox`.
    /// Restituisce `true` se l'elemento è stato rimosso, `false` altrimenti.
    pub fn remove(&mut self, item_id: u64, bbox: &Rect) -> bool {
        let query_env = rect_to_aabb(bbox);
        let target = self
            .tree
            .locate_in_envelope_intersecting(query_env)
            .find(|it| it.item_id == item_id)
            .copied()
            .or_else(|| self.tree.iter().find(|it| it.item_id == item_id).copied());

        if let Some(target_item) = target {
            self.tree.remove(&target_item).is_some()
        } else {
            false
        }
    }

    /// Query spaziale su un inviluppo rettangolare (min_x, min_y, max_x, max_y).
    /// Restituisce tutti gli elementi intersecanti ordinati rigorosamente per `z_index` crescente,
    /// garantendo il corretto ordine pittorico naturale durante il rendering.
    pub fn locate_in_envelope(
        &self,
        min_x: f64,
        min_y: f64,
        max_x: f64,
        max_y: f64,
    ) -> Vec<SpatialItem> {
        let clamp_limit = 1e12;
        let x1 = if min_x.is_nan() {
            -clamp_limit
        } else {
            min_x.clamp(-clamp_limit, clamp_limit)
        };
        let x2 = if max_x.is_nan() {
            clamp_limit
        } else {
            max_x.clamp(-clamp_limit, clamp_limit)
        };
        let y1 = if min_y.is_nan() {
            -clamp_limit
        } else {
            min_y.clamp(-clamp_limit, clamp_limit)
        };
        let y2 = if max_y.is_nan() {
            clamp_limit
        } else {
            max_y.clamp(-clamp_limit, clamp_limit)
        };

        let env_min_x = x1.min(x2);
        let env_max_x = x1.max(x2);
        let env_min_y = y1.min(y2);
        let env_max_y = y1.max(y2);

        let envelope = AABB::from_corners([env_min_x, env_min_y], [env_max_x, env_max_y]);
        let mut results: Vec<SpatialItem> = self
            .tree
            .locate_in_envelope_intersecting(envelope)
            .copied()
            .collect();
        results.sort_unstable_by_key(|it| (it.z_index, it.item_id));
        results
    }

    /// Restituisce il numero di elementi indicizzati nell'albero R*.
    #[inline]
    pub fn len(&self) -> usize {
        self.tree.size()
    }

    /// Restituisce true se l'indice è vuoto.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.tree.size() == 0
    }

    /// Riferimento all'albero R* sottostante per ispezione o query avanzate.
    #[inline]
    pub fn tree(&self) -> &RTree<SpatialItem> {
        &self.tree
    }

    /// Svuota completamente l'indice spaziale.
    pub fn clear(&mut self) {
        self.tree = RTree::new();
    }
}
