//! The raster export options: PNG Options (resolution, background, anti-aliasing, interlaced),
//! JPEG Options (no transparency; colour model, quality 0–10, method and scans, profile, image
//! map), WebP Options (lossless, or lossy at a quality 0–100), and PNG-8 and GIF Options (the palette: colour reduction, colours, dither,
//! transparency and its dither, matte), TIFF, BMP and Targa Options ([`super::tiff_bmp_tga`]) and PSD Options
//! ([`super::psd_options`]). Fields
//! are `document.export` params (format, path, artboard choice…); `__`-prefixed ones are the
//! dialog's.

use craft::theme::{Rgba, Srgba};
use craft::ui::choice::Swatch;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::SliderField;
use craft::ui::layout_components::Row;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::UnitKind;
use craft::ui::text::NumberField;
use craft::ui::value_drag::SliderScale;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::fileio::{self, ArtboardPick, Format};
use vectorcraft_render::AntiAlias;
use vectorcraft_render::encode::jpeg::{self, ColorModel, Method};
use vectorcraft_render::encode::quantize::{Dither, PaletteOptions, Reduction};

use super::export_host::{self, After, Export, Open};
use super::form::{fields_of, json_of};
use super::{psd_options, tiff_bmp_tga};
use crate::{VectorcraftApp, io};

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
/// Matte param values and their labels (`Other`: a colour).
const MATTES: [&str; 3] = ["none", "white", "black"];
const MATTE_LABELS: [&str; 4] = ["None", "White", "Black", "Other"];
/// The Transparency Dither labels, in [`Dither::ALL`] order.
const TRANSPARENCY_DITHER_LABELS: [&str; 4] = ["No Transparency Dither", "Diffusion", "Pattern", "Noise"];
/// JPEG quality on the dialog's 0–10 scale: the band each step falls in.
const QUALITY_BANDS: [&str; 11] = ["Low", "Low", "Low", "Medium", "Medium", "Medium", "High", "High", "Maximum", "Maximum", "Maximum"];
/// WebP compression labels: `lossless` true, then false.
const WEBP_COMPRESSION_LABELS: [&str; 2] = ["Lossless", "Lossy"];
/// The lossy WebP quality a new export starts with (0–100).
const WEBP_QUALITY: u8 = 90;
/// The colour a colour choice's `Other` starts from.
const OTHER_COLOR: &str = "#808080";
/// Params the engine reads as whole numbers; a row may hold them as floats or as option ids.
const INTEGER_KEYS: [&str; 5] = ["quality", "scans", "colors", "ditherAmount", "depth"];

/// The dialog kind of a raster format (`pngOptions`, `jpgOptions`, `webpOptions`, `gifOptions`,
/// `png8Options`).
fn kind(f: &Format) -> String {
    format!("{}Options", f.id)
}

/// Open the options of raster format `f` for an export whose params (format, path, artboard
/// choice) are `params`.
pub fn open(app: &mut VectorcraftApp, f: &Format, params: Value) {
    open_with(app, f, params, false);
}

/// [`open`] for Export Selection when `selection`: the size shown is the selection's crop.
pub(crate) fn open_with(app: &mut VectorcraftApp, f: &Format, mut params: Value, selection: bool) {
    let size = app.session.active().and_then(|st| match selection {
        true => vectorcraft_engine::cmd::fileio::selection_doc(st, "document.exportSelection", false).ok().map(|(_, r)| (r.width(), r.height())),
        false => export_size(&st.doc, &params),
    });
    if let Some(o) = params.as_object_mut() {
        let mut initial = defaults(app, f);
        initial.insert("ppi".into(), json!(72));
        for (key, value) in initial {
            if o.get(&key).is_none_or(Value::is_null) {
                o.insert(key, value);
            }
        }
        if let Some((w, h)) = size {
            o.insert("__size".into(), json!([w, h]));
        }
    }
    let Some(ctx) = app.ui_ctx.clone() else { return };
    let size = size.map(|(w, h)| [w, h]);
    let (id, label, key) = (f.id, f.label, kind(f));
    let map = params.as_object().cloned().unwrap_or_default();
    let initial = fields_of(&raster_rows(id, false, size, &DialogFields::new()), &map);
    let dialog = HostedParamDialog::new(&ctx, key.clone(), initial, move |fields| {
        normalize_colors(fields);
        if id == "bmp" {
            tiff_bmp_tga::normalize_bmp(fields);
        }
        ParamDialog::new(key.clone(), heading(label), raster_rows(id, false, size, fields)).values(Bind::bound(fields)).ok_label(tl!("Export").into())
    });
    export_host::open(app, Export::Raster { format: id }, selection, dialog);
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
        "webp" => {
            o.extend([("lossless".into(), json!(true)), ("quality".into(), json!(WEBP_QUALITY))]);
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
                ("transparencyDither".into(), json!(p.transparency_dither.id())),
                ("transparencyDitherAmount".into(), json!(p.transparency_dither_amount)),
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
            None if n == 1 => doc.artboards.first()?.rect,
            _ => return None,
        },
        None => doc.artboards.get(pick.one(n).ok()?)?.rect,
    };
    Some((r.width(), r.height()))
}

