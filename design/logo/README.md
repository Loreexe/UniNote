# UniNote — Logo ufficiale

Logo vettoriale puro del progetto **UniNote**, realizzato con **VectorCraft** in modalità batch headless (`vectorcraft-cli run`). Nessun raster importato: tutta la geometria è costruita con primitive vettoriali (`shape.rectangle`, `shape.ellipse`, `path.create`) e verniciate con `paint.setFill` / `paint.setStroke` / `stroke.set`.

---

## Concept valutati

Sono state valutate tre direzioni di design, tutte su griglia 1024x1024 e ispirate al linguaggio DMS / Material 3 dell'app.

### (a) Monogramma "UN" costruito da tratti di penna — SCELTO

- **Idea**: la "U" e la "N" sono disegnate con un tratto continuo e uniforme (spessore 86 pt, cap e join arrotondati) come un tratto di penna a inchiostro calibrato. Sotto il monogramma corre uno **swash** calligrafico in oro accademico, con la macchia di inchiostro alla sua estremità destra: e la lancia della penna appoggiata sul foglio.
- **Pro**: mantiene l'idea guida dell'icona attuale (penna stilizzata + macchie di inchiostro) ma la rende un **monogramma**, cioe un segno riconoscibile e non una semplice illustrazione. Funziona a 16 px come a 1024 px perche la forma e data dal contrasto di tre sole masse. Coerente con l'estetica DMS (forme morbide, contrasto elevato, niente dettagli superflui).
- **Contro**: lo swash in oro aggiunge un secondo colore; va mantenuto identico in tutte le varianti.

### (b) Pagina / quaderno stilizzato con punta di penna e oncie

- **Idea**: un rettangolo di foglio inclinato ad angolo arrotondato, una penna che entra dall'alto a destra e due macchie di inchiostro sulla pagina.
- **Pro**: leggibilissimo, dice subito "annotazione".
- **Contro**: e quasi la stessa immagine dell'icona hicolor attuale; un badge-documento con penna non si distingue a 32 px e non porta il nome UniNote. Scartato perche non risolve il problema di identita, lo duplica.

### (c) "N" calligrafico da un unico tresso di inchiostro

- **Idea**: una sola curva continua, un tresso calligrafico che disegna la N e si chiude con un'onda.
- **Pro**: molto elegante e calligrafico in assoluto.
- **Contro**: una singola N non dice "Uni" e ad artefatto piccolo si perde la lettura della diagonale. Scartato.

---

## Concept scelto e motivazione

**Concept (a): monogramma "UN" a tratto di penna con swash calligrafico.**

È l'unico dei tre che:

