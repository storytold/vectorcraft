//! Missing fonts: listed (`text.missingFonts`), their files found in a folder on threads of their
//! own (`text.findFontFiles`), and copied into a folder without replacing a file there. These tests
//! never set VectorCraft's Fonts folder and never add a font to the process's font database:
//! `tests/font_folder.rs` does both, in a process of its own.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_text::FontMatch;

use crate::cmd::findfiles::{self, Limits, Rules, Visitor};
use crate::cmd::fontfiles::{self, FontSearch, MAX_FONT_FILE, Sought, copy_into};
use crate::{EngineError, Session};

/// A fresh temporary folder (canonical, as a search reports it).
fn folder(tag: &str) -> PathBuf {
    let dir = vectorcraft_testkit::temp_dir(&format!("fontfiles-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    findfiles::plain(std::fs::canonicalize(&dir).unwrap())
}

/// A session whose folder searches may go anywhere in `root`, on two threads.
fn session_for(root: &Path) -> Session {
    let mut s = Session::new();
    s.search_rules = Some(Rules { user_content: vec![root.to_path_buf()], ..Rules::default() });
    s.search_threads = Some(2);
    s
}

/// `text.findFontFiles {}` once the search ended (10 seconds at most).
fn finished(s: &mut Session) -> Value {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let r = s.execute("text.findFontFiles", &json!({})).unwrap();
        if r["state"] != "searching" || Instant::now() > until {
            return r;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// New point type in `font` and `style` → its id.
fn text(s: &mut Session, y: f64, font: &str, style: &str) -> u64 {
    s.execute("text.create", &json!({"x": 10, "y": y, "text": "Ab", "font": font, "style": style})).unwrap()["id"].as_u64().unwrap()
}

fn bundled(file: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/fonts").join(file)).unwrap()
}

#[test]
fn missing_fonts_are_listed_symbols_included() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    text(&mut s, 20.0, "No Such Font Family", "Regular");
    // Type naming another version of a missing font is listed with it.
    let other = vectorcraft_doc::NodeId(text(&mut s, 40.0, "No Such Font Family", "Regular"));
    s.edit("version", |d, _| {
        if let vectorcraft_doc::NodeKind::Text(t) = &mut d.node_mut(other).unwrap().kind {
            t.runs.iter_mut().for_each(|r| r.style.font_version = Some("Version 2.000".into()));
        }
        Ok(())
    })
    .unwrap();
    // A style no Source Sans 3 has, bundled or installed.
    text(&mut s, 60.0, "Source Sans 3", "Black Wide");
    text(&mut s, 100.0, "Source Sans 3", "Semibold");
    let id = text(&mut s, 140.0, "Symbol Only Family", "Bold");
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("symbol.new", &json!({})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    // CSS generic families, which no font file provides, are left out.
    text(&mut s, 180.0, "sans-serif", "Regular");
    text(&mut s, 220.0, "SERIF", "Bold");
    let r = s.execute("text.missingFonts", &json!({})).unwrap();
    let rows: Vec<(&str, &str, &str)> = r["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| (f["family"].as_str().unwrap(), f["style"].as_str().unwrap(), f["status"].as_str().unwrap()))
        .collect();
    assert_eq!(
        rows,
        [("No Such Font Family", "Regular", "missing"), ("Source Sans 3", "Black Wide", "substitute"), ("Symbol Only Family", "Bold", "missing")]
    );
    assert_eq!((r["count"].as_u64(), r["fonts"][1]["resolved"]["family"].as_str()), (Some(3), Some("Source Sans 3")));
    assert!(r.get("fontsNextToDocument").is_none(), "an unsaved document has no folder");
    let listed = s.execute("text.fonts", &json!({})).unwrap();
    assert!(!listed.as_array().unwrap().iter().any(|f| f["family"] == "Symbol Only Family"), "Find Font lists the layers' type only");
    assert!(listed.as_array().unwrap().iter().any(|f| f["family"] == "sans-serif" && f["status"] == "missing"), "{listed}");
    assert!(matches!(Session::new().execute("text.missingFonts", &json!({})), Err(EngineError::Disabled(..))));
}

/// A font whose name is longer than 256 bytes is left out of the list, and a search for the fonts
/// listed finds the other fonts' files.
#[test]
fn an_overlong_font_name_leaves_the_other_fonts_searchable() {
    let root = folder("long-name");
    std::fs::write(root.join("Findme.otf"), vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
    let mut s = session_for(&root);
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    text(&mut s, 20.0, &"L".repeat(300), "Regular");
    text(&mut s, 60.0, "Findme Sans 3", "Regular");
    let listed = s.execute("text.missingFonts", &json!({})).unwrap();
    let fonts: Vec<Value> = listed["fonts"].as_array().unwrap().iter().map(|f| json!({"family": f["family"], "style": f["style"]})).collect();
    assert_eq!(fonts, [json!({"family": "Findme Sans 3", "style": "Regular"})]);
    s.execute("text.findFontFiles", &json!({"folder": root, "fonts": fonts})).unwrap();
    assert_eq!(finished(&mut s)["fonts"][0]["files"], json!([root.join("Findme.otf")]));
}

/// The Fonts folder next to a document is offered only where a search may start: not inside other
/// apps' folders, such as the attachments a mail app keeps in the home folder's Library.
#[test]
fn the_fonts_folder_next_to_a_document_in_another_apps_folder_isnt_offered() {
    let root = folder("next-to");
    let home = root.join("Users/me");
    let mut s = Session::new();
    s.search_rules = Some(Rules { home: Some(home.clone()), ..Rules::default() });
    for (dir, offered) in [("Library/Mail Downloads/pkg", false), ("Documents/pkg", true)] {
        std::fs::create_dir_all(home.join(dir).join("Fonts")).unwrap();
        let mut doc = vectorcraft_doc::Document::new(100.0, 100.0);
        doc.title = "doc".into();
        s.add_document(doc, Some(home.join(dir).join("doc.vectorcraft").to_string_lossy().into_owned()));
        text(&mut s, 20.0, "No Such Font Family", "Regular");
        let r = s.execute("text.missingFonts", &json!({})).unwrap();
        assert_eq!(r.get("fontsNextToDocument").is_some(), offered, "{dir}: {r}");
    }
}

/// A document opened by a relative path gives the Fonts folder next to it as an absolute path,
/// which `text.findFontFiles` takes.
#[cfg(unix)]
#[test]
fn the_fonts_folder_next_to_a_document_is_an_absolute_path() {
    let root = folder("relative");
    std::fs::create_dir_all(root.join("pkg/Fonts")).unwrap();
    let cwd = std::env::current_dir().unwrap();
    let up = "../".repeat(cwd.components().count().saturating_sub(1));
    let relative = format!("{up}{}", root.join("pkg/doc.vectorcraft").strip_prefix("/").unwrap().display());
    assert!(Path::new(&relative).is_relative());
    let mut s = session_for(&root);
    let mut doc = vectorcraft_doc::Document::new(100.0, 100.0);
    doc.title = "doc".into();
    s.add_document(doc, Some(relative));
    text(&mut s, 20.0, "No Such Font Family", "Regular");
    let next = s.execute("text.missingFonts", &json!({})).unwrap()["fontsNextToDocument"].as_str().unwrap().to_string();
    assert!(Path::new(&next).is_absolute(), "{next}");
    assert_eq!(std::fs::canonicalize(&next).unwrap(), root.join("pkg/Fonts"));
    assert!(s.execute("text.findFontFiles", &json!({"folder": next})).is_ok());
    assert_ne!(finished(&mut s)["state"], "failed");
}

#[test]
fn a_search_finds_a_missing_fonts_file_by_its_names() {
    let root = folder("find");
    std::fs::create_dir_all(root.join("x/y")).unwrap();
    std::fs::write(root.join("x/y/Findme.otf"), vectorcraft_testkit::fonts::renamed("Findme Sans 3")).unwrap();
    std::fs::write(root.join("other.ttf"), bundled("SourceSerif4-Regular.ttf")).unwrap();
    std::fs::write(root.join("readme.txt"), "Findme Sans 3").unwrap();
    let mut s = session_for(&root);
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    text(&mut s, 20.0, "Findme Sans 3", "Regular");
    // Never found: the search reads the whole folder.
    text(&mut s, 60.0, "No Such Font Family", "Regular");
    let r = s.execute("text.findFontFiles", &json!({"folder": root})).unwrap();
    assert!(matches!(r["state"].as_str(), Some("searching" | "done")), "{r}");
    let id = r["id"].clone();
    let r = finished(&mut s);
    assert_eq!((&r["state"], &r["id"], r.get("stopped")), (&json!("done"), &id, None), "{r}");
    let found = root.join("x").join("y").join("Findme.otf").to_string_lossy().into_owned();
    let findme = r["fonts"].as_array().unwrap().iter().find(|f| f["family"] == "Findme Sans 3").cloned().unwrap();
    assert_eq!((findme["status"].as_str(), findme["files"].clone()), (Some("missing"), json!([found])));
    assert_eq!(r["searched"]["fontFiles"], 2, "the font files are read, not the text file: {r}");
    assert_eq!(r["folder"], json!(root));
    // Fonts given by name: the search ends once each has a file.
    let r = s.execute("text.findFontFiles", &json!({"folder": root, "fonts": [{"family": "Findme Sans 3"}]})).unwrap();
    assert_ne!(r["id"], id, "a new search");
    let r = finished(&mut s);
    assert_eq!((&r["state"], &r["fonts"][0]["files"]), (&json!("done"), &json!([found])), "{r}");
    // Fonts that are available aren't looked for.
    let r = s.execute("text.findFontFiles", &json!({"folder": root, "fonts": [{"family": "Source Sans 3"}]}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
    // Stopped by a new search, or by `stop`.
    s.execute("text.findFontFiles", &json!({"folder": root})).unwrap();
    let r = s.execute("text.findFontFiles", &json!({"stop": true})).unwrap();
    assert!(matches!(r["state"].as_str(), Some("stopped" | "done")), "{r}");
}

#[test]
fn junk_search_params_never_fail_internally() {
    let root = folder("junk");
    let mut s = session_for(&root);
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    text(&mut s, 20.0, "No Such Font Family", "Regular");
    let internal = |r: &crate::Result<Value>| matches!(r, Err(EngineError::Internal { .. }));
    let mut cases = vec![];
    for v in vectorcraft_testkit::strategies::junk_values() {
        cases.extend([
            json!({"folder": root, "maxSeconds": v}),
            json!({"folder": root, "fonts": v}),
            json!({"folder": root, "stop": v}),
            json!({"folder": v}),
            json!({"folder": root, "fonts": [v]}),
            json!({"folder": root, "fonts": [{"family": v}]}),
            json!({"folder": root, "fonts": [{"family": "No Such Font Family", "style": v}]}),
        ]);
        assert!(s.execute("text.addFontFiles", &json!({"files": v})).is_err());
        assert!(s.execute("text.addFontFiles", &json!({"files": [v]})).is_err());
    }
    let long = "x".repeat(257);
    let many: Vec<Value> = (0..1001).map(|i| json!({"family": format!("No Such Family {i}")})).collect();
    cases.extend([
        json!({"folder": root, "fonts": [{"family": long}]}),
        json!({"folder": root, "fonts": [{"family": "x", "style": long}]}),
        json!({"folder": root, "fonts": [{"family": "a\u{0}b"}]}),
        json!({"folder": root, "fonts": many}),
        json!({"folder": root, "fonts": many[..1000]}),
        json!({"folder": "/a\u{0}b"}),
        json!({"folder": "relative/folder"}),
        json!({"folder": "/\u{FFFD}"}),
        json!({"folder": format!("/{}", "x".repeat(4096))}),
        json!({"folder": root.join("No Such Folder")}),
    ]);
    let mut started = 0;
    for p in &cases {
        let r = s.execute("text.findFontFiles", p);
        assert!(!internal(&r), "{p}: {r:?}");
        if r.is_ok() && p.get("folder").is_some_and(|f| f.is_string()) {
            started += 1;
            assert_ne!(finished(&mut s)["state"], "searching", "{p}");
        }
    }
    assert!(started > 40, "only {started} searches started");
    // After a finished search, files it didn't find aren't copied.
    for v in [json!(["/no/such.ttf"]), json!([root.join("readme.ttf")])] {
        assert!(s.execute("text.addFontFiles", &json!({"files": v})).is_err());
    }
}

/// Reads every file, and waits on reading one until its channel sends or closes.
struct Blocked(Mutex<mpsc::Receiver<()>>);

impl Visitor for Blocked {
    fn wants(&self, _: &str) -> bool {
        true
    }
    fn items(&self, _: &Path) -> Vec<usize> {
        let _ = self.0.lock().unwrap().recv();
        vec![]
    }
}

#[test]
fn stop_ends_only_the_search_it_names() {
    let root = folder("stop");
    std::fs::write(root.join("a.ttf"), b"font").unwrap();
    let mut s = session_for(&root);
    let (tx, rx) = mpsc::channel();
    let search = findfiles::start(root.clone(), s.search_rules.clone(), Limits::default(), 1, 1, Arc::new(Blocked(Mutex::new(rx)))).unwrap();
    let fonts = vec![Sought { family: "A".into(), style: "Regular".into(), status: FontMatch::Missing }];
    s.font_search = Some(FontSearch { id: 7, folder: root.to_string_lossy().into_owned(), fonts, search });
    let until = Instant::now() + Duration::from_secs(10);
    while s.execute("text.findFontFiles", &json!({})).unwrap()["searched"]["fontFiles"] != 1 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    fontfiles::stop(&mut s, 8);
    assert_eq!(s.execute("text.findFontFiles", &json!({})).unwrap()["state"], "searching");
    fontfiles::stop(&mut s, 7);
    let r = s.execute("text.findFontFiles", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["stopped"].as_str(), r["id"].as_u64()), (Some("stopped"), Some("stop"), Some(7)));
    drop(tx);
}

#[test]
fn adding_fonts_needs_vectorcraft_s_fonts_folder() {
    let mut s = Session::new();
    let r = s.execute("text.addFontFiles", &json!({"files": ["/fonts/a.ttf"]}));
    assert!(matches!(&r, Err(EngineError::Other(m)) if m.contains("isn't set here")), "{r:?}");
    // As many files as a search keeps (one for each of 1000 fonts) can be added at once.
    let files = |n: usize| json!({"files": (0..n).map(|i| format!("/fonts/{i}.ttf")).collect::<Vec<_>>()});
    let r = s.execute("text.addFontFiles", &files(1000));
    assert!(matches!(&r, Err(EngineError::Other(m)) if m.contains("isn't set here")), "{r:?}");
    assert!(matches!(s.execute("text.addFontFiles", &files(1001)), Err(EngineError::BadParams { .. })));
}

#[test]
fn copying_never_replaces_a_file() {
    let root = folder("copy");
    let (src, dest) = (root.join("src"), root.join("dest"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(src.join("a.otf"), b"new font").unwrap();
    std::fs::write(dest.join("a.otf"), b"keep").unwrap();
    let r = copy_into(&[src.join("a.otf")], &dest);
    assert_eq!(r.copied, [(src.join("a.otf"), dest.join("a 2.otf"))]);
    assert_eq!((std::fs::read(dest.join("a.otf")).unwrap(), std::fs::read(dest.join("a 2.otf")).unwrap()), (b"keep".to_vec(), b"new font".to_vec()));
    // The same file again: kept, not copied a third time.
    let r = copy_into(&[src.join("a.otf")], &dest);
    assert_eq!((r.copied.len(), r.kept.clone()), (0, vec![(src.join("a.otf"), dest.join("a 2.otf"))]));
    assert!(!dest.join("a 3.otf").exists());
    // A link of that name, even one to nothing, is neither replaced nor written through.
    #[cfg(unix)]
    {
        std::fs::write(src.join("b.otf"), b"b").unwrap();
        std::os::unix::fs::symlink(root.join("nothing.otf"), dest.join("b.otf")).unwrap();
        let r = copy_into(&[src.join("b.otf")], &dest);
        assert_eq!(r.copied, [(src.join("b.otf"), dest.join("b 2.otf"))]);
        assert!(!root.join("nothing.otf").exists());
    }
    // A name taken in another case: on a file system that ignores case, the copy gets a number.
    std::fs::write(dest.join("C.OTF"), b"other").unwrap();
    std::fs::write(src.join("c.otf"), b"c").unwrap();
    let ignores_case = dest.join("c.otf").exists();
    let r = copy_into(&[src.join("c.otf")], &dest);
    let to = dest.join(if ignores_case { "c 2.otf" } else { "c.otf" });
    assert_eq!((r.copied, std::fs::read(dest.join("C.OTF")).unwrap()), (vec![(src.join("c.otf"), to)], b"other".to_vec()));
    // Every numbered name taken.
    std::fs::write(src.join("d.otf"), b"d").unwrap();
    std::fs::write(dest.join("d.otf"), b"other").unwrap();
    for n in 2..=99 {
        std::fs::write(dest.join(format!("d {n}.otf")), b"other").unwrap();
    }
    let r = copy_into(&[src.join("d.otf")], &dest);
    assert_eq!(r.skipped, [(src.join("d.otf"), "every numbered name for it is taken in the Fonts folder".to_string())]);
    // Too large (a sparse file), a folder, and a file that isn't there.
    let huge = std::fs::File::create(src.join("huge.otf")).unwrap();
    huge.set_len(MAX_FONT_FILE + 1).unwrap();
    std::fs::create_dir_all(src.join("folder.otf")).unwrap();
    let r = copy_into(&[src.join("huge.otf"), src.join("folder.otf"), src.join("none.otf")], &dest);
    let reasons: Vec<&str> = r.skipped.iter().map(|(_, why)| why.as_str()).collect();
    assert_eq!(reasons[..2], ["larger than 256 MB", "not a file"]);
    assert!(r.copied.is_empty() && reasons.len() == 3, "{r:?}");
    assert!(!dest.join("huge.otf").exists() && !dest.join("folder.otf").exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn only_font_files_are_copied() {
    let root = folder("allowed");
    let (src, dest) = (root.join("src"), root.join("dest"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    let names = ["notes.txt", "Script", "web.WOFF2", "font.woff"];
    for name in names {
        std::fs::write(src.join(name), name).unwrap();
    }
    let r = copy_into(&names.map(|n| src.join(n)), &dest);
    let copied: Vec<PathBuf> = r.copied.iter().map(|(from, _)| from.clone()).collect();
    assert_eq!(copied, [src.join("web.WOFF2"), src.join("font.woff")]);
    let not_a_font = |n: &str| (src.join(n), "not a font file".to_string());
    assert_eq!(r.skipped, [not_a_font("notes.txt"), not_a_font("Script")]);
    assert!(!dest.join("notes.txt").exists() && !dest.join("Script").exists());
    let _ = std::fs::remove_dir_all(&root);
}

/// A search reads no font file larger than Add Fonts copies, and finds a smaller copy of the font.
#[test]
fn a_search_skips_font_files_too_large_to_add() {
    let root = folder("too-large");
    let (big, small) = (root.join("big").join("Sizeme.otf"), root.join("small").join("Sizeme.otf"));
    let font = vectorcraft_testkit::fonts::renamed("Sizeme Sans 3");
    for path in [&big, &small] {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, &font).unwrap();
    }
    // One byte past the limit, without writing it: the file system leaves the rest unallocated.
    std::fs::File::options().write(true).open(&big).unwrap().set_len(MAX_FONT_FILE + 1).unwrap();
    let mut s = session_for(&root);
    // A font no file has keeps the search going through every folder.
    s.execute("text.findFontFiles", &json!({"folder": root, "fonts": [{"family": "Sizeme Sans 3"}, {"family": "No Such Sans"}]})).unwrap();
    let r = finished(&mut s);
    assert_eq!(r["fonts"][0]["files"], json!([small]), "{r}");
    let _ = std::fs::remove_dir_all(&root);
}

/// A suitcase font: a file without an extension or with `.suit`, empty but for the fonts in its
/// resource fork, which only macOS keeps. A search finds it, and Add Fonts copies it with its fork.
#[cfg(target_os = "macos")]
#[test]
fn suitcase_fonts_are_found_and_copied_with_their_fork() {
    use vectorcraft_testkit::fonts::{renamed, suitcase_fork};
    let root = folder("suitcase");
    let (src, dest) = (root.join("src"), root.join("dest"));
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::create_dir_all(&dest).unwrap();
    let suitcase = |path: &Path, family: &str| {
        let fork = suitcase_fork(&[renamed(family)]);
        std::fs::write(path, b"").unwrap();
        std::fs::write(path.join("..namedfork/rsrc"), &fork).unwrap();
        fork
    };
    let found = src.join("sub/Suitme");
    let fork = suitcase(&found, "Suitme Sans 3");
    // A file without an extension that isn't a suitcase is checked and left out.
    std::fs::write(src.join("README"), b"text").unwrap();
    let mut s = session_for(&root);
    s.execute("text.findFontFiles", &json!({"folder": src, "fonts": [{"family": "Suitme Sans 3"}]})).unwrap();
    let r = finished(&mut s);
    assert_eq!(r["fonts"][0]["files"], json!([found]), "{r}");
    // A suitcase with the `.suit` extension is found too, and a `.suit` file that isn't one is left out.
    let suit = src.join("Suited.SUIT");
    suitcase(&suit, "Suited Sans 3");
    std::fs::write(src.join("Plain.suit"), b"text").unwrap();
    // A font no file has keeps the search going through every folder.
    s.execute("text.findFontFiles", &json!({"folder": src, "fonts": [{"family": "Suited Sans 3"}, {"family": "No Such Sans"}]})).unwrap();
    let r = finished(&mut s);
    assert_eq!((r["fonts"][0]["files"].clone(), r["searched"]["fontFiles"].as_u64()), (json!([suit]), Some(2)), "{r}");
    let r = copy_into(std::slice::from_ref(&found), &dest);
    assert_eq!(r.copied, [(found.clone(), dest.join("Suitme"))]);
    assert!(vectorcraft_text::is_suitcase(&dest.join("Suitme")));
    assert_eq!(std::fs::read(dest.join("Suitme/..namedfork/rsrc")).unwrap(), fork);
    // The same suitcase again is kept; another of that name gets a number.
    assert_eq!(copy_into(std::slice::from_ref(&found), &dest).kept, [(found.clone(), dest.join("Suitme"))]);
    let other = src.join("Suitme");
    suitcase(&other, "Other Sans 33");
    assert_eq!(copy_into(std::slice::from_ref(&other), &dest).copied, [(other.clone(), dest.join("Suitme 2"))]);
    // A suitcase named like a font file keeps its fork too.
    let named = src.join("Suitext.ttf");
    let named_fork = suitcase(&named, "Suitext Sans3");
    assert_eq!(copy_into(std::slice::from_ref(&named), &dest).copied, [(named.clone(), dest.join("Suitext.ttf"))]);
    assert_eq!(std::fs::read(dest.join("Suitext.ttf/..namedfork/rsrc")).unwrap(), named_fork);
    // The font scan reads the copies.
    let db = vectorcraft_text::FontDb::with_font_dirs(vec![dest.clone()]);
    assert_eq!((db.styles("Suitme Sans 3"), db.styles("Other Sans 33")), (vec!["Regular".to_string()], vec!["Regular".to_string()]));
    let _ = std::fs::remove_dir_all(&root);
}
