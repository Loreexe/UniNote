//! Accelerated application-level selection engine with 2D R-Tree spatial pre-filtering.

use std::collections::HashSet;
use uninote_core::model::{ItemId, ItemKind, Page};
use uninote_stroke::selection::{
    is_stroke_selected_by_lasso, is_stroke_selected_by_rect, SelectionEngine,
};
use uninote_stroke::types::Rect;

/// Motore di selezione accelerato a livello applicativo con pre-filtraggio spaziale R-Tree.
pub struct AppSelectionEngine;

impl AppSelectionEngine {
    /// Selezione rettangolare accelerata tramite pre-filtraggio su `PageSpatialIndex`.
    pub fn select_rect(page: &Page, rect: &Rect) -> (Vec<ItemId>, Option<Rect>) {
        let norm_rect = rect.normalized();
        if norm_rect.width() < 2.0 && norm_rect.height() < 2.0 {
            return (Vec::new(), None);
        }

        // Se l'indice spaziale non è vuoto, pre-filtra tramite l'inviluppo rettangolare
        if !page.spatial_index.is_empty() {
            let candidates = page.spatial_index.locate_in_envelope(
                norm_rect.min_x,
                norm_rect.min_y,
                norm_rect.max_x,
                norm_rect.max_y,
            );
            if candidates.is_empty() {
                return (Vec::new(), None);
            }

            let candidate_set: HashSet<u64> = candidates.iter().map(|c| c.item_id).collect();
            let mut selected_ids = Vec::new();
            let mut selected_strokes = Vec::new();

            for item in &page.items {
                if !candidate_set.contains(&item.id) {
                    continue;
                }
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_rect(s, &norm_rect) {
                    selected_ids.push(item.id);
                    selected_strokes.push(s);
                }
            }

            let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
            (selected_ids, bbox)
        } else {
            // Fallback trasparente su scansione lineare se l'indice spaziale non è popolato
            let mut selected_ids = Vec::new();
            let mut selected_strokes = Vec::new();

            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_rect(s, &norm_rect) {
                    selected_ids.push(item.id);
                    selected_strokes.push(s);
                }
            }

            let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
            (selected_ids, bbox)
        }
    }

    /// Selezione lazo accelerata tramite pre-filtraggio AABB su `PageSpatialIndex`.
    pub fn select_lasso(page: &Page, poly: &[(f64, f64)]) -> (Vec<ItemId>, Option<Rect>) {
        if poly.len() < 3 {
            return (Vec::new(), None);
        }

        // Calcola AABB del poligono lazo
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for &(px, py) in poly {
            if px < min_x {
                min_x = px;
            }
            if px > max_x {
                max_x = px;
            }
            if py < min_y {
                min_y = py;
            }
            if py > max_y {
                max_y = py;
            }
        }

        if !min_x.is_finite() || !min_y.is_finite() || !max_x.is_finite() || !max_y.is_finite() {
            return (Vec::new(), None);
        }

        if !page.spatial_index.is_empty() {
            let candidates = page
                .spatial_index
                .locate_in_envelope(min_x, min_y, max_x, max_y);
            if candidates.is_empty() {
                return (Vec::new(), None);
            }

            let candidate_set: HashSet<u64> = candidates.iter().map(|c| c.item_id).collect();
            let mut selected_ids = Vec::new();
            let mut selected_strokes = Vec::new();

            for item in &page.items {
                if !candidate_set.contains(&item.id) {
                    continue;
                }
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_lasso(s, poly) {
                    selected_ids.push(item.id);
                    selected_strokes.push(s);
                }
            }

            let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
            (selected_ids, bbox)
        } else {
            // Fallback trasparente su scansione lineare se l'indice spaziale non è popolato
            let mut selected_ids = Vec::new();
            let mut selected_strokes = Vec::new();

            for item in &page.items {
                let ItemKind::Stroke(ref s) = item.kind;
                if is_stroke_selected_by_lasso(s, poly) {
                    selected_ids.push(item.id);
                    selected_strokes.push(s);
                }
            }

            let bbox = SelectionEngine::compute_selection_bounding_box(selected_strokes);
            (selected_ids, bbox)
        }
    }
}

/// Identificatore delle 8 maniglie perimetrali del bounding box di selezione.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HandleId {
    TopLeft,
    TopMiddle,
    TopRight,
    RightMiddle,
    BottomRight,
    BottomMiddle,
    BottomLeft,
    LeftMiddle,
}

impl HandleId {
    /// Restituisce tutte le 8 maniglie in ordine perimetrale orario a partire da TopLeft.
    pub const ALL: [HandleId; 8] = [
        HandleId::TopLeft,
        HandleId::TopMiddle,
        HandleId::TopRight,
        HandleId::RightMiddle,
        HandleId::BottomRight,
        HandleId::BottomMiddle,
        HandleId::BottomLeft,
        HandleId::LeftMiddle,
    ];

