use crate::schema;
use rusqlite::{params, Connection, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;
use uninote_core::{Document, DocumentFormat, Item, ItemKind, Page, PageBackground};
use uninote_stroke::Stroke;
use uuid::Uuid;

/// Metadati serializzati in JSON per sfondi PDF salvati nella colonna `background_extra`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfPageStorageMeta {
    pub source_path: String,
    pub page_index: usize,
    pub original_width: f64,
    pub original_height: f64,
}

pub struct DocumentStore {
    conn: Connection,
}

impl DocumentStore {
    pub fn open_or_create(path: &Path) -> Result<Self> {
        let mut conn = Connection::open(path)?;
        schema::initialize_schema(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        schema::initialize_schema(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn connection(&self) -> &Connection {
        &self.conn
    }

    pub fn connection_mut(&mut self) -> &mut Connection {
        &mut self.conn
    }

    pub fn verify_integrity(&self) -> Result<bool> {
        let mut stmt = self.conn.prepare("PRAGMA integrity_check;")?;
        let result: String = stmt.query_row([], |row| row.get(0))?;
        Ok(result == "ok")
    }

    pub fn save_document(&mut self, doc: &Document) -> Result<()> {
        let tx = self.conn.transaction()?;

        // Metadata
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('doc_id', ?1)",
            params![doc.id.to_string()],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('title', ?1)",
            params![doc.title],
        )?;
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('version', ?1)",
            params![doc.version.to_string()],
        )?;
        let format_str = match doc.format {
            DocumentFormat::Notebook => "notebook",
            DocumentFormat::Whiteboard => "whiteboard",
        };
        tx.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('document_format', ?1)",
            params![format_str],
        )?;

        // Pagine ed elementi
        tx.execute("DELETE FROM pages", [])?;
        tx.execute("DELETE FROM items", [])?;

        for page in &doc.pages {
            let (bg_type, bg_spacing, bg_extra) = match &page.background {
                PageBackground::Blank => ("blank", 0.0, String::new()),
                PageBackground::Ruled { line_spacing, .. } => {
                    ("ruled", *line_spacing, String::new())
                }
                PageBackground::Grid { grid_size } => ("grid", *grid_size, String::new()),
                PageBackground::PdfPage {
                    source_path,
                    page_index,
                    original_width,
                    original_height,
                } => {
                    let meta = PdfPageStorageMeta {
                        source_path: source_path.clone(),
                        page_index: *page_index,
                        original_width: *original_width,
                        original_height: *original_height,
                    };
                    let extra = serde_json::to_string(&meta).unwrap_or_default();
                    ("pdf_page", *page_index as f64, extra)
                }
            };

            tx.execute(
                "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    page.id.to_string(),
                    page.index as i64,
                    page.width,
                    page.height,
                    bg_type,
                    bg_spacing,
                    bg_extra,
                ],
            )?;

            for item in &page.items {
                let (item_type, bbox_str, data_json) = match &item.kind {
                    ItemKind::Stroke(stroke) => {
                        let bbox = stroke.bounding_box();
                        let bbox_json = serde_json::to_string(&bbox).unwrap_or_default();
                        let stroke_json = serde_json::to_string(&stroke).unwrap_or_default();
                        ("stroke", bbox_json, stroke_json)
                    }
                };

                tx.execute(
                    "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        item.id as i64,
                        page.id.to_string(),
                        item.z_index as i64,
                        item_type,
                        bbox_str,
                        data_json,
                    ],
                )?;
            }
        }

        tx.commit()?;
        Ok(())
    }

    pub fn load_document(&self) -> Result<Document> {
        let doc_id_str: String = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'doc_id'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| Uuid::new_v4().to_string());
        let doc_id = Uuid::parse_str(&doc_id_str).unwrap_or_else(|_| Uuid::new_v4());

        let title: String = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'title'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "Senza titolo".to_string());

        let version_str: String = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or_else(|_| "1".to_string());
        let version: u32 = version_str.parse().unwrap_or(1);

        let format: DocumentFormat = self
            .conn
            .query_row(
                "SELECT value FROM metadata WHERE key = 'document_format'",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|s| match s.as_str() {
                "whiteboard" => DocumentFormat::Whiteboard,
                _ => DocumentFormat::Notebook,
            })
            .unwrap_or(DocumentFormat::Notebook);

        let mut page_stmt = self.conn.prepare(
            "SELECT id, page_index, width, height, background_type, background_spacing, background_extra
             FROM pages ORDER BY page_index ASC",
        )?;

        let page_rows = page_stmt.query_map([], |row| {
            let id_str: String = row.get(0)?;
            let index: i64 = row.get(1)?;
            let width: f64 = row.get(2)?;
            let height: f64 = row.get(3)?;
            let bg_type: String = row.get(4)?;
            let bg_spacing: f64 = row.get(5)?;
            let bg_extra: String = row.get(6).unwrap_or_default();
            Ok((
                id_str,
                index as usize,
                width,
                height,
                bg_type,
                bg_spacing,
                bg_extra,
            ))
        })?;

        let mut pages = Vec::new();
        for p in page_rows {
            let (id_str, index, width, height, bg_type, bg_spacing, bg_extra) = p?;
            let page_id = Uuid::parse_str(&id_str).unwrap_or_else(|_| Uuid::new_v4());
            let background = match bg_type.as_str() {
                "ruled" => PageBackground::Ruled {
                    line_spacing: bg_spacing,
                    margin_left: 40.0,
                },
                "blank" => PageBackground::Blank,
                "grid" => PageBackground::Grid {
                    grid_size: bg_spacing,
                },
                "pdf_page" | "pdf" => {
                    if let Ok(meta) = serde_json::from_str::<PdfPageStorageMeta>(&bg_extra) {
                        PageBackground::PdfPage {
                            source_path: meta.source_path,
                            page_index: meta.page_index,
                            original_width: meta.original_width,
                            original_height: meta.original_height,
                        }
                    } else {
                        // Fallback di sicurezza se il JSON è mancante o corrotto
                        PageBackground::PdfPage {
                            source_path: String::new(),
                            page_index: bg_spacing as usize,
                            original_width: width,
                            original_height: height,
                        }
                    }
                }
                _ => {
                    if bg_type.starts_with('{') {
                        serde_json::from_str::<PageBackground>(&bg_type).unwrap_or(
                            PageBackground::Grid {
                                grid_size: bg_spacing,
                            },
                        )
                    } else {
                        PageBackground::Grid {
                            grid_size: bg_spacing,
                        }
                    }
                }
            };

            let mut item_stmt = self.conn.prepare(
                "SELECT id, z_index, item_type, data_json FROM items
                 WHERE page_id = ?1 ORDER BY z_index ASC",
            )?;

            let item_rows = item_stmt.query_map(params![id_str], |row| {
                let id: i64 = row.get(0)?;
                let z_index: i64 = row.get(1)?;
                let item_type: String = row.get(2)?;
                let data_json: String = row.get(3)?;
                Ok((id as u64, z_index as u64, item_type, data_json))
            })?;

            let mut items = Vec::new();
            for it in item_rows {
                let (id, z_index, item_type, data_json) = it?;
                if item_type == "stroke" {
                    if let Ok(stroke) = serde_json::from_str::<Stroke>(&data_json) {
                        items.push(Item {
                            id,
                            z_index,
                            kind: ItemKind::Stroke(stroke),
                        });
                    }
                }
            }

            pages.push(Page {
                id: page_id,
                index,
                width,
                height,
                background,
                items,
            });
        }

        if pages.is_empty() {
            match format {
                DocumentFormat::Notebook => {
                    pages.push(Page::new_a4(0, PageBackground::default()));
                }
                DocumentFormat::Whiteboard => {
                    pages.push(Page::new_whiteboard(PageBackground::default()));
                }
            }
        }

        Ok(Document {
            id: doc_id,
            title,
            version,
            format,
            pages,
        })
    }
}
