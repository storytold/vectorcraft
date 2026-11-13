//! The Stroke panel: weight, cap, join, alignment, dashes, arrowheads, width profiles and brushes.
//! With type selected it edits the characters' stroke (weight, cap, join, miter limit, dashes).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{
    ArrowAlign, Arrowhead, CharStyle, CustomArrowhead, Dash, LineCap, LineJoin, Node, NodeKind, SavedProfile, Scaling, StrokeAlign, StrokeLayer,
    Unit, WidthProfile,
};
use vectorcraft_geom::PathData;

use super::appearance::{ItemTarget, edit_items, item_target};
use super::paint::painted;
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, ok, str_param};
use crate::DocState;
use crate::inspect::{ALIGNS, CAPS, JOINS, StrokeMixed, node_stroke, scale_layer};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "stroke.set",
            "Stroke Options",
            ["Window", "Stroke"],
            None,
            "{weight?: pt, cap?: butt|round|square, join?: miter|round|bevel, miterLimit?, align?: center|inside|outside, dash?: [d,g,…]|null (a 0 dash with a round or projecting cap draws dots or squares; a new pattern keeps the current offset and alignment), dashOffset? (exact dashes only), alignDashes?: bool (true: dashes fitted to corners and path ends, every run between them holding whole periods with a dash centred on each corner and end; false, the default for a new pattern: exact lengths), startArrow?, endArrow?: Arrow|ArrowOpen|LineArrow|Barbed|Concave|DoubleArrow|HalfArrowLeft|HalfArrowRight|Chevron|Feather|Swallowtail|Triangle|TriangleOpen|TriangleReverse|TriangleBar|Kite|Leaf|Drop|Circle|CircleOpen|HalfCircle|Oval|Target|Square|SquareOpen|Tag|TagOpen|Diamond|DiamondOpen|Hexagon|HexagonOpen|Star|Cross|Plus|Bar|DoubleBar|DotOnBar|Slash|DoubleSlash|Bracket|Fork|null|{custom: name} (a custom arrowhead of the arrowheads file, stroke.arrowheads.list)|{Custom: {name, path}} (a custom head with its art, as document.inspect reports one; refused when the path fills no area, parts thinner than a millionth of a head unit counting as none, or has more than 4096 anchors, more than 4096 subpaths or a point more than 64 head units from the tip) (HalfArrowLeft/Right: one barb, left or right of the direction the head points), arrowAlign?: \"extend\" (tip past the end point, default)|\"tip\" (tip on the end point; the stroke is shortened), profile?: \"uniform\"|\"lens\"|\"taperStart\"|\"taperEnd\"|\"pinch\"|\"teardrop\"|\"wave\"|a saved profile's name (stroke.widthProfile.list), widthPoints?, gradientMode? (as document.inspect reports them in strokeOptions, so those read back unchanged; ignored here: a custom profile is set with the Width tool, gradientMode with paint.editGradient), item?: stroke item index|null (omitted: the Appearance panel's active item if it is a stroke, else the top stroke, created when missing), ids?|id?, target?: \"object\"|\"contents\", aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} Without a stroke item, type takes weight, cap, join, miterLimit, align (around its glyph outlines; the type stays live) and the dash options as its characters' stroke (every run; text.setRangeStyle `strokeOptions` styles a range), and images and symbol instances (also in groups) are left alone. With nothing selected (and no ids) they set up the next object drawn (appearance.newArt; weight always does). On a path directly in a graph series, the graph also stores a weight it changes on the topmost visible stroke (docs/mcp.md) → {objects: how many objects took the stroke options}",
            has_doc,
            stroke_set
        ),
        cmd!(
            "stroke.weightUp",
            "Stroke Weight Up",
            [],
            Some("Cmd+Alt+."),
            "{ids?|id?} raise each selected stroke's weight by one step of Units › Stroke (a quarter unit below one unit, then whole units; up to 1000 pt), each from its own weight: the selected objects' active or top stroke, type's stroked characters; objects without a stroke are left alone. With nothing selected (and no ids), the weight of the next object drawn. On a path directly in a graph series, the graph also stores the new weight of the topmost visible stroke (docs/mcp.md) → {objects}",
            has_doc,
            |s, p| stroke_step(s, p, true)
        ),
        cmd!(
            "stroke.weightDown",
            "Stroke Weight Down",
            [],
            Some("Cmd+Alt+,"),
            "{ids?|id?} lower each selected stroke's weight by one step, as stroke.weightUp raises it, down to a quarter unit (a thinner weight is kept) → {objects}",
            has_doc,
            |s, p| stroke_step(s, p, false)
        ),
        cmd!(
            "stroke.setAdvanced",
            "Stroke Options",
            [],
            None,
            "{arrowScale?: [start %, end %], swapArrows?: bool, flipProfile?: \"along\"|\"across\", brush?: name|null, item?: stroke item index|null, ids?|id?, target?: \"object\"|\"contents\"} Stroke panel extras (with nothing selected, all but brush set up the next object drawn)",
            has_doc,
            stroke_advanced
        ),
        cmd!(
            "stroke.widthProfile.add",
            "Add to Profiles",
            [],
            None,
            "{name?} save the selected stroke's variable width to the Profile list (kept with the preferences) under `name` (default \"Width Profile N\"; unique, not a built-in's) → {name}",
            can_add_profile,
            profile_add
        ),
        cmd!(
            "stroke.widthProfile.delete",
            "Delete Profile",
            [],
            None,
            "{name?} remove a saved profile from the Profile list (default: the selected stroke's); built-in profiles can't be deleted, and strokes keep their widths → {deleted}",
            has_saved_profiles,
            profile_delete
        ),
        cmd!(
            "stroke.widthProfile.reset",
            "Reset Profiles",
            [],
            None,
            "{} remove every saved profile, leaving the built-ins → {removed: count}",
            has_saved_profiles,
            profile_reset
        ),
        cmd!(
            query "stroke.widthProfile.list",
            "Width Profiles",
            [],
            None,
            "{} → {profiles: [{id (stroke.set `profile`), label, builtIn, points: [[t, left, right]…]}], current: the selected stroke's profile id or name (nothing selected: the next object drawn's), \"custom\" when it isn't listed, null without a stroke}",
            always,
            profile_list
        ),
    ]
}

