//! The swatch, brush, symbol and graphic style library commands under automation roots: a confined
//! request sees a library's file, the user library folder and the path in a loaded library's id only
//! inside its read roots, and the id it is shown finds the library.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::file_access::{AutomationRoots, confine};
use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn text(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// A fresh scratch folder for one test, with the folders `in`, `out`, `a`, `b` and `user`.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-library-roots-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for sub in ["in", "out", "a", "b", "user"] {
        std::fs::create_dir_all(dir.join(sub)).unwrap();
    }
    dir
}

/// One kind of library and its commands.
struct Kind {
    name: &'static str,
    list: &'static str,
    get: &'static str,
    save: &'static str,
    load: &'static str,
    ext: &'static str,
    /// Adds an item named by the second argument to the document.
    add: fn(&mut Session, &str),
    set_user_dir: fn(&mut Session, Option<String>),
}

fn rectangle(s: &mut Session) -> u64 {
    run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}))["id"].as_u64().unwrap()
}

const SWATCHES: Kind = Kind {
    name: "swatches",
    list: "swatch.library.list",
    get: "swatch.library.get",
    save: "swatch.library.save",
    load: "swatch.library.load",
    ext: "vcswatches",
    add: |s, name| {
        run(s, "swatch.new", json!({"name": name, "color": "#3366cc"}));
    },
    set_user_dir: |s, dir| s.swatch_libraries.set_user_dir(dir),
};

const BRUSHES: Kind = Kind {
    name: "brushes",
    list: "brush.libraries",
    get: "brush.library",
    save: "brush.saveLibrary",
    load: "brush.loadLibrary",
    ext: crate::cmd::brushlib::BRUSHES_EXT,
    add: |s, name| {
        run(s, "brush.new", json!({"type": "calligraphic", "name": name, "params": {"angle": 20, "roundness": 40, "size": 7}}));
    },
    set_user_dir: |s, dir| s.brush_libraries.set_user_dir(dir),
};

const SYMBOLS: Kind = Kind {
    name: "symbols",
    list: "symbol.libraries",
    get: "symbol.library",
    save: "symbol.saveLibrary",
    load: "symbol.loadLibrary",
    ext: crate::cmd::symbollib::SYMBOLS_EXT,
    add: |s, name| {
        let id = rectangle(s);
        run(s, "symbol.new", json!({"name": name, "ids": [id]}));
    },
    set_user_dir: |s, dir| s.symbol_libraries.set_user_dir(dir),
};

const STYLES: Kind = Kind {
    name: "styles",
    list: "graphicStyle.libraries",
    get: "graphicStyle.library",
    save: "graphicStyle.saveLibrary",
    load: "graphicStyle.loadLibrary",
    ext: vectorcraft_doc::style_libs::STYLES_EXT,
    add: |s, name| {
        let id = rectangle(s);
        run(s, "graphicStyle.new", json!({"name": name, "id": id}));
    },
    set_user_dir: |s, dir| s.style_libraries.set_user_dir(dir),
};

/// The loaded libraries of a list reply: id, path and count.
fn loaded(list: &Value) -> Vec<(Value, Value, Value)> {
    let libs = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "loaded");
    libs.map(|l| (l["id"].clone(), l["path"].clone(), l["count"].clone())).collect()
}

/// Does `reply` hold `path` anywhere, as JSON writes it?
fn holds(reply: &Value, path: &Path) -> bool {
    let written = json!(text(path)).to_string();
    reply.to_string().contains(written.trim_matches('"'))
}

