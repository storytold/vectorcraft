//! Object menu: transforms, arrange, group, lock/hide, compound paths, clipping masks, isolation,
//! align & distribute, object properties.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::corners::set_corners;
use vectorcraft_doc::{Appearance, Document, Knockout, LiveCorners, LiveShape, Node, NodeId, NodeKind, OrientedBox, Scaling};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Affine, FillRule, Point, Rect, Vec2};

use super::edit::{duplicate_in, selected_roots};
use super::opacitymask::percent;
use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.transform",
            "Transform",
            [],
            None,
            "{matrix: [a,b,c,d,e,f], copy?: bool, ids?, strokes?: bool, corners?: bool, typeAreas?: bool, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} apply an affine to the selection (or ids); strokes/corners: Scale Strokes & Effects / Scale Corners (default: the preferences); typeAreas: area type among them (not type inside a group) reshapes its frame by the matrix and its text reflows at its size, as a bounding-box handle drag does (default: the type transforms too)",
            has_doc,
            transform
        ),
        cmd!(
            "object.move",
            "Move…",
            ["Object", "Transform"],
            Some("Cmd+Shift+M"),
            "{dx, dy, copy?, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)}",
            has_selection,
            move_cmd
        ),
        cmd!(
            "object.rotate",
            "Rotate…",
            ["Object", "Transform"],
            None,
            "{angle: deg (counter-clockwise), absolute?: bool (angle is the bounding box's new angle, not an amount), origin?: [x,y] (default: the bounding box centre), copy?, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} → {ids}",
            has_selection,
            rotate
        ),
        cmd!(
            "object.scale",
            "Scale…",
            ["Object", "Transform"],
            None,
            "{sx: %, sy?: %, origin?: [x,y], copy?, strokes?: bool (Scale Strokes & Effects: stroke weights, dashes and effect distances scale; off keeps them, type strokes included), corners?: bool (Scale Corners: live corner radii scale), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} (strokes/corners default to the preferences)",
            has_selection,
            scale
        ),
        cmd!(
            "object.reflect",
            "Reflect…",
            ["Object", "Transform"],
            None,
            "{axis: \"vertical\"|\"horizontal\"|deg, origin?, copy?, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)}",
            has_selection,
            reflect
        ),
        cmd!(
            "object.shear",
            "Shear…",
            ["Object", "Transform"],
            None,
            "{angle: deg, axis?: \"horizontal\"|\"vertical\", origin?, copy?, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)}",
            has_selection,
            shear
        ),
        cmd!("object.transformAgain", "Transform Again", ["Object", "Transform"], Some("Cmd+D"), "{}", has_selection, transform_again),
        cmd!(
            "object.nudge",
            "Nudge",
            [],
            None,
            "{dx: -1|0|1, dy: -1|0|1, big?: bool (×10), copy?: bool} arrow-key nudge by the keyboard increment (the selected anchors, else objects, else ruler guides)",
            has_selection_or_guides,
            nudge
        ),
        cmd!("object.arrange.bringToFront", "Bring to Front", ["Object", "Arrange"], Some("Cmd+Shift+]"), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Front
        )),
        cmd!("object.arrange.bringForward", "Bring Forward", ["Object", "Arrange"], Some("Cmd+]"), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Forward
        )),
        cmd!("object.arrange.sendBackward", "Send Backward", ["Object", "Arrange"], Some("Cmd+["), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Backward
        )),
        cmd!("object.arrange.sendToBack", "Send to Back", ["Object", "Arrange"], Some("Cmd+Shift+["), "{}", has_selection, |s, _| arrange(
            s,
            Arrange::Back
        )),
        cmd!("object.arrange.sendToCurrentLayer", "Send to Current Layer", ["Object", "Arrange"], None, "{}", has_selection, send_to_current_layer),
        cmd!("object.group", "Group", ["Object"], Some("Cmd+G"), "{} → {id}", has_selection, group),
        cmd!("object.ungroup", "Ungroup", ["Object"], Some("Cmd+Shift+G"), "{}", has_selection, ungroup),
        cmd!("object.lock", "Selection", ["Object", "Lock"], Some("Cmd+2"), "{}", has_selection, lock),
        cmd!("object.unlockAll", "Unlock All", ["Object"], Some("Cmd+Alt+2"), "{}", has_doc, unlock_all),
        cmd!("object.hide", "Selection", ["Object", "Hide"], Some("Cmd+3"), "{}", has_selection, hide),
        cmd!("object.showAll", "Show All", ["Object"], Some("Cmd+Alt+3"), "{}", has_doc, show_all),
        cmd!("object.compoundPath.make", "Make", ["Object", "Compound Path"], Some("Cmd+8"), "{}", has_selection, compound_make),
        cmd!("object.compoundPath.release", "Release", ["Object", "Compound Path"], Some("Cmd+Alt+Shift+8"), "{}", has_selection, compound_release),
        cmd!(
            "object.clippingMask.make",
            "Make",
            ["Object", "Clipping Mask"],
            Some("Cmd+7"),
            "{} the topmost selected object (a path, compound path or text, which loses its paint) clips the others: compound holes, even-odd fills and glyph outlines clip as drawn. Paint given to the clipping path later (paint commands with its id) shows: its fill behind the clipped art, its stroke over it → {id} of the clip group",
            has_multi,
            clip_make
        ),
        cmd!(
            "object.clippingMask.release",
            "Release",
            ["Object", "Clipping Mask"],
            Some("Cmd+Alt+7"),
            "{} the selected clip groups become plain groups; their clipping path (path, compound path or text) stays, with the paint it has (none unless painted after Make)",
            has_selection,
            clip_release
        ),
        cmd!(
            "object.clippingMask.editContents",
            "Edit Contents",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipped art of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, false)
        ),
        cmd!(
            "object.clippingMask.editMask",
            "Edit Clipping Path",
            ["Object", "Clipping Mask"],
            None,
            "{} select the clipping paths of the selected clip groups → {count}",
            has_selection,
            |s, _| clip_edit(s, true)
        ),
        cmd!("object.isolate", "Enter Isolation Mode", [], None, "{id}", has_doc, isolate),
        cmd!("object.exitIsolation", "Exit Isolation Mode", [], None, "{}", has_doc, exit_isolation),
        cmd!(
            "object.setProps",
            "Object Properties",
            [],
            None,
            "{ids?|id?, name?, visible?, locked? (visible: false or locked: true deselects every selected object hidden or locked after the call, the key object included), opacity?: 0..100, blend?: \"Multiply\"…, isolate?, knockout?: \"on\"|\"off\"|\"neutral\"|bool (true = on, false = neutral), knockoutShape?: bool, data?: {key: \"value\" | null (removes it)} (the object's own data, SVG data-* attributes: {pivot: \"100,180\"} is data-pivot; document.node → attrs.data)}",
            has_doc,
            set_props
        ),
        cmd!(
            "object.align",
            "Align",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", to?: \"selection\"|\"artboard\"|\"key\" (default: the key object when the selection has one, select.key, else the selection), artboard?: index (0-based; with to: \"artboard\", the artboard to align to; the app passes the active one; default: the artboard under the center of the selection, else the first), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes)}",
            has_selection,
            align
        ),
        cmd!(
            "object.distribute",
            "Distribute",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes)}",
            has_multi,
            distribute
        ),
        cmd!(
            "object.distributeSpacing",
            "Distribute Spacing",
            ["Window", "Align"],
            None,
            "{axis: \"horizontal\"|\"vertical\", spacing?: pt (the key object, select.key, stays put and the others are spaced from it), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes)}",
            has_multi,
            distribute_spacing
        ),
        cmd!(
            "object.setBounds",
            "Set Bounds",
            [],
            None,
            "{x?, y?, width?, height?, reference?: 0..8 (9-point grid), proportional?, strokes?, corners?} (Transform panel; width/height and the reference point follow the bounding box, rotated with rotated objects; x, y are page coordinates; with Use Preview Bounds the values measure the visual bounds; strokes/corners as object.scale)",
            has_selection,
            set_bounds
        ),
        cmd!("object.expandShape", "Expand Shape", ["Object", "Shape"], None, "{} convert live shapes to plain paths", has_selection, expand_shape),
        cmd!(
            "object.setLiveShape",
            "Live Shape Properties",
            [],
            None,
            "{id?, ids?, radius?: pt, kind?: \"round\"|\"invertedRound\"|\"chamfer\", corners?: [i…], items?: [{id, corners?: [i…]}] (several objects, each with its own corners, in place of ids and corners: the canvas's corner widgets across a selection), sides?: n, polygonAngle?: degrees (counterclockwise from its first vertex straight up), polygonRadius?: pt (centre to vertex), sideLength?: pt (sets the radius), makeSidesEqual?: true (a polygon scaled unevenly: drops the uneven scale and shear, keeping its centre, angle and mean radius), pieStart?, pieEnd?: degrees (an ellipse's pie, counterclockwise from 3 o'clock; 0 to 360 is the whole ellipse), invertPie?: true (swaps them: the other part of the ellipse)} (Live Corners on any path: radius and kind set its corners (anchors without handles between two straight sides): `corners` (anchor indices of the path with its corners uncut, counting every subpath's anchors in order: a rectangle's 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left; a polygon's from the first vertex clockwise), else the corners with a Direct-Selected anchor, else every corner; each radius is drawn no larger than half the corner's shorter side allows; a path that isn't a live shape keeps its uncut outline so its corners stay editable, and is plain again once none is cut; sides: a polygon's, which keep the radius they shared; the polygon params apply in that order: sides, makeSidesEqual, polygonAngle, then sideLength or else polygonRadius)",
            has_selection,
            set_live_shape
        ),
    ]
}

