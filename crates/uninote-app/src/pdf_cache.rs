//! Architettura a doppio livello di cache Cairo per il rendering e l'esportazione di documenti PDF.
//!
//! # Architettura a Doppio Livello (Dual-Tier Cairo Cache)
//!
//! - **Livello 1 (Fast Path Raster - 120Hz Blit)**:
//!   - Mantiene in memoria una superficie raster `cairo::ImageSurface` pre-renderizzata
//!     a risoluzione 2x HiDPI (ARgb32) per le pagine PDF visibili.
//!   - Durante il pan e il pinch-to-zoom continuo a 120Hz, `canvas.rs` esegue un semplice
//!     blit di memoria con filtro bilineare:
//!     `cr.set_source_surface(&surface, 0.0, 0.0); cr.paint();`
//!     Tempo di esecuzione misurato: < 200 µs (anziché 15-50 ms di un render vettoriale completo).
//!   - Gestione memoria: le superfici raster sono soggette a sfratto LRU (Least Recently Used)
//!     con un limite predefinito (default: 8 pagine, ~64 MB RAM).
//!
//! - **Livello 2 (Vector Handles - Sharpness at Rest & Export)**:
//!   - Mantiene aperti gli handle vettoriali nativi `poppler::PopplerDocument` e `PopplerPage`.
//!   - Al termine delle gesture (at rest), se il fattore di zoom è elevato (> 2x), il gestore
//!     rigenera la superficie raster alla risoluzione nativa per la massima nitidezza.
//!   - Durante l'esportazione PDF (`cairo::PdfSurface`), il gestore fornisce la pagina Poppler
//!     vettoriale per invocare `page.render_for_printing(&cr)`, garantendo la riscrittura
//!     dei flussi vettoriali nativi senza alcuna rasterizzazione né perdita di qualità.

use cairo::{Context, Format, ImageSurface};
use poppler::{PopplerDocument, PopplerPage};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Identificativo univoco di una specifica pagina PDF in cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PdfPageKey {
    /// Percorso canonico al file PDF sorgente.
    pub source_path: PathBuf,
    /// Indice a base zero della pagina.
    pub page_index: usize,
}

impl PdfPageKey {
    pub fn new(source_path: impl Into<PathBuf>, page_index: usize) -> Self {
        Self {
            source_path: source_path.into(),
            page_index,
        }
    }
}

/// Voce della cache raster di Livello 1 (`ImageSurface`).
pub struct RasterCacheEntry {
    /// Superficie Cairo raster pre-renderizzata in memoria.
    pub surface: ImageSurface,
    /// Larghezza originaria in punti tipografici (pt).
    pub width_pt: f64,
    /// Altezza originaria in punti tipografici (pt).
    pub height_pt: f64,
    /// Fattore di sovracampionamento con cui è stata generata la superficie (es. 2.0 per 2x HiDPI).
    pub scale: f64,
    /// Timestamp dell'ultimo accesso per ordinamento LRU.
    pub last_accessed: Instant,
    /// Contatore sequenziale di accessi per ordinamento LRU deterministico.
    pub access_seq: u64,
}

/// Voce della cache vettoriale di Livello 2 (`PopplerDocument`).
pub struct VectorDocEntry {
    /// Documento Poppler nativo aperto.
    pub doc: PopplerDocument,
    /// Conteggio totale delle pagine del PDF.
    pub num_pages: usize,
    /// Timestamp dell'ultimo accesso.
    pub last_accessed: Instant,
}

/// Errori restituibili dalle operazioni di caching e rendering PDF.
#[derive(Debug, Clone)]
pub enum PdfCacheError {
    /// File PDF non trovato o inaccessibile.
    FileNotFound(PathBuf),
    /// Errore emesso dalla libreria Poppler durante l'apertura o il rendering.
    PopplerError(String),
    /// Indice di pagina richiesto non valido.
    PageNotFound {
        page_index: usize,
        total_pages: usize,
    },
    /// Errore interno della libreria Cairo (allocazione o stato surface).
    CairoError(String),
    /// Dimensioni della pagina non valide o pari a zero.
    InvalidDimensions { width: f64, height: f64 },
}

