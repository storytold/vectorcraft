//! What the interpreter doesn't tell the import, read from the file itself: the optional content
//! groups (layers) with their states and nesting, which group each marked-content sequence belongs
//! to, the isolate and knockout flags of transparency groups, the overprint parameters each path
//! and each stretch of text is painted with, the Separation and DeviceN inks they are painted in
//! (by their colorant names, which the interpreter drops), and the names of the fonts.
//!
//! hayro-interpret reports a marked-content sequence by its tag alone and a transparency group
//! without its flags, and skips the content of groups that are off. So each page's content is
//! walked here in the interpreter's order (page content, then form XObjects as they are drawn,
//! skipping what it skips): the k-th `BDC`/`BMC` it reports is the k-th one found here (its tag
//! checks that), the k-th form transparency group is the k-th group found here, and the k-th
//! path it fills or strokes is the k-th one painted here (the page's count checks that). To import
//! content that is off as hidden layers, [`all_on`] gives the file an update that turns every
//! group on.

use std::collections::{HashMap, HashSet};

use crate::import_color::items;
use craft_pdf::interpret::CacheKey;
use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::content::TypedIter;
use craft_pdf::syntax::content::ops::TypedInstruction;
use craft_pdf::syntax::object::{Array, Dict, FromBytes, MaybeRef, Name, Object, ObjectIdentifier, dict_or_stream};
use craft_pdf::syntax::page::{Page, Resources};

use crate::PdfError;
use crate::import_color::ink_names;

/// The interpreter's limit on nested form XObjects.
const MAX_DEPTH: u32 = 50;
/// The most content operators walked per page (a guard against forms drawn within each other
/// many times over): the art past it can't be told apart by group. A drawing of a million paths
/// takes a few million; the walk reads about 15 million a second, twice as fast as the
/// interpreter draws them.
const MAX_OPS: usize = 64_000_000;
/// The most marked-content sequences noted per page (24 bytes each): past it the rest of the
/// page isn't told apart either.
const MAX_TAGS: usize = 4_000_000;
/// The most groups read.
const MAX_GROUPS: usize = 10_000;
/// Match the interpreter's ordinary content window and maximum incomplete instruction.
const CONTENT_WINDOW: usize = 64 << 10;
const MAX_WINDOW: usize = 256 << 20;
/// The most Separation and DeviceN inks noted per page.
const MAX_INKS: usize = 4096;
/// The most changes of ink noted per page in the paths, and in the text (24 bytes each).
const MAX_INK_CHANGES: usize = 2_000_000;
/// The deepest layers nest in the configuration's `/Order`.
pub(crate) const MAX_NESTING: usize = 32;

/// An optional content group: a layer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Ocg {
    pub name: String,
    /// On in the file's default configuration (shown by viewers).
    pub on: bool,
    /// Printed (its print usage isn't off).
    pub print: bool,
    pub locked: bool,
    /// The group it is listed under in the layer order (a sublayer of it).
    pub parent: Option<usize>,
}

/// The file's optional content groups.
#[derive(Default)]
pub(crate) struct Ocgs {
    pub list: Vec<Ocg>,
    index: HashMap<ObjectIdentifier, usize>,
    /// The groups that are off (as the interpreter decides it).
    off: HashSet<ObjectIdentifier>,
    /// The groups whose view state applies when the file opens (an `/AS` entry for viewing).
    view_auto: HashSet<ObjectIdentifier>,
    /// The groups placed in the layer order.
    placed: HashSet<usize>,
}

