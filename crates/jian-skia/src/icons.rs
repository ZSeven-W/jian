//! Bundled icon glyph paths — Lucide-compatible SVG path data for the
//! subset of icons the runtime needs to render MVP designs.
//!
//! Each entry is the raw `d=` attribute from the upstream Lucide icon
//! (24×24 viewBox, stroke-based line art). At draw time the backend
//! parses the string into a `skia_safe::Path`, scales + translates it
//! into the target rect, and strokes it with `stroke-width: 2` in
//! the caller's colour.
//!
//! To add a new icon: look up its SVG at <https://lucide.dev/icons/>
//! and paste every `<path d="...">` / `<line>` / `<circle>` /
//! `<rect>` concatenated into one `d` string. Circles /
//! rects get hand-rewritten as arcs / closed paths.

/// Return the combined SVG path data for `name`, or `None` if we
/// haven't bundled the glyph yet.
pub fn lookup(name: &str) -> Option<&'static str> {
    Some(match name {
        // Brand / editor
        "pen-tool" => concat!(
            "M12 19l7-7 3 3-7 7-3-3z ",
            "M18 13l-1.5-7.5L2 2l3.5 14.5L13 18l5-5z ",
            "M2 2l7.586 7.586 ",
            // circle cx=11 cy=11 r=2 → approximate with cubic arcs
            "M13 11 a2 2 0 1 1 -4 0 a2 2 0 1 1 4 0 Z"
        ),
        // Input decorations
        "mail" => concat!(
            // rect 2,4 20x16 rx=2 → rounded rect path
            "M4 4 h16 a2 2 0 0 1 2 2 v12 a2 2 0 0 1 -2 2 h-16 a2 2 0 0 1 -2 -2 v-12 a2 2 0 0 1 2 -2 Z ",
            "M22 7 l-10 5 -10 -5"
        ),
        "lock" => concat!(
            // rect 3,11 18x11 rx=2
            "M5 11 h14 a2 2 0 0 1 2 2 v7 a2 2 0 0 1 -2 2 h-14 a2 2 0 0 1 -2 -2 v-7 a2 2 0 0 1 2 -2 Z ",
            "M7 11 V7 a5 5 0 0 1 10 0 v4"
        ),
        "eye-off" => concat!(
            "M9.88 9.88 a3 3 0 1 0 4.24 4.24 ",
            "M10.73 5.08 A10.43 10.43 0 0 1 12 5 c7 0 10 7 10 7 a13.16 13.16 0 0 1 -1.67 2.68 ",
            "M6.61 6.61 A13.526 13.526 0 0 0 2 12 s3 7 10 7 a9.74 9.74 0 0 0 5.39 -1.61 ",
            "M2 2 L22 22"
        ),
        // Social buttons
        "chrome" => concat!(
            // outer circle r=10 at (12,12)
            "M22 12 a10 10 0 1 1 -20 0 a10 10 0 1 1 20 0 Z ",
            // inner circle r=4
            "M16 12 a4 4 0 1 1 -8 0 a4 4 0 1 1 8 0 Z ",
            // three spokes
            "M21.17 8 L12 8 ",
            "M3.95 6.06 L8.54 14 ",
            "M10.88 21.94 L15.46 14"
        ),
        "smartphone" => concat!(
            // rect 5,2 14x20 rx=2
            "M7 2 h10 a2 2 0 0 1 2 2 v16 a2 2 0 0 1 -2 2 h-10 a2 2 0 0 1 -2 -2 v-16 a2 2 0 0 1 2 -2 Z ",
            "M12 18 h0.01"
        ),
        _ => return None,
    })
}
