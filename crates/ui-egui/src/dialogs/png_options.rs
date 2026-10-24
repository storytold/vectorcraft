//! The raster export options: PNG Options (resolution, background, anti-aliasing, interlaced),
//! JPEG Options (no transparency; colour model, quality 0–10, method and scans, profile, image
//! map), WebP Options, and PNG-8 and GIF Options (the palette: colour reduction, colours, dither,
//! transparency and matte), TIFF, BMP and Targa Options ([`super::tiff_bmp_tga`]) and PSD Options
//! ([`super::psd_options`]). Fields
//! are `document.export` params (format, path, artboard choice…); `__`-prefixed ones are the
//! dialog's.

use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format};
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::jpeg::{self, ColorModel, Method};
use vectorcraft_render::encode::quantize::{Dither, PaletteOptions, Reduction};

use super::{DialogSpec, form, psd_options, tiff_bmp_tga};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, io, widgets};

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, ok: Some("Export"), min_width: 360.0, ..DialogSpec::FORM };

/// Resolution presets (pixels per inch).
pub(super) const RESOLUTIONS: [f64; 3] = [72.0, 150.0, 300.0];
/// Their labels, then `Other` (any resolution).
pub(super) const RESOLUTION_LABELS: [&str; 4] = ["Screen (72 ppi)", "Medium (150 ppi)", "High (300 ppi)", "Other"];
/// Background param values.
const BACKGROUNDS: [&str; 3] = ["transparent", "white", "black"];
/// Their labels, then `Other` (a colour).
const BACKGROUND_LABELS: [&str; 4] = ["Transparent", "White", "Black", "Other"];
/// JPEG image map param values and their labels.
const IMAGE_MAPS: [&str; 3] = ["none", "client", "server"];
const IMAGE_MAP_LABELS: [&str; 3] = ["None", "Client-side (.html)", "Server-side (.map)"];
/// Palette sizes offered (any 2–256 can be typed).
const COLOR_COUNTS: [&str; 8] = ["2", "4", "8", "16", "32", "64", "128", "256"];
/// Matte param values and their labels (`Other`: a colour).
const MATTES: [&str; 3] = ["none", "white", "black"];
const MATTE_LABELS: [&str; 4] = ["None", "White", "Black", "Other"];
/// JPEG quality on the dialog's 0–10 scale: the band each step falls in.
const QUALITY_BANDS: [&str; 11] = ["Low", "Low", "Low", "Medium", "Medium", "Medium", "High", "High", "Maximum", "Maximum", "Maximum"];

/// The dialog kind of a raster format (`pngOptions`, `jpgOptions`, `webpOptions`, `gifOptions`,
/// `png8Options`).
fn kind(f: &Format) -> String {
    format!("{}Options", f.id)
}

fn format_of(d: &Dialog) -> Option<&'static Format> {
    fileio::format(d.kind.strip_suffix("Options")?)
}

/// Open the options of raster format `f` for an export whose params (format, path, artboard
/// choice) are `params`.
pub fn open(app: &mut VectorcraftApp, f: &Format, mut params: Value) {
    let size = app.session.active().and_then(|st| export_size(&st.doc, &params));
    if let Some(o) = params.as_object_mut() {
        o.insert("ppi".into(), json!(72));
        o.extend(defaults(app, f));
        if let Some((w, h)) = size {
            o.insert("__size".into(), json!([w, h]));
        }
    }
    app.ui.dialog = Some(Dialog::new(&kind(f), params));
}

