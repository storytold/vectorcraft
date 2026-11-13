//! Layers panel, artboards.
//!
//! The Layers panel works on rows: layers, sublayers, groups and objects. Clicking a row makes it
//! the highlighted row (`layer.setCurrent`; Shift and Ctrl change several, `layer.highlight`) and
//! its layer the current one, where new art goes. Panel commands without `ids` act on the
//! highlighted rows, else on the current layer. More panel-menu operations live in
//! [`super::layerpanel`].

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, ArtboardSide, Document, LAYER_COLORS, LayerColor, Node, NodeId, NodeKind, Scaling};
use vectorcraft_geom::{Affine, Rect};

use super::{
    CommandSpec, Result, Session, bad, bool_or, check_numbers, check_switches, checked_id, checked_ids_param, checked_node, checked_nodes_param, cmd,
    color_value, f64_or, has_doc, has_selection, ids_param, index, object_param, ok, str_param, switch_or,
};
use crate::EngineError;

/// Most rows one panel command takes (ids are untrusted input).
pub(crate) const MAX_ROWS: usize = 100_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.new",
            "New Layer…",
            ["Window", "Layers"],
            Some("Cmd+L"),
            "{name?, top?: bool, below?: bool, color?, template?, locked?, visible?, printable?, preview?, dimImages?} a layer directly above the current layer, at its level (a sublayer beside a current sublayer); `top`: above all layers (Ctrl/Cmd-click the button); `below`: directly below the current layer, at its level (Ctrl/Cmd+Alt-click the button), not with `top`. Options as layer.setProps; refused while an opacity mask is edited (the Layers panel lists only the mask) → {id}",
            super::maskedit::outside_mask_editing,
            new_layer
        ),
        cmd!(
            "layer.newSublayer",
            "New Sublayer…",
            ["Window", "Layers"],
            None,
            "{parent?: layer id (default: the highlighted row's layer, else the current layer), bottom?: bool, name?, …options as layer.new} a sublayer on top of the parent's contents; `bottom`: under the parent's contents, above a clipping layer's clipping path (Ctrl/Cmd+Alt-click the button); refused while an opacity mask is edited → {id}",
            super::maskedit::outside_mask_editing,
            new_sublayer
        ),
        cmd!(
            "layer.delete",
            "Delete Selection",
            ["Window", "Layers"],
            None,
            "{ids?|id?} delete rows: layers, sublayers, groups or objects with what they hold (default: the highlighted rows, else the current layer). The last layer stays → {deleted}",
            has_doc,
            delete_rows
        ),
        cmd!(
            "layer.duplicate",
            "Duplicate Selection",
            ["Window", "Layers"],
            None,
            "{ids?|id?} copy rows (default: the highlighted rows, else the current layer), each just above its original; a layer's copy is named `<name> copy`. The copies are highlighted → {ids}",
            has_doc,
            duplicate_rows
        ),
        cmd!(
            "layer.setCurrent",
            "Set Current Layer",
            [],
            None,
            "{id} click a row: it alone is highlighted, and the layer it is (or the layer it is in) becomes the current layer, where new art goes. Not an undo step → {id, layer}",
            has_doc,
            set_current
        ),
        cmd!(
            "layer.highlight",
            "Highlight Rows",
            [],
            None,
            "{ids, mode?: \"set\"|\"add\"|\"toggle\"} highlight Layers panel rows (Shift-click: a range, Ctrl-click: toggle one); the last one's layer becomes current. Not an undo step → {rows}",
            has_doc,
            highlight
        ),
        cmd!(
            "layer.setProps",
            "Layer Options…",
            ["Window", "Layers"],
            None,
            "{ids?|id? (default: the highlighted rows, else the current layer), name? (with prefs.identifyObjectsBy xmlId: a valid XML ID, stored as the object name), visible?, locked?, template?, printable?, preview?, dimImages?: 0..100|false, color?: index 0..26|\"#rrggbb\"|preset name (a hex color with an alpha part, #rrggbbaa or #rgba, is an error)} Layer Options; rows that are not layers take name, visible and locked, and `ids` may name ruler guides (their `guide.list` id), which hide or lock on their own. Template on also locks the layer and dims its images to 50% (unless given); off unlocks it and stops dimming. One undo step",
            has_doc,
            set_props
        ),
        cmd!(
            "layer.collectInNew",
            "Collect in New Layer",
            ["Window", "Layers"],
            None,
            "{ids?} move rows (default: the highlighted rows, else the selected objects) into a new layer where the topmost of them was (a sublayer when they are in a layer), deselecting every selected object left hidden or locked (the key object too) → {id}",
            has_doc,
            collect
        ),
        cmd!(
            "layer.selectAll",
            "Select Art",
            [],
            None,
            "{id, add?: bool} the Layers panel's selection column: select the visible, unlocked art of a layer (its sublayers' too), or the object itself; `add` (Shift-click) adds it to the selection, or removes it when it is all selected already → {selected: [..]}",
            has_doc,
            select_art
        ),
        cmd!(
            "layer.move",
            "Move Rows",
            [],
            None,
            "{ids|id, target, place?: \"above\"|\"below\"|\"inside\" (default inside), copy?: bool} drag rows in the Layers panel: above or below the target row (beside it, in its layer or group), or inside a layer or group (on top of its contents). Layers go only in layers or at the top level, objects in layers and groups; an object placed beside a top-level layer goes inside it. A row never goes into itself; a locked layer or group takes nothing. `copy` (Alt-drag) moves copies. Every selected object left hidden or locked, such as one moved into a hidden layer or group, is deselected (the key object too). One undo step → {ids}",
            has_doc,
            move_rows
        ),
        cmd!(
            "node.move",
            "Reorder",
            [],
            None,
            "{id, parent?: id (null = top level), index} put a row at `index` (0 = bottom) of `parent` (layers only at the top level or in layers), deselecting every selected object left hidden or locked (the key object too)",
            has_doc,
            move_node
        ),
        cmd!(
            "artboard.new",
            "New Artboard",
            ["Window", "Artboards"],
            None,
            "{x?, y?, width?, height? (each more than 0 and at most 3,999,856 pt, as file.new takes them), name?, color?: color (its background color, as artboard.setProps takes it, #rrggbbaa and #rgba refused; default: none), guideLabel?: name | \"#rrggbb\" | null (its guide label, as artboard.setProps takes it; default: none), showCenterMark?: bool, showCrossHairs?: bool, showVideoSafeAreas?: bool (as artboard.setProps takes them; default false), videoRulerPixelAspectRatio?: number 0.1–10 (as artboard.setProps takes it; default 1), side?: \"left\"|\"right\"|\"top\"|\"bottom\" (the size of artboard `beside`, 20 pt from that side of it; refused with x, y, width or height), beside?: n (the artboard `side` places it by; default the last)} (default place: right of the last)",
            has_doc,
            artboard_new
        ),
        cmd!(
            "artboard.delete",
            "Delete Artboard",
            ["Window", "Artboards"],
            None,
            "{index | indices: [index…]} delete artboards (with their guides) in one step; one is always left, and a locked one is an error",
            has_doc,
            artboard_delete
        ),
        cmd!(
            "artboard.deleteEmpty",
            "Delete Empty Artboards",
            ["Window", "Artboards"],
            None,
            "{} delete every artboard with no artwork on it (no object, hidden, locked and template art included, whose visual bounds meet or touch it, clip groups and clipping layers by their members inside the clip; guides don't count), with its guides, in one step; locked artboards stay; when every artboard is empty and none is locked, artboard 1 stays (with a locked one left, artboard 1 can go) → {deleted: [index…]} (the indices before deleting)",
            has_empty_artboards,
            artboard_delete_empty
        ),
        cmd!(
            "artboard.setProps",
            "Artboard Options…",
            ["Window", "Artboards"],
            None,
            "{index | artboards: [{index, name?, color?, guideLabel?, locked?, showCenterMark?, showCrossHairs?, showVideoSafeAreas?, videoRulerPixelAspectRatio?, x?, y?, width?, height?}…] (several artboards in one undo step, each with its own fields; the art inside each before any of them changes goes with it, and an object inside several goes with the first listed that moves or scales its art), name?, color?: color (\"#rrggbb\", {c,m,y,k}, … as the paint commands take, #rrggbbaa and #rgba refused: the background color the canvas draws the artboard's paper in, multiplied with the Simulate Colored Paper color while that is on; exports and print leave it out) | null (no background color), guideLabel?: name | \"#rrggbb\" | null (the guide label: the canvas draws the artboard's own guides (guide.add with artboard) in this color instead of the guide color preference, selected ones still in the selection color; a layer color's name spelled exactly as Layer Options lists it (\"Light Blue\", \"Red\", …) or #rrggbb; null for none, which new artboards and older files have; screen-only, so exports and print leave it out; it can change on a locked artboard; an invalid value anywhere in the list changes nothing), locked?: bool (the Artboards panel's lock column: a locked artboard keeps its rectangle (a new x, y, width or height is an error unless locked is false), can't be moved, deleted or cut, and Rearrange and Fit to Artwork Bounds leave it as it is; the art on it (each object of a layer or sublayer whose bounds with strokes overlap it, a group as one object) acts as locked art until it is unlocked: clicks, marquees, select.set, select.all and the tools pass over it (art drawn or pasted onto it stays selected and editable until a selection leaves it out or it is moved off the locked artboards), it is deselected, and when another artboard moves or scales it stays unless lockedAndHidden), showCenterMark?: bool (Artboard Options' Show Center Mark: the canvas draws a mark at the artboard's center), showCrossHairs?: bool (Show Cross Hairs: the canvas draws a line from the middle of each side through the center), showVideoSafeAreas?: bool (Show Video Safe Areas: the canvas draws the action-safe area 3.5% and the title-safe area 5% in from each edge, as EBU R 95 sets them; the three are screen-only, as the artboard's edges are: exports and print leave them out), videoRulerPixelAspectRatio?: number 0.1–10 (Video Ruler Pixel Aspect Ratio: the width of one video pixel over its height, which View › Rulers › Show Video Rulers counts the artboard in; default 1, square pixels), x?, y?, width?, height?, moveArt?: bool (default false; a position-only change moves the contained art with its artboard), scaleArt?: bool (Scale Artwork with Artboard: resized, the artboard takes the art fully inside it (locked and hidden art only with lockedAndHidden?: bool, default prefs moveLockedWithArtboard) and its guides from its old rectangle onto the new one, each side by its own ratio; strokes/corners/patterns as object.scale), strokes?: bool, corners?: bool, patterns?: bool (the three as object.scale, for art that moves or scales with the artboard), lockedAndHidden?: bool (locked and hidden art moves along; default: the preference; recorded in the journal so a replay matches)} → null, or with scaleArt {scaled: the art scaled}; no undo step when nothing changes",
            has_doc,
            artboard_set
        ),
        cmd!(
            "artboard.fitToArt",
            "Fit to Artwork Bounds",
            ["Object", "Artboards"],
            None,
            "{index?: n | indices?: [n…] (0-based; the app passes the selected artboards: the Artboard tool's, else the active one; default: the first)} each artboard fits the art on it that shows (with its strokes and effects; hidden objects and layers, guides and template layers don't count), in one undo step (none when every artboard already fits): an object counts for every artboard its bounds overlap, and a group or clipping layer counts as one object; an artboard with no art, or a locked one, stays as it is, and the command fails when every artboard it names is locked or none has art",
            has_doc,
            artboard_fit_art
        ),
        cmd!(
            "artboard.fitToSelection",
            "Fit to Selected Art",
            ["Object", "Artboards"],
            None,
            "{index?: n (0-based; the app passes the active artboard; default: the first)} the artboard fits the selected art (with its strokes); a locked artboard stays as it is (an error when the fit would change it)",
            has_selection,
            artboard_fit_sel
        ),
        cmd!(
            "layer.clippingMask.toggle",
            "Make/Release Clipping Mask",
            ["Window", "Layers"],
            None,
            "{id?: layer or group (default: the one highlighted layer or group row, else the one selected group, else the current layer)} make: its top object (a path, compound path or text, which loses its paint) clips the rest and moves to the bottom (new art added on top is clipped); release: it stops clipping, the clipping path stays unpainted → {clip}",
            has_doc,
            clip_toggle
        ),
        cmd!(
            "layer.target",
            "Target",
            [],
            None,
            "{id, add?: bool (Shift-click: the item joins the selection, or leaves it when all selected; nothing is targeted)} target a layer, group or object as clicking its target circle in the Layers panel does: a layer gets its visible, unlocked art (its sublayers' too) selected and becomes the current layer, and appearance.*, effect.*, transparency.* and the opacity-mask commands without `ids` then act on the layer itself; anything else is selected. Any other selection change ends the targeting (`document.inspect` → target) → {id, selected: [..]}",
            has_doc,
            target
        ),
        cmd!(
            "layer.pasteRemembersLayers",
            "Paste Remembers Layers",
            ["Window", "Layers"],
            None,
            "{on?} (default: toggle) the document option: on, the Paste commands put objects back into the layers or sublayers they were copied from (in that document while they are still there, else by the names of the layer and the layers around it, the topmost match first; a missing sublayer is made inside its layer, a missing top-level layer on top; a hidden or locked one is an error and nothing is pasted), off into the current layer; one undo step when it changes (document.inspect → pasteRemembersLayers) → {on}",
            has_doc,
            paste_remembers_layers
        ),
    ]
}

