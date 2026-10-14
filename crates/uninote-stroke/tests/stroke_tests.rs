use uninote_stroke::{
    catmull_rom_to_bezier, sample_catmull_rom_spline, DeviceKind, InputDisposition, InputPhase,
    InputSample, OneEuroConfig, OneEuroPointFilter, PalmRejectionEngine, PalmState, PipelineMode,
    RecordedSession, StrokeBuilder, StrokePoint, StrokeStyle, TouchMode,
};

#[test]
fn test_stroke_discrete_dot() {
    let style = StrokeStyle::default();
    let mut builder = StrokeBuilder::new(1, style, PipelineMode::Baseline);

    let sample_down = InputSample::new(
        (100.0, 100.0),
        Some(0.6),
        1_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    builder.process_sample(sample_down);

    let sample_up = InputSample::new(
        (100.5, 100.5),
        Some(0.6),
        2_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    builder.process_sample(sample_up);

    let stroke = builder.finish();
    assert!(
        stroke.is_dot,
        "Dovrebbe essere riconosciuto come punto isolato"
    );
    assert_eq!(stroke.points.len(), 1);
}

#[test]
fn test_stroke_duplicate_samples_dropped_unless_pressure_changes() {
    let style = StrokeStyle::default();
    let mut builder = StrokeBuilder::new(2, style, PipelineMode::Baseline);

    // Down
    builder.process_sample(InputSample::new(
        (50.0, 50.0),
        Some(0.5),
        100_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    ));

    // Identico campione: posizione e pressione uguali -> scartato
    let pt1 = builder.process_sample(InputSample::new(
        (50.0, 50.0),
        Some(0.5),
        200_000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));
    assert!(pt1.is_none());

    // Stessa posizione ma pressione cambiata -> accettato
    let pt2 = builder.process_sample(InputSample::new(
        (50.0, 50.0),
        Some(0.8),
        300_000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));
    assert!(pt2.is_some());

    let (received, dropped) = builder.samples_stats();
    assert_eq!(received, 3);
    assert_eq!(dropped, 1);
}

#[test]
fn test_one_euro_filter_robustness() {
    let mut filter = OneEuroPointFilter::new(OneEuroConfig::default());

    // Campione iniziale
    let (x0, y0) = filter.filter(10.0, 20.0, 1_000_000_000);
    assert_eq!((x0, y0), (10.0, 20.0));

    // Timestamp con delta negativo o zero non deve andare in crash o generare NaN
    let (x1, y1) = filter.filter(11.0, 21.0, 1_000_000_000);
    assert!(x1.is_finite());
    assert!(y1.is_finite());

    // Salto rapido: verifica reattività
    let (x2, y2) = filter.filter(100.0, 200.0, 2_000_000_000);
    assert!(x2 > 10.0 && x2 <= 100.0);
    assert!(y2 > 20.0 && y2 <= 200.0);
}

#[test]
fn test_palm_rejection_state_machine() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    // Inizialmente Idle: tocco ammesso per gesti
    let touch_down = InputSample {
        position: (200.0, 200.0),
        pressure: None,
        tilt: None,
        timestamp_ns: 1_000_000,
        device_kind: DeviceKind::Touch,
        source_id: 10,
        phase: InputPhase::Down,
    };
    assert_eq!(
        engine.evaluate_sample(&touch_down),
        InputDisposition::AcceptGesture
    );

    // Penna tocca il vetro: stato PenDrawing
    let pen_down = InputSample::new(
        (50.0, 50.0),
        Some(0.7),
        2_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    assert_eq!(
        engine.evaluate_sample(&pen_down),
        InputDisposition::AcceptDraw
    );
    assert_eq!(engine.current_state(), PalmState::PenDrawing);

    // Mentre la penna è attiva, qualsiasi contatto touch viene soppresso
    let palm_down = InputSample {
        position: (300.0, 400.0),
        pressure: None,
        tilt: None,
        timestamp_ns: 3_000_000,
        device_kind: DeviceKind::Touch,
        source_id: 20,
        phase: InputPhase::Down,
    };
    assert_eq!(
        engine.evaluate_sample(&palm_down),
        InputDisposition::Suppress
    );

    // Penna rilasciata: passa a PenCooldown
    let pen_up = InputSample::new(
        (60.0, 60.0),
        Some(0.1),
        4_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    assert_eq!(
        engine.evaluate_sample(&pen_up),
        InputDisposition::AcceptDraw
    );
    assert_eq!(engine.current_state(), PalmState::PenCooldown);

    // Lo slot del palmo (source_id 20) continua a essere soppresso anche se la penna si è alzata
    let palm_move = InputSample {
        position: (305.0, 405.0),
        pressure: None,
        tilt: None,
        timestamp_ns: 5_000_000,
        device_kind: DeviceKind::Touch,
        source_id: 20,
        phase: InputPhase::Move,
    };
    assert_eq!(
        engine.evaluate_sample(&palm_move),
        InputDisposition::Suppress
    );

    // Modalità TouchMode::Off: sopprime qualsiasi tocco
    engine.set_touch_mode(TouchMode::Off);
    let touch_off_sample = InputSample {
        position: (10.0, 10.0),
        pressure: None,
        tilt: None,
        timestamp_ns: 10_000_000_000,
        device_kind: DeviceKind::Touch,
        source_id: 30,
        phase: InputPhase::Down,
    };
    assert_eq!(
        engine.evaluate_sample(&touch_off_sample),
        InputDisposition::Suppress
    );
}

#[test]
fn test_recorder_and_replay_determinism() {
    let mut session = RecordedSession::new("test_session");

    let style = StrokeStyle::default();
    session.record_sample(InputSample::new(
        (10.0, 10.0),
        Some(0.5),
        1_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    ));
    session.record_sample(InputSample::new(
        (20.0, 20.0),
        Some(0.6),
        2_000_000,
        DeviceKind::Stylus,
        InputPhase::Move,
    ));
    session.record_sample(InputSample::new(
        (30.0, 30.0),
        Some(0.7),
        3_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    ));

    // Serializzazione e deserializzazione JSON
    let json = session.to_json().expect("Serializzazione JSON fallita");
    let deserialized = RecordedSession::from_json(&json).expect("Deserializzazione JSON fallita");
    assert_eq!(deserialized.samples.len(), 3);

    // Esecuzione replay
    let strokes_base = session.replay(PipelineMode::Baseline, style.clone());
    let strokes_filtered = session.replay(PipelineMode::OneEuroFiltered, style);

    assert_eq!(strokes_base.len(), 1);
    assert_eq!(strokes_filtered.len(), 1);
    assert_eq!(strokes_base[0].points.len(), 2);
    assert_eq!(strokes_filtered[0].points.len(), 2);

    // Nella baseline, la posizione finale del punto 1 coincide esattamente con il campione
    assert_eq!(strokes_base[0].points[1].x, 20.0);
    assert_eq!(strokes_base[0].points[1].y, 20.0);
}

#[test]
fn test_one_euro_filter_high_frequency_and_direction_reversal() {
    let mut filter = OneEuroPointFilter::new(OneEuroConfig::default());

    // Simula digitizer ad alta frequenza (240 Hz = ~4.16 ms per campione)
    let dt_ns = 4_166_666;
    let mut t_ns = 1_000_000_000;

    // Tratto verso destra ad alta velocità: da x=0 a x=100
    for i in 0..10 {
        t_ns += dt_ns;
        let raw_x = i as f64 * 10.0;
        let (fx, fy) = filter.filter(raw_x, 50.0, t_ns);
        assert!(fx.is_finite());
        assert!(fy.is_finite());
    }

    // Inversione improvvisa verso sinistra a spigolo acuto (come nella lettera 'V')
    for i in 1..=10 {
        t_ns += dt_ns;
        let raw_x = 100.0 - (i as f64 * 10.0);
        let (fx, fy) = filter.filter(raw_x, 50.0, t_ns);
        assert!(fx.is_finite());
        assert!(fy.is_finite());
        // Non deve esserci overshoot oltre 105.0
        assert!(fx <= 105.0);
    }
}

#[test]
fn test_palm_raw_touch_capture_suppression() {
    let mut engine = PalmRejectionEngine::new(TouchMode::Off);

    // In TouchMode::Off qualsiasi tocco a livello Capture viene soppresso immediatamente
    assert!(engine.should_suppress_raw_touch(true, 100, 1_000_000));
    assert!(engine.should_suppress_raw_touch(false, 100, 2_000_000));
    assert_eq!(engine.suppressed_touch_events, 2);

    // In TouchMode::GesturesOnly con penna in prossimità
    engine.set_touch_mode(TouchMode::GesturesOnly);
    engine.on_pen_proximity(true, 3_000_000);
    assert_eq!(engine.current_state(), PalmState::PenProximity);

    // Palmo che tocca il vetro: bloccato al 100%
    assert!(engine.should_suppress_raw_touch(true, 200, 4_000_000));
    assert!(engine.should_suppress_raw_touch(false, 200, 5_000_000));

    // Rilascio slot touch
    engine.on_touch_release(200);
}

#[test]
fn test_mouse_strictly_suppressed_when_pen_active() {
    let mut engine = PalmRejectionEngine::new(TouchMode::Off);

    // Con penna in prossimità, il mouse non deve poter iniziare a disegnare
    engine.on_pen_proximity(true, 1_000_000);
    let mouse_sample = InputSample::new(
        (50.0, 50.0),
        None,
        2_000_000,
        DeviceKind::Mouse,
        InputPhase::Down,
    );
    assert_eq!(
        engine.evaluate_sample(&mouse_sample),
        InputDisposition::Suppress
    );

    // Con penna che scrive, mouse soppresso
    let pen_down = InputSample::new(
        (10.0, 10.0),
        Some(0.8),
        3_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    assert_eq!(
        engine.evaluate_sample(&pen_down),
        InputDisposition::AcceptDraw
    );
    assert_eq!(
        engine.evaluate_sample(&mouse_sample),
        InputDisposition::Suppress
    );

    // Con penna in cooldown, mouse ancora soppresso
    let pen_up = InputSample::new(
        (12.0, 12.0),
        Some(0.1),
        4_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    assert_eq!(
        engine.evaluate_sample(&pen_up),
        InputDisposition::AcceptDraw
    );
    assert_eq!(engine.current_state(), PalmState::PenCooldown);
    assert_eq!(
        engine.evaluate_sample(&mouse_sample),
        InputDisposition::Suppress
    );
}

#[test]
fn test_catmull_rom_spline_math_and_continuity() {
    // 0 o 1 punto non generano segmenti
    assert!(catmull_rom_to_bezier(&[]).is_empty());

    let pt0 = StrokePoint {
        x: 0.0,
        y: 0.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1,
    };
    assert!(catmull_rom_to_bezier(std::slice::from_ref(&pt0)).is_empty());

    // 2 punti generano 1 segmento retto con punti di controllo a 1/3 e 2/3
    let pt1 = StrokePoint {
        x: 30.0,
        y: 0.0,
        pressure: 0.6,
        width: 2.5,
        timestamp_ns: 2,
    };
    let two_pts = catmull_rom_to_bezier(&[pt0.clone(), pt1.clone()]);
    assert_eq!(two_pts.len(), 1);
    assert!((two_pts[0].cp1.0 - 10.0).abs() < 1e-4);
    assert!((two_pts[0].cp2.0 - 20.0).abs() < 1e-4);

    // 4 punti che formano una curva
    let pt2 = StrokePoint {
        x: 40.0,
        y: 20.0,
        pressure: 0.7,
        width: 3.0,
        timestamp_ns: 3,
    };
    let pt3 = StrokePoint {
        x: 30.0,
        y: 40.0,
        pressure: 0.8,
        width: 3.5,
        timestamp_ns: 4,
    };

    let segments = catmull_rom_to_bezier(&[pt0, pt1, pt2, pt3]);
    assert_eq!(segments.len(), 3);

    // C0 continuity: p1 del segmento i coincide con p0 del segmento i+1
    for i in 0..(segments.len() - 1) {
        let seg_curr = &segments[i];
        let seg_next = &segments[i + 1];
        assert!((seg_curr.p1.0 - seg_next.p0.0).abs() < 1e-5);
        assert!((seg_curr.p1.1 - seg_next.p0.1).abs() < 1e-5);
    }

    // C1 continuity: vettore tangente uscente seg[i] collineare al vettore entrante seg[i+1]
    for i in 0..(segments.len() - 1) {
        let incoming_tan = (
            segments[i].p1.0 - segments[i].cp2.0,
            segments[i].p1.1 - segments[i].cp2.1,
        );
        let outgoing_tan = (
            segments[i + 1].cp1.0 - segments[i + 1].p0.0,
            segments[i + 1].cp1.1 - segments[i + 1].p0.1,
        );

        assert!((incoming_tan.0 - outgoing_tan.0).abs() < 1e-5);
        assert!((incoming_tan.1 - outgoing_tan.1).abs() < 1e-5);
    }
}

#[test]
fn test_catmull_rom_overshoot_control() {
    // Sequenza con brusco salto per innescare potenziale overshoot
    let points = vec![
        StrokePoint {
            x: 0.0,
            y: 0.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 1,
        },
        StrokePoint {
            x: 10.0,
            y: 0.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 2,
        },
        StrokePoint {
            x: 10.1,
            y: 50.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 3,
        },
        StrokePoint {
            x: 10.2,
            y: 50.1,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 4,
        },
    ];

    let segments = catmull_rom_to_bezier(&points);
    assert_eq!(segments.len(), 3);

    // Controllo che i bracci tangenti non superino chord_len / 3.0
    for seg in &segments {
        let chord = ((seg.p1.0 - seg.p0.0).powi(2) + (seg.p1.1 - seg.p0.1).powi(2)).sqrt();
        let max_allowed = chord / 3.0 + 1e-6;

        let handle1 = ((seg.cp1.0 - seg.p0.0).powi(2) + (seg.cp1.1 - seg.p0.1).powi(2)).sqrt();
        let handle2 = ((seg.p1.0 - seg.cp2.0).powi(2) + (seg.p1.1 - seg.cp2.1).powi(2)).sqrt();

        assert!(handle1 <= max_allowed, "Handle 1 {handle1} > {max_allowed}");
        assert!(handle2 <= max_allowed, "Handle 2 {handle2} > {max_allowed}");
    }
}

#[test]
fn test_sample_catmull_rom_spline_density_and_width() {
    let points = vec![
        StrokePoint {
            x: 0.0,
            y: 0.0,
            pressure: 0.2,
            width: 1.0,
            timestamp_ns: 1,
        },
        StrokePoint {
            x: 10.0,
            y: 10.0,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: 2,
        },
        StrokePoint {
            x: 30.0,
            y: 15.0,
            pressure: 0.8,
            width: 4.0,
            timestamp_ns: 3,
        },
    ];

    let sampled = sample_catmull_rom_spline(&points, 2.0);
    assert!(!sampled.is_empty());

    // Coordinate e larghezze finite
    for (x, y, w) in &sampled {
        assert!(x.is_finite());
        assert!(y.is_finite());
        assert!(*w >= 0.99 && *w <= 4.01);
    }

    // Distanza massima tra punti consecutivi campionati non deve superare significativamente max_step_px
    for w in sampled.windows(2) {
        let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        assert!(d <= 3.5, "Distanza campionata troppo ampia: {d}");
    }
}
