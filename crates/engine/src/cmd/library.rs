//! Libraries (Window › Libraries): local libraries of graphics, colours and text styles that every
//! document can use. A library is one `.vclibrary` file (JSON) in the library folder next to the
//! preferences, which the desktop app sets; without a folder (the web, headless sessions) libraries
//! live for the session only. A graphic is kept as a native document with a PNG thumbnail: adding
//! one copies the selection as Copy does, and placing it pastes a copy as Paste does (its images,
//! symbols, patterns and swatches come along; a swatch name the document gives another colour is
//! merged into the document's). Items can be put in user-named groups, and a library can be
//! exported to a `.vclibrary` file and imported from one (#926). Items and groups are kept in the
//! order the Libraries panel shows them, which dragging them there changes.
//!
//! A gradient is kept without the placement it had on its art, and fits each object it paints. A
//! pattern is kept as a native document holding one swatch's worth of the pattern (its
//! definition and the resources its art uses) and a thumbnail; using it adds the pattern to the
//! document, as its swatch, and paints the selection with it.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vectorcraft_color::{Color, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId, TextStyleDef};

use super::clipboard::SwatchChoices;
use super::{CommandSpec, EngineError, Result, Session, always, bad, cmd, color_value, has_doc, ids_param, index, ok, point_param, str_param};
use crate::Clipboard;

/// The extension of a library file.
pub const LIBRARY_EXT: &str = "vclibrary";
/// The extensions of library files (open dialogs).
pub const LIBRARY_EXTS: &[&str] = &[LIBRARY_EXT];
/// Bytes a library file may have when read (graphics with large images included).
const MAX_FILE: u64 = 512 << 20;
/// Items of one kind a library may hold.
const MAX_ITEMS: usize = 10_000;
/// Groups a library may hold.
const MAX_GROUPS: usize = 1_000;
/// Characters of a library, group or item name kept from a file.
const MAX_NAME: usize = 256;
/// Base64 bytes a graphic's thumbnail may have (a 256-pixel PNG is far smaller).
const MAX_THUMBNAIL: usize = 4 << 20;
/// Pixels a side a thumbnail may claim (they are made at most 256).
const MAX_THUMBNAIL_SIDE: u32 = 1024;
/// Stops a gradient read from a file may have (fewer than 2 is no gradient).
const MAX_STOPS: usize = 1024;

/// A library: its name and its items, newest last.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub colors: Vec<LibraryColor>,
    #[serde(default)]
    pub char_styles: Vec<TextStyleDef>,
    #[serde(default)]
    pub para_styles: Vec<TextStyleDef>,
    #[serde(default)]
    pub graphics: Vec<LibraryGraphic>,
    /// Files written before gradients and patterns have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gradients: Vec<LibraryGradient>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<LibraryPattern>,
    /// User-named groups of its items, in the order they were made (#926); an item is in one
    /// group at most, the others are ungrouped. Files written before groups have none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<LibraryGroup>,
}

/// A user-named group of a library's items.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LibraryGroup {
    pub name: String,
    #[serde(default)]
    pub items: Vec<ItemRef>,
}

/// A library item: its kind and its key (its name; a graphic's id).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ItemRef {
    pub kind: ItemKind,
    pub item: String,
}

/// What a library item is, as groups name it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    Color,
    CharStyle,
    ParaStyle,
    Graphic,
    Gradient,
    Pattern,
    /// A kind this version doesn't know (a newer one wrote it): dropped when the file is read.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryColor {
    pub name: String,
    pub color: Color,
}

/// A graphic: the art as a native document (its own resources included) and its thumbnail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryGraphic {
    pub id: String,
    pub name: String,
    /// Size of the art (points).
    pub width: f64,
    pub height: f64,
    /// The native document, base64.
    pub data: String,
    /// A PNG of the art, at most 256 pixels a side, base64 (empty when it draws nothing).
    #[serde(default)]
    pub thumbnail: String,
}

/// A gradient: its stops, kind and angle, without a placement or swatch links.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryGradient {
    pub name: String,
    pub gradient: GradientPaint,
}

/// A pattern: a native document whose objects are painted with pattern `pattern` (its definition
/// and the resources its art uses), and its thumbnail.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LibraryPattern {
    pub name: String,
    /// The pattern's name in `data`.
    pub pattern: String,
    /// The native document, base64.
    pub data: String,
    /// A PNG of one swatch's worth of the pattern, at most 256 pixels a side, base64.
    #[serde(default)]
    pub thumbnail: String,
}

/// The libraries of a session ([`Session::libraries`]).
#[derive(Clone, Debug, Default)]
pub struct Libraries {
    /// The library folder: none in the web app and headless sessions.
    dir: Option<String>,
    /// (id: the file's stem, library), in name order.
    libs: Vec<(String, Library)>,
    /// The library the panel shows, which commands without a `library` act on.
    current: Option<String>,
}

impl Libraries {
    pub fn dir(&self) -> Option<&str> {
        self.dir.as_deref()
    }

    /// Set the library folder and read its libraries (unreadable files are skipped). The folder
    /// is the app's own: automation roots don't apply to it ([`crate::file_access`]).
    pub fn set_dir(&mut self, dir: Option<String>) {
        self.dir = dir;
        self.libs.clear();
        if let Some(dir) = self.dir.clone() {
            for path in library_paths(&dir) {
                let id = stem_of(&path);
                let lib = crate::file_access::unconfined(|| {
                    super::fileio::file_stamp(&path).filter(|(len, _)| *len <= MAX_FILE).and_then(|_| super::fileio::read_file(&path).ok())
                })
                .and_then(|b| serde_json::from_slice::<Library>(&b).ok());
                if let Some(mut lib) = lib {
                    lib.sanitize();
                    self.libs.push((id, lib));
                }
            }
        }
        self.sort();
    }

    fn sort(&mut self) {
        self.libs.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()).then(a.0.cmp(&b.0)));
    }

    /// The libraries, as (id, library), in name order.
    pub fn all(&self) -> &[(String, Library)] {
        &self.libs
    }

    /// The library the panel shows: the one chosen last, else the first.
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref().filter(|c| self.get(c).is_some()).or_else(|| self.libs.first().map(|(i, _)| i.as_str()))
    }

    pub fn get(&self, id: &str) -> Option<&Library> {
        self.libs.iter().find(|(i, _)| i == id).map(|(_, l)| l)
    }

    /// Library `key` by id, else by name (ignoring case).
    fn find(&self, key: &str) -> Option<String> {
        self.libs.iter().find(|(i, _)| i == key).or_else(|| self.libs.iter().find(|(_, l)| l.name.eq_ignore_ascii_case(key))).map(|(i, _)| i.clone())
    }

    /// A new empty library called `name` → its id.
    fn create(&mut self, name: &str) -> Result<String> {
        self.insert(Library { name: name.to_string(), ..Default::default() })
    }

    /// `lib` as a new library, its file named after it, made the current one → its id.
    fn insert(&mut self, lib: Library) -> Result<String> {
        let base = file_stem_for(&lib.name);
        let taken = |id: &str| self.libs.iter().any(|(i, _)| i.eq_ignore_ascii_case(id)) || self.file_exists(id);
        let id = std::iter::once(base.clone()).chain((2..).map(|n| format!("{base} {n}"))).find(|id| !taken(id)).unwrap_or_default();
        self.libs.push((id.clone(), lib));
        if let Err(e) = self.save(&id) {
            self.libs.retain(|(i, _)| i != &id);
            return Err(e);
        }
        self.sort();
        self.current = Some(id.clone());
        Ok(id)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn file_exists(&self, id: &str) -> bool {
        self.path(id).is_some_and(|p| std::path::Path::new(&p).exists())
    }

    #[cfg(target_arch = "wasm32")]
    fn file_exists(&self, _: &str) -> bool {
        false
    }

    fn path(&self, id: &str) -> Option<String> {
        let dir = self.dir.as_deref()?;
        Some(std::path::Path::new(dir).join(format!("{id}.{LIBRARY_EXT}")).to_string_lossy().into_owned())
    }

    /// Write library `id` to its file (nothing to do without a folder). The folder is the app's
    /// own and ids are safe file names: automation roots don't apply ([`crate::file_access`]).
    fn save(&self, id: &str) -> Result<()> {
        let (Some(path), Some(lib)) = (self.path(id), self.get(id)) else { return Ok(()) };
        let bytes = serde_json::to_vec(lib).map_err(|e| EngineError::Other(e.to_string()))?;
        crate::file_access::unconfined(|| {
            if let Some(dir) = self.dir.as_deref() {
                super::fileio::create_dir(dir)?;
            }
            super::fileio::write_file(&path, &bytes)
        })
    }

    /// Change library `id` with `f` and save it; when `f` fails, or the save does, the change is
    /// undone.
    fn change<T>(&mut self, id: &str, f: impl FnOnce(&mut Library) -> Result<T>) -> Result<T> {
        let i = self.libs.iter().position(|(i, _)| i == id).ok_or_else(|| EngineError::Other(format!("no library `{id}`")))?;
        let before = self.libs.get(i).map(|(_, l)| l.clone()).unwrap_or_default();
        let r = match self.libs.get_mut(i) {
            Some((_, lib)) => f(lib),
            None => return Err(EngineError::Other(format!("no library `{id}`"))),
        };
        let r = r.and_then(|r| self.save(id).map(|()| r));
        if let Err(e) = r {
            if let Some((_, lib)) = self.libs.get_mut(i) {
                *lib = before;
            }
            return Err(e);
        }
        self.sort();
        r
    }

    fn delete(&mut self, id: &str) -> Result<()> {
        if let Some(path) = self.path(id) {
            remove_file(&path)?;
        }
        self.libs.retain(|(i, _)| i != id);
        if self.current.as_deref() == Some(id) {
            self.current = None;
        }
        Ok(())
    }
}

