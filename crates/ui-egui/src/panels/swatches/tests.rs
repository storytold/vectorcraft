use super::*;
use crate::dialogs::swatch_options::KIND;
use crate::tests::picks::{answer_dialog, cancel_dialog, draw_dialogs, open_dialogs};
use craft::keymap::Mods;
use craft::ui::contract::ReorderMove;
use craft::ui::dialog::DialogId;
use egui::Pos2;
use vectorcraft_color::Color;
use vectorcraft_engine::Session;

mod context_menu;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One frame of the panel at `time` on the app's window context.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0)));
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), screen_rect, ..Default::default() }, |ui| show(app, ui));
    out.textures_delta.clear();
}

/// The app's window context, which its dialog host lives on.
fn context(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("a window context")
}

fn none() -> Mods {
    Mods::default()
}

fn alt() -> Mods {
    Mods { alt: true, ..Default::default() }
}

fn cmd() -> Mods {
    Mods { cmd: true, ..Default::default() }
}

fn cmd_alt() -> Mods {
    Mods { alt: true, cmd: true, ..Default::default() }
}

fn doc(app: &VectorcraftApp) -> Document {
    app.session.doc().unwrap().doc.as_ref().clone()
}

/// The row `name` is: a colour group or a swatch.
fn key_of(app: &VectorcraftApp, name: &str) -> SwatchKey {
    if doc(app).swatch_groups.iter().any(|g| g.name == name) { SwatchKey::Group(name.into()) } else { SwatchKey::Swatch(name.into()) }
}

/// The rows in panel order, as the browser shows every swatch.
fn shown_rows(app: &VectorcraftApp) -> Vec<SwatchKey> {
    shown_keys(&groups(app, Kind::All, ""))
}

/// Feeds the browser event `event` to the panel's handler.
fn feed(app: &mut VectorcraftApp, event: PresetEvent<SwatchKey>) {
    let shown = shown_rows(app);
    on_event(app, &shown, event);
}

/// The panel's rows `names` selected, as the browser selects them.
fn select_rows(app: &mut VectorcraftApp, names: &[&str]) {
    let keys: Vec<SwatchKey> = names.iter().map(|n| key_of(app, n)).collect();
    app.ui.swatches_panel.state.selection = Selection::new(keys.clone(), keys.last().cloned(), keys.first().cloned());
}

/// A click on swatch `name` with `mods` held, as the panel reports it (`used`, for the background with Alt).
fn clicked(app: &mut VectorcraftApp, name: &str, mods: Mods) {
    super::apply(app, name, mods.alt);
}

/// A double click on swatch `name`.
fn activated(app: &mut VectorcraftApp, name: &str) {
    let key = key_of(app, name);
    feed(app, PresetEvent::activate(key, none()));
}

/// `drag` released onto row `name` with `mods` held.
fn dropped(app: &mut VectorcraftApp, drag: PanelDrag, name: &str, mods: Mods) {
    let (shown, at) = (shown_rows(app), DropAt::Onto(key_of(app, name)));
    dnd::on_dropped(app, &shown, &at, &drag, mods);
}

/// `drag` released on the list below its last row with `mods` held.
fn dropped_below(app: &mut VectorcraftApp, drag: PanelDrag, mods: Mods) {
    let shown = shown_rows(app);
    let at = DropAt::gap(None, shown.len());
    dnd::on_dropped(app, &shown, &at, &drag, mods);
}

/// The drag the row `name` starts, carrying rows `names`.
fn rows_drag(app: &VectorcraftApp, name: &str, names: &[&str]) -> PanelDrag {
    dnd::drag_of(&doc(app), &key_of(app, name), names.iter().map(|n| n.to_string()).collect())
}

/// Art `ids` dragged off the canvas.
fn art_drag(app: &VectorcraftApp, ids: Vec<vectorcraft_doc::NodeId>) -> PanelDrag {
    PanelDrag::Art { document: app.session.active().map(|st| st.uid).unwrap_or_default(), ids }
}

/// The browser's reorder committing `names` before row `before` (a colour group: into it).
fn reorder_before(app: &mut VectorcraftApp, names: &[&str], before: &str) {
    let shown = shown_rows(app);
    let to = shown.iter().filter(|k| !names.contains(&name_of(k))).position(|k| name_of(k) == before).expect("the row is shown");
    let keys: Vec<SwatchKey> = names.iter().map(|n| key_of(app, n)).collect();
    let mv = ReorderMove::new(keys[0].clone(), keys, to, false);
    dnd::on_reorder(app, &shown, &mv);
}

/// The open dialog with `key`, the top one when several are.
fn dialog_id(app: &VectorcraftApp, key: &str) -> Option<DialogId> {
    open_dialogs(app).into_iter().rev().find(|d| d.key.as_deref() == Some(key)).map(|d| d.id)
}

fn dialog_open(app: &VectorcraftApp, key: &str) -> bool {
    dialog_id(app, key).is_some()
}

fn dialog_field(app: &VectorcraftApp, key: &str, field: &str) -> Value {
    open_dialogs(app).into_iter().rev().find(|d| d.key.as_deref() == Some(key)).and_then(|d| d.fields?.get(field).cloned()).unwrap_or(Value::Null)
}

fn set_field(app: &mut VectorcraftApp, key: &str, field: &str, value: Value) {
    let id = dialog_id(app, key).expect("the dialog is open");
    answer_dialog(app, id, &[(field, value)], false);
}

fn confirm(app: &mut VectorcraftApp, key: &str) {
    let id = dialog_id(app, key).expect("the dialog is open");
    answer_dialog(app, id, &[], true);
}

fn cancel(app: &mut VectorcraftApp, key: &str) {
    let id = dialog_id(app, key).expect("the dialog is open");
    cancel_dialog(app, id);
}

/// One frame of the hosted dialogs, which preview live edits.
fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    draw_dialogs(app, ctx);
}

#[test]
fn double_click_opens_swatch_options_and_ok_edits_linked_art() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
    let ctx = context(&app);
    activated(&mut app, "Red");
    assert_eq!((dialog_field(&app, KIND, "name"), dialog_field(&app, KIND, "mode")), (json!("Red"), json!("rgb")));
    // The dialog previews live on the canvas; OK keeps the edit as one undo step.
    let undo = undo_steps(&app);
    dialog_frame(&mut app, &ctx);
    assert!(app.session.in_interaction(), "Preview runs as an interaction");
    set_field(&mut app, KIND, "color", json!("#00ff00"));
    set_field(&mut app, KIND, "name", json!("Signal"));
    confirm(&mut app, KIND);
    assert!(!dialog_open(&app, KIND) && !app.session.in_interaction());
    let d = doc(&app);
    assert_eq!(d.swatch("Signal").and_then(|w| w.paint.color()).map(|c| c.to_hex()), Some("#00ff00".into()));
    let fill = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::Solid { color: Color::from_hex("#00ff00").unwrap(), swatch: Some("Signal".into()), tint: 1.0 });
    assert_eq!(undo_steps(&app), undo + 1);
}

#[test]
fn cancel_rolls_the_preview_back() {
    let mut app = app();
    let ctx = context(&app);
    let before = doc(&app);
    app.run("ui.swatchOptions", json!({"name": "Bright Blue"})).unwrap();
    set_field(&mut app, KIND, "color", json!("#000000"));
    dialog_frame(&mut app, &ctx);
    assert_ne!(doc(&app), before, "previewed");
    cancel(&mut app, KIND);
    assert!(!dialog_open(&app, KIND) && !app.session.in_interaction());
    assert_eq!(doc(&app), before);
}

