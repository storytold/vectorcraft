//! The document resources of the script object model: swatches, color groups, spots, symbols and
//! symbol instances, graphic styles and brushes, as the help pages
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Swatches/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Swatch/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/SwatchGroups/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/SwatchGroup/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Spots/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Spot/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Symbols/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Symbol/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/SymbolItems/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/SymbolItem/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/GraphicStyles/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/GraphicStyle/>,
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Brushes/> and
//! <https://ai-scripting.docsforadobe.dev/jsobjref/Brush/> describe them.
//!
//! Every change runs the command the Swatches, Symbols, Graphic Styles or Brushes panel runs, so it
//! joins the run's transaction; a change that changes nothing runs no command.
//!
//! VectorCraft's own rules, which the help pages do not state:
//! - A resource object holds the run, the document and a slot of the run's [`Table`], which names
//!   the resource. Renaming it through the object model keeps the object; removing it, or a rename
//!   or deletion that `vc.run` makes, leaves the object stale, and every member then throws
//!   `Stale Swatch` (`Stale Symbol`, and so on).
//! - `swatches` lists the swatches in the Swatches panel's order: None, Registration, the other
//!   ungrouped swatches, then each color group's. `spots` lists Registration and the spot colors in
//!   the same order. `swatchGroups` lists the color groups only.
//! - `swatches.add()` saves the current fill (a color, gradient or pattern) as a swatch, or black
//!   when the fill is None; `spots.add()` saves the current fill, in the document's color space
//!   unless it is Lab, or black when it is not a solid color, as a spot color. `getSelected()` gives
//!   `[]`, since a script has no Swatches panel selection.
//! - None and Registration cannot be renamed, recolored, grouped or removed, and `removeAll()`
//!   keeps them. The Default Graphic Style cannot be renamed or removed, and
//!   `graphicStyles.removeAll()` keeps it.
//! - `SwatchGroup.remove()` deletes the group and its swatches; `removeAll()` deletes its swatches
//!   and keeps the group. A group holds solid colors only.
//! - A spot's color, set through `Spot.color` or `Swatch.color`, takes an RGB, CMYK or Lab color
//!   in a document of either color space, and the spot keeps that color space. The Spot page says
//!   a color in another space than the document's gives an error; `spots.add()` follows it, and a
//!   spot's color does not. A process swatch's `Swatch.color` stores an RGB color in a CMYK
//!   document as CMYK. `colorType` reads `ColorModel.SPOT`, or
//!   `ColorModel.REGISTRATION` for Registration, and takes only that value. `spotKind` reads
//!   `SpotColorKind.SPOTCMYK` for a gray spot. `getInternalColor()` gives the components on the
//!   scale of the color classes (0 to 255 for RGB, 0 to 100 for CMYK and gray, L*a*b* as is).
//! - A `SpotColor` assigned to a path's `fillColor` or `strokeColor` links the paint to its `spot`,
//!   a Spot of the path's document, at its `tint` (VectorCraft's own defaults: 100 from
//!   `new SpotColor()`, 0 for a color object without a `tint`);
//!   reading the paint back gives that `spot`. A paint linked to a global process swatch reads as
//!   a `SpotColor` without a `spot`, which does not name the swatch, so assigning it throws an
//!   error that says it is not available yet, with nothing changed.
//! - `symbols.add(art)` keeps the art where it is and does not make it an instance.
//!   `Symbol.remove()` and `symbols.removeAll()` turn the instances into plain art.
//! - `applyTo` and `mergeTo` take one page item of the same document that can be edited;
//!   `Brush.applyTo` takes a path that is not a guide, a clipping path or a path in a compound path.
//! - A name is 1 to 1024 bytes, not blank, with no control characters.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::swatch::{self, Swatch};
use vectorcraft_color::{Color, Paint};
use vectorcraft_js::{Data, HostClass, HostError, HostFunction, HostItem, HostMember, Realm, RunError, host_ref};

use super::{
    Answer, CROSS_DOCUMENT, Doc, Host, Item, Link, NOT_FOUND, arg, class, colors, command, current, data_size, doc_of, doc_ref, editable, find_doc,
    float, given, handle_of, item_of, item_ref, later, named, num, on_doc, page_items, text, this_doc, typename_of, whole, with_doc,
};
use crate::{DocState, Document, NodeId, NodeKind};

/// The longest resource name a script may give, in bytes.
pub const MAX_NAME_BYTES: usize = 1024;
/// The most resource objects one run may hold.
pub const MAX_SLOTS: usize = 100_000;

/// The kinds of resource object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Kind {
    Swatch,
    Spot,
    Group,
    Symbol,
    Style,
    Brush,
}

impl Kind {
    fn class(self) -> &'static str {
        match self {
            Kind::Swatch => "Swatch",
            Kind::Spot => "Spot",
            Kind::Group => "SwatchGroup",
            Kind::Symbol => "Symbol",
            Kind::Style => "GraphicStyle",
            Kind::Brush => "Brush",
        }
    }

    fn collection(self) -> &'static str {
        match self {
            Kind::Swatch => "Swatches",
            Kind::Spot => "Spots",
            Kind::Group => "SwatchGroups",
            Kind::Symbol => "Symbols",
            Kind::Style => "GraphicStyles",
            Kind::Brush => "Brushes",
        }
    }

    /// The kinds that name the same records: a spot is also a swatch.
    fn family(self) -> &'static [Kind] {
        match self {
            Kind::Swatch | Kind::Spot => &[Kind::Swatch, Kind::Spot],
            Kind::Group => &[Kind::Group],
            Kind::Symbol => &[Kind::Symbol],
            Kind::Style => &[Kind::Style],
            Kind::Brush => &[Kind::Brush],
        }
    }

    fn stale(self) -> HostError {
        HostError::Error(format!("Stale {}", self.class()))
    }
}

const KINDS: [Kind; 6] = [Kind::Swatch, Kind::Spot, Kind::Group, Kind::Symbol, Kind::Style, Kind::Brush];

/// The resources a run's objects name: each slot holds a document's uid, a kind and the
/// resource's name, or `None` once it was removed.
#[derive(Default)]
pub(super) struct Table {
    slots: Vec<(u64, Kind, Option<String>)>,
    /// For each document's uid, the slot of each kind and name it holds.
    index: HashMap<u64, HashMap<(Kind, String), usize>>,
    /// Each document as [`forget_removed`] last checked it.
    checked: HashMap<u64, Arc<Document>>,
}

