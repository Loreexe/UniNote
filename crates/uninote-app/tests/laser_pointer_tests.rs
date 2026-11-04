use std::rc::Rc;
use uninote_app::canvas::{ActiveTool, LaserPoint, LaserTrail, NoteCanvas};
use uninote_app::hud::DiagnosticHud;

#[test]
fn test_laser_trail_lifespan_and_pruning() {
    let mut trail = LaserTrail {
        is_active: true,
        ..Default::default()
    };

    let now_ns: u64 = 2_000_000_000;
    const LIFESPAN_NS: u64 = 1_200_000_000;

    // Punto vecchio di 1.5 secondi (scaduto)
    trail.points.push(LaserPoint {
        x: 100.0,
        y: 100.0,
        timestamp_ns: now_ns - 1_500_000_000,
    });
    // Punto recente di 0.5 secondi (valido)
    trail.points.push(LaserPoint {
        x: 120.0,
        y: 110.0,
        timestamp_ns: now_ns - 500_000_000,
    });
    // Punto freschissimo (appena aggiunto)
    trail.points.push(LaserPoint {
        x: 130.0,
        y: 115.0,
        timestamp_ns: now_ns,
    });

    assert_eq!(trail.points.len(), 3);

    // Esegui la logica di pruning temporale del puntatore laser
    trail
        .points
        .retain(|p| now_ns.saturating_sub(p.timestamp_ns) < LIFESPAN_NS);

    assert_eq!(trail.points.len(), 2);
    assert_eq!(trail.points[0].x, 120.0);
    assert_eq!(trail.points[1].x, 130.0);

    // Avanza il tempo di ulteriori 1.5 secondi: tutti i punti devono scadere
    let future_ns = now_ns + 1_500_000_000;
    trail
        .points
        .retain(|p| future_ns.saturating_sub(p.timestamp_ns) < LIFESPAN_NS);
    assert!(trail.points.is_empty());
}

#[test]
fn test_laser_pointer_zero_persistence_and_cleanup_on_tool_change() {
    // Inizializza GTK se necessario per test headless
    let _ = gtk4::init();

    let hud = Rc::new(DiagnosticHud::new());
    let canvas = NoteCanvas::new(hud);
    canvas.set_active_tool(ActiveTool::LaserPointer);
    assert_eq!(canvas.active_tool(), ActiveTool::LaserPointer);

    // Simula punti nella scia
    {
        let state_rc = canvas.state();
        let mut st = state_rc.borrow_mut();
        st.laser_trail.is_active = true;
        st.laser_trail.points.push(LaserPoint {
            x: 50.0,
            y: 50.0,
            timestamp_ns: 100_000,
        });
    }

    let state_rc = canvas.state();
    assert_eq!(state_rc.borrow().laser_trail.points.len(), 1);
    assert!(state_rc.borrow().laser_trail.is_active);

    // Il documento non deve contenere alcun elemento permanente
    assert_eq!(state_rc.borrow().document.pages[0].items.len(), 0);

    // Cambio strumento a Penna: la scia deve essere azzerata immediatamente
    canvas.set_active_tool(ActiveTool::Pen);
    assert_eq!(canvas.active_tool(), ActiveTool::Pen);
    assert!(state_rc.borrow().laser_trail.points.is_empty());
    assert!(!state_rc.borrow().laser_trail.is_active);
    assert_eq!(state_rc.borrow().document.pages[0].items.len(), 0);

    // Verifica regolazione spessore laser e clamping
    assert_eq!(canvas.laser_width(), 5.0);

    canvas.set_laser_width(12.5);
    assert_eq!(canvas.laser_width(), 12.5);

    // Clamping min / max
    canvas.set_laser_width(0.5);
    assert_eq!(canvas.laser_width(), 1.5);

    canvas.set_laser_width(40.0);
    assert_eq!(canvas.laser_width(), 24.0);
}

#[test]
fn test_laser_preferences_serialization_and_defaults() {
    let mut prefs = uninote_app::preferences::AppPreferences::default();
    assert_eq!(prefs.laser_width, 5.0);
    prefs.laser_width = 8.5;

    let json = serde_json::to_string(&prefs).expect("serialize prefs");
    let deserialized: uninote_app::preferences::AppPreferences =
        serde_json::from_str(&json).expect("deserialize prefs");
    assert_eq!(deserialized.laser_width, 8.5);
}
