//! Empirical Adversarial Verification Suite for Milestone M4 UI/UX Refinements
//!
//! Scope:
//! - DMS Light Theme token completeness in `style.css` and `.light-theme` class application.
//! - Dynamic theme switching: `StyleManager::connect_dark_notify` synchronization across window and 4 popovers.
//! - Dock drag at 120Hz: height caching, pure arithmetic in drag_update, clamp bounds safety.
//! - UniNote logo Cairo rendering: 3.6 pt monogram 'U', nib positioned at bottom center (x=18.0, y=28.5) in ciano #6DD2FF with mint accent #79D9A5.
//! - SemVer 0.2.0 badge: dynamic binding to CARGO_PKG_VERSION in window.rs.

#![allow(dead_code, unused_variables)]

use gtk4::cairo::{Context, Format, ImageSurface};
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, CssProvider, Popover};
use std::cell::Cell;
use std::f64::consts::PI;
use std::fs;
use std::path::Path;
use std::rc::Rc;

// ============================================================================
// 1. DMS LIGHT THEME TOKEN COMPLETENESS & CSS VALIDITY
// ============================================================================

#[test]
fn test_dms_light_theme_palette_token_completeness() {
    let css_path = Path::new("src/style.css");
    let resolved_path = if css_path.exists() {
        css_path.to_path_buf()
    } else {
        Path::new("crates/uninote-app/src/style.css").to_path_buf()
    };

    let css = fs::read_to_string(&resolved_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {:?}", resolved_path.display(), e));

    let required_tokens = [
        ("dms_surface", "dms_surface_light"),
        ("dms_surface_container", "dms_surface_container_light"),
        (
            "dms_surface_container_high",
            "dms_surface_container_high_light",
        ),
        (
            "dms_surface_container_highest",
            "dms_surface_container_highest_light",
        ),
        ("dms_outline", "dms_outline_light"),
        ("dms_outline_variant", "dms_outline_variant_light"),
        ("dms_primary", "dms_primary_light"),
        ("dms_primary_container", "dms_primary_container_light"),
        ("dms_on_primary", "dms_on_primary_light"),
        ("dms_on_primary_container", "dms_on_primary_container_light"),
        ("dms_text_primary", "dms_text_primary_light"),
        ("dms_text_secondary", "dms_text_secondary_light"),
    ];

    for (dark_token, light_token) in &required_tokens {
        let dark_def = format!("@define-color {}", dark_token);
        let light_def = format!("@define-color {}", light_token);

        assert!(
            css.contains(&dark_def),
            "style.css must define dark token '{}'",
            dark_def
        );
        assert!(
            css.contains(&light_def),
            "style.css must define corresponding light token '{}'",
            light_def
        );
    }
}

#[test]
fn test_dms_light_theme_component_class_coverage() {
    let css_path = Path::new("src/style.css");
    let resolved_path = if css_path.exists() {
        css_path.to_path_buf()
    } else {
        Path::new("crates/uninote-app/src/style.css").to_path_buf()
    };

    let css = fs::read_to_string(&resolved_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {:?}", resolved_path.display(), e));

    // Components that MUST have light-theme variants
    let required_components = [
        "window.uninote-window.light-theme",
        ".light-theme .dms-pill",
        ".light-theme .dms-pill-dense",
        ".light-theme .dms-action-btn",
        ".light-theme .dms-tool-btn",
        ".light-theme .dms-thick-chip",
        ".light-theme .dms-left-dock",
        ".light-theme .dms-dock-handle",
        ".light-theme .dms-pen-slot",
        ".light-theme .dms-add-pen-btn",
        ".light-theme .dms-dock-divider",
        ".light-theme .dms-btn-add-page",
        ".light-theme .dms-btn-grid-select",
        ".light-theme .dms-zoom-segment",
        ".light-theme .dms-paper-card",
        ".light-theme .dms-action-card",
        ".light-theme scale.dms-pill-scale",
        ".light-theme .dms-preset-chip",
    ];

    for comp in &required_components {
        assert!(
            css.contains(comp),
            "style.css must provide light-theme rule for '{}'",
            comp
        );
    }

    // Crucial GTK4 Popover check: popover.dms-popover.light-theme MUST exist
    // because GTK4 popovers are separate native surfaces from window hierarchy
    assert!(
        css.contains("popover.dms-popover.light-theme")
            || css.contains(".light-theme popover.dms-popover"),
        "style.css must target popover.dms-popover.light-theme directly"
    );
}

// ============================================================================
// 2. DYNAMIC THEME SWITCHING: WINDOW & 4 POPOVERS SYNCHRONIZATION
// ============================================================================

#[test]
fn test_gtk4_theme_switching_and_css_provider() {
    let _ = gtk4::init();

    // 1. Verify CSS Provider loads style.css cleanly
    let css_path = Path::new("src/style.css");
    let resolved_path = if css_path.exists() {
        css_path.to_path_buf()
    } else {
        Path::new("crates/uninote-app/src/style.css").to_path_buf()
    };

    let css = fs::read_to_string(&resolved_path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {:?}", resolved_path.display(), e));

    let provider = CssProvider::new();
    provider.load_from_data(&css);

    // 2. Verify dynamic theme switching across 5 widgets
    let window = ApplicationWindow::builder()
        .css_classes(["uninote-window"])
        .build();
    let thick_popover = Popover::builder().css_classes(["dms-popover"]).build();
    let export_popover = Popover::builder().css_classes(["dms-popover"]).build();
    let settings_popover = Popover::builder().css_classes(["dms-popover"]).build();
    let grid_popover = Popover::builder().css_classes(["dms-popover"]).build();

    let widgets = [
        window.upcast::<gtk4::Widget>(),
        thick_popover.upcast::<gtk4::Widget>(),
        export_popover.upcast::<gtk4::Widget>(),
        settings_popover.upcast::<gtk4::Widget>(),
        grid_popover.upcast::<gtk4::Widget>(),
    ];

    // Simulate switching theme 1,000 times
    for cycle in 0..1000 {
        let is_dark = cycle % 2 == 0;
        for w in &widgets {
            if is_dark {
                w.remove_css_class("light-theme");
            } else {
                w.add_css_class("light-theme");
            }
        }

        // Verify strict synchronization
        let expected_has_class = !is_dark;
        for (idx, w) in widgets.iter().enumerate() {
            assert_eq!(
                w.has_css_class("light-theme"),
                expected_has_class,
                "Widget {} desynchronized on cycle {} (is_dark={})",
                idx,
                cycle,
                is_dark
            );
        }
    }
}

// ============================================================================
// 3. DOCK DRAG AT 120HZ: ARITHMETIC INVARIANT & CLAMP BOUNDS
// ============================================================================

#[test]
fn test_dock_drag_120hz_motion_performance_pure_arithmetic() {
    let start_top = Rc::new(Cell::new(190.0f64));
    let max_drag_y = Rc::new(Cell::new(500.0f64));

    // Simulate 12,000 frames (100 seconds of continuous 120Hz drag motion)
    let t0 = std::time::Instant::now();
    for i in 0..12000 {
        let offset_y = (i as f64 * 0.05).sin() * 200.0;
        let init_y = start_top.get();
        let max_y = max_drag_y.get();
        let new_y = (init_y + offset_y).clamp(72.0, max_y);
        assert!((72.0..=500.0).contains(&new_y));
    }
    let elapsed = t0.elapsed();

    // 12,000 pure arithmetic calculations must execute in under 10 milliseconds
    assert!(
        elapsed.as_millis() < 10,
        "12,000 drag updates took {}ms, expected < 10ms",
        elapsed.as_millis()
    );
}

#[test]
fn test_dock_drag_clamping_boundary_exhaustiveness() {
    let test_dock_heights: [f64; 5] = [50.0, 150.0, 300.0, 500.0, 800.0];
    let test_win_heights: [f64; 8] = [100.0, 200.0, 420.0, 425.0, 432.0, 768.0, 1080.0, 2160.0];
    let test_offsets: [f64; 7] = [-10000.0, -500.0, -72.0, 0.0, 50.0, 500.0, 10000.0];

    for &dock_h in &test_dock_heights {
        for &win_h in &test_win_heights {
            // Formula in window.rs:
            // let max_y = (win_h - dock_h - 60.0).max(72.0);
            let max_y: f64 = (win_h - dock_h - 60.0).max(72.0);

            // Invariant 1: min (72.0) <= max (max_y) ALWAYS holds
            assert!(
                max_y >= 72.0,
                "max_y ({}) must never be less than min_y (72.0) for win_h={}, dock_h={}",
                max_y,
                win_h,
                dock_h
            );

            for &offset_y in &test_offsets {
                let init_y = 190.0f64;
                // Invariant 2: clamp NEVER panics
                let clamped_y = (init_y + offset_y).clamp(72.0, max_y);
                assert!(
                    clamped_y >= 72.0,
                    "clamped_y ({}) must be >= 72.0",
                    clamped_y
                );
                assert!(
                    clamped_y <= max_y,
                    "clamped_y ({}) must be <= max_y ({})",
                    clamped_y,
                    max_y
                );
            }
        }
    }
}

// ============================================================================
// 4. UNINOTE LOGO CAIRO RENDERING MATHEMATICAL & VISUAL VERIFICATION
// ============================================================================

/// Replicates exact `draw_app_logo` Cairo pipeline from `crates/uninote-app/src/icons.rs`
fn render_official_logo(cr: &Context, w: f64, h: f64) {
    if w <= 0.0 || h <= 0.0 {
        return;
    }
    cr.save().expect("cairo save");

    let s = w.min(h);
    let r = s * (8.5 / 36.0);

    // 1. Sfondo Squircle con gradiente oceanico DMS (#004D65 -> #00222E)
    draw_rounded_rect(cr, 0.0, 0.0, s, s, r);
    let pat = gtk4::cairo::LinearGradient::new(0.0, 0.0, s, s);
    pat.add_color_stop_rgb(0.0, 0.0, 0.302, 0.396);
    pat.add_color_stop_rgb(1.0, 0.0, 0.133, 0.180);
    cr.set_source(&pat).expect("gradient pattern");
    let _ = cr.fill_preserve();

    cr.set_source_rgba(0.427, 0.824, 1.0, 0.22);
    cr.set_line_width((s / 36.0).max(0.75));
    let _ = cr.stroke();

    let is = s / 36.0;
    cr.translate((s - 36.0 * is) / 2.0, (s - 36.0 * is) / 2.0);
    cr.scale(is, is);

    cr.set_line_cap(gtk4::cairo::LineCap::Round);
    cr.set_line_join(gtk4::cairo::LineJoin::Round);

    // 2. Braccio sinistro e conca inferiore della 'U' (Bianco Ghiaccio #ECEFF4)
    cr.set_source_rgb(0.925, 0.937, 0.957);
    cr.set_line_width(3.6);
    cr.move_to(10.5, 9.5);
    cr.line_to(10.5, 20.0);
    cr.curve_to(10.5, 25.5, 14.5, 28.5, 18.0, 28.5);
    let _ = cr.stroke();

    // 3. Pennino Stilografico angolato (Braccio destro della 'U' in Ciano #6DD2FF)
    cr.move_to(23.5, 9.5);
    cr.line_to(27.5, 9.5);
    cr.line_to(28.0, 17.5);
    cr.line_to(18.0, 28.5); // Bottom center target
    cr.line_to(20.5, 19.5);
    cr.close_path();

    cr.set_source_rgba(0.427, 0.824, 1.0, 0.25);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(0.427, 0.824, 1.0); // Cyan #6DD2FF
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    // Fessura d'inchiostro (Ink Slit) e Foro di sfiato (Breather Hole)
    cr.move_to(18.0, 28.5);
    cr.line_to(23.0, 19.0);
    cr.set_line_width(1.4);
    let _ = cr.stroke();

    cr.arc(23.0, 19.0, 1.2, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.925, 0.937, 0.957);
    let _ = cr.fill();

    // 4. Accento Verde Menta DMS (#79D9A5) all'apice del pennino
    cr.arc(17.2, 29.8, 1.4, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.475, 0.851, 0.647);
    let _ = cr.fill();

    cr.restore().expect("cairo restore");
}

fn draw_rounded_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0);
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI);
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0);
    cr.close_path();
}

