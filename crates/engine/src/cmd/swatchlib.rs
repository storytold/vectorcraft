//! Swatch libraries (Window → Swatch Libraries, the Swatches panel's library button): read-only
//! sets of swatches the library panel shows. The built-in libraries are computed in
//! [`vectorcraft_color::libraries`]; User Defined ones are the library files in the user library
//! folder ([`Libraries`]), and Other Library… loads more from files. `swatch.library.add` copies
//! swatches into the document; `swatch.library.save` writes the document's ([`palette_io`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::libraries::{BuiltinLibrary, GRADIENT_LIBRARIES, SWATCH_LIBRARIES, builtin_library};
use vectorcraft_color::palette_io::{self, PaletteFormat};
use vectorcraft_color::recolor::Palette;
use vectorcraft_color::{Paint, Swatch, SwatchGroup, SwatchLibrary, default_swatches};

use super::fileio::{create_dir, read_file, write_file};
use super::menucmds::squash;
use super::swatch::{map_default_paints, str_list, swatch_json};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "swatch.library.list",
            "Swatch Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"gradients\"|\"user\" (User Defined: the library files in the user library folder, rescanned now)|\"loaded\" (swatch.library.load), count, path?}], userFolder} every library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "swatch.library.get",
            "Swatch Library",
            [],
            None,
            "{library: id or name} → {id, name, category, swatches: [{name, kind, group, global, spot, color?, hex?, gradient?}] (as swatch.list), groups: [{name, swatches: [names]}]}",
            always,
            get
        ),
        cmd!(
            "swatch.library.add",
            "Add to Swatches",
            ["Window", "Swatch Libraries"],
            None,
            "{library: id or name, names?: [swatch or colour group names in the library] (default: all of it; a group comes as a colour group, a single swatch ungrouped), apply?: \"fill\"|\"stroke\" (also apply the first one to the selection and the defaults, as paint.setFill / paint.setStroke), focus?: true (with apply: false keeps the active proxy)} copy library swatches into the document as one undo step. A swatch the document already has (same name and paint) isn't added again; a name taken by another swatch gets a number (\"Clay 1 2\") → {library, added: [names in the document], existing: [names already there], applied?: name}",
            has_doc,
            add
        ),
        cmd!(
            "swatch.resetDefaults",
            "Default Swatches",
            ["Window", "Swatch Libraries"],
            None,
            "{replace?: false} bring back the default swatches and colour groups of the document's colour mode that are missing (by name), as one undo step; replace: true makes the swatches exactly the defaults (art linked to a removed global swatch keeps its colour, unlinked) → {added: [names]}",
            has_doc,
            reset_defaults
        ),
        cmd!(
            "swatch.library.save",
            "Save Swatch Library…",
            ["Window", "Swatches"],
            None,
            "{path?, format?: \"vcswatches\" (lossless JSON: colour models, global, spot, gradients, groups) | \"gpl\" (8-bit RGB palette; groups as `# Group:` comments) | \"css\" (custom properties on :root) (default: the path's extension, else vcswatches), names?: [swatch or colour group names] (default: all; None and patterns are never saved), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's swatches as a library → {path, format, count, library?: id when saved to the user folder}; without path or user → {data: the file's text, format, count}",
            has_doc,
            save
        ),
        cmd!(
            "swatch.library.load",
            "Other Library…",
            ["Window", "Swatch Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load a swatch library (.vcswatches, .gpl, or the .ase Swatch Exchange, .acb colour book and .aco palette files other design apps write and install, spot colour books included), or the swatches of any document VectorCraft opens (see document.formats), for the library panel (Window → Swatch Libraries lists it until the app quits) → {library: id, name, count}",
            always,
            load
        ),
    ]
}

/// The libraries beyond the built-in ones: User Defined (the library files in the user library
/// folder, as last scanned) and loaded ones (`swatch.library.load`). Graphic style libraries use
/// the same with [`vectorcraft_doc::StyleLibrary`] ([`super::stylelib`]).
#[derive(Clone, Debug, Default)]
pub struct Libraries<L = SwatchLibrary> {
    /// The user library folder. The desktop app sets it; without one (the web, headless sessions)
    /// there are no User Defined libraries.
    user_dir: Option<String>,
    extra: Vec<Extra<L>>,
}

#[derive(Clone, Debug)]
struct Extra<L> {
    info: LibraryInfo,
    path: Option<String>,
    lib: Arc<L>,
}

