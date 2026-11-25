//! File → Print › Advanced, what needs the renderer and the flattener (the PDF writer does the
//! rest): Print as Bitmap renders each printed region as one image (at `advanced.bitmapResolution`,
//! or the document's raster effects resolution), and a flattener preset
//! flattens transparency, both on the document as it prints (only the layers that print,
//! overprints discarded or simulated in composite output) before it is laid out on paper.

use std::sync::Arc;

use vectorcraft_doc::marks::outset;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, LayerColor, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_pdf::{OutputMode, PrintOverprints, PrintSettings};
use vectorcraft_render::{AntiAlias, RenderOptions};

use super::fileio::pdf::pdf_error;
use super::flatten::{FlattenOptions, FlattenerPreset, flatten_document};
use super::menucmds::{MAX_PIXELS, unique_key};
use super::{EngineError, Result, bad};

/// The flattener options of `set`'s preset (`None`: transparency prints live); an error naming
/// `cmd` for a preset that is neither built in nor in `saved`.
pub(crate) fn flattener(cmd: &str, set: &PrintSettings, saved: &[FlattenerPreset]) -> Result<Option<FlattenOptions>> {
    let name = set.advanced.flattener_preset.trim();
    if name.is_empty() {
        return Ok(None);
    }
    let o = FlattenOptions::find_preset(name, saved)
        .ok_or_else(|| bad(cmd, format!("flattenerPreset `{name}`: high, medium, low or a saved one (see flattener.presets.list)")))?;
    Ok(Some(o))
}

/// `doc` as Print as Bitmap and the flattener preset of `set` print it (from `saved` presets), or
/// `None` when neither applies. Print as Bitmap is for composite output.
/// `rich_black`: Appearance of Black › Printing / Exporting › Output All Blacks as Rich Black.
#[cfg(test)]
pub(crate) fn prepare(cmd: &str, doc: &Document, set: &PrintSettings, saved: &[FlattenerPreset], rich_black: bool) -> Result<Option<Document>> {
    prepare_noting(cmd, doc, set, saved, rich_black, &mut vec![])
}

/// [`prepare`], with the note on an image Print as Bitmap prints below `advanced.bitmapResolution`
/// added to `notes`.
pub(crate) fn prepare_noting(
    cmd: &str,
    doc: &Document,
    set: &PrintSettings,
    saved: &[FlattenerPreset],
    rich_black: bool,
    notes: &mut Vec<String>,
) -> Result<Option<Document>> {
    let composite = set.output.mode == OutputMode::Composite;
    let bitmap = set.advanced.print_as_bitmap && composite;
    let flattener = flattener(cmd, set, saved)?;
    if !bitmap && flattener.is_none() {
        return Ok(None);
    }
    let printed = vectorcraft_pdf::printed_document(doc, set);
    if bitmap {
        let (out, lowest) = bitmap_document(cmd, &printed, set, rich_black)?;
        notes.extend(capped_note(set, lowest));
        return Ok(Some(out));
    }
    let Some(mut o) = flattener else { return Ok(None) };
    // Composite overprints that were discarded or simulated aren't preserved either.
    o.preserve_overprints &= !composite || set.advanced.overprints == PrintOverprints::Preserve;
    Ok(Some(flatten_document(&printed, &o)?.unwrap_or_else(|| printed.into_owned())))
}

/// Leave guides out of `nodes`: guide paths (the renderer draws them; they never print) and ruler
/// guides.
fn drop_guides(nodes: &mut Vec<Arc<Node>>) {
    fn has_guides(n: &Node) -> bool {
        n.is_guide() || n.children().is_some_and(|c| c.iter().any(|c| has_guides(c)))
    }
    nodes.retain(|n| !n.is_guide());
    for n in nodes.iter_mut().filter(|n| has_guides(n)) {
        if let Some(children) = Arc::make_mut(n).children_mut() {
            drop_guides(children);
        }
    }
}

/// The resolution Print as Bitmap renders `doc` at with `set` (ppi): `advanced.bitmapResolution`,
/// or the document's raster effects resolution when that is 0.
fn bitmap_ppi(doc: &Document, set: &PrintSettings) -> f64 {
    let asked = set.advanced.bitmap_resolution;
    if asked > 0.0 { asked } else { doc.raster_effects_ppi }
}

/// The pixels per point Print as Bitmap renders `area` at for `ppi`: at most MAX_PIXELS, and no
/// side longer than the renderer draws (rounded up to whole pixels when it renders).
fn bitmap_scale(area: Rect, ppi: f64) -> f64 {
    let side = area.width().max(area.height()).max(1.0);
    let longest = f64::from(vectorcraft_render::MAX_RASTER_SIDE) - 2.0;
    (ppi / 72.0).min((MAX_PIXELS / (area.width() * area.height()).max(1.0)).sqrt()).min(longest / side)
}

