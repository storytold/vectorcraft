//! File → Package: a saved document copied into a folder of its own with the files its linked
//! images show (`Links/`), the fonts its type uses (`Fonts/`, leaving out fonts whose licence
//! doesn't allow embedding) and a text report (the package's contents, then the Document Info
//! report). The packaged document links to the copies (Relink); the open document doesn't change.
//! Without a folder (the web, agents) the same files come back as a zip archive.

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::path::Path;

use serde_json::{Value, json};

use super::fileio::{create_dir, write_file};
use super::*;

const C: &str = "file.package";

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        "file.package",
        "Package",
        [],
        None,
        "{folder?, name?: (default: \"<document> Folder\"), copyLinks?: true, linksFolder?: true (the linked files go in Links/, else next to the document), relink?: true (the packaged document links to the copies), copyFonts?: true (the fonts its type uses go in Fonts/; fonts whose licence doesn't allow embedding are left out), report?: true (<document> Report.txt)} copy the saved document as it is now into folder/name as <document>.vectorcraft with its linked files and fonts; the open document doesn't change → {folder, files: [paths relative to folder/name], links, fonts (counts copied), missingLinks: [file names not found], skippedFonts: [{font, reason}]}; no folder → the same with {name: \"<name>.zip\", dataBase64} (a zip of the files in <name>/) instead of folder",
        has_doc,
        package
    )]
}

/// A file of the package: its path in the package folder (`/` separators) and bytes.
type Entry = (String, Vec<u8>);