/// A kind of library that lives in files ([`Libraries`]).
pub trait LibraryFile: Sized {
    /// The extensions of its files (lower case).
    const EXTS: &'static [&'static str];
    /// Read a library file whose name without the extension is `stem` (an unnamed library's name).
    fn read(text: &str, stem: &str) -> std::result::Result<Self, String>;
    /// [`Self::read`] from the file's bytes (binary formats override it).
    fn read_bytes(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        Self::read(&String::from_utf8_lossy(bytes), stem)
    }
    fn name(&self) -> &str;
}

impl LibraryFile for SwatchLibrary {
    const EXTS: &'static [&'static str] = LIBRARY_EXTS;
    fn read(text: &str, stem: &str) -> std::result::Result<Self, String> {
        palette_io::read(text, stem)
    }
    fn read_bytes(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        palette_io::read_bytes(bytes, stem)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// The extensions of library files [`palette_io::read_bytes`] reads: ours, GPL palettes, and the
/// Swatch Exchange (`.ase`), colour book (`.acb`) and colour palette (`.aco`) files other design
/// apps write and install (spot colour books included).
pub const LIBRARY_EXTS: &[&str] = &["vcswatches", "gpl", "ase", "acb", "aco"];

impl<L: LibraryFile> Libraries<L> {
    pub fn user_dir(&self) -> Option<&str> {
        self.user_dir.as_deref()
    }

    /// Set the user library folder and list its libraries.
    pub fn set_user_dir(&mut self, dir: Option<String>) {
        self.user_dir = dir;
        self.rescan();
    }

    /// Re-read the library files of the user library folder (unreadable ones are skipped).
    pub fn rescan(&mut self) {
        self.extra.retain(|e| e.info.category != "user");
        let Some(dir) = self.user_dir.clone() else { return };
        for path in library_files(&dir, L::EXTS) {
            let file = file_name(&path);
            let Some(lib) = read_file(&path).ok().and_then(|b| L::read_bytes(&b, stem(&file)).ok()) else { continue };
            let info = LibraryInfo { id: format!("user/{file}"), name: lib.name().to_string(), category: "user" };
            self.extra.push(Extra { info, path: Some(path), lib: Arc::new(lib) });
        }
    }

    /// The User Defined and loaded libraries, in menu order.
    pub fn infos(&self) -> impl Iterator<Item = &LibraryInfo> {
        self.extra.iter().map(|e| &e.info)
    }

    /// The User Defined or loaded library `id`.
    pub fn get(&self, id: &str) -> Option<(LibraryInfo, Arc<L>)> {
        self.extra.iter().find(|e| e.info.id == id).map(|e| (e.info.clone(), e.lib.clone()))
    }

    /// The file library `id` was read from.
    pub fn path(&self, id: &str) -> Option<&str> {
        self.extra.iter().find(|e| e.info.id == id).and_then(|e| e.path.as_deref())
    }

    /// Write library file `text` as the save commands do: into the user library folder as
    /// `name`.`ext` with `user: true` (→ `path`, and `library`: its id), to `path`, else back as
    /// `data`; the results go into `out`.
    pub(crate) fn write(&mut self, p: &Value, name: &str, ext: &str, text: String, out: &mut Value, cmd: &str) -> Result<()> {
        if bool_or(p, "user", false) {
            let dir = self
                .user_dir()
                .ok_or_else(|| bad(cmd, "no user library folder here (save with a path, or without one to get the data)"))?
                .to_string();
            // A file name from the library's name, without characters file systems reject.
            let base: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
            let file = format!("{}.{ext}", base.trim_matches(['.', ' ']));
            let path = std::path::Path::new(&dir).join(&file).to_string_lossy().to_string();
            create_dir(&dir)?;
            write_file(&path, text.as_bytes())?;
            self.rescan();
            out["path"] = json!(path);
            out["library"] = json!(format!("user/{file}"));
        } else if let Some(path) = str_param(p, "path") {
            write_file(path, text.as_bytes())?;
            out["path"] = json!(path);
        } else {
            out["data"] = json!(text);
        }
        Ok(())
    }

    /// Load a library from `p` (`path`, `data` or `dataBase64`; `name`: the file's name) as the
    /// load commands do, `parse(bytes, file name)` reading it. A file of the user library folder is
    /// its User Defined library; others are listed as loaded until the app quits.
    pub(crate) fn load(&mut self, p: &Value, cmd: &str, parse: impl FnOnce(&[u8], &str) -> Result<L>) -> Result<(LibraryInfo, Arc<L>)> {
        let path = str_param(p, "path");
        let bytes = match (path, str_param(p, "data"), str_param(p, "dataBase64")) {
            (Some(path), ..) => read_file(path)?,
            (None, Some(text), _) => text.as_bytes().to_vec(),
            (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?,
            _ => return Err(bad(cmd, "give `path`, `data` or `dataBase64`")),
        };
        let file = str_param(p, "name").map(str::to_string).or_else(|| path.map(file_name)).unwrap_or_else(|| "Library".into());
        let lib = parse(&bytes, &file)?;
        self.rescan();
        if let Some(e) = path.and_then(|p| self.extra.iter().find(|e| e.info.category == "user" && e.path.as_deref() == Some(p))) {
            return Ok((e.info.clone(), e.lib.clone()));
        }
        let info = LibraryInfo { id: format!("loaded/{}", path.unwrap_or(&file)), name: lib.name().to_string(), category: "loaded" };
        let lib = Arc::new(lib);
        self.extra.retain(|x| x.info.id != info.id);
        self.extra.push(Extra { info: info.clone(), path: path.map(str::to_string), lib: lib.clone() });
        Ok((info, lib))
    }
}

/// The library files with extensions `exts` in folder `dir`, sorted by name.
#[cfg(not(target_arch = "wasm32"))]
fn library_files(dir: &str, exts: &[&str]) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(dir) else { return vec![] };
    let mut files: Vec<String> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| exts.contains(&x.to_string_lossy().to_ascii_lowercase().as_str())))
        .map(|p| p.to_string_lossy().to_string())
        .collect();
    files.sort_by_key(|f| f.to_lowercase());
    files
}

