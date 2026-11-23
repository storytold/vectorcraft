//! Headless frames of the Appearance panel and the panels that follow its active item.

use craft::ui::dialog::{DialogHost, DialogId, PromptChoice, StandardPrompt};
// PLAN: assumed path of the hosted dialog type (`craft-ui-patterns` is not a dependency of this crate in the tree yet)
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use serde_json::{Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::Session;

use super::*;
use crate::tests::panel_harness::Panel;
use crate::tests::synthetic::key_frame;
use craft::ui::contract::Role;

pub(super) fn app_with_rect() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let id = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100}))["id"].clone();
    run(&mut app, "select.set", json!({ "ids": [id] }));
    app
}

pub(super) fn run(app: &mut VectorcraftApp, id: &str, p: Value) -> Value {
    app.session.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// One headless frame with input `events`: every text drawn, with its screen rect.
pub(super) fn frame_events(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    events: Vec<egui::Event>,
    f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    frame_raw(ctx, app, egui::RawInput { events, ..Default::default() }, f)
}

/// One headless frame with input `raw`: every text drawn, with its screen rect.
pub(super) fn frame_raw(
    ctx: &egui::Context,
    app: &mut VectorcraftApp,
    raw: egui::RawInput,
    mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    let mut out = ctx.run_ui(raw, |ui| f(app, ui));
    out.textures_delta.clear();
    fn collect(s: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match s {
            egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
            egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let mut texts = vec![];
    for c in &out.shapes {
        collect(&c.shape, &mut texts);
    }
    texts
}

/// Click at `pos` over three frames of `f` (hover, press, release).
pub(super) fn click(ctx: &egui::Context, app: &mut VectorcraftApp, pos: egui::Pos2, mut f: impl FnMut(&mut VectorcraftApp, &mut egui::Ui)) {
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame_events(ctx, app, vec![e], &mut f);
    }
}

/// The rect of the first drawn text equal to `s`.
pub(super) fn text_rect(texts: &[(String, egui::Rect)], s: &str) -> egui::Rect {
    texts.iter().find(|(t, _)| t == s).unwrap_or_else(|| panic!("`{s}` not drawn: {texts:?}")).1
}

/// One whole app frame in the app's window, so the dialog host and the effect dialogs' pump run as in production.
pub(super) fn app_frame(app: &mut VectorcraftApp) {
    key_frame(app, &window(app), vec![]);
}

/// The app's window context, or a new window for an app without one: every frame of a window draws in its context.
fn window(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().unwrap_or_else(crate::testing::ui_context)
}

/// The id of the open effect dialog.
fn effect_dialog_id(app: &VectorcraftApp) -> DialogId {
    // PLAN: assumed `Open::id` of ef.C1's record
    app.effect_dialogs.effect.as_ref().map(|open| open.id).expect("an open effect dialog")
}

/// Whether an effect dialog is open.
pub(super) fn dialog_open(app: &VectorcraftApp) -> bool {
    app.effect_dialogs.effect.is_some()
}

/// Runs `f` on the window's dialog host with the window's context.
pub(super) fn with_host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost, &egui::Context) -> R) -> R {
    let ctx = app.ui_ctx.clone().expect("a window context");
    app.dialogs().expect("the window's dialog host").with(|host| f(host, &ctx))
}

/// The number the open effect dialog holds for `key` (0 when it holds none).
pub(super) fn dialog_number(app: &mut VectorcraftApp, key: &str) -> f64 {
    let id = effect_dialog_id(app);
    with_host(app, |host, _| host.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().number(key))).unwrap_or(0.0)
}

/// The value the open effect dialog holds for `key`, as JSON.
pub(super) fn dialog_json(app: &mut VectorcraftApp, key: &str) -> Option<Value> {
    let id = effect_dialog_id(app);
    with_host(app, |host, _| host.get_by_id::<HostedParamDialog>(id).and_then(|d| d.fields().get(key)).and_then(|v| serde_json::to_value(v).ok()))
}

/// Sets `key` of the open effect dialog as the user editing that field would.
pub(super) fn set_dialog(app: &mut VectorcraftApp, key: &str, value: Value) {
    let id = effect_dialog_id(app);
    with_host(app, |host, ctx| host.set_json_field(ctx, id, key, value)).unwrap_or_else(|e| panic!("{key}: {e}"));
}

/// Presses OK on the open effect dialog and runs the frames that answer its held OK.
pub(super) fn confirm_dialog(app: &mut VectorcraftApp) {
    let id = effect_dialog_id(app);
    with_host(app, |host, ctx| host.confirm_by_id(ctx, id));
    app_frame(app);
    app_frame(app);
}

/// Cancels the open effect dialog and runs the frame that ends its preview.
pub(super) fn cancel_dialog(app: &mut VectorcraftApp) {
    let id = effect_dialog_id(app);
    with_host(app, |host, ctx| host.cancel_by_id(ctx, id));
    app_frame(app);
}

/// Answers the open Edit-or-Add-New question with `choice` (Edit is `Proceed`, Add New `Alternate`).
pub(super) fn answer_prompt(app: &mut VectorcraftApp, choice: PromptChoice) {
    with_host(app, |host, ctx| {
        let id = host.stack().top().expect("the Edit or Add New question is open");
        if let Some(prompt) = host.get_mut_by_id::<StandardPrompt>(id) {
            prompt.set_choice(choice);
        }
        host.confirm_by_id(ctx, id);
    });
    app_frame(app);
}

/// A harness over the Appearance panel of `app`, with the rows `open` disclosed.
fn appearance_panel(mut app: VectorcraftApp, open: &[appearance::RowKey]) -> Panel {
    // PLAN: assumed `RowKey` with `Sel`'s row variants (appearance-w6)
    app.ui.appearance_panel.open.extend(open.iter().cloned());
    Panel::new(app, appearance::show)
}

/// Runs a frame of `panel`, then whether it shows a control named `name`.
fn shows(panel: &mut Panel, name: &str) -> bool {
    panel.frame();
    panel.shows(name)
}

/// The Cmd+/ or Cmd+Alt+/ key pressed through the whole app frame.
fn press_slash(app: &mut VectorcraftApp, modifiers: egui::Modifiers) {
    let ev = egui::Event::Key { key: egui::Key::Slash, physical_key: None, pressed: true, repeat: false, modifiers };
    key_frame(app, &window(app), vec![ev]);
}

/// The name of the colour chip of the "Fill:" or "Stroke:" row.
fn chip_of(label: &str) -> String {
    // PLAN: assumed accessible name of the swatch chip
    format!("{} swatch", label.trim_end_matches(':'))
}

#[test]
fn stroke_panel_shows_the_active_rows_weight() {
    let mut app = app_with_rect();
    run(&mut app, "appearance.addStroke", json!({})); // [Fill, Stroke 1 pt, Stroke 1 pt]
    run(&mut app, "stroke.set", json!({"item": 1, "weight": 6}));
    let six = Unit::Points.format(6.0);
    let mut panel = Panel::new(app, stroke::show);
    // PLAN: assumed the weight field's accessible name
    let weight = |panel: &mut Panel| {
        panel.frame();
        panel.value("Weight")
    };
    assert_ne!(weight(&mut panel), Some(six.clone()), "top stroke shown");
    // Clicking the lower stroke row in the Appearance panel targets it.
    appearance::select_row(&mut panel.app(), appearance::Sel::Item(1));
    assert_eq!(panel.app().session.appearance_item(), Some(1));
    assert_eq!(weight(&mut panel), Some(six.clone()));
    {
        let app = panel.app();
        let (_, painted) = current_paints(&app);
        assert_eq!(painted, current_stroke(&app).unwrap().paint);
    }
    // The object row clears it.
    appearance::select_row(&mut panel.app(), appearance::Sel::None);
    assert_eq!(panel.app().session.appearance_item(), None);
    assert_ne!(weight(&mut panel), Some(six));
}

