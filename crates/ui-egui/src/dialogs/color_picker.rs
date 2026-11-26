//! The Color Picker (double-click a Fill/Stroke proxy, or `ui.colorPicker`) on the framework's
//! [`ColorPickerDialog`]. With art selected the colour shows on it live as it is picked; OK keeps it
//! as one undo step through `paint.setFill` / `paint.setStroke` and makes the colour the Color
//! Guide's base colour; Cancel puts the art back. Opened for an artboard (Properties › Artboards ›
//! Fill), it shows the colour on the artboard's paper and OK keeps it through
//! `artboard.setProps {index, color}`.
//!
//! A command dialog's colour row raises a `ColorRequest`; [`pump`] opens a picker over that dialog
//! and answers the row with its OK. The OK for a Save for Web colour table cell shifts the cell's
//! colour in the table instead ([`super::save_for_web::take_palette_pick`]).
//!
//! Fields agents set with `ui.dialog.set`: the framework picker's (`color`, `mode`, `webColors`,
//! `swatches`, `library`). `stroke`, `artboard` and `color` come from the opener.

use std::hash::Hash;
use std::sync::Arc;

use craft::cms::Profile;
use craft::cms::convert::CmykConverter;
use craft::color_ui::ui::{ColorPickerDialog, PickedColor, PickerLibrary, PickerSwatches};
use craft::theme::Srgba;
use craft::ui::dialog::{DialogId, DialogResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_color::cms;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::ColorMode;
use vectorcraft_engine::cmd::color_value;
use vectorcraft_engine::cmd::swatchlib;

use super::preview::LivePreview;
use crate::panels::color::{WorkingRgb, color_of_values, gamut, model_values};
use crate::panels::swatches::{DOCUMENT_SWATCHES, colour_libraries};
use crate::panels::{color_json, srgba};
use crate::{VectorcraftApp, screen_color};

/// What a picker's OK writes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Target {
    /// The fill proxy.
    Fill,
    /// The stroke proxy.
    Stroke,
    /// The background colour of this artboard.
    Artboard(usize),
    /// A colour row `key` of the open dialog `parent`.
    Answer { parent: DialogId, key: String },
}

/// One open Color Picker: what the dialog itself does not know.
pub(crate) struct Open {
    /// The dialog's id on the window's host.
    pub id: DialogId,
    /// What its OK writes.
    pub target: Target,
    /// The colour it opened with.
    pub start: Srgba,
    /// OK with `start` unchanged leaves the target as it is (an artboard opened without a `color`).
    pub keep: bool,
    /// The live preview of its colour on the target.
    pub preview: LivePreview,
}

/// The records of the colour area's open dialogs: one holder, so the app has one field for them.
#[derive(Default)]
pub(crate) struct ColorDialogs {
    // PLAN: the `Open` records of color_balance, saturate and recolor come from color-w18 and color-w19.
    pub balance: Option<super::color_balance::Open>,
    pub saturate: Option<super::saturate::Open>,
    pub recolor: Option<super::recolor::Open>,
    pub pickers: Vec<Open>,
}

/// The colour the proxy shows (a gradient's first stop; the last colour for None and patterns).
fn proxy_color(app: &VectorcraftApp, stroke: bool) -> Color {
    let (f, s) = crate::panels::current_paints(app);
    match if stroke { s } else { f } {
        Paint::Solid { color, .. } => color,
        Paint::Gradient(g) => g.gradient.stops.first().map_or(app.session.last_solid, |s| s.color),
        Paint::None | Paint::Pattern { .. } => app.session.last_solid,
    }
}

/// The colour a pick stands for in the document: its own model values when it has them (CMYK inks,
/// Lab outside sRGB), else the display sRGB in the working RGB space.
fn color_of(picked: &PickedColor) -> Color {
    match &picked.model {
        Some(values) => color_of_values(values),
        None => screen_color::sampled(&cms::active(), [picked.color.r, picked.color.g, picked.color.b]),
    }
}

/// The owner a screen pick for picker `id` is taken by.
pub(crate) fn owner(id: DialogId) -> impl Hash {
    ("color-picker", id)
}

/// The Color Picker that is open, newest last, that a screen pick can go to.
pub(crate) fn open_picker(app: &VectorcraftApp) -> Option<DialogId> {
    let dialogs = app.dialogs()?;
    let open = |id: DialogId| dialogs.with(|h| h.get_by_id::<ColorPickerDialog>(id).is_some());
    app.color_dialogs.pickers.iter().rev().map(|o| o.id).find(|&id| open(id))
}