fn heading(label: &str) -> String {
    crate::i18n::fmt(tl!("{format} Options"), &[("format", label)])
}

/// The rows of raster format `id`'s options: resolution, the size it gives, background,
/// anti-aliasing and the format's own. `screens` (Export for Screens' Format Settings) leaves out
/// what its rows choose (JPEG quality) and what writes other files (image maps). `size` is the
/// region's size in points, when one region is exported.
pub(crate) fn raster_params(id: &str, screens: bool, size: Option<[f64; 2]>) -> Vec<Param> {
    raster_rows(id, screens, size, &DialogFields::new())
}

/// [`raster_params`] for a dialog whose rows depend on its values (BMP's palette rows).
fn raster_rows(id: &str, screens: bool, size: Option<[f64; 2]>, fields: &DialogFields) -> Vec<Param> {
    let mut rows = vec![resolution_row()];
    if let Some([w, h]) = size {
        rows.push(size_row(w, h));
    }
    // JPEG has no transparency, nor have some TIFF, BMP and Targa options: their list starts at
    // White, which is what transparent becomes.
    let format = id.to_string();
    rows.push(color_choice("background", tl!("Background Color:"), &BACKGROUNDS, &BACKGROUND_LABELS, move |f| {
        usize::from(!tiff_bmp_tga::keeps_alpha(&format, f))
    }));
    rows.push(choice("antiAlias", tl!("Anti-aliasing:"), &AntiAlias::ALL.map(AntiAlias::id), &AntiAlias::ALL.map(AntiAlias::label)));
    match id {
        "png" => rows.push(check("interlaced", tl!("Interlaced"), false)),
        "jpg" => rows.extend(jpeg_rows(screens)),
        "gif" | "png8" => rows.extend(palette_rows()),
        "webp" => rows.extend(webp_rows()),
        "psd" => rows.extend(psd_options::params()),
        id => rows.extend(tiff_bmp_tga::params(id, fields)),
    }
    rows
}

/// A dropdown row bound to `key`: one of `ids`, shown by its label.
pub(super) fn choice(key: &str, label: &'static str, ids: &[impl AsRef<str>], labels: &[&'static str]) -> Param {
    let options = ids.iter().zip(labels).map(|(id, l)| (id.as_ref().to_string(), tl!(*l).into())).collect();
    Param::new(key, tl!(label), ParamKind::choice(options, ids.first().map(|id| id.as_ref().to_string())))
}

/// A check box row bound to `key`.
pub(super) fn check(key: &str, label: &'static str, default: bool) -> Param {
    Param::new(key, tl!(label), ParamKind::bool(default))
}

/// Resolution: a preset, or Other with a field of its own (`__otherPpi` marks Other, so a typed
/// 72 stays Other).
fn resolution_row() -> Param {
    Param::custom("ppi", tl!("Resolution:"), |ui, fields, _| {
        let ppi = fields.number("ppi").unwrap_or(72.0);
        let preset = RESOLUTIONS.iter().position(|p| *p == ppi).filter(|_| fields.bool("__otherPpi") != Some(true));
        let other = RESOLUTION_LABELS.len() - 1;
        let entries: Vec<Entry<usize>> = RESOLUTION_LABELS.iter().enumerate().map(|(i, label)| Entry::new(i, tl!(*label))).collect();
        let (mut picked, mut typed) = (None, None);
        let shown = Row::new().show(ui, |ui, _| {
            picked = Select::new(tl!("Resolution:"), Bind::given(preset.unwrap_or(other)), &entries).show(ui).value().copied();
            if preset.is_none() {
                let field = NumberField::new(tl!("Resolution:"), Bind::given(Some(ppi)))
                    .range(1.0..=4608.0)
                    .step(1.0)
                    .unit_kind(UnitKind::PixelsPerInch)
                    .show(ui);
                typed = field.edit.value().copied().flatten();
            }
        });
        if let Some(i) = picked {
            if let Some(p) = RESOLUTIONS.get(i) {
                fields.set("ppi", *p);
            }
            fields.set("__otherPpi", i == other);
        }
        if let Some(v) = typed {
            fields.set("ppi", v);
        }
        shown.output
    })
    .companions(["__otherPpi"])
    .opens_with(72.0)
}

