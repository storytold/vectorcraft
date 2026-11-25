//! File → Print › Advanced: simulated overprints print what Overprint Preview shows, Print as
//! Bitmap prints images of the art, and a flattener preset flattens transparency first.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_pdf::PrintSettings;
use vectorcraft_render::{RenderOptions, Renderer};

use crate::*;

/// A 200 × 100 CMYK document: a cyan square under a yellow one whose fill overprints, and a
/// half-transparent magenta one; their ids.
fn session() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100, "colorMode": "cmyk"})).unwrap();
    let mut ids = [0; 3];
    for (i, (x, color)) in [(10, "#00ffff"), (40, "#ffff00"), (120, "#ff00ff")].into_iter().enumerate() {
        let r = s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap();
        ids[i] = r["id"].as_u64().unwrap();
        s.execute("paint.setFill", &json!({"ids": [ids[i]], "color": color})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [ids[i]], "none": true})).unwrap();
    }
    s.execute("object.setOverprint", &json!({"ids": [ids[1]], "fill": true})).unwrap();
    s.execute("transparency.set", &json!({"ids": [ids[2]], "opacity": 50, "item": null})).unwrap();
    (s, ids)
}

fn settings(v: Value) -> PrintSettings {
    serde_json::from_value(v).unwrap()
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

/// `doc` rendered over its artboard at 2 px/pt on white.
fn pixels(doc: &Document, opts: RenderOptions) -> Vec<u8> {
    let opts = RenderOptions { background: Some([255; 4]), skip_templates: true, ..opts };
    Renderer::new().render_region_with(doc, Rect::new(0.0, 0.0, 200.0, 100.0), 2.0, &opts).pixels
}

/// The pixel of the page `pdf` prints at document point `(x, y)` of the first sheet.
fn printed_pixel(s: &mut Session, settings: Value, (x, y): (f64, f64)) -> [u8; 4] {
    let v = s.execute("file.print", &json!({ "settings": settings })).unwrap();
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let pv = s.execute("print.preview", &json!({ "settings": settings })).unwrap();
    let t: Vec<f64> = pv["sheets"][0]["transform"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    let page = back.artboards[0].rect;
    let (px, py) = (page.x0 + t[0] * x + t[2] * y + t[4], page.y0 + t[1] * x + t[3] * y + t[5]);
    let img = Renderer::new().render_region(&back, Rect::new(px - 0.5, py - 0.5, px + 0.5, py + 0.5), 1.0, true);
    img.pixels[..4].try_into().unwrap()
}

fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 4)
}

#[test]
fn simulated_overprints_print_what_overprint_preview_shows() {
    let (mut s, _) = session();
    let preview = pixels(doc(&s), RenderOptions { overprint_preview: true, ..Default::default() });
    let simulated = vectorcraft_pdf::printed_document(doc(&s), &settings(json!({"advanced": {"overprints": "simulate"}})));
    assert_eq!(pixels(&simulated, RenderOptions::default()), preview, "the same pixels as Overprint Preview");
    assert_ne!(pixels(doc(&s), RenderOptions::default()), preview, "overprinting shows");
    // On the printed page too: where yellow overprints cyan it prints green; discarded, yellow.
    let at = (55.0, 30.0);
    let i = ((30.0 * 2.0) as usize * 400 + (55.0 * 2.0) as usize) * 4;
    let shown: [u8; 4] = preview[i..i + 4].try_into().unwrap();
    let sim = printed_pixel(&mut s, json!({"advanced": {"overprints": "simulate"}}), at);
    assert!(near(sim, shown), "{sim:?} vs {shown:?}");
    let discarded = printed_pixel(&mut s, json!({"advanced": {"overprints": "discard"}}), at);
    assert!(!near(discarded, shown) && discarded[2] < 64 && discarded[0] > 192, "yellow knocks out: {discarded:?}");
}