impl std::fmt::Display for PdfCacheError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::FileNotFound(p) => write!(f, "File PDF non trovato: {}", p.display()),
            Self::PopplerError(e) => write!(f, "Errore Poppler: {}", e),
            Self::PageNotFound {
                page_index,
                total_pages,
            } => {
                write!(
                    f,
                    "Pagina {} non trovata (totale: {})",
                    page_index, total_pages
                )
            }
            Self::CairoError(e) => write!(f, "Errore Cairo: {}", e),
            Self::InvalidDimensions { width, height } => {
                write!(f, "Dimensioni pagina non valide: {}x{}", width, height)
            }
        }
    }
}

impl std::error::Error for PdfCacheError {}

impl From<cairo::Error> for PdfCacheError {
    fn from(err: cairo::Error) -> Self {
        Self::CairoError(format!("{:?}", err))
    }
}

/// Parametri di configurazione del `PdfCacheManager`.
#[derive(Debug, Clone)]
pub struct PdfCacheConfig {
    /// Fattore di sovracampionamento HiDPI predefinito (default: 2.0).
    pub default_hidpi_scale: f64,
    /// Numero massimo di superfici raster mantenute contemporaneamente in cache LRU (default: 8).
    pub max_raster_surfaces: usize,
    /// Numero massimo di documenti Poppler mantenuti aperti contemporaneamente (default: 4).
    pub max_vector_docs: usize,
}

impl Default for PdfCacheConfig {
    fn default() -> Self {
        Self {
            default_hidpi_scale: 2.0,
            max_raster_surfaces: 8,
            max_vector_docs: 4,
        }
    }
}

/// Gestore centrale a doppio livello per il caching di documenti PDF.
pub struct PdfCacheManager {
    config: PdfCacheConfig,
    /// Livello 1: Cache raster in memoria (chiave: percorso + indice pagina).
    raster_cache: HashMap<PdfPageKey, RasterCacheEntry>,
    /// Livello 2: Cache documenti vettoriali aperti (chiave: percorso).
    vector_docs: HashMap<PathBuf, VectorDocEntry>,
    /// Contatore sequenziale di accessi per LRU deterministico.
    access_counter: u64,
}

impl PdfCacheManager {
    /// Crea un nuovo `PdfCacheManager` con configurazione personalizzata.
    pub fn new(config: PdfCacheConfig) -> Self {
        Self {
            config,
            raster_cache: HashMap::new(),
            vector_docs: HashMap::new(),
            access_counter: 0,
        }
    }

    /// Configurazione ottimale per display Wayland a 120Hz.
    pub fn default_120hz() -> Self {
        Self::new(PdfCacheConfig::default())
    }

    /// Riferimento alla configurazione attiva.
    pub fn config(&self) -> &PdfCacheConfig {
        &self.config
    }

    // =========================================================================
    // LIVELLO 1: Fast Path Raster (120Hz Blit)
    // =========================================================================

    /// [Fast Path 120Hz]: Restituisce la superficie raster pre-renderizzata a 2x HiDPI
    /// per la pagina specificata. Se non è presente in cache, viene rasterizzata
    /// istantaneamente e memorizzata con evizione LRU automatica.
    ///
    /// Il valore restituito è un clone leggero `cairo::ImageSurface` (reference increment < 5ns)
    /// pronto per essere blittato nel contesto di disegno GTK.
    pub fn get_or_render_raster(
        &mut self,
        source_path: &Path,
        page_index: usize,
    ) -> Result<ImageSurface, PdfCacheError> {
        let key = PdfPageKey::new(source_path, page_index);

        if let Some(entry) = self.raster_cache.get_mut(&key) {
            self.access_counter += 1;
            entry.last_accessed = Instant::now();
            entry.access_seq = self.access_counter;
            return Ok(entry.surface.clone());
        }

        // Cache miss: renderizza la pagina e memorizza in Livello 1
        self.render_page_raster(source_path, page_index, self.config.default_hidpi_scale)
    }

    /// Verifica se una superficie raster è già presente in cache senza forzarne il rendering.
    pub fn get_raster_if_cached(
        &mut self,
        source_path: &Path,
        page_index: usize,
    ) -> Option<ImageSurface> {
        let key = PdfPageKey::new(source_path, page_index);
        if let Some(entry) = self.raster_cache.get_mut(&key) {
            self.access_counter += 1;
            entry.last_accessed = Instant::now();
            entry.access_seq = self.access_counter;
            Some(entry.surface.clone())
        } else {
            None
        }
    }

