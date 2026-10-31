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
//! `appearance.moveItem {from: "contents"}`). An image's own fills and strokes paint only the
//! shape that a Convert to Shape or Outline Object effect on them gives them, below or above the
//! image as its Contents row says; new ones go right below that row.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::appearance::SharedStack;
use vectorcraft_doc::{Appearance, AppearanceItem, CharStyle, FillLayer, Node, NodeKind, ParaStyle, StrokeLayer};
use vectorcraft_geom::Rect;

use super::edit::{outermost, selected_roots};
use super::opacitymask::percent;
use super::paint::paint_from;
use super::{AppearanceAttrs, EyedropperAttrs, FillAttrs, StrokeAttrs};
use super::{
    CommandSpec, NodeId, Result, Session, bad, check_numbers, check_switches, cmd, has_doc, has_selection, id_param, ids_param, index, leaf_targets,
    ok, paint_targets, str_param, switch_or, targets,
};
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "appearance.addFill",
            "Add New Fill",
            ["Window", "Appearance"],
            Some("Cmd+/"),
            "{ids?|id?, target?: \"object\"|\"contents\"} add a fill on top of each selected object's own stack, on an image right below its Contents row (a copy of its top fill; type without one: its characters' fill; else the default fill)",
            has_doc,
            |s, p| add_item(s, p, true)
        ),
        cmd!(
            "appearance.addStroke",
            "Add New Stroke",
            ["Window", "Appearance"],
            Some("Cmd+Alt+/"),
            "{ids?|id?, target?} add a stroke on top of each selected object's own stack, on an image right below its Contents row (a copy of its top stroke, else the default stroke)",
            has_doc,
            |s, p| add_item(s, p, false)
        ),
        cmd!(
            "appearance.clear",
            "Clear Appearance",
            ["Window", "Appearance"],
            None,
            "{ids?|id?, target?} leave each object one empty fill and stroke (None; a group none of its own) and reset its opacity and blend mode",
            has_doc,
            clear_appearance
        ),
        cmd!(
            "appearance.reduceToBasic",
            "Reduce to Basic Appearance",
            ["Window", "Appearance"],
            None,
            "{ids?|id?, target?} keep only the topmost visible fill and stroke, without their own opacity, blend mode or effects (a group keeps no rows it lacked), and drop the object's effects",
            has_doc,
            reduce_basic
        ),
        cmd!(
            "appearance.setItem",
            "Appearance Item",
            [],
            None,
            "{index: paint-order item index, ids?|id? (default: the selection), target?: \"object\"|\"contents\", opacity?: 0..100, blend?: name, visible?: bool, weight?: pt (strokes), color?|none?|swatch?|gradient? (as paint.setFill), tint?: 0..100, keepModel?: bool (both as paint.setFill), aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} edit one fill/stroke of each target object's own appearance stack",
            has_doc,
            set_item
        ),
        cmd!(
            "appearance.removeItem",
            "Remove Item",
            [],
            None,
            "{index | indices: [..] (paint-order item indices), ids?|id?, target?, aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} remove those fills/strokes from each target object's own stack",
            has_doc,
            remove_item
        ),
        cmd!(
            "appearance.addEffect",
            "Add Effect",
            [],
            None,
            "{…as effect.apply} same as effect.apply (an alias): {effect: id, params?: {…}, item?: appearance item index|null, ids?, target?} → {ids, index, item}",
            has_doc,
            super::effectcmd::apply
        ),
        cmd!(
            "appearance.duplicateItem",
            "Duplicate Item",
            ["Window", "Appearance"],
            None,
            "{index | indices: [..] (paint-order item indices), to?: paint-order index of the copy (one `index` only; default: right above it, as Alt-dragging a row places it; not with aligned rows of a mixed selection), ids?|id?, target?, aligned?: bool (index and item are rows of the first target's stack: each other target edits its equal one, as the Appearance panel does for a mixed selection)} copy fills/strokes with their effects",
            has_doc,
            duplicate_item
        ),
        cmd!(
            "appearance.moveItem",
            "Reorder Appearance Item",
            [],
            None,
            "{from: paint-order item index | \"contents\", to, contents?: count, ids?|id?, target?, aligned?: bool (refused when the targets' appearances differ)} move fill/stroke `from` to paint-order index `to`; on a group, layer or type the Contents (Characters) row stays between the same items (a moved item landing next to it keeps its side) unless `contents` says how many items paint below it afterwards. from \"contents\": move that row so `to` items paint below the members (characters) and the rest above → {index: where the item landed | contents: the row's slot}",
            has_doc,
            move_item
        ),
        cmd!(
            "appearance.copyFrom",
            "Eyedropper",
            [],
            None,
            "{source: id, ids?, pickUp?, apply? (trees as eyedropper.setOptions, merged over the Eyedropper Options for this call), reverse?: bool, append?: bool, defaultsOnly?: bool} copy the attributes both picked up and applied (by default the whole appearance stack, opacity and blend mode, and from type to type the character and paragraph attributes) from `source` to ids (default: highlighted characters while editing type, otherwise the object selection; highlighted characters reject enabled non-neutral object transparency), the fill, stroke and weight also to the paint defaults; `defaultsOnly` updates only paint defaults and cannot be combined with ids, reverse or append; `reverse` (Alt-click) copies from the first selected object (or ids) onto `source` instead, or with nothing selected what the Eyedropper picked up last (a click without Alt); `append` (Shift+Alt-click) adds the source's fills, strokes and effects on top of each target's stack; placed gradients land at the same place relative to each target's bounds (defaults: fitted to new art) → {ids}",
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
            "{ids?|id?, target?} make every hidden fill, stroke and effect (the object's and each item's) of each selected object visible again; errors when nothing is hidden → {ids}",
            has_doc,
            show_all_hidden
        ),
        cmd!(
            "appearance.targetContents",
            "Target Contents",
            [],
            None,
            "{ids?|id?, target?: \"object\"|\"contents\"} select the members of the selected groups and layers (or `ids`), as double-clicking the Appearance panel's Contents row does: the panel then lists, and appearance edits change, their own appearance; errors when none has members → {ids}",
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
    /// kind; otherwise it is the panel's active item, which applies only where it fits. `aligned`:
    /// `index` is a row of the first target's stack, and each other target edits the item equal to
    /// it there ([`ItemTarget::per_target`]): the active item always is (it is a row of the first
    /// selected object's stack), an explicit one with `aligned: true`.
    Item { index: usize, explicit: bool, aligned: bool },
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
            .map(|i| ItemTarget::Item { index: index(i), explicit: true, aligned: aligned(p) })
            .ok_or_else(|| bad(cmd, format!("`{key}` must be an appearance item index (paint order) or null"))),
        // The active item is a row of the first selected object's own stack.
        None if p.get("ids").is_some() || p.get("id").is_some() || targets_contents(p)? => Ok(ItemTarget::Top),
        None => Ok(s.appearance_item().map_or(ItemTarget::Top, |index| ItemTarget::Item { index, explicit: false, aligned: true })),
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
        let ItemTarget::Item { index, explicit: false, .. } = self else { return self };
        let fits =
            s.active().and_then(|st| st.doc.node(*st.selection.subjects().first()?)?.appearance.items.get(index).map(|it| it.is_fill() == fill));
        if fits == Some(true) { self } else { ItemTarget::Top }
    }

    /// This target for each of `ids` in `d` (in their order): an aligned item (a row of the first
    /// target's stack) is, in each other target, the item equal to it there (see
    /// [`SharedStack`]); `None` for a target that has none, which the edit leaves alone. A row the
    /// targets don't share, and any other target, stands for itself in every target.
    pub(crate) fn per_target(self, d: &vectorcraft_doc::Document, ids: &[NodeId]) -> Vec<Option<ItemTarget>> {
        let ItemTarget::Item { index, explicit, aligned: true } = self else { return vec![Some(self); ids.len()] };
        let Some(shared) = shared_stack(d, ids) else { return vec![Some(self); ids.len()] };
        if !shared.has_item(index) {
            return vec![Some(self); ids.len()];
        }
        (0..ids.len()).map(|k| shared.item_in(index, k).map(|index| ItemTarget::Item { index, explicit, aligned: true })).collect()
    }

    /// The item of `ap` a fill (`fill`) or stroke edit changes; `None` = the topmost one.
    pub(crate) fn resolve(self, ap: &Appearance, fill: bool, cmd: &str) -> Result<Option<usize>> {
        match self {
            ItemTarget::Top => Ok(None),
            ItemTarget::Item { index, explicit, .. } => match ap.item_of_kind(Some(index), fill) {
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
            ItemTarget::Item { index, explicit: true, .. } => Err(bad(cmd, format!("no appearance item {index}"))),
            ItemTarget::Item { explicit: false, .. } => Ok(None),
        }
    }
}

