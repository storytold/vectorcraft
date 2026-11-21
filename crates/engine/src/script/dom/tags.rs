//! `PageItem.tags`, `Document.tags` and the `Tag` objects in them, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Tags/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Tag/> describe them: name and value pairs on a
//! page item, which the native file keeps. Each change runs one of the `object.tag.*` commands.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - A `Tags` collection is live: it lists the tags in their order on the object, counted from 0.
//!   `Document.tags` lists the tags of every page item, in the order `pageItems` lists the items,
//!   and cannot add a tag, since a tag belongs to a page item.
//! - `add()` gives a tag with an empty name and value. Names may be empty or repeated, and
//!   `getByName` gives the first tag with the name.
//! - `name` and `value` are strings of at most 65,536 bytes, and an object has at most 64 tags.
//! - A tag that was deleted, or whose page item was, throws an error on every member.

use std::ops::ControlFlow;

use vectorcraft_doc::{MAX_TAG_TEXT, ObjectTag};

use super::{
    Answer, Doc, Host, Item, Link, NOT_FOUND, STALE_DOCUMENT, STALE_ITEM, arg, class, command, doc_of, doc_ref, edit_item, find_doc, float, get,
    handle_of, id_of, id_parts, item_of, item_ref, method, named, num, on_doc, page_items, property, text, typename, whole, with_doc,
};
use crate::NodeId;
use serde_json::json;
use vectorcraft_js::{Data, HostClass, HostError, HostMember, host_ref};

/// The error for a tag that was deleted, or that came from an earlier run.
pub const STALE_TAG: &str = "Stale tag";
/// The error of `Document.tags.add()`.
pub const NO_OWNER: &str = "A tag is added to a page item's tags, not to the document's";

/// What a `Tags` collection lists the tags of.
#[derive(Clone, Copy)]
enum Owner {
    Item(Item),
    Doc(Doc),
}

/// The owner of the `Tags` collection `d` names.
fn owner_of(h: &Host, d: &Data) -> std::result::Result<Owner, HostError> {
    let Some(handle) = handle_of(d, "Tags") else { return Err(HostError::Type("this is not a Tags collection".into())) };
    match handle.as_slice() {
        [run, uid] => find_doc(h, *run, *uid).map(Owner::Doc).ok_or_else(|| HostError::Error(STALE_DOCUMENT.into())),
        [run, uid, high, low] => Ok(Owner::Item(resolve_item(h, *run, *uid, *high, *low)?)),
        _ => Err(HostError::Type("this is not a Tags collection".into())),
    }
}

/// The page item a handle's run, document uid and id (as [`id_parts`] gave it) name, which must
/// still be in its document.
fn resolve_item(h: &Host, run: f64, uid: f64, high: f64, low: f64) -> std::result::Result<Item, HostError> {
    let doc = find_doc(h, run, uid).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let id = id_of(high, low).ok_or_else(|| HostError::Error(STALE_ITEM.into()))?;
    let alive = with_doc(h, doc, |_, st| st.doc.node(id).is_some_and(|n| typename(n).is_some()));
    if !alive {
        return Err(HostError::Error(STALE_ITEM.into()));
    }
    Ok(Item { doc, id })
}

/// Calls `f` with each tag `owner` lists, in order, and the page item it is on, until `f` breaks.
/// The tags are borrowed from the document, not copied. `Document.tags` charges the run by the
/// number of page items read.
fn each(h: &Host, owner: Owner, mut f: impl FnMut(NodeId, &ObjectTag) -> ControlFlow<()>) -> std::result::Result<(), HostError> {
    match owner {
        Owner::Item(item) => with_doc(h, item.doc, |_, st| {
            if let Some(n) = st.doc.node(item.id) {
                for t in crate::cmd::tags::tags_of(n) {
                    if f(item.id, t).is_break() {
                        break;
                    }
                }
            }
            Ok(())
        }),
        Owner::Doc(doc) => with_doc(h, doc, |_, st| {
            let ids = page_items(h, st);
            h.charge_call(ids.len() as u64)?;
            for id in ids.iter() {
                if let Some(n) = st.doc.node(*id) {
                    for t in crate::cmd::tags::tags_of(n) {
                        if f(*id, t).is_break() {
                            return Ok(());
                        }
                    }
                }
            }
            Ok(())
        }),
    }
}

