//! Paths stored with linked files and SVG images are followed only on this computer or on the
//! document's own share (`fileio::document_path`): opening a document, Link Info, watching linked
//! files, and the images an SVG file links to.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::cmd::fileio::touched;
use crate::tests_links::{Folder, RED, place, png, session};
use crate::*;

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

/// The final path Windows gives a folder of the drive `Z:` mapped to `\\server\share`. The
/// share's root folder has no trailing separator (`\\?\UNC\server\share`).
fn on_share(folder: &str) -> Option<std::path::PathBuf> {
    let rest = folder.get(2..)?.trim_start_matches(['\\', '/']).replace('/', "\\");
    let root = r"\\?\UNC\server\share";
    Some(if rest.is_empty() { root.into() } else { format!(r"{root}\{rest}").into() })
}

#[test]
fn a_document_on_a_mapped_drive_is_on_the_share_behind_it() {
    use crate::cmd::fileio::share_of;
    // craft-docpath refuses a link on the share when the document is given by its drive letter.
    let link = r"\\server\share\img\logo.png";
    assert_eq!(craft_docpath::resolve(link, Some(std::path::Path::new(r"Z:\jobs\doc.vectorcraft"))), Err(craft_docpath::Refused::Network));
    assert!(craft_docpath::resolve(link, Some(std::path::Path::new(r"\\server\share\jobs\doc.vectorcraft"))).is_ok());

    // The document's path on the share, from its folder's final path.
    let mut asked = vec![];
    let mut ask = |folder: &str| {
        asked.push(folder.to_string());
        on_share(folder)
    };
    assert_eq!(share_of(r"Z:\jobs\doc.vectorcraft", &mut ask).as_deref(), Some(r"\\server\share\jobs\doc.vectorcraft"));
    assert_eq!(share_of(r"Z:\doc.vectorcraft", &mut ask).as_deref(), Some(r"\\server\share\doc.vectorcraft"));
    assert_eq!(share_of("Z:/jobs/doc.vectorcraft", &mut ask).as_deref(), Some(r"\\server\share\jobs\doc.vectorcraft"));
    assert_eq!(asked, [r"Z:\jobs", r"Z:\", "Z:/jobs"]);
    let unc = |p: &'static str| move |_: &str| Some(std::path::PathBuf::from(p));
    assert_eq!(share_of(r"Z:\doc.vectorcraft", unc(r"\\?\unc\server\share\")).as_deref(), Some(r"\\server\share\doc.vectorcraft"));

    // A folder on a local volume (also behind a `subst` drive), a folder that can't be opened,
    // and a final path without a share name give no path on a share.
    assert_eq!(share_of(r"C:\art\doc.vectorcraft", unc(r"\\?\C:\art")), None);
    assert_eq!(share_of(r"S:\doc.vectorcraft", unc(r"\\?\D:\real")), None);
    assert_eq!(share_of(r"Z:\gone\doc.vectorcraft", |_| None), None);
    assert_eq!(share_of(r"Z:\doc.vectorcraft", unc(r"\\?\UNC\server")), None);
    // A document given by its share, or not by a drive, is not looked up.
    for doc in [
        r"\\server\share\doc.vectorcraft",
        r"\\?\Z:\doc.vectorcraft",
        "/Users/a/doc.vectorcraft",
        "doc.vectorcraft",
        "Z:doc.vectorcraft",
        r"Z:\jobs\",
    ] {
        assert_eq!(share_of(doc, |f: &str| panic!("{doc}: looked up {f}")), None, "{doc}");
    }
}

/// The final path Windows gives a folder on a drive: `Z:` and `V:` are mapped to `\\server\share`,
/// `Y:` is mapped to `\\server\other` and `W:` is mapped to `\\far\jobs`; `C:` and `D:` are volumes
/// of this computer.
fn mapped(folder: &str) -> Option<std::path::PathBuf> {
    let root = match folder.get(..1)?.to_ascii_uppercase().as_str() {
        "Z" | "V" => r"\\?\UNC\server\share",
        "Y" => r"\\?\UNC\server\other",
        "W" => r"\\?\UNC\far\jobs",
        "C" => r"\\?\C:",
        "D" => r"\\?\D:",
        _ => return None,
    };
    let rest = folder.get(2..)?.trim_start_matches(['\\', '/']).replace('/', "\\");
    Some(if rest.is_empty() { root.into() } else { format!(r"{root}\{rest}").into() })
}

#[test]
fn a_path_on_a_mapped_drive_is_on_the_share_behind_it() {
    use crate::cmd::fileio::drive_share_of;
    // Only the drive's root folder is looked up, so a path in a folder that doesn't exist still gets
    // its path on the share.
    let mut asked = vec![];
    let mut ask = |root: &str| {
        asked.push(root.to_string());
        mapped(root)
    };
    assert_eq!(drive_share_of(r"Y:\art\gone\x.png", &mut ask).as_deref(), Some(r"\\server\other\art\gone\x.png"));
    assert_eq!(drive_share_of(r"Y:\x.png", &mut ask).as_deref(), Some(r"\\server\other\x.png"));
    assert_eq!(drive_share_of(r"\\?\Y:\x.png", &mut ask).as_deref(), Some(r"\\server\other\x.png"));
    assert_eq!(drive_share_of("y:/art/x.png", &mut ask).as_deref(), Some(r"\\server\other\art\x.png"));
    assert_eq!(asked, [r"Y:\", r"Y:\", r"Y:\", "y:/"]);
    // A volume of this computer, and a drive that can't be opened, give no path on a share.
    assert_eq!(drive_share_of(r"D:\art\x.png", mapped), None);
    assert_eq!(drive_share_of(r"Q:\art\x.png", mapped), None);
    // UNC, root-relative, relative and drive-relative paths, and a drive's root, are not looked up.
    for path in [r"\\server\other\x.png", r"\\?\UNC\server\other\x.png", "/Users/a/x.png", "x.png", "Y:x.png", r"Y:\"] {
        assert_eq!(drive_share_of(path, |r: &str| panic!("{path}: looked up {r}")), None, "{path}");
    }
}

#[cfg(windows)]
#[test]
fn links_on_a_mapped_drives_share_are_followed() {
    use crate::cmd::fileio::{document_path_with, share_of};
    let doc = Some(r"Z:\jobs\doc.vectorcraft");
    let share = |d: &str| share_of(d, on_share);
    // A link on the share behind the drive is followed; one on another share still isn't.
    assert_eq!(document_path_with(r"\\server\share\img\logo.png", doc, share).as_deref(), Ok(r"\\server\share\img\logo.png"));
    assert_eq!(document_path_with(r"\\SERVER\Share\logo.png", doc, share).as_deref(), Ok(r"\\SERVER\Share\logo.png"));
    assert_eq!(document_path_with(r"\\other\share\logo.png", doc, share), Err(craft_docpath::Refused::Network));
    assert_eq!(document_path_with("file://server/share/logo.png", doc, share), Err(craft_docpath::Refused::Network));
    // On a local volume, the link stays refused.
    assert_eq!(document_path_with(r"\\server\share\logo.png", doc, |_| None), Err(craft_docpath::Refused::Network));
    // Links that were followed resolve as before, without looking the share up.
    let unasked = |d: &str| -> Option<String> { panic!("looked up {d}") };
    assert_eq!(document_path_with(r"img\logo.png", doc, unasked).as_deref(), Ok(r"Z:\jobs\img\logo.png"));
    assert_eq!(document_path_with(r"D:\art\logo.png", doc, unasked).as_deref(), Ok(r"D:\art\logo.png"));
}

#[cfg(windows)]
#[test]
fn links_on_a_drive_mapped_to_another_share_are_not_followed() {
    use crate::cmd::fileio::{document_path_on, drive_share_of, share_of};
    let share = |d: &str| share_of(d, mapped);
    let drive = |p: &str| drive_share_of(p, mapped);
    let refused = Err(craft_docpath::Refused::Network);
    // From a document on this computer, a drive mapped to any share is another computer.
    let local = Some(r"C:\art\doc.vectorcraft");
    for link in [r"Y:\img\logo.png", r"Z:\logo.png", r"\\?\Y:\logo.png", "file:///Y:/logo.png", r"Y:\gone\logo.png"] {
        assert_eq!(document_path_on(link, local, share, drive), refused, "{link}");
    }
    // Links on a mapped drive are refused from a document on a third share, given by its drive or
    // by its share, and from a document without a path.
    for doc in [Some(r"W:\jobs\doc.vectorcraft"), Some(r"\\far\jobs\doc.vectorcraft"), None] {
        assert_eq!(document_path_on(r"Y:\img\logo.png", doc, share, drive), refused, "{doc:?}");
        assert_eq!(document_path_on(r"Z:\img\logo.png", doc, share, drive), refused, "{doc:?}");
    }
}

#[cfg(windows)]
#[test]
fn links_on_the_documents_share_or_this_computer_are_followed() {
    use crate::cmd::fileio::{document_path_on, drive_share_of, share_of};
    let share = |d: &str| share_of(d, mapped);
    let drive = |p: &str| drive_share_of(p, mapped);
    // Links on the document's share are followed: through another drive mapped to it, by the share's
    // own path, and from the document given by its share.
    let doc = Some(r"Z:\jobs\doc.vectorcraft");
    assert_eq!(document_path_on(r"V:\img\logo.png", doc, share, drive).as_deref(), Ok(r"V:\img\logo.png"));
    let by_share = Some(r"\\server\share\jobs\doc.vectorcraft");
    assert_eq!(document_path_on(r"Z:\img\logo.png", by_share, share, drive).as_deref(), Ok(r"Z:\img\logo.png"));
    assert_eq!(document_path_on(r"\\server\share\logo.png", doc, share, drive).as_deref(), Ok(r"\\server\share\logo.png"));
    // A document written `\\?\Z:\…` is on the same share.
    let verbatim = Some(r"\\?\Z:\jobs\doc.vectorcraft");
    assert_eq!(document_path_on(r"V:\img\logo.png", verbatim, share, drive).as_deref(), Ok(r"V:\img\logo.png"));
    assert_eq!(document_path_on(r"\\server\share\logo.png", verbatim, share, drive).as_deref(), Ok(r"\\server\share\logo.png"));
    assert_eq!(document_path_on(r"Y:\img\logo.png", verbatim, share, drive), Err(craft_docpath::Refused::Network));
    // Links on this computer and on a drive that can't be opened are followed, from a document
    // anywhere.
    for doc in [Some(r"C:\art\doc.vectorcraft"), doc, Some(r"\\far\jobs\doc.vectorcraft"), None] {
        assert_eq!(document_path_on(r"D:\art\logo.png", doc, share, drive).as_deref(), Ok(r"D:\art\logo.png"), "{doc:?}");
        assert_eq!(document_path_on(r"Q:\art\logo.png", doc, share, drive).as_deref(), Ok(r"Q:\art\logo.png"), "{doc:?}");
    }
    // A path on the document's own drive is not looked up.
    let unasked = |p: &str| -> Option<String> { panic!("looked up {p}") };
    assert_eq!(document_path_on(r"img\logo.png", doc, share, unasked).as_deref(), Ok(r"Z:\jobs\img\logo.png"));
    assert_eq!(document_path_on(r"z:\logo.png", doc, share, unasked).as_deref(), Ok(r"z:\logo.png"));
    assert_eq!(document_path_on(r"C:\x\logo.png", Some(r"C:\art\doc.vectorcraft"), share, unasked).as_deref(), Ok(r"C:\x\logo.png"));
}

#[test]
fn a_document_off_a_mapped_drive_keeps_its_path() {
    use crate::cmd::fileio::{drive_share_path, share_path};
    let dir = Folder::new("docpath-share");
    // A folder on this computer's own disk, and one that doesn't exist.
    assert_eq!(share_path(&dir.file("doc.vectorcraft")), None);
    assert_eq!(share_path(&dir.file("gone/doc.vectorcraft")), None);
    assert_eq!(drive_share_path(&dir.file("gone/x.png")), None);
    // Elsewhere than Windows, no document or link is looked up.
    if cfg!(not(windows)) {
        assert_eq!(share_path(r"Z:\jobs\doc.vectorcraft"), None);
        assert_eq!(drive_share_path(r"Y:\img\logo.png"), None);
    }
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
