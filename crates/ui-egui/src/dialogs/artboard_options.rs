//! Artboard Options (double-click with the Artboard tool, double-click the tool, or the Artboards
//! panel's options icon or menu): the artboard's fields, sizes parsed with units, its position
//! from a reference point, its background color (`color`: a color as the paint commands take
//! it, or null for none), Show Center Mark and Show Cross Hairs (`showCenterMark`,
//! `showCrossHairs`), the display marks the canvas draws on the artboard
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/modify-display-settings-of-artboards.html),
//! and its video display options, Show Video Safe Areas and Video Ruler Pixel Aspect Ratio
//! (`showVideoSafeAreas`, `videoRulerPixelAspectRatio`)
//! (https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/set-video-display-options-for-artboards.html).
//! Sizing choices also go to the other selected artboards; fitting changes only the active
//! artboard around stationary artwork.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::color_value;
use vectorcraft_geom::{Point, Rect};

use super::{DialogSpec, form, run_and_close};
use crate::VectorcraftApp;
use crate::panels::artboards::{FILLS, fill_colors, fill_name};
use crate::panels::properties::ArtboardSizeChoice;
use crate::panels::transform::{artboard_rect, reference_index, reference_locator};
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Artboard Options").into(),
    body,
    confirm,
    // A new artboard's dialog opens without these; its swatch, checkboxes and Video Ruler Pixel
    // Aspect Ratio field add them.
    inputs: |_| &["color", "showCenterMark", "showCrossHairs", "showVideoSafeAreas", PIXEL_ASPECT],
    ..DialogSpec::FORM
};

/// Where the fields put the artboard: `x` and `y` are its top-left corner in document points, as
/// `artboard.setProps` and `artboard.new` take them.
fn placed(d: &Dialog) -> Rect {
    let (x, y) = (d.f64("x", 0.0), d.f64("y", 0.0));
    Rect::new(x, y, x + d.f64("width", 0.0), y + d.f64("height", 0.0))
}

