use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::{Document, ImageItem, Item, ItemId, ItemKind, Page, PageBackground};
use uninote_core::spatial::PageSpatialIndex;
use uninote_stroke::types::{Rect, StrokePoint};
use uuid::Uuid;

fn make_sample_stroke(id: ItemId, z_index: u64, points: &[(f64, f64)]) -> Item {
    let mut stroke = uninote_stroke::types::Stroke::new(
        id,
        uninote_stroke::types::StrokeStyle::default(),
        uninote_stroke::types::PipelineMode::CatmullRomSpline,
    );
    for (i, &(x, y)) in points.iter().enumerate() {
        stroke.points.push(StrokePoint {
            x,
            y,
            pressure: 0.5,
            width: 2.0,
            timestamp_ns: (i as u64 + 1) * 1000,
        });
    }
    stroke.recompute_cache();
    Item::new_stroke(id, z_index, stroke)
}

fn make_sample_image(id: ItemId, z_index: u64, x: f64, y: f64, w: f64, h: f64, hash: &str) -> Item {
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        Uuid::new_v4(),
        hash.to_string(),
        Rect::new(x, y, x + w, y + h),
        z_index as i64,
        1.0,
        800,
        600,
    );
    Item::new_image(id, z_index, img)
}

#[test]
fn test_image_item_creation_and_aspect_ratio() {
    let page_id = Uuid::new_v4();
    let hash = "a1b2c3d4e5f6".to_string();
    let bbox = Rect::new(50.0, 100.0, 450.0, 300.0); // w=400, h=200
    let img = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        hash.clone(),
        bbox,
        5,
        1.0,
        800,
        400,
    );

    assert_eq!(img.page_id, page_id);
    assert_eq!(img.asset_hash, hash);
    assert_eq!(img.bbox, bbox);
    assert_eq!(img.z_index, 5);
    assert_eq!(img.opacity, 1.0);
    assert_eq!(img.original_width, 800);
    assert_eq!(img.original_height, 400);
    assert!((img.aspect_ratio() - 2.0).abs() < 1e-9);

    // Defensive zero-height aspect ratio check
    let zero_h_img = ImageItem::new(page_id, hash, Rect::new(0.0, 0.0, 100.0, 0.0), 100, 0);
    assert!(zero_h_img.aspect_ratio().is_finite());
}

#[test]
fn test_image_item_translation() {
    let mut img = ImageItem::new(
        Uuid::new_v4(),
        "hash_trans",
        Rect::new(10.0, 20.0, 110.0, 100.0), // w=100, h=80
        100,
        80,
    );

    img.translate(15.0, -5.0);
    assert_eq!(img.bbox.min_x, 25.0);
    assert_eq!(img.bbox.min_y, 15.0);
    assert_eq!(img.bbox.max_x, 125.0);
    assert_eq!(img.bbox.max_y, 95.0);
    assert_eq!(img.bbox.width(), 100.0);
    assert_eq!(img.bbox.height(), 80.0);

    // Defensive non-finite translation (should be a no-op)
    img.translate(f64::NAN, 10.0);
    assert_eq!(img.bbox.min_x, 25.0);
    img.translate(10.0, f64::INFINITY);
    assert_eq!(img.bbox.min_y, 15.0);
}

#[test]
fn test_image_item_anisotropic_scale() {
    let mut img = ImageItem::new(
        Uuid::new_v4(),
        "hash_scale",
        Rect::new(100.0, 100.0, 300.0, 200.0), // w=200, h=100
        200,
        100,
    );

    // Scale 2x on X and 1.5x on Y around origin (100, 100)
    img.scale(2.0, 1.5, 100.0, 100.0);
    assert_eq!(img.bbox.min_x, 100.0);
    assert_eq!(img.bbox.min_y, 100.0);
    assert_eq!(img.bbox.width(), 400.0);
    assert_eq!(img.bbox.height(), 150.0);

    // Defensive non-finite scale (no-op)
    img.scale(f64::NAN, 1.0, 0.0, 0.0);
    assert_eq!(img.bbox.width(), 400.0);
}

