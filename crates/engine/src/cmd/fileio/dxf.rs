//! DXF export for every DXF path (`document.export {format: dxf}`, `document.exportDxf`, Export As
//! → DXF Options): the options parsed from the params, one drawing per artboard with Use
//! Artboards, and the formats that can't be written (DWG, PICT) with what to use instead.

use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use vectorcraft_cad::{ColorDepth, DxfOptions, DxfVersion, Preserve, RasterFormat};
use vectorcraft_doc::{Document, Unit};
use vectorcraft_geom::Rect;

use super::super::{CommandSpec, Result, Session, bad, cmd, has_doc};
use super::{ArtboardPick, Encoded, FormatOption};

const C: &str = "document.export";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "document.exportDxf",
        "Export DXF",
        [],
        None,
        "{path?, version?: R12|R13|R14|2000|2004|2007|2010|2013|2018 (default), unit?: mm (default; any unit name: pt, in, cm…), scale?: 1 (drawing units per unit), scaleLineweights?: false, colors?: 8|16|256|true (default; true colour needs 2004+, else 256), rasterFormat?: png (default)|jpeg (placed images, written next to the file), preserve?: appearance (default: type as outlines; strokes a CAD line can't draw as filled outlines; brush art)|editability (type as text; every stroke a line with its lineweight and dashes), alterPaths?: false (every stroke as its filled outline), outlineText?: false, selectedOnly?: false (the selected objects, in their layers), artboard?: 0 (the origin is its bottom-left corner), useArtboards?: true (one drawing per chosen artboard, artboards?/range?, holding the art over it: {stem}-{artboard}.dxf) | false (the origin at the art's bounds)} → {path, bytes, warnings, files?, linked?: [image files]}; no path → {dataBase64, …, linked?: [{name, dataBase64}]}. Layers become DXF layers (hidden: off; locked; non-printing: not plotted); paths become polylines and splines, fills their outlines (which laser and cutter software reads) and solid hatches. Same as document.export {format: dxf}",
        has_doc,
        export_dxf
    )]
}

/// The DXF options `document.formats` lists.
pub(super) const OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    super::USE_ARTBOARDS,
    FormatOption { name: "version", ty: "string", default: "\"2018\"", description: "R12, R13, R14, 2000, 2004, 2007, 2010, 2013 or 2018" },
    FormatOption {
        name: "unit",
        ty: "string",
        default: "\"mm\"",
        description: "the art unit that `scale` drawing units stand for (pt, in, mm, cm, m, ft…)",
    },
    FormatOption { name: "scale", ty: "number", default: "1", description: "drawing units per unit (1 mm = 1 unit)" },
    FormatOption { name: "scaleLineweights", ty: "boolean", default: "false", description: "lineweights grow with the scale" },
    FormatOption {
        name: "colors",
        ty: "string",
        default: "\"true\"",
        description: "8, 16 or 256 indexed colours, or true colour (DXF 2004 and later)",
    },
    FormatOption {
        name: "rasterFormat",
        ty: "string",
        default: "\"png\"",
        description: "png or jpeg: the files placed images are written to, next to the drawing",
    },
    FormatOption {
        name: "preserve",
        ty: "string",
        default: "\"appearance\"",
        description: "appearance (type as outlines, strokes a CAD line can't draw as filled outlines) or editability (type as text, strokes as lines)",
    },
    FormatOption {
        name: "alterPaths",
        ty: "boolean",
        default: "false",
        description: "Alter Paths for Appearance: every stroke as its filled outline",
    },
    FormatOption { name: "outlineText", ty: "boolean", default: "false", description: "type as glyph outlines (always with preserve: appearance)" },
    FormatOption { name: "selectedOnly", ty: "boolean", default: "false", description: "only the selected objects, in their layers" },
];

/// A format that can be neither opened nor written, and what to use instead.
#[derive(Clone, Copy, Debug)]
pub struct Unsupported {
    pub id: &'static str,
    pub label: &'static str,
    /// Lower-case extensions without the dot.
    pub extensions: &'static [&'static str],
    pub hint: &'static str,
}

/// Formats people ask for that VectorCraft doesn't read or write. Append-only.
pub const UNSUPPORTED: &[Unsupported] = &[
    Unsupported {
        id: "dwg",
        label: "DWG",
        extensions: &["dwg"],
        hint: "DWG is not available (no openly licensed writer exists): export DXF instead, which CAD apps open",
    },
    Unsupported {
        id: "pict",
        label: "PICT",
        extensions: &["pict", "pct", "pic"],
        hint: "PICT is a retired format and is not supported: convert the file to PDF or PNG first",
    },
    Unsupported {
        id: "pmd",
        label: "PageMaker",
        extensions: &["pmd"],
        hint: "PageMaker (.pmd) is not supported: open the layout in PageMaker and export PDF or EPS, or place a PDF export here",
    },
];

/// The unsupported format an id or extension (any case, leading dot allowed) names.
pub fn unsupported(id_or_ext: &str) -> Option<&'static Unsupported> {
    let k = id_or_ext.trim_start_matches('.').to_ascii_lowercase();
    UNSUPPORTED.iter().find(|u| u.id == k || u.extensions.contains(&k.as_str()))
}

