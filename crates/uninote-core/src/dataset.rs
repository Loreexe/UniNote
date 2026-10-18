//! Generatore deterministico di dataset accademici riproducibili (1k, 10k, 50k tratti).
//!
//! Conforme ai requisiti Milestone M5 (R1, R3, R5):
//! - Standalone PRNG a 64-bit LCG/PCG con zero dipendenze esterne.
//! - Workload accademici realistici: 60% paragrafi corsivi, 25% formule matematiche, 15% diagrammi/schemi.
//! - Supporto a Quaderno A4 (`DocumentFormat::Notebook`) e Lavagna Infinita (`DocumentFormat::Whiteboard`).
//! - Warmup automatico: `stroke.ensure_cache()` e `page.rebuild_spatial_index()`.

use crate::model::{
    Document, DocumentFormat, Item, ItemId, ItemKind, Page, PageBackground, A4_HEIGHT_PT,
    A4_WIDTH_PT,
};
use std::f64::consts::PI;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

/// Seed predefinito (0x554E_494E_D0C5_0001 = "UNIN_DOCS_1").
pub const DEFAULT_ACADEMIC_SEED: u64 = 0x554E_494E_D0C5_0001;

/// Scala dimensionale per la generazione del dataset accademico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcademicScale {
    /// 1.000 tratti: appunti di una lezione singola (3-4 pagine A4 o lavagna 2x2).
    Small1k,
    /// 10.000 tratti: quaderno di studio fitto multi-capitolo (25-35 pagine A4 o lavagna 4x4).
    Medium10k,
    /// 50.000 tratti: corso universitario completo / tesi (100-150 pagine A4 o lavagna 8x8).
    Massive50k,
    /// Alias per Massive50k (50.000 tratti).
    Large50k,
    /// Scala personalizzata con numero esatto di tratti.
    Custom(usize),
}

impl AcademicScale {
    pub fn target_strokes(&self) -> usize {
        match self {
            AcademicScale::Small1k => 1_000,
            AcademicScale::Medium10k => 10_000,
            AcademicScale::Massive50k | AcademicScale::Large50k => 50_000,
            AcademicScale::Custom(n) => *n,
        }
    }
}

/// Generatore pseudo-casuale lineare congruenziale (Knuth 64-bit LCG) con uscita uniforme.
#[derive(Debug, Clone)]
pub struct DeterministicPrng {
    state: u64,
}

impl DeterministicPrng {
    pub fn new(seed: u64) -> Self {
        let mut prng = Self {
            state: if seed == 0 {
                DEFAULT_ACADEMIC_SEED
            } else {
                seed
            },
        };
        // Warmup dello stato iniziale per disperdere i bit bassi
        for _ in 0..8 {
            prng.next_u64();
        }
        prng
    }

    #[inline]
    pub fn next_u64(&mut self) -> u64 {
        // Parametri Knuth 64-bit (MMIX / Numerical Recipes)
        self.state = self
            .state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        self.state
    }

    #[inline]
    pub fn next_f64(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64) / ((1u64 << 53) as f64)
    }

    #[inline]
    pub fn range_f64(&mut self, min: f64, max: f64) -> f64 {
        min + (max - min) * self.next_f64()
    }

    #[inline]
    pub fn range_usize(&mut self, min: usize, max: usize) -> usize {
        if min >= max {
            return min;
        }
        min + (self.next_u64() as usize % (max - min + 1))
    }

    #[inline]
    pub fn next_bool(&mut self, p: f64) -> bool {
        self.next_f64() < p
    }

    /// Genera campioni normali N(mean, std_dev^2) con trasformata di Box-Muller.
    pub fn gaussian_f64(&mut self, mean: f64, std_dev: f64) -> f64 {
        let u1 = self.next_f64().max(1e-15);
        let u2 = self.next_f64();
        let z0 = (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos();
        mean + z0 * std_dev
    }
}

/// Generatore deterministico ad alto livello di dataset universitari.
pub struct AcademicDatasetGenerator {
    prng: DeterministicPrng,
    current_item_id: u64,
}

impl Default for AcademicDatasetGenerator {
    fn default() -> Self {
        Self::new()
    }
}