#[test]
fn switching_documents_clears_the_selection() {
    let mut app = app();
    let ctx = context(&app);
    frame(&mut app, &ctx, 0.0);
    select_rows(&mut app, &["White"]);
    frame(&mut app, &ctx, 0.5);
    assert_eq!(selected(&app), ["White"]);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    frame(&mut app, &ctx, 1.0);
    frame(&mut app, &ctx, 1.5);
    assert!(selected(&app).is_empty());
}

/// A color swatch clicked in the panel becomes the Color Guide's base color (#3137).
#[test]
fn a_clicked_color_swatch_becomes_the_color_guides_base() {
    let mut app = app();
    app.ui.color_guide_base = Some(Color::WHITE);
    clicked(&mut app, "Red", none());
    assert_eq!(crate::panels::color_guide::base_color(&app).to_hex(), "#ed1c24");
}

/// A clicked spot color swatch, or a tint swatch of one, becomes the Color Guide's base linked to
/// that spot color at the swatch's tint; a process color swatch clears the link.
#[test]
fn a_clicked_spot_color_swatch_becomes_a_linked_color_guide_base() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": "#0066cc", "spot": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Ink", "tint": 40})).unwrap();
    let spot = |app: &VectorcraftApp| app.ui.color_guide_spot.as_ref().map(|s| (s.swatch.clone(), (s.tint * 100.0).round()));
    clicked(&mut app, "Ink", none());
    assert_eq!(spot(&app), Some(("Ink".to_string(), 100.0)));
    clicked(&mut app, "Ink 40%", none());
    assert_eq!(spot(&app), Some(("Ink".to_string(), 40.0)), "a tint swatch: its tint of the spot color");
    clicked(&mut app, "Red", none());
    assert_eq!(spot(&app), None, "a process color");
}

/// The hex colors of the stops of the selection's gradient fill, and its stroke's color.
fn stops_and_stroke(app: &VectorcraftApp) -> (Vec<String>, String) {
    let (fill, stroke) = crate::panels::current_paints(app);
    let Paint::Gradient(g) = fill else { panic!("the fill is still a gradient: {fill:?}") };
    (g.gradient.stops.iter().map(|s| s.color.to_hex()).collect(), stroke.color().map(|c| c.to_hex()).unwrap_or_default())
}

#[test]
fn alt_click_recolors_the_selected_gradient_stop() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let undo = undo_steps(&app);
    clicked(&mut app, "Red", alt());
    assert_eq!(stops_and_stroke(&app), (vec!["#ffffff".into(), "#ed1c24".into()], "#000000".into()), "the stop, not the stroke");
    assert_eq!(undo_steps(&app), undo + 1, "one undo step");
    assert_eq!(app.session.selected_stop(), Some(1), "the stop stays selected");
    assert!(app.session.fill_active, "the Fill proxy stays in front");
    // With no stop selected, Alt-click paints the inactive proxy.
    app.run("gradient.selectStop", json!({"index": null})).unwrap();
    clicked(&mut app, "Orange", alt());
    let orange = doc(&app).swatch("Orange").and_then(|w| w.paint.color()).unwrap().to_hex();
    assert_eq!(stops_and_stroke(&app), (vec!["#ffffff".into(), "#ed1c24".into()], orange));
}

#[test]
fn alt_click_links_the_selected_stop_to_a_global_swatch() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    clicked(&mut app, "Red", alt());
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("a gradient fill") };
    assert_eq!(g.gradient.stops[0].swatch.as_deref(), Some("Red"), "linked as a drop on the ramp links it");
    app.run("swatch.edit", json!({"name": "Red", "color": "#00ff00"})).unwrap();
    assert_eq!(stops_and_stroke(&app).0, ["#00ff00", "#000000"], "editing the swatch recolors the stop");
}

#[test]
fn alt_click_recolors_the_selected_freeform_point() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"color": "#000000", "focus": false})).unwrap();
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    app.run("paint.freeform.selectPoint", json!({"index": 1})).unwrap();
    clicked(&mut app, "Red", alt());
    let (fill, stroke) = crate::panels::current_paints(&app);
    let Paint::Gradient(g) = fill else { panic!("still a gradient: {fill:?}") };
    assert_eq!(g.freeform.expect("placed points").points[1].color.to_hex(), "#ed1c24");
    assert_eq!(stroke.color().map(|c| c.to_hex()).as_deref(), Some("#000000"), "the stroke keeps its color");
    assert_eq!(app.session.selected_freeform_point(), Some(1), "the point stays selected");
}

#[test]
fn delete_asks_then_removes_the_selection_and_unlinks_art() {
    let mut app = app();
    let confirm_key = crate::dialogs::confirm::KIND;
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
    delete(&mut app, vec!["Red".into(), "Brights".into()], false);
    assert!(dialog_open(&app, confirm_key), "delete asks first");
    assert!(doc(&app).swatch("Red").is_some(), "nothing is deleted before OK");
    confirm(&mut app, confirm_key);
    assert!(!dialog_open(&app, confirm_key));
    let d = doc(&app);
    assert!(d.swatch("Red").is_none() && d.swatch("Bright Red").is_none() && d.swatch_groups.iter().all(|g| g.name != "Brights"));
    let fill = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::solid(Color::from_hex("#ed1c24").unwrap()), "the art keeps its colour, unlinked");
    // Cancel keeps the swatch; Alt-click (now) deletes without asking.
    delete(&mut app, vec!["Orange".into()], false);
    assert!(dialog_open(&app, confirm_key));
    cancel(&mut app, confirm_key);
    assert!(doc(&app).swatch("Orange").is_some());
    delete(&mut app, vec!["Orange".into()], true);
    assert!(!dialog_open(&app, confirm_key) && doc(&app).swatch("Orange").is_none());
    assert_eq!(deletable(&app, &["[None]".into(), REGISTRATION.into(), "Grays".into()]), ["Grays"]);
}

/// Only None and Registration are reserved: a user swatch or color group whose name starts with
/// `[` can be deleted, the swatch can be edited, and names the document does not hold are left out.
#[test]
fn bracketed_user_swatches_and_groups_can_be_deleted_and_edited() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "[Mine]", "color": "#336699"})).unwrap();
    app.run("swatch.newGroup", json!({"name": "[Team]", "colors": ["#ff0000"]})).unwrap();
    let names = |v: &[&str]| v.iter().map(|n| n.to_string()).collect::<Vec<String>>();
    assert_eq!(deletable(&app, &names(&["[None]", REGISTRATION, "[Mine]", "[Team]", "Gone"])), ["[Mine]", "[Team]"]);
    assert_eq!(actions::editable(&app, &names(&["[Mine]"])).as_deref(), Some("[Mine]"));
    for name in ["[None]", REGISTRATION, "[Team]", "Gone"] {
        assert_eq!(actions::editable(&app, &names(&[name])), None, "{name}");
    }
    let doomed = deletable(&app, &names(&["[Mine]"]));
    delete(&mut app, doomed, false);
    confirm(&mut app, crate::dialogs::confirm::KIND);
    assert!(doc(&app).swatch("[Mine]").is_none());
}