/// Three libraries the person loaded, one inside the client's read root and two outside it in
/// different folders under the same file name (the one in `b` holds one more item), and a user
/// library folder outside the root. → the session, the folder and the three files.
fn three_libraries(kind: &Kind, test: &str) -> (Session, PathBuf, [PathBuf; 3]) {
    let base = folder(&format!("{}-{test}", kind.name));
    let file = format!("Kit.{}", kind.ext);
    let files = [base.join("in").join(&file), base.join("a").join(&file), base.join("b").join(&file)];
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    (kind.add)(&mut s, "First");
    for path in &files[..2] {
        run(&mut s, kind.save, json!({"path": text(path), "name": "Kit"}));
    }
    (kind.add)(&mut s, "Second");
    run(&mut s, kind.save, json!({"path": text(&files[2]), "name": "Kit"}));
    (kind.set_user_dir)(&mut s, Some(text(&base.join("user"))));
    for path in &files {
        run(&mut s, kind.load, json!({"path": text(path)}));
    }
    (s, base, files)
}

fn lists_leave_out_paths_outside_the_read_root(kind: &Kind) {
    let (mut s, base, [inside, a, b]) = three_libraries(kind, "list");
    let file = format!("Kit.{}", kind.ext);
    let person = run(&mut s, kind.list, json!({}));
    let rows = loaded(&person);
    assert_eq!(rows.len(), 3, "{person}");
    let (small, large) = (rows[1].2.clone(), rows[2].2.clone());
    assert_ne!(small, large, "{person}");
    let row = |p: &Path, count: &Value| (json!(format!("loaded/{}", text(p))), json!(text(p)), count.clone());
    assert_eq!(rows, [row(&inside, &small), row(&a, &small), row(&b, &large)], "the person sees every path: {person}");
    assert_eq!(person["userFolder"], json!(text(&base.join("user"))));

    // The client sees the paths inside its read root; the others' rows stay, in the same order,
    // under the file's name, numbered in the order of their full ids.
    let roots = AutomationRoots::new(&[&base.join("in")], &[&base.join("out")]).unwrap();
    let client = confine(roots.as_ref(), || run(&mut s, kind.list, json!({})));
    let shown = [format!("loaded/{file}"), format!("loaded/{file} (2)")];
    let hidden = |id: &str, count: &Value| (json!(id), Value::Null, count.clone());
    assert_eq!(loaded(&client), [row(&inside, &small), hidden(&shown[0], &small), hidden(&shown[1], &large)], "{client}");
    assert_eq!(client["userFolder"], Value::Null, "{client}");
    for outside in ["a", "b", "user", "out"] {
        assert!(!holds(&client, &base.join(outside)), "no path outside the read root: {client}");
    }
    let everything = |l: &Value| l["libraries"].as_array().unwrap().iter().map(|l| (l["name"].clone(), l["category"].clone())).collect::<Vec<_>>();
    assert_eq!(everything(&client), everything(&person), "every library is still listed");

    // The id shown finds its library, and replies about it show that id.
    let got = confine(roots.as_ref(), || run(&mut s, kind.get, json!({"library": shown[1]})));
    assert_eq!(got["id"], json!(shown[1]), "{got}");
    assert!(!holds(&got, &base.join("b")), "{got}");
    let got = confine(roots.as_ref(), || run(&mut s, kind.get, json!({"library": shown[0]})));
    assert_eq!(got["id"], json!(shown[0]), "{got}");
    assert!(s.execute(kind.get, &json!({"library": shown[0]})).is_err(), "the person has no such id");
    assert_eq!(run(&mut s, kind.list, json!({})), person, "the person's list is not changed");

    // A write root alone lets the client read none of them.
    let write_only = AutomationRoots::new(&[], &[&base.join("in")]).unwrap();
    let client = confine(write_only.as_ref(), || run(&mut s, kind.list, json!({})));
    assert!(loaded(&client).iter().all(|(_, path, _)| path.is_null()), "{client}");
    assert!(!holds(&client, &base), "{client}");
    let _ = std::fs::remove_dir_all(base);
}

#[test]
fn swatch_library_list_leaves_out_paths_outside_the_read_root() {
    lists_leave_out_paths_outside_the_read_root(&SWATCHES);
}

#[test]
fn brush_libraries_leave_out_paths_outside_the_read_root() {
    lists_leave_out_paths_outside_the_read_root(&BRUSHES);
}

