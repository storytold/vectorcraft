//! Object › Create Object Mosaic…: the selected image's Current Size, the mosaic's New Size, Tile
//! Spacing and Number of Tiles, Constrain Ratio (Width or Height), Result (Color or Grayscale),
//! Resize Using Percentages, Delete Raster and Use Ratio. OK runs `object.createObjectMosaic` as one
//! undo step.
//!
//! Fields: `width`, `height` (pt, or percentages of the image's size with `percentages`),
//! `spacingX`, `spacingY` (pt), `columns`, `rows`, `lock` (Constrain Ratio on), `constrain`
//! (`width` | `height`: the side Constrain Ratio and Use Ratio keep), `gray`, `percentages` and
//! `deleteRaster`. `__imageW` and `__imageH` hold the image's size (pt), read again from the
//! selected image each frame, before `ui.dialog.set` sets fields and on OK; `__noImage` is true
//! while no image is selected.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Create Object Mosaic.
pub const KIND: &str = "objectMosaic";

/// The command the dialog runs.
pub const COMMAND: &str = "object.createObjectMosaic";

/// Width of the label column.
const LABEL_W: f32 = 112.0;

/// Width of a size or count field.
const FIELD_W: f32 = 76.0;

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Create Object Mosaic").into(), body, confirm, min_width: 360.0, ..DialogSpec::FORM };

/// The size (pt) of the first selected image, which the mosaic is made from.
fn image_size(app: &VectorcraftApp) -> Option<(f64, f64)> {
    let st = app.session.active()?;
    let node = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).find(|n| matches!(n.kind, NodeKind::Image(_)))?;
    let b = node.geometric_bounds()?;
    Some((b.width(), b.height()))
}

