use crate::types::{Rect, Stroke};
use serde::{Deserialize, Serialize};

/// Modalità dello strumento di selezione.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SelectionMode {
    /// Selezione a mano libera tramite contorno poligonale chiuso.
    Lasso,
    /// Selezione tramite rettangolo diagonale trascinato.
    Rectangle,
}

/// Algoritmo Ray-Casting (Even-Odd rule) per verificare se un punto 2D (px, py) cade all'interno di un poligono.
///
/// Un raggio orizzontale viene tracciato da (px, py) verso +infinito lungo l'asse X.
/// Ogni intersezione con un segmento del poligono inverte lo stato di appartenenza (dentro/fuori).
pub fn point_in_polygon(px: f64, py: f64, polygon: &[(f64, f64)]) -> bool {
    if polygon.len() < 3 {
        return false;
    }

    let mut inside = false;
    let n = polygon.len();
    let mut j = n - 1;

    for i in 0..n {
        let (ix, iy) = polygon[i];
        let (jx, jy) = polygon[j];

        // Verifica se la coordinata Y del punto è compresa tra le quote Y dei due vertici
        if ((iy > py) != (jy > py)) && (px < (jx - ix) * (py - iy) / (jy - iy) + ix) {
            inside = !inside;
        }
        j = i;
    }

    inside
}

/// Determina se un tratto è selezionato dal poligono del lazo.
///
/// Regola geometrica: almeno l'80% (>= 0.80) dei punti campionati del tratto deve cadere all'interno del poligono.
pub fn is_stroke_selected_by_lasso(stroke: &Stroke, polygon: &[(f64, f64)]) -> bool {
    if stroke.points.is_empty() || polygon.len() < 3 {
        return false;
    }

    let mut inside_count = 0;
    for pt in &stroke.points {
        if point_in_polygon(pt.x, pt.y, polygon) {
            inside_count += 1;
        }
    }

    let fraction = inside_count as f64 / stroke.points.len() as f64;
    fraction >= 0.80
}

/// Determina se un tratto è selezionato da un rettangolo di selezione.
///
/// Regola geometrica: il tratto è selezionato se il suo bounding box è interamente racchiuso nel rettangolo,
/// oppure se almeno l'80% (>= 0.80) dei suoi punti campionati cade all'interno del rettangolo.
pub fn is_stroke_selected_by_rect(stroke: &Stroke, rect: &Rect) -> bool {
    if stroke.points.is_empty() {
        return false;
    }

    let norm_rect = rect.normalized();

    if let Some(bbox) = stroke.bounding_box() {
        if norm_rect.contains_rect(&bbox) {
            return true;
        }
    }

    let mut inside_count = 0;
    for pt in &stroke.points {
        if norm_rect.contains_point(pt.x, pt.y) {
            inside_count += 1;
        }
    }

    let fraction = inside_count as f64 / stroke.points.len() as f64;
    fraction >= 0.80
}

/// Tratto ausiliario per consentire il passaggio trasparente di Stroke, &Stroke o &&Stroke a SelectionEngine.
pub trait AsStrokeRef {
    fn as_stroke(&self) -> &Stroke;
}

impl AsStrokeRef for Stroke {
    fn as_stroke(&self) -> &Stroke {
        self
    }
}

impl<T: AsStrokeRef> AsStrokeRef for &T {
    fn as_stroke(&self) -> &Stroke {
        (*self).as_stroke()
    }
}

/// Motore geometrico di selezione per tratti vettoriali liberi e bounding box.
pub struct SelectionEngine;

impl SelectionEngine {
    /// Seleziona gli identificatori (`id`) dei tratti che soddisfano la soglia del lazo (>= 80% punti interni).
    pub fn select_lasso<T, I>(strokes: I, lasso_points: &[(f64, f64)]) -> Vec<u64>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        if lasso_points.len() < 3 {
            return Vec::new();
        }

        let mut selected = Vec::new();
        for item in strokes {
            let stroke = item.as_stroke();
            if is_stroke_selected_by_lasso(stroke, lasso_points) {
                selected.push(stroke.id);
            }
        }
        selected
    }

    /// Alias di convenienza per `select_lasso`.
    pub fn select_by_lasso<T, I>(strokes: I, lasso_points: &[(f64, f64)]) -> Vec<u64>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        Self::select_lasso(strokes, lasso_points)
    }

    /// Seleziona gli identificatori (`id`) dei tratti che soddisfano il criterio del rettangolo
    /// (bounding box interamente racchiuso oppure >= 80% punti interni).
    pub fn select_rect<T, I>(strokes: I, rect: Rect) -> Vec<u64>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        let mut selected = Vec::new();
        for item in strokes {
            let stroke = item.as_stroke();
            if is_stroke_selected_by_rect(stroke, &rect) {
                selected.push(stroke.id);
            }
        }
        selected
    }

    /// Alias di convenienza per `select_rect`.
    pub fn select_by_rect<T, I>(strokes: I, rect: Rect) -> Vec<u64>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        Self::select_rect(strokes, rect)
    }

    /// Calcola l'AABB unificato (bounding box complessivo) dell'insieme di tratti specificato.
    pub fn compute_selection_bounding_box<T, I>(strokes: I) -> Option<Rect>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        let mut union_box: Option<Rect> = None;
        for item in strokes {
            let stroke = item.as_stroke();
            if let Some(bbox) = stroke.bounding_box() {
                union_box = match union_box {
                    Some(u) => Some(u.union(&bbox)),
                    None => Some(bbox),
                };
            }
        }
        union_box
    }

    /// Alias di convenienza per `compute_selection_bounding_box`.
    pub fn compute_bounding_box<T, I>(strokes: I) -> Option<Rect>
    where
        T: AsStrokeRef,
        I: IntoIterator<Item = T>,
    {
        Self::compute_selection_bounding_box(strokes)
    }
}
