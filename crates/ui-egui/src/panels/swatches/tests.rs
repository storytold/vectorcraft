use super::*;
use crate::dialogs::swatch_options::KIND;
use egui::{Event, Modifiers, PointerButton, Pos2};
use vectorcraft_color::Color;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One frame of `draw` on a persistent context (clicks need the previous frame's layout).
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, time: f64, draw: fn(&mut VectorcraftApp, &mut Ui)) {
    frame_with(app, ctx, events, Modifiers::NONE, time, draw);
}

/// [`frame`] with `modifiers` held.
fn frame_with(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    events: Vec<Event>,
    modifiers: Modifiers,
    time: f64,
    draw: fn(&mut VectorcraftApp, &mut Ui),
) {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0)));
    let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
    let input = egui::RawInput { events, time: Some(time), screen_rect, ..Default::default() };
    let mut out = ctx.run_ui(input, |ui| draw(app, ui));
    out.textures_delta.clear();
}

fn context() -> egui::Context {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx
}

fn double_click(at: Pos2) -> Vec<Event> {
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    vec![Event::PointerMoved(at), button(true), button(false), button(true), button(false)]
}

/// Double-click the thumbnail of swatch `name` in the panel.
fn double_click_tile(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str) {
    frame(app, ctx, vec![], 0.0, show);
    let tile = ctx.read_response(egui::Id::unique(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect;
    frame(app, ctx, double_click(tile.center()), 1.0, show);
}

/// Click the thumbnail of swatch or group `name` with `modifiers` held (frames at `time`, `time + 0.5`).
fn click_tile(app: &mut VectorcraftApp, ctx: &egui::Context, name: &str, modifiers: Modifiers, time: f64) {
    frame(app, ctx, vec![], time, show);
    let at = ctx.read_response(egui::Id::unique(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect.center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
    frame_with(app, ctx, vec![Event::PointerMoved(at), button(true), button(false)], modifiers, time + 0.5, show);
}

fn selected(ctx: &egui::Context) -> Vec<String> {
    pstate(ctx, "swatch-selected")
}

fn dialog_frame(app: &mut VectorcraftApp, ctx: &egui::Context) {
    frame(app, ctx, vec![], 2.0, |app, ui| crate::dialogs::show(app, ui.ctx()));
}

#[test]
fn double_click_opens_swatch_options_and_ok_edits_linked_art() {
    let mut app = app();
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
    let ctx = context();
    double_click_tile(&mut app, &ctx, "Red");
    let d = app.ui.dialog.as_ref().expect("double-click opens Swatch Options");
    assert_eq!((d.kind.as_str(), d.str("__swatch").as_str(), d.str("mode").as_str()), (KIND, "Red", "rgb"));
    // The dialog previews live on the canvas; OK keeps the edit as one undo step.
    let undo = app.session.doc().unwrap().history.undo.len();
    dialog_frame(&mut app, &ctx);
    assert!(app.session.in_interaction(), "Preview runs as an interaction");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("color".into(), json!("#00ff00"));
    d.fields.insert("name".into(), json!("Signal"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.swatch("Signal").and_then(|w| w.paint.color()).map(|c| c.to_hex()), Some("#00ff00".into()));
    let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::Solid { color: Color::from_hex("#00ff00").unwrap(), swatch: Some("Signal".into()), tint: 1.0 });
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn cancel_rolls_the_preview_back_and_other_kinds_open_their_editors() {
    let mut app = app();
    let ctx = context();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("ui.swatchOptions", json!({"name": "Bright Blue"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("color".into(), json!("#000000"));
    dialog_frame(&mut app, &ctx);
    assert_ne!(app.session.doc().unwrap().doc, before, "previewed");
    crate::dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none() && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().doc, before);
    // A gradient opens Swatch Options with its name (and the gradient shown): OK renames it.
    double_click_tile(&mut app, &ctx, "Sunset");
    let d = app.ui.dialog.as_mut().expect("Swatch Options for a gradient");
    assert!(d.kind == KIND && d.fields.contains_key("__gradient") && !d.fields.contains_key("color"));
    d.fields.insert("name".into(), json!("Evening"));
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(matches!(app.session.doc().unwrap().doc.swatch("Evening").map(|w| &w.paint), Some(Paint::Gradient(_))));
    // Patterns open pattern editing, None nothing.
    assert!(app.run("ui.swatchOptions", json!({"name": "[None]"})).is_err());
    assert!(app.run("ui.swatchOptions", json!({})).is_err());
}

#[test]
fn modifier_clicks_select_several_swatches_and_groups() {
    let mut app = app();
    let ctx = context();
    click_tile(&mut app, &ctx, "Red", Modifiers::NONE, 0.0);
    assert_eq!(selected(&ctx), ["Red"]);
    assert_eq!(app.session.paint.fill.color().map(|c| c.to_hex()), Some("#ed1c24".into()), "a plain click applies");
    click_tile(&mut app, &ctx, "Amber", Modifiers::SHIFT, 2.0);
    assert_eq!(selected(&ctx), ["Red", "Orange Red", "Orange", "Amber"], "Shift extends over the shown order");
    click_tile(&mut app, &ctx, "Orange", Modifiers::COMMAND, 4.0);
    click_tile(&mut app, &ctx, "Brights", Modifiers::COMMAND, 6.0);
    assert_eq!(selected(&ctx), ["Red", "Orange Red", "Amber", "Brights"], "Cmd toggles swatches and colour groups");
    assert_eq!(app.session.paint.fill.color().map(|c| c.to_hex()), Some("#ed1c24".into()), "modifier clicks only select");
    click_tile(&mut app, &ctx, "Grays", Modifiers::NONE, 8.0);
    assert_eq!(selected(&ctx), ["Grays"], "a plain click on a group selects it alone");
}

#[test]
fn switching_documents_clears_the_swatch_range_anchor() {
    let mut app = app();
    let ctx = context();
    click_tile(&mut app, &ctx, "White", Modifiers::NONE, 0.0);
    assert_eq!(selected(&ctx), ["White"]);

    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    frame(&mut app, &ctx, vec![], 1.0, show);
    assert!(selected(&ctx).is_empty());

    click_tile(&mut app, &ctx, "Black", Modifiers::SHIFT, 2.0);
    assert_eq!(selected(&ctx), ["Black"]);
}

/// A color swatch or a recent color clicked in the panel becomes the Color Guide's base color
/// (#3137); a modifier click only selects.
#[test]
fn a_clicked_color_swatch_becomes_the_color_guides_base() {
    let mut app = app();
    let ctx = context();
    app.ui.color_guide_base = Some(Color::WHITE);
    click_tile(&mut app, &ctx, "Red", Modifiers::NONE, 0.0);
    let base = |app: &VectorcraftApp| crate::panels::color_guide::base_color(app).to_hex();
    assert_eq!(base(&app), "#ed1c24");
    click_tile(&mut app, &ctx, "Amber", Modifiers::SHIFT, 2.0);
    assert_eq!(base(&app), "#ed1c24", "a modifier click only selects");
    // A recent color: the panel's first recent chip.
    app.run("paint.setFill", json!({"color": "#2050a0"})).unwrap();
    app.ui.color_guide_base = Some(Color::WHITE);
    let mut chip = None;
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(280.0, 700.0)));
    let mut out = ctx.run_ui(egui::RawInput { time: Some(4.0), screen_rect, ..Default::default() }, |ui| {
        show(&mut app, ui);
        chip = ctx.read_response(ui.scope_id().with(("recent", 0usize))).map(|r| r.rect.center());
    });
    out.textures_delta.clear();
    let at = chip.expect("the recent chip is drawn");
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), button(true), button(false)], 5.0, show);
    assert_eq!(base(&app), "#2050a0");
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
    let ctx = context();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"color": "#000000"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    click_tile(&mut app, &ctx, "Red", Modifiers::ALT, 0.0);
    assert_eq!(stops_and_stroke(&app), (vec!["#ffffff".into(), "#ed1c24".into()], "#000000".into()), "the stop, not the stroke");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    assert_eq!(app.session.selected_stop(), Some(1), "the stop stays selected");
    assert!(app.session.fill_active, "the Fill proxy stays in front");
    // With no stop selected, Alt-click paints the inactive proxy.
    app.run("gradient.selectStop", json!({"index": null})).unwrap();
    click_tile(&mut app, &ctx, "Orange", Modifiers::ALT, 2.0);
    let orange = app.session.doc().unwrap().doc.swatch("Orange").and_then(|w| w.paint.color()).unwrap().to_hex();
    assert_eq!(stops_and_stroke(&app), (vec!["#ffffff".into(), "#ed1c24".into()], orange));
}

#[test]
fn alt_click_links_the_selected_stop_to_a_global_swatch() {
    let mut app = app();
    let ctx = context();
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    click_tile(&mut app, &ctx, "Red", Modifiers::ALT, 0.0);
    let Paint::Gradient(g) = crate::panels::current_paints(&app).0 else { panic!("a gradient fill") };
    assert_eq!(g.gradient.stops[0].swatch.as_deref(), Some("Red"), "linked as a drop on the ramp links it");
    app.run("swatch.edit", json!({"name": "Red", "color": "#00ff00"})).unwrap();
    assert_eq!(stops_and_stroke(&app).0, ["#00ff00", "#000000"], "editing the swatch recolors the stop");
}

#[test]
fn alt_click_recolors_the_selected_freeform_point() {
    let mut app = app();
    let ctx = context();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setStroke", json!({"color": "#000000", "focus": false})).unwrap();
    app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
    app.run("paint.freeform.selectPoint", json!({"index": 1})).unwrap();
    click_tile(&mut app, &ctx, "Red", Modifiers::ALT, 0.0);
    let (fill, stroke) = crate::panels::current_paints(&app);
    let Paint::Gradient(g) = fill else { panic!("still a gradient: {fill:?}") };
    assert_eq!(g.freeform.expect("placed points").points[1].color.to_hex(), "#ed1c24");
    assert_eq!(stroke.color().map(|c| c.to_hex()).as_deref(), Some("#000000"), "the stroke keeps its color");
    assert_eq!(app.session.selected_freeform_point(), Some(1), "the point stays selected");
}

#[test]
fn delete_asks_then_removes_the_selection_and_unlinks_art() {
    let mut app = app();
    let ctx = context();
    app.run("swatch.edit", json!({"name": "Red", "global": true})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Red"})).unwrap();
    delete(&mut app, vec!["Red".into(), "Brights".into()], false);
    let d = app.ui.dialog.as_ref().expect("delete asks first");
    assert_eq!((d.kind.as_str(), d.str("message").as_str()), (crate::dialogs::confirm::KIND, "Delete these 2 swatches and groups?"));
    dialog_frame(&mut app, &ctx);
    assert!(app.session.doc().unwrap().doc.swatch("Red").is_some(), "nothing is deleted before OK");
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let doc = &app.session.doc().unwrap().doc;
    assert!(doc.swatch("Red").is_none() && doc.swatch("Bright Red").is_none() && doc.swatch_groups.iter().all(|g| g.name != "Brights"));
    let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::solid(Color::from_hex("#ed1c24").unwrap()), "the art keeps its colour, unlinked");
    // Cancel keeps the swatch; Alt-click (now) deletes without asking.
    delete(&mut app, vec!["Orange".into()], false);
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("message"), "Delete “Orange”?");
    crate::dialogs::cancel(&mut app);
    assert!(app.session.doc().unwrap().doc.swatch("Orange").is_some());
    delete(&mut app, vec!["Orange".into()], true);
    assert!(app.ui.dialog.is_none() && app.session.doc().unwrap().doc.swatch("Orange").is_none());
    assert_eq!(deletable(&["[None]".into(), REGISTRATION.into(), "Grays".into()]), ["Grays"]);
}

#[test]
fn new_swatch_dialog_prefills_the_active_colour_and_saves_into_the_selected_group() {
    let mut app = app();
    let ctx = context();
    app.run("paint.setFill", json!({"color": "#ff8000"})).unwrap();
    new_swatch(&mut app, &["Bright Red".into()], Modifiers::COMMAND);
    let d = app.ui.dialog.as_ref().expect("New Swatch opens its dialog");
    assert_eq!((d.kind.as_str(), d.str("name").as_str(), d.str("group").as_str()), (crate::dialogs::new_swatch::KIND, "R=255 G=128 B=0", "Brights"));
    assert!(d.bool("spot") && d.bool("global"), "Cmd/Ctrl-click makes a spot colour");
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    let doc = &app.session.doc().unwrap().doc;
    let w = doc.swatch("R=255 G=128 B=0").expect("created");
    assert!(w.spot && doc.swatch_group_of(&w.name).is_some_and(|g| doc.swatch_groups[g].name == "Brights"));
    // Alt-click saves at once; a gradient takes only a name and stays out of colour groups.
    app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
    new_swatch(&mut app, &["Brights".into()], Modifiers::ALT);
    assert!(app.ui.dialog.is_none() && app.session.doc().unwrap().doc.swatch("New Gradient Swatch 1").is_some());
    new_swatch(&mut app, &[], Modifiers::NONE);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("name").as_str(), d.fields.contains_key("__paint")), ("New Gradient Swatch 2", true));
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.session.doc().unwrap().doc.swatch("New Gradient Swatch 2").is_some_and(|w| matches!(w.paint, Paint::Gradient(_))));
}

