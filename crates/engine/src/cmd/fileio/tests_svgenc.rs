//! SVG encodings, profiles and embedded fonts through the engine: the options reach the file,
//! files in each encoding open again, and `document.serialize` reads them as text.

use serde_json::json;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Grüße €"})).unwrap();
    s
}

fn serialize(s: &mut Session, svg: serde_json::Value) -> serde_json::Value {
    s.execute("document.serialize", &json!({"format": "svg", "svg": svg})).unwrap()
}

#[test]
fn each_encoding_reopens_and_serializes_as_text() {
    let mut s = session();
    for (encoding, name, bom) in [("utf8", "UTF-8", false), ("utf16", "UTF-16", true), ("latin1", "ISO-8859-1", false)] {
        let r = serialize(&mut s, json!({"encoding": encoding}));
        let text = r["text"].as_str().unwrap();
        assert!(text.contains("Grüße") && text.contains(&format!("encoding=\"{name}\"")), "{text}");
        let bytes = match r.get("dataBase64") {
            Some(b) => vectorcraft_format::base64_decode(b.as_str().unwrap()).unwrap(),
            None => text.as_bytes().to_vec(),
        };
        assert_eq!(bom, bytes.starts_with(&[0xfe, 0xff]), "{encoding}");
        assert_eq!(r.get("dataBase64").is_some(), encoding != "utf8", "{encoding}");
        for (file, data) in [("t.svg", bytes.clone()), ("t.svgz", vectorcraft_svg::compress_bytes(&bytes))] {
            let mut o = Session::new();
            o.execute("document.open", &json!({"name": file, "dataBase64": vectorcraft_format::base64_encode(&data)})).unwrap();
            let plain = o.execute("document.inspect", &json!({})).unwrap().to_string();
            assert!(plain.contains("Grüße €"), "{encoding} {file}: {plain}");
        }
    }
}

#[test]
fn profile_and_embedded_fonts_reach_the_writer() {
    let mut s = session();
    let tiny = serialize(&mut s, json!({"profile": "tiny12", "styling": "css"}));
    let tiny = tiny["text"].as_str().unwrap();
    assert!(tiny.contains("baseProfile=\"tiny\"") && !tiny.contains("<style>"), "{tiny}");
    let fonts = serialize(&mut s, json!({"embedFonts": true}));
    assert!(fonts["text"].as_str().unwrap().contains("@font-face{"));
    assert!(s.execute("document.serialize", &json!({"format": "svg", "svg": {"profile": "svg2"}})).is_err(), "unknown profiles are refused");
    assert!(s.execute("document.serialize", &json!({"format": "svg", "encoding": "ebcdic"})).is_err(), "unknown encodings are refused");
}

/// #550: an SVG of chosen artboards holds the art over each, not every artboard's art; with no
/// artboard named (as Save writes it) the whole document's art is there.
#[test]
fn a_chosen_artboard_s_svg_holds_only_its_own_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let x = |s: &Session, b: usize| s.doc().unwrap().doc.artboards[b].rect.x0;
    let (first, third) = (x(&s, 0), x(&s, 2));
    s.execute("shape.rectangle", &json!({"x": first + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": third + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    // A rectangle has straight sides only; the ellipse is curves.
    let shapes = |svg: &str| (vectorcraft_testkit::svg_shapes(svg), svg.contains(" C") || svg.contains("<ellipse") || svg.contains("<circle"));
    let text = |v: &serde_json::Value| v["text"].as_str().unwrap().to_string();
    let third_only = text(&s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "3"})).unwrap());
    assert_eq!(shapes(&third_only), (1, true), "{third_only}");
    let all = s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "all"})).unwrap();
    let files: Vec<String> = all["files"].as_array().unwrap().iter().map(text).collect();
    assert_eq!(files.iter().map(|f| shapes(f)).collect::<Vec<_>>(), [(1, false), (0, false), (1, true)]);
    // Export for Screens names one artboard at a time the same way.
    let screen = text(&s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap());
    assert_eq!(shapes(&screen), (1, false));
    let whole = text(&s.execute("document.serialize", &json!({"format": "svg"})).unwrap());
    assert_eq!(shapes(&whole).0, 2, "nothing named: the whole document's art");
}

