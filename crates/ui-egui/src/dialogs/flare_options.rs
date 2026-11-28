//! Flare Tool Options (double-click the Flare tool, or click with it): the options the Flare tool
//! keeps, which draw the next flare dragged out, last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool: "flare", values}`; opened by a click, it
//! accepts a flare preview there with them (`shape.flare`, on the active perspective plane while the
//! grid shows). While Alt (Option on macOS) is held, Cancel reads Reset, which puts every option
//! back to the tool's default and leaves the dialog open. A click-opened
//! dialog previews its settings live; Reset refreshes that preview and Cancel rolls it back.
//! Opened from the toolbar with flares selected, it shows the first one's settings, and OK draws
//! the selected flares again with the settings changed in the dialog (`shape.editFlare`); the tool
//! keeps its options.
//!
//! Fields: the options (`vectorcraft_tools::extra::FLARE_OPTIONS`): `diameter` and `pathLength`
//! (points), `opacity`, `brightness`, `growth`, `fuzziness`, `longest`, `rayFuzziness` and
//! `largest` (%), `rays` and `rings` (counts), `direction` (degrees); `raysOn` and `ringsOn`. The click
//! that opened it and the selected flares with the settings the dialog opened with are the open record's
//! (`ObjectDialogs::flare`), not fields.

// PLAN: the `craft_ui` paths and `LivePreview`'s home (`form`) are assumed; `general_lengths` is cv.B5b's.
use std::sync::Arc;

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamResult};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Map, Value, json};
use vectorcraft_geom::Point;
use vectorcraft_tools::extra::{FLARE_OPTIONS, flare_params};

use super::form;
use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use super::tool_options;
use crate::VectorcraftApp;
use crate::canvas::general_lengths;
use craft::ui::dialog::DialogId;

/// The dialog kind of the Flare Tool Options.
pub const KIND: &str = "flareOptions";

/// The open Flare Tool Options: the dialog, where a click opened it, the selected flares it edits (their ids and
/// the settings it opened with) and the live preview of either.
pub(crate) struct Open {
    /// The hosted dialog.
    id: DialogId,
    opening: Arc<()>,
    document: Option<u64>,
    /// Where a click opened it, to draw a flare there on OK.
    click: Option<Point>,
    /// The selected flares (ids) and the settings the dialog opened with, to change them on OK.
    edit: Option<(Value, Map<String, Value>)>,
    /// The live preview; a toolbar open draws nothing and has none, so Cancel leaves a gesture in progress alone.
    preview: Option<LivePreview>,
}

const PREVIEW_LABEL: &str = "Flare";

impl Open {
    fn on_original_drawing(&self, app: &VectorcraftApp) -> bool {
        self.preview.is_none() || app.session.active().map(|st| st.uid) == self.document
    }

    fn cancel_preview(self, app: &mut VectorcraftApp) {
        if self.on_original_drawing(app)
            && !other_preview(app)
            && let Some(preview) = self.preview
        {
            preview.cancel(app);
        }
    }
}

fn other_preview(app: &VectorcraftApp) -> bool {
    app.session.active().and_then(|st| st.interaction.as_ref()).is_some_and(|p| p.label != PREVIEW_LABEL)
}

