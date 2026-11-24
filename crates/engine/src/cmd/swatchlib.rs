//! Swatch libraries (Window → Swatch Libraries, the Swatches panel's library button): read-only
//! sets of swatches the library panel shows. The built-in libraries are computed in
//! [`vectorcraft_color::libraries`] and the pattern libraries in [`super::patternlib`]; User
//! Defined ones are the library files in the user library folder ([`Libraries`]), and Other
//! Library… loads more from files. `swatch.library.add` copies swatches (and a pattern swatch's
//! pattern) into the document; `swatch.library.save` writes the document's ([`palette_io`]).
//! `swatch.library.copyToUser` copies a loaded library into the user library folder.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::libraries::{BuiltinLibrary, GRADIENT_LIBRARIES, SWATCH_LIBRARIES, builtin_library};
use vectorcraft_color::palette_io::{self, PaletteFormat};
use vectorcraft_color::recolor::Palette;
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Paint, Swatch, SwatchGroup, SwatchLibrary, default_swatches};
use vectorcraft_doc::pattern::pattern_paint;

use super::fileio::{create_dir, extension, read_file, read_file_head, write_file, write_new_file};
use super::menucmds::squash;
use super::package::free_name;
use super::patternlib;
use super::swatch::{map_default_paints, map_swatch_links, str_list, swatch_json};
use super::{Color, CommandSpec, Result, Session, always, bad, cmd, has_doc, switch_or, text_param};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "swatch.library.list",
            "Swatch Libraries",
            [],
            None,
            "{} → {libraries: [{id, name, category: \"builtIn\"|\"gradients\"|\"patterns\"|\"user\" (User Defined: the library files in the user library folder, rescanned now)|\"loaded\" (swatch.library.load), count, path?}], userFolder} every library the library panel opens, in menu order",
            always,
            list
        ),
        cmd!(
            query "swatch.library.get",
            "Swatch Library",
            [],
            None,
            "{library: id or name} → {id, name, category, swatches: [{name, kind, group, global, spot, color?, hex?, gradient?, pattern?}] (as swatch.list), groups: [{name, swatches: [names]}]}",
            always,
            get
        ),
        cmd!(
            "swatch.library.add",
            "Add to Swatches",
            ["Window", "Swatch Libraries"],
            None,
            "{library: id or name, names?: [swatch or colour group names in the library] (default: all of it; a group comes as a colour group, a single swatch ungrouped), apply?: \"fill\"|\"stroke\" (also apply the first one to the selection and the defaults, as paint.setFill / paint.setStroke), focus?: true (with apply: false keeps the active proxy), defaultsOnly?: bool (with apply: apply it to the defaults only, as paint.setFill does; the journal records it for swatches applied while the Live Paint Bucket is the active tool)} copy library swatches into the document as one undo step. A swatch the document already has (same name and paint, and for a color the same process, global or spot kind, or a numbered copy of that name with the same color when the name has another color there) isn't added again; a name taken by another swatch gets a number (\"Clay 1 2\"); a tint or gradient whose global or spot color comes in under a numbered name, or is in the document as such a numbered copy, links to that copy; a global or spot color it links to that the document lacks comes in with it. A pattern swatch brings its pattern, as pasting art painted with it does: the same pattern the document has is used, also when its art links to the document's own color of a name, and a name another pattern or swatch has gets a number; a global or spot color its art links to whose name the document gives another color comes in numbered, skipping the names of the library's other swatches, and the pattern links to it → {library, added: [names in the document, the colors patterns, tints and gradients brought included], existing: [names already there], applied?: name}",
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
            "{path?, format?: \"vcswatches\" (lossless JSON: colour models, global, spot, gradients, groups) | \"gpl\" (8-bit RGB palette; groups as `# Group:` comments) | \"ase\" (swatch exchange, binary: solid colors in their own model (RGB, CMYK, Lab, Gray) as global, spot or process colors, and color groups; gradients are left out, and a tint swatch is saved as the color it shows) | \"css\" (custom properties on :root) (default: the path's extension, else vcswatches), names?: [swatch or colour group names] (default: all; None and patterns are never saved), name?: library name (default: the document's), user?: false (save into the user library folder, listed under User Defined)} save the document's swatches as a library → {path, format, count: the swatches written (gpl and ase hold solid colors only, and css leaves freeform gradients out), library?: id when saved to the user folder}; without path or user → {data: the file's text, format, count}, or {dataBase64, format, count} for ase",
            has_doc,
            save
        ),
        cmd!(
            "swatch.library.load",
            "Other Library…",
            ["Window", "Swatch Libraries"],
            None,
            "{path? | data?: file text | dataBase64?, name?: file name (default: the path's)} load a .vcswatches, .gpl, .ase (swatch exchange) or .acb (color book, read only) library, or the swatches of any document VectorCraft opens (see document.formats), such as a .ai or .eps swatch library, for the library panel (Window → Swatch Libraries lists it until the app quits; a file of the user library folder, or a file with the same extension and bytes as one there, opens as that User Defined library). A document's colors, tints, gradients and pattern swatches come in, in its order and color groups; swatch.library.add brings a pattern swatch's pattern with it. A document's graph designs are offered by Object › Graph › Column… and Marker… (graph.column, graph.marker and graph.design take the library's id) until the app quits. A document with no swatches is an error, and so is a .ai, .eps or PostScript file whose swatches are only the default ones (it has no palette of its own), unless it has graph designs, which then load with no swatches. A file larger than 256 MB is refused before it is read → {library: id, name, count, graphDesigns: the document's graph designs (0 for a library file)}",
            always,
            load
        ),
        cmd!(
            "swatch.library.copyToUser",
            "Copy to User Defined",
            ["Window", "Swatch Libraries"],
            None,
            "{library: id or name of a loaded library (category \"loaded\", swatch.library.load)} copy it into the user library folder of the desktop app, whose libraries VectorCraft lists as User Defined every time it starts (a loaded library is listed until the app quits): a library file as it is, the swatches of a document or a library loaded from data as a .vcswatches file (a library with pattern swatches is refused). A name the folder already has gets a number (\"Brand 2.ase\") and no file is replaced; when the folder already holds the same file, that file is used. The library is then listed as User Defined only, and until the app quits commands given its loaded id use the copy; a User Defined library comes back as it is → {library: its User Defined id, name, count, path (left out when it is outside the read roots), copied: false when nothing was written}",
            has_user_folder,
            copy_to_user
        ),
    ]
}

/// The graph designs of the documents Other Library… loaded (`swatch.library.load`), which the
/// Graph Column and Graph Marker dialogs offer besides the document's own; not saved.
#[derive(Clone, Debug, Default)]
pub struct DesignLibraries {
    libs: Vec<DesignLibrary>,
    /// How many times a library was set; the next library's [`DesignLibrary::generation`].
    loads: u64,
}

/// A document loaded as a swatch library that holds graph designs ([`DesignLibraries`]).
#[derive(Clone, Debug)]
pub struct DesignLibrary {
    /// The id `swatch.library.load` gave the swatch library. A User Defined copy of it
    /// (`swatch.library.copyToUser`) holds the swatches alone, so the designs stay under this id.
    pub id: String,
    pub name: String,
    /// The designs and the resources their art uses.
    pub doc: Arc<vectorcraft_doc::Document>,
    /// Which load of the library this is: it differs from every other load's, so a picture of a design of
    /// this library is stale once the library is loaded again.
    pub generation: u64,
}

impl DesignLibraries {
    /// The libraries, in the order they were loaded.
    pub fn iter(&self) -> impl Iterator<Item = &DesignLibrary> {
        self.libs.iter()
    }

    /// The library loaded as swatch library `id`.
    pub fn get(&self, id: &str) -> Option<&DesignLibrary> {
        self.libs.iter().find(|l| l.id == id)
    }

    /// Keep `doc`'s graph designs as library `id` named `name`, in place of the ones loaded as `id`
    /// before; with no designs, `id` no longer has any.
    fn set(&mut self, id: &str, name: &str, doc: Option<vectorcraft_doc::Document>) {
        self.libs.retain(|l| l.id != id);
        self.loads += 1;
        if let Some(doc) = doc {
            self.libs.push(DesignLibrary { id: id.to_string(), name: name.to_string(), doc: Arc::new(doc), generation: self.loads });
        }
    }
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
    /// Loaded libraries copied into the user library folder ([`Libraries::copy_to_user`]): the
    /// loaded id and the User Defined id [`Libraries::get`] finds for it.
    copied: Vec<(String, String)>,
    /// The name each User Defined file was saved under ([`Libraries::write`]), lower case id first,
    /// with a hash of the bytes written. A format that keeps no title (`.ase`) reads its name from
    /// the file name instead. [`Libraries::rescan`] drops an entry whose file no longer holds those
    /// bytes, and keeps one whose file is missing so a file put back can be checked.
    saved_as: HashMap<String, SavedAs>,
    /// The documents loaded swatch libraries were read from, by library id, for those with pattern
    /// swatches: the patterns live there ([`Libraries::pattern_doc`]).
    pattern_docs: HashMap<String, Arc<vectorcraft_doc::Document>>,
}

/// A User Defined file this session wrote: the library's name, and [`vectorcraft_doc::links::hash_bytes`]
/// of what was written, so a file replaced since then is no longer that library's.
#[derive(Clone, Debug)]
struct SavedAs {
    name: String,
    hash: String,
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
    /// Read the bytes of a library file whose name without the extension is `stem` (an unnamed
    /// library's name).
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String>;
    fn name(&self) -> &str;
}

impl LibraryFile for SwatchLibrary {
    const EXTS: &'static [&'static str] = LIBRARY_EXTS;
    fn read(bytes: &[u8], stem: &str) -> std::result::Result<Self, String> {
        palette_io::read_bytes(bytes, stem)
    }
    fn name(&self) -> &str {
        &self.name
    }
}