#[test]
fn a_flattener_preset_flattens_transparency_before_printing() {
    let (mut s, _) = session();
    let saved = s.prefs.flattener_presets.clone();
    let transparent = |d: &Document| {
        let mut any = false;
        d.walk(|n| any |= n.opacity < 1.0);
        any
    };
    assert!(transparent(doc(&s)));
    let none = crate::cmd::printadvanced::prepare("file.print", doc(&s), &settings(json!({})), &saved, false).unwrap();
    assert!(none.is_none(), "transparency prints live without a preset");
    for name in ["High Resolution", "low"] {
        let set = settings(json!({"advanced": {"flattenerPreset": name}}));
        let flat = crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, false).unwrap().unwrap();
        assert!(!transparent(&flat), "{name}: flattened");
    }
    // A saved preset by name; an unknown one is refused by preview and print alike.
    s.execute("flattener.presets.save", &json!({"name": "Proofs", "preset": "low"})).unwrap();
    let ok = json!({"settings": {"advanced": {"flattenerPreset": "Proofs"}}});
    assert!(s.execute("print.preview", &ok).is_ok() && s.execute("file.print", &ok).is_ok());
    let bad = json!({"settings": {"advanced": {"flattenerPreset": "Nope"}}});
    assert!(s.execute("print.preview", &bad).is_err() && s.execute("file.print", &bad).is_err());
}

#[test]
fn print_as_bitmap_prints_an_image_of_each_artboard() {
    let (mut s, _) = session();
    s.execute("artboard.new", &json!({"x": 300, "y": 0, "width": 200, "height": 100})).unwrap();
    let saved = s.prefs.flattener_presets.clone();
    let set = settings(json!({"advanced": {"printAsBitmap": true}}));
    let bitmap = crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, false).unwrap().unwrap();
    let mut kinds = vec![];
    bitmap.walk(|n| kinds.push(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(kinds.iter().filter(|k| **k).count(), 1, "one image: the empty artboard gets none");
    // The image looks like the art, at the raster effects resolution.
    assert_eq!(pixels(&bitmap, RenderOptions::default()).len(), pixels(doc(&s), RenderOptions::default()).len());
    let v = s.execute("file.print", &json!({"settings": {"advanced": {"printAsBitmap": true}, "skipBlank": true}})).unwrap();
    assert_eq!(v["pages"], 1, "the blank artboard is skipped");
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    let (mut images, mut paths) = (0, 0);
    back.walk(|n| match n.kind {
        NodeKind::Image(_) => images += 1,
        NodeKind::Path { .. } => paths += 1,
        _ => {}
    });
    assert_eq!((images, paths), (1, 0), "no vector art is left");
    // Separations print the art as it is.
    let seps = json!({"settings": {"advanced": {"printAsBitmap": true}, "output": {"mode": "separations"}}});
    let pv = s.execute("print.preview", &seps).unwrap();
    assert!(pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("Print as Bitmap")));
    let set = settings(seps["settings"].clone());
    assert!(crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, false).unwrap().is_none());
}

/// Print as Bitmap of an artboard longer than the renderer draws at the raster effects resolution
/// (32,768 pixels a side) prints an image of all of it, at a lower resolution.
#[test]
fn print_as_bitmap_prints_all_of_a_long_artboard() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 9000, "height": 100})).unwrap();
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 9000, "height": 100})).unwrap();
    let set = settings(json!({"advanced": {"printAsBitmap": true}}));
    let saved = s.prefs.flattener_presets.clone();
    let bitmap = crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, false).unwrap().unwrap();
    let mut placed = vec![];
    bitmap.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            placed.push((im.width, im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height)))));
        }
    });
    let [(width, b)] = placed[..] else { panic!("one image: {placed:?}") };
    assert!(width <= vectorcraft_render::MAX_RASTER_SIDE, "{width} pixels");
    assert!(b.x0 <= 0.0 && b.x1 >= 9000.0, "the image covers the artboard: {b:?}");
}

