use rusqlite::Connection;
use std::f64::consts::{PI, SQRT_2};
use tempfile::NamedTempFile;
use uninote_core::{CommandHistory, DocCommand, Document, DocumentFormat, PageBackground};
use uninote_storage::DocumentStore;
use uninote_stroke::geometry::catmull_rom_to_bezier;
use uninote_stroke::types::{
    DeviceKind, InputPhase, InputSample, PipelineMode, Rect, Stroke, StrokePoint, StrokeStyle,
};

fn create_dyadic_stroke(id: u64, mode: PipelineMode, point_count: usize, offset_x: f64) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    let mut raw_samples = Vec::with_capacity(point_count);

    for i in 0..point_count {
        let t = i as f64;
        let x = offset_x + t * 4.0;
        let y = 100.0 + t * 2.5;
        let pressure = 0.5;
        let width = 2.0;

        points.push(StrokePoint {
            x,
            y,
            pressure,
            width,
            timestamp_ns: (i * 10_000_000) as u64,
        });

        raw_samples.push(InputSample::new(
            (x, y),
            Some(pressure),
            (i * 10_000_000) as u64,
            DeviceKind::Stylus,
            if i == 0 {
                InputPhase::Down
            } else if i == point_count - 1 {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        ));
    }

    let is_dot = point_count == 1;
    let mut stroke = Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples,
        is_dot,
        mode,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

fn create_transcendental_stroke(
    id: u64,
    mode: PipelineMode,
    point_count: usize,
    offset_x: f64,
) -> Stroke {
    let mut points = Vec::with_capacity(point_count);
    let mut raw_samples = Vec::with_capacity(point_count);

    for i in 0..point_count {
        let t = i as f64;
        let x = offset_x + t * 5.0;
        let y = 100.0 + (t * 0.3).sin() * 30.0;
        let pressure = 0.3 + 0.5 * ((t * 0.1).cos().abs());
        let width = 2.0 + pressure * 2.0;

        points.push(StrokePoint {
            x,
            y,
            pressure,
            width,
            timestamp_ns: (i * 10_000_000) as u64,
        });

        raw_samples.push(InputSample::new(
            (x, y),
            Some(pressure),
            (i * 10_000_000) as u64,
            DeviceKind::Stylus,
            if i == 0 {
                InputPhase::Down
            } else if i == point_count - 1 {
                InputPhase::Up
            } else {
                InputPhase::Move
            },
        ));
    }

    let is_dot = point_count == 1;
    let mut stroke = Stroke {
        id,
        style: StrokeStyle::default(),
        points,
        raw_samples,
        is_dot,
        mode,
        cached_bbox: None,
        cached_bezier: Vec::new(),
    };
    stroke.recompute_cache();
    stroke
}

// =========================================================================
// 1. SERDE SKIP & RAW SQLITE INSPECTION
// =========================================================================

#[test]
fn test_sqlite_raw_storage_omits_cached_fields() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let mut store = DocumentStore::open_or_create(temp_file.path()).expect("Failed to open DB");

    let mut doc = Document::new_notebook("Serde Skip Test");

    let stroke1 = create_dyadic_stroke(1, PipelineMode::CatmullRomSpline, 20, 50.0);
    let stroke2 = create_dyadic_stroke(2, PipelineMode::Baseline, 10, 200.0);
    let stroke3 = create_dyadic_stroke(3, PipelineMode::OneEuroFiltered, 15, 350.0);
    let stroke4 = create_dyadic_stroke(4, PipelineMode::Baseline, 1, 500.0); // dot

    assert!(stroke1.cached_bbox.is_some());
    assert!(!stroke1.cached_bezier.is_empty());
    assert!(stroke2.cached_bbox.is_some());
    assert!(stroke3.cached_bbox.is_some());
    assert!(!stroke3.cached_bezier.is_empty());

    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(100);
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke1.clone(),
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke2.clone(),
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke3.clone(),
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: stroke4.clone(),
        },
        &mut doc,
    );

    // Save document to SQLite
    store.save_document(&doc).expect("save_document failed");

    // Open raw rusqlite connection to inspect database content directly
    let raw_conn = Connection::open(temp_file.path()).expect("Failed to open raw sqlite conn");

    let mut stmt = raw_conn
        .prepare("SELECT id, bounding_box, data_json FROM items ORDER BY id ASC")
        .expect("prepare failed");

    let rows: Vec<(i64, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .expect("query_map failed")
        .collect::<Result<Vec<_>, _>>()
        .expect("collect failed");

    assert_eq!(rows.len(), 4, "Must have exactly 4 items stored");

    for (id, bbox_str, data_json) in &rows {
        // 1. Verify bounding_box column is valid JSON Rect
        assert!(
            !bbox_str.is_empty(),
            "Item {} bounding_box column must not be empty",
            id
        );
        let parsed_bbox: Rect = serde_json::from_str(bbox_str)
            .unwrap_or_else(|e| panic!("Item {} bbox_str must parse to Rect: {}", id, e));
        assert!(parsed_bbox.width() >= 0.0);
        assert!(parsed_bbox.height() >= 0.0);

        // 2. Verify data_json strictly does NOT contain cache field keys
        assert!(
            !data_json.contains("cached_bbox"),
            "Item {} data_json must not contain 'cached_bbox' key! Found in: {}",
            id,
            data_json
        );
        assert!(
            !data_json.contains("cached_bezier"),
            "Item {} data_json must not contain 'cached_bezier' key! Found in: {}",
            id,
            data_json
        );

        // 3. Parse as generic json Value
        let v: serde_json::Value = serde_json::from_str(data_json).expect("Must parse JSON");
        assert!(
            v.get("cached_bbox").is_none(),
            "JSON Value must not have cached_bbox"
        );
        assert!(
            v.get("cached_bezier").is_none(),
            "JSON Value must not have cached_bezier"
        );
    }

    // 4. Load document back through DocumentStore
    let loaded_doc = store.load_document().expect("load_document failed");
    assert_eq!(loaded_doc.pages[0].items.len(), 4);

    for (orig_item, loaded_item) in doc.pages[0]
        .items
        .iter()
        .zip(loaded_doc.pages[0].items.iter())
    {
        assert_eq!(orig_item.id, loaded_item.id);
        assert_eq!(orig_item.z_index, loaded_item.z_index);

        let orig_s = orig_item.as_stroke().expect("orig stroke");
        let loaded_s = loaded_item.as_stroke().expect("loaded stroke");

        // Semantic equality passes
        assert_eq!(
            orig_s, loaded_s,
            "Loaded stroke must be equal to original under semantic PartialEq"
        );

        // In M3: loaded stroke has rehydrated cache fields via ensure_cache() on load
        assert!(
            loaded_s.cached_bbox.is_some(),
            "Freshly loaded stroke must have rehydrated cached_bbox via ensure_cache()"
        );
        assert_eq!(
            loaded_s.cached_bbox, orig_s.cached_bbox,
            "Rehydrated cached_bbox must match original exactly"
        );
        assert_eq!(
            loaded_s.cached_bezier, orig_s.cached_bezier,
            "Rehydrated cached_bezier must match original exactly"
        );
    }
}

