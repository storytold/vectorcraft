//! Puppet Warp tool.
//!
//! With artwork selected the tool shows pins at once: the ones already placed on it, else
//! automatic pins at the centre and the end of each limb ([`default_pins`]). A click on the artwork
//! adds a pin and selects it; Shift-click adds pins to the selection (or takes them out); dragging
//! a pin moves every selected pin and warps the art; Alt-dragging near (not on) a selected pin
//! turns the art around it, and that pin keeps the turn; Delete/Backspace remove the selected pins.
//! A click off the mesh places nothing and says so. Escape during a drag cancels it: the art, its
//! pins and the selected pins go back to how the press found them.
//!
//! The pins live in the document ([`PuppetPins`]) with the rest shape they started from, so every
//! warp starts from that shape (dragging a pin back restores the original, warps don't stack) and
//! Undo/Redo take the pins back with the art. They stay while the same artwork is selected; a new
//! selection starts afresh. Every change runs `object.puppetWarp {rest: true, ids, pins, moved,
//! angles, expand}` (pins = rest positions, moved = where they are now): adding or deleting pins
//! at once, drags as a preview committed on release, each one undo step.
//!
//! Control bar (`tool.setOption`): `showMesh` (on by default), `expand` (Expand Mesh, points, 0
//! allowed) and `selectAllPins`.

use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, PuppetPin, PuppetPins};
use vectorcraft_geom::{BezPath, Point, Rect};

use super::arap::{self, Mesh, MeshOptions, default_pins};
use super::{BLUE, collect_points, warp_node_with};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Outlines and bounds of the given objects (what the mesh is built over).
pub fn mesh_input(doc: &Document, ids: &[NodeId]) -> Option<(Vec<BezPath>, Rect)> {
    let nodes: Vec<&Node> = ids.iter().map(|id| doc.node(*id)).collect::<Option<_>>()?;
    outlines_of(&nodes)
}

fn outlines_of(nodes: &[&Node]) -> Option<(Vec<BezPath>, Rect)> {
    let mut outlines = vec![];
    let mut pts = vec![];
    for n in nodes {
        collect_points(n, &mut pts);
        n.walk(&mut |c| {
            if let Some(p) = c.path_data() {
                outlines.push(p.to_bezpath());
            }
        });
    }
    let first = *pts.first()?;
    let b = pts.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
    Some((outlines, b))
}

/// The mesh the engine uses for `ids` (shared so tool overlays match the command).
pub fn mesh_for(doc: &Document, ids: &[NodeId], expand: f64) -> Option<Mesh> {
    let (o, b) = mesh_input(doc, ids)?;
    Some(Mesh::build(&o, b, MeshOptions { expand, ..Default::default() }))
}

/// The mesh over `nodes` (a rest shape).
fn mesh_of(nodes: &[Arc<Node>], expand: f64) -> Option<Mesh> {
    let refs: Vec<&Node> = nodes.iter().map(|n| n.as_ref()).collect();
    let (o, b) = outlines_of(&refs)?;
    Some(Mesh::build(&o, b, MeshOptions { expand, ..Default::default() }))
}

/// The solver's view of the pins.
fn solver_pins(pins: &[PuppetPin]) -> Vec<arap::Pin> {
    pins.iter().map(|q| arap::Pin { angle: q.angle, ..arap::Pin::new(q.rest, q.at) }).collect()
}

/// Do the pins leave the art as it is (each where it started, turned by nothing)?
fn at_rest(pins: &[PuppetPin]) -> bool {
    pins.iter().all(|q| q.rest == q.at && q.angle.is_none_or(|a| a == 0.0))
}

/// A rest shape and the pins placed on it (`None`: none yet).
pub type RestPins = (Vec<Arc<Node>>, Option<Vec<PuppetPin>>);

/// The rest shape of `ids` and the pins placed on it: the document's pins when they belong to
/// these objects as the last warp left them; when something else edited them since, the same pins
/// starting again from the objects as they are; `None` pins: none placed yet.
pub fn rest_and_pins(doc: &Document, ids: &[NodeId]) -> Option<RestPins> {
    let current: Vec<Arc<Node>> = ids.iter().map(|id| doc.node_arc(*id).cloned()).collect::<Option<_>>()?;
    Some(match doc.puppet.as_deref().filter(|p| p.ids == ids) {
        Some(p) if p.holds(doc, ids) => (p.rest.clone(), Some(p.pins.clone())),
        Some(p) => (current, Some(p.pins.iter().map(|q| PuppetPin { rest: q.at, at: q.at, angle: q.angle.map(|_| 0.0) }).collect())),
        None => (current, None),
    })
}

