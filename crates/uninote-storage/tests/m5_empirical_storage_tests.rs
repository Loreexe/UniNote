//! M5 Empirical & Adversarial Storage Verification Suite
//!
//! Scope:
//! - SQLite Schema V2 migration from legacy V1 schema.
//! - Idempotent migration across single and multiple connections.
//! - Full data preservation (pages and items) across migration.
//! - Robustness against corrupted, truncated, or adversarial `background_extra` JSON.
//! - `PdfPageStorageMeta` high-volume roundtrip fidelity (heterogeneous dimensions, unicode paths).
//! - Interleaved mixed page backgrounds (Blank, Ruled, Grid, PdfPage) with stroke items.

use rusqlite::Connection;
use tempfile::NamedTempFile;
use uninote_core::{Document, DocumentFormat, ItemKind, Page, PageBackground};
use uninote_storage::{initialize_schema, DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{PipelineMode, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

fn make_test_stroke(id: u64, x: f64, y: f64) -> Stroke {
    Stroke {
        id,
        points: vec![
            StrokePoint {
                x,
                y,
                pressure: 0.5,
                width: 2.0,
                timestamp_ns: 1_000_000,
            },
            StrokePoint {
                x: x + 10.0,
                y: y + 10.0,
                pressure: 0.7,
                width: 2.8,
                timestamp_ns: 2_000_000,
            },
        ],
        style: StrokeStyle {
            color: (0.2, 0.4, 0.8, 1.0),
            base_width: 2.5,
            min_width_factor: 0.5,
            max_width_factor: 1.5,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    }
}

// ============================================================================
// 1. SCHEMA V2 MIGRATION & IDEMPOTENCY
// ============================================================================

#[test]
fn test_m5_idempotent_schema_initialization_multiple_calls_same_connection() {
    let mut conn = Connection::open_in_memory().expect("In-memory connection");

    // Chiama initialize_schema 10 volte consecutivamente
    for i in 1..=10 {
        initialize_schema(&mut conn)
            .unwrap_or_else(|e| panic!("Call {} of initialize_schema failed: {:?}", i, e));
    }

    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, SCHEMA_VERSION);

    let app_id: u32 = conn
        .query_row("PRAGMA application_id;", [], |r| r.get(0))
        .expect("PRAGMA application_id");
    assert_eq!(app_id, APPLICATION_ID);

    // Verifica che la colonna background_extra sia presente esattamente una volta
    let mut stmt = conn.prepare("PRAGMA table_info(pages)").unwrap();
    let rows = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    let count = rows.iter().filter(|c| *c == "background_extra").count();
    assert_eq!(
        count, 1,
        "background_extra must appear exactly once in table pages"
    );
}

#[test]
fn test_m5_idempotent_schema_initialization_multiple_connections_on_disk() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // Inizializza con la prima connessione
    {
        let store = DocumentStore::open_or_create(file_path).expect("First open");
        assert!(store.verify_integrity().expect("Integrity 1"));
    }

    // Riapri con 5 connessioni consecutive
    for i in 2..=6 {
        let store = DocumentStore::open_or_create(file_path)
            .unwrap_or_else(|e| panic!("Connection {} open failed: {:?}", i, e));
        assert!(store.verify_integrity().expect("Integrity"));
    }
}

#[test]
fn test_m5_v1_migration_full_fidelity_pages_and_items() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let doc_id = Uuid::new_v4();
    let page_id_1 = Uuid::new_v4();
    let page_id_2 = Uuid::new_v4();
    let page_id_3 = Uuid::new_v4();

    // 1. Simula un database V1 legacy reale creato con UniNote 0.1.0
    {
        let conn = Connection::open(file_path).expect("Open raw SQLite");
        conn.execute_batch(
            "PRAGMA application_id = 0x554E494E;
             PRAGMA user_version = 1;
             CREATE TABLE metadata (key TEXT PRIMARY KEY, value TEXT NOT NULL);
             CREATE TABLE pages (
                 id TEXT PRIMARY KEY,
                 page_index INTEGER NOT NULL,
                 width REAL NOT NULL,
                 height REAL NOT NULL,
                 background_type TEXT NOT NULL,
                 background_spacing REAL NOT NULL
             );
             CREATE TABLE items (
                 id INTEGER PRIMARY KEY,
                 page_id TEXT NOT NULL REFERENCES pages(id) ON DELETE CASCADE,
                 z_index INTEGER NOT NULL,
                 item_type TEXT NOT NULL,
                 bounding_box TEXT NOT NULL,
                 data_json TEXT NOT NULL
             );
             CREATE INDEX idx_items_page ON items(page_id);",
        )
        .expect("Create V1 schema");

        // Inserisci metadata V1
        conn.execute(
            "INSERT INTO metadata VALUES ('doc_id', ?1), ('title', 'Appunti V1'), ('version', '1')",
            [doc_id.to_string()],
        )
        .expect("Insert V1 metadata");

        // Inserisci 3 pagine con sfondi diversi
        conn.execute(
            "INSERT INTO pages VALUES (?1, 0, 595.28, 841.89, 'blank', 0.0)",
            [page_id_1.to_string()],
        )
        .expect("Insert page 1");
        conn.execute(
            "INSERT INTO pages VALUES (?1, 1, 595.28, 841.89, 'ruled', 22.68)",
            [page_id_2.to_string()],
        )
        .expect("Insert page 2");
        conn.execute(
            "INSERT INTO pages VALUES (?1, 2, 595.28, 841.89, 'grid', 14.17)",
            [page_id_3.to_string()],
        )
        .expect("Insert page 3");

        // Inserisci 3 stroke (uno per pagina)
        for (i, p_id) in [page_id_1, page_id_2, page_id_3].iter().enumerate() {
            let stroke = make_test_stroke((i + 1) as u64, 100.0 * (i + 1) as f64, 150.0);
            let stroke_json = serde_json::to_string(&stroke).unwrap();
            let bbox_json = serde_json::to_string(&stroke.bounding_box()).unwrap();

            conn.execute(
                "INSERT INTO items VALUES (?1, ?2, ?3, 'stroke', ?4, ?5)",
                rusqlite::params![
                    (i + 1) as i64,
                    p_id.to_string(),
                    (i + 1) as i64,
                    bbox_json,
                    stroke_json
                ],
            )
            .expect("Insert stroke");
        }
    }

    // 2. Apri il database con DocumentStore (innesca migrazione V1 -> V2)
    {
        let store = DocumentStore::open_or_create(file_path).expect("Open store on V1 db");
        assert!(store.verify_integrity().expect("Integrity after migration"));

        // Verifica user_version aggiornato a 3
        let user_ver: u32 = store
            .connection()
            .query_row("PRAGMA user_version;", [], |r| r.get(0))
            .expect("PRAGMA user_version");
        assert_eq!(user_ver, 3);

        // Carica documento e verifica fedeltà assoluta dei dati preesistenti
        let doc = store.load_document().expect("Load migrated document");
        assert_eq!(doc.id, doc_id);
        assert_eq!(doc.title, "Appunti V1");
        assert_eq!(doc.format, DocumentFormat::Notebook);
        assert_eq!(doc.pages.len(), 3);

        // Verifica pagina 0 (blank)
        assert_eq!(doc.pages[0].id, page_id_1);
        assert_eq!(doc.pages[0].background, PageBackground::Blank);
        assert_eq!(doc.pages[0].items.len(), 1);
        let s = doc.pages[0].items[0].as_stroke().expect("stroke");
        assert_eq!(s.id, 1);
        assert_eq!(s.points.len(), 2);

        // Verifica pagina 1 (ruled)
        assert_eq!(doc.pages[1].id, page_id_2);
        match doc.pages[1].background {
            PageBackground::Ruled { line_spacing, .. } => assert_eq!(line_spacing, 22.68),
            _ => panic!("Expected ruled on page 1"),
        }
        assert_eq!(doc.pages[1].items.len(), 1);

        // Verifica pagina 2 (grid)
        assert_eq!(doc.pages[2].id, page_id_3);
        match doc.pages[2].background {
            PageBackground::Grid { grid_size } => assert_eq!(grid_size, 14.17),
            _ => panic!("Expected grid on page 2"),
        }
        assert_eq!(doc.pages[2].items.len(), 1);
    }

    // 3. Modifica e salva nuovamente sul database migrato (aggiunge pagina PDF)
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let mut doc = store.load_document().expect("Load doc");

        doc.add_pdf_page("/tmp/lezioni/dispensa_fisica.pdf", 5, 960.0, 540.0);
        store.save_document(&doc).expect("Save updated doc");
    }

    // 4. Riapri e verifica che la nuova pagina PDF sia persistita con metadati
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store 3");
        let doc = store.load_document().expect("Load doc 3");
        assert_eq!(doc.pages.len(), 4);

        match &doc.pages[3].background {
            PageBackground::PdfPage {
                source_path,
                page_index,
                original_width,
                original_height,
            } => {
                assert_eq!(source_path, "/tmp/lezioni/dispensa_fisica.pdf");
                assert_eq!(*page_index, 5);
                assert_eq!(*original_width, 960.0);
                assert_eq!(*original_height, 540.0);
            }
            _ => panic!("Expected PdfPage on page 3"),
        }
    }
}

