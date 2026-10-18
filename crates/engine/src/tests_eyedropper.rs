//! Eyedropper Options and modes (M3.52): the Picks Up / Applies trees, type attributes, Alt-click
//! (reverse) and Shift+Alt-click (append), image pixels, colour models and the preference.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ImageBlob, ImageObject, Justify, LineCap, Node, NodeKind};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn click(s: &mut Session, x: f64, y: f64, mods: Mods) {
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y).with_mods(mods), ViewInfo::default()).unwrap();
}

/// A red-filled source with a 5 pt round-capped blue stroke at 50 % opacity, and a selected
/// target with the default white fill and a 2 pt black stroke.
fn source_and_target(s: &mut Session) -> (NodeId, NodeId) {
    let src = rect(s, 300.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#0000ff"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 5, "cap": "round"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let dst = rect(s, 10.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2})).unwrap();
    (src, dst)
}

#[test]
fn picking_up_without_weight_keeps_the_target_weight() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    s.execute("eyedropper.setOptions", &json!({"pickUp": {"appearance": {"stroke": {"weight": false}}}})).unwrap();
    let weight = s.paint.stroke_width;
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let n = node(&s, dst);
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.paint.color().unwrap().to_hex(), st.width, st.cap), ("#0000ff".into(), 2.0, LineCap::Round));
    assert_eq!(n.appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    assert_eq!(n.opacity, 0.5);
    assert_eq!(s.paint.stroke_width, weight, "new art keeps its weight too");
    // Applying no transparency (one call) keeps the target's opacity; undo is one step per copy.
    s.execute("transparency.set", &json!({"opacity": 100, "ids": [dst.0]})).unwrap();
    // A green fill, which the copy turns red again.
    s.execute("paint.setFill", &json!({"color": "#00ff00", "ids": [dst.0]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("appearance.copyFrom", &json!({"source": src.0, "apply": {"appearance": {"transparency": false}}})).unwrap();
    assert_eq!((node(&s, dst).opacity, s.doc().unwrap().history.undo.len()), (1.0, undo + 1));
    assert!(s.execute("appearance.copyFrom", &json!({"source": src.0, "pickUp": {"nope": true}})).is_err());
}

#[test]
fn every_appearance_attribute_copies_the_whole_stack() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    assert_eq!(node(&s, dst).appearance, node(&s, src).appearance);
    // Without the fill's overprint only the focal attributes go: the target keeps its two items.
    let dst2 = rect(&mut s, 150.0);
    s.execute("appearance.copyFrom", &json!({"source": src.0, "apply": {"appearance": {"fill": {"overprint": false}}}})).unwrap();
    assert_eq!(node(&s, dst2).appearance.items.len(), 2);
}

#[test]
fn type_picks_up_font_and_size() {
    let mut s = session();
    let src = id_of(s.execute("text.create", &json!({"x": 10, "y": 300, "text": "Source"})).unwrap());
    s.execute("text.setStyle", &json!({"font": "Inter", "size": 30, "tracking": 50, "justify": "center", "fill": "#00ff00"})).unwrap();
    let dst = id_of(s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Target"})).unwrap());
    let style = |s: &Session, id| match &node(s, id).kind {
        NodeKind::Text(t) => (t.first_style(), t.para.justify),
        _ => panic!("not type"),
    };
    let width = |s: &Session, id| node(s, id).geometric_bounds().unwrap().width();
    let before = width(&s, dst);
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let (st, justify) = style(&s, dst);
    assert_eq!((st.font_family.as_str(), st.size, st.tracking, justify), ("Inter", 30.0, 50.0, Justify::Center));
    assert_eq!(st.fill.color().unwrap().to_hex(), "#00ff00");
    assert!(width(&s, dst) > before, "laid out again at the new size");
    // Without character or paragraph attributes only the paints go.
    let dst2 = id_of(s.execute("text.create", &json!({"x": 10, "y": 500, "text": "Other"})).unwrap());
    let preserved_alignment = style(&s, dst2).1;
    s.execute("appearance.copyFrom", &json!({"source": src.0, "pickUp": {"character": false, "paragraph": false}})).unwrap();
    let (st, justify) = style(&s, dst2);
    assert_eq!((st.size, justify, st.fill.color().unwrap().to_hex()), (12.0, preserved_alignment, "#00ff00".into()));
}

#[test]
fn alt_click_applies_the_selection_to_the_clicked_object() {
    let mut s = session();
    let (src, dst) = source_and_target(&mut s);
    // `dst` is selected; Alt-click `src`: it takes the selection's white fill and 2 pt stroke.
    let defaults = s.paint.clone();
    click(&mut s, 325.0, 35.0, Mods { alt: true, ..Default::default() });
    let n = node(&s, src);
    assert_eq!((n.appearance.fill_paint(), n.appearance.stroke_width()), (Paint::solid(Color::WHITE), 2.0));
    assert_eq!(node(&s, dst).appearance.fill_paint(), Paint::solid(Color::WHITE), "the selection is unchanged");
    assert_eq!(s.paint, defaults, "the defaults stay");
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("appearance.copyFrom", &json!({"source": src.0, "reverse": true})).is_err(), "nothing to copy from");
}

#[test]
fn shift_alt_click_appends_the_appearance() {
    let mut s = session();
    let (_, dst) = source_and_target(&mut s);
    click(&mut s, 325.0, 35.0, Mods { alt: true, shift: true, ..Default::default() });
    let items = node(&s, dst).appearance.items.clone();
    assert_eq!(items.len(), 4, "the source's fill and stroke on top of the target's");
    assert_eq!((items[0].paint().color().unwrap().to_hex(), items[2].paint().color().unwrap().to_hex()), ("#ffffff".into(), "#ff0000".into()));
}

#[test]
fn cmyk_stays_cmyk() {
    let mut s = session();
    let src = rect(&mut s, 300.0);
    let cmyk = Color::cmyk(0.1, 0.5, 0.9, 0.2);
    s.execute("paint.setFill", &json!({"color": {"c": 0.1, "m": 0.5, "y": 0.9, "k": 0.2}})).unwrap();
    let dst = rect(&mut s, 10.0);
    click(&mut s, 325.0, 35.0, Mods { shift: true, ..Default::default() });
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(cmyk), "a sampled colour keeps its model");
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    click(&mut s, 325.0, 35.0, Mods::default());
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(cmyk));
    let _ = src;
}

