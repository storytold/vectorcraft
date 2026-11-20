use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind, TextObject};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

/// Point text, outlined: the id of the outlines' group.
fn outlined(s: &mut Session, mut p: serde_json::Value) -> u64 {
    // A text without a font is set in the system's default face, which differs from machine to
    // machine and decides which glyphs a near match can tell apart.
    if p.get("font").is_none() {
        p["font"] = json!("Source Sans 3");
        p["style"] = json!("Regular");
    }
    let id = s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap()
}

fn text_of(s: &Session, id: u64) -> TextObject {
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("text object") };
    (**t).clone()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

#[test]
fn outlined_point_text_retypes_to_the_same_text_font_size_and_place() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Retype me, 42!", "size": 36, "font": "Source Sans 3", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Retype me, 42!"]), "{r}");
    assert_eq!(r["unmatched"], json!([]));
    let id = r["ids"][0].as_u64().unwrap();
    let t = text_of(&s, id);
    assert_eq!(t.plain_text(), "Retype me, 42!");
    let st = t.first_style();
    assert_eq!((st.font_family.as_str(), st.font_style.as_str()), ("Source Sans 3", "Regular"));
    assert!(near(st.size, 36.0, 0.01), "size {}", st.size);
    let o = t.xf.translation();
    assert!(near(o.x, 40.0, 0.01) && near(o.y, 100.0, 0.01), "baseline start {o:?}");
    // The outlines are gone, in one undo step that brings them back.
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(g)).is_none(), "the emptied group goes");
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(g)).is_some() && d.node(NodeId(id)).is_none());
}

#[test]
fn turned_bold_and_serif_type_keeps_its_font_and_angle() {
    let mut s = session();
    let a = outlined(&mut s, json!({"x": 100, "y": 200, "text": "Bold", "size": 24, "font": "Source Sans 3", "style": "Bold", "rotation": 30}));
    let b = outlined(&mut s, json!({"x": 300, "y": 80, "text": "Serif", "size": 18, "font": "Source Serif 4", "style": "Regular"}));
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Bold", "Serif"]), "{r}");
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let bold = text_of(&s, ids[0]);
    assert_eq!(bold.first_style().font_style, "Bold");
    assert!(near(bold.first_style().size, 24.0, 0.01));
    // text.create turns as kurbo does: compare the turn the new type has with the old one's.
    let turned = kurbo::Affine::translate((100.0, 200.0)) * vectorcraft_geom::rotate_deg(30.0);
    for (x, y) in bold.xf.as_coeffs().iter().zip(turned.as_coeffs()) {
        assert!(near(*x, y, 1e-3), "{:?} vs {:?}", bold.xf, turned);
    }
    let serif = text_of(&s, ids[1]);
    assert_eq!(serif.first_style().font_family, "Source Serif 4");
    assert!(near(serif.first_style().size, 18.0, 0.01));
}

#[test]
fn a_drawn_shape_is_reported_and_left_alone() {
    let mut s = session();
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = s.execute("type.retype", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no selected shape matches"), "{e}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
    // With outlines that would match, a bad `fonts` is refused for itself, and nothing changes.
    let g = outlined(&mut s, json!({"x": 40, "y": 200, "text": "AB", "size": 40}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    for bad in [json!("Source Sans 3"), json!([1]), json!({"family": "x"}), json!(null)] {
        let e = s.execute("type.retype", &json!({"fonts": bad})).unwrap_err().to_string();
        assert!(e.contains("fonts must be a list of family names"), "{e}");
    }
    assert!(s.doc().unwrap().doc == before);
}

#[test]
fn mixed_selection_retypes_the_glyphs_and_keeps_the_rest() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Mix", "size": 30}));
    let rect = s.execute("shape.rectangle", &json!({"x": 10, "y": 200, "width": 80, "height": 40})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g, rect]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Mix"]));
    assert_eq!(r["unmatched"], json!([rect]));
    let d = &s.doc().unwrap().doc;
    assert!(matches!(d.node(NodeId(rect)).unwrap().kind, NodeKind::Path { .. }), "the rectangle stays");
}

#[test]
fn two_words_keep_their_space() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 20, "y": 60, "text": "two  words", "size": 20}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["two  words"]));
}

#[test]
fn craft_fonts_glyphs_retype_too() {
    let Some(f) = vectorcraft_text::CRAFT_FONTS.iter().find(|f| f.is_japanese()) else {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    };
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 20, "y": 60, "text": "漢字です", "size": 20, "font": f.family, "style": f.style}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({"fonts": [f.family]})).unwrap();
    assert_eq!(r["texts"], json!(["漢字です"]), "{r}");
    assert_eq!(r["fonts"][0]["family"], json!(f.family));
}

fn children_of(s: &Session, g: u64) -> Vec<u64> {
    s.doc().unwrap().doc.node(NodeId(g)).unwrap().children().unwrap().iter().map(|c| c.id.0).collect()
}

#[test]
fn a_shape_between_glyphs_ends_the_line_so_paint_order_holds() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let rect = s.execute("shape.rectangle", &json!({"x": 30, "y": 60, "width": 80, "height": 50})).unwrap()["id"].as_u64().unwrap();
    // The rectangle goes between A and B, inside the outlines' group.
    s.edit("test", |d, _| {
        let n = d.node(NodeId(rect)).unwrap().clone();
        d.remove(NodeId(rect))?;
        d.insert(Some(NodeId(g)), 1, n)?;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["A", "B"]), "{r}");
    assert_eq!(r["unmatched"], json!([rect]));
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(children_of(&s, g), vec![ids[0], rect, ids[1]], "A under the rectangle, B over it");
}

#[test]
fn glyphs_of_different_groups_and_transparency_stay_apart() {
    let mut s = session();
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "ab", "size": 20}));
    let b = outlined(&mut s, json!({"x": 120, "y": 100, "text": "ed", "size": 20}));
    let first_d = children_of(&s, b)[1];
    s.edit("test", |d, _| {
        d.node_mut(NodeId(b)).unwrap().opacity = 0.5;
        d.node_mut(NodeId(first_d)).unwrap().opacity = 0.25;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["ab", "e", "d"]), "same baseline, other group; d has its own opacity: {r}");
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(a)).is_none(), "a plain group gives way to its type");
    assert_eq!(d.node(NodeId(b)).unwrap().opacity, 0.5, "the half-transparent group stays");
    assert_eq!(children_of(&s, b), vec![ids[1], ids[2]]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(ids[2])).unwrap().opacity, 0.25, "the glyph's own opacity carries over");
}

#[test]
fn live_type_between_glyphs_ends_the_line_too() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let live = s.execute("text.create", &json!({"x": 30, "y": 90, "text": "x", "size": 60})).unwrap()["id"].as_u64().unwrap();
    s.edit("test", |d, _| {
        let n = d.node(NodeId(live)).unwrap().clone();
        d.remove(NodeId(live))?;
        d.insert(Some(NodeId(g)), 1, n)?;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["A", "B"]), "{r}");
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(children_of(&s, g), vec![ids[0], live, ids[1]], "B stays over the live type");
    let t = text_of(&s, ids[0]);
    assert!(t.first_style().features.iter().any(|f| f == "-liga"), "glyphs drawn one by one: no ligatures");
}

#[test]
fn an_open_path_along_a_glyph_is_not_a_glyph() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "I", "size": 40}));
    s.edit("test", |d, _| {
        let glyph = d.node(NodeId(g)).unwrap().children().unwrap()[0].id;
        for sp in &mut d.node_mut(glyph).unwrap().path_data_mut().unwrap().subpaths {
            sp.closed = false;
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    assert!(s.execute("type.retype", &json!({})).is_err(), "nothing matches");
}

#[test]
fn live_type_selected_between_loose_glyphs_still_ends_the_line() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let live = s.execute("text.create", &json!({"x": 30, "y": 90, "text": "x", "size": 60})).unwrap()["id"].as_u64().unwrap();
    let (a, b) = (children_of(&s, g)[0], children_of(&s, g)[1]);
    // A, the live type and B as siblings in the layer, as Ungroup leaves them.
    s.edit("test", |d, _| {
        let (par, idx, _) = d.position(NodeId(g)).unwrap();
        let live_n = d.node(NodeId(live)).unwrap().clone();
        let (an, bn) = (d.node(NodeId(a)).unwrap().clone(), d.node(NodeId(b)).unwrap().clone());
        d.remove(NodeId(live))?;
        d.remove(NodeId(g))?;
        d.insert(par, idx, an)?;
        d.insert(par, idx + 1, live_n)?;
        d.insert(par, idx + 2, bn)?;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [a, live, b]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["A", "B"]), "the selected live type between them ends the line: {r}");
}

#[test]
fn glyphs_selected_inside_groups_join_only_their_siblings() {
    let mut s = session();
    let g1 = outlined(&mut s, json!({"x": 40, "y": 100, "text": "ab", "size": 20}));
    let g2 = outlined(&mut s, json!({"x": 70, "y": 100, "text": "ed", "size": 20}));
    let (a, b, c) = (children_of(&s, g1)[0], children_of(&s, g1)[1], children_of(&s, g2)[0]);
    s.execute("select.set", &json!({"ids": [a, b, c]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["ab", "e"]), "a and b are siblings; e is in another group: {r}");
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(g2)).is_some(), "d's group stays with c's type and d in it");
}

#[test]
fn tracked_type_splits_where_its_spacing_cannot_be_set_again() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "abe", "size": 30})).unwrap()["id"].as_u64().unwrap();
    // Less than half a space: no whole number of spaces makes up the gap (0.4 em would be two
    // spaces in this font, and the outlines cannot tell those apart from tracking).
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 1, "end": 3, "tracking": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
    assert_eq!(texts.concat(), "abe", "{r}");
    assert!(texts.len() > 1, "the tracked glyphs are set where the outlines are, not where untracked type puts them: {r}");
    assert_eq!(r["unmatched"], json!([]));
}

#[test]
fn a_masked_glyph_stays_an_outline() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let a = children_of(&s, g)[0];
    s.edit("test", |d, _| {
        let art = vectorcraft_doc::Node::path(d.alloc_id(), vectorcraft_geom::PathData::default(), Default::default());
        d.node_mut(NodeId(a)).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["B"]), "{r}");
    assert_eq!(r["unmatched"], json!([a]));
    assert!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().mask.is_some(), "A keeps its mask");
}

