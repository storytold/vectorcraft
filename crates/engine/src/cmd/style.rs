//! The Graphic Styles panel: new, apply (replace or add), link, redefine, break link, rename,
//! delete, duplicate, sort and list.
//!
//! Applying a style links the targeted objects to it ([`Node::graphic_style`]). An object stays
//! linked while it keeps the style's look; editing its appearance or transparency breaks the link
//! ([`in_sync`]), and Redefine Graphic Style updates only the objects still linked.
//!
//! Styles keep placed gradients relative to the unit box ([`GraphicStyle::unit_box`]): a new or
//! redefined style stores them relative to its source's bounds, and each object a style is applied
//! to gets them at the same place relative to its own.
//!
//! With Override Character Color on ([`crate::Prefs::override_char_color`]), type a style is applied
//! to loses its characters' own fill (stroke) when the style has fills (strokes), so the style's
//! paint shows instead of the character colour under it.

use std::collections::HashMap;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, DEFAULT_GRAPHIC_STYLE, Document, GraphicStyle, Node, NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::*;
use crate::EngineError;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "graphicStyle.apply",
            "Apply Graphic Style",
            ["Window", "Graphic Styles"],
            None,
            "{name, add?: bool, ids? (layers too), target?: \"object\"|\"contents\"} give the objects (default: the selection; a group or layer itself, its fills and strokes painting its members and its effects applying to them as one piece; target contents: the objects inside instead) the style's appearance, opacity, blend mode, isolate and knockout, and link them to it (placed gradients land at the same place relative to each object's bounds); add: true (Alt-click) adds its fills, strokes and effects on top of the existing appearance instead and unlinks. With nothing selected (and no ids) the next object drawn takes the style instead (see appearance.newArt) → {newArt: true}",
            has_doc,
            style_apply
        ),
        cmd!(
            "graphicStyle.new",
            "New Graphic Style",
            ["Window", "Graphic Styles"],
            None,
            "{name?, id?} a style from object `id` or the first selected object: its appearance, opacity, blend mode, isolate and knockout (a group without fills or strokes of its own lends its topmost object's, type its characters'; placed gradients are kept relative to its bounds); links the object. Names are made unique → {name}",
            has_doc,
            style_new
        ),
        cmd!(
            "graphicStyle.delete",
            "Delete Graphic Style",
            ["Window", "Graphic Styles"],
            None,
            "{name} | {names: [name…]} delete styles; their objects keep their look but are unlinked",
            has_doc,
            style_delete
        ),
        cmd!("graphicStyle.duplicate", "Duplicate Graphic Style", ["Window", "Graphic Styles"], None, "{name} → {name}", has_doc, style_duplicate),
        cmd!(
            query "graphicStyle.list",
            "List Graphic Styles",
            [],
            None,
            "{} → {styles: [{id, name, fill, stroke, strokeWidth, fills, strokes, effects: [effect id…], opacity: 0..100, blend, isolate, knockout: \"on\"|\"off\"|\"neutral\", linked: [object ids]}], selected: the style the first selected object is linked to, or null, overrideCharColor}",
            has_doc,
            style_list
        ),
        cmd!(
            "graphicStyle.redefine",
            "Redefine Graphic Style",
            ["Window", "Appearance"],
            None,
            "{name?, id?} replace the style (default: the one the object was last linked to) with the look of object `id` or the first selected object; the objects still linked to it update, and that object becomes linked → {name}",
            has_doc,
            style_redefine
        ),
        cmd!(
            "graphicStyle.breakLink",
            "Break Link to Graphic Style",
            ["Window", "Graphic Styles"],
            None,
            "{ids?} unlink the objects (default: the selection) from their graphic style; they keep their look",
            has_doc,
            style_break_link
        ),
        cmd!(
            "graphicStyle.rename",
            "Rename Graphic Style",
            ["Window", "Graphic Styles"],
            None,
            "{name, to} (Graphic Style Options; names are unique and linked objects stay linked) → {name}",
            has_doc,
            style_rename
        ),
        cmd!(
            query "graphicStyle.unused",
            "Select All Unused",
            ["Window", "Graphic Styles"],
            None,
            "{} → {names: [the styles no object is linked to]} (the panel selects them)",
            has_doc,
            style_unused
        ),
        cmd!(
            "graphicStyle.sortByName",
            "Sort by Name",
            ["Window", "Graphic Styles"],
            None,
            "{} sort the styles by name (the Default Graphic Style stays first)",
            has_doc,
            style_sort
        ),
        cmd!(
            "graphicStyle.merge",
            "Merge Graphic Styles",
            ["Window", "Graphic Styles"],
            None,
            "{names: [name…] (two or more), name?} add a style combining the styles' fills, strokes and effects (each style's on top of the ones before it) with the first one's opacity, blend mode, isolate and knockout → {name}",
            has_doc,
            style_merge
        ),
        cmd!(
            "graphicStyle.move",
            "Move Graphic Style",
            [],
            None,
            "{name, to: index} move the style to position `to` of the list (0 = first, clamped), as dragging it in the Graphic Styles panel does",
            has_doc,
            style_move
        ),
        cmd!(
            query "graphicStyle.setOptions",
            "Graphic Styles Options",
            [],
            None,
            "{overrideCharColor?: bool} Override Character Color (the preference `overrideCharColor`, on by default): applying a style to type replaces its characters' fill and stroke with the style's fills and strokes → {overrideCharColor}",
            always,
            style_set_options
        ),
    ]
}

