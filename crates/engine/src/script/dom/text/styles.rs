//! Character and paragraph styles in the script object model: `document.characterStyles` and
//! `document.paragraphStyles`, each `CharacterStyle` and `ParagraphStyle`, and the styles the text
//! of a `TextRange` uses, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CharacterStyles/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/CharacterStyle/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ParagraphStyles/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/ParagraphStyle/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/TextRange/> describe them.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - A document's collection lists its Normal style first, then its other styles in the order the
//!   panel lists them.
//! - A style object names its style for the rest of the run, also after the script renames it
//!   through `name`. Once the style is removed, or renamed or removed other than through the object
//!   model, or its document is closed or reverted with File › Revert, every member throws
//!   `Stale style`; a style added later under the same name is another style.
//! - A style name is trimmed, and must then be 1 to 256 characters long with no control
//!   characters.
//! - A style's `characterAttributes` and `paragraphAttributes` read `undefined` for an attribute the
//!   style does not set. Setting one redefines the style: text in the style takes the new value
//!   where it had the style's old one and keeps its other changes.
//! - `applyTo(text, clearingOverrides)` takes a text range, a text frame, or the characters, words
//!   or paragraphs of one, in the style's document. A character style styles exactly that text and
//!   changes nothing on an empty range; a paragraph style styles every paragraph the text touches.
//!   `clearingOverrides` left out is false; true resets the text to the Normal style first.
//! - A range's `characterStyles` and `paragraphStyles` list each style its characters or
//!   paragraphs are in once, in the order the text first uses it, with the Normal style for text in
//!   no other style. They do not add or remove styles.
//! - `remove()` and `removeAll()` never remove a Normal style. Text in a removed style keeps its
//!   look and is in the Normal style after.
//! - A script loads the styles of another document with `vc.run('charStyle.load', {path: ...})`,
//!   `paraStyle.load` or `textStyle.loadAll`, which read only a file the script may read.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{CharStyle, Shared, TextObject, TextStyleDef};
use vectorcraft_geom::Point;
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostMember, host_ref};

use super::{
    ATTRS, Answer, Attr, Doc, Host, Link, NOT_FOUND, PARA_ATTRS, ParaAttr, Span, arg, attr_param, class, command, doc_of, doc_ref, float, font_ref,
    function, get, handle_of, item_of, method, named, num, on_doc, para_param, para_range, property, range_ref, resolve, span_of, styles_in, text,
    text_of, this_range, typename_of, usize_of, whole_span, with_doc,
};
use crate::DocState;
use crate::cmd::textstyles::{NORMAL_CHAR, NORMAL_PARA, style_attrs};

/// The error of a style object whose style is gone.
pub const STALE_STYLE: &str = "Stale style";
/// The longest style name, in characters, after trimming.
pub const MAX_STYLE_NAME: usize = 256;
/// The most styles one run's style objects name.
pub const MAX_HELD: usize = 4096;

/// A style that a run's style objects name: its document and the document's revert count then, its
/// kind and its name now.
struct Slot {
    uid: u64,
    reverts: u64,
    para: bool,
    name: String,
    live: bool,
}

/// A document's uid and its character and paragraph style lists when the slots were last checked.
type Seen = (u64, Shared<Vec<TextStyleDef>>, Shared<Vec<TextStyleDef>>);

/// The styles a run's style objects name; an object holds its slot's number.
#[derive(Default)]
pub(super) struct StyleState {
    slots: RefCell<Vec<Slot>>,
    seen: RefCell<Vec<Seen>>,
}