impl Ocgs {
    /// The groups listed in the catalog's `/OCProperties`, with their default states.
    pub fn read(pdf: &Pdf) -> Self {
        let mut o = Self::default();
        let xref = pdf.xref();
        let Some(props) = xref.get::<Dict<'_>>(xref.root_id()).and_then(|c| c.get::<Dict<'_>>(b"OCProperties")) else {
            return o;
        };
        let config = props.get::<Dict<'_>>(b"D").unwrap_or_default();
        let refs = |d: &Dict<'_>, key: &[u8]| -> Vec<ObjectIdentifier> {
            d.get::<Array<'_>>(key)
                .map(|a| items(&a).filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).take(MAX_GROUPS).collect())
                .unwrap_or_default()
        };
        let all = refs(&props, b"OCGs");
        if config.get::<Name<'_>>(b"BaseState").is_some_and(|n| n.as_ref() == b"OFF") {
            o.off.extend(all.iter().copied());
        }
        for id in refs(&config, b"ON") {
            o.off.remove(&id);
        }
        o.off.extend(refs(&config, b"OFF"));
        let auto = config.get::<Array<'_>>(b"AS").map(|a| a.iter::<Dict<'_>>().take(MAX_GROUPS).collect::<Vec<_>>()).unwrap_or_default();
        for d in auto.iter().filter(|d| d.get::<Name<'_>>(b"Event").is_some_and(|e| e.as_ref() == b"View")) {
            o.view_auto.extend(refs(d, b"OCGs"));
        }
        let locked: HashSet<_> = refs(&config, b"Locked").into_iter().collect();
        for id in all {
            if let Some(d) = xref.get::<Dict<'_>>(id) {
                o.add(id, &d, locked.contains(&id));
            }
        }
        if let Some(order) = config.get::<Array<'_>>(b"Order") {
            o.nest(&order, None, 0, xref, &mut 0);
        }
        o
    }

    /// Read the layer order `order` (listed under group `parent`): a group followed by an array
    /// lists that array's groups as its sublayers; an array starting with a label lists its
    /// groups under the label, which isn't a group (they go under `parent`). A group listed twice
    /// stays where it is first listed. `seen`: the entries read so far.
    fn nest(&mut self, order: &Array<'_>, parent: Option<usize>, depth: usize, xref: &craft_pdf::syntax::xref::XRef, seen: &mut usize) {
        let mut prev = None;
        for item in items(order) {
            *seen += 1;
            if *seen > 4 * MAX_GROUPS {
                return;
            }
            let sub = match item {
                MaybeRef::Ref(r) => {
                    let id = ObjectIdentifier::from(r);
                    let sub = xref.get::<Array<'_>>(id);
                    if sub.is_none() {
                        prev = self.place(id, parent);
                    }
                    sub
                }
                MaybeRef::NotRef(o) => o.into_array(),
            };
            if let Some(sub) = sub
                && depth < MAX_NESTING
            {
                let label = matches!(items(&sub).next(), Some(MaybeRef::NotRef(Object::String(_))));
                let under = prev.take().filter(|_| !label).or(parent);
                self.nest(&sub, under, depth + 1, xref, seen);
            }
        }
    }

    /// Put group `id` under `parent` in the layer order → its index, unless it isn't a group or
    /// is already placed. Each group is placed after its parent, so the nesting has no cycles.
    fn place(&mut self, id: ObjectIdentifier, parent: Option<usize>) -> Option<usize> {
        let i = *self.index.get(&id)?;
        let placed = self.placed.insert(i);
        let g = self.list.get_mut(i).filter(|_| placed)?;
        g.parent = parent;
        Some(i)
    }

    /// Note group `id` (its dictionary `d`) → its index.
    fn add(&mut self, id: ObjectIdentifier, d: &Dict<'_>, locked: bool) -> usize {
        if let Some(i) = self.index.get(&id) {
            return *i;
        }
        let name = d.get::<craft_pdf::syntax::object::String<'_>>(b"Name").map(|s| text_string(s.as_bytes())).unwrap_or_default();
        let print = usage(d, b"Print", b"PrintState") != Some(false);
        // A view state applies when the configuration says so on opening; an off one hides the
        // group all the same (it says how the group is meant to be seen).
        let view = usage(d, b"View", b"ViewState");
        let on = match view {
            Some(v) if self.view_auto.contains(&id) => v,
            Some(false) => false,
            _ => !self.off.contains(&id),
        };
        let name = if name.is_empty() { format!("Layer {}", self.list.len() + 1) } else { name };
        self.list.push(Ocg { name, on, print, locked, parent: None });
        self.index.insert(id, self.list.len() - 1);
        self.list.len() - 1
    }

    /// Does the interpreter leave out the art of some group (off in the default configuration)?
    pub fn skips_any(&self) -> bool {
        self.index.keys().any(|id| self.off.contains(id))
    }

    /// The group content marked with `/OC` (`d`: the group or membership dictionary, `id` its
    /// object) belongs to: a membership dictionary's first group.
    fn group_of(&mut self, d: &Dict<'_>, id: ObjectIdentifier, xref: &craft_pdf::syntax::xref::XRef) -> Option<usize> {
        if d.get::<Name<'_>>(b"Type").is_some_and(|t| t.as_ref() == b"OCMD") {
            let first = d.get::<Array<'_>>(b"OCGs").and_then(|a| items(&a).find_map(|i| i.as_obj_ref())).or_else(|| d.get_ref(b"OCGs"))?;
            let id = ObjectIdentifier::from(first);
            return self.note(id, &xref.get::<Dict<'_>>(id)?);
        }
        self.note(id, d)
    }

    /// The index of group `id` (dictionary `d`), noted if new (up to [`MAX_GROUPS`]).
    fn note(&mut self, id: ObjectIdentifier, d: &Dict<'_>) -> Option<usize> {
        match self.index.get(&id) {
            Some(i) => Some(*i),
            None => (self.list.len() < MAX_GROUPS).then(|| self.add(id, d, false)),
        }
    }
}

/// The state (`ON` → true, `OFF` → false) group `d`'s usage `category` gives in `key`.
fn usage(d: &Dict<'_>, category: &[u8], key: &[u8]) -> Option<bool> {
    let state = d.get::<Dict<'_>>(b"Usage")?.get::<Dict<'_>>(category)?.get::<Name<'_>>(key)?;
    match state.as_ref() {
        b"ON" => Some(true),
        b"OFF" => Some(false),
        _ => None,
    }
}

