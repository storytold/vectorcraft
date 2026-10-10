//! The Appearance panel: fill and stroke items, the active item that paint, stroke, gradient,
//! transparency and effect edits target, live effects, Clear / Reduce to Basic, and the
//! Eyedropper's appearance copy.
//!
//! **Item targeting.** Commands that edit a fill or stroke take `item?`: a paint-order index into
//! the appearance stack (`items[0]` is painted first, the bottom row of the panel). Omitted, they
//! edit the Appearance panel's active item (`appearance.setActiveItem`) when one is set for the
//! current selection and no `ids` are given, else the topmost fill or stroke; `null` always means
//! the topmost one. A targeted item lives in the selected objects' own stacks (a group's own fill,
//! not its contents'), so item edits apply to the selected objects themselves.
//!
//! **Object or contents.** The `appearance.*` and `effect.*` commands edit the stacks of the
//! selected objects themselves (`target: "object"`, the default; with `ids`, layers too) or, with
//! `target: "contents"`, of the painted objects inside the selected groups and layers. A group's or
//! layer's own fills and strokes paint its members' geometry and its effects apply to them as one
//! piece; the Contents row (type: Characters) is the slot that says which of its fills and strokes
//! paint below the members and which above ([`Appearance::contents_at`], moved by
//! `appearance.moveItem {from: "contents"}`).

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, FillLayer, Node, NodeKind, ParaStyle, StrokeLayer};
use vectorcraft_geom::Rect;

use super::edit::selected_roots;
use super::opacitymask::percent;
use super::paint::paint_from;
use super::*;
use super::{AppearanceAttrs, EyedropperAttrs, FillAttrs, StrokeAttrs};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "appearance.addFill",
            "Add New Fill",
            ["Window", "Appearance"],
            None,
            "{ids?, target?: \"object\"|\"contents\"} add a fill on top of each selected object's own stack (a copy of its top fill; type without one: its characters' fill; else the default fill)",
            has_doc,
            |s, p| add_item(s, p, true)
        ),
        cmd!(
            "appearance.addStroke",
            "Add New Stroke",
            ["Window", "Appearance"],
            None,
            "{ids?, target?} add a stroke on top of each selected object's own stack (a copy of its top stroke, else the default stroke)",
            has_doc,
            |s, p| add_item(s, p, false)
        ),
        cmd!(
            "appearance.clear",
            "Clear Appearance",
            ["Window", "Appearance"],
            None,
            "{ids?, target?} leave each object one empty fill and stroke (None; a group none of its own) and reset its opacity and blend mode",
            has_doc,
            clear_appearance
        ),
        cmd!(
            "appearance.reduceToBasic",
            "Reduce to Basic Appearance",
            ["Window", "Appearance"],
            None,
            "{ids?, target?} keep only the topmost visible fill and stroke, without their own opacity, blend mode or effects (a group keeps no rows it lacked), and drop the object's effects",
            has_doc,
            reduce_basic
        ),
        cmd!(
            "appearance.setItem",
            "Appearance Item",
            [],
            None,
            "{index: paint-order item index, ids? (default: the selection), target?: \"object\"|\"contents\", opacity?: 0..100, blend?: name, visible?: bool, weight?: pt (strokes), color?|none?|swatch?|gradient? (as paint.setFill)} edit one fill/stroke of each target object's own appearance stack",
            has_doc,
            set_item
        ),
        cmd!(
            "appearance.removeItem",
            "Remove Item",
            [],
            None,
            "{index | indices: [..] (paint-order item indices), ids?, target?} remove those fills/strokes from each target object's own stack",
            has_doc,
            remove_item
        ),
        cmd!(
            "appearance.addEffect",
            "Add Effect",
            [],
            None,
            "same as effect.apply (an alias): {effect: id, params?: {…}, item?: appearance item index|null, ids?, target?} → {ids, index, item}",
            has_doc,
            super::effectcmd::apply
        ),
        cmd!(
            "appearance.duplicateItem",
            "Duplicate Item",
            ["Window", "Appearance"],
            None,
            "{index | indices: [..] (paint-order item indices), to?: paint-order index of the copy (one `index` only; default: right above it, as Alt-dragging a row places it), ids?, target?} copy fills/strokes with their effects",
            has_doc,
            duplicate_item
        ),
        cmd!(
            "appearance.moveItem",
            "Reorder Appearance Item",
            [],
            None,
            "{from: paint-order item index | \"contents\", to, contents?: count, ids?, target?} move fill/stroke `from` to paint-order index `to`; on a group, layer or type the Contents (Characters) row stays between the same items (a moved item landing next to it keeps its side) unless `contents` says how many items paint below it afterwards. from \"contents\": move that row so `to` items paint below the members (characters) and the rest above → {index: where the item landed | contents: the row's slot}",
            has_doc,
            move_item
        ),
        cmd!(
            "appearance.copyFrom",
            "Eyedropper",
            [],
            None,
            "{source: id, ids?, pickUp?, apply? (trees as eyedropper.setOptions, merged over the Eyedropper Options for this call), reverse?: bool, append?: bool} copy the attributes both picked up and applied (by default the whole appearance stack, opacity and blend mode, and from type to type the character and paragraph attributes) from `source` to ids (default: the selection), the fill, stroke and weight also to the paint defaults; `reverse` (Alt-click) copies from the first selected object (or ids) onto `source` instead; `append` (Shift+Alt-click) adds the source's fills, strokes and effects on top of each target's stack; placed gradients land at the same place relative to each target's bounds (defaults: fitted to new art) → {ids}",
            has_doc,
            copy_from
        ),
        cmd!(
            "appearance.setActiveItem",
            "Select Appearance Item",
            [],
            None,
            "{index: paint-order item index in the first selected object's stack | null} make that fill/stroke row the target of the paint.setFill/setStroke, stroke.set/setAdvanced, paint.editGradient/setGradientGeom, transparency.set and effect.* calls that omit `item` (a fill row brings the Fill proxy forward, a stroke row the Stroke proxy); null or any selection change clears it → {index}",
            has_selection,
            set_active_item
        ),
        cmd!(
            "appearance.showAllHidden",
            "Show All Hidden Attributes",
            ["Window", "Appearance"],
            None,
            "{ids?, target?} make every hidden fill, stroke and effect (the object's and each item's) of each selected object visible again; errors when nothing is hidden → {ids}",
            has_doc,
            show_all_hidden
        ),
        cmd!(
            "appearance.targetContents",
            "Target Contents",
            [],
            None,
            "{ids?} select the members of the selected groups and layers (or `ids`), as double-clicking the Appearance panel's Contents row does: the panel then lists, and appearance edits change, their own appearance; errors when none has members → {ids}",
            has_doc,
            target_contents
        ),
        cmd!(
            "appearance.transfer",
            "Move Appearance",
            [],
            None,
            "{source: id, target: id, copy?: bool} give `target` (a layer, group or object) the appearance (fills, strokes, effects) and transparency (opacity, blend mode, isolation, knockout) of `source`, as dragging a target circle onto another in the Layers panel does; the source is left with a cleared appearance (as appearance.clear) unless `copy` (Alt-drag). Opacity masks stay; placed gradients keep their place relative to each object's bounds → {source, target}",
            has_doc,
            transfer
        ),
    ]
}

