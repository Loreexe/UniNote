use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, OnceLock};
use uninote_core::{DocumentFormat, Item};
use uninote_stroke::Rect;

/// MIME type standard per la serializzazione degli appunti di UniNote.
pub const MIME_UNINOTE_SELECTION: &str = "application/x-uninote-strokes+json";
/// MIME type di fallback per compatibilità testuale UTF-8.
pub const MIME_TEXT_PLAIN: &str = "text/plain;charset=utf-8";

/// Dati contenuti negli appunti (in memoria e serializzabili).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClipboardData {
    /// Formato del documento sorgente (Notebook A4 o Whiteboard).
    pub source_format: DocumentFormat,
    /// ID del documento sorgente (se disponibile).
    #[serde(default)]
    pub source_doc_id: Option<uuid::Uuid>,
    /// Indice della pagina sorgente (se disponibile).
    pub source_page_index: Option<usize>,
    /// Dimensioni (larghezza, altezza) della pagina sorgente (se applicabile).
    pub source_page_size: Option<(f64, f64)>,
    /// Bounding box complessivo originale degli elementi in coordinate documento.
    pub original_bbox: Rect,
    /// Centro geometrico del bounding box originale in coordinate documento.
    pub original_center: (f64, f64),
    /// Tratti/elementi clonati (con cache Bézier e bbox popolata).
    pub items: Vec<Item>,
    /// Conteggio delle operazioni di incolla consecutive effettuate per questo payload.
    #[serde(default)]
    pub paste_count: usize,
    /// Offset del viewport al momento della copia (per rilevamento pan/spostamento vista).
    #[serde(default)]
    pub source_viewport_offset: Option<(f64, f64)>,
}

/// Contenitore statico thread-safe globale in memoria per zero-lag synchronous access.
pub static APP_CLIPBOARD: OnceLock<Arc<Mutex<Option<ClipboardData>>>> = OnceLock::new();

/// Restituisce l'istanza globale condivisa degli appunti in memoria.
pub fn get_app_clipboard() -> Arc<Mutex<Option<ClipboardData>>> {
    APP_CLIPBOARD
        .get_or_init(|| Arc::new(Mutex::new(None)))
        .clone()
}

/// Serializza la struttura `ClipboardData` in stringa JSON.
pub fn serialize_clipboard(data: &ClipboardData) -> Result<String, serde_json::Error> {
    serde_json::to_string(data)
}

/// Deserializza una stringa JSON in `ClipboardData`.
pub fn deserialize_clipboard(json: &str) -> Result<ClipboardData, serde_json::Error> {
    serde_json::from_str(json)
}

/// Imposta i dati negli appunti in memoria ed esegue il broadcast su GdkClipboard di sistema.
pub fn set_clipboard_data(data: ClipboardData) {
    let serialized = serialize_clipboard(&data).ok();

    // 1. Memorizzazione sincrona in-memory a latenza zero
    let cb = get_app_clipboard();
    if let Ok(mut lock) = cb.lock() {
        *lock = Some(data);
    };

    // 2. Broadcast asincrono su GdkClipboard per finestre/processi paralleli
    if let Some(json_str) = serialized {
        if let Some(display) = gdk::Display::default() {
            let sys_cb = display.clipboard();
            let bytes = glib::Bytes::from(json_str.as_bytes());
            let p_app = gdk::ContentProvider::for_bytes(MIME_UNINOTE_SELECTION, &bytes);
            let p_txt = gdk::ContentProvider::for_bytes(MIME_TEXT_PLAIN, &bytes);
            let union = gdk::ContentProvider::new_union(&[p_app, p_txt]);
            let _ = sys_cb.set_content(Some(&union));
        }
    }
}

/// Recupera i dati dagli appunti.
/// Controlla prioritariamente la cache in memoria e, se vuota,
/// tenta la deserializzazione dai payload disponibili in memoria.
pub fn get_clipboard_data() -> Option<ClipboardData> {
    let cb = get_app_clipboard();
    let lock = cb.lock().ok()?;
    (*lock).clone()
}

/// Incrementa il contatore di incolla consecutivi e restituisce il valore precedente.
pub fn advance_paste_count() -> usize {
    let cb = get_app_clipboard();
    let mut lock = match cb.lock() {
        Ok(guard) => guard,
        Err(_) => return 0,
    };
    if let Some(ref mut data) = *lock {
        let prev = data.paste_count;
        data.paste_count += 1;
        prev
    } else {
        0
    }
}

/// Resetta/svuota gli appunti in memoria.
pub fn clear_clipboard() {
    let cb = get_app_clipboard();
    if let Ok(mut lock) = cb.lock() {
        *lock = None;
    };
}

/// Inizializza l'ascoltatore di cambio clipboard di sistema per sincronizzazione tra processi.
pub fn init_system_clipboard_sync() {
    if let Some(display) = gdk::Display::default() {
        let sys_cb = display.clipboard();

        // 1. Sincronizzazione iniziale immediata se la clipboard contiene dati da processo esterno
        if !sys_cb.is_local() {
            sys_cb.read_text_async(gtk4::gio::Cancellable::NONE, |res| {
                if let Ok(Some(text)) = res {
                    if let Ok(data) = deserialize_clipboard(&text) {
                        let in_mem = get_app_clipboard();
                        if let Ok(mut lock) = in_mem.lock() {
                            *lock = Some(data);
                        };
                    }
                }
            });
        }

        // 2. Ascoltatore reattivo per futuri aggiornamenti esterni (idempotente)
        static CONNECT_ONCE: std::sync::Once = std::sync::Once::new();
        CONNECT_ONCE.call_once(|| {
            sys_cb.connect_changed(|cb| {
                if !cb.is_local() {
                    cb.read_text_async(gtk4::gio::Cancellable::NONE, |res| {
                        if let Ok(Some(text)) = res {
                            if let Ok(data) = deserialize_clipboard(&text) {
                                let in_mem = get_app_clipboard();
                                if let Ok(mut lock) = in_mem.lock() {
                                    *lock = Some(data);
                                };
                            }
                        }
                    });
                }
            });
        });
    }
}
