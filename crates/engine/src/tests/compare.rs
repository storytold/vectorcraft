//! `document.compare`: an artboard rendered at an image's size, as a PNG export draws it, and
//! measured against the image (RMSE, the pixels over a threshold, a difference image), with the
//! image and the render flattened on one background; images of other proportions, files that
//! can't be read and paths outside the automation roots are errors.

use serde_json::{Value, json};

use crate::Session;

/// A 40 × 20 pt document with a black 10 × 10 pt square at (10, 5), no stroke.
fn session() -> (Session, Value) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": 10, "y": 5, "width": 10, "height": 10})).unwrap()["id"].clone();
    (s, id)
}

fn path_in(dir: &std::path::Path, name: &str) -> String {
    dir.join(name).to_string_lossy().into_owned()
}

/// The artboard exported as a PNG at 2 pixels a point on white.
fn export(s: &mut Session, path: &str) {
    s.execute("document.export", &json!({"path": path, "format": "png", "scale": 2, "background": "white"})).unwrap();
}

fn compare(s: &mut Session, p: Value) -> Value {
    s.execute("document.compare", &p).unwrap_or_else(|e| panic!("document.compare {p}: {e}"))
}

fn error(s: &mut Session, p: Value) -> String {
    s.execute("document.compare", &p).map(|v| panic!("document.compare {p} answered {v}")).unwrap_err().to_string()
}

