//! Missing linked files looked for in a folder on threads of their own (`links.findFiles`), the
//! files found ranked by the size and hash saved with each link, and `links.relink {copy}`, which
//! puts the file in the document's folder without replacing a file there.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::cmd::findfiles::{self, Limits, Rules, Visitor};
use crate::cmd::linkfiles::{self, LinkSearch, Sought};
use crate::tests_links::{BLUE, RED, place, png};
use crate::{EngineError, Session};

/// A fresh temporary folder (canonical, as a search reports it).
fn folder(tag: &str) -> PathBuf {
    let dir = vectorcraft_testkit::temp_dir(&format!("linkfiles-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    findfiles::plain(std::fs::canonicalize(&dir).unwrap())
}

/// Write `bytes` to the file at `path`, making its folders.
fn write(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// A session with a new document, whose folder searches may go anywhere in `root`, on two threads.
fn session_for(root: &Path) -> Session {
    let mut s = Session::new();
    s.search_rules = Some(Rules { user_content: vec![root.to_path_buf()], ..Rules::default() });
    s.search_threads = Some(2);
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// Place the file at `path`, linked → its id.
fn placed(s: &mut Session, path: &Path) -> u64 {
    place(s, &path.to_string_lossy()).0
}

/// `links.findFiles {}` once the search ended (10 seconds at most).
fn finished(s: &mut Session) -> Value {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let r = s.execute("links.findFiles", &json!({})).unwrap();
        if r["state"] != "searching" || Instant::now() > until {
            return r;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The files a search found for the linked file named `name`, best first: (path below `root`,
/// match, whether the name's letter case differs).
fn found(r: &Value, name: &str, root: &Path) -> Vec<(String, String, bool)> {
    let link = r["links"].as_array().unwrap().iter().find(|l| l["name"] == name).unwrap_or_else(|| panic!("{name} in {r}"));
    link["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            let path = Path::new(f["path"].as_str().unwrap()).strip_prefix(root).unwrap();
            let path = path.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect::<Vec<_>>().join("/");
            (path, f["match"].as_str().unwrap().to_string(), f["caseDiffers"] == true)
        })
        .collect()
}

/// The same file comes first, then the same size, then the name only; within each, the name in its
/// own letter case, then the folder names the file shares with the saved path, counted from the
/// innermost out up to the first that differs.
#[test]
fn files_found_are_ranked_by_what_the_link_saved() {
    let root = folder("rank");
    let mut s = session_for(&root);
    let (logo, photo) = (png(60, 40, RED), png(50, 30, BLUE));
    write(&root.join("orig/logo.png"), &logo);
    write(&root.join("orig/photo.png"), &photo);
    placed(&mut s, &root.join("orig/logo.png"));
    let photo_id = placed(&mut s, &root.join("orig/photo.png"));
    // A link saved without a hash (as older documents saved them) can't be the same file.
    s.edit("old link", |d, _| {
        if let NodeKind::Image(im) = &mut d.node_mut(NodeId(photo_id)).unwrap().kind {
            im.link.as_mut().unwrap().hash = None;
        }
        Ok(())
    })
    .unwrap();
    std::fs::remove_dir_all(root.join("orig")).unwrap();
    let search = root.join("search");
    let mut changed = logo.clone();
    *changed.last_mut().unwrap() ^= 1;
    let mut longer = logo.clone();
    longer.push(0);
    write(&search.join("a-other/logo.png"), &longer);
    write(&search.join("z/orig/logo.png"), &changed);
    write(&search.join("0/LOGO.PNG"), &logo);
    write(&search.join("a/b/logo.png"), &logo);
    write(&search.join("photo.png"), &photo);
    write(&search.join("q/photo.png"), &[photo.as_slice(), &[0]].concat());
    write(&search.join("readme.txt"), b"logo.png");
    write(&search.join("logo.png.txt"), &logo);
    let r = s.execute("links.findFiles", &json!({"folder": search})).unwrap();
    assert!(matches!(r["state"].as_str(), Some("searching" | "done")), "{r}");
    let r = finished(&mut s);
    assert_eq!((r["state"].as_str(), r["searched"]["read"].as_u64()), (Some("done"), Some(6)), "{r}");
    let row = |(path, kind, case): (&str, &str, bool)| (format!("search/{path}"), kind.to_string(), case);
    let logo_files =
        [("a/b/logo.png", "same", false), ("0/LOGO.PNG", "same", true), ("z/orig/logo.png", "name", false), ("a-other/logo.png", "name", false)];
    assert_eq!(found(&r, "logo.png", &root), logo_files.map(row));
    assert_eq!(found(&r, "photo.png", &root), [("photo.png", "size", false), ("q/photo.png", "name", false)].map(row));
    let photo_row = r["links"].as_array().unwrap().iter().find(|l| l["name"] == "photo.png").unwrap();
    assert_eq!((&photo_row["ids"], &photo_row["files"][0]["size"]), (&json!([photo_id]), &json!(photo.len())));
    assert_eq!(photo_row["path"], json!(root.join("orig/photo.png")));
}

/// Without `ids`, a search looks for the files that can't be found; with them, for those objects'
/// files, whatever their status.
#[test]
fn a_search_looks_for_the_missing_files_or_the_objects_given() {
    let root = folder("which");
    let mut s = session_for(&root);
    write(&root.join("orig/gone.png"), &png(40, 30, RED));
    write(&root.join("orig/here.png"), &png(40, 30, BLUE));
    let gone = placed(&mut s, &root.join("orig/gone.png"));
    let here = placed(&mut s, &root.join("orig/here.png"));
    std::fs::remove_file(root.join("orig/gone.png")).unwrap();
    let search = root.join("search");
    std::fs::create_dir_all(&search).unwrap();
    let names = |r: &Value| r["links"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let r = s.execute("links.findFiles", &json!({"folder": search})).unwrap();
    assert_eq!((names(&r), &r["links"][0]["ids"]), (vec!["gone.png".to_string()], &json!([gone])));
    let r = s.execute("links.findFiles", &json!({"folder": search, "ids": [here]})).unwrap();
    assert_eq!(names(&r), ["here.png"]);
    assert_eq!(finished(&mut s)["links"][0]["files"], json!([]));
    // What isn't an image, an embedded image, and a document with nothing missing.
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    let embedded = s.execute("file.place", &json!({"path": root.join("orig/here.png"), "link": false})).unwrap()["ids"][0].as_u64().unwrap();
    for (ids, why) in [(json!([rect]), "is not an image"), (json!([embedded]), "show no linked files"), (json!([u64::MAX]), "no such object")] {
        let r = s.execute("links.findFiles", &json!({"folder": search, "ids": ids}));
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains(why)), "{ids}: {r:?}");
    }
    write(&root.join("orig/gone.png"), &png(40, 30, RED));
    let r = s.execute("links.findFiles", &json!({"folder": search}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains("no linked files are missing")), "{r:?}");
    let r = s.execute("links.findFiles", &json!({"folder": "relative/folder", "ids": [here]}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains("absolute")), "{r:?}");
    // Without a document: the last search's state, and no new search.
    let mut none = Session::new();
    assert_eq!(none.execute("links.findFiles", &json!({})).unwrap(), json!({"state": "idle", "links": []}));
    assert!(matches!(none.execute("links.findFiles", &json!({"folder": search})), Err(EngineError::BadParams { .. })));
}

/// A search ends once every linked file has its same file, and only then: a file of the name with
/// other contents leaves it to search every folder.
#[test]
fn a_search_ends_once_every_linked_file_has_its_same_file() {
    let root = folder("early");
    let mut s = session_for(&root);
    s.search_threads = Some(1);
    let bytes = png(40, 30, RED);
    write(&root.join("orig/logo.png"), &bytes);
    placed(&mut s, &root.join("orig/logo.png"));
    std::fs::remove_file(root.join("orig/logo.png")).unwrap();
    let search = root.join("search");
    for a in 0..40 {
        for b in 0..40 {
            std::fs::create_dir_all(search.join(format!("f{a}/g{b}"))).unwrap();
        }
    }
    write(&search.join("logo.png"), &png(40, 30, BLUE));
    s.execute("links.findFiles", &json!({"folder": search})).unwrap();
    let r = finished(&mut s);
    assert_eq!((r["state"].as_str(), r["searched"]["folders"].as_u64()), (Some("done"), Some(1641)), "{r}");
    assert_eq!(r["links"][0]["files"][0]["match"], "name");
    write(&search.join("logo.png"), &bytes);
    s.execute("links.findFiles", &json!({"folder": search})).unwrap();
    let r = finished(&mut s);
    assert_eq!(r["links"][0]["files"][0]["match"], "same");
    assert!(r["state"] == "done" && r["searched"]["folders"].as_u64().unwrap() < 100, "{r}");
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
    std::fs::write(root.join("a.png"), b"png").unwrap();
    let mut s = session_for(&root);
    let (tx, rx) = mpsc::channel();
    let search = findfiles::start(root.clone(), s.search_rules.clone(), Limits::default(), 1, 2, Arc::new(Blocked(Mutex::new(rx)))).unwrap();
    let links = vec![Sought { name: "a.png".into(), path: "/gone/a.png".into(), ids: vec![], size: None, hash: None }];
    s.link_search = Some(LinkSearch { id: 7, folder: root.to_string_lossy().into_owned(), links, sizes: Arc::default(), search });
    let until = Instant::now() + Duration::from_secs(10);
    while s.execute("links.findFiles", &json!({})).unwrap()["searched"]["read"] != 1 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(2));
    }
    linkfiles::stop(&mut s, 8);
    assert_eq!(s.execute("links.findFiles", &json!({})).unwrap()["state"], "searching");
    linkfiles::stop(&mut s, 7);
    let r = s.execute("links.findFiles", &json!({})).unwrap();
    assert_eq!((r["state"].as_str(), r["stopped"].as_str(), r["id"].as_u64()), (Some("stopped"), Some("stop"), Some(7)));
    drop(tx);
}

#[test]
fn relinking_with_copy_puts_the_file_in_the_documents_folder() {
    let root = folder("copy");
    let mut s = session_for(&root);
    write(&root.join("orig/photo.png"), &png(40, 30, RED));
    let id = placed(&mut s, &root.join("orig/photo.png"));
    std::fs::create_dir_all(root.join("doc")).unwrap();
    s.execute("document.save", &json!({"path": root.join("doc/poster.vectorcraft")})).unwrap();
    std::fs::remove_dir_all(root.join("orig")).unwrap();
    let (found, blue) = (root.join("found/photo.png"), png(40, 30, BLUE));
    write(&found, &blue);
    let copy = root.join("doc").join("photo.png");
    let relink = |s: &mut Session, path: &Path| s.execute("links.relink", &json!({"ids": [id], "path": path, "copy": true}));
    let r = relink(&mut s, &found).unwrap();
    assert_eq!(r, json!({"relinked": [id], "notFound": [], "copied": [{"from": found, "to": copy}], "kept": []}));
    assert_eq!(std::fs::read(&copy).unwrap(), blue);
    let link = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Image(im) => im.link.clone().unwrap(),
        k => panic!("not an image: {k:?}"),
    };
    assert_eq!((link(&s).path, link(&s).hash), (copy.to_string_lossy().into_owned(), Some(vectorcraft_doc::links::hash_bytes(&blue))));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Relink");
    // The same file again: the copy is used.
    let r = relink(&mut s, &found).unwrap();
    assert_eq!((&r["copied"], &r["kept"]), (&json!([]), &json!([{"from": found, "to": copy}])));
    assert!(!root.join("doc/photo 2.png").exists());
    // Another file of the name: the copy gets a number, and the file there stays as it was.
    let (other, red) = (root.join("other/photo.png"), png(50, 30, RED));
    write(&other, &red);
    let numbered = root.join("doc").join("photo 2.png");
    assert_eq!(relink(&mut s, &other).unwrap()["copied"], json!([{"from": other, "to": numbered}]));
    assert_eq!((std::fs::read(&copy).unwrap(), std::fs::read(&numbered).unwrap()), (blue, red));
    // That file again: the numbered copy holds the same contents and is used.
    assert_eq!(relink(&mut s, &other).unwrap()["kept"], json!([{"from": other, "to": numbered}]));
    assert!(!root.join("doc/photo 3.png").exists());
    // A file in the document's folder is linked where it is.
    assert_eq!(relink(&mut s, &numbered).unwrap()["kept"], json!([{"from": numbered, "to": numbered}]));
    assert_eq!(link(&s).path, numbered.to_string_lossy());
    // A file that can't be shown isn't copied.
    write(&root.join("found/junk.png"), b"not an image");
    assert!(relink(&mut s, &root.join("found/junk.png")).is_err());
    assert!(!root.join("doc/junk.png").exists());
    // Copy takes a file, not a folder, and a saved document.
    let r = s.execute("links.relink", &json!({"ids": [id], "folder": root.join("found"), "copy": true}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains("needs path")), "{r:?}");
    let mut unsaved = session_for(&root);
    let image = placed(&mut unsaved, &found);
    let r = unsaved.execute("links.relink", &json!({"ids": [image], "path": found, "copy": true}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains("save the document first")), "{r:?}");
    let _ = std::fs::remove_dir_all(&root);
}

/// Copy takes images only: with copy, a call that includes a placed document fails and copies
/// nothing; without it, the document is relinked where its file is.
#[test]
fn copy_refuses_placed_documents() {
    let root = folder("copy-placed");
    let mut s = session_for(&root);
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    let badge = root.join("badge.vectorcraft");
    s.execute("document.save", &json!({"path": badge})).unwrap();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = s.execute("file.place", &json!({"path": badge, "link": true})).unwrap()["ids"][0].clone();
    std::fs::create_dir_all(root.join("doc")).unwrap();
    s.execute("document.save", &json!({"path": root.join("doc/poster.vectorcraft")})).unwrap();
    let r = s.execute("links.relink", &json!({"ids": [id], "path": badge, "copy": true}));
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg.contains("images only")), "{r:?}");
    assert!(!root.join("doc/badge.vectorcraft").exists());
    assert_eq!(s.execute("links.relink", &json!({"ids": [id], "path": badge})).unwrap()["relinked"], json!([id]));
}