/// Open the Flare Tool Options with the tool's values; `at`: where a click opened them, to draw a
/// flare there on OK. Opened without a click while flares are selected, they show the first one's
/// settings, to change the selected flares on OK.
pub fn open(app: &mut VectorcraftApp, at: Option<Point>) -> Value {
    let mut options = match app.session.tool_options_of("flare") {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    let mut edit = None;
    if at.is_none()
        && let Some((ids, settings)) = selected_flares(app)
    {
        // A flare without rays or rings shows that checkbox off, with the tool's count.
        for (key, on) in [("rays", "raysOn"), ("rings", "ringsOn")] {
            let n = settings.get(key).and_then(Value::as_f64).unwrap_or(0.0);
            options.insert(on.into(), json!(n > 0.0));
            if n > 0.0 {
                options.insert(key.into(), json!(n));
            }
        }
        for (key, ..) in FLARE_OPTIONS.iter().filter(|(k, ..)| !matches!(*k, "rays" | "rings")) {
            if let Some(v) = settings.get(*key) {
                options.insert(key.to_string(), v.clone());
            }
        }
        edit = Some((json!(ids), shown(&options)));
    }
    let lengths = general_lengths(app);
    let rows = params(lengths);
    let fields = form::fields_of(&rows, &options);
    let mut defaults = match vectorcraft_tools::create("flare").options() {
        Value::Object(o) => o,
        _ => Map::new(),
    };
    for key in ["raysOn", "ringsOn"] {
        defaults.insert(key.into(), json!(true));
    }
    let defaults = form::fields_of(&rows, &defaults);
    let previews = at.is_some() || edit.is_some();
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("no window");
        return Value::Null;
    };
    // An open the host replaces leaves its preview behind unless it is rolled back here.
    if let Some(old) = app.object_dialogs.flare.take() {
        old.cancel_preview(app);
    }
    let opening = Arc::new(());
    let cancelled = Arc::clone(&opening);
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(move |app| {
        if app.object_dialogs.flare.as_ref().is_some_and(|open| Arc::ptr_eq(&open.opening, &cancelled))
            && let Some(open) = app.object_dialogs.flare.take()
        {
            open.cancel_preview(app);
        }
    });
    let held = HeldOptions { parent: None, cancel: Some(cancel) };
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Flare Tool Options"), params(lengths)).values(Bind::bound(fields)).previews(previews).reset_to(defaults.clone())
    });
    let document = app.session.active().map(|st| st.uid);
    match open_held_with(app, dialog, ok, held) {
        Some(id) => app.object_dialogs.flare = Some(Open { id, opening, document, click: at, edit, preview: previews.then(LivePreview::default) }),
        None => app.status("no window"),
    }
    json!({ "dialog": KIND })
}

/// The selected flares that can be edited (ids) and the first one's settings as the dialog shows
/// them: its rings' path length and direction from its center to its end point, where the flare is
/// drawn now.
fn selected_flares(app: &VectorcraftApp) -> Option<(Vec<u64>, Map<String, Value>)> {
    let st = app.session.doc().ok()?;
    let flares: Vec<_> = st
        .selection
        .objects
        .iter()
        .filter(|id| st.doc.is_editable(**id))
        .filter_map(|id| vectorcraft_engine::flare_settings(&st.doc, *id).map(|spec| (id.0, spec)))
        .collect();
    let (_, first) = flares.first()?;
    let Value::Object(mut settings) = first.params.clone() else { return None };
    let d = first.end()? - first.center()?;
    settings.insert("pathLength".into(), json!(d.hypot()));
    // Counter-clockwise from 3 o'clock, 0° to 360°, as `shape.flare` takes it.
    settings.insert("direction".into(), json!((-d.y).atan2(d.x).to_degrees().rem_euclid(360.0)));
    Some((flares.iter().map(|(id, _)| *id).collect(), settings))
}

/// Whether the check box `key` of `fields` is on; a missing one counts as on.
fn on(fields: &Map<String, Value>, key: &str) -> bool {
    fields.get(key).and_then(Value::as_bool) != Some(false)
}

/// The number `fields` hold under `key`, else `default`.
fn number(fields: &Map<String, Value>, key: &str, default: f64) -> f64 {
    fields.get(key).and_then(Value::as_f64).unwrap_or(default)
}

/// The settings `fields` give a flare: each option, with a count of 0 when its checkbox is off.
fn shown(fields: &Map<String, Value>) -> Map<String, Value> {
    FLARE_OPTIONS
        .iter()
        .map(|&(key, default, ..)| {
            let v = match key {
                "rays" if !on(fields, "raysOn") => 0.0,
                "rings" if !on(fields, "ringsOn") => 0.0,
                "rays" | "rings" => number(fields, key, default).round(),
                _ => number(fields, key, default),
            };
            (key.to_string(), json!(v))
        })
        .collect()
}

