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

/// While the Live Paint Bucket is the active tool, the proxy shows the color it paints with, so the
/// selected objects' opacities and blend modes do not apply, and objects drawn differently are no
/// error.
#[test]
fn the_live_paint_bucket_color_is_measured_without_the_selections_drawing() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50, "blend": "multiply"})).unwrap();
    let selection = ratio(&s.execute("color.contrast", &json!({})).unwrap());
    s.select_tool("livePaintBucket", ViewInfo::default()).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    let bucket = s.execute("color.contrast", &json!({})).unwrap();
    assert!(ratio(&bucket) > selection, "the bucket's black is drawn opaque: {bucket} against {selection}");
    assert!((ratio(&bucket) - 21.0).abs() < 1e-3, "{bucket}");
    // A second selected object drawn at another opacity does not make the answer ambiguous.
    s.select_tool("selection", ViewInfo::default()).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.select_tool("livePaintBucket", ViewInfo::default()).unwrap();
    let both = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&both) - ratio(&bucket)).abs() < 1e-3, "{both}");
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
    // The fix is the color to store: the stain mixes it half way to black again, and the result
    // just meets AA against white.
    let r = s.execute("color.contrast", &json!({"against": "#ffffff", "opacity": 100})).unwrap();
    assert_eq!(r["normalText"]["aa"], false, "{r}");
    let fix = fix_color(&r["fix"]["aa"]);
    let drawn = fix.lerp(&vectorcraft_color::Color::BLACK, 0.5).to_rgb_uncalibrated();
    let got = vectorcraft_color::wcag::contrast_ratio(drawn, [1.0; 3]);
    assert!((4.5..4.57).contains(&got), "{fix:?} draws {drawn:?}: {got}");
}

/// A pick for a picked symbol object with a fill of None puts a solid color there, which the
/// instance's stain mixes like any other, so the Color Wheel's contrast lines measure it stained.
#[test]
fn a_pick_for_a_picked_symbol_object_with_no_fill_is_measured_with_the_stain() {
    let mut s = picked_symbol_part();
    let inst = s.active().unwrap().selection.symbol_part.as_ref().unwrap().instance;
    s.active_mut().unwrap().selection.symbol_part = None;
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("transparency.set", &json!({"item": fill_index(&s), "opacity": 50})).unwrap();
    s.active_mut().unwrap().selection.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![1] });
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert!(s.execute("color.contrast", &json!({})).is_err(), "no color to measure");
    let backdrop = crate::cmd::contrast_pick_backdrop(&s, &json!({"against": "#000000", "opacity": 100})).unwrap();
    let cms = vectorcraft_color::cms::active();
    let white = backdrop.ratio(backdrop.shown(&cms, &vectorcraft_color::Color::rgb(1.0, 1.0, 1.0)));
    let want = vectorcraft_color::wcag::contrast_ratio([0.5; 3], [0.0; 3]);
    assert!((white - want).abs() < 1e-3, "a white pick stained half way to black: {white} against {want}");
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

fn fix_color(v: &Value) -> vectorcraft_color::Color {
    crate::cmd::color_value(v).unwrap_or_else(|| panic!("not a color: {v}"))
}

fn oklch_hue(c: &vectorcraft_color::Color) -> f64 {
    let [_, a, b] = vectorcraft_color::libraries::to_oklab(c);
    b.atan2(a).to_degrees()
}