/// The pixel size the region of `w` × `h` points gives at the current resolution.
fn size_row(w: f64, h: f64) -> Param {
    Param::custom("__sizeText", tl!("Size:"), move |ui, fields, _| {
        let (pw, ph) = vectorcraft_render::region_pixels(vectorcraft_geom::Rect::new(0.0, 0.0, w, h), fields.number("ppi").unwrap_or(72.0) / 72.0);
        Text::new(format!("{pw} × {ph} px")).show(ui)
    })
}

/// A colour row: one of `ids` shown by its label, or Other (the last label) with a swatch that
/// asks the host's colour picker. The value is an id or `"#rrggbb"`. `first` is where the list
/// starts for the current values.
fn color_choice(
    key: &'static str,
    label: &'static str,
    ids: &'static [&'static str],
    labels: &'static [&'static str],
    first: impl Fn(&DialogFields) -> usize + Send + Sync + 'static,
) -> Param {
    Param::custom(key, tl!(label), move |ui, fields, cx| {
        let current = fields.text(key).unwrap_or_default().to_string();
        let at = ids.iter().position(|v| current.eq_ignore_ascii_case(v)).unwrap_or(ids.len());
        let start = first(fields);
        let entries: Vec<Entry<usize>> = labels.iter().enumerate().skip(start).map(|(i, l)| Entry::new(i, tl!(*l))).collect();
        let color = (at == ids.len()).then(|| Srgba::from(Rgba::from_hex(&current).unwrap_or(Rgba::rgb(128, 128, 128))));
        let (mut picked, mut pick_color) = (None, false);
        let shown = Row::new().show(ui, |ui, _| {
            picked = Select::new(tl!(label), Bind::given(at.max(start)), &entries).show(ui).value().copied();
            if let Some(color) = color {
                pick_color = Swatch::new(tl!(label), color).show(ui).clicked;
            }
        });
        if let Some(i) = picked {
            fields.set(key, ids.get(i).copied().unwrap_or(OTHER_COLOR));
        }
        if let Some(color) = color.filter(|_| pick_color) {
            cx.pick_color(key, color);
        }
        shown.output
    })
}

/// The picker answers a colour row with a colour value; the dialog keeps one representation, the
/// `"#rrggbb"` text the export reads.
fn normalize_colors(fields: &mut DialogFields) {
    for key in ["background", "matte"] {
        if let Some([r, g, b]) = fields.color(key) {
            let to8 = |c: f32| (c.clamp(0.0, 1.0) * 255.0).round() as u8;
            fields.set(key, format!("#{:02x}{:02x}{:02x}", to8(r), to8(g), to8(b)));
        }
    }
}

/// The JPEG Options rows: colour model, quality, method and scans, profile and image map (not
/// quality and image map for Export for Screens).
fn jpeg_rows(screens: bool) -> Vec<Param> {
    let mut rows = vec![choice("colorModel", tl!("Color Model:"), &ColorModel::ALL.map(ColorModel::id), &ColorModel::ALL.map(ColorModel::label))];
    if !screens {
        rows.push(quality_row());
    }
    rows.push(choice("method", tl!("Method:"), &Method::ALL.map(Method::id), &Method::ALL.map(Method::label)));
    let scans = ParamKind::int(Some(i64::from(*jpeg::SCANS.start())), Some(i64::from(*jpeg::SCANS.end())), 3);
    rows.push(Param::new("scans", tl!("Scans:"), scans).visible_when("method", Method::Progressive.id()));
    if !screens {
        rows.push(choice("imageMap", tl!("Image Map:"), &IMAGE_MAPS, &IMAGE_MAP_LABELS));
    }
    rows.push(check("embedIcc", tl!("Embed ICC Profile"), true));
    rows
}

