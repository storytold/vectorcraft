//! Brushed strokes and symbol-instance staining in the renderer.
//!
//! A stroke with `brush: Some(name)` is painted as the filled art `vectorcraft_brush::stroke_pieces`
//! generates. Pieces are cached per (node id, stroke item, path) and validated by the path, the
//! stroke layer and the brush definition (an `Arc` from the per-frame parsed library), so edits to
//! any of them regenerate the art and nothing else does. The copies of an object that share its id
//! (symbol instances, inline graphics, repeat copies) have their own paths, so each keeps its own
//! pieces. The pieces keep their `Arc`s while they are cached, so their stroke outlines, live-effect
//! geometry and the evaluations of live objects come from the renderer's caches too. Brush art
//! leaves out the symbol instances and pattern paints whose art draws a brushed stroke, so when
//! the art of a symbol or pattern changes, the pieces of brushes and strokes that can show one are
//! made again.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_brush::{Brush, BrushKind};
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind, StrokeLayer, SymbolBoxes};
use vectorcraft_geom::{BezPath, PathEl, Point, Rect};
use vello_cpu::RenderContext;

use crate::lru::{self, FrameCache};
use crate::{Frame, Renderer};

pub(crate) struct Entry {
    path: BezPath,
    stroke: StrokeLayer,
    brush: Arc<Brush>,
    pieces: Arc<Vec<Arc<Node>>>,
    /// Can the pieces show symbol or pattern art ([`shows_definitions`])?
    definitions: bool,
}

/// Can the brush art of `brush` along a stroke `st` show symbol or pattern art: does the stroke
/// paint with a pattern, or does the brush's own art show a symbol or a pattern?
fn shows_definitions(brush: &Brush, st: &StrokeLayer) -> bool {
    let art: Vec<&Node> = match &brush.kind {
        BrushKind::Art(a) => vec![&a.art],
        BrushKind::Scatter(s) => vec![&s.art],
        BrushKind::Pattern(p) => {
            [Some(&p.side), p.outer_corner.as_ref(), p.inner_corner.as_ref(), p.start.as_ref(), p.end.as_ref()].into_iter().flatten().collect()
        }
        BrushKind::Calligraphic(_) | BrushKind::Bristle(_) => vec![],
    };
    matches!(st.paint, vectorcraft_color::Paint::Pattern { .. }) || crate::fx::Reads::of_all(art.into_iter()).shows_definitions()
}

/// The names and art of `doc`'s patterns and symbols, in order.
fn definitions(doc: &Document) -> impl Iterator<Item = (&str, &Arc<Node>)> {
    doc.patterns.iter().flat_map(|p| p.art.iter().map(|a| (p.name.as_str(), a))).chain(doc.symbols.iter().map(|s| (s.name.as_str(), &s.art)))
}

/// Parsed brush library and generated brush art.
pub(crate) struct BrushCache {
    /// The document value the library was parsed from (`None` = defaults).
    src: Option<Option<vectorcraft_brush::serde_json::Value>>,
    /// A hash of that value (0 for the defaults), which rasters drawing brushed strokes record
    /// ([`crate::fx::Inputs`]).
    key: u64,
    /// (frame stamp, document address) the library was last validated for.
    checked: (u64, usize),
    lib: HashMap<String, Arc<Brush>>,
    /// The names and art of the document's patterns and symbols ([`definitions`]) when the library
    /// was last validated, the art held weakly as the pattern tiles hold theirs.
    definitions: Vec<(String, Weak<Node>)>,
    /// Brush art per (node id, stroke item index, [`path_key`] of the stroked path).
    pub(crate) entries: FrameCache<(u64, usize, u64), Entry>,
    /// How many times [`Renderer::draw_brush`] made brush art.
    #[cfg(test)]
    pub(crate) made: usize,
}

