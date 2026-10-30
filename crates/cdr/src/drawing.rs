//! Interpret the public CDR record layouts into native document objects.
use crate::{Imported, bitmap, container::Chunk, reader::Reader};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, Dash, Document, FillLayer, ImageObject, LayerColor, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer,
};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape};

// A cubic stores two control coordinates as well as its editable endpoint.
// Bound editable storage to one million anchors while allowing control-heavy art
// to use up to two million encoded/generated coordinates. Both count aliases.
const MAX_POINTS: usize = 2_000_000;
const MAX_ANCHORS: usize = 1_000_000;
const MAX_NAME_BYTES: usize = 16 * 1024 * 1024;
const MAX_COORD: f64 = 1e6;
const DEFAULT_PAGE_SIZE: (f64, f64) = (612.0 / 72.0, 792.0 / 72.0);

#[path = "powerclip.rs"]
mod powerclip;

/// Build editable pages in points from native records, retaining omission warnings.
pub(crate) fn import(chunks: &[Chunk], version: u32) -> Result<Imported, String> {
    let mut d = Drawing::new(version);
    d.index_clips(chunks)?;
    let mut size = Some(DEFAULT_PAGE_SIZE);
    d.styles(chunks, &mut size, &mut BTreeSet::new())?;
    let mut pages = Vec::new();
    find_lists(chunks, b"page", &mut pages);
    if pages.is_empty() {
        return Err("CDR contains no native drawing pages".into());
    }
    if pages.len() > 1000 {
        return Err("CDR page limit exceeded".into());
    }
    let initial_size = size.unwrap_or(DEFAULT_PAGE_SIZE);
    let mut document = Document::new(initial_size.0 * 72.0, initial_size.1 * 72.0);
    document.layers.clear();
    document.artboards.clear();
    let mut x = 0.0;
    for page in pages {
        if page.child(b"flgs").and_then(|c| c.data.get(2)).is_some_and(|v| *v != 0) {
            if has_art(page) {
                d.warn("Master-page artwork is omitted");
            }
            continue;
        }
        let mut page_size = size;
        if let Some(loda) = metadata(page, b"loda").or_else(|| metadata(page, b"lobj")) {
            let (_, args) = arguments(&loda.data)?;
            if let Some(arg) = args.get(&19130) {
                let mut r = Reader::new(arg);
                page_size = stored_size(r.coord()?, r.coord()?)?.or(size);
            }
        }
        let index = document.artboards.len();
        let frame;
        let mut nodes;
        if let Some((width, height)) = page_size {
            let (width, height) = (width * 72.0, height * 72.0);
            // Sized Corel pages have their y-up origin at the page centre.
            let page_xf = Affine::new([72.0, 0.0, 0.0, -72.0, x + width / 2.0, height / 2.0]);
            nodes = d.children(&page.children, page_xf)?;
            frame = Rect::new(x, 0.0, x + width, height);
        } else {
            // A real zero-page CDR can still contain complete native curves. With
            // no page centre, retain the drawing's origin and infer only its board.
            nodes = d.children(&page.children, Affine::new([72.0, 0.0, 0.0, -72.0, 0.0, 0.0]))?;
            frame = if let Some(mut bounds) =
                nodes.iter().filter(|node| node.visible).filter_map(|node| node.visual_bounds()).reduce(|bounds, next| bounds.union(next))
            {
                if bounds.width() == 0.0 {
                    bounds = bounds.inflate(0.5, 0.0);
                }
                if bounds.height() == 0.0 {
                    bounds = bounds.inflate(0.0, 0.5);
                }
                valid_size(bounds.width() / 72.0, bounds.height() / 72.0)?;
                // Subsequent boards use the importer's usual row layout. Translation
                // moves geometry and paint together; no recovered artwork is scaled.
                if index > 0 {
                    let shift = x - bounds.x0;
                    let xf = Affine::translate((shift, 0.0));
                    for node in &mut nodes {
                        Arc::make_mut(node).transform(xf, false);
                    }
                    bounds = xf.transform_rect_bbox(bounds);
                }
                if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
                    return Err("CDR inferred page exceeds document coordinate limits".into());
                }
                d.warn("CDR has no positive page size; an artboard was fitted to the imported artwork");
                bounds
            } else {
                d.warn("CDR has no positive page size or artwork bounds; a default artboard was used");
                Rect::new(x, 0.0, x + DEFAULT_PAGE_SIZE.0 * 72.0, DEFAULT_PAGE_SIZE.1 * 72.0)
            };
        }
        document.artboards.push(Artboard {
            id: index as u32 + 1,
            name: format!("Page {}", index + 1),
            rect: frame,
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
            show_video_safe_areas: false,
            video_ruler_pixel_aspect_ratio: 1.0,
            color: None,
        });
        let mut layer = Node::layer(d.id(), &format!("Page {}", index + 1), LayerColor::Preset(index as u8 % 27));
        if let Some(children) = layer.children_mut() {
            *children = nodes;
        }
        document.layers.push(Arc::new(layer));
        x = frame.x1 + 36.0;
    }
    if document.artboards.is_empty() {
        return Err("CDR contains no content pages".into());
    }
    if d.objects_seen > 0 && d.objects_imported == 0 {
        return Err("CDR contains no supported editable artwork; convert text to curves or export the drawing as SVG/PDF in CorelDRAW".into());
    }
    document.fix_next_id();
    document.images = d.bitmaps.into_values().map(|b| (b.key, b.blob)).collect();
    let mut warnings = d.warnings.into_iter().collect();
    document.limit_depth_noting(&mut warnings);
    Ok(Imported { document, version, warnings })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Extract the original generated native bitmap without using a producer fixture.
    fn bitmap_data() -> Vec<u8> {
        let (chunks, _) = crate::container::load(&crate::synth::clipped_bitmap()).unwrap();
        chunks.into_iter().find(|c| c.id == *b"bmp ").unwrap().data
    }

    /// Materialize a bitmap leaf as the container reader does for an external alias.
    fn bitmap_chunk(data: Vec<u8>) -> Chunk {
        Chunk { id: *b"bmp ", list: None, data, children: Vec::new() }
    }

    /// Repeated resource aliases inside nested lists must neither decode nor charge twice.
    #[test]
    fn exact_bitmap_aliases_share_decode_and_budget() {
        let data = bitmap_data();
        let records = [
            bitmap_chunk(data.clone()),
            Chunk { id: *b"LIST", list: Some(*b"grp "), data: Vec::new(), children: vec![bitmap_chunk(data.clone()), bitmap_chunk(data)] },
        ];
        let mut drawing = Drawing::new(1200);
        let mut seen = BTreeSet::new();
        let mut size = Some((1.0, 1.0));
        drawing.styles(&records[..1], &mut size, &mut seen).unwrap();
        let budget = drawing.bitmap_budget.clone();
        drawing.styles(&records[1..], &mut size, &mut seen).unwrap();
        assert_eq!(drawing.bitmaps.len(), 1);
        assert_eq!(seen.len(), 1);
        assert_eq!(drawing.bitmap_budget, budget);
    }

    /// Nonpixel metadata may vary; distinct decoding work stays charged, exact repeats do not.
    #[test]
    fn equivalent_bitmap_metadata_keeps_first_image_and_bounds_work() {
        let data = bitmap_data();
        let mut variant = data.clone();
        variant[16] ^= 1; // Outer resource metadata is not part of the RI pixel header.
        let records = [bitmap_chunk(data), bitmap_chunk(variant.clone()), bitmap_chunk(variant)];
        let mut drawing = Drawing::new(1200);
        let mut seen = BTreeSet::new();
        let mut size = Some((1.0, 1.0));
        drawing.styles(&records[..1], &mut size, &mut seen).unwrap();
        let first = drawing.bitmaps.get(&7).unwrap().blob.bytes.clone();
        let budget = drawing.bitmap_budget.clone();
        drawing.styles(&records[1..2], &mut size, &mut seen).unwrap();
        assert_ne!(drawing.bitmap_budget, budget);
        let budget = drawing.bitmap_budget.clone();
        drawing.styles(&records[2..], &mut size, &mut seen).unwrap();
        assert_eq!(drawing.bitmaps.len(), 1);
        assert_eq!(drawing.bitmaps.get(&7).unwrap().blob.bytes, first);
        assert_eq!(drawing.bitmap_budget, budget);
        assert_eq!(seen.len(), 2);
    }

    /// A changed actual pixel under the same resource identifier is an error, not a replacement.
    #[test]
    fn conflicting_bitmap_pixels_reject_without_replacing_first() {
        let data = bitmap_data();
        let mut variant = data.clone();
        variant[40 + 78] ^= 255; // First BGR pixel after the native RI header.
        let records = [bitmap_chunk(data), bitmap_chunk(variant)];
        let mut drawing = Drawing::new(1200);
        let mut seen = BTreeSet::new();
        let mut size = Some((1.0, 1.0));
        drawing.styles(&records[..1], &mut size, &mut seen).unwrap();
        let first = drawing.bitmaps.get(&7).unwrap().blob.bytes.clone();
        let error = drawing.styles(&records[1..], &mut size, &mut seen).unwrap_err();
        assert!(error.contains("conflicting CDR bitmap resource identifier"));
        assert_eq!(drawing.bitmaps.len(), 1);
        assert_eq!(drawing.bitmaps.get(&7).unwrap().blob.bytes, first);
    }

    /// A complete native drawing still opens when an external list repeats its bitmap definition.
    #[test]
    fn drawing_import_accepts_nested_bitmap_alias() {
        let (mut chunks, version) = crate::container::load(&crate::synth::clipped_bitmap()).unwrap();
        chunks.push(Chunk { id: *b"LIST", list: Some(*b"bmpf"), data: Vec::new(), children: vec![bitmap_chunk(bitmap_data())] });
        let imported = import(&chunks, version).unwrap();
        assert_eq!(imported.document.images.len(), 1);
        assert!(!imported.document.layers.is_empty());
    }

    /// Charge decoded UTF-8 bytes for both name encodings, including repeated occurrences.
    #[test]
    fn names_obey_the_cumulative_byte_quota() {
        for version in [1100, 1200] {
            let bytes = if version >= 1200 { "界".encode_utf16().flat_map(u16::to_le_bytes).collect() } else { "界".as_bytes().to_vec() };
            let mut drawing = Drawing::new(version);
            drawing.name_bytes = MAX_NAME_BYTES - 6;
            assert_eq!(drawing.name(Some(&bytes)).unwrap().as_deref(), Some("界"));
            assert_eq!(drawing.name(Some(&bytes)).unwrap().as_deref(), Some("界"));
            assert_eq!(drawing.name_bytes, MAX_NAME_BYTES);
            assert_eq!(drawing.name(None).unwrap(), None);
            assert_eq!(drawing.name(Some(&[])).unwrap(), None);
            assert!(drawing.name(Some(&bytes)).unwrap_err().contains("name materialization limit"));
            assert_eq!(drawing.name_bytes, MAX_NAME_BYTES);
        }
    }

    /// Original cubic records let tests vary raw coordinate and editable-anchor counts independently.
    fn cubic_records(segments: usize) -> Vec<u8> {
        let count = 1 + segments * 3;
        let mut bytes = (count as u32).to_le_bytes().to_vec();
        bytes.extend([0; 8]);
        for i in 0..segments {
            let x = (i % 1000) as i32 * 254;
            for (x, y) in [(x + 70, 254i32), (x + 180, -254), (x + 254, 0)] {
                bytes.extend(x.to_le_bytes());
                bytes.extend(y.to_le_bytes());
            }
        }
        bytes.push(0);
        for _ in 0..segments {
            bytes.extend([0xc0, 0xc0, 0x80]);
        }
        bytes
    }

    /// More than one million coordinates are legitimate when most are cubic handles.
    #[test]
    fn control_heavy_curves_keep_every_cubic_beyond_the_old_point_limit() {
        let segments = 356_245;
        let bytes = cubic_records(segments);
        let mut drawing = Drawing::new(2403);
        let path = drawing.curve(&bytes, false).unwrap();
        assert_eq!(drawing.points, 1_068_736);
        assert_eq!(drawing.anchors, segments + 1);
        assert_eq!(path.elements().len(), segments + 1);
        assert!(path.elements().iter().skip(1).all(|element| matches!(element, vectorcraft_geom::PathEl::CurveTo(..))));
        let native = PathData::from_bezpath(&path);
        assert_eq!(native.subpaths.len(), 1);
        assert_eq!(native.anchor_count(), segments + 1);
        assert!(native.subpaths[0].anchors[0].has_out());
        assert!(native.subpaths[0].anchors[1].has_in());
    }

    /// Coordinate work and native anchor storage keep independent finite ceilings.
    #[test]
    fn point_and_anchor_quotas_remain_cumulative_and_reject_without_reset() {
        let mut drawing = Drawing::new(1200);
        drawing.add_points(MAX_POINTS).unwrap();
        assert!(drawing.add_points(1).unwrap_err().contains("geometry point limit"));
        assert_eq!(drawing.points, MAX_POINTS);
        drawing.add_anchors(MAX_ANCHORS).unwrap();
        assert!(drawing.add_anchors(1).unwrap_err().contains("editable anchor limit"));
        assert_eq!(drawing.anchors, MAX_ANCHORS);

        let mut drawing = Drawing::new(1200);
        assert!(drawing.curve(&((MAX_POINTS + 1) as u32).to_le_bytes(), false).unwrap_err().contains("geometry point limit"));
        let bytes = cubic_records(1);
        drawing.anchors = MAX_ANCHORS - 1;
        assert!(drawing.curve(&bytes, false).unwrap_err().contains("editable anchor limit"));
        assert_eq!(drawing.anchors, MAX_ANCHORS - 1);
    }

    /// Repeated resources still charge both budgets; sharing source bytes is not free geometry.
    #[test]
    fn repeated_curve_occurrences_charge_their_materialized_anchors() {
        let bytes = cubic_records(2);
        let mut drawing = Drawing::new(1200);
        drawing.anchors = MAX_ANCHORS - 6;
        for _ in 0..2 {
            drawing.curve(&bytes, false).unwrap();
        }
        assert_eq!(drawing.anchors, MAX_ANCHORS);
        assert!(drawing.curve(&bytes, false).unwrap_err().contains("editable anchor limit"));
    }

    /// A post-close line implicitly materializes a start as well as its endpoint.
    #[test]
    fn implicit_post_close_starts_are_included_in_the_anchor_budget() {
        let flags = [0, 0x48, 0x40, 0x48];
        let mut bytes = 4u32.to_le_bytes().to_vec();
        for i in 0..4i32 {
            bytes.extend((i * 254000).to_le_bytes());
            bytes.extend(0i32.to_le_bytes());
        }
        bytes.extend(flags);
        let mut drawing = Drawing::new(1200);
        let path = drawing.curve(&bytes, false).unwrap();
        assert_eq!(drawing.anchors, 5);
        assert_eq!(path_anchor_bound(&path), 5);
        assert!(PathData::from_bezpath(&path).anchor_count() <= drawing.anchors);
        let mut drawing = Drawing::new(1200);
        drawing.anchors = MAX_ANCHORS - 4;
        assert!(drawing.curve(&bytes, false).unwrap_err().contains("editable anchor limit"));
    }
}