/// Cmd+/ and Cmd+Alt+/, pressed through the app's key stage, add a fill and a stroke to the selected
/// objects as the Appearance panel's Add New Fill and Add New Stroke buttons do, on a path and on
/// type. With nothing selected the buttons are off, and the keys leave the objects as they are.
#[test]
fn cmd_slash_adds_a_fill_and_cmd_alt_slash_a_stroke_as_the_panel_buttons_do() {
    /// A rectangle and a line of type, with the rectangle (`type_too`: both) selected.
    fn two_objects(type_too: bool) -> (VectorcraftApp, Vec<Value>) {
        let mut app = app_with_rect();
        let rect = first_selected(&app).expect("the rectangle").id.0;
        let text = run(&mut app, "text.create", json!({"x": 10, "y": 150, "text": "Hi"}))["id"].clone();
        let ids = if type_too { vec![json!(rect), text.clone()] } else { vec![json!(rect)] };
        run(&mut app, "select.set", json!({ "ids": ids }));
        (app, vec![json!(rect), text])
    }
    let stacks = |app: &VectorcraftApp, ids: &[Value]| -> Vec<vectorcraft_doc::Appearance> {
        let d = &app.session.active().expect("a document").doc;
        ids.iter().map(|id| (*d.node(vectorcraft_doc::NodeId(id.as_u64().expect("an id"))).expect("the object").appearance).clone()).collect()
    };
    let cmd = egui::Modifiers::COMMAND;
    for (type_too, mods, label, fill) in [
        (false, cmd, "Add New Fill", true),
        (false, cmd | egui::Modifiers::ALT, "Add New Stroke", false),
        (true, cmd, "Add New Fill", true),
        (true, cmd | egui::Modifiers::ALT, "Add New Stroke", false),
    ] {
        let (mut keyed, ids) = two_objects(type_too);
        let before = stacks(&keyed, &ids);
        press_slash(&mut keyed, mods);
        let (clicked, _) = two_objects(type_too);
        let mut panel = Panel::new(clicked, appearance::show);
        panel.frame();
        assert!(panel.enabled(label), "{label} is on with a selection");
        panel.click(label);
        let after = stacks(&keyed, &ids);
        assert_eq!(after, stacks(&panel.app(), &ids), "{label}: the key adds what the button adds");
        let selected = if type_too { 2 } else { 1 };
        for (i, (a, b)) in after.iter().zip(&before).enumerate() {
            let added = if i < selected { 1 } else { 0 };
            assert_eq!(a.items.len(), b.items.len() + added, "{label}, object {i}: {a:?}");
            if added == 1 {
                assert_eq!(a.items.last().map(|it| it.is_fill()), Some(fill), "{label}: on top of the stack");
            }
        }
    }
    // With nothing selected, both buttons are off and neither key changes the objects.
    let (mut app, ids) = two_objects(true);
    run(&mut app, "select.none", json!({}));
    let before = stacks(&app, &ids);
    let mut panel = Panel::new(app, appearance::show);
    panel.frame();
    assert!(!panel.enabled("Add New Fill") && !panel.enabled("Add New Stroke"));
    press_slash(&mut panel.app(), cmd);
    press_slash(&mut panel.app(), cmd | egui::Modifiers::ALT);
    assert_eq!(stacks(&panel.app(), &ids), before);
}

/// With nothing selected, Add New Fill and Add New Stroke are off in menus and the command palette
/// as the Appearance panel's buttons are, and their keys run nothing (no error in the status bar);
/// an empty `ids` list runs nothing either. Given ids, they still add to those objects.
#[test]
fn add_new_fill_and_stroke_are_off_without_a_selection() {
    let mut app = app_with_rect();
    let rect = first_selected(&app).expect("the rectangle").id;
    let items = |app: &VectorcraftApp| app.session.active().expect("a document").doc.node(rect).expect("the rectangle").appearance.items.len();
    assert!(crate::menus::enabled(&app, "appearance.addFill") && crate::menus::enabled(&app, "appearance.addStroke"));
    run(&mut app, "select.none", json!({}));
    let before = items(&app);
    for (id, modifiers) in
        [("appearance.addFill", egui::Modifiers::COMMAND), ("appearance.addStroke", egui::Modifiers::COMMAND | egui::Modifiers::ALT)]
    {
        assert!(!crate::menus::enabled(&app, id), "{id} is off");
        press_slash(&mut app, modifiers);
        crate::menus::invoke(&mut app, id, json!({}));
        assert_eq!((items(&app), app.ui.status.as_str()), (before, ""), "{id} ran nothing");
        crate::menus::invoke(&mut app, id, json!({"ids": []}));
        assert_eq!((items(&app), app.ui.status.as_str()), (before, ""), "{id} with no ids ran nothing");
    }
    crate::menus::invoke(&mut app, "appearance.addFill", json!({"ids": [rect.0]}));
    assert_eq!((items(&app), app.ui.status.as_str()), (before + 1, ""), "given ids, it adds a fill");
}

#[test]
fn transparency_panel_reads_the_active_item() {
    let mut app = app_with_rect();
    run(&mut app, "transparency.set", json!({"item": 0, "opacity": 40}));
    assert_eq!(current_transparency(&app).unwrap().0, 1.0);
    run(&mut app, "appearance.setActiveItem", json!({"index": 0}));
    assert!((current_transparency(&app).unwrap().0 - 0.4).abs() < 1e-6);
    let mut panel = Panel::new(app, transparency::show);
    panel.frame();
    // PLAN: assumed the opacity field's accessible name
    assert!(panel.value("Opacity").is_some_and(|v| v.starts_with("40")), "{:?}", panel.value("Opacity"));
}

/// "toggle cells": the eye of an item-effect row hides and shows the effect.
#[test]
fn item_effect_rows_toggle_their_eye() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    // The stroke row's disclosure is open: its Opacity sub-row and its effect show under it.
    let mut panel = appearance_panel(app, &[appearance::RowKey::Item(1)]);
    // A row's eye cell is named for what it shows and hides.
    let eye = "Visibility Roughen";
    let visible = |panel: &Panel| current_stroke_effects(&panel.app())[0].visible;
    panel.click(eye);
    assert!(!visible(&panel));
    panel.click(eye);
    assert!(visible(&panel));
    // Clicking the effect's row selects it and makes its stroke the active item.
    panel.click("Roughen");
    assert_eq!(panel.app().session.appearance_item(), Some(1));
}

/// The effects of the selected object's stroke (item 1).
fn current_stroke_effects(app: &VectorcraftApp) -> Vec<vectorcraft_doc::Effect> {
    first_selected(app).unwrap().appearance.items[1].effects().clone()
}

