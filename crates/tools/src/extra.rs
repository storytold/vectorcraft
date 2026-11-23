//! Flare, Reshape, Shaper and Graph tools.
//!
//! Flare: drag sets the center and its size and turns the rays as the pointer moves up or down
//! (Shift holds their turn, ↑/↓ add or remove rays and Ctrl/Cmd keeps the center's size while the
//! drag sizes the halo; click without dragging opens the Flare Tool Options, and Alt-click (Option
//! on macOS) draws a flare there with them at once), then a click sets the
//! end point of the rings (↑/↓ add or remove rings and ~ places them anew); Space held during
//! either drag moves the flare. The rest comes
//! from the tool's options (the Flare Tool Options, which last). With one flare selected, the tool
//! shows its center and end handles, and dragging either changes the flare's length and direction
//! (`shape.editFlare`). Reshape: a click on a selected
//! path sets a focal point (adding an anchor on a segment), Shift-click adds or removes more, and
//! a drag from a focal point moves them all; the engine's `path.reshape` stretches the selected
//! anchors between them and the anchors that stay.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2};

use crate::distort::{BLUE, ellipse_path};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "flare" => Box::new(FlareTool::default()),
        "reshape" => Box::new(ReshapeTool::default()),
        "shaper" => Box::new(ShaperTool::default()),
        id if is_graph_tool(id) => Box::new(GraphTool::new(id)),
        _ => return None,
    })
}

/// Whether `id` is one of the graph tools (`columnGraph`, `pieGraph`…).
pub fn is_graph_tool(id: &str) -> bool {
    id.ends_with("Graph") && vectorcraft_doc::GraphKind::parse(id).is_some()
}

/// The Flare Tool Options as `shape.flare` takes them: (key, default, least, most). The diameter
/// and the rings' path length are in points, the counts whole, the direction in degrees, the
/// rest percentages.
pub const FLARE_OPTIONS: [(&str, f64, f64, f64); 12] = [
    ("diameter", 100.0, 0.0, 1000.0),
    ("opacity", 50.0, 0.0, 100.0),
    ("brightness", 30.0, 0.0, 100.0),
    ("growth", 20.0, 0.0, 300.0),
    ("fuzziness", 50.0, 0.0, 100.0),
    ("rays", 15.0, 0.0, 50.0),
    ("longest", 300.0, 0.0, 1000.0),
    ("rayFuzziness", 100.0, 0.0, 100.0),
    ("pathLength", 300.0, 0.0, 1000.0),
    ("rings", 10.0, 0.0, 50.0),
    ("largest", 50.0, 0.0, 250.0),
    ("direction", 45.0, 0.0, 360.0),
];

/// Flare Tool: centre drag, then a click for the rings' end point.
pub struct FlareTool {
    /// Pressed at (centre) and the current drag point.
    drag: Option<(Point, Point)>,
    /// Centre placed, waiting for the end-point click: (centre, diameter).
    placed: Option<(Point, f64)>,
    hover: Point,
    /// The pointer at the last press or drag, from which Space held moves the flare.
    last: Point,
    began: bool,
    /// Whether Alt (Option on macOS) was held when the button went down for the center: a release
    /// without a drag then draws a flare with the options at once.
    alt_press: bool,
    /// Ray count changed with ↑/↓ during the center drag (None = the options' count).
    rays: Option<u64>,
    /// Ring count changed with ↑/↓ while the rings' end point is placed (None = the options' count).
    rings: Option<u64>,
    /// The rings' placement ~ chose (`shape.flare`'s `ringSeed`; None = the placement `seed` gives).
    ring_seed: Option<u64>,
    /// Whether backquote (the ~ key) was held at the last pointer event.
    quote: bool,
    /// Ctrl/Cmd held in the center drag: the center's diameter, the Halo growth (%) and the
    /// pointer's distance from the center when the key went down.
    held: Option<(f64, f64, f64)>,
    /// The Halo growth (%) the drag set while Ctrl/Cmd held the center (None = the options').
    growth: Option<f64>,
    /// The rays' turn (`shape.flare`'s `rotation`, degrees) the center drag set.
    turn: f64,
    /// The options' values, in [`FLARE_OPTIONS`] order.
    values: [f64; 12],
    /// Whether the flare has rays and rings (the options' Rays and Rings checkboxes).
    rays_on: bool,
    rings_on: bool,
    /// A drag of the selected flare's center or end handle.
    handle: Option<HandleDrag>,
}

/// A drag of a selected flare's handle: the flare, whether it is the end handle (else the center),
/// where the handle was, where it was pressed, whether Alt (Option on macOS) was held at the press
/// and whether the drag has begun its undo step.
#[derive(Clone, Copy, Debug)]
struct HandleDrag {
    id: vectorcraft_doc::NodeId,
    end: bool,
    from: Point,
    at: Point,
    alt: bool,
    began: bool,
}

/// The flare whose handles the Flare tool shows: the one selected object, when it is a flare that
/// can be edited, with its center and the rings' end point.
fn selected_flare(cx: &ToolContext) -> Option<(vectorcraft_doc::NodeId, Point, Point)> {
    let [id] = cx.selection.objects.as_slice() else { return None };
    if !cx.doc.is_editable(*id) {
        return None;
    }
    let node = cx.doc.node(*id)?;
    let spec = node.extras().flare.as_deref().filter(|spec| spec.draws(node))?;
    Some((*id, spec.center()?, spec.end()?))
}

/// `shape.editFlare`'s params for handle drag `h` with the pointer at `pointer`: the handle moves
/// as far as the pointer has.
fn handle_params(h: HandleDrag, pointer: Point) -> Value {
    let p = h.from + (pointer - h.at);
    if h.end { json!({ "ids": [h.id.0], "x2": p.x, "y2": p.y }) } else { json!({ "ids": [h.id.0], "cx": p.x, "cy": p.y }) }
}

impl Default for FlareTool {
    fn default() -> Self {
        Self {
            drag: None,
            placed: None,
            hover: Point::ZERO,
            last: Point::ZERO,
            began: false,
            alt_press: false,
            rays: None,
            rings: None,
            ring_seed: None,
            quote: false,
            held: None,
            growth: None,
            turn: 0.0,
            values: FLARE_OPTIONS.map(|(_, default, _, _)| default),
            rays_on: true,
            rings_on: true,
            handle: None,
        }
    }
}

/// Whether Flare Tool Options value `key` is a count (whole).
fn is_count(key: &str) -> bool {
    matches!(key, "rays" | "rings")
}

/// Flare Tool Options value `x` of `key` as JSON: a count as a whole number.
fn option_json(key: &str, x: f64) -> Value {
    if is_count(key) { json!(x as u64) } else { json!(x) }
}

/// `shape.flare`'s params for a flare at `c` drawn with the Flare Tool Options `o` (as the tool
/// reports them, [`Tool::options`]): with Rays or Rings off it has none.
pub fn flare_params(o: &Value, c: Point) -> Value {
    let mut v = json!({ "cx": c.x, "cy": c.y });
    for (key, default, ..) in FLARE_OPTIONS {
        v[key] = o.get(key).cloned().unwrap_or_else(|| option_json(key, default));
    }
    for (on, key) in [("raysOn", "rays"), ("ringsOn", "rings")] {
        if o.get(on).and_then(Value::as_bool) == Some(false) {
            v[key] = json!(0);
        }
    }
    v
}

/// The chosen rotation rate: one degree per document point of vertical pointer travel.
/// This is independent of pointer event frequency and elapsed time.
const RAY_DEGREES_PER_POINT: f64 = 1.0;