/// One question covers every swatch of a delete of one or several, and OK deletes them all in one undo step.
#[test]
fn deleting_several_swatches_asks_once_for_all_of_them() {
    let mut app = app();
    let key = crate::dialogs::confirm::KIND;
    for count in [1usize, 2, 5, 21] {
        let names: Vec<String> = (1..=count).map(|n| format!("Swatch {count}.{n}")).collect();
        for (n, name) in names.iter().enumerate() {
            app.run("swatch.new", json!({"name": name, "color": format!("#{n:02x}3366")})).unwrap();
        }
        let undo = undo_steps(&app);
        delete(&mut app, names.clone(), false);
        assert_eq!(open_dialogs(&app).iter().filter(|d| d.key.as_deref() == Some(key)).count(), 1, "{count}");
        confirm(&mut app, key);
        let d = doc(&app);
        assert!(names.iter().all(|n| d.swatch(n).is_none()), "{count}");
        assert_eq!(undo_steps(&app), undo + 1, "{count}");
    }
}

#[test]
fn new_swatch_dialog_prefills_the_active_colour_and_saves_into_the_selected_group() {
    let mut app = app();
    let ctx = context(&app);
    let key = crate::dialogs::new_swatch::KIND;
    app.run("paint.setFill", json!({"color": "#ff8000"})).unwrap();
    new_swatch(&mut app, &["Bright Red".into()], cmd());
    // The colour group the swatch goes into is the dialog's own: OK below puts it into Brights.
    assert_eq!(dialog_field(&app, key, "name"), json!("R=255 G=128 B=0"));
    assert_eq!(
        (dialog_field(&app, key, "type"), dialog_field(&app, key, "global")),
        (json!("spot"), json!(true)),
        "Cmd/Ctrl-click makes a spot colour"
    );
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    let d = doc(&app);
    let w = d.swatch("R=255 G=128 B=0").expect("created");
    assert!(w.spot && d.swatch_group_of(&w.name).is_some_and(|g| d.swatch_groups[g].name == "Brights"));
    // Alt-click saves at once; a gradient takes only a name and stays out of colour groups.
    app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
    new_swatch(&mut app, &["Brights".into()], alt());
    assert!(!dialog_open(&app, key) && doc(&app).swatch("New Gradient Swatch 1").is_some());
    new_swatch(&mut app, &[], none());
    assert_eq!(dialog_field(&app, key, "name"), json!("New Gradient Swatch 2"));
    // A gradient has only its name to set: the dialog keeps its paint for OK.
    assert_eq!((dialog_field(&app, key, "type"), dialog_field(&app, key, "color")), (Value::Null, Value::Null));
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    assert!(doc(&app).swatch("New Gradient Swatch 2").is_some_and(|w| matches!(w.paint, Paint::Gradient(_))));
}

/// In a CMYK document a swatch made from an RGB fill keeps RGB, as the Color Guide's copies
/// do: Alt-click on New Swatch, the New Swatch dialog, the Fill proxy dropped on the panel and
/// Alt-dropped on a color swatch, and the same for a gradient's stops.
#[test]
fn swatches_made_from_an_rgb_fill_stay_rgb_in_a_cmyk_document() {
    let mut app = app();
    let ctx = context(&app);
    let key = crate::dialogs::new_swatch::KIND;
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let newest = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatches.last().map(|w| w.paint.clone()).expect("a swatch");
    let stop_colors = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.color).collect::<Vec<_>>(),
        other => panic!("a gradient: {other:?}"),
    };
    let rgb = Color::from_hex("#2a6fb0").unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#2a6fb0", "keepModel": true})).unwrap();
    new_swatch(&mut app, &[], alt());
    assert_eq!(newest(&app), Paint::solid(rgb), "Alt-click on New Swatch");
    new_swatch(&mut app, &[], none());
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    assert_eq!(newest(&app), Paint::solid(rgb), "the New Swatch dialog");
    dropped(&mut app, PanelDrag::paint(Paint::solid(rgb)), "Red", none());
    assert_eq!(newest(&app), Paint::solid(rgb), "the proxy dropped on the panel: {}", app.ui.status);
    dropped(&mut app, PanelDrag::paint(Paint::solid(rgb)), "Red", alt());
    assert_eq!(doc(&app).swatch("Red").map(|w| w.paint.color()), Some(Some(rgb)), "Alt-dropped on Red");
    // A gradient's RGB stops.
    let stops = json!([{"offset": 0, "color": "#ff8000"}, {"offset": 1, "color": "#2a6fb0"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}, "keepModel": true})).unwrap();
    let grad = crate::panels::current_paints(&app).0;
    let rgb_stops = vec![Color::from_hex("#ff8000").unwrap(), rgb];
    assert_eq!(stop_colors(&grad), rgb_stops);
    new_swatch(&mut app, &[], alt());
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "Alt-click on New Swatch");
    new_swatch(&mut app, &[], none());
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "the New Swatch dialog");
    dropped(&mut app, PanelDrag::paint(grad.clone()), "Red", none());
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "the gradient dropped on the panel");
    dropped(&mut app, PanelDrag::paint(grad), "Ocean", alt());
    let ocean = doc(&app).swatch("Ocean").map(|w| w.paint.clone()).expect("Ocean");
    assert_eq!(stop_colors(&ocean), rgb_stops, "Alt-dropped on Ocean");
}

#[test]
fn new_color_group_dialog_makes_a_group_from_swatches_or_artwork() {
    let mut app = app();
    let ctx = context(&app);
    let key = crate::dialogs::new_color_group::KIND;
    app.run("ui.newColorGroup", json!({"swatches": ["Red", "Sunset"]})).unwrap();
    assert_eq!((dialog_field(&app, key, "name"), dialog_field(&app, key, "from")), (json!("Color Group"), json!("swatches")));
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    let d = doc(&app);
    let g = d.swatch_groups.iter().find(|g| g.name == "Color Group").expect("group made");
    assert_eq!(g.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Red"], "gradients stay out");
    // With art selected (and no swatches) it starts from the artwork: its colours become global.
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "color": "#123456"})).unwrap();
    app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("ui.newColorGroup", json!({})).unwrap();
    assert_eq!(
        (dialog_field(&app, key, "name"), dialog_field(&app, key, "from"), dialog_field(&app, key, "toGlobal")),
        (json!("Color Group 2"), json!("artwork"), json!(true))
    );
    dialog_frame(&mut app, &ctx);
    confirm(&mut app, key);
    let d = doc(&app);
    let g = d.swatch_groups.iter().find(|g| g.name == "Color Group 2").expect("group made");
    assert_eq!(g.swatches.iter().map(|w| (w.name.as_str(), w.global)).collect::<Vec<_>>(), [("R=18 G=52 B=86", true)]);
    let fill = d.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::Solid { color: Color::from_hex("#123456").unwrap(), swatch: Some("R=18 G=52 B=86".into()), tint: 1.0 });
}

fn names_of(app: &VectorcraftApp, group: Option<&str>) -> Vec<String> {
    let d = &app.session.active().unwrap().doc;
    let list = match group {
        Some(g) => &d.swatch_groups.iter().find(|x| x.name == g).unwrap().swatches,
        None => &*d.swatches,
    };
    list.iter().map(|w| w.name.clone()).collect()
}