/// The extensions of library files [`palette_io::read_bytes`] reads.
pub const LIBRARY_EXTS: &[&str] = &["vcswatches", "gpl", "ase", "acb"];

/// The extensions the Swatch Libraries filter of Other Library… lists: the library files, and the
/// `.ai`, `.ait` and `.eps` (`.epsf`) documents whose swatches load as a library (`swatch.library.load`).
pub const LIBRARY_PICK_EXTS: &[&str] = &["vcswatches", "gpl", "ase", "acb", "ai", "ait", "eps", "epsf"];

impl Libraries<SwatchLibrary> {
    /// Keep `doc`, the document loaded library `id` was read from, for its pattern swatches; `None`
    /// forgets the one kept for `id`.
    fn set_pattern_doc(&mut self, id: &str, doc: Option<Arc<vectorcraft_doc::Document>>) {
        match doc {
            Some(doc) => self.pattern_docs.insert(id.to_string(), doc),
            None => self.pattern_docs.remove(id),
        };
    }

    /// The document holding the patterns of loaded library `id`'s pattern swatches.
    pub fn pattern_doc(&self, id: &str) -> Option<Arc<vectorcraft_doc::Document>> {
        self.pattern_docs.get(id).cloned()
    }

    /// The document holding the pattern of swatch `w`, when `w` is one of a loaded library's own
    /// pattern swatches (a library panel draws them from it). Only the libraries that have pattern
    /// swatches are looked at, each by the address ranges of its swatch lists.
    pub fn pattern_source(&self, w: &Swatch) -> Option<Arc<vectorcraft_doc::Document>> {
        let Paint::Pattern { pattern, .. } = &w.paint else { return None };
        let at = std::ptr::from_ref(w);
        let holds = |v: &[Swatch]| v.as_ptr_range().contains(&at);
        let own = |lib: &SwatchLibrary| holds(&lib.swatches) || lib.groups.iter().any(|g| holds(&g.swatches));
        let (_, doc) = self.pattern_docs.iter().find(|(id, _)| self.extra.iter().any(|e| e.info.id == **id && own(&e.lib)))?;
        Some(doc.clone()).filter(|d| d.pattern(pattern).is_some())
    }
}

/// A library file the save commands write ([`Libraries::write`]).
pub(crate) enum FileData {
    /// Text (`.vcswatches`, `.gpl`, CSS, `.vcstyles`), returned as `data`.
    Text(String),
    /// Bytes (`.ase`), returned as `dataBase64`.
    Binary(Vec<u8>),
}

impl<L: LibraryFile> Libraries<L> {
    pub fn user_dir(&self) -> Option<&str> {
        self.user_dir.as_deref()
    }

    /// Set the user library folder and list its libraries.
    pub fn set_user_dir(&mut self, dir: Option<String>) {
        self.user_dir = dir;
        self.saved_as.clear();
        self.rescan();
    }

    /// Re-read the library files of the user library folder (unreadable ones are skipped). The
    /// folder is the app's own: automation roots don't apply to it ([`crate::file_access`]). Each
    /// file is read inside [`crate::guard::catch_panic`], and a file whose reading panics is
    /// skipped too. A file this session saved, whose bytes are no longer the ones written, is
    /// forgotten ([`Libraries::write`]); a missing file is left remembered until it is back.
    pub fn rescan(&mut self) {
        self.extra.retain(|e| e.info.category != "user");
        let Some(dir) = self.user_dir.clone() else { return };
        for path in library_files(&dir, L::EXTS) {
            let file = file_name(&path);
            let read = || crate::file_access::unconfined(|| read_file(&path)).ok();
            let Some(bytes) = crate::guard::catch_panic(read).ok().flatten() else { continue };
            let id = format!("user/{file}").to_lowercase();
            // Compared before parsing, so a file that no longer reads as a library is forgotten too.
            let hash = vectorcraft_doc::links::hash_bytes(&bytes);
            if self.saved_as.get(&id).is_some_and(|saved| saved.hash != hash) {
                self.saved_as.remove(&id);
            }
            let parsed = || L::read(&bytes, stem(&file)).ok();
            let Some(lib) = crate::guard::catch_panic(parsed).ok().flatten() else { continue };
            let info = LibraryInfo { id: format!("user/{file}"), name: library_tab_name(&file, lib.name()), category: "user" };
            self.extra.push(Extra { info, path: Some(path), lib: Arc::new(lib) });
        }
    }

    /// The User Defined and loaded libraries, in menu order.
    pub fn infos(&self) -> impl Iterator<Item = &LibraryInfo> {
        self.extra.iter().map(|e| &e.info)
    }

    /// The User Defined or loaded library `id`. The id of a loaded library copied into the user
    /// library folder finds the User Defined copy.
    pub fn get(&self, id: &str) -> Option<(LibraryInfo, Arc<L>)> {
        let find = |id: &str| self.extra.iter().find(|e| e.info.id == id).map(|e| (e.info.clone(), e.lib.clone()));
        find(id).or_else(|| self.copied.iter().find(|(loaded, _)| loaded == id).and_then(|(_, user)| find(user)))
    }

    /// The file library `id` was read from.
    pub fn path(&self, id: &str) -> Option<&str> {
        self.extra.iter().find(|e| e.info.id == id).and_then(|e| e.path.as_deref())
    }