// ---------- links ----------

/// Does a style applied to `n` (`top`: `n` is the target itself) descend into its children? Groups
/// and layers style their contents, and so do other containers (blends…) inside them, while a
/// compound path takes it as a whole, as with paint commands ([`leaf_targets`]).
fn descends(n: &Node, top: bool) -> bool {
    match n.kind {
        NodeKind::Group { .. } | NodeKind::Layer { .. } => true,
        NodeKind::Compound { .. } => false,
        _ => !top && n.is_container(),
    }
}

/// The objects whose appearance a style applied to `n` sets: `n`, or its painted contents.
fn painted<'a>(n: &'a Node, top: bool, out: &mut Vec<&'a Node>) {
    match n.children() {
        Some(ch) if descends(n, top) => ch.iter().for_each(|c| painted(c, false, out)),
        _ => out.push(n),
    }
}

/// Does `n` still have style `g`'s look (its transparency, and the style's appearance as its own;
/// a group or layer with no appearance of its own: on every painted object inside it, the look a
/// style made from it captures)? Linked objects that don't are no longer linked.
pub(crate) fn in_sync(n: &Node, g: &GraphicStyle) -> bool {
    if !g.transparency_matches(n) {
        return false;
    }
    if g.appearance_on(n).approx_eq(&n.appearance) {
        return true;
    }
    let mut leaves = vec![];
    painted(n, true, &mut leaves);
    n.appearance == Appearance::default() && leaves.iter().all(|l| l.id != n.id && g.appearance_on(l).approx_eq(&l.appearance))
}

/// The style `n` is linked to (and still looks like).
pub(crate) fn linked_style<'a>(d: &'a Document, n: &Node) -> Option<&'a GraphicStyle> {
    d.graphic_style_by_id(n.graphic_style?).filter(|g| in_sync(n, g))
}

/// Objects anywhere in the art that `f` accepts.
fn objects_where(d: &Document, mut f: impl FnMut(&Node) -> bool) -> Vec<NodeId> {
    let mut ids = vec![];
    for l in &d.layers {
        l.walk(&mut |n| {
            if f(n) {
                ids.push(n.id);
            }
        });
    }
    ids
}

/// The objects linked to each style id.
fn links(d: &Document) -> HashMap<u32, Vec<NodeId>> {
    let mut out: HashMap<u32, Vec<NodeId>> = HashMap::new();
    for l in &d.layers {
        l.walk(&mut |n| {
            if let Some(g) = linked_style(d, n) {
                out.entry(g.id).or_default().push(n.id);
            }
        });
    }
    out
}

/// `ap` with its placed gradients moved from box `from` (an object's bounds) to the unit box.
fn unitized(mut ap: Appearance, from: Option<Rect>) -> Appearance {
    if let Some(b) = from.filter(|_| ap.has_placed_gradient()) {
        ap.rebase_gradients(b, GraphicStyle::UNIT_BOX);
    }
    ap
}

/// The appearance a new or redefined style takes from `n`: its own. A group without fills or
/// strokes of its own lends those of its topmost painted object, and type its characters' fill
/// and stroke. Placed gradients come relative to the unit box (from the bounds of the object
/// they're on). The pen pressure of strokes stays with their own path.
pub(crate) fn captured(n: &Node) -> Appearance {
    let mut ap = n.appearance.clone().without_pressure();
    if !ap.items.is_empty() {
        return unitized(ap, n.geometric_bounds());
    }
    match &n.kind {
        NodeKind::Text(t) => {
            if let Some(r) = t.runs.first() {
                ap.items = r.style.basic_appearance().items;
                // Type paints its characters in text space.
                ap = unitized(ap, Some(t.local_bounds()));
            }
        }
        _ => {
            let mut leaves = vec![];
            painted(n, true, &mut leaves);
            if let Some(top) = leaves.last().filter(|l| l.id != n.id) {
                // The topmost object's own effects apply before the group's.
                let top = captured(top);
                ap.items = top.items;
                ap.effects.splice(0..0, top.effects);
            }
        }
    }
    ap
}