/// The stroke attributes object and character strokes share, parsed from `stroke.set` params
/// (also the `strokeOptions` of `text.setRangeStyle`).
#[derive(Debug, Default)]
pub(crate) struct StrokeChange {
    pub(crate) weight: Option<f64>,
    cap: Option<LineCap>,
    join: Option<LineJoin>,
    miter_limit: Option<f64>,
    /// `Some(None)`: solid.
    dash: Option<Option<Vec<f64>>>,
    dash_offset: Option<f64>,
    align_dashes: Option<bool>,
    /// Align Stroke: centred, inside or outside the shape (or the glyphs, on type).
    align: Option<StrokeAlign>,
}

/// The value param `key` names in `table` (see [`crate::inspect::CAPS`]); `None` when absent.
fn named<T: Copy>(table: &[(T, &str)], p: &Value, key: &str, cmd: &str) -> Result<Option<T>> {
    let Some(s) = str_param(p, key) else { return Ok(None) };
    let s = if key == "cap" && s == "projecting" { "square" } else { s };
    match table.iter().find(|(_, n)| *n == s) {
        Some((v, _)) => Ok(Some(*v)),
        None => Err(bad(cmd, format!("{key} must be {}, got {s}", table.iter().map(|(_, n)| *n).collect::<Vec<_>>().join("|")))),
    }
}

