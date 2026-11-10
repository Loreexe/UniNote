use tempfile::NamedTempFile;
use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, Item, TextItem, TextLayer};
use uninote_storage::DocumentStore;
use uninote_stroke::Rect;
use uuid::Uuid;

#[test]
fn test_text_item_creation_and_layer_distinction() {
    let page_id = Uuid::new_v4();
    let underlay_math = TextItem::new(
        page_id,
        r"\int_0^\infty e^{-x^2} dx = \frac{\sqrt{\pi}}{2}",
        Rect::new(50.0, 100.0, 250.0, 150.0),
        TextLayer::Underlay,
    );
    assert!(underlay_math.is_underlay());
    assert_eq!(underlay_math.layer, TextLayer::Underlay);

    let normal_box = TextItem::new(
        page_id,
        "# Appunti di Analisi\nDefinizione formale...",
        Rect::new(100.0, 200.0, 400.0, 350.0),
        TextLayer::Normal,
    );
    assert!(!normal_box.is_underlay());
    assert_eq!(normal_box.layer, TextLayer::Normal);
}

#[test]
fn test_text_item_doc_command_undo_redo_and_spatial_index() {
    let mut history = CommandHistory::new(50);
    let mut doc = Document::new_notebook("Test Text Commands");
    let page_id = doc.pages[0].id;

    let math_item = TextItem::new(
        page_id,
        r"E = mc^2",
        Rect::new(20.0, 30.0, 120.0, 70.0),
        TextLayer::Underlay,
    );
    let cmd = DocCommand::add_text(page_id, 1, 1, math_item.clone());
    history.execute(cmd, &mut doc);

    assert_eq!(doc.pages[0].items.len(), 1);
    assert!(doc.pages[0].items[0].is_text());
    assert_eq!(doc.pages[0].texts().next().unwrap().raw_text, r"E = mc^2");

    // Modifica testo
    let edit_cmd = DocCommand::EditText {
        page_id,
        item_id: 1,
        old_text: r"E = mc^2".to_string(),
        new_text: r"E^2 = (pc)^2 + (m_0 c^2)^2".to_string(),
        old_bbox: Rect::new(20.0, 30.0, 120.0, 70.0),
        new_bbox: Rect::new(20.0, 30.0, 200.0, 80.0),
    };
    history.execute(edit_cmd, &mut doc);
    assert_eq!(
        doc.pages[0].texts().next().unwrap().raw_text,
        r"E^2 = (pc)^2 + (m_0 c^2)^2"
    );

    // Undo modifica
    assert!(history.can_undo());
    history.undo(&mut doc);
    assert_eq!(doc.pages[0].texts().next().unwrap().raw_text, r"E = mc^2");

    // Undo inserimento
    history.undo(&mut doc);
    assert_eq!(doc.pages[0].items.len(), 0);

    // Redo inserimento
    history.redo(&mut doc);
    assert_eq!(doc.pages[0].items.len(), 1);

    // Redo modifica
    history.redo(&mut doc);
    assert_eq!(
        doc.pages[0].texts().next().unwrap().raw_text,
        r"E^2 = (pc)^2 + (m_0 c^2)^2"
    );
}

#[test]
fn test_text_item_sqlite_persistence_roundtrip() {
    let tmp = NamedTempFile::new().unwrap();
    let path = tmp.path();

    let mut store = DocumentStore::open_or_create(path).unwrap();
    let mut doc = Document::new_notebook("Test Storage Roundtrip");
    let page_id = doc.pages[0].id;

    let math_underlay = TextItem::with_id(
        Uuid::new_v4(),
        page_id,
        r"\sum_{n=1}^\infty \frac{1}{n^2} = \frac{\pi^2}{6}",
        Rect::new(40.0, 60.0, 220.0, 110.0),
        0,
        TextLayer::Underlay,
        18.0,
        [0.1, 0.2, 0.8, 1.0],
        None,
        None,
    );

    let text_box = TextItem::with_id(
        Uuid::new_v4(),
        page_id,
        "### Nota Marginale\nQuesto risultato è dovuto a Eulero.",
        Rect::new(250.0, 60.0, 500.0, 160.0),
        1,
        TextLayer::Normal,
        14.0,
        [0.0, 0.0, 0.0, 1.0],
        Some([0.95, 0.95, 0.95, 1.0]),
        Some([0.8, 0.8, 0.8, 1.0]),
    );

    doc.pages[0].add_item(Item::new_text(1, 1, math_underlay));
    doc.pages[0].add_item(Item::new_text(2, 2, text_box));

    store.save_document(&doc).unwrap();

    // Riapertura e reload dal database SQLite
    let reloaded = store.load_document().unwrap();
    assert_eq!(reloaded.pages.len(), 1);
    let p0 = &reloaded.pages[0];
    assert_eq!(p0.items.len(), 2);

    let t0 = p0.items[0].as_text().expect("item 0 must be Text");
    assert_eq!(t0.layer, TextLayer::Underlay);
    assert!(t0.is_underlay());
    assert_eq!(
        t0.raw_text,
        r"\sum_{n=1}^\infty \frac{1}{n^2} = \frac{\pi^2}{6}"
    );
    assert_eq!(t0.font_size, 18.0);

    let t1 = p0.items[1].as_text().expect("item 1 must be Text");
    assert_eq!(t1.layer, TextLayer::Normal);
    assert!(!t1.is_underlay());
    assert_eq!(t1.background_fill, Some([0.95, 0.95, 0.95, 1.0]));
    assert_eq!(t1.border_color, Some([0.8, 0.8, 0.8, 1.0]));

    // Verifica R-Tree index rehydrated
    let found = p0
        .spatial_index
        .locate_in_envelope(30.0, 50.0, 230.0, 120.0);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].item_id, 1);
}