/// Accumulate travel rather than an event-count step. Frozen/moving samples still update
/// the pointer baseline, so releasing a modifier never catches up skipped rotation.
fn ray_turn(turn: f64, dy: f64, shift: bool) -> f64 {
    if shift || !dy.is_finite() {
        return turn;
    }
    (turn + dy * RAY_DEGREES_PER_POINT).rem_euclid(360.0)
}

/// The ring placement ~ chooses after `last`, with the pointer at `at`: a new number below 2^53, so
/// a JSON client reads it exactly, and never `last` again.
fn next_ring_seed(last: Option<u64>, at: Point) -> u64 {
    let mut x = last.unwrap_or(0x9e37_79b9_7f4a_7c15) ^ at.x.to_bits() ^ at.y.to_bits().rotate_left(29);
    loop {
        // SplitMix64.
        x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = x;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        let seed = (z ^ (z >> 31)) & ((1 << 53) - 1);
        if Some(seed) != last {
            return seed;
        }
    }
}

impl FlareTool {
    /// `shape.flare`'s params: the options, with the dragged center and diameter, the rays' turn,
    /// the ray and ring counts ↑/↓ chose, the Halo growth Ctrl/Cmd set, the ring placement ~ chose
    /// and the rings' end point once placed.
    fn params(&self, c: Point, diameter: f64, end: Option<Point>) -> Value {
        let mut v = flare_params(&self.options(), c);
        v["diameter"] = json!(diameter.max(1.0));
        v["rotation"] = json!(self.turn);
        if let Some(r) = self.rays.filter(|_| self.rays_on) {
            v["rays"] = json!(r);
        }
        if let Some(r) = self.rings.filter(|_| self.rings_on) {
            v["rings"] = json!(r);
        }
        if let Some(g) = self.growth {
            v["growth"] = json!(g);
        }
        if let Some(s) = self.ring_seed.filter(|_| self.rings_on) {
            v["ringSeed"] = json!(s);
        }
        if let Some(e) = end {
            v["x2"] = json!(e.x);
            v["y2"] = json!(e.y);
        }
        v
    }

    /// Flare Tool Options value `key` as the tool has it.
    fn option(&self, key: &str) -> f64 {
        FLARE_OPTIONS.iter().position(|(k, ..)| *k == key).map_or(0.0, |i| self.values[i])
    }

    /// Count `now` (else the options' `key`) one up or down, within the options' range for `key`.
    fn step_count(&self, key: &str, now: Option<u64>, up: bool) -> u64 {
        let most = FLARE_OPTIONS.iter().find(|(k, ..)| *k == key).map_or(0.0, |o| o.3) as u64;
        let r = now.unwrap_or(self.option(key) as u64);
        if up { (r + 1).min(most) } else { r.saturating_sub(1).min(most) }
    }

    /// Ctrl/Cmd in the center drag, with the pointer `dist` from the center after `before` at the
    /// last drag: once the flare has begun, the center keeps the diameter it showed when the key
    /// went down, and the halo's radius keeps its ratio to the pointer's distance through the Halo
    /// growth, within the options' 0 to 300 %. Letting the key go sizes the center with the drag
    /// again, and the flare keeps the new Halo growth. The key is pressed while dragging: a press
    /// made with it already held goes to the selection tool the session lends for Ctrl/Cmd.
    fn hold_center(&mut self, before: f64, dist: f64, cmd: bool) {
        match (cmd, self.held) {
            (true, None) if self.began && before > 0.0 => {
                let growth = self.growth.unwrap_or_else(|| self.option("growth"));
                self.held = Some((2.0 * before, growth, before));
            }
            (false, Some(_)) => self.held = None,
            _ => {}
        }
        if let Some((_, growth, at)) = self.held {
            let most = FLARE_OPTIONS.iter().find(|(k, ..)| *k == "growth").map_or(0.0, |o| o.3);
            let g = (100.0 + growth) * dist / at - 100.0;
            if g.is_finite() {
                self.growth = Some(g.clamp(0.0, most));
            }
        }
    }

    /// The center's diameter with the pointer `dist` from it: twice that, unless Ctrl/Cmd holds it.
    fn center_diameter(&self, dist: f64) -> f64 {
        self.held.map_or(2.0 * dist, |(d, ..)| d)
    }

    /// Alt-click (Option-click on macOS) at `c`: one flare drawn with the options as they are, as one
    /// command, so a refusal (a click beyond the plane's horizon) reaches the caller as an error.
    /// While the perspective grid shows with an active plane, the flare goes on that plane with its
    /// sizes in plane units from the click, as the Flare Tool Options opened by a click draw it.
    fn click_flare(&self, cx: &ToolContext, c: Point) -> Vec<Action> {
        let params = flare_params(&self.options(), c);
        let grid = crate::distort::perspective::PerspectiveGrid::current(cx.doc);
        if grid.visible && grid.plane != crate::distort::perspective::Plane::None {
            vec![Action::Exec("perspective.draw".into(), json!({ "command": "shape.flare", "params": params, "at": [c.x, c.y] }))]
        } else {
            vec![Action::Exec("shape.flare".into(), params)]
        }
    }
}