impl StrokeChange {
    pub(crate) fn parse(p: &Value, cmd: &str) -> Result<Self> {
        let num = |k: &str| p.get(k).and_then(Value::as_f64);
        let dash = match p.get("dash") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::Array(a)) => {
                // Do not silently omit malformed segments and change the user's dash pattern.
                let values: Vec<f64> =
                    a.iter().map(|v| v.as_f64().ok_or_else(|| bad(cmd, format!("dash lengths must be numbers, got {v}")))).collect::<Result<_>>()?;
                Some(Some(values).filter(|d| !d.is_empty()))
            }
            Some(v) => return Err(bad(cmd, format!("dash must be a list of lengths or null, got {v}"))),
        };
        Ok(Self {
            weight: num("weight").map(|w| w.clamp(0.0, 1000.0)),
            cap: named(&CAPS, p, "cap", cmd)?,
            join: named(&JOINS, p, "join", cmd)?,
            miter_limit: num("miterLimit").map(|m| m.clamp(1.0, 500.0)),
            dash,
            dash_offset: num("dashOffset"),
            align_dashes: p.get("alignDashes").and_then(Value::as_bool),
            align: named(&ALIGNS, p, "align", cmd)?,
        })
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.weight.is_none()
            && self.cap.is_none()
            && self.join.is_none()
            && self.miter_limit.is_none()
            && self.dash.is_none()
            && self.dash_offset.is_none()
            && self.align_dashes.is_none()
            && self.align.is_none()
    }

    pub(crate) fn apply(&self, st: &mut StrokeLayer) {
        if let Some(w) = self.weight {
            st.width = w;
        }
        if let Some(a) = self.align {
            st.align = a;
        }
        if let Some(c) = self.cap {
            st.cap = c;
        }
        if let Some(j) = self.join {
            st.join = j;
        }
        if let Some(m) = self.miter_limit {
            st.miter_limit = m;
        }
        if let Some(pattern) = &self.dash {
            // A new pattern keeps the offset and alignment of the one it replaces.
            st.dash = pattern.clone().map(|pattern| Dash { pattern, ..st.dash.take().unwrap_or_default() });
        }
        if let Some(d) = st.dash.as_mut() {
            if let Some(o) = self.dash_offset {
                d.offset = o;
            }
            if let Some(a) = self.align_dashes {
                d.align_corners = a;
            }
        }
    }

    /// [`Self::apply`] to a character stroke.
    pub(crate) fn apply_char(&self, style: &mut CharStyle) {
        let mut st = style.stroke_layer();
        self.apply(&mut st);
        style.set_stroke_layer(&st);
    }

    /// [`Self::apply_char`] to type drawn at mean scale `k` (its `xf`'s): the weight and dash
    /// lengths are document-space, the character stroke is drawn in text space.
    pub(crate) fn apply_char_scaled(&self, style: &mut CharStyle, k: f64) {
        let mut st = style.stroke_layer();
        scale_layer(&mut st, k);
        self.apply(&mut st);
        scale_layer(&mut st, 1.0 / k);
        style.set_stroke_layer(&st);
    }
}

/// Does a Stroke panel edit find nothing selected (and no `ids`)? It then sets up new art.
fn no_targets(ids: &[vectorcraft_doc::NodeId], p: &Value) -> bool {
    ids.is_empty() && p.get("ids").is_none() && p.get("id").is_none()
}