#[test]
fn symbol_libraries_leave_out_paths_outside_the_read_root() {
    lists_leave_out_paths_outside_the_read_root(&SYMBOLS);
}

#[test]
fn graphic_style_libraries_leave_out_paths_outside_the_read_root() {
    lists_leave_out_paths_outside_the_read_root(&STYLES);
}

/// Adding from a concealed library, copying it into the user library folder and saving into that
/// folder show neither the library's file nor the folder outside the read root.
#[test]
fn swatch_library_replies_leave_out_paths_outside_the_read_root() {
    let (mut s, base, [inside, ..]) = three_libraries(&SWATCHES, "replies");
    let roots = AutomationRoots::new(&[&base.join("in")], &[&base.join("out")]).unwrap();
    let shown = "loaded/Kit.vcswatches";
    let added = confine(roots.as_ref(), || run(&mut s, "swatch.library.add", json!({"library": shown, "names": ["First"]})));
    assert_eq!(added["library"], shown, "{added}");
    // A library whose file is outside the root is not copied, and the error does not name the file.
    let refused = confine(roots.as_ref(), || s.execute("swatch.library.copyToUser", &json!({"library": shown})));
    let message = refused.map(|r| r.to_string()).unwrap_err().to_string();
    assert!(message.contains("outside the read roots"), "{message}");
    assert!(!message.contains(&text(&base)) && !message.contains("Kit.vcswatches"), "{message}");
    assert!(!base.join("user/Kit.vcswatches").exists());
    // The file inside the root is copied into the user library folder, which is outside it.
    let mine = format!("loaded/{}", text(&inside));
    let copied = confine(roots.as_ref(), || run(&mut s, "swatch.library.copyToUser", json!({"library": mine})));
    assert_eq!((copied["library"].as_str(), copied["copied"].as_bool()), (Some("user/Kit.vcswatches"), Some(true)), "{copied}");
    assert_eq!(copied["path"], Value::Null, "{copied}");
    let saved = confine(roots.as_ref(), || run(&mut s, "swatch.library.save", json!({"user": true, "name": "Mine"})));
    assert_eq!((saved["library"].as_str(), saved.get("path")), (Some("user/Mine.vcswatches"), None), "{saved}");
    for reply in [&added, &copied, &saved] {
        assert!(!holds(reply, &base.join("a")) && !holds(reply, &base.join("user")), "{reply}");
    }
    // A library loaded from data under the file name keeps its id; each concealed one takes the next
    // free one, so no two rows share an id.
    let data = run(&mut s, "swatch.library.save", json!({}))["data"].as_str().unwrap().to_string();
    run(&mut s, "swatch.library.load", json!({"data": data, "name": "Kit.vcswatches"}));
    let list = confine(roots.as_ref(), || run(&mut s, "swatch.library.list", json!({})));
    let ids: Vec<Value> = loaded(&list).into_iter().map(|(id, ..)| id).collect();
    assert_eq!(ids, [json!("loaded/Kit.vcswatches (2)"), json!("loaded/Kit.vcswatches (3)"), json!("loaded/Kit.vcswatches")], "{list}");
    // Copied into the user library folder, it is still found by that id, which no other library takes.
    confine(roots.as_ref(), || run(&mut s, "swatch.library.copyToUser", json!({"library": "loaded/Kit.vcswatches"})));
    let list = confine(roots.as_ref(), || run(&mut s, "swatch.library.list", json!({})));
    let ids: Vec<Value> = loaded(&list).into_iter().map(|(id, ..)| id).collect();
    assert_eq!(ids, [json!("loaded/Kit.vcswatches (2)"), json!("loaded/Kit.vcswatches (3)")], "{list}");
    // The person sees the folder.
    let saved = run(&mut s, "swatch.library.save", json!({"user": true, "name": "Theirs"}));
    assert_eq!(saved["path"], json!(text(&base.join("user").join("Theirs.vcswatches"))));
    let _ = std::fs::remove_dir_all(base);
}
