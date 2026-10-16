//! Object → Slice, View → Hide/Lock Slices and Select → Object → Slices.
//!
//! The model is in [`vectorcraft_doc::slices`]: user slices live in [`Document::slices`], object
//! slices are objects with slice options, auto slices are laid out around them. The Object → Slice
//! commands act on the selected slices: those the Slice Selection tool (or `object.slice.select`)
//! selected and the selected objects that are object slices. Commands that take slice ids take
//! user slice ids and the ids of objects with an object slice.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::slices::valid_rect;
use vectorcraft_doc::{Appearance, CellAlign, CellVAlign, Document, Node, NodeKind, Slice, SliceKind, SliceOptions};
use vectorcraft_geom::{Rect, shapes};

use super::*;

/// The most slices one Divide Slices makes.
const MAX_PIECES: usize = 1000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.slice.make",
            "Make",
            ["Object", "Slice"],
            None,
            "{ids?} make each selected object (or ids) an object slice: its slice follows the object's bounds → {ids: the new object slices}",
            has_selection,
            make
        ),
        cmd!(
            "object.slice.release",
            "Release",
            ["Object", "Slice"],
            None,
            "{} release the selected slices: an object slice leaves its object as it was, a user slice becomes an unpainted rectangle; the objects end up selected → {ids}",
            slices_targeted,
            release
        ),
        cmd!(
            "object.slice.fromGuides",
            "Create from Guides",
            ["Object", "Slice"],
            None,
            "{} replace the user slices with the grid the ruler guides cut the artboards into (two vertical and two horizontal guides give 9) → {ids}",
            has_doc,
            from_guides
        ),
        cmd!(
            "object.slice.fromSelection",
            "Create from Selection",
            ["Object", "Slice"],
            None,
            "{ids?|id?} a user slice over the visual bounds of the selected objects (or ids), selected → {id}",
            has_selection,
            from_selection
        ),
        cmd!(
            "object.slice.duplicate",
            "Duplicate Slice",
            ["Object", "Slice"],
            None,
            "{dx?: pt (default 10), dy?: pt (default 10)} copy the selected slices as user slices with their options, offset by dx, dy; the copies are selected → {ids}",
            slices_targeted,
            duplicate
        ),
        cmd!(
            "object.slice.combine",
            "Combine Slices",
            ["Object", "Slice"],
            None,
            "{} replace the two or more selected slices with one user slice over their bounds (the first one's options; object slices are released) → {id}",
            slices_targeted,
            combine
        ),
        cmd!(
            "object.slice.divide",
            "Divide Slices…",
            ["Object", "Slice"],
            None,
            "{rows?: slices down, rowHeight?: pt per slice, columns?: slices across, columnWidth?: pt per slice} cut each selected slice into a grid of user slices (evenly by count, or by size with a smaller last one; an object slice is released); the pieces are selected → {ids}",
            slices_targeted,
            divide
        ),
        cmd!(
            "object.slice.deleteAll",
            "Delete All",
            ["Object", "Slice"],
            None,
            "{} delete every user slice and release every object slice → {count}",
            doc_has_slices,
            delete_all
        ),
        cmd!(
            "object.slice.options",
            "Slice Options…",
            ["Object", "Slice"],
            None,
            "{kind?: \"image\"|\"noImage\"|\"htmlText\" (object slices of type only), name?, url?, target?, message?, alt?: (image), text?: cell text; HTML allowed (noImage), background?: \"\"|\"matte\"|\"#rrggbb\", hAlign?: \"default\"|\"left\"|\"center\"|\"right\", vAlign?: \"default\"|\"top\"|\"middle\"|\"baseline\"|\"bottom\"} set the selected slices' options as one undo step; no options: read them → the first slice's {kind, name, url, target, message, alt, text, background, hAlign, vAlign, htmlText: whether HTML Text applies}",
            slices_targeted,
            options
        ),
        cmd!(
            "object.slice.clipToArtboard",
            "Clip to Artboard",
            ["Object", "Slice"],
            None,
            "{on?: bool} toggle (or set) Clip to Artboard: on, slices are clipped to the artboards and auto slices fill them; off, auto slices cover the art and the slices → {on}",
            has_doc,
            clip_to_artboard
        ),
        cmd!(query "view.slices.hide", "Hide Slices", ["View"], None, "{hidden?: bool} toggle (or set) whether the canvas hides the slices (session view state) → {hidden}", has_doc, hide),
        cmd!(
            query "view.slices.lock",
            "Lock Slices",
            ["View"],
            None,
            "{locked?: bool} toggle (or set) the slice lock: locked slices can't be selected or edited with the Slice Selection tool → {locked}",
            has_doc,
            lock
        ),
        cmd!(
            query "slice.list",
            "List Slices",
            [],
            None,
            "{} the slices as laid out, numbered left to right and top to bottom → {slices: [{number, id (null for auto slices), source: \"user\"|\"object\"|\"auto\", name, x, y, width, height, options, selected}], clipToArtboard, hidden, locked}",
            has_doc,
            list
        ),
        cmd!("select.object.slices", "Slices", ["Select", "Object"], None, "{} select every user and object slice → {count}", has_doc, select_all),
        cmd!(
            "object.slice.create",
            "Create Slice",
            [],
            None,
            "{x, y, width, height} a user slice over that rectangle, selected (the Slice tool's drag) → {id}",
            has_doc,
            create
        ),
        cmd!(
            "object.slice.setRect",
            "Set Slice Rectangle",
            [],
            None,
            "{id: user slice, x, y, width, height} move or resize a user slice (an object slice follows its object) → {id}",
            has_doc,
            set_rect
        ),
        cmd!(
            "object.slice.move",
            "Move Slices",
            [],
            None,
            "{slices?: [id…] (default: the selected slices), dx, dy} move the slices: user slices move, object slices move their objects → {ids}",
            has_doc,
            move_slices
        ),
        cmd!(
            "object.slice.delete",
            "Delete Slices",
            [],
            None,
            "{slices?: [id…] (default: the selected slices)} delete the user slices and release the object slices (their objects stay) → {count}",
            has_doc,
            delete
        ),
        cmd!(
            "object.slice.select",
            "Select Slices",
            [],
            None,
            "{slices?: [id…] (user slice ids or ids of objects with an object slice; none: deselect the slices), toggle?: bool (Shift: add the unselected ones, drop the selected ones)} select slices as the Slice Selection tool does; the objects are deselected → {selected: [id…]}",
            has_doc,
            select
        ),
    ]
}

