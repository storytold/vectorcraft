//! Graphic style libraries (Window → Graphic Style Libraries, the Graphic Styles panel's library
//! button): read-only sets of graphic styles the library panel shows. The built-in libraries are
//! generated in [`vectorcraft_doc::style_libs`]; User Defined ones are the `.vcstyles` files in the
//! user library folder ([`Libraries`]), and Other Library… loads more from files.
//! `graphicStyle.addFromLibrary` copies styles into the document; `graphicStyle.saveLibrary`
//! writes the document's.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::style_libs::{self, STYLE_LIBRARIES, STYLES_EXT, builtin_style_library};
use vectorcraft_doc::{Document, GraphicStyle, PatternDef, StyleLibrary};

use super::menucmds::squash;
use super::swatch::str_list;
use super::swatchlib::{LibraryFile, LibraryInfo, stem};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "graphicStyle.libraries",
            "Graphic Style Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"user\" (User Defined: the .vcstyles files in the user library folder, rescanned now)|\"loaded\" (graphicStyle.loadLibrary), count, path?}], userFolder} every graphic style library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "graphicStyle.library",
            "Graphic Style Library",
            [],
            None,
            "{library: id or name} → {id, name, category, styles: [{name, fill, stroke, strokeWidth, fills, strokes, effects: [effect id…], opacity, blend, isolate, knockout}] (as graphicStyle.list)}",
            always,
            get
        ),
        cmd!(
            "graphicStyle.addFromLibrary",
            "Add to Graphic Styles",
            ["Window", "Graphic Style Libraries"],
            None,
            "{library: id or name, name? | names?: [style names in the library] (default: all of it), apply?: false (also apply the first one to `ids` or the selection, if any, as graphicStyle.apply), add?: false (with apply: add its appearance on top, as Alt-click), ids?} copy library styles into the document's Graphic Styles as one undo step (with the apply). A style the document already has (same name and look) isn't added again; a name taken by another style gets a number (\"Halo 2\"). The patterns they paint with come along (a name taken by another pattern or swatch gets a number) → {library, added: [names in the document], existing: [names already there], applied?: name}",
            has_doc,
            add
        ),
        cmd!(
            "graphicStyle.saveLibrary",
            "Save Graphic Style Library…",
            ["Window", "Graphic Styles"],
            None,
            "{path?, names?: [style names] (default: all), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's graphic styles as a .vcstyles library (JSON: the styles, unlinked from swatches, and the patterns they paint with) → {path, count, library?: id when saved to the user folder}; without path or user → {data: the file's text, count}",
            has_doc,
            save
        ),
        cmd!(
            "graphicStyle.loadLibrary",
            "Other Library…",
            ["Window", "Graphic Style Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load a .vcstyles library, or the graphic styles of any document Vector W3K2 opens (see document.formats), for the library panel (Window → Graphic Style Libraries lists it until the app quits) → {library: id, name, count}",
            always,
            load
        ),
    ]
}

impl LibraryFile for StyleLibrary {
    const EXTS: &'static [&'static str] = &[STYLES_EXT];
    fn read(text: &str, stem: &str) -> std::result::Result<Self, String> {
        style_libs::read(text, stem)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// User Defined and loaded graphic style libraries.
pub type Libraries = super::swatchlib::Libraries<StyleLibrary>;

fn builtin_info(b: &style_libs::BuiltinStyleLibrary) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category: "builtIn" }
}

/// Every graphic style library, in menu order: built-in, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    STYLE_LIBRARIES.iter().map(builtin_info).chain(s.style_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<StyleLibrary>)> {
    if let Some(found) = s.style_libraries.get(key) {
        return Some(found);
    }
    if let Some(b) = STYLE_LIBRARIES.iter().find(|b| b.id == key) {
        return Some((builtin_info(b), builtin_style_library(key)?));
    }
    let id = libraries(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    library(s, &id)
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<StyleLibrary>)> {
    let key = str_param(p, "library").ok_or_else(|| bad(cmd, "missing `library` (see graphicStyle.libraries)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no graphic style library `{key}` (see graphicStyle.libraries)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.style_libraries.rescan();
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            json!({"id": l.id, "name": l.name, "category": l.category, "count": count, "path": s.style_libraries.path(&l.id)})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.style_libraries.user_dir() }))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "graphicStyle.library")?;
    let styles: Vec<Value> = lib.styles.iter().map(super::style::style_json).collect();
    Ok(json!({"id": info.id, "name": info.name, "category": info.category, "styles": styles}))
}