#[test]
fn shapes_past_the_limits_are_reported_as_left_out() {
    let mut s = session();
    let top = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    let deep = outlined(&mut s, json!({"x": 40, "y": 200, "text": "B", "size": 40}));
    // B, deeper than Retype looks.
    s.edit("test", |d, _| {
        let (par, idx, _) = d.position(NodeId(deep)).unwrap();
        let mut n = d.node(NodeId(deep)).unwrap().clone();
        d.remove(NodeId(deep))?;
        for _ in 0..20 {
            n = vectorcraft_doc::Node::group(d.alloc_id(), vec![std::sync::Arc::new(n)]);
        }
        let wrapper = n.id.0;
        d.insert(par, idx, n)?;
        Ok(wrapper)
    })
    .map(|w| s.execute("select.set", &json!({"ids": [top, w]})).unwrap())
    .unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["A"]));
    assert_eq!(r["truncated"], json!(true), "{r}");
}

#[test]
fn a_glyph_painted_beyond_what_type_takes_stays_an_outline() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let a = children_of(&s, g)[0];
    s.edit("test", |d, _| {
        let n = d.node_mut(NodeId(a)).unwrap();
        let mut ap = (*n.appearance).clone();
        let fill = ap.items.iter().find(|i| i.is_fill()).unwrap().clone();
        ap.items.push(fill);
        n.appearance = ap.into();
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["B"]), "two fills: A stays an outline: {r}");
    assert_eq!(r["unmatched"], json!([a]));
}

#[test]
fn a_hidden_or_locked_glyph_stays_an_outline() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "ABD", "size": 40}));
    let k = children_of(&s, g);
    s.edit("test", |d, _| {
        d.node_mut(NodeId(k[0])).unwrap().visible = false;
        d.node_mut(NodeId(k[2])).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["B"]), "{r}");
    let d = &s.doc().unwrap().doc;
    assert!(!d.node(NodeId(k[0])).unwrap().visible, "A stays a hidden outline");
    assert!(d.node(NodeId(k[2])).unwrap().locked, "D stays a locked outline");
}

#[test]
fn an_isolated_or_knockout_group_keeps_its_type_inside() {
    for case in ["isolate", "knockout", "knockout_shape"] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
        s.edit("test", |d, _| {
            let n = d.node_mut(NodeId(g)).unwrap();
            match case {
                "isolate" => n.isolate = true,
                "knockout" => n.knockout = vectorcraft_doc::Knockout::On,
                _ => n.knockout_shape = true,
            }
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({})).unwrap();
        let id = r["ids"][0].as_u64().unwrap();
        assert_eq!(children_of(&s, g), vec![id], "{case}: the group stays, holding the type");
    }
}

#[test]
fn type_turned_half_round_stays_one_object() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 300, "y": 200, "text": "Upside", "size": 30, "rotation": 180}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Upside"]), "{r}");
}

#[test]
fn a_drawn_dot_or_bar_beside_type_stays_art() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let dot = s.execute("shape.ellipse", &json!({"x": 300, "y": 60, "width": 8, "height": 8})).unwrap()["id"].as_u64().unwrap();
    // A bar that is exactly a hyphen of the font, somewhere else: the worst case.
    let bar = outlined(&mut s, json!({"x": 200, "y": 300, "text": "-", "size": 50}));
    s.execute("select.set", &json!({"ids": [g, dot, bar]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["AB"]), "{r}");
    let un = r["unmatched"].as_array().unwrap();
    assert!(un.contains(&json!(dot)) && un.len() == 2, "the dot and the bar stay art: {r}");
    // Selected on its own, the hyphen is what was asked for.
    s.execute("select.set", &json!({"ids": [bar]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["-"]), "{r}");
}

/// How long Retype takes with a large CJK face to match against (run with `--ignored
/// --nocapture`; skips when no such face is installed). Not a pass/fail budget: it prints.
#[test]
#[ignore = "timing: needs an installed CJK font"]
fn timing_with_a_large_cjk_face() {
    let db = vectorcraft_text::FontDb::global();
    let Some(family) = ["Microsoft YaHei", "Yu Gothic", "MS Gothic", "Noto Sans CJK JP", "Hiragino Sans"].into_iter().find(|f| db.has_family(f))
    else {
        eprintln!("skipped: no large CJK font installed");
        return;
    };
    let face = db.resolve(family, "Regular").map(|(f, _)| f).unwrap();
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 20, "y": 80, "text": "漢字のテスト ABC", "size": 24, "font": family, "style": face.style}));
    for run in ["cold", "warm"] {
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let t0 = std::time::Instant::now();
        let r = s.execute("type.retype", &json!({"fonts": [family]})).unwrap();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        eprintln!("{run}: Retype with {family} ({} encoded glyphs) loaded: {ms:.0} ms → {}", face.mapped_chars().len(), r["texts"]);
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn a_compound_with_a_locked_or_hidden_part_stays_whole() {
    let mut s = session();
    // "B" outlines as a compound path of three contours.
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    let b = children_of(&s, g)[1];
    let part = children_of(&s, b)[1];
    s.edit("test", |d, _| {
        d.node_mut(NodeId(part)).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["A"]), "{r}");
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(part)).unwrap().locked, "B's locked part, and B, stay as they are");
}

#[test]
fn a_group_with_an_attributes_note_keeps_its_type_inside() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB", "size": 40}));
    s.execute("attributes.set", &json!({"ids": [g], "note": "logo, keep"})).unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    let id = r["ids"][0].as_u64().unwrap();
    assert_eq!(children_of(&s, g), vec![id], "the group and its note stay, holding the type");
}

#[test]
fn nothing_retypable_reads_no_font() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    let a = children_of(&s, g)[0];
    s.edit("test", |d, _| {
        let art = vectorcraft_doc::Node::path(d.alloc_id(), vectorcraft_geom::PathData::default(), Default::default());
        d.node_mut(NodeId(a)).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let e = s.execute("type.retype", &json!({})).unwrap_err().to_string();
    assert!(e.contains("(1 checked)"), "the masked shape is reported, and no font was indexed: {e}");
}

#[test]
fn tracked_one_contour_letters_still_become_type() {
    for text in ["LIT", "HELLO", "LIST"] {
        let mut s = session();
        let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": text, "size": 30})).unwrap()["id"].as_u64().unwrap();
        let n = text.len() as u64;
        s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": n, "tracking": 50})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({})).unwrap();
        assert_eq!(r["texts"], json!([text]), "evenly tracked, the line is one object: {r}");
        assert_eq!(r["fonts"][0]["tracking"], json!(50.0), "{r}");
        assert_eq!(r["unmatched"], json!([]), "{r}");
        assert_eq!(text_of(&s, r["ids"][0].as_u64().unwrap()).first_style().tracking, 50.0);
    }
}

/// How long Retype takes on a few hundred CJK outlines at once (run with `--ignored --nocapture`;
/// skips when no large CJK font is installed). Not a pass/fail budget: it prints.
#[test]
#[ignore = "timing: needs an installed CJK font"]
fn timing_with_hundreds_of_cjk_outlines() {
    let db = vectorcraft_text::FontDb::global();
    let Some(family) = ["Microsoft YaHei", "Yu Gothic", "MS Gothic", "Noto Sans CJK JP", "Hiragino Sans"].into_iter().find(|f| db.has_family(f))
    else {
        eprintln!("skipped: no large CJK font installed");
        return;
    };
    let face = db.resolve(family, "Regular").map(|(f, _)| f).unwrap();
    // 300 common ideographs, ten lines of thirty.
    let chars: Vec<char> = (0x4E00u32..).filter_map(char::from_u32).filter(|c| face.covers(*c)).take(300).collect();
    let mut s = session();
    let mut groups = vec![];
    for (i, line) in chars.chunks(30).enumerate() {
        let text: String = line.iter().collect();
        groups.push(outlined(&mut s, json!({"x": 20, "y": 40 + 30 * i, "text": text, "size": 20, "font": family, "style": face.style})));
    }
    for run in ["cold", "warm"] {
        s.execute("select.set", &json!({"ids": groups})).unwrap();
        let t0 = std::time::Instant::now();
        let r = s.execute("type.retype", &json!({"fonts": [family]})).unwrap();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let typed: usize = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().chars().count()).sum();
        eprintln!(
            "{run}: Retype of {} {family} outlines: {ms:.0} ms → {typed} characters typed, {} unmatched",
            chars.len(),
            r["unmatched"].as_array().unwrap().len()
        );
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn a_long_tracked_line_is_checked_in_time() {
    // Even tracking is kept on the line; the layout check of a long line must not lay out the rest
    // of the line again after each window.
    let text = "abe".repeat(150);
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 100, "text": text, "size": 4})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": text.len(), "tracking": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let t0 = std::time::Instant::now();
    let r = s.execute("type.retype", &json!({})).unwrap();
    let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
    assert_eq!(texts, vec![text], "the whole line is one object: {r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(50.0), "{r}");
    assert_eq!(r["unmatched"], json!([]));
    eprintln!("450 tracked glyphs: {:?}, {} objects", t0.elapsed(), texts.len());
}

/// Every path in the document flattened into lines `tol` from its curves, each contour started at
/// its second point and run backwards: outlines simplified and built nothing like the font's.
fn simplify_every_path(s: &mut Session, tol: f64) {
    let mut ids = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            ids.push(n.id);
        }
    });
    simplify_paths(s, &ids, tol);
}