/// The form's Name, Width and Height, then X and Y in the same grid, the reference point locator
/// before X (#1876). The locator is the one the Transform panel, the Control bar and Properties
/// share, and X and Y are where its point lies on the rulers, counted from the ruler origin (#1212),
/// as the Transform panel and the Control bar show an artboard's. A new Width or Height keeps the
/// reference point where it was, as their sizes do.
/// Then Background Color: the Fill list (None, White, Black; Custom for another color) and a swatch
/// that picks a color of its own, the Show Center Mark, Show Cross Hairs and Show Video Safe Areas
/// checkboxes, and the Video Ruler Pixel Aspect Ratio field.
fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (unit, origin, refi) = (app.session.general_unit(), app.ruler_origin(), reference_index(ui));
    let (was, w0, h0) = (placed(d), d.f64("width", 0.0), d.f64("height", 0.0));
    let bounds = [fit_bounds(app, ui.ctx(), false), fit_bounds(app, ui.ctx(), true)];
    let (choice, fit) = crate::panels::properties::artboard_size_controls_with_fit(app, ui, w0, h0, bounds.map(|b| b.is_some()));
    let picked = choice.is_some();
    if let Some(choice) = choice {
        if let Some((w, h)) = choice.size_for(w0, h0) {
            set_rect(d, artboard_rect(was, refi, None, None, Some(w), Some(h), false));
        }
        remember_size_choice(d, choice);
        d.fields.insert("__refx".into(), json!(refi % 3));
        d.fields.insert("__refy".into(), json!(refi / 3));
        d.fields.remove("__fit");
    }
    ui.add_space(8.0);
    form::grid_then(ui, d, unit, |ui, d| {
        let (w, h) = (d.f64("width", 0.0), d.f64("height", 0.0));
        if !picked && (w != w0 || h != h0) {
            d.fields.remove("__sizeChoice");
        }
        let n = artboard_rect(was, refi, None, None, Some(w), Some(h), false);
        // Only the side that changed moves, so an untouched coordinate stays exactly as it was. The
        // reference point's column or row is kept for the other selected artboards (`confirm`).
        for (key, changed, at, part) in [("x", w != w0, n.x0, refi % 3), ("y", h != h0, n.y0, refi / 3)] {
            if changed {
                d.fields.insert(key.into(), json!(at));
                d.fields.insert(format!("__ref{key}"), json!(part));
            }
        }
        position_rows(ui, d, refi, unit, origin);
    });
    // Apply a Fit after the ordinary size/reference-point grid: that grid must not replace
    // the chosen artwork's position with one derived from the previous artboard rectangle.
    if let Some(selection) = fit
        && let Some(bounds) = bounds[usize::from(selection)]
    {
        set_rect(d, bounds);
        d.fields.insert("__fit".into(), json!(true));
        d.fields.remove("__sizeChoice");
    }
    ui.add_space(8.0);
    let color = d.fields.get("color").and_then(color_value);
    let fills = fill_colors(app);
    let mut set = None;
    ui.horizontal(|ui| {
        let row = form::row_label(ui, tl!("Background Color:"));
        ui.label_inputs_by(row.id, |ui| {
            if let Some(k) =
                crate::widgets::dropdown_named(ui, "ab-options-fill", Some(tl!("Background Color:")), fill_name(app, color), &FILLS, 110.0)
            {
                set = fills.get(k).copied();
            }
            let [r, g, b, _] = color.unwrap_or(Color::WHITE).to_rgba8(1.0);
            let mut rgb = [r, g, b];
            let swatch_name = format!("{} {}", tl!("Background Color:"), tl!("Swatches"));
            if ui.name_inputs(swatch_name, |ui| ui.color_edit_button_srgb(&mut rgb)).changed() {
                set = Some(Some(Color::rgb(f32::from(rgb[0]) / 255.0, f32::from(rgb[1]) / 255.0, f32::from(rgb[2]) / 255.0)));
            }
        });
    });
    if let Some(c) = set {
        d.fields.insert("color".into(), c.as_ref().map_or(Value::Null, crate::panels::color_json));
    }
    ui.add_space(8.0);
    form::check(ui, d, "showCenterMark", tl!("Show Center Mark"));
    form::check(ui, d, "showCrossHairs", tl!("Show Cross Hairs"));
    form::check(ui, d, "showVideoSafeAreas", tl!("Show Video Safe Areas"));
    ui.horizontal(|ui| {
        let row = form::row_label(ui, tl!("Video Ruler Pixel Aspect Ratio:"));
        ui.label_inputs_by(row.id, |ui| {
            let range = vectorcraft_doc::VIDEO_PIXEL_ASPECT_RANGE;
            let now = d.f64(PIXEL_ASPECT, 1.0);
            if let Some(v) = crate::widgets::range_field(ui, ("dlg-num", PIXEL_ASPECT), now, range, "", 4, 60.0) {
                d.fields.insert(PIXEL_ASPECT.into(), json!(v));
            }
        });
    });
    false
}

/// The field of the Video Ruler Pixel Aspect Ratio, as `artboard.setProps` takes it.
pub(super) const PIXEL_ASPECT: &str = "videoRulerPixelAspectRatio";

/// Store document-coordinate draft bounds, retaining an editable minimum for zero-area art.
fn set_rect(d: &mut Dialog, r: Rect) {
    for (key, value) in [("x", r.x0), ("y", r.y0), ("width", r.width().max(1.0)), ("height", r.height().max(1.0))] {
        d.fields.insert(key.into(), json!(value));
    }
}

/// Preserve preset/orientation intent separately from typed dimensions for the selected set.
fn remember_size_choice(d: &mut Dialog, choice: ArtboardSizeChoice) {
    let value = match choice {
        ArtboardSizeChoice::Preset(w, h) => json!({"preset": [w, h]}),
        ArtboardSizeChoice::Orientation(landscape) => json!({"landscape": landscape}),
    };
    d.fields.insert("__sizeChoice".into(), value);
}

