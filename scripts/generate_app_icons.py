#!/usr/bin/env python3
"""
Script di generazione deterministica degli asset del logo e delle icone FreeDesktop/XDG di UniNote.
Genera le icone SVG scalabili, simboliche e i formati PNG raster (16, 32, 48, 64, 128, 256, 512),
oltre agli asset ufficiali per GitHub (avatar e social preview card 1280x640).
"""

import math
import os
import shutil
import cairo

PI = math.pi

def draw_rounded_rect(cr, x, y, w, h, r):
    r = min(r, w / 2.0, h / 2.0)
    cr.new_sub_path()
    cr.arc(x + w - r, y + r, r, -PI / 2.0, 0.0)
    cr.arc(x + w - r, y + h - r, r, 0.0, PI / 2.0)
    cr.arc(x + r, y + h - r, r, PI / 2.0, PI)
    cr.arc(x + r, y + r, r, PI, 3.0 * PI / 2.0)
    cr.close_path()

def draw_uninote_logo(cr, size):
    """
    Riproduce esattamente il logo Cairo definito in crates/uninote-app/src/icons.rs.
    """
    s = float(size)
    r = s * (8.5 / 36.0)

    # 1. Sfondo Squircle con gradiente oceanico DMS (#004D65 -> #00222E)
    draw_rounded_rect(cr, 0.0, 0.0, s, s, r)
    pat = cairo.LinearGradient(0.0, 0.0, s, s)
    pat.add_color_stop_rgb(0.0, 0.0, 0.302, 0.396)  # #004D65
    pat.add_color_stop_rgb(1.0, 0.0, 0.133, 0.180)  # #00222E
    cr.set_source(pat)
    cr.fill_preserve()

    # Bordo interno a luce radente ciano ultra-sottile
    cr.set_source_rgba(0.427, 0.824, 1.0, 0.22)
    cr.set_line_width(max(s / 36.0, 0.75))
    cr.stroke()

    # Griglia geometrica 36x36 per gli elementi del monogramma
    is_scale = s / 36.0
    cr.save()
    cr.translate((s - 36.0 * is_scale) / 2.0, (s - 36.0 * is_scale) / 2.0)
    cr.scale(is_scale, is_scale)

    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_join(cairo.LINE_JOIN_ROUND)

    # 2. Braccio sinistro e conca inferiore della 'U' (Bianco Ghiaccio #ECEFF4)
    cr.set_source_rgb(0.925, 0.937, 0.957)
    cr.set_line_width(3.6)
    cr.move_to(10.5, 9.5)
    cr.line_to(10.5, 20.0)
    cr.curve_to(10.5, 25.5, 14.5, 28.5, 18.0, 28.5)
    cr.stroke()

    # 3. Pennino Stilografico angolato (Braccio destro della 'U' in Ciano #6DD2FF)
    cr.move_to(23.5, 9.5)
    cr.line_to(27.5, 9.5)
    cr.line_to(28.0, 17.5)  # Spalla esterna
    cr.line_to(18.0, 28.5)  # Punta pennino
    cr.line_to(20.5, 19.5)  # Spalla interna
    cr.close_path()

    cr.set_source_rgba(0.427, 0.824, 1.0, 0.25)
    cr.fill_preserve()
    cr.set_source_rgb(0.427, 0.824, 1.0)
    cr.set_line_width(2.0)
    cr.stroke()

    # Fessura d'inchiostro e foro di sfiato
    cr.move_to(18.0, 28.5)
    cr.line_to(23.0, 19.0)
    cr.set_line_width(1.4)
    cr.stroke()

    cr.arc(23.0, 19.0, 1.2, 0.0, 2.0 * PI)
    cr.set_source_rgb(0.925, 0.937, 0.957)
    cr.fill()

    # 4. Accento Verde Menta DMS (#79D9A5)
    cr.arc(17.2, 29.8, 1.4, 0.0, 2.0 * PI)
    cr.set_source_rgb(0.475, 0.851, 0.647)
    cr.fill()

    cr.restore()