/// [`simplify_every_path`] for the paths `ids` only.
fn simplify_paths(s: &mut Session, ids: &[NodeId], tol: f64) {
    s.edit("test", |d, _| {
        for id in ids {
            let data = d.node_mut(*id).unwrap().path_data_mut().unwrap();
            let mut contours: Vec<Vec<kurbo::Point>> = vec![];
            kurbo::flatten(data.to_bezpath().iter(), tol, |el| match el {
                kurbo::PathEl::MoveTo(q) => contours.push(vec![q]),
                kurbo::PathEl::LineTo(q) => contours.last_mut().unwrap().push(q),
                _ => {}
            });
            let mut out = kurbo::BezPath::new();
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
            *data = vectorcraft_geom::PathData::from_bezpath(&out);
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn simplified_outlines_retype_only_with_a_tolerance_and_report_their_score() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Rag", "size": 40, "font": "Source Sans 3", "style": "Regular"}));
    simplify_every_path(&mut s, 0.05);
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = s.execute("type.retype", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no selected shape matches"), "no exact match: {e}");
    for bad in [json!(-0.01), json!(0.06), json!("0.02"), json!(true), json!(null)] {
        let e = s.execute("type.retype", &json!({"tolerance": bad})).unwrap_err().to_string();
        assert!(e.contains("tolerance must be a number from 0 to 0.05"), "{e}");
    }
    assert!(s.doc().unwrap().doc == before, "nothing changes");
    let r = s.execute("type.retype", &json!({"tolerance": 0.02})).unwrap();
    assert_eq!(r["texts"], json!(["Rag"]), "{r}");
    assert_eq!(r["unmatched"], json!([]));
    assert_eq!((r["fonts"][0]["family"].as_str(), r["fonts"][0]["style"].as_str()), (Some("Source Sans 3"), Some("Regular")));
    let score = r["fonts"][0]["score"].as_f64().unwrap();
    assert!(score > 0.5 && score < 1.0, "a near match scores below an exact one: {score}");
    let t = text_of(&s, r["ids"][0].as_u64().unwrap());
    assert!(near(t.first_style().size, 40.0, 0.4), "size {}", t.first_style().size);
    let o = t.xf.translation();
    assert!(near(o.x, 40.0, 0.4) && near(o.y, 100.0, 0.4), "baseline start {o:?}");
}

#[test]
fn exact_matches_score_one() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Rag", "size": 40}));
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({"tolerance": 0.02})).unwrap();
    assert_eq!(r["texts"], json!(["Rag"]), "{r}");
    assert_eq!(r["fonts"][0]["score"], json!(1.0));
}

#[test]
fn exact_matches_with_a_small_leftover_still_score_one() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Rag", "size": 40}));
    // Each glyph stretched 0.1% across its own centre: still within the exact matcher's reach, with
    // a leftover distance above 0.
    let mut ids = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::Path { .. }) {
            ids.push(n.id);
        }
    });
    s.edit("test", |d, _| {
        for id in &ids {
            let data = d.node_mut(*id).unwrap().path_data_mut().unwrap();
            let p = data.to_bezpath();
            let c = kurbo::Shape::bounding_box(&p).center();
            let xf = kurbo::Affine::translate(c.to_vec2()) * kurbo::Affine::scale_non_uniform(1.001, 1.0) * kurbo::Affine::translate(-c.to_vec2());
            *data = vectorcraft_geom::PathData::from_bezpath(&(xf * p));
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    // Each glyph's size now differs a little, so each is its own line.
    let texts: Vec<&str> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert_eq!(texts.concat(), "Rag", "{r}");
    for f in r["fonts"].as_array().unwrap() {
        assert_eq!(f["score"], json!(1.0), "{r}");
    }
}

#[test]
fn two_overlapping_lone_glyphs_beside_art_stay_outlines() {
    // Two M outlines start 0.3 em apart, closer than the first one's advance: a tight-tracking
    // line of two, which the layout check splits again. Each is still a lone one-contour glyph.
    let mut s = session();
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "M", "size": 30}));
    let b = outlined(&mut s, json!({"x": 49, "y": 100, "text": "M", "size": 30}));
    let dot = s.execute("shape.ellipse", &json!({"x": 300, "y": 60, "width": 8, "height": 8})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b, dot]})).unwrap();
    // Nothing is left to retype: both stay outlines, as the lone bar beside art does.
    let e = s.execute("type.retype", &json!({})).unwrap_err();
    assert!(format!("{e:?}").contains("no selected shape matches"), "{e:?}");
}

/// Where type `t` draws each glyph that has ink, in document space.
fn glyph_places(t: &TextObject) -> Vec<kurbo::Point> {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| t.xf * (g.xf * kurbo::Point::ORIGIN)).collect()
}

/// Type made with `p` (and `setup` run on its id), outlined and retyped: the original type, and
/// Retype's result.
fn retyped(s: &mut Session, p: serde_json::Value, setup: &[serde_json::Value]) -> (TextObject, serde_json::Value) {
    let id = s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap();
    for q in setup {
        let mut q = q.clone();
        q["id"] = json!(id);
        s.execute("text.setRangeStyle", &q).unwrap();
    }
    let before = text_of(s, id);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    (before, s.execute("type.retype", &json!({})).unwrap())
}

fn assert_same_places(a: &TextObject, b: &TextObject) {
    let (pa, pb) = (glyph_places(a), glyph_places(b));
    assert_eq!(pa.len(), pb.len());
    for (x, y) in pa.iter().zip(&pb) {
        assert!(x.distance(*y) < 0.05, "{x:?} vs {y:?}");
    }
}

#[test]
fn exact_glyphs_of_another_size_are_another_line_even_with_a_tolerance() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tia", "size": 40}));
    // The a (two contours, so not a lone one-contour glyph) made 4% larger about where it starts
    // on the baseline: still exactly the glyph.
    let a = *children_of(&s, g).last().unwrap();
    // The a's paths (a compound path's, or its own).
    let paths: Vec<u64> = match s.doc().unwrap().doc.node(NodeId(a)).unwrap().children() {
        Some(c) => c.iter().map(|c| c.id.0).collect(),
        None => vec![a],
    };
    s.edit("test", |d, _| {
        let x0 = paths
            .iter()
            .map(|id| kurbo::Shape::bounding_box(&d.node_mut(NodeId(*id)).unwrap().path_data_mut().unwrap().to_bezpath()).x0)
            .fold(f64::INFINITY, f64::min);
        let at = kurbo::Vec2::new(x0, 100.0);
        let xf = kurbo::Affine::translate(at) * kurbo::Affine::scale(1.04) * kurbo::Affine::translate(-at);
        for id in &paths {
            let data = d.node_mut(NodeId(*id)).unwrap().path_data_mut().unwrap();
            *data = vectorcraft_geom::PathData::from_bezpath(&(xf * data.to_bezpath()));
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({"tolerance": 0.05})).unwrap();
    assert_eq!(r["texts"], json!(["Ti", "a"]), "{r}");
    let sizes: Vec<f64> = r["fonts"].as_array().unwrap().iter().map(|f| f["size"].as_f64().unwrap()).collect();
    assert!(near(sizes[0], 40.0, 0.01) && near(sizes[1], 41.6, 0.01), "{sizes:?}");
    assert!(r["fonts"].as_array().unwrap().iter().all(|f| f["score"] == json!(1.0)), "{r}");
}

#[test]
fn turned_near_matched_round_glyphs_keep_the_line_turn() {
    // Round glyphs (o, O, the period) and glyphs that look alike half round (N, x, z) fit
    // simplified outlines at more than one turn: each takes the turn of its line.
    for (text, rotation) in [("no.", 90), ("no.", 180), ("zoo", 135), ("NOON", 180), ("xoxo", 200), ("on", 120)] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 300, "y": 200, "text": text, "size": 40, "rotation": rotation}));
        simplify_every_path(&mut s, 0.05);
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({"tolerance": 0.02})).unwrap();
        assert_eq!(r["texts"], json!([text]), "turned {rotation}: {r}");
        assert_eq!(r["unmatched"], json!([]), "{r}");
    }
}

#[test]
fn several_lines_become_one_area_type_object() {
    let mut s = session();
    let text = "First line\nSecond one\nThird";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 24}), &[]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["area"], json!(true), "{r}");
    assert_eq!(r["fonts"][0]["leading"], json!(null), "Auto leading gives the lines' spacing: {r}");
    let t = text_of(&s, r["ids"][0].as_u64().unwrap());
    assert!(matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }));
    assert_eq!(t.plain_text(), text);
    assert_same_places(&before, &t);
}

#[test]
fn lines_a_quarter_of_the_size_apart_join_and_closer_ones_do_not() {
    // At 20 pt, 5 pt is the least leading that joins lines.
    // 4.9999999 pt is 5 pt to the rounding of the outlines' positions.
    for (leading, objects) in [(5.0, 1), (4.9999999, 1), (4.9, 2)] {
        let mut s = session();
        let text = "One\nTwo";
        let (_, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 20}), &[json!({"leading": leading})]);
        assert_eq!(r["texts"].as_array().unwrap().len(), objects, "{leading}: {r}");
    }
}

#[test]
fn a_two_glyph_line_takes_the_tracking_of_the_lines_around_it() {
    let mut s = session();
    let text = "MMM\nMM\nMMM";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 24}), &[json!({"tracking": 50})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(50.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn lines_keep_their_leading_and_alignment() {
    for justify in ["left", "center", "right"] {
        let mut s = session();
        let text = "Wide first line\nmid\nlast one";
        let (before, r) = retyped(&mut s, json!({"x": 300, "y": 100, "text": text, "size": 20, "justify": justify}), &[json!({"leading": 31})]);
        assert_eq!(r["texts"], json!([text]), "{justify}: {r}");
        assert!(near(r["fonts"][0]["leading"].as_f64().unwrap(), 31.0, 1e-3), "{justify}: {r}");
        assert_eq!(r["fonts"][0]["area"], json!(true), "{justify}: {r}");
        let t = text_of(&s, r["ids"][0].as_u64().unwrap());
        assert!(matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }), "{justify}");
        assert_eq!(t.para.justify, before.para.justify, "{justify}");
        assert_same_places(&before, &t);
    }
}

