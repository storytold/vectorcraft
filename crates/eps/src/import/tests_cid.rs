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
