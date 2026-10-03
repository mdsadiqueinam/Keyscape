//! Physical key placement for a 60% board.
//!
//! This is the *only* hand-written table in the app, and it holds no layout data —
//! just which XKB key name sits where, and how wide it is. Every legend comes from
//! the keymap compiled at runtime.
//!
//! Geometry is always ANSI. `model: pc105` forces ANSI structure regardless of the
//! layout: compiling `gb`, `de` and `fr` under it yields exactly the same key names
//! and keycodes as `us` (`AB11` carries no symbol on any of them), differing only in
//! legends — which are correct. So ISO boards are drawn with an ANSI silhouette:
//! legends stay right, the Enter shape is cosmetic.

use std::sync::OnceLock;

/// One keycap placed on the board, in keyboard units.
pub struct Slot {
    pub name: &'static str,
    pub row: usize,
    /// Left edge in units, including accumulated inter-key gaps.
    pub x_units: f64,
    /// Width in units.
    pub w_units: f64,
    /// Row width, used to centre the board.
    pub row_units: f64,
}

const fn raw(name: &'static str, w: f64) -> (&'static str, f64) {
    (name, w)
}

/// Board rows before placement. Every row totals exactly 15 units.
const RAW_ROWS: &[&[(&str, f64)]] = &[
    &[
        raw("ESC", 1.0),
        raw("AE01", 1.0), raw("AE02", 1.0), raw("AE03", 1.0), raw("AE04", 1.0), raw("AE05", 1.0),
        raw("AE06", 1.0), raw("AE07", 1.0), raw("AE08", 1.0), raw("AE09", 1.0), raw("AE10", 1.0),
        raw("AE11", 1.0), raw("AE12", 1.0),
        raw("BKSP", 2.0),
    ],
    &[
        raw("TAB", 1.5),
        raw("AD01", 1.0), raw("AD02", 1.0), raw("AD03", 1.0), raw("AD04", 1.0), raw("AD05", 1.0),
        raw("AD06", 1.0), raw("AD07", 1.0), raw("AD08", 1.0), raw("AD09", 1.0), raw("AD10", 1.0),
        raw("AD11", 1.0), raw("AD12", 1.0),
        raw("BKSL", 1.5),
    ],
    &[
        raw("CAPS", 1.75),
        raw("AC01", 1.0), raw("AC02", 1.0), raw("AC03", 1.0), raw("AC04", 1.0), raw("AC05", 1.0),
        raw("AC06", 1.0), raw("AC07", 1.0), raw("AC08", 1.0), raw("AC09", 1.0),
        raw("AC10", 1.0), raw("AC11", 1.0),
        raw("RTRN", 2.25),
    ],
    &[
        raw("LFSH", 2.25),
        raw("AB01", 1.0), raw("AB02", 1.0), raw("AB03", 1.0), raw("AB04", 1.0), raw("AB05", 1.0),
        raw("AB06", 1.0), raw("AB07", 1.0), raw("AB08", 1.0), raw("AB09", 1.0), raw("AB10", 1.0),
        raw("RTSH", 2.75),
    ],
    &[
        raw("LCTL", 1.25), raw("LWIN", 1.25), raw("LALT", 1.25),
        raw("SPCE", 6.25),
        raw("RALT", 1.25), raw("RWIN", 1.25), raw("RCTL", 1.25), raw("COMP", 1.25),
    ],
];

/// Rows in board order. Built once; positions accumulate so every row ends at exactly
/// `UNITS` units plus the same number of gaps.
pub fn slots() -> &'static [Slot] {
    static SLOTS: OnceLock<Vec<Slot>> = OnceLock::new();
    SLOTS.get_or_init(|| {
        let mut out = Vec::new();
        for (row, keys) in RAW_ROWS.iter().enumerate() {
            let mut x = 0.0;
            for (name, w) in keys.iter() {
                out.push(Slot {
                    name,
                    row,
                    x_units: x,
                    w_units: *w,
                    row_units: 0.0,
                });
                x += *w;
            }
        }
        let row_units: f64 = RAW_ROWS.iter().map(|r| r.iter().map(|(_, w)| *w).sum()).fold(0.0, f64::max);
        for slot in &mut out {
            slot.row_units = row_units;
        }
        out
    })
}

pub const UNITS: f64 = 15.0;
pub const ROW_COUNT: f64 = 5.0;

/// Screen geometry derived from the window size.
#[derive(Clone, Copy)]
pub struct Metrics {
    pub unit: f64,
    pub gap: f64,
    pub x0: f64,
    pub y0: f64,
}

impl Metrics {
    /// Pixel rect of a slot.
    pub fn rect(&self, s: &Slot) -> (f64, f64, f64, f64) {
        let x = self.x0 + s.x_units * self.unit;
        let y = self.y0 + s.row as f64 * (self.unit + self.gap);
        let w = s.w_units * self.unit;
        (x, y, w, self.unit)
    }
}

/// Fit the board into the given area, leaving room for header and footer.
pub fn metrics(w: f64, h: f64, pad: f64, gap: f64, header: f64, footer: f64) -> Metrics {
    let avail_w = (w - pad * 2.0 - gap * (UNITS - 1.0)).max(40.0);
    let avail_h = (h - pad * 2.0 - header - footer - gap * (ROW_COUNT - 1.0)).max(40.0);
    let unit = (avail_w / UNITS).min(avail_h / ROW_COUNT).clamp(18.0, 64.0);
    let board_w = UNITS * unit + (UNITS - 1.0) * gap;
    let board_h = ROW_COUNT * unit + (ROW_COUNT - 1.0) * gap;
    Metrics {
        unit,
        gap,
        x0: (w - board_w) / 2.0,
        y0: header + pad + ((h - header - footer - pad * 2.0) - board_h).max(0.0) / 2.0,
    }
}

/// Legend text for non-character keys.
///
/// Deliberately plain ASCII words rather than the usual symbols (⎋ ⌫ ⇥ ⇧ ⌃ ⌥ ⌘).
/// Measured across the 344 fonts installed here: `^` exists in 148 of them but ⇧ in only
/// 13 and ⎋ in 10. Pango would paper over that on this machine via font fallback and
/// then render tofu on a leaner one. Printed words are what real keycap sets use and
/// they cannot break.
pub fn modifier_glyph(name: &str) -> Option<&'static str> {
    Some(match name {
        "ESC" => "ESC",
        "BKSP" => "BKSP",
        "TAB" => "TAB",
        "CAPS" => "CAPS",
        "RTRN" => "ENTER",
        "LFSH" | "RTSH" => "SHIFT",
        "LCTL" | "RCTL" => "CTRL",
        "LALT" | "RALT" => "ALT",
        "LWIN" | "RWIN" => "META",
        "COMP" => "MENU",
        "SPCE" => "",
        _ => return None,
    })
}

/// True when a key has no base character and should render as a single glyph.
pub fn is_modifier(name: &str) -> bool {
    modifier_glyph(name).is_some()
}