#[test]
fn clicking_an_image_samples_its_pixel() {
    let mut s = session();
    // Two pixels, orange and teal, 100 pt each, at (300, 0).
    let img = image::RgbaImage::from_raw(2, 1, [[255, 128, 0, 255], [0, 128, 128, 255]].concat()).unwrap();
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let xf = vectorcraft_geom::Affine::translate((300.0, 0.0)) * vectorcraft_geom::Affine::scale(100.0);
        let im = ImageObject { key: "px".into(), width: 2, height: 1, xf, link: None, placement: Default::default() };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).map_err(|e| crate::EngineError::Other(e.to_string()))
    })
    .unwrap();
    let dst = rect(&mut s, 10.0);
    click(&mut s, 450.0, 50.0, Mods::default());
    // Compared as stored (not as displayed through the working RGB space).
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(Color::rgb8(0, 128, 128)));
    s.execute("eyedropper.setOptions", &json!({"sampleSize": 5})).unwrap();
    click(&mut s, 350.0, 50.0, Mods::default());
    assert_eq!(node(&s, dst).appearance.fill_paint().color(), Some(Color::rgb8(128, 128, 64)), "5 x 5 average of both pixels");
}

/// A CMYK document with one image of `px` (one row, 100 pt a pixel) at (300, 0).
fn cmyk_doc_with_image(px: &[[u8; 4]]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600, "colorMode": "cmyk"})).unwrap();
    let w = px.len() as u32;
    let img = image::RgbaImage::from_raw(w, 1, px.concat()).unwrap();
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let xf = vectorcraft_geom::Affine::translate((300.0, 0.0)) * vectorcraft_geom::Affine::scale(100.0);
        let im = ImageObject { key: "px".into(), width: w, height: 1, xf, link: None, placement: Default::default() };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).map_err(|e| crate::EngineError::Other(e.to_string()))
    })
    .unwrap();
    s
}

