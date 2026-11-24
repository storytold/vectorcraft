//! The dialogs the window's dialog host shows, driven through the whole-window frame.

use egui::accesskit::Role;
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::*;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

/// The window the tests draw in.
pub(super) const SCREEN: egui::Vec2 = egui::vec2(1600.0, 1100.0);

pub(super) use crate::testing::windowed as window;

pub(super) fn app() -> VectorcraftApp {
    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// One frame of the whole window with `events`. The frame pumps the dialog host itself after the shell pass (wg.A18), so
/// a dialog's answer is taken in it.
pub(super) fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) -> egui::FullOutput {
    let ctx = app.ui_ctx.clone().expect("a window app");
    let raw = egui::RawInput { events, focused: true, screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, SCREEN)), ..Default::default() };

    crate::testing::app_frame(app, &ctx, raw)
}

/// Frames until what a command just opened has measured, placed itself and run its first preview.
pub(super) fn settle(app: &mut VectorcraftApp) {
    for _ in 0..6 {
        frame(app, vec![]);
    }
}

/// The dialogs the host has open, bottom first, each with its id and the fields automation reads.
fn open_dialogs(app: &VectorcraftApp) -> Vec<(egui::Id, Value)> {
    app.dialogs()
        .map(|host| host.with(|host| host.open_dialogs().into_iter().map(|d| (d.id, d.fields.unwrap_or(Value::Null))).collect()))
        .unwrap_or_default()
}

/// Whether the host shows a dialog.
pub(super) fn is_open(app: &VectorcraftApp) -> bool {
    !open_dialogs(app).is_empty()
}

fn top(app: &VectorcraftApp) -> (egui::Id, Value) {
    open_dialogs(app).pop().expect("a dialog is open")
}

/// The fields of the top dialog, as `ui.dialog` reads them.
pub(super) fn fields(app: &VectorcraftApp) -> Value {
    top(app).1
}

/// A number field of the top dialog; `None` while it reads Mixed or is absent.
pub(super) fn field(app: &VectorcraftApp, key: &str) -> Option<f64> {
    fields(app).get(key).and_then(Value::as_f64)
}

pub(super) fn num(app: &VectorcraftApp, key: &str) -> f64 {
    field(app, key).unwrap_or(f64::NAN)
}

pub(super) fn flag(app: &VectorcraftApp, key: &str) -> bool {
    fields(app).get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// Sets one field of the top dialog as `ui.dialog.set` does; a field the dialog refuses fails the test.
pub(super) fn set(app: &mut VectorcraftApp, field: &str, value: Value) {
    let (id, ctx) = (top(app).0, app.ui_ctx.clone().expect("a window app"));
    app.dialogs().expect("a window app").with(|host| host.set_json_field(&ctx, id, field, value)).unwrap_or_else(|e| panic!("{field}: {e}"));
}

pub(super) fn set_all<'a>(app: &mut VectorcraftApp, fields: impl IntoIterator<Item = (&'a str, Value)>) {
    for (field, value) in fields {
        set(app, field, value);
    }
}

/// Answers the top dialog OK or Cancel as the control channel does; the frames after it take the answer. Whether the
/// host took it (a dialog with an open child does not, nor a held OK the app refuses).
fn answer(app: &mut VectorcraftApp, ok: bool) -> bool {
    let (id, ctx) = (top(app).0, app.ui_ctx.clone().expect("a window app"));
    let taken = app.dialogs().expect("a window app").with(|host| if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) });
    frame(app, vec![]);
    frame(app, vec![]);
    // A held OK reports `false` at the press; the app's answer then closes the dialog.
    taken || (ok && open_dialogs(app).iter().all(|(open, _)| *open != id))
}

pub(super) fn confirm(app: &mut VectorcraftApp) -> bool {
    answer(app, true)
}

pub(super) fn cancel(app: &mut VectorcraftApp) -> bool {
    answer(app, false)
}

fn can_reset(app: &VectorcraftApp) -> bool {
    let id = top(app).0;
    app.dialogs().expect("a window app").with(|host| host.can_reset_by_id(id))
}

fn reset(app: &mut VectorcraftApp) -> bool {
    let (id, ctx) = (top(app).0, app.ui_ctx.clone().expect("a window app"));
    let done = app.dialogs().expect("a window app").with(|host| host.reset_by_id(&ctx, id));
    frame(app, vec![]);
    done
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.iter().filter(|c| !c.is_ruler_guide()).count())).sum()
}