/// A double click on a pattern swatch opens pattern editing and paints nothing: not the selected
/// art, and not the tile art the editing selects (#7095).
#[test]
fn double_clicking_a_pattern_edits_it_without_painting_its_tile() {
    let mut app = app();
    let tile = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [tile], "name": "Dots", "width": 20, "height": 20})).unwrap();
    app.run("object.pattern.done", json!({})).unwrap();
    let art = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [art], "color": "#2050a0"})).unwrap();
    app.run("select.set", json!({"ids": [art]})).unwrap();
    activated(&mut app, "Dots");
    let d = doc(&app);
    let pe = d.pattern_edit.as_ref().expect("the double-click opens pattern editing");
    assert_eq!(pe.pattern, "Dots");
    let fill = d.node(vectorcraft_doc::NodeId(art)).unwrap().appearance.fill_paint();
    assert!(!matches!(fill, Paint::Pattern { .. }), "the art is not painted: {}", app.ui.status);
    for n in d.node(pe.layer).unwrap().children().unwrap() {
        assert!(!matches!(n.appearance.fill_paint(), Paint::Pattern { .. }), "the tile keeps its own fill: {}", app.ui.status);
    }
}

#[test]
fn reordering_moves_into_groups_as_one_undo_step_each() {
    let mut app = app();
    let undo = undo_steps(&app);
    // Before Red.
    reorder_before(&mut app, &["Amber"], "Red");
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Amber");
    assert_eq!(undo_steps(&app), undo + 1);
    // Onto a folder: into that group, at its end.
    reorder_before(&mut app, &["Amber"], "Grays");
    assert_eq!(names_of(&app, Some("Grays")).last().map(String::as_str), Some("Amber"));
    assert_eq!(undo_steps(&app), undo + 2);
    // Several rows move together, in panel order; a gradient can't join a group.
    reorder_before(&mut app, &["Orange", "Lime", "Sunset"], "Bright Red");
    assert_eq!(names_of(&app, Some("Brights"))[..3], ["Orange", "Lime", "Bright Red"]);
    assert!(names_of(&app, None).contains(&"Sunset".to_string()));
}

/// Swatches and colour groups released on the footer buttons: New Swatch duplicates the swatches among
/// them, Delete Swatch asks first and deletes them in one undo step.
#[test]
fn drops_on_the_buttons_and_the_panel_run_their_commands() {
    let mut app = app();
    let key = crate::dialogs::confirm::KIND;
    let undo = undo_steps(&app);
    // Delete: asks, Cancel keeps, OK deletes.
    let drag = rows_drag(&app, "Orange", &["Orange"]);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::Delete, &drag);
    assert!(dialog_open(&app, key), "Delete asks first");
    cancel(&mut app, key);
    assert!(doc(&app).swatch("Orange").is_some() && undo_steps(&app) == undo, "Cancel keeps it");
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::Delete, &drag);
    confirm(&mut app, key);
    assert!(doc(&app).swatch("Orange").is_none());
    assert_eq!(undo_steps(&app), undo + 1, "one undo step");
    app.run("edit.undo", json!({})).unwrap();
    // The dragged selection goes together; None never does.
    let drag = rows_drag(&app, "Lime", &["[None]", "Red", "Lime"]);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::Delete, &drag);
    confirm(&mut app, key);
    let d = doc(&app);
    assert!(d.swatch("Red").is_none() && d.swatch("Lime").is_none() && d.swatch("[None]").is_some() && d.swatch("Orange").is_some());
    // None and Registration alone: nothing to delete.
    let before = doc(&app);
    for name in ["[None]", REGISTRATION] {
        let drag = rows_drag(&app, name, &[name]);
        dnd::on_footer_drop(&mut app, dnd::FooterTarget::Delete, &drag);
        assert!(!dialog_open(&app, key) && doc(&app) == before, "{name}");
    }
    // New Swatch: copies go after their swatches, as one undo step; reserved rows duplicate nothing.
    let undo = undo_steps(&app);
    let drag = rows_drag(&app, "Orange", &["Orange"]);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::New, &drag);
    let n = names_of(&app, None);
    assert_eq!(n.get(n.iter().position(|x| x == "Orange").unwrap() + 1).map(String::as_str), Some("Orange copy"));
    assert_eq!(undo_steps(&app), undo + 1);
    let kept = doc(&app);
    for name in ["[None]", REGISTRATION] {
        let drag = rows_drag(&app, name, &[name]);
        dnd::on_footer_drop(&mut app, dnd::FooterTarget::New, &drag);
        assert_eq!(doc(&app), kept, "{name}");
    }
}

/// A selected swatch dragged onto New Swatch or Delete Swatch carries the selected swatches that
/// the find field hides, which the browser leaves out of the drag, as a click on Delete Swatch
/// takes them too. The copies are selected in panel order. A grabbed swatch outside the selection
/// carries only itself.
#[test]
fn footer_drops_carry_selected_swatches_that_find_hides() {
    let mut app = app();
    let key = crate::dialogs::confirm::KIND;
    app.ui.swatches_panel.show_find = true;
    app.ui.swatches_panel.state.browser.filter = "Orange".into();
    let order = names_of(&app, None);
    let at = |n: &str| order.iter().position(|x| x == n).unwrap_or_else(|| panic!("no swatch {n}"));
    let mut copies = vec![("Lime", "Lime copy"), ("Orange", "Orange copy")];
    copies.sort_by_key(|(n, _)| at(n));
    let copies: Vec<String> = copies.into_iter().map(|(_, c)| c.to_string()).collect();
    // With Find set to Orange, the browser's drag holds only the selected row it shows.
    let drag = rows_drag(&app, "Orange", &["Orange"]);
    select_rows(&mut app, &["Lime", "Orange"]);
    let undo = undo_steps(&app);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::New, &drag);
    let d = doc(&app);
    assert!(d.swatch("Orange copy").is_some() && d.swatch("Lime copy").is_some(), "the hidden selected swatch is duplicated too");
    assert_eq!(selected(&app), copies, "both copies are selected, in panel order");
    assert_eq!(undo_steps(&app), undo + 1, "one undo step for both");
    select_rows(&mut app, &["Lime", "Orange"]);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::Delete, &drag);
    assert!(dialog_open(&app, key), "Delete asks first");
    confirm(&mut app, key);
    let d = doc(&app);
    assert!(d.swatch("Orange").is_none() && d.swatch("Lime").is_none(), "the hidden selected swatch is deleted too");
    // A grabbed swatch outside the selection carries only itself.
    let drag = rows_drag(&app, "Orange copy", &["Orange copy"]);
    select_rows(&mut app, &["Lime copy"]);
    dnd::on_footer_drop(&mut app, dnd::FooterTarget::New, &drag);
    let d = doc(&app);
    assert!(d.swatch("Orange copy copy").is_some() && d.swatch("Lime copy copy").is_none(), "only the grabbed swatch is duplicated");
}

#[test]
fn art_dropped_on_the_panel_becomes_a_pattern_swatch() {
    let mut app = app();
    let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
    let before = doc(&app);
    let drag = art_drag(&app, vec![vectorcraft_doc::NodeId(id)]);
    dropped_below(&mut app, drag, none());
    let patterns = app.run("pattern.list", json!({})).unwrap();
    assert_eq!(patterns["patterns"].as_array().map(Vec::len), Some(1), "{patterns}");
    assert_eq!(patterns["editing"], Value::Null, "no pattern editing mode");
    assert_eq!(doc(&app).node(vectorcraft_doc::NodeId(id)), before.node(vectorcraft_doc::NodeId(id)), "the art stays as it was");
}

