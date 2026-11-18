//! Export As Text, WebP options and Save for Office Documents.

use serde_json::{Value, json};

use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

fn text(s: &mut Session, x: f64, y: f64, t: &str) -> u64 {
    s.execute("text.create", &json!({"x": x, "y": y, "text": t})).unwrap()["id"].as_u64().unwrap()
}

#[test]
fn text_export_writes_the_stories_in_stacking_order() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    text(&mut s, 10.0, 200.0, "Back story\nsecond line");
    let front = text(&mut s, 10.0, 40.0, "Front");
    // A thread of two frames is one story, written once.
    let a =
        s.execute("text.create", &json!({"x": 10, "y": 80, "text": "Threaded words", "area": {"width": 120, "height": 40}})).unwrap()["id"].clone();
    let b = s.execute("text.create", &json!({"x": 200, "y": 80, "text": "", "area": {"width": 120, "height": 40}})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    // Hidden objects are left out.
    let hidden = text(&mut s, 10.0, 260.0, "Hidden");
    s.execute("select.set", &json!({"ids": [hidden]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();

    let export = |s: &mut Session, p: Value| b64(&s.execute("document.export", &merge(json!({"format": "txt"}), p)).unwrap());
    let utf8 = String::from_utf8(export(&mut s, json!({}))).unwrap();
    assert_eq!(utf8, "Back story\nsecond line\nFront\nThreaded words\n", "back to front, one line ending after each line");
    let crlf = export(&mut s, json!({"lineEndings": "crlf"}));
    assert_eq!(String::from_utf8(crlf).unwrap(), utf8.replace('\n', "\r\n"));
    let utf16 = export(&mut s, json!({"encoding": "utf16"}));
    assert_eq!(&utf16[..2], [0xFF, 0xFE], "byte order mark");
    let units: Vec<u16> = utf16[2..].chunks(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    assert_eq!(String::from_utf16(&units).unwrap(), utf8);
    // Only the selection.
    s.execute("select.set", &json!({"ids": [front]})).unwrap();
    assert_eq!(export(&mut s, json!({"selectionOnly": true})), b"Front\n");
    for bad in [json!({"encoding": "latin1"}), json!({"lineEndings": "cr"})] {
        assert!(s.execute("document.export", &merge(json!({"format": "txt"}), bad.clone())).is_err(), "{bad}");
    }
    // A .txt path picks the format.
    let dir = std::env::temp_dir().join(format!("vc-text-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("story.txt");
    assert_eq!(s.execute("document.export", &json!({"path": path.to_string_lossy()})).unwrap()["format"], "txt");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), utf8);
    let _ = std::fs::remove_dir_all(dir);
}

/// The chunk types of RIFF WebP file `file`, in order.
fn webp_chunks(file: &[u8]) -> Vec<[u8; 4]> {
    assert_eq!((&file[..4], &file[8..12]), (&b"RIFF"[..], &b"WEBP"[..]));
    let mut chunks = Vec::new();
    let mut at = 12;
    while at + 8 <= file.len() {
        let len = u32::from_le_bytes([file[at + 4], file[at + 5], file[at + 6], file[at + 7]]) as usize;
        chunks.push([file[at], file[at + 1], file[at + 2], file[at + 3]]);
        at += 8 + len + len % 2;
    }
    chunks
}

/// The size and straight RGBA pixels of image file `file`.
fn rgba(file: &[u8]) -> (u32, u32, Vec<u8>) {
    let img = craft_image_codecs::decode(file).unwrap();
    let (w, h) = img.dimensions();
    (w, h, img.to_rgba8())
}

/// A 120 × 80 pt document filled with a grid of rectangles in 96 colors.
fn varied_art() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 120, "height": 80})).unwrap();
    for row in 0..8 {
        for col in 0..12 {
            s.execute("shape.rectangle", &json!({"x": col * 10, "y": row * 10, "width": 10, "height": 10})).unwrap();
            let color = format!("#{:02x}{:02x}{:02x}", col * 21, row * 32, (col * 7 + row * 23) % 256);
            s.execute("paint.setFill", &json!({"color": color})).unwrap();
        }
    }
    s
}