impl Table {
    /// Marks every slot stale whose document is no longer open, or whose resource `docs` no
    /// longer has. A document unchanged since the last check is skipped, and its slots are not
    /// visited. → how many slots and resource names it compared.
    fn forget(&mut self, docs: &[DocState]) -> u64 {
        let mut work = 0u64;
        for (uid, held) in &mut self.index {
            let st = docs.iter().find(|d| d.uid == *uid);
            if let (Some(st), Some(seen)) = (st, self.checked.get(uid))
                && Arc::ptr_eq(seen, &st.doc)
            {
                continue;
            }
            work += held.len() as u64;
            let mut present: HashMap<Kind, HashSet<String>> = HashMap::new();
            let slots = &mut self.slots;
            held.retain(|(k, n), i| {
                let kept = st.is_some_and(|st| present.entry(*k).or_insert_with(|| names(&st.doc, *k).into_iter().collect()).contains(n));
                if !kept {
                    slots[*i].2 = None;
                }
                kept
            });
            work += present.values().map(|p| p.len() as u64).sum::<u64>();
            match st {
                Some(st) => self.checked.insert(*uid, st.doc.clone()),
                None => self.checked.remove(uid),
            };
        }
        let checked = &mut self.checked;
        self.index.retain(|uid, held| {
            if held.is_empty() {
                checked.remove(uid);
            }
            !held.is_empty()
        });
        work
    }

    /// The slot of `name`, made when there is none.
    fn slot(&mut self, uid: u64, kind: Kind, name: &str) -> std::result::Result<usize, HostError> {
        if let Some(i) = self.index.get(&uid).and_then(|held| held.get(&(kind, name.to_string()))) {
            return Ok(*i);
        }
        if self.slots.len() >= MAX_SLOTS {
            return Err(HostError::Range(format!("a script may hold at most {MAX_SLOTS} swatches, symbols, styles and brushes")));
        }
        self.slots.push((uid, kind, Some(name.to_string())));
        self.index.entry(uid).or_default().insert((kind, name.to_string()), self.slots.len() - 1);
        Ok(self.slots.len() - 1)
    }

    /// The name slot `i` holds for `uid` and `kind`.
    fn name(&self, i: usize, uid: u64, kind: Kind) -> Option<&str> {
        match self.slots.get(i) {
            Some((u, k, Some(n))) if *u == uid && *k == kind => Some(n),
            _ => None,
        }
    }

    /// The resource `from` of the family of `kind` is now called `to`; objects that named `to`
    /// before go stale.
    fn rename(&mut self, uid: u64, kind: Kind, from: &str, to: &str) {
        self.remove(uid, kind, to);
        let Some(held) = self.index.get_mut(&uid) else { return };
        for k in kind.family() {
            if let Some(i) = held.remove(&(*k, from.to_string())) {
                self.slots[i].2 = Some(to.to_string());
                held.insert((*k, to.to_string()), i);
            }
        }
    }

    /// The resource `name` of the family of `kind` is gone.
    fn remove(&mut self, uid: u64, kind: Kind, name: &str) {
        let Some(held) = self.index.get_mut(&uid) else { return };
        for k in kind.family() {
            if let Some(i) = held.remove(&(*k, name.to_string())) {
                self.slots[i].2 = None;
            }
        }
    }
}

/// After a `vc.run` command: the objects whose resource the command removed or renamed go stale.
/// The slots and names the check compares are charged to the run.
pub(super) fn forget_removed(h: &Host) -> std::result::Result<(), HostError> {
    let work = {
        let s = h.session.borrow();
        h.dom.resources.borrow_mut().forget(s.documents())
    };
    if work > 0 {
        h.charge_call(work)?;
    }
    Ok(())
}

type Kfn = fn(&Host, Kind, &[Data]) -> Answer;

fn kfunction(link: &Link, kind: Kind, f: Kfn) -> HostFunction {
    let l = link.clone();
    Rc::new(move |a: Vec<Data>| {
        let h = current(&l)?;
        h.charge_call(16 + a.iter().map(data_size).sum::<u64>())?;
        f(&h, kind, &a)
    })
}

fn kget(l: &Link, kind: Kind, f: Kfn) -> HostMember {
    HostMember::Get(kfunction(l, kind, f))
}

fn kproperty(l: &Link, kind: Kind, g: Kfn, s: Kfn) -> HostMember {
    HostMember::Property { get: kfunction(l, kind, g), set: kfunction(l, kind, s) }
}

fn kmethod(l: &Link, kind: Kind, f: Kfn) -> HostMember {
    HostMember::Method(kfunction(l, kind, f))
}

/// A name a script gave: a string of 1 to [`MAX_NAME_BYTES`] bytes, not blank, without control
/// characters.
fn name_arg(d: &Data, what: &str) -> std::result::Result<String, HostError> {
    let Data::String(s) = d else { return Err(HostError::Type(format!("{what} must be a string"))) };
    if s.len() > MAX_NAME_BYTES {
        return Err(HostError::Range(format!("{what} is longer than {MAX_NAME_BYTES} bytes")));
    }
    if s.chars().any(char::is_control) {
        return Err(HostError::Range(format!("{what} cannot hold control characters")));
    }
    if s.trim().is_empty() {
        return Err(HostError::Range(format!("{what} cannot be empty")));
    }
    Ok(s.clone())
}

// ---- reading the records ----

/// The swatches in panel order, each with its color group.
fn all_swatches(d: &Document) -> Vec<(&Swatch, Option<&str>)> {
    let none = d.swatches.iter().take_while(|w| w.paint.is_none()).count();
    let (specials, rest) = d.swatches.split_at(none);
    let ungrouped = specials.iter().chain([swatch::registration()]).chain(rest).map(|w| (w, None));
    let grouped = d.swatch_groups.iter().flat_map(|g| g.swatches.iter().map(|w| (w, Some(g.name.as_str()))));
    ungrouped.chain(grouped).collect()
}

fn find_swatch<'a>(d: &'a Document, name: &str) -> Option<(&'a Swatch, Option<&'a str>)> {
    all_swatches(d).into_iter().find(|(w, _)| w.name == name)
}

/// Whether `spots` lists `w`: a spot color, or Registration.
fn is_spot(w: &Swatch) -> bool {
    w.spot || w.name == swatch::REGISTRATION
}

/// The names of the resources of `kind` in `d`, in their panel's order.
fn names(d: &Document, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Swatch => all_swatches(d).into_iter().map(|(w, _)| w.name.clone()).collect(),
        Kind::Spot => all_swatches(d).into_iter().filter(|(w, _)| is_spot(w)).map(|(w, _)| w.name.clone()).collect(),
        Kind::Group => d.swatch_groups.iter().map(|g| g.name.clone()).collect(),
        Kind::Symbol => d.symbols.iter().map(|s| s.name.clone()).collect(),
        Kind::Style => d.graphic_styles.iter().map(|g| g.name.clone()).collect(),
        Kind::Brush => vectorcraft_brush::library(d).into_iter().map(|b| b.name).collect(),
    }
}

