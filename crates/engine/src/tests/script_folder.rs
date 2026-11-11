//! The Scripts folder: what a listing holds and leaves out, its order and labels, its limits, the
//! read of a listed script, `script.listInstalled` and the automation read root.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::json;

use crate::script::folder::{self, Entry, Limits, Listing};
use crate::*;

/// A fresh folder for the files of test `name`, holding an empty `Scripts` folder.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-script-folder-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("Scripts")).unwrap();
    dir
}

/// Writes the empty file `name` (folders included) under `root`.
fn touch(root: &Path, name: &str) -> PathBuf {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "").unwrap();
    path
}

/// Makes `link` lead to the folder `target`: a symbolic link, or on Windows a junction, which needs
/// no privilege.
fn link_folder(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    {
        let made = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(target).output().unwrap();
        assert!(made.status.success(), "mklink /J: {}", String::from_utf8_lossy(&made.stderr));
    }
}

/// A listing's rows as text: `label`, or `label/` followed by the folder's rows in brackets.
fn rows(entries: &[Entry]) -> String {
    let row = |e: &Entry| match e {
        Entry::Script { label, .. } => label.clone(),
        Entry::Folder { label, entries } => format!("{label}/[{}]", rows(entries)),
    };
    entries.iter().map(row).collect::<Vec<_>>().join(", ")
}

/// A listing within `limits` that takes no time.
fn list(root: &Path, limits: Limits) -> Listing {
    folder::list_with(root, &limits, &mut || Duration::ZERO)
}