impl AcademicDatasetGenerator {
    pub fn new() -> Self {
        Self::with_seed(DEFAULT_ACADEMIC_SEED)
    }

    pub fn with_seed(seed: u64) -> Self {
        Self {
            prng: DeterministicPrng::new(seed),
            current_item_id: 1,
        }
    }

    fn next_id(&mut self) -> ItemId {
        let id = self.current_item_id;
        self.current_item_id += 1;
        id
    }

    /// Genera un documento per il formato specificato.
    pub fn generate_document(&mut self, format: DocumentFormat, scale: AcademicScale) -> Document {
        match format {
            DocumentFormat::Notebook => self.generate_notebook(scale),
            DocumentFormat::Whiteboard => self.generate_whiteboard(scale),
        }
    }

    /// Genera un documento Quaderno A4 con il numero specificato di tratti accademici.
    pub fn generate_notebook(&mut self, scale: AcademicScale) -> Document {
        let total_target = scale.target_strokes();
        let mut doc = Document::new_notebook("Appunti Accademici Universitari");
        doc.pages.clear();

        let mut generated_so_far = 0;
        let mut page_index = 0;

        while generated_so_far < total_target {
            let mut page = Page::new_a4(page_index, PageBackground::Grid { grid_size: 14.17 });
            let mut cursor_y = 60.0;

            while cursor_y < (A4_HEIGHT_PT - 75.0) && generated_so_far < total_target {
                let remaining = total_target - generated_so_far;
                let choice = self.prng.range_usize(0, 99);

                let items = if choice < 60 || remaining < 4 {
                    // 60% Paragrafi corsivi
                    let lines = self.prng.range_usize(2, 4);
                    let max_w = A4_WIDTH_PT - 90.0;
                    let p_items = self.generate_paragraph(45.0, cursor_y, max_w, lines, remaining);
                    cursor_y += (lines as f64) * 22.0 + 14.0;
                    p_items
                } else if choice < 85 {
                    // 25% Formule matematiche
                    let cx = A4_WIDTH_PT / 2.0;
                    let f_items = self.generate_math_block(cx, cursor_y + 25.0, 320.0, remaining);
                    cursor_y += 65.0;
                    f_items
                } else {
                    // 15% Schemi e diagrammi geometrici
                    let cx = A4_WIDTH_PT / 2.0;
                    let d_items =
                        self.generate_diagram_block(cx, cursor_y + 45.0, 320.0, 90.0, remaining);
                    cursor_y += 105.0;
                    d_items
                };

                for mut it in items {
                    it.z_index = page.items.len() as u64 + 1;
                    page.items.push(it);
                    generated_so_far += 1;
                    if generated_so_far >= total_target {
                        break;
                    }
                }
            }

            // WARMUP: Reindicizzazione spaziale R-Tree immediata della pagina
            page.rebuild_spatial_index();
            doc.pages.push(page);
            page_index += 1;
        }

        doc
    }