/// Whether `d` has the resource `name` of `kind`.
fn exists(d: &Document, kind: Kind, name: &str) -> bool {
    match kind {
        Kind::Swatch => find_swatch(d, name).is_some(),
        Kind::Spot => find_swatch(d, name).is_some_and(|(w, _)| is_spot(w)),
        Kind::Group => d.swatch_groups.iter().any(|g| g.name == name),
        Kind::Symbol => d.symbols.iter().any(|s| s.name == name),
        Kind::Style => d.graphic_style(name).is_some(),
        Kind::Brush => vectorcraft_brush::find(d, name).is_some(),
    }
}

/// The object for the resource `name` of `kind` in the document `uid`.
fn resource_ref(h: &Host, uid: u64, kind: Kind, name: &str) -> Answer {
    let slot = h.dom.resources.borrow_mut().slot(uid, kind, name)?;
    Ok(host_ref(kind.class(), &[float(h.run), float(uid), float(slot as u64)]))
}

/// A resource an object names: its document and its name now.
struct Res {
    doc: Doc,
    name: String,
}

/// The resource `d` names, which must be an object of `kind` that is not stale.
fn resource_of(h: &Host, d: &Data, kind: Kind) -> std::result::Result<Res, HostError> {
    let Some(handle) = handle_of(d, kind.class()) else { return Err(HostError::Type(format!("this is not a {}", kind.class()))) };
    let [run, uid, slot] = handle.as_slice() else { return Err(HostError::Type(format!("this is not a {}", kind.class()))) };
    let doc = find_doc(h, *run, *uid).ok_or_else(|| kind.stale())?;
    let slot = whole(*slot).and_then(|s| usize::try_from(s).ok()).ok_or_else(|| kind.stale())?;
    let name = h.dom.resources.borrow().name(slot, doc.uid, kind).map(str::to_string).ok_or_else(|| kind.stale())?;
    if !with_doc(h, doc, |_, st| exists(&st.doc, kind, &name)) {
        return Err(kind.stale());
    }
    Ok(Res { doc, name })
}

fn this_res(h: &Host, a: &[Data], kind: Kind) -> std::result::Result<Res, HostError> {
    resource_of(h, arg(a, 0), kind)
}

/// Runs `id` with `params` on the document `doc`, its selection kept.
fn run_on(h: &Host, doc: Doc, id: &str, params: &Value) -> Answer {
    on_doc(h, doc.uid, true, |h| command(h, id, params))
}

/// The string `key` of a command's result.
fn result_name(r: &Data, key: &str) -> std::result::Result<String, HostError> {
    match r {
        Data::Object(o) => o.iter().find(|(k, _)| k == key).and_then(|(_, v)| if let Data::String(s) = v { Some(s.clone()) } else { None }),
        _ => None,
    }
    .ok_or_else(|| HostError::Error("the command gave no name".into()))
}

// ---- the collections ----

fn coll_doc(h: &Host, a: &[Data], kind: Kind) -> std::result::Result<Doc, HostError> {
    doc_of(h, arg(a, 0), kind.collection())
}

/// The names of the collection `this`, charged to the run.
fn coll_names(h: &Host, a: &[Data], kind: Kind) -> std::result::Result<(Doc, Vec<String>), HostError> {
    let doc = coll_doc(h, a, kind)?;
    let list = with_doc(h, doc, |_, st| names(&st.doc, kind));
    h.charge_call(list.len() as u64)?;
    Ok((doc, list))
}

fn coll_length(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    Ok(num(float(coll_names(h, a, kind)?.1.len() as u64)))
}

fn coll_parent(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    Ok(doc_ref(h, doc.uid))
}

fn coll_at(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let (doc, list) = coll_names(h, a, kind)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    match list.get(i) {
        Some(name) => resource_ref(h, doc.uid, kind, name),
        None => Ok(Data::Undefined),
    }
}

fn coll_by_name(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    let (doc, list) = coll_names(h, a, kind)?;
    if !list.iter().any(|n| n == name) {
        return Err(HostError::Error(NOT_FOUND.into()));
    }
    resource_ref(h, doc.uid, kind, name)
}

