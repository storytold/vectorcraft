//! File → Package: a saved document copied into a folder of its own with the files its linked
//! images show (`Links/`), the fonts its type uses (`Fonts/`, leaving out fonts whose licence
//! doesn't allow embedding) and a text report (the package's contents, then the Document Info
//! report). The packaged document links to the copies (Relink); the open document doesn't change.
//! Without a folder (the web, agents) the same files come back as a zip archive.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use vectorcraft_doc::links::hash_bytes;
use vectorcraft_doc::{Document, LinkInfo};

use serde_json::{Value, json};

use super::fileio::{create_dir, write_file};
use super::{CommandSpec, Result, Session, bad, bool_or, cmd, has_doc, str_param, unique_name};

const C: &str = "file.package";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.package",
        "Package",
        [],
        None,
        "{folder?, name?: (default: \"<document> Folder\"), copyLinks?: true, linksFolder?: true (the linked files go in Links/, else next to the document), relink?: true (the packaged document links to the copies), copyFonts?: true (the fonts its type uses go in Fonts/; fonts whose licence doesn't allow embedding are left out), report?: true (<document> Report.txt)} copy the saved document as it is now into folder/name as <document>.vectorcraft with its linked files and fonts; the open document doesn't change → {folder, files: [paths relative to folder/name], links, fonts (counts copied), missingLinks: [file names not found], skippedFonts: [{font, reason}], warnings: [nested files copied unchanged or cycles/depth stopped]}; no folder → the same with {name: \"<name>.zip\", dataBase64} (a zip of the files in <name>/) instead of folder",
        has_doc,
        package
    )]
}

/// A file of the package: its path in the package folder (`/` separators) and bytes.
type Entry = (String, Vec<u8>);

/// `name`, else the first free `stem 2.ext`, `stem 3.ext`… (compared without case, as most
/// desktop file systems do); the name is taken.
pub(crate) fn free_name(name: &str, taken: &mut HashSet<String>) -> String {
    let p = Path::new(name);
    let stem = p.file_stem().map_or_else(|| name.to_string(), |s| s.to_string_lossy().into_owned());
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    let out = unique_name(&stem, |s| taken.contains(&format!("{s}{ext}").to_lowercase())) + &ext;
    taken.insert(out.to_lowercase());
    out
}

/// A font file's extension from its first bytes.
fn font_ext(bytes: &[u8]) -> &'static str {
    match bytes.get(..4) {
        Some(b"OTTO") => "otf",
        Some(b"ttcf") => "ttc",
        _ => "ttf",
    }
}

/// The name a font's file gets in the package's `Fonts` folder: the name of the file at `path`,
/// with the extension the font's `data` calls for ([`font_ext`]) when that name has no font
/// extension; without a file, `family-style` with that extension.
fn packaged_name(path: Option<&Path>, family: &str, style: &str, data: &[u8]) -> String {
    match path.and_then(Path::file_name).map(|n| n.to_string_lossy().into_owned()) {
        Some(name) if vectorcraft_text::is_font_file(Path::new(&name)) => name,
        Some(name) => format!("{name}.{}", font_ext(data)),
        None => format!("{family}-{style}.{}", font_ext(data)).replace(|c: char| !(c.is_alphanumeric() || "-_.".contains(c)), ""),
    }
}

/// The options of a package.
struct Options {
    copy_links: bool,
    links_folder: bool,
    relink: bool,
    copy_fonts: bool,
    report: bool,
}

/// An asset that has been copied to the package, including rewritten linked documents.
#[derive(Clone)]
struct Copied {
    name: String,
    size: Option<u64>,
    hash: Option<String>,
}

/// The parent of a packaged document and its asset both live inside the package.
/// Use paths relative to that *document*, not the package's root: a linked document
/// in Links/ must reference its own sibling image as "image.png".
fn relative_in_package(document: &str, asset: &str) -> String {
    let parent = document.rsplit_once('/').map_or("", |(dir, _)| dir);
    let a: Vec<&str> = parent.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = asset.split('/').filter(|s| !s.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(a, b)| a == b).count();
    let mut out = vec![".."; a.len() - common];
    out.extend_from_slice(&b[common..]);
    out.join("/")
}

