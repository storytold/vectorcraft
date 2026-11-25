use super::*;
use crate::canvas::grid::{document_grid, grid_look, pixel_grid};
use craft::ui::canvas_controls::GridLine;
use serde_json::json;
use vectorcraft_engine::Session;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.guides = true;
    app.ui.view.grid = true;
    app
}

/// The theme's tokens, as a pass of the app's interface resolves them.
fn tokens() -> Tokens {
    let mut tokens = None;
    crate::testing::ui_context().run_ui(Default::default(), |ui| tokens = Some(Tokens::clone(Env::of(ui).tokens.tokens()))).textures_delta.clear();
    tokens.expect("the pass ran")
}

fn shown_of(app: &VectorcraftApp) -> Option<Shown> {
    shown(app, app.session.active().unwrap(), &tokens())
}

fn color_at(shown: &Shown, position: f64) -> Option<Rgba> {
    shown.0.iter().find(|g| g.position == position).and_then(|g| g.color)
}

/// An artboard's guide label colors its own guides only; a canvas guide and an unlabelled
/// artboard's guide keep the guide color preference.
#[test]
fn artboard_guide_label_colors_only_its_own_guides() {
    let mut app = app();
    app.session.execute("artboard.new", &json!({})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 0, "guideLabel": "Red"})).unwrap();
    for (pos, artboard) in [(100, json!(0)), (520, json!(1))] {
        app.session.execute("guide.add", &json!({"vertical": true, "pos": pos, "artboard": artboard})).unwrap();
    }
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 150})).unwrap();
    let s = shown_of(&app).unwrap();
    let pref = guide_color(&app.session.prefs, &tokens());
    let red = color_at(&s, 100.0).unwrap();
    assert_ne!(red, pref, "the labelled artboard's guide has the label colour");
    assert_eq!(color_at(&s, 520.0), Some(pref), "an artboard without a label");
    assert_eq!(color_at(&s, 150.0), Some(pref), "a canvas guide");
    app.ui.view.guides = false;
    assert!(shown_of(&app).is_none(), "Hide Guides hides labelled guides too");
}

/// The selected guides and the key guide go to the control by key; the document half stays with
/// the tools.
#[test]
fn shown_carries_the_selected_guides_and_the_key_guide() {
    let mut app = app();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    app.session.execute("guide.add", &json!({"vertical": false, "pos": 50})).unwrap();
    let (_, selected, key) = shown_of(&app).unwrap();
    assert!(selected.is_empty() && key.is_none());
    app.session.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    let (guides, selected, _) = shown_of(&app).unwrap();
    assert_eq!(selected.len(), 1);
    assert!(guides.iter().any(|g| g.key == selected[0] && g.position == 100.0));
}

/// The grid and the guides take their preferences: the Dots style is a dotted guide and a grid of
/// crossings, the colours are the preference colours, and Grids In Back is the preference.
#[test]
fn grid_and_guides_take_their_preferences() {
    let mut app = app();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    let (_, in_back) = document_grid(&app, app.session.active().unwrap()).unwrap();
    assert_eq!(in_back, app.session.prefs.grids_in_back);
    assert_eq!(shown_of(&app).unwrap().0[0].line, GuideLine::Solid);
    let (line, grid_color) = grid_look(&app.session.prefs);
    assert_eq!(line, GridLine::Solid);
    app.session.execute("prefs.set", &json!({"key": "gridStyle", "value": "dots"})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "guideStyle", "value": "dots"})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "gridColor", "value": "#112233"})).unwrap();
    app.session.execute("prefs.set", &json!({"key": "guideColor", "value": "#445566"})).unwrap();
    let (line, color) = grid_look(&app.session.prefs);
    assert_eq!(line, GridLine::Crossings);
    assert_ne!(color, grid_color);
    let s = shown_of(&app).unwrap();
    assert_eq!(s.0[0].line, GuideLine::Dots);
    assert_eq!(s.0[0].color, Some(Rgba::rgb(0x44, 0x55, 0x66)));
}

/// Trim View and Presentation Mode hide the guides and the grid; the flags stay on.
#[test]
fn trim_view_hides_the_guides_and_the_grid() {
    let mut app = app();
    app.session.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    assert!(shown_of(&app).is_some() && document_grid(&app, app.session.active().unwrap()).is_some());
    app.ui.view.trim_view = true;
    assert!(shown_of(&app).is_none() && document_grid(&app, app.session.active().unwrap()).is_none());
    assert!(app.ui.view.guides && app.ui.view.grid, "the flags are kept");
    app.ui.view.trim_view = false;
    app.ui.screen_mode = 3;
    assert!(shown_of(&app).is_none() && document_grid(&app, app.session.active().unwrap()).is_none());
}

/// The pixel grid shows only in Pixel Preview with the preference on; the control applies the
/// 600% threshold.
#[test]
fn pixel_grid_shows_in_pixel_preview_from_600_percent() {
    let mut app = app();
    assert!(pixel_grid(&app).is_none(), "not in Pixel Preview");
    app.ui.view.pixel_preview = true;
    assert!(pixel_grid(&app).is_some());
    app.session.execute("prefs.set", &json!({"key": "showPixelGrid", "value": false})).unwrap();
    assert!(pixel_grid(&app).is_none(), "the preference is off");
}