impl Library {
    /// Every item as (kind, key: its name, a graphic's id, name shown): colors, gradients and
    /// patterns, then character and paragraph styles, then graphics.
    pub fn items(&self) -> impl Iterator<Item = (ItemKind, &str, &str)> {
        fn styles(kind: ItemKind, list: &[TextStyleDef]) -> impl Iterator<Item = (ItemKind, &str, &str)> {
            list.iter().map(move |t| (kind, t.name.as_str(), t.name.as_str()))
        }
        self.colors
            .iter()
            .map(|c| (ItemKind::Color, c.name.as_str(), c.name.as_str()))
            .chain(self.gradients.iter().map(|g| (ItemKind::Gradient, g.name.as_str(), g.name.as_str())))
            .chain(self.patterns.iter().map(|p| (ItemKind::Pattern, p.name.as_str(), p.name.as_str())))
            .chain(styles(ItemKind::CharStyle, &self.char_styles))
            .chain(styles(ItemKind::ParaStyle, &self.para_styles))
            .chain(self.graphics.iter().map(|g| (ItemKind::Graphic, g.id.as_str(), g.name.as_str())))
    }

    /// The group of each grouped item: (kind, key) → its index in [`Library::groups`].
    pub fn group_index(&self) -> HashMap<(ItemKind, &str), usize> {
        self.groups.iter().enumerate().flat_map(|(i, g)| g.items.iter().map(move |r| ((r.kind, r.item.as_str()), i))).collect()
    }

    /// Group `name` (else the one of that name ignoring case): its index.
    fn group_position(&self, name: &str) -> Option<usize> {
        let lower = name.to_lowercase();
        self.groups.iter().position(|g| g.name == name).or_else(|| self.groups.iter().position(|g| g.name.to_lowercase() == lower))
    }

    /// Item `item` of `kind` (a graphic by id, else by name) as a group names it.
    fn item_ref(&self, kind: ItemKind, item: &str) -> Option<ItemRef> {
        let found = self.items().find(|(k, key, _)| *k == kind && *key == item);
        let found = found.or_else(|| self.items().find(|(k, _, name)| *k == kind && kind == ItemKind::Graphic && *name == item));
        found.map(|(kind, key, _)| ItemRef { kind, item: key.to_string() })
    }

    /// Take `r` out of its group.
    fn ungroup(&mut self, r: &ItemRef) {
        for g in &mut self.groups {
            g.items.retain(|x| x != r);
        }
    }

    /// Move item `r` next to `to`, an item of its kind: just before it, or just after it when
    /// `after`, in their kind's list (the order the panel shows them in).
    fn place_next_to(&mut self, r: &ItemRef, to: &ItemRef, after: bool) {
        fn shift<T>(list: &mut Vec<T>, key: impl Fn(&T) -> &str, from: &str, to: &str, after: bool) {
            let Some(i) = list.iter().position(|x| key(x) == from) else { return };
            let item = list.remove(i);
            let j = list.iter().position(|x| key(x) == to).map_or(list.len(), |j| j + usize::from(after));
            list.insert(j.min(list.len()), item);
        }
        let (from, to_key) = (r.item.as_str(), to.item.as_str());
        match r.kind {
            ItemKind::Color => shift(&mut self.colors, |c| c.name.as_str(), from, to_key, after),
            ItemKind::Gradient => shift(&mut self.gradients, |g| g.name.as_str(), from, to_key, after),
            ItemKind::Pattern => shift(&mut self.patterns, |p| p.name.as_str(), from, to_key, after),
            ItemKind::CharStyle => shift(&mut self.char_styles, |c| c.name.as_str(), from, to_key, after),
            ItemKind::ParaStyle => shift(&mut self.para_styles, |c| c.name.as_str(), from, to_key, after),
            ItemKind::Graphic => shift(&mut self.graphics, |g| g.id.as_str(), from, to_key, after),
            ItemKind::Unknown => {}
        }
    }

    /// Put item `r` in group `at` (an index; None: in none), out of the one it was in, and with
    /// `next_to` just before or after that item of its kind in its kind's list: in the group too,
    /// next to that item when it is there, else last. → the group's name.
    fn put(&mut self, r: ItemRef, at: Option<usize>, next_to: Option<&(ItemRef, bool)>) -> Option<String> {
        if let Some((other, after)) = next_to
            && *other != r
        {
            self.place_next_to(&r, other, *after);
        }
        self.ungroup(&r);
        at.and_then(|i| self.groups.get_mut(i)).map(|g| {
            let near = next_to.and_then(|(other, after)| g.items.iter().position(|x| x == other).map(|j| j + usize::from(*after)));
            g.items.insert(near.unwrap_or(g.items.len()).min(g.items.len()), r);
            g.name.clone()
        })
    }

    /// Make a library read from a file (hand-edited, imported) safe and consistent: at most
    /// [`MAX_ITEMS`] items of each kind and [`MAX_GROUPS`] groups, names clipped to [`MAX_NAME`]
    /// characters and unique within their kind (a repeated one gets a number), sizes finite, a
    /// thumbnail only when it is a small PNG, gradients of 2 to [`MAX_STOPS`] stops in range
    /// ([`clean_gradient`]; others are dropped), and groups holding only items there are, each in
    /// one group at most.
    fn sanitize(&mut self) {
        clip(&mut self.name);
        self.colors.truncate(MAX_ITEMS);
        self.gradients.retain_mut(|g| clean_gradient(&mut g.gradient));
        self.gradients.truncate(MAX_ITEMS);
        self.patterns.truncate(MAX_ITEMS);
        self.char_styles.truncate(MAX_ITEMS);
        self.para_styles.truncate(MAX_ITEMS);
        self.graphics.truncate(MAX_ITEMS);
        self.groups.truncate(MAX_GROUPS);
        uniquify(self.colors.iter_mut().map(|c| &mut c.name), "Color", " ", false);
        uniquify(self.gradients.iter_mut().map(|g| &mut g.name), "Gradient", " ", false);
        uniquify(self.patterns.iter_mut().map(|p| &mut p.name), "Pattern", " ", false);
        for p in &mut self.patterns {
            if !thumbnail_ok(&p.thumbnail) {
                p.thumbnail.clear();
            }
        }
        uniquify(self.char_styles.iter_mut().map(|t| &mut t.name), "Character Style", " ", false);
        uniquify(self.para_styles.iter_mut().map(|t| &mut t.name), "Paragraph Style", " ", false);
        uniquify(self.graphics.iter_mut().map(|g| &mut g.name), "Graphic", " ", false);
        uniquify(self.graphics.iter_mut().map(|g| &mut g.id), "graphic", "-", false);
        uniquify(self.groups.iter_mut().map(|g| &mut g.name), "Group", " ", true);
        for g in &mut self.graphics {
            for v in [&mut g.width, &mut g.height] {
                if !(v.is_finite() && *v >= 0.0) {
                    *v = 0.0;
                }
            }
            if !thumbnail_ok(&g.thumbnail) {
                g.thumbnail.clear();
            }
        }
        let mut groups = std::mem::take(&mut self.groups);
        let present: HashSet<(ItemKind, &str)> = self.items().map(|(k, key, _)| (k, key)).collect();
        let mut seen = HashSet::new();
        for g in &mut groups {
            g.items.retain(|r| present.contains(&(r.kind, r.item.as_str())) && seen.insert(r.clone()));
        }
        self.groups = groups;
    }
}