#[test]
fn cmd_shift_click_on_new_swatch_makes_a_global_process_color() {
    let mut app = app();
    let ctx = context();
    let global_process = Modifiers::COMMAND | Modifiers::SHIFT;
    app.run("paint.setFill", json!({"color": "#2a6fb0"})).unwrap();
    new_swatch(&mut app, &[], global_process);
    let d = app.ui.dialog.as_ref().expect("New Swatch opens its dialog");
    assert!(d.bool("global") && !d.bool("spot"), "Cmd/Ctrl+Shift-click opens a global process color");
    let name = d.str("name");
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    let w = app.session.doc().unwrap().doc.swatch(&name).cloned().expect("created");
    assert!(w.global && !w.spot);
    // With Alt also held it saves at once.
    app.run("paint.setFill", json!({"color": "#00a050"})).unwrap();
    new_swatch(&mut app, &[], global_process | Modifiers::ALT);
    assert!(app.ui.dialog.is_none());
    let w = app.session.doc().unwrap().doc.swatches.last().cloned().unwrap();
    assert_eq!(w.paint.color().map(|c| c.to_hex()), Some("#00a050".into()));
    assert!(w.global && !w.spot);
    // A gradient saves as a gradient swatch whether or not Cmd/Ctrl and Shift are held.
    app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
    new_swatch(&mut app, &[], Modifiers::COMMAND | Modifiers::ALT);
    new_swatch(&mut app, &[], global_process | Modifiers::ALT);
    let doc = &app.session.doc().unwrap().doc;
    for n in ["New Gradient Swatch 1", "New Gradient Swatch 2"] {
        assert!(doc.swatch(n).is_some_and(|w| matches!(w.paint, Paint::Gradient(_)) && !w.global && !w.spot), "{n}");
    }
}