fn origin_of(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Point> {
    if let Some(o) = point_param(p, "origin") {
        return Ok(o);
    }
    s.transform_box(ids).map(|b| b.center()).ok_or_else(|| EngineError::Other("selection has no bounds".into()))
}

/// What a transform command scales besides geometry: Scale Strokes & Effects and Scale Corners from
/// its `strokes` and `corners` params, else the preferences. The values join the command's journal
/// entry, so replaying it scales the same way whatever the preferences are then.
pub(crate) fn scaling(s: &mut Session, p: &Value) -> Scaling {
    let strokes = bool_or(p, "strokes", s.prefs.scale_strokes);
    let corners = bool_or(p, "corners", s.prefs.scale_corners);
    s.note_journal("strokes", json!(strokes));
    s.note_journal("corners", json!(corners));
    Scaling {
        strokes,
        effects: strokes.then_some(vectorcraft_render::effects::scale_effect),
        keep_type_strokes: !strokes,
        keep_corners: !corners,
        ..Scaling::default()
    }
}

/// Transform Patterns: do the pattern fills and strokes of `ids` transform with them (`patterns`
/// param, else General › Transform Pattern Tiles)? Noted in the journal when they use a pattern,
/// so a replay moves their tiles alike whatever the preference is then.
pub(crate) fn transform_patterns(s: &mut Session, p: &Value, ids: &[NodeId]) -> Result<bool> {
    let on = bool_or(p, "patterns", s.prefs.transform_pattern_tiles);
    let d = &s.doc()?.doc;
    let mut uses = false;
    for n in ids.iter().filter_map(|id| d.node(*id)) {
        n.walk(&mut |c| uses |= vectorcraft_doc::pattern::uses_pattern(c, None));
    }
    if uses {
        s.note_journal("patterns", json!(on));
    }
    Ok(on)
}

/// Apply `xf` to `ids` (`copy` param: duplicate first; `strokes`/`corners`: see [`scaling`];
/// `patterns`: see [`transform_patterns`]; `typeAreas`: see [`resize_type_area`]). Records
/// Transform Again.
pub(crate) fn apply_transform(s: &mut Session, label: &str, ids: Vec<NodeId>, xf: Affine, p: &Value) -> Result<Value> {
    let copy = bool_or(p, "copy", false);
    let areas = bool_or(p, "typeAreas", false);
    let mut sc = if Scaling::factor(xf).is_some() { scaling(s, p) } else { Scaling::default() };
    sc.patterns = transform_patterns(s, p, &ids)?;
    // A transform of ids that name no object, or an identity transform without a copy, moves
    // nothing: the document and its undo history stay as they are, and Transform Again repeats it.
    // With a copy, the reply lists the copies, and there are none.
    let d = &s.doc()?.doc;
    let none = ids.iter().all(|id| d.node(*id).is_none());
    let ids = if none || (!copy && xf == Affine::IDENTITY) {
        if copy { vec![] } else { ids }
    } else {
        s.edit(label, |d, sel| {
            let targets = if copy { duplicate_in(d, sel, &ids, Affine::IDENTITY)? } else { ids.clone() };
            for id in &targets {
                if let Some(n) = d.node_mut(*id)
                    && !(areas && resize_type_area(n, xf))
                {
                    n.transform(xf, sc);
                }
            }
            Ok(targets)
        })?
    };
    let st = s.doc_mut()?;
    if st.interaction.is_none() {
        st.last_transform = Some((xf, copy));
        st.last_perspective = None;
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Area type resized by its bounding box: `xf` reshapes its frame (the type area) and the text
/// reflows at its size. False for anything else (and type in perspective), which transforms as
/// usual.
fn resize_type_area(n: &mut Node, xf: Affine) -> bool {
    if n.perspective.is_some() {
        return false;
    }
    let NodeKind::Text(t) = &mut n.kind else { return false };
    super::typecmd::reshape_area_with(t, |t| t.transform_area(xf))
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let m = matrix_param(p, "matrix").ok_or_else(|| bad("object.transform", "missing matrix [a,b,c,d,e,f]"))?;
    let ids = match ids_param(p, "ids") {
        Some(v) => v,
        None => selected_roots(s)?,
    };
    apply_transform(s, "Transform", ids, m, p)
}

fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    apply_transform(s, "Move", ids, Affine::translate((f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0))), p)
}

fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let dx = f64_or(p, "dx", 0.0) * k;
    let dy = f64_or(p, "dy", 0.0) * k;
    if !s.doc()?.selection.anchors.is_empty() {
        // When the anchors move by 0, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 {
            return ok();
        }
        return super::path::move_anchors(s, &json!({ "dx": dx, "dy": dy }));
    }
    if s.doc()?.selection.is_empty() {
        let copy = bool_or(p, "copy", false);
        // When guides move by 0 without a copy, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 && !copy {
            return ok();
        }
        return super::docmenu::guide_move(s, &json!({ "dx": dx, "dy": dy, "copy": copy }));
    }
    let ids = selected_roots(s)?;
    apply_transform(s, "Move", ids, Affine::translate((dx, dy)), p)
}

fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

fn rotate(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let mut angle = f64_or(p, "angle", 0.0);
    if bool_or(p, "absolute", false) {
        angle = vectorcraft_geom::normalize_deg(angle - s.doc()?.doc.bbox_angle(&ids));
    }
    // Angles are counter-clockwise; y is down, so negate.
    let a = about(o, Affine::rotate(-angle.to_radians()));
    apply_transform(s, "Rotate", ids, a, p)
}

fn scale(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let sx = f64_req(p, "sx", "object.scale")? / 100.0;
    let sy = f64_or(p, "sy", sx * 100.0) / 100.0;
    if sx == 0.0 || sy == 0.0 {
        return Err(bad("object.scale", "scale must be non-zero"));
    }
    apply_transform(s, "Scale", ids, about(o, Affine::scale_non_uniform(sx, sy)), p)
}

fn reflect(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let m = match p.get("axis") {
        Some(Value::String(a)) if a == "horizontal" => Affine::scale_non_uniform(1.0, -1.0),
        Some(Value::Number(n)) => {
            let t = -n.as_f64().unwrap_or(90.0).to_radians();
            Affine::rotate(t) * Affine::scale_non_uniform(1.0, -1.0) * Affine::rotate(-t)
        }
        None | Some(Value::Null) => Affine::scale_non_uniform(-1.0, 1.0),
        Some(Value::String(a)) if a == "vertical" => Affine::scale_non_uniform(-1.0, 1.0),
        Some(v) => {
            let given = v.as_str().map_or_else(|| v.to_string(), str::to_string);
            return Err(bad("object.reflect", format!("axis must be \"vertical\", \"horizontal\" or an angle in degrees, not `{given}`")));
        }
    };
    apply_transform(s, "Reflect", ids, about(o, m), p)
}

fn shear(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let o = origin_of(s, p, &ids)?;
    let t = f64_or(p, "angle", 0.0).clamp(-89.0, 89.0).to_radians().tan();
    let vertical = choice(p, "object.shear", "axis", &["horizontal", "vertical"])? == Some("vertical");
    let m = if vertical { Affine::new([1.0, t, 0.0, 1.0, 0.0, 0.0]) } else { Affine::new([1.0, 0.0, -t, 1.0, 0.0, 0.0]) };
    apply_transform(s, "Shear", ids, about(o, m), p)
}

fn transform_again(s: &mut Session, _: &Value) -> Result<Value> {
    if let Some(again) = s.doc()?.last_perspective.clone() {
        return super::distortcmds::transform_again(s, &again);
    }
    let (m, copy) = s.doc()?.last_transform.ok_or_else(|| EngineError::Other("no previous transform".into()))?;
    let ids = selected_roots(s)?;
    apply_transform(s, "Transform Again", ids, m, &json!({ "copy": copy }))
}

enum Arrange {
    Front,
    Forward,
    Backward,
    Back,
}

fn arrange(s: &mut Session, how: Arrange) -> Result<Value> {
    let mut ids = selected_roots(s)?;
    if matches!(how, Arrange::Forward | Arrange::Front) {
        ids.reverse();
    }
    let label = match how {
        Arrange::Front => "Bring to Front",
        Arrange::Forward => "Bring Forward",
        Arrange::Backward => "Send Backward",
        Arrange::Back => "Send to Back",
    };
    // When nothing changes order, the document and its undo history stay as they are.
    super::overprint::edit_counted(s, label, |d| {
        let mut moved = 0;
        // Arrange within each parent independently (selections may span layers/groups).
        let mut by_parent: Vec<(Option<vectorcraft_doc::NodeId>, Vec<vectorcraft_doc::NodeId>)> = vec![];
        for id in &ids {
            let par = d.parent_of(*id);
            match by_parent.iter_mut().find(|(p, _)| *p == par) {
                Some((_, v)) => v.push(*id),
                None => by_parent.push((par, vec![*id])),
            }
        }
        for (par, group) in by_parent {
            for (k, id) in group.iter().enumerate() {
                let Some((_, idx, len)) = d.position(*id) else { continue };
                if len == 0 {
                    continue;
                }
                let last = len - 1;
                let to = match how {
                    Arrange::Front => last.saturating_sub(k),
                    Arrange::Back => k.min(last),
                    Arrange::Forward => (idx + 1).min(last.saturating_sub(k)).max(idx),
                    Arrange::Backward => idx.saturating_sub(1).max(k.min(idx)),
                };
                if to != idx {
                    d.move_node(*id, par, to)?;
                    moved += 1;
                }
            }
        }
        Ok(moved)
    })?;
    ok()
}

fn send_to_current_layer(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // Ids are reused after undo: the remembered current layer must still be a layer.
    let st = s.doc()?;
    let layer = st.current_layer().or_else(|| st.doc.default_layer()).ok_or_else(|| EngineError::Other("no current layer".into()))?;
    // When the selected objects are already the topmost in the current layer, in paint order, the
    // document and its undo history stay as they are.
    if st.doc.children(Some(layer)).is_some_and(|c| c.iter().rev().map(|n| n.id).take(ids.len()).eq(ids.iter().rev().copied())) {
        return ok();
    }
    s.edit("Send to Current Layer", |d, _| {
        for id in &ids {
            d.move_node(*id, Some(layer), usize::MAX)?;
        }
        Ok(())
    })?;
    ok()
}

fn group(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let gid = s.edit("Group", |d, sel| {
        let gid = group_nodes(d, &ids)?;
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

/// Put `ids` (top-level objects, in paint order) in a new group where the front-most one was.
pub(crate) fn group_nodes(d: &mut Document, ids: &[NodeId]) -> Result<NodeId> {
    let Some(top) = ids.last().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
    let gid = d.alloc_id();
    d.insert(par, idx + 1, Node::group(gid, vec![]))?;
    for id in ids {
        d.move_node(*id, Some(gid), usize::MAX)?;
    }
    Ok(gid)
}

fn ungroup(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no group selected, the document, the selection and the undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. }))) {
        return ok();
    }
    s.edit("Ungroup", |d, sel| {
        let mut new_sel = vec![];
        for id in &ids {
            let Some(n) = d.node(*id) else { continue };
            let NodeKind::Group { children, clip } = &n.kind else {
                new_sel.push(*id);
                continue;
            };
            let children = children.clone();
            let clip = *clip;
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            let opacity = n.opacity;
            d.remove(*id)?;
            for (k, c) in children.into_iter().enumerate() {
                let mut c = (*c).clone();
                // The clipping path keeps its paint (none, unless painted after Make).
                if clip
                    && k == 0
                    && let NodeKind::Path { clipping, .. } = &mut c.kind
                {
                    *clipping = false;
                }
                c.opacity *= opacity;
                new_sel.push(c.id);
                d.insert(par, idx + k, c)?;
            }
        }
        sel.set(new_sel);
        Ok(())
    })?;
    ok()
}