impl Tool for FlareTool {
    fn id(&self) -> &'static str {
        "flare"
    }
    fn busy(&self) -> bool {
        self.drag.is_some() || self.placed.is_some() || self.handle.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.hover = ev.pos;
        // ~ pressed while the rings' end point is placed places the rings anew (Rings on).
        let quote_pressed = ev.backquote && !self.quote;
        self.quote = ev.backquote;
        if quote_pressed && self.rings_on && self.placed.is_some() && matches!(ev.kind, PointerKind::Drag | PointerKind::Move | PointerKind::Up) {
            self.ring_seed = Some(next_ring_seed(self.ring_seed, ev.pos));
        }
        if let Some(mut h) = self.handle {
            return match ev.kind {
                PointerKind::Drag if h.began => vec![Action::Preview("shape.editFlare".into(), handle_params(h, ev.pos))],
                PointerKind::Drag if ev.pos.distance(h.at) < cx.tol(3.0) => vec![],
                PointerKind::Drag => {
                    h.began = true;
                    self.handle = Some(h);
                    vec![Action::Begin("Edit Flare".into()), Action::Preview("shape.editFlare".into(), handle_params(h, ev.pos))]
                }
                PointerKind::Up => {
                    self.handle = None;
                    if h.began {
                        vec![Action::Preview("shape.editFlare".into(), handle_params(h, ev.pos)), Action::Commit]
                    } else if h.alt {
                        // A click on a handle without a drag is a click with the tool.
                        self.click_flare(cx, h.at)
                    } else {
                        vec![Action::Dialog("flare".into(), json!({ "x": h.at.x, "y": h.at.y }))]
                    }
                }
                _ => vec![],
            };
        }
        match ev.kind {
            PointerKind::Down => {
                // A press on the selected flare's center or end handle drags that handle.
                if self.placed.is_none()
                    && let Some((id, c, e)) = selected_flare(cx)
                {
                    let tol = cx.tol(4.0);
                    let (dc, de) = (ev.pos.distance(c), ev.pos.distance(e));
                    if dc.min(de) <= tol {
                        let end = de < dc;
                        self.handle = Some(HandleDrag { id, end, from: if end { e } else { c }, at: ev.pos, alt: ev.mods.alt, began: false });
                        return vec![];
                    }
                }
                if self.placed.is_none() {
                    self.drag = Some((ev.pos, ev.pos));
                    self.rays = None;
                    self.rings = None;
                    self.ring_seed = None;
                    self.held = None;
                    self.growth = None;
                    self.turn = 0.0;
                    self.alt_press = ev.mods.alt;
                }
                self.last = ev.pos;
                vec![]
            }
            PointerKind::Drag => {
                if let Some((mut c, p)) = self.drag {
                    // Space held moves the center with the pointer, so the size and the turn stay.
                    let before = p.distance(c);
                    let dy = ev.pos.y - self.last.y;
                    crate::shape::space_moves(&mut c, &mut self.last, ev, self.began);
                    self.drag = Some((c, ev.pos));
                    if !ev.mods.space {
                        self.turn = ray_turn(self.turn, dy, ev.mods.shift);
                    }
                    let dist = ev.pos.distance(c);
                    let begins = !self.began && dist >= cx.tol(3.0);
                    self.began |= begins;
                    // With the key held from the start, the center keeps the first size the flare shows.
                    self.hold_center(if begins { dist } else { before }, dist, ev.mods.cmd);
                    let preview = Action::Preview("shape.flare".into(), self.params(c, self.center_diameter(dist), None));
                    if begins {
                        return vec![Action::Begin("Flare".into()), preview];
                    }
                    if self.began {
                        return vec![preview];
                    }
                } else if let Some((mut c, d)) = self.placed {
                    // Space held moves the whole flare, rings' end point included.
                    crate::shape::space_moves(&mut c, &mut self.last, ev, true);
                    self.placed = Some((c, d));
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos)))];
                }
                vec![]
            }
            PointerKind::Up => {
                if let Some((mut c, d)) = self.placed.take() {
                    crate::shape::space_moves(&mut c, &mut self.last, ev, true);
                    self.began = false;
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos))), Action::Commit];
                }
                let Some((c, p)) = self.drag.take() else { return vec![] };
                if self.began {
                    // Keep the interaction open: the next click places the end point.
                    self.placed = Some((c, self.center_diameter(p.distance(c))));
                    self.held = None;
                    vec![]
                } else if std::mem::take(&mut self.alt_press) {
                    self.click_flare(cx, c)
                } else {
                    vec![Action::Dialog("flare".into(), json!({ "x": c.x, "y": c.y }))]
                }
            }
            PointerKind::Move => {
                if let Some((c, d)) = self.placed {
                    return vec![Action::Preview("shape.flare".into(), self.params(c, d, Some(ev.pos)))];
                }
                vec![]
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        // ↑/↓ add/remove rays in the center drag and rings while the rings' end point is placed.
        if matches!(key, ToolKey::Up | ToolKey::Down) && self.began {
            let up = key == ToolKey::Up;
            let preview = match (self.drag, self.placed) {
                (Some((c, p)), _) if self.rays_on => {
                    self.rays = Some(self.step_count("rays", self.rays, up));
                    self.params(c, self.center_diameter(p.distance(c)), None)
                }
                (_, Some((c, d))) if self.rings_on => {
                    self.rings = Some(self.step_count("rings", self.rings, up));
                    self.params(c, d, Some(self.hover))
                }
                _ => return vec![],
            };
            return vec![Action::Preview("shape.flare".into(), preview)];
        }
        if key == ToolKey::Escape
            && let Some(h) = self.handle.take()
        {
            return if h.began { vec![Action::Cancel] } else { vec![] };
        }
        if key == ToolKey::Escape && self.busy() {
            self.rays = None;
            self.rings = None;
            self.ring_seed = None;
            self.held = None;
            self.growth = None;
            self.drag = None;
            self.placed = None;
            let began = std::mem::take(&mut self.began);
            return if began { vec![Action::Cancel] } else { vec![] };
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        // Switching away after the centre drag keeps the flare with the rings its options give.
        self.drag = None;
        self.held = None;
        if self.handle.take().is_some_and(|h| h.began) {
            return vec![Action::Commit];
        }
        if self.placed.take().is_some() && std::mem::take(&mut self.began) { vec![Action::Commit] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        match (self.drag, self.placed) {
            (Some((c, p)), _) => {
                let r = self.center_diameter(p.distance(c)) / 2.0;
                vec![Overlay::Path { path: ellipse_path(c, r, r, 0.0), color: BLUE, width: 1.0, dashed: false }]
            }
            (_, Some((c, _))) => vec![Overlay::Line { a: c, b: self.hover, color: BLUE, dashed: true }],
            // The selected flare's center and end handles.
            _ => match selected_flare(cx) {
                Some((_, c, e)) => vec![
                    Overlay::Line { a: c, b: e, color: BLUE, dashed: true },
                    Overlay::Anchor { p: c, color: BLUE, filled: self.handle.is_some_and(|h| !h.end), size: 7.0 },
                    Overlay::Anchor { p: e, color: BLUE, filled: self.handle.is_some_and(|h| h.end), size: 7.0 },
                ],
                None => vec![],
            },
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
    fn options(&self) -> Value {
        let mut v = json!({ "raysOn": self.rays_on, "ringsOn": self.rings_on });
        for ((key, ..), x) in FLARE_OPTIONS.iter().zip(self.values) {
            v[*key] = option_json(key, x);
        }
        v
    }
    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "raysOn" => self.rays_on = v.as_bool().unwrap_or(true),
            "ringsOn" => self.rings_on = v.as_bool().unwrap_or(true),
            _ => {
                let Some((slot, (_, default, lo, hi))) = self.values.iter_mut().zip(FLARE_OPTIONS).find(|(_, (k, ..))| *k == key) else { return };
                let x = v.as_f64().filter(|x| x.is_finite()).unwrap_or(default).clamp(lo, hi);
                *slot = if is_count(key) { x.round() } else { x };
            }
        }
    }
    /// The switches and [`FLARE_OPTIONS`] within their ranges, the counts whole.
    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        if matches!(key, "raysOn" | "ringsOn") {
            return c::boolean(key, v);
        }
        let Some(&(_, _, lo, hi)) = FLARE_OPTIONS.iter().find(|(k, ..)| *k == key) else { return Err(c::unknown(key)) };
        if is_count(key) && v.as_f64().is_none_or(|n| n.fract() != 0.0) {
            return Err(format!("`{key}` must be a whole number from {lo} to {hi}"));
        }
        c::number(key, v, lo, hi)
    }
}

/// Where a Reshape press landed on a selected path.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ReshapeHit {
    /// On an anchor: (subpath, anchor).
    Anchor(usize, usize),
    /// On a segment, away from its anchors: (subpath, the index of the anchor `path.reshape` adds
    /// there).
    Segment(usize, usize),
}

impl ReshapeHit {
    /// The anchor the press grabs, as `(subpath, anchor)` once any anchor has been added.
    fn anchor(self) -> (usize, usize) {
        match self {
            Self::Anchor(si, ai) | Self::Segment(si, ai) => (si, ai),
        }
    }

    /// Where the anchor `r` is once the press has added its anchor: one further on after an
    /// anchor added before it on its subpath.
    fn renumber(self, r: (usize, usize)) -> (usize, usize) {
        match self {
            Self::Segment(si, added) if r.0 == si && r.1 >= added => (r.0, r.1.saturating_add(1)),
            _ => r,
        }
    }
}

/// A Reshape press: the path, the point on it, what it landed on and whether Shift was held.
#[derive(Clone, Debug)]
struct ReshapePress {
    id: vectorcraft_doc::NodeId,
    at: Point,
    hit: ReshapeHit,
    shift: bool,
    /// The drag so far.
    d: Vec2,
    /// The other focal points the drag moves, found when it starts, before the preview moves them.
    others: Vec<((usize, usize), Point)>,
}

/// How near a focal point's stored position an anchor must be to be that focal point. An anchor
/// added on a segment lies where the curve's point was found, to within rounding.
const FOCAL_MATCH: f64 = 1e-6;

