//! A drawing's objects as VectorCraft art: paths, live rectangles and ellipses, groups, solid
//! fills and strokes, and the spot colors they use as swatches.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect};
use vectorcraft_cdr::Warning;
use vectorcraft_cdr::model::{self as cd, Geometry};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, Dash, FillLayer, LineCap, LineJoin, StrokeLayer};
use vectorcraft_doc::geom::{FillRule, PathData};
use vectorcraft_doc::{Document, Guide, LiveShape, Node, NodeKind};

/// The most nested groups mapped; the reader holds nesting well below it.
const MAX_DEPTH: usize = 256;

pub(super) struct Builder {
    pub(super) doc: Document,
    /// What mapping left out, as the reader's warning kinds.
    pub(super) notes: Vec<Warning>,
    /// A page side was cut to the longest artboard side.
    pub(super) pages_cut: bool,
    /// What copies of master art onto the pages may still take ([`super::MAX_MASTER_COPIES`] at
    /// the start).
    pub(super) copies_left: usize,
    /// Horizontal and vertical guidelines, as guides of the pages' artboards.
    pub(super) guide_list: Vec<Guide>,
    /// Guidelines at other angles, as guide paths.
    pub(super) guide_paths: Vec<Arc<Node>>,
    /// Colors read in each model, which picks the document's color mode.
    pub(super) cmyk: usize,
    pub(super) rgb: usize,
    /// The spot colors used, in the order first seen, with their full-strength color and the tint
    /// of the use it was worked out from (`None`: the file stores no color for it).
    spots: Vec<(String, Color, Option<f32>)>,
    spot_index: HashMap<String, usize>,
}

impl Builder {
    pub(super) fn new() -> Self {
        Builder {
            doc: Document::new(1.0, 1.0),
            notes: Vec::new(),
            pages_cut: false,
            copies_left: super::MAX_MASTER_COPIES,
            guide_list: Vec::new(),
            guide_paths: Vec::new(),
            cmyk: 0,
            rgb: 0,
            spots: Vec::new(),
            spot_index: HashMap::new(),
        }
    }

    /// Every spot color `d` uses, with its full-strength color taken from its use at the highest
    /// tint: a use at a low tint keeps little of the color, and one at 0% keeps none.
    pub(super) fn learn_spots(&mut self, d: &cd::Drawing) {
        let layers = d.pages.iter().flat_map(|p| &p.layers).chain(&d.master);
        let mut stack: Vec<&cd::Object> = layers.flat_map(|l| &l.objects).collect();
        while let Some(o) = stack.pop() {
            match o {
                cd::Object::Shape(s) => {
                    for c in s.fill.iter().chain(s.outline.iter().map(|o| &o.color)) {
                        self.learn_spot(c);
                    }
                }
                cd::Object::Group(kids) => stack.extend(kids),
                cd::Object::Clip { container, objects } => {
                    for c in container.fill.iter().chain(container.outline.iter().map(|o| &o.color)) {
                        self.learn_spot(c);
                    }
                    stack.extend(objects);
                }
            }
        }
    }

    /// Note spot color `c`, keeping the full-strength color of its use at the highest tint.
    fn learn_spot(&mut self, c: &cd::Color) -> Option<Color> {
        let cd::Color::Spot { name, tint, look } = c else { return None };
        let seen = look.map(|_| *tint);
        let base = look.map_or(Color::BLACK, |[r, g, b]| Color::rgb(r, g, b));
        let Some(&i) = self.spot_index.get(name) else {
            self.spot_index.insert(name.clone(), self.spots.len());
            self.spots.push((name.clone(), base, seen));
            return Some(base);
        };
        let slot = self.spots.get_mut(i)?;
        if seen.is_some() && seen > slot.2 {
            slot.1 = base;
            slot.2 = seen;
        }
        Some(slot.1)
    }

    fn note(&mut self, w: Warning) {
        if !self.notes.contains(&w) {
            self.notes.push(w);
        }
    }

