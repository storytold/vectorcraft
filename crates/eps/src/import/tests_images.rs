//! Image data read as the PostScript Language Reference (3rd ed.) defines it: several data sources
//! read in turn (§4.10 "Images", `image` and `colorimage` in chapter 8), rows padded to a whole
//! byte at every bit depth and odd widths (§4.10), and Flate predictors undone row by row (§3.13,
//! `FlateDecode` parameters) (#498). CMYK images keep their ink amounts, as the Decode array
//! reads their samples, and their transparent pixels become an opacity mask. CCITT fax data
//! (`CCITTFaxDecode`: ITU-T T.4 Group 3 and T.6 Group 4) decodes to rows of 1-bit samples.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use fax::{BitWriter, Bits};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{ImageBlob, Node, NodeKind};

use super::import;
use super::tests::{all, eps, read};
use super::tests_generators::pixel;

/// Each pixel of the first image, row by row.
fn pixels(r: &crate::Imported, w: u32, h: u32) -> Vec<[u8; 4]> {
    assert!(!r.preview && r.warnings.is_empty(), "{:?}", r.warnings);
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| pixel(&r.document, x, y)).collect()
}

/// The first image object.
fn first_image(r: &crate::Imported) -> Arc<Node> {
    assert!(!r.preview && r.warnings.is_empty(), "{:?}", r.warnings);
    all(&r.document).into_iter().find(|n| matches!(n.kind, NodeKind::Image(_))).expect("an image")
}

/// The blob of image object `n`.
fn blob<'d>(r: &'d crate::Imported, n: &Node) -> &'d ImageBlob {
    let NodeKind::Image(im) = &n.kind else { panic!("an image: {n:?}") };
    &r.document.images[&im.key]
}

/// The ink amounts of the first image, a CMYK TIFF.
fn inks(r: &crate::Imported) -> Inks {
    let b = blob(r, &first_image(r));
    assert_eq!(b.mime, "image/tiff");
    b.cmyk().expect("CMYK samples")
}

/// A CMYK image whose four data sources are procedures reading one file, a row of one ink each:
/// the image operator calls them in turn, so each gets its own rows. Read one after the other,
/// the cyan source took the first four rows of all four inks and the image came out in
/// horizontal bands (#498).
#[test]
fn procedures_reading_one_file_are_read_in_turn() {
    // 3 × 2 pixels; rows: C M Y K of row 0, then of row 1.
    let rows = ["ff0000", "00ff00", "000000", "000000", "000000", "000000", "ffff00", "0000ff"];
    let r = read(&format!(
        "/DeviceCMYK setcolorspace 30 20 scale /f currentfile /ASCIIHexDecode filter def \
         << /ImageType 1 /Width 3 /Height 2 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [3 0 0 -2 0 2] \
         /MultipleDataSources true /DataSource [ {{f 3 string readstring pop}} {{f 3 string readstring pop}} \
         {{f 3 string readstring pop}} {{f 3 string readstring pop}} ] >> image\n{}>",
        rows.concat()
    ));
    let (cyan, magenta, yellow, black, white) = ([255, 0, 0, 0], [0, 255, 0, 0], [0, 0, 255, 0], [0, 0, 0, 255], [0, 0, 0, 0]);
    let want = [cyan, magenta, white, yellow, yellow, black].concat();
    assert_eq!(inks(&r).data, want);

    // The operand form: `{r} {g} {b} true 3 colorimage`, procedures reading one file.
    let r = read(
        "30 20 scale /f currentfile /ASCIIHexDecode filter def \
         3 2 8 [3 0 0 -2 0 2] {f 3 string readstring pop} {f 3 string readstring pop} {f 3 string readstring pop} true 3 colorimage\n\
         ff0000 00ff00 0000ff 102030 405060 708090>",
    );
    assert_eq!(pixels(&r, 3, 2), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [16, 64, 112, 255], [32, 80, 128, 255], [48, 96, 144, 255]]);
}