#[test]
fn test_image_item_proportional_scale() {
    let mut img = ImageItem::new(
        Uuid::new_v4(),
        "hash_prop",
        Rect::new(50.0, 50.0, 200.0, 125.0), // w=150, h=75
        150,
        75,
    );

    let initial_ratio = img.aspect_ratio();
    img.scale_proportional(0.5, 50.0, 50.0);

    assert_eq!(img.bbox.width(), 75.0);
    assert_eq!(img.bbox.height(), 37.5);
    assert!((img.aspect_ratio() - initial_ratio).abs() < 1e-9);
}

#[test]
fn test_image_item_opacity_clamping() {
    let mut img = ImageItem::new(
        Uuid::new_v4(),
        "hash_op",
        Rect::new(0.0, 0.0, 50.0, 50.0),
        50,
        50,
    );

    img.set_opacity(0.42);
    assert!((img.opacity - 0.42).abs() < 1e-9);

    img.set_opacity(-0.5);
    assert_eq!(img.opacity, 0.0);

    img.set_opacity(1.8);
    assert_eq!(img.opacity, 1.0);

    // NaN should not corrupt existing valid opacity
    img.set_opacity(f64::NAN);
    assert_eq!(img.opacity, 1.0);
}

#[test]
fn test_image_item_serde_roundtrip() {
    let original = ImageItem {
        id: Uuid::new_v4(),
        page_id: Uuid::new_v4(),
        asset_hash: "deadbeefcafebabe1234567890abcdef".to_string(),
        bbox: Rect::new(12.5, 34.75, 560.125, 780.5),
        z_index: 42,
        opacity: 0.85,
        original_width: 1920,
        original_height: 1080,
    };

    let json = serde_json::to_string(&original).expect("serialize ImageItem");
    let recovered: ImageItem = serde_json::from_str(&json).expect("deserialize ImageItem");
    assert_eq!(original, recovered);

    let item = Item {
        id: 777,
        z_index: original.z_index as u64,
        kind: ItemKind::Image(original.clone()),
    };
    let item_json = serde_json::to_string(&item).expect("serialize Item");
    let recovered_item: Item = serde_json::from_str(&item_json).expect("deserialize Item");
    assert_eq!(item.id, recovered_item.id);
    assert_eq!(item.z_index, recovered_item.z_index);
    assert_eq!(recovered_item.as_image().unwrap(), &original);
}

#[test]
fn test_polymorphic_item_methods() {
    let stroke_item = make_sample_stroke(1, 10, &[(10.0, 10.0), (50.0, 50.0)]);
    let image_item = make_sample_image(2, 20, 100.0, 100.0, 200.0, 150.0, "img_poly");

    // Bounding boxes
    assert!(stroke_item.bounding_box().is_some());
    assert_eq!(
        image_item.bounding_box(),
        Some(Rect::new(100.0, 100.0, 300.0, 250.0))
    );

    // Type queries
    assert!(stroke_item.as_stroke().is_some());
    assert!(stroke_item.as_image().is_none());
    assert!(image_item.as_image().is_some());
    assert!(image_item.as_stroke().is_none());

    // Polymorphic translate
    let mut mut_img = image_item.clone();
    mut_img.translate(10.0, 20.0);
    assert_eq!(
        mut_img.bounding_box().unwrap(),
        Rect::new(110.0, 120.0, 310.0, 270.0)
    );

    // Polymorphic scale
    let mut mut_stroke = stroke_item.clone();
    let old_stroke_bbox = mut_stroke.bounding_box().unwrap();
    mut_stroke.scale(2.0, 2.0, old_stroke_bbox.min_x, old_stroke_bbox.min_y);
    let new_stroke_bbox = mut_stroke.bounding_box().unwrap();
    assert!(new_stroke_bbox.width() > old_stroke_bbox.width());
}

