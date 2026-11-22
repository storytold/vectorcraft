use super::align::choice;
use super::{
    Affine, Arc, Document, EngineError, LastTransform, Node, NodeId, NodeKind, OrientedBox, Point, Rect, Result, Scaling, Session, Value, Vec2, bad,
    bool_or, check_numbers, check_switches, duplicate_beside, effects, f64_or, f64_req, ids_param, json, matrix_param, ok, outermost, point_param,
    selected_roots_to_transform,
};

fn origin_of(s: &Session, p: &Value, ids: &[NodeId]) -> Result<Point> {
    if let Some(o) = point_param(p, "origin") {
        return Ok(o);
    }
    // Direct-selected anchors transform alone (see [`apply_transform`]), about their own centre.
    let st = s.doc()?;
    if !bool_or(p, "copy", false)
        && let Some(b) = effects::preview::anchor_bounds_with(&st.selection, &st.doc, s.bounds_measure())
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

/// `xf` on `n` with Transform Objects (`objects`, see [`transforms_objects`]) on or off. The objects
/// in `n` that are aligned to the pixel grid snap to it again ([`super::pixelgrid::keep_pixel_aligned`]).
pub(crate) fn transform_node(n: &mut Node, xf: Affine, sc: Scaling, objects: bool) {
    if objects {
        n.transform(xf, sc);
        super::pixelgrid::keep_pixel_aligned(n, sc);
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
    apply_transform_moving_guides(s, label, ids, xf, p, anchors, Placing::default())
}

/// Where [`apply_transform_moving_guides`] puts the selected ruler guides and the copies.
#[derive(Clone, Copy, Default)]
struct Placing {
    /// The selected ruler guides move by this instead of by the transform's translation.
    guides: Option<Vec2>,
    /// Each copy goes directly below its original instead of above it.
    below: bool,
}

/// [`apply_transform`], with the selected ruler guides moved by `placing.guides` instead of by the
/// translation `xf` when given (a nudge along a turned Constrain Angle moves the art along it and
/// the guides along the page's axes), and with each copy directly below its original with
/// `placing.below`.
fn apply_transform_moving_guides(
    s: &mut Session,
    label: &str,
    ids: Vec<NodeId>,
    xf: Affine,
    p: &Value,
    anchors: bool,
    placing: Placing,
) -> Result<Value> {
    let Placing { guides, below } = placing;
    let copy = bool_or(p, "copy", false);
    let areas = bool_or(p, "typeAreas", false);
    let objects = transforms_objects(p);
    let mut sc = if Scaling::factor(xf).is_some() { scaling(s, p) } else { Scaling::default() };
    sc.patterns = transform_patterns(s, p, &ids)?;
    let c = xf.as_coeffs();
    let is_translation = (c[0] - 1.0).abs() < 1e-9 && c[1].abs() < 1e-9 && c[2].abs() < 1e-9 && (c[3] - 1.0).abs() < 1e-9;
    let delta = guides.unwrap_or(Vec2::new(c[4], c[5]));
    let with_guides = objects && p.get("ids").is_none_or(Value::is_null) && is_translation;
    let has_guides = with_guides && !s.doc()?.selection.guides.is_empty();
    // A transform of ids that name no object, or an identity transform without a copy, moves
    // nothing: the document and its undo history stay as they are, and Transform Again repeats it.
    // With a copy, the reply lists the copies, and there are none.
    // An edit normally preserves the tree positions already found here. Settling an active
    // typing session can change the document before the edit closure runs, so look up again then.
    let reindex = copy || (!s.previewing && crate::cmd::typing_in_progress(s));
    let d = &s.doc()?.doc;
    let paths = d.index_paths(ids.iter().copied());
    let none = paths.is_empty() && !has_guides;
    // A move of direct-selected anchors may re-map the handles beside them (`interpolate`).
    let mut interpolate = None;
    let ids = if anchors && objects && !copy && !s.doc()?.selection.anchors.is_empty() {
        if c[..4] == [1.0, 0.0, 0.0, 1.0] {
            interpolate = Some(crate::cmd::path::interpolates_handles(s, p, label)?);
        }
        if xf != Affine::IDENTITY {
            crate::cmd::path::transform_anchors(s, label, xf, sc, interpolate.unwrap_or(false))?;
        }
        ids
    } else if none || (!copy && xf == Affine::IDENTITY) {
        if copy { vec![] } else { ids }
    } else {
        let kept = if copy { preview_copies(s, &ids, with_guides, below)? } else { None };
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
                        let copies: Vec<_> = sel.guides.iter().filter_map(|&i| d.ruler_guide(i)).collect();
                        (!copies.is_empty()).then_some(copies)
                    } else {
                        None
                    };
                    let new_ids = duplicate_beside(d, sel, &ids, Affine::IDENTITY, below)?;
                    if let Some(copies) = guide_copies {
                        sel.guides = copies.iter().map(|g| d.add_ruler_guide(g)).collect();
                    }
                    new_ids
                }
                None => ids.clone(),
            };
            // Pattern editing with Move Tile with Art: the tile art's bounds before a move.
            let moves_only = c[0] == 1.0 && c[1] == 0.0 && c[2] == 0.0 && c[3] == 1.0;
            let tile_art = if objects && !copy && moves_only { crate::cmd::patterncmds::tile_art_bounds(d) } else { None };
            // Transforms don't move nodes in the tree, so reuse the paths found for the no-copy
            // case; copies and a settled typing session need paths from the edited document.
            let paths = if reindex { d.index_paths(targets.iter().copied()) } else { paths };
            for id in &targets {
                if let Some(n) = paths.get(id).and_then(|p| d.node_mut_at(p))
                    && !(objects && areas && resize_type_area(n, xf))
                {
                    transform_node(n, xf, sc, objects);
                }
            }
            if objects {
                for id in &targets {
                    super::pixelgrid::keep_compound_aligned(d, *id, sc);
                }
                // Moving all of the tile art moves the tile as far.
                if let Some(before) = tile_art {
                    crate::cmd::patterncmds::move_tile_with_art(d, &targets, before);
                }
            }
            if with_guides && (delta.x != 0.0 || delta.y != 0.0) {
                for i in sel.guides.clone() {
                    d.edit_ruler_guide(i, |g| *g = g.moved(delta));
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
        if let Some(on) = interpolate {
            options["interpolate"] = json!(on);
        }
        st.last_transform = Some(LastTransform::Matrix(xf, options));
        st.last_perspective = None;
    }
    Ok(json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// Returns the copies of `roots` for a transform or a perspective edit with `copy` in an interaction
/// whose document is still its snapshot, as in a preview ([`Session::preview`] puts the snapshot
/// back first). These are the copies the interaction keeps when they were made of that snapshot for
/// the same `roots` on the same side (`below`: directly below each original); otherwise this copies
/// `roots` with [`duplicate_beside`] in a clone of the snapshot, untransformed, and the interaction
/// keeps that clone. It returns `None` outside an interaction,
/// and when the document is no longer the snapshot, as for a command run in a typing session or a
/// script batch after an earlier edit there: the command then copies the current document.
pub(crate) fn preview_copies(s: &mut Session, roots: &[NodeId], with_guides: bool, below: bool) -> Result<Option<Arc<crate::PreviewCopies>>> {
    let st = s.doc_mut()?;
    let Some(it) = &mut st.interaction else { return Ok(None) };
    if !Arc::ptr_eq(&st.doc, &it.doc) {
        return Ok(None);
    }
    let has_guide_copies = with_guides && !st.selection.guides.is_empty();
    if let Some(k) = &it.copies
        && Arc::ptr_eq(&k.from, &it.doc)
        && k.roots == roots
        && k.below == below
        && (k.selection.guides.is_empty() != has_guide_copies)
    {
        return Ok(Some(k.clone()));
    }
    let mut doc = it.doc.clone();
    let mut selection = st.selection.clone();
    let ids = duplicate_beside(Arc::make_mut(&mut doc), &mut selection, roots, Affine::IDENTITY, below)?;
    if has_guide_copies {
        let d = Arc::make_mut(&mut doc);
        let copies: Vec<_> = st.selection.guides.iter().filter_map(|&i| d.ruler_guide(i)).collect();
        selection.guides = copies.iter().map(|g| d.add_ruler_guide(g)).collect();
    }
    let k = Arc::new(crate::PreviewCopies { from: it.doc.clone(), roots: roots.to_vec(), below, doc, selection, ids });
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
    check_switches(p, &["anchors", "releaseOutside", "interpolate", "drawBehind"], "object.transform")?;
    // An Alt-drag copy in Draw Behind goes directly below its original.
    let below = bool_or(p, "drawBehind", false) && bool_or(p, "copy", false) && s.draw_mode == crate::DrawMode::Behind;
    let m = matrix_param(p, "matrix").ok_or_else(|| bad("object.transform", "missing matrix [a,b,c,d,e,f]"))?;
    // An object selected in a symbol instance's art stays where the symbol puts it: the selection
    // is left as it is, and the reply's warning goes to the status bar.
    if p.get("ids").is_none() && crate::cmd::brushsym::selected_part(s)?.is_some() {
        return Ok(json!({ "warning": crate::cmd::brushsym::part_only().to_string() }));
    }
    let (ids, anchors) = match ids_param(p, "ids", "object.transform")? {
        Some(v) => (outermost(&s.doc()?.doc, v), false),
        None => (selected_roots_to_transform(s)?, bool_or(p, "anchors", false)),
    };
    // Shaper's Construction mode: an original shape moved wholly outside its group's bounds leaves
    // the group.
    let moved = if bool_or(p, "releaseOutside", false) && !bool_or(p, "copy", false) && !anchors {
        crate::cmd::shaper::moving_sources(s, &ids)?
    } else {
        vec![]
    };
    // Outside a drag and an undo group, the move and the removal are one undo step, named for the
    // removal when there is one.
    let grouped = !moved.is_empty() && s.doc()?.interaction.is_none() && s.doc()?.undo_group.is_none();
    if grouped {
        s.begin_named_undo_group(None);
    }
    let r = apply_transform_moving_guides(s, "Transform", ids, m, p, anchors, Placing { guides: None, below });
    let released = match &r {
        Ok(_) if !moved.is_empty() => crate::cmd::shaper::release_outside(s, &moved),
        _ => Ok(false),
    };
    if grouped && let Some(st) = s.active_mut() {
        st.undo_group = None;
        if matches!(released, Ok(true))
            && let Some(e) = st.history.undo.last_mut()
        {
            e.label = "Remove from Shaper Group".into();
        }
    }
    released?;
    r
}

pub(super) fn move_cmd(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["dx", "dy"], "object.move")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.move")?;
    check_switches(p, &["interpolate"], "object.move")?;
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
    check_switches(p, &["big", "interpolate"], "object.nudge")?;
    let k = s.prefs.keyboard_increment * if bool_or(p, "big", false) { 10.0 } else { 1.0 };
    let (dx, dy) = (f64_or(p, "dx", 0.0) * k, f64_or(p, "dy", 0.0) * k);
    // Guides keep to the page's axes; art and anchors move along the constrain angle's.
    let (gx, gy) = (dx, dy);
    let (dx, dy) = along_constrain_angle(dx, dy, s.prefs.constrain_angle);
    let objects = transforms_objects(p);
    if objects && !s.doc()?.selection.anchors.is_empty() {
        // When the anchors move by 0, the document and its undo history stay as they are.
        if dx == 0.0 && dy == 0.0 {
            return ok();
        }
        return crate::cmd::path::move_anchors(s, &json!({ "dx": dx, "dy": dy, "patterns": p.get("patterns"), "interpolate": p.get("interpolate") }));
    }
    if objects && s.doc()?.selection.is_empty() {
        let copy = bool_or(p, "copy", false);
        // When guides move by 0 without a copy, the document and its undo history stay as they are.
        if gx == 0.0 && gy == 0.0 && !copy {
            return ok();
        }
        return crate::cmd::docmenu::guide_move(s, &json!({ "dx": gx, "dy": gy, "copy": copy }));
    }
    let ids = selected_roots_to_transform(s)?;
    if !objects && !bool_or(p, "copy", false) {
        // Pattern-only nudges of art without a pattern change nothing, including undo history.
        if !bool_or(p, "patterns", s.prefs.transform_pattern_tiles) {
            return ok();
        }
        let st = s.doc()?;
        let paths = st.doc.index_paths(ids.iter().copied());
        let mut uses_patterns = false;
        for node in ids.iter().filter_map(|id| paths.get(id)).filter_map(|path| st.doc.node_at(path)) {
            node.walk(&mut |n| uses_patterns |= vectorcraft_doc::pattern::uses_pattern(n, None));
        }
        if !uses_patterns {
            return ok();
        }
    }
    apply_transform_moving_guides(s, "Move", ids, Affine::translate((dx, dy)), p, true, Placing { guides: Some(Vec2::new(gx, gy)), below: false })
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
    let sx = f64_req(p, "sx", "object.scale")?;
    let (sx, sy) = scale_factors(sx, f64_or(p, "sy", sx)).map_err(|e| bad("object.scale", e))?;
    let frame = basis_frame(s, p, "object.scale", &ids)?;
    apply_transform(s, "Scale", ids, about(o, frame.conjugate(Affine::scale_non_uniform(sx, sy))), p, true)
}

/// The factors `object.scale` applies for `sx` and `sy` in percent, or the error it returns when
/// either factor is 0.
pub fn scale_factors(sx: f64, sy: f64) -> std::result::Result<(f64, f64), String> {
    let (sx, sy) = (sx / 100.0, sy / 100.0);
    if sx == 0.0 || sy == 0.0 {
        return Err("scale must be non-zero".into());
    }
    Ok((sx, sy))
}

/// The frame that `object.scale` and a named `object.reflect` act in, from the `basis` param:
/// `"box"` (the default) is the bounding box of `ids`, turned with rotated objects until Reset
/// Bounding Box ([`Document::bbox_angle`]); `"page"` is the page's axes. Anchors selected with
/// Direct Selection (with no copy and Transform Objects on) transform along the page's axes, as
/// their own box does. The basis joins the journal entry, so a replay acts in the same frame. A
/// saved step without `basis` ([`Session::replay`]) was recorded before Scale and Reflect followed
/// the box, so it acts along the page's axes, as it did when it was recorded.
fn basis_frame(s: &mut Session, p: &Value, cmd: &str, ids: &[NodeId]) -> Result<OrientedBox> {
    let basis = choice(p, cmd, "basis", &["box", "page"])?;
    let page = basis == Some("page") || (basis.is_none() && s.replaying());
    let st = s.doc()?;
    let anchors = !bool_or(p, "copy", false)
        && transforms_objects(p)
        && effects::preview::anchor_bounds_with(&st.selection, &st.doc, s.bounds_measure()).is_some();
    let angle = if page || anchors { 0.0 } else { st.doc.bbox_angle(ids) };
    s.note_journal("basis", json!(if page { "page" } else { "box" }));
    Ok(OrientedBox { angle, rect: Rect::ZERO })
}

pub(super) fn reflect(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &TRANSFORM_SWITCHES, "object.reflect")?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    // A named axis is the bounding box's own (see [`basis_frame`]); an angle is a line on the page.
    let named = reflect_axis(p.get("axis")).map_err(|e| bad("object.reflect", e))?;
    let m = match named {
        Some(m) => basis_frame(s, p, "object.reflect", &ids)?.conjugate(m),
        None => {
            choice(p, "object.reflect", "basis", &["box", "page"])?;
            let t = -p.get("axis").and_then(Value::as_f64).unwrap_or(90.0).to_radians();
            Affine::rotate(t) * Affine::scale_non_uniform(1.0, -1.0) * Affine::rotate(-t)
        }
    };
    apply_transform(s, "Reflect", ids, about(o, m), p, true)
}

/// The reflection `object.reflect` applies for its `axis` param: the matrix for a named axis
/// (`"vertical"`, the default, or `"horizontal"`), in the bounding box's frame; `None` for an angle,
/// which is a line on the page; or the error for any other value.
pub fn reflect_axis(axis: Option<&Value>) -> std::result::Result<Option<Affine>, String> {
    match axis {
        Some(Value::String(a)) if a == "horizontal" => Ok(Some(Affine::scale_non_uniform(1.0, -1.0))),
        Some(Value::Number(_)) => Ok(None),
        None | Some(Value::Null) => Ok(Some(Affine::scale_non_uniform(-1.0, 1.0))),
        Some(Value::String(a)) if a == "vertical" => Ok(Some(Affine::scale_non_uniform(-1.0, 1.0))),
        Some(v) => {
            let given = v.as_str().map_or_else(|| v.to_string(), str::to_string);
            Err(format!("axis must be \"vertical\", \"horizontal\" or an angle in degrees, not `{given}`"))
        }
    }
}

/// Angles closer than this many degrees to ±90 or ±270, or to 0 or ±180, count as those angles. It
/// is far above the rounding error of an angle up to 359° and below any angle a field shows.
const SHEAR_ANGLE_TOLERANCE: f64 = 1e-9;

/// The slant `object.shear` applies for `angle` in degrees: its tangent, and 0 for 0 and ±180. An
/// angle outside -359 to 359, or ±90 or ±270, is an error.
pub fn shear_tangent(angle: f64) -> std::result::Result<f64, String> {
    if !(-359.0..=359.0).contains(&angle) {
        return Err(format!("`angle` must be from -359 to 359 degrees, not {angle}"));
    }
    // ±90 and ±270 slant an edge onto the shear axis: their tangent is unbounded.
    let flat = angle.rem_euclid(180.0);
    if (flat - 90.0).abs() < SHEAR_ANGLE_TOLERANCE {
        return Err(format!("`angle` cannot be {angle}: ±90 and ±270 degrees have no shear"));
    }
    // 0 and ±180 have no slant, but the tangent of ±180° in radians is about 1e-16, not 0.
    Ok(if flat < SHEAR_ANGLE_TOLERANCE || 180.0 - flat < SHEAR_ANGLE_TOLERANCE { 0.0 } else { angle.to_radians().tan() })
}

pub(super) fn shear(s: &mut Session, p: &Value) -> Result<Value> {
    check_numbers(p, &["angle"], "object.shear")?;
    check_switches(p, &TRANSFORM_SWITCHES, "object.shear")?;
    let t = shear_tangent(f64_or(p, "angle", 0.0)).map_err(|e| bad("object.shear", e))?;
    let ids = selected_roots_to_transform(s)?;
    let o = origin_of(s, p, &ids)?;
    let horizontal = Affine::new([1.0, 0.0, -t, 1.0, 0.0, 0.0]);
    // A named axis, or a line on the page at an angle in degrees, counter-clockwise from horizontal.
    let m = match p.get("axis") {
        None | Some(Value::Null) => horizontal,
        Some(Value::String(a)) if a == "horizontal" => horizontal,
        Some(Value::String(a)) if a == "vertical" => Affine::new([1.0, t, 0.0, 1.0, 0.0, 0.0]),
        Some(Value::Number(n)) if n.as_f64().is_some_and(|a| (-SHEAR_AXIS_MAX..=SHEAR_AXIS_MAX).contains(&a)) => {
            // y is down, so the page's counter-clockwise angle turns the other way on screen.
            let r = -n.as_f64().unwrap_or(0.0).to_radians();
            Affine::rotate(r) * horizontal * Affine::rotate(-r)
        }
        Some(v) => {
            let given = v.as_str().map_or_else(|| v.to_string(), str::to_string);
            let range = format!("from -{SHEAR_AXIS_MAX} to {SHEAR_AXIS_MAX}");
            return Err(bad("object.shear", format!("axis must be \"horizontal\", \"vertical\" or an angle in degrees {range}, not `{given}`")));
        }
    };
    apply_transform(s, "Shear", ids, about(o, m), p, true)
}

/// The largest angle, either way from horizontal, of the axis `object.shear` takes in degrees.
const SHEAR_AXIS_MAX: f64 = 359.0;

/// The last transform on the selection, with the options `last_transform` keeps for it; an option
/// that `p` sets to true or false (a recorded Transform Again's) takes precedence. A Transform
/// Each is repeated by running it again (its transform is per object, about each one's bounds).
pub(super) fn transform_again(s: &mut Session, p: &Value) -> Result<Value> {
    check_switches(p, &TRANSFORM_SWITCHES, "object.transformAgain")?;
    check_switches(p, &["interpolate"], "object.transformAgain")?;
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