/// Separate strings and files as the sources: each is read for its own plane, whatever the row.
#[test]
fn separate_sources_keep_their_planes() {
    let r = read("30 20 scale 3 2 8 [3 0 0 -2 0 2] <ff0000102030> <00ff00405060> <0000ff708090> true 3 colorimage");
    assert_eq!(pixels(&r, 3, 2), [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [16, 64, 112, 255], [32, 80, 128, 255], [48, 96, 144, 255]]);
}

/// CMYK samples keep their ink amounts in a CMYK TIFF: in the operand and dictionary forms, from
/// hex and binary data, from one source or one a component, at 4, 8 and 16 bits, read through
/// the Decode array. Gray and RGB images stay PNGs.
#[test]
fn cmyk_images_keep_their_ink_amounts() {
    let want = [10, 20, 30, 40, 0, 102, 127, 8];
    let r = read("30 20 scale 2 1 8 [2 0 0 -1 0 1] <0a141e28 00667f08> false 4 colorimage");
    assert_eq!(inks(&r), Inks::new(2, 1, want.to_vec()).unwrap());
    assert!(first_image(&r).extras().mask.is_none(), "an opaque image has no mask");
    let r = read("30 20 scale 2 1 8 [2 0 0 -1 0 1] <0a00> <1466> <1e7f> <2808> true 4 colorimage");
    assert_eq!(inks(&r).data, want, "one source a component");
    let wide: String = want.iter().map(|v| format!("{:04x}", u16::from(*v) * 257)).collect();
    let r = read(&format!("30 20 scale 2 1 16 [2 0 0 -1 0 1] <{wide}> false 4 colorimage"));
    assert_eq!(inks(&r).data, want, "16 bits");
    let r = read("30 20 scale 2 1 4 [2 0 0 -1 0 1] <f0800f0f> false 4 colorimage");
    assert_eq!(inks(&r).data, [255, 0, 136, 0, 0, 255, 0, 255], "4 bits");
    // Binary data after the operator, with a Decode array that inverts the samples.
    let samples: String = want.iter().map(|v| char::from(*v)).collect();
    let r = read(&format!(
        "/DeviceCMYK setcolorspace 30 20 scale << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 8 /Decode [1 0 1 0 1 0 1 0] \
         /ImageMatrix [2 0 0 -1 0 1] /DataSource currentfile >> image\n{samples}"
    ));
    assert_eq!(inks(&r).data, want.map(|v| 255 - v), "binary, inverted");
    let r = read(
        "[/ICCBased << /N 4 >>] setcolorspace 30 20 scale << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 8 \
         /ImageMatrix [2 0 0 -1 0 1] /DataSource <0a141e28 00667f08> >> image",
    );
    assert_eq!(inks(&r).data, want, "ICC-based with four components");
    for program in ["30 20 scale 2 1 8 [2 0 0 -1 0 1] <0a141e 00667f> false 3 colorimage", "30 20 scale 2 1 8 [2 0 0 -1 0 1] <0a14> image"] {
        let r = read(program);
        assert_eq!(blob(&r, &first_image(&r)).mime, "image/png", "{program}");
    }
}

