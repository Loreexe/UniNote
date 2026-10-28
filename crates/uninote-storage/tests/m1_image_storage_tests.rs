use tempfile::NamedTempFile;
use uninote_core::model::{Document, ImageItem, Item, PageBackground};
use uninote_storage::worker::StorageWorker;
use uninote_storage::{export_portable_copy, DocumentStore, SCHEMA_VERSION};
use uninote_stroke::types::{PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle};
use uuid::Uuid;

fn make_sample_stroke(id: u64, z_index: u64) -> Item {
    let mut stroke = Stroke::new(id, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    stroke.points.push(StrokePoint {
        x: 10.0,
        y: 20.0,
        pressure: 0.5,
        width: 2.0,
        timestamp_ns: 1000,
    });
    stroke.points.push(StrokePoint {
        x: 50.0,
        y: 60.0,
        pressure: 0.6,
        width: 2.0,
        timestamp_ns: 2000,
    });
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

fn make_sample_image(id: u64, z_index: u64, page_id: Uuid, hash: &str, bbox: Rect) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        hash.to_string(),
        bbox,
        z_index as i64,
        0.9,
        1920,
        1080,
    );
    Item::new_image(id, z_index, img)
}

#[test]
fn test_assets_table_ddl_and_schema_version_3() {
    let store = DocumentStore::open_in_memory().expect("open_in_memory");
    assert_eq!(SCHEMA_VERSION, 3);

    let user_ver: u32 = store
        .connection()
        .query_row("PRAGMA user_version;", [], |r| r.get(0))
        .expect("PRAGMA user_version");
    assert_eq!(user_ver, 3);

    // Verify table structure in sqlite_master
    let table_exists: bool = store
        .connection()
        .query_row(
            "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='assets';",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|cnt| cnt == 1)
        .expect("query assets table");
    assert!(table_exists, "assets table must be created in schema v3");

    // Verify columns: hash, mime_type, data, created_at
    let mut stmt = store
        .connection()
        .prepare("PRAGMA table_info(assets);")
        .expect("PRAGMA table_info");
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .expect("query columns")
        .filter_map(Result::ok)
        .collect();
    assert_eq!(cols, vec!["hash", "mime_type", "data", "created_at"]);
}

#[test]
fn test_save_asset_and_load_asset() {
    let store = DocumentStore::open_in_memory().expect("open_in_memory");
    let test_bytes = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];
    let test_hash = "sha256_mock_sample_asset_hash";

    store
        .save_asset(test_hash, "image/png", &test_bytes)
        .expect("save_asset");

    let loaded = store
        .load_asset(test_hash)
        .expect("load_asset")
        .expect("asset should exist");
    assert_eq!(loaded, test_bytes);

    // Non-existent hash returns Ok(None)
    let missing = store.load_asset("missing_hash_12345").expect("load_asset");
    assert!(missing.is_none());
}

#[test]
fn test_save_asset_hash_addressed_deduplication() {
    let store = DocumentStore::open_in_memory().expect("open_in_memory");
    let test_bytes = vec![42u8; 1024];
    let test_hash = "dedup_hash_unique_123";

    // Insert 50 times with the same hash
    for _ in 0..50 {
        store
            .save_asset(test_hash, "image/jpeg", &test_bytes)
            .expect("save_asset idempotent");
    }

    let count: i64 = store
        .connection()
        .query_row(
            "SELECT count(*) FROM assets WHERE hash = ?;",
            [test_hash],
            |r| r.get(0),
        )
        .expect("count rows");
    assert_eq!(count, 1, "Deduplication must guarantee exactly 1 row");
}

