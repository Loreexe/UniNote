use uninote_stroke::{
    DeviceKind, InputDisposition, InputPhase, InputSample, OneEuroConfig, OneEuroPointFilter,
    PalmRejectionEngine, PalmState, PipelineMode, RecordedSession, StrokeBuilder, StrokeStyle,
    TouchMode,
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