    /// [Sharpness at Rest]: Aggiorna la scala di rendering raster per una pagina specifica
    /// quando l'utente si ferma su un fattore di zoom elevato (es. 3.0x - 4.0x).
    pub fn update_raster_scale(
        &mut self,
        source_path: &Path,
        page_index: usize,
        target_scale: f64,
    ) -> Result<ImageSurface, PdfCacheError> {
        let clamped_scale = target_scale.clamp(1.0, 5.0);
        self.render_page_raster(source_path, page_index, clamped_scale)
    }

    /// Renderizza internamente la pagina PDF alla scala specificata e la inserisce in cache.
    fn render_page_raster(
        &mut self,
        source_path: &Path,
        page_index: usize,
        scale: f64,
    ) -> Result<ImageSurface, PdfCacheError> {
        let page = self.get_vector_page_internal(source_path, page_index)?;
        let (w_pt, h_pt) = page.get_size();

        if w_pt <= 0.0 || h_pt <= 0.0 {
            return Err(PdfCacheError::InvalidDimensions {
                width: w_pt,
                height: h_pt,
            });
        }

        let px_w = (w_pt * scale).ceil() as i32;
        let px_h = (h_pt * scale).ceil() as i32;

        let surface = ImageSurface::create(Format::ARgb32, px_w, px_h)?;
        let ctx = Context::new(&surface)?;

        // Sfondo bianco opaco per garantire leggibilità ottimale
        ctx.set_source_rgb(1.0, 1.0, 1.0);
        let _ = ctx.paint();

        // Scala il contesto affinché 1 unità pt = `scale` pixel
        ctx.scale(scale, scale);

        // Rendering Poppler nativo nel contesto Cairo
        page.render(&ctx);

        // Evizione LRU se la dimensione eccede la capacità
        self.evict_lru_raster_if_needed();

        self.access_counter += 1;
        let key = PdfPageKey::new(source_path, page_index);
        let entry = RasterCacheEntry {
            surface: surface.clone(),
            width_pt: w_pt,
            height_pt: h_pt,
            scale,
            last_accessed: Instant::now(),
            access_seq: self.access_counter,
        };

        self.raster_cache.insert(key, entry);
        Ok(surface)
    }

    /// Sfratto LRU: rimuove la superficie meno recentemente utilizzata.
    fn evict_lru_raster_if_needed(&mut self) {
        if self.raster_cache.len() < self.config.max_raster_surfaces {
            return;
        }

        let oldest_key = self
            .raster_cache
            .iter()
            .min_by_key(|(_, entry)| entry.access_seq)
            .map(|(key, _)| key.clone());

        if let Some(key) = oldest_key {
            self.raster_cache.remove(&key);
        }
    }

    // =========================================================================
    // LIVELLO 2: Vector Handles (PopplerDocument & PopplerPage)
    // =========================================================================

    /// [Vector Handle]: Restituisce la `PopplerPage` nativa, utilizzata per
    /// l'esportazione tipografica (`page.render_for_printing(&cr)`) o per query su metadati.
    pub fn get_vector_page(
        &mut self,
        source_path: &Path,
        page_index: usize,
    ) -> Result<PopplerPage, PdfCacheError> {
        self.get_vector_page_internal(source_path, page_index)
    }

    fn get_vector_page_internal(
        &mut self,
        source_path: &Path,
        page_index: usize,
    ) -> Result<PopplerPage, PdfCacheError> {
        let doc_entry = self.get_or_load_vector_doc(source_path)?;
        let total = doc_entry.num_pages;

        if page_index >= total {
            return Err(PdfCacheError::PageNotFound {
                page_index,
                total_pages: total,
            });
        }

        doc_entry
            .doc
            .get_page(page_index)
            .ok_or(PdfCacheError::PageNotFound {
                page_index,
                total_pages: total,
            })
    }

