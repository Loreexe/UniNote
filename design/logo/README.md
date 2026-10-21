# UniNote — Logo ufficiale (revisione 2: U calligrafica)

Logo vettoriale puro del progetto **UniNote**, realizzato con **VectorCraft**.
Nessun raster importato: tutta la geometria è costruita con primitive vettoriali
(`path.create`, `shape.rectangle`, `shape.ellipse`, `text.create`) e verniciata con
`paint.setFill` / `paint.setStroke`.

Griglia: 1024x1024 px, document units = Pixels.

---

## Concept scelto

**Una U sola, scritta a penna** — un unico trucco continuo che entra in alto a
sinistra, scende, curva al fondo, risale in alto a destra con un leggero
superamento a fine tratto.

- **Tratto calligrafico vero**: la U non è un path con `stroke` uniforme ma un
  **path chiuso con fill** (il contorno della spazzola). Lo spessore varia
  lungo il trucco: massimo ~130 px in discesa, minimo ~40 px, con **capelli
  (taper) alle due estremità**. È questo contrasto che la fa leggere come
  scritta a mano e non come geometria.
- **Come è costruita**: una centerline a tratti cubici di Bezier viene
  campionata; in ogni campione la normale viene offsettata di ± spessore/2
  con un profilo di spessore variabile; i due lati vengono uniti in un anello
  chiuso e risampionati a 20 anchor, resi come spline Catmull-Rom → Bezier.
  Il risultato ha **20 anchor**: abbastanza pochi da essere editabile a mano,
  abbastanza molti da seguire la curva.
- **Una goccia d'inchiostro** in oro accademico (`Ink_Dot`), appena fuori
  dall'estremità superiore destra: il dettaglio del nib. È un accento isolato,
  non una curva.

### Vincoli rispettati

- **Niente arco, niente swash, niente smile, nessun elemento a "swoosh"**:
  sotto la U non c'è nessuna curva di alcun tipo.
- **Solo la U** nel mark: niente monogramma multi-lettera.
- **L'oro è solo un accento** (la goccia), mai una curva.
- Geometria 100% vettoriale.

---

## Cosa è stato respinto (e perché)

### (a) Monogramma "UN" geometrico con swash ad arco — RESPINTO (era il concept precedente)

Il monogramma "UN" disegnato a tratti di penna uniformi, con uno **swash in
oro che correva sotto le lettere**, è stato respinto: lo swash, con la sua
curva che avvolge sotto le lettere e finisce nella macchia d'inchiostro, si
legge **come l'Amazon smile**. È un segno che non dice "UniNote" ma dice
"e-commerce", e inoltre aggiunge un secondo colore e una seconda forma che
non regge la lettura a 16 px.

La lezione tenuta: **la U deve restare pulita, senza arc**. Nessun elemento
di sotto o di sopra la lettera.

### (b) Pagina / quaderno stilizzato con punta di penna e oncie — RESPINTO

È quasi la stessa immagine dell'icona hicolor attuale: il badge-documento con
penna non si distingue a 32 px e non porta il nome UniNote. Scartato perché non
risolve il problema di identità, lo duplica.

### (c) "N" calligrafica da un unico tresso di inchiostro — RESPINTO

Una singola N non dice "Uni" e ad artefatto piccolo si perde la lettura della
diagonale. Scartato.

---

## Varianti disponibili

Tre varianti della U, tutte costruite con lo stesso metodo e la stessa palette.
La **A** è il default (i file senza suffisso sono quelli della A).

| Suffisso | Variante | Descrizione |
|----------|----------|-------------|
| _(nessuno)_ | **A** | U semplice con taper. Fondo della U tondo ma morbido, discesa piena, salita che assottiglia. È la più neutra e la più "sicura" per un'icona. |
| `_B` | **B** | Fondo più tondo, ingresso della U leggermente inclinato a sinistra, contrasto calligrafico più forte fra discesa e salita. Più "manoscritta". |
| `_C` | **C** | U con **tresso in più**: l'uscita superiore destra si sposta più in fuori e più in alto. Più carattere, meno simmetrica. |

Tutte e tre sono **100% vettoriali** e **nominate** (layer e path), quindi
selezionabili e rinominabili a mano dentro VectorCraft.

---

## Struttura dei file VectorCraft

Ogni `.vectorcraft` contiene layer con nomi leggibili e un path per oggetto:

