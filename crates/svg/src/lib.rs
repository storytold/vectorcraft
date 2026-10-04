//! VectorCraft SVG import and export.
//!
//! * [`export`] writes a [`Document`] as SVG 1.1 with our own writer (presentation attributes,
//!   inline styles or internal CSS classes; gradients in `<defs>` with `userSpaceOnUse`; clip groups as
//!   `<clipPath>`; embedded images as `data:` URIs; text as `<text>`/`<tspan>`).
//! * [`import`] / [`import_with_report`] parse SVG with `usvg` and convert its normalized tree into
//!   document nodes with all transforms baked into the geometry.
//!
//! ## Export approximations
//!
//! * **Stroke alignment** is not expressible in SVG 1.1. An *inside* stroke of a closed path is
//!   written as a stroke of double width clipped to the path's own shape (`<clipPath>`); an
//!   *outside* stroke as a stroke of double width masked by a `<mask>` that hides the path's
//!   interior. Both are visually exact but re-import as a group (clip group / masked stroke)
//!   rather than an aligned stroke. Open paths have no inside: their strokes are written centred,
//!   as the canvas draws them.
//! * **Arrowheads, width profiles, dashes fitted to corners and dotted dashes** are written as the
//!   filled outlines the canvas paints (the line and each arrowhead, grouped under the stroke's
//!   opacity); **brushed strokes** as their brush art. Both re-import as filled paths.
//! * **Multiple fills/strokes** (or per-fill/stroke blend modes) are written as a `<g>` holding one
//!   `<path>` per appearance item, in paint order.
//! * Live geometry effects are written as their result and raster effects (shadows, glows, blurs)
//!   as SVG filters. Freeform gradients export as linear gradients.
//!
//! ## Import approximations
//!
//! * One CSS pixel (user unit) is one point, as on export; a root `width`/`height` in absolute units
//!   (`in`, `cm`, `mm`, `pt`, `pc`) keeps its physical size (72 pt per inch), so a 210 mm SVG opens on
//!   a 210 mm artboard.
//! * Masks and filters are ignored (reported as warnings); nested clip paths use the outer clip only.
//! * usvg only keeps `<text>` when fonts are loaded; we don't load a font database (too expensive and
//!   unavailable on wasm), so `<text>` elements are read directly from the XML as live point-type
//!   [`TextObject`]s (type on a path for a `<textPath>`) and placed on top of their layer.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod export;
mod import;

pub use vectorcraft_doc::Document;
pub use vectorcraft_doc::TextObject;

/// How style properties are written (the "Styling" export option).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Styling {
    /// `fill="#ff0000"` attributes.
    #[default]
    PresentationAttributes,
    /// `style="fill:#ff0000"`.
    InlineStyle,
    /// `class="cls-1"` with a `<style>` element in `<defs>`.
    InternalCss,
}

/// SVG export options (Export As → SVG).
#[derive(Clone, Debug, PartialEq)]
pub struct ExportOptions {
    /// Artboard index to export (sets the viewBox); `None` = the bounds of all art.
    pub artboard: Option<usize>,
    pub styling: Styling,
    /// Decimal places for coordinates (1–7 in the dialog; default 3).
    pub decimals: u8,
    /// Write `id` attributes from layer and object names.
    pub object_ids: bool,
    /// No indentation or newlines, no XML declaration.
    pub minify: bool,
    /// Omit `width`/`height` so the SVG scales to its container.
    pub responsive: bool,
    /// Fonts → Convert to Outlines: text becomes paths (portable, no font needed to view it).
    pub outline_text: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            artboard: Some(0),
            styling: Styling::PresentationAttributes,
            decimals: 3,
            object_ids: true,
            minify: false,
            responsive: false,
            outline_text: false,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error("SVG parse error: {0}")]
    Parse(String),
}

/// Export a document as an SVG string.
pub fn export(doc: &Document, opts: &ExportOptions) -> String {
    export::export(doc, opts)
}

/// Import an SVG document.
pub fn import(svg: &str) -> Result<Document, SvgError> {
    import_with_report(svg).map(|(d, _)| d)
}

/// Import an SVG document, also returning warnings about unsupported or approximated features.
pub fn import_with_report(svg: &str) -> Result<(Document, Vec<String>), SvgError> {
    import::import(svg)
}

/// Standard base64 (RFC 4648, with padding).
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Escape text for XML content and attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

/// Format a number with at most `decimals` places, trimming trailing zeros.
pub fn fmt_num(v: f64, decimals: u8) -> String {
    let v = if v.is_finite() { v } else { 0.0 };
    let s = format!("{:.*}", decimals as usize, v);
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s };
    if s == "-0" || s.is_empty() { "0".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn numbers() {
        assert_eq!(fmt_num(1.0, 3), "1");
        assert_eq!(fmt_num(1.23456, 3), "1.235");
        assert_eq!(fmt_num(-0.0001, 3), "0");
        assert_eq!(fmt_num(10.5, 1), "10.5");
        assert_eq!(fmt_num(100.0, 0), "100");
    }

    #[test]
    fn escaping() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
