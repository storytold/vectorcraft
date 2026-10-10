//! File → Package (`file.package`): the document with its linked files and fonts in a folder or a
//! zip, and the Document Info report it shares with Document Info › Save….

use std::collections::BTreeMap;
use std::io::Read;

use serde_json::{Value, json};

use super::tests_links::{BLUE, Folder, RED, open, place, png, save, session, write};
use super::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A saved document `dir/poster.vectorcraft` with two linked files (in `dir/art`) and a line of
/// type.
fn poster(dir: &Folder) -> Session {
    let (a, b) = (dir.file("art/red.png"), dir.file("art/blue.png"));
    write(&a, &png(600, 300, RED));
    write(&b, &png(300, 300, BLUE));
    let mut s = session();
    place(&mut s, &a);
    place(&mut s, &b);
    run(&mut s, "text.create", json!({"x": 10, "y": 40, "text": "Poster"}));
    save(&mut s, &dir.file("poster.vectorcraft"));
    s
}

/// The files in a zip archive by name (CRC-checked).
fn unzip(bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let le16 = |i: usize| u16::from_le_bytes([bytes[i], bytes[i + 1]]) as usize;
    let le32 = |i: usize| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
    let end = bytes.len() - 22;
    assert_eq!(le32(end), 0x0605_4b50, "end of central directory");
    let (count, mut i) = (le16(end + 10), le32(end + 16));
    let mut out = BTreeMap::new();
    for _ in 0..count {
        assert_eq!(le32(i), 0x0201_4b50);
        let (method, crc, packed, name_len, offset) = (le16(i + 10), le32(i + 16), le32(i + 20), le16(i + 28), le32(i + 42));
        let name = String::from_utf8(bytes[i + 46..i + 46 + name_len].to_vec()).unwrap();
        i += 46 + name_len + le16(i + 30) + le16(i + 32);
        assert_eq!(le32(offset), 0x0403_4b50);
        let data = &bytes[offset + 30 + le16(offset + 26) + le16(offset + 28)..][..packed];
        let mut content = vec![];
        match method {
            8 => drop(flate2::read::DeflateDecoder::new(data).read_to_end(&mut content).unwrap()),
            _ => content = data.to_vec(),
        }
        let mut c = flate2::Crc::new();
        c.update(&content);
        assert_eq!(c.sum() as usize, crc, "{name}");
        out.insert(name, content);
    }
    out
}

#[test]
fn two_links_and_a_font_are_collected_and_relinked() {
    let dir = Folder::new("package");
    let mut s = poster(&dir);
    let out = dir.file("out");
    let r = run(&mut s, "file.package", json!({"folder": out}));
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().map(|f| f.as_str().unwrap()).collect();
    assert_eq!(files, ["poster.vectorcraft", "Links/red.png", "Links/blue.png", "Fonts/SourceSans3-Regular.ttf", "poster Report.txt"]);
    assert_eq!((r["links"].as_u64(), r["fonts"].as_u64(), r["missingLinks"].clone()), (Some(2), Some(1), json!([])));
    let root = std::path::Path::new(&out).join("poster Folder");
    assert_eq!(std::path::Path::new(r["folder"].as_str().unwrap()), root);
    for f in &files {
        assert!(root.join(f).is_file(), "{f} written");
    }
    assert_eq!(std::fs::read(root.join("Links/red.png")).unwrap(), png(600, 300, RED));
    let font = std::fs::read(root.join("Fonts/SourceSans3-Regular.ttf")).unwrap();
    assert_eq!(&font[..4], &[0, 1, 0, 0], "a TrueType file");
    // The open document still links to the originals.
    let link = |s: &Session| {
        let mut paths = vec![];
        s.doc().unwrap().doc.visit_images(|_, im| paths.push(im.link.clone().unwrap()));
        paths
    };
    assert!(link(&s).iter().all(|l| l.path.contains("art")));
    // The packaged one opens with the copies, even with the originals gone.
    std::fs::remove_dir_all(dir.0.join("art")).unwrap();
    let r = open(&mut s, &root.join("poster.vectorcraft").to_string_lossy());
    assert_eq!(r["missingLinks"], json!([]), "{r}");
    for l in link(&s) {
        assert!(std::path::Path::new(&l.path).starts_with(root.join("Links")), "{l:?}");
        assert!(l.relative.as_deref().is_some_and(|r| r.starts_with("Links/")));
    }
    assert!(s.doc().unwrap().doc.images.values().all(|b| !b.is_proxy()), "the copies' pixels");
}

