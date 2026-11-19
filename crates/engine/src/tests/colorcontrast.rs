//! `color.contrast`: the WCAG 2 contrast of a color against the color behind the selection.

use serde_json::{Value, json};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn ratio(r: &Value) -> f64 {
    r["ratio"].as_f64().unwrap()
}

#[test]
fn a_pair_just_under_aa_fails_it() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#fdc96e", "against": "#355f78"})).unwrap();
    assert!((ratio(&r) - 4.496).abs() < 1e-3, "{r}");
    assert_eq!(r["against"], "#355f78");
    assert_eq!(r["normalText"], json!({"aa": false, "aaa": false}));
    assert_eq!(r["largeText"], json!({"aa": true, "aaa": false}));
    assert_eq!(r["graphics"], json!({"aa": true}), "graphics have no AAA level");
    assert_eq!(vectorcraft_color::wcag::ratio_text(ratio(&r)), "4.49:1");
}

#[test]
fn black_on_white_is_21_and_grays_fall_either_side_of_aa() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
    assert_eq!(r["normalText"], json!({"aa": true, "aaa": true}));
    let pass = s.execute("color.contrast", &json!({"color": "#767676", "against": "#ffffff"})).unwrap();
    assert!((ratio(&pass) - 4.542).abs() < 1e-3 && pass["normalText"]["aa"] == true, "{pass}");
    let fail = s.execute("color.contrast", &json!({"color": "#777777", "against": "#ffffff"})).unwrap();
    assert!((ratio(&fail) - 4.478).abs() < 1e-3 && fail["normalText"]["aa"] == false, "{fail}");
}

#[test]
fn the_background_is_the_locked_art_behind_the_selected_type() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 80})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffff00"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("text.create", &json!({"x": 60, "y": 70, "text": "Label", "size": 24, "color": "#000000"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert_eq!(r["against"], "#ffff00", "{r}");
    let want = vectorcraft_color::wcag::contrast_ratio([0.0; 3], [1.0, 1.0, 0.0]);
    assert!((ratio(&r) - want).abs() < 1e-3, "{r}");
}

#[test]
fn a_translucent_fill_counts_at_the_opacity_it_is_drawn_with() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert_eq!(r["against"], "#ffffff", "the selected rectangle is hidden: the artboard's white shows");
    let gray = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [1.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "{r}");
    // A given opacity replaces it, and a given color is drawn fully opaque.
    let full = s.execute("color.contrast", &json!({"opacity": 100})).unwrap();
    assert!((ratio(&full) - 21.0).abs() < 1e-3, "{full}");
    let given = s.execute("color.contrast", &json!({"color": "#000000"})).unwrap();
    assert!((ratio(&given) - 21.0).abs() < 1e-3, "{given}");
}

#[test]
fn a_translucent_group_counts_for_the_art_inside_it() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let gray = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [1.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "the group draws the black at 50 %: {r}");
}

#[test]
fn a_blend_mode_counts_as_it_draws_over_the_background() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"blend": "multiply"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert_eq!(r["against"], "#ff0000", "{r}");
    assert!((ratio(&r) - 1.0).abs() < 1e-3, "white multiplied over red draws red: {r}");
    // A group's blend mode counts for the art inside it, and a given color draws in Normal mode.
    s.execute("transparency.set", &json!({"blend": "normal"})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"blend": "multiply"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&r) - 1.0).abs() < 1e-3, "{r}");
    let given = s.execute("color.contrast", &json!({"color": "#ffffff"})).unwrap();
    let want = vectorcraft_color::wcag::contrast_ratio([1.0; 3], [1.0, 0.0, 0.0]);
    assert!((ratio(&given) - want).abs() < 1e-3, "{given}");
}

/// A selected blue square at (50, 50) over locked red art, with its fill row's blend mode set to
/// `row`.
fn blended_over_red(row: &str) -> Session {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#4080c0"})).unwrap();
    s.execute("transparency.set", &json!({"item": fill_index(&s), "blend": row})).unwrap();
    s
}

/// The index of the selected object's fill row.
fn fill_index(s: &Session) -> usize {
    let st = s.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap();
    n.appearance.items.iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap()
}