/// Make `g` a library gradient: no placement, freeform points or swatch links (a linked stop keeps
/// the color it shows), the angle its placement showed (a finite one), and stops in order with
/// offsets, opacities and midpoints in range. False when it has fewer than 2 or more than
/// [`MAX_STOPS`] stops.
fn clean_gradient(g: &mut GradientPaint) -> bool {
    if !(2..=MAX_STOPS).contains(&g.gradient.stops.len()) {
        return false;
    }
    g.angle = g.geom.map_or(g.angle, |geom| geom.angle_deg());
    g.geom = None;
    g.freeform = None;
    g.swatch = None;
    if !g.angle.is_finite() {
        g.angle = 0.0;
    }
    let within = |v: f32, lo: f32, hi: f32, or: f32| if v.is_finite() { v.clamp(lo, hi) } else { or };
    for st in &mut g.gradient.stops {
        st.offset = within(st.offset, 0.0, 1.0, 0.0);
        st.opacity = within(st.opacity, 0.0, 1.0, 1.0);
        st.midpoint = within(st.midpoint, 0.13, 0.87, 0.5);
        st.swatch = None;
        st.tint = 1.0;
    }
    g.gradient.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    true
}

/// `name` cut to [`MAX_NAME`] characters.
fn clip(name: &mut String) {
    if let Some((at, _)) = name.char_indices().nth(MAX_NAME) {
        name.truncate(at);
    }
}

/// Clip each of `names` ([`clip`]) and make it unique among them (ignoring case when `fold`): an
/// empty one becomes `fallback`, a repeated one gets a number after `sep` (`Blue 2`).
fn uniquify<'a>(names: impl Iterator<Item = &'a mut String>, fallback: &str, sep: &str, fold: bool) {
    let key = |n: &str| if fold { n.to_lowercase() } else { n.to_string() };
    let mut seen = HashSet::new();
    for name in names {
        clip(name);
        if name.trim().is_empty() {
            *name = fallback.to_string();
        }
        if seen.contains(&key(name)) {
            let base = name.clone();
            *name = (2..).map(|n| format!("{base}{sep}{n}")).find(|n| !seen.contains(&key(n))).unwrap_or_default();
        }
        seen.insert(key(name));
    }
}

/// Whether `b64` is empty or a PNG of at most [`MAX_THUMBNAIL_SIDE`] pixels a side, judged by its
/// header: the panel decodes it.
fn thumbnail_ok(b64: &str) -> bool {
    if b64.is_empty() {
        return true;
    }
    // 32 base64 characters: 24 bytes, the signature then the IHDR chunk's length, type, width and
    // height.
    let Some(head) = b64.get(..32).filter(|_| b64.len() <= MAX_THUMBNAIL).and_then(vectorcraft_format::base64_decode) else { return false };
    let side = |at: usize| {
        head.get(at..at + 4).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_be_bytes).is_some_and(|v| (1..=MAX_THUMBNAIL_SIDE).contains(&v))
    };
    head.starts_with(b"\x89PNG\r\n\x1a\n") && head.get(12..16) == Some(b"IHDR".as_slice()) && side(16) && side(20)
}

#[cfg(not(target_arch = "wasm32"))]
fn library_paths(dir: &str) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut v: Vec<String> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.to_string_lossy().eq_ignore_ascii_case(LIBRARY_EXT)))
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[cfg(target_arch = "wasm32")]
fn library_paths(_: &str) -> Vec<String> {
    vec![]
}

#[cfg(not(target_arch = "wasm32"))]
fn remove_file(path: &str) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(EngineError::Other(format!("{path}: {e}"))),
    }
}

#[cfg(target_arch = "wasm32")]
fn remove_file(_: &str) -> Result<()> {
    Ok(())
}

/// Library `name` as a file name without its extension: letters, digits, spaces, `-` and `_`
/// kept, other characters as `-`, and a `-` after a Windows device name such as `CON` or `COM1`
/// (`fileio::safe_file_name`); "Library" when nothing is left.
pub fn file_stem_for(name: &str) -> String {
    let safe: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '-' }).collect();
    let safe = safe.trim();
    if safe.is_empty() { "Library".to_string() } else { super::fileio::safe_file_name(safe) }
}

fn stem_of(path: &str) -> String {
    std::path::Path::new(path).file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// The kinds of library item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Graphic,
    FillColor,
    StrokeColor,
    FillGradient,
    StrokeGradient,
    FillPattern,
    StrokePattern,
    CharStyle,
    ParaStyle,
}

impl Kind {
    fn of(p: &Value, cmd: &str) -> Result<Self> {
        Ok(match str_param(p, "kind") {
            Some("graphic") => Kind::Graphic,
            Some("fillColor") => Kind::FillColor,
            Some("strokeColor") => Kind::StrokeColor,
            Some("color") => Kind::FillColor,
            Some("fillGradient" | "gradient") => Kind::FillGradient,
            Some("strokeGradient") => Kind::StrokeGradient,
            Some("fillPattern" | "pattern") => Kind::FillPattern,
            Some("strokePattern") => Kind::StrokePattern,
            Some("charStyle") => Kind::CharStyle,
            Some("paraStyle") => Kind::ParaStyle,
            _ => {
                return Err(bad(
                    cmd,
                    "kind must be graphic, fillColor, strokeColor, fillGradient, strokeGradient, fillPattern, strokePattern, charStyle or paraStyle",
                ));
            }
        })
    }

    /// The kind as a group names it (fill and stroke colors, gradients and patterns are one list
    /// each).
    fn item_kind(self) -> ItemKind {
        match self {
            Kind::Graphic => ItemKind::Graphic,
            Kind::FillColor | Kind::StrokeColor => ItemKind::Color,
            Kind::FillGradient | Kind::StrokeGradient => ItemKind::Gradient,
            Kind::FillPattern | Kind::StrokePattern => ItemKind::Pattern,
            Kind::CharStyle => ItemKind::CharStyle,
            Kind::ParaStyle => ItemKind::ParaStyle,
        }
    }