/// The images `file.print` prints with `settings`: their pixel sizes and their places on the page
/// in the print-ready PDF (read back), their pixel sizes in the PostScript file (its image
/// dictionaries, each size once), and the warnings of both.
struct Printed {
    pdf: Vec<(u32, u32)>,
    placed: Vec<Rect>,
    ps: Vec<(u32, u32)>,
    warnings: Vec<String>,
}

fn printed_images(s: &mut Session, settings: &Value) -> Printed {
    let warnings = |v: &Value| -> Vec<String> { v["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).map(str::to_string).collect() };
    let v = s.execute("file.print", &json!({ "settings": settings })).unwrap();
    let back = vectorcraft_pdf::import(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    let (mut pdf, mut placed) = (vec![], vec![]);
    back.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            pdf.push((im.width, im.height));
            placed.push(im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height))));
        }
    });
    let mut all = warnings(&v);
    let v = s.execute("file.print", &json!({ "settings": settings, "format": "postscript" })).unwrap();
    let ps = String::from_utf8_lossy(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).into_owned();
    let mut sizes = vec![];
    for part in ps.split("<< /ImageType ").skip(1) {
        let after = |key: &str| part.split_once(key).and_then(|(_, r)| r.split_whitespace().next()).and_then(|n| n.parse::<u32>().ok());
        if let (Some(w), Some(h)) = (after("/Width "), after("/Height ")) {
            sizes.push((w, h));
        }
    }
    sizes.sort_unstable();
    sizes.dedup();
    all.extend(warnings(&v));
    Printed { pdf, placed, ps: sizes, warnings: all }
}

const CAPPED: &str = "Print as Bitmap prints at";

/// `advanced.bitmapResolution` sets how many pixels Print as Bitmap prints the art with, in a
/// print-ready PDF and in PostScript, on the same place of the page; 0 or none keeps the
/// document's raster effects resolution.
#[test]
fn bitmap_resolution_sets_the_printed_image_size() {
    let (mut s, _) = session();
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 72})).unwrap();
    let mut places = vec![];
    for (ppi, size) in [(Value::Null, (200, 100)), (json!(0), (200, 100)), (json!(144), (400, 200)), (json!(36), (100, 50)), (json!(1), (3, 2))] {
        let mut advanced = json!({"printAsBitmap": true});
        if !ppi.is_null() {
            advanced["bitmapResolution"] = ppi.clone();
        }
        let p = printed_images(&mut s, &json!({ "advanced": advanced }));
        assert_eq!((p.pdf.as_slice(), p.ps.as_slice()), ([size].as_slice(), [size].as_slice()), "{ppi}");
        assert!(!p.warnings.iter().any(|w| w.starts_with(CAPPED)), "{ppi}: {:?}", p.warnings);
        // At 1 ppi the image is rounded up to whole pixels past the artboard.
        if ppi != json!(1) {
            places.extend(p.placed);
        }
    }
    let first = places[0];
    for r in &places {
        let apart = (r.x0 - first.x0).abs() + (r.y0 - first.y0).abs() + (r.width() - first.width()).abs() + (r.height() - first.height()).abs();
        assert!(apart < 1.0, "the image covers the same part of the page: {r:?} vs {first:?}");
    }
    // Art that does not print as a bitmap leaves it unused.
    let p = printed_images(&mut s, &json!({"advanced": {"bitmapResolution": 144}}));
    assert!(p.pdf.is_empty(), "{:?}", p.pdf);
}