/// The contrast against red of the color the canvas draws at the square's center.
fn drawn_ratio(s: &Session) -> f64 {
    let doc = &s.doc().unwrap().doc;
    let img = vectorcraft_render::Renderer::new().render_region(doc, vectorcraft_geom::Rect::new(99.0, 99.0, 101.0, 101.0), 1.0, true);
    let p = img.pixel(1, 1);
    vectorcraft_color::wcag::contrast_ratio([p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0), [1.0, 0.0, 0.0])
}

#[test]
fn nested_blend_modes_count_as_the_canvas_draws_them() {
    let mut s = blended_over_red("multiply");
    s.execute("transparency.set", &json!({"item": null, "blend": "screen"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert_eq!(r["against"], "#ff0000", "{r}");
    let drawn = drawn_ratio(&s);
    assert!((ratio(&r) - drawn).abs() < 0.02, "the fill multiplies, then the object screens: {r} against {drawn}");
    // A translucent row and object: the canvas takes the backdrop back out of a group that blends
    // through before it blends the group, so the opacities multiply over the nested modes.
    s.execute("transparency.set", &json!({"item": fill_index(&s), "opacity": 50})).unwrap();
    s.execute("transparency.set", &json!({"item": null, "blend": "overlay", "opacity": 70})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let drawn = drawn_ratio(&s);
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
    s.execute("transparency.set", &json!({"item": null, "opacity": 100})).unwrap();
    // Grouped, the group's mode blends the result again.
    s.execute("transparency.set", &json!({"item": null, "blend": "normal"})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"blend": "difference"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let drawn = drawn_ratio(&s);
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// In a 50 % group, a black square over a white one draws black over the black outside: the group's
/// opacity applies to the white behind the square once, not again over it.
#[test]
fn a_translucent_group_draws_the_art_behind_the_object_once() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let white = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let black = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.group", &json!({"ids": [white.0, black.0]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![black];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let img = vectorcraft_render::Renderer::new().render_region(doc, vectorcraft_geom::Rect::new(99.0, 99.0, 101.0, 101.0), 1.0, true);
    let p = img.pixel(1, 1);
    let behind = vectorcraft_color::Color::from_hex(r["against"].as_str().unwrap()).unwrap().to_rgb_uncalibrated();
    let drawn = vectorcraft_color::wcag::contrast_ratio([p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0), behind);
    assert!(p[0] < 3, "the canvas draws black: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// The pixel the canvas draws at (100, 100), and its contrast against the answer's `against`.
fn canvas_ratio(s: &Session, r: &Value) -> ([u8; 4], f64) {
    let doc = &s.doc().unwrap().doc;
    let img = vectorcraft_render::Renderer::new().render_region(doc, vectorcraft_geom::Rect::new(99.0, 99.0, 101.0, 101.0), 1.0, true);
    let p = img.pixel(1, 1);
    let behind = vectorcraft_color::Color::from_hex(r["against"].as_str().unwrap()).unwrap().to_rgb_uncalibrated();
    (p, vectorcraft_color::wcag::contrast_ratio([p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0), behind))
}

/// A selected white Multiply square over a white one in a 50 % group over black: the square
/// multiplies with the white inside the group, and the group's opacity applies once.
#[test]
fn a_blend_mode_in_a_translucent_group_blends_with_the_art_inside_it() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let back = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let square = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"blend": "multiply"})).unwrap();
    s.execute("object.group", &json!({"ids": [back.0, square.0]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![square];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!((i32::from(p[0]) - 128).abs() < 3, "the canvas draws half gray: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// In a dynamic symbol instance whose art is a 50 % group of a black square over a white one, over
/// black, the picked black square draws over the black outside the group, not over the group's
/// white.
#[test]
fn a_picked_symbol_object_in_a_translucent_group_of_the_art_draws_over_what_is_outside_it() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let white = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let black = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let group = id(s.execute("object.group", &json!({"ids": [white.0, black.0]})).unwrap());
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let inst = id(s.execute("symbol.new", &json!({"name": "Pair", "ids": [group.0], "type": "dynamic"})).unwrap());
    let doc = &s.doc().unwrap().doc;
    let art = &doc.symbols.iter().find(|x| x.name == "Pair").unwrap().art;
    let path = if vectorcraft_doc::symbol_overrides::part(art, &[0, 1]).is_some() { vec![0, 1] } else { vec![1] };
    let st = s.active_mut().unwrap();
    st.selection.objects = vec![inst];
    st.selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path });
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!(p[0] < 3, "the canvas draws black: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// As above with the group's 50 % set by the instance instead of the symbol, and the picked black
/// square itself at 50 %: the square draws over the group's white, read with the instance's
/// opacity for the group taken out, then the group's 50 % applies once over the black outside.
#[test]
fn an_instances_opacity_for_a_group_of_its_art_applies_once() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let white = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let black = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let group = id(s.execute("object.group", &json!({"ids": [white.0, black.0]})).unwrap());
    let inst = id(s.execute("symbol.new", &json!({"name": "Pair", "ids": [group.0], "type": "dynamic"})).unwrap());
    let doc = &s.doc().unwrap().doc;
    let art = &doc.symbols.iter().find(|x| x.name == "Pair").unwrap().art;
    let path = if vectorcraft_doc::symbol_overrides::part(art, &[0, 1]).is_some() { vec![0, 1] } else { vec![1] };
    let st = s.active_mut().unwrap();
    let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(inst).unwrap();
    let over = vectorcraft_doc::SymbolOverride { path: path[..path.len() - 1].to_vec(), opacity: Some(0.5), ..Default::default() };
    n.extras_mut().symbol_overrides = vec![over];
    st.selection.objects = vec![inst];
    st.selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path });
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!((i32::from(p[0]) - 64).abs() < 3, "the canvas draws a quarter gray: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// A selected 50 % black square in a 50 % group, over a 50 % white square, all in a Difference group
/// over gray: the inner group's opacity applies to the black square only, and the white beside the
/// inner group counts once in the alpha the Difference group takes the gray back out by.
#[test]
fn a_groups_opacity_leaves_out_the_art_beside_it_in_a_blending_group() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#8c8c8c"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let white = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let black = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let inner = id(s.execute("object.group", &json!({"ids": [black.0]})).unwrap());
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("object.group", &json!({"ids": [white.0, inner.0]})).unwrap();
    s.execute("transparency.set", &json!({"blend": "difference"})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![black];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    // The canvas pixel is rounded to 8 bits, which moves this dark gray's ratio by up to 0.04; with
    // the white counted twice the answer is 3.15.
    assert!((ratio(&r) - drawn).abs() < 0.05, "{r} against {drawn} ({p:?})");
}

/// An object that isolates its blending, with a white Multiply fill over a blue one, over red: the
/// white multiplies with the blue row below it.
#[test]
fn a_row_in_an_isolating_object_blends_with_the_rows_below_it() {
    let mut s = blended_over_red("normal");
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.node(st.selection.objects[0]).unwrap().appearance.items;
    let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "blend": "multiply"})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("transparency.set", &json!({"item": null, "isolate": true})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!(p[0] < 3 && p[2] > 252, "the canvas draws blue: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// An object that isolates its blending, in a Screen group over black: a 50 % white fill over an
/// opaque white one composites to opaque white inside the object, so the group screens opaque
/// white. With the lower fill removed, the object holds the white at 50 %.
#[test]
fn an_isolating_object_in_a_blending_group_carries_the_alpha_of_its_composite() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let square = id(s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.node(square).unwrap().appearance.items;
    let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    let lower = items.iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "opacity": 50})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("transparency.set", &json!({"item": null, "isolate": true})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("transparency.set", &json!({"blend": "screen"})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![square];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!(p[0] > 252 && p[1] > 252 && p[2] > 252, "the canvas draws white: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
    assert_eq!(r["normalText"], json!({"aa": true, "aaa": true}), "{r}");
    s.execute("appearance.removeItem", &json!({"index": lower})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!((i32::from(p[0]) - 128).abs() < 3, "the canvas draws half gray: {p:?}");
    // The canvas pixel is rounded to 8 bits, which moves a ratio near half gray by up to 0.04.
    assert!((ratio(&r) - drawn).abs() < 0.05, "{r} against {drawn}");
}

/// A 50 % white fill over an opaque blue one draws over the blue, not over the red art behind it.
#[test]
fn a_translucent_row_draws_over_the_rows_below_it() {
    let mut s = blended_over_red("normal");
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.node(st.selection.objects[0]).unwrap().appearance.items;
    let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "opacity": 50})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!((i32::from(p[0]) - 128).abs() < 3 && p[2] > 252, "the canvas draws the white at 50 % over the blue: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// A path's Normal fill row and type's character paint draw one black alike.
#[test]
fn a_normal_fill_row_and_type_of_one_color_leave_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let t = id(s.execute("text.create", &json!({"x": 80, "y": 60, "text": "Label", "size": 24, "color": "#000000"})).unwrap());
    s.execute("select.set", &json!({"ids": [a, t]})).unwrap();
    let r = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
}

/// In a group that isolates its blending, a selected white Multiply square blends with the blue
/// art the group holds, and with nothing outside it.
#[test]
fn a_blend_mode_in_an_isolating_group_blends_with_the_art_inside_it_only() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let back = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    let square = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"blend": "multiply"})).unwrap();
    s.execute("object.group", &json!({"ids": [back.0, square.0]})).unwrap();
    s.execute("transparency.set", &json!({"isolate": true})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![square];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!(p[0] < 3 && p[2] > 252, "the canvas draws blue: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
    // With the blue art out of the group, the square blends with nothing and draws white over red.
    s.execute("select.set", &json!({"ids": [back.0]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![square];
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let (p, drawn) = canvas_ratio(&s, &r);
    assert!(p[1] > 252, "the canvas draws white: {p:?}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

#[test]
fn template_layers_and_guides_are_left_out_of_the_background() {
    let mut s = session();
    let first = s.doc().unwrap().doc.layers[0].id;
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("layer.setProps", &json!({"id": first.0, "template": true})).unwrap();
    s.execute("layer.new", &json!({"top": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert_eq!(r["against"], "#ffffff", "the artboard, without the template's red or the guide: {r}");
}

#[test]
fn objects_that_draw_one_color_differently_have_no_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [a, b], "color": "#000000"})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let r = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let e = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ: give `opacity` or `color`"), "{e}");
    s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    // A given opacity takes the place of the objects' own.
    let r = s.execute("color.contrast", &json!({"against": "#ffffff", "opacity": 100})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
    // A blend mode that differs is not one a given opacity replaces.
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 100, "blend": "multiply"})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let e = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ: give `color`"), "{e}");
}

/// A 50 % white row over a blue row in an opaque object draws over the blue; an opaque white row in
/// a 50 % object draws over what is behind it, though both draw the white at 50 %.
#[test]
fn a_row_and_an_object_of_one_total_opacity_draw_differently_over_the_rows_below() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.node(st.selection.objects[0]).unwrap().appearance.items;
    let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
    s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "opacity": 50})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ: give `opacity` or `color`"), "{e}");
    // A given background stands for the rows below, so both draw the white at 50 % over it.
    let r = s.execute("color.contrast", &json!({"against": "#000000"})).unwrap();
    let gray = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [0.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "{r}");
}

/// Two squares with the same 50 % white top fill, one over an opaque black fill and one over an
/// opaque white one, draw the white over different rows: no one answer. Over the same lower fill
/// they leave one answer, also when one of them has a lower row with no paint.
#[test]
fn translucent_rows_over_different_lower_rows_have_no_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let square = |s: &mut Session, x: i32, lower: &str| {
        let n = id(s.execute("shape.rectangle", &json!({"x": x, "y": 50, "width": 50, "height": 100})).unwrap());
        s.execute("paint.setFill", &json!({"color": lower})).unwrap();
        s.execute("appearance.addFill", &json!({})).unwrap();
        let st = s.active().unwrap();
        let items = &st.doc.node(vectorcraft_doc::NodeId(n)).unwrap().appearance.items;
        let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
        s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "opacity": 50})).unwrap();
        s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
        n
    };
    let a = square(&mut s, 50, "#000000");
    let b = square(&mut s, 100, "#ffffff");
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ"), "{e}");
    let c = square(&mut s, 150, "#000000");
    s.execute("select.set", &json!({"ids": [a, c]})).unwrap();
    s.execute("color.contrast", &json!({})).unwrap();
    // A row with no paint draws nothing, so it does not count among the rows below.
    s.execute("select.set", &json!({"ids": [c]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.node(vectorcraft_doc::NodeId(c)).unwrap().appearance.items;
    let fills: Vec<usize> = items.iter().enumerate().filter(|(_, i)| i.is_fill()).map(|(k, _)| k).collect();
    let (middle, top) = (fills[fills.len() - 2], fills[fills.len() - 1]);
    s.execute("appearance.setItem", &json!({"ids": [c], "index": top, "color": "#ffffff", "opacity": 50})).unwrap();
    s.execute("appearance.setItem", &json!({"ids": [c], "index": middle, "none": true})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("select.set", &json!({"ids": [a, c]})).unwrap();
    s.execute("color.contrast", &json!({})).unwrap();
}

#[test]
fn a_selected_group_whose_objects_draw_one_color_differently_has_no_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [a, b], "color": "#000000"})).unwrap();
    let g = id(s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let r = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.execute("select.set", &json!({"ids": [g]})).unwrap();
    let e = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ: give `opacity` or `color`"), "{e}");
}

#[test]
fn a_hidden_object_in_a_selected_group_leaves_one_answer() {
    let id = |v: Value| v["id"].as_u64().unwrap();
    // The first and then the second object hidden: the hidden one is neither compared nor measured.
    for first in [false, true] {
        let mut s = session();
        let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
        let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
        s.execute("paint.setFill", &json!({"ids": [a, b], "color": "#000000"})).unwrap();
        let g = id(s.execute("object.group", &json!({"ids": [a, b]})).unwrap());
        s.execute("select.set", &json!({"ids": [if first { a } else { b }]})).unwrap();
        s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
        s.execute("object.hide", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        let r = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
        assert!((ratio(&r) - 21.0).abs() < 1e-6, "first hidden {first}: {r}");
    }
}

#[test]
fn a_given_color_is_drawn_at_a_given_opacity() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff", "opacity": 50})).unwrap();
    let opaque = s.execute("color.contrast", &json!({"color": "#808080", "against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - ratio(&opaque)).abs() < 0.05, "{r} {opaque}");
    assert!(ratio(&r) < 5.0, "{r}");
}

#[test]
fn an_opaque_group_around_one_of_the_objects_leaves_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [a, b], "color": "#000000"})).unwrap();
    s.execute("object.group", &json!({"ids": [b]})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let r = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
}

#[test]
fn objects_in_two_groups_of_one_opacity_answer_against_a_given_color() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let a = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap());
    let b = id(s.execute("shape.rectangle", &json!({"x": 80, "y": 20, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [a, b], "color": "#000000"})).unwrap();
    for o in [a, b] {
        let g = id(s.execute("object.group", &json!({"ids": [o]})).unwrap());
        s.execute("select.set", &json!({"ids": [g]})).unwrap();
        s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    // Sampled, each group is hidden in turn to read what is outside it, so the two differ.
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ: give `opacity` or `color`"), "{e}");
}

#[test]
fn a_hidden_fill_row_has_no_color_to_measure() {
    let mut s = blended_over_red("normal");
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("appearance.setItem", &json!({"index": fill_index(&s), "visible": false})).unwrap();
    let e = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap_err().to_string();
    assert!(e.contains("the active fill or stroke is hidden: give `color`"), "{e}");
    let given = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    assert!((ratio(&given) - 21.0).abs() < 1e-6, "{given}");
}

/// An opacity mask on the object, or on a group around it, sets how much of the color shows at each
/// point, so the color has no one opacity to measure; a disabled mask draws nothing.
#[test]
fn a_color_drawn_through_an_opacity_mask_has_no_one_answer() {
    let mut s = session();
    let rect = |s: &mut Session, color: &str| {
        let id = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"color": color, "ids": [id]})).unwrap();
        id
    };
    let masked = |s: &mut Session, owner: u64, color: &str| {
        let m = rect(s, color);
        s.execute("select.set", &json!({"ids": [owner, m]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [owner]})).unwrap();
    };
    let black = rect(&mut s, "#000000");
    masked(&mut s, black, "#000000");
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("drawn through an opacity mask: give `color`"), "{e}");
    let e = s.execute("color.contrast", &json!({"against": "#ffffff", "opacity": 100})).unwrap_err().to_string();
    assert!(e.contains("drawn through an opacity mask"), "{e}");
    let given = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    assert!((ratio(&given) - 21.0).abs() < 1e-6, "{given}");
    s.execute("transparency.disableOpacityMask", &json!({})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-3, "a disabled mask leaves the black as it is: {r}");
    // A group's mask counts for the objects inside it.
    let mut s = session();
    let inner = rect(&mut s, "#000000");
    let group = s.execute("object.group", &json!({"ids": [inner]})).unwrap()["id"].as_u64().unwrap();
    masked(&mut s, group, "#000000");
    s.execute("select.set", &json!({"ids": [inner]})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("drawn through an opacity mask"), "{e}");
}