/// The object row names the selection as the Control bar does.
#[test]
fn object_row_names_the_selection_like_the_control_bar() {
    let app = app_with_rect();
    let a = first_selected(&app).unwrap().id.0;
    let mut panel = Panel::new(app, appearance::show);
    assert!(shows(&mut panel, "Rectangle"));
    let b = run(&mut panel.app(), "shape.rectangle", json!({"x": 120, "y": 10, "width": 50, "height": 50}))["id"].clone();
    // Objects of one kind with one appearance are named by their kind and list it (#4174).
    run(&mut panel.app(), "select.set", json!({"ids": [a, b]}));
    assert!(shows(&mut panel, "Rectangle") && shows(&mut panel, "Stroke:"));
    assert!(!shows(&mut panel, "Mixed Appearances"));
    // Objects whose opacity differs list what they share.
    run(&mut panel.app(), "select.set", json!({"ids": [b]}));
    run(&mut panel.app(), "transparency.set", json!({"opacity": 50}));
    run(&mut panel.app(), "select.set", json!({"ids": [a, b]}));
    assert!(shows(&mut panel, "Mixed Appearances") && shows(&mut panel, "Stroke:"));
    // Type objects are "Type".
    let t = run(&mut panel.app(), "text.create", json!({"x": 10, "y": 150, "text": "Hi"}))["id"].clone();
    run(&mut panel.app(), "select.set", json!({ "ids": [t] }));
    assert_eq!(appearance::object_label(&panel.app()), "Type");
    assert!(shows(&mut panel, "Type"));
}

/// The selected object's fills/strokes as (is fill, stroke weight).
fn stack_items(app: &VectorcraftApp) -> Vec<(bool, Option<f32>)> {
    first_selected(app)
        .unwrap()
        .appearance
        .items
        .iter()
        .map(|it| match it {
            vectorcraft_doc::AppearanceItem::Stroke(s) => (false, Some(s.width as f32)),
            _ => (true, None),
        })
        .collect()
}

const DELETE: &str = "Delete Selected Item";
const DUPLICATE: &str = "Duplicate Selected Item";

/// A point of the row named by the link `name`, past the link's text: a press on the link follows it, so
/// production's tests pressed rows there.
pub(super) fn row_spot(panel: &mut Panel, name: &str) -> egui::Pos2 {
    panel.frame();
    let link = panel.rect_role(name, Role::LINK);
    egui::pos2(link.right() + 8.0, link.center().y)
}

/// Clicks the row named by the link `name` past its text, with `mods` held.
pub(super) fn click_row(panel: &mut Panel, name: &str, mods: egui::Modifiers) {
    let at = row_spot(panel, name);
    panel.click_at_with(at, mods);
}

/// Drags the row named by the link `from`, pressed past its text, onto the control named `to`.
pub(super) fn drag_row(panel: &mut Panel, from: &str, to: &str, mods: egui::Modifiers) {
    let from = row_spot(panel, from);
    let to = panel.rect(to).center();
    panel.drag_between(from, to, mods);
}

/// "drop on footer button": fill and stroke rows dropped on Delete Selected Item are removed, on
/// Duplicate Selected Item copied, as rows dropped on the Layers panel's buttons are.
#[test]
fn rows_dropped_on_delete_are_removed_and_on_duplicate_copied() {
    // [Fill, Stroke]: dropping the stroke on Delete leaves the fill.
    let mut panel = appearance_panel(app_with_rect(), &[]);
    drag_row(&mut panel, "Stroke:", DELETE, egui::Modifiers::NONE);
    assert_eq!(stack_items(&panel.app()), [(true, None)]);
    // Dropping the fill on Duplicate adds one copy of it.
    let mut panel = appearance_panel(app_with_rect(), &[]);
    drag_row(&mut panel, "Fill:", DUPLICATE, egui::Modifiers::NONE);
    let items = stack_items(&panel.app());
    assert_eq!(items.iter().filter(|(f, _)| *f).count(), 2, "{items:?}");
    assert_eq!(items.len(), 3, "{items:?}");
}

/// Dragging one of the selected rows onto Delete removes every selected row; dragging a row outside
/// the selection removes only that row.
#[test]
fn dropping_a_selected_row_acts_on_the_selection() {
    let setup = || {
        // [Fill, Stroke] and the object's Twist; the fill and the stroke are highlighted.
        let mut app = app_with_rect();
        run(&mut app, "effect.apply", json!({"effect": "distort.twist", "item": null}));
        let mut panel = appearance_panel(app, &[]);
        click_row(&mut panel, "Stroke:", egui::Modifiers::NONE);
        click_row(&mut panel, "Fill:", egui::Modifiers::COMMAND);
        panel
    };
    // The stroke is in the selection: the fill and the stroke go, Twist stays.
    let mut panel = setup();
    drag_row(&mut panel, "Stroke:", DELETE, egui::Modifiers::NONE);
    assert!(stack_items(&panel.app()).is_empty());
    assert_eq!(object_effects(&panel.app()), ["distort.twist"]);
    // Twist is not: only it goes.
    let mut panel = setup();
    drag_row(&mut panel, "Twist", DELETE, egui::Modifiers::NONE);
    assert_eq!(stack_items(&panel.app()).len(), 2);
    assert!(object_effects(&panel.app()).is_empty());
    // Onto Duplicate, the selected rows are both copied.
    let mut panel = setup();
    drag_row(&mut panel, "Stroke:", DUPLICATE, egui::Modifiers::NONE);
    assert_eq!(stack_items(&panel.app()).len(), 4);
}

/// An effect row dropped on Delete removes that effect, on Duplicate copies it; the stroke it
/// belongs to stays.
#[test]
fn effect_rows_dropped_on_delete_or_duplicate() {
    for (button, left) in [(DELETE, 0), (DUPLICATE, 2)] {
        let mut app = app_with_rect();
        run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
        let mut panel = appearance_panel(app, &[appearance::RowKey::Item(1)]);
        panel.drag("Roughen", button, egui::Modifiers::NONE);
        assert_eq!(current_stroke_effects(&panel.app()).len(), left, "{button}");
        assert_eq!(stack_items(&panel.app()).len(), 2, "{button}: the fill and stroke stay");
    }
}

/// The Contents row can be neither copied nor removed: dropped on either button it changes nothing.
/// A row dropped on something that is not a button changes nothing either, and the next drop still works.
#[test]
fn contents_rows_and_drops_elsewhere_change_nothing() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
    let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [a, b]}));
    run(&mut app, "object.group", json!({}));
    run(&mut app, "appearance.addFill", json!({}));
    let mut panel = appearance_panel(app, &[]);
    let before = first_selected(&panel.app()).unwrap().appearance.clone();
    for button in [DELETE, DUPLICATE] {
        panel.drag("Contents", button, egui::Modifiers::NONE);
        assert_eq!(first_selected(&panel.app()).unwrap().appearance, before, "{button}");
    }
    drag_row(&mut panel, "Fill:", "Contents", egui::Modifiers::NONE);
    assert_eq!(first_selected(&panel.app()).unwrap().appearance, before);
    drag_row(&mut panel, "Fill:", DELETE, egui::Modifiers::NONE);
    assert!(first_selected(&panel.app()).unwrap().appearance.items.is_empty());
}

/// The selected object's own effect ids, in order.
fn object_effects(app: &VectorcraftApp) -> Vec<String> {
    first_selected(app).unwrap().appearance.effects.iter().map(|e| e.id.clone()).collect()
}

/// A rectangle with Outer Glow, Drop Shadow and Twist on the object.
fn rect_with_three_effects() -> VectorcraftApp {
    let mut app = app_with_rect();
    for e in ["stylize.outerGlow", "stylize.dropShadow", "distort.twist"] {
        run(&mut app, "effect.apply", json!({"effect": e, "item": null}));
    }
    app
}

