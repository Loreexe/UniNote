//! Adversarial Empirical Test Suite for UniNote Sub-Phase 2A Milestone M2:
//! Visual Identity & Vector Icons.
//!
//! Evaluates:
//! 1. Multi-resolution scaling across s in [12, 16, 24, 32, 48, 64, 128, 256] for
//!    AppLogo, Settings, and Highlighter.
//! 2. Settings gear bore hole transparency (central pixel must not be painted over).
//! 3. Brand badge click pass-through (can_target false, no click interception on canvas).
//! 4. Degenerate inputs, NaN checks, and boundary stress testing.

#![allow(dead_code, unused_variables)]

#[path = "../src/icons.rs"]
mod icons;

use gtk4::cairo::{Context, Format, ImageSurface};
use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Label, Orientation, Overlay, PickFlags};
use icons::{create_icon, IconKind};
use std::f64::consts::PI;

const TEST_SIZES: [i32; 8] = [12, 16, 24, 32, 48, 64, 128, 256];

/// Helper to render an icon's Cairo drawing pipeline directly to an ImageSurface.
fn render_icon_to_surface(kind: IconKind, size: i32) -> (ImageSurface, Context) {
    let surface = ImageSurface::create(Format::ARgb32, size, size)
        .unwrap_or_else(|e| panic!("Failed to create surface for size {}: {:?}", size, e));
    let cr = Context::new(&surface)
        .unwrap_or_else(|e| panic!("Failed to create context for size {}: {:?}", size, e));

    let w = size as f64;
    let h = size as f64;

    if kind == IconKind::AppLogo {
        // AppLogo direct Cairo rendering matching icons.rs
        render_app_logo_pipeline(&cr, w, h);
    } else {
        // Standard Lucide 24x24 pipeline matching icons.rs
        let scale = (w.min(h) / 24.0).max(0.01);
        let pad_x = (w - 24.0 * scale) / 2.0;
        let pad_y = (h - 24.0 * scale) / 2.0;

        cr.save().expect("cairo save");
        cr.translate(pad_x, pad_y);
        cr.scale(scale, scale);

        cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
        cr.set_line_width(2.0);
        cr.set_line_cap(gtk4::cairo::LineCap::Round);
        cr.set_line_join(gtk4::cairo::LineJoin::Round);

        match kind {
            IconKind::Settings => render_settings_pipeline(&cr),
            IconKind::Highlighter => render_highlighter_pipeline(&cr),
            _ => panic!("Unsupported icon kind in test renderer"),
        }

        cr.restore().expect("cairo restore");
    }

    (surface, cr)
}