fn safe_asset_name(name: &str) -> String {
    let name: String =
        name.chars().map(|c| if c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let name = name.trim().trim_matches('.');
    if name.is_empty() { "asset".into() } else { name.into() }
}

/// A file placed: (page, bounding, page box) of each placed object showing it.
type PlacedPages = Vec<(u32, bool, vectorcraft_doc::PageBox)>;

#[cfg(test)]
thread_local! {
    /// Tests: how many placed PDF or `.ai` pages [`page_fonts`] read on this thread.
    pub(crate) static PAGES_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Tests: how many linked `.vectorcraft` files had their preview and PDF read on this thread,
    /// to keep them in the rewritten copy.
    pub(crate) static PARTS_READ: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The fonts of the type on `pages` of the PDF or `.ai` file `name` (`bytes`, found at `path`);
/// none for other files.
fn page_fonts(name: &str, bytes: &[u8], path: &str, pages: &PlacedPages) -> BTreeSet<super::fonts::UsedFont> {
    let mut fonts = BTreeSet::new();
    if super::place::document::is_pdf(name, bytes) {
        for &(page, bounding, page_box) in pages {
            #[cfg(test)]
            PAGES_READ.with(|c| c.set(c.get() + 1));
            if let Ok(a) = super::place::document::document_art(name, bytes, Some(path), page, bounding, page_box, C) {
                fonts.extend(super::fonts::art_fonts(&a.doc, &a.nodes));
            }
        }
    }
    fonts
}

/// Collect dependencies depth-first so nested VectorCraft files are rewritten before
/// their parents: each parent can record the final hash of the rewritten child.
/// Cycles, excessive nesting, and unreadable children are reported, not fatal.
struct Collector<'a> {
    opts: &'a Options,
    root: Option<PathBuf>,
    entries: Vec<Entry>,
    copies: BTreeMap<String, Copied>,
    visiting: BTreeMap<String, String>,
    taken: HashSet<String>,
    link_names: HashSet<String>,
    font_files: HashSet<String>,
    fonts: usize,
    missing: Vec<String>,
    skipped: Vec<Value>,
    warnings: Vec<String>,
    /// The report's lines, one section per document: the packaged one first, then each placed
    /// document in the order it is reached.
    sections: Vec<Vec<String>>,
}

impl<'a> Collector<'a> {
    fn new(opts: &'a Options, root: Option<PathBuf>, doc_name: &str) -> Self {
        let mut taken = HashSet::new();
        taken.insert(doc_name.to_lowercase());
        Self {
            opts,
            root,
            entries: Vec::new(),
            copies: BTreeMap::new(),
            visiting: BTreeMap::new(),
            taken,
            link_names: HashSet::new(),
            font_files: HashSet::new(),
            fonts: 0,
            missing: Vec::new(),
            skipped: Vec::new(),
            warnings: Vec::new(),
            sections: Vec::new(),
        }
    }

    fn warn(&mut self, lines: &mut Vec<String>, message: String) {
        lines.push(format!("WARNING: {message}"));
        self.warnings.push(message);
    }

    /// Warns when a file that placed `.ai` file `source` links (`name`) was copied as `copy`: the
    /// copy of `source` finds it only by its name, beside it.
    fn renamed(&mut self, lines: &mut Vec<String>, source: &str, path: &str, name: &str, copy: &str) {
        if Path::new(copy).file_name().and_then(std::ffi::OsStr::to_str) != Some(name) {
            let message = format!(
                "{path}: copied as {copy}; the copy of {source} looks for it as {name} and shows the file of that name, if any, else its preview"
            );
            self.warn(lines, message);
        }
    }

    /// Collects the files `doc` links. `by_name`: `doc` is a placed `.ai` file's editing data,
    /// whose links are not rewritten, so its copy finds them by name beside it.
    fn collect(&mut self, doc: &mut Document, source: &str, destination: &str, depth: usize, by_name: bool) -> Result<()> {
        // The caller checks cycles and depth before descending; active documents have
        // a destination reserved so back-links can point at their single packaged copy.
        self.visiting.insert(source.to_string(), destination.to_string());
        // This document's section comes before those of the documents it places.
        let section = self.sections.len();
        self.sections.push(vec![]);
        let mut lines = vec![];
        // The packaged document's sections read as before; a placed document's name its file.
        let heading = |lines: &mut Vec<String>, title: &str| {
            lines.push(title.into());
            if depth > 0 {
                lines.push(format!("Document: {source}"));
            }
        };
        // A file's saved path is only meaningful inside its own document.
        // Different linked documents can reuse the same stale absolute path yet
        // resolve it to distinct local assets. Keep a mapping per document.
        let mut local = BTreeMap::<String, Copied>::new();
        // The fonts of the type on the PDF and `.ai` pages placed linked.
        let mut placed_fonts = BTreeSet::new();
        let mut placed = BTreeMap::<String, PlacedPages>::new();
        doc.visit_placed(|_, p| {
            let page = (p.link.page.unwrap_or(1), p.bounding, p.page_box);
            let pages = placed.entry(p.link.path.clone()).or_default();
            if !pages.contains(&page) {
                pages.push(page);
            }
        });
        if self.opts.copy_fonts && !self.opts.copy_links && !placed.is_empty() {
            for file in super::links::linked_files(doc, Some(source)) {
                if let (Some(pages), Some(bytes)) = (placed.get(&file.path), &file.bytes) {
                    placed_fonts.extend(page_fonts(&file.name, bytes, file.found_path.as_deref().unwrap_or(&file.path), pages));
                }
            }
        }
        if self.opts.copy_links {
            heading(&mut lines, "LINKED FILES");
            for file in super::links::linked_files(doc, Some(source)) {
                let found_path = file.found_path.as_deref().unwrap_or(&file.path);
                if let Some(ancestor) = self.visiting.get(found_path).cloned() {
                    // The ancestor is already included (possibly the root document). A
                    // cyclic reference cannot have a final content hash until both files
                    // have been written, so leave its size/hash unspecified, not stale.
                    self.warn(&mut lines, format!("{}: circular placed-document link to {ancestor}; stopped following the cycle", file.path));
                    local.insert(file.path.clone(), Copied { name: ancestor, size: None, hash: None });
                    continue;
                }
                if let Some(existing) = self.copies.get(found_path).cloned() {
                    if by_name {
                        self.renamed(&mut lines, source, &file.path, &file.name, &existing.name);
                    }
                    // Another document may place other pages of a PDF copied already.
                    if self.opts.copy_fonts
                        && let (Some(pages), Some(bytes)) = (placed.get(&file.path), &file.bytes)
                    {
                        placed_fonts.extend(page_fonts(&file.name, bytes, found_path, pages));
                    }
                    local.insert(file.path.clone(), existing);
                    continue;
                }
                let Some(mut bytes) = file.bytes else {
                    lines.push(format!("{}: not found ({}), not copied", file.name, file.path));
                    self.missing.push(if depth == 0 { file.name.clone() } else { format!("{} ({source})", file.name) });
                    continue;
                };
                if self.copies.len() >= 10_000 {
                    return Err(bad(C, "more than 10000 linked files: package dependencies in smaller parts"));
                }
                let dir = if self.opts.links_folder { "Links/" } else { "" };
                let name = safe_asset_name(&file.name);
                let unique = free_name(&name, if self.opts.links_folder { &mut self.link_names } else { &mut self.taken });
                let destination_file = format!("{dir}{unique}");
                if by_name {
                    self.renamed(&mut lines, source, &file.path, &file.name, &destination_file);
                }
                // A placed PDF or .ai file is copied as it is, as an image is; the files a .ai file
                // read from its editing data links are copied beside it.
                let pages = placed.get(&file.path);
                let placed_file = pages.is_some();
                if placed_file && let Some(mut art) = super::place::document::editing_data(&file.name, &bytes) {
                    if depth >= vectorcraft_doc::placed_document::MAX_DEPTH {
                        self.warn(&mut lines, format!("{}: nesting limit reached; copied unchanged without collecting its own links", file.path));
                    } else {
                        self.collect(&mut art, found_path, &destination_file, depth + 1, true)?;
                    }
                } else if self.opts.copy_fonts
                    && let Some(pages) = pages
                {
                    placed_fonts.extend(page_fonts(&file.name, &bytes, found_path, pages));
                }
                if placed_file && super::place::document::is_native(&file.name, &bytes) {
                    if depth >= vectorcraft_doc::placed_document::MAX_DEPTH {
                        self.warn(&mut lines, format!("{}: nesting limit reached; copied unchanged without collecting its own links", file.path));
                    } else {
                        match vectorcraft_format::load_file(&bytes) {
                            Ok(saved) => {
                                let compressed = bytes.starts_with(&[0x1f, 0x8b]);
                                let include_linked =
                                    saved.doc.linked_only_images().iter().any(|key| saved.doc.images.get(key).is_some_and(|blob| !blob.is_proxy()));
                                let mut nested = saved.doc;
                                self.collect(&mut nested, found_path, &destination_file, depth + 1, false)?;
                                if self.opts.relink {
                                    #[cfg(test)]
                                    PARTS_READ.with(|c| c.set(c.get() + 1));
                                    // Preserve auxiliary native-file content when rewriting.
                                    let mut options = vectorcraft_format::SaveOptions::for_doc(&nested);
                                    options.preview = vectorcraft_format::preview(&bytes);
                                    options.pdf = vectorcraft_format::pdf_content(&bytes);
                                    options.profiles = saved.profiles;
                                    options.compress = compressed;
                                    options.include_linked = include_linked;
                                    match vectorcraft_format::save_with(&nested, &options) {
                                        Ok(rewritten) => bytes = rewritten,
                                        Err(e) => self.warn(
                                            &mut lines,
                                            format!(
                                                "{}: could not rewrite linked document ({e}); copied unchanged, its own links were not relinked",
                                                file.path
                                            ),
                                        ),
                                    }
                                }
                            }
                            Err(e) => self.warn(
                                &mut lines,
                                format!("{}: unreadable linked document ({e}); copied unchanged, its own links were not collected", file.path),
                            ),
                        }
                    }
                }
                let copy = Copied { name: destination_file.clone(), size: Some(bytes.len() as u64), hash: Some(hash_bytes(&bytes)) };
                self.copies.insert(found_path.to_string(), copy.clone());
                local.insert(file.path.clone(), copy);
                lines.push(format!("{} → {destination_file}", file.path));
                self.entries.push((destination_file, bytes));
            }
            lines.push(String::new());
        }
        if self.opts.copy_fonts {
            let db = vectorcraft_text::FontDb::global();
            heading(&mut lines, "FONTS");
            let mut used_fonts = super::fonts::used_fonts(doc);
            used_fonts.append(&mut placed_fonts);
            for used in used_fonts {
                let font = super::fonts::font_label(&used);
                let (family, style, version) = used;
                // Found by any of its names, as the canvas draws it (in the version the type names).
                let resolved = db
                    .resolve(&family, &style)
                    .filter(|(_, m)| *m != vectorcraft_text::FontMatch::Missing)
                    .map(|(f, m)| (db.face_version(&family, &style, version.as_deref()).unwrap_or(f), m));
                // A style the family lacks is shown in its closest style, whose file is copied.
                let shown = match &resolved {
                    Some((f, vectorcraft_text::FontMatch::Style)) => format!(" (shown in {} {})", f.family, f.style),
                    _ => String::new(),
                };
                let reason = match resolved {
                    Some((f, _)) if f.embeddable() => {
                        let data = f.file_data();
                        let file = packaged_name(f.path(), &f.family, &f.style, data);
                        // A collection, or a style shown in another face's file, is copied once.
                        if self.font_files.insert(file.to_lowercase()) {
                            self.entries.push((format!("Fonts/{file}"), data.to_vec()));
                            self.fonts += 1;
                        }
                        lines.push(format!("{font}{shown} → Fonts/{file}"));
                        continue;
                    }
                    Some(_) => "its licence doesn't allow embedding",
                    None => "not available on this computer",
                };
                lines.push(format!("{font}{shown}: {reason}, not copied"));
                self.skipped.push(json!({ "font": font, "reason": reason }));
            }
            lines.push(String::new());
        }
        if self.opts.relink {
            // The child files have already been rewritten; the hash and size of each
            // target must reflect the *packaged* bytes rather than the original.
            doc.update_links(|link: &mut LinkInfo| {
                if let Some(copy) = local.get(&link.path) {
                    let relative = relative_in_package(destination, &copy.name);
                    link.path = self.root.as_ref().map_or_else(|| copy.name.clone(), |r| r.join(&copy.name).to_string_lossy().into_owned());
                    link.relative = Some(relative);
                    link.size = copy.size;
                    link.hash = copy.hash.clone();
                    link.modified = None;
                }
            });
        }
        if let Some(s) = self.sections.get_mut(section) {
            *s = lines;
        }
        self.visiting.remove(source);
        Ok(())
    }
}

fn package(s: &mut Session, p: &Value) -> Result<Value> {
    let package = prepare(s, p)?;
    match &mut s.package_hand_off {
        Some(slot) => {
            *slot = Some(package);
            Ok(Value::Null)
        }
        None => package.write(&mut |path, bytes| write_file(path, bytes)),
    }
}

/// A package of the active document as `file.package` takes it from the session: the options, the
/// names, the report's Document Info text and the document as it is now
/// ([`Session::prepare_package`]). [`Package::write`] collects and writes it without the session,
/// on any thread.
pub struct Package {
    o: Options,
    doc_path: String,
    stem: String,
    name: String,
    root: Option<PathBuf>,
    info: Option<String>,
    doc: std::sync::Arc<Document>,
}

impl Session {
    /// Run `file.package` with `params` as a command (checked and journaled as any other) that
    /// collects and writes nothing → the package it prepared, which [`Package::write`] collects
    /// and writes (on the desktop with Background Export on, File › Package… does this on the
    /// background thread).
    pub fn prepare_package(&mut self, params: &Value) -> Result<Package> {
        self.package_hand_off = Some(None);
        let r = self.execute(C, params);
        let handed = self.package_hand_off.take().flatten();
        match (r, handed) {
            (Err(e), _) => Err(e),
            (Ok(_), Some(package)) => Ok(package),
            // Every run that succeeds hands one over; otherwise the package is prepared here.
            (Ok(_), None) => prepare(self, params),
        }
    }
}

/// What `file.package` reads from the session: the options, the checks of the document and the
/// name, the report's Document Info text and the document.
fn prepare(s: &mut Session, p: &Value) -> Result<Package> {
    let o = Options {
        copy_links: bool_or(p, "copyLinks", true),
        links_folder: bool_or(p, "linksFolder", true),
        relink: bool_or(p, "relink", true),
        copy_fonts: bool_or(p, "copyFonts", true),
        report: bool_or(p, "report", true),
    };
    let doc_path = s.doc()?.path.clone().ok_or_else(|| bad(C, "save the document first: Package collects a saved document"))?;
    let stem = Path::new(&doc_path).file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let name = str_param(p, "name").map_or_else(|| format!("{stem} Folder"), str::to_string);
    if name.trim().is_empty() || name == "." || name == ".." || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        return Err(bad(C, format!("`{name}` can't name a folder")));
    }
    let folder = str_param(p, "folder");
    let root = folder.map(|f| Path::new(f).join(&name));
    let info = if o.report { Some(super::docinfo::report(s, false)?) } else { None };
    let doc = s.doc()?.doc.clone();
    Ok(Package { o, doc_path, stem, name, root, info, doc })
}

