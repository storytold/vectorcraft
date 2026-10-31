//! EPS and PostScript files opened and placed: our own EPS files restore the document they carry,
//! others are read by the PostScript interpreter or come in as their preview.

use std::io::Write as _;

use serde_json::{Value, json};

use super::tests_svgedit::{comparable, rich};
use super::*;

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap_or_else(|| panic!("no dataBase64 in {v}"))).unwrap()
}

fn open(s: &mut Session, name: &str, bytes: &[u8]) -> Result<Value> {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}))
}

/// `bytes` with `from` replaced by `to` (the same length: the preview header's offsets stay).
fn patched(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    assert_eq!(from.len(), to.len());
    let at = bytes.windows(from.len()).position(|w| w == from).unwrap_or_else(|| panic!("no {:?}", String::from_utf8_lossy(from)));
    let mut out = bytes.to_vec();
    out[at..at + to.len()].copy_from_slice(to);
    out
}

/// An EPS file of ours as another app would write it: without the document it carries.
fn foreign(eps: &[u8]) -> Vec<u8> {
    patched(eps, b"%VectorCraft_BeginData: native", b"%VectorCraft_BeginData: nativx")
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

#[test]
fn an_eps_file_of_ours_reopens_as_the_same_document() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    let eps = b64(&s.execute("document.exportEps", &json!({})).unwrap());
    let r = open(&mut s, "rich.eps", &eps).unwrap();
    assert_eq!((&r["warnings"], &r["restored"], &r["format"]), (&json!([]), &json!(true), &json!("eps")), "{r}");
    assert_eq!(comparable(&s.doc().unwrap().doc), before, "the document came back exactly");
    assert_eq!(s.doc().unwrap().path, None, "an EPS file is not saved over");
    // A document that can't be read: the PostScript is read instead, with a warning.
    let at = eps.windows(9).position(|w| w == b"native\n% ").unwrap() + 12;
    let mut damaged = eps.clone();
    damaged[at] = b'z';
    let r = open(&mut s, "damaged.eps", &damaged).unwrap();
    assert_eq!((&r["warnings"][0], &r["restored"]), (&json!(load::EDITING_DAMAGED), &json!(false)), "{r}");
    assert!(objects(&s) > 0);
}

#[test]
fn other_eps_files_are_read_by_the_postscript_interpreter() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 20, "y": 30, "width": 100, "height": 50})).unwrap();
    let eps = foreign(&b64(&s.execute("document.exportEps", &json!({"previewFormat": "none", "cmykPostScript": false})).unwrap()));
    let r = open(&mut s, "plain.eps", &eps).unwrap();
    assert_eq!((&r["restored"], &r["format"]), (&json!(false), &json!("eps")), "{r}");
    let d = &s.doc().unwrap().doc;
    // The page is the bounding box: the art's bounds.
    assert_eq!(d.artboards[0].rect.size(), vectorcraft_geom::Size::new(101.0, 51.0), "the stroke's half width rounds up to whole points");
    assert_eq!(objects(&s), 1);
    assert_eq!(s.doc().unwrap().title(), "plain.eps");
}

/// A CMYK image the PostScript interpreter reads keeps its ink amounts when the EPS opens and when
/// it is placed, with its transparent pixel as its opacity mask.
#[test]
fn cmyk_images_keep_their_inks_when_an_eps_opens_or_is_placed() {
    let eps = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 60 20\n%%EndComments\n/DeviceCMYK setcolorspace 60 20 scale \
        << /ImageType 4 /Width 2 /Height 1 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [2 0 0 -1 0 1] \
        /MaskColor [10 20 30 40] /DataSource <0a141e28 00667f08> >> image\nshowpage\n%%EOF\n";
    // Each image object's ink amounts (`None` when it isn't CMYK) and whether it has a mask.
    let images = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let mut found = vec![];
        d.walk(|n| {
            if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
                found.push((d.images[&im.key].cmyk().map(|i| i.data), n.extras().mask.is_some()));
            }
        });
        found
    };
    let want = (Some(vec![10, 20, 30, 40, 0, 102, 127, 8]), true);
    let mut s = Session::new();
    let r = open(&mut s, "cmyk.eps", eps).unwrap();
    assert_eq!(r["format"], "eps", "{r}");
    assert_eq!(images(&s), std::slice::from_ref(&want));
    let r = s.execute("file.place", &json!({"name": "cmyk.eps", "dataBase64": vectorcraft_format::base64_encode(eps)})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(images(&s), [want.clone(), want]);
}