/// Art released with Alt held over a pattern swatch becomes that pattern's tile art in one undo
/// step: no pattern or swatch is added and the art stays. With Alt over a color swatch, or without
/// Alt over the pattern swatch, the art becomes a new pattern swatch.
#[test]
fn art_alt_dropped_on_a_pattern_swatch_replaces_its_tile_art() {
    let mut app = app();
    let square = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    app.run("object.pattern.make", json!({"ids": [square], "name": "Dots", "edit": false})).unwrap();
    let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
    let star = vectorcraft_doc::NodeId(id);
    let count = |app: &mut VectorcraftApp| app.run("pattern.list", json!({})).unwrap()["patterns"].as_array().map(Vec::len);
    let tile_art = |app: &VectorcraftApp| app.session.doc().unwrap().doc.pattern("Dots").map(|p| p.art.clone());
    let (before, undo, star_before) = (tile_art(&app), undo_steps(&app), doc(&app).node(star).cloned());
    let drag = art_drag(&app, vec![star]);
    dropped(&mut app, drag.clone(), "Dots", alt());
    let art = tile_art(&app).expect("Dots stays");
    assert_ne!(Some(&art), before.as_ref(), "{}", app.ui.status);
    assert_eq!(art.len(), 1);
    assert_eq!(Some(&art[0].kind), star_before.as_ref().map(|n| &n.kind), "the star is the tile art");
    assert_eq!(count(&mut app), Some(1), "no new pattern");
    assert_eq!(undo_steps(&app), undo + 1, "one undo step");
    assert_eq!(doc(&app).node(star).cloned(), star_before, "the star stays as it was");
    dropped(&mut app, drag.clone(), "Bright Red", alt());
    assert_eq!(count(&mut app), Some(2), "Alt over a color swatch makes a pattern");
    dropped(&mut app, drag, "Dots", none());
    assert_eq!(count(&mut app), Some(3), "a plain drop on a pattern swatch makes a pattern");
    assert_eq!(tile_art(&app), Some(art), "Dots keeps its tile art");
}

/// A paint from a Fill/Stroke proxy, a Recent Colors chip or the Color panel dropped on a grouped
/// swatch becomes a swatch in that group, in its own colour model, as one undo step that paints no art.
#[test]
fn a_paint_dropped_on_the_panel_becomes_a_swatch() {
    let mut app = app();
    let (id, cmyk) = art_with_two_recent_colors(&mut app);
    let (art, undo) = (fill_of(&app, id), undo_steps(&app));
    let rgb = Color::from_hex("#123456").unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(rgb)), "Bright Red", none());
    assert_eq!(names_of(&app, Some("Brights")).last().map(String::as_str), Some("R=18 G=52 B=86"), "into that group");
    // A recent chip's CMYK colour, onto an ungrouped swatch; the same colour again gets another name.
    let count = doc(&app).swatches_iter().count();
    dropped(&mut app, PanelDrag::paint(Paint::solid(cmyk)), "Red", none());
    let first = newest_swatch(&app);
    assert_eq!(first.paint, Paint::solid(cmyk), "the chip's color, still CMYK");
    dropped(&mut app, PanelDrag::paint(Paint::solid(cmyk)), "Red", none());
    assert_ne!(newest_swatch(&app).name, first.name, "another name");
    assert_eq!(doc(&app).swatches_iter().count(), count + 2);
    assert_eq!(undo_steps(&app), undo + 3, "one undo step each");
    assert_eq!(fill_of(&app, id), art, "the drop paints no art");
}

/// A color Alt-dropped on Registration, which can't be edited, becomes a new swatch and leaves
/// Registration as it is.
#[test]
fn a_color_alt_dropped_on_registration_becomes_a_new_swatch() {
    let mut app = app();
    let (_, cmyk) = art_with_two_recent_colors(&mut app);
    let registration = doc(&app).swatch(REGISTRATION).map(|w| w.paint.clone());
    let count = doc(&app).swatches_iter().count();
    dropped(&mut app, PanelDrag::paint(Paint::solid(cmyk)), REGISTRATION, alt());
    let d = doc(&app);
    assert_eq!(d.swatches_iter().count(), count + 1, "{}", app.ui.status);
    assert_eq!(d.swatches.last().map(|w| w.paint.clone()), Some(Paint::solid(cmyk)));
    assert_eq!(d.swatch(REGISTRATION).map(|w| w.paint.clone()), registration, "Registration is unchanged");
}

/// A rectangle filled with a CMYK color and then an RGB one, so Recent Colors holds the RGB color
/// first and the CMYK one second; → its id and the CMYK color.
fn art_with_two_recent_colors(app: &mut VectorcraftApp) -> (vectorcraft_doc::NodeId, Color) {
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "color": {"c": 10, "m": 60, "y": 20, "k": 5}, "keepModel": true})).unwrap();
    app.run("paint.setFill", json!({"ids": [id], "color": "#2050a0"})).unwrap();
    let cmyk = app.session.recent_colors[1];
    assert!(matches!(cmyk, Color::Cmyk { .. }), "the second recent color is CMYK: {cmyk:?}");
    (vectorcraft_doc::NodeId(id), cmyk)
}

/// The fill of art `id`.
fn fill_of(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> Option<Paint> {
    app.session.doc().unwrap().doc.node(id).map(|n| n.appearance.fill_paint())
}

/// The newest ungrouped swatch.
fn newest_swatch(app: &VectorcraftApp) -> Swatch {
    app.session.doc().unwrap().doc.swatches.last().cloned().expect("a swatch")
}

/// How many swatches are spot colors.
fn spot_count(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.swatches_iter().filter(|w| w.spot).count()
}

fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// A color Cmd/Ctrl-dropped on the panel becomes a global spot color swatch in the color's own
/// model, in the group it is dropped into, as one undo step that paints no art; without the
/// modifier it stays a process color.
#[test]
fn a_color_cmd_dropped_on_the_panel_becomes_a_spot_swatch() {
    let mut app = app();
    let (id, cmyk) = art_with_two_recent_colors(&mut app);
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let (art, undo) = (fill_of(&app, id), undo_steps(&app));
    let rgb = Color::from_hex("#123456").unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(rgb)), "Bright Red", cmd());
    let d = doc(&app);
    let made = names_of(&app, Some("Brights")).last().and_then(|n| d.swatch(n).cloned()).expect("a swatch");
    assert_eq!((made.paint, made.spot, made.global), (Paint::solid(rgb), true, true), "{}", app.ui.status);
    assert_eq!(undo_steps(&app), undo + 1, "one undo step");
    assert_eq!(fill_of(&app, id), art, "the drop paints no art");
    // A CMYK chip in an RGB document: a CMYK spot color.
    dropped(&mut app, PanelDrag::paint(Paint::solid(cmyk)), "Red", cmd());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot), (Paint::solid(cmyk), true), "{}", app.ui.status);
    // Without the modifier: a process color.
    let spots = spot_count(&app);
    dropped(&mut app, PanelDrag::paint(Paint::solid(Color::from_hex("#654321").unwrap())), "Red", none());
    let newest = newest_swatch(&app);
    assert_eq!((newest.spot, newest.global, spot_count(&app)), (false, false, spots));
    // An RGB color in a CMYK document stays RGB; the document stays CMYK.
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(rgb)), "Red", cmd());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot), (Paint::solid(rgb), true), "{}", app.ui.status);
    assert_eq!(doc(&app).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    let count = doc(&app).swatches_iter().count();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(doc(&app).swatches_iter().count(), count - 1);
}

