//! The PostScript reader: each operator family on a small program, files the EPS writer made,
//! the preview fallback and refused files.

use std::sync::Arc;

use vectorcraft_color::{Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, ColorMode, Document, LineCap, LineJoin, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use crate::import::{Dsc, import};
use crate::{EpsOptions, Level, Preview, Raster};

/// `body` as an EPS file with a 100 × 100 pt bounding box.
fn eps(body: &str) -> Vec<u8> {
    format!("%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n{body}\nshowpage\n%%EOF\n").into_bytes()
}

fn read(body: &str) -> crate::Imported {
    import(&eps(body)).unwrap()
}

/// The objects of the document's layer.
fn objects(d: &Document) -> Vec<Arc<Node>> {
    d.layers[0].children().unwrap().clone()
}

/// Every object, down through groups, in order.
fn all(d: &Document) -> Vec<Arc<Node>> {
    fn walk(n: &Arc<Node>, out: &mut Vec<Arc<Node>>) {
        out.push(n.clone());
        for c in n.children().into_iter().flatten() {
            walk(c, out);
        }
    }
    let mut out = vec![];
    for n in objects(d) {
        walk(&n, &mut out);
    }
    out
}

fn fill(n: &Node) -> Option<&Paint> {
    n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) => Some(&f.paint),
        _ => None,
    })
}

fn stroke(n: &Node) -> Option<&vectorcraft_doc::StrokeLayer> {
    n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Stroke(s) => Some(s),
        _ => None,
    })
}

fn bounds(n: &Node) -> Rect {
    n.geometric_bounds().unwrap()
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 0.05)
}

