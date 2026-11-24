//! The options of TIFF (colour model, byte order, LZW, profile), BMP (colour model, layout, depth,
//! palette, RLE, row order) and Targa (depth), after the raster export rows they share with PNG
//! Options ([`super::png_options`]): the `tiffOptions`, `bmpOptions` and `tgaOptions` dialogs.

use craft::ui::patterns::form_dialogs::{DialogFields, Param};
use serde_json::{Map, Value, json};
use vectorcraft_render::encode::bmp::{self, BmpOptions};
use vectorcraft_render::encode::jpeg::ColorModel;
use vectorcraft_render::encode::quantize::{Dither, PaletteOptions, Reduction};
use vectorcraft_render::encode::tga::{self, TgaOptions};
use vectorcraft_render::encode::tiff::{ByteOrder, TiffOptions};

use super::png_options::{check, choice};

/// BMP colour models and layouts: param values and labels.
const BMP_MODELS: [&str; 2] = ["rgb", "gray"];
const BMP_MODEL_LABELS: [&str; 2] = ["RGB", "Grayscale"];
const LAYOUTS: [&str; 2] = ["windows", "os2"];
const LAYOUT_LABELS: [&str; 2] = ["Windows", "OS/2"];
/// Labels of [`bmp::DEPTHS`] and [`tga::DEPTHS`].
const BMP_DEPTH_LABELS: [&str; 6] = ["1 bit", "4 bit", "8 bit", "16 bit", "24 bit", "32 bit"];
const TGA_DEPTH_LABELS: [&str; 3] = ["16 bits/pixel", "24 bits/pixel", "32 bits/pixel"];

/// The options format `id` starts with (`cmyk`: a CMYK document exports CMYK TIFFs).
pub(super) fn defaults(id: &str, cmyk: bool, o: &mut Map<String, Value>) {
    match id {
        "tiff" => {
            let t = TiffOptions::default();
            let model = if cmyk { ColorModel::Cmyk } else { t.color_model };
            o.extend([
                ("colorModel".into(), json!(model.id())),
                ("byteOrder".into(), json!(t.byte_order.id())),
                ("lzw".into(), json!(t.lzw)),
                ("embedIcc".into(), json!(t.embed_icc)),
            ]);
        }
        "bmp" => {
            let (b, p) = (BmpOptions::default(), PaletteOptions::default());
            o.extend([
                ("colorModel".into(), json!(BMP_MODELS[usize::from(b.gray)])),
                ("fileFormat".into(), json!(LAYOUTS[usize::from(b.os2)])),
                ("depth".into(), json!(b.depth.to_string())),
                ("rle".into(), json!(b.rle)),
                ("flipRows".into(), json!(b.top_down)),
                ("reduction".into(), json!(p.reduction.id())),
                ("dither".into(), json!(p.dither.id())),
            ]);
        }
        "tga" => {
            o.insert("depth".into(), json!(TgaOptions::default().depth.to_string()));
        }
        _ => {}
    }
}

/// The bits per pixel the dialog holds: an option id (text) or a number.
fn depth(f: &DialogFields) -> f64 {
    f.number("depth").or_else(|| f.text("depth").and_then(|t| t.parse().ok())).unwrap_or(24.0)
}

/// Does format `id` keep transparency with the dialog's options (else it is flattened on white)?
pub(super) fn keeps_alpha(id: &str, f: &DialogFields) -> bool {
    match id {
        "jpg" => false,
        "tiff" => ColorModel::from_id(f.text("colorModel").unwrap_or_default()).unwrap_or_default() == ColorModel::Rgb,
        "bmp" => depth(f) == 32.0,
        "tga" => depth(f) != 24.0,
        // A flat PSD is flattened; its layers keep transparency.
        "psd" => f.text("layers") == Some("true"),
        _ => true,
    }
}

/// The rows of format `id`'s own options; BMP's depend on `fields`.
pub(super) fn params(id: &str, fields: &DialogFields) -> Vec<Param> {
    match id {
        "tiff" => vec![
            choice("colorModel", tl!("Color Model:"), &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label)),
            choice("byteOrder", tl!("Byte Order:"), &ByteOrder::ALL.map(ByteOrder::id), &ByteOrder::ALL.map(ByteOrder::label)),
            check("lzw", tl!("LZW Compression"), true),
            check("embedIcc", tl!("Embed ICC Profile"), true),
        ],
        "bmp" => bmp_params(fields),
        "tga" => vec![choice("depth", tl!("Depth:"), &tga::DEPTHS.map(|d| d.to_string()), &TGA_DEPTH_LABELS)],
        _ => Vec::new(),
    }
}

fn bmp_params(fields: &DialogFields) -> Vec<Param> {
    let os2 = fields.text("fileFormat") == Some(LAYOUTS[1]);
    let depths = bmp::DEPTHS.map(|d| d.to_string());
    let mut rows = vec![
        choice("colorModel", tl!("Color Model:"), &BMP_MODELS, &BMP_MODEL_LABELS),
        choice("fileFormat", tl!("File Format:"), &LAYOUTS, &LAYOUT_LABELS),
        choice("depth", tl!("Depth:"), &depths, &BMP_DEPTH_LABELS)
            .disabled_options(bmp::DEPTHS.iter().filter(|d| os2 && !bmp::OS2_DEPTHS.contains(d)).map(|d| d.to_string())),
    ];
    // The palette of a colour image at 4 or 8 bits (1 bit is black and white; greys are fixed).
    if matches!(depth(fields) as u8, 4 | 8) && fields.text("colorModel") != Some(BMP_MODELS[1]) {
        rows.push(choice("reduction", tl!("Color Reduction:"), &Reduction::ALL.map(Reduction::id), &Reduction::ALL.map(Reduction::label)));
        rows.push(choice("dither", tl!("Dither:"), &Dither::ALL.map(Dither::id), &Dither::ALL.map(Dither::label)));
    }
    // OS/2 has neither compression nor flipped rows; RLE needs 4 or 8 bits, and flipped rows need it off.
    let rle = check("rle", tl!("Compress (RLE)"), false);
    let flip = check("flipRows", tl!("Flip Row Order"), false);
    rows.push(if os2 { rle.enabled_when("fileFormat", LAYOUTS[0]) } else { rle.enabled_when_in("depth", ["4", "8"]) });
    rows.push(if os2 { flip.enabled_when("fileFormat", LAYOUTS[0]) } else { flip.enabled_when("rle", false) });
    rows
}

/// Keep the BMP options writable together: OS/2 has no 16 or 32 bits, compression or flipped
/// rows; only 4- and 8-bit images are compressed, and those store their rows bottom-up.
pub(super) fn normalize_bmp(f: &mut DialogFields) {
    let os2 = f.text("fileFormat") == Some(LAYOUTS[1]);
    let mut bits = depth(f) as u8;
    if os2 && !bmp::OS2_DEPTHS.contains(&bits) {
        bits = 24;
        f.set("depth", bits.to_string());
    }
    let rle = f.bool("rle").unwrap_or(false) && !os2 && matches!(bits, 4 | 8);
    let flip = f.bool("flipRows").unwrap_or(false) && !os2 && !rle;
    for (key, on) in [("rle", rle), ("flipRows", flip)] {
        if f.bool(key).unwrap_or(false) != on {
            f.set(key, on);
        }
    }
}
