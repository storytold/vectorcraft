use std::io::Cursor;

use image::DynamicImage;
use serde_json::{Value, json};
use vectorcraft_doc::color_mask::{ColorMask, MaskBackground};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, NodeId, NodeKind};
use vectorcraft_geom::Rect;

use super::{APPLY_COLOR_MASK, COLOR_MASK, REMOVE_COLOR_MASK};
use crate::Session;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [200, 0, 0, 255];

fn png(img: DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// 12 × 10 pixels: white, with a red block from (3, 3) to (9, 7) that holds one white pixel at
/// (6, 5).
fn framed() -> Vec<u8> {
    png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(12, 10, |x, y| {
        let block = (3..9).contains(&x) && (3..7).contains(&y);
        image::Rgba(if block && (x, y) != (6, 5) { RED } else { WHITE })
    })))
}

/// Hide white wherever it is, exactly.
fn hide_white() -> Value {
    json!({"background": "colors", "colors": "#ffffff", "contiguous": false, "tolerance": 0, "softness": 0})
}

/// A session with `bytes` opened as a document, its image selected.
fn open(bytes: &[u8]) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    (s, id)
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

fn image_of(d: &Document, id: NodeId) -> ImageObject {
    let NodeKind::Image(im) = &d.node(id).unwrap().kind else { panic!("an image") };
    (**im).clone()
}

fn mask_of(d: &Document, id: NodeId) -> Option<ColorMask> {
    Document::color_mask_of(d.node(id).unwrap()).cloned()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The straight RGBA of pixel (`x`, `y`) of image `id` as `d` draws it, on nothing.
fn drawn(d: &Document, id: NodeId, x: u32, y: u32) -> [u8; 4] {
    let im = image_of(d, id);
    let at = im.xf * vectorcraft_geom::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
    let r = vectorcraft_render::Renderer::new().render_region(d, Rect::new(at.x - 0.5, at.y - 0.5, at.x + 0.5, at.y + 0.5), 1.0, false);
    let px = r.to_straight();
    [px[0], px[1], px[2], px[3]]
}

#[test]
fn the_mask_hides_the_chosen_colors_while_the_pixels_stay() {
    let (mut s, id) = open(&framed());
    let before = (image_of(doc(&s), id), doc(&s).images[&image_of(doc(&s), id).key].bytes.clone(), undo_len(&s));
    assert_eq!(drawn(doc(&s), id, 0, 0), WHITE);
    let r = s.execute(COLOR_MASK, &hide_white()).unwrap();
    assert_eq!(r, json!({"changed": 1, "ids": [id.0], "hidden": 12 * 10 - 6 * 4 + 1}));
    let d = doc(&s);
    assert_eq!(image_of(d, id), before.0, "the image object is as it was");
    assert_eq!(d.images[&before.0.key].bytes, before.1, "its pixels are as they were");
    let m = mask_of(d, id).unwrap();
    assert_eq!((m.from.as_str(), m.background, m.contiguous), (before.0.key.as_str(), MaskBackground::Colors, false));
    assert_eq!(drawn(d, id, 0, 0)[3], 0, "white is hidden");
    assert_eq!(drawn(d, id, 6, 5)[3], 0, "inside the block too");
    assert_eq!(drawn(d, id, 4, 4), RED, "the red shows");
    assert_eq!(undo_len(&s), before.2 + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(mask_of(doc(&s), id), None);
    assert_eq!(drawn(doc(&s), id, 0, 0), WHITE);
}

#[test]
fn the_mask_is_edited_reported_and_removed() {
    let (mut s, id) = open(&framed());
    let r = s.execute(COLOR_MASK, &json!({})).unwrap();
    assert_eq!(
        r,
        json!({"background": "edges", "colors": "#ffffff", "contiguous": true, "match": "distance", "tolerance": 10.0, "softness": 10.0, "masked": false})
    );
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let steps = undo_len(&s);
    // The same settings again: nothing changes.
    assert_eq!(s.execute(COLOR_MASK, &hide_white()).unwrap()["changed"], 0);
    assert_eq!(undo_len(&s), steps, "no undo step");
    // Contiguous: the white inside the block shows again.
    let r = s.execute(COLOR_MASK, &json!({"background": "colors", "colors": "#ffffff", "tolerance": 0, "softness": 0})).unwrap();
    assert_eq!(r["changed"], 1);
    assert_eq!((drawn(doc(&s), id, 0, 0)[3], drawn(doc(&s), id, 6, 5)), (0, WHITE));
    let r = s.execute(COLOR_MASK, &json!({})).unwrap();
    assert_eq!(
        r,
        json!({"background": "colors", "colors": "#ffffff", "contiguous": true, "match": "distance", "tolerance": 0.0, "softness": 0.0, "masked": true})
    );
    let steps = undo_len(&s);
    assert_eq!(s.execute(REMOVE_COLOR_MASK, &json!({})).unwrap(), json!({"changed": 1, "ids": [id.0]}));
    assert_eq!(undo_len(&s), steps + 1);
    assert_eq!((mask_of(doc(&s), id), drawn(doc(&s), id, 0, 0)), (None, WHITE));
    // Nothing left to remove: the command is not enabled, and adds no undo step.
    let e = s.execute(REMOVE_COLOR_MASK, &json!({})).unwrap_err();
    assert!(e.to_string().contains("select an image with a color mask"), "{e}");
    assert_eq!(undo_len(&s), steps + 1);
}

#[test]
fn applying_the_mask_makes_it_part_of_the_pixels() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let m = mask_of(doc(&s), id).unwrap();
    let steps = undo_len(&s);
    assert_eq!(s.execute(APPLY_COLOR_MASK, &json!({})).unwrap(), json!({"changed": 1, "ids": [id.0]}));
    let d = doc(&s);
    assert_eq!((image_of(d, id).key, mask_of(d, id)), (m.key.clone(), None));
    let px = image::load_from_memory(&d.images[&m.key].bytes).unwrap().to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(4, 4).0), (0, RED));
    assert_eq!(drawn(d, id, 0, 0)[3], 0);
    assert_eq!(undo_len(&s), steps + 1);
    assert!(s.execute(APPLY_COLOR_MASK, &json!({})).is_err(), "no mask left");
}