#[test]
fn a_failing_color_answers_the_closest_passing_color_of_its_hue() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#fdc96e", "against": "#355f78"})).unwrap();
    assert_eq!(r["for"], "normalText");
    let aa = fix_color(&r["fix"]["aa"]);
    let back = s.execute("color.contrast", &json!({"color": r["fix"]["aa"], "against": "#355f78"})).unwrap();
    assert!((4.5..4.57).contains(&ratio(&back)), "{back}");
    let want = oklch_hue(&vectorcraft_color::Color::from_hex("#fdc96e").unwrap());
    assert!((oklch_hue(&aa) - want).abs() < 1.0, "{} {want}", oklch_hue(&aa));
    assert!(matches!(aa, vectorcraft_color::Color::Rgb { .. }), "an RGB color answers RGB: {aa:?}");
    // White on #355F78 is 6.87:1 and black 3.05:1, so no color reaches AAA's 7:1.
    assert!(r["fix"]["aaa"].is_null(), "{r}");
    let large = s.execute("color.contrast", &json!({"color": "#fdc96e", "against": "#355f78", "for": "largeText"})).unwrap();
    let aaa = s.execute("color.contrast", &json!({"color": large["fix"]["aaa"], "against": "#355f78"})).unwrap();
    assert!((4.5..4.57).contains(&ratio(&aaa)), "large text AAA is AA's 4.5:1 for normal text: {aaa}");
}

/// The canvas paints a color with 8 bits per channel, so a fix passes once rounded that way too:
/// #777777 on white is 4.48:1, a gray of about 118.7/255 meets 4.5:1 but paints as #777777 again,
/// and the fix has to paint as #767676 (4.54:1) or darker.
#[test]
fn a_fix_passes_as_the_canvas_paints_it() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#777777", "against": "#ffffff"})).unwrap();
    let fix = fix_color(&r["fix"]["aa"]);
    let painted = fix.to_rgb_uncalibrated().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0);
    let got = vectorcraft_color::wcag::contrast_ratio(painted, [1.0; 3]);
    assert!(got >= 4.5, "{fix:?} paints as {painted:?}: {got}");
    // The fix is the closest one: the gray one 8-bit step lighter fails.
    let lighter = painted.map(|v| (v + 1.0 / 255.0).min(1.0));
    assert!(vectorcraft_color::wcag::contrast_ratio(lighter, [1.0; 3]) < 4.5, "{fix:?} paints as {painted:?}");
    let back = s.execute("color.contrast", &json!({"color": r["fix"]["aa"], "against": "#ffffff"})).unwrap();
    assert!((4.5..4.57).contains(&ratio(&back)), "{back}");
}

/// The backdrop shows 100 % K alone as `color.contrast` measures it under each Appearance of Black,
/// so the Color Wheel's contrast lines use the same color.
#[test]
fn the_backdrop_shows_black_as_the_appearance_of_black_setting_does() {
    let k = vectorcraft_color::Color::cmyk(0.0, 0.0, 0.0, 1.0);
    let cms = vectorcraft_color::cms::active();
    let mut s = Session::new();
    let p = json!({"color": serde_json::to_value(k).unwrap(), "against": "#ffffff"});
    let (accurate, backdrop) = crate::cmd::contrast_measure(&s, &p).unwrap();
    assert_ne!(backdrop.shown(&cms, &k), [0.0; 3]);
    s.prefs.black_on_screen = "rich".into();
    let (rich, backdrop) = crate::cmd::contrast_measure(&s, &p).unwrap();
    assert_eq!(backdrop.shown(&cms, &k), [0.0; 3]);
    assert!(ratio(&rich) > ratio(&accurate), "{rich} {accurate}");
}