#[test]
fn text_export_joins_lines_of_point_type_into_paragraphs() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 500})).unwrap();
    let line = |s: &mut Session, x: f64, y: f64, size: f64, t: &str| {
        s.execute("text.create", &json!({"x": x, "y": y, "size": size, "text": t})).unwrap()["id"].as_u64().unwrap()
    };
    line(&mut s, 10.0, 40.0, 20.0, "Title");
    // Two justified columns, 9 pt on 12 pt: the first runs on into the second, and a short line
    // ends a paragraph. A wrapped line's last space is left out.
    let full = |n: u32| format!("{n} lorem ipsum dolor sit amet consectetur ");
    for (k, t) in [full(1), full(2), "Short end.".into(), full(3), full(4)].iter().enumerate() {
        line(&mut s, 10.0, 100.0 + 12.0 * k as f64, 9.0, t);
    }
    line(&mut s, 300.0, 100.0, 9.0, &full(5));
    line(&mut s, 300.0, 112.0, 9.0, "Last one.");
    // Ragged lines: a line all bold (a heading) is a paragraph of its own, and a line starting
    // bold among lines that don't (a lead-in) starts one.
    for (k, (t, bold)) in [("Notes", 5), ("A: first", 2), ("second", 0), ("B: third", 2), ("fourth", 0)].iter().enumerate() {
        let id = line(&mut s, 10.0, 300.0 + 8.4 * k as f64, 7.0, t);
        if *bold > 0 {
            s.execute("text.setRangeStyle", &json!({"id": id, "start": 0, "end": bold, "style": "Bold"})).unwrap();
        }
    }
    // A story keeps its own paragraphs.
    s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Story one\nStory two", "area": {"width": 200, "height": 60}})).unwrap();

    let export = |s: &mut Session, p: Value| {
        String::from_utf8(b64(&s.execute("document.export", &merge(json!({"format": "txt", "paragraphs": true}), p)).unwrap())).unwrap()
    };
    let full = |n: u32| format!("{n} lorem ipsum dolor sit amet consectetur");
    let want = format!(
        "Title\r\n{}\n{}\nShort end.\r\n{}\n{}\n{}\nLast one.\r\nNotes\r\nA: first\nsecond\r\nB: third\nfourth\r\nStory one\r\nStory two\r\n",
        full(1),
        full(2),
        full(3),
        full(4),
        full(5)
    );
    assert_eq!(export(&mut s, json!({"lineEndings": "crlf"})), want);
    assert_eq!(export(&mut s, json!({"lineEndings": "crlf", "lineBreaks": "space"})), want.replace('\n', " ").replace("\r ", "\r\n"));
    assert_eq!(export(&mut s, json!({"lineBreaks": "crlf"})).lines().count(), want.lines().count(), "every line still on its own");
    assert!(s.execute("document.export", &json!({"format": "txt", "paragraphs": true, "lineBreaks": "tab"})).is_err());
    // Without paragraphs, every line ends alike, as before.
    assert_eq!(String::from_utf8(b64(&s.execute("document.export", &json!({"format": "txt"})).unwrap())).unwrap().matches('\n').count(), 15);
}