    /// Whether the kind paints strokes by default.
    fn stroke(self) -> bool {
        matches!(self, Kind::StrokeColor | Kind::StrokeGradient | Kind::StrokePattern)
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "library.list",
            "Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, graphics, colors, gradients, patterns, charStyles, paraStyles, groups} (counts)], current: the current library's id or null, folder: the library folder, or null when libraries last for the session only (the web, headless)}",
            always,
            list
        ),
        cmd!(
            query "library.get",
            "Library",
            [],
            None,
            "{library?: id or name (default: the current library)} → {id, name, graphics: [{id, name, width, height, thumbnail: PNG base64}], colors: [{name, hex, color}], gradients: [{name, gradient: {gradient: {kind, stops}, angle}}], patterns: [{name, pattern: its name in the item, thumbnail: PNG base64}], charStyles: [{name, attrs}], paraStyles: [{name, attrs}], groups: [{name, items: [{kind: color|gradient|pattern|charStyle|paraStyle|graphic, item: name (graphic: id)}]}] (an item is in one group at most; the others are ungrouped)}",
            always,
            get
        ),
        cmd!(
            "library.setCurrent",
            "Show Library",
            [],
            None,
            "{library: id or name} the library the Libraries panel shows, which library commands without a `library` act on → {id}",
            always,
            set_current
        ),
        cmd!(
            "library.create",
            "Create New Library",
            [],
            None,
            "{name?: \"My Library\"} a new empty library (its file in the library folder), made the current one → {id, name}",
            always,
            create
        ),
        cmd!(
            "library.rename",
            "Rename Library",
            [],
            None,
            "{library?, name} rename a library (default: the current one) → {id, name}",
            always,
            rename
        ),
        cmd!(
            "library.delete",
            "Delete Library",
            [],
            None,
            "{library?} delete a library (default: the current one) and its file; not undoable → null",
            always,
            delete
        ),
        cmd!(
            "library.add",
            "Add to Library",
            [],
            None,
            "{library?: id or name (default: the current library, made when there is none), kind: graphic|fillColor|strokeColor|fillGradient|strokeGradient|fillPattern|strokePattern|charStyle|paraStyle, ids?: objects (default: the selection), name?, color?: colour (a colour to add, instead of the selection's), swatch?: a gradient or pattern swatch of the document (fillGradient…strokePattern: added instead of the selection's paint), group?: name, before?: item | after?: item (an item of the same kind)} add an item from the selection: graphic: a copy of the objects with every resource they use; fillColor/strokeColor: the first selected object's solid fill/stroke colour (else the default fill/stroke); fillGradient/strokeGradient: the first selected object's fill/stroke gradient (else the default's), without its placement or swatch links, named after its swatch, else \"Gradient\"; fillPattern/strokePattern: the first selected object's fill/stroke pattern (else the default's) with the resources its art uses, named after it; charStyle/paraStyle: the selected text's character/paragraph attributes (the Type tool's range, else the first selected text object). A new item goes into `group` (default: none) and, with `before` or `after`, just before or after that item in the order its kind is shown in, as dropping it among the Libraries panel's items does; an item the library already has (same kind and contents) isn't added again, nor moved → {library, kind, name, id?: graphic id, existing: bool, group?: name or null (a new item's)}",
            has_doc,
            add
        ),
        cmd!(
            "library.use",
            "Use Library Item",
            [],
            None,
            "{library?, kind, item: name (graphic: id or name), center?: [x, y] (graphic; default: the first artboard's centre), to?: fill|stroke (colors, gradients, patterns; default: the kind's), recorded?: the item as it was when used (recorded in the journal so a replay matches)} graphic: place a copy centred on `center`, its resources pasted along, selected, one undo step; colors and gradients: paint the selection (and the defaults) as paint.setFill/paint.setStroke, a gradient fitting each object; patterns: add the pattern and its swatch to the document (a name the document gives another pattern gets a number; the same pattern is reused) and paint the selection (and the defaults) with it, one undo step; charStyle/paraStyle: add the style to the document (a name taken by different attributes gets a number) and apply it to the selected text, one undo step → {kind, ids?, style?}",
            has_doc,
            use_item
        ),
        cmd!(
            "library.removeItem",
            "Delete Library Item",
            [],
            None,
            "{library?, kind, item: name (graphic: id or name)} delete an item (out of its group too) → null",
            always,
            remove_item
        ),
        cmd!(
            "library.createGroup",
            "New Library Group",
            [],
            None,
            "{library?, name?: \"Group\" (a name the library's groups have, ignoring case, gets a number), items?: [{kind, item}] (moved into it out of their groups)} a new group of items, after the others → {library, group: its name}",
            always,
            create_group
        ),
        cmd!(
            "library.renameGroup",
            "Rename Library Group",
            [],
            None,
            "{library?, group: name, name} rename a group (another group's name, ignoring case, is an error) → {library, group: the new name}",
            always,
            rename_group
        ),
        cmd!(
            "library.deleteGroup",
            "Delete Library Group",
            [],
            None,
            "{library?, group: name} delete a group: its items stay in the library, ungrouped → {library, ungrouped: how many}",
            always,
            delete_group
        ),
        cmd!(
            "library.moveItem",
            "Move Library Item to Group",
            [],
            None,
            "{library?, kind, item: name (graphic: id or name), group?: name (null or none: out of its group), before?: item | after?: item (an item of the same kind, named as `item` is)} put an item in a group (out of the one it was in); with `before` or `after`, also move it just before or after that item in the order its kind is shown in, as dragging it in the Libraries panel does → {library, kind, item, group: name or null}",
            always,
            move_item
        ),
        cmd!(
            "library.moveGroup",
            "Move Library Group",
            [],
            None,
            "{library?, group: name, to: index} move a group to position `to` of the library's groups (0 = first; past the end: last), as dragging its header in the Libraries panel does → {library, group, to: where it went}",
            always,
            move_group
        ),
        cmd!(
            query "library.export",
            "Export Library",
            [],
            None,
            "{library?: id or name (default: the current library), path?} write a library (its colours, gradients, patterns, styles, graphics with their art and thumbnails, and groups) as a .vclibrary file (JSON, as libraries are kept) → {path, name}; without path → {data: the file's text, name}",
            always,
            export
        ),
        cmd!(
            query "library.import",
            "Import Library",
            [],
            None,
            "{path? | data?: file text | dataBase64?, name?: the library's name (default: the file's)} add the library of a .vclibrary file (as library.export writes) as a new library, made the current one; a name another library has gets a number (`Brand (2)`) → {id, name, graphics, colors, gradients, patterns, charStyles, paraStyles, groups (counts)}",
            always,
            import
        ),
    ]
}

/// The library `p` names (`library`: id or name), else the current one.
fn library_id(s: &Session, p: &Value, cmd: &str) -> Result<String> {
    match str_param(p, "library") {
        Some(key) => s.libraries.find(key).ok_or_else(|| bad(cmd, format!("no library `{key}`"))),
        None => s.libraries.current().map(str::to_string).ok_or_else(|| bad(cmd, "there is no library: create one first")),
    }
}

fn set_current(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.setCurrent";
    let key = str_param(p, "library").ok_or_else(|| bad(C, "missing `library`"))?;
    let id = s.libraries.find(key).ok_or_else(|| bad(C, format!("no library `{key}`")))?;
    s.libraries.current = Some(id.clone());
    Ok(json!({ "id": id }))
}

