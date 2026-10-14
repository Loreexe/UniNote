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
    pub palm_engine: PalmRejectionEngine,
    pub pipeline_mode: PipelineMode,
    pub current_style: StrokeStyle,
    pub next_stroke_id: u64,
    pub recorder: Option<RecordedSession>,
    pub last_sample_time: Option<Instant>,
    pub mouse_drag_start: Option<(f64, f64)>,
}

impl CanvasState {
    pub fn new() -> Self {
        Self {
            completed_strokes: Vec::new(),
            active_builder: None,
            palm_engine: PalmRejectionEngine::new(TouchMode::Off),
            pipeline_mode: PipelineMode::OneEuroFiltered,
            current_style: StrokeStyle::default(),
            next_stroke_id: 1,
            recorder: None,
            last_sample_time: None,
            mouse_drag_start: None,
        }
    }

    pub fn clear(&mut self) {
        self.completed_strokes.clear();
        self.active_builder = None;
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
            for stroke in &state.completed_strokes {
                Self::draw_stroke(cr, stroke);
            }

            // 4. Disegno del tratto live in corso
            if let Some(ref builder) = state.active_builder {
                Self::draw_stroke(cr, &builder.stroke);
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

    fn draw_stroke(cr: &gtk4::cairo::Context, stroke: &Stroke) {
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

        // Disegno polilineare continuo con round join e cap
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

    /// Intercetta gli eventi hardware in fase di cattura (Capture Phase) PRIMA che raggiungano
    /// le gesture e i controller di drag, garantendo la soppressione radicale del tocco e del palmo.
    fn setup_capture_controller(&self) {
        let legacy = gtk4::EventControllerLegacy::new();
        legacy.set_propagation_phase(gtk4::PropagationPhase::Capture);
        let state_clone = self.state.clone();
        let hud_clone = self.hud.clone();
        let app_start = self.app_start;

        legacy.connect_event(move |_controller, event| {
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
                let suppress = st
                    .palm_engine
                    .should_suppress_raw_touch(is_down, slot, now_ns);
                let suppressed_count = st.palm_engine.suppressed_touch_events;
                let palm_state = st.palm_engine.current_state();
                drop(st);

                if suppress {
                    hud_clone.update_metrics(|m| {
                        m.touch_suppressed = suppressed_count;
                        m.palm_state = palm_state;
                    });
                    hud_clone.update_view();
                    return glib::Propagation::Stop;
                }
            }

            // Soppressione puntatore emulato se la penna è attiva/prossimità o se il touch è OFF
            if event.is_pointer_emulated() {
                let st = state_clone.borrow();
                if st.palm_engine.is_pen_active() || st.palm_engine.touch_mode() == TouchMode::Off {
                    return glib::Propagation::Stop;
                }
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
            if let Some(ref mut builder) = st.active_builder {
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

                // 1. Elabora tutti i campioni intermedi ad alta frequenza del backlog hardware
                if let Some(backlog) = gesture.backlog() {
                    for coord in backlog {
                        let flags = coord.flags();
                        if flags.contains(gdk::AxisFlags::X) && flags.contains(gdk::AxisFlags::Y) {
                            let axes = coord.axes();
                            let bx = axes[0];
                            let by = axes[1];
                            let bp = if flags.contains(gdk::AxisFlags::PRESSURE) {
                                Some(axes[2])
                            } else {
                                pressure
                            };
                            let b_sample = InputSample::new(
                                (bx, by),
                                bp,
                                now_ns,
                                DeviceKind::Stylus,
                                InputPhase::Move,
                            );
                            builder.process_sample(b_sample.clone());
                            if let Some(ref mut rec) = st.recorder {
                                rec.record_sample(b_sample);
                            }
                        }
                    }
                }

                // 2. Elabora il punto di motion corrente
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
        let hud = hud_clone.clone();
        stylus.connect_proximity(move |_gesture, _x, _y| {
            let now_ns = app_start.elapsed().as_nanos() as u64;
            let mut st = state.borrow_mut();
            st.palm_engine.on_pen_proximity(true, now_ns);
            let palm_state = st.palm_engine.current_state();
            drop(st);

            hud.update_metrics(|m| {
                m.palm_state = palm_state;
            });
            hud.update_view();
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
            // Se la penna è attiva (scrittura, prossimità o cooldown) o c'è già uno stroke in corso: NEGA
            if st.palm_engine.is_pen_active() || st.active_builder.is_some() {
                drop(st);
                gesture.set_state(gtk4::EventSequenceState::Denied);
                return;
            }

            let now_ns = app_start.elapsed().as_nanos() as u64;
            let sample =
                InputSample::new((x, y), None, now_ns, DeviceKind::Mouse, InputPhase::Down);

            if st.palm_engine.evaluate_sample(&sample) == InputDisposition::AcceptDraw {
                st.mouse_drag_start = Some((x, y));
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
