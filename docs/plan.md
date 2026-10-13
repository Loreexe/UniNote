# Piano di Sviluppo — Fase 0: Prova di Fattibilità della Scrittura

## 1. Obiettivo della Fase 0
Costruire un prototipo avviabile e testabile di Uninote focalizzato esclusivamente sulla qualità del tratto, la gestione della pressione, la diagnostica e la protezione dal palmo.

## 2. Componenti da Implementare
1. **Workspace Cargo**:
   - `crates/uninote-stroke`: libreria geometrica, filtri, palm rejection, registrazione/replay input.
   - `crates/uninote-app`: finestra Libadwaita + GTK4, canvas di disegno, diagnostica a schermo.
2. **Pipeline del Tratto**:
   - Baseline lineare senza smoothing (zero latenza algoritmica).
   - One Euro Filter (filtro adattivo su posizione e filtro indipendente su pressione).
   - Generazione geometria continua con larghezza variabile in base alla pressione.
   - Punti isolati (punti su 'i', accenti, punti e virgola) gestiti come dischi circolari definiti.
3. **Palm Rejection**:
   - Macchina a stati per penna e touch.
   - Blocco immediato del touch all'avvicinarsi/contatto della penna.
   - Finestra di guardia (cooldown) configurabile post-scrittura.
   - Modalità "Touch Off" attivabile da toolbar.
4. **Diagnostica e Tracciamento**:
   - Overlay a schermo con: sorgente evento (Stylus/Mouse/Touch), campioni ricevuti/elaborati/scartati, stima sample rate (Hz), valore pressione live, stato palm rejection.
   - Registratore ed esecutore di campioni su file per test di regressione deterministici.
5. **Collaudo e Documentazione**:
   - Suite di test automatici (unit test).
   - Istruzioni di esecuzione per l'utente su CachyOS / Niri.
   - Matrice di test manuali per stilo attivo ELAN, touchscreen e tavoletta Wacom.

## 3. Criteri di Uscita della Fase 0
- [x] Ambiente e hardware ispezionati e documentati (`docs/environment.md`).
- [x] Decisioni architetturali documentate (`docs/decisions.md`).
- [ ] Codice compilabile senza warning né errori con `cargo check`, `cargo test`, `cargo clippy`.
- [ ] Nessuna duplicazione o conflitto tra stilo e mouse emulato.
- [ ] Possibilità di confrontare visivamente la baseline con il tratto filtrato.
- [ ] Prova manuale richiesta e approvata dall'utente prima di procedere alla Fase 1.

## 4. Attività Rimandate alle Fasi Successive
- **Fase 1**: Modello documento multipagina A4, scorrimento verticale, pan/zoom, undo/redo persistito, salvataggio SQLite asincrono con WAL e crash recovery.
- **Fase 2**: Evidenziatore, selezione/spostamento, annotazione sfondi PDF ed export PDF.
- **Fase 3 & 4**: Ottimizzazioni avanzate di rendering, forme geometriche, lavagna infinita.