fn counts(lib: &Library) -> Value {
    json!({
        "graphics": lib.graphics.len(),
        "colors": lib.colors.len(),
        "gradients": lib.gradients.len(),
        "patterns": lib.patterns.len(),
        "charStyles": lib.char_styles.len(),
        "paraStyles": lib.para_styles.len(),
        "groups": lib.groups.len(),
    })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let libraries: Vec<Value> = s
        .libraries
        .all()
        .iter()
        .map(|(id, lib)| {
            let mut v = json!({ "id": id, "name": lib.name });
            if let (Some(o), Value::Object(c)) = (v.as_object_mut(), counts(lib)) {
                o.extend(c);
            }
            v
        })
        .collect();
    Ok(json!({ "libraries": libraries, "current": s.libraries.current(), "folder": s.libraries.dir() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let id = library_id(s, p, "library.get")?;
    let lib = s.libraries.get(&id).ok_or_else(|| bad("library.get", format!("no library `{id}`")))?;
    Ok(json!({
        "id": id,
        "name": lib.name,
        "graphics": lib.graphics.iter().map(|g| json!({"id": g.id, "name": g.name, "width": g.width, "height": g.height, "thumbnail": g.thumbnail})).collect::<Vec<_>>(),
        "colors": lib.colors.iter().map(|c| json!({"name": c.name, "hex": c.color.to_hex(), "color": c.color})).collect::<Vec<_>>(),
        "gradients": lib.gradients.iter().map(|g| json!({"name": g.name, "gradient": g.gradient})).collect::<Vec<_>>(),
        "patterns": lib.patterns.iter().map(|p| json!({"name": p.name, "pattern": p.pattern, "thumbnail": p.thumbnail})).collect::<Vec<_>>(),
        "charStyles": lib.char_styles.iter().map(|t| json!({"name": t.name, "attrs": t.attrs})).collect::<Vec<_>>(),
        "paraStyles": lib.para_styles.iter().map(|t| json!({"name": t.name, "attrs": t.attrs})).collect::<Vec<_>>(),
        "groups": lib.groups,
    }))
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or("My Library").to_string();
    let id = s.libraries.create(&name)?;
    Ok(json!({ "id": id, "name": name }))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.rename";
    let id = library_id(s, p, C)?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(C, "missing `name`"))?.to_string();
    let n = name.clone();
    s.libraries.change(&id, |l| {
        l.name = n;
        Ok(())
    })?;
    Ok(json!({ "id": id, "name": name }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let id = library_id(s, p, "library.delete")?;
    s.libraries.delete(&id)?;
    ok()
}

/// `base`, or `base 2`, `base 3`… the first that `taken` doesn't hold.
fn unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    std::iter::once(base.to_string()).chain((2..).map(|n| format!("{base} {n}"))).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.add";
    let kind = Kind::of(p, C)?;
    let given = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    // Read what is added before a library is made for it, so a failure makes none.
    let item = match kind {
        Kind::Graphic => Item::Graphic(graphic_of(s, p, C)?),
        Kind::FillColor | Kind::StrokeColor => Item::Color(color_of(s, p, kind.stroke(), C)?),
        Kind::FillGradient | Kind::StrokeGradient => Item::Gradient(gradient_of(s, p, kind.stroke(), C)?),
        Kind::FillPattern | Kind::StrokePattern => Item::Pattern(pattern_of(s, p, kind.stroke(), C)?),
        Kind::CharStyle | Kind::ParaStyle => {
            let para = kind == Kind::ParaStyle;
            let attrs = super::textstyles::selected_style_attrs(s, para)?.ok_or_else(|| bad(C, "select some type first"))?;
            Item::Style(para, attrs)
        }
    };
    let place = Placement::of(p, C)?;
    let kind_name = str_param(p, "kind").unwrap_or_default().to_string();
    let item_kind = kind.item_kind();
    let put = |lib: &mut Library| {
        let full = |n: usize| if n >= MAX_ITEMS { Err(bad(C, format!("the library holds {MAX_ITEMS} items of this kind already"))) } else { Ok(()) };
        Ok(match item {
            Item::Graphic(mut g) => {
                full(lib.graphics.len())?;
                if let Some(old) = lib.graphics.iter().find(|o| o.data == g.data) {
                    return Ok(json!({ "name": old.name, "id": old.id, "existing": true }));
                }
                let base = given.clone().unwrap_or_else(|| g.name.clone());
                g.name = unique(&base, |n| lib.graphics.iter().any(|o| o.name == n));
                // Ids are `graphic`, `graphic-2`…: taken ones are looked for as they are kept.
                g.id = unique("graphic", |n| lib.graphics.iter().any(|o| o.id == n.replace(' ', "-"))).replace(' ', "-");
                let v = json!({ "name": g.name, "id": g.id, "existing": false });
                let key = g.id.clone();
                lib.graphics.push(g);
                placed(lib, &place, ItemRef { kind: item_kind, item: key }, v)?
            }
            Item::Color((name, color)) => {
                full(lib.colors.len())?;
                if let Some(old) = lib.colors.iter().find(|o| o.color == color) {
                    return Ok(json!({ "name": old.name, "existing": true }));
                }
                let name = unique(&given.clone().unwrap_or(name), |n| lib.colors.iter().any(|o| o.name == n));
                lib.colors.push(LibraryColor { name: name.clone(), color });
                placed(lib, &place, ItemRef { kind: item_kind, item: name.clone() }, json!({ "name": name, "existing": false }))?
            }
            Item::Gradient(mut g) => {
                full(lib.gradients.len())?;
                if let Some(old) = lib.gradients.iter().find(|o| o.gradient == g.gradient) {
                    return Ok(json!({ "name": old.name, "existing": true }));
                }
                g.name = unique(&given.clone().unwrap_or(g.name), |n| lib.gradients.iter().any(|o| o.name == n));
                let v = json!({ "name": g.name, "existing": false });
                let key = g.name.clone();
                lib.gradients.push(g);
                placed(lib, &place, ItemRef { kind: item_kind, item: key }, v)?
            }
            Item::Pattern(mut pat) => {
                full(lib.patterns.len())?;
                if let Some(old) = lib.patterns.iter().find(|o| o.data == pat.data) {
                    return Ok(json!({ "name": old.name, "existing": true }));
                }
                pat.name = unique(&given.clone().unwrap_or(pat.name), |n| lib.patterns.iter().any(|o| o.name == n));
                let v = json!({ "name": pat.name, "existing": false });
                let key = pat.name.clone();
                lib.patterns.push(pat);
                placed(lib, &place, ItemRef { kind: item_kind, item: key }, v)?
            }
            Item::Style(para, attrs) => {
                let list = if para { &mut lib.para_styles } else { &mut lib.char_styles };
                full(list.len())?;
                if let Some(old) = list.iter().find(|o| o.attrs == attrs) {
                    return Ok(json!({ "name": old.name, "existing": true }));
                }
                let base = given.clone().unwrap_or_else(|| style_name(&attrs, para));
                let name = unique(&base, |n| list.iter().any(|o| o.name == n));
                list.push(TextStyleDef { name: name.clone(), attrs });
                placed(lib, &place, ItemRef { kind: item_kind, item: name.clone() }, json!({ "name": name, "existing": false }))?
            }
        })
    };
    // Into the library `p` names, else the current one, else a new "My Library", which is kept
    // only once the item is in it (a `group` or item to place it by that it lacks makes none).
    let (id, mut r) = if p.get("library").is_some_and(|v| !v.is_null()) || s.libraries.current().is_some() {
        let id = library_id(s, p, C)?;
        let r = s.libraries.change(&id, put)?;
        (id, r)
    } else {
        let mut lib = Library { name: "My Library".to_string(), ..Default::default() };
        let r = put(&mut lib)?;
        (s.libraries.insert(lib)?, r)
    };
    r["library"] = json!(id);
    r["kind"] = json!(kind_name);
    Ok(r)
}

/// New item `r` of `lib` put where `place` says, its group added to `v` (what `library.add`
/// reports).
fn placed(lib: &mut Library, place: &Placement, r: ItemRef, mut v: Value) -> Result<Value> {
    v["group"] = json!(place.apply(lib, r, "library.add")?);
    Ok(v)
}

enum Item {
    Graphic(LibraryGraphic),
    Color((String, Color)),
    Gradient(LibraryGradient),
    Pattern(LibraryPattern),
    Style(bool, Map<String, Value>),
}

/// The selected objects (or `ids`) as a library graphic (its id and name set by the caller).
fn graphic_of(s: &Session, p: &Value, cmd: &str) -> Result<LibraryGraphic> {
    let st = s.doc()?;
    let ids = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => st.selection.in_paint_order(&st.doc),
    };
    let roots = super::edit::roots_of(&st.doc, ids);
    if roots.is_empty() {
        return Err(bad(cmd, "select the art to add first"));
    }
    let clip = Clipboard::copy(st, &roots);
    let bounds = clip.bounds().ok_or_else(|| bad(cmd, "the selection draws nothing"))?;
    let mut doc = clip.to_document();
    let board = bounds.inflate(1.0, 1.0);
    // One artboard round the art: what the thumbnail shows.
    doc.artboards.truncate(1);
    match doc.artboards.first_mut() {
        Some(a) => a.rect = board,
        None => return Err(bad(cmd, "the graphic's document has no artboard")),
    }
    let name = match roots.as_slice() {
        [one] => st.doc.node(*one).map(|n| n.display_name()).unwrap_or_default(),
        _ => String::new(),
    };
    let name = if name.trim().is_empty() || name.starts_with('<') { "Graphic".to_string() } else { name };
    let thumbnail = super::fileio::preview_png(&doc)?.map(|b| vectorcraft_format::base64_encode(&b)).unwrap_or_default();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&doc, false));
    Ok(LibraryGraphic { id: String::new(), name, width: bounds.width(), height: bounds.height(), data, thumbnail })
}