    /// The ids [`Libraries::get`] finds: the User Defined and loaded libraries', and the loaded ids of
    /// libraries copied into the user library folder.
    fn ids(&self) -> impl Iterator<Item = String> + '_ {
        self.extra.iter().map(|e| e.info.id.clone()).chain(self.copied.iter().map(|(loaded, _)| loaded.clone()))
    }

    /// The file library `id` was read from, when the request being served may read it.
    pub fn shown_path(&self, id: &str) -> Option<&str> {
        self.path(id).filter(|p| readable(p))
    }

    /// The user library folder, when the request being served may read it.
    pub fn shown_user_dir(&self) -> Option<&str> {
        self.user_dir().filter(|d| readable(d))
    }

    /// The User Defined library whose file has the extension of file name `file` and holds `bytes`.
    fn user_file(&self, file: &str, bytes: &[u8]) -> Option<&Extra<L>> {
        let ext = extension(file);
        self.extra.iter().find(|e| {
            e.info.category == "user"
                && e.path.as_deref().is_some_and(|p| extension(p) == ext && crate::file_access::unconfined(|| read_file(p)).is_ok_and(|b| b == bytes))
        })
    }

    /// Write library file `data` as the save commands do: into the user library folder as
    /// `name`.`ext` with `user: true` (→ `path`, and `library`: its id), to `path`, else back as
    /// `data` (text) or `dataBase64` (bytes); the results go into `out`.
    pub(crate) fn write(&mut self, p: &Value, name: &str, ext: &str, data: FileData, out: &mut Value, cmd: &str) -> Result<()> {
        let bytes = match &data {
            FileData::Text(text) => text.as_bytes(),
            FileData::Binary(bytes) => bytes.as_slice(),
        };
        let path = text_param(p, "path", cmd)?;
        if switch_or(p, "user", false, cmd)? {
            let dir = self
                .user_dir()
                .ok_or_else(|| bad(cmd, "no user library folder here (save with a path, or without one to get the data)"))?
                .to_string();
            let mut file = library_file_name(name, ext);
            // `COM¹` is saved as `COM¹-`, which can be another library's own name, and a library put
            // under a numbered name holds a file another name can ask for: such a file is kept.
            // `saved_as` is that memory for this run, and only while the file still holds what was
            // written. A file that could not be read has no name to trust: one a device name could
            // make is kept too. Any other unreadable file is replaced, as it was before.
            self.rescan();
            let device = crate::file_access::is_device_name;
            let id = format!("user/{file}").to_lowercase();
            let fallback = |other: &str| library_file_name(other, ext).to_lowercase() != file.to_lowercase();
            // A format with no title reads its name from the file name, so a file a device name could
            // have made has no known owner until it is saved here.
            let untitled = L::read(bytes, "\0").is_ok_and(|lib| lib.name() == "\0");
            let owner = |e: &Extra<L>| match self.saved_as.get(&e.info.id.to_lowercase()) {
                Some(saved) => Some(saved.name.clone()),
                None if untitled && device_file_name(e.lib.name()) => None,
                None => Some(e.lib.name().to_string()),
            };
            let file_key = file.to_lowercase();
            let on_disk = library_files(&dir, L::EXTS).iter().any(|p| file_name(p).to_lowercase() == file_key);
            let listed = self.extra.iter().any(|e| e.info.category == "user" && e.info.id.to_lowercase() == id);
            let unread = on_disk && !listed && (device(name) || device_file_name(stem(&file)));
            let held = unread
                || self.extra.iter().any(|e| {
                    e.info.id.to_lowercase() == id
                        && owner(e).is_none_or(|other| other != name && (device(name) || device(&other) || fallback(&other)))
                });
            if held {
                // Saved again, it goes back to the file it was given the first time.
                let own = self
                    .extra
                    .iter()
                    .find(|e| e.info.category == "user" && owner(e).as_deref() == Some(name) && extension(&e.info.id) == extension(&file));
                file = match own.and_then(|e| e.info.id.strip_prefix("user/")) {
                    Some(own) => own.to_string(),
                    None => {
                        let mut taken: HashSet<String> = library_files(&dir, L::EXTS).iter().map(|p| file_name(p).to_lowercase()).collect();
                        free_name(&file, &mut taken)
                    }
                };
            }
            let path = std::path::Path::new(&dir).join(&file).to_string_lossy().to_string();
            // The app's own folder, under a name made safe: no root applies.
            crate::file_access::unconfined(|| create_dir(&dir).and_then(|()| write_file(&path, bytes)))?;
            self.saved_as
                .insert(format!("user/{file}").to_lowercase(), SavedAs { name: name.to_string(), hash: vectorcraft_doc::links::hash_bytes(bytes) });
            self.rescan();
            // The folder is the app's own: a confined request sees the file only inside its read roots.
            if readable(&path) {
                out["path"] = json!(path);
            }
            out["library"] = json!(format!("user/{file}"));
        } else if let Some(path) = path {
            write_file(path, bytes)?;
            out["path"] = json!(path);
        } else {
            match data {
                FileData::Text(text) => out["data"] = json!(text),
                FileData::Binary(bytes) => out["dataBase64"] = json!(vectorcraft_format::base64_encode(&bytes)),
            }
        }
        Ok(())
    }

    /// Load a library from `p` (`path`, `data` or `dataBase64`; `name`: the file's name) as the
    /// load commands do, `parse(bytes, file name)` reading it. A file of the user library folder,
    /// or a file with the same extension and bytes as one there, is that User Defined library;
    /// others are listed as loaded until the app quits.
    pub(crate) fn load(&mut self, p: &Value, cmd: &str, parse: impl FnOnce(&[u8], &str) -> Result<L>) -> Result<(LibraryInfo, Arc<L>)> {
        self.load_within(p, cmd, None, parse)
    }

    /// [`Self::load`], with a file at `path` read up to `max` bytes and refused when it is longer.
    pub(crate) fn load_within(
        &mut self,
        p: &Value,
        cmd: &str,
        max: Option<usize>,
        parse: impl FnOnce(&[u8], &str) -> Result<L>,
    ) -> Result<(LibraryInfo, Arc<L>)> {
        let path = text_param(p, "path", cmd)?;
        let data = text_param(p, "data", cmd)?;
        let data_base64 = text_param(p, "dataBase64", cmd)?;
        let name = text_param(p, "name", cmd)?;
        let bytes = match (path, data, data_base64) {
            (Some(path), ..) => match max {
                Some(max) => {
                    let bytes = read_file_head(path, max.saturating_add(1))?;
                    if bytes.len() > max {
                        return Err(bad(cmd, too_big_message(max)));
                    }
                    bytes
                }
                None => read_file(path)?,
            },
            (None, Some(text), _) => text.as_bytes().to_vec(),
            (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(cmd, "bad dataBase64"))?,
            _ => return Err(bad(cmd, "give `path`, `data` or `dataBase64`")),
        };
        let file = name.map(str::to_string).or_else(|| path.map(file_name)).unwrap_or_else(|| "Library".into());
        let lib = parse(&bytes, &file)?;
        self.rescan();
        let user =
            |p: &str| self.extra.iter().find(|e| e.info.category == "user" && e.path.as_deref() == Some(p)).or_else(|| self.user_file(p, &bytes));
        if let Some(e) = path.and_then(user) {
            return Ok((e.info.clone(), e.lib.clone()));
        }
        let info = LibraryInfo { id: format!("loaded/{}", path.unwrap_or(&file)), name: library_tab_name(&file, lib.name()), category: "loaded" };
        let lib = Arc::new(lib);
        self.extra.retain(|x| x.info.id != info.id);
        self.extra.push(Extra { info: info.clone(), path: path.map(str::to_string), lib: lib.clone() });
        Ok((info, lib))
    }

    /// Copy loaded library `id` into the user library folder, where it is listed as User Defined in
    /// place of the loaded entry; `id` then finds the copy ([`Libraries::get`]). A library read
    /// from a library file (an extension of `L::EXTS`) is copied as that file's bytes under the
    /// file's name; any other (the swatches of a document, a library loaded from data) is written
    /// by `encode` as a `.{ext}` file named after the library. A name the folder already has,
    /// compared without case, gets a number ("Brand 2.ase"), and no file is replaced. When a file
    /// of the folder with the same extension holds the same bytes, its library is used and nothing
    /// is written. → the User Defined library, and whether a file was written.
    pub(crate) fn copy_to_user(
        &mut self,
        id: &str,
        ext: &str,
        encode: impl FnOnce(&L) -> Result<Vec<u8>>,
        cmd: &str,
    ) -> Result<(LibraryInfo, Arc<L>, bool)> {
        let dir = self.user_dir().ok_or_else(|| bad(cmd, "no user library folder here"))?.to_string();
        let (source, lib) = self
            .extra
            .iter()
            .find(|e| e.info.id == id && e.info.category == "loaded")
            .map(|e| (e.path.clone(), e.lib.clone()))
            .ok_or_else(|| bad(cmd, format!("`{id}` isn't a loaded library")))?;
        let (file, bytes) = match source.filter(|p| L::EXTS.contains(&extension(p).as_str())) {
            Some(p) => {
                // The file's bytes are read: outside the request's read roots, the error names no
                // path. A document's swatches (the arm below) are written from the loaded library,
                // and the document is not read again.
                if !readable(&p) {
                    return Err(bad(cmd, "the library's file is outside the read roots"));
                }
                let ext = extension(&p);
                let base = file_name(&p);
                // A sandbox copy of an import often lands as tmp….tmp.acb (#2996): store it under the
                // book title. A normal .acb path keeps its file name so the tab matches Illustrator
                // (#3395).
                let file = if temporary_library_file_name(&base) { library_file_name(lib.name(), &ext) } else { base };
                (file, read_file(&p)?)
            }
            None => (library_file_name(lib.name(), ext), encode(&lib)?),
        };
        self.rescan();
        let (user_id, written) = match self.user_file(&file, &bytes).map(|e| e.info.id.clone()) {
            Some(same) => (same, None),
            None => {
                let mut taken: HashSet<String> = library_files(&dir, L::EXTS).iter().map(|p| file_name(p).to_lowercase()).collect();
                let file = free_name(&file, &mut taken);
                let path = std::path::Path::new(&dir).join(&file).to_string_lossy().to_string();
                // The app's own folder, under a name made safe: no root applies.
                crate::file_access::unconfined(|| create_dir(&dir).and_then(|()| write_new_file(&path, &bytes)))?;
                self.rescan();
                (format!("user/{file}"), Some(path))
            }
        };
        let Some((info, lib)) = self.get(&user_id) else {
            if let Some(path) = &written {
                // Best effort: the copy that can't be read is all there is to clean up.
                let _ = std::fs::remove_file(path);
            }
            return Err(bad(cmd, format!("the copy `{user_id}` can't be read as a library")));
        };
        self.extra.retain(|x| x.info.id != id);
        self.copied.retain(|(loaded, _)| loaded != id);
        self.copied.push((id.to_string(), info.id.clone()));
        Ok((info, lib, written.is_some()))
    }
}

/// A library file name that looks like a host temporary (e.g. macOS `tmp1F2C.tmp.acb`).
fn temporary_library_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(".tmp.")
}

/// The name library panels and menus show: color books (.acb) use the file name without its
/// extension (Illustrator), except when the path is a host temporary (#2996), then the book title.
fn library_tab_name(file: &str, lib_name: &str) -> String {
    let base = file_name(file);
    if extension(&base) == "acb" && !temporary_library_file_name(&base) { stem(&base).to_string() } else { lib_name.to_string() }
}

/// The file name of library `name` with extension `ext`: characters file systems reject become
/// `-`, and dots and spaces at either end are dropped. A name with nothing left is `Library`, and
/// one that is a Windows device name (`CON`, `NUL`, `COM1`…, also with an extension) gets a `-`
/// after its first part (`CON-.json`, `CON-.txt.json`).
pub fn library_file_name(name: &str, ext: &str) -> String {
    let base: String = name.chars().map(|c| if "/\\:*?\"<>|".contains(c) || c.is_control() { '-' } else { c }).collect();
    let base = base.trim_matches(['.', ' ']);
    if base.is_empty() {
        format!("Library.{ext}")
    } else if crate::file_access::is_device_name(base) {
        // The part before the first dot is what makes it a device, so the `-` goes there.
        let (stem, rest) = base.split_once('.').map_or((base, String::new()), |(stem, rest)| (stem, format!(".{rest}")));
        format!("{stem}-{rest}.{ext}")
    } else {
        format!("{base}.{ext}")
    }
}