/// In a CMYK document a swatch made from an RGB fill keeps RGB, as the Color Guide's copies
/// do: Alt-click on New Swatch, the New Swatch dialog, the Fill proxy dropped on the panel and
/// Alt-dropped on a color swatch, and the same for a gradient's stops.
#[test]
fn swatches_made_from_an_rgb_fill_stay_rgb_in_a_cmyk_document() {
    let mut app = app();
    let ctx = context();
    app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
    let newest = |app: &VectorcraftApp| app.session.doc().unwrap().doc.swatches.last().map(|w| w.paint.clone()).expect("a swatch");
    let stop_colors = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.color).collect::<Vec<_>>(),
        other => panic!("a gradient: {other:?}"),
    };
    let drop_on = |app: &mut VectorcraftApp, name: &str, paint: Paint, m: Modifiers, time: f64| {
        frame(app, &ctx, vec![], time, show);
        let at = tile_center(&ctx, name);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::paint(paint));
        let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: m };
        frame_with(app, &ctx, vec![Event::PointerMoved(at), release], m, time + 0.5, show);
    };
    let rgb = Color::from_hex("#2a6fb0").unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#2a6fb0", "keepModel": true})).unwrap();
    new_swatch(&mut app, &[], Modifiers::ALT);
    assert_eq!(newest(&app), Paint::solid(rgb), "Alt-click on New Swatch");
    new_swatch(&mut app, &[], Modifiers::NONE);
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(newest(&app), Paint::solid(rgb), "the New Swatch dialog");
    drop_on(&mut app, "Red", Paint::solid(rgb), Modifiers::NONE, 3.0);
    assert_eq!(newest(&app), Paint::solid(rgb), "the proxy dropped on the panel: {}", app.ui.status);
    drop_on(&mut app, "Red", Paint::solid(rgb), Modifiers::ALT, 5.0);
    assert_eq!(app.session.doc().unwrap().doc.swatch("Red").map(|w| w.paint.color()), Some(Some(rgb)), "Alt-dropped on Red");
    // A gradient's RGB stops.
    let stops = json!([{"offset": 0, "color": "#ff8000"}, {"offset": 1, "color": "#2a6fb0"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}, "keepModel": true})).unwrap();
    let grad = crate::panels::current_paints(&app).0;
    let rgb_stops = vec![Color::from_hex("#ff8000").unwrap(), rgb];
    assert_eq!(stop_colors(&grad), rgb_stops);
    new_swatch(&mut app, &[], Modifiers::ALT);
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "Alt-click on New Swatch");
    new_swatch(&mut app, &[], Modifiers::NONE);
    frame(&mut app, &ctx, vec![], 6.0, |app, ui| crate::dialogs::show(app, ui.ctx()));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "the New Swatch dialog");
    drop_on(&mut app, "Red", grad.clone(), Modifiers::NONE, 7.0);
    assert_eq!(stop_colors(&newest(&app)), rgb_stops, "the gradient dropped on the panel");
    drop_on(&mut app, "Ocean", grad, Modifiers::ALT, 9.0);
    let ocean = app.session.doc().unwrap().doc.swatch("Ocean").map(|w| w.paint.clone()).expect("Ocean");
    assert_eq!(stop_colors(&ocean), rgb_stops, "Alt-dropped on Ocean");
}