    /// `objects` (front to back, as the file lists them) bottom to top, mapped by `to_doc`.
    pub(super) fn objects(&mut self, objects: &[cd::Object], to_doc: Affine) -> Vec<Arc<Node>> {
        self.objects_at(objects, to_doc, 0)
    }

    fn objects_at(&mut self, objects: &[cd::Object], to_doc: Affine, depth: usize) -> Vec<Arc<Node>> {
        if depth > MAX_DEPTH {
            self.note(Warning::TooDeep);
            return Vec::new();
        }
        let mut out = Vec::with_capacity(objects.len());
        for o in objects.iter().rev() {
            match o {
                cd::Object::Shape(s) => out.extend(self.shape(s, to_doc, false)),
                cd::Object::Group(kids) => {
                    let children = self.objects_at(kids, to_doc, depth + 1);
                    if !children.is_empty() {
                        out.push(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children, clip: false })));
                    }
                }
                // A clipping group whose clipping path keeps the container's paint: its fill
                // behind the clipped objects and its outline over them.
                cd::Object::Clip { container, objects } => {
                    // A clipping path's stroke paints over the clipped objects, so an outline
                    // behind the fill becomes a shape of its own below the clipping group, the
                    // two in a group of their own.
                    let behind = container.outline.as_ref().is_some_and(|o| o.behind_fill);
                    let (outline, clip) = if behind {
                        let outline = self.shape(&cd::Shape { fill: None, ..(**container).clone() }, to_doc, false);
                        (outline, self.shape(&cd::Shape { outline: None, ..(**container).clone() }, to_doc, true))
                    } else {
                        (None, self.shape(container, to_doc, true))
                    };
                    let Some(clip) = clip else { continue };
                    let mut children = vec![clip];
                    children.extend(self.objects_at(objects, to_doc, depth + 1));
                    let group = Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children, clip: true }));
                    match outline {
                        Some(outline) => {
                            let children = vec![outline, group];
                            out.push(Arc::new(Node::new(self.doc.alloc_id(), NodeKind::Group { children, clip: false })));
                        }
                        None => out.push(group),
                    }
                }
            }
        }
        out
    }

    /// Shape `s` placed by `to_doc`; as a clipping path when `clipping`.
    fn shape(&mut self, s: &cd::Shape, to_doc: Affine, clipping: bool) -> Option<Arc<Node>> {
        let m = to_doc * affine(&s.matrix);
        let (path, live) = match &s.geometry {
            Geometry::Path(p) => (PathData::from_bezpath(&(m * bez(p))), None),
            Geometry::Rect { w, h, radii } => live_rect(*w, *h, *radii, m)?,
            Geometry::Ellipse { w, h } => {
                let (x0, y0) = (w.min(0.0), h.min(0.0));
                let live = LiveShape::Ellipse { w: w.abs(), h: h.abs(), pie: (0.0, 360.0), xf: m * Affine::translate((x0, y0)) };
                (live.to_path(), Some(live))
            }
        };
        if path.is_empty() || !in_range(&path) {
            self.note(Warning::TooDeep);
            return None;
        }
        // In paint order: an outline behind the fill comes first.
        let mut items = Vec::new();
        if let Some(o) = s.outline.as_ref().filter(|o| o.behind_fill) {
            items.push(AppearanceItem::Stroke(self.stroke(o)));
        }
        if let Some(c) = &s.fill {
            items.push(AppearanceItem::Fill(FillLayer::new(self.paint(c))));
        }
        if let Some(o) = s.outline.as_ref().filter(|o| !o.behind_fill) {
            items.push(AppearanceItem::Stroke(self.stroke(o)));
        }
        // A curve of several subpaths is filled with holes where they overlap: VectorCraft's own
        // choice of rule for it.
        let rule = FillRule::EvenOdd;
        let mut n = Node::new(self.doc.alloc_id(), NodeKind::Path { path, rule, live: live.map(Box::new), clipping, guide: false });
        n.appearance = Appearance { items, ..Appearance::default() }.into();
        Some(Arc::new(n))
    }

    fn stroke(&mut self, o: &cd::Outline) -> StrokeLayer {
        let width = if o.width.is_finite() { o.width.clamp(0.0, 1000.0) } else { 0.0 };
        let mut l = StrokeLayer::new(self.paint(&o.color), width);
        l.cap = match o.cap {
            cd::Cap::Butt => LineCap::Butt,
            cd::Cap::Round => LineCap::Round,
            cd::Cap::Square => LineCap::Square,
        };
        l.join = match o.join {
            cd::Join::Miter => LineJoin::Miter,
            cd::Join::Round => LineJoin::Round,
            cd::Join::Bevel => LineJoin::Bevel,
        };
        // Dashes are stored in multiples of the outline's width.
        if !o.dashes.is_empty() && width > 0.0 {
            let pattern: Vec<f64> = o.dashes.iter().map(|d| d * width).collect();
            l.dash = Some(Dash { pattern, offset: 0.0, align_corners: false });
        }
        l
    }

    fn paint(&mut self, c: &cd::Color) -> Paint {
        match c {
            cd::Color::Rgb([r, g, b]) => {
                self.rgb += 1;
                Paint::solid(Color::rgb(*r, *g, *b))
            }
            cd::Color::Cmyk([c, m, y, k]) => {
                self.cmyk += 1;
                Paint::solid(Color::cmyk(*c, *m, *y, *k))
            }
            cd::Color::Gray(k) => Paint::solid(Color::Gray { k: *k }),
            cd::Color::Registration => Paint::Solid { color: Color::cmyk(1.0, 1.0, 1.0, 1.0), swatch: Some(REGISTRATION.into()), tint: 1.0 },
            cd::Color::Spot { name, tint, .. } => {
                let base = self.learn_spot(c).unwrap_or(Color::BLACK);
                let tint = tint.clamp(0.0, 1.0);
                let swatch = self.spot_index.get(name).and_then(|&i| self.spots.get(i)).map_or_else(|| name.clone(), |s| s.0.clone());
                Paint::Solid { color: base.tinted(tint), swatch: Some(swatch), tint }
            }
        }
    }

    /// Gives each spot color a swatch name of its own in the new document: a name its swatches,
    /// color groups or Registration already use becomes the first free "name 2", "name 3"…, so
    /// the ink stays a spot color. The drawing's other spot names are kept as they are. Call it
    /// once the document is made and before any paint.
    pub(super) fn name_spots(&mut self) {
        let mut free = self.doc.free_swatch_names();
        let clashes: Vec<bool> = self.spots.iter().map(|s| s.0 == REGISTRATION || self.doc.swatch_name_taken(&s.0)).collect();
        for (s, _) in self.spots.iter().zip(&clashes).filter(|(_, c)| !**c) {
            free.take(&s.0);
        }
        for (s, _) in self.spots.iter_mut().zip(&clashes).filter(|(_, c)| **c) {
            s.0 = free.take(&s.0);
        }
    }

    /// The spot colors the drawing used, as global spot swatches; a swatch of the same name the
    /// document already has is kept as it is.
    pub(super) fn finish_swatches(&mut self, doc: &mut Document) {
        self.spot_index.clear();
        for (name, color, _) in std::mem::take(&mut self.spots) {
            if !doc.swatches.iter().any(|s| s.name == name) {
                doc.swatches.push(Swatch { name, paint: Paint::solid(color), global: true, spot: true });
            }
        }
    }

    /// A guideline through `at` (document space) at `angle` degrees counterclockwise, as a guide
    /// path from edge to edge of the page `rect`, so it moves with the page's artboard. A
    /// guideline that misses the page runs through `at` for the page's diagonal each way.
    pub(super) fn angled_guide(&mut self, at: Point, angle: f64, rect: Rect) {
        let (s, c) = angle.to_radians().sin_cos();
        // The document's y axis points down.
        let dir = kurbo::Vec2::new(c, -s);
        let (a, b) = match chord(at, dir, rect) {
            Some((t0, t1)) => (at + dir * t0, at + dir * t1),
            None => {
                let d = dir * rect.width().hypot(rect.height()).max(1.0);
                (at - d, at + d)
            }
        };
        let mut bp = BezPath::new();
        bp.move_to(a);
        bp.line_to(b);
        let path = PathData::from_bezpath(&bp);
        if !in_range(&path) {
            self.note(Warning::TooDeep);
            return;
        }
        let n = Node::new(self.doc.alloc_id(), NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: true });
        self.guide_paths.push(Arc::new(n));
    }
}

