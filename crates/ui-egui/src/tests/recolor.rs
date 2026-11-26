//! Recolor Artwork (M3.78) and Edit or Apply Color Group (M3.79), driven through the dialog host.

use std::sync::Arc;

use craft::color_ui::ui::ColorPickerDialog;
use craft::ui::dialog::{DialogId, HeldOk, StackDialog};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::recolor::ColorKey;
use vectorcraft_engine::Session;

use crate::dialogs::recolor::fixture::Recolor;
use crate::{VectorcraftApp, dialogs};

// PLAN: assumed control names: the wheel's accessible name is "Color wheel" / "Segmented color wheel", its markers are
// named by their `#rrggbb` hex, menu items carry no leading padding, and the Assign row's hex field is "Color N".

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app
}

/// Recolor Artwork opened on `app` by `ui.recolorDialog` with `params`; no frame has run.
fn open(app: VectorcraftApp, params: Value) -> Recolor {
    let r = Recolor::new(app);
    r.app().run("ui.recolorDialog", params).unwrap();
    r
}

/// Field `k` of the open dialog; null when it has no such field.
fn field(r: &Recolor, k: &str) -> Value {
    r.field(k).unwrap_or_default()
}

/// Whether Undo Changes and Redo Changes can run.
fn steps(r: &Recolor) -> (bool, bool) {
    let app = r.app();
    (dialogs::recolor::can_step(&app, true), dialogs::recolor::can_step(&app, false))
}

/// The Color Picker open over Recolor Artwork: its id, the key it was asked for with and its fields.
fn picker(r: &Recolor) -> Option<(DialogId, String, Value)> {
    r.host(|host, _| host.get::<ColorPickerDialog>().map(|p| (p.id(), p.key().to_owned(), p.json_fields().unwrap_or_default()))).flatten()
}

fn pointer(at: Pos2, button: PointerButton, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE }
}

fn press(at: Pos2, clicks: usize) -> Vec<Event> {
    let button = |pressed| pointer(at, PointerButton::Primary, pressed);
    std::iter::once(Event::PointerMoved(at)).chain((0..clicks).flat_map(|_| [button(true), button(false)])).collect()
}

/// A primary click at `at`, then the dialog settles.
fn click(r: &mut Recolor, at: Pos2) {
    r.frame_with(press(at, 1), Modifiers::NONE);
    r.settled();
}

/// A secondary click at `at`, then the dialog settles with its menu open.
fn right_click(r: &mut Recolor, at: Pos2) {
    r.frame_with(
        vec![Event::PointerMoved(at), pointer(at, PointerButton::Secondary, true), pointer(at, PointerButton::Secondary, false)],
        Modifiers::NONE,
    );
    r.settled();
}

/// Presses at the first point of `path` and moves through the rest, one frame each; the button stays held.
fn drag_path(r: &mut Recolor, path: &[Pos2]) {
    let Some((first, rest)) = path.split_first() else { return };
    r.frame_with(vec![Event::PointerMoved(*first)], Modifiers::NONE);
    r.frame_with(vec![pointer(*first, PointerButton::Primary, true)], Modifiers::NONE);
    for p in rest {
        r.frame_with(vec![Event::PointerMoved(*p)], Modifiers::NONE);
    }
}

/// Lets go of the primary button at `at` and settles.
fn let_go(r: &mut Recolor, at: Pos2) {
    r.frame_with(vec![pointer(at, PointerButton::Primary, false)], Modifiers::NONE);
    r.settled();
}

/// `steps + 1` points from `from` to `to`.
fn line(from: Pos2, to: Pos2, steps: usize) -> Vec<Pos2> {
    (0..=steps).map(|k| from.lerp(to, k as f32 / steps as f32)).collect()
}

/// The point `d` from `c` in the direction of hue `hue` (counter-clockwise from the right, as the wheel draws).
fn polar(c: Pos2, hue: f32, d: f32) -> Pos2 {
    c + vec2(hue.to_radians().cos(), -hue.to_radians().sin()) * d
}

/// Clicks the control named `label`, then the dialog settles.
fn tap(r: &mut Recolor, label: &str) {
    r.click_label(label);
    r.settled();
}

/// Sets field `k` and lets the dialog draw it.
fn put(r: &mut Recolor, k: &str, v: Value) {
    r.set(k, v).unwrap();
    r.settled();
}

fn rect(r: &Recolor, label: &str) -> Rect {
    r.by_label(label).unwrap_or_else(|| panic!("no control named {label}")).rect
}

fn has(r: &Recolor, label: &str) -> bool {
    r.by_label(label).is_some()
}

/// Clicks the channel rail named `name` at `at` (0 to 1) of its thumb's travel.
fn rail(r: &mut Recolor, name: &str, at: f32) {
    let y = r.slider(name).unwrap_or_else(|| panic!("no slider named {name}")).rect.center().y;
    let travel = r.slider_travel(name).unwrap_or_else(|| panic!("{name}'s rail has no thumb"));
    click(r, pos2(travel.min + at * travel.span(), y));
}

/// The wheel's centre and radius. The disc sits inside its control's rect by the theme's padding, so the radius is
/// read from where the wheel draws a marker: its distance from the centre over the share of the radius its colour
/// stands for (saturation, or brightness on the hue and brightness wheel).
fn wheel(r: &Recolor) -> (Pos2, f32) {
    let wheel = r.by_label("Color wheel").or_else(|| r.by_label("Segmented color wheel")).expect("the wheel").rect;
    let c = wheel.center();
    let bright = field(r, "wheelAxes") == json!("hueBrightness");
    let radius = field(r, "rows").as_array().into_iter().flatten().find_map(|row| {
        let hsb = vectorcraft_engine::cmd::color_value(&row["to"])?.to_hsb();
        let share = if bright { hsb[2] } else { hsb[1] };
        let at = r.by_label(&hex_of(&row["to"]))?.rect.center();
        (share > 0.2).then(|| at.distance(c) / share)
    });
    (c, radius.unwrap_or(wheel.width().min(wheel.height()) / 2.0))
}

/// Where the wheel draws hue `hue` at saturation `s` (or brightness, on the hue and brightness wheel).
fn wheel_at(r: &Recolor, hue: f32, s: f32) -> Pos2 {
    let (c, radius) = wheel(r);
    polar(c, hue, radius * s)
}

/// The marker's name: the lowercase hex of the colour `to`, as the wheel names it.
fn hex_of(to: &Value) -> String {
    let [r, g, b] = vectorcraft_engine::cmd::color_value(to).unwrap().to_rgb();
    let byte = |v: f32| (v * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", byte(r), byte(g), byte(b))
}

/// Where the wheel draws row `i`'s marker.
fn marker(r: &Recolor, i: usize) -> Pos2 {
    rect(r, &hex_of(&field(r, "rows")[i]["to"])).center()
}

fn key(r: &mut Recolor, key: Key) {
    r.frame_with(vec![Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: Modifiers::NONE }], Modifiers::NONE);
    r.settled();
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn snapshot(app: &VectorcraftApp) -> Arc<vectorcraft_doc::Document> {
    app.session.doc().unwrap().doc.clone()
}

/// Rectangles filled with `colors`, all selected.
fn art(app: &mut VectorcraftApp, colors: &[&str]) {
    for (i, c) in colors.iter().enumerate() {
        let id = app.run("shape.rectangle", json!({"x": 10 + 30 * i, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"ids": [id], "color": c})).unwrap();
        app.run("paint.setStroke", json!({"ids": [id], "none": true})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
}

fn colors(app: &mut VectorcraftApp) -> usize {
    app.session.execute("recolor.colors", &json!({})).unwrap()["colors"].as_array().unwrap().len()
}

fn group_keys(app: &VectorcraftApp, name: &str) -> Vec<Value> {
    let d = &app.session.doc().unwrap().doc;
    let g = d.swatch_groups.iter().find(|g| g.name == name).unwrap();
    g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| json!(ColorKey::of(&c).to_string())).collect()
}

/// Recolor Artwork on art filled with `colors`, on the Edit tab.
fn edit(colors: &[&str]) -> Recolor {
    let mut app = app();
    art(&mut app, colors);
    let mut r = open(app, json!({}));
    r.set("tab", json!("edit")).unwrap();
    assert_eq!(field(&r, "linked"), json!(true));
    r.settled();
    r
}

/// Recolor Artwork open on the Assign tab of `app` (its art selected).
fn assign(app: VectorcraftApp, params: Value) -> Recolor {
    let mut r = open(app, params);
    r.set("method", json!("exact")).unwrap();
    r.settled();
    r
}

/// The open Recolor with Preset dialog: its id and its fields.
fn preset(r: &Recolor) -> Option<(DialogId, Value)> {
    r.host(|host, _| {
        let open = host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(dialogs::recolor_preset::KIND));
        open.map(|d| (d.id, d.fields.unwrap_or_default()))
    })
    .flatten()
}

/// Field `k` of the open Recolor with Preset dialog; null when it is not open.
fn preset_field(r: &Recolor, k: &str) -> Value {
    preset(r).map(|(_, fields)| fields[k].clone()).unwrap_or_default()
}

/// Sets field `k` of the open Recolor with Preset dialog, as `ui.dialog.set` does.
fn set_preset(r: &mut Recolor, k: &str, v: Value) -> Result<(), String> {
    let (id, _) = preset(r).ok_or("no Recolor with Preset")?;
    r.host(|host, ctx| host.set_json_field(ctx, id, k, v)).unwrap_or_else(|| Err("no window".into()))
}

/// OK in Recolor with Preset, as `ui.dialog.confirm` gives it, and the frames in which it closes.
/// `Err` with the reason it was refused, which keeps the dialog open.
fn confirm_preset(r: &mut Recolor) -> Result<(), String> {
    let (id, _) = preset(r).ok_or("no Recolor with Preset")?;
    r.host(|host, ctx| host.confirm_by_id(ctx, id));
    r.frame().frame();
    match r.host(|host, _| host.held_state(id)).flatten() {
        Some(HeldOk::Refused(reason)) => Err(reason.as_str().to_owned()),
        _ if preset(r).is_some() => Err("Recolor with Preset stayed open".into()),
        _ => Ok(()),
    }
}

/// Cancel in Recolor with Preset, and the frame in which it closes.
fn cancel_preset(r: &mut Recolor) {
    if let Some((id, _)) = preset(r) {
        r.host(|host, ctx| host.cancel_by_id(ctx, id));
    }
    r.frame();
}

#[test]
fn the_dialog_reduces_previews_and_applies_as_one_undo_step() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0", "#ff3030"]);
    let mut r = open(app, json!({}));
    r.settled();
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 5, "Auto: a row per colour");
    assert!(r.app().session.in_interaction(), "the dialog previews");
    // Two colours, Exact; OK without another frame still reduces first.
    r.set("colors", json!(2)).unwrap();
    r.set("method", json!("exact")).unwrap();
    let undo = undo_len(&r.app());
    r.confirm().unwrap();
    assert!(!r.is_open());
    assert_eq!(colors(&mut r.app()), 2);
    assert_eq!(undo_len(&r.app()), undo + 1);
    r.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(colors(&mut r.app()), 5);
    // Cancel rolls the preview back.
    r.app().run("ui.recolorDialog", json!({"colors": 1})).unwrap();
    r.set("method", json!("exact")).unwrap();
    r.settled();
    assert_eq!(colors(&mut r.app()), 1, "previewed");
    r.cancel();
    assert_eq!(colors(&mut r.app()), 5);
}

