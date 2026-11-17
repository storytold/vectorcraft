//! Exports of objects alone (Export Selection, Export for Screens by selection or by layer, Asset
//! Export, a CSS rule's picture and `selectedOnly`) draw the objects as the canvas draws them: with
//! the clipping paths, opacity masks, effects, opacity and blend modes of the layers and groups that
//! hold them.

use serde_json::{Value, json};
use vectorcraft_geom::Rect;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    s
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> u64 {
    v["id"].as_u64().unwrap_or_else(|| panic!("no id in {v}"))
}

/// A red 300 pt square at (100, 100) clipped by a 200 pt circle at (150, 150): (square, clip group).
fn clipped(s: &mut Session) -> (u64, u64) {
    let square = id(&run(s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 300, "height": 300})));
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    let circle = id(&run(s, "shape.ellipse", json!({"x": 150, "y": 150, "width": 200, "height": 200})));
    let group = id(&run(s, "object.clippingMask.make", json!({"ids": [circle, square]})));
    (square, group)
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap_or_else(|| panic!("no dataBase64 in {v}"))).unwrap()
}

fn rect_of(v: &Value) -> Rect {
    let n = |i: usize| v[i].as_f64().unwrap_or_else(|| panic!("bad bounds {v}"));
    Rect::new(n(0), n(1), n(0) + n(2), n(1) + n(3))
}

/// What the canvas draws of the whole document over `r`, at 1 px per point.
fn canvas(s: &Session, r: Rect) -> image::RgbaImage {
    let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, r, 1.0, false);
    image::load_from_memory(&img.to_png().unwrap()).unwrap().to_rgba8()
}

/// The pixels of `png` that differ from `want` by more than 64 in a channel (anti-aliased edges
/// differ by less), and every pixel when the sizes differ.
fn differing(png: &[u8], want: &image::RgbaImage) -> usize {
    let got = image::load_from_memory(png).unwrap().to_rgba8();
    if got.dimensions() != want.dimensions() {
        return (want.width() * want.height()) as usize;
    }
    got.pixels().zip(want.pixels()).filter(|(a, b)| a.0.iter().zip(b.0.iter()).any(|(x, y)| x.abs_diff(*y) > 64)).count()
}

/// Export Selection of what is selected, as a PNG, matches the canvas over its crop: fewer than 200
/// pixels differ (anti-aliased edges), or 200 times `edges` with [`selection_png_matches_canvas_with`].
fn selection_png_matches_canvas(s: &mut Session, want: Rect, what: &str) {
    selection_png_matches_canvas_with(s, want, what, 1);
}

fn selection_png_matches_canvas_with(s: &mut Session, want: Rect, what: &str, edges: usize) {
    let r = run(s, "document.exportSelection", json!({"format": "png", "scale": 1}));
    let got = rect_of(&r["bounds"]);
    assert!((got.x0 - want.x0).abs() < 1.5 && (got.y0 - want.y0).abs() < 1.5, "{what}: crop {got:?}, want {want:?}");
    assert!((got.width() - want.width()).abs() < 3.0 && (got.height() - want.height()).abs() < 3.0, "{what}: crop {got:?}, want {want:?}");
    let off = differing(&b64(&r), &canvas(s, got));
    assert!(off < 200 * edges, "{what}: {off} pixels differ from the canvas");
}

#[test]
fn export_selection_of_a_clipped_object_keeps_its_clip() {
    let mut s = session();
    let (square, _) = clipped(&mut s);
    run(&mut s, "select.set", json!({"ids": [square]}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 350.0, 350.0), "the square alone");
    let svg = b64(&run(&mut s, "document.exportSelection", json!({"format": "svg"})));
    let text = String::from_utf8_lossy(&svg);
    assert!(text.contains("<clipPath") && text.contains("clip-path"), "the SVG clips the square: {text}");
    // Opened again, the SVG draws what the canvas draws.
    let mut t = Session::new();
    run(&mut t, "document.open", json!({"name": "sel.svg", "dataBase64": vectorcraft_format::base64_encode(&svg)}));
    let board = t.doc().unwrap().doc.artboards[0].rect;
    let reopened = canvas(&t, board);
    let mut png = vec![];
    reopened.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let off = differing(&png, &canvas(&s, Rect::new(150.0, 150.0, 350.0, 350.0)));
    assert!(off < 200, "the reopened SVG: {off} pixels differ from the canvas");
}

