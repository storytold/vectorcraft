//! CID-keyed fonts (PLRM 3rd ed., §5.11): CMaps built with the CIDInit procedure set's
//! operators, `composefont`, `FMapType` 9 fonts mapping codes to CIDs, Type 0 CIDFonts (Type 1
//! charstrings in the binary data `StartData` reads: Adobe Technical Note #5014) and Type 2
//! CIDFonts (TrueType glyphs through `CIDMap`). The CIDFonts are written from scratch: Type 0
//! glyphs are hand-made charstrings (Adobe Type 1 Font Format, chapter 6), Type 2 glyphs come
//! from the bundled Source Sans 3.

use vectorcraft_doc::NodeKind;

use super::tests::{all, bounds, check};
use super::tests_composite::{glyph, sfnts};
use crate::import::import;

/// A Type 1 charstring number (Adobe Type 1 Font Format, §6.2).
fn num(v: i32) -> Vec<u8> {
    match v {
        -107..=107 => vec![(v + 139) as u8],
        108..=1131 => vec![((v - 108) / 256 + 247) as u8, ((v - 108) % 256) as u8],
        -1131..=-108 => vec![((-v - 108) / 256 + 251) as u8, ((-v - 108) % 256) as u8],
        _ => [vec![255], v.to_be_bytes().to_vec()].concat(),
    }
}

/// A charstring from numbers and commands (`hsbw` 13, `rmoveto` 21, `rlineto` 5, `closepath`
/// 9, `callsubr` 10, `return` 11, `endchar` 14).
fn cs(parts: &[(&[i32], u8)]) -> Vec<u8> {
    parts.iter().flat_map(|(args, op)| args.iter().flat_map(|a| num(*a)).chain([*op])).collect()
}

/// `plain` encrypted with the charstring key and 4 leading bytes (Adobe Type 1 Font Format, §7).
fn encrypt(plain: &[u8]) -> Vec<u8> {
    let mut r: u16 = 4330;
    [0u8; 4]
        .iter()
        .chain(plain)
        .map(|&p| {
            let c = p ^ (r >> 8) as u8;
            r = (u16::from(c).wrapping_add(r)).wrapping_mul(52845).wrapping_add(22719);
            c
        })
        .collect()
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// A Type 0 CIDFont `/TestCID` with three CIDs: 0 an empty glyph 500 wide; 1 a square (100 to
/// 500 across, 500 high) 600 wide, its charstring plain (FDArray dictionary 0, `lenIV` -1); 2 a
/// triangle (500 across, 400 high) 500 wide drawn by a subroutine, encrypted (dictionary 1,
/// `lenIV` 4). The binary data section, written in hexadecimal when `hex_data`.
fn cid_font(hex_data: bool) -> Vec<u8> {
    let empty = cs(&[(&[0, 500], 13), (&[], 14)]);
    let square = cs(&[(&[0, 600], 13), (&[100, 0], 21), (&[400, 0], 5), (&[0, 500], 5), (&[-400, 0], 5), (&[], 9), (&[], 14)]);
    let triangle = encrypt(&cs(&[(&[0, 500], 13), (&[0], 10), (&[], 14)]));
    let subr = encrypt(&cs(&[(&[0, 0], 21), (&[500, 0], 5), (&[-250, 400], 5), (&[], 9), (&[], 11)]));
    // The offset table: FDArray index (1 byte) and offset (2 bytes) of CIDs 0 to 2 and the end.
    let table = 4 * 3;
    let offsets = [table, table + empty.len(), table + empty.len() + square.len(), table + empty.len() + square.len() + triangle.len()];
    let mut data = vec![];
    for (fd, at) in [0u8, 0, 1, 0].iter().zip(offsets) {
        data.push(*fd);
        data.extend_from_slice(&(at as u16).to_be_bytes());
    }
    data.extend(empty);
    data.extend(square);
    data.extend(triangle);
    let mut out = format!(
        "/CIDInit /ProcSet findresource begin\n20 dict begin\n/CIDFontName /TestCID def /CIDFontType 0 def\n\
         /CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Shapes) def /Supplement 0 def end def\n\
         /FontBBox [0 0 1000 1000] def\n/FDArray [\n\
         4 dict begin /FontMatrix [0.001 0 0 0.001 0 0] def /Private 2 dict dup begin /lenIV -1 def end def currentdict end\n\
         4 dict begin /FontMatrix [0.001 0 0 0.001 0 0] def /Private 2 dict dup begin /lenIV 4 def /Subrs [<{}>] def end def currentdict end\n\
         ] def\n/CIDMapOffset 0 def /FDBytes 1 def /GDBytes 2 def /CIDCount 3 def\n",
        hex(&subr)
    )
    .into_bytes();
    if hex_data {
        out.extend(format!("(Hex) {} StartData\n{}>\n", data.len(), hex(&data)).into_bytes());
    } else {
        out.extend(format!("(Binary) {} StartData ", data.len()).into_bytes());
        out.extend(data);
        out.push(b'\n');
    }
    out
}

/// CMap `/Test-H`: one-byte codes 00–7F and two-byte codes 8000–FFFF; A and B are CIDs 1 and 2,
/// <8001> CID 2, and codes 00–1F without a character take CID 1 as their notdef.
const CMAP: &str = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
    /CMapName /Test-H def /CMapType 1 def /WMode 0 def\n\
    2 begincodespacerange <00> <7f> <8000> <ffff> endcodespacerange\n\
    1 begincidrange <41> <42> 1 endcidrange\n1 begincidchar <8001> 2 endcidchar\n\
    1 beginnotdefrange <00> <1f> 1 endnotdefrange\n\
    endcmap CMapName currentdict /CMap defineresource pop end end\n\
    /TestFont /Test-H [/TestCID] composefont pop\n";

/// `body` after the CIDFont and the CMap, as an EPS file.
fn program(hex_data: bool, body: &str) -> Vec<u8> {
    let head = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n".to_vec();
    [head, cid_font(hex_data), CMAP.as_bytes().to_vec(), format!("{body}\nshowpage\n%%EOF\n").into_bytes()].concat()
}

fn paths(r: &crate::Imported) -> Vec<vectorcraft_geom::Rect> {
    all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).map(|n| bounds(n)).collect()
}