#[test]
fn confirm_runs_each_dialogs_command() {
    let mut app = app();
    open_tool_dialog(&mut app, "star", json!({"x": 50, "y": 50}));
    set_all(&mut app, [("radius1", json!(40)), ("radius2", json!(20)), ("points", json!(6))]);
    assert!(confirm(&mut app));
    open_tool_dialog(&mut app, "lineSegment", json!({"x": 0, "y": 0}));
    set_all(&mut app, [("length", json!(30)), ("angle", json!(90))]);
    assert!(confirm(&mut app));
    assert_eq!(objects(&app), 2);
    app.run("select.all", json!({})).unwrap();
    // Scale with Uniform uses the horizontal value for both axes; Copy keeps the original.
    crate::menus::invoke(&mut app, "object.scale", json!({}));
    set_all(&mut app, [("sx", json!(200)), ("sy", json!(50)), ("uniform", json!(true)), ("copy", json!(true))]);
    assert!(confirm(&mut app));
    assert_eq!(objects(&app), 4);
    // The parameter dialog closes, then runs its command with the edited values.
    app.run("ui.paramDialog", json!({"command": "view.saved.new", "label": "New View", "params": {"name": "Close-up"}})).unwrap();
    assert!(confirm(&mut app));
    assert!(!is_open(&app));
    assert_eq!(app.session.doc().unwrap().doc.views.last().map(|v| v.name.as_str()), Some("Close-up"));
    let (command, params) = app.session.journal.last().unwrap();
    assert_eq!((command.as_str(), &params["name"]), ("view.saved.new", &json!("Close-up")));
    // New Document creates the document and closes.
    let docs = app.session.documents().len();
    app.run("file.newDialog", json!({})).unwrap();
    set_all(
        &mut app,
        [
            ("width", json!("300 pt")),
            ("height", json!("200 pt")),
            ("units", json!("Points")),
            ("name", json!("Card")),
            ("artboards", json!(1)),
            ("colorMode", json!("RGB")),
        ],
    );
    assert!(confirm(&mut app));
    assert!(!is_open(&app));
    assert_eq!(app.session.documents().len(), docs + 1);
}

#[test]
fn effect_preview_is_kept_as_one_undo_step() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("effect.dialog", json!({"effect": "distort.roughen"})).unwrap();
    set(&mut app, "preview", json!(true));
    frame(&mut app, vec![]);
    assert!(app.session.in_interaction(), "the preview runs as an interaction");
    assert!(confirm(&mut app));
    assert!(!is_open(&app) && !app.session.in_interaction());
    assert_eq!(app.last_effect.as_ref().map(|e| e.0.as_str()), Some("distort.roughen"));
    assert_ne!(app.session.doc().unwrap().doc, before);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
}

/// A label followed by its value takes the UI language's colon (#7991): a style's attribute list shows one colon.
#[test]
fn label_and_value_colons_follow_the_ui_language() {
    let fr = crate::i18n::Lang::from_code("fr").unwrap();
    crate::i18n::set_current(fr);
    let attribute = super::form::attribute_line("fontSize", &json!(12));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(attribute, format!("{}\u{a0}: 12", crate::i18n::tr(fr, "Font Size")));
    assert_eq!(super::form::attribute_line("fontSize", &json!(12)), "Font Size: 12");
}