/// "Delete keys": Ctrl-clicking two of the object's effects highlights both, and Delete removes both
/// in one undo step, leaving the fill and stroke.
#[test]
fn ctrl_clicked_effects_are_deleted_together() {
    let mut panel = appearance_panel(rect_with_three_effects(), &[]);
    panel.click_with("Outer Glow", egui::Modifiers::NONE);
    panel.click_with("Drop Shadow", egui::Modifiers::COMMAND);
    panel.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert_eq!(object_effects(&panel.app()), ["distort.twist"]);
    assert_eq!(stack_items(&panel.app()).len(), 2, "the fill and stroke stay");
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_eq!(object_effects(&panel.app()).len(), 3, "one undo step");
}

/// Ctrl-clicking an effect of another list (the stroke's) starts a new selection there: Delete then
/// removes only that effect.
#[test]
fn ctrl_clicking_an_effect_of_another_list_starts_a_new_selection() {
    let mut app = rect_with_three_effects();
    run(&mut app, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    let mut panel = appearance_panel(app, &[appearance::RowKey::Item(1)]);
    click_row(&mut panel, "Outer Glow", egui::Modifiers::NONE);
    click_row(&mut panel, "Roughen", egui::Modifiers::COMMAND);
    panel.click(DELETE);
    assert!(current_stroke_effects(&panel.app()).is_empty());
    assert_eq!(object_effects(&panel.app()).len(), 3);
}

/// "row reorder": Alt held at the release copies both highlighted effects onto the stroke; the
/// originals stay.
#[test]
fn alt_dragging_highlighted_effects_copies_them_together() {
    let mut panel = appearance_panel(rect_with_three_effects(), &[]);
    click_row(&mut panel, "Outer Glow", egui::Modifiers::NONE);
    click_row(&mut panel, "Twist", egui::Modifiers::COMMAND);
    drag_row(&mut panel, "Outer Glow", "Stroke:", egui::Modifiers::ALT);
    let ids: Vec<_> = current_stroke_effects(&panel.app()).iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["stylize.outerGlow", "distort.twist"]);
    assert_eq!(object_effects(&panel.app()).len(), 3, "the originals stay");
}

/// "row link to popup": the name of an effect row opens the effect's settings dialog on a click, with
/// or without Ctrl/Cmd or Shift held.
#[test]
fn an_effects_name_edits_it_and_its_empty_space_highlights_it() {
    for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
        let mut panel = appearance_panel(rect_with_three_effects(), &[]);
        panel.click_role_with("Outer Glow", Role::LINK, m);
        assert!(dialog_open(&panel.app()), "{m:?}: a click on the name opens the dialog");
    }
    // PLAN: the row's empty space has no name, so the single Ctrl-click and the double-click there need a point driver on op.A15
}

#[test]
fn transform_effect_name_edits_the_displayed_appearance_target() {
    for target_kind in ["object", "group", "layer"] {
        let mut app = app_with_rect();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        if target_kind == "group" {
            let group = app.run("object.group", json!({})).unwrap()["id"].clone();
            app.run("layer.target", json!({"id": group})).unwrap();
        } else if target_kind == "layer" {
            let layer = app.session.doc().unwrap().doc.layers[0].id;
            app.run("layer.target", json!({"id": layer.0})).unwrap();
        }
        let target = app.session.doc().unwrap().selection.subjects()[0];
        app.run("effect.apply", json!({"effect": "distort.transform", "item": null, "params": {"moveH": 12}})).unwrap();
        let mut panel = Panel::new(app, appearance::show);
        panel.click_role("Transform", Role::LINK);
        let mut app = panel.app();
        assert!(dialog_open(&app), "Transform did not open for {target_kind}");
        assert_eq!(dialog_number(&mut app, "moveH"), 12.0);
        set_dialog(&mut app, "moveH", json!(35));
        app_frame(&mut app);
        assert!(app.session.in_interaction());
        let rendered = vectorcraft_testkit::raster::render_artboard(&app.session.doc().unwrap().doc);
        assert_eq!(rendered.over_white(30, 50), [255, 255, 255], "{target_kind}: original position");
        assert_eq!(rendered.over_white(130, 50), [255, 0, 0], "{target_kind}: transformed position");
        confirm_dialog(&mut app);
        let node = app.session.doc().unwrap().doc.node(target).unwrap();
        assert_eq!(node.appearance.effects.len(), 1);
        assert_eq!(node.appearance.effects[0].params["moveH"].as_f64(), Some(35.0));
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.node(target).unwrap().appearance.effects[0].params["moveH"].as_f64(), Some(12.0));
    }
}

#[test]
fn transform_effect_dialog_reads_the_selected_compound_members_owner() {
    for item in [None, Some(0)] {
        let mut app = app_with_rect();
        let member = app.session.doc().unwrap().selection.subjects()[0];
        let other = app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [member.0, other]})).unwrap();
        let compound = app.run("object.compoundPath.make", json!({})).unwrap()["id"].as_u64().unwrap();
        app.run("effect.apply", json!({"effect": "distort.transform", "item": item, "params": {"moveH": 12, "copies": 2}})).unwrap();
        app.run("select.set", json!({"ids": [member.0]})).unwrap();
        app.run("effect.dialog", json!({"effect": "distort.transform", "item": item, "index": 0})).unwrap();
        assert_eq!(dialog_number(&mut app, "moveH"), 12.0);
        assert_eq!(dialog_number(&mut app, "copies"), 2.0);
        set_dialog(&mut app, "moveH", json!(35));
        app_frame(&mut app);
        confirm_dialog(&mut app);
        let effects = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(compound)).unwrap().appearance;
        assert_eq!(effects.effects_at(item).unwrap()[0].params["moveH"].as_f64(), Some(35.0));
        assert_eq!(effects.effects_at(item).unwrap()[0].params["copies"].as_f64(), Some(2.0));
        app.run("edit.undo", json!({})).unwrap();
        let reply = app.run("effect.dialog", json!({"effect": "distort.transform", "item": item})).unwrap();
        assert_eq!(reply, json!({"pending": "effectExists"}));
        answer_prompt(&mut app, PromptChoice::Proceed);
        assert_eq!(dialog_number(&mut app, "moveH"), 12.0);
        set_dialog(&mut app, "moveH", json!(60));
        app_frame(&mut app);
        cancel_dialog(&mut app);
        let effects = &app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(compound)).unwrap().appearance;
        assert_eq!(effects.effects_at(item).unwrap()[0].params["moveH"].as_f64(), Some(12.0));
    }
}

#[test]
fn transform_effect_dialog_keeps_the_active_items_selection_anchor() {
    for item in [None, Some(json!(0)), Some(Value::Null)] {
        let mut app = app_with_rect();
        let back = app.session.doc().unwrap().selection.subjects()[0];
        for at in [json!(0), Value::Null] {
            app.run("effect.apply", json!({"effect": "distort.transform", "item": at, "params": {"moveH": 99}})).unwrap();
        }
        let front = app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 20, "height": 20})).unwrap()["id"].clone();
        for at in [json!(0), Value::Null] {
            app.run("effect.apply", json!({"effect": "distort.transform", "item": at, "params": {"moveH": 12}})).unwrap();
        }
        app.run("select.set", json!({"ids": [front, back.0]})).unwrap();
        app.run("appearance.setActiveItem", json!({"index": 0})).unwrap();
        let mut p = json!({"effect": "distort.transform"});
        if let Some(item) = &item {
            p["item"] = item.clone();
        }
        assert_eq!(app.run("effect.dialog", p).unwrap(), json!({"pending": "effectExists"}));
        answer_prompt(&mut app, PromptChoice::Proceed);
        let expected = if item.is_none() { 12.0 } else { 99.0 };
        assert_eq!(dialog_number(&mut app, "moveH"), expected);
    }
}