/// The Appearance panel's active row, remembered for the selection it was chosen in.
#[derive(Clone, Debug)]
pub(crate) struct ActiveItem {
    doc: u64,
    objects: Vec<NodeId>,
    index: usize,
}

impl Session {
    /// The Appearance panel's active fill/stroke: a paint-order index into the first selected
    /// object's stack, while the selection it was chosen for is unchanged.
    pub fn appearance_item(&self) -> Option<usize> {
        let a = self.active_appearance_item.as_ref()?;
        let st = self.active()?;
        if st.uid != a.doc || st.selection.subjects() != a.objects.as_slice() {
            return None;
        }
        let n = st.doc.node(*a.objects.first()?)?;
        (a.index < n.appearance.items.len()).then_some(a.index)
    }

    /// Re-point the active item after an edit of `ids` changed its stack (`None` drops it). Edits
    /// of other objects leave it alone.
    fn remap_appearance_item(&mut self, ids: &[NodeId], f: impl FnOnce(usize) -> Option<usize>) {
        if let Some(a) = &mut self.active_appearance_item
            && a.objects.first().is_some_and(|o| ids.contains(o))
        {
            match f(a.index) {
                Some(i) => a.index = i,
                None => self.active_appearance_item = None,
            }
        }
    }
}

/// Which fill or stroke of an appearance stack an edit changes (the `item` param).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ItemTarget {
    /// The topmost fill or stroke (for effects: the object-level effects).
    Top,
    /// Item `index`. `explicit`: named by the `item` param, so it must exist and be of the edited
    /// kind; otherwise it is the panel's active item, which applies only where it fits.
    Item { index: usize, explicit: bool },
}