/// Marks stale each style object whose style the command just run removed or renamed, so a style
/// removed and made again under its name by later commands, or by later steps of the same batch,
/// is another style. A document whose style lists did not change is not checked again. A live
/// slot's style was in its document's list when the command started, so a step that removed or
/// renamed it is in the trail's removed names.
pub(in crate::script) fn after_command(h: &Host) {
    let trail = std::mem::take(&mut h.session.borrow_mut().script_style_trail);
    let mut slots = h.dom.text.styles.slots.borrow_mut();
    if !slots.iter().any(|s| s.live) {
        return;
    }
    for slot in slots.iter_mut().filter(|x| x.live) {
        if trail.reverted.contains(&slot.uid)
            || (!trail.gone.is_empty() && slot.name != normal(slot.para) && trail.gone.contains(&(slot.uid, slot.para, slot.name.clone())))
        {
            slot.live = false;
        }
    }
    let s = h.session.borrow();
    let mut seen = h.dom.text.styles.seen.borrow_mut();
    seen.retain(|(uid, ..)| s.documents().iter().any(|d| d.uid == *uid));
    for st in s.documents() {
        let same = seen.iter().any(|(uid, c, p)| *uid == st.uid && Shared::ptr_eq(c, &st.doc.char_styles) && Shared::ptr_eq(p, &st.doc.para_styles));
        if same {
            continue;
        }
        seen.retain(|(uid, ..)| *uid != st.uid);
        seen.push((st.uid, st.doc.char_styles.clone(), st.doc.para_styles.clone()));
        for slot in slots.iter_mut().filter(|x| x.live && x.uid == st.uid) {
            slot.live = slot.reverts == st.reverts && has_style(st, slot.para, &slot.name);
        }
    }
    for slot in slots.iter_mut().filter(|x| x.live) {
        if !s.documents().iter().any(|d| d.uid == slot.uid) {
            slot.live = false;
        }
    }
}

/// The style objects' state when a savepoint was taken: each slot's name and whether it was
/// live, and the style lists last checked.
pub(in crate::script) struct Mark {
    slots: Vec<(String, bool)>,
    seen: Vec<Seen>,
}

/// The style objects' state now, for [`put_back`] when the savepoint taken with it is restored.
pub(in crate::script) fn mark(h: &Host) -> Mark {
    let styles = &h.dom.text.styles;
    let slots = styles.slots.borrow().iter().map(|s| (s.name.clone(), s.live)).collect();
    Mark { slots, seen: styles.seen.borrow().clone() }
}

/// Puts the style objects back as they were at `mark`, after the documents were put back to the
/// savepoint taken with it. A slot added since then stays live only while its document has the
/// style and no older live slot names it.
pub(in crate::script) fn put_back(h: &Host, mark: Mark) {
    let styles = &h.dom.text.styles;
    let mut slots = styles.slots.borrow_mut();
    let kept = mark.slots.len();
    for (slot, (name, live)) in slots.iter_mut().zip(mark.slots) {
        slot.name = name;
        slot.live = live;
    }
    let s = h.session.borrow();
    for i in kept..slots.len() {
        let (older, newer) = slots.split_at_mut(i);
        let Some(slot) = newer.first_mut() else { break };
        let named = older.iter().any(|o| o.live && o.uid == slot.uid && o.para == slot.para && o.name == slot.name);
        slot.live = !named && s.documents().iter().any(|st| st.uid == slot.uid && st.reverts == slot.reverts && has_style(st, slot.para, &slot.name));
    }
    *styles.seen.borrow_mut() = mark.seen;
}

fn normal(para: bool) -> &'static str {
    if para { NORMAL_PARA } else { NORMAL_CHAR }
}

fn cmd(para: bool, verb: &str) -> String {
    format!("{}.{verb}", if para { "paraStyle" } else { "charStyle" })
}

fn style_class(para: bool) -> &'static str {
    if para { "ParagraphStyle" } else { "CharacterStyle" }
}

fn list_class(para: bool) -> &'static str {
    if para { "ParagraphStyles" } else { "CharacterStyles" }
}

fn defs(st: &DocState, para: bool) -> &Vec<TextStyleDef> {
    if para { &st.doc.para_styles } else { &st.doc.char_styles }
}

/// The document's style names of a kind, the Normal style first.
fn names(st: &DocState, para: bool) -> Vec<String> {
    let mut out = vec![normal(para).to_string()];
    out.extend(defs(st, para).iter().map(|d| d.name.clone()).filter(|n| n != normal(para)));
    out
}

fn has_style(st: &DocState, para: bool, name: &str) -> bool {
    name == normal(para) || defs(st, para).iter().any(|d| d.name == name)
}

fn stale() -> HostError {
    HostError::Error(STALE_STYLE.into())
}

fn held_too_many() -> HostError {
    HostError::Range(format!("a script can name at most {MAX_HELD} styles in one run"))
}