    /// Genera un documento Lavagna Infinita strutturato in matrice 2D di panel tematici.
    pub fn generate_whiteboard(&mut self, scale: AcademicScale) -> Document {
        let total_target = scale.target_strokes();
        let (grid_cols, grid_rows) = match scale {
            AcademicScale::Small1k => (3, 3),   // 9 panels (~111 strokes/panel)
            AcademicScale::Medium10k => (8, 8), // 64 panels (~156 strokes/panel)
            AcademicScale::Massive50k | AcademicScale::Large50k => (16, 16), // 256 panels (~195 strokes/panel)
            AcademicScale::Custom(n) => {
                let panels = n.div_ceil(160).max(1);
                let side = (panels as f64).sqrt().ceil() as usize;
                (side, side)
            }
        };

        let mut doc = Document::new_whiteboard_with_title(
            "Lavagna Infinita Universitaria (Brainstorming & Teoria)",
            PageBackground::Blank,
        );
        let page = &mut doc.pages[0];

        let panel_w = 1200.0;
        let panel_h = 900.0;
        let gutter = 600.0;

        let total_panels = grid_cols * grid_rows;
        let strokes_per_panel = total_target.div_ceil(total_panels);
        let mut generated_so_far = 0;

        for r in 0..grid_rows {
            for c in 0..grid_cols {
                if generated_so_far >= total_target {
                    break;
                }
                let origin_x = (c as f64 - grid_cols as f64 / 2.0 + 0.5) * (panel_w + gutter);
                let origin_y = (r as f64 - grid_rows as f64 / 2.0 + 0.5) * (panel_h + gutter);

                let quota = strokes_per_panel.min(total_target - generated_so_far);
                let mut panel_strokes = 0;
                let col_w = (panel_w - 60.0) / 2.0;
                let mut col = 0;
                let mut local_y = origin_y + 40.0;

                while panel_strokes < quota && col < 2 && generated_so_far < total_target {
                    let col_x = origin_x + 30.0 + (col as f64) * (col_w + 20.0);
                    let rem = (quota - panel_strokes).min(total_target - generated_so_far);
                    let choice = self.prng.range_usize(0, 99);

                    let items = if choice < 60 || rem < 4 {
                        let lines = self.prng.range_usize(2, 4);
                        let p = self.generate_paragraph(col_x, local_y, col_w, lines, rem);
                        local_y += (lines as f64) * 22.0 + 14.0;
                        p
                    } else if choice < 85 {
                        let cx = col_x + col_w / 2.0;
                        let m = self.generate_math_block(cx, local_y + 25.0, col_w, rem);
                        local_y += 65.0;
                        m
                    } else {
                        let cx = col_x + col_w / 2.0;
                        let d = self.generate_diagram_block(
                            cx,
                            local_y + 45.0,
                            col_w.min(280.0),
                            90.0,
                            rem,
                        );
                        local_y += 105.0;
                        d
                    };

                    for mut it in items {
                        it.z_index = page.items.len() as u64 + 1;
                        page.items.push(it);
                        panel_strokes += 1;
                        generated_so_far += 1;
                        if generated_so_far >= total_target {
                            break;
                        }
                    }

                    if local_y >= (origin_y + panel_h - 40.0) {
                        col += 1;
                        local_y = origin_y + 40.0;
                    }
                }
            }
        }

        // Se residuano tratti per raggiungere il target esatto, li aggiungiamo al panel finale
        let overflow_x = (grid_cols as f64 / 2.0 + 1.0) * (panel_w + gutter);
        let mut overflow_y = 0.0;
        while generated_so_far < total_target {
            let rem = total_target - generated_so_far;
            let items = self.generate_paragraph(overflow_x, overflow_y, 500.0, 2, rem);
            for mut it in items {
                it.z_index = page.items.len() as u64 + 1;
                page.items.push(it);
                generated_so_far += 1;
                if generated_so_far >= total_target {
                    break;
                }
            }
            overflow_y += 60.0;
        }

        // WARMUP: Reindicizzazione spaziale R-Tree immediata della lavagna
        page.rebuild_spatial_index();
        doc
    }