/// Parse the `item` param of `cmd` (see the module docs).
pub(crate) fn item_target(s: &Session, p: &Value, cmd: &str) -> Result<ItemTarget> {
    item_target_at(s, p, "item", cmd)
}

/// [`item_target`] read from param `key` (e.g. `fromItem`).
pub(crate) fn item_target_at(s: &Session, p: &Value, key: &str, cmd: &str) -> Result<ItemTarget> {
    match p.get(key) {
        Some(Value::Null) => Ok(ItemTarget::Top),
        Some(v) => v
            .as_u64()
            .map(|i| ItemTarget::Item { index: i as usize, explicit: true })
            .ok_or_else(|| bad(cmd, format!("`{key}` must be an appearance item index (paint order) or null"))),
        // The active item is a row of the first selected object's own stack.
        None if p.get("ids").is_some() || p.get("id").is_some() || targets_contents(p)? => Ok(ItemTarget::Top),
        None => Ok(s.appearance_item().map_or(ItemTarget::Top, |index| ItemTarget::Item { index, explicit: false })),
    }
}

impl ItemTarget {
    /// Objects a fill/stroke edit changes: the selected objects' own stacks when an item is
    /// targeted, else the painted leaves ([`paint_targets`]).
    pub(crate) fn targets(self, s: &Session, p: &Value) -> Result<Vec<NodeId>> {
        match self {
            ItemTarget::Top => paint_targets(s, p),
            ItemTarget::Item { .. } => appearance_targets(s, p),
        }
    }

    /// The target of a fill (`fill`) or stroke edit: the panel's active item stands only for edits
    /// of its own kind, so a fill row leaves stroke edits on the topmost stroke of the painted
    /// leaves (as without an active item) and vice versa. Explicit items are kept (and checked).
    pub(crate) fn of_kind(self, s: &Session, fill: bool) -> Self {
        let ItemTarget::Item { index, explicit: false } = self else { return self };
        let fits =
            s.active().and_then(|st| st.doc.node(*st.selection.subjects().first()?)?.appearance.items.get(index).map(|it| it.is_fill() == fill));
        if fits == Some(true) { self } else { ItemTarget::Top }
    }

    /// The item of `ap` a fill (`fill`) or stroke edit changes; `None` = the topmost one.
    pub(crate) fn resolve(self, ap: &Appearance, fill: bool, cmd: &str) -> Result<Option<usize>> {
        match self {
            ItemTarget::Top => Ok(None),
            ItemTarget::Item { index, explicit } => match ap.item_of_kind(Some(index), fill) {
                None if explicit => Err(bad(
                    cmd,
                    match ap.items.get(index) {
                        None => format!("no appearance item {index}"),
                        Some(_) => format!("appearance item {index} is not a {}", if fill { "fill" } else { "stroke" }),
                    },
                )),
                found => Ok(found),
            },
        }
    }

    /// The item whose own effects an effect edit changes; `None` = the object-level effects.
    pub(crate) fn effects_item(self, ap: &Appearance, cmd: &str) -> Result<Option<usize>> {
        match self {
            ItemTarget::Top => Ok(None),
            ItemTarget::Item { index, .. } if index < ap.items.len() => Ok(Some(index)),
            ItemTarget::Item { index, explicit: true } => Err(bad(cmd, format!("no appearance item {index}"))),
            ItemTarget::Item { explicit: false, .. } => Ok(None),
        }
    }
}

/// Whether the `target` param asks for the contents of groups and layers (see the module docs).
fn targets_contents(p: &Value) -> Result<bool> {
    match p.get("target") {
        None | Some(Value::Null) => Ok(false),
        Some(v) => match v.as_str() {
            Some("object") => Ok(false),
            Some("contents") => Ok(true),
            _ => Err(bad("appearance", format!("`target` must be \"object\" or \"contents\", not {v}"))),
        },
    }
}

/// The objects appearance edits act on without `ids`: the targeted object, group or layer
/// (`layer.target`), else the selected objects (a selected compound-path member stands for its
/// compound).
pub(crate) fn subject_roots(s: &Session) -> Result<Vec<NodeId>> {
    match s.doc()?.selection.target {
        Some(t) => Ok(vec![t]),
        None => selected_roots(s),
    }
}