/// The style object for the style `name` of the document `uid`: the same slot as every other
/// object naming it in this run.
fn style_ref(h: &Host, uid: u64, para: bool, name: &str) -> Answer {
    let reverts = h.session.borrow().documents().iter().find(|d| d.uid == uid).map_or(0, |d| d.reverts);
    let mut slots = h.dom.text.styles.slots.borrow_mut();
    let i = match slots.iter().position(|s| s.live && s.uid == uid && s.reverts == reverts && s.para == para && s.name == name) {
        Some(i) => i,
        None => {
            if slots.len() >= MAX_HELD {
                return Err(held_too_many());
            }
            slots.push(Slot { uid, reverts, para, name: name.to_string(), live: true });
            slots.len() - 1
        }
    };
    Ok(host_ref(style_class(para), &[float(h.run), float(uid), float(i as u64)]))
}

/// A style a style object names.
struct Style {
    doc: Doc,
    para: bool,
    slot: usize,
    name: String,
}

/// The style `d` names, when it is an object of one of `classes` (with the kind each names).
fn style_of(h: &Host, d: &Data, classes: &[(&str, bool)]) -> std::result::Result<Style, HostError> {
    let found = classes.iter().find_map(|(c, para)| handle_of(d, c).map(|handle| (handle, *para)));
    let Some((handle, para)) = found else {
        return Err(HostError::Type(format!("this is not a {}", classes.first().map_or("CharacterStyle", |c| c.0))));
    };
    let [run, uid, slot] = handle.as_slice() else { return Err(stale()) };
    let doc = super::super::find_doc(h, *run, *uid).ok_or_else(stale)?;
    let slot = usize_of(*slot).ok_or_else(stale)?;
    let mut slots = h.dom.text.styles.slots.borrow_mut();
    let s = slots.get_mut(slot).filter(|s| s.live && s.uid == doc.uid && s.para == para).ok_or_else(stale)?;
    if !with_doc(h, doc, |_, st| st.reverts == s.reverts && has_style(st, para, &s.name)) {
        s.live = false;
        return Err(stale());
    }
    Ok(Style { doc, para, slot, name: s.name.clone() })
}

fn this_style(h: &Host, a: &[Data]) -> std::result::Result<Style, HostError> {
    style_of(h, arg(a, 0), &[("CharacterStyle", false), ("ParagraphStyle", true)])
}

/// Marks the slots of the styles `names` of the document `uid` as removed.
fn forget(h: &Host, uid: u64, para: bool, names: &[String]) {
    for s in h.dom.text.styles.slots.borrow_mut().iter_mut() {
        if s.uid == uid && s.para == para && names.contains(&s.name) {
            s.live = false;
        }
    }
}

/// A style name a script gave, trimmed.
fn style_name(v: &Data) -> std::result::Result<String, HostError> {
    let Data::String(s) = v else { return Err(HostError::Type("a style name must be a string".into())) };
    if s.len() > super::super::MAX_TEXT_BYTES {
        return Err(HostError::Range(format!("a style name must be at most {MAX_STYLE_NAME} characters")));
    }
    let t = s.trim();
    if t.is_empty() {
        return Err(HostError::Range("a style name must not be empty".into()));
    }
    if t.chars().count() > MAX_STYLE_NAME {
        return Err(HostError::Range(format!("a style name must be at most {MAX_STYLE_NAME} characters")));
    }
    if t.chars().any(char::is_control) {
        return Err(HostError::Range("a style name must not hold control characters".into()));
    }
    Ok(t.to_string())
}

/// Runs the style command `verb` of the kind on the document of `doc`.
fn run(h: &Host, doc: Doc, para: bool, verb: &str, params: &Value) -> std::result::Result<Data, HostError> {
    on_doc(h, doc.uid, true, |h| command(h, &cmd(para, verb), params))
}

// ---- CharacterStyle and ParagraphStyle ----

fn style_name_get(h: &Host, a: &[Data]) -> Answer {
    Ok(text(this_style(h, a)?.name))
}