/// Detect object records even inside containers whose artwork is unsupported.
fn has_art(c: &Chunk) -> bool {
    c.list.as_ref().is_some_and(|k| k == b"obj ") || c.children.iter().any(has_art)
}
/// Find this object's property, descending through `lgob`/`trfl` wrappers only.
fn metadata<'a>(c: &'a Chunk, id: &[u8; 4]) -> Option<&'a Chunk> {
    c.child(id).or_else(|| c.children.iter().filter(|c| matches!(c.list.as_ref(), Some(b"lgob" | b"trfl"))).find_map(|c| metadata(c, id)))
}
/// Collect matching lists without descending into a match a second time.
fn find_lists<'a>(chunks: &'a [Chunk], kind: &[u8; 4], out: &mut Vec<&'a Chunk>) {
    for c in chunks {
        if c.list.as_ref() == Some(kind) {
            out.push(c);
        } else {
            find_lists(&c.children, kind, out);
        }
    }
}
/// Validate positive page dimensions in inches against the document point limit.
fn valid_size(w: f64, h: f64) -> Result<(f64, f64), String> {
    if w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0 && w * 72.0 <= MAX_COORD && h * 72.0 <= MAX_COORD {
        Ok((w, h))
    } else {
        Err("invalid CDR page dimensions".into())
    }
}