/// A translucent fix also meets the minimum with the color, the opacity, the background and the
/// composited pixel rounded to 8 bits per channel.
#[test]
fn a_translucent_fix_passes_at_eight_bits() {
    let eight = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
    let at_eight_bits = |fix: &vectorcraft_color::Color, opacity: f32, against: f32| {
        let (a, b) = (eight(opacity), eight(against));
        let pixel = fix.to_rgb_uncalibrated().map(|c| eight(eight(c) * a + b * (1.0 - a)));
        vectorcraft_color::wcag::contrast_ratio(pixel, [b; 3])
    };
    let mut s = Session::new();
    // A gray can pass only before rounding: #808080 at 91.1 % over black measures 4.53:1, and at
    // 232 / 255 its pixel is #747474 (4.49:1).
    let r = s.execute("color.contrast", &json!({"color": "#707070", "against": "#000000", "opacity": 91.1})).unwrap();
    let fix = fix_color(&r["fix"]["aa"]);
    assert!(at_eight_bits(&fix, 0.911, 0.0) >= 4.5, "{fix:?}");
    // #040404 at 195 / 255 over a gray of 137.51 / 255 measures 4.5002:1, and over 138 / 255 it is
    // 4.4965:1.
    let bg = 137.51 / 255.0;
    let p = json!({"color": "#303030", "against": [bg, bg, bg], "opacity": 19500.0 / 255.0});
    let r = s.execute("color.contrast", &p).unwrap();
    let fix = fix_color(&r["fix"]["aa"]);
    assert!(at_eight_bits(&fix, 195.0 / 255.0, bg) >= 4.5, "{fix:?}");
    // Black at 41.7 % over white measures 3.008:1, and its pixel at 8 bits is 2.995:1, so no gray
    // at that opacity gets a fix.
    let p = json!({"color": "#202020", "against": "#ffffff", "opacity": 41.7, "for": "graphics"});
    let r = s.execute("color.contrast", &p).unwrap();
    assert_eq!(r["graphics"]["aa"], false, "{r}");
    assert!(r["fix"]["aa"].is_null(), "{r}");
}

#[test]
fn a_passing_color_has_no_fix_and_graphics_have_no_aaa_fix() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    assert_eq!(r["fix"], json!({"aa": null, "aaa": null}));
    let g = s.execute("color.contrast", &json!({"color": "#cccccc", "against": "#ffffff", "for": "graphics"})).unwrap();
    assert_eq!(g["for"], "graphics");
    let fix = g["fix"].as_object().unwrap();
    assert_eq!(fix.keys().collect::<Vec<_>>(), ["aa"], "{g}");
    let back = s.execute("color.contrast", &json!({"color": g["fix"]["aa"], "against": "#ffffff"})).unwrap();
    assert!((3.0..3.04).contains(&ratio(&back)), "{back}");
    let large = s.execute("color.contrast", &json!({"color": "#fdc96e", "against": "#355f78", "for": "largeText"})).unwrap();
    assert!(large["fix"]["aa"].is_null(), "4.49:1 meets large text AA: {large}");
    assert!(!large["fix"]["aaa"].is_null(), "{large}");
}

#[test]
fn a_cmyk_color_answers_a_cmyk_fix() {
    let mut s = Session::new();
    let r = s.execute("color.contrast", &json!({"color": {"c": 0, "m": 40, "y": 70, "k": 20}, "against": "#355f78"})).unwrap();
    assert_eq!(r["normalText"]["aa"], false, "{r}");
    let fix = fix_color(&r["fix"]["aa"]);
    assert!(matches!(fix, vectorcraft_color::Color::Cmyk { .. }), "{fix:?}");
    let back = s.execute("color.contrast", &json!({"color": r["fix"]["aa"], "against": "#355f78"})).unwrap();
    assert!(ratio(&back) >= 4.5, "{back}");
}

#[test]
fn a_fix_is_drawn_at_the_same_opacity_and_none_passes_out_of_reach() {
    let mut s = Session::new();
    // #808080 at 50 % over white draws as a light gray: black at 50 % draws 0.5 gray, 3.98:1, which
    // meets large text AA (3:1) but not AAA (4.5:1).
    let p = json!({"color": "#808080", "against": "#ffffff", "opacity": 50, "for": "largeText"});
    let r = s.execute("color.contrast", &p).unwrap();
    assert_eq!(r["largeText"]["aa"], false, "{r}");
    assert!(r["fix"]["aaa"].is_null(), "no gray at 50 % reaches 4.5:1: {r}");
    let back = s.execute("color.contrast", &json!({"color": r["fix"]["aa"], "against": "#ffffff", "opacity": 50})).unwrap();
    assert!((3.0..3.04).contains(&ratio(&back)), "{back}");
}

