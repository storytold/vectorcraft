//! The editing data reader on synthetic data, written from the format as `read` documents it.

use std::sync::Arc;

use kurbo::{Point, Rect};
use vectorcraft_color::gradient::GradientKind;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, Knockout, LayerColor, LineCap, LineJoin, Node, NodeKind};

use super::*;

/// Editing data: header comments (crop marks 0 0 200 100), then `body`.
fn stream(body: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 14.0\n%AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
}

/// One layer named `L` holding `art`.
fn layer(art: &str) -> String {
    format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n0 A\n0 Xw\n{art}\nLB\n%AI5_EndLayer--\n")
}

fn read_ok(body: &str) -> Structure {
    match read(stream(body).as_bytes()) {
        Ok(i) => i,
        Err(e) => panic!("{e}"),
    }
}

/// The children of the first layer.
fn art(doc: &Document) -> Vec<Arc<Node>> {
    doc.layers.first().and_then(|l| l.children()).cloned().unwrap_or_default()
}

fn fill(n: &Node) -> Paint {
    n.appearance.fill().map(|f| f.paint.clone()).unwrap_or_default()
}

fn rect_path(x0: f64, y0: f64, x1: f64, y1: f64) -> String {
    format!("{x0} {y0} m\n{x0} {y1} L\n{x1} {y1} L\n{x1} {y0} L\n{x0} {y0} L\n")
}

const RED: &str = "0 1 1 0 1 0 0 Xa\n";

#[test]
fn layers_and_sublayers() {
    let body = format!(
        "%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(Bottom) Ln\n{RED}{}f\n\
         %AI5_BeginLayer\n0 1 0 0 0 0 0 -1 10 200 30 0 50 0 Lb\n(Inner) Ln\n{}f\nLB\n%AI5_EndLayer--\nLB\n%AI5_EndLayer--\n\
         %AI5_BeginLayer\n1 0 1 1 1 0 1 3 79 79 255 0 70 0 Lb\n(Top) Ln\n{}f\nLB\n%AI5_EndLayer--\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
        rect_path(40.0, 0.0, 50.0, 10.0)
    );
    let doc = read_ok(&body).doc;
    let names: Vec<_> = doc.layers.iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Bottom", "Top"], "bottom first");
    let bottom = &doc.layers[0];
    assert!(bottom.visible && !bottom.locked);
    let kids = bottom.children().unwrap();
    assert_eq!(kids.len(), 2);
    let inner = &kids[1];
    assert_eq!(inner.name.as_deref(), Some("Inner"));
    assert!(!inner.visible && inner.locked);
    let NodeKind::Layer { color, printable, .. } = &inner.kind else { panic!("a sublayer") };
    assert_eq!((*color, *printable), (LayerColor::Custom([10, 200, 30]), false));
    let NodeKind::Layer { color, preview, dim_images, .. } = &doc.layers[1].kind else { panic!() };
    assert_eq!((*color, *preview, *dim_images), (LayerColor::Preset(3), false, Some(70)));
}