/// Zero-by-zero means no usable page size, as observed in a drawing-only CDR24.3.
/// Partial zeros, negative dimensions and out-of-range dimensions remain errors.
fn stored_size(w: f64, h: f64) -> Result<Option<(f64, f64)>, String> {
    if w == 0.0 && h == 0.0 { Ok(None) } else { valid_size(w, h).map(Some) }
}

/// Bound anchors before decoding a native curve. A line/cubic following a close
/// starts an implicit subpath in PathData, so charge its extra starting anchor too.
fn curve_anchor_bound(types: &[u8]) -> usize {
    let mut anchors = 0;
    let mut open = false;
    for typ in types {
        match typ >> 6 {
            0 => {
                anchors += 1;
                open = true;
            }
            1 | 2 => {
                anchors += 1 + usize::from(!open);
                open = typ & 8 == 0;
            }
            _ => {}
        }
    }
    anchors
}

/// Bound the anchors of procedurally expanded paths before native materialization.
fn path_anchor_bound(path: &BezPath) -> usize {
    let mut anchors = 0;
    let mut open = false;
    for element in path.elements() {
        match element {
            vectorcraft_geom::PathEl::MoveTo(_) => {
                anchors += 1;
                open = true;
            }
            vectorcraft_geom::PathEl::LineTo(_) | vectorcraft_geom::PathEl::QuadTo(_, _) | vectorcraft_geom::PathEl::CurveTo(_, _, _) => {
                anchors += 1 + usize::from(!open);
                open = true;
            }
            vectorcraft_geom::PathEl::ClosePath => open = false,
        }
    }
    anchors
}

/// Read an object kind and bounded borrowed arguments from its `loda` table.
/// Offsets are stored in forward order and argument types in reverse order.
fn arguments(data: &[u8]) -> Result<(u32, BTreeMap<u32, &[u8]>), String> {
    let mut r = Reader::new(data);
    let len = r.u32()? as usize;
    let count = r.u32()? as usize;
    let offsets = r.u32()? as usize;
    let types = r.u32()? as usize;
    let kind = r.u32()?;
    if len > data.len() || count > 4096 {
        return Err("invalid CDR object argument table".into());
    }
    let mut ro = r.at(offsets)?;
    let mut rt = r.at(types)?;
    let mut off = Vec::new();
    let mut typ = Vec::new();
    for _ in 0..count {
        off.push(ro.u32()? as usize);
        typ.push(rt.u32()?);
    }
    // Some files omit the sentinel offset. End each argument at the next offset,
    // or at the first following table/end boundary.
    let mut boundaries = off.clone();
    boundaries.extend([offsets, types, len]);
    boundaries.sort_unstable();
    boundaries.dedup();
    let mut result = BTreeMap::new();
    for (i, &start) in off.iter().enumerate() {
        let end = *boundaries.get(boundaries.partition_point(|v| *v <= start)).ok_or("invalid CDR argument offset")?;
        if end > len {
            return Err("CDR argument exceeds its declared object length".into());
        }
        let id = typ.get(count - 1 - i).copied().ok_or("missing CDR argument type")?;
        result.insert(id, data.get(start..end).ok_or("CDR argument outside record")?);
    }
    Ok((kind, result))
}

struct Drawing<'a> {
    version: u32,
    fills: BTreeMap<u32, Fill>,
    strokes: BTreeMap<u32, Option<StrokeLayer>>,
    warnings: BTreeSet<String>,
    points: usize,
    anchors: usize,
    next_id: u64,
    objects_seen: usize,
    objects_imported: usize,
    paint_values: usize,
    name_bytes: usize,
    bitmaps: BTreeMap<u32, bitmap::Bitmap>,
    bitmap_budget: bitmap::BitmapBudget,
    clips: BTreeMap<u32, &'a Chunk>,
    clip_owners: BTreeMap<u32, &'a Chunk>,
    active_clips: BTreeSet<u32>,
    visits: usize,
    depth: usize,
}

#[derive(Clone)]
enum Fill {
    Plain(Paint),
    Fountain(Fountain),
}