#[cfg(target_arch = "wasm32")]
fn library_files(_: &str, _: &[&str]) -> Vec<String> {
    vec![]
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().to_string())
}

/// `name` without its extension.
pub(crate) fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(s, _)| s)
}

/// A library as the menus and the library panel list it.
#[derive(Clone, Debug, PartialEq)]
pub struct LibraryInfo {
    pub id: String,
    pub name: String,
    /// "builtIn", "gradients", "user" or "loaded".
    pub category: &'static str,
}

/// The built-in libraries with their categories.
fn builtins() -> impl Iterator<Item = (&'static str, &'static BuiltinLibrary)> {
    SWATCH_LIBRARIES.iter().map(|b| ("builtIn", b)).chain(GRADIENT_LIBRARIES.iter().map(|b| ("gradients", b)))
}

fn builtin_info((category, b): (&'static str, &BuiltinLibrary)) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category }
}

/// Every library, in menu order: built-in, gradients, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    builtins().map(builtin_info).chain(s.swatch_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<SwatchLibrary>)> {
    if let Some(found) = s.swatch_libraries.get(key) {
        return Some(found);
    }
    if let Some(b) = builtins().find(|(_, b)| b.id == key) {
        return Some((builtin_info(b), builtin_library(key)?));
    }
    let id = libraries(s).into_iter().find(|l| l.name.eq_ignore_ascii_case(key))?.id;
    library(s, &id)
}

/// The Limit to Library key for the active document's own swatches.
pub const DOCUMENT_SWATCHES: &str = "document";

/// The colours Limit to Library snaps to: the solid colours of library `key` (an id or name) or, for
/// [`DOCUMENT_SWATCHES`], of the active document's swatches and colour groups. `None` when there is
/// no such library (or document).
pub fn limit_palette(s: &Session, key: &str) -> Option<Palette> {
    let colors = |sw: &Swatch| sw.paint.color();
    if key == DOCUMENT_SWATCHES {
        return Some(Palette::new(s.active()?.doc.swatches_iter().filter_map(colors)));
    }
    Some(Palette::new(library(s, key)?.1.iter().filter_map(colors)))
}

/// The `limitTo` parameter's palette ([`limit_palette`]); `None` without one (or for "").
pub(crate) fn limit_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Palette>> {
    let Some(key) = str_param(p, "limitTo").filter(|k| !k.is_empty()) else { return Ok(None) };
    let palette = limit_palette(s, key)
        .ok_or_else(|| bad(cmd, format!("no swatch library `{key}` (see swatch.library.list; \"{DOCUMENT_SWATCHES}\": the document's swatches)")))?;
    if palette.is_empty() {
        return Err(bad(cmd, format!("`{key}` has no colours")));
    }
    Ok(Some(palette))
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<SwatchLibrary>)> {
    let key = str_param(p, "library").ok_or_else(|| bad(cmd, "missing `library` (see swatch.library.list)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no swatch library `{key}` (see swatch.library.list)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.swatch_libraries.rescan();
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            json!({"id": l.id, "name": l.name, "category": l.category, "count": count, "path": s.swatch_libraries.path(&l.id)})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.swatch_libraries.user_dir() }))
}