    /// Genera un paragrafo di testo corsivo universitario con micro-drift e modulazione stilo MPP.
    pub fn generate_paragraph(
        &mut self,
        start_x: f64,
        start_y: f64,
        max_width: f64,
        lines: usize,
        max_items: usize,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let mut cur_y = start_y;

        for line_idx in 0..lines {
            if items.len() >= max_items {
                break;
            }
            let mut cur_x = if line_idx == 0 {
                start_x + 18.0
            } else {
                start_x
            };
            let line_drift_amp = self.prng.range_f64(-1.2, 1.2);
            let line_drift_freq = self.prng.range_f64(0.005, 0.015);

            while cur_x < (start_x + max_width - 30.0) && items.len() < max_items {
                let word_span = self.prng.range_f64(28.0, 65.0);
                let point_count = self.prng.range_usize(12, 24);
                let base_width = self.prng.range_f64(1.8, 2.5);

                // Colori accademici realistici
                let color_choice = self.prng.range_usize(0, 19);
                let color = if color_choice < 14 {
                    (0.08, 0.22, 0.65, 1.0) // Deep Blue accademico
                } else if color_choice < 18 {
                    (0.12, 0.13, 0.15, 1.0) // Charcoal Black
                } else {
                    (0.78, 0.12, 0.15, 1.0) // Rosso correzione/evidenza
                };

                let mut points = Vec::with_capacity(point_count);
                let mut px = cur_x;

                for i in 0..point_count {
                    let t = i as f64 / (point_count - 1).max(1) as f64;
                    let drift = (px * line_drift_freq).sin() * line_drift_amp;
                    let cursive_y = (t * 2.0 * PI * 2.5).sin() * 5.0 + (t * 4.0 * PI).cos() * 1.5;
                    let pressure =
                        (0.32 + 0.48 * (t * PI).sin() + self.prng.range_f64(-0.04, 0.04))
                            .clamp(0.15, 0.95);
                    let pt_w = base_width * (0.6 + 0.8 * pressure);

                    points.push(StrokePoint {
                        x: px,
                        y: cur_y + drift + cursive_y,
                        pressure,
                        width: pt_w,
                        timestamp_ns: 1_000_000 * (i as u64),
                    });

                    px += (word_span / point_count as f64) + self.prng.range_f64(-0.3, 0.6);
                }

                let mut stroke = Stroke {
                    id: self.next_id(),
                    points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.5,
                        max_width_factor: 1.6,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                stroke.ensure_cache();
                items.push(Item {
                    id: stroke.id,
                    z_index: 0,
                    kind: ItemKind::Stroke(stroke),
                });

                cur_x = px + self.prng.range_f64(8.0, 14.0);

                // 8% probabilità di punto fermo isolato (is_dot)
                if self.prng.next_bool(0.08) && items.len() < max_items {
                    let dot_id = self.next_id();
                    let mut dot_stroke = Stroke {
                        id: dot_id,
                        points: vec![StrokePoint {
                            x: cur_x,
                            y: cur_y + 3.0,
                            pressure: 0.6,
                            width: 2.8,
                            timestamp_ns: 2_000_000,
                        }],
                        style: StrokeStyle::default(),
                        raw_samples: Vec::new(),
                        is_dot: true,
                        mode: PipelineMode::Baseline,
                        cached_bbox: None,
                        cached_bezier: Vec::new(),
                    };
                    dot_stroke.ensure_cache();
                    items.push(Item {
                        id: dot_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(dot_stroke),
                    });
                    cur_x += 8.0;
                }
            }

            cur_y += 22.0;
        }

        items
    }

    /// Genera un blocco di formule matematiche universitarie (frazioni, integrali, radici, sommatorie).
    pub fn generate_math_block(
        &mut self,
        cx: f64,
        cy: f64,
        _width: f64,
        max_items: usize,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let archetype = self.prng.range_usize(0, 3);
        let color = (0.05, 0.20, 0.55, 1.0); // Academic deep formula blue
        let base_width = 2.2;

        match archetype {
            0 => {
                // 1. Frazione: Barra di divisione orizzontale + numeratore + denominatore
                let bar_w = self.prng.range_f64(45.0, 75.0);
                let bar_id = self.next_id();
                let mut bar_points = Vec::with_capacity(12);
                for i in 0..12 {
                    let t = i as f64 / 11.0;
                    bar_points.push(StrokePoint {
                        x: cx - bar_w / 2.0 + bar_w * t,
                        y: cy + self.prng.range_f64(-0.2, 0.2),
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: (i as u64) * 1000,
                    });
                }
                let mut bar_stroke = Stroke {
                    id: bar_id,
                    points: bar_points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.8,
                        max_width_factor: 1.2,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                bar_stroke.ensure_cache();
                items.push(Item {
                    id: bar_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(bar_stroke),
                });

                // Numeratore (2 tratti)
                for (ox, oy) in [(-15.0, -14.0), (10.0, -14.0)] {
                    if items.len() >= max_items {
                        break;
                    }
                    let s_id = self.next_id();
                    let mut s = self.generate_symbol_stroke(s_id, cx + ox, cy + oy, 12.0, color);
                    s.ensure_cache();
                    items.push(Item {
                        id: s_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(s),
                    });
                }

                // Denominatore (2 tratti)
                for (ox, oy) in [(-15.0, 14.0), (10.0, 14.0)] {
                    if items.len() >= max_items {
                        break;
                    }
                    let s_id = self.next_id();
                    let mut s = self.generate_symbol_stroke(s_id, cx + ox, cy + oy, 12.0, color);
                    s.ensure_cache();
                    items.push(Item {
                        id: s_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(s),
                    });
                }
            }
            1 => {
                // 2. Integrale definito (\int_a^b f(x) dx)
                let int_id = self.next_id();
                let mut int_points = Vec::with_capacity(30);
                for i in 0..30 {
                    let t = i as f64 / 29.0;
                    let y = cy - 28.0 + 56.0 * t;
                    let hook = (t * PI).cos() * 8.5;
                    let pressure = 0.40 + 0.45 * (t * PI).sin();
                    int_points.push(StrokePoint {
                        x: cx - 50.0 + hook,
                        y,
                        pressure,
                        width: base_width * (0.7 + 0.6 * pressure),
                        timestamp_ns: (i as u64) * 2000,
                    });
                }
                let mut int_stroke = Stroke {
                    id: int_id,
                    points: int_points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.6,
                        max_width_factor: 1.5,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                int_stroke.ensure_cache();
                items.push(Item {
                    id: int_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(int_stroke),
                });

                // Pedice e apice
                if items.len() < max_items {
                    let sub_id = self.next_id();
                    let mut sub =
                        self.generate_symbol_stroke(sub_id, cx - 42.0, cy + 24.0, 7.0, color);
                    sub.ensure_cache();
                    items.push(Item {
                        id: sub_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(sub),
                    });
                }
                if items.len() < max_items {
                    let sup_id = self.next_id();
                    let mut sup =
                        self.generate_symbol_stroke(sup_id, cx - 38.0, cy - 24.0, 7.0, color);
                    sup.ensure_cache();
                    items.push(Item {
                        id: sup_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(sup),
                    });
                }

                // Funzione f(x) e dx
                for ox in [-15.0, 10.0, 30.0] {
                    if items.len() >= max_items {
                        break;
                    }
                    let s_id = self.next_id();
                    let mut s = self.generate_symbol_stroke(s_id, cx + ox, cy, 14.0, color);
                    s.ensure_cache();
                    items.push(Item {
                        id: s_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(s),
                    });
                }
            }
            2 => {
                // 3. Radice quadrata con radicando (\sqrt{...})
                let rad_id = self.next_id();
                let rad_points = vec![
                    StrokePoint {
                        x: cx - 40.0,
                        y: cy + 4.0,
                        pressure: 0.4,
                        width: base_width,
                        timestamp_ns: 1000,
                    },
                    StrokePoint {
                        x: cx - 34.0,
                        y: cy + 16.0,
                        pressure: 0.7,
                        width: base_width * 1.2,
                        timestamp_ns: 2000,
                    },
                    StrokePoint {
                        x: cx - 26.0,
                        y: cy - 22.0,
                        pressure: 0.9,
                        width: base_width * 1.5,
                        timestamp_ns: 3000,
                    },
                    StrokePoint {
                        x: cx + 45.0,
                        y: cy - 22.0,
                        pressure: 0.6,
                        width: base_width,
                        timestamp_ns: 4000,
                    },
                ];
                let mut rad_stroke = Stroke {
                    id: rad_id,
                    points: rad_points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.7,
                        max_width_factor: 1.4,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                rad_stroke.ensure_cache();
                items.push(Item {
                    id: rad_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(rad_stroke),
                });

                // Contenuto sotto radice
                for ox in [-10.0, 15.0] {
                    if items.len() >= max_items {
                        break;
                    }
                    let s_id = self.next_id();
                    let mut s = self.generate_symbol_stroke(s_id, cx + ox, cy - 6.0, 12.0, color);
                    s.ensure_cache();
                    items.push(Item {
                        id: s_id,
                        z_index: 0,
                        kind: ItemKind::Stroke(s),
                    });
                }
            }
            _ => {
                // 4. Lettera Greca / Sommatoria \Sigma
                let sig_id = self.next_id();
                let sig_points = vec![
                    StrokePoint {
                        x: cx - 18.0,
                        y: cy - 20.0,
                        pressure: 0.6,
                        width: base_width,
                        timestamp_ns: 1000,
                    },
                    StrokePoint {
                        x: cx + 18.0,
                        y: cy - 20.0,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: 2000,
                    },
                    StrokePoint {
                        x: cx - 4.0,
                        y: cy,
                        pressure: 0.85,
                        width: base_width * 1.2,
                        timestamp_ns: 3000,
                    },
                    StrokePoint {
                        x: cx + 18.0,
                        y: cy + 20.0,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: 4000,
                    },
                    StrokePoint {
                        x: cx - 18.0,
                        y: cy + 20.0,
                        pressure: 0.6,
                        width: base_width,
                        timestamp_ns: 5000,
                    },
                ];
                let mut sig_stroke = Stroke {
                    id: sig_id,
                    points: sig_points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.8,
                        max_width_factor: 1.3,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                sig_stroke.ensure_cache();
                items.push(Item {
                    id: sig_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(sig_stroke),
                });
            }
        }

        items
    }

    /// Genera uno schema o diagramma geometrico (assi cartesiani con curva analitica o flowchart a blocchi).
    pub fn generate_diagram_block(
        &mut self,
        cx: f64,
        cy: f64,
        width: f64,
        height: f64,
        max_items: usize,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        let diag_type = self.prng.range_usize(0, 1);
        let color = (0.15, 0.18, 0.22, 1.0);
        let base_width = 2.0;

        if diag_type == 0 {
            // Assi cartesiani X-Y con curva analitica
            let x_min = cx - width / 2.0 + 20.0;
            let x_max = cx + width / 2.0 - 20.0;
            let y_min = cy - height / 2.0 + 15.0;
            let y_max = cy + height / 2.0 - 15.0;

            // Asse Y verticale
            let y_axis_id = self.next_id();
            let mut y_points = Vec::with_capacity(8);
            for i in 0..8 {
                let t = i as f64 / 7.0;
                y_points.push(StrokePoint {
                    x: x_min + 30.0,
                    y: y_max - (y_max - y_min) * t,
                    pressure: 0.7,
                    width: base_width,
                    timestamp_ns: (i as u64) * 1000,
                });
            }
            let mut y_stroke = Stroke {
                id: y_axis_id,
                points: y_points,
                style: StrokeStyle {
                    color,
                    base_width,
                    min_width_factor: 0.8,
                    max_width_factor: 1.2,
                },
                raw_samples: Vec::new(),
                is_dot: false,
                mode: PipelineMode::CatmullRomSpline,
                cached_bbox: None,
                cached_bezier: Vec::new(),
            };
            y_stroke.ensure_cache();
            items.push(Item {
                id: y_axis_id,
                z_index: 0,
                kind: ItemKind::Stroke(y_stroke),
            });

            // Asse X orizzontale
            if items.len() < max_items {
                let x_axis_id = self.next_id();
                let mut x_points = Vec::with_capacity(8);
                let axis_y = y_max - 20.0;
                for i in 0..8 {
                    let t = i as f64 / 7.0;
                    x_points.push(StrokePoint {
                        x: x_min + 15.0 + (x_max - x_min) * t,
                        y: axis_y,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: (i as u64) * 1000,
                    });
                }
                let mut x_stroke = Stroke {
                    id: x_axis_id,
                    points: x_points,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.8,
                        max_width_factor: 1.2,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                x_stroke.ensure_cache();
                items.push(Item {
                    id: x_axis_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(x_stroke),
                });
            }

            // Curva analitica tracciata (es. sinusoide o risposta)
            if items.len() < max_items {
                let curve_id = self.next_id();
                let curve_color = (0.85, 0.20, 0.25, 1.0); // Academic Red Curve
                let mut curve_pts = Vec::with_capacity(24);
                let start_plot_x = x_min + 30.0;
                let plot_w = x_max - start_plot_x - 10.0;
                let axis_y = y_max - 20.0;

                for i in 0..24 {
                    let t = i as f64 / 23.0;
                    let px = start_plot_x + plot_w * t;
                    let py = axis_y - (t * PI * 1.5).sin() * (height * 0.55);
                    curve_pts.push(StrokePoint {
                        x: px,
                        y: py,
                        pressure: 0.6,
                        width: base_width * 1.1,
                        timestamp_ns: (i as u64) * 1500,
                    });
                }
                let mut c_stroke = Stroke {
                    id: curve_id,
                    points: curve_pts,
                    style: StrokeStyle {
                        color: curve_color,
                        base_width: 2.2,
                        min_width_factor: 0.8,
                        max_width_factor: 1.3,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                c_stroke.ensure_cache();
                items.push(Item {
                    id: curve_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(c_stroke),
                });
            }
        } else {
            // Flowchart / Schema a blocchi con freccia
            let box_w = 90.0;
            let box_h = 45.0;

            for (bx, by) in [(cx - 70.0, cy), (cx + 70.0, cy)] {
                if items.len() >= max_items {
                    break;
                }
                let box_id = self.next_id();
                let rect_pts = vec![
                    StrokePoint {
                        x: bx - box_w / 2.0,
                        y: by - box_h / 2.0,
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: 1000,
                    },
                    StrokePoint {
                        x: bx + box_w / 2.0,
                        y: by - box_h / 2.0,
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: 2000,
                    },
                    StrokePoint {
                        x: bx + box_w / 2.0,
                        y: by + box_h / 2.0,
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: 3000,
                    },
                    StrokePoint {
                        x: bx - box_w / 2.0,
                        y: by + box_h / 2.0,
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: 4000,
                    },
                    StrokePoint {
                        x: bx - box_w / 2.0,
                        y: by - box_h / 2.0,
                        pressure: 0.65,
                        width: base_width,
                        timestamp_ns: 5000,
                    },
                ];
                let mut b_stroke = Stroke {
                    id: box_id,
                    points: rect_pts,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.8,
                        max_width_factor: 1.2,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                b_stroke.ensure_cache();
                items.push(Item {
                    id: box_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(b_stroke),
                });
            }

            // Freccia connettiva tra i due blocchi
            if items.len() < max_items {
                let arr_id = self.next_id();
                let arr_pts = vec![
                    StrokePoint {
                        x: cx - 25.0,
                        y: cy,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: 1000,
                    },
                    StrokePoint {
                        x: cx + 22.0,
                        y: cy,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: 2000,
                    },
                    StrokePoint {
                        x: cx + 14.0,
                        y: cy - 6.0,
                        pressure: 0.6,
                        width: base_width,
                        timestamp_ns: 3000,
                    },
                    StrokePoint {
                        x: cx + 22.0,
                        y: cy,
                        pressure: 0.7,
                        width: base_width,
                        timestamp_ns: 4000,
                    },
                    StrokePoint {
                        x: cx + 14.0,
                        y: cy + 6.0,
                        pressure: 0.6,
                        width: base_width,
                        timestamp_ns: 5000,
                    },
                ];
                let mut arr_stroke = Stroke {
                    id: arr_id,
                    points: arr_pts,
                    style: StrokeStyle {
                        color,
                        base_width,
                        min_width_factor: 0.8,
                        max_width_factor: 1.2,
                    },
                    raw_samples: Vec::new(),
                    is_dot: false,
                    mode: PipelineMode::CatmullRomSpline,
                    cached_bbox: None,
                    cached_bezier: Vec::new(),
                };
                arr_stroke.ensure_cache();
                items.push(Item {
                    id: arr_id,
                    z_index: 0,
                    kind: ItemKind::Stroke(arr_stroke),
                });
            }
        }

        items
    }

    /// Ausiliario per generare un breve tratto simbolico curvilineo (variabile matematica).
    fn generate_symbol_stroke(
        &mut self,
        id: ItemId,
        x: f64,
        y: f64,
        size: f64,
        color: (f64, f64, f64, f64),
    ) -> Stroke {
        let count = 10;
        let mut points = Vec::with_capacity(count);
        for i in 0..count {
            let t = i as f64 / (count - 1) as f64;
            points.push(StrokePoint {
                x: x - size / 2.0 + size * t,
                y: y + (t * PI).sin() * (size / 2.0),
                pressure: 0.55,
                width: 2.0,
                timestamp_ns: (i as u64) * 1000,
            });
        }
        Stroke {
            id,
            points,
            style: StrokeStyle {
                color,
                base_width: 2.0,
                min_width_factor: 0.7,
                max_width_factor: 1.3,
            },
            raw_samples: Vec::new(),
            is_dot: false,
            mode: PipelineMode::CatmullRomSpline,
            cached_bbox: None,
            cached_bezier: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_prng_deterministic_sequence() {
        let mut prng1 = DeterministicPrng::new(DEFAULT_ACADEMIC_SEED);
        let mut prng2 = DeterministicPrng::new(DEFAULT_ACADEMIC_SEED);

        for _ in 0..100 {
            assert_eq!(prng1.next_u64(), prng2.next_u64());
            assert_eq!(prng1.next_f64(), prng2.next_f64());
        }
    }

    #[test]
    fn test_academic_dataset_deterministic_reproducibility() {
        let mut gen1 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
        let doc1 = gen1.generate_notebook(AcademicScale::Small1k);

        let mut gen2 = AcademicDatasetGenerator::with_seed(DEFAULT_ACADEMIC_SEED);
        let doc2 = gen2.generate_notebook(AcademicScale::Small1k);

        assert_eq!(doc1.pages.len(), doc2.pages.len());
        let total_items1: usize = doc1.pages.iter().map(|p| p.items.len()).sum();
        let total_items2: usize = doc2.pages.iter().map(|p| p.items.len()).sum();
        assert_eq!(total_items1, 1000);
        assert_eq!(total_items2, 1000);

        for p in 0..doc1.pages.len() {
            assert_eq!(doc1.pages[p].items.len(), doc2.pages[p].items.len());
            assert_eq!(
                doc1.pages[p].spatial_index.len(),
                doc2.pages[p].spatial_index.len()
            );
            for i in 0..doc1.pages[p].items.len() {
                let ItemKind::Stroke(s1) = &doc1.pages[p].items[i].kind;
                let ItemKind::Stroke(s2) = &doc2.pages[p].items[i].kind;
                assert_eq!(s1.id, s2.id);
                assert_eq!(s1.points.len(), s2.points.len());
                for pt in 0..s1.points.len() {
                    assert_eq!(s1.points[pt].x, s2.points[pt].x);
                    assert_eq!(s1.points[pt].y, s2.points[pt].y);
                    assert_eq!(s1.points[pt].pressure, s2.points[pt].pressure);
                }
            }
        }
    }

    #[test]
    fn test_academic_dataset_cache_warmth() {
        let mut gen = AcademicDatasetGenerator::new();
        let doc = gen.generate_notebook(AcademicScale::Small1k);

        for page in &doc.pages {
            assert_eq!(page.spatial_index.len(), page.items.len());
            for item in &page.items {
                let ItemKind::Stroke(s) = &item.kind;
                assert!(s.cached_bbox.is_some(), "cached_bbox must be pre-populated");
                if !s.is_dot && s.points.len() >= 2 {
                    assert!(
                        !s.cached_bezier.is_empty(),
                        "cached_bezier must be pre-populated"
                    );
                }
            }
        }
    }

    #[test]
    fn test_whiteboard_spatial_culling_ratio_10k() {
        let mut gen = AcademicDatasetGenerator::new();
        let doc_10k = gen.generate_whiteboard(AcademicScale::Medium10k);
        assert_eq!(doc_10k.pages.len(), 1);
        let page = &doc_10k.pages[0];
        assert_eq!(page.items.len(), 10_000);
        assert_eq!(page.spatial_index.len(), 10_000);

        // Viewport HD 1920x1080 zoom 100% centrato all'origine
        let visible = page
            .spatial_index
            .locate_in_envelope(-960.0, -540.0, 960.0, 540.0);
        let total = page.items.len();
        let culled_pct = ((total - visible.len()) as f64 / total as f64) * 100.0;

        assert!(
            culled_pct > 90.0,
            "Expected culled_pct > 90%, got {:.2}%",
            culled_pct
        );
    }
}