/// A square with a 50 % top fill row over locked orange, with and without a lower fill row and in a
/// Screen group or not: isolating the object's blending leaves what a translucent Normal row draws
/// as it is, on the canvas and in the answer, for each blend mode of the object.
#[test]
fn isolation_leaves_a_translucent_normal_row_as_it_draws() {
    let id = |v: Value| v["id"].as_u64().unwrap();
    for blend in ["normal", "multiply", "screen", "difference", "overlay"] {
        for fill in ["#808080", "#ffffff", "#2040a0"] {
            for lower in [false, true] {
                let mut s = session();
                s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 160, "height": 160})).unwrap();
                s.execute("paint.setFill", &json!({"color": "#ff8000"})).unwrap();
                s.execute("object.lock", &json!({})).unwrap();
                let n = id(s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
                s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
                if lower {
                    s.execute("appearance.addFill", &json!({})).unwrap();
                }
                let st = s.active().unwrap();
                let items = &st.doc.node(vectorcraft_doc::NodeId(n)).unwrap().appearance.items;
                let top = items.iter().rposition(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
                s.execute("appearance.setItem", &json!({"index": top, "color": fill, "opacity": 50})).unwrap();
                s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
                s.execute("transparency.set", &json!({"item": null, "blend": blend})).unwrap();
                if lower {
                    s.execute("object.group", &json!({})).unwrap();
                    s.execute("transparency.set", &json!({"blend": "screen"})).unwrap();
                    s.execute("select.set", &json!({"ids": [n]})).unwrap();
                }
                let plain = s.execute("color.contrast", &json!({})).unwrap();
                let (p, drawn) = canvas_ratio(&s, &plain);
                s.execute("transparency.set", &json!({"item": null, "isolate": true})).unwrap();
                let isolated = s.execute("color.contrast", &json!({})).unwrap();
                let (q, _) = canvas_ratio(&s, &isolated);
                let case = format!("{blend} {fill} lower row and group: {lower}");
                assert_eq!(p, q, "the canvas draws the same: {case}");
                assert!((ratio(&plain) - ratio(&isolated)).abs() < 1e-6, "{case}: {plain} {isolated}");
                assert!((ratio(&plain) - drawn).abs() < 0.02, "{case}: {plain} against {drawn}");
            }
        }
    }
}

/// Two squares over locked orange, each with a 50 % white top fill over a blue Multiply fill, one
/// isolating its blending: the blue multiplies with the orange in one and draws over nothing in the
/// other, so the white draws over different rows and there is no one answer. A given background
/// stands for the rows below and answers.
#[test]
fn a_translucent_row_over_a_blending_row_in_objects_of_different_isolation_has_no_one_answer() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 180, "height": 180})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff8000"})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let square = |s: &mut Session, x: i32, isolate: bool| {
        let n = id(s.execute("shape.rectangle", &json!({"x": x, "y": 50, "width": 50, "height": 100})).unwrap());
        s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
        s.execute("appearance.addFill", &json!({})).unwrap();
        let st = s.active().unwrap();
        let items = &st.doc.node(vectorcraft_doc::NodeId(n)).unwrap().appearance.items;
        let top = items.iter().rposition(|i| i.is_fill()).unwrap();
        let lower = items.iter().position(|i| i.is_fill()).unwrap();
        s.execute("appearance.setItem", &json!({"index": top, "color": "#ffffff", "opacity": 50})).unwrap();
        s.execute("appearance.setItem", &json!({"index": lower, "blend": "multiply"})).unwrap();
        s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
        if isolate {
            s.execute("transparency.set", &json!({"item": null, "isolate": true})).unwrap();
        }
        n
    };
    let a = square(&mut s, 30, false);
    let b = square(&mut s, 120, true);
    s.execute("select.set", &json!({"ids": [a]})).unwrap();
    let ra = ratio(&s.execute("color.contrast", &json!({})).unwrap());
    s.execute("select.set", &json!({"ids": [b]})).unwrap();
    let rb = ratio(&s.execute("color.contrast", &json!({})).unwrap());
    assert!((ra - rb).abs() > 0.1, "the white draws over different rows: {ra} {rb}");
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("the selected objects' colors differ"), "{e}");
    s.execute("color.contrast", &json!({"against": "#000000"})).unwrap();
    // A blending row at no opacity does not show, so the isolation no longer changes the answer.
    for n in [a, b] {
        let st = s.active().unwrap();
        let lower = st.doc.node(vectorcraft_doc::NodeId(n)).unwrap().appearance.items.iter().position(|i| i.is_fill()).unwrap();
        s.execute("appearance.setItem", &json!({"ids": [n], "index": lower, "opacity": 0})).unwrap();
    }
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("color.contrast", &json!({})).unwrap();
}