/// The transparent pixels of a CMYK image (`MaskColor`, an ImageType 3 mask) become its opacity
/// mask, a grayscale image over it as in PDF import, and the inks stay as they are. A key color
/// that no pixel has adds no mask.
#[test]
fn masked_cmyk_images_keep_their_inks_and_carry_the_mask_as_an_opacity_mask() {
    let data = "/Width 2 /Height 1 /BitsPerComponent 8 /Decode [0 1 0 1 0 1 0 1] /ImageMatrix [2 0 0 -1 0 1] /DataSource <0a141e28 00667f08>";
    let keyed = format!("/DeviceCMYK setcolorspace 30 20 scale << /ImageType 4 {data} /MaskColor [10 20 30 40] >> image");
    let stencil = format!(
        "/DeviceCMYK setcolorspace 30 20 scale << /ImageType 3 /InterleaveType 3 /DataDict << /ImageType 1 {data} >> \
         /MaskDict << /ImageType 1 /Width 2 /Height 1 /BitsPerComponent 1 /Decode [0 1] /ImageMatrix [2 0 0 -1 0 1] /DataSource <40> >> >> image"
    );
    for (program, alpha) in [(&keyed, [0, 255]), (&stencil, [255, 0])] {
        let r = read(program);
        assert_eq!(inks(&r).data, [10, 20, 30, 40, 0, 102, 127, 8], "the inks as they were: {program}");
        let n = first_image(&r);
        let mask = n.extras().mask.as_ref().expect("an opacity mask");
        let gray = image::load_from_memory(&blob(&r, &mask.art).bytes).unwrap().to_luma8();
        assert_eq!(gray.into_raw(), alpha, "the mask's values: {program}");
        assert_eq!(n.geometric_bounds(), mask.art.geometric_bounds(), "the mask covers the image");
    }
    let r = read(&keyed.replace("[10 20 30 40]", "[1 2 3 4]"));
    assert!(first_image(&r).extras().mask.is_none());
}

/// A CMYK JPEG gives its samples as stored, for the Decode array to read: Adobe's inverted
/// samples with `[1 0 1 0 1 0 1 0]`, the ink amounts themselves with `[0 1 0 1 0 1 0 1]`.
#[test]
fn cmyk_jpegs_keep_their_ink_amounts() {
    use vectorcraft_render::encode::jpeg::{ColorModel, JpegOptions, encode};
    let want: Vec<u8> = [[0, 0, 0, 0], [255, 0, 0, 0], [10, 20, 30, 40], [0, 102, 255, 0]].concat();
    // The encoder stores Adobe's inverted samples of the ink amounts it is given.
    let jpeg = |inks: &[u8]| {
        let o = JpegOptions { color_model: ColorModel::Cmyk, embed_icc: false, ..JpegOptions::default() };
        encode(inks, 2, 2, 100, None, &o).unwrap().iter().map(|b| format!("{b:02x}")).collect::<String>()
    };
    let inverted: Vec<u8> = want.iter().map(|v| 255 - v).collect();
    for (decode, data) in [("[1 0 1 0 1 0 1 0]", jpeg(&want)), ("[0 1 0 1 0 1 0 1]", jpeg(&inverted))] {
        let r = read(&format!(
            "/DeviceCMYK setcolorspace 30 30 scale << /ImageType 1 /Width 2 /Height 2 /BitsPerComponent 8 /Decode {decode} \
             /ImageMatrix [2 0 0 -2 0 2] /DataSource currentfile /ASCIIHexDecode filter /DCTDecode filter >> image\n{data}>"
        ));
        let got = inks(&r).data;
        assert!(got.iter().zip(&want).all(|(a, b)| a.abs_diff(*b) <= 3), "{decode}: {got:?}");
    }
}

/// The bits a fax encoder writes, in order.
#[derive(Clone, Default)]
struct Written(Rc<RefCell<Vec<Bits>>>);

impl BitWriter for Written {
    type Error = std::convert::Infallible;

    fn write(&mut self, bits: Bits) -> Result<(), Self::Error> {
        self.0.borrow_mut().push(bits);
        Ok(())
    }
}

/// A 70 × 7 picture, `true` for black: a white row and a black row (runs past 63 take a make-up
/// code), then runs of several lengths.
fn picture() -> Vec<Vec<bool>> {
    (0..7).map(|y| (0..70).map(|x| if y < 2 { y == 1 } else { (x * (y + 2) / 5 + y) % 3 == 0 }).collect()).collect()
}