/// With automation roots in force, a copy is written only where the write root allows, whichever
/// name it takes: a link of the file's name into the write root doesn't let a numbered copy land
/// beside it.
#[cfg(unix)]
#[test]
fn a_copy_stays_inside_the_write_root() {
    let root = folder("roots");
    let mut s = session_for(&root);
    write(&root.join("orig/photo.png"), &png(40, 30, RED));
    let id = placed(&mut s, &root.join("orig/photo.png"));
    std::fs::create_dir_all(root.join("doc")).unwrap();
    s.execute("document.save", &json!({"path": root.join("doc/poster.vectorcraft")})).unwrap();
    let found = root.join("found/photo.png");
    write(&found, &png(40, 30, BLUE));
    let allowed = root.join("allowed");
    write(&allowed.join("photo.png"), b"kept as it is");
    std::os::unix::fs::symlink(allowed.join("photo.png"), root.join("doc/photo.png")).unwrap();
    let roots = crate::file_access::AutomationRoots::new(&[&root], &[&allowed]).unwrap();
    let r = crate::file_access::confine(roots.as_ref(), || s.execute("links.relink", &json!({"ids": [id], "path": found, "copy": true})));
    assert!(matches!(&r, Err(EngineError::Other(m)) if m.contains("outside the write root")), "{r:?}");
    assert!(!root.join("doc/photo 2.png").exists());
    assert_eq!(std::fs::read(allowed.join("photo.png")).unwrap(), b"kept as it is");
    let _ = std::fs::remove_dir_all(&root);
}

