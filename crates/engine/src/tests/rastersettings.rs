//! Effect → Document Raster Effects Settings (`document.rasterEffectsSettings`): the settings, the
//! raster effect images they make (PDF export, Expand Appearance), the Rasterize defaults, and
//! Effect › Rasterize's image.

use serde_json::{Value, json};
use vectorcraft_doc::rastersettings::MAX_ADD_AROUND;
use vectorcraft_doc::{Background, Document, NodeId, NodeKind, RasterColorModel, RasterEffectsSettings};

use crate::*;

const C: &str = "document.rasterEffectsSettings";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A red rectangle with a drop shadow.
fn shadowed(s: &mut Session) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [r.0]})).unwrap();
    r
}

/// The decoded pixels of the first raster effect image of `doc` flattened for output.
fn effect_image(doc: &Document) -> image::RgbaImage {
    let flat = crate::flatten_raster_effects(doc).unwrap();
    let kids = flat.layers[0].children().unwrap();
    let NodeKind::Group { children, .. } = &kids[0].kind else { panic!("a shadow becomes a group") };
    let NodeKind::Image(im) = &children[0].kind else { panic!("the image goes under the object") };
    image::load_from_memory(&flat.images[&im.key].bytes).unwrap().to_rgba8()
}

#[test]
fn settings_report_change_in_one_step_and_reject_junk() {
    let mut s = session();
    let r = s.execute(C, &json!({})).unwrap();
    assert_eq!(
        r,
        json!({"resolution": 72.0, "colorModel": "rgb", "background": "transparent", "antiAlias": true, "clippingMask": false, "addAround": 0.0, "preserveSpotColors": true})
    );
    let r = s
        .execute(C, &json!({"resolution": "high", "colorModel": "grayscale", "background": "white", "antiAlias": false, "clippingMask": true, "addAround": 36}))
        .unwrap();
    assert_eq!((r["resolution"].as_f64(), r["colorModel"].as_str(), r["addAround"].as_f64()), (Some(300.0), Some("grayscale"), Some(36.0)));
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    // The report fed back is no change.
    s.execute(C, &r).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), 1);
    for bad in [
        json!({"colorModel": "cmyk"}),
        json!({"colorModel": "lab"}),
        json!({"background": "pink"}),
        json!({"antiAlias": "yes"}),
        json!({"addAround": -1}),
        json!({"addAround": 5000}),
        json!({"resolution": 0}),
        json!({"dpi": 300}),
    ] {
        assert!(s.execute(C, &bad).is_err(), "{bad}");
    }
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.raster_effects_ppi, &d.raster_effects), (72.0, &RasterEffectsSettings::default()));
    // A CMYK document names its own mode.
    let mut s = Session::new();
    s.execute("file.new", &json!({"colorMode": "cmyk"})).unwrap();
    assert_eq!(s.execute(C, &json!({"colorModel": "cmyk"})).unwrap()["colorModel"], "cmyk");
}

#[test]
fn a_white_background_makes_effect_images_opaque() {
    let mut s = session();
    shadowed(&mut s);
    let clear = effect_image(&s.doc().unwrap().doc);
    assert_eq!(clear.get_pixel(0, 0)[3], 0, "transparent by default");
    s.execute(C, &json!({"background": "white"})).unwrap();
    let white = effect_image(&s.doc().unwrap().doc);
    assert!(white.pixels().all(|p| p[3] == 255), "every pixel is opaque");
    assert_eq!(white.get_pixel(0, 0).0, [255, 255, 255, 255]);
    // With a clipping mask, the white stays under the art only.
    s.execute(C, &json!({"clippingMask": true})).unwrap();
    assert_eq!(effect_image(&s.doc().unwrap().doc).get_pixel(0, 0)[3], 0);
}

#[test]
fn add_around_grows_the_effect_image() {
    let mut s = session();
    shadowed(&mut s);
    let before = effect_image(&s.doc().unwrap().doc);
    s.execute(C, &json!({"addAround": 36})).unwrap();
    let after = effect_image(&s.doc().unwrap().doc);
    // 36 pt on every side at 72 ppi.
    assert_eq!((after.width() - before.width(), after.height() - before.height()), (72, 72));
}