/// The document's swatches and the swatch libraries as the picker's Color Swatches view: solid
/// colours only, tints as their colour.
fn swatch_view(app: &VectorcraftApp) -> PickerSwatches {
    let solid = |swatches: &[vectorcraft_color::Swatch]| -> Vec<(Label, Srgba)> {
        swatches.iter().filter_map(|s| Some((Label::from(s.name.clone()), srgba(&s.paint.color()?)))).collect()
    };
    let mut libraries = Vec::new();
    if let Some(st) = app.session.active() {
        let all: Vec<_> = st.doc.swatches.iter().chain(st.doc.swatch_groups.iter().flat_map(|g| g.swatches.iter())).cloned().collect();
        libraries.push(PickerLibrary::new(Label::from(DOCUMENT_SWATCHES), solid(&all)));
    } else {
        libraries.push(PickerLibrary::new(Label::from(DOCUMENT_SWATCHES), Vec::new()));
    }
    for lib in colour_libraries(app) {
        let swatches = swatchlib::library(&app.session, &lib.id)
            .map(|(_, l)| solid(&l.swatches.iter().chain(l.groups.iter().flat_map(|g| g.swatches.iter())).cloned().collect::<Vec<_>>()))
            .unwrap_or_default();
        libraries.push(PickerLibrary::new(Label::from(lib.name.clone()), swatches));
    }
    PickerSwatches::new(libraries, tl!("Open a document to see its swatches."))
}

/// The working CMYK profile as the picker's CMYK converter, so its CMYK fields follow the document's
/// inks; `None` (the picker's built-in coated CMYK) when the profile can't link to sRGB.
fn working_cmyk() -> Option<CmykConverter> {
    let bytes = cms::icc_bytes(&cms::active().settings().cmyk).ok()?;
    CmykConverter::new(Arc::new(Profile::parse(&bytes).ok()?)).ok()
}

/// `ui.colorPicker {stroke?, artboard?, color?}`: open the picker for the fill (or stroke) proxy,
/// or for artboard `artboard`'s background color, starting from `color`, else the proxy's color
/// or the artboard's color (white when it has none).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let stroke = p.get("stroke").and_then(Value::as_bool).unwrap_or(!app.session.fill_active);
    let artboard = match p.get("artboard") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let index = v.as_u64().and_then(|i| usize::try_from(i).ok());
            let found = index.and_then(|i| Some((i, app.session.active()?.doc.artboards.get(i)?.color)));
            let (i, color) = found.ok_or("no such artboard")?;
            Some((i, color.unwrap_or(Color::WHITE)))
        }
    };
    let color = match (p.get("color"), artboard) {
        (Some(v), _) => color_value(v).ok_or_else(|| format!("bad color {v}"))?,
        (None, Some((_, c))) => c,
        (None, None) => proxy_color(app, stroke),
    };
    let dialogs = app.dialogs().ok_or("no window")?;
    let start = srgba(&color);
    let mut dialog = ColorPickerDialog::new(
        match (artboard, stroke) {
            (Some(_), _) => "artboard",
            (None, true) => "stroke",
            (None, false) => "fill",
        },
        tl!("Color Picker"),
        start,
    )
    .eyedropper(true)
    .add_to_swatches(false)
    .gamut(gamut(app))
    .working_rgb(WorkingRgb)
    .swatch_view(swatch_view(app));
    if app.session.active().is_some_and(|s| s.doc.color_mode == ColorMode::Cmyk)
        && let Some(converter) = working_cmyk()
    {
        dialog = dialog.cmyk(&converter);
    }
    if matches!(color, Color::Cmyk { .. } | Color::Lab { .. }) {
        dialog = dialog.kept(model_values(&color));
    }
    // PLAN: assumed the dialog's own `instance` keeps artboards apart; the host replaces a picker only for the same target.
    if let Some((i, _)) = artboard {
        dialog = dialog.instance(i);
    }
    let id = dialogs.with(|h| h.open(dialog));
    // A picker the open replaced leaves its preview behind: end it.
    let before = app.color_dialogs.pickers.len();
    app.color_dialogs.pickers.retain(|o| o.id != id);
    if app.color_dialogs.pickers.len() != before {
        let _ = app.session.cancel_interaction();
    }
    let target = match artboard {
        Some((i, _)) => Target::Artboard(i),
        None if stroke => Target::Stroke,
        None => Target::Fill,
    };
    // OK with the artboard's color (or the white that stands in for none) unchanged keeps the artboard as it is.
    let keep = artboard.is_some() && p.get("color").is_none();
    app.color_dialogs.pickers.push(Open { id, target, start, keep, preview: LivePreview::default() });
    Ok(Value::Null)
}

/// Once per frame (after the shell pass): opens a picker for each dialog's colour request, previews
/// each open picker's colour on the art, hands it the screen picks, and settles the ones that
/// closed.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    while let Some((parent, request)) = dialogs.with(|h| h.take_color_request()) {
        let start = request.color.unwrap_or(Srgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 });
        let key = request.key.clone();
        let child = ColorPickerDialog::from_request(request).eyedropper(true).add_to_swatches(false).gamut(gamut(app)).working_rgb(WorkingRgb);
        let id = dialogs.with(|h| h.open_child(child, parent));
        app.color_dialogs.pickers.push(Open { id, target: Target::Answer { parent, key }, start, keep: false, preview: LivePreview::default() });
    }
    let busy = app.screen_pick.busy();
    let mut pickers = std::mem::take(&mut app.color_dialogs.pickers);
    pickers.retain_mut(|open| settle(app, ctx, &dialogs, open, busy));
    // A picker opened meanwhile by a command the pass ran.
    pickers.append(&mut app.color_dialogs.pickers);
    app.color_dialogs.pickers = pickers;
}

