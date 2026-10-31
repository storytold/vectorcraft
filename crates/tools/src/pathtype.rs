//! Type on a path's brackets: selected type on a path shows a start, a centre and an end bracket
//! with the Selection and Direct Selection tools. Dragging the start or end bracket sets where the
//! type begins or ends along its path; dragging the centre bracket slides the type along the path
//! (its brackets with it) and, dragged across the path, flips the type to the other side
//! (Cmd/Ctrl held: it only slides), as the public help describes
//! ([Create type on a path](https://web.archive.org/web/20220127000000/https://helpx.adobe.com/illustrator/using/creating-type-path.html):
//! "drag the center bracket along the path […] To flip the direction of text along a path, drag
//! the bracket across the path"). Each drag is one undo step of `type.pathOptions`. The tools
//! draw the brackets from the same geometry they hit-test, in screen pixels so they read at any
//! zoom: the start bracket's foot ends in an arrowhead pointing into the type, the end bracket's
//! in a stop bar, and the centre bracket carries a square handle. The bracket under the pointer
//! draws thicker, and the nearest of a bracket and a text port takes a press. While the centre
//! bracket is dragged, a dashed line beside the path shows where the type lands, solid on the
//! other side when it will flip. With more than 10,000 type on a path objects selected, they
//! neither draw nor hit-test any bracket.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextKind};
use vectorcraft_geom::kurbo::{Line, ParamCurveNearest};
use vectorcraft_geom::{Affine, ArcPath, Point, Vec2};

use crate::{Action, Overlay, PointerEvent, ToolContext};

/// How far a bracket reaches (screen px): up from the path on the type's side, down across it,
/// and the foot of the start and end brackets along the path, into the type's span.
const UP_PX: f64 = 22.0;
const DOWN_PX: f64 = 6.0;
const FOOT_PX: f64 = 7.0;
/// The arrowhead of the start bracket's foot and the stop bar of the end bracket's (screen px).
const TIP_PX: f64 = 3.0;
/// The side of the centre bracket's square handle (screen px).
const HANDLE_PX: f32 = 7.0;
/// How near (screen px) a press must be to a bracket to take it.
const HIT_PX: f64 = 8.0;
/// Stroke widths (screen px) of a bracket, and of the one under the pointer.
const WIDTH: f32 = 1.25;
const HOT_WIDTH: f32 = 2.5;
/// How far from the path (screen px) the landing line of a centre-bracket drag runs.
const GHOST_PX: f64 = 10.0;
/// How far past the path (screen px) the centre bracket goes before the type flips.
const FLIP_PX: f64 = 6.0;
/// The most selected type on a path objects whose brackets the tools draw and hit-test: a tenth of
/// the canvas's limit on the anchor points it outlines (`OVERLAY_MAX_ANCHORS` in
/// `crates/ui-egui/src/canvas.rs`). Past it they neither draw nor hit-test any bracket.
const MAX_OBJECTS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bracket {
    Start,
    Centre,
    End,
}

impl Bracket {
    /// Hit-test order: the centre bracket first, then the start one (a closed path's span with
    /// no end of its own has both ends at the start).
    const ALL: [Bracket; 3] = [Bracket::Centre, Bracket::Start, Bracket::End];
}

/// The brackets of a selected type on a path.
#[derive(Clone, Debug)]
pub struct Brackets {
    pub id: NodeId,
    /// Text space → document.
    xf: Affine,
    /// The path in text space.
    arc: ArcPath,
    /// [`TextKind::path_span`].
    span: (f64, f64),
    /// The end bracket was placed (not the end of the path, or once round it).
    end_set: bool,
    /// The type reads right to left: it begins at the end bracket and ends at the start one.
    rtl: bool,
}

impl Brackets {
    /// The brackets of the selected type on a path the tools can edit (outside perspective).
    pub fn of(cx: &ToolContext) -> Vec<Self> {
        cx.selection.objects.iter().filter_map(|id| Self::of_node(cx, *id)).collect()
    }

