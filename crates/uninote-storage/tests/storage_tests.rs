use tempfile::NamedTempFile;
use uninote_core::{DocCommand, Document, DocumentFormat, ItemKind, PageBackground};
use uninote_storage::{export_portable_copy, DocumentStore, APPLICATION_ID, SCHEMA_VERSION};
use uninote_stroke::{Stroke, StrokePoint, StrokeStyle};

#[test]
fn test_database_initialization_and_schema_version() {
    let store = DocumentStore::open_in_memory().expect("Apertura in memoria");

    // Verifica integrità
    assert!(store.verify_integrity().expect("Verifica integrità"));

    // Verifica application_id e user_version
    let app_id: u32 = store
        .connection()
        .query_row("PRAGMA application_id;", [], |r| r.get(0))
        .expect("PRAGMA application_id");
    assert_eq!(app_id, APPLICATION_ID);

    let user_ver: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, SCHEMA_VERSION);
}

#[test]
fn test_save_and_reload_document_fidelity() {
    let temp_file = NamedTempFile::new().expect("File temporaneo");
    let file_path = temp_file.path();

    let mut doc = Document::new_empty("Analisi Matematica I");
    let page_id_0 = doc.pages[0].id;
    let page_id_1 = doc.add_page(PageBackground::Ruled {
        line_spacing: 24.0,
        margin_left: 35.0,
    });

    // Aggiungi uno stroke sulla pagina 0
    let stroke_0 = Stroke {
        id: 101,
        points: vec![
            StrokePoint {
                x: 100.0,
                y: 120.0,
                pressure: 0.45,
                width: 2.5,
                timestamp_ns: 1_000_000,
            },
            StrokePoint {
                x: 110.0,
                y: 130.0,
                pressure: 0.55,
                width: 2.8,
                timestamp_ns: 2_000_000,
            },
        ],
        style: StrokeStyle {
            color: (0.1, 0.2, 0.8, 1.0),
            base_width: 3.0,
            min_width_factor: 0.3,
            max_width_factor: 2.0,
        },
        raw_samples: Vec::new(),
        is_dot: false,
        mode: uninote_stroke::PipelineMode::CatmullRomSpline,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };

    let mut history = uninote_core::CommandHistory::default();
    history.execute(
        DocCommand::AddStroke {
            page_id: page_id_0,
            stroke: stroke_0.clone(),
        },
        &mut doc,
    );

    // Salva nel database
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Apertura store");
        store.save_document(&doc).expect("Salvataggio documento");
    }

    // Riapri da disco e verifica
    {
        let store = DocumentStore::open_or_create(file_path).expect("Riapertura store");
        assert!(store.verify_integrity().expect("Integrità file"));

        let loaded_doc = store.load_document().expect("Caricamento documento");
        assert_eq!(loaded_doc.id, doc.id);
        assert_eq!(loaded_doc.title, "Analisi Matematica I");
        assert_eq!(loaded_doc.pages.len(), 2);

        // Pagina 0
        let p0 = &loaded_doc.pages[0];
        assert_eq!(p0.id, page_id_0);
        assert_eq!(p0.items.len(), 1);
        let ItemKind::Stroke(ref s) = p0.items[0].kind;
        assert_eq!(s.id, 101);
        assert_eq!(s.points.len(), 2);
        assert_eq!(s.points[0].x, 100.0);
        assert_eq!(s.style.color, (0.1, 0.2, 0.8, 1.0));

        // Pagina 1
        let p1 = &loaded_doc.pages[1];
        assert_eq!(p1.id, page_id_1);
        match &p1.background {
            PageBackground::Ruled { line_spacing, .. } => {
                assert_eq!(*line_spacing, 24.0);
            }
            _ => panic!("Sfondo pagina 1 non corrispondente"),
        }
    }
}

#[test]
fn test_online_backup_portable_copy() {
    let src_file = NamedTempFile::new().expect("File temporaneo sorgente");
    let dst_file = NamedTempFile::new().expect("File temporaneo destinazione");

    let mut doc = Document::new_empty("Appunti di Fisica");
    doc.add_page(PageBackground::Blank);

    let mut store = DocumentStore::open_or_create(src_file.path()).expect("Store sorgente");
    store.save_document(&doc).expect("Salvataggio");

    // Esegui la copia portabile tramite Online Backup API
    export_portable_copy(store.connection(), dst_file.path()).expect("Online Backup");

    // Apri la copia in modo totalmente indipendente
    let backup_store = DocumentStore::open_or_create(dst_file.path()).expect("Store backup");
    assert!(backup_store.verify_integrity().expect("Integrità backup"));

    let loaded_backup = backup_store.load_document().expect("Caricamento backup");
    assert_eq!(loaded_backup.title, "Appunti di Fisica");
    assert_eq!(loaded_backup.pages.len(), 2);
}

