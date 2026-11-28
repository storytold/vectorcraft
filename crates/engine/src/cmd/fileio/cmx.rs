//! `.cmx` exchange files for every path that writes them (`craft_cdr::write_cmx`):
//! `document.export {format: cmx}` and Export As. The chosen artboard (default the first) is the
//! file's page; with Use Artboards each chosen artboard (default all) is a page of one file. Each
//! top-level layer with art over a page is a layer of that page, holding that art as
//! [`vectorcraft_metafile::painted_layers`] gives it: fills and strokes in their colors (CMYK, RGB,
//! gray, spot colors with their tints and Registration), type as glyph outlines, and gradients and
//! patterns as one color. What the file can't hold comes back in the warnings.

use craft_cdr::Limits;
use craft_cdr::model as cd;
use kurbo::PathEl;
use serde_json::Value;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Document, LineCap, LineJoin, StrokeLayer};
use vectorcraft_geom::{BezPath, FillRule, Point};
use vectorcraft_metafile::Painted;

use super::super::{Result, bad};
use super::{ArtboardPick, Encoded, FormatOption};

const C: &str = "document.export";

/// The options `document.formats` lists for CMX.
pub(super) const OPTIONS: &[FormatOption] = &[super::ARTBOARD, super::ARTBOARDS, super::RANGE, super::USE_ARTBOARDS];

const IMAGES: &str = "placed images are left out of CMX files";
const NONZERO: &str = "fills that use the nonzero fill rule are written even-odd: where a shape crosses itself, or parts of it that turn the same way overlap, they may leave holes";
const SPLIT: &str = "paths with more than 7,281 points were written as several curves: their fills where their parts overlap, and their strokes where the curves meet, may draw differently";
const PAST_PAGE: &str = "art that crosses the edge of a page is written whole, including the part past the edge";
const DASHES: &str = "dash and gap lengths were rounded to whole stroke widths, the unit CMX files store them in";
const CUT: &str = "filled shapes with a part of more than 7,281 points were written as open pieces of that part, so their fills draw differently";
const ZERO_TINTS: &str = "spot colors at a low tint were written in the color they print at that tint, so their ink's color may open changed";
const LONG_NAMES: &str = "layer and spot color names longer than 256 characters open again shortened";
const UNNAMED_SPOTS: &str = "spot colors without a name were written as process colors";
const UNKNOWN_LOOKS: &str = "spot colors whose color is not known were written as black at their tints";
const MERGED_LOOKS: &str = "spot colors that share a name but not a color were all written in one of those colors";
const LEFT_OUT: &str = "art the file can't hold was left out";
const DASH_OFFSETS: &str = "dash offsets are left out: dashes start at the start of each path";
const ALIGNED_DASHES: &str = "dashes aligned to corners and path ends are written as plain dashes";
const MITER_LIMITS: &str = "miter limits are left out: mitered corners open again with a miter limit of 10";

