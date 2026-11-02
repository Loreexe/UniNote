//! Gestore di cache in memoria LRU per superfici raster Cairo (`ImageSurface`).
//!
//! Fornisce accesso ad altissima frequenza (120 Hz) alle immagini decodificate
//! associate agli `ImageItem`, memorizzate tramite digest SHA-256 e indicizzate per asset_hash.

use cairo::ImageSurface;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::Cursor;
use std::rc::Rc;
use std::time::Instant;
use uninote_storage::DocumentStore;

/// Voce singola memorizzata nella cache delle immagini.
pub struct ImageCacheEntry {
    /// Superficie raster Cairo pre-decodificata in memoria.
    pub surface: ImageSurface,
    /// Larghezza originale in pixel.
    pub width: u32,
    /// Altezza originale in pixel.
    pub height: u32,
    /// Timestamp dell'ultimo accesso per ordinamento LRU.
    pub last_accessed: Instant,
    /// Contatore sequenziale di accessi per LRU deterministico.
    pub access_seq: u64,
}

/// Gestore della cache in memoria per immagini decodificate.
pub struct ImageCacheManager {
    /// Mappa hash SHA-256 -> Voce cache.
    entries: HashMap<String, ImageCacheEntry>,
    /// Numero massimo di immagini mantenute simultaneamente in RAM (default: 64).
    max_entries: usize,
    /// Contatore progressivo per LRU.
    seq_counter: u64,
}

impl Default for ImageCacheManager {
    fn default() -> Self {
        Self::new(64)
    }
}

impl ImageCacheManager {
    /// Crea un nuovo gestore di cache con limite specificato di elementi.
    pub fn new(max_entries: usize) -> Self {
        Self {
            entries: HashMap::new(),
            max_entries: max_entries.max(4),
            seq_counter: 0,
        }
    }

    /// Restituisce la superficie associata a un determinato hash se presente in cache.
    pub fn get(&mut self, hash: &str) -> Option<ImageSurface> {
        self.seq_counter = self.seq_counter.wrapping_add(1);
        let seq = self.seq_counter;
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.last_accessed = Instant::now();
            entry.access_seq = seq;
            Some(entry.surface.clone())
        } else {
            None
        }
    }

    /// Restituisce il riferimento immutabile alla voce della cache se presente.
    pub fn get_entry(&mut self, hash: &str) -> Option<&ImageCacheEntry> {
        self.seq_counter = self.seq_counter.wrapping_add(1);
        let seq = self.seq_counter;
        if let Some(entry) = self.entries.get_mut(hash) {
            entry.last_accessed = Instant::now();
            entry.access_seq = seq;
            Some(entry)
        } else {
            None
        }
    }

    /// Restituisce le dimensioni (width, height) dell'immagine indicizzata se presente.
    pub fn get_dimensions(&mut self, hash: &str) -> Option<(u32, u32)> {
        self.get_entry(hash).map(|e| (e.width, e.height))
    }

    /// Verifica la presenza di un determinato hash in cache senza alterare l'ordine LRU.
    pub fn contains(&self, hash: &str) -> bool {
        self.entries.contains_key(hash)
    }

    /// Inserisce una superficie già creata associandola al digest hash.
    pub fn insert(
        &mut self,
        hash: impl Into<String>,
        surface: ImageSurface,
        width: u32,
        height: u32,
    ) {
        if self.entries.len() >= self.max_entries {
            self.evict_lru();
        }
        self.seq_counter = self.seq_counter.wrapping_add(1);
        let entry = ImageCacheEntry {
            surface,
            width,
            height,
            last_accessed: Instant::now(),
            access_seq: self.seq_counter,
        };
        self.entries.insert(hash.into(), entry);
    }

    /// Decodifica byte PNG e inserisce la superficie risultante nella cache.
    pub fn insert_from_png_bytes(
        &mut self,
        hash: impl Into<String>,
        png_bytes: &[u8],
    ) -> Result<ImageSurface, String> {
        let mut cursor = Cursor::new(png_bytes);
        let surface = ImageSurface::create_from_png(&mut cursor)
            .map_err(|e| format!("Errore decodifica PNG Cairo: {:?}", e))?;

        let w = surface.width() as u32;
        let h = surface.height() as u32;
        let surface_clone = surface.clone();
        self.insert(hash, surface, w, h);
        Ok(surface_clone)
    }

    /// Carica un asset dal database SQLite se mancante in memoria e lo inserisce nella cache.
    pub fn get_or_load_from_store(
        &mut self,
        hash: &str,
        store: &DocumentStore,
    ) -> Option<ImageSurface> {
        if let Some(surf) = self.get(hash) {
            return Some(surf);
        }
        match store.load_asset(hash) {
            Ok(Some(bytes)) => self.insert_from_png_bytes(hash.to_string(), &bytes).ok(),
            _ => None,
        }
    }

    /// Rimuove la voce usata meno di recente (LRU) per liberare memoria.
    pub fn evict_lru(&mut self) {
        if let Some(oldest_key) = self
            .entries
            .iter()
            .min_by_key(|(_, entry)| entry.access_seq)
            .map(|(k, _)| k.clone())
        {
            self.entries.remove(&oldest_key);
        }
    }

    /// Svuota completamente la cache in memoria.
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Numero di immagini attualmente memorizzate in cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Indica se la cache è vuota.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

thread_local! {
    /// Istanza globale della cache immagini confinata al thread UI corrente.
    static GLOBAL_IMAGE_CACHE: Rc<RefCell<ImageCacheManager>> = Rc::new(RefCell::new(ImageCacheManager::default()));
}

/// Restituisce il gestore globale thread-local della cache immagini per il thread UI corrente.
pub fn get_global_image_cache() -> Rc<RefCell<ImageCacheManager>> {
    GLOBAL_IMAGE_CACHE.with(|cache| cache.clone())
}
