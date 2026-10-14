//! Persistenza SQLite atomica, WAL, worker asincrono e Online Backup per UniNote.

pub mod backup;
pub mod database;
pub mod schema;
pub mod worker;

pub use backup::export_portable_copy;
pub use database::DocumentStore;
pub use schema::{APPLICATION_ID, SCHEMA_VERSION};
pub use worker::{SaveStatus, StorageRequest, StorageWorker};