#[test]
fn paths_fill_and_stroke_in_document_space() {
    let r = read("newpath 10 20 moveto 60 20 lineto 60 70 lineto closepath 1 0 0 setrgbcolor fill");
    let n = &objects(&r.document)[0];
    // PostScript's y goes up from the bottom of the bounding box.
    assert!(near(bounds(n), Rect::new(10.0, 30.0, 60.0, 80.0)), "{:?}", bounds(n));
    assert_eq!(fill(n), Some(&Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    // Curves and arcs.
    let r = read("0 0 moveto 0 50 50 50 50 0 curveto stroke 50 50 20 0 360 arc fill");
    let o = objects(&r.document);
    assert_eq!(o.len(), 2);
    assert!(stroke(&o[0]).is_some() && fill(&o[0]).is_none());
    assert!(near(bounds(&o[1]), Rect::new(30.0, 30.0, 70.0, 70.0)), "{:?}", bounds(&o[1]));
    // Relative operators, rectangles.
    let r = read("10 10 moveto 20 0 rlineto 0 20 rlineto closepath fill 5 5 10 10 rectfill 0 0 4 4 rectstroke");
    let o = objects(&r.document);
    assert!(near(bounds(&o[0]), Rect::new(10.0, 70.0, 30.0, 90.0)));
    assert!(near(bounds(&o[1]), Rect::new(5.0, 85.0, 15.0, 95.0)));
    assert!(stroke(&o[2]).is_some());
}

#[test]
fn colours_in_every_model() {
    let r = read(
        "0 0 10 10 rectfill 0.25 setgray 0 0 10 10 rectfill 0 1 0 0 setcmykcolor 0 0 10 10 rectfill 0 1 1 sethsbcolor 0 0 10 10 rectfill \
         [/Separation (Gold) /DeviceCMYK {dup 0 exch 0.5 mul 0} bind] setcolorspace 0.5 setcolor 0 0 10 10 rectfill \
         /DeviceRGB setcolorspace 0 0 1 setcolor 0 0 10 10 rectfill \
         [/Indexed /DeviceRGB 1 <ff0000 00ff00>] setcolorspace 1 setcolor 0 0 10 10 rectfill",
    );
    let fills: Vec<Paint> = objects(&r.document).iter().map(|n| fill(n).unwrap().clone()).collect();
    // The initial colour is black; grey is ink coverage in the document.
    assert_eq!(fills[0], Paint::solid(Color::gray(1.0)));
    assert_eq!(fills[1], Paint::solid(Color::gray(0.75)));
    assert_eq!(fills[2], Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)));
    let [r0, g0, b0] = fills[3].color().unwrap().to_rgb();
    assert!(r0 > 0.99 && g0 < 0.01 && b0 < 0.01);
    // A spot ink links to its spot swatch at the tint.
    let Paint::Solid { swatch: Some(name), tint, .. } = &fills[4] else { panic!("{:?}", fills[4]) };
    assert_eq!((name.as_str(), *tint), ("Gold", 0.5));
    let gold = r.document.swatch("Gold").unwrap();
    assert!(gold.spot && gold.paint.color() == Some(Color::cmyk(1.0, 0.0, 0.5, 0.0)), "{gold:?}");
    assert_eq!(fills[5], Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    assert_eq!(fills[6], Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    // Mostly CMYK art opens in CMYK.
    let r = read("0 0 0 1 setcmykcolor 0 0 5 5 rectfill 1 0 0 0 setcmykcolor 0 0 5 5 rectfill");
    assert_eq!(r.document.color_mode, ColorMode::Cmyk);
    assert_eq!(read("1 0 0 setrgbcolor 0 0 5 5 rectfill").document.color_mode, ColorMode::Rgb);
}

#[test]
fn gsave_and_grestore_bring_the_state_back() {
    let r = read("gsave 1 0 0 setrgbcolor 5 setlinewidth 50 50 translate 0 0 10 10 rectfill grestore 0 0 10 10 rectfill");
    let o = objects(&r.document);
    assert_eq!(fill(&o[0]), Some(&Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    assert!(near(bounds(&o[0]), Rect::new(50.0, 40.0, 60.0, 50.0)));
    assert_eq!(fill(&o[1]), Some(&Paint::solid(Color::gray(1.0))));
    assert!(near(bounds(&o[1]), Rect::new(0.0, 90.0, 10.0, 100.0)));
    // save/restore too.
    let o = objects(&read("save 0.5 setgray restore 0 0 1 1 rectfill").document);
    assert_eq!(fill(&o[0]), Some(&Paint::solid(Color::gray(1.0))));
}

#[test]
fn transforms_concat() {
    let r = read("[2 0 0 2 10 10] concat 0 0 10 10 rectfill 90 rotate 0 0 5 5 rectfill");
    let o = objects(&r.document);
    assert!(near(bounds(&o[0]), Rect::new(10.0, 70.0, 30.0, 90.0)), "{:?}", bounds(&o[0]));
    // Rotated a quarter turn about (10, 10), scaled 2: x goes up, y to the left.
    assert!(near(bounds(&o[1]), Rect::new(0.0, 80.0, 10.0, 90.0)), "{:?}", bounds(&o[1]));
    let r = read("10 20 translate 2 3 scale 0 0 moveto 10 10 lineto stroke");
    assert!(near(bounds(&objects(&r.document)[0]), Rect::new(10.0, 50.0, 30.0, 80.0)));
    // Matrix operators: transform and itransform.
    let r = read("2 2 scale 3 4 transform itransform translate 0 0 1 1 rectfill");
    assert!(near(bounds(&objects(&r.document)[0]), Rect::new(6.0, 90.0, 8.0, 92.0)), "{:?}", bounds(&objects(&r.document)[0]));
}

#[test]
fn line_state_reaches_the_stroke() {
    let r = read("2 setlinewidth 1 setlinecap 2 setlinejoin 4 setmiterlimit [3 1] 0.5 setdash 2 2 scale 0 0 moveto 10 0 lineto stroke");
    let n = &objects(&r.document)[0];
    let st = stroke(n).unwrap();
    // The width and dashes grow with the transform.
    assert_eq!((st.width, st.cap, st.join, st.miter_limit), (4.0, LineCap::Round, LineJoin::Bevel, 4.0));
    let dash = st.dash.as_ref().unwrap();
    assert_eq!((dash.pattern.clone(), dash.offset), (vec![6.0, 2.0], 1.0));
    // A zero-width line is a hairline.
    assert!(stroke(&objects(&read("0 setlinewidth 0 0 moveto 9 9 lineto stroke").document)[0]).unwrap().width > 0.0);
}

#[test]
fn procedures_bind_def_loops_and_dictionaries() {
    let r = read(
        "/bd {bind def} bind def /sq {newpath moveto 10 0 rlineto 0 10 rlineto -10 0 rlineto closepath fill} bd \
         0 20 60 {0 sq} for 3 {} repeat /n 0 def {n 1 add /n exch def n 2 ge {exit} if} loop \
         5 dict begin /x 70 def x 70 sq end \
         << /a 1 /b 2 >> /b get 2 eq {0 0 sq} if \
         [1 2 3] {pop} forall (abc) length 3 eq {80 80 sq} if \
         /nothing where {pop} {80 0 sq} ifelse {nonsense} stopped {90 90 moveto 95 95 lineto stroke} if",
    );
    let o = objects(&r.document);
    // Four squares from the loop, one in the dictionary, one from get, one from length, one from
    // where, the stroke after stopped.
    assert_eq!(o.len(), 9, "{:?}", r.warnings);
    assert!(near(bounds(&o[3]), Rect::new(60.0, 90.0, 70.0, 100.0)));
    assert!(near(bounds(&o[4]), Rect::new(70.0, 20.0, 80.0, 30.0)));
}

#[test]
fn a_fill_then_a_stroke_of_the_same_path_is_one_object() {
    let r = read("10 10 moveto 50 50 lineto 90 10 lineto closepath gsave 1 0 0 setrgbcolor fill grestore 0 0 1 setrgbcolor stroke");
    let o = objects(&r.document);
    assert_eq!(o.len(), 1);
    assert_eq!(fill(&o[0]), Some(&Paint::solid(Color::rgb(1.0, 0.0, 0.0))));
    assert_eq!(stroke(&o[0]).unwrap().paint, Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
}

#[test]
fn clips_become_clipping_groups() {
    let r = read("gsave 0 0 50 50 rectclip 0 0 100 100 rectfill 10 10 5 5 rectfill grestore 60 60 5 5 rectfill");
    let o = objects(&r.document);
    assert_eq!(o.len(), 2);
    let NodeKind::Group { children, clip: true } = &o[0].kind else { panic!("{:?}", o[0].kind) };
    assert_eq!(children.len(), 3);
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(near(bounds(&children[0]), Rect::new(0.0, 50.0, 50.0, 100.0)));
    // A clip around the whole page clips nothing.
    let o = objects(&read("0 0 100 100 rectclip 1 1 5 5 rectfill").document);
    assert!(matches!(o[0].kind, NodeKind::Path { .. }));
    // eoclip of a path.
    let o = objects(&read("10 10 moveto 30 10 lineto 30 30 lineto closepath eoclip newpath 0 0 50 50 rectfill").document);
    assert!(matches!(o[0].kind, NodeKind::Group { clip: true, .. }));
}

#[test]
fn shadings_become_gradients() {
    let axial =
        "<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> >>";
    let r = read(&format!("gsave 10 10 moveto 90 10 lineto 50 90 lineto closepath clip newpath {axial} shfill grestore"));
    let o = objects(&r.document);
    // The clip path filled with the gradient.
    assert_eq!(o.len(), 1, "{o:?}");
    let Some(Paint::Gradient(g)) = fill(&o[0]) else { panic!("{:?}", o[0]) };
    assert_eq!(g.gradient.kind, GradientKind::Linear);
    assert_eq!(g.gradient.stops.len(), 2);
    assert_eq!(g.gradient.stops[1].color, Color::rgb(0.0, 0.0, 1.0));
    let geom = g.geom.unwrap();
    assert_eq!((geom.start, geom.end), (Point::new(0.0, 100.0), Point::new(100.0, 100.0)));
    // Stitched functions keep their bounds as stops; radial shadings, and shading patterns.
    let radial = "<< /ShadingType 3 /ColorSpace /DeviceGray /Coords [50 50 0 50 50 40] /Function << /FunctionType 3 /Domain [0 1] /Bounds [0.25] /Encode [0 1 0 1] /Functions [<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> << /FunctionType 2 /Domain [0 1] /C0 [1] /C1 [0] /N 1 >>] >> >>";
    let r = read(&format!("<< /PatternType 2 /Shading {radial} >> matrix makepattern setpattern 0 0 100 100 rectfill"));
    let o = objects(&r.document);
    let Some(Paint::Gradient(g)) = fill(&o[0]) else { panic!() };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    let offsets: Vec<f32> = g.gradient.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, vec![0.0, 0.25, 1.0]);
}

#[test]
fn images_and_image_masks() {
    // 2 × 2 RGB from hex data in the file, through a procedure.
    let r = read(
        "10 10 translate 20 20 scale 2 2 8 [2 0 0 -2 0 2] {currentfile 6 string readhexstring pop} false 3 colorimage\nff000000ff000000ffffffff\n",
    );
    let o = objects(&r.document);
    let NodeKind::Image(im) = &o[0].kind else { panic!("{:?} {:?}", o, r.warnings) };
    assert_eq!((im.width, im.height), (2, 2));
    assert!(near(bounds(&o[0]), Rect::new(10.0, 70.0, 30.0, 90.0)), "{:?}", bounds(&o[0]));
    let png = image::load_from_memory(&r.document.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(png.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(png.get_pixel(1, 1).0, [255, 255, 255, 255]);
    // The dictionary form through ASCII85 and run-length filters, grey with a decode array.
    let rl = crate::ps::ascii85(&[1, 0, 255, 128]);
    let r = read(&format!(
        "/src currentfile /ASCII85Decode filter /RunLengthDecode filter def /DeviceGray setcolorspace \
         << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 8 /Decode [1 0] /ImageMatrix [2 0 0 1 0 0] /DataSource src >> image\n{rl}"
    ));
    let NodeKind::Image(im) = &objects(&r.document)[0].kind else { panic!("{:?}", r.warnings) };
    let png = image::load_from_memory(&r.document.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!((png.get_pixel(0, 0).0, png.get_pixel(1, 0).0), ([255, 255, 255, 255], [0, 0, 0, 255]));
    // An image mask paints the current colour where its bits say.
    let r = read("1 0 0 setrgbcolor 8 1 true [8 0 0 1 0 0] <f0> imagemask");
    let NodeKind::Image(im) = &objects(&r.document)[0].kind else { panic!() };
    let png = image::load_from_memory(&r.document.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!((png.get_pixel(0, 0).0, png.get_pixel(7, 0).0), ([255, 0, 0, 255], [255, 0, 0, 0]));
}

#[test]
fn type_becomes_point_type_in_the_font_named() {
    let r = read("/Helvetica-Bold findfont 12 scalefont setfont 10 20 moveto (Hi) show (!) show");
    let o = objects(&r.document);
    assert_eq!(o.len(), 2);
    let NodeKind::Text(t) = &o[0].kind else { panic!() };
    assert_eq!(t.plain_text(), "Hi");
    let style = &t.runs[0].style;
    assert_eq!((style.font_family.as_str(), style.font_style.as_str()), ("Helvetica", "Bold"));
    assert!((style.size - 12.0).abs() < 1e-6);
    // On the baseline at (10, 80) in the document, upright.
    assert!((t.xf.translation().x - 10.0).abs() < 1e-6 && (t.xf.translation().y - 80.0).abs() < 1e-6, "{:?}", t.xf);
    assert!((t.xf.as_coeffs()[0] - 1.0).abs() < 1e-6 && (t.xf.as_coeffs()[3] - 1.0).abs() < 1e-6, "{:?}", t.xf);
    // The second show starts where the first ended.
    let NodeKind::Text(t2) = &o[1].kind else { panic!() };
    assert!(t2.xf.translation().x > 15.0);
    assert_eq!(crate::family_style("ABCDEF+TimesNewRomanPS-BoldItalicMT"), ("Times New Roman".into(), "Bold Italic".into()));
    assert_eq!(crate::family_style("Times-Roman"), ("Times".into(), "Regular".into()));
}

#[test]
fn the_writers_own_output_reads_back() {
    use vectorcraft_doc::{Appearance, Dash};
    use vectorcraft_geom::shapes;
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let look = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 4.0);
    let mut red = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 60.0)), look);
    if let Some(AppearanceItem::Stroke(s)) = red.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Stroke(_))) {
        s.dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 0.0, align_corners: false });
        s.cap = LineCap::Round;
    }
    d.insert(Some(layer), 0, red).unwrap();
    let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
    g.geom = Some(vectorcraft_color::GradientGeom { start: Point::new(110.0, 50.0), end: Point::new(190.0, 50.0), aspect: 1.0, focal: None });
    let grad = Node::path(
        d.alloc_id(),
        shapes::ellipse(Rect::new(110.0, 10.0, 190.0, 90.0)),
        Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0),
    );
    d.insert(Some(layer), 1, grad).unwrap();
    for level in [Level::Three, Level::Two] {
        let r = d.artboards[0].rect;
        let o = EpsOptions { region: r, origin: Point::new(r.x0, r.y1), preview: Preview::None, level, ..EpsOptions::default() };
        let bytes = crate::export(&d, &o, None, None).unwrap().bytes;
        let back = import(&bytes).unwrap();
        assert!(back.warnings.is_empty(), "{level:?}: {:?}", back.warnings);
        assert_eq!(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 100.0));
        let objs = all(&back.document);
        let rect = objs.iter().find(|n| fill(n).is_some_and(|p| p.color() == Some(Color::rgb(1.0, 0.0, 0.0)))).unwrap();
        assert!(near(bounds(rect), Rect::new(10.0, 10.0, 90.0, 60.0)), "{:?}", bounds(rect));
        let st = objs.iter().find_map(|n| stroke(n)).unwrap();
        assert_eq!((st.width, st.cap, st.dash.as_ref().unwrap().pattern.clone()), (4.0, LineCap::Round, vec![6.0, 3.0]));
        // Level 3 writes the gradient as a shading (a gradient again); level 2 as bands.
        let gradients = objs.iter().filter(|n| matches!(fill(n), Some(Paint::Gradient(_)))).count();
        assert_eq!(gradients, usize::from(level == Level::Three), "{level:?}");
    }
}