#[test]
fn an_unsaved_document_cant_be_packaged() {
    let mut s = session();
    let e = s.execute("file.package", &json!({"folder": std::env::temp_dir()})).unwrap_err().to_string();
    assert!(e.contains("save the document first"), "{e}");
    assert!(Session::new().execute("file.package", &json!({})).is_err(), "no document");
}

#[test]
fn without_a_folder_the_zip_has_the_same_files() {
    let dir = Folder::new("package-zip");
    let mut s = poster(&dir);
    let r = run(&mut s, "file.package", json!({"name": "Poster Kit"}));
    assert_eq!(r["name"], "Poster Kit.zip");
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    let want: Vec<String> = r["files"].as_array().unwrap().iter().map(|f| format!("Poster Kit/{}", f.as_str().unwrap())).collect();
    let mut want_sorted = want.clone();
    want_sorted.sort();
    assert_eq!(zip.keys().cloned().collect::<Vec<_>>(), want_sorted);
    assert_eq!(zip["Poster Kit/Links/blue.png"], png(300, 300, BLUE));
    // The packaged document links to the files beside it.
    let doc = vectorcraft_format::load(&zip["Poster Kit/poster.vectorcraft"]).unwrap();
    let mut rel = vec![];
    doc.visit_images(|_, im| rel.push(im.link.clone().unwrap().relative.unwrap()));
    rel.sort();
    assert_eq!(rel, ["Links/blue.png", "Links/red.png"]);
    let report = String::from_utf8(zip["Poster Kit/poster Report.txt"].clone()).unwrap();
    assert!(report.contains("LINKED FILES") && report.contains("→ Links/red.png"), "{report}");
    // The same entries as a folder package.
    let out = dir.file("out");
    let f = run(&mut s, "file.package", json!({"folder": out, "name": "Poster Kit"}));
    assert_eq!(f["files"], r["files"]);
}

#[test]
fn options_leave_out_links_fonts_and_the_report() {
    let dir = Folder::new("package-options");
    let mut s = poster(&dir);
    let r = run(&mut s, "file.package", json!({"linksFolder": false, "copyFonts": false, "report": false}));
    assert_eq!(r["files"], json!(["poster.vectorcraft", "red.png", "blue.png"]));
    let r = run(&mut s, "file.package", json!({"copyLinks": false}));
    assert_eq!(r["files"], json!(["poster.vectorcraft", "Fonts/SourceSans3-Regular.ttf", "poster Report.txt"]));
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    let doc = vectorcraft_format::load(&zip["poster Folder/poster.vectorcraft"]).unwrap();
    doc.visit_images(|_, im| assert!(im.link.as_ref().unwrap().path.contains("art"), "not relinked: the originals"));
    let r = run(&mut s, "file.package", json!({"relink": false}));
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    assert!(zip.contains_key("poster Folder/Links/red.png"), "copied");
    let doc = vectorcraft_format::load(&zip["poster Folder/poster.vectorcraft"]).unwrap();
    doc.visit_images(|_, im| assert!(im.link.as_ref().unwrap().path.contains("art"), "not relinked"));
    assert!(s.execute("file.package", &json!({"name": "../up"})).is_err());
}