/// The transforms of the images in `doc` flattened for output.
fn image_transforms(doc: &Document) -> Vec<[f64; 6]> {
    let mut xfs = vec![];
    crate::flatten_raster_effects(doc).unwrap().walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            xfs.push(im.xf.as_coeffs());
        }
    });
    xfs
}

#[test]
fn a_file_with_add_around_out_of_range_opens_with_it_in_range() {
    let mut s = session();
    shadowed(&mut s);
    let mut d = (*s.doc().unwrap().doc).clone();
    d.raster_effects.add_around = 1e300;
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(&d)).unwrap();
    assert_eq!(back.raster_effects.add_around, MAX_ADD_AROUND);
    let xfs = image_transforms(&back);
    assert!(!xfs.is_empty() && xfs.iter().flatten().all(|c| c.is_finite()), "{xfs:?}");
    // 1,000 pt on every side at 72 ppi.
    assert_eq!(effect_image(&back).width() - effect_image(&s.doc().unwrap().doc).width(), 2000);
    d.raster_effects.add_around = -5.0;
    assert_eq!(vectorcraft_format::load(&vectorcraft_format::save_file(&d)).unwrap().raster_effects.add_around, 0.0);
}

#[test]
fn art_too_large_for_f64_gets_no_effect_image() {
    let mut s = session();
    let r = shadowed(&mut s);
    // Settings out of range in a document made without opening a file.
    let mut d = (*s.doc().unwrap().doc).clone();
    d.raster_effects.add_around = 1e300;
    assert_eq!(image_transforms(&d), Vec::<[f64; 6]>::new());
    let mut d = (*s.doc().unwrap().doc).clone();
    let NodeKind::Path { path, live, .. } = &mut d.node_mut(r).unwrap().kind else { panic!("a path") };
    (*path, *live) = (vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(-1e300, -1e300, 1e300, 1e300)), None);
    assert_eq!(image_transforms(&d), Vec::<[f64; 6]>::new());
    // Expand Appearance of it by id, with nothing selected, puts no image into the document (the
    // rectangle at its own size gets one).
    let normal = (*s.doc().unwrap().doc).clone();
    let mut expanded_images = |d: Document| {
        let st = s.doc_mut().unwrap();
        (st.doc, st.selection) = (std::sync::Arc::new(d), Default::default());
        s.execute("effect.expandAppearance", &json!({"ids": [r.0]})).unwrap();
        let mut images = 0;
        s.doc().unwrap().doc.walk(|n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
        images
    };
    assert_eq!(expanded_images(normal), 1);
    assert_eq!(expanded_images(d), 0);
}

#[test]
fn the_color_model_and_anti_alias_reach_the_pixels() {
    let mut s = session();
    shadowed(&mut s);
    s.execute(C, &json!({"colorModel": "bitmap", "antiAlias": false})).unwrap();
    let img = effect_image(&s.doc().unwrap().doc);
    assert!(img.pixels().filter(|p| p[3] > 0).all(|p| p[0] == p[1] && p[1] == p[2] && (p[0] == 0 || p[0] == 255)), "black and white");
    // Anti-alias off: Rasterize leaves no partly covered pixel on a circle's edge.
    let e = id(&s.execute("shape.ellipse", &json!({"x": 180, "y": 180, "width": 50, "height": 50})).unwrap());
    s.execute("select.set", &json!({"ids": [e.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"colorModel": "rgb"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id(&out)).unwrap().kind else { panic!("an image") };
    let img = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert!(img.pixels().all(|p| p[3] == 0 || p[3] == 255), "hard edges");
    assert!(img.pixels().any(|p| p[3] == 0) && img.pixels().any(|p| p[3] == 255));
}

#[test]
fn rasterize_takes_the_document_settings_as_defaults() {
    let mut s = session();
    s.execute(C, &json!({"background": "white", "colorModel": "grayscale", "addAround": 10})).unwrap();
    let r = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true, "ids": [r.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"ppi": 72})).unwrap();
    assert_eq!((out["width"].as_u64(), out["height"].as_u64()), (Some(60), Some(60)), "10 pt around");
    let d = &s.doc().unwrap().doc;
    let NodeKind::Image(im) = &d.node(id(&out)).unwrap().kind else { panic!("an image") };
    let img = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255], "white around the art");
    let p = img.get_pixel(30, 30).0;
    assert!(p[0] == p[1] && p[1] == p[2] && p[0] < 128, "red in grayscale: {p:?}");
    // Params win; a clipping mask makes a clip group with the art's outline.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let out = s.execute("object.rasterize", &json!({"ppi": 72, "padding": 0, "clippingMask": true, "background": "transparent"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: true } = &d.node(id(&out)).unwrap().kind else { panic!("a clip group") };
    let clip = children[0].geometric_bounds().unwrap();
    assert!((clip.width() - 40.0).abs() < 1e-6 && matches!(children[1].kind, NodeKind::Image(_)));
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), [id(&out)]);
    assert!(s.execute("object.rasterize", &json!({"colorModel": "lab"})).is_err());
}