/// Encode `doc` as CMX: one file with a page for the chosen artboard (default the first), or with
/// `use_artboards` a page for each chosen artboard (default all), in the order they are chosen.
pub(super) fn encode(doc: &Document, f: &super::Format, p: &Value, use_artboards: Option<bool>) -> Result<Encoded> {
    let pick: ArtboardPick = super::encode::options(f, p)?;
    let n = doc.artboards.len();
    let boards = match use_artboards {
        Some(true) => pick.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect()),
        _ => vec![pick.one(n).map_err(|e| bad(C, e))?],
    };
    if boards.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    let rects = boards
        .iter()
        .map(|&b| doc.artboards.get(b).map(|a| a.rect).ok_or_else(|| bad(C, format!("no artboard {}", b + 1))))
        .collect::<Result<Vec<_>>>()?;
    vectorcraft_metafile::check_regions(&rects).map_err(|e| bad(C, e))?;
    let mut warnings = Vec::new();
    let mut note = |w: &str| {
        if !warnings.iter().any(|x: &String| x == w) {
            warnings.push(w.to_string());
        }
    };
    let mut pages = Vec::with_capacity(rects.len());
    for r in &rects {
        let art = vectorcraft_metafile::painted_layers(doc, *r).map_err(|e| bad(C, e))?;
        art.warnings.iter().for_each(|w| note(w));
        if art.images {
            note(IMAGES);
        }
        // The file has no clipping the reader reads, so art is not cut at the page's edge.
        let edge = r.abs().inflate(PAGE_EDGE_TOLERANCE, PAGE_EDGE_TOLERANCE);
        // A stroke reaches half its width past its path.
        let past = |bp: &BezPath, reach: f64| {
            let b = kurbo::Shape::bounding_box(bp).inflate(reach, reach);
            !bp.elements().is_empty() && (b.x0 < edge.x0 || b.y0 < edge.y0 || b.x1 > edge.x1 || b.y1 > edge.y1)
        };
        if art.layers.iter().flat_map(|(_, p)| p).any(|op| match op {
            Painted::Fill { path, .. } => past(path, 0.0),
            Painted::Stroke { path, width, .. } => past(path, width.max(0.0) / 2.0),
        }) {
            note(PAST_PAGE);
        }
        let page = Page { center: r.abs().center() };
        // The model lists layers and objects front to back; the document paints them back to front.
        // A layer with nothing drawn on the page is left out, as the reader leaves out a layer that
        // holds nothing.
        let layers = art
            .layers
            .iter()
            .rev()
            .map(|(name, painted)| (name, page.objects(&art.swatches, painted, &mut note)))
            .filter(|(_, objects)| !objects.is_empty())
            .map(|(name, objects)| cd::Layer {
                name: name.clone(),
                kind: cd::LayerKind::Drawing,
                flags: cd::Flags::default(),
                objects: objects.into_iter().rev().collect(),
                guides: Vec::new(),
            })
            .collect();
        pages.push(cd::Page { size: Some((r.width().abs(), r.height().abs())), layers });
    }
    let drawing = cd::Drawing { version: 0, page_size: None, pages, master: Vec::new(), guides: Vec::new(), warnings: Vec::new() };
    // The limits `document.open` reads `.cmx` files with, so the file opens whole again.
    let written = craft_cdr::write_cmx(&drawing, Limits::default()).map_err(|e| bad(C, e.to_string()))?;
    for (set, w) in [
        (written.nonzero_fills, NONZERO),
        (written.split_paths, SPLIT),
        (written.cut_subpaths, CUT),
        (written.long_names, LONG_NAMES),
        (written.zero_tints, ZERO_TINTS),
        (written.rounded_dashes, DASHES),
        (written.unnamed_spots, UNNAMED_SPOTS),
        (written.unknown_looks, UNKNOWN_LOOKS),
        (written.merged_looks, MERGED_LOOKS),
        (written.left_out, LEFT_OUT),
    ] {
        if set {
            note(w);
        }
    }
    let mut enc = Encoded::one(written.bytes);
    enc.warnings = warnings;
    enc.warnings.extend(crate::cmd::fonts::export_overflow_warning(doc, &doc.layers, Some(&rects)));
    Ok(enc)
}

/// A page's placement: document space (y down) to the page's coordinates (from its center, y up).
struct Page {
    center: Point,
}

impl Page {
    fn pt(&self, p: Point) -> cd::Pt {
        cd::Pt { x: p.x - self.center.x, y: self.center.y - p.y }
    }

    /// `bp` in the page's coordinates. A quadratic segment becomes the cubic that draws it.
    fn path(&self, bp: &BezPath) -> cd::Path {
        let mut subpaths: Vec<cd::SubPath> = Vec::new();
        // The current point and the start of the current subpath, in document space.
        let (mut last, mut start) = (Point::ZERO, Point::ZERO);
        for el in bp.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    subpaths.push(cd::SubPath { start: self.pt(p), segments: Vec::new(), closed: false });
                    (last, start) = (p, p);
                }
                PathEl::LineTo(p) => {
                    self.segment(&mut subpaths, last, cd::Segment::Line(self.pt(p)));
                    last = p;
                }
                PathEl::QuadTo(c, p) => {
                    let (c1, c2) = (last + (c - last) * (2.0 / 3.0), p + (c - p) * (2.0 / 3.0));
                    self.segment(&mut subpaths, last, cd::Segment::Cubic(self.pt(c1), self.pt(c2), self.pt(p)));
                    last = p;
                }
                PathEl::CurveTo(c1, c2, p) => {
                    self.segment(&mut subpaths, last, cd::Segment::Cubic(self.pt(c1), self.pt(c2), self.pt(p)));
                    last = p;
                }
                PathEl::ClosePath => {
                    if let Some(s) = subpaths.last_mut() {
                        s.closed = true;
                    }
                    last = start;
                }
            }
        }
        // A subpath of a lone point draws nothing.
        subpaths.retain(|s| !s.segments.is_empty());
        cd::Path { subpaths }
    }

    /// Add `seg` to the open subpath, or to a new one from `from` after a close.
    fn segment(&self, subpaths: &mut Vec<cd::SubPath>, from: Point, seg: cd::Segment) {
        match subpaths.last_mut() {
            Some(s) if !s.closed => s.segments.push(seg),
            _ => subpaths.push(cd::SubPath { start: self.pt(from), segments: vec![seg], closed: false }),
        }
    }

    /// The fills and strokes of one layer as shapes, in paint order. A fill followed by a stroke of
    /// the same path is one shape with both. A path with no segments draws nothing and is left out.
    fn objects(&self, swatches: &[Swatch], painted: &[Painted], note: &mut impl FnMut(&str)) -> Vec<cd::Object> {
        let mut out = Vec::with_capacity(painted.len());
        let mut i = 0;
        while let Some(op) = painted.get(i) {
            i += 1;
            let shape = match op {
                Painted::Fill { path, rule, rgb, paint } => {
                    let outline = match painted.get(i) {
                        Some(s @ Painted::Stroke { path: stroked, .. }) if stroked == path => {
                            i += 1;
                            outline(swatches, s, note)
                        }
                        _ => None,
                    };
                    let fill = color(swatches, *rgb, paint.as_ref());
                    let mut shape = cd::Shape::new(cd::Geometry::Path(self.path(path)), cd::Matrix::IDENTITY, Some(fill), outline);
                    shape.nonzero = *rule == FillRule::NonZero;
                    shape
                }
                Painted::Stroke { path, .. } => {
                    cd::Shape::new(cd::Geometry::Path(self.path(path)), cd::Matrix::IDENTITY, None, outline(swatches, op, note))
                }
            };
            if matches!(&shape.geometry, cd::Geometry::Path(p) if p.subpaths.is_empty()) {
                continue;
            }
            out.push(cd::Object::Shape(Box::new(shape)));
        }
        out
    }
}