#[test]
fn export_selection_of_a_clipping_layer_keeps_the_layer_clip() {
    let mut s = session();
    let layer = id(&run(&mut s, "layer.new", json!({"name": "L"})));
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 300, "height": 300}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "shape.ellipse", json!({"x": 150, "y": 150, "width": 200, "height": 200}));
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer}));
    run(&mut s, "select.all", json!({}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 350.0, 350.0), "a clipping layer's objects");
}

#[test]
fn export_selection_keeps_the_opacity_effects_and_mask_of_the_groups_around_it() {
    let mut s = session();
    let (square, group) = clipped(&mut s);
    let other = id(&run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})));
    let outer = id(&run(&mut s, "object.group", json!({"ids": [group, other]})));
    run(&mut s, "transparency.set", json!({"ids": [outer], "opacity": 50}));
    run(&mut s, "select.set", json!({"ids": [square]}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 350.0, 350.0), "inside a group at 50%");
    // A drop shadow on the clip group reaches past its clip.
    run(&mut s, "effect.apply", json!({"ids": [group], "effect": "stylize.dropShadow"}));
    let r = run(&mut s, "document.exportSelection", json!({"format": "png", "scale": 1}));
    let got = rect_of(&r["bounds"]);
    assert!(got.x1 > 352.0 && got.y1 > 352.0, "the crop takes in the clip group's shadow: {got:?}");
    assert!(differing(&b64(&r), &canvas(&s, got)) < 200, "with the shadow");
    run(&mut s, "edit.undo", json!({}));
    // An opacity mask on the clip group.
    let mask = id(&run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 300, "height": 300})));
    run(
        &mut s,
        "paint.setFill",
        json!({"gradient": {"kind": "linear", "stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]}}),
    );
    run(&mut s, "transparency.makeOpacityMask", json!({"ids": [mask, group]}));
    run(&mut s, "select.set", json!({"ids": [square]}));
    let r = run(&mut s, "document.exportSelection", json!({"format": "png", "scale": 1}));
    let got = rect_of(&r["bounds"]);
    assert!(differing(&b64(&r), &canvas(&s, got)) < 200, "with an opacity mask on the clip group");
}

#[test]
fn export_selection_across_clip_groups_and_of_whole_ones() {
    let mut s = session();
    let (a, ga) = clipped(&mut s);
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 400, "y": 400, "width": 90, "height": 90})));
    let c = id(&run(&mut s, "shape.rectangle", json!({"x": 420, "y": 420, "width": 50, "height": 50})));
    let gb = id(&run(&mut s, "object.clippingMask.make", json!({"ids": [c, b]})));
    // A member of each clip group: each keeps its own clip.
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 470.0, 470.0), "a member of each of two clip groups");
    // The clip groups themselves.
    run(&mut s, "select.set", json!({"ids": [ga, gb]}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 470.0, 470.0), "two clip groups");
    // Nested clip groups: the inner group's member is clipped by both, cropped to the outer clip.
    let strip = id(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 200, "width": 500, "height": 100})));
    run(&mut s, "object.clippingMask.make", json!({"ids": [strip, ga]}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    selection_png_matches_canvas(&mut s, Rect::new(0.0, 200.0, 500.0, 300.0), "a member of nested clip groups");
}

#[test]
fn a_selected_clipping_path_and_a_blend_object_export_on_their_own() {
    let mut s = session();
    let (square, group) = clipped(&mut s);
    let circle = s.doc().unwrap().doc.node(vectorcraft_doc::NodeId(group)).unwrap().children().unwrap()[0].id.0;
    assert_ne!(circle, square);
    // The clipping path alone: the clip group's other art stays out.
    run(&mut s, "select.set", json!({"ids": [circle]}));
    let r = run(&mut s, "document.exportSelection", json!({"format": "png", "scale": 1}));
    let img = image::load_from_memory(&b64(&r)).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (200, 200), "cropped to the clipping path");
    assert!(!img.pixels().any(|p| p[0] > 200 && p[1] < 80 && p[3] > 0), "the clipped square is left out");
    // An object of a blend goes in on its own, without the blend's steps.
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 60})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 400, "y": 400, "width": 60, "height": 60})));
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    run(&mut s, "object.blend.make", json!({}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    let r = run(&mut s, "document.exportSelection", json!({"format": "png", "scale": 1}));
    let got = rect_of(&r["bounds"]);
    assert!(got.x1 < 90.0 && got.y1 < 90.0, "the blend object's own bounds: {got:?}");
    let img = image::load_from_memory(&b64(&r)).unwrap().to_rgba8();
    let centre = img.get_pixel(img.width() / 2, img.height() / 2);
    assert!(centre[0] > 200 && centre[3] > 200, "the object is drawn: {centre:?}");
}

#[test]
fn a_css_picture_of_a_clipped_object_is_clipped() {
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let star = id(&run(&mut s, "shape.star", json!({"cx": 250, "cy": 250, "radius1": 150, "radius2": 70, "points": 5})));
    let circle = id(&run(&mut s, "shape.ellipse", json!({"x": 200, "y": 200, "width": 100, "height": 100})));
    run(&mut s, "object.clippingMask.make", json!({"ids": [circle, star]}));
    let r = run(&mut s, "css.export", json!({"ids": [star], "rasterize": true, "unnamed": true}));
    let image = r["images"].as_array().and_then(|i| i.first()).unwrap_or_else(|| panic!("no picture: {r}"));
    let png = b64(image);
    let size = image::load_from_memory(&png).unwrap().to_rgba8().dimensions();
    assert_eq!(size, (100, 100), "the picture is cropped to the clip");
    assert!(differing(&png, &canvas(&s, Rect::new(200.0, 200.0, 300.0, 300.0))) < 100, "the picture matches the canvas");
}

#[test]
fn an_asset_on_a_hidden_layer_is_drawn_clipped() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    let (square, _) = clipped(&mut s);
    let asset = run(&mut s, "assets.add", json!({"ids": [square]}))["assets"][0].as_u64().unwrap();
    run(&mut s, "layer.setProps", json!({"id": layer, "visible": false}));
    let r = run(&mut s, "assets.export", json!({"assets": [asset], "formats": [{"format": "png", "scale": 1}]}));
    let img = image::load_from_memory(&b64(&r["files"][0])).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (200, 200), "cropped to the clip");
    let (centre, corner) = (img.get_pixel(100, 100), img.get_pixel(3, 3));
    assert!(centre[0] > 200 && centre[3] > 200, "the square is drawn: {centre:?}");
    assert_eq!(corner[3], 0, "outside the circle stays clear: {corner:?}");
}

