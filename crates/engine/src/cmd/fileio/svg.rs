//! SVG Options: the SVG settings of `document.export`, `serialize` and `save`, read from the
//! params' top level or from an `svg` object (which wins), and the files an SVG export writes
//! (one per artboard, plus the images it links to).

use serde::Deserialize;
use serde_json::{Map, Value};
use vectorcraft_doc::Document;
use vectorcraft_svg::ExportOptions;

use super::encode::{ARTBOARD_PARAMS, ArtboardPick, Encoded};
use super::{ARTBOARD, ARTBOARDS, FormatOption, RANGE};

/// The options `document.formats` lists for SVG (the defaults are [`ExportOptions::default`]'s).
pub const OPTIONS: &[FormatOption] = &[
    ARTBOARD,
    ARTBOARDS,
    RANGE,
    FormatOption {
        name: "useArtboards",
        ty: "boolean",
        default: "true",
        description: "false: one SVG of the bounds of all art instead of artboards (several artboards write one file each; a named artboard's file holds only the art over it)",
    },
    FormatOption {
        name: "styling",
        ty: "string",
        default: "\"presentation\"",
        description: "presentation (attributes) | style (style attributes) | entities (style attributes as DOCTYPE entities) | css (internal <style> classes)",
    },
    FormatOption {
        name: "outlineText",
        ty: "boolean",
        default: "false",
        description: "Fonts: text as glyph outlines (viewable without the fonts) instead of <text>; default: Document Setup → Type → Export (Preserve Text Appearance: true)",
    },
    FormatOption {
        name: "images",
        ty: "string",
        default: "\"embed\"",
        description: "embed (data: URIs) | link (linked images keep their file; embedded ones are written next to the SVG, or returned as `linked`)",
    },
    FormatOption {
        name: "objectIds",
        ty: "string",
        default: "\"layerNames\"",
        description: "layerNames (readable ids from names) | minimal (only referenced ids) | unique (content-hashed prefix on every id and class)",
    },
    FormatOption { name: "decimals", ty: "integer", default: "3", description: "coordinate precision, 1–7 decimal places" },
    FormatOption { name: "minify", ty: "boolean", default: "false", description: "no XML declaration, indentation or line breaks" },
    FormatOption { name: "responsive", ty: "boolean", default: "false", description: "no width/height: the SVG scales to its container" },
    FormatOption {
        name: "preserveEditing",
        ty: "boolean",
        default: "false",
        description: "embed the native document in <metadata> so VectorCraft reopens the SVG with nothing lost",
    },
    FormatOption {
        name: "metadata",
        ty: "boolean",
        default: "false",
        description: "write <metadata> with the title, format and File Info (Dublin Core)",
    },
    FormatOption {
        name: "fewerTspans",
        ty: "boolean",
        default: "false",
        description: "type: one positioned <tspan> per line instead of one per style run, tab and justified word (smaller; viewers space the line themselves)",
    },
    FormatOption {
        name: "hiddenLayers",
        ty: "boolean",
        default: "false",
        description: "keep hidden layers and objects, not displayed (display=\"none\"); document.save sets it unless given",
    },
    FormatOption {
        name: "encoding",
        ty: "string",
        default: "\"utf8\"",
        description: "utf8 | utf16 (big-endian, with a byte order mark) | latin1 (ISO 8859-1; other characters as &#x…; references)",
    },
    FormatOption {
        name: "profile",
        ty: "string",
        default: "\"svg11\"",
        description: "svg11 (SVG 1.1) | tiny12 (SVG Tiny 1.2, simplified: presentation attributes only; no filters, masks, symbols, blend modes or embedded fonts)",
    },
    FormatOption {
        name: "embedFonts",
        ty: "boolean",
        default: "false",
        description: "embed the fonts type uses as @font-face, subset to the characters used (whole when the font's licence forbids subsetting, left out with a warning when it forbids embedding)",
    },
    FormatOption {
        name: "svg",
        ty: "object",
        default: "null",
        description: "these options as one object, e.g. {styling: \"css\", decimals: 2}; it wins over top-level keys and rejects unknown ones",
    },
];

/// Picks the artboards of an SVG export (beside [`ArtboardPick`]).
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Boards {
    #[serde(flatten)]
    pick: ArtboardPick,
    use_artboards: Option<bool>,
}

