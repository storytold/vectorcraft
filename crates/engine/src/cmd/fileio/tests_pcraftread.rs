//! Reading `.pcraft` documents built here from scratch (a hand-written manifest, PNG previews
//! encoded here and layers whose surfaces are one default pixel, in a stored ZIP archive): the
//! full-size composite is the image when the bundle has one; otherwise visible Normal raster layers
//! are combined at the canvas's size; otherwise the preview is read, with a note when it is smaller
//! than the canvas. Damaged and unreadable bundles are refused with a message.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use super::*;

/// A surface of one `pixel` (hex of its bytes) everywhere, in `sample` (U8 or U16).
fn surface(sample: &str, mode: &str, alpha: bool, pixel: &str) -> String {
    format!(r#"{{"format":{{"mode":"{mode}","sample":"{sample}","alpha":{alpha}}},"default":"{pixel}","tiles":[]}}"#)
}

/// An 8-bit RGBA surface of one pixel everywhere.
fn rgba8(p: [u8; 4]) -> String {
    surface("U8", "Rgb", true, &p.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

/// Effects that are on, with none in the list.
const NO_EFFECTS: &str = r#"{"enabled":true,"items":[],"psd_raw":null}"#;

/// A layer: `visible`, `blend`, `opacity`, `content`, its `mask` and `effects` (JSON), and `extra`
/// members (each `,"key":value`).
fn layer_with(visible: bool, blend: &str, opacity: f32, content: &str, mask: &str, effects: &str, extra: &str) -> String {
    format!(
        r#"{{"id":1,"name":"L","visible":{visible},"locks":{{}},"blend":"{blend}","opacity":{opacity},"fill_opacity":1.0,"clipped":false,
            "mask":{mask},"effects":{effects},"label":"None","content":{content},"psd_blocks":[],"psd_id":null,"fill_cache":null{extra}}}"#
    )
}

/// [`layer_with`] without a mask or effects.
fn layer(visible: bool, blend: &str, opacity: f32, content: &str, extra: &str) -> String {
    layer_with(visible, blend, opacity, content, "null", NO_EFFECTS, extra)
}

fn raster(surface: &str) -> String {
    format!(r#"{{"kind":"raster","surface":{surface}}}"#)
}

/// A visible Normal raster layer of `pixel` at `opacity`.
fn plain(pixel: [u8; 4], opacity: f32) -> String {
    layer(true, "Normal", opacity, &raster(&rgba8(pixel)), "")
}

/// A `w` × `h` PNG of `pixel`.
fn png(w: u32, h: u32, pixel: [u8; 4]) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba(pixel)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// What a test bundle holds besides its layers.
struct Bundle<'a> {
    size: (u32, u32),
    dpi: f32,
    depth: &'a str,
    version: u32,
    layers: Vec<String>,
    /// The full-size composite's entry, when the manifest names it (`None` bytes: not in the archive).
    full: Option<Option<Vec<u8>>>,
    /// The preview's entry, likewise.
    preview: Option<Option<Vec<u8>>>,
}

impl Bundle<'_> {
    fn new(layers: Vec<String>) -> Self {
        Bundle { size: (300, 200), dpi: 150.0, depth: "U8", version: 1, layers, full: None, preview: Some(Some(png(30, 20, [9, 9, 9, 255]))) }
    }

    fn bytes(&self) -> Vec<u8> {
        let (w, h) = self.size;
        let named = |m: &Option<Option<Vec<u8>>>, path: &str| if m.is_some() { format!(r#""{path}""#) } else { "null".into() };
        let mut manifest = format!(
            r#"{{"format_version":{},"generator":"test 0.1.0",
                "document":{{"id":1,"name":"Untitled","size":{{"width":{w},"height":{h}}},"resolution_dpi":{},"mode":"Rgb","depth":"{}",
                  "icc_profile":null,"layers":[{}],"channels":[],"guides":{{}},"selection":null,"metadata":{{}},"global_light":{{}}}},
                "thumbnail":null,"composite":{}"#,
            self.version,
            self.dpi,
            self.depth,
            self.layers.join(","),
            named(&self.preview, "composite/preview.png"),
        );
        if self.full.is_some() {
            manifest.push_str(r#","full_composite":"composite/full.png""#);
        }
        manifest.push('}');
        let mut files: Vec<(&str, Vec<u8>)> = vec![("manifest.json", manifest.into_bytes())];
        if let Some(Some(b)) = &self.preview {
            files.push(("composite/preview.png", b.clone()));
        }
        if let Some(Some(b)) = &self.full {
            files.push(("composite/full.png", b.clone()));
        }
        super::zip::store(&files).unwrap()
    }
}

/// Place `file` (named `art.pcraft`) in a new document: the reply and the placed image's (natural
/// width in points, its stored pixels).
fn place(file: &[u8]) -> (Value, f64, image::DynamicImage) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    let r = s.execute("file.place", &json!({"name": "art.pcraft", "dataBase64": vectorcraft_format::base64_encode(file)})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let node = doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    let NodeKind::Image(im) = &node.kind else { panic!("an image, got {:?}", node.kind) };
    let width = node.geometric_bounds().map(|b| b.width()).unwrap();
    let pixels = image::load_from_memory(&doc.images[&im.key].bytes).unwrap();
    (r, width, pixels)
}

fn warnings(r: &Value) -> Vec<String> {
    r["warnings"].as_array().unwrap().iter().map(|w| w.as_str().unwrap().to_string()).collect()
}

fn place_error(file: &[u8]) -> String {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    s.execute("file.place", &json!({"name": "art.pcraft", "dataBase64": vectorcraft_format::base64_encode(file)})).unwrap_err().to_string()
}

#[test]
fn the_full_size_composite_is_placed_rather_than_the_layers_or_the_preview() {
    // The red layer could be combined, and the preview the manifest names is not in the archive.
    let mut b = Bundle::new(vec![plain([255, 0, 0, 255], 1.0)]);
    b.full = Some(Some(png(300, 200, [0, 200, 0, 255])));
    b.preview = Some(None);
    let file = b.bytes();
    assert!(super::pcraftread::is_pcraft(&file));
    assert_eq!(detect("art.zip", &file).map(|f| f.id), Some("pcraft"), "by its content");
    let (r, width, pixels) = place(&file);
    assert_eq!(r["format"], "pcraft");
    assert!(warnings(&r).is_empty(), "{r}");
    assert_eq!((pixels.width(), pixels.height()), (300, 200));
    assert_eq!(pixels.to_rgba8().get_pixel(5, 5).0, [0, 200, 0, 255]);
    // Sized by the document's resolution: 300 px at 150 ppi are 144 pt (the PNG keeps the
    // resolution in whole pixels per meter).
    assert!((width - 144.0).abs() < 0.5, "{width}");
}

#[test]
fn visible_normal_raster_layers_are_combined_at_the_canvas_size() {
    let file = Bundle::new(vec![
        plain([255, 0, 0, 255], 1.0),
        // Half its alpha and half its opacity: a quarter over the red.
        plain([0, 0, 255, 128], 0.5),
        // Hidden layers draw nothing, whatever they are.
        layer(false, "Normal", 1.0, &raster(&rgba8([0, 255, 0, 255])), ""),
        layer(false, "Screen", 1.0, r#"{"kind":"text","text":"A","cache":null}"#, r#","advanced":{"knockout":"deep"}"#),
    ])
    .bytes();
    let (r, width, pixels) = place(&file);
    assert!(warnings(&r).is_empty(), "{r}");
    assert_eq!((pixels.width(), pixels.height()), (300, 200), "the canvas's size, not the 30 × 20 preview's");
    let [red, green, blue, alpha] = pixels.to_rgba8().get_pixel(150, 100).0;
    assert!(red > blue && blue > 0 && green == 0 && alpha == 255, "{:?}", [red, green, blue, alpha]);
    assert!((width - 144.0).abs() < 0.5, "{width}");

    // A 16-bit document's layers combine into 16-bit samples.
    let mut b = Bundle::new(vec![layer(true, "Normal", 1.0, &raster(&surface("U16", "Rgb", true, "ffff000000000080")), "")]);
    b.depth = "U16";
    let (_, _, pixels) = place(&b.bytes());
    assert!(matches!(pixels, image::DynamicImage::ImageRgba16(_)), "{:?}", pixels.color());
}

#[test]
fn a_layer_mask_hides_by_its_density_unless_it_is_off() {
    // A mask that hides everything, at half density.
    let mask = |enabled: bool, feather: f32| {
        format!(r#"{{"surface":{},"enabled":{enabled},"linked":true,"density":0.5,"feather":{feather}}}"#, surface("U8", "Grayscale", false, "00"))
    };
    let blue_at = |mask: &str| {
        let top = layer_with(true, "Normal", 1.0, &raster(&rgba8([0, 0, 255, 255])), mask, NO_EFFECTS, "");
        let (r, _, pixels) = place(&Bundle::new(vec![plain([255, 0, 0, 255], 1.0), top]).bytes());
        (warnings(&r), pixels.to_rgba8().get_pixel(0, 0).0)
    };
    let (w, [red, _, blue, _]) = blue_at(&mask(true, 0.0));
    assert!(w.is_empty() && red > 100 && blue > 100, "half hidden: {red} {blue}");
    let (w, [red, _, blue, _]) = blue_at(&mask(false, 0.0));
    assert!(w.is_empty() && red == 0 && blue == 255, "a mask that is off hides nothing: {red} {blue}");
    let (w, [red, ..]) = blue_at(&mask(true, 4.0));
    assert!(w.iter().any(|w| w.contains("feathered layer mask")) && red == 9, "a feathered mask leaves the preview: {w:?}");
}

#[test]
fn layers_that_cant_be_combined_leave_the_preview_with_a_note() {
    let file = Bundle::new(vec![plain([255, 0, 0, 255], 1.0), layer(true, "Multiply", 1.0, &raster(&rgba8([0, 0, 255, 255])), "")]).bytes();
    let (r, width, pixels) = place(&file);
    assert_eq!((pixels.width(), pixels.height()), (30, 20));
    let w = warnings(&r);
    assert!(w.len() == 1 && w[0].contains("30 × 20 preview") && w[0].contains("300 × 200") && w[0].contains("Normal"), "{w:?}");
    // The preview covers the canvas, at the same size as the full image.
    assert!((width - 144.0).abs() < 0.5, "{width}");

    // Each other setting that changes how a layer draws leaves the preview too.
    let pixels = raster(&rgba8([0; 4]));
    let shadow = r#"{"enabled":true,"items":[{"DropShadow":{}}],"psd_raw":null}"#;
    for (content, effects, extra, said) in [
        (pixels.as_str(), shadow, "", "layer effects"),
        (&pixels, NO_EFFECTS, r#","blend_if":{"ranges":[1]}"#, "advanced blending"),
        (&pixels, NO_EFFECTS, r#","excluded_channels":2"#, "advanced blending"),
        (&pixels, NO_EFFECTS, r#","vector_mask":{"enabled":true,"path":{}}"#, "vector mask"),
        (&pixels, NO_EFFECTS, r#","frame":{"path":{}}"#, "frame"),
        (&pixels, NO_EFFECTS, r#","a_new_setting":true"#, "a_new_setting"),
        (r#"{"kind":"group","children":[],"expanded":true}"#, NO_EFFECTS, "", "layer group"),
        (r#"{"kind":"text","text":"A","cache":null}"#, NO_EFFECTS, "", "type layer"),
    ] {
        let l = layer_with(true, "Normal", 1.0, content, "null", effects, extra);
        let (r, _, pixels) = place(&Bundle::new(vec![l]).bytes());
        let w = warnings(&r);
        assert!(pixels.width() == 30 && w.len() == 1 && w[0].contains(said), "{said}: {w:?}");
    }

    // A preview of the canvas's own size is not reduced, so nothing is said.
    let mut b = Bundle::new(vec![layer(true, "Multiply", 1.0, &raster(&rgba8([0; 4])), "")]);
    b.size = (30, 20);
    let (r, _, pixels) = place(&b.bytes());
    assert!(warnings(&r).is_empty() && pixels.width() == 30, "{r}");
}

/// A linked preview keeps its size when it is read again: the resolution it was placed at is the
/// one its stored PNG gives back, at least 1 ppi when the canvas is far larger than the preview.
#[test]
fn a_linked_preview_keeps_its_size_when_it_is_updated() {
    let dir = vectorcraft_testkit::TempDir::new("pcraft-preview-size");
    for (size, dpi) in [((300, 200), 150.0), ((300, 200), 72.0), ((3000, 2000), 10.0)] {
        let mut b = Bundle::new(vec![layer(true, "Multiply", 1.0, &raster(&rgba8([0; 4])), "")]);
        (b.size, b.dpi) = (size, dpi);
        let path = dir.join("art.pcraft");
        std::fs::write(&path, b.bytes()).unwrap();
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
        let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": true})).unwrap();
        let id = NodeId(r["ids"][0].as_u64().unwrap());
        let width = |s: &Session| s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().width();
        let placed = width(&s);
        for preserve in ["transforms", "bounds"] {
            s.execute("links.relink", &json!({"ids": [id.0], "path": path.to_string_lossy(), "preserve": preserve})).unwrap();
            s.execute("links.update", &json!({"ids": [id.0]})).unwrap();
            assert!((width(&s) - placed).abs() < 1e-6, "{size:?} at {dpi} ppi, {preserve}: {} then {placed}", width(&s));
        }
        // The 30 px preview places at the canvas's width, or at 1 ppi when that would take less.
        let ppi = (f64::from(dpi) * 30.0 / f64::from(size.0)).max(1.0);
        assert!((placed - 30.0 * 72.0 / ppi).abs() < 0.5, "{size:?} at {dpi} ppi: {placed}");
    }
}

#[test]
fn damaged_or_unreadable_bundles_are_refused() {
    let multiply = || vec![layer(true, "Multiply", 1.0, &raster(&rgba8([0; 4])), "")];
    // The full-size composite the manifest names is missing, or not the canvas's size: never the preview.
    let mut b = Bundle::new(multiply());
    b.full = Some(None);
    assert!(place_error(&b.bytes()).contains("composite/full.png"));
    b.full = Some(Some(png(30, 20, [0; 4])));
    assert!(place_error(&b.bytes()).contains("not 300 × 200"));
    b.full = Some(Some(b"not a png".to_vec()));
    assert!(place_error(&b.bytes()).contains("isn't a PNG"));
    // No preview and layers that can't be combined.
    let mut b = Bundle::new(multiply());
    b.preview = None;
    assert!(place_error(&b.bytes()).contains("no full-size image or preview"));
    // A newer format version.
    let mut b = Bundle::new(multiply());
    b.version = 99;
    assert!(place_error(&b.bytes()).contains("newer"));
}

#[test]
fn pcraft_files_open_and_place_linked_and_updates_read_them_again() {
    let b64 = vectorcraft_format::base64_encode;
    let red = Bundle::new(vec![plain([255, 0, 0, 255], 1.0)]).bytes();
    let blue = Bundle::new(vec![plain([0, 0, 255, 255], 1.0)]).bytes();
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "art.pcraft", "dataBase64": b64(&red)})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("pcraft"), &json!([])), "{r}");
    let multiply = Bundle::new(vec![layer(true, "Multiply", 1.0, &raster(&rgba8([0; 4])), "")]).bytes();
    let r = s.execute("document.open", &json!({"name": "art.pcraft", "dataBase64": b64(&multiply)})).unwrap();
    assert!(r["warnings"].as_array().unwrap().iter().any(|w| w.as_str().unwrap().contains("preview")), "Open says so too: {r}");

    let dir = vectorcraft_testkit::TempDir::new("pcraft-link");
    let path = dir.join("art.pcraft");
    std::fs::write(&path, &red).unwrap();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    let r = s.execute("file.place", &json!({"path": path.to_string_lossy(), "link": true})).unwrap();
    let id = r["ids"][0].as_u64().unwrap();
    let shown = |s: &Session| {
        let doc = &s.doc().unwrap().doc;
        let NodeKind::Image(im) = &doc.node(NodeId(id)).unwrap().kind else { panic!("an image") };
        assert!(im.link.is_some(), "linked");
        image::load_from_memory(&doc.images[&im.key].bytes).unwrap().to_rgba8().get_pixel(0, 0).0
    };
    assert_eq!(shown(&s), [255, 0, 0, 255]);
    std::fs::write(&path, &blue).unwrap();
    s.execute("links.update", &json!({"ids": [id]})).unwrap();
    assert_eq!(shown(&s), [0, 0, 255, 255], "the update reads the file again");
    // A file that now has only its preview to show says so.
    std::fs::write(&path, &multiply).unwrap();
    let r = s.execute("links.update", &json!({"ids": [id]})).unwrap();
    let w = warnings(&r);
    assert!(w.len() == 1 && w[0].starts_with("art.pcraft: only the 30 × 20 preview"), "{r}");
    // A preview whose header reads but whose pixels don't is not taken: the link counts as missing.
    let mut damaged = png(30, 20, [9, 9, 9, 255]);
    let n = damaged.len();
    damaged[40..n - 12].fill(0xff);
    let mut b = Bundle::new(vec![layer(true, "Multiply", 1.0, &raster(&rgba8([0; 4])), "")]);
    b.preview = Some(Some(damaged));
    let damaged = b.bytes();
    let e = s.execute("document.open", &json!({"name": "art.pcraft", "dataBase64": b64(&damaged)})).unwrap_err();
    assert!(e.to_string().contains("pixels can't be read"), "{e}");
    std::fs::write(&path, damaged).unwrap();
    let r = s.execute("links.update", &json!({"ids": [id]})).unwrap();
    assert_eq!(r["missing"], json!([id]), "{r}");
    let r = s.execute("links.replaceEmbedded", &json!({"ids": [id], "dataBase64": b64(&multiply)})).unwrap();
    assert!(warnings(&r).iter().any(|w| w.contains("preview")), "{r}");

    // The dialogs list it; the desktop packaging doesn't associate it; nothing writes it.
    assert!(OPEN_EXTS.contains(&"pcraft") && PLACE_EXTS.contains(&"pcraft") && UNASSOCIATED_EXTS.contains(&"pcraft"));
    let r = s.execute("document.formats", &json!({})).unwrap();
    let f = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "pcraft").unwrap().clone();
    assert_eq!((f["read"].as_bool(), f["write"].as_bool()), (Some(true), Some(false)));
}
