//! An object's tags: name and value pairs scripts keep on page items (`object.tags`,
//! `object.tag.add`, `object.tag.set`, `object.tag.remove`, `object.tag.removeAll`). The native file
//! keeps them; other formats do not write them.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, MAX_TAG_TEXT, MAX_TAGS, Node, ObjectTag};

use super::{CommandSpec, NodeId, Result, Session, bad, cmd, has_doc, ids_param, targets, text_param, u64_req};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "object.tags",
            "Attributes",
            [],
            None,
            "{id} → {tags: [{tag, name, value}]} the object's tags in order; `tag` tells each apart from the others while its name and value change",
            has_doc,
            list
        ),
        cmd!(
            "object.tag.add",
            "Attributes",
            [],
            None,
            "{id, name?: string, value?: string} adds a tag after the object's others (\"\" for a name or value left out), as one undo step; an object has at most 64 tags, each name and value at most 65,536 bytes → {tag}",
            has_doc,
            add
        ),
        cmd!(
            "object.tag.set",
            "Attributes",
            [],
            None,
            "{id, tag, name?: string, value?: string} changes the tag's name or value, as one undo step; a value it already has changes nothing → {changed: bool}",
            has_doc,
            set
        ),
        cmd!("object.tag.remove", "Attributes", [], None, "{id, tag} deletes the tag, as one undo step → {}", has_doc, remove),
        cmd!(
            "object.tag.removeAll",
            "Attributes",
            [],
            None,
            "{ids?|id?} deletes every tag of the objects in ids or the selection, as one undo step → {changed: objects}",
            has_doc,
            remove_all
        ),
    ]
}

/// The page item `p`'s `id` names: an object of the document, not a layer or a ruler guide.
fn object(s: &Session, p: &Value, c: &str) -> Result<NodeId> {
    let id = NodeId(u64_req(p, "id", c)?);
    let d = &s.doc()?.doc;
    match d.node(id) {
        Some(n) if is_page_item(n) => Ok(id),
        Some(_) => Err(bad(c, format!("object {} is a layer or a ruler guide, which has no tags", id.0))),
        None => Err(bad(c, format!("no object {}", id.0))),
    }
}

fn is_page_item(n: &Node) -> bool {
    !matches!(n.kind, vectorcraft_doc::NodeKind::Layer { .. }) && !n.is_ruler_guide()
}

/// The tags of `n`.
pub(crate) fn tags_of(n: &Node) -> &[ObjectTag] {
    n.extras().attrs.as_deref().map_or(&[], |a| a.tags.as_slice())
}

/// The tag `p`'s `tag` names on object `id`: its index among the object's tags.
fn tag_index(d: &Document, id: NodeId, p: &Value, c: &str) -> Result<usize> {
    let tag = u64_req(p, "tag", c)?;
    let n = d.node(id).ok_or_else(|| bad(c, format!("no object {}", id.0)))?;
    tags_of(n).iter().position(|t| t.id == tag).ok_or_else(|| bad(c, format!("object {} has no tag {tag}", id.0)))
}

/// The text `p` gives for `key` (a tag's name or value), at most [`MAX_TAG_TEXT`] bytes. `None`
/// when `p` leaves it out; a null is an error.
fn tag_text<'a>(p: &'a Value, key: &str, c: &str) -> Result<Option<&'a str>> {
    if p.get(key).is_some_and(Value::is_null) {
        return Err(bad(c, format!("`{key}` must be a string")));
    }
    let t = text_param(p, key, c)?;
    if t.is_some_and(|t| t.len() > MAX_TAG_TEXT) {
        return Err(bad(c, format!("a tag's {key} is longer than 65,536 bytes")));
    }
    Ok(t)
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tags";
    let id = object(s, p, C)?;
    let d = &s.doc()?.doc;
    let tags = d.node(id).map(tags_of).unwrap_or_default();
    Ok(json!({"tags": tags.iter().map(|t| json!({"tag": t.id, "name": t.name, "value": t.value})).collect::<Vec<_>>()}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tag.add";
    let id = object(s, p, C)?;
    let name = tag_text(p, "name", C)?.unwrap_or_default().to_string();
    let value = tag_text(p, "value", C)?.unwrap_or_default().to_string();
    let count = s.doc()?.doc.node(id).map_or(0, |n| tags_of(n).len());
    if count >= MAX_TAGS {
        return Err(bad(C, format!("object {} has 64 tags, the most an object has", id.0)));
    }
    let tag = s.edit("Attributes", |d, _| {
        // A document-wide id: a tag deleted earlier in the run never shares it.
        let tag = d.alloc_tag_id(id).ok_or_else(|| bad(C, "the document has no tag ids left"))?;
        let n = d.node_mut(id).ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
        n.edit_attrs(|a| a.tags.push(ObjectTag { id: tag, name, value }));
        Ok(tag)
    })?;
    Ok(json!({"tag": tag}))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tag.set";
    let id = object(s, p, C)?;
    let name = tag_text(p, "name", C)?;
    let value = tag_text(p, "value", C)?;
    if name.is_none() && value.is_none() {
        return Err(bad(C, "give name or value"));
    }
    let at = tag_index(&s.doc()?.doc, id, p, C)?;
    let same = s
        .doc()?
        .doc
        .node(id)
        .and_then(|n| tags_of(n).get(at))
        .is_some_and(|t| name.is_none_or(|v| t.name == v) && value.is_none_or(|v| t.value == v));
    if same {
        return Ok(json!({"changed": false}));
    }
    s.edit("Attributes", |d, _| {
        let n = d.node_mut(id).ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
        n.edit_attrs(|a| {
            if let Some(t) = a.tags.get_mut(at) {
                if let Some(v) = name {
                    v.clone_into(&mut t.name);
                }
                if let Some(v) = value {
                    v.clone_into(&mut t.value);
                }
            }
        });
        Ok(())
    })?;
    Ok(json!({"changed": true}))
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tag.remove";
    let id = object(s, p, C)?;
    let at = tag_index(&s.doc()?.doc, id, p, C)?;
    s.edit("Attributes", |d, _| {
        let n = d.node_mut(id).ok_or_else(|| bad(C, format!("no object {}", id.0)))?;
        let gone = n.edit_attrs(|a| (at < a.tags.len()).then(|| a.tags.remove(at).id));
        if let Some(t) = gone {
            d.raise_next_id(t.saturating_add(1));
        }
        Ok(())
    })?;
    Ok(json!({}))
}

fn remove_all(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.tag.removeAll";
    ids_param(p, "ids", C)?;
    let ids = targets(s, p)?;
    let d = &s.doc()?.doc;
    for id in &ids {
        if d.node(*id).is_none() {
            return Err(bad(C, format!("no object {}", id.0)));
        }
    }
    let tagged: BTreeSet<NodeId> = ids.into_iter().filter(|id| d.node(*id).is_some_and(|n| !tags_of(n).is_empty())).collect();
    if !tagged.is_empty() {
        s.edit("Attributes", |d, _| {
            let mut last = 0;
            for id in &tagged {
                if let Some(n) = d.node_mut(*id) {
                    last = n.edit_attrs(|a| a.tags.drain(..).map(|t| t.id).fold(last, u64::max));
                }
            }
            d.raise_next_id(last.saturating_add(1));
            Ok(())
        })?;
    }
    Ok(json!({"changed": tagged.len()}))
}
