//! Object → Vector Halftone in the menus.

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus};

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

#[test]
fn the_halftone_menu_item_opens_a_previewing_dialog() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    let id = app.session.active().unwrap().selection.objects[0];
    menus::invoke(&mut app, "object.vectorHalftone", json!({}));
    let dialog = app.effect_dialogs.halftone.as_ref().map(|o| o.id).expect("the halftone dialog opens");
    let (ctx, host) = (app.ui_ctx.clone().unwrap(), app.dialogs().unwrap());
    dialogs::host::pump(&mut app, &ctx);
    assert!(app.session.in_interaction(), "the preview runs");
    assert!(app.session.active().unwrap().doc.node(id).is_none(), "the preview shows the halftone in place of the art");
    // A control channel method, not an engine command.
    let (req, _) = craft::automation::ControlRequest::new("ui.dialog.set", json!({"field": "mode", "value": "cmyk"}));
    let craft::automation::Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("ui.dialog.set answers at once") };
    assert_eq!(r["ok"], true, "{r}");
    dialogs::host::pump(&mut app, &ctx);
    let undo = app.session.doc().unwrap().history.undo.len();
    host.with(|h| h.confirm_by_id(&ctx, dialog));
    dialogs::host::pump(&mut app, &ctx);
    assert!(!app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.node(st.selection.objects[0]).unwrap().name.as_deref(), Some("Vector Halftone"));
}