impl Default for BrushCache {
    fn default() -> Self {
        Self {
            src: None,
            key: 0,
            checked: (0, 0),
            lib: HashMap::new(),
            definitions: Vec::new(),
            entries: FrameCache::new(lru::BRUSH_BUDGET, 512),
            #[cfg(test)]
            made: 0,
        }
    }
}

impl BrushCache {
    fn library(&mut self, doc: &Document, stamp: u64) {
        let key = (stamp, doc as *const Document as usize);
        if self.checked == key {
            return;
        }
        self.checked = key;
        if !self.definitions.iter().map(|(n, a)| (n.as_str(), a.as_ptr())).eq(definitions(doc).map(|(n, a)| (n, Arc::as_ptr(a)))) {
            self.definitions = definitions(doc).map(|(n, a)| (n.to_string(), Arc::downgrade(a))).collect();
            self.entries.retain(|e| !e.definitions);
        }
        let cur = doc.unknown.get(vectorcraft_brush::DOC_KEY);
        if self.src.as_ref().is_some_and(|s| s.as_ref() == cur) {
            return;
        }
        self.src = Some(cur.cloned());
        self.key = cur.map_or(0, |v| {
            let mut h = DefaultHasher::new();
            vectorcraft_brush::serde_json::to_string(v).unwrap_or_default().hash(&mut h);
            h.finish()
        });
        self.lib = vectorcraft_brush::library(doc).into_iter().map(|b| (b.name.clone(), Arc::new(b))).collect();
    }

    /// [`Self::key`] for the brush library `doc` holds (checked once a frame, `stamp`).
    pub(crate) fn library_key(&mut self, doc: &Document, stamp: u64) -> u64 {
        self.library(doc, stamp);
        self.key
    }
}

/// Cull bounds that allow for brush art reaching past the stroke (scatter, wide nibs), symbol
/// instances measured by their art from `symbols`, with the brushed strokes in that art.
pub(crate) fn cull_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = crate::fx::cull_bounds(symbols, n)?;
    let pad = pad(symbols, n);
    Some(b.inflate(pad, pad))
}

/// How far brush art can paint past `n`'s brushed strokes, or past the brushed strokes in the art
/// of symbol instance `n`: `16 × w + 8`, where `w` is the widest stroke (at least 1) of `n` when it
/// has a visible brushed stroke and of the objects in that art that have one
/// ([`SymbolBoxes::brush_width`]), and 0 when neither has one.
pub(crate) fn pad(symbols: &SymbolBoxes, n: &Node) -> f64 {
    let own = vectorcraft_brush::has_brush(n)
        .then(|| n.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.width) } else { None }).fold(1.0, f64::max));
    let art = match &n.kind {
        NodeKind::SymbolInstance { symbol, .. } => symbols.brush_width(symbol).map(|w| w.max(1.0)),
        _ => None,
    };
    own.into_iter().chain(art).reduce(f64::max).map_or(0.0, |w| 16.0 * w + 8.0)
}

pub use vectorcraft_brush::instance_art;

/// A hash of the kinds and points of `bp`'s elements. With the node id and the stroke item, it
/// forms the key of the brush art made along `bp`.
fn path_key(bp: &BezPath) -> u64 {
    let mut h = DefaultHasher::new();
    for el in bp.elements() {
        let (kind, points) = match *el {
            PathEl::MoveTo(p) => (0u8, [p, Point::ZERO, Point::ZERO]),
            PathEl::LineTo(p) => (1, [p, Point::ZERO, Point::ZERO]),
            PathEl::QuadTo(a, b) => (2, [a, b, Point::ZERO]),
            PathEl::CurveTo(a, b, c) => (3, [a, b, c]),
            PathEl::ClosePath => (4, [Point::ZERO; 3]),
        };
        kind.hash(&mut h);
        for p in points {
            (p.x.to_bits(), p.y.to_bits()).hash(&mut h);
        }
    }
    h.finish()
}