def draw_symbolic_logo(cr, size):
    """Disegna l'icona simbolica monocroma (senza sfondo squircle gradiente) per standard GNOME."""
    s = float(size)
    is_scale = s / 36.0
    cr.save()
    cr.scale(is_scale, is_scale)
    cr.set_line_cap(cairo.LINE_CAP_ROUND)
    cr.set_line_join(cairo.LINE_JOIN_ROUND)

    # Monocromo bianco/colore primario corrente
    cr.set_source_rgb(1.0, 1.0, 1.0)
    cr.set_line_width(3.6)
    cr.move_to(10.5, 9.5)
    cr.line_to(10.5, 20.0)
    cr.curve_to(10.5, 25.5, 14.5, 28.5, 18.0, 28.5)
    cr.stroke()

    cr.move_to(23.5, 9.5)
    cr.line_to(27.5, 9.5)
    cr.line_to(28.0, 17.5)
    cr.line_to(18.0, 28.5)
    cr.line_to(20.5, 19.5)
    cr.close_path()
    cr.set_line_width(2.0)
    cr.stroke()

    cr.move_to(18.0, 28.5)
    cr.line_to(23.0, 19.0)
    cr.set_line_width(1.4)
    cr.stroke()

    cr.arc(23.0, 19.0, 1.2, 0.0, 2.0 * PI)
    cr.fill()

    cr.restore()

def generate_png(size, output_path):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    cr = cairo.Context(surface)
    draw_uninote_logo(cr, size)
    surface.write_to_png(output_path)

def generate_svg(size, output_path):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    surface = cairo.SVGSurface(output_path, size, size)
    cr = cairo.Context(surface)
    draw_uninote_logo(cr, size)
    surface.finish()

def generate_symbolic_svg(size, output_path):
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    surface = cairo.SVGSurface(output_path, size, size)
    cr = cairo.Context(surface)
    draw_symbolic_logo(cr, size)
    surface.finish()

def generate_social_preview(output_path):
    """Genera la preview card OpenGraph (1280x640) per GitHub."""
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    w, h = 1280, 640
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
    cr = cairo.Context(surface)

    # Sfondo DMS Dark OLED
    cr.set_source_rgb(0.039, 0.082, 0.102)  # #0A151A
    cr.paint()

    # Griglia decorativa sottile a puntini (Whiteboard style)
    cr.set_source_rgba(0.2, 0.4, 0.5, 0.15)
    for x in range(40, w, 40):
        for y in range(40, h, 40):
            cr.arc(x, y, 1.5, 0, 2 * PI)
            cr.fill()

    # Logo grande a sinistra (280x280)
    logo_size = 280
    logo_x = 100
    logo_y = (h - logo_size) / 2
    cr.save()
    cr.translate(logo_x, logo_y)
    draw_uninote_logo(cr, logo_size)
    cr.restore()

    # Testi tipografici a destra
    text_x = 440
    cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)

    # Titolo "UniNote"
    cr.set_font_size(76)
    cr.set_source_rgb(0.925, 0.937, 0.957)  # #ECEFF4
    cr.move_to(text_x, 260)
    cr.show_text("UniNote")

    # Sottotitolo Accademico
    cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(28)
    cr.set_source_rgb(0.427, 0.824, 1.0)  # Ciano #6DD2FF
    cr.move_to(text_x, 320)
    cr.show_text("Academic Freehand Notes & Vector PDF Suite")

    # Badge e Caratteristiche
    cr.set_font_size(20)
    cr.set_source_rgb(0.65, 0.75, 0.8)
    cr.move_to(text_x, 380)
    cr.show_text("Rust  •  GTK 4  •  Libadwaita  •  Cairo  •  SQLite WAL")

    cr.move_to(text_x, 420)
    cr.set_source_rgb(0.475, 0.851, 0.647)  # Menta #79D9A5
    cr.show_text("120Hz Inking  •  MPP/ELAN & Wacom  •  Infinite Whiteboard & A4")

    surface.write_to_png(output_path)

