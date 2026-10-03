//! Keyscape — shows the keyboard layout you are actually typing on.
//!
//! Everything displayed is read from the system at runtime: the active layout comes
//! from `ibus engine`, and the keymap is compiled from `/usr/share/X11/xkb` by
//! libxkbcommon. No layout data is bundled.
//!
//! GTK4 removed `gdk_cairo_show_layout`, so Pango text cannot be drawn onto a Cairo
//! context at all. The window is therefore an `Overlay`: a `DrawingArea` paints the
//! backdrop and the keycap bodies, and real `gtk::Label`s sit on top for the legends —
//! which also means Arabic and other joining scripts shape correctly for free.

mod deadkeys;
mod geometry;
mod layout;
mod preview;
mod render;
mod theme;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{
    Align, Application, ApplicationWindow, Box as GtkBox, Button, CssProvider, DrawingArea, Label,
    Overlay, Orientation, glib, pango,
};

const PAD: f64 = 26.0;
const GAP: f64 = 7.5;
const HEADER: f64 = 78.0;
const FOOTER: f64 = 44.0;
/// Seconds the staggered entrance runs before we stop animating.
const ENTRANCE: f64 = 1.4;
/// Blob drift repaint interval.
const DRIP: f64 = 1.0 / 24.0;
/// Legend font stack. Used for BOTH the CSS and the Pango metrics query, so the sizes
/// we fit against are the sizes that actually render.
fn legend_fonts() -> String {
    std::env::var("KEYSCAPE_FONTS").unwrap_or_else(|_| {
        "'Cantarell', 'Noto Sans', 'DejaVu Sans', sans-serif".to_string()
    })
}

/// Inset of the corner legends from the cap edge.
pub const LEG_PAD: f64 = 3.0;
/// Pango units per CSS pixel: 1px = 0.75pt at 96dpi, and Pango counts 1024 units per pt.
const PANGO_PER_PX: f64 = 768.0;

/// How far to lift a centred legend, in em, so it reads as optically centred.
///
/// Centring on the ascent/descent box reserves ~0.29em below the baseline for
/// descenders, which pushes the visible mass of a letter that low. Measured on
/// Cantarell/Noto Sans (ascent 1.069em, descent 0.293em, cap height 0.714em) the glyph
/// centre lands 0.324em below the box centre; 0.30em matches that without over-lifting
/// the lowercase-heavy and descender keys.
pub const OPTICAL_RISE: f64 = 0.30;

fn now() -> f64 {
    glib::monotonic_time() as f64 / 1_000_000.0
}

struct State {
    info: layout::LayoutInfo,
    theme_idx: usize,
    /// Seconds since start when the layout last changed; negative once settled.
    changed_at: f64,
    forced: Option<(String, String)>,
    reduced_motion: bool,
    last_draw: f64,
}

struct Ui {
    state: RefCell<State>,
    css: RefCell<CssProvider>,
    area: DrawingArea,
    /// Per-keycap overlay, positioned and sized to the cap rect.
    caps: Vec<Overlay>,
    shift: Vec<Label>,
    base: Vec<Label>,
    altgr: Vec<Label>,
    title: Label,
    subtitle: Label,
    foot_left: Label,
    foot_right: Label,
    swatches: Vec<Button>,
    animating: Cell<bool>,
    unit: Cell<f64>,
    /// Per-cap (base px, small px) legend sizes, fitted to the cap's real line boxes.
    sizes: RefCell<Vec<(f64, f64)>>,
}

fn theme_of(ui: &Ui) -> &'static theme::Theme {
    let st = ui.state.borrow();
    &theme::THEMES[st.theme_idx.min(theme::THEMES.len() - 1)]
}

fn hex(c: [f64; 3]) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c[0].clamp(0.0, 1.0) * 255.0) as u32,
        (c[1].clamp(0.0, 1.0) * 255.0) as u32,
        (c[2].clamp(0.0, 1.0) * 255.0) as u32,
    )
}