#[test]
fn new_color_group_dialog_makes_a_group_from_swatches_or_artwork() {
    let mut app = app();
    let ctx = context();
    app.run("ui.newColorGroup", json!({"swatches": ["Red", "Sunset"]})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("name").as_str(), d.bool("fromArtwork")), (crate::dialogs::new_color_group::KIND, "Color Group", false));
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    let doc = &app.session.doc().unwrap().doc;
    let g = doc.swatch_groups.iter().find(|g| g.name == "Color Group").expect("group made");
    assert_eq!(g.swatches.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), ["Red"], "gradients stay out");
    // With art selected (and no swatches) it starts from the artwork: its colours become global.
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [id], "color": "#123456"})).unwrap();
    app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.run("ui.newColorGroup", json!({})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("name").as_str(), d.bool("fromArtwork"), d.bool("toGlobal")), ("Color Group 2", true, true));
    dialog_frame(&mut app, &ctx);
    crate::dialogs::confirm(&mut app).unwrap();
    let doc = &app.session.doc().unwrap().doc;
    let g = doc.swatch_groups.iter().find(|g| g.name == "Color Group 2").expect("group made");
    assert_eq!(g.swatches.iter().map(|w| (w.name.as_str(), w.global)).collect::<Vec<_>>(), [("R=18 G=52 B=86", true)]);
    let fill = doc.node(vectorcraft_doc::NodeId(id.as_u64().unwrap())).unwrap().appearance.fill_paint();
    assert_eq!(fill, Paint::Solid { color: Color::from_hex("#123456").unwrap(), swatch: Some("R=18 G=52 B=86".into()), tint: 1.0 });
}

