//! File → Print › Advanced, what needs the renderer and the flattener (the PDF writer does the
//! rest): Print as Bitmap renders each printed region as one image, and a flattener preset
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
use super::*;

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
pub(crate) fn prepare(cmd: &str, doc: &Document, set: &PrintSettings, saved: &[FlattenerPreset], rich_black: bool) -> Result<Option<Document>> {
    let composite = set.output.mode == OutputMode::Composite;
    let bitmap = set.advanced.print_as_bitmap && composite;
    let flattener = flattener(cmd, set, saved)?;
    if !bitmap && flattener.is_none() {
        return Ok(None);
    }
    let printed = vectorcraft_pdf::printed_document(doc, set);
    if bitmap {
        return bitmap_document(cmd, &printed, set, rich_black).map(Some);
    }
    let Some(mut o) = flattener else { return Ok(None) };
    // Composite overprints that were discarded or simulated aren't preserved either.
    o.preserve_overprints &= !composite || set.advanced.overprints == PrintOverprints::Preserve;
    Ok(Some(flatten_document(&printed, &o)?.unwrap_or_else(|| printed.into_owned())))
}

/// Leave guide paths out of `nodes` (the renderer draws them; they never print).
fn drop_guides(nodes: &mut Vec<Arc<Node>>) {
    fn has_guides(n: &Node) -> bool {
        matches!(n.kind, NodeKind::Path { guide: true, .. }) || n.children().is_some_and(|c| c.iter().any(|c| has_guides(c)))
    }
    nodes.retain(|n| !matches!(n.kind, NodeKind::Path { guide: true, .. }));
    for n in nodes.iter_mut().filter(|n| has_guides(n)) {
        if let Some(children) = Arc::make_mut(n).children_mut() {
            drop_guides(children);
        }
    }
}

/// `doc` (as it prints) with its art replaced by one image of each printed region (an artboard
/// with its bleed, or all the art), at the document's raster effects resolution; regions where
/// nothing is drawn get none.
fn bitmap_document(cmd: &str, doc: &Document, set: &PrintSettings, rich_black: bool) -> Result<Document> {
    let regions = vectorcraft_pdf::print_regions(doc, set).map_err(|e| pdf_error(cmd, e))?;
    let bleed = set.bleed.of(doc);
    let mut art = doc.clone();
    drop_guides(&mut art.layers);
    let anti_alias = if doc.raster_effects.anti_alias { AntiAlias::Art } else { AntiAlias::None };
    let opts = RenderOptions { skip_templates: true, anti_alias, rich_black, ..Default::default() };
    let mut out = doc.clone();
    let mut renderer = vectorcraft_render::Renderer::new();
    let mut images = vec![];
    for (_, r) in regions {
        let area = outset(r, bleed);
        // At most MAX_PIXELS, and 65535 pixels a side.
        let side = area.width().max(area.height()).max(1.0);
        let scale = (doc.raster_effects_ppi / 72.0).min((MAX_PIXELS / (area.width() * area.height()).max(1.0)).sqrt()).min(65_535.0 / side);
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
    }
    let mut layer = Node::layer(out.alloc_id(), "Bitmap", LayerColor::Preset(0));
    if let Some(children) = layer.children_mut() {
        *children = images;
    }
    out.layers = vec![Arc::new(layer)];
    Ok(out)
}