/// `shape.editFlare`'s params for the flares `ids`: the settings `fields` give that differ from `start`, the
/// settings the dialog opened with.
fn edit_params(ids: &Value, start: &Map<String, Value>, fields: &Map<String, Value>) -> Value {
    let mut params = json!({ "ids": ids });
    for (key, v) in shown(fields) {
        if start.get(&key) != Some(&v) {
            params[key.as_str()] = if matches!(key.as_str(), "rays" | "rings") { json!(v.as_f64().unwrap_or(0.0) as u64) } else { v };
        }
    }
    params
}

/// The rows of the dialog; the distances are shown in the document's unit (`lengths`). The ranges and defaults are
/// the tool's (`FLARE_OPTIONS`).
fn params(lengths: LengthUnits) -> Vec<Param> {
    let row = |key: &'static str, label: &'static str| {
        let (default, lo, hi) = FLARE_OPTIONS.iter().find(|(k, ..)| *k == key).map_or((0.0, 0.0, 1000.0), |&(_, d, lo, hi)| (d, lo, hi));
        let row = Param::number(key, label, default, 1.0, 0).bounds(Some(lo), Some(hi));
        match key {
            "diameter" | "pathLength" => row.length(lengths),
            "rays" | "rings" => row,
            "direction" => row.unit(UnitKind::Degrees),
            _ => row.unit(UnitKind::Percent),
        }
    };
    vec![
        Param::section(tl!("Center")),
        row("diameter", tl!("Diameter:")),
        row("opacity", tl!("Opacity:")),
        row("brightness", tl!("Brightness:")),
        Param::section(tl!("Halo")).beside(),
        row("growth", tl!("Growth:")),
        row("fuzziness", tl!("Fuzziness:")),
        Param::section_check("raysOn", tl!("Rays"), true),
        row("rays", tl!("Number:")),
        row("longest", tl!("Longest:")),
        row("rayFuzziness", tl!("Fuzziness:")),
        Param::section_check("ringsOn", tl!("Rings"), true).beside(),
        row("pathLength", tl!("Path:")),
        row("rings", tl!("Number:")),
        row("largest", tl!("Largest:")),
        row("direction", tl!("Direction:")),
    ]
}

/// What the preview and OK run for the dialog `open` with the values `fields`: a click-created flare at that
/// click, the selected flares with the settings changed; toolbar options draw no artwork.
fn command(app: &VectorcraftApp, open: &Open, fields: &Map<String, Value>) -> Option<(String, Value)> {
    if let Some((ids, start)) = &open.edit {
        return Some(("shape.editFlare".into(), edit_params(ids, start, fields)));
    }
    let at = open.click?;
    // Use the same clamping and whole-count conversion as accepted tool settings.
    let mut tool = vectorcraft_tools::create("flare");
    for (key, default, ..) in FLARE_OPTIONS {
        tool.set_option(key, &json!(number(fields, key, default)));
    }
    for key in ["raysOn", "ringsOn"] {
        tool.set_option(key, &json!(on(fields, key)));
    }
    let params = flare_params(&tool.options(), at);
    Some(vectorcraft_engine::perspective_click(&app.session, "shape.flare", &params, at).unwrap_or_else(|| ("shape.flare".into(), params)))
}

/// The tool options `fields` hold: each option the dialog has, and the two check boxes.
fn option_values(fields: &Map<String, Value>) -> Map<String, Value> {
    let mut values: Map<String, Value> = FLARE_OPTIONS
        .iter()
        .filter(|(k, ..)| fields.contains_key(*k))
        .map(|(k, default, ..)| (k.to_string(), json!(number(fields, k, *default))))
        .collect();
    for key in ["raysOn", "ringsOn"] {
        values.insert(key.into(), json!(on(fields, key)));
    }
    values
}

