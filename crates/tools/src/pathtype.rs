//! Type on a path's brackets: selected type on a path shows a start, a centre and an end bracket
//! with the Selection and Direct Selection tools. Dragging the start or end bracket sets where the
//! type begins or ends along its path; dragging the centre bracket slides the type along the path
//! (its brackets with it) and, dragged across the path, flips the type to the other side
//! (Cmd/Ctrl held: it only slides). Each drag is one undo step of `type.pathOptions`. The tools
//! draw the brackets from the same geometry they hit-test. With more than 10,000 type on a path
//! objects selected, they neither draw nor hit-test any bracket.

use serde_json::{Map, Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextKind};
use vectorcraft_geom::kurbo::{Line, ParamCurveNearest};
use vectorcraft_geom::{Affine, ArcPath, Point, Vec2};

use crate::{Action, Overlay, PointerEvent, ToolContext};

/// How far a bracket reaches (screen px): up from the path on the type's side, down across it,
/// and the foot of the start and end brackets along the path, into the type's span.
const UP_PX: f64 = 14.0;
const DOWN_PX: f64 = 4.0;
const FOOT_PX: f64 = 4.0;
/// How far past the path (screen px) the centre bracket goes before the type flips.
const FLIP_PX: f64 = 6.0;
/// The most points the tools draw for the brackets of a selection, the same number as the canvas's
/// limit on the anchor points it outlines (`OVERLAY_MAX_ANCHORS` in `crates/ui-egui/src/canvas.rs`).
/// Past it the tools neither draw nor hit-test any bracket.
const MAX_POINTS: usize = 100_000;
/// The points the tools draw for one type on a path's brackets: the two ends of each of the five
/// lines.
const POINTS_EACH: usize = 10;

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
        Some(Self { id, xf: t.xf, arc, span: t.kind.path_span()?, end_set: end.is_some() })
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

    /// The lines that draw bracket `b` at `zoom` (screen px per point).
    fn lines(&self, b: Bracket, zoom: f64) -> Vec<(Point, Point)> {
        let px = 1.0 / zoom.max(1e-9);
        let (p, dir, up) = self.frame(self.at(b));
        let top = p + up * UP_PX * px;
        let stem = (p - up * DOWN_PX * px, top);
        match b {
            Bracket::Centre => vec![stem],
            Bracket::Start => vec![stem, (top, top + dir * FOOT_PX * px)],
            Bracket::End => vec![stem, (top, top - dir * FOOT_PX * px)],
        }
    }

    /// The bracket within `tol` (document units) of `p`.
    fn hit(&self, p: Point, zoom: f64, tol: f64) -> Option<Bracket> {
        Bracket::ALL.into_iter().find(|b| self.lines(*b, zoom).iter().any(|(a, c)| Line::new(*a, *c).nearest(p, 1e-9).distance_sq <= tol * tol))
    }

    /// The fraction of the path's length nearest document point `p`.
    fn fraction_at(&self, p: Point) -> Option<f64> {
        self.arc.fraction_at(self.xf.inverse() * p)
    }
}

/// Whether the selection holds more than [`MAX_POINTS`] / [`POINTS_EACH`] type on a path objects,
/// in perspective or not (counting stops there).
fn too_many(cx: &ToolContext) -> bool {
    let on_path = |id: &&NodeId| cx.doc.node(**id).is_some_and(|n| matches!(&n.kind, NodeKind::Text(t) if matches!(t.kind, TextKind::OnPath { .. })));
    cx.selection.objects.iter().filter(on_path).nth(MAX_POINTS / POINTS_EACH).is_some()
}

/// The selected type on a path's bracket under `p`, if any.
fn bracket_at(cx: &ToolContext, p: Point) -> Option<(Brackets, Bracket)> {
    Brackets::of(cx).into_iter().find_map(|b| {
        let which = b.hit(p, cx.zoom, cx.tol(5.0))?;
        Some((b, which))
    })
}

/// Is `p` over a bracket of the selected type on a path?
pub fn over_bracket(cx: &ToolContext, p: Point) -> bool {
    !too_many(cx) && bracket_at(cx, p).is_some()
}

/// The brackets of the selected type on a path, in their layer's colour.
pub fn overlays(cx: &ToolContext) -> Vec<Overlay> {
    if too_many(cx) {
        return vec![];
    }
    let mut out = vec![];
    for b in Brackets::of(cx) {
        let color = cx.doc.layer_color(b.id);
        for which in Bracket::ALL {
            out.extend(b.lines(which, cx.zoom).into_iter().map(|(a, b)| Overlay::Line { a, b, color, dashed: false }));
        }
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
}

impl BracketDrag {
    /// Start a drag when the press `ev` is on a bracket of the selected type on a path.
    pub fn hit(cx: &ToolContext, ev: &PointerEvent) -> Option<Self> {
        if too_many(cx) {
            return None;
        }
        let (brackets, which) = bracket_at(cx, ev.pos)?;
        let grab = brackets.fraction_at(ev.pos)?;
        Some(Self { brackets, which, start: ev.pos, grab, began: false })
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
        out.push(Action::Preview("type.pathOptions".into(), self.params(cx, p, u, cmd)));
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
        assert_eq!(b.hit(Point::new(160.0, 290.0), 1.0, 2.0), Some(Bracket::Start));
        assert_eq!(b.hit(Point::new(280.0, 290.0), 1.0, 2.0), Some(Bracket::Centre));
        assert_eq!(b.hit(Point::new(400.0, 290.0), 1.0, 2.0), Some(Bracket::End));
        assert_eq!(b.hit(Point::new(220.0, 290.0), 1.0, 2.0), None);
        // Drawn over the type's side of the path, with feet pointing into the span.
        let o = overlays(&cx);
        assert_eq!(o.len(), 5);
        assert!(o.contains(&Overlay::Line { a: Point::new(160.0, 286.0), b: Point::new(164.0, 286.0), color: d.layer_color(id), dashed: false }));
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
        let lines = |o: &[Overlay]| o.iter().filter(|o| matches!(o, Overlay::Line { .. })).count();
        let tools = |cx: &ToolContext| -> Vec<(usize, Cursor)> {
            let select = crate::select::SelectionTool::default();
            let direct = crate::direct::DirectSelectionTool::new(false);
            let tools: [&dyn Tool; 2] = [&select, &direct];
            tools.iter().map(|t| (lines(&t.overlays(cx)), t.cursor(cx, at, Mods::default()))).collect()
        };
        {
            let cx = cx(&d, &s, &p);
            assert_eq!(overlays(&cx).len(), 50_000);
            assert!(over_bracket(&cx, at) && BracketDrag::hit(&cx, &press).is_some());
            assert_eq!(tools(&cx), vec![(50_000, Cursor::PathBracket); 2]);
        }
        add(&mut d, &mut s, 1);
        let cx = cx(&d, &s, &p);
        assert!(overlays(&cx).is_empty());
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
}