impl Unsupported {
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "label": self.label, "extensions": self.extensions, "hint": self.hint })
    }
}

/// The DXF params, as they come (each checked when read).
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Params {
    #[serde(flatten)]
    boards: ArtboardPick,
    version: Option<Value>,
    unit: Option<String>,
    scale: Option<f64>,
    scale_lineweights: Option<bool>,
    colors: Option<Value>,
    raster_format: Option<String>,
    preserve: Option<String>,
    alter_paths: Option<bool>,
    outline_text: Option<bool>,
}

/// A string or number param as text (`2018` and `"2018"` alike).
pub(crate) fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// `field` read by `read`, or an error listing what it may be.
fn choice<T>(field: &str, v: Option<String>, read: impl Fn(&str) -> Option<T>, default: T, allowed: &str) -> Result<T> {
    match v {
        None => Ok(default),
        Some(s) => read(&s).ok_or_else(|| bad(C, format!("DXF {field} `{s}`: {allowed}"))),
    }
}

/// The DXF options in `p` (the region and crop are set per drawing) and the artboard choice.
fn parse(p: &Value) -> Result<(DxfOptions, ArtboardPick)> {
    let q: Params = if p.is_object() { Params::deserialize(p).map_err(|e| bad(C, format!("DXF options: {e}")))? } else { Params::default() };
    let d = DxfOptions::default();
    let scale = q.scale.unwrap_or(d.scale);
    if !(scale.is_finite() && vectorcraft_cad::SCALE_RANGE.contains(&scale)) {
        return Err(bad(C, format!("DXF scale must be a positive number up to {}, not {scale}", vectorcraft_cad::SCALE_RANGE.end())));
    }
    let opts = DxfOptions {
        version: choice(
            "version",
            q.version.as_ref().map(text),
            DxfVersion::from_id,
            d.version,
            "R12, R13, R14, 2000, 2004, 2007, 2010, 2013 or 2018",
        )?,
        unit: choice("unit", q.unit, Unit::named, d.unit, "a unit such as mm, cm, in or pt")?,
        scale,
        scale_lineweights: q.scale_lineweights.unwrap_or(d.scale_lineweights),
        colors: choice("colors", q.colors.as_ref().map(text), ColorDepth::from_id, d.colors, "8, 16, 256 or true")?,
        raster: choice("rasterFormat", q.raster_format, RasterFormat::from_id, d.raster, "png or jpeg")?,
        preserve: choice("preserve", q.preserve, Preserve::from_id, d.preserve, "appearance or editability")?,
        alter_paths: q.alter_paths.unwrap_or(d.alter_paths),
        outline_text: q.outline_text.unwrap_or(d.outline_text),
        ..d
    };
    Ok((opts, q.boards))
}

/// Check the DXF options in `p` without exporting (dialogs keep themselves open on an error).
pub fn check(p: &Value) -> std::result::Result<(), String> {
    parse(p).map(|_| ()).map_err(|e| e.to_string())
}

/// Encode `doc` as DXF: one drawing whose origin is the chosen artboard's bottom-left corner
/// (default the first), or with `use_artboards` one drawing per chosen artboard (default all)
/// holding the art over it.
pub(super) fn encode(doc: &Document, p: &Value, use_artboards: Option<bool>) -> Result<Encoded> {
    let (opts, pick) = parse(p)?;
    let n = doc.artboards.len();
    let rect = |b: usize| doc.artboards.get(b).map(|a| a.rect).ok_or_else(|| bad(C, format!("no artboard {}", b + 1)));
    let drawings: Vec<(Option<usize>, Rect, bool)> = match use_artboards {
        Some(true) => {
            let boards = pick.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect());
            boards.into_iter().map(|b| rect(b).map(|r| (Some(b), r, true))).collect::<Result<_>>()?
        }
        _ if n == 0 => vec![(None, doc.art_bounds().unwrap_or(Rect::ZERO), false)],
        _ => {
            let b = match pick.resolve(n).map_err(|e| bad(C, e))?.as_deref() {
                None => 0,
                Some([b]) => *b,
                Some(_) => return Err(bad(C, "a DXF drawing has one origin: pass useArtboards: true for one drawing per artboard")),
            };
            vec![(Some(b), rect(b)?, false)]
        }
    };
    let mut enc = Encoded::default();
    for (board, region, crop) in drawings {
        let out = vectorcraft_cad::export(doc, &DxfOptions { region, crop, ..opts.clone() }).map_err(|e| bad(C, e))?;
        enc.files.push((board, out.bytes));
        for w in out.warnings {
            if !enc.warnings.contains(&w) {
                enc.warnings.push(w);
            }
        }
        for img in out.images {
            if !enc.linked.iter().any(|l| l.name == img.name) {
                enc.linked.push(vectorcraft_svg::LinkedImage { name: img.name, bytes: Arc::new(img.bytes) });
            }
        }
    }
    if enc.files.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    Ok(enc)
}

fn export_dxf(s: &mut Session, p: &Value) -> Result<Value> {
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    q["format"] = json!("dxf");
    super::export::export(s, &q)
}