/// Drop Shadow's Color as text the effect can't draw: the row refuses it as it is set, and the value put in the dialog
/// directly is refused by the engine at OK, which keeps the dialog open, with Preview on or off, and leaves the
/// document and the last effect as they were.
#[test]
fn effect_dialog_refuses_a_color_it_cannot_draw() {
    use craft::ui::patterns::form_dialogs::{FieldValue, HostedParamDialog};
    for preview in [true, false] {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("select.all", json!({})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        app.run("effect.dialog", json!({"effect": "stylize.dropShadow"})).unwrap();
        set(&mut app, "preview", json!(preview));
        let (id, ctx) = (top(&app).0, app.ui_ctx.clone().expect("a window app"));
        let host = app.dialogs().expect("a window app");
        assert!(host.with(|h| h.set_json_field(&ctx, id, "color", json!("not-a-color"))).is_err(), "preview {preview}");
        host.with(|h| h.get_mut_by_id::<HostedParamDialog>(id).map(|d| d.set_field("color", FieldValue::from("not-a-color"))))
            .expect("a parameter dialog");
        frame(&mut app, vec![]);
        confirm(&mut app);
        // PLAN: the reason the dialog shows is the framework's row for a refused answer; it is not read here.
        assert!(is_open(&app) && app.last_effect.is_none(), "preview {preview}");
        assert_eq!(app.session.doc().unwrap().doc, before, "preview {preview}");
    }
}

/// After File › Document Color Mode, a Drop Shadow's, an Outer Glow's and a Shift to Color's dialog open on the
/// converted color, and OK without editing it keeps the color.
#[test]
fn effect_dialogs_edit_a_color_that_document_color_mode_converted() {
    for effect in ["stylize.dropShadow", "stylize.outerGlow", "adjust.shiftToColor"] {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.run("effect.apply", json!({"ids": [id], "effect": effect, "item": null, "params": {"color": "#ff0000"}})).unwrap();
        app.run("file.documentColorMode", json!({"mode": "cmyk"})).unwrap();
        let color = |app: &VectorcraftApp| {
            app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["color"].clone()
        };
        let converted = color(&app);
        let hex = serde_json::from_value::<vectorcraft_doc::color::Color>(converted.clone())
            .unwrap_or_else(|e| panic!("{effect}: {converted}: {e}"))
            .to_hex();
        app.run("effect.dialog", json!({"effect": effect, "index": 0, "item": null})).unwrap();
        settle(&mut app);
        // A colour row reads as the framework's `[r, g, b]`.
        // Shift to Color's own colour row keeps the document's colour.
        let shown = fields(&app)["color"].clone();
        let shown_hex = match serde_json::from_value::<[f64; 3]>(shown.clone()) {
            Ok(rgb) => format!("#{:02x}{:02x}{:02x}", (rgb[0] * 255.0).round() as u8, (rgb[1] * 255.0).round() as u8, (rgb[2] * 255.0).round() as u8),
            Err(_) => {
                serde_json::from_value::<vectorcraft_doc::color::Color>(shown.clone()).unwrap_or_else(|e| panic!("{effect}: {shown}: {e}")).to_hex()
            }
        };
        assert_eq!(shown_hex.to_lowercase(), hex.to_lowercase(), "{effect}: {shown}");
        assert!(confirm(&mut app));
        assert_eq!(color(&app), converted, "{effect}");
    }
}

#[test]
fn raster_filter_dialogs_preview_and_apply() {
    for (effect, key, value) in [
        ("artistic.cutout", "levels", "6"),
        ("artistic.neonGlow", "color", "#ff00ff"),
        ("artistic.posterEdges", "edgeThickness", "5"),
        ("artistic.smudgeStick", "strokeLength", "6"),
        ("artistic.sponge", "definition", "20"),
        ("artistic.watercolor", "texture", "2"),
        ("blur.radial", "method", "zoom"),
        ("blur.smart", "quality", "high"),
        ("blur.smart", "blurMode", "overlayEdge"),
        ("sharpen.unsharpMask", "amount", "120"),
        ("pixelate.colorHalftone", "channel1", "30"),
        ("pixelate.crystallize", "cellSize", "20"),
        ("pixelate.mezzotint", "type", "longLines"),
        ("pixelate.pointillize", "cellSize", "12"),
        ("sketch.basRelief", "lightDirection", "left"),
        ("sketch.charcoal", "thickness", "5"),
        ("sketch.conteCrayon", "texture", "brick"),
        ("sketch.conteCrayon", "foregroundLevel", "16"),
        ("sketch.halftonePattern", "patternType", "line"),
        ("sketch.photocopy", "darkness", "40"),
        ("sketch.stamp", "lightDarkBalance", "10"),
        ("sketch.plaster", "lightDirection", "bottomLeft"),
        ("texture.craquelure", "crackDepth", "9"),
        ("texture.grain", "grainType", "stippled"),
        ("texture.mosaicTiles", "tileSize", "20"),
        ("texture.patchwork", "relief", "20"),
        ("texture.stainedGlass", "borderThickness", "2"),
        ("texture.texturizer", "lightDirection", "bottomLeft"),
        // Drop Shadow chooses a color or a darkness; Warp's Bend is a number the dialog takes whole.
        ("stylize.dropShadow", "darkness", "40"),
        ("warp.arc", "bend", "-37"),
    ] {
        let mut app = app();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("effect.dialog", json!({"effect": effect})).unwrap();
        let v: Value = value.parse::<f64>().map_or_else(|_| json!(value), |n| json!(n));
        // A colour row takes the framework's `[r, g, b]`; the effect stores it as hex.
        let sent = match value.strip_prefix('#').and_then(|hex| u32::from_str_radix(hex, 16).ok()) {
            Some(n) => json!([16, 8, 0].map(|shift| f64::from((n >> shift) & 255) / 255.0)),
            None => v.clone(),
        };
        // Darkness takes input once it is chosen over Color: a greyed-out row refuses it.
        if key == "darkness" && effect == "stylize.dropShadow" {
            set(&mut app, "shade", json!("darkness"));
        }
        set(&mut app, key, sent);
        frame(&mut app, vec![]);
        assert!(app.session.in_interaction(), "{effect}: previews");
        assert!(confirm(&mut app));
        let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        assert_eq!(n.appearance.effects.len(), 1, "{effect}");
        // Whole numbers are stored as JSON integers: a number compares by value.
        let stored = &n.appearance.effects[0].params[key];
        assert_eq!(stored.as_f64().map_or_else(|| stored.clone(), Value::from), v, "{effect}");
    }
    // Fields follow the effect's documented order (its dialog's), not the alphabet.
    let doc = vectorcraft_effects::effect_info("blur.smart").unwrap().params;
    let ranks: Vec<usize> = ["radius", "threshold", "quality", "blurMode"].iter().map(|k| super::effect_params::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[3] < usize::MAX, "{ranks:?}");
    assert_eq!(super::effect_params::doc_rank(doc, "nope"), usize::MAX);
    let doc = vectorcraft_effects::effect_info("pixelate.colorHalftone").unwrap().params;
    let ranks: Vec<usize> =
        ["maxRadius", "channel1", "channel2", "channel3", "channel4"].iter().map(|k| super::effect_params::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[4] < usize::MAX, "{ranks:?}");
    let doc = vectorcraft_effects::effect_info("texture.texturizer").unwrap().params;
    let ranks: Vec<usize> =
        ["texture", "scaling", "relief", "lightDirection", "invert"].iter().map(|k| super::effect_params::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[4] < usize::MAX, "{ranks:?}");
}

#[test]
fn effect_dialog_applies_to_its_appearance_item() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.roughen", "item": 1})).unwrap();
    frame(&mut app, vec![]);
    assert!(confirm(&mut app));
    let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    assert_eq!(n.appearance.items[1].effects().len(), 1, "the effect goes to the stroke");
    assert!(n.appearance.effects.is_empty() && n.appearance.items[0].effects().is_empty());
}

