//! Symbol libraries (Window → Symbol Libraries, the Symbols panel's library button): read-only sets
//! of symbols the library panel shows. A library is a document holding the symbols with the
//! resources they use (nested symbols, patterns, swatches, brushes, graphic styles, images) and
//! their Symbol Options; saved, it is a native `.vectorcraft` file, which File › Open also opens.
//! The built-in library is generated here ([`BUILTIN`]); User Defined ones are the `.vectorcraft`
//! files in the user library folder ([`Libraries`]), and Other Library… loads the symbols of any
//! document VectorCraft opens. `symbol.addFromLibrary` copies symbols into the document;
//! `symbol.saveLibrary` writes the document's.

use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use super::brushsym::{CURRENT_SYMBOL, natural_size, normalize, place_xf, set_natural_size};
use super::clipboard::Clipboard;
use super::clipboard::resources::SwatchChoices;
use super::swatch::str_list;
use super::swatchlib::{FileData, LibraryFile, LibraryInfo, stem};
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, has_doc, number_or, str_param};

/// The extension of saved symbol libraries: native documents.
pub const SYMBOLS_EXT: &str = vectorcraft_format::EXTENSION;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "symbol.libraries",
            "Symbol Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"user\" (User Defined: the .vectorcraft files in the user library folder, rescanned now)|\"loaded\" (symbol.loadLibrary), count, path?}], userFolder} every symbol library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "symbol.library",
            "Symbol Library",
            [],
            None,
            "{library: id or name} → {id, name, category, symbols: [{name, size: [w, h], registration: 0..8, exportType}] (as symbol.list)}",
            always,
            get
        ),
        cmd!(
            "symbol.addFromLibrary",
            "Add to Symbols",
            ["Window", "Symbol Libraries"],
            None,
            "{library: id or name, name? | names?: [symbol names in the library] (default: all of it), place?: false (also place an instance of the first one, its registration point at x, y: default the first artboard's centre), x?, y?, swatchConflict? (as edit.paste)} copy library symbols into the document's Symbols as one undo step (with the place), with their Symbol Options and the resources they use (the symbols they nest, patterns, swatches, brushes, graphic styles, images). A symbol the document already has (same name and art) isn't added again; a name another symbol has gets a number (\"Star 2\"). The first one becomes the current symbol → {library, names: [each symbol's name in the document], added: [names added], existing: [names already there], placed?: id}",
            has_doc,
            add
        ),
        cmd!(
            "symbol.saveLibrary",
            "Save Symbol Library…",
            ["Window", "Symbols"],
            None,
            "{path?, names?: [symbol names] (default: all), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's symbols, with their Symbol Options and the resources they use, as a symbol library: a .vectorcraft document holding only them → {path, count, library?: id when saved to the user folder}; without path or user → {dataBase64: the file's bytes, count}",
            has_doc,
            save
        ),
        cmd!(
            "symbol.loadLibrary",
            "Other Library…",
            ["Window", "Symbol Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load the symbols of any document VectorCraft opens (see document.formats), a saved symbol library included, for the library panel (Window → Symbol Libraries lists it until the app quits; a file of the user library folder, or a file with the same bytes as one there, opens as that User Defined library) → {library: id, name, count}",
            always,
            load
        ),
    ]
}

/// A symbol library: its name and a document holding its symbols (in panel order) with the
/// resources they use and their Symbol Options; the document has no art of its own.
#[derive(Clone, Debug)]
pub struct SymbolLibrary {
    pub name: String,
    pub doc: Document,
}

impl Default for SymbolLibrary {
    fn default() -> Self {
        Self { name: String::new(), doc: Document::new(100.0, 100.0) }
    }
}

impl SymbolLibrary {
    pub fn len(&self) -> usize {
        self.doc.symbols.len()
    }
    pub fn is_empty(&self) -> bool {
        self.doc.symbols.is_empty()
    }
    pub fn symbol(&self, name: &str) -> Option<&Symbol> {
        self.doc.symbols.iter().find(|s| s.name == name)
    }

    /// Document `src`'s symbols `names` (empty: all of them, in panel order) as a library named
    /// `name`: the symbols, the symbols they nest and every other resource they use, with their
    /// Symbol Options.
    pub fn from_document(src: &Document, names: &[String], name: String) -> std::result::Result<Self, String> {
        if let Some(n) = names.iter().find(|n| !src.symbols.iter().any(|s| s.name == **n)) {
            return Err(format!("no symbol `{n}`"));
        }
        let picked = src.symbols.iter().filter(|s| names.is_empty() || names.contains(&s.name));
        let clip = Clipboard::gather(src, picked.map(|s| instance(&s.name)).collect());
        let mut doc = clip.to_document();
        for l in &mut doc.layers {
            if let Some(ch) = Arc::make_mut(l).children_mut() {
                ch.clear();
            }
        }
        // The panel order of the document they came from.
        let order = |n: &str| src.symbols.iter().position(|s| s.name == n);
        doc.symbols.sort_by_key(|s| order(&s.name));
        for s in &doc.symbols.clone() {
            copy_options(src, &s.name, &mut doc, &s.name);
        }
        doc.title = name.clone();
        Ok(Self { name, doc })
    }
}