#[test]
fn a_long_block_near_auto_leading_keeps_its_own_leading() {
    // 24.001 pt is Auto's 24 pt within a ten-thousandth of the size per line, but 250 lines
    // move the last one a quarter of a point.
    let mut s = session();
    let text = vec!["ab"; 250].join("\n");
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 40, "text": text, "size": 20}), &[json!({"leading": 24.001})]);
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"].as_array().map_or(0, |f| f.len()));
    assert!(near(r["fonts"][0]["leading"].as_f64().unwrap(), 24.001, 1e-4), "{}", r["fonts"][0]);
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_long_block_at_auto_leading_of_a_fractional_size_keeps_auto() {
    // Auto leading of 20.002 pt is 24.0024 pt, finer than the thousandth of a point the leading
    // between two lines is read to.
    let mut s = session();
    // Six hundred lines: read to a thousandth, each gap would be 0.0004 pt short, and the last
    // line 0.24 pt off.
    let text = vec!["ab"; 600].join("\n");
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 40, "text": text, "size": 20.002}), &[]);
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"].as_array().map_or(0, |f| f.len()));
    assert_eq!(r["fonts"][0]["leading"], json!(null), "{}", r["fonts"][0]);
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_long_block_at_a_leading_finer_than_a_thousandth_keeps_it() {
    // 23.9996 pt read to a thousandth is 24 pt, Auto for 20 pt type: over 600 lines the last one
    // would be 0.24 pt off.
    let mut s = session();
    let text = vec!["ab"; 600].join("\n");
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 40, "text": text, "size": 20}), &[json!({"leading": 23.9996})]);
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"].as_array().map_or(0, |f| f.len()));
    assert!(near(r["fonts"][0]["leading"].as_f64().unwrap(), 23.9996, 1e-6), "{}", r["fonts"][0]);
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_block_whose_first_gap_is_a_little_off_stays_one_object() {
    // The second line moved up 0.004 pt: its gap read alone, carried over 100 lines, would put
    // the last one 0.4 pt off.
    let mut s = session();
    let text = ["ab"; 100].join("\n");
    let id = s.execute("text.create", &json!({"x": 40, "y": 40, "text": text, "size": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "leading": 24.004})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let kids = children_of(&s, g);
    s.execute("select.set", &json!({"ids": kids[2..4]})).unwrap();
    s.execute("object.move", &json!({"dx": 0, "dy": -0.004})).unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"].as_array().map_or(0, |f| f.len()));
    assert!(near(r["fonts"][0]["leading"].as_f64().unwrap(), 24.004, 1e-3), "{}", r["fonts"][0]);
}

#[test]
fn lines_whose_frame_would_pass_the_coordinate_limit_stay_point_type() {
    let mut s = session();
    let text = "Wide line\nWide";
    let id = s.execute("text.create", &json!({"x": 3_999_000, "y": 100, "text": text, "size": 20})).unwrap()["id"].as_u64().unwrap();
    let right = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap().x1;
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    // The ink ends 2 pt short of the limit; Convert To Area Type's slack would reach past it.
    s.execute("object.move", &json!({"dx": vectorcraft_doc::MAX_COORD - 2.0 - right, "dy": 0})).unwrap();
    let before = text_of(&s, id);
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["area"], json!(false), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_line_type_cannot_join_ends_the_block_there() {
    // The fourth line keeps a fraction of a unit of tracking that the others do not: joined, it
    // moves, so the lines before it stay one object and the lines after it join again.
    let mut s = session();
    let line = "M".repeat(40);
    let text = [line.as_str(); 6].join("\n");
    let start = 3 * 41;
    let (before, r) =
        retyped(&mut s, json!({"x": 20, "y": 40, "text": text, "size": 20}), &[json!({"start": start, "end": start + 40, "tracking": 0.6})]);
    let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
    assert_eq!(texts.len(), 3, "{texts:?} {}", r["fonts"]);
    assert_eq!(texts.join("\n"), text);
    let places: Vec<kurbo::Point> = r["ids"].as_array().unwrap().iter().flat_map(|id| glyph_places(&text_of(&s, id.as_u64().unwrap()))).collect();
    let want = glyph_places(&before);
    assert_eq!(places.len(), want.len());
    for (x, y) in places.iter().zip(&want) {
        assert!(x.distance(*y) < 0.05, "{x:?} vs {y:?}");
    }
}

/// The glyphs of the outline groups `others` moved to the end of group `g`, in order, and the
/// emptied groups removed.
fn into_one_group(s: &mut Session, g: u64, others: &[u64]) {
    s.edit("test", |d, _| {
        for o in others {
            let kids: Vec<_> = d.node(NodeId(*o)).unwrap().children().unwrap().to_vec();
            d.remove(NodeId(*o))?;
            for k in kids {
                let at = d.node(NodeId(g)).unwrap().children().unwrap().len();
                d.insert(Some(NodeId(g)), at, (*k).clone())?;
            }
        }
        Ok(())
    })
    .unwrap();
}

#[test]
fn lines_that_do_not_stack_as_a_paragraph_stay_apart() {
    let mut s = session();
    // Far apart, then lined up at nothing.
    let a = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Top", "size": 20}));
    let b = outlined(&mut s, json!({"x": 40, "y": 300, "text": "Far", "size": 20}));
    let c = outlined(&mut s, json!({"x": 75, "y": 324, "text": "Askew", "size": 20}));
    into_one_group(&mut s, a, &[b, c]);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Top", "Far", "Askew"]), "{r}");
    assert!(r["fonts"].as_array().unwrap().iter().all(|f| f["area"] == json!(false)), "{r}");
}

#[test]
fn a_shape_between_lines_keeps_them_apart() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "AB\nCD", "size": 30}));
    let rect = s.execute("shape.rectangle", &json!({"x": 30, "y": 60, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
    s.edit("test", |d, _| {
        let n = d.node(NodeId(rect)).unwrap().clone();
        d.remove(NodeId(rect))?;
        d.insert(Some(NodeId(g)), 2, n)?;
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["AB", "CD"]), "the rectangle is painted between the lines: {r}");
}

#[test]
fn tracked_lines_keep_their_tracking_and_spaces() {
    let mut s = session();
    let text = "SPACED OUT\nTITLE";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 24}), &[json!({"tracking": 400})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(400.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_short_line_takes_the_tracking_of_its_block() {
    // One glyph has no gap to measure tracking by: it joins the tracked lines.
    for text in ["TITLE\nB", "B\nTITLE", "TITLE\nB\nTITLE"] {
        let mut s = session();
        let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 24}), &[json!({"tracking": 400})]);
        assert_eq!(r["texts"], json!([text]), "{r}");
        assert_eq!(r["fonts"][0]["tracking"], json!(400.0), "{r}");
        assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    }
}

#[test]
fn unevenly_tracked_type_still_becomes_type() {
    let mut s = session();
    // Less than half a space more after e: no gap can be read as a word space.
    let text = "abcdefgh";
    let (before, r) = retyped(
        &mut s,
        json!({"x": 40, "y": 100, "text": text, "size": 30}),
        &[json!({"start": 0, "end": 4, "tracking": 50}), json!({"start": 4, "end": 8, "tracking": 120})],
    );
    let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
    assert_eq!(texts.concat(), text, "{r}");
    assert!(texts.len() > 1, "{r}");
    assert_eq!(r["unmatched"], json!([]), "{r}");
    // Each object's glyphs are where the outlines were.
    let places: Vec<kurbo::Point> = r["ids"].as_array().unwrap().iter().flat_map(|id| glyph_places(&text_of(&s, id.as_u64().unwrap()))).collect();
    let want = glyph_places(&before);
    assert_eq!(places.len(), want.len());
    for (x, y) in places.iter().zip(&want) {
        assert!(x.distance(*y) < 0.05, "{x:?} vs {y:?}");
    }
    // The second half keeps its own tracking.
    assert_eq!(r["fonts"].as_array().unwrap().last().unwrap()["tracking"], json!(120.0), "{r}");
}

#[test]
fn a_tracked_line_keeps_its_tracking_and_spaces() {
    let mut s = session();
    let text = "SPACED OUT";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 24}), &[json!({"tracking": 400})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(400.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_long_line_tracked_two_spaces_wide_keeps_its_tracking() {
    // Read as it is, the line holds two spaces in each gap, which the tracking measure lays out too.
    let mut s = session();
    let text = "M".repeat(400);
    let (_, r) = retyped(&mut s, json!({"x": 10, "y": 100, "text": text, "size": 10}), &[json!({"tracking": 400})]);
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"]);
    assert_eq!(r["fonts"][0]["tracking"], json!(400.0), "{}", r["fonts"]);
}

#[test]
fn a_long_tightly_tracked_line_keeps_its_word_spaces() {
    // Read as it is, the line holds no spaces; the tracked reading holds eight in each word gap.
    let mut s = session();
    let text = vec!["MMM"; 100].join("        ");
    let (_, r) = retyped(&mut s, json!({"x": 10, "y": 100, "text": text, "size": 10}), &[json!({"tracking": -180})]);
    assert_eq!(r["texts"], json!([text]), "{}", r["fonts"]);
    assert_eq!(r["fonts"][0]["tracking"], json!(-180.0), "{}", r["fonts"]);
}

#[test]
fn spaced_letters_keep_their_spaces_when_type_draws_them() {
    // Most gaps are word spaces: untracked type with the spaces draws every glyph in place.
    // Every gap one word space: tracking a space wide would draw them too, without the spaces.
    // L T J: without its spaces the pairs kern, which a measure of the gaps would read as more space.
    for text in ["a a a aa", "1 2 3", "a b c d", "x = y + 1", "L T J"] {
        let mut s = session();
        let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[]);
        assert_eq!(r["texts"], json!([text]), "{r}");
        assert_eq!(r["fonts"][0]["tracking"], json!(0.0), "{r}");
        assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    }
}

#[test]
fn a_near_glyph_moved_by_more_than_the_tolerance_ends_the_line() {
    // The m (one path) of simplified outlines moved along the baseline by `shift` of the font
    // size (40), retyped with a tolerance of 0.03: it may sit 1% + 3% of the size from where the
    // type draws it.
    let run = |shift: f64| {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tim", "size": 40}));
        simplify_every_path(&mut s, 0.05);
        let m = *children_of(&s, g).last().unwrap();
        s.edit("test", |d, _| {
            let data = d.node_mut(NodeId(m)).unwrap().path_data_mut().unwrap();
            *data = vectorcraft_geom::PathData::from_bezpath(&(kurbo::Affine::translate((40.0 * shift, 0.0)) * data.to_bezpath()));
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({"tolerance": 0.03})).unwrap();
        let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
        assert_eq!(texts.concat(), "Tim", "{r}");
        texts.len()
    };
    assert_eq!(run(0.02), 1, "within the tolerance: one line");
    assert!(run(0.06) > 1, "past it: the line splits where the m was moved");
}

/// The bounding box of shape `id` (a path, or a compound path's paths), in document space.
fn shape_box(s: &Session, id: u64) -> kurbo::Rect {
    let d = &s.doc().unwrap().doc;
    let n = d.node(NodeId(id)).unwrap();
    let paths: Vec<&vectorcraft_doc::Node> = match n.children() {
        Some(c) => c.iter().map(|c| &**c).collect(),
        None => vec![n],
    };
    paths
        .iter()
        .filter_map(|p| match &p.kind {
            NodeKind::Path { path, .. } => Some(kurbo::Shape::bounding_box(&path.to_bezpath())),
            _ => None,
        })
        .reduce(|a, b| a.union(b))
        .unwrap()
}

/// The bounding box of each glyph with ink that type `t` draws, in document space.
fn drawn_boxes(t: &TextObject) -> Vec<kurbo::Rect> {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).map(|g| kurbo::Shape::bounding_box(&(t.xf * g.outline.clone()))).collect()
}

#[test]
fn a_near_glyph_whose_size_and_turn_differ_past_the_tolerance_ends_the_line() {
    // The m of simplified outlines made 4% larger, and turned 0.04 radians as well, about where it
    // starts on the baseline, retyped with a tolerance of 0.05. Each change alone is within the
    // size and turn a near-matched line allows, but drawn at the line's size and turn the m would
    // lie further than 0.05 of its spread from its outline: it becomes type of its own. Larger
    // alone, it is still within the tolerance and stays on the line.
    for (scale, turn, lines) in [(1.04, 0.0, 1), (1.04, 0.04, 2)] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tim", "size": 40}));
        simplify_every_path(&mut s, 0.05);
        let kids = children_of(&s, g);
        let m = *kids.last().unwrap();
        let x0 = shape_box(&s, m).x0;
        s.edit("test", |d, _| {
            let at = kurbo::Vec2::new(x0, 100.0);
            let xf = kurbo::Affine::translate(at) * kurbo::Affine::rotate(turn) * kurbo::Affine::scale(scale) * kurbo::Affine::translate(-at);
            let data = d.node_mut(NodeId(m)).unwrap().path_data_mut().unwrap();
            *data = vectorcraft_geom::PathData::from_bezpath(&(xf * data.to_bezpath()));
            Ok(())
        })
        .unwrap();
        let before: Vec<kurbo::Rect> = kids.iter().map(|k| shape_box(&s, *k)).collect();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({"tolerance": 0.05})).unwrap();
        let texts: Vec<&str> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert_eq!(texts.concat(), "Tim", "{r}");
        assert_eq!(texts.len(), lines, "scale {scale}, turn {turn}: {r}");
        if lines == 1 {
            continue;
        }
        let drawn: Vec<kurbo::Rect> = r["ids"].as_array().unwrap().iter().flat_map(|id| drawn_boxes(&text_of(&s, id.as_u64().unwrap()))).collect();
        assert_eq!(drawn.len(), before.len(), "{r}");
        for b in &before {
            // The glyph drawn nearest where the outline was.
            let d = drawn.iter().min_by(|p, q| p.center().distance(b.center()).total_cmp(&q.center().distance(b.center()))).unwrap();
            let size = b.width().hypot(b.height());
            assert!(
                (d.width() - b.width()).abs() < size * 0.01 && (d.height() - b.height()).abs() < size * 0.01,
                "scale {scale}, turn {turn}: the outline's box {b:?}, the type's {d:?}: {r}"
            );
        }
    }
}