/// `row` as one-dimensional runs (modified Huffman), white and black in turn from a white one
/// (ITU-T T.4 §4.1.1).
fn runs(row: &[bool]) -> Vec<Bits> {
    let mut out = vec![];
    let (mut x, mut black) = (0, false);
    while x < row.len() {
        let n = row[x..].iter().take_while(|b| **b == black).count();
        let code = |n: usize| if black { fax::maps::black::encode(n as u16) } else { fax::maps::white::encode(n as u16) }.unwrap();
        if n >= 64 {
            out.push(code(n & !63));
        }
        out.push(code(n % 64));
        (x, black) = (x + n, !black);
    }
    out
}

/// `rows` as CCITT data. With `k` below 0 it is Group 4 (each row coded against the one above,
/// ITU-T T.6), ending with EOFB. Otherwise it is Group 3 (T.4): one-dimensional rows, and with `k`
/// above 0 every `k`th row from the first one-dimensional and the others coded against the row
/// above, each after its tag bit. Group 3 rows follow an EOL when `eol`, and six end the data.
/// With `aligned`, each row (its EOL first) and the six EOLs at the end start at a byte.
fn fax_data(rows: &[Vec<bool>], k: i32, eol: bool, aligned: bool) -> Vec<u8> {
    let written = Written::default();
    let mut encoder = fax::encoder::Encoder::new(written.clone());
    let mut bits: Vec<Bits> = vec![];
    let align = |bits: &mut Vec<Bits>| {
        let used: usize = bits.iter().map(|b| usize::from(b.len)).sum();
        if aligned && !used.is_multiple_of(8) {
            bits.push(Bits { data: 0, len: (8 - used % 8) as u8 });
        }
    };
    for (i, row) in rows.iter().enumerate() {
        let at = written.0.borrow().len();
        encoder.encode_line(row.iter().map(|b| if *b { fax::Color::Black } else { fax::Color::White }), row.len() as u16).unwrap();
        let two_d = written.0.borrow()[at..].to_vec();
        align(&mut bits);
        if k < 0 {
            bits.extend(two_d);
            continue;
        }
        if eol {
            bits.push(fax::maps::EOL);
        }
        let one_d = k == 0 || i % k as usize == 0;
        if k > 0 {
            bits.push(Bits { data: u16::from(one_d), len: 1 });
        }
        bits.extend(if one_d { runs(row) } else { two_d });
    }
    if k < 0 {
        bits.extend([fax::maps::EOL, fax::maps::EOL]);
    } else if eol {
        align(&mut bits);
        let end: &[Bits] = if k > 0 { &[fax::maps::EOL, Bits { data: 1, len: 1 }] } else { &[fax::maps::EOL] };
        bits.extend(end.repeat(6));
    }
    let mut w = fax::VecWriter::new();
    for b in bits {
        w.write(b).unwrap();
    }
    w.finish()
}

