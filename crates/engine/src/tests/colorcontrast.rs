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
    assert!(e.contains("the selected objects' colors differ: give `color`"), "{e}");
    s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff"})).unwrap();
    // A given opacity takes the place of the objects' own.
    let r = s.execute("color.contrast", &json!({"against": "#ffffff", "opacity": 100})).unwrap();
    assert!((ratio(&r) - 21.0).abs() < 1e-6, "{r}");
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
    assert!(e.contains("the selected objects' colors differ: give `color`"), "{e}");
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
    assert!(e.contains("the selected objects' colors differ: give `color`"), "{e}");
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
    assert!((4.5..4.51).contains(&got), "{fix:?} draws {drawn:?}: {got}");
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
    assert!((4.5..4.51).contains(&ratio(&back)), "{back}");
    let want = oklch_hue(&vectorcraft_color::Color::from_hex("#fdc96e").unwrap());
    assert!((oklch_hue(&aa) - want).abs() < 1.0, "{} {want}", oklch_hue(&aa));
    assert!(matches!(aa, vectorcraft_color::Color::Rgb { .. }), "an RGB color answers RGB: {aa:?}");
    // White on #355F78 is 6.87:1 and black 3.05:1, so no color reaches AAA's 7:1.
    assert!(r["fix"]["aaa"].is_null(), "{r}");
    let large = s.execute("color.contrast", &json!({"color": "#fdc96e", "against": "#355f78", "for": "largeText"})).unwrap();
    let aaa = s.execute("color.contrast", &json!({"color": large["fix"]["aaa"], "against": "#355f78"})).unwrap();
    assert!((4.5..4.51).contains(&ratio(&aaa)), "large text AAA is AA's 4.5:1 for normal text: {aaa}");
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
    assert!((3.0..3.01).contains(&ratio(&back)), "{back}");
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
    assert!((3.0..3.01).contains(&ratio(&back)), "{back}");
}

#[test]
fn for_takes_only_the_three_categories() {
    let mut s = Session::new();
    for bad in [json!("NormalText"), json!("aa"), json!(1), json!(true)] {
        let e = s.execute("color.contrast", &json!({"color": "#000000", "against": "#ffffff", "for": bad})).unwrap_err().to_string();
        assert!(e.contains("`for` must be one of normalText, largeText, graphics"), "{e}");
    }
}