#[test]
fn test_save_and_reload_whiteboard_format() {
    let temp_file = NamedTempFile::new().expect("File temporaneo");
    let file_path = temp_file.path();

    let doc = Document::new_whiteboard(PageBackground::Grid { grid_size: 20.0 });
    assert_eq!(doc.format, DocumentFormat::Whiteboard);
    assert!(doc.pages[0].width.is_infinite());
    assert!(doc.pages[0].height.is_infinite());

    // Salva whiteboard su SQLite
    {
        let mut store = DocumentStore::open_or_create(file_path).expect("Apertura store");
        store.save_document(&doc).expect("Salvataggio whiteboard");
    }

    // Ricarica e verifica formato e dimensioni infinite
    {
        let store = DocumentStore::open_or_create(file_path).expect("Riapertura store");
        let loaded = store.load_document().expect("Caricamento whiteboard");

        assert_eq!(loaded.format, DocumentFormat::Whiteboard);
        assert_eq!(loaded.id, doc.id);
        assert_eq!(loaded.pages.len(), 1);

        let p0 = &loaded.pages[0];
        assert!(p0.width.is_infinite(), "Whiteboard width must be infinite");
        assert!(
            p0.height.is_infinite(),
            "Whiteboard height must be infinite"
        );
        assert_eq!(p0.background, PageBackground::Grid { grid_size: 20.0 });
    }
}

#[test]
fn test_backward_compatibility_missing_document_format() {
    let store = DocumentStore::open_in_memory().expect("Apertura store in memoria");

    // Simula database legacy pre-0.2.0 senza chiave 'document_format' in metadata
    let doc_id = uuid::Uuid::new_v4();
    store
        .connection()
        .execute(
            "INSERT INTO metadata (key, value) VALUES ('doc_id', ?1)",
            [doc_id.to_string()],
        )
        .expect("Insert doc_id");
    store
        .connection()
        .execute(
            "INSERT INTO metadata (key, value) VALUES ('title', 'Legacy 0.1.0 Notebook')",
            [],
        )
        .expect("Insert title");
    store
        .connection()
        .execute(
            "INSERT INTO metadata (key, value) VALUES ('version', '1')",
            [],
        )
        .expect("Insert version");

    let page_id = uuid::Uuid::new_v4();
    store
        .connection()
        .execute(
            "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing)
             VALUES (?1, 0, 595.28, 841.89, 'blank', 0.0)",
            [page_id.to_string()],
        )
        .expect("Insert page");

    let loaded = store.load_document().expect("Caricamento documento legacy");
    // Deve effettuare il fallback trasparente a DocumentFormat::Notebook
    assert_eq!(loaded.format, DocumentFormat::Notebook);
    assert_eq!(loaded.id, doc_id);
    assert_eq!(loaded.title, "Legacy 0.1.0 Notebook");
    assert_eq!(loaded.pages.len(), 1);
    assert_eq!(loaded.pages[0].width, 595.28);
    assert_eq!(loaded.pages[0].height, 841.89);
}

#[test]
fn test_sqlite_v1_to_v2_migration_and_pdf_persistence() {
    let mut conn = rusqlite::Connection::open_in_memory().unwrap();
    // Simula schema V1 senza colonna background_extra
    conn.execute_batch(
        "CREATE TABLE pages (
            id TEXT PRIMARY KEY,
            page_index INTEGER NOT NULL,
            width REAL NOT NULL,
            height REAL NOT NULL,
            background_type TEXT NOT NULL,
            background_spacing REAL NOT NULL
        );
        PRAGMA user_version = 1;",
    )
    .unwrap();
    conn.execute(
        "INSERT INTO pages VALUES ('00000000-0000-0000-0000-000000000001', 0, 595.28, 841.89, 'ruled', 22.0)",
        [],
    )
    .unwrap();

    // Esegui la migrazione
    uninote_storage::initialize_schema(&mut conn).unwrap();

    let user_ver: u32 = conn
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .unwrap();
    assert_eq!(user_ver, 2);

    // Verifica che la colonna sia presente e che il record V1 sia intatto
    let (bg_type, bg_extra): (String, String) = conn
        .query_row(
            "SELECT background_type, background_extra FROM pages WHERE page_index = 0",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(bg_type, "ruled");
    assert_eq!(bg_extra, "");
}

#[test]
fn test_save_and_reload_pdf_page_fidelity() {
    let temp_file = NamedTempFile::new().expect("File temporaneo");
    let file_path = temp_file.path();

    let mut doc = Document::new_empty("Slide Corso Universitario");
    let pdf_source = "/tmp/lezioni/lezione01.pdf";
    doc.pages[0] = uninote_core::Page::new_pdf_page(0, pdf_source, 0, 960.0, 540.0);
    doc.add_pdf_page(pdf_source, 1, 960.0, 540.0);

    {
        let mut store = DocumentStore::open_or_create(file_path).unwrap();
        store.save_document(&doc).unwrap();
    }

    {
        let store = DocumentStore::open_or_create(file_path).unwrap();
        assert!(store.verify_integrity().unwrap());
        let loaded = store.load_document().unwrap();
        assert_eq!(loaded.pages.len(), 2);
        assert_eq!(loaded.pages[0].width, 960.0);
        assert_eq!(loaded.pages[0].height, 540.0);

        match &loaded.pages[0].background {
            PageBackground::PdfPage {
                source_path,
                page_index,
                original_width,
                original_height,
            } => {
                assert_eq!(source_path, pdf_source);
                assert_eq!(*page_index, 0);
                assert_eq!(*original_width, 960.0);
                assert_eq!(*original_height, 540.0);
            }
            _ => panic!("Expected PdfPage background on page 0"),
        }

        match &loaded.pages[1].background {
            PageBackground::PdfPage {
                source_path,
                page_index,
                original_width,
                original_height,
            } => {
                assert_eq!(source_path, pdf_source);
                assert_eq!(*page_index, 1);
                assert_eq!(*original_width, 960.0);
                assert_eq!(*original_height, 540.0);
            }
            _ => panic!("Expected PdfPage background on page 1"),
        }
    }
}