#[test]
fn an_unreadable_program_falls_back_to_the_preview() {
    let ps = eps("0 0 10 10 rectfill frobnicate");
    let tiff = crate::tiff::encode(&Raster { width: 4, height: 4, rgba: [0, 128, 0, 255].repeat(16) }, false, false).unwrap();
    let bytes = crate::dos_eps(&ps, &tiff).unwrap();
    let r = import(&bytes).unwrap();
    assert!(r.preview);
    assert!(r.warnings[0].contains("frobnicate") && r.warnings[0].contains("preview"), "{:?}", r.warnings);
    let o = objects(&r.document);
    let NodeKind::Image(im) = &o[0].kind else { panic!() };
    assert_eq!((im.width, im.height), (4, 4));
    assert!(near(bounds(&o[0]), Rect::new(0.0, 0.0, 100.0, 100.0)));
    // Without a preview, what was drawn before the error stays, with a warning.
    let r = import(&ps).unwrap();
    assert!(!r.preview && objects(&r.document).len() == 1);
    assert!(r.warnings[0].contains("frobnicate"), "{:?}", r.warnings);
}

#[test]
fn garbage_and_runaway_programs_are_refused() {
    assert!(import(b"").is_err());
    assert!(import(b"hello world").is_err());
    assert!(import(&[0xC5, 0xD0, 0xD3, 0xC6, 0xff, 0xff]).is_err());
    let e = import(b"%!PS\nfoo bar").unwrap_err();
    assert!(e.contains("foo"), "{e}");
    for body in ["{} loop", "/f {f} def f", "0 1 1e300 {pop} for", "[ 0 1 200000 {} for ]", "1000000000 array", "(abc", "{ 1 2"] {
        assert!(import(&eps(body)).is_err(), "{body}");
    }
    // Damaged numbers draw nothing far away.
    let r = read("1e300 1e300 moveto 1e301 0 lineto stroke 0 0 1 1 rectfill");
    assert_eq!(objects(&r.document).len(), 1);
}