/// The bytes of the first `data:image/png;base64,` image in `svg`.
fn svg_image(svg: &str) -> DynamicImage {
    let at = svg.find("data:image/png;base64,").expect("an embedded PNG") + "data:image/png;base64,".len();
    let end = at + svg[at..].find('"').unwrap();
    image::load_from_memory(&vectorcraft_format::base64_decode(&svg[at..end]).unwrap()).unwrap()
}

#[test]
fn outputs_write_the_masked_pixels() {
    let (mut s, _) = open(&framed());
    let export =
        |s: &Session, format: &str| crate::cmd::fileio::encode_all(doc(s), format, &json!({"useArtboards": true})).unwrap().files.remove(0).1;
    let (png0, svg0, pdf0, eps0) = (export(&s, "png"), export(&s, "svg"), export(&s, "pdf"), export(&s, "eps"));
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let alpha = |bytes: &[u8]| image::load_from_memory(bytes).unwrap().to_rgba8().get_pixel(0, 0).0[3];
    assert_eq!((alpha(&png0), alpha(&export(&s, "png"))), (255, 0), "PNG");
    let svg_alpha = |bytes: &[u8]| svg_image(std::str::from_utf8(bytes).unwrap()).to_rgba8().get_pixel(0, 0).0[3];
    assert_eq!((svg_alpha(&svg0), svg_alpha(&export(&s, "svg"))), (255, 0), "SVG");
    let has = |bytes: &[u8], what: &[u8]| bytes.windows(what.len()).any(|w| w == what);
    assert!(!has(&pdf0, b"/SMask") && has(&export(&s, "pdf"), b"/SMask"), "PDF: the image with an alpha channel");
    assert!(!has(&eps0, b"/MaskColor") && has(&export(&s, "eps"), b"/MaskColor"), "EPS: the hidden pixels masked out");
}

#[test]
fn the_mask_saves_and_opens_again() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let saved = vectorcraft_format::save_file(doc(&s));
    let back = vectorcraft_format::load(&saved).unwrap();
    assert_eq!(mask_of(&back, id), mask_of(doc(&s), id));
    let m = mask_of(&back, id).unwrap();
    assert_eq!(back.images[&m.key].bytes, doc(&s).images[&m.key].bytes, "the masked pixels are saved with it");
    assert_eq!(back.images[&m.from].bytes, doc(&s).images[&m.from].bytes, "and the image's own");
    assert_eq!(drawn(&back, id, 0, 0)[3], 0);
}

#[test]
fn copy_and_paste_carry_the_mask() {
    let (mut s, _) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    super::super::DECODES.with(|c| c.set(0));
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(super::super::DECODES.with(|c| c.get()), 0, "the masked pixels come along, so nothing is decoded to make them again");
    let id = s.doc().unwrap().selection.objects[0];
    let d = doc(&s);
    let m = mask_of(d, id).expect("the pasted image keeps its mask");
    assert!(m.made_for(&image_of(d, id)) && d.images.contains_key(&m.key), "with its masked pixels");
    assert_eq!(drawn(d, id, 0, 0)[3], 0);
    assert_eq!(drawn(d, id, 4, 4), RED);
}