    fn of_node(cx: &ToolContext, id: NodeId) -> Option<Self> {
        let n = cx.doc.node(id).filter(|n| n.extras().perspective.is_none() && cx.doc.is_editable(id))?;
        let NodeKind::Text(t) = &n.kind else { return None };
        let TextKind::OnPath { path, end, .. } = &t.kind else { return None };
        let det = t.xf.determinant();
        let arc = ArcPath::new(path);
        if !det.is_finite() || det.abs() < 1e-12 || arc.is_empty() {
            return None;
        }
        let text = t.plain_text();
        let first = text.split(['\n', '\u{2029}']).next().unwrap_or_default();
        let rtl = !t.vertical && vectorcraft_text::paragraph_is_rtl(first, t.para_at(0).direction);
        Some(Self { id, xf: t.xf, arc, span: t.kind.path_span()?, end_set: end.is_some(), rtl })
    }

    /// Where bracket `b` stands, as a fraction of the path's length (past 1 round a closed path).
    fn at(&self, b: Bracket) -> f64 {
        match b {
            Bracket::Start => self.span.0,
            Bracket::Centre => (self.span.0 + self.span.1) * 0.5,
            Bracket::End => self.span.1,
        }
    }

    /// The point at fraction `f` of the path, the path's direction there and the side the type
    /// stands on, in document space.
    fn frame(&self, f: f64) -> (Point, Vec2, Vec2) {
        let (p, dir) = self.arc.at(f * self.arc.len());
        let unit = |v: Vec2| {
            let w = self.xf * v.to_point() - self.xf * Point::ZERO;
            w / w.hypot().max(1e-12)
        };
        // Glyphs stand on the left of the path's direction (y down): their top faces (dir.y, -dir.x).
        (self.xf * p, unit(dir), unit(Vec2::new(dir.y, -dir.x)))
    }

    /// The lines that draw bracket `b` at `zoom` (screen px per point): a stem across the path, and
    /// for the start and end brackets a foot along it into the type's span, ending in an arrowhead
    /// where the type begins (the start bracket, or the end one for type read right to left) and a
    /// stop bar where it ends.
    fn lines(&self, b: Bracket, zoom: f64) -> Vec<(Point, Point)> {
        let px = 1.0 / zoom.max(1e-9);
        let (p, dir, up) = self.frame(self.at(b));
        let top = p + up * UP_PX * px;
        let stem = (p - up * DOWN_PX * px, top);
        let tip = TIP_PX * px;
        // The foot points into the span: along the path from the start bracket, back from the end.
        let into = match b {
            Bracket::Centre => return vec![stem],
            Bracket::Start => dir,
            Bracket::End => -dir,
        };
        let end = top + into * FOOT_PX * px;
        if (b == Bracket::Start) != self.rtl {
            vec![stem, (top, end), (end, end - into * tip + up * tip), (end, end - into * tip - up * tip)]
        } else {
            vec![stem, (top, end), (end + up * tip, end - up * tip)]
        }
    }

    /// The centre bracket's square handle, at the top of its stem.
    fn handle(&self, zoom: f64) -> Point {
        let (p, _, up) = self.frame(self.at(Bracket::Centre));
        p + up * UP_PX / zoom.max(1e-9)
    }

    /// The outline of the centre bracket's handle: a square [`HANDLE_PX`] wide, turned with the
    /// path.
    fn handle_outline(&self, zoom: f64) -> vectorcraft_geom::kurbo::BezPath {
        let (_, dir, up) = self.frame(self.at(Bracket::Centre));
        let c = self.handle(zoom);
        let h = f64::from(HANDLE_PX) / 2.0 / zoom.max(1e-9);
        let mut path = vectorcraft_geom::kurbo::BezPath::new();
        path.move_to(c + (dir + up) * h);
        path.line_to(c + (-dir + up) * h);
        path.line_to(c + (-dir - up) * h);
        path.line_to(c + (dir - up) * h);
        path.close_path();
        path
    }

    /// How far (document units) bracket `b` is from `p`: its lines, and the centre's handle.
    fn distance(&self, b: Bracket, p: Point, zoom: f64) -> f64 {
        let lines = self.lines(b, zoom).into_iter().map(|(a, c)| Line::new(a, c).nearest(p, 1e-9).distance_sq.sqrt());
        let handle = (b == Bracket::Centre).then(|| (self.handle(zoom).distance(p) - f64::from(HANDLE_PX) / 2.0 / zoom.max(1e-9)).max(0.0));
        lines.chain(handle).fold(f64::INFINITY, f64::min)
    }

