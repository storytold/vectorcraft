use super::align::choice;
use super::*;

fn origin_of(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Point> {
    if let Some(o) = point_param(p, "origin") {
        return Ok(o);
    }
    // Direct-selected anchors transform alone (see [`apply_transform`]), about their own centre.
    let st = s.doc()?;
    if !bool_or(p, "copy", false)
        && let Some(b) = effects::preview::anchor_bounds(&st.selection, &st.doc, s.prefs.use_preview_bounds)
    {
        return Ok(b.center());
    }
    s.transform_box(ids).map(|b| b.center()).ok_or_else(|| EngineError::Other("selection has no bounds".into()))
}

/// What a transform command scales besides geometry: Scale Strokes & Effects and Scale Corners from
/// its `strokes` and `corners` params, else the preferences. The values join the command's journal
/// entry, so replaying it scales the same way whatever the preferences are then.
pub(crate) fn scaling(s: &mut Session, p: &Value) -> Scaling {
    let sc = scaling_options(s, p);
    s.note_journal("strokes", json!(sc.strokes));
    s.note_journal("corners", json!(!sc.keep_corners));
    sc
}

/// [`scaling`] without noting the values in the journal.
pub(super) fn scaling_options(s: &Session, p: &Value) -> Scaling {
    let strokes = bool_or(p, "strokes", s.prefs.scale_strokes);
    let corners = bool_or(p, "corners", s.prefs.scale_corners);
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

/// The on/off params of the transform commands that [`apply_transform`], [`scaling`],
/// [`transform_patterns`] and [`transforms_objects`] read. The transform commands in this file
/// check them with [`check_switches`] before they change anything.
pub(super) const TRANSFORM_SWITCHES: [&str; 6] = ["copy", "strokes", "corners", "patterns", "objects", "typeAreas"];

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
    apply_transform_moving_guides(s, label, ids, xf, p, anchors, None)
}

/// [`apply_transform`], with the selected ruler guides moved by `guides` instead of by the
/// translation `xf` when given (a nudge along a turned Constrain Angle moves the art along it and
/// the guides along the page's axes).
fn apply_transform_moving_guides(
    s: &mut Session,
    label: &str,
    ids: Vec<NodeId>,
    xf: Affine,
    p: &Value,
    anchors: bool,
    guides: Option<Vec2>,
) -> Result<Value> {
    let copy = bool_or(p, "copy", false);
    let areas = bool_or(p, "typeAreas", false);
    let objects = transforms_objects(p);
    let mut sc = if Scaling::factor(xf).is_some() { scaling(s, p) } else { Scaling::default() };
    sc.patterns = transform_patterns(s, p, &ids)?;
    let c = xf.as_coeffs();
    let is_translation = (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9;
    let delta = guides.unwrap_or(Vec2::new(c[4], c[5]));
    let with_guides = objects && ids_param(p, "ids").is_none() && is_translation;
    let has_guides = with_guides && !s.doc()?.selection.guides.is_empty();
    // A transform of ids that name no object, or an identity transform without a copy, moves
    // nothing: the document and its undo history stay as they are, and Transform Again repeats it.
    // With a copy, the reply lists the copies, and there are none.
    let d = &s.doc()?.doc;
    let none = d.index_paths(ids.iter().copied()).is_empty() && !has_guides;
    let ids = if anchors && objects && !copy && !s.doc()?.selection.anchors.is_empty() {
        if xf != Affine::IDENTITY {
            crate::cmd::path::transform_anchors(s, label, xf, sc)?;
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
    crate::cmd::typecmd::reshape_area_with(t, |t| t.transform_area(xf))
}

pub(super) fn transform(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &TRANSFORM_SWITCHES, "object.transform")?;
    check_switches(p, &["anchors"], "object.transform")?;
    let m = matrix_param(p, "matrix").ok_or_else(|| bad("object.transform", "missing matrix [a,b,c,d,e,f]"))?;
    let (ids, anchors) = match ids_param(p, "ids") {
        Some(v) => (outermost(&s.doc()?.doc, v), false),
        None => (selected_roots_to_transform(s)?, bool_or(p, "anchors", false)),
    };
    apply_transform(s, "Transform", ids, m, p, anchors)
}

pub(super) fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["dx", "dy"], "object.move")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.move")?;
    let ids = selected_roots_to_transform(s)?;
    apply_transform(s, "Move", ids, Affine::translate((f64_or(p, "dx", 0.0), f64_or(p, "dy", 0.0))), p, true)
}

/// The arrow keys' step `(dx, dy)` (page axes, y down) along the x and y axes turned by General ›
/// Constrain Angle (`deg`, counter-clockwise on the page): with 30°, → moves 30° up from the
/// horizontal, as the reference app's arrow keys follow the constrain angle (users set it to −90°
/// to nudge along a view rotated by 90°:
/// <https://community.adobe.com/t5/illustrator-discussions/arrow-keys-and-ruler-guides-not-adjusted-when-using-rotate-view/m-p/12489564>).
pub(crate) fn along_constrain_angle(dx: f64, dy: f64, deg: f64) -> (f64, f64) {
    // Whole turns are the page's axes, exactly; quarter turns are exact too.
    let Some(axes) = vectorcraft_geom::constrain_axes(deg) else { return (dx, dy) };
    let v = axes * Point::new(dx, dy);
    (v.x, v.y)
}

pub(super) fn nudge(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["dx", "dy"], "object.nudge")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.nudge")?;
    check_switches(p, &["big"], "object.nudge")?;
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let (dx, dy) = (f64_or(p, "dx", 0.0) * k, f64_or(p, "dy", 0.0) * k);
    // Guides keep to the page's axes; art and anchors move along the constrain angle's.
    let (gx, gy) = (dx, dy);
    let (dx, dy) = along_constrain_angle(dx, dy, s.prefs.constrain_angle);
    if !s.doc()?.selection.anchors.is_empty() {
        // When the anchors move by 0, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 {
            return ok();
        }
        return crate::cmd::path::move_anchors(s, &json!({ "dx": dx, "dy": dy, "patterns": p.get("patterns") }));
    }
    if s.doc()?.selection.is_empty() {
        let copy = bool_or(p, "copy", false);
        // When guides move by 0 without a copy, the document and its undo history stay as they are.
        if gx == 0.0 && gy == 0.0 && !copy {
            return ok();
        }
        return crate::cmd::docmenu::guide_move(s, &json!({ "dx": gx, "dy": gy, "copy": copy }));
    }
    let ids = selected_roots_to_transform(s)?;
    apply_transform_moving_guides(s, "Move", ids, Affine::translate((dx, dy)), p, true, Some(Vec2::new(gx, gy)))
}

pub(super) fn about(o: Point, a: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * a * Affine::translate(-o.to_vec2())
}

pub(super) fn rotate(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["angle"], "object.rotate")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.rotate")?;
    check_switches(p, &["absolute"], "object.rotate")?;
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

pub(super) fn scale(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["sy"], "object.scale")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.scale")?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    let sx = f64_req(p, "sx", "object.scale")? / 100.0;
    let sy = f64_or(p, "sy", sx * 100.0) / 100.0;
    if sx == 0.0 || sy == 0.0 {
        return Err(bad("object.scale", "scale must be non-zero"));
    }
    apply_transform(s, "Scale", ids, about(o, Affine::scale_non_uniform(sx, sy)), p, true)
}

pub(super) fn reflect(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &TRANSFORM_SWITCHES, "object.reflect")?;
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

pub(super) fn shear(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["angle"], "object.shear")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.shear")?;
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
pub(super) fn transform_again(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &TRANSFORM_SWITCHES, "object.transformAgain")?;
    if let Some(again) = s.doc()?.last_perspective.clone() {
        return crate::cmd::distortcmds::transform_again(s, &again);
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
        LastTransform::Each(params) => crate::cmd::menucmds::transform_each_again(s, &merge(params)),
    }
}