fn coll_index(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => coll_by_name(h, kind, a),
        Data::Number(_) => match coll_at(h, kind, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// The names `removeAll()` deletes from the collection of `kind`.
fn removable(d: &Document, kind: Kind) -> Vec<String> {
    match kind {
        Kind::Swatch | Kind::Spot => {
            let only_spots = kind == Kind::Spot;
            all_swatches(d).into_iter().filter(|(w, _)| !w.is_reserved() && (!only_spots || w.spot)).map(|(w, _)| w.name.clone()).collect()
        }
        Kind::Style => d.graphic_styles.iter().filter(|g| g.name != crate::doc::DEFAULT_GRAPHIC_STYLE).map(|g| g.name.clone()).collect(),
        _ => names(d, kind),
    }
}

/// `removeAll()`: deletes the collection's members in one command.
fn coll_remove_all(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    let (list, members) = with_doc(h, doc, |_, st| {
        let list = removable(&st.doc, kind);
        let members: Vec<String> = match kind {
            Kind::Group => st.doc.swatch_groups.iter().flat_map(|g| g.swatches.iter().map(|w| w.name.clone())).collect(),
            _ => Vec::new(),
        };
        (list, members)
    });
    h.charge_call(16 * list.len() as u64)?;
    if list.is_empty() {
        return Ok(Data::Undefined);
    }
    let (id, params) = match kind {
        Kind::Swatch | Kind::Spot | Kind::Group => ("swatch.delete", json!({"names": list, "unlink": true})),
        Kind::Symbol => ("symbol.delete", json!({"names": list})),
        Kind::Style => ("graphicStyle.delete", json!({"names": list})),
        Kind::Brush => return Err(HostError::Error("Brushes.removeAll is not available".into())),
    };
    run_on(h, doc, id, &params)?;
    let mut t = h.dom.resources.borrow_mut();
    for n in &list {
        t.remove(doc.uid, kind, n);
    }
    for n in &members {
        t.remove(doc.uid, Kind::Swatch, n);
    }
    Ok(Data::Undefined)
}

/// The current fill when it is a solid color.
fn current_solid(h: &Host) -> Option<Color> {
    let s = h.session.borrow();
    match s.new_art().fill_paint() {
        Paint::Solid { color, .. } => Some(color),
        _ => None,
    }
}

/// `swatches.add()`: the current fill (a color, gradient or pattern) as a new swatch, or black when
/// the fill is None.
fn swatches_add(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    let none = h.session.borrow().new_art().fill_paint().is_none();
    let params = if none { json!({"color": colors::param(&Color::BLACK)}) } else { json!({}) };
    let r = run_on(h, doc, "swatch.new", &params)?;
    let name = result_name(&r, "name")?;
    resource_ref(h, doc.uid, Kind::Swatch, &name)
}

/// Black in the color space of `doc`.
fn black_in(h: &Host, doc: Doc) -> Color {
    if is_cmyk(h, doc) { Color::cmyk(0.0, 0.0, 0.0, 1.0) } else { Color::BLACK }
}

/// Whether the document `doc` is in CMYK.
fn is_cmyk(h: &Host, doc: Doc) -> bool {
    with_doc(h, doc, |_, st| st.doc.color_mode == crate::doc::ColorMode::Cmyk)
}

/// Whether `color` may be a spot's color in a document that is in CMYK (`cmyk`) or RGB: a color of
/// that space, or Lab.
fn fits_spot(cmyk: bool, color: Color) -> bool {
    matches!(color, Color::Lab { .. }) || matches!((cmyk, color), (true, Color::Cmyk { .. }) | (false, Color::Rgb { .. }))
}

/// `spots.add()`: the current fill, or black, as a new spot color. A color that is neither Lab nor
/// in the document's color space is converted to that space with `swatch.new`'s `mode`.
fn spots_add(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    let color = current_solid(h).unwrap_or_else(|| black_in(h, doc));
    let cmyk = is_cmyk(h, doc);
    let mut params = json!({"color": colors::param(&color), "spot": true});
    if !fits_spot(cmyk, color) {
        params["mode"] = json!(if cmyk { "cmyk" } else { "rgb" });
    }
    let r = run_on(h, doc, "swatch.new", &params)?;
    let name = result_name(&r, "name")?;
    resource_ref(h, doc.uid, Kind::Spot, &name)
}

/// `swatchGroups.add()`: a new, empty color group.
fn groups_add(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    let r = run_on(h, doc, "swatch.newGroup", &json!({}))?;
    let name = result_name(&r, "name")?;
    resource_ref(h, doc.uid, Kind::Group, &name)
}

fn swatches_selected(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    coll_doc(h, a, kind)?;
    Ok(Data::Array(Vec::new()))
}

/// The registration point index of a `SymbolRegistrationPoint` value, 4 (the center) when left out.
fn registration(d: &Data) -> std::result::Result<u8, HostError> {
    let Some(d) = given(d) else { return Ok(4) };
    let Data::String(s) = d else { return Err(HostError::Type("registrationPoint must be a SymbolRegistrationPoint value".into())) };
    let at = REGISTRATION.iter().position(|r| s.strip_prefix("SymbolRegistrationPoint.") == Some(*r));
    at.and_then(|i| u8::try_from(i).ok()).ok_or_else(|| HostError::Type("registrationPoint must be a SymbolRegistrationPoint value".into()))
}

/// `SymbolRegistrationPoint`, in the order of the 9-point grid `symbol.new` takes.
const REGISTRATION: [&str; 9] = [
    "SYMBOLTOPLEFTPOINT",
    "SYMBOLTOPMIDDLEPOINT",
    "SYMBOLTOPRIGHTPOINT",
    "SYMBOLMIDDLELEFTPOINT",
    "SYMBOLCENTERPOINT",
    "SYMBOLMIDDLERIGHTPOINT",
    "SYMBOLBOTTOMLEFTPOINT",
    "SYMBOLBOTTOMMIDDLEPOINT",
    "SYMBOLBOTTOMRIGHTPOINT",
];

/// The page item `d` names, which must be in the document `doc`.
fn same_doc_item(h: &Host, doc: Doc, d: &Data) -> std::result::Result<Item, HostError> {
    let item = item_of(h, d)?;
    if item.doc.uid != doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    Ok(item)
}

/// `symbols.add(sourceArt, registrationPoint?)`: a symbol made from a copy of `sourceArt`, which
/// stays as it is.
fn symbols_add(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = coll_doc(h, a, kind)?;
    if matches!(arg(a, 1), Data::Undefined) {
        return Err(HostError::Type("symbols.add needs the source art".into()));
    }
    let item = same_doc_item(h, doc, arg(a, 1))?;
    let reg = registration(arg(a, 2))?;
    let r = run_on(h, doc, "symbol.new", &json!({"ids": [item.id.0], "keepArt": true, "registration": reg}))?;
    let name = result_name(&r, "name")?;
    resource_ref(h, doc.uid, Kind::Symbol, &name)
}

// ---- Swatch and Spot ----

fn res_name(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    Ok(text(this_res(h, a, kind)?.name))
}

fn res_typename(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    this_res(h, a, kind)?;
    Ok(text(kind.class()))
}

fn res_parent(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    Ok(doc_ref(h, r.doc.uid))
}

fn reserved(name: &str) -> HostError {
    HostError::Error(format!("{name} is built in and cannot change"))
}

/// The swatch `r` names, cloned.
fn swatch_now(h: &Host, r: &Res, kind: Kind) -> std::result::Result<Swatch, HostError> {
    with_doc(h, r.doc, |_, st| find_swatch(&st.doc, &r.name).map(|(w, _)| w.clone())).ok_or_else(|| kind.stale())
}

/// Renames the resource `this` names with the command of its kind.
fn res_set_name(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let new = name_arg(arg(a, 1), "name")?.trim().to_string();
    let r = this_res(h, a, kind)?;
    if new == r.name {
        return Ok(Data::Undefined);
    }
    let (id, params, key) = match kind {
        Kind::Swatch | Kind::Spot => {
            if swatch_now(h, &r, kind)?.is_reserved() {
                return Err(reserved(&r.name));
            }
            ("swatch.edit", json!({"name": r.name, "newName": new}), "name")
        }
        Kind::Group => ("swatch.renameGroup", json!({"group": r.name, "name": new}), "name"),
        Kind::Symbol => ("symbol.options", json!({"name": r.name, "newName": new}), "name"),
        Kind::Style if r.name == crate::doc::DEFAULT_GRAPHIC_STYLE => return Err(reserved(&r.name)),
        Kind::Style => ("graphicStyle.rename", json!({"name": r.name, "to": new}), "name"),
        Kind::Brush => return Err(HostError::Error("Brush.name is read-only".into())),
    };
    // Swatches, spots and color groups share one set of names.
    let (taken, what) = with_doc(h, r.doc, |_, st| match kind {
        Kind::Swatch | Kind::Spot | Kind::Group => {
            (exists(&st.doc, Kind::Swatch, &new) || exists(&st.doc, Kind::Group, &new), "a swatch or color group")
        }
        _ => (exists(&st.doc, kind, &new), if kind == Kind::Symbol { "a Symbol" } else { "a GraphicStyle" }),
    });
    if taken {
        return Err(HostError::Error(format!("{what} named {new} already exists")));
    }
    let out = run_on(h, r.doc, id, &params)?;
    let to = result_name(&out, key)?;
    h.dom.resources.borrow_mut().rename(r.doc.uid, kind, &r.name, &to);
    Ok(Data::Undefined)
}

/// Deletes the resource `this` names with the command of its kind.
fn res_remove(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let mut members = Vec::new();
    let (id, params) = match kind {
        Kind::Swatch | Kind::Spot => {
            if swatch_now(h, &r, kind)?.is_reserved() {
                return Err(reserved(&r.name));
            }
            ("swatch.delete", json!({"name": r.name, "unlink": true}))
        }
        Kind::Group => {
            members = group_members(h, &r);
            ("swatch.delete", json!({"name": r.name, "unlink": true}))
        }
        Kind::Symbol => ("symbol.delete", json!({"name": r.name})),
        Kind::Style if r.name == crate::doc::DEFAULT_GRAPHIC_STYLE => return Err(reserved(&r.name)),
        Kind::Style => ("graphicStyle.delete", json!({"name": r.name})),
        Kind::Brush => return Err(HostError::Error("Brush.remove is not available".into())),
    };
    run_on(h, r.doc, id, &params)?;
    let mut t = h.dom.resources.borrow_mut();
    t.remove(r.doc.uid, kind, &r.name);
    for m in &members {
        t.remove(r.doc.uid, Kind::Swatch, m);
    }
    Ok(Data::Undefined)
}

/// `Swatch.color`: a new color object. A spot swatch or Registration gives a `SpotColor` of that
/// Spot at 100%, and a tint of a spot a `SpotColor` of that spot at the tint.
fn swatch_color(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let w = swatch_now(h, &r, kind)?;
    if is_spot(&w) && w.tint_of().is_none() {
        let spot = resource_ref(h, r.doc.uid, Kind::Spot, &w.name)?;
        return Ok(Data::Object(vec![("typename".into(), text("SpotColor")), ("spot".into(), spot), ("tint".into(), num(100.0))]));
    }
    paint_color(h, r.doc, &w.paint)
}

/// What reading `paint` in the document `doc` gives: a new color object, whose `spot` names the
/// Spot when `paint` is linked to one.
pub(super) fn paint_color(h: &Host, doc: Doc, paint: &Paint) -> Answer {
    let color = colors::of_paint(paint);
    let base = match paint {
        Paint::Solid { swatch: Some(base), .. } => is_spot_of(h, doc, base).then(|| base.clone()),
        _ => None,
    };
    match (color, base) {
        (Data::Object(mut o), Some(base)) => {
            o.insert(1, ("spot".into(), resource_ref(h, doc.uid, Kind::Spot, &base)?));
            Ok(Data::Object(o))
        }
        (color, _) => Ok(color),
    }
}

/// Whether the document `doc` has a Spot named `name`.
pub(super) fn is_spot_of(h: &Host, doc: Doc, name: &str) -> bool {
    with_doc(h, doc, |_, st| exists(&st.doc, Kind::Spot, name))
}

/// The name of the Spot that the `spot` of a `SpotColor` assigned to a page item of the document
/// `doc` names.
pub(super) fn assigned_spot(h: &Host, spot: &Data, doc: Doc) -> std::result::Result<String, HostError> {
    if handle_of(spot, Kind::Spot.class()).is_none() {
        return Err(HostError::Type("SpotColor.spot must be a Spot".into()));
    }
    let r = resource_of(h, spot, Kind::Spot)?;
    if r.doc.uid != doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    Ok(r.name)
}

/// `Spot.color`: the spot's own color, as a new color object.
fn spot_color(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    Ok(colors::of_paint(&swatch_now(h, &r, kind)?.paint))
}

/// Gives the swatch `this` names the solid color `color`, with `swatch.edit`.
fn set_swatch_color(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let what = if kind == Kind::Spot { "Spot.color" } else { "Swatch.color" };
    let color = colors::assigned(arg(a, 1), what)?.ok_or_else(|| HostError::Error(format!("{what} cannot be NoColor")))?;
    let r = this_res(h, a, kind)?;
    let w = swatch_now(h, &r, kind)?;
    if w.is_reserved() {
        return Err(reserved(&r.name));
    }
    let now = match &w.paint {
        Paint::Solid { color, swatch: None, .. } => *color,
        Paint::Solid { .. } => return Err(HostError::Error(format!("{} is a tint of another swatch; change that swatch's color", r.name))),
        _ => return Err(HostError::Error(format!("{} is not a solid color", r.name))),
    };
    if w.spot && matches!(color, Color::Gray { .. }) {
        return Err(HostError::Error("a spot's color must be an RGBColor, a CMYKColor or a LabColor".into()));
    }
    // A spot keeps the color space it is given; a process swatch stores an RGB color in a CMYK
    // document as CMYK, as `swatch.edit` does without `keepModel`.
    let stored = if w.spot { color } else { stored_in(h, r.doc, color) };
    if now == stored {
        return Ok(Data::Undefined);
    }
    let mut params = json!({"name": r.name, "color": colors::param(&color)});
    if w.spot {
        params["keepModel"] = json!(true);
    }
    run_on(h, r.doc, "swatch.edit", &params)?;
    Ok(Data::Undefined)
}

/// `color` as `swatch.edit` stores it in the document `doc` without `keepModel`: an RGB color in a
/// CMYK document becomes CMYK through the color settings.
fn stored_in(h: &Host, doc: Doc, color: Color) -> Color {
    let mode = with_doc(h, doc, |_, st| st.doc.color_mode);
    match color {
        Color::Rgb { .. } if mode != crate::doc::ColorMode::Rgb => {
            let cms = vectorcraft_color::cms::active();
            cms.convert(&color, mode.model(), cms.settings().intent)
        }
        _ => color,
    }
}

/// The `ColorModel` of the Spot `name`: Registration's own, or `SPOT`.
fn spot_model(name: &str) -> &'static str {
    if name == swatch::REGISTRATION { "ColorModel.REGISTRATION" } else { "ColorModel.SPOT" }
}

fn spot_color_type(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    Ok(text(spot_model(&this_res(h, a, kind)?.name)))
}

fn spot_set_color_type(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let model = match arg(a, 1) {
        Data::String(s) if ["ColorModel.SPOT", "ColorModel.PROCESS", "ColorModel.REGISTRATION"].contains(&s.as_str()) => s.as_str(),
        _ => return Err(HostError::Type("colorType must be a ColorModel value".into())),
    };
    let r = this_res(h, a, kind)?;
    if model == spot_model(&r.name) {
        return Ok(Data::Undefined);
    }
    if r.name == swatch::REGISTRATION {
        return Err(reserved(&r.name));
    }
    Err(HostError::Error("a Spot stays a spot color; edit it through its Swatch".into()))
}

fn spot_color_of(h: &Host, kind: Kind, a: &[Data]) -> std::result::Result<Color, HostError> {
    let r = this_res(h, a, kind)?;
    match swatch_now(h, &r, kind)?.paint {
        Paint::Solid { color, .. } => Ok(color),
        _ => Err(HostError::Error(format!("{} is not a solid color", r.name))),
    }
}

fn spot_kind(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    Ok(text(match spot_color_of(h, kind, a)? {
        Color::Rgb { .. } => "SpotColorKind.SPOTRGB",
        Color::Lab { .. } => "SpotColorKind.SPOTLAB",
        Color::Cmyk { .. } | Color::Gray { .. } => "SpotColorKind.SPOTCMYK",
    }))
}

/// `getInternalColor()`: the components of the spot's color, as a new array.
fn spot_internal(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let o = match colors::of_paint(&Paint::solid(spot_color_of(h, kind, a)?)) {
        Data::Object(o) => o,
        _ => Vec::new(),
    };
    Ok(Data::Array(o.into_iter().filter(|(k, _)| k != "typename").map(|(_, v)| v).collect()))
}

// ---- SwatchGroup ----

fn group_members(h: &Host, r: &Res) -> Vec<String> {
    with_doc(h, r.doc, |_, st| {
        st.doc.swatch_groups.iter().find(|g| g.name == r.name).map(|g| g.swatches.iter().map(|w| w.name.clone()).collect()).unwrap_or_default()
    })
}

fn group_all(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let members = group_members(h, &r);
    h.charge_call(16 * members.len() as u64)?;
    members.iter().map(|n| resource_ref(h, r.doc.uid, Kind::Swatch, n)).collect::<std::result::Result<Vec<_>, _>>().map(Data::Array)
}

/// Moves the swatch or spot argument 1 into the group `this` names, with `swatch.move`.
fn group_add(h: &Host, kind: Kind, a: &[Data], member: Kind) -> Answer {
    let g = this_res(h, a, kind)?;
    let w = resource_of(h, arg(a, 1), member)?;
    if w.doc.uid != g.doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    let (sw, group) =
        with_doc(h, w.doc, |_, st| find_swatch(&st.doc, &w.name).map(|(s, g)| (s.clone(), g.map(str::to_string)))).ok_or_else(|| member.stale())?;
    if sw.is_reserved() {
        return Err(reserved(&w.name));
    }
    if !matches!(sw.paint, Paint::Solid { .. }) {
        return Err(HostError::Error(format!("a color group holds solid colors only, and {} is not one", w.name)));
    }
    if group.as_deref() == Some(g.name.as_str()) {
        return Ok(Data::Undefined);
    }
    run_on(h, g.doc, "swatch.move", &json!({"names": [w.name], "group": g.name}))?;
    Ok(Data::Undefined)
}

fn group_add_swatch(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    group_add(h, kind, a, Kind::Swatch)
}

fn group_add_spot(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    group_add(h, kind, a, Kind::Spot)
}

/// `SwatchGroup.removeAll()`: deletes the group's swatches and keeps the group.
fn group_remove_all(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let members = group_members(h, &r);
    h.charge_call(16 * members.len() as u64)?;
    if members.is_empty() {
        return Ok(Data::Undefined);
    }
    run_on(h, r.doc, "swatch.delete", &json!({"names": members, "unlink": true}))?;
    let mut t = h.dom.resources.borrow_mut();
    for m in &members {
        t.remove(r.doc.uid, Kind::Swatch, m);
    }
    Ok(Data::Undefined)
}

// ---- Symbol and SymbolItem ----

fn symbol_duplicate(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let out = run_on(h, r.doc, "symbol.duplicate", &json!({"name": r.name}))?;
    let name = result_name(&out, "name")?;
    resource_ref(h, r.doc.uid, Kind::Symbol, &name)
}

/// The ids of a document's symbol instances, in the order of its page items, and how many page
/// items it looked at.
fn instances(h: &Host, st: &DocState) -> (Vec<NodeId>, usize) {
    let items = page_items(h, st);
    let ids = items.iter().copied().filter(|i| st.doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::SymbolInstance { .. }))).collect();
    (ids, items.len())
}