/// The centre of the tile or row of `name` as laid out by the last frame.
fn tile_center(ctx: &egui::Context, name: &str) -> Pos2 {
    ctx.read_response(egui::Id::unique(("swatch-tile", name))).unwrap_or_else(|| panic!("no tile for {name}")).rect.center()
}

/// Press at `from`, drag to `to` and release there, a frame per step from `time`.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2, time: f64) {
    drag_with(app, ctx, from, to, Modifiers::NONE, time);
}

/// [`drag`] with `modifiers` held.
fn drag_with(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2, modifiers: Modifiers, time: f64) {
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
    frame_with(app, ctx, vec![Event::PointerMoved(from), button(from, true)], modifiers, time, show);
    frame_with(app, ctx, vec![Event::PointerMoved(from + vec2(6.0, 4.0))], modifiers, time + 0.1, show);
    frame_with(app, ctx, vec![Event::PointerMoved(to)], modifiers, time + 0.2, show);
    frame_with(app, ctx, vec![Event::PointerMoved(to), button(to, false)], modifiers, time + 0.3, show);
}

fn names_of(app: &VectorcraftApp, group: Option<&str>) -> Vec<String> {
    let d = &app.session.active().unwrap().doc;
    let list = match group {
        Some(g) => &d.swatch_groups.iter().find(|x| x.name == g).unwrap().swatches,
        None => &*d.swatches,
    };
    list.iter().map(|w| w.name.clone()).collect()
}

#[test]
fn find_field_filters_and_shift_extends_over_what_is_shown() {
    let mut app = app();
    let ctx = context();
    set_pstate(&ctx, "swatch-show-find", true);
    ctx.data_mut(|d| d.insert_temp(find_id(), "BRIGHT".to_string()));
    frame(&mut app, &ctx, vec![], 0.0, show);
    assert!(ctx.read_response(egui::Id::unique(("swatch-tile", "Red"))).is_none(), "Red doesn't match");
    click_tile(&mut app, &ctx, "Bright Red", Modifiers::NONE, 1.0);
    click_tile(&mut app, &ctx, "Bright Blue", Modifiers::SHIFT, 2.0);
    assert_eq!(selected(&ctx), ["Bright Red", "Bright Yellow", "Bright Green", "Bright Blue"]);
    // A group whose name matches shows all its swatches.
    ctx.data_mut(|d| d.insert_temp(find_id(), "grays".to_string()));
    frame(&mut app, &ctx, vec![], 3.0, show);
    frame(&mut app, &ctx, vec![], 3.5, show);
    assert!(ctx.read_response(egui::Id::unique(("swatch-tile", "K=50"))).is_some());
    assert!(ctx.read_response(egui::Id::unique(("swatch-tile", "Bright Red"))).is_none());
}

#[test]
fn list_view_rows_take_double_clicks_and_folder_clicks_select_the_group() {
    let mut app = app();
    let ctx = context();
    set_pstate(&ctx, "swatch-view", View::SmallList);
    double_click_tile(&mut app, &ctx, "Red");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "double-click in a list opens Swatch Options");
    let d = &app.session.active().unwrap().doc;
    let sel = vec!["Brights".to_string()];
    assert!(highlighted(d, &sel).contains(&"Bright Violet"), "a selected folder highlights its swatches");
    assert_eq!(describe(&Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), true, false), "Global Process Color, CMYK");
    assert_eq!(describe(&Paint::solid(Color::gray(0.5)), true, true), "Spot Color, Grayscale");
}