#[test]
fn an_exact_glyph_moved_splits_the_line_whatever_the_tolerance() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tim", "size": 40}));
    let m = *children_of(&s, g).last().unwrap();
    s.edit("test", |d, _| {
        let data = d.node_mut(NodeId(m)).unwrap().path_data_mut().unwrap();
        *data = vectorcraft_geom::PathData::from_bezpath(&(kurbo::Affine::translate((40.0 * 0.03, 0.0)) * data.to_bezpath()));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({"tolerance": 0.05})).unwrap();
    assert_eq!(r["texts"], json!(["Ti", "m"]), "exact glyphs keep the exact limits: {r}");
}

#[test]
fn an_exact_glyph_after_a_near_one_keeps_the_exact_limits() {
    // T exact, o simplified (a near match), m exact moved by 3% of the size along the baseline.
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tom", "size": 40}));
    let kids = children_of(&s, g);
    let o: Vec<NodeId> = match s.doc().unwrap().doc.node(NodeId(kids[1])).unwrap().children() {
        Some(c) => c.iter().map(|c| c.id).collect(),
        None => vec![NodeId(kids[1])],
    };
    simplify_paths(&mut s, &o, 0.05);
    let m = kids[2];
    s.edit("test", |d, _| {
        let data = d.node_mut(NodeId(m)).unwrap().path_data_mut().unwrap();
        *data = vectorcraft_geom::PathData::from_bezpath(&(kurbo::Affine::translate((40.0 * 0.03, 0.0)) * data.to_bezpath()));
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({"tolerance": 0.05})).unwrap();
    assert_eq!(r["texts"], json!(["To", "m"]), "the near o does not loosen the m's limits: {r}");
}

/// The outlines of `text` made into one compound path: the compound's id.
fn one_compound(s: &mut Session, text: &str) -> u64 {
    let g = outlined(s, json!({"x": 40, "y": 100, "text": text, "size": 40}));
    let glyphs = children_of(s, g);
    s.execute("select.set", &json!({"ids": glyphs})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn a_word_outlined_as_one_compound_path_splits_into_its_glyphs() {
    let mut s = session();
    let c = one_compound(&mut s, "Tim");
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["Tim"]), "the i's stem and dot are one glyph: {r}");
    assert_eq!(r["unmatched"], json!([]));
    assert!(s.doc().unwrap().doc.node(NodeId(c)).is_none(), "the compound becomes the type");
}

#[test]
fn a_lone_period_in_a_compound_selected_alone_stays_an_outline() {
    let mut s = session();
    // An A and, far below it, a period, made into one compound path and selected alone: the
    // period, alone on its baseline beside the A, is a dot as much as a period, so it stays an
    // outline, and so does the A in the same compound.
    let t = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    let dot = outlined(&mut s, json!({"x": 40, "y": 300, "text": ".", "size": 40}));
    let mut ids = children_of(&s, t);
    ids.extend(children_of(&s, dot));
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = match s.execute("type.retype", &json!({})) {
        Ok(r) => panic!("the period became type: {r}"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains("(1 checked)"), "{e}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
}

#[test]
fn lines_from_one_compound_path_keep_their_order() {
    let mut s = session();
    // Two lines of outlines made into one compound path, farther apart than the lines of one
    // paragraph, so each stays an object of its own.
    let top = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Tim", "size": 40}));
    let low = outlined(&mut s, json!({"x": 40, "y": 300, "text": "Ham", "size": 40}));
    let mut glyphs = children_of(&s, top);
    glyphs.extend(children_of(&s, low));
    s.execute("select.set", &json!({"ids": glyphs})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!(ids.len(), 2, "{r}");
    let texts: Vec<String> = ids.iter().map(|id| text_of(&s, *id).plain_text()).collect();
    let d = &s.doc().unwrap().doc;
    let at: Vec<usize> = ids.iter().map(|id| d.position(NodeId(*id)).unwrap().1).collect();
    // The lines stack in the order of the compound's contours: the first line behind the second.
    let first = texts.iter().position(|t| t == "Tim").unwrap();
    let second = texts.iter().position(|t| t == "Ham").unwrap();
    assert!(at[first] < at[second], "{texts:?} at {at:?}");
}

#[test]
fn stacked_lines_from_one_compound_path_become_one_object() {
    let mut s = session();
    // Two lines of one paragraph, outlined and made into one compound path: they join as the
    // lines of separate outlines do.
    let text = "Tim\nHam";
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 40}));
    let glyphs = children_of(&s, g);
    s.execute("select.set", &json!({"ids": glyphs})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["unmatched"], json!([]), "{r}");
    assert!(s.doc().unwrap().doc.node(NodeId(c)).is_none(), "the compound becomes the type");
}

#[test]
fn a_compound_whose_parts_are_not_all_glyphs_stays_whole() {
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "Ti", "size": 40}));
    let mut ids = children_of(&s, g);
    // A shape no glyph is, beside the letters, in the same compound.
    ids.push(s.execute("shape.star", &json!({"cx": 140, "cy": 85, "radius1": 12, "radius2": 5, "points": 7})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = s.execute("type.retype", &json!({})).unwrap_err().to_string();
    assert!(e.contains("(1 checked)"), "reported once: {e}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
}

#[test]
fn glyphs_that_overlap_in_an_even_odd_compound_stay_an_outline() {
    let mut s = session();
    // Two o's moved onto each other and made into one compound path: the even-odd rule leaves
    // the overlap unpainted, and two glyphs set side by side would paint it.
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "oo", "size": 40}));
    let kids = children_of(&s, g);
    s.execute("object.move", &json!({"ids": [kids[1]], "dx": -9, "dy": 0})).unwrap();
    s.execute("select.set", &json!({"ids": kids})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = match s.execute("type.retype", &json!({})) {
        Ok(r) => panic!("the overlapping o's became type: {r}"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains("(1 checked)"), "{e}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
}

#[test]
fn a_translucent_compound_whose_glyphs_overlap_stays_an_outline() {
    // Two i's, the second moved half onto the first and down off its baseline, made into one
    // compound path filled by the nonzero rule with every contour running one way: the parts paint
    // the shape's area, but on two lines they become two objects, and at 50% opacity their
    // overlap would be painted twice.
    for opacity in [1.0f32, 0.5] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "ii", "size": 40}));
        let kids = children_of(&s, g);
        let dx = shape_box(&s, kids[0]).x0 - shape_box(&s, kids[1]).x0 + 2.0;
        s.execute("object.move", &json!({"ids": [kids[1]], "dx": dx, "dy": 6})).unwrap();
        s.execute("select.set", &json!({"ids": kids})).unwrap();
        let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        let paths = children_of(&s, c);
        s.edit("test", |d, _| {
            // Make Compound Path turns the second glyph's contours the other way round.
            for p in &paths {
                let data = d.node_mut(NodeId(*p)).unwrap().path_data_mut().unwrap();
                let b = data.to_bezpath();
                if kurbo::Shape::area(&b) < 0.0 {
                    *data = vectorcraft_geom::PathData::from_bezpath(&b.reverse_subpaths());
                }
            }
            let n = d.node_mut(NodeId(c)).unwrap();
            n.opacity = opacity;
            if let NodeKind::Compound { rule, .. } = &mut n.kind {
                *rule = vectorcraft_geom::FillRule::NonZero;
            }
            Ok(())
        })
        .unwrap();
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        let r = s.execute("type.retype", &json!({}));
        if opacity == 1.0 {
            let r = r.unwrap();
            assert_eq!(r["texts"], json!(["i", "i"]), "opaque, the two lines become type: {r}");
        } else {
            assert!(r.is_err(), "translucent, the compound stays an outline: {r:?}");
        }
    }
}

#[test]
fn overlapping_exact_glyphs_stay_an_outline_when_near_matches_are_allowed() {
    // Exact parts the even-odd rule rejects stay rejected when the near phase finds them again.
    for dx in [-9.0, -11.0, -12.0, -13.0, -14.0] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "oo", "size": 40}));
        let kids = children_of(&s, g);
        s.execute("object.move", &json!({"ids": [kids[1]], "dx": dx, "dy": 0})).unwrap();
        s.execute("select.set", &json!({"ids": kids})).unwrap();
        let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        for tolerance in [0.0, 0.05] {
            if let Ok(r) = s.execute("type.retype", &json!({"tolerance": tolerance})) {
                panic!("the overlapping o's became type at dx {dx}, tolerance {tolerance}: {r}");
            }
        }
    }
}

#[test]
fn exact_parts_of_a_compound_are_checked_at_tolerance_zero() {
    let mut s = session();
    // A T and an o whose boxes overlap by 4 units, one painted area apart (the o sits under the
    // bar's arm): a strip that the probes of tolerance 0.05 cannot see into but those of an exact
    // match can. Both parts match exactly, so asking for near matches does not change the result.
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "To", "size": 40}));
    let kids = children_of(&s, g);
    let (t, o) = (kids[0], kids[1]);
    let bounds = |s: &Session, id: u64| s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
    let dx = bounds(&s, t).x1 - 4.0 - bounds(&s, o).x0;
    s.execute("object.move", &json!({"ids": [o], "dx": dx, "dy": 0})).unwrap();
    s.execute("select.set", &json!({"ids": kids})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    for params in [json!({}), json!({"tolerance": 0.05})] {
        s.execute("select.set", &json!({"ids": [c]})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let r = s.execute("type.retype", &params).unwrap();
        assert_eq!(r["unmatched"], json!([]), "{params}: {r}");
        assert!(s.doc().unwrap().doc.node(NodeId(c)).is_none(), "{params}: the compound becomes type");
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc == before, "{params}: undo restores it");
    }
}

