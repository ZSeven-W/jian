//! Per-mount font registration shared by CanvasKit paint and measurement.

use crate::canvaskit::CkRuntime;

pub struct FontRegistry {
    runtime: CkRuntime,
}

impl FontRegistry {
    pub(crate) fn new(runtime: CkRuntime) -> Self {
        Self { runtime }
    }

    /// Register bytes under the document-facing alias. The font's real family
    /// is parsed because the vendored CanvasKit exposes no family-name API.
    pub fn register(&self, alias: &str, bytes: &[u8]) -> Result<String, String> {
        let family = parse_family(bytes).ok_or("font has no decodable family name")?;
        let alias = if alias.trim().is_empty() {
            &family
        } else {
            alias.trim()
        };
        self.runtime
            .register_font(alias, &family, bytes)
            .then_some(family)
            .ok_or_else(|| "CanvasKit rejected font bytes".to_owned())
    }

    /// True only when every Unicode scalar resolves to a nonzero glyph in
    /// the requested family or the registered fallback chain.
    pub fn covers_text(&self, alias: &str, text: &str) -> bool {
        self.runtime.covers_text(alias, text)
    }
}

fn parse_family(bytes: &[u8]) -> Option<String> {
    let face = ttf_parser::Face::parse(bytes, 0).ok()?;
    let mut legacy = None;
    let mut typographic = None;
    for name in face.names() {
        if name.name_id != 1 && name.name_id != 16 {
            continue;
        }
        let Some(value) = name.to_string() else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        if name.name_id == 16 {
            typographic.get_or_insert_with(|| value.to_owned());
        } else {
            legacy.get_or_insert_with(|| value.to_owned());
        }
    }
    typographic.or(legacy)
}

#[cfg(test)]
mod tests {
    use super::parse_family;

    #[test]
    fn parses_real_family_and_rejects_garbage() {
        let bytes = include_bytes!("../assets/fonts/Roboto-Regular.ttf");
        assert_eq!(parse_family(bytes).as_deref(), Some("Roboto"));
        assert_eq!(parse_family(b"not a font"), None);
    }
}