| Layer | Oggetto | Contenuto |
|-------|---------|-----------|
| `Badge_Background` | `Badge_Background` | rettangolo arrotondato 1024x1024, raggio 210 (solo nei prodotti con badge) |
| `U_BrushStroke` | `U_BrushStroke` | **path chiuso con fill** della U calligrafica (20 anchor) |
| `Ink_Dot` | `Ink_Dot` | ellisse 26x26, la goccia d'inchiostro in oro |
| `Wordmark` | `Wordmark_UniNote` | testo "UniNote", Noto Sans Bold 104 (solo nei lockup) |

Nessun `stroke` su nessun oggetto: la U è interamente un **fill**.

---

## Prodotti

| File | Contenuto |
|------|-----------|
| `uninote_mark[.svg|.vectorcraft]` | la U sola, fondo trasparente |
| `uninote_mark_1024.png` / `_512.png` | la U sola, fondo trasparente |
| `uninote_logo_dark[.svg|.vectorcraft]` | la U su badge arrotondato fondo scuro `#0C2230` |
| `uninote_logo_dark_outline.svg` | idem, testo convertito in path (per compatibilità col set precedente) |
| `uninote_logo_light[.svg|.vectorcraft]` | la U su badge arrotondato fondo chiaro `#F4FAFF` |
| `uninote_logo_light_outline.svg` | idem, testo convertito in path |
| `uninote_logo_lockup_dark[.svg|.vectorcraft]` | badge scuro + wordmark "UniNote" |
| `uninote_logo_lockup_light[.svg|.vectorcraft]` | badge chiaro + wordmark "UniNote" |
| `uninote_logo_lockup_*_outline.svg` | stesso lockup con il testo **convertito in path** (nessun font richiesto) |
| `*_1024.png` / `*_512.png` | render raster di ogni prodotto |

Le varianti B e C hanno lo stesso set di file con il suffisso `_B` / `_C`.

### Formati

- `.vectorcraft` — **file nativo, deliverable principale**: apribile e
  modificabile a mano in VectorCraft.
- `.svg` — vettoriale scalare (con `data-name` sui path, così i nomi dei layer
  sopravvivono all'export).
- `.png` — 1024x1024 e 512x512, fondo trasparente per il mark, fondo pieno
  per i prodotti con badge.

---

## Palette (dal repo, DMS)

| Ruolo | Hex | Uso |
|-------|-----|-----|
| Ciano elettrico | `#6DD2FF` | la U sui fondi scuri |
| Oro accademico | `#FFD875` | la goccia d'inchiostro (solo accento) |
| Inchiostro | `#D9E4EB` | il wordmark sui fondi scuri |
| Fondo scuro | `#0C2230` | badge e versione dark |
| Fondo chiaro | `#F4FAFF` | badge e versione light |
| Inchiostro su chiaro | `#151F25` | la U e il wordmark sulla versione light |
| Oro su chiaro | `#C28A1E` | la goccia sulla versione light (oro scurito per contrasto) |

---

## Uso previsto

- **`uninote_mark_*`**: favicon, icona dell'app, avatar, anteprima del
  repository. Fondo trasparente, si appoggia su qualsiasi superficie.
- **`uninote_logo_dark_*` / `uninote_logo_light_*`**: badge quadrato con angoli
  morbidi (raggio 210/1024 ≈ 20%), da usare dove serve un quadrato pieno.
- **`uninote_logo_lockup_*`**: intestazioni, splash, materiale stampato. Il
  lockup scuro va su fondi scuri, il chiaro su fondi chiari. La variante
  `_outline` va preferita quando il file viene passato a terzi (nessun font
  richiesto).

---

## Note tecniche

- I file sono stati costruiti con `vectorcraft-cli run` in modalità headless,
  a partire da una centerline calcolata da script (`gen_assets.py`).
- Verifica: `vectorcraft-cli info <file>` su ogni SVG/`.vectorcraft` →
  `"warnings": []` e path presenti (controllato su tutti i 27 SVG e sui
  `.vectorcraft` riaperti in VectorCraft).
- Il wordmark usa **Noto Sans Bold**; per la versione `_outline` il testo è
  convertito in path e non richiede il font.
- Gli strumenti di verifica visiva (rendering ASCII del PNG) sono in
  `~/craft-workspace/uninote-logo/` (`png_ascii.py`, `ascii_poly.py`).