#[test]
fn new_pixels_take_the_mask_again() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    // Crop Image replaces the image's own pixels: the mask is made for the cropped ones.
    let r = s.execute("object.cropImage", &json!({"rect": [0, 0, 6, 5]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap_or(id.0));
    let d = doc(&s);
    let (im, m) = (image_of(d, id), mask_of(d, id).unwrap());
    assert_eq!(m.from, im.key, "made for the cropped pixels");
    assert_eq!((drawn(d, id, 0, 0)[3], drawn(d, id, 4, 4)), (0, RED));
    // Pixels put in place by any edit (a link read again) take it too.
    let blue = png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(6, 5, |x, _| image::Rgba(if x < 3 { WHITE } else { [0, 0, 200, 255] }))));
    s.edit("Swap", |d, _| {
        let blob = ImageBlob::png(blue);
        let key = blob.content_key();
        d.images.insert(key.clone(), blob);
        if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
            im.key = key;
        }
        Ok(())
    })
    .unwrap();
    let d = doc(&s);
    assert_eq!(mask_of(d, id).unwrap().from, image_of(d, id).key);
    assert_eq!((drawn(d, id, 0, 0)[3], drawn(d, id, 4, 0)), (0, [0, 0, 200, 255]));
}

#[test]
fn a_document_read_with_new_pixels_takes_the_mask_again() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let mut d = doc(&s).clone();
    let blob = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(4, 4, image::Rgba(WHITE)))));
    let red = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x == 0 { WHITE } else { RED })))));
    let key = red.content_key();
    d.images.insert(key.clone(), red);
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.key = key.clone();
    }
    assert_eq!(drawn(&d, id, 0, 0), WHITE, "until the mask is made again, the image's own pixels");
    super::refresh(None, &mut d);
    assert_eq!(mask_of(&d, id).unwrap().from, key);
    assert_eq!(drawn(&d, id, 0, 0)[3], 0);
    // Pixels the mask would hide entirely: the mask stays, hiding nothing.
    let all = blob.content_key();
    d.images.insert(all.clone(), blob);
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.key = all.clone();
    }
    super::refresh(None, &mut d);
    let m = mask_of(&d, id).unwrap();
    assert_eq!((m.key.as_str(), m.from.as_str(), m.colors.as_slice()), (all.as_str(), all.as_str(), &[[255, 255, 255]][..]));
    assert_eq!(drawn(&d, id, 0, 0), WHITE);
}

/// A new document with a selected 12 × 10 image of `blob` under `key`, linked to `/nowhere/art.png`.
fn linked(key: &str, blob: ImageBlob) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let key = key.to_string();
    let id = s
        .edit("Place", |d, sel| {
            d.images.insert(key.clone(), blob);
            let id = d.alloc_id();
            let im = ImageObject {
                key,
                width: 12,
                height: 10,
                xf: Default::default(),
                link: Some(vectorcraft_doc::LinkInfo::new("/nowhere/art.png")),
                placement: Default::default(),
                pixels: None,
            };
            d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(im))))?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    (s, id)
}

/// A linked image keeps its link; outputs that refer to linked files write its masked pixels, and
/// applying the mask embeds it.
#[test]
fn a_linked_image_stays_linked_and_outputs_its_masked_pixels() {
    let (mut s, id) = linked("linked", ImageBlob::png(framed()));
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let d = doc(&s);
    assert_eq!((image_of(d, id).key.as_str(), image_of(d, id).link.map(|l| l.path)), ("linked", Some("/nowhere/art.png".to_string())));
    assert_eq!(drawn(d, id, 0, 0)[3], 0);
    // Images written as files beside the SVG: the masked pixels are written, not a link to the
    // linked file.
    let r = s.execute("document.serialize", &json!({"format": "svg", "images": "link"})).unwrap();
    let text = r["text"].as_str().unwrap();
    assert!(!text.contains("/nowhere/art.png"), "{text}");
    assert_eq!(r["linked"].as_array().map(Vec::len), Some(1), "{r}");
    let r = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    assert_eq!(svg_image(r["text"].as_str().unwrap()).to_rgba8().get_pixel(0, 0).0[3], 0);
    let m = mask_of(doc(&s), id).unwrap();
    s.execute(APPLY_COLOR_MASK, &json!({})).unwrap();
    let im = image_of(doc(&s), id);
    assert_eq!((im.key, im.link), (m.key, None), "applied: the masked pixels, embedded");
}

/// A mask that hides nothing (no pixel matches) leaves a linked image linked in outputs, and
/// applying it only takes it off: the image keeps its pixels and its link.
#[test]
fn a_mask_that_hides_nothing_keeps_the_link() {
    let (mut s, id) = linked("linked", ImageBlob::png(framed()));
    let r = s.execute(COLOR_MASK, &json!({"background": "colors", "colors": "#0000ff", "contiguous": false, "tolerance": 0, "softness": 0})).unwrap();
    assert_eq!((r["changed"].as_u64(), r["hidden"].as_u64()), (Some(1), Some(0)));
    assert_eq!(mask_of(doc(&s), id).unwrap().key, "linked", "the masked pixels are the image's own");
    let r = s.execute("document.serialize", &json!({"format": "svg", "images": "link"})).unwrap();
    assert!(r["text"].as_str().unwrap().contains("/nowhere/art.png"), "{r}");
    assert_eq!(s.execute(APPLY_COLOR_MASK, &json!({})).unwrap()["changed"], 1);
    let im = image_of(doc(&s), id);
    assert_eq!((im.key.as_str(), im.link.map(|l| l.path), mask_of(doc(&s), id)), ("linked", Some("/nowhere/art.png".to_string()), None));
}