/// One picker's pass; `false` when it has closed and its record can go.
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context, dialogs: &craft::ui::shell::WindowDialogs, open: &mut Open, busy: bool) -> bool {
    let live = dialogs.with(|h| {
        h.get_mut_by_id::<ColorPickerDialog>(open.id).map(|d| {
            d.screen_busy(busy);
            (d.current(), d.take_screen_pick())
        })
    });
    let Some((current, gesture)) = live else {
        let result = dialogs.with(|h| h.take_by_id::<ColorPickerDialog>(open.id));
        finish(app, dialogs, open, result);
        return false;
    };
    let held = gesture.and_then(|g| screen_color::hold(app, ctx, owner(open.id), g));
    let sampled = held.or_else(|| screen_color::take(app, ctx, owner(open.id)));
    let shown = match sampled {
        Some(color) => {
            let picked = PickedColor::rgb(color);
            dialogs.with(|h| h.get_mut_by_id::<ColorPickerDialog>(open.id).map(|d| d.set_color(picked)));
            Some(picked)
        }
        None => current,
    };
    if let Some(picked) = shown {
        preview(app, open, &picked);
    }
    true
}

/// What a closed picker did: OK writes its colour, Cancel puts the art back.
fn finish(
    app: &mut VectorcraftApp,
    dialogs: &craft::ui::shell::WindowDialogs,
    open: &mut Open,
    result: Option<DialogResult<craft::color_ui::ui::ColorPickerResult>>,
) {
    let preview = std::mem::take(&mut open.preview);
    match (result, &open.target) {
        (Some(DialogResult::Ok(r)), Target::Answer { parent, key }) => {
            if !super::save_for_web::take_palette_pick(app, *parent, key, open.start, r.picked.color) {
                dialogs.with(|h| h.answer_color(*parent, key, r.picked.color));
            }
        }
        (Some(DialogResult::Ok(r)), _) => {
            if let Err(e) = commit(app, open, preview, &r.picked) {
                app.status(e);
            }
        }
        _ => preview.cancel(app),
    }
}

/// The command that applies colour `c` to the picker's target, and its params.
fn paint_command(target: &Target, c: Color) -> (&'static str, Value) {
    match target {
        Target::Artboard(i) => ("artboard.setProps", json!({ "index": i, "color": color_json(&c) })),
        Target::Stroke => ("paint.setStroke", json!({ "color": color_json(&c) })),
        Target::Fill | Target::Answer { .. } => ("paint.setFill", json!({ "color": color_json(&c) })),
    }
}

/// The picked colour as the document takes it: an RGB colour is CMYK in a CMYK document; CMYK,
/// grey and Lab colours stay as they are.
fn in_document_model(app: &VectorcraftApp, c: Color) -> Color {
    if matches!(c, Color::Rgb { .. }) && app.session.active().is_some_and(|s| s.doc.color_mode == ColorMode::Cmyk) {
        let [cy, m, y, k] = c.to_cmyk_managed(cms::active().settings().intent);
        return Color::cmyk(cy, m, y, k);
    }
    c
}

/// Show the picked colour on the target while the picker is open: each new colour replaces the last
/// preview. Nothing previews until the colour first changes, nor without a selection (the colour
/// then only becomes the default paint, on OK). An artboard shows it with or without a selection.
fn preview(app: &mut VectorcraftApp, open: &mut Open, picked: &PickedColor) {
    // A picker over a command dialog only fills a row; the dialog previews it.
    if matches!(open.target, Target::Answer { .. }) {
        return;
    }
    let (cmd, params) = paint_command(&open.target, in_document_model(app, color_of(picked)));
    let untouched = !app.session.in_interaction() && picked.color == open.start;
    let artboard = matches!(open.target, Target::Artboard(_));
    let selected = artboard || app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    if untouched || !selected {
        return;
    }
    let label = match open.target {
        Target::Artboard(_) => "Artboard Options",
        Target::Stroke => "Stroke Color",
        _ => "Fill Color",
    };
    open.preview.run(app, true, label, cmd, &params);
}

/// OK: apply the colour to the target, keeping the live preview as one undo step. An untouched
/// artboard picker with `keep` leaves the artboard as it was.
fn commit(app: &mut VectorcraftApp, open: &Open, preview: LivePreview, picked: &PickedColor) -> Result<(), String> {
    if open.keep && !app.session.in_interaction() && picked.color == open.start {
        return Ok(());
    }
    let c = in_document_model(app, color_of(picked));
    let (cmd, params) = paint_command(&open.target, c);
    preview.commit(app, cmd, params)?;
    // A color picked for the fill or stroke also becomes the Color Guide's base color.
    if !matches!(open.target, Target::Artboard(_)) {
        crate::panels::color_guide::choose_base(app, c);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