#[test]
fn glyphs_side_by_side_in_an_even_odd_compound_still_become_type() {
    let mut s = session();
    let c = one_compound(&mut s, "oo");
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!(["oo"]), "{r}");
}

#[test]
fn a_compound_whose_glyph_stays_an_outline_keeps_its_other_glyphs_too() {
    let mut s = session();
    // An A (two contours), and far below it a period: a lone one-contour glyph beside other
    // shapes stays an outline, so the A, in the same compound, must stay too.
    let t = outlined(&mut s, json!({"x": 40, "y": 100, "text": "A", "size": 40}));
    let dot = outlined(&mut s, json!({"x": 40, "y": 300, "text": ".", "size": 40}));
    let paths = |s: &Session, id: u64| match s.doc().unwrap().doc.node(NodeId(id)).unwrap().children() {
        Some(c) => c.iter().map(|c| c.id.0).collect(),
        None => vec![id],
    };
    let mut ids: Vec<u64> = paths(&s, t);
    ids.extend(paths(&s, dot));
    assert_eq!(ids.len(), 2, "one path each");
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let c = s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [c, r]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let e = match s.execute("type.retype", &json!({})) {
        Ok(r) => panic!("the A became type without its period: {r}"),
        Err(e) => e.to_string(),
    };
    assert!(e.contains("(2 checked)"), "{e}");
    assert!(s.doc().unwrap().doc == before, "nothing changes");
}

#[test]
fn a_split_takes_a_shorter_run_when_the_longest_leaves_a_group_unmatched() {
    use kurbo::Shape;
    // Three groups A, B and C side by side. A and B together are a glyph, but then C alone is
    // none; A alone and B and C together are glyphs, so the split is A, BC.
    let mut p = kurbo::BezPath::new();
    for x in [0.0, 100.0, 200.0] {
        p.extend(kurbo::Rect::new(x, 0.0, x + 40.0, 40.0).path_elements(0.1));
    }
    let find = |path: &kurbo::BezPath, part: bool, _near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, b.x0.round() as i64, b.x1.round() as i64) {
            (true, 0, 140) => vec!["AB"],
            (true, 0, 40) => vec!["A"],
            (true, 100, 240) => vec!["BC"],
            _ => vec![],
        }
    };
    let groups = all_groups(&p);
    assert_eq!(super::parts(&p, &groups, &find, &|_, _| true), vec![vec!["A"], vec!["BC"]]);
    // A split the parts' paint does not allow is dropped.
    assert!(super::parts(&p, &groups, &find, &|_, _| false).is_empty());
    // With no way to match every group, nothing.
    let none = |path: &kurbo::BezPath, part: bool, _near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        if part && b.x0.round() as i64 == 0 && b.x1.round() as i64 == 140 { vec!["AB"] } else { vec![] }
    };
    assert!(super::parts(&p, &groups, &none, &|_, _| true).is_empty());
}

#[test]
fn a_split_the_paint_check_rejects_gives_way_to_another_complete_split() {
    use kurbo::Shape;
    let mut p = kurbo::BezPath::new();
    for x in [0.0, 100.0, 200.0] {
        p.extend(kurbo::Rect::new(x, 0.0, x + 40.0, 40.0).path_elements(0.1));
    }
    let find = |path: &kurbo::BezPath, part: bool, _near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, b.x0.round() as i64, b.x1.round() as i64) {
            (true, 0, 140) => vec!["AB"],
            (true, 0, 40) => vec!["A"],
            (true, 100, 140) => vec!["B"],
            (true, 200, 240) => vec!["C"],
            _ => vec![],
        }
    };
    let groups = all_groups(&p);
    // The longest runs first (AB, C) are rejected; the three single glyphs are kept.
    let singles = |runs: &[kurbo::BezPath], _near: bool| runs.len() == 3;
    assert_eq!(super::parts(&p, &groups, &find, &singles), vec![vec!["A"], vec!["B"], vec!["C"]]);
    // When no complete split is accepted, nothing.
    assert!(super::parts(&p, &groups, &find, &|_, _| false).is_empty());
}

#[test]
fn a_split_between_the_longest_and_the_shortest_runs_is_tried() {
    use kurbo::Shape;
    // Four groups A, B, C and D. ABC and D (the longest runs first) and the four single glyphs
    // (the shortest) are complete splits the paint check rejects; AB and CD is the one it accepts.
    let mut p = kurbo::BezPath::new();
    for x in [0.0, 100.0, 200.0, 300.0] {
        p.extend(kurbo::Rect::new(x, 0.0, x + 40.0, 40.0).path_elements(0.1));
    }
    let find = |path: &kurbo::BezPath, part: bool, _near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, b.x0.round() as i64, b.x1.round() as i64) {
            (true, 0, 240) => vec!["ABC"],
            (true, 0, 140) => vec!["AB"],
            (true, 200, 340) => vec!["CD"],
            (true, 0, 40) => vec!["A"],
            (true, 100, 140) => vec!["B"],
            (true, 200, 240) => vec!["C"],
            (true, 300, 340) => vec!["D"],
            _ => vec![],
        }
    };
    let groups = all_groups(&p);
    let pairs = |runs: &[kurbo::BezPath], _near: bool| runs.len() == 2 && runs.first().is_some_and(|r| r.bounding_box().x1.round() as i64 == 140);
    assert_eq!(super::parts(&p, &groups, &find, &pairs), vec![vec!["AB"], vec!["CD"]]);
}

#[test]
fn at_most_max_split_tries_complete_splits_are_checked() {
    use kurbo::Shape;
    // Twelve groups, every run of up to three of them a glyph: far more complete splits than the
    // cap. The paint check is asked about the cap's number of them and no more, and each run is
    // looked up at most once.
    let mut p = kurbo::BezPath::new();
    for i in 0..12 {
        let x = f64::from(i) * 100.0;
        p.extend(kurbo::Rect::new(x, 0.0, x + 40.0, 40.0).path_elements(0.1));
    }
    let groups = all_groups(&p);
    assert_eq!(groups.len(), 12);
    let looked_up = std::cell::Cell::new(0usize);
    let find = |_: &kurbo::BezPath| -> Vec<&'static str> {
        looked_up.set(looked_up.get() + 1);
        vec!["glyph"]
    };
    let asked = &std::cell::Cell::new(0usize);
    let accept_at = |n: usize| {
        move |_: &[kurbo::BezPath]| {
            asked.set(asked.get() + 1);
            asked.get() == n
        }
    };
    // Rejected every time: the search stops at the cap.
    assert!(super::split_parts(&groups, &find, &accept_at(usize::MAX)).is_empty());
    assert_eq!(asked.get(), super::MAX_SPLIT_TRIES);
    assert!(looked_up.get() <= super::MAX_RUN * groups.len(), "{} lookups", looked_up.get());
    // A first split that is accepted looks up its own runs only: four runs of three groups.
    asked.set(0);
    looked_up.set(0);
    assert_eq!(super::split_parts(&groups, &find, &accept_at(1)).len(), 4);
    assert_eq!(looked_up.get(), 4);
    // The last split tried is still taken; the one past the cap is not tried.
    asked.set(0);
    assert!(super::split_parts(&groups, &find, &accept_at(super::MAX_SPLIT_TRIES)).len() > 1);
    asked.set(0);
    assert!(super::split_parts(&groups, &find, &accept_at(super::MAX_SPLIT_TRIES + 1)).is_empty());
    assert_eq!(asked.get(), super::MAX_SPLIT_TRIES);
}

/// `path`'s contour groups, with no limit on the contours grouped.
fn all_groups(path: &kurbo::BezPath) -> Vec<kurbo::BezPath> {
    let mut budget = usize::MAX;
    super::split_groups(path, &mut budget)
}

#[test]
fn exact_matches_of_the_parts_come_before_a_near_match_of_the_whole() {
    use kurbo::Shape;
    // Two groups side by side, each exactly a glyph (two periods), and the whole only nearly one
    // (a dieresis): the parts win.
    let mut p = kurbo::BezPath::new();
    for x in [0.0, 100.0] {
        p.extend(kurbo::Rect::new(x, 0.0, x + 40.0, 40.0).path_elements(0.1));
    }
    let find = |path: &kurbo::BezPath, part: bool, near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, near, b.x0.round() as i64, b.x1.round() as i64) {
            (false, true, 0, 140) => vec!["dieresis"],
            (true, _, 0, 40) | (true, _, 100, 140) => vec!["period"],
            _ => vec![],
        }
    };
    let groups = all_groups(&p);
    assert_eq!(super::parts(&p, &groups, &find, &|_, _| true), vec![vec!["period"], vec!["period"]]);
    // When the parts match only nearly too, the whole near match comes first.
    let near_only = |path: &kurbo::BezPath, part: bool, near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, near, b.x0.round() as i64, b.x1.round() as i64) {
            (false, true, 0, 140) => vec!["dieresis"],
            (true, true, 0, 40) | (true, true, 100, 140) => vec!["period"],
            _ => vec![],
        }
    };
    assert_eq!(super::parts(&p, &groups, &near_only, &|_, _| true), vec![vec!["dieresis"]]);
    // The split's paint check is told whether the parts were found nearly: exact parts are
    // checked at tolerance 0 whatever tolerance the command was given.
    let seen = std::cell::RefCell::new(vec![]);
    let record = |_: &[kurbo::BezPath], near: bool| {
        seen.borrow_mut().push(near);
        true
    };
    assert_eq!(super::parts(&p, &groups, &find, &record), vec![vec!["period"], vec!["period"]]);
    assert_eq!(*seen.borrow(), vec![false]);
    seen.borrow_mut().clear();
    let split_near_only = |path: &kurbo::BezPath, part: bool, near: bool| -> Vec<&'static str> {
        let b = path.bounding_box();
        match (part, near, b.x0.round() as i64, b.x1.round() as i64) {
            (true, true, 0, 40) | (true, true, 100, 140) => vec!["period"],
            _ => vec![],
        }
    };
    assert_eq!(super::parts(&p, &groups, &split_near_only, &record), vec![vec!["period"], vec!["period"]]);
    assert_eq!(*seen.borrow(), vec![true]);
}

#[test]
fn a_shape_that_lost_a_glyph_drops_the_lines_of_its_other_glyphs() {
    use std::collections::HashMap;
    // Shapes 1 and 2 are two glyphs each, shape 3 one. Line 0 sets one glyph of 1 and one of 2;
    // line 1 sets the other glyph of 2 and shape 3. Shape 1's other glyph is set nowhere.
    let (a, b, c) = (NodeId(1), NodeId(2), NodeId(3));
    let glyphs_of: HashMap<NodeId, usize> = [(a, 2), (b, 2), (c, 1)].into();
    let lines = vec![vec![a, b], vec![b, c]];
    let dropped = super::partial_lines(&lines, &glyphs_of);
    assert!(dropped.contains(&0), "shape 1 is set only in part: {dropped:?}");
    assert!(dropped.contains(&1), "then shape 2 is too: {dropped:?}");
    // Every glyph set: nothing goes.
    let whole = vec![vec![a, b], vec![a, b, c]];
    assert!(super::partial_lines(&whole, &glyphs_of).is_empty());
}