    /// Ottiene o apre un `PopplerDocument` memorizzato nella cache vettoriale.
    fn get_or_load_vector_doc(
        &mut self,
        source_path: &Path,
    ) -> Result<&mut VectorDocEntry, PdfCacheError> {
        if !self.vector_docs.contains_key(source_path) {
            if !source_path.exists() {
                return Err(PdfCacheError::FileNotFound(source_path.to_path_buf()));
            }

            let doc = PopplerDocument::new_from_file(source_path, None).map_err(|err| {
                PdfCacheError::PopplerError(format!("Impossibile aprire il file PDF: {:?}", err))
            })?;

            let num_pages = doc.get_n_pages();

            // Evizione documenti Poppler se superato il limite
            if self.vector_docs.len() >= self.config.max_vector_docs {
                let oldest_path = self
                    .vector_docs
                    .iter()
                    .min_by_key(|(_, entry)| entry.last_accessed)
                    .map(|(p, _)| p.clone());
                if let Some(p) = oldest_path {
                    self.vector_docs.remove(&p);
                }
            }

            let entry = VectorDocEntry {
                doc,
                num_pages,
                last_accessed: Instant::now(),
            };
            self.vector_docs.insert(source_path.to_path_buf(), entry);
        }

        let entry = self
            .vector_docs
            .get_mut(source_path)
            .expect("documento inserito");
        entry.last_accessed = Instant::now();
        Ok(entry)
    }

    /// Restituisce le dimensioni native (pt) di una pagina PDF.
    pub fn get_page_size(
        &mut self,
        source_path: &Path,
        page_index: usize,
    ) -> Result<(f64, f64), PdfCacheError> {
        let page = self.get_vector_page_internal(source_path, page_index)?;
        Ok(page.get_size())
    }

    /// Restituisce il conteggio totale delle pagine del documento PDF.
    pub fn get_page_count(&mut self, source_path: &Path) -> Result<usize, PdfCacheError> {
        let doc_entry = self.get_or_load_vector_doc(source_path)?;
        Ok(doc_entry.num_pages)
    }

    /// Pre-carica in cache (warm-up) una lista di pagine (es. pagina corrente ± 2 adiacenti).
    pub fn prefetch_pages(&mut self, source_path: &Path, page_indices: &[usize]) {
        for &idx in page_indices {
            let key = PdfPageKey::new(source_path, idx);
            if !self.raster_cache.contains_key(&key) {
                let _ = self.render_page_raster(source_path, idx, self.config.default_hidpi_scale);
            }
        }
    }

    /// Svuota completamente la cache raster per liberare memoria RAM.
    pub fn clear_raster_cache(&mut self) {
        self.raster_cache.clear();
    }

    /// Rilascia tutte le risorse in cache (superfici raster e documenti vettoriali).
    pub fn clear_all(&mut self) {
        self.raster_cache.clear();
        self.vector_docs.clear();
    }

    /// Restituisce la memoria stimata occupata dalle superfici raster in byte.
    pub fn estimated_raster_memory_bytes(&self) -> usize {
        self.raster_cache
            .values()
            .map(|entry| {
                let px_w = (entry.width_pt * entry.scale).ceil() as usize;
                let px_h = (entry.height_pt * entry.scale).ceil() as usize;
                px_w * px_h * 4 // 4 byte per pixel ARgb32
            })
            .sum()
    }

    /// Restituisce il numero di superfici attualmente residenti in cache raster.
    pub fn cached_surface_count(&self) -> usize {
        self.raster_cache.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pdf_page_key_equality() {
        let k1 = PdfPageKey::new("/tmp/test.pdf", 0);
        let k2 = PdfPageKey::new("/tmp/test.pdf", 0);
        let k3 = PdfPageKey::new("/tmp/test.pdf", 1);
        let k4 = PdfPageKey::new("/tmp/other.pdf", 0);

        assert_eq!(k1, k2);
        assert_ne!(k1, k3);
        assert_ne!(k1, k4);
    }

    #[test]
    fn test_pdf_cache_config_default() {
        let config = PdfCacheConfig::default();
        assert_eq!(config.default_hidpi_scale, 2.0);
        assert_eq!(config.max_raster_surfaces, 8);
        assert_eq!(config.max_vector_docs, 4);
    }

    #[test]
    fn test_cache_manager_initial_state() {
        let mgr = PdfCacheManager::default_120hz();
        assert_eq!(mgr.cached_surface_count(), 0);
        assert_eq!(mgr.estimated_raster_memory_bytes(), 0);
    }
}
