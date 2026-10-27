use super::edit::regenerate;
use super::*;

/// The document's graph design names, in order.
pub(super) fn design_names(d: &Document) -> Vec<String> {
    d.graph_designs.iter().map(|x| x.name.clone()).collect()
}

/// Whether graph `g` draws a series with design `name`, as its marker or its columns.
fn draws_with(g: &GraphSpec, name: &str) -> bool {
    g.series_markers.iter().flatten().any(|m| m == name) || g.series_columns.iter().flatten().any(|c| c.design == name)
}

/// Regenerate the graphs whose series use design `name` (it was added or removed).
fn refresh_graphs_using(d: &mut Document, name: &str) -> Result<()> {
    let mut ids = vec![];
    d.walk(|n| {
        if n.extras().graph.as_deref().is_some_and(|g| draws_with(g, name)) {
            ids.push(n.id);
        }
    });
    for id in ids {
        if let Some(spec) = d.node(id).and_then(|n| n.extras().graph.as_deref().cloned()) {
            regenerate(d, id, spec)?;
        }
    }
    Ok(())
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
    // The list, with the designs no graph draws with (Select Unused picks those).
    let d = &s.doc()?.doc;
    Ok(json!({ "designs": design_names(d), "unused": unused_designs(d) }))
}