/// The objects `stroke.set` edits with no `item`: a selected group or layer with a stroke row of
/// its own edits that row, as the Stroke panel does with the group targeted (#6739); the other
/// targets reach their painted leaves, as before.
fn own_stroke_or_leaves(s: &Session, p: &Value) -> Result<Vec<vectorcraft_doc::NodeId>> {
    let picked = super::targets(s, p)?;
    let d = &s.doc()?.doc;
    let own = |id: &vectorcraft_doc::NodeId| {
        d.node(*id).is_some_and(|n| {
            matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.is_compound_shape() && n.appearance.stroke().is_some()
        })
    };
    let (mut out, rest): (Vec<_>, Vec<_>) = picked.into_iter().partition(own);
    for id in super::leaf_targets(s, &rest)? {
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

fn stroke_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.set";
    let change = StrokeChange::parse(p, C)?;
    let item = item_target(s, p, C)?.of_kind(s, false);
    let mut ids = if item == ItemTarget::Top { own_stroke_or_leaves(s, p)? } else { item.targets(s, p)? };
    let new_art = no_targets(&ids, p);
    if item == ItemTarget::Top {
        // Images and symbol instances take no stroke from the panel.
        let d = &s.doc()?.doc;
        ids.retain(|id| !matches!(d.node(*id).map(|n| &n.kind), Some(NodeKind::Image(_) | NodeKind::SymbolInstance { .. })));
    }
    let arrow = |k: &str| -> Result<Option<Option<Arrowhead>>> {
        match p.get(k) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(n)) if n == "none" => Ok(Some(None)),
            // A custom head by name, from the arrowheads file (stroke.arrowheads.list).
            Some(Value::Object(o)) if o.len() == 1 && o.contains_key("custom") => {
                let Some(Value::String(name)) = o.get("custom") else { return Err(bad(C, format!("{k}: custom must be a name"))) };
                let head = s.arrowheads.get(name).ok_or_else(|| bad(C, format!("no custom arrowhead {name} (see stroke.arrowheads.list)")))?;
                Ok(Some(Some(Arrowhead::Custom(head))))
            }
            // A custom head with its art: an outline the head would drop is refused, not drawn empty.
            Some(Value::Object(o)) if o.contains_key("Custom") => {
                let raw = o.get("Custom").and_then(|c| c.get("path")).cloned().unwrap_or(Value::Null);
                let path: PathData = serde_json::from_value(raw).map_err(|e| bad(C, format!("{k}: bad custom arrowhead path: {e}")))?;
                // The region the outline fills (non-zero rule), down to slivers a millionth of a
                // head unit thin. Measured only once `fits` has bounded the anchors.
                let fills = |p: &PathData| {
                    let area = vectorcraft_pathops::unite_all_with(&[(p, vectorcraft_geom::FillRule::NonZero)], 1e-6);
                    area.subpaths.iter().map(|sp| sp.area().abs()).sum::<f64>() > 1e-12
                };
                if !CustomArrowhead::fits(&path) || !fills(&path) {
                    let (most, far) = (CustomArrowhead::MAX_ANCHORS, CustomArrowhead::MAX_UNITS);
                    return Err(bad(
                        C,
                        format!(
                            "{k}: a custom arrowhead's path needs an area to fill and at most {most} anchors and {most} subpaths, each anchor within {far} head units of its tip"
                        ),
                    ));
                }
                let head = p.get(k).cloned().unwrap_or(Value::Null);
                serde_json::from_value::<Arrowhead>(head).map(|a| Some(Some(a))).map_err(|e| bad(C, format!("{k}: bad custom arrowhead: {e}")))
            }
            Some(v) => serde_json::from_value::<Arrowhead>(v.clone()).map(|a| Some(Some(a))).map_err(|_| bad(C, format!("unknown arrowhead {v}"))),
        }
    };
    let (sa, ea) = (arrow("startArrow")?, arrow("endArrow")?);
    let arrow_align = match str_param(p, "arrowAlign") {
        None => None,
        Some("extend") => Some(ArrowAlign::Extend),
        Some("tip") => Some(ArrowAlign::Tip),
        Some(o) => return Err(bad(C, format!("arrowAlign must be extend|tip, got {o}"))),
    };
    let profile = match str_param(p, "profile") {
        None => None,
        Some(id) => Some(s.resolve_profile(id).ok_or_else(|| bad(C, format!("unknown profile {id} (see stroke.widthProfile.list)")))?),
    };
    let set = |st: &mut StrokeLayer| {
        change.apply(st);
        if let Some(a) = &sa {
            st.start_arrow = a.clone();
        }
        if let Some(a) = &ea {
            st.end_arrow = a.clone();
        }
        if let Some(a) = arrow_align {
            st.arrow_align = a;
        }
        if let Some(pr) = &profile {
            st.profile = pr.clone();
        }
    };
    edit_items(s, &ids, item, C, "Stroke", false, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            // Character strokes are drawn in text space, scaled by the type's transform.
            let k = Scaling::factor(t.xf).filter(|k| k.is_finite() && *k > 1e-12).unwrap_or(1.0);
            t.runs.iter_mut().for_each(|r| change.apply_char_scaled(&mut r.style, k));
            return Ok(());
        }
        if index.is_none() && n.appearance.stroke().is_none() {
            // Options alone (cap, join, miter limit, dashes, alignment) don't make a stroke
            // visible: the row made for them has no paint. A weight, an arrowhead or a profile
            // still gives the object the black stroke the panel draws (#6739).
            let visible = change.weight.is_some() || sa.is_some() || ea.is_some() || profile.is_some();
            n.appearance.set_stroke(if visible { Paint::solid(Color::BLACK) } else { Paint::None });
        }
        if let Some(st) = n.appearance.stroke_at_mut(index) {
            set(st);
        }
        super::object::snap_if_aligned(n);
        Ok(())
    })?;
    // Commit the new-art template only after the complete request is validated
    // and the selected artwork edit has succeeded.
    if let Some(w) = change.weight {
        s.paint.stroke_width = w;
    }
    if new_art && let Some(st) = s.new_art_stroke_mut() {
        set(st);
    }
    Ok(json!({"objects": ids.iter().collect::<std::collections::BTreeSet<_>>().len()}))
}