/// Is `stem` a file name [`library_file_name`] gives a device name (`COM¹-`, `COM¹-.x`), or one of
/// those with the number [`free_name`] adds (`COM¹- 2`)?
fn device_file_name(stem: &str) -> bool {
    let base = stem.rsplit_once(' ').filter(|(_, n)| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).map_or(stem, |(base, _)| base);
    base.split('.').next().and_then(|first| first.strip_suffix('-')).is_some_and(crate::file_access::is_device_name)
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

/// The file name a concealed library or path is shown by: the last part of `path` as this platform
/// splits a path, so that outside Windows a `\` stays part of the name. Another platform's absolute
/// path ([`crate::file_access::written_elsewhere`]), and a path this platform gives no file name (one
/// that ends in `..`), are split after the last `/` or `\` instead. `path` itself when it has no such part.
pub fn concealed_file_name(path: &str) -> String {
    let native = (!crate::file_access::written_elsewhere(path)).then(|| std::path::Path::new(path).file_name()).flatten();
    native.map_or_else(|| path.rsplit(['/', '\\']).find(|part| !part.is_empty()).unwrap_or(path).to_string(), |n| n.to_string_lossy().into_owned())
}

/// May the request being served read `path` ([`crate::file_access::check_read`])? Always, without
/// roots in force. Another platform's absolute path ([`crate::file_access::written_elsewhere`]) is
/// never readable under a read root.
fn readable(path: &str) -> bool {
    !crate::file_access::written_elsewhere(path) && crate::file_access::check_read(path).is_ok()
}

/// The loaded library ids (`loaded/<path>`) whose file the request being served may not read, each
/// with the id a reply shows instead: `loaded/<file name>`, as a library loaded without a path is
/// named. The file name is [`concealed_file_name`], and a path saved as another platform's absolute
/// path counts as one the request may not read. The ids are those of the
/// session's swatch, brush, symbol and graphic style libraries, then `others` (ids the UI state refers
/// to). When another id already is or shows that id, the next free
/// one of `loaded/<file name> (2)`, `(3)` and so on is shown, so that two libraries stay two. A library
/// keeps the id it was first shown under for the rest of the session, so an id a client was given
/// finds the same library after another one with the same file name is loaded; only a library that
/// shows that id itself (one loaded later without a file under that name) moves it to the next free
/// one. Libraries shown for the first time are numbered the session's first, in the sorted order of
/// their full ids, then `others`, in the same order. Without roots in force, none.
pub fn concealed_library_ids(s: &Session, others: impl IntoIterator<Item = String>) -> Vec<(String, String)> {
    if crate::file_access::current().is_none() {
        return vec![];
    }
    let session: BTreeSet<String> =
        s.swatch_libraries.ids().chain(s.brush_libraries.ids()).chain(s.symbol_libraries.ids()).chain(s.style_libraries.ids()).collect();
    let others: BTreeSet<String> = others.into_iter().filter(|id| !session.contains(id)).collect();
    let mut hidden = Vec::new();
    let mut taken = BTreeSet::new();
    for ids in [session, others] {
        for id in ids {
            let name = id.strip_prefix("loaded/").filter(|path| !path.is_empty()).map(|path| (concealed_file_name(path), path));
            match name {
                Some((name, path)) if name != path && !readable(path) => hidden.push((id, name)),
                _ => {
                    taken.insert(id);
                }
            }
        }
    }
    let mut assigned = s.shown_library_ids.lock().unwrap_or_else(|e| e.into_inner());
    let mut kept = vec![None; hidden.len()];
    for (slot, (id, _)) in kept.iter_mut().zip(&hidden) {
        if let Some(shown) = assigned.get(id).filter(|shown| !taken.contains(*shown)) {
            taken.insert(shown.clone());
            *slot = Some(shown.clone());
        }
    }
    // A new id is never one given to another library earlier in the session.
    let mut given: BTreeSet<String> = assigned.values().cloned().collect();
    hidden
        .into_iter()
        .zip(kept)
        .map(|((id, name), kept)| {
            let shown = kept.unwrap_or_else(|| {
                let mut shown = format!("loaded/{name}");
                let mut n = 2;
                while taken.contains(&shown) || given.contains(&shown) {
                    shown = format!("loaded/{name} ({n})");
                    n += 1;
                }
                taken.insert(shown.clone());
                given.insert(shown.clone());
                assigned.insert(id.clone(), shown.clone());
                shown
            });
            (id, shown)
        })
        .collect()
}

/// The id a reply shows for library `id`, given the [`concealed_library_ids`] in force.
pub(crate) fn shown_id(concealed: &[(String, String)], id: &str) -> String {
    concealed.iter().find(|(own, _)| own == id).map_or_else(|| id.to_string(), |(_, shown)| shown.clone())
}

/// The id a reply shows for library `id` ([`concealed_library_ids`]).
pub(crate) fn shown_library_id(s: &Session, id: &str) -> String {
    shown_id(&concealed_library_ids(s, []), id)
}

/// The library id behind `key` when `key` is the id a reply shows for a concealed loaded library.
pub(crate) fn concealed_library_id(s: &Session, key: &str) -> Option<String> {
    if !key.starts_with("loaded/") {
        return None;
    }
    concealed_library_ids(s, []).into_iter().find(|(_, shown)| shown == key).map(|(own, _)| own)
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
    /// "builtIn", "gradients", "patterns", "user" or "loaded".
    pub category: &'static str,
}

/// The built-in libraries with their categories.
fn builtins() -> impl Iterator<Item = (&'static str, &'static BuiltinLibrary)> {
    SWATCH_LIBRARIES.iter().map(|b| ("builtIn", b)).chain(GRADIENT_LIBRARIES.iter().map(|b| ("gradients", b)))
}

fn builtin_info((category, b): (&'static str, &BuiltinLibrary)) -> LibraryInfo {
    LibraryInfo { id: b.id.into(), name: b.name.into(), category }
}

/// Every library, in menu order: built-in, gradients, patterns, User Defined, loaded.
pub fn libraries(s: &Session) -> Vec<LibraryInfo> {
    builtins().map(builtin_info).chain(patternlib::infos()).chain(s.swatch_libraries.infos().cloned()).collect()
}

/// Library `key` (an id, or a name in any case) with its info.
pub fn library(s: &Session, key: &str) -> Option<(LibraryInfo, Arc<SwatchLibrary>)> {
    if let Some(found) = s.swatch_libraries.get(key) {
        return Some(found);
    }
    if let Some(found) = concealed_library_id(s, key).and_then(|id| s.swatch_libraries.get(&id)) {
        return Some(found);
    }
    if let Some(b) = builtins().find(|(_, b)| b.id == key) {
        return Some((builtin_info(b), builtin_library(key)?));
    }
    if let Some(found) = patternlib::library(key) {
        return Some(found);
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
    Some(Palette::new(library_colors(s, key)?))
}

/// The solid colours of library `key` (an id or name) or, for [`DOCUMENT_SWATCHES`], of the
/// active document's swatches and colour groups. `None` when there is no such library (or
/// document).
pub fn library_colors(s: &Session, key: &str) -> Option<Vec<Color>> {
    let colors = |sw: &Swatch| sw.paint.color();
    if key == DOCUMENT_SWATCHES {
        return Some(s.active()?.doc.swatches_iter().filter_map(colors).collect());
    }
    Some(library(s, key)?.1.iter().filter_map(colors).collect())
}

/// The colour swatches of library `key` (an id or name) or, for [`DOCUMENT_SWATCHES`], of the
/// active document (tint swatches left out: they stand for their base). `None` when there is no
/// such library (or document).
pub fn library_swatches(s: &Session, key: &str) -> Option<Vec<Swatch>> {
    let own = |sw: &&Swatch| matches!(sw.paint, Paint::Solid { swatch: None, .. });
    if key == DOCUMENT_SWATCHES {
        return Some(s.active()?.doc.swatches_iter().filter(own).cloned().collect());
    }
    Some(library(s, key)?.1.iter().filter(own).cloned().collect())
}

/// The document holding the pattern of library swatch `w`: a built-in pattern library's or a loaded
/// library's own pattern swatch (a library panel draws them from it).
pub fn pattern_source(s: &Session, w: &Swatch) -> Option<Arc<vectorcraft_doc::Document>> {
    patternlib::swatch_source(w).or_else(|| s.swatch_libraries.pattern_source(w))
}

/// The `limitTo` parameter's palette ([`limit_palette`]); `None` without one (or for "" or null).
/// A `limitTo` that is not a string, such as `5`, names no library.
pub(crate) fn limit_param(s: &Session, p: &Value, cmd: &str) -> Result<Option<Palette>> {
    let no_library =
        |key: &str| bad(cmd, format!("no swatch library `{key}` (see swatch.library.list; \"{DOCUMENT_SWATCHES}\": the document's swatches)"));
    let key = match p.get("limitTo") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::String(k)) if k.is_empty() => return Ok(None),
        Some(Value::String(k)) => k.as_str(),
        Some(v) => return Err(no_library(&v.to_string())),
    };
    let palette = limit_palette(s, key).ok_or_else(|| no_library(key))?;
    if palette.is_empty() {
        return Err(bad(cmd, format!("`{key}` has no colours")));
    }
    Ok(Some(palette))
}

/// The `names` parameter ([`str_list`]): empty when it is left out or null; anything but a list of
/// strings is an error.
pub(super) fn names_param(p: &Value, cmd: &str) -> Result<Vec<String>> {
    match p.get("names") {
        None | Some(Value::Null) => Ok(vec![]),
        Some(Value::Array(a)) if a.iter().all(Value::is_string) => Ok(str_list(p, "names")),
        Some(_) => Err(bad(cmd, "`names` must be a list of names")),
    }
}

fn library_param(s: &Session, p: &Value, cmd: &str) -> Result<(LibraryInfo, Arc<SwatchLibrary>)> {
    let key = text_param(p, "library", cmd)?.ok_or_else(|| bad(cmd, "missing `library` (see swatch.library.list)"))?;
    library(s, key).ok_or_else(|| bad(cmd, format!("no swatch library `{key}` (see swatch.library.list)")))
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    s.swatch_libraries.rescan();
    let concealed = concealed_library_ids(s, []);
    let libs: Vec<Value> = libraries(s)
        .into_iter()
        .map(|l| {
            let count = library(s, &l.id).map_or(0, |(_, lib)| lib.len());
            let (id, path) = (shown_id(&concealed, &l.id), s.swatch_libraries.shown_path(&l.id));
            json!({"id": id, "name": l.name, "category": l.category, "count": count, "path": path})
        })
        .collect();
    Ok(json!({ "libraries": libs, "userFolder": s.swatch_libraries.shown_user_dir() }))
}

/// The document's swatches `names` (swatches or colour groups; empty: all) as a library named
/// `name`, without None, and without pattern swatches unless `patterns`.
fn document_library(d: &vectorcraft_doc::Document, names: &[String], name: String, patterns: bool, cmd: &str) -> Result<SwatchLibrary> {
    if let Some(n) = names.iter().find(|n| !d.swatch_name_taken(n)) {
        return Err(bad(cmd, format!("no swatch or colour group `{n}`")));
    }
    let want = |n: &str| names.is_empty() || names.iter().any(|x| x == n);
    let savable = |w: &&Swatch| match w.paint {
        Paint::Solid { .. } | Paint::Gradient(_) => true,
        Paint::Pattern { .. } => patterns,
        _ => false,
    };
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
    let path = text_param(p, "path", C)?;
    let format = match text_param(p, "format", C)?.or_else(|| path.map(|p| p.rsplit_once('.').map_or("", |(_, e)| e))).filter(|f| !f.is_empty()) {
        Some(f) => PaletteFormat::parse(f).ok_or_else(|| bad(C, format!("unknown format `{f}` (vcswatches, gpl, ase or css)")))?,
        None => PaletteFormat::Native,
    };
    let names = names_param(p, C)?;
    let st = s.doc()?;
    let name = text_param(p, "name", C)?.map(str::trim).filter(|n| !n.is_empty()).map_or_else(|| stem(&st.title()).to_string(), str::to_string);
    let lib = document_library(&st.doc, &names, name, false, C)?;
    let bytes = palette_io::write(&lib, format).map_err(|e| bad(C, e))?;
    let data = if format.binary() { FileData::Binary(bytes) } else { FileData::Text(String::from_utf8_lossy(&bytes).into_owned()) };
    let mut out = json!({"format": format.id(), "count": lib.iter().filter(|w| format.holds(&w.paint)).count()});
    s.swatch_libraries.write(p, &lib.name, format.id(), data, &mut out, C)?;
    Ok(out)
}

/// The error for a library file larger than `max` bytes.
fn too_big_message(max: usize) -> String {
    format!("the file is larger than {} MB", max >> 20)
}

/// Whether the library `p` gives (`path`, `data` or `dataBase64`) is larger than `max` bytes,
/// checked before the file is read, copied or decoded.
pub(crate) fn library_too_big(p: &Value, cmd: &str, max: usize) -> Result<bool> {
    Ok(match text_param(p, "path", cmd)? {
        Some(path) => super::fileio::file_stamp(path).is_some_and(|(len, _)| len > max as u64),
        None => match text_param(p, "data", cmd)? {
            Some(text) => text.len() > max,
            // Base64 holds 3 bytes in every 4 characters (padding and white space hold none).
            None => {
                text_param(p, "dataBase64", cmd)?.is_some_and(|b| b.bytes().filter(|c| *c != b'=' && !c.is_ascii_whitespace()).count() * 3 / 4 > max)
            }
        },
    })
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.load";
    let max = super::place::document::MAX_FILE;
    if library_too_big(p, C, max)? {
        return Err(bad(C, too_big_message(max)));
    }
    let mut designs = None;
    // The document a library with pattern swatches was read from, which holds their patterns.
    let mut patterns = None;
    // The file is read no further than the limit, so a file that grew since it was checked is
    // refused too.
    let (info, lib) = s.swatch_libraries.load_within(p, C, Some(max), |bytes, file| {
        // A library file, or a document whose swatches become the library and whose graph
        // designs the Graph Column and Graph Marker dialogs offer.
        if palette_io::sniff_bytes(bytes) {
            return palette_io::read_bytes(bytes, stem(file)).map_err(|e| bad(C, e));
        }
        let loaded = super::fileio::load(file, bytes).map_err(|e| bad(C, e.to_string()))?;
        if loaded.preview_only {
            return Err(bad(C, "only this Affinity file's embedded preview could be read; use File › Open to see it with its warning"));
        }
        // A `.ai`, EPS or PostScript file without a palette of its own opens with the default swatches.
        let defaults = matches!(loaded.format.id, "ai" | "ait" | "eps" | "ps") && has_only_default_swatches(&loaded.doc);
        let mut lib = document_library(&loaded.doc, &[], stem(file).to_string(), true, C)?;
        let doc_designs = super::clipboard::resources::graph_design_library(&loaded.doc);
        if defaults || lib.is_empty() {
            // A document with graph designs loads for them, with no swatches.
            if doc_designs.is_none() {
                return Err(bad(C, format!("`{file}` has no swatches of its own")));
            }
            lib.swatches.clear();
            lib.groups.clear();
        }
        designs = doc_designs;
        if lib.iter().any(|w| matches!(w.paint, Paint::Pattern { .. })) {
            patterns = Some(Arc::new(loaded.doc));
        }
        Ok(lib)
    })?;
    if info.category == "loaded" {
        s.swatch_libraries.set_pattern_doc(&info.id, patterns);
    }
    let count = designs.as_ref().map_or(0, |d| d.graph_designs.len());
    s.graph_design_libraries.set(&info.id, &info.name, designs);
    Ok(json!({"library": shown_library_id(s, &info.id), "name": info.name, "count": lib.len(), "graphDesigns": count}))
}

/// `d`'s swatches and color groups are exactly the default ones of a color mode, as a new
/// document's are.
fn has_only_default_swatches(d: &vectorcraft_doc::Document) -> bool {
    use vectorcraft_color::cms::Model;
    [Model::Rgb, Model::Cmyk].into_iter().any(|model| {
        let (swatches, groups) = default_swatches(model);
        d.swatches[..] == swatches[..] && d.swatch_groups[..] == groups[..]
    })
}

fn get(s: &mut Session, p: &Value) -> Result<Value> {
    let (info, lib) = library_param(s, p, "swatch.library.get")?;
    let ungrouped = lib.swatches.iter().map(|w| swatch_json(w, None));
    let grouped = lib.groups.iter().flat_map(|g| g.swatches.iter().map(|w| swatch_json(w, Some(&g.name))));
    let groups: Vec<Value> =
        lib.groups.iter().map(|g| json!({"name": g.name, "swatches": g.swatches.iter().map(|w| &w.name).collect::<Vec<_>>()})).collect();
    Ok(
        json!({"id": shown_library_id(s, &info.id), "name": info.name, "category": info.category, "swatches": ungrouped.chain(grouped).collect::<Vec<_>>(), "groups": groups}),
    )
}

/// There is a user library folder to copy libraries into (the desktop app sets one).
fn has_user_folder(s: &Session) -> std::result::Result<(), String> {
    match s.swatch_libraries.user_dir() {
        Some(_) => Ok(()),
        None => Err("no user library folder here".into()),
    }
}

fn copy_to_user(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.copyToUser";
    let (info, lib) = library_param(s, p, C)?;
    let (info, lib, copied) = match info.category {
        "loaded" if lib.iter().any(|w| matches!(w.paint, Paint::Pattern { .. })) => {
            return Err(bad(C, format!("`{}` has pattern swatches, which a .vcswatches file can't hold", info.name)));
        }
        "loaded" => {
            let native = |lib: &SwatchLibrary| palette_io::write(lib, PaletteFormat::Native).map_err(|e| bad(C, e));
            s.swatch_libraries.copy_to_user(&info.id, PaletteFormat::Native.id(), native, C)?
        }
        "user" => (info, lib, false),
        _ => return Err(bad(C, format!("`{}` is built in and always listed", info.name))),
    };
    let mut out = json!({"library": info.id, "name": info.name, "count": lib.len(), "copied": copied});
    // As the saves with `user: true`: no `path` when the request may not read the file.
    if let Some(path) = s.swatch_libraries.shown_path(&info.id) {
        out["path"] = json!(path);
    }
    Ok(out)
}

/// The library swatches `names` stand for, each once, with the colour group it goes into (`None`:
/// ungrouped): a group's name brings the group, a swatch's name the swatch alone; no names, all.
fn picked<'a>(lib: &'a SwatchLibrary, names: &[String], cmd: &str) -> Result<Vec<(Option<&'a str>, &'a Swatch)>> {
    let mut out: Vec<(Option<&str>, &Swatch)> = vec![];
    let mut seen: HashSet<&str> = HashSet::new();
    let mut push = |g: Option<&'a str>, w: &'a Swatch| {
        if seen.insert(w.name.as_str()) {
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

/// The swatch of `d` that library pattern swatch `w` stands for once its pattern is `target` in `d`:
/// one that paints `target` placed as `w` places its pattern, named as `w` or a numbered copy of
/// that name ("Tiles 2"); for a swatch named after its pattern, under any name.
fn library_pattern_swatch(d: &vectorcraft_doc::Document, w: &Swatch, target: &str) -> Option<String> {
    let Paint::Pattern { pattern, xf } = &w.paint else { return None };
    let paint = Paint::Pattern { pattern: target.to_string(), xf: *xf };
    let named = |n: &str| {
        w.name == *pattern
            || n == w.name
            || n.strip_prefix(w.name.as_str())
                .and_then(|r| r.strip_prefix(' '))
                .is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
    };
    d.swatches_iter().find(|x| x.paint == paint && named(&x.name)).map(|x| x.name.clone())
}

/// Every name a swatch or colour group of `d` uses (they share one namespace, with the registration swatch).
fn taken_names(d: &vectorcraft_doc::Document) -> HashSet<String> {
    let names = d.swatches_iter().map(|w| &w.name).chain(d.swatch_groups.iter().map(|g| &g.name));
    names.cloned().chain([REGISTRATION.to_string()]).collect()
}

/// [`vectorcraft_doc::Document::free_swatch_name`] against `taken`, which takes the name returned.
fn free_in(taken: &mut HashSet<String>, base: &str) -> String {
    let name = if taken.contains(base) {
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken.contains(n)).unwrap_or_else(|| base.to_string())
    } else {
        base.to_string()
    };
    taken.insert(name.clone());
    name
}

/// [`free_in`] for a library swatch's copy: a numbered name also skips the names `reserved` says
/// the library's other swatches have, so that a later add finds the copy as this swatch's.
fn free_copy_in(taken: &mut HashSet<String>, base: &str, reserved: impl Fn(&str) -> bool) -> String {
    let name = if taken.contains(base) {
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken.contains(n) && !reserved(n)).unwrap_or_else(|| base.to_string())
    } else {
        base.to_string()
    };
    taken.insert(name.clone());
    name
}

/// [`patternlib::import`] of pattern `pattern` of `src` into `d`, keeping the library's numbered
/// swatch names (the keys of `names`, such as "Ink 2") that `d` does not use for the library's own
/// swatches of those names, so that a color the art brings under a numbered name does not take one
/// of them. While the import runs, a placeholder swatch holds each such name the art does not
/// bring, and a copy of the library's color holds each one the art brings, which the art then
/// takes. The placeholders are removed before it returns, and so is a copy the art did not take;
/// a copy the art took is listed with the colors the import brought. Names of the patterns the
/// import brings are not held.
fn import_holding(
    d: &mut vectorcraft_doc::Document,
    src: &vectorcraft_doc::Document,
    pattern: &str,
    names: &HashMap<&str, &Swatch>,
) -> Result<(String, patternlib::SwatchNames, patternlib::SwatchNames)> {
    let numbered = |n: &str| n.rsplit_once(' ').is_some_and(|(_, k)| !k.is_empty() && k.bytes().all(|b| b.is_ascii_digit()));
    let taken = taken_names(d);
    let mut free: Vec<&str> = names.keys().copied().filter(|n| numbered(n) && !taken.contains(*n)).collect();
    free.sort_unstable();
    let (mut held, mut copies): (Vec<Swatch>, Vec<Swatch>) = (vec![], vec![]);
    if !free.is_empty() {
        let brings = patternlib::brought_names(src, pattern);
        let patterns = patternlib::brought_patterns(src, pattern);
        for n in free.into_iter().filter(|n| !patterns.contains(*n)) {
            if !brings.contains(n) {
                held.push(Swatch { name: n.to_string(), paint: Paint::None, global: false, spot: false });
            } else if let Some(w) = names.get(n).filter(|w| w.paint.color().is_some() && linked_names(&w.paint).is_empty()) {
                copies.push((*w).clone());
            }
        }
    }
    let start = d.swatches.len();
    d.swatches.extend(held.iter().chain(&copies).cloned());
    let imported = patternlib::import(d, src, pattern);
    let used = |n: &str| {
        imported
            .as_ref()
            .is_ok_and(|(_, colors, took)| !colors.iter().any(|(from, _)| from == n) && !took.iter().any(|(from, to)| from == n && to != n))
    };
    let used: HashSet<String> = copies.iter().map(|w| w.name.clone()).filter(|n| used(n)).collect();
    let mut i = 0;
    d.swatches.retain(|w| {
        i += 1;
        let ours = held.iter().chain(&copies).any(|h| h.name == w.name && h.paint == w.paint);
        !(i > start && ours && !used.contains(&w.name))
    });
    let (target, mut colors, mut took) = imported?;
    took.retain(|(from, _)| !used.contains(from));
    colors.extend(copies.into_iter().map(|w| w.name).filter(|n| used.contains(n)).map(|n| (n.clone(), n)));
    Ok((target, colors, took))
}

/// Link the swatches of `d` that `pending` names (document name, library swatch) to their colors
/// under the names `links` gives them, for each whose colors `links` names now. Those leave
/// `pending` and are returned.
fn settle<'a>(
    d: &mut vectorcraft_doc::Document,
    pending: &mut Vec<(String, &'a Swatch)>,
    links: &HashMap<String, String>,
) -> Vec<(String, &'a Swatch)> {
    let mut done = vec![];
    pending.retain(|(name, w)| {
        let all = all_linked(&w.paint, links);
        if all {
            done.push((name.clone(), *w));
        }
        !all
    });
    relink_in(d, &done, links);
    done
}

/// Give the swatches of `d` that `list` names (document name, library swatch) the library swatch's
/// paint with its colors linked to the names `links` gives them, in one pass over the document's
/// swatches.
fn relink_in(d: &mut vectorcraft_doc::Document, list: &[(String, &Swatch)], links: &HashMap<String, String>) {
    if list.is_empty() {
        return;
    }
    let paints: HashMap<&str, Paint> = list.iter().map(|(name, w)| (name.as_str(), relinked(&w.paint, links))).collect();
    for x in d.swatches_iter_mut() {
        if let Some(paint) = paints.get(x.name.as_str()) {
            x.paint = paint.clone();
        }
    }
}

/// Whether paint `a` shows what paint `b` shows: the same color, or the same gradient with the
/// colors its stops show, whichever swatches those colors link to.
fn shows(a: &Paint, b: &Paint) -> bool {
    let unlinked = |p: &Paint| match p {
        Paint::Gradient(g) => {
            let mut g = g.clone();
            g.swatch = None;
            g.gradient.stops.iter_mut().for_each(|s| s.swatch = None);
            Paint::Gradient(g)
        }
        other => other.clone(),
    };
    match (a.color(), b.color()) {
        (Some(x), Some(y)) => x == y,
        _ => unlinked(a) == unlinked(b),
    }
}

/// The names of the swatches the colors of paint `p` link to: a tint's base color, or the colors
/// of a gradient's stops.
fn linked_names(p: &Paint) -> Vec<&str> {
    match p {
        Paint::Solid { swatch: Some(b), .. } => vec![b.as_str()],
        Paint::Gradient(g) => g.gradient.stops.iter().filter_map(|s| s.swatch.as_deref()).collect(),
        _ => vec![],
    }
}

/// Paint `p` with each link to a swatch that `links` names changed to the name it maps to.
fn relinked(p: &Paint, links: &HashMap<String, String>) -> Paint {
    let mut out = p.clone();
    let swap = |name: &mut String| {
        if let Some(to) = links.get(name.as_str()) {
            *name = to.clone();
        }
    };
    match &mut out {
        Paint::Solid { swatch: Some(b), .. } => swap(b),
        Paint::Gradient(g) => g.gradient.stops.iter_mut().filter_map(|s| s.swatch.as_mut()).for_each(swap),
        _ => {}
    }
    out
}

/// Whether `links` names every swatch the colors of paint `p` link to, the built-in Registration
/// swatch aside (every document has it under its own name).
fn all_linked(p: &Paint, links: &HashMap<String, String>) -> bool {
    linked_names(p).iter().all(|n| *n == REGISTRATION || links.contains_key(*n))
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "swatch.library.add";
    let (info, lib) = library_param(s, p, C)?;
    let apply = match text_param(p, "apply", C)? {
        None => None,
        Some("fill") => Some("paint.setFill"),
        Some("stroke") => Some("paint.setStroke"),
        Some(other) => return Err(bad(C, format!("apply is \"fill\" or \"stroke\", not `{other}`"))),
    };
    let focus = switch_or(p, "focus", true, C)?;
    switch_or(p, "defaultsOnly", false, C)?;
    let picked = picked(&lib, &names_param(p, C)?, C)?;
    // Swatches the document already has (same name and paint; for a pattern swatch, a swatch of the
    // same pattern) are reused, not added again. The patterns are in a built-in pattern library's
    // document, or in the document a loaded library was read from.
    let patterns = patternlib::library_doc(&info.id).or_else(|| s.swatch_libraries.pattern_doc(&info.id));
    let d = &s.doc()?.doc;
    // The document's swatches by name (the first of a name wins, as in `Document::swatch`), looked
    // up once so that a large palette takes linear time.
    let mut by_name: std::collections::HashMap<&str, &Swatch> = std::collections::HashMap::new();
    // The document's numbered copies ("Ink 2") by the name they are numbered after, in order.
    let mut copies: HashMap<&str, Vec<&Swatch>> = HashMap::new();
    for w in d.swatches_iter() {
        let std::collections::hash_map::Entry::Vacant(e) = by_name.entry(w.name.as_str()) else { continue };
        e.insert(w);
        if let Some((base, n)) = w.name.rsplit_once(' ')
            && !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
        {
            copies.entry(base).or_default().push(w);
        }
    }
    // The library's swatches by name (the first of a name wins), looked up once.
    let mut lib_by_name: HashMap<&str, &Swatch> = HashMap::new();
    for w in lib.swatches.iter().chain(lib.groups.iter().flat_map(|g| &g.swatches)) {
        lib_by_name.entry(w.name.as_str()).or_insert(w);
    }
    let group_names: HashSet<&str> = d.swatch_groups.iter().map(|g| g.name.as_str()).collect();
    // A color whose name the document gives another color, or a color group, is the numbered copy
    // of it that an earlier add or a pattern's art brought in, when that copy is the same color. A
    // numbered copy whose name the library gives a swatch of the same paint stands for that swatch
    // alone, as adding that swatch finds it under its own name. Paints are compared with their
    // colors linked as `links` says ([`relinked`]).
    let same_kind = |x: &Swatch, w: &Swatch| x.global == w.global && x.spot == w.spot;
    // A color is the same swatch only as the same kind: a library global color is not the
    // document's process color of the same name and paint, which a tint cannot link to.
    let same_color_kind = |x: &Swatch, w: &Swatch| w.paint.color().is_none() || same_kind(x, w);
    let numbered = |w: &Swatch, paint: &Paint, links: &HashMap<String, String>| {
        let c = copies.get(w.name.as_str())?;
        let own = |x: &Swatch| lib_by_name.get(x.name.as_str()).is_some_and(|l| relinked(&l.paint, links) == x.paint && same_color_kind(x, l));
        let found = c.iter().find(|x| !own(x) && x.paint == *paint && same_kind(x, w));
        found.map(|x| x.name.clone())
    };
    let unlinked = HashMap::new();
    // Whether a library global or spot color that paint `p` links to has no place in `links`
    // while the document gives its name another swatch, which such a link would find instead.
    let misplaced = |p: &Paint, links: &HashMap<String, String>| {
        linked_names(p)
            .into_iter()
            .any(|n| !links.contains_key(n) && by_name.contains_key(n) && lib_by_name.get(n).is_some_and(|b| b.global || b.spot))
    };
    let picked_names: HashSet<&str> = picked.iter().map(|(_, w)| w.name.as_str()).collect();
    // The picked patterns the document already has, each probed once: library pattern name → its
    // name in the document and the colors of the document its art takes under other names.
    let mut present: HashMap<&str, Option<(String, patternlib::SwatchNames)>> = HashMap::new();
    if let Some(src) = &patterns {
        for (_, w) in &picked {
            let Paint::Pattern { pattern, .. } = &w.paint else { continue };
            present.entry(pattern.as_str()).or_insert_with(|| patternlib::present_pattern(d, src, pattern));
        }
    }
    // Whether the document's swatch `x` of the name of library swatch `w` is `w` with its colors
    // linked to swatches of the document → the library's global and spot colors to bring back,
    // those this call picks included.
    // `x` shows `w`'s paint with each color linked to: the name `links` gives it; its own name when
    // that is the document's color of the library color's paint and kind; or, for a library color
    // the document has under no name, that color's own name, which then comes in under it.
    // Registration and colors the library does not have keep their names.
    let fit = |x: &Swatch, w: &Swatch, links: &HashMap<String, String>| -> Option<Vec<String>> {
        let (ours, theirs) = (linked_names(&w.paint), linked_names(&x.paint));
        if ours.len() != theirs.len() {
            return None;
        }
        let map: HashMap<String, String> = ours.iter().zip(&theirs).map(|(l, t)| (l.to_string(), t.to_string())).collect();
        if relinked(&w.paint, &map) != x.paint {
            return None;
        }
        let mut restore = vec![];
        for (&l, &t) in ours.iter().zip(&theirs) {
            let ok = match (links.get(l), lib_by_name.get(l).filter(|b| b.global || b.spot)) {
                (Some(to), _) => to == t,
                (None, None) => l == t,
                (None, Some(b)) => match by_name.get(t) {
                    Some(y) => l == t && y.paint == b.paint && same_kind(y, b),
                    None if l == t && !group_names.contains(t) => {
                        restore.push(l.to_string());
                        true
                    }
                    None => false,
                },
            };
            if !ok {
                return None;
            }
        }
        Some(restore)
    };
    // `paint` is `w`'s paint with the colors it links to as `links` has them in the document → the
    // name of the document's swatch, and the library colors to bring back for it ([`fit`]).
    let there = |w: &Swatch, paint: &Paint, links: &HashMap<String, String>| -> Option<(String, Vec<String>)> {
        if let (Paint::Pattern { pattern, .. }, Some(_)) = (&w.paint, &patterns) {
            let (target, _) = present.get(pattern.as_str())?.as_ref()?;
            return library_pattern_swatch(d, w, target).map(|n| (n, vec![]));
        }
        if let Some(restore) = by_name.get(w.name.as_str()).filter(|x| same_color_kind(x, w)).and_then(|x| fit(x, w, links)) {
            return Some((w.name.clone(), restore));
        }
        if misplaced(&w.paint, links) {
            return None;
        }
        let found = match by_name.get(w.name.as_str()) {
            Some(_) => numbered(w, paint, links),
            None if group_names.contains(w.name.as_str()) => numbered(w, paint, links),
            None => (w.name == REGISTRATION && d.swatch(&w.name).is_some_and(|x| x.paint == *paint)).then(|| w.name.clone()),
        };
        // A numbered copy brings back the colors it links to that the document deleted, as a
        // swatch of `w`'s own name does.
        found.map(|n| {
            let back = by_name.get(n.as_str()).and_then(|x| fit(x, w, links)).unwrap_or_default();
            (n, back)
        })
    };
    // The global or spot color of the document that library color `b` is: its own name, or the
    // numbered copy of it that adding `b` would find. While no color or color group has its name,
    // a numbered copy is looked for only when this call does not add `b` under its own name.
    let base_in_doc = |b: &Swatch| match by_name.get(b.name.as_str()) {
        Some(x) if x.paint == b.paint && same_kind(x, b) => Some(b.name.clone()),
        Some(_) => numbered(b, &b.paint, &unlinked),
        None if group_names.contains(b.name.as_str()) || !picked_names.contains(b.name.as_str()) => numbered(b, &b.paint, &unlinked),
        None => None,
    };
    // The library's global and spot colors that the picked tints and gradients link to, with the
    // name each has in the document: library name → document name. Each is looked up for itself,
    // whether or not this call picks it, once.
    let mut base_names: HashMap<String, String> = HashMap::new();
    let mut looked_up: HashSet<&str> = HashSet::new();
    for (_, w) in &picked {
        for base in linked_names(&w.paint) {
            if !looked_up.insert(base) {
                continue;
            }
            if let Some(n) = lib_by_name.get(base).filter(|b| b.global || b.spot).and_then(|b| base_in_doc(b)) {
                base_names.insert(base.to_string(), n);
            }
        }
    }
    // A color that the art of a picked pattern the document holds takes under another name (a
    // numbered copy that the library also names) is the one the picked tints and gradients link
    // to, as when the pattern came in, unless this call adds the color itself. The colors the art
    // takes are those of an import into a copy of the document, which adds nothing.
    if looked_up.iter().any(|n| !base_names.contains_key(*n)) {
        for (_, w) in &picked {
            let Paint::Pattern { pattern, .. } = &w.paint else { continue };
            let Some(Some((_, took))) = present.get(pattern.as_str()) else { continue };
            for (from, to) in took.iter().cloned() {
                let fits = |b: &&Swatch| by_name.get(to.as_str()).is_some_and(|x| (x.global || x.spot) && x.paint == b.paint && same_kind(x, b));
                if looked_up.contains(from.as_str()) && !picked_names.contains(from.as_str()) && lib_by_name.get(from.as_str()).is_some_and(fits) {
                    base_names.entry(from).or_insert(to);
                }
            }
        }
    }
    // Library name → the name of the document's swatch.
    let mut existing: Vec<(String, String)> = vec![];
    let mut fresh = vec![];
    // The library colors that the document's tints and gradients found here link to by name while
    // the document has no swatch of that name (one deleted without unlinking them): they come back
    // under their own names.
    let mut restore: Vec<String> = vec![];
    // Those that this call picks: they are added first, so that they keep their names, which the
    // art of a pattern added before them could otherwise take.
    let mut reserved: HashSet<String> = HashSet::new();
    for &(group, w) in &picked {
        // A tint or gradient is compared with its colors linked to the names they have here.
        let paint = relinked(&w.paint, &base_names);
        match there(w, &paint, &base_names) {
            Some((name, back)) => {
                existing.push((w.name.clone(), name));
                for n in back {
                    if picked_names.contains(n.as_str()) {
                        reserved.insert(n);
                    } else if !restore.contains(&n) {
                        restore.push(n);
                    }
                }
            }
            None => fresh.push((group, w)),
        }
    }
    if !reserved.is_empty() {
        let (first, rest): (Vec<_>, Vec<_>) = fresh.into_iter().partition(|(_, w)| reserved.contains(&w.name));
        fresh = first;
        fresh.extend(rest);
    }
    let undo_before = s.doc()?.history.undo.len();
    // Library name → the name it got in the document. The colors the patterns brought that no
    // library swatch of this call stands for are added too; they are listed after the others and
    // are never the one `apply` paints with.
    let mut brought_alone: Vec<(String, String)> = vec![];
    // Library name → the name of the document's global or spot color for it, which the tints and
    // gradients added here link to: one that is global or spot as the library color is.
    let base_of = |l: &str, n: &str| {
        let (Some(w), Some(x)) = (lib_by_name.get(l), by_name.get(n)) else { return false };
        (x.global || x.spot) && same_kind(x, w)
    };
    let mut links: HashMap<String, String> = existing.iter().filter(|(l, n)| base_of(l, n)).cloned().collect();
    links.extend(base_names);
    let added: Vec<(String, String)> = if fresh.is_empty() && restore.is_empty() {
        vec![]
    } else {
        let (added, reused, brought) = s.edit("Add to Swatches", |d, _| {
            let mut out = vec![];
            // Library pattern swatches that stand for a swatch added earlier in this call.
            let mut reused = vec![];
            // Each library group goes into one document group: the group of that name the document
            // already had, or one made here.
            let before = d.swatch_groups.len();
            let mut placed: Vec<(&str, usize)> = vec![];
            let mut taken = taken_names(d);
            // The swatches the patterns imported in this call brought with their art: (library
            // name, document name).
            let mut brought: Vec<(String, String)> = vec![];
            // The colors to bring back come first, under their own names, which no swatch or color
            // group of the document has; the tints and gradients added here link to them too.
            for n in &restore {
                let Some(b) = lib_by_name.get(n.as_str()) else { continue };
                let name = free_in(&mut taken, &b.name);
                d.swatches.push(Swatch { name: name.clone(), ..(*b).clone() });
                links.insert(n.clone(), name.clone());
                brought.push((n.clone(), name));
            }
            // The swatches added here with a link `links` did not name yet: (document name, library
            // swatch).
            let mut pending: Vec<(String, &Swatch)> = vec![];
            for (group, w) in fresh {
                let (name, sw) = if let (Paint::Pattern { pattern, .. }, Some(src)) = (&w.paint, &patterns) {
                    // The pattern comes with its swatch; a pattern the document has without one
                    // gets one. The swatch a pattern that comes in brings is named after the pattern:
                    // it takes the library swatch's name and color group instead.
                    // The tints and gradients added so far link to their colors first, so that the
                    // pattern's art finds them. A color they wait for that the pattern's art brings
                    // is named by importing the pattern into a copy first.
                    let brings = if pending.is_empty() { Default::default() } else { patternlib::brought_names(src, pattern) };
                    let waited = |n: &&str| !links.contains_key(*n) && brings.contains(*n);
                    // A global or spot color the pattern brought, or took from the document under
                    // a numbered name: library name → document name. A color this call picks is
                    // added under its own name instead of the one the pattern took. A library global
                    // or spot color is only a document color of its paint and kind, as for `base_of`.
                    let colors_of = |d: &vectorcraft_doc::Document, colors: &[(String, String)], took: &[(String, String)]| {
                        let mut out: HashMap<String, String> = HashMap::new();
                        for (from, to) in colors.iter().chain(took.iter().filter(|(from, _)| !picked_names.contains(from.as_str()))) {
                            let library = lib_by_name.get(from.as_str()).filter(|b| b.global || b.spot);
                            let fits = |x: &Swatch| (x.global || x.spot) && library.is_none_or(|b| x.paint == b.paint && same_kind(x, b));
                            if d.swatch(to).is_some_and(fits) {
                                out.entry(from.clone()).or_insert_with(|| to.clone());
                            }
                        }
                        out
                    };
                    // The names learned from the copy, which the import into the document confirms.
                    let mut learned = vec![];
                    if pending.iter().any(|(_, w)| linked_names(&w.paint).iter().any(waited)) {
                        let mut probe = d.clone();
                        let (_, colors, took) = import_holding(&mut probe, src, pattern, &lib_by_name)?;
                        for (from, to) in colors_of(&probe, &colors, &took) {
                            if let std::collections::hash_map::Entry::Vacant(e) = links.entry(from.clone()) {
                                e.insert(to);
                                learned.push(from);
                            }
                        }
                    }
                    let settled = settle(d, &mut pending, &links);
                    let had = taken_names(d);
                    let (target, colors, took) = import_holding(d, src, pattern, &lib_by_name)?;
                    // Those colors are the ones the library's tints of them link to, unless the
                    // document already has one for them. A name the copy gave that the import
                    // into the document did not give is replaced, and the tints linked to it are
                    // linked again.
                    let mut found = colors_of(d, &colors, &took);
                    let mut changed = false;
                    for from in learned {
                        let real = found.remove(&from);
                        if real.as_ref() != links.get(&from) {
                            changed = true;
                            match real {
                                Some(to) => links.insert(from, to),
                                None => links.remove(&from),
                            };
                        }
                    }
                    for (from, to) in found {
                        links.entry(from).or_insert(to);
                    }
                    if changed {
                        relink_in(d, &settled, &links);
                        pending.extend(settled.into_iter().filter(|(_, w)| !all_linked(&w.paint, &links)));
                    }
                    brought.extend(colors);
                    if let Some(name) = library_pattern_swatch(d, w, &target).filter(|n| had.contains(n)) {
                        reused.push((w.name.clone(), name));
                        taken = taken_names(d);
                        continue;
                    }
                    let made = patternlib::pattern_swatch(d, &target).filter(|n| !had.contains(n));
                    if let Some(made) = &made {
                        d.swatches.retain(|x| x.name != *made);
                    }
                    taken = taken_names(d);
                    let name = match made {
                        Some(made) if w.name == *pattern => made,
                        None if w.name == *pattern => d.free_swatch_name(&target),
                        _ => free_in(&mut taken, &w.name),
                    };
                    taken.insert(name.clone());
                    // The library swatch's placement of its pattern is kept.
                    let paint = match &w.paint {
                        Paint::Pattern { xf, .. } => Paint::Pattern { pattern: target, xf: *xf },
                        _ => pattern_paint(&target),
                    };
                    (name.clone(), Swatch { name, paint, global: false, spot: false })
                } else {
                    // A color a pattern added earlier in this call brought with its art is that
                    // swatch when it shows the library swatch's color: it moves to the library
                    // swatch's place, when its colors link where the library swatch's do: to the
                    // names `links` gives them, not elsewhere for a color this call has yet to add,
                    // and otherwise to a color of the library color's paint and process, global or spot kind,
                    // or under the same name when the library has no such swatch (Registration).
                    let fits = |x: &Paint| {
                        let (ours, theirs) = (linked_names(&w.paint), linked_names(x));
                        ours.len() == theirs.len()
                            && ours.iter().zip(&theirs).all(|(l, t)| match links.get(*l) {
                                Some(to) => to == t,
                                None => {
                                    !picked_names.contains(l)
                                        && match lib_by_name.get(*l) {
                                            Some(b) => d.swatch(t).is_some_and(|x| x.paint == b.paint && same_kind(x, b)),
                                            None => l == t,
                                        }
                                }
                            })
                    };
                    let same = |to: &str| {
                        d.swatch(to).is_some_and(|x| shows(&x.paint, &w.paint) && x.global == w.global && x.spot == w.spot && fits(&x.paint))
                    };
                    let at = brought.iter().position(|(from, to)| *from == w.name && same(to));
                    let moved = at.map(|i| brought.swap_remove(i).1).and_then(|n| {
                        let i = d.swatches.iter().position(|x| x.name == n)?;
                        Some(d.swatches.remove(i))
                    });
                    match moved {
                        Some(sw) => (sw.name.clone(), sw),
                        None => {
                            // A numbered copy skips the names of the library's other swatches,
                            // which a later add looks up as theirs.
                            let name = free_copy_in(&mut taken, &w.name, |n| lib_by_name.contains_key(n));
                            // A tint or gradient links to its colors under the names they have in
                            // the document; a color that comes in later in this call is linked
                            // when it has come in.
                            let paint = relinked(&w.paint, &links);
                            if !all_linked(&w.paint, &links) {
                                pending.push((name.clone(), w));
                            }
                            (name.clone(), Swatch { name, paint, ..(*w).clone() })
                        }
                    }
                };
                if sw.global || sw.spot {
                    links.insert(w.name.clone(), name.clone());
                }
                match group {
                    Some(g) => {
                        let found = placed.iter().find(|(n, _)| *n == g).map(|&(_, i)| i);
                        let i = match found.or_else(|| d.swatch_groups.iter().take(before).position(|x| x.name == *g)) {
                            Some(i) => i,
                            None => {
                                let name = free_in(&mut taken, g);
                                d.swatch_groups.push(SwatchGroup { name, swatches: vec![] });
                                d.swatch_groups.len() - 1
                            }
                        };
                        if found.is_none() {
                            placed.push((g, i));
                        }
                        d.swatch_groups[i].swatches.push(sw);
                    }
                    None => d.swatches.push(sw),
                }
                out.push((w.name.clone(), name));
            }
            // A library global or spot color that a tint or gradient added here links to, which the
            // document does not have and nothing in this call brought, comes in with it, as pasting
            // art painted with the tint brings its color. It is listed with the colors patterns
            // brought.
            let mut wanted: Vec<&str> = vec![];
            let mut seen: HashSet<&str> = HashSet::new();
            for (_, w) in &pending {
                for n in linked_names(&w.paint) {
                    if n != REGISTRATION && !links.contains_key(n) && seen.insert(n) {
                        wanted.push(n);
                    }
                }
            }
            for n in wanted {
                let Some(b) = lib_by_name.get(n).filter(|b| b.global || b.spot) else { continue };
                let name = free_copy_in(&mut taken, &b.name, |x| lib_by_name.contains_key(x));
                d.swatches.push(Swatch { name: name.clone(), ..(*b).clone() });
                links.insert(n.to_string(), name.clone());
                brought.push((n.to_string(), name));
            }
            relink_in(d, &pending, &links);
            Ok((out, reused, brought))
        })?;
        existing.extend(reused);
        brought_alone = brought;
        added
    };
    let names = |list: &[(String, String)]| list.iter().map(|(_, n)| n.clone()).collect::<Vec<_>>();
    let mut added_names = names(&added);
    added_names.extend(names(&brought_alone));
    let mut out = json!({"library": shown_library_id(s, &info.id), "added": added_names, "existing": names(&existing)});
    if let (Some(cmd), Some((_, first))) = (apply, picked.first()) {
        let name = added.iter().chain(&existing).find(|(l, _)| *l == first.name).map_or(first.name.clone(), |(_, n)| n.clone());
        let only = super::paint::defaults_only(s, p, C)?;
        s.execute(cmd, &super::paint::with_defaults_only(json!({"swatch": name, "focus": focus}), only))?;
        // Adding and applying undo together.
        squash(s, undo_before, "Add to Swatches");
        out["applied"] = json!(name);
    }
    Ok(out)
}

fn reset_defaults(s: &mut Session, p: &Value) -> Result<Value> {
    let replace = switch_or(p, "replace", false, "swatch.resetDefaults")?;
    let model = s.doc()?.doc.color_mode.model();
    let (defaults, groups) = default_swatches(model);
    // With replace, links to swatches that are no longer global ones are dropped (the art keeps
    // its colour); filled in by the edit.
    let mut live: Vec<String> = vec![];
    let colorized_ok = super::paint::check_colorized(&s.doc()?.doc).is_ok();
    let added = s.edit("Default Swatches", |d, _| {
        let mut added = vec![];
        if replace {
            let before: Vec<String> = d.swatches_iter().map(|w| w.name.clone()).collect();
            (d.swatches, d.swatch_groups) = (defaults.into(), groups.into());
            added.extend(d.swatches_iter().filter(|w| !before.contains(&w.name)).map(|w| w.name.clone()));
            live.extend(d.swatches_iter().filter(|w| w.global).map(|w| w.name.clone()));
            map_swatch_links(d, &mut |_, l, _| unlink_dead(&live, l));
            super::paint::check_colorized_kept(colorized_ok, d).map_err(|e| bad("swatch.resetDefaults", e))?;
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
        map_default_paints(s, &mut |_, l, _| unlink_dead(&live, l))?;
    }
    Ok(json!({ "added": added }))
}

/// Drop a link to a swatch not in `live`; true when it did.
fn unlink_dead(live: &[String], link: &mut Option<String>) -> bool {
    link.as_ref().is_some_and(|n| !live.contains(n)) && link.take().is_some()
}