/// A focal point: its `(subpath, anchor)` and where that anchor lies.
type FocalPoint = ((usize, usize), Point);

/// Reshape Tool: a click on a selected path makes the anchor or the point on a segment there the
/// focal point (on a segment it adds an anchor), Shift-click adds or removes more, and a drag
/// moves the focal points with the drag and stretches the selected anchors around them.
#[derive(Default)]
pub struct ReshapeTool {
    /// The press, until the button is released.
    press: Option<ReshapePress>,
    began: bool,
    /// Pointer and offset readout while a reshape drag is in progress.
    measure: Option<(Point, String)>,
    /// The focal points the tool highlighted: their path, and each one's `(subpath, anchor)` and
    /// where that anchor lies. An anchor the tool adds before one renumbers it. A focal point
    /// whose anchor no longer lies there (another edit moved or removed it) shows nothing and
    /// moves nothing.
    focal: Option<(vectorcraft_doc::NodeId, Vec<FocalPoint>)>,
}

impl ReshapeTool {
    /// The selected path under `p` (Reshape works on selected paths only), the point on it and
    /// what the point is.
    fn hit(cx: &ToolContext, p: Point) -> Option<(vectorcraft_doc::NodeId, Point, ReshapeHit)> {
        let tol = cx.tol(4.0);
        Self::paths(cx).into_iter().find_map(|id| {
            let pd = cx.doc.node(id)?.path_data()?;
            let (si, seg, _, q, d) = pd.nearest(p)?;
            if d > tol {
                return None;
            }
            // `path.reshape` takes the anchor within `tol` of the point on the path.
            let anchor = pd
                .subpaths
                .get(si)?
                .anchors
                .iter()
                .enumerate()
                .map(|(i, a)| (i, a.p.distance(q)))
                .filter(|&(_, dist)| dist <= tol)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            Some(match anchor {
                Some((ai, _)) => (id, pd.subpaths.get(si)?.anchors.get(ai)?.p, ReshapeHit::Anchor(si, ai)),
                // `SubPath::insert_anchor` puts the new anchor after the segment's first anchor.
                None => (id, q, ReshapeHit::Segment(si, seg.saturating_add(1))),
            })
        })
    }

    /// The paths Reshape works on: the selected paths that can be edited, in selection order.
    fn paths(cx: &ToolContext) -> Vec<vectorcraft_doc::NodeId> {
        let editable = cx.doc.editable();
        cx.selection.objects.iter().copied().filter(|&id| editable(id) && cx.doc.node(id).is_some_and(|n| n.path_data().is_some())).collect()
    }

    /// The highlighted focal points of path `id` as `(subpath, anchor)`, with their positions: the
    /// anchors of the path that lie where the tool stored them. None while Reshape does not work
    /// on the path.
    fn focal_points(&self, cx: &ToolContext, id: vectorcraft_doc::NodeId) -> Vec<((usize, usize), Point)> {
        let Some((fid, points)) = &self.focal else { return vec![] };
        if *fid != id || !Self::paths(cx).contains(&id) {
            return vec![];
        }
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { return vec![] };
        let mut out: Vec<((usize, usize), Point)> = vec![];
        for &((si, ai), p) in points {
            let here = pd.subpaths.get(si).and_then(|sp| sp.anchors.get(ai)).map(|a| a.p);
            if let Some(q) = here.filter(|q| q.distance(p) <= FOCAL_MATCH)
                && !out.iter().any(|(o, _)| *o == (si, ai))
            {
                out.push(((si, ai), q));
            }
        }
        out
    }

    /// The focal points other than the pressed point that a drag from `press` moves: all the
    /// highlighted ones when the press is on one of them, else none (Shift only keeps the drag's
    /// angle).
    fn dragged_focal(&self, cx: &ToolContext, press: &ReshapePress) -> Vec<((usize, usize), Point)> {
        let focal = self.focal_points(cx, press.id);
        let pressed = |r: &(usize, usize)| matches!(press.hit, ReshapeHit::Anchor(si, ai) if (si, ai) == *r);
        if focal.iter().any(|(r, _)| pressed(r)) { focal.into_iter().filter(|(r, _)| !pressed(r)).collect() } else { vec![] }
    }

    /// `path.reshape`'s params for `press` dragged by `d`.
    fn params(cx: &ToolContext, press: &ReshapePress, d: Vec2) -> Value {
        let others: Vec<[usize; 2]> = press.others.iter().map(|&((si, ai), _)| [si, ai]).collect();
        // `focal` is sent when empty too: without it, `path.reshape` follows the earlier rule.
        json!({ "id": press.id.0, "x": press.at.x, "y": press.at.y, "dx": d.x, "dy": d.y, "tol": cx.tol(4.0), "focal": others })
    }
}

impl Tool for ReshapeTool {
    fn id(&self) -> &'static str {
        "reshape"
    }
    fn busy(&self) -> bool {
        self.press.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.press =
                    Self::hit(cx, ev.pos).map(|(id, at, hit)| ReshapePress { id, at, hit, shift: ev.mods.shift, d: Vec2::ZERO, others: vec![] });
                self.began = false;
                self.measure = None;
                vec![]
            }
            PointerKind::Drag => {
                let Some(at) = self.press.as_ref().map(|p| p.at) else { return vec![] };
                let mut d: Vec2 = ev.pos - at;
                if ev.mods.shift {
                    d = vectorcraft_geom::constrain_angle_from(d, 45.0, cx.constrain_angle);
                }
                let mut out = vec![];
                if !self.began {
                    if d.hypot() < cx.tol(2.0) {
                        return out;
                    }
                    self.began = true;
                    let others = self.press.as_ref().map(|p| self.dragged_focal(cx, p)).unwrap_or_default();
                    if let Some(p) = self.press.as_mut() {
                        p.others = others;
                    }
                    out.push(Action::Begin("Reshape".into()));
                }
                let Some(press) = self.press.as_mut() else { return out };
                press.d = d;
                self.measure = cx.measurement_labels.then(|| (ev.pos + Vec2::new(cx.tol(12.0), cx.tol(12.0)), cx.offset_label(d.x, d.y)));
                out.push(Action::Preview("path.reshape".into(), Self::params(cx, press, d)));
                out
            }
            PointerKind::Up => {
                self.measure = None;
                let Some(press) = self.press.take() else { return vec![] };
                if std::mem::take(&mut self.began) {
                    // The focal points move with the whole drag, the grabbed point too, and are
                    // renumbered after an anchor added on a segment.
                    let mut points: Vec<((usize, usize), Point)> = press.others.iter().map(|&(r, p)| (press.hit.renumber(r), p + press.d)).collect();
                    points.push((press.hit.anchor(), press.at + press.d));
                    self.focal = Some((press.id, points));
                    return vec![Action::Commit];
                }
                // A click: the point becomes the focal point, or with Shift is added to them or,
                // when it is one already, removed.
                let mut points: Vec<((usize, usize), Point)> = if press.shift { self.focal_points(cx, press.id) } else { vec![] };
                let mut out = vec![];
                match press.hit {
                    ReshapeHit::Anchor(si, ai) => {
                        let before = points.len();
                        points.retain(|&(r, _)| r != (si, ai));
                        if points.len() == before {
                            points.push(((si, ai), press.at));
                        }
                    }
                    ReshapeHit::Segment(..) => {
                        // A reshape by no distance adds the anchor and keeps the selection as it is.
                        out.push(Action::Exec("path.reshape".into(), Self::params(cx, &press, Vec2::ZERO)));
                        for (r, _) in &mut points {
                            *r = press.hit.renumber(*r);
                        }
                        points.push((press.hit.anchor(), press.at));
                    }
                }
                self.focal = (!points.is_empty()).then_some((press.id, points));
                out
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.press.take().is_some() {
            self.measure = None;
            if std::mem::take(&mut self.began) {
                return vec![Action::Cancel];
            }
        }
        vec![]
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.focal = None;
        self.measure = None;
        if self.press.take().is_some() && std::mem::take(&mut self.began) { vec![Action::Cancel] } else { vec![] }
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        // Each focal point: a filled anchor square inside a larger hollow one.
        if let Some((id, _)) = &self.focal {
            let color = cx.doc.layer_color(*id);
            for (_, p) in self.focal_points(cx, *id) {
                out.push(Overlay::Anchor { p, color, filled: false, size: 10.0 });
                out.push(Overlay::Anchor { p, color, filled: true, size: 5.0 });
            }
        }
        if cx.measurement_labels
            && let Some((p, t)) = &self.measure
        {
            out.push(Overlay::Measure { p: *p, text: t.clone() });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.press.is_some() || Self::hit(cx, p).is_some() { Cursor::Move } else { Cursor::Arrow }
    }
}