/// An image Object › Rasterize or Expand Appearance made, `n` or the image in its clip group:
/// (width, height, where its pixels lie, its PNG, the clipping path's geometry).
fn rasterized(d: &Document, n: &vectorcraft_doc::Node) -> (u32, u32, vectorcraft_geom::Affine, Vec<u8>, Option<vectorcraft_geom::PathData>) {
    let (image, clip) = match &n.kind {
        NodeKind::Group { children, clip: true } => (&*children[1], children[0].path_data().cloned()),
        _ => (n, None),
    };
    let NodeKind::Image(im) = &image.kind else { panic!("an image: {:?}", image.kind_label()) };
    (im.width, im.height, im.xf, d.images[&im.key].bytes.to_vec(), clip)
}

/// A rectangle of 40 × 30 pt with its default stroke and a red fill, selected.
fn red_rectangle(s: &mut Session) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": 50.5, "y": 50.25, "width": 40, "height": 30})).unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [r.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    r
}

#[test]
fn the_rasterize_effect_keeps_the_path_and_expands_to_the_image_object_rasterize_makes() {
    let cases = [
        (
            json!({"useDocumentResolution": false, "resolution": 150, "background": "white", "colorModel": "grayscale", "antiAlias": "none", "addAround": 3}),
            json!({"ppi": 150, "background": "white", "colorModel": "grayscale", "antiAlias": "none", "addAround": 3}),
        ),
        // At the document's resolution (300 ppi here), clipped to the art's outline.
        (
            json!({"clippingMask": true, "antiAlias": "type"}),
            json!({"ppi": 300, "clippingMask": true, "antiAlias": "type", "background": "transparent"}),
        ),
    ];
    for (effect, object) in cases {
        let mut s = session();
        s.execute(C, &json!({"resolution": "high"})).unwrap();
        let r = red_rectangle(&mut s);
        let before = s.doc().unwrap().doc.node(r).unwrap().path_data().cloned();
        s.execute("effect.apply", &json!({"effect": "rasterize", "params": effect, "ids": [r.0]})).unwrap();
        let d = s.doc().unwrap().doc.clone();
        let n = d.node(r).unwrap();
        assert_eq!((n.path_data().cloned(), n.appearance.effects[0].id.as_str()), (before, "rasterize"), "the path stays as it was");
        // Saved and read back.
        let back = vectorcraft_format::load(&vectorcraft_format::save_file(&d)).unwrap();
        assert_eq!(back.node(r).unwrap().appearance.effects, n.appearance.effects);
        // Object › Rasterize of the same rectangle.
        let mut o = session();
        o.execute(C, &json!({"resolution": "high"})).unwrap();
        red_rectangle(&mut o);
        let made = id(&o.execute("object.rasterize", &object).unwrap());
        let od = &o.doc().unwrap().doc;
        let want = rasterized(od, od.node(made).unwrap());
        // PDF export draws that image in its place.
        let flat = crate::flatten_raster_effects(&d).unwrap();
        assert_eq!(rasterized(&flat, &flat.layers[0].children().unwrap()[0]), want, "{effect}");
        // So does Expand Appearance.
        s.execute("effect.expandAppearance", &json!({"ids": [r.0]})).unwrap();
        let d = &s.doc().unwrap().doc;
        assert_eq!(rasterized(d, d.node(r).unwrap()), want, "{effect}");
    }
}