#[test]
fn an_export_compares_equal_and_a_moved_square_counts_the_pixels_it_changed() {
    let dir = vectorcraft_testkit::temp_dir("compare-moved");
    let png = path_in(&dir, "art.png");
    let (mut s, id) = session();
    export(&mut s, &png);
    let before = (s.doc().unwrap().revision, s.doc().unwrap().history.undo.len());
    let r = compare(&mut s, json!({"path": png}));
    assert_eq!(r, json!({"width": 80, "height": 40, "rmse": 0.0, "over": 0, "total": 3200, "threshold": 0.08}));
    assert_eq!((s.doc().unwrap().revision, s.doc().unwrap().history.undo.len()), before, "comparing doesn't edit the document");
    // 4 pt is 8 pixels: 8 columns of the square's 20 rows turn white and 8 turn black, every
    // channel by 255.
    s.execute("object.move", &json!({"ids": [id], "dx": 4, "dy": 0})).unwrap();
    let r = compare(&mut s, json!({"path": png}));
    let over = 2 * 8 * 20;
    assert_eq!((r["over"].as_u64(), r["total"].as_u64()), (Some(over), Some(3200)), "{r}");
    let rmse = ((over as f64 / 3200.0).sqrt() * 1e6).round() / 1e6;
    assert_eq!(r["rmse"].as_f64(), Some(rmse), "{r}");
    // Above every difference, nothing is over.
    assert_eq!(compare(&mut s, json!({"path": png, "threshold": 1}))["over"], json!(0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_image_and_the_render_are_flattened_on_the_background() {
    let dir = vectorcraft_testkit::temp_dir("compare-background");
    let png = path_in(&dir, "clear.png");
    // Transparent everywhere, with red in the color channels that the alpha hides.
    image::RgbaImage::from_pixel(80, 40, image::Rgba([255, 0, 0, 0])).save(&png).unwrap();
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 20})).unwrap();
    // Nothing drawn: both are the background alone.
    for background in ["#808080", "#ffffff"] {
        let r = compare(&mut s, json!({"path": png, "background": background}));
        assert_eq!((r["rmse"].as_f64(), r["over"].as_u64()), (Some(0.0), Some(0)), "{background}: {r}");
    }
    // A black left half: 128 from gray in every channel of half the pixels, 255 from white.
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    let r = compare(&mut s, json!({"path": png, "background": "#808080"}));
    let rmse = |d: f64| ((0.5f64).sqrt() * d / 255.0 * 1e6).round() / 1e6;
    assert_eq!((r["rmse"].as_f64(), r["over"].as_u64()), (Some(rmse(128.0)), Some(1600)), "{r}");
    let r = compare(&mut s, json!({"path": png}));
    assert_eq!((r["rmse"].as_f64(), r["over"].as_u64()), (Some(rmse(255.0)), Some(1600)), "{r}");
    // A threshold just above 128/255 leaves the gray differences out.
    assert_eq!(compare(&mut s, json!({"path": png, "background": "#808080", "threshold": 0.51}))["over"], json!(0));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_image_of_other_proportions_is_refused_and_one_pixel_is_allowed() {
    let dir = vectorcraft_testkit::temp_dir("compare-size");
    let (mut s, _) = session();
    let square = path_in(&dir, "square.png");
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255; 4])).save(&square).unwrap();
    let e = error(&mut s, json!({"path": square}));
    assert!(e.contains("30 × 30 pixels") && e.contains("30 × 15 pixels"), "{e}");
    let taller = path_in(&dir, "taller.png");
    image::RgbaImage::from_pixel(80, 41, image::Rgba([255; 4])).save(&taller).unwrap();
    let r = compare(&mut s, json!({"path": taller}));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64(), r["total"].as_u64()), (Some(80), Some(41), Some(3280)), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn an_export_whose_height_was_rounded_compares_equal() {
    let dir = vectorcraft_testkit::temp_dir("compare-rounded");
    let png = path_in(&dir, "art.png");
    // 20.25 pt at 2 pixels a point is 40.5 rows, which the export rounds to 41.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 40, "height": 20.25})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#000000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 5, "width": 10, "height": 10})).unwrap();
    export(&mut s, &png);
    let r = compare(&mut s, json!({"path": png}));
    assert_eq!(r, json!({"width": 80, "height": 41, "rmse": 0.0, "over": 0, "total": 3280, "threshold": 0.08}));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_linked_image_is_drawn_from_its_file_as_a_png_export_draws_it() {
    use crate::tests_links::{BLUE, Folder, RED, open, place, png, save, session, write};
    let dir = Folder::new("compare-linked");
    let pic = dir.file("photo.png");
    write(&pic, &png(600, 300, RED));
    let mut s = session();
    place(&mut s, &pic);
    let doc_path = dir.file("poster.vectorcraft");
    save(&mut s, &doc_path);
    // The file changes after the save: the reopened document shows the red preview saved for it,
    // and a PNG export reads the blue file.
    write(&pic, &png(600, 300, BLUE));
    open(&mut s, &doc_path);
    let out = dir.file("export.png");
    s.execute("document.export", &json!({"path": out, "format": "png", "background": "white"})).unwrap();
    let r = compare(&mut s, json!({"path": out}));
    assert_eq!((r["rmse"].as_f64(), r["over"].as_u64(), r.get("warnings")), (Some(0.0), Some(0), None), "{r}");
    // Without the file the preview is drawn, and the reply says so.
    std::fs::remove_file(&pic).unwrap();
    let r = compare(&mut s, json!({"path": out}));
    assert_eq!(r["over"].as_u64(), r["total"].as_u64(), "{r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.len() == 1 && warnings[0].as_str().unwrap().contains("output as their previews"), "{r}");
    // With type that doesn't fit its frame as well, the warnings come in the export's order.
    let overset =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Ab\nCd\nEf\nGh", "size": 12, "area": {"width": 100, "height": 20}})).unwrap();
    assert_eq!(overset["overflow"], json!([overset["id"]]), "{overset}");
    let exported = s.execute("document.export", &json!({"path": dir.file("now.png"), "format": "png", "background": "white"})).unwrap();
    let r = compare(&mut s, json!({"path": out}));
    assert_eq!(r["warnings"].as_array().map(Vec::len), Some(2), "{r}");
    assert_eq!(r["warnings"], exported["warnings"], "{r}");
}

#[test]
fn an_image_that_cannot_be_decoded_and_a_missing_font_warn_as_a_png_export_does() {
    let dir = vectorcraft_testkit::temp_dir("compare-warnings");
    let png = path_in(&dir, "art.png");
    let (mut s, _) = session();
    export(&mut s, &png);
    assert_eq!(compare(&mut s, json!({"path": png})).get("warnings"), None);
    crate::tests_menucmds::place_undecodable_png(&mut s);
    s.execute("text.create", &json!({"x": 2, "y": 18, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let overset = s.execute("text.create", &json!({"x": 22, "y": 2, "text": "Ab\nCd\nEf", "size": 6, "area": {"width": 15, "height": 8}})).unwrap();
    assert_eq!(overset["overflow"], json!([overset["id"]]), "{overset}");
    let exported = s.execute("document.export", &json!({"path": path_in(&dir, "now.png"), "format": "png", "scale": 2})).unwrap();
    let r = compare(&mut s, json!({"path": png}));
    assert_eq!(r["warnings"], exported["warnings"], "{r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert!(warnings.iter().any(|w| w == "an image couldn't be decoded and is missing from the picture"), "{r}");
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("Nonexistent Sans Probe")), "{r}");
    assert!(warnings.iter().any(|w| w.as_str().unwrap().contains("doesn't fit its frame")), "{r}");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn diff_path_writes_each_pixels_differences() {
    let dir = vectorcraft_testkit::temp_dir("compare-diff");
    let png = path_in(&dir, "art.png");
    let diff = path_in(&dir, "diff.png");
    let (mut s, id) = session();
    export(&mut s, &png);
    s.execute("object.move", &json!({"ids": [id], "dx": 4, "dy": 0})).unwrap();
    let r = compare(&mut s, json!({"path": png, "diffPath": diff}));
    assert_eq!(r["diffPath"], json!(diff));
    let img = image::open(&diff).unwrap().to_rgba8();
    assert_eq!(img.dimensions(), (80, 40));
    // The square spanned pixels 20–39 and now spans 28–47, rows 10–29.
    for (x, y, want) in [(24, 20, 255), (44, 20, 255), (30, 20, 0), (24, 5, 0), (60, 20, 0)] {
        assert_eq!(img.get_pixel(x, y).0, [want, want, want, 255], "({x}, {y})");
    }
    let changed = img.pixels().filter(|p| p.0[0] != 0).count();
    assert_eq!(changed as u64, r["over"].as_u64().unwrap());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn missing_unreadable_and_malformed_inputs_are_errors() {
    let dir = vectorcraft_testkit::temp_dir("compare-errors");
    let (mut s, _) = session();
    let png = path_in(&dir, "art.png");
    export(&mut s, &png);
    let missing = path_in(&dir, "missing.png");
    assert!(error(&mut s, json!({"path": missing})).contains(&missing));
    let text = path_in(&dir, "notes.png");
    std::fs::write(&text, "not an image\n").unwrap();
    assert!(error(&mut s, json!({"path": text})).contains("not a PNG, JPEG, WebP, TIFF or BMP image"));
    let gif = path_in(&dir, "art.gif");
    image::RgbaImage::from_pixel(80, 40, image::Rgba([255; 4])).save(&gif).unwrap();
    assert!(error(&mut s, json!({"path": gif})).contains("not a PNG, JPEG, WebP, TIFF or BMP image"));
    for (p, want) in [
        (json!({}), "missing `path`"),
        (json!({"path": png, "artboard": 3}), "no artboard 3"),
        (json!({"path": png, "threshold": 2}), "`threshold` must be a number from 0 to 1"),
        (json!({"path": png, "threshold": "low"}), "`threshold` must be a number from 0 to 1"),
        (json!({"path": png, "background": "transparent"}), "a color such as"),
        (json!({"path": png, "diffPath": 5}), "`diffPath` must be a string"),
    ] {
        let e = error(&mut s, p.clone());
        assert!(e.contains(want), "{p}: {e}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// A planar TIFF whose header claims more strips than the decoder can count (2^31 + 1 rows a strip
/// each, times 5 planes) is refused as too large to draw, without a panic.
#[test]
fn a_tiff_with_more_strips_than_can_be_counted_is_too_large() {
    let dir = vectorcraft_testkit::temp_dir("compare-strips");
    let (mut s, _) = session();
    let plane = |v: u16| vec![v; 6];
    let channels = [plane(200), plane(40), plane(10), plane(0), plane(90)];
    let t = vectorcraft_testkit::tiff::Tiff {
        width: 3,
        height: 2,
        photometric: 5,
        bits: 8,
        extra: &[0],
        planar: true,
        ppi: Some(150),
        channels: &channels,
    };
    let mut file = vectorcraft_testkit::tiff::tiff(&t);
    // ImageLength (the directory's second entry, its value at offset 30) and RowsPerStrip (the
    // eighth, at offset 102).
    file[30..34].copy_from_slice(&(1u32 << 31 | 1).to_le_bytes());
    file[102..106].copy_from_slice(&1u32.to_le_bytes());
    assert_eq!(vectorcraft_doc::cmyk::oversized_tiff(&file), Some((3, 1 << 31 | 1)));
    let tif = path_in(&dir, "strips.tif");
    std::fs::write(&tif, &file).unwrap();
    let e = error(&mut s, json!({"path": tif}));
    assert_eq!(e, "invalid parameters for `document.compare`: an image of 3 × 2147483649 pixels is too large to draw");
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_image_and_the_difference_image_stay_inside_the_automation_roots() {
    let dir = vectorcraft_testkit::temp_dir("compare-roots");
    for d in ["in", "out", "elsewhere"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    let (mut s, _) = session();
    let inside = path_in(&dir.join("in"), "art.png");
    let outside = path_in(&dir.join("elsewhere"), "art.png");
    export(&mut s, &inside);
    export(&mut s, &outside);
    let roots = crate::file_access::AutomationRoots::new(&[&dir.join("in")], &[&dir.join("out")]).unwrap().unwrap();
    let mut run = |p: Value| crate::file_access::confine(Some(&roots), || s.execute("document.compare", &p));
    assert!(run(json!({"path": inside})).is_ok());
    let e = run(json!({"path": outside})).unwrap_err().to_string();
    assert!(e.contains("outside the read root"), "{e}");
    let stray = path_in(&dir.join("elsewhere"), "diff.png");
    let e = run(json!({"path": inside, "diffPath": stray})).unwrap_err().to_string();
    assert!(e.contains("outside the write root"), "{e}");
    assert!(!std::path::Path::new(&stray).exists());
    let kept = path_in(&dir.join("out"), "diff.png");
    assert_eq!(run(json!({"path": inside, "diffPath": kept})).unwrap()["diffPath"], json!(kept));
    let _ = std::fs::remove_dir_all(dir);
}