/// Color Mask refuses a linked image that shows its preview and an image too large to change,
/// with its own command's errors.
#[test]
fn color_mask_refuses_previews_and_images_too_large() {
    let preview = std::sync::Arc::new(framed());
    let (mut s, id) = linked("missing", ImageBlob { mime: "image/png".into(), bytes: preview.clone(), proxy: Some(preview) });
    let e = s.execute(COLOR_MASK, &hide_white()).unwrap_err().to_string();
    assert!(e.contains(COLOR_MASK) && e.contains("the linked image shows its preview: embed or relink it first"), "{e}");
    assert_eq!(mask_of(doc(&s), id), None);
    // Without settings, the dialog's query reads the settings and opens on the defaults.
    assert_eq!(s.execute(COLOR_MASK, &json!({})).unwrap()["masked"], json!(false));
    let (mut s, _) = open(&crate::tests_linkspanel::png_header(10_632, 10_631));
    let e = s.execute(COLOR_MASK, &hide_white()).unwrap_err().to_string();
    assert!(e.contains(COLOR_MASK) && e.contains("an image of 10632 × 10631 pixels is too large to change"), "{e}");
}

/// Crop Image off the pixel grid cuts the image's own pixels, so the hidden colors stay in them
/// and show again once the mask is removed.
#[test]
fn cropping_off_the_pixel_grid_keeps_the_hidden_colors_in_the_pixels() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    s.execute("object.cropImage", &json!({"rect": [0.5, 0.5, 6, 5]})).unwrap();
    let d = doc(&s);
    let (im, m) = (image_of(d, id), mask_of(d, id).unwrap());
    assert_eq!(m.from, im.key, "the mask is made for the cut pixels");
    let own = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert!(own.pixels().all(|p| p.0[3] == 255), "the cut pixels are the image's own, white included");
    assert_eq!(drawn(d, id, 0, 0)[3], 0, "white is hidden");
    s.execute(REMOVE_COLOR_MASK, &json!({"ids": [id.0]})).unwrap();
    assert_eq!(drawn(doc(&s), id, 0, 0)[3], 255);
}

/// Apply Color Mask on a mask made for older pixels makes it again for the image's own pixels,
/// and masked pixels of another size than the object records keep its place and size.
#[test]
fn applying_a_mask_made_for_older_pixels_makes_it_again_and_keeps_the_size() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let before = doc(&s).node(id).unwrap().geometric_bounds().unwrap();
    // Pixels twice the size, put in place without an edit, so no refresh has run.
    let big = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(24, 20, |x, y| {
        let block = (6..18).contains(&x) && (6..14).contains(&y);
        image::Rgba(if block { RED } else { WHITE })
    }))));
    let key = big.content_key();
    let st = s.doc_mut().unwrap();
    let d = std::sync::Arc::make_mut(&mut st.doc);
    d.images.insert(key.clone(), big);
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.key = key.clone();
    }
    assert_eq!(s.execute(APPLY_COLOR_MASK, &json!({})).unwrap()["changed"], 1);
    let d = doc(&s);
    let im = image_of(d, id);
    assert_ne!(im.key, key, "the masked pixels of the new pixels");
    assert_eq!((im.width, im.height, mask_of(d, id)), (24, 20, None));
    let after = d.node(id).unwrap().geometric_bounds().unwrap();
    assert!((after.width() - before.width()).abs() < 1e-9 && (after.height() - before.height()).abs() < 1e-9, "{before:?} {after:?}");
    let px = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(10, 10).0), (0, RED));
}

/// A 12 × 10 grayscale image without alpha: white, with a black block from (3, 3) to (9, 7).
fn gray_framed() -> Vec<u8> {
    png(DynamicImage::ImageLuma8(image::GrayImage::from_fn(12, 10, |x, y| {
        image::Luma([if (3..9).contains(&x) && (3..7).contains(&y) { 0 } else { 255 }])
    })))
}

/// Masked pixels have an alpha channel: a grayscale image colorized with a process color can't
/// take a mask, and a masked one can't take a process color.
#[test]
fn a_process_color_does_not_colorize_masked_gray_pixels() {
    const REFUSED: &str = "a process color can't colorize a grayscale image with transparency";
    let (mut s, id) = open(&gray_framed());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert!(doc(&s).node(id).unwrap().extras().colorize.is_some(), "a gray image without alpha takes a process color");
    let (before, steps) = (doc(&s).clone(), undo_len(&s));
    let e = s.execute(COLOR_MASK, &hide_white()).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
    assert_eq!((doc(&s), undo_len(&s)), (&before, steps));
    // Black colorizes gray pixels with alpha: the mask is set, then a process color is refused.
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    assert_eq!(s.execute(COLOR_MASK, &hide_white()).unwrap()["changed"], 1);
    let e = s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
}

