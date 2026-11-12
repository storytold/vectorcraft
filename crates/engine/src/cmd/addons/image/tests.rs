use std::io::Cursor;
use std::sync::Arc;

use image::{DynamicImage, ImageBuffer, Luma, Rgb};
use serde_json::{Value, json};
use vectorcraft_doc::{ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use super::{Background, Match, REMOVE_BACKGROUND as C, Settings, level, levels};
use crate::Session;
use crate::cmd::fileio::ppi;

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [200, 0, 0, 255];

/// The command's defaults with `background`.
fn settings(background: Background) -> Settings {
    Settings { background, contiguous: true, measure: Match::Distance, tolerance: 10.0, softness: 10.0 }
}

fn png(img: DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// 12 × 10 pixels: white, with a red block from (3, 3) to (9, 7) that holds one white pixel at
/// (6, 5), which no white edge pixel reaches.
fn framed() -> image::RgbaImage {
    image::RgbaImage::from_fn(12, 10, |x, y| {
        let block = (3..9).contains(&x) && (3..7).contains(&y);
        image::Rgba(if block && (x, y) != (6, 5) { RED } else { WHITE })
    })
}

/// A session with `bytes` opened as a document, its image selected.
fn open(bytes: &[u8]) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    (s, id)
}

fn image_of(s: &Session, id: NodeId) -> ImageObject {
    let NodeKind::Image(im) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("an image") };
    (**im).clone()
}

fn bytes_of(s: &Session, id: NodeId) -> Arc<Vec<u8>> {
    s.doc().unwrap().doc.images[&image_of(s, id).key].bytes.clone()
}

fn pixels(s: &Session, id: NodeId) -> DynamicImage {
    image::load_from_memory(&bytes_of(s, id)).unwrap()
}

fn alpha(img: &DynamicImage, x: u32, y: u32) -> u8 {
    img.to_rgba8().get_pixel(x, y).0[3]
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn the_edge_colors_are_cleared_where_they_reach_in_from_an_edge() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let before = (image_of(&s, id), undo_len(&s));
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(r, json!({"changed": 1, "ids": [id.0], "cleared": 12 * 10 - 6 * 4}));
    let out = pixels(&s, id);
    assert_eq!(out.color(), image::ColorType::Rgba8);
    assert_eq!((alpha(&out, 0, 0), alpha(&out, 11, 9), alpha(&out, 2, 4)), (0, 0, 0), "the white around the block");
    assert_eq!(out.to_rgba8().get_pixel(4, 4).0, RED, "the block");
    assert_eq!(alpha(&out, 6, 5), 255, "white inside the block, which no edge reaches");
    let after = image_of(&s, id);
    assert_eq!((after.width, after.height, after.xf), (12, 10, before.0.xf), "the same place and size");
    assert_eq!(undo_len(&s), before.1 + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(image_of(&s, id), before.0);
}

/// Picked colors, like the other backgrounds, are cleared where they reach in from an edge, and with
/// `contiguous: false` wherever they are.
#[test]
fn picked_colors_are_cleared_from_the_edges_or_wherever_they_are() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    s.execute(C, &json!({"background": "colors", "colors": ["#fff"], "tolerance": 0, "softness": 0})).unwrap();
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 0, 0), alpha(&out, 6, 5), alpha(&out, 4, 4)), (0, 255, 255), "not inside the block");
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    s.execute(C, &json!({"background": "colors", "colors": ["#fff"], "contiguous": false, "tolerance": 0, "softness": 0})).unwrap();
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 0, 0), alpha(&out, 6, 5), alpha(&out, 4, 4)), (0, 0, 255), "inside the block too; the red stays");
}

/// With `contiguous: false` the edge color is cleared inside the subject too.
#[test]
fn the_edge_color_can_be_cleared_wherever_it_is() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let r = s.execute(C, &json!({"contiguous": false})).unwrap();
    assert_eq!(r["cleared"], 12 * 10 - 6 * 4 + 1);
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 0, 0), alpha(&out, 6, 5), alpha(&out, 4, 4)), (0, 0, 255));
}

/// `colors` as one string split by commas and spaces, written as other color params take them.
#[test]
fn picked_colors_can_be_one_string_of_colors() {
    for colors in ["#00ff00, ffffff", "green white", "#0f0,#FFF"] {
        let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
        s.execute(C, &json!({"background": "colors", "colors": colors, "contiguous": false, "tolerance": 0, "softness": 0})).unwrap();
        let out = pixels(&s, id);
        assert_eq!((alpha(&out, 0, 0), alpha(&out, 6, 5), alpha(&out, 4, 4)), (0, 0, 255), "{colors}");
    }
}