// =========================================================================
// 2. BACKWARD COMPATIBILITY: LEGACY SCHEMA DESERIALIZATION
// =========================================================================

#[test]
fn test_sqlite_backward_compatibility_legacy_v1_payload() {
    let temp_file = NamedTempFile::new().expect("Failed to create temp file");
    let mut conn = Connection::open(temp_file.path()).expect("Open connection failed");
    uninote_storage::schema::initialize_schema(&mut conn).expect("initialize_schema failed");

    // Manually insert legacy metadata, page, and item with raw JSON containing NO cache fields
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('doc_id', '11111111-1111-1111-1111-111111111111')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('title', 'Legacy V1 Doc')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('version', '1')",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO metadata (key, value) VALUES ('document_format', 'notebook')",
        [],
    )
    .unwrap();

    let page_uuid = "22222222-2222-2222-2222-222222222222";
    conn.execute(
        "INSERT INTO pages (id, page_index, width, height, background_type, background_spacing, background_extra)
         VALUES (?1, 0, 595.28, 841.89, 'blank', 0.0, '')",
        [page_uuid],
    ).unwrap();

    // Legacy JSON payload as produced by v0.1 / v0.2
    let legacy_json = r#"{
        "id": 999,
        "style": {
            "color": [0.1, 0.1, 0.12, 1.0],
            "base_width": 2.8,
            "min_width_factor": 0.4,
            "max_width_factor": 1.8
        },
        "points": [
            {"x": 10.0, "y": 20.0, "pressure": 0.5, "width": 2.5, "timestamp_ns": 100},
            {"x": 30.0, "y": 40.0, "pressure": 0.6, "width": 2.7, "timestamp_ns": 200},
            {"x": 60.0, "y": 70.0, "pressure": 0.7, "width": 2.9, "timestamp_ns": 300}
        ],
        "raw_samples": [],
        "is_dot": false,
        "mode": "CatmullRomSpline"
    }"#;

    conn.execute(
        "INSERT INTO items (id, page_id, z_index, item_type, bounding_box, data_json)
         VALUES (1, ?1, 1, 'stroke', '{\"min_x\": 8.75, \"min_y\": 18.75, \"max_x\": 61.45, \"max_y\": 71.45}', ?2)",
        [page_uuid, legacy_json],
    ).unwrap();

    drop(conn);

    // Open via DocumentStore
    let store = DocumentStore::open_or_create(temp_file.path()).unwrap();
    let doc = store
        .load_document()
        .expect("Must load legacy V1 document without error");

    assert_eq!(doc.title, "Legacy V1 Doc");
    assert_eq!(doc.format, DocumentFormat::Notebook);
    assert_eq!(doc.pages[0].items.len(), 1);

    let s = doc.pages[0].items[0].as_stroke().expect("stroke");
    assert_eq!(s.id, 999);
    assert_eq!(s.points.len(), 3);
    // In M3: load_document automatically rehydrates cache via ensure_cache()
    assert!(s.cached_bbox.is_some());
    assert_eq!(s.cached_bezier.len(), 2);
    assert_eq!(s.bounding_box(), s.compute_bounding_box());
}