#[test]
fn a_missing_link_is_reported_not_copied() {
    let dir = Folder::new("package-missing");
    let mut s = poster(&dir);
    std::fs::remove_file(dir.file("art/blue.png")).unwrap();
    // Reopened, the missing file's image shows its preview: nothing to copy.
    open(&mut s, &dir.file("poster.vectorcraft"));
    let r = run(&mut s, "file.package", json!({}));
    assert_eq!((r["links"].as_u64(), r["missingLinks"].clone()), (Some(1), json!(["blue.png"])));
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    let report = String::from_utf8(zip["poster Folder/poster Report.txt"].clone()).unwrap();
    assert!(report.contains("blue.png: not found"), "{report}");
}

#[test]
fn a_font_named_by_another_of_its_names_is_copied() {
    let dir = Folder::new("package-alias");
    let mut s = poster(&dir);
    // The bundled Semibold by its PostScript name, as an SVG or a PDF can name it.
    run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Alias", "font": "SourceSans3-Semibold"}));
    let r = run(&mut s, "file.package", json!({}));
    assert_eq!((r["fonts"].as_u64(), r["skippedFonts"].clone()), (Some(2), json!([])), "{}", r["skippedFonts"]);
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    let report = String::from_utf8(zip["poster Folder/poster Report.txt"].clone()).unwrap();
    assert!(report.contains("SourceSans3-Semibold Regular → Fonts/"), "{report}");
    let details = report.lines().find(|l| l.starts_with("SourceSans3-Semibold Regular: ")).unwrap_or_default();
    assert!(details.contains("embedding allowed") && !details.contains("missing"), "{report}");
}

#[test]
fn a_substituted_style_names_the_style_shown() {
    let dir = Folder::new("package-style");
    let mut s = poster(&dir);
    // No Source Sans 3 has a Black Wide, bundled or installed: its closest style stands in, and
    // that style's file is copied.
    run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Heavy", "font": "Source Sans 3", "style": "Black Wide"}));
    let r = run(&mut s, "file.package", json!({}));
    assert_eq!(r["skippedFonts"], json!([]));
    let zip = unzip(&vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap());
    let report = String::from_utf8(zip["poster Folder/poster Report.txt"].clone()).unwrap();
    assert!(report.contains("Source Sans 3 Black Wide (shown in Source Sans 3 "), "{report}");
    assert!(report.contains("Source Sans 3 Black Wide: substituted: shown in Source Sans 3 "), "{report}");
}

#[test]
fn the_document_info_report_has_every_category() {
    let dir = Folder::new("docinfo-report");
    let mut s = poster(&dir);
    s.execute("file.place", &json!({"path": dir.file("art/red.png"), "link": false})).unwrap();
    let text = run(&mut s, "document.info", json!({"format": "text"}))["text"].as_str().unwrap().to_string();
    for h in [
        "DOCUMENT",
        "OBJECTS",
        "GRAPHIC STYLES",
        "SPOT COLORS",
        "PATTERN OBJECTS",
        "GRADIENT SWATCHES",
        "SYMBOLS",
        "FONTS",
        "FONT DETAILS",
        "LINKED IMAGES",
        "EMBEDDED IMAGES",
    ] {
        assert!(text.contains(&format!("\n{h}\n")), "{h}: {text}");
    }
    assert!(text.contains("Images: 3") && text.contains("Source Sans 3 Regular: built in; embedding allowed"), "{text}");
    assert!(text.contains("red.png: 600 × 300 px — ") && text.contains("Color Mode: RGB") && text.contains("\nSwatches: "), "{text}");
    let fonts = run(&mut s, "document.info", json!({"format": "text", "category": "fonts"}))["text"].as_str().unwrap().to_string();
    assert!(fonts.contains("FONTS") && !fonts.contains("OBJECTS"), "{fonts}");
    let i = run(&mut s, "document.info", json!({"category": "linkedImages"}));
    assert_eq!((i["sections"].as_array().unwrap().len(), i["sections"][0]["id"].as_str()), (1, Some("linkedImages")));
    assert_eq!(i["sections"][0]["rows"].as_array().unwrap().len(), 2);
    let all = run(&mut s, "document.info", json!({}));
    assert_eq!(all["sections"].as_array().unwrap().len(), 11);
    assert!(s.execute("document.info", &json!({"category": "brushes"})).is_err());
    assert!(s.execute("document.info", &json!({"format": "xml"})).is_err());
    // The bundled fonts allow embedding.
    let face = vectorcraft_text::FontDb::global().face("Source Sans 3", "Regular").unwrap();
    assert!(face.embeddable() && face.path().is_none());
}