/// `name = …`: renames the style; the Normal style keeps its name.
fn style_name_set(h: &Host, a: &[Data]) -> Answer {
    let style = this_style(h, a)?;
    let to = style_name(arg(a, 1))?;
    if to == style.name {
        return Ok(Data::Undefined);
    }
    if style.name == normal(style.para) {
        return Err(HostError::Error(format!("{} cannot be renamed", style.name)));
    }
    if with_doc(h, style.doc, |_, st| has_style(st, style.para, &to)) {
        return Err(HostError::Error(format!("a style named {to} already exists")));
    }
    run(h, style.doc, style.para, "rename", &json!({"name": style.name, "to": to}))?;
    forget(h, style.doc.uid, style.para, std::slice::from_ref(&to));
    if let Some(s) = h.dom.text.styles.slots.borrow_mut().get_mut(style.slot) {
        s.name = to;
        s.live = true;
    }
    Ok(Data::Undefined)
}

fn style_parent(h: &Host, a: &[Data]) -> Answer {
    Ok(doc_ref(h, this_style(h, a)?.doc.uid))
}

fn style_typename(h: &Host, a: &[Data]) -> Answer {
    Ok(text(style_class(this_style(h, a)?.para)))
}

fn attributes(h: &Host, a: &[Data], class: &str) -> Answer {
    let style = this_style(h, a)?;
    Ok(host_ref(class, &[float(h.run), float(style.doc.uid), float(style.slot as u64)]))
}

fn style_character_attributes(h: &Host, a: &[Data]) -> Answer {
    attributes(h, a, "CharacterStyleAttributes")
}

fn style_paragraph_attributes(h: &Host, a: &[Data]) -> Answer {
    attributes(h, a, "ParagraphStyleAttributes")
}

/// `remove()`: deletes the style; its text keeps its look and is in the Normal style after.
fn style_remove(h: &Host, a: &[Data]) -> Answer {
    let style = this_style(h, a)?;
    if style.name == normal(style.para) {
        return Err(HostError::Error(format!("{} cannot be removed", style.name)));
    }
    run(h, style.doc, style.para, "delete", &json!({"name": style.name}))?;
    forget(h, style.doc.uid, style.para, &[style.name]);
    Ok(Data::Undefined)
}

/// The text a style is applied to: a text range, a text frame, or the characters, words or
/// paragraphs of one.
fn target_span(h: &Host, d: &Data) -> std::result::Result<Span, HostError> {
    if handle_of(d, "TextFrame").is_some() {
        return Ok(whole_span(h, item_of(h, d)?)?.1);
    }
    const TEXT: [&str; 4] = ["TextRange", "Characters", "Words", "Paragraphs"];
    if !TEXT.iter().any(|c| handle_of(d, c).is_some()) {
        return Err(HostError::Type("applyTo takes a TextRange or a TextFrame".into()));
    }
    span_of(h, d, &TEXT)
}

/// `applyTo(textItem, clearingOverrides?)`.
fn style_apply_to(h: &Host, a: &[Data]) -> Answer {
    let style = this_style(h, a)?;
    let target = target_span(h, arg(a, 1))?;
    let clear = match arg(a, 2) {
        Data::Undefined => false,
        Data::Bool(b) => *b,
        _ => return Err(HostError::Type("clearingOverrides must be true or false".into())),
    };
    if target.item.doc.uid != style.doc.uid {
        return Err(HostError::Error("the text is in another document than the style".into()));
    }
    let (index, s, e, _) = resolve(h, target)?;
    h.charge_call(16 + (e - s) as u64)?;
    let (start, end) = if style.para {
        let (_, ba, bb) = para_range(&index, s, e);
        (ba, bb)
    } else {
        // An empty range inside text changes nothing; empty text takes the style on its runs.
        if s == e && index.len() > 0 {
            return Ok(Data::Undefined);
        }
        (index.byte(s, false), index.byte(e, true))
    };
    let p = json!({"name": style.name, "clearOverrides": clear, "id": target.item.id.0, "start": start, "end": end});
    run(h, style.doc, style.para, "apply", &p)?;
    Ok(Data::Undefined)
}

// ---- the attributes of a style ----

/// The style attribute each `text.setRangeStyle` parameter `attr_param` gives sets.
const CHAR_KEYS: [(&str, &str); 11] = [
    ("size", "size"),
    ("leading", "leading"),
    ("tracking", "tracking"),
    ("baselineShift", "baseline_shift"),
    ("hScale", "h_scale"),
    ("vScale", "v_scale"),
    ("rotation", "rotation"),
    ("underline", "underline"),
    ("strikethrough", "strikethrough"),
    ("font", "font_family"),
    ("style", "font_style"),
];