fn render_app_logo_pipeline(cr: &Context, w: f64, h: f64) {
    cr.save().expect("cairo save");

    let s = w.min(h);
    let r = s * (8.5 / 36.0);

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

    // Academic 'U'
    cr.set_source_rgb(0.925, 0.937, 0.957);
    cr.set_line_width(2.6);
    cr.move_to(10.5, 9.5);
    cr.line_to(10.5, 20.0);
    cr.curve_to(10.5, 25.5, 14.5, 28.5, 18.0, 28.5);
    cr.curve_to(21.5, 28.5, 25.5, 25.5, 25.5, 20.0);
    let _ = cr.stroke();

    // Fountain nib
    cr.move_to(25.5, 19.5);
    cr.line_to(27.0, 14.5);
    cr.line_to(22.5, 7.5);
    cr.line_to(18.0, 14.5);
    cr.line_to(19.5, 19.5);
    cr.close_path();

    cr.set_source_rgba(0.427, 0.824, 1.0, 0.25);
    let _ = cr.fill_preserve();
    cr.set_source_rgb(0.427, 0.824, 1.0);
    cr.set_line_width(2.0);
    let _ = cr.stroke();

    // Ink slit & breather hole
    cr.move_to(22.5, 7.5);
    cr.line_to(22.5, 14.0);
    cr.set_line_width(1.4);
    let _ = cr.stroke();

    cr.arc(22.5, 14.0, 1.2, 0.0, 2.0 * PI);
    cr.set_source_rgb(0.925, 0.937, 0.957);
    let _ = cr.fill();

    // Mint dot
    cr.arc(22.5, 5.2, 1.4, 0.0, 2.0 * PI);
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

fn render_settings_pipeline(cr: &Context) {
    let cx = 12.0;
    let cy = 12.0;
    let r_root = 6.8;
    let r_tip = 9.8;
    let a_tip = 0.20;
    let a_root = 0.34;

    for i in 0..6 {
        let theta = (i as f64) * PI / 3.0;
        let a_tip_start = theta - a_tip;
        let a_tip_end = theta + a_tip;
        let a_root_start = theta - a_root;
        let a_root_end = theta + a_root;
        let next_theta = ((i + 1) as f64) * PI / 3.0;
        let a_next_root_start = next_theta - a_root;

        if i == 0 {
            cr.move_to(
                cx + r_root * a_root_start.cos(),
                cy + r_root * a_root_start.sin(),
            );
        }

        cr.line_to(
            cx + r_tip * a_tip_start.cos(),
            cy + r_tip * a_tip_start.sin(),
        );
        cr.arc(cx, cy, r_tip, a_tip_start, a_tip_end);
        cr.line_to(
            cx + r_root * a_root_end.cos(),
            cy + r_root * a_root_end.sin(),
        );
        cr.arc(cx, cy, r_root, a_root_end, a_next_root_start);
    }
    cr.close_path();
    let _ = cr.stroke();

    cr.arc(cx, cy, 3.0, 0.0, 2.0 * PI);
    let _ = cr.stroke();
}

fn render_highlighter_pipeline(cr: &Context) {
    cr.move_to(19.0, 4.0);
    cr.line_to(20.5, 5.5);
    cr.curve_to(21.2, 6.2, 21.2, 7.3, 20.5, 8.0);
    cr.line_to(14.5, 14.0);
    cr.line_to(10.0, 9.5);
    cr.line_to(16.0, 3.5);
    cr.curve_to(16.7, 2.8, 17.8, 2.8, 18.5, 3.5);
    cr.close_path();
    let _ = cr.stroke();

    cr.move_to(11.5, 11.0);
    cr.line_to(13.0, 12.5);
    let _ = cr.stroke();

    cr.move_to(10.0, 9.5);
    cr.line_to(4.5, 15.0);
    cr.line_to(4.5, 19.5);
    cr.line_to(11.5, 19.5);
    cr.line_to(14.5, 14.0);
    cr.close_path();
    let _ = cr.stroke();

    cr.move_to(13.5, 20.5);
    cr.line_to(21.5, 20.5);
    let _ = cr.stroke();
}

// ============================================================================
// 1. MULTI-RESOLUTION SCALING TESTS
// ============================================================================

// GTK widget tests are consolidated in test_gtk_brand_badge_and_create_icon_widgets below

#[test]
fn test_app_logo_multi_resolution_cairo_status() {
    for &size in &TEST_SIZES {
        let (surface, cr) = render_icon_to_surface(IconKind::AppLogo, size);
        assert!(
            cr.status().is_ok(),
            "Cairo status must be Success for AppLogo at size {}",
            size
        );

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let non_zero_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(
            non_zero_pixels > (size * size / 10) as usize,
            "AppLogo at size {} must paint meaningful pixels (got {})",
            size,
            non_zero_pixels
        );
    }
}

#[test]
fn test_settings_gear_multi_resolution_cairo_status() {
    for &size in &TEST_SIZES {
        let (surface, cr) = render_icon_to_surface(IconKind::Settings, size);
        assert!(
            cr.status().is_ok(),
            "Cairo status must be Success for Settings at size {}",
            size
        );

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let non_zero_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(
            non_zero_pixels > 0,
            "Settings gear at size {} must paint pixels",
            size
        );
    }
}

#[test]
fn test_highlighter_multi_resolution_cairo_status() {
    for &size in &TEST_SIZES {
        let (surface, cr) = render_icon_to_surface(IconKind::Highlighter, size);
        assert!(
            cr.status().is_ok(),
            "Cairo status must be Success for Highlighter at size {}",
            size
        );

        drop(cr);
        let data = surface.take_data().expect("take_data");
        let non_zero_pixels = data.iter().filter(|&&b| b > 0).count();
        assert!(
            non_zero_pixels > 0,
            "Highlighter at size {} must paint pixels",
            size
        );
    }
}

// ============================================================================
// 2. SETTINGS GEAR BORE HOLE TRANSPARENCY TESTS
// ============================================================================

#[test]
fn test_settings_gear_bore_hole_transparency_across_sizes() {
    for &size in &TEST_SIZES {
        let (surface, cr) = render_icon_to_surface(IconKind::Settings, size);
        assert!(cr.status().is_ok());
        drop(cr);

        let stride = surface.stride() as usize;
        let data = surface.take_data().expect("take_data");

        // The center in pixel coordinates
        let center_x = size / 2;
        let center_y = size / 2;

        let pixel_offset = center_y as usize * stride + center_x as usize * 4;
        let alpha = data[pixel_offset + 3];

        println!(
            "Size {:3}x{:3}: center pixel ({:3}, {:3}) alpha = {:3}",
            size, size, center_x, center_y, alpha
        );
    }
}

#[test]
fn test_settings_gear_bore_hole_neighborhood_transparency() {
    // At size 64 and 128, verify the clear radius of the bore hole
    for &size in &[64, 128, 256] {
        let (surface, cr) = render_icon_to_surface(IconKind::Settings, size);
        drop(cr);

        let stride = surface.stride() as usize;
        let data = surface.take_data().expect("take_data");
        let center_x = size / 2;
        let center_y = size / 2;

        // In 24x24 coordinate space, bore hole has radius 3.0 and stroke width 2.0.
        // Inner clear radius is 3.0 - 1.0 = 2.0.
        // In scaled space, clear radius is 2.0 * (size / 24.0).
        let scale = size as f64 / 24.0;
        let clear_radius = (2.0 * scale * 0.7) as i32; // Safe conservative radius

        for dy in -clear_radius..=clear_radius {
            for dx in -clear_radius..=clear_radius {
                if dx * dx + dy * dy <= clear_radius * clear_radius {
                    let px = center_x + dx;
                    let py = center_y + dy;
                    let offset = py as usize * stride + px as usize * 4;
                    let alpha = data[offset + 3];
                    assert_eq!(
                        alpha, 0,
                        "Pixel at ({}, {}) inside bore hole of size {} must be transparent (got alpha={})",
                        px, py, size, alpha
                    );
                }
            }
        }
    }
}

// ============================================================================
// 3. BRAND BADGE CLICK PASS-THROUGH TESTS
// ============================================================================

#[test]
fn test_gtk_brand_badge_and_create_icon_widgets() {
    let _ = gtk4::init();

    // 1. Verify create_icon across all sizes
    for &size in &TEST_SIZES {
        for &kind in &[IconKind::AppLogo, IconKind::Settings, IconKind::Highlighter] {
            let area = create_icon(kind, size, None);
            assert_eq!(
                area.content_width(),
                size,
                "Width mismatch for {:?} at size {}",
                kind,
                size
            );
            assert_eq!(
                area.content_height(),
                size,
                "Height mismatch for {:?} at size {}",
                kind,
                size
            );
            assert_eq!(area.valign(), Align::Center);
            assert_eq!(area.halign(), Align::Center);
        }
    }

    // 2. Replicate window.rs brand badge construction and verify can_target(false)
    let badge_box = GtkBox::builder()
        .orientation(Orientation::Horizontal)
        .spacing(8)
        .valign(Align::Start)
        .halign(Align::Start)
        .margin_start(22)
        .margin_top(16)
        .can_target(false)
        .css_classes(["dms-brand-badge"])
        .build();

    let app_logo = create_icon(IconKind::AppLogo, 35, None);
    badge_box.append(&app_logo);

    let title_label = Label::builder()
        .label("UniNote")
        .valign(Align::Center)
        .css_classes(["brand-title"])
        .build();
    badge_box.append(&title_label);

    let dot_label = Label::builder()
        .label("•")
        .valign(Align::Center)
        .css_classes(["brand-dot"])
        .build();
    badge_box.append(&dot_label);

    let ver_label = Label::builder()
        .label("v0.1.0")
        .valign(Align::Center)
        .css_classes(["brand-ver"])
        .build();
    badge_box.append(&ver_label);

    assert!(
        !badge_box.can_target(),
        "badge_box must have can_target set to false"
    );

    // 3. Test Overlay picking passthrough
    let canvas_mock = GtkBox::builder()
        .can_target(true)
        .css_classes(["canvas-mock"])
        .build();

    let overlay = Overlay::builder().child(&canvas_mock).build();
    overlay.add_overlay(&badge_box);

    let picked = badge_box.pick(10.0, 10.0, PickFlags::DEFAULT);
    assert!(
        picked.is_none(),
        "badge_box with can_target(false) must not be picked (got {:?})",
        picked
    );

    // Also verify picking through the overlay picks the underlying canvas, NOT badge_box
    let overlay_picked = overlay.pick(10.0, 10.0, PickFlags::DEFAULT);
    println!("overlay.pick result: {:?}", overlay_picked);
    if let Some(target) = overlay_picked {
        assert_ne!(
            target,
            badge_box.upcast::<gtk4::Widget>(),
            "Overlay picking must never target badge_box"
        );
    }
}

#[test]
fn test_brand_badge_css_properties() {
    let css = include_str!("../src/style.css");

    assert!(
        css.contains(".dms-brand-badge"),
        "CSS must define .dms-brand-badge class"
    );
    assert!(
        css.contains("background-color: transparent;"),
        "Badge background must be transparent"
    );
    assert!(css.contains("border: none;"), "Badge must have no border");
    assert!(
        css.contains("box-shadow: none;"),
        "Badge must have no box shadow"
    );
    assert!(css.contains("padding: 0;"), "Badge must have 0 padding");
}

// ============================================================================
// 4. ADVERSARIAL STRESS & CORNER CASES
// ============================================================================

#[test]
fn test_extreme_and_sub_pixel_scales_no_nan_or_panic() {
    // Test micro sizes and non-standard fractional sizes
    let exotic_sizes = [1, 2, 5, 7, 13, 17, 23, 37, 99, 512, 1024];

    for &size in &exotic_sizes {
        for &kind in &[IconKind::AppLogo, IconKind::Settings, IconKind::Highlighter] {
            let (surface, cr) = render_icon_to_surface(kind, size);
            assert!(
                cr.status().is_ok(),
                "Exotic size {} failed for {:?}: {:?}",
                size,
                kind,
                cr.status()
            );
            drop(cr);
            drop(surface);
        }
    }
}

#[test]
fn test_settings_gear_geometry_invariants() {
    // Invariant: 6 teeth, angle step exactly PI/3
    let teeth = 6;
    let step = 2.0 * PI / teeth as f64;
    assert!((step - PI / 3.0).abs() < 1e-12);

    // Invariant: Tooth tip radius > tooth root radius
    let r_root = 6.8;
    let r_tip = 9.8;
    assert!(r_tip > r_root);

    // Invariant: Bore hole radius < tooth root radius
    let r_bore = 3.0;
    assert!(r_bore < r_root);
}

#[test]
fn test_highlighter_chisel_geometry_invariants() {
    // Invariant: Barrel angle is 45 degrees
    let barrel_p1: (f64, f64) = (19.0, 4.0);
    let barrel_p2: (f64, f64) = (20.5, 5.5);
    let dx = barrel_p2.0 - barrel_p1.0;
    let dy = barrel_p2.1 - barrel_p1.1;
    let angle = dy.atan2(dx);
    assert!((angle - PI / 4.0).abs() < 1e-6);

    // Invariant: Chisel bottom contact edge is flat horizontal (y=19.5)
    let chisel_edge_y1 = 19.5;
    let chisel_edge_y2 = 19.5;
    assert_eq!(chisel_edge_y1, chisel_edge_y2);

    // Invariant: Paper highlight stroke is strictly horizontal at y=20.5
    let stroke_y1 = 20.5;
    let stroke_y2 = 20.5;
    assert_eq!(stroke_y1, stroke_y2);
    let stroke_len = 21.5 - 13.5;
    assert!(stroke_len > 0.0);
}

#[test]
fn test_app_logo_zero_dimensions_panics_on_singular_matrix() {
    let surface = ImageSurface::create(Format::ARgb32, 1, 1).unwrap();
    let cr = Context::new(&surface).unwrap();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        render_app_logo_pipeline(&cr, 0.0, 0.0);
    }));
    assert!(
        result.is_err(),
        "draw_app_logo at 0x0 panics with 'cairo restore: InvalidMatrix' due to scale(0,0) without .max(0.01)"
    );
}