// =========================================================================
// 3. 1,000 UNDO/REDO CYCLES WITH DocCommand::MoveItems (DYADIC INVARIANCE)
// =========================================================================

#[test]
fn test_move_items_1000_undo_redo_cycles_cache_synchronization_dyadic() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(200);

    // Add 3 different dyadic strokes
    let s1 = create_dyadic_stroke(10, PipelineMode::CatmullRomSpline, 25, 100.0);
    let s2 = create_dyadic_stroke(20, PipelineMode::Baseline, 15, 300.0);
    let s3 = create_dyadic_stroke(30, PipelineMode::OneEuroFiltered, 20, 500.0);

    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s1.clone(),
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s2.clone(),
        },
        &mut doc,
    );
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s3.clone(),
        },
        &mut doc,
    );

    let id1 = doc.pages[0].items[0].id;
    let id2 = doc.pages[0].items[1].id;
    let id3 = doc.pages[0].items[2].id;

    // Snapshot initial state
    let orig_bbox1 = s1.cached_bbox.unwrap();
    let orig_bezier1 = s1.cached_bezier.clone();
    let orig_pts1 = s1.points.clone();

    let orig_bbox2 = s2.cached_bbox.unwrap();
    let orig_pts2 = s2.points.clone();

    let orig_bbox3 = s3.cached_bbox.unwrap();
    let orig_bezier3 = s3.cached_bezier.clone();
    let orig_pts3 = s3.points.clone();

    let dx = 32.5;
    let dy = -16.25;

    // Execute MoveItems for all 3 items simultaneously
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![id1, id2, id3],
            delta_x: dx,
            delta_y: dy,
        },
        &mut doc,
    );

    // Run 1,000 undo/redo cycles
    for cycle in 0..1000 {
        let undone = history.undo(&mut doc);
        assert!(undone, "Undo failed at cycle {}", cycle);

        // Spot-check cache integrity every 200 cycles in undone state
        if cycle % 200 == 0 {
            for item in &doc.pages[0].items {
                let s = item.as_stroke().expect("stroke");
                let cached = s.cached_bbox.unwrap();
                let comp = s.compute_bounding_box().unwrap();
                assert!((cached.min_x - comp.min_x).abs() < 1e-12);
                assert!((cached.max_x - comp.max_x).abs() < 1e-12);
                assert!((cached.min_y - comp.min_y).abs() < 1e-12);
                assert!((cached.max_y - comp.max_y).abs() < 1e-12);

                if matches!(
                    s.mode,
                    PipelineMode::CatmullRomSpline | PipelineMode::OneEuroFiltered
                ) {
                    for (c_seg, r_seg) in s
                        .cached_bezier
                        .iter()
                        .zip(catmull_rom_to_bezier(&s.points).iter())
                    {
                        assert!((c_seg.p0.0 - r_seg.p0.0).abs() < 1e-12);
                        assert!((c_seg.p0.1 - r_seg.p0.1).abs() < 1e-12);
                        assert!((c_seg.cp1.0 - r_seg.cp1.0).abs() < 1e-12);
                        assert!((c_seg.cp1.1 - r_seg.cp1.1).abs() < 1e-12);
                        assert!((c_seg.cp2.0 - r_seg.cp2.0).abs() < 1e-12);
                        assert!((c_seg.cp2.1 - r_seg.cp2.1).abs() < 1e-12);
                        assert!((c_seg.p1.0 - r_seg.p1.0).abs() < 1e-12);
                        assert!((c_seg.p1.1 - r_seg.p1.1).abs() < 1e-12);
                    }
                }
            }
        }

        let redone = history.redo(&mut doc);
        assert!(redone, "Redo failed at cycle {}", cycle);
    }

    // Now in REDONE state: verify all items are at (orig + delta) with exact cached geometry
    {
        let it1 = &doc.pages[0].items[0];
        let s1_redone = it1.as_stroke().expect("stroke");
        let expected_bbox1 = orig_bbox1.translate(dx, dy);
        assert!((s1_redone.cached_bbox.unwrap().min_x - expected_bbox1.min_x).abs() < 1e-13);
        assert!((s1_redone.cached_bbox.unwrap().max_x - expected_bbox1.max_x).abs() < 1e-13);
        assert_eq!(s1_redone.cached_bezier.len(), orig_bezier1.len());

        for (curr_p, orig_p) in s1_redone.points.iter().zip(orig_pts1.iter()) {
            assert!((curr_p.x - (orig_p.x + dx)).abs() < 1e-13);
            assert!((curr_p.y - (orig_p.y + dy)).abs() < 1e-13);
        }
    }

    // Undo once more to return to UNDONE state
    assert!(history.undo(&mut doc));

    // Now in UNDONE state: verify strictly zero drift (< 1e-13) against initial state
    {
        let it1 = &doc.pages[0].items[0];
        let s1_undone = it1.as_stroke().expect("stroke");
        let bbox = s1_undone.cached_bbox.unwrap();
        assert!((bbox.min_x - orig_bbox1.min_x).abs() < 1e-13);
        assert!((bbox.max_x - orig_bbox1.max_x).abs() < 1e-13);
        assert!((bbox.min_y - orig_bbox1.min_y).abs() < 1e-13);
        assert!((bbox.max_y - orig_bbox1.max_y).abs() < 1e-13);

        for (curr_p, orig_p) in s1_undone.points.iter().zip(orig_pts1.iter()) {
            assert!((curr_p.x - orig_p.x).abs() < 1e-13);
            assert!((curr_p.y - orig_p.y).abs() < 1e-13);
        }
        for (curr_b, orig_b) in s1_undone.cached_bezier.iter().zip(orig_bezier1.iter()) {
            assert!((curr_b.p0.0 - orig_b.p0.0).abs() < 1e-13);
            assert!((curr_b.p0.1 - orig_b.p0.1).abs() < 1e-13);
            assert!((curr_b.cp1.0 - orig_b.cp1.0).abs() < 1e-13);
            assert!((curr_b.cp1.1 - orig_b.cp1.1).abs() < 1e-13);
            assert!((curr_b.cp2.0 - orig_b.cp2.0).abs() < 1e-13);
            assert!((curr_b.cp2.1 - orig_b.cp2.1).abs() < 1e-13);
            assert!((curr_b.p1.0 - orig_b.p1.0).abs() < 1e-13);
            assert!((curr_b.p1.1 - orig_b.p1.1).abs() < 1e-13);
        }
    }

    {
        let it2 = &doc.pages[0].items[1];
        let s2_undone = it2.as_stroke().expect("stroke");
        let bbox = s2_undone.cached_bbox.unwrap();
        assert!((bbox.min_x - orig_bbox2.min_x).abs() < 1e-13);
        for (curr_p, orig_p) in s2_undone.points.iter().zip(orig_pts2.iter()) {
            assert!((curr_p.x - orig_p.x).abs() < 1e-13);
            assert!((curr_p.y - orig_p.y).abs() < 1e-13);
        }
    }

    {
        let it3 = &doc.pages[0].items[2];
        let s3_undone = it3.as_stroke().expect("stroke");
        let bbox = s3_undone.cached_bbox.unwrap();
        assert!((bbox.min_x - orig_bbox3.min_x).abs() < 1e-13);
        for (curr_p, orig_p) in s3_undone.points.iter().zip(orig_pts3.iter()) {
            assert!((curr_p.x - orig_p.x).abs() < 1e-13);
            assert!((curr_p.y - orig_p.y).abs() < 1e-13);
        }
        for (curr_b, orig_b) in s3_undone.cached_bezier.iter().zip(orig_bezier3.iter()) {
            assert!((curr_b.p0.0 - orig_b.p0.0).abs() < 1e-13);
            assert!((curr_b.p0.1 - orig_b.p0.1).abs() < 1e-13);
        }
    }
}