/// `name`, else the first free `stem 2.ext`, `stem 3.ext`… (compared without case, as most
/// desktop file systems do); the name is taken.
fn free_name(name: &str, taken: &mut HashSet<String>) -> String {
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

fn package(s: &mut Session, p: &Value) -> Result<Value> {
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
    let st = s.doc()?;
    let mut entries: Vec<Entry> = vec![];
    let mut taken = HashSet::new();
    let mut lines = vec![];

    // The linked files.
    let (mut copies, mut missing) = (BTreeMap::new(), vec![]);
    if o.copy_links {
        let dir = if o.links_folder { "Links/" } else { "" };
        let mut names = HashSet::new();
        lines.push("LINKED FILES".to_string());
        for f in super::links::linked_files(&st.doc, Some(&doc_path)) {
            let Some(bytes) = f.bytes else {
                lines.push(format!("{}: not found ({}), not copied", f.name, f.path));
                missing.push(f.name);
                continue;
            };
            let rel = format!("{dir}{}", free_name(&f.name, if o.links_folder { &mut names } else { &mut taken }));
            lines.push(format!("{} → {rel}", f.path));
            copies.insert(f.path, rel.clone());
            entries.push((rel, bytes));
        }
        lines.push(String::new());
    }

    // The fonts.
    let (mut fonts, mut skipped) = (0, vec![]);
    if o.copy_fonts {
        let db = vectorcraft_text::FontDb::global();
        let mut files = HashSet::new();
        lines.push("FONTS".to_string());
        for (family, style) in super::fonts::used_fonts(&st.doc) {
            let font = format!("{family} {style}");
            // Found by any of its names, as the canvas draws it.
            let resolved = db.resolve(&family, &style).filter(|(_, m)| *m != vectorcraft_text::FontMatch::Missing);
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
                    if files.insert(file.to_lowercase()) {
                        entries.push((format!("Fonts/{file}"), data.to_vec()));
                        fonts += 1;
                    }
                    lines.push(format!("{font}{shown} → Fonts/{file}"));
                    continue;
                }
                Some(_) => "its licence doesn't allow embedding",
                None => "not available on this computer",
            };
            lines.push(format!("{font}{shown}: {reason}, not copied"));
            skipped.push(json!({ "font": font, "reason": reason }));
        }
        lines.push(String::new());
    }

    // The document, linking to the copies.
    let mut doc = (*st.doc).clone();
    let doc_name = format!("{stem}.{}", vectorcraft_format::EXTENSION);
    if let Some(root) = &root
        && let Some(d) = super::links::with_relative_paths(&doc, &root.join(&doc_name).to_string_lossy())
    {
        doc = d;
    }
    if o.relink && !copies.is_empty() {
        let copied = |im: &vectorcraft_doc::ImageObject| im.link.as_ref().is_some_and(|l| copies.contains_key(&l.path));
        doc.update_images(copied, |im| {
            if let Some(l) = &mut im.link
                && let Some(rel) = copies.get(&l.path)
            {
                l.path = root.as_ref().map_or_else(|| rel.clone(), |r| r.join(rel).to_string_lossy().into_owned());
                (l.relative, l.modified) = (Some(rel.clone()), None);
            }
        });
    }
    entries.insert(0, (doc_name.clone(), vectorcraft_format::save_file(&doc)));
    taken.insert(doc_name.to_lowercase());

    // The report.
    if let Some(info) = info {
        let report_name = free_name(&format!("{stem} Report.txt"), &mut taken);
        let mut text = format!("Package: {doc_name}\nFrom: {doc_path}\n\n");
        lines.iter().for_each(|l| text.push_str(&format!("{l}\n")));
        text.push_str(&info);
        entries.push((report_name, text.into_bytes()));
    }

    let files: Vec<&str> = entries.iter().map(|(p, _)| p.as_str()).collect();
    let mut out = json!({ "files": files, "links": copies.len(), "fonts": fonts, "missingLinks": missing, "skippedFonts": skipped });
    match root {
        Some(root) => {
            for (rel, bytes) in &entries {
                let path = root.join(rel);
                if let Some(dir) = path.parent() {
                    create_dir(&dir.to_string_lossy())?;
                }
                write_file(&path.to_string_lossy(), bytes)?;
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

// ---------- zip archives ----------

/// The MS-DOS time and date a zip entry records: now (UTC), else 1980-01-01.
fn dos_time() -> (u16, u16) {
    let Some(t) = vectorcraft_doc::metadata::now_unix() else { return (0, 0x21) };
    let [y, mo, d, h, mi, s] = vectorcraft_doc::metadata::civil(t);
    let date = (((y - 1980).clamp(0, 127) << 9) | (mo << 5) | d) as u16;
    (((h << 11) | (mi << 5) | (s / 2)) as u16, date)
}

/// A zip archive of `files` ((path with `/` separators, bytes)), each deflated, or stored when
/// that isn't smaller.
pub(crate) fn zip(files: &[(String, &[u8])]) -> Result<Vec<u8>> {
    let too_big = || bad(C, "too large for a zip archive: package to a folder");
    let io = |e: std::io::Error| EngineError::Other(e.to_string());
    let (time, date) = dos_time();
    let (mut out, mut central) = (Vec::new(), Vec::new());
    for (name, bytes) in files {
        let mut crc = flate2::Crc::new();
        crc.update(bytes);
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(bytes).map_err(io)?;
        let deflated = enc.finish().map_err(io)?;
        let (method, data): (u16, &[u8]) = if deflated.len() < bytes.len() { (8, &deflated) } else { (0, bytes) };
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        let (packed, size) = (u32::try_from(data.len()).map_err(|_| too_big())?, u32::try_from(bytes.len()).map_err(|_| too_big())?);
        let name_len = u16::try_from(name.len()).map_err(|_| too_big())?;
        // Version 2.0, UTF-8 names, method, time, date, CRC-32, sizes, name length, no extra field.
        let common = [&20u16.to_le_bytes()[..], &0x0800u16.to_le_bytes(), &method.to_le_bytes(), &time.to_le_bytes(), &date.to_le_bytes()].concat();
        let sums = [crc.sum().to_le_bytes(), packed.to_le_bytes(), size.to_le_bytes()].concat();
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&common);
        out.extend_from_slice(&sums);
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&common);
        central.extend_from_slice(&sums);
        central.extend_from_slice(&name_len.to_le_bytes());
        // No extra field or comment, disk 0, no attributes.
        central.extend_from_slice(&[0; 12]);
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let (start, len) = (u32::try_from(out.len()).map_err(|_| too_big())?, u32::try_from(central.len()).map_err(|_| too_big())?);
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    Ok(out)
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