#[test]
fn explicit_effect_items_use_paint_order_with_a_selected_compound_member() {
    let mut app = app_with_rect();
    let back = app.session.doc().unwrap().selection.subjects()[0];
    for item in [json!(0), Value::Null] {
        app.run("effect.apply", json!({"effect": "distort.transform", "item": item, "params": {"moveH": 99}})).unwrap();
    }
    let member = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 80, "height": 80})).unwrap()["id"].clone();
    let inner = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [member, inner]})).unwrap();
    app.run("object.compoundPath.make", json!({})).unwrap();
    for item in [json!(0), Value::Null] {
        app.run("effect.apply", json!({"effect": "distort.transform", "item": item, "params": {"moveH": 12}})).unwrap();
    }
    app.run("select.set", json!({"ids": [member, back.0]})).unwrap();
    for item in [json!(0), Value::Null] {
        app.run("effect.dialog", json!({"effect": "distort.transform", "item": item, "index": 0})).unwrap();
        assert_eq!(dialog_number(&mut app, "moveH"), 99.0);
        cancel_dialog(&mut app);
    }
}

#[test]
fn editing_an_active_effect_keeps_aligned_rows_in_mixed_stacks() {
    let mut app = app_with_rect();
    let back = app.session.doc().unwrap().selection.subjects()[0];
    app.run("appearance.addStroke", json!({})).unwrap();
    app.run("appearance.moveItem", json!({"from": 2, "to": 0})).unwrap();
    app.run("effect.apply", json!({"effect": "distort.transform", "item": 0, "params": {"moveH": 99}})).unwrap();
    app.run("effect.apply", json!({"effect": "distort.transform", "item": 1, "params": {"moveH": 12}})).unwrap();
    let front = app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "distort.transform", "item": 0, "params": {"moveH": 12}})).unwrap();
    app.run("select.set", json!({"ids": [front, back.0]})).unwrap();
    app.run("appearance.setActiveItem", json!({"index": 0})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.transform", "index": 0})).unwrap();
    assert_eq!(dialog_number(&mut app, "moveH"), 12.0);
    set_dialog(&mut app, "moveH", json!(35));
    app.run("appearance.setActiveItem", json!({"index": null})).unwrap();
    app_frame(&mut app);
    confirm_dialog(&mut app);
    let doc = &app.session.doc().unwrap().doc;
    let front = &doc.node(vectorcraft_doc::NodeId(front)).unwrap().appearance;
    let back = &doc.node(back).unwrap().appearance;
    assert_eq!(front.effects_at(Some(0)).unwrap()[0].params["moveH"], 35);
    assert_eq!(back.effects_at(Some(1)).unwrap()[0].params["moveH"], 35);
    assert_eq!(back.effects_at(Some(0)).unwrap()[0].params["moveH"], 99);
}

#[test]
fn editing_an_effect_keeps_its_original_item_when_the_active_row_changes() {
    for prompt in [false, true] {
        let mut app = app_with_rect();
        let id = app.session.doc().unwrap().selection.subjects()[0];
        app.run("effect.apply", json!({"effect": "distort.transform", "item": null, "params": {"moveH": 99}})).unwrap();
        app.run("effect.apply", json!({"effect": "distort.transform", "item": 0, "params": {"moveH": 12}})).unwrap();
        app.run("appearance.setActiveItem", json!({"index": 0})).unwrap();
        let mut p = json!({"effect": "distort.transform"});
        if !prompt {
            p["index"] = json!(0);
        }
        app.run("effect.dialog", p).unwrap();
        app.run("appearance.setActiveItem", json!({"index": null})).unwrap();
        if prompt {
            answer_prompt(&mut app, PromptChoice::Proceed);
        }
        assert_eq!(dialog_number(&mut app, "moveH"), 12.0);
        set_dialog(&mut app, "moveH", json!(35));
        app_frame(&mut app);
        confirm_dialog(&mut app);
        let appearance = &app.session.doc().unwrap().doc.node(id).unwrap().appearance;
        assert_eq!(appearance.effects_at(Some(0)).unwrap()[0].params["moveH"].as_f64(), Some(35.0));
        assert_eq!(appearance.effects[0].params["moveH"].as_f64(), Some(99.0));
    }
}

#[test]
fn new_effects_keep_implicit_item_targeting_until_confirmation() {
    for prompt in [false, true] {
        let mut app = app_with_rect();
        let id = app.session.doc().unwrap().selection.subjects()[0];
        app.run("effect.apply", json!({"effect": "distort.transform", "item": null, "params": {"moveH": 99}})).unwrap();
        app.run("effect.apply", json!({"effect": "distort.transform", "item": 0, "params": {"moveH": 12}})).unwrap();
        app.run("appearance.setActiveItem", json!({"index": 0})).unwrap();
        app.run("effect.dialog", json!({"effect": if prompt { "distort.transform" } else { "distort.roughen" }})).unwrap();
        app.run("appearance.setActiveItem", json!({"index": null})).unwrap();
        if prompt {
            answer_prompt(&mut app, PromptChoice::Alternate);
        }
        confirm_dialog(&mut app);
        let appearance = &app.session.doc().unwrap().doc.node(id).unwrap().appearance;
        assert_eq!(appearance.effects_at(Some(0)).unwrap().len(), 1);
        assert_eq!(appearance.effects.len(), 2);
    }
}

