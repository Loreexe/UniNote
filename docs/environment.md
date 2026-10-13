# Ambiente di Sviluppo e Rilevazione Hardware

Data rilevazione: 14 ottobre 2026

## 1. Sistema Operativo e Sessione
- **Distribuzione**: CachyOS (base Arch Linux x86_64, ottimizzazioni kernel e pacchetti CachyOS).
- **Sessione**: Wayland nativo (`XDG_SESSION_TYPE=wayland`, `WAYLAND_DISPLAY=wayland-1`).
- **Compositor**: Niri 26.04 (`niri 26.04 (8ed0da4)`).

## 2. Display e Risoluzione
Rilevato tramite `niri msg outputs`:
- **Output**: `BOE 0x0A2A Unknown` (`eDP-1`, display integrato laptop).
- **Risoluzione fisica**: 1920x1200 @ 60.001 Hz (preferred mode, 16:10).
- **Dimensioni fisiche pannello**: 300 mm x 190 mm (~14 pollici).
- **Risoluzione logica**: 1536x960.
- **Fattore di scala (Scale)**: **1.25** (Fractional scaling Wayland attivo).
- **Refresh rate**: 60 Hz (VRR non supportato dal pannello integrato).
- **Implicazione frame budget**: $1000 / 60 \approx 16.67\text{ ms}$ per frame.

## 3. Dispositivi di Input Rilevati
Rilevati tramite `/proc/bus/input/devices`:
- **Touchscreen**: `ELAN3233:00 04F3:42AB` (I2C bus `AMDI0010:00`, handler `event7`, `mouse0`).
  - Tipo: touchscreen capacitivo multi-touch (coordinate assolute `ABS_MT`).
- **Stilo Attivo**: `ELAN3233:00 04F3:42AB Stylus` (I2C bus, handler `event11`, `mouse1`, `event12`).
  - Tipo: digitizer stilo attivo integrato nel display.
  - Eventi e capacità hardware esposte dal driver: coordinate assolute `ABS_X`, `ABS_Y`, pressione stilo `ABS_PRESSURE`, pulsanti contatto/stilo (`BTN_TOUCH`, `BTN_STYLUS`, `BTN_STYLUS2`).
- **Touchpad**: `DELL0CA9:00 27C6:0D53 Touchpad` (I2C bus `AMDI0010:01`, handler `event10`, `mouse3`).
- **Tavoletta Wacom esterna**: Non collegata al momento dell'ispezione. Sarà testata quando l'utente la collegherà via USB/Bluetooth.

## 4. Librerie e Toolchain di Sistema
- **GTK4**: 4.24.1 (headers C e pkg-config disponibili).
- **Libadwaita**: 1.10.0 (headers C e pkg-config disponibili).
- **SQLite3**: 3.53.4.
- **Compilatori nativi**: GCC (`/usr/bin/gcc`), Clang (`/usr/bin/clang`), Git (`/usr/bin/git`), cURL (`/usr/bin/curl`).
- **Toolchain Rust**: Rustc `1.99.0 (b940084d7 2026-09-28)`, Cargo `1.99.0` (pacchetto `cachyos/arch extra/rust 1:1.99.0-1`).

## 5. Limiti di Verifica e Vincoli Operativi
- L'agente AI in ambiente headless/non-interattivo non ha accesso al puntatore fisico, alla penna sul vetro né alla tavoletta Wacom.
- Nessuna misurazione della latenza fisica "penna-pixel" (end-to-end con fotocamera ad alta velocità) può essere dichiarata come effettuata.
- L'agente verifica:
  1. Compilazione e assenza di warning/errori clippy e type-check.
  2. Test automatici di geometria, interpolazione, filtri e replayer di eventi.
  3. Logica e transizioni della macchina a stati di palm rejection.
- La validazione dell'input stilo reale, della risposta alla pressione, della cancellazione del palmo sul vetro e del comportamento su Wacom è delegata al collaudo manuale dell'utente secondo la procedura fornita.
