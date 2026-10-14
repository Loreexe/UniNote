use std::collections::HashSet;
use uninote_stroke::palm::{
    InputDisposition, PalmRejectionEngine, PalmState, TouchMode, PROXIMITY_TIMEOUT_NS,
};
use uninote_stroke::types::{DeviceKind, InputPhase, InputSample};

/// Simple deterministic Pseudo-Random Number Generator (Xorshift64) to ensure
/// reproducible stress tests without adding external dependencies.
struct SimpleRng {
    state: u64,
}

impl SimpleRng {
    fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 { 0xdeadbeefcafebabe } else { seed },
        }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn gen_range(&mut self, min: u64, max: u64) -> u64 {
        if min >= max {
            return min;
        }
        min + (self.next_u64() % (max - min))
    }

    fn shuffle<T>(&mut self, slice: &mut [T]) {
        let len = slice.len();
        for i in (1..len).rev() {
            let j = (self.next_u64() as usize) % (i + 1);
            slice.swap(i, j);
        }
    }
}

// =========================================================================
// TEST 1: Extreme Timestamp Deltas
// =========================================================================
#[test]
fn test_stress_extreme_timestamp_deltas() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    // 1.1 Multi-day uptime:
    // 1 day = 86_400 * 10^9 ns = 86_400_000_000_000 ns.
    // 30 days = 2_592_000_000_000_000 ns.
    let day_30_ns: u64 = 30 * 24 * 3600 * 1_000_000_000;
    engine.on_pen_proximity(true, day_30_ns);
    assert_eq!(engine.current_state(), PalmState::PenProximity);

    // Touch 150ms after proximity on day 30: must be suppressed
    assert!(
        engine.should_suppress_raw_touch(true, 1, day_30_ns + 150_000_000),
        "Touch at day 30 + 150ms should be suppressed"
    );
    engine.on_touch_release(1);

    // Touch at day 30 + 300ms + 1ns: must auto-expire cleanly to Idle
    let allowed = !engine.should_suppress_raw_touch(true, 2, day_30_ns + PROXIMITY_TIMEOUT_NS + 1);
    assert!(
        allowed,
        "Touch at day 30 + 300ms + 1ns should be allowed after auto-expiry"
    );
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(2);

    // 1.2 Extreme values near u64::MAX:
    let near_max_ns: u64 = u64::MAX - 2_000_000_000; // 2 seconds before u64::MAX
    engine.on_pen_proximity(true, near_max_ns);
    assert_eq!(engine.current_state(), PalmState::PenProximity);

    // 100ms later (still near u64::MAX): must be suppressed
    assert!(
        engine.should_suppress_raw_touch(true, 3, near_max_ns + 100_000_000),
        "Touch near u64::MAX within proximity window must be suppressed"
    );
    engine.on_touch_release(3);

    // Expiry near u64::MAX
    assert!(
        !engine.should_suppress_raw_touch(true, 4, near_max_ns + PROXIMITY_TIMEOUT_NS + 10),
        "Touch near u64::MAX after 300ms must auto-expire"
    );
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(4);

    // 1.3 Exact u64::MAX edge
    engine.on_pen_proximity(true, u64::MAX - 100);
    // At u64::MAX, elapsed is 100ns (well under 300ms)
    assert!(
        engine.should_suppress_raw_touch(true, 5, u64::MAX),
        "Touch at u64::MAX must be suppressed if proximity occurred 100ns prior"
    );
    engine.on_touch_release(5);

    // 1.4 Backwards / negative timestamp deltas (clock jump or out-of-order events):
    // If timestamp_ns < last_pen_time_ns, saturating_sub yields 0.
    // It should not panic or underflow.
    let t_ref = 500_000_000_000;
    engine.on_pen_proximity(true, t_ref);
    // Backwards event at t_ref - 100_000_000:
    let backwards_t = t_ref - 100_000_000;
    let supp = engine.should_suppress_raw_touch(true, 6, backwards_t);
    // 0 elapsed < 300ms, so it stays suppressed and does not panic
    assert!(supp);
    engine.on_touch_release(6);

    // 1.5 Jumping from near u64::MAX down to 0 (wrap-around simulation):
    engine.on_pen_proximity(true, u64::MAX - 50);
    // Next touch arrives with timestamp 1000 (after wrap)
    // 1000.saturating_sub(u64::MAX - 50) == 0 (no panic!)
    let _ = engine.should_suppress_raw_touch(true, 7, 1000);
    engine.on_touch_release(7);
}

