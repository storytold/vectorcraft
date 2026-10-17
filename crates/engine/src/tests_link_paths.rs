//! Paths stored with linked files and SVG images are followed only on this computer or on the
//! document's own share (`fileio::document_path`): opening a document, Link Info, watching linked
//! files, and the images an SVG file links to.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use super::*;
use crate::cmd::fileio::touched;
use crate::tests_links::{Folder, RED, place, png, session};

/// Paths to other computers (and, on Unix, a local file written as one: `//tmp/…`, which Linux
/// reads as `/tmp/…`).
fn far(dir: &Folder) -> Vec<String> {
    let mut out = vec![r"\\192.0.2.1\share\x.png".to_string(), "file://192.0.2.1/s/x.png".to_string()];
    if cfg!(unix) {
        out.push(format!("/{}", dir.file("far/far.png")));
    }
    out
}

/// Did any path read reach another computer (or the local file written as one)?
fn far_reads(seen: &[String]) -> Vec<&String> {
    seen.iter().filter(|p| p.contains("192.0.2.1") || p.starts_with("//") || p.starts_with(r"\\")).collect()
}

/// A document saved at `dir/doc.vectorcraft` with one linked image per path in `paths`, each
/// showing `dir/pic.png` until its link is set → the document's path.
fn save_linked(dir: &Folder, paths: &[String]) -> String {
    let pic = dir.file("pic.png");
    std::fs::write(&pic, png(8, 8, RED)).unwrap();
    let mut s = session();
    let ids: Vec<NodeId> = paths.iter().map(|_| place(&mut s, &pic)).collect();
    let mut doc = (*s.doc().unwrap().doc).clone();
    for (id, path) in ids.iter().zip(paths) {
        let Some(NodeKind::Image(im)) = doc.node_mut(*id).map(|n| &mut n.kind) else { panic!("not an image") };
        im.link = Some(vectorcraft_doc::LinkInfo::new(path.clone()));
    }
    let path = dir.file("doc.vectorcraft");
    std::fs::write(&path, vectorcraft_format::save(&doc, false)).unwrap();
    path
}

fn names(rows: &Value) -> Vec<&str> {
    rows.as_array().unwrap().iter().map(|r| r["path"].as_str().unwrap()).collect()
}

#[test]
fn linked_files_on_other_computers_are_never_read() {
    let dir = Folder::new("docpath-open");
    std::fs::create_dir_all(dir.0.join("far")).unwrap();
    std::fs::write(dir.file("far/far.png"), png(8, 8, RED)).unwrap();
    std::fs::write(dir.file("near.png"), png(8, 8, RED)).unwrap();
    let far = far(&dir);
    let mut paths = far.clone();
    paths.push("near.png".into());
    let doc = save_linked(&dir, &paths);

    let mut s = Session::new();
    let (r, seen) = touched(|| s.execute("document.open", &json!({"path": doc})).unwrap());
    assert_eq!(far_reads(&seen), Vec::<&String>::new());
    // The far paths are missing, saying why; the file beside the document is found.
    assert_eq!(names(&r["missingLinks"]), far.iter().map(String::as_str).collect::<Vec<_>>(), "{r}");
    for row in r["missingLinks"].as_array().unwrap() {
        assert_eq!(row["refused"], "the path is on another computer", "{row}");
    }
    let check = s.execute("links.check", &json!({})).unwrap();
    let near = check["links"].as_array().unwrap().iter().find(|l| l["name"] == "near.png").unwrap();
    assert_ne!(near["status"], "missing", "{check}");

    // Link Info and the linked-file watch don't look at them either.
    let ids: Vec<u64> = r["missingLinks"].as_array().unwrap().iter().map(|row| row["ids"][0].as_u64().unwrap()).collect();
    let (_, seen) = touched(|| {
        for id in &ids {
            let info = s.execute("links.info", &json!({"id": id})).unwrap();
            assert_eq!(info["refused"], "the path is on another computer", "{info}");
        }
        s.execute("links.updateChanged", &json!({})).unwrap();
        s.start_link_scan();
        let t0 = std::time::Instant::now();
        while s.poll_link_scan().is_none() {
            assert!(t0.elapsed().as_secs() < 10, "the look never finished");
            std::thread::yield_now();
        }
    });
    assert_eq!(far_reads(&seen), Vec::<&String>::new());
    // The watch kept no stamp of them (`//tmp/…` would have one had it been looked at).
    assert!(s.link_stamps.keys().all(|(_, p)| !far.contains(p)), "{:?}", s.link_stamps.keys());
}

#[test]
fn svg_images_on_other_computers_are_never_read() {
    let dir = Folder::new("docpath-svg");
    std::fs::create_dir_all(dir.0.join("far")).unwrap();
    std::fs::write(dir.file("far/far.png"), png(8, 8, RED)).unwrap();
    std::fs::write(dir.file("near.png"), png(8, 8, RED)).unwrap();
    let mut hrefs = vec![r"\\192.0.2.1\share\x.png".to_string(), "file:////192.0.2.1/s/x.png".to_string(), "file://192.0.2.1/s/x.png".to_string()];
    if cfg!(unix) {
        hrefs.push(format!("/{}", dir.file("far/far.png")));
    }
    hrefs.push("near.png".into());
    let images: String = hrefs.iter().map(|h| format!("<image width=\"8\" height=\"8\" href=\"{h}\"/>")).collect();
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\">{images}</svg>");
    let path = dir.file("art.svg");
    std::fs::write(&path, &svg).unwrap();

    let mut s = Session::new();
    let (r, seen) = touched(|| s.execute("document.open", &json!({"path": path})).unwrap());
    assert_eq!(far_reads(&seen), Vec::<&String>::new());
    // Every far image is a placeholder; only the one beside the SVG is read.
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.iter().filter(|w| w.as_str().unwrap().contains("a placeholder keeps its place")).count(), hrefs.len() - 1, "{r}");
    let near = |p: &&String| p.contains("docpath-svg") && p.ends_with("near.png");
    assert!(seen.iter().any(|p| near(&p)), "{seen:?}");

    // Pasted SVG text has no folder: only absolute local paths are read.
    let (_, seen) = touched(|| crate::cmd::fileio::import_svg(&svg, None).unwrap());
    assert_eq!(far_reads(&seen), Vec::<&String>::new());
    assert!(!seen.iter().any(|p| near(&p)), "{seen:?}");
}