/// A subject along a whole edge stays: only the most common edge color is the background.
#[test]
fn a_subject_along_one_edge_is_not_the_background() {
    let img = image::RgbaImage::from_fn(10, 10, |_, y| image::Rgba(if y >= 7 { RED } else { WHITE }));
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(img)));
    s.execute(C, &json!({})).unwrap();
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 5, 0), alpha(&out, 5, 9), alpha(&out, 0, 8)), (0, 255, 255));
}

/// The preview steps of one interaction, as the dialog's are, decode an image once and give what a
/// run on freshly decoded pixels gives. The pixels are kept only until the interaction ends (OK or
/// Cancel), and a run that is not a preview step keeps none.
#[test]
fn preview_steps_on_the_same_pixels_decode_them_once() {
    let decodes = || super::DECODES.with(std::cell::Cell::get);
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let start = decodes();
    let p = json!({"background": "colors", "colors": "#c80000", "contiguous": false, "tolerance": 0, "softness": 0});
    s.begin_interaction("Remove Background").unwrap();
    s.preview(C, &json!({"tolerance": 0, "softness": 0})).unwrap();
    s.preview(C, &json!({"tolerance": 20, "softness": 0})).unwrap();
    s.preview(C, &p).unwrap();
    assert_eq!((decodes() - start, s.decoded_images.len()), (1, 1), "decoded once for three preview steps");
    s.commit_interaction().unwrap();
    assert_eq!(s.decoded_images.len(), 0, "OK lets the pixels go");
    let (mut fresh, fresh_id) = open(&png(DynamicImage::ImageRgba8(framed())));
    fresh.execute(C, &p).unwrap();
    assert_eq!((decodes() - start, fresh.decoded_images.len()), (2, 0), "a run that is not a preview step keeps nothing");
    assert_eq!(bytes_of(&s, id), bytes_of(&fresh, fresh_id), "the same result");
    assert_eq!((alpha(&pixels(&s, id), 4, 4), alpha(&pixels(&s, id), 0, 0)), (0, 255), "the red cleared, the white kept");
    s.execute("edit.undo", &json!({})).unwrap();
    s.begin_interaction("Remove Background").unwrap();
    s.preview(C, &json!({})).unwrap();
    assert_eq!(s.decoded_images.len(), 1);
    s.cancel_interaction().unwrap();
    assert_eq!(s.decoded_images.len(), 0, "Cancel lets the pixels go");
    s.execute(C, &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute(C, &json!({})).unwrap();
    assert_eq!(decodes() - start, 5, "each run that is not a preview step decodes");
}

/// Pixels a preview step takes from the step before keep what they were decoded with: a 16-bit gray
/// image with an ICC profile comes out as a run on freshly decoded pixels gives it.
#[test]
fn reused_pixels_keep_their_depth_gray_and_profile() {
    use image::{ImageDecoder as _, ImageEncoder as _};
    let icc = b"a profile's bytes".to_vec();
    let gray16 = ImageBuffer::from_fn(8, 8, |x, y| Luma([if (2..6).contains(&x) && (2..6).contains(&y) { 7_000u16 } else { 61_000 }]));
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder.set_icc_profile(icc.clone()).unwrap();
    DynamicImage::ImageLuma16(gray16).write_with_encoder(encoder).unwrap();
    let decodes = || super::DECODES.with(std::cell::Cell::get);
    let (mut s, id) = open(&bytes);
    let start = decodes();
    s.begin_interaction("Remove Background").unwrap();
    s.preview(C, &json!({"tolerance": 0, "softness": 0})).unwrap();
    s.preview(C, &json!({})).unwrap();
    assert_eq!(decodes() - start, 1, "the second step took the pixels of the first");
    let (mut fresh, fresh_id) = open(&bytes);
    fresh.execute(C, &json!({})).unwrap();
    assert_eq!(bytes_of(&s, id), bytes_of(&fresh, fresh_id), "the same result");
    let DynamicImage::ImageLumaA16(out) = pixels(&s, id) else { panic!("16-bit gray with alpha") };
    assert_eq!((out.get_pixel(0, 0).0, out.get_pixel(3, 3).0), ([61_000, 0], [7_000, 65_535]));
    let result = bytes_of(&s, id);
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(result.as_slice())).unwrap();
    assert_eq!(decoder.icc_profile().unwrap(), Some(icc));
}