#[test]
fn type0_cidfonts_draw_their_charstrings_through_the_cmap() {
    for hex_data in [false, true] {
        let r = import(&program(hex_data, "/TestFont 50 selectfont 10 20 moveto (AB) show")).unwrap();
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        assert!(!all(&r.document).iter().any(|n| matches!(n.kind, NodeKind::Text(_))));
        let p = paths(&r);
        assert_eq!(p.len(), 2, "{p:?}");
        // The square, 100 to 500 units across at 50 pt an em; the triangle 600 units on.
        let (sq, tri) = (p[0], p[1]);
        assert!((sq.x0 - 15.0).abs() < 0.01 && (sq.width() - 20.0).abs() < 0.01 && (sq.height() - 25.0).abs() < 0.01, "{sq:?}");
        assert!((sq.y1 - 80.0).abs() < 0.01, "{sq:?}");
        assert!((tri.x0 - 40.0).abs() < 0.01 && (tri.width() - 25.0).abs() < 0.01 && (tri.height() - 20.0).abs() < 0.01, "{tri:?}");
    }
}

/// Codes take as many bytes as their codespace range says; codes without a character take their
/// notdef mapping's CID, else CID 0 (PLRM 3rd ed., §5.11.5).
#[test]
fn codes_map_by_their_codespace_with_notdef_fallbacks() {
    let fonts = String::from_utf8(cid_font(true)).unwrap() + CMAP;
    let width = |s: &str, w: f64| check(&format!("{fonts} /TestFont 50 selectfont {s} stringwidth pop {w} sub abs 0.01 lt"));
    width("(AB)", 55.0);
    // A two-byte code mapped by a character mapping.
    width("<8001>", 25.0);
    // A two-byte code (one character) without a mapping: CID 0.
    width("<9041>", 25.0);
    // A one-byte code with a notdef mapping: CID 1.
    width("<01>", 30.0);
}

/// `usecmap` brings in another CMap's ranges and mappings; the CMap's own come after them.
#[test]
fn usecmap_takes_the_parents_mappings() {
    let r = import(&program(
        true,
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /Test-H usecmap /CMapName /Child def\n\
         1 begincidchar <43> 2 endcidchar endcmap CMapName currentdict /CMap defineresource pop end end\n\
         /Child2 /Child [/TestCID] composefont 50 scalefont setfont 10 20 moveto (AC) show",
    ))
    .unwrap();
    let p = paths(&r);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!((p[1].x0 - 40.0).abs() < 0.01 && (p[1].height() - 20.0).abs() < 0.01, "{p:?}");
}

/// A Type 2 CIDFont with Source Sans 3's glyphs: CIDs 1 and 2 are H and I.
fn cid2_font(cid_map: &str) -> String {
    format!(
        "20 dict begin /CIDFontName /TestCID2 def /CIDFontType 2 def /FontType 11 def /FontMatrix [1 0 0 1 0 0] def\n\
         /CIDSystemInfo 3 dict dup begin /Registry (Test) def /Ordering (Latin) def /Supplement 0 def end def\n\
         /FontBBox [0 0 1 1] def /CIDCount 3 def /GDBytes 2 def /CIDMap {cid_map} def\n\
         /sfnts [\n{}] def CIDFontName currentdict end /CIDFont defineresource pop\n",
        sfnts()
    )
}