/// The script's `Justification` members and the alignments a paragraph style stores.
const JUSTIFY: [(&str, &str); 7] = [
    ("LEFT", "Left"),
    ("CENTER", "Center"),
    ("RIGHT", "Right"),
    ("FULLJUSTIFYLASTLINELEFT", "JustifyLeft"),
    ("FULLJUSTIFYLASTLINECENTER", "JustifyCenter"),
    ("FULLJUSTIFYLASTLINERIGHT", "JustifyRight"),
    ("FULLJUSTIFY", "JustifyAll"),
];

/// The style an attributes object belongs to and the attributes it sets.
fn attrs_of(h: &Host, a: &[Data], para: bool) -> std::result::Result<(Style, Map<String, Value>), HostError> {
    let class = if para { "ParagraphStyleAttributes" } else { "CharacterStyleAttributes" };
    let style = style_of(h, arg(a, 0), &[(class, para)])?;
    let attrs = with_doc(h, style.doc, |_, st| style_attrs(&st.doc, para, &style.name)).ok_or_else(stale)?;
    Ok((style, attrs))
}

fn char_value(h: &Host, attr: Attr, m: &Map<String, Value>) -> Data {
    let n = |k: &str| m.get(k).and_then(Value::as_f64).map_or(Data::Undefined, num);
    let b = |k: &str| m.get(k).and_then(Value::as_bool).map_or(Data::Undefined, Data::Bool);
    match attr {
        Attr::Size => n("size"),
        Attr::Leading => n("leading"),
        Attr::AutoLeading => match m.get("leading") {
            Some(Value::Null) => Data::Bool(true),
            Some(v) if v.is_number() => Data::Bool(false),
            _ => Data::Undefined,
        },
        Attr::Tracking => n("tracking"),
        Attr::BaselineShift => n("baseline_shift"),
        Attr::HorizontalScale => n("h_scale"),
        Attr::VerticalScale => n("v_scale"),
        Attr::Rotation => n("rotation"),
        Attr::Underline => b("underline"),
        Attr::StrikeThrough => b("strikethrough"),
        Attr::TextFont => match m.get("font_family").and_then(Value::as_str) {
            Some(family) => font_ref(h, family, m.get("font_style").and_then(Value::as_str).unwrap_or("Regular")),
            None => Data::Undefined,
        },
    }
}

/// The `typename` of a style's `characterAttributes`, which throws once the style is gone.
fn attrs_typename_char(h: &Host, a: &[Data]) -> Answer {
    attrs_of(h, a, false)?;
    Ok(text("CharacterAttributes"))
}

/// The `typename` of a style's `paragraphAttributes`, which throws once the style is gone.
fn attrs_typename_para(h: &Host, a: &[Data]) -> Answer {
    attrs_of(h, a, true)?;
    Ok(text("ParagraphAttributes"))
}

fn read_char(h: &Host, a: &[Data], attr: Attr) -> Answer {
    let (_, m) = attrs_of(h, a, false)?;
    Ok(char_value(h, attr, &m))
}

/// Redefines the style with `attrs`, unless they are what it has.
fn redefine(h: &Host, style: &Style, old: &Map<String, Value>, attrs: Map<String, Value>) -> Answer {
    if attrs == *old {
        return Ok(Data::Undefined);
    }
    run(h, style.doc, style.para, "setAttrs", &json!({"name": style.name, "attrs": attrs}))?;
    Ok(Data::Undefined)
}

fn write_char(h: &Host, a: &[Data], attr: Attr, name: &str) -> Answer {
    let (style, old) = attrs_of(h, a, false)?;
    // `autoLeading = false` keeps the leading auto leading gives the style's size in a paragraph
    // with the default justification.
    let base = CharStyle::default();
    let size = old.get("size").and_then(Value::as_f64).unwrap_or(base.size);
    let leading = old.get("leading").and_then(Value::as_f64);
    let sample = CharStyle { size, leading, ..base };
    let t = TextObject::point(Point::default(), "", sample.clone());
    let (param, _) = attr_param(h, attr, name, arg(a, 1), (&t, &sample, 0))?;
    let mut attrs = old.clone();
    for (k, v) in param.as_object().into_iter().flatten() {
        let Some((_, key)) = CHAR_KEYS.iter().find(|(p, _)| *p == k.as_str()) else { continue };
        let v = if *key == "leading" && v == "auto" { Value::Null } else { v.clone() };
        attrs.insert((*key).to_string(), v);
    }
    redefine(h, &style, &old, attrs)
}