impl Session {
    /// View → Hide Slices.
    pub fn slices_hidden(&self) -> bool {
        self.menu.slices_hidden
    }
    /// View → Lock Slices.
    pub fn slices_locked(&self) -> bool {
        self.menu.slices_locked
    }
}

// ---------- targets and params ----------

/// The slices a command acts on: `slices` (ids that must all be slices), else the selected ones.
fn slice_targets(s: &Session, p: &Value, cmd: &str) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    match ids_param(p, "slices") {
        Some(ids) => match ids.iter().find(|id| !st.doc.is_slice(**id)) {
            Some(id) => Err(bad(cmd, format!("{id} is not a slice"))),
            None => Ok(dedup(ids)),
        },
        None => Ok(selected_slices(&st.doc, &st.selection)),
    }
}

/// The selected slices: those the Slice Selection tool selected and the selected objects that are
/// object slices.
fn selected(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    Ok(selected_slices(&st.doc, &st.selection))
}

pub(crate) fn selected_slices(d: &Document, sel: &vectorcraft_doc::Selection) -> Vec<NodeId> {
    let objects = sel.objects.iter().filter(|id| d.node(**id).is_some_and(|n| n.extras().slice.is_some()));
    dedup(sel.slices.iter().chain(objects).copied().collect())
}