#[test]
fn type2_cidfonts_draw_their_truetype_glyphs_through_cidmap() {
    let (h, wh) = glyph('H');
    let (i, wi) = glyph('I');
    for cid_map in [format!("<0000{h:04x}{i:04x}>"), format!("[<0000{h:04x}> <{i:04x}>]")] {
        let body = format!("{}/T2 /Identity-H [/TestCID2] composefont 50 scalefont setfont 10 20 moveto <00010002> show", cid2_font(&cid_map));
        let r = super::tests::read(&body);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let p = paths(&r);
        assert_eq!(p.len(), 2, "{cid_map}: {p:?}");
        let (bh, bi) = (p[0], p[1]);
        assert!(bh.x0 > 10.0 && bh.x0 < 18.0 && bh.height() > 25.0 && bh.height() < 40.0, "{bh:?}");
        assert!(bi.x0 > 10.0 + 50.0 * wh - 1.0 && bi.x1 < 10.0 + 50.0 * (wh + wi), "{bi:?}");
    }
}

/// `Identity-V`: each glyph an em below the one before (writing mode 1).
#[test]
fn identity_v_sets_glyphs_downwards() {
    let (h, _) = glyph('H');
    let (i, _) = glyph('I');
    let body = format!(
        "{}/T2V /Identity-V [/TestCID2] composefont 40 scalefont setfont 50 90 moveto <00010002> show",
        cid2_font(&format!("<0000{h:04x}{i:04x}>"))
    );
    let p = paths(&super::tests::read(&body));
    assert_eq!(p.len(), 2, "{p:?}");
    // Document y runs down: the I is 40 pt (an em) below the H.
    assert!((p[1].y1 - p[0].y1 - 40.0).abs() < 0.5, "{p:?}");
    check(&format!(
        "{}/T2V /Identity-V [/TestCID2] composefont 40 scalefont setfont (\\000\\001\\000\\002) stringwidth -80 sub abs 0.01 lt exch abs 0.01 lt and",
        cid2_font("0")
    ));
}

/// A CIDFont the file doesn't carry draws nothing, an em a character, and says so.
#[test]
fn cidfonts_the_file_lacks_draw_nothing_with_a_warning() {
    let r = super::tests::read(
        "/X /Identity-H [/NotInFile] composefont 50 scalefont setfont 10 20 moveto <00410042> show 0 0 1 setrgbcolor currentpoint 5 5 rectfill",
    );
    let p = paths(&r);
    assert_eq!(p.len(), 1, "{p:?}");
    // The point moved two ems along.
    assert!((p[0].x0 - 110.0).abs() < 0.01, "{p:?}");
    assert!(r.warnings.iter().any(|w| w.contains("CID-keyed")), "{:?}", r.warnings);
}

/// The CIDInit operators are found through the procedure set, not systemdict.
#[test]
fn cidinit_operators_are_not_in_systemdict() {
    check("systemdict /begincmap known not /CIDInit /ProcSet findresource /begincmap known and systemdict /composefont known and");
}

/// A CFF INDEX (Adobe Technical Note #5176, §5) of `items`, with 2-byte offsets.
fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    out.push(2);
    let mut at = 1u16;
    out.extend(at.to_be_bytes());
    for i in items {
        at += i.len() as u16;
        out.extend(at.to_be_bytes());
    }
    out.extend(items.iter().flatten());
    out
}

/// A CFF DICT operand as a 5-byte integer (TN #5176, Table 3), so offsets don't change its size.
fn cff_int(v: i32) -> Vec<u8> {
    [vec![29], v.to_be_bytes().to_vec()].concat()
}

/// A Type 2 charstring (Adobe Technical Note #5177) from numbers, each a 3-byte integer
/// (`28`), and operators (`rmoveto` 21, `rlineto` 5, `endchar` 14).
fn cs2(parts: &[(&[i16], u8)]) -> Vec<u8> {
    parts.iter().flat_map(|(args, op)| args.iter().flat_map(|a| [vec![28], a.to_be_bytes().to_vec()].concat()).chain([*op])).collect()
}

