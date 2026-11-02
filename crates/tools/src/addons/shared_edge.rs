//! Shared Edge Merge Tool (Path Precision Tools): where two paths, selected or not, touch at two
//! coincident anchors, hovering with the round brush shows the two stretches between them (dashed)
//! and the edge merging them (solid, green when they are merged already), and a click replaces both
//! with that edge, half way between them (`addon.pathtools.mergeEdge`, one undo step). Shift follows
//! the upper path, Alt the lower one. Where the anchors aren't there yet, a click makes one end:
//! near anchors meet half way, and an anchor one path lacks is added to match. Shift+Alt asks for
//! an end inside a stretch that could merge.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{BezPath, Point};
use vectorcraft_pathops::addons::pathtools::{Follow, MergePlan, MergePreview, ends_at, merge_preview, stretches_at};

use crate::builder::HIGHLIGHT_RED;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub use vectorcraft_pathops::addons::pathtools::{COINCIDENCE, MAX_COINCIDENCE};

/// The tool's id.
pub const ID: &str = "sharedEdge";
/// The brush's diameter by default and its range (pt).
pub const DIAMETER: f64 = 20.0;
pub const DIAMETER_RANGE: (f64, f64) = (0.25, 200.0);
/// Most paths under the brush compared with one another.
const MAX_PATHS: usize = 6;
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];
const GREEN: [u8; 3] = [0x2e, 0xb8, 0x6a];
const GREY: [u8; 3] = [0x80, 0x80, 0x80];
const EDGE_PX: f32 = 2.0;
const ANCHOR_PX: f32 = 7.0;

/// What the brush is over.
#[derive(Clone, Debug, PartialEq)]
struct Target {
    /// The upper path.
    a: NodeId,
    /// The lower path.
    b: NodeId,
    at: Point,
    follow: Follow,
    /// Shift+Alt: an end, even where the stretch could merge.
    ends: bool,
    preview: MergePreview,
    /// The document it was found in.
    revision: (u64, u64),
}

pub struct SharedEdgeTool {
    diameter: f64,
    tolerance: f64,
    together: bool,
    at: Option<Point>,
    target: Option<Target>,
    pressed: bool,
}

impl Default for SharedEdgeTool {
    fn default() -> Self {
        Self { diameter: DIAMETER, tolerance: COINCIDENCE, together: false, at: None, target: None, pressed: false }
    }
}

/// The editable paths the brush touches, nearest first: (paint order, id).
fn paths_under(cx: &ToolContext, at: Point, radius: f64) -> Vec<(usize, NodeId)> {
    let editable = cx.doc.editable();
    let mut order = 0;
    let mut out: Vec<(f64, usize, NodeId)> = vec![];
    cx.doc.walk(|n| {
        order += 1;
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        if path.control_bounds().is_none_or(|r| !r.inflate(radius, radius).contains(at)) {
            return;
        }
        let d = path.nearest(at).map_or(f64::INFINITY, |(.., q, _)| q.distance(at));
        if d <= radius && editable(n.id) {
            out.push((d, order, n.id));
        }
    });
    out.sort_by(|x, y| x.0.total_cmp(&y.0));
    out.truncate(MAX_PATHS);
    out.into_iter().map(|(_, order, id)| (order, id)).collect()
}

impl SharedEdgeTool {
    /// What a click at `at` with `mods` would do, on the nearest paths that it can.
    fn look(&self, cx: &ToolContext, at: Point, mods: Mods) -> Option<Target> {
        let radius = self.diameter / 2.0;
        let path = |id: NodeId| cx.doc.node(id).and_then(|n| n.path_data());
        let under = paths_under(cx, at, radius);
        let mut pairs: Vec<(NodeId, NodeId)> = vec![];
        for (i, x) in under.iter().enumerate() {
            pairs.extend(under.iter().skip(i + 1).map(|y| if x.0 > y.0 { (x.1, y.1) } else { (y.1, x.1) }));
        }
        let find = |f: &dyn Fn(NodeId, NodeId) -> Option<MergePlan>| pairs.iter().find_map(|&(a, b)| Some((a, b, f(a, b)?)));
        let ends = mods.shift && mods.alt;
        let edge = if ends { None } else { find(&|a, b| stretches_at(path(a)?, path(b)?, at, radius, self.tolerance)) };
        let (a, b, plan) = edge.or_else(|| find(&|a, b| ends_at(path(a)?, path(b)?, at, radius)))?;
        let follow = match (mods.shift, mods.alt) {
            (true, false) => Follow::First,
            (false, true) => Follow::Second,
            _ => Follow::Average,
        };
        let preview = merge_preview(path(a)?, path(b)?, &plan, follow)?;
        Some(Target { a, b, at, follow, ends, preview, revision: cx.revision })
    }