/// Images outside the layers (a symbol's art, opacity-mask art) take their masks again too when
/// their own pixels are replaced, as Update Link replaces them there.
#[test]
fn masks_outside_the_layers_are_made_again() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let mut d = doc(&s).clone();
    let masked = d.node(id).unwrap().clone();
    let red = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x == 0 { WHITE } else { RED })))));
    let key = red.content_key();
    d.images.insert(key.clone(), red);
    let mut moved = masked.clone();
    if let NodeKind::Image(im) = &mut moved.kind {
        im.key = key.clone();
    }
    d.symbols.push(vectorcraft_doc::Symbol { name: "Logo".into(), art: std::sync::Arc::new(moved.clone()), dynamic: false });
    let holder_id = d.alloc_id();
    let mut holder = vectorcraft_doc::Node::new(holder_id, NodeKind::Group { children: vec![], clip: false });
    holder.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(moved, false)));
    d.insert(d.default_layer(), 0, holder).unwrap();
    super::refresh(None, &mut d);
    let from = |n: &vectorcraft_doc::Node| Document::color_mask_of(n).unwrap().from.clone();
    assert_eq!(from(&d.symbols.last().unwrap().art), key, "in the symbol's art");
    let art = &d.node(holder_id).unwrap().extras().mask.as_ref().unwrap().art;
    assert_eq!(from(art), key, "in the opacity mask's art");
    let m = Document::color_mask_of(art).unwrap();
    let px = image::load_from_memory(&d.images[&m.key].bytes).unwrap().to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(1, 0).0), (0, RED));
}

/// `ids` stand in for the selection: an image that is not selected is changed by its id.
#[test]
fn images_given_by_id_need_not_be_selected() {
    let (mut s, id) = open(&framed());
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(
        s.execute(COLOR_MASK, &json!({"ids": [id.0], "background": "colors", "colors": "#fff", "tolerance": 0, "softness": 0})).unwrap()["changed"],
        1
    );
    assert_eq!(s.execute(REMOVE_COLOR_MASK, &json!({"ids": [id.0]})).unwrap()["changed"], 1);
    s.execute(COLOR_MASK, &json!({"ids": [id.0], "background": "colors", "colors": "#fff", "tolerance": 0, "softness": 0})).unwrap();
    assert_eq!(s.execute(APPLY_COLOR_MASK, &json!({"ids": [id.0]})).unwrap()["changed"], 1);
    assert_eq!(mask_of(doc(&s), id), None);
}

#[test]
fn what_cannot_be_masked_is_refused_and_nothing_changes() {
    let (mut s, id) = open(&framed());
    let steps = undo_len(&s);
    let e = s.execute(COLOR_MASK, &json!({"tolerance": 101})).unwrap_err();
    assert!(e.to_string().contains("`tolerance` must be from 0 to 100"), "{e}");
    let e = s
        .execute(COLOR_MASK, &json!({"background": "colors", "colors": "#fff, #c80000", "contiguous": false, "tolerance": 0, "softness": 0}))
        .unwrap_err();
    assert!(e.to_string().contains("the mask would hide every pixel"), "{e}");
    assert_eq!((mask_of(doc(&s), id), undo_len(&s)), (None, steps));
    let inks = vectorcraft_doc::cmyk::Inks::new(4, 2, [0, 0, 0, 0].repeat(8)).unwrap();
    let (mut s, _) = open(&ImageBlob::cmyk_tiff(&inks).unwrap().bytes);
    let e = s.execute(COLOR_MASK, &hide_white()).unwrap_err();
    assert!(e.to_string().contains("a CMYK image can't take a color mask"), "{e}");
    // Without settings, the dialog's query reads the settings and opens on the defaults.
    assert_eq!(s.execute(COLOR_MASK, &json!({})).unwrap()["masked"], json!(false));
}

/// Remove Color Mask and Apply Color Mask refuse an object in `ids` that is not an image, as
/// Color Mask does.
#[test]
fn an_object_that_is_not_an_image_is_refused() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap()["id"].as_u64().unwrap();
    let steps = undo_len(&s);
    for cmd in [COLOR_MASK, REMOVE_COLOR_MASK, APPLY_COLOR_MASK] {
        let mut p = if cmd == COLOR_MASK { hide_white() } else { json!({}) };
        p["ids"] = json!([id.0, rect]);
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains(&format!("object {rect} is not an image")), "{cmd}: {e}");
    }
    assert!(mask_of(doc(&s), id).is_some());
    assert_eq!(undo_len(&s), steps);
}

/// A mask that can't be made for the image's pixels (they are missing) is not made again on every
/// later edit: the art holding it is left as it is.
#[test]
fn a_mask_that_cannot_be_made_is_not_made_again_on_every_edit() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let mut d = doc(&s).clone();
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.key = "gone".into();
    }
    super::refresh(None, &mut d);
    assert_eq!(mask_of(&d, id).unwrap().key, "gone", "it hides nothing");
    let layer = d.layers[0].clone();
    super::refresh(None, &mut d);
    assert!(std::sync::Arc::ptr_eq(&layer, &d.layers[0]), "the second pass changes nothing");
}