/// `w` (pt) one step up or down in `unit` (Units › Stroke): a quarter unit up to one unit,
/// whole units above it, from 0.25 unit to 1000 pt. A weight between steps goes to the next step
/// in that direction (0.6 pt up is 0.75 pt); a weight below a quarter unit doesn't go down.
pub(crate) fn step_weight(w: f64, unit: Unit, up: bool) -> f64 {
    const EPS: f64 = 1e-9;
    let k = Some(unit.to_pt(1.0)).filter(|k| k.is_finite() && *k > 0.0).unwrap_or(1.0);
    let w = if w.is_finite() { w.max(0.0) } else { 0.0 };
    let v = w / k;
    let next = if up {
        if v < 1.0 - EPS { ((v * 4.0 + EPS).floor() + 1.0) / 4.0 } else { (v + EPS).floor() + 1.0 }
    } else if v <= 1.0 + EPS {
        (((v * 4.0 - EPS).ceil() - 1.0) / 4.0).max(0.25)
    } else {
        (v - EPS).ceil() - 1.0
    };
    let r = (next * k).min(1000.0);
    // Never a step the wrong way: past 1000 pt up stays, below a quarter unit down stays.
    if up { r.max(w) } else { r.min(w) }
}

/// `stroke.weightUp` / `stroke.weightDown`: each target stroke stepped from its own weight.
fn stroke_step(s: &mut Session, p: &Value, up: bool) -> Result<Value> {
    let c = if up { "stroke.weightUp" } else { "stroke.weightDown" };
    let unit = s.stroke_unit();
    let item = item_target(s, p, c)?.of_kind(s, false);
    let ids = item.targets(s, p)?;
    if no_targets(&ids, p) {
        let w = step_weight(s.paint.stroke_width, unit, up);
        s.paint.stroke_width = w;
        if let Some(st) = s.new_art_stroke_mut() {
            st.width = w;
        }
        return Ok(json!({"objects": 0}));
    }
    let label = if up { "Stroke Weight Up" } else { "Stroke Weight Down" };
    edit_items(s, &ids, item, c, label, false, |n, index| {
        if index.is_none()
            && let NodeKind::Text(t) = &mut n.kind
        {
            // Character strokes are drawn in text space, scaled by the type's transform.
            let k = Scaling::factor(t.xf).filter(|k| k.is_finite() && *k > 1e-12).unwrap_or(1.0);
            for r in t.runs.iter_mut().filter(|r| !matches!(r.style.stroke, Paint::None)) {
                let change = StrokeChange { weight: Some(step_weight(r.style.stroke_width * k, unit, up)), ..StrokeChange::default() };
                change.apply_char_scaled(&mut r.style, k);
            }
            return Ok(());
        }
        if let Some(st) = n.appearance.stroke_at_mut(index) {
            st.width = step_weight(st.width, unit, up);
        }
        super::object::snap_if_aligned(n);
        Ok(())
    })?;
    Ok(json!({"objects": ids.iter().collect::<std::collections::BTreeSet<_>>().len()}))
}

impl Session {
    /// Units > Stroke (Preferences): the unit stroke weights and dash lengths show in.
    pub fn stroke_unit(&self) -> Unit {
        self.unit(crate::units::Measure::Stroke)
    }

