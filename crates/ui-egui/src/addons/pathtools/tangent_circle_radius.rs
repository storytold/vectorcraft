//! The Tangent Circle Tool's radius dialog, which a click on the second segment opens when the
//! tool's Ask For Circle Radius option is on. It starts at the radius of the circle the click found,
//! and each radius previews on the canvas the circle of that radius touching the two segments
//! nearest the click ([`tangent_circle::radius_circle`]). A radius that gives no circle there shows
//! the tool's note and leaves OK off, as does an edit to either segment after the click. OK draws
//! the circle with `shape.circleThroughPoints` in one undo step, and with Auto-Update Default on
//! also sets the tool's `radius` to the one given when it is in the tool's range; Cancel draws nothing.

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_doc::Document;
use vectorcraft_tools::addons::tangent_circle;

use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;
use crate::dialogs::host::{HeldOptions, open_held_with};
use crate::dialogs::preview::LivePreview;

/// The dialog's kind.
pub const KIND: &str = tangent_circle::RADIUS_DIALOG;
/// The command that draws the circle.
const DRAW: &str = "shape.circleThroughPoints";
/// The error for a radius outside the tool's range.
const RADIUS_RANGE: &str = "`radius` must be from 0.01 to 1000000 pt";

/// The open radius dialog: the dialog, what the tool found (the segments, the click and its circle), the id of the
/// document the click was in (`DocState::uid`), the circle last looked for and the live preview.
pub(crate) struct Open {
    id: DialogId,
    click: Value,
    doc: Option<u64>,
    /// The radius the circle was last looked for at, and the circle's parameters for [`DRAW`] (`None`: no circle).
    tried: Option<(f64, Option<Value>)>,
    /// Why there is no circle at the radius the dialog held at the last frame (in English), which the dialog shows
    /// under its rows with OK off; `None` while there is one.
    note: Option<String>,
    preview: LivePreview,
    /// The document that was active when the preview last ran (`DocState::uid`).
    shown_in: Option<u64>,
}

/// Open the dialog on what the tool found (its [`tangent_circle::RADIUS_DIALOG`] parameters).
pub(crate) fn open(app: &mut VectorcraftApp, p: &Value) {
    let radius = p.get("radius").and_then(Value::as_f64).unwrap_or(tangent_circle::RADIUS);
    let doc = app.session.doc().ok().map(|s| s.uid);
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("no window");
        return;
    };
    // A dialog still open from an earlier click closes without an answer, and its preview is rolled back. The new
    // dialog has its id (the dialog's kind), so the earlier dialog's held answer, with its OK and cancel hook, serves
    // the new one.
    let mut held = None;
    if let Some(mut old) = app.addon_dialogs.tangent_radius.take() {
        if let Some(host) = app.dialogs() {
            host.with(|h| {
                h.cancel_by_id(&ctx, old.id);
                let _ = h.take_by_id::<HostedParamDialog>(old.id);
            });
            held = Some(host);
        }
        take_preview(app, &mut old).cancel(app);
    }
    let lengths = general_lengths(app);
    let fields = DialogFields::from_pairs([("radius", FieldValue::Number(radius))]);
    // The field has no bounds: a radius typed outside the tool's range is kept as typed and shows [`RADIUS_RANGE`].
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        let rows = vec![Param::number("radius", tl!("Radius:"), radius, 1.0, 2).length(lengths)];
        ParamDialog::new(KIND, tl!("Tangent Circle"), rows).values(Bind::bound(fields)).previews(true)
    });
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(|app| {
        if let Some(mut open) = app.addon_dialogs.tangent_radius.take() {
            take_preview(app, &mut open).cancel(app);
        }
    });
    let opened = match held {
        Some(host) => Some(host.with(|h| h.open(dialog.held_ok()))),
        None => open_held_with(app, dialog, ok, HeldOptions { parent: None, cancel: Some(cancel) }),
    };
    match opened {
        Some(id) => {
            app.addon_dialogs.tangent_radius =
                Some(Open { id, click: p.clone(), doc, tried: None, note: None, preview: LivePreview::default(), shown_in: doc });
        }
        None => app.status("no window"),
    }
}