/// The CMYK color the color settings separate `rgb` into (with their intent).
fn managed_cmyk(rgb: Color) -> Color {
    let cms = vectorcraft_color::cms::active();
    cms.convert(&rgb, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent)
}

/// #2276: an RGB image's pixel sampled into a CMYK document separates through the color
/// settings, as Document Color Mode converts it, not by the profile-free formula
/// (83/170/223 came in as C63 M24 Y0 K13).
#[test]
fn an_rgb_image_sampled_in_a_cmyk_document_separates_through_the_color_settings() {
    let mut s = cmyk_doc_with_image(&[[83, 170, 223, 255], [224, 238, 255, 255]]);
    let dst = rect(&mut s, 10.0);
    click(&mut s, 350.0, 50.0, Mods::default());
    let got = node(&s, dst).appearance.fill_paint().color().unwrap();
    assert_eq!(got, managed_cmyk(Color::rgb8(83, 170, 223)));
    let Color::Cmyk { c, m, y, k } = got else { panic!("{got:?}") };
    // Generic CMYK: about C59 M12 Y3 K0, with no black, where the formula gave K13.
    assert!((0.5..0.75).contains(&c) && (0.05..0.2).contains(&m) && y < 0.05 && k < 0.02, "{c} {m} {y} {k}");
    // A light blue shows close to the color it was picked from; the formula's C12 M7 showed duller.
    click(&mut s, 450.0, 50.0, Mods::default());
    let got = node(&s, dst).appearance.fill_paint().color().unwrap();
    assert!(matches!(got, Color::Cmyk { .. }), "{got:?}");
    let light = Color::rgb8(224, 238, 255);
    let off = |c: Color| vectorcraft_color::cms::delta_e2000(c.to_lab(), light.to_lab());
    let naive = light.in_model(vectorcraft_color::cms::Model::Cmyk);
    assert!(off(got) < 2.0, "{} is ΔE {} from #e0eeff ({} at ΔE {} before)", got.to_hex(), off(got), naive.to_hex(), off(naive));
}

/// #2276: colors typed or sent in RGB to a CMYK document separate through the color settings:
/// fills and strokes, gradient stops, new swatches and a sampled gradient stop.
#[test]
fn rgb_colors_given_to_a_cmyk_document_separate_through_the_color_settings() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600, "colorMode": "cmyk"})).unwrap();
    let a = rect(&mut s, 10.0);
    let blue = Color::rgb8(83, 170, 223);
    s.execute("paint.setFill", &json!({"color": "#53aadf"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#53AADF"})).unwrap();
    let n = node(&s, a);
    assert_eq!(n.appearance.fill_paint().color(), Some(managed_cmyk(blue)));
    assert_eq!(n.appearance.stroke_paint().color(), Some(managed_cmyk(blue)));
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#53aadf"}]);
    s.execute("paint.setFill", &json!({"gradient": {"stops": stops}})).unwrap();
    let Paint::Gradient(g) = node(&s, a).appearance.fill_paint() else { panic!("a gradient") };
    assert_eq!(g.gradient.stops[1].color, managed_cmyk(blue));
    // The Gradient panel's eyedropper samples into the selected stop.
    s.execute("paint.sampleColor", &json!({"color": "#53aadf", "stop": 0})).unwrap();
    let Paint::Gradient(g) = node(&s, a).appearance.fill_paint() else { panic!("a gradient") };
    assert_eq!(g.gradient.stops[0].color, managed_cmyk(blue));
    let name = s.execute("swatch.new", &json!({"color": "#53aadf"})).unwrap()["name"].as_str().unwrap().to_string();
    assert_eq!(s.doc().unwrap().doc.swatch(&name).and_then(|w| w.paint.color()), Some(managed_cmyk(blue)));
    // keepModel still keeps RGB.
    s.execute("paint.setFill", &json!({"color": "#53aadf", "keepModel": true})).unwrap();
    assert_eq!(node(&s, a).appearance.fill_paint().color(), Some(blue));
}

/// A new CMYK document.
fn cmyk_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600, "colorMode": "cmyk"})).unwrap();
    s
}