// =========================================================================
// TEST 2: Rapid Fluttering between Proximity and PenDrawing
// =========================================================================
#[test]
fn test_stress_rapid_fluttering_proximity_pendrawing() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let mut rng = SimpleRng::new(0x12345678_abcdef01);

    let mut current_time_ns: u64 = 1_000_000_000;

    for i in 0..5_000 {
        // Step 1: Pen enters proximity
        current_time_ns += rng.gen_range(1, 10_000); // 1ns to 10µs
        engine.on_pen_proximity(true, current_time_ns);
        assert_eq!(engine.current_state(), PalmState::PenProximity);
        assert!(engine.is_pen_active());

        // An intermittent palm touch occurs during proximity
        if i % 5 == 0 {
            let slot = (i % 20) as u64 + 100;
            let supp = engine.should_suppress_raw_touch(true, slot, current_time_ns);
            assert!(supp, "Palm during proximity must be suppressed");
            engine.on_touch_release(slot);
        }

        // Step 2: Pen touches glass (PenDrawing)
        current_time_ns += rng.gen_range(1, 50_000);
        let pen_down = InputSample::new(
            (100.0, 100.0),
            Some(0.7),
            current_time_ns,
            DeviceKind::Stylus,
            InputPhase::Down,
        );
        let disp = engine.evaluate_sample(&pen_down);
        assert_eq!(disp, InputDisposition::AcceptDraw);
        assert_eq!(engine.current_state(), PalmState::PenDrawing);
        assert!(engine.is_pen_active());

        // Step 3: Rapid motion samples
        for _ in 0..3 {
            current_time_ns += rng.gen_range(1, 5_000);
            let pen_move = InputSample::new(
                (101.0, 101.0),
                Some(0.8),
                current_time_ns,
                DeviceKind::Stylus,
                InputPhase::Move,
            );
            assert_eq!(
                engine.evaluate_sample(&pen_move),
                InputDisposition::AcceptDraw
            );
            assert_eq!(engine.current_state(), PalmState::PenDrawing);
        }

        // Step 4: Pen lifts from glass (PenCooldown)
        current_time_ns += rng.gen_range(1, 10_000);
        let pen_up = InputSample::new(
            (102.0, 102.0),
            Some(0.0),
            current_time_ns,
            DeviceKind::Stylus,
            InputPhase::Up,
        );
        assert_eq!(
            engine.evaluate_sample(&pen_up),
            InputDisposition::AcceptDraw
        );
        assert_eq!(engine.current_state(), PalmState::PenCooldown);
        assert!(engine.is_pen_active());

        // Step 5: Stylus leaves proximity or immediately re-enters
        if i % 2 == 0 {
            current_time_ns += rng.gen_range(1, 10_000);
            engine.on_pen_proximity(true, current_time_ns);
            assert_eq!(engine.current_state(), PalmState::PenProximity);
        }
    }

    // After 5,000 rapid flutter cycles, advance time by 350ms
    current_time_ns += 350_000_000;
    // Next touch event must cleanly auto-expire whatever proximity/cooldown state was active
    let supp = engine.should_suppress_raw_touch(true, 999, current_time_ns);
    assert!(!supp, "Touch 350ms after flutter must be accepted");
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(999);
    assert_eq!(engine.active_touch_slots_count(), 0);
    assert_eq!(engine.suppressed_touch_slots_count(), 0);
}