/// The document the click was in, while it is the active one. A preview adds the circle to it and
/// leaves the two segments as they are.
fn doc<'a>(app: &'a VectorcraftApp, open: &Open) -> Option<&'a Document> {
    let clicked = open.doc?;
    app.session.doc().ok().filter(|s| s.uid == clicked).map(|s| &*s.doc)
}

/// The id of the active document (`DocState::uid`).
fn active_uid(app: &VectorcraftApp) -> Option<u64> {
    app.session.doc().ok().map(|s| s.uid)
}

/// The dialog's live preview, taken out of `open`. With another document active than the one it previewed in, the
/// preview is forgotten instead: the switch already rolled it back there, and rolling back here would cancel an
/// interaction of the document now active.
fn take_preview(app: &VectorcraftApp, open: &mut Open) -> LivePreview {
    let preview = std::mem::take(&mut open.preview);
    if open.shown_in == active_uid(app) { preview } else { LivePreview::default() }
}

/// Whether `r` is in the tool's radius range.
fn in_range(r: f64) -> bool {
    let (lo, hi) = tangent_circle::RADIUS_RANGE;
    (lo..=hi).contains(&r)
}

/// The radius `fields` hold, when it is in the tool's range or is the radius of the circle the click found
/// (which a click without Shift may find outside that range).
fn radius(open: &Open, fields: &DialogFields) -> Result<f64, String> {
    let found = open.click.get("radius").and_then(Value::as_f64);
    fields.number("radius").filter(|r| in_range(*r) || Some(*r) == found).ok_or_else(|| RADIUS_RANGE.to_string())
}

/// The radius `fields` hold and the circle of that radius, as [`DRAW`]'s parameters. `Err` says why there is none:
/// a radius outside the tool's range, no document clicked in, either segment in it no longer the one clicked (a
/// control request edited or removed its path), or no circle of that radius there. The circle is looked for again
/// only when the radius changed.
fn circle(app: &VectorcraftApp, open: &mut Open, fields: &DialogFields) -> Result<(f64, Value), String> {
    let r = radius(open, fields)?;
    let no_circle = || tangent_circle::NO_CIRCLE.to_string();
    if !doc(app, open).is_some_and(|doc| tangent_circle::segments_unchanged(doc, &open.click)) {
        return Err(no_circle());
    }
    let found = match &open.tried {
        Some((tried, found)) if *tried == r => found.clone(),
        _ => {
            let found = tangent_circle::radius_circle(&open.click, r);
            open.tried = Some((r, found.clone()));
            found
        }
    };
    found.map(|points| (r, points)).ok_or_else(no_circle)
}

/// Why the active document cannot take a new circle now: its target layer is locked or hidden. The draw would be
/// refused, with or without a preview.
fn target_refusal(app: &VectorcraftApp) -> Option<String> {
    app.session.doc().ok()?.target_parent().err().map(|e| e.to_string())
}

/// The words the dialog shows for `why` ([`circle`]'s error), in the interface's language.
fn shown(why: &str) -> String {
    if why == tangent_circle::NO_CIRCLE { tl!(tangent_circle::NO_CIRCLE).to_string() } else { crate::i18n::msg(why) }
}

/// Once a frame: previews the circle of the radius the dialog holds, or rolls the preview back and shows why there
/// is none, with OK off.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.addon_dialogs.tangent_radius.take() else { return };
    let Some(host) = app.dialogs() else {
        app.addon_dialogs.tangent_radius = Some(open);
        return;
    };
    let id = open.id;
    let Some((previewing, fields)) = host.with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| (d.is_previewing(), d.fields().clone()))) else {
        app.addon_dialogs.tangent_radius = Some(open);
        return;
    };
    let mut preview = take_preview(app, &mut open);
    let found = circle(app, &mut open, &fields);
    // Without a circle the preview rolls back; the preview reports its own failures on the status line.
    let previewed = match &found {
        Ok((_, points)) => (previewing, points.clone()),
        Err(_) => (false, Value::Null),
    };
    let _ = preview.run(app, previewed.0, "Tangent Circle", DRAW, &previewed.1);
    open.preview = preview;
    open.shown_in = active_uid(app);
    // A target layer locked or hidden after the circle previewed refuses the draw: OK stays off. The preview stays,
    // since rolling it back would also roll back the edit that locked or hid the layer.
    open.note = found.err().or_else(|| target_refusal(app));
    let ready = open.note.as_deref().map_or(Ok(()), |why| Err(Label::from(shown(why))));
    app.addon_dialogs.tangent_radius = Some(open);
    host.with(|h| {
        if let Some(d) = h.get_mut_by_id::<HostedParamDialog>(id) {
            d.set_notes(ready.as_ref().err().cloned().into_iter().collect());
            d.set_ready(ready);
        }
    });
}