#[test]
fn transform_effect_preview_commit_cancel_and_undo_cover_artwork_targets() {
    use vectorcraft_testkit::raster::{Image, render_artboard};
    let rightmost =
        |image: &Image| (0..image.width).rev().find(|&x| (0..image.height).any(|y| image.over_white(x, y).iter().any(|&c| c < 245))).unwrap();
    for kind in ["object", "fill", "stroke", "group", "layer", "text", "image"] {
        for preview in [true, false] {
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
            app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
            match kind {
                "text" => {
                    app.run("text.create", json!({"x": 20, "y": 80, "size": 20, "text": "HI"})).unwrap();
                }
                "image" => {
                    let mut png = Vec::new();
                    {
                        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(20, 20, image::Rgba([255, 0, 0, 255]))).into();
                        let (__w, __h) = (__dyn.width(), __dyn.height());
                        let (__layout, __sample, __raw) = match __dyn {
                            image::DynamicImage::ImageLuma8(b) => {
                                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw())
                            }
                            image::DynamicImage::ImageLumaA8(b) => {
                                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw())
                            }
                            image::DynamicImage::ImageRgb8(b) => {
                                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw())
                            }
                            image::DynamicImage::ImageRgba8(b) => {
                                (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw())
                            }
                            image::DynamicImage::ImageLuma16(b) => (
                                craft_image_codecs::ChannelLayout::Gray,
                                craft_image_codecs::SampleType::U16,
                                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                            ),
                            image::DynamicImage::ImageLumaA16(b) => (
                                craft_image_codecs::ChannelLayout::GrayA,
                                craft_image_codecs::SampleType::U16,
                                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                            ),
                            image::DynamicImage::ImageRgb16(b) => (
                                craft_image_codecs::ChannelLayout::Rgb,
                                craft_image_codecs::SampleType::U16,
                                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                            ),
                            image::DynamicImage::ImageRgba16(b) => (
                                craft_image_codecs::ChannelLayout::Rgba,
                                craft_image_codecs::SampleType::U16,
                                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
                            ),
                            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
                        };
                        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
                        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
                            .map(|__b| png.extend_from_slice(&__b))
                    }
                    .unwrap();
                    app.run(
                        "file.place",
                        json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "link": false, "at": [30, 70]}),
                    )
                    .unwrap();
                }
                _ => {
                    app.run("shape.rectangle", json!({"x": 20, "y": 60, "width": 20, "height": 20})).unwrap();
                }
            }
            if kind != "image" {
                app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
            }
            if kind == "stroke" {
                app.run("paint.setFill", json!({"none": true})).unwrap();
                app.run("stroke.set", json!({"weight": 4})).unwrap();
            }
            if kind == "group" {
                app.run("object.group", json!({})).unwrap();
            } else if kind == "layer" {
                let layer = app.session.doc().unwrap().doc.layers[0].id;
                app.run("layer.target", json!({"id": layer.0})).unwrap();
            }
            let item = match kind {
                "fill" => Some(0),
                "stroke" => Some(1),
                _ => None,
            };
            app.run("effect.apply", json!({"effect": "distort.transform", "item": item})).unwrap();
            let baseline = render_artboard(&app.session.doc().unwrap().doc);
            let mut panel = Panel::new(app, appearance::show);
            // The row's disclosure shows its effect rows.
            if let Some(i) = item {
                panel.click(if i == 0 { "Expand Fill" } else { "Expand Stroke" });
            }
            panel.click_role("Transform", Role::LINK);
            let mut app = panel.app();
            assert!(dialog_open(&app), "{kind}: dialog did not open");
            set_dialog(&mut app, "moveH", json!(40));
            set_dialog(&mut app, "copies", json!(2));
            set_dialog(&mut app, "preview", json!(preview));
            app_frame(&mut app);
            let painted = render_artboard(&app.session.doc().unwrap().doc);
            if preview {
                assert!(rightmost(&painted) >= rightmost(&baseline) + 75, "{kind}: preview must draw both copies");
            } else {
                assert_eq!(painted.rgba, baseline.rgba, "{kind}: Preview off");
            }
            confirm_dialog(&mut app);
            assert!(!dialog_open(&app), "{kind}: OK closes the dialog");
            let committed = render_artboard(&app.session.doc().unwrap().doc);
            assert!(rightmost(&committed) >= rightmost(&baseline) + 75, "{kind}: OK must draw both copies");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(render_artboard(&app.session.doc().unwrap().doc).rgba, baseline.rgba, "{kind}: undo");
            let undo = app.session.active().unwrap().history.undo.len();
            app.run("effect.dialog", json!({"effect": "distort.transform", "item": item, "index": 0})).unwrap();
            set_dialog(&mut app, "moveH", json!(40));
            set_dialog(&mut app, "copies", json!(2));
            app_frame(&mut app);
            cancel_dialog(&mut app);
            assert_eq!(render_artboard(&app.session.doc().unwrap().doc).rgba, baseline.rgba, "{kind}: Cancel");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{kind}: Cancel must preserve undo history");
        }
    }
}

/// "row link to popup": a fill or stroke row's label ("Fill:", "Stroke:") is a link: a click opens the
/// Color or Stroke panel with or without modifiers.
#[test]
fn fill_and_stroke_labels_are_links_and_their_empty_space_only_highlights() {
    for (label, panel_name) in [("Stroke:", "stroke"), ("Fill:", "color")] {
        for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
            let mut panel = appearance_panel(app_with_rect(), &[]);
            panel.click_with(label, m);
            assert_eq!(panel.app().ui.transient_panel.as_deref(), Some(panel_name), "{label} {m:?}");
        }
    }
    // PLAN: the row's empty space has no name; its refusals need a point driver on op.A15
}

/// "Delete keys": after a click on a group's Contents row or a type object's Characters link, Delete
/// stays with the panel: the row can't be removed, so nothing is.
#[test]
fn after_a_contents_click_delete_removes_nothing() {
    let group = || {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let a = run(&mut app, "shape.rectangle", json!({"x": 0, "y": 0, "width": 20, "height": 20}))["id"].clone();
        let b = run(&mut app, "shape.rectangle", json!({"x": 30, "y": 0, "width": 20, "height": 20}))["id"].clone();
        run(&mut app, "select.set", json!({"ids": [a, b]}));
        run(&mut app, "object.group", json!({}));
        run(&mut app, "appearance.addFill", json!({}));
        app
    };
    let type_object = || {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        run(&mut app, "file.new", json!({"width": 100, "height": 100}));
        let t = run(&mut app, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].clone();
        run(&mut app, "select.set", json!({"ids": [t]}));
        app
    };
    for (app, label) in [(group(), "Contents"), (type_object(), "Characters")] {
        let before = first_selected(&app).unwrap().clone();
        let mut panel = Panel::new(app, appearance::show);
        panel.click(label);
        panel.key(egui::Key::Delete, egui::Modifiers::NONE);
        let after = first_selected(&panel.app()).unwrap_or_else(|| panic!("{label}: the object stays"));
        assert_eq!(after.appearance, before.appearance, "{label}: nothing is removed");
    }
}

/// A tool that claims Delete and Backspace (the Artboard tool's active artboard) leaves them to a
/// focused Layers row.
#[test]
fn after_a_row_click_delete_reaches_the_layers_panel_not_the_artboard_tool() {
    for key in [egui::Key::Delete, egui::Key::Backspace] {
        let mut app = app_with_rect();
        run(&mut app, "layer.new", json!({}));
        run(&mut app, "artboard.new", json!({"width": 200, "height": 100}));
        app.select_tool("artboard");
        let mut panel = Panel::new(app, layers::show);
        panel.click("Layer 2");
        panel.key(key, egui::Modifiers::NONE);
        let app = panel.app();
        let d = &app.session.active().unwrap().doc;
        assert_eq!((d.layers.len(), d.artboards.len()), (1, 2), "{key:?}: Layer 2 is deleted, both artboards stay");
    }
}

/// "row link to popup": "Opacity:" is a link to the Transparency panel, whatever modifiers are held:
/// on a stroke's Opacity row it makes that stroke the active row first; on the object's own Opacity
/// row no fill/stroke is active.
#[test]
fn the_opacity_label_opens_the_transparency_panel_on_its_row() {
    for m in [egui::Modifiers::NONE, egui::Modifiers::COMMAND, egui::Modifiers::SHIFT] {
        // With the stroke open there are two Opacity rows; the harness picks the first, the stroke's.
        let mut panel = appearance_panel(app_with_rect(), &[appearance::RowKey::Item(1)]);
        panel.click_with("Opacity:", m);
        {
            let app = panel.app();
            assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), Some(1)), "{m:?}");
        }
        // Closed, only the object's own Opacity row shows.
        let mut panel = appearance_panel(app_with_rect(), &[]);
        panel.click_with("Opacity:", m);
        let app = panel.app();
        assert_eq!((app.ui.transient_panel.as_deref(), app.session.appearance_item()), (Some("transparency"), None), "{m:?}");
    }
}