fn stylesheet(t: &theme::Theme, unit: f64, sizes: &[(f64, f64)]) -> String {
    // One size rule per cap: the legend box has to fit three line boxes inside one cap,
    // and that budget differs per script, so it cannot be a single global size.
    let mut per_cap = String::new();
    for (i, (base_px, small_px)) in sizes.iter().enumerate() {
        // A `valign: center` label rises by half its bottom margin, so double the lift.
        let lift = (base_px * OPTICAL_RISE * 2.0).round();
        per_cap.push_str(&format!(
            ".b{i} {{ font-size: {}px; margin-top: {LEG_PAD}px; margin-bottom: {}px; }}
             .s{i} {{ font-size: {}px; }}\n",
            base_px.max(6.0).round(),
            (LEG_PAD + lift).round(),
            small_px.max(6.0).round(),
        ));
    }
    let _ = unit;
    let mut swatches = String::new();
    for (i, s) in theme::THEMES.iter().enumerate() {
        swatches.push_str(&format!(
            ".sw{i} {{ background-color: {}; }}\n.sw{i}.on {{ border: 2px solid {}; }}\n",
            hex(s.accent_a),
            hex(s.accent_c),
        ));
    }
    format!(
        "label {{ font-family: {}; }}
         .leg-base  {{ font-weight: 700; color: {}; }}
         .leg-mod   {{ font-weight: 500; color: {}; }}
         .leg-shift {{ font-weight: 600; color: {}; }}
         .leg-altgr {{ font-weight: 700; color: {}; }}
         {per_cap}
         .title     {{ font-size: 22px; font-weight: 700; color: {}; }}
         .subtitle  {{ font-size: 12px; font-weight: 500; color: {}; }}
         .brand     {{ font-size: 11px; font-weight: 700; color: {}; }}
         .foot      {{ font-size: 11px; font-weight: 500; color: {}; }}
         .warn      {{ font-size: 11px; font-weight: 600; color: #ff9a9a; }}
         .swatch    {{ min-width: 14px; min-height: 14px; padding: 0;
                        border-radius: 8px; border: 2px solid transparent; }}
         {swatches}",
        legend_fonts(),
        hex(t.txt_base),
        hex(t.txt_dim),
        hex(t.txt_dim),
        hex(t.accent_c),
        hex(t.txt_hi),
        hex(t.txt_dim),
        hex(t.accent_a),
        hex(t.txt_dim),
    )
}

/// Update legend text in place. Labels are created once and reused so the window does
/// not churn widgets every time the layout changes.
fn sync_caps(ui: &Ui) {
    let st = ui.state.borrow();
    for (i, cap) in st.info.caps.iter().enumerate() {
        let set = |slot: &Vec<Label>, text: &Option<String>| {
            let Some(l) = slot.get(i) else { return };
            match text {
                Some(t) => {
                    if l.text() != t.as_str() {
                        l.set_text(t);
                    }
                    l.set_visible(true);
                }
                None => l.set_visible(false),
            }
        };
        set(&ui.shift, &cap.shift);
        set(&ui.altgr, &cap.altgr);
        if let Some(l) = ui.base.get(i) {
            let class = if cap.modifier { "leg-mod" } else { "leg-base" };
            if l.css_classes().iter().all(|c| c != class) {
                l.remove_css_class(if cap.modifier { "leg-base" } else { "leg-mod" });
                l.add_css_class(class);
            }
            match &cap.base {
                Some(t) => {
                    if l.text() != t.as_str() {
                        l.set_text(t);
                    }
                    l.set_visible(true);
                }
                None => l.set_visible(false),
            }
        }
    }
}

fn sync_text(ui: &Ui) {
    let st = ui.state.borrow();
    let info = &st.info;
    ui.title.set_text(&info.display_name);
    ui.subtitle.set_text(&info.subtitle());
    let theme_name = theme::THEMES[st.theme_idx.min(theme::THEMES.len() - 1)].name;
    // A layout we could not compile must say so, not just show the wrong board.
    match &info.error {
        Some(err) => {
            ui.foot_left.set_text(err);
            ui.foot_left.remove_css_class("foot");
            ui.foot_left.add_css_class("warn");
        }
        None => {
            ui.foot_left.set_text(&format!(
                "via {} · {} keys · ANSI 60%",
                info.source,
                info.caps.len()
            ));
            ui.foot_left.remove_css_class("warn");
            ui.foot_left.add_css_class("foot");
        }
    }
    ui.foot_right.set_text(&format!("{theme_name} · Super+Space to switch"));
    for (i, b) in ui.swatches.iter().enumerate() {
        let want = if i == st.theme_idx { "on" } else { "" };
        let has = b.css_classes().iter().any(|c| c == "on");
        if want == "on" && !has {
            b.add_css_class("on");
        } else if want != "on" && has {
            b.remove_css_class("on");
        }
    }
}

/// Line box, in px, that Pango gives a legend at `px` for the given script.
///
/// This is measured rather than assumed because the fallback fonts vary wildly:
/// Cantarell reports 1.362em, DejaVu Sans 1.164em, but Noto Sans Arabic **2.112em** --
/// so an Arabic label box alone can exceed a whole keycap. Passing the language is what
/// makes Pango resolve the same font the label will actually use.
fn line_px(ctx: &pango::Context, px: f64, lang: Option<&str>) -> f64 {
    let mut desc = pango::FontDescription::from_string(&format!("{} 12px", legend_fonts()));
    desc.set_absolute_size(px * PANGO_PER_PX);
    let language = lang.map(pango::Language::from_string);
    ctx.metrics(Some(&desc), language.as_ref()).height() as f64 / PANGO_PER_PX
}

/// Best-effort script tag for a legend, used only to hint font selection.
fn script_lang(text: &str) -> Option<&'static str> {
    for c in text.chars() {
        let u = c as u32;
        if matches!(u, 0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFEFF) {
            return Some("ar");
        }
        if (0x0590..=0x05FF).contains(&u) {
            return Some("he");
        }
        if (0x0900..=0x097F).contains(&u) {
            return Some("hi");
        }
        if (0x0E00..=0x0E7F).contains(&u) {
            return Some("th");
        }
        if (0x3040..=0x30FF).contains(&u) {
            return Some("ja");
        }
        if matches!(u, 0x4E00..=0x9FFF) || matches!(u, 0xAC00..=0xD7AF) {
            return Some("zh");
        }
        if (0x0370..=0x03FF).contains(&u) {
            return Some("el");
        }
        if (0x0400..=0x04FF).contains(&u) {
            return Some("ru");
        }
    }
    None
}

/// Nominal legend sizes as a fraction of the key unit.
pub const BASE_EM: f64 = 0.40;
pub const CORNER_EM: f64 = 0.28;

/// Fit every cap's legends inside its own cap.
///
/// Only a *single* legend box has to fit: the layout is diagonal, with L1 top-left and
/// L2 bottom-right around a centred L0, so the corner boxes may overlap L0 vertically as
/// long as their ink stays horizontally clear -- which single-character legends always
/// are. Summing the three line boxes instead (the obvious reading) would wrongly shrink
/// Latin from 25.6px to 19.6px to satisfy a collision that cannot happen.
fn compute_sizes(ui: &Ui, ctx: &pango::Context) -> Vec<(f64, f64)> {
    let unit = ui.unit.get();
    let avail = unit - LEG_PAD * 2.0;
    let base0 = unit * BASE_EM;
    let small0 = unit * CORNER_EM;

    let caps: Vec<Option<String>> = {
        let st = ui.state.borrow();
        st.info.caps.iter().map(|c| c.base.clone()).collect()
    };

    caps.iter()
        .map(|base| {
            let lang = base.as_deref().and_then(script_lang);
            let kb = if line_px(ctx, base0, lang) > avail {
                avail / line_px(ctx, base0, lang)
            } else {
                1.0
            };
            let ks = if line_px(ctx, small0, lang) > avail {
                avail / line_px(ctx, small0, lang)
            } else {
                1.0
            };
            (base0 * kb, small0 * ks)
        })
        .collect()
}

/// Reposition and resize every cap overlay to match the fitted board.
fn place_caps(ui: &Ui, m: &geometry::Metrics) {
    for (i, slot) in geometry::slots().iter().enumerate() {
        let Some(cap) = ui.caps.get(i) else { continue };
        let (x, y, w, h) = m.rect(slot);
        cap.set_margin_start(x.round() as i32);
        cap.set_margin_top(y.round() as i32);
        cap.set_size_request(w.round() as i32, h.round() as i32);
    }
}

fn apply_opacity(ui: &Ui, elapsed: f64) {
    for (i, cap) in ui.caps.iter().enumerate() {
        let p = ease(elapsed, i as f64);
        cap.set_opacity(p);
    }
}

fn ease(elapsed: f64, index: f64) -> f64 {
    if elapsed < 0.0 {
        return 1.0;
    }
    let t = ((elapsed - index * 0.010) / 0.34).clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Push current theme + key size into CSS.
fn refresh_css(ui: &Ui) {
    let sizes = ui.sizes.borrow().clone();
    let css = stylesheet(theme_of(ui), ui.unit.get(), &sizes);
    if let Ok(p) = ui.css.try_borrow_mut() {
        p.load_from_data(&css);
    }
}

/// Reload the layout and restart the entrance animation.
fn reload(ui: &Rc<Ui>) {
    {
        let mut st = ui.state.borrow_mut();
        st.info = layout::load(st.forced.clone());
        st.changed_at = now();
        st.last_draw = 0.0;
    }
    sync_caps(ui);
    sync_text(ui);
    ui.area.queue_draw();
    arm(ui);
}

/// Start the frame-clock driver if it is not already running.
fn arm(ui: &Rc<Ui>) {
    if ui.animating.replace(true) {
        return;
    }
    let inner = ui.clone();
    let ticker = ui.area.clone();
    ticker.add_tick_callback(move |_, _| {
        let t = now();
        let (settling, drifting, due, elapsed) = {
            let mut st = inner.state.borrow_mut();
            let elapsed = if st.changed_at < 0.0 {
                -1.0
            } else {
                let e = t - st.changed_at;
                if e > ENTRANCE {
                    st.changed_at = -1.0;
                    -1.0
                } else {
                    e
                }
            };
            let due = t - st.last_draw >= DRIP;
            if due {
                st.last_draw = t;
            }
            (elapsed >= 0.0, !st.reduced_motion, due, elapsed)
        };

        if settling {
            apply_opacity(&inner, elapsed);
            inner.area.queue_draw();
            return glib::ControlFlow::Continue;
        }
        if drifting && due {
            inner.area.queue_draw();
            return glib::ControlFlow::Continue;
        }
        inner.animating.set(false);
        glib::ControlFlow::Break
    });
}

fn set_theme(ui: &Rc<Ui>, idx: usize) {
    ui.state.borrow_mut().theme_idx = idx;
    refresh_css(ui);
    sync_text(ui);
    ui.area.queue_draw();
}

fn build_window(app: &Application, forced: Option<(String, String)>, check: bool) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Keyscape")
        .default_width(1280)
        .default_height(660)
        .build();

    // GTK exposes the desktop "reduce animation" preference; honour it plus an override.
    let reduced = gtk::Settings::default().is_none_or(|s| !s.is_gtk_enable_animations())
        || std::env::var("KEYSCAPE_NO_MOTION").is_ok();

    let area = DrawingArea::new();
    area.set_hexpand(true);
    area.set_vexpand(true);

    let overlay = Overlay::new();
    overlay.set_child(Some(&area));
    overlay.set_hexpand(true);
    overlay.set_vexpand(true);

    // One small overlay per keycap: sized and positioned to the cap rect, with the
    // legends as aligned children so GTK does the placement.
    let mut caps = Vec::new();
    let mut shift = Vec::new();
    let mut base = Vec::new();
    let mut altgr = Vec::new();
    for i in 0..geometry::slots().len() {
        let cap = Overlay::new();
        cap.set_halign(Align::Start);
        cap.set_valign(Align::Start);
        // Give it a non-zero size up front: a zero-sized child overlay has no
        // allocation, and its labels warn when they try to snapshot themselves.
        cap.set_size_request(1, 1);

        let mk = |class: &str, h: Align, v: Align, m: i32| {
            let l = Label::new(None);
            l.add_css_class(class);
            l.set_halign(h);
            l.set_valign(v);
            l.set_margin_start(m);
            l.set_margin_end(m);
            l.set_margin_top(m);
            l.set_margin_bottom(m);
            l.set_single_line_mode(true);
            l.set_ellipsize(gtk::pango::EllipsizeMode::None);
            l
        };
        let ls = mk("leg-shift", Align::Start, Align::Start, 3);
        ls.add_css_class(&format!("s{i}"));
        let lb = mk("leg-base", Align::Center, Align::Center, 0);
        lb.add_css_class(&format!("b{i}"));
        let la = mk("leg-altgr", Align::End, Align::End, 3);
        la.add_css_class(&format!("s{i}"));
        cap.set_child(Some(&ls));
        cap.add_overlay(&lb);
        cap.add_overlay(&la);
        overlay.add_overlay(&cap);
        caps.push(cap);
        shift.push(ls);
        base.push(lb);
        altgr.push(la);
    }

    // Header: brand, name, subtitle.
    let brand = Label::new(Some("KEYSCAPE"));
    brand.add_css_class("brand");
    brand.set_valign(Align::Center);
    let title = Label::new(None);
    title.add_css_class("title");
    let subtitle = Label::new(None);
    subtitle.add_css_class("subtitle");
    let titles = GtkBox::new(Orientation::Vertical, 0);
    titles.set_valign(Align::Center);
    titles.append(&title);
    titles.append(&subtitle);
    let head = GtkBox::new(Orientation::Horizontal, 12);
    head.set_halign(Align::Start);
    head.set_valign(Align::Start);
    head.set_margin_start(26);
    head.set_margin_top(20);
    head.append(&brand);
    head.append(&titles);
    let header_ov = Overlay::new();
    header_ov.set_child(Some(&head));
    header_ov.set_halign(Align::Fill);
    header_ov.set_valign(Align::Start);
    overlay.add_overlay(&header_ov);

    // Footer: provenance on the left, theme name and hint on the right.
    let foot_left = Label::new(None);
    foot_left.add_css_class("foot");
    let foot_right = Label::new(None);
    foot_right.add_css_class("foot");
    foot_right.set_hexpand(true);
    foot_right.set_xalign(1.0);
    let foot = GtkBox::new(Orientation::Horizontal, 12);
    foot.set_halign(Align::Fill);
    foot.set_valign(Align::End);
    foot.set_margin_start(26);
    foot.set_margin_end(26);
    foot.set_margin_bottom(16);
    foot.append(&foot_left);
    foot.append(&foot_right);
    let footer_ov = Overlay::new();
    footer_ov.set_child(Some(&foot));
    footer_ov.set_halign(Align::Fill);
    footer_ov.set_valign(Align::End);
    overlay.add_overlay(&footer_ov);

    // Theme swatches, top right.
    let mut swatches = Vec::new();
    let swatch_row = GtkBox::new(Orientation::Horizontal, 7);
    swatch_row.set_halign(Align::End);
    swatch_row.set_valign(Align::Start);
    swatch_row.set_margin_end(26);
    swatch_row.set_margin_top(22);
    for (i, t) in theme::THEMES.iter().enumerate() {
        let b = Button::new();
        b.add_css_class("swatch");
        b.add_css_class(&format!("sw{i}"));
        b.set_tooltip_text(Some(t.name));
        swatch_row.append(&b);
        swatches.push(b);
    }
    let swatch_ov = Overlay::new();
    swatch_ov.set_child(Some(&swatch_row));
    swatch_ov.set_halign(Align::Fill);
    swatch_ov.set_valign(Align::Start);
    overlay.add_overlay(&swatch_ov);

    let ui = Rc::new(Ui {
        state: RefCell::new(State {
            info: layout::LayoutInfo {
                caps: Vec::new(),
                display_name: String::new(),
                layout: String::new(),
                variant: String::new(),
                source: String::new(),
                error: None,
            },
            theme_idx: 0,
            changed_at: now(),
            forced,
            reduced_motion: reduced,
            last_draw: 0.0,
        }),
        css: RefCell::new(CssProvider::new()),
        area: area.clone(),
        caps,
        shift,
        base,
        altgr,
        title,
        subtitle,
        foot_left,
        foot_right,
        swatches,
        animating: Cell::new(false),
        unit: Cell::new(0.0),
        sizes: RefCell::new(Vec::new()),
    });

    for b in ui.swatches.iter() {
        let ui = ui.clone();
        let idx = ui
            .swatches
            .iter()
            .position(|x| std::ptr::eq(x, b))
            .unwrap_or(0);
        b.connect_clicked(move |_| set_theme(&ui, idx));
    }

    {
        let ui = ui.clone();
        area.set_draw_func(move |a, cr, w, h| {
            let (w, h) = (w as f64, h as f64);
            let m = geometry::metrics(w, h, PAD, GAP, HEADER, FOOTER);

            if (m.unit - ui.unit.get()).abs() > 0.5 {
                ui.unit.set(m.unit);
                *ui.sizes.borrow_mut() = compute_sizes(&ui, &a.pango_context());
                refresh_css(&ui);
                place_caps(&ui, &m);
            }

            let st = ui.state.borrow();
            let elapsed = if st.changed_at < 0.0 {
                -1.0
            } else {
                now() - st.changed_at
            };
            let scene = render::Scene {
                caps: &st.info.caps,
                theme: &theme::THEMES[st.theme_idx.min(theme::THEMES.len() - 1)],
                metrics: m,
                time: now(),
                entrance: elapsed,
                header: HEADER,
                footer: FOOTER,
            };
            render::paint(cr, w, h, &scene);
            let _ = a;
        });
    }

    window.set_child(Some(&overlay));
    window.style_context().add_provider(&ui.css.borrow().clone(), 1000);
    window.set_default_size(1280, 660);
    window.set_size_request(680, 460);

    sync_caps(&ui);
    sync_text(&ui);
    // Fit once up front so the first frame already has correct cap geometry.
    place_caps(&ui, &geometry::metrics(1280.0, 660.0, PAD, GAP, HEADER, FOOTER));
    ui.unit.set(geometry::metrics(1280.0, 660.0, PAD, GAP, HEADER, FOOTER).unit);
    reload(&ui);
    *ui.sizes.borrow_mut() = compute_sizes(&ui, &area.pango_context());
    refresh_css(&ui);

    // Watch for layout switches. `ibus engine` is the only signal measured to track a
    // real switch; GNOME's `input-sources current` goes stale.
    {
        let ui = ui.clone();
        glib::timeout_add_local(Duration::from_millis(1200), move || {
            if let Some((l, v)) = layout::active() {
                let changed = {
                    let st = ui.state.borrow();
                    st.info.layout != l || st.info.variant != v
                };
                if changed {
                    reload(&ui);
                }
            }
            glib::ControlFlow::Continue
        });
    }

    // Keep the board fitted while the window is resized or dragged between monitors.
    let refresh = ui.clone();
    area.add_tick_callback(move |_, _| {
        refresh.area.queue_draw();
        glib::ControlFlow::Break
    });

    if check {
        let ui = ui.clone();
        let win = window.clone();
        glib::timeout_add_local(Duration::from_millis(800), move || {
            report(&ui);
            win.close();
            glib::ControlFlow::Break
        });
    }

    window.present();
}

/// Print the live widget geometry. The preview renderer cannot prove that GTK actually
/// allocates and aligns the legend labels, so this checks the real tree.
fn report(ui: &Rc<Ui>) {
    let st = ui.state.borrow();
    println!("layout      : {} ({})", st.info.display_name, st.info.subtitle());
    println!("theme       : {}", theme_of(ui).name);
    println!("key unit    : {:.1}px", ui.unit.get());
    println!("caps        : {}", ui.caps.len());
    let ctx = ui.area.pango_context();
    let unit = ui.unit.get();
    let avail = unit - LEG_PAD * 2.0;
    for (tag, lang) in [("Latin", None), ("Arabic", Some("ar"))] {
        let b = line_px(&ctx, unit * 0.40, lang);
        let sm = line_px(&ctx, unit * 0.235, lang);
        println!(
            "metrics {tag:<7}: base line {b:5.1}px  small line {sm:5.1}px  stack {:5.1}px  avail {avail:5.1}px",
            b + 2.0 * sm
        );
    }
    // Report the tightest-fitted cap, which is the one carrying all three legends.
    {
        let sizes = ui.sizes.borrow();
        let three: Vec<usize> = {
            let st = ui.state.borrow();
            st.info
                .caps
                .iter()
                .enumerate()
                .filter(|(_, c)| c.shift.is_some() && c.altgr.is_some())
                .map(|(i, _)| i)
                .collect()
        };
        if let Some(&i) = three.first()
            && let Some((b, sm)) = sizes.get(i)
        {
            println!("fitted 3-legend: base {b:.1}px  small {sm:.1}px  (cap {i})");
        }
        let min_b = sizes.iter().map(|(b, _)| *b).fold(f64::MAX, f64::min);
        let min_s = sizes.iter().map(|(_, s)| *s).fold(f64::MAX, f64::min);
        println!("smallest      : base {min_b:.1}px  small {min_s:.1}px");
    }
    println!();

    let mut empty = 0usize;
    let mut legends = 0usize;
    for (i, cap) in ui.caps.iter().enumerate() {
        let a = cap.allocation();
        if a.width() <= 1 || a.height() <= 1 {
            empty += 1;
        }
        let mut texts = Vec::new();
        let mut child = cap.first_child();
        while let Some(k) = child {
            if let Some(l) = k.downcast_ref::<Label>() {
                let t = l.text().to_string();
                if !t.is_empty() {
                    legends += 1;
                    // Overlay children are allocated in the cap's own coordinate
                    // space, so compare against the cap's size, not its position.
                    let la = k.allocation();
                    let inside = la.x() >= 0
                        && la.y() >= 0
                        && la.x() + la.width() <= a.width()
                        && la.y() + la.height() <= a.height();
                    texts.push(format!("{t:?}{}", if inside { "" } else { "  <-- OUTSIDE CAP" }));
                }
            }
            child = k.next_sibling();
        }
        if i < 3 || i >= ui.caps.len() - 2 {
            println!(
                "cap{i:<3} alloc={:>4}x{:<4} at ({:>4},{:<4})  {}",
                a.width(),
                a.height(),
                a.x(),
                a.y(),
                texts.join("  ")
            );
        }
    }
    println!();
    println!("empty allocations : {empty}");
    println!("legends placed    : {legends}");
    println!(
        "verdict           : {}",
        if empty == 0 && legends > 40 { "PASS" } else { "FAIL" }
    );
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    // `--render <file.png> [theme]` draws one frame headlessly, no display needed.
    if let Some(pos) = argv.iter().position(|a| a == "--render") {
        let file = argv.get(pos + 1).cloned().unwrap_or_else(|| "keyscape.png".into());
        let theme_idx = argv.get(pos + 2).and_then(|s| s.parse().ok()).unwrap_or(0);
        let forced = argv
            .get(pos + 3)
            .map(|l| (l.clone(), argv.get(pos + 4).cloned().unwrap_or_default()));
        preview::to_png(
            &file,
            theme_idx,
            forced,
            preview::Fit { w: 1280.0, h: 660.0, pad: PAD, gap: GAP, header: HEADER, footer: FOOTER },
        );
        return;
    }

    if argv.iter().any(|a| a == "--selftest") {
        let app = Application::builder()
            .application_id("tech.sadique.keyscape")
            .build();
        app.connect_activate(|app| build_window(app, None, true));
        app.run_with_args::<&str>(&[]);
        return;
    }

    let args: Vec<String> = argv.iter().filter(|a| !a.starts_with('-')).cloned().collect();
    let forced = args
        .first()
        .map(|l| (l.clone(), args.get(1).cloned().unwrap_or_default()));

    let app = Application::builder()
        .application_id("tech.sadique.keyscape")
        .build();
    app.connect_activate(move |app| build_window(app, forced.clone(), false));
    app.run_with_args::<&str>(&[]);
}