fn para_key(attr: ParaAttr) -> &'static str {
    match attr {
        ParaAttr::Justification => "justify",
        ParaAttr::FirstLineIndent => "first_line_indent",
        ParaAttr::LeftIndent => "left_indent",
        ParaAttr::RightIndent => "right_indent",
        ParaAttr::SpaceBefore => "space_before",
        ParaAttr::SpaceAfter => "space_after",
    }
}

fn read_para(h: &Host, a: &[Data], attr: ParaAttr) -> Answer {
    let (_, m) = attrs_of(h, a, true)?;
    let v = m.get(para_key(attr));
    Ok(match attr {
        ParaAttr::Justification => match v.and_then(Value::as_str) {
            Some("Auto") => text("Justification.LEFT"),
            Some(s) => JUSTIFY.iter().find(|(_, j)| *j == s).map_or(Data::Undefined, |(k, _)| text(format!("Justification.{k}"))),
            None => Data::Undefined,
        },
        _ => v.and_then(Value::as_f64).map_or(Data::Undefined, num),
    })
}

fn write_para(h: &Host, a: &[Data], attr: ParaAttr, name: &str) -> Answer {
    let (style, old) = attrs_of(h, a, true)?;
    let (_, _, wants) = para_param(attr, name, arg(a, 1))?;
    let value = match (&wants, attr) {
        (Data::String(s), ParaAttr::Justification) => {
            let m = s.strip_prefix("Justification.").unwrap_or(s);
            let found = JUSTIFY.iter().find(|(k, _)| *k == m).map(|(_, j)| *j);
            json!(found.ok_or_else(|| HostError::Type("justification must be a Justification value".into()))?)
        }
        (Data::Number(n), _) => json!(n),
        _ => return Err(HostError::Type(format!("{name} must be a finite number"))),
    };
    let mut attrs = old.clone();
    attrs.insert(para_key(attr).to_string(), value);
    redefine(h, &style, &old, attrs)
}

/// The body of a member that captures what it reads or sets.
type Body = Rc<dyn Fn(&Host, &[Data]) -> Answer>;

/// Members whose getter and setter each charge the call, as [`function`] does.
fn charged(link: &Link, g: impl Fn(&Host, &[Data]) -> Answer + 'static, s: impl Fn(&Host, &[Data]) -> Answer + 'static) -> HostMember {
    let wrap = |f: Body| -> HostFunction {
        let l = link.clone();
        Rc::new(move |a: Vec<Data>| {
            let h = super::super::current(&l)?;
            h.charge_call(16 + a.iter().map(super::super::data_size).sum::<u64>())?;
            f(&h, &a)
        })
    };
    HostMember::Property { get: wrap(Rc::new(g)), set: wrap(Rc::new(s)) }
}

fn char_members(link: &Link) -> Vec<(String, HostMember)> {
    ATTRS
        .iter()
        .map(|&(name, attr)| (name.to_string(), charged(link, move |h, a| read_char(h, a, attr), move |h, a| write_char(h, a, attr, name))))
        .collect()
}

fn para_members(link: &Link) -> Vec<(String, HostMember)> {
    PARA_ATTRS
        .iter()
        .map(|&(name, attr)| (name.to_string(), charged(link, move |h, a| read_para(h, a, attr), move |h, a| write_para(h, a, attr, name))))
        .collect()
}

// ---- CharacterStyles and ParagraphStyles ----

/// What a collection of styles lists: a document's styles, or those a text range uses.
enum Of {
    Doc(Doc),
    Range(Span),
}

