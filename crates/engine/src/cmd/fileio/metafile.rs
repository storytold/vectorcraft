//! EMF and WMF for every path that writes or reads Windows metafiles (`vectorcraft_metafile`):
//! `document.export {format: emf|wmf}` and Export As (one picture of an artboard, or one per
//! artboard with Use Artboards; raster effects as images, rendered at the document's raster
//! effects resolution), `document.open` and File → Place (one artboard, the picture's frame).

use serde_json::Value;
use vectorcraft_doc::Document;
use vectorcraft_metafile::Kind;

use super::super::{Result, bad};
use super::{ArtboardPick, Encoded, FormatOption};

const C: &str = "document.export";

/// The options `document.formats` lists for EMF and WMF.
pub(super) const OPTIONS: &[FormatOption] = &[super::ARTBOARD, super::ARTBOARDS, super::RANGE, super::USE_ARTBOARDS];

/// Encode `doc` as `kind`: the chosen artboard (default the first), or with `use_artboards` one
/// picture per chosen artboard (default all).
pub(super) fn encode(doc: &Document, p: &Value, use_artboards: Option<bool>, kind: Kind) -> Result<Encoded> {
    let f = super::format(kind.id()).ok_or_else(|| bad(C, "no metafile format"))?;
    let pick: ArtboardPick = super::encode::options(f, p)?;
    let n = doc.artboards.len();
    let boards = match use_artboards {
        Some(true) => pick.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect()),
        _ => vec![pick.one(n).map_err(|e| bad(C, e))?],
    };
    // Raster effects (shadows, glows, blurs) as images, as PDF export does.
    let flat = crate::flatten_raster_effects(doc);
    let src = flat.as_ref().unwrap_or(doc);
    let mut enc = Encoded::default();
    for b in boards {
        let region = doc.artboards.get(b).ok_or_else(|| bad(C, format!("no artboard {}", b + 1)))?.rect;
        let out = vectorcraft_metafile::export(src, region, kind).map_err(|e| bad(C, e))?;
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

/// A metafile's document and the import's warnings.
pub(super) fn import(bytes: &[u8]) -> Result<(Document, Vec<String>)> {
    let i = vectorcraft_metafile::import(bytes).map_err(super::err)?;
    Ok((i.document, i.warnings))
}