/// Shaper Tool: draw a rough shape and it becomes a clean live shape (rectangle, ellipse,
/// triangle/polygon, line); scribble over art to delete it.
#[derive(Default)]
pub struct ShaperTool {
    points: Vec<Point>,
}

impl Tool for ShaperTool {
    fn id(&self) -> &'static str {
        "shaper"
    }
    fn busy(&self) -> bool {
        !self.points.is_empty()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        use vectorcraft_geom::recognize::{Recognized, recognize};
        match ev.kind {
            PointerKind::Down => {
                if let Some(a) = crate::shaperwidget::click(cx, ev.pos) {
                    self.points.clear();
                    return vec![a];
                }
                self.points = vec![ev.pos];
                vec![]
            }
            PointerKind::Drag => {
                if !self.points.is_empty() && self.points.len() < 4095 && self.points.last().is_none_or(|l| l.distance(ev.pos) >= cx.tol(1.5)) {
                    self.points.push(ev.pos);
                }
                vec![]
            }
            PointerKind::Up => {
                // A fast final segment may arrive only on release; it still closes the stroke.
                if !self.points.is_empty() && self.points.last().is_none_or(|p| *p != ev.pos) {
                    self.points.push(ev.pos);
                }
                let pts = std::mem::take(&mut self.points);
                if pts.first().is_some_and(|first| pts.iter().all(|p| p.distance(*first) < cx.tol(3.0))) {
                    // In Construction mode a tap selects a source shape. On a selected group, a tap on a
                    // source shape's outline enters Construction mode.
                    let construction =
                        cx.isolation.and_then(|id| cx.doc.node(id)).is_some_and(|n| n.name.as_deref() == Some(vectorcraft_doc::shaper::SOURCES));
                    let mut params = json!({"point": [ev.pos.x, ev.pos.y], "tolerance": cx.tol(3.0).min(1000.0)});
                    if construction {
                        params["source"] = json!(true);
                    }
                    return vec![Action::Exec("shaper.select".into(), params)];
                }
                let r = |x: vectorcraft_geom::Rect, rotation: f64| json!({ "x": x.x0, "y": x.y0, "width": x.width(), "height": x.height(), "rotation": rotation });
                match recognize(&pts) {
                    Some(Recognized::Line { a, b }) => vec![Action::Exec("shape.line".into(), json!({ "x1": a.x, "y1": a.y, "x2": b.x, "y2": b.y }))],
                    Some(Recognized::Rectangle { rect, rotation }) => vec![Action::Exec("shape.rectangle".into(), r(rect, rotation))],
                    Some(Recognized::Ellipse { rect, rotation }) => vec![Action::Exec("shape.ellipse".into(), r(rect, rotation))],
                    Some(Recognized::Polygon { center, radius, sides, rotation }) => {
                        vec![Action::Exec(
                            "shape.polygon".into(),
                            json!({ "cx": center.x, "cy": center.y, "radius": radius, "sides": sides, "rotation": rotation }),
                        )]
                    }
                    Some(Recognized::Scribble(_)) => vec![Action::Exec(
                        "shaper.scribble".into(),
                        json!({"points": pts.iter().map(|p| [p.x, p.y]).collect::<Vec<_>>(), "tolerance": cx.tol(4.0)}),
                    )],
                    None => vec![],
                }
            }
            // The mode widget answers each click of a double-click on it.
            PointerKind::DoubleClick if crate::shaperwidget::over_widget(cx, ev.pos) => vec![],
            PointerKind::DoubleClick => vec![
                Action::Exec("shaper.select".into(), json!({"point": [ev.pos.x, ev.pos.y], "source": true})),
                Action::SwitchTool("selection".into()),
            ],
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.points.clear();
        }
        vec![]
    }
    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        if self.points.len() < 2 {
            return vec![];
        }
        vec![Overlay::Path { path: crate::draw2::polyline(&self.points), color: BLUE, width: 1.5, dashed: false }]
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Shaper
    }
}

/// Graph tools: drag the plot rectangle (Shift = square, Alt = from the centre); a click opens the
/// size dialog. A new graph opens the Graph Data dialog.
pub struct GraphTool {
    id: &'static str,
    start: Option<Point>,
    last: Point,
    mods: Mods,
    began: bool,
}

impl GraphTool {
    fn new(id: &str) -> Self {
        let id = crate::catalog::tool_info(id).map(|t| t.id).unwrap_or("columnGraph");
        Self { id, start: None, last: Point::ZERO, mods: Mods::default(), began: false }
    }
    fn kind(&self) -> &'static str {
        vectorcraft_doc::GraphKind::parse(self.id).unwrap_or_default().id()
    }
    /// The plot rectangle a drag from `s` to `p` draws (Shift = square, Alt = from the centre).
    fn plot_rect(s: Point, p: Point, m: Mods) -> vectorcraft_geom::Rect {
        let mut d = p - s;
        if m.shift {
            let k = d.x.abs().max(d.y.abs());
            d = Vec2::new(k * d.x.signum(), k * d.y.signum());
        }
        if m.alt { vectorcraft_geom::Rect::from_points(s - d, s + d) } else { vectorcraft_geom::Rect::from_points(s, s + d) }
    }
    fn params(&self, s: Point, p: Point, m: Mods) -> Value {
        let r = Self::plot_rect(s, p, m);
        json!({ "type": self.kind(), "x": r.x0, "y": r.y0, "width": r.width().max(1.0), "height": r.height().max(1.0) })
    }
}