/// The Puppet Warp pins in effect on some objects, with the mesh they warp.
pub struct PinSet {
    pub ids: Vec<NodeId>,
    pub pins: Vec<PuppetPin>,
    /// No pin was placed yet: these are the automatic ones.
    pub auto: bool,
    pub mesh: Mesh,
    /// The mesh vertices as the pins warp them.
    pub deformed: Vec<Point>,
    /// The pins warp the art (else `deformed` is the rest mesh).
    warped: bool,
}

impl PinSet {
    /// The pins on `ids` (see [`rest_and_pins`]) with the mesh built with Expand `expand`.
    pub fn of(doc: &Document, ids: &[NodeId], expand: f64) -> Option<Self> {
        let (rest, pins) = rest_and_pins(doc, ids)?;
        let mesh = mesh_of(&rest, expand)?;
        let auto = pins.is_none();
        let pins = pins.unwrap_or_else(|| default_pins(&mesh).into_iter().map(|p| PuppetPin { rest: p, at: p, angle: None }).collect());
        let warped = !at_rest(&pins);
        let deformed = if warped { arap::deform(&mesh, &solver_pins(&pins)) } else { mesh.verts.clone() };
        Some(Self { ids: ids.to_vec(), pins, auto, mesh, deformed, warped })
    }
    /// Where on the rest shape the art now at `q` comes from (None: off the mesh).
    pub fn rest_of(&self, q: Point) -> Option<Point> {
        if self.warped { self.mesh.unmap(&self.deformed, q) } else { self.mesh.contains(q).then_some(q) }
    }
    /// The turn (radians) the art has around pin `i`: the one it holds, else the warp's there.
    pub fn turn_of(&self, i: usize) -> f64 {
        self.pins.get(i).map_or(0.0, |q| q.angle.or_else(|| self.mesh.turn_at(&self.deformed, q.rest)).unwrap_or(0.0))
    }
    /// The `object.puppetWarp` params that set `pins` on these objects.
    pub fn params(&self, pins: &[PuppetPin], expand: f64) -> Value {
        json!({
            "rest": true,
            "ids": crate::json_ids(&self.ids),
            "pins": pins.iter().map(|q| json!([q.rest.x, q.rest.y])).collect::<Vec<_>>(),
            "moved": pins.iter().map(|q| json!([q.at.x, q.at.y])).collect::<Vec<_>>(),
            "angles": pins.iter().map(|q| q.angle.map(f64::to_degrees)).collect::<Vec<_>>(),
            "expand": expand,
        })
    }
    /// The warped mesh as one path (for the overlay).
    fn mesh_path(&self) -> BezPath {
        let mut bp = BezPath::new();
        for t in &self.mesh.tris {
            let [a, b, c] = t.map(|i| self.deformed.get(i).copied().unwrap_or(self.mesh.verts[i]));
            bp.move_to(a);
            bp.line_to(b);
            bp.line_to(c);
            bp.close_path();
        }
        bp
    }
}

/// Put `pins` on `ids`: warp their rest shape (see [`rest_and_pins`]) so each pin lands where it
/// is now, and keep the pins in the document. With no pins the art stays as it is and the pins
/// start again from it.
pub fn warp_from_rest(doc: &mut Document, ids: &[NodeId], pins: Vec<PuppetPin>, expand: f64) -> Result<(), String> {
    let (rest, _) = rest_and_pins(doc, ids).ok_or("nothing to warp")?;
    let rest = if pins.is_empty() {
        ids.iter().map(|id| doc.node_arc(*id).cloned()).collect::<Option<Vec<_>>>().ok_or("nothing to warp")?
    } else {
        let mesh = mesh_of(&rest, expand).ok_or("nothing to warp")?;
        if let Some(i) = pins.iter().position(|q| !mesh.contains(q.rest)) {
            return Err(format!("pin {i} is off the artwork (outside its mesh)"));
        }
        let deformed = arap::deform(&mesh, &solver_pins(&pins));
        let f = |q: Point| mesh.map(&deformed, q);
        for (id, r) in ids.iter().zip(&rest) {
            let mut n = Node::clone(r);
            warp_node_with(&mut n, &f);
            *doc.node_mut(*id).ok_or("the artwork is gone")? = n;
        }
        rest
    };
    let result = ids.iter().map(|id| doc.node_arc(*id).cloned()).collect::<Option<Vec<_>>>().ok_or("the artwork is gone")?;
    doc.puppet = Some(Arc::new(PuppetPins { ids: ids.to_vec(), rest, result, pins }));
    Ok(())
}

