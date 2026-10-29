//! A fill color colorizes grayscale and 1-bit images (`paint.setFill` on the image).

use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_doc::{ImageBlob, Node, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"c": 0, "m": 100, "y": 100, "k": 0}, "spot": true})).unwrap();
    s
}

fn png(img: image::DynamicImage) -> ImageBlob {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    ImageBlob::png(out)
}

/// A gray 2 × 1 image (black, white), with an alpha channel when `alpha`.
fn gray(alpha: bool) -> ImageBlob {
    if alpha {
        png(image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(2, 1, vec![0, 255, 255, 128]).unwrap()))
    } else {
        png(image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap()))
    }
}

/// A selected embedded 2 × 1 image of `blob`.
fn image(s: &mut Session, key: &str, blob: ImageBlob) -> NodeId {
    let key = key.to_string();
    s.edit("Place", |d, sel| {
        d.images.insert(key.clone(), blob);
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key,
            width: 2,
            height: 1,
            xf: Default::default(),
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

fn colorize(s: &Session, id: NodeId) -> Option<Paint> {
    s.doc().unwrap().doc.node(id).unwrap().extras().colorize.as_deref().cloned()
}

#[test]
fn a_fill_color_colorizes_a_gray_image_and_none_takes_it_off() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink", "tint": 40})).unwrap();
    let Some(Paint::Solid { swatch, tint, .. }) = colorize(&s, id) else { panic!("not colorized: {:?}", colorize(&s, id)) };
    assert_eq!((swatch.as_deref(), tint), (Some("Ink"), 0.4));
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.appearance.items.is_empty(), "no fill item over the image");
    assert_eq!(s.proxy_paints().0, colorize(&s, id).unwrap(), "the Fill proxy shows the color");
    // A process color too.
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: None, .. })));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: Some(_), .. })), "undo restores the spot color");
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
    // `none` takes the color off after an edit of the pixels leaves the image in color.
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    s.execute("edit.colors.invert", &json!({"includeImages": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!() };
    assert_eq!(d.image_blob(im).and_then(|b| b.gray_source()), None, "the pixels are no longer gray");
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
}

#[test]
fn an_image_with_alpha_takes_a_spot_color_black_or_white_only() {
    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    let e = s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap_err().to_string();
    assert!(e.contains("a process color can't colorize a grayscale image with transparency"), "{e}");
    assert_eq!(colorize(&s, id), None);
    s.execute("paint.setFill", &json!({"color": {"gray": 100}})).unwrap();
    assert!(colorize(&s, id).is_some(), "black");
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    assert!(matches!(colorize(&s, id), Some(Paint::Solid { swatch: Some(_), .. })));
}

/// A swatch edit that would leave an image with alpha in a process color is refused, as
/// `paint.setFill` refuses one.
#[test]
fn swatch_edits_keep_an_image_with_alpha_out_of_process_colors() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Other", "color": {"c": 100, "m": 0, "y": 0, "k": 0}, "global": true})).unwrap();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let before = colorize(&s, id);
    let message = "a process color can't colorize a grayscale image with transparency";
    for (cmd, p) in [
        ("swatch.edit", json!({"name": "Ink", "spot": false})),
        ("swatch.edit", json!({"name": "Ink", "spot": false, "global": false})),
        ("swatch.setSpot", json!({"name": "Ink", "spot": false})),
        ("swatch.delete", json!({"names": ["Ink"]})),
        ("swatch.merge", json!({"names": ["Other", "Ink"]})),
        ("swatch.resetDefaults", json!({"replace": true})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(message), "{cmd} {p}: {e}");
        assert_eq!(colorize(&s, id), before, "{cmd} {p} left the image as it was");
    }
    // A new color for the spot swatch keeps it a spot color.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": {"c": 100, "m": 0, "y": 0, "k": 0}})).unwrap();
    // An image without alpha takes a process color.
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Ink", "spot": false})).unwrap();
    assert!(colorize(&s, id).is_some());
}

/// With the Appearance panel's active item on a fill of the image, the paint goes to that fill.
#[test]
fn the_active_fill_item_takes_the_paint_instead() {
    let mut s = session();
    let id = image(&mut s, "alpha", gray(true));
    s.execute("appearance.addFill", &json!({})).unwrap();
    let index = s.doc().unwrap().doc.node(id).unwrap().appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": index})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colorize(&s, id), None);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let blue = vectorcraft_color::Color::rgb(0.0, 0.0, 1.0);
    assert_eq!(n.appearance.fill().and_then(|f| f.paint.color()), Some(blue), "the fill item took the color");
}

/// `none` on a gray image without a color takes the paint off its fill item.
#[test]
fn none_on_a_gray_image_without_a_color_clears_its_fill_item() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("appearance.addFill", &json!({})).unwrap();
    let index = s.doc().unwrap().doc.node(id).unwrap().appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": index})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": null})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    assert_eq!(colorize(&s, id), None);
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert_eq!(n.appearance.fill().map(|f| &f.paint), Some(&Paint::None), "the fill item's paint is none");
}

#[test]
fn a_color_image_takes_a_fill_item_as_before() {
    let mut s = session();
    let rgb = png(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap()));
    let id = image(&mut s, "rgb", rgb);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    assert_eq!(colorize(&s, id), None);
    assert!(s.doc().unwrap().doc.node(id).unwrap().appearance.fill().is_some(), "a fill item, as before");
}

#[test]
fn colorized_images_save_and_open_with_their_color() {
    let mut s = session();
    let id = image(&mut s, "gray", gray(false));
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    assert!(back.node(id).unwrap().extras().colorize.is_some());
    // Swatch edits reach it as they reach fills.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": {"c": 100, "m": 0, "y": 0, "k": 0}})).unwrap();
    let Some(Paint::Solid { color, .. }) = colorize(&s, id) else { panic!() };
    assert_eq!(color, vectorcraft_color::Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 });
}