/// The document's swatches `names` (swatches or colour groups; empty: all) as a library named
/// `name`, without None and patterns.
fn document_library(d: &vectorcraft_doc::Document, names: &[String], name: String, cmd: &str) -> Result<SwatchLibrary> {
    if let Some(n) = names.iter().find(|n| !d.swatch_name_taken(n)) {
        return Err(bad(cmd, format!("no swatch or colour group `{n}`")));
    }
    let want = |n: &str| names.is_empty() || names.iter().any(|x| x == n);
    let savable = |w: &&Swatch| matches!(w.paint, Paint::Solid { .. } | Paint::Gradient(_));
    let swatches = d.swatches.iter().filter(savable).filter(|w| want(&w.name)).cloned().collect();
    let groups = d
        .swatch_groups
        .iter()
        .filter_map(|g| {
            let all = want(&g.name);
            let swatches: Vec<Swatch> = g.swatches.iter().filter(savable).filter(|w| all || want(&w.name)).cloned().collect();
            (!swatches.is_empty() || (all && !names.is_empty())).then(|| SwatchGroup { name: g.name.clone(), swatches })
        })
        .collect();
    Ok(SwatchLibrary { name, swatches, groups })
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.save";
    let path = str_param(p, "path");
    let format = match str_param(p, "format").or_else(|| path.map(|p| p.rsplit_once('.').map_or("", |(_, e)| e))).filter(|f| !f.is_empty()) {
        Some(f) => PaletteFormat::parse(f).ok_or_else(|| bad(C, format!("unknown format `{f}` (vcswatches, gpl or css)")))?,
        None => PaletteFormat::Native,
    };
    let st = s.doc()?;
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = document_library(&st.doc, &str_list(p, "names"), name, C)?;
    let (count, text) = (lib.len(), palette_io::write(&lib, format));
    let mut out = json!({"format": format.id(), "count": count});
    s.swatch_libraries.write(p, &lib.name, format.id(), text, &mut out, C)?;
    Ok(out)
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.load";
    let (info, lib) = s.swatch_libraries.load(p, C, |bytes, file| {
        // A library file, or a document whose swatches become the library.
        if palette_io::sniff_bytes(bytes) {
            palette_io::read_bytes(bytes, stem(file)).map_err(|e| bad(C, e))
        } else {
            let doc = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?.doc;
            document_library(&doc, &[], stem(file).to_string(), C)
        }
    })?;
    Ok(json!({"library": info.id, "name": info.name, "count": lib.len()}))
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "swatch.library.get")?;
    let ungrouped = lib.swatches.iter().map(|w| swatch_json(w, None));
    let grouped = lib.groups.iter().flat_map(|g| g.swatches.iter().map(|w| swatch_json(w, Some(&g.name))));
    let groups: Vec<Value> =
        lib.groups.iter().map(|g| json!({"name": g.name, "swatches": g.swatches.iter().map(|w| &w.name).collect::<Vec<_>>()})).collect();
    Ok(
        json!({"id": info.id, "name": info.name, "category": info.category, "swatches": ungrouped.chain(grouped).collect::<Vec<_>>(), "groups": groups}),
    )
}

