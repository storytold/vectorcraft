//! Effect › SVG Filters: the SVG filters a document holds (`svgFilter.list`, `define`, `delete`,
//! `import`) and applying one to objects as a live `svg.filter` effect (`svgFilter.apply`). An
//! applied effect names the document's filter: editing the filter changes what every object that
//! uses it draws, and deleting it leaves their effects in place, drawing nothing, until a filter of
//! that name exists again. Each effect holds the definition it draws with, kept in step here.
//!
//! The default filters ([`svg_filter::defaults`]) are listed for every document. Applying or
//! editing one adds a copy to the document under the default's name, and from then on that copy is
//! the filter of that name. Deleting the copy lists the default again.

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, SvgFilter};
use vectorcraft_render::effects::svg_filter::defaults::{DEFAULT_FILTERS, default_filter};
use vectorcraft_render::effects::{self, svg_filter};

use super::fileio::{absolute_path, read_file_head};
use super::{CommandSpec, Result, Session, bad, cmd, has_doc, text_param};

/// The largest SVG file Import SVG Filter reads.
const MAX_IMPORT_BYTES: usize = 16 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "svgFilter.list",
            "SVG Filters",
            [],
            None,
            "{} → {filters: [{name, xml}…], defaults: [{name, xml, inDocument}…]} filters: the SVG filters the document holds, in the order they were added; defaults: the default filters every document lists (VectorCraft's own set), with inDocument true when the document holds a filter of that name, which then stands in the default's place. The SVG Filters submenu and Apply SVG Filter list the defaults first, then the document's other filters",
            has_doc,
            list
        ),
        cmd!(
            "svgFilter.define",
            "New SVG Filter",
            [],
            None,
            "{xml: one SVG <filter> element (its id is its name; 1..64 primitives; at most 64 KB), replace?: the name of the filter it replaces and keeps (Edit SVG Filter)} add an SVG filter to the document, after the others, or replace one, as one undo step; every object whose svg.filter effect names it draws with the new definition; a name another filter of the document has is refused and a document holds at most 64 filters; replace with the name of a default filter the document does not hold adds the edited copy to the document under that name, and code equal to the default's adds nothing; a new filter may take a default's name, and then stands in its place → {name, filters}",
            has_doc,
            define
        ),
        cmd!(
            "svgFilter.delete",
            "Delete SVG Filter",
            [],
            None,
            "{name} remove an SVG filter from the document, as one undo step; objects whose svg.filter effect names it keep the effect, which draws nothing until a filter of that name is defined or imported again, or the default of that name is applied; a default filter the document holds no copy of is refused (the defaults are not deleted), and deleting the document's copy of one lists the default again → {filters}",
            has_doc,
            delete
        ),
        cmd!(
            "svgFilter.import",
            "Import SVG Filter…",
            [],
            None,
            "{path?|svg?} add the <filter> elements of an SVG file (at most 16 MB; not SVGZ; its first 256 filters) or of SVG text to the document's SVG filters, as one undo step; a filter whose name the document already has, and one svgFilter.define would refuse, is left out; objects whose effect names an added filter draw with it → {added: [names…], skipped: count, filters}",
            has_doc,
            import
        ),
        cmd!(
            "svgFilter.apply",
            "Apply SVG Filter",
            [],
            None,
            "{name, ids?|id?, item?, index?} apply the document's SVG filter `name`, or the default filter of that name when the document holds none (svgFilter.list), to ids or the selection as a live svg.filter effect, as one undo step, like effect.apply → {ids, index, item}; with index, the filter takes the place of the applied svg.filter effect at index of item, like effect.setParams → {ids}; a default is copied into the document in the same undo step, which is refused when the document already holds 64 filters",
            has_doc,
            apply
        ),
    ]
}

fn filters_json(d: &Document) -> Value {
    json!(d.svg_filters.iter().map(|f| json!({ "name": f.name, "xml": f.xml })).collect::<Vec<_>>())
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let held = |name: &str| d.svg_filters.iter().any(|f| f.name == name);
    let defaults: Vec<Value> = DEFAULT_FILTERS.iter().map(|f| json!({ "name": f.name, "xml": f.xml, "inDocument": held(f.name) })).collect();
    Ok(json!({ "filters": filters_json(d), "defaults": defaults }))
}