1. **Codifica il nome** (UN -> UniNote) invece di illustrare solo il prodotto.
2. **Conserva l'identita esistente** (penna, inchiostro, macchia) senza copiarla: la penna della vecchia icona era un oggetto figurato, qui diventa **il modo di tracciare** il monogramma.
3. **Scala senza perdersi**: U + N + swash si riducono a tre masse leggibili anche in favicon.
4. **Rispetta la palette DMS** gia documentata nel repo: ciano elettrico per il monogramma (l'accento dell'app, il colore della penna attiva), oro accademico per lo swash (l'inchiostro secondario), fondo scuro `#0C2230` (superficie scura DMS) o chiaro `#F4FAFF` (versione chiara).

---

## Palette

| Ruolo | Colore | Hex | Uso |
|---|---|---|---|
| Monogramma (fondo scuro) | Ciano elettrico | `#6DD2FF` | tratti "U" e "N" |
| Monogramma (fondo chiaro) | Slate profondo | `#151F25` | tratti "U" e "N" |
| Swash / macchia (fondo scuro) | Oro accademico | `#FFD875` | curva calligrafica e goccia d'inchiostro |
| Swash / macchia (fondo chiaro) | Oro scurito | `#C28A1E` | curva calligrafica e goccia d'inchiostro |
| Wordmark (fondo scuro) | Inchiostro chiaro caldo | `#D9E4EB` | testo "UniNote" |
| Wordmark (fondo chiaro) | Slate profondo | `#151F25` | testo "UniNote" |
| Sfondo badge (scuro) | Superficie scura | `#0C2230` | badge arrotondato 210 pt |
| Sfondo badge (chiaro) | Versione chiara | `#F4FAFF` | badge arrotondato 210 pt |

---

## Formati prodotti

Tutti i file sono in `design/logo/`.

### SVG scalari (vettoriale puro)

| File | Contenuto | Note |
|---|---|---|
| `uninote_logo_dark.svg` | Logo completo su fondo scuro | 5 path + 1 oggetto di testo |
| `uninote_logo_light.svg` | Logo completo su fondo chiaro | 5 path + 1 oggetto di testo |
| `uninote_mark.svg` | Solo il simbolo (monogramma + swash), senza wordmark | 5 path, nessun font |
| `uninote_logo_dark_outline.svg` | Logo scuro con il testo convertito in tracciati | 15 path + 1 compound path, 0 elementi `<text>` |
| `uninote_logo_light_outline.svg` | Logo chiaro con il testo convertito in tracciati | idem |

### PNG (raster derivati dagli SVG)

| File | Dimensioni |
|---|---|
| `uninote_logo_dark_1024.png` | 1024 x 1024 |
| `uninote_logo_dark_512.png` | 512 x 512 |
| `uninote_logo_light_1024.png` | 1024 x 1024 |
| `uninote_logo_light_512.png` | 512 x 512 |
| `uninote_mark_1024.png` | 1024 x 1024 |
| `uninote_mark_512.png` | 512 x 512 |

### Sorgenti VectorCraft (riapribili e modificabili)

- `uninote_logo_dark.vectorcraft`
- `uninote_logo_light.vectorcraft`
- `uninote_mark.vectorcraft`

---

## Uso previsto

- **Icona dell'app / favicon**: usare `uninote_mark.svg` (oppure `uninote_mark_512.png` per le icone raster). Il monogramma da solo e l'unico elemento che resta leggibile sotto i 32 px.
- **Header e intestazioni**: `uninote_logo_dark.svg` su fondi scuri (DMS, tema notturno) e `uninote_logo_light.svg` su fondi chiari (Material You, tema diurno).
- **Social preview / header GitHub**: `uninote_logo_dark_1024.png` (o il corrispondente `_light_`).
- **Stampa e documentazione**: gli SVG con testo in contorni (`*_outline.svg`) evitano problemi di font mancante; il font del wordmark e **Noto Sans Bold**, comunemente disponibile.
- **Area di riserva minima**: 1/4 dell'altezza del badge (circa 64 px su 1024) attorno al monogramma.

---

## Note tecniche

- L'icona di sistema `data/icons/hicolor/` **non** e stata modificata: il nuovo logo convive con essa in `design/logo/` in attesa di una revisione separata del packaging.
- Il font del wordmark e dichiarato come "Noto Sans" con fallback `sans-serif`; su sistemi senza Noto Sans il renderer usa il fallback e il testo puo variare leggermente di metrica. Per ambienti deterministici usare le varianti `_outline.svg`.
- Il badge di sfondo ha raggio 210 pt (angoli molto morbidi, coerenti con DMS) e i tratti del monogramma usano `stroke-linecap: round` / `stroke-linejoin: round`, quindi nessun angolo vivo.
- I file sorgente `.vectorcraft` permettono di riaprire e modificare il logo nel motore VectorCraft senza ricostruirlo da zero.

- Ricostruzione completa con il solo motore VectorCraft (estratto, variante scura):

```bash
vectorcraft-cli run --cmd file.new --params '{"name":"UniNote Logo Dark","width":1024,"height":1024,"units":"Points","backgroundContents":"transparent"}' --cmd shape.rectangle --params '{"x":0,"y":0,"width":1024,"height":1024,"radius":210}' --cmd paint.setFill --params '{"color":"#0C2230"}' --cmd paint.setStroke --params '{"none":true}' --cmd path.create --params '{"d":"M 232 200 L 232 470 C 232 600 292 662 352 662 C 412 662 472 600 472 470 L 472 200"}' --cmd paint.setFill --params '{"none":true}' --cmd paint.setStroke --params '{"color":"#6DD2FF"}' --cmd stroke.set --params '{"weight":86,"cap":"round","join":"round"}' --export uninote_logo_dark.svg
```

- Verifica degli export:

```bash
vectorcraft-cli info design/logo/uninote_logo_dark.svg
vectorcraft-cli bench design/logo/uninote_logo_dark.svg --size 1024x1024 --iters 3
```