/// Override Character Color: type taking appearance `ap` loses its characters' fill when `ap` has
/// fills, and their stroke when it has strokes.
fn override_chars(n: &mut Node, ap: &Appearance) {
    let NodeKind::Text(t) = &mut n.kind else { return };
    let (fills, strokes) = (ap.items.iter().any(|i| i.is_fill()), ap.items.iter().any(|i| !i.is_fill()));
    for r in &mut t.runs {
        if fills {
            r.style.fill = Paint::None;
        }
        if strokes {
            r.style.stroke = Paint::None;
        }
    }
}

/// Give `n` style `g`: its appearance (a group or layer too: its own, above its contents; type its
/// characters' colour too with `chars`, Override Character Color) and transparency.
fn style_node(n: &mut Node, g: &GraphicStyle, chars: bool) {
    n.appearance = g.appearance_on(n).into_owned();
    if chars {
        override_chars(n, &g.appearance);
    }
    g.apply_transparency(n);
}

/// Give `id` style `g` ([`style_node`]) and link it.
fn apply_style(d: &mut Document, id: NodeId, g: &GraphicStyle, chars: bool) {
    if let Some(n) = d.node_mut(id) {
        style_node(n, g, chars);
        n.graphic_style = Some(g.id);
    }
}

fn set_link(d: &mut Document, ids: &[NodeId], link: Option<u32>) {
    for id in ids {
        if let Some(n) = d.node_mut(*id) {
            n.graphic_style = link;
        }
    }
}

fn style_index(d: &Document, name: &str) -> Result<usize> {
    d.graphic_style_index(name).ok_or_else(|| EngineError::Other(format!("no graphic style `{name}`")))
}

/// The object a style is made from: `id`, else the first selected object.
fn source<'a>(s: &'a Session, p: &Value, cmd: &str) -> Result<(NodeId, &'a Node)> {
    let id = match id_param(p, "id") {
        Some(id) => id,
        None => super::edit::selected_roots(s)?.first().copied().ok_or_else(|| bad(cmd, "select an object (or pass `id`)"))?,
    };
    Ok((id, s.doc()?.doc.node(id).ok_or_else(|| bad(cmd, format!("no object {}", id.0)))?))
}

impl Session {
    /// The graphic style of the first selected object: the one it was last linked to, and whether
    /// it is still linked (still has that style's look).
    pub fn selection_graphic_style(&self) -> Option<(&GraphicStyle, bool)> {
        let st = self.active()?;
        let n = st.doc.node(*st.selection.subjects().first()?)?;
        let g = st.doc.graphic_style_by_id(n.graphic_style?)?;
        Some((g, in_sync(n, g)))
    }

    /// A copy of `n` with style `g` applied as `graphicStyle.apply` applies it (Override Character
    /// Color as set): the Graphic Styles panel's previews.
    pub fn styled(&self, n: &Node, g: &GraphicStyle) -> Node {
        let mut n = n.clone();
        style_node(&mut n, g, self.prefs.override_char_color);
        n
    }
}

// ---------- commands ----------

fn style_apply(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("graphicStyle.apply", "missing name"))?;
    let i = style_index(&s.doc()?.doc, name)?;
    let add = bool_or(p, "add", false);
    let chars = s.prefs.override_char_color;
    let ids = super::appearance::appearance_targets(s, p)?;
    if ids.is_empty() {
        if p.get("ids").is_some() || p.get("id").is_some() {
            return Err(bad("graphicStyle.apply", "no such objects"));
        }
        // Nothing selected: the next object drawn takes the style.
        let g = s.doc()?.doc.graphic_styles[i].clone();
        s.new_art_style(&g, add);
        return Ok(json!({ "newArt": true }));
    }
    s.edit("Apply Graphic Style", |d, _| {
        if add {
            let g = d.graphic_styles[i].clone();
            for id in &ids {
                if let Some(n) = d.node_mut(*id) {
                    let ap = g.appearance_on(n).into_owned();
                    n.appearance.items.extend(ap.items);
                    n.appearance.effects.extend(ap.effects);
                    n.graphic_style = None;
                }
            }
        } else {
            d.graphic_style_id(i);
            let g = d.graphic_styles[i].clone();
            for id in &ids {
                apply_style(d, *id, &g, chars);
            }
        }
        Ok(())
    })?;
    ok()
}

