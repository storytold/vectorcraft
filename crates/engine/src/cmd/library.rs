//! Libraries (Window › Libraries): local libraries of graphics, colours and text styles that every
//! document can use. A library is one `.vclibrary` file (JSON) in the library folder next to the
//! preferences, which the desktop app sets; without a folder (the web, headless sessions) libraries
//! live for the session only. A graphic is kept as a native document with a PNG thumbnail: adding
//! one copies the selection as Copy does, and placing it pastes a copy as Paste does (its images,
//! symbols, patterns and swatches come along; a swatch name the document gives another colour is
//! merged into the document's).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::TextStyleDef;

use super::*;
use crate::Clipboard;

/// The extension of a library file.
pub const LIBRARY_EXT: &str = "vclibrary";
/// Bytes a library file may have when read (graphics with large images included).
const MAX_FILE: u64 = 512 << 20;
/// Items of one kind a library may hold.
const MAX_ITEMS: usize = 10_000;

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
                    lib.cap();
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
        let base: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '-' }).collect();
        let base = base.trim();
        let base = if base.is_empty() { "Library" } else { base };
        let taken = |id: &str| self.libs.iter().any(|(i, _)| i.eq_ignore_ascii_case(id)) || self.file_exists(id);
        let id = std::iter::once(base.to_string()).chain((2..).map(|n| format!("{base} {n}"))).find(|id| !taken(id)).unwrap_or_default();
        self.libs.push((id.clone(), Library { name: name.to_string(), ..Default::default() }));
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

    /// Change library `id` with `f` and save it; on a failed save the change is undone.
    fn change<T>(&mut self, id: &str, f: impl FnOnce(&mut Library) -> Result<T>) -> Result<T> {
        let i = self.libs.iter().position(|(i, _)| i == id).ok_or_else(|| EngineError::Other(format!("no library `{id}`")))?;
        let before = self.libs.get(i).map(|(_, l)| l.clone()).unwrap_or_default();
        let r = match self.libs.get_mut(i) {
            Some((_, lib)) => f(lib)?,
            None => return Err(EngineError::Other(format!("no library `{id}`"))),
        };
        if let Err(e) = self.save(id) {
            if let Some((_, lib)) = self.libs.get_mut(i) {
                *lib = before;
            }
            return Err(e);
        }
        self.sort();
        Ok(r)
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
    /// At most [`MAX_ITEMS`] of each kind (a hand-edited file may hold more).
    fn cap(&mut self) {
        self.colors.truncate(MAX_ITEMS);
        self.char_styles.truncate(MAX_ITEMS);
        self.para_styles.truncate(MAX_ITEMS);
        self.graphics.truncate(MAX_ITEMS);
    }
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

fn stem_of(path: &str) -> String {
    std::path::Path::new(path).file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned())
}

/// The kinds of library item.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Graphic,
    FillColor,
    StrokeColor,
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
            Some("charStyle") => Kind::CharStyle,
            Some("paraStyle") => Kind::ParaStyle,
            _ => return Err(bad(cmd, "kind must be graphic, fillColor, strokeColor, charStyle or paraStyle")),
        })
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "library.list",
            "Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, graphics, colors, charStyles, paraStyles} (counts)], current: the current library's id or null, folder: the library folder, or null when libraries last for the session only (the web, headless)}",
            always,
            list
        ),
        cmd!(
            query "library.get",
            "Library",
            [],
            None,
            "{library?: id or name (default: the current library)} → {id, name, graphics: [{id, name, width, height, thumbnail: PNG base64}], colors: [{name, hex, color}], charStyles: [{name, attrs}], paraStyles: [{name, attrs}]}",
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
            "{library?: id or name (default: the current library, made when there is none), kind: graphic|fillColor|strokeColor|charStyle|paraStyle, ids?: objects (default: the selection), name?} add an item from the selection: graphic: a copy of the objects with every resource they use; fillColor/strokeColor: the first selected object's solid fill/stroke colour (else the default fill/stroke); charStyle/paraStyle: the selected text's character/paragraph attributes (the Type tool's range, else the first selected text object). An item the library already has (same kind and contents) isn't added again → {library, kind, name, id?: graphic id, existing: bool}",
            has_doc,
            add
        ),
        cmd!(
            "library.use",
            "Use Library Item",
            [],
            None,
            "{library?, kind, item: name (graphic: id or name), center?: [x, y] (graphic; default: the first artboard's centre), to?: fill|stroke (colours; default: the kind's)} graphic: place a copy centred on `center`, its resources pasted along, selected, one undo step; colours: paint the selection (and the defaults) as paint.setFill/paint.setStroke; charStyle/paraStyle: add the style to the document (a name taken by different attributes gets a number) and apply it to the selected text, one undo step → {kind, ids?, style?}",
            has_doc,
            use_item
        ),
        cmd!("library.removeItem", "Delete Library Item", [], None, "{library?, kind, item: name (graphic: id or name)} → null", always, remove_item),
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
        "charStyles": lib.char_styles.len(),
        "paraStyles": lib.para_styles.len(),
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
        "charStyles": lib.char_styles.iter().map(|t| json!({"name": t.name, "attrs": t.attrs})).collect::<Vec<_>>(),
        "paraStyles": lib.para_styles.iter().map(|t| json!({"name": t.name, "attrs": t.attrs})).collect::<Vec<_>>(),
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