/// How far (points) a path may reach past its page before the export says art crosses the edge.
const PAGE_EDGE_TOLERANCE: f64 = 0.01;

/// The outline of a stroke: its width, color, cap, join and dashes in stroke widths. The file holds
/// no miter limit, so a mitered stroke with a limit other than the one a stroke opens with is
/// named in the warnings.
fn outline(swatches: &[Swatch], p: &Painted, note: &mut impl FnMut(&str)) -> Option<cd::Outline> {
    let Painted::Stroke { rgb, paint, width, cap, join, miter, dash, .. } = p else { return None };
    if *join == LineJoin::Miter && *miter != StrokeLayer::new(Paint::None, *width).miter_limit {
        note(MITER_LIMITS);
    }
    let dashes = match dash {
        Some(d) if *width > 0.0 => {
            if d.offset != 0.0 {
                note(DASH_OFFSETS);
            }
            if d.align_corners {
                note(ALIGNED_DASHES);
            }
            d.pattern.iter().map(|x| x / width).collect()
        }
        _ => Vec::new(),
    };
    Some(cd::Outline {
        width: *width,
        color: color(swatches, *rgb, paint.as_ref()),
        cap: match cap {
            LineCap::Butt => cd::Cap::Butt,
            LineCap::Round => cd::Cap::Round,
            LineCap::Square => cd::Cap::Square,
        },
        join: match join {
            LineJoin::Miter => cd::Join::Miter,
            LineJoin::Round => cd::Join::Round,
            LineJoin::Bevel => cd::Join::Bevel,
        },
        dashes,
        behind_fill: false,
        scale_with: false,
    })
}

/// The color a fill or stroke is written in: Registration, a spot color at its tint with the look
/// of its swatch among `swatches`, or the solid color in its own model (Lab as RGB); `rgb` for the
/// one color of a gradient or pattern.
fn color(swatches: &[Swatch], rgb: [u8; 3], paint: Option<&Paint>) -> cd::Color {
    let from_rgb = |[r, g, b]: [u8; 3]| cd::Color::Rgb([r, g, b].map(|v| f32::from(v) / 255.0));
    let Some(Paint::Solid { color, swatch, tint }) = paint else { return from_rgb(rgb) };
    if swatch.as_deref() == Some(REGISTRATION) {
        return cd::Color::Registration;
    }
    if let Some(s) = swatch.as_deref().and_then(|name| swatches.iter().find(|s| s.name == name)).filter(|s| s.spot) {
        let look = s.paint.color().map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b].map(|v| f32::from(v) / 255.0)
        });
        return cd::Color::Spot { name: s.name.clone(), tint: tint.clamp(0.0, 1.0), look };
    }
    match *color {
        Color::Rgb { r, g, b } => cd::Color::Rgb([r, g, b]),
        Color::Cmyk { c, m, y, k } => cd::Color::Cmyk([c, m, y, k]),
        Color::Gray { k } => cd::Color::Gray(k),
        Color::Lab { .. } => from_rgb(rgb),
    }
}