/// The document of `SymbolItems` and its symbol instances; the page items scanned for them are
/// charged to the run.
fn instances_of(h: &Host, a: &[Data]) -> std::result::Result<(Doc, Vec<NodeId>), HostError> {
    let doc = doc_of(h, arg(a, 0), "SymbolItems")?;
    let (ids, scanned) = with_doc(h, doc, |_, st| instances(h, st));
    h.charge_call(scanned as u64)?;
    Ok((doc, ids))
}

fn sitems_length(h: &Host, _: Kind, a: &[Data]) -> Answer {
    Ok(num(float(instances_of(h, a)?.1.len() as u64)))
}

fn sitems_parent(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "SymbolItems")?;
    Ok(doc_ref(h, doc.uid))
}

fn sitems_at(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let (doc, ids) = instances_of(h, a)?;
    let Data::Number(n) = arg(a, 1) else { return Ok(Data::Undefined) };
    let Some(i) = whole(*n).and_then(|i| usize::try_from(i).ok()) else { return Ok(Data::Undefined) };
    Ok(with_doc(h, doc, |_, st| ids.get(i).map_or(Data::Undefined, |id| item_ref(h, st, *id))))
}

fn sitems_by_name(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let Data::String(name) = arg(a, 1) else { return Err(HostError::Type("give a name, as a string".into())) };
    let (doc, ids) = instances_of(h, a)?;
    with_doc(h, doc, |_, st| {
        let id = ids.iter().find(|i| st.doc.node(**i).and_then(|n| n.name.as_deref()) == Some(name.as_str()));
        id.map(|id| item_ref(h, st, *id)).ok_or_else(|| HostError::Error(NOT_FOUND.into()))
    })
}