#[test]
fn scribble_dialog_settings_presets_load_parameters() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "stylize.scribble"})).unwrap();
    let tight = vectorcraft_effects::scribble_settings().iter().find(|p| p.id == "tight").unwrap();
    for (k, v) in tight.params.as_object().unwrap() {
        set(&mut app, k, v.clone());
    }
    frame(&mut app, vec![]);
    assert_eq!(num(&app, "spacing"), 2.0, "the preset's values read as the preset");
    assert!(confirm(&mut app));
    assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["spacing"].as_f64(), Some(2.0));
}

#[test]
fn scribble_dialog_shows_the_angle_as_stored_and_each_variation() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "stylize.scribble"})).unwrap();
    set(&mut app, "angle", json!(270));
    frame(&mut app, vec![]);
    assert_eq!(field(&app, "angle"), Some(-90.0), "the field reads what is stored");
    assert!(confirm(&mut app));
    let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
    assert_eq!(n.appearance.effects[0].params["angle"].as_f64(), Some(-90.0));
    // Each variation follows the option it varies, as in the reference app's Scribble Options.
    let doc = vectorcraft_effects::effect_info("stylize.scribble").unwrap().params;
    let order = ["angle", "overlap", "overlapVariation", "strokeWidth", "curviness", "curvinessVariation", "spacing", "spacingVariation", "seed"];
    let ranks: Vec<usize> = order.iter().map(|k| super::effect_params::doc_rank(doc, k)).collect();
    assert!(ranks.windows(2).all(|w| w[0] < w[1]) && ranks[8] < usize::MAX, "{ranks:?}");
}

/// Object › Path › Split Into Grid opens on a grid the size of the selection, previews on the canvas and, on OK, splits
/// the path and adds the guides as one undo step.
#[test]
fn split_into_grid_opens_at_the_selections_size_and_previews() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 112, "height": 62})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    assert!(super::object_dialogs::open_from_menu(&mut app, "object.path.splitIntoGrid"));
    assert_eq!((num(&app, "totalWidth"), num(&app, "totalHeight"), flag(&app, "addGuides")), (112.0, 62.0, false));
    set_all(&mut app, [("columns", json!(3)), ("columnGutter", json!(8)), ("addGuides", json!(true)), ("preview", json!(true))]);
    frame(&mut app, vec![]);
    // The preview: 2 × 3 cells and the guides along their edges, not yet an undo step.
    assert_eq!((objects(&app), app.session.doc().unwrap().doc.ruler_guides().len()), (6, 10));
    assert!(app.session.in_interaction());
    assert!(confirm(&mut app));
    let st = app.session.doc().unwrap();
    assert_eq!((objects(&app), st.doc.ruler_guides().len(), st.history.undo.len()), (6, 10, undo + 1));
    assert!(!is_open(&app) && !app.session.in_interaction());
    // Cancel rolls a preview back.
    app.run("select.all", json!({})).unwrap();
    assert!(super::object_dialogs::open_from_menu(&mut app, "object.path.splitIntoGrid"));
    set(&mut app, "preview", json!(true));
    frame(&mut app, vec![]);
    assert_eq!(objects(&app), 4, "one grid over the six cells");
    cancel(&mut app);
    assert_eq!(objects(&app), 6);
}

#[test]
fn the_transform_effect_dialog_has_its_controls_previews_them_and_shows_them_again() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.transform"})).unwrap();
    settle(&mut app);
    assert!(app.session.in_interaction(), "the preview runs");
    // The options start at the catalog defaults (#885).
    assert_eq!((flag(&app, "objects"), flag(&app, "patterns"), flag(&app, "strokes")), (true, false, false));
    let effect = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].clone();
    // The reference point and Random update the preview.
    set_all(&mut app, [("moveH", json!(30)), ("copies", json!(3)), ("reference", json!(8))]);
    frame(&mut app, vec![]);
    assert_eq!((effect(&app).params["reference"].clone(), effect(&app).params["copies"].clone()), (json!(8), json!(3)));
    set(&mut app, "random", json!(true));
    frame(&mut app, vec![]);
    assert_eq!(effect(&app).params["random"], json!(true));
    // So does Scale Strokes & Effects.
    set(&mut app, "strokes", json!(true));
    frame(&mut app, vec![]);
    assert_eq!(effect(&app).params["strokes"], json!(true));
    assert!(confirm(&mut app));
    assert!(!app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.len(), 1);
    // Editing it from the Appearance panel shows the same values.
    app.run("effect.dialog", json!({"effect": "distort.transform", "index": 0})).unwrap();
    assert_eq!((num(&app, "reference"), num(&app, "copies"), num(&app, "moveH"), flag(&app, "random")), (8.0, 3.0, 30.0, true));
    assert_eq!((flag(&app, "objects"), flag(&app, "strokes")), (true, true));
}