/// A CID-keyed CFF font `/CffCID` (TN #5176, §18): glyph 0 `.notdef`, glyph 1 CID 1 a square
/// (100 to 500 across, 500 high) 600 wide, glyph 2 CID 7 a triangle (500 across, 400 high) 500
/// wide (the first number of a charstring its width, `nominalWidthX` 0).
fn cid_cff() -> Vec<u8> {
    let glyphs = vec![
        cs2(&[(&[0], 14)]),
        cs2(&[(&[600, 100, 0], 21), (&[400, 0, 0, 500, -400, 0], 5), (&[], 14)]),
        cs2(&[(&[500, 0, 0], 21), (&[500, 0, -250, 400], 5), (&[], 14)]),
    ];
    let header = vec![1, 0, 4, 2];
    let names = cff_index(&[b"CffCID".to_vec()]);
    let strings = cff_index(&[b"Test".to_vec(), b"Shapes".to_vec()]);
    let global = cff_index(&[]);
    // The top DICT: ROS (12 30), CIDCount (12 34), charset (15), CharStrings (17), FDArray
    // (12 36), FDSelect (12 37), each offset a 5-byte integer.
    let top = |charset: i32, strings_at: i32, fd_array: i32, fd_select: i32| -> Vec<u8> {
        [
            cff_int(391),
            cff_int(392),
            cff_int(0),
            vec![12, 30],
            cff_int(8),
            vec![12, 34],
            cff_int(charset),
            vec![15],
            cff_int(strings_at),
            vec![17],
            cff_int(fd_array),
            vec![12, 36],
            cff_int(fd_select),
            vec![12, 37],
        ]
        .concat()
    };
    let top_len = cff_index(&[top(0, 0, 0, 0)]).len();
    let charset_at = header.len() + names.len() + top_len + strings.len() + global.len();
    // Format 0: the CIDs of glyphs 1 and 2.
    let charset = vec![0, 0, 1, 0, 7];
    let fd_select_at = charset_at + charset.len();
    let fd_select = vec![0, 0, 0, 0];
    let strings_at = fd_select_at + fd_select.len();
    let char_strings = cff_index(&glyphs);
    let private_at = strings_at + char_strings.len();
    // defaultWidthX (20) and nominalWidthX (21) 0.
    let private = [cff_int(0), vec![20], cff_int(0), vec![21]].concat();
    let fd_array_at = private_at + private.len();
    let fd = [cff_int(private.len() as i32), cff_int(private_at as i32), vec![18]].concat();
    let fd_array = cff_index(&[fd]);
    let top = cff_index(&[top(charset_at as i32, strings_at as i32, fd_array_at as i32, fd_select_at as i32)]);
    [header, names, top, strings, global, charset, fd_select, char_strings, private, fd_array].concat()
}

/// A FontSet holding CID-keyed Compact Font Format data (PLRM 3rd ed., §5.8.1, `StartData`;
/// TN #5176) defines its CIDFont, whose glyphs are drawn by their CIDs.
#[test]
fn cff_fontsets_draw_their_cidfonts_glyphs() {
    let data = cid_cff();
    let head = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n".to_vec();
    let set =
        [format!("/FontSetInit /ProcSet findresource begin\n/CffCID {} StartData ", data.len()).into_bytes(), data, b"\nend\n".to_vec()].concat();
    let body = "/CffFont /Identity-H [/CffCID] composefont 50 scalefont setfont 10 20 moveto <00010007> show\n\
                (\\000\\001\\000\\007) stringwidth pop 55 sub abs 0.01 lt { 0 0 5 5 rectfill } if\nshowpage\n%%EOF\n";
    let r = import(&[head, set, body.as_bytes().to_vec()].concat()).unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let p = paths(&r);
    assert_eq!(p.len(), 3, "{p:?}");
    let (sq, tri) = (p[0], p[1]);
    assert!((sq.x0 - 15.0).abs() < 0.01 && (sq.width() - 20.0).abs() < 0.01 && (sq.height() - 25.0).abs() < 0.01, "{sq:?}");
    assert!((tri.x0 - 40.0).abs() < 0.01 && (tri.width() - 25.0).abs() < 0.01 && (tri.height() - 20.0).abs() < 0.01, "{tri:?}");
}

/// Damaged FontSet data (each byte of the CFF data changed, in turn) is read as fonts or as
/// nothing, never a crash.
#[test]
fn damaged_cff_fontsets_never_panic() {
    let data = cid_cff();
    let head = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n".to_vec();
    let body = b"/CffFont /Identity-H [/CffCID] composefont 50 scalefont setfont 10 20 moveto <000100070002> show showpage\n%%EOF\n".to_vec();
    for at in 0..data.len() {
        for v in [0u8, 0x7f, 0xff, data[at].wrapping_add(1)] {
            let mut d = data.clone();
            d[at] = v;
            let set =
                [format!("/FontSetInit /ProcSet findresource begin\n/CffCID {} StartData ", d.len()).into_bytes(), d, b"\nend\n".to_vec()].concat();
            let _ = import(&[head.clone(), set, body.clone()].concat());
        }
    }
}