#[test]
fn dragging_tiles_reorders_and_moves_into_groups_as_one_undo_step_each() {
    let mut app = app();
    let ctx = context();
    frame(&mut app, &ctx, vec![], 0.0, show);
    let undo = app.session.doc().unwrap().history.undo.len();
    // Onto the left half of Red: before it.
    let (from, red) = (tile_center(&ctx, "Amber"), tile_center(&ctx, "Red"));
    drag(&mut app, &ctx, from, red - vec2(4.0, 0.0), 1.0);
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Amber");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    // Onto a folder: into that group, at its end. (Two frames, so tile rects are current.)
    frame(&mut app, &ctx, vec![], 2.0, show);
    frame(&mut app, &ctx, vec![], 2.5, show);
    drag(&mut app, &ctx, tile_center(&ctx, "Amber"), tile_center(&ctx, "Grays"), 3.0);
    assert_eq!(names_of(&app, Some("Grays")).last().map(String::as_str), Some("Amber"));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 2);
    // A selection drags together, in panel order; a gradient can't join a group.
    set_pstate(&ctx, "swatch-selected", vec!["Lime".to_string(), "Sunset".to_string(), "Orange".to_string()]);
    frame(&mut app, &ctx, vec![], 4.0, show);
    frame(&mut app, &ctx, vec![], 4.5, show);
    drag(&mut app, &ctx, tile_center(&ctx, "Lime"), tile_center(&ctx, "Bright Red") - vec2(4.0, 0.0), 5.0);
    assert_eq!(names_of(&app, Some("Brights"))[..3], ["Orange", "Lime", "Bright Red"]);
    assert!(names_of(&app, None).contains(&"Sunset".to_string()));
    // Dropped back on itself: nothing moves.
    let (before, undo) = (names_of(&app, None), app.session.doc().unwrap().history.undo.len());
    frame(&mut app, &ctx, vec![], 6.0, show);
    frame(&mut app, &ctx, vec![], 6.5, show);
    let white = tile_center(&ctx, "White");
    drag(&mut app, &ctx, white, white + vec2(2.0, 0.0), 7.0);
    assert_eq!((names_of(&app, None), app.session.doc().unwrap().history.undo.len()), (before, undo));
}

#[test]
fn art_dropped_on_the_panel_becomes_a_pattern_swatch() {
    let mut app = app();
    let id = app.run("shape.star", json!({"cx": 50, "cy": 50, "radius1": 20, "radius2": 9, "points": 5})).unwrap()["id"].as_u64().unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let ctx = context();
    frame(&mut app, &ctx, vec![], 0.0, show);
    // Released over a swatch tile: the list takes it.
    let at = tile_center(&ctx, "Bright Red");
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::Art(vec![vectorcraft_doc::NodeId(id)]));
    let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), release], 1.0, show);
    let patterns = app.run("pattern.list", json!({})).unwrap();
    assert_eq!(patterns["patterns"].as_array().map(Vec::len), Some(1), "{patterns}");
    assert_eq!(patterns["editing"], Value::Null, "no pattern editing mode");
    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)), before.node(vectorcraft_doc::NodeId(id)), "the art stays as it was");
    assert!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).is_none());
}

#[test]
fn a_proxy_dropped_on_the_panel_becomes_a_swatch() {
    let mut app = app();
    let ctx = context();
    frame(&mut app, &ctx, vec![], 0.0, show);
    let at = tile_center(&ctx, "Bright Red");
    egui::DragAndDrop::set_payload(&ctx, PanelDrag::paint(Paint::solid(Color::from_hex("#123456").unwrap())));
    let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, vec![Event::PointerMoved(at), release], 1.0, show);
    assert_eq!(names_of(&app, Some("Brights")).last().map(String::as_str), Some("R=18 G=52 B=86"), "dropped on a group's colour: into that group");
    assert!(egui::DragAndDrop::payload::<PanelDrag>(&ctx).is_none());
}