// ============================================================================
// 2. ADVERSARIAL & CORRUPTED BACKGROUND_EXTRA JSON
// ============================================================================

#[test]
fn test_m5_adversarial_corrupted_background_extra_variants() {
    let corrupt_payloads = [
        "",                                  // Empty string (legacy default)
        "   ",                               // Whitespace only
        "not json",                          // Plain syntax error
        "{",                                 // Incomplete JSON
        "{\"source_path\":",                 // Truncated JSON
        "[1, 2, 3]",                         // Array instead of object
        "\"string literal\"",                // JSON string instead of object
        "42",                                // Number instead of object
        "null",                              // JSON null
        "true",                              // JSON boolean
        "{}",                                // Empty JSON object (all fields missing)
        "{\"source_path\":\"test.pdf\"}",    // Missing page_index, width, height
        "{\"source_path\": 123}",            // Type mismatch on source_path
        "{\"page_index\": \"zero\"}",        // Type mismatch on page_index
        "{\"original_width\": \"invalid\"}", // Type mismatch on original_width
        "{\"source_path\": null}",           // Null source path
    ];

    for (i, payload) in corrupt_payloads.iter().enumerate() {
        let temp_file = NamedTempFile::new().expect("Temp file");
        let file_path = temp_file.path();

        // Popola direttamente il database con il payload corrotto nella colonna background_extra
        {
            let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
            let p_id = Uuid::new_v4();
            store
                .connection_mut()
                .execute(
                    "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
                     VALUES (?1, 0, 800.0, 600.0, 'pdf_page', 3.0, ?2)",
                    rusqlite::params![p_id.to_string(), payload],
                )
                .unwrap_or_else(|e| panic!("Insert corrupt payload {} failed: {:?}", i, e));
        }

        // Il caricamento DEVE degradare con successo al fallback sicuro senza panico né crash
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let doc = store
            .load_document()
            .unwrap_or_else(|e| panic!("Payload {} caused load_document to fail: {:?}", i, e));

        assert_eq!(doc.pages.len(), 1);
        match &doc.pages[0].background {
            PageBackground::PdfPage {
                source_path,
                page_index,
                original_width,
                original_height,
            } => {
                // Nel fallback di sicurezza, source_path è vuoto, page_index = bg_spacing (3),
                // e width/height corrispondono alle dimensioni della pagina (800.0, 600.0)
                assert_eq!(source_path, "");
                assert_eq!(*page_index, 3);
                assert_eq!(*original_width, 800.0);
                assert_eq!(*original_height, 600.0);
            }
            _ => panic!("Expected graceful fallback to PdfPage for payload {}", i),
        }
    }
}