#[test]
fn test_zero_blob_rewrites_on_document_save() {
    let temp_file = NamedTempFile::new().expect("NamedTempFile");
    let path = temp_file.path();

    let asset_bytes = vec![0xCA, 0xFE, 0xBA, 0xBE, 0xAA, 0xBB, 0xCC, 0xDD];
    let asset_hash = "zero_blob_rewrite_hash_01";

    let mut doc = Document::new_empty("Zero BLOB Rewrite Test");
    let page_id = doc.pages[0].id;

    // 1. Save asset first (as would occur upon image import)
    {
        let store = DocumentStore::open_or_create(path).expect("open_or_create");
        store
            .save_asset(asset_hash, "image/png", &asset_bytes)
            .expect("save_asset");
    }

    // 2. Add ImageItem to document pointing to asset_hash
    let image_item = make_sample_image(
        1001,
        1,
        page_id,
        asset_hash,
        Rect::new(50.0, 50.0, 250.0, 150.0),
    );
    doc.pages[0].items.push(image_item);

    // Record created_at timestamp in assets table
    let initial_created_at: i64 = {
        let store = DocumentStore::open_or_create(path).expect("open store");
        store
            .connection()
            .query_row(
                "SELECT created_at FROM assets WHERE hash = ?;",
                [asset_hash],
                |r| r.get(0),
            )
            .expect("get created_at")
    };

    // 3. Save document multiple times
    for _ in 0..5 {
        let mut store = DocumentStore::open_or_create(path).expect("open store");
        store.save_document(&doc).expect("save_document");
    }

    // 4. Verify assets table was untouched
    {
        let store = DocumentStore::open_or_create(path).expect("open store");
        let post_created_at: i64 = store
            .connection()
            .query_row(
                "SELECT created_at FROM assets WHERE hash = ?;",
                [asset_hash],
                |r| r.get(0),
            )
            .expect("get post created_at");
        assert_eq!(
            initial_created_at, post_created_at,
            "created_at in assets table must never be touched during document save"
        );

        let row_count: i64 = store
            .connection()
            .query_row("SELECT count(*) FROM assets;", [], |r| r.get(0))
            .expect("count assets");
        assert_eq!(row_count, 1);

        // Verify items table has item_type = 'image' and contains valid JSON
        let (item_type, data_json): (String, String) = store
            .connection()
            .query_row(
                "SELECT item_type, data_json FROM items WHERE id = 1001;",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .expect("query image item");
        assert_eq!(item_type, "image");
        assert!(data_json.contains(asset_hash));
    }
}

#[test]
fn test_document_roundtrip_with_mixed_strokes_and_images() {
    let temp_file = NamedTempFile::new().expect("NamedTempFile");
    let path = temp_file.path();

    let asset1_hash = "asset_hash_one_abc";
    let asset2_hash = "asset_hash_two_xyz";
    let asset1_data = vec![1, 2, 3, 4, 5];
    let asset2_data = vec![10, 20, 30, 40, 50, 60];

    // Pre-save assets
    {
        let store = DocumentStore::open_or_create(path).expect("open");
        store
            .save_asset(asset1_hash, "image/png", &asset1_data)
            .expect("save asset1");
        store
            .save_asset(asset2_hash, "image/webp", &asset2_data)
            .expect("save asset2");
    }

    let mut doc = Document::new_empty("Mixed Document");
    let page0_id = doc.pages[0].id;
    let page1_id = doc.add_page(PageBackground::Grid { grid_size: 20.0 });

    // Page 0: stroke 1 (z=1), image 1 (z=2), stroke 2 (z=3)
    let s1 = make_sample_stroke(101, 1);
    let img1 = make_sample_image(
        102,
        2,
        page0_id,
        asset1_hash,
        Rect::new(20.0, 30.0, 120.0, 130.0),
    );
    let s2 = make_sample_stroke(103, 3);
    doc.pages[0].items = vec![s1, img1, s2];
    doc.pages[0].rebuild_spatial_index();

    // Page 1: image 2 (z=1)
    let img2 = make_sample_image(
        201,
        1,
        page1_id,
        asset2_hash,
        Rect::new(100.0, 100.0, 300.0, 250.0),
    );
    doc.pages[1].items = vec![img2];
    doc.pages[1].rebuild_spatial_index();

    // Save document
    {
        let mut store = DocumentStore::open_or_create(path).expect("open for save");
        store.save_document(&doc).expect("save_document");
    }

    // Reload document
    let loaded_doc = {
        let store = DocumentStore::open_or_create(path).expect("open for load");
        store.load_document().expect("load_document")
    };

    assert_eq!(loaded_doc.pages.len(), 2);

    // Verify Page 0
    let p0 = &loaded_doc.pages[0];
    assert_eq!(p0.items.len(), 3);
    assert_eq!(p0.strokes().count(), 2);
    assert_eq!(p0.images().count(), 1);
    assert_eq!(p0.items[1].id, 102);
    assert_eq!(p0.items[1].z_index, 2);
    let loaded_img1 = p0.items[1].as_image().expect("must be image");
    assert_eq!(loaded_img1.asset_hash, asset1_hash);
    assert_eq!(loaded_img1.bbox, Rect::new(20.0, 30.0, 120.0, 130.0));
    assert_eq!(loaded_img1.opacity, 0.9);

    // Verify Page 1
    let p1 = &loaded_doc.pages[1];
    assert_eq!(p1.items.len(), 1);
    assert_eq!(p1.images().count(), 1);
    let loaded_img2 = p1.items[0].as_image().expect("must be image");
    assert_eq!(loaded_img2.asset_hash, asset2_hash);

    // Spatial indices must be valid and rehydrated
    assert_eq!(p0.spatial_index.len(), 3);
    assert_eq!(p1.spatial_index.len(), 1);
}

#[test]
fn test_portable_backup_preserves_assets() {
    let src_file = NamedTempFile::new().expect("src temp");
    let dst_file = NamedTempFile::new().expect("dst temp");
    let src_path = src_file.path();
    let dst_path = dst_file.path();

    let asset_hash = "backup_asset_hash_999";
    let asset_bytes = vec![0xDE, 0xAD, 0xBE, 0xEF];

    {
        let mut store = DocumentStore::open_or_create(src_path).expect("open src");
        store
            .save_asset(asset_hash, "image/png", &asset_bytes)
            .expect("save asset");
        let mut doc = Document::new_empty("Backup Test");
        let page_id = doc.pages[0].id;
        doc.pages[0].items.push(make_sample_image(
            1,
            1,
            page_id,
            asset_hash,
            Rect::new(0.0, 0.0, 50.0, 50.0),
        ));
        store.save_document(&doc).expect("save doc");

        // Export portable copy using store's active connection
        export_portable_copy(store.connection(), dst_path).expect("export_portable_copy");
    }

    // Open destination and verify assets + document
    {
        let store = DocumentStore::open_or_create(dst_path).expect("open dst");
        let loaded_asset = store
            .load_asset(asset_hash)
            .expect("load asset")
            .expect("must exist in dst");
        assert_eq!(loaded_asset, asset_bytes);

        let loaded_doc = store.load_document().expect("load doc");
        assert_eq!(loaded_doc.pages[0].images().count(), 1);
    }
}

#[test]
fn test_async_storage_worker_asset_persistence() {
    let temp_file = NamedTempFile::new().expect("temp file");
    let path = temp_file.path().to_path_buf();

    let worker = StorageWorker::spawn(path.clone(), |_status| {}).expect("spawn worker");

    let test_hash = "worker_async_asset_hash";
    let test_bytes = vec![7, 8, 9, 10, 11];

    // Send SaveAsset request via worker helper
    worker.save_asset(test_hash, "image/png", test_bytes.clone(), None);

    // Send save_revision
    let doc = Document::new_empty("Worker Asset Test");
    worker.save_revision(doc, 1);

    // Load asset via worker
    let rx = worker.load_asset(test_hash);
    let loaded = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("recv result")
        .expect("worker load_asset succeeded")
        .expect("asset must exist");
    assert_eq!(loaded, test_bytes);

    // Verify persisted directly in SQLite database file
    let store = DocumentStore::open_or_create(&path).expect("reopen store");
    let loaded_direct = store
        .load_asset(test_hash)
        .expect("load asset direct")
        .expect("must exist in db file");
    assert_eq!(loaded_direct, test_bytes);
}