/// The rows a panel command acts on: `ids` or `id`, else the highlighted rows, else the current
/// layer (capped at [`MAX_ROWS`]).
pub(crate) fn rows_param(s: &Session, p: &Value, command: &str) -> Result<Vec<NodeId>> {
    rows_or_guides_param(s, p, command, false)
}

/// The rows [`rows_param`] reads; given ruler guide ids are taken too when `guides`, else refused.
fn rows_or_guides_param(s: &Session, p: &Value, command: &str, guides: bool) -> Result<Vec<NodeId>> {
    // Presence is explicit intent: validate every target before fallback, truncation or edits.
    let mut ids = if p.get("ids").is_some() {
        checked_nodes_param(s, p, "ids", command, guides)?
    } else if let Some(id) = p.get("id") {
        vec![checked_node(s, id, command, guides)?]
    } else {
        let st = s.doc()?;
        let rows = st.highlighted_rows();
        if rows.is_empty() { st.current_layer().into_iter().collect() } else { rows }
    };
    ids.truncate(MAX_ROWS);
    Ok(ids)
}

/// `ids` that exist, without repeats or rows inside another listed row, in paint order (bottom
/// first).
pub(crate) fn row_roots(d: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let set: HashSet<NodeId> = ids.iter().copied().filter(|i| d.node(*i).is_some()).collect();
    let keep: Vec<NodeId> = set
        .iter()
        .copied()
        .filter(|id| d.ancestry(*id).is_some_and(|a| a.split_last().is_none_or(|(_, above)| !above.iter().any(|x| set.contains(x)))))
        .collect();
    d.paint_order(keep)
}

/// Whether `parent` (None: the top level) can hold `n`: layers go at the top level or in layers,
/// anything else in layers and groups.
pub(crate) fn accepts(d: &Document, parent: Option<NodeId>, n: &Node) -> bool {
    match parent {
        None => n.is_layer(),
        Some(p) => match d.node(p).map(|x| &x.kind) {
            Some(NodeKind::Layer { .. }) => true,
            // A ruler guide stays directly in a layer or sublayer.
            Some(NodeKind::Group { .. }) => !n.is_layer() && !n.is_ruler_guide(),
            _ => false,
        },
    }
}

/// Whether `id` or a container around it is locked.
pub(crate) fn locked_within(d: &Document, id: NodeId) -> bool {
    d.ancestry(id).unwrap_or_default().iter().any(|a| d.node(*a).is_some_and(|n| n.locked))
}

/// The number and the names of a document's layers and sublayers, from which [`NewLayers::make`]
/// names and colors a new layer. [`NewLayers::of`] reads them in one walk of the document, and
/// [`NewLayers::add`] counts each new layer as it goes in.
pub(crate) struct NewLayers {
    count: usize,
    taken: HashSet<String>,
}

impl NewLayers {
    pub(crate) fn of(d: &Document) -> Self {
        let mut layers = Self { count: 0, taken: HashSet::new() };
        d.walk(|n| layers.note(n));
        layers
    }

    /// A new layer named `name` (default: the next `Layer N`, as [`Document::next_layer_name`]
    /// gives it) in the next layer color.
    pub(crate) fn make(&self, d: &mut Document, name: Option<&str>) -> Node {
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| (self.count + 1..).map(|i| format!("Layer {i}")).find(|name| !self.taken.contains(name)).unwrap_or_default());
        let id = d.alloc_id();
        Node::layer(id, &name, LayerColor::Preset((self.count % LAYER_COLORS.len()) as u8))
    }

    /// Count `n`, a layer going into the document, and the layers inside it.
    pub(crate) fn add(&mut self, n: &Node) {
        n.walk(&mut |c| self.note(c));
    }

    fn note(&mut self, n: &Node) {
        if n.is_layer() {
            self.count += 1;
            self.taken.extend(n.name.clone());
        }
    }
}

