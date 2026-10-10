//! EPS as other apps write it: for each common generator, a short hand-written program with the
//! constructs its files typically use (its prolog's kind of procedures, how it sets colour, fills
//! with gradients and patterns, places images and sets type), each read as vectors without a
//! fallback to the preview. No generator's files or prologs are copied: these reproduce the
//! PostScript constructs only.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ColorMode, Document, Node, NodeKind};
use vectorcraft_geom::{FillRule, Point, Rect};

use super::tests::{all, bounds, fill, near, objects, read, stroke};
use crate::import::{Imported, import};
use crate::ps::{ascii85, deflate};

/// An EPS file of a 200 × 150 pt page made by `creator`.
fn file(creator: &str, program: &str) -> Vec<u8> {
    format!("%!PS-Adobe-3.0 EPSF-3.0\n%%Creator: {creator}\n%%BoundingBox: 0 0 200 150\n%%LanguageLevel: 3\n%%EndComments\n{program}\n%%EOF\n")
        .into_bytes()
}

/// `r` came in as vectors, without a warning of an error or of art left out.
fn clean(r: &Imported) {
    assert!(!r.preview, "{:?}", r.warnings);
    let bad = ["error", "couldn't", "left out", "mid-grey", "middle colour", "unknown"];
    assert!(r.warnings.iter().all(|w| !bad.iter().any(|b| w.contains(b))), "{:?}", r.warnings);
}

fn open(creator: &str, program: &str) -> Imported {
    let r = import(&file(creator, program)).unwrap();
    clean(&r);
    r
}

/// The objects of `kind` (`Node::kind_label`), down through groups.
fn of_kind(d: &Document, kind: &str) -> Vec<std::sync::Arc<Node>> {
    all(d).into_iter().filter(|n| n.kind_label() == kind).collect()
}

/// ASCII85 of Flate-compressed `data`, as inline data after an operator.
fn a85_flate(data: &[u8]) -> String {
    ascii85(&deflate(data))
}

/// Pixel `(x, y)` (straight RGBA) of the first image in `d`.
pub(super) fn pixel(d: &Document, x: u32, y: u32) -> [u8; 4] {
    let im = all(d)
        .into_iter()
        .find_map(|n| match &n.kind {
            NodeKind::Image(im) => Some(im.key.clone()),
            _ => None,
        })
        .unwrap();
    let img = image::load_from_memory(&d.images[&im].bytes).unwrap().to_rgba8();
    img.get_pixel(x, y).0
}

/// A Coons or tensor patch (type 7: sixteen points) in the packed form of a mesh shading with
/// 8-bit flags, 32-bit coordinates over `[0 200 0 150]` and 16-bit colour components.
fn patch(points: &[(f64, f64)], colors: &[[f64; 3]]) -> Vec<u8> {
    let mut v = vec![0u8];
    for (x, y) in points {
        v.extend(((x / 200.0 * f64::from(u32::MAX)).round() as u32).to_be_bytes());
        v.extend(((y / 150.0 * f64::from(u32::MAX)).round() as u32).to_be_bytes());
    }
    for c in colors {
        for k in c {
            v.extend(((k * 65535.0).round() as u16).to_be_bytes());
        }
    }
    v
}

/// The twelve boundary points of the square patch `(x0, y0)`–`(x1, y1)` in shading order (up its
/// left side, along its top, down its right side, back along its bottom).
fn square(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    let (dx, dy) = ((x1 - x0) / 3.0, (y1 - y0) / 3.0);
    vec![
        (x0, y0),
        (x0, y0 + dy),
        (x0, y0 + 2.0 * dy),
        (x0, y1),
        (x0 + dx, y1),
        (x0 + 2.0 * dx, y1),
        (x1, y1),
        (x1, y0 + 2.0 * dy),
        (x1, y0 + dy),
        (x1, y0),
        (x0 + 2.0 * dx, y0),
        (x0 + dx, y0),
    ]
}