#[test]
fn export_for_screens_and_assets_by_selection_keep_the_clip() {
    let mut s = session();
    let (square, _) = clipped(&mut s);
    run(&mut s, "select.set", json!({"ids": [square]}));
    let r = run(&mut s, "document.exportForScreens", json!({"selection": "one", "formats": [{"format": "png", "scale": 1}, {"format": "svg"}]}));
    let want = canvas(&s, Rect::new(150.0, 150.0, 350.0, 350.0));
    let png = r["files"].as_array().unwrap().iter().find(|f| f["name"].as_str().is_some_and(|n| n.ends_with(".png"))).unwrap();
    assert!(differing(&b64(png), &want) < 200, "Export for Screens by selection: {}", png["name"]);
    let svg = r["files"].as_array().unwrap().iter().find(|f| f["name"].as_str().is_some_and(|n| n.ends_with(".svg"))).unwrap();
    let text = String::from_utf8(b64(svg)).unwrap();
    assert!(text.contains("<clipPath"), "the SVG clips the square: {text}");
    let asset = run(&mut s, "assets.add", json!({"ids": [square]}))["assets"][0].as_u64().unwrap();
    let list = run(&mut s, "assets.list", json!({}));
    let bounds = rect_of(&list["assets"][0]["bounds"]);
    assert!((bounds.width() - 200.0).abs() < 3.0, "the asset's crop is its clip: {bounds:?}");
    let r = run(&mut s, "assets.export", json!({"assets": [asset], "formats": [{"format": "png", "scale": 1}]}));
    assert!(differing(&b64(&r["files"][0]), &want) < 200, "Asset Export: {}", r["files"][0]["name"]);
}