#[test]
fn a_compound_of_too_many_path_elements_is_not_grouped() {
    // Two contours side by side, each a polygon of `k` corners.
    let two = |k: usize| {
        let mut p = kurbo::BezPath::new();
        for x in [0.0, 300.0] {
            for i in 0..k {
                let a = i as f64 / k as f64 * std::f64::consts::TAU;
                let q = kurbo::Point::new(x + 100.0 + 100.0 * a.cos(), 100.0 + 100.0 * a.sin());
                if i == 0 { p.move_to(q) } else { p.line_to(q) }
            }
            p.close_path();
        }
        p
    };
    let mut budget = usize::MAX;
    assert_eq!(super::split_groups(&two(100), &mut budget).len(), 2, "a small compound is grouped");
    let big = two(super::MAX_SPLIT_ELEMENTS / 2);
    assert!(big.elements().len() > super::MAX_SPLIT_ELEMENTS);
    let mut budget = usize::MAX;
    assert!(super::split_groups(&big, &mut budget).is_empty(), "past the element limit it is matched whole only");
    assert_eq!(budget, usize::MAX, "and takes no contours from the budget");
}

#[test]
fn the_runs_and_near_keys_a_selection_is_looked_up_by_are_bounded() {
    use kurbo::Shape;
    use vectorcraft_text::retype::MAX_NEAR_KEYS;
    // A compound path of `n` rectangles side by side, each of another width: every run of up to
    // three of them has its own near key.
    let row = |n: usize| {
        let mut p = kurbo::BezPath::new();
        for i in 0..n {
            let x = i as f64 * 100.0;
            p.extend(kurbo::Rect::new(x, 0.0, x + 10.0 + i as f64 * 0.37, 40.0).path_elements(0.1));
        }
        p
    };
    // Each shape with its contour groups, as the command looks them up.
    let with_groups = |shapes: &[kurbo::BezPath]| -> Vec<(kurbo::BezPath, Vec<kurbo::BezPath>)> {
        let mut budget = super::MAX_SPLIT_TOTAL_CONTOURS;
        shapes.iter().map(|p| (p.clone(), super::split_groups(p, &mut budget))).collect()
    };
    let keys_of = |shapes: &[(kurbo::BezPath, Vec<kurbo::BezPath>)], tolerance: f64| {
        super::lookup_keys(shapes.iter().map(|(p, g)| (p, g.as_slice())), tolerance)
    };
    let few = with_groups(&[row(20)]);
    assert_eq!(super::runs(&few[0].1).len(), 20 + 19 + 18, "every run of 1 to 3 groups");
    let (_, keys) = keys_of(&few, 0.01);
    assert!(keys.len() > 20, "{}", keys.len());
    assert!(keys_of(&few, 0.0).1.is_empty(), "no near keys without a tolerance");
    // Past 256 groups the shape is matched whole only, and past 512 contours its contours are not
    // grouped at all.
    assert!(all_groups(&row(300)).is_empty());
    assert_eq!(all_groups(&row(256)).len(), 256);
    let mut nested = kurbo::BezPath::new();
    for i in 0..200 {
        // Each square with two squares inside it: 200 groups of 3 contours.
        let x = i as f64 * 100.0;
        nested.extend(kurbo::Rect::new(x, 0.0, x + 90.0, 90.0).path_elements(0.1));
        nested.extend(kurbo::Rect::new(x + 10.0, 10.0, x + 40.0, 40.0).path_elements(0.1));
        nested.extend(kurbo::Rect::new(x + 50.0, 50.0, x + 80.0, 80.0).path_elements(0.1));
    }
    assert!(all_groups(&nested).is_empty(), "600 contours");
    assert_eq!(vectorcraft_text::retype::clusters(&nested).len(), 200, "which would make 200 groups");
    // Many shapes with many runs each: the near keys stop at the cap.
    let mut shapes: Vec<kurbo::BezPath> = (0..8).map(|k| kurbo::Affine::scale_non_uniform(1.0, 1.0 + k as f64 * 0.5) * row(200)).collect();
    let (_, keys) = keys_of(&with_groups(&shapes), 0.01);
    assert_eq!(keys.len(), MAX_NEAR_KEYS);
    // A plain shape selected after them still gets its own near key: whole shapes come first.
    let plain = kurbo::Circle::new((0.0, 0.0), 30.0).to_path(0.1);
    let key = vectorcraft_text::retype::near_key(&plain).unwrap();
    shapes.push(plain);
    let (_, keys) = keys_of(&with_groups(&shapes), 0.01);
    assert!(keys.contains(&key));
    // One command groups at most MAX_SPLIT_TOTAL_CONTOURS contours: the shapes after them are
    // matched whole only.
    let full = vec![row(256); super::MAX_SPLIT_TOTAL_CONTOURS / 256 + 1];
    let split: Vec<usize> = with_groups(&full).iter().map(|(_, g)| g.len()).collect();
    assert!(split[..split.len() - 1].iter().all(|n| *n == 256), "{split:?}");
    assert_eq!(split.last(), Some(&0));
}

#[test]
fn slight_tracking_without_spaces_is_kept() {
    // Tracking 3 moves the last glyph less than the layout check notices: both readings draw it.
    // A word space both readings keep does not cost the tracking either.
    for text in ["MMM", "M MM"] {
        let mut s = session();
        let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": 3})]);
        assert_eq!(r["texts"], json!([text]), "{r}");
        assert_eq!(r["fonts"][0]["tracking"], json!(3.0), "{r}");
        assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    }
}

/// The lines Retype reads from the outlines in group `g` (all in the face of its first glyph's
/// first match), before their tracking is measured and their layout checked.
fn lines_in(s: &Session, g: u64) -> Vec<super::Line> {
    let d = &s.doc().unwrap().doc;
    let (mut barrier, mut truncated, mut pieces) = (true, false, vec![]);
    super::collect(d.node(NodeId(g)).unwrap(), d.parent_of(NodeId(g)), 0, &mut barrier, &mut truncated, &mut pieces);
    let db = vectorcraft_text::FontDb::global();
    let wanted = pieces.iter().filter_map(|pc| vectorcraft_text::retype::shape_key(&pc.path)).collect();
    let index = super::RetypeIndex::build(db, super::candidate_faces(db, d, &[]), &wanted);
    let face = index.matches(&pieces[0].path)[0].face.id();
    let mut lines: Vec<super::Line> = vec![];
    for pc in &pieces {
        let cands: Vec<_> = index.matches(&pc.path).into_iter().filter(|m| m.face.id() == face).collect();
        if !lines.last_mut().is_some_and(|l| super::join(l, pc, &cands)) {
            lines.extend(super::new_line(pc, &cands, 0.0));
        }
    }
    lines
}

#[test]
fn the_glyphs_after_one_type_cannot_draw_keep_their_tracking() {
    // Glyph `bad` of a tracked line read as another glyph of its font, as when the line's type
    // would draw another form there: the glyphs after it are still one tracked object.
    for bad in [0usize, 1] {
        let mut s = session();
        let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "MMMMMM", "size": 30})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id, "tracking": 300})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let lines = lines_in(&s, g);
        assert_eq!(lines.len(), 1);
        let mut budget = 1000;
        let mut l = lines.into_iter().next().unwrap().tracked(false, &mut budget);
        assert_eq!(l.tracking, 300.0);
        let other = l.matches[bad].gid + 1;
        l.matches[bad].gid = other;
        let bad_id = l.pieces[bad];
        let mut unmatched = vec![];
        let out = super::verified(vec![l], false, &mut unmatched);
        assert_eq!(unmatched, vec![bad_id], "{bad}");
        let last = out.last().unwrap();
        assert_eq!(last.pieces.len(), 5 - bad, "{bad}: {:?}", out.iter().map(|l| (l.pieces.len(), l.tracking)).collect::<Vec<_>>());
        assert_eq!(last.tracking, 300.0, "{bad}");
    }
}

#[test]
fn the_tracking_measure_counts_the_spaces_it_lays_out() {
    // Three glyphs ten spaces apart: a budget of six times the glyphs, short of the first
    // layout's 23 characters, lays nothing out.
    let mut s = session();
    let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "a          b          c", "size": 30}));
    let lines = lines_in(&s, g);
    assert_eq!(lines.len(), 1);
    assert!(lines[0].text.chars().filter(|c| *c == ' ').count() >= 20, "{}", lines[0].text);
    let mut budget = 3 * 6;
    let l = lines.into_iter().next().unwrap().tracked(false, &mut budget);
    assert_eq!((budget, l.tracking), (3 * 6, 0.0));
}

#[test]
fn the_tracking_measure_needs_only_the_layouts_it_runs() {
    // The line as read, without spaces, with one space per gap, then the tracked reading: a
    // budget of exactly those characters recovers the tracking.
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "MMMMMM", "size": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "tracking": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let lines = lines_in(&s, g);
    assert_eq!(lines.len(), 1);
    let mut budget = lines[0].text.chars().count() + 6 + 11 + 6;
    let l = lines.into_iter().next().unwrap().tracked(false, &mut budget);
    assert_eq!((budget, l.tracking), (0, 300.0));
}

#[test]
fn the_rest_of_a_widely_tracked_line_keeps_its_tracking_after_a_split() {
    // Tracking 40 spaces wide: read untracked, the rest after the split holds 40 spaces per gap,
    // and its measure still has the budget to lay them out.
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "M".repeat(100), "size": 2})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "tracking": 8000})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let mut budget = 100_000;
    let mut l = lines_in(&s, g).into_iter().next().unwrap().tracked(false, &mut budget);
    assert_eq!(l.tracking, 8000.0);
    l.matches[0].gid += 1;
    let mut unmatched = vec![];
    let out = super::verified(vec![l], false, &mut unmatched);
    assert_eq!(unmatched.len(), 1);
    let last = out.last().unwrap();
    assert_eq!((last.pieces.len(), last.tracking), (99, 8000.0), "{:?}", out.iter().map(|l| (l.pieces.len(), l.tracking)).collect::<Vec<_>>());
}

#[test]
fn glyphs_type_cannot_draw_do_not_use_up_the_tracking_measure() {
    // A long tracked line whose first five glyphs are read as other glyphs: the rest is still one
    // tracked object.
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "i".repeat(100), "size": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "tracking": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let mut budget = 1000;
    let mut l = lines_in(&s, g).into_iter().next().unwrap().tracked(false, &mut budget);
    assert_eq!(l.tracking, 300.0);
    for m in l.matches.iter_mut().take(5) {
        m.gid += 1;
    }
    let mut unmatched = vec![];
    let out = super::verified(vec![l], false, &mut unmatched);
    assert_eq!(unmatched.len(), 5);
    let last = out.last().unwrap();
    assert_eq!((last.pieces.len(), last.tracking), (95, 300.0), "{:?}", out.iter().map(|l| (l.pieces.len(), l.tracking)).collect::<Vec<_>>());
}