/// Pattern `name` of `lib` in `d`: the document's own when it has the same pattern (under any
/// name), else the library's added (under a free name). Returns the name it has in `d`.
fn bring_pattern(d: &mut Document, lib: &StyleLibrary, name: &str) -> String {
    let Some(def) = lib.patterns.iter().find(|p| p.name == name) else { return name.to_string() };
    if let Some(same) = d.patterns.iter().find(|p| PatternDef { name: def.name.clone(), ..(*p).clone() } == *def) {
        return same.name.clone();
    }
    let new = unique_name(name, |n| d.pattern(n).is_some() || d.swatch_name_taken(n));
    super::patterncmds::add_pattern(d, PatternDef { name: new.clone(), ..def.clone() });
    new
}

/// Library style `g` as a document style: its patterns brought along (`renamed`: library pattern
/// name → its name in `d`, filled in as they come) and a free name and id.
fn adopt(d: &mut Document, lib: &StyleLibrary, g: &GraphicStyle, renamed: &mut HashMap<String, String>) -> GraphicStyle {
    let mut g = g.clone();
    for it in &mut g.appearance.items {
        if let Paint::Pattern { pattern, .. } = it.paint_mut() {
            *pattern = renamed.entry(pattern.clone()).or_insert_with_key(|n| bring_pattern(d, lib, n)).clone();
        }
    }
    g.name = unique_name(&g.name, |n| d.graphic_style(n).is_some());
    g.id = d.next_graphic_style_id();
    g
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graphicStyle.addFromLibrary";
    let (info, lib) = library_param(s, p, C)?;
    let mut names = str_list(p, "names");
    names.extend(str_param(p, "name").map(str::to_string));
    if let Some(n) = names.iter().find(|n| lib.style(n).is_none()) {
        return Err(bad(C, format!("no graphic style `{n}` in `{}`", lib.name)));
    }
    let mut picked: Vec<&GraphicStyle> =
        if names.is_empty() { lib.styles.iter().collect() } else { names.iter().filter_map(|n| lib.style(n)).collect() };
    // Each once, in the order asked for.
    let mut seen = HashSet::new();
    picked.retain(|g| seen.insert(g.name.as_str()));
    // Styles the document already has (same name and look) are reused, not added again.
    let d = &s.doc()?.doc;
    let (existing, fresh): (Vec<&GraphicStyle>, Vec<&GraphicStyle>) =
        picked.iter().partition(|g| d.graphic_style(&g.name).is_some_and(|x| x.same_look(g)));
    let undo_before = s.doc()?.history.undo.len();
    // Library name → the name it got in the document.
    let added: Vec<(String, String)> = if fresh.is_empty() {
        vec![]
    } else {
        s.edit("Add to Graphic Styles", |d, _| {
            let mut renamed = HashMap::new();
            let mut out = vec![];
            for g in fresh {
                let new = adopt(d, &lib, g, &mut renamed);
                out.push((g.name.clone(), new.name.clone()));
                d.graphic_styles.push(new);
            }
            Ok(out)
        })?
    };
    let doc_name = |lib_name: &str| added.iter().find(|(l, _)| l == lib_name).map_or_else(|| lib_name.to_string(), |(_, n)| n.clone());
    let mut out = json!({
        "library": info.id,
        "added": added.iter().map(|(_, n)| n).collect::<Vec<_>>(),
        "existing": existing.iter().map(|g| &g.name).collect::<Vec<_>>(),
    });
    let first = picked.first().map(|g| doc_name(&g.name));
    let targets = p.get("ids").is_some() || !super::appearance::appearance_targets(s, &json!({}))?.is_empty();
    if let Some(name) = first.filter(|_| bool_or(p, "apply", false) && targets) {
        let mut params = json!({"name": name, "add": bool_or(p, "add", false)});
        if let Some(ids) = p.get("ids") {
            params["ids"] = ids.clone();
        }
        s.execute("graphicStyle.apply", &params)?;
        // Adding and applying undo together.
        squash(s, undo_before, "Add to Graphic Styles");
        out["applied"] = json!(name);
    }
    Ok(out)
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graphicStyle.saveLibrary";
    let st = s.doc()?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = StyleLibrary::from_document(&st.doc, &str_list(p, "names"), name).map_err(|e| bad(C, e))?;
    let mut out = json!({ "count": lib.len() });
    s.style_libraries.write(p, &lib.name, STYLES_EXT, style_libs::write(&lib), &mut out, C)?;
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "graphicStyle.loadLibrary";
    let (info, lib) = s.style_libraries.load(p, C, |bytes, file| {
        // A library file, or a document whose graphic styles become the library.
        match std::str::from_utf8(bytes).ok().filter(|t| style_libs::sniff(t)) {
            Some(t) => style_libs::read(t, stem(file)).map_err(|e| bad(C, e)),
            None => {
                let doc = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?.doc;
                StyleLibrary::from_document(&doc, &[], stem(file).to_string()).map_err(|e| bad(C, e))
            }
        }
    })?;
    Ok(json!({"library": info.id, "name": info.name, "count": lib.len()}))
}