#[derive(Clone)]
struct Fountain {
    kind: GradientKind,
    stops: Vec<GradientStop>,
    angle: f64,
    edge: f64,
    center: (f64, f64),
    /// Modern gradient centre offset and axis vector, relative to object bounds.
    tail: Option<[f64; 4]>,
}
impl Fill {
    /// Place a solid or fountain paint in the object's bounds and document transform.
    fn paint(&self, bounds: Rect, xf: Affine) -> Result<Paint, String> {
        match self {
            Self::Plain(paint) => Ok(paint.clone()),
            Self::Fountain(f) => {
                let mut geom = GradientGeom::fit(f.kind, bounds, -f.angle);
                let center = bounds.center();
                let offset = vectorcraft_geom::Vec2::new(bounds.width() * f.center.0 / 100.0, bounds.height() * f.center.1 / 100.0);
                geom.start += offset;
                geom.end += offset;
                if f.kind == GradientKind::Linear {
                    let mid = geom.start.midpoint(geom.end);
                    let v = (geom.end - geom.start) * (1.0 - 2.0 * f.edge);
                    geom.start = mid - v * 0.5;
                    geom.end = mid + v * 0.5;
                } else {
                    geom.end = geom.start + (geom.end - geom.start) * (1.0 - 2.0 * f.edge);
                }
                if let Some([cx, cy, ax, ay]) = f.tail {
                    let c = center + vectorcraft_geom::Vec2::new(cx * bounds.width(), -cy * bounds.height());
                    let v = vectorcraft_geom::Vec2::new(ax * bounds.width(), -ay * bounds.height());
                    if v.hypot() > 1e-12 {
                        if f.kind == GradientKind::Linear {
                            geom.start = c - v * 0.5;
                            geom.end = c + v * 0.5;
                        } else {
                            geom.start = c;
                            geom.end = c + v * 0.5;
                            geom.aspect = if bounds.width().abs() > 1e-12 { bounds.height() / bounds.width() } else { 1.0 };
                        }
                    }
                }
                geom.transform(xf, f.kind);
                if ![geom.start.x, geom.start.y, geom.end.x, geom.end.y, geom.aspect].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
                    return Err("CDR gradient placement exceeds document limits".into());
                }
                let mut gradient = GradientPaint::new(Gradient::new(f.kind, f.stops.clone()));
                gradient.geom = Some(geom);
                Ok(Paint::Gradient(Box::new(gradient)))
            }
        }
    }
}
impl<'a> Drawing<'a> {
    /// Start one import with independent resource maps and cumulative work budgets.
    fn new(version: u32) -> Self {
        Self {
            version,
            fills: BTreeMap::new(),
            strokes: BTreeMap::new(),
            warnings: BTreeSet::new(),
            points: 0,
            anchors: 0,
            next_id: 1,
            objects_seen: 0,
            objects_imported: 0,
            paint_values: 0,
            name_bytes: 0,
            bitmaps: BTreeMap::new(),
            bitmap_budget: bitmap::BitmapBudget::default(),
            clips: BTreeMap::new(),
            clip_owners: BTreeMap::new(),
            active_clips: BTreeSet::new(),
            visits: 0,
            depth: 0,
        }
    }
    /// Retain at most 100 distinct warnings about omitted or approximated features.
    fn warn(&mut self, text: &str) {
        if self.warnings.len() < 100 {
            self.warnings.insert(text.to_owned());
        }
    }
    /// Allocate a unique document node ID within the bounded record traversal.
    fn id(&mut self) -> NodeId {
        let id = self.next_id;
        self.next_id += 1;
        NodeId(id)
    }
    /// Collect fill/outline definitions and page defaults before resolving objects.
    fn styles<'s>(&mut self, chunks: &'s [Chunk], size: &mut Option<(f64, f64)>, bitmap_sources: &mut BTreeSet<&'s [u8]>) -> Result<(), String> {
        for c in chunks {
            if self.fills.len() + self.strokes.len() > 20_000 {
                return Err("CDR style count limit exceeded".into());
            }
            match &c.id {
                b"fild" | b"fill" => {
                    let (id, p) = self.fill(&c.data)?;
                    self.fills.insert(id, p);
                }
                b"outl" => {
                    let (id, s) = self.stroke(&c.data)?;
                    self.strokes.insert(id, s);
                }
                b"mcfg" => {
                    let mut r = Reader::new(&c.data);
                    r.skip(if self.version >= 1300 {
                        12
                    } else if self.version >= 900 {
                        4
                    } else if self.version < 700 {
                        28
                    } else {
                        0
                    })?;
                    *size = stored_size(r.coord()?, r.coord()?)?;
                }
                b"txsm" => self.warn("Text objects are omitted; convert text to curves in CorelDRAW for editable outlines"),
                b"bmp " => {
                    // External LIST aliases materialize identical leaves. Compare borrowed bytes
                    // before decoding so those aliases consume neither image quota nor another PNG.
                    if !bitmap_sources.insert(c.data.as_slice()) {
                        continue;
                    }
                    if let Some(bitmap) = bitmap::decode(&c.data, &mut self.bitmap_budget)? {
                        if bitmap.cmyk_converted {
                            self.warn("CMYK bitmap colours are approximated in RGB without the embedded colour profile");
                        }
                        if let Some(first) = self.bitmaps.get(&bitmap.id) {
                            // Different metadata/padding may encode the same pixels. Compare the
                            // full deterministic PNG, never just its content-derived key. This
                            // distinct decode remains charged to bound actual conversion work.
                            if first.width != bitmap.width
                                || first.height != bitmap.height
                                || first.blob.bytes.as_slice() != bitmap.blob.bytes.as_slice()
                            {
                                return Err("conflicting CDR bitmap resource identifier".into());
                            }
                        } else {
                            self.bitmaps.insert(bitmap.id, bitmap);
                        }
                    } else {
                        self.warn("An unsupported CDR bitmap encoding was omitted");
                    }
                }
                b"bmpf" => self.warn("Bitmap pattern fills are omitted"),
                b"iccd" => self.warn("Embedded colour profiles are not imported; numeric colours are preserved"),
                _ => {}
            }
            self.styles(&c.children, size, bitmap_sources)?;
        }
        Ok(())
    }
    /// Decode supported numeric colour models and warn when substituting black.
    fn color(&mut self, r: &mut Reader<'_>) -> Result<Color, String> {
        let model = r.u16()?;
        let palette = r.u16()?;
        r.skip(4)?;
        let b = r.take(4)?;
        let a = b.first().copied().unwrap_or(0);
        let c = b.get(1).copied().unwrap_or(0);
        let d = b.get(2).copied().unwrap_or(0);
        let e = b.get(3).copied().unwrap_or(0);
        let color = match model {
            2 => Color::cmyk((a as f32 / 100.0).min(1.0), (c as f32 / 100.0).min(1.0), (d as f32 / 100.0).min(1.0), (e as f32 / 100.0).min(1.0)),
            3 | 17 => Color::cmyk(a as f32 / 255.0, c as f32 / 255.0, d as f32 / 255.0, e as f32 / 255.0),
            4 => Color::cmyk(a as f32 / 255.0, c as f32 / 255.0, d as f32 / 255.0, 0.0),
            5 | 21 => Color::rgb8(d, c, a),
            8 => {
                if a == 0 {
                    Color::WHITE
                } else {
                    Color::BLACK
                }
            }
            9 => Color::gray(1.0 - a as f32 / 255.0),
            12 => Color::lab(a as f32 * 100.0 / 255.0, c as i8 as f32, d as i8 as f32),
            18 => Color::lab(a as f32 * 100.0 / 255.0, c as f32 - 128.0, d as f32 - 128.0),
            20 => {
                self.warn("Registration colour is approximated with black");
                Color::BLACK
            }
            _ => {
                self.warn(&format!("Colour model {model} (palette {palette}) is unsupported and uses black"));
                Color::BLACK
            }
        };
        Ok(color)
    }
    /// Read a versioned fill definition, omitting unsupported fill kinds with a warning.
    fn fill(&mut self, bytes: &[u8]) -> Result<(u32, Fill), String> {
        let mut r = Reader::new(bytes);
        let id = r.u32()?;
        if self.version >= 1300 {
            loop {
                let tag = r.u32()?;
                let len = r.u32()? as usize;
                if tag == 1300 {
                    break;
                }
                r.skip(len)?;
            }
        }
        let kind = r.u16()?;
        if kind == 0 {
            return Ok((id, Fill::Plain(Paint::None)));
        }
        if kind == 2 {
            return Ok((id, Fill::Fountain(self.fountain(&mut r)?)));
        }
        if kind != 1 {
            self.warn(&format!("CDR fill type {kind} is omitted (solid, linear and radial fills are supported)"));
            return Ok((id, Fill::Plain(Paint::None)));
        }
        let color = if self.version < 1300 {
            r.skip(2)?;
            self.color(&mut r)?
        } else {
            r.skip(4)?;
            let n = r.u32()? as usize;
            let mut props = Reader::new(r.take(n)?);
            let mut found = None;
            for _ in 0..1024 {
                let kind = props.u8()?;
                let n = props.u32()? as usize;
                let data = props.take(n)?;
                if kind == 0 {
                    break;
                }
                if kind == 1 {
                    found = Some(self.color(&mut Reader::new(data))?);
                }
            }
            found.ok_or("CDR solid fill has no colour property")?
        };
        Ok((id, Fill::Plain(Paint::solid(color))))
    }
    /// Read bounded fountain stops and placement, recording interpolation approximations.
    fn fountain(&mut self, r: &mut Reader<'_>) -> Result<Fountain, String> {
        r.skip(if self.version >= 1300 { 8 } else { 2 })?;
        let kind = r.u8()?;
        let kind = match kind {
            1 => GradientKind::Linear,
            2 => GradientKind::Radial,
            _ => {
                self.warn("Conical or square fountain fills are approximated with a radial gradient");
                GradientKind::Radial
            }
        };
        r.skip(if self.version >= 1300 { 17 } else { 19 })?;
        let edge = if self.version >= 1300 { r.u16()? as i16 as f64 } else { r.u32()? as i32 as f64 } / 100.0;
        let angle = r.u32()? as i32 as f64 / 1_000_000.0;
        let center = (r.u32()? as i32 as f64, r.u32()? as i32 as f64);
        r.skip(2)?;
        let mode = r.u32()? & 0xff;
        let midpoint = r.u8()? as f32 / 100.0;
        r.skip(1)?;
        let count = (r.u32()? & 0xffff) as usize;
        if count > 4096 {
            return Err("CDR gradient stop limit exceeded".into());
        }
        if self.version >= 1300 {
            r.skip(3)?;
        }
        let mut stops = Vec::new();
        self.add_paint_values(count)?;
        for _ in 0..count {
            let color = self.color(r)?;
            r.skip(if self.version >= 1500 {
                26
            } else if self.version >= 1300 {
                5
            } else {
                0
            })?;
            let offset = (r.u32()? & 0xffff) as f32 / 100.0;
            if offset > 1.0 {
                return Err("invalid CDR gradient stop position".into());
            }
            let mut stop = GradientStop::new(if kind == GradientKind::Radial { 1.0 - offset } else { offset }, color);
            stop.midpoint = midpoint.clamp(0.01, 0.99);
            stops.push(stop);
            if self.version >= 1300 {
                r.skip(3)?;
            }
        }
        if stops.len() < 2 {
            return Err("CDR gradient contains fewer than two stops".into());
        }
        stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
        if mode != 0 {
            self.warn("Fountain colour interpolation modes are approximated in the document colour model");
        }
        if self.version >= 1300 {
            r.skip(3)?;
        }
        let tail =
            if self.version >= 1600 && r.bytes.len().saturating_sub(r.pos) >= 32 { Some([r.f64()?, r.f64()?, r.f64()?, r.f64()?]) } else { None };
        if edge.abs() > 0.5 || center.0.abs() > 1000.0 || center.1.abs() > 1000.0 || tail.is_some_and(|v| v.iter().any(|v| v.abs() > 1000.0)) {
            return Err("invalid CDR gradient placement".into());
        }
        if tail.is_some_and(|v| v.first().is_some_and(|v| *v != 0.0) || v.get(1).is_some_and(|v| *v != 0.0) || v.get(3).is_some_and(|v| *v != 0.0)) {
            self.warn("Advanced fountain centre offsets and rotated axes may be approximated");
        }
        Ok(Fountain { kind, stops, angle, edge, center, tail })
    }
    /// Decode outline width, colour, caps, joins and dashes, or an explicit no-outline style.
    fn stroke(&mut self, bytes: &[u8]) -> Result<(u32, Option<StrokeLayer>), String> {
        let mut r = Reader::new(bytes);
        let id = r.u32()?;
        if self.version >= 1300 {
            loop {
                let kind = r.u32()?;
                let len = r.u32()? as usize;
                if kind == 1 {
                    break;
                }
                r.skip(len)?;
            }
        }
        let kind = r.u16()?;
        let cap = r.u16()?;
        let join = r.u16()?;
        if self.version < 1300 {
            r.skip(2)?;
        }
        let width = r.coord()?.abs() * 72.0;
        let stretch = r.u16()?;
        if self.version < 1300 {
            r.skip(2)?;
        }
        let angle = r.u32()?;
        r.skip(if self.version >= 1300 { 48 } else { 52 })?;
        let color = self.color(&mut r)?;
        if kind & 1 != 0 {
            return Ok((id, None));
        }
        let mut stroke = StrokeLayer::new(Paint::solid(color), width.max(0.01));
        stroke.cap = match cap {
            1 => LineCap::Round,
            2 => LineCap::Square,
            _ => LineCap::Butt,
        };
        stroke.join = match join {
            1 => LineJoin::Round,
            2 => LineJoin::Bevel,
            _ => LineJoin::Miter,
        };
        if stretch != 100 || angle != 0 {
            self.warn("Calligraphic outline stretch and angle are approximated with a circular pen");
        }
        if kind & 0x20 != 0 {
            self.warn("Outlines that scale with objects use their stored widths");
        }
        if kind & 0x10 != 0 {
            self.warn("Outlines behind the fill are drawn above it");
        }
        r.skip(16)?;
        let count = r.u16()? as usize;
        if count > 1024 {
            return Err("CDR outline dash limit exceeded".into());
        }
        self.add_paint_values(count)?;
        let dash_start = r.pos;
        if count > 0 && kind & 4 != 0 {
            let mut pattern = Vec::new();
            for _ in 0..count {
                pattern.push(r.u16()? as f64 * stroke.width);
            }
            stroke.dash = Some(Dash { pattern, ..Dash::default() });
        }
        let mut arrows = r.at(dash_start + if self.version >= 1300 { 20 } else { 22 })?;
        if arrows.u32()? != 0 || arrows.u32()? != 0 {
            self.warn("Outline arrowheads are omitted");
        }
        Ok((id, Some(stroke)))
    }
    /// Compose this object's affine transforms, converting their translations to inches.
    fn transform(&mut self, c: &Chunk) -> Result<Affine, String> {
        let Some(c) = metadata(c, b"trfd") else { return Ok(Affine::IDENTITY) };
        let mut r = Reader::new(&c.data);
        let len = r.u32()? as usize;
        let count = r.u32()? as usize;
        let table = r.u32()? as usize;
        if count > 4096 || len > c.data.len() {
            return Err("invalid CDR transform table".into());
        }
        let mut offsets = r.at(table)?;
        let mut xf = Affine::IDENTITY;
        for _ in 0..count {
            let offset = offsets.u32()? as usize;
            let mut value = r.at(offset)?;
            if self.version >= 1300 {
                value.skip(8)?;
            }
            if value.u16()? != 8 {
                self.warn("A non-affine CDR transform is omitted");
                continue;
            }
            value.skip(6)?;
            let a = value.f64()?;
            let c = value.f64()?;
            let tx = value.f64()? / 254000.0;
            let b = value.f64()?;
            let d = value.f64()?;
            let ty = value.f64()? / 254000.0;
            xf *= Affine::new([a, b, c, d, tx, ty]);
        }
        Ok(xf)
    }
    /// Assemble layers, groups and objects in document paint order under the parent transform.
    fn children(&mut self, chunks: &[Chunk], parent: Affine) -> Result<Vec<Arc<Node>>, String> {
        if self.depth >= 64 {
            return Err("CDR drawing reference depth limit exceeded".into());
        }
        self.visits = self.visits.checked_add(chunks.len()).ok_or("CDR drawing visit count overflow")?;
        if self.visits > 200_000 {
            return Err("CDR drawing reference expansion limit exceeded".into());
        }
        self.depth += 1;
        let result = self.children_inner(chunks, parent);
        self.depth -= 1;
        result
    }
    /// Visit one bounded level; the wrapper also charges referenced PowerClip contents.
    fn children_inner(&mut self, chunks: &[Chunk], parent: Affine) -> Result<Vec<Arc<Node>>, String> {
        let mut nodes = Vec::new();
        for c in chunks {
            let Some(kind) = c.list.as_ref() else { continue };
            if kind == b"obj " {
                if let Some(n) = self.object(c, parent)? {
                    nodes.push(Arc::new(n));
                }
            } else if kind == b"grp " || kind == b"lnkg" || kind == b"layr" {
                // Ordinary group children already carry the complete transform.
                // Linked brush groups place children through the group's matrix.
                let xf = if kind == b"lnkg" { parent * self.transform(c)? } else { parent };
                let children = self.children(&c.children, xf)?;
                let mut group = if kind == b"layr" {
                    let mut n = Node::layer(self.id(), "Layer", LayerColor::Preset(0));
                    if let Some(c) = n.children_mut() {
                        *c = children;
                    }
                    n
                } else {
                    Node::group(self.id(), children)
                };
                if let Some(loda) = metadata(c, b"loda").or_else(|| metadata(c, b"lobj")) {
                    let (_, args) = arguments(&loda.data)?;
                    group.name = self.name(args.get(&1000).copied())?;
                    group.opacity = self.opacity(args.get(&8000).copied())?;
                }
                if kind == b"layr" {
                    let layer_type = c.child(b"flgs").and_then(|c| c.data.first()).copied().unwrap_or(0);
                    if layer_type & 0x0f == 0x0a {
                        group.visible = false;
                        if let vectorcraft_doc::NodeKind::Layer { printable, .. } = &mut group.kind {
                            *printable = false;
                        }
                    }
                }
                if group.children().is_some_and(|c| !c.is_empty()) {
                    nodes.push(Arc::new(group));
                }
            } else if kind == b"clpt" {
                // The indexed resource groups are drawn only through their frame references.
                if c.children.iter().any(|child| child.list.as_ref() != Some(b"grp ") || child.child(b"spnd").is_none()) && has_art(c) {
                    self.warn("An unsupported clipping container layout was omitted");
                    self.objects_seen += 1;
                }
            } else if kind != b"vect" && kind != b"stlt" {
                nodes.extend(self.children(&c.children, parent)?);
            }
        }
        // Native records store the frontmost object/layer first.
        nodes.reverse();
        Ok(nodes)
    }
    /// Decode a bounded name and charge every materialized alias against the UTF-8 byte quota.
    fn name(&mut self, bytes: Option<&[u8]>) -> Result<Option<String>, String> {
        let Some(bytes) = bytes else { return Ok(None) };
        let name = if self.version >= 1200 {
            let words: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .take(4096)
                .map(|p| u16::from_le_bytes([p.first().copied().unwrap_or(0), p.get(1).copied().unwrap_or(0)]))
                .take_while(|v| *v != 0)
                .collect();
            String::from_utf16_lossy(&words)
        } else {
            // Four bytes per scalar suffice for the first 4096 UTF-8 characters.
            // Bound the source scan/conversion too: a large invalid name can be
            // expanded repeatedly through aliases before the retained-byte quota.
            let prefix = bytes.get(..bytes.len().min(4096 * 4)).unwrap_or_default();
            String::from_utf8_lossy(prefix.split(|b| *b == 0).next().unwrap_or_default()).chars().take(4096).collect()
        };
        let total = self.name_bytes.checked_add(name.len()).ok_or("CDR name byte count overflow")?;
        if total > MAX_NAME_BYTES {
            return Err("CDR name materialization limit exceeded".into());
        }
        self.name_bytes = total;
        Ok(if name.is_empty() { None } else { Some(name) })
    }
    /// Convert optional uniform transparency to opacity, rejecting values above 1000.
    fn opacity(&mut self, bytes: Option<&[u8]>) -> Result<f32, String> {
        let Some(bytes) = bytes else { return Ok(1.0) };
        let mut r = Reader::new(bytes);
        r.skip(if self.version >= 1300 { 14 } else { 10 })?;
        let v = r.u16()?;
        if v > 1000 {
            return Err("invalid CDR opacity".into());
        }
        Ok(1.0 - v as f32 / 1000.0)
    }
    /// Resolve one supported object into an editable path, appearance and optional name.
    /// Unsupported geometry returns `None` with a warning; malformed data returns an error.
    fn object(&mut self, c: &Chunk, parent: Affine) -> Result<Option<Node>, String> {
        self.objects_seen += 1;
        let Some(loda) = metadata(c, b"loda").or_else(|| metadata(c, b"lobj")) else {
            self.warn("An object without native geometry was omitted");
            return Ok(None);
        };
        let (kind, args) = arguments(&loda.data)?;
        if kind == 20 && self.version < 1300 {
            self.warn("A legacy CDR polygon layout was omitted");
            return Ok(None);
        }
        let path = if kind == 0x26 {
            // Native B-splines keep their controls in a named property set, not
            // in the ordinary coordinate argument. Do not confuse other sets
            // (effects, plugins, etc.) with this verified geometry identifier.
            const SPLINE_PROPERTY: [u8; 16] = [0xef, 0xe9, 0x0e, 0x66, 0x4c, 0xa5, 0xbc, 0x40, 0x91, 0x0b, 0x8a, 0x16, 0x16, 0xc1, 0x5e, 0x4a];
            let mut properties = c
                .children
                .iter()
                .filter(|child| child.list.as_ref() == Some(b"npps") && child.child(b"ppid").is_some_and(|id| id.data.as_slice() == SPLINE_PROPERTY));
            let Some(property) = properties.next().filter(|_| self.version >= 1500) else {
                self.warn("An unsupported CDR B-spline property layout was omitted");
                return Ok(None);
            };
            if properties.next().is_some() {
                return Err("ambiguous CDR B-spline geometry properties".into());
            }
            let data = property.child(b"ppdt").ok_or("CDR B-spline property is missing its control points")?;
            let remaining = MAX_POINTS.saturating_sub(self.points).min(MAX_ANCHORS.saturating_sub(self.anchors));
            let (path, points) = crate::bspline::bspline(&data.data, remaining)?;
            self.add_points(points)?;
            self.add_anchors(path_anchor_bound(&path))?;
            path
        } else {
            let Some(coords) = args.get(&30) else {
                self.warn("An object without supported coordinate data was omitted");
                return Ok(None);
            };
            if kind == 5 {
                return self.bitmap_object(c, coords, parent, &args);
            }
            match kind {
                1 => self.rectangle(coords)?,
                2 => self.ellipse(coords)?,
                3 | 0x25 => self.curve(coords, kind == 0x25)?,
                20 => {
                    let parameters = args.get(&11000).ok_or("CDR polygon is missing its repetition parameters")?;
                    let remaining = MAX_POINTS.saturating_sub(self.points).min(MAX_ANCHORS.saturating_sub(self.anchors));
                    let (path, points) = crate::polygon::polygon(coords, parameters, self.version, remaining)?;
                    self.add_points(points)?;
                    self.add_anchors(path_anchor_bound(&path))?;
                    path
                }
                _ => {
                    self.warn(&format!("Native CDR object type {kind} is omitted"));
                    return Ok(None);
                }
            }
        };
        let local_bounds = path.bounding_box();
        let xf = parent * self.transform(c)?;
        let path = PathData::from_bezpath(&(xf * path));
        let bounds = path.to_bezpath().bounding_box();
        if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
            return Err("CDR geometry is non-finite or exceeds document coordinate limits".into());
        }
        let mut items = Vec::new();
        if let Some(arg) = args.get(&20) {
            let id = Reader::new(arg).u32()?;
            if id != 0 {
                let n = self.fills.get(&id).map_or(0, |f| match f {
                    Fill::Fountain(f) => f.stops.len(),
                    _ => 0,
                });
                self.add_paint_values(n)?;
                if let Some(fill) = self.fills.get(&id) {
                    let paint = fill.paint(local_bounds, xf)?;
                    if !matches!(paint, Paint::None) {
                        items.push(AppearanceItem::Fill(FillLayer::new(paint)));
                    }
                } else {
                    self.warn("An unresolved CDR fill style was omitted");
                }
            }
        }
        if let Some(arg) = args.get(&10) {
            let id = Reader::new(arg).u32()?;
            if id != 0 {
                let n = self.strokes.get(&id).and_then(|s| s.as_ref()).and_then(|s| s.dash.as_ref()).map_or(0, |d| d.pattern.len());
                self.add_paint_values(n)?;
                if let Some(Some(stroke)) = self.strokes.get(&id) {
                    items.push(AppearanceItem::Stroke(stroke.clone()));
                } else if !self.strokes.contains_key(&id) {
                    self.warn("An unresolved CDR outline style was omitted");
                }
            }
        }
        if items.iter().any(|item| {
            let paint = match item {
                AppearanceItem::Fill(fill) => &fill.paint,
                AppearanceItem::Stroke(stroke) => &stroke.paint,
            };
            match paint {
                Paint::Solid { color: Color::Cmyk { .. }, .. } => true,
                Paint::Gradient(gradient) => gradient.gradient.stops.iter().any(|stop| matches!(stop.color, Color::Cmyk { .. })),
                _ => false,
            }
        }) {
            self.warn("CMYK colours retain numeric values; display may differ because embedded colour profiles are not imported");
        }
        self.note_properties(&args);
        let mut node = Node::path(self.id(), path, Appearance { items, ..Appearance::default() });
        // Original same-winding compound paths confirm alternate coverage by default;
        // argument 14001 explicitly selects winding coverage (also for PowerClip frames).
        if let NodeKind::Path { rule, .. } = &mut node.kind {
            *rule = match args.get(&14001).map(|arg| Reader::new(arg).u32()).transpose()? {
                None | Some(0) => FillRule::EvenOdd,
                Some(1) => FillRule::NonZero,
                _ => return Err("unsupported CDR path fill rule".into()),
            };
        }
        node.name = self.name(args.get(&1000).copied())?;
        node.opacity = self.opacity(args.get(&8000).copied())?;
        if let Some(reference) = args.get(&8005) {
            node = self.powerclip(c, node, reference, parent)?;
        }
        self.objects_imported += 1;
        Ok(Some(node))
    }
    /// Retain warnings for object appearance properties that are not yet interpreted.
    fn note_properties(&mut self, args: &BTreeMap<u32, &[u8]>) {
        if args.contains_key(&200) {
            self.warn("Inherited CDR object styles are not resolved");
        }
        if args.contains_key(&201) {
            self.warn("Native appearance and transparency overrides in JSON styles are omitted");
        }
        for id in args.keys() {
            if ![10, 20, 30, 100, 200, 201, 1000, 8000, 8005, 11000, 14001, 19130, 40050].contains(id) {
                self.warn(&format!("Unsupported CDR object property {id} was omitted"));
            }
        }
    }
    /// Place an embedded image in its native coordinates and retain its editable crop outline.
    fn bitmap_object(&mut self, c: &Chunk, coords: &[u8], parent: Affine, args: &BTreeMap<u32, &[u8]>) -> Result<Option<Node>, String> {
        let mut r = Reader::new(coords);
        let (x0, y0, x1, y1) = (r.coord()?, r.coord()?, r.coord()?, r.coord()?);
        r.skip(32)?;
        let image_id = r.u32()?;
        r.skip(if (800..900).contains(&self.version) { 12 } else { 20 })?;
        let crop_start = r.pos;
        let crop_count = r.u32()? as usize;
        let Some(bitmap) = self.bitmaps.get(&image_id) else {
            self.warn("An image object with an unavailable CDR bitmap resource was omitted");
            return Ok(None);
        };
        let (width, height, key) = (bitmap.width, bitmap.height, bitmap.key.clone());
        let xf = parent * self.transform(c)?;
        let image_xf = xf * Affine::new([(x1 - x0) / width as f64, 0.0, 0.0, (y1 - y0) / height as f64, x0, y0]);
        let bounds = image_xf.transform_rect_bbox(Rect::new(0.0, 0.0, width as f64, height as f64));
        if x0 == x1 || y0 == y1 || ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
            return Err("CDR bitmap placement is empty or exceeds document coordinate limits".into());
        }
        let image = Node::new(
            self.id(),
            NodeKind::Image(Box::new(ImageObject { key, width, height, xf: image_xf, link: None, placement: Default::default(), pixels: None })),
        );
        let mut node = if crop_count == 0 {
            image
        } else {
            let crop = self.curve(coords.get(crop_start..).ok_or("truncated CDR bitmap crop")?, false)?;
            let crop = xf * crop;
            let bounds = crop.bounding_box();
            if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD) {
                return Err("CDR bitmap crop exceeds document coordinate limits".into());
            }
            // Corel's bitmap crop uses even-odd coverage, including same-winding holes.
            // Keep its appearance empty: a painted clipping path is rendered underneath the
            // image and would replace transparent pixels with the clipping path's fill.
            let mask = Node::new(
                self.id(),
                NodeKind::Path { path: PathData::from_bezpath(&crop), rule: FillRule::EvenOdd, live: None, clipping: true, guide: false },
            );
            Node::new(self.id(), NodeKind::Group { children: vec![Arc::new(mask), Arc::new(image)], clip: true })
        };
        self.note_properties(args);
        node.name = self.name(args.get(&1000).copied())?;
        node.opacity = self.opacity(args.get(&8000).copied())?;
        self.objects_imported += 1;
        Ok(Some(node))
    }
    /// Charge coordinate work against the cumulative two-million-coordinate limit.
    fn add_points(&mut self, n: usize) -> Result<(), String> {
        let total = self.points.checked_add(n).ok_or("CDR point count overflow")?;
        if total > MAX_POINTS {
            return Err("CDR geometry point limit exceeded".into());
        }
        self.points = total;
        Ok(())
    }
    /// Bound materialized editable anchors independently of cubic control coordinates.
    /// Upper bounds include duplicate closing endpoints and implicit post-close starts.
    fn add_anchors(&mut self, n: usize) -> Result<(), String> {
        let total = self.anchors.checked_add(n).ok_or("CDR anchor count overflow")?;
        if total > MAX_ANCHORS {
            return Err("CDR editable anchor limit exceeded".into());
        }
        self.anchors = total;
        Ok(())
    }
    /// Charge both style definitions and per-object gradient/dash copies against their limit.
    fn add_paint_values(&mut self, n: usize) -> Result<(), String> {
        self.paint_values = self.paint_values.checked_add(n).ok_or("CDR paint count overflow")?;
        if self.paint_values > 250_000 {
            return Err("CDR paint materialization limit exceeded".into());
        }
        Ok(())
    }
    /// Decode rectangle corners into editable local-space cubic or straight segments.
    fn rectangle(&mut self, bytes: &[u8]) -> Result<BezPath, String> {
        self.add_points(17)?;
        self.add_anchors(17)?;
        let (path, approximated) = crate::corners::rectangle(bytes, self.version)?;
        if approximated {
            self.warn("An unsupported or overlapping rectangle corner treatment was approximated");
        }
        Ok(path)
    }
    /// Approximate an ellipse, arc or pie with up to four cubic segments in local inches.
    fn ellipse(&mut self, bytes: &[u8]) -> Result<BezPath, String> {
        self.add_points(16)?;
        self.add_anchors(16)?;
        let mut r = Reader::new(bytes);
        let w = r.coord()?;
        let h = r.coord()?;
        let a1 = (r.u32()? as i32 as f64 / 1_000_000.0).to_radians();
        let a2 = (r.u32()? as i32 as f64 / 1_000_000.0).to_radians();
        let pie = r.u32()? != 0;
        let tau = std::f64::consts::TAU;
        let start = a1.rem_euclid(tau);
        let sweep = (a2 - a1).rem_euclid(tau);
        let full = sweep.abs() < 1e-10;
        let sweep = if full { tau } else { sweep };
        let steps = (sweep / std::f64::consts::FRAC_PI_2).ceil().clamp(1.0, 4.0) as usize;
        let step = sweep / steps as f64;
        let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w.abs() / 2.0, h.abs() / 2.0);
        let point = |a: f64| Point::new(cx + rx * a.cos(), cy - ry * a.sin());
        let mut p = BezPath::new();
        p.move_to(point(start));
        for i in 0..steps {
            let a = start + i as f64 * step;
            let b = a + step;
            let k = 4.0 / 3.0 * (step / 4.0).tan();
            let p0 = point(a);
            let p3 = point(b);
            p.curve_to((p0.x - k * rx * a.sin(), p0.y - k * ry * a.cos()), (p3.x + k * rx * b.sin(), p3.y + k * ry * b.cos()), (p3.x, p3.y));
        }
        if full {
            p.close_path();
        } else if pie {
            p.line_to((cx, cy));
            p.close_path();
        }
        Ok(p)
    }
    /// Decode bounded polycurve points and node flags into straight and cubic subpaths.
    fn curve(&mut self, bytes: &[u8], path: bool) -> Result<BezPath, String> {
        let mut r = Reader::new(bytes);
        let count = if path {
            r.skip(4)?;
            let n = r.u16()? as usize + r.u16()? as usize;
            r.skip(16)?;
            n
        } else {
            r.u32()? as usize
        };
        self.add_points(count)?;
        if count > bytes.len().saturating_sub(r.pos) / 9 {
            return Err("truncated CDR curve points".into());
        }
        let coordinate_bytes = count.checked_mul(8).ok_or("CDR curve coordinate size overflow")?;
        let mut points = Reader::new(r.take(coordinate_bytes)?);
        let types = r.take(count)?;
        self.add_anchors(curve_anchor_bound(types))?;
        let mut p = BezPath::new();
        let mut controls = Vec::new();
        let mut started = false;
        // Coordinates and flags occupy separate source arrays. Borrow both and
        // read them in lockstep instead of allocating a second full point vector.
        for typ in types {
            let point = Point::new(points.coord()?, points.coord()?);
            match typ >> 6 {
                0 => {
                    p.move_to(point);
                    controls.clear();
                    started = true;
                }
                1 if started => {
                    p.line_to(point);
                    controls.clear();
                    if typ & 8 != 0 {
                        p.close_path();
                    }
                }
                2 if started => {
                    if let (Some(a), Some(b)) = (controls.first(), controls.get(1)) {
                        p.curve_to(*a, *b, point);
                    } else {
                        return Err("CDR cubic curve is missing its control points".into());
                    }
                    controls.clear();
                    if typ & 8 != 0 {
                        p.close_path();
                    }
                }
                3 if started => {
                    if controls.len() >= 2 {
                        return Err("too many CDR cubic control points".into());
                    }
                    controls.push(point);
                }
                _ => return Err("CDR curve segment precedes its move point".into()),
            }
        }
        if !controls.is_empty() {
            return Err("CDR curve ends with unused control points".into());
        }
        Ok(p)
    }
}
