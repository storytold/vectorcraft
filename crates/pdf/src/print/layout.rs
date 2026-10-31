//! Where each printed page's art goes: the artboards (or all the art) that print, the paper's
//! orientation, the scale, the placement on the imageable area or the tiles, and the turns and
//! mirroring of the page (flipped orientation, emulsion down, transverse).
//!
//! The art is placed in a drawing space: the paper (y down from its top-left corner), or, when
//! tiling, the whole printed area (the artboard grown by its bleed and marks) from its top-left
//! corner, of which each page shows one tile.

use kurbo::{Affine, Rect};
use serde::Serialize;
use vectorcraft_doc::marks::outset;
use vectorcraft_doc::range::parse_range;
use vectorcraft_doc::{Document, Node, NodeKind, SymbolBoxes};

use super::{Emulsion, Orientation, PrintArtboards, PrintScaling, PrintSettings};
use crate::PdfError;

/// Most tiles of one artboard.
pub const MAX_TILES: usize = 1000;

/// One page of the job, before copies and inks.
#[derive(Clone, Debug)]
pub(crate) struct Layout {
    /// The artboard (`None`: all the art, artboards ignored).
    pub artboard: Option<usize>,
    /// The tile (0-based) of how many.
    pub tile: Option<(usize, usize)>,
    /// The page size in points (turned when transverse).
    pub size: (f64, f64),
    /// How the paper is turned.
    pub orientation: Orientation,
    /// Horizontal and vertical scale (1 = 100%).
    pub scale: (f64, f64),
    /// Drawing space → page space.
    pub view: Affine,
    /// The tile, in drawing space.
    pub window: Option<Rect>,
    /// Document space → drawing space.
    pub place: Affine,
    /// The art that prints on this page, in document space (within the bleed box).
    pub area: Rect,
    /// The artboard in drawing space, and its bleed there (`[top, bottom, left, right]`): where
    /// the marks go.
    pub trim: Rect,
    pub bleed: [f64; 4],
    /// The TrimBox and BleedBox in page space.
    pub page_trim: Rect,
    pub page_bleed: Rect,
}

/// The tiles of one artboard (`print.preview`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TileGrid {
    /// 0-based artboard (`None`: artboards ignored).
    pub artboard: Option<usize>,
    pub columns: usize,
    pub rows: usize,
    /// The 1-based tiles that print.
    pub printed: Vec<usize>,
    /// Every tile as `[x0, y0, x1, y1]` in document space, across then down.
    pub tiles: Vec<[f64; 4]>,
}

/// Calls `f` with the bounds of each object of `n` that prints: visible, not on a template
/// layer and not a guide (layers are already picked by the Print Layers option). Symbol
/// instances are measured by their art from `symbols`.
fn printed(n: &Node, symbols: &SymbolBoxes, f: &mut impl FnMut(Rect)) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Layer { template: true, .. } | NodeKind::Path { guide: true, .. } => {}
        NodeKind::Layer { children, .. } | NodeKind::Group { children, clip: false } => {
            for c in children {
                printed(c, symbols, f);
            }
        }
        _ => {
            if let Some(b) = n.visual_bounds_with(symbols) {
                f(b);
            }
        }
    }
}

/// Is there art that prints in `r`? The box of a symbol instance whose art holds brushed strokes
/// grows by as far as their brush art can paint ([`SymbolBoxes::with_brush_art`]).
fn has_art(doc: &Document, r: Rect) -> bool {
    let symbols = doc.symbol_boxes().with_brush_art();
    let mut any = false;
    for l in &doc.layers {
        printed(l, &symbols, &mut |b| any |= b.intersect(r).area() > 0.0 || r.contains_rect(b));
    }
    any
}

/// The bounds of all the art that prints.
fn art_bounds(doc: &Document) -> Option<Rect> {
    let symbols = doc.symbol_boxes();
    let mut all: Option<Rect> = None;
    for l in &doc.layers {
        printed(l, &symbols, &mut |b| all = Some(all.map_or(b, |a| a.union(b))));
    }
    all
}

/// The regions that print: `(artboard, rect)`.
pub(crate) fn regions(doc: &Document, set: &PrintSettings) -> Result<Vec<(Option<usize>, Rect)>, PdfError> {
    let count = doc.artboards.len();
    let picked: Vec<usize> = match set.artboards {
        PrintArtboards::Ignore => {
            let r = art_bounds(doc).ok_or_else(|| PdfError::BadSetting("nothing to print: the document has no art that prints".into()))?;
            return Ok(vec![(None, r)]);
        }
        _ if count == 0 => return Err(PdfError::NoArtboards),
        PrintArtboards::All => (0..count).collect(),
        PrintArtboards::Range => parse_range(&set.range, count).map_err(PdfError::BadSetting)?,
    };
    let mut out: Vec<(Option<usize>, Rect)> = picked.into_iter().filter_map(|i| Some((Some(i), doc.artboards.get(i)?.rect))).collect();
    if set.skip_blank {
        out.retain(|(_, r)| has_art(doc, *r));
        if out.is_empty() {
            return Err(PdfError::BadSetting("nothing to print: every artboard picked is blank".into()));
        }
    }
    Ok(out)
}

