//! Object › Create Object Mosaic…: the selected image's Current Size, the mosaic's New Size, Tile
//! Spacing and Number of Tiles, Constrain Ratio (Width or Height), Result (Color or Grayscale),
//! Resize Using Percentages, Delete Raster and Use Ratio. OK runs `object.createObjectMosaic` as one
//! undo step.
//!
//! The dialog is a `StackDialog` on the window's dialog host. [`sync`] pushes the selected image's
//! size and the document's length units into it before each frame and before each `ui.dialog.*`
//! method, so New Size is converted and constrained against the image as it is now, also after it was
//! scaled while the dialog was open. [`pump`] runs what the dialog asked for outside the host's lock:
//! Use Ratio's counts and the held OK, which the command may refuse.
//!
//! Fields (`ui.dialog.set`): `width`, `height` (pt, or percentages of the image's size with
//! `percentages`), `spacingX`, `spacingY` (pt), `columns`, `rows`, `lock` (Constrain Ratio on),
//! `constrain` (`width` | `height`: the side Constrain Ratio and Use Ratio keep), `gray`,
//! `percentages` and `deleteRaster`.

use std::ops::RangeInclusive;

use craft::theme::variant::Tone; // PLAN: vectorcraft-ui-egui's manifest needs the `craft` facade dependency; `craft_ui` does not re-export `Tone`.
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, StackDialog, dialog_id};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::{Env, Label, LengthUnits};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::sections::GroupBox;
use craft::ui::text::{NumberField, StepArrows};
use egui::{Context, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use crate::VectorcraftApp;

/// The dialog kind of Create Object Mosaic.
pub const KIND: &str = "objectMosaic";

/// The command the dialog runs.
pub const COMMAND: &str = "object.createObjectMosaic";

/// What the dialog says while no image is selected.
const NO_IMAGE: &str = "select an image";

/// The new sizes the command takes, in points or percentages (`object.createObjectMosaic`).
const SIZE_RANGE: RangeInclusive<f64> = 0.1..=1.0e6;

/// The tile counts the command takes.
const COUNT_RANGE: RangeInclusive<f64> = 1.0..=1000.0;

/// A side of the mosaic.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Side {
    #[default]
    Width,
    Height,
}

impl Side {
    /// The side's field name and the command's spelling of it.
    fn key(self) -> &'static str {
        match self {
            Side::Width => "width",
            Side::Height => "height",
        }
    }

    fn other(self) -> Side {
        match self {
            Side::Width => Side::Height,
            Side::Height => Side::Width,
        }
    }
}

/// The values the dialog edits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mosaic {
    width: f64,
    height: f64,
    spacing_x: f64,
    spacing_y: f64,
    columns: f64,
    rows: f64,
    /// Constrain Ratio.
    lock: bool,
    /// The side Constrain Ratio and Use Ratio keep.
    side: Side,
    gray: bool,
    percentages: bool,
    delete_raster: bool,
}

/// Create Object Mosaic on the dialog host.
pub struct ObjectMosaicDialog {
    id: DialogId,
    values: Mosaic,
    /// The size (pt) of the image as of the last [`sync`]; the last size read while none is selected.
    image: (f64, f64),
    /// Whether an image is selected.
    selected: bool,
    lengths: LengthUnits,
    /// What the dialog says under its rows: a refused switch or a refused Use Ratio.
    note: Option<Label>,
    /// Use Ratio was pressed; [`pump`] works out the counts.
    use_ratio: bool,
    /// OK, until [`pump`] has run the command on it.
    held: HeldAnswer<Mosaic>,
}