fn list_of(h: &Host, d: &Data) -> std::result::Result<(Of, bool), HostError> {
    let para = match (handle_of(d, "CharacterStyles"), handle_of(d, "ParagraphStyles")) {
        (Some(_), _) => false,
        (_, Some(_)) => true,
        _ => return Err(HostError::Type("this is not a CharacterStyles".into())),
    };
    let class = list_class(para);
    let len = handle_of(d, class).map_or(0, |v| v.len());
    Ok((if len == 2 { Of::Doc(doc_of(h, d, class)?) } else { Of::Range(span_of(h, d, &[class])?) }, para))
}

/// The document and the names of the styles a collection lists.
fn listed(h: &Host, of: &Of, para: bool) -> std::result::Result<(Doc, Vec<String>), HostError> {
    match of {
        Of::Doc(doc) => {
            let names = with_doc(h, *doc, |_, st| names(st, para));
            h.charge_call(names.len() as u64)?;
            Ok((*doc, names))
        }
        Of::Range(r) => {
            let (index, s, e, _) = resolve(h, *r)?;
            let used = with_doc(h, r.item.doc, |_, st| {
                let t = text_of(st, r.item.id).ok_or_else(|| HostError::Error(super::STALE_ITEM.into()))?;
                let used: Vec<Option<String>> = if para {
                    let (paras, _, _) = para_range(&index, s, e);
                    paras.map(|i| t.para_at(i).style_name.clone()).collect()
                } else {
                    styles_in(t, index.byte(s, false), index.byte(e, true), false)?.into_iter().map(|(c, _)| c.style_name).collect()
                };
                h.charge_call(used.len() as u64)?;
                let mut out: Vec<String> = Vec::new();
                for n in used {
                    let n = n.filter(|n| has_style(st, para, n)).unwrap_or_else(|| normal(para).to_string());
                    if !out.contains(&n) {
                        out.push(n);
                    }
                }
                if out.is_empty() {
                    out.push(normal(para).to_string());
                }
                Ok::<_, HostError>(out)
            })?;
            Ok((r.item.doc, used))
        }
    }
}

fn list_length(h: &Host, a: &[Data]) -> Answer {
    let (of, para) = list_of(h, arg(a, 0))?;
    Ok(num(float(listed(h, &of, para)?.1.len() as u64)))
}

fn list_parent(h: &Host, a: &[Data]) -> Answer {
    let (of, _) = list_of(h, arg(a, 0))?;
    match of {
        Of::Doc(doc) => Ok(doc_ref(h, doc.uid)),
        Of::Range(r) => {
            let (_, s, e, stamp) = resolve(h, r)?;
            Ok(range_ref(h, r.item, s, e, stamp, "TextRange"))
        }
    }
}

fn list_at(h: &Host, a: &[Data]) -> Answer {
    let (of, para) = list_of(h, arg(a, 0))?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = usize_of(*n) else { return Ok(Data::Undefined) };
    let found = match &of {
        Of::Doc(doc) => with_doc(h, *doc, |_, st| match i {
            0 => Some(normal(para).to_string()),
            _ => defs(st, para).iter().map(|d| &d.name).filter(|n| *n != normal(para)).nth(i - 1).cloned(),
        })
        .map(|n| (*doc, n)),
        Of::Range(_) => {
            let (doc, names) = listed(h, &of, para)?;
            names.get(i).cloned().map(|n| (doc, n))
        }
    };
    match found {
        Some((doc, name)) => style_ref(h, doc.uid, para, &name),
        None => Ok(Data::Undefined),
    }
}

fn list_by_name(h: &Host, a: &[Data]) -> Answer {
    let (of, para) = list_of(h, arg(a, 0))?;
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    if name.len() > super::super::MAX_TEXT_BYTES {
        return Err(HostError::Range("the name is too long".into()));
    }
    let (doc, names) = listed(h, &of, para)?;
    match names.iter().find(|n| *n == name) {
        Some(n) => style_ref(h, doc.uid, para, n),
        None => Err(HostError::Error(NOT_FOUND.into())),
    }
}

