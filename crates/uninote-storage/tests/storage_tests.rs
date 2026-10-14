use tempfile::NamedTempFile;
use uninote_core::{DocCommand, Document, ItemKind, PageBackground};
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