    /// The bracket nearest `p` within `tol` (document units), and how far it is.
    fn hit(&self, p: Point, zoom: f64, tol: f64) -> Option<(Bracket, f64)> {
        Bracket::ALL.into_iter().map(|b| (b, self.distance(b, p, zoom))).filter(|(_, d)| *d <= tol).min_by(|a, b| a.1.total_cmp(&b.1))
    }

    /// The line beside the path where the type lands when its span starts at fraction `start`: on
    /// the type's side, or on the other one when it will `flip`.
    fn ghost(&self, start: f64, flip: bool, zoom: f64) -> vectorcraft_geom::kurbo::BezPath {
        let (s, e) = self.span;
        let off = GHOST_PX / zoom.max(1e-9) * if flip { -1.0 } else { 1.0 };
        let closed = self.arc.is_closed();
        let mut path = vectorcraft_geom::kurbo::BezPath::new();
        const STEPS: usize = 48;
        for i in 0..=STEPS {
            let f = start + (e - s) * i as f64 / STEPS as f64;
            let f = if closed { f.rem_euclid(1.0) } else { f.clamp(0.0, 1.0) };
            let (q, _, up) = self.frame(f);
            let q = q + up * off;
            if i == 0 { path.move_to(q) } else { path.line_to(q) }
        }
        path
    }

    /// The fraction of the path's length nearest document point `p`.
    fn fraction_at(&self, p: Point) -> Option<f64> {
        self.arc.fraction_at(self.xf.inverse() * p)
    }
}

/// Whether the selection holds more than [`MAX_OBJECTS`] type on a path objects,
/// in perspective or not (counting stops there).
fn too_many(cx: &ToolContext) -> bool {
    let on_path = |id: &&NodeId| cx.doc.node(**id).is_some_and(|n| matches!(&n.kind, NodeKind::Text(t) if matches!(t.kind, TextKind::OnPath { .. })));
    cx.selection.objects.iter().filter(on_path).nth(MAX_OBJECTS).is_some()
}

/// The selected type on a path's bracket nearest `p` within [`HIT_PX`], and how far it is.
fn bracket_at(cx: &ToolContext, p: Point) -> Option<(Brackets, Bracket, f64)> {
    Brackets::of(cx)
        .into_iter()
        .filter_map(|b| {
            let (which, d) = b.hit(p, cx.zoom, cx.tol(HIT_PX))?;
            Some((b, which, d))
        })
        .min_by(|a, b| a.2.total_cmp(&b.2))
}

/// How far (document units) the bracket of the selected type on a path under `p` is from it; None
/// when no bracket is within reach.
pub fn bracket_distance(cx: &ToolContext, p: Point) -> Option<f64> {
    if too_many(cx) {
        return None;
    }
    bracket_at(cx, p).map(|(_, _, d)| d)
}

/// Is `p` over a bracket of the selected type on a path, nearer than any text port (a start
/// bracket and an in port sit side by side: the nearer takes the press)?
pub fn over_bracket(cx: &ToolContext, p: Point) -> bool {
    bracket_distance(cx, p).is_some_and(|d| crate::text::port_near(cx, p).is_none_or(|(_, _, port)| d <= port))
}