/// A CMap mapping two-byte codes to base fonts' codes and glyph names (`beginbfchar`,
/// `beginbfrange`, `usefont`: Adobe Technical Note #5099, "Building CMap Files for CID-Keyed
/// Fonts"): font 0 a Type 42 font that carries Source Sans 3, font 1 a font found by name.
const BF_CMAP: &str = "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
    /CMapName /TestBF-H def /CMapType 1 def /WMode 0 def\n\
    1 begincodespacerange <0000> <ffff> endcodespacerange\n\
    0 usefont 1 beginbfchar <0001> <01> endbfchar 1 beginbfrange <0002> <0003> [/g2 /g1] endbfrange\n\
    1 usefont 1 beginbfrange <0010> <0012> <41> endbfrange\n\
    endcmap CMapName currentdict /CMap defineresource pop end end\n";

#[test]
fn cmaps_map_codes_to_base_fonts_codes_and_glyph_names() {
    let (h, wh) = glyph('H');
    let (i, wi) = glyph('I');
    // `/Plain42`: codes 1 and 2 are glyphs g1 (H) and g2 (I); g2 is also named outside the encoding.
    let plain = format!(
        "10 dict begin /FontName /Plain42 def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
         /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for Encoding 1 /g1 put\n\
         /CharStrings 3 dict def CharStrings begin /.notdef 0 def /g1 {h} def /g2 {i} def end\n\
         /sfnts [\n{}] def FontName currentdict end definefont pop\n",
        sfnts()
    );
    let body = format!("{plain}{BF_CMAP}/BF /TestBF-H [/Plain42 /Helvetica] composefont 50 scalefont setfont 10 20 moveto <000100020003> show");
    let r = super::tests::read(&body);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    let p = paths(&r);
    assert_eq!(p.len(), 3, "{p:?}");
    // H, I, H, each its own width along.
    assert!(p[0].x0 > 10.0 && p[0].x0 < 18.0, "{p:?}");
    assert!(p[1].x0 > 10.0 + 50.0 * wh - 1.0 && p[1].x1 < 10.0 + 50.0 * (wh + wi), "{p:?}");
    assert!(p[2].x0 > 10.0 + 50.0 * (wh + wi) && (p[2].width() - p[0].width()).abs() < 0.01, "{p:?}");
    // Codes to a font found by name set its characters as type.
    let body = format!("{plain}{BF_CMAP}/BF /TestBF-H [/Plain42 /Helvetica] composefont 20 scalefont setfont 10 20 moveto <001000110012> show");
    let r = super::tests::read(&body);
    let text: String = all(&r.document)
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.plain_text()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "ABC");
}

/// A CID-keyed font over a predefined Unicode CMap the file doesn't define, its CIDFont only
/// named: the codes say their characters, set as type in the font of the CIDFont's name,
/// whether found as `CIDFont-CMap` or composed.
#[test]
fn named_cidfonts_over_unicode_cmaps_set_their_characters_as_type() {
    let texts = |r: &crate::Imported| -> Vec<vectorcraft_doc::TextObject> {
        all(&r.document)
            .iter()
            .filter_map(|n| match &n.kind {
                NodeKind::Text(t) => Some((**t).clone()),
                _ => None,
            })
            .collect()
    };
    let r = super::tests::read("/Ryumin-Light-UniJIS-UCS2-H findfont 20 scalefont setfont 10 20 moveto <4e2d0041> show");
    assert!(super::tests::program_warnings(&r.warnings).is_empty(), "{:?}", r.warnings);
    let t = texts(&r);
    assert_eq!(t.len(), 1, "{:?}", all(&r.document));
    assert_eq!(t[0].plain_text(), "\u{4e2d}A");
    assert_eq!((t[0].runs[0].style.font_family.as_str(), t[0].runs[0].style.size), ("Ryumin", 20.0));
    let r = super::tests::read("/X /UniGB-UTF8-H [/NotInFile] composefont 20 scalefont setfont 10 20 moveto (\\344\\270\\255A) show");
    assert_eq!(texts(&r).iter().map(|t| t.plain_text()).collect::<String>(), "\u{4e2d}A");
    // The point moves past the type.
    check("/Ryumin-Light-UniJIS-UCS2-H findfont 20 scalefont setfont <00410042> stringwidth pop dup 5 gt exch 40 lt and");
}

