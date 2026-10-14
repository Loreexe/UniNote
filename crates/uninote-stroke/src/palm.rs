use crate::types::{DeviceKind, InputPhase, InputSample};
use std::collections::HashSet;

/// Modalità di gestione del touchscreen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchMode {
    /// Qualsiasi input touch viene ignorato a livello di canvas (nessun disegno, nessun gesto).
    Off,
    /// Il tocco è riservato esclusivamente ai gesti a due dita (pan/zoom), mai al disegno.
    GesturesOnly,
}

/// Stato corrente della macchina a stati di protezione dal palmo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PalmState {
    /// Nessun contatto attivo.
    Idle,
    /// La penna è nel raggio di prossimità del digitizer ma non tocca il vetro.
    PenProximity,
    /// La penna è fisicamente in contatto e sta tracciando.
    PenDrawing,
    /// La penna si è appena sollevata; periodo di guardia attivo per evitare rimbalzi del palmo.
    PenCooldown,
}

/// Decisione presa dalla macchina a stati per un determinato campione di input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputDisposition {
    /// Il campione è accettato per il disegno (penna o mouse consentito).
    AcceptDraw,
    /// Il campione è accettato per i gesti (solo touch valido in modalità gesti).
    AcceptGesture,
    /// Il campione è rigettato e soppresso (palmo o tocco non consentito).
    Suppress,
}

/// Motore di rifiuto del palmo con macchina a stati esplicita e tracciamento slot touch.
pub struct PalmRejectionEngine {
    state: PalmState,
    touch_mode: TouchMode,
    cooldown_ns: u64,
    last_pen_time_ns: u64,
    /// Slot touch che sono iniziati durante un periodo di soppressione e devono essere ignorati fino al rilascio fisico.
    suppressed_touch_slots: HashSet<u64>,
    // Statistiche diagnostiche
    pub total_pen_events: u64,
    pub total_touch_events: u64,
    pub suppressed_touch_events: u64,
    pub accepted_touch_events: u64,
}

impl PalmRejectionEngine {
    pub fn new(touch_mode: TouchMode) -> Self {
        Self {
            state: PalmState::Idle,
            touch_mode,
            cooldown_ns: 200_000_000, // 200 ms di finestra di guardia predefinita
            last_pen_time_ns: 0,
            suppressed_touch_slots: HashSet::new(),
            total_pen_events: 0,
            total_touch_events: 0,
            suppressed_touch_events: 0,
            accepted_touch_events: 0,
        }
    }
}

impl Default for PalmRejectionEngine {
    fn default() -> Self {
        Self::new(TouchMode::GesturesOnly)
    }
}

impl PalmRejectionEngine {
    pub fn set_touch_mode(&mut self, mode: TouchMode) {
        self.touch_mode = mode;
        if mode == TouchMode::Off {
            self.suppressed_touch_slots.clear();
        }
    }

    pub fn touch_mode(&self) -> TouchMode {
        self.touch_mode
    }

    pub fn current_state(&self) -> PalmState {
        self.state
    }

    pub fn is_pen_active(&self) -> bool {
        matches!(
            self.state,
            PalmState::PenDrawing | PalmState::PenProximity | PalmState::PenCooldown
        )
    }

    /// Segnala l'ingresso o l'uscita della penna dal raggio di prossimità hardware.
    pub fn on_pen_proximity(&mut self, in_proximity: bool, timestamp_ns: u64) {
        if in_proximity {
            self.state = PalmState::PenProximity;
            self.last_pen_time_ns = timestamp_ns;
        } else if self.state == PalmState::PenProximity {
            self.state = PalmState::PenCooldown;
            self.last_pen_time_ns = timestamp_ns;
        }
    }

    /// Verifica rapida per decidere se un evento touch generico deve essere soppresso a livello Capture.
    pub fn should_suppress_raw_touch(
        &mut self,
        is_down: bool,
        slot_id: u64,
        timestamp_ns: u64,
    ) -> bool {
        self.total_touch_events += 1;

        if self.touch_mode == TouchMode::Off {
            self.suppressed_touch_events += 1;
            return true;
        }

        // Verifica scadenza cooldown
        if self.state == PalmState::PenCooldown
            && timestamp_ns.saturating_sub(self.last_pen_time_ns) >= self.cooldown_ns
        {
            self.state = PalmState::Idle;
        }

        if self.is_pen_active() {
            if is_down {
                self.suppressed_touch_slots.insert(slot_id);
            }
            self.suppressed_touch_events += 1;
            return true;
        }

        if self.suppressed_touch_slots.contains(&slot_id) {
            self.suppressed_touch_events += 1;
            return true;
        }

        false
    }

    pub fn on_touch_release(&mut self, slot_id: u64) {
        self.suppressed_touch_slots.remove(&slot_id);
    }

    /// Valuta un campione di input e determina se deve essere elaborato o soppresso.
    pub fn evaluate_sample(&mut self, sample: &InputSample) -> InputDisposition {
        match sample.device_kind {
            DeviceKind::Stylus => {
                self.total_pen_events += 1;
                self.last_pen_time_ns = sample.timestamp_ns;

                match sample.phase {
                    InputPhase::Down | InputPhase::Move => {
                        self.state = PalmState::PenDrawing;
                    }
                    InputPhase::Up | InputPhase::Cancel => {
                        self.state = PalmState::PenCooldown;
                    }
                }

                InputDisposition::AcceptDraw
            }

            DeviceKind::Mouse => {
                // Il mouse è consentito per il disegno SOLO se la penna non è attiva/prossimità/cooldown
                if self.is_pen_active() {
                    InputDisposition::Suppress
                } else {
                    InputDisposition::AcceptDraw
                }
            }

            DeviceKind::Touch => {
                self.total_touch_events += 1;

                // 1. In modalità TouchMode::Off tutto il touch è sempre bloccato
                if self.touch_mode == TouchMode::Off {
                    self.suppressed_touch_events += 1;
                    return InputDisposition::Suppress;
                }

                // 2. Verifica se siamo ancora nel periodo di cooldown post-penna
                if self.state == PalmState::PenCooldown {
                    if sample.timestamp_ns.saturating_sub(self.last_pen_time_ns) < self.cooldown_ns
                    {
                        // Cooldown attivo: sopprimi
                    } else {
                        // Cooldown scaduto: torna a Idle
                        self.state = PalmState::Idle;
                    }
                }

                // 3. Se la penna sta disegnando, è in prossimità o è in cooldown, sopprimi il tocco
                let pen_blocking = self.is_pen_active();

                if pen_blocking {
                    if sample.phase == InputPhase::Down {
                        // Registra questo slot touch per ignorarlo fino al rilascio fisico completo
                        self.suppressed_touch_slots.insert(sample.source_id);
                    }
                    self.suppressed_touch_events += 1;
                    return InputDisposition::Suppress;
                }

                // 4. Se lo slot touch è iniziato mentre la penna bloccava, continua a sopprimerlo
                if self.suppressed_touch_slots.contains(&sample.source_id) {
                    if sample.phase == InputPhase::Up || sample.phase == InputPhase::Cancel {
                        self.suppressed_touch_slots.remove(&sample.source_id);
                    }
                    self.suppressed_touch_events += 1;
                    return InputDisposition::Suppress;
                }

                // 5. In TouchMode::GesturesOnly, il tocco è ammesso SOLO come gesto, mai come disegno
                self.accepted_touch_events += 1;
                InputDisposition::AcceptGesture
            }

            DeviceKind::Unknown => InputDisposition::Suppress,
        }
    }
}