impl Renderer {
    /// Paint stroke `st` of `n` (outline `bp`) with its brush. Returns false when the brush is
    /// unknown (the caller then draws a plain stroke).
    pub(crate) fn draw_brush(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, st: &StrokeLayer) -> bool {
        let Some(name) = st.brush.as_deref() else { return false };
        self.brushes.library(f.doc, self.stamp);
        let Some(brush) = self.brushes.lib.get(name).cloned() else { return false };
        let idx = n.appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st))).unwrap_or(usize::MAX);
        let key = (n.id.0, idx, path_key(bp));
        let stamp = self.stamp;
        let (pieces, kept) = match self.brushes.entries.get(&key, stamp) {
            Some(e) if Arc::ptr_eq(&e.brush, &brush) && e.stroke == *st && e.path == *bp => (e.pieces.clone(), true),
            _ => {
                #[cfg(test)]
                {
                    self.brushes.made += 1;
                }
                let pieces: Arc<Vec<Arc<Node>>> =
                    Arc::new(vectorcraft_brush::stroke_pieces(f.doc, &brush, bp, st).into_iter().map(Arc::new).collect());
                let bytes = size_of::<Entry>() + lru::path_bytes(bp) + pieces.iter().map(|p| lru::node_bytes(p)).sum::<usize>();
                let definitions = shows_definitions(&brush, st);
                let entry = Entry { path: bp.clone(), stroke: st.clone(), brush, pieces: pieces.clone(), definitions };
                let kept = self.brushes.entries.insert(key, entry, bytes, stamp);
                (pieces, kept)
            }
        };
        let mut draw = |r: &mut Self, c: &mut RenderContext, fr: &Frame| r.draw_brush_pieces(c, fr, &pieces, kept);
        if st.opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal {
            self.group(ctx, f, crate::group::Composite { blend: st.blend, opacity: st.opacity, ..Default::default() }, &mut draw);
        } else {
            draw(self, ctx, f);
        }
        true
    }

    /// Draw brush art `pieces`, each whatever its visibility and bounds. A piece that is a path
    /// without effects is drawn as `cur`. When the brush cache keeps the pieces (`kept`), their
    /// `Arc`s stay the same from frame to frame: such a path piece takes its stroke outlines from the
    /// stroke cache as on the fast path ([`Self::cached_stroke`]), and each piece is also drawn as
    /// `fx_owner` through [`Self::draw_kept_node`], so its live-effect geometry and, when it is a
    /// live object, its evaluation come from the caches. Pieces the cache does not keep are drawn as
    /// art made again ([`Self::made_again`]).
    fn draw_brush_pieces(&mut self, ctx: &mut RenderContext, f: &Frame, pieces: &[Arc<Node>], kept: bool) {
        let outer = (std::mem::replace(&mut self.made_again, !kept), self.cur.take());
        for p in pieces {
            self.cur = (matches!(p.kind, NodeKind::Path { guide: false, .. }) && !crate::fx::has_fx(p)).then(|| p.clone());
            if kept {
                let owner = self.fx_owner.replace(p.clone());
                self.draw_kept_node(ctx, f, p);
                self.fx_owner = owner;
            } else {
                self.draw_node(ctx, f, p, true);
            }
        }
        (self.made_again, self.cur) = outer;
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::{Affine, Rect, shapes};

    use crate::{RenderOptions, Rendered, Renderer};

    fn doc_with_line(brush: Option<&str>, width: f64) -> Document {
        let mut d = Document::new(200.0, 100.0);
        let id = d.alloc_id();
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), width);
        ap.stroke_mut().unwrap().brush = brush.map(str::to_string);
        let n = Node::path(id, shapes::line((20.0, 50.0).into(), (180.0, 50.0).into()), ap);
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    }

    fn render(r: &mut Renderer, d: &Document) -> Rendered {
        r.render(d, 200, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    }

    fn dark(p: [u8; 4]) -> bool {
        (p[0] as u32 + p[1] as u32 + p[2] as u32) < 200
    }

    #[test]
    fn calligraphic_brush_paints_wider_than_the_stroke() {
        let mut r = Renderer::new();
        let plain = render(&mut r, &doc_with_line(None, 1.0));
        assert!(!dark(plain.pixel(100, 53)));
        let d = doc_with_line(Some("10 pt. Oval"), 1.0);
        let img = render(&mut r, &d);
        assert!(dark(img.pixel(100, 50)) && dark(img.pixel(100, 52)), "brush art is painted: {:?}", img.pixel(100, 52));
        assert!(!dark(img.pixel(100, 60)));
        // Second frame hits the cache and paints the same.
        let again = render(&mut r, &d);
        assert_eq!(again.pixels, img.pixels);
    }

    /// Object opacity fades a brushed stroke's art as a whole, with or without a dash and a fill:
    /// the same as the object at full opacity inside a group at that opacity.
    #[test]
    fn object_opacity_fades_a_brushed_stroke_once() {
        for (dashed, fill) in [(false, false), (true, false), (false, true), (true, true)] {
            let node = |d: &mut Document, opacity: f32| {
                let id = d.alloc_id();
                let fill = if fill { Paint::solid(Color::rgb(0.9, 0.1, 0.1)) } else { Paint::None };
                let mut ap = Appearance::basic(fill, Paint::solid(Color::BLACK), 1.0);
                let st = ap.stroke_mut().unwrap();
                st.brush = Some("10 pt. Oval".into());
                if dashed {
                    st.dash = Some(vectorcraft_doc::Dash { pattern: vec![12.0, 4.0], ..Default::default() });
                }
                let mut n = Node::path(id, shapes::rectangle(Rect::new(30.0, 30.0, 170.0, 70.0)), ap);
                n.opacity = opacity;
                n
            };
            let mut faded = Document::new(200.0, 100.0);
            let n = node(&mut faded, 0.5);
            let l = faded.layers[0].id;
            faded.insert(Some(l), 0, n).unwrap();
            let mut grouped = Document::new(200.0, 100.0);
            let n = node(&mut grouped, 1.0);
            let mut g = Node::group(grouped.alloc_id(), vec![std::sync::Arc::new(n)]);
            g.opacity = 0.5;
            let l = grouped.layers[0].id;
            grouped.insert(Some(l), 0, g).unwrap();
            let (a, b) = (render(&mut Renderer::new(), &faded), render(&mut Renderer::new(), &grouped));
            assert!(
                dark(
                    render(&mut Renderer::new(), &{
                        let mut d = Document::new(200.0, 100.0);
                        let n = node(&mut d, 1.0);
                        let l = d.layers[0].id;
                        d.insert(Some(l), 0, n).unwrap();
                        d
                    })
                    .pixel(30, 50)
                ),
                "the brush paints the edge"
            );
            let off =
                a.pixels.chunks(4).zip(b.pixels.chunks(4)).filter(|(x, y)| x.iter().zip(*y).any(|(p, q)| (*p as i32 - *q as i32).abs() > 2)).count();
            assert_eq!(off, 0, "dashed {dashed}, fill {fill}: {off} pixels differ from the object faded as a whole");
        }
    }

    #[test]
    fn unknown_brush_falls_back_to_plain_stroke_and_scatter_renders() {
        let mut r = Renderer::new();
        let img = render(&mut r, &doc_with_line(Some("No Such Brush"), 2.0));
        assert!(dark(img.pixel(100, 50)));
        let img = render(&mut r, &doc_with_line(Some("Dots"), 1.0));
        let dark_px = (20..180).filter(|x| dark(img.pixel(*x, 50))).count();
        assert!(dark_px > 20 && dark_px < 150, "dots leave gaps: {dark_px}");
    }

    #[test]
    fn brush_definition_edits_rerender() {
        let mut r = Renderer::new();
        let mut d = doc_with_line(Some("3 pt. Round"), 1.0);
        let a = render(&mut r, &d);
        let mut lib = vectorcraft_brush::library(&d);
        if let vectorcraft_brush::BrushKind::Calligraphic(c) = &mut lib[0].kind {
            c.size = 20.0;
        }
        vectorcraft_brush::store(&mut d, &lib);
        let b = render(&mut r, &d);
        assert!(!dark(a.pixel(100, 58)) && dark(b.pixel(100, 58)));
    }

    #[test]
    fn stained_symbol_instance() {
        let mut d = Document::new(100.0, 100.0);
        let art = Node::path(
            vectorcraft_doc::NodeId(0),
            shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: std::sync::Arc::new(art) });
        let id = d.alloc_id();
        let mut inst = Node::new(id, vectorcraft_doc::NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::translate((50.0, 50.0)) });
        inst.appearance.set_fill(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
        let l = d.layers[0].id;
        d.insert(Some(l), 0, inst).unwrap();
        let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
        assert_eq!(img.pixel(50, 50), [255, 0, 0, 255]);
    }

    #[test]
    fn an_instance_is_culled_by_the_brushed_strokes_in_its_art() {
        // The art: a line from (0, 40) to (0, 60) with a 1 pt stroke painted by a 20 pt round nib,
        // drawn from x -6. The line lies left of the view, and the nib paints 10 pt either side of it.
        let mut d = Document::new(100.0, 100.0);
        let mut lib = vectorcraft_brush::library(&d);
        let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
            panic!("the 3 pt. Round brush")
        };
        c.size = 20.0;
        vectorcraft_brush::store(&mut d, &lib);
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        ap.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
        let art = Node::path(vectorcraft_doc::NodeId(0), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), ap);
        d.symbols.push(vectorcraft_doc::Symbol { name: "Nib".into(), art: std::sync::Arc::new(art) });
        let id = d.alloc_id();
        let inst = Node::new(id, vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Nib".into(), xf: Affine::translate((-6.0, 0.0)) });
        let l = d.layers[0].id;
        d.insert(Some(l), 0, inst).unwrap();
        let img = render(&mut Renderer::new(), &d);
        assert!(dark(img.pixel(1, 50)), "{:?}", img.pixel(1, 50));
    }

    #[test]
    fn a_repeat_is_culled_by_the_brush_art_of_what_it_holds() {
        // A one-by-one grid repeat of the line of the test above drawn from x -8, then of an
        // instance of a symbol of that line: the nib paints 10 pt either side of the line, into
        // the view.
        let line = |d: &mut Document| {
            let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
            ap.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
            Node::path(d.alloc_id(), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), ap)
        };
        for instance in [false, true] {
            let mut d = Document::new(100.0, 100.0);
            let mut lib = vectorcraft_brush::library(&d);
            let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
                panic!("the 3 pt. Round brush")
            };
            c.size = 20.0;
            vectorcraft_brush::store(&mut d, &lib);
            let moved = Affine::translate((-8.0, 0.0));
            let source = if instance {
                let art = line(&mut d);
                d.symbols.push(vectorcraft_doc::Symbol { name: "Nib".into(), art: std::sync::Arc::new(art) });
                Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Nib".into(), xf: moved })
            } else {
                let mut n = line(&mut d);
                n.transform(moved, false);
                n
            };
            let grid = vectorcraft_doc::TileType::Grid;
            let kind = vectorcraft_doc::RepeatKind::Grid {
                h_spacing: 0.0,
                v_spacing: 0.0,
                rows: 1,
                cols: 1,
                grid_type: grid,
                flip_rows: false,
                flip_cols: false,
            };
            let spec = vectorcraft_doc::RepeatSpec { kind, source: vec![std::sync::Arc::new(source)] };
            let repeat = Node::new(d.alloc_id(), vectorcraft_doc::NodeKind::Repeat(spec));
            let l = d.layers[0].id;
            d.insert(Some(l), 0, repeat).unwrap();
            let img = render(&mut Renderer::new(), &d);
            assert!(dark(img.pixel(1, 50)), "an instance: {instance}; {:?}", img.pixel(1, 50));
        }
    }
}
