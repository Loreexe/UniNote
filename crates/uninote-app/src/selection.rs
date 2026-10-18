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