/// `p`'s `name`, trimmed and not empty.
fn name_param(p: &Value, cmd: &str) -> Result<String> {
    let n = text_param(p, "name", cmd)?.map(str::trim).filter(|n| !n.is_empty());
    n.map(str::to_string).ok_or_else(|| bad(cmd, "give `name`, an SVG filter's name"))
}

/// Is `e` an `svg.filter` effect that names filter `name`?
fn names(e: &vectorcraft_doc::Effect, name: &str) -> bool {
    e.id == effects::SVG_FILTER && e.params.get("name").and_then(Value::as_str) == Some(name)
}

/// The `svg.filter` effects of appearance `a` (its own and its fills' and strokes') that name
/// filter `name`.
fn named<'a>(a: &'a Appearance, name: &'a str) -> impl Iterator<Item = &'a vectorcraft_doc::Effect> + 'a {
    let items = a.items.iter().flat_map(|i| i.effects().iter());
    a.effects.iter().chain(items).filter(move |e| names(e, name))
}

/// Make the effects of appearance `a` that name filter `name` draw with `xml`.
fn update_appearance(a: &mut Appearance, name: &str, xml: &str) {
    let items = a.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
    for e in a.effects.iter_mut().chain(items) {
        if names(e, name) && e.params.is_object() {
            e.params["xml"] = json!(xml);
        }
    }
}

/// Does an effect in `d` that names filter `name` draw with something other than `xml` (`""`:
/// nothing)? Symbol definitions, graph designs, patterns, opacity masks and graphic styles count.
fn out_of_step(d: &Document, name: &str, xml: &str) -> bool {
    let stale = |a: &Appearance| named(a, name).any(|e| e.params.get("xml").and_then(Value::as_str) != Some(xml));
    d.any_node(|n| stale(&n.appearance)) || d.graphic_styles.iter().any(|s| stale(&s.appearance))
}

/// Make every effect in `d` that names filter `name` draw with `xml` (`""` when the document no
/// longer has the filter: the effect keeps its name and draws nothing), symbol definitions, graph
/// designs, patterns, opacity masks and graphic styles included.
fn update_users(d: &mut Document, name: &str, xml: &str) {
    // Through `DerefMut` only for an appearance that has such an effect (it is shared copy-on-write).
    d.edit_nodes_where(|n| named(&n.appearance, name).next().is_some(), |n| update_appearance(&mut n.appearance, name, xml));
    for s in d.graphic_styles.iter_mut().filter(|s| named(&s.appearance, name).next().is_some()) {
        update_appearance(&mut s.appearance, name, xml);
    }
}

