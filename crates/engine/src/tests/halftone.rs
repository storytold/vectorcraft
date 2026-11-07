//! Object → Vector Halftone.

use serde_json::json;

use crate::tests_adjust::{fill_hex, node, red_rect, session};
use crate::tests_menucmds::{place_undecodable_png, warns_undecoded};
use crate::*;

#[test]
fn halftone_turns_art_into_dots_by_tone() {
    let mut s = session();
    let id = red_rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("object.vectorHalftone", &json!({"frequency": 18})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert!(s.doc().unwrap().doc.node(id).is_none(), "the original is replaced");
    let g = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let dots = r["dots"].as_u64().unwrap();
    // 60 pt at 18 lpi (4 pt cells) turned 45°: about 225 cells, all black.
    assert!((180..=320).contains(&dots), "{dots}");
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 1);
    let (pd, b) = (ch[0].path_data().unwrap(), ch[0].geometric_bounds().unwrap());
    assert!(pd.subpaths.len() as u64 >= dots / 2);
    // Clipped to the art.
    assert!(b.x0 >= 19.9 && b.y0 >= 19.9 && b.x1 <= 80.1 && b.y1 <= 80.1, "{b:?}");
}

#[test]
fn halftone_says_when_an_image_could_not_be_decoded() {
    for mode in ["mono", "cmyk"] {
        let mut s = session();
        let id = red_rect(&mut s);
        s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
        let cut = place_undecodable_png(&mut s);
        s.execute("select.set", &json!({"ids": [id.0, cut.0]})).unwrap();
        let r = s.execute("object.vectorHalftone", &json!({"frequency": 18, "mode": mode})).unwrap();
        assert!(warns_undecoded(&r), "{mode}: {r}");
    }
}

#[test]
fn halftone_options() {
    let mut s = session();
    let id = red_rect(&mut s);
    // Red art in CMYK: magenta and yellow screens, no cyan or black.
    let r = s.execute("object.vectorHalftone", &json!({"mode": "cmyk", "shape": "square", "keepOriginal": true})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_some(), "the original is kept");
    let g = node(&s, NodeId(r["id"].as_u64().unwrap()));
    let inks: Vec<String> = g.children().unwrap().iter().map(|c| fill_hex(c)).collect();
    assert_eq!(inks.len(), 2, "{inks:?}");
    assert!(g.children().unwrap().iter().all(|c| c.blend == vectorcraft_color::BlendMode::Multiply));
    // Light art gets no dots; inverted, it does.
    let mut s = session();
    let id = red_rect(&mut s);
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    assert!(s.execute("object.vectorHalftone", &json!({})).is_err());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    assert!(s.execute("object.vectorHalftone", &json!({"invert": true, "shape": "line"})).is_ok());
    // Bad input is refused.
    for bad in [json!({"frequency": 0}), json!({"shape": "star"}), json!({"mode": "rgb"}), json!({"color": "nope"})] {
        let mut s = session();
        red_rect(&mut s);
        assert!(s.execute("object.vectorHalftone", &bad).is_err(), "{bad}");
    }
}