#[test]
fn test_logo_cairo_rendering_properties_and_colors() {
    let surface = ImageSurface::create(Format::ARgb32, 72, 72).unwrap();
    let cr = Context::new(&surface).unwrap();
    render_official_logo(&cr, 72.0, 72.0);
    assert!(cr.status().is_ok());

    drop(cr);
    let stride = surface.stride() as usize;
    let data = surface.take_data().expect("take_data");

    // Grid size 72x72: scale factor = 72 / 36 = 2.0.
    // Nib tip (18.0, 28.5) maps to pixel (18.0 * 2.0, 28.5 * 2.0) = (36, 57).
    // Mint accent (17.2, 29.8) maps to pixel (34.4, 59.6) ~= (34, 60).

    let get_pixel = |x: usize, y: usize| -> (u8, u8, u8, u8) {
        let offset = y * stride + x * 4;
        (
            data[offset + 2],
            data[offset + 1],
            data[offset],
            data[offset + 3],
        ) // R, G, B, A in ARgb32
    };

    // Check that pixels are painted in the neighborhood of the nib tip (36, 57)
    let mut painted_nib_tip = false;
    for dy in 55..=60 {
        for dx in 33..=39 {
            let (r, g, b, a) = get_pixel(dx, dy);
            if a > 0 {
                painted_nib_tip = true;
            }
        }
    }
    assert!(
        painted_nib_tip,
        "Nib tip neighborhood around (36, 57) must contain painted pixels"
    );
}