/// RGB colors given to the type commands in a CMYK document separate through the color settings,
/// as `paint.setFill` stores them: `text.create {color}`, `text.setStyle {fill}` and
/// `text.setRangeStyle {fill, stroke}`. CMYK colors are stored as given.
#[test]
fn rgb_type_colors_given_to_a_cmyk_document_separate_through_the_color_settings() {
    let mut s = cmyk_session();
    let blue = managed_cmyk(Color::rgb8(83, 170, 223));
    let t = id_of(s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hello", "color": "#53aadf"})).unwrap());
    let runs = |s: &Session| match node(s, t).kind {
        NodeKind::Text(o) => o.runs.iter().map(|r| (r.style.fill.color(), r.style.stroke.color())).collect::<Vec<_>>(),
        k => panic!("not type: {k:?}"),
    };
    assert_eq!(runs(&s)[0].0, Some(blue));
    s.execute("text.setStyle", &json!({"ids": [t.0], "fill": "#ff0000"})).unwrap();
    assert_eq!(runs(&s)[0].0, Some(managed_cmyk(Color::rgb8(255, 0, 0))));
    s.execute("text.setRangeStyle", &json!({"id": t.0, "start": 0, "end": 2, "fill": "#53aadf", "stroke": "#53aadf"})).unwrap();
    assert_eq!(runs(&s)[0], (Some(blue), Some(blue)));
    s.execute("text.setStyle", &json!({"ids": [t.0], "fill": {"c": 10, "m": 20, "y": 30, "k": 40}})).unwrap();
    assert!(runs(&s).iter().all(|r| r.0 == Some(Color::cmyk(0.1, 0.2, 0.3, 0.4))), "{:?}", runs(&s));
    s.execute("text.setRangeStyle", &json!({"id": t.0, "start": 0, "end": 2, "stroke": "#53aadf", "keepModel": true})).unwrap();
    assert_eq!(runs(&s)[0].1, Some(Color::rgb8(83, 170, 223)), "keepModel keeps RGB");
}

/// `shapeBuilder.merge {fill: "#rrggbb"}` in a CMYK document fills the merged shape as
/// `fill: {color}` does.
#[test]
fn a_shape_builder_hex_fill_in_a_cmyk_document_separates_through_the_color_settings() {
    let mut s = cmyk_session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    let r = s.execute("shapeBuilder.merge", &json!({"ids": [a.0, b.0], "points": [[20, 30], [80, 30]], "fill": "#53aadf"})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(node(&s, merged).appearance.fill_paint().color(), Some(managed_cmyk(Color::rgb8(83, 170, 223))));
    // The paint form takes keepModel.
    let (a, b) = (rect(&mut s, 200.0), rect(&mut s, 230.0));
    let fill = json!({"color": "#53aadf", "keepModel": true});
    let r = s.execute("shapeBuilder.merge", &json!({"ids": [a.0, b.0], "points": [[210, 30], [270, 30]], "fill": fill})).unwrap();
    let merged = NodeId(r["merged"].as_u64().unwrap());
    assert_eq!(node(&s, merged).appearance.fill_paint().color(), Some(Color::rgb8(83, 170, 223)));
}

/// RGB colors given to gradient stops, freeform gradient points and gradient mesh points in a CMYK
/// document separate through the color settings: `paint.editGradient {stops}`, a freeform
/// `gradient` given to `paint.setFill`, `paint.freeform.addPoint` and `setPoint`, and
/// `object.mesh.setPointColor` and `addLine`. The Mesh tool adds a line in the fill's own model.
#[test]
fn rgb_gradient_and_mesh_colors_given_to_a_cmyk_document_separate_through_the_color_settings() {
    let mut s = cmyk_session();
    let (blue, red) = (managed_cmyk(Color::rgb8(83, 170, 223)), managed_cmyk(Color::rgb8(255, 0, 0)));
    let a = rect(&mut s, 10.0);
    let gradient = |s: &Session| match node(s, a).appearance.fill_paint() {
        Paint::Gradient(g) => *g,
        p => panic!("not a gradient: {p:?}"),
    };
    let stops = |s: &Session| gradient(s).gradient.stops.iter().map(|st| st.color).collect::<Vec<_>>();
    s.execute("paint.editGradient", &json!({"stops": [{"offset": 0, "color": {"gray": 0}}, {"offset": 1, "color": "#53aadf"}]})).unwrap();
    assert_eq!(stops(&s), vec![Color::gray(0.0), blue]);
    // keepModel keeps RGB, and stops sent back with only their offsets changed keep their colors.
    let rgb = [Color::rgb8(255, 0, 0), Color::rgb8(83, 170, 223)];
    s.execute("paint.editGradient", &json!({"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#53aadf"}], "keepModel": true}))
        .unwrap();
    assert_eq!(stops(&s), rgb);
    s.execute("paint.editGradient", &json!({"stops": [{"offset": 0.2, "color": "#ff0000"}, {"offset": 1, "color": "#53aadf"}]})).unwrap();
    assert_eq!(stops(&s), rgb);
    // A stop naming an RGB process swatch takes the swatch's color as it is.
    let orange = Color::rgb8(255, 128, 0);
    s.execute("swatch.new", &json!({"name": "Screen", "color": "#ff8000", "keepModel": true})).unwrap();
    let named = json!([{"offset": 0, "swatch": "Screen"}, {"offset": 1, "color": "#53aadf"}]);
    s.execute("paint.editGradient", &json!({"stops": named})).unwrap();
    assert_eq!(stops(&s), [orange, rgb[1]], "the RGB blue the gradient has stays RGB");
    s.execute("paint.setFill", &json!({"gradient": {"stops": named}})).unwrap();
    assert_eq!(stops(&s), [orange, blue]);
    let magenta = Color::cmyk(0.0, 1.0, 0.0, 0.0);
    let points = json!([{"at": [20, 20], "color": "#53aadf"}, {"at": [50, 50], "color": {"c": 0, "m": 100, "y": 0, "k": 0}}]);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "freeform", "freeform": {"points": points}}})).unwrap();
    let colors = |s: &Session| gradient(s).freeform.map(|f| f.points.iter().map(|p| p.color).collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(colors(&s), vec![blue, magenta]);
    let i = s.execute("paint.freeform.addPoint", &json!({"at": [40, 20], "color": "#ff0000"})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(colors(&s)[i], red);
    s.execute("paint.freeform.setPoint", &json!({"index": 1, "color": "#53aadf"})).unwrap();
    assert_eq!(colors(&s)[1], blue);
    // New stops recolor the three points from their CMYK values.
    s.execute("paint.editGradient", &json!({"stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#ff0000"}]})).unwrap();
    let stored = vectorcraft_color::Gradient {
        kind: Default::default(),
        stops: vec![vectorcraft_color::GradientStop::new(0.0, managed_cmyk(Color::WHITE)), vectorcraft_color::GradientStop::new(1.0, red)],
    };
    // The points mix in the model the stops share (#2829): CMYK here.
    assert_eq!(colors(&s), [0.0, 0.5, 1.0].map(|t| stored.sample_with(t, Color::lerp_keep_model).0), "{:?}", colors(&s));

    let m = rect(&mut s, 300.0);
    s.execute("object.mesh.create", &json!({"ids": [m.0], "rows": 1, "cols": 1})).unwrap();
    let mesh = |s: &Session| match node(s, m).kind {
        NodeKind::Mesh(g) => g.points.iter().map(|p| p.color).collect::<Vec<_>>(),
        k => panic!("not a mesh: {k:?}"),
    };
    s.execute("object.mesh.setPointColor", &json!({"id": m.0, "index": 1, "color": "#53aadf", "keepModel": true})).unwrap();
    assert_eq!(mesh(&s)[1], Color::rgb8(83, 170, 223), "keepModel keeps RGB");
    s.execute("object.mesh.setPointColor", &json!({"id": m.0, "index": 0, "color": "#53aadf"})).unwrap();
    assert_eq!(mesh(&s)[0], blue);
    let i = s.execute("object.mesh.addLine", &json!({"id": m.0, "x": 320, "y": 30, "color": "#ff0000"})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(mesh(&s)[i], red);
    let cyan = Color::cmyk(0.5, 0.2, 0.0, 0.0);
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 50, "m": 20, "y": 0, "k": 0}})).unwrap();
    let before = mesh(&s).len();
    s.select_tool("mesh", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 335.0, 45.0), ViewInfo::default()).unwrap();
    let after = mesh(&s);
    assert!(after.len() > before, "the click adds a line");
    assert!(after.contains(&cyan), "{after:?}");
    // An RGB fill kept with keepModel stays RGB on the new point.
    let orange = Color::rgb8(255, 128, 0);
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff8000", "keepModel": true})).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 340.0, 20.0), ViewInfo::default()).unwrap();
    assert!(mesh(&s).contains(&orange), "{:?}", mesh(&s));
}