impl Tool for GraphTool {
    fn id(&self) -> &'static str {
        self.id
    }
    fn busy(&self) -> bool {
        self.start.is_some()
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        match ev.kind {
            PointerKind::Down => {
                self.start = Some(ev.pos);
                self.last = ev.pos;
                self.began = false;
                vec![]
            }
            PointerKind::Drag => {
                let Some(s) = self.start else { return vec![] };
                self.last = ev.pos;
                self.mods = ev.mods;
                let mut out = vec![];
                if !self.began {
                    if ev.pos.distance(s) < cx.tol(3.0) {
                        return out;
                    }
                    self.began = true;
                    out.push(Action::Begin("Graph".into()));
                }
                out.push(Action::Preview("graph.create".into(), self.params(s, ev.pos, ev.mods)));
                out
            }
            PointerKind::Up => {
                let Some(s) = self.start.take() else { return vec![] };
                if std::mem::take(&mut self.began) {
                    vec![
                        Action::Preview("graph.create".into(), self.params(s, ev.pos, ev.mods)),
                        Action::Commit,
                        Action::Dialog("graphData".into(), json!({})),
                    ]
                } else {
                    vec![Action::Dialog("graph".into(), json!({ "x": s.x, "y": s.y, "type": self.kind() }))]
                }
            }
            _ => vec![],
        }
    }
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.start.take().is_some() && std::mem::take(&mut self.began) {
            return vec![Action::Cancel];
        }
        vec![]
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if let Some(s) = self.start.filter(|_| self.began && cx.measurement_labels) {
            let r = Self::plot_rect(s, self.last, self.mods);
            vec![Overlay::Measure { p: self.last, text: cx.size_label(r.width(), r.height()) }]
        } else {
            vec![]
        }
    }
    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The Flare Tool Options: kept within their ranges, and what the next drag draws with.
    #[test]
    fn flare_options_last_and_draw_the_next_flare() {
        let mut t = FlareTool::default();
        assert_eq!(t.options()["rays"], json!(15));
        for (k, v) in [("diameter", json!(5000)), ("rays", json!(7.6)), ("opacity", json!("x")), ("ringsOn", json!(false)), ("nope", json!(1))] {
            t.set_option(k, &v);
        }
        let o = t.options();
        assert_eq!(
            (o["diameter"].as_f64(), o["rays"].as_u64(), o["opacity"].as_f64(), o["ringsOn"].as_bool()),
            (Some(1000.0), Some(8), Some(50.0), Some(false))
        );
        assert!(o.get("nope").is_none());
        // A drag sets the centre and diameter; the options give the rest, rings off.
        let p = t.params(Point::new(10.0, 20.0), 40.0, None);
        assert_eq!((p["cx"].as_f64(), p["diameter"].as_f64(), p["rays"].as_u64(), p["rings"].as_u64()), (Some(10.0), Some(40.0), Some(8), Some(0)));
        assert_eq!(p["longest"].as_f64(), Some(300.0));
        t.set_option("raysOn", &json!(false));
        assert_eq!(t.params(Point::ZERO, 1.0, None)["rays"].as_u64(), Some(0));
    }

    /// The last `shape.flare` preview of `a`.
    fn flare_preview(a: &[Action]) -> Value {
        match a.iter().rev().find(|a| matches!(a, Action::Preview(..))) {
            Some(Action::Preview(c, v)) if c == "shape.flare" => v.clone(),
            other => panic!("no flare preview: {other:?}"),
        }
    }

    /// Alt-click (Option-click on macOS): with Alt held when the button goes down, a release without
    /// a drag draws one flare at the click with the options as they are, Rays and Rings off included,
    /// and opens no dialog. A plain click, or Alt pressed only for the release, opens the Flare Tool
    /// Options; an Alt drag draws the flare in two stages as any drag does; Escape drops the press.
    #[test]
    fn alt_click_draws_a_flare_with_the_options_at_once() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let click = |t: &mut FlareTool, down: Mods, up: Mods| {
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 120.0, 140.0).with_mods(down));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 140.0).with_mods(up))
        };
        let mut t = FlareTool::default();
        for (k, v) in [("diameter", json!(40)), ("rays", json!(7)), ("growth", json!(80)), ("ringsOn", json!(false))] {
            t.set_option(k, &v);
        }
        let options = t.options();
        let a = click(&mut t, alt, alt);
        let c = Point::new(120.0, 140.0);
        let v = flare_params(&options, c);
        assert_eq!(a, vec![Action::Exec("shape.flare".into(), v.clone())]);
        assert_eq!(
            (v["cx"].as_f64(), v["cy"].as_f64(), v["diameter"].as_f64(), v["rays"].as_u64(), v["growth"].as_f64(), v["rings"].as_u64()),
            (Some(120.0), Some(140.0), Some(40.0), Some(7), Some(80.0), Some(0))
        );
        assert_eq!(t.options(), options, "the click leaves the options as they are");
        assert!(!t.busy());
        // Alt let go before the release still counts; Alt pressed only for the release does not.
        assert_eq!(click(&mut t, alt, Mods::default()), vec![Action::Exec("shape.flare".into(), v)]);
        let dialog = vec![Action::Dialog("flare".into(), json!({ "x": 120.0, "y": 140.0 }))];
        assert_eq!(click(&mut t, Mods::default(), alt), dialog);
        assert_eq!(click(&mut t, Mods::default(), Mods::default()), dialog, "a plain click after an Alt-click opens the dialog");
        // An Alt drag places the center and waits for the rings' end point, as any drag does.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0).with_mods(alt));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0).with_mods(alt)).first(), Some(&Action::Begin("Flare".into())));
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 100.0).with_mods(alt)), vec![]);
        assert!(t.placed.is_some());
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 200.0).with_mods(alt));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 200.0).with_mods(alt));
        assert_eq!(a.last(), Some(&Action::Commit));
        assert_eq!((flare_preview(&a)["diameter"].as_f64(), flare_preview(&a)["x2"].as_f64()), (Some(80.0), Some(300.0)));
        // Escape before the release drops the press: no flare and no dialog.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 120.0, 140.0).with_mods(alt));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![]);
        assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 140.0).with_mods(alt)), vec![]);
    }

    /// With the perspective grid showing and a plane active, an Alt-click draws the flare on that
    /// plane from the click (`perspective.draw` with `at`), as the click-opened dialog does.
    #[test]
    fn alt_click_draws_on_the_active_perspective_plane() {
        use crate::distort::perspective::{PerspectiveGrid, Plane};
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (mut d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let alt = Mods { alt: true, ..Default::default() };
        let c = Point::new(300.0, 380.0);
        let mut t = FlareTool::default();
        PerspectiveGrid { visible: true, plane: Plane::Left, ..PerspectiveGrid::current(&d) }.store(&mut d);
        let mut alt_click = |d: &vectorcraft_doc::Document| {
            let cx = cx(d, &s, &p);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, c.x, c.y).with_mods(alt));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, c.x, c.y).with_mods(alt))
        };
        let params = flare_params(&FlareTool::default().options(), c);
        let on_plane = json!({ "command": "shape.flare", "params": params, "at": [c.x, c.y] });
        assert_eq!(alt_click(&d), vec![Action::Exec("perspective.draw".into(), on_plane)]);
        // With no active plane, or the grid hidden, the flare is drawn flat.
        for g in [
            PerspectiveGrid { plane: Plane::None, ..PerspectiveGrid::current(&d) },
            PerspectiveGrid { visible: false, ..PerspectiveGrid::current(&d) },
        ] {
            let mut d = d.clone();
            g.store(&mut d);
            assert_eq!(alt_click(&d), vec![Action::Exec("shape.flare".into(), params.clone())]);
        }
    }

    /// Equal vertical travel gives equal rotation, regardless of event partitioning.
    #[test]
    fn flare_drag_turns_the_rays_and_shift_holds_their_turn() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let space = Mods { space: true, ..Default::default() };
        let mut results = vec![];
        for steps in [1, 8, 64] {
            let mut t = FlareTool::default();
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
            for i in 1..=steps {
                t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0 + 80.0 * i as f64 / steps as f64));
            }
            assert_eq!(t.turn, 80.0);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 180.0));
            assert_eq!(t.turn, 80.0, "horizontal movement doesn't turn");
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 150.0));
            assert_eq!(t.turn, 50.0, "reverse travel subtracts its distance");
            let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 200.0).with_mods(shift)));
            assert_eq!(v["rotation"].as_f64(), Some(50.0));
            assert_eq!(v["diameter"].as_f64(), Some(200.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 220.0));
            assert_eq!(t.turn, 70.0, "release does not catch up frozen movement");
            t.key(&cx, ToolKey::Up, Mods::default());
            assert_eq!(t.turn, 70.0);
            t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 120.0, 220.0));
            t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 200.0));
            let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 260.0));
            assert_eq!(a.last(), Some(&Action::Commit));
            assert_eq!(flare_preview(&a)["rotation"].as_f64(), Some(70.0));
            results.push(t.turn);
        }
        assert!(results.windows(2).all(|p| p[0] == p[1]));
        // Space held before the first drag update freezes rotation too.
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 140.0).with_mods(space));
        assert_eq!(t.turn, 0.0);
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 150.0));
        assert_eq!(t.turn, 10.0);
    }

    /// With one flare selected, the tool shows its center and end handles where the flare is now
    /// drawn, and dragging either edits the flare (`shape.editFlare`) as one undo step; a press
    /// away from them draws a new flare.
    #[test]
    fn dragging_a_selected_flares_handles_changes_its_length_and_direction() {
        use crate::testutil::*;
        use vectorcraft_doc::flare::{FlareSpec, part_node, parts};
        use vectorcraft_doc::{Node, Selection};
        let (mut d, _) = doc_with_rect();
        let gid = d.alloc_id();
        let v = json!({ "cx": 100.0, "cy": 100.0, "x2": 300.0, "y2": 100.0, "rays": 3, "rings": 2, "seed": 5, "diameter": 30.0,
            "opacity": 50.0, "brightness": 30.0, "growth": 20.0, "fuzziness": 50.0, "longest": 300.0, "rayFuzziness": 100.0,
            "rotation": 0.0, "largest": 50.0 });
        let xf = vectorcraft_geom::Affine::translate((10.0, 20.0));
        let children: Vec<_> = parts(&v).into_iter().map(|part| std::sync::Arc::new(part_node(d.alloc_id(), part, xf, false))).collect();
        let edited = children[0].id;
        let mut g = Node::group(gid, children);
        g.extras_mut().flare = Some(Box::new(FlareSpec { params: v, transform: xf }));
        let layer = d.layers[0].id;
        d.insert(Some(layer), usize::MAX, g).unwrap_or_else(|e| panic!("{e}"));
        let s = Selection { objects: vec![gid], ..Default::default() };
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = FlareTool::default();
        let ov = t.overlays(&cx);
        let anchors: Vec<Point> = ov.iter().filter_map(|o| if let Overlay::Anchor { p, .. } = o { Some(*p) } else { None }).collect();
        assert_eq!(anchors, vec![Point::new(110.0, 120.0), Point::new(310.0, 120.0)]);
        // The end handle, grabbed 2 points off: it moves as far as the pointer.
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 312.0, 120.0)).is_empty());
        assert!(t.busy());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 312.0, 160.0));
        assert_eq!(a.first(), Some(&Action::Begin("Edit Flare".into())));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 302.0, 200.0));
        assert_eq!(a, vec![Action::Preview("shape.editFlare".into(), json!({ "ids": [gid.0], "x2": 300.0, "y2": 200.0 })), Action::Commit]);
        // The center handle; Escape cancels its drag.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 121.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 121.0));
        assert_eq!(a.last(), Some(&Action::Preview("shape.editFlare".into(), json!({ "ids": [gid.0], "cx": 150.0, "cy": 120.0 }))));
        assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
        assert!(!t.busy());
        // A click on a handle without a drag is a click with the tool: it opens the dialog, and
        // with Alt held it draws a flare there with the tool's options.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 110.0, 120.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 110.0, 120.0));
        assert_eq!(a, vec![Action::Dialog("flare".into(), json!({ "x": 110.0, "y": 120.0 }))]);
        assert!(!t.busy());
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 310.0, 121.0).with_mods(alt));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 310.0, 121.0).with_mods(alt));
        assert_eq!(a, vec![Action::Exec("shape.flare".into(), flare_params(&t.options(), Point::new(310.0, 121.0)))]);
        assert!(!t.busy());
        // Away from the handles a drag draws a new flare.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 200.0, 300.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 240.0, 300.0));
        assert_eq!(a.first(), Some(&Action::Begin("Flare".into())));
        // Without a flare selected, no handles show.
        let none = Selection::default();
        assert!(FlareTool::default().overlays(&crate::testutil::cx(&d, &none, &p)).is_empty());
        // A flare with an edited part is no longer edited as a flare: no handles show.
        d.node_mut(edited).unwrap_or_else(|| panic!("no part")).opacity = 0.25;
        assert!(FlareTool::default().overlays(&crate::testutil::cx(&d, &s, &p)).is_empty());
    }

    /// Space held while dragging out a flare moves it at its current size and turn, in the center
    /// drag and in the drag that places the rings' end point; let go, the drag goes on from there.
    #[test]
    fn space_moves_the_flare_being_drawn() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let space = Mods { space: true, ..Default::default() };
        let step = 30.0;
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 130.0));
        // Space held: the pointer's y grows, but the flare moves without turning.
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 180.0).with_mods(space)));
        let at = |v: &Value| (v["cx"].as_f64(), v["cy"].as_f64(), v["diameter"].as_f64());
        let turned = |v: &Value, by: f64| v["rotation"].as_f64().is_some_and(|r| (r - by).abs() < 1e-9);
        assert_eq!(at(&v), (Some(150.0), Some(150.0), Some(60.0)));
        assert!(turned(&v, step));
        // Space let go: the size follows the pointer again, from the moved center, and so does the turn.
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 190.0)));
        assert_eq!(at(&v), (Some(150.0), Some(150.0), Some(80.0)));
        assert!(turned(&v, step + 10.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 150.0, 190.0));
        // The second drag: Space held moves the flare, its rings' end point with it.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 320.0, 310.0).with_mods(space)));
        assert_eq!((at(&v), v["x2"].as_f64(), v["y2"].as_f64()), ((Some(170.0), Some(160.0), Some(80.0)), Some(320.0), Some(310.0)));
        assert!(turned(&v, step + 10.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 340.0, 310.0));
        assert_eq!(a.last(), Some(&Action::Commit));
        let v = flare_preview(&a);
        assert_eq!((v["cx"].as_f64(), v["cy"].as_f64(), v["x2"].as_f64()), (Some(170.0), Some(160.0), Some(340.0)));
        // A release with Space held away from the last drag moves the flare there too.
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 130.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 130.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 320.0, 300.0));
        let v = flare_preview(&t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 340.0, 310.0).with_mods(space)));
        assert_eq!((v["cx"].as_f64(), v["cy"].as_f64(), v["x2"].as_f64()), (Some(120.0), Some(110.0), Some(340.0)));
    }

    /// ↑/↓ change the rays in the center drag and the rings while the rings' end point is placed,
    /// within the options' 0 to 50; with Rings off they leave the second drag alone.
    #[test]
    fn flare_arrows_change_the_rays_then_the_rings() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let counts = |v: &Value| (v["rays"].as_u64(), v["rings"].as_u64());
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0));
        assert_eq!(counts(&flare_preview(&t.key(&cx, ToolKey::Up, Mods::default()))), (Some(16), Some(10)));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 320.0, 300.0));
        assert_eq!(counts(&flare_preview(&t.key(&cx, ToolKey::Up, Mods::default()))), (Some(16), Some(11)));
        t.key(&cx, ToolKey::Down, Mods::default());
        assert_eq!(counts(&flare_preview(&t.key(&cx, ToolKey::Down, Mods::default()))), (Some(16), Some(9)));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 330.0, 300.0));
        assert_eq!(a.last(), Some(&Action::Commit));
        assert_eq!(counts(&flare_preview(&a)), (Some(16), Some(9)));
        // The next flare starts from the options' counts; the counts stop at 0 and 50.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 100.0));
        let mut v = Value::Null;
        for _ in 0..12 {
            v = flare_preview(&t.key(&cx, ToolKey::Down, Mods::default()));
        }
        assert_eq!(counts(&v), (Some(15), Some(0)));
        for _ in 0..60 {
            v = flare_preview(&t.key(&cx, ToolKey::Up, Mods::default()));
        }
        assert_eq!(counts(&v), (Some(15), Some(50)));
        t.key(&cx, ToolKey::Escape, Mods::default());
        // Rings off: the arrows do nothing in the second drag.
        t.set_option("ringsOn", &json!(false));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 100.0));
        assert!(t.key(&cx, ToolKey::Up, Mods::default()).is_empty());
    }

    /// ~ (backquote) pressed while the rings' end point is placed gives the rings a new placement
    /// each time, which the flare drawn keeps; holding it, the center drag, Rings off and the next
    /// flare give none.
    #[test]
    fn flare_tilde_places_the_rings_anew() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let quote = |kind, x, y, held| PointerEvent { backquote: held, ..PointerEvent::new(kind, x, y) };
        let seed = |a: &[Action]| flare_preview(a).get("ringSeed").and_then(Value::as_u64);
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        // In the center drag ~ does nothing.
        assert_eq!(seed(&t.pointer(&cx, &quote(PointerKind::Drag, 140.0, 100.0, true))), None);
        t.pointer(&cx, &quote(PointerKind::Drag, 141.0, 100.0, false));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 141.0, 100.0));
        assert_eq!(seed(&t.pointer(&cx, &PointerEvent::new(PointerKind::Move, 200.0, 200.0))), None);
        let first = seed(&t.pointer(&cx, &quote(PointerKind::Move, 200.0, 200.0, true)));
        assert!(first.is_some_and(|s| s < 1 << 53));
        // Held, it places them once; pressed again, anew.
        assert_eq!(seed(&t.pointer(&cx, &quote(PointerKind::Move, 210.0, 200.0, true))), first);
        t.pointer(&cx, &quote(PointerKind::Down, 210.0, 200.0, false));
        t.pointer(&cx, &quote(PointerKind::Drag, 220.0, 200.0, false));
        let second = seed(&t.pointer(&cx, &quote(PointerKind::Drag, 220.0, 200.0, true)));
        assert!(second.is_some() && second != first);
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 230.0, 200.0));
        assert_eq!((a.last(), seed(&a)), (Some(&Action::Commit), second));
        // The next flare starts without one, and with Rings off ~ does not change the flare.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(seed(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0))), None);
        t.set_option("ringsOn", &json!(false));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 140.0, 100.0));
        assert_eq!(seed(&t.pointer(&cx, &quote(PointerKind::Move, 200.0, 200.0, true))), None);
        assert_eq!(t.ring_seed, None);
    }

    /// Ctrl/Cmd in the center drag keeps the center's diameter as the last drag showed it, and the
    /// drag sizes the halo through the Halo growth (0 to 300 %); let go, the drag sizes the center
    /// again and the flare keeps the new growth.
    #[test]
    fn flare_ctrl_keeps_the_center_and_the_drag_sizes_the_halo() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let cmd = Mods { cmd: true, ..Default::default() };
        let sized = |a: &[Action]| {
            let v = flare_preview(a);
            (v["diameter"].as_f64().unwrap_or(0.0), v["growth"].as_f64().unwrap_or(0.0))
        };
        let mut t = FlareTool::default();
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 140.0, 100.0))), (80.0, 20.0));
        // The key goes down with the pointer's next move: the center stays as the last drag showed it.
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 180.0, 100.0).with_mods(cmd))), (80.0, 140.0));
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 120.0, 100.0).with_mods(cmd))), (80.0, 0.0));
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 900.0, 100.0).with_mods(cmd))), (80.0, 300.0));
        assert_eq!(sized(&t.key(&cx, ToolKey::Up, Mods::default())), (80.0, 300.0));
        // Let go: the center follows the drag again and the growth stays.
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 150.0, 100.0))), (100.0, 300.0));
        // Pressed again, the hold starts from the new center and growth.
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 125.0, 100.0).with_mods(cmd))), (100.0, 100.0));
        // A release with the key held places the held center; the second drag draws with it.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 125.0, 100.0).with_mods(cmd));
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 300.0, 300.0));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Up, 300.0, 330.0));
        assert_eq!((a.last(), sized(&a)), (Some(&Action::Commit), (100.0, 100.0)));
        // With the key held from the press, the center keeps the first size the flare shows; the
        // next flare starts from the options' growth.
        t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 100.0));
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 110.0).with_mods(cmd))), (20.0, 20.0));
        assert_eq!(sized(&t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, 100.0, 120.0).with_mods(cmd))), (20.0, 140.0));
    }

    #[test]
    fn reshape_drag_honors_measurement_labels() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, id) = doc_with_rect();
        let s = Selection { objects: vec![id], ..Selection::default() };
        let p = paint();
        let mut t = ReshapeTool::default();
        let drag = |t: &mut ReshapeTool, c: &ToolContext| {
            t.pointer(c, &PointerEvent::new(PointerKind::Down, 100.0, 150.0));
            t.pointer(c, &PointerEvent::new(PointerKind::Drag, 130.0, 150.0));
        };
        let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
        drag(&mut t, &on);
        let want = on.offset_label(30.0, 0.0);
        assert!(t.overlays(&on).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text == &want)));
        let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        assert!(!t.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })), "overlay follows the current pref");
        let mut quiet = ReshapeTool::default();
        drag(&mut quiet, &off);
        assert!(!quiet.overlays(&off).iter().any(|o| matches!(o, Overlay::Measure { .. })));
    }

    #[test]
    fn graph_plot_measure_labels() {
        use crate::testutil::*;
        use vectorcraft_doc::Selection;

        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let measure_count = |t: &GraphTool, c: &ToolContext| t.overlays(c).iter().filter(|o| matches!(o, Overlay::Measure { .. })).count();
        let measure_text = |t: &GraphTool, c: &ToolContext| {
            t.overlays(c).iter().find_map(|o| match o {
                Overlay::Measure { text, .. } => Some(text.clone()),
                _ => None,
            })
        };
        let mut t = GraphTool::new("columnGraph");
        let drag = |t: &mut GraphTool, c: &ToolContext| {
            t.pointer(c, &PointerEvent::new(PointerKind::Down, 10.0, 10.0));
            t.pointer(c, &PointerEvent::new(PointerKind::Drag, 110.0, 60.0));
        };
        let on = ToolContext { measurement_labels: true, ..cx(&d, &s, &p) };
        drag(&mut t, &on);
        assert_eq!(measure_count(&t, &on), 1);
        let text = measure_text(&t, &on).unwrap_or_default();
        assert!(text.starts_with("W: "), "{text}");
        assert!(text.contains("\nH: "), "{text}");
        let off = ToolContext { measurement_labels: false, ..cx(&d, &s, &p) };
        let mut quiet = GraphTool::new("columnGraph");
        drag(&mut quiet, &off);
        assert_eq!(measure_count(&quiet, &off), 0);
    }

    #[test]
    fn create_covers_both() {
        assert_eq!(create("flare").unwrap().id(), "flare");
        assert_eq!(create("reshape").unwrap().id(), "reshape");
        assert_eq!(create("shaper").unwrap().id(), "shaper");
        assert!(create("pen").is_none());
        for g in
            ["columnGraph", "stackedColumnGraph", "barGraph", "stackedBarGraph", "lineGraph", "areaGraph", "scatterGraph", "pieGraph", "radarGraph"]
        {
            assert_eq!(create(g).unwrap().id(), g);
        }
    }
}