/// A professional handoff: the poster links to a reusable illustration, whose
/// own artwork links to a raster asset. Both dependency levels must travel.
fn poster_with_placed_document(dir: &Folder) -> Session {
    let image = dir.file("art/photo.png");
    write(&image, &png(600, 300, RED));
    let mut part = session();
    place(&mut part, &image);
    let part_path = dir.file("art/part.vectorcraft");
    save(&mut part, &part_path);

    let mut poster = session();
    let placed = run(&mut poster, "file.place", json!({ "path": part_path, "link": true }));
    assert_eq!(placed["linked"], true, "{placed}");
    save(&mut poster, &dir.file("poster.vectorcraft"));
    poster
}

/// Every link must point to a packaged copy, including links inside a linked
/// VectorCraft file. Validate after original assets have been removed.
fn verify_portable_nested_package(folder: &std::path::Path) {
    let main = folder.join("poster.vectorcraft");
    let part = folder.join("Links/part.vectorcraft");
    let image = folder.join("Links/photo.png");
    assert!(main.is_file() && part.is_file() && image.is_file());

    let mut parent = session();
    let opened = open(&mut parent, &main.to_string_lossy());
    assert_eq!(opened["missingLinks"], json!([]), "main package has broken links: {opened}");
    let mut targets = Vec::new();
    parent.doc().unwrap().doc.visit_placed(|_, p| targets.push(p.link.clone()));
    assert_eq!(targets.len(), 1);
    let link = &targets[0];
    assert_eq!(link.relative.as_deref(), Some("Links/part.vectorcraft"));
    let packaged_part = std::fs::read(&part).unwrap();
    let expected_hash = vectorcraft_doc::links::hash_bytes(&packaged_part);
    assert_eq!(link.hash.as_deref(), Some(expected_hash.as_str()));
    let mut nested = session();
    let opened = open(&mut nested, &part.to_string_lossy());
    assert_eq!(opened["missingLinks"], json!([]), "nested document has broken links: {opened}");
    let mut img_links = Vec::new();
    nested.doc().unwrap().doc.visit_images(|_, im| img_links.extend(im.link.clone()));
    assert_eq!(img_links.len(), 1);
    assert_eq!(img_links[0].relative.as_deref(), Some("photo.png"));
    assert_eq!(std::fs::read(&image).unwrap(), png(600, 300, RED));
}

#[test]
fn package_recursively_relinks_placed_documents_and_their_images() {
    let dir = Folder::new("package-placed");
    let mut s = poster_with_placed_document(&dir);
    let out = dir.file("delivery");
    let result = run(&mut s, "file.package", json!({ "folder": out }));
    assert_eq!((result["links"].as_u64(), result["missingLinks"].clone()), (Some(2), json!([])), "{result}");
    let folder = dir.0.join("delivery/poster Folder");
    // The package must work on a second computer without the source tree.
    std::fs::remove_dir_all(dir.0.join("art")).unwrap();
    verify_portable_nested_package(&folder);
}

#[test]
fn zipped_package_recursively_relinks_without_absolute_creator_paths() {
    let dir = Folder::new("package-placed-zip");
    let mut s = poster_with_placed_document(&dir);
    let result = run(&mut s, "file.package", json!({}));
    assert_eq!(result["links"], 2, "{result}");
    let bytes = vectorcraft_format::base64_decode(result["dataBase64"].as_str().unwrap()).unwrap();
    let archive = unzip(&bytes);
    let extraction = dir.file("extracted");
    for (name, bytes) in archive {
        write(&format!("{extraction}/{name}"), &bytes);
    }
    std::fs::remove_dir_all(dir.0.join("art")).unwrap();
    verify_portable_nested_package(&dir.0.join("extracted/poster Folder"));
}