/// The first tag `owner` lists for which `pick` holds: its page item and its id.
fn find(h: &Host, owner: Owner, mut pick: impl FnMut(&ObjectTag) -> bool) -> std::result::Result<Option<(NodeId, u64)>, HostError> {
    let mut found = None;
    each(h, owner, |id, t| {
        if pick(t) {
            found = Some((id, t.id));
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    })?;
    Ok(found)
}

fn doc_of_owner(owner: Owner) -> Doc {
    match owner {
        Owner::Item(i) => i.doc,
        Owner::Doc(d) => d,
    }
}

fn tag_ref(h: &Host, doc: Doc, id: NodeId, tag: u64) -> Data {
    let [high, low] = id_parts(id);
    let [tag_high, tag_low] = id_parts(NodeId(tag));
    host_ref("Tag", &[float(h.run), float(doc.uid), high, low, tag_high, tag_low])
}

/// `PageItem.tags`.
pub(super) fn item_tags(h: &Host, a: &[Data]) -> Answer {
    let item = item_of(h, arg(a, 0))?;
    let [high, low] = id_parts(item.id);
    Ok(host_ref("Tags", &[float(h.run), float(item.doc.uid), high, low]))
}

/// `Document.tags`.
pub(super) fn doc_tags(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("Tags", &[float(h.run), float(doc.uid)]))
}

// ---- Tags ----

fn tags_length(h: &Host, a: &[Data]) -> Answer {
    let owner = owner_of(h, arg(a, 0))?;
    let mut count = 0u64;
    each(h, owner, |_, _| {
        count += 1;
        ControlFlow::Continue(())
    })?;
    Ok(num(float(count)))
}

fn tags_parent(h: &Host, a: &[Data]) -> Answer {
    match owner_of(h, arg(a, 0))? {
        Owner::Item(item) => Ok(with_doc(h, item.doc, |_, st| item_ref(h, st, item.id))),
        Owner::Doc(doc) => Ok(doc_ref(h, doc.uid)),
    }
}

fn tags_at(h: &Host, a: &[Data]) -> Answer {
    let owner = owner_of(h, arg(a, 0))?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    let mut left = i;
    let found = find(h, owner, |_| {
        let here = left == 0;
        left = left.wrapping_sub(1);
        here
    })?;
    Ok(found.map_or(Data::Undefined, |(id, tag)| tag_ref(h, doc_of_owner(owner), id, tag)))
}

