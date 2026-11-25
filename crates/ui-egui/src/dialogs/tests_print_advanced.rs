//! The Print dialog's Advanced and Color Management options and the Print Tiling tool's placement
//! in it: they label and save with the document.

use serde_json::json;
use vectorcraft_engine::Session;

use super::tests_print::{confirm, fields, pump, set};
use crate::VectorcraftApp;

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 120, "height": 90})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app
}

#[test]
fn advanced_options_label_and_save_with_the_document() {
    let mut app = app();
    app.run("print.tiling.set", json!({"origin": [-20, -10]})).unwrap();
    app.run("file.print", json!({})).unwrap();
    pump(&mut app);
    assert_eq!(fields(&app)["tileOrigin"]["placed"], true, "the tool's placement comes in");
    set(&mut app, "advanced.printAsBitmap", json!(true));
    set(&mut app, "advanced.overprints", json!("simulate"));
    set(&mut app, "advanced.flattenerPreset", json!("High Resolution"));
    set(&mut app, "color.profile", json!(vectorcraft_color::cms::GENERIC_CMYK));
    pump(&mut app);
    let mut changed = vec![json!({"option": "advanced.overprints", "value": "simulate"})];
    super::print::labelled(&mut changed);
    assert_ne!(changed[0]["value"], "simulate", "the option's value is worded");
    assert!(super::print::option_label("advanced.printAsBitmap").starts_with("Advanced"));
    // Done keeps them with the document.
    set(&mut app, "choice", json!("done"));
    confirm(&mut app);
    let saved = app.session.execute("print.setup", &json!({})).unwrap()["settings"].clone();
    assert_eq!(saved["advanced"], json!({"printAsBitmap": true, "overprints": "simulate", "flattenerPreset": "High Resolution"}));
    assert_eq!(saved["color"]["profile"], vectorcraft_color::cms::GENERIC_CMYK);
    assert_eq!(saved["tileOrigin"]["placed"], true);
}
