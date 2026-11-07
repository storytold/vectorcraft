//! Two installed versions of one family and style (a font library keeps an old version beside a
//! new one): both load, the family and style alone keep resolving to the first, and type that names
//! the other version is set in it.

use kurbo::Point;
use vectorcraft_doc::CharStyle;

use crate::test_fonts::{TWIN_FAMILY as FAMILY, TWIN_VERSIONS as VERSIONS, twin_font};
use crate::*;

/// A database of the bundled fonts, the newer version of the test family loaded first.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(twin_font(true).expect("the newer test font builds")), 1);
    assert_eq!(db.add_font(twin_font(false).expect("the older test font builds")), 1, "another version is kept");
    assert_eq!(db.add_font(twin_font(false).expect("the older test font builds")), 0, "the same version isn't added twice");
    db
}

fn width(db: &FontDb, version: Option<&str>) -> f64 {
    let st = CharStyle { font_family: FAMILY.into(), font_version: version.map(Into::into), size: 100.0, ..CharStyle::default() };
    layout(db, &TextObject::point(Point::ZERO, "Hamburgefonstiv", st)).glyphs.iter().map(|g| g.advance).sum()
}

#[test]
fn a_version_is_found_by_its_version_string() {
    let db = db();
    let face = |v: Option<&str>| db.face_version(FAMILY, "Regular", v).expect("a face");
    assert_eq!(db.face(FAMILY, "Regular").expect("a face").version, VERSIONS[1], "the first loaded stays the default");
    assert_eq!(face(None).version, VERSIONS[1]);
    assert_eq!(face(Some(VERSIONS[0])).version, VERSIONS[0]);
    assert_eq!(face(Some("Version 9.000")).version, VERSIONS[1], "a version not installed: the default");
    let all = db.versions(&face(None));
    assert_eq!(all.iter().map(|f| f.version.as_str()).collect::<Vec<_>>(), [VERSIONS[1], VERSIONS[0]]);
}

#[test]
fn type_naming_a_version_is_set_in_it() {
    let db = db();
    let (newer, older) = (width(&db, None), width(&db, Some(VERSIONS[0])));
    assert!((newer - older).abs() > 50.0, "the two versions set the line differently: {newer} vs {older}");
    assert_eq!(width(&db, Some(VERSIONS[1])), newer);
}