fn lock(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With every selected object already locked, the document and its undo history stay as they
    // are, and the objects are deselected.
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_none_or(|n| n.locked)) {
        s.select(|_, sel| sel.clear())?;
        return ok();
    }
    s.edit("Lock", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.locked = true;
            }
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

fn collect_flag(d: &Document, f: impl Fn(&Node) -> bool) -> Vec<NodeId> {
    let mut v = vec![];
    d.walk(|n| {
        if !n.is_layer() && f(n) {
            v.push(n.id)
        }
    });
    v
}

fn unlock_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| n.locked);
    // With nothing locked, the document, the selection and the undo history stay as they are.
    if ids.is_empty() {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Unlock All", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.locked = false;
            }
        }
        sel.set(ids.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "count": ids.len() }))
}

fn hide(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With every selected object already hidden, the document and its undo history stay as they
    // are, and the objects are deselected.
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_none_or(|n| !n.visible)) {
        s.select(|_, sel| sel.clear())?;
        return ok();
    }
    s.edit("Hide", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.visible = false;
            }
        }
        sel.clear();
        Ok(())
    })?;
    ok()
}

fn show_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = collect_flag(&s.doc()?.doc, |n| !n.visible);
    // With nothing hidden, the document, the selection and the undo history stay as they are.
    if ids.is_empty() {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Show All", |d, sel| {
        for id in &ids {
            if let Some(n) = d.node_mut(*id) {
                n.visible = true;
            }
        }
        sel.set(ids.iter().copied());
        Ok(())
    })?;
    Ok(json!({ "count": ids.len() }))
}

fn compound_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(top) = ids.last().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let id = s.edit("Make Compound Path", |d, sel| {
        let appearance = d.node(top).map(|n| n.appearance.clone()).unwrap_or_default();
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let mut paths: Vec<Arc<Node>> = vec![];
        for id in &ids {
            let n = d.node(*id).cloned().ok_or(EngineError::NoNode(*id))?;
            match &n.kind {
                NodeKind::Path { .. } => paths.push(Arc::new(n)),
                NodeKind::Compound { children, .. } => paths.extend(children.iter().cloned()),
                NodeKind::Group { .. } => {
                    n.walk(&mut |c| {
                        if matches!(c.kind, NodeKind::Path { .. }) {
                            paths.push(Arc::new(c.clone()))
                        }
                    });
                }
                _ => return Err(EngineError::Other("compound paths can only contain paths".into())),
            }
        }
        let cid = d.alloc_id();
        let mut c = Node::new(cid, NodeKind::Compound { children: vec![], rule: FillRule::NonZero });
        c.appearance = appearance;
        d.insert(par, idx + 1, c)?;
        for id in &ids {
            d.remove(*id)?;
        }
        let ch = d.node_mut(cid).and_then(|n| n.children_mut()).ok_or(EngineError::NoNode(cid))?;
        for mut p in paths {
            let pn = Arc::make_mut(&mut p);
            pn.appearance = Appearance::default();
            if let NodeKind::Path { live, .. } = &mut pn.kind {
                *live = None;
            }
            ch.push(p);
        }
        sel.set([cid]);
        Ok(cid)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn compound_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no compound path selected, the document, the selection and the undo history stay as
    // they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))) {
        return ok();
    }
    s.edit("Release Compound Path", |d, sel| {
        let mut out = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned() else { continue };
            let NodeKind::Compound { children, .. } = &n.kind else { continue };
            let (par, idx, _) = d.position(*id).ok_or(EngineError::NoNode(*id))?;
            d.remove(*id)?;
            for (k, c) in children.iter().enumerate() {
                let mut c = (**c).clone();
                c.appearance = n.appearance.clone();
                out.push(c.id);
                d.insert(par, idx + k, c)?;
            }
        }
        sel.set(out);
        Ok(())
    })?;
    ok()
}

/// Make `top` (a path, compound path or text object) the clipping path: it loses its paint (it
/// stays unpainted after Release).
pub(super) fn make_clipping_path(d: &mut Document, top: NodeId) -> Result<()> {
    as_clipping_path(d.node_mut(top).ok_or(EngineError::NoNode(top))?)
}

/// Turn path, compound path or text `c` into a clipping path (its paint goes).
pub(super) fn as_clipping_path(c: &mut Node) -> Result<()> {
    if !matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
        return Err(EngineError::Other("the top object must be a path, compound path or text object to use as a clipping mask".into()));
    }
    c.appearance = Appearance::basic(Paint::None, Paint::None, 0.0);
    match &mut c.kind {
        NodeKind::Path { clipping, .. } => *clipping = true,
        NodeKind::Text(t) => {
            for r in &mut t.runs {
                (r.style.fill, r.style.stroke) = (Paint::None, Paint::None);
            }
        }
        _ => {}
    }
    Ok(())
}

/// Stop clip group or clipped layer `id` clipping; its clipping path stays, with its paint.
pub(super) fn release_clip(d: &mut Document, id: NodeId) {
    let Some(n) = d.node_mut(id).filter(|n| n.clips()) else { return };
    n.set_clips(false);
    let clip = n.children().and_then(|c| c.first()).map(|c| c.id);
    if let Some(c) = clip.and_then(|c| d.node_mut(c))
        && let NodeKind::Path { clipping, .. } = &mut c.kind
    {
        *clipping = false;
    }
}

fn clip_make(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let Some(&top) = ids.last() else { return Err(EngineError::Other("nothing selected".into())) };
    let gid = s.edit("Make Clipping Mask", |d, sel| {
        make_clipping_path(d, top)?;
        let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
        let gid = d.alloc_id();
        d.insert(par, idx + 1, Node::new(gid, NodeKind::Group { children: vec![], clip: true }))?;
        d.move_node(top, Some(gid), 0)?;
        for id in &ids[..ids.len() - 1] {
            d.move_node(*id, Some(gid), usize::MAX)?;
        }
        sel.set([gid]);
        Ok(gid)
    })?;
    Ok(json!({ "id": gid.0 }))
}

fn clip_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no clip group selected, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(Node::clips)) {
        return ok();
    }
    s.edit("Release Clipping Mask", |d, _| {
        for id in &ids {
            release_clip(d, *id);
        }
        Ok(())
    })?;
    ok()
}