fn label(text: &str) -> Label {
    text.to_owned().into()
}

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
    let Some((w, h)) = image_size(app) else { return Err(NO_IMAGE.into()) };
    let none = json!({});
    let given = if given.is_null() { &none } else { given };
    app.session.check_params(COMMAND, given).map_err(|e| e.to_string())?;
    let (cols, rows) = vectorcraft_engine::cmd::mosaic_counts(&app.session, given).map_err(|e| e.to_string())?;
    let n = |key: &str, d: f64| given.get(key).and_then(Value::as_f64).unwrap_or(d);
    let b = |key: &str| given.get(key).and_then(Value::as_bool).unwrap_or(false);
    let pct = b("percentages");
    // The new size starts at the image's, within the sizes the command takes (0.1 to 1,000,000 pt).
    let (base_w, base_h) = if pct { (100.0, 100.0) } else { (w.clamp(0.1, 1.0e6), h.clamp(0.1, 1.0e6)) };
    let named = |key: &str| match given.get(key).and_then(Value::as_str) {
        Some("width") => Some(Side::Width),
        Some("height") => Some(Side::Height),
        _ => None,
    };
    let constrain = named("constrain");
    // The side the dialog keeps: Constrain Ratio's, else the one Use Ratio names.
    let side = constrain.or(named("useRatio")).unwrap_or_default();
    let values = Mosaic {
        width: n("width", base_w),
        height: n("height", base_h),
        spacing_x: n("spacingX", 0.0),
        spacing_y: n("spacingY", 0.0),
        columns: cols as f64,
        rows: rows as f64,
        lock: constrain.is_some(),
        side,
        gray: b("gray"),
        percentages: pct,
        delete_raster: b("deleteRaster"),
    };
    let dialogs = app.dialogs().ok_or("no window")?;
    let lengths = crate::canvas::general_lengths(app);
    dialogs.with(|host| host.open(ObjectMosaicDialog::new(values, (w, h), lengths)));
    Ok(json!({ "dialog": KIND }))
}

/// Pushes the selected image's size and the document's length units into the open dialog. A frame runs
/// it before the dialogs, and `ui.dialog.*` before each method.
pub(crate) fn sync(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let size = image_size(app);
    let lengths = crate::canvas::general_lengths(app);
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<ObjectMosaicDialog>() {
            dialog.sync(size, lengths);
        }
    });
}

/// What the dialog asked of the app.
enum Work {
    /// Use Ratio: the counts for these values.
    Ratio(Mosaic, (f64, f64)),
    /// The held OK: the command for these values.
    Run(Mosaic, (f64, f64)),
}

/// Runs what the open dialog asked of the app, outside the host's lock: Use Ratio's counts, and the
/// held OK, whose command may refuse (the dialog then stays open with the reason).
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let work = dialogs.with(|host| {
        // Nothing reads the unit answer of a closed dialog.
        let _ = host.take::<ObjectMosaicDialog>();
        let dialog = host.get_mut::<ObjectMosaicDialog>()?;
        if std::mem::take(&mut dialog.use_ratio) {
            return Some(Work::Ratio(dialog.values.clone(), dialog.image));
        }
        dialog.held.take().map(|values| Work::Run(values, dialog.image))
    });
    match work {
        None => {}
        Some(Work::Ratio(mut values, image)) => {
            follow_ratio(&mut values, image);
            let mut p = params(&values, image);
            p["useRatio"] = json!(values.side.key());
            let counts = vectorcraft_engine::cmd::mosaic_counts(&app.session, &p).map_err(|e| e.to_string());
            dialogs.with(|host| {
                let Some(dialog) = host.get_mut::<ObjectMosaicDialog>() else { return };
                match counts {
                    Ok((cols, rows)) => {
                        dialog.values.columns = cols as f64;
                        dialog.values.rows = rows as f64;
                        dialog.note = None;
                    }
                    Err(why) => dialog.note = Some(why.into()),
                }
            });
        }
        Some(Work::Run(mut values, image)) => {
            follow_ratio(&mut values, image);
            // With no image selected the dialog says what it needs, not that the command is unavailable.
            let answer = match image_size(app) {
                Some(_) => app.run(COMMAND, params(&values, image)).map(|_| ()).map_err(Label::from),
                None => Err(Label::from(NO_IMAGE.to_string())),
            };
            dialogs.with(|host| {
                if let Some(dialog) = host.get_mut::<ObjectMosaicDialog>() {
                    dialog.held.answer(answer);
                }
            });
        }
    }
}

impl ObjectMosaicDialog {
    fn new(values: Mosaic, image: (f64, f64), lengths: LengthUnits) -> Self {
        ObjectMosaicDialog { id: dialog_id(KIND), values, image, selected: true, lengths, note: None, use_ratio: false, held: HeldAnswer::new() }
    }

    /// `size` is the selected image's size, `None` while none is selected: the last size stays and a
    /// switch to percentages is refused until one is.
    fn sync(&mut self, size: Option<(f64, f64)>, lengths: LengthUnits) {
        self.lengths = lengths;
        self.selected = size.is_some();
        if let Some(size) = size {
            self.image = size;
            if self.note.as_ref().is_some_and(|n| n.as_str() == NO_IMAGE) {
                self.note = None;
            }
        }
    }
}