/// A 4-bit gray image compressed with Flate and TIFF predictor 2 draws the samples it had before
/// differencing, in an EPS and in a PostScript `.ai`.
#[test]
fn tiff_predicted_4_bit_images_open_from_eps_and_postscript_ai_files() {
    // Two rows of four samples, 0 5 15 3 and 9 9 2 0, differenced to 0 5 10 4 and 9 0 9 14.
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&[0x05, 0xa4, 0x90, 0x9e]).unwrap();
    let hex: String = z.finish().unwrap().iter().map(|b| format!("{b:02x}")).collect();
    let ps = format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 40 20\n%%EndComments\n40 20 scale 4 2 4 [4 0 0 -2 0 2] currentfile /ASCIIHexDecode filter \
         << /Predictor 2 /Colors 1 /BitsPerComponent 4 /Columns 4 >> /FlateDecode filter image\n{hex}>\nshowpage\n%%EOF\n"
    );
    for name in ["predicted.eps", "predicted.ai"] {
        let mut s = Session::new();
        let r = open(&mut s, name, ps.as_bytes()).unwrap();
        assert_eq!(r["warnings"], json!([]), "{name}: {r}");
        let d = &s.doc().unwrap().doc;
        let mut grays = vec![];
        d.walk(|n| {
            if let vectorcraft_doc::NodeKind::Image(im) = &n.kind {
                grays = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_luma8().into_raw();
            }
        });
        assert_eq!(grays, [0, 5, 15, 3, 9, 9, 2, 0].map(|v| v * 17), "{name}");
    }
}

#[test]
fn an_unreadable_eps_file_comes_in_as_its_preview() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    s.execute("shape.ellipse", &json!({"x": 20, "y": 30, "width": 100, "height": 50})).unwrap();
    let eps = foreign(&b64(&s.execute("document.exportEps", &json!({"previewFormat": "tiffColor"})).unwrap()));
    // An operator the interpreter doesn't know (another app's own).
    let eps = patched(&eps, b"showpage", b"frobnica");
    let r = open(&mut s, "art.eps", &eps).unwrap();
    let w = r["warnings"][0].as_str().unwrap();
    assert!(w.contains("frobnica") && w.contains("preview"), "{r}");
    let d = &s.doc().unwrap().doc;
    let layer = &d.layers[0];
    let image = &layer.children().unwrap()[0];
    assert!(matches!(image.kind, vectorcraft_doc::NodeKind::Image(_)), "{:?}", image.kind);
    assert_eq!(image.geometric_bounds().unwrap(), d.artboards[0].rect);
}

#[test]
fn postscript_ai_files_and_garbage() {
    let mut s = Session::new();
    let ps = b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n1 0 0 setrgbcolor 10 10 50 50 rectfill\nshowpage\n%%EOF\n";
    // A PostScript .ai opens as artwork; Save asks where (the file isn't written back).
    let r = open(&mut s, "legacy.ai", ps).unwrap();
    assert_eq!((&r["format"], &r["restored"]), (&json!("ai"), &json!(false)), "{r}");
    assert_eq!(s.doc().unwrap().path, None);
    assert_eq!(objects(&s), 1);
    // Illustrator's groups come in as groups, nested as they were.
    let ai = b"%!PS-Adobe-3.0
%%Creator: Adobe Illustrator(R) 8.0
%%BoundingBox: 0 0 100 100
%%EndComments
               /u {} def /U {} def /L {lineto} def /f {closepath fill} def
               u 0 0 moveto 9 0 L 9 9 L f u 20 0 moveto 29 0 L 29 9 L f 40 0 moveto 49 0 L 49 9 L f U U
showpage
%%EOF
";
    open(&mut s, "groups.ai", ai).unwrap();
    let d = s.doc().unwrap().doc.clone();
    let outer = &d.layers[0].children().unwrap()[0];
    let inner = outer.children().map(|c| (c.len(), c[1].children().map(Vec::len))).unwrap();
    assert!(matches!(outer.kind, vectorcraft_doc::NodeKind::Group { clip: false, .. }), "{:?}", outer.kind);
    assert_eq!(inner, (2, Some(2)));
    // One that names the prolog defining its operators without including it (as other apps write
    // the format) opens with the layers its program has (#1027).
    let named = b"%!PS-Adobe-3.0\n%%BoundingBox: 0 0 100 100\n%%DocumentNeededResources: procset Adobe_IllustratorA_AI3 1.0 0\n%AI3_TemplateBox: 50 50 50 50\n%%EndComments\n\
                  %%BeginProlog\n%%IncludeResource: procset Adobe_IllustratorA_AI3 1.0 0\n%%EndProlog\n%%BeginSetup\nAdobe_IllustratorA_AI3 /initialize get exec\n%%EndSetup\n\
                  %AI5_BeginLayer\n1 1 1 1 0 0 2 0 0 0 Lb\n(PLAN::Walls) Ln\n0 0 0 1 K\n0 i 1 w\n10 10 m\n90 90 L\nS\nLB\n%AI5_EndLayer--\n%%PageTrailer\n%%Trailer\n%%EOF\n";
    let r = open(&mut s, "cad.ai", named).unwrap();
    assert_eq!(r["warnings"].as_array().map_or(0, Vec::len), 0, "{r}");
    let d = s.doc().unwrap().doc.clone();
    assert_eq!((d.layers.len(), d.layers[0].name.as_deref(), objects(&s)), (1, Some("PLAN::Walls"), 1));
    // A .ait one opens as a new untitled document.
    let r = open(&mut s, "legacy.ait", ps).unwrap();
    assert!(r["title"].as_str().unwrap().starts_with("Untitled"), "{r}");
    // Garbage named .eps says what it is.
    let e = open(&mut s, "junk.eps", b"not postscript at all").unwrap_err().to_string();
    assert!(e.contains("junk.eps") && e.contains("PostScript"), "{e}");
    let e = open(&mut s, "junk.eps", &[0xC5, 0xD0, 0xD3, 0xC6, 1, 2, 3]).unwrap_err().to_string();
    assert!(e.contains("junk.eps"), "{e}");
}