#[test]
fn the_bounding_box_comes_from_the_header_or_the_trailer() {
    let d = Dsc::read(b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 1 2 30 40\n%%HiResBoundingBox: 1.5 2.5 29.5 39.5\n%%EndComments\n");
    assert_eq!(d.bbox, Some([1.5, 2.5, 29.5, 39.5]));
    let d = Dsc::read(b"%!PS-Adobe-3.0\n%%BoundingBox: (atend)\n%%Pages: 3\n%%EndComments\nshowpage\n%%Trailer\n%%BoundingBox: 0 0 10 20\n");
    assert_eq!((d.bbox, d.pages), (Some([0.0, 0.0, 10.0, 20.0]), Some(3)));
    let r = import(b"%!PS-Adobe-3.0\n%%Pages: 2\n0 0 5 5 rectfill showpage 0 0 9 9 rectfill showpage\n").unwrap();
    // No bounding box: a letter page; only the first page.
    assert_eq!(r.document.artboards[0].rect, Rect::new(0.0, 0.0, 612.0, 792.0));
    assert_eq!(objects(&r.document).len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("first of the file's 2 pages")), "{:?}", r.warnings);
}

#[test]
fn embedded_font_programs_are_skipped() {
    let body = "/MyFont 10 dict begin /FontName /MyFont def /FontMatrix [0.001 0 0 0.001 0 0] def currentdict end currentfile eexec\n\
                0123456789abcdef garbage that is encrypted\n0000000000000000000000000000000000000000000000000000000000000000\ncleartomark\n\
                /MyFont findfont 10 scalefont setfont 10 10 moveto (x) show";
    let r = read(body);
    let NodeKind::Text(t) = &objects(&r.document)[0].kind else { panic!("{:?}", r.warnings) };
    assert_eq!(t.runs[0].style.font_family, "My Font");
}