fn define(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFilter.define";
    let xml = match p.get("xml") {
        Some(Value::String(x)) => x.trim().to_string(),
        Some(_) => return Err(bad(C, "`xml` must be a string")),
        None => return Err(bad(C, "give `xml`, one SVG <filter> element")),
    };
    let replace = match p.get("replace") {
        None | Some(Value::Null) => None,
        Some(Value::String(r)) => Some(r.trim().to_string()),
        Some(_) => return Err(bad(C, "`replace` must be a string")),
    };
    svg_filter::check(&xml).map_err(|e| bad(C, e))?;
    let filters = &s.doc()?.doc.svg_filters;
    let at = replace.as_ref().and_then(|r| filters.iter().position(|f| f.name == *r));
    // A default the document does not hold: its edited copy is added under the default's name.
    let default = replace.as_ref().filter(|_| at.is_none()).and_then(|r| default_filter(r));
    if let Some(r) = replace.as_ref().filter(|_| at.is_none() && default.is_none()) {
        return Err(bad(C, format!("the document has no SVG filter named {r:?}")));
    }
    // An edited filter keeps its name, whatever id its new code gives it.
    let xml = match &replace {
        Some(r) => svg_filter::with_id(&xml, r).ok_or_else(|| bad(C, "give one <filter> element"))?,
        None => xml,
    };
    let name = svg_filter::check(&xml).map_err(|e| bad(C, e))?;
    if filters.iter().enumerate().any(|(i, f)| f.name == name && Some(i) != at) {
        return Err(bad(C, format!("the document already has an SVG filter named {name:?}")));
    }
    let new = SvgFilter { name: name.clone(), xml };
    // Nothing to record when the filter, and every object that uses it, is already as given: a
    // default saved with its own code stays out of the document.
    let as_given = match default {
        Some(d) => d.to_filter() == new,
        None => at.and_then(|i| filters.get(i)) == Some(&new),
    };
    if as_given && !out_of_step(&s.doc()?.doc, &new.name, &new.xml) {
        return Ok(json!({ "name": name, "filters": filters_json(&s.doc()?.doc) }));
    }
    if at.is_none() && filters.len() >= SvgFilter::MAX {
        return Err(bad(C, format!("a document holds at most {} SVG filters", SvgFilter::MAX)));
    }
    let label = if replace.is_some() { "Edit SVG Filter" } else { "New SVG Filter" };
    s.edit(label, |d, _| {
        update_users(d, &new.name, &new.xml);
        match at.and_then(|i| d.svg_filters.get_mut(i)) {
            Some(f) => *f = new,
            None => d.svg_filters.push(new),
        }
        Ok(())
    })?;
    Ok(json!({ "name": name, "filters": filters_json(&s.doc()?.doc) }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFilter.delete";
    let name = name_param(p, C)?;
    if !s.doc()?.doc.svg_filters.iter().any(|f| f.name == name) {
        if default_filter(&name).is_some() {
            return Err(bad(C, format!("{name:?} is a default SVG filter, and the document holds no copy of it to delete")));
        }
        return Err(bad(C, format!("the document has no SVG filter named {name:?}")));
    }
    s.edit("Delete SVG Filter", |d, _| {
        d.svg_filters.retain(|f| f.name != name);
        update_users(d, &name, "");
        Ok(())
    })?;
    Ok(json!({ "filters": filters_json(&s.doc()?.doc) }))
}

/// The bytes of `p`'s `path` (an SVG file, read up to [`MAX_IMPORT_BYTES`]) or `svg`.
fn import_bytes(p: &Value, cmd: &str) -> Result<Vec<u8>> {
    let bytes = match (text_param(p, "path", cmd)?, text_param(p, "svg", cmd)?) {
        // One byte past the cap is enough to refuse the file below.
        (Some(path), _) => read_file_head(&absolute_path(path), MAX_IMPORT_BYTES + 1)?,
        (None, Some(svg)) => svg.as_bytes().to_vec(),
        _ => return Err(bad(cmd, "give `path`, an SVG file, or `svg`, SVG text")),
    };
    if bytes.len() > MAX_IMPORT_BYTES {
        return Err(bad(cmd, format!("the SVG file is larger than {MAX_IMPORT_BYTES} bytes")));
    }
    Ok(bytes)
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFilter.import";
    let bytes = import_bytes(p, C)?;
    // SVGZ would unpack past the size read above.
    if vectorcraft_svg::is_svgz(&bytes) {
        return Err(bad(C, "Import SVG Filter reads SVG files, not SVGZ"));
    }
    let text = vectorcraft_svg::text_of(&bytes).map_err(|e| bad(C, format!("the file isn't valid SVG: {e}")))?;
    let held = s.doc()?.doc.svg_filters.clone();
    let room = SvgFilter::MAX.saturating_sub(held.len());
    // Filters past the room left and names the document has are left out unchecked.
    let (found, skipped) = vectorcraft_svg::svg_filters(&text, room, |name| held.iter().any(|f| f.name == name))
        .map_err(|e| bad(C, format!("the file isn't valid SVG: {e}")))?;
    let added: Vec<SvgFilter> = found.into_iter().map(|(name, xml)| SvgFilter { name, xml }).collect();
    let names: Vec<String> = added.iter().map(|f| f.name.clone()).collect();
    if !added.is_empty() {
        s.edit("Import SVG Filter", |d, _| {
            for f in &added {
                update_users(d, &f.name, &f.xml);
            }
            d.svg_filters.extend(added);
            Ok(())
        })?;
    }
    Ok(json!({ "added": names, "skipped": skipped, "filters": filters_json(&s.doc()?.doc) }))
}

fn apply(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "svgFilter.apply";
    let name = name_param(p, C)?;
    let filters = &s.doc()?.doc.svg_filters;
    // The document's filter of that name, or the default of that name, which the document then takes.
    let (f, copied) = match (filters.iter().find(|f| f.name == name), default_filter(&name)) {
        (Some(f), _) => (f.clone(), false),
        (None, Some(_)) if filters.len() >= SvgFilter::MAX => {
            return Err(bad(C, format!("a document holds at most {} SVG filters, and applying a default filter adds it", SvgFilter::MAX)));
        }
        (None, Some(d)) => (d.to_filter(), true),
        (None, None) => return Err(bad(C, format!("the document has no SVG filter named {name:?}"))),
    };
    // A given `index`, null included, is checked as an index: only a missing one adds an effect.
    let replaces = p.get("index").is_some();
    let mut q = match replaces {
        true => json!({ "index": p["index"], "params": { "name": f.name, "xml": f.xml } }),
        false => json!({ "effect": effects::SVG_FILTER, "params": { "name": f.name, "xml": f.xml } }),
    };
    for key in ["ids", "id", "item"] {
        if let Some(v) = p.get(key) {
            q[key] = v.clone();
        }
    }
    if !copied {
        return apply_as_given(s, p, &q, replaces, None);
    }
    // The effect and the copy are two edits joined into one step. The history keeps room for both
    // until they are joined, so the application lets one old step go at most, as any edit does.
    let limit = s.doc()?.history.limit;
    if let Some(st) = s.active_mut() {
        st.history.limit = limit.saturating_add(2);
    }
    let out = apply_as_given(s, p, &q, replaces, Some(f));
    if let Some(st) = s.active_mut() {
        st.history.limit = limit;
        st.history.trim();
    }
    out
}

/// [`apply`] once the filter is known: the effect added (`q`) or put in the applied filter's place
/// (`replaces`), and `copy`, a default's copy, added to the document in the same undo step.
fn apply_as_given(s: &mut Session, p: &Value, q: &Value, replaces: bool, copy: Option<SvgFilter>) -> Result<Value> {
    const C: &str = "svgFilter.apply";
    let newest = s.doc()?.history.step().0;
    let out = if replaces {
        super::appearance::index_param(p, "index", C)?;
        // Only an applied SVG filter gives its place: another effect would take the filter's
        // parameters as its own.
        let edited = super::effectcmd::set_params_effects(s, q);
        if edited.is_empty() || edited.iter().any(|e| e != effects::SVG_FILTER) {
            return Err(bad(C, "`index` must be the index of an applied SVG filter"));
        }
        super::effectcmd::set_params(s, q)?
    } else {
        super::effectcmd::apply(s, q)?
    };
    if let Some(f) = copy {
        // The copy joins the step that applied it, under that step's name. That step is found by
        // its document, not its place: in an undo group both edits join the group's one step.
        let h = &s.doc()?.history;
        let applied = h.undo.last().filter(|_| h.step().0 != newest).map(|step| (step.label.clone(), step.doc.clone()));
        let label = applied.as_ref().map_or_else(|| "SVG Filter".to_string(), |(l, _)| l.clone());
        s.edit(&label, |d, _| {
            update_users(d, &f.name, &f.xml);
            d.svg_filters.push(f);
            Ok(())
        })?;
        let undo = &s.doc()?.history.undo;
        let below = undo.len().checked_sub(2);
        if let (Some(at), Some((_, doc))) = (below, &applied)
            && undo.get(at).is_some_and(|e| std::sync::Arc::ptr_eq(&e.doc, doc))
        {
            super::menucmds::squash(s, at, &label);
        }
    }
    Ok(out)
}