/// A paragraph's lines don't run on into lines on another artboard (a PDF's next pages), nor into
/// a table's column far to its right, though their baselines line up and are spaced alike; and a
/// bold heading in the body's font and size, a little further up and to the left, doesn't cut the
/// first line off from the rest.
#[test]
fn paragraphs_stay_on_their_artboard_and_next_to_their_column() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 500})).unwrap();
    s.execute("artboard.new", &json!({"x": 650, "y": 0, "width": 600, "height": 500})).unwrap();
    let line =
        |s: &mut Session, x: f64, y: f64, t: &str| s.execute("text.create", &json!({"x": x, "y": y, "size": 8, "text": t})).unwrap()["id"].clone();
    let heading = line(&mut s, 302.0, 88.8, "Verkaufs- und Lieferbedingungen");
    s.execute("text.setRangeStyle", &json!({"id": heading, "start": 0, "end": 31, "style": "Bold"})).unwrap();
    line(&mut s, 300.0, 100.0, "Es gelten unsere Allgemeinen Verkaufs- und Lieferbedingungen, die zum");
    line(&mut s, 300.0, 108.5, "Zeitpunkt der Bestellung Gültigkeit haben.");
    // A table's column on the next artboard, and one far to the right on this one.
    for x in [700.0, 30.0] {
        line(&mut s, x, 100.0, "Stück");
        line(&mut s, x, 108.5, "Stück");
    }
    let got =
        String::from_utf8(b64(&s.execute("document.export", &json!({"format": "txt", "paragraphs": true, "lineBreaks": "space"})).unwrap())).unwrap();
    assert!(
        got.contains(
            "Verkaufs- und Lieferbedingungen\nEs gelten unsere Allgemeinen Verkaufs- und Lieferbedingungen, die zum Zeitpunkt der Bestellung Gültigkeit haben.\n"
        ),
        "{got}"
    );
    assert_eq!(got.matches("Stück Stück\n").count(), 2, "{got}");

    // Ragged lines that mostly run nearly as long as the longest aren't justified: a shorter line
    // that ends no sentence goes on.
    let ragged = [
        "Verpackung und Bearbeitung 15,- € netto. Bestellungen von Lagerartikeln,",
        "die an einem Arbeitstag bis 14.00 Uhr eingehen, werden im Laufe des",
        "nächsten Arbeitstages ausgeliefert. Ist eine Belieferung bis 09.00 Uhr am",
        "Folgetag gewünscht, ist diese als „Express-Lieferung“ (max. 30 kg) gegen",
        "eine Kostenpauschale in Höhe 40,- € netto möglich. Bei schwereren Teilen",
        "werden die tatsächlich anfallenden Porto- und Verpackungskosten",
        "berechnet.",
    ];
    for (k, t) in ragged.iter().enumerate() {
        line(&mut s, 30.0, 300.0 + 8.5 * k as f64, t);
    }
    let got =
        String::from_utf8(b64(&s.execute("document.export", &json!({"format": "txt", "paragraphs": true, "lineBreaks": "space"})).unwrap())).unwrap();
    assert!(got.contains(&format!("{}\n", ragged.join(" "))), "{got}");
}

#[test]
fn paragraphs_with_ids_come_back_edited() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 500})).unwrap();
    let line = |s: &mut Session, x: f64, y: f64, t: &str| {
        s.execute("text.create", &json!({"x": x, "y": y, "size": 9, "text": t})).unwrap()["id"].as_u64().unwrap()
    };
    let title = s.execute("text.create", &json!({"x": 10, "y": 40, "size": 20, "text": "Title"})).unwrap()["id"].as_u64().unwrap();
    // A justified paragraph that runs on from one column into the next.
    let full = |n: u32| format!("{n} lorem ipsum dolor sit amet consectetur");
    let mut lines = vec![];
    for (k, t) in [full(1), full(2), full(3), full(4)].iter().enumerate() {
        lines.push(line(&mut s, 10.0, 100.0 + 12.0 * k as f64, t));
    }
    lines.push(line(&mut s, 300.0, 100.0, &full(5)));
    lines.push(line(&mut s, 300.0, 112.0, "Last one."));
    let story = s.execute("text.create", &json!({"x": 10, "y": 400, "text": "Story one\nStory two", "area": {"width": 200, "height": 60}})).unwrap()
        ["id"]
        .as_u64()
        .unwrap();

    let export =
        |s: &mut Session, p: Value| String::from_utf8(b64(&s.execute("document.export", &merge(json!({"format": "txt"}), p)).unwrap())).unwrap();
    let with_ids = export(&mut s, json!({"ids": true, "lineBreaks": "space"}));
    let first = lines[0];
    assert_eq!(
        with_ids,
        format!(
            "[#{title}] Title\n[#{first}] {} {} {} {} {} Last one.\n[#{story}.1] Story one\n[#{story}.2] Story two\n",
            full(1),
            full(2),
            full(3),
            full(4),
            full(5)
        )
    );

    // Translated: the paragraph, a story's second paragraph; the title as it was; an id it hasn't.
    let new = "Ein viel längerer Absatz, der sich jetzt in den Rahmen umbricht und weiterläuft, wohin auch immer er will.";
    let file = format!("[#{title}] Title\r\n[#{first}] {new}\r\n[#{story}.2] Geschichte\nzwei\r\n[#999999] nowhere\r\n");
    let r = s.execute("text.importParagraphs", &json!({ "data": file })).unwrap();
    assert_eq!((r["replaced"].as_u64(), r["unchanged"].as_u64(), r["missing"].clone()), (Some(2), Some(1), json!(["999999"])), "{r}");
    // One frame per column, threaded; the first keeps the first line's id, and the lines are gone.
    let threads = s.execute("text.thread.info", &json!({})).unwrap();
    let thread: Vec<u64> = threads[0].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
    assert_eq!((thread.len(), thread[0]), (2, first), "{threads}");
    let doc = s.doc().unwrap().doc.clone();
    assert!(lines[1..].iter().all(|id| thread.contains(id) || doc.node(vectorcraft_doc::NodeId(*id)).is_none()));
    let Some(vectorcraft_doc::NodeKind::Text(t)) = doc.node(vectorcraft_doc::NodeId(first)).map(|n| &n.kind) else { panic!("no frame") };
    assert!(matches!(t.kind, vectorcraft_doc::TextKind::Area { .. }));
    assert_eq!(t.para.justify, vectorcraft_doc::Justify::JustifyLeft, "justified as the lines were");
    let plain = export(&mut s, json!({}));
    assert_eq!(plain, format!("Title\n{new}\nStory one\nGeschichte\u{2028}zwei\n"));
    // The same file again changes nothing: the frames answer to the paragraph's id.
    let again = s.execute("text.importParagraphs", &json!({ "data": file })).unwrap();
    assert_eq!((again["replaced"].as_u64(), again["unchanged"].as_u64()), (Some(0), Some(3)), "{again}");
    // One undo step puts the lines back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(export(&mut s, json!({"ids": true, "lineBreaks": "space"})), with_ids);
    assert!(s.execute("text.importParagraphs", &json!({"data": "no ids"})).is_err());
}

