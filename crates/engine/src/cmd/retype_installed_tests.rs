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

/// `paths` as a search compares them, each matched whole.
fn whole(paths: Vec<BezPath>) -> Vec<super::Shape> {
    paths.into_iter().map(super::Shape::whole).collect()
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
    let read = db.read_unloaded_file(&path, u64::MAX).faces.into_iter().next().unwrap();
    let shapes = glyphs(&db, "Rx");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let keys = keys(&shapes);
    let loaded = super::load(&db, &read);
    assert_ne!(loaded.id(), read.id(), "the face loaded first is kept");
    let fit = |ix: &super::super::RetypeIndex, k: usize| !ix.matches(refs[k]).is_empty();
    let index = || super::super::RetypeIndex::build_unloaded(&db, read.clone(), &keys);
    let (fits, _) = super::still_fit(&db, &read, &loaded, index(), &keys, &[], vec![0, 1], &fit);
    assert!(fits.is_empty(), "Inter's R and x are not these");
    let (fits, _) = super::still_fit(&db, &read, &read, index(), &keys, &[], vec![0, 1], &fit);
    assert_eq!(fits, [0, 1]);
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
fn a_shape_without_a_glyph_s_key_does_not_keep_the_search_going() {
    let (db, _) = installed("keyless", &["Retype Copy Keyless A", "Retype Copy Keyless B"]);
    let mut shapes = glyphs(&db, "k");
    // More contours than any glyph's key counts: no face can match it.
    let mut dots = BezPath::new();
    for i in 0..49 {
        dots.extend(kurbo::Circle::new((10.0 * f64::from(i), 300.0), 3.0).path_elements(0.1));
    }
    assert!(shape_key(&dots).is_none() && !vectorcraft_text::retype::is_plain(&dots));
    shapes.push(dots);
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let found = search(&db, &refs, false).unwrap();
    assert_eq!((found.read, found.faces.len()), (1, 1), "the second file is not read");
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
fn faces_read_are_kept_for_split_shapes_only() {
    let (db, _) = installed("kept-whole", &["Retype Kept Whole A", "Retype Kept Whole B", "Retype Kept Whole C"]);
    // The k matches in the first file and the circle in none, so every file is read; the later
    // files hold the k's glyph without completing a shape. With every shape whole, none is kept.
    let mut shapes = glyphs(&db, "k");
    shapes.push(kurbo::Circle::new((300.0, 100.0), 7.3).to_path(0.1));
    assert!(shape_key(&shapes[1]).is_some(), "the circle has a key to look glyphs up by");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    super::KEPT_PARTIAL.with(|n| n.set(0));
    let found = search(&db, &refs, false).unwrap();
    assert_eq!((found.read, found.faces.len()), (3, 1));
    assert_eq!(super::KEPT_PARTIAL.with(std::cell::Cell::get), 0, "no face read is kept");
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
fn a_file_that_grows_past_the_limits_after_the_listing_is_skipped_and_its_bytes_count() {
    let (db, mut files) = installed("grown", &["Retype Copy Y", "Retype Copy Z"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    // The order the search reads them in: smallest first.
    let len = |p: &PathBuf| std::fs::metadata(p).unwrap().len();
    files.sort_by(|a, b| len(a).cmp(&len(b)).then_with(|| a.cmp(b)));
    let (first, second) = (len(&files[0]), len(&files[1]));
    let grown = files[0].clone();
    // The first file grows past the limit for one file as its reading starts, after the listing.
    let sink: vectorcraft_doc::cancel::ProgressSink = std::rc::Rc::new(move |done, _, _| {
        if done == 0 {
            let mut data = std::fs::read(&grown).unwrap();
            data.resize(data.len() + second as usize + 1, 0);
            std::fs::write(&grown, data).unwrap();
        }
    });
    // Bytes for both files as listed: the grown file's reading takes one more than the limit for
    // one file, which leaves too little for the second.
    let limits = Limits { file: second, read: first + second, glyphs: usize::MAX, faces: usize::MAX };
    let token = vectorcraft_doc::cancel::CancelToken::new();
    let all = |_: &vectorcraft_text::FontFace| true;
    let found = vectorcraft_doc::cancel::scope(&token, Some(sink), || search_within(&db, &refs, true, limits, &all)).unwrap();
    assert_eq!((found.read, found.skipped, found.faces.len()), (0, 2, 0));
}

#[test]
fn the_glyph_budget_counts_the_glyphs_drawn() {
    let (db, files) = installed("glyph-budget", &["Retype Copy Budget"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let face = db.read_unloaded_file(&files[0], u64::MAX).faces.into_iter().next().unwrap();
    let mapped = face.mapped_chars().len();
    let drawn = vectorcraft_text::retype::encoded_glyphs(&face);
    assert!(drawn < mapped, "{drawn} of {mapped}");
    // A budget below the face's characters and at the glyphs drawn: the face is compared.
    let open = Limits { file: u64::MAX, read: u64::MAX, glyphs: drawn, faces: usize::MAX };
    let found = search_within(&db, &refs, true, open, &|_| true).unwrap();
    assert_eq!((found.read, found.skipped, found.faces.len()), (1, 0, 1));
}

#[test]
fn a_run_away_from_the_list_hands_in_the_loaded_faces_that_match_with_the_installed_ones() {
    let (db, _) = installed("list-run", &["Retype Copy List"]);
    db.add_font(
        vectorcraft_text::test_fonts::named_copy(false, ("Retype Loaded List", "Regular", "Version 1.000", "RetypeLoadedList-Regular")).unwrap(),
    );
    let loaded: Vec<_> = db.loaded_faces().into_iter().filter(|f| f.family == "Retype Loaded List").collect();
    let search = super::RetypeSearch {
        command: "type.retypeFonts",
        doc: 1,
        shapes: whole(glyphs(&db, "Rx")),
        faces: super::Faces::Given(loaded.clone()),
        tolerance: 0.0,
    };
    let found = search.run_in(&db).unwrap();
    let families: Vec<&str> = found.installed.faces.iter().map(|f| f.family.as_str()).collect();
    // The loaded face first, then the installed one, each once.
    assert_eq!(families, ["Retype Loaded List", "Retype Copy List"]);
    assert_eq!((found.installed.read, found.installed.skipped), (1, 0));
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
    // Only the faces listed are compared ahead of the other loaded faces later.
    let ahead: Vec<String> = super::found(&db).iter().map(|f| f.family.clone()).collect();
    assert!(ahead.iter().any(|f| f == "Retype Copy S") && !ahead.iter().any(|f| f == "Retype Copy R"), "{ahead:?}");
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
        assert!(super::RetypeRun::prepare(&s, cmd, &json!({"installed": false})).unwrap().is_none());
    }
    // The command's params are checked as it checks them.
    assert!(super::RetypeRun::prepare(&s, "type.retypeFonts", &json!({"count": 0})).is_err());
    assert!(super::RetypeRun::prepare(&s, "type.retype", &json!({"fonts": 1})).is_err());
    assert!(super::RetypeRun::prepare(&s, "type.text", &json!({})).is_err());
    // An unknown key fails before any search, as the command fails on it.
    for cmd in ["type.retype", "type.retypeFonts"] {
        let e = super::RetypeRun::prepare(&s, cmd, &json!({"typo": true})).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(e.contains("typo"), "{cmd}: {e}");
        assert!(s.execute(cmd, &json!({"typo": true})).is_err());
    }
    // Retype: the loaded Source Sans 3 matches both shapes, so no installed file is read.
    let run = super::RetypeRun::prepare(&s, "type.retype", &json!({})).unwrap().unwrap();
    assert_eq!(run.command(), "type.retype");
    assert_eq!(run.params(), &json!({}));
    assert_eq!(run.run_in(&db).unwrap().json(), json!({"read": 0, "skipped": 0}));
    assert!(unloaded(&db, "Retype Copy X"));
    // The list compares every installed face: the copy is read and loaded.
    let run = super::RetypeRun::prepare(&s, "type.retypeFonts", &json!({"count": 5})).unwrap().unwrap();
    assert_eq!(run.params(), &json!({"count": 5}));
    assert!(run.current(&s));
    let found = run.run_in(&db).unwrap();
    assert_eq!(found.json(), json!({"read": 1, "skipped": 0}));
    // The loaded faces that match are handed in with it, ahead of it.
    let families: Vec<&str> = found.installed.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families.last(), Some(&"Retype Copy X"), "{families:?}");
    assert_eq!(families.iter().filter(|f| **f == "Retype Copy X").count(), 1, "{families:?}");
    assert!(families.contains(&"Source Sans 3"), "{families:?}");
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
fn plain_shapes_alone_start_no_run_away_from_the_command() {
    // A rectangle and a bar are plain: the search reads no installed font for them, so the
    // command runs at once.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let bar = s.execute("shape.rectangle", &json!({"x": 10, "y": 100, "width": 4, "height": 60})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [rect, bar]})).unwrap();
    for (cmd, p) in [("type.retype", json!({})), ("type.retype", json!({"tolerance": 0.05})), ("type.retypeFonts", json!({}))] {
        assert!(super::RetypeRun::prepare(&s, cmd, &p).unwrap().is_none(), "{cmd} {p}");
    }
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
    let groups = vec![vec![]; pieces.len()];
    // The first shape is taken as matched already: the others are looked for.
    let keep = super::super::RetypeIndex::build(&db, db.face("Source Sans 3", "Regular"), &keys(&[pieces[0].path.clone()]));
    let found = super::super::match_pieces(&pieces, &groups, &[&keep], 0.0);
    assert!(!found[0].is_empty() && found[1].is_empty() && found[2].is_empty());
    let left = super::super::left_shapes(&pieces, &groups, &found);
    assert_eq!(left.len(), 2);
    let installed = super::search_for(&db, &left, 0.0, false).unwrap();
    assert_eq!(installed.read, 1);
    // The faces found join the index, and every shape is matched again.
    let all_keys = keys(&pieces.iter().map(|pc| pc.path.clone()).collect::<Vec<_>>());
    let extra = super::super::RetypeIndex::build(&db, installed.faces.iter().cloned(), &all_keys);
    let found = super::super::match_pieces(&pieces, &groups, &[&keep, &extra], 0.0);
    let whole = |k: usize| found[k].first().expect("one part").clone();
    assert_eq!(whole(0).first().map(|m| m.face.family.as_str()), Some("Source Sans 3"), "a matched shape keeps its match");
    // and gets the face found too, so its line can be set in the face the others match.
    assert!(whole(0).iter().any(|m| m.face.family == "Retype Copy K" && m.ch == 'W'), "the W also matches the face found");
    let k = whole(1);
    let m = k.first().expect("the installed copy matches the k");
    assert_eq!((m.face.family.as_str(), m.ch), ("Retype Copy K", 'k'));
    assert!(db.loaded_faces().iter().any(|f| f.id() == m.face.id()), "the match is in a loaded face");
    // The hyphen, a plain shape, matches the font the k found.
    let hyphen = whole(2);
    let m = hyphen.first().expect("the installed copy matches the hyphen");
    assert_eq!((m.face.family.as_str(), m.ch), ("Retype Copy K", '-'));
}

/// `path` flattened into lines `tol` from its curves, each contour started at its second point
/// and run backwards: an outline built nothing like the font's.
fn simplified(path: &BezPath, tol: f64) -> BezPath {
    let mut contours: Vec<Vec<kurbo::Point>> = vec![];
    kurbo::flatten(path.iter(), tol, |el| match el {
        kurbo::PathEl::MoveTo(q) => contours.push(vec![q]),
        kurbo::PathEl::LineTo(q) => contours.last_mut().unwrap().push(q),
        _ => {}
    });
    let mut out = BezPath::new();
    for mut c in contours {
        if c.len() > 1 && c.first() == c.last() {
            c.pop();
        }
        c.rotate_left(1);
        c.reverse();
        out.move_to(c[0]);
        for q in &c[1..] {
            out.line_to(*q);
        }
        out.close_path();
    }
    out
}

#[test]
fn an_installed_face_is_found_through_the_glyphs_of_a_split_shape_and_through_near_matches() {
    // A word outlined as one compound path: the whole shape is no glyph, and its glyphs are.
    let (db, _) = installed("split", &["Retype Copy Split"]);
    let mut word = BezPath::new();
    for g in glyphs(&db, "Wk") {
        word.extend(g.iter());
    }
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let found = super::search_for(&db, &[super::Sought::unpainted(&word, &groups)], 0.0, false).unwrap();
    assert_eq!(found.read, 1);
    let families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families, ["Retype Copy Split"], "the face the split's glyphs match is found");
    assert!(!unloaded(&db, "Retype Copy Split"));
    // A simplified outline: no installed glyph matches it exactly, and one does within a tolerance.
    let (db, _) = installed("near", &["Retype Copy Near"]);
    let r = simplified(&glyphs(&db, "R")[0], 0.05);
    let shape = [super::Sought::unpainted(&r, &[])];
    let exact = super::search_for(&db, &shape, 0.0, false).unwrap();
    assert_eq!((exact.read, exact.faces.len()), (1, 0));
    assert!(unloaded(&db, "Retype Copy Near"));
    let near = super::search_for(&db, &shape, 0.02, false).unwrap();
    let families: Vec<&str> = near.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families, ["Retype Copy Near"], "the face the outline nearly matches is found");
    assert!(!unloaded(&db, "Retype Copy Near"));
}

#[test]
fn a_split_shape_whose_glyphs_are_in_different_installed_faces_finds_them_all() {
    // A W of Source Sans 3 and a k of Inter, outlined as one compound path, with a copy of each
    // font installed: neither face holds both glyphs.
    let dir = vectorcraft_testkit::temp_dir("retype-installed-split-two");
    for (inter, family) in [(false, "Retype Mix S"), (true, "Retype Mix I")] {
        let postscript = format!("{}-Regular", family.replace(' ', ""));
        let data = vectorcraft_text::test_fonts::named_copy(inter, (family, "Regular", "Version 1.000", &postscript)).unwrap();
        std::fs::write(dir.join(format!("{postscript}.ttf")), data).unwrap();
    }
    let db = FontDb::with_font_dirs(vec![dir]);
    let mut word = BezPath::new();
    word.extend(glyphs(&db, "W")[0].iter());
    let inter = db.face("Inter", "Regular").unwrap();
    let k = Affine::translate((70.0, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&inter, inter.glyph_for('k'));
    word.extend(k.iter());
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let found = super::search_for(&db, &[super::Sought::unpainted(&word, &groups)], 0.0, false).unwrap();
    assert_eq!(found.read, 2);
    let mut families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    families.sort_unstable();
    assert_eq!(families, ["Retype Mix I", "Retype Mix S"], "the faces of both glyphs are found");
    assert!(!unloaded(&db, "Retype Mix S") && !unloaded(&db, "Retype Mix I"));
}

#[test]
fn a_split_shape_completes_with_the_loaded_faces_it_is_searched_beside() {
    // A W of Source Sans 3 and a k of Inter, outlined as one compound path. Source Sans 3 is the
    // loaded face compared first; the only other face is an installed copy of Inter.
    let dir = vectorcraft_testkit::temp_dir("retype-installed-beside");
    let data = vectorcraft_text::test_fonts::named_copy(true, ("Retype Beside I", "Regular", "Version 1.000", "RetypeBesideI-Regular")).unwrap();
    std::fs::write(dir.join("RetypeBesideI-Regular.ttf"), data).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let mut word = BezPath::new();
    word.extend(glyphs(&db, "W")[0].iter());
    let inter = db.face("Inter", "Regular").unwrap();
    let k = Affine::translate((70.0, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&inter, inter.glyph_for('k'));
    word.extend(k.iter());
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let shapes = [super::Sought::unpainted(&word, &groups)];
    let (keys, near) = super::keys_of(&shapes, 0.0);
    let first = super::super::RetypeIndex::build_near(&db, db.face("Source Sans 3", "Regular"), &keys, &near);
    let found = super::search_beside(&db, &shapes, &[&first], 0.0, false).unwrap();
    let families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families, ["Source Sans 3", "Retype Beside I"], "the installed face completes the loaded face's glyphs");
}

#[test]
fn one_loaded_face_per_glyph_completes_a_split_shape_however_many_hold_it() {
    // A W of Source Sans 3 and a k of Inter in one compound path. More loaded copies of Source
    // Sans 3 than a search finds faces hold the W; an installed copy of Inter holds the k.
    let n = vectorcraft_text::retype::MAX_FACES + 4;
    let dir = vectorcraft_testkit::temp_dir("retype-installed-many");
    let data = vectorcraft_text::test_fonts::named_copy(true, ("Retype Many I", "Regular", "Version 1.000", "RetypeManyI-Regular")).unwrap();
    std::fs::write(dir.join("RetypeManyI-Regular.ttf"), data).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    for i in 0..n {
        let family = format!("Retype Many {i}");
        db.add_font(
            vectorcraft_text::test_fonts::named_copy(false, (&family, "Regular", "Version 1.000", &format!("RetypeMany{i}-Regular"))).unwrap(),
        );
    }
    let mut word = BezPath::new();
    word.extend(glyphs(&db, "W")[0].iter());
    let inter = db.face("Inter", "Regular").unwrap();
    let k = Affine::translate((70.0, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&inter, inter.glyph_for('k'));
    word.extend(k.iter());
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    let shapes = [super::Sought::unpainted(&word, &groups)];
    let (keys, near) = super::keys_of(&shapes, 0.0);
    let copies: Vec<_> = db.loaded_faces().into_iter().filter(|f| f.family.starts_with("Retype Many ")).collect();
    assert_eq!(copies.len(), n);
    let indexes: Vec<super::super::RetypeIndex> = copies
        .chunks(vectorcraft_text::retype::MAX_FACES)
        .map(|g| super::super::RetypeIndex::build_near(&db, g.iter().cloned(), &keys, &near))
        .collect();
    let beside: Vec<&super::super::RetypeIndex> = indexes.iter().collect();
    let found = super::search_beside(&db, &shapes, &beside, 0.0, false).unwrap();
    let families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families.len(), 2, "{families:?}");
    assert_eq!(families.last(), Some(&"Retype Many I"));
}

#[test]
fn a_split_shape_completes_with_loaded_faces_past_the_first_and_installed_ones() {
    // A W of Source Sans 3 and a k of Inter in one compound path. Past the faces compared first
    // (the bundled ones), a loaded copy of Inter holds the k and an installed copy of Source Sans 3
    // the W.
    let dir = vectorcraft_testkit::temp_dir("retype-installed-kept");
    let data = vectorcraft_text::test_fonts::named_copy(false, ("Retype Kept S", "Regular", "Version 1.000", "RetypeKeptS-Regular")).unwrap();
    std::fs::write(dir.join("RetypeKeptS-Regular.ttf"), data).unwrap();
    let db = FontDb::with_font_dirs(vec![dir]);
    let compared: Vec<u32> = db.loaded_faces().iter().map(|f| f.id()).collect();
    db.add_font(vectorcraft_text::test_fonts::named_copy(true, ("Retype Kept I", "Regular", "Version 1.000", "RetypeKeptI-Regular")).unwrap());
    let mut word = BezPath::new();
    word.extend(glyphs(&db, "W")[0].iter());
    let inter = db.face("Inter", "Regular").unwrap();
    let k = Affine::translate((70.0, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&inter, inter.glyph_for('k'));
    word.extend(k.iter());
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let shapes = [super::Sought::unpainted(&word, &groups)];
    let (rest, done, kept) = super::loaded_rest_for(&db, &shapes, &[], 0.0, &compared).unwrap();
    assert!(rest.is_empty() && done == [false]);
    assert_eq!(kept.len(), 1, "the group with the k is kept for the installed fonts");
    let beside: Vec<&super::super::RetypeIndex> = kept.iter().collect();
    let found = super::search_beside(&db, &shapes, &beside, 0.0, false).unwrap();
    let families: Vec<&str> = found.faces.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families, ["Retype Kept I", "Retype Kept S"], "the loaded copy's k and the installed copy's W");
}

#[test]
fn an_installed_face_whose_glyphs_paint_otherwise_than_the_shape_is_not_found() {
    // Two o's moved onto each other in one compound path filled by the even-odd rule: the overlap
    // is unpainted, and two glyphs set side by side would paint it.
    let paths = |db: &FontDb| {
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let o = Affine::scale(0.03) * db.outline_uncached(&face, face.glyph_for('o'));
        let mut word = BezPath::new();
        for x in [40.0, 49.0] {
            word.extend((Affine::translate((x, 100.0)) * o.clone()).iter());
        }
        word
    };
    let (db, _) = installed("paint", &["Retype Copy Paint"]);
    let word = paths(&db);
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let even_odd = super::Sought { path: &word, groups: &groups, fill: Some(false), once: true };
    let found = super::search_for(&db, &[even_odd], 0.0, false).unwrap();
    assert_eq!((found.read, found.faces.len()), (1, 0), "the o's paint the overlap");
    assert!(unloaded(&db, "Retype Copy Paint"));
    // Without a fill to compare, the same glyphs are found.
    let (db, _) = installed("paint-none", &["Retype Copy Unpainted"]);
    let word = paths(&db);
    let found = super::search_for(&db, &[super::Sought::unpainted(&word, &groups)], 0.0, false).unwrap();
    assert_eq!(found.faces.len(), 1);
}

#[test]
fn retype_reads_no_installed_font_for_plain_shapes_alone() {
    let (db, _) = installed("retype-plain", &["Retype Copy M"]);
    let pieces = pieces("W-");
    let groups = vec![vec![]; pieces.len()];
    let keep = super::super::RetypeIndex::build(&db, db.face("Source Sans 3", "Regular"), &keys(&[pieces[0].path.clone()]));
    let found = super::super::match_pieces(&pieces, &groups, &[&keep], 0.0);
    let left = super::super::left_shapes(&pieces, &groups, &found);
    assert_eq!(left.len(), 1, "the hyphen is left");
    let installed = super::search_for(&db, &left, 0.0, false).unwrap();
    assert_eq!((installed.read, installed.faces.len()), (0, 0));
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

/// A database with no installed fonts and `n` copies of the bundled Source Sans 3 Regular loaded,
/// named `{tag} 0` to `{tag} n-1`, after `inter` copies of Inter named `{tag} Inter 0` and on.
fn loaded_copies(tag: &str, inter: usize, n: usize) -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    let copies = (0..inter).map(|i| (true, format!("{tag} Inter {i}"))).chain((0..n).map(|i| (false, format!("{tag} {i}"))));
    for (inter, family) in copies {
        let postscript = format!("{}-Regular", family.replace(' ', ""));
        db.add_font(vectorcraft_text::test_fonts::named_copy(inter, (&family, "Regular", "Version 1.000", &postscript)).unwrap());
    }
    db
}

#[test]
fn loaded_faces_past_the_ones_compared_first_are_compared_too() {
    let n = vectorcraft_text::retype::MAX_FACES + 4;
    let db = loaded_copies("Retype Rest", 0, n);
    let shapes = glyphs(&db, "Rx");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    // Every loaded face that matches, more than one index takes.
    let rest = super::loaded_rest(&db, &refs, &[]).unwrap();
    for i in 0..n {
        assert!(rest.iter().any(|f| f.family == format!("Retype Rest {i}")), "copy {i} of {}", rest.len());
    }
    // The faces compared already are left out.
    let last = format!("Retype Rest {}", n - 1);
    let compared: Vec<u32> = db.loaded_faces().iter().filter(|f| f.family != last).map(|f| f.id()).collect();
    let rest = super::loaded_rest(&db, &refs, &compared).unwrap();
    assert_eq!(rest.iter().map(|f| f.family.as_str()).collect::<Vec<_>>(), [last.as_str()]);
    // A plain shape alone compares none.
    let bar = [kurbo::Rect::new(0.0, 0.0, 80.0, 10.0).to_path(0.1)];
    assert!(super::loaded_rest(&db, &[&bar[0]], &[]).unwrap().is_empty());
}

#[test]
fn a_split_shape_completes_with_loaded_faces_past_the_ones_compared_first() {
    // A W of Source Sans 3 and a k of Inter in one compound path. Source Sans 3 and Inter are
    // compared first; past them, a loaded copy of Inter holds the k.
    let db = loaded_copies("Retype Rest Split", 1, 0);
    let mut word = BezPath::new();
    word.extend(glyphs(&db, "W")[0].iter());
    let inter = db.face("Inter", "Regular").unwrap();
    let k = Affine::translate((70.0, 100.0)) * Affine::scale(0.03) * db.outline_uncached(&inter, inter.glyph_for('k'));
    word.extend(k.iter());
    let mut budget = super::super::MAX_SPLIT_TOTAL_CONTOURS;
    let groups = super::super::split_groups(&word, &mut budget);
    assert_eq!(groups.len(), 2);
    let shapes = [super::Sought::unpainted(&word, &groups)];
    let (keys, near) = super::keys_of(&shapes, 0.0);
    let source = db.face("Source Sans 3", "Regular").unwrap();
    let compared = [source.id(), inter.id()];
    let first = super::super::RetypeIndex::build_near(&db, [source], &keys, &near);
    let (rest, done, kept) = super::loaded_rest_for(&db, &shapes, &[&first], 0.0, &compared).unwrap();
    assert!(kept.is_empty(), "no split shape is left");
    let families: Vec<&str> = rest.iter().map(|f| f.family.as_str()).collect();
    assert_eq!(families, ["Retype Rest Split Inter 0"], "the copy completes the glyphs of the face compared first");
    assert_eq!(done, [true]);
}

#[test]
fn files_past_the_glyph_budget_are_skipped_without_being_read() {
    let (db, mut files) = installed("budget-spent", &["Retype Copy E1", "Retype Copy E2"]);
    let shapes = glyphs(&db, "k");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    // The order the search reads them in: smallest first.
    let len = |p: &PathBuf| std::fs::metadata(p).unwrap().len();
    files.sort_by(|a, b| len(a).cmp(&len(b)).then_with(|| a.cmp(b)));
    assert_eq!(db.unloaded_installed_faces().len(), 2);
    let face = db.read_unloaded_file(&files[0], u64::MAX).faces.into_iter().next().unwrap();
    let drawn = vectorcraft_text::retype::encoded_glyphs(&face);
    // The second file holds no font after the listing: had it been read, it would give no face
    // to skip.
    std::fs::write(&files[1], vec![0u8; len(&files[1]) as usize]).unwrap();
    let limits = Limits { file: u64::MAX, read: u64::MAX, glyphs: drawn, faces: usize::MAX };
    let found = search_within(&db, &refs, true, limits, &|_| true).unwrap();
    assert_eq!((found.read, found.skipped, found.faces.len()), (1, 1, 1));
}

#[test]
fn a_run_away_from_the_command_hands_in_the_loaded_faces_past_the_ones_compared_first() {
    let db = loaded_copies("Retype Run Rest", vectorcraft_text::retype::MAX_FACES, 1);
    let shapes = glyphs(&db, "Rx");
    // The Inter copies are compared first: none matches, and the copy loaded after them does.
    let faces: Vec<_> = db.loaded_faces().into_iter().filter(|f| f.family.starts_with("Retype Run Rest Inter")).collect();
    assert_eq!(faces.len(), vectorcraft_text::retype::MAX_FACES);
    let search =
        super::RetypeSearch { command: "type.retype", doc: 1, shapes: whole(shapes.clone()), faces: super::Faces::Given(faces), tolerance: 0.0 };
    let found = search.run_in(&db).unwrap();
    assert!(found.installed.faces.iter().any(|f| f.family == "Retype Run Rest 0"), "the loaded copy is handed in");
    assert_eq!((found.installed.read, found.installed.skipped), (0, 0), "no installed file is read for it");
    // Every loaded face was compared: the command does not compare them again.
    assert!(db.loaded_faces().iter().all(|f| found.installed.scanned.contains(&f.id())));
    // A face compared first that matches is handed in too, as loading others may move it past
    // the faces the command compares first.
    let copy: Vec<_> = db.loaded_faces().into_iter().filter(|f| f.family == "Retype Run Rest 0").collect();
    let found = super::RetypeSearch { command: "type.retype", doc: 1, shapes: whole(shapes), faces: super::Faces::Given(copy), tolerance: 0.0 }
        .run_in(&db)
        .unwrap();
    assert_eq!(found.installed.faces.first().map(|f| f.family.as_str()), Some("Retype Run Rest 0"));
}

#[test]
fn a_full_list_counts_skipped_installed_faces_only_when_a_shape_would_start_a_search() {
    let (db, _) = installed("full-list", &["Retype Copy Full"]);
    for i in 0..super::super::fonts::MAX_COUNT {
        let family = format!("Retype Full {i}");
        let postscript = format!("RetypeFull{i}-Regular");
        db.add_font(vectorcraft_text::test_fonts::named_copy(false, (&family, "Regular", "Version 1.000", &postscript)).unwrap());
    }
    let faces: Vec<_> = db.loaded_faces().into_iter().filter(|f| f.family.starts_with("Retype Full ")).collect();
    // A hyphen is plain: it fills the list and would start no search, so nothing is skipped.
    let hyphen = glyphs(&db, "-");
    let refs: Vec<&BezPath> = hyphen.iter().collect();
    let (ranked, found) = super::super::fonts::rank_fonts(&db, faces.clone(), &refs, true, None).unwrap();
    assert_eq!(ranked.len(), super::super::fonts::MAX_COUNT);
    assert_eq!((found.read, found.skipped), (0, 0));
    // A shape that is not plain would: the installed face counts as skipped.
    let shapes = glyphs(&db, "-q");
    let refs: Vec<&BezPath> = shapes.iter().collect();
    let (_, found) = super::super::fonts::rank_fonts(&db, faces, &refs, true, None).unwrap();
    assert_eq!((found.read, found.skipped), (0, 1));
    assert!(unloaded(&db, "Retype Copy Full"));
}

#[test]
fn what_is_handed_in_and_not_taken_can_be_dropped() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let doc = s.doc().unwrap().uid;
    super::RetypeFound { doc, installed: super::Installed::default() }.hand_in(&mut s);
    assert!(s.retype_hand_off.is_some());
    super::RetypeFound::drop_handed(&mut s);
    assert!(s.retype_hand_off.is_none());
}