#[test]
fn package_keeps_distinct_linked_documents_with_colliding_file_names() {
    let dir = Folder::new("package-collisions");
    let mut poster = session();
    for (number, color) in [(1, RED), (2, BLUE)] {
        let image = dir.file(&format!("art{number}/picture.png"));
        write(&image, &png(300, 300, color));
        let mut part = session();
        place(&mut part, &image);
        let name = dir.file(&format!("art{number}/part.vectorcraft"));
        save(&mut part, &name);
        run(&mut poster, "file.place", json!({ "path": name, "link": true }));
    }
    save(&mut poster, &dir.file("poster.vectorcraft"));
    let result = run(&mut poster, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["links"], 4, "{result}");
    assert_eq!(result["missingLinks"], json!([]));
    let folder = dir.0.join("delivery/poster Folder");
    let packaged: Vec<_> =
        result["files"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|name| name.ends_with(".vectorcraft")).collect();
    assert_eq!(packaged.len(), 3, "both nested documents must be retained: {packaged:?}");
    std::fs::remove_dir_all(dir.0.join("art1")).unwrap();
    std::fs::remove_dir_all(dir.0.join("art2")).unwrap();
    let mut check = session();
    assert_eq!(open(&mut check, &folder.join("poster.vectorcraft").to_string_lossy())["missingLinks"], json!([]));
    for name in packaged.into_iter().filter(|n| n.starts_with("Links/")) {
        let mut inner = session();
        let result = open(&mut inner, &folder.join(name).to_string_lossy());
        assert_eq!(result["missingLinks"], json!([]), "{name}: {result}");
    }
}

#[test]
fn package_finds_nested_assets_after_the_source_tree_moves() {
    let dir = Folder::new("package-moved-nested");
    let _original = poster_with_placed_document(&dir);
    let moved = dir.0.join("moved");
    std::fs::create_dir_all(&moved).unwrap();
    std::fs::rename(dir.0.join("art"), moved.join("art")).unwrap();
    std::fs::rename(dir.0.join("poster.vectorcraft"), moved.join("poster.vectorcraft")).unwrap();
    // The file paths embedded in both documents still point to the old location.
    // Opening the parent repairs its direct link, but the child must resolve
    // the image from the folder where it was actually found.
    let mut opened = session();
    open(&mut opened, &moved.join("poster.vectorcraft").to_string_lossy());
    let result = run(&mut opened, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["missingLinks"], json!([]), "moved file's nested link wasn't found: {result}");
    std::fs::remove_dir_all(moved.join("art")).unwrap();
    verify_portable_nested_package(&dir.0.join("delivery/poster Folder"));
}

