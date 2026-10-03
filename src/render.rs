//! Cairo painting for everything that is not text.
//!
//! GTK4 removed `gdk_cairo_show_layout`, so Pango text cannot be drawn onto a Cairo
//! context at all — it has to go through the render tree. Legends therefore live in
//! real `gtk::Label`s positioned over this canvas (see `main.rs`), which also means
//! Arabic and other joining scripts shape correctly for free.
//!
//! This module paints the backdrop, the glass panel, and the keycap bodies.

use crate::geometry::{slots, Metrics};
use crate::layout::Cap;
use crate::theme::Theme;
use gtk::cairo::{self, Antialias, Context, LinearGradient, RadialGradient};

/// Everything needed for one frame.
pub struct Scene<'a> {
    pub caps: &'a [Cap],
    pub theme: &'static Theme,
    pub metrics: Metrics,
    /// Seconds since app start, for blob drift and the pulse dot.
    pub time: f64,
    /// Seconds since the layout last changed; negative once the entrance is done.
    pub entrance: f64,
    /// Metres of vertical breathing room reserved for header and footer labels.
    pub header: f64,
    pub footer: f64,
}

const CAP_RADIUS: f64 = 7.0;

/// Rounded rectangle. Cairo has no `rounded_rectangle`, so build it from four arcs.
pub fn round_rect(cr: &Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0).max(0.0);
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -pi / 2.0, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, pi / 2.0);
    cr.arc(x + r, y + h - r, r, pi / 2.0, pi);
    cr.arc(x + r, y + r, r, pi, 3.0 * pi / 2.0);
    cr.close_path();
    cr.set_line_join(cairo::LineJoin::Round);
}

fn lighten(c: [f64; 3], amount: f64) -> [f64; 3] {
    [
        (c[0] + (1.0 - c[0]) * amount).min(1.0),
        (c[1] + (1.0 - c[1]) * amount).min(1.0),
        (c[2] + (1.0 - c[2]) * amount).min(1.0),
    ]
}

/// Per-cap entrance progress: eased, staggered left-to-right, top-to-bottom.
fn ease(entrance: f64, index: f64) -> f64 {
    if entrance < 0.0 {
        return 1.0;
    }
    let delay = index * 0.011;
    let p = ((entrance - delay) / 0.32).clamp(0.0, 1.0);
    1.0 - (1.0 - p).powi(3)
}

