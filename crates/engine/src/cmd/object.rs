//! Object menu: transforms, arrange, group, lock/hide, compound paths, clipping masks, isolation,
//! align & distribute, object properties.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::corners::set_corners;
use vectorcraft_doc::{Appearance, Document, Knockout, LiveCorners, LiveShape, Node, NodeId, NodeKind, OrientedBox, Scaling};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, Vec2};

use super::edit::{duplicate_in, outermost, selected_roots, selected_roots_to_transform};
use super::opacitymask::percent;
use super::*;
use crate::{EngineError, LastTransform};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.transform",
            "Transform",
            [],
            None,
            "{matrix: [a,b,c,d,e,f], copy?: bool, ids?, strokes?: bool, corners?: bool, typeAreas?: bool, anchors?: bool, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} apply an affine to the selection (or ids); anchors (without ids or copy): paths with anchors selected with Direct Selection have only those anchors and their handles transformed, as the Transform tools and object.move/rotate/scale/reflect/shear do (default: the whole objects); strokes/corners: Scale Strokes & Effects / Scale Corners (default: the preferences); typeAreas: area type among them (not type inside a group) reshapes its frame by the matrix and its text reflows at its size, as a bounding-box handle drag does (default: the type transforms too)",
            has_doc,
            transform
        ),
        cmd!(
            "object.move",
            "Move…",
            ["Object", "Transform"],
            Some("Cmd+Shift+M"),
            "{ids?: [id…] (selected first; default: the selection), dx, dy, copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection_or_guides,
            move_cmd
        ),
        cmd!(
            "object.rotate",
            "Rotate…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), angle: deg (counter-clockwise), absolute?: bool (angle is the bounding box's new angle, not an amount), origin?: [x,y] (default: the bounding box centre), copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)} → {ids}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            rotate
        ),
        cmd!(
            "object.scale",
            "Scale…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), sx: %, sy?: %, origin?: [x,y], copy?, strokes?: bool (Scale Strokes & Effects: stroke weights, dashes and effect distances scale; off keeps them, type strokes included), corners?: bool (Scale Corners: live corner radii scale), objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)} (strokes/corners default to the preferences). With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            scale
        ),
        cmd!(
            "object.reflect",
            "Reflect…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), axis: \"vertical\"|\"horizontal\"|deg, origin?, copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            reflect
        ),
        cmd!(
            "object.shear",
            "Shear…",
            ["Object", "Transform"],
            None,
            "{ids?: [id…] (selected first; default: the selection), angle: deg, axis?: \"horizontal\"|\"vertical\", origin?, copy?, objects?: bool (Transform Objects, default true; false: the art stays and, with patterns, only its pattern tiles transform), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size)}. With anchors selected with Direct Selection (and no copy), only those anchors and their handles transform, about the centre of their own box.",
            has_selection,
            shear
        ),
        cmd!(
            "object.transformAgain",
            "Transform Again",
            ["Object", "Transform"],
            Some("Cmd+D"),
            "{copy?, strokes?, corners?, patterns?, typeAreas?: bool (as object.transform's; default: the value the transform it repeats used, else object.transform's default)} the last transform's matrix on the selection (after a perspective move or scale: that plane-space transform, without these params)",
            has_selection,
            transform_again
        ),
        cmd!(
            "object.nudge",
            "Nudge",
            [],
            None,
            "{dx: -1|0|1, dy: -1|0|1, big?: bool (×10), copy?: bool, patterns?: bool (Transform Patterns: pattern fills and strokes transform with the objects that move whole; default: prefs transformPatternTiles), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size), objects?: bool (Transform Objects, as object.move)} arrow-key nudge by the keyboard increment (the selected anchors, else objects, else ruler guides)",
            has_selection_or_guides,
            nudge
        ),
        cmd!(
            "object.arrange.bringToFront",
            "Bring to Front",
            ["Object", "Arrange"],
            Some("Cmd+Shift+]"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Front)
        ),
        cmd!(
            "object.arrange.bringForward",
            "Bring Forward",
            ["Object", "Arrange"],
            Some("Cmd+]"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Forward)
        ),
        cmd!(
            "object.arrange.sendBackward",
            "Send Backward",
            ["Object", "Arrange"],
            Some("Cmd+["),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Backward)
        ),
        cmd!(
            "object.arrange.sendToBack",
            "Send to Back",
            ["Object", "Arrange"],
            Some("Cmd+Shift+["),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            |s, _| arrange(s, Arrange::Back)
        ),
        cmd!(
            "object.arrange.sendToCurrentLayer",
            "Send to Current Layer",
            ["Object", "Arrange"],
            None,
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            send_to_current_layer
        ),
        cmd!(
            "object.group",
            "Group",
            ["Object"],
            Some("Cmd+G"),
            "{ids?: [id…] (selected first; default: the selection)} → {id}",
            has_selection,
            group
        ),
        cmd!(
            "object.ungroup",
            "Ungroup",
            ["Object"],
            Some("Cmd+Shift+G"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            ungroup
        ),
        cmd!(
            "object.lock",
            "Selection",
            ["Object", "Lock"],
            Some("Cmd+2"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            lock
        ),
        cmd!("object.unlockAll", "Unlock All", ["Object"], Some("Cmd+Alt+2"), "{}", has_doc, unlock_all),
        cmd!(
            "object.hide",
            "Selection",
            ["Object", "Hide"],
            Some("Cmd+3"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            hide
        ),
        cmd!("object.showAll", "Show All", ["Object"], Some("Cmd+Alt+3"), "{}", has_doc, show_all),
        cmd!(
            "object.compoundPath.make",
            "Make",
            ["Object", "Compound Path"],
            Some("Cmd+8"),
            "{ids?: [id…] (selected first; default: the selection)}",
            has_selection,
            compound_make
        ),
        cmd!(
            "object.compoundPath.release",
            "Release",
            ["Object", "Compound Path"],
            Some("Cmd+Alt+Shift+8"),
            "{ids?: [id…] (selected first; default: the selection)} release the selected compound paths into their paths, each with the compound path's paint; a path of several subpaths (an SVG `d` with several `M`s) splits into one path per subpath",
            has_selection,
            compound_release
        ),
        cmd!(
            "object.clippingMask.make",
            "Make",
            ["Object", "Clipping Mask"],
            Some("Cmd+7"),
            "{ids?: [id…] (selected first; default: the selection)} the topmost selected object (a path, compound path or text, which loses its paint) clips the others: compound holes, even-odd fills and glyph outlines clip as drawn. Paint given to the clipping path later (paint commands with its id) shows: its fill behind the clipped art, its stroke over it → {id} of the clip group",
            has_multi,
            clip_make
        ),
        cmd!(
            "object.clippingMask.release",
            "Release",
            ["Object", "Clipping Mask"],
            Some("Cmd+Alt+7"),
            "{ids?: [id…] (selected first; default: the selection)} the selected clip groups become plain groups; their clipping path (path, compound path or text) stays, with the paint it has (none unless painted after Make)",
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
        cmd!("object.isolate", "Enter Isolation Mode", [], None, "{id?} (default: the selection's first object)", has_doc, isolate),
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
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", to?: \"selection\"|\"artboard\"|\"key\" (default: the key object when the selection has one, select.key, else the selection; a key guide, select.key {guide}, stays put and the objects line up on its line: a vertical guide sets where left, center and right go and vertical alignment leaves them, a horizontal one the other way round; aligning to the key, the other selected ruler guides move too: a vertical guide to the key's left edge, center or right edge, a horizontal one to its top, center or bottom), artboard?: index (0-based; with to: \"artboard\", the artboard to align to; the app passes the active one; default: the artboard under the center of the selection, else the first), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} With anchor points selected (Direct Selection), the points align instead: to their own bounds, or to the artboard (#1180)",
            has_selection,
            align
        ),
        cmd!(
            "object.distribute",
            "Distribute",
            ["Window", "Align"],
            None,
            "{horizontal?: \"left\"|\"center\"|\"right\", vertical?: \"top\"|\"center\"|\"bottom\", spacing?: pt (used with a key object, select.key: the key object stays put and the lines are placed this far apart, outward from its line, in their order on that line; 0 puts them all on its line; not used without a key object or with anchor points selected), to?: \"selection\"|\"artboard\"|\"key\" (default: the selection; \"key\" distributes as \"selection\" does; not used with anchor points selected, or with spacing and a key object), artboard?: index (0-based; with to: \"artboard\", the artboard to distribute across; the app passes the active one; default: as object.align's), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} space the objects' left edges, centers or right edges (top edges, centers or bottom edges) in their order on that line: without spacing and a key object they are spaced evenly, the first and last objects stay put, and fewer than three objects don't move; with spacing and a key object, see spacing (two objects are enough); with to: \"artboard\", the first one's left (top) edge goes to the artboard's left (top) edge, the last one's right (bottom) edge goes to the artboard's right (bottom) edge, and two objects are enough. With anchor points selected (Direct Selection), the points are spaced evenly between the first and the last along the axis instead (#1636)",
            can_distribute,
            distribute
        ),
        cmd!(
            "object.distributeSpacing",
            "Distribute Spacing",
            ["Window", "Align"],
            None,
            "{axis: \"horizontal\"|\"vertical\", spacing?: pt (the key object, select.key, stays put and the others are spaced from it), to?: \"selection\"|\"artboard\"|\"key\" (used without spacing and without anchor points selected; default: the selection; \"key\" spaces as \"selection\" does), artboard?: index (0-based; with to: \"artboard\", the artboard to space across; the app passes the active one; default: as object.align's), bounds?: \"preview\"|\"geometric\" (default: the Use Preview Bounds preference; preview bounds take in strokes), patterns?: bool (Transform Patterns: pattern fills and strokes transform with the art; default: prefs transformPatternTiles)} without spacing, make the gaps between the objects equal, in the order of their left (top) edges: the first object's left (top) edge and the last one's right (bottom) edge stay put, or with to: \"artboard\" go to the artboard's left (top) and right (bottom) edges. With anchor points selected (Direct Selection), the points are spaced evenly along the axis instead, or `spacing` apart from the first (#1636)",
            can_distribute,
            distribute_spacing
        ),
        cmd!(
            "object.setBounds",
            "Set Bounds",
            [],
            None,
            "{x?, y?, width?, height?, reference?: 0..8 (9-point grid), proportional?, strokes?, corners?, copy?: bool, patterns?: bool (Transform Patterns), typeAreas?: bool (area type: the frame is reshaped and the text reflows at its size), objects?: bool (Transform Objects, as object.move)} (Transform panel; width/height and the reference point follow the bounding box, rotated with rotated objects; x, y are page coordinates; with Use Preview Bounds the values measure the visual bounds; strokes/corners as object.scale; with anchor points selected (Direct Selection) and no copy, the values are the box around those points, square to the page, and only they move or scale, as with object.move (#1824))",
            has_selection,
            set_bounds
        ),
        cmd!("object.expandShape", "Expand Shape", ["Object", "Shape"], None, "{} convert live shapes to plain paths", has_selection, expand_shape),
        cmd!(
            "object.setLiveShape",
            "Live Shape Properties",
            [],
            None,
            "{id?, ids?, radius?: pt, kind?: \"round\"|\"invertedRound\"|\"chamfer\", corners?: [i…], items?: [{id, corners?: [i…]}] (several objects, each with its own corners, in place of ids and corners: the canvas's corner widgets across a selection), sides?: n, polygonAngle?: degrees (counterclockwise from its first vertex straight up), polygonRadius?: pt (centre to vertex), sideLength?: pt (sets the radius), makeSidesEqual?: true (a polygon scaled unevenly: drops the uneven scale and shear, keeping its centre, angle and mean radius), pieStart?, pieEnd?: degrees (an ellipse's pie, counterclockwise from 3 o'clock; 0 to 360 is the whole ellipse), invertPie?: true (swaps them: the other part of the ellipse)} (Live Corners on any path: radius and kind set its corners (anchors that aren't smooth, where two sides, straight or curved (#1144), meet at an angle): `corners` (anchor indices of the path with its corners uncut, counting every subpath's anchors in order: a rectangle's 0 top-left, 1 top-right, 2 bottom-right, 3 bottom-left; a polygon's from the first vertex clockwise), else the corners with a Direct-Selected anchor, else every corner; each radius is drawn no larger than half the corner's shorter side allows; a path that isn't a live shape keeps its uncut outline so its corners stay editable, and is plain again once none is cut; sides: a polygon's, which keep the radius they shared; the polygon params apply in that order: sides, makeSidesEqual, polygonAngle, then sideLength or else polygonRadius)",
            has_selection,
            set_live_shape
        ),
    ]
}

fn origin_of(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Point> {
    if let Some(o) = point_param(p, "origin") {
        return Ok(o);
    }
    // Direct-selected anchors transform alone (see [`apply_transform`]), about their own centre.
    let st = s.doc()?;
    if !bool_or(p, "copy", false)
        && let Some(b) = st.selection.anchor_bounds(&st.doc, s.prefs.use_preview_bounds)
    {
        return Ok(b.center());
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
    let paths = d.index_paths(ids.iter().copied());
    for n in ids.iter().filter_map(|id| paths.get(id)).filter_map(|p| d.node_at(p)) {
        n.walk(&mut |c| uses |= vectorcraft_doc::pattern::uses_pattern(c, None));
    }
    if uses {
        s.note_journal("patterns", json!(on));
    }
    Ok(on)
}

/// Transform Objects (`objects` param, on unless it is false): off, a transform leaves the art
/// where it is and, with Transform Patterns, moves only its pattern tiles.
pub(crate) fn transforms_objects(p: &Value) -> bool {
    bool_or(p, "objects", true)
}

/// `xf` on `n` with Transform Objects (`objects`, see [`transforms_objects`]) on or off.
pub(crate) fn transform_node(n: &mut Node, xf: Affine, sc: Scaling, objects: bool) {
    if objects {
        n.transform(xf, sc);
    } else if sc.patterns {
        vectorcraft_doc::pattern::transform_patterns_only(n, xf);
    }
}

/// Apply `xf` to `ids` (`copy` param: duplicate first; `strokes`/`corners`: see [`scaling`];
/// `patterns`: see [`transform_patterns`]; `objects`: see [`transforms_objects`]; `typeAreas`:
/// see [`resize_type_area`]). Outside an
/// interaction, records `xf` and the options it used for Transform Again. In an interaction whose
/// document is still its snapshot, a transform with `copy` keeps its copies ([`preview_copies`]),
/// and the next one that copies the same objects of the same snapshot transforms those copies.
///
/// `anchors` (`ids` being the selection): with anchors selected with Direct Selection and no
/// copy, only those anchors transform, their handles with them, and the objects selected whole.
pub(crate) fn apply_transform(s: &mut Session, label: &str, ids: Vec<NodeId>, xf: Affine, p: &Value, anchors: bool) -> Result<Value> {
    let copy = bool_or(p, "copy", false);
    let areas = bool_or(p, "typeAreas", false);
    let objects = transforms_objects(p);
    let mut sc = if Scaling::factor(xf).is_some() { scaling(s, p) } else { Scaling::default() };
    sc.patterns = transform_patterns(s, p, &ids)?;
    let c = xf.as_coeffs();
    let is_translation = (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9;
    let delta = Vec2::new(c[4], c[5]);
    let with_guides = objects && ids_param(p, "ids").is_none() && is_translation;
    let has_guides = with_guides && !s.doc()?.selection.guides.is_empty();
    // A transform of ids that name no object, or an identity transform without a copy, moves
    // nothing: the document and its undo history stay as they are, and Transform Again repeats it.
    // With a copy, the reply lists the copies, and there are none.
    let d = &s.doc()?.doc;
    let none = d.index_paths(ids.iter().copied()).is_empty() && !has_guides;
    let ids = if anchors && objects && !copy && !s.doc()?.selection.anchors.is_empty() {
        if xf != Affine::IDENTITY {
            super::path::transform_anchors(s, label, xf, sc)?;
        }
        ids
    } else if none || (!copy && xf == Affine::IDENTITY) {
        if copy { vec![] } else { ids }
    } else {
        let kept = if copy { preview_copies(s, &ids, with_guides)? } else { None };
        s.edit(label, |d, sel| {
            let targets = match &kept {
                // The document becomes the snapshot with the kept copies, untransformed, and the
                // loop below transforms only the copies.
                Some(k) => {
                    *d = Document::clone(&k.doc);
                    sel.clone_from(&k.selection);
                    k.ids.clone()
                }
                None if copy => {
                    let guide_copies = if with_guides {
                        let copies: Vec<_> = sel.guides.iter().filter_map(|&i| d.guides.get(i).cloned()).collect();
                        (!copies.is_empty()).then_some(copies)
                    } else {
                        None
                    };
                    let new_ids = duplicate_in(d, sel, &ids, Affine::IDENTITY)?;
                    if let Some(copies) = guide_copies {
                        let first = d.guides.len();
                        d.guides.extend(copies);
                        sel.guides = (first..d.guides.len()).collect();
                    }
                    new_ids
                }
                None => ids.clone(),
            };
            // Transforms don't move nodes in the tree, so the paths found up front stay right.
            let paths = d.index_paths(targets.iter().copied());
            for id in &targets {
                if let Some(n) = paths.get(id).and_then(|p| d.node_mut_at(p))
                    && !(objects && areas && resize_type_area(n, xf))
                {
                    transform_node(n, xf, sc, objects);
                }
            }
            if with_guides && (delta.x != 0.0 || delta.y != 0.0) {
                for i in sel.guides.clone() {
                    if let Some(g) = d.guides.get_mut(i) {
                        *g = g.moved(delta);
                    }
                }
            }
            Ok(targets)
        })?
    };
    let st = s.doc_mut()?;
    if st.interaction.is_none() {
        let mut options = json!({ "copy": copy, "typeAreas": areas, "patterns": sc.patterns, "objects": objects });
        if Scaling::factor(xf).is_some() {
            options["strokes"] = json!(sc.strokes);
            options["corners"] = json!(!sc.keep_corners);
        }
        st.last_transform = Some(LastTransform::Matrix(xf, options));
        st.last_perspective = None;
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Returns the copies of `roots` for a transform or a perspective edit with `copy` in an interaction
/// whose document is still its snapshot, as in a preview ([`Session::preview`] puts the snapshot
/// back first). These are the copies the interaction keeps when they were made of that snapshot for
/// the same `roots`; otherwise this copies `roots` with [`duplicate_in`] in a clone of the snapshot,
/// untransformed, and the interaction keeps that clone. It returns `None` outside an interaction,
/// and when the document is no longer the snapshot, as for a command run in a typing session or a
/// script batch after an earlier edit there: the command then copies the current document.
pub(crate) fn preview_copies(s: &mut Session, roots: &[NodeId], with_guides: bool) -> Result<Option<Arc<crate::PreviewCopies>>> {
    let st = s.doc_mut()?;
    let Some(it) = &mut st.interaction else { return Ok(None) };
    if !Arc::ptr_eq(&st.doc, &it.doc) {
        return Ok(None);
    }
    let has_guide_copies = with_guides && !st.selection.guides.is_empty();
    if let Some(k) = &it.copies
        && Arc::ptr_eq(&k.from, &it.doc)
        && k.roots == roots
        && (k.selection.guides.is_empty() != has_guide_copies)
    {
        return Ok(Some(k.clone()));
    }
    let mut doc = it.doc.clone();
    let mut selection = st.selection.clone();
    let ids = duplicate_in(Arc::make_mut(&mut doc), &mut selection, roots, Affine::IDENTITY)?;
    if has_guide_copies {
        let d = Arc::make_mut(&mut doc);
        let first = d.guides.len();
        let copies: Vec<_> = st.selection.guides.iter().filter_map(|&i| d.guides.get(i).cloned()).collect();
        d.guides.extend(copies);
        selection.guides = (first..d.guides.len()).collect();
    }
    let k = Arc::new(crate::PreviewCopies { from: it.doc.clone(), roots: roots.to_vec(), doc, selection, ids });
    it.copies = Some(k.clone());
    Ok(Some(k))
}

/// Area type resized by its bounding box: `xf` reshapes its frame (the type area) and the text
/// reflows at its size. False for anything else (and type in perspective), which transforms as
/// usual.
fn resize_type_area(n: &mut Node, xf: Affine) -> bool {
    if n.extras().perspective.is_some() {
        return false;
    }
    let NodeKind::Text(t) = &mut n.kind else { return false };
    super::typecmd::reshape_area_with(t, |t| t.transform_area(xf))
}

fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    let m = matrix_param(p, "matrix").ok_or_else(|| bad("object.transform", "missing matrix [a,b,c,d,e,f]"))?;
    let (ids, anchors) = match ids_param(p, "ids") {
        Some(v) => (outermost(&s.doc()?.doc, v), false),
        None => (selected_roots_to_transform(s)?, bool_or(p, "anchors", false)),
    };
    apply_transform(s, "Transform", ids, m, p, anchors)
}

fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["dx", "dy"], "object.move")?;
    let ids = selected_roots_to_transform(s)?;
    apply_transform(s, "Move", ids, Affine::translate((f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0))), p, true)
}

fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["dx", "dy"], "object.nudge")?;
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let dx = f64_or(p, "dx", 0.0) * k;
    let dy = f64_or(p, "dy", 0.0) * k;
    if !s.doc()?.selection.anchors.is_empty() {
        // When the anchors move by 0, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 {
            return ok();
        }
        return super::path::move_anchors(s, &json!({ "dx": dx, "dy": dy, "patterns": p.get("patterns") }));
    }
    if s.doc()?.selection.is_empty() {
        let copy = bool_or(p, "copy", false);
        // When guides move by 0 without a copy, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 && !copy {
            return ok();
        }
        return super::docmenu::guide_move(s, &json!({ "dx": dx, "dy": dy, "copy": copy }));
    }
    let ids = selected_roots_to_transform(s)?;
    apply_transform(s, "Move", ids, Affine::translate((dx, dy)), p, true)
}

fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

fn rotate(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["angle"], "object.rotate")?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    let mut angle = f64_or(p, "angle", 0.0);
    if bool_or(p, "absolute", false) {
        angle = vectorcraft_geom::normalize_deg(angle - s.doc()?.doc.bbox_angle(&ids));
    }
    // Angles are counter-clockwise; y is down, so negate.
    let a = about(o, Affine::rotate(-angle.to_radians()));
    apply_transform(s, "Rotate", ids, a, p, true)
}

fn scale(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["sy"], "object.scale")?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    let sx = f64_req(p, "sx", "object.scale")? / 100.0;
    let sy = f64_or(p, "sy", sx * 100.0) / 100.0;
    if sx == 0.0 || sy == 0.0 {
        return Err(bad("object.scale", "scale must be non-zero"));
    }
    apply_transform(s, "Scale", ids, about(o, Affine::scale_non_uniform(sx, sy)), p, true)
}

fn reflect(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots_to_transform(s)?;
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
    apply_transform(s, "Reflect", ids, about(o, m), p, true)
}

fn shear(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["angle"], "object.shear")?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    let t = f64_or(p, "angle", 0.0).clamp(-89.0, 89.0).to_radians().tan();
    let vertical = choice(p, "object.shear", "axis", &["horizontal", "vertical"])? == Some("vertical");
    let m = if vertical { Affine::new([1.0, t, 0.0, 1.0, 0.0, 0.0]) } else { Affine::new([1.0, 0.0, -t, 1.0, 0.0, 0.0]) };
    apply_transform(s, "Shear", ids, about(o, m), p, true)
}