/// A Type 2 CIDFont the file carries (its glyph space one unit an em), over a Unicode CMap the
/// file doesn't define, sets its characters as type at the font's size, not a thousand times it.
#[test]
fn type2_cidfonts_over_unicode_cmaps_set_type_at_their_size() {
    let r =
        super::tests::read(&format!("{}/X /UniGB-UCS2-H [/TestCID2] composefont 20 scalefont setfont 10 20 moveto <4e2d0041> show", cid2_font("0")));
    let sizes: Vec<f64> = all(&r.document)
        .iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Text(t) => Some(t.runs[0].style.size * t.xf.determinant().abs().sqrt()),
            _ => None,
        })
        .collect();
    assert!(!sizes.is_empty() && sizes.iter().all(|s| (s - 20.0).abs() < 0.5), "{sizes:?} {:?}", r.warnings);
}

/// A code a base-font CMap doesn't map, in a base font, is that font's code 0 (`.notdef`): it
/// moves the point by that glyph's width, not by a thousand ems, and the font isn't missing.
#[test]
fn unmapped_codes_in_base_fonts_take_their_notdef() {
    let plain = format!(
        "10 dict begin /FontName /Plain42 def /FontType 42 def /FontMatrix [1 0 0 1 0 0] def /FontBBox [0 0 1 1] def /PaintType 0 def\n\
         /Encoding 256 array def 0 1 255 {{ Encoding exch /.notdef put }} for\n\
         /CharStrings 1 dict def CharStrings begin /.notdef 0 def end\n\
         /sfnts [\n{}] def FontName currentdict end definefont pop\n",
        sfnts()
    );
    check(&format!(
        "{plain}{BF_CMAP}/BF /TestBF-H [/Plain42 /Helvetica] composefont 50 scalefont setfont <0009> stringwidth pop dup 0 gt exch 100 lt and"
    ));
    let r = super::tests::read(&format!(
        "{plain}{BF_CMAP}/BF /TestBF-H [/Plain42 /Helvetica] composefont 50 scalefont setfont 10 20 moveto <0009> show 0 0 10 10 rectfill"
    ));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

/// A CMap's mappings take font 0 until its own `usefont`, whatever font number an earlier CMap
/// ended with (Adobe Technical Note #5014).
#[test]
fn each_cmap_starts_at_font_0() {
    let r = import(&program(
        true,
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Two-H def\n\
         1 begincodespacerange <00> <ff> endcodespacerange 1 usefont 1 begincidchar <41> 1 endcidchar\n\
         endcmap CMapName currentdict /CMap defineresource pop end end\n\
         /CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /One-H def\n\
         1 begincodespacerange <00> <ff> endcodespacerange 1 begincidrange <41> <42> 1 endcidrange\n\
         endcmap CMapName currentdict /CMap defineresource pop end end\n\
         /One /One-H [/TestCID] composefont 50 scalefont setfont 10 20 moveto (AB) show",
    ))
    .unwrap();
    assert_eq!(paths(&r).len(), 2, "{:?}", r.warnings);
}

/// A Type 0 CIDFont with a matrix of its own (its FDArray matrices map 1000 units to an em):
/// vertical type moves an em a character, not a thousand.
#[test]
fn cidfonts_with_their_own_matrix_move_an_em_a_character() {
    let font = String::from_utf8(cid_font(true)).unwrap().replace("/CIDFontType 0 def", "/CIDFontType 0 def /FontMatrix [1 0 0 1 0 0] def");
    check(&format!(
        "{font}{CMAP} /V /Identity-V [/TestCID] composefont 40 scalefont setfont (\\000\\001\\000\\002) stringwidth -80 sub abs 0.01 lt exch abs 0.01 lt and"
    ));
}

/// `restore` undoes the matrices `StartData` gives a CIDFont and its FDArray dictionaries, as
/// it undoes the program's own changes.
#[test]
fn restore_undoes_the_matrices_start_data_sets() {
    let font = String::from_utf8(cid_font(true)).unwrap().replacen(
        "4 dict begin /FontMatrix [0.001 0 0 0.001 0 0] def /Private 2 dict dup begin /lenIV -1 def end def currentdict end",
        "fd0",
        1,
    );
    check(&format!(
        "/fd0 4 dict begin /FontMatrix [0.001 0 0 0.001 0 0] def /Private 2 dict dup begin /lenIV -1 def end def currentdict end def\n\
         /sv save def\n{font}\nsv restore fd0 /FontMatrix get 0 get 0.001 sub abs 1e-9 lt"
    ));
}

/// Notdef mappings added to a CMap that was already shown with take effect (its mappings are
/// read again).
#[test]
fn notdef_mappings_added_later_are_read() {
    let fonts = String::from_utf8(cid_font(true)).unwrap() + CMAP;
    check(&format!(
        "{fonts} /TestFont 50 selectfont <21> stringwidth pop pop\n\
         /CIDInit /ProcSet findresource begin /Test-H /CMap findresource begin 1 beginnotdefrange <20> <2f> 1 endnotdefrange end end\n\
         <21> stringwidth pop 30 sub abs 0.01 lt"
    ));
}

/// A CMap built of many small blocks pays for copying its mappings at each block.
#[test]
fn many_small_cmap_blocks_run_out_of_the_budget() {
    super::tests::runs_out_of_budget("/CIDInit /ProcSet findresource begin 12 dict begin begincmap { 1 begincidchar <41> 1 endcidchar } loop");
}

/// A CIDFont's subroutines are decrypted once, not again for every glyph: measuring a glyph of
/// a font with 10 MB of them in a loop ends within the budget.
#[test]
fn cidfont_subroutines_are_read_once() {
    let font = String::from_utf8(cid_font(true)).unwrap();
    let at = font.find("/Subrs [<").unwrap() + "/Subrs [".len();
    let font = format!(
        "{}{} 0 1 9998 {{ pop 1000 string }} for{}",
        &font[..at],
        &font[at..font[at..].find(']').unwrap() + at],
        &font[font[at..].find(']').unwrap() + at..]
    );
    super::tests::runs_out_of_budget(&format!("{font}{CMAP} /TestFont 50 selectfont {{ (B) stringwidth pop pop }} loop"));
}

/// Binary data larger than most operators' counts (4 MB) is read: CJK CIDFonts carry more.
#[test]
fn start_data_reads_more_than_4_mb() {
    let mut font = cid_font(false);
    let marker = b"(Binary) ";
    let at = font.windows(marker.len()).position(|w| w == marker).unwrap();
    let rest = font.split_off(at);
    let len_end = rest.iter().skip(marker.len()).position(|b| *b == b' ').unwrap() + marker.len();
    let n: usize = std::str::from_utf8(&rest[marker.len()..len_end]).unwrap().parse().unwrap();
    let data_at = len_end + " StartData ".len();
    let mut data = rest[data_at..data_at + n].to_vec();
    data.resize(5_000_000, 0);
    font.extend(format!("(Binary) {} StartData ", data.len()).into_bytes());
    font.extend(data);
    font.push(b'\n');
    let head = b"%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n".to_vec();
    let r = import(&[head, font, CMAP.as_bytes().to_vec(), b"/TestFont 50 selectfont 10 20 moveto (AB) show\nshowpage\n%%EOF\n".to_vec()].concat())
        .unwrap();
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
    assert_eq!(paths(&r).len(), 2);
}

/// Decoding a string through a CMap with many mappings counts against the operation budget.
#[test]
fn decoding_through_a_large_cmap_runs_out_of_the_budget() {
    // A CIDFont the file doesn't carry: its glyphs cost nothing, so the decoding is what counts.
    super::tests::runs_out_of_budget(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Big-H def\n\
         2 begincodespacerange <00> <7f> <8000> <ffff> endcodespacerange\n\
         16000 begincidrange 0 1 15999 { /i exch def 2 string dup 0 i 256 idiv 128 add put dup 1 i 256 mod put dup 1 } for endcidrange\n\
         endcmap CMapName currentdict /CMap defineresource pop end end\n\
         /Big /Big-H [/NotInFile] composefont 50 scalefont setfont /s 1000 string def { s stringwidth pop pop } loop",
    );
}

/// A CID-keyed font's glyphs are grouped under the font's name: a CID stands for no character.
#[test]
fn cid_glyph_groups_take_the_fonts_name() {
    let r = import(&program(true, "/TestFont 50 selectfont 10 20 moveto (AB) show")).unwrap();
    let names: Vec<_> = all(&r.document).iter().filter(|n| matches!(n.kind, NodeKind::Group { .. })).map(|n| n.name.clone()).collect();
    assert_eq!(names, [Some("TestFont".to_string())]);
}

/// One show of a long string through a CMap with many mappings stops at the operation budget,
/// not after decoding the whole string.
#[test]
fn one_long_show_through_a_large_cmap_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Big-H def\n\
         2 begincodespacerange <00> <7f> <8000> <ffff> endcodespacerange\n\
         /mk { 0 1 15999 { /i exch def 2 string dup 0 i 256 idiv 128 add put dup 1 i 256 mod put dup 1 } for } def\n\
         16000 begincidrange mk endcidrange 16000 begincidrange mk endcidrange 16000 begincidrange mk endcidrange 16000 begincidrange mk endcidrange\n\
         16000 beginnotdefrange mk endnotdefrange 16000 beginnotdefrange mk endnotdefrange 16000 beginnotdefrange mk endnotdefrange 16000 beginnotdefrange mk endnotdefrange\n\
         endcmap CMapName currentdict /CMap defineresource pop end end\n\
         /Big /Big-H [/NotInFile] composefont 50 scalefont setfont 4000000 string stringwidth pop pop",
    );
}

