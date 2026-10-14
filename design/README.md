# UniNote — Prototipazione UI / UX in Stile Dank Material Shell (DMS)

Questo modulo contiene la riprogettazione completa dell'interfaccia grafica di **UniNote** ispirata a **Dank Material Shell (DMS)** e alle linee guida **Material Design 3 (M3) / Matugen**, perfettamente integrata con l'ambiente desktop Wayland moderno e il compositor **Niri**.

Il progetto è archiviato come file nativo **FigCraft** (`.figcraft`), navigabile e modificabile sia tramite FigCraft desktop che headless via `figcraft-cli`.

---

## 🎨 Filosofia di Design: DMS vs Desktop Tradizionale

A differenza dell'interfaccia desktop legacy con HeaderBar rigida ancorata in alto e pulsanti minuscoli, lo stile DMS adotta:

1. **Floating Inking Island (Barra a Isola Flottante Compatta)**:
   - Barra degli strumenti a pillola con raggio di curvatura generoso (**28px**), larghezza ottimizzata a **756px** e perfettamente centrata, staccata dai bordi con ombreggiatura morbida M3 Elevation.
   - Strumenti a chip pillola: **Penna** (attivo in ciano elettrico `#6DD2FF`), **Evidenziatore**, **Gomma**, **Lazo**.
   - Selettore spessore tratto pill **2.8 pt** e pulsanti circolari Undo / Redo / Quick Settings.

2. **Floating Pen Dock sul Bordo Sinistro (Slot Colori con Icone Penna)**:
   - Dock verticale a pillola galleggiante lungo il margine sinistro dello schermo (`cornerRadius: 27px`, `x: 22`, `y: 190`).
   - Ciascuno slot presenta **l'icona stilizzata della penna** colorata nel rispettivo inchiostro:
     - *Penna 1 (Attiva)*: Ciano Elettrico `#6DD2FF` con background container ad alto contrasto.
     - *Penna 2*: Bianco Caldo / Inchiostro Principale `#D9E4EB`.
     - *Penna 3*: Corallo / Allerta `#FFB4AB` (Matugen Destructive).
     - *Penna 4*: Verde Menta `#79D9A5`.
     - *Penna 5*: Oro Accademico `#FFD875`.
   - Pulsante `+` in basso per aprire la selezione personalizzata di nuovi colori.
   - Posizionamento ergonomico ideale per il tocco rapido con la mano sinistra durante la scrittura con stilo.

3. **Sincronizzazione Dinamica con il Tema di Sistema (Niri + DMS)**:
   - Dank Material Shell genera i token dinamici via Matugen (`scheme-vibrant`) in `~/.config/gtk-4.0/dank-colors.css` (e `gtk.css`).
   - UniNote (GTK4/Libadwaita) eredita nativamente queste variabili CSS (`@accent_bg_color`, `@window_bg_color`, `@card_bg_color`), aggiornando automaticamente l'interfaccia a ogni cambio di wallpaper o tema del desktop senza richiedere riavvii.

4. **Floating Bottom Dock (Dock di Navigazione Pagine)**:
   - Ispirato al dock flottante di DMS: navigatore pagine (`‹ Pagina 1 di 3 ›`), pulsante tonale `+ Pagina`, selettore sfondo e controlli zoom fluidi.

5. **HUD & Control Center Style Hardware Monitor**:
   - Scheda di monitoraggio floating su tela: frequenza stilo live `133.4 Hz` (protocollo MPP), framerate `60 FPS`, latenza di rendering vettoriale `412 µs`, progress bar spessa M3 della pressione e chip di stato Palm Rejection.

---

## 📁 Struttura dei File

```
design/
├── uninote.figcraft                   # File di progetto FigCraft nativo e persistente (Single Source of Truth)
├── uninote_ui_preview.figcraft        # Symlink a uninote.figcraft per retrocompatibilità
├── README.md                          # Documentazione dell'architettura UI e workflow
└── previews/                          # Esportazioni ad altissima definizione a 2x
    ├── 01 UniNote DMS _ Tema Scuro _Niri _ Wayland_.png
    ├── 02 UniNote DMS _ Tema Chiaro _Material You_.png
    ├── 03 UniNote DMS _ Pannello Hardware _ Quick Settings.png
    └── 04 UniNote DMS _ Design System _ Matugen Tokens.png
```

---

## 🖼️ Viste Progettate

1. **`01 UniNote DMS — Tema Scuro (Niri / Wayland)`**:
   - Vista operativa quotidiana notturna a basso affaticamento visivo.
   - Foglio A4 Dark Slate (`#151F25`) con quadrettatura metrica discreta e inchiostri fluorescenti ad alta leggibilità (Ciano, Oro e Menta).
2. **`02 UniNote DMS — Tema Chiaro (Material You)`**:
   - Variante diurna ariosa su sfondo morbido `#F4FAFF` e carta bianca calda.
3. **`03 UniNote DMS — Pannello Hardware & Quick Settings`**:
   - Pannello di controllo in stile DMS Quick Settings / Control Center per la calibrazione dello stilo MPP (soglia anti-hover 0.060 con slider spesso M3, curva di risposta lineare, pad di prova disegno dal vivo e modalità Palm Rejection).
4. **`04 UniNote DMS — Design System & Matugen Tokens`**:
   - Specifica dei token di colore, matrice degli stati dei pulsanti a pillola, gerarchia dei font e confronto con lo stile GTK tradizionale.

---

## 🚀 Workflow: Modifica In-Place ed Esportazione

Il file `design/uninote.figcraft` è il **vero e unico file di progetto FigCraft**: non viene rigenerato da zero, ma preservato e modificato in-place sia manualmente dall'utente che programmaticamente dall'agente.

1. **Apertura ed editing visivo nella GUI di FigCraft**:
   ```bash
   /home/cachylore/Applications/figcraft-0.5.1-linux-x86_64.AppImage design/uninote.figcraft
   ```
   Tutte le modifiche salvate dall'utente (`Ctrl+S`) rimangono persistenti all'interno del file.

2. **Modifiche programmatiche dell'agente**:
   Qualsiasi modifica richiesta viene applicata direttamente sui nodi esistenti del file tramite `figcraft-cli run --in design/uninote.figcraft --out design/uninote.figcraft` o aggiornamenti JSON puntuali, preservando i layer esistenti.

3. **Esportazione delle anteprime ad alta risoluzione (2x)**:
   ```bash
   figcraft-cli export design/uninote.figcraft -o design/previews/ --scale 2
   ```