fn list_index(h: &Host, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => list_by_name(h, a),
        Data::Number(_) => match list_at(h, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// The document of a document's collection; a range's collection cannot be changed.
fn changed_doc(h: &Host, a: &[Data]) -> std::result::Result<(Doc, bool), HostError> {
    match list_of(h, arg(a, 0))? {
        (Of::Doc(doc), para) => Ok((doc, para)),
        (Of::Range(_), _) => Err(HostError::Error("the styles of a text range cannot be added or removed".into())),
    }
}

/// `add(name)`: a new style that sets no attributes.
fn list_add(h: &Host, a: &[Data]) -> Answer {
    let (doc, para) = changed_doc(h, a)?;
    let name = style_name(arg(a, 1))?;
    if with_doc(h, doc, |_, st| has_style(st, para, &name)) {
        return Err(HostError::Error(format!("a style named {name} already exists")));
    }
    // The new style needs a slot of its own: a run that names as many styles as it may adds none.
    if h.dom.text.styles.slots.borrow().len() >= MAX_HELD {
        return Err(held_too_many());
    }
    run(h, doc, para, "new", &json!({"name": name, "attrs": {}}))?;
    // An object of a removed style of this name stays stale.
    forget(h, doc.uid, para, std::slice::from_ref(&name));
    style_ref(h, doc.uid, para, &name)
}

/// `removeAll()`: deletes every style but the Normal style, in one command.
fn list_remove_all(h: &Host, a: &[Data]) -> Answer {
    let (doc, para) = changed_doc(h, a)?;
    let gone: Vec<String> = with_doc(h, doc, |_, st| names(st, para)).into_iter().skip(1).collect();
    if gone.is_empty() {
        return Ok(Data::Undefined);
    }
    run(h, doc, para, "delete", &json!({"names": gone}))?;
    forget(h, doc.uid, para, &gone);
    Ok(Data::Undefined)
}

fn doc_char_styles(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("CharacterStyles", &[float(h.run), float(doc.uid)]))
}

fn doc_para_styles(h: &Host, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "Document")?;
    Ok(host_ref("ParagraphStyles", &[float(h.run), float(doc.uid)]))
}

fn range_styles(h: &Host, a: &[Data], para: bool) -> Answer {
    let r = this_range(h, a)?;
    let (_, s, e, stamp) = resolve(h, r)?;
    Ok(range_ref(h, r.item, s, e, stamp, list_class(para)))
}

fn range_char_styles(h: &Host, a: &[Data]) -> Answer {
    range_styles(h, a, false)
}

fn range_para_styles(h: &Host, a: &[Data]) -> Answer {
    range_styles(h, a, true)
}

/// `document.characterStyles` and `document.paragraphStyles`.
pub(in crate::script) fn doc_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("characterStyles", get(l, doc_char_styles)), ("paragraphStyles", get(l, doc_para_styles))])
}

/// `TextRange.characterStyles` and `TextRange.paragraphStyles`.
pub(super) fn range_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("characterStyles", get(l, range_char_styles)), ("paragraphStyles", get(l, range_para_styles))])
}

/// The style classes.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let style = |para: bool| {
        let mut members = named(vec![
            ("typename", get(l, style_typename)),
            ("name", property(l, style_name_get, style_name_set)),
            ("parent", get(l, style_parent)),
            ("applyTo", method(l, style_apply_to)),
            ("remove", method(l, style_remove)),
        ]);
        if para {
            members.push(("paragraphAttributes".into(), get(l, style_paragraph_attributes)));
            members.push(super::super::later(l, "ParagraphStyle", "characterAttributes"));
        } else {
            members.push(("characterAttributes".into(), get(l, style_character_attributes)));
        }
        class(style_class(para), None, members, None)
    };
    let list = |para: bool| {
        let name = list_class(para);
        class(
            name,
            None,
            named(vec![
                ("typename", typename_of(name)),
                ("length", get(l, list_length)),
                ("parent", get(l, list_parent)),
                ("index", method(l, list_index)),
                ("getByName", method(l, list_by_name)),
                ("add", method(l, list_add)),
                ("removeAll", method(l, list_remove_all)),
            ]),
            Some(function(l, list_at)),
        )
    };
    let mut chars = named(vec![("typename", get(l, attrs_typename_char))]);
    chars.extend(char_members(l));
    let mut paras = named(vec![("typename", get(l, attrs_typename_para))]);
    paras.extend(para_members(l));
    vec![
        style(false),
        style(true),
        list(false),
        list(true),
        class("CharacterStyleAttributes", None, chars, None),
        class("ParagraphStyleAttributes", None, paras, None),
    ]
}