/// `[top, bottom, left, right]` sides scaled by `(sx, sy)`.
fn scaled(sides: [f64; 4], (sx, sy): (f64, f64)) -> [f64; 4] {
    [sides[0] * sy, sides[1] * sy, sides[2] * sx, sides[3] * sx]
}

/// The tiles along one axis of a printed area `extent` long, `tile` long each and `step` apart:
/// the first (tile `k` starts at `start + k × step`) and how many. Tiles showing no more of the area
/// than the `overlap` are left out. Without a `start`, the first tile starts at the area's edge.
fn span(start: Option<f64>, extent: f64, tile: f64, step: f64, overlap: f64) -> (f64, f64) {
    // Tiles that end (or start) exactly on an edge show none of the area beyond it.
    const EPS: f64 = 1e-6;
    match start {
        None => (0.0, ((extent - overlap) / step).ceil().max(1.0)),
        Some(g) => {
            let first = ((overlap - tile - g) / step + EPS).floor() + 1.0;
            let last = ((extent - overlap - g) / step - EPS).ceil() - 1.0;
            (first, (last - first + 1.0).max(1.0))
        }
    }
}

/// The page turns of a `w` × `h` paper (flipped, emulsion down, transverse) as one transform from
/// the paper onto the page, and the page's size.
fn turns(set: &PrintSettings, (w, h): (f64, f64), flipped: bool) -> (Affine, (f64, f64)) {
    let mut v = Affine::IDENTITY;
    if flipped {
        v = Affine::new([-1.0, 0.0, 0.0, -1.0, w, h]) * v;
    }
    if set.output.emulsion == Emulsion::Down {
        v = Affine::new([-1.0, 0.0, 0.0, 1.0, w, 0.0]) * v;
    }
    if set.transverse {
        // A quarter turn clockwise: the page is the paper on its side.
        return (Affine::new([0.0, 1.0, -1.0, 0.0, h, 0.0]) * v, (h, w));
    }
    (v, (w, h))
}

