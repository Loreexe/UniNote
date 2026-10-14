use crate::hud::DiagnosticHud;
use gtk4::gdk;
use gtk4::prelude::*;
use gtk4::DrawingArea;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;
use uninote_stroke::{
    DeviceKind, InputDisposition, InputPhase, InputSample, PalmRejectionEngine, PipelineMode,
    RecordedSession, Stroke, StrokeBuilder, StrokeStyle, TouchMode,
};

/// Stato interno condiviso del canvas.
pub struct CanvasState {
    pub completed_strokes: Vec<Stroke>,
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
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            completed_strokes: Vec::new(),
            active_builder: None,
            active_device: None,
            palm_engine: PalmRejectionEngine::new(TouchMode::Off),
            pipeline_mode: PipelineMode::CatmullRomSpline,
            current_style: StrokeStyle::default(),
            next_stroke_id: 1,
            recorder: None,
            last_sample_time: None,
            last_stylus_time: None,
            mouse_drag_start: None,
        }
    }

    pub fn clear(&mut self) {
        self.completed_strokes.clear();
        self.active_builder = None;
        self.active_device = None;
        self.mouse_drag_start = None;
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
        canvas
    }

    pub fn widget(&self) -> &DrawingArea {
        &self.area
    }

    pub fn state(&self) -> Rc<RefCell<CanvasState>> {
        self.state.clone()
    }

    fn setup_drawing(&self) {
        let state_clone = self.state.clone();
        let hud_clone = self.hud.clone();

        self.area.set_draw_func(move |_area, cr, width, height| {
            let start_render = Instant::now();

            // 1. Sfondo foglio di scrittura (bianco caldo naturale)
            cr.set_source_rgb(0.98, 0.98, 0.99);
            let _ = cr.paint();

            // 2. Griglia leggera di riferimento millimetrico
            cr.set_source_rgba(0.85, 0.88, 0.92, 0.4);
            cr.set_line_width(0.7);
            let step = 32.0;
            let mut x = step;
            while x < width as f64 {
                cr.move_to(x, 0.0);
                cr.line_to(x, height as f64);
                x += step;
            }
            let mut y = step;
            while y < height as f64 {
                cr.move_to(0.0, y);
                cr.line_to(width as f64, y);
                y += step;
            }
            let _ = cr.stroke();

            // 3. Disegno dei tratti completati
            let state = state_clone.borrow();
            let current_mode = state.pipeline_mode;
            for stroke in &state.completed_strokes {
                Self::draw_stroke(cr, stroke, current_mode);
            }

            // 4. Disegno del tratto live in corso
            if let Some(ref builder) = state.active_builder {
                Self::draw_stroke(cr, &builder.stroke, current_mode);
            }

            // 5. Aggiornamento metriche di rendering
            let render_duration = start_render.elapsed().as_micros() as u64;
            hud_clone.record_frame_tick();
            hud_clone.update_metrics(|m| {
                m.render_time_us = render_duration;
            });
            hud_clone.update_view();
        });
    }

    fn draw_stroke(cr: &gtk4::cairo::Context, stroke: &Stroke, mode: PipelineMode) {
        if stroke.points.is_empty() {
            return;
        }

        let (r, g, b, a) = stroke.style.color;
        cr.set_source_rgba(r, g, b, a);
        cr.set_line_cap(gtk4::cairo::LineCap::Round);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);

        if stroke.is_dot || stroke.points.len() == 1 {
            let p = &stroke.points[0];
            let radius = (p.width / 2.0).max(1.5);
            cr.arc(p.x, p.y, radius, 0.0, 2.0 * std::f64::consts::PI);
            let _ = cr.fill();
            return;
        }

        match mode {
            PipelineMode::Baseline => {
                // Disegno polilineare diretto a segmenti retti (Baseline Off da specifica)
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

                    // Se la variazione di spessore è minima, usiamo curve_to nativo di Cairo
                    if width_diff < 0.25 || len <= 3.0 {
                        let avg_w = (seg.width0 + seg.width1) / 2.0;
                        cr.set_line_width(avg_w.max(1.0));
                        cr.move_to(seg.p0.0, seg.p0.1);
                        cr.curve_to(
                            seg.cp1.0, seg.cp1.1, seg.cp2.0, seg.cp2.1, seg.p1.0, seg.p1.1,
                        );
                        let _ = cr.stroke();
                    } else {
                        // Suddivisione in micro-passi lungo la Bézier per variazioni fluide di pressione
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
    }

    /// Intercetta gli eventi hardware in fase di cattura (Capture Phase) PRIMA che raggiungano
    /// le gesture e i controller di drag, garantendo la soppressione del tocco e del palmo sul foglio.
    fn setup_capture_controller(&self) {
        let legacy = gtk4::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let state_clone = self.state.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        legacy.connect_event(move |_controller, event| {
            // Eventi stilo/penna hardware: devono SEMPRE procedere al gesture controller dedicato
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
                if is_up {
                    st.palm_engine.on_touch_release(slot);
                }
                let _ = st
                    .palm_engine
                    .should_suppress_raw_touch(is_down, slot, now_ns);
                let suppressed_count = st.palm_engine.suppressed_touch_events;
                let palm_state = st.palm_engine.current_state();
                drop(st);

                // Aggiorna metriche HUD in modo asincrono/leggero (SENZA layout sincrono set_label che causava freeze)
                hud_clone.update_metrics(|m| {
                    m.touch_suppressed = suppressed_count;
                    m.palm_state = palm_state;
                });

                // IL TOCCO NON SCRIVE MAI SUL CANVAS DI DISEGNO: stop assoluto alla propagazione
                return glib::Propagation::Stop;
            }

            // Soppressione categorica dei puntatori emulati dal touch sul canvas di disegno
            if event.is_pointer_emulated() {
                return glib::Propagation::Stop;
            }

            glib::Propagation::Proceed
        });

        self.area.add_controller(legacy);
    }

    fn setup_stylus_controller(&self) {
        const TIP_LIFT_PRESSURE_THRESHOLD: f64 = 0.015;

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
            // Reclama la sequenza per inibire gesture concorrenti
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let sample = InputSample::new(
                (x, y),
                pressure,
                now_ns,
                DeviceKind::Stylus,
                InputPhase::Down,
            );

            let mut st = state.borrow_mut();
            st.last_stylus_time = Some(Instant::now());
            st.active_device = Some(DeviceKind::Stylus);

            // Sicurezza: se per qualsiasi motivo era rimasto un tratto pendente non chiuso, finalizzalo ora
            if let Some(prev_builder) = st.active_builder.take() {
                let stroke = prev_builder.finish();
                st.completed_strokes.push(stroke);
            }

            if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                let id = st.next_stroke_id;
                st.next_stroke_id += 1;
                let mut builder =
                    StrokeBuilder::new(id, st.current_style.clone(), st.pipeline_mode);
                builder.process_sample(sample.clone());
                st.active_builder = Some(builder);

                if let Some(ref mut rec) = st.recorder {
                    rec.record_sample(sample);
                }

                let palm_state = st.palm_engine.current_state();
                drop(st);

                // Aggiorna HUD
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
            // Reclama la sequenza
            gesture.set_state(gtk4::EventSequenceState::Claimed);
            let pressure = gesture.axis(gdk::AxisUse::Pressure);
            let now = Instant::now();
            let now_ns = app_start.elapsed().as_nanos() as u64;

            let mut st_guard = state.borrow_mut();
            let st = &mut *st_guard;
            st.last_stylus_time = Some(now);

            if let Some(ref mut builder) = st.active_builder {
                // 1. Rilevamento dello stato di contatto fisico della punta (Tip Contact Detection):
                // In GTK4 su Wayland/libinput, quando la penna tocca fisicamente il vetro, l'evento ha BUTTON1_MASK.
                // Nel momento in cui la punta stacca nel raggio di hover/prossimità, BUTTON1_MASK viene rilasciato
                // oppure l'asse di pressione diventa None o <= 0.015.
                let has_button1 = gesture.current_event().is_some_and(|ev| {
                    ev.modifier_state()
                        .contains(gdk::ModifierType::BUTTON1_MASK)
                });

                let p_val = pressure.unwrap_or(0.0);
                let is_lifted =
                    !has_button1 || pressure.is_none() || p_val <= TIP_LIFT_PRESSURE_THRESHOLD;

                if is_lifted {
                    // Punta staccata: finalizza all'istante il tratto.
                    // Questo elimina categoricamente ogni sbavatura/spike verso la stanghetta successiva (es. le 'E' e le 't').
                    if let Some(b) = st.active_builder.take() {
                        let stroke = b.finish();
                        st.completed_strokes.push(stroke);
                    }
                    st.active_device = None;
                    st.last_sample_time = None;
                    drop(st_guard);
                    area.queue_draw();
                    return;
                }

                // 2. Protezione dai salti aerei e scatti (Leap Protection):
                // Se tra l'ultimo punto e quello corrente la distanza supera 24px in un singolo campione
                // e la pressione è debole (< 0.10), si tratta di uno stacco rapido in volo non intercettato.
                if let Some(last_pt) = builder.stroke.points.last() {
                    let dist = ((last_pt.x - x).powi(2) + (last_pt.y - y).powi(2)).sqrt();
                    if dist > 24.0 && p_val < 0.10 {
                        if let Some(b) = st.active_builder.take() {
                            let stroke = b.finish();
                            st.completed_strokes.push(stroke);
                        }
                        st.active_device = None;
                        st.last_sample_time = None;
                        drop(st_guard);
                        area.queue_draw();
                        return;
                    }
                }

                // Calcolo sample rate istantaneo
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

                // Elabora il punto di motion corrente direttamente con coordinate native sub-pixel
                let sample = InputSample::new(
                    (x, y),
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

            let sample =
                InputSample::new((x, y), pressure, now_ns, DeviceKind::Stylus, InputPhase::Up);
            st.palm_engine.evaluate_sample(&sample);

            if let Some(mut builder) = st.active_builder.take() {
                builder.process_sample(sample.clone());
                let stroke = builder.finish();
                st.completed_strokes.push(stroke);

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

            // Se la penna entra in prossimità aerea (hover), qualsiasi tratto in corso è terminato
            let closed_stroke = if let Some(builder) = st.active_builder.take() {
                let stroke = builder.finish();
                st.completed_strokes.push(stroke);
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
        drag.set_touch_only(false);
        let state_clone = self.state.clone();
        let area_clone = self.area.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        // Drag Begin (Mouse Down)
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        let drag_weak = drag.downgrade();
        drag.connect_drag_begin(move |_gesture, x, y| {
            let gesture = match drag_weak.upgrade() {
                Some(g) => g,
                None => return,
            };

            // Rifiuto categorico di qualsiasi evento emulato, touch o originato da penna
            if let Some(event) = gesture.current_event() {
                if event.is_pointer_emulated() || event.device_tool().is_some() {
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                    return;
                }
                let event_type = event.event_type();
                if event_type == gdk::EventType::TouchBegin
                    || event_type == gdk::EventType::TouchUpdate
                    || event_type == gdk::EventType::TouchEnd
                    || event_type == gdk::EventType::TouchCancel
                {
                    gesture.set_state(gtk4::EventSequenceState::Denied);
                    return;
                }
                if let Some(device) = event.device() {
                    if device.source() != gdk::InputSource::Mouse {
                        gesture.set_state(gtk4::EventSequenceState::Denied);
                        return;
                    }
                }
            }

            let mut st = state.borrow_mut();

            // Se la penna è attiva, è in prossimità, è in cooldown, oppure è stata usata di recente (< 2000 ms):
            // NEGA categoricamente il disegno del mouse/touchpad per impedire che il palmo appoggiato sul trackpad
            // o sul bordo del laptop generi linee spurie o interferisca con la scrittura!
            let recent_stylus = st
                .last_stylus_time
                .is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(2000));

            if st.active_device == Some(DeviceKind::Stylus)
                || st.palm_engine.is_pen_active()
                || recent_stylus
                || st.active_builder.is_some()
            {
                drop(st);
                gesture.set_state(gtk4::EventSequenceState::Denied);
                return;
            }

            let now_ns = app_start.elapsed().as_nanos() as u64;
            let sample =
                InputSample::new((x, y), None, now_ns, DeviceKind::Mouse, InputPhase::Down);

            if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                st.mouse_drag_start = Some((x, y));
                st.active_device = Some(DeviceKind::Mouse);
                let id = st.next_stroke_id;
                st.next_stroke_id += 1;
                let mut builder =
                    StrokeBuilder::new(id, st.current_style.clone(), st.pipeline_mode);
                builder.process_sample(sample.clone());
                st.active_builder = Some(builder);

                if let Some(ref mut rec) = st.recorder {
                    rec.record_sample(sample);
                }

                let palm_state = st.palm_engine.current_state();
                drop(st);

                hud.update_metrics(|m| {
                    m.device_name = "Mouse / Trackpad".to_string();
                    m.live_pressure = None;
                    m.palm_state = palm_state;
                    m.samples_received += 1;
                });
            } else {
                gesture.set_state(gtk4::EventSequenceState::Denied);
            }

            area.queue_draw();
        });

        // Drag Update (Mouse Move)
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        drag.connect_drag_update(move |_gesture, offset_x, offset_y| {
            let mut st_guard = state.borrow_mut();
            let st = &mut *st_guard;

            // ISOLAMENTO RIGIDO: il mouse non può MAI toccare uno stroke se il dispositivo attivo non è il Mouse!
            if st.active_device != Some(DeviceKind::Mouse) {
                return;
            }

            if let Some((start_x, start_y)) = st.mouse_drag_start {
                let cur_x = start_x + offset_x;
                let cur_y = start_y + offset_y;
                let now_ns = app_start.elapsed().as_nanos() as u64;

                if let Some(ref mut builder) = st.active_builder {
                    let sample = InputSample::new(
                        (cur_x, cur_y),
                        None,
                        now_ns,
                        DeviceKind::Mouse,
                        InputPhase::Move,
                    );
                    builder.process_sample(sample.clone());

                    if let Some(ref mut rec) = st.recorder {
                        rec.record_sample(sample);
                    }

                    let (received, dropped) = builder.samples_stats();
                    drop(st_guard);

                    hud.update_metrics(|m| {
                        m.samples_received = received;
                        m.samples_dropped = dropped;
                    });
                }
            }

            area.queue_draw();
        });

        // Drag End (Mouse Up)
        let state = state_clone.clone();
        let area = area_clone.clone();
        let hud = hud_clone.clone();
        drag.connect_drag_end(move |_gesture, offset_x, offset_y| {
            let mut st = state.borrow_mut();

            if st.active_device != Some(DeviceKind::Mouse) {
                st.mouse_drag_start = None;
                return;
            }
            st.active_device = None;

            if let Some((start_x, start_y)) = st.mouse_drag_start.take() {
                let cur_x = start_x + offset_x;
                let cur_y = start_y + offset_y;
                let now_ns = app_start.elapsed().as_nanos() as u64;

                let sample = InputSample::new(
                    (cur_x, cur_y),
                    None,
                    now_ns,
                    DeviceKind::Mouse,
                    InputPhase::Up,
                );
                st.palm_engine.evaluate_sample(&sample);

                if let Some(mut builder) = st.active_builder.take() {
                    builder.process_sample(sample.clone());
                    let stroke = builder.finish();
                    st.completed_strokes.push(stroke);

                    if let Some(ref mut rec) = st.recorder {
                        rec.record_sample(sample);
                    }
                }

                let palm_state = st.palm_engine.current_state();
                drop(st);

                hud.update_metrics(|m| {
                    m.palm_state = palm_state;
                });
            }

            area.queue_draw();
        });

        self.area.add_controller(drag);
    }
}