/// A new layer named `name` (default: the next `Layer N`) in the next layer colour.
pub(crate) fn make_layer(d: &mut Document, name: Option<&str>) -> Node {
    NewLayers::of(d).make(d, name)
}

/// A layer colour: a preset index (0–26), `#rrggbb` or a preset's name (any case). A hex color
/// with an alpha part (`#rrggbbaa` or `#rgba`) is an error.
pub(super) fn layer_color_param(v: &Value) -> std::result::Result<LayerColor, String> {
    match v {
        Value::Number(n) => match n.as_u64() {
            Some(i) if index(i) < LAYER_COLORS.len() => Ok(LayerColor::Preset(i as u8)),
            _ => Err(format!("a colour index is 0 to {}, not {n}", LAYER_COLORS.len() - 1)),
        },
        Value::String(s) => {
            if let Some(i) = LAYER_COLORS.iter().position(|(name, _)| name.eq_ignore_ascii_case(s.trim())) {
                return Ok(LayerColor::Preset(i as u8));
            }
            let refused = || format!("`{s}` is not a colour (#rrggbb or a preset name)");
            // `Color::from_hex` reads `#rrggbbaa` as `#rrggbb`, dropping the alpha.
            let digits = s.trim();
            if digits.strip_prefix('#').unwrap_or(digits).len() == 8 {
                return Err(refused());
            }
            let c = vectorcraft_color::Color::from_hex(s).ok_or_else(refused)?;
            let [r, g, b, _] = c.to_rgba8(1.0);
            Ok(LayerColor::Custom([r, g, b]))
        }
        _ => Err(format!("a colour is an index, #rrggbb or a preset name, not {v}")),
    }
}

/// Layer Options in `p` applied to layer or row `n` (validated first by [`check_options`]).
/// `xml_id`: Preferences › Units › Identify Objects By › XML ID — `name` is an XML id and is
/// stored as the object name ([`vectorcraft_doc::xml_id`]).
fn apply_options(n: &mut Node, p: &Value, xml_id: bool) {
    if let Some(v) = str_param(p, "name") {
        if v.trim().is_empty() {
            // A layer keeps a name; an object goes back to its generated `<Kind>` one.
            if !n.is_layer() {
                n.name = None;
            }
        } else {
            let name = if xml_id { vectorcraft_doc::xml_id::to_object_name(v) } else { v.to_string() };
            n.name = Some(name.chars().take(512).collect());
        }
    }
    if let Some(v) = p.get("visible").and_then(Value::as_bool) {
        n.visible = v;
    }
    let locked = p.get("locked").and_then(Value::as_bool);
    if let Some(v) = locked {
        n.locked = v;
    }
    let mut lock = None;
    if let NodeKind::Layer { template, printable, color, preview, dim_images, .. } = &mut n.kind {
        let dim_given = p.get("dimImages").is_some();
        if let Some(v) = p.get("template").and_then(Value::as_bool)
            && v != *template
        {
            *template = v;
            // A template is locked and its images dimmed; it stops being either with it.
            lock = Some(v);
            if !dim_given {
                *dim_images = v.then_some(50);
            }
        }
        if let Some(v) = p.get("printable").and_then(Value::as_bool) {
            *printable = v;
        }
        if let Some(v) = p.get("preview").and_then(Value::as_bool) {
            *preview = v;
        }
        match p.get("dimImages") {
            Some(Value::Bool(false) | Value::Null) => *dim_images = None,
            Some(Value::Bool(true)) => *dim_images = Some(50),
            Some(v) => {
                if let Some(x) = v.as_f64().filter(|x| x.is_finite()) {
                    *dim_images = Some(x.clamp(0.0, 100.0).round() as u8);
                }
            }
            None => {}
        }
        if let Some(c) = p.get("color").and_then(|v| layer_color_param(v).ok()) {
            *color = c;
        }
    }
    if let (Some(v), None) = (lock, locked) {
        n.locked = v;
    }
}

/// Bad values in Layer Options params, as an error for `cmd`.
/// `xml_id`: Preferences › Units › Identify Objects By › XML ID — a non-empty `name` must be a
/// valid XML id.
fn check_options(cmd: &str, p: &Value, xml_id: bool) -> Result<()> {
    if let Some(v) = p.get("color") {
        layer_color_param(v).map_err(|e| bad(cmd, e))?;
    }
    for k in ["visible", "locked", "template", "printable", "preview"] {
        if p.get(k).is_some_and(|v| !v.is_boolean()) {
            return Err(bad(cmd, format!("`{k}` is true or false")));
        }
    }
    if let Some(v) = p.get("dimImages")
        && !(v.is_boolean() || v.is_null() || v.as_f64().is_some_and(f64::is_finite))
    {
        return Err(bad(cmd, "`dimImages` is a percentage or false"));
    }
    if xml_id
        && let Some(name) = str_param(p, "name").filter(|n| !n.trim().is_empty())
        && !vectorcraft_doc::xml_id::is_valid(name)
    {
        return Err(bad(cmd, "Identify Objects By is XML ID: the name must be a valid XML ID (start with a letter, underscore or colon; no spaces)"));
    }
    Ok(())
}

/// Layers panel → Paste Remembers Layers.
fn paste_remembers_layers(s: &mut Session, p: &Value) -> Result<Value> {
    let cur = s.doc()?.doc.paste_remembers_layers;
    let on = switch_or(p, "on", !cur, "layer.pasteRemembersLayers")?;
    if on != cur {
        s.edit("Paste Remembers Layers", |d, _| {
            d.paste_remembers_layers = on;
            Ok(())
        })?;
    }
    Ok(json!({ "on": on }))
}

/// The Layers panel's target circle.
fn target(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.target";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let st = s.doc()?;
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    if !st.doc.is_editable(id) {
        return Err(bad(C, "the object is hidden or locked"));
    }
    if !in_isolation(&st.doc, st.isolation, id) {
        return Err(bad(C, "the object is outside the isolated group"));
    }
    let layer = n.is_layer();
    let rows = st.layer_rows.clone();
    if switch_or(p, "add", false, C)? {
        s.select(|d, sel| sel.toggle_target(d, id))?;
    } else {
        s.select(|d, sel| sel.set_target(d, id))?;
    }
    let st = s.doc_mut()?;
    // Targeting doesn't change which rows are highlighted.
    st.layer_rows = rows;
    if layer {
        st.active_layer = Some(id);
    }
    Ok(json!({ "id": id.0, "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// The Layers panel's clipping mask button: the top object of a layer (or group) clips the rest,
/// or the clipping mask is released.
fn clip_toggle(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let container = |id: &NodeId| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }));
    let row = match st.highlighted_rows().as_slice() {
        [id] if container(id) => Some(*id),
        _ => None,
    };
    let group = match st.selection.objects.as_slice() {
        [id] if matches!(st.doc.node(*id).map(|n| &n.kind), Some(NodeKind::Group { .. })) => Some(*id),
        _ => None,
    };
    // An `id` given is used as given: one that isn't an object id fails (#785).
    let given = match p.get("id").filter(|v| !v.is_null()) {
        Some(v) => Some(checked_id(s, v, "layer.clippingMask.toggle")?),
        None => None,
    };
    let id = given.or(row).or(group).or(st.current_layer()).ok_or_else(|| bad("layer.clippingMask.toggle", "no layer"))?;
    let clip = s.edit("Make/Release Clipping Mask", |d, _| {
        let n = d.node(id).ok_or(EngineError::NoNode(id))?;
        if !matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. }) {
            return Err(bad("layer.clippingMask.toggle", "not a layer or group"));
        }
        if n.clips() {
            super::object::release_clip(d, id);
            return Ok(false);
        }
        // The topmost object: a layer's ruler guides above its art do not clip.
        let top = n
            .children()
            .and_then(|c| c.iter().rev().find(|c| !c.is_ruler_guide()))
            .map(|c| c.id)
            .ok_or_else(|| EngineError::Other("the layer has no objects to clip by".into()))?;
        super::object::make_clipping_path(d, top)?;
        d.move_node(top, Some(id), 0)?;
        if let Some(n) = d.node_mut(id) {
            n.set_clips(true);
        }
        Ok(true)
    })?;
    Ok(json!({ "clip": clip }))
}

