use super::edit::{regenerate, regenerate_detached};
use super::{Arc, Document, GraphDesign, GraphSpec, Node, Result, Session, Value, bad, json, ok, str_param};
use crate::cmd::clipboard::resources::import_graph_designs;
use crate::cmd::swatchlib::{concealed_library_id, shown_library_id};

/// The document's graph design names, in order.
pub(super) fn design_names(d: &Document) -> Vec<String> {
    d.graph_designs.iter().map(|x| x.name.clone()).collect()
}

/// The library param `library` names: a document Other Library… loaded (`swatch.library.load`)
/// that holds graph designs, by its id, the id a reply showed for it, or its name (in any case, as
/// swatch library names are matched). `None` without
/// `library`.
fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Arc<Document>>> {
    let key = match p.get("library") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(k)) => k.as_str(),
        Some(_) => {
            let list = "graph.design";
            return Err(bad(cmd, format!("`library` is a library id or name (see {list})")));
        }
    };
    let libs = &s.graph_design_libraries;
    let own = concealed_library_id(s, key);
    libs.get(key)
        .or_else(|| own.as_deref().and_then(|id| libs.get(id)))
        .or_else(|| libs.iter().find(|l| l.name.eq_ignore_ascii_case(key)))
        .map(|l| Some(l.doc.clone()))
        .ok_or_else(|| bad(cmd, format!("no library `{key}`")))
}

/// The loaded libraries' designs, as lists show them: `[{library, name, designs}]`; `None` when no
/// loaded library holds graph designs.
pub(super) fn libraries_json(s: &Session) -> Option<Value> {
    let libs: Vec<Value> = s
        .graph_design_libraries
        .iter()
        .map(|l| json!({ "library": shown_library_id(s, &l.id), "name": l.name, "designs": design_names(&l.doc) }))
        .collect();
    (!libs.is_empty()).then(|| json!(libs))
}

/// The library design `p`'s `design` names, with the library to bring it in from, when `p` gives a
/// `library` and the document has no design of that name: the document's own design of a name is
/// the one a graph draws with.
pub(super) fn from_library(s: &Session, p: &Value, cmd: &str) -> Result<Option<(Arc<Document>, String)>> {
    let Some(lib) = library_param(s, p, cmd)? else { return Ok(None) };
    let Some(Value::String(n)) = p.get("design") else { return Ok(None) };
    if s.doc()?.doc.graph_designs.iter().any(|x| x.name == *n) {
        return Ok(None);
    }
    if !lib.graph_designs.iter().any(|x| x.name == *n) {
        return Err(bad(cmd, format!("no design named `{n}`")));
    }
    Ok(Some((lib, n.clone())))
}

/// Add designs `names` of library `lib` to `d` with the resources their art uses, and redraw the
/// graphs that already name one the document lacked.
pub(super) fn bring_in(d: &mut Document, lib: &Document, names: &[String]) -> Result<()> {
    for (name, added) in import_graph_designs(lib, d, names)? {
        if added {
            refresh_graphs_using(d, &name)?;
        }
    }
    Ok(())
}

/// [`bring_in`] library design `name` for graph `id`, whose new spec is `spec`: when that redraws
/// the graph, `spec` takes the rectangle the redraw gave it, so a graph moved or resized since it
/// was last drawn stays where it is.
pub(super) fn bring_in_for(d: &mut Document, lib: &Document, name: &str, id: super::NodeId, spec: &mut GraphSpec) -> Result<()> {
    bring_in(d, lib, &[name.to_string()])?;
    if let Some(g) = d.node(id).and_then(|n| n.extras().graph.as_deref()) {
        spec.rect = g.rect;
    }
    Ok(())
}

/// Whether graph `g` draws a series with design `name`, as its marker or its columns.
fn draws_with(g: &GraphSpec, name: &str) -> bool {
    g.series_markers.iter().flatten().any(|m| m == name) || g.series_columns.iter().flatten().any(|c| c.design == name)
}

/// Regenerate the graphs whose series use design `name` (it was added or removed), in the layers,
/// in opacity-mask art and in symbols' art.
fn refresh_graphs_using(d: &mut Document, name: &str) -> Result<()> {
    let names = std::collections::BTreeSet::from([name.to_string()]);
    let (mut ids, mut masked) = (vec![], vec![]);
    d.walk(|n| {
        if n.extras().graph.as_deref().is_some_and(|g| draws_with(g, name)) {
            ids.push(n.id);
        }
        if n.extras().mask.as_deref().is_some_and(|m| holds(&m.art, &names)) {
            masked.push(n.id);
        }
    });
    for id in ids {
        if let Some(spec) = d.node(id).and_then(|n| n.extras().graph.as_deref().cloned()) {
            regenerate(d, id, spec)?;
        }
    }
    // The graphs in objects' opacity-mask art.
    for id in masked {
        let Some(mut art) = d.node(id).and_then(|n| n.extras().mask.as_deref()).map(|m| (*m.art).clone()) else { continue };
        redraw_in(d, &mut art, &names);
        if let Some(mask) = d.node_mut(id).and_then(|n| n.extras_if_any()).and_then(|e| e.mask.as_deref_mut()) {
            mask.art = Arc::new(art);
        }
    }
    // And the graphs in symbols' art, which instances draw.
    let symbols: std::collections::BTreeSet<String> = d.symbols.iter().map(|s| s.name.clone()).collect();
    redraw_symbols_with_designs(d, &symbols, &names);
    Ok(())
}