/// The options raster format `f` starts with (after the resolution): the document's background
/// (New Document → Background Contents; JPEG has no alpha), anti-aliasing and the format's own.
pub(super) fn defaults(app: &VectorcraftApp, f: &Format) -> Map<String, Value> {
    let white = f.id == "jpg" || app.session.active().is_some_and(|st| st.doc.setup.background == vectorcraft_doc::Background::White);
    let mut o = Map::new();
    o.insert("background".into(), json!(if white { "white" } else { "transparent" }));
    o.insert("antiAlias".into(), json!(AntiAlias::default().id()));
    // A CMYK document exports CMYK JPEGs, TIFFs and PSDs by default.
    let cmyk = app.session.active().is_some_and(|st| st.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk);
    match f.id {
        "png" => {
            o.insert("interlaced".into(), json!(false));
        }
        "jpg" => {
            let model = if cmyk { ColorModel::Cmyk } else { ColorModel::Rgb };
            let defaults = jpeg::JpegOptions::default();
            o.extend([
                ("quality".into(), json!(90)),
                ("colorModel".into(), json!(model.id())),
                ("method".into(), json!(defaults.method.id())),
                ("scans".into(), json!(defaults.scans)),
                ("embedIcc".into(), json!(defaults.embed_icc)),
                ("imageMap".into(), json!(IMAGE_MAPS[0])),
            ]);
        }
        "gif" | "png8" => {
            let p = PaletteOptions::default();
            o.extend([
                ("reduction".into(), json!(p.reduction.id())),
                ("colors".into(), json!(p.colors)),
                ("dither".into(), json!(p.dither.id())),
                ("ditherAmount".into(), json!(p.dither_amount)),
                ("transparency".into(), json!(p.transparency)),
                ("matte".into(), json!(MATTES[1])),
                ("interlaced".into(), json!(false)),
            ]);
        }
        "psd" => psd_options::defaults(cmyk, &mut o),
        id => tiff_bmp_tga::defaults(id, cmyk, &mut o),
    }
    o
}

/// The size in points of the one region an export covers (`None` when it writes several
/// artboards): the chosen artboard, or the bounds of the visible art.
fn export_size(doc: &vectorcraft_doc::Document, p: &Value) -> Option<(f64, f64)> {
    let pick = ArtboardPick::deserialize(p).ok()?;
    let n = doc.artboards.len();
    let r = match p["useArtboards"].as_bool() {
        Some(false) => vectorcraft_render::encode::art_bounds(doc)?,
        Some(true) => match pick.resolve(n).ok()?.as_deref() {
            Some([i]) => doc.artboards.get(*i)?.rect,
            None if n == 1 => doc.artboards[0].rect,
            _ => return None,
        },
        None => doc.artboards.get(pick.one(n).ok()?)?.rect,
    };
    Some((r.width(), r.height()))
}

fn heading(d: &Dialog) -> String {
    crate::i18n::fmt(tl!("{format} Options"), &[("format", format_of(d).map_or(tl!("Export"), |f| f.label))])
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let id = format_of(d).map_or("png", |f| f.id);
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(tl!(text)).color(t.text_dim));
    egui::Grid::new("raster-options").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        label(ui, tl!("Resolution:"));
        let ppi = d.f64("ppi", 72.0);
        let preset = RESOLUTIONS.iter().position(|p| *p == ppi).filter(|_| !d.bool("__otherPpi"));
        ui.horizontal(|ui| {
            if let Some(i) = widgets::dropdown(ui, "ro-ppi", RESOLUTION_LABELS[preset.unwrap_or(3)], &RESOLUTION_LABELS, 150.0) {
                if let Some(p) = RESOLUTIONS.get(i) {
                    d.fields.insert("ppi".into(), json!(p));
                }
                d.fields.insert("__otherPpi".into(), json!(i == 3));
            }
            if preset.is_none()
                && let Some(v) = widgets::range_field(ui, "ro-ppi-other", ppi, 1.0..=4608.0, " ppi", 0, 80.0)
            {
                d.fields.insert("ppi".into(), json!(v));
            }
        });
        ui.end_row();

        option_rows(ui, d, id, false);

        if let Some([w, h]) =
            d.fields.get("__size").and_then(Value::as_array).map(|a| [0, 1].map(|i| a.get(i).and_then(Value::as_f64).unwrap_or(0.0)))
        {
            label(ui, tl!("Size:"));
            let (pw, ph) = vectorcraft_render::region_pixels(vectorcraft_geom::Rect::new(0.0, 0.0, w, h), d.f64("ppi", 72.0) / 72.0);
            ui.label(egui::RichText::new(format!("{pw} × {ph} px")).color(t.text));
            ui.end_row();
        }
    });
    false
}

