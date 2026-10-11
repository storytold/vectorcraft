//! Swatch libraries: listing, reading, adding to the document (deduped, one undo step, applied)
//! and Default Swatches.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn list_and_get_every_builtin_library() {
    let mut s = Session::new();
    let libs = run(&mut s, "swatch.library.list", json!({}));
    let libs = libs["libraries"].as_array().unwrap();
    let ids: Vec<&str> = libs.iter().map(|l| l["id"].as_str().unwrap()).collect();
    for id in [
        "web-safe-216",
        "grays-neutrals",
        "earth-tones",
        "skin-tone-ramps",
        "pastels",
        "brights",
        "metallic-gradients",
        "perceptual-scales",
        "harmony-sets",
    ] {
        assert!(ids.contains(&id), "{id} missing from {ids:?}");
    }
    let web = libs.iter().find(|l| l["id"] == "web-safe-216").unwrap();
    assert_eq!((web["name"].as_str(), web["count"].as_u64()), (Some("Web Safe 216"), Some(216)));
    // By id or by name in any case; groups list their swatches.
    let g = run(&mut s, "swatch.library.get", json!({"library": "earth tones"}));
    assert_eq!(g["id"], "earth-tones");
    let clay = g["groups"].as_array().unwrap().iter().find(|x| x["name"] == "Clay").unwrap();
    assert_eq!(clay["swatches"].as_array().unwrap().len(), 6);
    assert!(g["swatches"].as_array().unwrap().iter().all(|w| w["kind"] == "color" && w["group"].is_string()));
    let m = run(&mut s, "swatch.library.get", json!({"library": "metallic-gradients"}));
    assert!(m["swatches"].as_array().unwrap().iter().all(|w| w["kind"] == "gradient"));
    assert!(s.execute("swatch.library.get", &json!({"library": "nope"})).is_err());
}

