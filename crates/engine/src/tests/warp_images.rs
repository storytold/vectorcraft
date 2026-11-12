//! Effect › Warp on placed images: the pixels bend with the frame, in the canvas and in exports.

use serde_json::{Value, json};
use vectorcraft_doc::live::{WarpNet, WarpStyle, uses_net, warp_version};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Affine, Point, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// The box of the image [`quarters`] places.
const FRAME: Rect = Rect::new(100.0, 100.0, 180.0, 180.0);

/// The color of the quarter of the image [`quarters`] places that holds (u, v) of its box: red top
/// left, green top right, blue bottom left, yellow bottom right.
fn quarter(u: f64, v: f64) -> [u8; 3] {
    match (u < 0.5, v < 0.5) {
        (true, true) => [255, 0, 0],
        (false, true) => [0, 255, 0],
        (true, false) => [0, 0, 255],
        (false, false) => [255, 255, 0],
    }
}

/// A selected embedded 40×40 px image filling [`FRAME`], each quarter one color ([`quarter`]).
fn quarters(s: &mut Session) -> NodeId {
    let img = image::RgbaImage::from_fn(40, 40, |x, y| {
        let [r, g, b] = quarter(x as f64 / 40.0, y as f64 / 40.0);
        image::Rgba([r, g, b, 255])
    });
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, sel| {
        d.images.insert("quarters".into(), vectorcraft_doc::ImageBlob::png(png));
        let id = d.alloc_id();
        let xf = Affine::translate((FRAME.x0, FRAME.y0)) * Affine::scale(2.0);
        let im = vectorcraft_doc::ImageObject {
            key: "quarters".into(),
            width: 40,
            height: 40,
            xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

/// Where an Arc warp bending `bend` % takes the point at (u, v) of [`FRAME`].
fn arced(bend: f64, u: f64, v: f64) -> Point {
    let net = WarpNet::new(WarpStyle::Arc, bend / 100.0, 0.0, 0.0, FRAME.width(), FRAME.height(), false);
    FRAME.origin() + net.at(u, v).to_vec2()
}

/// Points of the image's box: near each quarter's middle and on both sides of the line between
/// the top and bottom quarters, where a warp that moves the pixels up or down changes the color.
const SAMPLES: [(f64, f64); 10] =
    [(0.2, 0.2), (0.8, 0.2), (0.2, 0.8), (0.8, 0.8), (0.4, 0.4), (0.6, 0.4), (0.4, 0.6), (0.6, 0.6), (0.3, 0.62), (0.7, 0.62)];

/// The document drawn over (0, 0)–(400, 400) at one pixel a point, on white.
fn drawn(s: &Session) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 400.0, 400.0), 1.0, true)
}

/// The sample points whose color in `img` at `at(u, v)` is not the image's color at (u, v).
fn wrong(img: &vectorcraft_render::Rendered, at: &dyn Fn(f64, f64) -> Point) -> Vec<(f64, f64, [u8; 4])> {
    SAMPLES
        .iter()
        .filter_map(|&(u, v)| {
            let p = at(u, v);
            let px = img.pixel(p.x.floor() as u32, p.y.floor() as u32);
            let want = quarter(u, v);
            let near = (0..3).all(|i| (px[i] as i32 - want[i] as i32).abs() <= 40);
            (!near).then_some((u, v, px))
        })
        .collect()
}