/// Within this many pixels of a selected pin (and not on it), Alt-dragging turns the art around it.
const TURN_REACH_PX: f64 = 24.0;
/// Radius (pixels) of the dotted circle shown around the pin the art would turn around.
const TURN_RING_PX: f64 = 16.0;
/// The feedback for a click off the mesh.
const OFF_MESH: &str = "Pins go on the artwork";
const WARN: [u8; 3] = [0xd9, 0x3f, 0x3f];

/// A drag in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Gesture {
    /// The selected pins move by `to - from`.
    Move { from: Point, to: Point },
    /// The art turns around pin `pin`: `base` (radians) is its turn when the drag began, `from`
    /// the pointer's angle around it then, `angle` how far the drag has turned it since.
    Turn { pin: usize, base: f64, from: f64, angle: f64 },
}

/// What the cached pins were computed from (holding the objects keeps the pointers unique).
struct Key {
    ids: Vec<NodeId>,
    nodes: Vec<Arc<Node>>,
    puppet: Option<Arc<PuppetPins>>,
    expand: f64,
}

impl Key {
    fn of(cx: &ToolContext, expand: f64) -> Option<Self> {
        let ids = cx.selection.objects.clone();
        let nodes = ids.iter().map(|id| cx.doc.node_arc(*id).cloned()).collect::<Option<_>>()?;
        Some(Self { ids, nodes, puppet: cx.doc.puppet.clone(), expand })
    }
    fn same(&self, o: &Key) -> bool {
        let ptrs = |a: &[Arc<Node>], b: &[Arc<Node>]| a.len() == b.len() && a.iter().zip(b).all(|(x, y)| Arc::ptr_eq(x, y));
        let puppet = match (&self.puppet, &o.puppet) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (a, b) => a.is_none() && b.is_none(),
        };
        self.ids == o.ids && ptrs(&self.nodes, &o.nodes) && puppet && self.expand == o.expand
    }
}

struct Cached {
    key: Key,
    set: PinSet,
    mesh: BezPath,
}

pub struct PuppetWarpTool {
    /// The objects the selected pins belong to.
    ids: Vec<NodeId>,
    /// Selected pins (indices into the pins).
    selected: Vec<usize>,
    drag: Option<Gesture>,
    /// The selected pins before the press that began the drag (Escape puts them back).
    before_drag: Vec<usize>,
    /// The pins as the drag began (each preview starts from them).
    start: Option<Arc<Cached>>,
    /// The pointer and the modifiers held while hovering (for the turn ring).
    hover: Option<(Point, Mods)>,
    /// Where a click off the mesh was (its feedback shows until the pointer moves on).
    off_mesh: Option<Point>,
    /// Control bar: Show Mesh.
    show_mesh: bool,
    /// Control bar: Expand Mesh (points, 0 allowed).
    expand: f64,
    /// The pins and mesh of the selection, recomputed when the art, its pins or Expand change
    /// (overlays ask every frame).
    cache: Mutex<Option<Arc<Cached>>>,
}

impl Default for PuppetWarpTool {
    fn default() -> Self {
        Self {
            ids: vec![],
            selected: vec![],
            drag: None,
            before_drag: vec![],
            start: None,
            hover: None,
            off_mesh: None,
            show_mesh: true,
            expand: 3.0,
            cache: Mutex::new(None),
        }
    }
}