/// Objects whose own appearance stack a command edits: `ids`/`id` (layers too), else the
/// [`subject_roots`]; with `target: "contents"` the painted objects inside the groups and layers
/// among them ([`leaf_targets`]).
pub(crate) fn appearance_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let roots = if p.get("ids").is_none() && p.get("id").is_none() {
        subject_roots(s)?
    } else {
        let d = &s.doc()?.doc;
        targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some()).collect()
    };
    if targets_contents(p)? { leaf_targets(s, &roots) } else { Ok(roots) }
}

/// [`appearance_targets`] of `cmd`, which needs at least one.
fn require_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let ids = appearance_targets(s, p)?;
    if ids.is_empty() {
        return Err(bad(cmd, "select objects or give ids"));
    }
    Ok(ids)
}

/// One undo step (`label`) running `f` on each of `ids` with the item a fill (`fill`) or stroke
/// edit aimed at `item` changes there (`None`: the topmost one; text then edits its characters).
/// Nothing targeted is not an edit.
pub(crate) fn edit_items(
    s: &mut Session,
    ids: &[NodeId],
    item: ItemTarget,
    cmd: &str,
    label: &str,
    fill: bool,
    f: impl FnMut(&mut Node, Option<usize>) -> Result<()>,
) -> Result<()> {
    edit_items_then(s, ids, item, cmd, label, fill, f, |_, _| Ok(()))
}

/// [`edit_items`], then `after` on the ids `f` changed, inside the same undo step.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edit_items_then(
    s: &mut Session,
    ids: &[NodeId],
    item: ItemTarget,
    cmd: &str,
    label: &str,
    fill: bool,
    mut f: impl FnMut(&mut Node, Option<usize>) -> Result<()>,
    mut after: impl FnMut(&mut vectorcraft_doc::Document, &[NodeId]) -> Result<()>,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    s.edit(label, |d, _| {
        let mut changed = Vec::new();
        for id in ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let index = item.resolve(&n.appearance, fill, cmd)?;
            f(n, index)?;
            changed.push(*id);
        }
        after(d, &changed)
    })
}

/// Whether an edit aimed at `item` changes a stroke: the `stroke` param, else the kind of the
/// targeted item (on the first target object), else `default`.
pub(crate) fn edits_stroke(s: &Session, p: &Value, item: ItemTarget, default: bool) -> Result<bool> {
    if let Some(b) = p.get("stroke").and_then(Value::as_bool) {
        return Ok(b);
    }
    let ItemTarget::Item { index, .. } = item else { return Ok(default) };
    let ids = appearance_targets(s, p)?;
    let d = &s.doc()?.doc;
    Ok(ids.first().and_then(|id| d.node(*id)?.appearance.items.get(index)).map_or(default, |it| !it.is_fill()))
}

pub(crate) fn index_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).map(|i| i as usize).ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
}

fn set_active_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.setActiveItem";
    let index = match p.get("index") {
        Some(Value::Null) => {
            s.active_appearance_item = None;
            return Ok(json!({ "index": Value::Null }));
        }
        Some(_) => index_param(p, "index", C)?,
        None => return Err(bad(C, "missing `index` (an item index or null)")),
    };
    let st = s.doc()?;
    let objects = st.selection.subjects().to_vec();
    let first = objects.first().and_then(|id| st.doc.node(*id)).ok_or_else(|| EngineError::Other("nothing selected".into()))?;
    let fill = first.appearance.items.get(index).ok_or_else(|| bad(C, format!("no appearance item {index}")))?.is_fill();
    let doc = st.uid;
    s.fill_active = fill;
    s.active_appearance_item = Some(ActiveItem { doc, objects, index });
    Ok(json!({ "index": index }))
}

fn add_item(s: &mut Session, p: &Value, fill: bool) -> Result<Value> {
    let ids = require_targets(s, p, if fill { "appearance.addFill" } else { "appearance.addStroke" })?;
    let defaults = s.paint.clone();
    s.edit(if fill { "Add New Fill" } else { "Add New Stroke" }, |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                // A copy of the top fill (stroke); without one, type's characters' and otherwise the
                // default paint, so a new fill on a group or type shows.
                let paint = match (n.appearance.paint_at(None, fill), &n.kind) {
                    (Some(p), _) => p.clone(),
                    (None, NodeKind::Text(_)) => super::paint::proxy_paint(n, !fill, None),
                    (None, _) => {
                        if fill {
                            defaults.fill.clone()
                        } else {
                            defaults.stroke.clone()
                        }
                    }
                };
                let item = if fill {
                    AppearanceItem::Fill(FillLayer::new(paint))
                } else {
                    AppearanceItem::Stroke(StrokeLayer::new(paint, n.appearance.stroke_width().max(1.0)))
                };
                n.appearance.items.push(item);
            }
        }
        Ok(())
    })?;
    ok()
}