/// A `bitmapResolution` given to `file.print` applies to that job alone; `print.setup` saves it.
#[test]
fn a_bitmap_resolution_given_to_file_print_applies_to_that_job() {
    let (mut s, _) = session();
    s.execute("document.rasterEffectsSettings", &json!({"resolution": 72})).unwrap();
    let undo = s.active().unwrap().history.undo.len();
    let p = printed_images(&mut s, &json!({"advanced": {"printAsBitmap": true, "bitmapResolution": 144}}));
    assert_eq!(p.pdf, [(400, 200)]);
    assert!(doc(&s).print_setup.is_none(), "the saved settings stay as they are");
    assert_eq!(doc(&s).raster_effects_ppi, 72.0, "and so does the raster effects resolution");
    assert_eq!(s.active().unwrap().history.undo.len(), undo, "printing adds no undo step");
    let v = s.execute("print.setup", &json!({"settings": {"advanced": {"printAsBitmap": true, "bitmapResolution": 144}}})).unwrap();
    assert_eq!(v["settings"]["advanced"]["bitmapResolution"], 144.0);
    assert_eq!(printed_images(&mut s, &json!({})).pdf, [(400, 200)], "a job without settings prints at the saved one");
    assert_eq!(printed_images(&mut s, &json!({"advanced": {"bitmapResolution": 0}})).pdf, [(200, 100)], "0 over it: the document's");
    // A saved 0 is left out of the settings, which read it back as 0.
    let v = s.execute("print.setup", &json!({"settings": {"advanced": {"bitmapResolution": 0}}})).unwrap();
    assert!(v["settings"]["advanced"].get("bitmapResolution").is_none(), "{}", v["settings"]["advanced"]);
}

/// A `bitmapResolution` outside 0 and 1–9600 ppi, or that is not a number, is refused by
/// `print.setup`, `print.preview` and `file.print` alike, and nothing is saved.
#[test]
fn a_bitmap_resolution_out_of_range_is_refused() {
    let (mut s, _) = session();
    let undo = s.active().unwrap().history.undo.len();
    for bad in [json!(0.5), json!(-1), json!(-0.001), json!(9600.5), json!(1e300), json!("high"), json!(true), json!([300])] {
        let p = json!({"settings": {"advanced": {"printAsBitmap": true, "bitmapResolution": bad}}});
        for cmd in ["print.setup", "print.preview", "file.print"] {
            let e = s.execute(cmd, &p).map(|v| panic!("{cmd} {bad}: {v}")).unwrap_err().to_string();
            assert!(!bad.is_number() || e.contains("advanced.bitmapResolution must be 1–9600 ppi"), "{cmd} {bad}: {e}");
        }
    }
    assert!(doc(&s).print_setup.is_none());
    assert_eq!(s.active().unwrap().history.undo.len(), undo);
    for ok in [1.0, 9600.0] {
        s.execute("print.preview", &json!({"settings": {"advanced": {"printAsBitmap": true, "bitmapResolution": ok}}})).unwrap();
    }
}

/// A page the pixel limits print below the `bitmapResolution` asked for gets a warning that names
/// the resolution it prints at, in `file.print`'s PDF and PostScript replies and in
/// `print.preview`.
#[test]
fn a_bitmap_resolution_the_pixel_limits_lower_is_named() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 9000, "height": 2})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 9000, "height": 2})).unwrap();
    let set = json!({"advanced": {"printAsBitmap": true, "bitmapResolution": 600}});
    let p = printed_images(&mut s, &set);
    // 32,766 pixels across 9,000 pt: 262.1 ppi.
    let note = "Print as Bitmap prints at 262 ppi, not the 600 ppi bitmapResolution asks for";
    assert_eq!(p.warnings.iter().filter(|w| w.starts_with(note)).count(), 2, "{:?}", p.warnings);
    let [(w, _)] = p.pdf[..] else { panic!("one image: {:?}", p.pdf) };
    assert!((32_000..=vectorcraft_render::MAX_RASTER_SIDE).contains(&w), "{w} pixels");
    assert_eq!(p.ps, p.pdf);
    let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
    assert!(pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with(note)), "{}", pv["warnings"]);
    // A resolution that fits, the document's resolution, and output that is not a bitmap: no note.
    for set in [
        json!({"advanced": {"printAsBitmap": true, "bitmapResolution": 200}}),
        json!({"advanced": {"printAsBitmap": true}}),
        json!({"advanced": {"bitmapResolution": 600}}),
        json!({"advanced": {"printAsBitmap": true, "bitmapResolution": 600}, "output": {"mode": "separations"}}),
    ] {
        let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
        assert!(!pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with(CAPPED)), "{set}: {}", pv["warnings"]);
    }
}