/// The red pixels of `png`.
fn red(png: &[u8]) -> usize {
    image::load_from_memory(png).unwrap().to_rgba8().pixels().filter(|p| p[0] > 200 && p[1] < 80 && p[3] > 200).count()
}

#[test]
fn a_selection_on_a_non_printing_layer_exports_to_pdf() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    let (square, _) = clipped(&mut s);
    run(&mut s, "layer.setProps", json!({"id": layer, "printable": false}));
    run(&mut s, "select.set", json!({"ids": [square]}));
    let pdf = b64(&run(&mut s, "document.exportSelection", json!({"format": "pdf", "createLayers": false, "preserveEditing": false})));
    let mut t = Session::new();
    run(&mut t, "document.open", json!({"name": "sel.pdf", "dataBase64": vectorcraft_format::base64_encode(&pdf)}));
    let page = t.doc().unwrap().doc.artboards[0].rect;
    let mut png = vec![];
    canvas(&t, page).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    assert!(red(&png) > 20_000, "the PDF draws the square: {} red pixels", red(&png));
    assert!(differing(&png, &canvas(&s, Rect::new(150.0, 150.0, 350.0, 350.0))) < 300, "the PDF draws what the canvas draws");
}

#[test]
fn export_for_screens_of_layers_draws_them_as_the_canvas_does() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id.0;
    clipped(&mut s);
    // A layer at 50% opacity: its art is drawn at 50%.
    run(&mut s, "transparency.set", json!({"ids": [layer], "opacity": 50}));
    let r = run(&mut s, "document.exportForScreens", json!({"layers": [layer], "formats": [{"format": "png", "scale": 1}]}));
    let png = b64(&r["files"][0]);
    assert!(differing(&png, &canvas(&s, Rect::new(150.0, 150.0, 350.0, 350.0))) < 200, "the layer's opacity applies");
    // A template layer's art is still drawn.
    run(&mut s, "transparency.set", json!({"ids": [layer], "opacity": 100}));
    run(&mut s, "layer.setProps", json!({"id": layer, "template": true}));
    let r = run(&mut s, "document.exportForScreens", json!({"layers": [layer], "formats": [{"format": "png", "scale": 1}, {"format": "svg"}]}));
    let files = r["files"].as_array().unwrap();
    let file = |ext: &str| b64(files.iter().find(|f| f["name"].as_str().is_some_and(|n| n.ends_with(ext))).unwrap());
    assert!(red(&file(".png")) > 20_000, "the template layer's art is drawn: {} red pixels", red(&file(".png")));
    assert!(String::from_utf8(file(".svg")).unwrap().contains("<clipPath"), "the SVG holds the clipped art");
}

#[test]
fn a_text_export_of_the_selection_leaves_out_a_type_clipping_path() {
    let mut s = session();
    // The front object, the type "Clip", becomes the clipping path.
    let inner = id(&run(&mut s, "text.create", json!({"x": 100, "y": 110, "text": "Inner"})));
    let clip = id(&run(&mut s, "text.create", json!({"x": 100, "y": 100, "text": "Clip"})));
    run(&mut s, "object.clippingMask.make", json!({"ids": [clip, inner]}));
    let mut first = None;
    s.doc().unwrap().doc.layers[0].walk(&mut |n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::Group { clip: true, .. }) {
            first = n.children().and_then(|c| c.first()).map(|c| c.id.0);
        }
    });
    assert_eq!(first, Some(clip), "the type is the clipping path");
    run(&mut s, "select.set", json!({"ids": [inner]}));
    let txt = b64(&run(&mut s, "document.export", json!({"format": "txt", "selectionOnly": true})));
    assert_eq!(String::from_utf8(txt).unwrap(), "Inner\n");
    let txt = b64(&run(&mut s, "document.exportSelection", json!({"format": "txt"})));
    assert_eq!(String::from_utf8(txt).unwrap(), "Inner\n");
}