/// The evaluator limits each glyph, not the file: measuring a glyph whose subroutines (1 to 8
/// each calling the next four times) run a quarter of a million operations in a loop ends within
/// the operation budget.
#[test]
fn cid_glyphs_measured_in_a_loop_run_out_of_the_budget() {
    let font = String::from_utf8(cid_font(true)).unwrap();
    let at = font.find("/Subrs [<").unwrap() + "/Subrs [".len();
    let end = font[at..].find(']').unwrap() + at;
    let mut subrs = vec![encrypt(&cs(&[(&[1], 10), (&[0, 0], 21), (&[500, 0], 5), (&[-250, 400], 5), (&[], 9), (&[], 11)]))];
    for k in 1..=8 {
        subrs.push(encrypt(&cs(&[(&[k + 1], 10), (&[k + 1], 10), (&[k + 1], 10), (&[k + 1], 10), (&[], 11)])));
    }
    subrs.push(encrypt(&cs(&[(&[], 11)])));
    let list: String = subrs.iter().map(|s| format!("<{}> ", hex(s))).collect();
    let font = format!("{}{list}{}", &font[..at], &font[end..]);
    check(&format!("{font}{CMAP} /TestFont 50 selectfont (B) stringwidth pop 25 sub abs 0.01 lt"));
    super::tests::runs_out_of_budget(&format!("{font}{CMAP} /TestFont 50 selectfont {{ (B) stringwidth pop pop }} loop"));
}