/// Once the background is cleared, most edge pixels are clear and a second run clears nothing: the
/// subject's color along an edge is not taken for the background.
#[test]
fn a_second_run_leaves_the_subject() {
    let img = image::RgbaImage::from_fn(10, 10, |x, y| {
        image::Rgba(if y >= 7 {
            RED
        } else if (4..6).contains(&x) && (3..5).contains(&y) {
            [0, 0, 200, 255]
        } else {
            WHITE
        })
    });
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(img)));
    s.execute(C, &json!({})).unwrap();
    let (first, steps) = (image_of(&s, id), undo_len(&s));
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(r, json!({"changed": 0, "ids": [], "cleared": 0}));
    assert_eq!((image_of(&s, id), undo_len(&s)), (first, steps), "no change and no undo step");
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 5, 0), alpha(&out, 5, 9), alpha(&out, 4, 3)), (0, 255, 255), "the red and the blue stay");
}

#[test]
fn tolerance_and_softness_set_how_near_a_color_is_cleared() {
    // White; 1.96 % from white; 21.6 % from white.
    let px = [WHITE, [250, 250, 250, 255], [200, 200, 200, 255]];
    let at = |i: usize| px[i].map(|v| u16::from(v) * 257);
    let run = |tolerance, softness| levels(3, 1, &at, &Settings { tolerance, softness, ..settings(Background::Colors(vec![[255; 3]])) });
    assert_eq!(run(0.0, 0.0), [0, 255, 255], "only white itself at 0");
    assert_eq!(run(2.0, 0.0), [0, 0, 255]);
    assert_eq!(run(0.0, 10.0), [0, 50, 255], "1.96 % into a 10 % fade");
    assert_eq!(run(100.0, 0.0), [0, 0, 0], "everything at 100");
}

#[test]
fn a_result_with_no_pixel_left_is_refused_and_changes_nothing() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let before = (image_of(&s, id), undo_len(&s));
    let e = s.execute(C, &json!({"background": "colors", "colors": "#ffffff", "tolerance": 100})).unwrap_err();
    assert!(e.to_string().contains("every pixel matches the background"), "{e}");
    assert_eq!((image_of(&s, id), undo_len(&s)), before);
}

#[test]
fn an_image_whose_pixels_do_not_change_is_left_with_no_undo_step() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let before = (image_of(&s, id), undo_len(&s));
    let r = s.execute(C, &json!({"background": "colors", "colors": "#00ff00", "tolerance": 0, "softness": 0})).unwrap();
    assert_eq!(r, json!({"changed": 0, "ids": [], "cleared": 0}));
    assert_eq!((image_of(&s, id), undo_len(&s)), before);
    // An image whose edges and corners are already clear has no edge or corner color to clear,
    // whatever color the clear pixels hold.
    let ring =
        image::RgbaImage::from_fn(6, 6, |x, y| image::Rgba(if (1..5).contains(&x) && (1..5).contains(&y) { WHITE } else { [255, 255, 255, 0] }));
    for p in [json!({}), json!({"background": "corners", "contiguous": false})] {
        let (mut s, _) = open(&png(DynamicImage::ImageRgba8(ring.clone())));
        assert_eq!(s.execute(C, &p).unwrap()["changed"], 0, "{p}");
    }
}