/// The last transform on the selection, with the options `last_transform` keeps for it; an option
/// that `p` sets to true or false (a recorded Transform Again's) takes precedence. A Transform
/// Each is repeated by running it again (its transform is per object, about each one's bounds).
fn transform_again(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(again) = s.doc()?.last_perspective.clone() {
        return super::distortcmds::transform_again(s, &again);
    }
    let last = s.doc()?.last_transform.clone().ok_or_else(|| EngineError::Other("no previous transform".into()))?;
    let merge = |mut options: Value| {
        if let (Some(o), Some(given)) = (options.as_object_mut(), p.as_object()) {
            o.extend(given.iter().filter(|(_, v)| v.is_boolean()).map(|(k, v)| (k.clone(), v.clone())));
        }
        options
    };
    match last {
        LastTransform::Matrix(m, options) => {
            let ids = selected_roots_to_transform(s)?;
            apply_transform(s, "Transform Again", ids, m, &merge(options), true)
        }
        LastTransform::Each(params) => super::menucmds::transform_each_again(s, &merge(params)),
    }
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
    // When the current layer is hidden or locked, or is inside a hidden or locked layer, the
    // command fails with the error that `DocState::target_parent` gives for new art.
    if !st.doc.is_editable(layer) {
        return Err(EngineError::Other("the target layer is locked or hidden".into()));
    }
    // When the selected objects are already the topmost in the current layer, in paint order, the
    // document and its undo history stay as they are.
    if st.doc.children(Some(layer)).is_some_and(|c| c.iter().rev().map(|n| n.id).take(ids.len()).eq(ids.iter().rev().copied())) {
        return ok();
    }
    s.edit("Send to Current Layer", |d, _| {
        let moved = d.remove_many(&ids)?;
        d.children_mut(Some(layer))?.extend(moved);
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
    if ids.is_empty() {
        return Err(EngineError::Other("nothing selected".into()));
    }
    let gid = d.alloc_id();
    Ok(d.wrap(ids, |children| Node::group(gid, children))?)
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
        let mut ungrouped = vec![];
        for id in &ids {
            let Some(n) = d.node(*id) else { continue };
            let NodeKind::Group { children, clip } = &n.kind else {
                new_sel.push(*id);
                continue;
            };
            let mut out = Vec::with_capacity(children.len());
            for (k, c) in children.iter().enumerate() {
                let mut c = (**c).clone();
                // The clipping path keeps its paint (none, unless painted after Make).
                if *clip
                    && k == 0
                    && let NodeKind::Path { clipping, .. } = &mut c.kind
                {
                    *clipping = false;
                }
                c.opacity *= n.opacity;
                new_sel.push(c.id);
                out.push(c);
            }
            ungrouped.push((*id, out));
        }
        d.insert_beside(ungrouped, true)?;
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
    let Some(backmost) = ids.first().copied() else { return Err(EngineError::Other("nothing selected".into())) };
    let id = s.edit("Make Compound Path", |d, sel| {
        d.node(backmost).ok_or(EngineError::NoNode(backmost))?;
        let mut units: Vec<Vec<Arc<Node>>> = vec![];
        let mut appearance = None;
        for id in &ids {
            let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
            if !matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Group { .. }) {
                return Err(EngineError::Other("compound paths can only contain paths".into()));
            }
            compound_members(n, &mut appearance, &mut units);
        }
        let cid = d.alloc_id();
        let mut c = Node::new(cid, NodeKind::Compound { children: vec![], rule: FillRule::NonZero });
        c.appearance = appearance.unwrap_or_default();
        let ch = c.children_mut().ok_or(EngineError::NoNode(cid))?;
        // The paths in front run against the backmost one, so where they overlap it they cut holes
        // under the non-zero fill rule (#1197). A compound path's paths turn together, by its
        // first: its own holes (a letter's counters) stay holes.
        let direction = |n: &Node| n.path_data().and_then(|pd| pd.subpaths.first()).map_or(0.0, |sp| sp.signed_area().signum());
        let first = |unit: &[Arc<Node>]| unit.first().map_or(0.0, |p| direction(p));
        let back = units.first().map_or(0.0, |u| first(u));
        for (i, unit) in units.into_iter().enumerate() {
            let flip = i > 0 && back != 0.0 && first(&unit) == back;
            for mut p in unit {
                let pn = Arc::make_mut(&mut p);
                pn.appearance = Appearance::default().into();
                if let NodeKind::Path { path, live, .. } = &mut pn.kind {
                    *live = None;
                    if flip {
                        path.reverse();
                    }
                }
                ch.push(p);
            }
        }
        d.wrap(&ids, |_| c)?;
        sel.set([cid]);
        Ok(cid)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The paths in `n` (a path, a compound path, or a group searched in depth) onto `out`, in paint
/// order: each path alone, a compound path's paths together. Into `paint` goes the appearance of
/// the backmost found so far: the path's own, or its compound path's, which is what painted it. A
/// group has no paint of its own to give: a group of outlined type made a compound path that drew
/// nothing.
fn compound_members(n: &Node, paint: &mut Option<vectorcraft_doc::Shared<Appearance>>, out: &mut Vec<Vec<Arc<Node>>>) {
    match (&n.kind, n.children()) {
        (NodeKind::Path { .. }, _) => out.push(vec![Arc::new(n.clone())]),
        (NodeKind::Compound { .. }, Some(paths)) if !paths.is_empty() => out.push(paths.clone()),
        (_, children) => return children.into_iter().flatten().for_each(|c| compound_members(c, paint, out)),
    }
    paint.get_or_insert_with(|| n.appearance.clone());
}

/// Is `n` a compound path to release: a compound path, or a path of several subpaths (an SVG `d`
/// with several `M`s, a PDF path), which is one in Illustrator too (#1163)?
fn releases(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Compound { .. } => true,
        NodeKind::Path { path, .. } => path.subpaths.len() > 1,
        _ => false,
    }
}

fn compound_release(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    // With no compound path selected, the document, the selection and the undo history stay as
    // they are.
    let d = &s.doc()?.doc;
    if !ids.iter().any(|id| d.node(*id).is_some_and(releases)) {
        return ok();
    }
    s.edit("Release Compound Path", |d, sel| {
        let mut out = vec![];
        let mut released = vec![];
        for id in &ids {
            let Some(n) = d.node(*id).cloned().filter(releases) else { continue };
            let mut parts = vec![];
            match &n.kind {
                NodeKind::Compound { children, .. } => {
                    for c in children {
                        let mut c = (**c).clone();
                        c.appearance = n.appearance.clone();
                        out.push(c.id);
                        parts.push(c);
                    }
                }
                // Each subpath becomes a path of its own with the path's paint, in the same
                // stacking order; the first keeps the path's id.
                NodeKind::Path { path, .. } => {
                    for (k, sp) in path.subpaths.iter().enumerate() {
                        let mut c = if k == 0 { n.clone() } else { d.reid(&n) };
                        if let NodeKind::Path { path, live, .. } = &mut c.kind {
                            (*path, *live) = (PathData::single(sp.clone()), None);
                        }
                        out.push(c.id);
                        parts.push(c);
                    }
                }
                _ => {}
            }
            released.push((*id, parts));
        }
        d.insert_beside(released, true)?;
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
    c.appearance = Appearance::basic(Paint::None, Paint::None, 0.0).into();
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
        let gid = d.alloc_id();
        // The topmost object, the clipping path, goes first, and the others follow in paint order.
        d.wrap(&ids, |mut children| {
            children.rotate_right(1);
            Node::new(gid, NodeKind::Group { children, clip: true })
        })?;
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
    // Groups, layers and single objects (a path, a compound path, an image…); type isn't
    // isolated: a double-click on it edits its text.
    if matches!(st.doc.node(id).ok_or(EngineError::NoNode(id))?.kind, NodeKind::Text(_)) {
        return Err(bad("object.isolate", "type objects can't be isolated"));
    }
    let isolation = st.doc.node(id).filter(|n| n.extras().shaper.is_some()).and_then(|n| n.children()).and_then(|c| c.first()).map_or(id, |n| n.id);
    st.isolation = Some(isolation);
    st.selection.clear();
    st.revision += 1;
    ok()
}

fn exit_isolation(s: &mut Session, _: &Value) -> Result<Value> {
    let guides_locked = s.menu.guides_locked;
    let st = s.doc_mut()?;
    if let Some(i) = st.isolation.take() {
        super::distortcmds::finish_edit_text(st, i);
        let target = st.doc.parent_of(i).filter(|p| st.doc.node(*p).is_some_and(|n| n.extras().shaper.is_some())).unwrap_or(i);
        st.selection.set([target]);
        // A guide path isolated while Lock Guides is on isn't selected (#1534).
        if guides_locked {
            crate::deselect_guides(&st.doc, &mut st.selection);
        }
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
            && data.iter().flatten().all(|(k, v)| {
                n.extras().attrs.as_ref().and_then(|a| a.data.iter().find(|(key, _)| key == k)).map(|(_, now)| now.as_str()) == v.as_deref()
            })
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

/// `spacing` of `object.distribute` and `object.distributeSpacing`: a finite number, or `None`
/// when it is missing or null. Any other value is an error.
fn spacing(p: &Value, cmd: &str) -> Result<Option<f64>> {
    p.get("spacing")
        .filter(|v| !v.is_null())
        .map(|v| v.as_f64().filter(|g| g.is_finite()).ok_or_else(|| bad(cmd, "`spacing` must be a number")))
        .transpose()
}

/// `ids` with their bounds (`preview`: visual bounds).
fn items_bounds(d: &Document, ids: &[NodeId], preview: bool) -> Vec<(NodeId, Rect)> {
    ids.iter().filter_map(|id| Some((*id, d.bounds_of(&[*id], preview)?))).collect()
}

/// Move each object of `moves` by its offset and each ruler guide of `guides` (index, position)
/// to its position, as undo step `label`; pattern tiles move too when [`transform_patterns`] says
/// so for `ids`. When every move is a rounding error (1e-9 pt or less on both axes), the document
/// and its undo history stay as they are.
fn apply_moves(s: &mut Session, label: &str, p: &Value, ids: &[NodeId], moves: &[(NodeId, Vec2)], guides: &[(usize, f64)]) -> Result<Value> {
    let sc = Scaling { patterns: transform_patterns(s, p, ids)?, ..Scaling::default() };
    let guides: Vec<(usize, f64)> = {
        let d = &s.doc()?.doc;
        guides.iter().copied().filter(|&(i, pos)| d.guides.get(i).is_some_and(|g| (g.pos - pos).abs() > 1e-9)).collect()
    };
    if moves.iter().all(|(_, dv)| dv.x.abs() <= 1e-9 && dv.y.abs() <= 1e-9) && guides.is_empty() {
        return ok();
    }
    s.edit(label, |d, _| {
        for (id, dv) in moves {
            if let Some(n) = d.node_mut(*id) {
                n.transform(Affine::translate(*dv), sc);
            }
        }
        for &(i, pos) in &guides {
            if let Some(g) = d.guides.get_mut(i) {
                g.pos = pos;
            }
        }
        Ok(())
    })?;
    ok()
}

/// What `object.align` aligns to: `to`, else the key object (or key guide) when there is one,
/// else the selection.
fn align_to<'a>(s: &Session, p: &'a Value) -> Option<&'a str> {
    str_param(p, "to").or_else(|| s.doc().ok()?.selection.has_key().then_some("key"))
}

/// The key guide `object.align` lines the objects up on (aligning to the key with one set,
/// [`vectorcraft_doc::Selection::key_guide`]): its position, and whether it is vertical.
fn key_guide(s: &Session, p: &Value) -> Option<(f64, bool)> {
    let st = s.doc().ok()?;
    let g = st.doc.guides.get(st.selection.key_guide?)?;
    (align_to(s, p) == Some("key")).then_some((g.pos, g.vertical))
}

/// Where Align puts the objects, `r`, and the axes it moves them along (x, y): a key guide gives
/// the line across its axis only (a vertical guide fixes x, a horizontal one y), so the other
/// axis stays as it is.
fn align_reference(s: &Session, p: &Value, ids: &[NodeId], preview: bool, own: Option<Rect>) -> Result<(Rect, bool, bool)> {
    if let Some((pos, vertical)) = key_guide(s, p) {
        return Ok((Rect::new(pos, pos, pos, pos), vertical, !vertical));
    }
    let r = match own {
        Some(b) if align_to(s, p) != Some("artboard") => b,
        _ => reference_rect(s, p, ids, preview)?,
    };
    Ok((r, true, true))
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

/// Returns the artboard for `to: "artboard"` of `object.distribute` and `object.distributeSpacing`
/// (command `cmd`), the same artboard as `object.align` uses ([`reference_rect`]). It returns
/// `None` for the other choices. A `to` outside the choices is an error. Both commands call it
/// before they space selected anchor points, and they space those points without the artboard.
fn distribute_to(s: &Session, p: &Value, ids: &[NodeId], preview: bool, cmd: &str) -> Result<Option<Rect>> {
    match choice(p, cmd, "to", &["selection", "artboard", "key"])? {
        Some("artboard") => reference_rect(s, p, ids, preview).map(Some),
        _ => Ok(None),
    }
}

/// Returns the selected root (one of `ids`) that is the key object or contains it, such as the
/// compound path or selected group the key is in. Align to the key object, and Distribute and
/// Distribute Spacing with a spacing, keep it in place.
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
    // Anchor points selected with Direct Selection align themselves, as in Illustrator (#1180).
    if !s.doc()?.selection.anchors.is_empty() {
        return align_anchors(s, p, &ids, (h, v));
    }
    let (r, along_x, along_y) = align_reference(s, p, &ids, preview, None)?;
    let (h, v) = (h.filter(|_| along_x), v.filter(|_| along_y));
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
    let guides = guide_moves(s, p, r, h, v)?;
    apply_moves(s, "Align", p, &ids, &moves, &guides)
}

/// The selected ruler guides that Align to the key object (or key guide) moves, and where: a
/// vertical guide goes to the left edge, centre or right edge of `r` that `h` names, a horizontal
/// one to the top, centre or bottom that `v` names. The key guide stays put, and so do guides
/// when aligning to the selection or the artboard.
fn guide_moves(s: &Session, p: &Value, r: Rect, h: Option<&str>, v: Option<&str>) -> Result<Vec<(usize, f64)>> {
    if align_to(s, p) != Some("key") {
        return Ok(vec![]);
    }
    let st = s.doc()?;
    let at = |vertical: bool| match (vertical, h, v) {
        (true, Some("left"), _) => Some(r.x0),
        (true, Some("center"), _) => Some(r.center().x),
        (true, Some("right"), _) => Some(r.x1),
        (false, _, Some("top")) => Some(r.y0),
        (false, _, Some("center")) => Some(r.center().y),
        (false, _, Some("bottom")) => Some(r.y1),
        _ => None,
    };
    Ok(st
        .selection
        .guides
        .iter()
        .filter(|&&i| Some(i) != st.selection.key_guide)
        .filter_map(|&i| Some((i, at(st.doc.guides.get(i)?.vertical)?)))
        .collect())
}

/// [`align`] of the selected anchor points (whole paths among them give all theirs): each point
/// moves to the edge or centre of the points' bounds, or of the artboard with Align To Artboard.
fn align_anchors(s: &mut Session, p: &Value, ids: &[NodeId], (h, v): (Option<&str>, Option<&str>)) -> Result<Value> {
    let targets = super::path::anchor_targets(s)?;
    let d = &s.doc()?.doc;
    let pts: Vec<Point> = targets
        .iter()
        .flat_map(|(id, refs)| refs.iter().filter_map(move |(si, ai)| d.node(*id)?.path_data()?.subpaths.get(*si)?.anchors.get(*ai).map(|a| a.p)))
        .collect();
    let Some(bounds) = pts.iter().skip(1).fold(pts.first().map(|q| Rect::from_points(*q, *q)), |b, q| b.map(|b| b.union_pt(*q))) else {
        return ok();
    };
    // shortcut: no key anchor yet, so Align To Key Object aligns to the points' bounds as Selection does
    // (or to a key guide).
    let (r, along_x, along_y) = align_reference(s, p, ids, false, Some(bounds))?;
    let (h, v) = (h.filter(|_| along_x), v.filter(|_| along_y));
    let target = |q: Point| {
        let x = match h {
            Some("left") => r.x0,
            Some("center") => r.center().x,
            Some("right") => r.x1,
            _ => q.x,
        };
        let y = match v {
            Some("top") => r.y0,
            Some("center") => r.center().y,
            Some("bottom") => r.y1,
            _ => q.y,
        };
        Point::new(x, y)
    };
    if pts.iter().all(|q| {
        let dv = target(*q) - *q;
        dv.x.abs() <= 1e-9 && dv.y.abs() <= 1e-9
    }) {
        return ok();
    }
    s.edit("Align", |d, _| {
        for (id, refs) in &targets {
            let path = super::path::path_mut(d, *id)?;
            for &(si, ai) in refs {
                if let Some(a) = path.anchor_mut(si, ai) {
                    a.translate(target(a.p) - a.p);
                }
            }
        }
        Ok(())
    })?;
    ok()
}

/// Distribute needs two or more objects, or anchor points selected with Direct Selection (they
/// may all be on one path).
fn can_distribute(s: &Session) -> std::result::Result<(), String> {
    if s.active().is_some_and(|st| !st.selection.anchors.is_empty()) { Ok(()) } else { has_multi(s) }
}

/// [`distribute`] and [`distribute_spacing`] of the selected anchor points (whole paths among them
/// give all theirs): the points between the first and the last along the axis are spaced evenly,
/// or `gap` apart from the first. A point has no size, so every Distribute option of an axis
/// gives the same result.
fn distribute_anchors(s: &mut Session, label: &str, horiz: bool, gap: Option<f64>) -> Result<Value> {
    let targets = super::path::anchor_targets(s)?;
    let d = &s.doc()?.doc;
    let along = |q: Point| if horiz { q.x } else { q.y };
    let mut pts: Vec<(NodeId, usize, usize, f64)> = targets
        .iter()
        .flat_map(|(id, refs)| {
            refs.iter().filter_map(move |&(si, ai)| Some((*id, si, ai, along(d.node(*id)?.path_data()?.subpaths.get(si)?.anchors.get(ai)?.p))))
        })
        .collect();
    pts.sort_by(|a, b| a.3.total_cmp(&b.3));
    let (Some(first), Some(last)) = (pts.first().map(|q| q.3), pts.last().map(|q| q.3)) else {
        return ok();
    };
    // Two points are the ends already; with a gap the second is placed from the first.
    if pts.len() < if gap.is_some() { 2 } else { 3 } {
        return ok();
    }
    let step = gap.unwrap_or((last - first) / (pts.len() - 1) as f64);
    let moves: Vec<(NodeId, usize, usize, f64)> =
        pts.iter().enumerate().map(|(i, &(id, si, ai, at))| (id, si, ai, first + step * i as f64 - at)).collect();
    if moves.iter().all(|m| m.3.abs() <= 1e-9) {
        return ok();
    }
    s.edit(label, |d, _| {
        for &(id, si, ai, delta) in &moves {
            if let Some(a) = super::path::path_mut(d, id)?.anchor_mut(si, ai) {
                a.translate(if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) });
            }
        }
        Ok(())
    })?;
    ok()
}

fn distribute(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let preview = preview_bounds(s, p, "object.distribute")?;
    let board = distribute_to(s, p, &ids, preview, "object.distribute")?;
    let spacing = spacing(p, "object.distribute")?;
    // Anchor points selected with Direct Selection are distributed themselves, as Align aligns them.
    if !s.doc()?.selection.anchors.is_empty() {
        return match direction(p, "object.distribute")? {
            (None, None) => Err(bad("object.distribute", "give horizontal or vertical")),
            (h, _) => distribute_anchors(s, "Distribute", h.is_some(), None),
        };
    }
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
    // With a spacing, the key object (or the compound path or selected group it is in) stays where
    // it is, and the lines are placed `spacing` apart, outward from its line, in their order.
    // Without a key object the spacing is not used. `from_key` is the key's index in `items` and
    // the spacing.
    let from_key = spacing.zip(key_root(s, &ids)).and_then(|(g, k)| Some((items.iter().position(|(id, _)| *id == k)?, g)));
    // Two objects are enough on an artboard, where they go to its edges, and when they are spaced
    // from a key object.
    if n < 2 || (n < 3 && board.is_none() && from_key.is_none()) {
        return ok();
    }
    let (first_box, last_box) = (&items[0].1, &items[n - 1].1);
    // `first` and `last` are the lines that the first and last objects end on. Without an artboard
    // the first and last objects stay where they are. On an artboard the first object moves until
    // its left (top) edge is on the artboard's left (top) edge, and the last object moves until its
    // right (bottom) edge is on the artboard's right (bottom) edge.
    let (first, last) = match board {
        Some(r) if horiz => (key(first_box, horiz) + r.x0 - first_box.x0, key(last_box, horiz) + r.x1 - last_box.x1),
        Some(r) => (key(first_box, horiz) + r.y0 - first_box.y0, key(last_box, horiz) + r.y1 - last_box.y1),
        None => (key(first_box, horiz), key(last_box, horiz)),
    };
    let moves: Vec<(NodeId, Vec2)> = items
        .iter()
        .enumerate()
        .map(|(i, (id, r))| {
            let target = match from_key {
                Some((k, g)) => key(&items[k].1, horiz) + g * (i as f64 - k as f64),
                None => first + (last - first) * i as f64 / (n - 1) as f64,
            };
            let delta = target - key(r, horiz);
            (*id, if horiz { Vec2::new(delta, 0.0) } else { Vec2::new(0.0, delta) })
        })
        .collect();
    apply_moves(s, "Distribute", p, &ids, &moves, &[])
}

fn distribute_spacing(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = selected_roots(s)?;
    let horiz = choice(p, "object.distributeSpacing", "axis", &["horizontal", "vertical"])? != Some("vertical");
    let preview = preview_bounds(s, p, "object.distributeSpacing")?;
    let board = distribute_to(s, p, &ids, preview, "object.distributeSpacing")?;
    let spacing = spacing(p, "object.distributeSpacing")?;
    if !s.doc()?.selection.anchors.is_empty() {
        return distribute_anchors(s, "Distribute Spacing", horiz, spacing);
    }
    let mut items = items_bounds(&s.doc()?.doc, &ids, preview);
    items.sort_by(|a, b| if horiz { a.1.x0.total_cmp(&b.1.x0) } else { a.1.y0.total_cmp(&b.1.y0) });
    let n = items.len();
    let &[(_, first), .., (_, last)] = items.as_slice() else {
        return Err(EngineError::Other("select two or more objects to distribute".into()));
    };
    let size = |r: &Rect| if horiz { r.width() } else { r.height() };
    let start = |r: &Rect| if horiz { r.x0 } else { r.y0 };
    // The objects start at `lo`, and without a spacing the last one ends at `hi`. These are the
    // first object's left (top) edge and the last one's right (bottom) edge, or the artboard's
    // edges with Align to Artboard and no spacing.
    let (lo, hi) = match board.filter(|_| spacing.is_none()) {
        Some(r) if horiz => (r.x0, r.x1),
        Some(r) => (r.y0, r.y1),
        None if horiz => (first.x0, last.x1),
        None => (first.y0, last.y1),
    };
    let gap = match spacing {
        Some(g) => g,
        None => (hi - lo - items.iter().map(|i| size(&i.1)).sum::<f64>()) / (n - 1) as f64,
    };
    let mut pos = lo;
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
    apply_moves(s, "Distribute Spacing", p, &ids, &moves, &[])
}

fn set_bounds(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["x", "y", "width", "height"], "object.setBounds")?;
    let ids = selected_roots(s)?;
    let st = s.doc()?;
    if transforms_objects(p)
        && !bool_or(p, "copy", false)
        && let Some(b) = st.selection.anchor_bounds(&st.doc, s.prefs.use_preview_bounds)
    {
        return set_anchor_bounds(s, p, ids, b);
    }
    let d = &s.doc()?.doc;
    // Sizes are measured square to the bounding box, which a rotation turns with the objects:
    // the math below runs in its frame.
    let frame = d.oriented_bounds(&ids, false).ok_or_else(|| EngineError::Other("no bounds".into()))?;
    let g = frame.rect;
    // With Use Preview Bounds the values measure the visual box, whose margins around the
    // geometric box (left, top, right, bottom) scale with the strokes or stay. The margins of
    // `vs`, the visual box without the strokes in symbols' art, scale; those strokes keep their
    // weight when an instance scales, so the rest of the margins stay.
    let (v, vs) = if s.prefs.use_preview_bounds {
        (d.oriented_bounds(&ids, true).map_or(g, |b| b.rect), scaling_box(d, &ids, &frame).unwrap_or(g))
    } else {
        (g, g)
    };
    let margins = |b: Rect| [g.x0 - b.x0, g.y0 - b.y0, b.x1 - g.x1, b.y1 - g.y1];
    let (m, all) = (margins(vs), margins(v));
    let fixed = [all[0] - m[0], all[1] - m[1], all[2] - m[2], all[3] - m[3]];
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4) as usize;
    let rp = vectorcraft_geom::reference_point(v, refi);
    let (w, h) = wanted_size(p, v);
    // The geometric scale that gives the visual size: margins grow by the strokes' scale `f`. It is
    // exact for proportional scales of one path; [`Spans::refit`] checks it, and solves for the
    // scales where it misses.
    let strokes = bool_or(p, "strokes", s.prefs.scale_strokes);
    let ratio = |size: f64, geo: f64, margin: f64| if geo > 1e-9 { (size - margin) / geo } else { 1.0 };
    let f = |sx: f64, sy: f64| if strokes { (sx * sy).abs().sqrt() } else { 1.0 };
    // The strokes' scale `f0`: the size left after the margins in symbols' art, which stay, over
    // `vs`, whose margins scale.
    let (lx, ly) = (ratio(w - fixed[0] - fixed[2], vs.width(), 0.0), ratio(h - fixed[1] - fixed[3], vs.height(), 0.0));
    let f0 = f(if g.width() > 1e-9 { lx } else { 1.0 }, if g.height() > 1e-9 { ly } else { 1.0 });
    // When a size of 0 or more is smaller than the margins and some of them stay, the geometry
    // gets no width or height: the visual box is then as small as it can be. The margins in
    // symbols' art stay, and so does every margin when strokes do not scale.
    let fit = |size: f64, geo: f64, scaling: f64, kept: f64| {
        let s = ratio(size, geo, scaling * f0 + kept);
        let stays = if strokes { kept } else { scaling + kept };
        if s < 0.0 && size >= 0.0 && stays > 0.0 { 0.0 } else { s }
    };
    let sx = fit(w, g.width(), m[0] + m[2], fixed[0] + fixed[2]);
    let sy = fit(h, g.height(), m[1] + m[3], fixed[1] + fixed[3]);
    let scale = about(rp, Affine::scale_non_uniform(sx, sy));
    // Move the new visual box's reference point to x, y (default: where it was): the geometric
    // box's reference point moves with the art, the margin from it to `vs`'s by `f`, and the
    // margin from that one to the visual box's stays.
    let gp = vectorcraft_geom::reference_point(g, refi);
    let sp = vectorcraft_geom::reference_point(vs, refi);
    let new_rp = scale * gp + (sp - gp) * f(sx, sy) + (rp - sp);
    // x, y place the reference point on the page.
    let page = frame.to_doc() * rp;
    // When every value given is the current one, the transform is the identity. Computing it can
    // leave a rounding error in it on a rotated object or with Use Preview Bounds.
    let current = |k: &str, now: f64| p.get(k).and_then(Value::as_f64).is_none_or(|x| x == now);
    if current("x", page.x) && current("y", page.y) && current("width", v.width()) && current("height", v.height()) {
        return apply_transform(s, "Transform", ids, Affine::IDENTITY, p, false);
    }
    let to = frame.to_local(Point::new(f64_or(p, "x", page.x), f64_or(p, "y", page.y)));
    // The scales and the point above can miss the size or the reference point when the margins of
    // the objects differ, the members of groups and the objects in symbols' art included, and with
    // strokes that scale, when the sizes are out of proportion. The objects are then measured one
    // by one ([`Spans::measure`]), and the scales solved for.
    let solved = if s.prefs.use_preview_bounds && w >= 0.0 && h >= 0.0 {
        Spans::measure(d, &ids, &frame, strokes).refit(rp, refi, [w, h], (sx, sy, new_rp))
    } else {
        None
    };
    let (scale, new_rp) = solved.map_or((scale, new_rp), |(sx, sy, at)| (about(rp, Affine::scale_non_uniform(sx, sy)), at));
    apply_transform(s, "Transform", ids, frame.conjugate(Affine::translate(to - new_rp) * scale), p, false)
}

/// The `width` and `height` `object.setBounds` asks for a box now `b`: the one not given stays, or
/// with `proportional` follows the one given.
fn wanted_size(p: &Value, b: Rect) -> (f64, f64) {
    let mut w = f64_or(p, "width", b.width());
    let mut h = f64_or(p, "height", b.height());
    if bool_or(p, "proportional", false) {
        if p.get("width").is_some() && b.width() > 0.0 {
            h = b.height() * w / b.width();
        } else if p.get("height").is_some() && b.height() > 0.0 {
            w = b.width() * h / b.height();
        }
    }
    (w, h)
}

/// `object.setBounds` with anchors selected with Direct Selection (#1824): the values are those of
/// `b`, the box around the anchors (and the objects selected whole) square to the page, and only
/// the anchors, their handles with them, and those objects move or scale.
fn set_anchor_bounds(s: &mut Session, p: &Value, ids: Vec<NodeId>, b: Rect) -> Result<Value> {
    let refi = p.get("reference").and_then(Value::as_u64).unwrap_or(4) as usize;
    let rp = vectorcraft_geom::reference_point(b, refi);
    let (w, h) = wanted_size(p, b);
    // A single anchor, or anchors in a line, have no size to scale on that side.
    let ratio = |size: f64, now: f64| if now > 1e-9 { size / now } else { 1.0 };
    let scale = about(rp, Affine::scale_non_uniform(ratio(w, b.width()), ratio(h, b.height())));
    let to = Point::new(f64_or(p, "x", rp.x), f64_or(p, "y", rp.y));
    apply_transform(s, "Transform", ids, Affine::translate(to - rp) * scale, p, true)
}

/// The visual box of `ids` square to `frame`, as [`Document::oriented_bounds`] measures it, with
/// symbol instances measured without the strokes in their art
/// ([`vectorcraft_doc::SymbolBoxes::without_art_strokes`]).
fn scaling_box(d: &Document, ids: &[NodeId], frame: &OrientedBox) -> Option<Rect> {
    let symbols = d.symbol_boxes().without_art_strokes();
    let to_local = frame.to_doc().inverse();
    ids.iter().filter_map(|id| d.node(*id)).fold(None, |acc, n| {
        let b = if frame.angle == 0.0 { n.visual_bounds_with(&symbols) } else { n.bounds_in(&symbols, to_local, true) };
        vectorcraft_geom::union_opt(acc, b)
    })
}

/// One axis of an object that [`set_bounds`] sizes, square to its frame: the geometric box runs
/// from `lo` to `hi`, and the visual box reaches past each end (before, after) by a margin that
/// scales with the strokes and one that stays.
#[derive(Clone, Copy, Debug)]
struct Span {
    lo: f64,
    hi: f64,
    scaling: [f64; 2],
    kept: [f64; 2],
}

impl Span {
    /// The ends of the visual box after a scale by `s` about `c`, with the margins that scale
    /// grown by `k`. A negative scale turns the box over, and its margins with it.
    fn scaled(self, c: f64, s: f64, k: f64) -> (f64, f64) {
        let ([sb, sa], [kb, ka]) = (self.scaling, self.kept);
        let (before, after) = (sb * k + kb, sa * k + ka);
        let (a, b) = (c + s * (self.lo - c), c + s * (self.hi - c));
        if s < 0.0 { (b - after, a + before) } else { (a - before, b + after) }
    }
}

/// The objects that [`set_bounds`] sizes, measured one by one square to its frame.
struct Spans {
    x: Vec<Span>,
    y: Vec<Span>,
    /// Whether strokes scale. The margins that scale grow by the square root of the product of the
    /// two scales; with strokes that do not scale, every margin stays.
    strokes: bool,
}

impl Spans {
    /// The objects of `ids` that have a box, measured one by one as [`Document::oriented_bounds`]
    /// measures them square to `frame`, each plain group member by member and each symbol instance
    /// that [`open`] opens object by object in its art. Past [`MAX_SPANS`] parts, each object of
    /// `ids` is measured as one. The margins of the visual box without the strokes in symbols' art
    /// ([`scaling_box`]) scale with the strokes, and the rest stay. With `strokes` false, every
    /// margin stays.
    fn measure(d: &Document, ids: &[NodeId], frame: &OrientedBox, strokes: bool) -> Self {
        let (symbols, without, art) = (d.symbol_boxes(), d.symbol_boxes().without_art_strokes(), d.symbol_boxes().quick());
        let to_local = frame.to_doc().inverse();
        let measure = |n: &Node, s: &vectorcraft_doc::SymbolBoxes, visual: bool| match (frame.angle == 0.0, visual) {
            (true, false) => n.geometric_bounds_with(s),
            (true, true) => n.visual_bounds_with(s),
            (false, _) => n.bounds_in(s, to_local, visual),
        };
        let roots: Vec<&Node> = ids.iter().filter_map(|id| d.node(*id)).collect();
        let mut parts = vec![];
        if !roots.iter().all(|n| open(n, 0.0, d, &symbols, to_local, &mut parts)) {
            parts = roots.iter().map(|n| Part::Object(n, 0.0)).collect();
        }
        let mut out = Self { x: vec![], y: vec![], strokes };
        for part in parts {
            let (g, v, vs) = match part {
                Part::Object(n, grow) => {
                    let Some(g) = measure(n, &symbols, false) else { continue };
                    let v = measure(n, &symbols, true).unwrap_or(g).inflate(grow, grow);
                    let vs = if strokes { measure(n, &without, true).unwrap_or(g).inflate(grow, grow) } else { g };
                    (g, v, vs)
                }
                // Measured as the instance's own box measures its art (`Node::visual_bounds_with`):
                // the strokes at their own weight, and the instances in it by their art boxes.
                Part::Art(n, xf, grow, kept) => {
                    let mut placed = n.clone();
                    placed.transform(xf, false);
                    let Some(g) = placed.geometric_bounds_with(&art) else { continue };
                    let v = placed.visual_bounds_with(&art).map_or(g, |v| v.union(g)).inflate(kept + grow, kept + grow);
                    let vs = if strokes { g.inflate(grow, grow) } else { g };
                    (g, v, vs)
                }
            };
            out.x.push(Span { lo: g.x0, hi: g.x1, scaling: [g.x0 - vs.x0, vs.x1 - g.x1], kept: [vs.x0 - v.x0, v.x1 - vs.x1] });
            out.y.push(Span { lo: g.y0, hi: g.y1, scaling: [g.y0 - vs.y0, vs.y1 - g.y1], kept: [vs.y0 - v.y0, v.y1 - vs.y1] });
        }
        out
    }

    /// The visual box after a scale by `sx`, `sy` about `c`.
    fn visual(&self, c: Point, sx: f64, sy: f64) -> Option<Rect> {
        let k = if self.strokes { (sx * sy).abs().sqrt() } else { 1.0 };
        let boxes = self.x.iter().zip(&self.y).map(|(x, y)| {
            let ((x0, x1), (y0, y1)) = (x.scaled(c.x, sx, k), y.scaled(c.y, sy, k));
            Rect::new(x0, y0, x1, y1)
        });
        boxes.reduce(|a, b| a.union(b))
    }

    /// The scales along x and y that give the visual box the size `size` with a scale about `c`,
    /// or come as near to it as the margins that stay allow ([`Self::solve`]), and the point that
    /// goes to the reference point's place: point `refi` of the visual box they give. Returns
    /// `None`, which keeps `closed` (the scales and the point of [`set_bounds`]'s closed form), when
    /// the scales of `closed` put point `refi` at the point of `closed` and either give the size
    /// `size` or are the scales [`Self::solve`] finds, to within rounding (1e-12 of the largest
    /// value compared, or of 1).
    fn refit(&self, c: Point, refi: usize, size: [f64; 2], closed: (f64, f64, Point)) -> Option<(f64, f64, Point)> {
        let ([w, h], (sx, sy, at)) = (size, closed);
        let b = self.visual(c, sx, sy)?;
        let p = vectorcraft_geom::reference_point(b, refi);
        let tolerance = 1e-12 * [b.x0, b.y0, b.x1, b.y1, w, h, at.x, at.y].iter().fold(1.0, |m: f64, v| m.max(v.abs()));
        let near = |a: f64, b: f64| (a - b).abs() <= tolerance;
        let placed = near(p.x, at.x) && near(p.y, at.y);
        if placed && near(b.width(), w) && near(b.height(), h) {
            return None;
        }
        let (tx, ty) = self.solve(w, h);
        let same = |a: f64, b: f64| (a - b).abs() <= 1e-12 * a.abs().max(b.abs()).max(1.0);
        if placed && same(tx, sx) && same(ty, sy) {
            return None;
        }
        let b = self.visual(c, tx, ty)?;
        Some((tx, ty, vectorcraft_geom::reference_point(b, refi)))
    }

    /// The scales of 0 or more along x and y that give the visual box the size `w` × `h`, each
    /// found by [`scale_across`]. With strokes that scale, the margins that scale grow by `k`,
    /// found where it is the square root of the product of the two scales.
    fn solve(&self, w: f64, h: f64) -> (f64, f64) {
        let at = |k: f64| (scale_across(&self.x, k, w), scale_across(&self.y, k, h));
        if !self.strokes {
            return at(1.0);
        }
        // The scales get smaller as `k` grows, so the square root of their product minus `k` is at
        // least 0 at `k` 0 and at most 0 at `k` equal to that root. Bisection finds the `k` between
        // the two where it is 0.
        let (sx, sy) = at(0.0);
        let (mut lo, mut hi) = (0.0, (sx * sy).sqrt());
        if hi.is_nan() || hi <= 0.0 {
            return (sx, sy);
        }
        for _ in 0..200 {
            let mid = lo + (hi - lo) / 2.0;
            if mid <= lo || mid >= hi {
                break;
            }
            let (sx, sy) = at(mid);
            if (sx * sy).sqrt() > mid {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let miss = |k: f64| {
            let (sx, sy) = at(k);
            ((sx * sy).sqrt() - k).abs()
        };
        at(if miss(lo) < miss(hi) { lo } else { hi })
    }
}

/// Most parts [`Spans::measure`] opens the objects it sizes into: past it, each object is one part.
const MAX_SPANS: usize = 4096;

/// What [`Spans::measure`] measures as one span.
enum Part<'a> {
    /// An object of the selection, or a member of a plain group in it, and the outset of the
    /// strokes of the plain groups around it, which scale with its own.
    Object(&'a Node, f64),
    /// An object in the art of a symbol instance and the transform that places it square to the
    /// frame. The outset of the strokes of the instance and of the plain groups around the instance
    /// scales with the strokes; the outset of the strokes of the plain groups around the object in
    /// the art stays, as the strokes in the art do.
    Art(&'a Node, Affine, f64, f64),
}

/// Adds the parts that [`Spans::measure`] measures `n` by to `parts`, `grow` being the outset of
/// the strokes of the plain groups around it ([`Part::Object`]). A plain group ([`opens`]) adds the
/// parts of each member. A symbol instance without effects or an opacity mask of its own adds the
/// objects of its art, with plain groups there opened too ([`Part::Art`]), when its symbol's art
/// measures it, it is not in perspective, and its transform keeps the art square to the frame (a
/// turned or slanted instance is measured by the box around its turned art box). Anything else is
/// one part. Returns false when the parts pass [`MAX_SPANS`].
fn open<'a>(n: &'a Node, grow: f64, d: &'a Document, symbols: &vectorcraft_doc::SymbolBoxes, to_local: Affine, parts: &mut Vec<Part<'a>>) -> bool {
    if opens(n) {
        match &n.kind {
            NodeKind::Group { children, .. } => {
                let grow = grow + n.appearance.outset();
                return children.iter().all(|c| open(c, grow, d, symbols, to_local, parts));
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let xf = to_local * *xf;
                if n.projection().is_none()
                    && square(xf)
                    && symbols.art_bounds(symbol).is_some()
                    && let Some(art) = d.symbols.iter().find(|s| s.name == *symbol)
                {
                    return open_art(&art.art, xf, grow + n.appearance.outset(), 0.0, parts);
                }
            }
            _ => {}
        }
    }
    parts.push(Part::Object(n, grow));
    parts.len() <= MAX_SPANS
}

/// [`open`] for `n` in symbol art that `xf` places square to the frame: a plain group adds the parts
/// of each member, and anything else, a symbol instance included, is one part. `kept` is the outset
/// of the strokes of the plain groups around `n` in the art, and `grow` that of the instance and the
/// plain groups around it.
fn open_art<'a>(n: &'a Node, xf: Affine, grow: f64, kept: f64, parts: &mut Vec<Part<'a>>) -> bool {
    if let NodeKind::Group { children, .. } = &n.kind
        && opens(n)
    {
        let kept = kept + n.appearance.outset();
        return children.iter().all(|c| open_art(c, xf, grow, kept, parts));
    }
    parts.push(Part::Art(n, xf, grow, kept));
    parts.len() <= MAX_SPANS
}

/// Whether [`open`] opens `n`: a group without a clip, a Shaper construction, effects or an opacity
/// mask of its own, or a symbol instance without effects or an opacity mask of its own.
fn opens(n: &Node) -> bool {
    let visible = |effects: &[vectorcraft_doc::Effect]| effects.iter().any(|e| e.visible);
    let effects = visible(&n.appearance.effects) || n.appearance.items.iter().any(|i| visible(i.effects()));
    let kind = match n.kind {
        NodeKind::Group { clip, .. } => !clip && n.extras().shaper.is_none(),
        NodeKind::SymbolInstance { .. } => true,
        _ => false,
    };
    kind && !effects && n.extras().mask.is_none()
}

/// Whether `a` takes boxes square to the frame to boxes square to it: it scales, mirrors, turns by a
/// quarter turn and moves, and nothing else, to within rounding.
fn square(a: Affine) -> bool {
    let [m0, m1, m2, m3, ..] = a.as_coeffs();
    let tolerance = 1e-9 * [m0, m1, m2, m3].iter().fold(0.0, |m: f64, v| m.max(v.abs()));
    m1.abs() <= tolerance && m2.abs() <= tolerance || m0.abs() <= tolerance && m3.abs() <= tolerance
}

/// The largest scale of 0 or more at which `spans`, with the margins that scale grown by `k`,
/// reach across `size` together. Returns 1 when their geometric boxes reach 1e-9 or less across
/// together, as the closed form in [`set_bounds`] does, and 0 when the spans reach farther than
/// `size` at the scale 0.
fn scale_across(spans: &[Span], k: f64, size: f64) -> f64 {
    let o = spans.iter().map(|s| s.lo).fold(f64::INFINITY, f64::min);
    let across = spans.iter().map(|s| s.hi).fold(f64::NEG_INFINITY, f64::max) - o;
    // How far the spans reach together after a scale by `s` about `o`, and how fast that changes
    // with `s`: the far geometric end minus the near one, of the objects that reach farthest at the
    // two ends.
    let reach = |s: f64| {
        let (mut start, mut end, mut slope_start, mut slope_end) = (f64::INFINITY, f64::NEG_INFINITY, 0.0, 0.0);
        for sp in spans {
            let (a, b) = sp.scaled(o, s, k);
            if a < start {
                (start, slope_start) = (a, sp.lo - o);
            }
            if b > end {
                (end, slope_end) = (b, sp.hi - o);
            }
        }
        (end - start, slope_end - slope_start)
    };
    if across.is_nan() || across <= 1e-9 {
        return 1.0;
    }
    let least = reach(0.0).0;
    if least.is_nan() || least > size {
        return 0.0;
    }
    // `reach` is convex in `s` and at most `size` at 0, so it equals `size` at one largest scale.
    // Newton's method, started at a larger scale, reaches it one linear piece of `reach` at a
    // time; bisection finishes the search when a step makes no progress.
    let (mut lo, mut hi) = (0.0, size / across);
    for _ in 0..64 {
        if reach(hi).0 >= size {
            break;
        }
        hi = 2.0 * hi + 1.0;
    }
    for _ in 0..64 {
        let (r, slope) = reach(hi);
        if r <= size {
            return hi;
        }
        let next = hi - (r - size) / slope;
        if next.is_nan() || next <= lo || next >= hi {
            break;
        }
        hi = next;
    }
    for _ in 0..64 {
        let mid = lo + (hi - lo) / 2.0;
        if mid <= lo || mid >= hi {
            break;
        }
        if reach(mid).0 <= size {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    if size - reach(lo).0 <= reach(hi).0 - size { lo } else { hi }
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
        match live.as_deref() {
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
        let Some(c) = LiveCorners::new(path, live.as_deref()) else { return true };
        let picked = corners_of(*id).unwrap_or_else(|| c.picked(st.selection.anchors.get(id)));
        // Setting corners folds a scale left in a rectangle's transform into its size, which
        // changes it.
        live.as_deref().is_none_or(|l| l.folded() == *l)
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
            if let Some(l @ LiveShape::Polygon { .. }) = live.as_deref_mut()
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
            if let Some(l @ LiveShape::Ellipse { .. }) = live.as_deref_mut()
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
            let Some(c) = LiveCorners::new(path, live.as_deref()) else { continue };
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
                && let Some(c) = LiveCorners::new(path, live.as_deref())
                && c.sources() != layout
            {
                *anchors = c.anchors_of(&selected);
            }
        }
        Ok(())
    })?;
    ok()
}