    /// Indica se la maniglia è un vertice d'angolo o un punto mediano di bordo.
    #[inline]
    pub fn is_corner(&self) -> bool {
        matches!(
            self,
            HandleId::TopLeft | HandleId::TopRight | HandleId::BottomRight | HandleId::BottomLeft
        )
    }

    /// Restituisce la maniglia o il bordo opposto da utilizzare come perno fisso (anchor).
    #[inline]
    pub fn opposite(&self) -> HandleId {
        match self {
            HandleId::TopLeft => HandleId::BottomRight,
            HandleId::TopRight => HandleId::BottomLeft,
            HandleId::BottomRight => HandleId::TopLeft,
            HandleId::BottomLeft => HandleId::TopRight,
            HandleId::TopMiddle => HandleId::BottomMiddle,
            HandleId::BottomMiddle => HandleId::TopMiddle,
            HandleId::LeftMiddle => HandleId::RightMiddle,
            HandleId::RightMiddle => HandleId::LeftMiddle,
        }
    }

    /// Calcola la coordinata del punto perno fisso (anchor) in coordinate documento dato l'AABB.
    pub fn anchor_point(&self, bbox: &Rect) -> (f64, f64) {
        let mid_x = (bbox.min_x + bbox.max_x) / 2.0;
        let mid_y = (bbox.min_y + bbox.max_y) / 2.0;
        match self {
            HandleId::TopLeft => (bbox.max_x, bbox.max_y),
            HandleId::TopRight => (bbox.min_x, bbox.max_y),
            HandleId::BottomRight => (bbox.min_x, bbox.min_y),
            HandleId::BottomLeft => (bbox.max_x, bbox.min_y),
            HandleId::TopMiddle => (mid_x, bbox.max_y),
            HandleId::BottomMiddle => (mid_x, bbox.min_y),
            HandleId::LeftMiddle => (bbox.max_x, mid_y),
            HandleId::RightMiddle => (bbox.min_x, mid_y),
        }
    }

    /// Calcola la posizione della maniglia in coordinate documento dato l'AABB.
    pub fn position(&self, bbox: &Rect) -> (f64, f64) {
        let mid_x = (bbox.min_x + bbox.max_x) / 2.0;
        let mid_y = (bbox.min_y + bbox.max_y) / 2.0;
        match self {
            HandleId::TopLeft => (bbox.min_x, bbox.min_y),
            HandleId::TopMiddle => (mid_x, bbox.min_y),
            HandleId::TopRight => (bbox.max_x, bbox.min_y),
            HandleId::RightMiddle => (bbox.max_x, mid_y),
            HandleId::BottomRight => (bbox.max_x, bbox.max_y),
            HandleId::BottomMiddle => (mid_x, bbox.max_y),
            HandleId::BottomLeft => (bbox.min_x, bbox.max_y),
            HandleId::LeftMiddle => (bbox.min_x, mid_y),
        }
    }
}

/// Tipologia di operazione di trascinamento interattivo sulla selezione.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SelectionDragKind {
    #[default]
    None,
    Translate,
    ScaleHandle(HandleId),
}

/// Esito dell'hit-test sul bounding box e sulle sue maniglie.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionHitTestResult {
    /// Contatto su una delle 8 maniglie di ridimensionamento perimetrali.
    Handle(HandleId),
    /// Contatto sul corpo interno del bounding box o sul grip centrale (traslazione).
    Body,
    /// Clic esterno: deselezione e avvio nuovo gesto lazo/rettangolo.
    None,
}