/// A background sampled across an edge averages two colors; the answer's `against` is exactly the
/// background the ratio used, so giving it back answers the same ratio.
#[test]
fn the_background_written_back_answers_the_same_ratio() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80.3, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#2050a0"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#808080"})).unwrap();
    let below = s.execute("color.contrast", &json!({})).unwrap();
    let hex = below["against"].as_str().unwrap();
    assert!(hex != "#ffffff" && hex != "#2050a0", "the sample straddles the edge: {below}");
    let given = s.execute("color.contrast", &json!({"against": hex})).unwrap();
    assert!((ratio(&below) - ratio(&given)).abs() < 1e-6, "{below} {given}");
}

/// In a 50 % group, a selected black square over a red one draws over the white outside the group,
/// while a given background stands for the art below every level, so giving the answer's `against`
/// back draws the black at 50 % over it instead.
#[test]
fn a_background_given_back_stands_for_the_art_outside_a_translucent_group() {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    let red = id(s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 120, "height": 120})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let black = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.group", &json!({"ids": [red.0, black.0]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s.active_mut().unwrap().selection.objects = vec![black];
    let below = s.execute("color.contrast", &json!({})).unwrap();
    let (_, drawn) = canvas_ratio(&s, &below);
    assert!((ratio(&below) - drawn).abs() < 0.02, "{below} against {drawn}");
    let hex = below["against"].as_str().unwrap();
    let behind = vectorcraft_color::Color::from_hex(hex).unwrap().to_rgb_uncalibrated();
    let given = s.execute("color.contrast", &json!({"against": hex})).unwrap();
    let want = vectorcraft_color::wcag::contrast_ratio(vectorcraft_color::wcag::over([0.0; 3], 0.5, behind), behind);
    assert!((ratio(&given) - want).abs() < 1e-3, "{given}");
    assert!((ratio(&below) - ratio(&given)).abs() > 0.1, "{below} {given}");
}