/// The note on Print as Bitmap printing below the `advanced.bitmapResolution` of `set`, when the
/// `lowest` resolution (ppi) an image prints at is lower. None when the job leaves the resolution
/// to the document.
fn capped_note(set: &PrintSettings, lowest: Option<f64>) -> Option<String> {
    let asked = set.advanced.bitmap_resolution;
    let lowest = lowest.filter(|l| asked > 0.0 && l.is_finite() && *l < asked - 1e-6)?;
    Some(format!(
        "Print as Bitmap prints at {} ppi, not the {asked} ppi bitmapResolution asks for: a page prints as at most 64 million pixels and 32,768 pixels a side",
        lowest.floor()
    ))
}

/// What [`prepare`] notes on Print as Bitmap for `doc` printed with `set`, without rendering it,
/// for the preview: a page with art that prints whose image the pixel limits keep below
/// `advanced.bitmapResolution`. The art is judged by its bounds, so a page holding only art that
/// draws nothing (fully transparent) is counted, while the print itself leaves its image out.
pub(crate) fn bitmap_notes(cmd: &str, doc: &Document, set: &PrintSettings) -> Result<Vec<String>> {
    let asked = set.advanced.bitmap_resolution;
    if !(set.advanced.print_as_bitmap && set.output.mode == OutputMode::Composite && asked > 0.0) {
        return Ok(vec![]);
    }
    let printed = vectorcraft_pdf::printed_document(doc, set);
    let regions = vectorcraft_pdf::print_regions(&printed, set).map_err(|e| pdf_error(cmd, e))?;
    let bleed = vectorcraft_pdf::page_bleed(&set.bleed, &printed);
    let lowest = regions
        .iter()
        .map(|(_, r)| outset(*r, bleed))
        .filter(|area| vectorcraft_pdf::prints_art_in(&printed, *area))
        .map(|area| bitmap_scale(area, asked) * 72.0)
        .reduce(f64::min);
    Ok(capped_note(set, lowest).into_iter().collect())
}

/// `doc` (as it prints) with its art replaced by one image of each printed region (an artboard
/// with its bleed, or all the art), at [`bitmap_ppi`] within [`bitmap_scale`]'s limits; regions
/// where nothing is drawn get none. Also the lowest resolution an image prints at (ppi), `None`
/// without images.
fn bitmap_document(cmd: &str, doc: &Document, set: &PrintSettings, rich_black: bool) -> Result<(Document, Option<f64>)> {
    let regions = vectorcraft_pdf::print_regions(doc, set).map_err(|e| pdf_error(cmd, e))?;
    let bleed = vectorcraft_pdf::page_bleed(&set.bleed, doc);
    let ppi = bitmap_ppi(doc, set);
    let mut art = doc.clone();
    drop_guides(&mut art.layers);
    let anti_alias = if doc.raster_effects.anti_alias { AntiAlias::Art } else { AntiAlias::None };
    let opts = RenderOptions { skip_templates: true, anti_alias, rich_black, ..Default::default() };
    let mut out = doc.clone();
    let mut renderer = vectorcraft_render::Renderer::new();
    let mut images = vec![];
    let mut lowest: Option<f64> = None;
    for (_, r) in regions {
        let area = outset(r, bleed);
        let scale = bitmap_scale(area, ppi);
        if !(scale.is_finite() && scale > 0.0) {
            continue;
        }
        // Whole pixels.
        let region = Rect::new(
            area.x0,
            area.y0,
            area.x0 + (area.width() * scale).ceil().max(1.0) / scale,
            area.y0 + (area.height() * scale).ceil().max(1.0) / scale,
        );
        let img = renderer.render_region_with(&art, region, scale, &opts);
        if img.pixels.as_chunks::<4>().0.iter().all(|px| px[3] == 0) {
            continue;
        }
        let key = unique_key(&out, "print-bitmap");
        out.images.insert(key.clone(), ImageBlob::new("image/png", img.to_png().map_err(EngineError::Other)?));
        let xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
        let image = ImageObject { key, width: img.width, height: img.height, xf, link: None, placement: Default::default(), pixels: None };
        images.push(Arc::new(Node::new(out.alloc_id(), NodeKind::Image(Box::new(image)))));
        lowest = Some(lowest.map_or(scale * 72.0, |l| l.min(scale * 72.0)));
    }
    let mut layer = Node::layer(out.alloc_id(), "Bitmap", LayerColor::Preset(0));
    if let Some(children) = layer.children_mut() {
        *children = images;
    }
    out.layers = vec![Arc::new(layer)];
    Ok((out, lowest))
}