/// A document with what the writer writes in its own ways: gradients, type, an image with
/// transparent pixels, a clipping group, a spot colour, dashes.
fn rich_doc() -> Document {
    use vectorcraft_color::{Gradient, GradientGeom, GradientPaint, Swatch};
    use vectorcraft_doc::{Appearance, CharStyle, ImageBlob, ImageObject, TextObject};
    use vectorcraft_geom::{Affine, shapes};
    let mut d = Document::new(300.0, 200.0);
    let layer = d.layers[0].id;
    let add = |d: &mut Document, n: Node| d.insert(Some(layer), usize::MAX, n).unwrap();
    let solid = |r, g, b| Paint::solid(Color::rgb(r, g, b));
    let n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 60.0)),
        Appearance::basic(solid(1.0, 0.0, 0.0), solid(0.0, 0.0, 1.0), 4.0),
    );
    add(&mut d, n);
    for (kind, r) in [(GradientKind::Linear, Rect::new(110.0, 10.0, 190.0, 90.0)), (GradientKind::Radial, Rect::new(200.0, 10.0, 290.0, 90.0))] {
        let mut g = GradientPaint::new(Gradient { kind, ..Gradient::default() });
        g.geom = Some(GradientGeom::fit(kind, r, 0.0));
        let n = Node::path(d.alloc_id(), shapes::ellipse(r), Appearance::basic(Paint::Gradient(Box::new(g)), Paint::None, 0.0));
        add(&mut d, n);
    }
    let style = CharStyle { size: 30.0, fill: solid(0.0, 0.5, 0.0), stroke: Paint::None, ..CharStyle::default() };
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(10.0, 120.0), "Type", style))));
    add(&mut d, n);
    let mut img = image::RgbaImage::new(8, 4);
    for (x, _, p) in img.enumerate_pixels_mut() {
        *p = image::Rgba(if x < 4 { [0, 0, 255, 255] } else { [255, 200, 0, 255] });
    }
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("pic".into(), ImageBlob::new("image/png", png));
    let im = ImageObject {
        key: "pic".into(),
        width: 8,
        height: 4,
        xf: Affine::new([10.0, 0.0, 0.0, 10.0, 110.0, 110.0]),
        link: None,
        placement: Default::default(),
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(im));
    add(&mut d, n);
    d.swatches.push(Swatch { name: "Spot".into(), paint: Paint::solid(Color::cmyk(0.0, 0.5, 1.0, 0.0)), global: true, spot: true });
    let spot = Paint::Solid { color: Color::cmyk(0.0, 0.5, 1.0, 0.0), swatch: Some("Spot".into()), tint: 1.0 };
    let mut clip = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(200.0, 110.0, 290.0, 190.0)), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let mut inner = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(190.0, 100.0, 300.0, 150.0)), Appearance::basic(spot, Paint::None, 0.0));
    inner.appearance.items.push(AppearanceItem::Stroke({
        let mut st = vectorcraft_doc::StrokeLayer::new(solid(0.0, 0.0, 0.0), 2.0);
        st.dash = Some(vectorcraft_doc::Dash { pattern: vec![4.0, 2.0], offset: 0.0, align_corners: false });
        st
    }));
    let g = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(inner)], clip: true });
    add(&mut d, g);
    d
}

