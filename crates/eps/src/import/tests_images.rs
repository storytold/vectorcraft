//! Image data read as the PostScript Language Reference (3rd ed.) defines it: several data sources
//! read in turn (§4.10 "Images", `image` and `colorimage` in chapter 8), rows padded to a whole
//! byte at every bit depth and odd widths (§4.10), and Flate predictors undone row by row (§3.13,
//! `FlateDecode` parameters) (#498). CMYK images keep their ink amounts, as the Decode array
//! reads their samples, and their transparent pixels become an opacity mask.

use std::sync::Arc;

use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{ImageBlob, Node, NodeKind};

use super::tests::{all, read};
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