/// A color Cmd/Ctrl-dropped away from any row becomes an ungrouped spot color swatch.
#[test]
fn a_color_cmd_dropped_on_empty_list_space_becomes_a_spot_swatch() {
    let mut app = app();
    let count = doc(&app).swatches_iter().count();
    let rgb = Color::from_hex("#123456").unwrap();
    dropped_below(&mut app, PanelDrag::paint(Paint::solid(rgb)), cmd());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot, newest.global), (Paint::solid(rgb), true, true), "{}", app.ui.status);
    assert_eq!(doc(&app).swatches_iter().count(), count + 1);
}

/// Cmd/Ctrl makes a spot color only of a solid color dragged from elsewhere: a gradient and
/// Registration make no spot color, and swatches dragged within the panel move.
#[test]
fn a_cmd_drop_of_other_paints_or_rows_makes_no_spot_color() {
    let mut app = app();
    let spots = spot_count(&app);
    let count = doc(&app).swatches_iter().count();
    let stops =
        vec![vectorcraft_color::GradientStop::new(0.0, Color::from_hex("#ff0000").unwrap()), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)];
    let grad = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::new(GradientKind::Radial, stops))));
    dropped(&mut app, PanelDrag::paint(grad), "Red", cmd());
    assert!(matches!(newest_swatch(&app).paint, Paint::Gradient(_)), "a gradient swatch: {}", app.ui.status);
    assert_eq!((doc(&app).swatches_iter().count(), spot_count(&app)), (count + 1, spots));
    dropped(&mut app, PanelDrag::paint(Paint::registration()), "Red", cmd());
    assert_eq!(spot_count(&app), spots, "Registration makes no spot color");
    // A swatch dragged with Cmd/Ctrl within the panel moves.
    let count = doc(&app).swatches_iter().count();
    let drag = rows_drag(&app, "Lime", &["Lime"]);
    dropped(&mut app, drag, "Red", cmd());
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Lime", "{}", app.ui.status);
    assert_eq!((doc(&app).swatches_iter().count(), spot_count(&app)), (count, spots));
}

/// A tint of a global color Cmd/Ctrl-dropped becomes a new spot color of the tinted color, and its
/// base stays as it is. With Alt also held, a color dropped on a solid swatch replaces it; dropped on
/// a tint swatch, which only takes a new name, it becomes a spot color.
#[test]
fn cmd_drops_of_tints_and_with_alt_follow_the_replacement_rule() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    let orange = doc(&app).swatch("Orange").cloned();
    let tint = doc(&app).tint_paint("Orange", 0.4).expect("a tint of Orange");
    let shown = tint.color().expect("a color");
    dropped(&mut app, PanelDrag::paint(tint), "Red", cmd());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot, newest.global), (Paint::solid(shown), true, true), "{}", app.ui.status);
    assert_eq!(doc(&app).swatch("Orange").cloned(), orange, "Orange is unchanged");
    // Cmd/Ctrl and Alt on a solid swatch: it takes the color, and no swatch is added.
    let count = doc(&app).swatches_iter().count();
    let blue = Color::from_hex("#2040c0").unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(blue)), "Red", cmd_alt());
    let red = doc(&app).swatch("Red").cloned().expect("Red");
    assert_eq!((red.paint.color(), red.spot), (Some(blue), false), "{}", app.ui.status);
    assert_eq!(doc(&app).swatches_iter().count(), count);
    // Cmd/Ctrl and Alt on a tint swatch: a new spot color, the tint swatch unchanged.
    let green = Color::from_hex("#20a040").unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(green)), "Orange 40%", cmd_alt());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot), (Paint::solid(green), true), "{}", app.ui.status);
    assert!(doc(&app).swatch("Orange 40%").is_some_and(|w| w.tint_of().is_some()));
}

/// A tint dragged from Orange keeps the color the drag showed when Orange changes before the drop:
/// Cmd/Ctrl makes a spot color of it, Cmd/Ctrl+Alt on a solid swatch gives it that color, and Alt alone
/// sends the tint as it is, so the swatch takes the tint of Orange as it is at the drop.
#[test]
fn a_dropped_tint_follows_the_captured_color_only_with_cmd() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    let tint = doc(&app).tint_paint("Orange", 0.4).expect("a tint of Orange");
    let shown = tint.color().expect("a color");
    app.run("swatch.edit", json!({"name": "Orange", "paint": {"color": "#2040c0"}})).unwrap();
    let now = doc(&app).tint_paint("Orange", 0.4).and_then(|p| p.color()).expect("a tint of Orange");
    assert_ne!(now, shown);
    dropped(&mut app, PanelDrag::paint(tint.clone()), "Red", cmd());
    let newest = newest_swatch(&app);
    assert_eq!((newest.paint, newest.spot), (Paint::solid(shown), true), "{}", app.ui.status);
    dropped(&mut app, PanelDrag::paint(tint.clone()), "Red", cmd_alt());
    assert_eq!(doc(&app).swatch("Red").and_then(|w| w.paint.color()), Some(shown), "{}", app.ui.status);
    dropped(&mut app, PanelDrag::paint(tint), "Red", alt());
    assert_eq!(doc(&app).swatch("Red").and_then(|w| w.paint.color()), Some(now), "{}", app.ui.status);
}

#[test]
fn alt_dropping_a_gradient_on_a_gradient_swatch_replaces_it() {
    let mut app = app();
    let grad = |c: &str| {
        let stops =
            vec![vectorcraft_color::GradientStop::new(0.0, Color::from_hex(c).unwrap()), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)];
        Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::new(GradientKind::Radial, stops))))
    };
    let count = doc(&app).swatches_iter().count();
    dropped(&mut app, PanelDrag::paint(grad("#ff0000")), "Ocean", alt());
    let d = doc(&app);
    let Some(Paint::Gradient(g)) = d.swatch("Ocean").map(|w| &w.paint) else { panic!("Ocean is a gradient") };
    assert_eq!((g.gradient.kind, d.swatches_iter().count()), (GradientKind::Radial, count), "replaced, not added");
    // On a swatch of another kind, it becomes a new swatch.
    dropped(&mut app, PanelDrag::paint(grad("#00ff00")), "Red", alt());
    assert_eq!(doc(&app).swatches_iter().count(), count + 1);
    assert!(doc(&app).swatch("Red").is_some_and(|w| w.paint.color().is_some()));
    // A tint swatch only takes a new name, so a color Alt-dropped on one becomes a new swatch.
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    dropped(&mut app, PanelDrag::paint(Paint::solid(Color::from_hex("#123456").unwrap())), "Orange 40%", alt());
    let d = doc(&app);
    assert_eq!(d.swatches_iter().count(), count + 3, "{}", app.ui.status);
    assert!(d.swatch("Orange 40%").is_some_and(|w| w.tint_of().is_some()));
}