// =========================================================================
// TEST 3: Rapid Sequence of 10+ Touch Contacts Landing & Lifting in Random Order
// =========================================================================
#[test]
fn test_stress_10_plus_touch_contacts_random_order() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let mut rng = SimpleRng::new(0x98765432_10fedcba);

    let num_contacts = 16;
    let base_time: u64 = 10_000_000_000;

    // Phase A: Land 16 touch slots in random order during Idle state
    let mut slot_ids: Vec<u64> = (1..=num_contacts).collect();
    rng.shuffle(&mut slot_ids);

    let mut down_time = base_time;
    for &slot in &slot_ids {
        down_time += rng.gen_range(100_000, 1_000_000); // 100µs to 1ms
        let supp = engine.should_suppress_raw_touch(true, slot, down_time);
        assert!(
            !supp,
            "Touch slot {slot} during Idle must NOT be suppressed"
        );
    }
    assert_eq!(engine.active_touch_slots_count(), num_contacts as usize);
    assert_eq!(engine.suppressed_touch_slots_count(), 0);

    // Phase B: While all 16 fingers are on the glass, Pen enters proximity and draws
    let pen_time = down_time + 5_000_000;
    engine.on_pen_proximity(true, pen_time);
    let pen_sample = InputSample::new(
        (50.0, 50.0),
        Some(0.9),
        pen_time + 1_000_000,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    engine.evaluate_sample(&pen_sample);
    assert_eq!(engine.current_state(), PalmState::PenDrawing);

    // Additional palm contacts (slots 20..=25) land while pen is drawing
    let palm_slots: Vec<u64> = (20..=25).collect();
    for &pslot in &palm_slots {
        let supp = engine.should_suppress_raw_touch(true, pslot, pen_time + 2_000_000);
        assert!(
            supp,
            "Palm contact {pslot} during pen drawing must be suppressed"
        );
    }
    assert_eq!(
        engine.active_touch_slots_count(),
        (num_contacts + palm_slots.len() as u64) as usize
    );
    assert_eq!(engine.suppressed_touch_slots_count(), palm_slots.len());

    // Phase C: Pen finishes drawing and lifts
    let pen_lift_time = pen_time + 50_000_000;
    let pen_up = InputSample::new(
        (60.0, 60.0),
        Some(0.0),
        pen_lift_time,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    engine.evaluate_sample(&pen_up);
    assert_eq!(engine.current_state(), PalmState::PenCooldown);

    // Cooldown expires (250ms later)
    let post_cooldown_time = pen_lift_time + 250_000_000;

    // Phase D: All contacts (both original 16 and 6 palm contacts) lift in completely shuffled order
    let mut all_slots = slot_ids.clone();
    all_slots.extend_from_slice(&palm_slots);
    rng.shuffle(&mut all_slots);

    let mut remaining = all_slots.len();
    for &slot in &all_slots {
        // Contact moves before releasing
        let _ = engine.should_suppress_raw_touch(false, slot, post_cooldown_time);
        // Contact lifts
        engine.on_touch_release(slot);
        remaining -= 1;

        assert_eq!(
            engine.active_touch_slots_count(),
            remaining,
            "Active touch slots must decrement on each release"
        );

        // Crucial invariant: suppressed_touch_slots count must always be <= active_touch_slots
        assert!(
            engine.suppressed_touch_slots_count() <= engine.active_touch_slots_count(),
            "Suppressed touch slots ({}) must never exceed active slots ({})",
            engine.suppressed_touch_slots_count(),
            engine.active_touch_slots_count()
        );
    }

    // Invariant: Once all contacts are released, BOTH sets must be exactly 0
    assert_eq!(engine.active_touch_slots_count(), 0);
    assert_eq!(engine.suppressed_touch_slots_count(), 0);
}

// =========================================================================
// TEST 4: Under No Circumstance Can suppressed_touch_slots Remain Dirty
// =========================================================================
#[test]
fn test_stress_suppressed_touch_slots_never_dirty_after_all_released() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let mut rng = SimpleRng::new(0x42424242_1337c0de);

    let mut sim_time: u64 = 5_000_000;

    for round in 0..1_000 {
        let pen_active_initially = (rng.next_u64() & 1) == 0;
        if pen_active_initially {
            sim_time += 1_000_000;
            engine.on_pen_proximity(true, sim_time);
        }

        let contact_count = rng.gen_range(1, 20) as usize;
        let mut active_set = HashSet::new();

        // Staggered landings
        for _ in 0..contact_count {
            sim_time += rng.gen_range(10_000, 500_000);
            let slot = rng.gen_range(1, 500);
            let is_down = true;
            let _ = engine.should_suppress_raw_touch(is_down, slot, sim_time);
            active_set.insert(slot);

            // Invariant: suppressed <= active
            assert!(
                engine.suppressed_touch_slots_count() <= engine.active_touch_slots_count(),
                "Round {round}: suppressed ({}) > active ({})",
                engine.suppressed_touch_slots_count(),
                engine.active_touch_slots_count()
            );
        }

        // Mid-sequence pen transition
        sim_time += 2_000_000;
        let pen_transition = rng.gen_range(0, 4);
        match pen_transition {
            0 => {
                let s = InputSample::new(
                    (10.0, 10.0),
                    Some(0.5),
                    sim_time,
                    DeviceKind::Stylus,
                    InputPhase::Down,
                );
                engine.evaluate_sample(&s);
            }
            1 => {
                let s = InputSample::new(
                    (10.0, 10.0),
                    Some(0.0),
                    sim_time,
                    DeviceKind::Stylus,
                    InputPhase::Up,
                );
                engine.evaluate_sample(&s);
            }
            2 => {
                engine.on_pen_proximity(true, sim_time);
            }
            _ => {
                engine.on_pen_proximity(false, sim_time);
            }
        }

        // Staggered releases in random order
        let mut slots_to_release: Vec<u64> = active_set.into_iter().collect();
        rng.shuffle(&mut slots_to_release);

        for slot in slots_to_release {
            engine.on_touch_release(slot);
            assert!(
                engine.suppressed_touch_slots_count() <= engine.active_touch_slots_count(),
                "Round {round}: invariant violated during release"
            );
        }

        // Also simulate an errant release for an unknown slot (does it corrupt anything?)
        engine.on_touch_release(99_999);

        // Core verification: When all active touch slots are released, suppressed slots MUST be 0
        assert_eq!(
            engine.active_touch_slots_count(),
            0,
            "Round {round}: active slots must be 0"
        );
        assert_eq!(
            engine.suppressed_touch_slots_count(),
            0,
            "Round {round}: suppressed slots MUST NOT remain dirty when active slots is 0!"
        );
    }
}