fn clear_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = require_targets(s, p, "appearance.clear")?;
    s.edit("Clear Appearance", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                clear(n);
            }
        }
        Ok(())
    })?;
    s.active_appearance_item = None;
    ok()
}

/// Clear Appearance on `n`: no fill, no stroke, and the object's Opacity row back to Default. A
/// group keeps no fill or stroke rows of its own.
fn clear(n: &mut Node) {
    n.appearance = if is_group(n) { Appearance::default() } else { Appearance::basic(Paint::None, Paint::None, 1.0) };
    n.opacity = 1.0;
    n.blend = BlendMode::Normal;
}

fn transfer(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.transfer";
    let source = id_param(p, "source").ok_or_else(|| bad(C, "missing `source` id"))?;
    let target = id_param(p, "target").ok_or_else(|| bad(C, "missing `target` id"))?;
    if source == target {
        return Err(bad(C, "`source` and `target` are the same object"));
    }
    let copy = bool_or(p, "copy", false);
    s.edit(if copy { "Copy Appearance" } else { "Move Appearance" }, |d, _| {
        let src = d.node(source).cloned().ok_or(EngineError::NoNode(source))?;
        let to = d.node_mut(target).ok_or(EngineError::NoNode(target))?;
        let mut ap = src.appearance.clone();
        if let (Some(f), Some(t)) = (src.geometric_bounds(), to.geometric_bounds()) {
            ap.rebase_gradients(f, t);
        }
        // Only groups, layers and type have a Contents (Characters) row to keep a slot for.
        if to.contents_label().is_none() {
            ap.contents_index = None;
        }
        to.appearance = ap;
        (to.opacity, to.blend, to.isolate, to.knockout, to.knockout_shape) = (src.opacity, src.blend, src.isolate, src.knockout, src.knockout_shape);
        if !copy && let Some(n) = d.node_mut(source) {
            clear(n);
            (n.isolate, n.knockout, n.knockout_shape) = (false, Default::default(), false);
        }
        Ok(())
    })?;
    s.active_appearance_item = None;
    Ok(json!({ "source": source.0, "target": target.0 }))
}

/// A group or layer (no fill or stroke rows of its own unless added).
fn is_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

fn reduce_basic(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = require_targets(s, p, "appearance.reduceToBasic")?;
    s.edit("Reduce to Basic Appearance", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                // The topmost visible fill and stroke stay, without their own transparency or effects.
                let top = |fill: bool| n.appearance.items.iter().rev().find(|i| i.visible() && i.is_fill() == fill).cloned();
                let (fill, stroke) = (top(true), top(false));
                // Other objects always keep a fill row (None when there was none); a group only
                // keeps the rows it had.
                let fill =
                    (fill.is_some() || !is_group(n)).then(|| AppearanceItem::Fill(FillLayer::new(fill.map_or(Paint::None, |f| f.paint().clone()))));
                n.appearance = Appearance { items: fill.into_iter().collect(), ..Default::default() };
                if let Some(AppearanceItem::Stroke(mut st)) = stroke {
                    st.effects.clear();
                    st.opacity = 1.0;
                    st.blend = BlendMode::Normal;
                    n.appearance.items.push(AppearanceItem::Stroke(st));
                }
            }
        }
        Ok(())
    })?;
    s.active_appearance_item = None;
    ok()
}

fn set_item(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = index_param(p, "index", "appearance.setItem")?;
    let paint = paint_from(s, p)?;
    let ids = appearance_targets(s, p)?;
    if ids.is_empty() {
        return Err(bad("appearance.setItem", "select objects or give ids"));
    }
    let blend = str_param(p, "blend").and_then(BlendMode::parse);
    s.edit("Appearance", |d, _| {
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let bounds = n.geometric_bounds();
            let Some(item) = n.appearance.items.get_mut(idx) else { continue };
            let (pp, op, bl, vis, bounds) = match item {
                AppearanceItem::Fill(f) => (&mut f.paint, &mut f.opacity, &mut f.blend, &mut f.visible, bounds),
                AppearanceItem::Stroke(st) => {
                    if let Some(w) = p.get("weight").and_then(Value::as_f64) {
                        st.width = w.max(0.0);
                    }
                    let bounds = bounds.map(|b| st.paint_bounds(b));
                    (&mut st.paint, &mut st.opacity, &mut st.blend, &mut st.visible, bounds)
                }
            };
            if let Some(pa) = &paint {
                *pp = super::gradient::place_paint(pa, p, bounds);
            }
            if let Some(o) = p.get("opacity").and_then(Value::as_f64) {
                *op = percent(o);
            }
            if let Some(b) = blend {
                *bl = b;
            }
            if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                *vis = v;
            }
        }
        Ok(())
    })?;
    if let Some(pa) = &paint {
        s.remember_paint(pa);
    }
    ok()
}