/// The original of a pattern being edited takes its masks again too.
#[test]
fn the_pattern_being_edited_takes_its_masks_again() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let mut d = doc(&s).clone();
    let mut art = d.node(id).unwrap().clone();
    let red = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(4, 4, |x, _| image::Rgba(if x == 0 { WHITE } else { RED })))));
    let key = red.content_key();
    d.images.insert(key.clone(), red);
    if let NodeKind::Image(im) = &mut art.kind {
        im.key = key.clone();
    }
    let original = vectorcraft_doc::PatternDef::new("Tile", vec![std::sync::Arc::new(art)]);
    d.pattern_edit = Some(vectorcraft_doc::PatternEdit { pattern: "Tile".into(), layer: NodeId(999_999), original: Some(original) });
    super::refresh(None, &mut d);
    let original = d.pattern_edit.as_ref().and_then(|e| e.original.as_ref()).unwrap();
    assert_eq!(Document::color_mask_of(&original.art[0]).unwrap().from, key);
}

/// A linked file read again as the document opens (Update Links: Automatically) takes the mask
/// for its new pixels.
#[test]
fn a_linked_file_read_again_on_open_takes_the_mask_again() {
    let dir = crate::tests_links::Folder::new("color-mask");
    let pic = dir.file("art.png");
    crate::tests_links::write(&pic, &framed());
    let mut s = crate::tests_links::session();
    let id = crate::tests_links::place(&mut s, &pic);
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let saved = dir.file("art.vectorcraft");
    crate::tests_links::save(&mut s, &saved);
    // New pixels in the file: blue where the block was red.
    let blue = png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(12, 10, |x, y| {
        image::Rgba(if (3..9).contains(&x) && (3..7).contains(&y) { [0, 0, 200, 255] } else { WHITE })
    })));
    crate::tests_links::write(&pic, &blue);
    s.execute("prefs.set", &json!({"key": "updateLinks", "value": "automatically"})).unwrap();
    let r = crate::tests_links::open(&mut s, &saved);
    assert_eq!(r["updatedLinks"][0]["ids"], json!([id.0]), "{r}");
    let d = doc(&s);
    let (im, m) = (image_of(d, id), mask_of(d, id).unwrap());
    assert_eq!(m.from, im.key, "made for the file's new pixels");
    let px = image::load_from_memory(&d.images[&m.key].bytes).unwrap().to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(4, 4).0), (0, [0, 0, 200, 255]));
}

/// The bytes of every file `format` writes for `d` with `p`.
fn written(d: &Document, format: &str, p: Value) -> Vec<Vec<u8>> {
    crate::cmd::fileio::encode_all(d, format, &p).unwrap().files.into_iter().map(|f| f.1).collect()
}

/// `out` for the opened image without a mask, with one hiding its red block, and with the mask
/// removed again: the mask changes what is written, and removing it gives the same output as
/// before. (Outputs that flatten see-through pixels over the white page show the hidden block as
/// white.)
fn written_with_the_mask<T: PartialEq + std::fmt::Debug>(out: impl Fn(&Document) -> T) {
    let (mut s, _) = open(&framed());
    let plain = out(doc(&s));
    s.execute(COLOR_MASK, &json!({"background": "colors", "colors": "#c80000", "contiguous": false, "tolerance": 0, "softness": 0})).unwrap();
    let masked = out(doc(&s));
    s.execute(REMOVE_COLOR_MASK, &json!({})).unwrap();
    assert_ne!(plain, masked, "the masked pixels are written");
    assert_eq!(out(doc(&s)), plain, "without the mask, the image's own");
}

#[test]
fn dxf_writes_the_masked_pixels() {
    written_with_the_mask(|d| written(d, "dxf", json!({})));
}

#[test]
fn emf_and_wmf_write_the_masked_pixels() {
    written_with_the_mask(|d| written(d, "emf", json!({})));
    written_with_the_mask(|d| written(d, "wmf", json!({})));
}

#[test]
fn legacy_ai_writes_the_masked_pixels() {
    written_with_the_mask(|d| written(d, "ai", json!({"aiVersion": 7})));
}

#[test]
fn ai_editing_data_writes_the_masked_pixels() {
    let o = vectorcraft_eps::AiEditingOptions { title: "art".into(), created: Some(0) };
    written_with_the_mask(|d| vectorcraft_eps::export_ai_editing(d, &o).unwrap().program);
}