/// Bytes as hexadecimal digits.
fn hex(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

/// Images read through `CCITTFaxDecode` draw: Group 4, Group 3 one-dimensional and Group 3 mixed
/// one- and two-dimensional (ITU-T T.4 and T.6), with and without EOLs, with rows starting at a
/// byte, ended by a row count (`EndOfBlock false`) or by the end-of-block code, which overrides
/// `Rows`. `BlackIs1` gives black as 1 bits, so the image comes out inverted unless a Decode array
/// of `[1 0]` reads it.
#[test]
fn ccitt_fax_images_draw() {
    let rows = picture();
    let want: Vec<u8> = rows.concat().iter().map(|black| if *black { 0 } else { 255 }).collect();
    let cases = [
        (-1, false, false, "/K -1 /Columns 70"),
        (-1, false, false, "/K -1 /Columns 70 /Rows 4"),
        (-1, false, false, "/K -1 /Columns 70 /Rows 7 /EndOfBlock false"),
        (-1, false, true, "/K -1 /Columns 70 /EncodedByteAlign true"),
        (0, false, false, "/Columns 70 /Rows 7"),
        (0, true, false, "/Columns 70 /EndOfLine true"),
        (0, true, false, "/Columns 70 /Rows 4 /EndOfLine true"),
        (0, true, true, "/Columns 70 /Rows 7 /EndOfLine true /EncodedByteAlign true"),
        (2, true, false, "/K 2 /Columns 70 /Rows 7 /EndOfLine true"),
        (3, true, false, "/K 3 /Columns 70 /EndOfLine true"),
    ];
    for (k, eol, aligned, params) in cases {
        let data = hex(&fax_data(&rows, k, eol, aligned));
        for (black_is_1, decode, inverted) in [("", "", false), (" /BlackIs1 true", "", true), (" /BlackIs1 true", " /Decode [1 0]", false)] {
            let r = read(&format!(
                "30 20 scale << /ImageType 1 /Width 70 /Height 7 /BitsPerComponent 1{decode} /ImageMatrix [70 0 0 -7 0 7] \
                 /DataSource currentfile /ASCIIHexDecode filter << {params}{black_is_1} >> /CCITTFaxDecode filter >> image\n{data}>"
            ));
            let got: Vec<u8> = pixels(&r, 70, 7).iter().map(|p| if inverted { 255 - p[0] } else { p[0] }).collect();
            assert_eq!(got, want, "{params}{black_is_1}{decode}");
        }
    }
}

/// Binary CCITT data right after the operator that reads it: an image mask paints its black
/// pixels, and the program goes on after the data's end-of-block code (EOFB in Group 4, RTC in
/// Group 3, its EOLs followed by tag bits in two-dimensional data).
#[test]
fn ccitt_fax_data_in_the_program_ends_at_its_end_of_block() {
    let rows = picture();
    for (k, eol, aligned, params) in [
        (-1, false, false, "/K -1 /Columns 70"),
        (0, true, false, "/Columns 70 /EndOfLine true"),
        (2, true, false, "/K 2 /Columns 70 /Rows 7 /EndOfLine true"),
        (3, true, true, "/K 3 /Columns 70 /EndOfLine true /EncodedByteAlign true"),
    ] {
        let mut file = eps("");
        file.truncate(file.len() - "\nshowpage\n%%EOF\n".len());
        file.extend_from_slice(
            format!("gsave 30 20 scale 70 7 false [70 0 0 -7 0 7] currentfile << {params} >> /CCITTFaxDecode filter imagemask\n").as_bytes(),
        );
        file.extend(fax_data(&rows, k, eol, aligned));
        file.extend_from_slice(b"\ngrestore 1 0 0 setrgbcolor 50 50 10 10 rectfill\nshowpage\n%%EOF\n");
        let r = import(&file).unwrap();
        let alpha: Vec<bool> = pixels(&r, 70, 7).iter().map(|p| p[3] == 255).collect();
        assert_eq!(alpha, rows.concat(), "{params}");
        assert!(all(&r.document).iter().any(|n| matches!(n.kind, NodeKind::Path { .. })), "the rectangle after the data is drawn: {params}");
    }
}

/// Damaged CCITT data keeps the rows decoded before the damage, and damage in the first row
/// stops the program. Parameters past the limits are refused.
#[test]
fn damaged_or_oversized_ccitt_fax_data() {
    let rows = picture();
    // Four rows, then an end-of-block code read as a fifth row: the four rows draw.
    let data = hex(&fax_data(&rows[..4], -1, false, false));
    let r = read(&format!(
        "30 20 scale 70 7 1 [70 0 0 -7 0 7] currentfile /ASCIIHexDecode filter << /K -1 /Columns 70 /Rows 7 /EndOfBlock false >> \
         /CCITTFaxDecode filter image\n{data}>"
    ));
    let got: Vec<u8> = pixels(&r, 70, 7).iter().map(|p| p[0]).collect();
    let want: Vec<u8> = rows[..4].concat().iter().map(|black| if *black { 0 } else { 255 }).collect();
    assert_eq!(got[..4 * 70], want);
    for (params, why) in [
        ("/K -1 /Columns 70 /Rows 7", "ioerror in `filter`: CCITTFaxDecode"),
        ("/K -1 /Columns 65535 /Rows 100000 /EndOfBlock false", "decoded data is too large"),
        ("/K -1 /Columns 65536", "rangecheck in `filter`: CCITTFaxDecode"),
        ("/K -1 /Columns 0", "rangecheck in `filter`: CCITTFaxDecode"),
        ("/K -1 /Columns 70 /Rows -1", "rangecheck in `filter`: CCITTFaxDecode"),
    ] {
        let program = format!("0 0 10 10 rectfill 30 20 scale 70 7 1 [70 0 0 -7 0 7] <0000000000> << {params} >> /CCITTFaxDecode filter image");
        let r = import(&eps(&program)).unwrap();
        assert!(r.warnings.iter().any(|w| w.contains(why)), "{params}: {:?}", r.warnings);
        assert!(!all(&r.document).iter().any(|n| matches!(n.kind, NodeKind::Image(_))), "{params}");
    }
}

/// Rows of an odd width at 1, 2, 4, 12 and 16 bits a sample start at a byte (their last byte
/// padded), in one source and in several.
#[test]
fn rows_of_every_depth_start_at_a_byte() {
    // 3 pixels a row, two rows: 0, max, mid / max, 0, 0 (grey), each row padded.
    let cases = [
        (1, "a0 80"),                  // 101(00000) 100(00000)
        (2, "cc c0"),                  // 11 00 11 (00), 11 00 00 (00)
        (4, "f0 80 f0 00"),            // f 0 8 (0), f 0 0 (0)
        (12, "fff0008000 fff0000000"), // fff 000 800 (0), fff 000 000 (0)
        (16, "ffff00008000 ffff00000000"),
    ];
    for (bpc, data) in cases {
        let data = data.replace(' ', "");
        let r = read(&format!("30 20 scale 3 2 {bpc} [3 0 0 -2 0 2] <{data}> image"));
        let px: Vec<u8> = pixels(&r, 3, 2).iter().map(|p| p[0]).collect();
        let want: &[u8] = match bpc {
            1 => &[255, 0, 255, 255, 0, 0],
            2 => &[255, 0, 255, 255, 0, 0],
            4 => &[255, 0, 136, 255, 0, 0],
            12 => &[255, 0, 128, 255, 0, 0],
            _ => &[255, 0, 128, 255, 0, 0],
        };
        assert_eq!(px, want, "{bpc} bits");
    }
    // Several 4-bit sources, odd width: each plane's rows padded on their own.
    let r = read("30 20 scale 3 2 4 [3 0 0 -2 0 2] <f0f000f0> <0f00f000> <00f00ff0> true 3 colorimage");
    let (on, off) = (255, 0);
    let want = [[on, off, off], [off, on, off], [on, off, on], [off, on, off], [off, off, on], [on, off, on]];
    assert_eq!(pixels(&r, 3, 2), want.map(|[r, g, b]| [r, g, b, 255]));
}

/// A PNG-predicted Flate image of an odd width: every row filter (None, Sub, Up, Average, Paeth)
/// undone on its own row.
#[test]
fn predicted_rows_of_an_odd_width() {
    // 3 RGB pixels a row, five rows; each row predicted with its own filter.
    let raw: Vec<[u8; 9]> = (0..5u8).map(|y| std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(y.wrapping_mul(53)))).collect();
    let mut encoded = vec![];
    let mut prev = [0u8; 9];
    for (kind, row) in raw.iter().enumerate() {
        encoded.push(kind as u8);
        for i in 0..9 {
            let a = if i >= 3 { row[i - 3] } else { 0 };
            let b = prev[i];
            let c = if i >= 3 { prev[i - 3] } else { 0 };
            let pred = match kind {
                1 => a,
                2 => b,
                3 => ((u16::from(a) + u16::from(b)) / 2) as u8,
                4 => {
                    let p = i16::from(a) + i16::from(b) - i16::from(c);
                    let (pa, pb, pc) = ((p - i16::from(a)).abs(), (p - i16::from(b)).abs(), (p - i16::from(c)).abs());
                    if pa <= pb && pa <= pc {
                        a
                    } else if pb <= pc {
                        b
                    } else {
                        c
                    }
                }
                _ => 0,
            };
            encoded.push(row[i].wrapping_sub(pred));
        }
        prev = *row;
    }
    let r = read(&format!(
        "/DeviceRGB setcolorspace 30 50 scale << /ImageType 1 /Width 3 /Height 5 /BitsPerComponent 8 /Decode [0 1 0 1 0 1] /ImageMatrix [3 0 0 -5 0 5] \
         /DataSource currentfile /ASCII85Decode filter << /Predictor 15 /Colors 3 /Columns 3 >> /FlateDecode filter >> image\n{}",
        crate::ps::ascii85(&crate::ps::deflate(&encoded))
    ));
    let want: Vec<[u8; 4]> = raw.iter().flat_map(|row| (0..3).map(move |x| [row[3 * x], row[3 * x + 1], row[3 * x + 2], 255])).collect();
    assert_eq!(pixels(&r, 3, 5), want);
}