/// Selects the clipping path (`mask`) or the clipped contents of every selected clip group (or of the
/// clip group containing a selected object), like Illustrator's Edit Contents / Edit Clipping Path toggle.
fn clip_edit(s: &mut Session, mask: bool) -> Result<Value> {
    let st = s.doc()?;
    let mut ids = vec![];
    for id in st.selection.objects.iter().copied() {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if let Some(NodeKind::Group { children, clip: true }) = st.doc.node(c).map(|n| &n.kind) {
                if mask {
                    ids.extend(children.first().map(|c| c.id));
                } else {
                    ids.extend(children.iter().skip(1).map(|c| c.id));
                }
                break;
            }
            cur = st.doc.position(c).and_then(|(par, _, _)| par);
        }
    }
    ids.dedup();
    if ids.is_empty() {
        return Err(EngineError::Other("select a clip group".into()));
    }
    s.select(|_, sel| sel.set(ids.iter().copied()))?;
    Ok(json!({ "count": ids.len() }))
}

fn isolate(s: &mut Session, p: &Value) -> Result<Value> {
    let id = id_param(p, "id")
        .or_else(|| s.active().and_then(|d| d.selection.objects.first().copied()))
        .ok_or_else(|| bad("object.isolate", "missing id"))?;
    let st = s.doc_mut()?;
    if st.doc.node(id).is_none_or(|n| !n.is_container()) {
        return Err(bad("object.isolate", "only groups and layers can be isolated"));
    }
    let isolation = st.doc.node(id).filter(|n| n.shaper.is_some()).and_then(|n| n.children()).and_then(|c| c.first()).map_or(id, |n| n.id);
    st.isolation = Some(isolation);
    st.selection.clear();
    st.revision += 1;
    ok()
}

fn exit_isolation(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc_mut()?;
    if let Some(i) = st.isolation.take() {
        super::distortcmds::finish_edit_text(st, i);
        let target = st.doc.parent_of(i).filter(|p| st.doc.node(*p).is_some_and(|n| n.shaper.is_some())).unwrap_or(i);
        st.selection.set([target]);
    }
    st.revision += 1;
    ok()
}

/// The most data entries `object.setProps` takes at once.
const MAX_DATA: usize = 256;

/// `object.setProps`'s `data`: each key (a data-* attribute's name: letters, digits, `-`, `_`,
/// `.`) with its value, or `None` to remove it.
fn data_param(m: &serde_json::Map<String, Value>) -> Result<Vec<(String, Option<String>)>> {
    const C: &str = "object.setProps";
    if m.len() > MAX_DATA {
        return Err(bad(C, format!("data takes at most {MAX_DATA} keys at once")));
    }
    m.iter()
        .map(|(k, v)| {
            let k = k.strip_prefix("data-").unwrap_or(k);
            if k.is_empty() || !k.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')) || k == "name" || k.starts_with("vc-") {
                return Err(bad(C, format!("data key `{k}`: letters, digits, -, _ and . (not name or vc-…)")));
            }
            let v = match v {
                Value::Null => None,
                Value::String(s) => Some(s.clone()),
                other => Some(other.to_string()),
            };
            Ok((k.to_string(), v))
        })
        .collect()
}

fn set_props(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let opacity = p.get("opacity").and_then(Value::as_f64).map(percent);
    let blend = match str_param(p, "blend") {
        Some(b) => Some(BlendMode::parse(b).ok_or_else(|| bad("object.setProps", format!("unknown blend mode `{b}`")))?),
        None => None,
    };
    let data = match p.get("data") {
        None | Some(Value::Null) => None,
        Some(Value::Object(m)) => Some(data_param(m)?),
        Some(v) => return Err(bad("object.setProps", format!("data must be an object of key: value, not {v}"))),
    };
    let knockout = match p.get("knockout") {
        Some(v) => Some(
            Knockout::from_value(v)
                .ok_or_else(|| bad("object.setProps", format!("knockout must be \"on\", \"off\", \"neutral\" or a bool, not {v}")))?,
        ),
        None => None,
    };
    let hides = p.get("visible").and_then(Value::as_bool) == Some(false) || p.get("locked").and_then(Value::as_bool) == Some(true);
    // When every object already has every value given, the document and its undo history stay as
    // they are.
    let flag = |k: &str| p.get(k).and_then(Value::as_bool);
    let has_values = |n: &Node| {
        str_param(p, "name").is_none_or(|v| n.name.as_deref() == Some(v).filter(|v| !v.is_empty()))
            && flag("visible").is_none_or(|v| n.visible == v)
            && flag("locked").is_none_or(|v| n.locked == v)
            && opacity.is_none_or(|v| n.opacity == v)
            && blend.is_none_or(|b| n.blend == b)
            && flag("isolate").is_none_or(|v| n.isolate == v)
            && knockout.is_none_or(|k| n.knockout == k)
            && flag("knockoutShape").is_none_or(|v| n.knockout_shape == v)
            && data
                .iter()
                .flatten()
                .all(|(k, v)| n.attrs.as_ref().and_then(|a| a.data.iter().find(|(key, _)| key == k)).map(|(_, now)| now.as_str()) == v.as_deref())
    };
    let d = &s.doc()?.doc;
    if ids.iter().all(|id| d.node(*id).is_some_and(has_values)) {
        // Hidden or locked objects can't stay selected.
        if hides {
            s.select(|d, sel| sel.deselect_uneditable(d))?;
        }
        return ok();
    }
    s.edit("Object Properties", |d, sel| {
        for id in &ids {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            if let Some(v) = str_param(p, "name") {
                n.name = if v.is_empty() { None } else { Some(v.to_string()) };
            }
            if let Some(v) = p.get("visible").and_then(Value::as_bool) {
                n.visible = v;
            }
            if let Some(v) = p.get("locked").and_then(Value::as_bool) {
                n.locked = v;
            }
            if let Some(v) = opacity {
                n.opacity = v;
            }
            if let Some(b) = blend {
                n.blend = b;
            }
            if let Some(v) = p.get("isolate").and_then(Value::as_bool) {
                n.isolate = v;
            }
            if let Some(k) = knockout {
                n.knockout = k;
            }
            if let Some(v) = p.get("knockoutShape").and_then(Value::as_bool) {
                n.knockout_shape = v;
            }
            if let Some(changes) = &data {
                n.edit_attrs(|a| {
                    for (k, v) in changes {
                        match (a.data.iter().position(|(key, _)| key == k), v) {
                            (Some(i), Some(v)) => {
                                if let Some(e) = a.data.get_mut(i) {
                                    e.1 = v.clone();
                                }
                            }
                            (Some(i), None) => {
                                a.data.remove(i);
                            }
                            (None, Some(v)) => a.data.push((k.clone(), v.clone())),
                            (None, None) => {}
                        }
                    }
                });
            }
        }
        // Hidden or locked objects can't stay selected.
        if hides {
            sel.deselect_uneditable(d);
        }
        Ok(())
    })?;
    ok()
}