/// Validate stored sizing intent before deriving the other selected artboards' dimensions.
fn size_choice(d: &Dialog) -> Option<ArtboardSizeChoice> {
    let value = d.fields.get("__sizeChoice")?;
    if let Some(landscape) = value.get("landscape").and_then(Value::as_bool) {
        return Some(ArtboardSizeChoice::Orientation(landscape));
    }
    let preset = value.get("preset")?.as_array()?;
    let (w, h) = (preset.first()?.as_f64()?, preset.get(1)?.as_f64()?);
    (w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0).then_some(ArtboardSizeChoice::Preset(w, h))
}

#[derive(Clone)]
struct ArtworkFitBounds {
    key: (u64, u64),
    bounds: Option<Rect>,
}

/// Artwork fitting includes all drawn artwork, even outside every artboard; selection uses
/// the shared selection cache. Draft/target geometry never restricts membership. Global artwork
/// bounds are cached only for the active document and its revision.
fn fit_bounds(app: &mut VectorcraftApp, ctx: &egui::Context, selection: bool) -> Option<Rect> {
    if selection {
        return app.selection_bounds();
    }
    let st = app.session.active()?;
    let key = (st.uid, st.revision);
    let id = egui::Id::unique("artboard-options-artwork-fit-bounds");
    if let Some(cached) = ctx.data_mut(|data| data.get_temp::<ArtworkFitBounds>(id))
        && cached.key == key
    {
        return cached.bounds;
    }
    let bounds = vectorcraft_render::encode::art_bounds(&st.doc);
    ctx.data_mut(|data| data.insert_temp(id, ArtworkFitBounds { key, bounds }));
    bounds
}

/// The X and Y rows: where reference point `refi` lies, counted from the ruler origin `origin`,
/// with the locator before X's label. A typed X or Y moves the artboard so that its reference point
/// lands there.
fn position_rows(ui: &mut egui::Ui, d: &mut Dialog, refi: usize, unit: Unit, origin: Point) {
    let r = placed(d);
    let rp = vectorcraft_geom::reference_point(r, refi);
    let mut indent = 0.0;
    for (key, label, at, zero) in [("x", "X:", rp.x, origin.x), ("y", "Y:", rp.y, origin.y)] {
        let row = ui
            .horizontal(|ui| {
                // Y's label lines up with X's, after the locator.
                if key == "x" {
                    let left = ui.cursor().min.x;
                    reference_locator(ui, refi);
                    indent = ui.min_rect().right() - left;
                } else {
                    ui.add_space(indent);
                }
                form::row_label(ui, label)
            })
            .inner;
        let value = ui.label_inputs_by(row.id, |ui| crate::widgets::num_field(ui, ("dlg-ab-xy", key), Some(at - zero), unit, form::FIELD_W));
        if let Some(v) = value {
            let n = if key == "x" {
                artboard_rect(r, refi, Some(v + zero), None, None, None, false).x0
            } else {
                artboard_rect(r, refi, None, Some(v + zero), None, None, false).y0
            };
            d.fields.insert(key.into(), json!(n));
        }
        ui.end_row();
    }
}

/// The dialog kind of Artboard Options.
pub const KIND: &str = "artboardOptions";

/// The active artboard's index: the Artboard tool's while it is in use, else the window's.
pub(crate) fn active_index(app: &VectorcraftApp) -> usize {
    if app.session.tool_id() == "artboard" {
        app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0)
    } else {
        app.view().map_or(0, |v| v.artboard)
    }
}