// =========================================================================
// 4. TRANSCENDENTAL / IRRATIONAL DELTA 1,000 UNDO/REDO CYCLES
// =========================================================================

#[test]
fn test_move_items_1000_undo_redo_cycles_transcendental_offsets() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(100);

    let stroke = create_transcendental_stroke(55, PipelineMode::CatmullRomSpline, 30, 150.0);
    let orig_bbox = stroke.cached_bbox.unwrap();
    let orig_bezier = stroke.cached_bezier.clone();
    let orig_points = stroke.points.clone();

    history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    let item_id = doc.pages[0].items[0].id;

    let dx = PI;
    let dy = SQRT_2;

    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![item_id],
            delta_x: dx,
            delta_y: dy,
        },
        &mut doc,
    );

    // Run 1,000 undo/redo cycles with irrational translation
    for _ in 0..1000 {
        assert!(history.undo(&mut doc));
        assert!(history.redo(&mut doc));
    }

    // Return to undone state
    assert!(history.undo(&mut doc));

    let s = doc.pages[0].items[0].as_stroke().expect("stroke");
    let bbox = s.cached_bbox.unwrap();

    // Verify IEEE-754 precision bound (< 1e-11)
    assert!(
        (bbox.min_x - orig_bbox.min_x).abs() < 1e-11,
        "BBox min_x drift exceeded 1e-11: diff={:e}",
        (bbox.min_x - orig_bbox.min_x).abs()
    );
    assert!(
        (bbox.max_x - orig_bbox.max_x).abs() < 1e-11,
        "BBox max_x drift exceeded 1e-11: diff={:e}",
        (bbox.max_x - orig_bbox.max_x).abs()
    );

    for (p_curr, p_orig) in s.points.iter().zip(orig_points.iter()) {
        assert!((p_curr.x - p_orig.x).abs() < 1e-11);
        assert!((p_curr.y - p_orig.y).abs() < 1e-11);
    }

    for (b_curr, b_orig) in s.cached_bezier.iter().zip(orig_bezier.iter()) {
        assert!((b_curr.p0.0 - b_orig.p0.0).abs() < 1e-11);
        assert!((b_curr.p0.1 - b_orig.p0.1).abs() < 1e-11);
        assert!((b_curr.cp1.0 - b_orig.cp1.0).abs() < 1e-11);
        assert!((b_curr.cp1.1 - b_orig.cp1.1).abs() < 1e-11);
        assert!((b_curr.cp2.0 - b_orig.cp2.0).abs() < 1e-11);
        assert!((b_curr.cp2.1 - b_orig.cp2.1).abs() < 1e-11);
        assert!((b_curr.p1.0 - b_orig.p1.0).abs() < 1e-11);
        assert!((b_curr.p1.1 - b_orig.p1.1).abs() < 1e-11);
    }
}