/// OK: draw the circle (keeping its preview) and close; with Auto-Update Default on, the radius
/// becomes the tool's when it is in the tool's range. Without a circle the dialog stays open.
fn ok(app: &mut VectorcraftApp, result: &ParamResult) -> Result<(), String> {
    let Some(mut open) = app.addon_dialogs.tangent_radius.take() else { return Err(shown(tangent_circle::NO_CIRCLE)) };
    let (r, points) = match circle(app, &mut open, &result.fields) {
        Ok(found) => found,
        Err(why) => {
            app.addon_dialogs.tangent_radius = Some(open);
            return Err(shown(&why));
        }
    };
    let auto = app.session.tool_options_of(tangent_circle::ID).get("autoUpdateDefault").and_then(Value::as_bool) == Some(true);
    // A failed commit leaves the interaction running: the dialog keeps its preview, so Cancel still ends it.
    if let Err(e) = open.preview.clone().commit(app, DRAW, points) {
        app.addon_dialogs.tangent_radius = Some(open);
        return Err(e);
    }
    if auto && in_range(r) {
        app.run("tool.setOption", json!({ "tool": tangent_circle::ID, "key": "radius", "value": r }))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};
    use vectorcraft_engine::{Session, ViewInfo};
    use vectorcraft_tools::addons::tangent_circle as tool;
    use vectorcraft_tools::{PointerEvent, PointerKind};

    use crate::VectorcraftApp;
    use crate::addons::dialogs::tests::{cancel, confirm, dialog_id, frame, read, set};

    fn count(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().doc.layers[0].children().unwrap().len()
    }

    fn undo_steps(app: &VectorcraftApp) -> usize {
        app.session.doc().unwrap().history.undo.len()
    }

    /// A test window with a 500 pt document, the lines `ys` from x 100 to 300, and the tool with Ask For Circle
    /// Radius on.
    fn app_with_lines(ys: &[i64]) -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 500, "height": 500})).unwrap();
        for y in ys {
            app.run("path.create", json!({"anchors": [{"x": 100, "y": y}, {"x": 300, "y": y}]})).unwrap();
        }
        app.session.select_tool(tool::ID, ViewInfo::default()).unwrap();
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "askRadius", "value": true})).unwrap();
        app
    }

    /// A click with the active tool at (`x`, `y`), Shift held when `shift`, with what it asks the
    /// interface for done.
    fn click(app: &mut VectorcraftApp, x: f64, y: f64, shift: bool) {
        for kind in [PointerKind::Down, PointerKind::Up] {
            let mut ev = PointerEvent::new(kind, x, y);
            ev.mods.shift = shift;
            let r = app.session.pointer(&ev, ViewInfo::default());
            crate::canvas::apply_requests(app, r);
        }
    }

    fn is_open(app: &VectorcraftApp) -> bool {
        app.addon_dialogs.tangent_radius.is_some()
    }

    /// Why the dialog shows no circle and leaves OK off, at the last frame.
    fn note(app: &VectorcraftApp) -> Option<String> {
        app.addon_dialogs.tangent_radius.as_ref()?.note.clone()
    }

    /// Presses OK and lets the held answer through: whether the dialog closed.
    fn press_ok(app: &mut VectorcraftApp) -> bool {
        let id = dialog_id(app);
        let enabled = confirm(app, id);
        frame(app);
        frame(app);
        enabled && !is_open(app)
    }

    /// Presses OK while it is off: it takes no press, and the dialog stays open.
    fn press_ok_while_off(app: &mut VectorcraftApp) {
        let id = dialog_id(app);
        assert!(!confirm(app, id), "OK is off");
        frame(app);
        assert!(is_open(app));
    }

    #[test]
    fn the_radius_dialog_previews_says_when_there_is_none_cancels_and_draws_on_ok() {
        let mut app = app_with_lines(&[100, 140]);
        app.run("path.create", json!({"anchors": [{"x": 100, "y": 300}, {"x": 300, "y": 300}, {"x": 300, "y": 400}]})).unwrap();
        let (paths, steps) = (count(&app), undo_steps(&app));
        // A click on each line: the dialog at the click's circle (radius 20), previewed.
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        assert!(is_open(&app), "{}", app.ui.status);
        let id = dialog_id(&app);
        assert!((read(&app, id, |f| f.number("radius")).unwrap() - 20.0).abs() < 1e-6);
        frame(&mut app);
        assert_eq!(count(&app), paths + 1, "previewed: {}", app.ui.status);
        assert_eq!(note(&app), None);
        // Lines 40 pt apart take no other radius: the note, nothing previewed, and OK off.
        set(&app, id, "radius", json!(25));
        frame(&mut app);
        assert_eq!(note(&app).as_deref(), Some(tool::NO_CIRCLE));
        assert_eq!(count(&app), paths);
        press_ok_while_off(&mut app);
        assert_eq!(count(&app), paths);
        // A radius outside the tool's range (as a control request can set it) is an error.
        for bad in [json!(0), json!(-20), json!(2e6)] {
            set(&app, id, "radius", bad.clone());
            frame(&mut app);
            assert_eq!(note(&app).as_deref(), Some(super::RADIUS_RANGE), "{bad}");
            press_ok_while_off(&mut app);
            assert_eq!(count(&app), paths, "{bad}");
        }
        // Back at 20, previewed again; Cancel draws nothing and leaves no undo step.
        set(&app, id, "radius", json!(20));
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        cancel(&app, id);
        frame(&mut app);
        assert!(!is_open(&app));
        assert_eq!((count(&app), undo_steps(&app)), (paths, steps));
        // The first line stays picked: a click on the second asks again, and OK draws the circle in
        // one undo step; the tool's radius stays.
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert!(press_ok(&mut app), "{}", app.ui.status);
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps + 1));
        let st = app.session.doc().unwrap();
        let b = st.doc.bounds_of(&[st.doc.layers[0].children().unwrap()[paths].id], false).unwrap();
        assert!((b.x0 - 130.0).abs() < 1e-3 && (b.y0 - 100.0).abs() < 1e-3 && (b.width() - 40.0).abs() < 1e-3, "{b:?}");
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
        // Auto-Update Default: Shift in the L's corner asks at the tool's radius; Cancel keeps the
        // tool's radius, and OK with another radius draws that fillet and makes it the tool's.
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "autoUpdateDefault", "value": true})).unwrap();
        click(&mut app, 150.0, 300.0, false);
        click(&mut app, 300.0, 350.0, true);
        let id = dialog_id(&app);
        assert_eq!(read(&app, id, |f| f.number("radius")), Some(tool::RADIUS));
        set(&app, id, "radius", json!(12));
        frame(&mut app);
        cancel(&app, id);
        frame(&mut app);
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
        click(&mut app, 300.0, 350.0, true);
        let id = dialog_id(&app);
        set(&app, id, "radius", json!(10));
        frame(&mut app);
        assert!(press_ok(&mut app), "{}", app.ui.status);
        assert_eq!((count(&app), undo_steps(&app)), (paths + 2, steps + 2));
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(10.0));
    }

    #[test]
    fn another_document_or_a_segment_changed_after_the_click_gives_no_circle() {
        let mut app = app_with_lines(&[100, 140]);
        let paths = count(&app);
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(note(&app), None);
        assert_eq!(count(&app), paths + 1);
        // Clicked in another document than the active one: no circle and OK off, until that one is
        // active again.
        let clicked = app.addon_dialogs.tangent_radius.as_ref().and_then(|o| o.doc);
        assert_eq!(clicked, Some(app.session.doc().unwrap().uid));
        let set_doc = |app: &mut VectorcraftApp, doc: Option<u64>| app.addon_dialogs.tangent_radius.as_mut().unwrap().doc = doc;
        set_doc(&mut app, clicked.map(|d| d + 1));
        frame(&mut app);
        assert_eq!(note(&app).as_deref(), Some(tool::NO_CIRCLE));
        assert_eq!(count(&app), paths);
        press_ok_while_off(&mut app);
        set_doc(&mut app, clicked);
        frame(&mut app);
        assert_eq!(note(&app), None);
        assert_eq!(count(&app), paths + 1);
        // The dialog holds a first segment the document no longer has (as when a control request
        // moves that line): no circle, nothing previewed, and OK refused.
        app.addon_dialogs.tangent_radius.as_mut().unwrap().click["first"] = json!([[100.0, 110.0], [100.0, 110.0], [300.0, 110.0], [300.0, 110.0]]);
        frame(&mut app);
        assert_eq!(note(&app).as_deref(), Some(tool::NO_CIRCLE));
        assert_eq!(count(&app), paths);
        press_ok_while_off(&mut app);
        assert_eq!(count(&app), paths);
    }

    #[test]
    fn a_failed_ok_keeps_the_preview_for_cancel() {
        let mut app = app_with_lines(&[100, 140]);
        let (paths, steps) = (count(&app), undo_steps(&app));
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        // The layer the preview started from is locked before OK, so OK's draw is refused and the dialog stays open.
        {
            let st = app.session.doc_mut().unwrap();
            let layer = st.doc.layers[0].id;
            let it = st.interaction.as_mut().unwrap();
            std::sync::Arc::make_mut(&mut it.doc).node_mut(layer).unwrap().locked = true;
        }
        let id = dialog_id(&app);
        assert!(confirm(&app, id), "OK is on");
        frame(&mut app);
        assert!(is_open(&app));
        assert!(app.session.in_interaction());
        // The dialog still owns the interaction: a radius with no circle there ends it.
        set(&app, id, "radius", json!(25));
        frame(&mut app);
        assert_eq!(note(&app).as_deref(), Some(tool::NO_CIRCLE));
        assert!(!app.session.in_interaction());
        // Cancel closes it with nothing drawn and no undo step.
        cancel(&app, id);
        frame(&mut app);
        assert!(!is_open(&app));
        assert!(!app.session.in_interaction());
        assert_eq!((count(&app), undo_steps(&app)), (paths, steps));
    }

    #[test]
    fn cancel_right_after_a_failed_ok_rolls_the_preview_back() {
        let mut app = app_with_lines(&[100, 140]);
        let (paths, steps) = (count(&app), undo_steps(&app));
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        {
            let st = app.session.doc_mut().unwrap();
            let layer = st.doc.layers[0].id;
            let it = st.interaction.as_mut().unwrap();
            std::sync::Arc::make_mut(&mut it.doc).node_mut(layer).unwrap().locked = true;
        }
        let id = dialog_id(&app);
        assert!(confirm(&app, id), "OK is on");
        frame(&mut app);
        assert!(is_open(&app));
        assert!(app.session.in_interaction());
        // Cancel is the next press after the refused OK, with no radius change or other frame in between.
        cancel(&app, id);
        frame(&mut app);
        assert!(!is_open(&app));
        assert!(!app.session.in_interaction());
        assert_eq!((count(&app), undo_steps(&app)), (paths, steps));
    }

    #[test]
    fn a_click_found_radius_outside_the_tool_range_can_be_drawn() {
        // Lines 2,400,000 pt apart: a click on each finds the circle of radius 1,200,000, past the
        // largest radius the tool takes.
        let mut app = app_with_lines(&[100, 2_400_100]);
        app.run("tool.setOption", json!({"tool": tool::ID, "key": "autoUpdateDefault", "value": true})).unwrap();
        let (paths, steps) = (count(&app), undo_steps(&app));
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 2_400_100.0, false);
        let id = dialog_id(&app);
        assert!((read(&app, id, |f| f.number("radius")).unwrap() - 1_200_000.0).abs() < 1e-3, "{}", app.ui.status);
        frame(&mut app);
        assert_eq!(note(&app), None);
        // OK draws it; the tool's radius stays, since the tool does not take that radius.
        assert!(press_ok(&mut app), "{}", app.ui.status);
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps + 1));
        assert_eq!(app.session.tool_options_of(tool::ID)["radius"], json!(tool::RADIUS));
    }

    #[test]
    fn a_second_click_replaces_the_open_dialog() {
        let mut app = app_with_lines(&[100, 140]);
        let (paths, steps) = (count(&app), undo_steps(&app));
        let open_ids = |app: &VectorcraftApp| app.dialogs().unwrap().with(|h| h.open_dialogs().iter().map(|d| d.id).collect::<Vec<_>>());
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        // The first line stays picked: another click on the second (as a control request can send while the dialog
        // is open) asks again in one dialog, previewing one circle.
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(open_ids(&app).len(), 1);
        assert!(is_open(&app));
        assert_eq!(count(&app), paths + 1);
        // Cancel closes it and rolls its preview back, with no undo step.
        cancel(&app, dialog_id(&app));
        frame(&mut app);
        assert!(!is_open(&app));
        assert!(open_ids(&app).is_empty());
        assert_eq!((count(&app), undo_steps(&app)), (paths, steps));
        // Asked again, OK draws one circle in one undo step.
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert!(press_ok(&mut app), "{}", app.ui.status);
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps + 1));
    }

    #[test]
    fn a_switch_to_another_document_leaves_its_interaction_alone() {
        let mut app = app_with_lines(&[100, 140]);
        let paths = count(&app);
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(count(&app), paths + 1);
        // Another document comes to the front (which rolls the preview back in the first) and starts an interaction
        // of its own before the dialog's next frame: the dialog leaves that interaction running.
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.begin_interaction("Other").unwrap();
        app.session.preview("path.create", &json!({"anchors": [{"x": 10, "y": 10}, {"x": 50, "y": 10}]})).unwrap();
        frame(&mut app);
        assert_eq!(note(&app).as_deref(), Some(tool::NO_CIRCLE));
        assert!(app.session.in_interaction());
        assert_eq!(count(&app), 1);
        // Cancel leaves it running too.
        cancel(&app, dialog_id(&app));
        frame(&mut app);
        assert!(!is_open(&app));
        assert!(app.session.in_interaction());
        assert_eq!(count(&app), 1);
    }

    #[test]
    fn a_circle_the_target_layer_cannot_take_shows_why_and_leaves_ok_off() {
        let mut app = app_with_lines(&[100, 140]);
        let paths = count(&app);
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        frame(&mut app);
        assert_eq!(note(&app), None);
        assert_eq!(count(&app), paths + 1);
        // A control request locks the only layer while the dialog is open: no layer takes the circle, and OK is off.
        app.run("layer.setProps", json!({"locked": true})).unwrap();
        let steps = undo_steps(&app);
        frame(&mut app);
        assert!(note(&app).is_some_and(|n| n.contains("locked")), "{:?}", note(&app));
        press_ok_while_off(&mut app);
        assert_eq!((count(&app), undo_steps(&app)), (paths + 1, steps));
        // Unlocked again at the same radius: previewed, and OK draws the circle.
        app.run("layer.setProps", json!({"locked": false})).unwrap();
        frame(&mut app);
        assert_eq!(note(&app), None);
        assert_eq!(count(&app), paths + 1);
        assert!(press_ok(&mut app), "{}", app.ui.status);
        assert_eq!(count(&app), paths + 1);
    }

    #[test]
    fn a_radius_that_is_not_a_number_is_refused_and_keeps_the_circle() {
        let mut app = app_with_lines(&[100, 140]);
        let paths = count(&app);
        click(&mut app, 150.0, 100.0, false);
        click(&mut app, 200.0, 140.0, false);
        let id = dialog_id(&app);
        let ctx = crate::addons::dialogs::tests::ctx(&app);
        for bad in [json!("x"), Value::Null] {
            assert!(app.dialogs().unwrap().with(|h| h.set_json_field(&ctx, id, "radius", bad.clone())).is_err(), "{bad}");
            frame(&mut app);
            assert_eq!(note(&app), None, "{bad}");
            assert_eq!(count(&app), paths + 1, "{bad}");
        }
    }
}