/// The first selected object's solid fill (`stroke`: stroke) colour, else the default one, with a
/// name: its swatch's, else its hex.
fn color_of(s: &Session, p: &Value, stroke: bool, cmd: &str) -> Result<(String, Color)> {
    // A colour given (a swatch dropped on the panel).
    if let Some(c) = p.get("color").filter(|v| !v.is_null()) {
        let color = color_value(c).ok_or_else(|| bad(cmd, "bad colour"))?;
        return Ok((color.to_hex(), color));
    }
    match selected_paint(s, p, stroke)? {
        Paint::Solid { color, swatch, .. } => {
            let name = swatch.filter(|n| !n.is_empty()).unwrap_or_else(|| color.to_hex());
            Ok((name, color))
        }
        Paint::None => Err(bad(cmd, if stroke { "the selection has no stroke colour" } else { "the selection has no fill colour" })),
        _ => Err(bad(cmd, "only solid colours can be added (not gradients or patterns)")),
    }
}

/// The first selected object's (or `ids`') fill (`stroke`: stroke) paint (type's: its first
/// character's, as the proxy shows it), else the default one.
fn selected_paint(s: &Session, p: &Value, stroke: bool) -> Result<Paint> {
    let st = s.doc()?;
    let ids = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => st.selection.objects.clone(),
    };
    let from_art = ids.iter().filter_map(|id| st.doc.node(*id)).find_map(|n| {
        let (paint, to_doc, _) = n.proxy_paint(stroke, None)?;
        // Type's characters paint in its own space: a gradient there takes its placement in the
        // document, so that it keeps the direction it shows.
        match (paint, n.proxy_gradient(stroke, None)) {
            (Paint::Gradient(g), Some((_, geom))) if to_doc != vectorcraft_geom::Affine::IDENTITY => {
                Some(Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..(**g).clone() })))
            }
            _ => Some(paint.clone()),
        }
    });
    Ok(match from_art {
        Some(p) => p,
        None => {
            let d = &s.paint;
            if stroke { d.stroke.clone() } else { d.fill.clone() }
        }
    })
}

/// Gradient swatch `swatch` of the document, else the first selected object's fill (`stroke`:
/// stroke) gradient, else the default one, as a library gradient ([`clean_gradient`]) named after
/// its swatch, else "Gradient".
fn gradient_of(s: &Session, p: &Value, stroke: bool, cmd: &str) -> Result<LibraryGradient> {
    let (name, paint) = match str_param(p, "swatch") {
        Some(name) => {
            let sw = s.doc()?.doc.swatch(name).filter(|w| matches!(w.paint, Paint::Gradient(_)));
            (Some(name.to_string()), sw.ok_or_else(|| bad(cmd, format!("no gradient swatch `{name}`")))?.paint.clone())
        }
        None => (None, selected_paint(s, p, stroke)?),
    };
    let Paint::Gradient(mut g) = paint else {
        return Err(bad(cmd, if stroke { "the selection has no gradient stroke" } else { "the selection has no gradient fill" }));
    };
    let name = name.or_else(|| g.swatch.clone()).filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Gradient".to_string());
    if !clean_gradient(&mut g) {
        return Err(bad(cmd, "the library item is damaged"));
    }
    Ok(LibraryGradient { name, gradient: *g })
}

/// Pattern swatch `swatch` of the document, else the first selected object's fill (`stroke`:
/// stroke) pattern, else the default one, as a library pattern named after it: a document of one
/// rectangle painted with it, one period of its tiling, with the resources its art uses.
fn pattern_of(s: &Session, p: &Value, stroke: bool, cmd: &str) -> Result<LibraryPattern> {
    let d = &s.doc()?.doc;
    let name = match str_param(p, "swatch") {
        Some(name) => match d.swatch(name).map(|w| &w.paint) {
            Some(Paint::Pattern { pattern, .. }) => pattern.clone(),
            _ if d.pattern(name).is_some() => name.to_string(),
            _ => return Err(bad(cmd, format!("no pattern swatch `{name}`"))),
        },
        None => match selected_paint(s, p, stroke)? {
            Paint::Pattern { pattern, .. } => pattern,
            _ => return Err(bad(cmd, if stroke { "the selection has no pattern stroke" } else { "the selection has no pattern fill" })),
        },
    };
    let def = d.pattern(&name).ok_or_else(|| bad(cmd, format!("no pattern swatch `{name}`")))?;
    let (w, h) = def.period();
    let r = vectorcraft_geom::Rect::from_origin_size((0.0, 0.0), (w, h));
    let paint = vectorcraft_doc::pattern::pattern_paint(&name);
    let node = Node::path(NodeId(1), vectorcraft_geom::shapes::rectangle(r), Appearance::basic(paint, Paint::None, 0.0));
    let mut doc = Clipboard::gather(d, vec![node]).to_document();
    doc.artboards.truncate(1);
    match doc.artboards.first_mut() {
        Some(a) => a.rect = r,
        None => return Err(bad(cmd, "the graphic's document has no artboard")),
    }
    let thumbnail = super::fileio::preview_png(&doc)?.map(|b| vectorcraft_format::base64_encode(&b)).unwrap_or_default();
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&doc, false));
    Ok(LibraryPattern { name: name.clone(), pattern: name, data, thumbnail })
}

/// A character style's name from its font, style and size ("Source Sans 3 Bold 24 pt"); a
/// paragraph style's from its alignment and size too, else "Paragraph Style".
fn style_name(attrs: &Map<String, Value>, para: bool) -> String {
    let s = |k: &str| attrs.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let size = attrs.get("size").and_then(Value::as_f64).map(|v| format!("{} pt", (v * 100.0).round() / 100.0));
    let words: Vec<String> = [s("font_family"), s("font_style")].into_iter().chain(size).filter(|w| !w.is_empty()).collect();
    match (para, words.is_empty()) {
        (false, false) => words.join(" "),
        (false, true) => "Character Style".into(),
        (true, _) => "Paragraph Style".into(),
    }
}

/// The library item `p` names: (library id, kind).
fn item_param<'a>(s: &Session, p: &'a Value, cmd: &str) -> Result<(String, Kind, &'a str)> {
    let id = library_id(s, p, cmd)?;
    let kind = Kind::of(p, cmd)?;
    let item = str_param(p, "item").ok_or_else(|| bad(cmd, "missing `item`"))?;
    Ok((id, kind, item))
}