// =========================================================================
// 5. ADVERSARIAL EDGE CASES FOR MoveItems
// =========================================================================

#[test]
fn test_move_items_adversarial_inputs_no_cache_corruption() {
    let mut doc = Document::new_whiteboard(PageBackground::Blank);
    let page_id = doc.pages[0].id;
    let mut history = CommandHistory::new(50);

    let stroke = create_transcendental_stroke(88, PipelineMode::CatmullRomSpline, 10, 100.0);
    let orig_bbox = stroke.cached_bbox.unwrap();
    let orig_pts = stroke.points.clone();

    history.execute(DocCommand::AddStroke { page_id, stroke }, &mut doc);
    let item_id = doc.pages[0].items[0].id;

    // 1. MoveItems with NaN delta: must be rejected with no state mutation
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![item_id],
            delta_x: f64::NAN,
            delta_y: 10.0,
        },
        &mut doc,
    );

    let s_nan = doc.pages[0].items[0].as_stroke().expect("stroke");
    assert!(s_nan.points[0].x.is_finite());
    assert_eq!(s_nan.cached_bbox.unwrap(), orig_bbox);
    assert_eq!(s_nan.points, orig_pts);

    // 2. MoveItems with Infinity delta: must be rejected with no state mutation
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![item_id],
            delta_x: 10.0,
            delta_y: f64::INFINITY,
        },
        &mut doc,
    );

    let s_inf = doc.pages[0].items[0].as_stroke().expect("stroke");
    assert!(s_inf.points[0].y.is_finite());
    assert_eq!(s_inf.cached_bbox.unwrap(), orig_bbox);
    assert_eq!(s_inf.points, orig_pts);

    // 3. MoveItems with empty item_ids: no-op, no corruption
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![],
            delta_x: 100.0,
            delta_y: 100.0,
        },
        &mut doc,
    );
    let s_empty = doc.pages[0].items[0].as_stroke().expect("stroke");
    assert_eq!(s_empty.cached_bbox.unwrap(), orig_bbox);
    assert_eq!(s_empty.points, orig_pts);

    // 4. MoveItems with nonexistent item_id: no-op
    history.execute(
        DocCommand::MoveItems {
            page_id,
            item_ids: vec![999999],
            delta_x: 100.0,
            delta_y: 100.0,
        },
        &mut doc,
    );
    let s_nonexistent = doc.pages[0].items[0].as_stroke().expect("stroke");
    assert_eq!(s_nonexistent.cached_bbox.unwrap(), orig_bbox);
    assert_eq!(s_nonexistent.points, orig_pts);
}