impl LibraryFile for SymbolLibrary {
    const EXTS: &[&str] = &[SYMBOLS_EXT];
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        let doc = vectorcraft_format::load(bytes).map_err(|e| e.to_string())?;
        SymbolLibrary::from_document(&doc, &[], stem.to_string())
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// User Defined and loaded symbol libraries.
pub type Libraries = super::swatchlib::Libraries<SymbolLibrary>;

/// A library generated in code.
pub struct BuiltinSymbolLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn() -> SymbolLibrary,
}

/// The built-in symbol libraries, in menu order: original art generated in code.
pub const BUILTIN: &[BuiltinSymbolLibrary] = &[BuiltinSymbolLibrary { id: "basic-shapes", name: "Basic Shapes", make: basic_shapes }];

/// Built-in library `id`, made once.
fn builtin_library(id: &str) -> Option<Arc<SymbolLibrary>> {
    static MADE: LazyLock<Vec<Arc<SymbolLibrary>>> = LazyLock::new(|| BUILTIN.iter().map(|b| Arc::new((b.make)())).collect());
    BUILTIN.iter().position(|b| b.id == id).and_then(|i| MADE.get(i).cloned())
}

/// Basic Shapes: plain filled and outlined shapes, 40 pt (the arrow 48 × 32 pt).
fn basic_shapes() -> SymbolLibrary {
    let square = Rect::new(-20.0, -20.0, 20.0, 20.0);
    let c = Point::ZERO;
    let arrow = [(-24.0, -6.0), (4.0, -6.0), (4.0, -16.0), (24.0, 0.0), (4.0, 16.0), (4.0, 6.0), (-24.0, 6.0)];
    let arrow: Vec<Point> = arrow.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let shapes = [
        ("Circle", shapes::ellipse(square), "#2f80c9"),
        ("Square", shapes::rectangle(square), "#d9534f"),
        ("Rounded Square", shapes::rounded_rectangle(square, 8.0), "#f0ad4e"),
        ("Triangle", shapes::polygon(c, 22.0, 3, 0.0), "#5cb85c"),
        ("Diamond", shapes::polygon(c, 20.0, 4, 0.0), "#8e5bc6"),
        ("Hexagon", shapes::polygon(c, 20.0, 6, 0.0), "#1fa39b"),
        ("Star", shapes::star(c, 21.0, 9.0, 5, 0.0), "#f2c233"),
        ("Burst", shapes::star(c, 21.0, 15.0, 12, 0.0), "#e8743b"),
        ("Arrow", vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&arrow, true)), "#4a5a6a"),
    ];
    let mut doc = Document::new(100.0, 100.0);
    for (name, path, hex) in shapes {
        let fill = Color::from_hex(hex).unwrap_or(Color::BLACK);
        // The outline: the fill, darkened.
        let edge = fill.lerp(&Color::BLACK, 0.4);
        let id = doc.alloc_id();
        let art = Node::path(id, path, Appearance::basic(Paint::solid(fill), Paint::solid(edge), 1.0));
        let Ok((art, size, _)) = normalize(art) else { continue };
        doc.symbols.push(Symbol { name: name.into(), art: Arc::new(art) });
        set_natural_size(&mut doc, name, Some(size));
    }
    doc.title = "Basic Shapes".into();
    SymbolLibrary { name: "Basic Shapes".into(), doc }
}

/// An instance of symbol `name` (what the resource gathering follows).
fn instance(name: &str) -> Node {
    Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY })
}

/// Symbol `from`'s Symbol Options in `src` (natural size, registration, export type, 9-slice
/// guides) given to symbol `to` of `d`.
fn copy_options(src: &Document, from: &str, d: &mut Document, to: &str) {
    set_natural_size(d, to, Some(natural_size(src, from)));
    d.set_symbol_registration(to, src.symbol_registration(from));
    d.set_symbol_export_type(to, src.symbol_export_type(from));
    d.set_symbol_nine_slice(to, src.symbol_nine_slice(from));
}

fn builtin_info(b: &BuiltinSymbolLibrary) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category: "builtIn" }
}