impl Package {
    /// Collect the linked files and fonts and write the package's files with `write` into
    /// `folder/name`, making its folders first, or into the zip without a folder → the reply of
    /// `file.package`.
    pub fn write(self, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
        collect_and_write(self, write)
    }
}

/// [`Package::write`].
fn collect_and_write(package: Package, write: &mut dyn FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
    let Package { o, doc_path, stem, name, root, info, doc } = package;
    let mut doc = (*doc).clone();
    let doc_name = format!("{stem}.{}", vectorcraft_format::EXTENSION);
    // Prepare original links for the destination *before* rebasing copied links.
    // Recalculating afterward would overwrite the portable relative paths.
    if let Some(root) = &root
        && let Some(d) = super::links::with_relative_paths(&doc, &root.join(&doc_name).to_string_lossy())
    {
        doc = d;
    }
    let mut collector = Collector::new(&o, root.clone(), &doc_name);
    collector.collect(&mut doc, &doc_path, &doc_name, 0, false)?;
    let mut entries = std::mem::take(&mut collector.entries);
    entries.insert(0, (doc_name.clone(), vectorcraft_format::save_file(&doc)));
    let mut taken = std::mem::take(&mut collector.taken);
    let lines = collector.sections.concat();
    let missing = std::mem::take(&mut collector.missing);
    let skipped = std::mem::take(&mut collector.skipped);
    let warnings = std::mem::take(&mut collector.warnings);
    let fonts = collector.fonts;
    let links = collector.copies.len();
    let copied_sources: Vec<String> = collector.copies.keys().cloned().collect();

    // The report.
    if let Some(info) = info {
        let report_name = free_name(&format!("{stem} Report.txt"), &mut taken);
        let mut text = format!("Package: {doc_name}\nFrom: {doc_path}\n\n");
        lines.iter().for_each(|l| text.push_str(&format!("{l}\n")));
        text.push_str(&info);
        entries.push((report_name, text.into_bytes()));
    }

    let files: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    let mut out = json!({ "files": files, "links": links, "fonts": fonts, "missingLinks": missing, "skippedFonts": skipped, "warnings": warnings });
    match root {
        Some(root) => {
            // Never replace the open document or a file it links to: the folder may be the document's own.
            let sources: Vec<&str> = std::iter::once(doc_path.as_str()).chain(copied_sources.iter().map(String::as_str)).collect();
            check_no_overwrite(&root, &entries, &sources)?;
            for (rel, bytes) in &entries {
                let path = root.join(rel);
                if let Some(dir) = path.parent() {
                    create_dir(&dir.to_string_lossy())?;
                }
                write(&path.to_string_lossy(), bytes)?;
            }
            out["folder"] = json!(root.to_string_lossy());
        }
        None => {
            let named: Vec<(String, &[u8])> = entries.iter().map(|(rel, b)| (format!("{name}/{rel}"), b.as_slice())).collect();
            let zip = zip(&named)?;
            out["name"] = json!(format!("{name}.zip"));
            out["bytes"] = json!(zip.len());
            out["dataBase64"] = json!(vectorcraft_format::base64_encode(&zip));
        }
    }
    Ok(out)
}