/// The brackets of the selected type on a path, in their layer's colour: the one under `hover`
/// thicker, and while `drag` moves the centre bracket the line where the type lands.
pub fn overlays(cx: &ToolContext, hover: Option<Point>, drag: Option<&BracketDrag>) -> Vec<Overlay> {
    if too_many(cx) {
        return vec![];
    }
    let hot = match drag {
        Some(d) => Some((d.brackets.id, d.which)),
        None => hover.and_then(|p| bracket_at(cx, p)).map(|(b, which, _)| (b.id, which)),
    };
    let mut out = vec![];
    for b in Brackets::of(cx) {
        let color = cx.doc.layer_color(b.id);
        for which in Bracket::ALL {
            let width = if hot == Some((b.id, which)) { HOT_WIDTH } else { WIDTH };
            let mut path = vectorcraft_geom::kurbo::BezPath::new();
            for (a, c) in b.lines(which, cx.zoom) {
                path.move_to(a);
                path.line_to(c);
            }
            out.push(Overlay::Path { path, color, width, dashed: false });
        }
        let width = if hot == Some((b.id, Bracket::Centre)) { HOT_WIDTH } else { WIDTH };
        out.push(Overlay::Path { path: b.handle_outline(cx.zoom), color, width, dashed: false });
    }
    if let Some(d) = drag
        && let Some((start, flip)) = d.landing
    {
        let color = cx.doc.layer_color(d.brackets.id);
        out.push(Overlay::Path { path: d.brackets.ghost(start, flip, cx.zoom), color, width: if flip { HOT_WIDTH } else { WIDTH }, dashed: !flip });
    }
    out
}

/// Dragging a bracket of type on a path.
#[derive(Clone, Debug)]
pub struct BracketDrag {
    brackets: Brackets,
    which: Bracket,
    start: Point,
    /// Where the press was along the path (a fraction of its length).
    grab: f64,
    began: bool,
    /// Where a centre-bracket drag lands the type: its start along the path, and whether it flips.
    landing: Option<(f64, bool)>,
}