/// Transform Each opened by its menu item, on Absolute when `absolute` (as it was last confirmed).
fn open_each(app: &mut VectorcraftApp, absolute: bool) {
    if absolute {
        let ctx = app.ui_ctx.clone().expect("a window app");
        let mut kept = craft::ui::patterns::form_dialogs::DialogFields::new();
        kept.set("relative", false);
        craft::ui::runtime::memory::set_saved_value(&ctx, &super::transform_each::EACH_KEY, Some(kept));
    }
    crate::menus::invoke(app, "object.transformEach", json!({}));
}

/// The Scale section's Relative and Absolute modes: Absolute shows the width and height the selected objects share, OK
/// sends `relative: false` with the sizes typed, and Relative keeps its own percentages.
#[test]
fn transform_each_switches_between_relative_and_absolute_scale() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, false);
    settle(&mut app);
    assert_eq!(fields(&app)["relative"], json!(true), "Relative at first");
    assert_eq!((num(&app, "scaleH"), num(&app, "scaleV")), (100.0, 100.0));
    set(&mut app, "relative", json!(false));
    assert_eq!((fields(&app)["relative"].clone(), num(&app, "absH"), num(&app, "absV")), (json!(false), 60.0, 20.0), "the size the rectangle has");
    // Exact sizes: 40 × 30 pt about the centre, as Absolute takes them.
    set_all(&mut app, [("absH", json!(40)), ("absV", json!(30))]);
    assert!(confirm(&mut app));
    let b = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1, b.y1), (20.0, 5.0, 60.0, 35.0));
    let (cmd, p) = app.session.journal.last().unwrap();
    assert_eq!((cmd.as_str(), &p["relative"], &p["scaleH"], &p["scaleV"]), ("object.transformEach", &json!(false), &json!(40.0), &json!(30.0)));
    // The dialog opens on Absolute again, with the size the objects have now; Relative keeps its own percentages.
    open_each(&mut app, false);
    settle(&mut app);
    assert_eq!(
        (fields(&app)["relative"].clone(), num(&app, "absH"), num(&app, "absV")),
        (json!(false), 40.0, 30.0),
        "Absolute is kept, at the size they have"
    );
    set(&mut app, "relative", json!(true));
    assert_eq!(
        (fields(&app)["relative"].clone(), num(&app, "scaleH"), num(&app, "scaleV")),
        (json!(true), 100.0, 100.0),
        "the percentages are the dialog's own"
    );
}

/// Switching to Absolute with Preview on shows the size the objects have without the preview, not the size it has scaled
/// them to.
#[test]
fn transform_each_switching_to_absolute_ignores_the_preview() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, false);
    settle(&mut app);
    set_all(&mut app, [("scaleH", json!(50)), ("preview", json!(true))]);
    frame(&mut app, vec![]);
    let width = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].geometric_bounds().unwrap().width();
    assert_eq!(width(&app), 30.0, "the preview scaled it");
    set(&mut app, "relative", json!(false));
    frame(&mut app, vec![]);
    assert_eq!((fields(&app)["relative"].clone(), num(&app, "absH"), num(&app, "absV")), (json!(false), 60.0, 20.0), "the size before the preview");
}

/// `ui.dialog.set` switching to Absolute shows the size the objects share, and the Absolute sizes a request sets after the
/// mode stay as set.
#[test]
fn transform_each_control_set_of_relative_keeps_the_sizes_after_it() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, false);
    set(&mut app, "relative", json!(false));
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (60.0, 20.0), "the size the rectangle has");
    set_all(&mut app, [("absH", json!(40)), ("absV", json!(30))]);
    assert_eq!((fields(&app)["relative"].clone(), num(&app, "absH"), num(&app, "absV")), (json!(false), 40.0, 30.0));
}

/// With the link on, switching to Absolute keeps the proportions the fields then show, and a scale set over the control
/// channel follows the link as a typed one does.
#[test]
fn transform_each_scale_link_follows_a_mode_switch_and_control_sets() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, false);
    set_all(&mut app, [("__constrain", json!(true)), ("relative", json!(false))]);
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (60.0, 20.0), "the size the rectangle has");
    // The link keeps the 3 : 1 the fields show: 30 wide is 10 high.
    set(&mut app, "absH", json!(30));
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (30.0, 10.0));
}

/// Absolute reads the size the selected objects share, not the bounds of the selection: two rectangles of the same size
/// show it, and a size typed makes every object that size.
#[test]
fn transform_each_absolute_shows_the_size_the_objects_share() {
    let mut app = app();
    for x in [10, 120] {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 60, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, false);
    set(&mut app, "relative", json!(false));
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (60.0, 20.0), "not the 170 pt the two together span");
}

/// Each object is measured by its own box, as the Transform panel and the Control bar show it: rotated circles of one size
/// read as that size, though their axis-aligned bounds differ by a hair (a rotated Bezier circle's bounding box runs
/// slightly larger than the circle).
#[test]
fn transform_each_absolute_measures_each_object_its_own_way() {
    let mut app = app();
    app.run("document.setUnits", json!({"units": "millimeters"})).unwrap();
    let size = 11.8 * 72.0 / 25.4;
    for (x, angle) in [(10.0, 0.0), (100.0, 30.0)] {
        let id = app.run("shape.ellipse", json!({"x": x, "y": 10, "width": size, "height": size})).unwrap()["id"].as_u64().unwrap();
        if angle != 0.0 {
            app.run("select.none", json!({})).unwrap();
            app.run("select.add", json!({"ids": [id]})).unwrap();
            app.run("object.rotate", json!({"angle": angle})).unwrap();
        }
    }
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, true);
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (size, size), "11.8 mm each");
}