#[test]
fn test_m5_background_extra_with_unknown_and_nested_fields() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    // JSON valido con campi addizionali non previsti (future-proofing)
    let payload = r#"{
        "source_path": "/home/user/document.pdf",
        "page_index": 7,
        "original_width": 1920.0,
        "original_height": 1080.0,
        "future_field_dpi": 300,
        "future_metadata": { "author": "UniNote", "encrypted": false }
    }"#;

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        let p_id = Uuid::new_v4();
        store
            .connection_mut()
            .execute(
                "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
                 VALUES (?1, 0, 1920.0, 1080.0, 'pdf_page', 7.0, ?2)",
                rusqlite::params![p_id.to_string(), payload],
            )
            .expect("Insert page");
    }

    let store = DocumentStore::open_or_create(file_path).expect("Reopen");
    let doc = store.load_document().expect("Load document");

    match &doc.pages[0].background {
        PageBackground::PdfPage {
            source_path,
            page_index,
            original_width,
            original_height,
        } => {
            assert_eq!(source_path, "/home/user/document.pdf");
            assert_eq!(*page_index, 7);
            assert_eq!(*original_width, 1920.0);
            assert_eq!(*original_height, 1080.0);
        }
        _ => panic!("Expected PdfPage"),
    }
}

// ============================================================================
// 3. HIGH-VOLUME & SPECIAL CHARACTERS ROUNDTRIP FIDELITY
// ============================================================================