/// An Opacity row is no item: while one is highlighted Duplicate is greyed out, dragging it changes
/// nothing, and Delete resets it to Default without removing the object or a fill/stroke.
#[test]
fn an_opacity_row_cannot_be_deleted_duplicated_or_dragged() {
    let mut app = app_with_rect();
    run(&mut app, "transparency.set", json!({"item": 1, "opacity": 40}));
    appearance::select_row(&mut app, appearance::Sel::ItemOpacity(1));
    let before = stack_items(&app);
    let mut panel = appearance_panel(app, &[appearance::RowKey::Item(1)]);
    panel.frame();
    assert!(!panel.enabled(DUPLICATE), "Duplicate is greyed out");
    drag_row(&mut panel, "Opacity:", "Fill:", egui::Modifiers::NONE);
    assert_eq!(stack_items(&panel.app()), before, "dragged: nothing moves");
    panel.key(egui::Key::Delete, egui::Modifiers::NONE);
    assert!(first_selected(&panel.app()).is_some(), "the object stays");
    assert_eq!(stack_items(&panel.app()), before);
    assert_eq!(current_transparency(&panel.app()).unwrap().0, 1.0, "the opacity is back to Default");
}

/// A link to a panel that shows already (floating in front of its window) opens no popup: the
/// panel is in sight as it is.
#[test]
fn a_row_link_to_a_panel_in_sight_opens_no_popup() {
    let mut app = app_with_rect();
    app.run("window.panel.float", json!({"panel": "stroke"})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, "stroke"));
    let mut panel = Panel::new(app, appearance::show);
    panel.click("Stroke:");
    panel.frame();
    assert_eq!(panel.app().ui.transient_panel, None);
}

/// "row link to popup": on a type object "Characters" is a link to the Character panel.
#[test]
fn characters_opens_the_character_panel() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    run(&mut app, "file.new", json!({"width": 100, "height": 100}));
    let t = run(&mut app, "text.create", json!({"x": 0, "y": 60, "text": "Hi"}))["id"].clone();
    run(&mut app, "select.set", json!({"ids": [t]}));
    let mut panel = Panel::new(app, appearance::show);
    panel.click("Characters");
    assert_eq!(panel.app().ui.transient_panel.as_deref(), Some("character"));
}

/// "toggle cells": with nothing selected, the fill and stroke swatches work as an object's do: a
/// click opens the swatches, whose pick paints the next object drawn; Shift-click opens the Color
/// panel on that fill or stroke. "Fill:" and "Stroke:" make theirs the Color panel's paint too.
#[test]
fn with_nothing_selected_the_swatches_set_up_new_art() {
    let swatch = |app: &VectorcraftApp| app.session.active().unwrap().doc.swatches.iter().find(|s| !s.paint.is_none()).unwrap().clone();
    for (label, fill) in [("Fill:", true), ("Stroke:", false)] {
        let mut app = app_with_rect();
        run(&mut app, "select.set", json!({"ids": []}));
        let pick = swatch(&app);
        let mut panel = Panel::new(app, appearance::show);
        let chip = chip_of(label);
        panel.click(&chip);
        panel.click(&pick.name);
        {
            let app = panel.app();
            let ap = app.session.new_art();
            let item = ap.items.iter().rev().find(|it| it.is_fill() == fill).unwrap();
            assert_eq!(item.paint(), &pick.paint, "{label}: the pick paints new art");
        }
        panel.click_with(&chip, egui::Modifiers::SHIFT);
        {
            let app = panel.app();
            assert_eq!((app.ui.transient_panel.as_deref(), app.session.fill_active), (Some("color"), fill), "{label}: Shift-click");
        }
        panel.app().ui.transient_panel = None;
        run(&mut panel.app(), "paint.toggleActive", json!({"fill": !fill}));
        panel.click(label);
        let app = panel.app();
        assert_eq!(
            (app.ui.transient_panel.as_deref(), app.session.fill_active),
            (Some(if fill { "color" } else { "stroke" }), fill),
            "{label}: its link"
        );
    }
}

/// "weight target": the stroke row's weight is the same field with or without a selection: typed in,
/// it sets the selected stroke's weight, or with nothing selected the next object drawn's.
#[test]
fn the_stroke_weight_field_edits_with_or_without_a_selection() {
    for selected in [true, false] {
        let mut app = app_with_rect();
        if !selected {
            run(&mut app, "select.set", json!({"ids": []}));
        }
        let mut panel = Panel::new(app, appearance::show);
        // PLAN: assumed the weight field's accessible name
        panel.type_into("Weight", "6");
        let app = panel.app();
        let width =
            if selected { first_selected(&app).unwrap().appearance.stroke().unwrap().width } else { app.session.new_art().stroke().unwrap().width };
        assert_eq!(width, 6.0, "selected: {selected}");
    }
}

/// Dragging a highlighted effect onto Delete removes every highlighted effect, the active one too:
/// nothing is left active then, so the effect that moved up into the active one's place is not
/// taken for it (Delete Selected Item then removes nothing).
#[test]
fn a_drop_on_delete_that_removes_the_active_effect_leaves_no_effect_active() {
    let mut panel = appearance_panel(rect_with_three_effects(), &[]);
    // Drop Shadow, then Outer Glow with Ctrl: both highlighted, Outer Glow active.
    click_row(&mut panel, "Drop Shadow", egui::Modifiers::NONE);
    click_row(&mut panel, "Outer Glow", egui::Modifiers::COMMAND);
    drag_row(&mut panel, "Drop Shadow", DELETE, egui::Modifiers::NONE);
    assert_eq!(object_effects(&panel.app()), ["distort.twist"]);
    panel.click(DELETE);
    assert_eq!(object_effects(&panel.app()), ["distort.twist"], "Twist was never highlighted");
}

/// Dragging an effect that isn't highlighted onto Delete removes only it: the highlighted effects
/// below it move up with their rows, so they stay highlighted and active, and Delete Selected Item
/// then removes them.
#[test]
fn a_drop_on_delete_above_the_active_effect_keeps_the_highlight_on_its_rows() {
    let mut panel = appearance_panel(rect_with_three_effects(), &[]);
    // Drop Shadow, then Twist with Ctrl: both highlighted, Twist active.
    click_row(&mut panel, "Drop Shadow", egui::Modifiers::NONE);
    click_row(&mut panel, "Twist", egui::Modifiers::COMMAND);
    drag_row(&mut panel, "Outer Glow", DELETE, egui::Modifiers::NONE);
    assert_eq!(object_effects(&panel.app()), ["stylize.dropShadow", "distort.twist"]);
    panel.click(DELETE);
    assert!(object_effects(&panel.app()).is_empty(), "both highlighted effects removed: {:?}", object_effects(&panel.app()));
}

