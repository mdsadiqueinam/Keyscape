//! Detecting the active layout and turning its keymap into drawable caps.

use crate::deadkeys;
use crate::geometry::{self, slots};
use std::process::Command;
use xkbcommon::xkb::{Context, Keymap};

/// One drawable keycap: legends only. Placement comes from `geometry::slots()`.
pub struct Cap {
    /// Centre legend: the character, or a symbol glyph for modifier keys.
    pub base: Option<String>,
    /// Small legend, top-left.
    pub shift: Option<String>,
    /// Small legend, bottom-right in the accent colour.
    pub altgr: Option<String>,
    pub modifier: bool,
}

/// Everything the UI needs to describe the current layout.
pub struct LayoutInfo {
    pub caps: Vec<Cap>,
    /// Human name from the keymap, e.g. "English (US)".
    pub display_name: String,
    /// Raw xkb id and variant, e.g. "ara" / "digits".
    pub layout: String,
    pub variant: String,
    /// Which signal told us: "ibus", "gsettings" or "fallback".
    pub source: String,
    /// Set when detection worked but the layout could not be compiled.
    pub error: Option<String>,
}

impl LayoutInfo {
    pub fn subtitle(&self) -> String {
        match self.variant.is_empty() {
            true => self.layout.clone(),
            false => format!("{} · {}", self.layout, self.variant),
        }
    }
}

fn sh(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() { None } else { Some(s) }
}

/// Ask the desktop which layout is active.
///
/// `ibus engine` reports `xkb:<layout>:<variant>:<options>` and is the only signal
/// measured to track a real layout switch. GNOME's `input-sources current` goes stale,
/// and `setxkbmap -query` can only ever report the first configured layout, so neither
/// is trusted here.
fn detect() -> (String, String, String) {
    if let Some(engine) = sh("ibus", &["engine"]) {
        let mut parts = engine.split(':');
        if parts.next() == Some("xkb")
            && let Some(layout) = parts.next().filter(|s| !s.is_empty())
        {
            let variant = parts.next().unwrap_or("");
            return (layout.to_string(), variant.to_string(), "ibus".into());
        }
    }

    if let (Some(cur), Some(sources)) = (
        sh("gsettings", &["get", "org.gnome.desktop.input-sources", "current"]),
        sh("gsettings", &["get", "org.gnome.desktop.input-sources", "sources"]),
    ) {
        let ids: Vec<&str> = sources
            .split("('xkb', '")
            .skip(1)
            .filter_map(|s| s.split('\'').next())
            .collect();
        let idx: usize = cur
            .trim_start_matches("uint32 ")
            .trim()
            .parse()
            .unwrap_or(0);
        if let Some(id) = ids.get(idx) {
            let first = id.split('+').next().unwrap_or(id);
            let (l, v) = match first.split_once('(') {
                Some((l, v)) => (l.to_string(), v.trim_end_matches(')').to_string()),
                None => (first.to_string(), String::new()),
            };
            return (l, v, "gsettings".into());
        }
    }

    ("us".into(), String::new(), "fallback".into())
}

/// A legend for one key at one shift level, or `None` if there is nothing to show.
fn label(km: &Keymap, name: &str, level: u32) -> Option<String> {
    let kc = km.key_by_name(name)?;
    let sym = *km.key_get_syms_by_level(kc, 0, level).first()?;
    if sym.raw() == 0 {
        return None;
    }
    // Dead keys have no UTF-8 form at all, so look them up by name first.
    let keysym_name = xkbcommon::xkb::keysym_get_name(sym);
    if let Some(g) = deadkeys::glyph(&keysym_name) {
        return Some(g.to_string());
    }
    let utf8 = xkbcommon::xkb::keysym_to_utf8(sym);
    let text: String = utf8.chars().filter(|c| !c.is_control()).collect();
    let text = text.trim_end().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// The layout currently in effect, for cheap polling. `None` when no signal gave a
/// real answer, so callers can avoid mistaking the hard-coded fallback for truth.
pub fn active() -> Option<(String, String)> {
    let (layout, variant, source) = detect();
    match source.as_str() {
        "fallback" => None,
        _ => Some((layout, variant)),
    }
}

/// Detect, compile and describe the active layout.
pub fn load(force: Option<(String, String)>) -> LayoutInfo {
    let (layout, variant, source) = match force {
        Some((l, v)) => (l, v, "argument".to_string()),
        None => detect(),
    };

    // NOTE: do not touch the include path. A fresh Context already points at
    // /usr/share/X11/xkb; `include_path_append_default()` clears it on Debian builds
    // and adds nothing back, and `include_path_append()` is broken in this crate.
    let ctx = Context::new(0);

    let build = |km: Option<&Keymap>| {
        let mut out = Vec::new();
        let Some(km) = km else { return out };
        for slot in slots() {
            let modifier = geometry::is_modifier(slot.name);
            let base = if modifier {
                geometry::modifier_glyph(slot.name)
                    .filter(|g| !g.is_empty())
                    .map(str::to_string)
                    .or_else(|| label(km, slot.name, 0))
            } else {
                label(km, slot.name, 0)
            };
            out.push(Cap {
                base,
                shift: label(km, slot.name, 1),
                altgr: label(km, slot.name, 2).or_else(|| label(km, slot.name, 3)),
                modifier,
            });
        }
        out
    };

    // `pc105` is deliberate: it forces ANSI structure for every layout, which keeps the
    // drawn board identical while legends stay correct (see geometry.rs).
    match Keymap::new_from_names(&ctx, "evdev", "pc105", &layout, &variant, None, 0) {
        Some(km) => {
            let display_name = km.layout_get_name(0).to_string();
            LayoutInfo {
                caps: build(Some(&km)),
                display_name,
                layout,
                variant,
                source,
                error: None,
            }
        }
        None => {
            let missing = format!(
                "this system has no XKB data for '{layout}' — \
                 install a matching keyboard layout to use it here"
            );
            LayoutInfo {
                caps: build(
                    Keymap::new_from_names(&ctx, "evdev", "pc105", "us", "", None, 0).as_ref(),
                ),
                display_name: "unavailable".into(),
                layout,
                variant,
                source,
                error: Some(missing),
            }
        }
    }
}