/// A PDF text string: UTF-16 (with its byte order mark), UTF-8 (with one) or PDFDocEncoding
/// (read as Latin-1, which it matches for letters).
pub(crate) fn text_string(b: &[u8]) -> String {
    if let Some(rest) = b.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c)).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = b.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    b.iter().map(|&c| c as char).collect()
}

/// One page as the interpreter walks it.
#[derive(Default)]
pub(crate) struct Scan {
    /// Each marked-content sequence: its tag ([`tag_key`]) and the group (index into
    /// [`Ocgs::list`]) it marks.
    pub tags: Vec<(u64, Option<usize>)>,
    /// Each form transparency group: isolated, knockout.
    pub groups: Vec<(bool, bool)>,
    /// Content was incomplete or a scan limit was reached. No document may be returned from
    /// this scan, since its routing no longer describes every object the interpreter draws.
    pub cut: bool,
    /// Each fill and each stroke of a path the interpreter draws (a fill-and-stroke operator is
    /// a fill, then a stroke; `sh` is a fill): whether it overprints.
    pub paints: Vec<bool>,
    /// How the text shown before each of [`Self::paints`] (since the one before it) overprints,
    /// and last the text after them all: one more entry than `paints`.
    pub texts: Vec<TextOverprint>,
    /// Something on the page overprints (when nothing does, the overprint flags in `paints` and
    /// `texts` aren't read).
    pub overprints: bool,
    /// Something overprints with overprint mode 0, where a `DeviceCMYK` colour's zero components
    /// knock out; the document's overprint is the nonzero one (mode 1).
    pub zero_mode: bool,
    /// The colorant names of the Separation and DeviceN spaces the page sets (`cs`, `CS`), and of
    /// those of the shading patterns it sets (`scn`, `SCN`) and of the shadings it draws (`sh`),
    /// each once (none when the page uses more than [`MAX_INKS`] inks or changes ink more than
    /// [`MAX_INK_CHANGES`] times).
    pub inks: Vec<Vec<String>>,
    /// The inks each of [`Self::paints`] is painted in.
    paint_inks: Changes<Inks>,
    /// The inks of the text before each of [`Self::paints`], counted as [`Self::texts`] is: of its
    /// fills and of its strokes (indices into [`Self::inks`]), where all of its text shown in an
    /// ink is shown in the same one.
    text_inks: Changes<[Option<u32>; 2]>,
}

impl Scan {
    /// The colorant names of the ink the walk found `at` painted in: a path's, or the fills' (the
    /// `stroke`s') of the text before a path. `pattern`: the interpreter paints with a pattern,
    /// whose shading's ink it is (text painted with one isn't followed); else the color space's.
    pub fn ink(&self, at: Walked, stroke: bool, pattern: bool) -> Option<&[String]> {
        let i = match at {
            Walked::Path(k) => {
                let inks = self.paint_inks.at(k);
                if pattern { inks.pattern } else { inks.space }
            }
            Walked::Text(_) if pattern => None,
            Walked::Text(k) => self.text_inks.at(k)[usize::from(stroke)],
        }?;
        self.inks.get(usize::try_from(i).ok()?).map(Vec::as_slice)
    }
}

/// The inks a fill or a stroke paints in (indices into [`Scan::inks`]): that of the color space
/// set last (`cs`, `CS`), and that of the shading of the pattern named last (`scn`, `SCN`). The
/// interpreter's paint, a color or a pattern, says which of the two it is.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Inks {
    space: Option<u32>,
    pattern: Option<u32>,
}

/// What the interpreter draws, counted as the walk counts it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Walked {
    /// The `k`-th path ([`Scan::paints`]).
    Path(usize),
    /// The text shown before the `k`-th path ([`Scan::texts`]).
    Text(usize),
}

/// A value for each index from 0, kept as the indices where it changes (the default before the
/// first).
#[derive(Default)]
struct Changes<T>(Vec<(usize, T)>);

impl<T: Copy + Default + PartialEq> Changes<T> {
    /// The value is `v` from index `at` on (`at` past the indices set before). `false`: the value
    /// changes there and [`MAX_INK_CHANGES`] changes are kept already, so this one isn't kept.
    fn set(&mut self, at: usize, v: T) -> bool {
        if self.0.last().map_or(T::default(), |(_, last)| *last) == v {
            return true;
        }
        if self.0.len() >= MAX_INK_CHANGES {
            return false;
        }
        self.0.push((at, v));
        true
    }

    /// The value at index `k`.
    fn at(&self, k: usize) -> T {
        let n = self.0.partition_point(|(at, _)| *at <= k);
        n.checked_sub(1).and_then(|i| self.0.get(i)).map_or(T::default(), |(_, v)| *v)
    }
}

/// The ink of the text shown between two painted paths: of its fills, or of its strokes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum RunInk {
    /// No text is shown in an ink.
    #[default]
    None,
    /// All of the text shown in an ink is shown in this one.
    One(u32),
    /// Text is shown in several inks, which can't be told apart by glyph.
    Several,
}