impl Session {
    /// The bounds of `ids` the Transform panel, the bounding box, Align and Distribute measure:
    /// visual bounds (strokes included) with Use Preview Bounds on, else geometric bounds.
    pub fn transform_bounds(&self, ids: &[NodeId]) -> Option<Rect> {
        self.doc().ok()?.doc.bounds_of(ids, self.prefs.use_preview_bounds)
    }
    /// [`Session::transform_bounds`] square to the objects' own angle (their rotated bounding
    /// box, see [`Document::oriented_bounds`]).
    pub fn transform_box(&self, ids: &[NodeId]) -> Option<OrientedBox> {
        self.doc().ok()?.doc.oriented_bounds(ids, self.prefs.use_preview_bounds)
    }
}

/// Measure with preview (visual) bounds? The `bounds` param, else Use Preview Bounds.
fn preview_bounds(s: &Session, p: &Value, cmd: &str) -> Result<bool> {
    match choice(p, cmd, "bounds", &["preview", "geometric"])? {
        None => Ok(s.prefs.use_preview_bounds),
        Some(b) => Ok(b == "preview"),
    }
}

/// Param `key` of the Align commands and Shear: one of `values`, or `None` when it is missing or
/// null. Any other value is an error that lists `values`.
fn choice<'a>(p: &'a Value, cmd: &str, key: &str, values: &[&str]) -> Result<Option<&'a str>> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    match v.as_str() {
        Some(s) if values.contains(&s) => Ok(Some(s)),
        _ => Err(bad(cmd, format!("`{key}` must be one of {}", values.join(", ")))),
    }
}

/// `horizontal` and `vertical` of `object.align` and `object.distribute`, each checked by [`choice`].
fn direction<'a>(p: &'a Value, cmd: &str) -> Result<(Option<&'a str>, Option<&'a str>)> {
    Ok((choice(p, cmd, "horizontal", &["left", "center", "right"])?, choice(p, cmd, "vertical", &["top", "center", "bottom"])?))
}

/// `ids` with their bounds (`preview`: visual bounds).
fn items_bounds(d: &Document, ids: &[NodeId], preview: bool) -> Vec<(NodeId, Rect)> {
    ids.iter().filter_map(|id| Some((*id, d.bounds_of(&[*id], preview)?))).collect()
}

/// Is every move in `moves` a rounding error (1e-9 pt or less on both axes)?
fn moves_nothing(moves: &[(NodeId, Vec2)]) -> bool {
    moves.iter().all(|(_, dv)| dv.x.abs() <= 1e-9 && dv.y.abs() <= 1e-9)
}

/// What `object.align` aligns to: `to`, else the key object when there is one, else the
/// selection.
fn align_to<'a>(s: &Session, p: &'a Value) -> Option<&'a str> {
    str_param(p, "to").or_else(|| s.doc().ok()?.selection.key.map(|_| "key"))
}

fn reference_rect(s: &Session, p: &Value, ids: &[NodeId], preview: bool) -> Result<Rect> {
    let st = s.doc()?;
    match align_to(s, p) {
        Some("artboard") => {
            let b = st.doc.bounds_of(ids, preview).unwrap_or_default();
            // Align to Artboard uses artboard `artboard` (the app passes the active one). Without
            // it, or with an index the document doesn't have, it uses the artboard under the center
            // of the selection, or the first one when no artboard is there.
            let named = p.get("artboard").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).and_then(|i| st.doc.artboards.get(i));
            let i = st.doc.artboard_at(b.center()).unwrap_or(0);
            Ok(named.or(st.doc.artboards.get(i)).map(|a| a.rect).unwrap_or(b))
        }
        Some("key") => {
            let k = st.selection.key.ok_or_else(|| EngineError::Other("no key object".into()))?;
            st.doc.bounds_of(&[k], preview).ok_or(EngineError::NoNode(k))
        }
        _ => st.doc.bounds_of(ids, preview).ok_or_else(|| EngineError::Other("nothing to align".into())),
    }
}

/// Returns the selected root (one of `ids`) that is the key object or contains it, such as the
/// compound path or selected group the key is in. Align to the key object and Distribute Spacing
/// with a spacing keep it in place.
fn key_root(s: &Session, ids: &[NodeId]) -> Option<NodeId> {
    let st = s.doc().ok()?;
    let ancestry = st.doc.ancestry(st.selection.key?)?;
    ids.iter().copied().find(|id| ancestry.contains(id))
}

fn align(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let preview = preview_bounds(s, p, "object.align")?;
    let (h, v) = direction(p, "object.align")?;
    choice(p, "object.align", "to", &["selection", "artboard", "key"])?;
    if h.is_none() && v.is_none() {
        return Err(bad("object.align", "give horizontal or vertical"));
    }
    let r = reference_rect(s, p, &ids, preview)?;
    let key = key_root(s, &ids);
    let moves: Vec<(NodeId, Vec2)> = {
        let d = &s.doc()?.doc;
        items_bounds(d, &ids, preview)
            .into_iter()
            .filter(|(id, _)| Some(*id) != key || align_to(s, p) != Some("key"))
            .map(|(id, b)| {
                let dx = match h {
                    Some("left") => r.x0 - b.x0,
                    Some("center") => r.center().x - b.center().x,
                    Some("right") => r.x1 - b.x1,
                    _ => 0.0,
                };
                let dy = match v {
                    Some("top") => r.y0 - b.y0,
                    Some("center") => r.center().y - b.center().y,
                    Some("bottom") => r.y1 - b.y1,
                    _ => 0.0,
                };
                (id, Vec2::new(dx, dy))
            })
            .collect()
    };
    // When nothing moves, the document and its undo history stay as they are.
    if moves_nothing(&moves) {
        return ok();
    }
    s.edit("Align", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn distribute(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let preview = preview_bounds(s, p, "object.distribute")?;
    let mut items = items_bounds(&s.doc()?.doc, &ids, preview);
    let (horiz, key): (bool, fn(&Rect, bool) -> f64) = match direction(p, "object.distribute")? {
        (Some(h), _) => (
            true,
            match h {
                "left" => |r: &Rect, _| r.x0,
                "right" => |r: &Rect, _| r.x1,
                _ => |r: &Rect, _| r.center().x,
            },
        ),
        (_, Some(v)) => (
            false,
            match v {
                "top" => |r: &Rect, _| r.y0,
                "bottom" => |r: &Rect, _| r.y1,
                _ => |r: &Rect, _| r.center().y,
            },
        ),
        _ => return Err(bad("object.distribute", "give horizontal or vertical")),
    };
    items.sort_by(|a, b| key(&a.1, horiz).total_cmp(&key(&b.1, horiz)));
    let n = items.len();
    if n < 3 {
        return ok();
    }
    let first = key(&items[0].1, horiz);
    let last = key(&items[n - 1].1, horiz);
    let moves: Vec<(NodeId, Vec2)> = items
        .iter()
        .enumerate()
        .map(|(i, (id, r))| {
            let target = first + (last - first) * i as f64 / (n - 1) as f64;
            let delta = target - key(r, horiz);
            (*id, if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) })
        })
        .collect();
    // When nothing moves, the document and its undo history stay as they are.
    if moves_nothing(&moves) {
        return ok();
    }
    s.edit("Distribute", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn distribute_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let horiz = choice(p, "object.distributeSpacing", "axis", &["horizontal", "vertical"])? != Some("vertical");
    let preview = preview_bounds(s, p, "object.distributeSpacing")?;
    let mut items = items_bounds(&s.doc()?.doc, &ids, preview);
    items.sort_by(|a, b| if horiz { a.1.x0.total_cmp(&b.1.x0) } else { a.1.y0.total_cmp(&b.1.y0) });
    let n = items.len();
    let &[(_, first), .., (_, last)] = items.as_slice() else {
        return Err(EngineError::Other("select two or more objects to distribute".into()));
    };
    let size = |r: &Rect| if horiz { r.width() } else { r.height() };
    let start = |r: &Rect| if horiz { r.x0 } else { r.y0 };
    let spacing = p
        .get("spacing")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().filter(|g| g.is_finite()).ok_or_else(|| bad("object.distributeSpacing", "`spacing` must be a number")))
        .transpose()?;
    let gap = match spacing {
        Some(g) => g,
        None => {
            let span = if horiz { last.x1 - first.x0 } else { last.y1 - first.y0 };
            (span - items.iter().map(|i| size(&i.1)).sum::<f64>()) / (n - 1) as f64
        }
    };
    let mut pos = start(&first);
    let mut deltas = vec![];
    for (id, r) in &items {
        deltas.push((*id, pos - start(r)));
        pos += size(r) + gap;
    }
    // With a spacing (aligning to a key object) the key object stays where it is, with the
    // compound path or selected group it is in: the others are spaced from it.
    let key = key_root(s, &ids);
    let fixed = spacing.and_then(|_| deltas.iter().find(|(id, _)| Some(*id) == key)).map_or(0.0, |k| k.1);
    let moves: Vec<(NodeId, Vec2)> =
        deltas.into_iter().map(|(id, d)| (id, if horiz { Vec2::new(d - fixed, 0.0) } else { Vec2::new(0.0, d - fixed) })).collect();
    // When nothing moves, the document and its undo history stay as they are.
    if moves_nothing(&moves) {
        return ok();
    }
    s.edit("Distribute Spacing", |d, _| {
        for (id, dv) in &moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), false);
            }
        }
        Ok(())
    })?;
    ok()
}