#[test]
fn the_rasterize_effect_follows_edits_to_the_path() {
    let mut s = session();
    let r = red_rectangle(&mut s);
    s.execute("effect.apply", &json!({"effect": "rasterize", "params": {"background": "white"}, "ids": [r.0]})).unwrap();
    let width = |s: &Session| {
        let flat = crate::flatten_raster_effects(&s.doc().unwrap().doc).unwrap();
        rasterized(&flat, &flat.layers[0].children().unwrap()[0]).0
    };
    let before = width(&s);
    // Direct Selection moves the right-hand anchors 20 pt: the image widens with the path.
    s.execute("select.anchors", &json!({"id": r.0, "anchors": [[0, 1], [0, 2]]})).unwrap();
    s.execute("path.moveAnchors", &json!({"dx": 20, "dy": 0})).unwrap();
    let n = s.doc().unwrap().doc.node(r).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Path { .. }) && n.appearance.effects.len() == 1);
    assert_eq!(width(&s), before + 20, "72 ppi: a pixel a point");
}

#[test]
fn rasterize_measures_the_art_its_geometry_effects_draw() {
    let mut s = session();
    let r = red_rectangle(&mut s);
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 100}, "ids": [r.0]})).unwrap();
    s.execute("effect.apply", &json!({"effect": "rasterize", "params": {"background": "white"}, "ids": [r.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids": [r.0]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (width, _, xf, ..) = rasterized(d, d.node(r).unwrap());
    // The rectangle as the Transform effect draws it: 100 pt to the right, 41 pt wide with its stroke.
    assert_eq!((xf.translation().x, width), (150.0, 41));
}

#[test]
fn the_clipping_mask_follows_the_art_its_geometry_effects_draw() {
    let clip = |s: &Session, n: NodeId| match &s.doc().unwrap().doc.node(n).unwrap().kind {
        NodeKind::Group { children, clip: true } => children[0].geometric_bounds(),
        _ => None,
    };
    let moved = |s: &mut Session, n: NodeId| {
        s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 100}, "ids": [n.0]})).unwrap();
    };
    // Expand Appearance clips the image to the rectangle drawn 100 pt to the right, from 150.5 to
    // 190.5 pt.
    let mut s = session();
    let r = red_rectangle(&mut s);
    moved(&mut s, r);
    s.execute("effect.apply", &json!({"effect": "rasterize", "params": {"clippingMask": true}, "ids": [r.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({"ids": [r.0]})).unwrap();
    let b = clip(&s, r).expect("a clip group");
    assert!((b.x0 - 150.5).abs() < 1e-6 && (b.x1 - 190.5).abs() < 1e-6, "{b:?}");
    // Object › Rasterize clips its image there too.
    let mut s = session();
    let r = red_rectangle(&mut s);
    moved(&mut s, r);
    let made = id(&s.execute("object.rasterize", &json!({"clippingMask": true})).unwrap());
    let b = clip(&s, made).expect("a clip group");
    assert!((b.x0 - 150.5).abs() < 1e-6 && (b.x1 - 190.5).abs() < 1e-6, "{b:?}");
    // Type's image is clipped to its glyphs, 100 pt right of where they are without the effect.
    let glyphs = |effect: bool| {
        let mut s = session();
        let t = id(&s.execute("text.create", &json!({"x": 50, "y": 100, "text": "H", "size": 40})).unwrap());
        if effect {
            moved(&mut s, t);
        }
        s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
        let made = id(&s.execute("object.rasterize", &json!({"clippingMask": true})).unwrap());
        clip(&s, made).expect("a clip group")
    };
    let (plain, shifted) = (glyphs(false), glyphs(true));
    assert!((shifted.x0 - plain.x0 - 100.0).abs() < 1e-6 && (shifted.x1 - plain.x1 - 100.0).abs() < 1e-6, "{plain:?} {shifted:?}");
}

#[test]
fn object_rasterize_takes_in_everything_an_effect_stack_paints() {
    let mut s = session();
    let r = red_rectangle(&mut s);
    s.execute("effect.apply", &json!({"effect": "blur.gaussian", "params": {"radius": 4}, "ids": [r.0]})).unwrap();
    let live = json!({"useDocumentResolution": false, "resolution": 72, "background": "white", "addAround": 10});
    s.execute("effect.apply", &json!({"effect": "rasterize", "params": live, "ids": [r.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let made = id(&s.execute("object.rasterize", &json!({"ppi": 72, "background": "transparent"})).unwrap());
    let d = &s.doc().unwrap().doc;
    let (width, height, xf, png, _) = rasterized(d, d.node(made).unwrap());
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    // 41 pt of rectangle and stroke, and 6 pt of blur and 10 pt of white on each side: white from
    // the first column to the last.
    assert_eq!((width, xf.translation().x), (73, 34.0));
    assert_eq!((img.get_pixel(0, height / 2).0, img.get_pixel(width - 1, height / 2).0), ([255; 4], [255; 4]));
}

#[test]
fn raster_settings_round_trip_and_old_files_load() {
    let mut d = Document::new(10.0, 10.0);
    d.raster_effects =
        RasterEffectsSettings { color_model: RasterColorModel::Bitmap, background: Background::White, add_around: 12.0, ..Default::default() };
    let back = vectorcraft_format::load(&vectorcraft_format::save_file(&d)).unwrap();
    assert_eq!(back.raster_effects, d.raster_effects);
    // Saved before the settings existed: only the resolution.
    let old = Document::new(10.0, 10.0);
    let bytes = vectorcraft_format::save_file(&old);
    assert!(!String::from_utf8_lossy(&bytes).contains("raster_effects\""), "default settings aren't written");
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!((back.raster_effects_ppi, back.raster_effects), (72.0, RasterEffectsSettings::default()));
}

/// Save the active document with the raster effects resolution `ppi` and open the file.
fn reopen_with_ppi(s: &mut Session, ppi: f64) {
    let mut d = (*s.doc().unwrap().doc).clone();
    d.raster_effects_ppi = ppi;
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "ppi.vectorcraft", "dataBase64": data})).unwrap();
}

#[test]
fn a_file_with_the_resolution_out_of_range_opens_with_it_in_range() {
    for (saved, opened, pixels) in [(0.0, 1.0, 1), (-5.0, 1.0, 1), (5000.0, 2400.0, 334), (f64::MAX, 2400.0, 334)] {
        let mut s = session();
        let r = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 10, "height": 10})).unwrap());
        s.execute("paint.setStroke", &json!({"none": true, "ids": [r.0]})).unwrap();
        reopen_with_ppi(&mut s, saved);
        assert_eq!(s.doc().unwrap().doc.raster_effects_ppi, opened, "saved as {saved}");
        assert_eq!(s.execute(C, &json!({})).unwrap()["resolution"], opened);
        assert_eq!(s.execute("document.info", &json!({})).unwrap()["document"]["rasterEffectsPpi"], opened);
        // Rasterize without a ppi param renders the 10 pt square at it.
        s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
        let out = s.execute("object.rasterize", &json!({})).unwrap();
        assert_eq!((out["width"].as_u64(), out["height"].as_u64()), (Some(pixels), Some(pixels)), "saved as {saved}");
    }
}

#[test]
fn a_gradient_exports_at_the_resolution_the_file_opens_with() {
    for saved in [0.0, -5.0] {
        let mut s = session();
        let r = id(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 720, "height": 720})).unwrap());
        s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}, "ids": [r.0]})).unwrap();
        reopen_with_ppi(&mut s, saved);
        // EMF writes the gradient as an image at the raster effects resolution: 720 pt at 1 ppi.
        let region = vectorcraft_geom::Rect::new(0.0, 0.0, 720.0, 720.0);
        let out = vectorcraft_metafile::export(&s.doc().unwrap().doc, region, vectorcraft_metafile::Kind::Emf).unwrap();
        let mut sizes = vec![];
        vectorcraft_metafile::import(&out.bytes).unwrap().document.walk(|n| {
            if let NodeKind::Image(im) = &n.kind {
                sizes.push((im.width, im.height));
            }
        });
        assert_eq!(sizes, [(10, 10)], "saved as {saved}");
    }
}