/// Keeps the live preview in step with the dialog's values and its Preview check box, once a frame.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    // PLAN: registered in hub-w2's pump file (wg.A18), one line.
    let Some(mut open) = app.object_dialogs.flare.take() else { return };
    if !open.on_original_drawing(app) {
        app.object_dialogs.flare = Some(open);
        return;
    }
    if let Some(mut preview) = open.preview.take() {
        let state = app
            .dialogs()
            .and_then(|host| host.with(|host| host.get_by_id::<HostedParamDialog>(open.id).map(|d| (d.is_previewing(), d.fields().clone()))));
        if let Some((previewing, fields)) = state {
            let values = form::json_of(&params(general_lengths(app)), &fields);
            if previewing
                && other_preview(app)
                && let Err(e) = app.keep_typing()
            {
                app.status(e);
            }
            if !other_preview(app)
                && let Some((cmd, params)) = command(app, &open, &values)
            {
                // The preview reports its own failures on the status line.
                let _ = preview.run(app, previewing, PREVIEW_LABEL, &cmd, &params);
            }
        }
        open.preview = Some(preview);
    }
    app.object_dialogs.flare = Some(open);
}

/// OK: set the tool's options; opened by a click, draw a flare there with them. Opened for the
/// selected flares, draw them again with the settings changed, leaving the tool's options.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let open = app.object_dialogs.flare.as_ref().ok_or("the Flare Tool Options dialog is gone")?;
    if !open.on_original_drawing(app) {
        return Err(tl!("Return to the original drawing to place or edit the flare").into());
    }
    if open.preview.is_some() && other_preview(app) {
        app.keep_typing()?;
        if other_preview(app) {
            return Err(tl!("Finish the current edit before drawing the flare").into());
        }
    }
    let fields = form::json_of(&params(general_lengths(app)), &result.fields);
    let accepted = accept(app, &fields);
    if accepted.is_ok() {
        app.object_dialogs.flare = None;
    }
    accepted
}