#[test]
fn deep_clip_groups_symbols_and_large_selections_keep_their_clips() {
    // Five clip groups inside each other, each clip a little smaller.
    let mut s = session();
    let (square, mut group) = clipped(&mut s);
    for i in 1..5 {
        let inset = 10.0 * f64::from(i);
        let clip = id(&run(&mut s, "shape.rectangle", json!({"x": 150.0 + inset, "y": 150.0 + inset, "width": 200.0 - 2.0 * inset, "height": 400})));
        group = id(&run(&mut s, "object.clippingMask.make", json!({"ids": [clip, group]})));
    }
    run(&mut s, "select.set", json!({"ids": [square]}));
    selection_png_matches_canvas(&mut s, Rect::new(190.0, 190.0, 310.0, 590.0), "a member of five nested clip groups");

    // A symbol instance in a clip group.
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let art = id(&run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 300, "height": 300})));
    run(&mut s, "select.set", json!({"ids": [art]}));
    let instance = id(&run(&mut s, "symbol.new", json!({"name": "Red"})));
    let circle = id(&run(&mut s, "shape.ellipse", json!({"x": 150, "y": 150, "width": 200, "height": 200})));
    run(&mut s, "object.clippingMask.make", json!({"ids": [circle, instance]}));
    run(&mut s, "select.set", json!({"ids": [instance]}));
    selection_png_matches_canvas(&mut s, Rect::new(150.0, 150.0, 350.0, 350.0), "a symbol instance in a clip group");

    // 200 clip groups, a member of each selected.
    let mut s = session();
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let mut members = vec![];
    for i in 0..200 {
        let (x, y) = (f64::from(i % 20) * 25.0, f64::from(i / 20) * 25.0);
        let m = id(&run(&mut s, "shape.rectangle", json!({"x": x, "y": y, "width": 20, "height": 20})));
        let c = id(&run(&mut s, "shape.ellipse", json!({"x": x, "y": y, "width": 20, "height": 20})));
        run(&mut s, "object.clippingMask.make", json!({"ids": [c, m]}));
        members.push(m);
    }
    run(&mut s, "select.set", json!({"ids": members}));
    // The circles' anti-aliased edges differ by up to about 30 pixels each; without their clips the
    // squares' corners differ by 86 pixels each.
    selection_png_matches_canvas_with(&mut s, Rect::new(0.0, 0.0, 495.0, 245.0), "members of 200 clip groups", 40);
}

#[test]
fn an_asset_in_a_hidden_or_locked_clip_group_is_drawn_clipped() {
    for change in ["object.hide", "object.lock"] {
        let mut s = session();
        let (square, group) = clipped(&mut s);
        let asset = run(&mut s, "assets.add", json!({"ids": [square]}))["assets"][0].as_u64().unwrap();
        let want = canvas(&s, Rect::new(150.0, 150.0, 350.0, 350.0));
        run(&mut s, "select.set", json!({"ids": [group]}));
        run(&mut s, change, json!({}));
        let r = run(&mut s, "assets.export", json!({"assets": [asset], "formats": [{"format": "png", "scale": 1}]}));
        assert!(differing(&b64(&r["files"][0]), &want) < 200, "{change}: the asset draws as the canvas did");
    }
}

#[test]
fn selected_only_keeps_holders_and_takes_a_blend_object_alone() {
    let mut s = session();
    let (square, _) = clipped(&mut s);
    run(&mut s, "select.set", json!({"ids": [square]}));
    let r = run(&mut s, "document.export", json!({"format": "png", "scale": 1, "selectedOnly": true}));
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!(differing(&b64(&r), &canvas(&s, board)) < 200, "selectedOnly keeps the clip");
    // An object of a blend is written on its own.
    let mut s = session();
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 60, "height": 60})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 400, "y": 400, "width": 60, "height": 60})));
    run(&mut s, "select.set", json!({"ids": [a, b]}));
    run(&mut s, "object.blend.make", json!({}));
    run(&mut s, "select.set", json!({"ids": [a]}));
    let svg = run(&mut s, "document.serialize", json!({"format": "svg", "selectedOnly": true}))["text"].as_str().unwrap().to_string();
    let mut t = Session::new();
    run(&mut t, "document.open", json!({"name": "a.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}));
    let mut paths = 0;
    for l in &t.doc().unwrap().doc.layers {
        l.walk(&mut |n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. })));
    }
    assert_eq!(paths, 1, "the blend object alone, without the steps: {svg}");
}
