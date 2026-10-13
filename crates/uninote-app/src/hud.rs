use gtk4::prelude::*;
use gtk4::{Box, Label, Orientation};
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use uninote_stroke::{PalmState, PipelineMode, TouchMode};

/// Dati diagnostici live per l'overlay HUD.
#[derive(Debug, Clone)]
pub struct DiagnosticMetrics {
    pub device_name: String,
    pub pipeline_mode: PipelineMode,
    pub touch_mode: TouchMode,
    pub palm_state: PalmState,
    pub live_pressure: Option<f64>,
    pub sample_rate_hz: f64,
    pub fps: f64,
    pub samples_received: u64,
    pub samples_dropped: u64,
    pub touch_suppressed: u64,
    pub render_time_us: u64,
    pub is_recording: bool,
}

impl Default for DiagnosticMetrics {
    fn default() -> Self {
        Self {
            device_name: "In attesa di input".to_string(),
            pipeline_mode: PipelineMode::OneEuroFiltered,
            touch_mode: TouchMode::Off,
            palm_state: PalmState::Idle,
            live_pressure: None,
            sample_rate_hz: 0.0,
            fps: 60.0,
            samples_received: 0,
            samples_dropped: 0,
            touch_suppressed: 0,
            render_time_us: 0,
            is_recording: false,
        }
    }
}

/// Widget Overlay HUD semi-trasparente per diagnostica in tempo reale.
pub struct DiagnosticHud {
    container: Box,
    label_status: Label,
    label_perf: Label,
    label_stats: Label,
    metrics: Rc<RefCell<DiagnosticMetrics>>,
    last_frame_instant: RefCell<Instant>,
}

impl DiagnosticHud {
    pub fn new() -> Self {
        let container = Box::builder()
            .orientation(Orientation::Vertical)
            .spacing(4)
            .margin_start(16)
            .margin_top(16)
            .halign(gtk4::Align::Start)
            .valign(gtk4::Align::Start)
            .build();

        // Stile visivo: card scura semi-trasparente e font monospazio compatto
        container.set_widget_name("diagnostic-hud");
        let provider = gtk4::CssProvider::new();
        provider.load_from_data(
            "#diagnostic-hud {
                background-color: rgba(20, 24, 30, 0.85);
                border: 1px solid rgba(255, 255, 255, 0.15);
                border-radius: 8px;
                padding: 10px 14px;
                color: #e0e6ed;
                font-family: monospace;
                font-size: 11px;
            }",
        );
        gtk4::style_context_add_provider_for_display(
            &gtk4::gdk::Display::default().expect("Display default non disponibile"),
            &provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );

        let label_status = Label::builder()
            .xalign(0.0)
            .label("Dispositivo: Inizializzazione...")
            .build();
        let label_perf = Label::builder()
            .xalign(0.0)
            .label("Frequenza: 0 Hz | FPS: 60.0")
            .build();
        let label_stats = Label::builder()
            .xalign(0.0)
            .label("Campioni: 0 rec / 0 drop")
            .build();

        container.append(&label_status);
        container.append(&label_perf);
        container.append(&label_stats);

        Self {
            container,
            label_status,
            label_perf,
            label_stats,
            metrics: Rc::new(RefCell::new(DiagnosticMetrics::default())),
            last_frame_instant: RefCell::new(Instant::now()),
        }
    }

    pub fn widget(&self) -> &Box {
        &self.container
    }

    pub fn update_metrics<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut DiagnosticMetrics) -> R,
    {
        let mut m = self.metrics.borrow_mut();
        f(&mut m)
    }

    pub fn update_view(&self) {
        let m = self.metrics.borrow();

        let pressure_str = match m.live_pressure {
            Some(p) => {
                let bars = (p * 10.0).round() as usize;
                let bar_str: String =
                    "█".repeat(bars.min(10)) + &"░".repeat(10usize.saturating_sub(bars));
                format!("[{}] {:.2}", bar_str, p)
            }
            None => "[N/D (Mouse/NoPres)]".to_string(),
        };

        let mode_str = match m.pipeline_mode {
            PipelineMode::Baseline => "Baseline (Zero lag)",
            PipelineMode::OneEuroFiltered => "One Euro (Filtrato)",
        };

        let touch_str = match m.touch_mode {
            TouchMode::Off => "Touch OFF",
            TouchMode::GesturesOnly => "Touch Gesti",
        };

        let palm_str = match m.palm_state {
            PalmState::Idle => "Idle",
            PalmState::PenDrawing => "Penna Scrive",
            PalmState::PenProximity => "Prossimità",
            PalmState::PenCooldown => "Cooldown",
        };

        let rec_str = if m.is_recording { " ● REC" } else { "" };

        self.label_status.set_label(&format!(
            "Sorgente: {} | {}\nModalità: {} | {}\nStato Palmo: {}{}",
            m.device_name, pressure_str, mode_str, touch_str, palm_str, rec_str
        ));

        self.label_perf.set_label(&format!(
            "Input Rate: {:.1} Hz | Frame Rate: {:.1} FPS | Render: {} µs",
            m.sample_rate_hz, m.fps, m.render_time_us
        ));

        self.label_stats.set_label(&format!(
            "Campioni: {} ric / {} scartati | Palmo soppressi: {}",
            m.samples_received, m.samples_dropped, m.touch_suppressed
        ));
    }

    pub fn record_frame_tick(&self) {
        let now = Instant::now();
        let mut last = self.last_frame_instant.borrow_mut();
        let dt = now.duration_since(*last).as_secs_f64();
        *last = now;

        if dt > 0.001 {
            let instant_fps = 1.0 / dt;
            let mut m = self.metrics.borrow_mut();
            m.fps = m.fps * 0.9 + instant_fps * 0.1; // Media mobile esponenziale dolce
        }
    }
}