/// A new layer with the options in `p`, at `parent`/`index`; it becomes current and highlighted.
fn add_layer(s: &mut Session, p: &Value, cmd: &str, label: &str, place: impl FnOnce(&Document) -> Result<(Option<NodeId>, usize)>) -> Result<Value> {
    check_switches(p, &["top"], cmd)?;
    let xml_id = s.prefs.identify_objects_by == "xmlId";
    check_options(cmd, p, xml_id)?;
    let name = str_param(p, "name").filter(|n| !n.trim().is_empty()).map(str::to_string);
    let id = s.edit(label, |d, _| {
        let (parent, mut index) = place(d)?;
        // The clipping path stays at the bottom of a clipping layer.
        if d.children(parent).is_some_and(|c| !c.is_empty()) && parent.and_then(|p| d.node(p)).is_some_and(Node::clips) {
            index = index.max(1);
        }
        let mut n = make_layer(d, name.as_deref());
        apply_options(&mut n, p, xml_id);
        Ok(d.insert(parent, index, n)?)
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(id);
    st.layer_rows = vec![id];
    Ok(json!({ "id": id.0 }))
}

fn new_layer(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.new";
    let below = switch_or(p, "below", false, C)?;
    if below && bool_or(p, "top", false) {
        return Err(bad(C, "`top` and `below` can't both be true"));
    }
    let st = s.doc()?;
    // Above the current layer, at its level; below it with `below`; on top of everything with `top`.
    let cur = st.current_layer().filter(|_| !bool_or(p, "top", false));
    add_layer(s, p, C, "New Layer", |d| match cur.and_then(|c| d.position(c)) {
        Some((parent, i, _)) => Ok((parent, if below { i } else { i + 1 })),
        None => Ok((None, if below { 0 } else { usize::MAX })),
    })
}

fn new_sublayer(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.newSublayer";
    let st = s.doc()?;
    let row = st.highlighted_rows().last().and_then(|r| st.doc.layer_containing(*r));
    // A `parent` given is used as given: one that isn't an object id fails (#785).
    let given = match p.get("parent").filter(|v| !v.is_null()) {
        Some(v) => Some(checked_id(s, v, C)?),
        None => None,
    };
    let parent = given.or(row).or(st.current_layer()).ok_or_else(|| bad(C, "no parent layer"))?;
    if !st.doc.node(parent).is_some_and(Node::is_layer) {
        return Err(bad(C, "the parent must be a layer or sublayer"));
    }
    let index = if switch_or(p, "bottom", false, C)? { 0 } else { usize::MAX };
    add_layer(s, p, C, "New Sublayer", |_| Ok((Some(parent), index)))
}

fn delete_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = rows_param(s, p, "layer.delete")?;
    let st = s.doc()?;
    let ids = row_roots(&st.doc, &ids);
    if ids.is_empty() {
        return Err(bad("layer.delete", "no such rows"));
    }
    let listed: HashSet<NodeId> = ids.iter().copied().collect();
    if st.doc.layers.iter().all(|l| listed.contains(&l.id)) {
        return Err(EngineError::Other("a document needs at least one layer".into()));
    }
    s.edit("Delete Selection", |d, sel| {
        // The guides on the deleted layers go with them.
        d.remove_many(&ids)?;
        sel.prune(d);
        Ok(())
    })?;
    let st = s.doc_mut()?;
    st.layer_rows.clear();
    if st.current_layer().is_none() {
        st.active_layer = st.doc.default_layer();
    }
    Ok(json!({ "deleted": ids.len() }))
}

fn duplicate_rows(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = rows_param(s, p, "layer.duplicate")?;
    let ids = row_roots(&s.doc()?.doc, &ids);
    if ids.is_empty() {
        return Err(bad("layer.duplicate", "no such rows"));
    }
    let copies = s.edit("Duplicate Selection", |d, _| {
        // A copied layer's ruler guides are copied with it.
        super::edit::copy_above(d, &ids, |_, n, c| {
            if n.is_layer() {
                c.name = Some(format!("{} copy", n.display_name()));
            }
        })
    })?;
    s.doc_mut()?.layer_rows = copies.clone();
    Ok(json!({ "ids": copies.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn set_current(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.setCurrent";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let st = s.doc_mut()?;
    let layer = st.doc.layer_containing(id).ok_or(EngineError::NoNode(id))?;
    st.active_layer = Some(layer);
    st.layer_rows = vec![id];
    st.revision += 1;
    Ok(json!({ "id": id.0, "layer": layer.0 }))
}

fn highlight(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.highlight";
    let mut ids = ids_param(p, "ids", C)?.ok_or_else(|| bad(C, "missing ids"))?;
    ids.truncate(MAX_ROWS);
    let mode = str_param(p, "mode").unwrap_or("set");
    let st = s.doc_mut()?;
    ids.retain(|id| st.doc.node(*id).is_some());
    let mut rows = st.highlighted_rows();
    match mode {
        "set" => rows.clear(),
        "add" | "toggle" => {}
        other => return Err(bad(C, format!("mode is set, add or toggle, not `{other}`"))),
    }
    let mut last = None;
    for id in ids {
        if mode == "toggle" && rows.contains(&id) {
            rows.retain(|r| *r != id);
        } else if !rows.contains(&id) {
            rows.push(id);
            last = Some(id);
        }
    }
    if let Some(layer) = last.or(rows.last().copied()).and_then(|id| st.doc.layer_containing(id)) {
        st.active_layer = Some(layer);
    }
    st.layer_rows = rows;
    st.revision += 1;
    Ok(json!({ "rows": st.layer_rows.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.setProps";
    let xml_id = s.prefs.identify_objects_by == "xmlId";
    check_options(C, p, xml_id)?;
    // A ruler guide's row takes visible and locked: its own eye and lock.
    let ids = rows_or_guides_param(s, p, C, true)?;
    if ids.is_empty() {
        return Err(bad(C, "no rows"));
    }
    s.edit("Layer Options", |d, sel| {
        for id in &ids {
            apply_options(d.node_mut(*id).ok_or(EngineError::NoNode(*id))?, p, xml_id);
        }
        sel.deselect_uneditable(d);
        Ok(())
    })?;
    ok()
}

fn collect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.collectInNew";
    let ids = if p.get("ids").is_some() {
        checked_ids_param(s, p, "ids", C)?
    } else {
        let rows = s.doc()?.highlighted_rows();
        if rows.is_empty() { super::edit::selected_roots(s)? } else { rows }
    };
    let ids = row_roots(&s.doc()?.doc, &ids);
    let Some(top) = ids.last().copied() else { return Err(bad(C, "select rows or objects to collect")) };
    let lid = s.edit("Collect in New Layer", |d, sel| {
        // Where the topmost of them is: beside it in its layer, or on top of the layer holding
        // its group.
        let beside = d.position(top).is_some_and(|(par, _, _)| par.is_none_or(|p| d.node(p).is_some_and(Node::is_layer)));
        let holder = d.layer_containing(top);
        let mut layer = make_layer(d, None);
        let lid = layer.id;
        if beside {
            d.wrap(&ids, |rows| {
                if let Some(ch) = layer.children_mut() {
                    *ch = rows;
                }
                layer
            })?;
        } else {
            let rows = d.remove_many(&ids)?;
            if let Some(ch) = layer.children_mut() {
                *ch = rows;
            }
            d.insert(holder, usize::MAX, layer)?;
        }
        sel.prune(d);
        sel.deselect_uneditable(d);
        Ok(lid)
    })?;
    let st = s.doc_mut()?;
    st.active_layer = Some(lid);
    st.layer_rows = vec![lid];
    Ok(json!({ "id": lid.0 }))
}

/// Whether `id` lies inside the isolated group or layer (always, outside isolation mode).
fn in_isolation(doc: &Document, isolation: Option<NodeId>, id: NodeId) -> bool {
    isolation.is_none_or(|iso| iso != id && doc.ancestry(id).is_some_and(|a| a.contains(&iso)))
}

/// The selection column: select a row's art.
fn select_art(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.selectAll";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let add = switch_or(p, "add", false, C)?;
    let st = s.doc()?;
    let n = st.doc.node(id).ok_or(EngineError::NoNode(id))?;
    // An isolated layer's own row selects the art inside it.
    let isolated_layer = st.isolation == Some(id) && n.is_layer();
    let ids: Vec<NodeId> = if !st.doc.is_editable(id) || !(isolated_layer || in_isolation(&st.doc, st.isolation, id)) {
        vec![]
    } else if n.is_layer() {
        n.layer_art(true)
    } else if n.is_ruler_guide() {
        // A ruler guide row selects the guide through `guide.select`, not as art.
        vec![]
    } else {
        vec![id]
    };
    let all_selected = !ids.is_empty() && ids.iter().all(|i| st.selection.contains(*i));
    let rows = st.layer_rows.clone();
    s.select(|_, sel| {
        if !add {
            sel.set(ids.iter().copied());
        } else if all_selected {
            for i in &ids {
                sel.remove(*i);
            }
        } else {
            for i in &ids {
                sel.add(*i);
            }
        }
    })?;
    let st = s.doc_mut()?;
    // The selection column leaves the highlighted rows as they are.
    st.layer_rows = rows;
    Ok(json!({ "selected": st.selection.objects.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Place {
    Above,
    Below,
    Inside,
}

/// Drag rows in the Layers panel.
fn move_rows(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "layer.move";
    let mut ids = match ids_param(p, "ids", C)? {
        Some(ids) => ids,
        None => vec![object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing ids"))?],
    };
    ids.truncate(MAX_ROWS);
    let target = object_param(p, "target", C)?.ok_or_else(|| bad(C, "missing target"))?;
    let place = match str_param(p, "place").unwrap_or("inside") {
        "above" => Place::Above,
        "below" => Place::Below,
        "inside" | "into" => Place::Inside,
        other => return Err(bad(C, format!("place is above, below or inside, not `{other}`"))),
    };
    let copy = switch_or(p, "copy", false, C)?;
    let st = s.doc()?;
    let d = &st.doc;
    d.node(target).ok_or(EngineError::NoNode(target))?;
    let ids = row_roots(d, &ids);
    if ids.is_empty() {
        return Err(bad(C, "no such rows"));
    }
    // Never into itself or a row inside it.
    let around = d.ancestry(target).unwrap_or_default();
    if ids.iter().any(|i| around.contains(i)) {
        return Err(bad(C, "a row can't go into itself"));
    }
    let nodes: Vec<&Node> = ids.iter().filter_map(|i| d.node(*i)).collect();
    let (parent, anchor) = match place {
        Place::Inside => (Some(target), None),
        _ => match d.parent_of(target) {
            // Objects can't be at the top level: beside a top-level layer means in it.
            None if nodes.iter().any(|n| !n.is_layer()) => (Some(target), None),
            parent => (parent, Some(target)),
        },
    };
    if let Some(n) = nodes.iter().find(|n| !accepts(d, parent, n)) {
        let into = parent.and_then(|p| d.node(p)).map_or("the top level", |p| if p.is_layer() { "a layer" } else { p.kind_label() });
        return Err(bad(C, format!("{} can't go into {into}", n.display_name())));
    }
    if parent.is_some_and(|p| locked_within(d, p)) {
        return Err(EngineError::Other("the layer or group is locked".into()));
    }
    let label = if copy { "Duplicate Rows" } else { "Move Rows" };
    let moved = s.edit(label, |d, sel| {
        let nodes = if copy {
            let mut nodes = vec![];
            for id in &ids {
                let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
                // A copied layer's ruler guides are copied with it.
                nodes.push(Arc::new(d.reid(&n)));
            }
            nodes
        } else {
            d.remove_many(&ids)?
        };
        let count = d.children(parent).map_or(0, Vec::len);
        let mut index = match anchor {
            None => count,
            Some(a) => {
                let (_, i, _) = d.position(a).ok_or(EngineError::NoNode(a))?;
                if place == Place::Above { i + 1 } else { i }
            }
        };
        // The clipping path stays at the bottom of a clip group or a clipping layer.
        if count > 0 && parent.and_then(|p| d.node(p)).is_some_and(Node::clips) {
            index = index.max(1);
        }
        let out: Vec<NodeId> = nodes.iter().map(|n| n.id).collect();
        let ch = d.children_mut(parent)?;
        let index = index.min(ch.len());
        ch.splice(index..index, nodes);
        // A layer's ruler guides stay above its art, each in the order it now has.
        ch.sort_by_key(|c| c.is_ruler_guide());
        // Copied type whose layout caches `reid` dropped has them again.
        if copy {
            super::typecmd::refresh_bounds_of(d, &out);
        }
        sel.prune(d);
        sel.deselect_uneditable(d);
        Ok(out)
    })?;
    let st = s.doc_mut()?;
    st.layer_rows = moved.clone();
    Ok(json!({ "ids": moved.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn move_node(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "node.move";
    let id = object_param(p, "id", C)?.ok_or_else(|| bad(C, "missing id"))?;
    let parent = object_param(p, "parent", C)?;
    let index = p.get("index").and_then(Value::as_u64).map_or(usize::MAX, index);
    let d = &s.doc()?.doc;
    let n = d.node(id).ok_or(EngineError::NoNode(id))?;
    if !accepts(d, parent, n) {
        return Err(bad(C, "layers go only at the top level or in layers, objects in layers and groups"));
    }
    s.edit("Reorder", |d, sel| {
        d.move_node(id, parent, index)?;
        sel.deselect_uneditable(d);
        Ok(())
    })?;
    ok()
}

/// The rectangle from `x`, `y`, `width` and `height`, each defaulting to `default`'s. Sides are kept as given.
fn rect_unclamped(p: &Value, default: Rect) -> Rect {
    let x = f64_or(p, "x", default.x0);
    let y = f64_or(p, "y", default.y0);
    let w = f64_or(p, "width", default.width());
    let h = f64_or(p, "height", default.height());
    Rect::new(x, y, x + w, y + h)
}

/// [`rect_unclamped`] with each supplied side raised to at least 1 pt, as `artboard.setProps` uses it.
/// A side left out keeps `default`'s, even below 1 pt.
fn rect_from(p: &Value, default: Rect) -> Rect {
    let r = rect_unclamped(p, default);
    let floor = |key: &str, side: f64| if p.get(key).and_then(Value::as_f64).is_some() { side.max(1.0) } else { side };
    Rect::new(r.x0, r.y0, r.x0 + floor("width", r.width()), r.y0 + floor("height", r.height()))
}

/// An artboard command's `color` param, its background color: None when absent, `Some(None)` for
/// null (no color), else `Some(Some(color))`.
fn artboard_color(p: &Value, cmd: &str) -> Result<Option<Option<vectorcraft_color::Color>>> {
    match p.get("color") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => color_value(v).map(|c| Some(Some(c))).ok_or_else(|| bad(cmd, format!("bad color {v}"))),
    }
}

/// An artboard command's `guideLabel` param, the color of its own guides: None when absent,
/// `Some(None)` for null (no label), else `Some(Some(label))` for a preset's name exactly as
/// [`LAYER_COLORS`] spells it or `#rrggbb` ([`LayerColor::from_label_text`]). Any other value,
/// such as a preset's index, a name in other letter case or `#rrggbbaa`, is an error.
fn artboard_guide_label(p: &Value, cmd: &str) -> Result<Option<Option<LayerColor>>> {
    match p.get("guideLabel") {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(v) => v
            .as_str()
            .and_then(LayerColor::from_label_text)
            .map(|c| Some(Some(c)))
            .ok_or_else(|| bad(cmd, "`guideLabel` is null, a layer color's name or #rrggbb")),
    }
}

/// An artboard command's text param `key`: None when absent or null, else the string. Any other
/// value, such as 5, is an error: "`{key}` must be a string".
fn artboard_str<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<Option<&'a str>> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => Ok(Some(v)),
        Some(_) => Err(bad(cmd, format!("`{key}` must be a string"))),
    }
}

/// An artboard command's `side` param: None when absent or null, else the side it names. Any other
/// value is an error that names the four sides.
pub(crate) fn artboard_side(p: &Value, cmd: &str) -> Result<Option<ArtboardSide>> {
    let bad_side = || bad(cmd, "`side` must be \"left\", \"right\", \"top\" or \"bottom\"");
    match p.get("side") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(v)) => ArtboardSide::parse(v).map(Some).ok_or_else(bad_side),
        Some(_) => Err(bad_side()),
    }
}

/// An artboard command's `videoRulerPixelAspectRatio` param: None when absent, else a number within
/// [`vectorcraft_doc::VIDEO_PIXEL_ASPECT_RANGE`].
fn video_pixel_aspect(p: &Value, cmd: &str) -> Result<Option<f64>> {
    match p.get("videoRulerPixelAspectRatio") {
        None => Ok(None),
        Some(v) => v
            .as_f64()
            .filter(|r| vectorcraft_doc::VIDEO_PIXEL_ASPECT_RANGE.contains(r))
            .map(Some)
            .ok_or_else(|| bad(cmd, "videoRulerPixelAspectRatio must be a number from 0.1 to 10")),
    }
}

fn artboard_new(s: &mut Session, p: &Value) -> Result<Value> {
    let d0 = &s.doc()?.doc;
    let last = d0.artboards.last().map(|a| a.rect).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
    let def = Rect::new(last.x1 + 20.0, last.y0, last.x1 + 20.0 + last.width(), last.y1);
    let color = artboard_color(p, "artboard.new")?.flatten();
    let guide_label = artboard_guide_label(p, "artboard.new")?.flatten();
    check_switches(p, &["showCenterMark", "showCrossHairs", "showVideoSafeAreas"], "artboard.new")?;
    check_numbers(p, &["x", "y", "width", "height"], "artboard.new")?;
    if ["width", "height"].iter().filter_map(|k| p.get(*k).and_then(Value::as_f64)).any(|side| !super::newdoc::side_in_range(side)) {
        return Err(super::newdoc::side_error("artboard.new", d0.units));
    }
    let r = match artboard_side(p, "artboard.new")? {
        Some(side) => {
            if ["x", "y", "width", "height"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) {
                return Err(bad("artboard.new", "`side` places the artboard: leave out x, y, width and height"));
            }
            let beside = match p.get("beside") {
                None | Some(Value::Null) => d0.artboards.len().saturating_sub(1),
                Some(_) => super::appearance::index_param(p, "beside", "artboard.new")?,
            };
            let src = d0.artboards.get(beside).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            src + side.offset(src)
        }
        None if p.get("beside").is_some_and(|v| !v.is_null()) => return Err(bad("artboard.new", "`beside` needs `side`")),
        None => rect_unclamped(p, def),
    };
    // A size taken from another artboard (`side`, or width or height left out) keeps to the limit too.
    if !super::newdoc::side_in_range(r.width()) || !super::newdoc::side_in_range(r.height()) {
        return Err(super::newdoc::side_error("artboard.new", d0.units));
    }
    let given = artboard_str(p, "name", "artboard.new")?.map(str::trim).filter(|n| !n.is_empty());
    let (show_center_mark, show_cross_hairs) = (bool_or(p, "showCenterMark", false), bool_or(p, "showCrossHairs", false));
    let show_video_safe_areas = bool_or(p, "showVideoSafeAreas", false);
    let video_ruler_pixel_aspect_ratio = video_pixel_aspect(p, "artboard.new")?.unwrap_or(1.0);
    let idx = s.edit("New Artboard", |d, _| {
        let id = d.next_artboard_id();
        let name = given.map_or_else(|| format!("Artboard {}", d.artboards.len() + 1), str::to_string);
        d.artboards.push(Artboard {
            id,
            name,
            rect: r,
            locked: false,
            show_center_mark,
            show_cross_hairs,
            show_video_safe_areas,
            video_ruler_pixel_aspect_ratio,
            color,
            guide_label,
        });
        Ok(d.artboards.len() - 1)
    })?;
    Ok(json!({ "index": idx }))
}

fn artboard_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let indices = super::panelcmds::artboard_indices(&s.doc()?.doc, p, "artboard.delete")?;
    super::panelcmds::refuse_locked(&s.doc()?.doc, &indices)?;
    s.edit("Delete Artboard", |d, sel| remove_artboards(d, sel, &indices))?;
    ok()
}

/// Delete artboards `indices` (sorted, each once) of `d` with their guides; at least one must be
/// left. The other artboards keep their order, no later artboard takes a deleted one's id, and
/// `sel` forgets the deleted guides. One walk over the artboards and one over the ruler guides,
/// however many go. Delete Artboard, Delete Empty Artboards, Cut Artboard and Artwork Cleanup's
/// Empty Artboards.
pub(crate) fn remove_artboards(d: &mut Document, sel: &mut vectorcraft_doc::Selection, indices: &[usize]) -> Result<()> {
    if indices.iter().any(|i| *i >= d.artboards.len()) {
        return Err(EngineError::Other("no such artboard".into()));
    }
    if indices.len() >= d.artboards.len() {
        return Err(EngineError::Other("a document needs at least one artboard".into()));
    }
    if indices.is_empty() {
        return Ok(());
    }
    let mut gone: HashSet<u32> = HashSet::with_capacity(indices.len());
    let mut i = 0;
    d.artboards.retain(|a| {
        let keep = indices.binary_search(&i).is_err();
        i += 1;
        if !keep {
            gone.insert(a.id);
        }
        keep
    });
    count_artboard_pass();
    for &id in &gone {
        d.retire_artboard_id(id);
    }
    d.retain_guides(sel, |g| !g.artboard.is_some_and(|a| gone.contains(&a)));
    count_artboard_pass();
    Ok(())
}

#[cfg(test)]
thread_local! {
    /// Tests: how many walks [`remove_artboards`] made over the artboards and the ruler guides on
    /// this thread.
    pub(crate) static ARTBOARD_PASSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Tests: count one walk over the artboards or the ruler guides.
fn count_artboard_pass() {
    #[cfg(test)]
    ARTBOARD_PASSES.set(ARTBOARD_PASSES.get() + 1);
}

/// The artboards Delete Empty Artboards deletes: those with no artwork
/// ([`Document::artboard_has_art`]) that aren't locked, but artboard 1 when every artboard is one
/// of them (a document keeps one, as Delete Artboards keeps artboard 1).
pub(crate) fn empty_artboards(d: &Document) -> Vec<usize> {
    let mut empty: Vec<usize> = d.artboards.iter().enumerate().filter(|(_, a)| !a.locked && !d.artboard_has_art(a.rect)).map(|(i, _)| i).collect();
    if empty.len() == d.artboards.len() {
        empty.retain(|&i| i != 0);
    }
    empty
}

/// Is [`empty_artboards`] of `d` not empty? A document with one artboard keeps it, so there is
/// nothing to delete; with two or more, any unlocked artboard without art settles it (when every
/// artboard is empty, all but artboard 1 go). [`Document::any_artboard_without_art`] finds an
/// artboard away from every path without measuring along the paths.
fn any_empty_artboards(d: &Document) -> bool {
    d.artboards.len() > 1 && d.any_artboard_without_art(d.artboards.iter().filter(|a| !a.locked).map(|a| a.rect))
}

fn has_empty_artboards(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.any_empty_artboards(any_empty_artboards) { Ok(()) } else { Err("no empty artboards to delete".into()) }
}

fn artboard_delete_empty(s: &mut Session, _: &Value) -> Result<Value> {
    let indices = empty_artboards(&s.doc()?.doc);
    if indices.is_empty() {
        return Err(EngineError::Other("no empty artboards to delete".into()));
    }
    s.edit("Delete Empty Artboards", |d, sel| remove_artboards(d, sel, &indices))?;
    Ok(json!({ "deleted": indices }))
}

/// One artboard's part of `artboard.setProps`: its index, new rectangle, name, color, lock,
/// display marks (Show Center Mark, Show Cross Hairs, Show Video Safe Areas) and Video Ruler Pixel
/// Aspect Ratio, how far it moved, whether it was resized, the map its art and guides take when
/// they go with it (`fit` when it scales them), and that art.
struct ArtboardEdit<'a> {
    index: usize,
    now: Rect,
    name: Option<&'a str>,
    color: Option<Option<vectorcraft_color::Color>>,
    guide_label: Option<Option<LayerColor>>,
    lock: Option<bool>,
    center_mark: Option<bool>,
    cross_hairs: Option<bool>,
    safe_areas: Option<bool>,
    pixel_aspect: Option<f64>,
    delta: vectorcraft_geom::Vec2,
    resized: bool,
    fit: Option<Affine>,
    transform: Option<Affine>,
    art: Vec<NodeId>,
}

impl ArtboardEdit<'_> {
    /// Does the edit change artboard `a` (the one at its index) or the art and guides on it?
    fn changes(&self, a: &Artboard) -> bool {
        let name = self.name.map(|n| if n.is_empty() { format!("Artboard {}", self.index + 1) } else { n.to_string() });
        self.now != a.rect
            || self.transform.is_some()
            || name.is_some_and(|n| n != a.name)
            || self.color.is_some_and(|c| c != a.color)
            || self.guide_label.is_some_and(|c| c != a.guide_label)
            || self.lock.is_some_and(|l| l != a.locked)
            || self.center_mark.is_some_and(|m| m != a.show_center_mark)
            || self.cross_hairs.is_some_and(|c| c != a.show_cross_hairs)
            || self.safe_areas.is_some_and(|v| v != a.show_video_safe_areas)
            || self.pixel_aspect.is_some_and(|r| r != a.video_ruler_pixel_aspect_ratio)
    }
}

fn artboard_set(s: &mut Session, p: &Value) -> Result<Value> {
    // One artboard (`index` and its fields), or several (`artboards`, each with its own fields).
    let entries: Vec<&Value> = match p.get("artboards") {
        Some(list) => {
            list.as_array().filter(|l| !l.is_empty()).ok_or_else(|| bad("artboard.setProps", "artboards must be a non-empty list"))?.iter().collect()
        }
        None => vec![p],
    };
    check_switches(p, &["moveArt", "scaleArt", "strokes", "corners", "patterns", "lockedAndHidden"], "artboard.setProps")?;
    for q in &entries {
        check_switches(q, &["locked", "showCenterMark", "showCrossHairs", "showVideoSafeAreas"], "artboard.setProps")?;
        check_numbers(q, &["x", "y", "width", "height"], "artboard.setProps")?;
    }
    // Scale Artwork with Artboard: resized, an artboard takes the art fully inside it (as Move
    // Artwork with Artboard gathers it) and its guides from the old rectangle onto the new one.
    let scale_art = bool_or(p, "scaleArt", false);
    // Locked and hidden art too?
    let all = bool_or(p, "lockedAndHidden", s.prefs.move_locked_with_artboard);
    let d = &s.doc()?.doc;
    // Each artboard's art is the art inside it before any of them changes; an object inside
    // several of them goes with the first one listed that moves or scales its art.
    let mut taken = HashSet::new();
    let mut edits: Vec<ArtboardEdit> = Vec::with_capacity(entries.len());
    for q in entries {
        let index = super::panelcmds::artboard_index(d, q, "artboard.setProps")?;
        if edits.iter().any(|e| e.index == index) {
            return Err(bad("artboard.setProps", "an artboard is listed twice"));
        }
        let was = d.artboards.get(index).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        // The background color: a color sets it, null clears it, absent leaves it as it is.
        let color = artboard_color(q, "artboard.setProps")?;
        // The guide label likewise; a locked artboard's can change, as its name can.
        let guide_label = artboard_guide_label(q, "artboard.setProps")?;
        let now = rect_from(q, was);
        let lock = q.get("locked").and_then(Value::as_bool);
        let (center_mark, cross_hairs) = (q.get("showCenterMark").and_then(Value::as_bool), q.get("showCrossHairs").and_then(Value::as_bool));
        let (safe_areas, pixel_aspect) = (q.get("showVideoSafeAreas").and_then(Value::as_bool), video_pixel_aspect(q, "artboard.setProps")?);
        // A locked artboard keeps its rectangle unless this unlocks it; its name can change.
        let reshaped = [now.x0 - was.x0, now.y0 - was.y0, now.x1 - was.x1, now.y1 - was.y1].iter().any(|d| d.abs() > 1e-6);
        if reshaped && lock != Some(false) {
            super::panelcmds::refuse_locked(d, &[index])?;
        }
        // Values within 1e-6 pt of a locked artboard's (as Artboard Options passes them) leave its
        // rectangle exactly as it is.
        let held = lock != Some(false) && d.artboards.get(index).is_some_and(|a| a.locked);
        let now = if held { was } else { now };
        let resized = (now.width() - was.width()).abs() > 1e-6 || (now.height() - was.height()).abs() > 1e-6;
        let fit = if scale_art && resized { rect_map(was, now) } else { None };
        // Position fields honor Move Artwork independently of Scale Artwork. A resize still
        // leaves the art alone unless scaling was requested. Fit-to-art callers keep the default.
        let delta = now.origin() - was.origin();
        let moving = !resized && bool_or(p, "moveArt", false) && delta != vectorcraft_geom::Vec2::ZERO;
        let transform = fit.or_else(|| moving.then(|| Affine::translate(delta)));
        let art = if transform.is_some() { d.art_moving_with(index, all).into_iter().filter(|id| taken.insert(*id)).collect() } else { vec![] };
        edits.push(ArtboardEdit {
            index,
            now,
            name: artboard_str(q, "name", "artboard.setProps")?.map(str::trim),
            color,
            guide_label,
            lock,
            center_mark,
            cross_hairs,
            safe_areas,
            pixel_aspect,
            delta,
            resized,
            fit,
            transform,
            art,
        });
    }
    // Nothing to change: no undo step.
    if !edits.iter().any(|e| d.artboards.get(e.index).is_none_or(|a| e.changes(a))) {
        return if scale_art { Ok(json!({ "scaled": [] })) } else { ok() };
    }
    // Noted in the journal when art goes along, so a replay transforms the same objects whatever
    // the preference is then.
    if edits.iter().any(|e| e.transform.is_some()) {
        s.note_journal("lockedAndHidden", json!(all));
    }
    let scaling =
        if edits.iter().any(|e| e.fit.is_some_and(|xf| Scaling::factor(xf).is_some())) { super::object::scaling(s, p) } else { Scaling::default() };
    let art: Vec<NodeId> = edits.iter().flat_map(|e| e.art.iter().copied()).collect();
    let patterns = super::object::transform_patterns(s, p, &art)?;
    s.edit("Artboard Options", |d, _| {
        for e in &edits {
            let a = d.artboards.get_mut(e.index).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            a.rect = e.now;
            if let Some(n) = e.name {
                // A blank name gives the artboard its default name, Artboard N for the Nth artboard.
                a.name = if n.is_empty() { format!("Artboard {}", e.index + 1) } else { n.to_string() };
            }
            if let Some(c) = e.color {
                a.color = c;
            }
            if let Some(c) = e.guide_label {
                a.guide_label = c;
            }
            if let Some(l) = e.lock {
                a.locked = l;
            }
            if let Some(m) = e.center_mark {
                a.show_center_mark = m;
            }
            if let Some(c) = e.cross_hairs {
                a.show_cross_hairs = c;
            }
            if let Some(v) = e.safe_areas {
                a.show_video_safe_areas = v;
            }
            if let Some(r) = e.pixel_aspect {
                a.video_ruler_pixel_aspect_ratio = r;
            }
            let id = a.id;
            if let Some(xf) = e.transform {
                let mut sc = if e.fit.is_some_and(|xf| Scaling::factor(xf).is_some()) { scaling } else { Scaling::default() };
                sc.patterns = patterns;
                d.map_artboard_guides(id, xf);
                for id in &e.art {
                    if let Some(n) = d.node_mut(*id) {
                        n.transform(xf, sc);
                        super::object::keep_pixel_aligned(n, sc);
                    }
                }
            } else if !e.resized {
                // Moved (not resized), it takes its guides along.
                d.move_artboard_guides(id, e.delta);
            }
        }
        Ok(())
    })?;
    if !scale_art {
        return ok();
    }
    let scaled: Vec<_> = edits.iter().filter(|e| e.fit.is_some()).flat_map(|e| e.art.iter().map(|i| i.0)).collect();
    Ok(json!({ "scaled": scaled }))
}

/// The map taking artboard rectangle `from` onto `to`, each side scaled by its own ratio. None
/// when `from` has no area or the map isn't finite (a size out of range).
fn rect_map(from: Rect, to: Rect) -> Option<Affine> {
    if from.width() <= 1e-9 || from.height() <= 1e-9 {
        return None;
    }
    let (sx, sy) = (to.width() / from.width(), to.height() / from.height());
    let xf = Affine::translate(to.origin().to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-from.origin().to_vec2());
    xf.is_finite().then_some(xf)
}

/// Each artboard of `index` or `indices` (default: the first) fits the art on it that exports
/// draw ([`vectorcraft_render::encode::art_bounds_on`]): hidden objects and layers, guides and
/// template layers don't count. An artboard with no art on it, or a locked one, stays as it is,
/// and the command fails when every one it names is locked or none has art. The fits are one undo
/// step named Artboard Options.
fn artboard_fit_art(s: &mut Session, p: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let indices =
        if p.get("index").is_none() && p.get("indices").is_none() { vec![0] } else { super::panelcmds::artboard_indices(d, p, "artboard.fitToArt")? };
    let unlocked: Vec<usize> = indices.iter().copied().filter(|i| d.artboards.get(*i).is_some_and(|a| !a.locked)).collect();
    if unlocked.is_empty() {
        super::panelcmds::refuse_locked(d, &indices)?;
    }
    let mut fits = vec![];
    // One renderer measures every artboard: each 3D solid is built once.
    let mut renderer = vectorcraft_render::Renderer::new();
    for i in unlocked {
        let was = d.artboards.get(i).map(|a| a.rect).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
        if let Some(b) = renderer.art_bounds_on(d, was) {
            // At least 1 pt a side, as `rect_from` makes Artboard Options' sizes.
            fits.push((i, was, Rect::new(b.x0, b.y0, b.x0 + b.width().max(1.0), b.y0 + b.height().max(1.0))));
        }
    }
    if fits.is_empty() {
        return Err(EngineError::Other("no artwork".into()));
    }
    // An artboard that already fits its art changes nothing, and no undo step is recorded for it.
    fits.retain(|(_, was, now)| [now.x0 - was.x0, now.y0 - was.y0, now.x1 - was.x1, now.y1 - was.y1].iter().any(|d| d.abs() > 1e-6));
    if fits.is_empty() {
        return ok();
    }
    s.edit("Artboard Options", |d, _| {
        for &(i, was, now) in &fits {
            let a = d.artboards.get_mut(i).ok_or_else(|| EngineError::Other("no such artboard".into()))?;
            a.rect = now;
            let id = a.id;
            // Moved and not resized, it takes its guides along, as in Artboard Options.
            if (now.width() - was.width()).abs() <= 1e-6 && (now.height() - was.height()).abs() <= 1e-6 {
                d.move_artboard_guides(id, now.origin() - was.origin());
            }
        }
        Ok(())
    })?;
    ok()
}

fn artboard_fit_sel(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.doc()?;
    let i = if p.get("index").is_none() { 0 } else { super::panelcmds::artboard_index(&st.doc, p, "artboard.fitToSelection")? };
    let b = st.doc.bounds_of(&st.selection.objects, true).ok_or_else(|| EngineError::Other("no selection bounds".into()))?;
    s.execute("artboard.setProps", &json!({"index": i, "x": b.x0, "y": b.y0, "width": b.width(), "height": b.height()}))
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::{Appearance, AppearanceItem, LineCap, LineJoin, StrokeAlign, StrokeLayer, SymbolBoxes};

    use super::*;

    /// Does art lie on `rect`, measuring every object's visual bounds: the rule of
    /// [`Document::artboard_has_art`] with no quick box in front of it.
    fn measured_has_art(d: &Document, rect: Rect) -> bool {
        let rect = rect.inflate(1e-6, 1e-6);
        fn meets(b: Option<Rect>, r: Rect) -> bool {
            b.is_some_and(|b| b.x0 <= r.x1 && r.x0 <= b.x1 && b.y0 <= r.y1 && r.y0 <= b.y1)
        }
        fn has(c: &Node, s: &SymbolBoxes, rect: Rect) -> bool {
            match &c.kind {
                NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                    let Some((clip, rest)) = children.split_first() else { return false };
                    let paint = clip.clip_paint();
                    if (paint.fill.is_some() || paint.stroke.is_some()) && meets(clip.visual_bounds_with(s), rect) {
                        return true;
                    }
                    let Some(b) = c.visual_bounds_with(s).filter(|b| meets(Some(*b), rect)) else { return false };
                    let inside = Rect::new(b.x0.max(rect.x0), b.y0.max(rect.y0), b.x1.min(rect.x1), b.y1.min(rect.y1));
                    rest.iter().any(|m| has(m, s, inside))
                }
                NodeKind::Layer { children, .. } | NodeKind::Group { children, .. } => children.iter().any(|m| has(m, s, rect)),
                NodeKind::Path { guide: true, .. } => false,
                _ => meets(c.visual_bounds_with(s), rect),
            }
        }
        let symbols = d.symbol_boxes();
        d.layers.iter().any(|l| has(l, &symbols, rect))
    }

    /// Would Delete Empty Artboards delete something, by [`measured_has_art`] of each artboard:
    /// the unlocked artboards without art, less artboard 1 when that is every artboard.
    fn measured_any_empty(d: &Document) -> bool {
        let mut empty: Vec<usize> =
            d.artboards.iter().enumerate().filter(|(_, a)| !a.locked && !measured_has_art(d, a.rect)).map(|(i, _)| i).collect();
        if empty.len() == d.artboards.len() {
            empty.retain(|&i| i != 0);
        }
        !empty.is_empty()
    }

    /// On documents made from a fixed seed (one to five artboards placed anywhere, overlapping
    /// some of the time, some locked; rectangles, long curved paths and symbol instances, some
    /// with wide strokes reaching past their shape, some grouped or clipped), the menu's check
    /// [`any_empty_artboards`] gives the same answer as `!empty_artboards(d).is_empty()` and as
    /// measuring every object against every artboard, and both answers come up.
    #[test]
    fn the_empty_artboard_check_matches_measuring_every_artboard() {
        let mut seed: u64 = 0x0a2d_5eed_e4b7;
        let mut next = |n: u64| {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) % n
        };
        let (mut enabled, mut disabled_with_art) = (0, 0);
        for _ in 0..120 {
            let mut s = Session::new();
            s.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
            for _ in 0..next(5) {
                let (x, y) = (next(1400) as f64 - 100.0, next(800) as f64 - 100.0);
                s.execute("artboard.new", &json!({"x": x, "y": y, "width": 20 + next(300), "height": 20 + next(200)})).unwrap();
            }
            let artboards = s.doc().unwrap().doc.artboards.len();
            for i in 0..artboards {
                if next(5) == 0 {
                    s.execute("artboard.setProps", &json!({"index": i, "locked": true})).unwrap();
                }
            }
            let mut symbol = false;
            let mut ids = vec![];
            for _ in 0..next(6) {
                let id = match next(4) {
                    0 => {
                        let (segments, span) = (20 + next(80), 10 + next(700));
                        let (x, y) = (next(1400) as i64 - 100, next(800) as i64 - 100);
                        let mut pt = || format!("{} {}", x + next(span) as i64, y + next(span) as i64);
                        let mut d = format!("M {}", pt());
                        for _ in 0..segments {
                            d += &format!(" C {} {} {}", pt(), pt(), pt());
                        }
                        s.execute("path.create", &json!({"d": d})).unwrap()["id"].as_u64().unwrap()
                    }
                    1 => {
                        let (x, y) = (next(1400) as f64 - 100.0, next(800) as f64 - 100.0);
                        if !symbol {
                            let r = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5 + next(60), "height": 5 + next(60)})).unwrap();
                            s.execute("symbol.new", &json!({"name": "Mark", "ids": [r["id"]]})).unwrap();
                            s.execute("object.delete", &json!({"ids": [r["id"]]})).ok();
                            symbol = true;
                        }
                        s.execute("symbol.place", &json!({"name": "Mark", "x": x, "y": y})).unwrap()["id"].as_u64().unwrap()
                    }
                    _ => {
                        let (x, y) = (next(1400) as f64 - 100.0, next(800) as f64 - 100.0);
                        let r = json!({"x": x, "y": y, "width": 1 + next(150), "height": 1 + next(150)});
                        s.execute("shape.rectangle", &r).unwrap()["id"].as_u64().unwrap()
                    }
                };
                if next(2) == 0 {
                    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 1.0 + next(120) as f64);
                    st.join = [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel][next(3) as usize];
                    st.cap = [LineCap::Butt, LineCap::Round, LineCap::Square][next(3) as usize];
                    st.align = [StrokeAlign::Center, StrokeAlign::Inside, StrokeAlign::Outside][next(3) as usize];
                    st.miter_limit = 1.0 + next(20) as f64;
                    let look = Appearance { items: vec![AppearanceItem::Stroke(st)], ..Appearance::default() };
                    s.edit("Strokes", |d, _| {
                        d.node_mut(NodeId(id)).unwrap().appearance = look.into();
                        Ok(())
                    })
                    .unwrap();
                }
                ids.push(id);
            }
            if ids.len() > 1 {
                match next(3) {
                    0 => {
                        s.execute("select.set", &json!({"ids": ids})).unwrap();
                        s.execute("object.group", &json!({})).unwrap();
                    }
                    1 => {
                        let (x, y) = (next(1400) as f64 - 100.0, next(800) as f64 - 100.0);
                        let clip = json!({"x": x, "y": y, "width": 1 + next(400), "height": 1 + next(300)});
                        let clip = s.execute("shape.rectangle", &clip).unwrap()["id"].as_u64().unwrap();
                        let members: Vec<u64> = ids.iter().copied().chain([clip]).collect();
                        s.execute("select.set", &json!({"ids": members})).unwrap();
                        s.execute("object.clippingMask.make", &json!({})).unwrap();
                    }
                    _ => {}
                }
            }
            let d = &s.doc().unwrap().doc;
            let measured = measured_any_empty(d);
            let at = || d.artboards.iter().map(|a| (a.rect, a.locked)).collect::<Vec<_>>();
            assert_eq!(any_empty_artboards(d), measured, "{:?}", at());
            assert_eq!(!empty_artboards(d).is_empty(), measured, "{:?}", at());
            enabled += usize::from(measured);
            let several = d.artboards.len() > 1 && !ids.is_empty();
            disabled_with_art += usize::from(several && !measured);
        }
        assert!(enabled > 15 && enabled < 105, "both answers come up: {enabled} of 120 enabled");
        assert!(disabled_with_art > 3, "documents with several artboards and art come up disabled: {disabled_with_art}");
    }
}