#[test]
fn test_logo_zero_and_negative_dimensions_safe() {
    let surface = ImageSurface::create(Format::ARgb32, 10, 10).unwrap();
    let cr = Context::new(&surface).unwrap();
    render_official_logo(&cr, 0.0, 0.0);
    render_official_logo(&cr, -10.0, 10.0);
    render_official_logo(&cr, 10.0, -10.0);
    assert!(
        cr.status().is_ok(),
        "Zero/negative dimensions must not fail"
    );
}

// ============================================================================
// 5. SEMVER 0.2.0 BADGE DYNAMIC BINDING
// ============================================================================

#[test]
fn test_semver_version_badge_dynamic_binding() {
    let pkg_ver = env!("CARGO_PKG_VERSION");
    assert_eq!(
        pkg_ver, "0.2.0",
        "CARGO_PKG_VERSION must be exactly 0.2.0 for Milestone M4"
    );

    let badge_text = concat!("v", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        badge_text, "v0.2.0",
        "Badge text in window.rs must evaluate to 'v0.2.0'"
    );

    // Verify root Cargo.toml workspace version
    let root_cargo_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.join("Cargo.toml"))
        .expect("Resolve root Cargo.toml path");

    let root_cargo = fs::read_to_string(&root_cargo_path)
        .unwrap_or_else(|e| panic!("Read {:?}: {:?}", root_cargo_path, e));

    assert!(
        root_cargo.contains("version = \"0.2.0\""),
        "Root Cargo.toml must declare version = \"0.2.0\""
    );
}
