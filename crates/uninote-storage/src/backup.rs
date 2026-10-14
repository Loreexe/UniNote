use rusqlite::{Connection, Result};
use std::path::Path;

/// Esporta una copia portabile autoconsistente del database SQLite usando l'Online Backup API.
/// Garantisce il consolidamento atomico di tutte le pagine WAL nel file destinazione
/// senza dipendenze da file WAL esterni o lock bloccanti sulla connessione principale.
pub fn export_portable_copy(src_conn: &Connection, dst_path: &Path) -> Result<()> {
    let mut dst_conn = Connection::open(dst_path)?;
    let backup = rusqlite::backup::Backup::new(src_conn, &mut dst_conn)?;
    backup.run_to_completion(10, std::time::Duration::from_millis(5), None)?;
    Ok(())
}