#[test]
fn an_object_that_isolates_its_blending_draws_its_fill_as_it_is() {
    let mut s = blended_over_red("multiply");
    s.execute("transparency.set", &json!({"item": null, "isolate": true})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let drawn = drawn_ratio(&s);
    let own = vectorcraft_color::wcag::contrast_ratio(vectorcraft_color::Color::from_hex("#4080c0").unwrap().to_rgb_uncalibrated(), [1.0, 0.0, 0.0]);
    assert!((drawn - own).abs() < 0.02, "the canvas draws the fill unblended: {drawn} against {own}");
    assert!((ratio(&r) - drawn).abs() < 0.02, "{r} against {drawn}");
}

/// A dynamic symbol instance of a white square in front of a red one, at 50 % opacity, with its
/// white square picked.
fn picked_symbol_part() -> Session {
    let mut s = session();
    let id = |v: Value| vectorcraft_doc::NodeId(v["id"].as_u64().unwrap());
    let back = id(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let front = id(s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let inst = id(s.execute("symbol.new", &json!({"name": "Pair", "ids": [back.0, front.0], "type": "dynamic"})).unwrap());
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let st = s.active_mut().unwrap();
    st.selection.objects = vec![inst];
    st.selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![1] });
    s
}

#[test]
fn a_picked_symbol_object_is_measured_against_the_rest_of_its_instance() {
    let mut s = picked_symbol_part();
    let r = s.execute("color.contrast", &json!({"opacity": 100})).unwrap();
    // The red square at 50 % over the artboard's white (#ff7f7f or #ff8080 as the renderer rounds
    // it), not the artboard alone.
    let hex = r["against"].as_str().unwrap();
    let gb = vectorcraft_color::Color::from_hex(hex).unwrap().to_rgb_uncalibrated();
    assert!(hex.starts_with("#ff") && (gb[1] - 0.5).abs() < 0.01 && (gb[2] - 0.5).abs() < 0.01, "{r}");
}

#[test]
fn a_picked_symbol_object_is_measured_with_its_instances_stain() {
    let mut s = picked_symbol_part();
    let inst = s.active().unwrap().selection.symbol_part.as_ref().unwrap().instance;
    // The instance's fill stains its art half way to black.
    s.active_mut().unwrap().selection.symbol_part = None;
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"item": fill_index(&s), "opacity": 50})).unwrap();
    let n = s.doc().unwrap().doc.node(inst).unwrap().clone();
    assert!(vectorcraft_doc::symbol_overrides::stain(&n).is_some_and(|(_, a)| (a - 0.5).abs() < 1e-6));
    s.active_mut().unwrap().selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![1] });
    let r = s.execute("color.contrast", &json!({"against": "#000000", "opacity": 100})).unwrap();
    let want = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [0.0; 3]);
    assert!((ratio(&r) - want).abs() < 1e-3, "white stained half way to black: {r}");
}