#[test]
fn add_dedupes_as_one_undo_step() {
    let mut s = session();
    let before = undo_len(&s);
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Clay", "Ochre 2", "Clay 3"]}));
    // The Clay group comes whole (its third swatch once); Ochre 2 comes ungrouped.
    let added: Vec<&str> = r["added"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
    assert_eq!(added.len(), 7, "{added:?}");
    assert_eq!(undo_len(&s), before + 1, "one undo step");
    let d = doc(&s);
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Clay").map(|g| g.swatches.len()), Some(6));
    assert!(d.swatches.iter().any(|w| w.name == "Ochre 2"));
    // Adding again finds them all and changes nothing (no undo step).
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Clay", "Ochre 2"]}));
    assert_eq!((r["added"].as_array().unwrap().len(), r["existing"].as_array().unwrap().len()), (0, 7));
    assert_eq!(undo_len(&s), before + 1);
    // A name taken by a different colour gets a number.
    run(&mut s, "swatch.new", json!({"name": "Ochre 3", "color": "#123456"}));
    let r = run(&mut s, "swatch.library.add", json!({"library": "earth-tones", "names": ["Ochre 3"]}));
    assert_eq!(r["added"], json!(["Ochre 3 2"]));
    // Undo removes the whole first add.
    for _ in 0..3 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert!(doc(&s).swatch_groups.iter().all(|g| g.name != "Clay") && doc(&s).swatch("Ochre 2").is_none());
    assert!(s.execute("swatch.library.add", &json!({"library": "earth-tones", "names": ["Nope"]})).is_err());
}

#[test]
fn add_with_apply_paints_the_selection_in_the_same_undo_step() {
    let mut s = session();
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
    let before = undo_len(&s);
    let r = run(&mut s, "swatch.library.add", json!({"library": "web-safe-216", "names": ["#FF6600"], "apply": "fill"}));
    assert_eq!((r["added"].clone(), r["applied"].clone()), (json!(["#FF6600"]), json!("#FF6600")));
    let fill = doc(&s).node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#ff6600".into()));
    assert_eq!(undo_len(&s), before + 1, "added and applied as one step");
    run(&mut s, "edit.undo", json!({}));
    assert!(doc(&s).swatch("#FF6600").is_none());
    // A gradient swatch applied to the stroke; an existing swatch is applied without an add.
    run(&mut s, "swatch.library.add", json!({"library": "metallic-gradients", "names": ["Gold"], "apply": "stroke"}));
    let n = doc(&s).node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap();
    assert!(matches!(n.appearance.stroke_paint(), Paint::Gradient(_)));
    let r = run(&mut s, "swatch.library.add", json!({"library": "metallic-gradients", "names": ["Gold"], "apply": "fill"}));
    assert_eq!((r["added"].clone(), r["applied"].clone()), (json!([]), json!("Gold")));
}

#[test]
fn reset_defaults_restores_missing_swatches_or_replaces_them() {
    let mut s = session();
    let defaults: Vec<String> = doc(&s).swatches_iter().map(|w| w.name.clone()).collect();
    run(&mut s, "swatch.delete", json!({"names": ["Red", "Grays"]}));
    run(&mut s, "swatch.new", json!({"name": "Mine", "color": "#abcdef"}));
    let r = run(&mut s, "swatch.resetDefaults", json!({}));
    assert!(r["added"].as_array().unwrap().iter().any(|n| n == "Red"));
    let d = doc(&s);
    assert!(d.swatch("Mine").is_some(), "the user's swatches stay");
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Grays").map(|g| g.swatches.len()), Some(9));
    assert!(defaults.iter().all(|n| d.swatch(n).is_some()));
    // Replace: exactly the defaults; art linked to a removed global swatch keeps its colour.
    run(&mut s, "swatch.edit", json!({"name": "Mine", "global": true}));
    let id = run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5}))["id"].clone();
    run(&mut s, "select.set", json!({"ids": [id]}));
    run(&mut s, "paint.setFill", json!({"swatch": "Mine"}));
    assert_eq!(s.paint.fill, Paint::Solid { color: Color::from_hex("#abcdef").unwrap(), swatch: Some("Mine".into()), tint: 1.0 });
    run(&mut s, "swatch.resetDefaults", json!({"replace": true}));
    let d = doc(&s);
    assert_eq!(d.swatches_iter().map(|w| w.name.clone()).collect::<Vec<_>>(), defaults);
    let fill = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::solid(Color::from_hex("#abcdef").unwrap()));
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#abcdef").unwrap()), "the default fill is unlinked too");
}

/// A fresh scratch folder for one test.
fn temp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("vc-swatchlib-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

#[test]
fn save_and_load_round_trip_keeps_cmyk_spot_and_groups() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": {"c": 1.0, "m": 0.5, "y": 0.0, "k": 0.2}, "spot": true}));
    run(&mut s, "swatch.newGroup", json!({"name": "Brand", "colors": ["#123456", "#abcdef"]}));
    let r = run(&mut s, "swatch.library.save", json!({"names": ["Ink", "Brand", "Sunset"], "name": "Brand Kit"}));
    assert_eq!((r["format"].as_str(), r["count"].as_u64()), (Some("vcswatches"), Some(4)));
    let data = r["data"].as_str().unwrap().to_string();
    let r = run(&mut s, "swatch.library.load", json!({"data": data, "name": "kit.vcswatches"}));
    assert_eq!((r["name"].as_str(), r["count"].as_u64()), (Some("Brand Kit"), Some(4)));
    let id = r["library"].as_str().unwrap().to_string();
    let (_, lib) = cmd::swatchlib::library(&s, &id).unwrap();
    let ink = lib.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global);
    assert_eq!(ink.paint.color(), Some(Color::cmyk(1.0, 0.5, 0.0, 0.2)));
    assert_eq!(lib.groups[0].name, "Brand");
    assert_eq!(lib.groups[0].swatches.len(), 2);
    assert!(matches!(lib.swatch("Sunset").unwrap().paint, Paint::Gradient(_)));
    // Listed as loaded; adding from it works like any library.
    let list = run(&mut s, "swatch.library.list", json!({}));
    assert!(list["libraries"].as_array().unwrap().iter().any(|l| l["id"] == id.as_str() && l["category"] == "loaded"));
    assert!(s.execute("swatch.library.save", &json!({"names": ["Nope"]})).is_err());
    assert!(s.execute("swatch.library.save", &json!({"user": true})).is_err(), "no user folder in a headless session");
}