// =========================================================================
// TEST 5: Proximity Expires Cleanly after 300ms without Latched States
// =========================================================================
#[test]
fn test_stress_proximity_auto_expiry_boundary_and_latching() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    let t0 = 10_000_000_000; // 10s
    engine.on_pen_proximity(true, t0);
    assert_eq!(engine.current_state(), PalmState::PenProximity);

    // 1. Exactly 1 nanosecond BEFORE expiry: (300ms - 1ns = 299_999_999 ns)
    let t_pre = t0 + PROXIMITY_TIMEOUT_NS - 1;
    let supp_pre = engine.should_suppress_raw_touch(true, 1, t_pre);
    assert!(supp_pre, "Touch at 299.999999ms must STILL be suppressed");
    assert_eq!(engine.current_state(), PalmState::PenProximity);
    engine.on_touch_release(1);

    // 2. Exactly AT 300ms (boundary condition):
    let t_exact = t0 + PROXIMITY_TIMEOUT_NS;
    let supp_exact = engine.should_suppress_raw_touch(true, 2, t_exact);
    assert!(
        !supp_exact,
        "Touch at exactly 300.000000ms must be ALLOWED and auto-expire proximity"
    );
    assert_eq!(
        engine.current_state(),
        PalmState::Idle,
        "State must cleanly transition to Idle at 300ms"
    );
    engine.on_touch_release(2);

    // 3. Subsequent touches hours later without any pen events:
    let t_hours_later = t0 + 3600 * 1_000_000_000;
    assert!(
        !engine.should_suppress_raw_touch(true, 3, t_hours_later),
        "Touch 1 hour later must be accepted immediately"
    );
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(3);

    // 4. Test that on_pen_proximity(false) AFTER auto-expiry does NOT re-latch cooldown:
    engine.on_pen_proximity(true, 20_000_000_000);
    // Auto-expire via touch at +300ms
    assert!(!engine.should_suppress_raw_touch(true, 4, 20_000_000_000 + PROXIMITY_TIMEOUT_NS));
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(4);

    // Hardware now sends exit signal on_pen_proximity(false) 100ms AFTER auto-expiry
    engine.on_pen_proximity(false, 20_000_000_000 + PROXIMITY_TIMEOUT_NS + 100_000_000);
    // Because state was Idle, it MUST NOT transition to PenCooldown!
    assert_eq!(
        engine.current_state(),
        PalmState::Idle,
        "Exiting proximity after auto-expiry must remain Idle, not enter Cooldown!"
    );

    // 5. Verify evaluate_sample with DeviceKind::Touch respects 300ms boundary
    engine.on_pen_proximity(true, 30_000_000_000);
    let sample_pre = InputSample::new(
        (10.0, 10.0),
        None,
        30_000_000_000 + PROXIMITY_TIMEOUT_NS - 1,
        DeviceKind::Touch,
        InputPhase::Down,
    );
    assert_eq!(
        engine.evaluate_sample(&sample_pre),
        InputDisposition::Suppress
    );
    assert_eq!(engine.current_state(), PalmState::PenProximity);
    engine.on_touch_release(sample_pre.source_id);

    let sample_post = InputSample::new(
        (10.0, 10.0),
        None,
        30_000_000_000 + PROXIMITY_TIMEOUT_NS,
        DeviceKind::Touch,
        InputPhase::Down,
    );
    assert_eq!(
        engine.evaluate_sample(&sample_post),
        InputDisposition::AcceptGesture
    );
    assert_eq!(engine.current_state(), PalmState::Idle);
    engine.on_touch_release(sample_post.source_id);
}

