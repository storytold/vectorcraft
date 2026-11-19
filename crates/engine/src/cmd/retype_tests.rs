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
            lines.extend(super::new_line(pc, &cands));
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
