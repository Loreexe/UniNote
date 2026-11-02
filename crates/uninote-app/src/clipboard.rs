use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Read;
use std::sync::{Arc, Mutex, OnceLock};
use uninote_core::{DocumentFormat, Item};
use uninote_stroke::Rect;

/// MIME type standard per la serializzazione degli appunti di UniNote.
pub const MIME_UNINOTE_SELECTION: &str = "application/x-uninote-strokes+json";
/// MIME type di fallback per compatibilità testuale UTF-8.
pub const MIME_TEXT_PLAIN: &str = "text/plain;charset=utf-8";

/// MIME types riconosciuti per immagini raster negli appunti di sistema.
pub const IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/jpg",
    "image/webp",
    "image/bmp",
    "image/tiff",
    "image/x-png",
];

/// Payload immagine estratto dagli appunti di sistema normalizzato in PNG deterministico.
#[derive(Debug, Clone)]
pub struct ClipboardImage {
    /// Byte compressi in formato PNG deterministico.
    pub bytes: Vec<u8>,
    /// Larghezza originale in pixel.
    pub width: u32,
    /// Altezza originale in pixel.
    pub height: u32,
    /// MIME type standard dell'immagine (tipicamente "image/png").
    pub mime_type: String,
}

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

/// Legge in modo asincrono un'immagine o texture da `gdk::Clipboard`.
///
/// Workflow:
/// 1. Tenta prioritariamente `gdk::Clipboard::read_texture_async`. Se disponibile,
///    estrae le dimensioni e normalizza i pixel in PNG tramite `texture.save_to_png_bytes()`.
/// 2. In caso di fallimento o assenza di texture diretta, esegue il fallback su stream
///    binario tramite `gdk::Clipboard::read_async` per `IMAGE_MIME_TYPES`.
/// 3. Converte i byte ricevuti in `gdk::Texture` tramite `Texture::from_bytes` e
///    successivamente in PNG tramite `save_to_png_bytes()`.
/// 4. Se nessun formato valido è presente o se l'ambiente è headless, invoca `callback(None)`.
pub fn read_clipboard_image_async<F>(callback: F)
where
    F: FnOnce(Option<ClipboardImage>) + 'static,
{
    let Some(display) = gdk::Display::default() else {
        callback(None);
        return;
    };

    let clipboard = display.clipboard();

    // 1. Tenta prioritariamente la lettura diretta come texture GDK
    clipboard.read_texture_async(gtk4::gio::Cancellable::NONE, move |res| {
        match res {
            Ok(Some(texture)) => {
                let w = texture.width();
                let h = texture.height();
                if w > 0 && h > 0 {
                    let png_bytes = texture.save_to_png_bytes();
                    if !png_bytes.is_empty() {
                        callback(Some(ClipboardImage {
                            bytes: png_bytes.to_vec(),
                            width: w as u32,
                            height: h as u32,
                            mime_type: "image/png".to_string(),
                        }));
                        return;
                    }
                }
                callback(None);
            }
            _ => {
                // 2. Fallback: lettura stream asincrona per formati raster alternativi
                let fallback_display = match gdk::Display::default() {
                    Some(d) => d,
                    None => {
                        callback(None);
                        return;
                    }
                };
                let fallback_cb = fallback_display.clipboard();
                fallback_cb.read_async(
                    IMAGE_MIME_TYPES,
                    glib::Priority::DEFAULT,
                    gtk4::gio::Cancellable::NONE,
                    move |stream_res| match stream_res {
                        Ok((stream, _out_mime)) => {
                            let mut reader = stream.into_read();
                            let mut raw_data = Vec::new();
                            if reader.read_to_end(&mut raw_data).is_ok() && !raw_data.is_empty() {
                                let glib_bytes = glib::Bytes::from(&raw_data);
                                if let Ok(texture) = gdk::Texture::from_bytes(&glib_bytes) {
                                    let w = texture.width();
                                    let h = texture.height();
                                    if w > 0 && h > 0 {
                                        let png_bytes = texture.save_to_png_bytes();
                                        if !png_bytes.is_empty() {
                                            callback(Some(ClipboardImage {
                                                bytes: png_bytes.to_vec(),
                                                width: w as u32,
                                                height: h as u32,
                                                mime_type: "image/png".to_string(),
                                            }));
                                            return;
                                        }
                                    }
                                }
                            }
                            callback(None);
                        }
                        Err(_) => {
                            callback(None);
                        }
                    },
                );
            }
        }
    });
}

/// Inizializza l'ascoltatore di cambio clipboard di sistema per sincronizzazione tra processi.
pub fn init_system_clipboard_sync() {
    if let Some(display) = gdk::Display::default() {
        let sys_cb = display.clipboard();

        let handle_clipboard_change = |cb: &gdk::Clipboard| {
            if !cb.is_local() {
                let formats = cb.formats();
                let has_image = formats.contains_type(gdk::Texture::static_type())
                    || IMAGE_MIME_TYPES
                        .iter()
                        .any(|mime| formats.contain_mime_type(mime));

                if has_image {
                    // Immagine esterna rilevata: azzera i vecchi tratti interni per dare precedenza all'immagine
                    clear_clipboard();
                } else {
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
            }
        };

        // 1. Sincronizzazione iniziale immediata se la clipboard contiene dati da processo esterno
        handle_clipboard_change(&sys_cb);

        // 2. Ascoltatore reattivo per futuri aggiornamenti esterni (idempotente)
        static CONNECT_ONCE: std::sync::Once = std::sync::Once::new();
        CONNECT_ONCE.call_once(|| {
            sys_cb.connect_changed(handle_clipboard_change);
        });
    }
}