#[test]
fn groups_names_hidden_and_locked() {
    let art_ = format!(
        "0 Ae\nu\n{RED}{}f\n%_/ArtDictionary :\n%_/XMLUID : (My_Rect__x23_2) ; (AI10_ArtUID) ,\n%_;\n%_\n\
         1 A\n0 Ae\nu\n0 A\n1 Xw\n{}f\n0 Xw\nU\n%_/ArtDictionary :\n%_(Inner group) /String (AIArtName) ,\n%_;\n9 () XW\n\
         0 A\nU\n%_/ArtDictionary :\n%_/XMLUID : (Outer) ; (AI10_ArtUID) ,\n%_;\n%_\n9 () XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let top = art(&doc);
    assert_eq!(top.len(), 1);
    let outer = &top[0];
    assert_eq!(outer.name.as_deref(), Some("Outer"));
    let kids = outer.children().unwrap();
    assert_eq!(kids[0].name.as_deref(), Some("My Rect #2"));
    let inner = &kids[1];
    assert_eq!(inner.name.as_deref(), Some("Inner group"));
    assert!(inner.locked, "locked when it opened");
    let hidden = &inner.children().unwrap()[0];
    assert!(!hidden.visible);
}

#[test]
fn paths_colours_and_strokes() {
    let art_ = format!(
        "{RED}0 R\n0 0 1 0 0 0 1 XA\n1 J 2 j 3 w 4 M [6 3 ]1 d\n1 XR\n{}b\n\
         0.1 0.2 0.3 0.4 k\n{}f\n0.7 g\n{}F\n0 0.5 1 0 (Ink) 0.4 x\n{}f\n\
         10 50 m\n20 60 30 60 40 50 c\n50 40 60 40 v\n70 60 80 60 y\nS\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
        rect_path(40.0, 0.0, 50.0, 10.0),
        rect_path(60.0, 0.0, 70.0, 10.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    assert_eq!(a.len(), 5);
    // y up from the crop marks' top: (0, 0)-(10, 10) is at y 90-100.
    let NodeKind::Path { path, rule, .. } = &a[0].kind else { panic!() };
    assert_eq!(path.bounds(), Some(Rect::new(0.0, 90.0, 10.0, 100.0)));
    assert!(path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].anchors.len(), 4, "the closing point merges with the first");
    assert_eq!(*rule, vectorcraft_geom::FillRule::EvenOdd);
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.width, st.cap, st.join, st.miter_limit), (3.0, LineCap::Round, LineJoin::Bevel, 4.0));
    assert_eq!(st.paint.color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    let dash = st.dash.clone().unwrap();
    assert_eq!((dash.pattern, dash.offset), (vec![6.0, 3.0], 1.0));
    assert_eq!(fill(&a[1]).color(), Some(Color::cmyk(0.1, 0.2, 0.3, 0.4)));
    assert!(matches!(fill(&a[2]).color(), Some(Color::Gray { k }) if (k - 0.3).abs() < 1e-6));
    assert!(!matches!(&a[2].kind, NodeKind::Path { path, .. } if path.subpaths[0].closed), "F leaves the path open");
    let Paint::Solid { swatch, tint, .. } = fill(&a[3]) else { panic!() };
    assert_eq!(swatch.as_deref(), Some("Ink"));
    assert!((tint - 0.6).abs() < 1e-6);
    let ink = doc.swatch("Ink").unwrap();
    assert!(ink.spot && ink.global);
    // Curves: `c`, `v` (first control at the current point) and `y` (second at the end).
    let NodeKind::Path { path, .. } = &a[4].kind else { panic!() };
    assert_eq!(path.subpaths[0].anchors.len(), 4);
    assert!(a[4].appearance.fill().is_none() && a[4].appearance.stroke().is_some());
}

#[test]
fn compound_paths_and_clipping() {
    let art_ = format!(
        "0 Ae\n*u\n{RED}{}f\n{}f\n*U\n%_/ArtDictionary :\n%_/XMLUID : (Donut) ; (AI10_ArtUID) ,\n%_;\n\
         0 Ae\nq\n{}f\n{}h\nW\nn\nQ\n%_/ArtDictionary :\n%_/XMLUID : (Clipped) ; (AI10_ArtUID) ,\n%_;\n9 () XW\n",
        rect_path(0.0, 0.0, 30.0, 30.0),
        rect_path(10.0, 10.0, 20.0, 20.0),
        rect_path(40.0, 0.0, 80.0, 40.0),
        rect_path(50.0, 10.0, 60.0, 20.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    let NodeKind::Compound { children, .. } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!((children.len(), a[0].name.as_deref()), (2, Some("Donut")));
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    let NodeKind::Group { children, clip: true } = &a[1].kind else { panic!("a clipping group") };
    assert_eq!(a[1].name.as_deref(), Some("Clipped"));
    // The clipping path goes first.
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(matches!(children[1].kind, NodeKind::Path { clipping: false, .. }));
    // A clipping path in a layer clips the layer.
    let doc = read_ok(&layer(&format!("{}f\n{}W\nn\n", rect_path(0.0, 0.0, 9.0, 9.0), rect_path(0.0, 0.0, 5.0, 5.0)))).doc;
    let NodeKind::Layer { clip, children, .. } = &doc.layers[0].kind else { panic!() };
    assert!(*clip && matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
}

#[test]
fn transparency_of_objects_and_groups() {
    let art_ = format!(
        "{RED}1 0.5 0 0 0 Xy\n{}f\n0 Ae\nu\n0 1 0 0 0 Xy\n{}f\nU\n2 0.7 1 1 0 Xy\n0 0 Xd\n6 () XW\n0 1 0 0 0 Xy\n{}f\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
        rect_path(40.0, 0.0, 50.0, 10.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    assert_eq!((a[0].opacity, a[0].blend), (0.5, BlendMode::Multiply));
    assert_eq!((a[1].opacity, a[1].blend, a[1].isolate, a[1].knockout), (0.7, BlendMode::Screen, true, Knockout::On));
    let inner = &a[1].children().unwrap()[0];
    assert_eq!(inner.opacity, 1.0);
    assert_eq!((a[2].opacity, a[2].blend), (1.0, BlendMode::Normal), "the next object's state isn't the group's");
}

#[test]
fn gradients() {
    let defs = "%AI5_BeginGradient: (G)\n(G) 1 2 Bd\n[\n<00FF>\n0\n0 0 0 0 1 0 0 2 1 6 %_BS\n\
                %_0 0 0 0 1 0 0 2 1 6 40 0 Bs\n%_0 0 0 0 0 0 1 2 0.5 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!(
        "{RED}{}Bb\n0 0 0 0 Bh\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n50 0 0 -50 100 50 Bm\nf\n0 BB\n{}f\n",
        rect_path(50.0, 0.0, 150.0, 100.0),
        rect_path(0.0, 0.0, 10.0, 10.0),
    );
    let doc = read_ok(&format!("{defs}{}", layer(&art_))).doc;
    let a = art(&doc);
    let Paint::Gradient(g) = fill(&a[0]) else { panic!("{:?}", fill(&a[0])) };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert_eq!(g.gradient.stops.len(), 2);
    assert!((g.gradient.stops[0].midpoint - 0.4).abs() < 1e-6);
    assert_eq!(g.gradient.stops[1].opacity, 0.5);
    let geom = g.geom.unwrap();
    assert_eq!((geom.start, geom.end), (Point::new(100.0, 50.0), Point::new(150.0, 50.0)));
    // The gradient was the one object's.
    assert_eq!(fill(&a[1]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn artboards() {
    let data = "%_/Document :\n%_/Dictionary :\n%_/Array :\n\
                %_/Dictionary :\n%_0 200 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_300 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_(One) /UnicodeString (Name) ,\n%_; ,\n\
                %_/Dictionary :\n%_400 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_600 -150 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
                %_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n";
    let art_ = rect_path(420.0, -70.0, 470.0, -20.0) + "f\n";
    let doc = read_ok(&format!("{data}{}", layer(&art_))).doc;
    let boards: Vec<_> = doc.artboards.iter().map(|a| (a.name.clone(), a.rect)).collect();
    assert_eq!(boards, [("One".to_string(), Rect::new(0.0, 0.0, 300.0, 200.0)), ("Artboard 2".to_string(), Rect::new(400.0, 200.0, 600.0, 350.0))]);
    let NodeKind::Path { path, .. } = &art(&doc)[0].kind else { panic!() };
    assert_eq!(path.bounds(), Some(Rect::new(420.0, 220.0, 470.0, 270.0)));
    // Without artboards or crop marks: the art size around the template box's centre.
    let old = format!(
        "%!PS-Adobe-3.0\n%AI5_ArtSize: 600 400\n%AI3_TemplateBox: 300.5 199.5 300.5 199.5\n%%EndComments\n{}",
        layer("10 10 m\n20 20 L\nS\n")
    );
    let doc = read(old.as_bytes()).unwrap().doc;
    assert_eq!(doc.artboards[0].rect, Rect::new(0.0, 0.0, 600.0, 400.0));
}

#[test]
fn a_drawn_look_stands_for_its_object() {
    // The look (a group, with a part on `%_` lines for newer readers), then the object on `%_`
    // lines and its style.
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!(
        "0 Ae\nu\n%_0 Ae\n%_u\n%_{hidden_rect}S\n%_U\n{RED}{}f\n0 0 0 0 0 0 1 XA\n{}S\nU\n\
         %_/ArtDictionary :\n%_/XMLUID : (Neon_00000092429200007574234970000005974767392385616558_) ; (AI10_ArtUID) ,\n%_;\n\
         0 0.4 0 0 0 Xy\n0 0 Xd\n6 () XW\n%_{hidden_rect}n\n%_/ArtDictionary :\n%_/XMLUID : (Neon) ; (AI10_ArtUID) ,\n%_;\n1 (Neon style) XW\n\
         0 1 0 0 0 Xy\n{}f\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(30.0, 0.0, 40.0, 10.0),
    );
    let r = read_ok(&layer(&art_));
    let a = art(&r.doc);
    assert_eq!(a.len(), 2, "{a:#?}");
    assert_eq!(a[0].name.as_deref(), Some("Neon"));
    assert_eq!(a[0].opacity, 0.4);
    assert_eq!(a[0].children().unwrap().len(), 2, "the newer readers' part is left out");
    assert!(r.warnings.iter().any(|w| w.contains("drawn look (1)")), "{:?}", r.warnings);
}

/// Editing data with one layer holding an image: `matrix` (`[a b c d tx ty]`), its size, colour
/// space, kind and alpha channels, its samples, and the size the data comment declares.
fn image(matrix: &str, (w, h): (u32, u32), (space, kind, alpha): (&str, u32, u32), data: &[u8], declared: usize) -> Vec<u8> {
    let mut s = format!(
        "%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/{space} XN\n{matrix} {w} {h} 0 Xh\n\
         {matrix} 0 0 {w} {h} {w} {h} 8 {kind} {alpha} 0 1 0 4 4 0 0\n%%BeginData: {declared}\rXI\n"
    )
    .into_bytes();
    s.extend_from_slice(data);
    s.extend_from_slice(b"%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\r\nLB\r\n%AI5_EndLayer--\r\n");
    let mut out = stream("").into_bytes();
    let at = out.windows(14).position(|w| w == b"%%PageTrailer\n").unwrap();
    out.splice(at..at, s);
    out
}

fn pixels(doc: &Document) -> (image::RgbaImage, kurbo::Affine) {
    let a = art(doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let blob = doc.images.get(&im.key).unwrap();
    (image::load_from_memory(&blob.bytes).unwrap().to_rgba8(), im.xf)
}

#[test]
fn images_with_alpha_channels() {
    // RGB samples, then the alpha channel (not counted in the data's size).
    let data = [255, 0, 0, 0, 255, 0, 255, 128];
    let doc = read(&image("[ 10 0 0 10 50 90 ]", (2, 1), ("DeviceRGB", 3, 1), &data, 10)).unwrap().doc;
    let (img, xf) = pixels(&doc);
    assert_eq!((img.get_pixel(0, 0).0, img.get_pixel(1, 0).0), ([255, 0, 0, 255], [0, 255, 0, 128]));
    // Pixel space (y down) to the top left at (50, 90) in art space: (50, 10) on the page.
    assert_eq!(xf * Point::ZERO, Point::new(50.0, 10.0));
    assert_eq!(xf * Point::new(2.0, 1.0), Point::new(70.0, 20.0));
    // CMYK and grey.
    let doc = read(&image("[ 1 0 0 1 0 100 ]", (1, 1), ("DeviceCMYK", 4, 0), &[0, 0, 0, 255], 6)).unwrap().doc;
    let px = pixels(&doc).0.get_pixel(0, 0).0;
    assert!(px[0] < 64 && px[3] == 255, "100% black is dark: {px:?}");
    let doc = read(&image("[ 1 0 0 1 0 100 ]", (2, 1), ("DeviceGray", 1, 0), &[0, 200], 6)).unwrap().doc;
    assert_eq!(pixels(&doc).0.get_pixel(1, 0).0, [200, 200, 200, 255]);
    // Data cut short: the image goes, with a warning.
    let r = read(&image("[ 1 0 0 1 0 100 ]", (4, 4), ("DeviceRGB", 3, 0), &[1, 2, 3], 6)).unwrap();
    assert!(art(&r.doc).is_empty() && r.warnings.iter().any(|w| w.contains("cut short")));
}

#[test]
fn text_objects_are_slots_named_after_their_story() {
    let words = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n3 /StoryIndex ,\n;\n%_/ArtDictionary :\n%_/XMLUID : (Title) ; (AI10_ArtUID) ,\n%_;\n";
    let r = read_ok(&layer(&format!("1 Xw\n{words}0 Xw\n")));
    let a = art(&r.doc);
    assert_eq!(slot_of(a[0].name.as_deref()), Some(Some(3)));
    assert!(!a[0].visible);
    assert_eq!(r.slot_names.get(&a[0].id).map(String::as_str), Some("Title"));
    // A copy on `%_` lines of a story that has a plain text object goes; one without stays.
    let commented = |story: u32| -> String { format!("/AI11Text :\n{story} /StoryIndex ,\n;\n").lines().map(|l| format!("%_{l}\n")).collect() };
    let r = read_ok(&layer(&format!("/AI11Text :\n0 /StoryIndex ,\n;\n{}{}", commented(0), commented(1))));
    let stories: Vec<_> = art(&r.doc).iter().map(|n| slot_of(n.name.as_deref())).collect();
    assert_eq!(stories, [Some(Some(0)), Some(Some(1))]);
}

#[test]
fn what_it_doesnt_read_is_an_error_where_it_shows() {
    let e = read(stream(&layer("1 2 frobnicate\n")).as_bytes()).unwrap_err();
    assert!(e.contains("`frobnicate`"), "{e}");
    let e = read(stream(&layer("(Jive) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n")).as_bytes()).unwrap_err();
    assert!(e.contains("pattern fills"), "{e}");
    let e = read(stream(&layer("/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n")).as_bytes()).unwrap_err();
    assert!(e.contains("symbols"), "{e}");
    // On a layer that doesn't show, it is left out of it.
    let hidden = "%AI5_BeginLayer\n0 1 1 1 0 0 0 1 255 79 79 0 50 0 Lb\n(Off) Ln\n1 2 frobnicate\n(J) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\nLB\n%AI5_EndLayer--\n";
    let r = read_ok(hidden);
    assert_eq!(r.hidden_unread.iter().cloned().collect::<Vec<_>>(), ["`frobnicate`", "pattern fills"]);
    // Unbalanced containers, a layer that doesn't end, no layers.
    assert!(read(stream(&layer("U\n")).as_bytes()).is_err());
    assert!(read(stream("%AI5_BeginLayer\n(L) Ln\n0 Ae\nu\n").as_bytes()).is_err());
    assert!(read(stream("").as_bytes()).is_err());
    // The setup's procedures and what follows the page don't count.
    read_ok(&format!("Adobe_level2_AI5 /initialize get exec\n{}", layer("")));
}

#[test]
fn sections_it_skips() {
    let body = "%AI5_Begin_NonPrinting\n%AI14_BeginSymbol\n0 Ae\nu\nfrobnicate\n%AI10_EndSymbol\n%AI3_BeginPattern: (P)\nU\nU\n%AI3_EndPattern\n%AI5_End_NonPrinting--\n\
                %AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n%AI17_Begin_Content_if_version_gt:23 1\nfrobnicate\n%AI17_End_Versioned_Content\nfrobnicate\n%AI17_End_Versioned_Content\n\
                %_/Document :\n%_/Binary : /ASCII85Decode ,\n%_(]<[%\n%;;;~>\n%_; (Profile) ,\n%_;\n";
    let doc = read_ok(&format!("{body}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n")))).doc;
    assert_eq!(art(&doc).len(), 1);
}

/// Non-native art (a placed PDF's content) keeps its PDF as ASCII85 in comment lines after
/// `/Data ,`. ASCII85 has `_`, so a line may start with `%_`, which otherwise reads as hidden
/// tokens, and a `(` in it would swallow the rest of the layer (a letterhead saved by Illustrator
/// 28 wouldn't open: "it ends inside a layer"). Data that isn't a PDF is left out with a note.
#[test]
fn non_native_art_that_is_not_a_pdf_is_left_out_with_a_note() {
    let foreign = "/ForeignObject :\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n0 0 Xd\n6 () XW\n0 Ae\n";
    let s = read_ok(&layer(&format!("{foreign}{}", rect_path(0.0, 0.0, 1.0, 1.0) + "f\n")));
    assert_eq!(art(&s.doc).len(), 1);
    assert!(s.warnings.iter().any(|w| w.starts_with(read::NON_NATIVE_ART_UNREAD)), "{:?}", s.warnings);
}

/// A PDF with one page of `size` points holding `content`.
fn tiny_pdf(size: f64, content: &str) -> Vec<u8> {
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_string(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_string(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {size} {size}] /Contents 4 0 R >>"),
        format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len() + 1),
    ];
    let mut pdf = b"%PDF-1.4\n".to_vec();
    let mut offsets = vec![];
    for (i, o) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n{o}\nendobj\n", i + 1).as_bytes());
    }
    let xref = pdf.len();
    pdf.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for o in offsets {
        pdf.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    pdf.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    pdf
}

/// `bytes` as ASCII85 in comment lines of 60 characters, ending with `~>`.
fn ascii85_comment_lines(bytes: &[u8]) -> String {
    let mut text = String::new();
    for chunk in bytes.chunks(4) {
        let mut word = [0u8; 4];
        word[..chunk.len()].copy_from_slice(chunk);
        let mut v = u32::from_be_bytes(word);
        let mut digits = [0u8; 5];
        for d in digits.iter_mut().rev() {
            *d = (v % 85) as u8 + b'!';
            v /= 85;
        }
        text.extend(digits.iter().take(chunk.len() + 1).map(|d| char::from(*d)));
    }
    let mut lines = String::new();
    for line in text.as_bytes().chunks(60) {
        lines.push('%');
        lines.push_str(std::str::from_utf8(line).unwrap());
        lines.push('\n');
    }
    lines.push_str("%~>\n");
    lines
}

/// The PDF of non-native art is drawn by the PDF importer, its art fitted to the object's box:
/// `/Bounds` in a space measured from `/Origin` with y down, then `/RTransform`. Two squares at
/// 1..5 × 2..8 of a 10-point page, blue below red: bounds (1, 2)–(5, 8) → from the origin
/// (10, 20), y down, (11, 18)–(15, 12) → twice that plus (100, 50): x 122..130, y 74..86 in art
/// space, which is y 14..26 in the document (crop marks 100 high). The space has y down, so the
/// PDF's bottom (blue) is the top of the box.
#[test]
fn non_native_art_is_drawn_from_its_pdf_fitted_to_its_bounds() {
    let pdf = tiny_pdf(10.0, "0 0 1 rg 1 2 4 3 re f 1 0 0 rg 1 5 4 3 re f");
    let foreign = format!(
        "q\n/ForeignObject :\n1 /Version ,\n2 0 0 2 100 50  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n{};\n0 0 Xd\n6 () XW\n0 Ae\n{}h\nW\nn\nQ\n",
        ascii85_comment_lines(&pdf),
        rect_path(120.0, 70.0, 130.0, 90.0)
    );
    let s = read_ok(&layer(&foreign));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let objects = art(&s.doc);
    assert_eq!(objects.len(), 1, "{objects:?}");
    // A clipping group: the clip path first, then the non-native art's group.
    let NodeKind::Group { children, clip: true } = &objects[0].kind else { panic!("{:?}", objects[0].kind) };
    assert_eq!(children.len(), 2);
    let group = &children[1];
    assert_eq!(group.name.as_deref(), Some(read::NON_NATIVE_ART));
    let bounds = group.geometric_bounds().unwrap();
    for (got, want) in [(bounds.x0, 122.0), (bounds.y0, 14.0), (bounds.x1, 130.0), (bounds.y1, 26.0)] {
        assert!((got - want).abs() < 1e-6, "{bounds:?}");
    }
    let paths = group.children().unwrap();
    assert_eq!(paths.len(), 2);
    let fill =
        |n: &Node| n.appearance.items.iter().find_map(|it| if let AppearanceItem::Fill(f) = it { Some(f.paint.clone()) } else { None }).unwrap();
    let (blue, red) = (fill(&paths[0]), fill(&paths[1]));
    assert!(matches!(&blue, Paint::Solid { color, swatch: None, .. } if color.to_rgba8(1.0) == [0, 0, 255, 255]), "{blue:?}");
    assert!(matches!(&red, Paint::Solid { color, swatch: None, .. } if color.to_rgba8(1.0) == [255, 0, 0, 255]), "{red:?}");
    let (b, r) = (paths[0].geometric_bounds().unwrap(), paths[1].geometric_bounds().unwrap());
    assert!((b.y0 - 14.0).abs() < 1e-6 && (b.y1 - 20.0).abs() < 1e-6, "blue {b:?}");
    assert!((r.y0 - 20.0).abs() < 1e-6 && (r.y1 - 26.0).abs() < 1e-6, "red {r:?}");
    // Its ids are this document's.
    assert_ne!(group.id, paths[0].id);
    assert_ne!(paths[0].id, paths[1].id);
}

#[test]
fn cmyk_documents() {
    let doc = read_ok(&format!("%AI9_ColorModel: 2\n{}", layer(""))).doc;
    assert_eq!(doc.color_mode, ColorMode::Cmyk);
}

#[test]
fn hostile_data_ends_without_panics() {
    let deep = "0 Ae\nu\n".repeat(300) + &"U\n".repeat(300);
    assert!(read(stream(&layer(&deep)).as_bytes()).unwrap_err().contains("nested too deeply"));
    let keys = "/X :\n".repeat(300);
    assert!(read(stream(&layer(&keys)).as_bytes()).is_err());
    for junk in [
        &b"%AI5_BeginLayer\n(\xff\xfe"[..],
        b"%AI5_BeginLayer\n<zz",
        b"]]]] [ [ ; , : : ;",
        b"1e308 1e308 m 1e308 -1e308 L f",
        b"%%BeginData: 5\rXI\n",
    ] {
        let _ = read(junk);
    }
    // Images with absurd sizes are left out.
    let r = read(&image("[ 1 0 0 1 0 0 ]", (1 << 20, 1 << 20), ("DeviceRGB", 3, 0), &[0; 12], 12)).unwrap();
    assert!(art(&r.doc).is_empty());
}

fn zstd(data: &[u8]) -> Vec<u8> {
    ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
}

/// An EPS whose page draws a line, and the editing data `packed` after it (`marker`: its codec).
fn eps(marker: &str, packed: &[u8]) -> Vec<u8> {
    let lines: String = crate::ps::ascii85(packed).lines().map(|l| format!("%{l}\n")).collect();
    format!(
        "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 200 100\n%%EndComments\n0 0 moveto 10 10 lineto stroke\nshowpage\n%%EOF\n\
         %AI9_PrivateDataBegin\n%!PS-Adobe-3.0 EPSF-3.0\n%AI3_Cropmarks: 0 0 200 100\n%AI5_FileFormat 14.0\n{marker}\n{lines}%AI9_PrivateDataEnd\n"
    )
    .into_bytes()
}

#[test]
fn eps_containers() {
    let body = stream(&layer(&(rect_path(0.0, 0.0, 10.0, 10.0) + "f\n")));
    let body = body.as_bytes();
    for file in [eps("%AI24_DataStream", &zstd(body)), eps("%AI9_DataStream", &crate::ps::deflate(body))] {
        let (ps, _) = crate::sections(&file).unwrap();
        let data = eps_data(ps).unwrap().unwrap();
        assert!(data.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0\n%AI3_Cropmarks"), "the header comes first");
        assert!(data.ends_with(body));
        assert_eq!(read(&data).unwrap().doc.layers.len(), 1);
    }
    // Damaged data says why.
    let (ps, _) = crate::sections(&eps("%AI24_DataStream", b"not zstd")).map(|(p, t)| (p.to_vec(), t.map(<[u8]>::to_vec))).unwrap();
    assert!(eps_data(&ps).unwrap().unwrap_err().contains("Zstandard"));
    // Data that isn't compressed is used as it is.
    let plain = format!("%!PS-Adobe-3.0\n%%EOF\n%AI9_PrivateDataBegin\n{}%AI9_PrivateDataEnd\n", stream(&layer("")));
    assert!(read(&eps_data(plain.as_bytes()).unwrap().unwrap()).is_ok());
    // An EPS without editing data has none.
    let other = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 10 10\n%%EndComments\n0 0 moveto 10 10 lineto stroke\nshowpage\n";
    assert!(eps_data(other).is_none());
}

#[test]
fn ai_private_data() {
    let body = stream(&layer(""));
    let body = body.as_bytes();
    let header = b"%%BoundingBox: 0 0 1 1\r\n";
    for raw in [
        [&b"%AI24_ZStandard_Data"[..], &zstd(body)].concat(),
        [&header[..], b"%AI12_CompressedData", &crate::ps::deflate(body)].concat(),
        body.to_vec(),
    ] {
        let data = decode_private(&raw).unwrap();
        assert!(data.ends_with(b"%%EOF\n"));
        assert!(read(&data).is_ok());
    }
    assert!(decode_private(b"%AI12_CompressedDataxx").is_err());
}

#[test]
fn many_zstandard_frames_are_read_in_one_pass_and_capped() {
    let frame = zstd(b" ");
    let frames = |n: usize| -> Vec<u8> { [&b"%AI24_ZStandard_Data"[..], &frame.repeat(n), &[0; 64]].concat() };
    let started = std::time::Instant::now();
    assert_eq!(decode_private(&frames(4000)).unwrap().len(), 4000);
    assert!(decode_private(&frames(50_000)).unwrap_err().contains("Zstandard"));
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
}

#[test]
fn closing_brackets_without_their_opening_dont_rescan_the_stack() {
    let n = 200_000;
    let body = format!("{}\n{}\n", "1 ".repeat(n), "] ".repeat(n));
    let started = std::time::Instant::now();
    let _ = read(stream(&layer(&body)).as_bytes());
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
}