fn dedup(mut ids: Vec<NodeId>) -> Vec<NodeId> {
    let mut seen = std::collections::HashSet::new();
    ids.retain(|id| seen.insert(*id));
    ids
}

fn slices_targeted(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if selected_slices(&st.doc, &st.selection).is_empty() { Err("no slice selected".into()) } else { Ok(()) }
}

fn doc_has_slices(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    if st.doc.slices.is_empty() && object_slice_ids(&st.doc).is_empty() { Err("the document has no slices".into()) } else { Ok(()) }
}

/// Every object with slice options, hidden ones included.
fn object_slice_ids(d: &Document) -> Vec<NodeId> {
    let mut ids = vec![];
    d.walk(|n| {
        if n.extras().slice.is_some() {
            ids.push(n.id);
        }
    });
    ids
}

/// A rectangle a slice can have: a size, and coordinates on the canvas.
pub(crate) fn slice_rect_ok(r: Rect) -> bool {
    valid_rect(r) && [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.abs() <= crate::MAX_COORD)
}

/// A new user slice over `rect` with `options`.
pub(crate) fn add_slice(d: &mut Document, rect: Rect, options: SliceOptions) -> NodeId {
    let id = d.alloc_id();
    d.slices.push(Slice { id, rect, options });
    id
}

/// Remove slice `id`: a user slice goes, an object slice's object loses its slice options.
pub(crate) fn remove_slice(d: &mut Document, id: NodeId) {
    let before = d.slices.len();
    d.slices.retain(|s| s.id != id);
    if d.slices.len() == before
        && let Some(n) = d.node_mut(id)
    {
        n.extras_mut().slice = None;
    }
}

fn ids_json(ids: &[NodeId]) -> Value {
    json!(ids.iter().map(|i| i.0).collect::<Vec<_>>())
}

/// Slice options as the commands give them back (every field).
pub(crate) fn options_json(o: &SliceOptions) -> Value {
    json!({
        "kind": o.kind.key(), "name": o.name, "url": o.url, "target": o.target, "message": o.message, "alt": o.alt,
        "text": o.text, "background": o.background, "hAlign": o.h_align.key(), "vAlign": o.v_align.key(),
    })
}

// ---------- Object → Slice ----------

fn make(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = match ids_param(p, "ids") {
        Some(ids) => super::edit::outermost(&s.doc()?.doc, ids),
        None => super::edit::selected_roots(s)?,
    };
    let d = &s.doc()?.doc;
    let made: Vec<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(|n| !n.is_layer() && n.extras().slice.is_none())).collect();
    if made.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    s.edit("Make Slice", |d, _| {
        for id in &made {
            if let Some(n) = d.node_mut(*id) {
                n.extras_mut().slice = Some(Box::default());
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&made) }))
}