/// Objects of different sizes read "Mixed", and a size typed sets every one of them to it.
#[test]
fn transform_each_absolute_reads_mixed_and_sets_every_object() {
    let mut app = app();
    let small = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap();
    let large = app.run("shape.rectangle", json!({"x": 120, "y": 10, "width": 40, "height": 80})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    // The dialog opens in Absolute, as it was last confirmed.
    open_each(&mut app, true);
    settle(&mut app);
    assert_eq!((field(&app, "absH"), field(&app, "absV")), (None, None), "no size they share");
    // A width set sets every object to it, and the side left Mixed keeps its size.
    set(&mut app, "absH", json!(50));
    assert!(confirm(&mut app));
    assert!(!is_open(&app));
    let size = |id: u64| app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert_eq!((size(small).width(), size(small).height()), (50.0, 20.0));
    assert_eq!((size(large).width(), size(large).height()), (50.0, 80.0));
}

/// A line's height of 0 reads Mixed, as the command refuses a scale of 0: the dialog can still move or rotate it, and
/// typing a height sets it.
#[test]
fn transform_each_absolute_reads_mixed_for_a_side_of_zero() {
    let mut app = app();
    let id = app.run("path.create", json!({"anchors": [{"x": 10, "y": 50}, {"x": 110, "y": 50}], "closed": false})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, true);
    assert_eq!((num(&app, "absH"), field(&app, "absV")), (100.0, None), "its width, and Mixed for its height");
    set(&mut app, "moveH", json!(10));
    assert!(confirm(&mut app));
    let b = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert_eq!((b.x0, b.y0, b.x1), (20.0, 50.0, 120.0));
}

/// `ui.dialog.set` takes an Absolute size as text in a unit ("10 mm"), as the dialog's fields do, and reaches the fields
/// while they read Mixed.
#[test]
fn transform_each_control_set_takes_a_size_in_units() {
    let mut app = app();
    let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 60, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("shape.rectangle", json!({"x": 120, "y": 10, "width": 40, "height": 80})).unwrap();
    app.run("select.all", json!({})).unwrap();
    open_each(&mut app, true);
    assert_eq!(field(&app, "absH"), None, "Mixed");
    set(&mut app, "absH", json!("10 mm"));
    assert!((num(&app, "absH") - 10.0 * 72.0 / 25.4).abs() < 1e-6, "the unit reader gives points: {}", num(&app, "absH"));
    assert!(confirm(&mut app));
    let b = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert!((b.width() - 10.0 * 72.0 / 25.4).abs() < 1e-6, "10 mm wide: {}", b.width());
}

/// Absolute measures the objects the command transforms: a compound path's member stands for its compound, so the size
/// shown is the compound's.
#[test]
fn transform_each_absolute_measures_the_compound_a_member_stands_for() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap();
    let inner = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.all", json!({})).unwrap();
    app.run("object.compoundPath.make", json!({})).unwrap();
    app.run("select.none", json!({})).unwrap();
    app.run("select.add", json!({"ids": [inner]})).unwrap();
    open_each(&mut app, true);
    assert_eq!((num(&app, "absH"), num(&app, "absV")), (100.0, 100.0), "the compound's size, not the member's");
}

/// The corners in the open dialog, and those of the first object's first effect.
fn free_distort_corners(app: &VectorcraftApp) -> (Value, Value) {
    let s = app.session.active().unwrap();
    let node = s.doc.node(s.selection.objects[0]).unwrap();
    (fields(app)["corners"].clone(), node.appearance.effects[0].params["corners"].clone())
}

/// Free Distort's corners, set on the dialog, preview and apply as one undo step. The drag of a handle and the arrow keys
/// that move it are the transform box's (its own trials).
#[test]
fn free_distort_drags_one_corner_and_previews_it() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    app.run("effect.dialog", json!({"effect": "distort.freeDistort"})).unwrap();
    settle(&mut app);
    let want = json!([[0.0, 0.0], [1.15, 0.1], [1.0, 1.0], [-0.01, 1.1]]);
    set(&mut app, "corners", want.clone());
    frame(&mut app, vec![]);
    assert_eq!(free_distort_corners(&app), (want.clone(), want.clone()), "the preview follows the corners");
    assert!(confirm(&mut app));
    assert!(!is_open(&app) && !app.session.in_interaction());
    let s = app.session.active().unwrap();
    assert_eq!(s.doc.node(s.selection.objects[0]).unwrap().appearance.effects[0].params["corners"], want);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before, "one undo step");
}