fn style_new(s: &mut Session, p: &Value) -> Result<Value> {
    let (src, n) = source(s, p, "graphicStyle.new")?;
    let d = &s.doc()?.doc;
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => unique_name(n, |x| d.graphic_style(x).is_some()),
        None => d.new_graphic_style_name(),
    };
    let g = GraphicStyle { id: d.next_graphic_style_id(), ..GraphicStyle::of(name.clone(), captured(n), n) };
    s.edit("New Graphic Style", |d, _| {
        set_link(d, &[src], Some(g.id));
        d.graphic_styles.push(g);
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn style_delete(s: &mut Session, p: &Value) -> Result<Value> {
    let names: Vec<String> = match (p.get("names").and_then(Value::as_array), str_param(p, "name")) {
        (Some(a), _) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        (None, Some(n)) => vec![n.to_string()],
        (None, None) => return Err(bad("graphicStyle.delete", "missing name")),
    };
    s.edit(if names.len() > 1 { "Delete Graphic Styles" } else { "Delete Graphic Style" }, |d, _| {
        for name in &names {
            let i = style_index(d, name)?;
            let id = d.graphic_styles.remove(i).id;
            if id != 0 {
                let ids = objects_where(d, |n| n.graphic_style == Some(id));
                set_link(d, &ids, None);
            }
        }
        Ok(())
    })?;
    ok()
}

fn style_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").ok_or_else(|| bad("graphicStyle.duplicate", "missing name"))?.to_string();
    let new = s.edit("Duplicate Graphic Style", |d, _| {
        let pos = style_index(d, &name)?;
        let nm = unique_name(&format!("{name} copy"), |n| d.graphic_style(n).is_some());
        let g = GraphicStyle { name: nm.clone(), id: d.next_graphic_style_id(), ..d.graphic_styles[pos].clone() };
        d.graphic_styles.insert(pos + 1, g);
        Ok(nm)
    })?;
    Ok(json!({"name": new}))
}

fn style_list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let links = links(d);
    let styles: Vec<Value> = d
        .graphic_styles
        .iter()
        .map(|g| {
            let mut v = style_json(g);
            v["id"] = json!(g.id);
            v["linked"] = json!(links.get(&g.id).map(|v| v.iter().map(|i| i.0).collect::<Vec<_>>()).unwrap_or_default());
            v
        })
        .collect();
    let selected = s.selection_graphic_style().filter(|(_, linked)| *linked).map(|(g, _)| g.name.clone());
    Ok(json!({ "styles": styles, "selected": selected, "overrideCharColor": s.prefs.override_char_color }))
}

/// Style `g` described as `graphicStyle.list` lists it (without `id` and `linked`).
pub(crate) fn style_json(g: &GraphicStyle) -> Value {
    let ap = &g.appearance;
    let count = |fill: bool| ap.items.iter().filter(|i| i.is_fill() == fill).count();
    json!({
        "name": g.name,
        "fill": ap.fill_paint().label(),
        "stroke": ap.stroke_paint().label(),
        "strokeWidth": ap.stroke_width(),
        "fills": count(true),
        "strokes": count(false),
        "effects": ap.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        "opacity": (g.opacity * 100.0).round(),
        "blend": g.blend.label(),
        "isolate": g.isolate,
        "knockout": g.knockout.label(),
    })
}