/// `first_selected` gives the `Arc` the document holds for the first selected object, nested in a
/// group too, not a copy of it; nothing with nothing selected or no document open.
#[test]
fn first_selected_is_the_node_the_document_holds() {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    assert!(first_selected(&app).is_none(), "no document");
    run(&mut app, "file.new", json!({"width": 200, "height": 200}));
    let a = run(&mut app, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].clone();
    let b = run(&mut app, "shape.ellipse", json!({"x": 70, "y": 10, "width": 50, "height": 50}))["id"].clone();
    let held = |app: &VectorcraftApp, id: &Value| layers::node_arc(&app.session.active().unwrap().doc, NodeId(id.as_u64().unwrap())).unwrap();
    run(&mut app, "select.set", json!({ "ids": [a, b] }));
    let id = app.session.active().unwrap().selection.objects[0];
    let first = first_selected(&app).expect("a selected object");
    assert_eq!(first.id, id, "the first of the selection");
    assert!(Arc::ptr_eq(&first, &held(&app, &json!(id.0))), "the first selected object, as the document holds it");
    run(&mut app, "object.group", json!({}));
    run(&mut app, "select.set", json!({ "ids": [a] }));
    assert!(Arc::ptr_eq(&first_selected(&app).expect("the rectangle"), &held(&app, &a)), "a node inside a group");
    run(&mut app, "select.none", json!({}));
    assert!(first_selected(&app).is_none(), "nothing selected");
}

/// Properties, Appearance and Transparency show the selected object's current opacity after an
/// edit and after Undo, never an earlier one: each panel is kept across the edits, as the app keeps it.
#[test]
fn panels_show_the_selected_object_after_an_edit_and_after_undo() {
    let mut panels = [
        ("Properties", Panel::new(app_with_rect(), properties::show)),
        ("Appearance", Panel::new(app_with_rect(), appearance::show)),
        ("Transparency", Panel::new(app_with_rect(), transparency::show)),
    ];
    // Runs `id` in every panel's app, then checks each panel shows `want` and not `not`.
    let mut edit_shows = |id: &str, p: Value, want: &str, not: &str| {
        for (name, panel) in &mut panels {
            run(&mut panel.app(), id, p.clone());
            panel.frame();
            // Appearance's Opacity row shows its percent as trailing text, which describes the row.
            let shown = panel.value("Opacity").filter(|v| !v.is_empty()).or_else(|| panel.description("Opacity")).unwrap_or_default();
            assert!(shown.contains(want), "{name} shows {want}: {shown}");
            assert!(!shown.contains(not), "{name} no longer shows {not}: {shown}");
        }
    };
    // A percent field's value is its number, with the % painted after it; the Appearance row reads "Opacity: 60%".
    edit_shows("transparency.set", json!({"opacity": 60}), "60", "35");
    edit_shows("transparency.set", json!({"opacity": 35}), "35", "60");
    edit_shows("edit.undo", json!({}), "60", "35");
}

/// Right-clicking a stroke row, on its label, picks it and opens its menu: Duplicate, Delete and Hide Stroke.
#[test]
fn right_clicking_a_stroke_row_selects_it_and_exposes_context_menu() {
    let mut panel = appearance_panel(app_with_rect(), &[]);
    assert_eq!(panel.app().session.appearance_item(), None);
    panel.secondary_click("Stroke:");
    assert_eq!(panel.app().session.appearance_item(), Some(1), "right-clicking the stroke row picked it");
    for name in ["Duplicate Stroke", "Delete Stroke", "Hide Stroke"] {
        assert!(panel.shows(name), "the menu has {name}");
    }
}

/// The menu's Duplicate Stroke copies the stroke in one undo step.
#[test]
fn clicking_duplicate_stroke_duplicates_the_stroke_with_one_undo_step() {
    let mut panel = appearance_panel(app_with_rect(), &[]);
    let before = stack_items(&panel.app());
    panel.secondary_click("Stroke:");
    panel.click("Duplicate Stroke");
    let items = stack_items(&panel.app());
    assert_eq!(items.len(), before.len() + 1, "{items:?}");
    assert_eq!(items.iter().filter(|(fill, _)| !*fill).count(), 2, "two strokes: {items:?}");
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_eq!(stack_items(&panel.app()), before, "one undo step");
}

/// Right-clicking an effect row, on its label, opens Edit Effect…, Duplicate, Delete and Hide Effect; Delete Effect
/// removes it in one undo step.
#[test]
fn clicking_delete_effect_removes_the_effect_from_appearance() {
    let mut app = app_with_rect();
    run(&mut app, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    let mut panel = appearance_panel(app, &[]);
    panel.secondary_click("Drop Shadow");
    for name in ["Edit Effect…", "Duplicate Effect", "Delete Effect", "Hide Effect"] {
        assert!(panel.shows(name), "the menu has {name}");
    }
    panel.click("Delete Effect");
    assert!(object_effects(&panel.app()).is_empty());
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_eq!(object_effects(&panel.app()), ["stylize.dropShadow"], "one undo step");
}

/// Right-clicking the object row picks it, so no fill or stroke stays active, and opens Clear Appearance and Reduce
/// to Basic Appearance; Clear Appearance clears it in one undo step.
#[test]
fn right_clicking_root_object_row_exposes_clear_and_reduce_appearance() {
    let mut app = app_with_rect();
    appearance::select_row(&mut app, appearance::Sel::Item(1));
    assert_eq!(app.session.appearance_item(), Some(1));
    let mut panel = appearance_panel(app, &[]);
    panel.secondary_click("Rectangle");
    assert_eq!(panel.app().session.appearance_item(), None, "the object row is picked");
    assert!(panel.shows("Clear Appearance") && panel.shows("Reduce to Basic Appearance"));
    // The menu's item, not the footer's button of the same name.
    panel.click_menu_item("Clear Appearance");
    let cleared = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 1.0);
    assert_eq!(first_selected(&panel.app()).unwrap().appearance, cleared);
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_ne!(first_selected(&panel.app()).unwrap().appearance, cleared, "one undo step");
}

/// Reduce to Basic Appearance is on when any selected object has more than a basic appearance, and reduces each of
/// them.
#[test]
fn reduce_to_basic_enabled_when_any_selected_subject_needs_reduction() {
    let mut app = app_with_rect();
    let basic = first_selected(&app).unwrap().id.0;
    let extra = run(&mut app, "shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut app, "appearance.addStroke", json!({}));
    run(&mut app, "select.set", json!({"ids": [basic]}));
    let mut panel = appearance_panel(app, &[]);
    panel.secondary_click("Rectangle");
    assert!(panel.shows("Reduce to Basic Appearance") && !panel.enabled("Reduce to Basic Appearance"), "off with only a basic object");
    panel.key(egui::Key::Escape, egui::Modifiers::NONE);
    run(&mut panel.app(), "select.set", json!({"ids": [basic, extra]}));
    panel.secondary_click("Rectangle");
    assert!(panel.enabled("Reduce to Basic Appearance"), "on with an object to reduce");
    panel.click("Reduce to Basic Appearance");
    let app = panel.app();
    let doc = &app.session.active().unwrap().doc;
    for id in [basic, extra] {
        assert!(doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.is_basic(), "{id} is basic");
    }
}

/// With two fills/strokes picked, the menu's Duplicate Items copies both in one undo step.
#[test]
fn right_clicking_multi_selected_items_shows_plural_duplicate_and_delete() {
    let mut panel = appearance_panel(app_with_rect(), &[]);
    click_row(&mut panel, "Stroke:", egui::Modifiers::NONE);
    click_row(&mut panel, "Fill:", egui::Modifiers::COMMAND);
    panel.secondary_click("Stroke:");
    assert!(panel.shows("Duplicate Items") && panel.shows("Delete Items"));
    panel.click("Duplicate Items");
    assert_eq!(stack_items(&panel.app()).len(), 4, "both rows copied");
    run(&mut panel.app(), "edit.undo", json!({}));
    assert_eq!(stack_items(&panel.app()).len(), 2, "one undo step");
}