    /// The stroke the Stroke panel, the Control bar and the Properties panel show, picked as the
    /// Stroke proxy picks its paint: the Appearance panel's active item when it is a stroke, else
    /// the first selected object's (a group's first painted object's) top stroke, type's first
    /// run's character stroke; with an object of a dynamic symbol instance selected
    /// ([`DocState::selected_symbol_part`]), that object's as the instance draws it; with nothing
    /// selected, the next object drawn's ([`Session::new_art_stroke`]). None without a document or
    /// the selection's stroke.
    pub fn shown_stroke(&self) -> Option<StrokeLayer> {
        let st = self.active()?;
        if let Some(part) = st.selected_symbol_part() {
            return node_stroke(&part);
        }
        let Some(first) = st.selection.objects.first() else { return Some(self.new_art_stroke()) };
        let first = st.doc.node(*first)?;
        if let Some(i) = first.appearance.item_of_kind(self.appearance_item(), false) {
            return first.appearance.stroke_at(Some(i)).cloned();
        }
        let mut nodes = vec![];
        painted(first, true, &mut nodes);
        node_stroke(nodes.first()?)
    }
}

impl DocState {
    /// Which Stroke panel values the selected objects' strokes (type: every run's) don't share,
    /// and whether Align Stroke applies to them (no open path among them). With an object of a
    /// dynamic symbol instance selected ([`DocState::selected_symbol_part`]), that object alone is
    /// read. One walk of the selection, so callers that ask every frame cache it by revision.
    pub fn stroke_mixed(&self) -> StrokeMixed {
        let part = self.selected_symbol_part();
        let mut nodes = vec![];
        if let Some(part) = &part {
            painted(part, true, &mut nodes);
        } else {
            for id in &self.selection.objects {
                if let Some(n) = self.doc.node(*id) {
                    painted(n, true, &mut nodes);
                }
            }
        }
        let mut strokes = vec![];
        let mut can_align = true;
        for n in &nodes {
            match &n.kind {
                NodeKind::Text(t) => {
                    // The characters' stroke aligns to the glyph outlines, which are closed.
                    strokes.extend(t.runs.iter().map(|r| r.style.stroke_layer()));
                }
                _ => {
                    can_align &= encloses(n);
                    strokes.extend(n.appearance.stroke().cloned());
                }
            }
        }
        let Some((first, rest)) = strokes.split_first() else { return StrokeMixed { can_align, ..Default::default() } };
        let differs = |same: fn(&StrokeLayer, &StrokeLayer) -> bool| rest.iter().any(|s| !same(first, s));
        StrokeMixed {
            weight: differs(|a, b| a.width == b.width),
            cap: differs(|a, b| a.cap == b.cap),
            join: differs(|a, b| a.join == b.join),
            miter_limit: differs(|a, b| a.miter_limit == b.miter_limit),
            align: differs(|a, b| a.align == b.align),
            dash: differs(|a, b| a.dash == b.dash),
            can_align,
        }
    }
}

/// Does `n` enclose an area a stroke can be aligned inside or outside of (no open path)?
fn encloses(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { path, .. } => path.is_closed(),
        NodeKind::Compound { children, .. } => children.iter().all(|c| encloses(c)),
        _ => true,
    }
}

/// Mirror a width profile along the path (t → 1 − t; the two sides of a discontinuous point swap
/// too) or across it (swap left/right widths).
pub(crate) fn flip_profile(p: &WidthProfile, along: bool) -> WidthProfile {
    let mut pts: Vec<(f64, f64, f64)> =
        if along { p.points.iter().rev().map(|&(t, l, r)| (1.0 - t, l, r)).collect() } else { p.points.iter().map(|&(t, l, r)| (t, r, l)).collect() };
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    WidthProfile { points: pts }
}