#[test]
fn test_m5_pdf_page_storage_meta_unicode_and_special_paths() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let complex_paths = [
        "/home/studente/Università degli Studi/Analisi II (2026)/Dispense & Esercizi/Lezione 01 — Limiti e Continuità 🎓.pdf",
        "/var/tmp/paths with 'single' and \"double\" quotes/slide.pdf",
        "C:\\Users\\Studente\\OneDrive - Università\\Fisica I.pdf",
        "/mnt/dati/math_symbols_∫_∑_√_π_λ/slide.pdf",
        "/tmp/newline\nin\npath/doc.pdf",
        "/tmp/tab\tin\tpath/doc.pdf",
    ];

    let mut doc = Document::new_empty("Unicode and Special PDF Paths");
    doc.pages.clear();

    for (i, path) in complex_paths.iter().enumerate() {
        let width = 500.0 + (i as f64 * 50.0);
        let height = 700.0 + (i as f64 * 30.0);
        doc.add_pdf_page(*path, i, width, height);
    }

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save document");
    }

    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load document");
        assert_eq!(loaded.pages.len(), complex_paths.len());

        for (i, page) in loaded.pages.iter().enumerate() {
            let expected_width = 500.0 + (i as f64 * 50.0);
            let expected_height = 700.0 + (i as f64 * 30.0);
            assert_eq!(page.width, expected_width);
            assert_eq!(page.height, expected_height);

            match &page.background {
                PageBackground::PdfPage {
                    source_path,
                    page_index,
                    original_width,
                    original_height,
                } => {
                    assert_eq!(source_path, complex_paths[i]);
                    assert_eq!(*page_index, i);
                    assert_eq!(*original_width, expected_width);
                    assert_eq!(*original_height, expected_height);
                }
                _ => panic!("Expected PdfPage for index {}", i),
            }
        }
    }
}

