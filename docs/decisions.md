# Decisioni di Architettura e Progetto (ADR)

## ADR-001: Scelta dello Stack Tecnologico
- **Decisione**: Rust stable + GTK4 (tramite `gtk4-rs`) + Libadwaita (tramite `libadwaita-rs`).
- **Motivazione**: Piena rispondenza alla specifica di Uninote (Sezione 3). GTK4 e Libadwaita offrono supporto moderno e nativo a Wayland e Niri, inclusa l'API `GtkGestureStylus` con lettura della pressione stilo, campioni temporali intermedi (`GdkTimeCoord`) e gestione multi-touch. Rust garantisce assenza di data race nella gestione concorrente di input e rendering.
- **Alternative considerate**:
  - *Fork di Rnote*: Rnote ha una forte base di codice ma è un'applicazione complessa e monolitica con molte dipendenze. Realizzare Uninote come app indipendente consente di implementare una pipeline del tratto pulita, la macchina a stati di palm rejection richiesta e un formato SQLite atomico senza debiti tecnici pregressi.
  - *Python / PySide6*: Scartato esplicitamente dalla specifica (Sezione 3).
  - *wgpu / Vello*: Prematuro per la Fase 0; Cairo / GSK sono già integrati in GTK4 e garantiscono minima complessità e massima stabilità per la proof of concept.

## ADR-002: Modularità del Progetto
- **Decisione**: Organizzazione in Workspace Cargo con due crate per la Fase 0:
  1. `uninote-stroke`: Crate di libreria pura (zero dipendenze da GTK). Contiene:
     - Tipi di input (`InputSample`, `InputPhase`, `DeviceKind`).
     - Modello geometrico del tratto (`Stroke`, `StrokePoint`).
     - Algoritmi di smoothing: `BaselinePipeline` (lineare, zero lag) e `FilteredPipeline` (One Euro Filter disaccoppiato per posizione e pressione).
     - Macchina a stati esplicita per la protezione dal palmo (`PalmRejectionEngine`).
     - Sistema di registrazione e riproduzione input (serializzazione NDJSON/JSON) per test deterministici.
  2. `uninote-app`: Crate binario contenente l'interfaccia GTK4/Libadwaita:
     - Finestra principale con headerbar minimale.
     - Canvas interattivo (`DrawingArea`) con rendering immediato tramite Cairo.
     - Adattatore eventi per `GtkGestureStylus`, mouse e touch.
     - Overlay HUD di diagnostica locale.
- **Motivazione**: Separare la logica matematica e di input dal toolkit UI consente di eseguire il 100% dei test automatici su filtri, curve e stati di palmo in CI o ambienti headless senza richiedere un display server attivo.

## ADR-003: Pipeline Geometrica e Smoothing
- **Decisione**:
  - **Baseline**: Generazione polilineare con campioni diretti; larghezza del tratto modulata in base alla pressione (con larghezza costante di fallback per dispositivi privi di pressione).
  - **Filtro One Euro**: Implementazione canonica con parametri disaccoppiati:
    - Posizione: frequenza campionamento, `min_cutoff` e `beta` regolabili per filtrare il jitter a bassa velocità preservando la reattività nei movimenti rapidi.
    - Pressione: filtro One Euro indipendente sulla sola pressione per smussare le transizioni improvvise del pennino.
  - **Stabilità Live vs Finale**: La geometria del tratto live calcolata a ogni campione è deterministica e non ricalcola l'intero tratto pregresso con algoritmi differenti al rilascio del pennino (`stroke up`), prevenendo qualsiasi scatto visivo ("stroke morphing").

## ADR-004: Palm Rejection e Macchina a Stati Touch
- **Decisione**:
  - Regola ferrea P0: la penna scrive, il tocco NON scrive.
  - Stati: `Idle`, `PenInContact`, `PenProximity`, `TouchSuppressed`, `TouchActive`.
  - Politica di soppressione:
    - Se la penna entra in prossimità o tocca il vetro, ogni contatto touch attivo viene immediatamente terminato/soppresso.
    - Qualsiasi tocco che inizia mentre la penna è attiva (o entro una finestra temporale di guardia `cooldown_ms`) viene marcato come ignorato per l'intera durata del contatto fino al rilascio fisico del dito.
  - Modalità touch: commutabile dall'interfaccia tra `TouchOff` (disabilita completamente qualsiasi interazione touch sul canvas) e `TouchGesturesOnly` (predisposto per pan/zoom a due dita nelle fasi successive).
