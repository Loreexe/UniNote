use rusqlite::{Connection, Result};

pub const APPLICATION_ID: u32 = 0x554E_494E; // "UNIN"
pub const SCHEMA_VERSION: u32 = 1;

/// Inizializza lo schema del database SQLite configurando WAL e tabelle.
pub fn initialize_schema(conn: &mut Connection) -> Result<()> {
    conn.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA user_version = {SCHEMA_VERSION};"
    ))?;

    let tx = conn.transaction()?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        )",
        [],
    )?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS pages (
            id TEXT PRIMARY KEY,
            page_index INTEGER NOT NULL,
            width REAL NOT NULL,
            height REAL NOT NULL,
            background_type TEXT NOT NULL,
            background_spacing REAL NOT NULL
        )",
        [],
    )?;

    tx.execute(
        "CREATE TABLE IF NOT EXISTS items (
            id INTEGER PRIMARY KEY,
            page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
            z_index INTEGER NOT NULL,
            item_type TEXT NOT NULL,
            bounding_box TEXT NOT NULL,
            data_json TEXT NOT NULL
        )",
        [],
    )?;

    tx.execute(
        "CREATE INDEX IF NOT EXISTS idx_items_page ON items(page_id)",
        [],
    )?;

    tx.commit()?;
    Ok(())
}