fn sitems_index(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    match arg(a, 1) {
        Data::String(_) => sitems_by_name(h, kind, a),
        Data::Number(_) => match sitems_at(h, kind, a)? {
            Data::Undefined => Err(HostError::Error(NOT_FOUND.into())),
            d => Ok(d),
        },
        _ => Err(HostError::Type("give a name or an index".into())),
    }
}

/// `symbolItems.add(symbol)`: an instance of `symbol` at the center of the first artboard.
fn sitems_add(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let doc = doc_of(h, arg(a, 0), "SymbolItems")?;
    let sym = resource_of(h, arg(a, 1), Kind::Symbol)?;
    if sym.doc.uid != doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    let out = run_on(h, doc, "symbol.place", &json!({"name": sym.name}))?;
    let id = match &out {
        Data::Object(o) => o.iter().find(|(k, _)| k == "id").and_then(|(_, v)| if let Data::Number(n) = v { whole(*n) } else { None }),
        _ => None,
    }
    .ok_or_else(|| HostError::Error("the instance was not made".into()))?;
    Ok(with_doc(h, doc, |_, st| item_ref(h, st, NodeId(id))))
}

/// `symbolItems.removeAll()`: deletes every instance in one command; a hidden or locked one stops
/// it before anything is deleted. The symbols stay.
fn sitems_remove_all(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let (doc, ids) = instances_of(h, a)?;
    let refused: Vec<u64> = with_doc(h, doc, |_, st| ids.iter().filter(|i| !editable(&st.doc, **i)).map(|i| i.0).take(20).collect());
    if !refused.is_empty() {
        let list = refused.iter().map(u64::to_string).collect::<Vec<_>>().join(", ");
        return Err(HostError::Error(format!("cannot delete hidden or locked symbol instances: {list}")));
    }
    if !ids.is_empty() {
        run_on(h, doc, "edit.clear", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}))?;
    }
    Ok(Data::Undefined)
}