#[test]
fn alt_dropping_a_gradient_on_a_gradient_swatch_replaces_it() {
    let mut app = app();
    let ctx = context();
    frame(&mut app, &ctx, vec![], 0.0, show);
    let grad = |c: &str| {
        let stops =
            vec![vectorcraft_color::GradientStop::new(0.0, Color::from_hex(c).unwrap()), vectorcraft_color::GradientStop::new(1.0, Color::WHITE)];
        Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::new(GradientKind::Radial, stops))))
    };
    let drop_on = |app: &mut VectorcraftApp, name: &str, paint: Paint, m: Modifiers, time: f64| {
        let at = tile_center(&ctx, name);
        egui::DragAndDrop::set_payload(&ctx, PanelDrag::paint(paint));
        let release = Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: m };
        frame_with(app, &ctx, vec![Event::PointerMoved(at), release], m, time, show);
    };
    let count = app.session.doc().unwrap().doc.swatches_iter().count();
    drop_on(&mut app, "Ocean", grad("#ff0000"), Modifiers::ALT, 1.0);
    let d = &app.session.doc().unwrap().doc;
    let Some(Paint::Gradient(g)) = d.swatch("Ocean").map(|w| &w.paint) else { panic!("Ocean is a gradient") };
    assert_eq!((g.gradient.kind, d.swatches_iter().count()), (GradientKind::Radial, count), "replaced, not added");
    // Without Alt, or on a swatch of another kind, it becomes a new swatch.
    frame(&mut app, &ctx, vec![], 2.0, show);
    drop_on(&mut app, "Red", grad("#00ff00"), Modifiers::ALT, 3.0);
    assert_eq!(app.session.doc().unwrap().doc.swatches_iter().count(), count + 1);
    assert!(app.session.doc().unwrap().doc.swatch("Red").is_some_and(|w| w.paint.color().is_some()));
    // A tint swatch only takes a new name, so a color Alt-dropped on one becomes a new swatch.
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    frame(&mut app, &ctx, vec![], 4.0, show);
    frame(&mut app, &ctx, vec![], 4.5, show);
    drop_on(&mut app, "Orange 40%", Paint::solid(Color::from_hex("#123456").unwrap()), Modifiers::ALT, 5.0);
    let d = &app.session.doc().unwrap().doc;
    assert_eq!(d.swatches_iter().count(), count + 3, "{}", app.ui.status);
    assert!(d.swatch("Orange 40%").is_some_and(|w| w.tint_of().is_some()));
}