/// Does `p` say its item and effect indices are rows of the first target's stack (`aligned`),
/// which each other target edits where it has them (see [`ItemTarget::per_target`])?
pub(crate) fn aligned(p: &Value) -> bool {
    p.get("aligned").and_then(Value::as_bool).unwrap_or(false)
}

/// What the appearances of `ids` share, when there are several of them and they all exist.
pub(crate) fn shared_stack(d: &vectorcraft_doc::Document, ids: &[NodeId]) -> Option<SharedStack> {
    if ids.len() < 2 {
        return None;
    }
    let aps: Vec<&Appearance> = ids.iter().map(|id| d.node(*id).map(|n| &*n.appearance)).collect::<Option<_>>()?;
    Some(SharedStack::of(&aps))
}

/// Do the appearances of `ids` differ (several objects whose stacks aren't all equal)?
pub(crate) fn appearances_differ(d: &vectorcraft_doc::Document, ids: &[NodeId]) -> bool {
    let mut aps = ids.iter().filter_map(|id| d.node(*id)).map(|n| &n.appearance);
    let Some(first) = aps.next() else { return false };
    aps.any(|a| a != first)
}

/// Indices `idx` of the first target's items (aligned with `aligned`) in each of `ids`: the first
/// target's own, and in each other one the items equal to them there (see [`SharedStack`]), each
/// index on its own: one the targets don't share is left out of the others (never taken as the
/// same index there). Not aligned, or one target: the same indices in each.
pub(crate) fn items_per_target(d: &vectorcraft_doc::Document, ids: &[NodeId], idx: &[usize], aligned: bool) -> Vec<Vec<usize>> {
    match shared_stack(d, ids).filter(|_| aligned) {
        Some(sh) => (0..ids.len()).map(|k| if k == 0 { idx.to_vec() } else { idx.iter().filter_map(|i| sh.item_in(*i, k)).collect() }).collect(),
        None => vec![idx.to_vec(); ids.len()],
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

/// Objects whose own appearance stack a command edits: `ids`/`id` (layers too; an id inside
/// another given one is dropped), else the [`subject_roots`]; with `target: "contents"` the painted
/// objects inside the groups and layers among them ([`leaf_targets`]).
pub(crate) fn appearance_targets(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let roots = if p.get("ids").is_none() && p.get("id").is_none() {
        subject_roots(s)?
    } else {
        let d = &s.doc()?.doc;
        outermost(d, targets(s, p)?.into_iter().filter(|id| d.node(*id).is_some()).collect())
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
/// Nothing targeted is not an edit, and when `f` leaves every target as it was, the document and
/// its undo history stay as they are.
pub(crate) fn edit_items(
    s: &mut Session,
    ids: &[NodeId],
    item: ItemTarget,
    cmd: &str,
    label: &str,
    fill: bool,
    f: impl FnMut(&mut Node, Option<usize>) -> Result<()>,
) -> Result<()> {
    edit_items_then(s, ids, item, cmd, label, fill, f, |_, _| Ok(false))
}

/// [`edit_items`], then `after` on the ids `f` ran on, inside the same undo step. `after` returns
/// whether it changed the document; when it did not and `f` left every target as it was, the
/// document and its undo history stay as they are.
#[allow(clippy::too_many_arguments)]
pub(crate) fn edit_items_then(
    s: &mut Session,
    ids: &[NodeId],
    item: ItemTarget,
    cmd: &str,
    label: &str,
    fill: bool,
    mut f: impl FnMut(&mut Node, Option<usize>) -> Result<()>,
    mut after: impl FnMut(&mut vectorcraft_doc::Document, &[NodeId]) -> Result<bool>,
) -> Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let before = s.doc()?.doc.clone();
    let per = item.per_target(&before, ids);
    super::overprint::edit_counted(s, label, |d| {
        let mut edited = Vec::new();
        for (id, item) in ids.iter().zip(&per) {
            let Some(item) = item else { continue };
            let Some(n) = d.node_mut(*id) else { continue };
            let index = item.resolve(&n.appearance, fill, cmd)?;
            f(n, index)?;
            edited.push(*id);
        }
        let more = after(d, &edited)?;
        Ok(count_changed(d, &before, &edited) + usize::from(more))
    })?;
    Ok(())
}

/// How many of `ids` differ in `d` from `before`.
pub(crate) fn count_changed(d: &vectorcraft_doc::Document, before: &vectorcraft_doc::Document, ids: &[NodeId]) -> usize {
    ids.iter().filter(|id| d.node(**id) != before.node(**id)).count()
}

/// Whether an edit aimed at `item` changes a stroke: the `stroke` param, else the kind of the
/// targeted item (on the first target object), else `default`. A `stroke` other than true, false or
/// null is an error of command `cmd`.
pub(crate) fn edits_stroke(s: &Session, p: &Value, item: ItemTarget, default: bool, cmd: &str) -> Result<bool> {
    check_switches(p, &["stroke"], cmd)?;
    if let Some(b) = p.get("stroke").and_then(Value::as_bool) {
        return Ok(b);
    }
    let ItemTarget::Item { index, .. } = item else { return Ok(default) };
    let ids = appearance_targets(s, p)?;
    let d = &s.doc()?.doc;
    Ok(ids.first().and_then(|id| d.node(*id)?.appearance.items.get(index)).map_or(default, |it| !it.is_fill()))
}

/// The whole number of 0 or more that `key` holds, read with [`index`].
pub(crate) fn index_param(p: &Value, key: &str, cmd: &str) -> Result<usize> {
    p.get(key).and_then(Value::as_u64).map(index).ok_or_else(|| bad(cmd, format!("missing integer `{key}`")))
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
    // Where the new item went in on the active item's object, when it is an image.
    let active = s.active_appearance_item.as_ref().and_then(|a| a.objects.first().copied());
    let mut inserted = None;
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
                if matches!(n.kind, NodeKind::Image(_)) {
                    // On an image it goes right below the Contents row (the image).
                    let k = n.appearance.contents_at();
                    n.appearance.items.insert(k, item);
                    n.appearance.set_contents_at(k + 1);
                    if active == Some(*id) {
                        inserted = Some(k);
                    }
                } else {
                    n.appearance.items.push(item);
                }
            }
        }
        Ok(())
    })?;
    // The rows above the new item move up by one.
    s.remap_appearance_item(&ids, |a| Some(a + usize::from(inserted.is_some_and(|k| k <= a))));
    ok()
}

