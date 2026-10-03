//! Headless renderer used by `--render <file.png> [theme]`.
//!
//! GTK4 cannot put Pango text on a Cairo context (see `main.rs`), and gdk4-rs exposes
//! no way to snapshot a widget to a texture, so this draws the *same* Cairo scene the
//! window draws and approximates the legends with Cairo's toy text API. It is enough
//! to check board geometry, spacing, radii and palette without a display attached.
//!
//! The approximation is not the real thing: toy text does no shaping, so Arabic shows
//! as isolated letters here even though the live window renders it correctly.

use std::fs::File;

use crate::geometry;
use crate::layout::{self, Cap};
use crate::render::{self, Scene};
use crate::theme;
use gtk::cairo::{self, FontSlant, FontWeight};

const FONT: &str = "DejaVu Sans";

/// Draw one frame to a PNG.
/// Board-fitting constants, mirrored from `main.rs`.
#[derive(Clone, Copy)]
pub struct Fit {
    pub w: f64,
    pub h: f64,
    pub pad: f64,
    pub gap: f64,
    pub header: f64,
    pub footer: f64,
}

pub fn to_png(path: &str, theme_idx: usize, forced: Option<(String, String)>, f: Fit) {
    let Fit { w, h, pad, gap, header, footer } = f;
    let info = layout::load(forced);
    let t = &theme::THEMES[theme_idx.min(theme::THEMES.len() - 1)];
    let m = geometry::metrics(w, h, pad, gap, header, footer);

    let surface = match cairo::ImageSurface::create(cairo::Format::ARgb32, w as i32, h as i32) {
        Ok(s) => s,
        Err(e) => return fail(format!("cannot create surface: {e}")),
    };
    let cr = match cairo::Context::new(&surface) {
        Ok(c) => c,
        Err(e) => return fail(format!("cannot create context: {e}")),
    };

    let scene = Scene {
        caps: &info.caps,
        theme: t,
        metrics: m,
        time: 0.0,
        entrance: -1.0,
        header,
        footer,
    };
    render::paint(&cr, w, h, &scene);

    for (i, slot) in geometry::slots().iter().enumerate() {
        if let Some(cap) = info.caps.get(i) {
            let (x, y, cw, ch) = m.rect(slot);
            legends(&cr, cap, t, x, y, cw, ch);
        }
    }

    // Header and footer.
    simple(&cr, "KEYSCAPE", 26.0, 32.0, 11.0, t.accent_a);
    simple(&cr, &info.display_name, 122.0, 36.0, 22.0, t.txt_hi);
    simple(&cr, &info.subtitle(), 122.0, 56.0, 12.0, t.txt_dim);
    simple(
        &cr,
        &format!("via {} · {} keys · ANSI 60%", info.source, info.caps.len()),
        26.0,
        h - footer + 22.0,
        11.0,
        t.txt_dim,
    );
    if let Ok(w_adv) = advance(&cr, &format!("{} · Super+Space to switch", t.name), 11.0) {
        simple(
            &cr,
            &format!("{} · Super+Space to switch", t.name),
            w - 26.0 - w_adv,
            h - footer + 22.0,
            11.0,
            t.txt_dim,
        );
    }

    let written = File::create(path).map_err(|e| e.to_string()).and_then(|mut f| {
        surface.write_to_png(&mut f).map_err(|e| e.to_string())
    });
    match written {
        Ok(()) => println!(
            "wrote {path}  ({}x{}, layout {}, {} caps, unit {:.1}px)",
            w as i32,
            h as i32,
            info.layout,
            info.caps.len(),
            m.unit
        ),
        Err(e) => fail(format!("cannot write {path}: {e}")),
    }
}

fn fail(msg: String) {
    eprintln!("{msg}");
}

/// Draw a left-anchored string with its baseline at y.
fn simple(cr: &cairo::Context, s: &str, x: f64, y: f64, size: f64, color: [f64; 3]) {
    cr.select_font_face(FONT, FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(size);
    cr.set_source_rgb(color[0], color[1], color[2]);
    cr.move_to(x, y);
    let _ = cr.show_text(s);
}

fn advance(cr: &cairo::Context, s: &str, size: f64) -> Result<f64, cairo::Error> {
    cr.select_font_face(FONT, FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(size);
    Ok(cr.text_extents(s)?.x_advance())
}

/// Base centred, shift top-left, altgr bottom-right — matching the live window.
fn legends(cr: &cairo::Context, cap: &Cap, t: &theme::Theme, x: f64, y: f64, w: f64, h: f64) {
    if let Some(b) = &cap.base {
        let size = px(h, if cap.modifier { 0.22 } else { 0.40 });
        let color = if cap.modifier { t.txt_dim } else { t.txt_base };
        cr.select_font_face(FONT, FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(size);
        if let Ok(fe) = cr.font_extents() {
            let baseline = y + h / 2.0 - (fe.ascent() + fe.descent()) / 2.0;
            centred(cr, b, size, color, x + w / 2.0, baseline);
        }
    }
    if let Some(s) = &cap.shift {
        let size = px(h, 0.235);
        let base_y = y + 3.0 + size * 0.92;
        if let Ok(a) = advance(cr, s, size) {
            centred(cr, s, size, t.txt_dim, x + 3.0 + a / 2.0, base_y);
        }
    }
    if let Some(a) = &cap.altgr {
        let size = px(h, 0.235);
        let base_y = y + h - 3.0 - size * 0.30;
        if let Ok(w_adv) = advance(cr, a, size) {
            centred(cr, a, size, t.accent_c, x + w - 3.0 - w_adv / 2.0, base_y);
        }
    }
}

fn centred(cr: &cairo::Context, s: &str, size: f64, color: [f64; 3], cx: f64, baseline: f64) {
    cr.select_font_face(FONT, FontSlant::Normal, FontWeight::Bold);
    cr.set_font_size(size);
    if let Ok(ext) = cr.text_extents(s) {
        cr.set_source_rgb(color[0], color[1], color[2]);
        cr.move_to(cx - ext.width() / 2.0, baseline);
        let _ = cr.show_text(s);
    }
}

fn px(unit: f64, factor: f64) -> f64 {
    (unit * factor).round().max(7.0)
}