/// The pages of `doc` (whose layers are those that print) as `set` lays them out, the tiles of
/// each artboard when tiling, and warnings (art that doesn't fit the imageable area).
pub(crate) fn layout(doc: &Document, set: &PrintSettings, warnings: &mut Vec<String>) -> Result<(Vec<Layout>, Vec<TileGrid>), PdfError> {
    let marks = set.marks.printer_marks();
    let bleed = set.bleed.of(doc);
    let (pw, ph) = set.paper();
    let origin = set.tile_origin.placed.then_some((set.tile_origin.x, set.tile_origin.y));
    let (mut pages, mut grids) = (vec![], vec![]);
    for (artboard, r) in regions(doc, set)? {
        let what = artboard.map_or_else(|| "The art".to_string(), |i| format!("Artboard {}", i + 1));
        if !(r.width() > 0.0 && r.height() > 0.0) {
            warnings.push(format!("{what} has no area and was left out"));
            continue;
        }
        let orientation = match set.auto_rotate {
            true if (r.width() > r.height()) != (pw > ph) => Orientation::Landscape,
            true => Orientation::Portrait,
            false => set.orientation,
        };
        let paper = if orientation.landscape() { (ph, pw) } else { (pw, ph) };
        let m = set.margin;
        let imageable = Rect::new(m, m, paper.0 - m, paper.1 - m);
        if imageable.width() < 1.0 || imageable.height() < 1.0 {
            return Err(PdfError::BadSetting(format!("margin: a {m} pt margin leaves no imageable area on the paper")));
        }
        // How far the bleed and marks reach around the artboard at scale `s`, `[top, bottom, left, right]`.
        let around = |s: (f64, f64)| {
            let b = scaled(bleed, s);
            let reach = marks.reach(b);
            (b, std::array::from_fn::<f64, 4, _>(|i| b[i].max(reach[i])))
        };
        let scale = match set.scaling {
            PrintScaling::None => (1.0, 1.0),
            PrintScaling::Fit => {
                // The largest scale whose artboard with its bleed and marks fits: the marks reach
                // further when a larger scale grows the bleed.
                let fit = |s: f64| {
                    let (_, most) = around((s, s));
                    ((imageable.width() - most[2] - most[3]) / r.width()).min((imageable.height() - most[0] - most[1]) / r.height())
                };
                let first = fit(1.0);
                let s = if first <= 1.0 { first } else { fit(first) };
                if !(s.is_finite() && s > 0.0) {
                    return Err(PdfError::BadSetting(format!("{what} doesn't fit the paper with its bleed and marks")));
                }
                (s, s)
            }
            _ => (set.scale.width / 100.0, set.scale.height / 100.0),
        };
        let (bleed_s, most) = around(scale);
        let size = (r.width() * scale.0, r.height() * scale.1);
        let extent = (most[2] + size.0 + most[3], most[0] + size.1 + most[1]);
        let flipped = !set.auto_rotate && orientation.flipped();
        let (turn, page_size) = turns(set, paper, flipped);
        let page = Rect::new(0.0, 0.0, page_size.0, page_size.1);
        let bleed_box = outset(r, bleed);
        let place_at = |trim: Rect| {
            Affine::translate(trim.origin().to_vec2()) * Affine::scale_non_uniform(scale.0, scale.1) * Affine::translate(-r.origin().to_vec2())
        };
        // The page's boxes: the trim box within the bleed box within the page.
        let boxes = |view: Affine, trim: Rect| {
            let b = view.transform_rect_bbox(outset(trim, bleed_s)).intersect(page);
            let t = view.transform_rect_bbox(trim).intersect(b);
            if t.area() > 0.0 { (t, b) } else { (page, page) }
        };
        let base = |view, window, place, area, trim, (page_trim, page_bleed), tile| Layout {
            artboard,
            tile,
            size: page_size,
            orientation,
            scale,
            view,
            window,
            place,
            area,
            trim,
            bleed: bleed_s,
            page_trim,
            page_bleed,
        };
        if !set.scaling.tiles() {
            // The printed area's top-left corner: where the tile origin puts it, else the placement.
            let (x, y) = match origin {
                Some((ox, oy)) => (imageable.x0 - ox * scale.0 - most[2], imageable.y0 - oy * scale.1 - most[0]),
                None => {
                    let (fx, fy) = set.placement.origin.factors();
                    (
                        imageable.x0 + (imageable.width() - extent.0) * fx + set.placement.x,
                        imageable.y0 + (imageable.height() - extent.1) * fy + set.placement.y,
                    )
                }
            };
            let trim = Rect::new(x + most[2], y + most[0], x + most[2] + size.0, y + most[0] + size.1);
            if !imageable.inflate(0.01, 0.01).contains_rect(Rect::new(x, y, x + extent.0, y + extent.1)) {
                warnings.push(format!("{what} with its bleed and marks is larger than the imageable area: part of it doesn't print"));
            }
            pages.push(base(turn, None, place_at(trim), bleed_box, trim, boxes(turn, trim), None));
            continue;
        }
        // Tiles across then down over the printed area, overlapping by `overlap`.
        let tile = if set.scaling == PrintScaling::TileFull { paper } else { (imageable.width(), imageable.height()) };
        let o = set.overlap;
        if o * 2.0 >= tile.0.min(tile.1) {
            return Err(PdfError::BadSetting(format!("overlap: {o} pt is half a tile or more")));
        }
        let step = (tile.0 - o, tile.1 - o);
        // The tile origin's page starts the grid: its imageable area's corner is the origin's point
        // (a full page's imageable area is inside the margin).
        let inside = if set.scaling == PrintScaling::TileFull { m } else { 0.0 };
        let start = origin.map(|(ox, oy)| (most[2] + ox * scale.0 - inside, most[0] + oy * scale.1 - inside));
        let (first_x, cols) = span(start.map(|s| s.0), extent.0, tile.0, step.0, o);
        let (first_y, rows) = span(start.map(|s| s.1), extent.1, tile.1, step.1, o);
        let grid = (start.map_or(0.0, |s| s.0) + first_x * step.0, start.map_or(0.0, |s| s.1) + first_y * step.1);
        if cols * rows > MAX_TILES as f64 {
            return Err(PdfError::BadSetting(format!("{what} would print on more than {MAX_TILES} tiles: scale it down")));
        }
        let (cols, rows) = (cols as usize, rows as usize);
        let count = cols * rows;
        let pick = if set.tile_range.trim().is_empty() {
            (0..count).collect()
        } else {
            parse_range(&set.tile_range, count).map_err(|e| PdfError::BadSetting(format!("tileRange of {what}: {e}")))?
        };
        let trim = Rect::new(most[2], most[0], most[2] + size.0, most[0] + size.1);
        let place = place_at(trim);
        let from_doc = place.inverse();
        let at = if set.scaling == PrintScaling::TileFull { (0.0, 0.0) } else { (imageable.x0, imageable.y0) };
        let window = |t: usize| {
            let (c, rw) = ((t % cols) as f64, (t / cols) as f64);
            let (x, y) = (grid.0 + c * step.0, grid.1 + rw * step.1);
            Rect::new(x, y, x + tile.0, y + tile.1)
        };
        for &t in &pick {
            let w = window(t);
            let view = turn * Affine::translate((at.0 - w.x0, at.1 - w.y0));
            let area = from_doc.transform_rect_bbox(w).intersect(bleed_box);
            pages.push(base(view, Some(w), place, area, trim, boxes(view, trim), Some((t, count))));
        }
        let tiles = (0..count).map(|t| from_doc.transform_rect_bbox(window(t))).map(|b| [b.x0, b.y0, b.x1, b.y1]).collect();
        grids.push(TileGrid { artboard, columns: cols, rows, printed: pick.iter().map(|t| t + 1).collect(), tiles });
    }
    if pages.is_empty() {
        return Err(PdfError::BadSetting("nothing to print".into()));
    }
    Ok((pages, grids))
}