fn clear_appearance(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = require_targets(s, p, "appearance.clear")?;
    // When every object is cleared already, the document and its undo history stay as they are.
    let before = s.doc()?.doc.clone();
    super::overprint::edit_counted(s, "Clear Appearance", |d| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                clear(n);
            }
        }
        Ok(count_changed(d, &before, &ids))
    })?;
    s.active_appearance_item = None;
    ok()
}

/// Clear Appearance on `n`: no fill, no stroke, and the object's Opacity row back to Default. A
/// group keeps no fill or stroke rows of its own.
fn clear(n: &mut Node) {
    n.appearance = if is_group(n) { Appearance::default().into() } else { Appearance::basic(Paint::None, Paint::None, 1.0).into() };
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
    let copy = switch_or(p, "copy", false, C)?;
    // When both objects come out as they were, the document and its undo history stay as they are.
    let before = s.doc()?.doc.clone();
    super::overprint::edit_counted(s, if copy { "Copy Appearance" } else { "Move Appearance" }, |d| {
        let src = d.node(source).cloned().ok_or(EngineError::NoNode(source))?;
        let to = d.node_mut(target).ok_or(EngineError::NoNode(target))?;
        let mut ap = src.appearance.clone();
        if let (Some(f), Some(t)) = (src.geometric_bounds(), to.geometric_bounds()) {
            ap.rebase_gradients(f, t);
        }
        // Only groups, layers, type and images have a Contents (Characters) row to keep a slot for.
        if to.contents_label().is_none() {
            ap.contents_index = None;
        }
        to.appearance = ap;
        (to.opacity, to.blend, to.isolate, to.knockout, to.knockout_shape) = (src.opacity, src.blend, src.isolate, src.knockout, src.knockout_shape);
        if !copy && let Some(n) = d.node_mut(source) {
            clear(n);
            (n.isolate, n.knockout, n.knockout_shape) = (false, Default::default(), false);
        }
        Ok(count_changed(d, &before, &[source, target]))
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
    // When every object comes out as it was, the document and its undo history stay as they are.
    let before = s.doc()?.doc.clone();
    super::overprint::edit_counted(s, "Reduce to Basic Appearance", |d| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                // The topmost visible fill and stroke stay, without their own transparency or effects.
                let top = |fill: bool| n.appearance.items.iter().rev().find(|i| i.visible() && i.is_fill() == fill).cloned();
                let (fill, stroke) = (top(true), top(false));
                // Other objects always keep a fill row (None when there was none); a group only
                // keeps the rows it had.
                let fill =
                    (fill.is_some() || !is_group(n)).then(|| AppearanceItem::Fill(FillLayer::new(fill.map_or(Paint::None, |f| f.paint().clone()))));
                n.appearance = Appearance { items: fill.into_iter().collect(), ..Default::default() }.into();
                if let Some(AppearanceItem::Stroke(mut st)) = stroke {
                    st.effects.clear();
                    st.opacity = 1.0;
                    st.blend = BlendMode::Normal;
                    n.appearance.items.push(AppearanceItem::Stroke(st));
                }
            }
        }
        Ok(count_changed(d, &before, &ids))
    })?;
    s.active_appearance_item = None;
    ok()
}