/// Enter in the field that has the focus is OK: the color group is made with the name typed, and the dialog closes.
#[test]
fn enter_in_a_field_is_ok() {
    let mut r = open(app(), json!({"colors": ["#ff8000", "#0080ff"]}));
    r.settled();
    r.set("groupName", json!("Enter Group")).unwrap();
    r.settled();
    r.click_label("Color Group:");
    r.settled();
    assert!(r.focused().is_some(), "a field of the dialog has the focus");
    let groups = r.app().session.doc().unwrap().doc.swatch_groups.len();
    key(&mut r, Key::Enter);
    assert!(!r.is_open(), "Enter closes Recolor Artwork");
    let app = r.app();
    let doc = &app.session.doc().unwrap().doc;
    assert_eq!(doc.swatch_groups.len(), groups + 1, "Enter makes the color group, as OK does");
    assert_eq!(doc.swatch_groups.last().map(|g| g.name.as_str()), Some("Enter Group"));
}

#[test]
fn presets_open_colour_jobs_and_libraries_and_both_tabs_draw() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff"]);
    let presets: Vec<(String, Value)> = crate::menus::menu_entries(&app)
        .iter()
        .filter(|e| e.path.last().is_some_and(|p| p == "Recolor with Preset"))
        .map(|e| (e.label.clone(), e.params.clone()))
        .collect();
    assert_eq!(presets.iter().map(|p| p.0.as_str()).collect::<Vec<_>>(), ["1 Color Job…", "2 Color Job…", "3 Color Job…", "Color Library…"]);
    let mut r = open(app, presets[1].1.clone());
    confirm_preset(&mut r).unwrap();
    r.settled();
    assert_eq!((field(&r, "colors"), field(&r, "method")), (json!(2), json!("scaleTints")));
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 2);
    r.cancel();
    r.app().run("ui.recolorDialog", presets[3].1.clone()).unwrap();
    assert_eq!(preset_field(&r, "limitTo"), "web-safe-216", "Color Library asks with the first library chosen");
    confirm_preset(&mut r).unwrap();
    assert_eq!(field(&r, "limitTo"), "web-safe-216", "Color Library starts on the first library");
    r.settled();
    let web = |k: &Value| vectorcraft_engine::cmd::color_value(k).unwrap().to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3);
    assert!(field(&r, "rows").as_array().unwrap().iter().all(|row| web(&row["to"])));
    r.cancel();
    // The Color Guide's colours become the new colours.
    r.app().run("ui.recolorDialog", json!({"colors": ["#123456", "#abcdef"]})).unwrap();
    r.settled();
    let tos: Vec<Value> = field(&r, "rows").as_array().unwrap().iter().map(|row| row["to"].clone()).collect();
    assert_eq!(tos, [json!("rgb 18 52 86"), json!("rgb 171 205 239")]);
    r.cancel();
}

#[test]
fn double_clicking_a_colour_group_opens_it_and_ok_rewrites_it() {
    let app = app();
    let mut r = open(app, json!({"group": "Brights"}));
    assert_eq!(field(&r, "group"), "Brights");
    let keys = group_keys(&r.app(), "Brights");
    let from: Vec<Value> = field(&r, "rows").as_array().unwrap().iter().map(|row| row["from"][0].clone()).collect();
    assert_eq!(from, keys, "a row per group colour, in order");
    r.settled();
    // A new first colour and a new name; nothing is selected, so only the group changes.
    let mut rows = field(&r, "rows");
    rows[0]["to"] = json!("cmyk 100 0 0 0");
    r.set("rows", rows).unwrap();
    r.set("groupName", json!("Brights 2026")).unwrap();
    let undo = undo_len(&r.app());
    r.confirm().unwrap();
    let mut want = keys.clone();
    want[0] = json!("cmyk 100 0 0 0");
    assert_eq!(group_keys(&r.app(), "Brights 2026"), want);
    assert_eq!(undo_len(&r.app()), undo + 1, "one undo step");
}

#[test]
fn the_options_button_edits_a_selected_group_and_applies_it_to_the_art() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff", "#ffff00", "#ff00ff"]);
    let mut r = open(app, json!({"group": "Brights"}));
    assert_eq!(field(&r, "group"), "Brights");
    r.settled();
    let group = group_keys(&r.app(), "Brights");
    let tos: Vec<Value> = field(&r, "rows").as_array().unwrap().iter().map(|row| row["to"].clone()).collect();
    assert_eq!(tos, group, "the art's colours in as many rows as the group has, taking its colours");
    r.confirm().unwrap();
    let used: Vec<String> = r.app().session.execute("recolor.colors", &json!({})).unwrap()["colors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["key"].as_str().unwrap().to_string())
        .collect();
    assert!(used.iter().all(|k| group.contains(&json!(k))), "the art uses the group's colours: {used:?}");
    assert!(matches!(ColorKey::parse(&used[0]).unwrap().color(), Color::Rgb { .. }));
}

#[test]
fn a_group_keeps_the_colours_no_art_row_took() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let before = group_keys(&app, "Brights");
    let mut r = open(app, json!({"group": "Brights"}));
    r.settled();
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 2, "two art colours");
    let mut rows = field(&r, "rows");
    rows[1]["to"] = json!("rgb 1 2 3");
    r.set("rows", rows).unwrap();
    r.confirm().unwrap();
    let after = group_keys(&r.app(), "Brights");
    assert_eq!(after.len(), before.len());
    assert_eq!((&after[0], &after[1]), (&before[0], &json!("rgb 1 2 3")));
    assert_eq!(after[2..], before[2..]);
}

/// The fill of the first object of the document.
fn first_fill(app: &VectorcraftApp) -> vectorcraft_color::Paint {
    fill_of(app, 0)
}

/// The color mode menu's sliders edit the first selected row's new color in the mode picked (the
/// other row keeps its color), the art previews it, and OK gives the art the color in that mode:
/// a CMYK color on RGB art makes it CMYK. Cancel leaves the art as it was, with no undo step.
#[test]
fn the_color_sliders_edit_the_selected_rows_new_color_in_their_mode() {
    for confirm in [true, false] {
        let mut app = app();
        art(&mut app, &["#0000ff", "#ff0000"]);
        let before = first_fill(&app);
        let undo = undo_len(&app);
        let mut r = assign(app, json!({}));
        let rows = field(&r, "rows");
        let blue = 0;
        assert_eq!(rows[blue]["from"], json!(["rgb 0 0 255"]), "the first object's blue is the first row");
        // Both rows selected, the second one first: the sliders edit the first row.
        put(&mut r, "__sel", json!([1, blue]));
        assert!(has(&r, "R"), "RGB art shows the RGB sliders");
        put(&mut r, "sliderMode", json!("cmyk"));
        rail(&mut r, "C", 0.5);
        let edited = field(&r, "rows");
        let to = edited[blue]["to"].as_str().unwrap().to_string();
        assert!(to.starts_with("cmyk 50 "), "the first selected row takes C 50%: {to}");
        assert_eq!(edited[1], rows[1], "the other selected row keeps its new color");
        let previewed = first_fill(&r.app());
        let Some(Color::Cmyk { c, .. }) = previewed.color() else { panic!("the art previews the CMYK color: {previewed:?}") };
        assert!((c - 0.5).abs() < 1e-3, "{c}");
        tap(&mut r, if confirm { "OK" } else { "Cancel" });
        assert!(!r.is_open());
        let app = r.app();
        if confirm {
            assert_eq!(first_fill(&app).color(), vectorcraft_engine::cmd::color_value(&json!(to)), "OK keeps the color, in CMYK");
            assert_eq!(undo_len(&app), undo + 1, "one undo step");
        } else {
            assert_eq!(first_fill(&app), before, "Cancel leaves the art unchanged");
            assert_eq!(undo_len(&app), undo, "and records no undo step");
        }
    }
}

/// A gray row shows HSB, and the sliders stay in HSB once an edit makes the color RGB.
#[test]
fn a_gray_row_keeps_the_hsb_sliders_while_it_is_edited() {
    let mut app = app();
    art(&mut app, &["gray 40"]);
    let mut r = assign(app, json!({}));
    put(&mut r, "__sel", json!([0]));
    assert!(has(&r, "H"), "a gray row shows HSB");
    rail(&mut r, "S", 0.5);
    let to = field(&r, "rows")[0]["to"].clone();
    assert!(to.as_str().is_some_and(|k| k.starts_with("rgb ")), "{to}");
    assert!(has(&r, "H"), "still HSB after the edit");
    assert_eq!(field(&r, "sliderMode"), json!("hsb"));
}