/// Draw the graphs in the subtrees of `ids` again that draw a series with one of the designs
/// `names`: pasted graphs, after the paste kept the document's own designs of those names.
pub(crate) fn redraw_with_designs(d: &mut Document, ids: &[super::NodeId], names: &std::collections::BTreeSet<String>) -> Result<()> {
    for id in ids {
        let Some(mut n) = d.node(*id).filter(|n| holds(n, names)).cloned() else { continue };
        redraw_in(d, &mut n, names);
        *d.node_mut(*id).ok_or(super::EngineError::NoNode(*id))? = n;
    }
    Ok(())
}

/// Draw the graphs in the art of symbols `symbols` again that draw a series with one of the
/// designs `names`: graphs of the symbols a paste added, after the paste kept the document's own
/// designs of those names.
pub(crate) fn redraw_symbols_with_designs(
    d: &mut Document,
    symbols: &std::collections::BTreeSet<String>,
    names: &std::collections::BTreeSet<String>,
) {
    for i in 0..d.symbols.len() {
        let Some(s) = d.symbols.get(i).filter(|s| symbols.contains(&s.name) && holds(&s.art, names)) else { continue };
        let old = s.art.clone();
        let mut art = (*old).clone();
        redraw_in(d, &mut art, names);
        let Some(s) = d.symbols.get_mut(i) else { continue };
        s.art = Arc::new(art);
        // Instances' changes to parts the redraw replaced are dropped.
        let name = s.name.clone();
        crate::cmd::symboledit::follow_art_change(d, &name, &old);
    }
}

/// Whether `n`'s subtree, the art of opacity masks included, holds a graph that draws a series
/// with one of the designs `names`.
fn holds(n: &Node, names: &std::collections::BTreeSet<String>) -> bool {
    let mut found = false;
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |m| {
        found |= m.extras().graph.as_deref().is_some_and(|g| names.iter().any(|name| draws_with(g, name)));
    });
    found
}

/// Redraw the graphs of `n`'s subtree, the art of opacity masks included, that draw a series with
/// one of the designs `names`.
fn redraw_in(d: &mut Document, n: &mut Node, names: &std::collections::BTreeSet<String>) {
    if let Some(mut art) = n.extras().mask.as_deref().filter(|m| holds(&m.art, names)).map(|m| (*m.art).clone()) {
        redraw_in(d, &mut art, names);
        if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_deref_mut()) {
            mask.art = Arc::new(art);
        }
    }
    if n.extras().graph.as_deref().is_some_and(|g| names.iter().any(|name| draws_with(g, name))) {
        regenerate_detached(d, n);
        return;
    }
    if let Some(children) = n.children_mut() {
        for c in children.iter_mut().filter(|c| holds(c, names)) {
            redraw_in(d, Arc::make_mut(c), names);
        }
    }
}

/// The designs no graph's series draw with (Select Unused).
fn unused_designs(d: &Document) -> Vec<String> {
    let mut used = std::collections::HashSet::new();
    d.walk(|n| {
        if let Some(g) = n.extras().graph.as_deref() {
            used.extend(g.series_markers.iter().flatten().cloned());
            used.extend(g.series_columns.iter().flatten().map(|c| c.design.clone()));
        }
    });
    d.graph_designs.iter().map(|x| x.name.clone()).filter(|n| !used.contains(n)).collect()
}

/// Design names from param `key`: one name or a list of names.
fn names_param(p: &Value, key: &str, cmd: &str) -> Result<Option<Vec<String>>> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::String(n)) => Ok(Some(vec![n.clone()])),
        Some(Value::Array(a)) if !a.is_empty() => a
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| bad(cmd, format!("`{key}` is a design name or a list of them"))))
            .collect::<Result<_>>()
            .map(Some),
        Some(_) => Err(bad(cmd, format!("`{key}` is a design name or a list of them"))),
    }
}