/// PDF/X-1a and PDF/X-3 flatten a masked image, whose hidden pixels are transparency, and write no
/// soft mask.
#[test]
fn pdf_x1a_and_x3_flatten_the_hidden_pixels() {
    for standard in ["pdfX1a", "pdfX3"] {
        let (mut s, _) = open(&framed());
        let export = |s: &mut Session| {
            let v = s.execute("document.exportPdf", &json!({"standard": standard, "compression": {"compressText": false}})).unwrap();
            let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
            (String::from_utf8_lossy(&bytes).into_owned(), v["warnings"].to_string())
        };
        let (_, warnings) = export(&mut s);
        assert!(!warnings.contains("flattened"), "{standard}, an opaque image: {warnings}");
        s.execute(COLOR_MASK, &hide_white()).unwrap();
        let (text, warnings) = export(&mut s);
        assert!(warnings.contains("flattened"), "{standard}: {warnings}");
        assert!(!text.contains("/SMask"), "{standard}");
    }
}

/// The legacy .ai flattens a masked image, whose hidden pixels are transparency, as it flattens an
/// image whose background was removed, and `transparentImages: "omit"` leaves both out.
#[test]
fn legacy_ai_flattens_a_masked_image_as_a_see_through_one() {
    let warnings = |s: &Session, p: &Value| crate::cmd::fileio::encode_all(doc(s), "ai", p).unwrap().warnings;
    let (mut masked, _) = open(&framed());
    masked.execute(COLOR_MASK, &hide_white()).unwrap();
    let (mut cleared, _) = open(&framed());
    cleared.execute(super::super::REMOVE_BACKGROUND, &hide_white()).unwrap();
    for (p, said) in [(json!({"aiVersion": 7}), "are flattened"), (json!({"aiVersion": 7, "transparentImages": "omit"}), "are left out")] {
        let w = warnings(&masked, &p);
        assert!(w.iter().any(|w| w.contains("transparent images") && w.contains(said)), "{p}: {w:?}");
        assert_eq!(w, warnings(&cleared, &p), "{p}");
    }
}

/// A pattern whose tile holds a masked image counts as one with a see-through image, so outputs
/// without transparency expand it before they flatten.
#[test]
fn a_pattern_tile_with_a_masked_image_counts_as_see_through() {
    let href = format!("data:image/png;base64,{}", vectorcraft_format::base64_encode(&framed()));
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100" viewBox="0 0 100 100"><defs><pattern id="p" patternUnits="userSpaceOnUse" width="12" height="10"><image width="12" height="10" xlink:href="{href}"/></pattern></defs><rect width="100" height="100" fill="url(#p)"/></svg>"##
    );
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "p.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let mut d = doc(&s).clone();
    let see_through =
        |d: &Document| crate::cmd::expand::has_transparent_image_patterns(d, &crate::cmd::flatten::see_through_images(d), &|_, _| false);
    assert!(!see_through(&d), "an opaque tile");
    let hidden = png(DynamicImage::ImageRgba8(image::RgbaImage::from_fn(12, 10, |x, _| image::Rgba(if x == 0 { [0; 4] } else { RED }))));
    d.images.insert("masked".into(), ImageBlob::png(hidden));
    fn image_in(n: &mut vectorcraft_doc::Node) -> Option<&mut vectorcraft_doc::Node> {
        if matches!(n.kind, NodeKind::Image(_)) {
            return Some(n);
        }
        n.children_mut()?.iter_mut().find_map(|c| image_in(std::sync::Arc::make_mut(c)))
    }
    let node = d.patterns.iter_mut().flat_map(|p| p.art.iter_mut()).find_map(|a| image_in(std::sync::Arc::make_mut(a))).unwrap();
    let NodeKind::Image(im) = &node.kind else { panic!("the tile's image") };
    let from = im.key.clone();
    node.extras_mut().color_mask = Some(Box::new(ColorMask {
        key: "masked".into(),
        from,
        background: MaskBackground::Colors,
        colors: vec![[255, 255, 255]],
        contiguous: false,
        measure: vectorcraft_doc::color_mask::MaskMatch::Distance,
        tolerance: 0.0,
        softness: 0.0,
    }));
    assert!(see_through(&d), "the tile shows its masked pixels");
}

/// SVG writes a colorized gray image with a mask as the coverage of its masked pixels, shared by
/// the objects that show it.
#[test]
fn svg_writes_a_colorized_masked_image_as_it_shows() {
    let (mut s, _) = open(&gray_framed());
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute(COLOR_MASK, &json!({"background": "colors", "colors": "#000000", "contiguous": false, "tolerance": 0, "softness": 0})).unwrap();
    s.execute("edit.duplicate", &json!({})).unwrap();
    let r = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    let text = r["text"].as_str().unwrap();
    assert_eq!(text.matches("data:image/png;base64,").count(), 1, "one shared image: {text}");
    assert_eq!(svg_image(text).to_rgba8().get_pixel(4, 4).0[3], 0, "the hidden black block covers nothing");
}

/// The opened masked image moved into what `wrap` makes of it (given the document, to take ids
/// from, and the image node).
fn wrapped(wrap: impl FnOnce(&mut Document, vectorcraft_doc::Node) -> vectorcraft_doc::Node) -> (Session, NodeId) {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    s.edit("Wrap", |d, _| {
        let (parent, index, _) = d.position(id).unwrap();
        let image = (*d.remove(id)?).clone();
        let holder = wrap(d, image);
        d.insert(parent, index, holder)?;
        Ok(())
    })
    .unwrap();
    (s, id)
}