#[test]
fn eps_files_place_as_one_group_clipped_to_their_bounding_box() {
    let mut s = rich();
    let eps = b64(&s.execute("document.exportEps", &json!({"useArtboards": true, "artboards": [0]})).unwrap());
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    let r = s.execute("file.place", &json!({"name": "rich.eps", "dataBase64": vectorcraft_format::base64_encode(&eps), "at": [300, 300]})).unwrap();
    assert_eq!(r["format"], "eps", "{r}");
    assert_eq!(r["ids"].as_array().unwrap().len(), 1);
    // The interpreter's reading of the same file places too.
    let r = s.execute("file.place", &json!({"name": "other.eps", "dataBase64": vectorcraft_format::base64_encode(&foreign(&eps))})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 1, "{r}");
    let info = s.execute("file.place.info", &json!({"name": "other.eps", "dataBase64": vectorcraft_format::base64_encode(&foreign(&eps))})).unwrap();
    assert_eq!(info["format"], "eps");
    // EPS is readable, and listed for the open and place dialogs.
    assert!(format("eps").is_some_and(|f| f.read && f.write));
    assert!(OPEN_EXTS.contains(&"eps") && PLACE_EXTS.contains(&"eps"));
}

/// PostScript `.ps` files open and place as EPS files do, under a PostScript format that exports
/// don't write, and `.epsf` files are EPS files; the open and place dialogs list both.
#[test]
fn ps_and_epsf_files_open_and_place() {
    use crate::cmd::clipboard::{EPS, PASTE_ORDER, file_flavour};
    // Pages without a bounding box (each artboard is a letter page), one of them a program whose
    // header is only `%!`.
    let page = b"%!PS-Adobe-3.0\n%%Pages: 1\n%%EndComments\n%%Page: 1 1\n1 0 0 setrgbcolor\n\
                 newpath 72 72 moveto 144 0 rlineto 0 72 rlineto -144 0 rlineto closepath fill\nshowpage\n%%EOF\n";
    let bare = b"%!\n0 0 1 setrgbcolor\n100 100 50 50 rectfill\nshowpage\n";
    let mut s = Session::new();
    for (name, bytes, id) in [("page.ps", &page[..], "ps"), ("PAGE.PS", page, "ps"), ("bare.ps", bare, "ps"), ("bare.epsf", bare, "eps")] {
        let r = open(&mut s, name, bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((&r["format"], &r["warnings"]), (&json!(id), &json!([])), "{name}: {r}");
        assert_eq!(objects(&s), 1, "{name}");
        assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(612.0, 792.0), "{name}");
    }
    s.execute("file.new", &json!({"width": 600, "height": 600})).unwrap();
    for (name, bytes, id) in [("page.ps", &page[..], "ps"), ("bare.ps", bare, "ps"), ("bare.epsf", bare, "eps")] {
        let r = s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)})).unwrap();
        assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!(id), Some(1)), "{name}: {r}");
    }
    // Copied in a file manager, they paste as EPS.
    for (name, bytes) in [("page.ps", &page[..]), ("bare.ps", bare), ("bare.epsf", bare), ("bare.eps", bare)] {
        assert_eq!(file_flavour(name, bytes, &PASTE_ORDER), Some(EPS), "{name}");
        let r = s
            .execute("clipboard.importEps", &json!({"dataBase64": vectorcraft_format::base64_encode(bytes)}))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(r["count"], json!(1), "{name}: {r}");
    }
    // The dialogs list them, and the desktop packaging doesn't associate them with the app.
    assert!(format("ps").is_some_and(|f| f.read && !f.write) && format("epsf").is_some_and(|f| f.id == "eps" && f.write));
    for e in ["ps", "epsf"] {
        assert!(OPEN_EXTS.contains(&e) && PLACE_EXTS.contains(&e) && UNASSOCIATED_EXTS.contains(&e), "{e}");
    }
    assert!(open_filters().any(|f| f == ("PostScript", &["ps"][..])) && place_filters().any(|f| f == ("PostScript", &["ps"][..])));
    // Exports write no .ps: Print to PostScript does.
    let e = writable_format(None, Some("out.ps")).unwrap_err();
    assert_eq!(e, "PostScript files can be opened but not written (see document.formats)");
}