pub(super) fn design(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graph.design";
    for key in ["paste", "rename", "to"] {
        if p.get(key).is_some_and(|v| !v.is_string()) {
            return Err(bad(C, format!("`{key}` is a design name")));
        }
    }
    let taken = |s: &Session, name: &str| s.doc().is_ok_and(|st| st.doc.graph_designs.iter().any(|x| x.name == name));
    // New Design: the selected art, named `new` or "New Design" (numbered when that's taken).
    if let Some(v) = p.get("new") {
        let name = match v {
            Value::String(n) => GraphDesign::clean_name(n).ok_or_else(|| bad(C, "a design needs a name"))?,
            Value::Bool(true) | Value::Null => crate::cmd::unique_name("New Design", |n| taken(s, n)),
            _ => return Err(bad(C, "`new` is a design name, or true")),
        };
        if taken(s, &name) {
            return Err(bad(C, format!("there is already a design named `{name}`")));
        }
        if s.doc()?.doc.graph_designs.len() >= GraphDesign::MAX {
            return Err(bad(C, format!("a document keeps at most {} graph designs", GraphDesign::MAX)));
        }
        let art = crate::cmd::brushsym::selection_art(s, p)?.ok_or_else(|| bad(C, "select the art for the design"))?;
        if art.count_with_masks() > GraphDesign::MAX_NODES {
            return Err(bad(C, format!("a design holds at most {} objects", GraphDesign::MAX_NODES)));
        }
        // A graph saved as a design is kept as the art it draws.
        let art = GraphDesign::plain_art(&art);
        let saved = name.clone();
        s.edit("New Design", |d, _| {
            let art = d.reid(&art);
            d.graph_designs.push(GraphDesign { name: saved.clone(), art: Arc::new(art) });
            refresh_graphs_using(d, &saved)
        })?;
        return Ok(json!({ "name": name }));
    }
    // Rename: the graphs drawing the design follow it to its new name.
    if let Some(old) = str_param(p, "rename") {
        if !taken(s, old) {
            return Err(bad(C, format!("no design named `{old}`")));
        }
        let to = str_param(p, "to").and_then(GraphDesign::clean_name).ok_or_else(|| bad(C, "`to` is the design's new name"))?;
        if to == old {
            return Ok(json!({ "name": to }));
        }
        if taken(s, &to) {
            return Err(bad(C, format!("there is already a design named `{to}`")));
        }
        let (old, new) = (old.to_string(), to.clone());
        s.edit("Rename Design", |d, _| {
            if let Some(x) = d.graph_designs.iter_mut().find(|x| x.name == old) {
                x.name = new.clone();
            }
            let mut graphs = vec![];
            d.walk(|n| {
                if n.extras().graph.as_deref().is_some_and(|g| draws_with(g, &old)) {
                    graphs.push(n.id);
                }
            });
            for id in graphs {
                if let Some(g) = d.node_mut(id).and_then(|n| n.extras_if_any().and_then(|e| e.graph.as_deref_mut())) {
                    for m in g.series_markers.iter_mut().flatten().filter(|m| **m == old) {
                        *m = new.clone();
                    }
                    for c in g.series_columns.iter_mut().flatten().filter(|c| c.design == old) {
                        c.design = new.clone();
                    }
                }
            }
            Ok(())
        })?;
        return Ok(json!({ "name": to }));
    }
    if let Some(name) = str_param(p, "paste") {
        let art = s
            .doc()?
            .doc
            .graph_designs
            .iter()
            .find(|x| x.name == name)
            .map(|x| x.art.clone())
            .ok_or_else(|| bad(C, format!("no design named `{name}`")))?;
        let parent = s.doc()?.insertion_parent();
        let id = s.edit("Paste Design", |d, sel| {
            let n = d.reid(&art);
            let id = n.id;
            d.insert(parent, usize::MAX, n)?;
            sel.set([id]);
            Ok(id)
        })?;
        return Ok(json!({ "id": id.0 }));
    }
    if let Some(names) = names_param(p, "delete", C)? {
        // Each name once: a repeated one would regenerate the graphs using it again.
        let names: std::collections::BTreeSet<String> = names.into_iter().collect();
        if let Some(n) = names.iter().find(|n| !taken(s, n)) {
            return Err(bad(C, format!("no design named `{n}`")));
        }
        s.edit("Delete Design", |d, _| {
            d.graph_designs.retain(|x| !names.contains(&x.name));
            for name in &names {
                refresh_graphs_using(d, name)?;
            }
            Ok(())
        })?;
        return ok();
    }
    // Designs from a document loaded as a library: those the document has under the same name stay
    // as they are.
    if let Some(mut wanted) = names_param(p, "add", C)? {
        let lib = library_param(s, p, C)?.ok_or_else(|| bad(C, "missing `library`"))?;
        if let Some(n) = wanted.iter().find(|n| !lib.graph_designs.iter().any(|x| x.name == **n)) {
            return Err(bad(C, format!("no design named `{n}`")));
        }
        let mut seen = std::collections::HashSet::new();
        wanted.retain(|n| seen.insert(n.clone()));
        let (existing, added): (Vec<String>, Vec<String>) = wanted.into_iter().partition(|n| taken(s, n));
        if !added.is_empty() {
            s.edit("New Design", |d, _| bring_in(d, &lib, &added))?;
        }
        return Ok(json!({ "added": added, "existing": existing }));
    }
    if let Some(lib) = library_param(s, p, C)? {
        return Ok(json!({ "designs": design_names(&lib) }));
    }
    // The list, with the designs no graph draws with (Select Unused picks those).
    let d = &s.doc()?.doc;
    let mut out = json!({ "designs": design_names(d), "unused": unused_designs(d) });
    if let Some(libs) = libraries_json(s) {
        out["libraries"] = libs;
    }
    Ok(out)
}