/// The image's size with no zero side, for dividing by.
fn clean((w, h): (f64, f64)) -> (f64, f64) {
    (w.max(1e-9), h.max(1e-9))
}

/// With Constrain Ratio on, the side that follows the kept side.
fn follower(m: &Mosaic) -> Option<Side> {
    m.lock.then(|| m.side.other())
}

/// With Constrain Ratio on, the side that follows and the size it gets from the side that leads.
fn derived(m: &Mosaic, image: (f64, f64)) -> Option<(Side, f64)> {
    let (iw, ih) = clean(image);
    Some(match follower(m)? {
        Side::Height => (Side::Height, if m.percentages { m.width } else { m.width * ih / iw }),
        Side::Width => (Side::Width, if m.percentages { m.height } else { m.height * iw / ih }),
    })
}

/// With Constrain Ratio on, the side that follows set from the side that leads, in the image's
/// proportions.
fn follow_ratio(m: &mut Mosaic, image: (f64, f64)) {
    match derived(m, image) {
        Some((Side::Width, v)) => m.width = v,
        Some((Side::Height, v)) => m.height = v,
        None => {}
    }
}

/// Resize Using Percentages turned on or off: the new size shown the other way, so it stays the
/// same size. When a size (all but the side Constrain Ratio sets) falls outside what the command
/// accepts either as it is or the other way, nothing changes and the error says why. With no image
/// (`None`), nothing changes and the error asks for one.
fn switch_percentages(m: &mut Mosaic, image: Option<(f64, f64)>, on: bool) -> Result<(), String> {
    let Some(image) = image else { return Err(NO_IMAGE.into()) };
    if m.percentages == on {
        return Ok(());
    }
    let (iw, ih) = clean(image);
    let skip = follower(m);
    let refused = |w: f64, h: f64| {
        let (side, v) = [(Side::Width, w), (Side::Height, h)].into_iter().find(|(side, v)| skip != Some(*side) && !SIZE_RANGE.contains(v))?;
        Some(format!("`{}` must be between {} and {} (got {v})", side.key(), SIZE_RANGE.start(), SIZE_RANGE.end()))
    };
    // A size the command refuses as it is stays refused, even when it falls in range the other way.
    let converted = if on { (m.width / iw * 100.0, m.height / ih * 100.0) } else { (iw * m.width / 100.0, ih * m.height / 100.0) };
    if let Some(why) = refused(m.width, m.height).or_else(|| refused(converted.0, converted.1)) {
        return Err(why);
    }
    (m.width, m.height) = converted;
    m.percentages = on;
    Ok(())
}

/// `constrain` as the command takes it: null with Constrain Ratio off, else the kept side.
fn constrained_side(m: &Mosaic) -> Value {
    if m.lock { json!(m.side.key()) } else { Value::Null }
}

/// `object.createObjectMosaic` parameters from the values. With Constrain Ratio on, the side it sets
/// is left out while it holds the size Constrain Ratio gives it: the command works it out from the
/// kept side and grows both in proportion to its 0.1 pt minimum.
fn params(m: &Mosaic, image: (f64, f64)) -> Value {
    let mut p = json!({
        "columns": m.columns, "rows": m.rows, "width": m.width, "height": m.height, "percentages": m.percentages,
        "constrain": constrained_side(m), "spacingX": m.spacing_x, "spacingY": m.spacing_y,
        "gray": m.gray, "deleteRaster": m.delete_raster,
    });
    if let (Some((side, v)), Some(o)) = (derived(m, image), p.as_object_mut())
        && o.get(side.key()).and_then(Value::as_f64).is_some_and(|x| x == v)
    {
        o.remove(side.key());
    }
    p
}

/// A number or a length text (`"2 cm"`) in points; `None` for anything else, and for a non-finite number.
fn points(value: &Value) -> Option<f64> {
    let v = match value {
        Value::String(s) => vectorcraft_doc::Unit::Points.parse(s)?,
        other => other.as_f64()?,
    };
    v.is_finite().then_some(v)
}

/// What a number field of the dialog holds.
#[derive(Clone)]
enum Kind {
    Length(RangeInclusive<f64>),
    Percent,
    Count,
}