/// The SVG options in `p`: its top-level option keys overlaid with its `svg` object. Empty when
/// `p` names none.
pub fn options_map(p: &Value) -> Result<Map<String, Value>, String> {
    let mut m = Map::new();
    let Some(o) = p.as_object() else { return Ok(m) };
    for opt in OPTIONS.iter().filter(|opt| opt.name != "svg") {
        if let Some(v) = o.get(opt.name).filter(|v| !v.is_null()) {
            m.insert(opt.name.into(), v.clone());
        }
    }
    match o.get("svg") {
        None | Some(Value::Null) => {}
        Some(Value::Object(svg)) => m.extend(svg.clone()),
        Some(_) => return Err("`svg` must be an object of SVG options (see document.formats)".into()),
    }
    Ok(m)
}

/// The writer options and the artboards (`None`: the art bounds) an SVG export of `doc` covers,
/// and whether `p` named those artboards (else the first stands for the whole document, as Save
/// writes it).
fn plan(doc: &Document, p: &Value) -> Result<(ExportOptions, Vec<Option<usize>>, bool), String> {
    let mut m = options_map(p)?;
    // Fonts: unless chosen, Document Setup → Type → Export decides (appearance = outlines).
    if !m.contains_key("outlineText") && doc.setup.export_text == vectorcraft_doc::ExportText::Appearance {
        m.insert("outlineText".into(), Value::Bool(true));
    }
    let picks: Map<String, Value> = ARTBOARD_PARAMS.iter().chain(["useArtboards"].iter()).filter_map(|k| m.remove_entry(*k)).collect();
    let boards = Boards::deserialize(Value::Object(picks)).map_err(|e| format!("SVG options: {e}"))?;
    let opts = ExportOptions::deserialize(Value::Object(m)).map_err(|e| format!("SVG options: {e}"))?;
    opts.check().map_err(|e| format!("SVG options: {e}"))?;
    let n = doc.artboards.len();
    if n == 0 || boards.use_artboards == Some(false) {
        return Ok((opts, vec![None], false));
    }
    let named = boards.pick.resolve(n)?;
    let chosen = named.is_some();
    Ok((opts, named.unwrap_or_else(|| vec![0]).into_iter().map(Some).collect(), chosen))
}

/// Encode `doc` as SVG (or gzipped, SVGZ): one file per chosen artboard, with the images they
/// link to and the writer's warnings.
pub(super) fn encode(doc: &Document, p: &Value, compressed: bool) -> Result<Encoded, String> {
    let (opts, boards, chosen) = plan(doc, p)?;
    // Preserve editing embeds the native document (once, shared by every file).
    let native = opts.preserve_editing.then(|| vectorcraft_format::save(doc, false));
    // Draw linked files as vectors when available, or their previews with a warning. Keep
    // that preparation out of the native attachment so missing links remain relinkable.
    let (full, warnings) = crate::cmd::place::document::full_documents(doc);
    let doc = &*full;
    // SVG has no filters for the Photoshop-style effects: their objects go in as images.
    let flat = crate::cmd::rasterfx::flatten_pixel_effects(doc);
    let doc = flat.as_ref().unwrap_or(doc);
    let mut enc = Encoded { warnings, ..Encoded::default() };
    for artboard in boards {
        // A chosen artboard's file holds the art over it (#550).
        let over = artboard.filter(|_| chosen).and_then(|b| doc.artboards.get(b)).map(|a| super::export::art_over(doc, a.rect));
        let mut out = vectorcraft_svg::export_full(over.as_ref().unwrap_or(doc), &ExportOptions { artboard, ..opts.clone() }, native.as_deref());
        let bytes = out.take_bytes();
        for l in out.linked {
            if !enc.linked.iter().any(|e| e.name == l.name) {
                enc.linked.push(l);
            }
        }
        for w in out.warnings {
            if !enc.warnings.contains(&w) {
                enc.warnings.push(w);
            }
        }
        enc.files.push((artboard, if compressed { vectorcraft_svg::compress_bytes(&bytes) } else { bytes }));
    }
    // Live type names its fonts; outlines and embedded fonts are the fallback font's.
    if (opts.outline_text || opts.embed_fonts)
        && let Some(w) = crate::cmd::fonts::substitution_warning(doc)
    {
        enc.warnings.push(w);
    }
    Ok(enc)
}