fn stroke_advanced(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.setAdvanced";
    let scale = match p.get("arrowScale") {
        None => None,
        Some(v) => {
            let a = v.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(C, "arrowScale must be [start, end]"))?;
            let f = |i: usize| a[i].as_f64().map(|x| x.clamp(1.0, 1000.0));
            Some((f(0).ok_or_else(|| bad(C, "bad arrowScale"))?, f(1).ok_or_else(|| bad(C, "bad arrowScale"))?))
        }
    };
    let flip = match str_param(p, "flipProfile") {
        None => None,
        Some("along") => Some(true),
        Some("across") => Some(false),
        Some(o) => return Err(bad(C, format!("flipProfile must be along|across, got {o}"))),
    };
    let swap = bool_or(p, "swapArrows", false);
    let brush = match p.get("brush") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(b)) => Some(Some(b.clone())),
        Some(_) => return Err(bad(C, "brush must be a name or null")),
    };
    if scale.is_none() && flip.is_none() && !swap && brush.is_none() {
        return Err(bad(C, "nothing to change"));
    }
    let item = item_target(s, p, C)?.of_kind(s, false);
    let ids = item.targets(s, p)?;
    let arrows_and_profile = |st: &mut StrokeLayer| {
        if let Some(sc) = scale {
            st.arrow_scale = sc;
        }
        if swap {
            std::mem::swap(&mut st.start_arrow, &mut st.end_arrow);
            st.arrow_scale = (st.arrow_scale.1, st.arrow_scale.0);
        }
        if let (Some(along), Some(pr)) = (flip, &st.profile) {
            st.profile = Some(flip_profile(pr, along));
        }
    };
    // As stroke.set: with nothing selected, for the next object drawn.
    if no_targets(&ids, p)
        && (scale.is_some() || swap || flip.is_some())
        && let Some(st) = s.new_art_stroke_mut()
    {
        arrows_and_profile(st);
    }
    edit_items(s, &ids, item, C, "Stroke", false, |n, index| {
        let Some(st) = n.appearance.stroke_at_mut(index) else { return Ok(()) };
        arrows_and_profile(st);
        if let Some(b) = &brush {
            st.brush = b.clone();
        }
        Ok(())
    })?;
    ok()
}

/// One row of the Profile list: a built-in profile or a saved one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfileEntry<'a> {
    /// What `stroke.set {profile}` takes: a built-in's id, a saved profile's name.
    pub id: &'a str,
    pub label: &'a str,
    pub points: &'a [(f64, f64, f64)],
    pub built_in: bool,
}

impl ProfileEntry<'_> {
    /// Does this row describe `p` (`None`: the plain stroke, the Uniform row)?
    pub fn matches(&self, p: Option<&WidthProfile>) -> bool {
        match p {
            None => self.built_in && self.id == "uniform",
            Some(p) => p.points == self.points,
        }
    }
}

impl Session {
    /// The Profile list: the built-in profiles, then the saved ones.
    pub fn profile_entries(&self) -> impl Iterator<Item = ProfileEntry<'_>> {
        let built_in = WidthProfile::PRESETS.iter().map(|p| ProfileEntry { id: p.id, label: p.label, points: p.points, built_in: true });
        let saved =
            self.prefs.width_profiles.iter().map(|p| ProfileEntry { id: &p.name, label: &p.name, points: &p.profile.points, built_in: false });
        built_in.chain(saved)
    }

    /// The Profile list row showing `p` (`None`: the plain stroke), if it is listed.
    pub fn profile_entry(&self, p: Option<&WidthProfile>) -> Option<ProfileEntry<'_>> {
        self.profile_entries().find(|e| e.matches(p))
    }

    /// The profile `stroke.set {profile: key}` applies: `Some(None)` for "uniform" (the plain
    /// stroke), else a built-in's id or a saved profile's name; `None` when nothing is called `key`.
    pub fn resolve_profile(&self, key: &str) -> Option<Option<WidthProfile>> {
        let e = self.profile_entries().find(|e| e.id == key)?;
        Some((!e.matches(None)).then(|| WidthProfile { points: e.points.to_vec() }))
    }

    /// The name Add to Profiles suggests: the first free "Width Profile N".
    pub fn next_profile_name(&self) -> String {
        (1..).map(|n| format!("Width Profile {n}")).find(|n| !self.profile_name_taken(n)).unwrap_or_default()
    }

    /// Does a listed profile have `name` as its id or label (ignoring case)?
    fn profile_name_taken(&self, name: &str) -> bool {
        self.profile_entries().any(|e| e.id.eq_ignore_ascii_case(name) || e.label.eq_ignore_ascii_case(name))
    }

    /// The variable width Add to Profiles saves: the shown stroke's profile, when it isn't listed.
    fn profile_to_add(&self) -> std::result::Result<WidthProfile, String> {
        let p = self.shown_stroke().and_then(|s| s.profile).ok_or("select a stroke with a variable width")?;
        match self.profile_entry(Some(&p)) {
            Some(e) => Err(format!("the selected stroke's profile is already listed as {}", e.label)),
            None => Ok(p),
        }
    }
}