def generate_header_banner(output_path):
    """Genera il banner per l'header del README.md (800x160)."""
    os.makedirs(os.path.dirname(output_path), exist_ok=True)
    w, h = 800, 160
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, w, h)
    cr = cairo.Context(surface)

    # Trasparente con logo a sinistra e testo
    logo_size = 120
    logo_x = 24
    logo_y = (h - logo_size) / 2
    cr.save()
    cr.translate(logo_x, logo_y)
    draw_uninote_logo(cr, logo_size)
    cr.restore()

    text_x = 170
    cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_BOLD)
    cr.set_font_size(52)
    cr.set_source_rgb(0.925, 0.937, 0.957)
    cr.move_to(text_x, 80)
    cr.show_text("UniNote")

    cr.select_font_face("Sans", cairo.FONT_SLANT_NORMAL, cairo.FONT_WEIGHT_NORMAL)
    cr.set_font_size(18)
    cr.set_source_rgb(0.427, 0.824, 1.0)
    cr.move_to(text_x, 115)
    cr.show_text("Academic Freehand Note-Taking & Vector PDF Slide Annotation Suite for Linux")

    surface.write_to_png(output_path)

def main():
    base_dir = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
    print(f"Generazione icone UniNote in {base_dir}...")

    # 1. SVG Vettoriali
    svg_path = os.path.join(base_dir, "data/icons/hicolor/scalable/apps/org.uninote.UniNote.svg")
    generate_svg(512, svg_path)
    print(f"[OK] Scalable SVG: {svg_path}")

    sym_path = os.path.join(base_dir, "data/icons/hicolor/symbolic/apps/org.uninote.UniNote-symbolic.svg")
    generate_symbolic_svg(16, sym_path)
    print(f"[OK] Symbolic SVG: {sym_path}")

    # MIME Type Icon (per file .uninote)
    mime_svg = os.path.join(base_dir, "data/icons/hicolor/scalable/mimetypes/application-x-uninote.svg")
    generate_svg(512, mime_svg)
    print(f"[OK] MimeType SVG: {mime_svg}")

    # 2. PNG Raster hicolor
    resolutions = [16, 32, 48, 64, 128, 256, 512]
    for res in resolutions:
        png_path = os.path.join(base_dir, f"data/icons/hicolor/{res}x{res}/apps/org.uninote.UniNote.png")
        generate_png(res, png_path)
        print(f"[OK] Raster PNG {res}x{res}: {png_path}")

        # MIME PNG a 48 e 128
        if res in [48, 128]:
            mime_png = os.path.join(base_dir, f"data/icons/hicolor/{res}x{res}/mimetypes/application-x-uninote.png")
            generate_png(res, mime_png)
            print(f"[OK] MimeType PNG {res}x{res}: {mime_png}")

    # 3. Branding GitHub
    avatar_path = os.path.join(base_dir, "docs/branding/avatar.png")
    generate_png(512, avatar_path)
    print(f"[OK] Avatar GitHub: {avatar_path}")

    preview_path = os.path.join(base_dir, "docs/branding/social_preview.png")
    generate_social_preview(preview_path)
    print(f"[OK] Social Preview Card: {preview_path}")

    header_path = os.path.join(base_dir, "docs/branding/uninote_header.png")
    generate_header_banner(header_path)
    print(f"[OK] Header Banner: {header_path}")

    # Copia nella cartella artefatti per visualizzazione diretta
    artifact_dir = "/home/cachylore/.gemini/antigravity/brain/88a9d6e4-0f51-46a3-a216-bfa05c324f00"
    if os.path.isdir(artifact_dir):
        shutil.copy(avatar_path, os.path.join(artifact_dir, "avatar.png"))
        shutil.copy(preview_path, os.path.join(artifact_dir, "social_preview.png"))
        shutil.copy(header_path, os.path.join(artifact_dir, "uninote_header.png"))
        shutil.copy(os.path.join(base_dir, "data/icons/hicolor/256x256/apps/org.uninote.UniNote.png"), os.path.join(artifact_dir, "logo_256.png"))
        print(f"[OK] Asset copiati nell'artefatto per visualizzazione immediata!")

    print("\nTutti gli asset del logo e delle icone generati con successo!")

if __name__ == "__main__":
    main()