fn set_bounds(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let d = &s.doc()?.doc;
    // Sizes are measured square to the bounding box, which a rotation turns with the objects:
    // the math below runs in its frame.
    let frame = d.oriented_bounds(&ids, false).ok_or_else(|| EngineError::Other("no bounds".into()))?;
    let g = frame.rect;
    // With Use Preview Bounds the values measure the visual box, whose margins around the
    // geometric box (left, top, right, bottom) scale with the strokes or stay.
    let v = if s.prefs.use_preview_bounds { d.oriented_bounds(&ids, true).map_or(g, |b| b.rect) } else { g };
    let m = [g.x0 - v.x0, g.y0 - v.y0, v.x1 - g.x1, v.y1 - g.y1];
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4) as usize;
    let rp = vectorcraft_geom::reference_point(v, refi);
    let mut w = f64_or(p, "width", v.width());
    let mut h = f64_or(p, "height", v.height());
    if bool_or(p, "proportional", false) {
        if p.get("width").is_some() && v.width() > 0.0 {
            h = v.height() * w / v.width();
        } else if p.get("height").is_some() && v.height() > 0.0 {
            w = v.width() * h / v.height();
        }
    }
    // The geometric scale that gives the visual size: margins grow by the strokes' scale `f`
    // (exact for proportional scales, which scale the visual box alike).
    let strokes = bool_or(p, "strokes", s.prefs.scale_strokes);
    let ratio = |size: f64, geo: f64, margin: f64| if geo > 1e-9 { (size - margin) / geo } else { 1.0 };
    let f = |sx: f64, sy: f64| if strokes { (sx * sy).abs().sqrt() } else { 1.0 };
    let (lx, ly) = (ratio(w, v.width(), 0.0), ratio(h, v.height(), 0.0));
    let f0 = f(if g.width() > 1e-9 { lx } else { 1.0 }, if g.height() > 1e-9 { ly } else { 1.0 });
    let sx = ratio(w, g.width(), (m[0] + m[2]) * f0);
    let sy = ratio(h, g.height(), (m[1] + m[3]) * f0);
    let scale = about(rp, Affine::scale_non_uniform(sx, sy));
    // Move the new visual box's reference point to x, y (default: where it was): the geometric
    // box's reference point moves with the art, the margin from it to the visual one by `f`.
    let gp = vectorcraft_geom::reference_point(g, refi);
    let new_rp = scale * gp + (rp - gp) * f(sx, sy);
    // x, y place the reference point on the page.
    let page = frame.to_doc() * rp;
    // When every value given is the current one, the transform is the identity. Computing it can
    // leave a rounding error in it on a rotated object or with Use Preview Bounds.
    let current = |k: &str, now: f64| p.get(k).and_then(Value::as_f64).is_none_or(|x| x == now);
    if current("x", page.x) && current("y", page.y) && current("width", v.width()) && current("height", v.height()) {
        return apply_transform(s, "Transform", ids, Affine::IDENTITY, p);
    }
    let to = frame.to_local(Point::new(f64_or(p, "x", page.x), f64_or(p, "y", page.y)));
    apply_transform(s, "Transform", ids, frame.conjugate(Affine::translate(to - new_rp) * scale), p)
}

fn expand_shape(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no live shape selected, the document and its undo history stay as they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { live: Some(_), .. }))) {
        return ok();
    }
    s.edit("Expand Shape", |d, _| {
        for id in &ids {
            if let Some(NodeKind::Path { live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) {
                *live = None;
            }
        }
        Ok(())
    })?;
    ok()
}

/// The `corners` of `object.setLiveShape`: anchor indices of a path's uncut outline.
fn corners_param(p: &Value) -> Result<Option<BTreeSet<usize>>> {
    let Some(v) = p.get("corners").filter(|v| !v.is_null()) else { return Ok(None) };
    let err = || bad("object.setLiveShape", format!("`corners` must list anchor indices (0, 1, 2…), not {v}"));
    let list = v.as_array().ok_or_else(err)?;
    list.iter().map(|k| k.as_u64().and_then(|k| usize::try_from(k).ok()).ok_or_else(err)).collect::<Result<_>>().map(Some)
}

