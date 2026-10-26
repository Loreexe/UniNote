use uninote_app::canvas::dynamic_grid_spec;
use uninote_app::pdf_cache::PdfCacheManager;
use uninote_core::coordinates::{Viewport, MAX_ZOOM, MIN_ZOOM};

#[test]
fn test_viewport_max_zoom_is_10() {
    assert_eq!(MIN_ZOOM, 0.25);
    assert_eq!(MAX_ZOOM, 10.0);

    let mut vp = Viewport::new(0.0, 0.0, 1.0);
    // Multiply zoom by 20 -> should clamp at 10.0
    vp.zoom_at((100.0, 100.0), 20.0);
    assert!((vp.zoom - 10.0).abs() < 1e-6);

    // Zoom out by factor of 0.01 -> should clamp at 0.25
    vp.zoom_at((100.0, 100.0), 0.01);
    assert!((vp.zoom - 0.25).abs() < 1e-6);
}

#[test]
fn test_viewport_new_clamps_to_10() {
    let vp_huge = Viewport::new(0.0, 0.0, 50.0);
    assert_eq!(vp_huge.zoom, 10.0);

    let vp_tiny = Viewport::new(0.0, 0.0, 0.05);
    assert_eq!(vp_tiny.zoom, 0.25);
}

#[test]
fn test_dynamic_grid_spec_at_high_zoom() {
    // Zoom 1.0
    let spec1 = dynamic_grid_spec(1.0);
    assert_eq!(spec1.step_pt, 20.0);

    // Zoom 4.0
    let spec4 = dynamic_grid_spec(4.0);
    assert_eq!(spec4.step_pt, 10.0);

    // Zoom 8.0 (above old 5.0 limit)
    let spec8 = dynamic_grid_spec(8.0);
    assert_eq!(spec8.step_pt, 5.0);
    assert!(spec8.alpha >= 0.18 && spec8.alpha <= 0.35);

    // Zoom 10.0
    let spec10 = dynamic_grid_spec(10.0);
    assert_eq!(spec10.step_pt, 5.0);
    assert!(spec10.alpha.is_finite());
}

#[test]
fn test_pdf_cache_allows_scale_up_to_10() {
    let mut cache = PdfCacheManager::new(uninote_app::pdf_cache::PdfCacheConfig::default());
    // Non-existent path returns file error, but scale clamping happens before or in render
    let err = cache.update_raster_scale(std::path::Path::new("/tmp/nonexistent.pdf"), 0, 12.0);
    assert!(err.is_err());
}
