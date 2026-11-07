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