#[test]
fn webp_is_lossless_by_default_and_lossy_when_asked() {
    let mut s = varied_art();
    let export = |s: &mut Session, p: Value| -> (Vec<u8>, Value) {
        let r = s.execute("document.export", &merge(json!({"format": "webp", "background": "white"}), p)).unwrap();
        (b64(&r), r["warnings"].clone())
    };
    let (lossless, warnings) = export(&mut s, json!({}));
    assert_eq!(webp_chunks(&lossless), [*b"VP8L"], "lossless by default");
    assert_eq!(warnings, json!([]));
    // Quality doesn't change a lossless file.
    assert_eq!(export(&mut s, json!({"lossless": true, "quality": 10})).0, lossless);

    let (lossy, warnings) = export(&mut s, json!({"lossless": false}));
    assert_eq!(webp_chunks(&lossy), [*b"VP8 "], "opaque art is a plain lossy file");
    assert_eq!(warnings, json!([]), "no note: the file is lossy");
    assert_eq!(export(&mut s, json!({"lossless": false, "quality": 90})).0, lossy, "quality 90 by default");
    // At quality 90 each channel stays within 4 levels of the exact pixels on average.
    let ((w, h, exact), (lw, lh, near)) = (rgba(&lossless), rgba(&lossy));
    assert_eq!((lw, lh), (w, h));
    assert_eq!((w, h), (120, 80));
    let diff: u64 = exact.iter().zip(&near).map(|(a, b)| u64::from(a.abs_diff(*b))).sum();
    let mean = diff as f64 / exact.len() as f64;
    assert!(mean <= 4.0, "mean difference {mean}");
    // Lower quality, smaller file.
    let low = export(&mut s, json!({"lossless": false, "quality": 10})).0;
    let high = export(&mut s, json!({"lossless": false, "quality": 95})).0;
    assert_eq!(webp_chunks(&low), [*b"VP8 "]);
    assert!(low.len() < high.len(), "{} vs {} bytes", low.len(), high.len());
    for q in [0, 100] {
        assert_eq!(webp_chunks(&export(&mut s, json!({"lossless": false, "quality": q})).0), [*b"VP8 "], "quality {q}");
    }
}

