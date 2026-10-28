use uninote_core::command::{CommandHistory, DocCommand};
use uninote_core::model::Document;
use uninote_stroke::types::{PipelineMode, Stroke, StrokePoint, StrokeStyle};

fn make_sample_stroke(id: u64, origin: (f64, f64)) -> Stroke {
    let mut s = Stroke::new(id, StrokeStyle::default(), PipelineMode::CatmullRomSpline);
    s.points.push(StrokePoint {
        x: origin.0,
        y: origin.1,
        pressure: 0.5,
        width: 3.0,
        timestamp_ns: 1000,
    });
    s.points.push(StrokePoint {
        x: origin.0 + 10.0,
        y: origin.1 + 10.0,
        pressure: 0.5,
        width: 3.0,
        timestamp_ns: 2000,
    });
    s.recompute_cache();
    s
}

#[test]
fn test_doc_command_transform_items_undo_redo_symmetry() {
    let mut doc = Document::new_notebook("Test Transform");
    let page_id = doc.pages[0].id;

    let s1 = make_sample_stroke(101, (100.0, 100.0));
    let mut history = CommandHistory::new(50);
    history.execute(
        DocCommand::AddStroke {
            page_id,
            stroke: s1,
        },
        &mut doc,
    );

    let item_id = doc.pages[0].items[0].id;
    let initial_pts: Vec<(f64, f64)> = doc.pages[0].items[0]
        .as_stroke()
        .unwrap()
        .points
        .iter()
        .map(|p| (p.x, p.y))
        .collect();

    // Esegui trasformazione: scala 1.5x e rotazione 90 deg (PI/2)
    let transform_cmd = DocCommand::TransformItems {
        page_id,
        item_ids: vec![item_id],
        origin_x: 100.0,
        origin_y: 100.0,
        scale_x: 1.5,
        scale_y: 1.5,
        rotation_rad: std::f64::consts::FRAC_PI_2,
    };
    history.execute(transform_cmd, &mut doc);

    let transformed_pts: Vec<(f64, f64)> = doc.pages[0].items[0]
        .as_stroke()
        .unwrap()
        .points
        .iter()
        .map(|p| (p.x, p.y))
        .collect();
    assert_ne!(initial_pts, transformed_pts);

    // Verifica che l'indice spaziale sia coerente
    assert_eq!(doc.pages[0].spatial_index.len(), 1);

    // UNDO: deve ripristinare le coordinate iniziali
    assert!(history.undo(&mut doc));
    let undone_pts: Vec<(f64, f64)> = doc.pages[0].items[0]
        .as_stroke()
        .unwrap()
        .points
        .iter()
        .map(|p| (p.x, p.y))
        .collect();
    for (p1, p2) in initial_pts.iter().zip(undone_pts.iter()) {
        assert!((p1.0 - p2.0).abs() < 1e-4);
        assert!((p1.1 - p2.1).abs() < 1e-4);
    }

    // REDO: deve riapplicare la trasformazione
    assert!(history.redo(&mut doc));
    let redone_pts: Vec<(f64, f64)> = doc.pages[0].items[0]
        .as_stroke()
        .unwrap()
        .points
        .iter()
        .map(|p| (p.x, p.y))
        .collect();
    for (p1, p2) in transformed_pts.iter().zip(redone_pts.iter()) {
        assert!((p1.0 - p2.0).abs() < 1e-4);
        assert!((p1.1 - p2.1).abs() < 1e-4);
    }
}

#[test]
fn test_doc_command_modify_items_style_undo_redo() {
    let mut doc = Document::new_notebook("Test Modify Style");
    let page_id = doc.pages[0].id;

    let s = make_sample_stroke(201, (200.0, 200.0));
    let old_style = s.style.clone();
    let mut new_style = old_style.clone();
    new_style.color = (0.2, 0.7, 0.3, 0.9);
    new_style.base_width = 5.5;

    let mut history = CommandHistory::new(50);
    history.execute(DocCommand::AddStroke { page_id, stroke: s }, &mut doc);

    let item_id = doc.pages[0].items[0].id;
    let modify_cmd = DocCommand::ModifyItemsStyle {
        page_id,
        items_style: vec![(item_id, old_style.clone(), new_style.clone())],
    };

    history.execute(modify_cmd, &mut doc);
    let cur_style = doc.pages[0].items[0].as_stroke().unwrap().style.clone();
    assert_eq!(cur_style.color, new_style.color);
    assert_eq!(cur_style.base_width, new_style.base_width);

    // UNDO: ripristina old_style
    assert!(history.undo(&mut doc));
    let cur_style_undone = doc.pages[0].items[0].as_stroke().unwrap().style.clone();
    assert_eq!(cur_style_undone.color, old_style.color);
    assert_eq!(cur_style_undone.base_width, old_style.base_width);

    // REDO: ripristina new_style
    assert!(history.redo(&mut doc));
    let cur_style_redone = doc.pages[0].items[0].as_stroke().unwrap().style.clone();
    assert_eq!(cur_style_redone.color, new_style.color);
    assert_eq!(cur_style_redone.base_width, new_style.base_width);
}