/// With Limit to Library, a row whose new color is a spot color of the library shows Tint, whose T
/// slider sets the row's tint of it; OK links the art to the spot color at that tint.
#[test]
fn the_tint_slider_sets_a_spot_rows_tint() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})).unwrap();
    let data = app.run("swatch.library.save", json!({"names": ["Ink"], "name": "Inks"})).unwrap()["data"].as_str().unwrap().to_string();
    let lib = app.run("swatch.library.load", json!({"data": data, "name": "inks.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
    app.run("swatch.delete", json!({"names": ["Ink"]})).unwrap();
    art(&mut app, &["cmyk 0 100 0 0"]);
    let mut r = assign(app, json!({"library": lib}));
    put(&mut r, "__sel", json!([0]));
    assert!(has(&r, "Tint"), "a spot row shows Tint");
    rail(&mut r, "Tint", 0.4);
    assert_eq!(field(&r, "rows")[0]["tint"], json!(40.0));
    tap(&mut r, "OK");
    match first_fill(&r.app()) {
        vectorcraft_color::Paint::Solid { swatch, tint, .. } => {
            assert_eq!(swatch.as_deref(), Some("Ink"), "linked to the spot color");
            assert!((tint - 0.4).abs() < 1e-4, "at the row's tint: {tint}");
        }
        p => panic!("{p:?}"),
    }
}

/// A spot row's tint goes when the Edit tab gives the row another new color: after a click on the
/// Brightness slider snaps the row to a second spot color, OK links the art to that one at full
/// strength. A click that snaps back to the same spot color keeps the tint.
#[test]
fn a_new_color_from_the_edit_tab_drops_a_spot_rows_tint() {
    let mut app = app();
    let lib = library_of(
        &mut app,
        &[json!({"name": "Mix Ink", "color": "#ff00ff", "spot": true}), json!({"name": "Mix Dark", "color": "#800080", "spot": true})],
    );
    art(&mut app, &["#ff00ff"]);
    let mut r = assign(app, json!({"library": lib}));
    let mut rows = field(&r, "rows");
    rows[0]["tint"] = json!(40.0);
    r.set("rows", rows).unwrap();
    put(&mut r, "tab", json!("edit"));
    rail(&mut r, "Brightness", 0.9);
    let row = field(&r, "rows")[0].clone();
    assert_eq!((&row["to"], &row["tint"]), (&json!("rgb 255 0 255"), &json!(40.0)), "snapped back to the same spot color");
    rail(&mut r, "Brightness", 0.4);
    let row = field(&r, "rows")[0].clone();
    assert_eq!(row["to"], json!("rgb 128 0 128"), "snapped to the darker spot color");
    assert!(row.get("tint").is_none(), "the row's tint went with its old color: {row}");
    tap(&mut r, "OK");
    match first_fill(&r.app()) {
        vectorcraft_color::Paint::Solid { swatch, tint, .. } => {
            assert_eq!(swatch.as_deref(), Some("Mix Dark"), "linked to the new spot color");
            assert!((tint - 1.0).abs() < 1e-4, "at full strength: {tint}");
        }
        p => panic!("{p:?}"),
    }
}

/// Randomize order moves a tint with its color, also between rows of the same new color: after
/// each click exactly one row is tinted, and some click moves the tint to the other row.
#[test]
fn randomize_order_moves_a_tint_between_rows_of_the_same_color() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let mut r = assign(app, json!({}));
    let mut rows = field(&r, "rows");
    for row in rows.as_array_mut().unwrap() {
        row["to"] = json!("rgb 0 255 0");
    }
    rows[0]["tint"] = json!(40.0);
    put(&mut r, "rows", rows);
    let mut moved = false;
    for _ in 0..8 {
        tap(&mut r, "Randomly change color order");
        let rows = field(&r, "rows");
        let tinted: Vec<usize> = (0..2).filter(|i| rows[*i].get("tint").is_some()).collect();
        assert_eq!(tinted.len(), 1, "one tinted row: {rows}");
        moved |= tinted == [1];
    }
    assert!(moved, "some click moves the tint to the other row");
}

/// Makes the swatches of the `swatch.new` params `swatches`, saves them as a library and loads it,
/// removes them from the document again, and returns the library's id.
fn library_of(app: &mut VectorcraftApp, swatches: &[Value]) -> String {
    let names: Vec<Value> = swatches.iter().map(|s| s["name"].clone()).collect();
    for s in swatches {
        app.run("swatch.new", s.clone()).unwrap();
    }
    let data = app.run("swatch.library.save", json!({"names": names, "name": "Lib"})).unwrap()["data"].as_str().unwrap().to_string();
    let lib = app.run("swatch.library.load", json!({"data": data, "name": "lib.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
    app.run("swatch.delete", json!({"names": names})).unwrap();
    lib
}

/// When the library's first swatch of a row's new color is a process color, the row shows no Tint,
/// even with a spot color of that color after it: `recolor.apply` gives the art the first one.
#[test]
fn a_process_swatch_before_a_spot_one_of_its_color_shows_no_tint() {
    let mut app = app();
    let lib = library_of(
        &mut app,
        &[
            json!({"name": "Mix Process", "color": {"c": 0, "m": 1, "y": 0, "k": 0}}),
            json!({"name": "Mix Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true}),
        ],
    );
    art(&mut app, &["cmyk 0 100 0 0"]);
    let mut r = assign(app, json!({"library": lib}));
    put(&mut r, "__sel", json!([0]));
    assert!(!has(&r, "Tint"), "no Tint for a process swatch");
    assert!(has(&r, "C"), "the CMYK sliders");
}

/// Opened on a color group without art, a row whose new color is a spot color of the library shows
/// the process modes and no Tint: OK saves only the group's colors, so a tint would be lost.
#[test]
fn a_spot_row_of_a_group_without_art_shows_no_tint() {
    let mut app = app();
    let lib = library_of(&mut app, &[json!({"name": "Mix Ink", "color": {"c": 0, "m": 1, "y": 0, "k": 0}, "spot": true})]);
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0"], "keepModel": true})).unwrap();
    let mut r = assign(app, json!({"group": "Mix", "library": lib}));
    assert_ne!(field(&r, "__art"), json!(true), "no art");
    put(&mut r, "__sel", json!([0]));
    assert!(!has(&r, "Tint"), "no Tint without art");
    assert!(has(&r, "C"), "the CMYK sliders");
}

/// A spot color whose components are finer than a row's color key still shows Tint: the row's
/// rounded color snaps back to it, as `recolor.apply` snaps it.
#[test]
fn a_spot_color_finer_than_the_rows_key_shows_tint() {
    let mut app = app();
    let lib = library_of(&mut app, &[json!({"name": "Mix Fine", "color": {"c": 0, "m": 0.12345, "y": 0, "k": 0}, "spot": true})]);
    art(&mut app, &["cmyk 0 12 0 0"]);
    let mut r = assign(app, json!({"library": lib}));
    put(&mut r, "__sel", json!([0]));
    assert!(has(&r, "Tint"), "the spot row shows Tint");
}

/// Reset with the preview on reads the art's own colors again and discards a slider edit: OK then
/// gives RGB art a CMYK color group's color in RGB, as it does without the edit.
#[test]
fn reset_after_a_slider_edit_keeps_the_arts_model() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff"]);
    let mut r = assign(app, json!({"group": "Mix"}));
    put(&mut r, "__sel", json!([0]));
    rail(&mut r, "C", 0.5);
    assert_eq!(field(&r, "rows")[0]["model"], json!("new"), "the sliders edited the row");
    tap(&mut r, "Reset the rows to the artwork's colors");
    assert_eq!(field(&r, "rows")[0]["from"], json!(["rgb 255 0 255"]), "Reset reads the art's colors, not the preview's");
    tap(&mut r, "OK");
    assert!(!r.is_open());
    assert!(matches!(first_fill(&r.app()).color(), Some(Color::Rgb { .. })), "the art stays RGB: {:?}", first_fill(&r.app()));
}

/// A change of the color count rebuilds the rows and discards a slider edit: OK then gives RGB art
/// a CMYK color group's color in RGB, as it does without the edit.
#[test]
fn a_new_color_count_after_a_slider_edit_keeps_the_arts_model() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0", "cmyk 100 0 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff", "#00ffff"]);
    let mut r = assign(app, json!({"group": "Mix"}));
    put(&mut r, "__sel", json!([0]));
    rail(&mut r, "C", 0.5);
    assert_eq!(field(&r, "rows")[0]["model"], json!("new"), "the sliders edited the row");
    put(&mut r, "colors", json!(1));
    assert_eq!(field(&r, "rows").as_array().map(Vec::len), Some(1), "the rows are rebuilt");
    assert_eq!(field(&r, "rows")[0].get("model"), None, "the slider edit is discarded");
    tap(&mut r, "OK");
    assert!(!r.is_open());
    assert!(matches!(first_fill(&r.app()).color(), Some(Color::Rgb { .. })), "the art stays RGB: {:?}", first_fill(&r.app()));
}

/// The fill of the `n`th object of the document.
fn fill_of(app: &VectorcraftApp, n: usize) -> vectorcraft_color::Paint {
    let d = &app.session.doc().unwrap().doc;
    let id = d.layers[0].children().unwrap()[n].id;
    d.node(id).unwrap().appearance.fill_paint()
}

/// A slider edit of one row gives only that row's art the slider's mode: with RGB art and a CMYK
/// color group, OK makes the edited row's art CMYK and leaves the other row's art RGB.
#[test]
fn a_slider_edit_changes_the_model_of_its_own_row_only() {
    let mut app = app();
    app.run("swatch.newGroup", json!({"name": "Mix", "colors": ["cmyk 0 100 0 0", "cmyk 100 0 0 0"], "keepModel": true})).unwrap();
    art(&mut app, &["#ff00ff", "#00ffff"]);
    let mut r = assign(app, json!({"group": "Mix"}));
    let rows = field(&r, "rows");
    let magenta = (0..2).find(|i| rows[*i]["from"] == json!(["rgb 255 0 255"])).expect("a row of the magenta art");
    put(&mut r, "__sel", json!([magenta]));
    rail(&mut r, "C", 0.5);
    let rows = field(&r, "rows");
    assert_eq!(rows[magenta]["model"], json!("new"), "the edited row: {rows}");
    assert_eq!(rows[1 - magenta].get("model"), None, "the other row: {rows}");
    tap(&mut r, "OK");
    assert!(!r.is_open());
    assert!(matches!(fill_of(&r.app(), 0).color(), Some(Color::Cmyk { .. })), "the edited row's art is CMYK: {:?}", fill_of(&r.app(), 0));
    assert!(matches!(fill_of(&r.app(), 1).color(), Some(Color::Rgb { .. })), "the other row's art stays RGB: {:?}", fill_of(&r.app(), 1));
}

/// The fills of the document's objects, in order.
fn fills(app: &VectorcraftApp) -> Vec<Option<Color>> {
    let d = &app.session.doc().unwrap().doc;
    d.layers[0].children().unwrap().iter().map(|n| d.node(n.id).unwrap().appearance.fill_paint().color()).collect()
}

/// The Edit tab's color mode menu offers Global Adjust: Saturation, Brightness, Temperature and
/// Luminosity open at 0%, which changes nothing; one slider changes every recolored color at once
/// (the rows keep their new colors) and the art previews it; back at 0% the art has its own colors
/// again; the amounts stay while the tab or the mode changes; OK keeps the change as one undo step
/// and Cancel leaves the art as it was.
#[test]
fn global_adjust_changes_every_color_at_once() {
    for confirm in [true, false] {
        let mut r = edit(&["#ff0000", "#0000ff", "#00ff00"]);
        let before = fills(&r.app());
        let undo = undo_len(&r.app());
        put(&mut r, "sliderMode", json!("global"));
        for name in ["Saturation", "Brightness", "Temperature", "Luminosity"] {
            assert!(has(&r, name), "{name}");
        }
        assert_eq!(fills(&r.app()), before, "at 0% the art keeps its colors");
        let rows = field(&r, "rows");
        rail(&mut r, "Saturation", 0.0);
        assert_eq!(field(&r, "globalAdjust")["saturation"], json!(-100.0));
        assert_eq!(field(&r, "rows"), rows, "the rows keep their new colors");
        let gray = |app: &VectorcraftApp| fills(app).iter().all(|c| c.is_some_and(|c| c.to_hsb()[1] < 1e-3));
        assert!(gray(&r.app()), "every color previews gray: {:?}", fills(&r.app()));
        rail(&mut r, "Saturation", 0.5);
        assert_eq!(field(&r, "globalAdjust")["saturation"], json!(0.0));
        assert_eq!(fills(&r.app()), before, "back at 0% the art has its own colors");
        rail(&mut r, "Saturation", 0.0);
        for (k, v) in [("tab", json!("assign")), ("tab", json!("edit")), ("sliderMode", json!("hsb")), ("sliderMode", json!("global"))] {
            put(&mut r, k, v);
        }
        assert_eq!(field(&r, "globalAdjust")["saturation"], json!(-100.0), "the amount stays");
        assert!(gray(&r.app()), "and still applies");
        tap(&mut r, if confirm { "OK" } else { "Cancel" });
        assert!(!r.is_open());
        let app = r.app();
        if confirm {
            assert!(gray(&app), "OK keeps the change: {:?}", fills(&app));
            assert_eq!(undo_len(&app), undo + 1, "one undo step");
        } else {
            assert_eq!(fills(&app), before, "Cancel leaves the art unchanged");
            assert_eq!(undo_len(&app), undo, "and records no undo step");
        }
    }
}

/// Colors opened alone with Limit to Library: OK saves the new colors with the Global Adjust as a
/// new color group, each snapped to the library again, as `recolor.apply` gives them to a group.
#[test]
fn global_adjust_on_a_new_group_keeps_its_colors_in_the_library() {
    let mut r = open(app(), json!({"colors": ["#123456", "#abcdef"], "library": ""}));
    assert_eq!(field(&r, "limitTo"), "web-safe-216");
    let web = |k: &Value| vectorcraft_engine::cmd::color_value(k).unwrap().to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3);
    let rows: Vec<Value> = field(&r, "rows").as_array().unwrap().iter().map(|row| row["to"].clone()).collect();
    assert!(rows.iter().all(web), "the rows open snapped: {rows:?}");
    r.set("globalAdjust", json!({"brightness": -60})).unwrap();
    r.confirm().unwrap();
    let keys = group_keys(&r.app(), "Color Group");
    assert_eq!(keys.len(), 2);
    assert_ne!(keys, rows, "the Global Adjust changed the colors");
    assert!(keys.iter().all(web), "every group color is a library color: {keys:?}");
}

/// #2960: the Preserve checkboxes open as OK last left them; Cancel keeps what OK left.
#[test]
fn the_preserve_checkboxes_open_as_they_were_last_confirmed() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#ffffff", "#000000", "#808080"]);
    let preserve = |r: &Recolor| ["preserveWhite", "preserveBlack", "preserveGrays"].map(|k| field(r, k));
    let mut r = open(app, json!({}));
    assert_eq!(preserve(&r), [json!(true), json!(true), json!(false)], "a first open");
    r.set("preserveWhite", json!(false)).unwrap();
    r.set("preserveBlack", json!(false)).unwrap();
    r.set("preserveGrays", json!(true)).unwrap();
    r.confirm().unwrap();
    r.app().run("select.all", json!({})).unwrap();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(preserve(&r), [json!(false), json!(false), json!(true)], "as confirmed");
    r.set("preserveWhite", json!(true)).unwrap();
    r.cancel();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(preserve(&r), [json!(false), json!(false), json!(true)], "Cancel keeps what OK left");
}

#[test]
fn with_every_color_preserved_the_colors_table_says_so_in_its_body() {
    let mut app = app();
    art(&mut app, &["#ffffff", "#000000"]);
    let mut r = assign(app, json!({}));
    assert!(r.rows().is_empty(), "White and Black are preserved");
    let note = "No colors to recolor (preserved colors are left out).";
    let inside = |r: &Recolor| rect(r, "Colors").contains_rect(rect(r, note));
    assert!(inside(&r), "the note is drawn inside the Colors table, not above its empty rows");
    r.frame();
    assert!(inside(&r), "and stays there on the next frame");
}

fn new_hsb(r: &Recolor) -> Vec<[f32; 3]> {
    field(r, "rows").as_array().unwrap().iter().map(|row| vectorcraft_engine::cmd::color_value(&row["to"]).unwrap().to_hsb()).collect()
}

/// #1733: on the Edit tab, dragging a handle of CMYK art (linked, Complementary) must leave every
/// handle on the wheel. The others turn with it (intended), but they keep their place relative to
/// it: they don't drift while the pointer is held still and don't end up under the dragged one.
#[test]
fn dragging_a_wheel_handle_keeps_the_other_handles_apart() {
    let mut r = edit(&["cmyk 0 100 100 0", "cmyk 100 0 0 0"]);
    let (c, radius) = wheel(&r);
    // Each row's new colour: its hue, and where the wheel puts it, worked out from the rows because mid-drag the
    // markers' names lag one move behind (the wheel draws before the pass's move applies, as production's did).
    let handles = |r: &Recolor| -> Vec<(f32, Pos2)> { new_hsb(r).iter().map(|hsb| (hsb[0], polar(c, hsb[0], radius * hsb[1]))).collect() };
    let gap = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0));
    let start = handles(&r);
    // Drag the red handle a quarter turn, a few points at a time, from where its marker is drawn.
    let from = marker(&r, 0);
    let s0 = from.distance(c) / radius;
    let mut path = vec![from];
    path.extend((1..=12).map(|k| polar(c, start[0].0 + 6.0 * k as f32, radius * (s0 + (0.75 - s0) * k as f32 / 12.0))));
    drag_path(&mut r, &path);
    let p = path[path.len() - 1];
    let moved = handles(&r);
    assert!(gap(moved[1].0, start[1].0) > 10.0, "linked: the other handle turned with it (intended): {start:?} -> {moved:?}");
    let brightness = |r: &Recolor| -> Vec<f32> { new_hsb(r).iter().map(|hsb| hsb[2]).collect() };
    let moved_v = brightness(&r);
    // Hold the pointer still: nothing should change.
    for _ in 0..20 {
        r.frame();
    }
    let held = handles(&r);
    let held_v = brightness(&r);
    assert!(
        moved_v.iter().zip(&held_v).all(|(a, b)| (a - b).abs() < 0.01),
        "the brightness changed while the pointer was held still: {moved_v:?} -> {held_v:?}"
    );
    assert!(gap(held[1].0, moved[1].0) < 1.0, "the other handle turned on its own while the pointer was held still: {moved:?} -> {held:?}");
    assert!(held[0].1.distance(held[1].1) > 18.0, "the other handle went under the dragged one (pointer {p:?}): {held:?}");
    let rows = field(&r, "rows");
    let_go(&mut r, p);
    assert_eq!(field(&r, "rows"), rows, "releasing keeps the colours the drag ended on");
    // OK keeps the whole drag as one undo step; undo brings the art's colours back.
    let keys = |app: &mut VectorcraftApp| -> Vec<Value> {
        let mut k: Vec<Value> =
            app.session.execute("recolor.colors", &json!({})).unwrap()["colors"].as_array().unwrap().iter().map(|c| c["key"].clone()).collect();
        k.sort_by_key(Value::to_string);
        k
    };
    let undo = undo_len(&r.app());
    r.confirm().unwrap();
    assert_eq!(undo_len(&r.app()), undo + 1);
    let mut to: Vec<Value> = rows.as_array().unwrap().iter().map(|row| row["to"].clone()).collect();
    to.sort_by_key(Value::to_string);
    assert_eq!(keys(&mut r.app()), to);
    r.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(keys(&mut r.app()), [json!("cmyk 0 100 100 0"), json!("cmyk 100 0 0 0")]);
}