/// Open the dialog on the selected image, with the values `given` holds (the menu item's params, or
/// a caller's `object.createObjectMosaic` params, its Use Ratio applied to the counts) and the
/// command's defaults for the rest. With no image selected, or with params the command refuses,
/// nothing opens and the error says why.
pub fn open(app: &mut VectorcraftApp, given: &Value) -> Result<Value, String> {
    let Some((w, h)) = image_size(app) else { return Err("select an image".into()) };
    let none = json!({});
    let given = if given.is_null() { &none } else { given };
    app.session.check_params(COMMAND, given).map_err(|e| e.to_string())?;
    let (cols, rows) = vectorcraft_engine::cmd::mosaic_counts(&app.session, given).map_err(|e| e.to_string())?;
    let n = |key: &str, d: f64| given.get(key).and_then(Value::as_f64).unwrap_or(d);
    let b = |key: &str| given.get(key).and_then(Value::as_bool).unwrap_or(false);
    let pct = b("percentages");
    // The new size starts at the image's, within the sizes the command takes (0.1 to 1,000,000 pt).
    let (base_w, base_h) = if pct { (100.0, 100.0) } else { (w.clamp(0.1, 1.0e6), h.clamp(0.1, 1.0e6)) };
    let named = |key: &str| given.get(key).and_then(Value::as_str).filter(|s| matches!(*s, "width" | "height"));
    let constrain = named("constrain");
    // The side the dialog keeps: Constrain Ratio's, else the one Use Ratio names.
    let side = constrain.or(named("useRatio")).unwrap_or("width");
    let fields = json!({
        "width": n("width", base_w), "height": n("height", base_h), "spacingX": n("spacingX", 0.0), "spacingY": n("spacingY", 0.0),
        "columns": cols, "rows": rows,
        "lock": constrain.is_some(), "constrain": side, "gray": b("gray"), "percentages": pct,
        "deleteRaster": b("deleteRaster"), "__imageW": w, "__imageH": h,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

/// The field that is true while no image is selected.
const NO_IMAGE: &str = "__noImage";

/// What the dialog shows when Resize Using Percentages is switched with no image selected.
const NO_IMAGE_WHY: &str = "select an image";

/// The selected image's size `size` (from [`image_size`]) read into the dialog, so New Size is
/// converted and constrained against the image as it is now, also after it was scaled while the
/// dialog was open. With no image selected the last size read stays and `__noImage` is set.
fn read_image(size: Option<(f64, f64)>, d: &mut Dialog) {
    let Some((w, h)) = size else {
        d.fields.insert(NO_IMAGE.into(), json!(true));
        return;
    };
    if d.fields.remove(NO_IMAGE).is_some() && d.str(form::REFUSED) == NO_IMAGE_WHY {
        d.fields.remove(form::REFUSED);
    }
    d.fields.insert("__imageW".into(), json!(w));
    d.fields.insert("__imageH".into(), json!(h));
}

/// [`read_image`] on the open dialog when it is this one. `ui.dialog.set` runs it before it sets
/// any field.
pub(crate) fn refresh(app: &mut VectorcraftApp) {
    let size = image_size(app);
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == KIND) {
        read_image(size, d);
    }
}

/// The image's size (pt) the dialog holds.
fn image(d: &Dialog) -> (f64, f64) {
    (d.f64("__imageW", 1.0).max(1e-9), d.f64("__imageH", 1.0).max(1e-9))
}

/// Whether Constrain Ratio and Use Ratio keep the width (else the height).
fn keeps_width(d: &Dialog) -> bool {
    d.str("constrain") != "height"
}

/// With Constrain Ratio on, the size field that follows the kept side.
fn follower(d: &Dialog) -> Option<&'static str> {
    d.bool("lock").then(|| if keeps_width(d) { "height" } else { "width" })
}

/// The new sizes the command takes, in points or percentages (`object.createObjectMosaic`).
const SIZE_RANGE: std::ops::RangeInclusive<f64> = 0.1..=1.0e6;

/// With Constrain Ratio on, the side that follows set from the side that leads, in the image's
/// proportions. A size `ui.dialog.set` gave the following side that the command refuses stays,
/// for OK to report.
fn follow_ratio(d: &mut Dialog) {
    let Some((key, v)) = derived(d) else { return };
    if d.bool(&kept(key)) {
        if !takes(d, key) {
            return;
        }
        d.fields.remove(&kept(key));
    }
    d.fields.insert(key.into(), json!(v));
}

/// The field that is true while size field `key` holds a value `ui.dialog.set` gave and the
/// command refuses.
fn kept(key: &str) -> String {
    format!("__kept.{key}")
}

/// Whether the command takes size field `key` as it is: a number from 0.1 to 1,000,000.
fn takes(d: &Dialog, key: &str) -> bool {
    number(d, key, 0.0).as_f64().is_some_and(|x| SIZE_RANGE.contains(&x))
}

/// `ui.dialog.set` of `field`: a new size the command refuses (not a number, or outside 0.1 to
/// 1,000,000) stays in its field while it is refused, also on the side Constrain Ratio sets, instead
/// of being replaced by the size worked out from the kept side.
pub(super) fn note_set(d: &mut Dialog, field: &str) {
    if !matches!(field, "width" | "height") {
        return;
    }
    if takes(d, field) {
        d.fields.remove(&kept(field));
    } else {
        d.fields.insert(kept(field), json!(true));
    }
}

/// With Constrain Ratio on, the side that follows and the size it gets from the side that leads.
fn derived(d: &Dialog) -> Option<(&'static str, f64)> {
    let key = follower(d)?;
    let (iw, ih) = image(d);
    let pct = in_percentages(d);
    Some(if key == "height" {
        let w = d.f64("width", 0.0);
        (key, if pct { w } else { w * ih / iw })
    } else {
        let h = d.f64("height", 0.0);
        (key, if pct { h } else { h * iw / ih })
    })
}

/// Resize Using Percentages turned on or off: the new size shown the other way, so it stays the
/// same size. When a size field does not hold a number, or a size the command takes (all but the
/// side Constrain Ratio sets) falls outside what the command accepts either as it is or the other
/// way, the fields and the mode stay as they are and the dialog shows why. With no image selected, nothing changes and
/// the dialog asks for one.
fn switch_percentages(d: &mut Dialog, on: bool) {
    if d.bool(NO_IMAGE) {
        d.fields.insert(form::REFUSED.into(), json!(NO_IMAGE_WHY));
        return;
    }
    let (iw, ih) = image(d);
    let size = |key: &str| number(d, key, 100.0).as_f64().ok_or_else(|| format!("`{key}` must be a number (got {})", d.str(key)));
    let (w, h) = match (size("width"), size("height")) {
        (Ok(w), Ok(h)) => (w, h),
        (Err(why), _) | (_, Err(why)) => {
            d.fields.insert(form::REFUSED.into(), json!(why));
            return;
        }
    };
    let skip = follower(d);
    let checked = |key: &str| skip != Some(key) || d.bool(&kept(key));
    let refused = |w: f64, h: f64| {
        let (key, v) = [("width", w), ("height", h)].into_iter().find(|(k, v)| checked(k) && !SIZE_RANGE.contains(v))?;
        Some(format!("`{key}` must be between {} and {} (got {v})", SIZE_RANGE.start(), SIZE_RANGE.end()))
    };
    // A size the command refuses as it is stays refused, even when it falls in range the other way.
    let converted = if on { (w / iw * 100.0, h / ih * 100.0) } else { (iw * w / 100.0, ih * h / 100.0) };
    if let Some(why) = refused(w, h).or_else(|| refused(converted.0, converted.1)) {
        d.fields.insert(form::REFUSED.into(), json!(why));
        return;
    }
    let (w, h) = converted;
    d.fields.remove(form::REFUSED);
    d.fields.remove(&kept("percentages"));
    d.fields.insert("width".into(), json!(w));
    d.fields.insert("height".into(), json!(h));
    d.fields.insert("percentages".into(), json!(on));
}

/// Whether the new size fields hold percentages: Resize Using Percentages, or while that field holds
/// a value that is not true or false, the mode it held before that value was set.
fn in_percentages(d: &Dialog) -> bool {
    match d.fields.get("percentages") {
        Some(Value::Bool(on)) => *on,
        Some(_) => d.bool(&kept("percentages")),
        None => false,
    }
}

/// `ui.dialog.set` of `percentages`: true or false switches the new size as the checkbox does, and
/// replaces a value that is not true or false; null is false. Another value is kept as it is, and
/// the command refuses it on OK.
pub(super) fn set_percentages(d: &mut Dialog, value: &Value) {
    let mode = in_percentages(d);
    // The command reads null as the field left out, which is off.
    match if value.is_null() { Some(false) } else { value.as_bool() } {
        Some(on) if on != mode => switch_percentages(d, on),
        Some(on) => {
            d.fields.remove(&kept("percentages"));
            d.fields.insert("percentages".into(), json!(on));
        }
        None => {
            d.fields.insert(kept("percentages"), json!(mode));
            d.fields.insert("percentages".into(), value.clone());
        }
    }
}

/// Use Ratio: the count on the kept side stays and the other makes the tiles square, as the
/// command counts them for the fields as they are (Constrain Ratio's side, the new size and the
/// spacing).
fn use_ratio(app: &VectorcraftApp, d: &mut Dialog) {
    let mut p = params(d);
    p["useRatio"] = json!(if keeps_width(d) { "width" } else { "height" });
    match vectorcraft_engine::cmd::mosaic_counts(&app.session, &p) {
        Ok((cols, rows)) => {
            d.fields.remove(form::REFUSED);
            d.fields.insert("columns".into(), json!(cols));
            d.fields.insert("rows".into(), json!(rows));
        }
        Err(e) => form::note_refusal(d, &e),
    }
}

/// Field `key` as `object.createObjectMosaic` takes it: a number (text with a unit, as
/// `ui.dialog.set` may give it, read in points), `default` when the field is missing, and any other
/// value as it is, for the command to refuse.
fn number(d: &Dialog, key: &str, default: f64) -> Value {
    match d.fields.get(key) {
        None => json!(default),
        Some(Value::String(s)) => vectorcraft_doc::Unit::Points.parse(s).map_or_else(|| json!(s), |v| json!(v)),
        Some(v) => v.clone(),
    }
}

/// Field `key` as the command takes an on/off value: false when the field is missing, and any
/// other value as it is, for the command to refuse one that is not true or false.
fn switch(d: &Dialog, key: &str) -> Value {
    d.fields.get(key).cloned().unwrap_or(json!(false))
}

/// `constrain` as the command takes it: null with Constrain Ratio off, else the side the field
/// holds (the width when it holds none), and any other value as it is, for the command to refuse.
fn constrained_side(d: &Dialog) -> Value {
    if !d.bool("lock") {
        return Value::Null;
    }
    match d.fields.get("constrain") {
        None | Some(Value::Null) => json!("width"),
        Some(v) => v.clone(),
    }
}

/// `object.createObjectMosaic` parameters from the fields. With Constrain Ratio on, the side it
/// sets is left out while it holds the size Constrain Ratio gives it: the command works it out from
/// the kept side and grows both in proportion to its 0.1 pt minimum. Any other value set on that
/// side, and a refused value `ui.dialog.set` gave it even when it equals that size, is passed on for
/// the command to check.
fn params(d: &Dialog) -> Value {
    let mut p = json!({
        "columns": number(d, "columns", 10.0), "rows": number(d, "rows", 10.0),
        "width": number(d, "width", 100.0), "height": number(d, "height", 100.0), "percentages": switch(d, "percentages"),
        "constrain": constrained_side(d),
        "spacingX": number(d, "spacingX", 0.0), "spacingY": number(d, "spacingY", 0.0),
        "gray": switch(d, "gray"), "deleteRaster": switch(d, "deleteRaster"),
    });
    if let (Some((key, v)), Some(o)) = (derived(d), p.as_object_mut())
        && !d.bool(&kept(key))
        && o.get(key).and_then(Value::as_f64).is_some_and(|x| x == v)
    {
        o.remove(key);
    }
    p
}

/// A Width and a Height field for `keys`: lengths in `unit` of at least `least` points, or
/// percentages with `pct`, up to 1,000,000. With `follows`, the field of that key is grayed (Constrain Ratio sets it).
fn pair(ui: &mut egui::Ui, d: &mut Dialog, keys: [&str; 2], unit: vectorcraft_doc::Unit, (pct, least): (bool, f64), follows: Option<&str>) {
    for (key, label) in keys.into_iter().zip([tl!("Width:"), tl!("Height:")]) {
        widgets::dim_label(ui, label);
        ui.add_enabled_ui(follows != Some(key), |ui| {
            if pct {
                if let Some(v) = widgets::spin_plain(ui, ("mosaic", key), d.f64(key, 100.0), "%", 2, FIELD_W, 1.0, 0.1, &[]) {
                    d.fields.insert(key.into(), json!(v.min(*SIZE_RANGE.end())));
                }
            } else {
                form::length_within(ui, d, key, unit, FIELD_W, Some(least..=1.0e6));
            }
        });
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let unit = app.session.general_unit();
    read_image(image_size(app), d);
    let (iw, ih) = image(d);
    widgets::label_row(ui, tl!("Current Size:"), LABEL_W, |ui| {
        widgets::dim_label(ui, tl!("Width:"));
        widgets::dim_name(ui, &unit.format(iw));
        widgets::dim_label(ui, tl!("Height:"));
        widgets::dim_name(ui, &unit.format(ih));
    });
    ui.add_space(4.0);
    let follows = follower(d);
    let pct = in_percentages(d);
    widgets::label_row(ui, tl!("New Size:"), LABEL_W, |ui| pair(ui, d, ["width", "height"], unit, (pct, 0.1), follows));
    follow_ratio(d);
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Tile Spacing:"), LABEL_W, |ui| pair(ui, d, ["spacingX", "spacingY"], unit, (false, 0.0), None));
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Number of Tiles:"), LABEL_W, |ui| {
        for (key, label) in [("columns", tl!("Width:")), ("rows", tl!("Height:"))] {
            widgets::dim_label(ui, label);
            if let Some(v) = widgets::spin_plain(ui, ("mosaic", key), d.f64(key, 10.0), "", 0, FIELD_W, 1.0, 1.0, &[]) {
                d.fields.insert(key.into(), json!(v.round().clamp(1.0, 1000.0)));
            }
        }
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Constrain Ratio:"), LABEL_W, |ui| {
        form::check(ui, d, "lock", tl!("Constrain Width and Height Proportions"));
        let width = keeps_width(d);
        if widgets::radio(ui, tl!("Width"), width, true) {
            d.fields.insert("constrain".into(), json!("width"));
        }
        if widgets::radio(ui, tl!("Height"), !width, true) {
            d.fields.insert("constrain".into(), json!("height"));
        }
    });
    follow_ratio(d);
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Result:"), LABEL_W, |ui| {
        let gray = d.bool("gray");
        if widgets::radio(ui, tl!("Color"), !gray, true) {
            d.fields.insert("gray".into(), json!(false));
        }
        if widgets::radio(ui, tl!("Grayscale"), gray, true) {
            d.fields.insert("gray".into(), json!(true));
        }
    });
    ui.add_space(4.0);
    widgets::label_row(ui, tl!("Options:"), LABEL_W, |ui| {
        ui.vertical(|ui| {
            if widgets::check(ui, tl!("Resize Using Percentages"), pct, true) {
                switch_percentages(d, !pct);
            }
            form::check(ui, d, "deleteRaster", tl!("Delete Raster"));
        });
    });
    if let Some(why) = d.fields.get(form::REFUSED).and_then(Value::as_str) {
        ui.add_space(4.0);
        ui.add(egui::Label::new(egui::RichText::new(why).color(Tokens::get(ui.ctx()).warning)).wrap());
    }
    ui.add_space(6.0);
    if widgets::flat_button(ui, tl!("Use Ratio"), 96.0).clicked() {
        use_ratio(app, d);
    }
    false
}