/// Reset puts the corners of an applied Free Distort back on the box, and is enabled only while they are off it; OK keeps
/// that.
#[test]
fn free_distort_reset_puts_the_corners_back() {
    let mut app = app();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("select.all", json!({})).unwrap();
    let moved = json!([[0.2, 0.1], [0.9, 0.0], [1.0, 3.0], [0.0, 1.0]]);
    app.run("effect.apply", json!({"effect": "distort.freeDistort", "params": {"corners": moved}})).unwrap();
    app.run("effect.dialog", json!({"effect": "distort.freeDistort", "index": 0})).unwrap();
    settle(&mut app);
    assert_eq!(fields(&app)["corners"], moved);
    assert!(can_reset(&app), "the corners are off the box");
    assert!(reset(&mut app));
    let rest = json!(super::free_distort::IDENTITY);
    assert_eq!(free_distort_corners(&app), (rest.clone(), rest.clone()));
    assert!(!can_reset(&app), "the corners are on the box");
    assert!(confirm(&mut app));
    let s = app.session.active().unwrap();
    let effects = &s.doc.node(s.selection.objects[0]).unwrap().appearance.effects;
    assert_eq!((effects.len(), effects[0].params["corners"].clone()), (1, rest));
}

/// The id of the last object drawn on the first layer: the group a grid dialog made.
fn last_group(app: &VectorcraftApp) -> vectorcraft_doc::Node {
    vectorcraft_doc::Node::clone(app.session.doc().unwrap().doc.layers[0].children().unwrap().last().unwrap())
}

#[test]
fn polar_grid_shared_canvas_click_defaults_units_counts_and_history() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut app = app();
    app.select_tool("polarGrid");
    app.run("tool.setOption", json!({"tool": "polarGrid", "values": {"concentric": 2, "radial": 3}})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    for kind in [PointerKind::Down, PointerKind::Up] {
        crate::canvas::dispatch(&mut app, &PointerEvent::new(kind, 30.0, 40.0), Default::default());
    }
    assert!(is_open(&app), "a canvas click opens the grid form");
    let f = fields(&app);
    assert_eq!((&f["width"], &f["height"]), (&json!(100.0), &json!(100.0)), "100 pt");
    assert_eq!((&f["concentric"], &f["radial"]), (&json!(2), &json!(3)));
    // 25.4 mm and 0.5 in, in points: the rows take numbers.
    set_all(&mut app, [("width", json!(72.0)), ("height", json!(36.0))]);
    assert!(confirm(&mut app));
    assert!(!is_open(&app));
    let group = last_group(&app);
    let children = group.children().unwrap();
    assert_eq!(children.len(), 6);
    assert_eq!(children.iter().filter(|n| n.path_data().unwrap().is_closed()).count(), 3);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let made = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, before);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.doc().unwrap().doc, made);
}

/// Part of gap G11: an Arc tool click opens its options at the pointer; OK draws an arc there.
#[test]
fn arc_click_opens_options_and_ok_draws() {
    use vectorcraft_doc::NodeKind;
    let mut app = app();
    open_tool_dialog(&mut app, "arc", json!({"x": 100.0, "y": 50.0}));
    let f = fields(&app);
    assert!(f.get("lengthX").is_some() && f.get("lengthY").is_some());
    assert!(confirm(&mut app));
    let n = last_group(&app);
    assert!(matches!(n.kind, NodeKind::Path { .. }));
    let pd = n.path_data().unwrap();
    let a0 = pd.subpaths[0].anchors[0].p;
    let a1 = pd.subpaths[0].anchors[1].p;
    assert!((a0.x - 100.0).abs() < 1e-6 && (a0.y - 50.0).abs() < 1e-6);
    assert!((a1.x - 200.0).abs() < 1e-6 && (a1.y - 150.0).abs() < 1e-6);
}

#[test]
fn polar_grid_shared_cancel_reopen_zero_counts_and_failed_creation() {
    let mut app = app();
    app.run("tool.setOption", json!({"tool": "polarGrid", "values": {"concentric": 2, "radial": 3}})).unwrap();
    let before = app.session.doc().unwrap().doc.clone();
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    set(&mut app, "concentric", json!(8));
    cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, before);
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    assert_eq!(fields(&app)["concentric"], json!(2), "Cancel discards and the dialog reopens from the saved values");
    set_all(&mut app, [("concentric", json!(0)), ("radial", json!(0))]);
    assert!(confirm(&mut app));
    let group = last_group(&app);
    assert_eq!(group.children().unwrap().len(), 1);
    assert!(group.children().unwrap()[0].path_data().unwrap().is_closed());
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    assert_eq!(fields(&app)["radial"], json!(0));
    cancel(&mut app);
    // A creation the locked layer refuses keeps the dialog open and saves nothing.
    let layer = app.session.doc().unwrap().doc.layers[0].id;
    app.run("layer.setProps", json!({"ids": [layer.0], "locked": true})).unwrap();
    let options = app.session.tool_options_of("polarGrid");
    let before = app.session.doc().unwrap().doc.clone();
    open_tool_dialog(&mut app, "polarGrid", json!({"x": 10, "y": 20}));
    set(&mut app, "radial", json!(9));
    confirm(&mut app);
    assert!(is_open(&app));
    assert_eq!(app.session.tool_options_of("polarGrid"), options);
    assert_eq!(app.session.doc().unwrap().doc, before);
}