fn release(s: &mut Session, _: &Value) -> Result<Value> {
    let targets = selected(s)?;
    let parent = s.doc()?.insertion_parent();
    let ids = s.edit("Release Slice", |d, sel| {
        let mut out = vec![];
        for id in &targets {
            match d.slice(*id).map(|s| s.rect) {
                // A user slice becomes an unpainted rectangle.
                Some(r) => {
                    d.slices.retain(|s| s.id != *id);
                    let parent = parent.ok_or_else(|| EngineError::Other("Release: no layer to put the rectangle in".into()))?;
                    let rid = d.alloc_id();
                    d.insert(Some(parent), usize::MAX, Node::path(rid, shapes::rectangle(r), Appearance::basic(Paint::None, Paint::None, 1.0)))?;
                    out.push(rid);
                }
                None => {
                    remove_slice(d, *id);
                    out.push(*id);
                }
            }
        }
        sel.set(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn from_guides(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let region = d.slice_clip().ok_or_else(|| EngineError::Other("Create from Guides: the document has no artboard".into()))?;
    let cuts = |vertical: bool, lo: f64, hi: f64| -> Vec<f64> {
        let mut v: Vec<f64> = d.guides.iter().filter(|g| g.vertical == vertical && g.pos > lo && g.pos < hi).map(|g| g.pos).collect();
        v.extend([lo, hi]);
        v.sort_by(f64::total_cmp);
        v.dedup();
        v
    };
    let (xs, ys) = (cuts(true, region.x0, region.x1), cuts(false, region.y0, region.y1));
    let rects: Vec<Rect> =
        ys.windows(2).flat_map(|y| xs.windows(2).map(move |x| Rect::new(x[0], y[0], x[1], y[1]))).filter(|r| valid_rect(*r)).collect();
    if rects.len() > MAX_PIECES {
        return Err(EngineError::Other(format!("Create from Guides: the guides make more than {MAX_PIECES} slices")));
    }
    let ids = s.edit("Create Slices from Guides", |d, _| {
        d.slices.clear();
        Ok(rects.iter().map(|r| add_slice(d, *r, SliceOptions::default())).collect::<Vec<_>>())
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn from_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = targets(s, p)?;
    let r = s.doc()?.doc.bounds_of(&ids, true).filter(|r| slice_rect_ok(*r));
    let r = r.ok_or_else(|| EngineError::Other("Create from Selection: the selection has no area".into()))?;
    let id = s.edit("Create Slice from Selection", |d, sel| {
        let id = add_slice(d, r, SliceOptions::default());
        sel.set_slices([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = selected(s)?;
    let (dx, dy) = (f64_or(p, "dx", 10.0), f64_or(p, "dy", 10.0));
    let ids = s.edit("Duplicate Slice", |d, sel| {
        let mut out = vec![];
        for id in &targets {
            let (Some(r), Some(o)) = (d.slice_bounds(*id), d.slice_options(*id).cloned()) else { continue };
            let r = r + vectorcraft_geom::Vec2::new(dx, dy);
            if !slice_rect_ok(r) {
                return Err(bad("object.slice.duplicate", "the copy would leave the canvas"));
            }
            out.push(add_slice(d, r, o));
        }
        sel.set_slices(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn combine(s: &mut Session, _: &Value) -> Result<Value> {
    let targets = selected(s)?;
    if targets.len() < 2 {
        return Err(bad("object.slice.combine", "select two or more slices to combine"));
    }
    let id = s.edit("Combine Slices", |d, sel| {
        let r = targets.iter().filter_map(|id| d.slice_bounds(*id)).reduce(|a, b| a.union(b));
        let r = r.ok_or_else(|| EngineError::Other("Combine Slices: the slices have no bounds".into()))?;
        let o = targets.first().and_then(|id| d.slice_options(*id)).cloned().unwrap_or_default();
        for id in &targets {
            remove_slice(d, *id);
        }
        let id = add_slice(d, r, o);
        sel.set_slices([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

/// The cuts that split `lo..hi` into `count` equal parts or parts `size` long (the last one
/// shorter), whichever `count`/`size` gives; `None` for neither.
fn splits(lo: f64, hi: f64, count: Option<u64>, size: Option<f64>) -> std::result::Result<Vec<f64>, String> {
    let len = hi - lo;
    let n = match (count, size) {
        (Some(n), _) => n,
        (None, Some(sz)) if sz.is_finite() && sz > 0.0 => (len / sz - 1e-9).ceil().max(1.0).min(MAX_PIECES as f64 + 1.0) as u64,
        (None, Some(_)) => return Err("a slice size must be a positive number of points".into()),
        (None, None) => 1,
    };
    if n == 0 || n as usize > MAX_PIECES {
        return Err(format!("divide into 1 to {MAX_PIECES} slices"));
    }
    let step = match (count, size) {
        (None, Some(sz)) => sz,
        _ => len / n as f64,
    };
    Ok((0..=n).map(|i| if i == n { hi } else { lo + step * i as f64 }).collect())
}

fn divide(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.slice.divide";
    let targets = selected(s)?;
    let count = |k: &str| p.get(k).map(|v| v.as_u64().ok_or_else(|| bad(C, format!("`{k}` must be a whole number")))).transpose();
    let size = |k: &str| p.get(k).map(|v| v.as_f64().ok_or_else(|| bad(C, format!("`{k}` must be a number of points")))).transpose();
    let (rows, row_h, cols, col_w) = (count("rows")?, size("rowHeight")?, count("columns")?, size("columnWidth")?);
    if rows.is_none() && row_h.is_none() && cols.is_none() && col_w.is_none() {
        return Err(bad(C, "give rows, rowHeight, columns or columnWidth"));
    }
    let ids = s.edit("Divide Slices", |d, sel| {
        let mut out = vec![];
        for id in &targets {
            let (Some(r), Some(o)) = (d.slice_bounds(*id), d.slice_options(*id).cloned()) else { continue };
            let ys = splits(r.y0, r.y1, rows, row_h).map_err(|e| bad(C, e))?;
            let xs = splits(r.x0, r.x1, cols, col_w).map_err(|e| bad(C, e))?;
            if (ys.len() - 1) * (xs.len() - 1) + out.len() > MAX_PIECES {
                return Err(bad(C, format!("that makes more than {MAX_PIECES} slices")));
            }
            remove_slice(d, *id);
            for (i, rect) in ys.windows(2).flat_map(|y| xs.windows(2).map(move |x| Rect::new(x[0], y[0], x[1], y[1]))).enumerate() {
                // The pieces keep the options; only the first keeps the name.
                let o = if i == 0 { o.clone() } else { SliceOptions { name: String::new(), ..o.clone() } };
                out.push(add_slice(d, rect, o));
            }
        }
        sel.set_slices(out.iter().copied());
        Ok(out)
    })?;
    Ok(json!({ "ids": ids_json(&ids) }))
}

fn delete_all(s: &mut Session, _: &Value) -> Result<Value> {
    let n = s.edit("Delete All Slices", |d, _| {
        let objects = object_slice_ids(d);
        let n = d.slices.len() + objects.len();
        d.slices.clear();
        for id in objects {
            remove_slice(d, id);
        }
        Ok(n)
    })?;
    Ok(json!({ "count": n }))
}

/// The Slice Options edits a command's params ask for.
#[derive(Default)]
struct Edits {
    kind: Option<SliceKind>,
    text: Vec<(&'static str, String)>,
    background: Option<String>,
    h_align: Option<CellAlign>,
    v_align: Option<CellVAlign>,
}

impl Edits {
    const TEXT: [&'static str; 6] = ["name", "url", "target", "message", "alt", "text"];

    fn parse(p: &Value) -> Result<Self> {
        const C: &str = "object.slice.options";
        let text = |k: &str| -> Result<Option<String>> {
            p.get(k).map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(C, format!("`{k}` must be a string")))).transpose()
        };
        let named = |k: &str| text(k).map(|v| v.map(|s| s.trim().to_string()));
        let mut e = Edits {
            kind: named("kind")?.map(|k| SliceKind::parse(&k).ok_or_else(|| bad(C, "`kind` must be image, noImage or htmlText"))).transpose()?,
            h_align: named("hAlign")?
                .map(|k| CellAlign::parse(&k).ok_or_else(|| bad(C, "`hAlign` must be default, left, center or right")))
                .transpose()?,
            v_align: named("vAlign")?
                .map(|k| CellVAlign::parse(&k).ok_or_else(|| bad(C, "`vAlign` must be default, top, middle, baseline or bottom")))
                .transpose()?,
            ..Default::default()
        };
        e.background = named("background")?
            .map(|b| match b.to_ascii_lowercase().as_str() {
                "" | "none" => Ok(String::new()),
                "matte" => Ok("matte".to_string()),
                _ => vectorcraft_color::Color::from_hex(&b)
                    .map(|c| c.to_hex())
                    .ok_or_else(|| bad(C, "`background` must be \"\", \"matte\" or #rrggbb")),
            })
            .transpose()?;
        for k in Self::TEXT {
            // Names, links and frames are trimmed; the cell text and alt text keep their spaces.
            let v = if matches!(k, "text" | "alt" | "message") { text(k)? } else { named(k)? };
            if let Some(v) = v {
                e.text.push((k, v));
            }
        }
        Ok(e)
    }

    fn is_empty(&self) -> bool {
        self.kind.is_none() && self.text.is_empty() && self.background.is_none() && self.h_align.is_none() && self.v_align.is_none()
    }

    fn apply(&self, o: &mut SliceOptions) {
        if let Some(k) = self.kind {
            o.kind = k;
        }
        for (k, v) in &self.text {
            let field = match *k {
                "name" => &mut o.name,
                "url" => &mut o.url,
                "target" => &mut o.target,
                "message" => &mut o.message,
                "alt" => &mut o.alt,
                _ => &mut o.text,
            };
            field.clone_from(v);
        }
        if let Some(b) = &self.background {
            o.background.clone_from(b);
        }
        if let Some(a) = self.h_align {
            o.h_align = a;
        }
        if let Some(a) = self.v_align {
            o.v_align = a;
        }
    }
}

/// Can slice `id` be HTML Text: an object slice of a type object?
fn html_text_allowed(d: &Document, id: NodeId) -> bool {
    d.slice(id).is_none() && d.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_)))
}

fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = selected(s)?;
    let edits = Edits::parse(p)?;
    let html_ok = |d: &Document| targets.iter().all(|id| html_text_allowed(d, *id));
    if edits.kind == Some(SliceKind::HtmlText) && !html_ok(&s.doc()?.doc) {
        return Err(bad("object.slice.options", "HTML Text applies to object slices of type only"));
    }
    if !edits.is_empty() {
        s.edit("Slice Options", |d, _| {
            for id in &targets {
                let o = match d.slice_mut(*id) {
                    Some(sl) => Some(&mut sl.options),
                    None => d.node_mut(*id).and_then(|n| n.extras_if_any().and_then(|e| e.slice.as_deref_mut())),
                };
                if let Some(o) = o {
                    edits.apply(o);
                }
            }
            Ok(())
        })?;
    }
    let d = &s.doc()?.doc;
    let o = targets.first().and_then(|id| d.slice_options(*id)).cloned().unwrap_or_default();
    let mut v = options_json(&o);
    v["htmlText"] = json!(html_ok(d));
    Ok(v)
}

fn clip_to_artboard(s: &mut Session, p: &Value) -> Result<Value> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(!s.doc()?.doc.slices_clip_to_artboard);
    if on != s.doc()?.doc.slices_clip_to_artboard {
        s.edit("Clip to Artboard", |d, _| {
            d.slices_clip_to_artboard = on;
            Ok(())
        })?;
    }
    Ok(json!({ "on": on }))
}

fn hide(s: &mut Session, p: &Value) -> Result<Value> {
    let v = p.get("hidden").and_then(Value::as_bool).unwrap_or(!s.menu.slices_hidden);
    s.menu.slices_hidden = v;
    Ok(json!({ "hidden": v }))
}

fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    let v = p.get("locked").and_then(Value::as_bool).unwrap_or(!s.menu.slices_locked);
    s.menu.slices_locked = v;
    Ok(json!({ "locked": v }))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let st = s.doc()?;
    let d = &st.doc;
    let slices: Vec<Value> = d
        .slice_layout()
        .iter()
        .map(|a| {
            let o = a.id.and_then(|id| d.slice_options(id)).cloned().unwrap_or_default();
            json!({
                "number": a.number, "id": a.id.map(|i| i.0), "source": a.source.key(), "name": d.slice_name(a),
                "x": a.rect.x0, "y": a.rect.y0, "width": a.rect.width(), "height": a.rect.height(),
                "options": options_json(&o), "selected": a.id.is_some_and(|id| st.selection.slices.contains(&id)),
            })
        })
        .collect();
    Ok(json!({ "slices": slices, "clipToArtboard": d.slices_clip_to_artboard, "hidden": s.menu.slices_hidden, "locked": s.menu.slices_locked }))
}

fn select_all(s: &mut Session, _: &Value) -> Result<Value> {
    let ids = s.doc()?.doc.slice_ids();
    let n = ids.len();
    s.select(|_, sel| sel.set_slices(ids))?;
    Ok(json!({ "count": n }))
}

// ---------- the Slice and Slice Selection tools ----------

/// `{x, y, width, height}` as a rectangle a slice can have.
fn rect_param(p: &Value, cmd: &str) -> Result<Rect> {
    let (x, y) = (f64_req(p, "x", cmd)?, f64_req(p, "y", cmd)?);
    let r = Rect::new(x, y, x + f64_req(p, "width", cmd)?, y + f64_req(p, "height", cmd)?).abs();
    if slice_rect_ok(r) { Ok(r) } else { Err(bad(cmd, "the rectangle needs a size and must lie on the canvas")) }
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let r = rect_param(p, "object.slice.create")?;
    let id = s.edit("Slice", |d, sel| {
        let id = add_slice(d, r, SliceOptions::default());
        sel.set_slices([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0 }))
}

fn set_rect(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.slice.setRect";
    let id = id_param(p, "id").ok_or_else(|| bad(C, "missing `id`"))?;
    let r = rect_param(p, C)?;
    let d = &s.doc()?.doc;
    if d.slice(id).is_none() {
        return Err(bad(C, if d.is_slice(id) { format!("{id} is an object slice: it follows its object") } else { format!("{id} is not a slice") }));
    }
    s.edit("Resize Slice", |d, _| {
        if let Some(sl) = d.slice_mut(id) {
            sl.rect = r;
        }
        Ok(())
    })?;
    Ok(json!({ "id": id.0 }))
}

fn move_slices(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.slice.move";
    let targets = slice_targets(s, p, C)?;
    let v = vectorcraft_geom::Vec2::new(f64_req(p, "dx", C)?, f64_req(p, "dy", C)?);
    if !(v.x.is_finite() && v.y.is_finite()) {
        return Err(bad(C, "dx and dy must be numbers"));
    }
    s.edit("Move Slice", |d, _| {
        for id in &targets {
            match d.slice_mut(*id) {
                Some(sl) if slice_rect_ok(sl.rect + v) => sl.rect = sl.rect + v,
                Some(_) => return Err(bad(C, "the slice would leave the canvas")),
                None => {
                    if let Some(n) = d.node_mut(*id) {
                        n.transform(vectorcraft_geom::Affine::translate(v), vectorcraft_doc::Scaling::default());
                    }
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": ids_json(&targets) }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let targets = slice_targets(s, p, "object.slice.delete")?;
    if targets.is_empty() {
        return Ok(json!({ "count": 0 }));
    }
    s.edit("Delete Slice", |d, _| {
        for id in &targets {
            remove_slice(d, *id);
        }
        Ok(())
    })?;
    Ok(json!({ "count": targets.len() }))
}

fn select(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = slice_targets(s, &json!({ "slices": p.get("slices").cloned().unwrap_or(json!([])) }), "object.slice.select")?;
    let toggle = bool_or(p, "toggle", false);
    s.select(|_, sel| {
        let mut now = if toggle { sel.slices.clone() } else { vec![] };
        for id in ids {
            match now.iter().position(|x| *x == id) {
                Some(i) if toggle => {
                    now.remove(i);
                }
                Some(_) => {}
                None => now.push(id),
            }
        }
        sel.set_slices(now);
    })?;
    Ok(json!({ "selected": ids_json(&s.doc()?.selection.slices) }))
}
