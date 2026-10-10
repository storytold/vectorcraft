//! The Libraries panel's libraries (#745): making, renaming and deleting them, adding graphics,
//! colours and text styles from the selection, using them in another document, and keeping them
//! in files across sessions.

use serde_json::{Value, json};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers[0].children().map_or(0, |c| c.len())
}

fn hex_of(s: &Session, id: &Value, stroke: bool) -> String {
    let n = s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone();
    let paint = if stroke { n.appearance.stroke().unwrap().paint.clone() } else { n.appearance.fill().unwrap().paint.clone() };
    paint.color().unwrap().to_hex()
}

#[test]
fn libraries_are_made_renamed_and_deleted() {
    let mut s = session();
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"], json!([]));
    let id = run(&mut s, "library.create", json!({"name": "Brand"}))["id"].as_str().unwrap().to_string();
    let other = run(&mut s, "library.create", json!({"name": "Brand"}))["id"].as_str().unwrap().to_string();
    assert_ne!(id, other, "two libraries of one name are two files");
    run(&mut s, "library.rename", json!({"library": other, "name": "Client"}));
    let list = run(&mut s, "library.list", json!({}));
    let names: Vec<&str> = list["libraries"].as_array().unwrap().iter().map(|l| l["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Brand", "Client"]);
    assert!(list["folder"].is_null(), "no folder: libraries last for the session");
    run(&mut s, "library.delete", json!({"library": "client"}));
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"].as_array().unwrap().len(), 1);
    assert!(s.execute("library.get", &json!({"library": "Client"})).is_err());
}

#[test]
fn a_graphic_goes_into_a_library_and_comes_out_in_another_document() {
    let mut s = session();
    let id = run(&mut s, "shape.ellipse", json!({"x": 10, "y": 10, "width": 40, "height": 30}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#cc3300", "ids": [id]}));
    run(&mut s, "select.set", json!({"ids": [id]}));
    // With no library yet, adding makes "My Library".
    let added = run(&mut s, "library.add", json!({"kind": "graphic"}));
    assert_eq!(added["existing"], false);
    let lib = added["library"].as_str().unwrap().to_string();
    let item = added["id"].as_str().unwrap().to_string();
    let got = run(&mut s, "library.get", json!({"library": lib}));
    assert_eq!(got["name"], "My Library");
    let g = &got["graphics"][0];
    // Its size as it draws: the 1 pt stroke included.
    assert!((g["width"].as_f64().unwrap() - 41.0).abs() < 1e-6 && (g["height"].as_f64().unwrap() - 31.0).abs() < 1e-6, "{g}");
    assert!(!g["thumbnail"].as_str().unwrap().is_empty(), "a thumbnail of the art");
    // The same art again is the item already there.
    assert_eq!(run(&mut s, "library.add", json!({"kind": "graphic"}))["existing"], true);
    // Placed in another document, centred where asked, selected, one undo step.
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let placed = run(&mut s, "library.use", json!({"library": lib, "kind": "graphic", "item": item, "center": [150, 100]}));
    let new_id = placed["ids"][0].clone();
    let st = s.doc().unwrap();
    let b = st.doc.node(NodeId(new_id.as_u64().unwrap())).unwrap().geometric_bounds().unwrap();
    assert!((b.center().x - 150.0).abs() < 1e-6 && (b.center().y - 100.0).abs() < 1e-6, "{b:?}");
    assert_eq!(st.selection.objects, vec![NodeId(new_id.as_u64().unwrap())]);
    assert_eq!(hex_of(&s, &new_id, false), "#cc3300");
    assert_eq!(objects(&s), 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(objects(&s), 0);
    // A graphic is removed by id.
    run(&mut s, "library.removeItem", json!({"library": lib, "kind": "graphic", "item": item}));
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "graphic", "item": item})).is_err());
}

#[test]
fn colours_come_from_the_selection_and_paint_it() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    let a = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#123456", "ids": [a]}));
    run(&mut s, "paint.setStroke", json!({"color": "#abcdef", "ids": [a]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}))["name"], "#123456");
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "strokeColor"}))["name"], "#abcdef");
    assert_eq!(run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}))["existing"], true);
    let b = run(&mut s, "shape.rectangle", json!({"x": 50, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [b]}));
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillColor", "item": "#123456"}));
    run(&mut s, "library.use", json!({"library": lib, "kind": "fillColor", "item": "#123456", "to": "stroke"}));
    assert_eq!(hex_of(&s, &b, false), "#123456");
    assert_eq!(hex_of(&s, &b, true), "#123456");
    assert!(s.execute("library.use", &json!({"library": lib, "kind": "fillColor", "item": "#nope"})).is_err());
}