pub fn paint(cr: &Context, w: f64, h: f64, s: &Scene) {
    let t = s.theme;
    cr.set_antialias(Antialias::Best);

    // Backdrop: vertical gradient.
    let bg = LinearGradient::new(0.0, 0.0, 0.0, h);
    bg.add_color_stop_rgba(0.0, t.bg0[0], t.bg0[1], t.bg0[2], 1.0);
    bg.add_color_stop_rgba(1.0, t.bg1[0], t.bg1[1], t.bg1[2], 1.0);
    let _ = cr.set_source(&bg);
    let _ = cr.paint();

    // Gradient mesh: three slowly drifting blobs.
    let (dx, dy) = (w * 0.42, h * 0.40);
    let blobs: [([f64; 3], f64, f64, f64); 3] = [
        (t.blob_a, 0.22 + 0.05 * (s.time * 0.11).sin(), 0.28 + 0.04 * (s.time * 0.09).cos(), 0.34),
        (t.blob_b, 0.76 + 0.04 * (s.time * 0.08).cos(), 0.22 + 0.05 * (s.time * 0.13).sin(), 0.30),
        (t.blob_c, 0.52 + 0.06 * (s.time * 0.07).sin(), 0.80 + 0.03 * (s.time * 0.10).cos(), 0.26),
    ];
    for (c, fx, fy, alpha) in blobs {
        let cx = fx * w;
        let cy = fy * h;
        let r = dx.max(dy);
        let g = RadialGradient::new(cx, cy, 0.0, cx, cy, r);
        g.add_color_stop_rgba(0.0, c[0], c[1], c[2], alpha);
        g.add_color_stop_rgba(0.55, c[0], c[1], c[2], alpha * 0.32);
        g.add_color_stop_rgba(1.0, c[0], c[1], c[2], 0.0);
        let _ = cr.set_source(&g);
        let _ = cr.fill();
    }

    // Glass panel behind the board.
    let m = s.metrics;
    let pad = m.gap * 1.6;
    let board_w = 15.0 * m.unit + 14.0 * m.gap;
    let board_h = 5.0 * m.unit + 4.0 * m.gap;
    let px = m.x0 - pad;
    let py = m.y0 - pad;
    round_rect(cr, px, py, board_w + pad * 2.0, board_h + pad * 2.0, 16.0);
    cr.set_source_rgba(t.surface[0], t.surface[1], t.surface[2], t.surface[3]);
    let _ = cr.fill_preserve();
    cr.set_source_rgba(t.stroke[0], t.stroke[1], t.stroke[2], t.stroke[3]);
    cr.set_line_width(1.0);
    let _ = cr.stroke();

    // Keycaps.
    for (i, slot) in slots().iter().enumerate() {
        let Some(cap) = s.caps.get(i) else { continue };
        let p = ease(s.entrance, i as f64);
        if p <= 0.0 {
            continue;
        }
        let (x, y, cw, ch) = m.rect(slot);
        let y = y + (1.0 - p) * 14.0;

        let _ = cr.save();
        cr.translate(x, y);

        // Body shadow.
        round_rect(cr, 0.0, 2.5, cw, ch, CAP_RADIUS);
        cr.set_source_rgba(
            t.key_shadow[0], t.key_shadow[1], t.key_shadow[2],
            t.key_shadow[3] * p,
        );
        let _ = cr.fill();

        // Face, with a light-to-dark vertical fall for depth.
        round_rect(cr, 0.0, 0.0, cw, ch, CAP_RADIUS);
        let _ = cr.save();
        cr.clip();
        let top = lighten(t.key_face, 0.14);
        let g = LinearGradient::new(0.0, 0.0, 0.0, ch);
        g.add_color_stop_rgba(0.0, top[0], top[1], top[2], p);
        g.add_color_stop_rgba(1.0, t.key_face[0], t.key_face[1], t.key_face[2], p);
        let _ = cr.set_source(&g);
        let _ = cr.paint();

        // Accent wash on the lower edge, keyed to the altgr legend.
        if cap.altgr.is_some() && !cap.modifier {
            let a = LinearGradient::new(0.0, ch * 0.55, 0.0, ch);
            a.add_color_stop_rgba(0.0, t.accent_c[0], t.accent_c[1], t.accent_c[2], 0.0);
            a.add_color_stop_rgba(1.0, t.accent_c[0], t.accent_c[1], t.accent_c[2], 0.16 * p);
            let _ = cr.set_source(&a);
            let _ = cr.paint();
        }
        let _ = cr.restore();

        // Hairline edge.
        round_rect(cr, 0.0, 0.0, cw, ch, CAP_RADIUS);
        cr.set_source_rgba(t.key_edge[0], t.key_edge[1], t.key_edge[2], t.key_edge[3] * p);
        cr.set_line_width(1.0);
        let _ = cr.stroke();

        let _ = cr.restore();
    }

    // Hairline under the header and above the footer.
    cr.set_source_rgba(t.stroke[0], t.stroke[1], t.stroke[2], t.stroke[3] * 0.7);
    cr.set_line_width(1.0);
    cr.move_to(24.0, s.header - 12.0);
    cr.line_to(w - 24.0, s.header - 12.0);
    let _ = cr.stroke();
    cr.move_to(24.0, h - s.footer + 4.0);
    cr.line_to(w - 24.0, h - s.footer + 4.0);
    let _ = cr.stroke();

    // Live indicator: pulsing dot.
    let pulse = 0.5 + 0.5 * (s.time * 2.4).sin();
    cr.arc(w - 92.0, s.header / 2.0, 4.0 + pulse * 1.2, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(t.accent_a[0], t.accent_a[1], t.accent_a[2], 0.22 + pulse * 0.20);
    let _ = cr.fill();
    cr.arc(w - 92.0, s.header / 2.0, 3.0, 0.0, std::f64::consts::TAU);
    cr.set_source_rgba(t.accent_a[0], t.accent_a[1], t.accent_a[2], 0.95);
    let _ = cr.fill();

    // If detection failed, tint the board edge so the warning reads at a glance.
    if s.caps.is_empty() {
        round_rect(cr, px, py, board_w + pad * 2.0, board_h + pad * 2.0, 16.0);
        cr.set_source_rgba(0.95, 0.35, 0.35, 0.35);
        cr.set_line_width(2.0);
        let _ = cr.stroke();
    }

}