/// cairo (Inkscape's EPS and PostScript export, and other GTK apps): a prolog of PDF-like
/// operator procedures, `rectclip` and `cm`, gradients as `shfill` with stitched functions whose
/// `Encode` a loop builds, tiling patterns, a Type 42 font with text set through `Tm`/`Tf`/`Tj`,
/// images with a 1-bit mask interleaved by row (`ImageType 3`), stencil masks, images flushed with
/// `status`/`flushfile`, and mesh gradients from a reusable stream.
#[test]
fn cairo_files_read_as_vectors() {
    // Mask rows (1: paint, `Decode [1 0]`) before each image row.
    let image = a85_flate(&[0b1000_0000, 255, 0, 0, 0, 255, 0, 0b0100_0000, 0, 0, 255, 255, 255, 255]);
    let mut points = square(150.0, 10.0, 190.0, 50.0);
    points.extend([(160.0, 20.0), (160.0, 40.0), (180.0, 40.0), (180.0, 20.0)]);
    let mesh = a85_flate(&patch(&points, &[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0], [1.0, 1.0, 0.0]]));
    let program = format!(
        r##"%%BeginProlog
50 dict begin
/q {{ gsave }} bind def /Q {{ grestore }} bind def
/cm {{ 6 array astore concat }} bind def
/w {{ setlinewidth }} bind def /J {{ setlinecap }} bind def /j {{ setlinejoin }} bind def
/M {{ setmiterlimit }} bind def /d {{ setdash }} bind def
/m {{ moveto }} bind def /l {{ lineto }} bind def /c {{ curveto }} bind def /h {{ closepath }} bind def
/re {{ 4 2 roll moveto 1 index 0 rlineto 0 exch rlineto neg 0 rlineto closepath }} bind def
/S {{ stroke }} bind def /f {{ fill }} bind def /f* {{ eofill }} bind def /n {{ newpath }} bind def
/W {{ clip }} bind def /W* {{ eoclip }} bind def /BT {{ }} bind def /ET {{ }} bind def
/BDC {{ mark 3 1 roll /BDC pdfmark }} bind def /EMC {{ mark /EMC pdfmark }} bind def
/cairo_store_point {{ /cairo_point_y exch def /cairo_point_x exch def }} def
/Tj {{ show currentpoint cairo_store_point }} bind def
/cairo_selectfont {{ cairo_font_matrix aload pop pop pop 0 0 6 array astore
  cairo_font exch selectfont cairo_point_x cairo_point_y moveto }} bind def
/Tf {{ pop /cairo_font exch def /cairo_font_matrix where {{ pop cairo_selectfont }} if }} bind def
/Tm {{ 2 copy 8 2 roll 6 array astore /cairo_font_matrix exch def cairo_store_point
  /cairo_font where {{ pop cairo_selectfont }} if }} bind def
/g {{ setgray }} bind def /rg {{ setrgbcolor }} bind def /d1 {{ setcachedevice }} bind def
/cairo_flush_ascii85_file {{ cairo_ascii85_file status {{ cairo_ascii85_file flushfile }} if }} def
/cairo_image {{ image cairo_flush_ascii85_file }} def
/cairo_imagemask {{ imagemask cairo_flush_ascii85_file }} def
%%EndProlog
%%BeginSetup
11 dict begin
/FontType 42 def /FontName /DejaVuSans def /PaintType 0 def
/FontMatrix [ 1 0 0 1 0 0 ] def /FontBBox [ 0 0 0 0 ] def
/Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for
Encoding 72 /H put Encoding 105 /i put
/CharStrings 3 dict dup begin /.notdef 0 def /H 1 def /i 2 def end readonly def
/sfnts [ <00010000000100000000000000> ] def
/f-0-0 currentdict end definefont pop
%%EndSetup
%%Page: 1 1
q 0 0 200 150 rectclip
1 0 0 -1 0 150 cm q
0.8 0.2 0.2 rg 10 10 40 30 re f
0 g 1.5 w 1 J 0 j [ 4 2] 0 d 10 60 m 40 45 60 90 90 60 c S [] 0.0 d
/BDC where {{ pop /Artifact << >> BDC EMC }} if
q 100 10 60 30 re W n
/CairoFunction << /FunctionType 3 /Domain [ 0 1 ] /Functions [
  << /FunctionType 2 /Domain [ 0 1 ] /C0 [ 1 0 0 ] /C1 [ 0 1 0 ] /N 1 >>
  << /FunctionType 2 /Domain [ 0 1 ] /C0 [ 0 1 0 ] /C1 [ 0 0 1 ] /N 1 >> ]
  /Bounds [ 0.5 ] /Encode [ 1 1 2 {{ pop 0 1 }} for ] >> def
<< /ShadingType 2 /ColorSpace /DeviceRGB /Coords [ 100 0 160 0 ] /Extend [ true true ] /Function CairoFunction >> shfill
Q
/CairoPattern {{ q 0 0 10 10 rectclip q 0.8 0 0.8 rg 0 0 5 5 re f Q Q }} bind def
<< /PatternType 1 /PaintType 1 /TilingType 1 /XStep 10 /YStep 10 /BBox [0 0 10 10]
   /PaintProc {{ pop CairoPattern }} >> [ 1 0 0 1 0 0 ] makepattern setpattern
10 90 40 40 re f
0 0.2 0.4 rg BT 12 0 0 -12 60 140 Tm /f-0-0 1 Tf (Hi)Tj ET
q [ 20 0 0 -20 110 110 ] concat
/cairo_ascii85_file currentfile /ASCII85Decode filter def
/DeviceRGB setcolorspace
<< /ImageType 3 /InterleaveType 2
  /DataDict << /ImageType 1 /Width 2 /Height 2 /Interpolate false /BitsPerComponent 8
    /Decode [ 0 1 0 1 0 1 ] /DataSource cairo_ascii85_file /FlateDecode filter /ImageMatrix [ 2 0 0 -2 0 2 ] >>
  /MaskDict << /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 1 /Decode [ 1 0 ] /ImageMatrix [ 2 0 0 -2 0 2 ] >>
>> cairo_image
{image}
Q
q 0 0 1 rg [ 20 0 0 -10 150 140 ] concat
/cairo_ascii85_file currentfile /ASCII85Decode filter def
8 1 true [ 8 0 0 -1 0 1 ] cairo_ascii85_file cairo_imagemask
{mask}
Q
q currentfile /ASCII85Decode filter /FlateDecode filter /ReusableStreamDecode filter
{mesh}
/CairoData exch def
<< /ShadingType 7 /ColorSpace /DeviceRGB /DataSource CairoData /BitsPerCoordinate 32
   /BitsPerComponent 16 /BitsPerFlag 8 /Decode [ 0 200 0 150 0 1 0 1 0 1 ] >> shfill
currentdict /CairoData undef
Q
Q Q
showpage
%%Trailer
end"##,
        mask = ascii85(&[0xF0]),
    );
    let r = open("cairo 1.18.4 (https://cairographics.org)", &program);
    let d = &r.document;
    // The tiling pattern is a pattern swatch of its cell.
    assert_eq!(d.patterns.len(), 1);
    let pat = &d.patterns[0];
    assert!(near(pat.tile, Rect::new(0.0, 0.0, 10.0, 10.0)), "{:?}", pat.tile);
    assert!(d.swatch(&pat.name).is_some_and(|s| matches!(s.paint, Paint::Pattern { .. })));
    assert!(all(d).iter().any(|n| matches!(fill(n), Some(Paint::Pattern { .. }))));
    // The stitched gradient, the dashed curve, the type, the masked image, the stencil and the mesh.
    assert!(all(d).iter().any(|n| matches!(fill(n), Some(Paint::Gradient(_)))));
    assert!(all(d).iter().any(|n| stroke(n).is_some_and(|s| s.dash.is_some())));
    assert_eq!(of_kind(d, "Type").len(), 1);
    assert_eq!(of_kind(d, "Image").len(), 2);
    let im = all(d).into_iter().find(|n| matches!(&n.kind, NodeKind::Image(im) if im.width == 2)).unwrap();
    let NodeKind::Image(im) = &im.kind else { panic!() };
    let img = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
    // Painted where the mask is 1, transparent where it is 0.
    assert_eq!([img.get_pixel(0, 0).0, img.get_pixel(1, 0).0], [[255, 0, 0, 255], [0, 255, 0, 0]]);
    assert_eq!([img.get_pixel(0, 1).0, img.get_pixel(1, 1).0], [[0, 0, 255, 0], [255, 255, 255, 255]]);
    let mesh = of_kind(d, "Mesh");
    assert_eq!(mesh.len(), 1);
    // `cm` turned user space y down, as the document is.
    assert!(near(bounds(&mesh[0]), Rect::new(150.0, 10.0, 190.0, 50.0)), "{:?}", bounds(&mesh[0]));
}

/// cairo's Type 3 fonts (fonts it can't embed otherwise): glyph procedures in an array, picked
/// through `CharStrings` by `BuildGlyph`, widths from `d1`: the glyphs are drawn as their
/// outlines, one group per string.
#[test]
fn type3_fonts_draw_their_glyph_procedures() {
    let program = r##"/d1 { setcachedevice } bind def
8 dict begin
/FontType 3 def
/FontMatrix [ 0.001 0 0 0.001 0 0 ] def
/FontBBox [ 0 0 1000 1000 ] def
/Encoding 256 array def 0 1 255 { Encoding exch /.notdef put } for
Encoding 65 /g1 put
/Glyphs [ { } { 600 0 0 0 500 700 d1 0 0 moveto 500 0 lineto 250 700 lineto closepath fill } ] def
/CharStrings 2 dict dup begin /.notdef 0 def /g1 1 def end readonly def
/BuildGlyph { exch dup /Glyphs get exch /CharStrings get 3 -1 roll 2 copy known not { pop /.notdef } if get get exec } bind def
/BuildChar { 1 index /Encoding get exch get 1 index /BuildGlyph get exec } bind def
currentdict end /f-1-0 exch definefont pop
1 0 0 setrgbcolor
/f-1-0 findfont 20 scalefont setfont 10 10 moveto (AA) show
currentpoint 2 copy translate
% `stringwidth` measures them without drawing.
(A) stringwidth pop 12 eq { 0 0 1 setrgbcolor } if 0 0 moveto 3 0 65 2 0 (AA) awidthshow
showpage"##;
    let r = open("cairo", program);
    let d = &r.document;
    let groups = of_kind(d, "Group");
    assert_eq!(groups.len(), 2, "{:?}", all(d).iter().map(|n| n.kind_label()).collect::<Vec<_>>());
    assert_eq!(groups[0].name.as_deref(), Some("AA"));
    let glyphs: Vec<Rect> = groups[0].children().unwrap().iter().map(|n| bounds(n)).collect();
    // 500 × 700 units at 20 pt (0.02 pt a unit) on the baseline 10 pt up the page; the second glyph
    // after the first one's width (600 units).
    assert!(near(glyphs[0], Rect::new(10.0, 126.0, 20.0, 140.0)), "{glyphs:?}");
    assert!(near(glyphs[1], Rect::new(22.0, 126.0, 32.0, 140.0)), "{glyphs:?}");
    assert_eq!(fill(&groups[0].children().unwrap()[0]).and_then(Paint::color), Some(Color::rgb(1.0, 0.0, 0.0)));
    // `awidthshow` spaces them (2 more, 3 more after an "A"), in blue: the width measured right.
    let second: Vec<Rect> = groups[1].children().unwrap().iter().map(|n| bounds(n)).collect();
    assert!((second[1].x0 - second[0].x0 - 17.0).abs() < 1e-6, "{second:?}");
    assert_eq!(fill(&groups[1].children().unwrap()[0]).and_then(Paint::color), Some(Color::rgb(0.0, 0.0, 1.0)));
}