/// Setting type over a Unicode CMap copies the composite font for every string: the copy is
/// charged by the font's size, so a font with a huge number of keys shown in a loop runs out of
/// the operation budget.
#[test]
fn unicode_type_in_a_huge_font_in_a_loop_runs_out_of_the_budget() {
    super::tests::runs_out_of_budget(
        "/Ryumin-Light-UniJIS-UCS2-H findfont dup length 200000 add dict copy dup begin 0 1 199999 { dup def } for end\n\
         /Huge exch definefont 20 scalefont setfont { <0041> stringwidth pop pop } loop",
    );
}

/// A `beginbfrange` destination array that holds itself is read without recursing into it (an
/// array element that is an array maps nothing), so showing its code is no crash.
#[test]
fn a_bfrange_array_holding_itself_is_no_crash() {
    let body = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /Self-H def\n\
                1 begincodespacerange <00> <ff> endcodespacerange\n\
                /a 1 array def a 0 a put 1 beginbfrange <00> <00> a endbfrange\n\
                endcmap CMapName currentdict /CMap defineresource pop end end\n\
                /S /Self-H [/Helvetica] composefont 20 scalefont setfont 10 20 moveto <00> show 0 0 10 10 rectfill";
    let r = super::tests::read(body);
    assert_eq!(paths(&r).len(), 1, "{:?}", r.warnings);
}

/// Drawing a glyph of a FontSet's CIDFont counts the charstring bytes it enters, as Type 1
/// glyphs do, so glyphs that run long (nested subroutines) are paid for by the `show` that
/// draws them, not a flat amount.
#[test]
fn cff_glyphs_count_the_charstrings_they_run() {
    let cff = super::cid::Cff::new(std::rc::Rc::from(cid_cff()), 0);
    assert!(cff.glyph(1).is_some());
    assert!(cff.take_work() > 0);
    assert_eq!(cff.take_work(), 0);
}

/// `usecmap` copies the parent's mappings into new arrays: the copies count against the memory
/// budget, so keeping the CMaps made from a large one in a loop stops with an error instead of
/// growing without bound.
#[test]
fn usecmap_copies_count_against_the_memory_budget() {
    let r = import(&super::tests::eps(
        "0 0 10 10 rectfill /CIDInit /ProcSet findresource begin 12 dict begin begincmap\n\
         1 begincodespacerange <0000> <ffff> endcodespacerange\n\
         0 1 599 { pop 100 begincidrange 0 1 99 { pop <0000> <0001> 0 } for endcidrange } for\n\
         endcmap /Big currentdict /CMap defineresource pop end end\n\
         /CIDInit /ProcSet findresource begin { 12 dict begin /Big usecmap currentdict end } loop",
    ))
    .unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("too much memory")), "{:?}", r.warnings);
}
