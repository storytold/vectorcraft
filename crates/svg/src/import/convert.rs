use super::{Affine, Arc, BezPath, BlendMode, Color, FillRule, GradientStop, HashMap, Node, PathData, Point, XNode, roxmltree};
use vectorcraft_geom::path::{Anchor, AnchorKind, SubPath};
use vectorcraft_geom::shapes::KAPPA;

/// `n` with every object id (its descendants' and its mask art's too) zeroed, to compare art.
pub(super) fn without_ids(n: &Node) -> Node {
    let mut n = n.clone();
    n.id = vectorcraft_doc::NodeId(0);
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
        m.art = Arc::new(without_ids(&m.art));
    }
    for c in n.children_mut().into_iter().flatten() {
        *c = Arc::new(without_ids(c));
    }
    n
}

/// `base`, or `base 2`, `base 3`… : the first that isn't `taken`. `next` keeps where each base's
/// suffixes go on from, so names sharing a base take linear work in all.
pub(super) fn unique_name(base: &str, next: &mut HashMap<String, usize>, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    let k = next.entry(base.to_string()).or_insert(2);
    loop {
        let name = format!("{base} {k}");
        *k += 1;
        if !taken(&name) {
            return name;
        }
    }
}

/// The stops our export added for midpoints (`data-vc-midpoint`), by gradient id: for each stop,
/// the midpoint it stands for.
pub(super) fn midpoint_stops(xml: &roxmltree::Document) -> HashMap<String, Vec<Option<f32>>> {
    xml.descendants()
        .filter(|n| matches!(n.tag_name().name(), "linearGradient" | "radialGradient"))
        .filter_map(|g| {
            let id = g.attribute("id")?;
            let marks: Vec<Option<f32>> = g
                .children()
                .filter(|c| c.tag_name().name() == "stop")
                .map(|c| c.attribute("data-vc-midpoint").and_then(|v| v.parse().ok()))
                .collect();
            marks.iter().any(Option::is_some).then(|| (id.to_string(), marks))
        })
        .collect()
}

/// A gradient's stops, with the midpoint stops `marks` names folded back into the midpoints of
/// the stops before them.
pub(super) fn gradient_stops(g: &usvg::BaseGradient, marks: Option<&[Option<f32>]>) -> Vec<GradientStop> {
    let marks = marks.filter(|m| m.len() == g.stops().len());
    let mut out: Vec<GradientStop> = Vec::with_capacity(g.stops().len());
    for (i, s) in g.stops().iter().enumerate() {
        if let (Some(mid), Some(prev)) = (marks.and_then(|m| m.get(i).copied().flatten()), out.last_mut()) {
            prev.midpoint = mid.clamp(0.0, 1.0);
            continue;
        }
        let c = s.color();
        out.push(GradientStop { opacity: s.opacity().get(), ..GradientStop::new(s.offset().get(), Color::rgb8(c.red, c.green, c.blue)) });
    }
    out
}

/// The options [`crate::export::MASK_FLAGS`] records on exported `<mask>` elements, by mask id:
/// (clip, invert).
pub(super) fn mask_flags(xml: &roxmltree::Document) -> HashMap<String, (bool, bool)> {
    let attr = crate::export::MASK_FLAGS;
    if !xml.input_text().contains(attr) {
        return HashMap::new();
    }
    let flags = |n: roxmltree::Node| {
        let has = |f: &str| n.attribute(attr).is_some_and(|v| v.split_whitespace().any(|x| x == f));
        Some((n.attribute("id")?.to_string(), (!has("noclip"), has("invert"))))
    };
    xml.descendants().filter(|n| n.tag_name().name() == "mask" && n.has_attribute(attr)).filter_map(flags).collect()
}

/// Prefix of the ids [`prepass`] gives the elements it has to find again that have none.
pub(super) const MADE_UP_ID: &str = "vectorcraft-id-";

/// Is `id` one [`prepass`] made up (not a name)?
pub(super) fn made_up(id: &str) -> bool {
    id.starts_with(MADE_UP_ID)
}

/// The URL an `<a>` element links to.
pub(crate) fn href<'a>(a: XNode<'a, '_>) -> Option<&'a str> {
    a.attribute("href").or_else(|| a.attribute(("http://www.w3.org/1999/xlink", "href"))).filter(|h| !h.is_empty())
}

pub(super) fn aff(t: usvg::Transform) -> Affine {
    Affine::new([t.sx as f64, t.ky as f64, t.kx as f64, t.sy as f64, t.tx as f64, t.ty as f64])
}

pub(super) fn is_plain(g: &usvg::Group) -> bool {
    g.clip_path().is_none() && only_clips(g)
}

/// Does group `g` leave its art as it is, but for a clip path (no mask, filter, opacity or blend)?
pub(super) fn only_clips(g: &usvg::Group) -> bool {
    g.mask().is_none() && g.filters().is_empty() && g.opacity().get() >= 1.0 && g.blend_mode() == usvg::BlendMode::Normal
}

pub(super) fn blend(b: usvg::BlendMode) -> BlendMode {
    use usvg::BlendMode as B;
    match b {
        B::Normal => BlendMode::Normal,
        B::Multiply => BlendMode::Multiply,
        B::Screen => BlendMode::Screen,
        B::Overlay => BlendMode::Overlay,
        B::Darken => BlendMode::Darken,
        B::Lighten => BlendMode::Lighten,
        B::ColorDodge => BlendMode::ColorDodge,
        B::ColorBurn => BlendMode::ColorBurn,
        B::HardLight => BlendMode::HardLight,
        B::SoftLight => BlendMode::SoftLight,
        B::Difference => BlendMode::Difference,
        B::Exclusion => BlendMode::Exclusion,
        B::Hue => BlendMode::Hue,
        B::Saturation => BlendMode::Saturation,
        B::Color => BlendMode::Color,
        B::Luminosity => BlendMode::Luminosity,
    }
}