fn tags_by_name(h: &Host, a: &[Data]) -> Answer {
    let owner = owner_of(h, arg(a, 0))?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    let found = find(h, owner, |t| t.name == *name)?;
    found.map(|(id, tag)| tag_ref(h, doc_of_owner(owner), id, tag)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
}

fn tags_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => tags_by_name(h, a),
        Data::Number(_) => match tags_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `tags.add()`: a new tag with an empty name and value after the item's other tags.
fn tags_add(h: &Host, a: &[Data]) -> Answer {
    let item = match owner_of(h, arg(a, 0))? {
        Owner::Item(item) => item,
        Owner::Doc(_) => return Err(HostError::Error(NO_OWNER.into())),
    };
    edit_item(h, item, "object.tag.add", &json!({"id": item.id.0}))?;
    // The new tag is the item's last. Its id is read from the document, since the command's answer
    // gives it as a number, which is exact only up to 2^53.
    let tag = with_doc(h, item.doc, |_, st| st.doc.node(item.id).and_then(|n| crate::cmd::tags::tags_of(n).last().map(|t| t.id)));
    let tag = tag.ok_or_else(|| HostError::Error("the tag was not added".into()))?;
    Ok(tag_ref(h, item.doc, item.id, tag))
}

/// `tags.removeAll()`: deletes every tag in the collection, in one command.
fn tags_remove_all(h: &Host, a: &[Data]) -> Answer {
    let owner = owner_of(h, arg(a, 0))?;
    let mut ids: Vec<u64> = Vec::new();
    each(h, owner, |id, _| {
        if ids.last() != Some(&id.0) {
            ids.push(id.0);
        }
        ControlFlow::Continue(())
    })?;
    if !ids.is_empty() {
        let doc = doc_of_owner(owner);
        on_doc(h, doc.uid, true, |h| command(h, "object.tag.removeAll", &json!({"ids": ids})))?;
    }
    Ok(Data::Undefined)
}

// ---- Tag ----

/// A tag: its page item and its id there.
#[derive(Clone, Copy)]
struct TagRef {
    item: Item,
    tag: u64,
}

/// The tag `d` names, which must still be on its page item.
fn tag_of(h: &Host, d: &Data) -> std::result::Result<(TagRef, ObjectTag), HostError> {
    let Some(handle) = handle_of(d, "Tag") else { return Err(HostError::Type("this is not a Tag".into())) };
    let [run, uid, high, low, tag_high, tag_low] = handle.as_slice() else { return Err(HostError::Type("this is not a Tag".into())) };
    let item = resolve_item(h, *run, *uid, *high, *low)?;
    let tag = id_of(*tag_high, *tag_low).ok_or_else(|| HostError::Error(STALE_TAG.into()))?.0;
    let found = with_doc(h, item.doc, |_, st| st.doc.node(item.id).and_then(|n| crate::cmd::tags::tags_of(n).iter().find(|t| t.id == tag).cloned()));
    let t = found.ok_or_else(|| HostError::Error(STALE_TAG.into()))?;
    Ok((TagRef { item, tag }, t))
}

fn tag_name(h: &Host, a: &[Data]) -> Answer {
    Ok(text(tag_of(h, arg(a, 0))?.1.name))
}

fn tag_value(h: &Host, a: &[Data]) -> Answer {
    Ok(text(tag_of(h, arg(a, 0))?.1.value))
}

/// Sets the tag's `key` (`name` or `value`) to the string the script gives, with no command when
/// it already has it.
fn set_text(h: &Host, a: &[Data], key: &str) -> Answer {
    let Data::String(v) = arg(a, 1) else { return Err(HostError::Type(format!("{key} must be a string"))) };
    if v.len() > MAX_TAG_TEXT {
        return Err(HostError::Range(format!("a tag's {key} is longer than 65,536 bytes")));
    }
    let (t, now) = tag_of(h, arg(a, 0))?;
    let same = if key == "name" { now.name == *v } else { now.value == *v };
    if !same {
        edit_item(h, t.item, "object.tag.set", &json!({"id": t.item.id.0, "tag": t.tag, key: v}))?;
    }
    Ok(Data::Undefined)
}

fn tag_set_name(h: &Host, a: &[Data]) -> Answer {
    set_text(h, a, "name")
}

fn tag_set_value(h: &Host, a: &[Data]) -> Answer {
    set_text(h, a, "value")
}

fn tag_parent(h: &Host, a: &[Data]) -> Answer {
    let (t, _) = tag_of(h, arg(a, 0))?;
    Ok(with_doc(h, t.item.doc, |_, st| item_ref(h, st, t.item.id)))
}

fn tag_remove(h: &Host, a: &[Data]) -> Answer {
    let (t, _) = tag_of(h, arg(a, 0))?;
    edit_item(h, t.item, "object.tag.remove", &json!({"id": t.item.id.0, "tag": t.tag}))?;
    Ok(Data::Undefined)
}

/// `Tags.typename`, which throws like every other member once the collection's owner is gone.
fn tags_typename(h: &Host, a: &[Data]) -> Answer {
    owner_of(h, arg(a, 0))?;
    Ok(text("Tags"))
}

/// `Tag.typename`, which throws like every other member once the tag is gone.
fn tag_typename(h: &Host, a: &[Data]) -> Answer {
    tag_of(h, arg(a, 0))?;
    Ok(text("Tag"))
}

/// The `Tags` and `Tag` classes.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let tags: Vec<(String, HostMember)> = named(vec![
        ("typename", get(l, tags_typename)),
        ("length", get(l, tags_length)),
        ("parent", get(l, tags_parent)),
        ("index", method(l, tags_index)),
        ("getByName", method(l, tags_by_name)),
        ("add", method(l, tags_add)),
        ("removeAll", method(l, tags_remove_all)),
    ]);
    let tag = named(vec![
        ("typename", get(l, tag_typename)),
        ("name", property(l, tag_name, tag_set_name)),
        ("value", property(l, tag_value, tag_set_value)),
        ("parent", get(l, tag_parent)),
        ("remove", method(l, tag_remove)),
    ]);
    vec![class("Tags", None, tags, Some(super::function(l, tags_at))), class("Tag", None, tag, None)]
}