/// An error when writing any of `entries` (paths relative to `root`) would replace one of the
/// `sources` (compared by their real paths, so aliases and `..` don't hide a collision).
fn check_no_overwrite(root: &Path, entries: &[Entry], sources: &[&str]) -> Result<()> {
    let real: Vec<PathBuf> = sources.iter().filter_map(|s| std::fs::canonicalize(s).ok()).collect();
    for (rel, _) in entries {
        let path = root.join(rel);
        if let Ok(dest) = std::fs::canonicalize(&path)
            && real.contains(&dest)
        {
            return Err(bad(C, format!("`{}` would be overwritten by the package: choose a different folder or name", path.display())));
        }
    }
    Ok(())
}

// ---------- zip archives ----------

/// The MS-DOS time and date a zip entry records: now (UTC), else 1980-01-01.
fn dos_time() -> (u16, u16) {
    let Some(t) = vectorcraft_doc::metadata::output_time() else { return (0, 0x21) };
    let [y, mo, d, h, mi, s] = vectorcraft_doc::metadata::civil(t);
    let date = (((y - 1980).clamp(0, 127) << 9) | (mo << 5) | d) as u16;
    (((h << 11) | (mi << 5) | (s / 2)) as u16, date)
}

/// A zip archive of `files` ((path with `/` separators, bytes)), each deflated, or stored when
/// that isn't smaller, every entry dated now.
pub(crate) fn zip(files: &[(String, &[u8])]) -> Result<Vec<u8>> {
    super::fileio::zip::deflate(files, dos_time(), || false).map_err(|_| bad(C, "too large for a zip archive: package to a folder"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A font read from a file without a font extension (one a font manager keeps, for example) is
    /// packaged with the extension its data calls for.
    #[test]
    fn packaged_fonts_have_a_font_extension() {
        let (otf, ttc, ttf) = (b"OTTO....".as_slice(), b"ttcf....".as_slice(), b"\0\x01\0\0....".as_slice());
        let name = |path: Option<&str>, data: &[u8]| packaged_name(path.map(Path::new), "Some Family", "Bold", data);
        assert_eq!(name(Some("/fonts/A8F3C2"), otf), "A8F3C2.otf");
        assert_eq!(name(Some("/fonts/Shared Fonts.dat"), ttc), "Shared Fonts.dat.ttc");
        assert_eq!(name(Some("/fonts/Kept.TTF"), otf), "Kept.TTF");
        assert_eq!(name(Some("/fonts/Kept.otc"), ttc), "Kept.otc");
        assert_eq!(name(None, ttf), "SomeFamily-Bold.ttf");
    }
}