/// JPEG quality on the dialog's 0–10 scale (stored ×10).
fn quality_row() -> Param {
    Param::custom("quality", tl!("Quality:"), |ui, fields, _| {
        let q = (fields.number("quality").unwrap_or(90.0) / 10.0).round().clamp(0.0, 10.0);
        let mut slid = None;
        let shown = Row::new().show(ui, |ui, _| {
            let out = SliderField::new(tl!("Quality:"), Bind::given(q), 0.0..=10.0).step(1.0).show(ui);
            slid = out.edit.value().map(|v| v.round());
            let q = slid.unwrap_or(q);
            Text::new(tl!(QUALITY_BANDS.get(q as usize).copied().unwrap_or_default())).show(ui);
        });
        if let Some(v) = slid {
            fields.set("quality", v * 10.0);
        }
        shown.output
    })
    .opens_with(90.0)
}

/// The WebP Options rows: Lossless or Lossy compression, and the quality 0–100 that Lossy uses
/// (disabled while Lossless is chosen, keeping its value).
fn webp_rows() -> Vec<Param> {
    let [lossless, lossy] = WEBP_COMPRESSION_LABELS;
    let compression = ParamKind::radio(vec![("true".into(), tl!(lossless).into()), ("false".into(), tl!(lossy).into())], Some("true".into()));
    vec![
        Param::new("lossless", tl!("Compression:"), compression),
        Param::new("quality", tl!("Quality:"), ParamKind::range(0.0, 100.0, f64::from(WEBP_QUALITY), SliderScale::Linear))
            .enabled_when("lossless", "false"),
    ]
}

/// The PNG-8 and GIF Options rows: the palette (reduction, colours, dither and amount),
/// transparency and matte, interlacing, and Transparency Dither with the amount Diffusion uses
/// (both disabled while Transparency is off, keeping their values).
fn palette_rows() -> Vec<Param> {
    let dithering = Dither::ALL.iter().copied().filter(|d| *d != Dither::None).map(Dither::id);
    vec![
        choice("reduction", tl!("Color Reduction:"), &Reduction::ALL.map(Reduction::id), &Reduction::ALL.map(Reduction::label)),
        Param::new("colors", tl!("Colors:"), ParamKind::int(Some(2), Some(256), 256)),
        choice("dither", tl!("Dither:"), &Dither::ALL.map(Dither::id), &Dither::ALL.map(Dither::label)),
        Param::new("ditherAmount", tl!("Dither Amount:"), ParamKind::int(Some(0), Some(100), 100)).enabled_when_in("dither", dithering),
        color_choice("matte", tl!("Matte:"), &MATTES, &MATTE_LABELS, |_| 0),
        check("transparency", tl!("Transparency"), true),
        choice("transparencyDither", tl!("Transparency Dither:"), &Dither::ALL.map(Dither::id), &TRANSPARENCY_DITHER_LABELS)
            .enabled_when("transparency", true),
        Param::new("transparencyDitherAmount", tl!("Transparency Dither Amount:"), ParamKind::int(Some(0), Some(100), 100))
            .visible_when("transparencyDither", Dither::Diffusion.id())
            .enabled_when("transparency", true),
        check("interlaced", tl!("Interlaced"), false),
    ]
}

/// Whole-number params the rows hold as floats or option ids are written as integers, as the
/// export reads them.
fn integers(params: &mut Value) {
    let Some(o) = params.as_object_mut() else { return };
    for key in INTEGER_KEYS {
        let number = match o.get(key) {
            Some(Value::String(text)) => text.parse::<f64>().ok(),
            Some(Value::Number(n)) => n.as_f64().filter(|_| !n.is_u64() && !n.is_i64()),
            _ => None,
        };
        if let Some(n) = number.filter(|n| n.is_finite() && n.fract() == 0.0 && *n >= 0.0) {
            o.insert(key.into(), json!(n as u64));
        }
    }
}

/// Write the file(s) with the chosen options.
pub(super) fn confirm(app: &mut VectorcraftApp, open: &Open, fields: &DialogFields) -> Result<After, String> {
    let format = fields.text("format").unwrap_or_default().to_string();
    let path = fields.text("path").unwrap_or_default().to_string();
    let id = fileio::format(&format).map_or("png", |f| f.id);
    let mut params = Value::Object(json_of(&raster_rows(id, false, None, fields), fields));
    integers(&mut params);
    if format == "psd" {
        psd_options::drop_unused(&mut params);
    }
    io::export(app, Some(&format), Some(path), &super::export_as::exported(open.selection, params)).map(|_| None)
}
