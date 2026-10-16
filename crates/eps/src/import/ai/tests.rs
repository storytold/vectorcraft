//! The editing data reader on synthetic data, written from the format as `read` documents it.

use std::sync::Arc;

use kurbo::{Point, Rect};
use vectorcraft_color::gradient::GradientKind;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, Knockout, LayerColor, LineCap, LineJoin, Node, NodeKind};

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

/// A pattern's definition (`%AI3_BeginPattern`): its name and tile (8 points square here, y up),
/// then its art on `%_` lines (a group of the tile's unpainted box and a dot, a round-capped
/// stroke of no length), `E`.
const DOTS: &str = "%AI3_BeginPattern: (Dots)\n(Dots) 0 0 8 8\n%_0 A\n%_0 Xw\n%_0 Ae\n%_u\n%_0 O\n%_0 8 m\n%_8 8 L\n%_8 0 L\n%_0 0 L\n%_0 8 L\n%_n\n\
                    %_0 R\n%_0 0 0 1 K\n%_1 J 0 j 2 w 4 M []0 d\n%_4 4 m\n%_4 4 L\n%_S\n%_U\n%_/ArtDictionary :\n%_;\n%_\n%_9 () XW\nE\n%AI3_EndPattern\n";

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
    let unread = "%AI3_BeginPattern: (Old)\n(Old) 0 0 8 8 [\n(0 O 0 R 0 g) @\n(\n0 0 m 8 8 L S\n) &\n] E\n%AI3_EndPattern\n\
                  %AI3_BeginPattern: (Odd)\n(Odd) 0 0 8 8\n%_U\nE\n%AI3_EndPattern\n%AI3_BeginPattern: (Flat)\n(Flat) 0 0 8 0\n%_0 0 m\n%_8 0 L\n%_S\nE\n%AI3_EndPattern\n";
    let s = read_ok(&format!("{unread}{}", layer(&(rect_path(0.0, 0.0, 1.0, 1.0) + "f\n"))));
    assert!(s.doc.patterns.is_empty() && s.doc.swatches.iter().all(|w| !matches!(w.paint, Paint::Pattern { .. })));
    for name in ["Old", "Odd", "Flat"] {
        let e = read(
            stream(&format!("{unread}{}", layer(&format!("({name}) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n{}f\n", rect_path(0.0, 0.0, 1.0, 1.0)))))
                .as_bytes(),
        )
        .unwrap_err();
        assert!(e.contains("pattern fills"), "{name}: {e}");
    }
    // Type in a pattern isn't read either.
    let typed = "%AI3_BeginPattern: (Words)\n(Words) 0 0 8 8\n%_/AI11Text :\n%_0 /StoryIndex ,\n%_;\nE\n%AI3_EndPattern\n";
    assert!(read_ok(&format!("{typed}{}", layer(""))).doc.patterns.is_empty());
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
    // A clipping path with a fill paints it too, under the art it clips; without one, nothing.
    let filled = format!("0 Ae\nq\n{RED}{}f\n0 0 1 0 0 0 1 Xa\n{}W\nf\nQ\n", rect_path(0.0, 0.0, 30.0, 30.0), rect_path(5.0, 5.0, 25.0, 25.0));
    let doc = read_ok(&layer(&filled)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    let kinds: Vec<_> = children.iter().map(|c| (matches!(c.kind, NodeKind::Path { clipping: true, .. }), fill(c).color())).collect();
    assert_eq!(kinds, [(true, None), (false, Some(Color::rgb(0.0, 0.0, 1.0))), (false, Some(Color::rgb(1.0, 0.0, 0.0)))]);
    // A clipping path in a layer clips the layer.
    let doc = read_ok(&layer(&format!("{}f\n{}W\nn\n", rect_path(0.0, 0.0, 9.0, 9.0), rect_path(0.0, 0.0, 5.0, 5.0)))).doc;
    let NodeKind::Layer { clip, children, .. } = &doc.layers[0].kind else { panic!() };
    assert!(*clip && matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
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

    // Without a preview it can read (none, cut short, not 32 bits, absurd sizes), without a path or
    // a matrix, the file isn't read from its editing data.
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
        let e = read(&placed(true, matrix, path, preview, "")).unwrap_err();
        assert!(e.contains("placed files"), "{matrix} {path} {preview:?}: {e}");
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
}

#[test]
fn what_it_doesnt_read_is_an_error_where_it_shows() {
    let e = read(stream(&layer("1 2 frobnicate\n")).as_bytes()).unwrap_err();
    assert!(e.contains("`frobnicate`"), "{e}");
    let e = read(stream(&layer("(Jive) 0 0 1 1 0 0 0 0 0 [1 0 0 1 0 0] p\n")).as_bytes()).unwrap_err();
    assert!(e.contains("pattern fills"), "{e}");
    let e = read(stream(&layer("/SymbolInstance :\n(Dot) /SymbolRef ,\n;\n")).as_bytes()).unwrap_err();
    assert!(e.contains("symbols"), "{e}");
    // An opacity mask (#2004): drawing the layer's art whole would show what the mask hides, so
    // the mask is named and the file is read from its PDF part instead.
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
    let boards: Vec<_> = super::artboards(data.as_bytes()).into_iter().map(|(n, r)| (n, r.x0)).collect();
    assert_eq!(boards, [("one".to_string(), 0.0), ("two".to_string(), 200.0)]);
}

#[test]
fn a_compound_clipping_path_with_a_fill_paints_once_under_its_clip() {
    // GUARDS: the painted copies of a compound's clipping subpaths went into the compound itself,
    // doubling each subpath, so under even-odd the clip cancelled to nothing.
    let art_ = format!(
        "0 Ae\nq\n{RED}{}f\n1 XR\n*u\n0 0 0 0 0 0 1 Xa\n{}W\nf\n{}W\nf\n*U\nQ\n",
        rect_path(0.0, 0.0, 100.0, 100.0),
        rect_path(10.0, 10.0, 40.0, 40.0),
        rect_path(50.0, 10.0, 80.0, 40.0),
    );
    let doc = read_ok(&layer(&art_)).doc;
    let NodeKind::Group { children, clip: true } = &art(&doc)[0].kind else { panic!("a clipping group") };
    assert_eq!(children.len(), 3, "{children:#?}");
    let NodeKind::Compound { children: clip, rule } = &children[0].kind else { panic!("{:?}", children[0].kind) };
    assert_eq!(*rule, vectorcraft_geom::FillRule::EvenOdd);
    assert_eq!(clip.len(), 2);
    assert!(clip.iter().all(|c| matches!(c.kind, NodeKind::Path { clipping: true, .. })));
    let NodeKind::Compound { children: painted, .. } = &children[1].kind else { panic!("{:?}", children[1].kind) };
    assert_eq!((painted.len(), fill(&children[1]).color()), (2, Some(Color::rgb(0.0, 0.0, 1.0))));
    assert_eq!(fill(&children[2]).color(), Some(Color::rgb(1.0, 0.0, 0.0)));
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
    /BasicFilter :\n(Adobe Drop Shadow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n3 /Real (horz) ,\n4 /Real (vert) ,\n; /Dict ;\n /Part ,\n;\n\
    /KnownStyle :\n(Lit) /Name ,\n/ActiveStyle :\n/CompoundFilter :\n(Chain Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /CompoundFilter :\n(Stack Style Filter) 0 0 /Filter ,\n1 /Visible ,\n\
    /BasicFilter :\n(Fill Style Filter) 0 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n/FillStyle : 0 O\n1 (G) 0 0 0 1 1 0 0 1 0 0 Bg\n0 1 Xd\n/Def ;\n (FillStyle) ,\n; /Dict ;\n /Part ,\n\
    /BasicFilter :\n(Conduit Filter) 0 0 /Filter ,\n1 /Visible ,\n;\n /Part ,\n;\n /Part ,\n\
    /BasicFilter :\n(Adobe Outer Glow) 1 0 /Filter ,\n1 /Visible ,\n/Dictionary : /NotRecorded ,\n0.5 /Real (opac) ,\n2 /Real (blur) ,\n; /Dict ;\n /Part ,\n;\n\
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
