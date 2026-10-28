use crate::backup;
use crate::database::DocumentStore;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use uninote_core::Document;

/// Stato del salvataggio del documento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveStatus {
    /// Tutte le modifiche fino alla revisione indicata sono persistite su disco.
    Saved(u64),
    /// Salvataggio asincrono in corso per la revisione indicata.
    Saving(u64),
    /// Modifiche presenti in memoria non ancora persistite.
    Unsaved,
    /// Errore durante l'operazione di persistenza.
    Error(String),
}

/// Richieste inviate al worker di persistenza su thread dedicato.
pub enum StorageRequest {
    /// Salva la revisione del documento.
    SaveRevision { doc: Document, revision: u64 },
    /// Esporta una copia portabile mediante SQLite Backup API.
    ExportBackup {
        target_path: PathBuf,
        responder: Sender<Result<(), String>>,
    },
    /// Salva un asset binario con deduplicazione.
    SaveAsset {
        hash: String,
        mime_type: String,
        data: Vec<u8>,
        responder: Option<Sender<Result<(), String>>>,
    },
    /// Carica i byte di un asset a partire dall'hash.
    LoadAsset {
        hash: String,
        responder: Sender<Result<Option<Vec<u8>>, String>>,
    },
    /// Arresta il thread worker.
    Shutdown,
}

/// Gestore del thread di background per salvataggi asincroni senza latenza nel percorso penna.
pub struct StorageWorker {
    sender: Sender<StorageRequest>,
    worker_thread: Option<JoinHandle<()>>,
}

impl StorageWorker {
    pub fn spawn<F>(file_path: PathBuf, on_status_change: F) -> Result<Self, rusqlite::Error>
    where
        F: Fn(SaveStatus) + Send + Sync + 'static,
    {
        let (tx, rx) = channel::<StorageRequest>();
        let status_cb = Arc::new(on_status_change);

        let thread_handle = thread::spawn(move || {
            let mut store = match DocumentStore::open_or_create(&file_path) {
                Ok(s) => s,
                Err(e) => {
                    status_cb(SaveStatus::Error(format!(
                        "Impossibile aprire il file: {e}"
                    )));
                    return;
                }
            };

            while let Ok(req) = rx.recv() {
                match req {
                    StorageRequest::SaveRevision { doc, revision } => {
                        status_cb(SaveStatus::Saving(revision));
                        match store.save_document(&doc) {
                            Ok(()) => {
                                status_cb(SaveStatus::Saved(revision));
                            }
                            Err(e) => {
                                status_cb(SaveStatus::Error(format!("Errore salvataggio: {e}")));
                            }
                        }
                    }
                    StorageRequest::ExportBackup {
                        target_path,
                        responder,
                    } => {
                        let res = backup::export_portable_copy(store.connection(), &target_path)
                            .map_err(|e| e.to_string());
                        let _ = responder.send(res);
                    }
                    StorageRequest::SaveAsset {
                        hash,
                        mime_type,
                        data,
                        responder,
                    } => {
                        let res = store
                            .save_asset(&hash, &mime_type, &data)
                            .map_err(|e| e.to_string());
                        if let Some(resp) = responder {
                            let _ = resp.send(res);
                        }
                    }
                    StorageRequest::LoadAsset { hash, responder } => {
                        let res = store.load_asset(&hash).map_err(|e| e.to_string());
                        let _ = responder.send(res);
                    }
                    StorageRequest::Shutdown => {
                        break;
                    }
                }
            }
        });

        Ok(Self {
            sender: tx,
            worker_thread: Some(thread_handle),
        })
    }

    pub fn save_revision(&self, doc: Document, revision: u64) {
        let _ = self
            .sender
            .send(StorageRequest::SaveRevision { doc, revision });
    }

    pub fn export_backup(&self, target_path: PathBuf) -> Receiver<Result<(), String>> {
        let (tx, rx) = channel();
        let _ = self.sender.send(StorageRequest::ExportBackup {
            target_path,
            responder: tx,
        });
        rx
    }

    pub fn save_asset(
        &self,
        hash: impl Into<String>,
        mime_type: impl Into<String>,
        data: Vec<u8>,
        responder: Option<Sender<Result<(), String>>>,
    ) {
        let _ = self.sender.send(StorageRequest::SaveAsset {
            hash: hash.into(),
            mime_type: mime_type.into(),
            data,
            responder,
        });
    }

    pub fn load_asset(&self, hash: impl Into<String>) -> Receiver<Result<Option<Vec<u8>>, String>> {
        let (tx, rx) = channel();
        let _ = self.sender.send(StorageRequest::LoadAsset {
            hash: hash.into(),
            responder: tx,
        });
        rx
    }
}

impl Drop for StorageWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(StorageRequest::Shutdown);
        if let Some(handle) = self.worker_thread.take() {
            let _ = handle.join();
        }
    }
}