/// Preview off restores the artwork and on restores the edit; Cancel leaves the undo count and OK
/// is one undo step that restores all the art of a full history.
#[test]
fn the_last_edit_color_can_be_selected_edited_and_undone() {
    for confirm in [false, true] {
        let mut r = edit(&["#ff0000", "#ffff00", "#00ff00", "#0000ff", "#ff00ff"]);
        let before = snapshot(&r.app());
        let undo = undo_len(&r.app());
        put(&mut r, "__sel", json!([4]));
        put(&mut r, "linked", json!(false));
        let rows = field(&r, "rows");
        rail(&mut r, "Brightness", 0.25);
        let edited = field(&r, "rows");
        assert_ne!(edited[4]["to"], rows[4]["to"], "the last color changed");
        assert_eq!(&edited.as_array().unwrap()[..4], &rows.as_array().unwrap()[..4], "unlinked editing preserves the other colors");
        let after = snapshot(&r.app());
        assert_ne!(after, before, "the edit previews on the artwork");
        tap(&mut r, "Preview");
        assert_eq!(snapshot(&r.app()), before, "Preview off restores the artwork");
        tap(&mut r, "Preview");
        assert_eq!(snapshot(&r.app()), after, "Preview on restores the edit");
        tap(&mut r, if confirm { "OK" } else { "Cancel" });
        assert!(!r.is_open());
        if confirm {
            assert_eq!(snapshot(&r.app()), after);
            assert_eq!(undo_len(&r.app()), undo + 1, "one undo step");
            r.app().run("edit.undo", json!({})).unwrap();
        } else {
            assert_eq!(undo_len(&r.app()), undo);
        }
        assert_eq!(snapshot(&r.app()), before);
    }
}

/// With linked colors a drag on the segmented wheel turns the others from where it started, and an
/// excluded row keeps its color; the smooth wheel shows the result. Its Brightness slider works too.
#[test]
fn a_segmented_wheel_marker_moves_freely() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200", "rgb 20 160 20"]);
    let mut rows = field(&r, "rows");
    let green = (0..3).find(|i| rows[*i]["from"] == json!(["rgb 20 160 20"])).unwrap();
    let red = (0..3).find(|i| rows[*i]["from"] == json!(["rgb 200 100 100"])).unwrap();
    rows[green]["exclude"] = json!(true);
    r.set("rows", rows).unwrap();
    put(&mut r, "__sel", json!([red]));
    tap(&mut r, "Show segmented color wheel");
    let start = new_hsb(&r);
    let (c, _) = wheel(&r);
    let from = marker(&r, red);
    let d = from.distance(c);
    let hue = start[red][0];
    // 37° on in small steps.
    let mut path = vec![from];
    path.extend((1..=10).map(|k| polar(c, hue + 3.7 * k as f32, d)));
    drag_path(&mut r, &path);
    let_go(&mut r, path[path.len() - 1]);
    let moved = new_hsb(&r);
    let gap = |a: f32, b: f32| (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0));
    let other = 3 - red - green;
    assert!((gap(moved[other][0], start[other][0]) - 37.0).abs() < 1.5, "linked: {:?} -> {:?}", start[other], moved[other]);
    assert_eq!(field(&r, "rows")[green]["to"], json!("rgb 20 160 20"), "the excluded row stays");
    let rows = field(&r, "rows");
    tap(&mut r, "Show smooth color wheel");
    assert_eq!(field(&r, "rows"), rows, "the smooth wheel shows the same colors");
    tap(&mut r, "Show segmented color wheel");
    rail(&mut r, "Brightness", 0.25);
    assert!((new_hsb(&r)[red][2] - 0.25).abs() < 0.02, "the Brightness slider: {:?}", new_hsb(&r));
}

/// The button under the wheel tools switches the wheel to hue and brightness: its name says what a
/// click shows next, the slider under the wheel becomes a Saturation slider, and switching changes
/// no color, previews nothing and records no undo step. The choice holds for the segmented wheel and
/// across the color bars, where the button is off; the next opening shows hue and saturation again.
#[test]
fn the_brightness_and_hue_wheel_swaps_the_wheel_axis_and_the_slider() {
    let mut r = edit(&["#ff0000", "#0000ff", "#00ff00"]);
    let rows = field(&r, "rows");
    let doc = snapshot(&r.app());
    let undo = undo_len(&r.app());
    assert_eq!(field(&r, "wheelAxes"), Value::Null, "hue and saturation first");
    assert!(has(&r, "Brightness") && !has(&r, "Saturation"));
    tap(&mut r, "Show brightness and hue on wheel");
    assert_eq!(field(&r, "wheelAxes"), json!("hueBrightness"));
    assert!(has(&r, "Saturation") && !has(&r, "Brightness"), "the slider edits the component the wheel leaves out");
    assert!(has(&r, "Show saturation and hue on wheel"), "the button names the way back");
    assert_eq!(field(&r, "rows"), rows, "switching changes no color");
    assert_eq!(snapshot(&r.app()), doc, "nor the art");
    assert_eq!(undo_len(&r.app()), undo);
    assert!(!steps(&r).0, "the axis is no step of Undo Changes");
    tap(&mut r, "Show segmented color wheel");
    assert_eq!(field(&r, "wheelAxes"), json!("hueBrightness"), "the segmented wheel keeps the choice");
    assert!(has(&r, "Saturation"));
    tap(&mut r, "Show color bars");
    assert!(!r.enabled("Show saturation and hue on wheel"), "the button is for the wheels");
    tap(&mut r, "Show smooth color wheel");
    assert!(has(&r, "Saturation"), "the bars keep the choice");
    tap(&mut r, "Show saturation and hue on wheel");
    assert_eq!(field(&r, "wheelAxes"), json!("hueSaturation"));
    assert!(has(&r, "Brightness") && !has(&r, "Saturation"));
    assert_eq!(field(&r, "rows"), rows);
    put(&mut r, "wheelAxes", json!("sideways"));
    assert!(has(&r, "Brightness"), "an unknown value shows hue and saturation");
    put(&mut r, "wheelAxes", json!("hueBrightness"));
    r.cancel();
    r.app().run("select.all", json!({})).unwrap();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(field(&r, "wheelAxes"), Value::Null, "a new opening shows hue and saturation");
}