// =========================================================================
// TEST 6: Mouse Sample Expiry Bug Investigation
// =========================================================================
#[test]
fn test_mouse_evaluation_under_proximity_and_cooldown_expiry() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    let t0 = 10_000_000_000;
    engine.on_pen_proximity(true, t0);
    assert_eq!(engine.current_state(), PalmState::PenProximity);

    // Mouse sample arriving 5 seconds after pen proximity without any prior touch events:
    let mouse_sample = InputSample::new(
        (100.0, 100.0),
        None,
        t0 + 5_000_000_000, // 5 seconds later (> 300ms)
        DeviceKind::Mouse,
        InputPhase::Down,
    );

    let disp = engine.evaluate_sample(&mouse_sample);
    assert_eq!(
        disp,
        InputDisposition::AcceptDraw,
        "Mouse drawing must be accepted once proximity timeout has expired"
    );
    assert_eq!(engine.current_state(), PalmState::Idle);

    // Case B: PenCooldown expiry also not checked for Mouse
    let mut engine2 = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    let pen_up = InputSample::new(
        (50.0, 50.0),
        Some(0.0),
        t0,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    engine2.evaluate_sample(&pen_up);
    assert_eq!(engine2.current_state(), PalmState::PenCooldown);

    let mouse_sample_after_cooldown = InputSample::new(
        (100.0, 100.0),
        None,
        t0 + 1_000_000_000, // 1 second later (> 200ms cooldown)
        DeviceKind::Mouse,
        InputPhase::Down,
    );
    let disp2 = engine2.evaluate_sample(&mouse_sample_after_cooldown);
    assert_eq!(
        disp2,
        InputDisposition::AcceptDraw,
        "Mouse drawing must be accepted once pen cooldown has expired"
    );
    assert_eq!(engine2.current_state(), PalmState::Idle);
}

// =========================================================================
// TEST 7: TouchEnd Ordering in Canvas Controller
// =========================================================================
#[test]
fn test_touch_release_suppression_ordering_in_canvas() {
    let mut engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);

    let t0 = 1_000_000_000;
    // Pen is drawing
    let pen_sample = InputSample::new(
        (10.0, 10.0),
        Some(0.8),
        t0,
        DeviceKind::Stylus,
        InputPhase::Down,
    );
    engine.evaluate_sample(&pen_sample);

    // Palm touches down (slot 42) -> suppressed
    let supp_down = engine.should_suppress_raw_touch(true, 42, t0 + 1_000_000);
    assert!(supp_down);

    // Pen lifts and cooldown expires
    let pen_up = InputSample::new(
        (10.0, 10.0),
        Some(0.0),
        t0 + 10_000_000,
        DeviceKind::Stylus,
        InputPhase::Up,
    );
    engine.evaluate_sample(&pen_up);
    let t_lift = t0 + 10_000_000 + 300_000_000; // after cooldown

    // If canvas calls on_touch_release BEFORE should_suppress_raw_touch:
    let mut buggy_engine = PalmRejectionEngine::new(TouchMode::GesturesOnly);
    buggy_engine.evaluate_sample(&pen_sample);
    buggy_engine.should_suppress_raw_touch(true, 42, t0 + 1_000_000);
    buggy_engine.evaluate_sample(&pen_up);

    // BUGGY ORDER in canvas.rs:
    buggy_engine.on_touch_release(42);
    let supp_end_buggy = buggy_engine.should_suppress_raw_touch(false, 42, t_lift);
    // Because on_touch_release cleared slot 42, should_suppress returns FALSE (leaked to GTK!)
    assert!(
        !supp_end_buggy,
        "Calling on_touch_release before should_suppress_raw_touch leaks TouchEnd to downstream gestures"
    );

    // CORRECT ORDER:
    let supp_end_correct = engine.should_suppress_raw_touch(false, 42, t_lift);
    engine.on_touch_release(42);
    assert!(
        supp_end_correct,
        "Calling should_suppress_raw_touch before on_touch_release correctly suppresses TouchEnd"
    );
}