/// The corners' color: 40 × 30 pixels of light gray around a red block, cleared within 5 steps on
/// each channel (`match: channels`, a tolerance of `5 / 255 × 100`, no softness), from the edges
/// or wherever it is.
#[test]
fn the_corners_color_is_cleared() {
    let img = image::RgbaImage::from_fn(40, 30, |x, y| {
        image::Rgba(if (10..30).contains(&x) && (8..22).contains(&y) { [200, 40, 40, 255] } else { [240, 240, 240, 255] })
    });
    for contiguous in [false, true] {
        let (mut s, id) = open(&png(DynamicImage::ImageRgba8(img.clone())));
        let p = json!({"background": "corners", "contiguous": contiguous, "match": "channels", "tolerance": 5.0 / 255.0 * 100.0, "softness": 0});
        let r = s.execute(C, &p).unwrap();
        assert_eq!(r["cleared"], 40 * 30 - 14 * 20, "{p}");
        let out = pixels(&s, id).to_rgba8();
        assert_eq!((out.get_pixel(0, 0).0[3], out.get_pixel(15, 15).0), (0, [200, 40, 40, 255]), "{p}");
    }
    // A clear corner is left out of the average: the other three are white, and white is cleared.
    let mut img = image::RgbaImage::from_pixel(8, 8, image::Rgba(WHITE));
    img.put_pixel(0, 0, image::Rgba([0; 4]));
    img.put_pixel(4, 4, image::Rgba(RED));
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(img)));
    let r = s.execute(C, &json!({"background": "corners", "tolerance": 0, "softness": 0})).unwrap();
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 7, 7), alpha(&out, 3, 3), alpha(&out, 4, 4)), (0, 0, 255));
    assert_eq!(r["cleared"], 8 * 8 - 2, "the pixels made clear, not the corner that was clear already");
}

/// One picked color matched by channel at a tolerance of 0: cleared wherever it is, and a pixel a
/// long way from it kept.
#[test]
fn one_picked_color_is_cleared_by_channel() {
    let mut img = image::RgbaImage::from_pixel(4, 4, image::Rgba([10, 20, 30, 255]));
    img.put_pixel(2, 2, image::Rgba([250, 250, 250, 255]));
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(img)));
    let p = json!({"background": "colors", "colors": "#0a141e", "contiguous": false, "match": "channels", "tolerance": 0, "softness": 0});
    assert_eq!(s.execute(C, &p).unwrap()["cleared"], 15);
    let out = pixels(&s, id);
    assert_eq!((alpha(&out, 0, 0), alpha(&out, 2, 2)), (0, 255));
}

/// With `match: channels`, a tolerance of `t / 255 × 100` clears a color `k` 8-bit steps from the
/// background on one channel exactly when `k ≤ t`, for every `t` and `k` from 0 to 255; with
/// `distance` the same color is nearer when the others match.
#[test]
fn channels_match_the_largest_difference_on_one_channel() {
    for t in 0..=255u16 {
        let s = Settings { measure: Match::Channels, tolerance: f64::from(t) / 255.0 * 100.0, softness: 0.0, ..settings(Background::Edges) };
        for k in 0..=255u16 {
            for c in [[k * 257, 0, 0], [0, k * 257, 0], [0, 0, k * 257]] {
                assert_eq!(level(c, &[[0; 3]], &s) == 0, k <= t, "t {t}, k {k}");
            }
        }
    }
    let at = |measure| {
        level([0, 40 * 257, 0], &[[0; 3]], &Settings { measure, tolerance: 32.0 / 255.0 * 100.0, softness: 0.0, ..settings(Background::Edges) })
    };
    assert_eq!((at(Match::Channels), at(Match::Distance)), (255, 0), "40 steps on green: past 32 on one channel; 9 % of the RGB distance");
}

#[test]
fn sixteen_bit_and_gray_images_keep_their_depth_and_channels() {
    let deep = ImageBuffer::from_fn(8, 8, |x, y| Rgb(if (2..6).contains(&x) && (2..6).contains(&y) { [51_400, 0, 1] } else { [65_535; 3] }));
    let (mut s, id) = open(&png(DynamicImage::ImageRgb16(deep)));
    s.execute(C, &json!({})).unwrap();
    let DynamicImage::ImageRgba16(out) = pixels(&s, id) else { panic!("16-bit RGBA") };
    assert_eq!(out.get_pixel(0, 0).0[3], 0);
    assert_eq!(out.get_pixel(3, 3).0, [51_400, 0, 1, 65_535], "16-bit samples kept as they were");
    // Samples one 8-bit step apart are told apart: only the background is cleared at 0.
    let near = ImageBuffer::from_fn(4, 4, |x, y| Rgb(if (1..3).contains(&x) && (1..3).contains(&y) { [65_280; 3] } else { [65_535; 3] }));
    let (mut s, id) = open(&png(DynamicImage::ImageRgb16(near)));
    s.execute(C, &json!({"tolerance": 0, "softness": 0})).unwrap();
    let DynamicImage::ImageRgba16(out) = pixels(&s, id) else { panic!("16-bit RGBA") };
    assert_eq!((out.get_pixel(0, 0).0[3], out.get_pixel(1, 1).0[3]), (0, 65_535));
    let gray = ImageBuffer::from_fn(8, 8, |x, y| Luma([if (2..6).contains(&x) && (2..6).contains(&y) { 30u8 } else { 240 }]));
    let (mut s, id) = open(&png(DynamicImage::ImageLuma8(gray)));
    s.execute(C, &json!({})).unwrap();
    let DynamicImage::ImageLumaA8(out) = pixels(&s, id) else { panic!("gray with alpha") };
    assert_eq!((out.get_pixel(0, 0).0, out.get_pixel(3, 3).0), ([240, 0], [30, 255]));
    let gray16 = ImageBuffer::from_fn(8, 8, |x, y| Luma([if (2..6).contains(&x) && (2..6).contains(&y) { 7_000u16 } else { 61_000 }]));
    let (mut s, id) = open(&png(DynamicImage::ImageLuma16(gray16)));
    s.execute(C, &json!({})).unwrap();
    let DynamicImage::ImageLumaA16(out) = pixels(&s, id) else { panic!("16-bit gray with alpha") };
    assert_eq!((out.get_pixel(0, 0).0, out.get_pixel(3, 3).0), ([61_000, 0], [7_000, 65_535]));
}