#[test]
fn libraries_saved_in_the_user_folder_are_user_defined() {
    let dir = temp_dir("user");
    let mut s = session();
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "swatch.library.save", json!({"user": true, "name": "My: Greys", "format": "gpl", "names": ["Grays"]}));
    assert_eq!(r["library"], "user/My- Greys.gpl");
    let path = r["path"].as_str().unwrap().to_string();
    assert!(std::fs::read_to_string(&path).unwrap().starts_with("GIMP Palette\nName: My: Greys\n"));
    let list = run(&mut s, "swatch.library.list", json!({}));
    let user: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "user").collect();
    assert_eq!(user.len(), 1);
    assert_eq!((user[0]["name"].as_str(), user[0]["count"].as_u64()), (Some("My: Greys"), Some(9)));
    // Opening a file of the user folder gives its User Defined library.
    let r = run(&mut s, "swatch.library.load", json!({ "path": path }));
    assert_eq!(r["library"], "user/My- Greys.gpl");
    // CSS by extension.
    let css = dir.join("out.css").to_string_lossy().to_string();
    run(&mut s, "swatch.library.save", json!({ "path": css }));
    assert!(std::fs::read_to_string(&css).unwrap().contains("  --white: #ffffff;"));
    // A name with nothing a file name can keep is saved as Library.vcswatches, and User Defined lists it.
    let r = run(&mut s, "swatch.library.save", json!({"user": true, "name": "..."}));
    assert_eq!(r["library"], "user/Library.vcswatches");
    assert_eq!(cmd::swatchlib::library(&s, "user/Library.vcswatches").map(|(info, _)| info.name), Some("...".into()));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn another_documents_swatches_load_as_a_library() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Signal", "color": "#ff3300"}));
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": b64, "name": "Poster.vectorcraft"}));
    assert_eq!(r["name"], "Poster");
    let (_, lib) = cmd::swatchlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert!(lib.swatch("Signal").is_some() && lib.swatch("[None]").is_none());
    assert!(s.execute("swatch.library.load", &json!({"data": "hello", "name": "x.txt"})).is_err());
}