#[test]
fn the_writers_output_reads_back_looking_the_same() {
    let d = rich_doc();
    let r = d.artboards[0].rect;
    let mut renderer = vectorcraft_render::Renderer::new();
    let want = renderer.render_region(&d, r, 1.0, true);
    for level in [Level::Three, Level::Two] {
        let o = EpsOptions { region: r, origin: Point::new(r.x0, r.y1), preview: Preview::None, level, ..EpsOptions::default() };
        let back = import(&crate::export(&d, &o, None, None).unwrap().bytes).unwrap();
        assert!(back.warnings.is_empty(), "{level:?}: {:?}", back.warnings);
        assert!(back.document.swatch("Spot").is_some_and(|s| s.spot), "{level:?}");
        let got = renderer.render_region(&back.document, back.document.artboards[0].rect, 1.0, true);
        assert_eq!((got.width, got.height), (want.width, want.height));
        // Pixels that differ visibly: anti-aliased edges, gradient steps at level 2.
        let off = want.pixels.chunks(4).zip(got.pixels.chunks(4)).filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 48)).count();
        assert!(off * 100 < want.pixels.len() / 4, "{level:?}: {off} pixels differ");
    }
}

#[test]
fn device_settings_are_accepted_and_change_nothing() {
    let r = read(
        "currentscreen setscreen currenttransfer settransfer currentcolortransfer setcolortransfer currenthalftone sethalftone \
         currentblackgeneration setblackgeneration currentundercolorremoval setundercolorremoval 1 setflat true setstrokeadjust \
         << /PageSize [100 100] >> setpagedevice 1000 setcachelimit mark 1 2 setucacheparams ucache /StandardEncoding findencoding pop \
         0 0 5 5 rectfill",
    );
    assert_eq!(objects(&r.document).len(), 1, "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn font_cache_queries_answer_like_an_empty_cache() {
    // An old .ai prolog asks the font cache for its state before it sets anything up.
    let r = read("cachestatus 7 {pop} repeat ucachestatus 5 {pop} repeat currentcacheparams 4 {pop} repeat 0 0 5 5 rectfill");
    assert_eq!(objects(&r.document).len(), 1, "{:?}", r.warnings);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// Check that `body` draws one square, blue: it sets blue when what it checks holds, else red.
fn blue(body: &str) {
    let r = read(&format!("{body} 10 10 80 80 rectfill"));
    let o = objects(&r.document);
    assert_eq!(o.len(), 1, "{body}: {:?}", r.warnings);
    assert_eq!(fill(&o[0]), Some(&Paint::solid(Color::rgb(0.0, 0.0, 1.0))), "{body}");
    assert!(r.warnings.is_empty(), "{body}: {:?}", r.warnings);
}

/// Check that PostScript `cond` leaves `true`.
fn check(cond: &str) {
    blue(&format!("{cond} {{ 0 0 1 setrgbcolor }} {{ 1 0 0 setrgbcolor }} ifelse"));
}

#[test]
fn the_generic_category_defines_new_categories() {
    // #234: the reference app's prolog.
    blue("/Generic /Category findresource pop 0 0 1 setrgbcolor");
    blue(
        "[/CSA /Gradient /Procedure] { /Generic /Category findresource dup length dict copy /Category defineresource pop } forall 0 0 1 setrgbcolor",
    );
    let define = "[/CSA /Gradient] { /Generic /Category findresource dup length dict copy /Category defineresource pop } forall \
                  /G1 << /Kind 7 >> /Gradient defineresource pop";
    check(&format!("{define} /G1 /Gradient findresource /Kind get 7 eq"));
    check(&format!("{define} /Gradient /Category resourcestatus {{ pop pop true }} {{ false }} ifelse"));
    check(&format!("{define} /G1 /Gradient resourcestatus {{ pop pop true }} {{ false }} ifelse"));
    check(&format!("{define} /G1 /Gradient undefineresource /G1 /Gradient resourcestatus not"));
    check(&format!("{define} {{ /G2 /Gradient findresource }} stopped"));
    // Unknown categories aren't instances of Category; a category is a dictionary.
    check("/Nonsense /Category resourcestatus not");
    check("{ /Nonsense /Category findresource } stopped");
    check("{ /X 5 /Category defineresource } stopped");
}

#[test]
fn clipsave_and_cliprestore_bring_the_clip_back() {
    // #234.
    blue("clipsave 0 0 50 50 rectclip cliprestore 0 0 1 setrgbcolor");
    // The fill after cliprestore covers the whole square, unclipped.
    let r = read("clipsave 0 0 50 50 rectclip 1 0 0 setrgbcolor 0 0 10 10 rectfill cliprestore 0 0 1 setrgbcolor 10 10 80 80 rectfill");
    let o = objects(&r.document);
    assert_eq!(o.len(), 2, "{:?}", r.warnings);
    assert!(matches!(o[0].kind, NodeKind::Group { clip: true, .. }), "{:?}", o[0].kind);
    assert!(matches!(o[1].kind, NodeKind::Path { clipping: false, .. }), "{:?}", o[1].kind);
    assert!(near(bounds(&o[1]), Rect::new(10.0, 10.0, 90.0, 90.0)), "{:?}", bounds(&o[1]));
    // Nested, leaving the rest of the graphics state alone.
    let r = read(
        "0 0 60 60 rectclip clipsave 0 0 20 20 rectclip clipsave 0 0 5 5 rectclip cliprestore 0 1 0 setrgbcolor cliprestore \
         10 10 80 80 rectfill",
    );
    let NodeKind::Group { children, clip: true } = &objects(&r.document)[0].kind else { panic!("{:?}", r.warnings) };
    assert!(near(bounds(&children[0]), Rect::new(0.0, 40.0, 60.0, 100.0)), "{:?}", bounds(&children[0]));
    assert_eq!(fill(&children[1]), Some(&Paint::solid(Color::rgb(0.0, 1.0, 0.0))));
    // grestore undoes the clipsaves after its gsave; cliprestore with nothing saved does nothing.
    let o = objects(&read("cliprestore gsave clipsave 0 0 50 50 rectclip grestore cliprestore 10 10 80 80 rectfill").document);
    assert!(matches!(o[0].kind, NodeKind::Path { .. }), "{:?}", o[0].kind);
}

#[test]
fn languagelevel_is_an_integer_and_runs() {
    // #234.
    blue(
        "/Level2? systemdict /languagelevel known dup { pop systemdict /languagelevel get 2 ge } if def \
         Level2? { 0 0 1 setrgbcolor } { 1 0 0 setrgbcolor } ifelse",
    );
    check("languagelevel 3 eq");
    check("{ languagelevel } bind exec 3 eq");
}

#[test]
fn intervals_share_their_arrays_and_strings() {
    // #234: astore into a subarray stores into the array.
    blue("/a 4 array def 1 2 a 0 2 getinterval astore pop a 0 get 1 eq { 0 0 1 setrgbcolor } { 1 0 0 setrgbcolor } ifelse");
    // put, putinterval and copy into an interval; an interval of an interval.
    check("/a [0 0 0 0 0] def a 1 3 getinterval 1 2 getinterval 0 7 put a 2 get 7 eq");
    check("/a [0 0 0 0] def a 2 2 getinterval 0 [8 9] putinterval a 3 get 9 eq");
    check("/a [0 0 0 0] def [5 6] a 1 2 getinterval copy pop a 2 get 6 eq");
    check("/s (abcd) def s 1 2 getinterval 0 (XY) putinterval s (aXYd) eq");
    check("/s (abcd) def s 2 1 getinterval 0 90 put s (abZd) eq");
    check("/s 8 string def 42 s 2 6 getinterval cvs pop s 2 2 getinterval (42) eq");
    // The same interval is the same object; another one is not.
    check("/a 4 array def a 0 2 getinterval a 0 2 getinterval eq");
    check("/a 4 array def a 0 2 getinterval a 0 3 getinterval ne a 0 2 getinterval a 1 2 getinterval ne and");
    // Reads and writes stay inside the interval.
    check("/a [1 2 3 4] def { a 1 2 getinterval 2 get } stopped { a 1 2 getinterval 2 0 put } stopped and");
    check("/a [1 2 3 4] def a 1 2 getinterval length 2 eq a 1 2 getinterval aload pop 3 eq exch 2 eq and and");
    check("/n 0 def [1 2 3 4] 1 2 getinterval { n add /n exch def } forall n 5 eq");
    // A procedure from an interval runs only its part.
    check("{ 1 2 3 } 1 2 getinterval cvx exec add 5 eq");
    // The prolog's stack save: the operands into part of one array, and back.
    check(
        "/stk 10 array def /lev 3 array def 1 2 3 count dup lev exch 0 exch put stk exch 0 exch getinterval astore pop \
         clear stk 0 lev 0 get getinterval aload pop add add 6 eq",
    );
}

#[test]
fn resourceforall_enumerates_matching_names() {
    // #234.
    blue("(*) { pop } 128 string /Category resourceforall 0 0 1 setrgbcolor");
    let count = |template: &str| format!("/n 0 def ({template}) {{ pop /n n 1 add def }} 128 string /ProcSet resourceforall n");
    let define = "[/Foo /Fop /Bar /F*x] { 1 dict /ProcSet defineresource pop } forall";
    check(&format!("{define} {} 4 eq", count("*")));
    check(&format!("{define} {} 2 eq", count("Fo?")));
    check(&format!("{define} {} 3 eq", count("F*")));
    check(&format!("{define} {} 1 eq", count("F\\\\*x")));
    check(&format!("{define} {} 0 eq", count("Q*")));
    check(&format!("{define} {} 1 eq", count("*a*")));
    // Each name is copied into the scratch string; exit stops.
    check(&format!("{define} /last () def (B*) {{ dup length string copy /last exch def }} 8 string /ProcSet resourceforall last (Bar) eq"));
    check(&format!("{define} /n 0 def (*) {{ pop /n n 1 add def exit }} 8 string /ProcSet resourceforall n 1 eq"));
    // A scratch string too small for the names, as the prolog probes it.
    check(&format!("{define} {{ (*) {{ pop }} 2 string /ProcSet resourceforall }} stopped"));
    check("/n 0 def (Gen*) { pop /n n 1 add def } 16 string /Category resourceforall n 1 eq");
}

/// A `w` × `h` 8-bit palette TIFF with an alpha sample (`ExtraSamples` [1]), uncompressed in
/// strips of `per` rows, in either byte order, as the reference app writes its EPS previews:
/// pixel (x, y) is entry `(x + y) % 3` of a red, green, blue map, at alpha `12 x`.
fn palette_tiff(w: u16, h: u16, per: u16, big: bool) -> Vec<u8> {
    let u16b = |v: u16| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    let u32b = |v: u32| if big { v.to_be_bytes() } else { v.to_le_bytes() };
    // Shorts in an entry, left-justified.
    let shorts = |v: &[u16]| -> [u8; 4] {
        let mut f = [0; 4];
        for (i, s) in v.iter().enumerate() {
            f[2 * i..2 * i + 2].copy_from_slice(&u16b(*s));
        }
        f
    };
    let row = u32::from(w) * 2;
    let mut out = vec![];
    out.extend(if big { *b"MM\0*" } else { *b"II*\0" });
    out.extend([0; 4]);
    let data_at = out.len() as u32;
    for y in 0..h {
        for x in 0..w {
            out.extend([((x + y) % 3) as u8, (x * 12) as u8]);
        }
    }
    let map_at = out.len() as u32;
    for (c, full) in [0xFFFF, 0x8000, 0xFFFF].into_iter().enumerate() {
        for i in 0..256 {
            out.extend(u16b(if i == c { full } else { 0 }));
        }
    }
    let strips: Vec<u16> = (0..h.div_ceil(per)).collect();
    let offsets_at = out.len() as u32;
    for k in &strips {
        out.extend(u32b(data_at + u32::from(k * per) * row));
    }
    let counts_at = out.len() as u32;
    for k in &strips {
        out.extend(u32b(u32::from(per.min(h - k * per)) * row));
    }
    let ifd = out.len() as u32;
    out[4..8].copy_from_slice(&u32b(ifd));
    let n = strips.len() as u32;
    let entries: [(u16, u16, u32, [u8; 4]); 12] = [
        (256, 3, 1, shorts(&[w])),
        (257, 3, 1, shorts(&[h])),
        (258, 3, 2, shorts(&[8, 8])),
        (259, 3, 1, shorts(&[1])),
        (262, 3, 1, shorts(&[3])),
        (273, 4, n, u32b(if n == 1 { data_at } else { offsets_at })),
        (277, 3, 1, shorts(&[2])),
        (278, 3, 1, shorts(&[per])),
        (279, 4, n, u32b(if n == 1 { u32::from(h) * row } else { counts_at })),
        (284, 3, 1, shorts(&[1])),
        (320, 3, 768, u32b(map_at)),
        (338, 3, 1, shorts(&[1])),
    ];
    out.extend(u16b(entries.len() as u16));
    for (tag, kind, count, field) in entries {
        out.extend(u16b(tag));
        out.extend(u16b(kind));
        out.extend(u32b(count));
        out.extend(field);
    }
    out.extend([0; 4]);
    out
}

#[test]
fn a_palette_preview_with_alpha_is_placed() {
    // #234: the program fails, so the preview is placed.
    let ps = eps("0 0 1 setrgbcolor 10 10 80 80 rectfill frobnicate");
    for (big, per) in [(false, 5), (true, 5), (false, 10), (true, 3)] {
        let tiff = palette_tiff(20, 10, per, big);
        let r = import(&crate::dos_eps(&ps, &tiff).unwrap()).unwrap();
        assert!(r.preview, "{big} {per}");
        assert!(r.warnings[0].contains("preview"), "{:?}", r.warnings);
        let o = objects(&r.document);
        let NodeKind::Image(im) = &o[0].kind else { panic!("{:?}", o[0].kind) };
        assert_eq!((im.width, im.height), (20, 10));
        assert!(near(bounds(&o[0]), Rect::new(0.0, 0.0, 100.0, 100.0)));
        let png = image::load_from_memory(&r.document.images[&im.key].bytes).unwrap().to_rgba8();
        for (x, y) in [(0, 0), (1, 0), (2, 0), (19, 9), (7, 4), (3, 6)] {
            let [r, g, b] = [[255, 0, 0], [0, 128, 0], [0, 0, 255]][((x + y) % 3) as usize];
            assert_eq!(png.get_pixel(x, y).0, [r, g, b, (x * 12) as u8], "({x}, {y}) {big} {per}");
        }
    }
}

#[test]
fn a_damaged_palette_preview_is_refused_without_panicking() {
    let tiff = palette_tiff(20, 10, 5, false);
    assert!(crate::tiff::palette_rgba(&tiff).is_some());
    // Cut anywhere before the last entry's value (the directory is at the end): nothing.
    for cut in 0..tiff.len() - 6 {
        assert!(crate::tiff::palette_rgba(&tiff[..cut]).is_none(), "{cut}");
    }
    // Any byte changed: a picture or nothing.
    for big in [false, true] {
        let tiff = palette_tiff(20, 10, 5, big);
        for at in 0..tiff.len() {
            for v in [0, 1, 2, 0x7f, 0x80, 0xff] {
                let mut t = tiff.clone();
                t[at] = v;
                let _ = crate::tiff::palette_rgba(&t);
            }
        }
    }
    // A damaged preview of an unreadable program: the hard error.
    let ps = eps("frobnicate");
    for t in [&tiff[..tiff.len() / 2], b"II*\0garbage", &[0; 64]] {
        let e = import(&crate::dos_eps(&ps, t).unwrap()).unwrap_err();
        assert!(e.contains("can't be read"), "{e}");
    }
}
