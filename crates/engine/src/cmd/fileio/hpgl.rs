//! HP-GL for every path that writes plot files (`vectorcraft_cad::hpgl`): `document.export {format:
//! hpgl}` and Export As (one plot of an artboard, or one per artboard with Use Artboards), with the
//! options parsed from the params.

use serde_json::Value;
use vectorcraft_cad::hpgl::write::{DEFAULT_TOLERANCE, MAX_PENS, TOLERANCE_RANGE};
use vectorcraft_cad::hpgl::{HpglOptions, Origin, Version};
use vectorcraft_doc::Document;

use super::super::{Result, bad, number_or, switch_or, text_param};
use super::{ArtboardPick, Encoded, FormatOption};

const C: &str = "document.export";

/// The options `document.formats` lists for HP-GL.
pub(super) const OPTIONS: &[FormatOption] = &[
    super::ARTBOARD,
    super::ARTBOARDS,
    super::RANGE,
    super::USE_ARTBOARDS,
    FormatOption {
        name: "hpglVersion",
        ty: "string",
        default: "\"hpgl\"",
        description: "hpgl (IN, SP, PU and PD only; a coordinate outside -32,768 to 32,767 gets a warning) or hpgl2 (also the pens' colors, NP and PC, and each pen's paths as one encoded polyline, PE)",
    },
    FormatOption {
        name: "curveTolerance",
        ty: "number",
        default: "0.1",
        description: "how far the straight moves a curve becomes may lie from it, in points (0.01 to 10)",
    },
    FormatOption {
        name: "pens",
        ty: "array",
        default: "null",
        description: "the pen table, pen 1's color first: up to 255 \"#rrggbb\" colors, each path drawn with the pen of the nearest color; null: a pen for each color in the order the art first draws in it, numbered alike in every file of one export",
    },
    FormatOption {
        name: "origin",
        ty: "string",
        default: "\"bottomLeft\"",
        description: "bottomLeft or center: where (0, 0) lies on the artboard (or the art's bounds)",
    },
    FormatOption {
        name: "sortPaths",
        ty: "boolean",
        default: "false",
        description: "draw each pen's next path from the end nearest where the pen stopped, instead of in paint order",
    },
    FormatOption {
        name: "insideFirst",
        ty: "boolean",
        default: "false",
        description: "draw a path only after the pen's paths that lie inside it (a letter's counter before the letter, a hole before the part), the paths otherwise in paint order; with sortPaths, the nearest of the paths free to draw comes next",
    },
];

/// The HP-GL options in `p` (the region is set per plot).
fn parse(p: &Value) -> Result<HpglOptions> {
    let mut o = HpglOptions::default();
    if let Some(s) = text_param(p, "hpglVersion", C)? {
        let key = "hpglVersion";
        o.version = Version::from_id(s).ok_or_else(|| bad(C, format!("`{key}` must be one of {}", Version::ALL.map(Version::id).join(", "))))?;
    }
    let x = number_or(p, "curveTolerance", DEFAULT_TOLERANCE, C)?;
    if !TOLERANCE_RANGE.contains(&x) {
        let (key, min, max) = ("curveTolerance", TOLERANCE_RANGE.start(), TOLERANCE_RANGE.end());
        return Err(bad(C, format!("`{key}` must be between {min} and {max} (got {x})")));
    }
    o.tolerance = x;
    match p.get("pens") {
        None | Some(Value::Null) => {}
        Some(v) => {
            let colors: Option<Vec<[u8; 3]>> = v.as_array().filter(|a| (1..=MAX_PENS).contains(&a.len())).and_then(|a| a.iter().map(hex).collect());
            o.pens = colors.ok_or_else(|| bad(C, "`pens` must be an array of 1 to 255 #rrggbb colors"))?;
        }
    }
    if let Some(s) = text_param(p, "origin", C)? {
        let key = "origin";
        o.origin = Origin::from_id(s).ok_or_else(|| bad(C, format!("`{key}` must be one of {}", Origin::ALL.map(Origin::id).join(", "))))?;
    }
    o.sort = switch_or(p, "sortPaths", false, C)?;
    o.inside_first = switch_or(p, "insideFirst", false, C)?;
    Ok(o)
}

/// A `#rrggbb` color.
fn hex(v: &Value) -> Option<[u8; 3]> {
    let s = v.as_str()?.strip_prefix('#').filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let byte = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    Some([byte(0)?, byte(2)?, byte(4)?])
}

/// Encode `doc` as HP-GL: a plot of the chosen artboard (default the first), or with
/// `use_artboards` one plot per chosen artboard (default all), with one pen numbering for them all.
pub(super) fn encode(doc: &Document, p: &Value, use_artboards: Option<bool>) -> Result<Encoded> {
    let opts = parse(p)?;
    let f = super::format("hpgl").ok_or_else(|| bad(C, "no HP-GL format"))?;
    let pick: ArtboardPick = super::encode::options(f, p)?;
    let n = doc.artboards.len();
    let boards = match use_artboards {
        Some(true) => pick.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect()),
        _ => vec![pick.one(n).map_err(|e| bad(C, e))?],
    };
    let regions = boards
        .iter()
        .map(|&b| doc.artboards.get(b).map(|a| a.rect).ok_or_else(|| bad(C, format!("no artboard {}", b + 1))))
        .collect::<Result<Vec<_>>>()?;
    let plots = vectorcraft_cad::hpgl::export_regions(doc, &opts, &regions).map_err(|e| bad(C, e))?;
    let mut enc = Encoded::default();
    for (b, out) in boards.into_iter().zip(plots) {
        enc.files.push((Some(b), out.bytes));
        for w in out.warnings {
            if !enc.warnings.contains(&w) {
                enc.warnings.push(w);
            }
        }
    }
    if enc.files.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    let regions: Vec<vectorcraft_geom::Rect> = enc.files.iter().filter_map(|(b, _)| b.and_then(|b| doc.artboards.get(b))).map(|a| a.rect).collect();
    enc.warnings.extend(crate::cmd::fonts::export_overflow_warning(doc, &doc.layers, Some(&regions)));
    Ok(enc)
}