pub(super) fn bezpath(p: &usvg::tiny_skia_path::Path, m: Affine) -> BezPath {
    use usvg::tiny_skia_path::PathSegment as S;
    let pt = |p: usvg::tiny_skia_path::Point| m * Point::new(p.x as f64, p.y as f64);
    let mut bp = BezPath::new();
    for s in p.segments() {
        match s {
            S::MoveTo(p) => bp.move_to(pt(p)),
            S::LineTo(p) => bp.line_to(pt(p)),
            S::QuadTo(a, p) => bp.quad_to(pt(a), pt(p)),
            S::CubicTo(a, b, p) => bp.curve_to(pt(a), pt(b), pt(p)),
            S::Close => bp.close_path(),
        }
    }
    bp
}

/// The path usvg makes of a `<circle>` or `<ellipse>` (`p`: one closed outline of arcs from the
/// right end of the horizontal axis round through the bottom) as the ellipse VectorCraft draws: four
/// smooth anchors on the axes in the same order, each quarter a cubic with handles [`KAPPA`] of the
/// radii long, the ellipse export writes as `<ellipse>`. `None` when `p` is not such an outline.
pub(super) fn drawn_ellipse(p: &usvg::tiny_skia_path::Path) -> Option<PathData> {
    use usvg::tiny_skia_path::PathSegment as S;
    let mut ends = vec![];
    let mut closed = false;
    for s in p.segments() {
        match s {
            S::MoveTo(q) if ends.is_empty() => ends.push(Point::new(q.x as f64, q.y as f64)),
            S::LineTo(q) | S::QuadTo(_, q) | S::CubicTo(_, _, q) if !ends.is_empty() && !closed => ends.push(Point::new(q.x as f64, q.y as f64)),
            S::Close if !closed => closed = true,
            _ => return None,
        }
    }
    let (x0, x1) = ends.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.x), hi.max(q.x)));
    let (y0, y1) = ends.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), q| (lo.min(q.y), hi.max(q.y)));
    let (c, rx, ry) = (Point::new((x0 + x1) / 2.0, (y0 + y1) / 2.0), (x1 - x0) / 2.0, (y1 - y0) / 2.0);
    // usvg works in f32: points match within a few of its steps at the size of the coordinates, and
    // a radius of fewer than two steps does not survive the conversion.
    let step = f64::from(f32::EPSILON) * [x0, x1, y0, y1].iter().fold(1.0f64, |m, v| m.max(v.abs()));
    let tol = 16.0 * step;
    let axes = [Point::new(x1, c.y), Point::new(c.x, y1), Point::new(x0, c.y), Point::new(c.x, y0)];
    let off = 1e-3 + 4.0 * tol / rx.min(ry);
    let on_ellipse = |q: &Point| (((q.x - c.x) / rx).powi(2) + ((q.y - c.y) / ry).powi(2) - 1.0).abs() <= off;
    if !closed
        || rx < 2.0 * step
        || ry < 2.0 * step
        || ends.first().is_none_or(|s| s.distance(axes[0]) > tol)
        || !axes.iter().all(|a| ends.iter().any(|q| q.distance(*a) <= tol))
        || !ends.iter().all(on_ellipse)
    {
        return None;
    }
    let anchors = (0..4)
        .map(|i| {
            let (p, next, prev) = (axes[i], axes[(i + 1) % 4], axes[(i + 3) % 4]);
            Anchor { p, h_in: p + (prev - c) * KAPPA, h_out: p + (next - c) * KAPPA, kind: AnchorKind::Smooth }
        })
        .collect();
    Some(PathData::single(SubPath::new(anchors, true)))
}

/// The shapes of clip path `cp` on an element whose user space `ts` maps to the document.
pub(super) fn clip_shapes(cp: &usvg::ClipPath, ts: Affine) -> Vec<(BezPath, FillRule)> {
    fn collect(g: &usvg::Group, m: Affine, out: &mut Vec<(BezPath, FillRule)>) {
        for c in g.children() {
            match c {
                usvg::Node::Group(g) => collect(g, m * aff(g.transform()), out),
                usvg::Node::Path(p) => out.push((bezpath(p.data(), m), p.fill().map(|f| rule(f.rule())).unwrap_or_default())),
                _ => {}
            }
        }
    }
    let mut shapes = Vec::new();
    collect(cp.root(), ts * aff(cp.transform()), &mut shapes);
    shapes
}

/// Does clip path `cp` (on an element whose user space `ts` maps to the document) leave `art`,
/// placed by `xf`, whole? (Only its bounds are checked: the box must be inside a clip shape.)
pub(super) fn uncut(cp: &usvg::ClipPath, ts: Affine, art: &Node, xf: Affine) -> bool {
    let Some(b) = art.visual_bounds() else { return false };
    if cp.clip_path().is_some() {
        return false;
    }
    // Art touching the clip's edge isn't cut by it.
    let e = 1e-6 * (b.width() + b.height()).max(1e-9);
    let b = b.inflate(-e, -e);
    let shapes = clip_shapes(cp, ts);
    let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    corners.iter().all(|c| shapes.iter().any(|(s, _)| kurbo::Shape::contains(s, xf * *c)))
}

pub(super) fn rule(r: usvg::FillRule) -> FillRule {
    match r {
        usvg::FillRule::NonZero => FillRule::NonZero,
        usvg::FillRule::EvenOdd => FillRule::EvenOdd,
    }
}
