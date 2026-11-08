//! The editing data reader on synthetic data, written from the format as `read` documents it.

use std::sync::Arc;

use kurbo::{Affine, Point, Rect};
use vectorcraft_color::gradient::GradientKind;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, Guide, Knockout, LayerColor, LineCap, LineJoin, Node, NodeKind, Unit};

use super::*;

/// [`super::read`] as an EPS reads it.
fn read(data: &[u8]) -> Result<Structure, String> {
    super::read(data, true)
}

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

fn assert_unread(r: &Structure, kind: &str) {
    assert_eq!(r.unread.len(), 1);
    let (id, kinds) = &r.unread[0];
    assert_eq!(kinds, &[kind]);
    assert!(r.doc.node(*id).is_some(), "the unread object has a placeholder");
}

#[test]
fn ruler_units_from_the_header_set_the_documents_units() {
    let body = layer("");
    let with_mm = format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 14.0\n%AI5_RulerUnits: 1\n%AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    );
    assert_eq!(read_ok(&with_mm).doc.units, Unit::Millimeters);
    assert_eq!(read_ok(&stream(&body)).doc.units, Unit::Points, "default when the comment is absent");
    let with_in = with_mm.replace("AI5_RulerUnits: 1", "AI5_RulerUnits: 0");
    assert_eq!(read(with_in.as_bytes()).unwrap().doc.units, Unit::Inches);
    let with_px = with_mm.replace("AI5_RulerUnits: 1", "AI5_RulerUnits: 6");
    assert_eq!(read(with_px.as_bytes()).unwrap().doc.units, Unit::Pixels);
}

#[test]
fn arrays_nested_past_the_depth_limit_are_refused() {
    let deep = |n: usize| layer(&format!("{}{} pop", "[".repeat(n), "]".repeat(n)));
    let nested = |n: usize| read(stream(&deep(n)).as_bytes()).err().is_some_and(|e| e.contains("nested too deeply"));
    assert!(!nested(200));
    assert!(nested(100_000));
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

/// A node's Attributes panel note (empty: none).
fn note(n: &Node) -> String {
    n.extras().attrs.as_deref().map(|a| a.note.clone()).unwrap_or_default()
}

/// The notes of the first layer's objects, in order.
fn notes(doc: &Document) -> Vec<String> {
    art(doc).iter().map(|n| note(n)).collect()
}

/// A note comment names the paths after it until another one replaces it, and an empty one clears
/// it (§5.3.1: `%AI3_Note: <note>`).
#[test]
fn a_note_comment_is_the_note_of_the_paths_after_it() {
    let [a, b, c, d] = [0.0, 20.0, 40.0, 60.0].map(|x| rect_path(x, 0.0, x + 10.0, 10.0));
    let body = format!("%AI3_Note:hello\n{a}f\n{b}f\n%AI3_Note:\n{c}f\n%AI3_Note:world\n{d}f\n");
    assert_eq!(notes(&read_ok(&layer(&body)).doc), ["hello", "hello", "", "world"]);
    // `%%Note:` sets it too.
    assert_eq!(notes(&read_ok(&layer(&format!("%%Note:old style\n{a}f\n"))).doc), ["old style"]);
    // A path without a note has no attributes.
    assert!(art(&read_ok(&layer(&format!("{a}f\n"))).doc)[0].extras().attrs.is_none());
}

/// `%AI5_NoteMore:` adds a line to the note; the text is kept as written, and its bytes are read as
/// UTF-8, else Windows-1252.
#[test]
fn a_note_continues_on_more_lines_and_keeps_its_text() {
    let a = rect_path(0.0, 0.0, 10.0, 10.0);
    let body = format!("%AI3_Note:  two  (words) caf@ \n%AI5_NoteMore:more\n{a}f\n");
    let data = stream(&layer(&body)).replace('@', "\u{1}").into_bytes();
    let data: Vec<u8> = data.into_iter().map(|b| if b == 1 { 0xE9 } else { b }).collect();
    assert_eq!(notes(&read(&data).unwrap().doc), ["  two  (words) café \nmore"]);
    // A continuation of an empty note is its second line; one with no note before it is ignored.
    assert_eq!(notes(&read_ok(&layer(&format!("%AI3_Note:\n%AI5_NoteMore:x\n{a}f\n"))).doc), ["\nx"]);
    assert_eq!(notes(&read_ok(&layer(&format!("%AI5_NoteMore:x\n{a}f\n"))).doc), [""]);
    // A note too long is cut at 1,024 characters, however many lines it has.
    let long = format!("%AI3_Note:{}\n{}{a}f\n", "x".repeat(5000), "%AI5_NoteMore:yy\n".repeat(3000));
    assert_eq!(notes(&read_ok(&layer(&long)).doc)[0].chars().count(), 1024);
    let long = format!("%AI3_Note:x\n{}{a}f\n", "%AI5_NoteMore:yy\n".repeat(3000));
    assert_eq!(notes(&read_ok(&layer(&long)).doc)[0].chars().count(), 1024);
    // A long UTF-8 line stays UTF-8 where it is cut.
    let long = format!("%AI3_Note:{}\n{a}f\n", "é".repeat(3000));
    assert_eq!(notes(&read_ok(&layer(&long)).doc), ["é".repeat(1024)]);
    // A note on a `%_` line is read too.
    assert_eq!(notes(&read_ok(&layer(&format!("%_%AI3_Note:hidden\n{a}f\n"))).doc), ["hidden"]);
}

/// A note before a group or a compound path isn't theirs: their paths keep their own.
#[test]
fn a_note_before_a_group_or_compound_path_is_not_theirs() {
    let [a, b] = [0.0, 20.0].map(|x| rect_path(x, 0.0, x + 10.0, 10.0));
    for (open, close) in [("u", "U"), ("*u", "*U")] {
        let body = format!("%AI3_Note:outer\n0 Ae\n{open}\n%AI3_Note:inner\n{a}f\n{b}f\n{close}\n");
        let doc = read_ok(&layer(&body)).doc;
        let top = &art(&doc)[0];
        assert_eq!(note(top), "", "{open}");
        let kids: Vec<String> = top.children().unwrap().iter().map(|k| note(k)).collect();
        assert_eq!(kids, ["inner", "inner"], "{open}");
    }
}

/// A filled clipping path keeps its note, and no other object takes it.
#[test]
fn a_filled_clipping_paths_note_is_its_own() {
    let body =
        format!("0 Ae\nq\n{RED}%AI3_Note:clip\n{}W\nf\n%AI3_Note:\n{}f\nQ\n", rect_path(5.0, 5.0, 25.0, 25.0), rect_path(0.0, 0.0, 30.0, 30.0));
    let doc = read_ok(&layer(&body)).doc;
    fn walk(nodes: &[Arc<Node>], out: &mut Vec<(bool, String)>) {
        for n in nodes {
            out.push((matches!(n.kind, NodeKind::Path { clipping: true, .. }), note(n)));
            walk(n.children().map(Vec::as_slice).unwrap_or_default(), out);
        }
    }
    let mut all = Vec::new();
    walk(&art(&doc), &mut all);
    let noted: Vec<_> = all.iter().filter(|(_, n)| n == "clip").collect();
    assert_eq!(noted, [&(true, "clip".to_string())], "{all:?}");
    assert_eq!(all.len(), 3, "the group, its clipping path and the art: {all:?}");
}

/// `1 Ap` shows the center point of the paths after it until `0 Ap` (§12, showCenter), which leaves
/// them at an imported path's default: hidden.
#[test]
fn ap_shows_the_center_of_the_paths_after_it() {
    let [a, b, c] = [0.0, 20.0, 40.0].map(|x| rect_path(x, 0.0, x + 10.0, 10.0));
    let ellipse = "50 0 m\n50 5.5 45.5 10 40 10 c\n34.5 10 30 5.5 30 0 c\n30 -5.5 34.5 -10 40 -10 c\n45.5 -10 50 -5.5 50 0 c\n";
    let body = format!("1 Ap\n{a}f\n{b}f\n0 Ap\n{c}f\n{ellipse}f\n2 Ap\n{a}f\n");
    let doc = read_ok(&layer(&body)).doc;
    let shown: Vec<_> = art(&doc).iter().map(|n| (n.extras().attrs.as_deref().and_then(|a| a.show_center), n.shows_center())).collect();
    assert_eq!(shown, [(Some(true), true), (Some(true), true), (None, false), (None, false), (None, false)]);
}

/// What a skipped section sets doesn't reach the art after it.
#[test]
fn a_note_or_ap_in_a_skipped_section_does_not_reach_the_art_after_it() {
    let a = rect_path(0.0, 0.0, 10.0, 10.0);
    let body = format!("%AI3_BeginBrushPattern\n1 Ap\n%AI3_Note:brush\n{a}f\n%AI3_EndBrushPattern\n{}", layer(&format!("{a}f\n")));
    let doc = read_ok(&body).doc;
    let n = &art(&doc)[0];
    assert_eq!(note(n), "");
    assert!(!n.shows_center());
}

/// A node's URL and data (object tags), empty when it has no attributes.
fn tags(n: &Node) -> (String, Vec<(String, String)>) {
    n.extras().attrs.as_deref().map(|a| (a.url.clone(), a.data.clone())).unwrap_or_default()
}

fn tag(id: &str, value: &str) -> (String, String) {
    (id.to_string(), value.to_string())
}

/// `/identifier (string) XT` tags the object before it (version 7 specification §7): `AdobeURL`
/// is its URL, and other tags go to its data in order, an empty string included. Paths, groups,
/// compound paths, text objects and images take tags.
#[test]
fn object_tags_are_kept_on_the_object_before_them() {
    let [a, b, c, d] = [0.0, 20.0, 40.0, 60.0].map(|x| rect_path(x, 0.0, x + 10.0, 10.0));
    let text = "0 To\n1 0 0 1 10 50 0 Tp\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Hi) Tx\nTO\n";
    let body = format!(
        "{a}N\n/part_no (A-12) XT\n/AdobeURL (https://example.com/a) XT\n/part_no (A-13) XT\n\
         u\n{b}f\nU\n/hand () XT\n*u\n{c}f\n{d}f\n*U\n/shape (two \\(parts\\)) XT\n{text}/AdobeURL (https://example.com/t) XT\n{a}f\n"
    );
    let r = read(stream7(&layer7("L", &body)).as_bytes()).unwrap();
    let got: Vec<_> = art(&r.doc).iter().map(|n| tags(n)).collect();
    assert_eq!(
        got,
        [
            ("https://example.com/a".to_string(), vec![tag("part_no", "A-12"), tag("part_no", "A-13")]),
            (String::new(), vec![tag("hand", "")]),
            (String::new(), vec![tag("shape", "two (parts)")]),
            ("https://example.com/t".to_string(), vec![]),
            (String::new(), vec![]),
        ]
    );
    assert!(r.warnings.is_empty() && r.unread.is_empty(), "{:?} {:?}", r.warnings, r.unread);
    // The group's and the compound path's paths keep no tags of their own.
    for n in &art(&r.doc)[1..3] {
        assert!(n.children().unwrap().iter().all(|k| k.extras().attrs.is_none()));
    }
    // A later `AdobeURL` replaces an earlier one.
    let body = format!("{a}f\n/AdobeURL (https://example.com/1) XT\n/AdobeURL (https://example.com/2) XT\n");
    assert_eq!(tags(&art(&read7(&layer7("L", &body)).doc)[0]).0, "https://example.com/2");
}

/// A tag with no object right before it (at a layer's start, after a state change, after a group
/// opens) and a tag with an empty identifier tag nothing; one with operands of another kind is an
/// operator the reader doesn't know.
#[test]
fn object_tags_without_an_object_before_them_tag_nothing() {
    let a = rect_path(0.0, 0.0, 10.0, 10.0);
    for body in
        [format!("/first (x) XT\n{a}f\n"), format!("{a}f\n1 w\n/late (x) XT\n"), format!("u\n/inside (x) XT\n{a}f\nU\n"), format!("{a}f\n/ (x) XT\n")]
    {
        let doc = read7(&layer7("L", &body)).doc;
        let mut all = art(&doc);
        all.extend(all.iter().flat_map(|n| n.children().cloned().unwrap_or_default()).collect::<Vec<_>>());
        assert!(all.iter().all(|n| n.extras().attrs.is_none()), "{body}");
    }
    for bad in ["(x) (y) XT\n", "/id 3 XT\n", "XT\n"] {
        let err = read(stream7(&layer7("L", &format!("{a}f\n{bad}"))).as_bytes()).unwrap_err();
        assert!(err.contains("operators VectorCraft doesn't know"), "{bad}: {err}");
    }
    // Outside any layer too.
    let err = read(stream7(&format!("(x) (y) XT\n{}", layer7("L", &format!("{a}f\n")))).as_bytes()).unwrap_err();
    assert!(err.contains("operators VectorCraft doesn't know"), "{err}");
}

/// A filled clipping path's tags are its own, and a clip group's tags after `Q` are the group's.
/// One object keeps at most 64 tags in its data.
#[test]
fn object_tags_on_clipping_paths_and_the_tag_limit() {
    let body = format!(
        "0 Ae\nq\n{RED}{}W\nf\n/edge (clip) XT\n{}f\nQ\n/group (clipped) XT\n",
        rect_path(5.0, 5.0, 25.0, 25.0),
        rect_path(0.0, 0.0, 30.0, 30.0)
    );
    let doc = read_ok(&layer(&body)).doc;
    let top = &art(&doc);
    assert_eq!(top.len(), 1);
    assert_eq!(tags(&top[0]).1, [tag("group", "clipped")]);
    assert_eq!(top[0].children().unwrap().len(), 2, "the clipping path and the art");
    let tagged: Vec<_> = top[0].children().unwrap().iter().filter(|k| !tags(k).1.is_empty()).map(|k| k.kind.clone()).collect();
    assert_eq!(tagged.len(), 1, "{tagged:?}");
    assert!(matches!(tagged[0], NodeKind::Path { clipping: true, .. }), "{tagged:?}");
    let many: String = (0..70).map(|i| format!("/t{i} ({i}) XT\n")).collect();
    let body = format!("{}f\n{many}/AdobeURL (https://example.com) XT\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let (url, data) = tags(&art(&read_ok(&layer(&body)).doc)[0]);
    assert_eq!(data.len(), 64);
    assert_eq!(data.last(), Some(&tag("t63", "63")));
    assert_eq!(url, "https://example.com", "the URL is kept past the limit");
    // A tag whose identifier or string is longer than 64 KiB is ignored.
    let long = "x".repeat(64 * 1024 + 1);
    let body = format!("{}f\n/{long} (a) XT\n/b ({long}) XT\n/c (ok) XT\n", rect_path(0.0, 0.0, 10.0, 10.0));
    assert_eq!(tags(&art(&read_ok(&layer(&body)).doc)[0]).1, [tag("c", "ok")]);
}

/// An image takes the tags after it, as any other object does.
#[test]
fn object_tags_after_an_image_are_its_own() {
    let mut data = image("[ 10 0 0 10 50 90 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3);
    let at = data.windows(6).position(|w| w == b"N\r\nLB\r").unwrap();
    data.splice(at..at, b"/AdobeURL (https://example.com/i) XT\r\n/sku (I-1) XT\r\n".iter().copied());
    let r = read(&data).unwrap();
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Image(_)), "{a:#?}");
    assert_eq!(tags(&a[0]), ("https://example.com/i".to_string(), vec![tag("sku", "I-1")]));
}

/// The state written after a group's end or a gradient mesh's `/End X!` is still theirs when the
/// object's tags come first.
#[test]
fn object_tags_before_a_group_or_mesh_state_leave_it_theirs() {
    let body = format!("{RED}0 Ae\nu\n0 0.5 0 0 0 Xy\n{}f\nU\n/hand (x) XT\n0 0 Xd\n6 () XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&layer(&body)).doc;
    assert_eq!((art(&doc)[0].opacity, tags(&art(&doc)[0]).1), (0.5, vec![tag("hand", "x")]));
    let corner =
        |x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| format!("%_[1 1 1 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1);
    let mesh = format!(
        "/Mesh X!\n%_(DeviceRGB) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n/End X!\n",
        corner(0, 0, (0, 10), (10, 0)),
        corner(0, 40, (10, 40), (0, 30)),
        corner(40, 40, (40, 30), (30, 40)),
        corner(40, 0, (30, 0), (40, 10)),
    );
    let doc = read_ok(&layer(&format!("{mesh}/hand (y) XT\n1 0.35 0 0 0 Xy\n0 0 Xd\n6 () XW\n"))).doc;
    let (m, l) = (&art(&doc)[0], &doc.layers[0]);
    assert!(matches!(m.kind, NodeKind::Mesh(_)), "{m:#?}");
    assert_eq!((m.opacity, m.blend, tags(m).1), (0.35, BlendMode::Multiply, vec![tag("hand", "y")]));
    assert_eq!((l.opacity, l.blend), (1.0, BlendMode::Normal), "not the layer's");
}

/// The tags of an object whose drawn look stands for it (its style not found) stay with the look.
#[test]
fn a_drawn_look_keeps_its_objects_tags() {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let body = format!(
        "0 Ae\nu\n{RED}{}f\nU\n0 0 Xd\n6 () XW\n%_{hidden_rect}n\n%_/AdobeURL (https://example.com/n) XT\n%_/sku (N-1) XT\n1 (Gone style) XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
    );
    let r = read_ok(&layer(&body));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(tags(&a[0]), ("https://example.com/n".to_string(), vec![tag("sku", "N-1")]));
    // A look with tags of its own keeps them, and takes the object's after them.
    let a = art(&read_ok(&layer(&body.replace("U\n0 0 Xd", "U\n/own (L) XT\n0 0 Xd"))).doc);
    assert_eq!(tags(&a[0]), ("https://example.com/n".to_string(), vec![tag("own", "L"), tag("sku", "N-1")]));
}

/// A clipping subpath on `%_` lines in a drawn look leaves with the fill read apart from it.
#[test]
fn a_hidden_clipping_paths_fill_leaves_the_look_with_it() {
    let (a, b, c) = (rect_path(0.0, 0.0, 30.0, 30.0), rect_path(5.0, 5.0, 25.0, 25.0), rect_path(0.0, 0.0, 10.0, 10.0));
    let two = rect_path(8.0, 8.0, 20.0, 20.0);
    let second: String = format!("0 0 1 0 0 0 1 Xa\n{two}h\nW\nf\n").lines().map(|l| format!("%_{l}\n")).collect();
    let hidden_clip = format!("*u\n{RED}{a}h\nW\nf\n{second}*U\n");
    let body = format!("0 Ae\nu\nu\n{hidden_clip}{b}f\nU\nU\n0 0 Xd\n6 () XW\n%_{}n\n1 (Gone style) XW\n", c.replace('\n', "\n%_"));
    let doc = read_ok(&layer(&body)).doc;
    let mut leaves = vec![];
    doc.layers.iter().for_each(|l| {
        l.walk(&mut |n| {
            if let NodeKind::Path { clipping, .. } = &n.kind {
                leaves.push((*clipping, fill(n).color()));
            }
        })
    });
    // The first subpath, its fill and the look's plain path stay; the second subpath and its fill leave.
    let (red, blue) = (Some(Color::rgb(1.0, 0.0, 0.0)), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(leaves, [(true, None), (false, red), (false, blue)]);
}

/// Tags in a brush's art don't keep it from being read, and stay out of the art: it isn't an object
/// of the document.
#[test]
fn a_brushs_art_with_tags_is_read_without_them() {
    let art_ = format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n0 g\n{}f\n/sku (D-1) XT\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0));
    let r = read_ok(&format!("{art_}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    let vectorcraft_brush::BrushKind::Scatter(s) = brush_named(&lib, "Spray") else { panic!("scatter: {:?}", r.warnings) };
    assert_eq!(s.art.geometric_bounds().map(|b| (b.width(), b.height())), Some((4.0, 4.0)));
    assert!(s.art.extras().attrs.is_none(), "{:?}", s.art.extras().attrs);
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

/// Each subpath's anchor kinds, in order.
fn anchor_kinds(n: &Node) -> Vec<Vec<vectorcraft_geom::AnchorKind>> {
    let Some(path) = n.path_data() else { panic!("{:?}", n.kind) };
    path.subpaths.iter().map(|s| s.anchors.iter().map(|a| a.kind).collect()).collect()
}

/// A path's anchors at (x, y) in document space, and their handles.
fn anchor_points(n: &Node) -> Vec<[(f64, f64); 3]> {
    let Some(path) = n.path_data() else { panic!("{:?}", n.kind) };
    path.subpaths.iter().flat_map(|s| s.anchors.iter()).map(|a| [(a.p.x, a.p.y), (a.h_in.x, a.h_in.y), (a.h_out.x, a.h_out.y)]).collect()
}

/// A segment operator in lower case makes the anchor it ends on smooth, in upper case a corner
/// (§5.3.2), whether the segment is straight or curved and whatever its handles: the points and
/// handles stay where they are.
#[test]
fn a_segment_operators_case_sets_the_kind_of_the_anchor_it_ends_on() {
    use vectorcraft_geom::AnchorKind::{Corner, Smooth};
    let doc = read_ok(&layer("0 0 m\n10 10 20 0 30 0 c\n40 10 50 0 60 0 C\nh\nS\n")).doc;
    let a = art(&doc);
    assert_eq!(anchor_kinds(&a[0]), [vec![Corner, Smooth, Corner]], "handles that aren't in line stay a smooth point's");
    assert_eq!(
        anchor_points(&a[0]),
        [[(0.0, 100.0), (0.0, 100.0), (10.0, 90.0)], [(30.0, 100.0), (20.0, 100.0), (40.0, 90.0)], [(60.0, 100.0), (50.0, 100.0), (60.0, 100.0)]]
    );
    // `l`, `L`, `v`, `y`, `C` with handles in line (a corner all the same) and `c`, on an open path.
    let doc = read_ok(&layer("0 0 m\n10 0 l\n20 0 L\n30 10 40 10 v\n50 10 60 10 y\n70 10 80 10 90 10 C\n100 10 110 0 110 0 c\nS\n")).doc;
    let a = art(&doc);
    assert_eq!(anchor_kinds(&a[0]), [vec![Corner, Smooth, Corner, Smooth, Smooth, Corner, Smooth]]);
    assert_eq!(anchor_points(&a[0])[5], [(90.0, 90.0), (80.0, 90.0), (100.0, 90.0)]);
}

/// A closed path's first anchor takes the kind of the segment back to it; a path closed without
/// one keeps its first anchor's kind from its handles.
#[test]
fn a_closed_paths_start_takes_the_case_of_the_segment_back_to_it() {
    use vectorcraft_geom::AnchorKind::{Corner, Smooth};
    let start = |close: &str| {
        let doc = read_ok(&layer(&format!("{close}s\n"))).doc;
        anchor_kinds(&art(&doc)[0])
    };
    assert_eq!(start("0 0 m\n40 0 L\n40 40 L\n0 0 l\n"), [vec![Smooth, Corner, Corner]]);
    assert_eq!(start("0 0 m\n40 0 l\n40 40 l\n0 0 L\n"), [vec![Corner, Smooth, Smooth]]);
    // The start's handles are in line, so it would be smooth by them alone.
    assert_eq!(start("0 0 m\n10 0 40 30 40 40 c\n40 50 -10 0 0 0 C\n"), [vec![Corner, Smooth]]);
    assert_eq!(start("0 0 m\n10 0 40 30 40 40 C\n40 50 -10 0 0 0 c\n"), [vec![Smooth, Corner]]);
    assert_eq!(start("0 0 m\n40 0 L\n40 40 l\nh\n"), [vec![Corner, Corner, Smooth]], "no segment back to the start");
    assert_eq!(start("0 0 m\n40 0 L\n40 40 l\n"), [vec![Corner, Corner, Smooth]], "`s` closes it the same way");
}

/// Every `m` starts a subpath of its own, in a compound path's members and within one path.
#[test]
fn each_subpath_takes_the_case_of_its_own_segments() {
    use vectorcraft_geom::AnchorKind::{Corner, Smooth};
    let body = "*u\n0 0 m\n10 0 l\n10 10 L\nh\nf\n20 0 m\n30 0 L\n30 10 l\n20 0 l\nf\n*U\n\
                0 50 m\n10 50 L\n10 60 l\nh\n20 50 m\n30 50 l\n30 60 L\n20 50 l\nf\n";
    let a = art(&read_ok(&layer(body)).doc);
    let NodeKind::Compound { children, .. } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(anchor_kinds(&children[0]), [vec![Corner, Smooth, Corner]]);
    assert_eq!(anchor_kinds(&children[1]), [vec![Smooth, Corner, Smooth]]);
    assert_eq!(anchor_kinds(&a[1]), [vec![Corner, Corner, Smooth], vec![Smooth, Smooth, Corner]]);
}

/// Guide paths and clipping paths keep the case of their segments too.
#[test]
fn guide_and_clipping_paths_take_the_case_of_their_segments() {
    use vectorcraft_geom::AnchorKind::{Corner, Smooth};
    let a = art(&read_ok(&layer("(N) *\n10 10 m\n50 10 l\n90 10 L\n(N) *\n")).doc);
    assert!(matches!(a[0].kind, NodeKind::Path { guide: true, .. }));
    assert_eq!(anchor_kinds(&a[0]), [vec![Corner, Smooth, Corner]]);
    let clipped = format!("0 Ae\nq\n{}f\n0 0 m\n30 0 l\n30 30 L\n0 0 l\nh\nW\nn\nQ\n", rect_path(0.0, 0.0, 40.0, 40.0));
    let a = art(&read_ok(&layer(&clipped)).doc);
    let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!(anchor_kinds(&children[0]), [vec![Smooth, Smooth, Corner]]);
}

/// Operands that don't make a segment leave no kind behind, so the kinds after them stay with
/// their own anchors.
#[test]
fn a_segment_that_isnt_read_leaves_the_kinds_in_step() {
    use vectorcraft_geom::AnchorKind::{Corner, Smooth};
    let a = art(&read_ok(&layer("10 c\n0 0 m\n5 l\n10 0 L\n(x) 20 y\n20 0 l\n30 0 L\nS\n")).doc);
    assert_eq!(anchor_kinds(&a[0]), [vec![Corner, Corner, Smooth, Corner]]);
    // A subpath of a lone `m` is a point of its own.
    let a = art(&read_ok(&layer("5 5 m\n0 0 m\n10 0 l\n20 0 L\nS\n")).doc);
    assert_eq!(anchor_kinds(&a[0]), [vec![Corner], vec![Corner, Smooth, Corner]]);
}

/// `0 w` is the thinnest line the device draws (PostScript's `setlinewidth`), so a stroke of width 0
/// comes in as a hairline: on a path, on a gradient-filled path that `1 BB` strokes, and on type.
#[test]
fn a_line_width_of_zero_is_a_hairline() {
    let red = Some(Color::rgb(1.0, 0.0, 0.0));
    let a = art(&read_ok(&layer("0 1 1 0 1 0 0 XA\n2 J\n0 w\n20 20 m\n180 80 L\nS\n")).doc);
    assert_eq!(a.len(), 1);
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.width, st.paint.color(), st.cap), (0.25, red, LineCap::Square));

    let art_ = format!(
        "0 1 1 0 1 0 0 XA\n0 w\n{}Bb\n0 0 0 0 Bh\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n50 0 0 -50 100 50 Bm\nf\n1 BB\n",
        rect_path(50.0, 0.0, 150.0, 100.0)
    );
    let a = art(&read_ok(&format!("{G_DEFS}{}", layer(&art_))).doc);
    assert!(matches!(fill(&a[0]), Paint::Gradient(_)), "{:?}", fill(&a[0]));
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.width, st.paint.color()), (0.25, red));

    let text = "0 To\n1 0 0 1 10 50 0 Tp\nTP\n1 Tr\n0 1 1 0 1 0 0 XA\n0 w\n/_Helvetica 9 Tf\n(hi) Tx\nTO\n";
    let a = art(&read_ok(&layer(text)).doc);
    let Some(NodeKind::Text(t)) = a.first().map(|n| &n.kind) else { panic!("{a:#?}") };
    let style = &t.runs[0].style;
    assert_eq!((style.stroke_width, style.stroke.color(), style.fill.clone()), (0.25, red, Paint::None));
}

/// An art style's stroke of width 0 comes in as a hairline too.
#[test]
fn an_art_style_s_line_width_of_zero_is_a_hairline() {
    let styles = "%AI9_BeginArtStyles\n/KnownStyle :\n(Thin) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n\
        /StrokeStyle : 0 R\n0 1 1 0 1 0 0 XA\n0 J 0 j 0 w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n\
        /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n;\n%AI9_EndArtStyles\n";
    let line = "20 20 m\n180 80 L\n";
    let art_ = format!("0 Ae\nu\n0 1 1 0 1 0 0 XA\n0 w\n{line}S\nU\n%_0 1 1 0 1 0 0 XA\n%_0 w\n%_{}S\n1 (Thin) XW\n", line.replace('\n', "\n%_"));
    let r = read_ok(&format!("{styles}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look: {a:#?}");
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.width, st.paint.color()), (0.25, Some(Color::rgb(1.0, 0.0, 0.0))));
}

/// `Xk` and `XK` are global process colours: a name, a tint and whether the colour is RGB, as
/// `Xx` has them (#1025). They come in as global swatches that aren't inks, linked at their tint.
#[test]
fn global_process_colours_are_global_swatches_at_their_tint() {
    let art_ = format!("0 0 0 1 (Black global) 0.4 0 Xk\n0 0 0 0 1 0.5 0 (Brand orange) 0 1 XK\n{}b\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    let Paint::Solid { swatch, tint, color } = fill(&a[0]) else { panic!("{:?}", fill(&a[0])) };
    assert_eq!(swatch.as_deref(), Some("Black global"));
    assert!((tint - 0.6).abs() < 1e-6 && matches!(color, Color::Cmyk { k, .. } if (k - 0.6).abs() < 1e-6), "{tint} {color:?}");
    let stroke = a[0].appearance.stroke().unwrap().paint.clone();
    assert!(
        matches!(&stroke, Paint::Solid { swatch: Some(s), tint, color } if s == "Brand orange" && *tint == 1.0 && *color == Color::rgb(1.0, 0.5, 0.0)),
        "{stroke:?}"
    );
    for name in ["Black global", "Brand orange"] {
        let sw = doc.swatch(name).unwrap();
        assert!(sw.global && !sw.spot, "{sw:?}");
    }
    assert_eq!(doc.swatch("Black global").unwrap().paint.color(), Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)), "the swatch is the full colour");
}

/// A gradient stop naming a global process colour doesn't make it an ink (#1771): the colour
/// operator's definition (`Xk`) makes the swatch, though the gradient, defined first, names the
/// colour first. The lines are the ones posted in #1771, with a neutral name.
#[test]
fn a_gradient_stop_naming_a_process_colour_leaves_its_swatch_process() {
    let defs = "%AI5_BeginGradient: (Brand)\n(Brand) 0 2 Bd\n[\n<00FF>\n0\n\
                0.930000007152557 0.100000001490116 0 0 1 1 6 50 0 %_BS\n\
                %_0.930000007152557 0.100000001490116 0 0 (Test Cyan) 0 0 5 1 6 50 0 Bs\n\
                %_0 0 0 0 1 0 0 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!("0.930000007152557 0.100000001490116 0 0 (Test Cyan) 0 0 Xk\n{}f\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let near =
        |c: Option<Color>| matches!(c, Some(Color::Cmyk { c, m, y, k }) if (c - 0.93).abs() < 1e-3 && (m - 0.1).abs() < 1e-3 && y == 0.0 && k == 0.0);
    let doc = read_ok(&format!("{defs}{}", layer(&art_))).doc;
    let sw = doc.swatch("Test Cyan").unwrap();
    assert!(sw.global && !sw.spot, "a global process colour, not an ink: {sw:?}");
    assert!(near(sw.paint.color()), "C=93 M=10: {sw:?}");
    assert!(near(fill(&art(&doc)[0]).color()), "the fill: {:?}", fill(&art(&doc)[0]));
    // Named only by the stop, the colour is still a swatch of its own, at the stop's values.
    let doc = read_ok(&format!("{defs}{}", layer(&format!("{}f\n", rect_path(0.0, 0.0, 10.0, 10.0))))).doc;
    let sw = doc.swatch("Test Cyan").unwrap();
    assert!(near(sw.paint.color()), "{sw:?}");
}

/// A gradient whose name in the file is shown as the name of a color a later gradient's stop names
/// is numbered: the color keeps its swatch.
#[test]
fn a_gradient_shown_as_a_stops_color_name_is_numbered() {
    let first = "%AI5_BeginGradient: (Test Cyan\\n1)\n(Test Cyan\\n1) 0 2 Bd\n[\n%_1 0 50 0 Bs\n%_0 0 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let brand = "%AI5_BeginGradient: (Brand)\n(Brand) 0 2 Bd\n[\n\
                 %_0.93 0.1 0 0 (Test Cyan) 0 0 5 1 6 50 0 Bs\n%_0 0 0 0 1 0 0 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!("Bb\n1 (Test Cyan\\n1) 0 0 0 10 1 0 0 1 0 0 Bg\n{}f\n0 BB\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&format!("{first}{brand}{}", layer(&art_))).doc;
    let Paint::Gradient(g) = fill(&art(&doc)[0]) else { panic!("{:?}", fill(&art(&doc)[0])) };
    assert_eq!(g.swatch.as_deref(), Some("Test Cyan 2"));
    let sw = doc.swatch("Test Cyan").unwrap();
    assert!(matches!(sw.paint, Paint::Solid { .. }) && sw.global, "{sw:?}");
}

/// A gradient whose name has no newline keeps it when a gradient shown as that name is defined
/// first: the one shown up to its newline is numbered.
#[test]
fn a_gradient_name_without_a_newline_is_kept_whichever_comes_first() {
    let gradient = |name: &str| format!("%AI5_BeginGradient: ({name})\n({name}) 0 2 Bd\n[\n%_1 0 50 0 Bs\n%_0 0 50 100 Bs\nBD\n%AI5_EndGradient\n");
    let filled = |name: &str| format!("Bb\n1 ({name}) 0 0 0 10 1 0 0 1 0 0 Bg\n{}f\n0 BB\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&format!("{}{}{}", gradient("Sky\\n1"), gradient("Sky"), layer(&format!("{}{}", filled("Sky\\n1"), filled("Sky"))))).doc;
    let links: Vec<_> = art(&doc).iter().map(|n| if let Paint::Gradient(g) = fill(n) { g.swatch.clone() } else { None }).collect();
    assert_eq!(links, [Some("Sky 2".to_string()), Some("Sky".to_string())]);
    // A gradient without stops isn't kept, and its name stays free.
    let empty = "%AI5_BeginGradient: (Sky)\n(Sky) 0 0 Bd\n[\nBD\n%AI5_EndGradient\n";
    let doc = read_ok(&format!("{empty}{}{}", gradient("Sky\\n1"), layer(&filled("Sky\\n1")))).doc;
    assert!(matches!(fill(&art(&doc)[0]), Paint::Gradient(g) if g.swatch.as_deref() == Some("Sky")));
}

/// A pattern's definition (`%AI3_BeginPattern`): its name and tile (8 points square here, y up),
/// then its art on `%_` lines (a group of the tile's unpainted box and a dot, a round-capped
/// stroke of no length), `E`.
const DOTS: &str = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 8 8\n%_0 A\n%_0 Xw\n%_0 Ae\n%_u\n%_0 O\n%_0 8 m\n%_8 8 L\n%_8 0 L\n%_0 0 L\n%_0 8 L\n%_n\n\
                    %_0 R\n%_0 0 0 1 K\n%_1 J 0 j 2 w 4 M []0 d\n%_4 4 m\n%_4 4 L\n%_S\n%_U\n%_/ArtDictionary :\n%_;\n%_\n%_9 () XW\nE\n%AI3_EndPattern\n";

/// An unpainted box that a pattern's definition marks as its tile's box isn't art; a painted one
/// with the mark stays.
#[test]
fn a_patterns_marked_tile_box_is_not_art() {
    let mark = "%_/ArtDictionary :\n%_1 /Bool (AIPattern_Editor_Backing_Tile_Rect) ,\n%_;\n%_\n";
    let def = |paint: &str, op: &str| {
        format!(
            "%AI3_BeginPattern: (Box)\n(Box) 0 0 8 8\n%_0 A\n%_0 Xw\n%_{paint}0 XR\n%_0 8 m\n%_8 8 L\n%_8 0 L\n%_0 0 L\n%_0 8 L\n%_{op}\n{mark}\
             %_0 0 0 1 k\n%_2 2 m\n%_6 2 L\n%_6 6 L\n%_2 6 L\n%_f\nE\n%AI3_EndPattern\n"
        )
    };
    let layer = format!(
        "%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n(Box) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\nLB\n%AI5_EndLayer--\n",
        rect_path(10.0, 20.0, 70.0, 80.0)
    );
    let s = read_ok(&format!("{}{layer}", def("", "n")));
    let art = &s.doc.pattern("Box").unwrap().art;
    assert_eq!(art.len(), 1, "the square only");
    assert_eq!(art[0].geometric_bounds(), Some(Rect::new(2.0, -6.0, 6.0, -2.0)));
    let s = read_ok(&format!("{}{layer}", def("1 0 0 0 k\n%_", "f")));
    assert_eq!(s.doc.pattern("Box").unwrap().art.len(), 2, "a painted box is art");
    // The mark after a group or a compound path of an unpainted box leaves it in the art.
    for (open, close, kind) in [("u", "U", "group"), ("*u", "*U", "compound path")] {
        let def = format!(
            "%AI3_BeginPattern: (Box)\n(Box) 0 0 8 8\n%_0 A\n%_0 Xw\n%_{open}\n%_0 8 m\n%_8 8 L\n%_8 0 L\n%_0 0 L\n%_0 8 L\n%_n\n%_{close}\n{mark}\
             %_0 0 0 1 k\n%_2 2 m\n%_6 2 L\n%_6 6 L\n%_2 6 L\n%_f\nE\n%AI3_EndPattern\n"
        );
        let s = read_ok(&format!("{def}{layer}"));
        assert_eq!(s.doc.pattern("Box").unwrap().art.len(), 2, "after a {kind}");
    }
    // Outside a pattern's definition the mark leaves the box in the art.
    let marked = format!(
        "%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\n0 XR\n{}n\n{mark}0 0 0 1 k\n{}f\nLB\n%AI5_EndLayer--\n",
        rect_path(10.0, 20.0, 70.0, 80.0),
        rect_path(20.0, 30.0, 40.0, 50.0)
    );
    let s = read_ok(&marked);
    assert_eq!(s.doc.layers[0].children().unwrap().len(), 2, "outside a pattern");
}

/// Patterns are read into pattern swatches, and `p` fills with one (#1025): the art keeps its
/// layers, the fill places the tiles by its matrix.
#[test]
fn patterns_are_pattern_swatches_and_fill_with_them() {
    let sub = |name: &str, art: &str| format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n({name}) Ln\n{art}LB\n%AI5_EndLayer--\n");
    let body = format!(
        "{DOTS}%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(Ground) Ln\n{}{}LB\n%AI5_EndLayer--\n",
        sub("Lawn", &format!("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 20.0, 70.0, 80.0))),
        sub("Paving", &format!("(Dots) 10 20 2 2 0 0 0 0 0 [2 0 0 2 10 20] P\n{}S\n", rect_path(90.0, 20.0, 150.0, 80.0))),
    );
    let s = read_ok(&body);
    let doc = &s.doc;
    let ground = &doc.layers[0];
    let subs: Vec<_> = ground.children().unwrap().iter().map(|l| l.name.clone().unwrap_or_default()).collect();
    assert_eq!(subs, ["Lawn", "Paving"]);
    let def = doc.pattern("Dots").unwrap();
    assert_eq!(def.tile, Rect::new(0.0, -8.0, 8.0, 0.0), "the tile, y down");
    assert_eq!(def.art.len(), 1, "the tile's group");
    let tile_box = &def.art[0].children().unwrap()[0];
    assert_eq!(tile_box.geometric_bounds(), Some(def.tile), "the tile's box is where the tile is");
    assert_eq!(def.art[0].children().unwrap().len(), 2);
    assert!(matches!(&doc.swatch("Dots").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Dots"));
    let lawn = &ground.children().unwrap()[0].children().unwrap()[0];
    let Paint::Pattern { pattern, xf } = fill(lawn) else { panic!("{:?}", fill(lawn)) };
    assert_eq!(pattern, "Dots");
    // The tile's top left (0, 8) in art space is (0, 92) on the page (crop marks 100 high); a dot
    // centre (4, 4) is at (4, 96).
    assert_eq!(xf * Point::ZERO, Point::new(0.0, 92.0));
    assert_eq!(xf * Point::new(4.0, 4.0), Point::new(4.0, 96.0));
    // A stroke with it, scaled twice and moved by (10, 20): the tile's top left is at (10, 36) in
    // art space.
    let paving = &ground.children().unwrap()[1].children().unwrap()[0];
    let Some(Paint::Pattern { xf, .. }) = paving.appearance.stroke().map(|st| st.paint.clone()) else { panic!() };
    assert_eq!(xf * Point::ZERO, Point::new(10.0, 64.0));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
}

/// A pattern's name in the file is shown up to its newline (AI7 spec §4): two patterns named
/// `Dots` apart from what follows the newline are "Dots" and "Dots 2", each fill paints with its
/// own, and one defined again under its name in the file replaces the first.
#[test]
fn pattern_names_are_shown_up_to_their_newline() {
    let def = |name: &str, tile: u32| DOTS.replace("(Dots)", &format!("({name})")).replace("0 0 8 8\n", &format!("0 0 {tile} {tile}\n"));
    let filled = |name: &str| format!("({name}) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 20.0, 70.0, 80.0));
    let body = format!(
        "{}{}{}{}",
        def("Dots\\n1", 8),
        def("Dots\\n2", 8),
        def("Dots\\n2", 9),
        layer(&format!("{}{}", filled("Dots\\n2"), filled("Dots\\n1")))
    );
    let s = read_ok(&body);
    let doc = &s.doc;
    let tiles: Vec<_> = doc.patterns.iter().map(|p| (p.name.as_str(), p.tile.width())).collect();
    assert_eq!(tiles, [("Dots", 8.0), ("Dots 2", 9.0)]);
    for name in ["Dots", "Dots 2"] {
        assert!(matches!(&doc.swatch(name).unwrap().paint, Paint::Pattern { pattern, .. } if pattern == name), "{name}");
    }
    let fills: Vec<_> = art(doc).iter().map(|n| if let Paint::Pattern { pattern, .. } = fill(n) { pattern } else { String::new() }).collect();
    assert_eq!(fills, ["Dots 2", "Dots"]);
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    // Without a palette (or with an empty one) the document keeps its default swatches, and a named
    // color the art uses has its swatch: a pattern takes none of their names.
    let empty = "%AI5_BeginPalette\n0 0 Pb\nPB\n%AI5_EndPalette\n";
    let ink = format!("0 1 1 0 (Ink) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    for palette in ["", empty] {
        let body =
            format!("{}{}{palette}{}", def("Black\\n1", 8), def("Ink\\n1", 8), layer(&format!("{}{}{ink}", filled("Black\\n1"), filled("Ink\\n1"))));
        let doc = read_ok(&body).doc;
        for (solid, pattern) in [("Black", "Black 2"), ("Ink", "Ink 2")] {
            assert!(matches!(&doc.swatch(pattern).unwrap().paint, Paint::Pattern { pattern: p, .. } if p == pattern), "{palette:?}");
            assert!(matches!(doc.swatch(solid).unwrap().paint, Paint::Solid { .. }), "{palette:?}");
        }
        let fills: Vec<_> = art(&doc).iter().map(|n| fill(n)).collect();
        assert!(matches!(&fills[..2], [Paint::Pattern { pattern: a, .. }, Paint::Pattern { pattern: b, .. }] if a == "Black 2" && b == "Ink 2"));
    }
    // Nor the name of a custom color the program of an EPS gives, or None's.
    let ps = b"%!PS-Adobe-3.0\n%%CMYKCustomColor: 0 0 1 0 (Outer Ink)\n%%EndComments\n";
    let data = stream(&format!("{}{}{}", def("Outer Ink\\n1", 8), def("[None]\\n1", 8), layer(&filled("Outer Ink\\n1"))));
    let doc = super::read_beside(data.as_bytes(), true, ps).unwrap().doc;
    assert_eq!(doc.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Outer Ink 2", "[None] 2"]);
    assert!(matches!(doc.swatch("Outer Ink").unwrap().paint, Paint::Solid { .. }));
    assert!(matches!(fill(&art(&doc)[0]), Paint::Pattern { pattern, .. } if pattern == "Outer Ink 2"));
}

/// A pattern shown as the name of one of the document's other swatches is numbered: a swatch or a
/// color group of the palette the reader keeps (under any version's prefix, a swatch it names by
/// its values too), Registration, and a custom color of the editing data's own header. A pattern
/// whose name has no newline keeps it, whichever is defined first. The default swatches go when a
/// palette replaces them, and their names are free then; a palette only in versioned content that
/// isn't read replaces nothing.
#[test]
fn pattern_names_are_numbered_around_the_documents_swatches() {
    let def = |name: &str| DOTS.replace("(Dots)", &format!("({name})"));
    let palette = "%AI12_BeginPalette\n0 0 Pb\n0 0 0 0.5 k\n(Mist)\nPc\n1 (Greys) 1 Pg\n0 0 0 0.2 k\nPc\nPB\n%AI12_EndPalette\n";
    let header = "%%CMYKCustomColor: 0 0 1 0 (Head Ink)\n";
    let doc = |defs: &[&str], palette: &str| {
        let body = format!("{}{palette}{}", defs.iter().map(|n| def(n)).collect::<String>(), layer(""));
        read(stream_with_header(header, &body).as_bytes()).unwrap().doc
    };
    let names = |d: &Document| d.patterns.iter().map(|p| p.name.clone()).collect::<Vec<_>>();
    let d = doc(&["Mist\\n1", "Greys\\n1", "[Registration]\\n1", "Head Ink\\n1", "Red\\n1", "Sky\\n1", "Sky"], palette);
    let generated = d.swatch_groups[0].swatches[0].name.clone();
    let all: Vec<&str> = d.swatches_iter().map(|s| s.name.as_str()).collect();
    assert!(d.swatch("Mist").is_some() && d.swatch("Head Ink").is_some(), "{all:?}");
    assert!(matches!(&d.swatch("Red").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Red"), "{all:?}");
    assert_eq!(names(&d), ["Mist 2", "Greys 2", "[Registration] 2", "Head Ink 2", "Red", "Sky 2", "Sky"]);
    let d = doc(&[&format!("{generated}\\n1")], palette);
    assert_eq!(names(&d), [format!("{generated} 2")]);
    // Without a palette the default swatches stay, and so they do when the only palette is in
    // versioned content that isn't read.
    let versioned = format!("%AI17_Begin_Content_if_version_gt:17 1\n%AI17_Alternate_Content\n{palette}%AI17_End_Versioned_Content\n");
    for palette in ["", &versioned] {
        let d = doc(&["Red\\n1"], palette);
        assert!(matches!(d.swatch("Red").unwrap().paint, Paint::Solid { .. }), "{palette:?}");
        assert_eq!(names(&d), ["Red 2"], "{palette:?}");
    }
    // A pattern and a color in versioned content that isn't read take no name.
    let unread = format!(
        "%AI17_Begin_Content_if_version_gt:17 1\n%AI17_Alternate_Content\n{}{}%AI17_End_Versioned_Content\n",
        def("Sky"),
        layer("0 1 1 0 (Sea) 0 x\n")
    );
    let d = doc(&["Sky\\n1", "Sea\\n1"], &unread);
    assert_eq!(names(&d), ["Sky", "Sea"]);
    // Nor does a color of a palette that a later one replaces, or of a palette's alternate.
    let replaced = "%AI5_BeginPalette\n0 0 Pb\n0 1 1 0 (Sky) 0 x\n(Sky)\nPc\nPB\n%AI5_EndPalette\n\
                    %AI5_BeginPalette\n0 0 Pb\n%AI17_Begin_Content_if_version_gt:17 1\n0 0 0 1 k\n(Ink)\nPc\n\
                    %AI17_Alternate_Content\n0 1 0 0 (Sea) 0 x\n(Sea)\nPc\n%AI17_End_Versioned_Content\nPB\n%AI5_EndPalette\n";
    let d = doc(&["Sky\\n1", "Sea\\n1"], replaced);
    assert_eq!(names(&d), ["Sky", "Sea"]);
    // Versioned content that doesn't end is read whole, its colors too.
    let unended = format!("%AI17_Begin_Content_if_version_gt:17 1\n%AI17_Alternate_Content\n{}", layer("0 1 1 0 (Sea) 0 x\n"));
    let d = doc(&["Sea\\n1"], &unended);
    assert_eq!(names(&d), ["Sea 2"]);
}

/// A pattern whose name has no newline keeps it, and its swatch takes the first free "Name 2" when
/// another swatch has the name: a default swatch, a named color the art uses, or a color of the
/// palette. A pattern a swatch already paints with gets no other.
#[test]
fn a_pattern_swatch_named_as_another_swatch_is_numbered() {
    let def = |name: &str| DOTS.replace("(Dots)", &format!("({name})"));
    let filled = |name: &str| format!("({name}) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 20.0, 70.0, 80.0));
    let ink = format!("0 1 1 0 (Ink) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let paints = |d: &Document, name: &str| d.swatches_iter().find(|s| s.name == name).map(|s| s.paint.clone());
    let doc = read_ok(&format!("{}{}{}", def("White"), def("Ink"), layer(&format!("{}{}{ink}", filled("White"), filled("Ink"))))).doc;
    assert_eq!(doc.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["White", "Ink"]);
    for name in ["White", "Ink"] {
        assert!(matches!(paints(&doc, name), Some(Paint::Solid { .. })), "{name}");
        let numbered = format!("{name} 2");
        assert!(matches!(paints(&doc, &numbered), Some(Paint::Pattern { pattern, .. }) if pattern == name), "{numbered}");
    }
    let fills: Vec<_> = art(&doc).iter().map(|n| fill(n)).collect();
    assert!(matches!(&fills[..2], [Paint::Pattern { pattern: a, .. }, Paint::Pattern { pattern: b, .. }] if a == "White" && b == "Ink"));
    // A pattern defined later under the numbered name keeps its swatch's name.
    let doc = read_ok(&format!("{}{}{}", def("White"), def("White 2"), layer(&filled("White")))).doc;
    assert!(matches!(paints(&doc, "White 2"), Some(Paint::Pattern { pattern, .. }) if pattern == "White 2"));
    assert!(matches!(paints(&doc, "White 3"), Some(Paint::Pattern { pattern, .. }) if pattern == "White"));
    // A color of the palette.
    let palette = "%AI5_BeginPalette\n0 0 Pb\n0 0 0 0.5 k\n(Mist)\nPc\nPB\n%AI5_EndPalette\n";
    let doc = read_ok(&format!("{}{palette}{}", def("Mist"), layer(&filled("Mist")))).doc;
    assert!(matches!(paints(&doc, "Mist"), Some(Paint::Solid { .. })));
    assert!(matches!(paints(&doc, "Mist 2"), Some(Paint::Pattern { pattern, .. }) if pattern == "Mist"));
    // A palette swatch under another name already paints with the pattern.
    let palette = "%AI5_BeginPalette\n0 0 Pb\n0 0 0 0.5 k\n(Mist)\nPc\n(Mist) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Fog)\nPc\nPB\n%AI5_EndPalette\n";
    let doc = read_ok(&format!("{}{palette}{}", def("Mist"), layer(&filled("Mist")))).doc;
    let names: Vec<&str> = doc.swatches_iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["[None]", "Mist", "Fog"]);
}

/// A named color whose name a gradient or pattern swatch of the palette has takes the first free
/// "Name 2", and the fills and gradient stops linked to it follow it.
#[test]
fn a_color_named_as_a_palette_gradient_or_pattern_is_numbered() {
    let gradient = "%AI5_BeginGradient: (Brand)\n(Brand) 0 2 Bd\n[\n\
                    %_0 1 1 0 (Brand) 0 0 5 1 6 50 0 Bs\n%_0 0 0 0 1 0 0 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let palette = "%AI5_BeginPalette\n0 0 Pb\nBb\n2 (Brand) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Brand)\nPc\n\
                   (Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n(Ink)\nPc\nPB\n%AI5_EndPalette\n";
    let colored = |name: &str| format!("0 1 1 0 ({name}) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let doc = read_ok(&format!("{gradient}{DOTS}{palette}{}", layer(&format!("{}{}", colored("Brand"), colored("Ink"))))).doc;
    let paints = |name: &str| doc.swatches_iter().find(|s| s.name == name).map(|s| s.paint.clone());
    assert!(matches!(paints("Brand"), Some(Paint::Gradient(_))));
    assert!(matches!(paints("Ink"), Some(Paint::Pattern { .. })));
    for name in ["Brand 2", "Ink 2"] {
        let sw = doc.swatches_iter().find(|s| s.name == name).unwrap_or_else(|| panic!("{name}"));
        assert!(matches!(sw.paint, Paint::Solid { .. }) && sw.global && sw.spot, "{sw:?}");
    }
    let links: Vec<_> = art(&doc).iter().map(|n| if let Paint::Solid { swatch, .. } = fill(n) { swatch } else { None }).collect();
    assert_eq!(links, [Some("Brand 2".to_string()), Some("Ink 2".to_string())]);
    let Some(Paint::Gradient(g)) = paints("Brand") else { panic!() };
    assert_eq!(g.gradient.stops[0].swatch.as_deref(), Some("Brand 2"), "the stop's link");
    // A color defined later under the numbered name keeps it and its links.
    let later = format!("0 0 1 0 (Brand 2) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let doc = read_ok(&format!("{gradient}{DOTS}{palette}{}", layer(&format!("{}{later}", colored("Brand"))))).doc;
    let color = |name: &str| doc.swatches_iter().find(|s| s.name == name).and_then(|s| s.paint.color());
    assert_eq!(color("Brand 2"), Some(Color::cmyk(0.0, 0.0, 1.0, 0.0)));
    assert_eq!(color("Brand 3"), Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)));
    let links: Vec<_> = art(&doc).iter().map(|n| if let Paint::Solid { swatch, .. } = fill(n) { swatch } else { None }).collect();
    assert_eq!(links, [Some("Brand 3".to_string()), Some("Brand 2".to_string())]);
}

/// A palette's gradient swatch holds its gradient without a link to a swatch, whether it is named
/// by its gradient or has a name of its own, and the art filled with the gradient links to the
/// swatch named by the gradient.
#[test]
fn a_palette_gradient_swatch_is_not_linked_to_itself() {
    let gradient = "%AI5_BeginGradient: (Sky)\n(Sky) 0 2 Bd\n[\n%_1 0 50 0 Bs\n%_0 0 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let palette = "%AI5_BeginPalette\n0 0 Pb\nBb\n2 (Sky) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Sky)\nPc\n\
                   Bb\n2 (Sky) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n(Brand)\nPc\nPB\n%AI5_EndPalette\n";
    let filled = format!("Bb\n1 (Sky) 0 0 0 10 1 0 0 1 0 0 Bg\n{}f\n0 BB\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&format!("{gradient}{palette}{}", layer(&filled))).doc;
    for name in ["Sky", "Brand"] {
        let sw = doc.swatch(name).unwrap_or_else(|| panic!("no {name} swatch"));
        let Paint::Gradient(g) = &sw.paint else { panic!("{sw:?}") };
        assert_eq!(g.swatch, None, "{name}'s own paint");
        assert_eq!(g.geom, None);
        assert_eq!(g.gradient.stops.len(), 2);
    }
    assert!(matches!(fill(&art(&doc)[0]), Paint::Gradient(g) if g.swatch.as_deref() == Some("Sky")));
}

/// A palette's pattern swatch named by its pattern takes the name the pattern is shown with.
#[test]
fn a_palette_pattern_swatch_takes_its_patterns_shown_name() {
    let palette = "%AI5_BeginPalette\n0 0 Pb\n(Dots\\n1) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\nPc\nPB\n%AI5_EndPalette\n";
    let doc = read_ok(&format!("{}{palette}{}", DOTS.replace("(Dots)", "(Dots\\n1)"), layer(""))).doc;
    let names: Vec<&str> = doc.swatches.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["[None]", "Dots"]);
    assert!(matches!(&doc.swatch("Dots").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Dots"));
}

/// Where a definition draws its tile doesn't move the fill: the tile's lower left is the pattern's
/// origin, so the same dots drawn at 30..38 × 40..48 tile as they do at 0..8 × 0..8.
#[test]
fn a_pattern_tiles_from_its_lower_left_wherever_its_definition_drew_it() {
    let moved = DOTS
        .replace("(Dots) 0 0 8 8", "(Dots) 30 40 38 48")
        .replace("%_0 8 m\n%_8 8 L\n%_8 0 L\n%_0 0 L\n%_0 8 L", "%_30 48 m\n%_38 48 L\n%_38 40 L\n%_30 40 L\n%_30 48 L")
        .replace("%_4 4 m\n%_4 4 L", "%_34 44 m\n%_34 44 L");
    let body = format!("{moved}{}", layer(&format!("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 20.0, 70.0, 80.0))));
    let s = read_ok(&body);
    assert_eq!(s.doc.pattern("Dots").unwrap().tile, Rect::new(30.0, -48.0, 38.0, -40.0));
    let Paint::Pattern { xf, .. } = fill(&art(&s.doc)[0]) else { panic!() };
    assert_eq!(xf * Point::ZERO, Point::new(0.0, 92.0));
}

/// A gradient in a pattern's definition is placed by an `Xm` written y down about the canvas point of
/// art space's origin (here 7941, 7941: the canvas's centre less the template box's): its ramp from
/// (10, 0) up to (10, 20) in the pattern's space is `0 -20 20 0 -7931 7941 Xm`.
#[test]
fn a_gradient_in_a_patterns_definition_is_placed_from_the_canvas() {
    let defs =
        "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 0 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let tile = rect_path(0.0, 0.0, 20.0, 20.0).replace('\n', "\n%_");
    let pattern = format!(
        "%AI3_BeginPattern: (Shade)\n(Shade) 0 0 20 20\n%_0 A\n%_0 Xw\n%_0 Ae\n%_u\n%_Bb\n%_1 (G) 0 0 90 1 1 0 0 1 0 0 1 Bg\n\
         %_0 -20 20 0 -7931 7941 Xm\n%_{tile}f\n%_0 BB\n%_U\nE\n%AI3_EndPattern\n"
    );
    let file = stream(&format!("{defs}{pattern}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))))
        .replace("%%EndComments", "%AI3_TemplateBox: 250.5 -250.5 250.5 -250.5\n%%EndComments");
    let doc = read(file.as_bytes()).unwrap().doc;
    let def = doc.pattern("Shade").unwrap();
    let shaded = &def.art[0].children().unwrap()[0];
    let Paint::Gradient(g) = fill(shaded) else { panic!("{:?}", fill(shaded)) };
    let geom = g.geom.unwrap();
    // The pattern's space is y down.
    assert!(geom.start.distance(Point::new(10.0, 0.0)) < 1e-9 && geom.end.distance(Point::new(10.0, -20.0)) < 1e-9, "{geom:?}");
}

/// A pattern whose definition it can't read whole isn't kept: a fill with it is something the file
/// has that isn't read, where it shows. Its definition costs nothing else.
#[test]
fn a_pattern_it_cant_read_is_an_error_only_where_it_fills() {
    let unread = "%AI3_BeginPattern: (Old)\n(Old) 0 0 8 8 [\n(0 O 0 R 0 g) @\n(\n0 0 m 8 8 L frobnicate S\n) &\n] E\n%AI3_EndPattern\n\
                  %AI3_BeginPattern: (Odd)\n(Odd) 0 0 8 8\n%_U\nE\n%AI3_EndPattern\n%AI3_BeginPattern: (Flat)\n(Flat) 0 0 8 0\n%_0 0 m\n%_8 0 L\n%_S\nE\n%AI3_EndPattern\n";
    let s = read_ok(&format!("{unread}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
    assert!(s.doc.patterns.is_empty() && s.doc.swatches.iter().all(|w| !matches!(w.paint, Paint::Pattern { .. })));
    for name in ["Old", "Odd", "Flat"] {
        let r = read(
            stream(&format!("{unread}{}", layer(&format!("({name}) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(0.0, 0.0, 1.0, 1.0)))))
                .as_bytes(),
        )
        .unwrap();
        assert_unread(&r, "pattern fills");
    }
    // Type in a pattern isn't read either.
    let typed = "%AI3_BeginPattern: (Words)\n(Words) 0 0 8 8\n%_/AI11Text :\n%_0 /StoryIndex ,\n%_;\nE\n%AI3_EndPattern\n";
    assert!(read_ok(&format!("{typed}{}", layer(""))).doc.patterns.is_empty());
}

/// A pattern whose definition paints with a pattern it doesn't have isn't kept, on `%_` lines or
/// not: a fill with it is something the file has that isn't read.
#[test]
fn a_pattern_painting_with_an_unread_pattern_is_not_kept() {
    for prefix in ["%_", ""] {
        let tile = rect_path(0.0, 0.0, 8.0, 8.0).replace('\n', &format!("\n{prefix}"));
        let def = format!(
            "%AI3_BeginPattern: (Nest)\n(Nest) 0 0 8 8\n{prefix}(Missing) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{prefix}{tile}f\nE\n%AI3_EndPattern\n"
        );
        let s = read_ok(&format!("{def}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
        assert!(s.doc.patterns.is_empty(), "{prefix:?}: {:?}", s.doc.patterns.iter().map(|p| &p.name).collect::<Vec<_>>());
        let r = read(
            stream(&format!("{def}{}", layer(&format!("(Nest) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(0.0, 0.0, 1.0, 1.0))))).as_bytes(),
        )
        .unwrap();
        assert_unread(&r, "pattern fills");
        // A color set before the tile is drawn replaces the missing pattern, and the pattern is kept.
        let repainted = def.replace(&format!("{prefix}{tile}f"), &format!("{prefix}0 g\n{prefix}{tile}f"));
        let s = read_ok(&format!("{repainted}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
        assert!(s.doc.pattern("Nest").is_some(), "{prefix:?}");
    }
}

/// A pattern's definition inside another's isn't read, so the outer one isn't kept either.
#[test]
fn a_pattern_with_a_definition_inside_it_isnt_kept() {
    let tile = rect_path(0.0, 0.0, 8.0, 8.0).replace('\n', "\n%_");
    let inner = "%AI3_BeginPattern: (In)\n(In) 0 0 1 1\n%_0 0 m\n%_1 1 L\n%_S\nE\n%AI3_EndPattern\n";
    let def = format!("%AI3_BeginPattern: (Out)\n(Out) 0 0 8 8\n{inner}%_{tile}f\nE\n%AI3_EndPattern\n");
    let s = read_ok(&format!("{def}{}", layer("")));
    assert!(s.doc.patterns.is_empty(), "{:?}", s.doc.patterns.iter().map(|p| &p.name).collect::<Vec<_>>());
    // Without the definition inside it, it is kept.
    let s = read_ok(&format!("{}{}", def.replace(inner, ""), layer("")));
    assert!(s.doc.pattern("Out").is_some());
}

/// A definition whose art follows its head as operators is still read when that art starts with a
/// dash array (`[]0 d`) off `%_` lines, which isn't a version 7 layer list.
#[test]
fn a_pattern_whose_art_starts_with_a_dash_array_is_read() {
    let def = format!("%AI3_BeginPattern: (Dash)\n(Dash) 0 0 8 8 []0 d\n{}f\nE\n%AI3_EndPattern\n", rect_path(0.0, 0.0, 8.0, 8.0));
    let s = read_ok(&format!("{def}{}", layer("")));
    assert_eq!(s.doc.pattern("Dash").map(|p| p.art.len()), Some(1));
}

/// Editing data in the version 7 form: `%AI5_FileFormat 2.0`, the art size around the template
/// box's center (an artboard 200 by 100 from the origin), `setup` (pattern definitions), then `body`.
fn pattern_stream7(setup: &str, body: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 2.0\n%AI5_ArtSize: 200 100\n%AI3_TemplateBox: 100 50 100 50\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n{setup}%%EndSetup\n{body}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
}

/// One layer named `name` with the version 7 form's ten operands before `Lb`, holding `art`.
fn pattern_layer7(name: &str, art: &str) -> String {
    format!("%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n({name}) Ln\n0 A\n{art}LB\n%AI5_EndLayer--\n")
}

/// A definition of `name` in the version 7 form: its 20-point tile, then `layers`, the layer list.
fn pattern7(name: &str, layers: &str) -> String {
    format!("%AI3_BeginPattern: ({name})\n({name}) 0 0 20 20 [\n%AI3_Tile\n{layers}] E\n%AI3_EndPattern\n")
}

fn pattern_read7(setup: &str, body: &str) -> Structure {
    match read(pattern_stream7(setup, body).as_bytes()) {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    }
}

/// A definition in the version 7 form (§3.4): each layer's colors (`@`) paint its tile's art (`&`),
/// the first layer the lowest. The operator lines in the strings end in CR, and each layer starts
/// from the definition's state, so the second layer's stroke doesn't take the first one's width.
#[test]
fn version_7_pattern_layers_paint_in_order_with_their_own_colors() {
    let layers = "(0 O 0 R 1 0 0 0 k\r1 0 0 0 K) @\n(\r%AI6_BeginPatternLayer\r0 J 0 j 3 w 4 M []0 d\r2 2 m\r9 2 L\r9 9 L\r2 9 L\rf\r%AI6_EndPatternLayer\r) &\n\
                  (0 O 1 R 0 0 0 1 k\r0 0 0 1 K) @\n(\r2 12 m\r18 12 L\rS\r) &\n";
    let fill_with = format!("(Grid) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 10.0, 90.0, 90.0));
    let s = pattern_read7(&pattern7("Grid", layers), &pattern_layer7("Art", &fill_with));
    assert!(s.unread.is_empty() && s.warnings.is_empty(), "{:?} {:?}", s.unread, s.warnings);
    let def = s.doc.pattern("Grid").unwrap();
    assert_eq!(def.tile, Rect::new(0.0, -20.0, 20.0, 0.0), "the tile, y down");
    assert_eq!(def.art.len(), 2, "one path for each layer");
    let (square, line) = (&def.art[0], &def.art[1]);
    assert_eq!(square.geometric_bounds(), Some(Rect::new(2.0, -9.0, 9.0, -2.0)));
    assert_eq!(fill(square).color(), Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)));
    assert!(square.appearance.stroke().is_none());
    assert!(line.appearance.fill().is_none());
    let stroke = line.appearance.stroke().unwrap();
    assert_eq!(stroke.paint.color(), Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    assert_eq!(stroke.width, 1.0, "the first layer's width stays with it");
    assert!(stroke.overprint);
    assert!(matches!(fill(&art(&s.doc)[0]), Paint::Pattern { pattern, .. } if pattern == "Grid"));
}

/// `_ &` fills the tile's box with the layer's fill color, without a stroke (§3.4).
#[test]
fn a_version_7_patterns_underbar_fills_the_tile_box() {
    let layers = "(0 O 0 R 0 1 0 0 k 0 0 0 1 K) @ _ &\n(0 0 0 1 k 0 0 0 1 K) @ (2 2 m 18 18 L S) &\n";
    let s = pattern_read7(&pattern7("Back", layers), &pattern_layer7("Art", ""));
    let def = s.doc.pattern("Back").unwrap();
    let back = &def.art[0];
    assert_eq!(back.geometric_bounds(), Some(def.tile));
    assert_eq!(fill(back).color(), Some(Color::cmyk(0.0, 1.0, 0.0, 0.0)));
    assert!(back.appearance.stroke().is_none());
    assert!(def.art[1].appearance.stroke().is_some());
}

/// A version 7 pattern shown as the name of a custom color its own layer strings use is numbered:
/// the color keeps its swatch.
#[test]
fn a_version_7_pattern_shown_as_its_colors_name_is_numbered() {
    let layers = "(0 O 0 R 0 0 0 1 \\(Ink\\) 0 x) @ (2 2 m 9 2 L 9 9 L f) &\n";
    let s = pattern_read7(&pattern7("Ink\\n1", layers), &pattern_layer7("Art", ""));
    assert_eq!(s.doc.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Ink 2"]);
    assert!(matches!(&s.doc.swatch("Ink 2").unwrap().paint, Paint::Pattern { pattern, .. } if pattern == "Ink 2"));
    assert!(matches!(s.doc.swatch("Ink").unwrap().paint, Paint::Solid { .. }));
}

/// A layer's strings are read with their escapes undone: `\r`, `\n`, an octal tab, a backslash at
/// a line's end, and escaped parentheses around a custom color's name.
#[test]
fn a_version_7_patterns_strings_are_read_with_their_escapes_undone() {
    let layers = "(0 O 0 R 0 0 0 1 \\(Ink \\(A\\)\\) 0 x) @\n(2 2 m\\r9 2 L\\n9 9 L\\0112 9 \\\nL f) &\n";
    let s = pattern_read7(&pattern7("Ink", layers), &pattern_layer7("Art", ""));
    let def = s.doc.pattern("Ink").unwrap();
    assert_eq!(def.art.len(), 1);
    assert_eq!(def.art[0].geometric_bounds(), Some(Rect::new(2.0, -9.0, 9.0, -2.0)));
    assert!(s.doc.swatch("Ink (A)").is_some(), "{:?}", s.doc.swatches.iter().map(|w| &w.name).collect::<Vec<_>>());
}

/// A definition in the version 7 form that can't be read whole isn't kept, and a fill with it is
/// something the file has that isn't read: an operator it doesn't know, type, a definition inside
/// it, colors with other operators, a layer without its tile or its `&`, a string longer than the
/// format allows, an `E`, a group or an array a layer doesn't end, more than 65,536 layers, a list
/// without its `]` or its `E`, anything after its `E`.
#[test]
fn a_version_7_pattern_it_cant_read_whole_isnt_kept() {
    let line = "(0 0 m 8 8 L S) &\n";
    let long = format!("(0 g) @ ({}0 0 m 8 8 L S) &\n", " ".repeat(65_536));
    let open = "%AI3_BeginPattern: (P)\n(P) 0 0 20 20 [\n(0 g) @ (0 0 m 8 8 L S) &\n";
    let defs = [
        pattern7("P", "(0 g) @ (0 0 m 8 8 L frobnicate S) &\n"),
        pattern7("P", "(0 g) @ (0 To 1 0 0 1 2 2 0 Tp TP (a) Tx TO) &\n"),
        pattern7("P", "(0 g) @ (\r%AI3_BeginPattern: (In)\r(In) 0 0 1 1\rE\r%AI3_EndPattern\r0 0 m 8 8 L S) &\n"),
        pattern7("P", &format!("%AI3_BeginPattern: (In)\n(In) 0 0 1 1 [\n] E\n%AI3_EndPattern\n(0 g) @ {line}")),
        pattern7("P", &format!("(0 g 0 0 m) @ {line}")),
        pattern7("P", "(0 g) @\n"),
        pattern7("P", "(0 g) @ _\n"),
        pattern7("P", &long),
        pattern7("P", "(0 g) @ (0 0 m 8 8 L S E) &\n"),
        pattern7("P", "(0 g) @ (u 0 0 m 8 8 L S) &\n"),
        pattern7("P", "(0 g) @ (0 0 m 8 8 L S [) &\n"),
        pattern7("P", "(0 g) @ ([ 0 0 m 8 8 L S) &\n"),
        pattern7("P", &"(0 g) @ _ &\n".repeat(65_537)),
        format!("{open}E\n%AI3_EndPattern\n"),
        format!("{open}]\n%AI3_EndPattern\n"),
        format!("{open}] E\n0 0 m 8 8 L S\n%AI3_EndPattern\n"),
    ];
    let fill_with = format!("(P) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(0.0, 0.0, 1.0, 1.0));
    for (i, def) in defs.iter().enumerate() {
        assert!(pattern_read7(def, &pattern_layer7("Art", "")).doc.patterns.is_empty(), "case {i}");
        assert_unread(&pattern_read7(def, &pattern_layer7("Art", &fill_with)), "pattern fills");
    }
    // The same definition, whole, is kept.
    let s = pattern_read7(&pattern7("P", &format!("(0 g) @ {line}")), &pattern_layer7("Art", &fill_with));
    assert!(s.doc.pattern("P").is_some() && s.unread.is_empty());
}

/// With a pattern defined in the version 7 form, a fill's other operands are applied in order
/// before its initial matrix (§5.4.1): the shear along its axis, the reflection about the line at
/// its angle, the rotation, the scale, then the offset. Identity ones move the tiles by the offset
/// alone. `P` places a stroke's tiles the same way.
#[test]
fn a_pattern_fill_applies_its_offset_scale_rotation_reflection_and_shear() {
    let def = pattern7("Grid", "(0 g) @ _ &\n");
    // The operands, and the pattern's space → art space they make (a b c d e f).
    let cases = [
        ("10 15 1 1 0 0 0 0 0 [1 0 0 1 0 0]", [1.0, 0.0, 0.0, 1.0, 10.0, 15.0]),
        ("25 30 1.5 .75 30 1 45 10 0 [1 0 0 1 0 0]", [-0.75, 0.649519, 1.166793, 0.489528, 25.0, 30.0]),
        ("-5 7 2 .5 -45 0 0 -15 45 [1 0 0 1 0 0]", [1.793151, -0.353553, 1.035276, 0.353553, -5.0, 7.0]),
        ("3 -4 1.2 .8 20 1 30 12 25 [.8 .1 -.2 1.1 4 -6]", [-0.028557, 0.816531, 1.054026, 0.117492, 7.2, -10.1]),
    ];
    // Art space → the document (the artboard is 100 high), and a tile's art (y down from its top
    // left) → the pattern's space.
    let space = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 100.0]);
    let tile = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 20.0]);
    for (ops, want) in cases {
        for (op, paint) in [("p", "f"), ("P", "S")] {
            let s = pattern_read7(&def, &pattern_layer7("Art", &format!("(Grid) {ops} {op}\n{}{paint}\n", rect_path(10.0, 10.0, 90.0, 90.0))));
            let node = &art(&s.doc)[0];
            let p = if op == "p" { fill(node) } else { node.appearance.stroke().unwrap().paint.clone() };
            let Paint::Pattern { xf, .. } = p else { panic!("{ops} {op}: {p:?}") };
            let got = (space.inverse() * xf * tile.inverse()).as_coeffs();
            assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6), "{ops} {op}: {got:?}");
        }
    }
    // The tiles' corners: with identity operands but the offset, the tile's top left (0, 20) is at
    // (10, 35) in art space; with the last ones, its lower right (20, 0) is at (6.628860, 6.230623).
    let placed = |ops: &str| {
        let s = pattern_read7(&def, &pattern_layer7("Art", &format!("(Grid) {ops} p\n{}f\n", rect_path(10.0, 10.0, 90.0, 90.0))));
        let Paint::Pattern { xf, .. } = fill(&art(&s.doc)[0]) else { panic!() };
        xf
    };
    assert_eq!(placed(cases[0].0) * Point::ZERO, Point::new(10.0, 65.0));
    assert!((placed(cases[3].0) * Point::new(20.0, 20.0)).distance(Point::new(6.628860, 93.769377)) < 1e-5);
    // Without its nine operands, a fill with the pattern isn't read.
    let r = pattern_read7(&def, &pattern_layer7("Art", &format!("(Grid) 0 0 1 1 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(10.0, 10.0, 90.0, 90.0))));
    assert_unread(&r, "pattern fills");
}

/// With a pattern defined in the newer form (art on `%_` lines), the matrix alone places the tiles
/// (#1025): the other operands don't move them, and a fill without all nine is still read.
#[test]
fn a_newer_pattern_fill_is_placed_by_its_matrix_alone() {
    let placed = |ops: &str| {
        let s = read_ok(&format!("{DOTS}{}", layer(&format!("(Dots) {ops} p\n{}f\n", rect_path(10.0, 20.0, 70.0, 80.0)))));
        assert!(s.unread.is_empty() && s.warnings.is_empty(), "{ops}: {:?} {:?}", s.unread, s.warnings);
        let Paint::Pattern { xf, .. } = fill(&art(&s.doc)[0]) else { panic!("{ops}: {:?}", fill(&art(&s.doc)[0])) };
        xf
    };
    let plain = placed("0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0]");
    assert_eq!(plain * Point::ZERO, Point::new(0.0, 92.0));
    assert_eq!(placed("25 30 1.5 .75 30 1 45 10 0 [1 0 0 1 0 0]"), plain);
    assert_eq!(placed("0 0 1 1 0 0 0 0 [1 0 0 1 0 0]"), plain);
}

/// An art style `Tiled` whose fill is `(name) {ops} p`.
fn tiled_style(name: &str, ops: &str) -> String {
    format!(
        "%AI9_BeginArtStyles\n/KnownStyle :\n(Tiled) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n({name}) {ops} p\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n;\n%AI9_EndArtStyles\n"
    )
}

/// A fill set through an art style on a text object places the tiles as a `p` fill with the same
/// operands does: with the other operands before the matrix for a pattern defined in the version 7
/// form, by the matrix alone for one defined in the newer form.
#[test]
fn a_style_pattern_fill_is_placed_as_a_direct_one() {
    let rect = rect_path(10.0, 20.0, 70.0, 80.0);
    // The text object's drawn look (without a pattern to take the placement from), then the object.
    let look = "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\nU\n";
    let own = "%_/AI11Text :\n%_0 /FreeUndo ,\n%_0 /FrameIndex ,\n%_0 /StoryIndex ,\n%_;\n";
    let ops = "25 30 1.5 .75 30 1 45 10 0 [.8 .1 -.2 1.1 4 -6]";
    for (name, def) in [("Grid", pattern7("Grid", "(0 g) @ _ &\n")), ("Dots", DOTS.to_string())] {
        let styled = read_ok(&format!("{}{def}{}", tiled_style(name, ops), layer(&format!("{look}{own}1 (Tiled) XW\n"))));
        let direct = read_ok(&format!("{def}{}", layer(&format!("({name}) {ops} p\n{rect}f\n"))));
        let xf = |s: &Structure| match fill(&art(&s.doc)[0]) {
            Paint::Pattern { xf, .. } => xf,
            p => panic!("{name}: {p:?} {:?} {:?}", s.unread, s.warnings),
        };
        let (got, want) = (xf(&styled).as_coeffs(), xf(&direct).as_coeffs());
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-9), "{name}: {got:?} {want:?}");
    }
}

/// An art style's pattern fill paints with the pattern under the name it is shown with.
#[test]
fn a_style_pattern_fill_paints_with_the_shown_name() {
    let look = "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\nU\n";
    let own = "%_/AI11Text :\n%_0 /FreeUndo ,\n%_0 /FrameIndex ,\n%_0 /StoryIndex ,\n%_;\n";
    let defs = format!("{}{}", pattern7("Grid", "(0 g) @ _ &\n"), pattern7("Grid\\n1", "(0 g) @ _ &\n"));
    let s = read_ok(&format!("{}{defs}{}", tiled_style("Grid\\n1", "0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0]"), layer(&format!("{look}{own}1 (Tiled) XW\n"))));
    let pattern = |n: &Node| match fill(n) {
        Paint::Pattern { pattern, .. } => pattern,
        p => panic!("{p:?} {:?} {:?}", s.unread, s.warnings),
    };
    assert_eq!(pattern(&art(&s.doc)[0]), "Grid 2");
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
    // A clipping path with a fill keeps it, and paints it under the art it clips (`Node::clip_paint`); without one,
    // nothing.
    let blue = Some(Color::rgb(0.0, 0.0, 1.0));
    let filled = format!("0 Ae\nq\n{RED}{}f\n0 0 1 0 0 0 1 Xa\n{}W\nf\nQ\n", rect_path(0.0, 0.0, 30.0, 30.0), rect_path(5.0, 5.0, 25.0, 25.0));
    let doc = read_ok(&layer(&filled)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    let kinds: Vec<_> = children.iter().map(|c| (matches!(c.kind, NodeKind::Path { clipping: true, .. }), fill(c).color())).collect();
    assert_eq!(kinds, [(true, blue), (false, Some(Color::rgb(1.0, 0.0, 0.0)))]);
    assert_eq!(children[0].clip_paint().fill.map(|f| fill(&f).color()), Some(blue));
    // A clipping path in a layer clips the layer.
    let doc = read_ok(&layer(&format!("{}f\n{}W\nn\n", rect_path(0.0, 0.0, 9.0, 9.0), rect_path(0.0, 0.0, 5.0, 5.0)))).doc;
    let NodeKind::Layer { clip, children, .. } = &doc.layers[0].kind else { panic!() };
    assert!(*clip && matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    // A filled clipping path in a layer keeps its fill too.
    let doc = read_ok(&layer(&format!("{RED}{}f\n0 0 1 0 0 0 1 Xa\n{}W\nf\n", rect_path(0.0, 0.0, 9.0, 9.0), rect_path(0.0, 0.0, 5.0, 5.0)))).doc;
    let NodeKind::Layer { clip: true, children, .. } = &doc.layers[0].kind else { panic!("{:?}", doc.layers[0].kind) };
    let kinds: Vec<_> = children.iter().map(|c| (matches!(c.kind, NodeKind::Path { clipping: true, .. }), fill(c).color())).collect();
    assert_eq!(kinds, [(true, blue), (false, Some(Color::rgb(1.0, 0.0, 0.0)))]);
    // A compound clipping path keeps its subpaths' fill.
    let compound = format!(
        "0 Ae\nq\n{RED}{}f\n0 0 1 0 0 0 1 Xa\n*u\n{}h\nW\nf\n{}h\nW\nf\n*U\nQ\n",
        rect_path(0.0, 0.0, 30.0, 30.0),
        rect_path(5.0, 5.0, 25.0, 25.0),
        rect_path(10.0, 10.0, 20.0, 20.0)
    );
    let doc = read_ok(&layer(&compound)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children.len() == 2, "{children:?}");
    assert_eq!((fill(&children[0]).color(), fill(&children[1]).color()), (blue, Some(Color::rgb(1.0, 0.0, 0.0))));
    // One subpath filled and a later one stroked: the compound clipping path keeps the stroke, and the filled subpath's
    // fill is a compound path of its own under the art, so the stroked subpath stays unfilled.
    let mixed = format!(
        "0 Ae\nq\n{RED}{}f\n0 0 1 0 0 0 1 Xa\n0 1 0 0 0 1 0 XA\n3 w\n*u\n{}h\nW\nf\n{}h\nW\nS\n*U\nQ\n",
        rect_path(0.0, 0.0, 30.0, 30.0),
        rect_path(5.0, 5.0, 25.0, 25.0),
        rect_path(10.0, 10.0, 20.0, 20.0)
    );
    let doc = read_ok(&layer(&mixed)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children.len() == 3, "{children:?}");
    let stroke = children[0].appearance.stroke().map(|s| (s.paint.color(), s.width));
    assert_eq!((fill(&children[0]).color(), stroke), (None, Some((Some(Color::rgb(0.0, 1.0, 0.0)), 3.0))));
    let NodeKind::Compound { children: filled, .. } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert_eq!(filled.len(), 1, "only the filled subpath: {filled:?}");
    assert!(filled.iter().all(|p| matches!(p.kind, NodeKind::Path { clipping: false, .. })), "{filled:?}");
    assert_eq!((fill(&children[1]).color(), children[1].appearance.stroke().is_none()), (blue, true));
    assert_eq!(fill(&children[2]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    // A subpath that doesn't clip keeps its fill; only the clipping subpath's fill is read apart.
    let green = Some(Color::rgb(0.0, 1.0, 0.0));
    let plain = format!(
        "0 Ae\nq\n{RED}{}f\n*u\n0 0 1 0 0 0 1 Xa\n{}f\n0 1 0 0 0 1 0 Xa\n{}h\nW\nf\n*U\nQ\n",
        rect_path(0.0, 0.0, 30.0, 30.0),
        rect_path(5.0, 5.0, 25.0, 25.0),
        rect_path(10.0, 10.0, 20.0, 20.0)
    );
    let doc = read_ok(&layer(&plain)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    let NodeKind::Compound { children: subpaths, .. } = &children[0].kind else { panic!("{:?}", children[0].kind) };
    let kinds: Vec<_> = subpaths.iter().map(|c| (matches!(c.kind, NodeKind::Path { clipping: true, .. }), fill(c).color())).collect();
    assert_eq!(kinds, [(false, blue), (true, None)]);
    let NodeKind::Compound { children: apart, .. } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert_eq!((apart.len(), fill(&children[1]).color()), (1, green), "only the clipping subpath's fill: {apart:?}");
    // A tag after the compound clipping path is its own.
    let doc = read_ok(&layer(&mixed.replace("*U\n", "*U\n/edge (clip) XT\n"))).doc;
    let NodeKind::Group { children, .. } = &art(&doc)[0].kind else { panic!("a clipping group") };
    assert_eq!((tags(&children[0]).1, tags(&children[1]).1), (vec![tag("edge", "clip")], vec![]));
}

/// A stroked clipping path (`W S`, `W B`, or a gradient fill with `1 BB`) keeps its stroke and its fill on the clipping
/// path.
#[test]
fn a_clipping_paths_stroke_stays_on_the_clipping_path() {
    let blue = Some(Color::rgb(0.0, 0.0, 1.0));
    let red = Some(Color::rgb(1.0, 0.0, 0.0));
    // (clipping, fill colour, a gradient fill, stroke colour and width) of each of the clipping group's children.
    let children = |doc: &Document| {
        let NodeKind::Group { children, clip: true } = &art(doc)[0].kind else { panic!("{:?}", art(doc)[0].kind) };
        children
            .iter()
            .map(|c| {
                let clipping = matches!(c.kind, NodeKind::Path { clipping: true, .. });
                (clipping, fill(c).color(), matches!(fill(c), Paint::Gradient(_)), c.appearance.stroke().map(|s| (s.paint.color(), s.width)))
            })
            .collect::<Vec<_>>()
    };
    let under = rect_path(0.0, 0.0, 30.0, 30.0);
    let clip = rect_path(5.0, 5.0, 25.0, 25.0);
    let stroked = format!("0 Ae\nq\n{RED}{under}f\n0 0 1 0 0 0 1 XA\n3 w\n{clip}W\nS\nQ\n");
    assert_eq!(children(&read_ok(&layer(&stroked)).doc), [(true, None, false, Some((blue, 3.0))), (false, red, false, None)]);
    let both = format!("0 Ae\nq\n{RED}{under}f\n0 1 0 0 0 1 0 Xa\n0 0 1 0 0 0 1 XA\n3 w\n{clip}W\nB\nQ\n");
    assert_eq!(children(&read_ok(&layer(&both)).doc), [(true, Some(Color::rgb(0.0, 1.0, 0.0)), false, Some((blue, 3.0))), (false, red, false, None)]);
    let defs = "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n0 0 0 0 1 0 0 2 0 6 %_BS\n%_0 0 0 0 1 0 0 2 0 6 0 0 Bs\n\
                %_0 0 0 0 0 0 1 2 1 6 100 50 Bs\nBD\n%AI5_EndGradient\n";
    let gradient = format!("0 Ae\nq\n{RED}{under}f\n0 0 1 0 0 0 1 XA\n3 w\n{clip}W\nBb\n0 0 0 0 Bh\n1 (G) 5 5 0 20 1 0 0 1 0 0 1 Bg\nf\n1 BB\nQ\n");
    assert_eq!(children(&read_ok(&format!("{defs}{}", layer(&gradient))).doc), [(true, None, true, Some((blue, 3.0))), (false, red, false, None)]);
    // Several clipping paths each keep their own stroke, opacity and blend mode: the second one clips a clipping group
    // inside the first one's.
    let inner = rect_path(10.0, 10.0, 20.0, 20.0);
    let strokes = |first: &str, second: &str| {
        let body = format!("0 Ae\nq\n{RED}{under}f\n0 0 1 0 0 0 1 XA\n3 w\n{first}{clip}W\nS\n{second}{inner}W\nS\nQ\n");
        let doc = read_ok(&layer(&body)).doc;
        let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
        let NodeKind::Group { children: nested, clip: true } = &children[1].kind else { panic!("{:?}", children[1].kind) };
        [&children[0], &nested[0]].map(|c| {
            assert!(matches!(c.kind, NodeKind::Path { clipping: true, .. }), "{:?}", c.kind);
            (c.appearance.stroke().map(|s| (s.paint.color(), s.width)), c.opacity, c.blend)
        })
    };
    let plain = (Some((blue, 3.0)), 1.0, BlendMode::Normal);
    assert_eq!(strokes("", ""), [plain, plain]);
    assert_eq!(strokes("", "9 0.5 0 0 0 Xy\n"), [plain, (Some((blue, 3.0)), 0.5, BlendMode::Lighten)]);
    assert_eq!(strokes("", "5 w\n"), [plain, (Some((blue, 5.0)), 1.0, BlendMode::Normal)], "strokes that differ");
}

/// Clipping paths in a compound path that holds a drawn look become one compound path, which keeps the knockout shape
/// flag when they all have it.
#[test]
fn the_compound_of_clipping_paths_keeps_their_knockout_shape() {
    let (under, over) = (rect_path(0.0, 0.0, 30.0, 30.0), rect_path(5.0, 5.0, 25.0, 25.0));
    let (a, b) = (rect_path(0.0, 0.0, 10.0, 10.0), rect_path(5.0, 0.0, 20.0, 10.0));
    let clipping = |first: &str, second: &str| {
        let look = format!("0 Ae\nu\n{RED}{under}f\n0 0 0 0 0 0 1 Xa\n{over}f\nU\n9 () XW\n");
        let doc = read_ok(&layer(&format!("0 Ae\n*u\n{look}{first}{a}h\nW\nn\n{second}{b}h\nW\nn\n*U\n"))).doc;
        let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
        assert!(matches!(children[0].kind, NodeKind::Compound { .. }), "{:?}", children[0].kind);
        (children[0].opacity, children[0].knockout_shape)
    };
    let shape = "0 0.5 0 0 1 Xy\n";
    assert_eq!(clipping(shape, ""), (0.5, true));
    assert_eq!(clipping("", ""), (1.0, false));
    assert_eq!(clipping(shape, "0 0.5 0 0 0 Xy\n"), (0.5, false), "the flag on one of them only");
    assert_eq!(clipping(shape, "0 0.8 0 0 1 Xy\n"), (1.0, true), "opacities that differ");
}

/// Several clipping paths in a clipping group, a layer or an opacity mask's art clip to their intersection: each `W`
/// reduces the clip set by the ones before it (AI7 spec §5.6.2). Each clipping path after the first clips a clipping
/// group inside the one before it, and the innermost one holds the art; a filled clipping path keeps its fill.
#[test]
fn several_clipping_paths_clip_to_their_intersection() {
    let square = rect_path(0.0, 0.0, 80.0, 80.0);
    let (a, b) = (rect_path(20.0, 20.0, 60.0, 60.0), rect_path(25.0, 25.0, 70.0, 55.0));
    // (clipping, fill color, bounds) of a clipping group's children, with a nested clipping group's children listed
    // after it.
    type Level = Vec<(bool, Option<Color>, Option<Rect>)>;
    fn levels(children: &[Arc<Node>]) -> Vec<Level> {
        let mut out = vec![];
        let mut at = children;
        loop {
            out.push(at.iter().map(|c| (matches!(c.kind, NodeKind::Path { clipping: true, .. }), fill(c).color(), c.geometric_bounds())).collect());
            let Some(NodeKind::Group { children, clip: true }) = at.last().map(|c| &c.kind) else { return out };
            at = children;
        }
    }
    let red = Some(Color::rgb(1.0, 0.0, 0.0));
    let blue = Some(Color::rgb(0.0, 0.0, 1.0));
    // A rectangle as written, in the document's coordinates (y down from the 100 pt high page's top).
    let r = |x0, y0, x1, y1| Some(Rect::new(x0, 100.0 - y1, x1, 100.0 - y0));
    let (ra, rb) = (r(20.0, 20.0, 60.0, 60.0), r(25.0, 25.0, 70.0, 55.0));
    let group = |body: &str| {
        let doc = read_ok(&layer(body)).doc;
        let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
        levels(children)
    };
    // The art after both clipping paths, in a clipping group of the second inside the first's.
    let two = group(&format!("0 Ae\nq\n{a}h\nW\nn\n{b}h\nW\nn\n{RED}{square}f\nQ\n"));
    assert_eq!((two.len(), two[0].len()), (2, 2), "{two:?}");
    assert_eq!(two[0][0], (true, None, ra));
    assert_eq!(two[1], [(true, None, rb), (false, red, r(0.0, 0.0, 80.0, 80.0))]);
    // Three clipping paths nest twice; a filled clipping path keeps its fill at its own level, so the fill is clipped
    // by that path and the paths before it only.
    let three =
        group(&format!("0 Ae\nq\n{a}h\nW\nn\n0 0 1 0 0 0 1 Xa\n{b}h\nW\nf\n{}h\nW\nn\n{RED}{square}f\nQ\n", rect_path(30.0, 10.0, 50.0, 90.0)));
    assert_eq!(three.len(), 3, "{three:?}");
    assert_eq!((three[1].len(), three[1][0]), (2, (true, blue, rb)), "{three:?}");
    assert_eq!(three[2], [(true, None, r(30.0, 10.0, 50.0, 90.0)), (false, red, r(0.0, 0.0, 80.0, 80.0))]);
    // As with one clipping path, every object in the group is clipped by all of them, in the order written, wherever
    // it is written.
    let small = rect_path(0.0, 0.0, 10.0, 10.0);
    let before = group(&format!("0 Ae\nq\n{RED}{small}f\n{a}h\nW\nn\n{b}h\nW\nn\n0 0 1 0 0 0 1 Xa\n{square}f\nQ\n"));
    assert_eq!(before.len(), 2, "{before:?}");
    assert_eq!(before[1], [(true, None, rb), (false, red, r(0.0, 0.0, 10.0, 10.0)), (false, blue, r(0.0, 0.0, 80.0, 80.0))]);
    let between = group(&format!("0 Ae\nq\n{RED}{small}f\n{a}h\nW\nn\n0 0 1 0 0 0 1 Xa\n{square}f\n{b}h\nW\nn\n{RED}{square}f\nQ\n"));
    assert_eq!((between.len(), between[0].len()), (2, 2), "{between:?}");
    assert_eq!(
        between[1],
        [(true, None, rb), (false, red, r(0.0, 0.0, 10.0, 10.0)), (false, blue, r(0.0, 0.0, 80.0, 80.0)), (false, red, r(0.0, 0.0, 80.0, 80.0))]
    );
    // A compound clipping path and a clipping path after it: their union, then that intersected with the second.
    let c = rect_path(30.0, 10.0, 50.0, 90.0);
    let mixed = format!("0 Ae\nq\n*u\n{a}h\nW\nn\n{b}h\nW\nn\n*U\n{c}h\nW\nn\n{RED}{square}f\nQ\n");
    let doc = read_ok(&layer(&mixed)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children.len() == 2, "{children:?}");
    assert_eq!(levels(children)[1], [(true, None, r(30.0, 10.0, 50.0, 90.0)), (false, red, r(0.0, 0.0, 80.0, 80.0))]);
    // Clipping paths alone in a group (`u <path> W n <path> W n U`) clip the clipping group around it the same way.
    let grouped = group(&format!("0 Ae\nq\n{RED}{square}f\n0 Ae\nu\n{a}h\nW\nn\n{b}h\nW\nn\nU\nQ\n"));
    assert_eq!(grouped.len(), 2, "{grouped:?}");
    assert_eq!(grouped[1], [(true, None, rb), (false, red, r(0.0, 0.0, 80.0, 80.0))]);
    // A compound path of clipping paths is still one clipping path, their union.
    let compound = format!("0 Ae\nq\n*u\n{a}h\nW\nn\n{b}h\nW\nn\n*U\n{RED}{square}f\nQ\n");
    let doc = read_ok(&layer(&compound)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Compound { .. }) && children.len() == 2, "{children:?}");
    // Two clipping paths in a layer: the layer clips by the first, a clipping group in it by the second.
    let doc = read_ok(&layer(&format!("{a}h\nW\nn\n{b}h\nW\nn\n{RED}{square}f\n"))).doc;
    let NodeKind::Layer { clip: true, children, .. } = &doc.layers[0].kind else { panic!("{:?}", doc.layers[0].kind) };
    let in_layer = levels(children);
    assert_eq!(in_layer.len(), 2, "{in_layer:?}");
    assert_eq!(in_layer[1], [(true, None, rb), (false, red, r(0.0, 0.0, 80.0, 80.0))]);
    // Two clipping paths in an opacity mask's art: the art clips by the first, a clipping group in it by the second.
    let mask_art: String = format!("{a}h\nW\nn\n{b}h\nW\nn\n1 g\n{square}f\n").lines().map(|l| format!("%_{l}\n")).collect();
    let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{mask_art}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_; (AI9_Mask) ,\n%_;\n");
    let doc = read_ok(&layer(&format!("{RED}{square}f\n{mask}"))).doc;
    let m = art(&doc)[0].extras().mask.clone().unwrap();
    let NodeKind::Group { children, clip: true } = &m.art.kind else { panic!("{:?}", m.art.kind) };
    let in_mask = levels(children);
    assert_eq!(in_mask.len(), 2, "{in_mask:?}");
    assert_eq!((in_mask[0][0].0, in_mask[0][0].2, in_mask[1][0]), (true, ra, (true, None, rb)));
    assert_eq!(in_mask[1].len(), 2, "{in_mask:?}");
    // Clipping paths are nested as far as the document's depth limit allows; past it they are refused rather than
    // merged away, in a clipping group, a layer and an opacity mask's art.
    let fits = |text: &dyn Fn(usize) -> String| (2..400).take_while(|n| read(stream(&layer(&text(*n))).as_bytes()).is_ok()).last().unwrap();
    let deep = |n: usize, last: &str| format!("0 Ae\nq\n{}{last}{RED}{square}f\nQ\n", format!("{a}h\nW\nn\n").repeat(n));
    // The layer is 1 deep, the clipping group 2, and the art in the innermost of `n` levels `n + 2`.
    let in_group = fits(&|n| deep(n, ""));
    assert_eq!(in_group, vectorcraft_doc::MAX_DEPTH - 2);
    let r = read_ok(&layer(&deep(in_group, "")));
    let NodeKind::Group { children, clip: true } = &art(&r.doc)[0].kind else { panic!("{:?}", art(&r.doc)[0].kind) };
    assert_eq!(levels(children).len(), in_group, "{:?}", r.warnings);
    assert!(read(stream(&layer(&deep(400, ""))).as_bytes()).unwrap_err().contains("nested too deeply"));
    let in_layer = fits(&|n| format!("{}{RED}{square}f\n", format!("{a}h\nW\nn\n").repeat(n)));
    assert_eq!(in_layer, vectorcraft_doc::MAX_DEPTH - 1);
    let in_mask = fits(&|n| {
        let art: String = format!("{}1 g\n{square}f\n", format!("{a}h\nW\nn\n").repeat(n)).lines().map(|l| format!("%_{l}\n")).collect();
        format!("{RED}{square}f\n%_/ArtDictionary :\n%_/Mask :\n%_X=\n{art}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_; (AI9_Mask) ,\n%_;\n")
    });
    // The masked object is 2 deep and its mask's art 3.
    assert_eq!(in_mask, vectorcraft_doc::MAX_DEPTH - 3);
    // A compound clipping path holds its subpaths one level down: as the innermost one, it adds a level.
    let compound_last = format!("*u\n{a}h\nW\nn\n{b}h\nW\nn\n*U\n");
    assert!(read(stream(&layer(&deep(in_group - 1, &compound_last))).as_bytes()).unwrap_err().contains("nested too deeply"));
}

/// A clipping group's clipping path can sit alone in a group of its own, after the art it clips (`q … u <path> W n U
/// … Q`): it still clips the clipping group.
#[test]
fn a_clipping_path_alone_in_a_group_clips_its_clipping_group() {
    let body = format!("0 Ae\nq\n{RED}{}f\n0 Ae\nu\n{}h\nW\nn\nU\nQ\n", rect_path(0.0, 0.0, 30.0, 30.0), rect_path(5.0, 5.0, 25.0, 25.0));
    let doc = read_ok(&layer(&body)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
    assert_eq!(children.len(), 2, "{children:?}");
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert_eq!(fill(&children[1]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

/// #1410: a compound path shows its last path's paint, and paths grouped in it stay its paths.
#[test]
fn compound_path_takes_its_last_paths_paint() {
    let paths = format!("0 g\n{}f\n{RED}{}f\n", rect_path(0.0, 0.0, 30.0, 30.0), rect_path(10.0, 10.0, 20.0, 20.0));
    let bodies = [
        format!("0 Ae\n*u\n{paths}*U\n"),
        // The paths in a group.
        format!("0 Ae\n*u\n0 Ae\nu\n{paths}U\n*U\n"),
        // The last path unpainted.
        format!("*u\n{paths}{}n\n*U\n", rect_path(1.0, 1.0, 2.0, 2.0)),
    ];
    for body in bodies {
        let a = art(&read_ok(&layer(&body)).doc);
        let NodeKind::Compound { children, .. } = &a[0].kind else { panic!("{:?}", a[0].kind) };
        assert!(children.len() >= 2 && children.iter().all(|c| matches!(c.kind, NodeKind::Path { .. })), "{body}");
        assert_eq!(fill(&a[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)), "{body}");
    }
}

#[test]
fn a_group_whose_state_has_no_transparency_takes_the_current_one() {
    // GUARDS: a group's own state block keeps the last `Xy`, as the PDF part draws it; a drawn look's group stays opaque.
    let art_ = format!(
        "{RED}0 Ae\nu\n0 0.5 0 0 0 Xy\n{}f\nU\n0 0 Xd\n6 () XW\n0 1 0 0 0 Xy\n0 Ae\nu\n0 0.5 0 0 0 Xy\n{}f\nU\n9 () XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0)
    );
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    assert_eq!((a[0].opacity, a[0].children().unwrap()[0].opacity), (0.5, 0.5));
    assert_eq!((a[1].opacity, a[1].children().unwrap()[0].opacity), (1.0, 0.5));
}

#[test]
fn a_layer_keeps_its_own_transparency() {
    // Illustrator writes a layer's own opacity, blend and knockout after its art, before `LB`
    // (`Xy`, `Xd`, then `n () XW`); they belong to the layer, not to its last object (#2003).
    let art_ = format!("{RED}{}f\n0 0.35 0 2 0 Xy\n0 0 Xd\n7 () XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&layer(&art_)).doc;
    let l = &doc.layers[0];
    assert_eq!((l.opacity, l.blend), (0.35, BlendMode::Normal), "the layer's own state");
    assert_eq!((l.isolate, l.knockout), (false, Knockout::Neutral));
    // The art inside keeps its own state.
    assert_eq!(art(&doc)[0].opacity, 1.0);
    // A layer whose last object is a group keeps that group's state apart from the layer's.
    let art_ = format!("0 Ae\nu\n0 0.5 0 0 0 Xy\n{}f\nU\n0 0 Xd\n6 () XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let doc = read_ok(&layer(&art_)).doc;
    assert_eq!(doc.layers[0].opacity, 1.0, "no layer state written: the layer stays opaque");
    assert_eq!(art(&doc)[0].opacity, 0.5, "the group's own state");
    // A layer whose last object is a gradient mesh: an `Xy` right after the mesh's end is the
    // layer's when `LB` follows (directly or after the layer's `Xd` and `7 () XW`), and the mesh's
    // when its own `6 () XW` follows.
    let corner =
        |x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| format!("%_[1 1 1 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1);
    let mesh = format!(
        "0 0.8 0 0 0 Xy\n/Mesh X!\n%_(DeviceRGB) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n/End X!\n",
        corner(0, 0, (0, 10), (10, 0)),
        corner(0, 40, (10, 40), (0, 30)),
        corner(40, 40, (40, 30), (30, 40)),
        corner(40, 0, (30, 0), (40, 10)),
    );
    for (after, layer_state, mesh_state) in [
        ("1 0.35 0 0 0 Xy\n", (0.35, BlendMode::Multiply), (0.8, BlendMode::Normal)),
        ("1 0.35 0 0 0 Xy\n0 0 Xd\n7 () XW\n", (0.35, BlendMode::Multiply), (0.8, BlendMode::Normal)),
        ("1 0.35 0 0 0 Xy\n0 0 Xd\n6 () XW\n", (1.0, BlendMode::Normal), (0.35, BlendMode::Multiply)),
        // The stroke style in the layer's state block.
        ("1 0.35 0 0 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n0 0 Xd\n7 () XW\n", (0.35, BlendMode::Multiply), (0.8, BlendMode::Normal)),
    ] {
        let doc = read_ok(&layer(&format!("{mesh}{after}"))).doc;
        let l = &doc.layers[0];
        assert!(matches!(art(&doc)[0].kind, NodeKind::Mesh(_)), "{after:?}");
        assert_eq!((l.opacity, l.blend), layer_state, "the layer's state after {after:?}");
        assert_eq!((art(&doc)[0].opacity, art(&doc)[0].blend), mesh_state, "the mesh's state after {after:?}");
    }
}

#[test]
fn a_layers_state_after_its_last_group_or_compound_path_is_the_layers() {
    // GUARDS: the layer's `Xy` right after its last group or compound path, before the layer's
    // `0 0 Xd` and `7 () XW`, went to that group or compound path, and the layer stayed opaque.
    let a = rect_path(0.0, 0.0, 10.0, 10.0);
    let b = rect_path(2.0, 2.0, 8.0, 8.0);
    let layer_block = "1 0.6 1 2 1 Xy\n0 0 Xd\n7 () XW\n";
    for last in [format!("0 Ae\nu\n{RED}{a}f\nU\n"), format!("0 Ae\n*u\n{RED}{a}f\n{b}f\n*U\n")] {
        let doc = read_ok(&layer(&format!("{last}{layer_block}"))).doc;
        let l = &doc.layers[0];
        assert_eq!(
            (l.opacity, l.blend, l.isolate, l.knockout, l.knockout_shape),
            (0.6, BlendMode::Multiply, true, Knockout::Neutral, true),
            "{last}"
        );
        let n = &art(&doc)[0];
        assert_eq!((n.opacity, n.blend, n.isolate, n.knockout_shape), (1.0, BlendMode::Normal, false, false), "{last}");
        // The group's own state block (`6 () XW`) keeps it the group's.
        let doc = read_ok(&layer(&format!("{last}1 0.6 1 2 1 Xy\n0 0 Xd\n6 () XW\n"))).doc;
        assert_eq!(doc.layers[0].opacity, 1.0, "{last}");
        assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (0.6, true), "{last}");
    }
}

#[test]
fn a_layers_state_keeps_through_the_stroke_style_written_in_it() {
    // GUARDS: a layer's state block can have the stroke style between its `Xy` and `Xd`; read as
    // the art moving on, it lost the layer's opacity and the layer drew opaque.
    let state = "0 0.25 0 2 0 Xy\n0 J 0 j 1 w 10 M []0 d\n0 XR\n0 0 Xd\n7 () XW\n";
    let clip_group = format!(
        "q\n{}h\nW\nn\n0 Ae\nu\n{RED}{}f\nU\n9 () XW\nQ\n9 () XW\n{state}",
        rect_path(10.0, 10.0, 90.0, 90.0),
        rect_path(0.0, 0.0, 100.0, 100.0)
    );
    let path = format!("{RED}{}f\n{state}", rect_path(0.0, 0.0, 10.0, 10.0));
    for body in [clip_group, path] {
        let doc = read_ok(&layer(&body)).doc;
        let l = &doc.layers[0];
        assert_eq!((l.opacity, l.knockout), (0.25, Knockout::Neutral), "{body}");
        assert_eq!(art(&doc)[0].opacity, 1.0, "{body}");
    }
    // An object's own state block (`6 () XW`) isn't the layer's.
    let note = format!("{RED}{}f\n{}", rect_path(0.0, 0.0, 10.0, 10.0), state.replace("7 () XW", "6 () XW"));
    assert_eq!(read_ok(&layer(&note)).doc.layers[0].opacity, 1.0);
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

/// A mesh's opacity defines the knockout shape when an `Xy` says so before it or right after its
/// end, whether its patches make one mesh or a group of patches. An `Xy` that is the layer's leaves
/// the mesh's as it was.
#[test]
fn a_gradient_mesh_keeps_its_knockout_shape() {
    let one = straight_mesh(2, 2, (10.0, 10.0));
    let patches = one.replace("[2 2] /Size X#\n", "");
    let shape = "0 0.5 0 0 1 Xy\n";
    for (mesh, is_mesh) in [(&one, true), (&patches, false)] {
        let doc = read_ok(&layer(&format!("{shape}{mesh}"))).doc;
        assert_eq!(matches!(art(&doc)[0].kind, NodeKind::Mesh(_)), is_mesh, "{:?}", art(&doc)[0].kind);
        assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (0.5, true), "{is_mesh}");
    }
    let doc = read_ok(&layer(&format!("{one}{shape}0 0 Xd\n6 () XW\n"))).doc;
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (0.5, true), "written after its end");
    let doc = read_ok(&layer(&format!("{shape}{one}1 0.35 0 0 0 Xy\n0 0 Xd\n7 () XW\n"))).doc;
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (0.5, true), "the mesh's own");
    assert_eq!((doc.layers[0].opacity, doc.layers[0].knockout_shape), (0.35, false), "the layer's own");
    let doc = read_ok(&layer(&format!("{one}1 0.35 0 0 1 Xy\n0 0 Xd\n7 () XW\n"))).doc;
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (1.0, false), "the mesh's own");
    assert_eq!((doc.layers[0].opacity, doc.layers[0].knockout_shape), (0.35, true), "the layer's own");
}

#[test]
fn a_gradient_mesh_takes_the_transparency_written_after_it() {
    // GUARDS: a mesh's own state (`Xy`) comes right after its `/End X!`, before `Xd` and `XW`, as a
    // group's does after `U`; read as the next object's, a Multiply shadow mesh painted opaque.
    let corner =
        |x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| format!("%_[1 1 1 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1);
    let mesh = |x: i32| {
        format!(
            "/Mesh X!\n%_(DeviceRGB) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n/End X!\n",
            corner(x, 0, (x, 10), (x + 10, 0)),
            corner(x, 40, (x + 10, 40), (x, 30)),
            corner(x + 40, 40, (x + 40, 30), (x + 30, 40)),
            corner(x + 40, 0, (x + 30, 0), (x + 40, 10)),
        )
    };
    let art_ = format!(
        "0 1 0 0 0 Xy\n{}1 0.8 0 0 0 Xy\n0 0 Xd\n6 () XW\n{}0 0 Xd\n6 () XW\n2 0.7 0 0 0 Xy\n{RED}{}f\n",
        mesh(0),
        mesh(50),
        rect_path(100.0, 0.0, 110.0, 10.0)
    );
    let doc = read_ok(&layer(&art_)).doc;
    let a = art(&doc);
    assert!(matches!(a[0].kind, NodeKind::Mesh(_)) && matches!(a[1].kind, NodeKind::Mesh(_)), "{a:#?}");
    assert_eq!((a[0].opacity, a[0].blend), (0.8, BlendMode::Multiply), "the state after the mesh is its own");
    assert_eq!((a[1].opacity, a[1].blend), (0.8, BlendMode::Multiply), "the next mesh keeps the current state");
    assert_eq!((a[2].opacity, a[2].blend), (0.7, BlendMode::Screen), "a state after the mesh's `Xd` and `XW` is the next object's");
}

#[test]
fn a_masked_gradient_mesh_takes_the_transparency_written_after_its_mask() {
    // GUARDS: a mesh with an opacity mask has its state after the mask's dictionary, whose art (on
    // `%_` lines) has operators of its own; read as the next object's, each masked mesh in a run
    // drew with the blend mode of the one before it.
    let corners: String = ["10 20 10 25 1 15 20 1", "10 50 15 50 1 10 45 1", "40 50 40 45 1 35 50 1", "40 20 35 20 1 40 25 1"]
        .iter()
        .map(|g| format!("%_[0.5 1 {g}] /N X#\n"))
        .collect();
    let mesh = format!("/Mesh X!\n%_/DeviceGray /CS X#\n{corners}%_/E X#\n/End X!\n");
    let mask = "%_/ArtDictionary :\n%_/Mask :\n%_X=\n%_u\n%_1 g\n%_10 20 m\n%_10 50 L\n%_40 50 L\n%_40 20 L\n%_f\n%_U\n%_X+\n\
                %_(Art) ,\n%_1 /Bool (Clipping) ,\n%_1 /Bool (Linked) ,\n%_; (AI9_Mask) ,\n%_;\n";
    let next = format!("0 1 0 0 0 Xy\n{RED}{}f\n", rect_path(60.0, 20.0, 80.0, 40.0));
    for (state, expected) in [("1 0.5 0 0 0 Xy\n", (0.5, BlendMode::Multiply)), ("", (1.0, BlendMode::Overlay))] {
        let a = art(&read_ok(&layer(&format!("3 1 0 0 0 Xy\n{mesh}{mask}{state}0 0 Xd\n6 () XW\n{next}"))).doc);
        assert_eq!(a.len(), 2, "{a:?}");
        assert!(matches!(a[0].kind, NodeKind::Mesh(_)) && a[0].extras().mask.is_some(), "{:?}", a[0]);
        assert_eq!((a[0].opacity, a[0].blend), expected, "{state:?}: the state after the mask is the mesh's");
        assert_eq!((a[1].opacity, a[1].blend), (1.0, BlendMode::Normal), "the next object's own state");
    }
}

#[test]
fn a_transparency_set_in_a_dictionary_s_art_stays_after_it() {
    // GUARDS: an area text's frame (its `/ConfiningPath` art) sets its own transparency; the objects
    // after the text, with no `Xy` of their own, draw with it. Read as the dictionary's alone, they
    // drew with the Lighten of the path before the text and vanished on a light background.
    let text = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n/Art :\nX=\n0 Ap\n0 1 0 0 0 Xy\n0 w\n100 0 m\n100 10 L\nn\nX+\n\
                ; /ConfiningPath ,\n2 /TextAntialiasing ,\n;\n";
    let body = format!("9 0.5 0 0 0 Xy\n{RED}{}f\n{text}{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0), rect_path(20.0, 0.0, 30.0, 10.0));
    let a = art(&read_ok(&layer(&body)).doc);
    let paths: Vec<_> = a.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
    assert_eq!(paths.len(), 2, "{a:#?}");
    assert_eq!((paths[0].opacity, paths[0].blend), (0.5, BlendMode::Lighten), "the state before the text");
    assert_eq!((paths[1].opacity, paths[1].blend), (1.0, BlendMode::Normal), "the state the text's art set");
}

#[test]
fn a_transparency_set_in_a_clipping_group_stays_after_it() {
    // GUARDS: `Q` ends a clipping group but doesn't take the transparency back to what it was at
    // its `q`: the writer sets `Xy` only when the next object's state differs from the last one it
    // wrote. A Screen glow, then a clipping group whose path is set back to Normal, then type with
    // no `Xy`: restored at `Q`, the type drew in Screen and nearly vanished on a light background.
    let clip = format!("q\n0 1 0 0 0 Xy\n{}h\nW\nn\n{RED}{}f\nQ\n", rect_path(0.0, 0.0, 200.0, 100.0), rect_path(40.0, 0.0, 50.0, 10.0));
    let body = format!("2 0.8 0 0 0 Xy\n{RED}{}f\n{clip}{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0), rect_path(20.0, 0.0, 30.0, 10.0));
    let a = art(&read_ok(&layer(&body)).doc);
    assert_eq!(a.len(), 3, "{a:#?}");
    assert_eq!((a[0].opacity, a[0].blend), (0.8, BlendMode::Screen), "the glow's own state");
    assert!(matches!(a[1].kind, NodeKind::Group { clip: true, .. }), "{:?}", a[1].kind);
    assert_eq!((a[2].opacity, a[2].blend), (1.0, BlendMode::Normal), "the state set inside the clipping group");
}

#[test]
fn gradients() {
    let defs = "%AI5_BeginGradient: (G)\n(G) 1 2 Bd\n[\n<00FF>\n0\n0 0 0 0 1 0 0 2 1 6 %_BS\n\
                %_0 0 0 0 1 0 0 2 1 6 40 0 Bs\n%_0 0 0 0 0 0 1 2 0.5 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!(
        "{RED}{}Bb\n0 0 0 0 Bh\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n50 0 0 -50 100 50 Bm\nf\n0 BB\n{}f\n\
         Bb\n2 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n0 0 0 0 0 0 1 Xa\n{}f\n\
         {}Bb\n-20 -10 144 0.77 Bh\n1 (G) -0.1 0.4 0 1.35 1 0 0 -1 0 16384 1 Bg\n50 0 0 -50 100 50 Bm\nf\n0 BB\n",
        rect_path(50.0, 0.0, 150.0, 100.0),
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
        rect_path(50.0, 0.0, 150.0, 100.0),
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
    // A gradient instance that paints nothing doesn't stick to the next object, which sets its colour.
    assert_eq!(fill(&a[2]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    // A radial gradient is its matrix's unit circle whatever `Bg`'s origin and length, and its
    // highlight an offset from the centre in art space.
    let Paint::Gradient(g) = fill(&a[3]) else { panic!("{:?}", fill(&a[3])) };
    let geom = g.geom.unwrap();
    assert_eq!((geom.start, geom.end), (Point::new(100.0, 50.0), Point::new(150.0, 50.0)));
    let focal = geom.focal.unwrap();
    assert!((focal - Point::new(80.0, 60.0)).hypot() < 1e-6, "{focal:?}");
    // A linear gradient with a stop in the middle: `Xm` places its whole ramp, a `Bm` each of its
    // two parts. The object's own note of its gradient after it paints nothing.
    let defs = "(L) 0 3 Bd\n%_1 0 50 100 Bs\n%_0.5 0 50 40 Bs\n%_0 0 50 0 Bs\nBD\n";
    let art_ = format!(
        "{RED}{}Bb\n1 (L) 0 0 -90 1 1 0 0 1 0 0 1 Bg\n0 -100 -100 0 150 100 Xm\n0 -9000 -100 0 150 9100 Bc\n0 -40 -100 0 150 100 Bm\n\
         0 -60 -100 0 150 60 Bm\n0 -9000 -100 0 150 0 Bc\nf\n0 BB\n0 1 Xd\nBb\n2 (L) 0 0 -90 1 1 0 0 1 0 0 1 Bg\n0 BB\n6 () XW\n{}f\n",
        rect_path(50.0, 0.0, 150.0, 100.0),
        rect_path(0.0, 0.0, 10.0, 10.0),
    );
    let doc = read_ok(&format!("{defs}{}", layer(&art_))).doc;
    let a = art(&doc);
    let Paint::Gradient(g) = fill(&a[0]) else { panic!("{:?}", fill(&a[0])) };
    assert_eq!(g.gradient.stops.len(), 3);
    // Art space has y up from the crop marks' bottom.
    let geom = g.geom.unwrap();
    assert!(geom.start.distance(Point::new(150.0, 0.0)) < 1e-9 && geom.end.distance(Point::new(150.0, 100.0)) < 1e-9, "{geom:?}");
    assert_eq!(fill(&a[1]).color(), Some(Color::rgb(1.0, 0.0, 0.0)), "the note after the object isn't the next path's paint");
}

#[test]
fn a_compound_path_holding_its_drawn_look_paints_its_paths_apart() {
    // GUARDS: made one compound path, the look's paths take a single fill and the inner one punches a hole.
    let art_ = format!(
        "0 Ae\n*u\n0 Ae\nu\n{RED}{}f\n0 0 0 0 0 0 1 Xa\n{}f\nU\n9 () XW\n*U\n",
        rect_path(0.0, 0.0, 20.0, 20.0),
        rect_path(5.0, 5.0, 15.0, 15.0)
    );
    let a = art(&read_ok(&layer(&art_)).doc);
    fn leaves(n: &Node, out: &mut Vec<Paint>) {
        match n.children() {
            Some(children) if !matches!(n.kind, NodeKind::Compound { .. }) => children.iter().for_each(|c| leaves(c, out)),
            _ => out.push(fill(n)),
        }
    }
    let mut painted = vec![];
    a.iter().for_each(|n| leaves(n, &mut painted));
    let fills: Vec<_> = painted.iter().map(Paint::color).collect();
    assert_eq!(fills, [Some(Color::rgb(1.0, 0.0, 0.0)), Some(Color::rgb(0.0, 0.0, 1.0))], "{a:#?}");
}

/// Clipping paths in a compound path that holds its drawn look clip as one compound path. Each one's fill is a path of
/// its own under the art they clip, with the clipping path's opacity mask, so overlapping fills and fills that differ
/// paint as each clipping path paints them.
#[test]
fn clipping_paths_beside_a_compound_paths_look_keep_their_fills() {
    let (a, b) = (rect_path(0.0, 0.0, 20.0, 20.0), rect_path(5.0, 5.0, 25.0, 25.0));
    let look = format!("0 Ae\nu\n{RED}{}f\n0 0 0 0 0 0 1 Xa\n{}f\nU\n9 () XW\n", rect_path(0.0, 0.0, 30.0, 30.0), rect_path(10.0, 10.0, 15.0, 15.0));
    let mask_art: String =
        format!("0 0 0 0 (Global white) 1 0 Xk\n{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)).lines().map(|l| format!("%_{l}\n")).collect();
    let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{mask_art}%_X+\n%_(Art) ,\n%_; (AI9_Mask) ,\n%_;\n");
    let children = |second: &str, after_first: &str| {
        let body = format!("0 Ae\n*u\n{RED}{a}h\nW\nf\n{after_first}{second}{b}h\nW\nf\n{look}*U\n");
        let doc = read_ok(&layer(&body)).doc;
        // Every object, opacity masks' art included, has an id of its own.
        fn ids(n: &Node, seen: &mut std::collections::HashSet<vectorcraft_doc::NodeId>) {
            assert!(seen.insert(n.id), "{:?} appears twice", n.id);
            if let Some(m) = &n.extras().mask {
                ids(&m.art, seen);
            }
            for c in n.children().into_iter().flatten() {
                ids(c, seen);
            }
        }
        let mut seen = std::collections::HashSet::new();
        doc.layers.iter().for_each(|l| ids(l, &mut seen));
        let a = art(&doc);
        let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{a:#?}") };
        // An opacity mask: the swatch its art's fill is linked to.
        let mask = |c: &Node| {
            c.extras().mask.as_ref().map(|m| {
                let mut swatch = None;
                m.art.walk(&mut |p| {
                    if let Paint::Solid { swatch: Some(s), .. } = fill(p) {
                        swatch = Some(s);
                    }
                });
                swatch
            })
        };
        children.iter().map(|c| (c.kind.clone(), fill(c).color(), mask(c))).collect::<Vec<_>>()
    };
    let (red, blue) = (Some(Color::rgb(1.0, 0.0, 0.0)), Some(Color::rgb(0.0, 0.0, 1.0)));
    // A child's kind, fill color and, with an opacity mask, the swatch its art is linked to.
    type Child = (NodeKind, Option<Color>, Option<Option<String>>);
    let paths = |c: &[Child]| {
        assert!(matches!(c[0].0, NodeKind::Compound { .. }) && c.len() == 4, "{c:?}");
        c[..3].iter().map(|(k, c, m)| (matches!(k, NodeKind::Path { clipping: false, .. }), *c, m.clone())).collect::<Vec<_>>()
    };
    assert_eq!(paths(&children("", "")), [(false, None, None), (true, red, None), (true, red, None)]);
    assert_eq!(paths(&children("0 0 0 0 0 0 1 Xa\n", "")), [(false, None, None), (true, red, None), (true, blue, None)]);
    // The fill's copy of the mask stays linked to the mask art's global swatch.
    let linked = Some(Some("Global white".to_string()));
    assert_eq!(paths(&children("0 0 0 0 0 0 1 Xa\n", &mask)), [(false, None, None), (true, red, linked), (true, blue, None)]);
}

#[test]
fn a_compound_path_of_a_plain_group_stays_one_compound_path() {
    // GUARDS: #980. Files write `9 () XW` after every group, the one a compound path keeps its
    // subpaths in too: read as a drawn look, the compound became a group of filled paths, its
    // holes filled in. Here a ring (outer path, inner one reversed, one paint) and a path beside it.
    let ring = format!("{RED}1 D\n{}f\n0 D\n{}f\n", rect_path(0.0, 0.0, 20.0, 20.0), "5 5 m\n15 5 L\n15 15 L\n5 15 L\n5 5 L\n");
    let art_ = format!("0 Ae\nu\n0 Ae\n*u\n0 Ae\nu\n{ring}U\n9 () XW\n*U\n{}f\nU\n9 () XW\n", rect_path(8.0, 8.0, 12.0, 12.0));
    let a = art(&read_ok(&layer(&art_)).doc);
    let NodeKind::Group { children, .. } = &a[0].kind else { panic!("{a:#?}") };
    let NodeKind::Compound { children: subpaths, .. } = &children[0].kind else { panic!("a compound path: {:#?}", children[0]) };
    assert_eq!(subpaths.len(), 2, "{subpaths:#?}");
    assert!(subpaths.iter().all(|p| matches!(p.kind, NodeKind::Path { .. })));
    assert_eq!(fill(&children[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn gradients_through_a_skewed_matrix() {
    // GUARDS: a sheared `Xm` leaves the matrix's columns oblique. A linear ramp runs along the
    // normal to its isolines, not along its first column; a radial's ellipse is the one the unit
    // circle maps to, set by the principal axes.
    let defs = |name: &str, kind: u8| {
        format!(
            "%AI5_BeginGradient: ({name})\n({name}) {kind} 2 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 0 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n"
        )
    };
    let instance = |name: &str| format!("Bb\n1 ({name}) 0 0 0 1 1 0 0 1 0 0 1 Bg\n10 0 10 10 100 50 Xm\nf\n0 BB\n");
    let art_ = format!("{}{}{}{}", rect_path(0.0, 0.0, 200.0, 100.0), instance("Lin"), rect_path(0.0, 0.0, 200.0, 100.0), instance("Rad"));
    let doc = read_ok(&format!("{}{}{}", defs("Lin", 0), defs("Rad", 1), layer(&art_))).doc;
    let a = art(&doc);
    let Paint::Gradient(g) = fill(&a[0]) else { panic!("{:?}", fill(&a[0])) };
    let geom = g.geom.unwrap();
    assert!((geom.start - Point::new(100.0, 50.0)).hypot() < 1e-9 && (geom.end - Point::new(105.0, 55.0)).hypot() < 1e-9, "{geom:?}");
    let Paint::Gradient(g) = fill(&a[1]) else { panic!("{:?}", fill(&a[1])) };
    let geom = g.geom.unwrap();
    let golden = (1.0 + 5f64.sqrt()) / 2.0;
    assert!((geom.start.distance(geom.end) - 10.0 * golden).abs() < 1e-9, "{geom:?}");
    assert!((geom.aspect - 1.0 / golden.powi(2)).abs() < 1e-9, "{geom:?}");
}

#[test]
fn stops_sharing_a_ramp_point_keep_their_order() {
    // GUARDS: stops come last first, so a sort alone puts the two at 100 the wrong way round and
    // the ramp blends towards the colour meant only for past its end.
    let defs = "%AI5_BeginGradient: (S)\n(S) 0 3 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 100 Bs\n\
                %_0 0 0 0 0 1 0 2 1 6 50 100 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 0 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!("{}Bb\n1 (S) 0 0 0 1 1 0 0 1 0 0 1 Bg\n10 0 0 10 100 50 Xm\nf\n0 BB\n", rect_path(0.0, 0.0, 200.0, 100.0));
    let doc = read_ok(&format!("{defs}{}", layer(&art_))).doc;
    let Paint::Gradient(g) = fill(&art(&doc)[0]) else { panic!() };
    let colors: Vec<_> = g.gradient.stops.iter().map(|s| s.color).collect();
    assert_eq!(colors, [Color::rgb(0.0, 0.0, 1.0), Color::rgb(0.0, 1.0, 0.0), Color::rgb(1.0, 0.0, 0.0)]);
}

#[test]
fn stops_listed_upwards_keep_the_midpoint_of_the_one_that_starts_the_ramp() {
    // GUARDS: reversing a list that runs upwards swaps two stops at 0, and the ramp to 100 takes the
    // midpoint of the zero-length part between them.
    let defs = "%AI5_BeginGradient: (U)\n(U) 0 3 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 1 1 2 1 6 50 0 Bs\n\
                %_0 0 0 0 1 1 1 2 1 6 87 0 Bs\n%_0 0 0 0 0 0 0 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let art_ = format!("{}Bb\n1 (U) 0 0 0 1 1 0 0 1 0 0 1 Bg\n10 0 0 10 100 50 Xm\nf\n0 BB\n", rect_path(0.0, 0.0, 200.0, 100.0));
    let doc = read_ok(&format!("{defs}{}", layer(&art_))).doc;
    let Paint::Gradient(g) = fill(&art(&doc)[0]) else { panic!() };
    let midpoints: Vec<_> = g.gradient.stops.iter().map(|s| s.midpoint).collect();
    assert_eq!(midpoints, [0.5, 0.87, 0.5]);
}

#[test]
fn per_anchor_data_reads() {
    let art_ = format!("{RED}{}f\n(00800080) Xt 0 XD\n%_(00004000) Xt 1 XD\n", rect_path(0.0, 0.0, 10.0, 10.0));
    assert_eq!(art(&read_ok(&layer(&art_)).doc).len(), 1);
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
fn the_active_artboard_is_the_one_crop_area_active_names() {
    let board = |x: i32, w: i32| {
        format!(
            "%_/Dictionary :\n%_{x} 100 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_{} 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n",
            x + w
        )
    };
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    // Four artboards in the array, the second of no width (left out); `entry` follows the array.
    let opened = |entry: &str| {
        let boards = [board(0, 100), board(200, 0), board(400, 100), board(600, 100)].concat();
        let data = format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n{boards}%_; (ArtboardArray) ,\n{entry}%_; /NotRecorded ,\n%_;\n{square}");
        let doc = read_ok(&data).doc;
        doc.opened_artboard.map(|id| doc.artboards.iter().position(|a| a.id == id))
    };
    // The index counts every entry of the array, the one left out too.
    assert_eq!(opened("%_3 /Int (CropAreaActive) ,\n"), Some(Some(2)));
    assert_eq!(opened("%_2 /Int (CropAreaActive) ,\n"), Some(Some(1)));
    // After another entry of the dictionary that holds the array.
    assert_eq!(opened("%_1 /Int (Other) ,\n%_3 /Int (CropAreaActive) ,\n"), Some(Some(2)));
    // The first artboard, the one left out, one past the end, a value of another type, or none.
    for entry in
        ["%_0 /Int (CropAreaActive) ,\n", "%_1 /Int (CropAreaActive) ,\n", "%_4 /Int (CropAreaActive) ,\n", "%_2 /Real (CropAreaActive) ,\n", ""]
    {
        assert_eq!(opened(entry), None, "{entry}");
    }
}

/// The document data of artboards side by side, 100 points wide, each with the entries `extra`.
fn boards_with(extra: &[&str]) -> String {
    let boards: String = extra
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let x = i * 100;
            format!(
                "%_/Dictionary :\n{e}%_{x} 100 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_{} 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n",
                x + 100
            )
        })
        .collect();
    format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n{boards}%_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n")
}

#[test]
fn artboards_keep_their_display_options() {
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let options = |extra: &[&str]| {
        read_ok(&format!("{}{square}", boards_with(extra)))
            .doc
            .artboards
            .iter()
            .map(|a| (a.show_center_mark, a.show_cross_hairs, a.show_video_safe_areas, a.video_ruler_pixel_aspect_ratio))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        options(&["%_1 /Int (DisplayMark) ,\n%_1.33 /Real (PAR) ,\n", "%_6 /Int (DisplayMark) ,\n%_2 /Real (PAR) ,\n"]),
        [(true, false, false, 1.33), (false, true, true, 2.0)],
        "each artboard has its own"
    );
    assert_eq!(options(&["%_7 /Int (DisplayMark) ,\n%_1 /Real (PAR) ,\n"]), [(true, true, true, 1.0)]);
    // Values of another type or out of range leave the defaults.
    for bad in [
        "%_1 /Real (DisplayMark) ,\n%_1 /Int (PAR) ,\n",
        "%_-1 /Int (DisplayMark) ,\n%_20 /Real (PAR) ,\n",
        "%_1.5 /Int (DisplayMark) ,\n%_0.05 /Real (PAR) ,\n",
        "%_1 2 /Int (DisplayMark) ,\n%_(2) /Real (PAR) ,\n",
    ] {
        assert_eq!(options(&[bad]), [(false, false, false, 1.0)], "{bad}");
    }
}

/// The document data with the Document Raster Effects Settings `entries`.
fn raster_settings(entries: &str) -> String {
    format!("%_/Document :\n%_/Dictionary :\n%_/Dictionary :\n{entries}%_; (AI Auto Rasterize) ,\n%_; /NotRecorded ,\n%_;\n")
}

#[test]
fn document_raster_effects_settings_come_from_the_document() {
    use vectorcraft_doc::{Background, RasterColorModel, RasterEffectsSettings};
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let settings = |entries: &str| {
        let doc = read_ok(&format!("{}{square}", raster_settings(entries))).doc;
        (doc.raster_effects_ppi, doc.raster_effects)
    };
    let all = "%_6 /Int (colr) ,\n%_0 /Bool (spot) ,\n%_150 /Int (dpi.) ,\n%_1 /Bool (mask) ,\n%_0 /Bool (alis) ,\n%_7 /Real (padd) ,\n";
    assert_eq!(
        settings(all),
        (
            150.0,
            RasterEffectsSettings {
                color_model: RasterColorModel::Grayscale,
                background: Background::Transparent,
                anti_alias: false,
                clipping_mask: true,
                add_around: 7.0,
                preserve_spot_colors: false,
            }
        )
    );
    let (_, white) = settings("%_0 /Int (colr) ,\n");
    assert_eq!((white.color_model, white.background), (RasterColorModel::Document, Background::White));
    let (_, document) = settings("%_4 /Int (colr) ,\n");
    assert_eq!((document.color_model, document.background), (RasterColorModel::Document, Background::Transparent));
    // Values of another type, out of range or unknown leave the defaults.
    let defaults = (72.0, RasterEffectsSettings::default());
    for bad in [
        "%_150 /Real (dpi.) ,\n%_1 /Int (mask) ,\n%_7 /Int (padd) ,\n%_6 /Real (colr) ,\n",
        "%_5000 /Int (dpi.) ,\n%_2 /Bool (alis) ,\n%_-1 /Real (padd) ,\n%_3 /Int (colr) ,\n",
        "%_0 /Int (dpi.) ,\n%_(0) /Bool (spot) ,\n%_2000 /Real (padd) ,\n%_1 /Int (colr) ,\n",
        "%_150.5 /Int (dpi.) ,\n%_0.5 /Bool (mask) ,\n%_4.5 /Int (colr) ,\n",
    ] {
        assert_eq!(settings(bad), defaults, "{bad}");
    }
}

/// Editing data whose header has the comments `header` before `%%EndComments`.
fn stream_with_header(header: &str, body: &str) -> String {
    stream(body).replacen("%%EndComments", &format!("{header}%%EndComments"), 1)
}

#[test]
fn the_header_gives_the_author_and_custom_colors() {
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let header = "%%For: (Ann \\(QA\\) Lee) (Studio)\n%%DocumentCustomColors: (Ink A)\n%%+ (Ink B)\n%%+ (Ink C)\n\
                  %%CMYKCustomColor: 0.1 0.2 0.3 0.4 (Ink A)\n%%+ 0 1 0 0 (Ink B)\n%%RGBCustomColor: 1 0.5 0 (Ink C)\n\
                  %%+ 2 0 0 (Too bright)\n%%CMYKCustomColor: 0.1 0.2 0.3 (Too few)\n%%+ 0 0 0 1 (Unended\n";
    let after = "%%CMYKCustomColor: 0 0 0 1 (After the header)\n%%For: (Someone else) ()\n";
    let r = read(stream_with_header(header, &format!("{after}{square}")).as_bytes()).unwrap();
    assert_eq!(r.doc.metadata.author, "Ann (QA) Lee");
    let ink = |name: &str| r.doc.swatches.iter().find(|s| s.name == name).map(|s| (s.paint.color(), s.global, s.spot));
    assert_eq!(ink("Ink A"), Some((Some(Color::cmyk(0.1, 0.2, 0.3, 0.4)), true, true)));
    assert_eq!(ink("Ink B"), Some((Some(Color::cmyk(0.0, 1.0, 0.0, 0.0)), true, true)));
    assert_eq!(ink("Ink C"), Some((Some(Color::rgb(1.0, 0.5, 0.0)), true, true)));
    for name in ["Too bright", "Too few", "Unended", "After the header"] {
        assert_eq!(ink(name), None, "{name}");
    }
    // A color the art defines keeps its definition.
    let used = layer(&format!("0.5 0 0 0 (Ink A) 0 x\n{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read(stream_with_header(header, &used).as_bytes()).unwrap();
    let a: Vec<_> = r.doc.swatches.iter().filter(|s| s.name == "Ink A").map(|s| s.paint.color()).collect();
    assert_eq!(a, [Some(Color::cmyk(0.5, 0.0, 0.0, 0.0))]);
    // A color with the name of a default swatch takes its place in a file without a palette,
    // whether the header or the art defines it.
    let red = "%%CMYKCustomColor: 0 0.5 0.5 0 (Red)\n%%+ 0 0 0 1 ([None])\n";
    let r = read(stream_with_header(red, &square).as_bytes()).unwrap();
    let reds: Vec<_> = r.doc.swatches.iter().filter(|s| s.name == "Red").map(|s| (s.paint.color(), s.spot)).collect();
    assert_eq!(reds, [(Some(Color::cmyk(0.0, 0.5, 0.5, 0.0)), true)]);
    assert!(matches!(r.doc.swatch("[None]").map(|s| &s.paint), Some(Paint::None)), "a reserved name keeps its swatch");
    let art_red = layer(&format!("0 0.5 0.5 0 (Red) 0 x\n{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read(stream(&art_red).as_bytes()).unwrap();
    let reds: Vec<_> = r.doc.swatches.iter().filter(|s| s.name == "Red").map(|s| (s.paint.color(), s.spot)).collect();
    assert_eq!(reds, [(Some(Color::cmyk(0.0, 0.5, 0.5, 0.0)), true)]);
    // Without the comments: no author, the default swatches.
    let plain = read_ok(&square).doc;
    assert_eq!(plain.metadata.author, "");
    assert!(plain.swatches.iter().all(|s| !s.name.starts_with("Ink")));
}

#[test]
fn an_older_files_template_box_places_its_artboard() {
    let old =
        format!("%!PS-Adobe-3.0\n%AI5_ArtSize: 600 400\n%%TemplateBox: 300.5 199.5 300.5 199.5\n%%EndComments\n{}", layer("10 10 m\n20 20 L\nS\n"));
    let r = read(old.as_bytes()).unwrap();
    assert_eq!(r.doc.artboards[0].rect, Rect::new(0.0, 0.0, 600.0, 400.0));
    assert_eq!(r.template, Some([300.5, 199.5, 300.5, 199.5]));
}

#[test]
fn art_after_the_printers_crop_marks_reads() {
    // GUARDS: the crop marks between `%AI3_BeginCrops` and `%AI3_EndCrops` are skipped, and the art
    // after them is read.
    let crops = format!("%AI3_BeginCrops\n{}S\n%AI3_EndCrops\n", rect_path(-20.0, -20.0, -10.0, -10.0));
    let doc = read_ok(&format!("{crops}{}", layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0))))).doc;
    assert_eq!(art(&doc).len(), 1);
}

/// The document data `library` (a `variableSets` element) read with a layer holding a square: the
/// variables read and the notes about the library.
fn read_library(library: &str) -> (vectorcraft_doc::Variables, Vec<String>) {
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read_ok(&format!("{}{square}", vectorcraft_testkit::ai::variable_library_data(library)));
    let notes = r.warnings.into_iter().filter(|w| w.contains("variable library")).collect();
    ((*r.doc.variables).clone(), notes)
}

/// The note of a library read but for `n` entries.
fn partly_read(n: usize) -> String {
    format!("the file's variable library was partly read (entries skipped: {n}): the Variables panel lists the rest")
}

/// A variable library in the document data comes in with its variables in order, each of the kind
/// its trait gives (a variable without one is of unknown kind), and without bindings. A library
/// read whole gives no note.
#[test]
fn ai_variable_library_definitions() {
    use vectorcraft_doc::VariableKind;
    let (vars, notes) = read_library(&vectorcraft_testkit::ai::sample_variable_sets("none", &[]));
    let kinds: Vec<(&str, VariableKind)> = vars.variables.iter().map(|v| (v.name.as_str(), v.kind)).collect();
    assert_eq!(
        kinds,
        [
            ("v_text", VariableKind::Text),
            ("v_visible", VariableKind::Visibility),
            ("v_file", VariableKind::Image),
            ("v_graph", VariableKind::Graph),
            ("v_unbound", VariableKind::Unknown)
        ]
    );
    assert!(vars.bindings.is_empty());
    assert!(notes.is_empty(), "{notes:?}");
    // The library of the testkit's text variables, and an empty one.
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read_ok(&format!("{}{square}", vectorcraft_testkit::ai::variable_library(&["variable", "Team"], &["Home", "Away"])));
    let names: Vec<&str> = r.doc.variables.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["variable", "Team"], "{:?}", r.warnings);
    assert_eq!(r.doc.variables.dataset("Away").and_then(|d| d.values.get("Team")), Some(&vectorcraft_doc::DataValue::Text("Away Team".into())));
    let r = read_ok(&format!("{}{square}", vectorcraft_testkit::ai::variable_library(&[], &[])));
    assert_eq!(*r.doc.variables, vectorcraft_doc::Variables::default());
    assert!(!r.warnings.iter().any(|w| w.contains("variable")), "{:?}", r.warnings);
}

/// The objects a variable is bound to come with the library, each by the XML id its `docRef` gives,
/// in the order the library declares them: a variable bound to two objects is declared twice and
/// comes in once. A `docRef` that isn't an XML id is kept as unreadable, a declaration without one
/// binds nothing, and a second variable of the same name and another kind is left out with a note,
/// its binding counted as left out.
/// The objects read keep their XML ids; an id two objects have names neither of them.
#[test]
fn ai_variable_library_bindings() {
    use vectorcraft_testkit::ai::{XmlValue, named, variable_library_data, xml_element, xml_node};
    let declare = |name: &str, t: &str, art: Option<XmlValue>| {
        let mut attributes = vec![("trait", XmlValue::Str(t)), ("varName", XmlValue::Unicode(name))];
        attributes.extend(art.map(|a| ("docRef", a)));
        xml_node(1, "variable", XmlValue::Str(""), &attributes, &[])
    };
    let declared = vec![
        declare("v_shown", "visibility", Some(XmlValue::Raw("/XMLUIDREF : (one) ;"))),
        declare("v_text", "textcontent", Some(XmlValue::Str("one"))),
        declare("v_shown", "visibility", Some(XmlValue::Raw("/XMLUIDREF : (two) ;"))),
        declare("v_shown", "textcontent", Some(XmlValue::Raw("/XMLUIDREF : (two) ;"))),
        declare("v_free", "visibility", None),
    ];
    let set = xml_element("variableSet", &[("locked", "none")], &[xml_element("variables", &[], &declared)]);
    let library = variable_library_data(&xml_element("variableSets", &[], &[set]));
    let square = |x: f64, id: &str| format!("{RED}{}f\n{}", rect_path(x, 0.0, x + 10.0, 10.0), named(id));
    let squares = layer(&[square(0.0, "one"), square(20.0, "two"), square(40.0, "twice"), square(60.0, "twice")].concat());
    let r = read_ok(&format!("{library}{squares}"));
    let names: Vec<&str> = r.doc.variables.variables.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(names, ["v_shown", "v_text", "v_free"]);
    let refs: Vec<(&str, Option<&str>)> = r.bindings.iter().map(|b| (b.variable.as_str(), b.target.as_deref())).collect();
    assert_eq!(refs, [("v_shown", Some("one")), ("v_text", None), ("v_shown", Some("two"))]);
    assert!(r.warnings.contains(&partly_read(1)), "{:?}", r.warnings);
    // The object the declaration left out binds is counted for the bindings' note.
    assert_eq!(r.bindings_skipped, 1);
    // The reader doesn't bind them: the importer does, with the objects it keeps.
    assert!(r.doc.variables.bindings.is_empty());
    let ids: Vec<vectorcraft_doc::NodeId> = art(&r.doc).iter().map(|n| n.id).collect();
    assert_eq!(r.xml_ids.get("one"), Some(&Some(ids[0])));
    assert_eq!(r.xml_ids.get("two"), Some(&Some(ids[1])));
    assert_eq!(r.xml_ids.get("twice"), Some(&None));
}

/// The bindings count against the library's 8 MB of names and values: each one keeps its
/// variable's name and its target. The declarations past that bind nothing and are counted as left
/// out.
#[test]
fn ai_variable_library_bindings_stay_within_the_byte_budget() {
    use vectorcraft_testkit::ai::{XmlValue, variable_library_data, xml_element, xml_node};
    let (name, id) = ("é".repeat(255), "a".repeat(255));
    let target = format!("/XMLUIDREF : ({id}) ;");
    let declare = || {
        xml_node(
            1,
            "variable",
            XmlValue::Str(""),
            &[("trait", XmlValue::Str("visibility")), ("varName", XmlValue::Unicode(&name)), ("docRef", XmlValue::Raw(&target))],
            &[],
        )
    };
    let declared: Vec<_> = (0..11_500).map(|_| declare()).collect();
    let set = xml_element("variableSet", &[("locked", "none")], &[xml_element("variables", &[], &declared)]);
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read_ok(&format!("{}{square}", variable_library_data(&xml_element("variableSets", &[], &[set]))));
    assert_eq!(r.doc.variables.variables.len(), 1);
    // The variable's name, then each binding's name and target.
    let kept = ((8 << 20) - name.len()) / (name.len() + id.len());
    assert_eq!(r.bindings.len(), kept);
    assert_eq!(r.bindings_skipped, 11_500 - kept);
}

/// The data sets come in in order with their values: text of one paragraph per `p` element (`&`,
/// `<` and accented letters as they are), whether the objects show, and a linked file's path from
/// its URL, kept when it names no file. A variable of unknown kind has no value.
#[test]
fn ai_variable_library_dataset_values() {
    use vectorcraft_doc::DataValue;
    use vectorcraft_testkit::ai::{SAMPLE_MISSING_FILE, sample_variable_sets};
    let (vars, notes) = read_library(&sample_variable_sets("none", &[]));
    assert!(notes.is_empty(), "{notes:?}");
    let names: Vec<&str> = vars.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["ds_one", "ds_two", "ds_three"]);
    let two = &vars.datasets[1].values;
    assert_eq!(two.get("v_text"), Some(&DataValue::Text("line one\nline two & < é".into())));
    assert_eq!(two.get("v_visible"), Some(&DataValue::Visible(false)));
    assert_eq!(two.get("v_file"), Some(&DataValue::Image(SAMPLE_MISSING_FILE.into())));
    assert!(!std::path::Path::new(SAMPLE_MISSING_FILE).exists());
    assert_eq!(two.len(), 3, "{two:?}");
    let one = &vars.datasets[0].values;
    assert_eq!(one.get("v_text"), Some(&DataValue::Text("one".into())));
    assert_eq!(one.get("v_visible"), Some(&DataValue::Visible(true)));
    assert_eq!(one.get("v_file"), Some(&DataValue::Image("/Users/test/Art (one)/tile-a.eps".into())));
}

/// A data set's value element can carry a namespace prefix.
#[test]
fn ai_variable_library_dataset_values_with_a_prefix() {
    use vectorcraft_doc::DataValue;
    use vectorcraft_testkit::ai::sample_variable_sets;
    let library = sample_variable_sets("none", &[]).replace("%_(v_visible) /String (xmlnode-nodename)", "%_(c:v_visible) /String (xmlnode-nodename)");
    assert!(library.contains("c:v_visible"));
    let (vars, notes) = read_library(&library);
    assert!(notes.is_empty(), "{notes:?}");
    assert_eq!(vars.datasets[0].values.get("v_visible"), Some(&DataValue::Visible(true)));
    assert_eq!(vars.datasets[1].values.get("v_visible"), Some(&DataValue::Visible(false)));
}

/// The data set shown comes in as the active one, and the lock as it was set.
#[test]
fn ai_variable_library_state() {
    use vectorcraft_testkit::ai::sample_variable_sets;
    let (vars, notes) = read_library(&sample_variable_sets("variableAndTypes", &[]));
    assert_eq!(vars.active_dataset.as_deref(), Some("ds_two"));
    assert!(vars.locked, "{notes:?}");
    assert!(notes.is_empty(), "{notes:?}");
    let (vars, _) = read_library(&sample_variable_sets("none", &[]));
    assert!(!vars.locked);
    assert_eq!(vars.active_dataset.as_deref(), Some("ds_two"));
    // A lock VectorCraft doesn't know reads as unlocked, with a note.
    let (vars, notes) = read_library(&sample_variable_sets("everything", &[]));
    assert!(!vars.locked);
    assert_eq!(notes, [partly_read(1)]);
    // A data set shown that isn't one of the library's leaves none active.
    let library = sample_variable_sets("none", &[]).replace("%_(ds_two) /UnicodeString (xmlnode-nodevalue) ,\n%_2 /Int (xmlnode-nodetype) ,\n%_/Array :\n%_; (xmlnode-children) ,\n%_/Dictionary :\n%_; (xmlnode-attributes) ,\n%_; (i:activeDataSet)", "%_(ds_nine) /UnicodeString (xmlnode-nodevalue) ,\n%_2 /Int (xmlnode-nodetype) ,\n%_/Array :\n%_; (xmlnode-children) ,\n%_/Dictionary :\n%_; (xmlnode-attributes) ,\n%_; (i:activeDataSet)");
    assert!(library.contains("ds_nine"));
    let (vars, notes) = read_library(&library);
    assert_eq!(vars.active_dataset, None);
    assert_eq!(vars.datasets.len(), 3, "{notes:?}");
}

/// A value that can't be read is left out with a note, and the values and data sets after it still
/// come in.
#[test]
fn ai_variable_library_bad_sibling() {
    use vectorcraft_doc::DataValue;
    use vectorcraft_testkit::ai::{XmlValue, sample_variable_sets, xml_element, xml_text};
    // A value for a variable the library doesn't declare, and a second value for `v_visible`.
    let more = [xml_element("v_nope", &[], &[xml_text(XmlValue::Str("x"))]), xml_element("v_visible", &[], &[xml_text(XmlValue::Bool(true))])];
    let (vars, notes) = read_library(&sample_variable_sets("none", &more));
    assert_eq!(notes, [partly_read(2)]);
    assert_eq!(vars.datasets[1].values.get("v_visible"), Some(&DataValue::Visible(false)));
    assert_eq!(vars.datasets[2].values.get("v_text"), Some(&DataValue::Text("three".into())));
    assert_eq!(vars.active_dataset.as_deref(), Some("ds_two"));
}

/// A graph variable comes in, but its values aren't read: each is left out with a note.
#[test]
fn ai_variable_library_graph_partial() {
    use vectorcraft_doc::VariableKind;
    use vectorcraft_testkit::ai::{XmlValue, sample_variable_sets, xml_element, xml_text};
    let graph = xml_element("v_graph", &[], &[xml_text(XmlValue::Str("a,b\n1,2"))]);
    let (vars, notes) = read_library(&sample_variable_sets("none", &[graph]));
    assert_eq!(notes, [partly_read(1)]);
    assert_eq!(vars.variable("v_graph").map(|v| v.kind), Some(VariableKind::Graph));
    assert!(vars.datasets.iter().all(|d| !d.values.contains_key("v_graph")));
    assert_eq!(vars.datasets[1].values.len(), 3);
}

/// Declarations, data sets and values that are malformed, repeated or past the caps are left out
/// and counted; a value is never read as one of another kind. A library nothing can be read from
/// says so.
#[test]
fn ai_variable_library_edge_cases() {
    use vectorcraft_doc::{DataValue, VariableKind};
    use vectorcraft_testkit::ai::{XmlValue, xml_element, xml_node, xml_text};
    let set = |locked: &str, declared: Vec<String>, rows: Vec<String>| {
        let sets = xml_element("v:sampleDataSets", &[], &rows);
        xml_element("variableSets", &[], &[xml_element("variableSet", &[("locked", locked)], &[xml_element("variables", &[], &declared), sets])])
    };
    let var = |name: &str, t: &str| xml_element("variable", &[("varName", name), ("trait", t)], &[]);
    let declared = vec![
        var("A", "textcontent"),
        // The same name again, no name, an empty one, one that is too long, and an attribute node
        // whose name isn't text.
        var("A", "visibility"),
        xml_element("variable", &[("trait", "textcontent")], &[]),
        var("  ", "textcontent"),
        var(&"n".repeat(256), "textcontent"),
        xml_node(1, "variable", XmlValue::Str(""), &[("varName", XmlValue::Bool(true))], &[]),
        // A trait VectorCraft doesn't know: unknown kind.
        var("Fancy", "sparkle"),
        var("Shown", "visibility"),
        var("File", "fileref"),
        // Something that isn't a variable.
        xml_element("note", &[], &[]),
    ];
    let rows = vec![
        xml_element(
            "v:sampleDataSet",
            &[("dataSetName", "One")],
            &[
                xml_element("A", &[], &[xml_element("p", &[], &[xml_text(XmlValue::Unicode(" spaced "))]), xml_element("p", &[], &[])]),
                // A value for the unknown kind, a visibility that is text, a file that is a relative
                // path.
                xml_element("Fancy", &[], &[xml_text(XmlValue::Str("x"))]),
                xml_element("Shown", &[], &[xml_text(XmlValue::Str("true"))]),
                xml_element("File", &[], &[xml_text(XmlValue::Str("art/tile.eps"))]),
            ],
        ),
        // The same data set again, and one without a name.
        xml_element("v:sampleDataSet", &[("dataSetName", "One")], &[]),
        xml_element("v:sampleDataSet", &[], &[]),
        xml_element(
            "v:sampleDataSet",
            &[("dataSetName", "Two")],
            &[
                // Text whose paragraph holds an element, a file on a drive, a visibility of two texts.
                xml_element("A", &[], &[xml_element("p", &[], &[xml_element("b", &[], &[])])]),
                xml_element("File", &[], &[xml_text(XmlValue::Str("file:///C:/My%20Art/50%off.eps"))]),
                xml_element("Shown", &[], &[xml_text(XmlValue::Bool(true)), xml_text(XmlValue::Bool(false))]),
            ],
        ),
    ];
    let (vars, notes) = read_library(&set("none", declared, rows));
    let kinds: Vec<(&str, VariableKind)> = vars.variables.iter().map(|v| (v.name.as_str(), v.kind)).collect();
    assert_eq!(
        kinds,
        [("A", VariableKind::Text), ("Fancy", VariableKind::Unknown), ("Shown", VariableKind::Visibility), ("File", VariableKind::Image)]
    );
    let names: Vec<&str> = vars.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["One", "Two"]);
    let one = &vars.datasets[0].values;
    assert_eq!(one.get("A"), Some(&DataValue::Text(" spaced \n".into())));
    assert_eq!(one.len(), 1, "{one:?}");
    let two = &vars.datasets[1].values;
    assert_eq!(two.get("File"), Some(&DataValue::Image("C:/My Art/50%off.eps".into())));
    assert_eq!(two.len(), 1, "{two:?}");
    // 6 declarations, 2 data sets and 5 values.
    assert_eq!(notes, [partly_read(13)]);

    // Children that aren't an array: nothing can be read.
    let broken = xml_element("variableSets", &[], &[xml_element("variableSet", &[], &[])]).replace(
        "%_/Array :\n%_; (xmlnode-children) ,\n%_/Dictionary :\n%_; (xmlnode-attributes) ,\n%_; ,",
        "%_/Dictionary :\n%_; (xmlnode-children) ,\n%_/Dictionary :\n%_; (xmlnode-attributes) ,\n%_; ,",
    );
    assert!(broken.contains("%_/Dictionary :\n%_; (xmlnode-children)"));
    let (vars, notes) = read_library(&broken);
    assert_eq!(vars, vectorcraft_doc::Variables::default());
    assert_eq!(notes, ["the file's variable library couldn't be read: the Variables panel doesn't list it"]);

    // Past the caps: 256 variables and 1,000 data sets are kept.
    let declared: Vec<String> = (0..257).map(|i| var(&format!("v{i}"), "textcontent")).collect();
    let rows: Vec<String> = (0..1001).map(|i| xml_element("v:sampleDataSet", &[("dataSetName", &format!("d{i}"))], &[])).collect();
    let (vars, notes) = read_library(&set("none", declared, rows));
    assert_eq!((vars.variables.len(), vars.datasets.len()), (256, 1000));
    assert_eq!(notes, [partly_read(2)]);
    // A text longer than the library's 8 MiB is left out, and so is text past what is left of them.
    let long = |name: &str, len: usize| {
        let text = xml_element("p", &[], &[xml_text(XmlValue::Str(&"x".repeat(len)))]);
        xml_element("v:sampleDataSet", &[("dataSetName", name)], &[xml_element("A", &[], &[text])])
    };
    let rows = vec![long("Big", (8 << 20) + 1), long("Half", 5 << 20), long("More", 4 << 20)];
    let (vars, notes) = read_library(&set("none", vec![var("A", "textcontent")], rows));
    let held: Vec<(&str, usize)> = vars.datasets.iter().map(|d| (d.name.as_str(), d.values.len())).collect();
    assert_eq!(held, [("Big", 0), ("Half", 1), ("More", 0)]);
    assert_eq!(notes, [partly_read(2)]);
    // Linked files on a network share, and on `localhost`.
    let file = |name: &str, url: &str| {
        xml_element("v:sampleDataSet", &[("dataSetName", name)], &[xml_element("File", &[], &[xml_text(XmlValue::Str(url))])])
    };
    let rows = vec![file("Share", "file://server/My%20Share/a.eps"), file("Local", "file://localhost/Users/test/a.eps")];
    let (vars, notes) = read_library(&set("none", vec![var("File", "fileref")], rows));
    let paths: Vec<Option<&DataValue>> = vars.datasets.iter().map(|d| d.values.get("File")).collect();
    assert_eq!(paths, [Some(&DataValue::Image("//server/My Share/a.eps".into())), Some(&DataValue::Image("/Users/test/a.eps".into()))]);
    assert!(notes.is_empty(), "{notes:?}");
    // A library whose only node isn't a variable set.
    let (_, notes) = read_library(&xml_element("variableSets", &[], &[xml_element("note", &[], &[])]));
    assert_eq!(notes, ["the file's variable library couldn't be read: the Variables panel doesn't list it"]);
    // A second variable set is left out.
    let two_sets = xml_element("variableSets", &[], &[xml_element("variableSet", &[], &[]), xml_element("variableSet", &[], &[])]);
    let (_, notes) = read_library(&two_sets);
    assert_eq!(notes, ["the file's variable library couldn't be read: the Variables panel doesn't list it"]);
}

/// When the layers can't be read, or reading stops before the library, [`variables`] still reads
/// it, for the file read from its PDF part.
#[test]
fn ai_variable_library_without_the_layers() {
    use vectorcraft_testkit::ai::{sample_variable_sets, variable_library_data};
    let square = layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let library = variable_library_data(&sample_variable_sets("variableAndTypes", &[]));
    let unread = stream(&format!("{library}{}", layer("1 Zz\n")));
    assert!(read(unread.as_bytes()).is_err());
    let found = variables(unread.as_bytes());
    assert!(found.present && found.variables.locked && found.skipped == 0);
    assert_eq!((found.variables.variables.len(), found.variables.datasets.len()), (5, 3));
    assert_eq!(found.variables.active_dataset.as_deref(), Some("ds_two"));
    // The reading stops before it reaches the library (at an `LB` without its layer).
    let early = stream(&format!("LB\n{library}{square}"));
    assert!(read(early.as_bytes()).is_err());
    assert_eq!(variables(early.as_bytes()), found);
    assert_eq!(variables(stream(&square).as_bytes()), read::VariableRead::default());
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

/// An object kept as its drawn look: the look (a group), then the object on `%_` lines and its style.
fn drawn_look_art() -> String {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    format!(
        "0 Ae\nu\n{RED}{}f\n{}S\nU\n%_{hidden_rect}n\n%_/ArtDictionary :\n%_/XMLUID : (Look) ; (AI10_ArtUID) ,\n%_;\n1 (Look style) XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(0.0, 0.0, 10.0, 10.0),
    )
}

/// A pattern's art holding an object kept as its drawn look counts in the note, as a symbol's does;
/// a definition the same name replaces later counts no longer.
#[test]
fn a_drawn_look_in_a_patterns_art_is_noted() {
    let pattern = |art: &str| format!("%AI3_BeginPattern: (P)\n(P) 0 0 10 10\n0 A\n0 Xw\n{art}E\n%AI3_EndPattern\n");
    let looked = pattern(&drawn_look_art());
    let r = read_ok(&format!("{looked}{}", layer("")));
    assert_eq!(r.doc.patterns.len(), 1, "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("drawn look (1)")), "{:?}", r.warnings);
    let plain = pattern(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read_ok(&format!("{looked}{plain}{}", layer("")));
    assert_eq!(r.doc.patterns.len(), 1, "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// A brush's art holding an object kept as its drawn look counts in the note once a brush that is
/// kept uses the art; art no brush uses brings no note.
#[test]
fn a_drawn_look_in_a_brushs_art_is_noted_when_a_brush_uses_it() {
    let section = format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n{}%AI8_EndBrushPattern\n", drawn_look_art());
    let r = read_ok(&format!("{section}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    assert!(matches!(brush_named(&lib, "Spray"), vectorcraft_brush::BrushKind::Scatter(_)), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.contains("drawn look (1)")), "{:?}", r.warnings);
    let r = read_ok(&format!("{section}{}{}", plugin_object(CALLIGRAPHIC, "Nib", NIB), layer("")));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
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

/// An effect that rasterizes (a blur) leaves its drawn look as one image, then the object itself on
/// `%_` lines: the image stands for the object, as for a drawn look that's a group.
#[test]
fn a_drawn_look_that_is_an_image_stands_for_its_object() {
    let mut data = image("[ 10 0 0 10 50 90 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3);
    let object = format!("%_0 Ae\r\n%_u\r\n%_{}f\r\n%_U\r\n1 (Blurred) XW\r\n", rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\r\n%_"));
    let at = data.windows(6).position(|w| w == b"N\r\nLB\r").unwrap() + 3;
    data.splice(at..at, object.into_bytes());
    let r = read(&data).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Image(_)), "{:?}", a[0].kind);
    assert!(r.warnings.iter().any(|w| w.contains("drawn look (1)")), "{:?}", r.warnings);
}

/// The name and opacity mask after an image follow the operators that end its raster: `XH`, the
/// versioned `Xr` and the `N` after `%AI5_EndRaster`.
#[test]
fn an_images_name_and_mask_follow_the_end_of_its_raster() {
    let mask = "%_/ArtDictionary :\r\n%_/Mask :\r\n%_X=\r\n%_0 g\r\n%_0 0 m\r\n%_5 0 l\r\n%_5 5 l\r\n%_f\r\n%_X+\r\n%_ (Art) ,\r\n\
                %_0 /Bool (Clipping) ,\r\n%_0 /Bool (Inverted) ,\r\n%_0 /Bool (Disabled) ,\r\n%_1 /Bool (Linked) ,\r\n%_; (AI9_Mask) ,\r\n\
                %_/XMLUID : (Photo) ; (AI10_ArtUID) ,\r\n%_;\r\n%_\r\n";
    for tail in [
        "XH\r\n%AI5_EndRaster\r\nN\r\n",
        "XH\r\n%AI17_Begin_Content_if_version_gt:24 17\r\n72 72 Xr\r\n%AI17_End_Versioned_Content\r\n%AI5_EndRaster\r\nN\r\n",
    ] {
        let mut data = image("[ 10 0 0 10 50 90 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 7);
        let at = data.windows(6).position(|w| w == b"XH\r\n%A").unwrap();
        let end = data.windows(6).position(|w| w == b"N\r\nLB\r").unwrap() + 3;
        data.splice(at..end, [tail, mask].concat().into_bytes());
        let r = read(&data).unwrap();
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{a:#?}");
        assert!(matches!(a[0].kind, NodeKind::Image(_)), "{:?}", a[0].kind);
        assert_eq!(a[0].name.as_deref(), Some("Photo"));
        assert!(a[0].extras().mask.is_some());
    }
    // An operator of another kind in between ends the image: the name has no object.
    let mut data = image("[ 10 0 0 10 50 90 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 7);
    let end = data.windows(6).position(|w| w == b"N\r\nLB\r").unwrap() + 3;
    data.splice(end..end, b"0 A\r\n%_/ArtDictionary :\r\n%_/XMLUID : (Photo) ; (AI10_ArtUID) ,\r\n%_;\r\n%_\r\n".to_vec());
    assert_eq!(art(&read(&data).unwrap().doc)[0].name, None);
}

fn pixels(doc: &Document) -> (image::RgbaImage, kurbo::Affine) {
    let a = art(doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let blob = doc.images.get(&im.key).unwrap();
    (image::load_from_memory(&blob.bytes).unwrap().to_rgba8(), im.xf)
}

/// A 1-bit image's row holds each pixel's channels in turn, one bit each, padded to whole bytes;
/// a set bit is the channel at full strength.
#[test]
fn one_bit_images_read_each_channel() {
    let one_bit = |src: Vec<u8>, kind: u32| {
        let text = String::from_utf8(src).unwrap();
        text.replacen(&format!(" 8 {kind} 0 0 1 0 "), &format!(" 1 {kind} 0 0 0 0 "), 1).into_bytes()
    };
    // 3 × 2 RGB in hex: red, green, blue (100 010 001), then white, black, yellow (111 000 110).
    let doc = read(&one_bit(image("[ 1 0 0 1 0 100 ]", (3, 2), ("DeviceRGB", 3, 0), b"8880e300", 11), 3)).unwrap().doc;
    let img = pixels(&doc).0;
    let row = |y: u32| (0..3).map(|x| img.get_pixel(x, y).0).collect::<Vec<_>>();
    assert_eq!(row(0), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
    assert_eq!(row(1), [[255, 255, 255, 255], [0, 0, 0, 255], [255, 255, 0, 255]]);
    // 2 × 1 CMYK: cyan, then black (1000 0001).
    let doc = read(&one_bit(image("[ 1 0 0 1 0 100 ]", (2, 1), ("DeviceCMYK", 4, 0), b"81", 5), 4)).unwrap().doc;
    let img = pixels(&doc).0;
    for (x, ink) in [[1.0, 0.0, 0.0, 0.0], [0.0, 0.0, 0.0, 1.0]].into_iter().enumerate() {
        let [r, g, b, _] = Color::cmyk(ink[0], ink[1], ink[2], ink[3]).to_rgba8(1.0);
        assert_eq!(img.get_pixel(x as u32, 0).0, [r, g, b, 255], "pixel {x}");
    }
    // 3 × 1 gray: a bit a pixel (101).
    let doc = read(&one_bit(image("[ 1 0 0 1 0 100 ]", (3, 1), ("DeviceGray", 1, 0), b"a0", 5), 1)).unwrap().doc;
    let img = pixels(&doc).0;
    assert_eq!((0..3).map(|x| img.get_pixel(x, 0).0[0]).collect::<Vec<_>>(), [255, 0, 255]);
}

/// A gray image `w` × 1 in hex with ImageMask `mask` and `bits` bits a pixel, after the fill `fill`.
fn masked_gray(fill: &str, bits: u32, mask: u32, w: u32, hex: &[u8]) -> Vec<u8> {
    let text = String::from_utf8(image("[ 1 0 0 1 0 100 ]", (w, 1), ("DeviceGray", 1, 0), hex, hex.len())).unwrap();
    text.replacen(" 8 1 0 0 1 0 ", &format!(" {bits} 1 0 0 0 {mask} "), 1).replacen("(L) Ln\n", &format!("(L) Ln\n{fill}"), 1).into_bytes()
}

/// ImageMask 1 on a 1-bit gray image: the image is colorized with the fill color, so its black
/// pixels take the color and its white ones are transparent. ImageMask 0 keeps the opaque image.
#[test]
fn image_masks_are_colorized_with_the_fill() {
    // 4 × 1 at 1 bit: black, white, white, black (0110).
    let r = read(&masked_gray("0 0 1 0 k\n", 1, 1, 4, b"60")).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let a = art(&r.doc);
    let Some(Paint::Solid { color, .. }) = a[0].extras().colorize.as_deref() else { panic!("{:?}", a[0].extras().colorize) };
    assert_eq!(color.to_rgba8(1.0), Color::cmyk(0.0, 0.0, 1.0, 0.0).to_rgba8(1.0));
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let blob = &r.doc.images[&im.key];
    assert!(blob.gray_source().is_some_and(|g| !g.alpha), "an image a fill color can colorize");
    let img = pixels(&r.doc).0;
    assert_eq!(
        (0..4).map(|x| img.get_pixel(x, 0).0).collect::<Vec<_>>(),
        [[0, 0, 0, 255], [255, 255, 255, 255], [255, 255, 255, 255], [0, 0, 0, 255]]
    );
    // How much of the color each pixel shows: the black ones only.
    assert_eq!(blob.coverage().map(|c| c.2), Some(vec![255, 0, 0, 255]));
    // A spot color keeps its swatch and tint.
    let spot = "0 0 1 0 (Ink) 0.5 0 Xx\n";
    let a = art(&read(&masked_gray(spot, 1, 1, 4, b"60")).unwrap().doc);
    assert!(
        matches!(a[0].extras().colorize.as_deref(), Some(Paint::Solid { swatch: Some(s), tint, .. }) if s == "Ink" && (*tint - 0.5).abs() < 1e-6),
        "{:?}",
        a[0].extras().colorize
    );
    // A fill that isn't a solid color (here a pattern) colorizes it black.
    let patterned = String::from_utf8(masked_gray("(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n", 1, 1, 4, b"60")).unwrap();
    let doc = read(patterned.replacen("%AI5_BeginLayer", &format!("{DOTS}%AI5_BeginLayer"), 1).as_bytes()).unwrap().doc;
    let a = art(&doc);
    let Some(Paint::Solid { color, swatch: None, .. }) = a[0].extras().colorize.as_deref() else { panic!("{:?}", a[0].extras().colorize) };
    assert_eq!(color.to_rgba8(1.0), Color::BLACK.to_rgba8(1.0));
    // An 8-bit gray image with ImageMask 1 opens as before, with no color.
    let doc = read(&masked_gray("0 1 0 0 k\n", 8, 1, 2, b"00ff")).unwrap().doc;
    assert!(art(&doc)[0].extras().colorize.is_none());
    assert_eq!(pixels(&doc).0.get_pixel(1, 0).0, [255, 255, 255, 255]);
    // ImageMask 0: the opaque image, with no color.
    let doc = read(&masked_gray("0 0 1 0 k\n", 1, 0, 4, b"60")).unwrap().doc;
    assert!(art(&doc)[0].extras().colorize.is_none());
    assert_eq!(pixels(&doc).0.get_pixel(0, 0).0, [0, 0, 0, 255]);
    // An RGB image can't take a color: ImageMask 1 leaves it as it is.
    let text = String::from_utf8(image("[ 1 0 0 1 0 100 ]", (1, 1), ("DeviceRGB", 3, 0), b"ff0000", 6)).unwrap();
    let doc = read(text.replacen(" 8 3 0 0 1 0 ", " 8 3 0 0 0 1 ", 1).as_bytes()).unwrap().doc;
    assert!(art(&doc)[0].extras().colorize.is_none());
    assert_eq!(pixels(&doc).0.get_pixel(0, 0).0, [255, 0, 0, 255]);
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

/// Editing data `data` opens without its one image and with the warning "an image VectorCraft can't read was left out" once.
fn image_left_out_with_a_warning(data: &[u8]) {
    let r = read(data).unwrap();
    assert!(art(&r.doc).is_empty(), "{:#?}", art(&r.doc));
    let left_out = r.warnings.iter().filter(|w| *w == "an image VectorCraft can't read was left out").count();
    assert_eq!(left_out, 1, "{:?}", r.warnings);
}

#[test]
fn an_image_without_a_matrix_is_left_out_with_a_warning() {
    image_left_out_with_a_warning(&image("", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3));
}

#[test]
fn an_image_with_too_few_operands_is_left_out_with_a_warning() {
    // The operands hold 10 numbers after the matrix, one short of the binary flag.
    let text = String::from_utf8(image("[ 1 0 0 1 0 100 ]", (1, 1), ("DeviceRGB", 3, 0), b"abc", 3)).unwrap();
    let data = text.replacen("] 0 0 1 1 1 1 8 3 0 0 1 0 4 4 0 0\n", "] 0 0 1 1 1 1 8 3 0 0\n", 1);
    assert_ne!(data, text);
    image_left_out_with_a_warning(data.as_bytes());
}

#[test]
fn an_image_whose_matrix_isnt_six_numbers_is_left_out_with_a_warning() {
    image_left_out_with_a_warning(&image("[ 1 0 0 1 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3));
}

#[test]
fn an_image_whose_matrix_has_no_area_is_left_out_with_a_warning() {
    image_left_out_with_a_warning(&image("[ 0 0 0 0 0 0 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3));
    // This matrix maps the image onto a line.
    image_left_out_with_a_warning(&image("[ 1 2 2 4 0 100 ]", (1, 1), ("DeviceRGB", 3, 0), &[255, 0, 0], 3));
}

/// The raster matrix `[a b c d tx ty]` is an art-space matrix (y up) applied to the image's pixels
/// with their rows running downwards, `(u, -v)`: art x = a·u − c·v + tx, art y = b·u − d·v + ty.
/// Without rotation (b = c = 0) that is the same as flipping the matrix's result; a rotated image
/// tells the two apart (#2269).
#[test]
fn a_rotated_image_is_placed_as_its_matrix_maps_it() {
    // Turned 90° counter-clockwise: in art space each row runs upwards, the next row to its right.
    let doc = read(&image("[ 0 10 -10 0 50 50 ]", (2, 1), ("DeviceRGB", 3, 0), &[255, 0, 0, 0, 0, 255], 6)).unwrap().doc;
    let xf = pixels(&doc).1;
    // Art (x, y) is (x, 100 - y) on the page.
    let page = |x: f64, y: f64| Point::new(x, 100.0 - y);
    assert_eq!(xf * Point::ZERO, page(50.0, 50.0), "the top left pixel's corner stays at the matrix's origin");
    assert_eq!(xf * Point::new(2.0, 0.0), page(50.0, 70.0), "the first row runs up");
    assert_eq!(xf * Point::new(0.0, 1.0), page(60.0, 50.0), "the next row is to its right");
    // Any rotation and scale: the same formula.
    let (s, c) = 30f64.to_radians().sin_cos();
    let m = [3.0 * c, 3.0 * s, -2.0 * s, 2.0 * c, 40.0, 60.0];
    let matrix = format!("[ {} ]", m.map(|v| v.to_string()).join(" "));
    let doc = read(&image(&matrix, (2, 1), ("DeviceRGB", 3, 0), &[255, 0, 0, 0, 0, 255], 6)).unwrap().doc;
    let xf = pixels(&doc).1;
    for (u, v) in [(0.0, 0.0), (2.0, 0.0), (0.0, 1.0), (2.0, 1.0)] {
        let want = page(m[0] * u - m[2] * v + m[4], m[1] * u - m[3] * v + m[5]);
        let got = xf * Point::new(u, v);
        assert!((got - want).hypot() < 1e-9, "pixel ({u}, {v}): {got:?}, want {want:?}");
    }
}

/// A raster object in the version 7 form (§8): `%AI5_BeginRaster`, `head` (a link's `XG`, or
/// nothing), the operands `[matrix] 0 0 w h w h 8 3 0 0 0 0`, the operator `op` and `tail` (an
/// `XI`'s samples on `%` lines), `%AI5_EndRaster`.
fn raster7(head: &str, matrix: &str, (w, h): (u32, u32), op: &str, tail: &str) -> String {
    format!("%AI5_File:\n%AI5_BeginRaster\n{head}{matrix} 0 0 {w} {h} {w} {h} 8 3 0 0 0 0 {op}\n{tail}%AI5_EndRaster\n")
}

/// A linked 1-bit gray image with ImageMask 1 keeps the fill color as the color that colorizes it,
/// as an embedded one does. Other linked images have no color.
#[test]
fn linked_image_masks_are_colorized_with_the_fill() {
    let m = "[ 1 0 0 1 0 0 ]";
    let linked = |bits_kind_mask: &str| {
        let xf = raster7("(a.tif) 0 XG\n", m, (2, 2), "XF", "").replace(" 8 3 0 0 0 0 XF", &format!(" {bits_kind_mask} XF"));
        let r = read(stream7(&layer7("L", &format!("0 0 1 0 (Ink) 0.5 0 Xx\n{xf}"))).as_bytes()).unwrap();
        let a = art(&r.doc);
        assert!(matches!(&a[0].kind, NodeKind::Image(im) if im.link.is_some()), "{:?} {:?}", a, r.warnings);
        a[0].extras().colorize.as_deref().cloned()
    };
    assert!(
        matches!(linked("1 1 0 0 0 1"), Some(Paint::Solid { swatch: Some(s), tint, .. }) if s == "Ink" && (tint - 0.5).abs() < 1e-6),
        "a 1-bit image mask"
    );
    assert_eq!(linked("1 1 0 0 0 0"), None, "ImageMask 0");
    assert_eq!(linked("8 1 0 0 0 1"), None, "8-bit gray");
    assert_eq!(linked("8 3 0 0 0 1"), None, "RGB");
}

/// A linked image in the version 7 form (§8.2, §8.3), `(path) modified XG` and then `XF` with no
/// samples, is a linked image of that file. The data keeps no pixels of it, so it shows a
/// placeholder over the place the matrix maps the image's pixels to, as an embedded image (`XI`)
/// of the same operands is placed.
#[test]
fn a_version_7_linked_image_shows_a_placeholder_where_the_image_is() {
    let (s, c) = 30f64.to_radians().sin_cos();
    for m in [[2.0, 0.0, 0.0, 3.0, 100.0, 400.0], [3.0 * c, 3.0 * s, -2.0 * s, 2.0 * c, 40.0, 60.0]] {
        let matrix = format!("[ {} ]", m.map(|v| v.to_string()).join(" "));
        let linked = raster7("(Macintosh HD:Images:photo.tif) 0 XG\n", &matrix, (10, 20), "XF", "");
        let embedded = raster7("", &matrix, (10, 20), "XI", &"%FF0000\n".repeat(200));
        let r = read(stream7(&layer7("L", &format!("{linked}{embedded}"))).as_bytes()).unwrap();
        let a = art(&r.doc);
        assert_eq!(a.len(), 2, "{:?}", r.warnings);
        let (NodeKind::Image(link), NodeKind::Image(emb)) = (&a[0].kind, &a[1].kind) else { panic!("{:?}", a) };
        assert_eq!(link.link.as_ref().map(|l| l.path.as_str()), Some("Macintosh HD:Images:photo.tif"));
        assert_eq!(emb.link, None, "an image whose samples follow stays embedded");
        assert!(r.doc.images[&link.key].is_proxy(), "the placeholder is what a missing link shows");
        // The file's pixels, placed as the embedded image's are: relinking keeps that size.
        assert_eq!((link.width, link.height, link.xf), (emb.width, emb.height, emb.xf));
        assert_eq!((link.width, link.height), (10, 20));
        assert_eq!(r.warnings.iter().filter(|w| *w == read::LINKED_PLACEHOLDERS).count(), 1, "{:?}", r.warnings);
        assert!(!r.warnings.iter().any(|w| w.contains("left out")), "{:?}", r.warnings);
    }
    // Images of other files keep images of their own, though each shows the same placeholder; two
    // of one file share theirs.
    let m = "[ 1 0 0 1 0 100 ]";
    let three = ["(a.tif) 0 XG\n", "(b.tif) 0 XG\n", "(a.tif) 0 XG\n"].map(|xg| raster7(xg, m, (2, 2), "XF", "")).concat();
    let r = read(stream7(&layer7("L", &three)).as_bytes()).unwrap();
    let keys: Vec<_> = art(&r.doc).iter().filter_map(|n| if let NodeKind::Image(im) = &n.kind { Some(im.key.clone()) } else { None }).collect();
    assert!(matches!(&keys[..], [a, b, c] if a != b && a == c), "{keys:?}");
    assert_eq!(r.doc.images.len(), 2);
}

/// `XG` names the file of the `XF` after it only: an `XF` without a file name before it, after an
/// `XG` with operands of another shape or an empty path, or after an image that took the name,
/// is left out with a warning, as is one whose operands `XI` couldn't read. Of two `XG`s, the
/// second names the file.
#[test]
fn a_version_7_linked_image_without_its_file_name_is_left_out_with_a_warning() {
    let m = "[ 1 0 0 1 0 100 ]";
    let embedded = raster7("() 1 XG\n", m, (1, 1), "XI", "%FF0000\n");
    for (what, src) in [
        ("no XG", raster7("", m, (2, 2), "XF", "")),
        ("a name, not a string", raster7("/photo 0 XG\n", m, (2, 2), "XF", "")),
        ("no modified flag", raster7("(a.tif) XG\n", m, (2, 2), "XF", "")),
        ("an empty path", raster7("() 0 XG\n", m, (2, 2), "XF", "")),
        ("taken by an embedded image", format!("{}{}", raster7("(a.tif) 0 XG\n", m, (1, 1), "XI", "%FF0000\n"), raster7("", m, (2, 2), "XF", ""))),
        ("named in an earlier raster object", format!("%AI5_BeginRaster\n(a.tif) 0 XG\n%AI5_EndRaster\n{}", raster7("", m, (2, 2), "XF", ""))),
        ("named outside a raster object", format!("(a.tif) 0 XG\n{}", raster7("", m, (2, 2), "XF", ""))),
        ("too few operands", raster7("(a.tif) 0 XG\n", m, (2, 2), "XF", "").replace(" 8 3 0 0 0 0 XF", " 8 3 0 XF")),
        ("no matrix", raster7("(a.tif) 0 XG\n", "", (2, 2), "XF", "")),
        ("a matrix with no area", raster7("(a.tif) 0 XG\n", "[ 0 0 0 0 0 0 ]", (2, 2), "XF", "")),
        ("no pixels", raster7("(a.tif) 0 XG\n", m, (0, 2), "XF", "")),
        ("a side past the largest", raster7("(a.tif) 0 XG\n", m, (2_000_000, 2), "XF", "")),
        ("samples of 7 bits", raster7("(a.tif) 0 XG\n", m, (2, 2), "XF", "").replace(" 8 3 0 0 0 0 XF", " 7 3 0 0 0 0 XF")),
    ] {
        let body = format!("{embedded}{src}");
        let r = read(stream7(&layer7("L", &body)).as_bytes()).unwrap();
        let a = art(&r.doc);
        assert_eq!(a.len(), body.matches(" XI\n").count(), "{what}: {:?}", r.warnings);
        assert!(a.iter().all(|n| matches!(&n.kind, NodeKind::Image(im) if im.link.is_none())), "{what}: the embedded images only");
        let left_out = r.warnings.iter().filter(|w| *w == "an image VectorCraft can't read was left out").count();
        assert_eq!(left_out, 1, "{what}: {:?}", r.warnings);
        assert!(!r.warnings.iter().any(|w| w == read::LINKED_PLACEHOLDERS), "{what}: {:?}", r.warnings);
    }
    // More pixels than an embedded image may have: no pixels are read, so the linked image comes in.
    let r = read(stream7(&layer7("L", &raster7("(a.tif) 0 XG\n", m, (100_000, 100_000), "XF", ""))).as_bytes()).unwrap();
    let a = art(&r.doc);
    assert!(matches!(&a[..], [n] if matches!(&n.kind, NodeKind::Image(im) if (im.width, im.height) == (100_000, 100_000))), "{a:?}");
    assert!(!r.warnings.iter().any(|w| w == read::IMAGE_TOO_LARGE), "{:?}", r.warnings);
    let twice = raster7("(a.tif) 0 XG\n(b.tif) 0 XG\n", m, (2, 2), "XF", "");
    let r = read(stream7(&layer7("L", &twice)).as_bytes()).unwrap();
    let a = art(&r.doc);
    assert!(matches!(&a[..], [n] if matches!(&n.kind, NodeKind::Image(im) if im.link.as_ref().is_some_and(|l| l.path == "b.tif"))), "{a:?}");
}

/// CMYK images keep their ink amounts, as the file's PDF part keeps them, and an alpha channel
/// becomes their opacity mask: a grayscale image over the image.
#[test]
fn cmyk_images_keep_their_ink_amounts() {
    // 3 × 1: cyan, 100% black, then a mix.
    let inks = [255, 0, 0, 0, 0, 0, 0, 255, 12, 34, 56, 78];
    let doc = read(&image("[ 10 0 0 10 50 90 ]", (3, 1), ("DeviceCMYK", 4, 0), &inks, 14)).unwrap().doc;
    let a = art(&doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let blob = &doc.images[&im.key];
    assert!(blob.is_cmyk(), "{}", blob.mime);
    assert_eq!(blob.cmyk().map(|i| (i.width, i.height, i.data)), Some((3, 1, inks.to_vec())));
    assert!(a[0].extras().mask.is_none(), "no alpha channel, no mask");
    // The same samples with an alpha channel.
    let data = [&inks[..], &[255, 128, 0]].concat();
    let doc = read(&image("[ 10 0 0 10 50 90 ]", (3, 1), ("DeviceCMYK", 4, 1), &data, 17)).unwrap().doc;
    let a = art(&doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(doc.images[&im.key].cmyk().map(|i| i.data), Some(inks.to_vec()), "the inks are the same");
    let m = a[0].extras().mask.clone().expect("the alpha channel is an opacity mask");
    assert!(m.clip);
    let NodeKind::Image(mi) = &m.art.kind else { panic!("{:?}", m.art.kind) };
    assert_eq!((mi.width, mi.height, mi.xf), (3, 1, im.xf), "the mask covers the image");
    let gray = image::load_from_memory(&doc.images[&mi.key].bytes).unwrap().to_luma8();
    assert_eq!(gray.into_raw(), [255, 128, 0]);
    // An alpha channel that hides nothing adds no mask.
    let data = [&inks[..], &[255; 3]].concat();
    let doc = read(&image("[ 10 0 0 10 50 90 ]", (3, 1), ("DeviceCMYK", 4, 1), &data, 17)).unwrap().doc;
    let a = art(&doc);
    assert!(a[0].extras().mask.is_none());
    // Hex samples, and a four-channel image whose space the data doesn't name.
    let hex: String = inks.iter().map(|b| format!("{b:02x}")).collect();
    let text = String::from_utf8(image("[ 1 0 0 1 0 100 ]", (3, 1), ("Other", 4, 0), hex.as_bytes(), 26)).unwrap();
    let doc = read(text.replacen("8 4 0 0 1 0 4 4 0 0", "8 4 0 0 0 0 4 4 0 0", 1).as_bytes()).unwrap().doc;
    let NodeKind::Image(im) = &art(&doc)[0].kind else { panic!() };
    assert_eq!(doc.images[&im.key].cmyk().map(|i| i.data), Some(inks.to_vec()));
}

/// Editing data with one layer (showing or not) holding a placed file: its matrix, the file at
/// `path` in the box 10 20 14 22, and `preview` (what its preview section holds), then `after`.
fn placed(visible: bool, matrix: &str, path: &str, preview: Option<&[u8]>, after: &str) -> Vec<u8> {
    let text = stream(&layer("@@").replacen("1 1 1 1", if visible { "1 1 1 1" } else { "0 1 1 1" }, 1));
    let (head, tail) = text.split_once("@@").unwrap();
    let mut s = format!(
        "{head}%AI5_BeginPlace\r%%FileType: 1\r{matrix} 10 20 14 22 4 4 0\r({path})`\r%%IncludeFile:{path}\r\
         %AI17_Begin_Content_if_version_gt:24 12\r%AI28_BeginPlacedRelativePath\r()\r%AI28_EndPlacedRelativePath\r%AI17_Alternate_Content\r%AI17_End_Versioned_Content\r\
         %AI17_Begin_Content_if_version_gt:24 5\r"
    )
    .into_bytes();
    if let Some(p) = preview {
        s.extend_from_slice(b"%AI26_BeginPlacedObjectPreview\r");
        s.extend_from_slice(p);
        s.extend_from_slice(b"\r%AI26_EndPlacedObjectPreview\r");
    }
    s.extend_from_slice(format!("%AI17_End_Versioned_Content\r~\r%AI5_EndPlace\r{after}{tail}").as_bytes());
    s
}

/// [`placed`]'s file with its placed file written as the copy of an object on `%_` lines (the
/// editable object whose drawn look comes before it): the lines from `%AI5_BeginPlace` to
/// `%AI5_EndPlace` start with `%_`, the versioned content's markers aside. The preview's header
/// line does too; its samples follow it raw.
fn hide_placed(data: &[u8]) -> Vec<u8> {
    let find = |w: &[u8]| data.windows(w.len()).position(|x| x == w).unwrap();
    let (from, to) = (find(b"%AI5_BeginPlace"), find(b"%AI5_EndPlace\r") + 14);
    let mut out = data[..from].to_vec();
    for line in data[from..to].split(|b| *b == b'\r') {
        if !line.is_empty() && !line.starts_with(b"%AI17_") {
            out.extend_from_slice(b"%_");
        }
        out.extend_from_slice(line);
        out.push(b'\r');
    }
    out.pop();
    out.extend_from_slice(&data[to..]);
    out
}

#[test]
fn a_placed_file_on_commented_lines_is_read() {
    // Its `%_%AI5_BeginPlace` was dropped as a comment after `%_`, so the `~` that ends it was an
    // operator the reader didn't know, and the whole file came in as its PDF part; and its
    // preview, whose header starts with `%_`, wasn't read.
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let data = hide_placed(&placed(true, "[5 0 0 -5 0 200]", "/pics/photo.png", Some(&preview), ""));
    assert!(data.windows(18).any(|w| w == b"%_%AI5_BeginPlace\r"), "{}", String::from_utf8_lossy(&data));
    let r = read(&data).unwrap();
    let a = art(&r.doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!((im.link.as_ref().map(|l| l.path.as_str()), im.width, im.height), (Some("/pics/photo.png"), 2, 1));
    let (img, _) = pixels(&r.doc);
    assert_eq!((img.get_pixel(0, 0).0, img.get_pixel(1, 0).0), ([255, 0, 0, 255], [0, 127, 0, 128]), "its preview");
}

/// [`placed`]'s file whose relative folder section holds `folder` (written as it is, parentheses
/// included) instead of `()`.
fn placed_in(folder: &str, path: &str, preview: &[u8]) -> Vec<u8> {
    let data = placed(true, "[5 0 0 -5 0 200]", path, Some(preview), "");
    let (at, len) = (data.windows(4).position(|w| w == b"\r()\r").unwrap(), 4);
    let mut out = data[..at].to_vec();
    out.extend_from_slice(format!("\r{folder}\r").as_bytes());
    out.extend_from_slice(&data[at + len..]);
    out
}

/// The relative path of the one linked image of `data`.
fn relative_of(data: &[u8]) -> Option<String> {
    let r = read(data).unwrap();
    let a = art(&r.doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(im.link.as_ref().map(|l| l.path.as_str()), Some("/old/place/src/photo.png"));
    im.link.as_ref().and_then(|l| l.relative.clone())
}

#[test]
fn a_placed_file_keeps_its_folder_relative_to_the_document() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let path = "/old/place/src/photo.png";
    assert_eq!(relative_of(&placed_in("(src/)", path, &preview)).as_deref(), Some("src/photo.png"));
    assert_eq!(relative_of(&placed_in("(../art)", path, &preview)).as_deref(), Some("../art/photo.png"));
    assert_eq!(relative_of(&placed_in(r"(..\\art\\)", path, &preview)).as_deref(), Some("../art/photo.png"));
    assert_eq!(relative_of(&placed_in("( art /)", path, &preview)).as_deref(), Some(" art /photo.png"), "spaces are part of names");
    // On `%_` lines too.
    assert_eq!(relative_of(&hide_placed(&placed_in("(src/)", path, &preview))).as_deref(), Some("src/photo.png"));
    // An empty folder is the document's own: the file's name.
    assert_eq!(relative_of(&placed_in("()", path, &preview)).as_deref(), Some("photo.png"));
    // An absolute folder, one too long or not a string: the path alone, and the file still opens.
    let long = format!("({})", "a/".repeat(3000));
    for folder in ["(/elsewhere/)", "(C:\\\\art\\\\)", long.as_str(), "12", "/src"] {
        assert_eq!(relative_of(&placed_in(folder, path, &preview)), None, "{folder}");
    }
    // A section that doesn't end leaves the folder unread.
    let data = placed_in("(src/)", path, &preview);
    let end = data.windows(27).position(|w| w == b"%AI28_EndPlacedRelativePath").unwrap();
    let mut cut = data[..end].to_vec();
    cut.extend_from_slice(b"%AI28_Other");
    cut.extend_from_slice(&data[end + 27..]);
    assert_eq!(relative_of(&cut), None);
}

#[test]
fn hidden_placed_file_ended_by_comment_stays_hidden() {
    let mut data = hide_placed(&placed(true, "[5 0 0 -5 0 200]", "/pics/photo.png", None, ""));
    let end = data.windows(4).position(|w| w == b"%_~\r").unwrap();
    data.drain(end..end + 4);
    let r = read(&data).unwrap();
    assert!(art(&r.doc).is_empty());
    assert!(r.unread.is_empty(), "hidden placement has no shown placeholder");
    assert!(r.hidden_unread.contains("placed files"));
}

#[test]
fn previewless_placement_records_box_corner_as_alternative() {
    let mut data = placed(true, "[1 0 0 -1 -7970 8070]", "missing.png", None, "");
    let at = data.windows(15).position(|w| w == b"%AI5_BeginLayer").unwrap();
    data.splice(at..at, *b"%AI3_TemplateBox: 100 50 100 50\n");
    let r = read(&data).unwrap();
    assert_unread(&r, "placed files");
    assert_eq!(r.unread_elsewhere.len(), 1);
    assert_eq!(r.unread_elsewhere[0].0, r.unread[0].0);
    assert_ne!(r.unread_elsewhere[0].1, r.doc.node(r.unread[0].0).unwrap().visual_bounds().unwrap());
}

/// Editing data in the version 7 format (§2): art size 200 × 100 around the template box's centre
/// (0, 0), then `body`.
fn placed_stream7(body: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%%BoundingBox: 0 0 200 100\n%AI5_FileFormat 2.0\n%AI5_ArtSize: 200 100\n\
         %AI3_TemplateBox: 0 0 0 0\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n\
         %%PageTrailer\nshowpage\n%%Trailer\n%%EOF\n"
    )
}

/// One layer of the version 7 format (ten operands before `Lb`), showing or not, holding `art`.
fn placed_layer7(visible: bool, art: &str) -> String {
    format!("%AI5_BeginLayer\n{} 1 1 1 0 0 1 79 128 255 Lb\n(L) Ln\n0 A\n{art}\nLB\n%AI5_EndLayer--\n", u8::from(visible))
}

fn placed_read7(visible: bool, art: &str) -> Result<Structure, String> {
    read(placed_stream7(&placed_layer7(visible, art)).as_bytes())
}

/// The placed file's box (`llx lly urx ury`) taken by `matrix` into the document: where the
/// version 7 format puts it (§16.1).
fn placed_box(r: &Structure, matrix: [f64; 6], b: [f64; 4]) -> Rect {
    (r.to_doc * kurbo::Affine::new(matrix)).transform_rect_bbox(Rect::new(b[0], b[1], b[2], b[3]))
}

/// The placeholder's bounds and the other bounds [`Structure::unread_elsewhere`] keeps for it.
fn placeholder_boxes(r: &Structure) -> Vec<Rect> {
    let id = r.unread[0].0;
    let mut boxes = vec![r.doc.node(id).unwrap().visual_bounds().unwrap()];
    boxes.extend(r.unread_elsewhere.iter().filter(|(o, _)| *o == id).map(|(_, b)| *b));
    boxes
}

fn near(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-6)
}

#[test]
fn version7_placed_files_are_read_without_a_begin_place() {
    // `~` was an operator the reader didn't know, so the file's layers weren't read.
    let matrices = [
        [1.0, 0.0, 0.0, 1.0, 10.0, 20.0],
        [2.0, 0.0, 0.0, 0.5, -5.0, 3.0],
        [-1.29904, 0.75, 0.375, 0.649519, 125.8461, 159.7668],
        [-1.0, 0.0, 0.0, 1.0, 100.0, 0.0],
    ];
    for m in matrices {
        for extra in ["", " 4 4 0"] {
            let [a, b, c, d, e, f] = m;
            let body = format!(
                "[{a} {b} {c} {d} {e} {f}] 10 20 50 50{extra} (art.eps) `\n%%IncludeFile: art.eps\n~\n{RED}{}f",
                rect_path(0.0, 0.0, 5.0, 5.0)
            );
            let r = placed_read7(true, &body).unwrap_or_else(|e| panic!("{m:?}{extra}: {e}"));
            assert_eq!(art_kinds(&r.doc), ["path", "path"], "{m:?}{extra}: a placeholder, then the path after it");
            assert_unread(&r, "placed files");
            let want = placed_box(&r, m, [10.0, 20.0, 50.0, 50.0]);
            assert!(placeholder_boxes(&r).iter().any(|b| near(*b, want)), "{m:?}{extra}: {:?} {want:?}", placeholder_boxes(&r));
        }
    }
}

/// The kinds of the first layer's objects.
fn art_kinds(doc: &Document) -> Vec<&'static str> {
    art(doc)
        .iter()
        .map(|n| match n.kind {
            NodeKind::Group { .. } => "group",
            NodeKind::Path { .. } => "path",
            NodeKind::Image(_) => "image",
            _ => "other",
        })
        .collect()
}

#[test]
fn version7_placed_file_included_whole_is_read() {
    // The included document's operators, a `~`, a nested document and counted data in it aren't
    // the editing data's.
    let eps = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 10 20 50 50\n%%EndComments\n1 0 0 setrgbcolor frobnicate ~\n\
               %%BeginDocument: inner.eps\n~\n%%EndDocument\n%%BeginData: 14 Binary Bytes\n%%EndDocument\n%%EOF\n";
    let body = format!("[1 0 0 1 10 20] 10 20 50 50 (art.eps) `\n%%BeginDocument: art.eps\n{eps}%%EndDocument\n~\n");
    let r = placed_read7(true, &body).unwrap();
    assert_eq!(art_kinds(&r.doc), ["path"]);
    assert_unread(&r, "placed files");
    // One that doesn't end leaves the layer without its end.
    let cut = format!("[1 0 0 1 10 20] 10 20 50 50 (art.eps) `\n%%BeginDocument: art.eps\n{eps}~\n");
    assert!(placed_read7(true, &cut).is_err());
}

#[test]
fn version7_placed_file_without_a_file_is_read() {
    // `` ` `` followed by `~`: a placed file whose file isn't there.
    let r = placed_read7(true, "[1 0 0 1 10 20] 10 20 50 50 (gone.eps) `\n~\n").unwrap();
    assert_unread(&r, "placed files");
    // In a hidden layer it is left out, with a note.
    let r = placed_read7(false, "[1 0 0 1 10 20] 10 20 50 50 (gone.eps) `\n%%IncludeFile: gone.eps\n~\n").unwrap();
    assert!(r.unread.is_empty() && art(&r.doc).is_empty());
    assert!(r.hidden_unread.contains("placed files"), "{:?}", r.hidden_unread);
}

#[test]
fn version7_placed_file_in_a_group_keeps_its_place() {
    // In a group, the group is what isn't read.
    let body = format!("u\n{RED}{}f\n[1 0 0 1 10 20] 10 20 50 50 (art.eps) `\n%%IncludeFile: art.eps\n~\nU\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let r = placed_read7(true, &body).unwrap();
    assert_eq!(art_kinds(&r.doc), ["group"]);
    assert_eq!(r.unread.iter().map(|(id, kinds)| (*id, kinds.clone())).collect::<Vec<_>>(), [(art(&r.doc)[0].id, vec!["placed files".to_string()])]);
    // A locked one keeps its lock.
    let r = placed_read7(true, "1 A\n[1 0 0 1 10 20] 10 20 50 50 (art.eps) `\n%%IncludeFile: art.eps\n~\n").unwrap();
    assert_unread(&r, "placed files");
    assert!(r.doc.node(r.unread[0].0).unwrap().locked);
}

#[test]
fn placed_file_operands_of_another_shape_arent_read() {
    for operands in [
        // Five numbers in the matrix, three box numbers, two extra numbers, no path, a name for the
        // path, a number that isn't finite.
        "[1 0 0 1 10] 10 20 50 50 (a.eps)",
        "[1 0 0 1 10 20] 10 20 50 (a.eps)",
        "[1 0 0 1 10 20] 10 20 50 50 4 4 (a.eps)",
        "[1 0 0 1 10 20] 10 20 50 50",
        "[1 0 0 1 10 20] 10 20 50 50 /a.eps",
        "[1 0 0 1 10 20] 10 20 50 1e999 (a.eps)",
    ] {
        // Without a box to put a placeholder in, the file isn't read.
        let e = placed_read7(true, &format!("{operands} `\n~\n")).err().unwrap_or_default();
        assert!(e.contains("placed files"), "{operands}: {e}");
    }
}

#[test]
fn a_placed_file_with_a_zero_area_box_or_a_second_tick_isnt_read() {
    for body in [
        "[1 0 0 1 10 20] 10 20 10 50 (a.eps) `\n~\n",
        // The operands of a second `` ` `` before `~` don't take the first one's place.
        "[1 0 0 1 10 20] 10 20 50 50 (a.eps) `\n[1 0 0 1 0 0] 0 0 5 5 (b.eps) `\n~\n",
    ] {
        let e = placed_read7(true, body).err().unwrap_or_default();
        assert!(e.contains("placed files"), "{body}: {e}");
    }
    // A `%AI5_BeginPlace` while one is open: the open one isn't read either.
    let body = "%AI5_BeginPlace\n[1 0 0 1 10 20] 10 20 50 50 (a.eps) `\n%AI5_BeginPlace\n[1 0 0 1 0 0] 0 0 5 5 (b.eps) `\n~\n%AI5_EndPlace\n";
    assert!(placed_read7(true, body).err().unwrap_or_default().contains("placed files"));
    let r = placed_read7(false, body).unwrap();
    assert!(r.hidden_unread.contains("placed files") && art(&r.doc).is_empty());
    // One that no `~` ends isn't read; a hidden one, or one on a layer that doesn't show, is left
    // out with a note.
    let open = "[1 0 0 1 10 20] 10 20 50 50 (a.eps) `\n";
    assert!(placed_read7(true, &format!("%AI5_BeginPlace\n{open}")).err().unwrap_or_default().contains("placed files"));
    assert!(placed_read7(true, open).err().unwrap_or_default().contains("placed files"));
    for (visible, art) in [(true, format!("%_%AI5_BeginPlace\n%_{open}")), (true, format!("%_{open}")), (false, open.to_string())] {
        let r = placed_read7(visible, &art).unwrap_or_else(|e| panic!("{art}: {e}"));
        assert!(r.hidden_unread.contains("placed files"), "{art}: {:?}", r.hidden_unread);
    }
}

#[test]
fn a_tilde_without_a_placed_file_is_an_unknown_operator() {
    let e = placed_read7(true, "~\n").err().unwrap_or_default();
    assert!(e.contains('~'), "{e}");
    // An apostrophe isn't the placed file's operator.
    let e = placed_read7(true, "[1 0 0 1 10 20] 10 20 50 50 (a.eps) '\n~\n").err().unwrap_or_default();
    assert!(e.contains('\''), "{e}");
}

/// A previewless locked placement keeps its edit lock on the page-art placeholder.
#[test]
fn previewless_placement_placeholder_inherits_lock() {
    let mut data = placed(true, "[5 0 0 -5 0 200]", "missing.png", None, "");
    let at = data.windows(15).position(|w| w == b"%AI5_BeginPlace").unwrap();
    data.splice(at..at, *b"1 A\n");
    let result = read(&data).unwrap();
    assert_unread(&result, "placed files");
    let node = result.doc.node(result.unread[0].0).unwrap();
    assert!(node.locked);
    assert!(node.visible);
}

/// A placed file's XML id names the linked image it comes in as, for the variable library's
/// bindings.
#[test]
fn a_placed_files_xml_id_names_its_image() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let name = "%_/ArtDictionary :\r%_/XMLUID : (photo_x5F_1) ; (AI10_ArtUID) ,\r%_;\r";
    let r = read(&placed(true, "[5 0 0 -5 0 200]", "/pics/photo.png", Some(&preview), name)).unwrap();
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Image(_)), "{:?}", a[0].kind);
    assert_eq!(r.xml_ids.get("photo_x5F_1"), Some(&Some(a[0].id)));
}

/// The XML ids kept stay within `MAX_XML_ID_BYTES` in all: an id that would pass it is left out,
/// and a shorter one after it still fits.
#[test]
fn xml_ids_stay_within_their_byte_budget() {
    use vectorcraft_doc::NodeId;
    let (mut ids, mut bytes) = (std::collections::HashMap::new(), 0);
    super::read::remember_xml_id(&mut ids, &mut bytes, "a".repeat(super::read::MAX_XML_ID_BYTES - 4), NodeId(1));
    super::read::remember_xml_id(&mut ids, &mut bytes, "long_id".into(), NodeId(2));
    super::read::remember_xml_id(&mut ids, &mut bytes, "id_2".into(), NodeId(3));
    super::read::remember_xml_id(&mut ids, &mut bytes, "id_2".into(), NodeId(4));
    assert_eq!(ids.len(), 2);
    assert!(!ids.contains_key("long_id"));
    assert_eq!(ids.get("id_2"), Some(&None), "two objects with one id are neither of them that id's");
    assert_eq!(bytes, super::read::MAX_XML_ID_BYTES);
}

/// Placed files of other paths keep images of their own though their previews are alike; two of
/// one path share theirs.
#[test]
fn placed_files_of_other_paths_keep_images_of_their_own() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let object = |path: &str| {
        let data = placed(true, "[5 0 0 -5 0 200]", path, Some(&preview), "");
        let find = |s: &[u8]| data.windows(s.len()).position(|w| w == s).unwrap();
        data[find(b"%AI5_BeginPlace")..find(b"%AI5_EndPlace\r") + 14].to_vec()
    };
    let mut data = placed(true, "[5 0 0 -5 0 200]", "a.png", Some(&preview), "");
    let end = data.windows(14).position(|w| w == b"%AI5_EndPlace\r").unwrap() + 14;
    data.splice(end..end, [object("b.png"), object("a.png")].concat());
    let r = read(&data).unwrap();
    let keys: Vec<_> = art(&r.doc).iter().filter_map(|n| if let NodeKind::Image(im) = &n.kind { Some(im.key.clone()) } else { None }).collect();
    assert!(matches!(&keys[..], [a, b, c] if a != b && a == c), "{keys:?}");
    assert_eq!(r.doc.images.len(), 2);
}

/// A placed file followed by the dictionary `numbers` (`value /Int (key)` lines, on `%_` lines).
fn placed_with(numbers: &[(&str, &str)]) -> Structure {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let entries: String = numbers.iter().map(|(key, v)| format!("%_{v} /Int ({key}) ,\r")).collect();
    let dictionary = format!("%_/ArtDictionary :\r{entries}%_;\r%_\r");
    read(&placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), &dictionary)).unwrap()
}

#[test]
fn a_placed_files_dictionary_names_the_page_and_box_it_shows() {
    use vectorcraft_doc::PageBox;
    const PAGE: &str = "AI11 PlacedPDFPageIndex";
    const CROP: &str = "AI11 PlacedPDFCropTo";
    for (crop, bounding, page_box) in [
        ("0", false, PageBox::Art),
        ("1", false, PageBox::Crop),
        ("2", false, PageBox::Trim),
        ("3", false, PageBox::Bleed),
        ("4", false, PageBox::Media),
        ("5", true, PageBox::Crop),
    ] {
        let r = placed_with(&[(PAGE, "3"), (CROP, crop)]);
        let a = art(&r.doc);
        let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
        assert_eq!(r.placed_pages.len(), 1, "{crop}");
        let p = r.placed_pages[0];
        assert_eq!((p.id, p.page, p.bounding, p.page_box, p.size), (a[0].id, 3, bounding, page_box, (4.0, 2.0)), "{crop}");
        assert_eq!(r.doc.images.keys().collect::<Vec<_>>(), [&im.key], "{crop}: the preview is stored once, under the image's key");
        assert!(r.doc.images[&im.key].is_proxy(), "{crop}");
    }
    // One entry alone: the first page, or the page's crop box.
    let r = placed_with(&[(CROP, "4")]);
    assert_eq!((r.placed_pages[0].page, r.placed_pages[0].page_box), (1, PageBox::Media));
    let r = placed_with(&[(PAGE, "2")]);
    assert_eq!((r.placed_pages[0].bounding, r.placed_pages[0].page_box), (false, PageBox::Crop));
}

#[test]
fn a_placed_files_dictionary_it_cant_read_leaves_a_linked_image() {
    const PAGE: &str = "AI11 PlacedPDFPageIndex";
    const CROP: &str = "AI11 PlacedPDFCropTo";
    let plain = placed_with(&[]);
    let plain_key = {
        let a = art(&plain.doc);
        let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
        im.key.clone()
    };
    assert!(plain.placed_pages.is_empty());
    for entries in [
        vec![(PAGE, "0"), (CROP, "4")],
        vec![(PAGE, "-1"), (CROP, "4")],
        vec![(PAGE, "1.5"), (CROP, "4")],
        vec![(PAGE, "1e30"), (CROP, "4")],
        vec![(PAGE, "2"), (CROP, "6")],
        vec![(PAGE, "2"), (CROP, "-1")],
        vec![(PAGE, "2"), (CROP, "0.5")],
        vec![(PAGE, "(2)"), (CROP, "4")],
        vec![(PAGE, "2"), (CROP, "(4)")],
        vec![(PAGE, "(2)")],
    ] {
        let r = placed_with(&entries);
        assert!(r.placed_pages.is_empty(), "{entries:?}");
        let a = art(&r.doc);
        let NodeKind::Image(im) = &a[0].kind else { panic!("{entries:?}: {:?}", a[0].kind) };
        assert_eq!((im.key.as_str(), im.link.as_ref().and_then(|l| l.page)), (plain_key.as_str(), None), "{entries:?}");
    }
}

/// The dictionaries of the art in a placed file's opacity mask, read inside its own dictionary,
/// don't speak for the placed file.
#[test]
fn a_placed_files_mask_art_leaves_its_page_to_the_placed_files_dictionary() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let dictionary = "%_/ArtDictionary :\r%_/Mask :\r%_X=\r%_0 g\r%_0 0 m\r%_5 0 l\r%_5 -5 l\r%_f\r\
                      %_/ArtDictionary :\r%_/XMLUID : (Inner) ; (AI10_ArtUID) ,\r%_;\r%_\r%_X+\r%_ (Art) ,\r%_; (AI9_Mask) ,\r\
                      %_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_4 /Int (AI11 PlacedPDFCropTo) ,\r%_;\r%_\r";
    let r = read(&placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), dictionary)).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:?}");
    assert!(a[0].extras().mask.is_some(), "the mask is read");
    assert_eq!(r.placed_pages.len(), 1);
    assert_eq!((r.placed_pages[0].id, r.placed_pages[0].page_box), (a[0].id, vectorcraft_doc::PageBox::Media));
}

/// A placed file in a placed file's opacity mask, read before the outer file's dictionary closes,
/// leaves the outer file's page to that dictionary.
#[test]
fn a_placed_file_in_a_placed_files_mask_leaves_the_outer_files_page() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let mut inner =
        b"%_%AI5_BeginPlace\r%_%%FileType: 1\r%_[5 0 0 -5 0 200] 10 20 14 22 4 4 0\r%_(/art/mask.pdf)`\r%_%AI26_BeginPlacedObjectPreview\r%_"
            .to_vec();
    inner.extend_from_slice(&preview);
    inner.extend_from_slice(
        b"\r%_%AI26_EndPlacedObjectPreview\r%_~\r%_%AI5_EndPlace\r%_/ArtDictionary :\r%_5 /Int (AI11 PlacedPDFPageIndex) ,\r%_;\r%_\r",
    );
    let mut dictionary = b"%_/ArtDictionary :\r%_/Mask :\r%_X=\r".to_vec();
    dictionary.extend_from_slice(&inner);
    dictionary
        .extend_from_slice(b"%_X+\r%_ (Art) ,\r%_; (AI9_Mask) ,\r%_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_4 /Int (AI11 PlacedPDFCropTo) ,\r%_;\r%_\r");
    let data = placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), "@@DICT@@");
    let at = data.windows(8).position(|x| x == b"@@DICT@@").unwrap();
    let data = [&data[..at], &dictionary[..], &data[at + 8..]].concat();
    let r = read(&data).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:?}");
    assert!(a[0].extras().mask.is_some(), "the mask is read");
    let outer: Vec<_> = r.placed_pages.iter().filter(|p| p.id == a[0].id).collect();
    assert_eq!(outer.len(), 1, "{:?}", r.placed_pages);
    assert_eq!((outer[0].page, outer[0].page_box), (2, vectorcraft_doc::PageBox::Media));
    assert!(r.placed_pages.iter().any(|p| p.id != a[0].id && p.page == 5), "the inner one names its own page: {:?}", r.placed_pages);
}

/// A placed file in a placed file's opacity mask that shows the same file with no page of its own
/// keeps the preview they share when the outer one's dictionary names a page.
#[test]
fn a_placed_file_in_a_placed_files_mask_keeps_the_preview_they_share() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let mut dictionary =
        b"%_/ArtDictionary :\r%_/Mask :\r%_X=\r%_%AI5_BeginPlace\r%_%%FileType: 1\r%_[5 0 0 -5 0 200] 10 20 14 22 4 4 0\r%_(/art/flyer.pdf)`\r\
          %_%AI26_BeginPlacedObjectPreview\r%_"
            .to_vec();
    dictionary.extend_from_slice(&preview);
    dictionary.extend_from_slice(
        b"\r%_%AI26_EndPlacedObjectPreview\r%_~\r%_%AI5_EndPlace\r%_X+\r%_ (Art) ,\r%_; (AI9_Mask) ,\r\
          %_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_;\r%_\r",
    );
    let data = placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), "@@DICT@@");
    let at = data.windows(8).position(|x| x == b"@@DICT@@").unwrap();
    let data = [&data[..at], &dictionary[..], &data[at + 8..]].concat();
    let r = read(&data).unwrap();
    let a = art(&r.doc);
    assert_eq!(r.placed_pages.len(), 1, "{:?}", r.placed_pages);
    let mask = a[0].extras().mask.clone().expect("the mask is read");
    let mut keys = vec![];
    mask.art.walk(&mut |n| {
        if let NodeKind::Image(im) = &n.kind {
            keys.push(im.key.clone());
        }
    });
    let [inner] = &keys[..] else { panic!("{keys:?}") };
    assert!(r.doc.images.contains_key(inner), "the mask's image keeps its preview");
    let NodeKind::Image(outer) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert!(r.doc.images.contains_key(&outer.key) && outer.key != *inner);
}

#[test]
fn a_placed_file_with_its_box_written_backward_stays_a_linked_image() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let dictionary = "%_/ArtDictionary :\r%_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_4 /Int (AI11 PlacedPDFCropTo) ,\r%_;\r%_\r";
    let data = placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), dictionary);
    let at = data.windows(11).position(|x| x == b"10 20 14 22").unwrap();
    let data = [&data[..at], b"14 22 10 20", &data[at + 11..]].concat();
    let r = read(&data).unwrap();
    assert!(r.placed_pages.is_empty());
    let a = art(&r.doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(im.link.as_ref().and_then(|l| l.page), None);
}

#[test]
fn only_the_dictionary_right_after_a_placed_file_names_its_page() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let dictionary = "%_/ArtDictionary :\r%_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_4 /Int (AI11 PlacedPDFCropTo) ,\r%_;\r%_\r";
    // After another object: that object's dictionary. Art with such a dictionary stays art.
    let after = format!("0 g\r0 0 m\r5 0 l\r5 5 l\rf\r{dictionary}");
    let r = read(&placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), &after)).unwrap();
    assert!(r.placed_pages.is_empty());
    assert_eq!(art(&r.doc).len(), 2);
    // A second dictionary after the first: the first one spoke for the placed file.
    let after = format!("%_/ArtDictionary :\r%_(Photo) /String (AIArtName) ,\r%_;\r%_\r{dictionary}");
    let r = read(&placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), &after)).unwrap();
    assert!(r.placed_pages.is_empty());
    // Without a placed file before it, nothing happens.
    let r = read(stream(&layer(&format!("0 g\r0 0 m\r5 0 l\r5 5 l\rf\r{dictionary}"))).as_bytes()).ok();
    assert!(r.is_none_or(|r| r.placed_pages.is_empty()));
}

/// Two pages of a file with the same preview don't share a key: reading one page's file in would
/// change the other's image.
#[test]
fn placed_pages_of_one_file_keep_images_of_their_own() {
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let one =
        placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), "%_/ArtDictionary :\r%_1 /Int (AI11 PlacedPDFPageIndex) ,\r%_;\r%_\r");
    let two =
        placed(true, "[5 0 0 -5 0 200]", "/art/flyer.pdf", Some(&preview), "%_/ArtDictionary :\r%_2 /Int (AI11 PlacedPDFPageIndex) ,\r%_;\r%_\r");
    let find = |d: &[u8], w: &[u8]| d.windows(w.len()).position(|x| x == w).unwrap();
    let body = two[find(&two, b"%AI5_BeginPlace")..find(&two, b"LB\n%AI5_EndLayer")].to_vec();
    let mut both = one.clone();
    let at = find(&both, b"LB\n%AI5_EndLayer");
    both.splice(at..at, body);
    let r = read(&both).unwrap();
    assert_eq!(r.placed_pages.len(), 2);
    let keys: Vec<String> = art(&r.doc)
        .iter()
        .map(|n| match &n.kind {
            NodeKind::Image(im) => im.key.clone(),
            k => panic!("{k:?}"),
        })
        .collect();
    assert_ne!(keys[0], keys[1]);
    assert!(keys.iter().all(|k| r.doc.images.contains_key(k)) && r.doc.images.len() == 2, "{keys:?}");
}

#[test]
fn placed_files_are_linked_images_showing_their_preview() {
    // A 2×1 preview (premultiplied ARGB, a padded row) of a file of 4×2 pixels.
    let preview = [&b"2 1 12 32 17 "[..], &[255, 255, 0, 0, 128, 0, 64, 0, 9, 9, 9, 9]].concat();
    let name = "%_/ArtDictionary :\r%_(Photo) /String (AIArtName) ,\r%_;\r";
    let r = read(&placed(true, "[5 0 0 -5 0 200]", "/pics/photo.png", Some(&preview), name)).unwrap();
    let a = art(&r.doc);
    let NodeKind::Image(im) = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!((im.link.as_ref().map(|l| l.path.as_str()), im.width, im.height), (Some("/pics/photo.png"), 2, 1));
    assert_eq!(a[0].name.as_deref(), Some("Photo"));
    assert!(r.doc.images[&im.key].is_proxy(), "the preview is what a missing link shows");
    let (img, xf) = pixels(&r.doc);
    assert_eq!((img.get_pixel(0, 0).0, img.get_pixel(1, 0).0), ([255, 0, 0, 255], [0, 127, 0, 128]));
    // Without a template box the pixels start at the box's corner, the first row the top one:
    // (10, 20) is at (50, 100) in art space, the page's top.
    assert_eq!((xf * Point::ZERO, xf * Point::new(2.0, 1.0)), (Point::new(50.0, 0.0), Point::new(70.0, 10.0)));
    assert!(r.warnings.iter().any(|w| w.contains("low-resolution preview")), "{:?}", r.warnings);
    // With a template box, the pixels start at the canvas point of art space's origin (8091.5,
    // 8241.5 here), wherever the box is.
    let mut data = placed(true, "[5 0 0 -5 -40407.5 41307.5]", "a.png", Some(&preview), "");
    let at = data.windows(15).position(|w| w == b"%AI5_BeginLayer").unwrap();
    data.splice(at..at, *b"%AI3_TemplateBox: 100 50 100 50\n");
    let xf = pixels(&read(&data).unwrap().doc).1;
    assert_eq!((xf * Point::ZERO, xf * Point::new(2.0, 1.0)), (Point::new(50.0, 0.0), Point::new(70.0, 10.0)));
    // Turned a quarter: the box's rows run along x.
    let r = read(&placed(true, "[0 5 5 0 0 0]", "a.png", Some(&preview), "")).unwrap();
    let xf = pixels(&r.doc).1;
    assert_eq!((xf * Point::ZERO, xf * Point::new(2.0, 1.0)), (Point::new(100.0, 50.0), Point::new(110.0, 30.0)));

    // A placed file with a box but no usable preview is marked for page-art replacement.
    let cut = &preview[..preview.len() - 6];
    let huge = b"70000 70000 280000 32 17 \0\0\0\0";
    let overflow = b"1 2 18446744073709551615 32 17 \0\0\0\0\0\0\0\0";
    for (matrix, path, preview) in [
        ("[5 0 0 -5 0 200]", "a.png", None),
        ("[5 0 0 -5 0 200]", "a.png", Some(cut)),
        ("[5 0 0 -5 0 200]", "a.png", Some(&b"2 1 2 8 17 \0\0"[..])),
        ("[5 0 0 -5 0 200]", "a.png", Some(&huge[..])),
        ("[5 0 0 -5 0 200]", "a.png", Some(&overflow[..])),
        ("[5 0 0 -5 0 200]", "a.png", Some(&b"2 1"[..])),
        ("[5 0 0 -5 0 200]", "", Some(&preview[..])),
        ("[0 0 0 0 0 0]", "a.png", Some(&preview[..])),
        ("[1 2 3]", "a.png", Some(&preview[..])),
    ] {
        let result = read(&placed(true, matrix, path, preview, ""));
        if matrix == "[0 0 0 0 0 0]" || matrix == "[1 2 3]" {
            let e = result.err().unwrap_or_else(|| panic!("expected missing bounds for {matrix} {path}"));
            assert!(e.contains("placed files"), "{matrix} {path} {preview:?}: {e}");
        } else {
            assert_unread(&result.unwrap(), "placed files");
        }
    }
    // On a layer that doesn't show, such a file is left out of it.
    let r = read(&placed(false, "[5 0 0 -5 0 200]", "a.png", None, "")).unwrap();
    assert!(r.hidden_unread.contains("placed files") && art(&r.doc).is_empty());
    // `~` ends nothing else.
    assert!(read(stream(&layer("~\n")).as_bytes()).unwrap_err().contains("`~`"));
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
    // A text object with an appearance: its drawn look (a group whose type is a story of the
    // undo-free text document, which counts its own), then the object on `%_` lines, which is
    // the type.
    let look = "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\nU\n0 0 Xd\n6 () XW\n";
    let own: String = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n7 /StoryIndex ,\n;\n".lines().map(|l| format!("%_{l}\n")).collect();
    let plain = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n";
    let r = read_ok(&layer(&format!("{look}{own}1 (Style) XW\n{plain}")));
    let stories: Vec<_> = art(&r.doc).iter().map(|n| slot_of(n.name.as_deref())).collect();
    assert_eq!(stories, [Some(Some(7)), Some(Some(0))], "the look's story 0 isn't the text document's");
    // What else the look draws stays, under the type.
    let look = look.replace("U\n", &format!("{RED}{}f\nU\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    let r = read_ok(&layer(&format!("{look}{own}1 (Style) XW\n")));
    let a = art(&r.doc);
    assert_eq!((a.len(), a[0].children().map(Vec::len), slot_of(a[1].name.as_deref())), (2, Some(1), Some(Some(7))));
    // The type of the look in a group of it is the object's too.
    let nested = look.replacen("u\n", "u\nu\n", 1).replacen("U\n", "U\nU\n", 1);
    let r = read_ok(&layer(&format!("{nested}{own}1 (Style) XW\n")));
    let mut slots = vec![];
    r.doc.walk(|n| {
        if let Some(story) = slot_of(n.name.as_deref()) {
            slots.push(story);
        }
    });
    assert_eq!(slots, [Some(7)]);
    // Whatever comes of the object's style, the type of its look is the object's.
    for marker in ["1 () XW", "2 (Style) XW"] {
        let r = read_ok(&layer(&format!("{look}{own}{marker}\n")));
        let mut looks = 0;
        r.doc.walk(|n| looks += usize::from(super::is_look_slot(n.name.as_deref())));
        assert_eq!(looks, 0, "{marker}");
    }
    // In the look of a group, the type is that of a copy the look draws: a slot without a story.
    let group = "u\n%_/AI11Text :\n%_0 /FreeUndo ,\n%_0 /FrameIndex ,\n%_7 /StoryIndex ,\n%_;\n%_U\n";
    let r = read_ok(&layer(&format!("{look}{group}1 (Style) XW\n")));
    let mut slots = vec![];
    r.doc.walk(|n| {
        if slot_of(n.name.as_deref()).is_some() {
            slots.push(n.name.clone());
        }
    });
    assert!(slots.len() == 1 && slots.iter().all(|n| super::is_look_slot(n.as_deref())), "{slots:?}");
}

/// A file in the legacy format (version 7) whose layer holds `art`, crop marks 0 0 200 100.
fn type_stream7(art: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 3.0\n%AI3_Cropmarks: 0 0 200 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n",
        layer(art)
    )
}

/// The text objects of a legacy-format file whose layer holds `art`.
fn legacy_texts(art: &str) -> Vec<vectorcraft_doc::TextObject> {
    let doc = match read(type_stream7(art).as_bytes()) {
        Ok(r) => r.doc,
        Err(e) => panic!("{e}"),
    };
    art_texts(&doc)
}

/// The characters of each text object of a legacy-format file whose setup is `setup` and whose
/// layer holds point type with the string `(chars)` in each font of `fonts`.
fn legacy_chars(setup: &str, fonts: &[&str], chars: &str) -> Vec<String> {
    let art: String = fonts.iter().map(|f| point_type("", chars).replace("/_Helvetica 12 Tf", &format!("/{f} 12 Tf"))).collect();
    let data = type_stream7(&art).replace("%%BeginSetup\n%%EndSetup", &format!("%%BeginSetup\n{setup}%%EndSetup"));
    let doc = match read(data.as_bytes()) {
        Ok(r) => r.doc,
        Err(e) => panic!("{e}"),
    };
    art_texts(&doc).iter().map(|t| t.runs.iter().map(|r| r.text.as_str()).collect()).collect()
}

/// `TZ` (§3.3.1) gives a re-encoded font encoding pairs of its own: its strings decode by them
/// first, then by `TE`'s encoding when `useDefault` is 1, and other fonts' strings keep `TE`'s. `TZ` is read in an
/// `%AI3_BeginEncoding` section and in an `%%BeginEncoding` one, with a Multiple Master weight
/// vector after it, and without the direction operand.
#[test]
fn a_re_encoded_fonts_own_encoding_pairs_decode_only_its_strings() {
    let setup = "[39/quoteright 96/grave 128/Adieresis\nTE\n\
                 %AI3_BeginEncoding: _F F\n[ 39/quotesingle 200/eacute\n/_F/F 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n\
                 %%BeginEncoding: _M M\n[ 39/quotesingle\n/_M/M 0 0 1 [0.5 0.25] TZ\n%%EndEncoding\n\
                 %AI3_BeginEncoding: _N N\n[ 39/quotesingle\n/_N/N 0 1 TZ\n%AI3_EndEncoding AdobeType\n";
    let got = legacy_chars(setup, &["_F", "_M", "_N", "_G"], "\\047\\140\\200\\310");
    assert_eq!(got, ["'`Äé", "'`ÄÈ", "'`ÄÈ", "’`ÄÈ"]);
    // The re-encoded fonts are the fonts they were made from.
    let data = type_stream7(&point_type("", "a").replace("/_Helvetica", "/_M"))
        .replace("%%BeginSetup\n%%EndSetup", "%%BeginSetup\n%%BeginEncoding: _M Courier\n[/_M/Courier 0 0 1 TZ\n%%EndEncoding\n%%EndSetup");
    let texts = art_texts(&read(data.as_bytes()).unwrap().doc);
    assert_eq!(texts[0].runs[0].style.font_family, "Courier");
}

/// The reader does not use the pairs of a `TZ` whose font names aren't the last two operands, or
/// are followed by other values. The first `TZ` for a font is used, also when it has no pairs.
#[test]
fn a_tz_of_another_shape_changes_no_encoding() {
    let setup = "[39/quoteright\nTE\n\
                 %AI3_BeginEncoding: _F F\n[ 39/quotesingle\n/_F/F (x) 1 TZ\n%AI3_EndEncoding AdobeType\n\
                 %AI3_BeginEncoding: _G G\n[ 39/quotesingle\n/_G 0 /G 1 TZ\n%AI3_EndEncoding AdobeType\n\
                 %AI3_BeginEncoding: _H H\n[ 39/grave\n/_H/H 0 0 1 TZ\n[ 39/quotesingle\n/_H/H 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n\
                 %AI3_BeginEncoding: _K K\n[/_K/K 0 0 1 TZ\n[ 39/quotesingle\n/_K/K 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n";
    assert_eq!(legacy_chars(setup, &["_F", "_G", "_H", "_K"], "\\047"), ["’", "’", "`", "’"]);
}

/// A `TZ` whose `useDefault` is 0 leaves `TE`'s encoding out for its font (§3.3.1): the codes its
/// pairs leave out decode as Windows-1252.
#[test]
fn a_tz_with_use_default_0_leaves_te_out() {
    let setup = "[39/quoteright 128/Adieresis\nTE\n\
                 %AI3_BeginEncoding: _F F\n[ 96/grave\n/_F/F 0 0 0 TZ\n%AI3_EndEncoding TrueType\n\
                 %AI3_BeginEncoding: _S S\n[/_S/S 0 0 0 TZ\n%AI3_EndEncoding AdobeType\n\
                 %AI3_BeginEncoding: _M M\n[ 96/grave\n/_M/M 0 0 [0.5] TZ\n%AI3_EndEncoding AdobeType\n";
    assert_eq!(legacy_chars(setup, &["_F", "_S", "_M", "_G"], "\\047\\140\\200"), ["'`€", "'`€", "'`€", "’`Ä"]);
}

/// Strings in a composite font `base-CMap` decode by that predefined CMap (codes to CIDs, then the
/// character collection's CIDs to Unicode), and the font is `base`; `TE`'s encoding does not apply
/// to them. Codes the CMap leaves out are dropped. A font whose name ends in no predefined CMap
/// decodes byte by byte.
#[test]
fn legacy_type_in_a_cjk_font_decodes_by_its_cmap() {
    let typed = |font: &str, chars: &str| point_type("", chars).replace("/_Helvetica", &format!("/{font}"));
    let cases: &[(&str, &str, &str)] = &[
        // 日 is 0x93FA and 本 0x967B in Shift-JIS; `\r` ends a paragraph.
        ("83pv-RKSJ-H", "\\223\\372\\226{A\\r\\223\\372", "日本A\n日"),
        // A tab stays a tab, also after a UTF-16 string's last whole code.
        ("90ms-RKSJ-H", "\\223\\372\\tA", "日\tA"),
        ("H", "F|\\tF|", "日\t日"),
        ("UniJIS-UTF16-H", "e\\345\\t", "日\t"),
        // One-byte codes in the Mac Shift-JIS CMap: 0x5C yen, 0x7E tilde, 0x80 backslash, 0xFD
        // copyright, and the underscore and grave accent.
        ("83pv-RKSJ-H", "\\\\~\\200\\375_`", "¥~\\©_`"),
        ("90ms-RKSJ-H", "\\\\~_`", "¥~_`"),
        ("90ms-RKSJ-V", "\\223\\372", "日"),
        ("90pv-RKSJ-V", "\\223\\372", "日"),
        // JIS row and cell: 日 is 0x4652.
        ("H", "F|", "日"),
        ("V", "F|", "日"),
        ("EUC-H", "\\306\\374", "日"),
        ("GBK-EUC-H", "\\326\\320", "中"),
        ("GBpc-EUC-H", "\\326\\320", "中"),
        ("B5pc-H", "\\244\\244", "中"),
        ("KSCms-UHC-H", "\\307\\321", "한"),
        ("KSCpc-EUC-H", "\\307\\321", "한"),
        ("UniJIS-UCS2-H", "e\\345", "日"),
        ("UniGB-UTF8-H", "\\344\\270\\255", "中"),
        // An encoded U+FFFD is a character; the invalid byte 0xFF is dropped.
        ("UniGB-UTF8-H", "\\357\\277\\275\\377\\344\\270\\255", "\u{fffd}中"),
        ("UniJIS-UTF32-H", "\\000\\000e\\345", "日"),
        ("UniAKR-UTF16-H", "\\254\\000", "가"),
        // CMaps VectorCraft doesn't carry: read with the nearest carried one, row-and-cell codes in
        // their EUC form (中 is 0x5650 in GB 2312, 한 0x4751 in KS X 1001), or byte by byte.
        ("78-RKSJ-H", "\\223\\372", "日"),
        ("RKSJ-V", "\\223\\372", "日"),
        ("GB-H", "VP", "中"),
        ("KSC-V", "GQ", "한"),
        ("GBT-H", "AB", "AB"),
        // Codes are as long as the code space range they fall in, and unmapped ones are dropped:
        // 0x8540 as two bytes, and 0xFD and EUC's 0x8F (in no range) as one.
        ("90ms-RKSJ-H", "\\205\\100\\223\\372A\\375B", "日AB"),
        ("EUC-H", "\\217A\\306\\374", "A日"),
    ];
    let art: String = cases.iter().map(|(cmap, chars, _)| typed(&format!("_Test-W3-{cmap}"), chars)).collect();
    let art = [art, typed("_Test-W3-Identity-H", "\\223"), typed("_Test-W3-UniFoo-UTF8-H", "\\223"), typed("_Helvetica", "\\223")].concat();
    let data = type_stream7(&art).replace("%%BeginSetup\n%%EndSetup", "%%BeginSetup\n[147/Adieresis\nTE\n%%EndSetup");
    let texts = art_texts(&read(data.as_bytes()).unwrap().doc);
    let chars: Vec<String> = texts.iter().map(|t| t.plain_text()).collect();
    let mut want: Vec<&str> = cases.iter().map(|(_, _, want)| *want).collect();
    want.extend(["Ä", "Ä", "Ä"]);
    assert_eq!(chars, want);
    let test = crate::import::family_style("Test-W3");
    for (t, (cmap, _, _)) in texts.iter().zip(cases) {
        assert_eq!((&t.runs[0].style.font_family, &t.runs[0].style.font_style), (&test.0, &test.1), "{cmap}");
    }
    assert_eq!(texts[0].paragraph_count(), 2);
    assert_ne!(texts[cases.len()].runs[0].style.font_style, test.1, "an Identity CMap is not read");
    assert_ne!(texts[cases.len() + 1].runs[0].style.font_style, test.1, "`UniFoo` is not a character collection");
}

/// A font whose `TZ` gives encoding pairs is not a composite font, even when its name ends in a
/// CMap name: its strings decode byte by byte, and the font keeps its whole name. A `TZ` without
/// encoding pairs only names a composite font.
#[test]
fn a_re_encoded_font_ending_in_a_cmap_name_is_not_composite() {
    let setup = "%AI3_BeginEncoding: _F-Bold-H F-Bold-H\n[ 39/quotesingle\n/_F-Bold-H/F-Bold-H 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n";
    assert_eq!(legacy_chars(setup, &["_F-Bold-H"], "F|'"), ["F|'"]);
    let setup = "%AI3_BeginEncoding: _G-Bold-H G-Bold-H\n[ 65/A\n/_G-Bold-H/G-Bold-H 0 0 1 TZ\n%AI3_EndEncoding AdobeType\n";
    assert_eq!(legacy_chars(setup, &["_G-Bold-H"], "F|"), ["F|"]);
    let setup = "%AI3_BeginEncoding: _R-W3-83pv-RKSJ-H R-W3-83pv-RKSJ-H\n[/_R-W3-83pv-RKSJ-H/R-W3-83pv-RKSJ-H 0 1 1 TZ\n%AI3_EndEncoding AdobeType\n";
    assert_eq!(legacy_chars(setup, &["_R-W3-83pv-RKSJ-H"], "\\223\\372"), ["日"]);
}

/// In a font with a Unicode CMap, a one-byte `(\r)` string, or a 0x0D byte after a string's last
/// whole UTF-16 code, still ends a paragraph, as in other fonts, and an overflow `(\r) TX` still
/// ends the story.
#[test]
fn legacy_type_in_a_utf16_font_keeps_its_paragraph_breaks() {
    let art = point_type("", "e\\345")
        .replace("/_Helvetica", "/_Test-W3-UniJIS-UTF16-H")
        .replace("TO\n", "(\\r) Tx\n(e\\345\\r) Tx\n(e\\345) Tx\n(\\r) TX\nTO\n");
    let texts = legacy_texts(&art);
    assert_eq!(texts[0].plain_text(), "日\n日\n日");
    assert_eq!(texts[0].paragraph_count(), 3);
}

fn art_texts(doc: &Document) -> Vec<vectorcraft_doc::TextObject> {
    art(doc).iter().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some((**t).clone()) } else { None }).collect()
}

/// Point type at (10, 50): `attrs`, then the characters `text`.
fn point_type(attrs: &str, text: &str) -> String {
    format!("0 To\n1 0 0 1 10 50 0 Tp\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n{attrs}\n({text}) Tx\nTO\n")
}

/// `TC` and `TW` (§15.4.7) set the paragraph's letter and word spacing, `Tq` its hanging
/// punctuation and `Th` (§22) its hyphenation; each paragraph keeps the values in force at its
/// first character, and they last into the next text object.
#[test]
fn legacy_type_keeps_its_spacing_hanging_punctuation_and_hyphenation() {
    let two_paragraphs =
        point_type("10 15 20 TC\n70 110 140 TW\n1 Tq\n1 1 2 3 4 Th", "One\\r").replace("TO\n", "0 0 5 TC\n0 Tq\n0 0 4 5 6 Th\n(Two) Tx\nTO\n");
    let texts = legacy_texts(&format!("{two_paragraphs}{}", point_type("", "Three")));
    assert_eq!(texts.len(), 2);
    let one = texts[0].para_at(0);
    assert_eq!(one.justification.letter_spacing, [10.0, 15.0, 20.0]);
    assert_eq!(one.justification.word_spacing, [70.0, 110.0, 140.0]);
    assert!(one.roman_hanging && one.hyphenate);
    let h = &one.hyphenation;
    assert_eq!((h.after_first, h.before_last, h.hyphen_limit), (2, 3, 4));
    assert_eq!(h.words_longer_than, vectorcraft_doc::text::HyphenationOptions::default().words_longer_than, "no operand sets it");
    let two = texts[0].para_at(1);
    assert_eq!(two.justification.letter_spacing, [0.0, 0.0, 5.0]);
    assert_eq!(two.justification.word_spacing, [70.0, 110.0, 140.0], "word spacing stays");
    assert!(!two.roman_hanging && !two.hyphenate);
    // Without the limit flag, any number of hyphens in a row.
    assert_eq!((two.hyphenation.after_first, two.hyphenation.before_last, two.hyphenation.hyphen_limit), (4, 5, 0));
    // The next text object keeps them.
    assert_eq!(texts[1].para_at(0), two);
}

/// `Tg` (§15.3.4) turns binding punctuation on (1 and 3) or off (0 and 2); VectorCraft opens it on
/// as Burasagari's Force setting. Each paragraph of point type and area type keeps the value in
/// force at its first character, and it lasts into the next text object.
#[test]
fn legacy_type_keeps_its_binding_punctuation() {
    use vectorcraft_doc::Burasagari;
    let burasagari = |attrs: &str| legacy_texts(&point_type(attrs, "Hi"))[0].para_at(0).burasagari;
    assert_eq!(burasagari(""), Burasagari::None);
    for on in ["1", "3"] {
        assert_eq!(burasagari(&format!("0 Tg\n{on} Tg")), Burasagari::Forced, "{on} Tg");
    }
    for off in ["0", "2"] {
        assert_eq!(burasagari(&format!("1 Tg\n{off} Tg")), Burasagari::None, "{off} Tg");
    }
    let two_paragraphs = point_type("1 Tg", "One\\r").replace("TO\n", "0 Tg\n(Two) Tx\nTO\n");
    let texts = legacy_texts(&format!("{two_paragraphs}{}{}", point_type("3 Tg", "Three"), point_type("", "Four")));
    let each: Vec<Burasagari> = vec![texts[0].para_at(0).burasagari, texts[0].para_at(1).burasagari, texts[1].para_at(0).burasagari];
    assert_eq!(each, [Burasagari::Forced, Burasagari::None, Burasagari::Forced]);
    assert_eq!(texts[2].para_at(0).burasagari, Burasagari::Forced, "the next text object keeps it");
    let frame = "10 10 m\n110 10 L\n110 110 L\n10 110 L\nN";
    let area = format!("1 To\n1 0 0 1 10 110 0 Tp\n{frame}\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n1 Tg\n(Area) Tx\nTO\n");
    let texts = legacy_texts(&area);
    assert!(matches!(texts[0].kind, vectorcraft_doc::TextKind::Area { .. }));
    assert_eq!(texts[0].para_at(0).burasagari, Burasagari::Forced, "area type");
}

/// `TV` (§15.3.4) sets the writing direction of point type, area type, type on a path and linked
/// areas, and lasts into the next text object until it changes.
#[test]
fn legacy_type_keeps_its_writing_direction() {
    let frame = "10 10 m\n110 10 L\n110 110 L\n10 110 L\nN";
    let area = format!("1 To\n1 0 0 1 10 110 0 Tp\n{frame}\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Area) Tx\nTO\n");
    let on_path = "2 To\n1 0 0 1 0 0 0 Tp\n10 10 m\n110 10 L\nN\nTP\n(Path) Tx\nTO\n";
    let linked = format!("1 To\n1 0 0 1 10 110 0 Tp\n{frame}\nTP\n1 0 0 1 200 110 0 Tp\n{frame}\nTP\n(Linked) Tx\nTO\n");
    let art = format!("{}{area}{on_path}{linked}{}", point_type("1 TV", "Point"), point_type("0 TV", "Flat"));
    let texts = legacy_texts(&art);
    let vertical: Vec<bool> = texts.iter().map(|t| t.vertical).collect();
    assert_eq!(vertical, [true, true, true, true, true, false]);
    assert!(matches!(texts[1].kind, vectorcraft_doc::TextKind::Area { .. }));
    assert!(matches!(texts[2].kind, vectorcraft_doc::TextKind::OnPath { .. }));
    // Vertical point type runs down the canvas from `Tp`'s origin (10, 50), which opens at document
    // (10, 50) on the 100 pt tall artboard, on the column's center line.
    let long = &legacy_texts(&point_type("1 TV", "Point type"))[0];
    let bounds = long.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), long).bounds);
    assert!(bounds.height() > bounds.width(), "{bounds:?}");
    assert!(bounds.x0 < 10.0 && bounds.x1 > 10.0, "{bounds:?}");
    assert!((bounds.y0 - 50.0).abs() < 1.0 && bounds.y1 > 60.0, "{bounds:?}");
    // A text object keeps the direction in force at its first character, with a warning when a later
    // character has another; the mode lasts into the next text object.
    let changed = point_type("1 TV", "First").replace("TO\n", "0 TV\n(Second) Tx\nTO\n");
    let r = legacy_read(&format!("{changed}{}", point_type("", "Next")));
    let changed = art_texts(&r.doc);
    assert!(changed[0].vertical);
    assert_eq!(changed[0].plain_text(), "FirstSecond");
    assert!(!changed[1].vertical);
    assert_eq!(r.warnings, [DIRECTION_CHANGE_LEFT_OUT]);
    // A direction that changes and changes back also warns.
    let back = point_type("", "a").replace("TO\n", "1 TV\n(b) Tx\n0 TV\n(c) Tx\nTO\n");
    assert_eq!(legacy_read(&back).warnings, [DIRECTION_CHANGE_LEFT_OUT]);
    // A `TV` that no character follows does not warn, and an object with no characters opens no
    // type, while its `TV` lasts into the next object.
    let trailing = point_type("", "a").replace("TO\n", "1 TV\nTO\n");
    assert!(legacy_read(&trailing).warnings.is_empty());
    let empty = legacy_texts(&format!("0 To\n1 0 0 1 10 50 0 Tp\nTP\n1 TV\nTO\n{}", point_type("", "Next")));
    assert_eq!((empty.len(), empty[0].vertical), (1, true));
    // `TV` before `To` counts, and `TV` after the last character only reaches the next object.
    let before = legacy_texts(&format!("1 TV\n{}", point_type("", "Hi")));
    assert!(before[0].vertical);
    let after = point_type("", "Hi").replace("TO\n", "1 TV\nTO\n");
    let after = legacy_texts(&format!("{after}{}", point_type("", "Next")));
    assert_eq!((after[0].vertical, after[1].vertical), (false, true));
    // An overflow break (`(\r) TX`) keeps the direction in force where it was read: at the start of
    // the story it sets the object's direction, and later it warns when that differs.
    let leading = legacy_read(&point_type("1 TV\n(\\r) TX\n0 TV", "Body"));
    let leading = (art_texts(&leading.doc), leading.warnings);
    assert_eq!((leading.0[0].vertical, leading.0[0].plain_text().as_str()), (true, "\nBody"));
    assert_eq!(leading.1, [DIRECTION_CHANGE_LEFT_OUT]);
    let inside = legacy_read(&point_type("", "a").replace("TO\n", "1 TV\n(\\r) TX\n0 TV\n(b) Tx\nTO\n"));
    let inside = (art_texts(&inside.doc), inside.warnings);
    assert_eq!((inside.0[0].vertical, inside.0[0].plain_text().as_str()), (false, "a\nb"));
    assert_eq!(inside.1, [DIRECTION_CHANGE_LEFT_OUT]);
}

/// An overflow break (`(\r) TX`) that type follows ends a paragraph as `(\r) Tx` does: an empty
/// paragraph it ends keeps the attributes in force where the break was read.
#[test]
fn an_overflow_break_keeps_the_paragraph_attributes_it_was_read_with() {
    use vectorcraft_doc::Justify::{Center, Right};
    for op in ["Tx", "TX"] {
        let leading = &legacy_texts(&point_type(&format!("1 Ta\n(\\r) {op}\n2 Ta"), "Body"))[0];
        assert_eq!((leading.para_at(0).justify, leading.para_at(1).justify), (Center, Right), "{op}");
        let empty = point_type("1 Ta", "a\\r").replace("TO\n", &format!("(\\r) {op}\n2 Ta\n(b) Tx\nTO\n"));
        let empty = &legacy_texts(&empty)[0];
        assert_eq!(empty.plain_text(), "a\n\nb", "{op}");
        assert_eq!((empty.para_at(1).justify, empty.para_at(2).justify), (Center, Right), "{op}");
    }
}

/// The glyph origins of a legacy-format text object.
fn legacy_origins(t: &vectorcraft_doc::TextObject) -> Vec<(f64, f64)> {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.iter().map(|g| (g.origin.x, g.origin.y)).collect()
}

/// `Ty` (§15.3.4) marks the characters of a Kumi block in vertical and in horizontal type, `TY`
/// marks them in horizontal type, and `Tv` keeps Roman characters upright in vertical type; `Tv`
/// may come inside the text path. All three last into the next text object.
#[test]
fn legacy_type_keeps_kumi_blocks_and_upright_roman_characters() {
    let runs = |art: &str| -> Vec<(String, bool, bool)> {
        let texts = legacy_texts(art);
        texts[0].runs.iter().map(|r| (r.text.clone(), r.style.tate_chu_yoko, r.style.standard_vertical_roman)).collect()
    };
    let kumi = |begin: &str, end: &str| point_type(&format!("(A) Tx\n{begin}\n(12) Tx\n{end}"), "B");
    let marked = vec![("A".to_string(), false, false), ("12".to_string(), true, false), ("B".to_string(), false, false)];
    assert_eq!(runs(&format!("1 TV\n{}", kumi("1 Ty", "0 Ty"))), marked);
    assert_eq!(runs(&kumi("1 Ty", "0 Ty")), marked);
    assert_eq!(runs(&kumi("1 TY", "0 TY")), marked);
    assert!(!legacy_texts(&kumi("1 TY", "0 TY"))[0].vertical);
    // `TY` records a block that the set text doesn't show: vertical type opens without it, and the
    // block starts again in the next horizontal object.
    let unmarked = vec![("A12B".to_string(), false, false)];
    assert_eq!(runs(&format!("1 TV\n{}", kumi("1 TY", "0 TY"))), unmarked);
    let recorded = legacy_texts(&format!("1 TV\n1 TY\n{}0 TV\n{}", point_type("", "A"), point_type("", "B")));
    assert_eq!((recorded[0].runs[0].style.tate_chu_yoko, recorded[1].runs[0].style.tate_chu_yoko), (false, true));
    let upright = point_type("(a) Tx\n1 Tv", "b");
    assert_eq!(runs(&format!("1 TV\n{upright}")), [("a".to_string(), false, false), ("b".to_string(), false, true)]);
    let in_path = point_type("", "c").replace("0 Tp\nTP", "0 Tp\n1 Tv\nTP");
    assert_eq!(runs(&in_path), [("c".to_string(), false, true)]);
    assert_eq!(runs(&point_type("1 Tv\n(a) Tx\n0 Tv", "b")), [("a".to_string(), false, true), ("b".to_string(), false, false)]);
    // Each one lasts into the next text object.
    for (op, kept) in [("Ty", (true, false)), ("TY", (true, false)), ("Tv", (false, true))] {
        let texts = legacy_texts(&format!("{}{}", point_type(&format!("1 {op}"), "a"), point_type("", "b")));
        let st = &texts[1].runs[0].style;
        assert_eq!((st.tate_chu_yoko, st.standard_vertical_roman), kept, "{op}");
    }
    // Horizontal type keeps the mark and doesn't move the characters.
    let plain = &legacy_texts(&point_type("", "A12B"))[0];
    let in_horizontal = &legacy_texts(&kumi("1 Ty", "0 Ty"))[0];
    assert_eq!(legacy_origins(in_horizontal), legacy_origins(plain));
    // Two blocks with nothing between them open as one block.
    let vertical = |attrs: &str, text: &str| legacy_texts(&format!("1 TV\n{}", point_type(attrs, text))).remove(0);
    let two = vertical("1 Ty\n(AB) Tx\n0 Ty\n1 Ty", "CD");
    let one = vertical("1 Ty", "ABCD");
    assert_eq!(legacy_origins(&two), legacy_origins(&one));
    // Two or three digits that no block marks are set across the column by the layout's own rule.
    let digits = vertical("", "12");
    assert!(!digits.runs[0].style.tate_chu_yoko);
    assert_eq!(legacy_origins(&digits), legacy_origins(&vertical("1 Ty", "12")));
    assert_ne!(legacy_origins(&vertical("", "AB")), legacy_origins(&vertical("1 Ty", "AB")));
}

/// A legacy attribute whose operands are missing, are not numbers, or are flags other than 0 and
/// 1 leaves the attribute as it was, and operands are read by their place.
#[test]
fn legacy_type_attributes_with_bad_operands_stay_as_they_were() {
    let set = "10 15 20 TC\n70 110 140 TW\n1 Tq\n1 1 2 3 4 Th\n2 Ta\n0 TA\n5 6 7 Ti";
    let bad = "/x 15 20 TC\n110 140 TW\n2 Tq\n1 2 2 3 4 Th\n1 1 (2) 3 4 Th\n/x Ta\n2 TA\n5 /x 7 Ti";
    let texts = legacy_texts(&point_type(&format!("{set}\n{bad}"), "Hi"));
    let p = texts[0].para_at(0);
    assert_eq!(p.justification.letter_spacing, [10.0, 15.0, 20.0]);
    assert_eq!(p.justification.word_spacing, [70.0, 110.0, 140.0]);
    assert!(p.roman_hanging && p.hyphenate);
    assert_eq!((p.hyphenation.after_first, p.hyphenation.before_last, p.hyphenation.hyphen_limit), (2, 3, 4));
    assert_eq!(p.justify, vectorcraft_doc::Justify::Right);
    assert_eq!((p.first_line_indent, p.left_indent, p.right_indent), (5.0, 6.0, 7.0));
    assert_eq!(texts[0].runs[0].style.kerning, Some(0.0), "kerning stays off");
    let texts = legacy_texts(&point_type("1 Tg\n4 Tg\n-1 Tg\n0.5 Tg\n/x Tg\n(0) Tg\nTg", "Hi"));
    assert_eq!(texts[0].para_at(0).burasagari, vectorcraft_doc::Burasagari::Forced, "binding punctuation stays on");
    let texts = legacy_texts(&point_type("0 Tg\n4 Tg\n-1 Tg\n2.5 Tg", "Hi"));
    assert_eq!(texts[0].para_at(0).burasagari, vectorcraft_doc::Burasagari::None, "binding punctuation stays off");
    let texts = legacy_texts(&point_type("1 TV\n1 Tv\n1 Ty\n2 TV\n/x TV\nTV\n-1 Tv\n(x) Tv\n0.5 Ty\n2 TY", "Hi"));
    let st = &texts[0].runs[0].style;
    assert!(texts[0].vertical && st.standard_vertical_roman && st.tate_chu_yoko);
    let texts = legacy_texts(&point_type("2 TV\n-1 TV\n2 Tv\n-1 Tv\n0.5 Ty\n-1 Ty\n2 TY\n-1 TY\n1 TY\n2 TY\n0.5 TY", "Hi"));
    let st = &texts[0].runs[0].style;
    assert!(!texts[0].vertical && !st.standard_vertical_roman);
    assert!(st.tate_chu_yoko, "`TY` stays on");
    let texts = legacy_texts(&point_type("2 TV\n-1 TV\n2 Tv\n-1 Tv\n0.5 Ty\n-1 Ty\n2 TY\n-1 TY", "Hi"));
    assert!(!texts[0].runs[0].style.tate_chu_yoko);
    // A scale or leading pair with a bad operand leaves both as they were; one operand is the
    // horizontal scale (or the leading) of the format before version 7.
    let texts = legacy_texts(&point_type("50 80 Tz\n14 3 Tl\n/x 120 Tz\n/x 20 Tl", "Hi"));
    let st = &texts[0].runs[0].style;
    assert_eq!((st.h_scale, st.v_scale, st.leading), (50.0, 80.0, Some(14.0)));
    assert_eq!(texts[0].para_at(0).space_before, 3.0);
    let st = &legacy_texts(&point_type("50 80 Tz\n120 Tz", "Hi"))[0].runs[0].style;
    assert_eq!((st.h_scale, st.v_scale), (120.0, 100.0));
    // A start point that isn't a number leaves the matrix.
    let good = &legacy_texts(&point_type("", "Hi"))[0];
    let bad = &legacy_texts(&point_type("", "Hi").replace("10 50 0 Tp", "10 50 /x Tp"))[0];
    assert_eq!(bad.xf, good.xf);
}

/// `(text) justified TS` (special characters, §22.6) adds its characters to the story like `Tx`
/// and `Tj`, whichever its flag. Without a string, or outside a text object, it adds nothing.
#[test]
fn legacy_special_characters_join_the_story() {
    let texts = legacy_texts(&point_type("", "One").replace("TO\n", "(\\t) 0 TS\n(Two\\r) 1 TS\n(Three) Tx\nTO\n"));
    let [t] = texts.as_slice() else { panic!("{texts:?}") };
    assert_eq!(t.plain_text(), "One\tTwo\nThree");
    let texts = legacy_texts(&format!("(x) 0 TS\n{}", point_type("0 TS", "Hi")));
    let [t] = texts.as_slice() else { panic!("{texts:?}") };
    assert_eq!(t.plain_text(), "Hi");
}

/// A tab definition (`Xb`, `Xe`, `XB`, §22.6) sets the paragraph's tab stops, sorted by position
/// with one stop at each position; each paragraph keeps the stops in force at its first character,
/// and they last into the next text object. A definition with no stops leaves the default ones.
#[test]
fn legacy_tab_definitions_set_the_paragraph_tab_stops() {
    use vectorcraft_doc::TabAlign;
    let stops = |p: &vectorcraft_doc::ParaStyle| p.tabs.iter().map(|t| (t.position, t.align)).collect::<Vec<_>>();
    let ruler = "36 5 Xb\n0 0 3 144 Xe\n0 0 1 72 Xe\n0 0 2 200.5 Xe\n0 0 4 250 Xe\n0 0 2 72 Xe\nXB";
    let two_paragraphs = point_type(ruler, "A\\tB\\r").replace("TO\n", "36 0 Xb\nXB\n(C\\tD) Tx\nTO\n");
    let texts = legacy_texts(&format!("{two_paragraphs}{}", point_type("", "E")));
    assert_eq!(texts.len(), 2);
    let want = vec![(72.0, TabAlign::Left), (144.0, TabAlign::Right), (200.5, TabAlign::Center), (250.0, TabAlign::Decimal)];
    let one = texts[0].para_at(0);
    assert_eq!(stops(one), want);
    assert!(one.tabs.iter().all(|t| t.leader.is_empty() && t.align_on == '.'));
    assert!(texts[0].para_at(1).tabs.is_empty(), "an empty definition leaves the default stops");
    assert!(texts[1].para_at(0).tabs.is_empty(), "the next text object keeps the empty definition");
    let texts = legacy_texts(&format!("{}{}", point_type(ruler, "A"), point_type("", "B")));
    assert_eq!(stops(texts[1].para_at(0)), want, "the next text object keeps the stops");
}

/// A tab description whose type is not 1 to 4 or whose position is missing, not a number,
/// negative or past 100,000 pt is skipped; so is one outside a tab definition, and an `XB` without
/// an `Xb` leaves the stops as they were.
#[test]
fn legacy_tab_descriptions_out_of_range_are_skipped() {
    let bad = "0 0 5 90 Xe\n0 0 0 90 Xe\n0 0 1.5 90 Xe\n0 0 1 -1 Xe\n0 0 1 100001 Xe\n0 0 1 /x Xe\n1 Xe";
    let texts = legacy_texts(&point_type(&format!("0 0 1 10 Xe\n36 2 Xb\n{bad}\n0 0 3 100000 Xe\n0 0 1 0 Xe\nXB\nXB\n0 0 1 50 Xe"), "Hi"));
    let tabs: Vec<f64> = texts[0].para_at(0).tabs.iter().map(|t| t.position).collect();
    assert_eq!(tabs, [0.0, 100_000.0]);
}

/// A tab definition whose descriptions were all skipped leaves the default stops; a definition
/// keeps its first 1024 stops; and a definition left open (an `Xb` with no `XB`) ends at the next
/// text object's `To`, so that object's descriptions outside a definition are skipped.
#[test]
fn legacy_tab_definitions_skipped_capped_and_left_open() {
    let positions = |t: &vectorcraft_doc::TextObject| t.para_at(0).tabs.iter().map(|t| t.position).collect::<Vec<f64>>();
    let all_skipped = format!("{}{}", point_type("36 1 Xb\n0 0 1 72 Xe\nXB", "A"), point_type("36 1 Xb\n0 0 9 90 Xe\nXB", "B"));
    let texts = legacy_texts(&all_skipped);
    assert_eq!(positions(&texts[0]), [72.0]);
    assert!(texts[1].para_at(0).tabs.is_empty(), "a definition with every stop skipped leaves the default stops");
    let many: String = (0..1100).map(|i| format!("0 0 1 {i} Xe\n")).collect();
    let texts = legacy_texts(&point_type(&format!("36 1100 Xb\n{many}XB"), "C"));
    assert_eq!(positions(&texts[0]).len(), 1024);
    assert_eq!(positions(&texts[0]).last(), Some(&1023.0));
    let open = format!("{}{}", point_type("36 1 Xb\n0 0 1 72 Xe", "D"), point_type("0 0 1 90 Xe\nXB", "E"));
    let texts = legacy_texts(&open);
    assert!(texts[0].para_at(0).tabs.is_empty(), "an unended definition sets no stops");
    assert!(texts[1].para_at(0).tabs.is_empty(), "the next object's `To` ends the open definition");
}

/// Point type whose 1024 tab stops are followed by 2048 paragraph breaks, each after `attrs(i)`.
fn paragraphs_after_many_tabs(attrs: impl Fn(usize) -> String) -> String {
    let many: String = (0..1024).map(|i| format!("0 0 1 {i} Xe\n")).collect();
    let breaks: String = (0..2048).map(|i| format!("{}(x\\r) Tx\n", attrs(i))).collect();
    point_type(&format!("36 1024 Xb\n{many}XB"), "A").replace("TO\n", &format!("{breaks}TO\n"))
}

/// Paragraphs that start with the same attributes all keep the same tab stops, however many
/// there are, also when an attribute is set again to the value it has.
#[test]
fn legacy_tab_stops_last_through_many_paragraphs() {
    for again in ["", "0 Ta\n"] {
        let r = legacy_read(&paragraphs_after_many_tabs(|_| again.to_string()));
        let texts = art_texts(&r.doc);
        let [t] = texts.as_slice() else { panic!("{texts:?}") };
        assert_eq!(t.paragraph_count(), 2049);
        assert!((0..2049).all(|i| t.para_at(i).tabs.len() == 1024), "{again}");
        assert!(r.warnings.is_empty(), "{again}: {:?}", r.warnings);
    }
}

/// Each paragraph whose attributes differ from the paragraph before it holds its own copy of its
/// tab stops, and a text object's paragraphs hold no more than 1,048,576 stops: the paragraphs
/// past them take the default stops and keep their other attributes, with a warning.
#[test]
fn legacy_tab_stops_copied_into_paragraphs_are_capped() {
    let r = legacy_read(&paragraphs_after_many_tabs(|i| format!("{} Ta\n", i % 2)));
    let texts = art_texts(&r.doc);
    let [t] = texts.as_slice() else { panic!("{texts:?}") };
    let paras = t.paragraph_styles();
    assert_eq!(paras.len(), 2049);
    assert_eq!(paras.iter().map(|p| p.tabs.len()).sum::<usize>(), 1 << 20);
    assert!(paras[..1024].iter().all(|p| p.tabs.len() == 1024));
    assert!(paras[1024..].iter().all(|p| p.tabs.is_empty()));
    assert_eq!((paras[2046].justify, paras[2047].justify), (vectorcraft_doc::Justify::Left, vectorcraft_doc::Justify::Center));
    assert_eq!(r.warnings, ["tab stops past VectorCraft's limits were left out"]);
}

/// `Te` (end render, §22.6) reads in a text object with no operands and with four, inside a tab
/// definition too, and keeps nothing; so does `TM` (pop text matrix, §22.6).
#[test]
fn legacy_end_render_and_text_matrix_pop_read() {
    for te in ["Te", "0 0 1 36 Te", "36 0 Xb\n0 0 1 36 Te\nXB", "1 0 0 1 10 50 Tm\nTM"] {
        let texts = legacy_texts(&point_type(te, "Hi"));
        let [t] = texts.as_slice() else { panic!("{te}: {texts:?}") };
        assert_eq!(t.plain_text(), "Hi", "{te}");
    }
}

/// Spacing is kept to the ranges the Justification options take, a Minimum, Desired and Maximum
/// out of order go back to their defaults, and hyphenation's letter counts and limit to the ranges
/// `text.setFormat` takes.
#[test]
fn legacy_type_spacing_and_hyphenation_are_kept_in_range() {
    let texts = legacy_texts(&point_type("-500 0 900 TC\n0 2000 5000 TW\n1 1 0 40 99 Th", "Hi"));
    let p = texts[0].para_at(0);
    assert_eq!(p.justification.letter_spacing, [-100.0, 0.0, 500.0]);
    assert_eq!(p.justification.word_spacing, [0.0, 1000.0, 1000.0]);
    assert_eq!((p.hyphenation.after_first, p.hyphenation.before_last, p.hyphenation.hyphen_limit), (1, 15, 25));
    let texts = legacy_texts(&point_type("5 0 0 TC\n200 100 80 TW", "Hi"));
    let j = &texts[0].para_at(0).justification;
    let d = vectorcraft_doc::text::JustificationOptions::default();
    assert_eq!((j.letter_spacing, j.word_spacing), (d.letter_spacing, d.word_spacing));
}

/// A file's legacy type is read up to its allowance of characters, a single long run included.
#[test]
fn one_long_run_of_legacy_type_stops_at_the_allowance() {
    // The reader's allowance, `MAX_CHARS`.
    const ALLOWANCE: usize = 1 << 22;
    let long = "a".repeat(ALLOWANCE + 10);
    let texts = legacy_texts(&point_type("", &long));
    assert_eq!(texts[0].plain_text().chars().count(), ALLOWANCE);
}

/// The fill and stroke colors of a text object's first run (None: no paint).
fn run_paint(n: &Node) -> (Option<Color>, Option<Color>) {
    let NodeKind::Text(t) = &n.kind else { panic!("type: {:?}", n.kind) };
    (t.runs[0].style.fill.color(), t.runs[0].style.stroke.color())
}

/// Legacy type set in rendering modes 4 to 7 is a mask (§5.7): the text object is the clipping
/// path of its clipping group (`q` … `Q`) and clips the objects with it. Modes 4, 5 and 6 also
/// fill, stroke, or fill and stroke it; 7 paints nothing.
#[test]
fn legacy_type_in_a_mask_rendering_mode_clips_its_clipping_group() {
    let (black, red) = (Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)), Some(Color::rgb(1.0, 0.0, 0.0)));
    let square = format!("1 0 0 Xa\n{}f\n", rect_path(0.0, 0.0, 100.0, 100.0));
    let cases = [("4", (black, None)), ("5", (None, red)), ("6", (black, red)), ("7", (None, None))];
    for (mode, paint) in cases {
        let text = point_type(&format!("{mode} Tr\n1 0 0 XA\n1 w"), "Hi");
        let doc = legacy_read(&format!("q\n{text}{square}Q\n")).doc;
        let a = art(&doc);
        assert_eq!(a.len(), 1, "{mode} Tr");
        let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{mode} Tr: {:?}", a[0].kind) };
        assert_eq!(children.len(), 2, "{mode} Tr");
        assert_eq!(run_paint(&children[0]), paint, "{mode} Tr");
        assert!(matches!(children[1].kind, NodeKind::Path { clipping: false, .. }), "{mode} Tr");
        // The art takes its shape from the type: it is bounded by the characters, not the square.
        let b = a[0].geometric_bounds().unwrap();
        assert!(b.width() < 50.0 && b.height() < 50.0, "{mode} Tr: {b:?}");
    }
    // Directly in a layer, the layer clips. As with a `W` path in this reader, the art written before the
    // type is clipped too.
    let doc = legacy_read(&format!("{square}{}", point_type("7 Tr", "Hi"))).doc;
    let NodeKind::Layer { children, clip: true, .. } = &doc.layers[0].kind else { panic!("{:?}", doc.layers[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Text(_)) && matches!(children[1].kind, NodeKind::Path { .. }));
    // With a clipping path after it, the art shows where both are: the type clips a clipping group nested in it, and
    // keeps its paint.
    let doc = legacy_read(&format!("q\n{}{}h\nW\nn\n{square}Q\n", point_type("6 Tr\n1 0 0 XA\n1 w", "Hi"), rect_path(0.0, 0.0, 30.0, 30.0))).doc;
    let a = art(&doc);
    let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(run_paint(&children[0]), (black, red));
    let NodeKind::Group { children: inner, clip: true } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert!(matches!(inner[0].kind, NodeKind::Path { clipping: true, .. }) && matches!(inner[1].kind, NodeKind::Path { clipping: false, .. }));
}

/// A filled clipping path (`W` then `f`) beside type used as a mask keeps its fill, which paints at its own path's level,
/// as beside another clipping path (the reader's rule for `W` paths). Written after the type, it is painted within the
/// type's mask, since a later mask narrows the clip set by the earlier ones (§5.6.2); written before it, the type doesn't
/// clip it.
#[test]
fn a_filled_clipping_path_beside_legacy_type_used_as_a_mask() {
    let square = format!("1 0 0 Xa\n{}f\n", rect_path(0.0, 0.0, 100.0, 100.0));
    let filled = format!("0 0 1 Xa\n{}h\nW\nf\n", rect_path(0.0, 0.0, 30.0, 30.0));
    let text = point_type("7 Tr", "Hi");
    let is_clip = |n: &Node| matches!(n.kind, NodeKind::Path { clipping: true, .. }) && fill(n).color() == Some(Color::rgb(0.0, 0.0, 1.0));
    let is_art = |n: &Node| matches!(n.kind, NodeKind::Path { clipping: false, .. }) && fill(n).color() == Some(Color::rgb(1.0, 0.0, 0.0));
    // The type first: the filled path clips a clipping group inside the type's.
    let doc = legacy_read(&format!("q\n{text}{filled}{square}Q\n")).doc;
    let a = art(&doc);
    let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Text(_)) && children.len() == 2, "{children:?}");
    let NodeKind::Group { children: inner, clip: true } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert!(is_clip(&inner[0]) && is_art(&inner[1]) && inner.len() == 2, "{inner:?}");
    // The path first: it clips the outer clipping group, so only the path clips its fill; the type clips the art.
    let doc = legacy_read(&format!("q\n{filled}{text}{square}Q\n")).doc;
    let a = art(&doc);
    let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert!(is_clip(&children[0]) && children.len() == 2, "{children:?}");
    let NodeKind::Group { children: inner, clip: true } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert!(matches!(inner[0].kind, NodeKind::Text(_)) && is_art(&inner[1]) && inner.len() == 2, "{inner:?}");
}

/// `Tr` sets the rendering mode of all the type that follows (§15.3.2), in later text objects too: a second text
/// object with no `Tr` of its own is also a mask.
#[test]
fn a_legacy_mask_rendering_mode_lasts_into_the_next_text_object() {
    let square = format!("1 0 0 Xa\n{}f\n", rect_path(0.0, 0.0, 100.0, 100.0));
    let second = point_type("", "Yo").replace("0 Tr\n", "");
    let doc = legacy_read(&format!("q\n{}{second}{square}Q\n", point_type("7 Tr", "Hi"))).doc;
    let a = art(&doc);
    let NodeKind::Group { children, clip: true } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert!(matches!(children[0].kind, NodeKind::Text(_)), "{children:?}");
    let NodeKind::Group { children: inner, clip: true } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert!(matches!(inner[0].kind, NodeKind::Text(_)) && matches!(inner[1].kind, NodeKind::Path { clipping: false, .. }), "{inner:?}");
}

/// Type in a mask rendering mode that can't be one clipping path doesn't clip, with a warning: area type with linked
/// areas (threaded frames), the type of wraparound text, and type in a compound path.
#[test]
fn legacy_type_used_as_a_mask_that_cannot_clip_is_left_unclipped() {
    const WARNING: &str = "type used as a mask in linked areas, wraparound text or a compound path doesn't clip";
    let square = format!("1 0 0 Xa\n{}f\n", rect_path(0.0, 0.0, 100.0, 100.0));
    let linked = format!(
        "1 To\n1 0 0 1 0 40 0 Tp\n{}n\nTP\n7 Tr\n/_Helvetica 12 Tf\n(Linked ) Tx\n1 0 0 1 60 40 0 Tp\n{}n\nTP\n(areas) Tx\nTO\n",
        rect_path(0.0, 0.0, 50.0, 40.0),
        rect_path(60.0, 0.0, 110.0, 40.0)
    );
    let r = legacy_read(&format!("q\n{linked}{square}Q\n"));
    let a = art(&r.doc);
    let NodeKind::Group { children, clip: false } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(children.len(), 3);
    assert_eq!(r.doc.text_threads.len(), 1);
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    // Also when the reader's limits leave one frame of a long story in linked areas.
    let long = linked.replace("(areas) Tx", &format!("({}) Tx", "a".repeat(3_000_000)));
    let r = legacy_read(&format!("q\n{long}{square}Q\n"));
    let a = art(&r.doc);
    let NodeKind::Group { children, clip: false } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(children.len(), 2);
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    // Wraparound text: the type is what flows around the other objects, which it doesn't clip.
    let area = format!("1 To\n1 0 0 1 0 0 0 Tp\n{}n\nTP\n7 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Around) Tx\nTO\n", rect_path(0.0, 0.0, 100.0, 80.0));
    let r = legacy_read(&format!("q\n*w\n{area}{square}*W\nQ\n"));
    let a = art(&r.doc);
    let NodeKind::Group { children, clip: false } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let NodeKind::Group { children: wrap, clip: false } = &children[0].kind else { panic!("{:?}", children[0].kind) };
    assert!(matches!(wrap[0].kind, NodeKind::Text(_)) && wrap.len() == 2, "{wrap:?}");
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    // A compound path: its type is no clipping subpath, and the compound path doesn't clip the group.
    let r = legacy_read(&format!("q\n*u\n{}f\n{}*U\n{square}Q\n", rect_path(0.0, 0.0, 30.0, 30.0), point_type("7 Tr", "Hi")));
    assert_eq!(unclipped_texts(&art(&r.doc)), 1);
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    // The same with the type in a group inside the compound path or the wraparound text.
    let grouped = format!("u\n{}U\n", point_type("7 Tr", "Hi"));
    let r = legacy_read(&format!("q\n*u\n{}f\n{grouped}*U\n{square}Q\n", rect_path(0.0, 0.0, 30.0, 30.0)));
    assert_eq!(unclipped_texts(&art(&r.doc)), 1);
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    let grouped = format!("u\n{area}U\n");
    let r = legacy_read(&format!("q\n*w\n{grouped}{square}*W\nQ\n"));
    assert_eq!(unclipped_texts(&art(&r.doc)), 1);
    assert!(r.warnings.iter().any(|w| w == WARNING), "{:?}", r.warnings);
    // An opacity mask's art clips on its own, also on an object in wraparound text.
    let mask_art: String =
        format!("{}1 g\n{}f\n", point_type("7 Tr", "Hi"), rect_path(0.0, 0.0, 100.0, 100.0)).lines().map(|l| format!("%_{l}\n")).collect();
    let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{mask_art}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_; (AI9_Mask) ,\n%_;\n");
    let r = legacy_read(&format!("q\n*w\n{area}{square}{mask}*W\nQ\n"));
    assert!(r.warnings.iter().filter(|w| *w == WARNING).count() == 1, "{:?}", r.warnings);
    let a = art(&r.doc);
    let NodeKind::Group { children, .. } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    let NodeKind::Group { children: wrap, .. } = &children[0].kind else { panic!("{:?}", children[0].kind) };
    let Some(m) = wrap.iter().find_map(|n| n.extras().mask.clone()) else { panic!("{wrap:?}") };
    let NodeKind::Group { children: in_mask, clip: true } = &m.art.kind else { panic!("{:?}", m.art.kind) };
    assert!(matches!(in_mask[0].kind, NodeKind::Text(_)) && in_mask.len() == 2, "{in_mask:?}");
}

/// The text objects in `nodes` and the groups and compound paths under them, checking that nothing among them clips.
#[track_caller]
fn unclipped_texts(nodes: &[Arc<Node>]) -> usize {
    let mut texts = 0;
    for n in nodes {
        assert!(!matches!(n.kind, NodeKind::Group { clip: true, .. } | NodeKind::Path { clipping: true, .. }), "{:?}", n.kind);
        texts += usize::from(matches!(n.kind, NodeKind::Text(_)));
        texts += n.children().map_or(0, |c| unclipped_texts(c));
    }
    texts
}

/// Legacy type in rendering modes 0 to 3 is no mask (3: invisible, painting nothing). A mode that is not a whole
/// number from 0 to 9 leaves the mode as it was.
#[test]
fn legacy_type_in_other_rendering_modes_does_not_clip() {
    let (black, red) = (Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)), Some(Color::rgb(1.0, 0.0, 0.0)));
    let square = format!("{}f\n", rect_path(0.0, 0.0, 100.0, 100.0));
    let cases = [("0 Tr", (black, None)), ("3 Tr", (None, None)), ("1 Tr\n4.5 Tr\n10 Tr\n-4 Tr\n/x Tr", (None, red))];
    for (modes, paint) in cases {
        let text = point_type(&format!("{modes}\n1 0 0 XA\n1 w"), "Hi");
        let doc = legacy_read(&format!("q\n{text}{square}Q\n")).doc;
        let a = art(&doc);
        assert_eq!(a.len(), 1, "{modes}");
        let NodeKind::Group { children, clip: false } = &a[0].kind else { panic!("{modes}: {:?}", a[0].kind) };
        assert_eq!(run_paint(&children[0]), paint, "{modes}");
    }
}

/// A legacy-format file whose layer holds `art`, read.
fn legacy_read(art: &str) -> Structure {
    match read(type_stream7(art).as_bytes()) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

/// The text objects of the first layer, with their ids, top to bottom: a threaded text object's
/// frames in file order.
fn art_frames(doc: &Document) -> Vec<(vectorcraft_doc::NodeId, vectorcraft_doc::TextObject)> {
    art(doc).iter().rev().filter_map(|n| if let NodeKind::Text(t) = &n.kind { Some((n.id, (**t).clone())) } else { None }).collect()
}

/// An area's frame in the document.
fn frame_in_doc(t: &vectorcraft_doc::TextObject) -> Rect {
    let vectorcraft_doc::TextKind::Area { frame } = &t.kind else { panic!("area type: {:?}", t.kind) };
    kurbo::Shape::bounding_box(&(t.xf * frame.to_bezpath()))
}

/// Do two matrices match?
fn same_matrix(a: Affine, b: Affine) -> bool {
    a.as_coeffs().iter().zip(b.as_coeffs()).all(|(p, q)| (p - q).abs() < 1e-9)
}

/// Area type with linked areas (§15.5.3): each area's `Tp` … `TP` with its own matrix and frame,
/// the characters after each, and overflow text. It opens as one frame for each area, threaded in
/// file order, with the story in the first.
#[test]
fn legacy_area_type_with_linked_areas_is_threaded_frames() {
    let text = format!(
        "1 To\n1 0 0 1 0 40 0 Tp\n{}n\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Linked ) Tx\n\
         0 1 -1 0 150 20 0 Tp\n{}n\nTP\n(areas\\r) Tx\n(and more) TX\n(\\r) TX\nTO\n",
        rect_path(0.0, 0.0, 50.0, 40.0),
        rect_path(110.0, 20.0, 150.0, 80.0)
    );
    let r = legacy_read(&text);
    let frames = art_frames(&r.doc);
    let [(a, first), (b, second)] = frames.as_slice() else { panic!("two frames: {frames:?} {:?}", r.warnings) };
    assert_eq!(r.doc.text_threads, [vec![*a, *b]], "threaded in file order");
    let in_doc = |x0, y0, x1, y1| r.to_doc.transform_rect_bbox(Rect::new(x0, y0, x1, y1));
    assert!(near(frame_in_doc(first), in_doc(0.0, 0.0, 50.0, 40.0)), "{:?}", frame_in_doc(first));
    assert!(near(frame_in_doc(second), in_doc(110.0, 20.0, 150.0, 80.0)), "{:?}", frame_in_doc(second));
    // Each frame keeps its own area's matrix: the second's type runs up the page.
    let placed = |m: [f64; 6]| r.to_doc * Affine::new(m) * Affine::FLIP_Y;
    assert!(same_matrix(first.xf, placed([1.0, 0.0, 0.0, 1.0, 0.0, 40.0])), "{:?}", first.xf);
    assert!(same_matrix(second.xf, placed([0.0, 1.0, -1.0, 0.0, 150.0, 20.0])), "{:?}", second.xf);
    // The story: the characters after every area and the overflow text, in the first frame.
    assert_eq!(first.plain_text(), "Linked areas\nand more");
    assert_eq!(first.paragraph_styles().len(), 2);
    assert_eq!(second.plain_text(), "");
}

/// A linked area without a path is left out; a single area with a path is area type in no thread.
#[test]
fn legacy_area_type_leaves_out_a_linked_area_without_a_path() {
    let area = |m: &str, path: Option<Rect>, chars: &str| {
        let path = path.map(|p| format!("{}n\n", rect_path(p.x0, p.y0, p.x1, p.y1))).unwrap_or_default();
        format!("{m} Tp\n{path}TP\n{chars}")
    };
    let style = "0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n";
    let three = format!(
        "1 To\n{}{}{}TO\n",
        area("1 0 0 1 0 40 0", Some(Rect::new(0.0, 0.0, 50.0, 40.0)), &format!("{style}(One ) Tx\n")),
        area("1 0 0 1 60 40 0", None, "(two ) Tx\n"),
        area("1 0 0 1 110 40 0", Some(Rect::new(110.0, 0.0, 150.0, 40.0)), "(three) Tx\n"),
    );
    let r = legacy_read(&three);
    let frames = art_frames(&r.doc);
    let [(a, first), (b, second)] = frames.as_slice() else { panic!("two frames: {frames:?} {:?}", r.warnings) };
    assert_eq!(r.doc.text_threads, [vec![*a, *b]]);
    assert_eq!((first.plain_text().as_str(), second.plain_text().as_str()), ("One two three", ""));
    let one = format!(
        "1 To\n{}{}TO\n",
        area("1 0 0 1 60 40 0", None, &format!("{style}(x) Tx\n")),
        area("1 0 0 1 0 40 0", Some(Rect::new(0.0, 0.0, 50.0, 40.0)), "(y) Tx\n")
    );
    let r = legacy_read(&one);
    let frames = art_frames(&r.doc);
    let [(_, only)] = frames.as_slice() else { panic!("one frame: {frames:?}") };
    assert!(r.doc.text_threads.is_empty());
    assert_eq!(only.plain_text(), "xy");
    // The frame's own matrix, not the pathless area's.
    assert!(same_matrix(only.xf, r.to_doc * Affine::translate((0.0, 40.0)) * Affine::FLIP_Y), "{:?}", only.xf);
}

/// One text object reads at most 4,096 linked areas: the paths of the rest are left out, not read as
/// art, and their characters stay in the story.
#[test]
fn legacy_area_type_reads_up_to_its_limit_of_linked_areas() {
    const LIMIT: usize = 4096;
    let area = format!("1 0 0 1 0 0 0 Tp\n{}n\nTP\n", rect_path(0.0, 0.0, 50.0, 40.0));
    let text = format!("1 To\n{}0 Tr\n(a) Tx\n{area}(b) Tx\nTO\n", area.repeat(LIMIT));
    let r = legacy_read(&text);
    let frames = art_frames(&r.doc);
    assert_eq!(frames.len(), LIMIT);
    assert_eq!(art(&r.doc).len(), LIMIT, "no art paths");
    assert_eq!(r.doc.text_threads.iter().map(Vec::len).collect::<Vec<_>>(), [LIMIT]);
    assert_eq!(frames[0].1.plain_text(), "ab");
    assert_eq!(r.warnings, [LINKED_AREAS_LEFT_OUT]);
}

/// The frames of linked areas after the first copy the story's last paragraph attributes, and
/// those copies hold no more than 1,048,576 tab stops: the frames past them take the default
/// stops, with a warning.
#[test]
fn legacy_linked_area_frames_cap_their_copied_tab_stops() {
    let many: String = (0..1024).map(|i| format!("0 0 1 {i} Xe\n")).collect();
    let area = format!("1 0 0 1 0 0 0 Tp\n{}n\nTP\n", rect_path(0.0, 0.0, 50.0, 40.0));
    let text = format!("1 To\n{}0 Tr\n36 1024 Xb\n{many}XB\n(a) Tx\nTO\n", area.repeat(4096));
    let r = legacy_read(&text);
    let frames = art_frames(&r.doc);
    assert_eq!(frames.len(), 4096);
    let stops: Vec<usize> = frames.iter().map(|(_, f)| f.para_at(0).tabs.len()).collect();
    assert_eq!(stops.iter().filter(|&&n| n == 1024).count(), 1025);
    assert_eq!(stops.iter().filter(|&&n| n == 0).count(), 3071);
    assert_eq!(r.warnings, ["tab stops past VectorCraft's limits were left out"]);
}

/// The warning when linked areas are left out.
const LINKED_AREAS_LEFT_OUT: &str = "linked type areas past VectorCraft's limits were left out";
const DIRECTION_CHANGE_LEFT_OUT: &str = "type that changes writing direction inside a text object opens in the direction of its first character";

/// The frames of linked areas times the characters of their story are kept to the reader's
/// allowance of characters: a story of more than half of it gets one frame, and of a third, two.
#[test]
fn legacy_area_type_keeps_its_frames_times_characters_to_the_allowance() {
    const ALLOWANCE: usize = 1 << 22;
    let area = |x: f64| format!("1 0 0 1 {x} 0 0 Tp\n{}n\nTP\n", rect_path(x, 0.0, x + 50.0, 40.0));
    for (chars, kept) in [(ALLOWANCE / 2 + 1, 1), (ALLOWANCE / 3, 3), (ALLOWANCE / 3 + 1, 2)] {
        let text = format!("1 To\n{}0 Tr\n({}) Tx\n{}{}TO\n", area(0.0), "a".repeat(chars), area(60.0), area(120.0));
        let r = legacy_read(&text);
        let frames = art_frames(&r.doc);
        assert_eq!(frames.len(), kept, "{chars}");
        assert_eq!(r.doc.text_threads.len(), usize::from(kept > 1), "{chars}");
        assert_eq!(frames[0].1.plain_text().len(), chars);
        assert_eq!(r.warnings.iter().any(|w| w == LINKED_AREAS_LEFT_OUT), kept < 3, "{chars}");
    }
}

/// Point type and type on a path have one text path (§15.2): one with more is read at its first.
#[test]
fn legacy_point_and_path_type_with_several_text_paths_use_the_first() {
    let point = "0 To\n1 0 0 1 10 50 0 Tp\nTP\n0 Tr\n(a) Tx\n1 0 0 1 90 20 0 Tp\nTP\n(b) Tx\nTO\n";
    let r = legacy_read(point);
    let frames = art_frames(&r.doc);
    let [(_, only)] = frames.as_slice() else { panic!("one object: {frames:?}") };
    assert!(matches!(only.kind, vectorcraft_doc::TextKind::Point) && r.doc.text_threads.is_empty());
    assert_eq!(only.plain_text(), "ab");
    assert!(same_matrix(only.xf, r.to_doc * Affine::translate((10.0, 50.0)) * Affine::FLIP_Y), "the first text path's matrix: {:?}", only.xf);
    let on_path = "2 To\n1 0 0 1 0 0 0.5 Tp\n0 0 m\n100 0 L\nN\nTP\n0 Tr\n(a) Tx\n1 0 0 1 0 0 0 Tp\n0 50 m\n100 50 L\nN\nTP\n(b) Tx\nTO\n";
    let r = legacy_read(on_path);
    let frames = art_frames(&r.doc);
    let [(_, only)] = frames.as_slice() else { panic!("one object: {frames:?}") };
    let vectorcraft_doc::TextKind::OnPath { path, start, .. } = &only.kind else { panic!("{:?}", only.kind) };
    assert!((start - 0.5).abs() < 1e-9, "the first text path's start: {start}");
    let along = kurbo::Shape::bounding_box(&(only.xf * path.to_bezpath()));
    assert!((along.y0 - r.to_doc.transform_rect_bbox(Rect::new(0.0, 0.0, 100.0, 0.0)).y0).abs() < 1e-9, "{along:?}");
    assert!(same_matrix(only.xf, r.to_doc * Affine::FLIP_Y), "{:?}", only.xf);
    // Type on a path whose first text path has no path: the first one with a path, its matrix and
    // start included.
    let later = "2 To\n1 0 0 1 30 30 0.25 Tp\nTP\n0 Tr\n(a) Tx\n1 0 0 1 5 0 0.5 Tp\n0 50 m\n100 50 L\nN\nTP\n(b) Tx\nTO\n";
    let r = legacy_read(later);
    let frames = art_frames(&r.doc);
    let [(_, only)] = frames.as_slice() else { panic!("one object: {frames:?}") };
    let vectorcraft_doc::TextKind::OnPath { start, .. } = &only.kind else { panic!("{:?}", only.kind) };
    assert!((start - 0.5).abs() < 1e-9, "{start}");
    assert!(same_matrix(only.xf, r.to_doc * Affine::translate((5.0, 0.0)) * Affine::FLIP_Y), "{:?}", only.xf);
    // Area type with no frame at all: point type at its first text path.
    let frameless = "1 To\n1 0 0 1 10 50 0 Tp\nTP\n0 Tr\n(a) Tx\n1 0 0 1 90 20 0 Tp\nTP\n(b) Tx\nTO\n";
    let r = legacy_read(frameless);
    let frames = art_frames(&r.doc);
    let [(_, only)] = frames.as_slice() else { panic!("one object: {frames:?}") };
    assert!(matches!(only.kind, vectorcraft_doc::TextKind::Point), "{:?}", only.kind);
    assert!(same_matrix(only.xf, r.to_doc * Affine::translate((10.0, 50.0)) * Affine::FLIP_Y), "{:?}", only.xf);
}

/// The dictionary that follows a threaded text object (version 9 and later): its opacity mask and
/// text wrap go on every frame, and its name on the first frame, which holds the story.
#[test]
fn the_dictionary_after_linked_area_type_applies_to_its_frames() {
    let dict = "%_/ArtDictionary :\n%_(Story) /String (AIArtName) ,\n%_12.5 /Real (AI11TextWrapOffset) ,\n\
                %_0 /Bool (AI11TextWrapInvert) ,\n%_/Mask :\n%_1 /Bool (Linked) ,\n%_0 /Bool (Disabled) ,\n%_X=\n\
                %_0 0 0 1 k\n%_0 0 m\n%_5 0 L\n%_5 5 L\n%_0 5 L\n%_0 0 L\n%_f\n%_X+\n%_ (Art) ,\n%_1 /Bool (Clipping) ,\n\
                %_0 /Bool (Inverted) ,\n%_; (AI9_Mask) ,\n%_;\n";
    let area = |x: f64| format!("1 0 0 1 {x} 40 0 Tp\n{}n\nTP\n", rect_path(x, 0.0, x + 50.0, 40.0));
    let text = format!("1 To\n{}0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(a) Tx\n{}{}(b) Tx\nTO\n{dict}", area(0.0), area(60.0), area(120.0));
    let r = read_ok(&layer(&text));
    let ids: Vec<_> = art_frames(&r.doc).into_iter().map(|(id, _)| id).collect();
    assert_eq!(r.doc.text_threads, std::slice::from_ref(&ids), "{:?}", r.warnings);
    let nodes: Vec<&Node> = ids.iter().filter_map(|id| r.doc.node(*id)).collect();
    assert_eq!(nodes.len(), 3);
    for n in &nodes {
        assert!(n.extras().mask.is_some(), "every frame is masked: {:?}", n.id);
        assert_eq!(wrap_of(n), Some(vectorcraft_doc::TextWrap { offset: 12.5, invert: false }));
    }
    let names: Vec<_> = nodes.iter().map(|n| n.name.as_deref()).collect();
    assert_eq!(names, [Some("Story"), None, None]);
}

#[test]
fn what_it_doesnt_read_is_an_error_where_it_shows() {
    let e = read(stream(&layer("1 2 frobnicate\n")).as_bytes()).unwrap_err();
    assert!(e.contains("`frobnicate`"), "{e}");
    let r = read(stream(&layer(&format!("(Jive) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(0.0, 0.0, 1.0, 1.0)))).as_bytes()).unwrap();
    assert_unread(&r, "pattern fills");
    let e = read(stream(&layer("/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n")).as_bytes()).unwrap_err();
    assert!(e.contains("symbols"), "{e}");
    // A mask with no owner must still be reported (#2004).
    let mask = "%_/ArtDictionary :\n%_/XMLUID : (M) ; (AI10_ArtUID) ,\n%_/Mask :\n%_1 /Bool (Linked) ,\n%_0 /Bool (Disabled) ,\n%_X=\n\
                %_0 0 0 1 k\n%_0 0 m\n%_5 0 L\n%_5 5 L\n%_0 5 L\n%_0 0 L\n%_f\n%_X+\n%_ (Art) ,\n%_1 /Bool (Clipping) ,\n%_0 /Bool (Inverted) ,\n%_; (AI9_Mask) ,\n%_;\n";
    let e = read(stream(&layer(mask)).as_bytes()).unwrap_err();
    assert!(e.contains("opacity masks"), "{e}");
    // A mask that is disabled changes nothing: the art reads.
    read_ok(&layer(&mask.replace("%_0 /Bool (Disabled)", "%_1 /Bool (Disabled)")));
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
fn a_group_still_open_at_the_end_is_an_error() {
    let square = format!("{}f\n", rect_path(0.0, 0.0, 10.0, 10.0));
    for open in ["u", "q", "*u", "%_u"] {
        let e = read(stream(&format!("{}{open}\n{square}", layer(""))).as_bytes()).unwrap_err();
        assert_eq!(e, "it ends inside a group or compound path", "{open}");
    }
    // A group open inside a layer that doesn't end is the layer's error.
    let unended = format!("%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(L) Ln\nu\n{square}");
    assert_eq!(read(stream(&unended).as_bytes()).unwrap_err(), "it ends inside a layer");
    // Patterns read with a group of their own still read.
    assert!(read_ok(&format!("{DOTS}{}", layer(""))).doc.pattern("Dots").is_some());
}

#[test]
fn unresolved_pattern_reset_before_group_path_keeps_live_art() {
    for (operator, draw, reset) in [("p", "f", "0 0 0 1 k"), ("P", "S", "0 0 0 1 K")] {
        let body = format!("u\n(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n{reset}\n{}{draw}\nU\n", rect_path(0.0, 0.0, 4.0, 4.0),);
        let result = read_ok(&layer(&body));
        assert!(result.unread.is_empty(), "{operator}: {:?}", result.unread);
        let children = art(&result.doc);
        assert_eq!(children.len(), 1, "{operator}: group count");
        let group_art = children[0].children().expect("group children");
        assert_eq!(group_art.len(), 1, "{operator}: live path count");
        assert!(matches!(group_art[0].kind, NodeKind::Path { .. }));
    }
}

#[test]
fn unresolved_pattern_in_dictionary_does_not_mark_following_path_unread() {
    for (operator, draw) in [("p", "f"), ("P", "S")] {
        let body = format!("/Unused :\n(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n;\n{}{draw}\n", rect_path(0.0, 0.0, 4.0, 4.0),);
        let result = read_ok(&layer(&body));
        assert!(result.unread.is_empty(), "{operator}: {:?}", result.unread);
        let children = art(&result.doc);
        assert_eq!(children.len(), 1, "{operator}: live path count");
        assert!(matches!(children[0].kind, NodeKind::Path { .. }));
    }
}

#[test]
fn color_in_dictionary_does_not_clear_outer_unresolved_pattern() {
    for (operator, draw, color) in [("p", "f", "0 0 0 1 k"), ("P", "S", "0 0 0 1 K")] {
        let body = format!("(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n/Unused :\n{color}\n;\n{}{draw}\n", rect_path(0.0, 0.0, 4.0, 4.0),);
        let result = read_ok(&layer(&body));
        assert_unread(&result, "pattern fills");
        assert_eq!(art(&result.doc).len(), 1, "{operator}: unread path placeholder");
    }
}

#[test]
fn gradient_instance_after_unresolved_pattern_is_read() {
    let defs =
        "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 0 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    for (operator, flag, draw) in [("p", "", "f"), ("P", "1 ", "S")] {
        let body = format!(
            "(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n{}{draw}\n{}{flag}Bb\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n10 0 0 10 100 50 Xm\n{draw}\n0 BB\n",
            rect_path(0.0, 0.0, 4.0, 4.0),
            rect_path(10.0, 0.0, 14.0, 4.0),
        );
        let result = read(stream(&format!("{defs}{}", layer(&body))).as_bytes()).unwrap();
        assert_unread(&result, "pattern fills");
        let a = art(&result.doc);
        assert_eq!(a.len(), 2, "{operator}");
        assert_eq!(result.unread[0].0, a[0].id, "{operator}: only the pattern's path is unread");
    }
}

#[test]
fn unresolved_pattern_in_mask_art_stays_unread() {
    let mask_path = rect_path(0.0, 0.0, 4.0, 4.0).lines().map(|line| format!("%_{line}\n")).collect::<String>();
    let patterned = format!("%_(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{mask_path}%_f\n");
    // A placed file with no preview, and a symbol whose definition the file doesn't have.
    let placed = "%_%AI5_BeginPlace\n%_%%FileType: 1\n%_[5 0 0 -5 0 200] 10 20 14 22 4 4 0\n%_(missing.png)`\n%_%%IncludeFile:missing.png\n%_~\n%_%AI5_EndPlace\n";
    let symbol = "%_/SymbolInstance :\n%_(Absent) /SymbolRef ,\n%_;\n";
    for (art_, kind) in [
        (patterned.as_str(), "pattern fills"),
        ("%_/Mesh X!\n%_/End X!\n", "gradient meshes it can't read"),
        (placed, "placed files"),
        (symbol, "symbols"),
        ("%_1 2 frobnicate\n", "`frobnicate`"),
    ] {
        let mask = format!(
            "%_/ArtDictionary :\n%_/Mask :\n%_X=\n{art_}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_1 /Bool (Linked) ,\n%_; (AI9_Mask) ,\n%_;\n"
        );
        let masked = format!("{RED}{}f\n{mask}", rect_path(0.0, 0.0, 4.0, 4.0));
        let e = read(stream(&layer(&masked)).as_bytes()).unwrap_err();
        assert!(e.contains(kind), "{e}");
        // In a group, the group is the unread object.
        let result = read_ok(&layer(&format!("u\n{masked}U\n")));
        assert_unread(&result, kind);
        assert!(result.hidden_unread.is_empty(), "{kind}: {:?}", result.hidden_unread);
        // A disabled mask doesn't change how its object looks: what it holds is left out.
        let disabled = masked.replace("%_1 /Bool (Linked) ,\n", "%_1 /Bool (Linked) ,\n%_1 /Bool (Disabled) ,\n");
        for body in [disabled.clone(), format!("u\n{disabled}U\n")] {
            let result = read_ok(&layer(&body));
            assert!(result.unread.is_empty(), "{kind}: {:?}", result.unread);
            assert!(result.hidden_unread.contains(kind), "{kind}: {:?}", result.hidden_unread);
        }
    }
    // Operators a mask's art doesn't know are kept up to 16.
    let ops = (0..40).map(|i| format!("%_op{i}\n")).collect::<String>();
    let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{ops}%_X+\n%_(Art) ,\n%_1 /Bool (Linked) ,\n%_; (AI9_Mask) ,\n%_;\n");
    let result = read_ok(&layer(&format!("u\n{RED}{}f\n{mask}U\n", rect_path(0.0, 0.0, 4.0, 4.0))));
    assert_eq!(result.unread.len(), 1);
    assert_eq!(result.unread[0].1.len(), 16);
}

#[test]
fn hidden_unread_art_beside_drawn_look_needs_no_page_replacement() {
    let hidden_path = rect_path(0.0, 0.0, 4.0, 4.0).lines().map(|line| format!("%_{line}\n")).collect::<String>();
    for (operator, draw, reset) in [("p", "f", "0 0 0 1 k"), ("P", "S", "0 0 0 1 K")] {
        let body = format!(
            "u\n%_(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n{hidden_path}%_{draw}\n{reset}\n{}{draw}\nU\n",
            rect_path(10.0, 0.0, 14.0, 4.0),
        );
        let result = read_ok(&layer(&body));
        assert!(result.unread.is_empty(), "{operator}: {:?}", result.unread);
        assert!(result.hidden_unread.contains("pattern fills"));
        let children = art(&result.doc);
        let group_art = children[0].children().expect("group children");
        assert_eq!(group_art.len(), 2);
        assert!(matches!(group_art[1].kind, NodeKind::Path { .. }));
    }
}

#[test]
fn unknown_operator_on_hidden_line_does_not_mark_drawn_group_unread() {
    let body = format!("u\n%_1 2 frobnicate\n{RED}{}f\nU\n", rect_path(10.0, 0.0, 14.0, 4.0));
    let result = read_ok(&layer(&body));
    assert!(result.unread.is_empty(), "{:?}", result.unread);
    assert!(result.hidden_unread.contains("`frobnicate`"));
    assert_eq!(art(&result.doc)[0].children().expect("group children").len(), 1);
}

/// A commented closing delimiter does not hide a visible group's unread content.
#[test]
fn visible_group_closed_on_hidden_line_stays_unread() {
    let body = format!("u\n1 2 frobnicate\n{RED}{}f\n%_U\n", rect_path(10.0, 0.0, 14.0, 4.0));
    let result = read_ok(&layer(&body));
    assert_eq!(result.unread.len(), 1);
    assert!(result.unread[0].1.contains(&"`frobnicate`".to_string()));
    assert!(!result.hidden_unread.contains("`frobnicate`"));
}

/// `Xw` hides unread objects even when their operators are on ordinary lines.
#[test]
fn xw_hidden_unread_keeps_visible_group_sibling() {
    for unread in
        ["1 2 frobnicate\n", "(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n0 0 m\n4 0 L\n4 4 L\nf\n", "/SymbolInstance :\n(Absent) /SymbolRef ,\n;\n"]
    {
        let body = format!("u\n1 Xw\n{unread}0 Xw\n0 0 0 1 k\n{}f\nU\n", rect_path(10.0, 0.0, 14.0, 4.0));
        let result = read_ok(&layer(&body));
        assert!(result.unread.is_empty(), "{unread}: {:?}", result.unread);
        assert!(!result.hidden_unread.is_empty(), "{unread}");
        let children = art(&result.doc);
        let group_art = children[0].children().expect("group children");
        assert_eq!(group_art.iter().filter(|node| node.visible).count(), 1, "{unread}");
    }
    let mut data = placed(true, "[5 0 0 -5 0 200]", "missing.png", None, "0 Xw\n");
    let at = data.windows(15).position(|w| w == b"%AI5_BeginPlace").unwrap();
    data.splice(at..at, *b"1 Xw\n");
    let result = read(&data).unwrap();
    assert!(result.unread.is_empty());
    assert!(result.hidden_unread.contains("placed files"));
}

#[test]
fn hidden_unread_symbol_does_not_mark_drawn_group_unread() {
    let body = format!("u\n%_/SymbolInstance :\n%_(Absent) /SymbolRef ,\n%_;\n{RED}{}f\nU\n", rect_path(10.0, 0.0, 14.0, 4.0));
    let result = read_ok(&layer(&body));
    assert!(result.unread.is_empty(), "{:?}", result.unread);
    assert!(result.hidden_unread.contains("symbols"));
    assert_eq!(art(&result.doc)[0].children().expect("group children").len(), 1);
}

#[test]
fn unresolved_pattern_paint_persists_past_plain_group_until_replaced() {
    for (operator, draw, reset) in [("p", "f", "0 0 0 1 k"), ("P", "S", "0 0 0 1 K")] {
        let body = format!(
            "u\n(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n{}{draw}\nU\n{}{draw}\n{reset}\n{}{draw}\n",
            rect_path(0.0, 0.0, 4.0, 4.0),
            rect_path(10.0, 0.0, 14.0, 4.0),
            rect_path(20.0, 0.0, 24.0, 4.0),
        );
        let result = read_ok(&layer(&body));
        assert_eq!(result.unread.len(), 2, "{operator}: {:?}", result.unread);
        assert_eq!(art(&result.doc).len(), 3);
    }
}

#[test]
fn unresolved_pattern_paint_inside_clip_restores_at_q() {
    for (operator, draw, reset) in [("p", "f", "0 0 0 1 k"), ("P", "S", "0 0 0 1 K")] {
        let body = format!(
            "q\n(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\n{}{draw}\nQ\n{}{draw}\n",
            rect_path(0.0, 0.0, 4.0, 4.0),
            rect_path(10.0, 0.0, 14.0, 4.0),
        );
        let result = read_ok(&layer(&body));
        assert_eq!(result.unread.len(), 1, "{operator}: {:?}", result.unread);
        assert_eq!(art(&result.doc).len(), 2);

        let body = format!(
            "(Absent) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] {operator}\nq\n{reset}\n{}{draw}\nQ\n{}{draw}\n",
            rect_path(0.0, 0.0, 4.0, 4.0),
            rect_path(10.0, 0.0, 14.0, 4.0),
        );
        let result = read_ok(&layer(&body));
        assert_eq!(result.unread.len(), 1, "{operator}: {:?}", result.unread);
        assert_eq!(result.unread[0].0, art(&result.doc)[1].id);
    }
}

#[test]
fn sections_it_skips() {
    let body = "%AI5_Begin_NonPrinting\n%AI14_BeginSymbol\n0 Ae\nu\nfrobnicate\n%AI10_EndSymbol\n%AI3_BeginPattern: (P)\nU\nU\n%AI3_EndPattern\n%AI5_End_NonPrinting--\n\
                %AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n%AI17_Begin_Content_if_version_gt:23 1\nfrobnicate\n%AI17_End_Versioned_Content\nfrobnicate\n%AI17_End_Versioned_Content\n\
                %_/Document :\n%_/Binary : /ASCII85Decode ,\n%_(]<[%\n%;;;~>\n%_; (Profile) ,\n%_;\n";
    let doc = read_ok(&format!("{body}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n")))).doc;
    assert_eq!(art(&doc).len(), 1);
    // A symbol whose art can't be read isn't kept.
    assert!(doc.symbols.is_empty());
}

/// Symbol definitions as the editing data writes them, in a section that doesn't print.
fn symbols(defs: &str) -> String {
    format!("%AI5_Begin_NonPrinting\n{defs}%AI5_End_NonPrinting--\n")
}

/// A symbol named `name` whose art is `art` (in the symbol's space, y down), with the registration
/// point `registration` (1 to 9 from the top left, row by row).
fn symbol(name: &str, art: &str, registration: u32) -> String {
    format!(
        "%AI14_BeginSymbol\n({name})\n0 A\n0 Xw\n0 Ae\nu\n{art}U\n%_/ArtDictionary :\n%_{registration} /Int (AI13PatternRegistrationType) ,\n%_;\n%_\n9 () XW\n%AI10_EndSymbol\n"
    )
}

/// An instance of the symbol named `name`, `m` taking the symbol's art to art space.
fn instance(name: &str, m: [f64; 6]) -> String {
    let [a, b, c, d, e, f] = m;
    format!("/SymbolInstance :\n({name}) /SymbolRef ,\n{a} {b} {c} {d} {e} {f} /RTransformMatrix\n;\n")
}

/// A red square of 4 points about the origin, as a symbol's art.
fn dot() -> String {
    format!("{RED}{}f\n", rect_path(-2.0, -2.0, 2.0, 2.0))
}

/// The visual box of `n` in `doc`, its instances measured by their symbols' art.
fn instance_box(doc: &Document, n: &Node) -> Rect {
    n.visual_bounds_with(&doc.symbol_boxes()).expect("a box")
}

#[test]
fn a_symbol_comes_in_once_and_its_instances_stay_live() {
    let body = symbols(&symbol("Dot", &dot(), 5))
        + &layer(&(instance("Dot", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]) + &instance("Dot", [2.0, 0.0, 0.0, -2.0, 100.0, 50.0])));
    let mut doc = read_ok(&body).doc;
    assert_eq!(doc.symbols.len(), 1);
    assert_eq!(doc.symbols[0].name, "Dot");
    assert!(doc.symbol_registration("Dot").is_center());
    let a = art(&doc);
    assert_eq!(a.len(), 2);
    let xfs: Vec<_> = a
        .iter()
        .map(|n| if let NodeKind::SymbolInstance { symbol, xf } = &n.kind { (symbol.clone(), *xf) } else { panic!("{:?}", n.kind) })
        .collect();
    assert_eq!(xfs[0].0, "Dot");
    assert_ne!(xfs[0].1, xfs[1].1);
    assert_eq!(instance_box(&doc, &a[0]), Rect::new(18.0, 48.0, 22.0, 52.0));
    assert_eq!(instance_box(&doc, &a[1]), Rect::new(96.0, 46.0, 104.0, 54.0));
    // Editing the symbol's art changes both instances.
    let wider = read_ok(&(symbols(&symbol("Dot", &format!("{RED}{}f\n", rect_path(-4.0, -2.0, 4.0, 2.0)), 5)) + &layer(""))).doc;
    doc.symbols[0].art = wider.symbols[0].art.clone();
    assert_eq!(instance_box(&doc, &a[0]), Rect::new(16.0, 48.0, 24.0, 52.0));
    assert_eq!(instance_box(&doc, &a[1]), Rect::new(92.0, 46.0, 108.0, 54.0));
}

#[test]
fn an_instance_in_a_symbols_art_is_placed_from_the_canvas() {
    // The template box's center is (100.5, 50.5): art space's origin is at canvas (8091, 8242).
    let nested = instance("Dot", [1.0, 0.0, 0.0, -1.0, -8081.0, 8237.0]);
    let defs = symbols(&(symbol("Dot", &dot(), 5) + &symbol("Pair", &nested, 1)));
    let body = format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{defs}{}", layer(&instance("Pair", [1.0, 0.0, 0.0, -1.0, 50.0, 50.0])));
    let doc = read_ok(&body).doc;
    let names: Vec<&str> = doc.symbols.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["Dot", "Pair"]);
    assert_eq!(doc.symbol_registration("Pair"), vectorcraft_doc::SymbolRegistration(0));
    // Dot sits at (10, 5) in Pair's art, which the page instance puts at (50, 50).
    let a = art(&doc);
    assert_eq!(instance_box(&doc, &a[0]), Rect::new(58.0, 53.0, 62.0, 57.0));
}

#[test]
fn an_image_in_a_symbols_art_is_placed_from_the_canvas() {
    // The template box's center is (100.5, 50.5): art space's origin is at canvas (8091, 8242). The
    // image's matrix is in the canvas's space turned y up about that point, as an instance's in a
    // symbol's art is: its top left pixel goes to (-5, -2.5) in the symbol's art.
    let m = "[ 5 0 0 5 -8096 8244.5 ]";
    let raster = format!(
        "%AI5_File:\n%AI5_BeginRaster\n() 1 XG\n/DeviceRGB XN\n{m} 2 1 0 Xh\n{m} 0 0 2 1 2 1 8 3 0 0 1 0 4 4 0 0\n\
         %%BeginData: 6\rXI\nABCDEF%%EndData\r\nXH\r\n%AI5_EndRaster\r\nN\n"
    );
    let body = format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{}{}", symbols(&symbol("Pic", &raster, 5)), layer(""));
    let doc = read_ok(&body).doc;
    assert_eq!(doc.symbols.len(), 1);
    let NodeKind::Image(im) = &doc.symbols[0].art.kind else { panic!("{:?}", doc.symbols[0].art.kind) };
    for ((u, v), want) in [((0.0, 0.0), (-5.0, -2.5)), ((2.0, 0.0), (5.0, -2.5)), ((0.0, 1.0), (-5.0, 2.5))] {
        let got = im.xf * Point::new(u, v);
        assert!((got - Point::new(want.0, want.1)).hypot() < 1e-9, "pixel ({u}, {v}): {got:?}, want {want:?}");
    }
    // A linked image (`XG` and `XF`) is placed the same way.
    let linked = raster7("(a.tif) 0 XG\n", m, (2, 1), "XF", "");
    let doc = read_ok(&format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{}{}", symbols(&symbol("Link", &linked, 5)), layer(""))).doc;
    let NodeKind::Image(link) = &doc.symbols[0].art.kind else { panic!("{:?}", doc.symbols[0].art.kind) };
    assert!(link.link.is_some());
    assert!((link.xf * Point::new(0.0, 0.0) - Point::new(-5.0, -2.5)).hypot() < 1e-9, "{:?}", link.xf);
    // Without a template box the image can't be placed, so the symbol isn't read whole: its
    // instance comes in as the page draws it.
    let group = format!("0 Ae\nu\n{}{RED}{}f\nU\n", instance("Pic", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]), rect_path(10.0, 0.0, 14.0, 4.0));
    let r = read_ok(&(symbols(&symbol("Pic", &raster, 5)) + &layer(&group)));
    assert!(r.doc.symbols.is_empty(), "{:?}", r.doc.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert_eq!(r.unread.len(), 1, "{:?}", r.warnings);
}

#[test]
fn an_instance_keeps_its_hide_lock_transparency_and_clip() {
    let m = [1.0, 0.0, 0.0, -1.0, 20.0, 50.0];
    let art_ = format!(
        "1 Xw\n{}0 Xw\n1 A\n{}0 A\n{}1 0.5 1 1 1 Xy\n0 0 Xd\n6 () XW\n0 Ae\nq\n{}{}h\nW\nn\nQ\n0 0.25 0 0 0 Xy\n{}",
        instance("Dot", m),
        instance("Dot", m),
        instance("Dot", m),
        instance("Dot", m),
        rect_path(19.0, 49.0, 21.0, 51.0),
        instance("Dot", m),
    );
    let doc = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&art_))).doc;
    let a = art(&doc);
    assert_eq!(a.len(), 5);
    assert!(!a[0].visible && !a[0].locked);
    assert!(a[1].visible && a[1].locked);
    // An instance's transparency is the `Xy` after it, as a group's is; one set before it isn't its.
    assert_eq!((a[2].opacity, a[2].blend, a[2].isolate, a[2].knockout, a[2].knockout_shape), (0.5, BlendMode::Multiply, true, Knockout::On, true));
    assert_eq!((a[1].opacity, a[1].blend, a[1].isolate, a[1].knockout), (1.0, BlendMode::Normal, false, Knockout::Neutral));
    assert_eq!((a[3].opacity, a[4].opacity), (0.25, 1.0));
    let NodeKind::Group { children, clip: true } = &a[3].kind else { panic!("a clipping group: {:?}", a[3].kind) };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(matches!(children[1].kind, NodeKind::SymbolInstance { .. }));
}

/// The art of an opacity mask in a symbol's art takes ids of the document, as the rest of the art
/// does: no two objects of the document, in its layers or its symbols, share an id.
#[test]
fn a_symbols_opacity_mask_art_takes_ids_of_the_document() {
    fn ids(n: &Node, out: &mut Vec<vectorcraft_doc::NodeId>) {
        out.push(n.id);
        if let Some(m) = n.extras().mask.as_ref() {
            ids(&m.art, out);
        }
        for c in n.children().into_iter().flatten() {
            ids(c, out);
        }
    }
    let square = rect_path(-2.0, -2.0, 2.0, 2.0);
    let mask_art: String = format!("1 g\n{square}f\n").lines().map(|l| format!("%_{l}\n")).collect();
    let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{mask_art}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_; (AI9_Mask) ,\n%_;\n");
    let defs = symbols(&symbol("Masked", &format!("{RED}{square}f\n{mask}{}", dot()), 5));
    let shapes: String = (0..6).map(|i| format!("{RED}{}f\n", rect_path(f64::from(i) * 10.0, 0.0, f64::from(i) * 10.0 + 4.0, 4.0))).collect();
    let doc = read_ok(&(defs + &layer(&format!("{shapes}{}", instance("Masked", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))))).doc;
    let mut all = Vec::new();
    for n in doc.layers.iter().chain(doc.symbols.iter().map(|s| &s.art)) {
        ids(n, &mut all);
    }
    assert!(doc.symbols.iter().any(|s| s.art.children().into_iter().flatten().any(|c| c.extras().mask.is_some())), "the mask is read");
    let unique: std::collections::HashSet<_> = all.iter().copied().collect();
    assert_eq!(unique.len(), all.len(), "{all:?}");
}

/// A layer's `Xy` right after its last object, an instance, before the layer's `0 0 Xd` and
/// `7 () XW`, is the layer's; the instance's own state block (`6 () XW`) keeps it the instance's.
#[test]
fn a_layers_state_after_its_last_symbol_instance_is_the_layers() {
    let last = instance("Dot", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]);
    let doc = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&format!("{last}1 0.6 0 2 0 Xy\n0 0 Xd\n7 () XW\n")))).doc;
    assert_eq!((doc.layers[0].opacity, doc.layers[0].blend), (0.6, BlendMode::Multiply));
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].blend), (1.0, BlendMode::Normal));
    let doc = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&format!("{last}1 0.6 0 2 0 Xy\n0 0 Xd\n6 () XW\n")))).doc;
    assert_eq!(doc.layers[0].opacity, 1.0);
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].blend), (0.6, BlendMode::Multiply));
}

#[test]
fn the_symbol_list_orders_the_symbols_and_names_stay_apart() {
    let defs = symbol("A", &dot(), 5) + &symbol("B", &dot(), 5) + &symbol("A", &dot(), 5) + &symbol("C", &dot(), 9);
    for list in [
        "%AI24_BeginSymbolList\n(C)\n(Ghost)\n(B)\n(C)\n%AI24_EndSymbolList\n",
        "%AI10_BeginSymbolList\n(C)\n(Ghost)\n(B)\n(C)\n%AI10_EndSymbolList\n",
    ] {
        let doc = read_ok(&(symbols(&format!("{defs}{list}")) + &layer(&instance("A", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])))).doc;
        let names: Vec<&str> = doc.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["C", "B", "A", "A 2"], "{list}");
        assert_eq!(doc.symbol_registration("C"), vectorcraft_doc::SymbolRegistration(8));
        // An instance names the first symbol the file gives that name.
        assert!(matches!(&art(&doc)[0].kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "A"));
    }
}

/// A symbol whose art is filled with a pattern isn't kept: its instances come in as the page draws
/// them, within the box of its art. The pattern keeps the name it was numbered with.
#[test]
fn a_symbol_with_a_pattern_fill_isnt_kept() {
    let pattern = DOTS.replace("(Dots)", "(Red\\n1)");
    let lawn = symbol("Lawn", &format!("(Red\\n1) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(-2.0, -2.0, 2.0, 2.0)), 5);
    let r = read_ok(&format!("{pattern}{}{}", symbols(&lawn), layer(&instance("Lawn", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
    assert_eq!(r.doc.patterns.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Red 2"]);
    assert!(r.doc.symbols.is_empty());
    assert_unread(&r, "symbols");
    let (id, _) = &r.unread[0];
    assert_eq!(r.doc.node(*id).and_then(|n| n.geometric_bounds()), Some(Rect::new(18.0, 48.0, 22.0, 52.0)));
    // A pattern chosen and then replaced by a color before the art is painted fills nothing.
    let solid = symbol("Solid", &format!("(Red\\n1) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{RED}{}f\n", rect_path(-2.0, -2.0, 2.0, 2.0)), 5);
    let r = read_ok(&format!("{pattern}{}{}", symbols(&solid), layer(&instance("Solid", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
    assert_eq!(r.doc.symbols.len(), 1);
    assert!(r.unread.is_empty());
}

#[test]
fn an_instance_of_a_symbol_it_doesnt_have_is_the_unread_object() {
    let missing = instance("Absent", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]);
    // Alone on the layer, the file isn't read: the page has no box for it.
    let e = read(stream(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&missing))).as_bytes()).unwrap_err();
    assert!(e.contains("symbols"), "{e}");
    // In a group, the group is the unread object.
    let group = format!("0 Ae\nu\n{missing}{RED}{}f\nU\n", rect_path(10.0, 0.0, 14.0, 4.0));
    let r = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&group)));
    assert_unread(&r, "symbols");
    // Nor is a symbol whose art holds an instance of one the file doesn't have: the page has no box
    // for its instances.
    let holder = symbol("Holder", &(dot() + &instance("Absent", [1.0, 0.0, 0.0, -1.0, 0.0, 0.0])), 5);
    let body =
        format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{}{}", symbols(&holder), layer(&instance("Holder", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])));
    let e = read(stream(&body).as_bytes()).unwrap_err();
    assert!(e.contains("symbols"), "{e}");
    // An instance whose matrix doesn't have six finite numbers isn't read either.
    let bad = "/SymbolInstance :\n(Dot) /SymbolRef ,\n1 0 0 -1 20 /RTransformMatrix\n;\n";
    let r = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(&format!("0 Ae\nu\n{bad}U\n"))));
    assert_unread(&r, "symbols");
}

#[test]
fn an_instance_whose_appearance_differs_from_its_symbols_comes_in_as_the_page_draws_it() {
    let edited = "/SymbolInstance :\n(Dot) /SymbolRef ,\n1 0 0 -1 20 50 /RTransformMatrix\n,\n/Dictionary :\n; (AI19SymbolAppearanceMap) ,\n;\n";
    let r = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(edited)));
    assert_unread(&r, "symbols");
    let (id, _) = &r.unread[0];
    assert_eq!(r.doc.node(*id).and_then(|n| n.geometric_bounds()), Some(Rect::new(18.0, 48.0, 22.0, 52.0)), "the box is the symbol's, placed");
    // The appearance changes are found in a dictionary that an array holds as a value without a key.
    let in_array = "/SymbolInstance :\n(Dot) /SymbolRef ,\n1 0 0 -1 20 50 /RTransformMatrix\n,\n\
                    /Array :\n/Dictionary :\n/Dictionary :\n; (AI19SymbolAppearanceMap) ,\n;\n;\n; (Edits) ,\n;\n";
    let r = read_ok(&(symbols(&symbol("Dot", &dot(), 5)) + &layer(in_array)));
    assert_unread(&r, "symbols");
    // So does an instance of a symbol whose art isn't read whole, within that art's box.
    let partial = symbol("Partial", &format!("{}f\n1 Xw\n1 2 frobnicate\n0 Xw\n", rect_path(-2.0, -2.0, 2.0, 2.0)), 5);
    let r = read_ok(&(symbols(&partial) + &layer(&instance("Partial", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
    assert!(r.doc.symbols.is_empty());
    assert_unread(&r, "symbols");
    let (id, _) = &r.unread[0];
    assert_eq!(r.doc.node(*id).and_then(|n| n.geometric_bounds()), Some(Rect::new(18.0, 48.0, 22.0, 52.0)));
}

/// A symbol whose art holds an image that is left out (no matrix, a matrix with no area, data cut
/// short) isn't kept, so its instances don't draw it without the image.
#[test]
fn a_symbol_with_an_image_it_leaves_out_isnt_kept() {
    for (what, image) in [
        ("no matrix", raster7("", "", (1, 1), "XI", "%FF0000\n")),
        ("a matrix with no area", raster7("", "[ 0 0 0 0 0 0 ]", (1, 1), "XI", "%FF0000\n")),
        ("data cut short", raster7("", "[ 1 0 0 1 0 0 ]", (2, 2), "XI", "%FF0000\n")),
        ("no end", raster7("", "[ 1 0 0 1 0 0 ]", (1, 1), "XI", "%FF0000\n").replace("%AI5_EndRaster\n", "")),
        ("art after a trailer", format!("%%PageTrailer\n{}", raster7("", "[ 1 0 0 1 0 0 ]", (1, 1), "XI", "%FF0000\n"))),
    ] {
        // With a template box, so that the image could be placed.
        let defs = symbols(&symbol("Photo", &(dot() + &image), 5));
        let group = format!("0 Ae\nu\n{}{RED}{}f\nU\n", instance("Photo", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]), rect_path(10.0, 0.0, 14.0, 4.0));
        let r = read_ok(&format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{defs}{}", layer(&group)));
        assert!(r.doc.symbols.is_empty(), "{what}");
        assert_eq!(r.unread.len(), 1, "{what}: {:?}", r.warnings);
    }
}

/// A symbol's art written as a group holding one object stays that group when the group is tagged
/// (`XT`), so the tag isn't lost.
#[test]
fn a_symbols_tagged_group_keeps_its_tag() {
    let def = format!(
        "%AI14_BeginSymbol\n(Tagged)\n0 A\n0 Xw\n0 Ae\nu\n{}U\n/AdobeURL (https://example.com/a) XT\n%_/ArtDictionary :\n%_5 /Int (AI13PatternRegistrationType) ,\n%_;\n%_\n9 () XW\n%AI10_EndSymbol\n",
        dot()
    );
    let doc = read_ok(&(symbols(&def) + &layer(&instance("Tagged", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])))).doc;
    let art = &doc.symbols[0].art;
    assert!(matches!(art.kind, NodeKind::Group { .. }), "{:?}", art.kind);
    assert_eq!(tags(art).0, "https://example.com/a");
}

/// A definition whose art a trailer stops isn't kept, though the art before the trailer is whole.
#[test]
fn a_symbol_cut_off_by_a_trailer_isnt_kept() {
    let cut = format!("%AI14_BeginSymbol\n(Cut)\n0 A\n0 Xw\n0 Ae\n{}%%PageTrailer\n{}%AI10_EndSymbol\n", dot(), dot());
    let group = format!("0 Ae\nu\n{}{RED}{}f\nU\n", instance("Cut", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]), rect_path(10.0, 0.0, 14.0, 4.0));
    let r = read_ok(&(symbols(&cut) + &layer(&group)));
    assert!(r.doc.symbols.is_empty(), "{:?}", r.doc.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert_unread(&r, "symbols");
}

/// A symbol whose art holds type, a placed file, non-native art, an image it can't place or a
/// gradient the file doesn't define isn't kept. Its instances come in as the page draws them, within the box of the rest of its art, an
/// instance right on a layer as well as one in a group.
#[test]
fn a_symbols_art_it_leaves_out_gives_its_instances_the_box_of_the_rest() {
    let typed = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n".to_string();
    let placed = "%AI5_BeginPlace\n%AI5_EndPlace\n".to_string();
    let foreign =
        "/ForeignObject :\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n%,u@!!/MSk8%41#ocdN=10d&\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n"
            .to_string();
    let image = raster7("", "[ 1 0 0 1 0 0 ]", (1, 1), "XI", "%FF0000\n");
    let gradient = format!("Bb\n1 (Nowhere) 0 0 0 10 1 0 0 1 0 0 Bg\n{}f\n0 BB\n", rect_path(-2.0, -2.0, 2.0, 2.0));
    let template = "%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n";
    for (what, left, head) in [
        ("type", typed, template),
        ("a placed file", placed, template),
        ("non-native art", foreign, template),
        ("an image without a template box", image, ""),
        ("a gradient the file doesn't define", gradient, template),
    ] {
        let defs = symbols(&symbol("Mixed", &(dot() + &left), 5));
        let body = format!("{head}{defs}{}", layer(&instance("Mixed", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])));
        let r = read(stream(&body).as_bytes()).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(r.doc.symbols.is_empty(), "{what}");
        assert_unread(&r, "symbols");
        let (id, _) = &r.unread[0];
        assert_eq!(r.doc.node(*id).and_then(|n| n.geometric_bounds()), Some(Rect::new(18.0, 48.0, 22.0, 52.0)), "{what}");
    }
    // A gradient the file doesn't define on a path that isn't painted paints nothing.
    // Nor does one on a path that a group's start drops, for the paths after it.
    let square = rect_path(-2.0, -2.0, 2.0, 2.0);
    for (what, unpainted) in [
        ("not painted", format!("Bb\n1 (Nowhere) 0 0 0 10 1 0 0 1 0 0 Bg\n{square}n\n0 BB\n")),
        ("dropped", format!("Bb\n1 (Nowhere) 0 0 0 10 1 0 0 1 0 0 Bg\n{square}0 BB\nu\n{square}f\nU\n")),
    ] {
        let defs = symbols(&symbol("Kept", &(dot() + &unpainted), 5));
        let r = read_ok(&format!("{template}{defs}{}", layer(&instance("Kept", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
        assert_eq!(r.doc.symbols.len(), 1, "{what}: {:?}", r.warnings);
        assert!(r.unread.is_empty(), "{what}");
    }
}

#[test]
fn symbols_that_hold_each_other_lose_the_instances_that_close_the_loop() {
    let m = [1.0, 0.0, 0.0, -1.0, 0.0, 0.0];
    let defs = symbols(&(symbol("S1", &instance("S2", m), 5) + &symbol("S2", &instance("S1", m), 5)));
    let r =
        read_ok(&format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{defs}{}", layer(&(dot() + &instance("S1", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])))));
    assert!(r.warnings.contains(&vectorcraft_doc::depth::symbols_note()), "{:?}", r.warnings);
    assert!(!r.doc.clone().limit_depth(), "nothing is left that draws itself");
}

/// A definition after the first layer isn't read: the symbols were checked before that layer, and
/// one read later could hold an instance of a symbol the file doesn't have.
#[test]
fn a_symbol_defined_after_the_first_layer_is_not_read() {
    let late = symbols(&symbol("Late", &(dot() + &instance("Absent", [1.0, 0.0, 0.0, -1.0, 0.0, 0.0])), 5));
    let group = format!("0 Ae\nu\n{}{RED}{}f\nU\n", instance("Late", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]), rect_path(10.0, 0.0, 14.0, 4.0));
    let r = read_ok(&format!("%AI3_TemplateBox: 100.5 50.5 100.5 50.5\n{}{late}{}", layer(&dot()), layer(&group)));
    assert!(r.doc.symbols.is_empty(), "{:?}", r.doc.symbols.iter().map(|s| &s.name).collect::<Vec<_>>());
    assert_unread(&r, "symbols");
}

/// A filled square at x = `10 × i` (5 pt wide), to tell the alternatives of versioned content apart.
fn alt_square(i: u32) -> String {
    let x = f64::from(i) * 10.0;
    rect_path(x, 0.0, x + 5.0, 5.0) + "f\n"
}

/// The left edges of the layer's art, in order: which [`alt_square`]s came in.
fn squares(doc: &Document) -> Vec<f64> {
    art(doc).iter().filter_map(|n| n.geometric_bounds()).map(|b| b.x0.round()).collect()
}

/// Versioned content holds alternatives of the same art for newer and older versions (newer
/// files write a Dimension tool group, then the same group again for a version in between, then
/// plain art): only the first alternative is read, so the art comes in once, however many
/// alternatives follow and whatever versioned content is nested in them.
#[test]
fn versioned_content_reads_one_alternative() {
    let (begin, between, alternate, end) = (
        "%AI17_Begin_Content_if_version_gt:24 11\n",
        "%AI17_Alternate_Content_if_version_gt:24 10\n",
        "%AI17_Alternate_Content\n",
        "%AI17_End_Versioned_Content\n",
    );
    let (a, b, c) = (alt_square(1), alt_square(2), alt_square(3));
    let cases = [
        ("one alternative", format!("{begin}{a}{end}"), vec![10.0]),
        ("two alternatives", format!("{begin}{a}{alternate}{b}{end}"), vec![10.0]),
        ("three alternatives", format!("{begin}{a}{between}{b}{alternate}{c}{end}"), vec![10.0]),
        // Blocks nested in the first alternative read their own first; those in a later one aren't read.
        (
            "nested",
            format!(
                "{begin}{a}{begin}{b}{alternate}{c}{end}{between}{}{begin}{}{alternate}{}{end}{alternate}{}{end}",
                alt_square(4),
                alt_square(5),
                alt_square(6),
                alt_square(7)
            ),
            vec![10.0, 20.0],
        ),
        // Art after the block is read as usual.
        ("art after it", format!("{begin}{a}{between}{b}{alternate}{c}{end}{}", alt_square(8)), vec![10.0, 80.0]),
    ];
    for (case, body, want) in cases {
        let s = read_ok(&layer(&body));
        assert_eq!(squares(&s.doc), want, "{case}");
        assert!(!s.warnings.iter().any(|w| w == read::VERSIONED_UNENDED), "{case}: {:?}", s.warnings);
    }
}

/// Versioned content whose end is missing is read whole, with a note, rather than losing the rest
/// of the file.
#[test]
fn versioned_content_that_does_not_end_is_read_with_a_note() {
    let body = format!("%AI17_Begin_Content_if_version_gt:24 11\n{}%AI17_Alternate_Content_if_version_gt:24 10\n{}", alt_square(1), alt_square(2));
    let s = read_ok(&layer(&body));
    assert_eq!(squares(&s.doc), [10.0, 20.0]);
    assert!(s.warnings.iter().any(|w| w == read::VERSIONED_UNENDED), "{:?}", s.warnings);
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

/// The same data in a symbol's definition: its `%_` line with a `(` opens no string, so the
/// definition ends at its own end marker and the symbol after it comes in.
#[test]
fn non_native_art_in_a_symbol_ends_at_the_definitions_end() {
    let foreign = "/ForeignObject :\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n";
    let defs = symbols(&(symbol("Pdf", &format!("{foreign}{}", dot()), 5) + &symbol("Dot", &dot(), 5)));
    let s = read_ok(&(defs + &layer(&instance("Dot", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
    let names: Vec<&str> = s.doc.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Dot"), "{names:?}");
    let a = art(&s.doc);
    assert_eq!(a.len(), 1);
    assert!(matches!(&a[0].kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Dot"), "{:?}", a[0].kind);
}

/// Comment lines between a dictionary's name and its `:`, or an entry's key and its `,`, don't hide
/// the data from the scan of a symbol's definition.
#[test]
fn non_native_art_with_comments_in_a_symbol_ends_at_the_definitions_end() {
    let foreign = "/ForeignObject\n%note\n:\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data\n%note\n,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n";
    let defs = symbols(&(symbol("Pdf", &format!("{foreign}{}", dot()), 5) + &symbol("Dot", &dot(), 5)));
    let s = read_ok(&(defs + &layer(&instance("Dot", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
    let names: Vec<&str> = s.doc.symbols.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"Dot"), "{names:?}");
}

/// A string keys an entry as a name does: `(Data) ,` doesn't hide the data from the scan of a
/// symbol's definition either.
#[test]
fn non_native_art_keyed_by_a_string_in_a_symbol_ends_at_the_definitions_end() {
    for key in ["(Data)", "<44617461>"] {
        let foreign = format!(
            "/ForeignObject :\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n{key} ,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n"
        );
        let defs = symbols(&(symbol("Pdf", &format!("{foreign}{}", dot()), 5) + &symbol("Dot", &dot(), 5)));
        let s = read_ok(&(defs + &layer(&instance("Dot", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0]))));
        let names: Vec<&str> = s.doc.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Dot"), "{key}: {names:?}");
    }
}

/// A `/Binary` dictionary's data in a skipped alternate: its `%_` line with a `(` opens no string,
/// so the block ends at its own end marker.
#[test]
fn binary_data_in_a_skipped_alternate_ends_at_the_blocks_end() {
    let binary = "%_/Document :\n%_/Binary : /ASCII85Decode ,\n%_(]<[%\n%;;;~>\n%_; (Profile) ,\n%_;\n";
    let body = format!(
        "%AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n{binary}frobnicate\n%AI17_End_Versioned_Content\n{}",
        layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))
    );
    let s = read_ok(&body);
    assert_eq!(art(&s.doc).len(), 1);
    assert!(!s.warnings.iter().any(|w| w == read::VERSIONED_UNENDED), "{:?}", s.warnings);
}

/// A text document's data, the free type's undo copy included: its `%_` line with a `(` opens no
/// string, even after a line that reads as another section's end marker, so the section ends at
/// its own end marker and the art after it comes in.
#[test]
fn text_document_data_ends_at_the_sections_end() {
    for ty in ["AI11TextDocument", "AI11UndoFreeTextDocument"] {
        let document =
            format!("%AI11_BeginTextDocument\n/{ty} : /ASCII85Decode ,\n%,u@!!/MSk8\n%AI3_EndPattern\n%_(]<[%\n%;;;~>\n%_;\n%AI11_EndTextDocument\n");
        let s = read_ok(&format!("{document}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
        assert_eq!(art(&s.doc).len(), 1, "{ty}");
    }
}

/// Data without its `~>` isn't skipped: the section still ends at its own end marker, with no `~>`
/// after it, with one in a later section, and with later data that holds a `(` on a `%_` line.
#[test]
fn data_without_its_end_does_not_hide_the_sections_end() {
    let unended = "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n%,u@!!/MSk8\n%AI11_EndTextDocument\n";
    let ended = "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n%,u@!!/MSk8\n%_(]<[%\n%;;;~>\n%_;\n%AI11_EndTextDocument\n";
    let art_layer = layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"));
    for body in [format!("{unended}{art_layer}"), format!("{unended}{art_layer}{ended}"), format!("{unended}{ended}{art_layer}")] {
        let s = read_ok(&body);
        assert_eq!(art(&s.doc).len(), 1, "{body}");
    }
}

/// Data without its `~>` is skipped up to the section's end marker: its `%_` line with a `(` opens
/// no string, so the section ends there and the art after it comes in. A `%` right after the key
/// starts no marker, so it doesn't end the data.
#[test]
fn data_without_its_end_holding_a_string_ends_at_the_sections_end() {
    let art_layer = layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"));
    for document in [
        "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n%,u@!!/MSk8\n%_(]<[%\n%AI11_EndTextDocument\n",
        "%AI11_BeginTextDocument\r\n/AI11TextDocument : /ASCII85Decode ,\r\n%_(]<[%\r\n%AI11_EndTextDocument\r\n",
        "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,%AI11_EndTextDocument\n%_(]<[%\n%AI11_EndTextDocument\n",
    ] {
        let s = read_ok(&format!("{document}{art_layer}"));
        assert_eq!(art(&s.doc).len(), 1, "{document}");
    }
}

/// Data without its `~>` stops at the start of a section of the same kind nested after it too, so
/// the outer section still ends at its own end marker and not at the nested one's.
#[test]
fn data_without_its_end_stops_at_a_nested_sections_start() {
    let square = rect_path(0.0, 0.0, 1.0, 1.0) + "f\n";
    let body = format!(
        "%AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n/Binary : /ASCII85Decode ,\n%_(]<[%\n\
         %AI17_Begin_Content_if_version_gt:23 1\nfrobnicate\n%AI17_End_Versioned_Content\n{square}%AI17_End_Versioned_Content\n{}",
        layer(&square)
    );
    let s = read_ok(&body);
    assert_eq!(art(&s.doc).len(), 1);
    assert!(!s.warnings.iter().any(|w| w == read::VERSIONED_UNENDED), "{:?}", s.warnings);
}

/// Versioned content that doesn't end is scanned again for the swatches' names: data with its `~>`
/// is still skipped, though data with none comes after it, and the dictionary open where the
/// content starts still counts.
#[test]
fn swatch_names_are_found_past_data_in_versioned_content_that_does_not_end() {
    let entries = "1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n0 0 Xd\n6 () XW\n0 Ae\n";
    let alternate = "%AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n";
    let sea = format!("0 1 1 0 (Sea) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let unended = "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n%,u@!!/MSk8\n%AI11_EndTextDocument\n";
    for art_text in [format!("{alternate}/ForeignObject :\n{entries}{sea}{unended}"), format!("/ForeignObject :\n{alternate}{entries}{sea}{unended}")]
    {
        let body = format!("{}{}", DOTS.replace("(Dots)", "(Sea\\n1)"), layer(&art_text));
        let d = read(stream(&body).as_bytes()).unwrap().doc;
        let names: Vec<&str> = d.patterns.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Sea 2"], "{art_text}");
    }
}

/// A search for where data stops that found nothing outside a section doesn't stop a later
/// section's data at that section's end, so the named color after it still takes its name first.
/// The `/Binary` holds a group, where the reader doesn't take what follows its key as data.
#[test]
fn swatch_names_are_found_past_data_in_a_section_after_a_search_outside_sections_found_nothing() {
    let sea = format!("0 1 1 0 (Sea) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let before = "/Binary :\nu\n/ASCII85Decode ,\nU\n;\n";
    for section in [
        "%AI17_Begin_Content_if_version_gt:24 4\n%AI17_Alternate_Content\n/Binary : /ASCII85Decode ,\n%_(]<[%\n%AI17_End_Versioned_Content\n",
        "%AI11_BeginTextDocument\n/AI11TextDocument : /ASCII85Decode ,\n%_(]<[%\n%AI11_EndTextDocument\n",
    ] {
        let art_text = format!("{before}{section}{sea}");
        let body = format!("{}{}", DOTS.replace("(Dots)", "(Sea\\n1)"), layer(&art_text));
        let d = read(stream(&body).as_bytes()).unwrap().doc;
        let names: Vec<&str> = d.patterns.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Sea 2"], "{art_text}");
    }
}

/// The names the swatches take are found past the same data: a named color after it still takes
/// its name first, and the pattern named after it is numbered.
#[test]
fn swatch_names_are_found_past_non_native_art_data() {
    let foreign = "/ForeignObject :\n1 /Version ,\n2 0 0 2 10 20  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n%,u@!!/MSk8%41#ocdN=10d&\n%_O%*mGn(RduU.]4`[u[.f\n%_;(]<[%\n%1H@<P2`V<S,pbuU7L]\\~>\n;\n0 0 Xd\n6 () XW\n0 Ae\n";
    let sea = format!("0 1 1 0 (Sea) 0 x\n{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    let body = format!("{}{}", DOTS.replace("(Dots)", "(Sea\\n1)"), layer(&format!("{foreign}{sea}")));
    let d = read(stream(&body).as_bytes()).unwrap().doc;
    let names: Vec<&str> = d.patterns.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Sea 2"]);
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
/// space, which is y 14..26 in the document (crop marks 100 high). That placement mirrors (`2 0 0 2`
/// with the origin's flip), so the PDF's bottom (blue) is the top of the box.
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

/// The usual placement, `/RTransform` `1 0 0 -1 …`, doesn't mirror (with the origin's flip its
/// determinant is positive), so the art shows upright, as the file's own PDF part draws it: the
/// PDF's top (red) at the top. Bounds
/// (1, 2)–(5, 8) from the origin (10, 20), y down, → (11, 18)–(15, 12) → x 111..115, y 32..38 in
/// art space, y 62..68 in the document (crop marks 100 high).
#[test]
fn non_native_art_placed_without_a_mirror_stays_upright() {
    let pdf = tiny_pdf(10.0, "0 0 1 rg 1 2 4 3 re f 1 0 0 rg 1 5 4 3 re f");
    let foreign = format!(
        "q\n/ForeignObject :\n1 /Version ,\n1 0 0 -1 100 50  /RTransform ,\n10 20  /Origin ,\n1 2 5 8  /Bounds ,\n/Data ,\n{};\n0 0 Xd\n6 () XW\n0 Ae\n{}h\nW\nn\nQ\n",
        ascii85_comment_lines(&pdf),
        rect_path(110.0, 30.0, 116.0, 40.0)
    );
    let s = read_ok(&layer(&foreign));
    assert!(s.warnings.is_empty(), "{:?}", s.warnings);
    let objects = art(&s.doc);
    let NodeKind::Group { children, clip: true } = &objects[0].kind else { panic!("{:?}", objects[0].kind) };
    let group = &children[1];
    let bounds = group.geometric_bounds().unwrap();
    for (got, want) in [(bounds.x0, 111.0), (bounds.y0, 62.0), (bounds.x1, 115.0), (bounds.y1, 68.0)] {
        assert!((got - want).abs() < 1e-6, "{bounds:?}");
    }
    let paths = group.children().unwrap();
    let (blue, red) = (paths[0].geometric_bounds().unwrap(), paths[1].geometric_bounds().unwrap());
    assert!((red.y0 - 62.0).abs() < 1e-6 && (red.y1 - 65.0).abs() < 1e-6, "red {red:?}");
    assert!((blue.y0 - 65.0).abs() < 1e-6 && (blue.y1 - 68.0).abs() < 1e-6, "blue {blue:?}");
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

const G_DEFS: &str = "%AI5_BeginGradient: (G)\n(G) 1 2 Bd\n[\n<00FF>\n0\n0 0 0 0 1 0 0 2 1 6 %_BS\n\
                      %_0 0 0 0 1 0 0 2 1 6 40 0 Bs\n%_0 0 0 0 0 0 1 2 0.5 6 50 100 Bs\nBD\n%AI5_EndGradient\n";

/// A radial gradient instance whose highlight line is `bh`, on a rectangle.
fn highlighted(bh: &str) -> String {
    let art_ = format!("{RED}{}Bb\n{bh} Bh\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n50 0 0 -50 100 50 Bm\nf\n0 BB\n", rect_path(50.0, 0.0, 150.0, 100.0));
    format!("{G_DEFS}{}", layer(&art_))
}

#[test]
fn an_undefined_highlight_is_a_gradient_without_one() {
    // `--` stands for a number the writer has no value for, here the highlight's point.
    let none = read_ok(&highlighted("-- -- 269.6 0.87")).doc;
    let zero = read_ok(&highlighted("0 0 269.6 0.87")).doc;
    let Paint::Gradient(g) = fill(&art(&none)[0]) else { panic!("{:?}", fill(&art(&none)[0])) };
    let Paint::Gradient(z) = fill(&art(&zero)[0]) else { panic!("{:?}", fill(&art(&zero)[0])) };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert_eq!(g.geom, z.geom, "the same gradient as one with the highlight at the centre");
}

#[test]
fn an_undefined_number_anywhere_else_is_still_an_operator_the_reader_doesnt_know() {
    let err = read(stream(&layer(&format!("{RED}-- 5 m\n0 10 L\n10 10 L\nf\n"))).as_bytes()).unwrap_err();
    assert!(err.contains("operators VectorCraft doesn't know (`--`)"), "{err}");
}

#[test]
fn a_linked_images_colour_space_operator_is_ignored() {
    let art_ = format!("{RED}1 /DeviceGray XC\n{}f\n", rect_path(0.0, 0.0, 10.0, 10.0));
    assert_eq!(art(&read_ok(&layer(&art_)).doc).len(), 1);
}

/// Editing data past the old 256 MiB limit still opens on a 64-bit build (#3510): a card file with
/// embedded rasters unpacks to over 300 MiB.
#[cfg(target_pointer_width = "64")]
#[test]
fn editing_data_past_256_mib_is_read() {
    let mut body = stream(&layer("")).into_bytes();
    // Comment lines the reader skips, so the data stays valid while it grows past the old limit.
    let filler = vec![b'%'; (257 << 20) - body.len()];
    body.extend_from_slice(&filler);
    let raw = [&b"%AI12_CompressedData"[..], &crate::ps::deflate(&body)].concat();
    assert!(raw.len() < 1 << 20, "zeros compress");
    let data = decode_private(&raw).unwrap();
    assert_eq!(data.len(), body.len());
    assert_eq!(too_large(), format!("it decompresses to more than {} MB", max_data() >> 20));
}

/// The limit is an eighth of installed memory, never under the 512 MiB every 64-bit build read
/// before and never over 2 GiB (#3510).
#[cfg(target_pointer_width = "64")]
#[test]
fn the_editing_data_limit_follows_installed_memory() {
    const GIB: u64 = 1 << 30;
    assert_eq!(data_limit(None), 512 << 20, "unknown memory keeps the old limit");
    assert_eq!(data_limit(Some(2 * GIB)), 512 << 20);
    assert_eq!(data_limit(Some(4 * GIB)), 512 << 20);
    assert_eq!(data_limit(Some(8 * GIB)), GIB);
    assert_eq!(data_limit(Some(12 * GIB)), 3 * GIB / 2);
    assert_eq!(data_limit(Some(16 * GIB)), 2 * GIB);
    assert_eq!(data_limit(Some(256 * GIB)), 2 * GIB, "capped, so a tiny file can't take all of a big machine");
    assert!((MIN_DATA..=MAX_DATA).contains(&max_data()));
}

/// On a machine with 8 GiB or more, editing data past the old 512 MiB limit opens with its layers
/// (#3510, #4628): a layered file whose rasters unpack to 600 MiB.
#[cfg(target_pointer_width = "64")]
#[test]
fn editing_data_past_512_mib_is_read_where_memory_allows() {
    if max_data() < 601 << 20 {
        return;
    }
    let mut body = stream(&format!("{}{}", layer(&rect_path(0.0, 0.0, 10.0, 10.0)), layer(&rect_path(20.0, 20.0, 30.0, 30.0)))).into_bytes();
    let filler = vec![b'%'; (600 << 20) - body.len()];
    let at = body.windows(14).position(|w| w == b"%%PageTrailer\n").unwrap();
    body.splice(at..at, filler);
    let raw = [&b"%AI12_CompressedData"[..], &crate::ps::deflate(&body)].concat();
    let data = decode_private(&raw).unwrap();
    assert_eq!(data.len(), body.len());
    drop(body);
    let r = read(&data).unwrap();
    assert_eq!(r.doc.layers.len(), 2, "both layers are read");
}

/// Past the limit the data is still refused, so the file opens as its PDF part with a note.
#[test]
fn editing_data_past_the_limit_is_refused() {
    let raw = [&b"%AI12_CompressedData"[..], &crate::ps::deflate(b"%%")].concat();
    assert!(Codec::Zlib.decode(raw.get(20..).unwrap(), 1).is_err());
    assert!(Codec::Zlib.decode(raw.get(20..).unwrap(), 2).is_ok());
}

#[test]
fn many_zstandard_frames_are_read_in_one_pass_and_capped() {
    let frame = zstd(b" ");
    let frames = |n: usize| -> Vec<u8> { [&b"%AI24_ZStandard_Data"[..], &frame.repeat(n), &[0; 64]].concat() };
    let (few, many) = (frames(4000), frames(50_000));
    let before = padding_looks();
    assert_eq!(decode_private(&few).unwrap().len(), 4000);
    assert!(decode_private(&many).unwrap_err().contains("Zstandard"));
    // The first byte of each of the 4,000 frames and of the 4,097 read before the cap, and the
    // 64 zeros of padding once: looking past the frames to the padding before each frame would
    // look at 64 or more bytes per frame.
    let looked = padding_looks() - before;
    assert!(looked <= 2 * (4000 + 4097 + 64), "looked at {looked} bytes for padding");
}

/// A Zstandard frame that ends in a zero byte keeps it: only the zeros after the last frame are
/// padding.
#[test]
fn a_zstandard_frame_ending_in_a_zero_byte_is_read_whole() {
    // Magic number, a single-segment header with a 1-byte content size of 3, and one raw block
    // (last, 3 bytes) holding `ab` and a zero byte.
    let frame = [0x28, 0xB5, 0x2F, 0xFD, 0x20, 3, 0x19, 0, 0, b'a', b'b', 0];
    for padding in [0, 64] {
        let raw = [&b"%AI24_ZStandard_Data"[..], &frame, &frame, &vec![0; padding]].concat();
        assert_eq!(decode_private(&raw).unwrap(), b"ab\0ab\0");
    }
}

#[test]
fn closing_brackets_without_their_opening_dont_rescan_the_stack() {
    let n = 200_000;
    let body = format!("{}\n{}\n", "1 ".repeat(n), "] ".repeat(n));
    let started = std::time::Instant::now();
    let _ = read(stream(&layer(&body)).as_bytes());
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "{:?}", started.elapsed());
}

#[test]
fn artboards_after_what_stops_the_reading() {
    let board = |name: &str, x: i32| {
        format!(
            "%_/Dictionary :\n%_/FillStyle : 0 1 0 0 0 Xy\n%_/Def ; \n%_ (EachArtboardColor) ,\n%_{x} 100 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_({name}) /UnicodeString (Name) ,\n%_{} 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n",
            x + 100
        )
    };
    let array = format!("%_/Array :\n{}{}%_; (ArtboardArray) ,\n", board("one", 0), board("two", 200));
    // Art in the document's own data that the reader can't follow comes before the artboards.
    let data = format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n%_/Dictionary :\nLB\n{array}%_; /NotRecorded ,\n%_;\n");
    let boards: Vec<_> = super::artboards(data.as_bytes()).0.into_iter().map(|(n, r)| (n, r.x0)).collect();
    assert_eq!(boards, [("one".to_string(), 0.0), ("two".to_string(), 200.0)]);
}

#[test]
fn a_compound_clipping_path_with_a_fill_paints_once_under_its_clip() {
    // GUARDS: copies that painted a compound's clipping subpaths went into the compound itself,
    // doubling each subpath, so under even-odd the clip cancelled to nothing. The compound
    // clipping path keeps the fill itself.
    let art_ = format!(
        "0 Ae\nq\n{RED}{}f\n1 XR\n*u\n0 0 0 0 0 0 1 Xa\n{}W\nf\n{}W\nf\n*U\nQ\n",
        rect_path(0.0, 0.0, 100.0, 100.0),
        rect_path(10.0, 10.0, 40.0, 40.0),
        rect_path(50.0, 10.0, 80.0, 40.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    assert_eq!(children.len(), 2, "{children:#?}");
    let NodeKind::Compound { children: clip, rule } = &children[0].kind else { panic!("{:?}", children[0].kind) };
    assert_eq!(*rule, vectorcraft_geom::FillRule::EvenOdd);
    assert_eq!(clip.len(), 2);
    assert!(clip.iter().all(|c| matches!(c.kind, NodeKind::Path { clipping: true, .. })));
    assert_eq!(fill(&children[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(fill(&children[1]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

#[test]
fn a_radial_gradient_without_xm_is_its_bm_circle_else_its_bg_circle() {
    // GUARDS: files write `Bg` as the unit circle and place the radial with `Bm`; reading `Bg` alone collapses it.
    let defs =
        "%AI5_BeginGradient: (R)\n(R) 1 2 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 0 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let geom = |instance: &str| {
        let doc = read_ok(&format!("{defs}{}", layer(&format!("{}Bb\n10 0 0 0.5 Bh\n{instance}f\n0 BB\n", rect_path(0.0, 0.0, 200.0, 100.0))))).doc;
        let Paint::Gradient(g) = fill(&art(&doc)[0]) else { panic!() };
        g.geom.unwrap()
    };
    let placed = geom("1 (R) 0 0 0 1 1 0 0 1 0 0 Bg\n50 0 0 -50 100 50 Bm\n");
    assert_eq!((placed.start, placed.end, placed.focal), (Point::new(100.0, 50.0), Point::new(150.0, 50.0), Some(Point::new(110.0, 50.0))));
    let own = geom("1 (R) 60 50 0 20 1 0 0 1 0 0 Bg\n");
    assert_eq!((own.start, own.end, own.focal), (Point::new(60.0, 50.0), Point::new(80.0, 50.0), Some(Point::new(70.0, 50.0))));
}

/// Two art styles: `Blue`, a blue fill and a drop shadow; `Lit`, the object's gradient and an outer glow.
const ART_STYLES: &str = "%AI9_BeginArtStyles\n\
    /KnownStyle :\n(Blue) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
    /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n\
    /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n1 /Int (blnd) ,\n0.75 /Real (opac) ,\n5 /Real (blur) ,\n\
    0 /Int (csrc) ,\n/FillStyle : 0 O\n0 0 0 1 0 0 0 Xa\n0 1 Xd\n/Def ;\n (sclr) ,\n3 /Real (horz) ,\n4 /Real (vert) ,\n; /Dict ;\n /Part ,\n;\n\
    /KnownStyle :\n(Lit) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n1 (G) 0 0 0 1 1 0 0 1 0 0 Bg\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
    /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n\
    /BasicFilter :\n(Adobe Outer Glow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n2 /Int (blnd) ,\n0.5 /Real (opac) ,\n2 /Real (blur) ,\n\
    /FillStyle : 0 O\n0 0 0 0 1 1 0 Xa\n0 1 Xd\n/Def ;\n (sclr) ,\n; /Dict ;\n /Part ,\n;\n\
    %AI9_EndArtStyles\n";

#[test]
fn an_object_with_a_style_this_reads_comes_in_as_itself() {
    let defs =
        "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n<00FF>\n0\n%_0 0 0 0 1 0 0 2 1 6 50 0 Bs\n%_0 0 0 0 0 0 1 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!(
        "0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_1 (G) 0 0 0 1 1 0 0 1 0 0 Bg\n%_10 0 0 10 5 5 Xm\n%_{hidden_rect}f\n%_0 BB\n1 (Lit) XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0)
    );
    let r = read_ok(&format!("{ART_STYLES}{defs}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
    assert!(matches!(fill(&a[0]), Paint::Gradient(g) if g.swatch.as_deref() == Some("G")));
    let effects: Vec<_> = a[0].appearance.effects.iter().map(|e| (e.id.as_str(), e.params["opacity"].as_f64())).collect();
    assert_eq!(effects, [("stylize.outerGlow", Some(50.0))]);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// An art style named `name`: a blue fill under a red 2 pt stroke written as a Stroke Offset filter
/// whose `StrokeOffsetInside` entry is `inside` (`None`: left out).
fn offset_stroke_style(name: &str, inside: Option<u8>) -> String {
    let switch = inside.map(|v| format!("{v} /Bool (StrokeOffsetInside) ,\n")).unwrap_or_default();
    offset_stroke_style_with(name, &switch, OFFSET_STROKE)
}

/// The Stroke Offset stroke of [`offset_stroke_style`]: red, 2 pt, butt caps, miter joins, solid.
const OFFSET_STROKE: &str = "0 R\n0 1 1 0 1 0 0 XA\n0 J 0 j 2 w 10 M []0 d\n0 XR\n1 0 Xd";

/// An art style named `name`: a blue fill under a Stroke Offset filter whose dictionary holds
/// `entries` and the stroke style `stroke` (what follows `/StrokeStyle :`).
fn offset_stroke_style_with(name: &str, entries: &str, stroke: &str) -> String {
    format!(
        "/KnownStyle :\n({name}) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Adobe Stroke Offset) 1 0 /Filter ,\n1 /Visible ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n{entries}(Side) /String (DisplayString) ,\n\
         /StrokeStyle : {stroke}\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n;\n /Part ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n;\n"
    )
}

#[test]
fn a_stroke_offset_filter_is_a_stroke_aligned_inside_or_outside() {
    // GUARDS: a style's stroke aligned inside or outside (Stroke panel › Align Stroke) left the object as its drawn look.
    use vectorcraft_doc::StrokeAlign;
    for (inside, align) in [(Some(1), Some(StrokeAlign::Inside)), (Some(0), Some(StrokeAlign::Outside)), (None, None), (Some(2), None)] {
        let r = read_ok(&format!("%AI9_BeginArtStyles\n{}%AI9_EndArtStyles\n{}", offset_stroke_style("Edge", inside), layer(&styled_square("Edge"))));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{a:#?}");
        let stroke = a[0].appearance.stroke().map(|s| (s.align, s.width, s.paint.color()));
        match align {
            Some(align) => {
                assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
                assert_eq!(stroke, Some((align, 2.0, Some(Color::rgb(1.0, 0.0, 0.0)))));
                assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
                assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
            }
            // Without a 0/1 switch, which side it paints isn't known: the drawn look stays.
            None => assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{inside:?}: {:?}", r.warnings),
        }
    }
}

/// A Stroke Offset stroke keeps the caps, joins, miter limit, dashes and width profile of its
/// style, as a Stroke Style Filter's does.
#[test]
fn a_stroke_offset_filter_keeps_its_strokes_style() {
    use vectorcraft_doc::StrokeAlign;
    let profile = "/Dictionary : /NotRecorded ,\n/Array :\n0 /Real ,\n1 /Real ,\n; (BSVariableWidthTValuesArray) ,\n\
                   /Array :\n1 /Real ,\n0.1 /Real ,\n; (BSCenteredWidthsArray) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n";
    let entries = format!("0 /Bool (StrokeOffsetInside) ,\n{profile}");
    let stroke = "0 R\n0 1 1 0 1 0 0 XA\n1 J 2 j 3 w 4 M [5 10 ]0 d\n0 XR\n1 0 Xd";
    let style = offset_stroke_style_with("Edge", &entries, stroke);
    let r = read_ok(&format!("%AI9_BeginArtStyles\n{style}%AI9_EndArtStyles\n{}", layer(&styled_square("Edge"))));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
    let s = a[0].appearance.stroke().expect("a stroke");
    assert_eq!((s.align, s.width, s.cap, s.join, s.miter_limit), (StrokeAlign::Outside, 3.0, LineCap::Round, LineJoin::Bevel, 4.0));
    assert_eq!(s.dash.as_ref().map(|d| d.pattern.clone()), Some(vec![5.0, 10.0]));
    assert_eq!(s.profile.as_ref().map(|p| p.points.clone()), Some(vec![(0.0, 1.0, 1.0), (1.0, 0.1, 0.1)]));
}

/// A Stroke Offset stroke this doesn't read as a stroke keeps the object's drawn look: a switch
/// written as another kind, dashes aligned to corners and path ends, a gradient.
#[test]
fn a_stroke_offset_filter_with_a_stroke_this_does_not_read_keeps_the_drawn_look() {
    let outside = "0 /Bool (StrokeOffsetInside) ,\n";
    let gradient = "0 R\nBb\n0 0 0 0 Bh\n2 (Red ramp) 0 0 90 1 1 0 0 1 0 0 1 Bg\n0 BB\n0 J 0 j 2 w 10 M []0 d\n0 XR\n1 0 Xd";
    let cases = [
        ("a switch written as a number", offset_stroke_style_with("Edge", "1 /Int (StrokeOffsetInside) ,\n", OFFSET_STROKE)),
        (
            "dashes aligned to corners",
            offset_stroke_style_with(
                "Edge",
                &format!("{outside}/Dictionary : /NotRecorded ,\n1 /Bool (BSDashAdj) ,\n; (kAIBeautifulStrokesParamsDictKey) ,\n"),
                &OFFSET_STROKE.replace("[]0 d", "[5 10 ]0 d"),
            ),
        ),
        ("a gradient", offset_stroke_style_with("Edge", outside, gradient)),
    ];
    for (why, style) in cases {
        let r = read_ok(&format!("%AI9_BeginArtStyles\n{style}%AI9_EndArtStyles\n{}", layer(&styled_square("Edge"))));
        assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{why}: {:?}", r.warnings);
    }
}

/// A listed `[Default]` style whose stroke is aligned outside comes in as the default graphic
/// style with that stroke.
#[test]
fn a_default_style_with_a_stroke_aligned_outside_comes_in() {
    use vectorcraft_doc::StrokeAlign;
    let r =
        read_ok(&format!("{}{}", styles_and_list(&[offset_stroke_style("[Default]", Some(0))], &["[Default]"]), layer(&styled_square("[Default]"))));
    assert_eq!(style_names(&r.doc), [("[Default]".to_string(), 1)]);
    let stroke = r.doc.graphic_styles[0].appearance.stroke().map(|s| (s.align, s.width));
    assert_eq!(stroke, Some((StrokeAlign::Outside, 2.0)));
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
    assert_eq!(a[0].extras().graphic_style, Some(1));
}

#[test]
fn a_symbol_defined_before_the_art_styles_reads_its_styled_objects() {
    // GUARDS: objects in a symbol defined ahead of `%AI9_BeginArtStyles` (as Illustrator 24 writes
    // its editing data) found no style and came in as their drawn look.
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let styled = format!("u\n{RED}{}f\nU\n%_{hidden_rect}f\n1 (Blue) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{}{ART_STYLES}{}", symbols(&symbol("Tag", &styled, 1)), layer("")));
    let art = &r.doc.symbols.first().expect("the symbol").art;
    let object = match &art.kind {
        NodeKind::Group { children, .. } if children.len() == 1 => &children[0],
        _ => art,
    };
    assert!(matches!(object.kind, NodeKind::Path { .. }), "the object, not its look: {art:#?}");
    assert_eq!(object.appearance.fill().and_then(|f| f.paint.color()), Some(Color::rgb(0.0, 0.0, 1.0)));
    let effects: Vec<_> = object.appearance.effects.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(effects, ["stylize.dropShadow"]);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// `node` is the red square of [`styled_square`] read as itself with [`ART_STYLES`]' `Blue`: a blue
/// fill and a drop shadow, not the look it draws.
fn assert_blue_square(node: &Node) {
    assert!(matches!(node.kind, NodeKind::Path { .. }), "the object, not its look: {node:#?}");
    assert_eq!(fill(node).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    let effects: Vec<_> = node.appearance.effects.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(effects, ["stylize.dropShadow"]);
}

/// An object in a pattern's art with an art style the file defines comes in as itself, as in a
/// symbol's art, whichever section comes first.
#[test]
fn an_object_in_a_patterns_art_with_a_style_this_reads_comes_in_as_itself() {
    let pattern = format!("%AI3_BeginPattern: (P)\n(P) 0 0 10 10\n0 A\n0 Xw\n{}E\n%AI3_EndPattern\n", styled_square("Blue"));
    for body in [format!("{ART_STYLES}{pattern}{}", layer("")), format!("{pattern}{ART_STYLES}{}", layer(""))] {
        let r = read_ok(&body);
        let def = r.doc.pattern("P").unwrap_or_else(|| panic!("{:?} {:?}", r.unread, r.warnings));
        let [square] = def.art.as_slice() else { panic!("{:#?}", def.art) };
        assert_blue_square(square);
        assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    }
}

/// The same in a pattern defined in the version 7 form: a layer's tile string holding the object.
#[test]
fn an_object_in_a_version_7_patterns_tile_with_a_style_this_reads_comes_in_as_itself() {
    let tile = styled_square("Blue").replace('\n', "\r");
    let layers = format!("(0 O 0 R 0 0 0 1 k 0 0 0 1 K) @\n(\r{tile}) &\n");
    let s = pattern_read7(&format!("{ART_STYLES}{}", pattern7("Grid", &layers)), &pattern_layer7("Art", ""));
    let def = s.doc.pattern("Grid").unwrap_or_else(|| panic!("{:?} {:?}", s.unread, s.warnings));
    let [square] = def.art.as_slice() else { panic!("{:#?}", def.art) };
    assert_blue_square(square);
}

/// An object in a brush's art with an art style the file defines comes in as itself, and the brush
/// that uses the art is kept.
#[test]
fn an_object_in_a_brushs_art_with_a_style_this_reads_comes_in_as_itself() {
    let section = format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n{}%AI8_EndBrushPattern\n", styled_square("Blue"));
    let r = read_ok(&format!("{section}{ART_STYLES}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    let spray = lib.iter().find(|b| b.name == "Spray").unwrap_or_else(|| panic!("{:?}", r.warnings));
    assert!(matches!(spray.kind, vectorcraft_brush::BrushKind::Scatter(_)), "{:?}", spray.kind);
    let art = spray.art();
    let [square] = art.as_slice() else { panic!("{art:#?}") };
    assert_blue_square(square);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// An art styles section inside a symbol's, a pattern's or a brush's art is skipped: the file's own
/// art styles still style the layers' objects read after the definition.
#[test]
fn art_styles_inside_a_definition_leave_the_files_art_styles_alone() {
    let nested = format!("%AI9_BeginArtStyles\n{}%AI9_EndArtStyles\n", fill_style("Other", "0 0 0 0 1 0 0 Xa\n"));
    let square = styled_square("Blue");
    let definitions = [
        symbols(&symbol("Tag", &format!("{nested}{square}"), 1)),
        format!("%AI3_BeginPattern: (P)\n(P) 0 0 10 10\n0 A\n0 Xw\n{nested}{square}E\n%AI3_EndPattern\n"),
        format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n{nested}{square}%AI8_EndBrushPattern\n"),
    ];
    for definition in definitions {
        let r = read_ok(&format!("{ART_STYLES}{definition}{}", layer(&square)));
        let a = art(&r.doc);
        let [object] = a.as_slice() else { panic!("{definition}: {a:#?}") };
        assert_blue_square(object);
    }
}

/// An art style named `name` (its bytes as written) that fills with colour `xa` (`c m y k r g b Xa`).
fn fill_style(name: &str, xa: &str) -> String {
    fill_style_with(name, xa, "")
}

/// [`fill_style`] with the filters `extra` after its fills.
fn fill_style_with(name: &str, xa: &str, extra: &str) -> String {
    format!(
        "/KnownStyle :\n({name}) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n{xa}0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n{extra};\n"
    )
}

/// An art style named `name` with a drop shadow (every setting, Darkness) and no fills or strokes.
fn shadow_style(name: &str) -> String {
    format!(
        "/KnownStyle :\n({name}) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
         /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n2 /Int (blnd) ,\n0.75 /Real (opac) ,\n3 /Real (horz) ,\n4 /Real (vert) ,\n5 /Real (blur) ,\n1 /Int (csrc) ,\n50 /Real (dark) ,\n; /Dict ;\n /Part ,\n;\n"
    )
}

/// The art styles `defs` and the style list of `listed` (each written between parentheses as given).
fn styles_and_list(defs: &[String], listed: &[&str]) -> String {
    let list: String = listed.iter().map(|n| format!("({n})\n")).collect();
    format!("%AI9_BeginArtStyles\n{}%AI9_EndArtStyles\n%AI9_BeginArtStyleList\n{list}%AI9_EndArtStyleList\n", defs.concat())
}

/// A red square with its drawn look, read as itself with the art style `name` (`1 (name) XW`).
fn styled_square(name: &str) -> String {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    format!("0 Ae\nu\n{RED}{}f\nU\n%_{hidden_rect}f\n1 ({name}) XW\n", rect_path(0.0, 0.0, 10.0, 10.0))
}

fn style_names(doc: &Document) -> Vec<(String, u32)> {
    doc.graphic_styles.iter().map(|g| (g.name.clone(), g.id)).collect()
}

#[test]
fn listed_styles_replace_the_default_graphic_styles_and_link_their_objects() {
    let defs = [fill_style("Red", "0 0 0 0 1 0 0 Xa\n"), fill_style("Blue", "0 0 0 0 0 0 1 Xa\n"), shadow_style("Shadow")];
    let r = read_ok(&format!("{}{}", styles_and_list(&defs, &["Red", "Blue", "Shadow"]), layer(&styled_square("Blue"))));
    assert_eq!(style_names(&r.doc), [("Red".to_string(), 1), ("Blue".to_string(), 2), ("Shadow".to_string(), 3)]);
    let blue = &r.doc.graphic_styles[1];
    assert_eq!(blue.appearance.fill().and_then(|f| f.paint.color()), Some(Color::rgb(0.0, 0.0, 1.0)));
    let shadow = &r.doc.graphic_styles[2];
    assert_eq!(shadow.appearance.effects.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["stylize.dropShadow"]);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
    assert_eq!(a[0].extras().graphic_style, Some(2));
}

#[test]
fn only_listed_styles_that_are_defined_and_read_come_in_once() {
    // `Lit` paints with a gradient placed on each object; `Missing` has no definition.
    let defs = [fill_style("Blue", "0 0 0 0 0 0 1 Xa\n"), ART_STYLES.lines().filter(|l| !l.starts_with('%')).map(|l| format!("{l}\n")).collect()];
    let r = read_ok(&format!("{}{}", styles_and_list(&defs, &["Blue", "Missing", "", "Lit", "Blue"]), layer("")));
    assert_eq!(style_names(&r.doc), [("Blue".to_string(), 1)]);
}

#[test]
fn a_style_the_list_does_not_have_is_no_link() {
    let defs = [fill_style("Blue", "0 0 0 0 0 0 1 Xa\n"), fill_style("Anon", "0 0 0 0 0 1 0 Xa\n")];
    let r = read_ok(&format!("{}{}", styles_and_list(&defs, &["Blue"]), layer(&styled_square("Anon"))));
    assert_eq!(style_names(&r.doc), [("Blue".to_string(), 1)]);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 1.0, 0.0)), "the object still takes its appearance");
    assert_eq!(a[0].extras().graphic_style, None);
}

#[test]
fn a_group_with_a_listed_style_is_linked() {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let group = format!("0 Ae\nu\n{RED}{}f\nU\n%_0 Ae\n%_u\n%_{hidden_rect}f\n%_U\n1 (Shadow) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{}{}", styles_and_list(&[shadow_style("Shadow")], &["Shadow"]), layer(&group)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert_eq!(a[0].extras().graphic_style, Some(1));
}

#[test]
fn an_object_whose_transparency_differs_from_its_listed_style_is_not_linked() {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let square =
        format!("0 Ae\nu\n{RED}{}f\nU\n1 0.6 0 2 0 Xy\n6 () XW\n%_0 1 0 0 0 Xy\n%_{hidden_rect}f\n1 (Blue) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{}{}", styles_and_list(&[fill_style("Blue", "0 0 0 0 0 0 1 Xa\n")], &["Blue"]), layer(&square)));
    assert_eq!(style_names(&r.doc), [("Blue".to_string(), 1)]);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(a[0].opacity, 0.6);
    assert_eq!(a[0].extras().graphic_style, None, "the style has full opacity");
}

#[test]
fn a_listed_style_keeps_its_transparency_and_links_objects_that_have_it() {
    let blend = "/BasicFilter :\n(Blend Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n\
                 /BlendStyle : 1 0.6 1 1 0 Xy\n0 J 0 j 1 w 10 M []0 d0 XR\n/Def ;\n (BlendStyle) ,\n; /Dict ;\n /Part ,\n";
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let square =
        format!("0 Ae\nu\n{RED}{}f\nU\n1 0.6 1 1 0 Xy\n6 () XW\n%_0 1 0 0 0 Xy\n%_{hidden_rect}f\n1 (Blue) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{}{}", styles_and_list(&[fill_style_with("Blue", "0 0 0 0 0 0 1 Xa\n", blend)], &["Blue"]), layer(&square)));
    let g = &r.doc.graphic_styles[0];
    assert_eq!((g.name.as_str(), g.opacity, g.blend, g.isolate, g.knockout), ("Blue", 0.6, BlendMode::Multiply, true, Knockout::On));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(a[0].extras().graphic_style, Some(1), "{a:#?}");
}

#[test]
fn a_path_with_a_listed_style_that_fills_below_the_contents_is_linked() {
    let below = "/KnownStyle :\n(Under) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
                 /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
                 /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
                 /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
                 ;\n /Part ,\n;\n";
    let r = read_ok(&format!("{}{}", styles_and_list(&[below.to_string()], &["Under"]), layer(&styled_square("Under"))));
    assert_eq!(r.doc.graphic_styles[0].appearance.contents_index, Some(1));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(a[0].extras().graphic_style, Some(1), "{a:#?}");
}

#[test]
fn a_style_name_outside_ascii_links_its_objects() {
    // The list writes the name with escapes; the object's marker writes its UTF-8 bytes.
    let defs = [fill_style("Caf\u{e9} \u{3a9}", "0 0 0 0 0 0 1 Xa\n")];
    let r = read_ok(&format!("{}{}", styles_and_list(&defs, &["Caf\\303\\251 \\316\\251"]), layer(&styled_square("Caf\u{e9} \u{3a9}"))));
    assert_eq!(style_names(&r.doc), [("Caf\u{e9} \u{3a9}".to_string(), 1)]);
    assert_eq!(art(&r.doc)[0].extras().graphic_style, Some(1));
}

#[test]
fn an_object_in_a_symbols_art_with_a_listed_style_is_linked() {
    let defs = styles_and_list(&[fill_style("Blue", "0 0 0 0 0 0 1 Xa\n")], &["Blue"]);
    let body =
        format!("{defs}{}{}", symbols(&symbol("Tile", &styled_square("Blue"), 5)), layer(&instance("Tile", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])));
    let r = read_ok(&body);
    assert_eq!(style_names(&r.doc), [("Blue".to_string(), 1)]);
    let tile = &r.doc.symbols[0].art;
    assert!(matches!(tile.kind, NodeKind::Path { .. }), "{tile:#?}");
    assert_eq!(fill(tile).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(tile.extras().graphic_style, Some(1), "{tile:#?}");
    // A symbol's art written as a group holding one object stays that group when the group is
    // linked to the style.
    let empty = "/KnownStyle :\n(Empty) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
                 /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n";
    let defs = styles_and_list(&[empty.to_string()], &["Empty"]);
    let square = rect_path(0.0, 0.0, 10.0, 10.0);
    let hidden = square.replace('\n', "\n%_");
    // The styled group is the definition's own group, without one around it.
    let group = format!(
        "%AI14_BeginSymbol\n(Group)\n0 A\n0 Xw\n0 Ae\nu\n{RED}{square}f\nU\n%_u\n%_{hidden}f\n%_U\n1 (Empty) XW\n\
         %_/ArtDictionary :\n%_5 /Int (AI13PatternRegistrationType) ,\n%_;\n%_\n%AI10_EndSymbol\n"
    );
    let body = format!("{defs}{}{}", symbols(&group), layer(&instance("Group", [1.0, 0.0, 0.0, -1.0, 20.0, 50.0])));
    let art = &read_ok(&body).doc.symbols[0].art;
    assert!(matches!(art.kind, NodeKind::Group { .. }), "{art:#?}");
    assert_eq!(art.extras().graphic_style, Some(1), "{art:#?}");
}

#[test]
fn an_object_in_a_patterns_art_with_a_listed_style_is_linked() {
    let defs = styles_and_list(&[fill_style("Blue", "0 0 0 0 0 0 1 Xa\n")], &["Blue"]);
    let pattern = format!("%AI3_BeginPattern: (P)\n(P) 0 0 10 10\n0 A\n0 Xw\n{}E\n%AI3_EndPattern\n", styled_square("Blue"));
    let r = read_ok(&format!("{defs}{pattern}{}", layer("")));
    assert_eq!(style_names(&r.doc), [("Blue".to_string(), 1)]);
    let def = r.doc.pattern("P").unwrap_or_else(|| panic!("{:?}", r.warnings));
    let [square] = def.art.as_slice() else { panic!("{:#?}", def.art) };
    assert_eq!(fill(square).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(square.extras().graphic_style, Some(1), "{square:#?}");
}

#[test]
fn an_object_in_a_version_7_patterns_tile_with_a_listed_style_is_linked() {
    let defs = styles_and_list(&[fill_style("Blue", "0 0 0 0 0 0 1 Xa\n")], &["Blue"]);
    let tile = styled_square("Blue").replace('\n', "\r");
    let layers = format!("(0 O 0 R 0 0 0 1 k 0 0 0 1 K) @\n(\r{tile}) &\n");
    let s = pattern_read7(&format!("{defs}{}", pattern7("Grid", &layers)), &pattern_layer7("Art", ""));
    assert_eq!(style_names(&s.doc), [("Blue".to_string(), 1)]);
    let def = s.doc.pattern("Grid").unwrap_or_else(|| panic!("{:?}", s.warnings));
    let [square] = def.art.as_slice() else { panic!("{:#?}", def.art) };
    assert_eq!(fill(square).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(square.extras().graphic_style, Some(1), "{square:#?}");
}

#[test]
fn without_a_style_list_the_default_graphic_styles_stay() {
    let defaults = style_names(&Document::new(1.0, 1.0));
    let r = read_ok(&format!("{ART_STYLES}{}", layer(&styled_square("Blue"))));
    assert_eq!(style_names(&r.doc), defaults);
    assert_eq!(art(&r.doc)[0].extras().graphic_style, None);
    let r = read_ok(&format!("{}{}", styles_and_list(&[shadow_style("Shadow")], &["Missing"]), layer("")));
    assert_eq!(style_names(&r.doc), defaults, "a list naming no style it can read");
}

#[test]
fn an_object_with_a_feather_style_comes_in_with_a_live_effect() {
    let feather_style = "%AI9_BeginArtStyles\n\
        /KnownStyle :\n(Soft) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
        /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n\
        /BasicFilter :\n(Adobe Fuzzy Mask) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n8 /Real (Radius) ,\n; /Dict ;\n /Part ,\n;\n\
        %AI9_EndArtStyles\n";
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{hidden_rect}f\n%_0 BB\n1 (Soft) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{feather_style}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1);
    let [feather] = a[0].appearance.effects.as_slice() else { panic!("{:#?}", a[0].appearance) };
    assert_eq!((feather.id.as_str(), feather.params["radius"].as_f64()), ("stylize.feather", Some(8.0)));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// An art style section with style `PF`: an Effect › Pathfinder filter with `Command` `command`
/// and `Mix` `mix` in the object's chain, over a fill and stroke that paint nothing.
fn pathfinder_styles(command: u32, mix: f64) -> String {
    format!(
        "%AI9_BeginArtStyles\n/KnownStyle :\n(PF) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Adobe Pathfinder) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n\
         0 /Bool (RemovePoints) ,\n{mix} /Real (Mix) ,\n1 /Bool (ConvertCustom) ,\n{command} /Int (Command) ,\n1 /Real (TrapMaxTint) ,\n\
         1 /Real (TrapAspect) ,\n0.4 /Real (TrapTint) ,\n0.25 /Real (TrapThickness) ,\n0 /Bool (TrapReverse) ,\n10 /Real (Precision) ,\n\
         (Op) /UnicodeString (DisplayString) ,\n0 /Bool (TrapConvertCustom) ,\n0.05 /Real (TrapTintTolerance) ,\n1 /Bool (ExtractUnpainted) ,\n\
         ; /Dict ;\n /Part ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 1 0 0 0 Xy\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 1 0 0 0 Xy\n0 0 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         ;\n /Part ,\n;\n%AI9_EndArtStyles\n"
    )
}

/// A group of a red and a blue square with style `PF`: its drawn look (one path of their area),
/// then the group on `%_` lines.
fn pathfinder_group() -> String {
    let hidden = |color: &str, x: f64| format!("{color}{}f\n", rect_path(x, 0.0, x + 10.0, 10.0)).replace('\n', "\n%_");
    format!(
        "0 Ae\nu\n{RED}{}f\nU\n%_0 Ae\n%_u\n%_{}{}U\n1 (PF) XW\n",
        rect_path(0.0, 0.0, 15.0, 10.0),
        hidden(RED, 0.0),
        hidden("0 0 1 0 0 0 1 Xa\n", 5.0)
    )
}

#[test]
fn a_group_with_a_pathfinder_effect_keeps_its_members_and_the_effect() {
    // GUARDS: the style was not read, and the group came in as its drawn look: one path, with no effect.
    let r = read_ok(&format!("{}{}", pathfinder_styles(0, 0.5), layer(&pathfinder_group())));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let members = a[0].children().cloned().unwrap_or_default();
    assert_eq!(members.len(), 2, "the squares, not the look: {a:#?}");
    assert!(members.iter().all(|m| matches!(m.kind, NodeKind::Path { .. })), "{members:#?}");
    assert_eq!((fill(&members[0]).color(), fill(&members[1]).color()), (Some(Color::rgb(1.0, 0.0, 0.0)), Some(Color::rgb(0.0, 0.0, 1.0))));
    let effects: Vec<_> = a[0].appearance.effects.iter().map(|e| (e.id.as_str(), e.visible)).collect();
    assert_eq!(effects, [("pathfinder.add", true)]);
    assert!(a[0].appearance.items.is_empty(), "{:?}", a[0].appearance);
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

#[test]
fn each_pathfinder_operation_on_a_group_draws_as_vectorcraft_draws_it() {
    // GUARDS: only Add (`Command` 0) was read; the other operations came in as the drawn look's one path.
    let ops = [
        (1, "pathfinder.intersect"),
        (2, "pathfinder.exclude"),
        (3, "pathfinder.subtract"),
        (4, "pathfinder.minusBack"),
        (5, "pathfinder.divide"),
        (6, "pathfinder.outline"),
        (7, "pathfinder.trim"),
        (8, "pathfinder.merge"),
        (9, "pathfinder.crop"),
        (10, "pathfinder.hardMix"),
        (11, "pathfinder.softMix"),
    ];
    for (command, id) in ops {
        let mix = if command == 11 { 0.3 } else { 0.5 };
        let r = read_ok(&format!("{}{}", pathfinder_styles(command, mix), layer(&pathfinder_group())));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{command}: {a:#?}");
        let members = a[0].children().cloned().unwrap_or_default();
        assert_eq!(members.len(), 2, "{command}: the squares, not the look: {a:#?}");
        let [e] = a[0].appearance.effects.as_slice() else { panic!("{command}: {:?}", a[0].appearance) };
        assert_eq!((e.id.as_str(), e.visible), (id, true), "{command}");
        let params = if command == 11 { serde_json::json!({"mixingRate": 30.0}) } else { serde_json::json!({}) };
        assert_eq!(e.params, params, "{command}");
        assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{command}: {:?}", r.warnings);
        // The same squares in a group made in VectorCraft, with the effect of that operation.
        let square = |m: &Arc<Node>| {
            let bounds = m.geometric_bounds().unwrap();
            Arc::new(Node::path(m.id, vectorcraft_geom::shapes::rectangle(bounds), vectorcraft_doc::Appearance::basic(fill(m), Paint::None, 0.0)))
        };
        let mut own = Node::group(a[0].id, members.iter().map(square).collect());
        own.appearance.effects.push(vectorcraft_doc::Effect { id: id.into(), params: params.clone(), visible: true });
        // Each result's box and paints; Outline's lines start where each square's path starts, so
        // the results are compared in a sorted order.
        let faces = |g: &Node| -> Vec<String> {
            let mut faces: Vec<String> = vectorcraft_effects::pathfinder_children(g, None)
                .unwrap()
                .iter()
                .map(|n| {
                    let b = n.geometric_bounds().unwrap_or_default();
                    let round = |v: f64| (v * 1000.0).round() / 1000.0;
                    let paints = (n.appearance.fill_paint().color(), n.appearance.stroke_paint().color());
                    format!("{:?} {paints:?}", Rect::new(round(b.x0), round(b.y0), round(b.x1), round(b.y1)))
                })
                .collect();
            faces.sort();
            faces
        };
        let (opened, made) = (faces(&a[0]), faces(&own));
        assert!(!made.is_empty(), "{command}");
        assert_eq!(opened, made, "{command}");
    }
}

#[test]
fn a_pathfinder_style_this_does_not_read_keeps_the_drawn_look() {
    // Trap, and a Precision other than the one read.
    let other = pathfinder_styles(0, 0.5).replace("10 /Real (Precision)", "5 /Real (Precision)");
    let intersect = pathfinder_styles(1, 0.5).replace("10 /Real (Precision)", "5 /Real (Precision)");
    for styles in [pathfinder_styles(12, 0.5), other, intersect] {
        let r = read_ok(&format!("{styles}{}", layer(&pathfinder_group())));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{a:#?}");
        let look = a[0].children().cloned().unwrap_or_default();
        assert_eq!(look.len(), 1, "the look's one path: {a:#?}");
        assert!(a[0].appearance.effects.is_empty(), "{:?}", a[0].appearance);
    }
}

#[test]
fn a_path_with_a_pathfinder_effect_keeps_its_drawn_look() {
    // Effect › Pathfinder doesn't apply to a lone path: its style stays the drawn look's.
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_{hidden_rect}f\n1 (PF) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{}{}", pathfinder_styles(0, 0.5), layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "the look, not the path: {a:#?}");
    let mut effects = 0;
    r.doc.walk(|n| effects += n.appearance.effects.len());
    assert_eq!(effects, 0);
}

#[test]
fn an_object_with_bas_relief_comes_in_with_its_settings() {
    // GUARDS: Bas Relief's settings block wasn't read, so the object came in as its drawn look.
    let settings: Vec<u8> = [
        &[0, 0, 0, 1][..],
        b"Dtl ",
        &[0; 4],
        b"long",
        &5i32.to_be_bytes(),
        b"Smth",
        &[0; 4],
        b"long",
        &9i32.to_be_bytes(),
        b"LghD",
        &0x2000u32.to_be_bytes(),
        b"LghD",
        b"LDTR",
    ]
    .concat();
    let lines: String = crate::ps::ascii85(&settings).lines().map(|l| format!("%{l}\n")).collect();
    let style = format!(
        "%AI9_BeginArtStyles\n\
        /KnownStyle :\n(Carved) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
        /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
        /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n\
        /BasicFilter :\n(PSAdapter_plugin_BsRl) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/Binary : /ASCII85Decode ,\n{lines}; (data) ,\n\
        {} /Int (dataSize) ,\n; /Dict ;\n /Part ,\n;\n\
        %AI9_EndArtStyles\n",
        settings.len()
    );
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_0 0 0 1 k\n%_{hidden_rect}f\n1 (Carved) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{style}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1);
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look");
    let [relief] = a[0].appearance.effects.as_slice() else { panic!("{:#?}", a[0].appearance) };
    assert_eq!(
        (relief.id.as_str(), &relief.params),
        ("sketch.basRelief", &serde_json::json!({"detail": 5.0, "smoothness": 9.0, "lightDirection": "topRight"}))
    );
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

#[test]
fn an_object_with_convert_to_shape_comes_in_as_itself_with_the_effect_live() {
    // GUARDS: `Adobe Shape Effects` wasn't read, so the object came in as its drawn look.
    let styles = |keys: &str| {
        format!(
            "%AI9_BeginArtStyles\n\
            /KnownStyle :\n(Boxed) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Shape Effects) 1 0 /Filter ,\n1 /Visible ,\n(ShapeEffect.aip) /PluginFileName ,\n(Shape) /Title ,\n\
            /Dictionary : /NotRecorded ,\n(Rectangle) /UnicodeString (DisplayString) ,\n{keys}; /Dict ;\n /Part ,\n\
            /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n;\n\
            %AI9_EndArtStyles\n"
        )
    };
    let hidden_rect = rect_path(20.0, 20.0, 90.0, 60.0).replace('\n', "\n%_");
    // The drawn look is the shape: 20 pt wider on each side and 6 pt taller.
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{hidden_rect}f\n%_0 BB\n1 (Boxed) XW\n", rect_path(0.0, 14.0, 110.0, 66.0));
    let keys = "0 /Int (Shape) ,\n150 /Real (AbsWidth) ,\n0 /Real (Absolute) ,\n40 /Real (AbsHeight) ,\n20 /Real (RelWidth) ,\n15 /Real (CornerRadius) ,\n6 /Real (RelHeight) ,\n";
    let r = read_ok(&format!("{}{}", styles(keys), layer(&art_)));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look: {a:#?}");
    let b = a[0].geometric_bounds().unwrap();
    assert_eq!((b.width(), b.height()), (70.0, 40.0), "the object keeps its own path");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    let [e] = a[0].appearance.effects.as_slice() else { panic!("{:?}", a[0].appearance.effects) };
    assert_eq!(
        (e.id.as_str(), e.params["relative"].as_bool(), e.params["extraW"].as_f64(), e.params["extraH"].as_f64()),
        ("convertToShape.rectangle", Some(true), Some(40.0), Some(12.0))
    );
    // A shape that isn't read: the drawn look.
    let r = read_ok(&format!("{}{}", styles(&keys.replace("0 /Int (Shape)", "7 /Int (Shape)")), layer(&art_)));
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

#[test]
fn an_object_with_path_effects_comes_in_as_itself_with_them_live() {
    // GUARDS: an object with Offset Path, Outline Object or Outline Stroke came in as its drawn look.
    let styles = |offset: &str| {
        format!(
            "%AI9_BeginArtStyles\n\
            /KnownStyle :\n(Rim) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Adobe Offset Path) 1 0 /Filter ,\n1 /Visible ,\n(OffsetPath.aip) /PluginFileName ,\n(Offset Path) /Title ,\n\
            /Dictionary : /NotRecorded ,\n{offset}; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Adobe Outline Type) 1 0 /Filter ,\n0 /Visible ,\n(ArtConverters.aip) /PluginFileName ,\n(Outline Object) /Title ,\n\
            /Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Adobe Outline Stroke) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n; /Dict ;\n /Part ,\n\
            /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
            /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n0 0 0 0 0 0 1 Xa\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
            /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n;\n\
            %AI9_EndArtStyles\n"
        )
    };
    let hidden_rect = rect_path(20.0, 20.0, 60.0, 50.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{hidden_rect}f\n%_0 BB\n1 (Rim) XW\n", rect_path(16.0, 16.0, 64.0, 54.0));
    let offset = "4 /Real (ofst) ,\n6 /Real (mlim) ,\n1 /Int (jntp) ,\n";
    let r = read_ok(&format!("{}{}", styles(offset), layer(&art_)));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look: {a:#?}");
    let b = a[0].geometric_bounds().unwrap();
    assert_eq!((b.width(), b.height()), (40.0, 30.0), "the object keeps its own path");
    let effects: Vec<_> = a[0].appearance.effects.iter().map(|e| (e.id.as_str(), e.params.clone(), e.visible)).collect();
    assert_eq!(
        effects,
        [
            ("path.offsetPath", serde_json::json!({"offset": 4.0, "joins": "bevel", "miterLimit": 6.0}), true),
            ("path.outlineObject", serde_json::json!({}), false),
            ("path.outlineStroke", serde_json::json!({}), true),
        ]
    );
    // An Offset Path without its Miter Limit: the drawn look.
    let r = read_ok(&format!("{}{}", styles("4 /Real (ofst) ,\n1 /Int (jntp) ,\n"), layer(&art_)));
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
}

/// The paint of a blue fill in a style.
const BLUE_FILL: Option<&str> = Some("0 0 0 0 0 0 1 Xa\n");

/// An art style section with style `Bent`: effect `name` with dictionary `keys` on the whole
/// object, over a fill painting with `paint`, or over a fill and stroke that paint nothing.
fn distort_styles(name: &str, keys: &str, paint: Option<&str>) -> String {
    let (paints, color) = match paint {
        Some(p) => ("0 1", p),
        None => ("0 0", ""),
    };
    format!(
        "%AI9_BeginArtStyles\n/KnownStyle :\n(Bent) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n({name}) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n{keys}; /Dict ;\n /Part ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n(Stroke Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/StrokeStyle : 0 1 0 0 0 Xy\n0 0 Xd\n/Def ;\n (StrokeStyle) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n{color}{paints} Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         ;\n /Part ,\n;\n%AI9_EndArtStyles\n"
    )
}

/// Free Distort's 16 entries for a 40 × 30 pt object whose source frame is its own box, with its
/// destination corners `dst` (top left, top right, bottom left, bottom right, art space).
fn free_distort_keys(dst: [[f64; 2]; 4]) -> String {
    let src = [[20.0, 50.0], [60.0, 50.0], [20.0, 20.0], [60.0, 20.0]];
    let mut out = String::new();
    for (which, c) in [("src", src), ("dst", dst)] {
        for (i, [h, v]) in c.into_iter().enumerate() {
            out += &format!("{h} /Real ({which}{i}h) ,\n{v} /Real ({which}{i}v) ,\n");
        }
    }
    out
}

#[test]
fn an_object_with_a_distort_effect_that_draws_alike_comes_in_as_itself_with_it_live() {
    // GUARDS: only type took Distort & Transform effects; a path with Transform, Twist, a still Tweak,
    // Pucker & Bloat at 0 or an affine Free Distort came in as its drawn look.
    let hidden_rect = rect_path(20.0, 20.0, 60.0, 50.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{hidden_rect}f\n%_0 BB\n1 (Bent) XW\n", rect_path(16.0, 16.0, 64.0, 54.0));
    // A slanted box: its corners make a parallelogram.
    let slanted = free_distort_keys([[25.0, 50.0], [65.0, 50.0], [20.0, 20.0], [60.0, 20.0]]);
    for (name, keys, id) in [
        ("Adobe Transform", "5 /Real (moveH_Pts) ,\n0.5 /Real (rotate_Radians) ,\n0 /Bool (transformPatterns) ,\n", "distort.transform"),
        ("Adobe Twirl", "45 /Real (angle) ,\n", "distort.twist"),
        ("Adobe Scribble and Tweak", "30 /Real (horz) ,\n0 /Bool (anch) ,\n0 /Bool (in) ,\n0 /Bool (out) ,\n", "distort.tweak"),
        ("Adobe Punk and Bloat", "0 /Real (d_factor) ,\n", "distort.puckerBloat"),
        ("Adobe Free Distort", slanted.as_str(), "distort.freeDistort"),
        ("Adobe Free Distort", "", "distort.freeDistort"),
    ] {
        let r = read_ok(&format!("{}{}", distort_styles(name, keys, BLUE_FILL), layer(&art_)));
        assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{name}: {:?}", r.warnings);
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{name}: {a:#?}");
        assert!(matches!(a[0].kind, NodeKind::Path { .. }), "{name}: the object, not its look: {a:#?}");
        let b = a[0].geometric_bounds().unwrap();
        assert_eq!((b.width(), b.height()), (40.0, 30.0), "{name}: the object keeps its own path");
        assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)), "{name}");
        let ids: Vec<_> = a[0].appearance.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, [id], "{name}");
    }
}

#[test]
fn an_object_with_a_distort_effect_drawn_another_way_keeps_its_drawn_look() {
    // Pucker & Bloat and Zig Zag draw other outlines than the file's, a Free Distort whose corners
    // aren't a parallelogram bends the curves differently, and Transform's copies paint as one path.
    let hidden_rect = rect_path(20.0, 20.0, 60.0, 50.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{hidden_rect}f\n%_0 BB\n1 (Bent) XW\n", rect_path(16.0, 16.0, 64.0, 54.0));
    let bent = free_distort_keys([[25.0, 50.0], [65.0, 52.0], [20.0, 20.0], [60.0, 20.0]]);
    for (name, keys) in [
        ("Adobe Punk and Bloat", "-50 /Real (d_factor) ,\n"),
        ("Adobe Zigzag", "3 /Real (amount) ,\n1 /Real (absoluteness) ,\n"),
        ("Adobe Free Distort", bent.as_str()),
        ("Adobe Scribble and Tweak", "30 /Real (horz) ,\n"),
        ("Adobe Transform", "1 /Bool (randomize) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Transform", "5 /Real (moveH_Pts) ,\n2 /Int (numCopies) ,\n0 /Bool (transformPatterns) ,\n"),
    ] {
        let r = read_ok(&format!("{}{}", distort_styles(name, keys, BLUE_FILL), layer(&art_)));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{name}: {a:#?}");
        assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{name}: the drawn look: {a:#?}");
        assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{name}: {:?}", r.warnings);
    }
}

/// The gradient `G`, white to black, defined for a document.
const GRADIENT_G: &str = "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n%_1 0 50 0 Bs\n%_0 0 50 100 Bs\nBD\n%AI5_EndGradient\n";

#[test]
fn an_object_whose_outline_moves_away_from_its_gradient_keeps_its_drawn_look() {
    // GUARDS: a gradient stays where it is placed while Transform, Twist or Free Distort moves the
    // outline, so the live object lost the gradient its drawn look shows.
    let gradient = Some("Bb\n0 0 0 0 Bh\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n0 BB\n");
    let hidden_rect = rect_path(20.0, 20.0, 60.0, 50.0).replace('\n', "\n%_");
    let object = format!(
        "0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_1 (G) 20 35 0 40 1 0 0 1 0 0 Bg\n%_{hidden_rect}f\n%_0 BB\n1 (Bent) XW\n",
        rect_path(16.0, 16.0, 64.0, 54.0)
    );
    let hidden = |color: &str, x: f64| format!("{color}{}f\n", rect_path(x, 0.0, x + 10.0, 10.0)).replace('\n', "\n%_");
    let shaded = format!("Bb\n1 (G) 5 5 0 10 1 0 0 1 0 0 Bg\n{}f\n0 BB\n", rect_path(5.0, 0.0, 15.0, 10.0)).replace('\n', "\n%_");
    let group = format!("0 Ae\nu\n{RED}{}f\nU\n%_0 Ae\n%_u\n%_{}{shaded}U\n1 (Bent) XW\n", rect_path(0.0, 0.0, 15.0, 10.0), hidden(RED, 0.0));
    let slanted = free_distort_keys([[25.0, 50.0], [65.0, 50.0], [20.0, 20.0], [60.0, 20.0]]);
    let moves = [
        ("Adobe Transform", "5 /Real (moveH_Pts) ,\n0 /Bool (transformPatterns) ,\n"),
        ("Adobe Twirl", "45 /Real (angle) ,\n"),
        ("Adobe Free Distort", slanted.as_str()),
    ];
    for (name, keys) in moves {
        // A gradient in the style, and one on a member of a group.
        for (paint, art_) in [(gradient, &object), (None, &group)] {
            let r = read_ok(&format!("{GRADIENT_G}{}{}", distort_styles(name, keys, paint), layer(art_)));
            let a = art(&r.doc);
            assert_eq!(a.len(), 1, "{name}: {a:#?}");
            assert!(a[0].appearance.effects.is_empty(), "{name}: the drawn look: {a:#?}");
            assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{name}: {:?}", r.warnings);
        }
    }
    // An effect that leaves the outline where it is keeps the gradient under it.
    let r = read_ok(&format!("{GRADIENT_G}{}{}", distort_styles("Adobe Punk and Bloat", "", gradient), layer(&object)));
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "{a:#?}");
    assert!(matches!(fill(&a[0]), Paint::Gradient(_)), "{:?}", fill(&a[0]));
}

/// A group with style `Bent`, its drawn look one path, holding a red and a blue square.
fn distort_group() -> String {
    let hidden = |color: &str, x: f64| format!("{color}{}f\n", rect_path(x, 0.0, x + 10.0, 10.0)).replace('\n', "\n%_");
    format!(
        "0 Ae\nu\n{RED}{}f\nU\n%_0 Ae\n%_u\n%_{}{}U\n1 (Bent) XW\n",
        rect_path(0.0, 0.0, 15.0, 10.0),
        hidden(RED, 0.0),
        hidden("0 0 1 0 0 0 1 Xa\n", 5.0)
    )
}

#[test]
fn a_group_with_a_transform_keeps_its_members_and_the_effect() {
    // GUARDS: the group came in as its drawn look, one path with no effect.
    let keys = "5 /Real (moveH_Pts) ,\n0 /Bool (transformPatterns) ,\n";
    let r = read_ok(&format!("{}{}", distort_styles("Adobe Transform", keys, None), layer(&distort_group())));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let members = a[0].children().cloned().unwrap_or_default();
    assert_eq!(members.len(), 2, "the squares, not the look: {a:#?}");
    let [e] = a[0].appearance.effects.as_slice() else { panic!("{:?}", a[0].appearance) };
    assert_eq!((e.id.as_str(), e.params["moveH"].as_f64()), ("distort.transform", Some(5.0)));
    assert!(a[0].appearance.items.is_empty(), "{:?}", a[0].appearance);
}

/// `styles` (from [`distort_styles`]) with a red stroke 2 pt wide in place of the stroke that
/// paints nothing.
fn with_red_stroke(styles: &str) -> String {
    styles.replace("/StrokeStyle : 0 1 0 0 0 Xy\n0 0 Xd", "/StrokeStyle : 0 R\n0 1 1 0 1 0 0 XA\n0 1 0 0 0 Xy\n0 J 0 j 2 w 10 M []0 d\n0 XR\n1 0 Xd")
}

#[test]
fn twist_and_a_scaling_transform_are_read_only_where_their_outline_is_known() {
    // GUARDS: Twist was read on curves, strokes and groups, and Transform scaling a stroked object,
    // where the outline VectorCraft draws hasn't been checked.
    let object = |hidden: &str| {
        format!("0 Ae\nu\n{RED}{}f\nU\n%_Bb\n%_{}f\n%_0 BB\n1 (Bent) XW\n", rect_path(16.0, 16.0, 64.0, 54.0), hidden.replace('\n', "\n%_"))
    };
    let square = object(&rect_path(20.0, 20.0, 60.0, 50.0));
    let curved = object("20 35 m\n20 45 60 45 60 35 c\n60 25 20 25 20 35 c\n");
    let twist = distort_styles("Adobe Twirl", "45 /Real (angle) ,\n", BLUE_FILL);
    let scale = "0.5 /Real (scaleH_Factor) ,\n0 /Bool (scaleLines) ,\n0 /Bool (transformPatterns) ,\n";
    let corner = distort_styles("Adobe Transform", &format!("0 /Int (pinPoint) ,\n{scale}"), BLUE_FILL);
    let center = distort_styles("Adobe Transform", scale, BLUE_FILL);
    let moved = distort_styles("Adobe Transform", "0 /Int (pinPoint) ,\n5 /Real (moveH_Pts) ,\n0 /Bool (transformPatterns) ,\n", BLUE_FILL);
    let read = |styles: &str, art_: &str| read_ok(&format!("{styles}{}", layer(art_)));
    for (what, r) in [
        ("a twisted curve", read(&twist, &curved)),
        ("a twisted stroke", read(&with_red_stroke(&twist), &square)),
        ("a twisted group", read(&distort_styles("Adobe Twirl", "45 /Real (angle) ,\n", None), &distort_group())),
        ("a stroke scaled from a corner", read(&with_red_stroke(&corner), &square)),
        ("a stroke scaled from the center", read(&with_red_stroke(&center), &square)),
    ] {
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{what}: {a:#?}");
        assert!(a[0].appearance.effects.is_empty(), "{what}: the drawn look: {a:#?}");
        assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{what}: {:?}", r.warnings);
    }
    for (what, r) in [
        ("a twisted square", read(&twist, &square)),
        ("a stroke moved from a corner", read(&with_red_stroke(&moved), &square)),
        ("a fill scaled from a corner", read(&corner, &square)),
        ("a fill scaled from the center", read(&center, &square)),
    ] {
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{what}: {a:#?}");
        assert!(matches!(a[0].kind, NodeKind::Path { .. }) && a[0].appearance.effects.len() == 1, "{what}: {a:#?}");
    }
    let r = read(&twist, &square);
    let a = art(&r.doc);
    let [e] = a[0].appearance.effects.as_slice() else { panic!("{:?}", a[0].appearance) };
    assert_eq!((e.id.as_str(), e.params["angle"].as_f64()), ("distort.twist", Some(45.0)));
}

#[test]
fn a_bare_comment_line_in_a_prolog_leaves_its_end_alone() {
    // GUARDS: a bare `%` line's empty marker matched the empty begin marker of a section that has none.
    let r = read_ok(&format!("%%BeginProlog\n%\n%%EndProlog\n{}", layer(&format!("{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0)))));
    assert_eq!(art(&r.doc).len(), 1);
}

#[test]
fn a_styled_object_inside_a_drawn_look_survives_it() {
    // GUARDS: an adopted styled object was read on `%_` lines; while its id stayed among those, the
    // enclosing look's strip removed it.
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!(
        "0 Ae\nu\n0 Ae\nu\n{RED}{}f\nU\n%_{hidden_rect}f\n1 (Blue) XW\nU\n%_0 Ae\n%_u\n%_{hidden_rect}f\n%_U\n1 (Outer) XW\n",
        rect_path(0.0, 0.0, 10.0, 10.0)
    );
    let r = read_ok(&format!("{ART_STYLES}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let inner = a[0].children().unwrap();
    assert_eq!(inner.len(), 1, "{inner:#?}");
    assert_eq!(fill(&inner[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    let shadow = &inner[0].appearance.effects[0];
    assert_eq!((shadow.id.as_str(), shadow.params["x"].as_f64(), shadow.params["y"].as_f64()), ("stylize.dropShadow", Some(3.0), Some(4.0)));
}

/// An object whose style's Drop Shadow has Darkness (`csrc` 1, `dark` 50) comes in as itself with
/// that shadow, not as its drawn look.
#[test]
fn an_object_with_a_darkness_shadow_comes_in_as_itself() {
    let styles = ART_STYLES.replace("0 /Int (csrc) ,\n", "1 /Int (csrc) ,\n50 /Real (dark) ,\n");
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    let art_ = format!("0 Ae\nu\n{RED}{}f\nU\n%_{hidden_rect}f\n1 (Blue) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{styles}{}", layer(&art_)));
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the object, not its look: {a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    let [shadow] = a[0].appearance.effects.as_slice() else { panic!("{:?}", a[0].appearance.effects) };
    assert_eq!(
        (shadow.id.as_str(), shadow.params["shade"].as_str(), shadow.params["darkness"].as_f64(), shadow.params["x"].as_f64()),
        ("stylize.dropShadow", Some("darkness"), Some(50.0), Some(3.0))
    );
}

/// A styled object read in place of its drawn look keeps the transparency written on the look (the
/// object's own, after the look's group ends): here 60 % in Multiply.
#[test]
fn a_styled_object_keeps_the_transparency_written_on_its_look() {
    let hidden_rect = rect_path(0.0, 0.0, 10.0, 10.0).replace('\n', "\n%_");
    // The object on `%_` lines sets its own paths' transparency back to normal, as files do.
    let art_ =
        format!("0 Ae\nu\n{RED}{}f\nU\n1 0.6 0 2 0 Xy\n6 () XW\n%_0 1 0 0 0 Xy\n%_{hidden_rect}f\n1 (Blue) XW\n", rect_path(0.0, 0.0, 10.0, 10.0));
    let r = read_ok(&format!("{ART_STYLES}{}", layer(&art_)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!((a[0].opacity, a[0].blend), (0.6, BlendMode::Multiply));
    assert!(!a[0].knockout_shape);
    let r = read_ok(&format!("{ART_STYLES}{}", layer(&art_.replace("1 0.6 0 2 0 Xy", "1 0.6 0 2 1 Xy"))));
    assert!(art(&r.doc)[0].knockout_shape, "written on the look");
}

/// The knockout shape flag written on a text object's look goes to the object, or, where the look
/// draws more than the type, opaque or not, to the group that holds the look and the object.
#[test]
fn a_text_objects_look_gives_it_the_knockout_shape() {
    let look = "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\nU\n1 0.2 0 1 1 Xy\n0 0 Xd\n6 () XW\n";
    let own = "%_/AI11Text :\n%_0 /FreeUndo ,\n%_0 /FrameIndex ,\n%_0 /StoryIndex ,\n%_;\n";
    let doc = read_ok(&layer(&format!("{look}{own}1 (Style) XW\n"))).doc;
    let a = art(&doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!((a[0].opacity, a[0].knockout_shape), (0.2, true));
    let drawing = look.replace("U\n", &format!("{RED}{}f\nU\n", rect_path(0.0, 0.0, 10.0, 10.0)));
    for (state, opacity) in [("1 0.2 0 1 1 Xy", 0.2), ("0 1 0 1 1 Xy", 1.0)] {
        let drawn = drawing.replace("1 0.2 0 1 1 Xy", state);
        let doc = read_ok(&layer(&format!("{drawn}{own}1 (Style) XW\n"))).doc;
        let a = art(&doc);
        assert_eq!(a.len(), 1, "{state}: {a:#?}");
        assert_eq!((a[0].opacity, a[0].knockout_shape), (opacity, true), "{state}");
        assert!(a[0].children().unwrap().iter().all(|n| !n.knockout_shape), "{state}: {a:#?}");
    }
}

/// An envelope is written on `%_` lines as a group holding the mesh that shapes it, with its
/// distortion in the group's dictionary; the distorted art follows on plain lines. Only that art
/// is drawn, as in the file's PDF part, not the envelope's mesh (#2405).
#[test]
fn an_envelope_mesh_is_not_read_as_art() {
    let corner = |x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| {
        format!("%_[0.82 0.83 0.83 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1)
    };
    let mesh = format!(
        "%_/Mesh X!\n%_(DeviceRGB) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n%_/End X!\n",
        corner(0, 0, (0, 10), (10, 0)),
        corner(0, 40, (10, 40), (0, 30)),
        corner(40, 40, (40, 30), (30, 40)),
        corner(40, 0, (30, 0), (40, 10)),
    );
    let envelope = |dictionary: &str| {
        format!(
            "%_0 Ae\n%_u\n{mesh}%_0 0 Xd\n%_6 () XW\n%_U\n%_/ArtDictionary :\n{dictionary}%_;\n%_\n%_0 0 Xd\n%_6 () XW\n{RED}{}f\n",
            rect_path(0.0, 0.0, 40.0, 40.0)
        )
    };
    let has_mesh = |n: &Node| -> bool {
        fn walk(n: &Node) -> bool {
            matches!(n.kind, NodeKind::Mesh(_)) || n.children().is_some_and(|c| c.iter().any(|k| walk(k)))
        }
        walk(n)
    };

    let warped = envelope("%_(Warp: Arch) /UnicodeString (DisplayString) ,\n%_0.27 /Real (DeformValue) ,\n%_1 /Int (AIDeformStyle) ,\n");
    let doc = read_ok(&layer(&warped)).doc;
    let a = art(&doc);
    assert!(!a.iter().any(|n| has_mesh(n)), "{a:#?}");
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));

    // A mesh in a group with no distortion is still art.
    let doc = read_ok(&layer(&envelope(""))).doc;
    assert!(art(&doc).iter().any(|n| has_mesh(n)), "{:#?}", art(&doc));
}

/// An envelope whose contents are being edited is written the other way round: its contents, a
/// group on `%_` lines, then its dictionary, which holds the mesh that shapes them. The contents
/// aren't drawn as they are, only the distorted art on the plain lines after them.
#[test]
fn an_envelope_s_contents_being_edited_are_not_read_as_art() {
    let corner = |x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| {
        format!("%_[0.82 0.83 0.83 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1)
    };
    let mesh = format!(
        "%_/Mesh X!\n%_(DeviceRGB) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n%_/End X!\n%_0 0 Xd\n%_6 () XW\n",
        corner(0, 0, (0, 10), (10, 0)),
        corner(0, 40, (10, 40), (0, 30)),
        corner(40, 40, (40, 30), (30, 40)),
        corner(40, 0, (30, 0), (40, 10)),
    );
    let contents: String = rect_path(0.0, 0.0, 80.0, 20.0).lines().map(|l| format!("%_{l}\n")).collect();
    let body = format!(
        "%_u\n%_u\n%_0 0 1 0 0 0 1 Xa\n{contents}%_f\n%_U\n%_9 () XW\n%_U\n%_/ArtDictionary :\n%_1 /Bool (AIDeformEditModeKey) ,\n\
         %_3 /Int (AIDeformStyle) ,\n%_X=\n{mesh}%_X+\n%_ (AIDeformArtKey) ,\n%_;\n%_\n%_9 () XW\n{RED}{}f\n",
        rect_path(0.0, 0.0, 40.0, 40.0)
    );
    let doc = read_ok(&layer(&body)).doc;
    let a = art(&doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert_eq!(fill(&a[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

/// `art` on `%_` lines.
fn commented(art: &str) -> String {
    art.lines().map(|l| format!("%_{l}\n")).collect()
}

/// Envelope Options as new envelopes have them (Distort Appearance, Anti-Alias, Clipping Mask),
/// Fidelity 50, the contents not being edited.
const ENVELOPE_KEYS: &str = "131073 /Int (AIDeformOptionsKey) ,\n50 /Int (AIDeformFidelityKey) ,\n0 /Bool (AIDeformEditModeKey) ,\n";

/// An envelope as the editing data writes it: on `%_` lines a group holding `before`, then the
/// dictionary with `keys`, the art `block` in its `AIDeformArtKey` block and the kind `kind`; then
/// `drawn` in a group on plain lines, and the plugin's `XP`.
fn envelope_parts(before: &str, keys: &str, block: &str, kind: u32, drawn: &str) -> String {
    format!(
        "{}%_/ArtDictionary :\n{}%_X=\n{}%_X+\n%_ (AIDeformArtKey) ,\n%_{kind} /Int (AIDeformStyle) ,\n%_;\n%_\n%_9 () XW\nu\n{drawn}U\n9 () XW\n(Adobe Deform Plugin) 1 0 0 XP\n",
        commented(&format!("u\n{before}U\n")),
        commented(keys),
        commented(block),
    )
}

/// An envelope of `content` (in the plain group the editing data puts round it) shaped by the mesh
/// `shape`.
pub(super) fn envelope_of(shape: &str, keys: &str, content: &str, kind: u32, drawn: &str) -> String {
    envelope_parts(shape, keys, &format!("u\n{content}U\n9 () XW\n"), kind, drawn)
}

/// A grid point's place in art space and its handle towards a neighbour.
type MeshAt<'a> = &'a dyn Fn((usize, usize)) -> (f64, f64);
type MeshHandle<'a> = &'a dyn Fn((usize, usize), (usize, usize)) -> (f64, f64);

/// A grey `/Mesh` of `cols`×`rows` patches (`/P [i j]`, `i` along x, column by column), each
/// corner of patch `(i, j)` at `at`, from the one nearest the origin up, across and down, with its
/// handles towards the next corner and the previous one at `handle`; `anchor`, written after the
/// second corner of the first patch, puts an anchor on its second edge.
fn mesh_text(cols: usize, rows: usize, at: MeshAt<'_>, handle: MeshHandle<'_>, anchor: Option<[f64; 6]>) -> String {
    let mut s = format!("/Mesh X!\n2 /Version X#\n/Cartesian /Type X#\n[{cols} {rows}] /Size X#\n/Data X#\n/DeviceGray /CS X#\n");
    for i in 0..cols {
        for j in 0..rows {
            s += &format!("[{i} {j}] /P X#\n");
            let (a, b, c, d) = ((i, j), (i, j + 1), (i + 1, j + 1), (i + 1, j));
            for (k, (p, next, prev)) in [(a, b, d), (b, c, a), (c, d, b), (d, a, c)].into_iter().enumerate() {
                let ((x, y), (nx, ny), (px, py)) = (at(p), handle(p, next), handle(p, prev));
                s += &format!("[0.8 1 {x} {y} {nx} {ny} 1 {px} {py} 1] /N X#\n");
                if let (1, Some([x, y, ax, ay, bx, by])) = (k, anchor.filter(|_| (i, j) == (0, 0))) {
                    s += &format!("[{x} {y} {ax} {ay} {bx} {by} 1] /A X#\n");
                }
            }
            s += "/E X#\n";
        }
    }
    s + "/End X!\n"
}

/// A grid in art space, its points `step` apart from (40, 20), handles a third of the way.
pub(super) fn straight_mesh(cols: usize, rows: usize, step: (f64, f64)) -> String {
    let at = move |(i, j): (usize, usize)| (40.0 + i as f64 * step.0, 20.0 + j as f64 * step.1);
    let third = move |p: (usize, usize), q: (usize, usize)| {
        let ((x, y), (u, v)) = (at(p), at(q));
        (x + (u - x) / 3.0, y + (v - y) / 3.0)
    };
    mesh_text(cols, rows, &at, &third, None)
}

/// The red rectangle 40 20 160 80 in art space, which is the same box in the document's.
pub(super) fn envelope_content() -> String {
    format!("{RED}{}f\n", rect_path(40.0, 20.0, 160.0, 80.0))
}

/// What envelope `kind` draws of [`envelope_content`]: the rectangle's edges, many points to a
/// side, mapped as VectorCraft maps them, as a red path in art space.
pub(super) fn drawn_content(kind: &vectorcraft_doc::live::EnvelopeKind) -> String {
    let rect = Rect::new(40.0, 20.0, 160.0, 80.0);
    drawn_box(kind, rect, rect)
}

/// What envelope `kind`, mapping from box `src`, draws of a red rectangle `rect`, as in
/// [`drawn_content`].
fn drawn_box(kind: &vectorcraft_doc::live::EnvelopeKind, src: Rect, rect: Rect) -> String {
    let map = vectorcraft_doc::live::envelope_mapper(kind, src);
    let corners = [Point::new(rect.x0, rect.y0), Point::new(rect.x1, rect.y0), Point::new(rect.x1, rect.y1), Point::new(rect.x0, rect.y1)];
    let mut s = RED.to_string();
    for (k, a) in corners.iter().enumerate() {
        let b = corners[(k + 1) % 4];
        for t in 0..64 {
            let q = map(a.lerp(b, f64::from(t) / 64.0));
            s += &format!("{} {} {}\n", q.x, 100.0 - q.y, if k == 0 && t == 0 { "m" } else { "L" });
        }
    }
    s + "f\n"
}

/// The live envelope `n` is: its content, kind, fidelity, editing and options.
fn live_envelope(n: &Node) -> (&[Arc<Node>], &vectorcraft_doc::live::EnvelopeKind, f64, bool, vectorcraft_doc::live::EnvelopeOptions) {
    match &n.kind {
        NodeKind::Envelope { content, kind, fidelity, editing, options, frame } => {
            assert_eq!(**frame, kurbo::Affine::IDENTITY);
            (content, kind, *fidelity, *editing, *options)
        }
        other => panic!("not a live envelope: {other:#?}"),
    }
}

/// A warp's dictionary keys: style number `style`, bend, horizontal and vertical distortion as
/// fractions, and the orientation (vertical with `rotate` 1).
pub(super) fn warp_keys(style: u32, bend: f64, h: f64, v: f64, rotate: u32) -> String {
    format!(
        "{ENVELOPE_KEYS}{style} /Int (DeformStyle) ,\n{h} /Real (DeformHoriz) ,\n{bend} /Real (DeformValue) ,\n{v} /Real (DeformVert) ,\n{rotate} /Bool (Rotate) ,\n(Warp) /UnicodeString (DisplayString) ,\n"
    )
}

/// An envelope made with a warp opens as that live envelope of its contents, its style, bend,
/// distortions (fractions in the file) and orientation read; the warp's drawn art goes.
#[test]
fn a_warp_envelope_opens_live() {
    use vectorcraft_doc::live::EnvelopeKind;
    for (style, bend, h, v, rotate, id) in
        [(1, 0.5, 0.0, 0.0, 0, "arc"), (1, 0.5, -0.2, 0.3, 1, "arc"), (8, 0.5, 0.0, 0.0, 0, "flag"), (15, 0.5, 0.0, 0.0, 0, "twist")]
    {
        let want = EnvelopeKind::Warp { style: id.into(), bend: bend * 100.0, h: h * 100.0, v: v * 100.0, horizontal: rotate == 0 };
        let body =
            envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &warp_keys(style, bend, h, v, rotate), &envelope_content(), 1, &drawn_content(&want));
        let r = read_ok(&layer(&body));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{a:#?}");
        let (content, kind, fidelity, editing, options) = live_envelope(&a[0]);
        let EnvelopeKind::Warp { style: s, bend: b, h: dh, v: dv, horizontal } = kind else { panic!("{kind:?}") };
        assert_eq!((s.as_str(), *horizontal), (id, rotate == 0));
        assert!([(*b, bend), (*dh, h), (*dv, v)].iter().all(|(got, f)| (got - f * 100.0).abs() < 1e-9), "{kind:?}");
        assert_eq!((fidelity, editing, options), (50.0, false, vectorcraft_doc::live::EnvelopeOptions::NEW));
        assert_eq!(content.len(), 1, "{content:#?}");
        assert!(matches!(content[0].kind, NodeKind::Path { .. }), "{content:#?}");
        assert_eq!(fill(&content[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    }
}

/// Every warp style number reads as its style, in Effect › Warp's order.
#[test]
fn warp_envelopes_read_every_style() {
    use vectorcraft_doc::live::EnvelopeKind;
    let ids = [
        "arc",
        "arcLower",
        "arcUpper",
        "arch",
        "bulge",
        "shellLower",
        "shellUpper",
        "flag",
        "wave",
        "fish",
        "rise",
        "fisheye",
        "inflate",
        "squeeze",
        "twist",
    ];
    for (n, id) in ids.iter().enumerate() {
        let want = EnvelopeKind::Warp { style: (*id).into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true };
        let keys = warp_keys(n as u32 + 1, 0.3, 0.0, 0.0, 0);
        let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &keys, &envelope_content(), 1, &drawn_content(&want))));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{id}: {a:#?}");
        assert_eq!(live_envelope(&a[0]).1, &want, "{id}");
    }
}

/// An envelope made with a mesh opens as a live mesh envelope: its rows and columns, its points
/// row by row from the top left, and each point's four handles, a moved point and an uneven handle
/// included.
#[test]
fn a_mesh_envelope_opens_live_with_its_points_and_handles() {
    use vectorcraft_doc::live::{EnvelopeKind, H_DOWN, H_LEFT, H_RIGHT, H_UP};
    // 3 columns 40 apart and 2 rows 30 apart from (40, 20); the inner point (1, 1) moved, and its
    // handle towards (2, 1) longer and off the line.
    let at = |(i, j): (usize, usize)| if (i, j) == (1, 1) { (90.0, 55.0) } else { (40.0 + i as f64 * 40.0, 20.0 + j as f64 * 30.0) };
    let handle = |p: (usize, usize), q: (usize, usize)| {
        if (p, q) == ((1, 1), (2, 1)) {
            return (108.0, 62.0);
        }
        let ((x, y), (u, v)) = (at(p), at(q));
        (x + (u - x) / 3.0, y + (v - y) / 3.0)
    };
    let shape = mesh_text(3, 2, &at, &handle, None);
    // Row r of the envelope is row 2 - r of the file (art space has y up).
    let doc_pt = |(x, y): (f64, f64)| Point::new(x, 100.0 - y);
    let mut points = vec![];
    let mut handles = vec![];
    for r in 0..=2usize {
        for c in 0..=3usize {
            let g = (c, 2 - r);
            let p = doc_pt(at(g));
            points.push(p);
            let mut h = [kurbo::Vec2::ZERO; 4];
            let towards = |q: Option<(usize, usize)>| q.filter(|q| q.0 <= 3 && q.1 <= 2).map_or(kurbo::Vec2::ZERO, |q| doc_pt(handle(g, q)) - p);
            h[H_RIGHT] = towards(Some((g.0 + 1, g.1)));
            h[H_LEFT] = towards(g.0.checked_sub(1).map(|i| (i, g.1)));
            h[H_UP] = towards(Some((g.0, g.1 + 1)));
            h[H_DOWN] = towards(g.1.checked_sub(1).map(|j| (g.0, j)));
            handles.push(h);
        }
    }
    let want = EnvelopeKind::Mesh { rows: 2, cols: 3, points: points.clone(), handles: handles.clone() };
    let r = read_ok(&layer(&envelope_of(&shape, ENVELOPE_KEYS, &envelope_content(), 2, &drawn_content(&want))));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let EnvelopeKind::Mesh { rows, cols, points: got, handles: got_h } = live_envelope(&a[0]).1 else { panic!("{a:#?}") };
    assert_eq!((*rows, *cols, got.len(), got_h.len()), (2, 3, 12, 12));
    for (k, (p, q)) in got.iter().zip(&points).enumerate() {
        assert!((*p - *q).hypot() < 1e-6, "point {k}: {p:?}, not {q:?}");
    }
    // The moved inner point (row 1, column 1): each of its handles, the uneven one towards the right.
    let inner = 4 + 1;
    assert!((got[inner] - Point::new(90.0, 45.0)).hypot() < 1e-6, "{:?}", got[inner]);
    for slot in [H_RIGHT, H_LEFT, H_DOWN, H_UP] {
        assert!((got_h[inner][slot] - handles[inner][slot]).hypot() < 1e-6, "slot {slot}: {:?} vs {:?}", got_h[inner], handles[inner]);
    }
    assert!((got_h[inner][H_RIGHT] - kurbo::Vec2::new(18.0, -7.0)).hypot() < 1e-6, "{:?}", got_h[inner]);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// An envelope made with a top object opens as a live envelope whose top path is the edge of its
/// one-patch mesh, an anchor on the edge included; its contents stay the rectangle.
#[test]
fn a_top_object_envelope_opens_live_with_its_outline() {
    use vectorcraft_doc::live::EnvelopeKind;
    // Corners (40, 20) (40, 80) (160, 80) (160, 20); the top edge bows up through an anchor at (100, 95).
    let at = |(i, j): (usize, usize)| (40.0 + i as f64 * 120.0, 20.0 + j as f64 * 60.0);
    let handle = |p: (usize, usize), q: (usize, usize)| match (p, q) {
        ((0, 1), (1, 1)) => (60.0, 88.0),
        ((1, 1), (0, 1)) => (140.0, 88.0),
        _ => {
            let ((x, y), (u, v)) = (at(p), at(q));
            (x + (u - x) / 3.0, y + (v - y) / 3.0)
        }
    };
    let shape = mesh_text(1, 1, &at, &handle, Some([100.0, 95.0, 70.0, 95.0, 130.0, 95.0]));
    let mut bp = kurbo::BezPath::new();
    bp.move_to((40.0, 20.0));
    bp.curve_to((60.0, 12.0), (70.0, 5.0), (100.0, 5.0));
    bp.curve_to((130.0, 5.0), (140.0, 12.0), (160.0, 20.0));
    bp.curve_to((160.0, 40.0), (160.0, 60.0), (160.0, 80.0));
    bp.curve_to((120.0, 80.0), (80.0, 80.0), (40.0, 80.0));
    bp.curve_to((40.0, 60.0), (40.0, 40.0), (40.0, 20.0));
    bp.close_path();
    let want = EnvelopeKind::TopObject { path: vectorcraft_geom::PathData::from_bezpath(&bp), corners: None };
    let r = read_ok(&layer(&envelope_of(&shape, ENVELOPE_KEYS, &envelope_content(), 3, &drawn_content(&want))));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let (content, kind, ..) = live_envelope(&a[0]);
    let EnvelopeKind::TopObject { path, corners } = kind else { panic!("{kind:?}") };
    let anchors: Vec<Point> = path.subpaths.iter().flat_map(|s| s.anchors.iter().map(|a| a.p)).collect();
    for p in [(40.0, 20.0), (100.0, 5.0), (160.0, 20.0), (160.0, 80.0), (40.0, 80.0)] {
        assert!(anchors.iter().any(|q| (*q - Point::from(p)).hypot() < 1e-6), "{p:?} not in {anchors:?}");
    }
    // The mesh's four corners are kept as the outline's corners.
    let corners = corners.expect("the saved corners");
    let at: Vec<Point> = corners.iter().map(|i| anchors[*i as usize]).collect();
    let want_corners = [(40.0, 20.0), (160.0, 20.0), (160.0, 80.0), (40.0, 80.0)].map(Point::from);
    assert!(at.iter().zip(want_corners).all(|(p, q)| (*p - q).hypot() < 1e-6), "{at:?}");
    // The curve through the anchor: its handles there run level.
    let mid = path.subpaths.iter().flat_map(|s| s.anchors.iter()).find(|a| (a.p - Point::new(100.0, 5.0)).hypot() < 1e-6).unwrap();
    assert!((mid.h_in.y - 5.0).abs() < 1e-6 && (mid.h_out.y - 5.0).abs() < 1e-6, "{mid:?}");
    assert_eq!(content.len(), 1, "{content:#?}");
    assert_eq!(fill(&content[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// Envelope Options come from the options word, Fidelity from its own key.
#[test]
fn an_envelope_reads_its_options_and_fidelity() {
    use vectorcraft_doc::live::{EnvelopeKind, EnvelopeOptions, PreserveShape};
    let shape = straight_mesh(1, 1, (120.0, 60.0));
    // A flat 1 × 1 mesh over the box draws the box as it is.
    let drawn = drawn_content(&EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(40.0, 20.0), Point::new(160.0, 20.0), Point::new(40.0, 80.0), Point::new(160.0, 80.0)],
        handles: vec![],
    });
    for (word, fidelity, options) in [
        (
            65536,
            80,
            EnvelopeOptions {
                anti_alias: false,
                preserve_shape: PreserveShape::Transparency,
                distort_appearance: false,
                distort_linear_gradients: false,
                distort_pattern_fills: false,
                bounds_margin: [0.0; 4],
            },
        ),
        (
            0x20007,
            100,
            EnvelopeOptions {
                anti_alias: true,
                preserve_shape: PreserveShape::ClippingMask,
                distort_appearance: true,
                distort_linear_gradients: true,
                distort_pattern_fills: true,
                bounds_margin: [0.0; 4],
            },
        ),
        (
            0x20002,
            0,
            EnvelopeOptions {
                anti_alias: true,
                preserve_shape: PreserveShape::ClippingMask,
                distort_appearance: false,
                distort_linear_gradients: false,
                distort_pattern_fills: true,
                bounds_margin: [0.0; 4],
            },
        ),
    ] {
        let keys = format!("{word} /Int (AIDeformOptionsKey) ,\n{fidelity} /Int (AIDeformFidelityKey) ,\n0 /Bool (AIDeformEditModeKey) ,\n");
        let r = read_ok(&layer(&envelope_of(&shape, &keys, &envelope_content(), 2, &drawn)));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{word}: {a:#?}");
        let (_, _, got_fidelity, _, got) = live_envelope(&a[0]);
        assert_eq!((got_fidelity, got), (f64::from(fidelity), options), "{word}");
    }
}

/// The red rectangle 40 20 160 80 with a red stroke 10 pt wide, its art 130 × 70 with the stroke.
fn stroked_content() -> String {
    format!("{RED}0 1 1 0 1 0 0 XA\n10 w\n{}b\n", rect_path(40.0, 20.0, 160.0, 80.0))
}

/// A one-patch mesh over the stroked art of [`stroked_content`], and that patch as a flat grid.
fn stroked_shape() -> (String, Vec<Point>) {
    let at = |(i, j): (usize, usize)| (35.0 + i as f64 * 130.0, 15.0 + j as f64 * 70.0);
    let third = |p: (usize, usize), q: (usize, usize)| {
        let ((x, y), (u, v)) = (at(p), at(q));
        (x + (u - x) / 3.0, y + (v - y) / 3.0)
    };
    (mesh_text(1, 1, &at, &third, None), vec![Point::new(35.0, 15.0), Point::new(165.0, 15.0), Point::new(35.0, 85.0), Point::new(165.0, 85.0)])
}

/// An envelope whose file saves the size of its art with the stroke maps from that box, in a
/// synthetic mesh and top object. Their drawn art is the box mapped onto the shape, and the
/// geometric box would leave the stroke 5 pt past it.
#[test]
fn an_envelope_saved_with_its_stroke_maps_from_the_stroked_box() {
    use vectorcraft_doc::live::{EnvelopeKind, EnvelopeMap};
    let (mesh, points) = stroked_shape();
    let keys = format!("{ENVELOPE_KEYS}70 /Real (AIDeformArtHeightKey) ,\n130 /Real (AIDeformArtWidthKey) ,\n");
    let outer = Rect::new(35.0, 15.0, 165.0, 85.0);
    let flat = EnvelopeKind::Mesh { rows: 1, cols: 1, points, handles: vec![] };
    let drawn = drawn_box(&flat, outer, outer);
    for kind in [2, 3] {
        let r = read_ok(&layer(&envelope_of(&mesh, &keys, &stroked_content(), kind, &drawn)));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{a:#?}");
        assert!(r.warnings.is_empty(), "kind {kind}: {:?}", r.warnings);
        let (.., options) = live_envelope(&a[0]);
        assert_eq!(options.bounds_margin, [5.0_f32 / 60.0, 5.0 / 60.0, 5.0 / 120.0, 5.0 / 120.0]);
        let src = EnvelopeMap::of(&a[0]).unwrap().src();
        assert!(
            (src.x0 - 35.0).abs() < 1e-3 && (src.y0 - 15.0).abs() < 1e-3 && (src.x1 - 165.0).abs() < 1e-3 && (src.y1 - 85.0).abs() < 1e-3,
            "{src:?}"
        );
    }
}

/// With Distort Appearance off the stroke is drawn after the distortion: the envelope maps from the
/// geometric box whatever size its file saves.
#[test]
fn an_envelope_without_distort_appearance_maps_from_the_geometric_box() {
    use vectorcraft_doc::live::EnvelopeMap;
    let keys = "65536 /Int (AIDeformOptionsKey) ,\n50 /Int (AIDeformFidelityKey) ,\n0 /Bool (AIDeformEditModeKey) ,\n70 /Real (AIDeformArtHeightKey) ,\n130 /Real (AIDeformArtWidthKey) ,\n";
    let outer = Rect::new(35.0, 15.0, 165.0, 85.0);
    let flat = vectorcraft_doc::live::EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(35.0, 15.0), Point::new(165.0, 15.0), Point::new(35.0, 85.0), Point::new(165.0, 85.0)],
        handles: vec![],
    };
    let drawn = drawn_box(&flat, outer, outer);
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), keys, &stroked_content(), 2, &drawn)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(!live_envelope(&a[0]).4.distort_appearance);
    assert_eq!(live_envelope(&a[0]).4.bounds_margin, [0.0; 4]);
    let src = EnvelopeMap::of(&a[0]).unwrap().src();
    assert!((src.width() - 120.0).abs() < 1e-6 && (src.height() - 60.0).abs() < 1e-6, "{src:?}");
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// An envelope maps from the geometric box when its file saves that box's size, saves no size,
/// saves a size that isn't a number or saves a size far from both boxes; contents without a stroke
/// have one box, and contents with no height have no margins.
#[test]
fn an_envelope_not_saved_with_its_stroke_maps_from_the_geometric_box() {
    use vectorcraft_doc::live::{EnvelopeKind, EnvelopeMap};
    let (mesh, points) = stroked_shape();
    let inner = Rect::new(40.0, 20.0, 160.0, 80.0);
    let flat = EnvelopeKind::Mesh { rows: 1, cols: 1, points, handles: vec![] };
    // The stroked art mapped from the geometric box: it reaches past the mesh by the stroke's share.
    let drawn = drawn_box(&flat, inner, Rect::new(35.0, 15.0, 165.0, 85.0));
    let sizes = [
        "60 /Real (AIDeformArtHeightKey) ,\n120 /Real (AIDeformArtWidthKey) ,\n",
        "",
        "(tall) /UnicodeString (AIDeformArtHeightKey) ,\n(wide) /UnicodeString (AIDeformArtWidthKey) ,\n",
        "70 /Real (AIDeformArtHeightKey) ,\n",
        "1000 /Real (AIDeformArtHeightKey) ,\n1000 /Real (AIDeformArtWidthKey) ,\n",
        "71 /Real (AIDeformArtHeightKey) ,\n131 /Real (AIDeformArtWidthKey) ,\n",
    ];
    for size in sizes {
        let keys = format!("{ENVELOPE_KEYS}{size}");
        let r = read_ok(&layer(&envelope_of(&mesh, &keys, &stroked_content(), 2, &drawn)));
        let a = art(&r.doc);
        assert_eq!(a.len(), 1, "{size}: {a:#?}");
        let (.., options) = live_envelope(&a[0]);
        assert_eq!(options.bounds_margin, [0.0; 4], "{size}");
        let src = EnvelopeMap::of(&a[0]).unwrap().src();
        assert!((src.width() - 120.0).abs() < 1e-6 && (src.height() - 60.0).abs() < 1e-6, "{size}: {src:?}");
    }
    let keys = format!("{ENVELOPE_KEYS}60 /Real (AIDeformArtHeightKey) ,\n120 /Real (AIDeformArtWidthKey) ,\n");
    let plain = drawn_content(&EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(40.0, 20.0), Point::new(160.0, 20.0), Point::new(40.0, 80.0), Point::new(160.0, 80.0)],
        handles: vec![],
    });
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &keys, &envelope_content(), 2, &plain)));
    let a = art(&r.doc);
    assert_eq!(live_envelope(&a[0]).4.bounds_margin, [0.0; 4]);
    // A stroke so thin that the saved geometric size also lies close to the stroked box.
    let thin = format!("{RED}0 1 1 0 1 0 0 XA\n0.2 w\n{}b\n", rect_path(40.0, 20.0, 160.0, 80.0));
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &keys, &thin, 2, &plain)));
    assert_eq!(live_envelope(&art(&r.doc)[0]).4.bounds_margin, [0.0; 4]);
    // A stroked straight line: 120 × 0 without its stroke, and 120 × 10 as saved.
    let keys = format!("{ENVELOPE_KEYS}10 /Real (AIDeformArtHeightKey) ,\n120 /Real (AIDeformArtWidthKey) ,\n");
    let line = format!("{RED}0 1 1 0 1 0 0 XA\n10 w\n40 20 m\n160 20 L\nS\n");
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &keys, &line, 2, &plain)));
    for n in art(&r.doc) {
        if let NodeKind::Envelope { options, .. } = &n.kind {
            assert_eq!(options.bounds_margin, [0.0; 4]);
        }
    }
}

/// An envelope's contents keep their groups: a group of two rectangles stays that group.
#[test]
fn an_envelope_s_contents_keep_their_groups() {
    use vectorcraft_doc::live::EnvelopeKind;
    let group = format!(
        "u\n{RED}{}f\n{}f\nU\n/ArtDictionary :\n(Pair) /String (AIArtName) ,\n;\n",
        rect_path(40.0, 20.0, 90.0, 80.0),
        rect_path(110.0, 20.0, 160.0, 80.0)
    );
    let mut drawn = RED.to_string();
    drawn += &format!("{}f\n{}f\n", rect_path(40.0, 20.0, 90.0, 80.0), rect_path(110.0, 20.0, 160.0, 80.0));
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &group, 2, &drawn)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let (content, kind, ..) = live_envelope(&a[0]);
    assert!(matches!(kind, EnvelopeKind::Mesh { rows: 1, cols: 1, .. }), "{kind:?}");
    assert_eq!(content.len(), 1, "{content:#?}");
    assert_eq!(content[0].name.as_deref(), Some("Pair"));
    let kids = content[0].children().unwrap();
    assert_eq!(kids.len(), 2, "{kids:#?}");
    assert!(kids.iter().all(|k| matches!(k.kind, NodeKind::Path { .. })), "{kids:#?}");
}

/// While an envelope's contents are edited, the contents come first and the mesh is in the
/// dictionary: the envelope opens live all the same, in Edit Contents.
#[test]
fn an_envelope_whose_contents_are_edited_opens_live_in_edit_contents() {
    use vectorcraft_doc::live::EnvelopeKind;
    let at = |(i, j): (usize, usize)| (40.0 + i as f64 * 120.0, 20.0 + j as f64 * 60.0);
    let handle = |p: (usize, usize), q: (usize, usize)| match (p, q) {
        ((0, 1), (1, 1)) => (70.0, 95.0),
        ((1, 1), (0, 1)) => (130.0, 95.0),
        _ => {
            let ((x, y), (u, v)) = (at(p), at(q));
            (x + (u - x) / 3.0, y + (v - y) / 3.0)
        }
    };
    let mesh = mesh_text(1, 1, &at, &handle, None);
    let mut bp = kurbo::BezPath::new();
    bp.move_to((40.0, 20.0));
    bp.curve_to((70.0, 5.0), (130.0, 5.0), (160.0, 20.0));
    bp.curve_to((160.0, 40.0), (160.0, 60.0), (160.0, 80.0));
    bp.curve_to((120.0, 80.0), (80.0, 80.0), (40.0, 80.0));
    bp.curve_to((40.0, 60.0), (40.0, 40.0), (40.0, 20.0));
    bp.close_path();
    let want = EnvelopeKind::TopObject { path: vectorcraft_geom::PathData::from_bezpath(&bp), corners: None };
    let keys = ENVELOPE_KEYS.replace("0 /Bool (AIDeformEditModeKey)", "1 /Bool (AIDeformEditModeKey)");
    let body = envelope_parts(&envelope_content(), &keys, &format!("0 g\n{mesh}"), 3, &drawn_content(&want));
    let r = read_ok(&layer(&body));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    let (content, kind, _, editing, _) = live_envelope(&a[0]);
    assert!(editing);
    assert!(matches!(kind, EnvelopeKind::TopObject { .. }), "{kind:?}");
    assert_eq!(content.len(), 1, "{content:#?}");
    assert!(matches!(content[0].kind, NodeKind::Path { .. }), "{content:#?}");
    assert_eq!(fill(&content[0]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
}

/// An envelope that doesn't read (a warp style number past the list, a mesh envelope whose shape
/// isn't a mesh, one whose plugin never closes it) keeps its drawn art, with one note; the art
/// round it is read as ever.
#[test]
fn an_envelope_that_doesnt_read_keeps_its_drawn_art() {
    let blue = "0 0 1 1 0 0 1 Xa\n";
    let drawn = format!("{blue}{}f\n", rect_path(40.0, 20.0, 160.0, 80.0));
    let mesh = straight_mesh(1, 1, (120.0, 60.0));
    let cases = [
        envelope_of(&mesh, &warp_keys(99, 0.5, 0.0, 0.0, 0), &envelope_content(), 1, &drawn),
        envelope_of(&format!("{}f\n", rect_path(40.0, 20.0, 160.0, 80.0)), ENVELOPE_KEYS, &envelope_content(), 2, &drawn),
        envelope_of(&mesh, ENVELOPE_KEYS, &envelope_content(), 7, &drawn),
        envelope_of(&mesh, &ENVELOPE_KEYS.replace("131073", "131081"), &envelope_content(), 2, &drawn),
        envelope_of(&mesh, ENVELOPE_KEYS, &envelope_content(), 2, &drawn).replace("(Adobe Deform Plugin) 1 0 0 XP\n", ""),
    ];
    for case in cases {
        let body = format!("{RED}{}f\n{case}{RED}{}f\n", rect_path(0.0, 0.0, 10.0, 10.0), rect_path(190.0, 0.0, 200.0, 10.0));
        let r = read_ok(&layer(&body));
        let a = art(&r.doc);
        assert_eq!(a.len(), 3, "{case}\n{a:#?}");
        assert!(matches!(a[0].kind, NodeKind::Path { .. }) && matches!(a[2].kind, NodeKind::Path { .. }), "{a:#?}");
        let NodeKind::Group { children, .. } = &a[1].kind else { panic!("{a:#?}") };
        assert_eq!(fill(&children[0]).color(), Some(Color::rgb(0.0, 0.0, 1.0)), "{case}");
        assert_eq!(r.warnings, [super::envelope::DRAWN], "{case}");
    }
}

/// An envelope whose contents don't come out as its drawn art (its warp bends them another way)
/// keeps the drawn art.
#[test]
fn an_envelope_that_doesnt_come_out_as_drawn_keeps_its_drawn_art() {
    use vectorcraft_doc::live::EnvelopeKind;
    let other = EnvelopeKind::Warp { style: "arc".into(), bend: -50.0, h: 0.0, v: 0.0, horizontal: true };
    let body = envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), &warp_keys(1, 0.5, 0.0, 0.0, 0), &envelope_content(), 1, &drawn_content(&other));
    let r = read_ok(&layer(&body));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert_eq!(r.warnings, [super::envelope::DRAWN]);
}

/// A document of `n` warped text objects, with the art the live warp makes of each, in its look.
fn warped_type_doc(n: usize) -> (Document, Vec<super::envelope::WarpedLook>) {
    use vectorcraft_doc::{Appearance, Effect, TextObject};
    let mut doc = Document::new(400.0, 200.0);
    let mut looks = vec![];
    let mut layer = Node::layer(doc.alloc_id(), "Art", vectorcraft_doc::LayerColor::Preset(0));
    for i in 0..n {
        let t = TextObject::point(kurbo::Point::new(20.0, 40.0 + 40.0 * i as f64), "TILT", Default::default());
        let mut text = Node::new(doc.alloc_id(), NodeKind::Text(Box::new(t)));
        let mut appearance = Appearance::default();
        appearance.effects.push(Effect { id: "warp.arc".into(), params: serde_json::json!({"bend": 40.0}), visible: true });
        text.appearance = appearance.into();
        text.name = Some(format!("Type {i}"));
        text.locked = i == 0;
        text.visible = i != 0;
        let mut look = vectorcraft_effects::expand_outlined(None, &text, None).unwrap();
        look.id = doc.alloc_id();
        looks.push(super::envelope::WarpedLook { slot: text.id, look, left_out: None });
        layer.children_mut().unwrap().push(Arc::new(text));
    }
    doc.layers = vec![Arc::new(layer)];
    (doc, looks)
}

/// Warped text objects share the import's budget of checks. One past it keeps its drawn art with its
/// text object's name and state, as one that fails the check does, and its box and wrap setting.
#[test]
fn warped_type_past_the_budget_comes_in_as_its_look() {
    let (mut doc, mut looks) = warped_type_doc(2);
    let (first, second) = (looks[0].slot, looks[1].slot);
    let left_out = Rect::new(1.0, 2.0, 3.0, 4.0);
    looks[1].left_out = Some(left_out);
    let wrap = vectorcraft_doc::TextWrap::default();
    doc.node_mut(second).unwrap().extras_mut().wrap = Some(wrap);
    let warped = super::envelope::WarpedType { looks, checks: 1, ..Default::default() };
    let mut restyled = vec![Rect::new(5.0, 6.0, 7.0, 8.0), left_out];
    assert_eq!(warped.settle(&mut doc, Some, &Default::default(), &mut restyled), 1);
    assert_eq!(restyled, [Rect::new(5.0, 6.0, 7.0, 8.0)]);
    assert!(matches!(doc.node(first).unwrap().kind, NodeKind::Text(_)));
    let kept = doc.node(second).unwrap();
    assert!(matches!(kept.kind, NodeKind::Group { .. } | NodeKind::Path { .. }), "{kept:#?}");
    assert_eq!((kept.name.as_deref(), kept.visible, kept.locked), (Some("Type 1"), true, false));
    assert_eq!(kept.extras().wrap, Some(wrap));
}

/// The budget for warped type is what the envelopes left: their checks and the page's share they
/// spent, with the page they are held to.
#[test]
fn warped_type_gets_what_the_envelopes_left_of_the_budget() {
    let flat = vectorcraft_doc::live::EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(40.0, 20.0), Point::new(160.0, 20.0), Point::new(40.0, 80.0), Point::new(160.0, 80.0)],
        handles: vec![],
    };
    let exact = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &envelope_content(), 2, &drawn_content(&flat))));
    let beside = format!("{RED}{}f\n", rect_path(40.5, 20.0, 160.5, 80.0));
    let shifted = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &envelope_content(), 2, &beside)));
    for r in [&exact, &shifted] {
        assert_eq!(r.warped_type.checks, super::envelope::MAX_CHECKED - 1);
        assert!(r.warped_type.page.is_some());
    }
    live_envelope(&art(&shifted.doc)[0]);
    assert!(shifted.warped_type.spent.any > exact.warped_type.spent.any);
}

/// In a file with an artboard list, warped type is held to the first artboard, not to the crop marks.
#[test]
fn warped_type_is_held_to_the_first_artboard() {
    let boards = "%_/Document :\n%_/Dictionary :\n%_/Array :\n\
                  %_/Dictionary :\n%_0 200 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_300 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
                  %_/Dictionary :\n%_400 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_600 -150 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_; ,\n\
                  %_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n";
    let r = read_ok(&format!("{boards}{}", layer(&format!("{RED}{}f\n", rect_path(20.0, 20.0, 40.0, 40.0)))));
    assert_eq!(r.doc.artboards[0].rect, Rect::new(0.0, 0.0, 300.0, 200.0));
    assert_eq!(r.warped_type.page, Some(Rect::new(0.0, 0.0, 300.0, 200.0)));
}

/// Type whose Warp is hidden draws no warp, so it stays type even with no check left.
#[test]
fn type_with_a_hidden_warp_stays_type() {
    let (mut doc, looks) = warped_type_doc(1);
    let slot = looks[0].slot;
    let mut appearance = (*doc.node(slot).unwrap().appearance).clone();
    appearance.effects[0].visible = false;
    doc.node_mut(slot).unwrap().appearance = appearance.into();
    let warped = super::envelope::WarpedType { looks, checks: 0, ..Default::default() };
    assert_eq!(warped.settle(&mut doc, Some, &Default::default(), &mut vec![]), 0);
    assert!(matches!(doc.node(slot).unwrap().kind, NodeKind::Text(_)));
}

/// A warped frame of threaded type stays type, whatever its look draws: the thread's frames share
/// one story. So does a slot that became no text object.
#[test]
fn warped_type_in_a_thread_stays_type() {
    let (mut doc, looks) = warped_type_doc(2);
    let (first, second) = (looks[0].slot, looks[1].slot);
    let warped = super::envelope::WarpedType { looks, ..Default::default() };
    assert_eq!(warped.settle(&mut doc, |id| (id != second).then_some(id), &[first].into(), &mut vec![]), 0);
    assert!(matches!(doc.node(first).unwrap().kind, NodeKind::Text(_)));
    assert!(matches!(doc.node(second).unwrap().kind, NodeKind::Text(_)));
}

/// What is left of the page's share of the budget decides, and the page's share counts only for
/// what the document shows.
#[test]
fn warped_type_on_the_page_spends_what_is_left_of_the_budget() {
    use crate::import::native::compare::{MAX_DIFFERENCE, MAX_SOLID_DIFFERENCE};
    let (doc, mut looks) = warped_type_doc(2);
    // The second object shows; the first is hidden. Each look draws a little more than its object.
    let mark = read_ok(&layer(&format!("{RED}{}f\n", rect_path(2.0, 2.0, 2.6, 2.6))));
    let mark = art(&mark.doc)[0].clone();
    for (i, w) in looks.iter_mut().enumerate() {
        w.look.visible = true;
        w.look = Node::group(vectorcraft_doc::NodeId(u64::MAX - 10 - i as u64), vec![Arc::new(w.look.clone()), mark.clone()]);
    }
    let copy = |w: &super::envelope::WarpedLook| super::envelope::WarpedLook { slot: w.slot, look: w.look.clone(), left_out: None };
    let page = Some(Rect::new(0.0, 60.0, 60.0, 100.0));
    let settle = |w: &super::envelope::WarpedLook, spent| {
        let warped = super::envelope::WarpedType { looks: vec![copy(w)], checks: 1, spent, page };
        warped.settle(&mut doc.clone(), Some, &Default::default(), &mut vec![])
    };
    let spent = || super::envelope::Spent { any: MAX_DIFFERENCE / 2.0, solid: MAX_SOLID_DIFFERENCE / 2.0 };
    assert_eq!(settle(&looks[1], Default::default()), 0);
    assert_eq!(settle(&looks[1], spent()), 1);
    assert_eq!(settle(&looks[0], spent()), 0);
}

/// An envelope that comes out close to its drawn art, but with more of the page differing than the
/// page check can take from the live envelopes (its drawn art 2 pt to the side on a 200 × 100
/// page), keeps the drawn art, so the file still opens from its layers.
#[test]
fn an_envelope_that_differs_from_its_drawn_art_over_the_page_keeps_it() {
    let shifted = format!("{RED}{}f\n", rect_path(42.0, 20.0, 162.0, 80.0));
    let body = envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &envelope_content(), 2, &shifted);
    let r = read_ok(&layer(&body));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert_eq!(r.warnings, [super::envelope::DRAWN]);
}

/// An envelope whose drawn art holds an object the reader can't read keeps that drawn art, which
/// the page's art replaces.
#[test]
fn an_envelope_whose_drawn_art_holds_an_unread_object_keeps_it() {
    let flat = vectorcraft_doc::live::EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(40.0, 20.0), Point::new(160.0, 20.0), Point::new(40.0, 80.0), Point::new(160.0, 80.0)],
        handles: vec![],
    };
    let drawn = format!("{}frobnicate\n", drawn_content(&flat));
    let r = read_ok(&layer(&envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &envelope_content(), 2, &drawn)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{a:#?}");
    assert_eq!(r.unread.iter().map(|(id, _)| *id).collect::<Vec<_>>(), [a[0].id]);
    assert_eq!(r.warnings, [super::envelope::DRAWN]);
}

/// A live envelope takes its drawn art's own properties: its name here.
#[test]
fn a_live_envelope_keeps_its_drawn_art_s_name() {
    let flat = vectorcraft_doc::live::EnvelopeKind::Mesh {
        rows: 1,
        cols: 1,
        points: vec![Point::new(40.0, 20.0), Point::new(160.0, 20.0), Point::new(40.0, 80.0), Point::new(160.0, 80.0)],
        handles: vec![],
    };
    let body = envelope_of(&straight_mesh(1, 1, (120.0, 60.0)), ENVELOPE_KEYS, &envelope_content(), 2, &drawn_content(&flat))
        .replace("U\n9 () XW\n(Adobe Deform Plugin)", "U\n/ArtDictionary :\n(Bent) /String (AIArtName) ,\n;\n9 () XW\n(Adobe Deform Plugin)");
    let r = read_ok(&layer(&body));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    live_envelope(&a[0]);
    assert_eq!(a[0].name.as_deref(), Some("Bent"));
}

/// The same envelope beside the page opens live: what differs off the page isn't counted.
#[test]
fn an_envelope_off_the_page_opens_live_when_it_draws_close_to_its_drawn_art() {
    let at = |(i, j): (usize, usize)| (300.0 + i as f64 * 120.0, 20.0 + j as f64 * 60.0);
    let third = |p: (usize, usize), q: (usize, usize)| {
        let ((x, y), (u, v)) = (at(p), at(q));
        (x + (u - x) / 3.0, y + (v - y) / 3.0)
    };
    let content = format!("{RED}{}f\n", rect_path(300.0, 20.0, 420.0, 80.0));
    let shifted = format!("{RED}{}f\n", rect_path(302.0, 20.0, 422.0, 80.0));
    let r = read_ok(&layer(&envelope_of(&mesh_text(1, 1, &at, &third, None), ENVELOPE_KEYS, &content, 2, &shifted)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    live_envelope(&a[0]);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn a_gradient_mesh_reads_as_a_patch() {
    let corner = |rgb: &str, x: i32, y: i32, next: (i32, i32), prev: (i32, i32)| {
        format!("%_[{rgb} 1 {x} {y} {} {} 1 {} {} 1] /N X#\n", next.0, next.1, prev.0, prev.1)
    };
    let mesh = |space: &str| {
        format!(
            "/Mesh X!\n%_({space}) /CS X#\n%_[0 0] /P X#\n{}{}{}{}%_0 /E X#\n/End X!\n",
            corner("1 0 0", 0, 0, (0, 10), (10, 0)),
            corner("0 1 0", 0, 40, (10, 40), (0, 30)),
            corner("0 0 1", 40, 40, (40, 30), (30, 40)),
            corner("1 1 1", 40, 0, (30, 0), (40, 10)),
        )
    };
    let doc = read_ok(&layer(&mesh("DeviceRGB"))).doc;
    let NodeKind::Mesh(m) = &art(&doc)[0].kind else { panic!("{:?}", art(&doc)[0].kind) };
    let colors: Vec<_> = m.points.iter().map(|p| p.color).collect();
    assert_eq!(colors, [Color::rgb(1.0, 0.0, 0.0), Color::rgb(1.0, 1.0, 1.0), Color::rgb(0.0, 1.0, 0.0), Color::rgb(0.0, 0.0, 1.0)]);
    use vectorcraft_doc::live::{H_DOWN, H_RIGHT};
    let first = &m.points[0];
    assert_eq!(
        (first.p, first.handles[H_DOWN], first.handles[H_RIGHT]),
        (Point::new(0.0, 100.0), kurbo::Vec2::new(0.0, -10.0), kurbo::Vec2::new(10.0, 0.0))
    );
    let e = read(stream(&layer(&mesh("Separation"))).as_bytes()).unwrap_err();
    assert!(e.contains("gradient meshes"), "{e}");
}

/// Both mesh-corner layouts must keep colour, position and handles, rather than guessing
/// opacity from a coordinate. Unsupported weights and truncated corners remain errors.
#[test]
fn mesh_corners_with_and_without_opacity() {
    let geometry = ["10 20 10 25 1 15 20 1", "10 50 15 50 1 10 45 1", "40 50 40 45 1 35 50 1", "40 20 35 20 1 40 25 1"];
    for (space, colour) in [("DeviceRGB", "0.2 0.4 0.6"), ("DeviceCMYK", "0.1 0.2 0.3 0.4"), ("DeviceGray", "0.5")] {
        let mesh = |opacity: &str| {
            let corners: String = geometry.iter().map(|g| format!("%_[{colour} {opacity}{g}] /N X#\n")).collect();
            format!("/Mesh X!\n%_/{space} /CS X#\n{corners}%_/E X#\n/End X!\n")
        };
        let old = art(&read_ok(&layer(&mesh(""))).doc);
        let new = art(&read_ok(&layer(&mesh("1 "))).doc);
        assert_eq!(old[0].kind, new[0].kind);
        let translucent = art(&read_ok(&layer(&mesh("0.25 "))).doc);
        let NodeKind::Mesh(m) = &translucent[0].kind else { panic!("mesh") };
        assert!(m.points.iter().all(|p| p.opacity == 0.25));
        assert_eq!(m.points[0].p, Point::new(10.0, 80.0));
        for broken in [mesh("").replace("10 25 1", "10 25 0.5"), mesh("").replace("15 20 1]", "15 20]")] {
            assert!(read(stream(&layer(&broken)).as_bytes()).unwrap_err().contains("gradient meshes"));
        }
    }
}

#[test]
fn vector_opacity_mask_keeps_its_art_and_does_not_paint_as_a_sibling() {
    let owner = format!("{RED}{}f\n", rect_path(10.0, 20.0, 50.0, 60.0));
    let mask = "%_/ArtDictionary :\n%_/Mask :\n%_1 /Bool (Linked) ,\n%_X=\n%_u\n%_1 g\n%_10 20 m\n%_10 60 L\n%_30 60 L\n%_30 20 L\n%_f\n%_U\n%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_0 /Bool (Inverted) ,\n%_0 /Bool (Disabled) ,\n%_; (AI9_Mask) ,\n%_;\n";
    let following = format!("{}f\n", rect_path(70.0, 20.0, 80.0, 30.0));
    for (invert, disabled, linked) in [(false, false, true), (true, false, false), (false, true, true)] {
        let mask = mask
            .replace("0 /Bool (Inverted)", &format!("{} /Bool (Inverted)", u8::from(invert)))
            .replace("0 /Bool (Disabled)", &format!("{} /Bool (Disabled)", u8::from(disabled)))
            .replace("1 /Bool (Linked)", &format!("{} /Bool (Linked)", u8::from(linked)));
        let r = read_ok(&layer(&format!("{owner}{mask}{following}")));
        assert!(r.warnings.is_empty());
        let a = art(&r.doc);
        assert_eq!(a.len(), 2);
        assert_eq!(fill(&a[1]).color(), Some(Color::rgb(1.0, 0.0, 0.0)), "mask colour must not leak");
        let m = a[0].extras().mask.as_ref().unwrap();
        assert_eq!((m.clip, m.invert, m.disabled, m.linked), (true, invert, disabled, linked));
        let mask_path = &m.art.children().unwrap()[0];
        assert!(matches!(mask_path.kind, NodeKind::Path { .. }));
        assert_eq!(mask_path.geometric_bounds(), Some(Rect::new(10.0, 40.0, 30.0, 80.0)));
        assert_ne!(mask_path.id, a[0].id);
    }
    for broken in [mask.replace("%_X+\n", ""), mask.replace("%_U\n", ""), mask.replace("%_f", "%_unknownMaskOperator")] {
        assert!(read(stream(&layer(&format!("{owner}{broken}"))).as_bytes()).is_err());
    }
}

/// An opacity mask's art of one shown object is that object; other art is read into a group.
#[test]
fn opacity_mask_art_of_one_object_is_that_object() {
    let owner = format!("{RED}{}f\n", rect_path(10.0, 20.0, 50.0, 60.0));
    let square = "%_1 g\n%_10 20 m\n%_10 60 L\n%_30 60 L\n%_30 20 L\n%_f\n";
    let read_art = |ops: &str| {
        let mask = format!("%_/ArtDictionary :\n%_/Mask :\n%_X=\n{ops}%_X+\n%_(Art) ,\n%_1 /Bool (Clipping) ,\n%_; (AI9_Mask) ,\n%_;\n");
        let r = read_ok(&layer(&format!("{owner}{mask}")));
        art(&r.doc)[0].extras().mask.as_ref().unwrap().art.clone()
    };
    assert!(matches!(read_art(square).kind, NodeKind::Path { .. }));
    // A group is one object: it stays a group, holding its path.
    let grouped = read_art(&format!("%_u\n{square}%_U\n"));
    let NodeKind::Group { children, clip: false } = &grouped.kind else { panic!("{:?}", grouped.kind) };
    assert!(matches!(children.as_slice(), [p] if matches!(p.kind, NodeKind::Path { .. })));
    // Two objects are held in a group.
    let two = read_art(&format!("{square}{square}"));
    assert!(matches!(&two.kind, NodeKind::Group { children, clip: false } if children.len() == 2), "{:?}", two.kind);
    // A clipping path alone keeps its group, which clips.
    let clipped = read_art("%_10 20 m\n%_10 60 L\n%_30 60 L\n%_30 20 L\n%_h\n%_W\n%_n\n");
    assert!(matches!(&clipped.kind, NodeKind::Group { children, clip: true } if children.len() == 1), "{:?}", clipped.kind);
    // A hidden object stays in a group, which doesn't draw it.
    let hidden = read_art(&format!("%_1 Xw\n{square}%_0 Xw\n"));
    let NodeKind::Group { children, clip: false } = &hidden.kind else { panic!("{:?}", hidden.kind) };
    assert!(matches!(children.as_slice(), [p] if !p.visible && matches!(p.kind, NodeKind::Path { .. })));
    // So does a guide, which a group leaves out when guides aren't drawn.
    let guide = read_art("%_10 20 m\n%_30 60 L\n%_(N) *\n");
    let NodeKind::Group { children, clip: false } = &guide.kind else { panic!("{:?}", guide.kind) };
    assert!(matches!(children.as_slice(), [p] if matches!(p.kind, NodeKind::Path { guide: true, .. })));
}

/// `9 () XW` also follows legacy compound member groups. The final gradient paints
/// every member; the initial black paint is a placeholder, not a separate black object.
#[test]
fn winding_declared_compound_members_keep_the_final_gradient() {
    let defs = "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n0 0 0 0 1 0 0 2 50 0 Bs\n0 0 0 0 0 0 1 2 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let body = format!(
        "*u\nu\n1 D\n0 g\n{}f\n0 D\n{}Bb\n1 (G) 0 0 0 100 1 0 0 1 0 0 Bg\nf\n0 BB\nU\n9 () XW\n*U\n",
        rect_path(10.0, 10.0, 80.0, 80.0),
        rect_path(30.0, 30.0, 50.0, 50.0)
    );
    let r = read_ok(&format!("{defs}{}", layer(&body)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1);
    let NodeKind::Compound { children, .. } = &a[0].kind else { panic!("compound, not separately painted look") };
    assert_eq!(children.len(), 2);
    assert!(matches!(fill(&a[0]), Paint::Gradient(g) if g.gradient.stops.len() == 2));
    assert!(children.iter().all(|n| matches!(n.kind, NodeKind::Path { .. })));
    // Without winding declarations this is still an independently painted appearance group.
    let r = read_ok(&format!("{defs}{}", layer(&body.replace("1 D\n", "").replace("0 D\n", ""))));
    assert!(matches!(art(&r.doc)[0].kind, NodeKind::Group { .. }));
}

/// #779: path guides from Illustrator's `(op) *` operator (AI7 §6.1) import as guide paths.
#[test]
fn path_guides_from_the_star_operator() {
    let line = |x0: f64, y0: f64, x1: f64, y1: f64| format!("{x0} {y0} m\n{x1} {y1} L\n(N) *\n");
    let body = format!("{RED}{}f\n{}{}", rect_path(0.0, 0.0, 50.0, 50.0), line(10.0, 20.0, 90.0, 20.0), "(N) *\n15 10 m\n15 80 L\n(N) *\n",);
    let kids = art(&read_ok(&layer(&body)).doc);
    assert_eq!(kids.len(), 3);
    let guides = kids.iter().filter(|n| matches!(n.kind, NodeKind::Path { guide: true, .. })).count();
    assert_eq!(guides, 2, "the two construction lines are guide paths");
    assert!(matches!(kids[0].kind, NodeKind::Path { guide: false, .. }));
    for g in kids.iter().filter(|n| matches!(n.kind, NodeKind::Path { guide: true, .. })) {
        assert!(g.appearance.items.is_empty(), "an (N) guide is non-printing geometry with no paint");
    }
}

/// Legacy version 7 writes `(N) *` before and after the path (§6).
#[test]
fn path_guides_wrapped_before_and_after_the_path() {
    let body = "(N) *\n20 30 m\n80 30 L\n(N) *\n";
    let kids = art(&read_ok(&layer(body)).doc);
    assert_eq!(kids.len(), 1);
    assert!(matches!(kids[0].kind, NodeKind::Path { guide: true, .. }));
}

/// A stroked guide keeps the stroke for display but stays a guide path.
#[test]
fn stroked_path_guides_from_the_star_operator() {
    let body = "1 w\n0 0 0 1 k\n10 10 m\n90 10 L\n(S) *\n";
    let kids = art(&read_ok(&layer(body)).doc);
    assert_eq!(kids.len(), 1);
    let n = &kids[0];
    assert!(matches!(n.kind, NodeKind::Path { guide: true, .. }));
    assert_eq!(n.appearance.items.len(), 1);
    assert!(matches!(n.appearance.items[0], AppearanceItem::Stroke(_)));
}

/// [`stream`] with a template box centred on (306.5, 395.5): the canvas runs 8,191.5 points each
/// way from there.
fn canvas_stream(body: &str) -> String {
    stream(body).replacen("%%EndComments", "%AI3_TemplateBox: 306.5 395.5 306.5 395.5\n%%EndComments", 1)
}

/// The straight guide path from (`x0`, `y0`) to (`x1`, `y1`), `(op) *`.
fn guide_line(x0: f64, y0: f64, x1: f64, y1: f64, op: &str) -> String {
    format!("{x0} {y0} m\n{x1} {y1} L\n({op}) *\n")
}

/// #779: a straight `(N) *` guide path across the whole canvas, directly on a layer or sublayer, is
/// a ruler guide on that layer, at its place in the document; it leaves no path behind.
#[test]
fn canvas_wide_guide_paths_are_ruler_guides_of_their_layer() {
    let body = format!(
        "{}{}{RED}{}f\n%AI5_BeginLayer\n1 1 1 1 0 0 1 0 79 128 255 0 50 0 Lb\n(Sub) Ln\n{}LB\n%AI5_EndLayer--\n",
        guide_line(-7885.0, 40.0, 8498.0, 40.0, "N"),
        guide_line(25.0, 8587.0, 25.0, -7796.0, "N"),
        rect_path(0.0, 0.0, 10.0, 10.0),
        guide_line(-7885.0, 90.0, 8498.0, 90.0, "N"),
    );
    let doc = read(canvas_stream(&layer(&body)).as_bytes()).unwrap().doc;
    let top = &doc.layers[0];
    let kids = top.children().unwrap();
    assert_eq!(kids.len(), 2, "the square and the sublayer");
    assert!(matches!(kids[0].kind, NodeKind::Path { guide: false, .. }));
    let sub = &kids[1];
    assert!(sub.children().unwrap().is_empty());
    let want = [
        Guide { vertical: false, pos: 60.0, artboard: None, layer: Some(top.id) },
        Guide { vertical: true, pos: 25.0, artboard: None, layer: Some(top.id) },
        Guide { vertical: false, pos: 10.0, artboard: None, layer: Some(sub.id) },
    ];
    assert_eq!(doc.guides, want);
}

/// #779: a straight `(N) *` guide path from edge to edge of one artboard is a ruler guide of that
/// artboard.
#[test]
fn artboard_wide_guide_paths_are_ruler_guides_of_their_artboard() {
    // The crop marks' artboard, 0 0 200 100.
    let doc = read(canvas_stream(&layer(&guide_line(0.0, 30.0, 200.0, 30.0, "N"))).as_bytes()).unwrap().doc;
    assert_eq!(doc.guides, [Guide { vertical: false, pos: 70.0, artboard: Some(1), layer: Some(doc.layers[0].id) }]);
    assert!(art(&doc).is_empty());
    // The second of two artboards: art space x 400 to 600, y -150 to 0.
    let boards = "%_/Document :\n%_/Dictionary :\n%_/Array :\n\
                  %_/Dictionary :\n%_0 200 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_300 0 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
                  %_/Dictionary :\n%_400 0 /RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_600 -150 /RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_; ,\n\
                  %_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n";
    let doc = read_ok(&format!("{boards}{}", layer(&guide_line(500.0, 0.0, 500.0, -150.0, "N")))).doc;
    assert_eq!(doc.artboards[1].id, 2);
    assert_eq!(doc.guides, [Guide { vertical: true, pos: 500.0, artboard: Some(2), layer: Some(doc.layers[0].id) }]);
}

/// Guide paths that aren't ruler guides stay guide paths: `(S) *` ones (guides made from objects),
/// even across the whole canvas or an artboard, short ones, slanted ones, and ones in a group.
#[test]
fn other_guide_paths_stay_guide_paths() {
    let cases = [
        guide_line(-7885.0, 40.0, 8498.0, 40.0, "S"),
        guide_line(20.0, 40.0, 80.0, 40.0, "S"),
        guide_line(20.0, 40.0, 80.0, 40.0, "N"),
        guide_line(0.0, 30.0, 200.0, 30.0, "S"),
        guide_line(-7885.0, -7796.0, 8498.0, 8587.0, "N"),
        format!("u\n{}U\n", guide_line(-7885.0, 40.0, 8498.0, 40.0, "N")),
        format!("1 Xw\n{}0 Xw\n", guide_line(-7885.0, 40.0, 8498.0, 40.0, "N")),
        "-7885 40 m\n306 40 L\n8498 40 L\n(N) *\n".to_string(),
    ];
    for body in cases {
        let doc = read(canvas_stream(&layer(&body)).as_bytes()).unwrap().doc;
        assert!(doc.guides.is_empty(), "{body}");
        assert_eq!(art(&doc).len(), 1, "{body}");
    }
}

/// The layer whose `Lb` has `ops` as its options (the first one in the file).
fn layer_with(ops: &str) -> Node {
    let doc = read_ok(&format!("%AI5_BeginLayer\n{ops} Lb\n(L) Ln\nLB\n%AI5_EndLayer--\n")).doc;
    Node::clone(&doc.layers[0])
}

/// #779: the template flag four places after the colour index makes a template layer; locking,
/// printing and dimming keep their own flags. In thirteen and fourteen operands the flag before
/// the colour index and the last one change nothing.
#[test]
fn the_template_flag_of_a_layer() {
    let l = layer_with("1 1 0 0 1 0 0 4 255 79 255 1 50 0");
    let NodeKind::Layer { template, printable, dim_images, color, .. } = l.kind else { panic!() };
    assert!(template && !printable && l.locked);
    assert_eq!((dim_images, color), (Some(50), LayerColor::Preset(4)));
    for ops in ["1 1 1 1 1 0 1 5 1 2 3 1 30 0", "1 1 1 1 1 0 0 5 1 2 3 1 30 1", "1 1 1 1 1 0 1 5 1 2 3 1 30"] {
        let l = layer_with(ops);
        let NodeKind::Layer { template, printable, dim_images, color, .. } = l.kind else { panic!() };
        assert!(template && printable && !l.locked, "{ops}");
        assert_eq!((dim_images, color), (Some(30), LayerColor::Preset(5)), "{ops}");
    }
    let l = layer_with("1 1 1 1 0 0 1 0 79 128 255 0 50 0");
    assert!(matches!(l.kind, NodeKind::Layer { template: false, .. }));
}

/// The colour index is one place earlier in eleven and twelve operands, as in ten, and the twelfth
/// is the dimming percentage; with nine it is one place earlier still.
#[test]
fn layer_options_of_nine_to_twelve_operands() {
    let options = |l: Node| match l.kind {
        NodeKind::Layer { template, dim_images, color, .. } => (template, dim_images, color),
        _ => panic!(),
    };
    assert_eq!(options(layer_with("1 1 1 1 1 0 3 1 2 3 1")), (true, Some(50), LayerColor::Preset(3)));
    assert_eq!(options(layer_with("1 1 1 1 1 0 3 1 2 3 0 25")), (false, Some(25), LayerColor::Preset(3)));
    assert_eq!(options(layer_with("1 1 1 1 1 0 -1 10 20 30 1 25")), (true, Some(25), LayerColor::Custom([10, 20, 30])));
    assert_eq!(options(layer_with("1 1 1 1 0 0 3 1 2 3")), (false, None, LayerColor::Preset(3)));
    assert_eq!(options(layer_with("1 1 1 1 0 2 1 2 3")), (false, None, LayerColor::Preset(2)));
}

/// A blend's record as the editing data writes it after `(Adobe Path Blends) 1 0 n XP`: `DCBA`,
/// Specified Steps, then (as far as the reader reads it) the number of keys at word 10 and a record
/// of six words per key starting with its id. Hand-written; the other words are zero.
pub(super) fn blend_record(ids: &[char]) -> String {
    blend_record_spaced(ids, 1, 0.0, 0)
}

/// [`blend_record`] with the spacing `spacing` at word 1 (0 Smooth Color, 1 Specified Steps, 2
/// Specified Distance), the distance `distance` at word 2 and `steps` at word 4.
fn blend_record_spaced(ids: &[char], spacing: u32, distance: f32, steps: u32) -> String {
    let mut words: Vec<u32> = vec![u32::from_le_bytes(*b"DCBA"), spacing, distance.to_bits(), 0, steps, 0, 0, 0, 0, 0, ids.len() as u32];
    for id in ids {
        words.extend([*id as u32, 0, 0, 0, 0, 0]);
    }
    let hex: String = words.iter().flat_map(|w| w.to_le_bytes()).map(|b| format!("{b:02X}")).collect();
    let lines: Vec<String> = hex.as_bytes().chunks(60).map(|c| format!("%{}", String::from_utf8_lossy(c))).collect();
    format!("(Adobe Path Blends) 1 0 {} XP\n{}\n", hex.len() / 2, lines.join("\n"))
}

/// A square `s` wide with its lower left at (x, y), filled with RGB `rgb`, as `%_` lines when
/// `hidden`.
fn square(x: f64, y: f64, s: f64, rgb: [f64; 3], hidden: bool) -> String {
    let p = if hidden { "%_" } else { "" };
    format!("{p}0 0 0 0 {} {} {} Xa\n{p}{x} {y} m\n{p}{x} {} L\n{p}{} {} L\n{p}{} {y} L\n{p}f\n", rgb[0], rgb[1], rgb[2], y + s, x + s, y + s, x + s)
}

/// The dictionary naming a blend part: type `kind` (`K`, `S`) and, for a key, `id`.
fn blend_part(kind: &str, id: Option<&str>) -> String {
    let id = id.map(|i| format!("%_({i}) /String (ShapeBlendPathID) ,\n")).unwrap_or_default();
    format!("%_/ArtDictionary :\n{id}%_({kind}) /String (ShapeBlendPathType) ,\n%_;\n")
}

/// A blend of two 20 pt squares, red on the left (key `A`) and blue on the right (`B`), along
/// `spine` (path operators, `%_` lines), with its drawn steps (three between the keys) and the
/// record `record`.
pub(super) fn blend_file(spine: &str, ids: [Option<&str>; 2], record: &str) -> String {
    blend_file_colored(spine, ids, record, [1.0, 0.0, 0.0], [0.0, 0.0, 1.0])
}

/// [`blend_file`] with the left key in RGB `from`, the right one in `to`, and the steps drawn
/// between their colors.
fn blend_file_colored(spine: &str, ids: [Option<&str>; 2], record: &str, from: [f64; 3], to: [f64; 3]) -> String {
    blend_file_apart(spine, ids, record, from, to, 100.0, 3)
}

/// [`blend_file_colored`] with the right key `apart` points right of the left one and `between`
/// steps drawn between them.
fn blend_file_apart(spine: &str, ids: [Option<&str>; 2], record: &str, from: [f64; 3], to: [f64; 3], apart: f64, between: usize) -> String {
    let mut art = String::from("%_0 Ae\n%_u\n");
    art += &square(0.0, 40.0, 20.0, from, true);
    art += &blend_part("K", ids[0]);
    art += &square(apart, 40.0, 20.0, to, true);
    art += &blend_part("K", ids[1]);
    art += spine;
    art += "%_N\n";
    art += &blend_part("S", None);
    art += "%_U\n%_9 () XW\nu\n";
    for i in 0..between + 2 {
        let t = i as f64 / (between + 1) as f64;
        let rgb = [0, 1, 2].map(|c| from[c] + (to[c] - from[c]) * t);
        art += &square(t * apart, 40.0, 20.0, rgb, false);
    }
    art += "U\n9 () XW\n0 Ae\n";
    art += record;
    art
}

/// The blend read from `body`: the first object of the layer.
fn read_blend(body: &str) -> (Arc<Node>, Vec<String>) {
    let s = read_ok(&layer(body));
    let first = art(&s.doc).into_iter().next().expect("an object");
    (first, s.warnings)
}

/// Two keys, a straight spine and a record: one live blend of the two keys, three steps between
/// them (as drawn), its colours running from red to blue, the drawn steps and the blend's own art
/// gone.
#[test]
fn a_blend_comes_in_as_a_live_blend() {
    let body = blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B']));
    let s = read_ok(&layer(&body));
    let objects = art(&s.doc);
    assert_eq!(objects.len(), 1, "one object: {objects:?}");
    let NodeKind::Blend { children, spec } = &objects[0].kind else { panic!("not a blend: {:?}", objects[0].kind) };
    assert_eq!(children.len(), 2);
    assert_eq!(spec.spacing, vectorcraft_doc::live::BlendSpacing::Steps(3));
    let laid = vectorcraft_doc::blend::blend_expand(children, spec);
    assert_eq!(laid.len(), 5);
    let middle = laid.iter().find(|n| n.geometric_bounds().is_some_and(|b| (b.center().x - 60.0).abs() < 0.5)).expect("the middle step");
    let Paint::Solid { color, .. } = fill(middle) else { panic!("solid") };
    let [r, _, b] = color.to_rgb();
    assert!((r - 0.5).abs() < 0.02 && (b - 0.5).abs() < 0.02, "{r} {b}");
    assert!(!s.warnings.iter().any(|w| w == super::blend::DRAWN), "{:?}", s.warnings);
}

/// A blend whose spine was never edited has no spine written, only its keys: its spine is the
/// straight line between the keys' centres, in the record's order (here the left key, named `B`,
/// first), and the blend comes in live.
#[test]
fn a_blend_with_no_spine_written_runs_straight_between_its_keys() {
    let spine_part = format!("%_N\n{}", blend_part("S", None));
    let body = blend_file("", [Some("B"), Some("A")], &blend_record(&['B', 'A']));
    assert!(body.contains(&spine_part));
    let (blend, warnings) = read_blend(&body.replace(&spine_part, ""));
    let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a live blend: {:?} {warnings:?}", blend.kind) };
    assert!(!warnings.iter().any(|w| w == super::blend::DRAWN), "{warnings:?}");
    assert_eq!(spec.spacing, vectorcraft_doc::live::BlendSpacing::Steps(3));
    let spine = spec.spine.as_ref().and_then(|p| p.subpaths.first()).expect("a spine");
    let points: Vec<(f64, f64)> = spine.anchors.iter().map(|a| (a.p.x, a.p.y)).collect();
    assert_eq!(points.len(), 2, "{points:?}");
    assert!((points[0].0 - 10.0).abs() < 0.01 && (points[1].0 - 110.0).abs() < 0.01, "from the left key to the right one: {points:?}");
    assert_eq!(spec.key_anchors, vec![0, 1]);
    assert_eq!(vectorcraft_doc::blend::blend_expand(children, spec).len(), 5);
}

/// The tags after a blend's drawn steps are the live blend's.
#[test]
fn a_live_blend_keeps_the_tags_of_its_drawn_steps() {
    let body = blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B']));
    let (blend, warnings) = read_blend(&body.replace("\nU\n9 () XW\n0 Ae\n", "\nU\n/sku (B-1) XT\n9 () XW\n0 Ae\n"));
    assert!(matches!(blend.kind, NodeKind::Blend { .. }), "a live blend: {:?} {warnings:?}", blend.kind);
    assert_eq!(tags(&blend).1, [tag("sku", "B-1")]);
}

/// A spine written from the right key to the left one keeps the keys in their order on it (the
/// red key first), sitting on its last and first anchors.
#[test]
fn a_blends_spine_written_backwards_keeps_its_keys_order() {
    let (blend, warnings) = read_blend(&blend_file("%_110 50 m\n%_10 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B'])));
    let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a live blend: {:?} {warnings:?}", blend.kind) };
    assert!(matches!(fill(&children[0]), Paint::Solid { color, .. } if color.to_rgb()[0] > 0.9), "the first key is the red one");
    assert_eq!(spec.key_anchors, vec![1, 0], "the red key on the spine's end, the blue one on its start");
}

/// A blend's spacing comes from its record: Specified Distance with its distance (25 pt from one
/// step's edge to the corresponding edge of the next lays three out between keys 100 pt apart),
/// Smooth Color (keys 3/255 apart in blue take three steps), and Specified Steps with the steps
/// counted from the drawn ones, whatever its record's word 4 holds. The distance counts along the
/// spine between the keys, also where the spine runs on past the last key.
#[test]
fn a_blend_keeps_its_spacing() {
    use vectorcraft_doc::live::BlendSpacing;
    let straight = "%_10 50 m\n%_110 50 L\n";
    let longer = "%_10 50 m\n%_110 50 L\n%_1010 50 L\n";
    let (red, nearly_red, blue) = ([1.0, 0.0, 0.0], [1.0, 0.0, 3.0 / 255.0], [0.0, 0.0, 1.0]);
    for (spine, record, (from, to), spacing) in [
        (straight, blend_record_spaced(&['A', 'B'], 2, 25.0, 3), (red, blue), BlendSpacing::Distance(25.0)),
        (longer, blend_record_spaced(&['A', 'B'], 2, 25.0, 3), (red, blue), BlendSpacing::Distance(25.0)),
        (straight, blend_record_spaced(&['A', 'B'], 0, 4.0, 3), (red, nearly_red), BlendSpacing::SmoothColor),
        (straight, blend_record_spaced(&['A', 'B'], 1, 25.0, 3), (red, blue), BlendSpacing::Steps(3)),
        (straight, blend_record_spaced(&['A', 'B'], 1, 12.5, 7), (red, blue), BlendSpacing::Steps(3)),
    ] {
        let (blend, warnings) = read_blend(&blend_file_colored(spine, [Some("A"), Some("B")], &record, from, to));
        let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a live blend for {spacing:?}: {:?} {warnings:?}", blend.kind) };
        assert_eq!(spec.spacing, spacing);
        assert_eq!(vectorcraft_doc::blend::blend_expand(children, spec).len(), 5, "{spacing:?}");
        assert!(!warnings.iter().any(|w| w == super::blend::DRAWN), "{spacing:?}: {warnings:?}");
    }
}

/// Smooth Color blends of two 20 pt squares 200 pt apart come in live with the steps drawn
/// between them: 254 from red to blue, and 9 between two red squares.
#[test]
fn a_smooth_color_blend_comes_in_with_its_drawn_steps() {
    use vectorcraft_doc::live::BlendSpacing;
    let (red, blue) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
    for (to, between) in [(blue, 254), (red, 9)] {
        let record = blend_record_spaced(&['A', 'B'], 0, 4.0, between as u32);
        let body = blend_file_apart("%_10 50 m\n%_210 50 L\n", [Some("A"), Some("B")], &record, red, to, 200.0, between);
        let (blend, warnings) = read_blend(&body);
        let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a live blend for {between} steps: {:?} {warnings:?}", blend.kind) };
        assert_eq!(spec.spacing, BlendSpacing::SmoothColor);
        assert_eq!(vectorcraft_doc::blend::blend_expand(children, spec).len(), between + 2);
        assert!(!warnings.iter().any(|w| w == super::blend::DRAWN), "{between}: {warnings:?}");
    }
}

/// A blend whose spacing doesn't lay out its drawn steps keeps them: a distance that lays out
/// nine, and Smooth Color between red and blue (more steps than the three drawn); and so does a
/// spacing the reader doesn't know or a distance that isn't a positive number.
#[test]
fn a_blend_whose_spacing_doesnt_lay_out_its_steps_keeps_them() {
    let straight = "%_10 50 m\n%_110 50 L\n";
    for record in [
        blend_record_spaced(&['A', 'B'], 2, 10.0, 9),
        blend_record_spaced(&['A', 'B'], 0, 4.0, 254),
        blend_record_spaced(&['A', 'B'], 3, 25.0, 3),
        blend_record_spaced(&['A', 'B'], 2, 0.0, 3),
        blend_record_spaced(&['A', 'B'], 2, f32::NAN, 3),
        blend_record_spaced(&['A', 'B'], 2, 1e-30, 3),
    ] {
        let (blend, warnings) = read_blend(&blend_file(straight, [Some("A"), Some("B")], &record));
        assert!(!matches!(blend.kind, NodeKind::Blend { .. }), "{record}: {:?}", blend.kind);
        assert_eq!(blend.children().map(|c| c.len()), Some(5), "the five drawn squares");
        assert!(warnings.iter().any(|w| w == super::blend::DRAWN), "{warnings:?}");
    }
}

/// Three keys with Specified Distance, unevenly spaced on a straight spine (red at x 0, blue at
/// 100, green at 150, 20 pt squares): 25 pt lays three steps out between the first two keys and
/// one between the last two, and a blend drawn that way comes in live. The same seven objects
/// drawn two and two between the keys keep their drawn steps.
#[test]
fn a_blend_of_three_keys_keeps_its_distance_between_each_pair() {
    use vectorcraft_doc::live::BlendSpacing;
    let (red, blue, green) = ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]);
    let mix = |a: [f64; 3], b: [f64; 3], t: f64| [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * t);
    let file = |drawn: &[(f64, [f64; 3])]| {
        let mut art = String::from("%_0 Ae\n%_u\n");
        for (x, rgb, id) in [(0.0, red, "A"), (100.0, blue, "B"), (150.0, green, "C")] {
            art += &square(x, 40.0, 20.0, rgb, true);
            art += &blend_part("K", Some(id));
        }
        art += "%_10 50 m\n%_110 50 L\n%_160 50 L\n%_N\n";
        art += &blend_part("S", None);
        art += "%_U\n%_9 () XW\nu\n";
        for &(x, rgb) in drawn {
            art += &square(x, 40.0, 20.0, rgb, false);
        }
        art += "U\n9 () XW\n0 Ae\n";
        art += &blend_record_spaced(&['A', 'B', 'C'], 2, 25.0, 0);
        art
    };
    let three_and_one = [
        (0.0, red),
        (25.0, mix(red, blue, 0.25)),
        (50.0, mix(red, blue, 0.5)),
        (75.0, mix(red, blue, 0.75)),
        (100.0, blue),
        (125.0, mix(blue, green, 0.5)),
        (150.0, green),
    ];
    let (blend, warnings) = read_blend(&file(&three_and_one));
    let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a live blend: {:?} {warnings:?}", blend.kind) };
    assert_eq!(spec.spacing, BlendSpacing::Distance(25.0));
    assert_eq!(vectorcraft_doc::blend::blend_steps(children, spec), vec![3, 1]);
    let two_and_two = [
        (0.0, red),
        (100.0 / 3.0, mix(red, blue, 1.0 / 3.0)),
        (200.0 / 3.0, mix(red, blue, 2.0 / 3.0)),
        (100.0, blue),
        (350.0 / 3.0, mix(blue, green, 1.0 / 3.0)),
        (400.0 / 3.0, mix(blue, green, 2.0 / 3.0)),
        (150.0, green),
    ];
    let (blend, warnings) = read_blend(&file(&two_and_two));
    assert!(!matches!(blend.kind, NodeKind::Blend { .. }), "{:?}", blend.kind);
    assert_eq!(blend.children().map(|c| c.len()), Some(7), "the seven drawn squares");
    assert!(warnings.iter().any(|w| w == super::blend::DRAWN), "{warnings:?}");
}

/// A curved spine (a line, a curve and a line) is read with its points exactly; a blend whose drawn
/// steps don't follow it (they are drawn on a straight line here) keeps its drawn steps.
#[test]
fn a_curved_spine_keeps_its_points() {
    let spine = "%_110 50 m\n%_95 45 l\n%_80 20 40 20 25 45 c\n%_10 50 L\n";
    let s = read_ok(&layer(&format!("%_u\n{spine}%_N\n%_U\n")));
    let mut found = None;
    for n in art(&s.doc) {
        n.walk(&mut |c| {
            if let NodeKind::Path { path, .. } = &c.kind {
                found = Some(path.clone());
            }
        });
    }
    let path = found.expect("the spine is read");
    let a = &path.subpaths[0].anchors;
    assert_eq!(a.len(), 4);
    assert!((a[1].h_out.x - 80.0).abs() < 1e-9 && (a[2].h_in.x - 40.0).abs() < 1e-9 && (a[2].p.x - 25.0).abs() < 1e-9, "{a:?}");
    let (blend, warnings) = read_blend(&blend_file(spine, [Some("A"), Some("B")], &blend_record(&['A', 'B'])));
    assert!(!matches!(blend.kind, NodeKind::Blend { .. }), "steps drawn off the spine aren't made live");
    assert_eq!(blend.children().map(|c| c.len()), Some(5));
    assert!(warnings.iter().any(|w| w == super::blend::DRAWN));
}

/// Steps with a gradient fill can't be compared: the blend keeps its drawn steps.
#[test]
fn a_blend_whose_steps_it_cant_compare_keeps_its_drawn_steps() {
    let body = blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B']));
    // The drawn squares' fill made a gradient's.
    let gradient = "%AI5_BeginGradient: (g)\n(g) 0 2 Bd\n[\n1 0 0 0 1 %_Bs\n0 0 0 1 0 %_Bs\nBD\n%AI5_EndGradient\n";
    let drawn_from = body.find("\nu\n").expect("the drawn group") + 1;
    let (head, tail) = body.split_at(drawn_from);
    let tail = tail.replacen("Xa\n", "Xa\n0 1 0 0 1 0 0 1 0 0 Bb\n(g) 0 0 0 1 1 0 0 1 0 0 Bg\n", 5).replace("\nf\n", "\nf\n0 BB\n");
    let (blend, _) = read_blend(&format!("{gradient}{head}{tail}"));
    assert!(!matches!(blend.kind, NodeKind::Blend { .. }), "{:?}", blend.kind);
}

/// The blend's own art (its keys and spine) isn't left drawn when what follows it isn't a group of
/// drawn steps.
#[test]
fn a_blends_own_art_goes_when_its_steps_arent_a_group() {
    let body = blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B']));
    // The drawn steps as one square, not a group.
    let at = body.find("\nu\n").expect("the drawn group") + 1;
    let end = body.find("U\n9 () XW\n0 Ae\n").expect("its end");
    let body = format!("{}{}{}", &body[..at], square(0.0, 40.0, 20.0, [1.0, 0.0, 0.0], false), &body[end + 2..]);
    let s = read_ok(&layer(&body));
    let objects = art(&s.doc);
    assert_eq!(objects.len(), 1, "the drawn square alone: {objects:?}");
    assert!(matches!(objects[0].kind, NodeKind::Path { .. }));
    assert!(s.warnings.iter().any(|w| w == super::blend::DRAWN));
}

/// A record that isn't one, a key with no id, or ids the record doesn't list: the drawn steps
/// stay (with a note), without the blend's own art drawn over them.
#[test]
fn a_blend_it_cant_read_keeps_its_drawn_steps() {
    let straight = "%_10 50 m\n%_110 50 L\n";
    let garbled = "(Adobe Path Blends) 1 0 16 XP\n%00112233445566778899AABBCCDDEEFF\n";
    let short = "(Adobe Path Blends) 1 0 92 XP\n%44434241\n";
    for body in [
        blend_file(straight, [Some("A"), Some("B")], garbled),
        blend_file(straight, [Some("A"), Some("B")], short),
        blend_file(straight, [Some("A"), None], &blend_record(&['A', 'B'])),
        blend_file(straight, [Some("A"), Some("B")], &blend_record(&['X', 'Y'])),
    ] {
        let s = read_ok(&layer(&body));
        let objects = art(&s.doc);
        assert_eq!(objects.len(), 1, "only the drawn steps: {objects:?}");
        assert_eq!(objects[0].children().map(|c| c.len()), Some(5), "the five drawn squares");
        assert!(s.warnings.iter().any(|w| w == super::blend::DRAWN), "{:?}", s.warnings);
    }
}

/// A blend's opacity defines the knockout shape when the state block after its record says so,
/// live or as its drawn steps. An `Xy` that is the layer's leaves the blend's as it was.
#[test]
fn a_blend_keeps_its_knockout_shape() {
    let straight = "%_10 50 m\n%_110 50 L\n";
    let keys = [Some("A"), Some("B")];
    let record = blend_record(&['A', 'B']);
    let garbled = "(Adobe Path Blends) 1 0 16 XP\n%00112233445566778899AABBCCDDEEFF\n".to_string();
    for (record, live) in [(&record, true), (&garbled, false)] {
        let body = format!("{}0 0.6 0 1 1 Xy\n0 0 Xd\n6 () XW\n", blend_file(straight, keys, record));
        let doc = read_ok(&layer(&body)).doc;
        assert_eq!(matches!(art(&doc)[0].kind, NodeKind::Blend { .. }), live, "{:?}", art(&doc)[0].kind);
        assert!(art(&doc)[0].knockout_shape, "live: {live}");
    }
    let blend = blend_file(straight, keys, &record);
    // Written after its drawn steps, before the record.
    let doc = read_ok(&layer(&blend.replace("\nU\n9 () XW\n0 Ae\n", "\nU\n0 0.6 0 1 1 Xy\n9 () XW\n0 Ae\n"))).doc;
    assert!(matches!(art(&doc)[0].kind, NodeKind::Blend { .. }), "{:?}", art(&doc)[0].kind);
    assert_eq!((art(&doc)[0].opacity, art(&doc)[0].knockout_shape), (0.6, true));
    let doc = read_ok(&layer(&format!("{blend}0 0.6 0 1 1 Xy\n1 0.35 0 0 0 Xy\n0 0 Xd\n7 () XW\n"))).doc;
    assert!(art(&doc)[0].knockout_shape, "the blend's own");
    assert!(!doc.layers[0].knockout_shape, "the layer's own");
    let doc = read_ok(&layer(&format!("{blend}1 0.35 0 0 1 Xy\n0 0 Xd\n7 () XW\n"))).doc;
    assert!(!art(&doc)[0].knockout_shape, "the blend's own");
    assert!(doc.layers[0].knockout_shape, "the layer's own");
}

/// A blend's transparency (here knockout on, as blends are written) is the state block after its
/// record, as a group's follows its end: the live blend's, or its drawn steps' where they stay. It
/// is the blend's own, not the next object's.
#[test]
fn a_blends_transparency_follows_its_record() {
    let state = "0 0.6 0 1 0 Xy\n0 0 Xd\n6 () XW\n";
    let next = format!("0 1 0 0 0 Xy\n{RED}{}f\n", rect_path(150.0, 40.0, 170.0, 60.0));
    let straight = "%_10 50 m\n%_110 50 L\n";
    let garbled = "(Adobe Path Blends) 1 0 16 XP\n%00112233445566778899AABBCCDDEEFF\n".to_string();
    for (record, live) in [(blend_record(&['A', 'B']), true), (garbled, false)] {
        let body = format!("{}{state}{next}", blend_file(straight, [Some("A"), Some("B")], &record));
        let a = art(&read_ok(&layer(&body)).doc);
        assert_eq!(a.len(), 2, "{a:?}");
        assert_eq!(matches!(a[0].kind, NodeKind::Blend { .. }), live, "{:?}", a[0].kind);
        assert_eq!((a[0].opacity, a[0].knockout), (0.6, Knockout::On), "live: {live}");
        assert_eq!(a[1].opacity, 1.0, "the next object's own state");
    }
    // A layer whose last object is a blend: a state block that ends `7 () XW` before `LB` is the
    // layer's, and the blend keeps the state it had.
    let body = format!("{}1 0.35 0 0 0 Xy\n0 0 Xd\n7 () XW\n", blend_file(straight, [Some("A"), Some("B")], &blend_record(&['A', 'B'])));
    let doc = read_ok(&layer(&body)).doc;
    assert_eq!((doc.layers[0].opacity, doc.layers[0].blend), (0.35, BlendMode::Multiply), "the layer's state");
    let a = art(&doc);
    assert!(matches!(a[0].kind, NodeKind::Blend { .. }), "{:?}", a[0].kind);
    assert_eq!((a[0].opacity, a[0].blend), (1.0, BlendMode::Normal), "the blend's own state");
}

/// The blend is live: moving a key moves the steps between it and the other key with it.
#[test]
fn moving_a_blends_key_moves_its_steps() {
    let body = blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &blend_record(&['A', 'B']));
    let (blend, _) = read_blend(&body);
    let NodeKind::Blend { children, spec } = &blend.kind else { panic!("a blend") };
    let middle = |keys: &[Arc<Node>]| {
        let laid = vectorcraft_doc::blend::blend_expand(keys, spec);
        let xs: Vec<f64> = laid.iter().filter_map(|n| n.geometric_bounds()).map(|b| b.center().y).collect();
        xs.iter().sum::<f64>() / xs.len() as f64
    };
    let before = middle(children);
    let mut keys = children.clone();
    let mut moved = (*keys[1]).clone();
    moved.transform(vectorcraft_geom::Affine::translate((0.0, 40.0)), true);
    keys[1] = Arc::new(moved);
    let after = middle(&keys);
    assert!((after - before - 20.0).abs() < 1.0, "the steps follow: {before} -> {after}");
}

/// A record line longer than the record says is read only as far as its length: the digits after
/// it don't spoil it.
#[test]
fn a_blend_record_is_read_to_its_length() {
    let record = blend_record(&['A', 'B']);
    let longer = record.trim_end().to_string() + "DEADBEEF\n";
    let (blend, warnings) = read_blend(&blend_file("%_10 50 m\n%_110 50 L\n", [Some("A"), Some("B")], &longer));
    assert!(matches!(blend.kind, NodeKind::Blend { .. }), "{warnings:?}");
}

/// An object's dictionary with the text wrap `entries` (each a `values (key) ,` line).
fn wrap_dictionary(entries: &[&str]) -> String {
    let lines: String = entries.iter().map(|e| format!("%_{e}\n")).collect();
    format!("%_/ArtDictionary :\n{lines}%_;\n%_\n")
}

/// The wrap outline a file may keep beside the options: art, then its key.
const WRAP_PATH: &str = "X=\n%_4 As\n%_0 0 m\n%_0 20 L\n%_20 20 L\n%_20 0 L\n%_f\n%_X+\n%_ (AI11TextWrapPath) ,";

fn wrap_of(n: &Node) -> Option<vectorcraft_doc::TextWrap> {
    n.extras().wrap
}

#[test]
fn wrap_dictionary_path() {
    let dict = wrap_dictionary(&[WRAP_PATH, "0 /Bool (AI11TextWrapInvert) ,", "12.5 /Real (AI11TextWrapOffset) ,"]);
    let r = read_ok(&layer(&format!("{RED}{}f\n{dict}", rect_path(10.0, 10.0, 50.0, 50.0))));
    let kids = art(&r.doc);
    assert_eq!(kids.len(), 1, "the kept outline isn't art: {:?}", r.warnings);
    assert_eq!(wrap_of(&kids[0]), Some(vectorcraft_doc::TextWrap { offset: 12.5, invert: false }));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn wrap_dictionary_invert_and_default() {
    let inverted = wrap_dictionary(&["12.5 /Real (AI11TextWrapOffset) ,", "1 /Bool (AI11TextWrapInvert) ,"]);
    let default = wrap_dictionary(&["0 /Bool (AI11TextWrapInvert) ,"]);
    let plain = wrap_dictionary(&["(Plain) /String (AIArtName) ,"]);
    let r = read_ok(&layer(&format!(
        "{RED}{}f\n{inverted}{}f\n{default}{}f\n{plain}",
        rect_path(0.0, 0.0, 10.0, 10.0),
        rect_path(20.0, 0.0, 30.0, 10.0),
        rect_path(40.0, 0.0, 50.0, 10.0)
    )));
    let kids = art(&r.doc);
    assert_eq!(kids.len(), 3);
    assert_eq!(wrap_of(&kids[0]), Some(vectorcraft_doc::TextWrap { offset: 12.5, invert: true }));
    assert_eq!(wrap_of(&kids[1]), Some(vectorcraft_doc::TextWrap::default()), "6 pt when the offset isn't given");
    assert_eq!(wrap_of(&kids[2]), None, "an object without wrap options isn't a wrap object");
    assert_eq!(kids[2].name.as_deref(), Some("Plain"));
}

#[test]
fn wrap_dictionary_bad_offset() {
    let offsets = ["nan /Real", "1e308 /Real", "(far) /String", "2000 /Real", "-2000 /Real"];
    let mut body = RED.to_string();
    for (i, offset) in offsets.iter().enumerate() {
        let x = 20.0 * i as f64;
        body += &format!("{}f\n", rect_path(x, 0.0, x + 10.0, 10.0));
        body += &wrap_dictionary(&[&format!("{offset} (AI11TextWrapOffset) ,"), "1 /Bool (AI11TextWrapInvert) ,"]);
    }
    // An offset that isn't a number, with nothing else of the options, isn't a wrap object.
    body += &format!("{}f\n", rect_path(200.0, 0.0, 210.0, 10.0));
    body += &wrap_dictionary(&["(far) /String (AI11TextWrapOffset) ,", "7 /Bool (AI11TextWrapInvert) ,"]);
    let r = read_ok(&layer(&body));
    let offsets: Vec<Option<f64>> = art(&r.doc).iter().map(|n| wrap_of(n).map(|w| w.offset)).collect();
    assert_eq!(offsets, [Some(6.0), Some(1000.0), Some(6.0), Some(1000.0), Some(-1000.0), None]);
    assert!(art(&r.doc).iter().take(5).all(|n| wrap_of(n).is_some_and(|w| w.invert)));
}

#[test]
fn wrap_dictionary_group() {
    let dict = wrap_dictionary(&["6 /Real (AI11TextWrapOffset) ,", "0 /Bool (AI11TextWrapInvert) ,"]);
    let r = read_ok(&layer(&format!("u\n{RED}{}f\n{}f\nU\n{dict}", rect_path(0.0, 0.0, 10.0, 10.0), rect_path(20.0, 0.0, 30.0, 10.0))));
    let kids = art(&r.doc);
    assert_eq!(kids.len(), 1);
    assert_eq!(wrap_of(&kids[0]), Some(vectorcraft_doc::TextWrap::default()));
    let NodeKind::Group { children, .. } = &kids[0].kind else { panic!("a group: {:?}", kids[0].kind) };
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|c| wrap_of(c).is_none() && matches!(c.kind, NodeKind::Path { .. })));
}

/// Wraparound text in the version 7 format (§15.4.2): `*w`, objects, the area text, the objects it
/// wraps around, `*W`.
#[test]
fn legacy_wrap_group() {
    let text = format!("1 To\n1 0 0 1 0 0 0 Tp\n{}n\nTP\n0 Tr\n0 0 0 1 k\n/_Helvetica 12 Tf\n(Around) Tx\nTO\n", rect_path(0.0, 0.0, 100.0, 80.0));
    // The last object's own dictionary gives its options.
    let own = wrap_dictionary(&["3 /Real (AI11TextWrapOffset) ,", "1 /Bool (AI11TextWrapInvert) ,"]);
    let body = format!(
        "*w\n{RED}{}f\n{text}{}f\n{}f\n{own}*W\n",
        rect_path(0.0, 0.0, 5.0, 5.0),
        rect_path(30.0, 30.0, 50.0, 50.0),
        rect_path(60.0, 30.0, 70.0, 50.0)
    );
    let r = read_ok(&layer(&body));
    let kids = art(&r.doc);
    assert_eq!(kids.len(), 1, "{:?}", r.warnings);
    let NodeKind::Group { children, clip: false } = &kids[0].kind else { panic!("a group: {:?}", kids[0].kind) };
    let wraps: Vec<(bool, bool)> = children.iter().map(|c| (matches!(c.kind, NodeKind::Text(_)), wrap_of(c).is_some())).collect();
    assert_eq!(wraps, [(false, true), (true, false), (false, true), (false, true)], "objects before the text are wrap objects too: {:?}", r.warnings);
    let none = Some(vectorcraft_doc::TextWrap { offset: 0.0, invert: false });
    assert_eq!(wrap_of(&children[0]), none, "no offset");
    assert_eq!(wrap_of(&children[2]), none, "no offset");
    assert_eq!(wrap_of(&children[3]), Some(vectorcraft_doc::TextWrap { offset: 3.0, invert: true }));
    assert!(!r.warnings.iter().any(|w| w.contains("*w") || w.contains("*W")), "{:?}", r.warnings);
}

/// A wraparound group with no type in it: each of its objects is a wrap object, with no offset.
#[test]
fn legacy_wrap_group_without_text() {
    let body = format!("*w\n{RED}{}f\n{}f\n*W\n", rect_path(0.0, 0.0, 5.0, 5.0), rect_path(30.0, 30.0, 50.0, 50.0));
    let r = read_ok(&layer(&body));
    let kids = art(&r.doc);
    assert_eq!(kids.len(), 1, "{:?}", r.warnings);
    let NodeKind::Group { children, clip: false } = &kids[0].kind else { panic!("a group: {:?}", kids[0].kind) };
    let wraps: Vec<_> = children.iter().map(|c| wrap_of(c)).collect();
    assert_eq!(wraps, [Some(vectorcraft_doc::TextWrap { offset: 0.0, invert: false }); 2]);
}

/// `*w` and `*W` take no operands, and a wraparound group can't be an element of a compound path.
#[test]
fn legacy_wrap_group_errors() {
    let square = format!("{RED}{}f\n", rect_path(0.0, 0.0, 5.0, 5.0));
    for (body, error) in [
        (format!("1 *w\n{square}*W\n"), "`*w` takes no operands"),
        (format!("*w\n{square}0 *W\n"), "`*W` takes no operands"),
        (format!("*u\n*w\n{square}*W\n*U\n"), "a wraparound group is inside a compound path"),
        (format!("*u\nu\n*w\n{square}*W\nU\n*U\n"), "a wraparound group is inside a compound path"),
    ] {
        let e = read(stream(&layer(&body)).as_bytes()).unwrap_err();
        assert!(e.contains(error), "{body:?}: {e}");
    }
}

/// The printer's crop marks Illustrator draws after the last layer (`%AI3_BeginCrops`, inside
/// `userdict /AI3_noCropMarks known not { … } if`) aren't art outside the layers: the file reads
/// from its layers, without them (the crop area is `%AI3_Cropmarks`).
#[test]
fn crop_marks_after_the_layers_are_not_art() {
    let crops = "%AI3_BeginCrops\nuserdict /AI3_noCropMarks known not {\n0 A\nu\n0 R\n1 1 1 1 ([Registration]) 0 XS\n\
                 0 J 0 j 0.5 w 4 M []0 d\n0 XR\n-9 -400 m\n-36 -400 L\nS\nU\n} if\n%AI3_EndCrops\n";
    let r = read_ok(&format!("{}{crops}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
    assert_eq!(art(&r.doc).len(), 1);
    assert_eq!(r.doc.layers.len(), 1);
}

/// The name of an Objects on Path record.
const ALIGN_ON_PATH: &str = "Adobe Align On Path"; // brand-ok: the record's name in the files
/// The ids of an Objects on Path group's path and its four objects.
const ON_PATH_IDS: [&str; 5] = [
    "0a6d1e52-0000-4000-8000-000000000001",
    "0a6d1e52-0000-4000-8000-000000000002",
    "0a6d1e52-0000-4000-8000-000000000003",
    "0a6d1e52-0000-4000-8000-000000000004",
    "0a6d1e52-0000-4000-8000-000000000005",
];
const OPEN_PATH: &str = "20 20 m\n100 80 L\n180 20 L\nS\n";
const CLOSED_PATH: &str =
    "40 50 m\n40 66.57 66.86 80 100 80 c\n133.14 80 160 66.57 160 50 c\n160 33.43 133.14 20 100 20 c\n66.86 20 40 33.43 40 50 c\ns\n";

/// An Objects on Path group: the stroked `path` with the id `ids[0]`, then a 10 × 6 red square for
/// each other id, 50 pt apart from x 20, whose frame is turned `angle` radians.
fn on_path_group(path: &str, ids: &[&str], angle: f64) -> String {
    use vectorcraft_testkit::ai::on_path_member;
    let (first, rest) = ids.split_first().unwrap();
    let mut group = format!("0 Ae\nu\n/ConstraintGroup :\n128 /GroupType ,\n;\n0 0 1 0 0 0 1 XA\n2 w\n{path}{}", on_path_member(first, None, 0.0));
    for (i, id) in rest.iter().enumerate() {
        let x = 20.0 + 50.0 * i as f64;
        group += &format!("{RED}{}f\n{}", rect_path(x, 40.0, x + 10.0, 46.0), on_path_member(id, Some(i), angle));
    }
    group + "U\n"
}

/// The group the layer of `body` holds first, and its Objects on Path record.
fn on_path(body: &str) -> (Arc<Node>, Option<vectorcraft_doc::objects_on_path::ObjectsOnPath>) {
    let doc = read_ok(body).doc;
    let g = art(&doc).first().cloned().unwrap();
    let rec = g.extras().objects_on_path.as_deref().cloned();
    (g, rec)
}

/// An Objects on Path group opens as one, with its path, its objects in their order along the path,
/// Pivot and Rotate, and its art where the file draws it; the same art without the record opens as a
/// plain group.
#[test]
fn an_objects_on_path_group_opens_with_its_path_objects_and_options() {
    use vectorcraft_testkit::ai::{on_path_options, on_path_record};
    let group = on_path_group(OPEN_PATH, &ON_PATH_IDS, 0.5);
    let record = on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &on_path_options(4, 0, std::f64::consts::FRAC_PI_6, ""));
    let (g, rec) = on_path(&format!("{record}{}", layer(&group)));
    let rec = rec.expect("an Objects on Path group");
    let kids = g.children().unwrap();
    assert_eq!(kids.len(), 5);
    let NodeKind::Path { path, .. } = &kids.iter().find(|c| c.id == rec.path).unwrap().kind else { panic!("the path") };
    assert!(path.subpaths.first().is_some_and(|s| !s.closed), "the open path");
    let child = |id| kids.iter().find(|c| c.id == id).unwrap();
    let xs: Vec<f64> = rec.members.iter().map(|m| child(m.id).geometric_bounds().unwrap().x0).collect();
    assert_eq!(xs, [20.0, 70.0, 120.0, 170.0], "the objects in the file's order");
    for m in &rec.members {
        let center = child(m.id).geometric_bounds().unwrap().center();
        assert_eq!((m.angle, m.frame), (-0.5, kurbo::Affine::rotate_about(-0.5, center)), "the frame turned counterclockwise");
    }
    assert!((rec.rotate - 30.0).abs() < 1e-9, "{}", rec.rotate);
    assert_eq!(rec.pivot, 0);
    let (plain, none) = on_path(&layer(&group));
    assert!(none.is_none());
    let kinds = |g: &Node| g.children().unwrap().iter().map(|c| c.kind.clone()).collect::<Vec<_>>();
    assert_eq!(kinds(&g), kinds(&plain), "the art stays where it is drawn");
}

/// A group on a closed path opens as an Objects on Path group too.
#[test]
fn an_objects_on_path_group_on_a_closed_path_opens_live() {
    use vectorcraft_testkit::ai::{on_path_options, on_path_record};
    let record = on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &on_path_options(4, 4, 0.0, ""));
    let (g, rec) = on_path(&format!("{record}{}", layer(&on_path_group(CLOSED_PATH, &ON_PATH_IDS, 0.0))));
    let rec = rec.expect("an Objects on Path group");
    let NodeKind::Path { path, .. } = &g.children().unwrap().iter().find(|c| c.id == rec.path).unwrap().kind else { panic!("the path") };
    assert!(path.subpaths.first().is_some_and(|s| s.closed), "the closed path");
    assert_eq!((rec.members.len(), rec.pivot, rec.rotate), (4, 4, 0.0));
}

/// A record the group can't be read from, or with options an Objects on Path group doesn't hold,
/// leaves the group a plain group with its art.
#[test]
fn an_objects_on_path_record_that_doesnt_fit_leaves_a_plain_group() {
    use vectorcraft_testkit::ai::{on_path_options, on_path_record};
    let [p, a, b, c, d] = ON_PATH_IDS;
    let options = |more: &str| on_path_options(4, 4, 0.0, more);
    let good = options("");
    let group = on_path_group(OPEN_PATH, &ON_PATH_IDS, 0.0);
    assert!(on_path(&format!("{}{}", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good), layer(&group))).1.is_some());
    let cases: Vec<(&str, String, String)> = vec![
        ("the path's id is missing", on_path_record(ALIGN_ON_PATH, &["0a6d1e52-ffff", a, b, c, d], &good), group.clone()),
        ("an object's id is missing", on_path_record(ALIGN_ON_PATH, &[p, a, b, c], &good), group.clone()),
        ("an id twice", on_path_record(ALIGN_ON_PATH, &[p, a, b, b, d], &good), group.clone()),
        ("objects out of order", on_path_record(ALIGN_ON_PATH, &[p, b, a, c, d], &good), group.clone()),
        ("a spacing", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &options("%_56.5 /Real (Spacing) ,\n")), group.clone()),
        ("an option this reader doesn't know", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &options("%_1 /Bool (Flip) ,\n")), group.clone()),
        (
            "uneven spacing",
            on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good.replace("1 /Bool (IsEquiSpaced)", "0 /Bool (IsEquiSpaced)")),
            group.clone(),
        ),
        (
            "a later start",
            on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good.replace("0 /Real (StartRangeFraction)", "0.25 /Real (StartRangeFraction)")),
            group.clone(),
        ),
        ("a pivot past 8", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &on_path_options(4, 9, 0.0, "")), group.clone()),
        ("another constraint", on_path_record("Some Other Constraint", &ON_PATH_IDS, &good), group.clone()),
        (
            "another version",
            on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good).replace("%_1 /Int (Constraint Verison)", "%_2 /Int (Constraint Verison)"),
            group.clone(),
        ),
        ("an object without its angle", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good), group.replace("%_(0) /String (BBAccumRotation) ,\n", "")),
        (
            "another child in the group",
            on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good),
            group.replace("U\n", &format!("{}f\nU\n", rect_path(0.0, 0.0, 5.0, 5.0))),
        ),
        ("a path without its key", on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &good), group.replace("%_1 /Bool (KeyPath) ,\n", "")),
    ];
    for (why, record, group) in cases {
        let (g, rec) = on_path(&format!("{record}{}", layer(&group)));
        assert!(rec.is_none(), "{why}");
        assert!(matches!(g.kind, NodeKind::Group { clip: false, .. }), "{why}");
        assert!(g.children().unwrap().len() >= 5, "{why}: the art is kept");
    }
}

/// The path and objects of a group are measured once however many records name it: a thousand
/// copies of a record open about as fast as one copy followed by 999 records whose path id is
/// missing, when the group opens live, when its long path has no length, and when its first object
/// is a group of many paths and its last object has no bounds.
#[test]
fn repeated_objects_on_path_records_dont_measure_the_objects_again() {
    use vectorcraft_testkit::ai::{on_path_member, on_path_options, on_path_record};
    let paths: String = (0..20_000).map(|i| format!("{}f\n", rect_path(f64::from(i % 200), 40.0, f64::from(i % 200) + 1.0, 46.0))).collect();
    let big = format!("u\n{RED}{paths}U\n{}", on_path_member(ON_PATH_IDS[1], Some(0), 0.0));
    let empty = format!("u\nU\n{}", on_path_member(ON_PATH_IDS[2], Some(1), 0.0));
    let still = format!("20 20 m\n{}S\n", "20 20 L\n".repeat(20_000));
    let cases = [
        ("a live group", OPEN_PATH.to_string(), big.clone(), 2, true),
        ("a path without length", still, big.clone(), 2, false),
        ("an object without bounds", OPEN_PATH.to_string(), format!("{big}{empty}"), 3, false),
    ];
    for (why, path, objects, n, live) in cases {
        let ids = &ON_PATH_IDS[..n];
        let group = format!(
            "0 Ae\nu\n/ConstraintGroup :\n128 /GroupType ,\n;\n0 0 1 0 0 0 1 XA\n2 w\n{path}{}{objects}U\n",
            on_path_member(ids[0], None, 0.0)
        );
        let record = on_path_record(ALIGN_ON_PATH, ids, &on_path_options(n - 1, 4, 0.0, ""));
        let (start, end) = (record.find("%_/Dictionary :\n%_1 /Int").unwrap(), record.find("%_; (Constraint Data)").unwrap());
        let copy = &record[start..end];
        let unresolved = copy.replace(ids[0], "0a6d1e52-ffff-4000-8000-000000000000");
        let one = format!("{}{copy}{}{}", &record[..start], unresolved.repeat(999), &record[end..]);
        let many = format!("{}{}{}", &record[..start], copy.repeat(1000), &record[end..]);
        let time = |data: &str| {
            let started = std::time::Instant::now();
            let doc = read_ok(&format!("{data}{}", layer(&group))).doc;
            let elapsed = started.elapsed();
            let g = art(&doc)[0].clone();
            assert_eq!(g.children().unwrap().len(), n, "{why}");
            assert_eq!(g.extras().objects_on_path.is_some(), live, "{why}");
            elapsed
        };
        // The fastest of three runs each, taken in turn, so that other tests running at the same
        // time weigh on both.
        let (mut fastest_one, mut fastest_many) = (std::time::Duration::MAX, std::time::Duration::MAX);
        for _ in 0..3 {
            fastest_one = fastest_one.min(time(&one));
            fastest_many = fastest_many.min(time(&many));
        }
        assert!(fastest_many < fastest_one * 5, "{why}: one copy: {fastest_one:?}, 1000 copies: {fastest_many:?}");
    }
}

/// The copy of the drawn group that follows it as versioned content is skipped: the group opens once,
/// live.
#[test]
fn an_objects_on_path_group_in_versioned_content_opens_once() {
    use vectorcraft_testkit::ai::{on_path_options, on_path_record};
    let record = on_path_record(ALIGN_ON_PATH, &ON_PATH_IDS, &on_path_options(4, 4, 0.0, ""));
    let group = on_path_group(OPEN_PATH, &ON_PATH_IDS, 0.0);
    let copy = group.replace("/ConstraintGroup :\n128 /GroupType ,\n;\n", "");
    let versioned = format!("%AI17_Begin_Content_if_version_gt:24 14\n{group}%AI17_Alternate_Content\n{copy}%AI17_End_Versioned_Content\n");
    let doc = read_ok(&format!("{record}{}", layer(&versioned))).doc;
    let kids = art(&doc);
    assert_eq!(kids.len(), 1);
    assert_eq!(kids[0].children().unwrap().len(), 5);
    assert!(kids[0].extras().objects_on_path.is_some());
}

// ---- brushes ------------------------------------------------------------------------------

const CALLIGRAPHIC: &str = "Adobe Calligraphic Brush Tool";
const SCATTER: &str = "Adobe Scatter Brush Tool";
const ART_BRUSH: &str = "Adobe ArtOnPath Brush Tool";
const PATTERN_BRUSH: &str = "Adobe PatternOnPath Brush Tool";
const BRISTLE: &str = "Adobe dBrush Brush Tool";

/// A plug-in object of plug-in `kind` named `name`, its parameters `params` split into strings of
/// 16 characters at most (`-` after each but the last, `.` after that).
fn plugin_object(kind: &str, name: &str, params: &str) -> String {
    let chars: Vec<char> = params.chars().collect();
    let chunks: Vec<String> = chars.chunks(16).map(|c| c.iter().collect()).collect();
    let n = chunks.len();
    let lines: String = chunks.iter().enumerate().map(|(i, c)| format!("({c}) {}\n", if i + 1 == n { "." } else { "-" })).collect();
    format!("%AI5_Begin_NonPrinting\nNp\n%AI8_BeginPluginObject\n({kind})\n({name})\n{lines}%AI8_EndPluginObject\n%AI5_End_NonPrinting--\n")
}

/// The brush manager's order of the brushes `brushes` (plug-in, name).
fn brush_order(brushes: &[(&str, &str)]) -> String {
    let params: String = brushes.iter().map(|(k, n)| format!(" {k}/ {n}/")).collect();
    plugin_object(brush::MANAGER, brush::MANAGER, &params)
}

/// A brush's art named `name`: a filled square of side `side`.
fn brush_art(name: &str, side: f64) -> String {
    format!("%AI8_BeginBrushPattern\n({name})\n0 A\n0 g\n{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, side, side))
}

/// A Calligraphic brush with diameter 2 to 6, roundness 40 to 60 and angle -30 to 10, the diameter
/// varying at random, the roundness fixed and the angle with the pen's pressure.
const NIB: &str = "1 2 6 40 60 -30 10 1 0 2 0";
/// A Scatter brush of the art `Dot art`: size 50% to 150% at random, spacing 120% to 200% fixed,
/// scatter -25% to 25% with the pressure, rotation -45° to 30° at random and following the path.
const SPRAY: &str = "2 /Dot art/ 5 1 0 0 / 0 0.5 1.5 1 1.2 2 0 -0.25 0.25 2 -45 30 1 1 0";
/// An Art brush of the art `Ribbon art`.
const RIBBON: &str = "2 / Ribbon art/ / / / / 0 0/ 5 0 0 0 /  1 1 1 0 1 0 0 0 1 1 0 0 0 0 1";
/// A Pattern brush of four tiles (the fourth slot empty) at 80%.
const FENCE: &str = "2 / Tile one/ Tile two/ Tile three/ / Tile five/ 0 0/ 0 1 /  1 1 0.8 0 4 0 0 0 0.8 0.8 0 0 0 0 0";
/// A Bristle brush: stiffness 60%, thickness 40%, density 30%, opacity 80%, length 150%, size 2 mm.
const MOP: &str = "0 4 0.6 0.4 0.3 0.8 1.5 2 library shape";

/// The brushes of the five kinds, their art and the manager's order (the Bristle brush first).
fn five_brushes() -> String {
    let order = brush_order(&[(BRISTLE, "Mop"), (CALLIGRAPHIC, "Nib"), (SCATTER, "Spray"), (ART_BRUSH, "Ribbon"), (PATTERN_BRUSH, "Fence")]);
    let art: String = ["Dot art", "Ribbon art", "Tile one", "Tile two", "Tile three", "Tile five"].iter().map(|n| brush_art(n, 4.0)).collect();
    let objects = [
        (CALLIGRAPHIC, "Nib", NIB),
        (SCATTER, "Spray", SPRAY),
        (ART_BRUSH, "Ribbon", RIBBON),
        (PATTERN_BRUSH, "Fence", FENCE),
        (BRISTLE, "Mop", MOP),
    ];
    let objects: String = objects.iter().map(|(k, n, p)| plugin_object(k, n, p)).collect();
    format!("{art}{order}{objects}")
}

fn brush_named<'a>(lib: &'a [vectorcraft_brush::Brush], name: &str) -> &'a vectorcraft_brush::BrushKind {
    match lib.iter().find(|b| b.name == name) {
        Some(b) => &b.kind,
        None => panic!("no brush {name} in {:?}", lib.iter().map(|b| &b.name).collect::<Vec<_>>()),
    }
}

/// The brush manager's order gives the kind and name of each brush, in the panel's order.
#[test]
fn the_brush_managers_order_gives_each_brushs_kind_in_order() {
    let params = format!(" {BRISTLE}/ Mop/ {CALLIGRAPHIC}/ Nib/ {SCATTER}/ Spray/ {ART_BRUSH}/ Ribbon/ {PATTERN_BRUSH}/ Fence  /");
    let order = brush::parse_brush_manager(&params).unwrap();
    use brush::Tool;
    let want = [(Tool::Bristle, "Mop"), (Tool::Calligraphic, "Nib"), (Tool::Scatter, "Spray"), (Tool::Art, "Ribbon"), (Tool::Pattern, "Fence  ")];
    assert_eq!(order, want.map(|(t, n)| (t, n.to_string())), "a name keeps its trailing spaces");
    assert_eq!(brush::parse_brush_manager(" Some Other Plugin/ Thing/"), None);
}

/// A file's brushes replace the default ones, in the manager's order, with their values. A pattern
/// brush is left out (which slot holds which tile isn't known), with a note.
#[test]
fn a_files_brushes_come_in_with_their_values_in_the_panels_order() {
    let r = read_ok(&format!("{}{}", five_brushes(), layer(&(rect_path(0.0, 0.0, 10.0, 10.0) + "f\n"))));
    let lib = vectorcraft_brush::library(&r.doc);
    let names: Vec<&str> = lib.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["Mop", "Nib", "Spray", "Ribbon"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w == "brushes VectorCraft can't read yet were left out of the Brushes panel (1)"), "{:?}", r.warnings);
    use vectorcraft_brush::{BrushKind, Variation};
    let BrushKind::Calligraphic(c) = brush_named(&lib, "Nib") else { panic!("calligraphic") };
    assert_eq!((c.angle, c.roundness, c.size), (-10.0, 50.0, 4.0));
    assert_eq!(c.variation, [20.0, 10.0, 2.0]);
    assert_eq!(c.modes, Some([Variation::Pressure, Variation::Fixed, Variation::Random]));
    let BrushKind::Scatter(s) = brush_named(&lib, "Spray") else { panic!("scatter") };
    assert_eq!((s.size, s.spacing, s.scatter, s.rotation), ((50.0, 150.0), (120.0, 200.0), (-25.0, 25.0), (-45.0, 30.0)));
    assert_eq!(s.modes, Some([Variation::Random, Variation::Fixed, Variation::Pressure, Variation::Random]));
    assert!(s.rotation_relative_to_path);
    assert_eq!(s.art.geometric_bounds().map(|b| (b.width(), b.height())), Some((4.0, 4.0)), "the art of its section");
    let BrushKind::Art(a) = brush_named(&lib, "Ribbon") else { panic!("art") };
    assert!(a.art.geometric_bounds().is_some(), "the art of its section");
    assert_eq!(a.colorization, vectorcraft_brush::Colorization::Tints, "its code");
    let BrushKind::Bristle(b) = brush_named(&lib, "Mop") else { panic!("bristle") };
    let got = [b.stiffness, b.thickness, b.density, b.opacity, b.length, b.size];
    let want = [60.0, 40.0, 30.0, 80.0, 150.0, 2.0 * 72.0 / 25.4];
    assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-9), "{got:?}");
}

/// A Pattern brush's five slots name its art sections, in its order, an empty slot naming none;
/// its scale comes in as a percentage.
#[test]
fn a_pattern_brushs_five_slots_are_kept() {
    let d = brush::parse_tool_object(brush::Tool::Pattern, "Fence", FENCE).unwrap();
    let brush::DraftKind::Pattern { tiles, scale } = d.kind else { panic!("{d:?}") };
    let s = |n: &str| Some(n.to_string());
    assert_eq!(tiles, [s("Tile one"), s("Tile two"), s("Tile three"), None, s("Tile five")]);
    assert!((scale - 80.0).abs() < 1e-9);
    let d = brush::parse_tool_object(brush::Tool::Scatter, "Spray", SPRAY).unwrap();
    assert!(matches!(d.kind, brush::DraftKind::Scatter { ref art, .. } if art == "Dot art"), "{d:?}");
    let d = brush::parse_tool_object(brush::Tool::Art, "Ribbon", RIBBON).unwrap();
    assert!(matches!(d.kind, brush::DraftKind::Art { ref art, .. } if art == "Ribbon art"), "{d:?}");
}

/// A brush's art filled with a pattern the file defines before it is read, with that pattern.
#[test]
fn a_brushs_art_takes_the_patterns_defined_before_it() {
    let dots = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 4 4\n%_0 0 0 1 k\n%_1 1 m\n%_3 1 L\n%_3 3 L\n%_1 3 L\n%_f\nE\n%AI3_EndPattern\n";
    let art_ = format!(
        "%AI8_BeginBrushPattern\n(Dot art)\n0 A\n(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n%AI8_EndBrushPattern\n",
        rect_path(0.0, 0.0, 4.0, 4.0)
    );
    let r = read_ok(&format!("{dots}{art_}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    let vectorcraft_brush::BrushKind::Scatter(s) = brush_named(&lib, "Spray") else { panic!("scatter") };
    assert!(matches!(fill(&s.art), Paint::Pattern { ref pattern, .. } if pattern == "Dots"), "{:?}", r.warnings);
}

/// A spot color that only a brush's art uses comes in as a swatch, and the art's process colors
/// count toward the document's color mode, as a pattern's do.
#[test]
fn a_brushs_art_brings_its_spot_colors_and_color_mode() {
    let section = format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n0 1 1 0 (Ink) 0 x\n{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0));
    let r = read_ok(&format!("{section}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    assert!(matches!(brush_named(&lib, "Spray"), vectorcraft_brush::BrushKind::Scatter(_)), "{:?}", r.warnings);
    let ink = r.doc.swatch("Ink").map(|s| (s.paint.color(), s.global, s.spot));
    assert_eq!(ink, Some((Some(Color::cmyk(0.0, 1.0, 1.0, 0.0)), true, true)), "{:?}", r.warnings);
    assert_eq!(r.doc.color_mode, ColorMode::Cmyk);
    // Art that isn't kept brings nothing.
    let odd = "%AI8_BeginBrushPattern\n(Odd art)\n0 1 1 0 (Lost ink) 0 x\n0 0 m\n1 1 L\nnotanoperator\nf\n%AI8_EndBrushPattern\n";
    let r = read_ok(&format!("{odd}{}{}", plugin_object(SCATTER, "Odd", &SPRAY.replace("Dot art", "Odd art")), layer("")));
    assert!(r.doc.swatch("Lost ink").is_none());
    assert_eq!(r.doc.color_mode, ColorMode::Rgb);
    // Nor does art that only a brush left out uses (a pattern brush).
    let tile = "%AI8_BeginBrushPattern\n(Tile one)\n0 A\n0 1 1 0 (Fence ink) 0 x\n0 0 m\n4 0 L\n4 4 L\nf\n%AI8_EndBrushPattern\n";
    let r = read_ok(&format!("{tile}{}{}", plugin_object(PATTERN_BRUSH, "Fence", FENCE), layer("")));
    assert!(vectorcraft_brush::library(&r.doc).is_empty(), "{:?}", r.warnings);
    assert!(r.doc.swatch("Fence ink").is_none());
    assert_eq!(r.doc.color_mode, ColorMode::Rgb);
}

/// A spot color that the page's art and a brush's art both define keeps the page's value, and a
/// brush art section that a later one of the same name replaces brings none of its colors.
#[test]
fn a_brushs_art_colors_come_after_the_pages_and_a_replaced_sections_bring_nothing() {
    let section = |ink: &str, color: &str| {
        format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n{color} ({ink}) 0 x\n{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0))
    };
    let page = layer(&format!("0 0 1 0 (Ink) 0 x\n{}f", rect_path(0.0, 0.0, 10.0, 10.0)));
    let body = format!("{}{}{}{page}", section("Gone ink", "1 0 0 0"), section("Ink", "0 1 1 0"), plugin_object(SCATTER, "Spray", SPRAY));
    let r = read_ok(&body);
    assert!(matches!(brush_named(&vectorcraft_brush::library(&r.doc), "Spray"), vectorcraft_brush::BrushKind::Scatter(_)));
    assert_eq!(r.doc.swatch("Ink").and_then(|s| s.paint.color()), Some(Color::cmyk(0.0, 0.0, 1.0, 0.0)), "{:?}", r.warnings);
    assert!(r.doc.swatch("Gone ink").is_none());
}

/// The art of a file's brushes brings each color name's first definition once, and at most
/// [`super::brush::MAX_BRUSH_ART_COLORS`] names; the colors it leaves out still count toward the
/// color mode, and art that no brush uses, or that a later section replaces, takes no places.
#[test]
fn brush_art_keeps_each_color_name_once_and_a_bounded_number_of_names() {
    let open = |sections: &[(&str, &str)]| {
        let defs: String = sections
            .iter()
            .map(|(name, colors)| {
                format!("%AI8_BeginBrushPattern\n({name})\n0 A\n{colors}{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0))
            })
            .collect();
        let mist = plugin_object(SCATTER, "Mist", &SPRAY.replace("Dot art", "Dash art"));
        read_ok(&format!("{defs}{}{mist}{}", plugin_object(SCATTER, "Spray", SPRAY), layer(""))).doc
    };
    let max = super::brush::MAX_BRUSH_ART_COLORS;
    let ink = "0 1 1 0 (Ink) 0 x\n";
    let many: String = (0..max).map(|i| format!("1 0 0 0 (Name {i}) 0 x\n")).collect();
    // One name repeated takes one place.
    let doc = open(&[("Dot art", &format!("{}{ink}", "1 0 0 0 (Same) 0 x\n".repeat(max)))]);
    assert!(doc.swatch("Same").is_some() && doc.swatch("Ink").is_some());
    // Different names fill the places: the name past them is left out, and its color still counts.
    let doc = open(&[("Dot art", &format!("{many}{ink}"))]);
    assert!(doc.swatch("Name 0").is_some() && doc.swatch("Ink").is_none());
    assert_eq!(doc.color_mode, ColorMode::Cmyk);
    // The places are shared by the art of every brush, and a name in the art of two takes one.
    let doc = open(&[("Dot art", &many), ("Dash art", ink)]);
    assert_eq!(vectorcraft_brush::library(&doc).len(), 2);
    assert!(doc.swatch("Name 0").is_some() && doc.swatch("Ink").is_none());
    let half: String = many.lines().take(max / 2).map(|l| format!("{l}\n")).collect();
    let doc = open(&[("Dot art", &half), ("Dash art", &format!("{half}{ink}"))]);
    assert!(doc.swatch("Name 0").is_some() && doc.swatch("Ink").is_some());
    // The color operators of every brush's art take places before the gradient stops of any.
    let stops: String = (0..max / 250)
        .map(|g| {
            let named: String = (0..250).map(|s| format!("%_0 0 1 0 (Stop {g} {s}) 0 0 5 1 6 50 {} Bs\n", s * 100 / 250)).collect();
            format!("%AI5_BeginGradient: (G{g})\n(G{g}) 0 250 Bd\n[\n<00FF>\n0\n{named}BD\n%AI5_EndGradient\n")
        })
        .collect();
    let doc = open(&[("Dot art", &stops), ("Dash art", ink)]);
    assert!(doc.swatch("Stop 0 0").is_some() && doc.swatch("Ink").is_some(), "{:?}", doc.swatch("Stop 0 0"));
    // Art that no brush uses, and art that a later section of its name replaces, take no places.
    let doc = open(&[("Other art", &many), ("Dot art", &many), ("Dot art", ink)]);
    assert!(doc.swatch("Ink").is_some() && doc.swatch("Name 0").is_none());
}

/// A global process color that only a brush's art defines with its color operator stays a process
/// color when a gradient of the file names it as a stop, as the page's own definition does (#1771).
#[test]
fn a_brushs_art_defines_a_process_color_that_a_gradient_stop_names() {
    let defs = "%AI5_BeginGradient: (Brand)\n(Brand) 0 2 Bd\n[\n<00FF>\n0\n\
                %_0.93 0.1 0 0 (Test Cyan) 0 0 5 1 6 50 0 Bs\n%_0 0 0 0 1 0 0 2 1 6 50 100 Bs\nBD\n%AI5_EndGradient\n";
    let section = format!(
        "%AI8_BeginBrushPattern\n(Dot art)\n0 A\n0.93 0.1 0 0 (Test Cyan) 0 0 Xk\n{}f\n%AI8_EndBrushPattern\n",
        rect_path(0.0, 0.0, 4.0, 4.0)
    );
    let doc = read_ok(&format!("{defs}{section}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer(""))).doc;
    assert!(matches!(brush_named(&vectorcraft_brush::library(&doc), "Spray"), vectorcraft_brush::BrushKind::Scatter(_)));
    let sw = doc.swatch("Test Cyan").unwrap();
    assert!(sw.global && !sw.spot, "a global process color, not an ink: {sw:?}");
}

/// A brush art section inside a symbol's definition isn't kept, so its spot colors make no swatch.
#[test]
fn brush_art_inside_a_symbol_brings_no_colors() {
    let section =
        format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n0 1 1 0 (Hidden ink) 0 x\n{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0));
    let body = format!("{}{}{}", symbol("S", &format!("{section}{}", dot()), 5), plugin_object(SCATTER, "Spray", SPRAY), layer(""));
    let r = read_ok(&body);
    assert!(r.doc.symbols.iter().any(|s| s.name == "S"), "{:?}", r.warnings);
    assert!(r.doc.swatch("Hidden ink").is_none());
}

/// A brush's art filled with a pattern defined in the version 7 form places the tiles with the
/// fill's offset, scale, rotation, reflection and shear operands, and a brush whose art has such a
/// fill without all nine of them isn't read.
#[test]
fn a_brushs_art_places_a_version_7_patterns_tiles_with_its_operands() {
    let def = pattern7("Grid", "(0 g) @ _ &\n");
    let read_with = |ops: &str| {
        let section = format!("%AI8_BeginBrushPattern\n(Dot art)\n0 A\n(Grid) {ops} p\n{}f\n%AI8_EndBrushPattern\n", rect_path(0.0, 0.0, 4.0, 4.0));
        read_ok(&format!("{def}{section}{}{}", plugin_object(SCATTER, "Spray", SPRAY), layer("")))
    };
    let placed = |ops: &str| {
        let r = read_with(ops);
        let lib = vectorcraft_brush::library(&r.doc);
        let vectorcraft_brush::BrushKind::Scatter(s) = brush_named(&lib, "Spray") else { panic!("scatter") };
        match fill(&s.art) {
            Paint::Pattern { xf, .. } => xf,
            p => panic!("{ops}: {p:?} {:?}", r.warnings),
        }
    };
    let plain = placed("0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0]");
    // The operands and the move they make from the plain placement (a b c d e f): those of
    // `a_pattern_fill_applies_its_offset_scale_rotation_reflection_and_shear` with y down, as the
    // art's space has it.
    let cases = [
        ("10 15 1 1 0 0 0 0 0 [1 0 0 1 0 0]", [1.0, 0.0, 0.0, 1.0, 10.0, -15.0]),
        ("-5 7 2 .5 -45 0 0 -15 45 [1 0 0 1 0 0]", [1.793151, 0.353553, -1.035276, 0.353553, -5.0, -7.0]),
    ];
    for (ops, want) in cases {
        let got = (placed(ops) * plain.inverse()).as_coeffs();
        assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 1e-6), "{ops}: {got:?}");
    }
    let r = read_with("0 0 1 1 0 0 0 0 [1 0 0 1 0 0]");
    assert!(!vectorcraft_brush::library(&r.doc).iter().any(|b| b.name == "Spray"), "{:?}", r.warnings);
}

/// A Scatter brush whose 15 numbers start with 1 colors its art with Tints of the stroke, and one
/// whose 15 numbers start with 0 keeps the art's colors. The field before the art's name doesn't
/// count: a brush whose first field is 1 is read by its 15 numbers, and one with 8 numbers, as older
/// files write their brushes, isn't read as a Scatter brush.
#[test]
fn a_scatter_brush_in_tints_comes_in_tinted() {
    use vectorcraft_brush::{BrushKind, Colorization};
    let read = |params: &str| {
        let r = read_ok(&format!("{}{}{}", brush_art("Dot art", 4.0), plugin_object(SCATTER, "Spray", params), layer("")));
        vectorcraft_brush::library(&r.doc).into_iter().find(|b| b.name == "Spray")
    };
    for (field, first, colorization) in
        [("2", "1", Colorization::Tints), ("2", "0", Colorization::None), ("1", "1", Colorization::Tints), ("1", "0", Colorization::None)]
    {
        let params = format!("{field} /Dot art/ 5 0 0 0 / {first} 1 1 0 1 1 0 0 0 0 0 0 0 0 0");
        let Some(b) = read(&params) else { panic!("{params}: not read") };
        let BrushKind::Scatter(s) = b.kind else { panic!("scatter") };
        assert_eq!(s.colorization, colorization, "{params}");
    }
    let older = "1 /Dot art/ 0 1 /  1 0 1 0 1 0 0 0";
    assert_eq!(brush::parse_tool_object(brush::Tool::Scatter, "Spray", older), Err(brush::BrushError::Values));
    assert!(read(older).is_none(), "8 numbers aren't a Scatter brush's");
}

/// A brush's art section `name`: in a group, a frame (a rectangle from `frame.0` to `frame.1` on
/// both axes, painted by `op` and followed by `after`) and a filled square from 0 to 4.
fn framed_art(name: &str, frame: (f64, f64), op: &str, after: &str) -> String {
    format!(
        "%AI8_BeginBrushPattern\n({name})\n0 A\n0 Xw\nu\n0 XR\n{}{op}\n{after}0 g\n{}f\nU\n%AI8_EndBrushPattern\n",
        rect_path(frame.0, frame.0, frame.1, frame.1),
        rect_path(0.0, 0.0, 4.0, 4.0)
    )
}

/// The frame of a brush's art, an unnamed, unpainted rectangle at the art's bounds, isn't part of
/// the art, closed or open with its first corner again at the end. A frame wider than the art is
/// part of it, and so are a painted rectangle and a named one, which aren't frames.
#[test]
fn a_brushs_frame_is_left_out_only_at_the_arts_bounds() {
    use vectorcraft_brush::BrushKind;
    let named = "%_/ArtDictionary :\n%_/XMLUID : (Edge) ; (AI10_ArtUID) ,\n%_;\n%_\n";
    for (why, frame, op, after, kept) in [
        ("exact", (0.0, 4.0), "n", "", false),
        ("open", (0.0, 4.0), "N", "", false),
        ("wider", (-1.0, 5.0), "n", "", true),
        ("filled", (0.0, 4.0), "f", "", true),
        ("named", (0.0, 4.0), "n", named, true),
    ] {
        let params = "2 /Dot art/ 5 0 0 0 / 0 1 1 0 1 1 0 0 0 0 0 0 0 0 0";
        let r = read_ok(&format!("{}{}{}", framed_art("Dot art", frame, op, after), plugin_object(SCATTER, "Spray", params), layer("")));
        let lib = vectorcraft_brush::library(&r.doc);
        let BrushKind::Scatter(s) = brush_named(&lib, "Spray") else { panic!("scatter") };
        if kept {
            assert!(matches!(s.art.kind, NodeKind::Group { .. }), "{why}: {:#?}", s.art);
            assert_eq!(s.art.children().map(|c| c.len()), Some(2), "{why}");
        } else {
            assert_eq!(s.art.geometric_bounds().map(|b| b.width()), Some(4.0), "{why}: {:#?}", s.art);
            assert!(matches!(s.art.kind, NodeKind::Path { .. }), "{why}: the square alone: {:#?}", s.art);
        }
    }
}

/// A brush's art group that is hidden, locked, isolated or has a note keeps that state when its
/// frame is left out: the group stays, without the frame, rather than becoming its one other object.
#[test]
fn a_brushs_frame_leaves_the_groups_own_state() {
    use vectorcraft_doc::{Appearance, FillLayer, NodeId};
    let square = Rect::new(0.0, 0.0, 4.0, 4.0);
    let paint = FillLayer::new(Paint::solid(Color::BLACK));
    let ap = Appearance { items: vec![AppearanceItem::Fill(paint)], effects: vec![], contents_index: None };
    let dot = Node::path(NodeId(2), vectorcraft_geom::shapes::rectangle(square), ap);
    let frame = Node::path(NodeId(1), vectorcraft_geom::shapes::rectangle(square), Default::default());
    let group = Node::group(NodeId(3), vec![Arc::new(frame), Arc::new(dot)]);
    assert!(matches!(super::brush::without_frame(group.clone()).kind, NodeKind::Path { .. }), "a plain group is its one object");
    // A closed path through the four corners across the square, not along its sides, isn't a frame.
    let corners = [Point::new(0.0, 0.0), Point::new(4.0, 4.0), Point::new(4.0, 0.0), Point::new(0.0, 4.0)];
    let crossing = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&corners, true));
    let bowtie = Node::path(NodeId(4), crossing, Default::default());
    let dot = group.children().and_then(|c| c.last().cloned()).unwrap();
    let crossed = Node::group(NodeId(5), vec![Arc::new(bowtie), dot]);
    assert_eq!(super::brush::without_frame(crossed).children().map(|c| c.len()), Some(2), "a path across the corners stays");
    // An open path along three sides isn't a frame; one that comes back to its first corner is.
    let sides = [Point::new(0.0, 0.0), Point::new(4.0, 0.0), Point::new(4.0, 4.0), Point::new(0.0, 4.0)];
    let around = [sides.as_slice(), &sides[..1]].concat();
    for (why, anchors, frame) in [("three sides", sides.as_slice(), false), ("back to the first corner", around.as_slice(), true)] {
        let open = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(anchors, false));
        let dot = group.children().and_then(|c| c.last().cloned()).unwrap();
        let g = Node::group(NodeId(6), vec![Arc::new(Node::path(NodeId(7), open, Default::default())), dot]);
        let left = if frame { None } else { Some(2) };
        assert_eq!(super::brush::without_frame(g).children().map(|c| c.len()), left, "{why}");
    }
    // A rectangle with settings of its own (a note, Show Center, hidden) isn't a frame.
    for why in ["note", "show center", "hidden"] {
        let mut g = group.clone();
        if let Some(Some(first)) = g.children_mut().map(|c| c.first_mut()) {
            let first = Arc::make_mut(first);
            match why {
                "note" => first.extras_mut().attrs = Some(Box::new(vectorcraft_doc::ObjectAttributes { note: "edge".into(), ..Default::default() })),
                "show center" => {
                    first.extras_mut().attrs = Some(Box::new(vectorcraft_doc::ObjectAttributes { show_center: Some(true), ..Default::default() }))
                }
                _ => first.visible = false,
            }
        }
        assert_eq!(super::brush::without_frame(g).children().map(|c| c.len()), Some(2), "{why}: the rectangle stays");
    }
    for why in ["hidden", "locked", "isolated", "note"] {
        let mut g = group.clone();
        (g.visible, g.locked, g.isolate) = (why != "hidden", why == "locked", why == "isolated");
        if why == "note" {
            g.extras_mut().attrs = Some(Box::new(vectorcraft_doc::ObjectAttributes { note: "edge".into(), ..Default::default() }));
        }
        let out = super::brush::without_frame(g);
        assert!(matches!(out.kind, NodeKind::Group { .. }), "{why}: {out:#?}");
        assert_eq!(out.children().map(|c| c.len()), Some(1), "{why}: the frame left out");
        assert_eq!((out.visible, out.locked, out.isolate), (why != "hidden", why == "locked", why == "isolated"), "{why}");
        assert_eq!(out.extras().attrs.as_ref().map(|a| a.note.as_str()), (why == "note").then_some("edge"), "{why}");
    }
}

/// Leaving out a frame that is exactly the art's bounds doesn't move what a Scatter or Art brush
/// draws: the pieces along a path are where they are with the frame kept.
#[test]
fn a_brushs_frame_at_the_arts_bounds_draws_the_same_without_it() {
    use vectorcraft_brush::{ArtBrush, Brush, BrushKind, Scatter};
    let scatter = ("Spray", "Dot art", SCATTER, "2 /Dot art/ 5 0 0 0 / 0 1 1 0 1 1 0 0 0 0 0 0 0 0 0");
    for (name, art_name, tool, params) in [scatter, ("Ribbon", "Ribbon art", ART_BRUSH, RIBBON)] {
        let r = read_ok(&format!("{}{}{}", framed_art(art_name, (0.0, 4.0), "n", ""), plugin_object(tool, name, params), layer("")));
        let lib = vectorcraft_brush::library(&r.doc);
        let read = lib.iter().find(|b| b.name == name).cloned().unwrap();
        let art = match &read.kind {
            BrushKind::Scatter(s) => s.art.clone(),
            BrushKind::Art(a) => a.art.clone(),
            k => panic!("{name}: {k:?}"),
        };
        let bounds = art.geometric_bounds().unwrap();
        assert_eq!((bounds.width(), bounds.height()), (4.0, 4.0), "{name}");
        assert!(matches!(art.kind, NodeKind::Path { .. }), "{name}: the frame left out: {art:#?}");
        let frame = Node::path(vectorcraft_doc::NodeId(90), vectorcraft_geom::shapes::rectangle(bounds), Default::default());
        let framed_art = Node::group(vectorcraft_doc::NodeId(91), vec![Arc::new(frame), Arc::new(art)]);
        let kind = match &read.kind {
            BrushKind::Scatter(s) => BrushKind::Scatter(Scatter { art: framed_art, ..s.clone() }),
            BrushKind::Art(a) => BrushKind::Art(ArtBrush { art: framed_art, ..a.clone() }),
            k => panic!("{name}: {k:?}"),
        };
        let framed = Brush { kind, ..read.clone() };
        let mut line = kurbo::BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((100.0, 30.0));
        let stroke = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::BLACK), 1.0);
        let at = |b: &Brush| -> Vec<[i64; 4]> {
            vectorcraft_brush::stroke_pieces(&r.doc, b, &line, &stroke)
                .iter()
                .filter_map(|n| n.geometric_bounds())
                .map(|b| [b.x0, b.y0, b.x1, b.y1].map(|v| (v * 1000.0).round() as i64))
                .collect()
        };
        let (without, with) = (at(&read), at(&framed));
        assert!(!without.is_empty(), "{name}");
        if name == "Spray" {
            assert!(without.len() > 10, "{without:?}");
        }
        assert_eq!(without, with, "{name}");
    }
}

/// A file without brush objects keeps the default brushes.
#[test]
fn a_file_without_brushes_keeps_the_default_ones() {
    let r = read_ok(&layer(&(rect_path(0.0, 0.0, 10.0, 10.0) + "f\n")));
    assert!(!r.doc.unknown.contains_key(vectorcraft_brush::DOC_KEY));
}

/// A brush whose object can't be read, or whose art is missing or can't be read, is left out with a
/// note; the layers and the other brushes still come in.
#[test]
fn a_brush_that_cant_be_read_leaves_the_layers_and_other_brushes() {
    let bad_art = "%AI8_BeginBrushPattern\n(Odd art)\n0 g\n0 0 m\n1 1 L\nnotanoperator\nf\n%AI8_EndBrushPattern\n";
    let objects = [
        plugin_object(CALLIGRAPHIC, "Nib", NIB),
        plugin_object(CALLIGRAPHIC, "Short", "1 2 3"),
        plugin_object(CALLIGRAPHIC, "Huge", "1 2 inf 40 60 -30 10 1 0 2 0"),
        plugin_object(SCATTER, "Odd", &SPRAY.replace("Dot art", "Odd art")),
        plugin_object(SCATTER, "Lost", &SPRAY.replace("Dot art", "Nowhere")),
        plugin_object(ART_BRUSH, "Bare", "2 / / / / / / 0 0/ 5 0 0 0 /  1 1 1 0 1 0 0 0 1 1 0 0 0 0 1"),
        plugin_object(BRISTLE, "Nib", MOP),
    ]
    .concat();
    let r = read_ok(&format!("{bad_art}{objects}{}", layer(&(rect_path(0.0, 0.0, 10.0, 10.0) + "f\n"))));
    assert_eq!(art(&r.doc).len(), 1, "the layer's art");
    let lib = vectorcraft_brush::library(&r.doc);
    assert_eq!(lib.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Nib"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.ends_with("Brushes panel (6)")), "{:?}", r.warnings);
}

/// Files with brush objects and no brushes VectorCraft reads get an empty list, not the defaults.
#[test]
fn a_file_whose_brushes_are_all_left_out_has_none() {
    let r = read_ok(&format!("{}{}", plugin_object(PATTERN_BRUSH, "Fence", FENCE), layer("")));
    assert!(vectorcraft_brush::library(&r.doc).is_empty());
}

/// A brush object that ends after its plug-in name is counted as a brush left out, and the file
/// gets no default brushes.
#[test]
fn a_brush_object_without_a_name_is_counted_as_left_out() {
    let nameless = format!("%AI5_Begin_NonPrinting\nNp\n%AI8_BeginPluginObject\n({CALLIGRAPHIC})\n%AI8_EndPluginObject\n%AI5_End_NonPrinting--\n");
    let r = read_ok(&format!("{nameless}{}", layer("")));
    assert!(vectorcraft_brush::library(&r.doc).is_empty(), "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.ends_with("Brushes panel (1)")), "{:?}", r.warnings);
    let r = read_ok(&format!("{}{nameless}{}", plugin_object(CALLIGRAPHIC, "Nib", NIB), layer("")));
    let lib = vectorcraft_brush::library(&r.doc);
    assert_eq!(lib.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["Nib"], "{:?}", r.warnings);
    assert!(r.warnings.iter().any(|w| w.ends_with("Brushes panel (1)")), "{:?}", r.warnings);
}

/// A gradient's `Br` (a stop's ramp) isn't a brush: the gradient keeps its stops.
#[test]
fn a_gradients_br_is_not_a_brush() {
    let gradient =
        "%AI5_BeginGradient: (G)\n(G) 0 2 Bd\n[\n0 0 0 0 1 1 1 2 1 6 50 0 Bs\n0 0 0 1 0 0 0 2 0.5 6 50 100 Bs\n0 Br\nBD\n%AI5_EndGradient\n";
    let art_ = format!("{}Bb\n1 (G) 0 0 0 1 1 0 0 1 0 0 1 Bg\n10 0 0 10 100 50 Xm\nf\n0 BB\n", rect_path(0.0, 0.0, 20.0, 20.0));
    let r = read_ok(&format!("{gradient}{}", layer(&art_)));
    assert!(!r.doc.unknown.contains_key(vectorcraft_brush::DOC_KEY));
    let Paint::Gradient(g) = fill(&art(&r.doc)[0]) else { panic!("a gradient: {:?}", r.warnings) };
    assert_eq!(g.gradient.stops.len(), 2);
}

/// The Brush Options codes for how a Calligraphic brush's values vary: Tilt, Bearing and Rotation
/// are the pen's other channels (#372: a 6D Art Pen brush varies its diameter with the pressure,
/// its roundness with the bearing and its angle with the barrel's rotation). The stylus wheel isn't
/// read: it varies at random, as an unknown code does.
#[test]
fn a_calligraphic_brushs_pen_channels_come_in_by_their_codes() {
    use vectorcraft_brush::Variation::{Bearing, Fixed, Pressure, Random, Rotation, Tilt};
    let modes = |params: &str| {
        let d = brush::parse_tool_object(brush::Tool::Calligraphic, "Pen", params).unwrap();
        let brush::DraftKind::Calligraphic(c) = d.kind else { panic!("{d:?}") };
        c.modes.unwrap()
    };
    assert_eq!(modes("1 51 69 13 57 -346 14 2 5 6 0"), [Rotation, Bearing, Pressure]);
    assert_eq!(modes("1 2 6 40 60 -30 10 4 4 4 0"), [Tilt, Tilt, Tilt]);
    assert_eq!(modes("1 2 6 40 60 -30 10 3 3 3 0"), [Random, Random, Random], "the stylus wheel");
    assert_eq!(modes("1 4 4 50 50 10 10 3 9 9 0"), [Fixed, Fixed, Fixed], "nothing to vary");
}

/// A 6D Art Pen brush: diameter 4 to 8 with the pressure, roundness 30 to 50 with the bearing and
/// angle -150 to 210 with the barrel's rotation.
const SIX_D: &str = "1 4 8 30 50 -150 210 2 5 6 0";

/// The style `Inked`: a stroke in red of weight `weight` with the Calligraphic brush `brush` at
/// diameter, roundness and angle `settings` (each its value and variation either side), varying by
/// the codes `codes`, its angle relative to `relative` (0 the page).
fn brushed_style(brush: &str, weight: f64, settings: [(f64, f64); 3], codes: [u8; 3], relative: u8) -> String {
    let [(d, dv), (r, rv), (a, av)] = settings;
    let [dm, rm, am] = codes;
    format!(
        "%AI9_BeginArtStyles\n/KnownStyle :\n(Inked) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n({CALLIGRAPHIC}) 1 0 /Filter ,\n1 /Visible ,\n(Calligraphic Brush) /Title ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n\
         /StrokeStyle : 0 R\n0 1 1 0 1 0 0 XA\n0 1 0 0 0 Xy\n0 J 0 j {weight} w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n\
         {d} /Real (Diameter) ,\n{dv} /Real (DiameterVariation) ,\n{dm} /Int (DiameterDependsOn) ,\n\
         {r} /Real (Roundness) ,\n{rv} /Real (RoundnessVariation) ,\n{rm} /Int (RoundnessDependsOn) ,\n\
         {a} /Real (Angle) ,\n{av} /Real (AngleVariation) ,\n{am} /Int (AngleDependsOn) ,\n{relative} /Int (AngleRelativeTo) ,\n\
         ({brush}) /UnicodeString (DisplayString) ,\n/GObjRef : ({brush}) ({CALLIGRAPHIC}) /PluginObject ; (Brush) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n\
         /FillStyle : 0 O\n0 0 0 0 k\n0 0 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         ;\n /Part ,\n;\n%AI9_EndArtStyles\n"
    )
}

/// The settings [`SIX_D`] has, as an art style writes them.
const SIX_D_SETTINGS: [(f64, f64); 3] = [(6.0, 2.0), (40.0, 10.0), (30.0, 180.0)];

/// A brushed path as the file writes it: its drawn look (a filled outline) in a group, then the
/// path itself on `%_` lines, unpainted, with the style `Inked`.
fn brushed_path() -> String {
    let line = "20 20 m\n100 60 L\n";
    format!("0 Ae\nu\n{RED}{}f\nU\n0 0 Xd\n6 () XW\n%_{}n\n1 (Inked) XW\n", rect_path(20.0, 18.0, 100.0, 62.0), line.replace('\n', "\n%_"))
}

/// A path drawn with a Calligraphic brush comes in as the path, its stroke in that brush at its
/// weight (the weight scales the brush), not as its drawn look: here a 6D Art Pen brush, whose
/// channels the brush keeps (#372).
#[test]
fn a_path_with_a_calligraphic_brush_comes_in_brushed() {
    let def = plugin_object(CALLIGRAPHIC, "Six D", SIX_D);
    let style = brushed_style("Six D", 0.5, SIX_D_SETTINGS, [2, 5, 6], 0);
    let r = read_ok(&format!("{def}{style}{}", layer(&brushed_path())));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the path, not its look: {a:#?}");
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.brush.as_deref(), st.width, st.paint.color()), (Some("Six D"), 0.5, Some(Color::rgb(1.0, 0.0, 0.0))));
    assert!(a[0].appearance.fill().is_none_or(|f| f.paint.is_none()), "{:?}", a[0].appearance);
    use vectorcraft_brush::{BrushKind, Variation};
    let lib = vectorcraft_brush::library(&r.doc);
    let BrushKind::Calligraphic(c) = brush_named(&lib, "Six D") else { panic!("calligraphic") };
    assert_eq!(c.modes, Some([Variation::Rotation, Variation::Bearing, Variation::Pressure]));
    // With no pen recorded on the path, the nib is the brush's own at the stroke's weight.
    let pieces = vectorcraft_brush::stroke_pieces(&r.doc, &lib[0], &a[0].path_data().unwrap().to_bezpath(), st);
    let height = pieces.iter().filter_map(|n| n.geometric_bounds()).map(|b| b.height()).fold(0.0, f64::max);
    assert!(height > 40.0 && height < 46.0, "the line's rise and about half the nib's 3 pt: {height}");
}

/// A brushed path keeps its drawn look when its stroke can't be read as the brush: the object's
/// own settings differ from the brush's (Options of Selected Object), its angle follows the path,
/// or the file defines no brush of its name.
#[test]
fn a_brushed_path_the_brush_cant_draw_keeps_its_drawn_look() {
    let def = plugin_object(CALLIGRAPHIC, "Six D", SIX_D);
    let mut own = SIX_D_SETTINGS;
    own[0] = (9.0, 2.0);
    let cases = [
        ("its own settings", def.clone(), brushed_style("Six D", 1.0, own, [2, 5, 6], 0)),
        ("its own modes", def.clone(), brushed_style("Six D", 1.0, SIX_D_SETTINGS, [2, 5, 4], 0)),
        ("an angle on the path", def.clone(), brushed_style("Six D", 1.0, SIX_D_SETTINGS, [2, 5, 6], 1)),
        ("no such brush", def, brushed_style("Other", 1.0, SIX_D_SETTINGS, [2, 5, 6], 0)),
    ];
    for (why, def, style) in cases {
        let r = read_ok(&format!("{def}{style}{}", layer(&brushed_path())));
        let a = art(&r.doc);
        assert!(matches!(a[0].kind, NodeKind::Group { .. }), "{why}: the drawn look, {a:#?}");
        assert!(r.warnings.iter().any(|w| w.contains("drawn look")), "{why}: {:?}", r.warnings);
    }
}

/// The entries of an Art brush's filter for a path that stretches its art, at the brush's own width.
const STRETCHED: [(&str, &str, &str); 12] = [
    ("Proportional", "0", "Bool"),
    ("Segmented", "0", "Bool"),
    ("Scaling", "1", "Real"),
    ("MinScaling", "1", "Real"),
    ("MaxScaling", "1", "Real"),
    ("ScaleYDependsOn", "0", "Int"),
    ("Spacing", "0", "Real"),
    ("UpsideDown", "0", "Int"),
    ("Counterclockwise", "0", "Int"),
    ("Adjustment", "1", "Int"),
    ("FoldAdjustment", "1", "Bool"),
    ("Colorization", "1", "Int"),
];

/// The style `Inked`: a stroke in red of weight `weight` with the Art brush `brush`, with the entries
/// `entries` and `more` after them.
fn art_style(brush: &str, weight: f64, entries: &[(&str, &str, &str)], more: &str) -> String {
    let entries: String = entries.iter().map(|(key, value, kind)| format!("{value} /{kind} ({key}) ,\n")).collect();
    format!(
        "%AI9_BeginArtStyles\n/KnownStyle :\n(Inked) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
         /BasicFilter :\n({ART_BRUSH}) 1 0 /Filter ,\n1 /Visible ,\n(Art Brush) /Title ,\n2 /FillOrStroke ,\n/Dictionary : /NotRecorded ,\n\
         {entries}{more}/StrokeStyle : 0 R\n0 1 1 0 1 0 0 XA\n0 1 0 0 0 Xy\n0 J 0 j {weight} w 10 M []0 d\n0 XR\n1 0 Xd\n/Def ;\n (StrokeStyle) ,\n\
         ({brush}) /UnicodeString (DisplayString) ,\n/GObjRef : ({brush}) ({ART_BRUSH}) /PluginObject ; (Brush) ,\n; /Dict ;\n /Part ,\n\
         /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n\
         /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n\
         /FillStyle : 0 O\n0 0 0 0 k\n0 0 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
         ;\n /Part ,\n;\n%AI9_EndArtStyles\n"
    )
}

/// A brush's art named `name`: a wide block with a narrow one on top of it (art space: y up).
fn lopsided_art(name: &str) -> String {
    format!(
        "%AI8_BeginBrushPattern\n({name})\n0 A\n0 g\n{}f\n{}f\n%AI8_EndBrushPattern\n",
        rect_path(0.0, 0.0, 40.0, 4.0),
        rect_path(0.0, 4.0, 10.0, 8.0)
    )
}

/// A path drawn with an Art brush comes in as the path, its stroke in that brush at its weight (the
/// weight scales the art across the path), not as its drawn look. The art runs along the path as
/// the file draws it: the side of its section with y up goes to the right of the path's direction.
#[test]
fn a_path_with_an_art_brush_comes_in_brushed() {
    let def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let more = "15 /Int (Adobe Effect Expand Before Version) ,\n";
    let r = read_ok(&format!("{def}{}{}", art_style("Ribbon", 2.0, &STRETCHED, more), layer(&brushed_path())));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "the path, not its look: {a:#?}");
    assert!(!r.warnings.iter().any(|w| w.contains("drawn look")), "{:?}", r.warnings);
    let st = a[0].appearance.stroke().unwrap();
    assert_eq!((st.brush.as_deref(), st.width, st.paint.color()), (Some("Ribbon"), 2.0, Some(Color::rgb(1.0, 0.0, 0.0))));
    let lib = vectorcraft_brush::library(&r.doc);
    let vectorcraft_brush::BrushKind::Art(brush) = brush_named(&lib, "Ribbon") else { panic!("art") };
    assert_eq!((brush.width, brush.colorization), (100.0, vectorcraft_brush::Colorization::Tints));
    // Along a straight line, y down: the art is 8 tall at weight 1, its narrow block below the wide one.
    let line = kurbo::BezPath::from_vec(vec![kurbo::PathEl::MoveTo(Point::new(0.0, 0.0)), kurbo::PathEl::LineTo(Point::new(100.0, 0.0))]);
    let pieces = vectorcraft_brush::stroke_pieces(&r.doc, &lib[0], &line, st);
    let b = pieces.iter().filter_map(|n| n.geometric_bounds()).reduce(|a, b| a.union(b)).unwrap();
    assert!((b.height() - 16.0).abs() < 1e-3 && (b.width() - 100.0).abs() < 1e-3, "weight 2 doubles the art across, and it stretches along: {b:?}");
    assert!((b.y0 + 8.0).abs() < 1e-3, "{b:?}");
    let filled = |x: f64, y: f64| {
        pieces.iter().any(|n| {
            let mut hit = false;
            n.walk(&mut |c| hit |= c.path_data().is_some_and(|p| kurbo::Shape::winding(&p.to_bezpath(), Point::new(x, y)) != 0));
            hit
        })
    };
    assert!(filled(50.0, -4.0), "the wide block is above the line");
    assert!(filled(10.0, 4.0) && !filled(50.0, 4.0), "the narrow block is below it");
}

/// A fill under the brush stays on the path.
#[test]
fn a_filled_path_with_an_art_brush_comes_in_brushed() {
    let def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let style = art_style("Ribbon", 1.0, &STRETCHED, "").replace("0 0 0 0 k\n0 0 Xd", "0 0 1 0 k\n0 1 Xd");
    let r = read_ok(&format!("{def}{style}{}", layer(&brushed_path())));
    let a = art(&r.doc);
    assert!(matches!(a[0].kind, NodeKind::Path { .. }), "{a:#?}");
    assert_eq!(a[0].appearance.items.len(), 2, "{:?}", a[0].appearance);
    assert_eq!(a[0].appearance.stroke().and_then(|s| s.brush.as_deref()), Some("Ribbon"));
}

/// A compound path with an Art brush comes in as the compound path, its stroke in that brush.
#[test]
fn a_compound_path_with_an_art_brush_comes_in_brushed() {
    let def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let object = format!(
        "0 Ae\nu\n{RED}{}f\nU\n0 0 Xd\n6 () XW\n%_*u\n%_20 20 m\n%_100 20 L\n%_n\n%_20 60 m\n%_100 60 L\n%_n\n%_*U\n1 (Inked) XW\n",
        rect_path(20.0, 18.0, 100.0, 62.0)
    );
    let r = read_ok(&format!("{def}{}{}", art_style("Ribbon", 1.0, &STRETCHED, ""), layer(&object)));
    let a = art(&r.doc);
    assert_eq!(a.len(), 1, "{a:#?}");
    assert!(matches!(a[0].kind, NodeKind::Compound { .. }), "{a:#?}");
    assert_eq!(a[0].appearance.stroke().and_then(|s| s.brush.as_deref()), Some("Ribbon"));
}

/// An Art brush draws only art of paths, compound paths and groups in solid fills: not an image, a
/// mask, an effect, a gradient or pattern, a stroke, or art with no length along the path.
#[test]
fn an_art_brush_bends_only_paths_in_solid_fills() {
    use vectorcraft_color::gradient::{Gradient, GradientPaint, GradientStop};
    use vectorcraft_doc::{Effect, ImageObject, NodeId, OpacityMask};
    let def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let r = read_ok(&format!("{def}{}", layer(&brushed_path())));
    let lib = vectorcraft_brush::library(&r.doc);
    let vectorcraft_brush::BrushKind::Art(brush) = brush_named(&lib, "Ribbon") else { panic!("art") };
    let plain = &brush.art;
    assert!(brush::art_bends(plain), "{plain:#?}");
    let with = |change: &dyn Fn(&mut Node)| {
        let mut n = plain.clone();
        change(&mut n);
        n
    };
    let image = ImageObject { key: "k".into(), width: 4, height: 4, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)];
    let gradient = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let cases = [
        ("an image", Node::new(NodeId(90), NodeKind::Image(Box::new(image)))),
        ("a mask", with(&|n| n.extras_mut().mask = Some(Box::new(OpacityMask::new(plain.clone(), true))))),
        ("an effect", with(&|n| n.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: Default::default(), visible: true }))),
        (
            "an effect on a fill",
            with(&|n| {
                n.appearance.set_fill(Paint::solid(Color::BLACK));
                if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
                    f.effects.push(Effect { id: "stylize.dropShadow".into(), params: Default::default(), visible: true });
                }
            }),
        ),
        ("a gradient", with(&|n| n.appearance.set_fill(Paint::Gradient(Box::new(gradient.clone()))))),
        ("a stroke", with(&|n| n.appearance.set_stroke(Paint::solid(Color::BLACK)))),
    ];
    let wrong: Vec<&str> = cases.iter().filter(|(_, art)| brush::art_bends(art)).map(|(why, _)| *why).collect();
    assert!(wrong.is_empty(), "{wrong:?}");
}

/// An Art brushed path keeps its drawn look when the brush wouldn't draw it the way the file does:
/// its settings aren't those of art that stretches along the path at the brush's own width and
/// color, the brush's own aren't, its art isn't read, or the brush isn't one the panel keeps.
#[test]
fn an_art_brushed_path_the_brush_cant_draw_keeps_its_drawn_look() {
    let good = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let with = |key: &str, value: &str| -> Vec<(String, String, String)> {
        STRETCHED.iter().map(|(k, v, t)| ((*k).to_string(), if *k == key { value.to_string() } else { (*v).to_string() }, (*t).to_string())).collect()
    };
    let style_with = |key: &str, value: &str| {
        let owned = with(key, value);
        let entries: Vec<(&str, &str, &str)> = owned.iter().map(|(k, v, t)| (k.as_str(), v.as_str(), t.as_str())).collect();
        art_style("Ribbon", 1.0, &entries, "")
    };
    let plain_style = art_style("Ribbon", 1.0, &STRETCHED, "");
    let flipped = "2 / Ribbon art/ / / / / 0 0/ 5 0 0 0 /  1 1 1 0 1 1 0 0 1 1 0 0 0 0 1";
    let wide = "2 / Ribbon art/ / / / / 0 0/ 5 0 0 0 /  1 1 2 0 1 0 0 0 2 2 0 0 0 0 1";
    let own_colors = "2 / Ribbon art/ / / / / 0 0/ 5 0 0 0 /  1 0 1 0 1 0 0 0 1 1 0 0 0 0 1";
    let dots = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 4 4\n%_0 0 0 1 k\n%_1 1 m\n%_3 1 L\n%_3 3 L\n%_1 3 L\n%_f\nE\n%AI3_EndPattern\n";
    let patterned = format!(
        "%AI8_BeginBrushPattern\n(Ribbon art)\n0 A\n(Dots) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n%AI8_EndBrushPattern\n",
        rect_path(0.0, 0.0, 4.0, 4.0)
    );
    let malformed: Vec<(&str, &str, &str)> = STRETCHED.iter().map(|&(k, v, t)| if k == "Segmented" { (k, "1", "Real") } else { (k, v, t) }).collect();
    let cases = [
        ("a switch written as another kind", good.clone(), art_style("Ribbon", 1.0, &malformed, "")),
        ("art in a pattern", format!("{dots}{patterned}{}", plugin_object(ART_BRUSH, "Ribbon", RIBBON)), plain_style.clone()),
        ("tints of a swatch's ink", good.clone(), plain_style.replace("0 1 1 0 1 0 0 XA", "0 0 0 0 1 0.5 0 (Brand orange) 0 1 XK")),
        (
            "art with no length",
            format!(
                "%AI8_BeginBrushPattern\n(Ribbon art)\n0 A\n0 g\n0 0 m\n0 40 L\nf\n%AI8_EndBrushPattern\n{}",
                plugin_object(ART_BRUSH, "Ribbon", RIBBON)
            ),
            plain_style.clone(),
        ),
        (
            "art with a stroke",
            format!(
                "%AI8_BeginBrushPattern\n(Ribbon art)\n0 A\n0 G\n2 w\n{}S\n%AI8_EndBrushPattern\n{}",
                rect_path(0.0, 0.0, 40.0, 8.0),
                plugin_object(ART_BRUSH, "Ribbon", RIBBON)
            ),
            plain_style.clone(),
        ),
        (
            "a width other than the brush's 100%",
            format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", wide)),
            art_style(
                "Ribbon",
                1.0,
                &STRETCHED.map(|e| if matches!(e.0, "Scaling" | "MinScaling" | "MaxScaling") { (e.0, "2", e.2) } else { e }),
                "",
            ),
        ),
        (
            "a brush that keeps its art's colors",
            format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", own_colors)),
            art_style("Ribbon", 1.0, &STRETCHED.map(|e| if e.0 == "Colorization" { (e.0, "0", e.2) } else { e }), ""),
        ),
        ("an entry that isn't read", good.clone(), art_style("Ribbon", 1.0, &STRETCHED, "1 /Int (SomeOtherSetting) ,\n")),
        (
            "an effect on the path",
            good.clone(),
            plain_style.replace(
                ";\n /Part ,\n;\n%AI9_EndArtStyles",
                ";\n /Part ,\n/BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n2 /Int (blnd) ,\n0.75 /Real (opac) ,\n3 /Real (horz) ,\n4 /Real (vert) ,\n5 /Real (blur) ,\n1 /Int (csrc) ,\n50 /Real (dark) ,\n; /Dict ;\n /Part ,\n;\n%AI9_EndArtStyles",
            ),
        ),
        ("proportional", good.clone(), style_with("Proportional", "1")),
        ("between guides", good.clone(), style_with("Segmented", "1")),
        ("its own width", good.clone(), style_with("Scaling", "2")),
        ("a width that varies", good.clone(), style_with("MaxScaling", "2")),
        ("a width on the pen", good.clone(), style_with("ScaleYDependsOn", "2")),
        ("spacing", good.clone(), style_with("Spacing", "1")),
        ("flipped", good.clone(), style_with("UpsideDown", "1")),
        ("flipped the other way", good.clone(), style_with("Counterclockwise", "1")),
        ("another adjustment", good.clone(), style_with("Adjustment", "2")),
        ("no fold adjustment", good.clone(), style_with("FoldAdjustment", "0")),
        ("its own colorization", good.clone(), style_with("Colorization", "0")),
        ("tints and shades", good.clone(), style_with("Colorization", "2")),
        ("hue shift", good.clone(), style_with("Colorization", "3")),
        ("start of a guide", good.clone(), art_style("Ribbon", 1.0, &STRETCHED, "5 /Real (StartSegmentLength) ,\n")),
        ("variable width", good.clone(), art_style("Ribbon", 1.0, &STRETCHED, "2 /Int (BSWidthSource) ,\n")),
        (
            "width arrays",
            good.clone(),
            art_style(
                "Ribbon",
                1.0,
                &STRETCHED,
                "/Array :\n0 /Real ,\n1 /Real ,\n; (BSVariableWidthTValuesArray) ,\n/Array :\n1 /Real ,\n2 /Real ,\n; (BSCenteredWidthsArray) ,\n",
            ),
        ),
        ("a brush that flips", format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", flipped)), plain_style.clone()),
        ("another brush width", format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", wide)), plain_style.clone()),
        ("art not read", plugin_object(ART_BRUSH, "Ribbon", RIBBON), plain_style.clone()),
        ("no such brush", good.clone(), art_style("Other", 1.0, &STRETCHED, "")),
    ];
    let mut wrong = vec![];
    for (why, def, style) in cases {
        let r = read_ok(&format!("{def}{style}{}", layer(&brushed_path())));
        let lib = vectorcraft_brush::library(&r.doc);
        let art_read = why == "art not read" || why == "no such brush" || lib.iter().any(|b| b.name == "Ribbon");
        if !matches!(art(&r.doc)[0].kind, NodeKind::Group { .. }) || !r.warnings.iter().any(|w| w.contains("drawn look")) || !art_read {
            wrong.push(why);
        }
    }
    assert!(wrong.is_empty(), "these came in live, or their art wasn't read: {wrong:?}");
}

/// The name a stroke's brush has is the panel's, for a Calligraphic brush too: a left-out brush of
/// that name doesn't hide the one kept.
#[test]
fn a_calligraphic_brushed_stroke_takes_the_brush_the_panel_keeps_under_its_name() {
    let style = brushed_style("Six D", 1.0, SIX_D_SETTINGS, [2, 5, 6], 0);
    let path = layer(&brushed_path());
    let r = read_ok(&format!("{}{}{style}{path}", plugin_object(PATTERN_BRUSH, "Six D", FENCE), plugin_object(CALLIGRAPHIC, "Six D", SIX_D)));
    assert!(matches!(art(&r.doc)[0].kind, NodeKind::Path { .. }), "the pattern brush is left out");
}

/// The name a stroke's brush has is the panel's: a left-out brush of that name doesn't hide the one
/// kept, and a kept one of another kind isn't drawn as the name's Art brush.
#[test]
fn an_art_brushed_stroke_takes_the_brush_the_panel_keeps_under_its_name() {
    let style = art_style("Ribbon", 1.0, &STRETCHED, "");
    let path = layer(&brushed_path());
    let art_def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let r = read_ok(&format!("{}{art_def}{style}{path}", plugin_object(PATTERN_BRUSH, "Ribbon", FENCE)));
    assert!(matches!(art(&r.doc)[0].kind, NodeKind::Path { .. }), "the pattern brush is left out");
    let r = read_ok(&format!("{}{art_def}{style}{path}", plugin_object(CALLIGRAPHIC, "Ribbon", NIB)));
    assert!(matches!(art(&r.doc)[0].kind, NodeKind::Group { .. }), "the panel's Ribbon is the Calligraphic brush");
}

/// Type drawn with an Art brush keeps its drawn look under the type, which takes no style.
#[test]
fn type_with_an_art_brush_keeps_its_drawn_look() {
    let own: String = "/AI11Text :\n0 /FreeUndo ,\n0 /FrameIndex ,\n7 /StoryIndex ,\n;\n".lines().map(|l| format!("%_{l}\n")).collect();
    let object = format!(
        "0 Ae\nu\n/AI11Text :\n1 /FreeUndo ,\n0 /FrameIndex ,\n0 /StoryIndex ,\n;\n{RED}{}f\nU\n0 0 Xd\n6 () XW\n{own}1 (Inked) XW\n",
        rect_path(20.0, 18.0, 100.0, 62.0)
    );
    let def = format!("{}{}", lopsided_art("Ribbon art"), plugin_object(ART_BRUSH, "Ribbon", RIBBON));
    let r = read_ok(&format!("{def}{}{}", art_style("Ribbon", 1.0, &STRETCHED, ""), layer(&object)));
    let a = art(&r.doc);
    assert_eq!((a.len(), a[0].children().map(Vec::len), slot_of(a[1].name.as_deref())), (2, Some(1), Some(Some(7))), "the look and the type, {a:#?}");
    let mut brushed = 0;
    r.doc.walk(|n| brushed += usize::from(n.appearance.stroke().is_some_and(|s| s.brush.is_some())));
    assert_eq!(brushed, 0, "{:?}", r.warnings);
}

use read::{GRAPH_PLAIN, GRAPH_UNENDED};

/// Editing data in the legacy format (version 7 specification, chapter 2): `%AI5_FileFormat 2.0`,
/// the art size and the template box in the header, then `body`.
fn stream7(body: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%AI5_FileFormat 2.0\n%AI5_ArtSize: 612 792\n%AI3_TemplateBox: 306 396 306 396\n%%EndComments\n\
         %%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
}

/// One layer named `name` in the legacy format (ten operands before `Lb`) holding `art`.
fn layer7(name: &str, art: &str) -> String {
    format!("%AI5_BeginLayer\n1 1 1 1 0 0 0 79 128 255 Lb\n({name}) Ln\n0 A\n{art}\nLB\n%AI5_EndLayer--\n")
}

/// A graph's data on `%_` lines, in the order the version 7 specification's chapter 17 gives:
/// bounds, style and dialog settings, a subscription, three axes, a column width and the table
/// (two rows of two cells, one of them empty).
const GRAPH_DATA: &str = "%_Gs\n%_10 10 110 90 Gb\n%_3 0 0 12 1 1 0 0 2 30 0 Gy\n%_6 1 1 0 80 70 0\n%_ 50 60 80 70 Gd\n%_0 1 Gj\n\
    %_1 () Ga 0 12 0 0 0 0 0 () 0 0 GA\n%_2 () Ga 0 12 0 1 0.5 0 1 () 0 0 GA\n%_3 () Ga 0 12 0 4 1 0 0 () 0 0 GA\n%_40 Gw\n\
    %_2 2 1 1 Gz\n%_() Gc (East) Gc\n%_(Q1) Gc 7 Gc GC\n";

/// A graph's settings (`Gt` … `GT`): a design's name and paint operators on `%_` lines.
const GRAPH_SETTINGS: &str = "%_Gt\n%_(Bars) GD\n%_0.5 G\n%_0 g\n%_1 0 Gv\n%_GT\n";

/// A graph whose drawn art is `art` (in its group, then `%_GS`).
fn graph(settings: &str, art: &str) -> String {
    format!("{GRAPH_DATA}{settings}u\n{art}U\n%_GS\n")
}

/// A filled bar of a graph, with its `Go` tag.
fn bar(x: f64) -> String {
    format!("0 0 0 1 k\n{}f\n%_2 0 0 0 Go\n", rect_path(x, 10.0, x + 20.0, 50.0))
}

/// The gray (`k`) of `n`'s stroke.
fn stroke_k(n: &Node) -> Option<f32> {
    match n.appearance.stroke().and_then(|s| s.paint.color()) {
        Some(Color::Gray { k }) => Some(k),
        _ => None,
    }
}

/// A graph opens as its drawn art: the bar comes in, in its group, with one note.
#[test]
fn a_graph_opens_as_its_drawn_art() {
    let r = read(stream7(&layer7("Graph", &graph(GRAPH_SETTINGS, &bar(10.0)))).as_bytes()).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 1);
    let NodeKind::Group { children, clip: false } = &a[0].kind else { panic!("{:?}", a[0].kind) };
    assert_eq!(children.len(), 1);
    assert!(matches!(children[0].kind, NodeKind::Path { .. }));
    assert_eq!(fill(&children[0]).color(), Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    assert_eq!(r.warnings, [GRAPH_PLAIN]);
    assert!(r.hidden_unread.is_empty(), "the records aren't unread art: {:?}", r.hidden_unread);
    // Two graphs, the second without settings: one note still.
    let two = format!("{}{}", graph(GRAPH_SETTINGS, &bar(10.0)), graph("", &bar(40.0)));
    let r = read(stream7(&layer7("Graph", &two)).as_bytes()).unwrap();
    assert_eq!(art(&r.doc).len(), 2);
    assert_eq!(r.warnings, [GRAPH_PLAIN]);
    // A graph with only the records a minimal one needs (bounds and table), and the newer header.
    let minimal = "%_Gs\n%_0 0 50 50 Gb\n%_1 1 1 1 Gz\n%_3 Gc GC\n";
    let r = read_ok(&layer(&format!("{minimal}u\n{}U\n%_GS\n", bar(0.0))));
    assert_eq!(art(&r.doc).len(), 1);
    assert_eq!(r.warnings, [GRAPH_PLAIN]);
}

/// Paint operators on `%_` lines in a graph's settings set nothing; the same operators in its drawn
/// art, and after it, paint as usual.
#[test]
fn a_graphs_settings_paint_nothing_and_its_drawn_art_paints_as_usual() {
    let stroked = |x: f64| format!("{}S\n", rect_path(x, 60.0, x + 5.0, 70.0));
    let drawn = format!("{}0.25 G\n{}", stroked(0.0), stroked(10.0));
    let body = format!("0.75 G\n{}0.5 G\n{}", graph(GRAPH_SETTINGS, &drawn), stroked(20.0));
    let r = read(stream7(&layer7("Graph", &body)).as_bytes()).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 2);
    let kids = a[0].children().unwrap();
    assert_eq!(stroke_k(&kids[0]), Some(0.25), "the gray set before the graph, not its settings' `%_0.5 G`");
    assert_eq!(stroke_k(&kids[1]), Some(0.75), "a gray set in the drawn art");
    assert_eq!(stroke_k(&a[1]), Some(0.5), "a gray set after the graph");
}

/// Graph operators outside a graph stay operators the reader doesn't know: an error on a layer that
/// shows, noted as unread on `%_` lines (as any operator there), and no graph note.
#[test]
fn graph_operators_outside_a_graph_are_unknown() {
    for stray in ["Gs\n", "Go\n", "1 2 Gc\n"] {
        let err = read(stream7(&layer7("L", &format!("{}{stray}", rect_path(0.0, 0.0, 5.0, 5.0)))).as_bytes()).unwrap_err();
        assert!(err.contains("operators VectorCraft doesn't know"), "{stray}: {err}");
    }
    for (stray, op) in [("%_2 0 0 0 Go\n", "`Go`"), ("%_GS\n", "`GS`"), ("%_1 2 Gc\n", "`Gc`")] {
        let r = read(stream7(&layer7("L", &format!("{}f\n{stray}", rect_path(0.0, 0.0, 5.0, 5.0)))).as_bytes()).unwrap();
        assert!(r.hidden_unread.contains(op), "{stray}: {:?}", r.hidden_unread);
        assert!(r.warnings.is_empty(), "{stray}: {:?}", r.warnings);
    }
}

/// A graph that starts inside another is noted and the first goes on; one that doesn't end stops at
/// its layer's end, with the art read so far, and the next layer reads as usual.
#[test]
fn malformed_graphs_keep_their_art() {
    let nested = format!("{GRAPH_DATA}u\n{}%_Gs\n{}U\n%_GS\n", bar(0.0), bar(30.0));
    let r = read(stream7(&layer7("Graph", &nested)).as_bytes()).unwrap();
    assert_eq!(art(&r.doc)[0].children().unwrap().len(), 2);
    assert_eq!(r.warnings, [GRAPH_PLAIN, GRAPH_UNENDED]);

    let unended = format!("{GRAPH_DATA}%_Gt\nu\n{}U\n", bar(0.0));
    let next = format!("%_0.5 G\n{}S\n", rect_path(0.0, 60.0, 5.0, 70.0));
    let r = read(stream7(&format!("{}{}", layer7("Graph", &unended), layer7("Next", &next))).as_bytes()).unwrap();
    assert_eq!(art(&r.doc).len(), 1, "the drawn art read before the layer ended");
    let after = r.doc.layers[1].children().unwrap();
    assert_eq!(stroke_k(&after[0]), Some(0.5), "the next layer isn't in the graph's settings");
    assert_eq!(r.warnings, [GRAPH_PLAIN, GRAPH_UNENDED]);

    // One that doesn't end before the file does.
    let cut = stream7(&layer7("Graph", &format!("{GRAPH_DATA}u\n{}U\n", bar(0.0)))).replace("LB\n%AI5_EndLayer--\n", "");
    assert!(read(cut.as_bytes()).is_err(), "a layer that doesn't end is still an error");
}

/// A graph's records in the shapes the specification's appendix A gives: several cells on one line,
/// strings, arrays and names as operands, and empty groups among its drawn art.
#[test]
fn a_graphs_records_take_their_operands() {
    let settings = "%_Gt\n%_(Bars) GD [1 2 3] 0 Ge\n%_/Name 1 Gi (a) (b) Gl\n%_1 2 3 4 Gm 0 Gp 1 Gr 0 G+ 2 G1 0 Gf 0 Gg 0 Gx 1 GX\n%_GT\n";
    let drawn = format!("u\nU\nu\nu\nU\nU\n{}", bar(10.0));
    let r = read(stream7(&layer7("Graph", &graph(settings, &drawn))).as_bytes()).unwrap();
    let a = art(&r.doc);
    assert_eq!(a.len(), 1);
    let kids = a[0].children().unwrap();
    fn paths(n: &Node) -> usize {
        match n.children() {
            Some(c) => c.iter().map(|c| paths(c)).sum(),
            None => usize::from(matches!(n.kind, NodeKind::Path { .. })),
        }
    }
    assert!(matches!(kids.last().map(|k| &k.kind), Some(NodeKind::Path { .. })), "the bar is kept after the empty groups: {kids:?}");
    assert_eq!(paths(&a[0]), 1, "the empty groups hold no art");
    assert_eq!(r.warnings, [GRAPH_PLAIN]);
}

/// Editing data in the legacy format (`%AI5_FileFormat 2.0`, ten `Lb` operands), then `body`.
fn mask_stream7(body: &str) -> String {
    format!(
        "%!PS-Adobe-3.0\n%%Creator: test\n%%BoundingBox: 0 0 200 200\n%AI5_FileFormat 2.0\n%AI5_ArtSize: 200 200\n\
         %AI3_TemplateBox: 100 100 100 100\n%%EndComments\n%%BeginProlog\n%%EndProlog\n%%BeginSetup\n%%EndSetup\n{body}\n\
         %%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n"
    )
}

/// The start of a legacy layer named `name`, `masks` its multi-layer mask flag (`Lb`'s sixth operand).
fn begin7(name: &str, masks: u8) -> String {
    format!("%AI5_BeginLayer\n1 1 1 1 0 {masks} 1 79 128 255 Lb\n({name}) Ln\n")
}

const END7: &str = "LB\n%AI5_EndLayer--\n";

/// `Mb`, an 80 by 80 point mask path painted `W n`, `Md` (the version 7 specification's §10.4 order).
fn mask_start() -> String {
    format!("Mb\n{}h\nW\nn\nMd\n", rect_path(40.0, 40.0, 120.0, 120.0))
}

/// A filled square at `x`.
fn masked(x: f64) -> String {
    format!("0.5 0 0 0 k\n{}f\n", rect_path(x, x, x + 140.0, x + 140.0))
}

/// A layer's objects as a tree: `p` a path, `c` a clipping path, `o` a compound path, `g[…]` a
/// group, `k[…]` a clipping group, `l[…]` a sublayer.
fn mask_tree(nodes: &[Arc<Node>]) -> String {
    let one = |n: &Arc<Node>| match &n.kind {
        NodeKind::Layer { children, .. } => format!("l[{}]", mask_tree(children)),
        NodeKind::Group { children, clip: false } => format!("g[{}]", mask_tree(children)),
        NodeKind::Group { children, clip: true } => format!("k[{}]", mask_tree(children)),
        NodeKind::Path { clipping: true, .. } => "c".into(),
        NodeKind::Path { .. } => "p".into(),
        NodeKind::Compound { .. } => "o".into(),
        k => format!("{k:?}"),
    };
    nodes.iter().map(one).collect::<Vec<_>>().join(" ")
}

fn read7(body: &str) -> Structure {
    match read(mask_stream7(body).as_bytes()) {
        Ok(r) => r,
        Err(e) => panic!("{e}"),
    }
}

/// Each layer's tree, and whether the layer itself clips.
fn layer_trees(doc: &Document) -> Vec<(String, String, bool)> {
    doc.layers
        .iter()
        .map(|l| {
            let NodeKind::Layer { children, clip, .. } = &l.kind else { panic!("a layer") };
            (l.name.clone().unwrap_or_default(), mask_tree(children), *clip)
        })
        .collect()
}

/// A multi-layer mask (version 7 specification §10.4: `Mb`, the mask path, `Md`, objects in two
/// layers, `MB`) comes in as a clipping group in each layer it covers, each clipped by a copy of
/// the mask's path; what follows `MB` isn't clipped.
#[test]
fn a_multi_layer_mask_clips_each_layer_it_covers() {
    let body = format!("{}{}{}{END7}{}{}MB\n{}{END7}", begin7("A", 1), mask_start(), masked(15.0), begin7("B", 1), masked(30.0), masked(150.0));
    let r = read7(&body);
    assert_eq!(layer_trees(&r.doc), [("A".to_string(), "k[c p]".to_string(), false), ("B".to_string(), "k[c p] p".to_string(), false)]);
    let mask = |l: usize| r.doc.layers[l].children().unwrap()[0].children().unwrap()[0].clone();
    let (a, b) = (mask(0), mask(1));
    assert_ne!(a.id, b.id, "each layer has a copy of its own");
    assert_eq!(a.geometric_bounds(), b.geometric_bounds());
    let bounds = a.geometric_bounds().unwrap();
    assert_eq!((bounds.width(), bounds.height()), (80.0, 80.0));
    assert!(r.warnings.iter().any(|w| w.contains("clipping group in each layer")), "{:?}", r.warnings);
    // The layers' flag saying they have multi-layer masks changes nothing.
    let unflagged = read7(&body.replace(&begin7("A", 1), &begin7("A", 0)).replace(&begin7("B", 1), &begin7("B", 0)));
    assert_eq!(format!("{:?}", unflagged.doc.layers), format!("{:?}", r.doc.layers));
}

#[test]
fn a_multi_layer_mask_in_one_layer_or_over_three() {
    let one = format!("{}{}{}MB\n{}{END7}", begin7("A", 1), mask_start(), masked(15.0), masked(150.0));
    assert_eq!(layer_trees(&read7(&one).doc), [("A".to_string(), "k[c p] p".to_string(), false)]);
    let three = format!(
        "{}{}{}{END7}{}{}{END7}{}{}MB\n{END7}",
        begin7("A", 1),
        mask_start(),
        masked(15.0),
        begin7("B", 1),
        masked(30.0),
        begin7("C", 1),
        masked(45.0)
    );
    let trees: Vec<_> = layer_trees(&read7(&three).doc).into_iter().map(|(_, t, clip)| (t, clip)).collect();
    assert_eq!(trees, vec![("k[c p]".to_string(), false); 3]);
    // Objects before the mask's path are masked too; a layer between with nothing in it stays empty.
    let before = format!(
        "{}Mb\n{}{}h\nW\nn\nMd\n{END7}{}{END7}{}{}MB\n{END7}",
        begin7("A", 1),
        masked(15.0),
        rect_path(40.0, 40.0, 120.0, 120.0),
        begin7("B", 1),
        begin7("C", 1),
        masked(45.0)
    );
    let trees: Vec<_> = layer_trees(&read7(&before).doc).into_iter().map(|(_, t, _)| t).collect();
    assert_eq!(trees, ["k[c p]", "", "k[c p]"]);
}

#[test]
fn a_multi_layer_mask_in_a_group_or_sublayer() {
    // Begun and ended in a group of one layer.
    let grouped = format!("{}u\n{}{}MB\n{}U\n{END7}", begin7("A", 1), mask_start(), masked(15.0), masked(150.0));
    assert_eq!(layer_trees(&read7(&grouped).doc)[0].1, "g[k[c p] p]");
    // A sublayer in the mask: what the mask covers in the layer around it and in the sublayer.
    let nested = format!("{}{}{}{}{}{END7}{}MB\n{END7}", begin7("A", 1), mask_start(), masked(15.0), begin7("S", 1), masked(30.0), masked(45.0));
    assert_eq!(layer_trees(&read7(&nested).doc)[0].1, "k[c p] l[k[c p]] k[c p]");
}

#[test]
fn a_multi_layer_masks_path_can_be_painted_or_compound() {
    // `W f`: the path also paints, once, under what the mask covers after it: its fill is a path of its own in the first
    // layer, and the copies of the mask in each layer have no fill.
    let black = Some(Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let painted = format!("{}Mb\n0 0 0 1 k\n{}h\nW\nf\nMd\n{}MB\n{END7}", begin7("A", 1), rect_path(40.0, 40.0, 120.0, 120.0), masked(15.0));
    let doc = read7(&painted).doc;
    assert_eq!(layer_trees(&doc)[0].1, "k[c p p]");
    let group = doc.layers[0].children().unwrap()[0].clone();
    let kids = group.children().unwrap();
    assert_eq!(fill(&kids[0]), Paint::None, "the clipping path has no fill");
    assert_eq!(fill(&kids[1]).color(), black, "the painted path first");
    let two = format!(
        "{}Mb\n0 0 0 1 k\n{}h\nW\nf\nMd\n{}{END7}{}{}MB\n{END7}",
        begin7("A", 1),
        rect_path(40.0, 40.0, 120.0, 120.0),
        masked(15.0),
        begin7("B", 1),
        masked(30.0)
    );
    let doc = read7(&two).doc;
    assert_eq!(layer_trees(&doc), [("A".to_string(), "k[c p p]".to_string(), false), ("B".to_string(), "k[c p]".to_string(), false)]);
    let kids = |l: usize| doc.layers[l].children().unwrap()[0].children().unwrap().to_vec();
    let (a, b) = (kids(0), kids(1));
    assert_eq!((fill(&a[0]), fill(&b[0])), (Paint::None, Paint::None), "no copy of the mask paints");
    assert_eq!(fill(&a[1]).color(), black);
    assert_eq!(fill(&b[1]).color(), Some(Color::cmyk(0.5, 0.0, 0.0, 0.0)), "layer B's square, unpainted over");
    // A compound path of clipping paths.
    let compound = format!(
        "{}Mb\n*u\n{}h\nW\nn\n{}h\nW\nn\n*U\nMd\n{}MB\n{END7}",
        begin7("A", 1),
        rect_path(40.0, 40.0, 80.0, 80.0),
        rect_path(90.0, 90.0, 120.0, 120.0),
        masked(15.0)
    );
    assert_eq!(layer_trees(&read7(&compound).doc)[0].1, "k[o p]");
    // A filled one over two layers: its fill is a compound path of its own in the first, whose paths don't clip.
    let filled = format!(
        "{}Mb\n0 0 0 1 k\n*u\n{}h\nW\nf\n{}h\nW\nf\n*U\nMd\n{}{END7}{}{}MB\n{END7}",
        begin7("A", 1),
        rect_path(40.0, 40.0, 80.0, 80.0),
        rect_path(90.0, 90.0, 120.0, 120.0),
        masked(15.0),
        begin7("B", 1),
        masked(30.0)
    );
    let doc = read7(&filled).doc;
    assert_eq!(layer_trees(&doc), [("A".to_string(), "k[o o p]".to_string(), false), ("B".to_string(), "k[o p]".to_string(), false)]);
    let kids = |l: usize| doc.layers[l].children().unwrap()[0].children().unwrap().to_vec();
    let (a, b) = (kids(0), kids(1));
    assert_eq!((fill(&a[0]), fill(&b[0])), (Paint::None, Paint::None), "no copy of the mask paints");
    assert_eq!(fill(&a[1]).color(), black);
    let NodeKind::Compound { children: subpaths, .. } = &a[1].kind else { panic!("{:?}", a[1].kind) };
    assert!(subpaths.iter().all(|p| matches!(p.kind, NodeKind::Path { clipping: false, .. })), "{subpaths:#?}");
    let mut ids = std::collections::HashSet::new();
    for l in &doc.layers {
        l.walk(&mut |n| assert!(ids.insert(n.id), "{:?} appears twice", n.id));
    }
}

/// `Mv` starts a mask as `Mb` does, `ME` defines one as `Md` does, and `Mi` changes nothing.
#[test]
fn other_multi_layer_mask_operators() {
    let body = format!("{}{}{}{END7}{}{}MB\n{}{END7}", begin7("A", 1), mask_start(), masked(15.0), begin7("B", 1), masked(30.0), masked(150.0));
    let plain = format!("{:?}", read7(&body).doc.layers);
    let aliases = body.replace("Mb\n", "Mv\n").replace("Md\n", "ME\n");
    assert_eq!(format!("{:?}", read7(&aliases).doc.layers), plain);
    let with_mi = body.replace("Md\n", "Md\nMi\n").replace("MB\n", "Mi\nMB\n");
    assert_eq!(format!("{:?}", read7(&with_mi).doc.layers), plain);
}

#[test]
fn a_malformed_multi_layer_mask_is_an_error() {
    let path = format!("{}h\nW\nn\n", rect_path(40.0, 40.0, 120.0, 120.0));
    let sq = masked(15.0);
    let cases = [
        ("an end without a start", format!("{sq}MB\n")),
        ("a definition without a start", format!("{path}Md\n{sq}")),
        ("no mask path", format!("Mb\n{sq}Md\n{sq}MB\n")),
        ("a start inside another", format!("Mb\nMb\n{path}Md\n{sq}MB\n")),
        ("an end before the definition", format!("Mb\n{path}MB\n")),
        ("a definition in another group", format!("Mb\nu\n{path}Md\n{sq}MB\nU\n")),
        ("a group crossing the end", format!("{}u\n{sq}MB\nU\n", mask_start())),
        ("a group crossing the start", format!("u\n{}{sq}U\nMB\n", mask_start())),
    ];
    for (why, art) in cases {
        let body = format!("{}{art}{END7}", begin7("A", 1));
        let e = read(mask_stream7(&body).as_bytes()).unwrap_err();
        assert!(e.contains("mask"), "{why}: {e}");
    }
    let layered = [
        ("a layer ends before the definition", format!("{}Mb\n{sq}{END7}{}{path}Md\nMB\n{END7}", begin7("A", 1), begin7("B", 1))),
        ("a group open at the layer's end", format!("{}{}u\n{sq}{END7}{}U\nMB\n{END7}", begin7("A", 1), mask_start(), begin7("B", 1))),
        ("the end of the file inside the mask", format!("{}{}{sq}{END7}", begin7("A", 1), mask_start())),
    ];
    for (why, body) in layered {
        let e = read(mask_stream7(&body).as_bytes()).unwrap_err();
        assert!(e.contains("mask"), "{why}: {e}");
    }
    // Outside the layers, `Md` has nowhere to take its path from.
    let outside = format!("Mb\nMd\n{}{path}{sq}MB\n{END7}", begin7("A", 1));
    let e = read(mask_stream7(&outside).as_bytes()).unwrap_err();
    assert!(e.contains("mask"), "{e}");
}

/// A clipping path painted among the masked objects clips its whole layer, as outside a mask.
#[test]
fn a_layers_clipping_path_in_a_multi_layer_mask_clips_the_layer() {
    let path = format!("{}h\nW\nn\n", rect_path(10.0, 10.0, 190.0, 190.0));
    let body = format!("{}{}{}{path}{}MB\n{END7}", begin7("A", 1), mask_start(), masked(15.0), masked(30.0));
    assert_eq!(layer_trees(&read7(&body).doc), [("A".to_string(), "c k[c p p]".to_string(), true)]);
}

/// `Mb` before the first layer it covers and `MB` after the last one's `LB`.
#[test]
fn a_multi_layer_mask_begun_or_ended_between_layers() {
    let body = format!(
        "Mb\n{}{}h\nW\nn\nMd\n{}{END7}{}{}{END7}MB\n{}{}{END7}",
        begin7("A", 1),
        rect_path(40.0, 40.0, 120.0, 120.0),
        masked(15.0),
        begin7("B", 1),
        masked(30.0),
        begin7("C", 1),
        masked(150.0)
    );
    let trees: Vec<_> = layer_trees(&read7(&body).doc).into_iter().map(|(_, t, _)| t).collect();
    assert_eq!(trees, ["k[c p]", "k[c p]", "p"]);
}

/// Each copy of the mask counts toward the most objects read, with its points.
#[test]
fn copies_of_a_multi_layer_mask_count_toward_the_limit() {
    let points: String = (0..2_000_001).map(|i| format!("{} 40 L\n", 40 + i % 80)).collect();
    let mask = format!("Mb\n40 40 m\n{points}h\nW\nn\nMd\n");
    let one = format!("{}{mask}{}MB\n{END7}", begin7("A", 1), masked(15.0));
    assert_eq!(layer_trees(&read7(&one).doc)[0].1, "k[c p]");
    let two = format!("{}{mask}{}{END7}{}{}MB\n{END7}", begin7("A", 1), masked(15.0), begin7("B", 1), masked(30.0));
    let e = read(mask_stream7(&two).as_bytes()).unwrap_err();
    assert!(e.contains("objects"), "{e}");
    // A filled mask's fill, a path of its own, counts too.
    let filled = format!("{}{}{}MB\n{END7}", begin7("A", 1), mask.replace("W\nn\n", "W\nf\n"), masked(15.0));
    let e = read(mask_stream7(&filled).as_bytes()).unwrap_err();
    assert!(e.contains("objects"), "{e}");
}