/// The library `p` names, else the current one, else a new "My Library".
fn target_library(s: &mut Session, p: &Value, cmd: &str) -> Result<String> {
    if p.get("library").is_some_and(|v| !v.is_null()) || s.libraries.current().is_some() {
        return library_id(s, p, cmd);
    }
    s.libraries.create("My Library")
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
        Kind::FillColor | Kind::StrokeColor => Item::Color(color_of(s, p, kind == Kind::StrokeColor, C)?),
        Kind::CharStyle | Kind::ParaStyle => {
            let para = kind == Kind::ParaStyle;
            let attrs = super::textstyles::selected_style_attrs(s, para)?.ok_or_else(|| bad(C, "select some type first"))?;
            Item::Style(para, attrs)
        }
    };
    let id = target_library(s, p, C)?;
    let kind_name = str_param(p, "kind").unwrap_or_default().to_string();
    let r = s.libraries.change(&id, |lib| {
        let full = |n: usize| if n >= MAX_ITEMS { Err(bad(C, format!("the library holds {MAX_ITEMS} items of this kind already"))) } else { Ok(()) };
        Ok(match item {
            Item::Graphic(mut g) => {
                full(lib.graphics.len())?;
                if let Some(old) = lib.graphics.iter().find(|o| o.data == g.data) {
                    return Ok(json!({ "name": old.name, "id": old.id, "existing": true }));
                }
                let base = given.clone().unwrap_or_else(|| g.name.clone());
                g.name = unique(&base, |n| lib.graphics.iter().any(|o| o.name == n));
                g.id = unique("graphic", |n| lib.graphics.iter().any(|o| o.id == n)).replace(' ', "-");
                let v = json!({ "name": g.name, "id": g.id, "existing": false });
                lib.graphics.push(g);
                v
            }
            Item::Color((name, color)) => {
                full(lib.colors.len())?;
                if let Some(old) = lib.colors.iter().find(|o| o.color == color) {
                    return Ok(json!({ "name": old.name, "existing": true }));
                }
                let name = unique(&given.clone().unwrap_or(name), |n| lib.colors.iter().any(|o| o.name == n));
                lib.colors.push(LibraryColor { name: name.clone(), color });
                json!({ "name": name, "existing": false })
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
                json!({ "name": name, "existing": false })
            }
        })
    })?;
    let mut r = r;
    r["library"] = json!(id);
    r["kind"] = json!(kind_name);
    Ok(r)
}

enum Item {
    Graphic(LibraryGraphic),
    Color((String, Color)),
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
    let st = s.doc()?;
    let ids = match ids_param(p, "ids") {
        Some(ids) => ids,
        None => st.selection.objects.clone(),
    };
    let from_art = ids.iter().filter_map(|id| st.doc.node(*id)).find_map(|n| {
        let paint = if stroke { n.appearance.stroke().map(|x| &x.paint) } else { n.appearance.fill().map(|f| &f.paint) };
        paint.cloned()
    });
    let paint = match from_art {
        Some(p) => p,
        None => {
            let d = &s.paint;
            if stroke { d.stroke.clone() } else { d.fill.clone() }
        }
    };
    match paint {
        Paint::Solid { color, swatch, .. } => {
            let name = swatch.filter(|n| !n.is_empty()).unwrap_or_else(|| color.to_hex());
            Ok((name, color))
        }
        Paint::None => Err(bad(cmd, if stroke { "the selection has no stroke colour" } else { "the selection has no fill colour" })),
        _ => Err(bad(cmd, "only solid colours can be added (not gradients or patterns)")),
    }
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
            let stroke = match str_param(p, "to") {
                Some("fill") => false,
                Some("stroke") => true,
                Some(_) => return Err(bad(C, "to must be fill or stroke")),
                None => kind == Kind::StrokeColor,
            };
            let color = data.get("color").cloned().ok_or_else(|| bad(C, "no colour"))?;
            s.execute(if stroke { "paint.setStroke" } else { "paint.setFill" }, &json!({ "color": color }))?;
            Ok(json!({ "kind": if stroke { "strokeColor" } else { "fillColor" } }))
        }
        Kind::CharStyle | Kind::ParaStyle => {
            let para = kind == Kind::ParaStyle;
            let name = data.get("name").and_then(Value::as_str).unwrap_or("Style").to_string();
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

/// Run `f`'s commands as one undo step (undone together when it fails), unless an undo group is
/// already open (its own step then holds them).
fn in_one_step<T>(s: &mut Session, f: impl FnOnce(&mut Session) -> Result<T>) -> Result<T> {
    let open = s.active().is_some_and(|st| st.undo_group.is_some());
    if !open {
        s.begin_undo_group();
    }
    let r = f(s);
    if !open {
        s.end_undo_group(r.is_err());
    }
    r
}

fn remove_item(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "library.removeItem";
    let (id, kind, item) = item_param(s, p, C)?;
    let item = item.to_string();
    s.libraries.change(&id, |lib| {
        let before = (lib.graphics.len(), lib.colors.len(), lib.char_styles.len(), lib.para_styles.len());
        match kind {
            Kind::Graphic => {
                let at = lib.graphics.iter().position(|g| g.id == item).or_else(|| lib.graphics.iter().position(|g| g.name == item));
                if let Some(i) = at {
                    lib.graphics.remove(i);
                }
            }
            Kind::FillColor | Kind::StrokeColor => lib.colors.retain(|c| c.name != item),
            Kind::CharStyle => lib.char_styles.retain(|t| t.name != item),
            Kind::ParaStyle => lib.para_styles.retain(|t| t.name != item),
        }
        if before == (lib.graphics.len(), lib.colors.len(), lib.char_styles.len(), lib.para_styles.len()) {
            return Err(bad(C, format!("the library has no item `{item}`")));
        }
        Ok(())
    })?;
    ok()
}