impl RunInk {
    /// With more text shown in `ink` (`None`: in no ink).
    fn and(self, ink: Option<u32>) -> Self {
        match (self, ink) {
            (_, None) => self,
            (Self::None, Some(i)) => Self::One(i),
            (Self::One(j), Some(i)) if i == j => self,
            _ => Self::Several,
        }
    }

    fn ink(self) -> Option<u32> {
        match self {
            Self::One(i) => Some(i),
            _ => None,
        }
    }
}

/// How the text shown between two painted paths overprints.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) enum TextOverprint {
    /// No text is shown there.
    #[default]
    None,
    /// All of it is shown with these parameters.
    All { fill: bool, stroke: bool },
    /// Text is shown with different parameters, which can't be told apart by glyph.
    Mixed,
}

/// What the walk follows of the graphics state: the overprint parameters (ISO 32000-1 §8.6.7;
/// `OP`, `op` and `OPM` in Table 58) and the inks fills and strokes paint in.
#[derive(Clone, Copy, Default)]
struct State {
    stroke: bool,
    fill: bool,
    /// Overprint mode 1 (nonzero overprint).
    nonzero: bool,
    /// The inks of fills and of strokes.
    inks: [Inks; 2],
}

/// A marked-content tag as [`Scan::tags`] keeps it: its FNV-1a hash (the tags are only compared,
/// and a page can have millions).
pub(crate) fn tag_key(tag: &[u8]) -> u64 {
    tag.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
}

/// [`MAX_OPS`], lowered by tests that need a page past it.
#[cfg(not(test))]
fn max_ops() -> usize {
    MAX_OPS
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_MAX_OPS: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_OPS) };
}

#[cfg(test)]
fn max_ops() -> usize {
    TEST_MAX_OPS.with(std::cell::Cell::get)
}

#[cfg(not(test))]
fn max_window() -> usize {
    MAX_WINDOW
}

#[cfg(test)]
thread_local! {
    pub(crate) static TEST_MAX_WINDOW: std::cell::Cell<usize> = const { std::cell::Cell::new(MAX_WINDOW) };
}

#[cfg(test)]
fn max_window() -> usize {
    TEST_MAX_WINDOW.with(std::cell::Cell::get)
}

pub(crate) fn incomplete_content() -> PdfError {
    PdfError::Parse("PDF content is incomplete or exceeds the reader's limits; no document was imported".into())
}

/// Whitespace and comments after the final instruction do not make an incomplete tail.
fn blank_tail(mut bytes: &[u8]) -> bool {
    loop {
        let first = bytes.iter().position(|b| !matches!(b, 0 | 9 | 10 | 12 | 13 | 32)).unwrap_or(bytes.len());
        bytes = bytes.get(first..).unwrap_or_default();
        if bytes.first() == Some(&b'%') {
            let end = bytes.iter().position(|b| matches!(b, b'\r' | b'\n')).unwrap_or(bytes.len());
            bytes = bytes.get(end..).unwrap_or_default();
        } else {
            return bytes.is_empty();
        }
    }
}

struct Walker<'w, 'p> {
    ocgs: &'w mut Ocgs,
    /// The groups that are off for the interpreter (none once [`all_on`] turned them on).
    off: HashSet<ObjectIdentifier>,
    xref: &'p craft_pdf::syntax::xref::XRef,
    fonts: &'w mut FontNames,
    visible: Vec<bool>,
    ops: usize,
    max_ops: usize,
    max_form_bytes: usize,
    out: Scan,
    /// A form XObject hidden by its own optional content was met.
    hidden_form: bool,
    /// The saved graphics states' overprint parameters and inks, the current one last, pushed and
    /// popped as the interpreter does its states.
    states: Vec<State>,
    /// The text shown since the last painted path.
    text: TextOverprint,
    /// The inks of the text shown since the last painted path: of its fills, of its strokes.
    text_inks: [RunInk; 2],
    /// The index in [`Scan::inks`] of each list of colorant names.
    ink_index: HashMap<Vec<String>, u32>,
    /// More inks or changes of ink than are kept: the page's inks are left out.
    inks_cut: bool,
}

impl Walker<'_, '_> {
    fn is_visible(&self) -> bool {
        self.visible.last().copied().unwrap_or(true)
    }

    fn begin(&mut self, on: bool) {
        let v = self.is_visible() && on;
        self.visible.push(v);
    }