#[test]
fn scripts_and_subfolders_are_listed_by_name_without_the_extension() {
    let dir = folder("names");
    let root = dir.join("Scripts");
    for name in
        ["b.js", "A.JSX", "Dessin (été) #1.jsx", "v1.2 draft.Js", "Zeta/inner.jsx", "Zeta/Deep/x.js", "notes.txt", "x.jsxbin", "lib.jsxinc", "js"]
    {
        touch(&root, name);
    }
    std::fs::create_dir_all(root.join("Empty/Also Empty")).unwrap();
    let listing = folder::list(&root);
    assert_eq!(rows(&listing.entries), "A, b, Dessin (été) #1, v1.2 draft, Zeta/[Deep/[x], inner]");
    assert!(!listing.truncated && !listing.unreadable);
    // Each script keeps the path the folder gave, whatever its label.
    let paths: Vec<_> =
        listing.scripts().into_iter().map(|(folders, label, path)| (folders.join("/"), label.to_string(), path.to_path_buf())).collect();
    assert_eq!(paths[0], (String::new(), "A".to_string(), root.join("A.JSX")));
    assert_eq!(paths[4], ("Zeta/Deep".to_string(), "x".to_string(), root.join("Zeta").join("Deep").join("x.js")));
    assert_eq!(paths.len(), 6);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_script_whose_label_another_row_has_shows_its_whole_file_name() {
    let dir = folder("same");
    let root = dir.join("Scripts");
    for name in ["tidy.js", "tidy.jsx", "tidy/x.js", "alone.jsx"] {
        touch(&root, name);
    }
    assert_eq!(rows(&folder::list(&root).entries), "alone, tidy/[x], tidy.js, tidy.jsx");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn control_characters_in_a_name_are_written_as_escapes_and_a_long_label_is_cut() {
    let dir = folder("escapes");
    let root = dir.join("Scripts");
    // A right-to-left override, which would show the name backwards.
    touch(&root, "a\u{202E}sj.js");
    let long = "n".repeat(200);
    touch(&root, &format!("{long}.jsx"));
    let listing = folder::list(&root);
    let labels: Vec<&str> = listing.scripts().into_iter().map(|(_, label, _)| label).collect();
    assert_eq!(labels[0], "a\\u{202E}sj");
    assert_eq!(labels[1], format!("{}…", "n".repeat(folder::MAX_LABEL_CHARS - 1)));
    assert!(folder::holds(&root, &root.join(format!("{long}.jsx"))));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn hidden_names_and_links_are_left_out() {
    let dir = folder("hidden");
    let root = dir.join("Scripts");
    touch(&root, "shown.js");
    touch(&root, ".hidden.js");
    touch(&root, ".git/hook.js");
    let outside = touch(&dir, "Outside/secret.js");
    // A link out of the folder, and one back to the folder itself.
    link_folder(outside.parent().unwrap(), &root.join("Out"));
    link_folder(&root, &root.join("Loop"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("linked.js")).unwrap();
    #[cfg(windows)]
    {
        let marked = touch(&root, "marked.js");
        assert!(std::process::Command::new("attrib").arg("+h").arg(&marked).status().unwrap().success());
    }
    let listing = folder::list(&root);
    assert_eq!(rows(&listing.entries), "shown");
    assert!(!listing.truncated && !listing.unreadable);
    // Nor is a file behind a link read as a listed script.
    assert!(folder::read(&root, &root.join("shown.js"), 10).is_some());
    for path in [root.join("Out").join("secret.js"), root.join("Loop").join("shown.js"), root.join(".hidden.js"), root.join("linked.js")] {
        assert!(folder::read(&root, &path, 10).is_none(), "{}", path.display());
    }
    // A script, or the folder it is in, that Windows marks hidden after a listing is not read.
    #[cfg(windows)]
    {
        assert!(folder::read(&root, &root.join("marked.js"), 10).is_none());
        let inner = touch(&root, "Sub/inner.js");
        assert!(folder::read(&root, &inner, 10).is_some());
        assert!(std::process::Command::new("attrib").arg("+h").arg(root.join("Sub")).status().unwrap().success());
        assert!(folder::read(&root, &inner, 10).is_none());
    }
    let _ = std::fs::remove_dir_all(root.join("Out"));
    let _ = std::fs::remove_dir_all(root.join("Loop"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_folder_that_is_missing_is_empty_and_one_that_is_a_file_or_a_link_is_unreadable() {
    let dir = folder("root");
    let root = dir.join("Scripts");
    touch(&root, "a.js");
    assert_eq!(folder::list(&dir.join("None")), Listing::default());
    let file = touch(&dir, "file");
    assert_eq!(folder::list(&file), Listing { unreadable: true, ..Listing::default() });
    let link = dir.join("Link");
    link_folder(&root, &link);
    assert_eq!(folder::list(&link), Listing { unreadable: true, ..Listing::default() });
    assert!(folder::read(&link, &link.join("a.js"), 10).is_none());
    let _ = std::fs::remove_dir_all(&link);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn each_limit_stops_a_listing_with_what_it_found() {
    let dir = folder("limits");
    let root = dir.join("Scripts");
    for n in 0..10 {
        touch(&root, &format!("s{n}.js"));
    }
    let all = Limits::default();
    let entries = list(&root, Limits { entries: 4, ..all });
    assert_eq!((entries.scripts().len(), entries.truncated), (4, true));
    let scripts = list(&root, Limits { scripts: 3, ..all });
    assert_eq!((scripts.scripts().len(), scripts.truncated), (3, true));
    // The clock passes the time limit at the third entry.
    let mut calls = 0;
    let timed = folder::list_with(&root, &all, &mut || {
        calls += 1;
        if calls > 2 { all.time + Duration::from_millis(1) } else { Duration::ZERO }
    });
    assert_eq!((timed.scripts().len(), timed.truncated), (2, true));
    let whole = list(&root, all);
    assert_eq!((whole.scripts().len(), whole.truncated), (10, false));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn subfolders_past_the_fourth_level_or_the_folder_limit_are_left_out() {
    let dir = folder("depth");
    let root = dir.join("Scripts");
    touch(&root, "1/2/3/4/kept.js");
    touch(&root, "1/2/3/4/5/deeper.js");
    let listing = folder::list(&root);
    assert_eq!(rows(&listing.entries), "1/[2/[3/[4/[kept]]]]");
    assert!(listing.truncated);
    assert!(folder::holds(&root, &root.join("1/2/3/4/kept.js")));
    assert!(!folder::holds(&root, &root.join("1/2/3/4/5/deeper.js")));
    let two = folder("folders");
    let root = two.join("Scripts");
    touch(&root, "a/x.js");
    touch(&root, "b/y.js");
    let listing = list(&root, Limits { folders: 1, ..Limits::default() });
    assert_eq!(rows(&listing.entries), "a/[x]");
    assert!(listing.truncated);
    assert!(!folder::list(&root).truncated);
    let _ = std::fs::remove_dir_all(dir);
    let _ = std::fs::remove_dir_all(two);
}

#[test]
fn a_folder_with_more_scripts_than_the_limit_lists_that_many() {
    let dir = folder("many");
    let root = dir.join("Scripts");
    for n in 0..folder::MAX_SCRIPTS + 40 {
        touch(&root, &format!("s{n:04}.js"));
    }
    let listing = list(&root, Limits::default());
    assert_eq!((listing.scripts().len(), listing.truncated), (folder::MAX_SCRIPTS, true));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_read_takes_only_a_script_that_is_still_a_file_under_the_folder() {
    let dir = folder("read");
    let root = dir.join("Scripts");
    let script = touch(&root, "Sub/draw.jsx");
    std::fs::write(&script, "vc.log(1);").unwrap();
    assert_eq!(folder::read(&root, &script, 100).as_deref(), Some(&b"vc.log(1);"[..]));
    assert_eq!(folder::read(&root, &script, 4).as_deref(), Some(&b"vc.l"[..]));
    let outside = touch(&dir, "outside.js");
    touch(&root, "notes.txt");
    for path in [outside, root.join("Sub").join("..").join("..").join("outside.js"), root.join("notes.txt"), root.join("Sub"), root.join("gone.js")] {
        assert!(folder::read(&root, &path, 100).is_none(), "{}", path.display());
    }
    // A folder that took the script's place is not a script.
    std::fs::remove_file(&script).unwrap();
    std::fs::create_dir(&script).unwrap();
    assert!(folder::read(&root, &script, 100).is_none());
    // Nor is the script read once a link stands in for its folder.
    let other = touch(&dir, "Other/draw.jsx");
    std::fs::remove_dir_all(root.join("Sub")).unwrap();
    link_folder(other.parent().unwrap(), &root.join("Sub"));
    assert!(folder::read(&root, &root.join("Sub").join("draw.jsx"), 100).is_none());
    let _ = std::fs::remove_dir_all(root.join("Sub"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn list_installed_gives_the_folder_and_its_scripts_in_menu_order() {
    let dir = folder("command");
    let root = dir.join("Scripts");
    touch(&root, "Tidy Layers.jsx");
    touch(&root, "Export/All Artboards.js");
    let mut s = Session::new();
    assert_eq!(
        s.execute("script.listInstalled", &json!({})).unwrap(),
        json!({"folder": null, "scripts": [], "truncated": false, "unreadable": false})
    );
    s.attach_scripts_dir(root.clone());
    let text = |p: &Path| p.to_string_lossy().into_owned();
    assert_eq!(
        s.execute("script.listInstalled", &json!({})).unwrap(),
        json!({
            "folder": text(&root),
            "scripts": [
                {"path": text(&root.join("Export").join("All Artboards.js")), "label": "All Artboards", "folders": ["Export"]},
                {"path": text(&root.join("Tidy Layers.jsx")), "label": "Tidy Layers", "folders": []},
            ],
            "truncated": false,
            "unreadable": false,
        })
    );
    assert!(!find_command("script.listInstalled").unwrap().journal);
    assert!(s.execute("script.listInstalled", &json!({"folder": "x"})).is_err());
    // A folder that is not there yet lists nothing, and the listing does not make it.
    std::fs::remove_dir_all(&root).unwrap();
    assert_eq!(s.execute("script.listInstalled", &json!({})).unwrap()["scripts"], json!([]));
    assert!(!root.exists());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_installed_script_is_read_as_script_run_reads_a_file() {
    let dir = folder("installed");
    let root = dir.join("Scripts");
    let text = |p: &Path| p.to_string_lossy().into_owned();
    let mut s = Session::new();
    let script = touch(&root, "draw.jsx");
    std::fs::write(&script, "\u{feff}vc.log(1);").unwrap();
    assert_eq!(s.installed_script(&text(&script)).unwrap_err().to_string(), cmd::scripts::NOT_LISTED, "no folder is attached");
    s.attach_scripts_dir(root.clone());
    assert_eq!(s.installed_script(&text(&script)).unwrap(), "vc.log(1);");
    std::fs::write(&script, "@JSXBIN@ES@2.0@").unwrap();
    assert!(s.installed_script(&text(&script)).unwrap_err().to_string().contains("compiled script"));
    std::fs::write(&script, [0xff, 0xfe]).unwrap();
    assert_eq!(s.installed_script(&text(&script)).unwrap_err().to_string(), "the script is not UTF-8 text");
    std::fs::write(&script, vec![b' '; script::MAX_SOURCE_BYTES + 1]).unwrap();
    assert_eq!(s.installed_script(&text(&script)).unwrap_err().to_string(), cmd::scripts::TOO_LARGE);
    std::fs::remove_file(&script).unwrap();
    assert_eq!(s.installed_script(&text(&script)).unwrap_err().to_string(), cmd::scripts::NOT_LISTED);
    let outside = touch(&dir, "outside.js");
    assert_eq!(s.installed_script(&text(&outside)).unwrap_err().to_string(), cmd::scripts::NOT_LISTED);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_automation_read_root_keeps_an_agent_out_of_the_scripts_folder() {
    let dir = folder("roots");
    let root = dir.join("Scripts");
    let script = touch(&root, "draw.js");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let mut s = Session::new();
    s.attach_scripts_dir(root.clone());
    let roots = crate::file_access::AutomationRoots::new(&[&work], &[]).unwrap().unwrap();
    let e = crate::file_access::confine(Some(&roots), || s.execute("script.listInstalled", &json!({}))).unwrap_err();
    assert!(e.to_string().contains("outside the read root"), "{e}");
    let e = crate::file_access::confine(Some(&roots), || s.installed_script(&script.to_string_lossy())).unwrap_err();
    assert!(e.to_string().contains("outside the read root"), "{e}");
    // A read root that holds the folder lets both through.
    let roots = crate::file_access::AutomationRoots::new(&[&dir], &[]).unwrap().unwrap();
    crate::file_access::confine(Some(&roots), || {
        assert_eq!(s.execute("script.listInstalled", &json!({})).unwrap()["scripts"][0]["label"], "draw");
        assert_eq!(s.installed_script(&script.to_string_lossy()).unwrap(), "");
    });
    let _ = std::fs::remove_dir_all(dir);
}