/// A Difference square centered on the edge between locked black and white art: the art behind it
/// is not one color where it is read, so no one color stands for it; a given background answers,
/// and over one color the background is read as before.
#[test]
fn a_blend_mode_over_art_of_several_colors_has_no_one_answer() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 20, "width": 80, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    // Without the default stroke, which would cover the edge.
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"blend": "difference"})).unwrap();
    let e = s.execute("color.contrast", &json!({})).unwrap_err().to_string();
    assert!(e.contains("is not one color where it is read") && e.contains("give `against`"), "{e}");
    let given = s.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
    assert!((ratio(&given) - 21.0).abs() < 1e-6, "white minus white is black: {given}");
    // In Normal mode the color is drawn as it is, over the background read as one color.
    s.execute("transparency.set", &json!({"blend": "normal"})).unwrap();
    s.execute("color.contrast", &json!({})).unwrap();
    // Moved over the white art alone, the square blends with one color again.
    s.execute("transparency.set", &json!({"blend": "difference"})).unwrap();
    s.execute("object.move", &json!({"dx": 40, "dy": 0})).unwrap();
    let r = s.execute("color.contrast", &json!({})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-3, "white minus white is black: {r}");
}

/// A Difference square over a white square in a 50 % group that isolates its blending, over the
/// edge between black and white art: the square blends only with the white inside the group, so
/// the art of two colors outside it, which the group draws over in Normal mode, leaves one answer.
#[test]
fn a_blend_mode_inside_an_isolating_group_answers_over_art_of_several_colors() {
    let mut s = session();
    let id = |v: Value| v["id"].as_u64().unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 80, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 100, "y": 20, "width": 80, "height": 160})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let back = id(s.execute("shape.rectangle", &json!({"x": 60, "y": 60, "width": 80, "height": 80})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let square = id(s.execute("shape.rectangle", &json!({"x": 70, "y": 70, "width": 60, "height": 60})).unwrap());
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"blend": "difference"})).unwrap();
    s.execute("object.group", &json!({"ids": [back, square]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50, "isolate": true})).unwrap();
    s.execute("select.set", &json!({"ids": [square]})).unwrap();
    s.execute("color.contrast", &json!({})).unwrap();
}

/// With a fill of None, what a pick would be measured against is still read: the Color Wheel's
/// contrast lines show where a solid color in its place passes.
#[test]
fn a_pick_for_a_fill_of_none_is_measured_against_the_background() {
    let mut s = blended_over_red("normal");
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert!(s.execute("color.contrast", &json!({})).is_err(), "no color to measure");
    let backdrop = crate::cmd::contrast_pick_backdrop(&s, &json!({})).unwrap();
    let cms = vectorcraft_color::cms::active();
    let black = backdrop.ratio(backdrop.shown(&cms, &vectorcraft_color::Color::rgb(0.0, 0.0, 0.0)));
    let want = vectorcraft_color::wcag::contrast_ratio([0.0; 3], [1.0, 0.0, 0.0]);
    assert!((black - want).abs() < 1e-3, "black over the red art: {black} against {want}");
    assert_eq!(backdrop.against_hex(), "#ff0000");
}

#[test]
fn for_takes_only_the_three_categories() {
    let mut s = Session::new();
    for bad in [json!("NormalText"), json!("aa"), json!(1), json!(true)] {
        let e = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff", "for": bad})).unwrap_err().to_string();
        assert!(e.contains("`for` must be one of normalText, largeText, graphics"), "{e}");
    }
}