/// Calcola i fattori di scala (scale_x, scale_y) e il perno (origin_x, origin_y) rispetto al perno opposto.
///
/// Corner scaling: blocco rapporto d'aspetto (default) tramite proiezione ortogonale su diagonale o libero 2D.
/// Edge scaling: 1D lungo la normale (Sx=1 oppure Sy=1).
/// Clamp factor >= 0.05 e dimensione minima >= 5.0 pt lungo gli assi scalati con protezione anti-singolarità.
pub fn compute_scale_factors(
    handle: HandleId,
    bbox: &Rect,
    cur_doc_pt: (f64, f64),
    lock_aspect: bool,
) -> ((f64, f64), (f64, f64)) {
    let (ox, oy) = handle.anchor_point(bbox);
    let (p0_x, p0_y) = handle.position(bbox);
    let v0_x = p0_x - ox;
    let v0_y = p0_y - oy;
    let v_x = cur_doc_pt.0 - ox;
    let v_y = cur_doc_pt.1 - oy;

    let w0 = (bbox.max_x - bbox.min_x).abs().max(1e-4);
    let h0 = (bbox.max_y - bbox.min_y).abs().max(1e-4);

    let (mut sx, mut sy) = match handle {
        HandleId::LeftMiddle => {
            let factor = (bbox.max_x - cur_doc_pt.0) / w0;
            (factor, 1.0)
        }
        HandleId::RightMiddle => {
            let factor = (cur_doc_pt.0 - bbox.min_x) / w0;
            (factor, 1.0)
        }
        HandleId::TopMiddle => {
            let factor = (bbox.max_y - cur_doc_pt.1) / h0;
            (1.0, factor)
        }
        HandleId::BottomMiddle => {
            let factor = (cur_doc_pt.1 - bbox.min_y) / h0;
            (1.0, factor)
        }
        HandleId::TopLeft | HandleId::TopRight | HandleId::BottomRight | HandleId::BottomLeft => {
            if lock_aspect {
                let dot = v_x * v0_x + v_y * v0_y;
                let len_sq = v0_x * v0_x + v0_y * v0_y;
                let uniform_scale = if len_sq > 1e-6 { dot / len_sq } else { 1.0 };
                (uniform_scale, uniform_scale)
            } else {
                let factor_x = if v0_x.abs() > 1e-4 { v_x / v0_x } else { 1.0 };
                let factor_y = if v0_y.abs() > 1e-4 { v_y / v0_y } else { 1.0 };
                (factor_x, factor_y)
            }
        }
    };

    if !sx.is_finite() {
        sx = 1.0;
    }
    if !sy.is_finite() {
        sy = 1.0;
    }

    const MIN_SCALE: f64 = 0.05;
    const MIN_DIMENSION: f64 = 5.0;

    match handle {
        HandleId::LeftMiddle | HandleId::RightMiddle => {
            sx = sx.max(MIN_SCALE);
            if w0 * sx < MIN_DIMENSION {
                sx = MIN_DIMENSION / w0;
            }
            sy = 1.0;
        }
        HandleId::TopMiddle | HandleId::BottomMiddle => {
            sx = 1.0;
            sy = sy.max(MIN_SCALE);
            if h0 * sy < MIN_DIMENSION {
                sy = MIN_DIMENSION / h0;
            }
        }
        HandleId::TopLeft | HandleId::TopRight | HandleId::BottomRight | HandleId::BottomLeft => {
            sx = sx.max(MIN_SCALE);
            sy = sy.max(MIN_SCALE);
            if w0 * sx < MIN_DIMENSION {
                sx = MIN_DIMENSION / w0;
            }
            if h0 * sy < MIN_DIMENSION {
                sy = MIN_DIMENSION / h0;
            }
            if lock_aspect {
                let max_s = sx.max(sy);
                sx = max_s;
                sy = max_s;
            }
        }
    }

    ((ox, oy), (sx, sy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_handle_all_count_and_corners() {
        assert_eq!(HandleId::ALL.len(), 8);
        assert!(HandleId::TopLeft.is_corner());
        assert!(HandleId::TopRight.is_corner());
        assert!(HandleId::BottomRight.is_corner());
        assert!(HandleId::BottomLeft.is_corner());
        assert!(!HandleId::TopMiddle.is_corner());
        assert!(!HandleId::BottomMiddle.is_corner());
        assert!(!HandleId::LeftMiddle.is_corner());
        assert!(!HandleId::RightMiddle.is_corner());
    }

    #[test]
    fn test_handle_opposite_symmetry() {
        for handle in HandleId::ALL {
            assert_eq!(handle.opposite().opposite(), handle);
        }
    }

    #[test]
    fn test_handle_anchor_and_position() {
        let bbox = Rect::new(10.0, 20.0, 110.0, 220.0);
        // w = 100, h = 200, mid_x = 60, mid_y = 120
        assert_eq!(HandleId::TopLeft.position(&bbox), (10.0, 20.0));
        assert_eq!(HandleId::TopLeft.anchor_point(&bbox), (110.0, 220.0));

        assert_eq!(HandleId::BottomRight.position(&bbox), (110.0, 220.0));
        assert_eq!(HandleId::BottomRight.anchor_point(&bbox), (10.0, 20.0));

        assert_eq!(HandleId::TopMiddle.position(&bbox), (60.0, 20.0));
        assert_eq!(HandleId::TopMiddle.anchor_point(&bbox), (60.0, 220.0));

        assert_eq!(HandleId::RightMiddle.position(&bbox), (110.0, 120.0));
        assert_eq!(HandleId::RightMiddle.anchor_point(&bbox), (10.0, 120.0));
    }
}