/// Open Artboard Options for the active artboard ([`active_index`]).
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let index = active_index(app);
    let st = app.session.active().ok_or("no document")?;
    let a = st.doc.artboards.get(index).ok_or("no artboard")?;
    let r = a.rect;
    let color = a.color.as_ref().map(crate::panels::color_json);
    let fields = json!({
        "index": index, "name": a.name, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height(), "color": color,
        "showCenterMark": a.show_center_mark, "showCrossHairs": a.show_cross_hairs,
        "showVideoSafeAreas": a.show_video_safe_areas, PIXEL_ASPECT: a.video_ruler_pixel_aspect_ratio,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// OK sets the artboard's fields. A preset or orientation choice applies separately to each
/// selected artboard; a typed Width or Height that differs also goes to the others (when this
/// one is among them), each resized about the reference
/// point the form's last Width or Height change used (fields `__refx` and `__refy`, its column and
/// row; the top-left corner when the form set none), all in one undo step. Fitting applies only
/// to this artboard and never moves its artwork.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool("new") {
        return confirm_new(app, d);
    }
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height", PIXEL_ASPECT] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    if let Some(o) = p.as_object_mut() {
        o.retain(|k, _| !k.starts_with("__"));
    }
    let index = p["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
    let was = app.session.active().and_then(|st| st.doc.artboards.get(index)).map(|a| a.rect);
    let changed = |k: &str, old: Option<f64>| p[k].as_f64().filter(|v| old.is_some_and(|old| (v - old).abs() > 1e-6));
    let (w, h) = (changed("width", was.map(|r| r.width())), changed("height", was.map(|r| r.height())));
    let part = |k: &str| d.fields.get(k).and_then(Value::as_u64).map_or(0, |v| v.min(2) as usize);
    let refi = part("__refy") * 3 + part("__refx");
    let choice = size_choice(d);
    let mut entries = vec![];
    if !d.bool("__fit") && (choice.is_some() || w.is_some() || h.is_some()) {
        for i in crate::panels::artboards::resized_with(app, index).into_iter().filter(|i| *i != index) {
            if let Some(r) = app.session.active().and_then(|st| st.doc.artboards.get(i)).map(|a| a.rect) {
                let (w, h) = if let Some(choice) = choice {
                    let Some((w, h)) = choice.size_for(r.width(), r.height()) else { continue };
                    (Some(w), Some(h))
                } else {
                    (w, h)
                };
                let n = artboard_rect(r, refi, None, None, w, h, false);
                entries.push(json!({"index": i, "x": n.x0, "y": n.y0, "width": n.width(), "height": n.height()}));
            }
        }
    }
    entries.insert(0, p);
    // Fitting changes the artboard around existing art; it must neither move nor scale that art.
    let common = json!({
        "scaleArt": !d.bool("__fit") && crate::panels::artboards::scale_art(app),
        "moveArt": !d.bool("__fit") && crate::panels::artboards::move_art(app),
    });
    let p = crate::panels::artboards::set_props_params(common, entries).ok_or("no artboard")?;
    run_and_close(app, "artboard.setProps", p)
}

/// `ui.newArtboard` (Alt-click on the Artboards panel's New Artboard): Artboard Options for a new
/// artboard (field `new`), filled in as `artboard.new {}` would make it: named Artboard N (N one
/// more than the number of artboards), as large as the last artboard and 20 pt to the right of it.
pub fn open_new(app: &mut VectorcraftApp) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let abs = &st.doc.artboards;
    let last = abs.last().map_or(vectorcraft_geom::Rect::new(0.0, 0.0, 612.0, 792.0), |a| a.rect);
    let fields = json!({
        "new": true, "name": format!("Artboard {}", abs.len() + 1),
        "x": last.x1 + 20.0, "y": last.y0, "width": last.width(), "height": last.height(),
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// OK for a new artboard: `artboard.new` with the fields (an empty name gives the default one),
/// the background color picked, the display marks ticked and the Video Ruler Pixel Aspect Ratio
/// typed, and the new artboard becomes the active one.
fn confirm_new(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = json!({});
    for k in ["x", "y", "width", "height"] {
        p[k] = json!(d.f64(k, 0.0));
    }
    let name = d.str("name");
    if !name.trim().is_empty() {
        p["name"] = json!(name);
    }
    if let Some(c) = d.fields.get("color") {
        p["color"] = c.clone();
    }
    for k in ["showCenterMark", "showCrossHairs", "showVideoSafeAreas"] {
        if d.bool(k) {
            p[k] = json!(true);
        }
    }
    if d.fields.contains_key(PIXEL_ASPECT) {
        p[PIXEL_ASPECT] = json!(d.f64(PIXEL_ASPECT, 1.0));
    }
    let r = run_and_close(app, "artboard.new", p)?;
    if let Some(i) = r["index"].as_u64().and_then(|i| usize::try_from(i).ok()) {
        crate::panels::artboards::select(app, i);
    }
    Ok(r)
}