/// An SVG's linked files are read on the importer's own thread (it needs a deep stack): the roots
/// in force on the thread that opens the SVG hold there too.
#[test]
fn an_svg_reads_its_links_within_the_read_root() {
    let root = folder("svg-roots");
    for dir in ["in", "out"] {
        write(&root.join(dir).join("dot.png"), &png(4, 4, RED));
    }
    // What came of an SVG linking to the picture in `dir`: the pictures read, and the warnings.
    let import = |dir: &str| {
        let href = root.join(dir).join("dot.png").to_string_lossy().replace('\\', "/");
        let svg =
            format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><image width=\"4\" height=\"4\" href=\"{href}\"/></svg>");
        let (doc, warnings) = crate::cmd::fileio::import_svg(&svg, None).unwrap();
        (doc.images.len(), warnings)
    };
    let (inside, outside) = (import("in"), import("out"));
    assert_eq!(inside, outside, "both are read with no roots in force");
    let roots = crate::file_access::AutomationRoots::new(&[&root.join("in")], &[]).unwrap();
    crate::file_access::confine(roots.as_ref(), || {
        assert_eq!(import("in"), inside, "inside the read root: read as before");
        assert_ne!(import("out"), outside, "outside the read root: not read");
    });
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn junk_search_params_never_fail_internally() {
    let root = folder("junk");
    let mut s = session_for(&root);
    write(&root.join("orig/a.png"), &png(4, 4, RED));
    let id = placed(&mut s, &root.join("orig/a.png"));
    std::fs::remove_file(root.join("orig/a.png")).unwrap();
    let internal = |r: &crate::Result<Value>| matches!(r, Err(EngineError::Internal { .. }));
    let mut cases = vec![];
    for v in vectorcraft_testkit::strategies::junk_values() {
        cases.extend([
            json!({"folder": root, "maxSeconds": v}),
            json!({"folder": root, "ids": v}),
            json!({"folder": root, "ids": [v]}),
            json!({"folder": root, "stop": v}),
            json!({"folder": v}),
        ]);
        for p in [
            json!({"ids": [id], "path": root.join("b.png"), "copy": v}),
            json!({"ids": [id], "folder": root, "copy": v}),
            json!({"ids": v, "path": root.join("b.png"), "copy": true}),
        ] {
            let r = s.execute("links.relink", &p);
            assert!(!internal(&r), "{p}: {r:?}");
        }
    }
    cases.extend([
        json!({"folder": "/a\u{0}b"}),
        json!({"folder": "relative/folder"}),
        json!({"folder": "/\u{FFFD}"}),
        json!({"folder": format!("/{}", "x".repeat(4096))}),
        json!({"folder": root.join("No Such Folder")}),
        json!({"folder": root, "ids": [-1]}),
        json!({"folder": root, "ids": [id, id]}),
    ]);
    let mut started = 0;
    for p in &cases {
        let r = s.execute("links.findFiles", p);
        assert!(!internal(&r), "{p}: {r:?}");
        if r.is_ok() && p.get("folder").is_some_and(Value::is_string) {
            started += 1;
            assert_ne!(finished(&mut s)["state"], "searching", "{p}");
        }
    }
    assert!(started > 30, "only {started} searches started");
}