/// The symbol instance `this` names and its symbol's name.
fn this_instance(h: &Host, a: &[Data]) -> std::result::Result<(Item, String), HostError> {
    let item = item_of(h, arg(a, 0))?;
    let name = with_doc(h, item.doc, |_, st| match st.doc.node(item.id).map(|n| &n.kind) {
        Some(NodeKind::SymbolInstance { symbol, .. }) => Some(symbol.clone()),
        _ => None,
    });
    name.map(|n| (item, n)).ok_or_else(|| HostError::Type("this is not a SymbolItem".into()))
}

fn sitem_symbol(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let (item, name) = this_instance(h, a)?;
    resource_ref(h, item.doc.uid, Kind::Symbol, &name)
}

/// `SymbolItem.symbol = symbol`: `symbol.replace` for this instance.
fn sitem_set_symbol(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let sym = resource_of(h, arg(a, 1), Kind::Symbol)?;
    let (item, now) = this_instance(h, a)?;
    if sym.doc.uid != item.doc.uid {
        return Err(HostError::Error(CROSS_DOCUMENT.into()));
    }
    if sym.name == now {
        return Ok(Data::Undefined);
    }
    check_editable(h, item)?;
    run_on(h, item.doc, "symbol.replace", &json!({"name": sym.name, "ids": [item.id.0]}))?;
    Ok(Data::Undefined)
}

// ---- GraphicStyle and Brush ----

fn check_editable(h: &Host, item: Item) -> std::result::Result<(), HostError> {
    if with_doc(h, item.doc, |_, st| editable(&st.doc, item.id)) {
        Ok(())
    } else {
        Err(HostError::Error(format!("Cannot edit page item {}: it, or a layer or group it is in, is hidden or locked", item.id.0)))
    }
}

/// How many objects the object `id` is, with those inside it.
fn object_count(d: &Document, id: NodeId) -> u64 {
    fn count(n: &crate::doc::Node) -> u64 {
        1 + n.children().map_or(0, |c| c.iter().map(|c| count(c)).sum())
    }
    d.node(id).map_or(0, count)
}

/// The page item argument 1 of `applyTo` or `mergeTo`: one object of the resource's document that
/// can be edited, charged to the run by the objects it holds.
fn target(h: &Host, r: &Res, a: &[Data]) -> std::result::Result<Item, HostError> {
    if matches!(arg(a, 1), Data::Array(_)) {
        return Err(HostError::Type("give one page item".into()));
    }
    let item = same_doc_item(h, r.doc, arg(a, 1))?;
    check_editable(h, item)?;
    let n = with_doc(h, item.doc, |_, st| object_count(&st.doc, item.id));
    h.charge_call(16 * n)?;
    Ok(item)
}

fn style_apply(h: &Host, kind: Kind, a: &[Data], add: bool) -> Answer {
    let r = this_res(h, a, kind)?;
    let item = target(h, &r, a)?;
    run_on(h, r.doc, "graphicStyle.apply", &json!({"name": r.name, "ids": [item.id.0], "add": add}))?;
    Ok(Data::Undefined)
}

fn style_apply_to(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    style_apply(h, kind, a, false)
}

fn style_merge_to(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    style_apply(h, kind, a, true)
}

/// `Brush.applyTo(path)`: `brush.apply` for one path, unless its stroke already draws this brush
/// and the brush is the current one. A guide, a clipping path and a path in a compound path take no
/// brush from `brush.apply`, so they throw.
fn brush_apply_to(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let r = this_res(h, a, kind)?;
    let item = target(h, &r, a)?;
    let (path, brushable, same) = with_doc(h, item.doc, |_, st| {
        let n = st.doc.node(item.id);
        let path = n.is_some_and(|n| matches!(n.kind, NodeKind::Path { .. }));
        let in_compound = st.doc.parent_of(item.id).and_then(|p| st.doc.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
        let brushable = n.is_some_and(|n| matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. })) && !in_compound;
        let stroke = n.and_then(|n| n.appearance.stroke());
        let drawn =
            stroke.is_some_and(|s| s.brush.as_deref() == Some(r.name.as_str()) && s.brush_options.is_empty() && s.width > 0.0 && !s.paint.is_none());
        (path, brushable, drawn && vectorcraft_brush::current(&st.doc).as_deref() == Some(r.name.as_str()))
    });
    if !path {
        return Err(HostError::Type("a brush applies to a PathItem only".into()));
    }
    if !brushable {
        return Err(HostError::Error("a brush does not apply to a guide, a clipping path or a path in a compound path".into()));
    }
    if !same {
        run_on(h, r.doc, "brush.apply", &json!({"name": r.name, "ids": [item.id.0]}))?;
    }
    Ok(Data::Undefined)
}

// ---- registration ----

/// The enumerations of resources, as frozen objects whose members are `"Name.MEMBER"` strings.
const ENUMS: &[(&str, &[&str])] = &[
    ("SymbolRegistrationPoint", &REGISTRATION),
    ("ColorModel", &["PROCESS", "REGISTRATION", "SPOT"]),
    ("SpotColorKind", &["SPOTCMYK", "SPOTLAB", "SPOTRGB"]),
];

pub(super) fn install_enums(realm: &mut Realm) -> std::result::Result<(), RunError> {
    for (name, members) in ENUMS {
        let items = members.iter().map(|m| (m.to_string(), HostItem::Value(Data::String(format!("{name}.{m}"))))).collect();
        realm.define_host_object(name, items)?;
    }
    Ok(())
}

fn doc_collection(h: &Host, kind: Kind, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref(kind.collection(), &[float(h.run), float(doc.uid)]))
}

fn doc_symbol_items(h: &Host, _: Kind, a: &[Data]) -> Answer {
    let doc = this_doc(h, a)?;
    Ok(host_ref("SymbolItems", &[float(h.run), float(doc.uid)]))
}

/// The members `Document` gets: one collection for each kind, `symbolItems`, and `gradients` and
/// `patterns`, which are not available yet.
pub(super) fn document_members(l: &Link) -> Vec<(String, HostMember)> {
    let mut m = named(vec![
        ("swatches", kget(l, Kind::Swatch, doc_collection)),
        ("spots", kget(l, Kind::Spot, doc_collection)),
        ("swatchGroups", kget(l, Kind::Group, doc_collection)),
        ("symbols", kget(l, Kind::Symbol, doc_collection)),
        ("graphicStyles", kget(l, Kind::Style, doc_collection)),
        ("brushes", kget(l, Kind::Brush, doc_collection)),
        ("symbolItems", kget(l, Kind::Symbol, doc_symbol_items)),
    ]);
    m.extend(["gradients", "patterns"].into_iter().map(|n| later(l, "Document", n)));
    m
}