#[test]
fn alt_dragging_a_swatch_onto_another_of_its_kind_replaces_its_color() {
    let mut app = app();
    let ctx = context();
    app.run("swatch.edit", json!({"name": "Orange", "global": true})).unwrap();
    app.run("swatch.new", json!({"swatch": "Orange", "tint": 40})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"ids": [id], "swatch": "Orange"})).unwrap();
    frame(&mut app, &ctx, vec![], 0.0, show);
    frame(&mut app, &ctx, vec![], 0.5, show);
    let doc = |app: &VectorcraftApp| app.session.doc().unwrap().doc.clone();
    let (before, undo) = (names_of(&app, None), app.session.doc().unwrap().history.undo.len());
    let red = doc(&app).swatch("Red").and_then(|w| w.paint.color()).unwrap();
    drag_with(&mut app, &ctx, tile_center(&ctx, "Red"), tile_center(&ctx, "Orange"), Modifiers::ALT, 1.0);
    let d = doc(&app);
    let orange = d.swatch("Orange").expect("Orange keeps its name");
    assert_eq!((orange.paint.color(), orange.global), (Some(red), true), "Orange takes Red's color");
    assert_eq!(names_of(&app, None), before, "Red stays and Orange keeps its place");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let fill = d.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
    assert_eq!(fill.color(), Some(red), "art linked to Orange takes the new color");
    // A gradient dropped on a color, and a swatch dropped without Alt, move as before.
    frame(&mut app, &ctx, vec![], 2.0, show);
    frame(&mut app, &ctx, vec![], 2.5, show);
    drag_with(&mut app, &ctx, tile_center(&ctx, "Sunset"), tile_center(&ctx, "Amber") - vec2(4.0, 0.0), Modifiers::ALT, 3.0);
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Amber").unwrap() - 1], "Sunset");
    assert!(doc(&app).swatch("Amber").is_some_and(|w| w.paint.color().is_some()));
    frame(&mut app, &ctx, vec![], 4.0, show);
    frame(&mut app, &ctx, vec![], 4.5, show);
    drag(&mut app, &ctx, tile_center(&ctx, "Lime"), tile_center(&ctx, "Red") - vec2(4.0, 0.0), 5.0);
    let n = names_of(&app, None);
    assert_eq!(n[n.iter().position(|x| x == "Red").unwrap() - 1], "Lime");
    assert_ne!(doc(&app).swatch("Red").and_then(|w| w.paint.color()), doc(&app).swatch("Lime").and_then(|w| w.paint.color()));
    // A tint swatch only takes a new name, so a swatch Alt-dropped on one moves.
    frame(&mut app, &ctx, vec![], 6.0, show);
    frame(&mut app, &ctx, vec![], 6.5, show);
    drag_with(&mut app, &ctx, tile_center(&ctx, "Lime"), tile_center(&ctx, "Orange 40%") - vec2(4.0, 0.0), Modifiers::ALT, 7.0);
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
    let tint = entries(&app, Kind::Color, "Ink 40").into_iter().find(|e| e.name() == "Ink 40%").unwrap();
    assert!(matches!(tint, Entry::Swatch { global: true, spot: true, .. }), "a tint swatch shows its base's kind");
    app.run("paint.setFill", json!({"swatch": "Sunset"})).unwrap();
    assert_eq!(active_swatch(&app).as_deref(), Some("Sunset"), "the applied gradient swatch");
    app.run("ui.swatchOptions", json!({"name": "Ink 40%"})).unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.str("__swatch").as_str()), (KIND, "Ink"), "a tint swatch opens its base's options");
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
    let ctx = context();
    let tile = |ctx: &egui::Context| ctx.read_response(egui::Id::unique(("swatch-tile", "Last Ink"))).unwrap().rect;
    frame(&mut app, &ctx, vec![], 0.0, show);
    frame(&mut app, &ctx, vec![], 0.1, show);
    let far = tile(&ctx).top();
    set_pstate(&ctx, "swatch-selected", vec!["Red".to_string()]);
    app.run("select.set", json!({"ids": [id]})).unwrap();
    for t in [0.2, 0.3, 0.4] {
        frame(&mut app, &ctx, vec![], t, show);
    }
    let near = tile(&ctx).top();
    assert!(near < far - 100.0, "scrolled to the swatch: {far} → {near}");
    assert!(selected(&ctx).is_empty(), "the art's swatch is highlighted");
    // Once: later frames leave the scroll where the user puts it.
    assert_eq!(focus_swatch(&app, &egui::Ui::new(ctx.clone(), egui::Id::unique("t"), egui::UiBuilder::new()), Some("Last Ink")), None);
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
    let ctx = context();
    let ui = egui::Ui::new(ctx.clone(), egui::Id::unique("t"), egui::UiBuilder::new());
    let shown = |app: &VectorcraftApp| kept_swatch(app, &ui, active_swatch(app));
    assert_ne!(shown(&app).as_deref(), Some("Ink 185"), "nothing selected yet: new art's paint");
    app.run("select.set", json!({"ids": [id]})).unwrap();
    assert_eq!(shown(&app).as_deref(), Some("Ink 185"));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(shown(&app).as_deref(), Some("Ink 185"), "kept after deselecting");
    assert_eq!(shown(&app).as_deref(), Some("Ink 185"), "and on later frames");
    // A colour picked with nothing selected is new art's: its swatch shows.
    app.run("paint.setFill", json!({"swatch": "Black"})).unwrap();
    assert_eq!(shown(&app).as_deref(), Some("Black"));
    // Other art selected (a white one) shows its swatch, and keeps it once deselected.
    let white = app.run("shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50})).unwrap()["id"].clone();
    app.run("paint.setFill", json!({"ids": [white.clone()], "swatch": "White"})).unwrap();
    app.run("select.set", json!({"ids": [white]})).unwrap();
    assert_eq!(shown(&app).as_deref(), Some("White"));
    app.run("select.none", json!({})).unwrap();
    assert_eq!(shown(&app).as_deref(), Some("White"));
}

/// A selected image, or text in a colour no swatch has, highlights no swatch (not None); text in
/// a swatch's colour highlights that swatch.
#[test]
fn images_and_unswatched_text_highlight_no_swatch() {
    let mut app = app();
    let mut png = vec![];
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
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
fn medium_tiles_match_measured_metrics() {
    assert_eq!(View::MediumThumb.tile(), (15.5, 17.0));
    assert!(View::SmallList.is_list() && !View::LargeThumb.is_list());
}

#[test]
fn lab_spot_tiles_show_the_colour_the_spot_colors_options_pick() {
    let mut app = app();
    let lab = Color::lab(60.0, 50.0, 20.0);
    app.run("swatch.new", json!({"name": "Ink", "color": lab, "spot": true})).unwrap();
    let tile = |app: &VectorcraftApp| {
        let ink = entries(app, Kind::All, "Ink").into_iter().find(|e| e.name() == "Ink");
        let Some(Entry::Swatch { paint, .. }) = ink else { panic!("the Ink tile") };
        paint.color().unwrap()
    };
    assert_eq!(tile(&app), lab);
    app.run("swatch.spotOptions", json!({"useLab": false})).unwrap();
    assert_eq!(tile(&app), app.session.active().unwrap().doc.global_color("Ink").unwrap());
    assert!(matches!(tile(&app), Color::Cmyk { .. }));
}