/// [`ok`] over the dialog's values `fields`; an `Err` keeps the dialog open.
fn accept(app: &mut VectorcraftApp, fields: &Map<String, Value>) -> Result<(), String> {
    let editing = app.object_dialogs.flare.as_ref().is_some_and(|open| open.edit.is_some());
    let command = app.object_dialogs.flare.as_ref().and_then(|open| command(app, open, fields));
    let values = option_values(fields);
    let Some((cmd, params)) = command else {
        return tool_options::set_options(app, "flare", values);
    };
    if !editing {
        // Options the tool would refuse leave the dialog open, before the flare is accepted.
        vectorcraft_tools::option_check::check_values(vectorcraft_tools::create("flare").as_ref(), &values)?;
    }
    match app.object_dialogs.flare.as_mut().and_then(|open| open.preview.take()) {
        // Accept the preview once; option persistence follows successful artwork creation.
        Some(preview) => {
            if let Err(e) = preview.commit(app, &cmd, params) {
                if let Some(open) = app.object_dialogs.flare.as_mut() {
                    open.preview = Some(LivePreview::default());
                }
                return Err(e);
            }
        }
        None => {
            app.run(&cmd, params)?;
        }
    }
    if editing { Ok(()) } else { tool_options::set_options(app, "flare", values) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    fn app() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 600, "height": 600})).unwrap();
        app
    }

    /// The children of the newest flare (a group named Flare), by name.
    fn flare_parts(app: &VectorcraftApp) -> Vec<String> {
        let doc = &app.session.doc().unwrap().doc;
        let flare = doc.layers[0].children().unwrap().iter().rev().find(|n| n.name.as_deref() == Some("Flare")).expect("a flare");
        let NodeKind::Group { children, .. } = &flare.kind else { panic!("a flare is a group") };
        children.iter().filter_map(|c| c.name.clone()).collect()
    }

    fn rings(app: &VectorcraftApp) -> usize {
        flare_parts(app).iter().filter(|s| *s == "Ring").count()
    }

    /// The id of the open Flare Tool Options.
    fn id(app: &VectorcraftApp) -> DialogId {
        app.dialogs()
            .unwrap()
            .with(|host| host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(KIND)).map(|d| d.id))
            .expect("the dialog is open")
    }

    /// The open dialog's values.
    fn fields(app: &VectorcraftApp) -> Value {
        let id = id(app);
        app.dialogs().unwrap().with(|host| host.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields)).expect("it lists its fields")
    }

    fn set(app: &VectorcraftApp, key: &str, value: Value) {
        let (ctx, id) = (window(app), id(app));
        app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, key, value)).unwrap();
    }

    /// The test app's window context.
    fn window(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("the app has a window")
    }

    /// One frame's pump.
    fn pump(app: &mut VectorcraftApp) {
        let ctx = window(app);
        crate::dialogs::host::pump(app, &ctx);
    }

    /// Presses OK and lets the held answer through.
    fn press_ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (window(app), id(app));
        app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, id));
        pump(app);
    }

    fn press_cancel(app: &mut VectorcraftApp) {
        let (ctx, id) = (window(app), id(app));
        app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx, id));
        pump(app);
    }

    fn press_reset(app: &VectorcraftApp) {
        let (ctx, id) = (window(app), id(app));
        assert!(app.dialogs().unwrap().with(|host| host.reset_by_id(&ctx, id)));
    }

    /// A toolbar open holds no preview, so Cancel leaves a gesture in progress alone.
    #[test]
    fn cancelling_toolbar_options_preserves_an_existing_interaction() {
        let mut app = app();
        app.session.begin_interaction("Existing gesture").unwrap();
        app.session.preview("shape.rectangle", &json!({"x":10,"y":10,"width":40,"height":40})).unwrap();
        let previewed = app.session.doc().unwrap().doc.layers.clone();
        open(&mut app, None);
        pump(&mut app);
        press_cancel(&mut app);
        assert!(app.session.in_interaction());
        assert_eq!(app.session.doc().unwrap().doc.layers, previewed);
        app.session.cancel_interaction().unwrap();
    }

    #[test]
    fn reset_updates_live_preview_and_cancel_discards_art_and_options() {
        let mut app = app();
        app.run("tool.setOption", json!({"tool":"flare", "values":{"rings":3}})).unwrap();
        let original = app.session.doc().unwrap().doc.layers.clone();
        let depth = app.session.doc().unwrap().history.undo.len();
        let options = app.session.tool_options_of("flare");
        open(&mut app, Some(Point::new(220.0, 330.0)));
        pump(&mut app);
        assert_eq!(rings(&app), 3);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
        set(&app, "rings", json!(4.0));
        pump(&mut app);
        assert_eq!(rings(&app), 4);
        press_reset(&app);
        pump(&mut app);
        assert_eq!(rings(&app), 10);
        assert_eq!(app.session.tool_options_of("flare"), options, "preview doesn't persist options");
        press_cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.layers, original);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth);
        assert_eq!(app.session.tool_options_of("flare"), options);
        // Accepting the same reset preview produces one flare and one Undo step.
        open(&mut app, Some(Point::new(220.0, 330.0)));
        pump(&mut app);
        press_reset(&app);
        pump(&mut app);
        press_ok(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        assert_eq!(rings(&app), 10);
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.layers, original);
    }

    #[test]
    fn a_click_fills_in_the_options_and_draws_a_flare_there() {
        let mut app = app();
        app.run("tool.setOption", json!({"tool": "flare", "values": {"rings": 3}})).unwrap();
        crate::dialogs::open_tool_dialog(&mut app, "flare", json!({"x": 300, "y": 300}));
        assert_eq!(fields(&app)["rings"].as_f64(), Some(3.0));
        press_ok(&mut app);
        let parts = flare_parts(&app);
        assert_eq!(parts.iter().filter(|p| *p == "Ring").count(), 3, "{parts:?}");
    }

    #[test]
    fn a_pending_flare_click_waits_for_its_original_drawing() {
        let mut app = app();
        open(&mut app, Some(Point::new(100.0, 110.0)));
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        let count = |app: &VectorcraftApp| app.session.active().unwrap().doc.layers[0].children().unwrap().len();
        pump(&mut app);
        assert_eq!(count(&app), 0, "the other drawing never shows the flare preview");
        press_ok(&mut app);
        assert_eq!(count(&app), 0, "the other drawing never keeps the flare");
        assert!(app.object_dialogs.flare.is_some());
        app.run("document.activate", json!({"index": 0})).unwrap();
        press_ok(&mut app);
        assert_eq!(count(&app), 1);
        assert!(!flare_parts(&app).is_empty());
    }

    /// With a flare selected, the toolbar's Flare Tool Options show its settings, and OK draws it
    /// again with the settings changed, as one undo step, leaving the tool's options.
    #[test]
    fn with_a_flare_selected_the_options_show_its_settings_and_ok_draws_it_again() {
        let mut app = app();
        let flare = json!({"cx": 200, "cy": 200, "diameter": 60, "rays": 0, "rings": 4, "x2": 200, "y2": 0, "largest": 80, "seed": 5});
        app.run("shape.flare", flare).unwrap();
        app.run("object.move", json!({"dx": 30, "dy": 0})).unwrap();
        let options = app.session.tool_options_of("flare");
        app.run("tool.options", json!({"tool": "flare"})).unwrap();
        let shown = fields(&app);
        let n = |key: &str| shown[key].as_f64().unwrap_or(f64::NAN);
        assert_eq!((n("diameter"), shown["raysOn"].as_bool(), n("rays"), n("rings"), n("largest")), (60.0, Some(false), 15.0, 4.0, 80.0));
        assert!((n("pathLength") - 200.0).abs() < 1e-9 && (n("direction") - 90.0).abs() < 1e-9);
        set(&app, "rings", json!(7));
        set(&app, "raysOn", json!(true));
        let depth = app.session.doc().unwrap().history.undo.len();
        pump(&mut app);
        press_ok(&mut app);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1);
        let parts = flare_parts(&app);
        assert_eq!((parts.iter().filter(|p| *p == "Ring").count(), parts.iter().any(|p| p == "Rays")), (7, true), "{parts:?}");
        // Still where it was moved to.
        let doc = &app.session.doc().unwrap().doc;
        let id = app.session.doc().unwrap().selection.objects[0];
        assert!(vectorcraft_engine::flare_settings(doc, id).unwrap().center().unwrap().distance(Point::new(230.0, 200.0)) < 1e-9);
        assert_eq!(app.session.tool_options_of("flare"), options);
        // With nothing selected, the dialog shows the tool's options again.
        app.run("select.none", json!({})).unwrap();
        app.run("tool.options", json!({"tool": "flare"})).unwrap();
        assert_eq!(fields(&app)["diameter"].as_f64(), Some(100.0));
        assert!(app.object_dialogs.flare.as_ref().is_some_and(|open| open.edit.is_none()));
    }

    #[test]
    fn reset_puts_the_defaults_back_and_keeps_where_a_click_opened_it() {
        let mut app = app();
        app.run("tool.setOption", json!({"tool": "flare", "values": {"rings": 3, "raysOn": false, "largest": 120}})).unwrap();
        crate::dialogs::open_tool_dialog(&mut app, "flare", json!({"x": 300, "y": 300}));
        press_reset(&app);
        let shown = fields(&app);
        assert_eq!((shown["rings"].as_f64(), shown["raysOn"].as_bool(), shown["largest"].as_f64()), (Some(10.0), Some(true), Some(50.0)));
        // The tool keeps its options until OK, which draws the flare with the defaults where the click was.
        assert_eq!(app.session.tool_options_of("flare")["rings"].as_u64(), Some(3));
        pump(&mut app);
        press_ok(&mut app);
        let parts = flare_parts(&app);
        assert_eq!((parts.iter().filter(|p| *p == "Ring").count(), parts.iter().any(|p| p == "Rays")), (10, true), "{parts:?}");
    }
}