/// The ICC profile the colors are in stays with them.
#[test]
fn the_color_profile_is_kept() {
    use image::{ImageDecoder as _, ImageEncoder as _};
    let icc = b"a profile's bytes".to_vec();
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder.set_icc_profile(icc.clone()).unwrap();
    DynamicImage::ImageRgba8(framed()).write_with_encoder(encoder).unwrap();
    let blob = ImageBlob::png(bytes);
    let out = super::clear(&blob, &settings(Background::Edges), None).unwrap().unwrap();
    let mut decoder = image::codecs::png::PngDecoder::new(Cursor::new(out.blob.bytes.as_slice())).unwrap();
    assert_eq!(decoder.icc_profile().unwrap(), Some(icc));
}

/// The resolution the file declares stays with the pixels, so a later Replace or relink sizes the
/// image as before.
#[test]
fn the_resolution_is_kept() {
    let bytes = ppi::with_png_resolution(&png(DynamicImage::ImageRgba8(framed())), (300.0, 150.0));
    let (mut s, id) = open(&bytes);
    s.execute(C, &json!({})).unwrap();
    let (x, y) = ppi::resolution(&bytes_of(&s, id)).expect("a resolution");
    assert_eq!((x.round(), y.round()), (300.0, 150.0));
}

#[test]
fn a_cmyk_image_is_refused() {
    let inks = vectorcraft_doc::cmyk::Inks::new(4, 2, [0, 0, 0, 0].repeat(8)).unwrap();
    let tiff = ImageBlob::cmyk_tiff(&inks).unwrap();
    let (mut s, id) = open(&tiff.bytes);
    let before = (image_of(&s, id), undo_len(&s));
    let e = s.execute(C, &json!({})).unwrap_err();
    assert!(e.to_string().contains("a CMYK image has no transparency to set"), "{e}");
    assert_eq!((image_of(&s, id), undo_len(&s)), before);
}