/// On the hue and brightness wheel a marker sits at its color's brightness, and a drag sets hue and
/// brightness and keeps saturation; linked colors turn and darken with it. The art previews the
/// drag, Cancel restores it and OK keeps it as one undo step. The Saturation slider sets the
/// selected color's saturation and keeps its brightness.
#[test]
fn a_marker_on_the_brightness_wheel_sets_hue_and_brightness() {
    for confirm in [false, true] {
        let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
        put(&mut r, "__sel", json!([0]));
        let before = snapshot(&r.app());
        let undo = undo_len(&r.app());
        tap(&mut r, "Show brightness and hue on wheel");
        let start = new_hsb(&r);
        let [hue, s, v] = start[0];
        // Around by 30° and in to 40% brightness, a few points at a time from the marker.
        let (c, radius) = wheel(&r);
        let mut path = vec![marker(&r, 0)];
        path.extend((1..=10).map(|k| {
            let k = k as f32 / 10.0;
            polar(c, hue + 30.0 * k, radius * (v + (0.4 - v) * k))
        }));
        drag_path(&mut r, &path);
        let_go(&mut r, path[path.len() - 1]);
        let moved = new_hsb(&r);
        assert!(hue_gap(moved[0][0], hue + 30.0) < 1.0, "hue: {start:?} -> {moved:?}");
        assert!((moved[0][2] - 0.4).abs() < 0.02, "brightness: {start:?} -> {moved:?}");
        assert!((moved[0][1] - s).abs() < 0.02, "saturation stays: {start:?} -> {moved:?}");
        assert!(hue_gap(moved[1][0], start[1][0] + 30.0) < 1.5, "linked, it turns: {start:?} -> {moved:?}");
        assert!((moved[1][2] - (start[1][2] - (v - 0.4))).abs() < 0.02, "and darkens alike: {start:?} -> {moved:?}");
        rail(&mut r, "Saturation", 0.25);
        let slid = new_hsb(&r);
        assert!((slid[0][1] - 0.25).abs() < 0.02, "the Saturation slider: {slid:?}");
        assert!((slid[0][2] - 0.4).abs() < 0.02, "keeps the brightness: {slid:?}");
        let after = snapshot(&r.app());
        assert_ne!(after, before, "the art previews the edit");
        tap(&mut r, if confirm { "OK" } else { "Cancel" });
        assert!(!r.is_open());
        if confirm {
            assert_eq!(snapshot(&r.app()), after);
            assert_eq!(undo_len(&r.app()), undo + 1, "one undo step");
            r.app().run("edit.undo", json!({})).unwrap();
        } else {
            assert_eq!(undo_len(&r.app()), undo, "Cancel records no undo step");
        }
        assert_eq!(snapshot(&r.app()), before);
    }
}

/// On the hue and brightness wheels the Add Color tool adds the hue and brightness clicked at the
/// base color's saturation, the patch's color on the segmented wheel.
#[test]
fn the_add_color_tool_on_the_brightness_wheel_adds_the_brightness_clicked() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
    put(&mut r, "__sel", json!([0]));
    let s = new_hsb(&r)[0][1];
    tap(&mut r, "Show brightness and hue on wheel");
    tap(&mut r, "Add Color Tool");
    let at = wheel_at(&r, 120.0, 0.5);
    click(&mut r, at);
    let [hue, sat, b] = new_hsb(&r)[2];
    assert!((hue - 120.0).abs() < 1.0 && (sat - s).abs() < 0.01 && (b - 0.5).abs() < 0.01, "{:?}", [hue, sat, b]);
    assert_eq!(field(&r, "__sel"), json!([2]));
    // The new color is now the base color; select the first one again.
    put(&mut r, "__sel", json!([0]));
    tap(&mut r, "Show segmented color wheel");
    let at = wheel_at(&r, 187.0, 0.95);
    click(&mut r, at);
    let [hue, sat, b] = *new_hsb(&r).last().unwrap();
    assert!((hue - 180.0).abs() < 1.0 && (sat - s).abs() < 0.01 && (b - 0.9).abs() < 0.01, "the patch's color: {:?}", [hue, sat, b]);
}

/// The rows as `(current colors, new color)` pairs, sorted.
fn row_pairs(r: &Recolor) -> Vec<String> {
    let mut v: Vec<String> = field(r, "rows").as_array().unwrap().iter().map(|row| format!("{} {}", row["from"], row["to"])).collect();
    v.sort();
    v
}

/// The color bars: one per recolored row (none for an excluded row), the last one selected with a
/// click and edited with the H slider, then dragged to the top, which moves its row and changes no
/// color. Preview shows the edit, Cancel restores the art and OK keeps it as one undo step.
#[test]
fn color_bars_select_edit_and_reorder_the_rows() {
    for confirm in [false, true] {
        let mut r = edit(&["#ff0000", "#0000ff", "#00ff00", "#ffff00"]);
        let before = snapshot(&r.app());
        let undo = undo_len(&r.app());
        let mut rows = field(&r, "rows");
        rows[1]["exclude"] = json!(true);
        r.set("rows", rows).unwrap();
        r.set("linked", json!(false)).unwrap();
        r.set("sliderMode", json!("hsb")).unwrap();
        tap(&mut r, "Show color bars");
        assert!(!has(&r, "Color 2"), "no bar for the excluded row");
        let first = rect(&r, "Color 1");
        let last = rect(&r, "Color 4");
        // The List's rows touch: a row below another starts where the one above ends.
        assert!(r.by_label("Color 3").is_some_and(|t| t.rect.top() >= first.bottom() && t.rect.bottom() <= last.top()), "in row order");
        click(&mut r, last.center());
        assert_eq!(field(&r, "__sel"), json!([3]), "a click selects the bar's row");
        let unedited = field(&r, "rows");
        rail(&mut r, "H", 0.5);
        let edited = field(&r, "rows");
        assert_ne!(edited[3]["to"], unedited[3]["to"], "the H slider edits the selected bar's color");
        assert_eq!(&edited.as_array().unwrap()[..3], &unedited.as_array().unwrap()[..3], "and only that one");
        let previewed = snapshot(&r.app());
        assert_ne!(previewed, before, "the art previews the edit");
        let pairs = row_pairs(&r);
        let last = rect(&r, "Color 4");
        let top = pos2(last.center().x, first.top() + 2.0);
        drag_path(&mut r, &line(last.center(), top, 8));
        let_go(&mut r, top);
        let rows = field(&r, "rows");
        assert_eq!(rows[0], edited[3], "the dragged row moved to the top: {rows}");
        assert_eq!(rows.as_array().unwrap()[1..], [edited[0].clone(), edited[1].clone(), edited[2].clone()], "the others shift down");
        assert_eq!(row_pairs(&r), pairs, "moving a bar changes no color");
        assert_eq!(field(&r, "__sel"), json!([0]), "the moved row stays selected");
        assert_eq!(snapshot(&r.app()), previewed, "and the art keeps the same colors");
        tap(&mut r, if confirm { "OK" } else { "Cancel" });
        assert!(!r.is_open());
        let app = r.app();
        if confirm {
            assert_eq!(snapshot(&app), previewed, "OK keeps the edit");
            assert_eq!(undo_len(&app), undo + 1, "as one undo step");
        } else {
            assert_eq!(snapshot(&app), before, "Cancel restores the art");
            assert_eq!(undo_len(&app), undo);
        }
    }
}

/// The Add Color tool: a click on the smooth wheel adds the hue and saturation under it, at the
/// base color's brightness, as a row with no current colors that becomes the selected row; the art
/// keeps its colors, and the Assign tab shows the row without current colors. A second click on the
/// button turns the tool off, and a click on the wheel then adds nothing.
#[test]
fn the_add_color_tool_adds_a_row_with_no_current_colors() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
    put(&mut r, "__sel", json!([0]));
    let fills_before = fills(&r.app());
    let v = new_hsb(&r)[0][2];
    tap(&mut r, "Add Color Tool");
    assert_eq!(field(&r, "wheelTool"), json!("add"));
    let at = wheel_at(&r, 120.0, 0.5);
    click(&mut r, at);
    let rows = field(&r, "rows");
    assert_eq!(rows.as_array().unwrap().len(), 3, "a new row: {rows}");
    assert_eq!(rows[2]["from"], json!([]), "with no current colors");
    assert_eq!(rows[2]["added"], json!(true), "marked as an added color");
    let [hue, s, b] = new_hsb(&r)[2];
    assert!((hue - 120.0).abs() < 1.0 && (s - 0.5).abs() < 0.01 && (b - v).abs() < 0.01, "{:?}", [hue, s, b]);
    assert_eq!(field(&r, "__sel"), json!([2]), "the new color is selected");
    assert_eq!(fills(&r.app()), fills_before, "a color with no current colors recolors no art");
    // A second click adds another color; the tool stays on.
    let second = wheel_at(&r, 240.0, 0.75);
    click(&mut r, second);
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 4);
    tap(&mut r, "Add Color Tool");
    assert_eq!(field(&r, "wheelTool"), Value::Null, "a second click turns the tool off");
    click(&mut r, at);
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 4, "without the tool a click adds nothing");
    assert_eq!(field(&r, "__sel"), json!([2]), "it selects the marker clicked");
    put(&mut r, "tab", json!("assign"));
    assert!(has(&r, "Current Colors (2)"));
    assert!(has(&r, "Color 4"), "the Assign tab has a row for each added color");
    let undo = undo_len(&r.app());
    r.confirm().unwrap();
    assert_eq!(fills(&r.app()), fills_before, "OK leaves the art's colors");
    assert_eq!(undo_len(&r.app()), undo, "and records no undo step");
}

/// On the segmented wheel the Add Color tool adds the patch's color. Opened on a color group without
/// art, OK adds the added color to the group, as one undo step.
#[test]
fn an_added_color_joins_the_color_group() {
    let app = app();
    let before = group_keys(&app, "Brights");
    let mut r = assign(app, json!({"group": "Brights"}));
    put(&mut r, "tab", json!("edit"));
    tap(&mut r, "Show segmented color wheel");
    let v = new_hsb(&r)[0][2];
    tap(&mut r, "Add Color Tool");
    let at = wheel_at(&r, 187.0, 0.95);
    click(&mut r, at);
    let added = field(&r, "rows").as_array().unwrap().last().unwrap()["to"].clone();
    let [hue, s, b] = vectorcraft_engine::cmd::color_value(&added).unwrap().to_hsb();
    assert!((hue - 180.0).abs() < 1.0 && (s - 0.9).abs() < 0.01 && (b - v).abs() < 0.01, "the patch's color: {:?}", [hue, s, b]);
    let undo = undo_len(&r.app());
    r.confirm().unwrap();
    let mut want = before.clone();
    want.push(added);
    assert_eq!(group_keys(&r.app(), "Brights"), want);
    assert_eq!(undo_len(&r.app()), undo + 1, "one undo step");
}

/// The Remove Color tool: a click on the base color's marker removes nothing; a click on another
/// marker leaves its row of art's colors with no new color, so OK leaves that art's colors as they
/// were while the other rows recolor theirs. The tools are off for the color bars.
#[test]
fn the_remove_color_tool_removes_a_marker_but_not_the_base_colors() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200", "rgb 20 160 20"]);
    let fills_before = fills(&r.app());
    let mut rows = field(&r, "rows");
    let tos = ["rgb 255 0 0", "rgb 0 255 0", "rgb 0 0 255"];
    for (row, to) in rows.as_array_mut().unwrap().iter_mut().zip(tos) {
        row["to"] = json!(to);
    }
    r.set("rows", rows.clone()).unwrap();
    r.set("linked", json!(false)).unwrap();
    put(&mut r, "__sel", json!([0]));
    tap(&mut r, "Remove Color Tool");
    assert_eq!(field(&r, "wheelTool"), json!("remove"));
    let (red, green) = (marker(&r, 0), marker(&r, 1));
    click(&mut r, red);
    assert_eq!(field(&r, "rows"), rows, "the base color stays");
    click(&mut r, green);
    let now = field(&r, "rows");
    assert_eq!(now.as_array().unwrap().len(), 3, "the row of art's colors stays");
    assert_eq!((&now[0], &now[1]["to"], &now[2]), (&rows[0], &Value::Null, &rows[2]), "with no new color: {now}");
    assert_eq!(field(&r, "__sel"), json!([0]), "the base color stays selected");
    tap(&mut r, "Show color bars");
    assert!(!r.enabled("Remove Color Tool"), "the tools are for the wheels");
    let kept = rows[1]["from"][0].clone();
    r.confirm().unwrap();
    let after = fills(&r.app());
    for (b, a) in fills_before.iter().zip(&after) {
        let was = json!(ColorKey::of(&b.unwrap()).to_string());
        if was == kept {
            assert_eq!(a, b, "the art of the removed color keeps it");
        } else {
            assert_ne!(a, b, "the other art takes its new color");
        }
    }
}