    fn params(&self, t: &Target) -> Value {
        json!({
            "a": t.a.0,
            "b": t.b.0,
            "at": [t.at.x, t.at.y],
            "diameter": self.diameter,
            "tolerance": self.tolerance,
            "follow": t.follow.id(),
            "ends": t.ends,
        })
    }
}

fn outline(chain: &[kurbo::CubicBez]) -> BezPath {
    let mut path = BezPath::new();
    if let Some(c) = chain.first() {
        path.move_to(c.p0);
    }
    for c in chain {
        path.curve_to(c.p1, c.p2, c.p3);
    }
    path
}

impl Tool for SharedEdgeTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.pressed
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.at = Some(ev.pos);
        self.target = self.look(cx, ev.pos, ev.mods);
        match ev.kind {
            PointerKind::Down => self.pressed = true,
            PointerKind::Up if std::mem::take(&mut self.pressed) => {
                let target = self.target.take();
                let todo = target.filter(|t| !matches!(t.preview, MergePreview::Edge { done: true, .. }));
                return todo.map_or_else(Vec::new, |t| vec![Action::Exec("addon.pathtools.mergeEdge".into(), self.params(&t))]);
            }
            _ => {}
        }
        vec![]
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.pressed = false;
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.pressed && key == ToolKey::Escape
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.at = None;
        self.target = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(at) = self.at else { return vec![] };
        let r = self.diameter / 2.0;
        let ring = kurbo::Shape::to_path(&kurbo::Circle::new(at, r), 0.1 * r);
        let mut out = vec![Overlay::Path { path: ring, color: GREY, width: 1.0, dashed: false }];
        let curve = |chain: &[kurbo::CubicBez], color, dashed| Overlay::Path { path: outline(chain), color, width: EDGE_PX, dashed };
        let anchor = |p, color, filled| Overlay::Anchor { p, color, filled, size: ANCHOR_PX };
        match self.target.as_ref().filter(|t| t.revision == cx.revision).map(|t| &t.preview) {
            Some(MergePreview::Edge { merged, done: true, .. }) => out.push(curve(merged, GREEN, false)),
            Some(MergePreview::Edge { a, b, merged, .. }) => {
                out.extend([curve(a, BLUE, true), curve(b, BLUE, true), curve(merged, HIGHLIGHT_RED, false)])
            }
            Some(MergePreview::Ends { a, b, to }) => out.extend([anchor(*a, BLUE, false), anchor(*b, BLUE, false), anchor(*to, HIGHLIGHT_RED, true)]),
            None => {}
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::SharedEdge
    }

    fn options(&self) -> Value {
        json!({ "diameter": self.diameter, "tolerance": self.tolerance, "together": self.together })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "diameter" => {
                if let Some(d) = v.as_f64().filter(|d| (DIAMETER_RANGE.0..=DIAMETER_RANGE.1).contains(d)) {
                    self.diameter = d;
                }
            }
            "tolerance" => {
                if let Some(t) = v.as_f64().filter(|t| (0.0..=MAX_COINCIDENCE).contains(t)) {
                    self.tolerance = t;
                }
            }
            "together" => {
                if let Some(on) = v.as_bool() {
                    self.together = on;
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::{PathData, SubPath};

    /// A lower and an upper square sharing the edge x = 100 from y = 0 to y = 100; `gap` pushes the
    /// upper one right. → (the document, the lower one, the upper one).
    fn doc_with_squares(gap: f64) -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (lower, upper) = (d.alloc_id(), d.alloc_id());
        let square = |x: f64| [(x, 0.0), (x + 100.0, 0.0), (x + 100.0, 100.0), (x, 100.0)].map(|(x, y)| Point::new(x, y));
        for (i, (id, x)) in [(lower, 0.0), (upper, 100.0 + gap)].into_iter().enumerate() {
            d.insert(Some(l), i, Node::path(id, PathData::single(SubPath::polyline(&square(x), true)), Appearance::default_art())).unwrap();
        }
        (d, lower, upper)
    }

    fn ev(kind: PointerKind, x: f64, y: f64, shift: bool, alt: bool) -> PointerEvent {
        let mut e = PointerEvent::new(kind, x, y);
        (e.mods.shift, e.mods.alt) = (shift, alt);
        e
    }

    /// The params of a click at (x, y).
    fn click(t: &mut SharedEdgeTool, cx: &ToolContext, x: f64, y: f64, shift: bool, alt: bool) -> Option<Value> {
        t.pointer(cx, &ev(PointerKind::Down, x, y, shift, alt));
        match t.pointer(cx, &ev(PointerKind::Up, x, y, shift, alt)).as_slice() {
            [Action::Exec(cmd, p)] => {
                assert_eq!(cmd, "addon.pathtools.mergeEdge");
                Some(p.clone())
            }
            [] => None,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_edge_merged_already_shows_green_and_a_click_leaves_it() {
        let (d, ..) = doc_with_squares(0.0);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SharedEdgeTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Move, 100.0, 50.0, false, false)).is_empty());
        // They are one edge already: it shows green and a click has nothing to do.
        let shows = t.overlays(&cx);
        assert!(matches!(shows.as_slice(), [Overlay::Path { color: GREY, .. }, Overlay::Path { color: GREEN, dashed: false, .. }]), "{shows:?}");
        assert_eq!(click(&mut t, &cx, 100.0, 50.0, false, false), None);
        assert!(!t.busy());
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::SharedEdge);
    }

    #[test]
    fn a_click_averages_the_stretches_and_shift_and_alt_follow_a_path() {
        // The stretches differ, their ends coincident within the tolerance set.
        let (d, lower, upper) = doc_with_squares(1.0);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SharedEdgeTool::default();
        t.set_option("tolerance", &json!(3));
        t.pointer(&cx, &ev(PointerKind::Move, 100.5, 50.0, false, false));
        let shows = t.overlays(&cx);
        assert!(
            matches!(
                shows.as_slice(),
                [
                    _,
                    Overlay::Path { color: BLUE, dashed: true, .. },
                    Overlay::Path { color: BLUE, dashed: true, .. },
                    Overlay::Path { dashed: false, .. }
                ]
            ),
            "{shows:?}"
        );
        let plain = click(&mut t, &cx, 100.5, 50.0, false, false).expect("a merge");
        assert_eq!((plain["a"].clone(), plain["b"].clone(), plain["follow"].clone()), (json!(upper.0), json!(lower.0), json!("average")));
        assert_eq!((plain["at"].clone(), plain["ends"].clone(), plain["tolerance"].clone()), (json!([100.5, 50.0]), json!(false), json!(3.0)));
        assert_eq!(click(&mut t, &cx, 100.5, 50.0, true, false).expect("a merge")["follow"], json!("a"));
        assert_eq!(click(&mut t, &cx, 100.5, 50.0, false, true).expect("a merge")["follow"], json!("b"));
        // Shift+Alt: an end, inside the stretch.
        let end = click(&mut t, &cx, 100.5, 2.0, true, true).expect("an end");
        assert_eq!((end["ends"].clone(), end["follow"].clone()), (json!(true), json!("average")));
    }

    #[test]
    fn near_anchors_make_an_end_and_nothing_near_does_nothing() {
        let (d, ..) = doc_with_squares(5.0);
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SharedEdgeTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 102.0, 1.0, false, false));
        let shows = t.overlays(&cx);
        assert!(matches!(shows.as_slice(), [_, Overlay::Anchor { .. }, Overlay::Anchor { .. }, Overlay::Anchor { filled: true, .. }]), "{shows:?}");
        assert_eq!(click(&mut t, &cx, 102.0, 1.0, false, false).expect("an end")["ends"], json!(false));
        // One path alone, and empty canvas.
        assert_eq!(click(&mut t, &cx, 0.0, 50.0, false, false), None);
        assert_eq!(click(&mut t, &cx, 300.0, 300.0, false, false), None);
    }

    #[test]
    fn locked_paths_are_left_alone_and_escape_drops_a_press() {
        let (mut d, _, upper) = doc_with_squares(5.0);
        d.node_mut(upper).unwrap().locked = true;
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = SharedEdgeTool::default();
        assert_eq!(click(&mut t, &cx, 102.0, 1.0, false, false), None);
        t.pointer(&cx, &ev(PointerKind::Down, 102.0, 1.0, false, false));
        assert!(t.busy() && t.claims_key(&cx, ToolKey::Escape));
        assert!(t.key(&cx, ToolKey::Escape, Mods::default()).is_empty());
        assert!(!t.busy());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 102.0, 1.0, false, false)).is_empty());
    }

    #[test]
    fn options_stay_in_range() {
        let mut t = SharedEdgeTool::default();
        for (k, v) in
            [("diameter", json!(1000)), ("diameter", json!(0.1)), ("tolerance", json!(4)), ("tolerance", json!("x")), ("together", json!(1))]
        {
            t.set_option(k, &v);
        }
        assert_eq!(t.options(), json!({ "diameter": DIAMETER, "tolerance": COINCIDENCE, "together": false }));
        t.set_option("diameter", &json!(40));
        t.set_option("tolerance", &json!(0));
        t.set_option("together", &json!(true));
        assert_eq!(t.options(), json!({ "diameter": 40.0, "tolerance": 0.0, "together": true }));
    }
}