/// The paint-order item indices a command acts on (`indices`, else `index`), ascending and
/// without repeats.
fn item_indices(p: &Value, cmd: &str) -> Result<Vec<usize>> {
    let mut v = match p.get("indices") {
        Some(a) => a
            .as_array()
            .and_then(|a| a.iter().map(|x| x.as_u64().map(|i| i as usize)).collect::<Option<Vec<_>>>())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| bad(cmd, "`indices` must be a non-empty array of item indices"))?,
        None => vec![index_param(p, "index", cmd)?],
    };
    v.sort_unstable();
    v.dedup();
    Ok(v)
}

fn remove_item(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = item_indices(p, "appearance.removeItem")?;
    let ids = require_targets(s, p, "appearance.removeItem")?;
    s.edit("Remove Item", |d, _| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                // From the top down, so the lower indices stay valid.
                for &i in idx.iter().rev() {
                    n.appearance.remove_item(i);
                }
            }
        }
        Ok(())
    })?;
    s.remap_appearance_item(&ids, |a| (!idx.contains(&a)).then(|| a - idx.iter().filter(|i| **i < a).count()));
    ok()
}

fn duplicate_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.duplicateItem";
    let idx = item_indices(p, C)?;
    let to = p.get("to").map(|_| index_param(p, "to", C)).transpose()?;
    if to.is_some() && idx.len() > 1 {
        return Err(bad(C, "`to` takes a single `index`"));
    }
    let ids = require_targets(s, p, C)?;
    // Where the first object's copy landed (the active item lives in the first object's stack).
    let mut first_at = None;
    s.edit("Duplicate Item", |d, _| {
        let mut any = false;
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            let ap = &mut n.appearance;
            // From the top down, so the lower indices stay valid.
            for &i in idx.iter().rev() {
                if let Some(item) = ap.items.get(i).cloned() {
                    let at = to.map_or(i + 1, |t| t.min(ap.items.len()));
                    first_at.get_or_insert(at);
                    ap.insert_item(at, item);
                    any = true;
                }
            }
        }
        if any { Ok(()) } else { Err(bad(C, format!("no item at index {}", idx[0]))) }
    })?;
    // Rows at or above each copy move up by one.
    s.remap_appearance_item(&ids, |a| {
        Some(match to {
            Some(_) => a + usize::from(first_at.is_some_and(|at| at <= a)),
            None => a + idx.iter().filter(|i| **i < a).count(),
        })
    });
    ok()
}

fn show_all_hidden(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = require_targets(s, p, "appearance.showAllHidden")?;
    let ids = s.edit("Show All Hidden Attributes", |d, _| {
        let shown: Vec<NodeId> = roots.iter().copied().filter(|id| d.node_mut(*id).is_some_and(|n| n.appearance.show_all())).collect();
        if shown.is_empty() {
            return Err(EngineError::Other("Show All Hidden Attributes: nothing is hidden".into()));
        }
        Ok(shown)
    })?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.moveItem";
    let to = index_param(p, "to", C)?;
    let ids = require_targets(s, p, C)?;
    if p.get("from").and_then(Value::as_str) == Some("contents") {
        return move_contents(s, &ids, to);
    }
    let from = index_param(p, "from", C)?;
    let contents = p.get("contents").map(|_| index_param(p, "contents", C)).transpose()?;
    let mut landed = to;
    s.edit("Reorder Appearance", |d, _| {
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            landed = n.appearance.move_item(from, to).ok_or_else(|| bad(C, format!("no item at index {from}")))?;
            if let Some(k) = contents.filter(|_| n.contents_label().is_some()) {
                n.appearance.set_contents_at(k);
            }
        }
        Ok(())
    })?;
    // The moved row stays the active one; the rows it passed shift by one.
    s.remap_appearance_item(&ids, |a| {
        Some(if a == from {
            landed
        } else if from < a && a <= landed {
            a - 1
        } else if landed <= a && a < from {
            a + 1
        } else {
            a
        })
    });
    Ok(json!({ "index": landed }))
}