/// The smaller angle between hues `a` and `b`, in degrees.
fn hue_gap(a: f32, b: f32) -> f32 {
    (a - b).rem_euclid(360.0).min((b - a).rem_euclid(360.0))
}

/// A marker's right-click menu has Set As Base Color, Select Shade, Color Picker and Remove Color.
/// Set As Base Color marks the row as the base, whose menu then has neither Set As Base Color nor
/// Remove Color; with another color selected, a harmony rule builds from the base color, and OK
/// recolors the art.
#[test]
fn a_markers_menu_sets_the_base_color_that_the_rules_build_from() {
    let mut r = edit(&["#ff0000", "#0000ff", "#00ff00"]);
    let hsb = new_hsb(&r);
    let at = marker(&r, 1);
    right_click(&mut r, at);
    for item in ["Set As Base Color", "Select Shade", "Color Picker", "Remove Color"] {
        assert!(has(&r, item), "{item}");
    }
    tap(&mut r, "Set As Base Color");
    let rows = field(&r, "rows");
    assert_eq!(rows[1]["base"], json!(true), "{rows}");
    assert!(rows[0].get("base").is_none() && rows[2].get("base").is_none());
    right_click(&mut r, at);
    assert!(!r.enabled("Set As Base Color"), "it is the base already");
    assert!(!r.enabled("Remove Color"), "the base color can't be removed");
    key(&mut r, Key::Escape);
    // The selected color is another one; the rule still builds from the base.
    put(&mut r, "__sel", json!([0]));
    tap(&mut r, "Harmony Rules");
    tap(&mut r, "Triad");
    let now = new_hsb(&r);
    assert!(hue_gap(now[1][0], hsb[1][0]) < 1.0, "the base color keeps its hue: {hsb:?} -> {now:?}");
    assert!(hue_gap(now[0][0], now[1][0]) > 110.0 && hue_gap(now[2][0], now[1][0]) > 110.0, "the others are a triad from it: {now:?}");
    assert_eq!(field(&r, "rows")[1]["base"], json!(true), "the rule keeps the base mark");
    let before = fills(&r.app());
    r.confirm().unwrap();
    assert_eq!(fills(&r.app()), before, "OK keeps the previewed colors");
}

/// Select Shade on a marker's menu shows a grid of the color's hue; a click on a cell gives the color
/// that saturation and brightness, keeping its hue, and closes the menu. Unlinked, the other colors
/// stay; linked, they move as a drag would move them.
#[test]
fn select_shade_gives_a_marker_the_shade_clicked() {
    for linked in [false, true] {
        let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
        put(&mut r, "linked", json!(linked));
        let hsb = new_hsb(&r);
        let at = marker(&r, 0);
        right_click(&mut r, at);
        tap(&mut r, "Select Shade");
        let cell = "Saturation 50%, Brightness 57%";
        tap(&mut r, cell);
        let now = new_hsb(&r);
        assert!(hue_gap(now[0][0], hsb[0][0]) < 1.0 && (now[0][1] - 0.5).abs() < 0.01 && (now[0][2] - 4.0 / 7.0).abs() < 0.01, "{now:?}");
        let other_moved = (now[1][2] - hsb[1][2]).abs() > 0.01;
        assert_eq!(other_moved, linked, "linked {linked}: {hsb:?} -> {now:?}");
        assert!(!has(&r, cell), "the menu closes");
        assert_eq!(field(&r, "__sel"), json!([0]));
    }
}

/// A right-click on the wheel away from the markers offers Add New Color, which adds the hue and
/// saturation there as a new color. A color added on the line from the center to a marker moves
/// with that marker when the colors are unlinked, and the others stay.
#[test]
fn add_new_color_on_a_markers_line_moves_with_that_marker() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
    r.set("linked", json!(false)).unwrap();
    put(&mut r, "__sel", json!([0]));
    let hsb = new_hsb(&r);
    let at = wheel_at(&r, 90.0, 0.6);
    right_click(&mut r, at);
    assert!(!has(&r, "Set As Base Color"), "no marker there");
    tap(&mut r, "Add New Color");
    let rows = field(&r, "rows");
    assert_eq!(rows.as_array().unwrap().len(), 3, "{rows}");
    assert_eq!((&rows[2]["from"], &rows[2]["added"]), (&json!([]), &json!(true)));
    assert!(rows[2].get("follows").is_none(), "off every marker's line");
    let [hue, s, v] = new_hsb(&r)[2];
    assert!((hue - 90.0).abs() < 1.0 && (s - 0.6).abs() < 0.01 && (v - hsb[0][2]).abs() < 0.01, "{:?}", [hue, s, v]);
    assert_eq!(field(&r, "__sel"), json!([2]));
    // Halfway along the second color's line.
    let (c, _) = wheel(&r);
    let at = c.lerp(marker(&r, 1), 0.5);
    right_click(&mut r, at);
    tap(&mut r, "Add New Color");
    let rows = field(&r, "rows");
    assert_eq!(rows.as_array().unwrap().len(), 4);
    assert!(rows[1]["tie"].is_u64() && rows[3]["follows"] == rows[1]["tie"], "{rows}");
    // Drag the second color's marker a little way round: the color on its line turns with it.
    let before = new_hsb(&r);
    let start = marker(&r, 1);
    let d = start.distance(c);
    let path: Vec<Pos2> = std::iter::once(start).chain((1..=6).map(|k| polar(c, before[1][0] + 5.0 * k as f32, d))).collect();
    drag_path(&mut r, &path);
    let_go(&mut r, path[path.len() - 1]);
    let after = new_hsb(&r);
    let turned = hue_gap(after[1][0], before[1][0]);
    assert!(turned > 20.0, "the marker turned: {before:?} -> {after:?}");
    assert!((hue_gap(after[3][0], before[3][0]) - turned).abs() < 1.0, "the color on its line turned as far: {before:?} -> {after:?}");
    assert!(hue_gap(after[0][0], before[0][0]) < 0.5 && hue_gap(after[2][0], before[2][0]) < 0.5, "the others stay: {before:?} -> {after:?}");
}

/// Set Current Color As Base Color makes the selected color the base; it is off while the selected
/// color is the base. A color bar's menu has Set As Base Color too.
#[test]
fn set_current_color_as_base_color_marks_the_selected_color() {
    let mut r = edit(&["#ff0000", "#0000ff", "#00ff00"]);
    let name = "Set Current Color As Base Color";
    assert!(!r.enabled(name), "the base color is the selected one");
    put(&mut r, "__sel", json!([2]));
    tap(&mut r, name);
    assert_eq!(field(&r, "rows")[2]["base"], json!(true));
    assert!(!r.enabled(name));
    tap(&mut r, "Show color bars");
    let bar = rect(&r, "Color 1").center();
    right_click(&mut r, bar);
    tap(&mut r, "Set As Base Color");
    let rows = field(&r, "rows");
    assert_eq!((&rows[0]["base"], rows[2].get("base")), (&json!(true), None), "{rows}");
}

#[test]
fn row_context_menu_pick_from_swatches_and_limit_to_library() {
    for empty_new in [false, true] {
        let mut app = app();
        art(&mut app, &["#ff0000", "#0000ff"]);
        // A known, single-color palette makes it impossible to pass without snapping the rows.
        let doc = &app.session.doc().unwrap().doc;
        let names: Vec<_> = doc
            .swatches
            .iter()
            .filter(|s| !s.is_reserved())
            .map(|s| s.name.clone())
            .chain(doc.swatch_groups.iter().map(|g| g.name.clone()))
            .collect();
        app.run("swatch.delete", json!({"names": names})).unwrap();
        app.run("swatch.new", json!({"name": "Only Yellow", "color": "#ffff00"})).unwrap();
        let undo = undo_len(&app);
        let mut r = assign(app, json!({}));
        let row = 1;
        if empty_new {
            let mut rows = field(&r, "rows");
            rows[row]["to"] = Value::Null;
            put(&mut r, "rows", rows);
        }
        let before = field(&r, "rows");
        let expected = if empty_new { before[row]["from"][0].clone() } else { before[row]["to"].clone() };
        // The row's New chip, as production's right-click took it; `Color N` is the row's hex field.
        let at = rect(&r, &format!("New {}", row + 1)).center();
        right_click(&mut r, at);
        assert_eq!(field(&r, "__sel"), json!([row]), "right-click selects the targeted row");
        tap(&mut r, "Pick from Swatches");
        let (picker_id, picked_row, picked) = picker(&r).expect("Pick from Swatches opens the Color Picker");
        assert_eq!(picked_row, row.to_string(), "for the targeted row");
        assert_eq!(picked["swatches"], json!(true), "on its swatches");
        assert_eq!(vectorcraft_engine::cmd::color_value(&picked["color"]), vectorcraft_engine::cmd::color_value(&expected));
        r.host(|host, ctx| host.cancel_by_id(ctx, picker_id));
        r.settled();
        assert_eq!(field(&r, "rows"), before, "picker Cancel preserves all rows");

        right_click(&mut r, at);
        tap(&mut r, "Limit to Library");
        tap(&mut r, "Document Swatches");
        assert_eq!(field(&r, "limitTo"), json!("document"));
        let snapped = field(&r, "rows");
        for (i, row) in snapped.as_array().unwrap().iter().enumerate() {
            assert_eq!(row["to"], "rgb 255 255 0", "row {i} snaps to the palette");
        }
        assert_ne!(snapped, before, "the menu changes row colors");
        right_click(&mut r, at);
        tap(&mut r, "Limit to Library");
        tap(&mut r, "None");
        assert_eq!(field(&r, "limitTo"), json!(""));
        assert_eq!(undo_len(&r.app()), undo, "menu actions remain previews");
        tap(&mut r, "Cancel");
        assert_eq!(colors(&mut r.app()), 2, "Recolor Cancel restores the original artwork");
    }
}

/// A Color Picker opened over the dialog, on its color models, has the Color Swatches button that
/// switches it to the swatches.
#[test]
fn a_picker_over_the_dialog_offers_the_color_swatches_view() {
    let mut r = edit(&["#ff0000", "#0000ff"]);
    let at = marker(&r, 0);
    right_click(&mut r, at);
    tap(&mut r, "Color Picker");
    assert!(picker(&r).is_some(), "the marker's Color Picker opens the picker");
    assert!(has(&r, "Color Swatches"), "the picker has the Color Swatches button");
    tap(&mut r, "Color Swatches");
    assert!(has(&r, "Color Models"), "the button switches the picker to its swatches");
}

/// Pick from Swatches opens the picker on its swatches, showing the library that Limit to Library
/// limits the rows to.
#[test]
fn pick_from_swatches_shows_the_limit_to_library_library() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let lib = library_of(&mut app, &[json!({"name": "Lib Teal", "color": "#008080"}), json!({"name": "Lib Plum", "color": "#8e4585"})]);
    let mut r = assign(app, json!({"library": lib}));
    assert_eq!(field(&r, "limitTo"), json!(lib));
    let at = rect(&r, "New 1").center();
    right_click(&mut r, at);
    tap(&mut r, "Pick from Swatches");
    assert!(picker(&r).is_some(), "Pick from Swatches opens the Color Picker");
    assert!(has(&r, "Color Models"), "the picker shows its swatches, with the button back to the color models");
    assert!(has(&r, "Lib Teal") && has(&r, "Lib Plum"), "the swatches are the Limit to Library library's");
}