/// The rectangle of the opened image's frame (12 × 10 at the origin) filled with `fill`.
fn frame(d: &mut Document, fill: vectorcraft_color::Paint) -> vectorcraft_doc::Node {
    let rect = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 12.0, 10.0));
    vectorcraft_doc::Node::path(d.alloc_id(), rect, vectorcraft_doc::Appearance::basic(fill, vectorcraft_color::Paint::None, 0.0))
}

/// An image in a clip group of its own frame, which the canvas draws straight into the clipping
/// path, shows its masked pixels.
#[test]
fn a_clipped_image_draws_its_masked_pixels() {
    let (s, id) = wrapped(|d, image| {
        let clip = frame(d, vectorcraft_color::Paint::None);
        vectorcraft_doc::Node::new(
            d.alloc_id(),
            NodeKind::Group { children: vec![std::sync::Arc::new(clip), std::sync::Arc::new(image)], clip: true },
        )
    });
    assert_eq!((drawn(doc(&s), id, 0, 0)[3], drawn(doc(&s), id, 4, 4)), (0, RED));
}

/// An image whose opacity mask is one white shape, which the canvas draws straight into the
/// shape, shows its masked pixels.
#[test]
fn an_image_masked_by_a_white_shape_draws_its_masked_pixels() {
    let (s, id) = wrapped(|d, image| {
        let white = frame(d, vectorcraft_color::Paint::solid(vectorcraft_color::Color::WHITE));
        image.with_mask(Some(Box::new(vectorcraft_doc::OpacityMask::new(white, true))))
    });
    assert_eq!((drawn(doc(&s), id, 0, 0)[3], drawn(doc(&s), id, 4, 4)), (0, RED));
}

/// Pasting into a document that has other pixels under the image's names renames them, and the
/// mask follows: the masked pixels come along under their new name, nothing is decoded again.
#[test]
fn paste_renames_the_masked_pixels_with_the_image() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let m = mask_of(doc(&s), id).unwrap();
    let masked = doc(&s).images[&m.key].bytes.clone();
    s.execute("edit.copy", &json!({})).unwrap();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let other = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba(RED)))));
    s.edit("Taken names", |d, _| {
        d.images.insert(m.key.clone(), other.clone());
        d.images.insert(m.from.clone(), other.clone());
        Ok(())
    })
    .unwrap();
    super::super::DECODES.with(|c| c.set(0));
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(super::super::DECODES.with(|c| c.get()), 0, "nothing is decoded to make the mask again");
    let id = s.doc().unwrap().selection.objects[0];
    let d = doc(&s);
    let (im, pasted) = (image_of(d, id), mask_of(d, id).unwrap());
    assert!(pasted.key != m.key && pasted.from != m.from, "renamed: {pasted:?}");
    assert!(pasted.made_for(&im));
    assert_eq!(d.images[&pasted.key].bytes, masked);
    assert_eq!((drawn(d, id, 0, 0)[3], drawn(d, id, 4, 4)), (0, RED));
}

/// Placing a document (as an editable copy) whose image has a mask brings the masked pixels, and
/// the mask follows names the placing document already uses for other pixels.
#[test]
fn placing_a_document_brings_the_masked_pixels() {
    let (mut s, id) = open(&framed());
    s.execute(COLOR_MASK, &hide_white()).unwrap();
    let m = mask_of(doc(&s), id).unwrap();
    let file = vectorcraft_format::save_file(doc(&s));
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let other = ImageBlob::png(png(DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(2, 2, image::Rgba(RED)))));
    s.edit("Taken names", |d, _| {
        d.images.insert(m.from.clone(), other.clone());
        Ok(())
    })
    .unwrap();
    super::super::DECODES.with(|c| c.set(0));
    let r =
        s.execute("file.place", &json!({"name": "art.vectorcraft", "dataBase64": vectorcraft_format::base64_encode(&file), "link": false})).unwrap();
    assert_eq!(super::super::DECODES.with(|c| c.get()), 0, "nothing is decoded to make the mask again: {r}");
    let d = doc(&s);
    let mut found = vec![];
    for root in d.art_roots() {
        root.walk(&mut |n| {
            if let (NodeKind::Image(im), Some(m)) = (&n.kind, Document::color_mask_of(n)) {
                found.push(((**im).clone(), m.clone()));
            }
        });
    }
    let [(im, placed)] = found.as_slice() else { panic!("one masked image: {found:?}") };
    assert!(placed.made_for(im) && placed.from != m.from, "the image was renamed and the mask follows: {placed:?}");
    let px = image::load_from_memory(&d.images[&placed.key].bytes).unwrap().to_rgba8();
    assert_eq!((px.get_pixel(0, 0).0[3], px.get_pixel(4, 4).0), (0, RED));
}