fn style_redefine(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "graphicStyle.redefine";
    let (src, n) = source(s, p, cmd)?;
    let d = &s.doc()?.doc;
    let i = match str_param(p, "name") {
        Some(name) => style_index(d, name)?,
        None => n
            .graphic_style
            .and_then(|id| d.graphic_style_by_id(id))
            .and_then(|g| d.graphic_style_index(&g.name))
            .ok_or_else(|| bad(cmd, "the object has no graphic style (pass `name`)"))?,
    };
    let look = GraphicStyle::of(String::new(), captured(n), n);
    let name = d.graphic_styles[i].name.clone();
    let chars = s.prefs.override_char_color;
    s.edit(&format!("Redefine Graphic Style \u{201c}{name}\u{201d}"), |d, _| {
        let id = d.graphic_style_id(i);
        let old = std::mem::replace(&mut d.graphic_styles[i], GraphicStyle { name: name.clone(), id, ..look });
        // Objects still linked follow the new definition; edited ones are unlinked (they keep
        // their look), and the source keeps its own.
        let linked = objects_where(d, |n| n.graphic_style == Some(id) && n.id != src);
        let (follow, edited): (Vec<NodeId>, Vec<NodeId>) = linked.into_iter().partition(|nid| d.node(*nid).is_some_and(|n| in_sync(n, &old)));
        set_link(d, &edited, None);
        let g = d.graphic_styles[i].clone();
        for nid in follow {
            apply_style(d, nid, &g, chars);
        }
        set_link(d, &[src], Some(id));
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn style_break_link(s: &mut Session, p: &Value) -> Result<Value> {
    let ids = super::appearance::appearance_targets(s, p)?;
    s.edit("Break Link to Graphic Style", |d, _| {
        set_link(d, &ids, None);
        Ok(())
    })?;
    ok()
}

fn style_rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "graphicStyle.rename";
    let name = str_param(p, "name").ok_or_else(|| bad(cmd, "missing name"))?.to_string();
    let to = str_param(p, "to").map(str::trim).filter(|t| !t.is_empty()).ok_or_else(|| bad(cmd, "missing `to`"))?.to_string();
    s.edit("Graphic Style Options", |d, _| {
        let i = style_index(d, &name)?;
        if to != name && d.graphic_style(&to).is_some() {
            return Err(EngineError::Other(format!("a graphic style named `{to}` already exists")));
        }
        d.graphic_styles[i].name = to.clone();
        Ok(())
    })?;
    Ok(json!({ "name": to }))
}

fn style_unused(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let links = links(d);
    let names: Vec<&str> = d.graphic_styles.iter().filter(|g| !links.contains_key(&g.id)).map(|g| g.name.as_str()).collect();
    Ok(json!({ "names": names }))
}

fn style_sort(s: &mut Session, _: &Value) -> Result<Value> {
    s.edit("Sort by Name", |d, _| {
        d.graphic_styles.sort_by_cached_key(|g| (g.name != DEFAULT_GRAPHIC_STYLE, g.name.to_lowercase()));
        Ok(())
    })?;
    ok()
}

fn style_merge(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "graphicStyle.merge";
    let names: Vec<&str> = p.get("names").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    if names.len() < 2 {
        return Err(bad(cmd, "give two or more style `names`"));
    }
    let d = &s.doc()?.doc;
    let styles = names.iter().map(|n| style_index(d, n).map(|i| &d.graphic_styles[i])).collect::<Result<Vec<_>>>()?;
    let name = match str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()) {
        Some(n) => unique_name(n, |x| d.graphic_style(x).is_some()),
        None => d.new_graphic_style_name(),
    };
    // The first style's contents slot (groups and type) stays where it was in its stack.
    let mut ap = styles[0].appearance.clone();
    for g in &styles[1..] {
        ap.items.extend(g.appearance.items.iter().cloned());
        ap.effects.extend(g.appearance.effects.iter().cloned());
    }
    // Styles saved before unit-box styles keep their placed gradients in document coordinates.
    let unit_box = styles.iter().filter(|g| g.appearance.has_placed_gradient()).all(|g| g.unit_box);
    let g = GraphicStyle { name: name.clone(), appearance: ap, id: d.next_graphic_style_id(), unit_box, ..styles[0].clone() };
    s.edit("Merge Graphic Styles", |d, _| {
        d.graphic_styles.push(g);
        Ok(())
    })?;
    Ok(json!({ "name": name }))
}

fn style_move(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "graphicStyle.move";
    let name = str_param(p, "name").ok_or_else(|| bad(cmd, "missing name"))?;
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(cmd, "missing `to` (an index)"))?;
    let d = &s.doc()?.doc;
    let from = style_index(d, name)?;
    let to = (to as usize).min(d.graphic_styles.len() - 1);
    if from == to {
        return ok();
    }
    s.edit("Move Graphic Style", |d, _| {
        let g = d.graphic_styles.remove(from);
        d.graphic_styles.insert(to, g);
        Ok(())
    })?;
    ok()
}

fn style_set_options(s: &mut Session, p: &Value) -> Result<Value> {
    if let Some(on) = p.get("overrideCharColor").and_then(Value::as_bool) {
        s.prefs.override_char_color = on;
    }
    Ok(json!({ "overrideCharColor": s.prefs.override_char_color }))
}