/// Undo Changes and Redo Changes step back and forward through the dialog's own color edits, and
/// the art previews each step; the document's undo history is unchanged until OK, which records
/// one step. Both buttons start disabled, and a new edit after Undo Changes leaves nothing to redo.
#[test]
fn undo_and_redo_changes_step_through_the_dialogs_own_edits() {
    let mut r = edit(&["#ff0000", "#00ff00", "#0000ff"]);
    let undo = undo_len(&r.app());
    tap(&mut r, "Assign");
    assert!(!r.enabled("Undo Changes") && !r.enabled("Redo Changes"), "nothing to step to on opening");
    let state = |r: &Recolor| (field(r, "rows"), fills(&r.app()));
    let start = state(&r);
    tap(&mut r, "Randomly change color order");
    let first = state(&r);
    assert_ne!(first, start, "the order changed and previews");
    tap(&mut r, "Randomly change saturation and brightness");
    let second = state(&r);
    assert_ne!(second, first);
    assert!(r.enabled("Undo Changes") && !r.enabled("Redo Changes"));
    for (button, want) in [("Undo Changes", &first), ("Undo Changes", &start), ("Redo Changes", &first), ("Redo Changes", &second)] {
        tap(&mut r, button);
        assert_eq!(&state(&r), want, "{button}");
    }
    assert_eq!(undo_len(&r.app()), undo, "the dialog's steps leave the document's history alone");
    tap(&mut r, "Undo Changes");
    assert_eq!(state(&r), first);
    assert!(r.enabled("Redo Changes"));
    tap(&mut r, "Randomly change saturation and brightness");
    assert!(!r.enabled("Redo Changes"), "a new edit leaves nothing to redo");
    let last = state(&r);
    tap(&mut r, "OK");
    assert!(!r.is_open());
    assert_eq!(fills(&r.app()), last.1, "OK keeps the edits");
    assert_eq!(undo_len(&r.app()), undo + 1, "as one undo step");
    r.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(fills(&r.app()), start.1);
    // A new opening has nothing to undo.
    r.app().run("select.all", json!({})).unwrap();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    r.settled();
    assert!(!r.enabled("Undo Changes") && !r.enabled("Redo Changes"), "nothing to step to on a new opening");
    tap(&mut r, "Cancel");
}

/// A drag is one step from press to release: Undo Changes after dragging the Edit tab's Brightness
/// slider across several frames gives the rows the drag started on, with nothing left to undo.
#[test]
fn a_drag_is_one_step_of_undo_changes() {
    let mut r = edit(&["rgb 200 100 100", "rgb 100 200 200"]);
    let rows = field(&r, "rows");
    let rail = rect(&r, "Brightness");
    let v = new_hsb(&r)[0][2];
    let at = pos2(rail.left() + v * rail.width(), rail.center().y);
    let end = pos2(at.x + 72.0, at.y);
    drag_path(&mut r, &line(at, end, 6));
    let_go(&mut r, end);
    assert_ne!(field(&r, "rows"), rows, "the drag changed the colors");
    tap(&mut r, "Undo Changes");
    assert_eq!(field(&r, "rows"), rows, "one Undo Changes goes back to before the drag");
    assert!(!r.enabled("Undo Changes"), "the drag was one step");
    assert!(r.enabled("Redo Changes"));
}

/// Reset is a step of its own: Undo Changes gives back the rows Reset discarded.
#[test]
fn undo_changes_reverses_reset() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#00ff00", "#0000ff"]);
    let mut r = assign(app, json!({}));
    let start = field(&r, "rows");
    tap(&mut r, "Randomly change saturation and brightness");
    let edited = field(&r, "rows");
    assert_ne!(edited, start);
    tap(&mut r, "Reset the rows to the artwork's colors");
    assert_eq!(field(&r, "rows"), start, "Reset reads the art's colors");
    tap(&mut r, "Undo Changes");
    assert_eq!(field(&r, "rows"), edited, "Undo Changes gives back the rows before Reset");
    tap(&mut r, "Redo Changes");
    assert_eq!(field(&r, "rows"), start, "Redo Changes resets them again");
}

/// `ui.recolorUndo` and `ui.recolorRedo` step the open dialog as its buttons do, also through a
/// field set without a frame between, and a new count of Colors is one step that restores the count
/// and the rows; outside the dialog they are an error, and a new opening has nothing to undo.
#[test]
fn the_recolor_undo_and_redo_commands_step_the_open_dialog() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    assert!(app.run("ui.recolorUndo", json!({})).is_err(), "no dialog open");
    // A field set before the dialog's first frame is a step back to the fields it opened with.
    let mut r = open(app, json!({}));
    r.set("colors", json!(2)).unwrap();
    assert_eq!(steps(&r), (true, false), "Undo Changes can run before the first frame");
    let res = r.app().run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(res, json!({"changed": true, "canUndo": false, "canRedo": true}));
    assert_eq!(field(&r, "colors"), Value::Null);
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 4, "the rows of the art as it opened");
    r.cancel();
    r.app().run("select.all", json!({})).unwrap();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    r.settled();
    assert_eq!(steps(&r), (false, false));
    let start = field(&r, "rows");
    assert_eq!(start.as_array().unwrap().len(), 4);
    put(&mut r, "colors", json!(2));
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 2);
    let two = field(&r, "rows");
    put(&mut r, "globalAdjust", json!({"saturation": -50}));
    let res = r.app().run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(res, json!({"changed": true, "canUndo": true, "canRedo": true}));
    assert_eq!((field(&r, "globalAdjust"), field(&r, "rows")), (Value::Null, two.clone()));
    r.app().run("ui.recolorUndo", json!({})).unwrap();
    r.settled();
    assert_eq!((field(&r, "colors"), field(&r, "rows")), (Value::Null, start.clone()), "the count and its rows, not reduced again");
    assert!(!steps(&r).0);
    assert_eq!(r.app().run("ui.recolorUndo", json!({})).unwrap()["changed"], json!(false));
    r.app().run("ui.recolorRedo", json!({})).unwrap();
    assert_eq!((field(&r, "colors"), field(&r, "rows")), (json!(2), two));
    // A field set without a frame between is a step before Undo Changes goes back.
    r.set("method", json!("exact")).unwrap();
    assert_eq!(r.app().run("ui.recolorUndo", json!({})).unwrap()["changed"], json!(true));
    assert_eq!(field(&r, "method"), json!("scaleTints"));
    r.cancel();
    assert!(r.app().run("ui.recolorRedo", json!({})).is_err(), "Cancel closed the dialog");
    r.app().run("select.all", json!({})).unwrap();
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    r.settled();
    assert_eq!(steps(&r), (false, false), "a new opening");
}

/// An edit made while the pointer is held from the dialog's first frame is a step from the fields
/// the dialog opened with, not where Undo Changes starts.
#[test]
fn an_edit_with_the_pointer_down_from_the_first_frame_is_a_step() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    let mut r = open(app, json!({}));
    let at = Pos2::new(2.0, 2.0);
    r.frame_with(vec![Event::PointerMoved(at), pointer(at, PointerButton::Primary, true)], Modifiers::NONE);
    let start = field(&r, "rows");
    assert_eq!(start.as_array().unwrap().len(), 4);
    r.set("colors", json!(2)).unwrap();
    r.frame();
    r.frame_with(vec![pointer(at, PointerButton::Primary, false)], Modifiers::NONE);
    assert_eq!(field(&r, "rows").as_array().unwrap().len(), 2);
    let res = r.app().run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(res, json!({"changed": true, "canUndo": false, "canRedo": true}));
    assert_eq!((field(&r, "colors"), field(&r, "rows")), (Value::Null, start));
}

/// Rows or weights set before the first frame, which the art's reduction then replaces or drops,
/// are no step: Undo Changes is not available and changes nothing.
#[test]
fn rows_and_weights_set_before_the_art_is_read_are_no_step() {
    for (k, v) in [("rows", json!([{"to": "#123456"}])), ("weights", json!([25, 75]))] {
        let mut app = app();
        art(&mut app, &["#ff0000", "#0000ff"]);
        let mut r = open(app, json!({}));
        r.set(k, v).unwrap();
        assert!(!steps(&r).0, "{k}: Undo Changes is not available");
        let res = r.app().run("ui.recolorUndo", json!({})).unwrap();
        assert_eq!(res, json!({"changed": false, "canUndo": false, "canRedo": false}), "{k}");
        assert!(!r.app().session.in_interaction(), "{k}: a step that changes nothing previews nothing");
        assert_eq!(field(&r, "rows").as_array().unwrap().len(), 2, "{k}: the rows of the art");
    }
}

/// `ui.recolorUndo` and `ui.recolorRedo` preview the step they go to at once, without a frame
/// between, as the buttons do.
#[test]
fn the_recolor_undo_and_redo_commands_preview_the_step() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#e01010", "#0000ff", "#1010e0"]);
    let mut r = open(app, json!({}));
    r.settled();
    let start = fills(&r.app());
    put(&mut r, "globalAdjust", json!({"saturation": -100}));
    let adjusted = fills(&r.app());
    assert_ne!(adjusted, start, "the adjustment previews");
    r.app().run("ui.recolorUndo", json!({})).unwrap();
    assert_eq!(fills(&r.app()), start, "Undo Changes previews the step before");
    r.app().run("ui.recolorRedo", json!({})).unwrap();
    assert_eq!(fills(&r.app()), adjusted, "Redo Changes previews the step after");
    r.cancel();
}

/// Edit › Edit Colors › Recolor with Preset asks whether to limit the colors to a swatch library
/// before Recolor Artwork opens: Cancel opens nothing, and OK opens Recolor Artwork with the
/// preset's colors and the library chosen.
#[test]
fn recolor_with_preset_asks_for_a_library_first() {
    let mut r = Recolor::new(app());
    assert!(r.app().run("ui.recolorDialog", json!({"colors": 1, "preset": true})).is_err(), "nothing selected");
    assert!(preset(&r).is_none());
    art(&mut r.app(), &["#333333", "#999999"]);
    for (p, why) in [
        (json!({"colors": 1, "preset": 1}), "`preset` is true or false"),
        (json!({"colors": 0, "preset": true}), "a preset has at least one color"),
        (json!({"colors": 1, "preset": true, "group": "Grays"}), "a preset takes no group"),
    ] {
        assert!(r.app().run("ui.recolorDialog", p).is_err(), "{why}");
    }
    assert!(preset(&r).is_none() && !r.is_open());
    let presets: Vec<Value> = crate::menus::menu_entries(&r.app())
        .iter()
        .filter(|e| e.path.last().is_some_and(|p| p == "Recolor with Preset"))
        .map(|e| e.params.clone())
        .collect();
    assert_eq!(presets.len(), 4);
    for p in &presets {
        r.app().run("ui.recolorDialog", p.clone()).unwrap();
        r.settled();
        assert!(preset(&r).is_some() && !r.is_open(), "{p}");
        cancel_preset(&mut r);
        assert!(preset(&r).is_none() && !r.is_open(), "Cancel opens nothing: {p}");
    }
    // A preset opened over Recolor Artwork's preview rolls the preview back.
    let start = colors(&mut r.app());
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    r.set("colors", json!(1)).unwrap();
    r.set("method", json!("exact")).unwrap();
    r.settled();
    assert_eq!(colors(&mut r.app()), 1, "previewed");
    r.app().run("ui.recolorDialog", presets[0].clone()).unwrap();
    assert!(preset(&r).is_some());
    assert_eq!(colors(&mut r.app()), start, "the preview is rolled back");
    r.settled();
    assert!(!r.is_open(), "Recolor Artwork closed");
    assert_eq!(preset_field(&r, "limitTo"), json!(""), "a color job starts on None");
    set_preset(&mut r, "limitTo", json!("document")).unwrap();
    // OK with nothing to recolor leaves the dialog open.
    r.app().run("select.none", json!({})).unwrap();
    assert!(confirm_preset(&mut r).is_err());
    assert!(preset(&r).is_some() && !r.is_open(), "the dialog stays");
    r.app().run("select.all", json!({})).unwrap();
    confirm_preset(&mut r).unwrap();
    assert!(r.is_open());
    assert_eq!((field(&r, "colors"), field(&r, "method"), field(&r, "limitTo")), (json!(1), json!("scaleTints"), json!("document")));
    // Recolor Artwork opened over its own preview rolls that preview back too.
    r.set("method", json!("exact")).unwrap();
    r.set("limitTo", json!("")).unwrap();
    r.settled();
    assert_eq!(colors(&mut r.app()), 1, "previewed");
    r.app().run("ui.recolorDialog", json!({})).unwrap();
    assert_eq!(colors(&mut r.app()), start, "the preview is rolled back");
    r.cancel();
}

