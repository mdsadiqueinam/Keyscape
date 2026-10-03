//! Glyphs for dead keys.
//!
//! `xkb::keysym_to_utf8` returns an empty string for *every* `dead_*` keysym
//! (verified across the whole set), so without this table the UI would print the
//! literal word `dead_acute` on a keycap. Dead keys matter: German, French,
//! Spanish and Brazilian layouts are full of them.

/// Map a keysym name to the standalone character it looks like when typed alone.
pub fn glyph(keysym_name: &str) -> Option<&'static str> {
    Some(match keysym_name {
        "dead_grave" => "`",
        "dead_acute" => "\u{00B4}",
        "dead_circumflex" => "^",
        "dead_tilde" => "~",
        "dead_macron" => "\u{02C9}",
        "dead_breve" => "\u{02D8}",
        "dead_abovedot" => "\u{02D9}",
        "dead_diaeresis" => "\u{00A8}",
        "dead_abovering" => "\u{02DA}",
        "dead_doubleacute" => "\u{02DD}",
        "dead_caron" => "\u{02C7}",
        "dead_cedilla" => "\u{00B8}",
        "dead_ogonek" => "\u{02DB}",
        "dead_iota" => "\u{0269}",
        "dead_voiced_sound" => "\u{030C}",
        "dead_semivoiced_sound" => "\u{031B}",
        "dead_belowdot" => "\u{0323}",
        "dead_hook" => "\u{0309}",
        "dead_horn" => "\u{030B}",
        "dead_stroke" => "\u{0335}",
        "dead_greek" => "\u{0375}",
        "dead_abovecomma" => "\u{0312}",
        "dead_abovereversedcomma" => "\u{0313}",
        "dead_doublecircumflex" => "\u{02C6}",
        "dead_belowcircumflex" => "\u{032F}",
        "dead_belowtilde" => "\u{0330}",
        "dead_belowmacron" => "\u{0331}",
        "dead_lowline" => "\u{0332}",
        _ => return None,
    })
}