/// The library swatches `names` stand for, each once, with the colour group it goes into (`None`:
/// ungrouped): a group's name brings the group, a swatch's name the swatch alone; no names, all.
fn picked<'a>(lib: &'a SwatchLibrary, names: &[String], cmd: &str) -> Result<Vec<(Option<&'a str>, &'a Swatch)>> {
    let mut out: Vec<(Option<&str>, &Swatch)> = vec![];
    let mut push = |g: Option<&'a str>, w: &'a Swatch| {
        if !out.iter().any(|(_, x)| x.name == w.name) {
            out.push((g, w));
        }
    };
    if names.is_empty() {
        lib.swatches.iter().for_each(|w| push(None, w));
        lib.groups.iter().for_each(|g| g.swatches.iter().for_each(|w| push(Some(&g.name), w)));
    }
    for n in names {
        match (lib.group(n), lib.swatch(n)) {
            (Some(g), _) => g.swatches.iter().for_each(|w| push(Some(&g.name), w)),
            (None, Some(w)) => push(None, w),
            (None, None) => return Err(bad(cmd, format!("no swatch or colour group `{n}` in `{}`", lib.name))),
        }
    }
    Ok(out)
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.add";
    let (info, lib) = library_param(s, p, C)?;
    let apply = match str_param(p, "apply") {
        None => None,
        Some("fill") => Some("paint.setFill"),
        Some("stroke") => Some("paint.setStroke"),
        Some(other) => return Err(bad(C, format!("apply is \"fill\" or \"stroke\", not `{other}`"))),
    };
    let picked = picked(&lib, &str_list(p, "names"), C)?;
    // Swatches the document already has (same name and paint) are reused, not added again.
    let d = &s.doc()?.doc;
    let (existing, fresh): (Vec<_>, Vec<_>) = picked.iter().partition(|(_, w)| d.swatch(&w.name).is_some_and(|x| x.paint == w.paint));
    let existing: Vec<String> = existing.iter().map(|(_, w)| w.name.clone()).collect();
    let undo_before = s.doc()?.history.undo.len();
    // Library name → the name it got in the document.
    let added: Vec<(String, String)> = if fresh.is_empty() {
        vec![]
    } else {
        s.edit("Add to Swatches", |d, _| {
            let mut out = vec![];
            for (group, w) in fresh {
                let name = d.free_swatch_name(&w.name);
                let sw = Swatch { name: name.clone(), ..(*w).clone() };
                match group {
                    Some(g) => {
                        let i = match d.swatch_groups.iter().position(|x| x.name == *g) {
                            Some(i) => i,
                            None => {
                                let name = d.free_swatch_name(g);
                                d.swatch_groups.push(SwatchGroup { name, swatches: vec![] });
                                d.swatch_groups.len() - 1
                            }
                        };
                        d.swatch_groups[i].swatches.push(sw);
                    }
                    None => d.swatches.push(sw),
                }
                out.push((w.name.clone(), name));
            }
            Ok(out)
        })?
    };
    let mut out = json!({"library": info.id, "added": added.iter().map(|(_, n)| n).collect::<Vec<_>>(), "existing": existing});
    if let (Some(cmd), Some((_, first))) = (apply, picked.first()) {
        let name = added.iter().find(|(l, _)| *l == first.name).map_or(first.name.clone(), |(_, n)| n.clone());
        s.execute(cmd, &json!({"swatch": name, "focus": bool_or(p, "focus", true)}))?;
        // Adding and applying undo together.
        squash(s, undo_before, "Add to Swatches");
        out["applied"] = json!(name);
    }
    Ok(out)
}

fn reset_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    let replace = bool_or(p, "replace", false);
    let model = s.doc()?.doc.color_mode.model();
    let (defaults, groups) = default_swatches(model);
    // With replace, links to swatches that are no longer global ones are dropped (the art keeps
    // its colour); filled in by the edit.
    let mut live: Vec<String> = vec![];
    let added = s.edit("Default Swatches", |d, _| {
        let mut added = vec![];
        if replace {
            let before: Vec<String> = d.swatches_iter().map(|w| w.name.clone()).collect();
            (d.swatches, d.swatch_groups) = (defaults, groups);
            added.extend(d.swatches_iter().filter(|w| !before.contains(&w.name)).map(|w| w.name.clone()));
            live.extend(d.swatches_iter().filter(|w| w.global).map(|w| w.name.clone()));
            d.map_solid_paints(&mut |_, l, _| unlink_dead(&live, l));
            return Ok(added);
        }
        for (i, w) in defaults.into_iter().enumerate() {
            if !d.swatch_name_taken(&w.name) {
                added.push(w.name.clone());
                // None leads, as in a new document; the others go after the existing swatches.
                let at = if i == 0 && w.paint.is_none() { 0 } else { d.swatches.len() };
                d.swatches.insert(at, w);
            }
        }
        for g in groups {
            let i = match d.swatch_groups.iter().position(|x| x.name == g.name) {
                Some(i) => i,
                None if d.swatch_name_taken(&g.name) => continue,
                None => {
                    d.swatch_groups.push(SwatchGroup { name: g.name.clone(), swatches: vec![] });
                    d.swatch_groups.len() - 1
                }
            };
            for w in g.swatches {
                if !d.swatch_name_taken(&w.name) {
                    added.push(w.name.clone());
                    d.swatch_groups[i].swatches.push(w);
                }
            }
        }
        Ok(added)
    })?;
    if replace {
        map_default_paints(s, &mut |_, l, _| unlink_dead(&live, l));
    }
    Ok(json!({ "added": added }))
}

/// Drop a link to a swatch not in `live`; true when it did.
fn unlink_dead(live: &[String], link: &mut Option<String>) -> bool {
    link.as_ref().is_some_and(|n| !live.contains(n)) && link.take().is_some()
}