#[test]
fn alt_dragging_a_swatch_onto_another_of_its_kind_replaces_its_color() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Orange"})).unwrap();
    let (before, undo) = (names_of(&app, None), undo_steps(&app));
    let red = doc(&app).swatch("Red").and_then(|w| w.paint.color()).unwrap();
    let drag = rows_drag(&app, "Red", &["Red"]);
    dropped(&mut app, drag, "Orange", alt());
    let d = doc(&app);
    let orange = d.swatch("Orange").expect("Orange keeps its name");
    assert_eq!((orange.paint.color(), orange.global), (Some(red), true), "Orange takes Red's color");
    assert_eq!(names_of(&app, None), before, "Red stays and Orange keeps its place");
    assert_eq!(undo_steps(&app), undo + 1);
    let fill = d.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
    assert_eq!(fill.color(), Some(red), "art linked to Orange takes the new color");
    // A gradient dropped on a color, and a swatch dropped without Alt, move as before.
    let drag = rows_drag(&app, "Sunset", &["Sunset"]);
    dropped(&mut app, drag, "Amber", alt());
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Amber").unwrap() - 1], "Sunset");
    assert!(doc(&app).swatch("Amber").is_some_and(|w| w.paint.color().is_some()));
    let drag = rows_drag(&app, "Lime", &["Lime"]);
    dropped(&mut app, drag, "Red", none());
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Lime");
    assert_ne!(doc(&app).swatch("Red").and_then(|w| w.paint.color()), doc(&app).swatch("Lime").and_then(|w| w.paint.color()));
    // A tint swatch only takes a new name, so a swatch Alt-dropped on one moves.
    let drag = rows_drag(&app, "Lime", &["Lime"]);
    dropped(&mut app, drag, "Orange 40%", alt());
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Orange 40%").unwrap() - 1], "Lime", "{}", app.ui.status);
    assert_eq!(doc(&app).swatch("Orange 40%").and_then(|w| w.tint_of().map(|(base, _)| base.to_string())).as_deref(), Some("Orange"));
}

#[test]
fn menu_targets_follow_the_panel_selection() {
    let app = app();
    let sel: Vec<String> = ["Red", "Sunset", "Grays", "Orange", "Brights"].map(String::from).to_vec();
    assert_eq!(mergeable(&app, &sel), ["Red", "Orange"], "Merge Swatches takes the selected colours, first kept");
    assert_eq!(selected_group(&app, &sel).as_deref(), Some("Grays"));
    assert_eq!(selected_group(&app, &sel[..2]), None);
}

#[test]
fn kind_filter() {
    let solid = Paint::solid(Color::BLACK);
    let grad = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
    assert!(Kind::All.accepts(&grad, false));
    assert!(Kind::Color.accepts(&solid, false));
    assert!(!Kind::Color.accepts(&grad, false));
    assert!(Kind::Gradient.accepts(&grad, true));
    assert!(Kind::Groups.accepts(&solid, true));
    assert!(!Kind::Groups.accepts(&solid, false));
}

#[test]
fn the_active_tint_or_gradient_swatch_is_highlighted_and_a_tint_edits_its_base() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": "#cc0066", "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 40})).unwrap();
    assert_eq!(active_swatch(&app).as_deref(), Some("Ink"), "no tint swatch yet: its base");
    app.run("swatch.new", json!({})).unwrap();
    assert_eq!(active_swatch(&app).as_deref(), Some("Ink 40%"));
    let tint = groups(&app, Kind::Color, "Ink 40").into_iter().flat_map(|g| g.entries).find(|e| e.name == "Ink 40%").unwrap();
    assert_eq!(tint.kind, SwatchKind::Spot, "a tint swatch shows its base's kind");
    app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
    assert_eq!(active_swatch(&app).as_deref(), Some("Sunset"), "the applied gradient swatch");
    app.run("ui.swatchOptions", json!({"name": "Ink 40%"})).unwrap();
    assert_eq!(dialog_field(&app, KIND, "name"), json!("Ink"), "a tint swatch opens its base's options");
}