    /// Enter optional content `d` (object `id`): a group, or a membership dictionary whose policy
    /// decides from its groups.
    fn begin_oc(&mut self, d: &Dict<'_>, id: Option<ObjectIdentifier>) {
        let membership = id.is_none() || d.get::<Name<'_>>(b"Type").is_some_and(|t| t.as_ref() == b"OCMD");
        let on = match id {
            Some(id) if !membership => !self.off.contains(&id),
            _ => {
                let ids: Vec<ObjectIdentifier> = match d.get::<Array<'_>>(b"OCGs") {
                    Some(a) => items(&a).filter_map(|i| i.as_obj_ref()).map(ObjectIdentifier::from).collect(),
                    None => d.get_ref(b"OCGs").map(ObjectIdentifier::from).into_iter().collect(),
                };
                let on = |i: &ObjectIdentifier| !self.off.contains(i);
                ids.is_empty()
                    || match d.get::<Name<'_>>(b"P").as_deref() {
                        Some(b"AllOn") => ids.iter().all(on),
                        Some(b"AnyOff") => !ids.iter().all(on),
                        Some(b"AllOff") => !ids.iter().any(on),
                        _ => ids.iter().any(on),
                    }
            }
        };
        self.begin(on);
    }

    fn state(&self) -> State {
        self.states.last().copied().unwrap_or_default()
    }

    fn save(&mut self) {
        self.states.push(self.state());
    }

    /// The interpreter never pops its first state.
    fn restore(&mut self) {
        if self.states.len() > 1 {
            self.states.pop();
        }
    }

    fn overprinting(&mut self, on: bool) {
        self.out.overprints |= on;
        self.out.zero_mode |= on && !self.state().nonzero;
    }

    /// The interpreter draws a path: a fill (or a `stroke`) of the current one.
    fn painted(&mut self, stroke: bool) {
        if !self.is_visible() {
            return;
        }
        let s = self.state();
        let on = if stroke { s.stroke } else { s.fill };
        self.overprinting(on);
        let k = self.out.paints.len();
        self.out.paints.push(on);
        self.out.texts.push(std::mem::take(&mut self.text));
        self.inks_cut |= !self.out.paint_inks.set(k, s.inks[usize::from(stroke)]);
        self.inks_cut |= !self.out.text_inks.set(k, std::mem::take(&mut self.text_inks).map(RunInk::ink));
    }

    /// The interpreter shows text.
    fn shown(&mut self) {
        if !self.is_visible() {
            return;
        }
        let s = self.state();
        self.overprinting(s.fill || s.stroke);
        let now = TextOverprint::All { fill: s.fill, stroke: s.stroke };
        self.text = if self.text == TextOverprint::None || self.text == now { now } else { TextOverprint::Mixed };
        // Text painted with a pattern isn't followed, so only the color spaces' inks count.
        for (run, inks) in self.text_inks.iter_mut().zip(s.inks) {
            *run = run.and(inks.space);
        }
    }

    /// The color space of fills (or of `stroke`s) is in ink `ink` from now on.
    fn set_space(&mut self, stroke: bool, ink: Option<u32>) {
        if let Some(s) = self.states.last_mut() {
            s.inks[usize::from(stroke)].space = ink;
        }
    }

    /// The pattern of fills (or of `stroke`s) is in ink `ink` from now on.
    fn set_pattern(&mut self, stroke: bool, ink: Option<u32>) {
        if let Some(s) = self.states.last_mut() {
            s.inks[usize::from(stroke)].pattern = ink;
        }
    }

    /// The ink of color space `name` (`cs`, `CS`): a Separation or DeviceN space of the resources.
    /// The names the interpreter reads as device and pattern spaces come first, as it reads them.
    fn space_ink(&mut self, name: &Name<'_>, res: &Resources<'_>) -> Option<u32> {
        if matches!(name.as_ref(), b"DeviceRGB" | b"RGB" | b"DeviceGray" | b"G" | b"DeviceCMYK" | b"CMYK" | b"CalCMYK" | b"Pattern") {
            return None;
        }
        self.ink_of(&res.get_color_space(name)?)
    }

    /// The ink pattern `name` paints in (`scn`, `SCN`): a shading pattern's (a dictionary, where a
    /// tiling pattern is a stream) is its shading's.
    fn pattern_ink(&mut self, name: &Name<'_>, res: &Resources<'_>) -> Option<u32> {
        let Object::Dict(p) = res.get_pattern(name)? else { return None };
        self.shading_ink(&p.get::<Object<'_>>(b"Shading")?)
    }

    /// The ink of a shading (a dictionary or a stream): its color space's.
    fn shading_ink(&mut self, shading: &Object<'_>) -> Option<u32> {
        let (d, _) = dict_or_stream(shading)?;
        self.ink_of(&d.get::<Object<'_>>(b"ColorSpace")?)
    }

    /// The index in [`Scan::inks`] of color space `o` when it is a Separation or DeviceN array, its
    /// colorant names noted when new.
    fn ink_of(&mut self, o: &Object<'_>) -> Option<u32> {
        let Object::Array(a) = o else { return None };
        let (_, names) = ink_names(&mut a.flex_iter())?;
        if let Some(i) = self.ink_index.get(&names) {
            return Some(*i);
        }
        if self.out.inks.len() >= MAX_INKS {
            self.inks_cut = true;
            return None;
        }
        let i = u32::try_from(self.out.inks.len()).ok()?;
        self.ink_index.insert(names.clone(), i);
        self.out.inks.push(names);
        Some(i)
    }

    fn note_fonts(&mut self, res: &Resources<'_>) {
        for key in res.fonts.keys() {
            if let Some(f) = res.fonts.get::<Dict<'_>>(key.as_ref())
                && let Some(name) = f.get::<Name<'_>>(b"BaseFont")
            {
                self.fonts.entry(f.cache_key()).or_insert_with(|| (String::from_utf8_lossy(name.as_ref()).into_owned(), embeds_glyphs(&f)));
            }
        }
    }

    fn walk(&mut self, data: &[u8], res: &Resources<'_>, depth: u32) {
        let mut iter = TypedIter::new(data);
        self.note_fonts(res);
        // The interpreter saves the state for each content stream, and restores what the stream
        // left saved when it ends.
        let saved = self.states.len();
        self.save();
        self.walk_ops(&mut iter, res, depth);
        self.out.cut |= !blank_tail(data.get(iter.committed()..).unwrap_or_default());
        self.states.truncate(saved);
    }

    /// Scan the same stream windows as the interpreter without materializing the page.
    /// Graphics and optional-content state live across windows; page state is saved once.
    fn page(&mut self, page: &Page<'_>) {
        let resources = page.resources();
        self.note_fonts(resources);
        let saved = self.states.len();
        self.save();
        let mut content = page.content_reader();
        let mut window = Vec::new();
        let limit = max_window();
        let mut exhausted = false;
        loop {
            let (committed, needs_more) = {
                let mut ops = TypedIter::new_window(&window, exhausted);
                self.walk_ops(&mut ops, resources, 0);
                (ops.committed(), ops.needs_more())
            };
            if committed > window.len() {
                self.out.cut = true;
                break;
            }
            window.drain(..committed);
            if self.out.cut {
                break;
            }
            if !needs_more || exhausted {
                // No success on an undecidable tail or on a parser stop before the source ends.
                // The typed iterator does not expose every malformed-operator failure.
                self.out.cut |= !exhausted || !blank_tail(&window);
                break;
            }
            if window.len() >= limit {
                self.out.cut = true;
                break;
            }
            let want = window.len().max(CONTENT_WINDOW).min(limit.saturating_sub(window.len()));
            if window.try_reserve(want).is_err() {
                self.out.cut = true;
                break;
            }
            if content.read_into(&mut window, want) == 0 {
                exhausted = true;
            }
        }
        self.out.cut |= content.truncated();
        self.states.truncate(saved);
    }

    fn walk_ops(&mut self, iter: &mut TypedIter<'_>, res: &Resources<'_>, depth: u32) {
        while let Some(op) = iter.next() {
            self.ops += 1;
            if self.ops > self.max_ops || self.out.tags.len() >= MAX_TAGS {
                self.out.cut = true;
                return;
            }
            match op {
                TypedInstruction::BeginMarkedContentWithProperties(bdc) => {
                    let oc = match bdc.1 {
                        Object::Name(n) => {
                            res.properties.get_ref(n.as_ref()).map(|r| (res.properties.get::<Dict<'_>>(n.as_ref()).unwrap_or_default(), r))
                        }
                        o => dict_or_stream(o)
                            .and_then(|(props, _)| props.get_ref(b"OC").map(|r| (props.get::<Dict<'_>>(b"OC").unwrap_or_default(), r))),
                    };
                    let group = match oc {
                        Some((d, r)) => {
                            let id = ObjectIdentifier::from(r);
                            self.begin_oc(&d, Some(id));
                            self.ocgs.group_of(&d, id, self.xref)
                        }
                        None => {
                            self.begin(true);
                            None
                        }
                    };
                    self.out.tags.push((tag_key(bdc.0.as_ref()), group));
                }
                TypedInstruction::BeginMarkedContent(bmc) => {
                    self.begin(true);
                    self.out.tags.push((tag_key(bmc.0.as_ref()), None));
                }
                TypedInstruction::EndMarkedContent(_) => {
                    self.visible.pop();
                }
                TypedInstruction::SaveState(_) => self.save(),
                TypedInstruction::RestoreState(_) => self.restore(),
                TypedInstruction::SetGraphicsState(gs) => {
                    if let Some(d) = res.get_ext_g_state(gs.0)
                        && let Some(s) = self.states.last_mut()
                    {
                        let stroke = d.get::<bool>(b"OP");
                        s.stroke = stroke.unwrap_or(s.stroke);
                        // Without `/op`, `/OP` sets both.
                        s.fill = d.get::<bool>(b"op").or(stroke).unwrap_or(s.fill);
                        s.nonzero = d.get::<u8>(b"OPM").map_or(s.nonzero, |m| m != 0);
                    }
                }
                TypedInstruction::FillPathEvenOdd(_) | TypedInstruction::FillPathNonZero(_) | TypedInstruction::FillPathNonZeroCompatibility(_) => {
                    self.painted(false);
                }
                TypedInstruction::StrokePath(_) | TypedInstruction::CloseAndStrokePath(_) => self.painted(true),
                TypedInstruction::FillAndStrokeEvenOdd(_)
                | TypedInstruction::FillAndStrokeNonZero(_)
                | TypedInstruction::CloseFillAndStrokeEvenOdd(_)
                | TypedInstruction::CloseFillAndStrokeNonZero(_) => {
                    self.painted(false);
                    self.painted(true);
                }
                // Drawn when the interpreter can read the shading (the page's count tells), as a
                // fill in a state of its own, in the shading's ink.
                TypedInstruction::Shading(s) => {
                    if let Some(sh) = res.get_shading(s.0) {
                        let ink = self.shading_ink(&sh);
                        self.save();
                        self.set_pattern(false, ink);
                        self.painted(false);
                        self.restore();
                    }
                }
                TypedInstruction::ColorSpaceNonStroke(cs) => {
                    let ink = self.space_ink(cs.0, res);
                    self.set_space(false, ink);
                }
                TypedInstruction::ColorSpaceStroke(cs) => {
                    let ink = self.space_ink(cs.0, res);
                    self.set_space(true, ink);
                }
                // A pattern by name; without one, `scn` sets a color in the space set before.
                TypedInstruction::NonStrokeColorNamed(c) => {
                    if let Some(name) = c.1 {
                        let ink = self.pattern_ink(name, res);
                        self.set_pattern(false, ink);
                    }
                }
                TypedInstruction::StrokeColorNamed(c) => {
                    if let Some(name) = c.1 {
                        let ink = self.pattern_ink(name, res);
                        self.set_pattern(true, ink);
                    }
                }
                TypedInstruction::NonStrokeColorDeviceGray(_)
                | TypedInstruction::NonStrokeColorDeviceRgb(_)
                | TypedInstruction::NonStrokeColorCmyk(_) => {
                    self.set_space(false, None);
                }
                TypedInstruction::StrokeColorDeviceGray(_) | TypedInstruction::StrokeColorDeviceRgb(_) | TypedInstruction::StrokeColorCmyk(_) => {
                    self.set_space(true, None);
                }
                TypedInstruction::ShowText(_)
                | TypedInstruction::ShowTexts(_)
                | TypedInstruction::NextLineAndShowText(_)
                | TypedInstruction::ShowTextWithParameters(_) => self.shown(),
                TypedInstruction::XObject(x) => {
                    if !self.is_visible() {
                        continue;
                    }
                    let Some(s) = res.get_x_object(x.0) else { continue };
                    let d = s.dict();
                    if d.get::<Name<'_>>(b"Subtype").as_deref() != Some(b"Form") || d.get::<[f32; 4]>(b"BBox").is_none() {
                        continue;
                    }
                    if depth >= MAX_DEPTH {
                        self.out.cut = true;
                        return;
                    }
                    let Ok(data) = s.decoded() else {
                        self.out.cut = true;
                        return;
                    };
                    // Ordinary filtered form decoders may return a prefix at their limit.
                    // Reaching that bound cannot certify a complete form, even at an operator boundary.
                    if !s.filters().is_empty() && data.len() >= self.max_form_bytes {
                        self.out.cut = true;
                        return;
                    }
                    let oc = d.get::<Dict<'_>>(b"OC");
                    if let Some(oc) = &oc {
                        self.begin_oc(oc, d.get_ref(b"OC").map(ObjectIdentifier::from));
                        self.hidden_form |= !self.is_visible();
                    }
                    if self.is_visible() {
                        if let Some(g) = d.get::<Dict<'_>>(b"Group") {
                            if self.out.groups.len() >= MAX_GROUPS {
                                self.out.cut = true;
                                return;
                            }
                            self.out.groups.push((g.get::<bool>(b"I").unwrap_or(false), g.get::<bool>(b"K").unwrap_or(false)));
                        }
                        // (Its own resources, or without any those of what draws it: as the interpreter reads them.)
                        let inner = Resources::from_scope(d.get::<Dict<'_>>(b"Resources"), res.clone());
                        self.save();
                        self.walk(data.as_ref(), &inner, depth + 1);
                        self.restore();
                        if self.out.cut {
                            return;
                        }
                    }
                    if oc.is_some() {
                        self.visible.pop();
                    }
                }
                _ => {}
            }
        }
    }
}

/// The fonts of a document by cache key: base font name, and whether the file carries the glyphs
/// ([`embeds_glyphs`]).
pub(crate) type FontNames = HashMap<u128, (String, bool)>;

/// Whether font `f` carries its own glyphs: a font program in its descriptor (a composite font's in
/// its descendant's), or a Type 3 font's glyph procedures. A font that doesn't is drawn with a
/// stand-in, whose glyphs say nothing about which installed face the document means.
fn embeds_glyphs(f: &Dict<'_>) -> bool {
    if f.get::<Name<'_>>(b"Subtype").is_some_and(|s| s.as_ref() == b"Type3") {
        return true;
    }
    // A font program is a stream, so always an indirect object.
    let program = |d: &Dict<'_>| {
        d.get::<Dict<'_>>(b"FontDescriptor").is_some_and(|fd| [&b"FontFile"[..], b"FontFile2", b"FontFile3"].iter().any(|k| fd.get_ref(*k).is_some()))
    };
    program(f) || f.get::<Array<'_>>(b"DescendantFonts").and_then(|a| a.iter::<Dict<'_>>().next()).is_some_and(|d| program(&d))
}

/// Walk `page` as the interpreter will: `all_on` when [`all_on`] turned every group on. Fonts
/// found are noted in `fonts` (their cache key → base font name, and whether the file carries its
/// glyphs).
pub(crate) fn scan_page(page: &Page<'_>, ocgs: &mut Ocgs, all_on: bool, fonts: &mut FontNames, max_form_bytes: usize) -> Result<Scan, PdfError> {
    let scan = walk_page(page, ocgs, all_on, fonts, max_form_bytes).out;
    if scan.cut { Err(incomplete_content()) } else { Ok(scan) }
}

fn walk_page<'w, 'p>(page: &Page<'p>, ocgs: &'w mut Ocgs, all_on: bool, fonts: &'w mut FontNames, max_form_bytes: usize) -> Walker<'w, 'p> {
    let off = if all_on { HashSet::new() } else { ocgs.off.clone() };
    let mut w = Walker {
        ocgs,
        off,
        xref: page.xref(),
        fonts,
        visible: vec![],
        ops: 0,
        max_ops: max_ops(),
        max_form_bytes,
        out: Scan::default(),
        hidden_form: false,
        states: vec![State::default()],
        text: TextOverprint::None,
        text_inks: [RunInk::None; 2],
        ink_index: HashMap::new(),
        inks_cut: false,
    };
    w.page(page);
    w.inks_cut |= !w.out.text_inks.set(w.out.paints.len(), w.text_inks.map(RunInk::ink));
    if w.inks_cut {
        w.out.inks.clear();
    }
    w.out.texts.push(w.text);
    w
}

/// Does `page` draw a form XObject that is off by its own optional content (not a marked-content
/// sequence)? Its art couldn't be told apart by group once every group is on.
pub(crate) fn hides_forms(page: &Page<'_>, ocgs: &mut Ocgs, max_form_bytes: usize) -> Result<bool, PdfError> {
    let mut fonts = HashMap::new();
    let walk = walk_page(page, ocgs, false, &mut fonts, max_form_bytes);
    if walk.out.cut { Err(incomplete_content()) } else { Ok(walk.hidden_form) }
}

/// `bytes` with an update appended that drops the catalog's `/OCProperties`, so every group
/// draws; `None` when the file's structure can't take one.
pub(crate) fn all_on(bytes: &[u8], pdf: &Pdf) -> Option<Vec<u8>> {
    let root = pdf.xref().root_id();
    let catalog = pdf.xref().get::<Dict<'_>>(root)?;
    let body = catalog.data().strip_suffix(b">>")?;
    let prev = last_xref(bytes)?;
    let trailer = trailer_at(bytes, prev)?;
    let inner = trailer.strip_prefix(b"<<")?.strip_suffix(b">>")?;
    let mut out = bytes.to_vec();
    if !out.ends_with(b"\n") {
        out.push(b'\n');
    }
    let at = out.len();
    out.extend(format!("{} {} obj\n", root.obj_number, root.gen_number).bytes());
    out.extend_from_slice(body);
    // A later key wins: the groups' configuration is gone, so the interpreter draws them all.
    out.extend_from_slice(b" /OCProperties null >>\nendobj\n");
    let xref = out.len();
    out.extend(format!("xref\n{} 1\n{at:010} {:05} n \ntrailer\n<<", root.obj_number, root.gen_number).bytes());
    out.extend_from_slice(inner);
    out.extend(format!(" /Prev {prev} >>\nstartxref\n{xref}\n%%EOF\n").bytes());
    Some(out)
}

/// The offset the file's last `startxref` points to.
fn last_xref(bytes: &[u8]) -> Option<usize> {
    let at = bytes.windows(9).rposition(|w| w == b"startxref")?;
    let rest = bytes.get(at + 9..)?;
    let digits: String = rest.iter().skip_while(|c| c.is_ascii_whitespace()).take_while(|c| c.is_ascii_digit()).map(|&c| c as char).collect();
    digits.parse().ok().filter(|&p: &usize| p < bytes.len())
}

/// The trailer dictionary of the cross-reference section at `at` (a table's `trailer`, or a
/// cross-reference stream's dictionary), as written.
fn trailer_at(bytes: &[u8], at: usize) -> Option<&[u8]> {
    let rest = bytes.get(at..)?;
    let from = if rest.trim_ascii_start().starts_with(b"xref") { rest.windows(7).position(|w| w == b"trailer")? } else { 0 };
    let open = from + rest.get(from..)?.windows(2).position(|w| w == b"<<")?;
    let d = Dict::from_bytes(rest.get(open..)?)?;
    Some(d.data())
}
