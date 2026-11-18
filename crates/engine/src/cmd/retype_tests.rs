use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind, TextObject};

use crate::Session;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

/// Point text, outlined: the id of the outlines' group.
fn outlined(s: &mut Session, p: serde_json::Value) -> u64 {
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
    for bad in [json!("Source Sans 3"), json!([1]), json!({"family": "x"})] {
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
        let texts: Vec<String> = r["texts"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_owned()).collect();
        assert_eq!(texts.concat(), text, "split by the tracking, every letter still becomes type: {r}");
        assert!(texts.len() > 1, "the tracking splits the line: {r}");
        assert_eq!(r["unmatched"], json!([]), "{r}");
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
    // Tracking splits the line at every glyph: the layout check must not lay out the rest of the
    // line again after each split.
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
    assert_eq!(texts.concat(), text);
    assert_eq!(r["unmatched"], json!([]));
    eprintln!("450 tracked glyphs: {:?}, {} objects", t0.elapsed(), texts.len());
}
