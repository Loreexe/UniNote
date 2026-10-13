# UniNote

App desktop Linux per prendere appunti a mano, annotare dispense e disegnare con penna, touchscreen, mouse e tavoletta grafica Wacom.

## Stato del Progetto: Fase 0 (Prova di Fattibilità della Scrittura)

La **Fase 0** implementa il nucleo fondamentale della pipeline di input e rendering:
- Ricezione campioni hardware stilo con coordinate e pressione dinamica (`GtkGestureStylus`).
- Rendering baseline diretto (zero latenza) e variante con **One Euro Filter** (smussamento adattivo disaccoppiato su posizione e pressione).
- Supporto punti isolati e tratti puntiformi (punti sulle lettere, accenti, formule matematiche).
- Separazione stilo/mouse con prevenzione assoluta di tratti duplicati.
- Protezione dal palmo (**Palm Rejection**) tramite macchina a stati (`PalmState`), finestra di cooldown e soppressione degli slot capacitivi iniziati durante il contatto della penna.
- Modalità **Touch OFF** per inibire qualsiasi interazione del touchscreen sul foglio.
- Overlay diagnostico **HUD** a schermo con stima frequenza campioni (Hz), frame rate (FPS), tempo di render (µs) e monitoraggio della pressione.
- Registrazione e riproduzione deterministica di sessioni di campioni di input.

## Requisiti di Sistema
- **OS**: Linux (ottimizzato e testato su CachyOS / Arch Linux sotto Wayland / compositor Niri).
- **Librerie di sistema**: GTK 4 (`>= 4.12`), Libadwaita (`>= 1.5`).
- **Toolchain**: Rust stable (`>= 1.80.0`).

## Compilazione ed Esecuzione

```bash
# Verifica formattazione e tipi
make check
make fmt
make clippy

# Esecuzione della suite di test automatici
make test

# Avvio del prototipo
make run
```
oppure direttamente con Cargo:
```bash
cargo run -p uninote-app
```

## Struttura del Repository
- `crates/uninote-stroke`: Modulo puro in Rust per la geometria, i filtri One Euro, la macchina a stati di palm rejection e il recorder/replayer.
- `crates/uninote-app`: Applicazione GUI Libadwaita/GTK4 con canvas Cairo ad alta frequenza e HUD diagnostico.
- `docs/environment.md`: Analisi dell'hardware e dell'ambiente rilevato (display, digitizer ELAN, versioni librerie).
- `docs/decisions.md`: Registro delle decisioni architetturali (ADR).
- `docs/plan.md`: Obiettivi e criteri di completamento della Fase 0 e roadmap successiva.