/// `quarters` with Effect › Warp › Arc at 50 %: the warp the effect holds, checked to be the
/// Bézier patch [`arced`] follows.
fn arc_image(s: &mut Session) -> NodeId {
    let id = quarters(s);
    s.execute("effect.apply", &json!({"effect": "warp.arc", "params": {"bend": 50}})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let e = &n.appearance.effects[0];
    assert!(warp_version(&e.params["version"]).is_some_and(uses_net), "{}", e.params);
    id
}

fn decoded(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn an_images_pixels_bend_with_effect_warp() {
    let mut s = session();
    let id = arc_image(&mut s);
    // The arc lifts the middle of the image: the line between the quarters rises there.
    assert!(arced(50.0, 0.5, 0.6).y < FRAME.y0 + 0.5 * FRAME.height(), "{:?}", arced(50.0, 0.5, 0.6));
    let bent = drawn(&s);
    assert_eq!(wrong(&bent, &|u, v| arced(50.0, u, v)), vec![], "each pixel is drawn where the warp takes it");
    // Drawn as pieces of the one image, each placed by the warp.
    let d = &s.doc().unwrap().doc;
    let art = vectorcraft_render::effects::reshape(Some(d), d.node(id).unwrap(), None).unwrap();
    let mut pieces = 0;
    art.walk(&mut |c| pieces += matches!(c.kind, NodeKind::Image(_)) as usize);
    assert!(pieces >= 100, "{pieces} pieces");
    // Removing the warp draws the image as it was, in one undo step.
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("effect.remove", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    let plain = |u: f64, v: f64| Point::new(FRAME.x0 + u * FRAME.width(), FRAME.y0 + v * FRAME.height());
    assert_eq!(wrong(&drawn(&s), &plain), vec![]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(wrong(&drawn(&s), &|u, v| arced(50.0, u, v)), vec![], "undo brings the bend back");
}

/// `out`, an export of [`arc_image`]'s document named `name`, opened again: it draws the image
/// bent.
fn opens_bent(name: &str, out: &Value) {
    let mut back = session();
    back.execute("document.open", &json!({"name": name, "dataBase64": out["dataBase64"]})).unwrap();
    assert_eq!(wrong(&drawn(&back), &|u, v| arced(50.0, u, v)), vec![], "{name}");
}

#[test]
fn a_warped_image_keeps_its_bend_in_svg() {
    let mut s = session();
    arc_image(&mut s);
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let text = String::from_utf8(decoded(&svg)).unwrap();
    assert_eq!(text.matches("<image").count(), 1, "the pieces share the image's pixels");
    opens_bent("warped.svg", &svg);
}

#[test]
fn a_warped_image_keeps_its_bend_in_pdf() {
    let mut s = session();
    arc_image(&mut s);
    opens_bent("warped.pdf", &s.execute("document.export", &json!({"format": "pdf", "preserveEditing": false})).unwrap());
}

#[test]
fn a_warped_image_keeps_its_bend_in_ai() {
    let mut s = session();
    arc_image(&mut s);
    opens_bent("warped.ai", &s.execute("file.saveCopy", &json!({"format": "ai"})).unwrap());
}

#[test]
fn a_warped_image_keeps_its_bend_in_eps() {
    let mut s = session();
    arc_image(&mut s);
    opens_bent("warped.eps", &s.execute("document.exportEps", &json!({"previewFormat": "none", "useArtboards": false})).unwrap());
}

#[test]
fn a_warped_image_keeps_its_bend_in_png() {
    let mut s = session();
    arc_image(&mut s);
    let out = s.execute("document.export", &json!({"format": "png"})).unwrap();
    let png = image::load_from_memory(&decoded(&out)).unwrap().to_rgba8();
    // The artboard starts at (0, 0) and the export draws one pixel a point.
    assert_eq!((png.width(), png.height()), (800, 600));
    for &(u, v) in &SAMPLES {
        let p = arced(50.0, u, v);
        let px = png.get_pixel(p.x.floor() as u32, p.y.floor() as u32).0;
        let want = quarter(u, v);
        assert!((0..3).all(|i| (px[i] as i32 - want[i] as i32).abs() <= 40), "({u}, {v}) at {p:?}: {px:?}");
    }
}

/// A group the user made of two clip groups of the same image, each clipped by a rectangle, writes
/// the image once for each: only the pieces of a warp share one.
#[test]
fn svg_writes_an_image_clipped_twice_in_a_users_group_twice() {
    let mut s = session();
    let im = quarters(&mut s);
    s.edit("Clip twice", |d, _| {
        let image = d.node(im).cloned().unwrap();
        let mut clipped = vec![];
        for x in [FRAME.x0, FRAME.x0 + 40.0] {
            let mut clip =
                Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(x, FRAME.y0, x + 30.0, FRAME.y0 + 30.0)), Default::default());
            if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                *clipping = true;
            }
            let image = Node { id: d.alloc_id(), ..image.clone() };
            let children = vec![std::sync::Arc::new(clip), std::sync::Arc::new(image)];
            clipped.push(std::sync::Arc::new(Node::new(d.alloc_id(), NodeKind::Group { children, clip: true })));
        }
        let group = Node::group(d.alloc_id(), clipped);
        d.remove(im)?;
        d.insert(d.default_layer(), 0, group)?;
        Ok(())
    })
    .unwrap();
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let text = String::from_utf8(decoded(&svg)).unwrap();
    assert_eq!(text.matches("<image").count(), 2, "{text}");
}

/// Expand Appearance of a warped image gives objects that draw as the canvas did, with ids of
/// their own, in one undo step.
#[test]
fn expand_appearance_of_a_warped_image_draws_as_the_canvas_did() {
    let mut s = session();
    let id = arc_image(&mut s);
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    let d = &s.doc().unwrap().doc;
    let mut ids = std::collections::HashSet::new();
    let mut images = 0;
    for layer in &d.layers {
        layer.walk(&mut |c| {
            assert!(ids.insert(c.id), "{:?} twice", c.id);
            images += matches!(c.kind, NodeKind::Image(_)) as usize;
            assert!(c.appearance.effects.iter().all(|e| !e.id.starts_with("warp.")));
        });
    }
    assert!(images >= 100, "{images} pieces");
    assert_eq!(wrong(&drawn(&s), &|u, v| arced(50.0, u, v)), vec![]);
    s.execute("edit.undo", &json!({})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert!(matches!(n.kind, NodeKind::Image(_)) && n.appearance.effects.len() == 1);
    assert_eq!(wrong(&drawn(&s), &|u, v| arced(50.0, u, v)), vec![]);
}

/// A selected embedded grayscale 40×40 px image filling [`FRAME`], black above white, colorized
/// red by its fill color: red above, nothing below.
fn red_on_clear(s: &mut Session) -> NodeId {
    let img = image::GrayImage::from_fn(40, 40, |_, y| image::Luma([if y < 20 { 0 } else { 255 }]));
    let mut png = vec![];
    image::DynamicImage::ImageLuma8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let id = s
        .edit("Place", |d, sel| {
            d.images.insert("gray".into(), vectorcraft_doc::ImageBlob::png(png));
            let id = d.alloc_id();
            let xf = Affine::translate((FRAME.x0, FRAME.y0)) * Affine::scale(2.0);
            let im = vectorcraft_doc::ImageObject {
                key: "gray".into(),
                width: 40,
                height: 40,
                xf,
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [id.0]})).unwrap();
    id
}

#[test]
fn a_colorized_image_keeps_its_color_when_warped() {
    let mut s = session();
    red_on_clear(&mut s);
    s.execute("effect.apply", &json!({"effect": "warp.arc", "params": {"bend": 50}})).unwrap();
    let img = drawn(&s);
    let (red, white) = ([255u8, 0, 0], [255u8, 255, 255]);
    for (u, v, want) in [(0.5, 0.35, red), (0.2, 0.2, red), (0.5, 0.65, white), (0.8, 0.8, white)] {
        let p = arced(50.0, u, v);
        let px = img.pixel(p.x.floor() as u32, p.y.floor() as u32);
        assert!((0..3).all(|i| (px[i] as i32 - want[i] as i32).abs() <= 40), "({u}, {v}) at {p:?}: {px:?}");
    }
}

#[test]
fn a_warp_on_a_group_bends_the_image_in_it() {
    let mut s = session();
    let im = quarters(&mut s);
    // A group of the image and a line below it as wide as the image: the group's box is the image's.
    let line = s.execute("shape.line", &json!({"x1": FRAME.x0, "y1": FRAME.y1, "x2": FRAME.x1, "y2": FRAME.y1})).unwrap();
    let line = NodeId(line["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [im.0, line.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("effect.apply", &json!({"effect": "warp.arc", "params": {"bend": 50}})).unwrap();
    assert_eq!(wrong(&drawn(&s), &|u, v| arced(50.0, u, v)), vec![]);
}