#[test]
fn test_page_image_helpers() {
    let mut page = Page::new(0, PageBackground::Blank);
    let s = make_sample_stroke(1, 0, &[(0.0, 0.0), (10.0, 10.0)]);
    let img1 = make_sample_image(2, 1, 10.0, 10.0, 100.0, 100.0, "img1");
    let img2 = make_sample_image(3, 2, 120.0, 10.0, 100.0, 100.0, "img2");

    page.add_item(s);
    page.add_item(img1);
    page.add_item(img2);

    assert_eq!(page.items.len(), 3);
    assert_eq!(page.strokes().count(), 1);
    assert_eq!(page.images().count(), 2);
    assert_eq!(page.spatial_index.len(), 3);

    let removed = page.remove_item(2);
    assert!(removed.is_some());
    assert_eq!(page.items.len(), 2);
    assert_eq!(page.images().count(), 1);
    assert_eq!(page.spatial_index.len(), 2);
}

#[test]
fn test_spatial_index_mixed_culling_and_ordering() {
    let mut index = PageSpatialIndex::empty();

    let stroke = make_sample_stroke(10, 1, &[(10.0, 10.0), (30.0, 30.0)]);
    let img = make_sample_image(20, 2, 100.0, 100.0, 50.0, 50.0, "hash_spatial");

    index.insert(&stroke);
    index.insert(&img);
    assert_eq!(index.len(), 2);

    // Query covering only the stroke (stroke is roughly [9, 31])
    let q1 = index.locate_in_envelope(0.0, 0.0, 50.0, 50.0);
    assert_eq!(q1.len(), 1);
    assert_eq!(q1[0].item_id, 10);

    // Query covering only the image ([100, 150])
    let q2 = index.locate_in_envelope(80.0, 80.0, 160.0, 160.0);
    assert_eq!(q2.len(), 1);
    assert_eq!(q2[0].item_id, 20);

    // Query covering both: must be ordered (z_index, item_id) ascending
    let q3 = index.locate_in_envelope(0.0, 0.0, 200.0, 200.0);
    assert_eq!(q3.len(), 2);
    assert_eq!(q3[0].item_id, 10);
    assert_eq!(q3[0].z_index, 1);
    assert_eq!(q3[1].item_id, 20);
    assert_eq!(q3[1].z_index, 2);

    // Query disjoint whitespace: 100% culled
    let q4 = index.locate_in_envelope(500.0, 500.0, 600.0, 600.0);
    assert!(q4.is_empty());
}

#[test]
fn test_doc_command_add_image_undo_redo() {
    let mut doc = Document::new_notebook("AddImage Test");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let img = ImageItem::new(
        page_id,
        "add_undo_hash",
        Rect::new(50.0, 50.0, 250.0, 200.0),
        800,
        600,
    );
    let item_id = history.alloc_item_id();
    let cmd = DocCommand::add_image(page_id, item_id, 1, img);

    history.execute(cmd, &mut doc);
    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[0].spatial_index.len(), 1);
    assert_eq!(doc.pages[0].items[0].id, item_id);
    assert!(doc.pages[0].items[0].as_image().is_some());

    // Undo: image removed from items and spatial index
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 0);
    assert_eq!(doc.pages[0].spatial_index.len(), 0);

    // Redo: image restored to items and spatial index with preserved id
    assert!(history.redo(&mut doc));
    assert_eq!(doc.pages[0].items.len(), 1);
    assert_eq!(doc.pages[0].spatial_index.len(), 1);
    assert_eq!(doc.pages[0].items[0].id, item_id);
    assert!(doc.pages[0].items[0].as_image().is_some());
}