fn can_add_profile(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    s.profile_to_add().map(|_| ())
}

fn has_saved_profiles(s: &Session) -> std::result::Result<(), String> {
    if s.prefs.width_profiles.is_empty() { Err("no saved profiles".into()) } else { Ok(()) }
}

fn profile_add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthProfile.add";
    let profile = s.profile_to_add().map_err(|e| bad(C, e))?;
    let name = match str_param(p, "name").map(str::trim) {
        None => s.next_profile_name(),
        Some("") => return Err(bad(C, "the name is empty")),
        Some(n) if s.profile_name_taken(n) => return Err(bad(C, format!("a profile is already called {n}"))),
        Some(n) => n.to_string(),
    };
    s.prefs.width_profiles.push(SavedProfile { name: name.clone(), profile });
    Ok(json!({ "name": name }))
}

fn profile_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "stroke.widthProfile.delete";
    let name = match str_param(p, "name") {
        Some(n) => n.to_string(),
        None => {
            let shown = s.shown_stroke().and_then(|st| st.profile);
            let entry = shown.as_ref().and_then(|p| s.profile_entry(Some(p)));
            entry.map(|e| e.id.to_string()).ok_or_else(|| bad(C, "give a name, or select a stroke with a saved profile"))?
        }
    };
    if WidthProfile::PRESETS.iter().any(|b| b.id == name || b.label == name) {
        return Err(bad(C, format!("{name} is built in and can't be deleted")));
    }
    let i = s.prefs.width_profiles.iter().position(|sp| sp.name == name).ok_or_else(|| bad(C, format!("no saved profile called {name}")))?;
    s.prefs.width_profiles.remove(i);
    Ok(json!({ "deleted": name }))
}

fn profile_reset(s: &mut Session, _: &Value) -> Result<Value> {
    let removed = std::mem::take(&mut s.prefs.width_profiles).len();
    Ok(json!({ "removed": removed }))
}

fn profile_list(s: &mut Session, _: &Value) -> Result<Value> {
    let profiles: Vec<Value> = s
        .profile_entries()
        .map(|e| json!({"id": e.id, "label": e.label, "builtIn": e.built_in, "points": e.points.iter().map(|&(t, l, r)| [t, l, r]).collect::<Vec<_>>()}))
        .collect();
    let current = s.shown_stroke().map(|st| s.profile_entry(st.profile.as_ref()).map_or("custom", |e| e.id).to_string());
    Ok(json!({ "profiles": profiles, "current": current }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_steps_follow_units_and_stop_at_their_bounds() {
        assert_eq!(step_weight(0.6, Unit::Points, true), 0.75);
        assert_eq!(step_weight(0.6, Unit::Points, false), 0.5);
        assert_eq!(step_weight(2.5, Unit::Points, true), 3.0);
        assert_eq!(step_weight(2.5, Unit::Points, false), 2.0);
        assert_eq!(step_weight(0.1, Unit::Points, false), 0.1, "below a quarter point a step down keeps it");
        assert_eq!(step_weight(1000.0, Unit::Points, true), 1000.0);
        assert_eq!(step_weight(2000.0, Unit::Points, true), 2000.0, "a step up never lowers a weight");
        assert_eq!(step_weight(f64::NAN, Unit::Points, true), 0.25);
        let mm = Unit::Millimeters.to_pt(1.0);
        assert!((step_weight(mm, Unit::Millimeters, true) - 2.0 * mm).abs() < 1e-9);
        assert!((step_weight(mm, Unit::Millimeters, false) - 0.75 * mm).abs() < 1e-9);
    }
}