/// matplotlib's PostScript backend: a `mpldict` of short procedures defined with `_d`, Type 3
/// fonts converted from TrueType (`CharStrings` of glyph procedures with `sc`, `BuildGlyph` and
/// `BuildChar`) shown glyph by glyph with `glyphshow`, `clipbox`, marker procedures, and images as
/// `colorimage` reading hexadecimal data with `readhexstring`.
#[test]
fn matplotlib_files_read_as_vectors() {
    let program = r##"%%BeginProlog
/mpldict 11 dict def
mpldict begin
/_d { bind def } bind def
/m { moveto } _d
/l { lineto } _d
/r { rlineto } _d
/c { curveto } _d
/cl { closepath } _d
/ce { closepath eofill } _d
/box { m 1 index 0 r 0 exch r neg 0 r cl } _d
/clipbox { box clip newpath } _d
/sc { setcachedevice } _d
%!PS-Adobe-3.0 Resource-Font
10 dict begin
/FontName /DejaVuSans def
/PaintType 0 def
/FontMatrix [ 0.00048828125 0 0 0.00048828125 0 0 ] def
/FontBBox [ -2090 -948 3673 2524 ] def
/FontType 3 def
/Encoding [ /A /V ] def
/CharStrings 3 dict dup begin
/.notdef 0 def
/A { 1401 0 16 0 1384 1493 sc 16 0 m 700 1493 l 1384 0 l ce } _d
/V { 1401 0 16 0 1384 1493 sc 16 1493 m 700 0 l 1384 1493 l ce } _d
end readonly def
/BuildGlyph { exch begin CharStrings exch 2 copy known not { pop /.notdef } if get exec end } _d
/BuildChar { 1 index /Encoding get exch get 1 index /BuildGlyph get exec } _d
FontName currentdict end definefont pop
end
%%EndProlog
mpldict begin
0 0 translate
0 0 200 150 rectclip
gsave
0 0 m 200 0 l 200 150 l 0 150 l cl
1 setgray fill
grestore
gsave
10 10 180 130 clipbox
0.122 0.467 0.706 setrgbcolor 1.5 setlinewidth 1 setlinejoin 2 setlinecap [] 0 setdash
newpath 10 10 m 50 60 l 90 30 l stroke
/o { gsave newpath translate 3 0 m 0 0 3 0 360 arc cl gsave 1 0 0 setrgbcolor fill grestore stroke grestore } bind def
10 10 o 50 60 o
grestore
0 setgray
gsave 20 100 translate 0 rotate
/DejaVuSans 20.0 selectfont 0 0 m /A glyphshow 13.68 0 m /V glyphshow
grestore
gsave 120 20 translate 40 40 scale
/DataString 6 string def
2 2 8 [ 2 0 0 -2 0 2 ] { currentfile DataString readhexstring pop } bind false 3 colorimage
ff000000ff00
0000ffffffff
grestore
end
showpage"##;
    let r = open("Matplotlib v3.9.0, https://matplotlib.org/", program);
    let d = &r.document;
    // The two glyphs are their outlines (even-odd, as `ce` fills them), at 20 pt.
    let glyphs: Vec<_> = all(d).into_iter().filter(|n| matches!(n.kind, NodeKind::Path { rule: FillRule::EvenOdd, .. })).collect();
    assert_eq!(glyphs.len(), 2);
    let a = bounds(&glyphs[0]);
    assert!(near(a, Rect::new(20.16, 150.0 - 100.0 - 14.58, 33.52, 50.0)), "{a:?}");
    // Each marker is one object, filled and stroked; the image is its four pixels.
    let markers = all(d).into_iter().filter(|n| fill(n).is_some() && stroke(n).is_some()).count();
    assert_eq!(markers, 2);
    assert_eq!([pixel(d, 0, 0), pixel(d, 1, 0), pixel(d, 0, 1)], [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]]);
}

