use rusqlite::{Connection, Result, TransactionBehavior};

pub const APPLICATION_ID: u32 = 0x554E_494E; // "UNIN"
pub const SCHEMA_VERSION: u32 = 3;

/// Inizializza o aggiorna lo schema del database SQLite con migrazione V1/V2 -> V3.
pub fn initialize_schema(conn: &mut Connection) -> Result<()> {
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    conn.execute_batch(&format!(
        "PRAGMA application_id = {APPLICATION_ID};
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;"
    ))?;

    let current_mode: String = conn
        .query_row("PRAGMA journal_mode;", [], |r| r.get(0))
        .unwrap_or_default();
    if !current_mode.eq_ignore_ascii_case("wal") {
        let _ = conn.execute_batch("PRAGMA journal_mode = WAL;");
    }

    let current_version: u32 = conn.query_row("PRAGMA user_version;", [], |r| r.get(0))?;
    if current_version == SCHEMA_VERSION {
        return Ok(());
    }

    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;

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
            background_spacing REAL NOT NULL,
            background_extra TEXT NOT NULL DEFAULT ''
        )",
        [],
    )?;

    // Verifica dinamica e migrazione trasparente per database V1 preesistenti
    let has_background_extra: bool = {
        let mut stmt = tx.prepare("PRAGMA table_info(pages)")?;
        let mut rows = stmt.query([])?;
        let mut found = false;
        while let Some(row) = rows.next()? {
            let col_name: String = row.get(1)?;
            if col_name == "background_extra" {
                found = true;
                break;
            }
        }
        found
    };

    if !has_background_extra {
        tx.execute(
            "ALTER TABLE pages ADD COLUMN background_extra TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }

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

    tx.execute(
        "CREATE TABLE IF NOT EXISTS assets (
            hash TEXT PRIMARY KEY,
            mime_type TEXT NOT NULL,
            data BLOB NOT NULL,
            created_at INTEGER NOT NULL
        )",
        [],
    )?;

    tx.execute(&format!("PRAGMA user_version = {SCHEMA_VERSION};"), [])?;

    tx.commit()?;
    Ok(())
}