fn use_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.use";
    // A replay (the journal) carries what the item was, so it doesn't depend on the library.
    let recorded = p.get("recorded").cloned();
    let (kind, data) = match recorded {
        Some(r) => (Kind::of(p, C)?, r),
        None => {
            let (id, kind, item) = item_param(s, p, C)?;
            let lib = s.libraries.get(&id).ok_or_else(|| bad(C, format!("no library `{id}`")))?;
            let missing = || bad(C, format!("the library has no item `{item}`"));
            let data = match kind {
                Kind::Graphic => {
                    let g =
                        lib.graphics.iter().find(|g| g.id == item).or_else(|| lib.graphics.iter().find(|g| g.name == item)).ok_or_else(missing)?;
                    json!({ "data": g.data })
                }
                Kind::FillColor | Kind::StrokeColor => {
                    let c = lib.colors.iter().find(|c| c.name == item).ok_or_else(missing)?;
                    json!({ "color": c.color })
                }
                Kind::FillGradient | Kind::StrokeGradient => {
                    let g = lib.gradients.iter().find(|g| g.name == item).ok_or_else(missing)?;
                    json!({ "gradient": g.gradient })
                }
                Kind::FillPattern | Kind::StrokePattern => {
                    let pat = lib.patterns.iter().find(|x| x.name == item).ok_or_else(missing)?;
                    json!({ "pattern": pat.pattern, "data": pat.data })
                }
                Kind::CharStyle | Kind::ParaStyle => {
                    let list = if kind == Kind::ParaStyle { &lib.para_styles } else { &lib.char_styles };
                    let t = list.iter().find(|t| t.name == item).ok_or_else(missing)?;
                    json!({ "name": t.name, "attrs": t.attrs })
                }
            };
            s.note_journal("recorded", data.clone());
            (kind, data)
        }
    };
    match kind {
        Kind::Graphic => {
            let bytes = data
                .get("data")
                .and_then(Value::as_str)
                .and_then(vectorcraft_format::base64_decode)
                .ok_or_else(|| bad(C, "the graphic's data is damaged"))?;
            let doc = vectorcraft_format::load(&bytes).map_err(|e| bad(C, format!("the graphic can't be read: {e}")))?;
            let clip = Clipboard::from_document(&doc);
            if clip.is_empty() {
                return Err(bad(C, "the graphic is empty"));
            }
            let center = match point_param(p, "center") {
                Some(c) => c,
                None => s.doc()?.doc.artboards.first().map(|a| a.rect.center()).unwrap_or_default(),
            };
            let mut r = super::edit::place_clip(s, &clip, center)?;
            r["kind"] = json!("graphic");
            Ok(r)
        }
        Kind::FillColor | Kind::StrokeColor => {
            let stroke = to_stroke(p, kind)?;
            let color = data.get("color").cloned().ok_or_else(|| bad(C, "no colour"))?;
            s.execute(if stroke { "paint.setStroke" } else { "paint.setFill" }, &json!({ "color": color }))?;
            Ok(json!({ "kind": if stroke { "strokeColor" } else { "fillColor" } }))
        }
        Kind::FillGradient | Kind::StrokeGradient => {
            let stroke = to_stroke(p, kind)?;
            let damaged = || bad(C, "the library item is damaged");
            let mut g: GradientPaint = data.get("gradient").cloned().and_then(|v| serde_json::from_value(v).ok()).ok_or_else(damaged)?;
            if !clean_gradient(&mut g) {
                return Err(damaged());
            }
            // RGB stops in a CMYK document are stored as CMYK, as a library color is.
            for st in &mut g.gradient.stops {
                st.color = super::paint::new_color(s, &json!({}), st.color);
            }
            super::paint::apply_paint(s, &json!({}), Paint::Gradient(Box::new(g)), !stroke)?;
            Ok(json!({ "kind": if stroke { "strokeGradient" } else { "fillGradient" } }))
        }
        Kind::FillPattern | Kind::StrokePattern => {
            let stroke = to_stroke(p, kind)?;
            let pattern = use_pattern(s, &data, stroke)?;
            Ok(json!({ "kind": if stroke { "strokePattern" } else { "fillPattern" }, "pattern": pattern }))
        }
        Kind::CharStyle | Kind::ParaStyle => {
            let para = kind == Kind::ParaStyle;
            let name = data.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).unwrap_or("Style").to_string();
            let attrs = data.get("attrs").and_then(Value::as_object).cloned().unwrap_or_default();
            let doc = &s.doc()?.doc;
            // The document's style of that name when it is the same; else a new one.
            let same = super::textstyles::style_attrs(doc, para, &name).is_some_and(|a| a == attrs);
            let style = if same { name } else { unique(&name, |n| super::textstyles::style_attrs(doc, para, n).is_some()) };
            let prefix = if para { "paraStyle" } else { "charStyle" };
            in_one_step(s, |s| {
                if !same {
                    s.execute(&format!("{prefix}.new"), &json!({ "name": style, "attrs": attrs }))?;
                }
                s.execute(&format!("{prefix}.apply"), &json!({ "name": style }))
            })?;
            Ok(json!({ "kind": if para { "paraStyle" } else { "charStyle" }, "style": style }))
        }
    }
}

/// The `to` param of `library.use`: whether the item paints the stroke (default: the kind's).
fn to_stroke(p: &Value, kind: Kind) -> Result<bool> {
    match str_param(p, "to") {
        Some("fill") => Ok(false),
        Some("stroke") => Ok(true),
        Some(_) => Err(bad("library.use", "to must be fill or stroke")),
        None => Ok(kind.stroke()),
    }
}

/// Add library pattern `data` (`{pattern, data}`) to the document, with its swatch and the
/// resources its art uses (the same pattern there already is reused; a name the document gives
/// another one gets a number), and paint the selection's fill (`stroke`: stroke) and the default
/// with it, one undo step → the pattern's name in the document.
fn use_pattern(s: &mut Session, data: &Value, stroke: bool) -> Result<String> {
    const C: &str = "library.use";
    let damaged = || bad(C, "the library item is damaged");
    let bytes = data.get("data").and_then(Value::as_str).and_then(vectorcraft_format::base64_decode).ok_or_else(damaged)?;
    let doc = vectorcraft_format::load(&bytes).map_err(|_| damaged())?;
    let name = data.get("pattern").and_then(Value::as_str).filter(|n| doc.pattern(n).is_some()).ok_or_else(damaged)?.to_string();
    // Only what paints with the pattern is brought along.
    let clip = Clipboard::from_document(&doc);
    let clip = Clipboard::gather(&doc, clip.nodes.into_iter().filter(|n| vectorcraft_doc::pattern::uses_pattern(n, Some(&name))).collect());
    if clip.nodes.is_empty() {
        return Err(damaged());
    }
    let choices = SwatchChoices::default();
    // The pattern's name in the document, worked out on a copy: nothing to add is no edit.
    let mut probe = (*s.doc()?.doc).clone();
    let imported = clip.import_into(&mut probe, &choices, false);
    let renamed = imported.renamed.iter().find(|r| r["kind"] == "pattern" && r["from"] == name.as_str()).and_then(|r| r["to"].as_str());
    let target = renamed.unwrap_or(&name).to_string();
    let paint = vectorcraft_doc::pattern::pattern_paint(&target);
    // A pattern the document holds without a swatch (an imported SVG's) gets one.
    let has_swatch =
        |d: &vectorcraft_doc::Document| d.swatches_iter().any(|w| matches!(&w.paint, Paint::Pattern { pattern, .. } if *pattern == target));
    let added = imported.added > 0 || !has_swatch(&probe);
    drop(probe);
    in_one_step(s, |s| {
        if added {
            s.edit("Add to Swatches", |d, _| {
                clip.import_into(d, &choices, false);
                if !has_swatch(d) {
                    let swatch = d.free_swatch_name(&target);
                    d.swatches.push(vectorcraft_color::Swatch { name: swatch, paint: paint.clone(), global: false, spot: false });
                }
                Ok(())
            })?;
        }
        // The pattern itself, not a swatch of its name (a color swatch can share it); `swatch` keeps
        // the placement of a pattern the targets already have, as picking its swatch does.
        super::paint::apply_paint(s, &json!({ "swatch": target }), paint.clone(), !stroke)
    })?;
    Ok(target)
}

/// Run `f`'s commands as one undo step, or as part of the step of an undo group open in the active
/// document, and undo them together when it fails ([`Session::begin_undo_scope`]).
fn in_one_step<T>(s: &mut Session, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let scope = s.begin_undo_scope();
    let r = f(s);
    if let Some(scope) = scope {
        s.end_undo_scope(scope, r.is_err());
    }
    r
}

fn remove_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.removeItem";
    let (id, kind, item) = item_param(s, p, C)?;
    let item = item.to_string();
    s.libraries.change(&id, |lib| {
        let r = lib.item_ref(kind.item_kind(), &item).ok_or_else(|| bad(C, format!("the library has no item `{item}`")))?;
        match kind {
            Kind::Graphic => lib.graphics.retain(|g| g.id != r.item),
            Kind::FillColor | Kind::StrokeColor => lib.colors.retain(|c| c.name != r.item),
            Kind::FillGradient | Kind::StrokeGradient => lib.gradients.retain(|g| g.name != r.item),
            Kind::FillPattern | Kind::StrokePattern => lib.patterns.retain(|x| x.name != r.item),
            Kind::CharStyle => lib.char_styles.retain(|t| t.name != r.item),
            Kind::ParaStyle => lib.para_styles.retain(|t| t.name != r.item),
        }
        lib.ungroup(&r);
        Ok(())
    })?;
    ok()
}