/// Selecting art scrolls the tiles to its swatch, once, and clears the panel's own selection so
/// that swatch is the one highlighted.
#[test]
fn selecting_art_scrolls_to_its_swatch() {
    let mut app = app();
    let colors: Vec<String> = (0..400).map(|i| format!("#{:02x}{:02x}{:02x}", i % 256, (i / 2) % 256, 128)).collect();
    app.run("swatch.new", json!({ "colors": colors })).unwrap();
    app.run("swatch.new", json!({"name": "Last Ink", "color": "#123456", "spot": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Last Ink"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let ctx = context(&app);
    frame(&mut app, &ctx, 0.0);
    frame(&mut app, &ctx, 0.1);
    select_rows(&mut app, &["Red"]);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    for t in [0.2, 0.3, 0.4] {
        frame(&mut app, &ctx, t);
    }
    assert!(selected(&app).is_empty(), "the art's swatch is highlighted");
    // Once: the focus was given in a pass above, so later asks find nothing to scroll to.
    let mut model = std::mem::take(&mut app.ui.swatches_panel);
    assert_eq!(model.focus(&app, Some("Last Ink")), None);
    app.ui.swatches_panel = model;
}

/// Deselecting art keeps its swatch highlighted (not the White new art gets) until the paints for
/// new art change; selecting other art highlights its swatch.
#[test]
fn deselecting_keeps_the_arts_swatch_highlighted() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink 185", "color": "#e4002b", "spot": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Ink 185"})).unwrap();
    app.run("select.none", json!({})).unwrap();
    // New art's paints back to the defaults, as for art that came in with a file.
    app.run("paint.default", json!({})).unwrap();
    // The swatch the panel lights for the active paint, as a pass asks for it.
    let shown = |app: &mut VectorcraftApp| {
        let mut model = std::mem::take(&mut app.ui.swatches_panel);
        let kept = model.kept(app, active_swatch(app));
        app.ui.swatches_panel = model;
        kept
    };
    assert_ne!(shown(&mut app).as_deref(), Some("Ink 185"), "nothing selected yet: new art's paint");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(shown(&mut app).as_deref(), Some("Ink 185"));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(shown(&mut app).as_deref(), Some("Ink 185"), "kept after deselecting");
    assert_eq!(shown(&mut app).as_deref(), Some("Ink 185"), "and on later frames");
    // A colour picked with nothing selected is new art's: its swatch shows.
    app.run("paint.setFill", json!({"swatch": "Black"})).unwrap();
    assert_eq!(shown(&mut app).as_deref(), Some("Black"));
    // Other art selected (a white one) shows its swatch, and keeps it once deselected.
    let white = app.run("shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [white.clone()], "swatch": "White"})).unwrap();
    app.run("select.set", json!({"ids": [white]})).unwrap();
    assert_eq!(shown(&mut app).as_deref(), Some("White"));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(shown(&mut app).as_deref(), Some("White"));
}

/// A selected image, or text in a colour no swatch has, highlights no swatch (not None); text in
/// a swatch's colour highlights that swatch.
#[test]
fn images_and_unswatched_text_highlight_no_swatch() {
    let mut app = app();
    let mut png = vec![];
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
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
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "a.png", "dataBase64": data, "link": false})).unwrap();
    assert_eq!(active_swatch(&app), None, "an image: no swatch");
    let text = app.run("text.create", json!({"x": 10, "y": 50, "text": "Hi"})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    app.run("paint.setFill", json!({"color": "#3a7b5c"})).unwrap();
    assert_eq!(active_swatch(&app), None, "a colour of its own: no swatch");
    app.run("swatch.new", json!({"name": "Moss", "color": "#3a7b5c"})).unwrap();
    assert_eq!(active_swatch(&app).as_deref(), Some("Moss"), "a swatch's colour");
}

#[test]
fn lab_spot_tiles_show_the_colour_the_spot_colors_options_pick() {
    let mut app = app();
    let lab = Color::lab(60.0, 50.0, 20.0);
    app.run("swatch.new", json!({"name": "Ink", "color": lab, "spot": true})).unwrap();
    let tile = |app: &VectorcraftApp| {
        let d = doc(app);
        shown(&d, d.swatch("Ink").expect("the Ink swatch")).0.color().unwrap()
    };
    assert_eq!(tile(&app), lab);
    app.run("swatch.spotOptions", json!({"useLab": false})).unwrap();
    assert_eq!(tile(&app), app.session.active().unwrap().doc.global_color("Ink").unwrap());
    assert!(matches!(tile(&app), Color::Cmyk { .. }));
}

/// The panel's Color Group Options opens on the selected color group when that group is the whole
/// selection; with nothing selected, a swatch alone, or a swatch or a second group beside the group,
/// it opens nothing and reports the missing group. After OK the group stays selected under its new name.
#[test]
fn color_group_options_opens_on_a_sole_selected_group() {
    let mut app = app();
    let key = crate::dialogs::color_group_options::KIND;
    for sel in [vec![], vec!["Red"], vec!["Brights", "Red"], vec!["Brights", "Grays"]] {
        select_rows(&mut app, &sel);
        assert!(app.run("ui.colorGroupOptions", json!({})).is_err() && !dialog_open(&app, key), "{sel:?}");
    }
    select_rows(&mut app, &["Grays"]);
    app.run("ui.colorGroupOptions", json!({})).unwrap();
    assert_eq!(dialog_field(&app, key, "name"), json!("Grays"));
    set_field(&mut app, key, "name", json!("Neutrals"));
    confirm(&mut app, key);
    assert_eq!(selected(&app), ["Neutrals"]);
    app.run("ui.colorGroupOptions", json!({})).unwrap();
    assert_eq!(dialog_field(&app, key, "name"), json!("Neutrals"));
    cancel(&mut app, key);
    // The command run on its own, and as batch steps, keeps the selection the same way.
    app.run("swatch.renameGroup", json!({"group": "Neutrals", "name": "Stone"})).unwrap();
    assert_eq!(selected(&app), ["Stone"]);
    let steps = json!([
        {"command": "swatch.renameGroup", "params": {"group": "Stone", "name": "Slate"}},
        {"command": "swatch.renameGroup", "params": {"group": "Slate", "name": "Ash"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(selected(&app), ["Ash"]);
}

/// Color Group Options without a `group` opens on the color group selected alone in the panel; in
/// another document than the one the selection was made in it opens nothing.
#[test]
fn color_group_options_without_a_group_opens_on_the_selected_group() {
    let mut app = app();
    let key = crate::dialogs::color_group_options::KIND;
    // The panel has shown the document, as it has before a click selects a row in it.
    let ctx = context(&app);
    frame(&mut app, &ctx, 0.0);
    select_rows(&mut app, &["Grays"]);
    app.run("ui.colorGroupOptions", json!({})).unwrap();
    assert_eq!(dialog_field(&app, key, "name"), json!("Grays"));
    cancel(&mut app, key);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    frame(&mut app, &ctx, 0.0);
    assert!(app.run("ui.colorGroupOptions", json!({})).is_err() && !dialog_open(&app, key), "another document");
}

/// Renames in a batch move the selection to another group neither after an Ungroup of the
/// selected group nor in another document; a batch step that keeps the groups (Sort by Kind) does
/// not stop the selection from following a rename, and Delete Swatch on the group ends it.
#[test]
fn a_batch_moves_the_selection_only_with_its_group() {
    let mut app = app();
    select_rows(&mut app, &["Brights"]);
    let steps = json!([
        {"command": "swatch.ungroup", "params": {"name": "Brights"}},
        {"command": "swatch.renameGroup", "params": {"group": "Grays", "name": "Brights"}},
        {"command": "swatch.renameGroup", "params": {"group": "Brights", "name": "Vivid"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert!(selected(&app).is_empty(), "Ungroup ends the selection");
    // A step that keeps the groups, then a rename of the selected group.
    select_rows(&mut app, &["Vivid"]);
    let steps = json!([
        {"command": "swatch.sortByKind", "params": {}},
        {"command": "swatch.renameGroup", "params": {"group": "Vivid", "name": "Bold"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(selected(&app), ["Bold"]);
    app.run("swatch.delete", json!({"name": "Bold"})).unwrap();
    assert!(selected(&app).is_empty(), "Delete Swatch ends the selection");
    // Brights selected in one new document, renamed in another.
    for _ in 0..2 {
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    }
    app.run("document.activate", json!({"index": 1})).unwrap();
    select_rows(&mut app, &["Brights"]);
    let steps = json!([
        {"command": "document.activate", "params": {"index": 2}},
        {"command": "swatch.renameGroup", "params": {"group": "Brights", "name": "Vivid"}},
        {"command": "document.activate", "params": {"index": 1}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(selected(&app), ["Brights"]);
}

/// Sort by Name keeps the groups, so a rename after it in a batch still moves the selection, and
/// Edit Color Group's `rename` moves it as Color Group Options does, alone or in a batch.
#[test]
fn the_selection_follows_sorting_and_group_edits() {
    let mut app = app();
    select_rows(&mut app, &["Brights"]);
    let steps = json!([
        {"command": "swatch.sortByName", "params": {}},
        {"command": "swatch.renameGroup", "params": {"group": "Brights", "name": "Vivid"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(selected(&app), ["Vivid"], "Sort by Name");
    app.run("swatch.editGroup", json!({"group": "Vivid", "colors": ["#ff0000"], "rename": "Bold"})).unwrap();
    assert_eq!(selected(&app), ["Bold"], "Edit Color Group");
    let steps = json!([
        {"command": "swatch.editGroup", "params": {"group": "Bold", "colors": ["#00ff00"], "rename": "Loud"}},
        {"command": "swatch.renameGroup", "params": {"group": "Loud", "name": "Calm"}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(selected(&app), ["Calm"], "Edit Color Group in a batch");
}

/// Colors applied while the panel's Recent Colors row is hidden still join the list, and a saved
/// `swatches_show_recent_colors` that is not true or false reads as on.
#[test]
fn show_recent_colors_keeps_collecting_and_reads_as_on_unless_false() {
    let mut app = app();
    app.ui.swatches_show_recent_colors = false;
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    assert_eq!(app.session.recent_colors.first().map(|c| c.to_hex()).as_deref(), Some("#00ff00"), "colors still join the list");
    let saved = |v: Value| {
        let mut prefs = serde_json::to_value(crate::UiState::default()).unwrap();
        prefs["swatches_show_recent_colors"] = v;
        serde_json::from_value::<crate::UiState>(prefs).unwrap().swatches_show_recent_colors
    };
    assert_eq!((saved(json!(false)), saved(json!(true)), saved(json!("no"))), (false, true, true));
}