/// The members `SymbolItem` adds to `PageItem`'s.
pub(super) fn symbol_item_members(l: &Link) -> Vec<(String, HostMember)> {
    named(vec![("symbol", kproperty(l, Kind::Symbol, sitem_symbol, sitem_set_symbol))])
}

/// The collection classes, and one class for each kind of resource.
pub(super) fn classes(l: &Link) -> Vec<HostClass> {
    let mut out = Vec::new();
    for kind in KINDS {
        let mut m = named(vec![
            ("typename", typename_of(kind.collection())),
            ("length", kget(l, kind, coll_length)),
            ("parent", kget(l, kind, coll_parent)),
            ("index", kmethod(l, kind, coll_index)),
            ("getByName", kmethod(l, kind, coll_by_name)),
        ]);
        if kind != Kind::Brush {
            m.push(("removeAll".into(), kmethod(l, kind, coll_remove_all)));
        }
        match kind {
            Kind::Swatch => {
                m.push(("add".into(), kmethod(l, kind, swatches_add)));
                m.push(("getSelected".into(), kmethod(l, kind, swatches_selected)));
            }
            Kind::Spot => m.push(("add".into(), kmethod(l, kind, spots_add))),
            Kind::Group => m.push(("add".into(), kmethod(l, kind, groups_add))),
            Kind::Symbol => m.push(("add".into(), kmethod(l, kind, symbols_add))),
            Kind::Style | Kind::Brush => {}
        }
        out.push(class(kind.collection(), None, m, Some(kfunction(l, kind, coll_at))));

        let mut m = named(vec![("typename", kget(l, kind, res_typename)), ("parent", kget(l, kind, res_parent))]);
        if kind == Kind::Brush {
            m.push(("name".into(), kget(l, kind, res_name)));
        } else {
            m.push(("name".into(), kproperty(l, kind, res_name, res_set_name)));
            m.push(("remove".into(), kmethod(l, kind, res_remove)));
        }
        let extra = match kind {
            Kind::Swatch => vec![("color", kproperty(l, kind, swatch_color, set_swatch_color))],
            Kind::Spot => vec![
                ("color", kproperty(l, kind, spot_color, set_swatch_color)),
                ("colorType", kproperty(l, kind, spot_color_type, spot_set_color_type)),
                ("spotKind", kget(l, kind, spot_kind)),
                ("getInternalColor", kmethod(l, kind, spot_internal)),
            ],
            Kind::Group => vec![
                ("addSwatch", kmethod(l, kind, group_add_swatch)),
                ("addSpot", kmethod(l, kind, group_add_spot)),
                ("getAllSwatches", kmethod(l, kind, group_all)),
                ("removeAll", kmethod(l, kind, group_remove_all)),
            ],
            Kind::Symbol => vec![("duplicate", kmethod(l, kind, symbol_duplicate))],
            Kind::Style => vec![("applyTo", kmethod(l, kind, style_apply_to)), ("mergeTo", kmethod(l, kind, style_merge_to))],
            Kind::Brush => vec![("applyTo", kmethod(l, kind, brush_apply_to))],
        };
        m.extend(named(extra));
        out.push(class(kind.class(), None, m, None));
    }
    let k = Kind::Symbol;
    out.push(class(
        "SymbolItems",
        None,
        named(vec![
            ("typename", typename_of("SymbolItems")),
            ("length", kget(l, k, sitems_length)),
            ("parent", kget(l, k, sitems_parent)),
            ("index", kmethod(l, k, sitems_index)),
            ("getByName", kmethod(l, k, sitems_by_name)),
            ("add", kmethod(l, k, sitems_add)),
            ("removeAll", kmethod(l, k, sitems_remove_all)),
        ]),
        Some(kfunction(l, k, sitems_at)),
    ));
    out
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::{Kind, MAX_SLOTS, Table, names};
    use crate::{DocState, Document};

    #[test]
    fn a_check_visits_only_the_documents_that_changed() {
        let mut docs = vec![DocState::new(Document::new(612.0, 792.0), None), DocState::new(Document::new(612.0, 792.0), None)];
        let (one, two) = (docs[0].uid, docs[1].uid);
        let held = names(&docs[0].doc, Kind::Swatch);
        assert!(held.len() > 1, "a new document has swatches");
        let mut t = Table::default();
        let slots: Vec<usize> = held.iter().map(|n| t.slot(one, Kind::Swatch, n).unwrap_or(usize::MAX)).collect();
        let other = t.slot(two, Kind::Swatch, &held[0]).unwrap_or(usize::MAX);
        assert!(t.forget(&docs) > 0, "the first check compares every held slot");
        assert_eq!(t.forget(&docs), 0, "documents unchanged since the last check are not visited");
        docs[1].doc = Arc::new((*docs[1].doc).clone());
        let names_of_two = names(&docs[1].doc, Kind::Swatch).len() as u64;
        assert_eq!(t.forget(&docs), 1 + names_of_two, "only the changed document's slot and names are compared");
        assert_eq!(t.name(other, two, Kind::Swatch), Some(held[0].as_str()));
        assert!(slots.iter().zip(&held).all(|(s, n)| t.name(*s, one, Kind::Swatch) == Some(n.as_str())));
        docs.remove(0);
        assert_eq!(t.forget(&docs), held.len() as u64, "a closed document's slots are compared once");
        assert!(slots.iter().all(|s| t.name(*s, one, Kind::Swatch).is_none()), "a closed document's objects go stale");
        assert_eq!(t.forget(&docs), 0);
    }

    #[test]
    fn a_run_holds_a_bounded_number_of_resource_objects() {
        let mut t = Table::default();
        for i in 0..MAX_SLOTS {
            assert!(t.slot(1, Kind::Swatch, &format!("s{i}")).is_ok());
        }
        assert_eq!(t.slot(1, Kind::Swatch, "s0").ok(), Some(0), "a resource already held takes no new slot");
        assert!(t.slot(1, Kind::Swatch, "one more").is_err());
    }

    #[test]
    fn a_rename_onto_a_held_name_leaves_the_old_object_stale() {
        let mut t = Table::default();
        let (a, b) = (t.slot(1, Kind::Swatch, "A").unwrap_or(9), t.slot(1, Kind::Swatch, "B").unwrap_or(9));
        let spot = t.slot(1, Kind::Spot, "A").unwrap_or(9);
        t.rename(1, Kind::Swatch, "A", "B");
        assert_eq!(t.name(a, 1, Kind::Swatch), Some("B"));
        assert_eq!(t.name(spot, 1, Kind::Spot), Some("B"), "the spot object follows the swatch's rename");
        assert_eq!(t.name(b, 1, Kind::Swatch), None);
        assert_eq!(t.name(a, 2, Kind::Swatch), None, "a slot answers for its own document only");
    }
}