/// `appearance.moveItem {from: "contents"}`: put the Contents (Characters) row of each target that
/// has one above its bottom `to` items.
fn move_contents(s: &mut Session, ids: &[NodeId], to: usize) -> Result<Value> {
    let slot = s.edit("Reorder Appearance", |d, _| {
        let mut slot = None;
        for id in ids {
            let Some(n) = d.node_mut(*id).filter(|n| n.contents_label().is_some()) else { continue };
            n.appearance.set_contents_at(to);
            slot.get_or_insert(n.appearance.contents_at());
        }
        slot.ok_or_else(|| bad("appearance.moveItem", "only groups, layers and type have a Contents (Characters) row"))
    })?;
    Ok(json!({ "contents": slot }))
}

fn target_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let roots = appearance_targets(s, p)?;
    let d = &s.doc()?.doc;
    let ids: Vec<NodeId> = roots
        .iter()
        .filter_map(|id| d.node(*id))
        .filter(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }))
        .flat_map(|n| n.children().into_iter().flatten().filter(|c| c.visible && !c.locked).map(|c| c.id))
        .collect();
    if ids.is_empty() {
        return Err(bad("appearance.targetContents", "select a group or layer with visible, unlocked members"));
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

fn copy_from(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.copyFrom";
    let clicked = id_param(p, "source").ok_or_else(|| bad(C, "missing `source` id"))?;
    let (reverse, append) = (bool_or(p, "reverse", false), bool_or(p, "append", false));
    let attrs = s.prefs.eyedropper.attrs(p).map_err(|e| bad(C, e))?;
    let selected = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    // Reversed, the first selected object's attributes go onto the clicked object.
    let (src_id, mut targets) = if reverse {
        let first = leaf_targets(s, &selected)?.into_iter().next().ok_or_else(|| bad(C, "select the object to copy from"))?;
        (first, leaf_targets(s, &[clicked])?)
    } else {
        (clicked, leaf_targets(s, &selected)?)
    };
    targets.retain(|id| *id != src_id);
    let picked = Picked::of(s.doc()?.doc.node(src_id).ok_or(EngineError::NoNode(src_id))?);
    if !reverse {
        picked.to_defaults(s, &attrs.appearance);
    }
    if targets.is_empty() || attrs.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    s.edit("Eyedropper", |d, _| {
        for id in &targets {
            if let Some(n) = d.node_mut(*id) {
                picked.apply(n, &attrs, append);
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": targets.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// What the Eyedropper picks up from an object.
struct Picked {
    /// The appearance stack (type: its first run's fill and stroke as a basic appearance).
    appearance: Appearance,
    /// The box (in the source's paint space) its placed gradients are relative to.
    bounds: Option<Rect>,
    opacity: f32,
    blend: BlendMode,
    /// Type's first character style and its paragraph attributes.
    text: Option<(CharStyle, ParaStyle)>,
}

impl Picked {
    fn of(n: &Node) -> Self {
        let (appearance, bounds, text) = match &n.kind {
            NodeKind::Text(t) if !t.runs.is_empty() => {
                let st = &t.runs[0].style;
                (st.basic_appearance(), Some(t.local_bounds()), Some((st.clone(), t.para.clone())))
            }
            _ => (n.appearance.clone(), n.geometric_bounds(), None),
        };
        Self { appearance, bounds, opacity: n.opacity, blend: n.blend, text }
    }

    /// The appearance placed on an object whose box (in its paint space) is `to`: placed gradients
    /// land at the same place relative to it.
    fn placed(&self, to: Option<Rect>) -> Appearance {
        let mut ap = self.appearance.clone();
        if let (Some(f), Some(t)) = (self.bounds, to) {
            ap.rebase_gradients(f, t);
        }
        ap
    }

    /// The paint defaults for new art take the picked-up fill, stroke and weight.
    fn to_defaults(&self, s: &mut Session, a: &AppearanceAttrs) {
        if a.fill.color {
            s.paint.fill = super::gradient::unplaced(&self.appearance.fill_paint());
            s.remember_paint(&s.paint.fill.clone());
        }
        if a.stroke.color {
            s.paint.stroke = super::gradient::unplaced(&self.appearance.stroke_paint());
        }
        if a.stroke.weight && self.appearance.stroke().is_some() {
            s.paint.stroke_width = self.appearance.stroke_width();
        }
    }

    /// Apply attributes `a` to `n`. `append` adds the fills and strokes (and effects) on top of its
    /// stack instead. Type takes the paints on its characters, and character and paragraph
    /// attributes from type.
    fn apply(&self, n: &mut Node, a: &EyedropperAttrs, append: bool) {
        let ap = &a.appearance;
        if ap.transparency {
            n.opacity = self.opacity;
            n.blend = self.blend;
        }
        if append {
            let placed = self.placed(n.geometric_bounds());
            n.appearance.items.extend(placed.items.into_iter().filter(|it| if it.is_fill() { ap.fill.color } else { ap.stroke.color }));
            n.appearance.effects.extend(placed.effects);
            return;
        }
        if let NodeKind::Text(t) = &mut n.kind {
            let placed = self.placed(Some(t.local_bounds()));
            let text = self.text.as_ref();
            for r in &mut t.runs {
                if let Some((cs, _)) = text.filter(|_| a.character) {
                    // The character attributes; the paints follow the appearance attributes.
                    let st = &mut r.style;
                    *st = CharStyle {
                        fill: std::mem::take(&mut st.fill),
                        stroke: std::mem::take(&mut st.stroke),
                        stroke_width: st.stroke_width,
                        overprint_fill: st.overprint_fill,
                        overprint_stroke: st.overprint_stroke,
                        ..cs.clone()
                    };
                }
                run_attrs(&mut r.style, &placed, ap);
            }
            if let Some((_, para)) = text.filter(|_| a.paragraph) {
                // The source's first paragraph's attributes, on every paragraph.
                t.set_all_paras(para.clone());
            }
            if text.is_some() && (a.character || a.paragraph) {
                super::typecmd::refresh_bounds(t);
            }
            return;
        }
        let placed = self.placed(n.geometric_bounds());
        if ap.fill == FillAttrs::ALL && ap.stroke == StrokeAttrs::ALL {
            n.appearance = placed;
        } else {
            focal_attrs(&mut n.appearance, &placed, ap);
        }
    }
}

/// The focal (topmost) fill and stroke attributes `a` of `from` onto `to`. Taking a colour creates
/// a missing fill or stroke.
fn focal_attrs(to: &mut Appearance, from: &Appearance, a: &AppearanceAttrs) {
    if a.fill.color {
        to.set_fill(from.fill_paint());
    }
    if let (Some(t), Some(f)) = (to.fill_mut(), from.fill()) {
        if a.fill.transparency {
            (t.opacity, t.blend) = (f.opacity, f.blend);
        }
        if a.fill.overprint {
            t.overprint = f.overprint;
        }
    }
    if a.stroke.color {
        to.set_stroke(from.stroke_paint());
    }
    if let (Some(t), Some(k)) = (to.stroke_mut(), from.stroke()) {
        let s = &a.stroke;
        if s.transparency {
            (t.opacity, t.blend) = (k.opacity, k.blend);
        }
        if s.overprint {
            t.overprint = k.overprint;
        }
        if s.weight {
            t.width = k.width;
        }
        if s.cap {
            t.cap = k.cap;
        }
        if s.join {
            t.join = k.join;
        }
        if s.miter {
            t.miter_limit = k.miter_limit;
        }
        if s.dash {
            t.dash = k.dash.clone();
        }
    }
}

/// The fill and stroke attributes `a` of `from` onto a type run's characters (their stroke takes
/// what characters have: paint, weight, cap, join, miter limit, dashes and overprint). Characters
/// without a painted stroke get no weight, a painted one at least 1 pt.
fn run_attrs(st: &mut CharStyle, from: &Appearance, a: &AppearanceAttrs) {
    let mut ap = st.basic_appearance();
    focal_attrs(&mut ap, from, a);
    let (Some(f), Some(k)) = (ap.fill(), ap.stroke()) else { return };
    (st.fill, st.overprint_fill, st.overprint_stroke) = (f.paint.clone(), f.overprint, k.overprint);
    let width = if a.stroke.color { ap.stroke_width() } else { k.width };
    st.set_stroke_layer(&StrokeLayer { width, ..k.clone() });
    if !st.stroke.is_none() {
        super::gradient::run_stroke_weight(st);
    }
}