/// Rows of `bpc`-bit samples packed from the high bit (16-bit ones big-endian), each row padded to
/// a whole byte.
fn packed(rows: &[Vec<u32>], bpc: usize) -> Vec<u8> {
    let mut out = vec![];
    for row in rows {
        let mut bytes = vec![0u8; (row.len() * bpc).div_ceil(8)];
        for (k, v) in row.iter().enumerate() {
            if bpc == 16 {
                bytes[2 * k..2 * k + 2].copy_from_slice(&(*v as u16).to_be_bytes());
            } else {
                let bit = k * bpc;
                bytes[bit / 8] |= (*v as u8) << (8 - bpc - bit % 8);
            }
        }
        out.extend(bytes);
    }
    out
}

/// A TIFF-predicted Flate stream (Predictor 2, horizontal differencing: ISO 32000-1 §7.4.4.4) at
/// 1, 2, 4, 8 and 16 bits a component, gray and RGB, five pixels a row: each component is the
/// difference from the same component of the pixel to its left, within its bit width, and each
/// row starts again. The filter gives back the samples before differencing, read here as 8-bit
/// gray pixels one byte each.
#[test]
fn tiff_predicted_rows_at_every_depth() {
    let columns = 5;
    let mut wrong = vec![];
    for bpc in [1, 2, 4, 8, 16] {
        for colors in [1, 3] {
            let max = (1u32 << bpc) - 1;
            let rows: Vec<Vec<u32>> =
                (0..3u32).map(|y| (0..(columns * colors) as u32).map(|i| (i * i * 40_503 + i * 7 + y * 12_345 + 1) & max).collect()).collect();
            let differenced: Vec<Vec<u32>> = rows
                .iter()
                .map(|row| (0..row.len()).map(|i| if i < colors { row[i] } else { row[i].wrapping_sub(row[i - colors]) & max }).collect())
                .collect();
            let (want, data) = (packed(&rows, bpc), packed(&differenced, bpc));
            let width = want.len() / 3;
            let r = read(&format!(
                "30 30 scale << /ImageType 1 /Width {width} /Height 3 /BitsPerComponent 8 /Decode [0 1] /ImageMatrix [{width} 0 0 -3 0 3] \
                 /DataSource currentfile /ASCII85Decode filter << /Predictor 2 /Colors {colors} /BitsPerComponent {bpc} /Columns {columns} >> \
                 /FlateDecode filter >> image\n{}",
                crate::ps::ascii85(&crate::ps::deflate(&data))
            ));
            let got: Vec<u8> = pixels(&r, width as u32, 3).iter().map(|p| p[0]).collect();
            if got != want {
                wrong.push(format!("{bpc} bits, {colors} colors: {got:?}, not {want:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}