#[test]
fn tightly_tracked_type_keeps_its_tracking() {
    // Tracking tighter than a space: the space moves the next glyph back.
    // M MM: of its two gaps, the word space is the tighter, and the lower median.
    // M M MM: most gaps hold a word space, so the tightest extras alone seed a tracking that
    // splits the last glyph.
    for text in ["MMMM", "MMMM MM", "M MM", "MM M", "M M MM"] {
        let mut s = session();
        let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": -300})]);
        assert_eq!(r["texts"], json!([text]), "{r}");
        assert_eq!(r["fonts"][0]["tracking"], json!(-300.0), "{r}");
        assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    }
}

#[test]
fn kerned_pairs_in_tight_tracking_are_not_read_as_spaces() {
    // Tracking -250 gives a space a negative advance; the pair kerning of T and a is no space.
    let mut s = session();
    let text = "MMMTaMMM";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": -250})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(-250.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_word_space_between_kerned_letters_is_read_as_a_space() {
    // T and a kern when they touch; with a space between them they do not.
    let mut s = session();
    let text = "MMMT aMMM";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": -250})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(-250.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    // Tracking that cancels the space's advance: the space still stops the pair kerning.
    let mut s = session();
    let text = "MMMT aMMM";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": -200})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(-200.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    // Tracking that nearly cancels it: the space still moves the glyphs after it a little.
    let mut s = session();
    let text = "MMMM MM";
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": text, "size": 30}), &[json!({"tracking": -199.5})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert!(near(r["fonts"][0]["tracking"].as_f64().unwrap(), -199.5, 1e-3), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    // A word space a little narrower than its tracking makes it (the word after it moved left by
    // less than the layout check notices): one space still fits better than the kerned pair.
    for (tracking, dx) in [(-190, -0.18), (-193, -0.24)] {
        let mut s = session();
        let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "MMMc aMMM", "size": 30})).unwrap()["id"].as_u64().unwrap();
        s.execute("text.setRangeStyle", &json!({"id": id, "tracking": tracking})).unwrap();
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let kids = children_of(&s, g);
        assert_eq!(kids.len(), 8);
        s.execute("select.set", &json!({"ids": kids[4..]})).unwrap();
        s.execute("object.move", &json!({"dx": dx, "dy": 0})).unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({})).unwrap();
        assert_eq!(r["texts"], json!(["MMMc aMMM"]), "{tracking}: {r}");
    }
    // A space that kerns with the letter after it.
    let mut s = session();
    let text = "MMMd TMMM";
    let p = json!({"x": 40, "y": 100, "text": text, "size": 30, "font": "Source Serif 4", "style": "Regular"});
    let (before, r) = retyped(&mut s, p, &[json!({"tracking": -217})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(-217.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn an_untracked_word_is_not_read_as_spaced_letters_in_tight_tracking() {
    // Letters, each followed by a space, at tracking of minus half a space, sit where the word's
    // letters do.
    let mut s = session();
    let (_, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": "Third", "size": 24}), &[]);
    assert_eq!(r["texts"], json!(["Third"]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(0.0), "{r}");
}

#[test]
fn glyphs_barely_ahead_of_each_other_stay_one_line() {
    let mut s = session();
    let (before, r) = retyped(&mut s, json!({"x": 40, "y": 100, "text": "MMMM", "size": 30}), &[json!({"tracking": -700})]);
    assert_eq!(r["texts"], json!(["MMMM"]), "{r}");
    assert_eq!(r["fonts"][0]["tracking"], json!(-700.0), "{r}");
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn tracking_near_a_whole_number_is_kept_as_measured_when_rounding_moves_glyphs() {
    let mut s = session();
    let text = "i".repeat(400);
    let (before, r) = retyped(&mut s, json!({"x": 10, "y": 100, "text": text, "size": 10}), &[json!({"tracking": 50.04})]);
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert!(near(r["fonts"][0]["tracking"].as_f64().unwrap(), 50.04, 1e-3), "{}", r["fonts"]);
    assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
}

#[test]
fn a_tracking_translucent_glyphs_cannot_take_keeps_the_tracking_that_fits() {
    // The first three glyphs are tracked 50 and the last 150 further on. A tracking tighter than
    // zero is not tried for glyphs that cannot be one object, and the tracked prefix is kept.
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "MMMM", "size": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": 2, "tracking": 50})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "start": 2, "end": 3, "tracking": 150})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let kids = children_of(&s, g);
    s.edit("test", |d, _| {
        for k in &kids {
            d.node_mut(NodeId(*k)).unwrap().opacity = 0.5;
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    let texts: Vec<&str> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert_eq!(texts.concat(), "MMMM", "{r}");
    assert!(texts.iter().any(|t| t.len() > 1), "the tracked glyphs stay one object: {r}");
}

#[test]
fn overlapping_translucent_glyphs_are_not_tracked_into_one_object() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "MMMM", "size": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id, "tracking": -300})).unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    let kids = children_of(&s, g);
    s.edit("test", |d, _| {
        for k in &kids {
            d.node_mut(NodeId(*k)).unwrap().opacity = 0.5;
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    // Each glyph composites on its own over the one it overlaps, as the outlines did.
    assert_eq!(r["texts"], json!(["M", "M", "M", "M"]), "{r}");
    assert!(r["fonts"].as_array().unwrap().iter().all(|f| f["tracking"] == json!(0.0)), "{r}");
}

#[test]
fn translucent_gradient_or_stroked_lines_stay_apart() {
    for case in ["opacity", "blend", "gradient", "stroke"] {
        let mut s = session();
        let g = outlined(&mut s, json!({"x": 40, "y": 100, "text": "One\nTwo", "size": 30}));
        let kids = children_of(&s, g);
        match case {
            "gradient" => {
                let gradient = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#000"}, {"offset": 1, "color": "#fff"}]});
                s.execute("paint.setFill", &json!({"gradient": gradient, "ids": kids})).unwrap();
            }
            "stroke" => {
                s.execute("paint.setStroke", &json!({"color": "#ff0000", "ids": kids})).unwrap();
            }
            _ => {
                s.edit("test", |d, _| {
                    for k in &kids {
                        let n = d.node_mut(NodeId(*k)).unwrap();
                        if case == "opacity" {
                            n.opacity = 0.5;
                        } else {
                            n.blend = vectorcraft_color::BlendMode::Multiply;
                        }
                    }
                    Ok(())
                })
                .unwrap();
            }
        }
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("type.retype", &json!({})).unwrap();
        assert_eq!(r["texts"], json!(["One", "Two"]), "{case}: one object would composite or fill the lines as one: {r}");
    }
}

#[test]
fn lines_stay_point_type_past_the_text_wrap_limit_of_their_layer() {
    // A Text Wrap object above the lines that moves no glyph, on their layer or on another one,
    // with the limit lowered to nothing: only on their layer does it count.
    for same_layer in [true, false] {
        let mut s = session();
        let text = "First line\nSecond one";
        let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": text, "size": 24})).unwrap()["id"].as_u64().unwrap();
        let before = text_of(&s, id);
        s.execute("select.set", &json!({"ids": [id]})).unwrap();
        let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
        if !same_layer {
            // Area type below the Text Wrap object on the other layer: work there, none on this one.
            s.execute("layer.new", &json!({})).unwrap();
            let other = s.execute("text.create", &json!({"x": 450, "y": 300, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
            s.execute("type.convertToAreaType", &json!({"ids": [other]})).unwrap();
        }
        let rect = s.execute("shape.rectangle", &json!({"x": 500, "y": 330, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        s.execute("object.textWrap.make", &json!({"ids": [rect]})).unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        super::WRAP_LIMIT.with(|c| c.set(0));
        let r = s.execute("type.retype", &json!({}));
        super::WRAP_LIMIT.with(|c| c.set(super::MAX_WRAP_WORK));
        let r = r.unwrap();
        assert_eq!(r["texts"], json!([text]), "{r}");
        assert_eq!(r["fonts"][0]["area"], json!(!same_layer), "{same_layer}: {r}");
        assert_same_places(&before, &text_of(&s, r["ids"][0].as_u64().unwrap()));
    }
}

#[test]
fn empty_subpaths_of_a_text_wrap_object_count_as_wrap_work() {
    // Each one is copied into the wrap shapes of the area type below it.
    let mut s = session();
    let t = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "Area", "size": 12})).unwrap()["id"].as_u64().unwrap();
    s.execute("type.convertToAreaType", &json!({"ids": [t]})).unwrap();
    let rect = s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.textWrap.make", &json!({"ids": [rect]})).unwrap();
    let base = vectorcraft_doc::text::wrap_copy_units(&s.doc().unwrap().doc);
    let file_work = vectorcraft_doc::text::wrap_work(&s.doc().unwrap().doc);
    s.edit("test", |d, _| {
        if let NodeKind::Path { path, .. } = &mut d.node_mut(NodeId(rect)).unwrap().kind {
            for _ in 0..100 {
                path.subpaths.push(vectorcraft_geom::SubPath::new(vec![], true));
            }
        }
        Ok(())
    })
    .unwrap();
    assert!(vectorcraft_doc::text::wrap_copy_units(&s.doc().unwrap().doc) >= base + 100);
    // The count file import and the `.ai` editing copy use stays anchors and shapes only.
    assert_eq!(vectorcraft_doc::text::wrap_work(&s.doc().unwrap().doc), file_work);
}

#[test]
fn lines_under_a_text_wrap_object_stay_point_type() {
    let mut s = session();
    let text = "First line\nSecond one";
    let id = s.execute("text.create", &json!({"x": 40, "y": 100, "text": text, "size": 24})).unwrap()["id"].as_u64().unwrap();
    let before = text_of(&s, id);
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    let g = s.execute("type.createOutlines", &json!({})).unwrap()["ids"][0].as_u64().unwrap();
    // Above the outlines, over the end of both lines: area type would wrap round it.
    let rect = s.execute("shape.rectangle", &json!({"x": 90, "y": 70, "width": 60, "height": 50})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.textWrap.make", &json!({"ids": [rect]})).unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("type.retype", &json!({})).unwrap();
    assert_eq!(r["texts"], json!([text]), "{r}");
    assert_eq!(r["fonts"][0]["area"], json!(false), "{r}");
    let t = text_of(&s, r["ids"][0].as_u64().unwrap());
    assert!(matches!(t.kind, vectorcraft_doc::TextKind::Point));
    assert_same_places(&before, &t);
}