/// The grid rows of raster format `id`'s options after the resolution: background, anti-aliasing
/// and the format's own. `screens` (Export for Screens' Format Settings) leaves out what its rows
/// choose (JPEG quality) and what writes other files (image maps).
pub(super) fn option_rows(ui: &mut egui::Ui, d: &mut Dialog, id: &str, screens: bool) {
    let t = Tokens::get(ui.ctx());
    let label = |ui: &mut egui::Ui, text: &str| ui.label(egui::RichText::new(tl!(text)).color(t.text_dim));
    label(ui, tl!("Background Color:"));
    let bg = d.str("background");
    let choice = BACKGROUNDS.iter().position(|v| bg.eq_ignore_ascii_case(v)).unwrap_or(3);
    // JPEG has no transparency, nor have some TIFF, BMP and Targa options: their list starts at
    // White, which is what transparent becomes.
    let first = usize::from(!tiff_bmp_tga::keeps_alpha(id, d));
    ui.horizontal(|ui| {
        if let Some(i) = widgets::dropdown(ui, "ro-bg", BACKGROUND_LABELS[choice.max(first)], &BACKGROUND_LABELS[first..], 150.0) {
            let value = BACKGROUNDS.get(i + first).copied().unwrap_or("#808080");
            d.fields.insert("background".into(), json!(value));
        }
        if choice == 3 {
            color_button(ui, d, "background");
        }
    });
    ui.end_row();

    label(ui, tl!("Anti-aliasing:"));
    let aa = AntiAlias::from_id(&d.str("antiAlias")).unwrap_or_default();
    if let Some(i) = widgets::dropdown(ui, "ro-aa", aa.label(), &AntiAlias::ALL.map(AntiAlias::label), 150.0) {
        d.fields.insert("antiAlias".into(), json!(AntiAlias::ALL[i].id()));
    }
    ui.end_row();

    match id {
        "png" => {
            ui.label("");
            form::check(ui, d, "interlaced", tl!("Interlaced"));
            ui.end_row();
        }
        "jpg" => jpeg_rows(ui, d, &label, screens),
        "gif" | "png8" => palette_rows(ui, d, &label),
        "webp" => {
            ui.label("");
            label(ui, tl!("Lossless (lossy WebP isn't available yet)"));
            ui.end_row();
        }
        "psd" => psd_options::rows(ui, d, &label),
        id => tiff_bmp_tga::rows(ui, d, id, &label),
    }
}

/// The JPEG Options rows: colour model, quality, method and scans, profile and image map (not
/// quality and image map for Export for Screens).
fn jpeg_rows(ui: &mut egui::Ui, d: &mut Dialog, label: &dyn Fn(&mut egui::Ui, &str) -> egui::Response, screens: bool) {
    label(ui, tl!("Color Model:"));
    choice(ui, d, "colorModel", &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label));
    ui.end_row();

    if !screens {
        quality_row(ui, d, label);
    }
    method_row(ui, d, label);
    if !screens {
        label(ui, tl!("Image Map:"));
        choice(ui, d, "imageMap", &IMAGE_MAPS, &IMAGE_MAP_LABELS);
        ui.end_row();
    }

    ui.label("");
    form::check(ui, d, "embedIcc", tl!("Embed ICC Profile"));
    ui.end_row();
}

/// JPEG quality on the dialog's 0–10 scale (stored ×10).
fn quality_row(ui: &mut egui::Ui, d: &mut Dialog, label: &dyn Fn(&mut egui::Ui, &str) -> egui::Response) {
    label(ui, tl!("Quality:"));
    let mut q = (d.f64("quality", 90.0) / 10.0).round().clamp(0.0, 10.0) as u8;
    ui.horizontal(|ui| {
        if widgets::slider(ui, egui::Slider::new(&mut q, 0..=10)).changed() {
            d.fields.insert("quality".into(), json!(u32::from(q) * 10));
        }
        label(ui, QUALITY_BANDS[q as usize]);
    });
    ui.end_row();
}

/// The JPEG method, and its scans when progressive.
fn method_row(ui: &mut egui::Ui, d: &mut Dialog, label: &dyn Fn(&mut egui::Ui, &str) -> egui::Response) {
    label(ui, tl!("Method:"));
    ui.horizontal(|ui| {
        choice(ui, d, "method", &Method::ALL.map(Method::id), &Method::ALL.map(Method::label));
        if Method::from_id(&d.str("method")) == Some(Method::Progressive) {
            label(ui, tl!("Scans:"));
            let range = f64::from(*jpeg::SCANS.start())..=f64::from(*jpeg::SCANS.end());
            if let Some(scans) = widgets::range_field(ui, "scans", d.f64("scans", 3.0), range, "", 0, 44.0) {
                d.fields.insert("scans".into(), json!(scans as u8));
            }
        }
    });
    ui.end_row();
}