fn set_live_shape(s: &mut Session, p: &Value) -> Result<Value> {
    // `items`: objects each with its own `corners`; else `ids` (or the selection) sharing them.
    let items = match p.get("items").filter(|v| !v.is_null()) {
        None => None,
        Some(Value::Array(a)) => Some(
            a.iter()
                .map(|it| {
                    let id = it
                        .get("id")
                        .and_then(Value::as_u64)
                        .map(NodeId)
                        .ok_or_else(|| bad("object.setLiveShape", format!("each of `items` is {{id, corners?}} with an object id, not {it}")))?;
                    Ok((id, corners_param(it)?))
                })
                .collect::<Result<Vec<_>>>()?,
        ),
        Some(v) => return Err(bad("object.setLiveShape", format!("`items` must be an array of {{id, corners?}}, not {v}"))),
    };
    let ids = match &items {
        Some(items) => items.iter().map(|(id, _)| *id).collect(),
        None => targets(s, p)?,
    };
    let shared = corners_param(p)?;
    let corners_of = |id: NodeId| match &items {
        Some(items) => items.iter().find(|(i, _)| *i == id).and_then(|(_, c)| c.clone()),
        None => shared.clone(),
    };
    let kind = p
        .get("kind")
        .filter(|v| !v.is_null())
        .map(|v| {
            serde_json::from_value::<CornerKind>(v.clone())
                .map_err(|_| bad("object.setLiveShape", format!("`kind` must be \"round\", \"invertedRound\" or \"chamfer\", not {v}")))
        })
        .transpose()?;
    let radius = p.get("radius").and_then(Value::as_f64);
    let sides = p.get("sides").and_then(Value::as_u64);
    // A number that must be `what` (None when absent).
    let number = |k: &str, ok: fn(f64) -> bool, what: &str| -> Result<Option<f64>> {
        match p.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite() && ok(*x))
                .map(Some)
                .ok_or_else(|| bad("object.setLiveShape", format!("`{k}` must be {what}, not {v}"))),
        }
    };
    let angle = |k: &str| Ok::<_, EngineError>(number(k, |_| true, "an angle in degrees")?.map(|a| a.rem_euclid(360.0)));
    let length = |k: &str| number(k, |x| x > 0.0 && x <= crate::MAX_COORD, "a length in points above 0");
    // An ellipse's pie (Ellipse Properties: Pie Start and End Angle, degrees; Invert Pie swaps them).
    let (pie_start, pie_end) = (angle("pieStart")?, angle("pieEnd")?);
    let invert = p.get("invertPie").and_then(Value::as_bool) == Some(true);
    // A polygon's Polygon Properties: its angle, radius or side length, Make Sides Equal.
    let (polygon_angle, polygon_radius, side) = (angle("polygonAngle")?, length("polygonRadius")?, length("sideLength")?);
    let equal = p.get("makeSidesEqual").and_then(Value::as_bool) == Some(true);
    // An ellipse's pie becomes `pie_of(pie)`. An end of 0 is a full turn (Illustrator shows a whole
    // ellipse as 0° to 360°).
    let end_of = |a: f64| if a == 0.0 { 360.0 } else { a };
    let pie_of = |pie: (f64, f64)| {
        let pie = (pie_start.unwrap_or(pie.0), pie_end.map_or(pie.1, end_of));
        if invert { (pie.1.rem_euclid(360.0), end_of(pie.0)) } else { pie }
    };
    // When every value given is the current one, the document and its undo history stay as they
    // are.
    let st = s.doc()?;
    let has_values = |id: &NodeId| {
        let Some(NodeKind::Path { path, live, .. }) = st.doc.node(*id).map(|n| &n.kind) else { return true };
        match live {
            Some(l @ LiveShape::Polygon { sides: now, .. })
                if !(sides.is_none_or(|n| n.clamp(3, 1000) == u64::from(*now))
                    && (!equal || l.polygon_sides_equal())
                    && polygon_angle.is_none_or(|a| l.polygon_angle() == Some(a))
                    && side.and_then(|len| l.polygon_radius_for_side(len)).or(polygon_radius).is_none_or(|r| l.polygon_radius() == Some(r))) =>
            {
                return false;
            }
            Some(LiveShape::Ellipse { pie, .. }) if pie_of(*pie) != *pie => return false,
            _ => {}
        }
        if radius.is_none() && kind.is_none() {
            return true;
        }
        let Some(c) = LiveCorners::new(path, live.as_ref()) else { return true };
        let picked = corners_of(*id).unwrap_or_else(|| c.picked(st.selection.anchors.get(id)));
        // Setting corners folds a scale left in a rectangle's transform into its size, which
        // changes it.
        live.as_ref().is_none_or(|l| l.folded() == *l)
            && picked
                .iter()
                .all(|k| *k < c.base.anchor_count() && radius.is_none_or(|r| c.radius(*k) == r.max(0.0)) && kind.is_none_or(|kd| c.kind(*k) == kd))
    };
    if ids.iter().all(has_values) {
        return ok();
    }
    s.edit("Live Shape", |d, sel| {
        for id in &ids {
            let Some(NodeKind::Path { path, live, .. }) = d.node_mut(*id).map(|n| &mut n.kind) else { continue };
            if let Some(l @ LiveShape::Polygon { .. }) = live.as_mut()
                && (sides.is_some() || polygon_angle.is_some() || polygon_radius.is_some() || side.is_some() || equal)
            {
                if let Some(n) = sides {
                    l.set_sides(n);
                }
                if equal {
                    l.make_sides_equal();
                }
                if let Some(a) = polygon_angle {
                    l.set_polygon_angle(a);
                }
                // A side length sets the radius that gives it (with the new side count).
                if let Some(r) = side.and_then(|len| l.polygon_radius_for_side(len)).or(polygon_radius) {
                    l.set_polygon_radius(r);
                }
                *path = l.to_path();
            }
            if let Some(l @ LiveShape::Ellipse { .. }) = live.as_mut()
                && (pie_start.is_some() || pie_end.is_some() || invert)
            {
                if let LiveShape::Ellipse { pie, .. } = l {
                    *pie = pie_of(*pie);
                }
                *path = l.to_path();
            }
            if radius.is_none() && kind.is_none() {
                continue;
            }
            let partial = sel.anchors.get_mut(id);
            // The corners to edit, the Direct-Selected ones and the anchor layout before the edit.
            let Some(c) = LiveCorners::new(path, live.as_ref()) else { continue };
            let corners = corners_of(*id);
            let picked = match &corners {
                Some(k) if k.last().is_some_and(|k| *k >= c.base.anchor_count()) => {
                    let n = c.base.anchor_count();
                    let past: Vec<_> = k.range(n..).collect();
                    return Err(bad("object.setLiveShape", format!("object {id} has anchors 0–{} for `corners`, not {past:?}", n.saturating_sub(1))));
                }
                Some(k) => k.clone(),
                None => c.picked(partial.as_deref()),
            };
            let (selected, layout) = (partial.as_deref().map(|a| c.corners_of(a)), c.sources().to_vec());
            set_corners(path, live, &picked, radius, kind);
            // Direct-Selected corners stay selected as they gain or lose anchors.
            if let (Some(anchors), Some(selected)) = (partial, selected)
                && let Some(c) = LiveCorners::new(path, live.as_ref())
                && c.sources() != layout
            {
                *anchors = c.anchors_of(&selected);
            }
        }
        Ok(())
    })?;
    ok()
}
