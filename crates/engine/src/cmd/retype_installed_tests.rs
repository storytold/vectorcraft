use std::collections::BTreeSet;
use std::path::PathBuf;

use kurbo::{Affine, BezPath, Shape};
use serde_json::json;
use vectorcraft_text::FontDb;
use vectorcraft_text::retype::shape_key;

use super::{Limits, search, search_within};
use crate::Session;

/// A database whose installed fonts are copies of the bundled Source Sans 3 Regular named
/// `families`, one file each, in a folder of their own: they are installed and not loaded.
/// Returns the database and the files.
fn installed(tag: &str, families: &[&str]) -> (FontDb, Vec<PathBuf>) {
    let dir = vectorcraft_testkit::temp_dir(&format!("retype-installed-{tag}"));
    let mut files = vec![];
    for family in families {
        let postscript = format!("{}-Regular", family.replace(' ', ""));
        let data = vectorcraft_text::test_fonts::named_copy(false, (family, "Regular", "Version 1.000", &postscript)).unwrap();
        let path = dir.join(format!("{postscript}.ttf"));
        std::fs::write(&path, data).unwrap();
        files.push(path);
    }
    (FontDb::with_font_dirs(vec![dir]), files)
}

/// The glyphs of `text` in the bundled Source Sans 3 Regular, as outlines at 30 pt.
fn glyphs(db: &FontDb, text: &str) -> Vec<BezPath> {
    let face = db.face("Source Sans 3", "Regular").unwrap();
    text.chars()
        .enumerate()
        .map(|(i, c)| Affine::translate((40.0 + 30.0 * i as f64, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&face, face.glyph_for(c)))
        .collect()
}

fn keys(shapes: &[BezPath]) -> BTreeSet<u64> {
    shapes.iter().filter_map(shape_key).collect()
}

fn unloaded(db: &FontDb, family: &str) -> bool {
    db.unloaded_installed_faces().iter().any(|f| f.family == family)
}

#[test]
fn an_installed_face_whose_glyphs_match_is_found_and_loaded() {
    let (db, files) = installed("found", &["Retype Copy A"]);
    assert!(unloaded(&db, "Retype Copy A"));
    let shapes = glyphs(&db, "Rx");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let found = search(&db, &refs, false).unwrap();
    assert_eq!((found.read, found.skipped), (1, 0));
    let face = found.faces.first().expect("the installed copy matches");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Retype Copy A", "Regular"));
    assert_eq!(face.path(), Some(files[0].as_path()));
    // Loaded now, as it was read.
    assert!(db.loaded_faces().iter().any(|f| f.id() == face.id()));
    assert!(!unloaded(&db, "Retype Copy A"));
    assert_eq!(found.json(), json!({"read": 1, "skipped": 0}));
    // A later Retype compares it ahead of the other loaded faces, however many are loaded.
    assert_eq!(super::found(&db).first().map(|f| f.id()), Some(face.id()));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let candidates = super::super::candidate_faces(&db, &s.doc().unwrap().doc, &[]);
    assert_eq!(candidates.first().map(|f| f.id()), Some(face.id()));
    // Only this database's faces.
    assert!(super::found(&FontDb::with_font_dirs(vec![])).is_empty());
}

#[test]
fn a_matching_face_loads_without_the_family_s_other_files() {
    let dir = vectorcraft_testkit::temp_dir("retype-installed-family");
    for style in ["Regular", "Bold"] {
        let data =
            vectorcraft_text::test_fonts::named_copy(false, ("Retype Copy N", style, "Version 1.000", &format!("RetypeCopyN-{style}"))).unwrap();
        std::fs::write(dir.join(format!("RetypeCopyN-{style}.ttf")), data).unwrap();
    }
    let db = FontDb::with_font_dirs(vec![dir]);
    let shapes = glyphs(&db, "Rx");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let found = search(&db, &refs, false).unwrap();
    assert_eq!((found.read, found.faces.len()), (1, 1));
    let style = found.faces[0].style.clone();
    let loaded: Vec<String> = db.loaded_faces().iter().filter(|f| f.family == "Retype Copy N").map(|f| f.style.clone()).collect();
    assert_eq!(loaded, std::slice::from_ref(&style), "only the face that matched is loaded");
    let left: Vec<String> = db.unloaded_installed_faces().into_iter().filter(|f| f.family == "Retype Copy N").map(|f| f.style).collect();
    assert_eq!(left.len(), 1);
    assert_ne!(left[0], style);
}

#[test]
fn a_loaded_face_of_the_same_names_counts_only_for_the_shapes_it_matches() {
    use vectorcraft_text::test_fonts::named_copy;
    let names = ("Retype Twin", "Regular", "Version 1.000", "RetypeTwin-Regular");
    let db = FontDb::with_font_dirs(vec![]);
    // Inter's glyphs, loaded under the names a file of Source Sans 3's glyphs has too.
    db.add_font(named_copy(true, names).unwrap());
    let dir = vectorcraft_testkit::temp_dir("retype-installed-twin");
    let path = dir.join("RetypeTwin-Regular.ttf");
    std::fs::write(&path, named_copy(false, names).unwrap()).unwrap();
    let read = db.read_unloaded_file(&path, u64::MAX).into_iter().next().unwrap();
    let shapes = glyphs(&db, "Rx");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let keys = keys(&shapes);
    let loaded = super::load(&db, &read);
    assert_ne!(loaded.id(), read.id(), "the face loaded first is kept");
    assert!(super::still_fit(&db, &read, &loaded, &refs, &keys, vec![0, 1]).is_empty(), "Inter's R and x are not these");
    assert_eq!(super::still_fit(&db, &read, &read, &refs, &keys, vec![0, 1]), [0, 1]);
}

#[test]
fn an_installed_face_no_shape_matches_is_read_and_not_loaded() {
    let (db, _) = installed("none", &["Retype Copy B"]);
    let shapes = [kurbo::Circle::new((40.0, 40.0), 20.0).to_path(0.1)];
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let found = search(&db, &refs, false).unwrap();
    assert_eq!((found.read, found.faces.len()), (1, 0));
    assert!(unloaded(&db, "Retype Copy B"), "a face that matches nothing stays unloaded");
    // No shape: no file is read.
    let none = search(&db, &[], false).unwrap();
    assert_eq!((none.read, none.skipped), (0, 0));
}

#[test]
fn plain_shapes_alone_read_no_installed_font() {
    // A hyphen is a bar: many fonts have a glyph of its shape, so it tells no font apart.
    let (db, _) = installed("plain", &["Retype Copy L"]);
    let mut shapes = glyphs(&db, "-");
    shapes.push(kurbo::Rect::new(0.0, 0.0, 80.0, 40.0).to_path(0.1));
    assert!(shapes.iter().all(vectorcraft_text::retype::is_plain));
    let refs: Vec<&BezPath> = shapes.iter().collect();
    for every in [false, true] {
        let found = search(&db, &refs, every).unwrap();
        assert_eq!((found.read, found.skipped, found.faces.len()), (0, 0, 0));
    }
    assert!(unloaded(&db, "Retype Copy L"));
    // Beside a shape that is not plain, the search reads the file for that shape.
    shapes.extend(glyphs(&db, "q"));
    let refs: Vec<&BezPath> = shapes.iter().collect();
    assert_eq!(search(&db, &refs, false).unwrap().faces.len(), 1);
}

#[test]
fn the_search_stops_once_every_shape_matches_unless_every_face_is_wanted() {
    let (db, _) = installed("every", &["Retype Copy C", "Retype Copy D"]);
    let shapes = glyphs(&db, "ab");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let first = search(&db, &refs, false).unwrap();
    assert_eq!((first.read, first.faces.len()), (1, 1), "the second file is not read");
    let (db, _) = installed("every-all", &["Retype Copy E", "Retype Copy F"]);
    let all = search(&db, &refs, true).unwrap();
    assert_eq!((all.read, all.faces.len()), (2, 2));
    let mut families: Vec<&str> = all.faces.iter().map(|f| f.family.as_str()).collect();
    families.sort_unstable();
    assert_eq!(families, ["Retype Copy E", "Retype Copy F"]);
}

#[test]
fn faces_past_the_limits_are_skipped_and_counted() {
    let (db, files) = installed("limits", &["Retype Copy G", "Retype Copy H"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let len = files.iter().map(|f| std::fs::metadata(f).unwrap().len()).max().unwrap();
    // Files longer than the limit for one file.
    let open = Limits { file: u64::MAX, read: u64::MAX, glyphs: usize::MAX, faces: usize::MAX };
    let all = |_: &vectorcraft_text::FontFace| true;
    let small = search_within(&db, &refs, true, Limits { file: len / 2, ..open }, &all).unwrap();
    assert_eq!((small.read, small.skipped, small.faces.len()), (0, 2, 0));
    // Bytes for one file only.
    let one = search_within(&db, &refs, true, Limits { read: len + len / 2, ..open }, &all).unwrap();
    assert_eq!((one.read, one.skipped, one.faces.len()), (1, 1, 1));
    // Glyphs for no face.
    let (db, _) = installed("limits-glyphs", &["Retype Copy I"]);
    let few = search_within(&db, &refs, true, Limits { glyphs: 10, ..open }, &all).unwrap();
    assert_eq!((few.read, few.skipped, few.faces.len()), (0, 1, 0));
    assert!(unloaded(&db, "Retype Copy I"));
    // Faces found up to the cap: the faces after it are not compared, and count as skipped.
    let (db, _) = installed("limits-faces", &["Retype Copy O", "Retype Copy P", "Retype Copy Q"]);
    let capped = search_within(&db, &refs, true, Limits { faces: 1, ..open }, &all).unwrap();
    assert_eq!((capped.read, capped.skipped, capped.faces.len()), (1, 2, 1));
}

#[test]
fn faces_a_list_turns_down_are_loaded_and_take_no_place() {
    let (db, _) = installed("listed", &["Retype Copy R", "Retype Copy S", "Retype Copy T"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    // Two places, and the first face found is turned down: the next two are listed.
    let found = super::search_listed(&db, &refs, 2, &|f| f.family != "Retype Copy R").unwrap();
    let mut families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    families.sort_unstable();
    assert_eq!(families, ["Retype Copy S", "Retype Copy T"]);
    assert_eq!((found.read, found.skipped), (3, 0));
    assert!(!unloaded(&db, "Retype Copy R"), "the face turned down was loaded");
}

#[test]
fn a_cancel_stops_the_search_between_files() {
    let (db, _) = installed("cancel-between", &["Retype Copy U", "Retype Copy V", "Retype Copy W"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    let stop = token.clone();
    // The cancel comes as the second file starts: it is read, the third is not.
    let sink: vectorcraft_doc::cancel::ProgressSink = std::rc::Rc::new(move |done, _, _| {
        if done == 1 {
            stop.cancel();
        }
    });
    let r = vectorcraft_doc::cancel::scope(&token, Some(sink), || search(&db, &refs, true));
    assert!(matches!(r, Err(crate::EngineError::Cancelled)));
    let left = ["Retype Copy U", "Retype Copy V", "Retype Copy W"].iter().filter(|f| unloaded(&db, f)).count();
    assert_eq!(left, 1, "two files were read and loaded, the third was not");
}

#[test]
fn a_run_away_from_the_command_loads_the_faces_it_finds() {
    let (db, _) = installed("run", &["Retype Copy X"]);
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let ids: Vec<u64> =
        glyphs(&db, "Wk").iter().map(|p| s.execute("path.create", &json!({"d": p.to_svg()})).unwrap()["id"].as_u64().unwrap()).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    // Off: nothing to run.
    for cmd in ["type.retype", "type.retypeFonts"] {
        assert!(super::RetypeRun::prepare_in(&s, cmd, &json!({"installed": false}), &db).unwrap().is_none());
    }
    // The command's params are checked as it checks them.
    assert!(super::RetypeRun::prepare_in(&s, "type.retypeFonts", &json!({"count": 0}), &db).is_err());
    assert!(super::RetypeRun::prepare_in(&s, "type.retype", &json!({"fonts": 1}), &db).is_err());
    assert!(super::RetypeRun::prepare_in(&s, "type.text", &json!({}), &db).is_err());
    // Retype: the loaded Source Sans 3 matches both shapes, so no installed file is read.
    let run = super::RetypeRun::prepare_in(&s, "type.retype", &json!({}), &db).unwrap().unwrap();
    assert_eq!(run.command(), "type.retype");
    assert_eq!(run.params(), &json!({}));
    assert_eq!(run.run_in(&db).unwrap().json(), json!({"read": 0, "skipped": 0}));
    assert!(unloaded(&db, "Retype Copy X"));
    // The list compares every installed face: the copy is read and loaded.
    let run = super::RetypeRun::prepare_in(&s, "type.retypeFonts", &json!({"count": 5}), &db).unwrap().unwrap();
    assert_eq!(run.params(), &json!({"count": 5}));
    assert!(run.current(&s));
    let found = run.run_in(&db).unwrap();
    assert_eq!(found.json(), json!({"read": 1, "skipped": 0}));
    assert_eq!(found.installed.faces.iter().map(|f| f.family.as_str()).collect::<Vec<_>>(), ["Retype Copy X"]);
    assert!(!unloaded(&db, "Retype Copy X"));
    // A change of selection or content: it no longer applies.
    s.execute("select.set", &json!({"ids": [ids[0]]})).unwrap();
    assert!(!run.current(&s));
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    assert!(run.current(&s));
    s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
    assert!(!run.current(&s));
}

#[test]
fn what_a_search_found_is_handed_to_the_next_command_in_its_document() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let uid = s.doc().unwrap().uid;
    let handed = |doc| super::RetypeFound { doc, installed: super::Installed { read: 7, skipped: 3, ..Default::default() } };
    // Taken instead of a search of the installed fonts, also with installed false.
    handed(uid).hand_in(&mut s);
    let r = s.execute("type.retypeFonts", &json!({"installed": false})).unwrap();
    assert_eq!(r["installed"], json!({"read": 7, "skipped": 3}), "{r}");
    // Used once.
    let r = s.execute("type.retypeFonts", &json!({"installed": false})).unwrap();
    assert_eq!(r["installed"], json!({"read": 0, "skipped": 0}), "{r}");
    // Another document's is dropped.
    handed(uid + 1).hand_in(&mut s);
    let r = s.execute("type.retypeFonts", &json!({"installed": false})).unwrap();
    assert_eq!(r["installed"], json!({"read": 0, "skipped": 0}), "{r}");
    assert!(s.retype_hand_off.is_none(), "taken");
    // Retype takes it too, whether or not a loaded font matches the rectangle.
    handed(uid).hand_in(&mut s);
    let _ = s.execute("type.retype", &json!({"installed": false}));
    assert!(s.retype_hand_off.is_none());
}

#[test]
fn a_canceled_search_stops_before_reading_a_file() {
    let (db, _) = installed("cancel", &["Retype Copy J"]);
    let shapes = glyphs(&db, "z");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || search(&db, &refs, false));
    assert!(matches!(r, Err(crate::EngineError::Cancelled)));
    assert!(unloaded(&db, "Retype Copy J"), "nothing was loaded");
}

/// The shapes of `text` set in Source Sans 3 and outlined, as `type.retype` collects them.
fn pieces(text: &str) -> Vec<super::super::Piece> {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let id =
        s.execute("text.create", &json!({"x": 40, "y": 100, "text": text, "size": 30, "font": "Source Sans 3"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let group = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let d = &s.doc().unwrap().doc;
    let node = d.node(vectorcraft_doc::NodeId(group)).unwrap();
    let (mut barrier, mut truncated, mut pieces) = (true, false, vec![]);
    super::super::collect(node, None, 0, &mut barrier, &mut truncated, &mut pieces);
    pieces
}

#[test]
fn retype_fills_the_shapes_no_loaded_font_matches_from_the_installed_fonts() {
    let (db, _) = installed("retype", &["Retype Copy K"]);
    let pieces = pieces("Wk-");
    assert_eq!(pieces.len(), 3);
    // The first shape is taken as matched already: the others are looked for.
    let mut found = vec![vec![], vec![], vec![]];
    let keep = super::super::RetypeIndex::build(&db, db.face("Source Sans 3", "Regular"), &keys(&[pieces[0].path.clone()]));
    found[0] = keep.matches(&pieces[0].path);
    assert!(!found[0].is_empty());
    let installed = super::super::from_installed(&db, &pieces, &mut found).unwrap();
    assert_eq!(installed.read, 1);
    assert_eq!(found[0].first().map(|m| m.face.family.as_str()), Some("Source Sans 3"), "a matched shape keeps its match");
    // and gets the face found too, so its line can be set in the face the others match.
    assert!(found[0].iter().any(|m| m.face.family == "Retype Copy K" && m.ch == 'W'), "the W also matches the face found");
    let m = found[1].first().expect("the installed copy matches the k");
    assert_eq!((m.face.family.as_str(), m.ch), ("Retype Copy K", 'k'));
    assert!(db.loaded_faces().iter().any(|f| f.id() == m.face.id()), "the match is in a loaded face");
    // The hyphen, a plain shape, matches the font the k found.
    let m = found[2].first().expect("the installed copy matches the hyphen");
    assert_eq!((m.face.family.as_str(), m.ch), ("Retype Copy K", '-'));
}

#[test]
fn retype_reads_no_installed_font_for_plain_shapes_alone() {
    let (db, _) = installed("retype-plain", &["Retype Copy M"]);
    let pieces = pieces("W-");
    let mut found = vec![vec![], vec![]];
    let keep = super::super::RetypeIndex::build(&db, db.face("Source Sans 3", "Regular"), &keys(&[pieces[0].path.clone()]));
    found[0] = keep.matches(&pieces[0].path);
    let installed = super::super::from_installed(&db, &pieces, &mut found).unwrap();
    assert_eq!((installed.read, installed.faces.len()), (0, 0));
    assert!(found[1].is_empty(), "the hyphen alone stays unmatched");
    assert!(unloaded(&db, "Retype Copy M"));
}

#[test]
fn installed_must_be_a_bool_or_null() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    for cmd in ["type.retype", "type.retypeFonts"] {
        for bad in [json!(1), json!("yes"), json!([])] {
            let e = s.execute(cmd, &json!({"installed": bad})).unwrap_err().to_string();
            assert!(e.contains("`installed` must be true, false or null"), "{cmd} {bad}: {e}");
        }
    }
    // false: no installed font is read, and the result says so.
    let r = s.execute("type.retypeFonts", &json!({"installed": false})).unwrap();
    assert_eq!(r["installed"], json!({"read": 0, "skipped": 0}), "{r}");
}