/// Recolor Artwork opened on a colour group over its own preview of that group starts on the
/// group's colours as the rolled-back preview leaves them.
#[test]
fn reopening_a_group_over_its_preview_starts_on_the_groups_colours() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let before = group_keys(&app, "Brights");
    let mut r = open(app, json!({"group": "Brights"}));
    r.settled();
    let mut rows = field(&r, "rows");
    rows[1]["to"] = json!("rgb 1 2 3");
    r.set("rows", rows).unwrap();
    r.settled();
    assert_ne!(group_keys(&r.app(), "Brights"), before, "the preview changes the group");
    r.app().run("ui.recolorDialog", json!({"group": "Brights"})).unwrap();
    assert_eq!(group_keys(&r.app(), "Brights"), before, "the preview is rolled back");
    assert_eq!(field(&r, "__assign"), json!(before), "the new dialog starts on the group's colours");
    r.cancel();
}

/// Recolor Artwork opened on a colour group over a preview that renames it finds the group by its
/// name without the preview; an unknown group leaves the open dialog as it is.
#[test]
fn reopening_a_group_over_a_preview_that_renames_it_finds_the_group() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let before = group_keys(&app, "Brights");
    let mut r = open(app, json!({"group": "Brights"}));
    r.settled();
    r.set("groupName", json!("Renamed")).unwrap();
    r.settled();
    let named = |r: &Recolor, n: &str| r.app().session.doc().unwrap().doc.swatch_groups.iter().any(|g| g.name == n);
    assert!(named(&r, "Renamed") && !named(&r, "Brights"), "the preview renames the group");
    assert!(r.app().run("ui.recolorDialog", json!({"group": "Nope"})).is_err());
    assert_eq!(field(&r, "groupName"), json!("Renamed"), "an unknown group leaves the dialog open");
    r.app().run("ui.recolorDialog", json!({"group": "Brights"})).unwrap();
    assert!(named(&r, "Brights") && !named(&r, "Renamed"), "the preview is rolled back");
    assert_eq!(field(&r, "__assign"), json!(before), "the new dialog starts on the group's colours");
    r.cancel();
}

/// A `library` or `group` that is not a string is an error, with or without a preset, and opens
/// no dialog.
#[test]
fn recolor_rejects_a_library_or_group_that_is_not_a_string() {
    let mut app = app();
    art(&mut app, &["#ff0000", "#0000ff"]);
    let mut r = Recolor::new(app);
    for p in [
        json!({"colors": 1, "library": 123}),
        json!({"preset": true, "colors": 1, "library": 123}),
        json!({"preset": true, "colors": 1, "library": true}),
        json!({"group": 5}),
        json!({"group": ["Brights"]}),
    ] {
        assert!(r.app().run("ui.recolorDialog", p.clone()).is_err(), "{p}");
        assert!(!r.is_open() && preset(&r).is_none(), "{p} opens no dialog");
    }
    r.app().run("ui.recolorDialog", json!({"colors": 1, "library": null, "group": null})).unwrap();
    assert_eq!(field(&r, "limitTo"), json!(""), "a null library is no limit");
    r.cancel();
}

/// Every paint under `v`, in order: its color, the swatch it is linked to and its tint.
fn paints_in(v: &Value, out: &mut Vec<(Color, Option<String>, f64)>) {
    match v {
        Value::Object(m) => {
            if let Some(c) = m.get("color").and_then(|c| serde_json::from_value::<Color>(c.clone()).ok()) {
                out.push((c, m.get("swatch").and_then(Value::as_str).map(String::from), m.get("tint").and_then(Value::as_f64).unwrap_or(1.0)));
            }
            m.values().for_each(|x| paints_in(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| paints_in(x, out)),
        _ => {}
    }
}

/// Every number under an `opacity` key under `v`, in order.
fn opacities_in(v: &Value, out: &mut Vec<f64>) {
    match v {
        Value::Object(m) => {
            out.extend(m.get("opacity").and_then(Value::as_f64));
            m.values().for_each(|x| opacities_in(x, out));
        }
        Value::Array(a) => a.iter().for_each(|x| opacities_in(x, out)),
        _ => {}
    }
}

/// The art of every layer as saved.
fn art_json(app: &VectorcraftApp) -> Value {
    let d = &app.session.doc().unwrap().doc;
    Value::Array(d.layers.iter().filter_map(|l| l.children()).map(|c| serde_json::to_value(c).unwrap()).collect())
}

fn art_paints(app: &VectorcraftApp) -> Vec<(Color, Option<String>, f64)> {
    let mut out = vec![];
    paints_in(&art_json(app), &mut out);
    out
}

/// A gray other than black or white, with its lightness.
fn gray(c: &Color) -> Option<f32> {
    let [r, g, b] = c.to_rgb();
    ((r - g).abs() < 1e-3 && (g - b).abs() < 1e-3 && r > 0.01 && r < 0.99).then_some(r)
}

/// Gray art placed from an SVG file (fills, a stroke, a group at 50% opacity, `fill-opacity`, a
/// compound path, a gradient's stops and type) with a 1 Color Job of Recolor with Preset limited
/// to the document's swatches and a spot or global ink picked for its row becomes tints of that
/// ink linked to its swatch, darker grays at higher tints, with its opacities kept; editing the
/// swatch then recolors the art at the same tints. A process swatch gives tints of its color with
/// no link.
#[test]
fn a_one_color_job_limited_to_document_swatches_links_gray_art_to_the_ink() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="240" height="80"><rect x="0" y="0" width="20" height="20" fill="#000000"/><rect x="25" y="0" width="20" height="20" fill="#333333" stroke="#808080" stroke-width="2"/><g opacity="0.5"><rect x="50" y="0" width="20" height="20" fill="#808080"/></g><rect x="75" y="0" width="20" height="20" fill="#cccccc" fill-opacity="0.7"/><rect x="100" y="0" width="20" height="20" fill="#ffffff" stroke="#000000"/><path d="M130 0 L150 0 L150 20 Z M135 5 L145 5 L145 15 Z" fill="#666666" fill-rule="evenodd"/><defs><linearGradient id="g"><stop offset="0" stop-color="#222222"/><stop offset="1" stop-color="#dddddd"/></linearGradient></defs><rect x="160" y="0" width="20" height="20" fill="url(#g)"/><text x="0" y="60" font-size="18" fill="#555555">Ink</text></svg>"##;
    let path = std::env::temp_dir().join(format!("vectorcraft-recolor-preset-{}.svg", std::process::id()));
    std::fs::write(&path, svg).unwrap();
    for (kind, options, linked) in [("spot", json!({"spot": true}), true), ("global", json!({"global": true}), true), ("process", json!({}), false)] {
        let mut r = Recolor::new(app());
        let mut swatch = json!({"name": "Ink", "color": "cmyk 0 100 0 0"});
        swatch.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        r.app().run("swatch.new", swatch).unwrap();
        r.app().run("file.place", json!({"path": path.to_string_lossy(), "at": [120, 50]})).unwrap();
        r.app().run("select.all", json!({})).unwrap();
        let before = art_paints(&r.app());
        let grays: Vec<usize> = (0..before.len()).filter(|&i| gray(&before[i].0).is_some()).collect();
        assert!(grays.len() >= 9, "{kind}: the fills, the stroke, the stops and the type are gray: {before:?}");
        let mut opacities = vec![];
        opacities_in(&art_json(&r.app()), &mut opacities);
        assert!(opacities.iter().any(|o| (o - 0.5).abs() < 1e-6), "the group's opacity came in: {opacities:?}");
        assert!(opacities.iter().any(|o| (o - 0.7).abs() < 1e-6), "the fill opacity came in: {opacities:?}");

        let job = crate::menus::menu_entries(&r.app()).iter().find(|e| e.label == "1 Color Job…").unwrap().params.clone();
        r.app().run("ui.recolorDialog", job).unwrap();
        set_preset(&mut r, "limitTo", json!("document")).unwrap();
        confirm_preset(&mut r).unwrap();
        r.settled();
        let ink = r.app().session.doc().unwrap().doc.swatch("Ink").unwrap().paint.color().unwrap();
        let mut rows = field(&r, "rows").as_array().unwrap().clone();
        let row = rows.iter().position(|r| !r["exclude"].as_bool().unwrap_or(false)).unwrap();
        assert!(rows.iter().enumerate().all(|(i, r)| i == row || r["exclude"].as_bool().unwrap_or(false)), "one row: {rows:?}");
        rows[row]["to"] = json!(ColorKey::of(&ink).to_string());
        r.set("rows", Value::Array(rows)).unwrap();
        r.settled();
        r.confirm().unwrap();
        assert!(!r.is_open());

        let after = art_paints(&r.app());
        assert_eq!(after.len(), before.len(), "{kind}: the art has the same paints");
        let mut kept = vec![];
        opacities_in(&art_json(&r.app()), &mut kept);
        assert_eq!(kept, opacities, "{kind}: the opacities are kept");
        for &i in &grays {
            let (c, link, tint) = &after[i];
            assert!(gray(c).is_none(), "{kind}: paint {i} is no longer gray: {c:?}");
            assert_eq!(link.is_some(), linked, "{kind}: paint {i} linked: {link:?}");
            if linked {
                assert_eq!(link.as_deref(), Some("Ink"));
                assert!(*tint > 0.0 && *tint <= 1.0, "{kind}: paint {i} at a tint: {tint}");
            }
        }
        for (i, c) in before.iter().enumerate().filter(|(i, _)| !grays.contains(i)) {
            assert_eq!(after[i].0, c.0, "{kind}: black and white are kept");
        }
        for &a in &grays {
            for &b in &grays {
                if gray(&before[a].0) < gray(&before[b].0) {
                    if linked {
                        assert!(after[a].2 > after[b].2, "{kind}: a darker gray takes more ink: {:?} {:?}", after[a], after[b]);
                    } else {
                        assert_ne!(after[a].0, after[b].0, "{kind}: grays of different lightness get different colors: {a} {b}");
                    }
                }
            }
        }
        if linked {
            r.app().run("swatch.edit", json!({"name": "Ink", "color": "cmyk 100 0 0 0"})).unwrap();
            let edited = art_paints(&r.app());
            for &i in &grays {
                assert_eq!(edited[i].1.as_deref(), Some("Ink"), "{kind}: paint {i} stays linked");
                assert!((edited[i].2 - after[i].2).abs() < 1e-6, "{kind}: paint {i} keeps its tint");
                assert_ne!(edited[i].0, after[i].0, "{kind}: paint {i} takes the edited ink");
            }
        }
    }
    let _ = std::fs::remove_file(&path);
}