/// Where the line through `at` along the unit vector `dir` crosses `rect`: the distances along
/// it to the two edges it meets, the nearer first. None when it misses `rect` or only touches it
/// at a point.
fn chord(at: Point, dir: kurbo::Vec2, rect: Rect) -> Option<(f64, f64)> {
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for (p, d, min, max) in [(at.x, dir.x, rect.x0, rect.x1), (at.y, dir.y, rect.y0, rect.y1)] {
        if d.abs() < 1e-12 {
            if p < min || p > max {
                return None;
            }
        } else {
            let (t0, t1) = ((min - p) / d, (max - p) / d);
            lo = lo.max(t0.min(t1));
            hi = hi.min(t0.max(t1));
        }
    }
    (lo.is_finite() && hi.is_finite() && hi - lo > 1e-9).then_some((lo, hi))
}

fn affine(m: &cd::Matrix) -> Affine {
    Affine::new(m.0)
}

fn bez(p: &cd::Path) -> BezPath {
    let pt = |q: cd::Pt| Point::new(q.x, q.y);
    let mut bp = BezPath::new();
    for s in &p.subpaths {
        bp.move_to(pt(s.start));
        for seg in &s.segments {
            match *seg {
                cd::Segment::Line(e) => bp.line_to(pt(e)),
                cd::Segment::Cubic(a, b, e) => bp.curve_to(pt(a), pt(b), pt(e)),
            }
        }
        if s.closed {
            bp.close_path();
        }
    }
    bp
}