/// CorelDRAW: a `wCorel…Dict` procset of `@` procedures defined with `bd`, `ld` and `xd`,
/// `currentscreen` kept and `setscreen` replaced, CMYK colour kept in `$c $m $y $k`, a document
/// in `save`/`restore` blocks, `showpage` redefined, fonts reencoded by copying their dictionary
/// without its `FID`, and type set with `ashow` through `makefont`.
#[test]
fn coreldraw_files_read_as_vectors() {
    let program = r##"%%BeginProlog
%%BeginResource: procset wCorel3Dict 3.0 0
/wCorel3Dict 300 dict def wCorel3Dict begin
/bd{bind def}bind def/ld{load def}bd/xd{exch def}bd/_ null def/rp{{pop}repeat}bd
/@cp/closepath ld/@gs/gsave ld/@gr/grestore ld/@np/newpath ld/Tl/translate ld
/$sv 0 def/@sv{/$sv save def}bd/@rs{$sv restore}bd/spg/showpage ld/showpage{}bd
currentscreen/@dsp xd/$dsp/@dsp def/$dsa xd/$dsf xd/$sdf false def/$SDF false def
/$Scra 0 def/SetScr/setscreen ld/@ss{2 index 0 eq{$dsf 3 1 roll 4 -1 roll pop}if exch $Scra add exch SetScr}bd
/$c 0 def/$m 0 def/$y 0 def/$k 0 def/$t 1 def/$n _ def/$o 0 def/$fil 0 def/$fm 0 def
/$ctm matrix currentmatrix def/$ptm matrix def
/L2? false/languagelevel where{pop languagelevel 2 ge{pop true}if}if def
/@BeginSysCorelDict{systemdict/Corel30Dict known{systemdict/Corel30Dict get exec}if}bd
/@EndSysCorelDict{systemdict/Corel30Dict known{systemdict/Corel30Dict get exec}if}bd
/@sm{/$ctm $ctm currentmatrix def}bd/@rm{$ctm setmatrix}bd
/k{/$k xd/$y xd/$m xd/$c xd/$n _ def}bd
/@k{$c $m $y $k L2?{setcmykcolor}{4 rp 0 setgray}ifelse}bd
/m/moveto ld/L/lineto ld/C/curveto ld/@c/closepath ld
/F{@gs @k $fil 0 eq{fill}{eofill}ifelse @gr @np}bd
/S{@k stroke}bd/@w{setlinewidth}bd
/CorelDrawReencodeVect[16#80/Euro 16#e9/eacute]def
/@reencode{findfont dup length dict begin{1 index/FID ne{def}{pop pop}ifelse}forall
/Encoding Encoding 256 array copy def CorelDrawReencodeVect aload length 2 idiv{Encoding 3 1 roll put}repeat
currentdict end definefont pop}bd
/@F{/$fm xd findfont $fm makefont setfont}bd
end
%%EndResource
%%EndProlog
%%BeginSetup
wCorel3Dict begin
@BeginSysCorelDict
2.6131 setmiterlimit 1.00 setflat
/$fst 128 def
0 45 {dup mul exch dup mul add 1 exch sub} @ss
/_Helvetica/Helvetica @reencode
%%EndSetup
%%Page: 1 1
@sv
@sm
@sv
0 1 1 0 k
10 10 m 90 10 L 90 60 L 10 60 L 10 10 L @c
F
@rs
@sv
1 @w 1 0 0 0 k
10 70 m 50 90 70 90 90 70 C
S
@rs
@sv
0 0 0 1 k @k
/_Helvetica [12 0 0 12 0 0] @F
10 120 m 1 0 (Corel) ashow
@rs
@rs
@EndSysCorelDict
end
showpage
spg"##;
    let r = open("CorelDRAW 2024", program);
    let d = &r.document;
    assert_eq!(d.color_mode, ColorMode::Cmyk);
    let rect = all(d).into_iter().find(|n| fill(n).and_then(Paint::color) == Some(Color::cmyk(0.0, 1.0, 1.0, 0.0))).unwrap();
    assert!(near(bounds(&rect), Rect::new(10.0, 90.0, 90.0, 140.0)), "{:?}", bounds(&rect));
    assert!(all(d).iter().any(|n| stroke(n).is_some_and(|s| s.paint.color() == Some(Color::cmyk(1.0, 0.0, 0.0, 0.0)))));
    let NodeKind::Text(t) = &of_kind(d, "Type")[0].kind else { panic!() };
    assert!(t.plain_text().starts_with("Corel"));
}

/// Affinity Designer (and other PDF-engine exporters): PDF-like procedures, gradients as shading
/// patterns (`makepattern setpattern`) with sampled functions, spot colours (`Separation`) and
/// `DeviceN` inks with tint transforms, and images compressed with Flate and a PNG predictor.
#[test]
fn affinity_files_read_as_vectors() {
    // Two rows of two RGB pixels, PNG-filtered: Sub, then Up.
    let rows = [[1u8, 255, 0, 0, 1, 255, 0], [2, 0, 0, 255, 0, 0, 255]];
    let image = a85_flate(&rows.concat());
    let program = format!(
        r##"%%BeginProlog
/AFDict 40 dict def AFDict begin
/languagelevel where {{ pop languagelevel }} {{ 1 }} ifelse 3 lt {{ (This file needs PostScript Level 3) print quit }} if
/bd {{ bind def }} bind def
/q {{ gsave }} bd /Q {{ grestore }} bd /cm {{ [ 7 1 roll ] concat }} bd
/re {{ 4 2 roll moveto 1 index 0 rlineto 0 exch rlineto neg 0 rlineto closepath }} bd
/f {{ fill }} bd /f* {{ eofill }} bd /W {{ clip }} bd /n {{ newpath }} bd
/rg {{ setrgbcolor }} bd /k {{ setcmykcolor }} bd
end
%%EndProlog
%%BeginSetup
/setpagedevice where {{ pop << /PageSize [ 200 150 ] >> setpagedevice }} if
%%EndSetup
%%Page: 1 1
AFDict begin
q
<< /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [ 0 0 100 0 ]
  /Function << /FunctionType 0 /Domain [ 0 1 ] /Range [ 0 1 0 1 0 1 ] /Size [ 3 ] /BitsPerSample 8
    /DataSource <ff000000ff000000ff> >> /Extend [ true true ] >> >> matrix makepattern setpattern
10 10 100 40 re f
[ /Separation (PANTONE 185 C) /DeviceCMYK {{ dup 0.91 mul exch 0.76 mul 0 exch 0 4 1 roll }} ] setcolorspace
0.5 setcolor 10 60 40 40 re f
[ /DeviceN [ /Cyan /Magenta ] /DeviceCMYK {{ 0 0 }} ] setcolorspace
0.3 0.7 setcolor 60 60 40 40 re f
q 40 0 0 40 120 60 cm /DeviceRGB setcolorspace
<< /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 8 /Decode [ 0 1 0 1 0 1 ] /ImageMatrix [ 2 0 0 -2 0 2 ]
   /DataSource currentfile /ASCII85Decode filter << /Predictor 15 /Colors 3 /Columns 2 /BitsPerComponent 8 >> /FlateDecode filter >> image
{image}
Q
Q
end
showpage"##
    );
    let r = open("Affinity Designer 2", &program);
    let d = &r.document;
    let Some(Paint::Gradient(g)) = all(d).into_iter().find_map(|n| fill(&n).cloned()) else { panic!("no gradient") };
    // The sampled function's colours, not its middle one: red, green, blue.
    let at = |t: f32| g.gradient.stops.iter().find(|s| (s.offset - t).abs() < 1e-3).map(|s| s.color.to_rgb_uncalibrated());
    assert_eq!([at(0.0), at(0.5), at(1.0)], [Some([1.0, 0.0, 0.0]), Some([0.0, 1.0, 0.0]), Some([0.0, 0.0, 1.0])]);
    assert!(d.swatch("PANTONE 185 C").is_some_and(|s| s.spot));
    assert!(all(d).iter().any(|n| matches!(fill(n), Some(Paint::Solid { swatch: Some(s), .. }) if s == "PANTONE 185 C")));
    assert!(all(d).iter().any(|n| fill(n).and_then(Paint::color) == Some(Color::cmyk(0.3, 0.7, 0.0, 0.0))));
    assert_eq!(
        [pixel(d, 0, 0), pixel(d, 1, 0), pixel(d, 0, 1), pixel(d, 1, 1)],
        [[255, 0, 0, 255], [0, 255, 0, 255], [255, 0, 255, 255], [0, 255, 255, 255]]
    );
}

/// LibreOffice and OpenOffice: a prolog that keeps the operand and dictionary stack depths and a
/// `save` to restore at the end, coordinates negated in its procedures, fonts reencoded to
/// ISO Latin-1 and defined again under one name, and type stretched to a width with
/// `stringwidth`.
#[test]
fn libreoffice_files_read_as_vectors() {
    let program = r##"%%BeginProlog
%%BeginResource: procset SDRes-Prolog 1.0 0
/b4_inc_state save def
/dict_count countdictstack def
/op_count count 1 sub def
userdict begin
0 setgray 0 setlinecap 1 setlinewidth 0 setlinejoin 10 setmiterlimit [] 0 setdash newpath
/languagelevel where {pop languagelevel 1 ne {false setstrokeadjust false setoverprint} if} if
/bdef {bind def} bind def
/c {setrgbcolor} bdef
/l {neg lineto} bdef
/rl {neg rlineto} bdef
/lw {setlinewidth} bdef
/m {neg moveto} bdef
/ct {6 2 roll neg 6 2 roll neg 6 2 roll neg curveto} bdef
/t {neg translate} bdef
/s {scale} bdef
/gs {gsave} bdef
/gr {grestore} bdef
/f {findfont dup length dict begin {1 index /FID ne {def} {pop pop} ifelse} forall /Encoding ISOLatin1Encoding def
currentdict end /NFont exch definefont pop /NFont findfont} bdef
/p {closepath} bdef
/sf {scalefont setfont} bdef
/ef {eofill} bdef
/ps {stroke} bdef
/pum {matrix currentmatrix} bdef
/pom {setmatrix} bdef
/bs {/aString exch def /nXOfs exch def /nWidth exch def currentpoint nXOfs 0 rmoveto pum nWidth aString stringwidth pop div 1 scale aString show pom moveto} bdef
%%EndResource
%%EndProlog
%%BeginSetup
%%EndSetup
%%Page: 1 1
%%BeginPageSetup
%%EndPageSetup
pum
0.1 0.1 s
0 -1500 t
/tm matrix currentmatrix def
gs
tm setmatrix
0.2 0.4 0.8 c 100 100 m 900 100 l 900 600 l 100 600 l 100 100 l p ef
gr
gs
tm setmatrix
10 lw 0 0 0 c 100 800 m 400 700 600 900 900 800 ct ps
gr
gs
tm setmatrix
/Helvetica f 120 sf 0 0 0 c 100 1300 m 800 0 (Office) bs
gr
pom
count op_count sub {pop} repeat countdictstack dict_count sub {end} repeat b4_inc_state restore
%%PageTrailer
%%Trailer
showpage"##;
    let r = open("LibreOffice 24.2", program);
    let d = &r.document;
    let rect = all(d).into_iter().find(|n| fill(n).and_then(Paint::color) == Some(Color::rgb(0.2, 0.4, 0.8))).unwrap();
    assert!(near(bounds(&rect), Rect::new(10.0, 10.0, 90.0, 60.0)), "{:?}", bounds(&rect));
    assert!(all(d).iter().any(|n| stroke(n).is_some_and(|s| (s.width - 1.0).abs() < 1e-9)));
    let NodeKind::Text(t) = &of_kind(d, "Type")[0].kind else { panic!() };
    assert_eq!(t.plain_text(), "Office");
}

/// Ghostscript's `ps2write` and `eps2write`: a procset that captures dictionaries with `//name`
/// (resolved when read, used after their dictionary is gone), probes the interpreter (`gcheck`,
/// `currentglobal`, `/currentdistillerparams where`, an unguarded `pdfmark`), defines resources,
/// and runs page content kept between `stream` and `endstream` through `SubFileDecode`, with
/// `selectfont`, `xshow`, `rectfill`, `execform`, half-tone dictionaries and a triangle mesh.
#[test]
fn ghostscript_files_read_as_vectors() {
    let program = r##"%%BeginProlog
save
countdictstack
mark
newpath
/showpage {} def
/setpagedevice {pop} def
%%EndProlog
%%Page: 1 1
%%BeginProlog
10 dict begin
/this currentdict def
/ebuf 200 string def
/prnt { //ebuf cvs pop } bind def
/knownget { 2 copy known { get true } { pop pop false } ifelse } bind def
end
20 dict begin
/PDFR_GLOBAL false def
/cp2g { dup gcheck not { dup type /dicttype eq { dup length dict copy } if } if } bind def
/DefaultSwitch { dup where { pop pop } { false def } ifelse } bind def
/PDFR_DEBUG DefaultSwitch
/currentdistillerparams where { pop currentdistillerparams /CoreDistVersion get 5000 lt } { true } ifelse
{ /setdistillerparams {pop} def } if
[ /Title (gs) /DOCINFO pdfmark
currentglobal true setglobal
/PDFReader 10 dict def
setglobal
/OPDF << /Run { cvx exec } >> /ProcSet defineresource pop
/q { gsave } bind def /Q { grestore } bind def
/cm { [ 7 1 roll ] concat } bind def
/rg { setrgbcolor } bind def
/re { rectfill } bind def
/Tf { selectfont } bind def
/Td { moveto } bind def
/stream { currentfile 0 (endstream) /SubFileDecode filter /OPDF /ProcSet findresource /Run get exec } bind def
%%EndProlog
<< /HalftoneType 1 /Frequency 60 /Angle 45 /SpotFunction { dup mul exch dup mul add 1 exch sub } >> sethalftone
/Form1 << /FormType 1 /BBox [ 0 0 20 20 ] /Matrix [ 1 0 0 1 0 0 ] /PaintProc { pop 0 0 1 rg 0 0 20 20 re } >> def
stream
q 1 0 0 rg 10 10 80 40 re
Q /Helvetica 50 Tf 20 100 Td (gs) [ 25 0 ] xshow
endstream
q 120 100 translate Form1 execform Q
<< /ShadingType 4 /ColorSpace /DeviceRGB /DataSource [ 0 120 10 1 0 0  0 180 10 0 1 0  0 150 60 0 0 1  1 190 60 1 1 0 ] >> shfill
end
cleartomark countdictstack exch sub { end } repeat restore
showpage"##;
    let r = open("GPL Ghostscript 10.04.0 (eps2write)", program);
    let d = &r.document;
    let red = all(d).into_iter().find(|n| fill(n).and_then(Paint::color) == Some(Color::rgb(1.0, 0.0, 0.0))).unwrap();
    assert!(near(bounds(&red), Rect::new(10.0, 100.0, 90.0, 140.0)), "{:?}", bounds(&red));
    let form = all(d).into_iter().find(|n| fill(n).and_then(Paint::color) == Some(Color::rgb(0.0, 0.0, 1.0))).unwrap();
    assert!(near(bounds(&form), Rect::new(120.0, 30.0, 140.0, 50.0)), "{:?}", bounds(&form));
    assert_eq!(of_kind(d, "Type").len(), 1);
    // Two triangles of a free-form mesh, the second on the first one's edge.
    let meshes = of_kind(d, "Mesh");
    assert_eq!(meshes.len(), 2);
    assert!(near(bounds(&meshes[1]), Rect::new(150.0, 90.0, 190.0, 140.0)), "{:?}", bounds(&meshes[1]));
}

/// A procset written as the PostScript Language Reference (3rd ed.) describes the Level 2 and 3
/// facilities programs probe before drawing: a resource category made from `Generic` and
/// resources defined and found in it (§3.9), local and global VM (`currentglobal`, `setglobal`,
/// `gcheck`, §3.7.2), `internaldict` (§8.2), a halftone dictionary (§7.4), the font cache
/// (`cachestatus`), `rootfont` and the colour rendering dictionary (§7.1), each guarded or
/// probed through `stopped` (§3.10), saving the state they change and drawing with procedures of
/// their own.
#[test]
fn level_2_and_3_probes_read_as_vectors() {
    let program = r##"%%BeginProlog
/ExampleKit 40 dict def ExampleKit begin
/defp { bind def } bind def
% A category of our own, implemented by a copy of Generic's dictionary (§3.9.2).
/Generic /Category findresource dup length dict copy /ExampleShapes exch /Category defineresource pop
/Card << /W 80 /H 50 >> /ExampleShapes defineresource pop
% VM: a dictionary made in global VM, then the VM mode put back.
/wasglobal currentglobal def true setglobal /Shared 4 dict def wasglobal setglobal
/vm Shared gcheck { (global) } { (local) } ifelse def
% Probes that fail on some interpreters, inside stopped.
/hasinternal { 1183615869 internaldict } stopped { false } { pop true } ifelse def
/screen << /HalftoneType 1 /Frequency 85 /Angle 15 /SpotFunction { 180 mul cos exch 180 mul cos add 2 div } >> def
/oldscreen currenthalftone def
/rendering currentcolorrendering def
/cache [ cachestatus ] def
/frame { /h exch def /w exch def moveto w 0 rlineto 0 h rlineto w neg 0 rlineto closepath } defp
end
%%EndProlog
%%BeginSetup
ExampleKit begin
screen sethalftone
/Helvetica 12 selectfont
/face rootfont /FontName get def
%%EndSetup
%%Page: 1 1
gsave
0 0.94 0.94 0.12 setcmykcolor
10 90 /Card /ExampleShapes findresource dup /W get exch /H get frame fill
0.2 0.4 0.8 setrgbcolor 2 setlinewidth 100 20 moveto 150 80 180 0 190 60 curveto stroke
grestore
% Each probe answered as the reference says: draw only when it did.
vm length 5 eq hasinternal and rendering type /dicttype eq and cache length 7 eq and face /Helvetica eq and
{ 0 setgray 10 30 moveto (Art) show } if
oldscreen sethalftone
end
showpage"##;
    let r = open("Example Generator 1.0", program);
    let d = &r.document;
    let rect = all(d).into_iter().find(|n| fill(n).and_then(Paint::color) == Some(Color::cmyk(0.0, 0.94, 0.94, 0.12))).unwrap();
    assert!(near(bounds(&rect), Rect::new(10.0, 10.0, 90.0, 60.0)), "{:?}", bounds(&rect));
    assert!(all(d).iter().any(|n| stroke(n).is_some_and(|s| s.width == 2.0)));
    assert_eq!(of_kind(d, "Type").len(), 1);
}

/// Shadings other apps paint with: function-based ones (type 1) over a sampled two-input
/// function, free-form triangles packed in bits (type 4), lattices (type 5), Coons patches sharing
/// an edge (type 6), and a mesh as a shading pattern's fill; all as gradient meshes.
#[test]
fn mesh_and_function_shadings_become_gradient_meshes() {
    // Type 1: a 2 × 2 sampled function, red to green across, to blue down.
    let r = read(
        "<< /ShadingType 1 /ColorSpace /DeviceRGB /Domain [0 1 0 1] /Matrix [50 0 0 50 10 10] \
         /Function << /FunctionType 0 /Domain [0 1 0 1] /Range [0 1 0 1 0 1] /Size [2 2] /BitsPerSample 8 \
         /DataSource <ff000000ff000000ff000000> >> >> shfill",
    );
    clean(&r);
    let m = &of_kind(&r.document, "Mesh")[0];
    assert!(near(bounds(m), Rect::new(10.0, 40.0, 60.0, 90.0)), "{:?}", bounds(m));
    let NodeKind::Mesh(g) = &m.kind else { panic!() };
    // Its first corner (the domain's origin) is the first sample.
    assert_eq!(g.points[0].color, Color::rgb(1.0, 0.0, 0.0));

    // Type 4 packed: 8-bit flags, coordinates and components; a triangle and one on its edge.
    let tri = [[0u8, 0, 0, 255, 0, 0], [0, 255, 0, 0, 255, 0], [0, 0, 255, 0, 0, 255], [1, 255, 255, 255, 255, 0]].concat();
    let r = read(&format!(
        "<< /ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 \
         /Decode [0 255 0 255 0 1 0 1 0 1] /DataSource <{}> >> [0.2 0 0 0.2 0 0] concat shfill",
        tri.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ));
    clean(&r);
    assert_eq!(of_kind(&r.document, "Mesh").len(), 2);

    // Type 5: a 2 × 2 lattice is two triangles.
    let r = read(
        "<< /ShadingType 5 /ColorSpace /DeviceGray /VerticesPerRow 2 \
         /DataSource [10 10 0  50 10 1  10 50 0.5  50 50 1] >> shfill",
    );
    clean(&r);
    assert_eq!(of_kind(&r.document, "Mesh").len(), 2);

    // Type 6: a patch, then one sharing its right side (flag 2).
    let first: Vec<String> = square(10.0, 10.0, 40.0, 40.0).iter().map(|(x, y)| format!("{x} {y}")).collect();
    let right = "40 40 50 40 60 40 70 40 70 30 70 20 70 10 60 10 50 10";
    let r = read(&format!(
        "<< /ShadingType 6 /ColorSpace /DeviceRGB /DataSource [0 {} 1 0 0 0 1 0 0 0 1 1 1 0  2 {right} 1 0 1 0 1 1] >> shfill",
        first.join(" ")
    ));
    clean(&r);
    let patches = of_kind(&r.document, "Mesh");
    assert_eq!(patches.len(), 2);
    assert!(near(bounds(&patches[1]), Rect::new(40.0, 60.0, 70.0, 90.0)), "{:?}", bounds(&patches[1]));

    // A mesh through a shading pattern fills the path it paints, clipped to it.
    let r = read(
        "<< /PatternType 2 /Shading << /ShadingType 5 /ColorSpace /DeviceGray /VerticesPerRow 2 \
         /DataSource [0 0 0  100 0 1  0 100 0.5  100 100 1] >> >> matrix makepattern setpattern \
         newpath 20 20 30 0 360 arc fill",
    );
    clean(&r);
    let o = objects(&r.document);
    assert!(matches!(o[0].kind, NodeKind::Group { clip: true, .. }), "{:?}", o[0].kind_label());
    assert_eq!(of_kind(&r.document, "Mesh").len(), 2);
}

/// Images with a mask (`ImageType 3`) interleaved by sample, and with the mask in its own source.
#[test]
fn masked_images_in_every_interleaving() {
    // By sample: mask bit, then three colour bits, 1 bit each (two pixels a row, padded).
    let r = read(
        "/DeviceRGB setcolorspace 20 20 scale << /ImageType 3 /InterleaveType 1 \
         /DataDict << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 1 /Decode [0 1 0 1 0 1] /ImageMatrix [2 0 0 1 0 0] \
         /DataSource <c600> >> /MaskDict << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 1 /Decode [1 0] /ImageMatrix [2 0 0 1 0 0] >> >> image",
    );
    clean(&r);
    // 1100 0110: painted red, then masked out.
    assert_eq!((pixel(&r.document, 0, 0), pixel(&r.document, 1, 0)[3]), ([255, 0, 0, 255], 0));
    // Separate sources, the mask at twice the resolution.
    let r = read(
        "/DeviceGray setcolorspace 20 20 scale << /ImageType 3 /InterleaveType 3 \
         /DataDict << /ImageType 1 /Width 1 /Height 1 /BitsPerComponent 8 /Decode [0 1] /ImageMatrix [1 0 0 1 0 0] /DataSource <80> >> \
         /MaskDict << /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 1 /Decode [0 1] /ImageMatrix [2 0 0 2 0 0] /DataSource <4000> >> >> image",
    );
    clean(&r);
    assert_eq!(pixel(&r.document, 0, 0), [128, 128, 128, 255]);
}

/// The warning for a file read only up to an error names the error, the operator that raised it
/// and the procedures it ran in, innermost first; an error a program catches with `stopped` is
/// not reported.
#[test]
fn errors_name_the_operator_and_the_procedures() {
    let r = read("{ 1 (a) add } stopped pop 0 0 10 10 rectfill /inner { 1 (a) add } def /outer { inner } def outer");
    assert!(r.warnings[0].contains("(typecheck in `add`), in `inner` in `outer`"), "{:?}", r.warnings);
    let r = read("0 0 10 10 rectfill /p { frobnicate } def p");
    assert!(r.warnings[0].contains("it uses `frobnicate`, which VectorCraft's PostScript reader doesn't know, in `p`"), "{:?}", r.warnings);
    let r = read("0 0 10 10 rectfill 0 0 moveto (x) cvn moveto");
    assert!(r.warnings[0].contains("typecheck in `moveto`: a number"), "{:?}", r.warnings);
}

/// Operators programs use to probe the interpreter or measure things: file `status`, `token`,
/// `bytesavailable`, `pathforall`, `strokepath`, `nulldevice`, `currenthsbcolor`, the cache and
/// device parameter queries.
#[test]
fn probing_operators_answer() {
    super::tests::check("currentfile status (no such file) status not and");
    super::tests::check("(12 /x {a}) token { 12 eq exch (/x {a}) eq and } { false } ifelse");
    super::tests::check("() token not");
    super::tests::check(
        "/n 0 def 0 0 moveto 10 0 lineto { pop pop /n n 1 add def } { pop pop /n n 2 add def } { 6 { pop } repeat } {} pathforall n 3 eq",
    );
    super::tests::check("1 0 0 setrgbcolor currenthsbcolor 1 eq exch 1 eq and exch 0 eq and");
    super::tests::check("currentcacheparams counttomark 2 eq exch pop exch pop exch pop");
    super::tests::check("cachestatus 7 { pop } repeat true");
    super::tests::check("currentcolorrendering /ColorRenderingType known");
    super::tests::check("[ (x) /y ] gcheck not");
    // `strokepath` makes the stroke's outline the path; `nulldevice` paints nothing.
    let r = read("4 setlinewidth 10 50 moveto 90 50 lineto strokepath fill gsave nulldevice 0 0 100 100 rectfill grestore");
    clean(&r);
    let o = objects(&r.document);
    assert_eq!(o.len(), 1);
    assert!(near(bounds(&o[0]), Rect::new(10.0, 48.0, 90.0, 52.0)), "{:?}", bounds(&o[0]));
}

/// A file whose PostScript can't be read opens its Windows metafile preview (vectors) when its
/// binary header has one, else its EPSI bitmap preview.
#[test]
fn metafile_and_epsi_previews_stand_in() {
    let ps = super::tests::eps("0 0 10 10 rectfill frobnicate");
    // A placeable metafile of one red rectangle over its 1000-unit frame.
    let wmf = wmf_rectangle();
    let mut bytes = vec![0xC5, 0xD0, 0xD3, 0xC6];
    let words = [30u32, ps.len() as u32, 30 + ps.len() as u32, wmf.len() as u32, 0, 0];
    for w in words {
        bytes.extend(w.to_le_bytes());
    }
    bytes.extend(0xFFFFu16.to_le_bytes());
    bytes.extend(&ps);
    bytes.extend(&wmf);
    let r = import(&bytes).unwrap();
    assert!(r.preview);
    assert!(r.warnings[0].contains("frobnicate") && r.warnings[0].contains("metafile"), "{:?}", r.warnings);
    let paths: Vec<_> = all(&r.document).into_iter().filter(|n| n.kind_label() == "Path").collect();
    assert!(!paths.is_empty());
    assert!(near(bounds(&paths[0]), Rect::new(10.0, 10.0, 90.0, 90.0)), "{:?}", bounds(&paths[0]));

    // An EPSI preview: 4 × 2 pixels, 1 bit, black on the left.
    let epsi = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n%%BeginPreview: 4 2 1 2\n% C0\n% C0\n%%EndPreview\nfrobnicate\n";
    let r = import(epsi.as_bytes()).unwrap();
    assert!(r.preview, "{:?}", r.warnings);
    assert_eq!([pixel(&r.document, 0, 0), pixel(&r.document, 3, 1)], [[0, 0, 0, 255], [255, 255, 255, 255]]);
}

/// A placeable Windows metafile (1000 units an inch) of a red rectangle from (100, 100) to
/// (900, 900) on a frame of 1000 × 1000 units.
fn wmf_rectangle() -> Vec<u8> {
    let mut v = vec![];
    let w16 = |v: &mut Vec<u8>, x: u16| v.extend(x.to_le_bytes());
    let w32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_le_bytes());
    w32(&mut v, 0x9AC6_CDD7);
    for x in [0u16, 0, 0, 1000, 1000, 1000] {
        w16(&mut v, x);
    }
    w32(&mut v, 0);
    let sum = v.chunks(2).fold(0u16, |a, c| a ^ u16::from_le_bytes([c[0], c[1]]));
    w16(&mut v, sum);
    let records: Vec<Vec<u16>> = vec![
        vec![0x020B, 0, 0],               // SetWindowOrg
        vec![0x020C, 1000, 1000],         // SetWindowExt
        vec![0x02FC, 0, 0x00FF, 0, 0],    // CreateBrushIndirect: solid red
        vec![0x012D, 0],                  // SelectObject
        vec![0x041B, 900, 900, 100, 100], // Rectangle (bottom, right, top, left)
        vec![0x0000],                     // EOF
    ];
    let size: usize = 9 + records.iter().map(|r| r.len() + 2).sum::<usize>();
    for x in [1u16, 9, 0x0300] {
        w16(&mut v, x);
    }
    w32(&mut v, size as u32);
    w16(&mut v, 1);
    w32(&mut v, 7);
    w16(&mut v, 0);
    for r in records {
        w32(&mut v, r.len() as u32 + 2);
        for x in r {
            w16(&mut v, x);
        }
    }
    v
}

/// Immediately evaluated names (`//name`) are their values when read: inside a procedure that
/// runs after the dictionary that defined them is gone, and as an operand at the top.
#[test]
fn immediately_evaluated_names_are_read_as_their_values() {
    super::tests::check("2 dict begin /v 7 def /p { //v } def currentdict end /d exch def d /p get exec 7 eq");
    super::tests::check("/w 3 def //w 3 eq");
    let r = read("/p { 0 0 10 10 } def //p rectfill");
    clean(&r);
    let r = read("0 0 10 10 rectfill /q { //nosuchname } def");
    assert!(r.warnings[0].contains("nosuchname"), "{:?}", r.warnings);
    let _ = Point::ZERO;
}

/// User paths (`ufill`, `ustroke`, `uappend`, `upath`, as procedures and in the encoded form),
/// and reading a file byte by byte (`read`) with its position.
#[test]
fn user_paths_and_file_reads() {
    let r = read(
        "{ 0 0 50 50 setbbox 10 10 moveto 40 10 lineto 40 40 lineto closepath } ufill \
         [ [0 0 50 50 60 60 90 60 90 90] <00010303> ] ustroke \
         newpath 0 0 moveto 5 5 lineto",
    );
    clean(&r);
    let o = objects(&r.document);
    assert_eq!(o.len(), 2);
    assert!(near(bounds(&o[0]), Rect::new(10.0, 60.0, 40.0, 90.0)), "{:?}", bounds(&o[0]));
    assert!(stroke(&o[1]).is_some() && near(bounds(&o[1]), Rect::new(60.0, 10.0, 90.0, 40.0)), "{:?}", bounds(&o[1]));
    super::tests::check(
        "(ab) /ASCIIHexDecode filter pop (4142) /ASCIIHexDecode filter dup read pop 65 eq exch dup 0 setfileposition read pop 65 eq and",
    );
    super::tests::check("{ (x) deletefile } stopped revision 1 eq and");
    // The bounding box, then each point with its operator.
    super::tests::check("newpath 0 0 moveto 5 5 lineto false upath length 11 eq");
}

/// Programs that would run away through what this reader adds end at its limits: a pattern
/// whose cell paints with itself, a Type 3 glyph that shows itself, a sampled function or mesh
/// too large, a dash pattern too fine for `strokepath`.
#[test]
fn hostile_patterns_glyphs_and_shadings_end() {
    let r = read(
        "/P << /PatternType 1 /PaintType 1 /XStep 5 /YStep 5 /BBox [0 0 5 5] \
         /PaintProc { pop P setpattern 0 0 5 5 rectfill } >> matrix makepattern def \
         P setpattern 0 0 50 50 rectfill",
    );
    assert!(!r.preview && r.warnings.iter().any(|w| w.contains("nested too deeply")), "{:?}", r.warnings);
    let r = import(&super::tests::eps(
        "0 0 1 1 rectfill /T << /FontType 3 /FontMatrix [1 0 0 1 0 0] /Encoding [/a] \
         /BuildChar { pop pop 0 0 moveto (a) show } >> definefont setfont 0 0 moveto (a) show",
    ))
    .unwrap();
    assert!(r.warnings[0].contains("error"), "{:?}", r.warnings);
    let r = read(
        "0 0 1 1 rectfill << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 1 0] \
         /Function << /FunctionType 0 /Domain [0 1] /Range [0 1] /Size [99999999] /BitsPerSample 8 /DataSource () >> >> shfill",
    );
    assert!(r.warnings[0].contains("rangecheck in `shfill`: a sampled function"), "{:?}", r.warnings);
    // Procedures nesting through operators that run them end at the nesting limit.
    for body in ["/f { { f } exec } def f", "/f { true { f } if } def f", "/f { 1 { f } repeat } def f", "/f { { f } stopped } def f"] {
        assert!(import(&super::tests::eps(body)).is_err(), "{body}");
    }
    let r = read("[1e-9] 0 setdash 0 0 moveto 1000 1000 lineto strokepath fill");
    clean(&r);
    assert_eq!(objects(&r.document).len(), 1);
}

/// The stops of the gradient `body` draws, after asserting it came in cleanly as one gradient.
fn stitch_stops(body: &str) -> Vec<f32> {
    let r = read(body);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let o = objects(&r.document);
    let [node] = o.as_slice() else { panic!("{o:?}") };
    let Some(Paint::Gradient(g)) = fill(node) else { panic!("{node:?}") };
    g.gradient.stops.iter().map(|s| s.offset).collect()
}

/// A gradient stitched from linear pieces keeps their bounds as its stops up to the cap, and is
/// sampled past it. The cap is on the pieces; a well-formed stitch has one more piece than bounds.
#[test]
fn gradients_stitched_from_linear_pieces_keep_or_sample_their_stops() {
    // `bounds` interior bounds at i / (bounds + 1), with one linear piece more than that.
    let stitched = |bounds: usize| {
        let pieces = bounds + 1;
        format!(
            "/F << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> def \
             << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 3 /Domain [0 1] \
             /Functions [ {pieces} {{ F }} repeat ] /Bounds [ 1 1 {bounds} {{ {pieces} div }} for ] >> >> shfill"
        )
    };
    // A few pieces: the bounds are kept exactly as stops (three bounds -> five stops with the ends).
    assert_eq!(stitch_stops(&stitched(3)), vec![0.0, 0.25, 0.5, 0.75, 1.0]);
    // At the cap (1024 bounds, 1025 pieces) the 1024 bounds and the two ends are all kept.
    assert_eq!(stitch_stops(&stitched(1024)).len(), 1026);
    // One past it (1025 bounds, 1026 pieces) the function is sampled instead.
    assert_eq!(stitch_stops(&stitched(1025)).len(), 33);
}

/// A malformed stitching function with a single piece but thousands of bounds is sampled, not
/// turned into thousands of stops. This is the one case the stop-count `.filter` catches on its
/// own: the one piece is under the Functions-length guard, so only the bound count is out of range.
#[test]
fn a_stitching_function_with_one_piece_but_many_bounds_is_sampled() {
    let stops = stitch_stops(
        "/F << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> def \
         << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 3 /Domain [0 1] \
         /Functions [ F ] /Bounds [ 1 1 2000 { 2001 div } for ] >> >> shfill",
    );
    // Base keeps all 2000 bounds (2002 stops); the fix samples.
    assert_eq!(stops.len(), 33);
}

/// A short `Bounds` with a `Functions` array one past the cap is sampled: without the length
/// guard each of the ~1000 stops would read through the whole array to find its piece.
#[test]
fn a_gradient_with_a_functions_array_past_the_cap_is_sampled() {
    let stops = stitch_stops(
        "/F << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> def \
         << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function << /FunctionType 3 /Domain [0 1] \
         /Functions [ 1026 { F } repeat ] /Bounds [ 1 1 1000 { 1001 div } for ] >> >> shfill",
    );
    // Base keeps the 1000 bounds (1002 stops); the fix samples.
    assert_eq!(stops.len(), 33);
}

/// A stitching function (type 3) whose `Functions` array holds the function itself must not
/// recurse without end: `linear_points` (depth- and visit-bounded) and `eval` (depth-bounded)
/// both stop, so the shading raises an error instead of overflowing the stack (an abort
/// `vectorcraft_engine::guard` can't catch, and on wasm there is no net at all).
#[test]
fn a_stitching_function_that_refers_to_itself_is_rejected_not_crashed() {
    let r = read(
        "0 0 10 10 rectfill \
         /F 5 dict def F /FunctionType 3 put F /Domain [0 1] put F /Bounds [] put F /Encode [0 1] put F /Functions [ F ] put \
         << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function F >> shfill",
    );
    // Not an abort: the reader stops at the error, keeps the rectangle drawn before it, and leaves
    // the rest of the program unread (see `import`: what follows the error is left out).
    assert_eq!(objects(&r.document).len(), 1);
    assert!(r.warnings.iter().any(|w| w.contains("read up to an error") && w.contains("Function")), "{:?}", r.warnings);
}

/// A stitching function need not point at itself to blow up: a `Functions` array of 1025
/// references to a child that holds 1025 references to a child ... is a tree of 1025^levels nodes
/// though it is tiny in memory (the children are shared). The shared visit counter stops
/// `linear_points` after about 2 * (MAX_STITCH_STOPS + 1) nodes, so it is sampled at once instead
/// of being walked; on the unmodified reader this explores ~1025^3 nodes.
#[test]
fn a_deeply_shared_stitching_function_is_sampled_not_walked() {
    // leaf (linear) <- mid2[1025 leaf] <- mid1[1025 mid2] <- top[1025 mid1] (1000 bounds).
    let stops = stitch_stops(
        "/leaf << /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >> def \
         /mid2 << /FunctionType 3 /Domain [0 1] /Bounds [] /Functions [ 1025 { leaf } repeat ] >> def \
         /mid1 << /FunctionType 3 /Domain [0 1] /Bounds [] /Functions [ 1025 { mid2 } repeat ] >> def \
         /top  << /FunctionType 3 /Domain [0 1] /Bounds [ 1 1 1000 { 1001 div } for ] /Functions [ 1025 { mid1 } repeat ] >> def \
         << /ShadingType 2 /ColorSpace /DeviceGray /Coords [0 0 100 0] /Function top >> shfill",
    );
    assert_eq!(stops.len(), 33);
}

/// Executable strings and keys other than names (#474): `cvx` makes a string executable, and
/// running it runs its text (PLRM 3rd ed. §3.3.1, `cvx`, `exec`); `load` takes any key (§8.2).
/// A program closes its shading dictionaries with an executable string (`>>` at Level 2 and up,
/// else a dictionary built with `counttomark`), and runs procedures it keeps under integer keys
/// through `stopped`.
#[test]
fn executable_strings_and_integer_keys() {
    let program = r##"%%BeginProlog
/ShadeKit 12 dict def ShadeKit begin
% A Level 1 way to make a dictionary of the pairs above a mark.
/pairs (counttomark 2 idiv dict begin { counttomark 0 eq { exit } if def } loop pop currentdict end) cvx def
/close /languagelevel where { pop languagelevel 2 ge } { false } ifelse { (>>) } { (pairs) } ifelse cvx def
/open /mark load def
/actions 3 dict def actions begin 0 { 0 1 0 setrgbcolor } def 1 { 1 0 0 setrgbcolor } def end
/act { actions 1 index known { actions begin load end stopped pop } { pop } ifelse } bind def
end
%%EndProlog
ShadeKit begin
open /ShadingType 3 /ColorSpace /DeviceRGB
  /Function open /FunctionType 2 /Domain [0 1] /C0 [1 1 0] /C1 [0 0.5 0] /N 1 close
  /Extend [true true] /Coords [50 50 0 50 50 40] close
gsave 10 10 80 80 rectclip shfill grestore
mark /Width 3 pairs /Width get 3 eq { 0 act 10 10 20 20 rectfill } if
2 act
end
(x) cvx xcheck (x) cvx cvlit xcheck not and { 50 50 10 10 rectfill } if
showpage"##;
    let r = open("Example Generator 1.0", program);
    let d = &r.document;
    let Some(Paint::Gradient(g)) = all(d).into_iter().find_map(|n| fill(&n).cloned()) else { panic!("no gradient") };
    assert_eq!(g.gradient.kind, vectorcraft_color::GradientKind::Radial);
    // `act` ran the procedure under key 0 (green); there is none under key 2.
    assert!(all(d).iter().any(|n| fill(n).and_then(Paint::color) == Some(Color::rgb(0.0, 1.0, 0.0))));
    assert_eq!(objects(d).len(), 3);
}

/// `restore` puts local VM back as it was at the `save` (PLRM 3rd ed., §3.7.3 "Save and
/// Restore"): dictionaries get back the entries they had, whatever changed them. A program that
/// keeps a count of the saves it has open in a dictionary, and refuses to go past a depth, reads
/// to its end only if each `restore` puts the count back (part of #505: files with many images,
/// each drawn between `save` and `restore`).
#[test]
fn restore_puts_dictionaries_back() {
    let r = read(
        "/Depth 2 dict def Depth /open 0 put \
         /enter { save Depth /open get 1 add dup 8 gt { rangecheck } if Depth exch /open exch put } def \
         1 1 100 { pop enter 0 0 1 1 rectfill restore } for Depth /open get 0 eq { 10 10 5 5 rectfill } if",
    );
    clean(&r);
    assert_eq!(objects(&r.document).len(), 101);
    // `def`, `store`, `put`, `undef` and dictionary `copy` are undone, newest first.
    super::tests::check(
        "/a 1 def /d 2 dict def d /k 1 put save /a 2 def /b 3 def d /k 2 put d /k undef 1 dict dup /z 9 put d copy pop \
         /a 4 store restore a 1 eq /b where not and d /k get 1 eq and d /z known not and",
    );
    // A save restored (itself, or through an outer one) is no longer valid: `invalidrestore`.
    super::tests::check("save dup restore { restore } stopped");
    super::tests::check("save save exch restore { restore } stopped");
    // `cachestatus` gives a device's cache sizes; its last, the most bytes one cached glyph may
    // take, is positive (chapter 8), and programs divide by it.
    super::tests::check("cachestatus 7 1 roll 6 { pop } repeat 0 gt");
}