#[test]
fn a_picked_symbol_object_is_drawn_at_its_instances_opacity() {
    let mut s = picked_symbol_part();
    let r = s.execute("color.contrast", &json!({"against": "#000000"})).unwrap();
    let gray = vectorcraft_color::wcag::contrast_ratio(vectorcraft_color::wcag::over([1.0; 3], 0.5, [0.0; 3]), [0.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "the instance draws the white at 50 %: {r}");
}

#[test]
fn type_measures_its_characters_without_an_appearance_rows_opacity() {
    let mut s = session();
    s.execute("text.create", &json!({"x": 40, "y": 100, "text": "Label", "size": 24, "color": "#000000"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    let index = s.doc().unwrap().doc.node(s.doc().unwrap().selection.objects[0]).unwrap().appearance.items.len() - 1;
    s.execute("appearance.setItem", &json!({"index": index, "opacity": 0})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-3, "the characters stay opaque black: {r}");
}

#[test]
fn a_gradient_stop_counts_with_its_own_opacity() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    let stops = json!([{"offset": 0, "color": "#000000", "opacity": 0.5}, {"offset": 1, "color": "#ff0000"}]);
    s.execute("paint.setFill", &json!({"gradient": {"stops": stops}})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let gray = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [1.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "the first stop, black at 50 %: {r}");
}

/// Picks the points of the selected gradient mesh inside the box from `a` to `b` with the Lasso tool.
fn pick_points(s: &mut Session, a: (f64, f64), b: (f64, f64)) {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let v = ViewInfo::default();
    s.select_tool("lasso", v).unwrap();
    let pts = [(a.0, a.1), (b.0, a.1), (b.0, b.1), (a.0, b.1), (a.0, b.1)];
    for (i, (x, y)) in pts.iter().enumerate() {
        let kind = match i {
            0 => PointerKind::Down,
            4 => PointerKind::Up,
            _ => PointerKind::Drag,
        };
        s.pointer(&PointerEvent::new(kind, *x, *y), v).unwrap();
    }
}

#[test]
fn a_picked_mesh_point_counts_with_its_own_opacity() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("object.mesh.create", &json!({"rows": 1, "cols": 1})).unwrap();
    // The top-left point at 50 %, the top-right one opaque.
    pick_points(&mut s, (40.0, 40.0), (60.0, 60.0));
    assert_eq!(s.fill_mesh_points().first().map(|(_, v)| v.len()), Some(1), "one point is picked");
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    let gray = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [1.0; 3]);
    assert!((ratio(&r) - gray).abs() < 1e-3, "the picked point, black at 50 %: {r}");
    pick_points(&mut s, (140.0, 40.0), (160.0, 60.0));
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-3, "the opaque point: {r}");
    // Both picked: one color at two opacities has no one answer, unless `opacity` is given.
    pick_points(&mut s, (40.0, 40.0), (160.0, 60.0));
    let both = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(both.contains("the selected objects' colors differ: give `opacity` or `color`"), "{both}");
    assert!(s.execute("color.contrast", &json!({"opacity": 100})).is_ok());
}

#[test]
fn below_needs_a_selection_and_bad_params_are_errors() {
    let mut s = session();
    let none = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(none.contains("nothing selected"), "{none}");
    // A given background needs no selection: the proxy shows the defaults for new art.
    assert!(s.execute("color.contrast", &json!({"against": "#ffffff"})).is_ok());
    for bad in [
        json!({"color": "#000000", "against": "behind"}),
        json!({"color": "#000000", "against": 3}),
        json!({"color": "#00000080", "against": "#ffffff"}),
        json!({"color": "nope", "against": "#ffffff"}),
        json!({"color": "#000000", "against": "#ffffff", "opacity": 101}),
        json!({"color": "#000000", "against": "#ffffff", "opacity": -1}),
        json!({"color": "#000000", "against": "#ffffff", "opacity": "50"}),
    ] {
        assert!(s.execute("color.contrast", &bad).is_err(), "{bad}");
    }
}

#[test]
fn a_fill_of_none_has_no_color_to_measure() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("has no color"), "{e}");
    assert!(s.execute("color.contrast", &json!({"color": "#000000"})).is_ok());
}