#[test]
fn test_doc_command_move_and_transform_mixed_items_undo_redo() {
    let mut doc = Document::new_notebook("Mixed Transform Test");
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let stroke = make_sample_stroke(2001, 0, &[(10.0, 10.0), (30.0, 30.0)]);
    let img = make_sample_image(2002, 1, 50.0, 50.0, 100.0, 100.0, "hash_mv_tx");

    doc.pages[0].items.push(stroke);
    doc.pages[0].items.push(img);
    doc.pages[0].rebuild_spatial_index();

    let orig_stroke_box = doc.pages[0].items[0].bounding_box().unwrap();
    let orig_img_box = doc.pages[0].items[1].bounding_box().unwrap();

    // 1. Move both items by (+20, -10)
    let move_cmd = DocCommand::MoveItems {
        page_id,
        item_ids: vec![2001, 2002],
        delta_x: 20.0,
        delta_y: -10.0,
    };
    history.execute(move_cmd, &mut doc);

    let moved_stroke_box = doc.pages[0].items[0].bounding_box().unwrap();
    let moved_img_box = doc.pages[0].items[1].bounding_box().unwrap();

    assert_eq!(moved_stroke_box.min_x, orig_stroke_box.min_x + 20.0);
    assert_eq!(moved_stroke_box.min_y, orig_stroke_box.min_y - 10.0);
    assert_eq!(moved_img_box.min_x, orig_img_box.min_x + 20.0);
    assert_eq!(moved_img_box.min_y, orig_img_box.min_y - 10.0);

    // 2. Transform (scale) both items by 2.0x around origin (0, 0)
    let transform_cmd = DocCommand::TransformItems {
        page_id,
        item_ids: vec![2001, 2002],
        origin_x: 0.0,
        origin_y: 0.0,
        scale_x: 2.0,
        scale_y: 2.0,
        rotation_rad: 0.0,
    };
    history.execute(transform_cmd, &mut doc);

    let scaled_img_box = doc.pages[0].items[1].bounding_box().unwrap();
    assert_eq!(scaled_img_box.width(), orig_img_box.width() * 2.0);
    assert_eq!(scaled_img_box.height(), orig_img_box.height() * 2.0);

    // Undo Transform
    assert!(history.undo(&mut doc));
    assert_eq!(doc.pages[0].items[1].bounding_box().unwrap(), moved_img_box);

    // Undo Move
    assert!(history.undo(&mut doc));
    assert_eq!(
        doc.pages[0].items[0].bounding_box().unwrap(),
        orig_stroke_box
    );
    assert_eq!(doc.pages[0].items[1].bounding_box().unwrap(), orig_img_box);
}

#[test]
fn test_image_item_with_id_nan_and_infinite_opacity() {
    let page_id = Uuid::new_v4();
    let bbox = Rect::new(0.0, 0.0, 100.0, 100.0);

    let img_nan = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        "hash_nan",
        bbox,
        1,
        f64::NAN,
        100,
        100,
    );
    assert_eq!(img_nan.opacity, 1.0);

    let img_inf = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        "hash_inf",
        bbox,
        1,
        f64::INFINITY,
        100,
        100,
    );
    assert_eq!(img_inf.opacity, 1.0);

    let img_neg_inf = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        "hash_neg_inf",
        bbox,
        1,
        f64::NEG_INFINITY,
        100,
        100,
    );
    assert_eq!(img_neg_inf.opacity, 1.0);

    let img_valid = ImageItem::with_id(
        Uuid::new_v4(),
        page_id,
        "hash_valid",
        bbox,
        1,
        0.75,
        100,
        100,
    );
    assert_eq!(img_valid.opacity, 0.75);
}

#[test]
fn test_image_item_aspect_ratio_zero_and_degenerate_dimensions() {
    let page_id = Uuid::new_v4();

    // 1. original_width == 0 but original_height > 0
    let img_zero_w = ImageItem::new(
        page_id,
        "hash_zero_w",
        Rect::new(0.0, 0.0, 200.0, 100.0),
        0,
        100,
    );
    assert_eq!(img_zero_w.aspect_ratio(), 2.0); // falls back to normalized bbox ratio: 200 / 100

    // 2. Both original dimensions 0, valid bbox
    let img_zero_dims = ImageItem::new(
        page_id,
        "hash_zero_dims",
        Rect::new(10.0, 10.0, 110.0, 60.0),
        0,
        0,
    );
    assert_eq!(img_zero_dims.aspect_ratio(), 2.0); // 100 / 50

    // 3. Degenerate zero-size bbox and zero original dimensions
    let img_point_bbox = ImageItem::new(
        page_id,
        "hash_point",
        Rect::new(50.0, 50.0, 50.0, 50.0),
        0,
        0,
    );
    assert_eq!(img_point_bbox.aspect_ratio(), 1.0); // safe fallback to 1.0

    // 4. Inverted bbox (min > max)
    let img_inverted = ImageItem::new(
        page_id,
        "hash_inv",
        Rect::new(100.0, 100.0, 50.0, 50.0),
        0,
        0,
    );
    assert_eq!(img_inverted.aspect_ratio(), 1.0); // normalized width/height == 50/50 => 1.0
}