/// A linked image of `blob`, `width` × `height` pixels as the object records it, beside the open
/// document's image.
fn place_linked(s: &mut Session, key: &str, blob: ImageBlob, (width, height): (u32, u32)) -> NodeId {
    s.edit("Place", |d, sel| {
        d.images.insert(key.into(), blob);
        let id = d.alloc_id();
        let im = ImageObject {
            key: key.into(),
            width,
            height,
            xf: Default::default(),
            link: Some(LinkInfo::new("/nowhere/art.png")),
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn a_linked_image_is_embedded_and_one_that_shows_its_preview_is_refused() {
    let bytes = png(DynamicImage::ImageRgba8(framed()));
    let (mut s, _) = open(&bytes);
    let linked = place_linked(&mut s, "linked", ImageBlob::png(bytes.clone()), (12, 10));
    let clip = vectorcraft_doc::links::PlacementOptions { clip: true, ..Default::default() };
    s.edit("Placement", |d, _| {
        if let Some(NodeKind::Image(im)) = d.node_mut(linked).map(|n| &mut n.kind) {
            im.placement = clip;
        }
        Ok(())
    })
    .unwrap();
    let r = s.execute(C, &json!({"ids": [linked.0, linked.0]})).unwrap();
    assert_eq!(r, json!({"changed": 1, "ids": [linked.0], "cleared": 12 * 10 - 6 * 4}), "an image named twice changes once");
    let im = image_of(&s, linked);
    assert_eq!((im.link, alpha(&pixels(&s, linked), 0, 0)), (None, 0), "embedded, with its background cleared");
    assert_eq!(im.placement, clip, "its Placement Options kept, as Embed keeps them");
    // Pixels the file wasn't read for: the preview, as an image whose file is missing shows it.
    let preview = Arc::new(bytes);
    let missing = place_linked(
        &mut s,
        "missing",
        ImageBlob { mime: "image/png".into(), bytes: preview.clone(), proxy: Some(preview), file_font: Default::default() },
        (12, 10),
    );
    let before = (image_of(&s, missing), undo_len(&s));
    let e = s.execute(C, &json!({})).unwrap_err();
    assert!(e.to_string().contains("the linked image shows its preview: embed or relink it first"), "{e}");
    assert_eq!((image_of(&s, missing), undo_len(&s)), before);
}

/// An object whose pixels were read at another size than it records keeps its place and size.
#[test]
fn an_image_read_at_another_size_keeps_its_place_and_size() {
    let bytes = png(DynamicImage::ImageRgba8(framed()));
    let (mut s, _) = open(&bytes);
    let id = place_linked(&mut s, "linked", ImageBlob::png(bytes), (24, 20));
    s.execute(C, &json!({"ids": [id.0]})).unwrap();
    let im = image_of(&s, id);
    assert_eq!((im.width, im.height, im.xf), (12, 10, Affine::scale_non_uniform(2.0, 2.0)));
}

#[test]
fn bad_params_are_refused_and_change_nothing() {
    let (mut s, id) = open(&png(DynamicImage::ImageRgba8(framed())));
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = (image_of(&s, id), undo_len(&s));
    // With `ids`, the command is available when one of them is an image.
    let not_image = format!("object {rect} is not an image");
    let cases: [(Value, &str); 18] = [
        (json!({"background": "sky"}), "`background` must be edges, corners or colors"),
        (json!({"background": 5}), "`background` must be a string"),
        (json!({"match": "rgb"}), "`match` must be distance or channels"),
        (json!({"match": true}), "`match` must be a string"),
        (json!({"contiguous": "yes"}), "`contiguous` must be true or false"),
        (json!({"contiguous": 1}), "`contiguous` must be true or false"),
        (json!({"tolerance": 101}), "`tolerance` must be from 0 to 100"),
        (json!({"softness": -1}), "`softness` must be from 0 to 100"),
        (json!({"tolerance": "5"}), "`tolerance` must be a number"),
        (json!({"background": "colors"}), "`colors` must list 1 to 16 colors, each hex (#rrggbb or #rgb) or black, white, red, green or blue"),
        (json!({"background": "colors", "colors": []}), "`colors` must list"),
        (json!({"background": "colors", "colors": "#12"}), "`colors` must list"),
        (json!({"background": "colors", "colors": "#ffffff80"}), "`colors` must list"),
        (json!({"colors": ["#ffffff", 7]}), "`colors` must list"),
        (json!({"background": "colors", "colors": vec!["#000"; 17]}), "`colors` must list"),
        (json!({"background": "colors", "colors": "#000 ".repeat(100_000)}), "`colors` must list"),
        (json!({"ids": [id.0, rect]}), &not_image),
        (json!({"ids": [rect]}), "select an image"),
    ];
    for (p, msg) in cases {
        let e = s.execute(C, &p).unwrap_err().to_string();
        assert!(e.contains(msg), "{p}: {e}");
    }
    assert_eq!((image_of(&s, id), undo_len(&s)), before);
}

#[test]
fn an_image_too_large_to_change_is_refused_before_it_is_decoded() {
    let run = |w: u32, h: u32| {
        let (mut s, _) = open(&crate::tests_linkspanel::png_header(w, h));
        s.execute(C, &json!({})).unwrap_err().to_string()
    };
    // As gray samples, RGBA worked on and kept for the PNG, and the work: 19 bytes a pixel.
    let over = run(10_632, 10_631);
    assert!(over.contains("an image of 10632 × 10631 pixels is too large to change"), "{over}");
    // Within the allowance it is decoded, and its missing rows can't be read.
    let within = run(10_631, 10_631);
    assert!(within.contains("the image's pixels can't be read"), "{within}");
}