impl PuppetWarpTool {
    /// The pins on the selection (None: nothing selected).
    fn pins(&self, cx: &ToolContext) -> Option<Arc<Cached>> {
        let key = Key::of(cx, self.expand).filter(|k| !k.ids.is_empty())?;
        let mut cache = self.cache.lock().ok();
        if let Some(c) = cache.as_deref().and_then(Option::as_ref).filter(|c| c.key.same(&key)) {
            return Some(c.clone());
        }
        let set = PinSet::of(cx.doc, &key.ids, self.expand)?;
        let c = Arc::new(Cached { mesh: set.mesh_path(), key, set });
        if let Some(slot) = cache.as_deref_mut() {
            *slot = Some(c.clone());
        }
        Some(c)
    }
    /// The last pins computed (for `options`, which has no document).
    fn last_pins(&self) -> Option<Arc<Cached>> {
        self.cache.lock().ok().and_then(|c| c.clone())
    }
    /// A new selection starts with no pin selected.
    fn sync(&mut self, cx: &ToolContext) {
        if cx.selection.objects != self.ids {
            self.ids = cx.selection.objects.clone();
            self.selected.clear();
        }
    }
    fn pin_at(cx: &ToolContext, set: &PinSet, p: Point) -> Option<usize> {
        let tol = cx.tol(7.0);
        set.pins.iter().enumerate().map(|(i, q)| (i, q.at.distance(p))).filter(|(_, d)| *d <= tol).min_by(|a, b| a.1.total_cmp(&b.1)).map(|(i, _)| i)
    }
    /// The selected pin the art would turn around with Alt held at `p`: near it, not on a pin.
    fn turn_pin_at(&self, cx: &ToolContext, set: &PinSet, p: Point) -> Option<usize> {
        if Self::pin_at(cx, set, p).is_some() {
            return None;
        }
        let reach = cx.tol(TURN_REACH_PX);
        self.selected
            .iter()
            .filter_map(|&i| Some((i, set.pins.get(i)?.at.distance(p))))
            .filter(|(_, d)| *d <= reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    /// The pins during gesture `g`.
    fn targets(&self, set: &PinSet, g: Gesture) -> Vec<PuppetPin> {
        let mut pins = set.pins.clone();
        for (i, q) in pins.iter_mut().enumerate() {
            match g {
                Gesture::Move { from, to } if self.selected.contains(&i) => q.at += to - from,
                Gesture::Turn { pin, base, angle, .. } if pin == i => q.angle = Some(base + angle),
                _ => {}
            }
        }
        pins
    }
    /// Run `object.puppetWarp` with `pins`, then look at the pins it left.
    fn apply(&self, set: &PinSet, pins: &[PuppetPin]) -> Vec<Action> {
        vec![Action::Exec("object.puppetWarp".into(), set.params(pins, self.expand)), Action::Notify("pins".into())]
    }
    /// Select pin `i` (Shift: add it to the selection or take it out). → whether it is selected.
    fn pick(&mut self, i: usize, shift: bool) -> bool {
        match (shift, self.selected.iter().position(|s| *s == i)) {
            (true, Some(k)) => {
                self.selected.remove(k);
                false
            }
            (true, None) => {
                self.selected.push(i);
                true
            }
            // A press on one of several selected pins drags them all.
            (false, Some(_)) => true,
            (false, None) => {
                self.selected = vec![i];
                true
            }
        }
    }
}

impl Tool for PuppetWarpTool {
    fn id(&self) -> &'static str {
        "puppetWarp"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = Some((p, ev.mods));
                if self.off_mesh.is_some_and(|q| q.distance(p) > cx.tol(6.0)) {
                    self.off_mesh = None;
                }
                vec![]
            }
            PointerKind::Down => {
                self.sync(cx);
                self.off_mesh = None;
                let Some(c) = self.pins(cx) else {
                    // Click an object to select it: its pins appear at once.
                    if let Some(h) = vectorcraft_doc::hit::hit_test(cx.doc, p, cx.hit_options()) {
                        return vec![
                            Action::Exec("select.set".into(), json!({"ids": [h.top_object(cx.isolation).0]})),
                            Action::Notify("pins".into()),
                        ];
                    }
                    return vec![];
                };
                let set = &c.set;
                if ev.mods.alt
                    && let Some(i) = self.turn_pin_at(cx, set, p)
                    && let Some(q) = set.pins.get(i)
                {
                    self.drag = Some(Gesture::Turn { pin: i, base: set.turn_of(i), from: (p - q.at).atan2(), angle: 0.0 });
                    self.before_drag = self.selected.clone();
                    self.start = Some(c.clone());
                    return vec![Action::Begin("Puppet Warp".into())];
                }
                if let Some(i) = Self::pin_at(cx, set, p) {
                    let before = self.selected.clone();
                    if !self.pick(i, ev.mods.shift) {
                        return vec![];
                    }
                    self.drag = Some(Gesture::Move { from: p, to: p });
                    self.before_drag = before;
                    self.start = Some(c.clone());
                    return vec![Action::Begin("Puppet Warp".into())];
                }
                match set.rest_of(p) {
                    Some(rest) => {
                        let mut pins = set.pins.clone();
                        pins.push(PuppetPin { rest, at: p, angle: None });
                        self.pick(pins.len() - 1, ev.mods.shift);
                        self.apply(set, &pins)
                    }
                    None => {
                        // Off the mesh: nothing to pin there (the feedback says so).
                        self.off_mesh = Some(p);
                        if !ev.mods.shift {
                            self.selected.clear();
                        }
                        vec![]
                    }
                }
            }
            PointerKind::Drag => {
                let (Some(mut g), Some(c)) = (self.drag, self.start.clone()) else { return vec![] };
                match &mut g {
                    Gesture::Move { to, .. } => *to = p,
                    Gesture::Turn { pin, from, angle, .. } => {
                        let Some(q) = c.set.pins.get(*pin) else { return vec![] };
                        *angle = (p - q.at).atan2() - *from;
                    }
                }
                self.drag = Some(g);
                vec![Action::Preview("object.puppetWarp".into(), c.set.params(&self.targets(&c.set, g), self.expand))]
            }
            PointerKind::Up => match self.drag.take() {
                Some(_) => {
                    self.start = None;
                    vec![Action::Commit, Action::Notify("pins".into())]
                }
                None => vec![],
            },
            PointerKind::DoubleClick => vec![],
        }
    }
    /// Delete/Backspace remove the selected pins; with none selected they are the shortcut's
    /// (Clear deletes the selected objects).
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Delete | ToolKey::Backspace)
            && self.drag.is_none()
            && cx.selection.objects == self.ids
            && self.pins(cx).is_some_and(|c| self.selected.iter().any(|i| *i < c.set.pins.len()))
    }
    /// Escape during a pin drag or a turn cancels it: the art and its pins go back to the start of
    /// the drag, and the pins selected before the press are selected again.
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.drag.take().is_some() {
            self.start = None;
            self.selected = std::mem::take(&mut self.before_drag);
            return vec![Action::Cancel, Action::Notify("pins".into())];
        }
        if !self.claims_key(cx, key) {
            return vec![];
        }
        let Some(c) = self.pins(cx) else { return vec![] };
        let gone = std::mem::take(&mut self.selected);
        let pins: Vec<PuppetPin> = c.set.pins.iter().enumerate().filter(|(i, _)| !gone.contains(i)).map(|(_, q)| *q).collect();
        self.apply(&c.set, &pins)
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        if let Some(c) = self.pins(cx) {
            if self.show_mesh {
                out.push(Overlay::Path { path: c.mesh.clone(), color: [0x9a, 0x9a, 0x9a], width: 0.5, dashed: false });
            }
            let r = cx.tol(5.0);
            for (i, q) in c.set.pins.iter().enumerate() {
                let sel = self.selected.contains(&i) && cx.selection.objects == self.ids;
                out.push(Overlay::Path {
                    path: super::ellipse_path(q.at, r, r, 0.0),
                    color: BLUE,
                    width: if sel { 3.0 } else { 1.5 },
                    dashed: false,
                });
                out.push(Overlay::Anchor { p: q.at, color: BLUE, filled: sel, size: 3.0 });
            }
            // The dotted circle of a turn: around the pin being turned, or the one Alt would turn.
            let ring = match (self.drag, self.hover) {
                (Some(Gesture::Turn { pin, .. }), _) => Some(pin),
                (None, Some((h, mods))) if mods.alt => self.turn_pin_at(cx, &c.set, h),
                _ => None,
            };
            if let Some(q) = ring.and_then(|i| c.set.pins.get(i)) {
                let rr = cx.tol(TURN_RING_PX);
                out.push(Overlay::Path { path: super::ellipse_path(q.at, rr, rr, 0.0), color: BLUE, width: 1.0, dashed: true });
            }
        }
        if let Some(p) = self.off_mesh {
            out.push(Overlay::Label { p, text: OFF_MESH.into(), color: WARN });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, mods: Mods) -> Cursor {
        let Some(c) = self.pins(cx) else { return Cursor::Arrow };
        if mods.alt && self.turn_pin_at(cx, &c.set, p).is_some() {
            Cursor::Rotate
        } else if Self::pin_at(cx, &c.set, p).is_some() {
            Cursor::Move
        } else if c.set.rest_of(p).is_some() {
            Cursor::PuppetPin
        } else {
            // Off the mesh: no pin goes there.
            Cursor::NotAllowed
        }
    }
    fn options(&self) -> Value {
        let pins = self.last_pins().map(|c| c.set.pins.iter().map(|q| json!([q.at.x, q.at.y])).collect::<Vec<_>>()).unwrap_or_default();
        json!({"showMesh": self.show_mesh, "expand": self.expand, "pins": pins, "selected": self.selected})
    }
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "showMesh" => self.show_mesh = value.as_bool().unwrap_or(self.show_mesh),
            "expand" => {
                if let Some(v) = value.as_f64().filter(|v| v.is_finite()) {
                    self.expand = v.clamp(0.0, 1000.0);
                }
            }
            // Select All Pins (false: deselect them all).
            "selectAllPins" => {
                let n = self.last_pins().map_or(0, |c| c.set.pins.len());
                self.selected = if value.as_bool() == Some(false) { vec![] } else { (0..n).collect() };
            }
            _ => {}
        }
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn notify(&mut self, cx: &ToolContext, _what: &str) {
        self.sync(cx);
        // Pick up the pins the command left (and drop selected pins that are gone).
        let n = self.pins(cx).map_or(0, |c| c.set.pins.len());
        self.selected.retain(|i| *i < n);
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.selected.clear();
        self.hover = None;
        self.off_mesh = None;
        if let Ok(mut c) = self.cache.lock() {
            *c = None;
        }
        self.start = None;
        if self.drag.take().is_some() { vec![Action::Commit] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    fn alt() -> Mods {
        Mods { alt: true, ..Default::default() }
    }

    fn shift() -> Mods {
        Mods { shift: true, ..Default::default() }
    }

    /// The tool on a document, running the `object.puppetWarp` actions it emits the way the
    /// engine does (previews on the snapshot taken at Begin).
    struct Rig {
        doc: Document,
        sel: Selection,
        base: Option<Document>,
        tool: PuppetWarpTool,
        /// The params of the last command run.
        last: Value,
    }

    impl Rig {
        fn new() -> Self {
            let (doc, id) = doc_with_rect();
            Self { doc, sel: Selection { objects: vec![id], ..Default::default() }, base: None, tool: PuppetWarpTool::default(), last: Value::Null }
        }
        fn run(&mut self, p: &Value) {
            let pts = |k: &str| -> Vec<Point> {
                p[k].as_array().unwrap().iter().map(|q| Point::new(q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect()
            };
            let angles = p["angles"].as_array().unwrap().iter().map(|a| a.as_f64().map(f64::to_radians));
            let pins = pts("pins").into_iter().zip(pts("moved")).zip(angles).map(|((rest, at), angle)| PuppetPin { rest, at, angle }).collect();
            let ids: Vec<NodeId> = p["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect();
            assert_eq!(p["rest"], json!(true));
            warp_from_rest(&mut self.doc, &ids, pins, p["expand"].as_f64().unwrap()).unwrap();
            self.last = p.clone();
        }
        fn apply(&mut self, acts: Vec<Action>) -> Vec<Action> {
            for a in &acts {
                match a {
                    Action::Begin(_) => self.base = Some(self.doc.clone()),
                    Action::Preview(_, p) => {
                        self.doc = self.base.clone().unwrap();
                        self.run(p);
                    }
                    Action::Commit => self.base = None,
                    Action::Cancel => {
                        if let Some(b) = self.base.take() {
                            self.doc = b;
                        }
                    }
                    Action::Exec(c, p) if c == "object.puppetWarp" => self.run(p),
                    Action::Notify(w) => {
                        let p = paint();
                        self.tool.notify(&cx(&self.doc, &self.sel, &p), w);
                    }
                    _ => {}
                }
            }
            acts
        }
        fn ev(&mut self, kind: PointerKind, x: f64, y: f64, mods: Mods) -> Vec<Action> {
            let p = paint();
            let acts = self.tool.pointer(&cx(&self.doc, &self.sel, &p), &PointerEvent::new(kind, x, y).with_mods(mods));
            self.apply(acts)
        }
        fn click(&mut self, x: f64, y: f64, mods: Mods) -> Vec<Action> {
            let mut a = self.ev(PointerKind::Down, x, y, mods);
            a.extend(self.ev(PointerKind::Up, x, y, mods));
            a
        }
        fn drag(&mut self, from: (f64, f64), to: (f64, f64), mods: Mods) -> Vec<Action> {
            let mut a = self.ev(PointerKind::Down, from.0, from.1, mods);
            a.extend(self.ev(PointerKind::Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0, mods));
            a.extend(self.ev(PointerKind::Drag, to.0, to.1, mods));
            a.extend(self.ev(PointerKind::Up, to.0, to.1, mods));
            a
        }
        fn key(&mut self, k: ToolKey) -> Vec<Action> {
            let p = paint();
            let acts = self.tool.key(&cx(&self.doc, &self.sel, &p), k, Mods::default());
            self.apply(acts)
        }
        fn claims(&self, k: ToolKey) -> bool {
            let p = paint();
            self.tool.claims_key(&cx(&self.doc, &self.sel, &p), k)
        }
        fn overlays(&self) -> Vec<Overlay> {
            let p = paint();
            self.tool.overlays(&cx(&self.doc, &self.sel, &p))
        }
        fn cursor(&self, x: f64, y: f64, mods: Mods) -> Cursor {
            let p = paint();
            self.tool.cursor(&cx(&self.doc, &self.sel, &p), Point::new(x, y), mods)
        }
        fn pins(&self) -> Vec<PuppetPin> {
            self.doc.puppet.as_ref().map(|p| p.pins.clone()).unwrap_or_default()
        }
        fn bounds(&self) -> Rect {
            self.doc.node(self.sel.objects[0]).unwrap().geometric_bounds().unwrap()
        }
    }

    #[test]
    fn puppet_tool_autopins_adds_and_drags() {
        let mut r = Rig::new();
        // The automatic pins show as soon as there is art selected.
        assert_eq!(r.overlays().iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 3, "3 automatic pins");
        // Click inside the rect away from them: adds a pin (one command, the pins kept in the document).
        let acts = r.click(130.0, 170.0, Mods::default());
        assert!(matches!(&acts[0], Action::Exec(c, _) if c == "object.puppetWarp"), "{acts:?}");
        assert_eq!(r.pins().len(), 4, "3 auto pins + 1");
        assert_eq!(r.tool.options()["selected"], json!([3]));
        // Drag the new pin.
        assert_eq!(r.ev(PointerKind::Down, 130.0, 170.0, Mods::default()), vec![Action::Begin("Puppet Warp".into())]);
        let acts = r.ev(PointerKind::Drag, 120.0, 190.0, Mods::default());
        let Action::Preview(cmd, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!(cmd, "object.puppetWarp");
        assert_eq!(v["moved"][3], json!([120.0, 190.0]));
        assert_eq!(v["pins"][3], json!([130.0, 170.0]));
        assert_eq!(r.ev(PointerKind::Up, 120.0, 190.0, Mods::default())[0], Action::Commit);
        // Alt-click on a pin doesn't delete it (Alt turns the art around a pin): Delete does.
        r.click(120.0, 190.0, alt());
        assert_eq!(r.pins().len(), 4);
        assert!(r.claims(ToolKey::Delete));
        r.key(ToolKey::Delete);
        assert_eq!(r.pins().len(), 3);
        assert!(!r.claims(ToolKey::Delete), "nothing selected: Delete is Clear's");
    }

    #[test]
    fn shift_click_selects_several_pins_that_move_together() {
        let mut r = Rig::new();
        // Two new pins, the second Shift-added to the selection.
        r.click(110.0, 110.0, Mods::default());
        r.click(190.0, 110.0, shift());
        let (a, b) = (r.pins().len() - 2, r.pins().len() - 1);
        assert_eq!(r.tool.selected, vec![a, b]);
        // Dragging one moves both.
        r.ev(PointerKind::Down, 110.0, 110.0, Mods::default());
        let acts = r.ev(PointerKind::Drag, 110.0, 100.0, Mods::default());
        let Action::Preview(_, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((v["moved"][a].clone(), v["moved"][b].clone()), (json!([110.0, 100.0]), json!([190.0, 100.0])));
        assert_eq!(v["moved"][0], v["pins"][0], "unselected pins stay");
        r.ev(PointerKind::Up, 110.0, 100.0, Mods::default());
        // Shift-click takes a pin out of the selection without dragging.
        assert!(r.ev(PointerKind::Down, 190.0, 100.0, shift()).is_empty());
        assert_eq!(r.tool.selected, vec![a]);
        // Select All Pins, then delete them all: the art stays as it is.
        let before = r.bounds();
        r.tool.set_option("selectAllPins", &json!(true));
        assert_eq!(r.tool.selected, (0..r.pins().len()).collect::<Vec<_>>());
        r.key(ToolKey::Backspace);
        assert!(r.pins().is_empty());
        assert_eq!(r.bounds(), before);
    }

    #[test]
    fn escape_cancels_a_pin_drag_or_a_turn_and_selects_the_pins_selected_before_it() {
        let mut r = Rig::new();
        r.click(110.0, 110.0, Mods::default());
        r.click(190.0, 110.0, Mods::default());
        let b = r.pins().len() - 1;
        assert_eq!(r.tool.selected, vec![b]);
        assert!(r.key(ToolKey::Escape).is_empty(), "no drag: Escape is the shortcut's");
        let (pins, art) = (r.pins(), r.doc.node(r.sel.objects[0]).cloned().unwrap());
        // A press on the unselected pin selects it and drags it; Escape puts everything back.
        for (what, press, to, mods) in [("move", (110.0, 110.0), (130.0, 140.0), Mods::default()), ("turn", (205.0, 110.0), (190.0, 125.0), alt())] {
            assert_eq!(r.ev(PointerKind::Down, press.0, press.1, mods), vec![Action::Begin("Puppet Warp".into())], "{what}");
            r.ev(PointerKind::Drag, to.0, to.1, mods);
            assert_ne!(r.doc.node(r.sel.objects[0]), Some(&art), "{what}: the drag warps the art");
            assert_eq!(r.key(ToolKey::Escape), vec![Action::Cancel, Action::Notify("pins".into())], "{what}");
            assert_eq!(r.doc.node(r.sel.objects[0]), Some(&art), "{what}: Escape puts the art back");
            assert_eq!(r.pins(), pins, "{what}: and the pins");
            assert_eq!(r.tool.selected, vec![b], "{what}: and the pins selected before the press");
            assert!(!r.tool.busy(), "{what}");
            // The rest of the drag does nothing.
            assert!(r.ev(PointerKind::Drag, to.0 + 10.0, to.1, mods).is_empty(), "{what}");
            assert!(r.ev(PointerKind::Up, to.0 + 10.0, to.1, mods).is_empty(), "{what}");
            assert_eq!(r.pins(), pins, "{what}");
        }
    }

    #[test]
    fn alt_drag_near_a_selected_pin_turns_the_art_around_it() {
        let mut r = Rig::new();
        r.click(110.0, 150.0, Mods::default());
        let i = r.pins().len() - 1;
        // Alt near (not on) the selected pin: the turn ring and cursor.
        r.ev(PointerKind::Move, 125.0, 150.0, alt());
        assert_eq!(r.cursor(125.0, 150.0, alt()), Cursor::Rotate);
        assert!(r.overlays().iter().any(|o| matches!(o, Overlay::Path { dashed: true, .. })), "dotted ring");
        assert_ne!(r.cursor(125.0, 150.0, Mods::default()), Cursor::Rotate, "only with Alt");
        // Drag a quarter turn around it.
        assert_eq!(r.ev(PointerKind::Down, 125.0, 150.0, alt()), vec![Action::Begin("Puppet Warp".into())]);
        let acts = r.ev(PointerKind::Drag, 110.0, 165.0, alt());
        let Action::Preview(cmd, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!(cmd, "object.puppetWarp");
        assert!((v["angles"][i].as_f64().unwrap() - 90.0).abs() < 1e-6, "{v}");
        assert_eq!(v["moved"][i], v["pins"][i], "the pin stays put");
        assert_eq!(v["angles"][0], Value::Null, "other pins turn freely");
        r.ev(PointerKind::Up, 110.0, 165.0, alt());
        // The turned pin keeps its turn through the next warp.
        let q = r.pins()[0].at;
        r.drag((q.x, q.y), (q.x + 5.0, q.y), Mods::default());
        assert!((r.last["angles"][i].as_f64().unwrap() - 90.0).abs() < 1e-6, "{}", r.last);
    }

    #[test]
    fn show_mesh_is_on_by_default_and_expand_may_be_zero() {
        let mut t = PuppetWarpTool::default();
        assert_eq!((t.options()["showMesh"].clone(), t.options()["expand"].clone()), (json!(true), json!(3.0)));
        t.set_option("expand", &json!(0));
        assert_eq!(t.options()["expand"], json!(0.0));
        t.set_option("expand", &json!(f64::NAN));
        t.set_option("expand", &json!("x"));
        assert_eq!(t.options()["expand"], json!(0.0));
    }

    #[test]
    fn warps_start_from_the_rest_shape() {
        let mut r = Rig::new();
        let rest = r.doc.node(r.sel.objects[0]).cloned().unwrap();
        r.click(190.0, 190.0, Mods::default());
        // Two drags out and back: the second starts from where the first left the pin.
        r.drag((190.0, 190.0), (230.0, 230.0), Mods::default());
        assert!(r.bounds().x1 > 215.0, "{:?}", r.bounds());
        r.drag((230.0, 230.0), (190.0, 190.0), Mods::default());
        // Back where it started: the original shape, not a warp of a warp.
        let back = r.doc.node(r.sel.objects[0]).unwrap();
        let (a, b) = (rest.path_data().unwrap(), back.path_data().unwrap());
        let worst = a.subpaths[0].anchors.iter().zip(&b.subpaths[0].anchors).map(|(x, y)| x.p.distance(y.p)).fold(0.0, f64::max);
        assert!(worst < 0.01, "{worst}");
    }

    #[test]
    fn a_click_off_the_mesh_places_no_pin_and_says_so() {
        let mut r = Rig::new();
        assert_eq!(r.cursor(130.0, 170.0, Mods::default()), Cursor::PuppetPin, "on the mesh, away from the pins");
        r.click(130.0, 170.0, Mods::default());
        assert_eq!(r.cursor(400.0, 400.0, Mods::default()), Cursor::NotAllowed);
        assert!(r.click(400.0, 400.0, Mods::default()).is_empty());
        assert_eq!(r.pins().len(), 4);
        assert!(r.tool.selected.is_empty(), "the click deselects the pins");
        assert!(r.overlays().iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == OFF_MESH)));
        r.ev(PointerKind::Move, 450.0, 450.0, Mods::default());
        assert!(!r.overlays().iter().any(|o| matches!(o, Overlay::Label { .. })), "gone once the pointer moves on");
        // The command refuses a pin off the mesh.
        let ids = r.sel.objects.clone();
        let off = PuppetPin { rest: Point::new(400.0, 400.0), at: Point::new(400.0, 400.0), angle: None };
        assert!(warp_from_rest(&mut r.doc, &ids, vec![off], 3.0).unwrap_err().contains("off the artwork"));
    }
}