/// Every symbol library, in menu order: built-in, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    BUILTIN.iter().map(builtin_info).chain(s.symbol_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<SymbolLibrary>)> {
    if let Some(found) = s.symbol_libraries.get(key) {
        return Some(found);
    }
    if let Some(b) = BUILTIN.iter().find(|b| b.id == key) {
        return Some((builtin_info(b), builtin_library(key)?));
    }
    let id = libraries(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    library(s, &id)
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<SymbolLibrary>)> {
    let key = str_param(p, "library").ok_or_else(|| bad(cmd, "missing `library` (see symbol.libraries)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no symbol library `{key}` (see symbol.libraries)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.symbol_libraries.rescan();
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            json!({"id": l.id, "name": l.name, "category": l.category, "count": count, "path": s.symbol_libraries.path(&l.id)})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.symbol_libraries.user_dir() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "symbol.library")?;
    let d = &lib.doc;
    let symbols: Vec<Value> = d
        .symbols
        .iter()
        .map(|sym| {
            let (w, h) = natural_size(d, &sym.name);
            json!({"name": sym.name, "size": [w, h], "registration": d.symbol_registration(&sym.name).0, "exportType": d.symbol_export_type(&sym.name).id()})
        })
        .collect();
    Ok(json!({"id": info.id, "name": info.name, "category": info.category, "symbols": symbols}))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.addFromLibrary";
    let (info, lib) = library_param(s, p, C)?;
    let mut names = str_list(p, "names");
    names.extend(str_param(p, "name").map(str::to_string));
    if let Some(n) = names.iter().find(|n| lib.symbol(n).is_none()) {
        return Err(bad(C, format!("no symbol `{n}` in `{}`", lib.name)));
    }
    let mut picked: Vec<String> = if names.is_empty() { lib.doc.symbols.iter().map(|s| s.name.clone()).collect() } else { names };
    // Each once, in the order asked for.
    let mut seen = HashSet::new();
    picked.retain(|n| seen.insert(n.clone()));
    let place = bool_or(p, "place", false);
    if place && picked.is_empty() {
        return Err(bad(C, format!("`{}` has no symbols to place", lib.name)));
    }
    let choices = SwatchChoices::parse(C, p)?;
    let clip = Clipboard::gather(&lib.doc, picked.iter().map(|n| instance(n)).collect());
    let st = s.doc()?;
    let before: HashSet<String> = st.doc.symbols.iter().map(|s| s.name.clone()).collect();
    // A library symbol's name in the document: its own, or the one the import gave it.
    let target = |renamed: &[Value], n: &str| -> String {
        let to = renamed.iter().find(|r| r["kind"] == "symbol" && r["from"] == n).and_then(|r| r["to"].as_str());
        to.unwrap_or(n).to_string()
    };
    // Where the symbols land, worked out on a copy: nothing to add and nothing to place is no edit.
    let mut probe = (*st.doc).clone();
    let imported = clip.import_into(&mut probe, &choices, false);
    let doc_names: Vec<String> = picked.iter().map(|n| target(&imported.renamed, n)).collect();
    let centre = st.doc.artboards.first().map(|a| a.rect.center()).unwrap_or(Point::ZERO);
    let at = if place { Some(Point::new(number_or(p, "x", centre.x, C)?, number_or(p, "y", centre.y, C)?)) } else { None };
    let parent = if place { st.target_parent()? } else { None };
    let mut out = json!({ "library": info.id });
    if imported.added > 0 || place {
        let placed = s.edit("Add to Symbols", |d, sel| {
            let imported = clip.import_into(d, &choices, false);
            // Every symbol brought in (nested ones too) takes its Symbol Options along.
            for sym in &clip.symbols {
                let to = target(&imported.renamed, &sym.name);
                if !before.contains(&to) {
                    copy_options(&lib.doc, &sym.name, d, &to);
                }
            }
            let Some(first) = doc_names.first() else { return Ok(None) };
            d.unknown.insert(CURRENT_SYMBOL.into(), json!(first));
            let Some(at) = at else { return Ok(None) };
            let xf = place_xf(at, natural_size(d, first), d.symbol_registration(first));
            let id = d.alloc_id();
            d.insert(parent, usize::MAX, Node::new(id, NodeKind::SymbolInstance { symbol: first.clone(), xf }))?;
            sel.set([id]);
            Ok(Some(id))
        })?;
        if let Some(id) = placed {
            out["placed"] = json!(id.0);
        }
    } else if let Some(first) = doc_names.first() {
        s.execute("symbol.setCurrent", &json!({ "name": first }))?;
    }
    let (added, existing): (Vec<&String>, Vec<&String>) = doc_names.iter().partition(|n| !before.contains(*n));
    out["names"] = json!(doc_names);
    out["added"] = json!(added);
    out["existing"] = json!(existing);
    Ok(out)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.saveLibrary";
    let st = s.doc()?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = SymbolLibrary::from_document(&st.doc, &str_list(p, "names"), name).map_err(|e| bad(C, e))?;
    if lib.is_empty() {
        return Err(bad(C, "the document has no symbols to save"));
    }
    let mut out = json!({ "count": lib.len() });
    let bytes = vectorcraft_format::save_file(&lib.doc);
    s.symbol_libraries.write(p, &lib.name, SYMBOLS_EXT, FileData::Binary(bytes), &mut out, C)?;
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "symbol.loadLibrary";
    let (info, lib) = s.symbol_libraries.load(p, C, |bytes, file| {
        let loaded = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?;
        if loaded.preview_only {
            return Err(bad(C, "only this Affinity file's embedded preview could be read; use File › Open to see it with its warning"));
        }
        SymbolLibrary::from_document(&loaded.doc, &[], stem(file).to_string()).map_err(|e| bad(C, e))
    })?;
    Ok(json!({"library": info.id, "name": info.name, "count": lib.len()}))
}