#[test]
fn text_styles_come_from_the_selected_type_and_apply_in_one_step() {
    let mut s = session();
    let lib = run(&mut s, "library.create", json!({}))["id"].as_str().unwrap().to_string();
    assert!(s.execute("library.add", &json!({"library": lib, "kind": "charStyle"})).is_err(), "no type selected");
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Heading", "size": 31}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let name = run(&mut s, "library.add", json!({"library": lib, "kind": "charStyle"}))["name"].as_str().unwrap().to_string();
    assert!(name.ends_with("31 pt"), "{name}");
    run(&mut s, "library.add", json!({"library": lib, "kind": "paraStyle"}));
    // Another document: the style is added and applied, and one undo takes both back.
    run(&mut s, "file.new", json!({"width": 200, "height": 200}));
    let u = run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Body", "size": 9}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [u]}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "charStyle", "item": name}));
    assert_eq!(r["style"], name);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let size = |s: &Session| match &s.doc().unwrap().doc.node(NodeId(u.as_u64().unwrap())).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.runs[0].style.size,
        _ => 0.0,
    };
    assert_eq!(size(&s), 31.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(size(&s), 9.0);
    assert!(s.doc().unwrap().doc.char_styles.iter().all(|c| c.name != name), "the style went with the undo");
    // A style of that name with other attributes in the document: the library's comes in numbered.
    run(&mut s, "charStyle.new", json!({"name": name, "attrs": {"size": 5}}));
    let r = run(&mut s, "library.use", json!({"library": lib, "kind": "charStyle", "item": name}));
    assert_eq!(r["style"], format!("{name} 2"));
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn libraries_are_kept_in_files_across_sessions() {
    let dir = std::env::temp_dir().join(format!("vc-libraries-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let folder = dir.to_string_lossy().to_string();
    let mut s = session();
    s.libraries.set_dir(Some(folder.clone()));
    let lib = run(&mut s, "library.create", json!({"name": "Kept"}))["id"].as_str().unwrap().to_string();
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "graphic", "name": "Square"}));
    run(&mut s, "library.add", json!({"library": lib, "kind": "fillColor"}));
    let mut t = session();
    t.libraries.set_dir(Some(folder.clone()));
    let got = run(&mut t, "library.get", json!({"library": "Kept"}));
    assert_eq!(got["graphics"][0]["name"], "Square");
    assert_eq!(got["colors"].as_array().unwrap().len(), 1);
    run(&mut t, "library.delete", json!({"library": "Kept"}));
    let mut u = session();
    u.libraries.set_dir(Some(folder));
    assert_eq!(run(&mut u, "library.list", json!({}))["libraries"], json!([]), "deleting removes the file");
    // A damaged file is skipped.
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Bad.vclibrary"), b"{not json").unwrap();
    let mut v = session();
    v.libraries.set_dir(Some(dir.to_string_lossy().to_string()));
    assert_eq!(run(&mut v, "library.list", json!({}))["libraries"], json!([]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A library "Brand" holding a graphic, a colour and a character style, each added from the
/// selection → (session, the graphic's id, the colour's name, the style's name).
fn brand() -> (Session, String, String, String) {
    let mut s = session();
    run(&mut s, "library.create", json!({"name": "Brand"}));
    let r = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut s, "paint.setFill", json!({"color": "#336699", "ids": [r]}));
    run(&mut s, "select.set", json!({"ids": [r]}));
    let g = run(&mut s, "library.add", json!({"kind": "graphic", "name": "Logo"}))["id"].as_str().unwrap().to_string();
    let c = run(&mut s, "library.add", json!({"kind": "fillColor"}))["name"].as_str().unwrap().to_string();
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Type", "size": 18}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [t]}));
    let st = run(&mut s, "library.add", json!({"kind": "charStyle"}))["name"].as_str().unwrap().to_string();
    (s, g, c, st)
}

/// The current library's groups as `library.get` reports them: (name, [(kind, item)]).
fn groups(s: &mut Session) -> Vec<(String, Vec<(String, String)>)> {
    let got = run(s, "library.get", json!({}));
    got["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|g| {
            let items = g["items"].as_array().unwrap().iter().map(|r| (r["kind"].as_str().unwrap().into(), r["item"].as_str().unwrap().into()));
            (g["name"].as_str().unwrap().to_string(), items.collect())
        })
        .collect()
}

fn refs(v: &[(&str, &str)]) -> Vec<(String, String)> {
    v.iter().map(|(k, i)| (k.to_string(), i.to_string())).collect()
}

/// Graphics added one after another get ids of their own (the third used to repeat the second's).
#[test]
fn every_graphic_gets_an_id_of_its_own() {
    let mut s = session();
    run(&mut s, "library.create", json!({}));
    let mut ids = vec![];
    for x in [0, 30, 60, 90] {
        let r = run(&mut s, "shape.rectangle", json!({"x": x, "y": 0, "width": 10 + x, "height": 10}))["id"].clone();
        ids.push(run(&mut s, "library.add", json!({"kind": "graphic", "ids": [r]}))["id"].as_str().unwrap().to_string());
    }
    assert_eq!(ids, ["graphic", "graphic-2", "graphic-3", "graphic-4"]);
}

/// Items go into user-named groups and back out (#926); deleting a group keeps its items.
#[test]
fn items_are_grouped_moved_and_ungrouped() {
    let (mut s, g, c, st) = brand();
    assert!(groups(&mut s).is_empty(), "no groups yet");
    // A group made with items; a graphic is named by its name or its id.
    let r = run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": "Logo"}]}));
    assert_eq!(r["group"], "Logos");
    assert_eq!(groups(&mut s), [("Logos".into(), refs(&[("graphic", &g)]))]);
    // A name taken (ignoring case) gets a number.
    assert_eq!(run(&mut s, "library.createGroup", json!({"name": "logos"}))["group"], "logos 2");
    run(&mut s, "library.renameGroup", json!({"group": "logos 2", "name": "Palette"}));
    assert!(s.execute("library.renameGroup", &json!({"group": "Palette", "name": "LOGOS"})).is_err(), "another group's name");
    assert!(s.execute("library.renameGroup", &json!({"group": "Nope", "name": "X"})).is_err());
    // Moving an item takes it out of the group it was in; a colour is named as fill or stroke.
    run(&mut s, "library.moveItem", json!({"kind": "strokeColor", "item": c, "group": "palette"}));
    run(&mut s, "library.moveItem", json!({"kind": "charStyle", "item": st, "group": "Palette"}));
    let moved = run(&mut s, "library.moveItem", json!({"kind": "graphic", "item": "Logo", "group": "Palette"}));
    assert_eq!((moved["item"].as_str(), moved["group"].as_str()), (Some(g.as_str()), Some("Palette")));
    assert_eq!(groups(&mut s), [("Logos".into(), vec![]), ("Palette".into(), refs(&[("color", &c), ("charStyle", &st), ("graphic", &g)]))]);
    // Without a group, out of its group.
    assert!(run(&mut s, "library.moveItem", json!({"kind": "fillColor", "item": c}))["group"].is_null());
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": c, "group": "Nope"})).is_err());
    assert!(s.execute("library.moveItem", &json!({"kind": "fillColor", "item": "#nope", "group": "Palette"})).is_err());
    assert!(s.execute("library.createGroup", &json!({"items": [{"kind": "graphic", "item": "nope"}]})).is_err(), "no such item");
    assert_eq!(groups(&mut s).len(), 2, "a failed command makes no group");
    // A deleted item leaves its group.
    run(&mut s, "library.removeItem", json!({"kind": "charStyle", "item": st}));
    assert_eq!(groups(&mut s)[1].1, refs(&[("graphic", &g)]));
    // A deleted group's items stay, ungrouped.
    assert_eq!(run(&mut s, "library.deleteGroup", json!({"group": "Palette"}))["ungrouped"], 1);
    assert_eq!(run(&mut s, "library.get", json!({}))["graphics"].as_array().unwrap().len(), 1);
    assert_eq!(groups(&mut s), [("Logos".into(), vec![])]);
    assert_eq!(run(&mut s, "library.list", json!({}))["libraries"][0]["groups"], 1);
}

/// A library goes out to a file with its art and groups and comes back as a new library, numbered
/// when its name is taken; the graphic it brings places in a document (#926).
#[test]
fn a_library_is_exported_and_imported() {
    let (mut s, g, c, _) = brand();
    run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": g}, {"kind": "fillColor", "item": c}]}));
    let out = run(&mut s, "library.export", json!({}));
    assert_eq!(out["name"], "Brand");
    let data = out["data"].as_str().unwrap().to_string();
    let r = run(&mut s, "library.import", json!({"data": data}));
    let counts = ["graphics", "colors", "charStyles", "paraStyles", "groups"].map(|k| r[k].as_u64().unwrap());
    assert_eq!((r["name"].as_str(), counts), (Some("Brand (2)"), [1, 1, 1, 0, 1]));
    let id = r["id"].as_str().unwrap().to_string();
    assert_eq!(run(&mut s, "library.list", json!({}))["current"], id, "the imported library is the current one");
    let a = run(&mut s, "library.get", json!({"library": "Brand"}));
    let b = run(&mut s, "library.get", json!({"library": id}));
    for k in ["graphics", "colors", "charStyles", "paraStyles", "groups"] {
        assert_eq!(a[k], b[k], "{k}");
    }
    // Again: the next number; given a name, that name.
    let b64 = vectorcraft_format::base64_encode(data.as_bytes());
    assert_eq!(run(&mut s, "library.import", json!({"dataBase64": b64}))["name"], "Brand (3)");
    assert_eq!(run(&mut s, "library.import", json!({"data": data, "name": "Client"}))["name"], "Client");
    // Its graphic places in another document.
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let placed = run(&mut s, "library.use", json!({"library": id, "kind": "graphic", "item": "Logo", "center": [50, 50]}));
    assert_eq!(placed["ids"].as_array().unwrap().len(), 1);
    // Not a library file.
    let swatches = json!({"format": "vcswatches", "name": "S"}).to_string();
    for bad in [json!({"data": "{not json"}), json!({"data": "[1, 2]"}), json!({"data": swatches}), json!({"dataBase64": "%%"}), json!({})] {
        assert!(s.execute("library.import", &bad).is_err(), "{bad}");
    }
}

/// A file written before groups loads; one with repeated names, groups naming items that aren't
/// there or kinds this version doesn't know, and a thumbnail claiming a huge image is made safe.
#[test]
fn imported_libraries_are_made_consistent() {
    let mut s = session();
    let old = json!({"name": "Old", "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}]});
    let r = run(&mut s, "library.import", json!({"data": old.to_string()}));
    assert_eq!((r["colors"].as_u64(), r["groups"].as_u64()), (Some(1), Some(0)));
    // A PNG header claiming 100000 × 100000 pixels.
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend(100_000u32.to_be_bytes());
    png.extend(100_000u32.to_be_bytes());
    png.extend([8, 6, 0, 0, 0]);
    let messy = json!({
        "name": "  ",
        "colors": [{"name": "Red", "color": {"model": "rgb", "r": 1, "g": 0, "b": 0}}, {"name": "Red", "color": {"model": "rgb", "r": 0, "g": 1, "b": 0}}, {"name": "", "color": {"model": "rgb", "r": 0, "g": 0, "b": 1}}],
        "graphics": [
            {"id": "g", "name": "A", "width": 1, "height": 1, "data": "", "thumbnail": vectorcraft_format::base64_encode(&png)},
            {"id": "g", "name": "B", "width": -5, "height": 1, "data": ""}
        ],
        "groups": [
            {"name": "One", "items": [{"kind": "color", "item": "Red 2"}, {"kind": "graphic", "item": "nope"}, {"kind": "brush", "item": "Red"}]},
            {"name": "one", "items": [{"kind": "color", "item": "Red 2"}, {"kind": "graphic", "item": "g-2"}]}
        ]
    });
    let r = run(&mut s, "library.import", json!({"data": messy.to_string()}));
    assert_eq!(r["name"], "Library", "a blank name");
    let got = run(&mut s, "library.get", json!({}));
    let names: Vec<&str> = got["colors"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["Red", "Red 2", "Color"]);
    let graphics = got["graphics"].as_array().unwrap();
    assert_eq!((graphics[0]["id"].as_str(), graphics[1]["id"].as_str()), (Some("g"), Some("g-2")));
    assert_eq!((graphics[0]["thumbnail"].as_str(), graphics[1]["width"].as_f64()), (Some(""), Some(0.0)));
    assert_eq!(
        got["groups"],
        json!([{"name": "One", "items": [{"kind": "color", "item": "Red 2"}]}, {"name": "one 2", "items": [{"kind": "graphic", "item": "g-2"}]}])
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_library_is_exported_to_a_file_and_imported_from_it() {
    let dir = vectorcraft_testkit::temp_dir("library-export");
    let path = dir.join("Brand.vclibrary").to_string_lossy().to_string();
    let (mut s, g, ..) = brand();
    run(&mut s, "library.createGroup", json!({"name": "Logos", "items": [{"kind": "graphic", "item": g}]}));
    assert_eq!(run(&mut s, "library.export", json!({"library": "Brand", "path": path}))["path"], path.as_str());
    let mut t = session();
    let r = run(&mut t, "library.import", json!({"path": path}));
    assert_eq!((r["name"].as_str(), r["graphics"].as_u64(), r["groups"].as_u64()), (Some("Brand"), Some(1), Some(1)));
    assert!(t.execute("library.import", &json!({"path": dir.join("missing.vclibrary").to_string_lossy()})).is_err());
    // Groups are kept in the library folder's files too.
    let folder = dir.join("folder").to_string_lossy().to_string();
    let mut u = session();
    u.libraries.set_dir(Some(folder.clone()));
    run(&mut u, "library.import", json!({"path": path}));
    let mut v = session();
    v.libraries.set_dir(Some(folder));
    assert_eq!(groups(&mut v), [("Logos".into(), refs(&[("graphic", &g)]))]);
}