/// A rectangle from the origin to (`w`, `h`) with `radii` counterclockwise from its corner at
/// (`w`, 0), placed by `m`, as a live rectangle: each radius stays on the corner it belongs to,
/// whichever way `w` and `h` point.
fn live_rect(w: f64, h: f64, radii: [f64; 4], m: Affine) -> Option<(PathData, Option<LiveShape>)> {
    if !(w.is_finite() && h.is_finite()) {
        return None;
    }
    let (x0, y1, ww, hh) = (w.min(0.0), h.max(0.0), w.abs(), h.abs());
    // The live shape's own space has its origin at the top-left and y pointing down.
    let to_file = Affine::translate((x0, y1)) * Affine::FLIP_Y;
    let corners = [(w, 0.0), (w, h), (0.0, h), (0.0, 0.0)];
    let mut live_radii = [0.0; 4];
    for ((px, py), r) in corners.into_iter().zip(radii) {
        let (lx, ly) = (px - x0, y1 - py);
        let right = lx > ww / 2.0;
        let bottom = ly > hh / 2.0;
        let i = match (right, bottom) {
            (false, false) => 0,
            (true, false) => 1,
            (true, true) => 2,
            (false, true) => 3,
        };
        if let Some(slot) = live_radii.get_mut(i) {
            *slot = if r.is_finite() { r.max(0.0) } else { 0.0 };
        }
    }
    let mut live = LiveShape::Rectangle { w: ww, h: hh, radii: live_radii, kinds: Default::default(), radii_relative: false, xf: m * to_file };
    live.fold_scale();
    Some((live.to_path(), Some(live)))
}