#[test]
fn swatch_exchange_files_load_add_and_list_as_user_libraries() {
    let bytes = vectorcraft_testkit::ase::sample();
    let mut s = session();
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Brand.ase"}));
    assert_eq!((r["library"].as_str(), r["name"].as_str(), r["count"].as_u64()), (Some("loaded/Brand.ase"), Some("Brand"), Some(4)));
    let g = run(&mut s, "swatch.library.get", json!({"library": r["library"]}));
    let swatches = g["swatches"].as_array().unwrap();
    let kinds: Vec<(&str, bool, bool)> =
        swatches.iter().map(|w| (w["name"].as_str().unwrap(), w["global"].as_bool().unwrap(), w["spot"].as_bool().unwrap())).collect();
    assert_eq!(kinds, [("Sky", true, false), ("Ink", true, true), ("Mist", false, false), ("Clay", false, false)]);
    assert_eq!(swatches[3]["color"], json!({"model": "lab", "l": 50.0, "a": 20.0, "b": -30.0}));
    // Added to the document, the spot color stays a spot color and the group stays a color group.
    run(&mut s, "swatch.library.add", json!({"library": r["library"]}));
    let d = doc(&s);
    assert!(d.swatch("Ink").is_some_and(|w| w.spot && w.global));
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "Neutrals").map(|g| g.swatches.len()), Some(2));
    // A damaged file is an error from the swatch exchange reader, not from the document loader.
    let cut = vectorcraft_format::base64_encode(&bytes[..bytes.len() - 1]);
    let e = s.execute("swatch.library.load", &json!({"dataBase64": cut, "name": "Cut.ase"})).unwrap_err().to_string();
    assert!(e.contains("cut short"), "{e}");
    // `.ase` files in the user library folder are User Defined libraries.
    let dir = temp_dir("ase");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Brand.ase"), &bytes).unwrap();
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let list = run(&mut s, "swatch.library.list", json!({}));
    let user: Vec<(&str, u64)> = list["libraries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["category"] == "user")
        .map(|l| (l["id"].as_str().unwrap(), l["count"].as_u64().unwrap()))
        .collect();
    assert_eq!(user, [("user/Brand.ase", 4)]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn swatch_exchange_files_save_with_models_kinds_and_groups() {
    let mut s = session();
    run(&mut s, "swatch.new", json!({"name": "Ink", "color": {"c": 1.0, "m": 0.5, "y": 0.0, "k": 0.2}, "spot": true}));
    run(&mut s, "swatch.new", json!({"name": "Clay", "color": {"l": 50, "a": 20, "b": -30}}));
    run(&mut s, "swatch.newGroup", json!({"name": "Brand", "colors": ["#123456", "#abcdef"]}));
    let r = run(&mut s, "swatch.library.save", json!({"format": "ase", "names": ["Ink", "Clay", "Brand", "Grays", "Sunset"], "name": "Brand Kit"}));
    assert_eq!(r["format"], "ase");
    assert!(r.get("data").is_none(), "a binary file comes back as dataBase64");
    let b64 = r["dataBase64"].as_str().unwrap().to_string();
    assert!(vectorcraft_format::base64_decode(&b64).unwrap().starts_with(b"ASEF"));
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": b64, "name": "Brand Kit.ase"}));
    let (_, lib) = cmd::swatchlib::library(&s, r["library"].as_str().unwrap()).unwrap();
    assert_eq!(lib.name, "Brand Kit", "named after the file");
    let ink = lib.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global);
    assert_eq!(ink.paint.color(), Some(Color::cmyk(1.0, 0.5, 0.0, 0.2)));
    assert_eq!(lib.swatch("Clay").unwrap().paint.color(), Some(Color::lab(50.0, 20.0, -30.0)));
    assert!(lib.swatch("Sunset").is_none(), "a gradient is left out");
    let groups: Vec<(&str, usize)> = lib.groups.iter().map(|g| (g.name.as_str(), g.swatches.len())).collect();
    assert_eq!(groups, [("Grays", 9), ("Brand", 2)], "in document order");
    // Gray colors come back within float rounding.
    let grays = &doc(&s).swatch_groups.iter().find(|g| g.name == "Grays").unwrap().swatches;
    for (w, want) in lib.groups[0].swatches.iter().zip(grays) {
        let (Some(Color::Gray { k }), Some(Color::Gray { k: want_k })) = (w.paint.color(), want.paint.color()) else {
            panic!("{} isn't gray", w.name)
        };
        assert!(w.name == want.name && (k - want_k).abs() < 1e-4, "{}: {k} for {want_k}", w.name);
    }
    // Into the user library folder, and to a path ending in .ase.
    let dir = temp_dir("ase-save");
    s.swatch_libraries.set_user_dir(Some(dir.to_string_lossy().to_string()));
    let r = run(&mut s, "swatch.library.save", json!({"format": "ase", "user": true, "name": "Brand Kit"}));
    assert_eq!(r["library"], "user/Brand Kit.ase");
    assert!(std::fs::read(r["path"].as_str().unwrap()).unwrap().starts_with(b"ASEF"));
    assert!(cmd::swatchlib::library(&s, "user/Brand Kit.ase").is_some(), "the user folder lists it");
    let path = dir.join("out.ase").to_string_lossy().to_string();
    assert_eq!(run(&mut s, "swatch.library.save", json!({ "path": path }))["format"], "ase");
    assert!(std::fs::read(&path).unwrap().starts_with(b"ASEF"));
    let e = s.execute("swatch.library.save", &json!({"format": "aco"})).unwrap_err().to_string();
    assert!(e.contains("vcswatches, gpl, ase or css"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn save_counts_the_swatches_the_file_holds() {
    let mut s = session();
    // A new document has 46 solid colors and 4 linear or radial gradients besides None. Mesh is a freeform gradient.
    run(&mut s, "swatch.new", json!({"name": "Mesh", "gradient": {"kind": "freeform"}}));
    let mut counts = vec![];
    for format in ["vcswatches", "gpl", "ase", "css"] {
        let r = run(&mut s, "swatch.library.save", json!({ "format": format }));
        // The libraries load back with that many swatches, and the CSS file has one property for each.
        let held = if format == "css" {
            r["data"].as_str().unwrap().lines().filter(|l| l.starts_with("  --")).count() as u64
        } else {
            let load = json!({"data": r["data"], "dataBase64": r["dataBase64"], "name": format!("Saved.{format}")});
            run(&mut s, "swatch.library.load", load)["count"].as_u64().unwrap()
        };
        counts.push((format, r["count"].as_u64().unwrap(), held));
    }
    assert_eq!(counts, [("vcswatches", 51, 51), ("gpl", 46, 46), ("ase", 46, 46), ("css", 50, 50)]);
}

#[test]
fn loaded_libraries_copy_into_the_user_folder_without_replacing_files() {
    use vectorcraft_testkit::ase::{Block, ase, sample};
    let dir = temp_dir("copy");
    let (user, elsewhere) = (dir.join("user"), dir.join("elsewhere"));
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let source = elsewhere.join("Brand.ase");
    std::fs::write(&source, sample()).unwrap();
    let source = source.to_string_lossy().to_string();
    // Another library whose file name differs only in case.
    let other = ase(&[Block::Color("Only", b"RGB ", &[1.0, 0.0, 0.0], 2)]);
    std::fs::write(user.join("brand.ase"), &other).unwrap();
    let mut s = session();
    s.swatch_libraries.set_user_dir(Some(user.to_string_lossy().to_string()));
    let loaded = run(&mut s, "swatch.library.load", json!({ "path": source }))["library"].clone();
    assert_eq!(loaded, format!("loaded/{source}"));
    let r = run(&mut s, "swatch.library.copyToUser", json!({ "library": loaded }));
    // A swatch exchange library is named after its file.
    assert_eq!(
        (r["library"].as_str(), r["name"].as_str(), r["count"].as_u64(), r["copied"].as_bool()),
        (Some("user/Brand 2.ase"), Some("Brand 2"), Some(4), Some(true))
    );
    assert_eq!(r["path"], user.join("Brand 2.ase").to_string_lossy().to_string());
    assert_eq!(std::fs::read(user.join("Brand 2.ase")).unwrap(), sample(), "the file as it is");
    assert_eq!(std::fs::read(user.join("brand.ase")).unwrap(), other, "never replaced");
    let listed = |s: &mut Session| -> Vec<String> {
        let list = run(s, "swatch.library.list", json!({}));
        let libs = list["libraries"].as_array().unwrap().iter().filter(|l| matches!(l["category"].as_str(), Some("user" | "loaded")));
        libs.map(|l| l["id"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(listed(&mut s), ["user/Brand 2.ase", "user/brand.ase"], "listed as User Defined only");
    // The loaded id finds the copy, so choices made with it (Limit to Library) keep their colors.
    let found = cmd::swatchlib::library(&s, loaded.as_str().unwrap()).map(|(info, _)| info.id);
    assert_eq!(found.as_deref(), Some("user/Brand 2.ase"));
    assert!(cmd::swatchlib::limit_palette(&s, loaded.as_str().unwrap()).is_some());
    // The same file again opens the copy, which is listed once; copying it writes nothing.
    let again = run(&mut s, "swatch.library.load", json!({ "path": source }))["library"].clone();
    assert_eq!(again, "user/Brand 2.ase");
    assert_eq!(listed(&mut s), ["user/Brand 2.ase", "user/brand.ase"]);
    let r = run(&mut s, "swatch.library.copyToUser", json!({ "library": again }));
    assert_eq!((r["library"].as_str(), r["copied"].as_bool()), (Some("user/Brand 2.ase"), Some(false)));
    assert_eq!(std::fs::read_dir(&user).unwrap().count(), 2);
    // A document's swatches and a library loaded from data are written as .vcswatches.
    run(&mut s, "swatch.new", json!({"name": "Signal", "color": "#ff3300"}));
    let b64 = run(&mut s, "document.serialize", json!({}))["dataBase64"].as_str().unwrap().to_string();
    let poster = elsewhere.join("Poster.vectorcraft");
    std::fs::write(&poster, vectorcraft_format::base64_decode(&b64).unwrap()).unwrap();
    let loaded = run(&mut s, "swatch.library.load", json!({ "path": poster.to_string_lossy() }))["library"].clone();
    let r = run(&mut s, "swatch.library.copyToUser", json!({ "library": loaded }));
    assert_eq!((r["library"].as_str(), r["copied"].as_bool()), (Some("user/Poster.vcswatches"), Some(true)));
    assert!(cmd::swatchlib::library(&s, "user/Poster.vcswatches").unwrap().1.swatch("Signal").is_some());
    // Copied again, the document's swatches are written as the same bytes: the copy there is used.
    let loaded = run(&mut s, "swatch.library.load", json!({ "path": poster.to_string_lossy() }))["library"].clone();
    let r = run(&mut s, "swatch.library.copyToUser", json!({ "library": loaded }));
    assert_eq!((r["library"].as_str(), r["copied"].as_bool()), (Some("user/Poster.vcswatches"), Some(false)));
    let kit = vectorcraft_format::base64_encode(&sample());
    let loaded = run(&mut s, "swatch.library.load", json!({"dataBase64": kit, "name": "Kit.ase"}))["library"].clone();
    assert_eq!(run(&mut s, "swatch.library.copyToUser", json!({ "library": loaded }))["library"], "user/Kit.vcswatches");
    // A library without a name is copied as Library.
    let loaded = run(&mut s, "swatch.library.load", json!({"data": "GIMP Palette\nName:\n0 0 0 Black\n"}))["library"].clone();
    let r = run(&mut s, "swatch.library.copyToUser", json!({ "library": loaded }));
    assert_eq!((r["library"].as_str(), r["name"].as_str(), r["copied"].as_bool()), (Some("user/Library.vcswatches"), Some("Library"), Some(true)));
    // When the source no longer reads as a library, the copy fails and no new file is left in the folder.
    let junk = elsewhere.join("Junk.ase");
    std::fs::write(&junk, ase(&[Block::Color("Junk", b"RGB ", &[0.0, 1.0, 0.0], 2)])).unwrap();
    let loaded = run(&mut s, "swatch.library.load", json!({ "path": junk.to_string_lossy() }))["library"].clone();
    std::fs::write(&junk, b"not a library").unwrap();
    let files = std::fs::read_dir(&user).unwrap().count();
    let e = s.execute("swatch.library.copyToUser", &json!({ "library": loaded })).unwrap_err().to_string();
    assert!(e.contains("the copy `user/Junk.ase` can't be read as a library"), "{e}");
    assert_eq!(std::fs::read_dir(&user).unwrap().count(), files);
    // A User Defined library comes back as it is; a built-in one is an error.
    let r = run(&mut s, "swatch.library.copyToUser", json!({"library": "user/brand.ase"}));
    assert_eq!((r["library"].as_str(), r["copied"].as_bool()), (Some("user/brand.ase"), Some(false)));
    assert!(s.execute("swatch.library.copyToUser", &json!({"library": "pastels"})).unwrap_err().to_string().contains("built in"));
    assert!(s.execute("swatch.library.copyToUser", &json!({})).is_err());
    // Without a user library folder (the web, headless sessions) there is nowhere to copy to, and
    // the command is off.
    let mut headless = session();
    let info = find_command("swatch.library.copyToUser").unwrap().info(&headless);
    assert!(!info.enabled && info.disabled_reason.as_deref() == Some("no user library folder here"), "{info:?}");
    assert!(find_command("swatch.library.copyToUser").unwrap().info(&s).enabled);
    let loaded = run(&mut headless, "swatch.library.load", json!({"dataBase64": kit, "name": "Kit.ase"}))["library"].clone();
    let e = headless.execute("swatch.library.copyToUser", &json!({ "library": loaded })).unwrap_err().to_string();
    assert!(e.contains("no user library folder"), "{e}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_library_group_stays_one_group_when_a_swatch_has_its_name() {
    use vectorcraft_testkit::ase::{Block, ase};
    let mut s = session();
    assert!(doc(&s).swatch("Blue").is_some(), "a new document has a swatch named Blue");
    let bytes = ase(&[
        Block::GroupStart("Blue"),
        Block::Color("Deep", b"RGB ", &[0.0, 0.0, 0.4], 2),
        Block::Color("Pale", b"RGB ", &[0.6, 0.7, 1.0], 2),
        Block::GroupEnd,
        Block::GroupStart("Blue 2"),
        Block::Color("Frost", b"RGB ", &[0.9, 0.95, 1.0], 2),
        Block::GroupEnd,
    ]);
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "Blues.ase"}));
    run(&mut s, "swatch.library.add", json!({"library": r["library"]}));
    let groups: Vec<(&str, Vec<&str>)> =
        doc(&s).swatch_groups.iter().map(|g| (g.name.as_str(), g.swatches.iter().map(|w| w.name.as_str()).collect())).collect();
    assert!(groups.contains(&("Blue 2", vec!["Deep", "Pale"])), "{groups:?}");
    assert!(groups.contains(&("Blue 2 2", vec!["Frost"])), "the library's own Blue 2 stays apart: {groups:?}");
}

#[test]
fn text_libraries_sent_as_bytes_load_when_they_are_not_valid_utf8() {
    let mut s = session();
    let latin1 = vectorcraft_format::base64_encode(b"GIMP Palette\n237 28 36 Rouge fonc\xe9\n");
    let r = run(&mut s, "swatch.library.load", json!({"dataBase64": latin1, "name": "Rouge.gpl"}));
    assert_eq!(r["count"], 1);
}

#[test]
fn gradient_libraries_are_listed_and_gradient_swatches_rename_and_take_new_gradients() {
    let mut s = session();
    let list = run(&mut s, "swatch.library.list", json!({}));
    let grads: Vec<&Value> = list["libraries"].as_array().unwrap().iter().filter(|l| l["category"] == "gradients").collect();
    assert!(grads.len() >= 5 && grads.iter().any(|l| l["id"] == "sky-gradients"));
    run(&mut s, "swatch.library.add", json!({"library": "sky-gradients", "names": ["Dawn"]}));
    // Rename keeps the gradient.
    let before = doc(&s).swatch("Dawn").unwrap().paint.clone();
    run(&mut s, "swatch.edit", json!({"name": "Dawn", "newName": "Early"}));
    assert_eq!(doc(&s).swatch("Early").unwrap().paint, before);
    // An edited gradient replaces it, unplaced and unlinked; one undo step.
    let undo = undo_len(&s);
    let g =
        json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}], "swatch": "Early"}});
    run(&mut s, "swatch.edit", json!({"name": "Early", "paint": g}));
    let Paint::Gradient(gp) = &doc(&s).swatch("Early").unwrap().paint else { panic!("a gradient") };
    assert_eq!((gp.gradient.kind, gp.gradient.stops.len(), gp.swatch.clone(), gp.geom), (vectorcraft_color::GradientKind::Radial, 2, None, None));
    assert_eq!(undo_len(&s), undo + 1);
    // Kinds don't mix; a colour replaces a colour.
    assert!(s.execute("swatch.edit", &json!({"name": "Early", "paint": {"color": "#00ff00"}})).is_err());
    run(&mut s, "swatch.edit", json!({"name": "Red", "paint": {"color": "#00ff00"}}));
    assert_eq!(doc(&s).swatch("Red").unwrap().paint.color().map(|c| c.to_hex()), Some("#00ff00".into()));
}