/// An artboard with no art prints no image, so the pixel limits a bitmap of it would meet name no
/// lower resolution, also with only art that draws nothing on it in `file.print`; once a background
/// color or art that draws is on it, they do.
#[test]
fn an_empty_artboard_does_not_name_a_lower_bitmap_resolution() {
    let (mut s, _) = session();
    s.execute("artboard.new", &json!({"x": 0, "y": 500, "width": 9000, "height": 2})).unwrap();
    let set = json!({"advanced": {"printAsBitmap": true, "bitmapResolution": 600}});
    let p = printed_images(&mut s, &set);
    assert_eq!(p.pdf.len(), 1, "only the artboard with art prints an image: {:?}", p.pdf);
    assert!(!p.warnings.iter().any(|w| w.starts_with(CAPPED)), "{:?}", p.warnings);
    let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
    assert!(!pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with(CAPPED)), "{}", pv["warnings"]);
    // A background color prints, in file.print and in what print.preview says alike.
    s.execute("artboard.setProps", &json!({"index": 1, "color": "#336699"})).unwrap();
    let p = printed_images(&mut s, &set);
    assert_eq!(p.pdf.len(), 2, "{:?}", p.pdf);
    assert_eq!(p.warnings.iter().filter(|w| w.starts_with(CAPPED)).count(), 2, "{:?}", p.warnings);
    let pv = s.execute("print.preview", &json!({ "settings": set })).unwrap();
    assert!(pv["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().starts_with(CAPPED)), "{}", pv["warnings"]);
    s.execute("artboard.setProps", &json!({"index": 1, "color": null})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 0, "y": 500, "width": 9000, "height": 2})).unwrap()["id"].clone();
    // Art that draws nothing prints no image either.
    s.execute("object.setProps", &json!({"ids": [id], "opacity": 0})).unwrap();
    let p = printed_images(&mut s, &set);
    assert_eq!(p.pdf.len(), 1, "{:?}", p.pdf);
    assert!(!p.warnings.iter().any(|w| w.starts_with(CAPPED)), "{:?}", p.warnings);
    s.execute("object.setProps", &json!({"ids": [id], "opacity": 100})).unwrap();
    let p = printed_images(&mut s, &set);
    assert_eq!(p.pdf.len(), 2, "{:?}", p.pdf);
    assert_eq!(p.warnings.iter().filter(|w| w.starts_with(CAPPED)).count(), 2, "{:?}", p.warnings);
}

/// Appearance of Black › Printing / Exporting (#394): Print as Bitmap draws pure CMYK black as RGB
/// black when `rich_black` is on.
#[test]
fn print_as_bitmap_honours_rich_black() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 20, "height": 20, "colorMode": "cmyk"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    let set = settings(json!({"advanced": {"printAsBitmap": true}}));
    let saved = s.prefs.flattener_presets.clone();
    let centre = |prepared: &Document| {
        let mut key = None;
        prepared.walk(|n| {
            if let NodeKind::Image(im) = &n.kind {
                key = Some(im.key.clone());
            }
        });
        let bytes = &prepared.images[key.as_ref().expect("bitmap image")].bytes;
        let img = craft_image_codecs::decode(bytes)
            .map(|__cimg| {
                let (__w, __h) = __cimg.dimensions();
                match (__cimg.layout(), __cimg.sample_type()) {
                    (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                        image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                    }
                    _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
                }
            })
            .unwrap()
            .to_rgba8();
        img.get_pixel(img.width() / 2, img.height() / 2).0
    };
    let accurate = centre(&crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, false).unwrap().unwrap());
    assert!(accurate[0] > 10, "accurate: dark grey, {accurate:?}");
    let rich = centre(&crate::cmd::printadvanced::prepare("file.print", doc(&s), &set, &saved, true).unwrap().unwrap());
    assert_eq!(rich, [0, 0, 0, 255], "rich: RGB black");
}