impl BracketDrag {
    /// Start a drag when the press `ev` is on a bracket of the selected type on a path.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        if too_many(cx) {
            return None;
        }
        if !over_bracket(cx, ev.pos) {
            return None;
        }
        let (brackets, which, _) = bracket_at(cx, ev.pos)?;
        let grab = brackets.fraction_at(ev.pos)?;
        Some(Self { brackets, which, start: ev.pos, grab, began: false, landing: None })
    }

    pub fn drag(&mut self, cx: &ToolContext, p: Point, cmd: bool) -> Vec<Action> {
        let mut out = vec![];
        if !self.began {
            if p.distance(self.start) < cx.tol(3.0) {
                return out;
            }
            self.began = true;
            out.push(Action::Begin("Move Type on a Path".into()));
        }
        let Some(u) = self.brackets.fraction_at(p) else { return out };
        let params = self.params(cx, p, u, cmd);
        if self.which == Bracket::Centre {
            self.landing = params["start"].as_f64().map(|s| (s, params.get("flip").is_some()));
        }
        out.push(Action::Preview("type.pathOptions".into(), params));
        out
    }

    /// `type.pathOptions` params for the pointer at `p`, `u` along the path.
    fn params(&self, cx: &ToolContext, p: Point, u: f64, cmd: bool) -> Value {
        let b = &self.brackets;
        let closed = b.arc.is_closed();
        let (s, e) = b.span;
        let mut m = Map::new();
        m.insert("ids".into(), json!([b.id.0]));
        match self.which {
            // Open paths keep the start before the end.
            Bracket::Start => {
                m.insert("start".into(), json!(if closed { u } else { u.min(e) }));
            }
            Bracket::End => {
                m.insert("end".into(), json!(if closed { u } else { u.max(s) }));
            }
            Bracket::Centre => {
                let mut d = u - self.grab;
                if closed {
                    // The short way round.
                    d -= d.round();
                }
                let place = |x: f64| if closed { x.rem_euclid(1.0) } else { x.clamp(0.0, 1.0) };
                let start = place(s + d);
                m.insert("start".into(), json!(start));
                if b.end_set {
                    m.insert("end".into(), json!(if closed { place(e + d) } else { place(e + d).max(start) }));
                }
                // Across the path: the type flips to the other side.
                let (q, _, up) = b.frame(u);
                if !cmd && (p - q).dot(up) < -cx.tol(FLIP_PX) {
                    m.insert("flip".into(), json!(true));
                }
            }
        }
        Value::Object(m)
    }

    /// The drag has left the click: the bracket is moving along the path.
    pub fn adjusting(&self) -> bool {
        self.began
    }

    pub fn finish(self) -> Vec<Action> {
        if self.began { vec![Action::Commit] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use crate::{Cursor, Mods, PointerKind, Tool};
    use vectorcraft_doc::{Document, Selection};

    fn selected(id: NodeId) -> Selection {
        let mut s = Selection::default();
        s.add(id);
        s
    }

    fn preview(a: &[Action]) -> &Value {
        let Some(Action::Preview(c, v)) = a.last() else { panic!("no preview: {a:?}") };
        assert_eq!(c, "type.pathOptions");
        v
    }

    fn drag(cx: &ToolContext, from: Point, to: Point, mods: Mods) -> Vec<Action> {
        let mut d = BracketDrag::hit(cx, &PointerEvent::new(PointerKind::Down, from.x, from.y).with_mods(mods)).expect("a bracket");
        let a = d.drag(cx, to, mods.cmd);
        assert_eq!(a.first(), Some(&Action::Begin("Move Type on a Path".into())));
        assert_eq!(d.finish(), vec![Action::Commit]);
        a
    }

    #[test]
    fn brackets_stand_at_the_start_the_middle_and_the_end_of_the_span() {
        // A 300 pt path from (100, 300) with the type from 20 % on.
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let b = Brackets::of(&cx).pop().unwrap();
        let which = |p: Point| b.hit(p, 1.0, 2.0).map(|(w, _)| w);
        assert_eq!(which(Point::new(160.0, 290.0)), Some(Bracket::Start));
        assert_eq!(which(Point::new(280.0, 290.0)), Some(Bracket::Centre));
        assert_eq!(which(Point::new(400.0, 290.0)), Some(Bracket::End));
        assert_eq!(which(Point::new(220.0, 290.0)), None);
        // Drawn over the type's side of the path, 22 px up and 6 px down, with feet pointing into
        // the span: the start one ends in an arrowhead, the end one in a bar; the centre bracket
        // carries a square handle at the top of its stem.
        let o = overlays(&cx, None, None);
        assert_eq!(o.len(), 4);
        let color = d.layer_color(id);
        let segments = |o: &Overlay| match o {
            Overlay::Path { path, width, .. } => {
                assert_eq!(*width, WIDTH);
                path.segments()
                    .map(|s| {
                        let l = vectorcraft_geom::kurbo::ParamCurve::subsegment(&s, 0.0..1.0);
                        (vectorcraft_geom::kurbo::ParamCurve::start(&l), vectorcraft_geom::kurbo::ParamCurve::end(&l))
                    })
                    .collect::<Vec<_>>()
            }
            _ => vec![],
        };
        let all: Vec<_> = o.iter().flat_map(segments).collect();
        assert!(all.contains(&(Point::new(160.0, 306.0), Point::new(160.0, 278.0))), "{all:?}");
        assert!(all.contains(&(Point::new(160.0, 278.0), Point::new(167.0, 278.0))), "{all:?}");
        assert!(all.contains(&(Point::new(167.0, 278.0), Point::new(164.0, 275.0))), "the start foot's arrowhead: {all:?}");
        assert!(all.contains(&(Point::new(400.0, 278.0), Point::new(393.0, 278.0))), "{all:?}");
        assert!(all.contains(&(Point::new(393.0, 275.0), Point::new(393.0, 281.0))), "the end foot's bar: {all:?}");
        assert!(all.contains(&(Point::new(283.5, 274.5), Point::new(276.5, 274.5))), "the centre handle: {all:?}");
        assert!(o.iter().all(|o| matches!(o, Overlay::Path { color: c, dashed: false, .. } if *c == color)));
        // Nothing for unselected or other type.
        assert!(Brackets::of(&crate::testutil::cx(&d, &Selection::default(), &p)).is_empty());
        let (d2, area) = doc_with_area_type();
        let s2 = selected(area);
        assert!(Brackets::of(&crate::testutil::cx(&d2, &s2, &p)).is_empty());
    }

    /// 10,000 selected type on a path objects keep their brackets (100,000 points); with one more,
    /// the Selection and Direct Selection tools neither draw them nor start a drag or show the
    /// bracket cursor over them.
    #[test]
    fn more_than_ten_thousand_type_on_a_path_objects_show_no_brackets() {
        let (mut d, id) = doc_with_path_type();
        let l = d.layers[0].id;
        let kind = d.node(id).unwrap().kind.clone();
        let mut s = selected(id);
        let add = |d: &mut Document, s: &mut Selection, n: usize| {
            for _ in 0..n {
                let copy = d.alloc_id();
                d.insert(Some(l), usize::MAX, vectorcraft_doc::Node::new(copy, kind.clone())).unwrap();
                s.add(copy);
            }
        };
        add(&mut d, &mut s, 9_999);
        let p = paint();
        // On the start bracket of every copy (they all lie on the same path).
        let at = Point::new(160.0, 290.0);
        let press = PointerEvent::new(PointerKind::Down, at.x, at.y);
        let lines = |o: &[Overlay]| o.iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
        let tools = |cx: &ToolContext| -> Vec<(usize, Cursor)> {
            let select = crate::select::SelectionTool::default();
            let direct = crate::direct::DirectSelectionTool::new(false);
            let tools: [&dyn Tool; 2] = [&select, &direct];
            tools.iter().map(|t| (lines(&t.overlays(cx)), t.cursor(cx, at, Mods::default()))).collect()
        };
        {
            let cx = cx(&d, &s, &p);
            assert_eq!(overlays(&cx, None, None).len(), 40_000);
            assert!(over_bracket(&cx, at) && BracketDrag::hit(&cx, &press).is_some());
            assert_eq!(tools(&cx), vec![(40_000, Cursor::PathBracket); 2]);
        }
        add(&mut d, &mut s, 1);
        let cx = cx(&d, &s, &p);
        assert!(overlays(&cx, None, None).is_empty());
        assert!(!over_bracket(&cx, at) && BracketDrag::hit(&cx, &press).is_none());
        for (n, cursor) in tools(&cx) {
            assert_eq!(n, 0);
            assert_ne!(cursor, Cursor::PathBracket);
        }
    }

    #[test]
    fn dragging_the_centre_bracket_slides_the_type_and_across_the_path_flips_it() {
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let grab = Point::new(280.0, 295.0);
        // 60 pt along: 20 % further, the end (the end of the path) left alone.
        let v = preview(&drag(&cx, grab, Point::new(340.0, 295.0), Mods::default())).clone();
        assert!((v["start"].as_f64().unwrap() - 0.4).abs() < 1e-6, "{v}");
        assert!(v.get("end").is_none() && v.get("flip").is_none(), "{v}");
        assert_eq!(v["ids"], json!([id.0]));
        // Below the path: flipped as well.
        let v = preview(&drag(&cx, grab, Point::new(250.0, 320.0), Mods::default())).clone();
        assert!((v["start"].as_f64().unwrap() - 0.1).abs() < 1e-6 && v["flip"] == true, "{v}");
        // Cmd/Ctrl held: it only slides.
        let v = preview(&drag(&cx, grab, Point::new(250.0, 320.0), Mods { cmd: true, ..Mods::default() })).clone();
        assert!(v.get("flip").is_none(), "{v}");
    }

    #[test]
    fn dragging_the_start_and_end_brackets_moves_them() {
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let v = preview(&drag(&cx, Point::new(160.0, 295.0), Point::new(130.0, 280.0), Mods::default())).clone();
        assert!((v["start"].as_f64().unwrap() - 0.1).abs() < 1e-6 && v.get("end").is_none(), "{v}");
        let v = preview(&drag(&cx, Point::new(400.0, 295.0), Point::new(340.0, 310.0), Mods::default())).clone();
        assert!((v["end"].as_f64().unwrap() - 0.8).abs() < 1e-6 && v.get("start").is_none(), "{v}");
        // The end stops at the start.
        let v = preview(&drag(&cx, Point::new(400.0, 295.0), Point::new(110.0, 300.0), Mods::default())).clone();
        assert!((v["end"].as_f64().unwrap() - 0.2).abs() < 1e-6, "{v}");
    }

    /// The brackets take a press within 8 screen px at any zoom, and not further: their size and
    /// reach are in screen pixels.
    #[test]
    fn brackets_are_hit_within_eight_screen_pixels_at_any_zoom() {
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        for zoom in [0.25, 1.0, 8.0] {
            let mut cx = cx(&d, &s, &p);
            cx.zoom = zoom;
            let px = 1.0 / zoom;
            // The start bracket's stem stands at x 160 from 6 px below the path to 22 px above it.
            let near = |dx: f64, dy: f64| over_bracket(&cx, Point::new(160.0 - dx * px, 300.0 - dy * px));
            assert!(near(7.5, 10.0), "7.5 px beside the stem at zoom {zoom}");
            assert!(near(0.0, 21.0) && near(0.0, -5.0), "the stem's length at zoom {zoom}");
            assert!(!near(9.0, 10.0), "9 px beside the stem at zoom {zoom}");
            // The centre handle, 22 px above the path at x 280.
            let handle = Point::new(280.0, 300.0 - 22.0 * px);
            assert!(over_bracket(&cx, handle + Vec2::new(10.0 * px, 0.0)), "beside the handle at zoom {zoom}");
            let d = BracketDrag::hit(&cx, &PointerEvent::new(PointerKind::Down, handle.x, handle.y)).expect("the handle");
            assert_eq!(d.which, Bracket::Centre);
        }
    }

    /// The bracket under the pointer draws thicker, and while the centre bracket is dragged a dashed
    /// line beside the path shows where the type lands, solid on the far side once it will flip.
    #[test]
    fn the_hovered_bracket_is_thicker_and_a_drag_shows_where_the_type_lands() {
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let widths =
            |o: &[Overlay]| o.iter().filter_map(|o| if let Overlay::Path { width, .. } = o { Some(*width) } else { None }).collect::<Vec<_>>();
        assert_eq!(widths(&overlays(&cx, Some(Point::new(400.0, 290.0)), None)), [WIDTH, WIDTH, HOT_WIDTH, WIDTH]);
        assert_eq!(widths(&overlays(&cx, Some(Point::new(220.0, 250.0)), None)), [WIDTH; 4]);
        let mut d = BracketDrag::hit(&cx, &PointerEvent::new(PointerKind::Down, 280.0, 295.0)).unwrap();
        d.drag(&cx, Point::new(320.0, 295.0), false);
        let o = overlays(&cx, None, Some(&d));
        assert_eq!(widths(&o), [HOT_WIDTH, WIDTH, WIDTH, HOT_WIDTH, WIDTH], "the centre bracket and its handle stay hot");
        let Some(Overlay::Path { path, dashed: true, .. }) = o.last() else { panic!("a dashed landing line: {o:?}") };
        let b = vectorcraft_geom::kurbo::Shape::bounding_box(path);
        assert!((b.x0 - 200.0).abs() < 1e-6 && (b.x1 - 400.0).abs() < 1e-6 && (b.y0 - 290.0).abs() < 1e-6, "{b:?}");
        d.drag(&cx, Point::new(320.0, 330.0), false);
        let o = overlays(&cx, None, Some(&d));
        let Some(Overlay::Path { path, dashed: false, width, .. }) = o.last() else { panic!("a solid landing line: {o:?}") };
        assert_eq!(*width, HOT_WIDTH);
        assert!(vectorcraft_geom::kurbo::Shape::bounding_box(path).y0 > 300.0, "below the path: it flips");
    }

    /// A press on a bracket takes it rather than moving the type, and of a bracket and a text port
    /// side by side the nearer one takes the press, with the cursor to match.
    #[test]
    fn a_bracket_beats_moving_the_object_and_the_nearer_of_a_bracket_and_a_port_wins() {
        let (d, id) = doc_with_path_type();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let press = |at: Point| {
            let mut t = crate::select::SelectionTool::default();
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
            let out = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, at.x + 30.0, at.y));
            (out, t.cursor(&cx, at, Mods::default()))
        };
        // On the centre bracket, over the type itself: the type slides along its path.
        let (out, _) = press(Point::new(280.0, 296.0));
        assert!(out.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "type.pathOptions")), "{out:?}");
        assert!(!out.iter().any(|a| matches!(a, Action::Preview(c, _) if c == "object.move" || c == "transform.move")), "{out:?}");
        // The in port sits beside the start bracket: find its centre.
        let mut port = None;
        for i in 0..200 {
            for j in 0..80 {
                let q = Point::new(130.0 + i as f64 * 0.2, 280.0 + j as f64 * 0.5);
                if let Some((_, crate::text::Port::In, dist)) = crate::text::port_near(&cx, q)
                    && port.is_none_or(|(_, best)| dist < best)
                {
                    port = Some((q, dist));
                }
            }
        }
        let (centre, _) = port.expect("an in port near the start bracket");
        assert!(!over_bracket(&cx, centre), "on the port: the port is nearer");
        assert!(BracketDrag::hit(&cx, &PointerEvent::new(PointerKind::Down, centre.x, centre.y)).is_none());
        // On the stem, within the port's box if it reaches that far: the bracket is nearer.
        let stem = Point::new(160.0, centre.y.clamp(279.0, 305.0));
        assert!(over_bracket(&cx, stem));
        assert_eq!(press(stem).1, Cursor::PathBracket);
    }

    /// On a closed circle, a drag of the centre bracket slides the type round its path, flipping it
    /// across the path: it changes the type's place only, never its path nor moves the object.
    #[test]
    fn on_a_circle_the_centre_bracket_moves_the_type_only() {
        let (mut d, _) = crate::testutil::doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut t = vectorcraft_doc::TextObject::point(Point::ZERO, "Round", Default::default());
        let circle = vectorcraft_geom::shapes::ellipse(vectorcraft_geom::Rect::new(200.0, 200.0, 400.0, 400.0));
        t.kind = TextKind::OnPath { path: circle, start: 0.1, end: None };
        d.insert(Some(l), 1, vectorcraft_doc::Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let b = Brackets::of(&cx).pop().unwrap();
        let (on, _, up) = b.frame(b.at(Bracket::Centre));
        for (to, flips) in [(on + up * 5.0, false), (on - up * 40.0, true)] {
            let mut t = crate::select::SelectionTool::default();
            let grab = on + up * 10.0;
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, grab.x, grab.y));
            let to = to + b.frame(b.at(Bracket::Centre)).1 * 30.0;
            let out = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, to.x, to.y));
            let previews: Vec<_> = out.iter().filter_map(|a| if let Action::Preview(c, v) = a { Some((c.as_str(), v)) } else { None }).collect();
            assert_eq!(previews.len(), 1, "{out:?}");
            let (cmd, v) = previews[0];
            assert_eq!(cmd, "type.pathOptions");
            assert_eq!(v["ids"], json!([id.0]));
            let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
            assert!(keys.iter().all(|k| ["ids", "start", "end", "flip"].contains(&k.as_str())), "only the type's place: {keys:?}");
            assert_eq!(v.get("flip").is_some(), flips, "{v}");
            assert!(out.contains(&Action::Begin("Move Type on a Path".into())));
        }
    }

    /// For type read right to left the arrowhead marks where it begins, the end bracket, and the
    /// stop bar the start bracket; the feet still point into the span.
    #[test]
    fn right_to_left_type_has_its_arrowhead_on_the_end_bracket() {
        let (mut d, id) = doc_with_path_type();
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            t.para.direction = Some(vectorcraft_doc::ParaDirection::RightToLeft);
        }
        let s = selected(id);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let b = Brackets::of(&cx).pop().unwrap();
        assert!(b.rtl);
        // The start bracket at x 160: its foot runs right to x 167 and ends in a bar.
        assert_eq!(
            b.lines(Bracket::Start, 1.0),
            vec![
                (Point::new(160.0, 306.0), Point::new(160.0, 278.0)),
                (Point::new(160.0, 278.0), Point::new(167.0, 278.0)),
                (Point::new(167.0, 275.0), Point::new(167.0, 281.0)),
            ]
        );
        // The end bracket at x 400: its foot runs left to x 393 and ends in an arrowhead pointing left.
        let end = b.lines(Bracket::End, 1.0);
        assert_eq!(end.len(), 4);
        assert!(end.contains(&(Point::new(393.0, 278.0), Point::new(396.0, 275.0))), "{end:?}");
        // Arabic text with no direction set reads right to left too.
        if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
            let mut arabic = vectorcraft_doc::TextObject::point(Point::ZERO, "سكة", Default::default());
            arabic.kind = t.kind.clone();
            **t = arabic;
        }
        let cx = crate::testutil::cx(&d, &s, &p);
        assert!(Brackets::of(&cx).pop().unwrap().rtl);
    }
}