/// The PNG-8 and GIF Options rows: the palette (reduction, colours, dither and amount),
/// transparency and matte, interlacing.
fn palette_rows(ui: &mut egui::Ui, d: &mut Dialog, label: &dyn Fn(&mut egui::Ui, &str) -> egui::Response) {
    label(ui, tl!("Color Reduction:"));
    choice(ui, d, "reduction", &Reduction::ALL.map(Reduction::id), &Reduction::ALL.map(Reduction::label));
    ui.end_row();

    label(ui, tl!("Colors:"));
    let mut n = d.f64("colors", 256.0).round().clamp(2.0, 256.0) as u16;
    ui.horizontal(|ui| {
        if let Some(i) = widgets::dropdown(ui, "ro-colors", &n.to_string(), &COLOR_COUNTS, 70.0) {
            n = COLOR_COUNTS.get(i).and_then(|c| c.parse().ok()).unwrap_or(n);
            d.fields.insert("colors".into(), json!(n));
        }
        if let Some(n) = widgets::range_field(ui, "colors", f64::from(n), 2.0..=256.0, "", 0, 52.0) {
            d.fields.insert("colors".into(), json!(n as u16));
        }
    });
    ui.end_row();

    label(ui, tl!("Dither:"));
    ui.horizontal(|ui| {
        choice(ui, d, "dither", &Dither::ALL.map(Dither::id), &Dither::ALL.map(Dither::label));
        if Dither::from_id(&d.str("dither")).is_some_and(|x| x != Dither::None)
            && let Some(amount) = widgets::range_field(ui, "ditherAmount", d.f64("ditherAmount", 100.0), 0.0..=100.0, "%", 0, 52.0)
        {
            d.fields.insert("ditherAmount".into(), json!(amount as u8));
        }
    });
    ui.end_row();

    label(ui, tl!("Matte:"));
    let matte = d.str("matte");
    let at = MATTES.iter().position(|v| matte.eq_ignore_ascii_case(v)).unwrap_or(3);
    ui.horizontal(|ui| {
        if let Some(i) = widgets::dropdown(ui, "ro-matte", MATTE_LABELS[at], &MATTE_LABELS, 150.0) {
            d.fields.insert("matte".into(), json!(MATTES.get(i).copied().unwrap_or("#808080")));
        }
        if at == 3 {
            color_button(ui, d, "matte");
        }
    });
    ui.end_row();

    ui.label("");
    ui.horizontal(|ui| {
        form::check(ui, d, "transparency", tl!("Transparency"));
        form::check(ui, d, "interlaced", tl!("Interlaced"));
    });
    ui.end_row();
}

/// A colour button bound to `d.fields[key]` (`"#rrggbb"`; grey when unset).
pub(super) fn color_button(ui: &mut egui::Ui, d: &mut Dialog, key: &str) {
    let c = vectorcraft_color::Color::from_hex(&d.str(key)).map_or([128, 128, 128, 255], |c| c.to_rgba8(1.0));
    let mut rgb = [c[0], c[1], c[2]];
    if ui.color_edit_button_srgb(&mut rgb).changed() {
        d.fields.insert(key.into(), json!(format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])));
    }
}

/// A dropdown bound to `d.fields[key]`: one of `ids`, shown by its label (the first when unknown).
pub(super) fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, ids: &[&str], labels: &[&str]) {
    let cur = d.str(key);
    let at = ids.iter().position(|v| v.eq_ignore_ascii_case(&cur)).unwrap_or(0);
    if let Some(i) = widgets::dropdown(ui, ("ro", key), labels.get(at).copied().unwrap_or_default(), labels, 150.0)
        && let Some(v) = ids.get(i)
    {
        d.fields.insert(key.into(), json!(v));
    }
}

/// Write the file(s) with the chosen options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let (format, path) = (d.str("format"), d.str("path"));
    app.ui.dialog = None;
    io::export(app, Some(&format), Some(path), &form::params(d))
}