/// Color Halftone's Max Radius is in pixels of the raster effects resolution whatever the document's units (#3260): in a
/// document in centimetres a value still commits as pixels, within the documented 4 to 127 px: the row refuses a value
/// outside them as it is set (the framework's `FieldRefusal::Rejects`) and keeps the one it had.
#[test]
fn color_halftones_max_radius_stays_in_pixels_whatever_the_units() {
    for (typed, applied) in [(10.0, Some(10.0)), (2.0, None), (500.0, None), (4.5, Some(4.5))] {
        let mut app = app();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("document.setup", json!({"units": "Centimeters"})).unwrap();
        app.run("select.all", json!({})).unwrap();
        app.run("effect.dialog", json!({"effect": "pixelate.colorHalftone"})).unwrap();
        settle(&mut app);
        let before = field(&app, "maxRadius");
        let (id, ctx) = (top(&app).0, app.ui_ctx.clone().expect("a window app"));
        let set = app.dialogs().expect("a window app").with(|host| host.set_json_field(&ctx, id, "maxRadius", json!(typed)));
        assert_eq!(set.is_ok(), applied.is_some(), "{typed}: {set:?}");
        assert_eq!(field(&app, "maxRadius"), applied.or(before), "{typed}");
    }
}

/// A real Key Color pick returns to its initiating tool when choosing its replacement fails.
#[test]
fn a_failed_tool_choice_restores_the_brush_key_color_initiator() {
    use egui::{Event, Modifiers, PointerButton};

    /// Presses the Brush Options pipette button, which arms the pick and selects the Eyedropper.
    fn arm(app: &mut VectorcraftApp) {
        let mut bounds = None;
        for _ in 0..12 {
            let output = frame(app, vec![]);
            bounds = output.platform_output.accesskit_update.as_ref().and_then(|update| {
                update
                    .nodes
                    .iter()
                    .find(|(_, node)| node.role() == Role::Button && node.label() == Some("Eyedropper: click the art to sample the key color"))
                    .and_then(|(_, node)| node.bounds())
            });
            if bounds.is_some() {
                break;
            }
        }
        let bounds = bounds.expect("the real Brush Options pipette button");
        let at = egui::pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
        frame(app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(at)]);
        for pressed in [true, false] {
            frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        }
        assert_eq!(app.session.tool_id(), "eyedropper");
        assert_eq!(app.session.tool_options()["brushKeyColor"], true);
        assert_eq!(app.session.tool_options()["stop"], "paintbrush");
    }

    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let art = app.run("shape.rectangle", json!({"x": 300, "y": 30, "width": 20, "height": 12})).unwrap()["id"].clone();
    let brush = "Failed tool choice fixture";
    app.run(
        "brush.new",
        json!({"type": "art", "name": brush, "ids": [art],
        "params": {"colorization": {"method": "hueShift", "key": vectorcraft_color::Color::BLACK}}}),
    )
    .unwrap();
    app.run("select.none", json!({})).unwrap();
    app.select_tool("paintbrush");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    app.canvas.worker_started = true;
    app.session.prefs.interface_language = "en".into();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    app.run("ui.brushOptions", json!({"name": brush})).unwrap();
    arm(&mut app);
    let unchanged = |app: &VectorcraftApp| {
        let state = app.session.active().unwrap();
        (
            json!({"uid": state.uid, "doc": state.doc, "selection": state.selection, "revision": state.revision,
            "undo": state.history.undo.len(), "redo": state.history.redo.len()}),
            app.session.paint.clone(),
        )
    };
    let before = unchanged(&app);
    let draft = fields(&app);
    let group_tools = app.ui.group_tool.clone();
    let drawing_tool = app.ui.touch_drawing_tool.clone();

    // The existing failure hook exercises a valid replacement, not an unknown tool id.
    app.fail_next_tool_choice = Some("key-color replacement failed".into());
    assert_eq!(app.try_select_tool("rectangle"), Err(String::from("key-color replacement failed")));
    assert!(app.fail_next_tool_choice.is_none());
    assert_eq!(app.session.tool_options_of("eyedropper")["brushKeyColor"], false);
    assert!(app.session.tool_options_of("eyedropper")["stop"].is_null());
    assert_eq!(fields(&app), draft);
    assert_eq!(app.ui.group_tool, group_tools);
    assert_eq!(app.ui.touch_drawing_tool, drawing_tool);
    assert_eq!(unchanged(&app), before);
    assert_eq!(app.session.tool_id(), "paintbrush", "a failed replacement must restore the pick's initiating tool");

    cancel(&mut app);
    assert!(!is_open(&app));
    assert_eq!(app.session.tool_id(), "paintbrush", "Cancel cannot return an already consumed pick twice");
    assert_eq!(unchanged(&app), before);
    app.run("ui.brushOptions", json!({"name": brush})).unwrap();
    arm(&mut app);
    app.try_select_tool("rectangle").unwrap();
    assert_eq!(app.session.tool_id(), "rectangle", "a successful replacement stays selected");
    assert_eq!(app.session.tool_options_of("eyedropper")["brushKeyColor"], false);
    assert!(app.session.tool_options_of("eyedropper")["stop"].is_null());
    assert_eq!(fields(&app), draft);
    assert_eq!(unchanged(&app), before);
    cancel(&mut app);
    assert!(!is_open(&app));
    assert_eq!(app.session.tool_id(), "rectangle", "Cancel must preserve the successful replacement");
    assert_eq!(unchanged(&app), before);
}