#[test]
fn identical_stale_paths_in_different_nested_documents_keep_separate_assets() {
    let dir = Folder::new("package-stale-paths");
    let mut poster = session();
    for (number, color) in [(1, RED), (2, BLUE)] {
        let image = dir.file(&format!("art{number}/photo.png"));
        write(&image, &png(300, 300, color));
        let mut part = session();
        place(&mut part, &image);
        let name = dir.file(&format!("art{number}/part.vectorcraft"));
        save(&mut part, &name);

        // Two otherwise independent linked documents have the *same* obsolete
        // absolute image path. Their relative paths identify different files.
        let mut saved = vectorcraft_format::load(&std::fs::read(&name).unwrap()).unwrap();
        saved.update_links(|link| {
            link.path = "/old-computer/illustrations/photo.png".into();
            link.relative = Some("photo.png".into());
        });
        write(&name, &vectorcraft_format::save_file(&saved));
        run(&mut poster, "file.place", json!({ "path": name, "link": true }));
    }
    save(&mut poster, &dir.file("poster.vectorcraft"));
    let result = run(&mut poster, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["links"], 4, "both images and both documents must be copied: {result}");
    assert_eq!(result["missingLinks"], json!([]), "{result}");
    let folder = dir.0.join("delivery/poster Folder");
    std::fs::remove_dir_all(dir.0.join("art1")).unwrap();
    std::fs::remove_dir_all(dir.0.join("art2")).unwrap();

    let mut actual = Vec::new();
    for filename in result["files"].as_array().unwrap().iter().filter_map(Value::as_str).filter(|name| name.ends_with(".png")) {
        actual.push(std::fs::read(folder.join(filename)).unwrap());
    }
    assert_eq!(actual.len(), 2, "stale source paths must not cause asset deduplication");
    assert_ne!(actual[0], actual[1], "different original illustrations must keep different pixels");
    for filename in result["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .filter(|name| name.starts_with("Links/") && name.ends_with(".vectorcraft"))
    {
        let mut nested = session();
        let opened = open(&mut nested, &folder.join(filename).to_string_lossy());
        assert_eq!(opened["missingLinks"], json!([]), "{filename}: {opened}");
    }
}

#[test]
fn packaging_nested_files_preserves_embedded_preview_pdf_and_compression() {
    let dir = Folder::new("package-native-extras");
    let mut parent = poster_with_placed_document(&dir);
    let source = dir.file("art/part.vectorcraft");
    let original = std::fs::read(&source).unwrap();
    let doc = vectorcraft_format::load(&original).unwrap();
    let preview = png(8, 8, BLUE);
    let pdf = b"%PDF-1.7\n%Embedded client preview\n".to_vec();
    let mut opts = vectorcraft_format::SaveOptions::for_doc(&doc);
    opts.preview = Some(preview.clone());
    opts.pdf = Some(pdf.clone());
    opts.compress = true;
    write(&source, &vectorcraft_format::save_with(&doc, &opts).unwrap());

    let result = run(&mut parent, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["missingLinks"], json!([]), "{result}");
    let packaged = std::fs::read(dir.0.join("delivery/poster Folder/Links/part.vectorcraft")).unwrap();
    assert!(packaged.starts_with(&[0x1f, 0x8b]), "compression must be preserved");
    assert_eq!(vectorcraft_format::preview(&packaged), Some(preview));
    assert_eq!(vectorcraft_format::pdf_content(&packaged), Some(pdf));
    let nested = vectorcraft_format::load(&packaged).unwrap();
    let mut images = Vec::new();
    nested.visit_images(|_, image| images.push(image.link.clone()));
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].as_ref().unwrap().relative.as_deref(), Some("photo.png"));
}

/// The report keeps the packaged document's sections as they were, then gives each placed
/// document's own, headed by its file, instead of nesting them inside the parent's list.
#[test]
fn the_report_lists_a_placed_documents_links_in_a_section_of_its_own() {
    let dir = Folder::new("package-report-sections");
    let mut parent = poster_with_placed_document(&dir);
    run(&mut parent, "file.package", json!({ "folder": dir.file("delivery"), "copyFonts": false }));
    let report = std::fs::read_to_string(dir.0.join("delivery/poster Folder/poster Report.txt")).unwrap();
    let lines: Vec<&str> = report.lines().collect();
    let sections: Vec<usize> = lines.iter().enumerate().filter(|(_, l)| **l == "LINKED FILES").map(|(i, _)| i).collect();
    let [poster, part] = sections[..] else { panic!("two link sections: {report}") };
    assert!(lines[poster + 1].ends_with("→ Links/part.vectorcraft"), "the packaged document's section as before: {report}");
    assert!(lines[part + 1].starts_with("Document: ") && lines[part + 1].ends_with("part.vectorcraft"), "{report}");
    assert!(lines[part + 2].ends_with("→ Links/photo.png"), "{report}");
}