fn set_item(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = index_param(p, "index", "appearance.setItem")?;
    super::paint::check_keep_model(p, "appearance.setItem")?;
    check_numbers(p, &["weight", "opacity"], "appearance.setItem")?;
    check_switches(p, &["visible"], "appearance.setItem")?;
    let paint = paint_from(s, p)?;
    let ids = appearance_targets(s, p)?;
    if ids.is_empty() {
        return Err(bad("appearance.setItem", "select objects or give ids"));
    }
    let blend = str_param(p, "blend").and_then(BlendMode::parse);
    // When every object comes out as it was (for example, when none has item `index`), the document
    // and its undo history stay as they are.
    let before = s.doc()?.doc.clone();
    let per = items_per_target(&before, &ids, &[idx], aligned(p));
    super::overprint::edit_counted(s, "Appearance", |d| {
        for (id, at) in ids.iter().zip(&per) {
            let Some(&idx) = at.first() else { continue };
            let Some(n) = d.node_mut(*id) else { continue };
            if p.get("weight").is_none()
                && let Some(pa) = &paint
            {
                super::paint::restore_unpainted_stroke(&mut n.appearance, Some(idx), pa);
            }
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
                *pp = super::gradient::place_paint(&super::paint::keep_pattern_xf(pa, p, Some(&*pp)), p, bounds);
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
        Ok(count_changed(d, &before, &ids))
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
            .and_then(|a| a.iter().map(|x| x.as_u64().map(index)).collect::<Option<Vec<_>>>())
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
    // When no object has an item at any of the indices, the document and its undo history stay as
    // they are.
    let before = s.doc()?.doc.clone();
    let per = items_per_target(&before, &ids, &idx, aligned(p));
    super::overprint::edit_counted(s, "Remove Item", |d| {
        for (id, idx) in ids.iter().zip(&per) {
            if let Some(n) = d.node_mut(*id) {
                let mut idx = idx.clone();
                idx.sort_unstable();
                // From the top down, so the lower indices stay valid.
                for &i in idx.iter().rev() {
                    n.appearance.remove_item(i);
                }
            }
        }
        Ok(count_changed(d, &before, &ids))
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
    // Aligned rows of a mixed selection are copied right above themselves in each object.
    if aligned(p) && to.is_some() && shared_stack(&s.doc()?.doc, &ids).is_some() {
        return Err(bad(C, "`to` takes the rows of one object's stack, not aligned rows"));
    }
    let per = items_per_target(&s.doc()?.doc, &ids, &idx, aligned(p));
    s.edit("Duplicate Item", |d, _| {
        let mut any = false;
        for (id, idx) in ids.iter().zip(&per) {
            let Some(n) = d.node_mut(*id) else { continue };
            let ap = &mut n.appearance;
            let mut idx = idx.clone();
            idx.sort_unstable();
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
    // Where an aligned row would go in each of several different stacks isn't one place.
    if aligned(p) && appearances_differ(&s.doc()?.doc, &ids) {
        return Err(bad(C, "rows of objects whose appearances differ can't be reordered together"));
    }
    let contents = p.get("contents").map(|_| index_param(p, "contents", C)).transpose()?;
    let mut landed = to;
    // When every object's stack comes out as it was, the document and its undo history stay as they
    // are.
    let before = s.doc()?.doc.clone();
    super::overprint::edit_counted(s, "Reorder Appearance", |d| {
        for id in &ids {
            let Some(n) = d.node_mut(*id) else { continue };
            landed = n.appearance.move_item(from, to).ok_or_else(|| bad(C, format!("no item at index {from}")))?;
            if let Some(k) = contents.filter(|_| n.contents_label().is_some()) {
                n.appearance.set_contents_at(k);
            }
        }
        Ok(count_changed(d, &before, &ids))
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
    // When every Contents row is above the bottom `to` items already, the document and its undo
    // history stay as they are.
    let before = s.doc()?.doc.clone();
    let mut slot = None;
    super::overprint::edit_counted(s, "Reorder Appearance", |d| {
        for id in ids {
            let Some(n) = d.node_mut(*id).filter(|n| n.contents_label().is_some()) else { continue };
            n.appearance.set_contents_at(to);
            slot.get_or_insert(n.appearance.contents_at());
        }
        slot.ok_or_else(|| bad("appearance.moveItem", "only groups, layers, type and images have a Contents (Characters) row"))?;
        Ok(count_changed(d, &before, ids))
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

/// Sample enabled attributes, refusing object transparency that a text range cannot store.
fn copy_from(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "appearance.copyFrom";
    let clicked = id_param(p, "source").ok_or_else(|| bad(C, "missing `source` id"))?;
    let (reverse, append) = (switch_or(p, "reverse", false, C)?, switch_or(p, "append", false, C)?);
    let defaults_only = switch_or(p, "defaultsOnly", false, C)?;
    if defaults_only && (reverse || append || p.get("ids").is_some()) {
        return Err(bad(C, "`defaultsOnly` cannot be combined with `ids`, `reverse` or `append`"));
    }
    let attrs = s.prefs.eyedropper.attrs(p).map_err(|e| bad(C, e))?;
    let range = if !defaults_only && !reverse && !append && p.get("ids").is_none() {
        super::textedit::editing_range(s)?.filter(|(_, a, b)| a != b)
    } else {
        None
    };
    let selected = if defaults_only {
        vec![]
    } else {
        match ids_param(p, "ids") {
            Some(v) => outermost(&s.doc()?.doc, v),
            None => selected_roots(s)?,
        }
    };
    // Reversed, the first selected object's attributes go onto the clicked object; with nothing
    // selected, what the Eyedropper picked up last does (#4382).
    let (source, mut targets) = if reverse {
        let source = match leaf_targets(s, &selected)?.into_iter().next() {
            Some(first) => Some(first),
            None if s.eyedropper_sample.is_some() => None,
            None => return Err(bad(C, "select the object to copy from")),
        };
        (source, leaf_targets(s, &[clicked])?)
    } else {
        (Some(clicked), leaf_targets(s, &selected)?)
    };
    targets.retain(|id| Some(*id) != source);
    let node = match source {
        Some(id) => Arc::new(s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?.clone()),
        None => s.eyedropper_sample.clone().ok_or_else(|| bad(C, "select the object to copy from"))?,
    };
    let picked = Picked::of(&node);
    if !reverse && !defaults_only {
        s.eyedropper_sample = Some(node);
    }
    let range = range.filter(|(id, _, _)| targets.contains(id));
    // Character styles cannot represent object transparency. Refuse before even changing
    // paint defaults; applying it to the object would also alter the unselected characters.
    if let Some((id, _, _)) = range.filter(|_| attrs.appearance.transparency) {
        let target = s.doc()?.doc.node(id).ok_or(EngineError::NoNode(id))?;
        if picked.opacity != 1.0 || picked.blend != BlendMode::Normal || target.opacity != 1.0 || target.blend != BlendMode::Normal {
            return Err(bad(
                C,
                "object transparency cannot be sampled onto highlighted characters; disable Transparency in Eyedropper Options or select the whole object",
            ));
        }
    }
    if !reverse {
        picked.to_defaults(s, &attrs.appearance);
    }
    if targets.is_empty() || attrs.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    if let Some((id, start, end)) = range {
        let before = s.doc()?.doc.clone();
        super::overprint::edit_counted(s, "Eyedropper", |d| {
            let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) else { return Err(EngineError::NoNode(id)) };
            let placed = picked.placed(Some(t.local_bounds()));
            vectorcraft_text::edit::style_range(&mut t.runs, start, end, |st| picked.run(st, &placed, &attrs));
            if let Some((_, para)) = picked.text.as_ref().filter(|_| attrs.paragraph) {
                t.edit_paras(Some(t.paragraphs_in(start, end)), |p| *p = para.clone());
            }
            super::typecmd::refresh_bounds(t);
            Ok(count_changed(d, &before, &[id]))
        })?;
        return Ok(json!({ "ids": [id.0] }));
    }
    // When every target comes out as it was, the document and its undo history stay as they are.
    let before = s.doc()?.doc.clone();
    super::overprint::edit_counted(s, "Eyedropper", |d| {
        for id in &targets {
            if let Some(n) = d.node_mut(*id) {
                picked.apply(n, &attrs, append);
            }
        }
        Ok(count_changed(d, &before, &targets))
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
            _ => ((*n.appearance).clone(), n.geometric_bounds(), None),
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

    /// Copy character attributes and the enabled paint attributes through the same path for
    /// whole text objects and highlighted character ranges.
    fn run(&self, st: &mut CharStyle, placed: &Appearance, a: &EyedropperAttrs) {
        if let Some((cs, _)) = self.text.as_ref().filter(|_| a.character) {
            *st = CharStyle {
                fill: std::mem::take(&mut st.fill),
                stroke: std::mem::take(&mut st.stroke),
                stroke_width: st.stroke_width,
                overprint_fill: st.overprint_fill,
                overprint_stroke: st.overprint_stroke,
                ..cs.clone()
            };
        }
        run_attrs(st, placed, &a.appearance);
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
                self.run(&mut r.style, &placed, a);
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
            n.appearance = placed.into();
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