#[test]
fn the_options_are_a_preference() {
    let mut s = Session::new();
    assert!(s.execute("eyedropper.setOptions", &json!({"sampleSize": 4})).is_err());
    assert!(s.execute("eyedropper.setOptions", &json!({"apply": {"character": 1}})).is_err());
    s.execute("eyedropper.setOptions", &json!({"sampleSize": 3, "apply": {"paragraph": false}})).unwrap();
    let got = s.execute("prefs.get", &json!({"key": "eyedropper"})).unwrap();
    assert_eq!((&got["sampleSize"], &got["apply"]["paragraph"], &got["pickUp"]["paragraph"]), (&json!(3), &json!(false), &json!(true)));
    s.execute("prefs.set", &json!({"key": "eyedropper", "value": {"pickUp": {"appearance": false}}})).unwrap();
    assert!(!s.prefs.eyedropper.pick_up.appearance.fill.color && s.prefs.eyedropper.sample_size == 3);
    // Saved with the preferences; older preferences (no options) load the defaults.
    let back: Prefs = serde_json::from_value(s.prefs.to_json()).unwrap();
    assert_eq!(back, s.prefs);
    let mut old = Prefs::default().to_json();
    old.as_object_mut().unwrap().remove("eyedropper");
    assert_eq!(serde_json::from_value::<Prefs>(old).unwrap().eyedropper, EyedropperOptions::default());
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.eyedropper, EyedropperOptions::default());
}

#[test]
fn type_takes_the_stroke_options_it_has() {
    let mut s = session();
    let (src, _) = source_and_target(&mut s);
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("stroke.set", &json!({"dash": [4, 2]})).unwrap();
    let dst = id_of(s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Target"})).unwrap());
    s.execute("eyedropper.setOptions", &json!({"apply": {"appearance": {"stroke": {"dash": false}}}})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let NodeKind::Text(t) = &node(&s, dst).kind else { panic!("not type") };
    let st = t.first_style();
    assert_eq!((st.stroke.color().unwrap().to_hex(), st.stroke_width, st.stroke_cap), ("#0000ff".into(), 5.0, LineCap::Round));
    assert!(st.stroke_dash.is_none(), "the dash pattern isn't applied");
}