/// Whether every control point of `path` is finite and within `MAX_COORD` of the origin.
fn in_range(path: &PathData) -> bool {
    path.control_bounds().is_some_and(|r| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= crate::MAX_COORD))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A named spot color takes its full-strength color from its use at the highest tint, wherever
    /// that use comes in the drawing: a use at 0% stores only white.
    #[test]
    fn a_spot_color_keeps_its_color_from_its_strongest_use() {
        let red = [1.0, 0.0, 0.0];
        let shape = |tint: f32, look: [f32; 3]| {
            cd::Object::Shape(Box::new(cd::Shape {
                geometry: Geometry::Rect { w: 10.0, h: 10.0, radii: [0.0; 4] },
                matrix: cd::Matrix::IDENTITY,
                fill: Some(cd::Color::Spot { name: "Red ink".into(), tint, look: Some(look) }),
                outline: None,
            }))
        };
        for objects in [vec![shape(0.0, [1.0; 3]), shape(1.0, red)], vec![shape(1.0, red), shape(0.0, [1.0; 3])]] {
            let layer = cd::Layer { name: "L".into(), kind: cd::LayerKind::Drawing, objects, guides: vec![] };
            let d = cd::Drawing {
                version: 1300,
                page_size: None,
                pages: vec![cd::Page { size: None, layers: vec![layer] }],
                master: vec![],
                guides: vec![],
                warnings: vec![],
            };
            let doc = Builder::new().document(&d);
            let swatch = doc.swatches.iter().find(|s| s.name == "Red ink").unwrap();
            assert_eq!(swatch.paint, Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
            let layer = doc.layers.first().unwrap().children().unwrap();
            let full = layer.iter().find_map(|n| match n.appearance.items.first() {
                Some(AppearanceItem::Fill(f)) => match &f.paint {
                    Paint::Solid { color, tint, .. } if *tint == 1.0 => Some(*color),
                    _ => None,
                },
                _ => None,
            });
            assert_eq!(full, Some(Color::rgb(1.0, 0.0, 0.0)));
        }
    }

    /// A spot color named like one of a new document's process swatches gets a numbered name that
    /// no other spot color of the drawing uses, and its paint names that spot swatch.
    #[test]
    fn a_spot_color_named_like_a_default_swatch_stays_a_spot_color() {
        let shape = |name: &str| {
            cd::Object::Shape(Box::new(cd::Shape {
                geometry: Geometry::Rect { w: 10.0, h: 10.0, radii: [0.0; 4] },
                matrix: cd::Matrix::IDENTITY,
                fill: Some(cd::Color::Spot { name: name.into(), tint: 1.0, look: Some([0.0; 3]) }),
                outline: None,
            }))
        };
        let layer = cd::Layer { name: "L".into(), kind: cd::LayerKind::Drawing, objects: vec![shape("Black"), shape("Black 2")], guides: vec![] };
        let d = cd::Drawing {
            version: 1300,
            page_size: None,
            pages: vec![cd::Page { size: None, layers: vec![layer] }],
            master: vec![],
            guides: vec![],
            warnings: vec![],
        };
        assert!(Document::new(612.0, 792.0).swatches.iter().any(|s| s.name == "Black" && !s.spot));
        let doc = Builder::new().document(&d);
        let layer = doc.layers.first().unwrap().children().unwrap();
        let mut names: Vec<String> = layer
            .iter()
            .filter_map(|n| match n.appearance.items.first() {
                Some(AppearanceItem::Fill(f)) => match &f.paint {
                    Paint::Solid { swatch: Some(name), .. } => Some(name.clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        names.sort();
        assert_eq!(names, ["Black 2", "Black 3"]);
        for name in &names {
            let swatch = doc.swatches.iter().find(|s| &s.name == name).unwrap();
            assert!(swatch.spot && swatch.global, "{name}: {swatch:?}");
        }
    }

    /// Copies of master art onto the pages take from the budget by their objects, subpaths and
    /// segments: a curve of 300 segments costs 302 each time it is copied.
    #[test]
    fn master_copies_count_their_path_segments() {
        let segments = (1..=300).map(|i| cd::Segment::Line(cd::Pt { x: f64::from(i), y: 0.0 })).collect();
        let curve = cd::Object::Shape(Box::new(cd::Shape {
            geometry: Geometry::Path(cd::Path { subpaths: vec![cd::SubPath { start: cd::Pt { x: 0.0, y: 0.0 }, segments, closed: false }] }),
            matrix: cd::Matrix::IDENTITY,
            fill: None,
            outline: None,
        }));
        let master = cd::Layer { name: "M".into(), kind: cd::LayerKind::Drawing, objects: vec![curve], guides: vec![] };
        let d = cd::Drawing {
            version: 1300,
            page_size: None,
            pages: (0..10).map(|_| cd::Page { size: None, layers: vec![] }).collect(),
            master: vec![master],
            guides: vec![],
            warnings: vec![],
        };
        let mut b = Builder::new();
        b.copies_left = 1000;
        let doc = b.document(&d);
        let copies = doc.layers.first().and_then(|l| l.children()).map_or(0, |c| c.len());
        assert_eq!(copies, 3, "1,000 holds three copies of 302");
        assert!(b.notes.contains(&Warning::TooDeep));
    }

    /// Copies of master art take from the budget by the dashes of their outlines too: a rectangle
    /// whose outline has 299 dashes costs 300 each time it is copied.
    #[test]
    fn master_copies_count_their_dashes() {
        let outline = cd::Outline {
            width: 1.0,
            color: cd::Color::Gray(1.0),
            cap: cd::Cap::Butt,
            join: cd::Join::Miter,
            dashes: vec![1.0; 299],
            behind_fill: false,
        };
        let rect = cd::Object::Shape(Box::new(cd::Shape {
            geometry: Geometry::Rect { w: 10.0, h: 10.0, radii: [0.0; 4] },
            matrix: cd::Matrix::IDENTITY,
            fill: None,
            outline: Some(outline),
        }));
        let master = cd::Layer { name: "M".into(), kind: cd::LayerKind::Drawing, objects: vec![rect], guides: vec![] };
        let d = cd::Drawing {
            version: 1300,
            page_size: None,
            pages: (0..10).map(|_| cd::Page { size: None, layers: vec![] }).collect(),
            master: vec![master],
            guides: vec![],
            warnings: vec![],
        };
        let mut b = Builder::new();
        b.copies_left = 1000;
        let doc = b.document(&d);
        let copies = doc.layers.first().and_then(|l| l.children()).map_or(0, |c| c.len());
        assert_eq!(copies, 3, "1,000 holds three copies of 300");
        assert!(b.notes.contains(&Warning::TooDeep));
    }

    /// A corner rounded in the file stays on the same corner of the page, however the rectangle's
    /// size points.
    #[test]
    fn rectangle_radii_stay_on_their_corners() {
        // Only the corner at (w, 0) rounded; the page flips y as the document does.
        for (w, h) in [(100.0, 50.0), (-100.0, 50.0), (100.0, -50.0), (-100.0, -50.0)] {
            let (path, _) = live_rect(w, h, [10.0, 0.0, 0.0, 0.0], Affine::FLIP_Y).unwrap();
            let rounded = Point::new(w, -0.0);
            let bp = path.to_bezpath();
            // No anchor sits on the rounded corner itself; the other three corners are anchors.
            let anchors: Vec<Point> = bp.elements().iter().filter_map(|e| e.end_point()).collect();
            assert!(!anchors.iter().any(|p| (*p - rounded).hypot() < 1e-6), "{w} {h}: {anchors:?}");
            for c in [Point::new(w, -h), Point::new(0.0, -h), Point::new(0.0, 0.0)] {
                assert!(anchors.iter().any(|p| (*p - c).hypot() < 1e-6), "{w} {h}: {c:?} not in {anchors:?}");
            }
        }
    }
}