/// `p`'s `name`, trimmed and clipped, else `default`.
fn name_param(p: &Value, default: &str) -> String {
    let mut name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(default).to_string();
    clip(&mut name);
    name
}

/// `p`'s `group`.
fn group_param<'a>(p: &'a Value, cmd: &str) -> Result<&'a str> {
    str_param(p, "group").ok_or_else(|| bad(cmd, "missing `group`"))
}

/// Group `name` of `lib`: its index.
fn group_at(lib: &Library, name: &str, cmd: &str) -> Result<usize> {
    lib.group_position(name).ok_or_else(|| bad(cmd, format!("the library has no group `{name}`")))
}

fn create_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.createGroup";
    let id = library_id(s, p, C)?;
    let base = name_param(p, "Group");
    let wanted = match p.get("items") {
        None | Some(Value::Null) => vec![],
        Some(Value::Array(items)) if items.len() <= 4 * MAX_ITEMS => items
            .iter()
            .map(|v| Ok((Kind::of(v, C)?.item_kind(), str_param(v, "item").ok_or_else(|| bad(C, "each of `items` is {kind, item}"))?)))
            .collect::<Result<Vec<_>>>()?,
        Some(_) => return Err(bad(C, "`items` is a list of {kind, item}")),
    };
    let name = s.libraries.change(&id, |lib| {
        if lib.groups.len() >= MAX_GROUPS {
            return Err(bad(C, format!("the library holds {MAX_GROUPS} groups already")));
        }
        let refs = wanted
            .iter()
            .map(|&(kind, item)| lib.item_ref(kind, item).ok_or_else(|| bad(C, format!("the library has no item `{item}`"))))
            .collect::<Result<Vec<_>>>()?;
        let name = unique(&base, |n| lib.group_position(n).is_some());
        let mut items = vec![];
        let mut seen = HashSet::new();
        for r in refs {
            lib.ungroup(&r);
            if seen.insert(r.clone()) {
                items.push(r);
            }
        }
        lib.groups.push(LibraryGroup { name: name.clone(), items });
        Ok(name)
    })?;
    Ok(json!({ "library": id, "group": name }))
}

fn rename_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.renameGroup";
    let id = library_id(s, p, C)?;
    let group = group_param(p, C)?;
    if str_param(p, "name").is_none_or(|n| n.trim().is_empty()) {
        return Err(bad(C, "missing `name`"));
    }
    let name = name_param(p, "");
    s.libraries.change(&id, |lib| {
        let i = group_at(lib, group, C)?;
        if lib.group_position(&name).is_some_and(|j| j != i) {
            return Err(bad(C, format!("the library has a group `{name}` already")));
        }
        if let Some(g) = lib.groups.get_mut(i) {
            g.name.clone_from(&name);
        }
        Ok(())
    })?;
    Ok(json!({ "library": id, "group": name }))
}

fn delete_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.deleteGroup";
    let id = library_id(s, p, C)?;
    let group = group_param(p, C)?;
    let ungrouped = s.libraries.change(&id, |lib| {
        let i = group_at(lib, group, C)?;
        Ok(if i < lib.groups.len() { lib.groups.remove(i).items.len() } else { 0 })
    })?;
    Ok(json!({ "library": id, "ungrouped": ungrouped }))
}

fn move_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.moveItem";
    let (id, kind, item) = item_param(s, p, C)?;
    let place = Placement::of(p, C)?;
    let (key, to) = s.libraries.change(&id, |lib| {
        let r = lib.item_ref(kind.item_kind(), item).ok_or_else(|| bad(C, format!("the library has no item `{item}`")))?;
        let key = r.item.clone();
        let to = place.apply(lib, r, C)?;
        Ok((key, to))
    })?;
    Ok(json!({ "library": id, "kind": str_param(p, "kind"), "item": key, "group": to }))
}

/// Where `library.add` and `library.moveItem` put an item: its `group` (none: out of any), and
/// the item of its kind it goes `before` or `after`.
struct Placement<'a> {
    group: Option<&'a str>,
    next_to: Option<(&'a str, bool)>,
}

impl<'a> Placement<'a> {
    fn of(p: &'a Value, cmd: &str) -> Result<Self> {
        let next_to = match (str_param(p, "before"), str_param(p, "after")) {
            (Some(_), Some(_)) => return Err(bad(cmd, "give `before` or `after`, not both")),
            (Some(b), None) => Some((b, false)),
            (None, Some(a)) => Some((a, true)),
            (None, None) => None,
        };
        Ok(Self { group: str_param(p, "group"), next_to })
    }

    /// Put item `r` of `lib` where this says ([`Library::put`]) → its group's name.
    fn apply(&self, lib: &mut Library, r: ItemRef, cmd: &str) -> Result<Option<String>> {
        let at = self.group.map(|g| group_at(lib, g, cmd)).transpose()?;
        let next_to = self
            .next_to
            .map(|(other, after)| {
                lib.item_ref(r.kind, other).map(|o| (o, after)).ok_or_else(|| bad(cmd, format!("the library has no item `{other}`")))
            })
            .transpose()?;
        Ok(lib.put(r, at, next_to.as_ref()))
    }
}

fn move_group(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.moveGroup";
    let id = library_id(s, p, C)?;
    let group = group_param(p, C)?;
    let to = p.get("to").and_then(Value::as_u64).ok_or_else(|| bad(C, "missing `to` (an index)"))?;
    let (name, at) = s.libraries.change(&id, |lib| {
        let i = group_at(lib, group, C)?;
        let g = lib.groups.remove(i);
        let at = index(to).min(lib.groups.len());
        let name = g.name.clone();
        lib.groups.insert(at, g);
        Ok((name, at))
    })?;
    Ok(json!({ "library": id, "group": name, "to": at }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.export";
    let id = library_id(s, p, C)?;
    let lib = s.libraries.get(&id).ok_or_else(|| bad(C, format!("no library `{id}`")))?;
    let text = serde_json::to_string(lib).map_err(|e| bad(C, e.to_string()))?;
    let name = lib.name.clone();
    match str_param(p, "path") {
        Some(path) => {
            super::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({ "path": path, "name": name }))
        }
        None => Ok(json!({ "data": text, "name": name })),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.import";
    let too_big = || bad(C, format!("the file is larger than {} MB", MAX_FILE >> 20));
    let bytes: std::borrow::Cow<[u8]> = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => {
            if super::fileio::file_stamp(path).is_some_and(|(len, _)| len > MAX_FILE) {
                return Err(too_big());
            }
            super::fileio::read_file(path)?.into()
        }
        (None, Some(text), _) => text.as_bytes().into(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad dataBase64"))?.into(),
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    if bytes.len() as u64 > MAX_FILE {
        return Err(too_big());
    }
    let not_library = |e: String| bad(C, format!("not a library file: {e}"));
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| not_library(e.to_string()))?;
    drop(bytes);
    // A library names itself; another file of ours (swatches, styles) says what it is.
    if !v.get("name").is_some_and(Value::is_string) || v.get("format").is_some_and(|f| f.as_str() != Some(LIBRARY_EXT)) {
        return Err(not_library("it has no library name".into()));
    }
    let mut lib: Library = serde_json::from_value(v).map_err(|e| not_library(e.to_string()))?;
    lib.sanitize();
    let base = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).unwrap_or(lib.name.trim()).to_string();
    let mut base = if base.is_empty() { "Library".to_string() } else { base };
    clip(&mut base);
    let taken = |n: &str| s.libraries.all().iter().any(|(_, l)| l.name.to_lowercase() == n.to_lowercase());
    lib.name = std::iter::once(base.clone()).chain((2..).map(|n| format!("{base} ({n})"))).find(|n| !taken(n)).unwrap_or(base);
    let mut r = json!({ "name": lib.name });
    if let (Some(o), Value::Object(c)) = (r.as_object_mut(), counts(&lib)) {
        o.extend(c);
    }
    r["id"] = json!(s.libraries.insert(lib)?);
    Ok(r)
}
