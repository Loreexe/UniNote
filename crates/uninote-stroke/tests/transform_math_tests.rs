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
fn test_stroke_scale_and_rotate_pure_math() {
    let mut stroke = make_sample_stroke(1, (10.0, 10.0));
    let initial_bbox = stroke.bounding_box().expect("bbox must exist");

    // Scala 2.0x con origine al centro del bbox
    let center_x = (initial_bbox.min_x + initial_bbox.max_x) / 2.0;
    let center_y = (initial_bbox.min_y + initial_bbox.max_y) / 2.0;
    stroke.scale(2.0, 2.0, center_x, center_y);
    let scaled_bbox = stroke.bounding_box().expect("scaled bbox");
    assert!((scaled_bbox.width() - initial_bbox.width() * 2.0).abs() < 1e-4);

    // Rotazione di 360 gradi (TAU) deve riportare i punti alla posizione iniziale
    let before_rot = stroke.clone();
    stroke.rotate(std::f64::consts::TAU, center_x, center_y);
    for (p1, p2) in stroke.points.iter().zip(before_rot.points.iter()) {
        assert!((p1.x - p2.x).abs() < 1e-5);
        assert!((p1.y - p2.y).abs() < 1e-5);
    }
}

#[test]
fn test_stroke_set_color_and_base_width() {
    let mut stroke = make_sample_stroke(2, (50.0, 50.0));
    assert_eq!(stroke.style.color, (0.1, 0.1, 0.12, 1.0));

    // Ricolorazione
    let new_color = (0.8, 0.2, 0.2, 1.0);
    stroke.set_color(new_color);
    assert_eq!(stroke.style.color, new_color);

    // Modifica spessore base
    let old_base = stroke.style.base_width;
    stroke.set_base_width(old_base * 2.0);
    assert_eq!(stroke.style.base_width, old_base * 2.0);
    assert!((stroke.points[0].width - 6.0).abs() < 1e-5);
}