/// A number field named `caption` over `value`: a length in the document's unit, a percentage or a
/// whole count. A value the field cannot take keeps the old one.
fn number(ui: &mut Ui, caption: &Label, value: &mut f64, kind: &Kind, lengths: LengthUnits, enabled: bool) {
    let mut held = Some(*value);
    Field::new(caption.clone()).enabled(enabled).show(ui, |ui, _| {
        let field = match kind.clone() {
            Kind::Length(range) => NumberField::length(caption.clone(), Bind::bound(&mut held), lengths).range(range),
            Kind::Percent => NumberField::new(caption.clone(), Bind::bound(&mut held)).range(SIZE_RANGE).unit("%"),
            Kind::Count => NumberField::new(caption.clone(), Bind::bound(&mut held)).range(COUNT_RANGE).fixed_decimals(0).step(1.0),
        };
        field.step_arrows(StepArrows::Leading).enabled(enabled).show(ui);
    });
    if let Some(v) = held {
        *value = if matches!(kind, Kind::Count) { v.round() } else { v };
    }
}

impl StackDialog for ObjectMosaicDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(COMMAND)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let ObjectMosaicDialog { id: _, values, image, selected, lengths, note, use_ratio, held } = self;
        // A refusal shows until a value it refused changes.
        if held.refusal().is_some_and(|(_, refused)| refused != values) {
            held.clear_refusal();
        }
        let (waiting, accepted, image, selected, lengths) = (held.is_waiting(), held.is_accepted(), *image, *selected, *lengths);
        let shown = FormDialog::new(label(tl!("Create Object Mosaic")), Bind::bound(values)).accept_stays_open(true).ok_enabled(!waiting).show(
            ui,
            |ui, form, m| {
                if accepted {
                    form.request_close_as(CloseReason::Activated);
                }
                let units = Env::of(ui).units();
                let scale = units.length_scale(lengths).unwrap_or(1.0);
                let symbol = units.symbol(lengths.shown);
                let (width, height) = (label(tl!("Width:")), label(tl!("Height:")));
                Field::new(label(tl!("Current Size:"))).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        for (caption, size) in [(&width, image.0), (&height, image.1)] {
                            let shown = (size * scale * 1000.0).round() / 1000.0;
                            Text::new(format!("{} {shown} {symbol}", caption.as_str())).show(ui);
                        }
                    });
                });
                let leads = follower(m);
                let size_kind = if m.percentages { Kind::Percent } else { Kind::Length(SIZE_RANGE) };
                GroupBox::new(label(tl!("New Size:")), None).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        number(ui, &width, &mut m.width, &size_kind, lengths, leads != Some(Side::Width));
                        number(ui, &height, &mut m.height, &size_kind, lengths, leads != Some(Side::Height));
                    });
                });
                GroupBox::new(label(tl!("Tile Spacing:")), None).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        let kind = Kind::Length(0.0..=*SIZE_RANGE.end());
                        number(ui, &width, &mut m.spacing_x, &kind, lengths, true);
                        number(ui, &height, &mut m.spacing_y, &kind, lengths, true);
                    });
                });
                GroupBox::new(label(tl!("Number of Tiles:")), None).show(ui, |ui, _| {
                    Row::new().show(ui, |ui, _| {
                        number(ui, &width, &mut m.columns, &Kind::Count, lengths, true);
                        number(ui, &height, &mut m.rows, &Kind::Count, lengths, true);
                    });
                });
                GroupBox::new(label(tl!("Constrain Ratio:")), None).show(ui, |ui, _| {
                    Checkbox::new(Bind::bound(&mut m.lock)).label(label(tl!("Constrain Width and Height Proportions"))).show(ui);
                    let lock = m.lock;
                    RadioGroup::new(label(tl!("Constrain Ratio:")), Bind::bound(&mut m.side)).show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            Radio::new(Side::Width).label(label(tl!("Width"))).enabled(lock).show(ui);
                            Radio::new(Side::Height).label(label(tl!("Height"))).enabled(lock).show(ui);
                        });
                    });
                });
                GroupBox::new(label(tl!("Result:")), None).show(ui, |ui, _| {
                    RadioGroup::new(label(tl!("Result:")), Bind::bound(&mut m.gray)).show(ui, |ui, _| {
                        Row::new().show(ui, |ui, _| {
                            Radio::new(false).label(label(tl!("Color"))).show(ui);
                            Radio::new(true).label(label(tl!("Grayscale"))).show(ui);
                        });
                    });
                });
                GroupBox::new(label(tl!("Options:")), None).show(ui, |ui, _| {
                    if let Some(on) =
                        Checkbox::new(Bind::given(m.percentages)).label(label(tl!("Resize Using Percentages"))).show(ui).value().copied()
                    {
                        *note = switch_percentages(m, selected.then_some(image), on).err().map(Label::from);
                    }
                    Checkbox::new(Bind::bound(&mut m.delete_raster)).label(label(tl!("Delete Raster"))).show(ui);
                });
                follow_ratio(m, image);
                if let Some(why) = note.clone().or_else(|| held.refusal().map(|(reason, _)| reason.clone())) {
                    Notice::new(Tone::Warn, why).show(ui);
                }
                if Button::new(label(tl!("Use Ratio"))).show(ui).clicked() {
                    *use_ratio = true;
                }
                true
            },
        );
        if shown.picked == Some(0) && !waiting {
            held.file(values.clone());
        }
        match shown.closed {
            Some(_) if accepted => {
                held.finish();
                Some(DialogResult::Ok(()))
            }
            Some(_) => Some(DialogResult::Cancel),
            None => None,
        }
    }

    fn confirm(&mut self, _: &Context) -> Option<()> {
        if self.held.finish().is_some() {
            return Some(());
        }
        if !self.held.is_waiting() {
            self.held.file(self.values.clone());
        }
        None
    }

    fn held_state(&self) -> Option<craft::ui::dialog::HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let m = &self.values;
        Some(json!({
            "width": m.width, "height": m.height, "spacingX": m.spacing_x, "spacingY": m.spacing_y,
            "columns": m.columns, "rows": m.rows, "lock": m.lock, "constrain": m.side.key(),
            "gray": m.gray, "percentages": m.percentages, "deleteRaster": m.delete_raster,
        }))
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        let m = &mut self.values;
        match field {
            "width" | "height" => {
                let side = if field == "width" { Side::Width } else { Side::Height };
                if follower(m) == Some(side) {
                    return Err(FieldRefusal::Inactive.message(field));
                }
                let v = points(&value).filter(|v| SIZE_RANGE.contains(v)).ok_or_else(rejects)?;
                match side {
                    Side::Width => m.width = v,
                    Side::Height => m.height = v,
                }
            }
            "spacingX" | "spacingY" => {
                let v = points(&value).filter(|v| (0.0..=*SIZE_RANGE.end()).contains(v)).ok_or_else(rejects)?;
                if field == "spacingX" {
                    m.spacing_x = v;
                } else {
                    m.spacing_y = v;
                }
            }
            "columns" | "rows" => {
                let v = value.as_f64().filter(|v| v.fract() == 0.0 && COUNT_RANGE.contains(v)).ok_or_else(rejects)?;
                if field == "columns" {
                    m.columns = v;
                } else {
                    m.rows = v;
                }
            }
            "lock" => m.lock = value.as_bool().ok_or_else(rejects)?,
            "constrain" => {
                m.side = match value.as_str() {
                    Some("width") => Side::Width,
                    Some("height") => Side::Height,
                    _ => return Err(rejects()),
                }
            }
            "gray" => m.gray = value.as_bool().ok_or_else(rejects)?,
            "deleteRaster" => m.delete_raster = value.as_bool().ok_or_else(rejects)?,
            "percentages" => {
                // The command reads null as the field left out, which is off.
                let on = if value.is_null() { false } else { value.as_bool().ok_or_else(rejects)? };
                let switched = switch_percentages(m, self.selected.then_some(self.image), on);
                self.note = switched.as_ref().err().cloned().map(Label::from);
                switched?;
            }
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        follow_ratio(&mut self.values, self.image);
        self.held.clear_refusal();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::dialog::HeldOk;
    use vectorcraft_engine::Session;

    /// An app with a 100 × 50 pt embedded image selected.
    fn app_with_image() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
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

    /// Runs `f` on the open dialog, `None` when none is open. // PLAN: the sheet's `dialog(app) -> &mut ObjectMosaicDialog` cannot outlive the host's lock.
    fn with_dialog<R>(app: &mut VectorcraftApp, f: impl FnOnce(&mut ObjectMosaicDialog) -> R) -> Option<R> {
        app.dialogs()?.with(|host| host.get_mut::<ObjectMosaicDialog>().map(f))
    }

    fn is_open(app: &mut VectorcraftApp) -> bool {
        with_dialog(app, |_| ()).is_some()
    }

    /// The open dialog's fields.
    fn fields(app: &mut VectorcraftApp) -> Value {
        with_dialog(app, |d| d.json_fields()).flatten().expect("an open dialog")
    }

    fn number_of(app: &mut VectorcraftApp, key: &str) -> f64 {
        fields(app)[key].as_f64().unwrap()
    }

    /// `ui.dialog.set` on the open dialog.
    fn set(app: &mut VectorcraftApp, field: &str, value: Value) -> Result<(), String> {
        let ctx = crate::testing::ui_context();
        app.dialogs().expect("a window").with(|host| host.set_json_field(&ctx, dialog_id(KIND), field, value))
    }

    /// OK as a click: the dialog files its answer, the app runs the command, and an accepted answer closes the dialog.
    fn ok(app: &mut VectorcraftApp) {
        let ctx = crate::testing::ui_context();
        let confirm = |app: &mut VectorcraftApp| {
            app.dialogs().expect("a window").with(|host| host.confirm_by_id(&ctx, dialog_id(KIND)));
        };
        confirm(app);
        crate::dialogs::host::pump(app, &ctx);
        if app.dialogs().expect("a window").with(|host| host.held_state(dialog_id(KIND))) == Some(HeldOk::Accepted) {
            confirm(app);
        }
    }

    fn select_image(app: &mut VectorcraftApp) {
        let id = image_id(app);
        app.run("select.set", json!({"ids": [id]})).unwrap();
    }

    /// An image narrower than 0.1 pt opens with a new width of 0.1 pt, which OK makes.
    #[test]
    fn a_tiny_image_opens_with_a_size_the_command_takes() {
        let mut app = app_with_image();
        app.run("object.scale", json!({"sx": 0.05, "sy": 100})).unwrap();
        open(&mut app, &json!({})).unwrap();
        assert_eq!(number_of(&mut app, "width"), 0.1);
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| m.0), Some(0.1));
    }

    /// Constrain Ratio keeps the new size in the image's proportions from the side it names; Resize Using
    /// Percentages shows the same size as percentages; Use Ratio makes the tiles square from the kept side's count.
    #[test]
    fn constrain_ratio_percentages_and_use_ratio_set_the_fields() {
        let mut app = app_with_image();
        open(&mut app, &json!({"columns": 10, "rows": 10})).unwrap();
        let ctx = crate::testing::ui_context();
        set(&mut app, "lock", json!(true)).unwrap();
        set(&mut app, "width", json!(300)).unwrap();
        assert_eq!(number_of(&mut app, "height"), 150.0, "height follows the width");
        // Percentages: 300 × 150 pt of a 100 × 50 pt image is 300%.
        set(&mut app, "percentages", json!(true)).unwrap();
        assert_eq!((number_of(&mut app, "width"), number_of(&mut app, "height")), (300.0, 300.0));
        // Use Ratio: 10 columns across 300 pt are 30 pt tiles, so 5 rows down 150 pt.
        with_dialog(&mut app, |d| d.use_ratio = true);
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!((number_of(&mut app, "columns"), number_of(&mut app, "rows")), (10.0, 5.0));
        // Kept by height: 4 rows of 37.5 pt over 300 pt are 8 columns.
        set(&mut app, "constrain", json!("height")).unwrap();
        set(&mut app, "rows", json!(4)).unwrap();
        with_dialog(&mut app, |d| d.use_ratio = true);
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!((number_of(&mut app, "columns"), number_of(&mut app, "rows")), (8.0, 4.0));
        set(&mut app, "percentages", json!(false)).unwrap();
        assert_eq!((number_of(&mut app, "width"), number_of(&mut app, "height")), (300.0, 150.0));
        set(&mut app, "gray", json!(true)).unwrap();
        ok(&mut app);
        assert_eq!(mosaic(&app), Some((300.0, 150.0, 32)));
    }

    /// With Constrain Ratio on, a kept width of 0.1 pt over a 100 × 50 pt image sets a height of
    /// 0.05 pt, under the command's minimum: OK makes the 0.2 × 0.1 pt mosaic the command grows it to.
    #[test]
    fn a_constrained_size_under_the_minimum_grows_in_proportion() {
        let mut app = app_with_image();
        open(&mut app, &json!({})).unwrap();
        set(&mut app, "lock", json!(true)).unwrap();
        set(&mut app, "width", json!(0.1)).unwrap();
        assert_eq!(number_of(&mut app, "height"), 0.05, "height follows the width");
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((0.2, 0.1)));
    }

    /// A caller's params open the dialog with their values, and OK makes the mosaic they ask for.
    #[test]
    fn a_callers_params_open_the_dialog_with_their_values() {
        let mut app = app_with_image();
        let params = json!({"columns": 4, "rows": 2, "width": 200, "percentages": true, "constrain": "width", "gray": true});
        app.run("ui.paramDialog", json!({"command": COMMAND, "label": "Create Object Mosaic", "params": params})).unwrap();
        let f = fields(&mut app);
        assert_eq!(
            (&f["width"], &f["percentages"], &f["lock"], &f["constrain"], &f["gray"]),
            (&json!(200.0), &json!(true), &json!(true), &json!("width"), &json!(true))
        );
        ok(&mut app);
        assert_eq!(mosaic(&app), Some((200.0, 100.0, 8)));
        // Use Ratio given applies to the counts shown: 10 columns over 100 × 50 pt are 5 rows.
        select_image(&mut app);
        let params = json!({"columns": 10, "rows": 3, "useRatio": true});
        app.run("ui.paramDialog", json!({"command": COMMAND, "params": params})).unwrap();
        assert_eq!(number_of(&mut app, "rows"), 5.0);
        // A side Use Ratio names is the side the dialog keeps: 4 rows over 100 × 50 pt are 8 columns.
        let params = json!({"columns": 3, "rows": 4, "useRatio": "height"});
        app.run("ui.paramDialog", json!({"command": COMMAND, "params": params})).unwrap();
        let f = fields(&mut app);
        assert_eq!((&f["columns"], &f["rows"], &f["constrain"], &f["lock"]), (&json!(8.0), &json!(4.0), &json!("height"), &json!(false)));
        let ctx = crate::testing::ui_context();
        app.dialogs().unwrap().with(|host| host.cancel::<ObjectMosaicDialog>(&ctx));
        // Params the command refuses open nothing and say why.
        for (bad, key) in [(json!({"width": "abc"}), "width"), (json!({"constrain": "both"}), "constrain"), (json!({"widht": 200}), "widht")] {
            let e = app.run("ui.paramDialog", json!({"command": COMMAND, "params": bad})).unwrap_err();
            assert!(!is_open(&mut app), "{e}");
            assert!(e.contains(key), "{e}");
        }
    }

    /// `ui.dialog.set` of Resize Using Percentages shows the new size the other way, as the checkbox
    /// does, so OK makes the same mosaic. A switch that would leave the command's range is refused:
    /// the points stay and the note says why.
    #[test]
    fn setting_percentages_from_outside_keeps_the_size_or_is_refused() {
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 200, "height": 100})).unwrap();
        set(&mut app, "percentages", json!(true)).unwrap();
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["height"], &f["percentages"]), (&json!(200.0), &json!(200.0), &json!(true)));
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((200.0, 100.0)));
        app.run("edit.undo", json!({})).unwrap();
        // 0.1 × 0.1 pt of a 1000 × 500 pt image are 0.01% × 0.02%, under the command's 0.1%: the
        // points stay, with the limit shown, and OK makes that size.
        select_image(&mut app);
        app.run("object.scale", json!({"sx": 1000, "sy": 1000})).unwrap();
        open(&mut app, &json!({"width": 0.1, "height": 0.1})).unwrap();
        assert!(set(&mut app, "percentages", json!(true)).is_err());
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["height"], &f["percentages"]), (&json!(0.1), &json!(0.1), &json!(false)));
        let note = with_dialog(&mut app, |d| d.note.as_ref().map(|n| n.as_str().to_owned())).flatten().unwrap();
        assert!(note.contains("`width` must be between 0.1"), "{note}");
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((0.1, 0.1)));
    }

    /// `ui.dialog.set` refuses what the command refuses, and leaves the fields as they were.
    #[test]
    fn ui_dialog_set_refuses_what_the_command_refuses() {
        let mut app = app_with_image();
        open(&mut app, &json!({})).unwrap();
        let before = fields(&mut app);
        for (field, value) in [
            ("width", json!("abc")),
            ("width", json!(0.01)),
            ("percentages", json!("yes")),
            ("gray", json!(1)),
            ("lock", json!("yes")),
            ("constrain", json!("both")),
            ("columns", json!(0)),
        ] {
            assert!(set(&mut app, field, value.clone()).is_err(), "{field}: {value}");
            assert_eq!(fields(&mut app), before, "{field}: {value}");
        }
        // The side Constrain Ratio sets takes no input.
        set(&mut app, "lock", json!(true)).unwrap();
        let before = fields(&mut app);
        assert!(set(&mut app, "height", json!(80)).unwrap_err().contains("takes no input"));
        assert_eq!(fields(&mut app), before);
        // Null is false, as the command reads it.
        set(&mut app, "percentages", Value::Null).unwrap();
        assert_eq!(fields(&mut app)["percentages"], json!(false));
    }

    /// The image scaled from 100 × 50 pt to 200 × 100 pt while the dialog is open: `ui.dialog.set` runs
    /// `sync` first, so Resize Using Percentages converts New Size against it, and OK makes the size shown.
    /// With no image selected, switching is refused and the note asks for one.
    #[test]
    fn percentages_convert_against_the_image_scaled_while_the_dialog_is_open() {
        let ctx = crate::testing::ui_context();
        let control = |app: &mut VectorcraftApp, field: &str, value: Value| {
            let (req, _) = craft::automation::ControlRequest::new("ui.dialog.set", json!({"field": field, "value": value}));
            let craft::automation::Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("expected an immediate reply") };
            r
        };
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 200, "height": 100})).unwrap();
        app.run("object.scale", json!({"sx": 200, "sy": 200})).unwrap();
        let r = control(&mut app, "percentages", json!(true));
        assert_eq!(r["ok"], json!(true), "{r}");
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["height"], &f["percentages"]), (&json!(100.0), &json!(100.0), &json!(true)));
        app.run("object.scale", json!({"sx": 50, "sy": 50})).unwrap();
        // After the image went back to 100 × 50 pt: 100% of it is 100 × 50 pt.
        control(&mut app, "percentages", json!(false));
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["height"]), (&json!(100.0), &json!(50.0)));
        control(&mut app, "percentages", json!(true));
        app.run("object.scale", json!({"sx": 200, "sy": 200})).unwrap();
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| (m.0, m.1)), Some((200.0, 100.0)), "100% of the image as it is on OK");
        app.run("edit.undo", json!({})).unwrap();
        // No image selected: the mode and sizes stay and the dialog asks for an image.
        select_image(&mut app);
        open(&mut app, &json!({})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        control(&mut app, "percentages", json!(true));
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["percentages"]), (&json!(200.0), &json!(false)));
        let note = with_dialog(&mut app, |d| d.note.as_ref().map(|n| n.as_str().to_owned())).flatten();
        assert_eq!(note.as_deref(), Some(NO_IMAGE));
        select_image(&mut app);
        control(&mut app, "percentages", json!(true));
        let f = fields(&mut app);
        assert_eq!((&f["width"], &f["percentages"]), (&json!(100.0), &json!(true)));
        assert_eq!(with_dialog(&mut app, |d| d.note.is_none()), Some(true));
    }

    /// When the command refuses OK, the dialog stays open with its values and the reason; OK then makes
    /// the mosaic as one undo step and closes it.
    #[test]
    fn a_refused_field_keeps_the_dialog_open() {
        let mut app = app_with_image();
        open(&mut app, &json!({"width": 80})).unwrap();
        app.run("select.set", json!({"ids": []})).unwrap();
        ok(&mut app);
        assert_eq!(mosaic(&app), None);
        assert!(is_open(&mut app), "the dialog stays open");
        assert_eq!(number_of(&mut app, "width"), 80.0);
        let reason = with_dialog(&mut app, |d| d.held.refusal().map(|(reason, _)| reason.as_str().to_owned())).flatten().expect("a reason");
        assert!(reason.contains("image"), "{reason}");
        select_image(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        ok(&mut app);
        assert!(!is_open(&mut app));
        assert_eq!(mosaic(&app).map(|m| m.0), Some(80.0));
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
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
        set(&mut app, "columns", json!(3.0)).unwrap();
        ok(&mut app);
        assert_eq!(mosaic(&app).map(|m| m.2), Some(30));
    }
}