#[test]
fn lossy_webp_keeps_translucent_pixels() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 30, "height": 20})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    let file = |s: &mut Session, lossless: bool| b64(&s.execute("document.export", &json!({"format": "webp", "lossless": lossless})).unwrap());
    let (lossless, lossy) = (file(&mut s, true), file(&mut s, false));
    assert_eq!(webp_chunks(&lossy), [*b"VP8X", *b"ALPH", *b"VP8 "]);
    let ((w, h, exact), (lw, lh, near)) = (rgba(&lossless), rgba(&lossy));
    assert_eq!((lw, lh), (w, h));
    let at = |px: &[u8], x: u32, y: u32| -> [u8; 4] {
        let i = 4 * (y * w + x) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    };
    // The alpha is written exactly: half covered under the rectangle, clear past it.
    let alphas = |px: &[u8]| px.chunks(4).map(|p| p[3]).collect::<Vec<u8>>();
    assert_eq!(alphas(&near), alphas(&exact));
    assert!((120..=135).contains(&at(&near, 5, 5)[3]), "{:?}", at(&near, 5, 5));
    assert_eq!(at(&near, 28, 10)[3], 0);
}

#[test]
fn webp_quality_out_of_its_range_is_refused() {
    let mut s = varied_art();
    for q in [json!(101), json!(255), json!(256), json!(-1), json!(60.5), json!("high"), json!(true)] {
        let p = json!({"format": "webp", "lossless": false, "quality": q});
        let e = s.execute("document.export", &p).unwrap_err().to_string();
        assert!(e.contains("quality"), "{q}: {e}");
        // In Export for Screens too, in a row or in the format's settings.
        let row = json!({"formats": [{"format": "webp", "lossless": false, "quality": q}]});
        assert!(s.execute("document.exportForScreens", &row).is_err(), "{q}");
        let settings = json!({"formats": [{"format": "webp"}], "settings": {"webp": {"lossless": false, "quality": q}}});
        assert!(s.execute("document.exportForScreens", &settings).is_err(), "{q}");
    }
    // A lossless file refuses it as well: the value is not a WebP quality.
    assert!(s.execute("document.export", &json!({"format": "webp", "quality": 101})).is_err());
    let r = s.execute("document.exportForScreens", &json!({"formats": [{"format": "webp", "lossless": false, "quality": 100}]})).unwrap();
    let file = b64(&r["files"][0]);
    assert_eq!(webp_chunks(&file), [*b"VP8 "]);
}

#[test]
fn save_for_office_writes_a_png_at_the_resolution() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 612, "height": 792, "artboards": 2})).unwrap();
    let r = s.execute("document.exportForOffice", &json!({"ppi": 150})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(1275), Some(1650)), "Letter at 150 ppi");
    let img = craft_image_codecs::decode(&b64(&r))
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
    assert_eq!(img.dimensions(), (1275, 1650));
    assert_eq!(img.get_pixel(0, 0).0, [255, 255, 255, 255], "on white");
    let r = s.execute("document.exportForOffice", &json!({"ppi": 72, "transparent": true, "artboard": 1})).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(612), Some(792)));
    assert_eq!(
        craft_image_codecs::decode(&b64(&r))
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
            .to_rgba8()
            .get_pixel(0, 0)[3],
        0,
        "transparent"
    );
    assert!(s.execute("document.exportForOffice", &json!({"artboard": 2})).is_err());
    assert!(s.execute("document.exportForOffice", &json!({"ppi": -3})).is_err());
    let spec = crate::cmd::find_command("document.exportForOffice").unwrap();
    assert_eq!((spec.label, spec.menu), ("Save for Office Documents…", &["File"][..]));
}

#[test]
fn save_for_office_says_when_an_image_or_a_font_is_missing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    assert_eq!(s.execute("document.exportForOffice", &json!({"ppi": 72})).unwrap()["warnings"], json!([]));
    // The renderer can't decode the placed image, and the type's family isn't installed.
    crate::tests_menucmds::place_undecodable_png(&mut s);
    s.execute("text.create", &json!({"x": 5, "y": 20, "text": "Ab", "font": "Nonexistent Sans Probe"})).unwrap();
    let r = s.execute("document.exportForOffice", &json!({"ppi": 72})).unwrap();
    let warnings: Vec<&str> = r["warnings"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(warnings.contains(&"an image couldn't be decoded and is missing from the picture"), "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("fallback font") && w.contains("Nonexistent Sans Probe")), "{warnings:?}");
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(200), Some(100)), "the PNG is still written");
}