/// Art grouped across artboards (one variant per artboard in one group) splits by artboard too.
#[test]
fn a_chosen_artboard_s_svg_leaves_out_its_neighbours_art_in_a_shared_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 3})).unwrap();
    let x = |s: &Session, b: usize| s.doc().unwrap().doc.artboards[b].rect.x0;
    let (first, third) = (x(&s, 0), x(&s, 2));
    s.execute("shape.rectangle", &json!({"x": first + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("shape.ellipse", &json!({"x": third + 10.0, "y": 10, "width": 30, "height": 30})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let text = |v: &serde_json::Value| v["text"].as_str().unwrap().to_string();
    let third_only = text(&s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "3"})).unwrap());
    assert_eq!(vectorcraft_testkit::svg_shapes(&third_only), 1, "{third_only}");
    assert!(third_only.contains(" C") || third_only.contains("<ellipse") || third_only.contains("<circle"), "{third_only}");
}

/// An object off a chosen artboard whose drop shadow reaches onto it is in that artboard's SVG.
#[test]
fn a_chosen_artboard_s_svg_keeps_an_object_whose_shadow_reaches_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let x0 = s.doc().unwrap().doc.artboards[0].rect.x0;
    let id = s.execute("shape.rectangle", &json!({"x": x0 - 30.0, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let paths = |s: &mut Session| {
        let svg = s.execute("document.serialize", &json!({"format": "svg", "artboard": 0})).unwrap();
        let text = svg["text"].as_str().unwrap().to_string();
        vectorcraft_testkit::svg_shapes(&text)
    };
    assert_eq!(paths(&mut s), 0, "the plain rectangle is off the artboard");
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id], "params": {"x": 20, "y": 0, "blur": 0}})).unwrap();
    assert_eq!(paths(&mut s), 1, "its shadow reaches 10 pt onto the artboard");
}

/// #3544: an SVG of one artboard on its own (an `artboard` or `range`, Export for Screens, an asset)
/// has the artboard's (or asset's) name as `<title>`; Save's SVG and Export for Screens' Full
/// Document keep the document's.
#[test]
fn an_svg_of_one_artboard_is_titled_after_it() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"name": "Icons", "width": 24, "height": 24, "artboards": 2})).unwrap();
    for (index, name) in [(0, "arrow-left"), (1, "arrow-right")] {
        s.execute("artboard.setProps", &json!({"index": index, "name": name})).unwrap();
    }
    let square = s.execute("shape.rectangle", &json!({"x": 4, "y": 4, "width": 16, "height": 16})).unwrap()["id"].clone();
    let title = |svg: &str| svg.lines().find_map(|l| l.trim().strip_prefix("<title>")?.strip_suffix("</title>")).map(str::to_string);
    let serialize = |s: &mut Session, p: serde_json::Value| s.execute("document.serialize", &p).unwrap()["text"].as_str().unwrap().to_string();
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg", "artboard": 1}))).as_deref(), Some("arrow-right"));
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg", "artboard": 0}))).as_deref(), Some("arrow-left"));
    assert_eq!(title(&serialize(&mut s, json!({"format": "svg"}))).as_deref(), Some("Icons"), "Save's SVG");
    let all = s.execute("document.serialize", &json!({"format": "svg", "useArtboards": true, "range": "all"})).unwrap();
    let titles: Vec<_> = all["files"].as_array().unwrap().iter().map(|f| title(f["text"].as_str().unwrap())).collect();
    assert_eq!(titles, [Some("arrow-left".into()), Some("arrow-right".into())]);
    // Export for Screens: each artboard, an asset, and the Full Document.
    let screens = |s: &mut Session, p: serde_json::Value| -> Vec<Option<String>> {
        let r = s.execute("document.exportForScreens", &p).unwrap();
        r["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| title(&String::from_utf8(vectorcraft_format::base64_decode(f["dataBase64"].as_str().unwrap()).unwrap()).unwrap()))
            .collect()
    };
    let svg = json!([{"format": "svg"}]);
    assert_eq!(screens(&mut s, json!({"formats": svg})), [Some("arrow-left".into()), Some("arrow-right".into())]);
    let asset = s.execute("assets.add", &json!({"ids": [square], "name": "Square"})).unwrap()["assets"][0].clone();
    assert_eq!(screens(&mut s, json!({"assets": [asset], "formats": svg})), [Some("Square".into())]);
    assert_eq!(screens(&mut s, json!({"fullDocument": true, "formats": svg})), [Some("Icons".into())]);
}