/// OK: the new size worked out against the image's size as it is now, as a frame would show it.
/// A Constrain Ratio (`lock`) that is not true or false is refused, and the dialog stays open. The
/// dialog also stays open with its fields when the command refuses them, as [`form::commit_preview`]
/// does.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.fields.get("lock").is_some_and(|v| !v.is_boolean() && !v.is_null()) {
        return Err("`lock` must be true or false".into());
    }
    let mut d = d.clone();
    read_image(image_size(app), &mut d);
    follow_ratio(&mut d);
    let r = app.run(COMMAND, params(&d));
    if r.is_ok() {
        app.ui.dialog = None;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// A frame of the open dialog → the texts painted. The dialog measures itself on its first
    /// frame, so a fresh context needs two.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<String> {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
        out.shapes.iter().filter_map(|s| if let egui::epaint::Shape::Text(t) = &s.shape { Some(t.galley.text().to_string()) } else { None }).collect()
    }

    fn context() -> egui::Context {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    }

    /// A tile count the dialog holds.
    fn count(d: &Dialog, key: &str) -> u32 {
        d.f64(key, 0.0) as u32
    }

    /// An app with a 100 × 50 pt embedded image selected.
    fn app_with_image() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        app.run("paint.setStroke", json!({"none": true})).unwrap();
        app.run("object.rasterize", json!({"ppi": 72})).unwrap();
        app
    }

    /// The mosaic group's size and tile count.
    fn mosaic(app: &VectorcraftApp) -> Option<(f64, f64, usize)> {
        let mut found = None;
        app.session.active()?.doc.walk(|n| {
            if n.name.as_deref() == Some("Object Mosaic") {
                let b = n.geometric_bounds().unwrap();
                found = Some(((b.width() * 1e6).round() / 1e6, (b.height() * 1e6).round() / 1e6, n.children().map_or(0, |c| c.len())));
            }
        });
        found
    }

    fn open_from_menu(app: &mut VectorcraftApp) {
        let entry = crate::menus::menu_entries(app).into_iter().find(|e| e.label == "Create Object Mosaic…").expect("menu item");
        let (cmd, params) = crate::menus::click_target(&entry.label, entry.command.as_deref().unwrap(), &entry.params);
        crate::menus::invoke(app, &cmd, params);
    }

    /// The menu item opens this dialog, not the generic one, on the image's size, with every documented row; OK
    /// makes the mosaic as one undo step.
    #[test]
    fn the_menu_item_opens_the_dialog_with_its_rows() {
        let mut app = app_with_image();
        open_from_menu(&mut app);
        let d = app.ui.dialog.as_ref().expect("a dialog");
        assert_eq!(d.kind, KIND);
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0), d.f64("columns", 0.0), d.f64("rows", 0.0)), (100.0, 50.0, 10.0, 10.0));
        let ctx = context();
        frame(&mut app, &ctx);
        let texts = frame(&mut app, &ctx);
        for want in [
            "Create Object Mosaic",
            "Current Size:",
            "New Size:",
            "Tile Spacing:",
            "Number of Tiles:",
            "Constrain Ratio:",
            "Constrain Width and Height Proportions",
            "Result:",
            "Color",
            "Grayscale",
            "Resize Using Percentages",
            "Delete Raster",
            "Use Ratio",
            "100 pt",
            "50 pt",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want:?} in {texts:?}");
        }
        let undo = app.session.doc().unwrap().history.undo.len();
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(mosaic(&app), Some((100.0, 50.0, 100)));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    }

    /// An image narrower than 0.1 pt opens with a new width of 0.1 pt, which OK makes.
    #[test]
    fn a_tiny_image_opens_with_a_size_the_command_takes() {
        let mut app = app_with_image();
        app.run("object.scale", json!({"sx": 0.05, "sy": 100})).unwrap();
        open(&mut app, &json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("width", 0.0), 0.1);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| m.0), Some(0.1));
    }

    /// With no image selected the dialog does not open and the error says why.
    #[test]
    fn without_an_image_nothing_opens() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 50})).unwrap();
        let e = open(&mut app, &json!({})).unwrap_err();
        assert!(app.ui.dialog.is_none());
        assert!(e.contains("select an image"), "{e}");
    }

    /// Constrain Ratio keeps the new size in the image's proportions from the side it names; Resize Using
    /// Percentages shows the same size as percentages; Use Ratio makes the tiles square from the kept side's count.
    #[test]
    fn constrain_ratio_percentages_and_use_ratio_set_the_fields() {
        let mut app = app_with_image();
        open(&mut app, &json!({"columns": 10, "rows": 10})).unwrap();
        let ctx = context();
        frame(&mut app, &ctx);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("lock".into(), json!(true));
        d.fields.insert("width".into(), json!(300));
        frame(&mut app, &ctx);
        let mut d = app.ui.dialog.take().unwrap();
        assert_eq!(d.f64("height", 0.0), 150.0, "height follows the width");
        // Percentages: 300 × 150 pt of a 100 × 50 pt image is 300%.
        switch_percentages(&mut d, true);
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0)), (300.0, 300.0));
        // Use Ratio: 10 columns across 300 pt are 30 pt tiles, so 5 rows down 150 pt.
        use_ratio(&app, &mut d);
        assert_eq!((count(&d, "columns"), count(&d, "rows")), (10, 5));
        // Kept by height: 4 rows of 37.5 pt over 300 pt are 8 columns.
        d.fields.insert("constrain".into(), json!("height"));
        d.fields.insert("rows".into(), json!(4));
        use_ratio(&app, &mut d);
        assert_eq!((count(&d, "columns"), count(&d, "rows")), (8, 4));
        switch_percentages(&mut d, false);
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0)), (300.0, 150.0));
        d.fields.insert("gray".into(), json!(true));
        app.ui.dialog = Some(d);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app), Some((300.0, 150.0, 32)));
    }

    /// With Constrain Ratio on, a kept width of 0.1 pt over a 100 × 50 pt image sets a height of
    /// 0.05 pt, under the command's minimum: OK makes the 0.2 × 0.1 pt mosaic the command grows it to.
    #[test]
    fn a_constrained_size_under_the_minimum_grows_in_proportion() {
        let mut app = app_with_image();
        open(&mut app, &json!({})).unwrap();
        let ctx = context();
        frame(&mut app, &ctx);
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("lock".into(), json!(true));
        d.fields.insert("width".into(), json!(0.1));
        frame(&mut app, &ctx);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("height", 0.0), 0.05, "height follows the width");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((0.2, 0.1)));
    }

    /// A caller's params open the dialog with their values, and OK makes the mosaic they ask for.
    #[test]
    fn a_callers_params_open_the_dialog_with_their_values() {
        let mut app = app_with_image();
        let params = json!({"columns": 4, "rows": 2, "width": 200, "percentages": true, "constrain": "width", "gray": true});
        app.run("ui.paramDialog", json!({"command": COMMAND, "label": "Create Object Mosaic", "params": params})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!(d.kind, KIND);
        assert_eq!(
            (d.f64("width", 0.0), d.bool("percentages"), d.bool("lock"), d.str("constrain"), d.bool("gray")),
            (200.0, true, true, "width".into(), true)
        );
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app), Some((200.0, 100.0, 8)));
        // Use Ratio given applies to the counts shown: 10 columns over 100 × 50 pt are 5 rows.
        app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
        let params = json!({"columns": 10, "rows": 3, "useRatio": true});
        app.run("ui.paramDialog", json!({"command": COMMAND, "params": params})).unwrap();
        assert_eq!(count(app.ui.dialog.as_ref().unwrap(), "rows"), 5);
        // A side Use Ratio names is the side the dialog keeps: 4 rows over 100 × 50 pt are 8 columns.
        let params = json!({"columns": 3, "rows": 4, "useRatio": "height"});
        app.run("ui.paramDialog", json!({"command": COMMAND, "params": params})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((count(d, "columns"), count(d, "rows"), d.str("constrain"), d.bool("lock")), (8, 4, "height".into(), false));
        app.ui.dialog = None;
        // Params the command refuses open nothing and say why.
        for (bad, key) in [(json!({"width": "abc"}), "width"), (json!({"constrain": "both"}), "constrain"), (json!({"widht": 200}), "widht")] {
            let e = app.run("ui.paramDialog", json!({"command": COMMAND, "params": bad})).unwrap_err();
            assert!(app.ui.dialog.is_none(), "{e}");
            assert!(e.contains(key), "{e}");
        }
    }

    /// `ui.dialog.set` of Resize Using Percentages shows the new size the other way, as the checkbox
    /// does, so OK makes the same mosaic. Values the command refuses, set that way, are refused on OK
    /// and make no mosaic.
    #[test]
    fn setting_fields_from_outside_keeps_the_size_or_is_refused() {
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 200, "height": 100})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "percentages".into(), json!(true));
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0), d.bool("percentages")), (200.0, 200.0, true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((200.0, 100.0)));
        app.run("edit.undo", json!({})).unwrap();
        // 0.1 × 0.1 pt of a 1000 × 500 pt image are 0.01% × 0.02%, under the command's 0.1%: the
        // points stay, with the limit shown, and OK makes that size.
        app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
        app.run("object.scale", json!({"sx": 1000, "sy": 1000})).unwrap();
        open(&mut app, &json!({"width": 0.1, "height": 0.1})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "percentages".into(), json!(true));
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0), d.bool("percentages")), (0.1, 0.1, false));
        assert!(d.str(form::REFUSED).contains("`width` must be between 0.1"), "{}", d.str(form::REFUSED));
        let ctx = context();
        frame(&mut app, &ctx);
        let texts = frame(&mut app, &ctx);
        assert!(texts.iter().any(|t| t.contains("must be between")), "the limit is shown: {texts:?}");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((0.1, 0.1)));
        app.run("edit.undo", json!({})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        for (field, value) in [("percentages", json!("yes")), ("width", json!("abc")), ("gray", json!(1))] {
            app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
            open(&mut app, &json!({})).unwrap();
            crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), field.into(), value);
            let e = crate::dialogs::confirm(&mut app).unwrap_err();
            assert!(e.contains(field), "{field}: {e}");
            assert_eq!(mosaic(&app), None, "{field}");
        }
        // With Constrain Ratio on, a value the command refuses, set on the side it sets, is refused
        // too, also after a frame.
        for value in [json!("abc"), json!(0)] {
            app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
            open(&mut app, &json!({"constrain": "width"})).unwrap();
            crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "height".into(), value.clone());
            frame(&mut app, &ctx);
            let e = crate::dialogs::confirm(&mut app).unwrap_err();
            assert!(e.contains("height"), "{value}: {e}");
            assert_eq!(mosaic(&app), None, "{value}");
        }
        // Once that side holds a size the command takes, Constrain Ratio sets it again.
        app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
        open(&mut app, &json!({"constrain": "width"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        for (field, value) in [("height", json!(0)), ("lock", json!(false)), ("height", json!(80)), ("lock", json!(true))] {
            crate::dialogs::set_field(d, field.into(), value);
        }
        frame(&mut app, &ctx);
        assert_eq!(app.ui.dialog.as_ref().unwrap().f64("height", 0.0), 50.0);
        // A refused height stays while a refused width is set and then corrected.
        let d = app.ui.dialog.as_mut().unwrap();
        for (field, value) in [("height", json!(0)), ("width", json!(0)), ("width", json!(100))] {
            crate::dialogs::set_field(d, field.into(), value);
        }
        frame(&mut app, &ctx);
        let e = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(e.contains("height"), "{e}");
        assert_eq!(mosaic(&app), None);
    }

    /// A width `ui.dialog.set` gave that is not a number stays when Resize Using Percentages is set,
    /// with the mode unchanged and the reason shown, and OK refuses it.
    #[test]
    fn a_size_that_is_not_a_number_stays_when_percentages_switch() {
        let mut app = app_with_image();
        open(&mut app, &json!({})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "width".into(), json!("abc"));
        crate::dialogs::set_field(d, "percentages".into(), json!(true));
        assert_eq!((d.str("width"), d.bool("percentages")), ("abc".into(), false));
        assert!(d.str(form::REFUSED).contains("`width` must be a number"), "{}", d.str(form::REFUSED));
        let e = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(e.contains("width"), "{e}");
        assert_eq!(mosaic(&app), None);
    }

    /// A size `ui.dialog.set` gave that the command refuses stays when Resize Using Percentages
    /// switches, even when it is in range the other way (0.05 pt of a 10 pt wide image is 0.5%, and
    /// 0.05% of a 1,000 pt wide image is 0.5 pt), with the mode unchanged and the reason shown, and
    /// OK refuses it.
    #[test]
    fn a_size_out_of_range_stays_when_percentages_switch() {
        for (scale, given, on) in [(10, json!({}), true), (1000, json!({"percentages": true}), false)] {
            let mut app = app_with_image();
            app.run("object.scale", json!({"sx": scale, "sy": scale})).unwrap();
            open(&mut app, &given).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            crate::dialogs::set_field(d, "width".into(), json!(0.05));
            crate::dialogs::set_field(d, "percentages".into(), json!(on));
            assert_eq!((d.f64("width", 0.0), d.bool("percentages")), (0.05, !on), "{given}");
            assert!(d.str(form::REFUSED).contains("`width` must be between 0.1"), "{given}: {}", d.str(form::REFUSED));
            let e = crate::dialogs::confirm(&mut app).unwrap_err();
            assert!(e.contains("width"), "{given}: {e}");
            assert_eq!(mosaic(&app), None, "{given}");
        }
    }

    /// The image scaled from 100 × 50 pt to 200 × 100 pt while the dialog is open: Current Size shows
    /// the new size, and Resize Using Percentages, by `ui.dialog.set` or by its checkbox, converts
    /// New Size against it, so OK makes the size shown. With no image selected, switching is refused.
    #[test]
    fn percentages_convert_against_the_image_scaled_while_the_dialog_is_open() {
        let set = |app: &mut VectorcraftApp, field: &str, value: Value| {
            let (req, _) = crate::control::ControlRequest::new("ui.dialog.set", json!({"field": field, "value": value}));
            let crate::control::Outcome::Done(r) = crate::control::handle(app, &egui::Context::default(), &req) else {
                panic!("expected an immediate reply")
            };
            r
        };
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 200, "height": 100})).unwrap();
        app.run("object.scale", json!({"sx": 200, "sy": 200})).unwrap();
        let ctx = context();
        frame(&mut app, &ctx);
        let texts = frame(&mut app, &ctx);
        assert!(texts.iter().any(|t| t == "200 pt"), "Current Size shows the scaled image: {texts:?}");
        let r = set(&mut app, "percentages", json!(true));
        assert_eq!(r["ok"], json!(true), "{r}");
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0), d.bool("percentages")), (100.0, 100.0, true));
        app.run("object.scale", json!({"sx": 50, "sy": 50})).unwrap();
        // The checkbox, after the image went back to 100 × 50 pt: 100% of it is 100 × 50 pt.
        let size = image_size(&app);
        let d = app.ui.dialog.as_mut().unwrap();
        read_image(size, d);
        switch_percentages(d, false);
        assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0)), (100.0, 50.0));
        crate::dialogs::set_field(d, "percentages".into(), json!(true));
        app.run("object.scale", json!({"sx": 200, "sy": 200})).unwrap();
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((200.0, 100.0)), "100% of the image as it is on OK");
        app.run("edit.undo", json!({})).unwrap();
        // No image selected: the mode and sizes stay and the dialog asks for an image.
        app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
        open(&mut app, &json!({})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        set(&mut app, "percentages", json!(true));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("width", 0.0), d.bool("percentages")), (200.0, false));
        assert_eq!(d.str(form::REFUSED), NO_IMAGE_WHY);
        app.run("select.set", json!({"ids": [image_id(&app)]})).unwrap();
        set(&mut app, "percentages", json!(true));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("width", 0.0), d.bool("percentages"), d.str(form::REFUSED)), (100.0, true, String::new()));
    }

    /// A Constrain Ratio side other than width or height, or a Constrain Ratio that is not true or
    /// false, set by `ui.dialog.set`, is refused on OK and makes no mosaic.
    #[test]
    fn a_constrain_ratio_value_the_command_refuses_is_refused() {
        for (field, value, also) in [("constrain", json!("both"), Some(("lock", json!(true)))), ("lock", json!("yes"), None)] {
            let mut app = app_with_image();
            open(&mut app, &json!({})).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            if let Some((k, v)) = also {
                crate::dialogs::set_field(d, k.into(), v);
            }
            crate::dialogs::set_field(d, field.into(), value.clone());
            let e = crate::dialogs::confirm(&mut app).unwrap_err();
            assert!(e.contains(field), "{value}: {e}");
            assert_eq!(mosaic(&app), None, "{value}");
        }
    }

    /// When the command refuses the dialog's fields, OK leaves the dialog open with them, so they can
    /// be corrected; OK then makes the mosaic and closes it.
    #[test]
    fn a_refused_field_keeps_the_dialog_open() {
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 80})).unwrap();
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "gray".into(), json!(1));
        let e = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(e.contains("gray"), "{e}");
        assert_eq!(mosaic(&app), None);
        let d = app.ui.dialog.as_mut().expect("the dialog stays open");
        assert_eq!((d.f64("width", 0.0), d.fields.get("gray")), (80.0, Some(&json!(1))));
        crate::dialogs::set_field(d, "gray".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(mosaic(&app).map(|m| m.0), Some(80.0));
    }

    /// True or false set after a Resize Using Percentages value the command refuses replaces it: the
    /// same mode as before keeps the sizes, and the other mode shows them the other way. Null is
    /// false, as the command reads it. OK then makes the mosaic.
    #[test]
    fn true_or_false_replaces_a_refused_percentages_value() {
        let pct = json!({"percentages": true, "width": 50, "height": 50});
        for (given, sets, size) in [
            (json!({}), vec![json!("yes"), json!(false)], (100.0, 50.0)),
            (pct.clone(), vec![json!("yes"), json!(false)], (50.0, 25.0)),
            (pct.clone(), vec![Value::Null], (50.0, 25.0)),
            (pct, vec![json!("yes"), Value::Null], (50.0, 25.0)),
        ] {
            let mut app = app_with_image();
            open(&mut app, &given).unwrap();
            let d = app.ui.dialog.as_mut().unwrap();
            for value in sets {
                crate::dialogs::set_field(d, "percentages".into(), value);
            }
            assert_eq!(d.fields.get("percentages"), Some(&json!(false)), "{given}");
            assert_eq!((d.f64("width", 0.0), d.f64("height", 0.0)), size, "{given}");
            crate::dialogs::confirm(&mut app).unwrap();
            assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some(size), "{given}");
        }
    }

    /// With Constrain Ratio on, a refused height `ui.dialog.set` gave is refused on OK even when it
    /// equals the size Constrain Ratio works out from the width (0.05 pt from 0.1 pt over 100 × 50 pt).
    #[test]
    fn a_refused_height_equal_to_the_constrained_size_is_refused() {
        let mut app = app_with_image();
        open(&mut app, &json!({"constrain": "width"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "width".into(), json!(0.1));
        crate::dialogs::set_field(d, "height".into(), json!(0.05));
        let e = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(e.contains("height"), "{e}");
        assert_eq!(mosaic(&app), None);
    }

    /// A percentage typed in New Size above 1,000,000 is kept at 1,000,000, the most the command
    /// takes, so Enter makes the mosaic: 1,000,000% of the 100 pt width is 1,000,000 pt.
    #[test]
    fn a_typed_percentage_stays_within_what_the_command_takes() {
        let mut app = app_with_image();
        open(&mut app, &json!({"percentages": true})).unwrap();
        let ctx = context();
        let run = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| super::super::show(app, ui.ctx()));
            out.textures_delta.clear();
            out.shapes
                .iter()
                .filter_map(|s| {
                    if let egui::epaint::Shape::Text(t) = &s.shape {
                        Some((t.galley.text().to_string(), t.visual_bounding_rect().center()))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        run(&mut app, vec![]);
        let texts = run(&mut app, vec![]);
        // New Size's Width field comes right after the row's Width caption.
        let row = texts.iter().position(|(t, _)| t == "New Size:").expect("the New Size row");
        let width = texts[row..].iter().position(|(t, _)| t == "Width:").and_then(|i| texts.get(row + i + 1));
        assert_eq!(width.map(|(t, _)| t.as_str()), Some("100%"), "{texts:?}");
        let at = width.map(|(_, p)| *p).unwrap();
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, vec![egui::Event::PointerMoved(at), press(true)]);
        run(&mut app, vec![press(false)]);
        run(&mut app, vec![]);
        run(&mut app, vec![egui::Event::Text("2000000".into())]);
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        run(&mut app, vec![enter]);
        assert!(app.ui.dialog.is_none(), "Enter makes the mosaic: {:?}", app.ui.dialog.as_ref().map(|d| d.str(form::REFUSED)));
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((1e6, 50.0)));
    }

    /// The id of the image in the document.
    fn image_id(app: &VectorcraftApp) -> u64 {
        let mut id = None;
        app.session.active().unwrap().doc.walk(|n| {
            if matches!(n.kind, NodeKind::Image(_)) {
                id = Some(n.id.0);
            }
        });
        id.unwrap()
    }

    /// #1000: the count the dialog holds reaches the command, so a count of 3.0 makes three columns.
    #[test]
    fn a_count_reaches_the_command() {
        let mut app = app_with_image();
        open_from_menu(&mut app);
        app.ui.dialog.as_mut().unwrap().fields.insert("columns".into(), json!(3.0));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(mosaic(&app).map(|m| m.2), Some(30));
    }
}