/// A damaged or newer-format linked native file is still an asset worth delivering.
/// Package must preserve the exact bytes instead of failing the entire handoff.
#[test]
fn unreadable_placed_document_is_copied_unchanged_with_warning() {
    let dir = Folder::new("package-unreadable-placed");
    let mut parent = poster_with_placed_document(&dir);
    let damaged = dir.file("art/part.vectorcraft");
    let bytes = b"not a parseable VectorCraft file".to_vec();
    write(&damaged, &bytes);

    let result = run(&mut parent, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["links"], 1, "{result}");
    assert_eq!(result["missingLinks"], json!([]), "{result}");
    let warnings = result["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 1, "{result}");
    assert!(warnings[0].as_str().unwrap().contains("unreadable linked document"), "{result}");

    let packaged = dir.0.join("delivery/poster Folder");
    assert_eq!(std::fs::read(packaged.join("Links/part.vectorcraft")).unwrap(), bytes);
    let report = std::fs::read_to_string(packaged.join("poster Report.txt")).unwrap();
    assert!(report.lines().any(|line| line == "LINKED FILES"), "{report}");
    assert!(report.contains("copied unchanged") && report.contains("its own links were not collected"), "{report}");
}

#[test]
fn circular_placed_links_copy_each_document_once_and_warn() {
    let dir = Folder::new("package-placed-cycle");
    let source = dir.file("root.vectorcraft");
    let child_path = dir.file("child.vectorcraft");

    let mut root = session();
    save(&mut root, &source);
    let mut child = session();
    run(&mut child, "file.place", json!({ "path": source, "link": true }));
    save(&mut child, &child_path);
    run(&mut root, "file.place", json!({ "path": child_path, "link": true }));
    save(&mut root, &source);

    let result = run(&mut root, "file.package", json!({ "folder": dir.file("delivery") }));
    assert_eq!(result["links"], 1, "the root must not be copied a second time: {result}");
    assert_eq!(result["missingLinks"], json!([]), "{result}");
    assert!(result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("circular placed-document link")), "{result}");
    let packaged = dir.0.join("delivery/root Folder");
    let root_doc = vectorcraft_format::load(&std::fs::read(packaged.join("root.vectorcraft")).unwrap()).unwrap();
    let child_doc = vectorcraft_format::load(&std::fs::read(packaged.join("Links/child.vectorcraft")).unwrap()).unwrap();
    let mut direct = Vec::new();
    root_doc.visit_placed(|_, p| direct.push(p.link.relative.clone()));
    assert_eq!(direct, [Some("Links/child.vectorcraft".into())]);
    let mut back = Vec::new();
    child_doc.visit_placed(|_, p| {
        back.push((p.link.relative.clone(), p.link.hash.clone(), p.link.size));
    });
    assert_eq!(back, [(Some("../root.vectorcraft".into()), None, None)]);
    let report = std::fs::read_to_string(packaged.join("root Report.txt")).unwrap();
    assert!(report.lines().any(|line| line == "LINKED FILES"), "{report}");
    assert!(report.contains("stopped following the cycle"), "{report}");
}

#[test]
fn nesting_limit_copies_boundary_file_and_reports_uncollected_links() {
    let dir = Folder::new("package-placed-depth-limit");
    let mut deepest = session();
    let mut next = dir.file("leaf.vectorcraft");
    save(&mut deepest, &next);

    let mut root = None;
    for depth in (0..=vectorcraft_doc::placed_document::MAX_DEPTH).rev() {
        let mut current = session();
        run(&mut current, "file.place", json!({ "path": next, "link": true }));
        let name = dir.file(&format!("level{depth}.vectorcraft"));
        save(&mut current, &name);
        next = name;
        if depth == 0 {
            root = Some(current);
        }
    }

    let mut root = root.unwrap();
    let result = run(&mut root, "file.package", json!({ "folder": dir.file("delivery") }));
    let expected_links = vectorcraft_doc::placed_document::MAX_DEPTH + 1;
    assert_eq!(result["links"].as_u64(), Some(expected_links as u64), "{result}");
    assert!(result["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("nesting limit reached")), "{result}");
    let folder = dir.0.join("delivery/level0 Folder");
    let packaged = std::fs::read(folder.join("Links/leaf.vectorcraft")).unwrap();
    assert_eq!(packaged, std::fs::read(dir.file("leaf.vectorcraft")).unwrap());
    let report = std::fs::read_to_string(folder.join("level0 Report.txt")).unwrap();
    assert!(report.contains("nesting limit reached"), "{report}");
}