#[test]
fn test_m5_pdf_page_storage_meta_100_pages_stress() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_empty("Corso Completo 100 Slide");
    doc.pages.clear();

    let source = "/media/ssd/universita/Algoritmi_e_Strutture_Dati_Slide.pdf";

    for i in 0..100 {
        // Alterna formati: 16:9 widescreen, 4:3 standard, A4 vertical
        let (w, h) = match i % 3 {
            0 => (960.0, 540.0),   // 16:9
            1 => (720.0, 540.0),   // 4:3
            _ => (595.28, 841.89), // A4
        };
        doc.add_pdf_page(source, i, w, h);
    }

    assert_eq!(doc.pages.len(), 100);

    // Salva
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save 100 pages");
    }

    // Carica e verifica fedeltà assoluta su tutte le 100 pagine
    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load 100 pages");
        assert_eq!(loaded.pages.len(), 100);

        for (i, page) in loaded.pages.iter().enumerate() {
            let (expected_w, expected_h) = match i % 3 {
                0 => (960.0, 540.0),
                1 => (720.0, 540.0),
                _ => (595.28, 841.89),
            };

            assert_eq!(page.index, i);
            assert_eq!(page.width, expected_w);
            assert_eq!(page.height, expected_h);

            match &page.background {
                PageBackground::PdfPage {
                    source_path,
                    page_index,
                    original_width,
                    original_height,
                } => {
                    assert_eq!(source_path, source);
                    assert_eq!(*page_index, i);
                    assert_eq!(*original_width, expected_w);
                    assert_eq!(*original_height, expected_h);
                }
                _ => panic!("Expected PdfPage at index {}", i),
            }
        }
    }
}

#[test]
fn test_m5_interleaved_mixed_page_backgrounds_and_strokes() {
    let temp_file = NamedTempFile::new().expect("Temp file");
    let file_path = temp_file.path();

    let mut doc = Document::new_empty("Documento Misto Universitario");
    doc.pages.clear();

    // 0: A4 Blank
    doc.pages.push(Page::new_a4(0, PageBackground::Blank));
    // 1: A4 Ruled
    doc.pages.push(Page::new_a4(
        1,
        PageBackground::Ruled {
            line_spacing: 24.0,
            margin_left: 40.0,
        },
    ));
    // 2: PDF slide 16:9
    doc.add_pdf_page("/tmp/lezione.pdf", 0, 960.0, 540.0);
    // 3: A4 Grid
    doc.pages
        .push(Page::new_a4(3, PageBackground::Grid { grid_size: 15.0 }));
    // 4: PDF slide 16:9 (pagina 1)
    doc.add_pdf_page("/tmp/lezione.pdf", 1, 960.0, 540.0);

    // Aggiungi stroke su ciascuna pagina
    for (i, p) in doc.pages.iter_mut().enumerate() {
        let stroke = make_test_stroke((i + 10) as u64, 50.0 + i as f64 * 10.0, 100.0);
        p.items.push(uninote_core::Item {
            id: (i + 10) as u64,
            z_index: 0,
            kind: ItemKind::Stroke(stroke),
        });
    }

    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Open store");
        store.save_document(&doc).expect("Save mixed document");
    }

    {
        let store = DocumentStore::open_or_create(file_path).expect("Reopen store");
        let loaded = store.load_document().expect("Load mixed document");
        assert_eq!(loaded.pages.len(), 5);

        // Pagina 0: Blank
        assert_eq!(loaded.pages[0].background, PageBackground::Blank);
        assert_eq!(loaded.pages[0].items.len(), 1);

        // Pagina 1: Ruled
        match loaded.pages[1].background {
            PageBackground::Ruled { line_spacing, .. } => assert_eq!(line_spacing, 24.0),
            _ => panic!("Expected Ruled"),
        }
        assert_eq!(loaded.pages[1].items.len(), 1);

        // Pagina 2: PDF Page 0
        match &loaded.pages[2].background {
            PageBackground::PdfPage {
                page_index,
                original_width,
                ..
            } => {
                assert_eq!(*page_index, 0);
                assert!(*original_width > 0.0);
            }
            _ => panic!("Expected PdfPage 0"),
        }
        assert_eq!(loaded.pages[2].items.len(), 1);

        // Pagina 3: Grid
        match loaded.pages[3].background {
            PageBackground::Grid { grid_size } => assert_eq!(grid_size, 15.0),
            _ => panic!("Expected Grid"),
        }
        assert_eq!(loaded.pages[3].items.len(), 1);

        // Pagina 4: PDF Page 1
        match &loaded.pages[4].background {
            PageBackground::PdfPage { page_index, .. } => {
                assert_eq!(*page_index, 1);
            }
            _ => panic!("Expected PdfPage 1"),
        }
        assert_eq!(loaded.pages[4].items.len(), 1);
    }
}
