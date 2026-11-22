use serde_json::json;
use vectorcraft_engine::Session;

use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// #4919: Object › Plug-ins › Color Effects › Create Swatch Legend… opens a dialog with the
/// columns, chip size, gap and values; it previews, Cancel rolls it back, and OK draws the legend
/// with the values picked as one undo step. With swatches selected in the Swatches panel, it can
/// draw only those.
#[test]
fn create_swatch_legend_opens_its_dialog_previews_and_runs_on_ok() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 600, "height": 400})).unwrap();
    app.run("swatch.new", json!({"name": "Ink", "color": "#123456"})).unwrap();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Color Effects"] && e.label == "Create Swatch Legend…").expect("menu item");
    let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
    menus::invoke(&mut app, &cmd, p);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(super::KIND));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Create Swatch Legend", "Columns:", "Size:", "Gap:", "HEX", "CMYK", "Grayscale", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().map_or(0, Vec::len);
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(count(&app), 1, "previewed: one legend group");
    crate::dialogs::cancel(&mut app);
    assert_eq!(count(&app), 0, "rolled back");
    // OK with HEX and CMYK and 2 columns.
    menus::invoke(&mut app, &cmd, serde_json::Value::Null);
    let n = app.session.doc().unwrap().history.undo.len();
    let d = app.ui.dialog.as_mut().unwrap();
    for (k, v) in [("hex", json!(true)), ("cmyk", json!(true)), ("columns", json!(2))] {
        d.fields.insert(k.into(), v);
    }
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), n + 1);
    let st = app.session.doc().unwrap();
    let g = st.doc.layers[0].children().unwrap().last().unwrap();
    let label = g.children().unwrap().iter().find_map(|c| match &c.kind {
        vectorcraft_doc::NodeKind::Text(t) if t.plain_text().starts_with("Ink") => Some(t.plain_text()),
        _ => None,
    });
    let label = label.expect("Ink's label");
    assert!(label.contains("#123456") && label.contains("C="), "{label}");
    // Only the swatch selected in the Swatches panel.
    let ctx = egui::Context::default();
    crate::panels::set_pstate(&ctx, "swatch-selected", vec!["Ink".to_string()]);
    assert_eq!(super::selected_swatches(&app, &ctx), ["Ink"]);
    let mut d = crate::state::Dialog::new(super::KIND, super::fields());
    d.fields.insert("selectedOnly".into(), json!(true));
    d.fields.insert("names".into(), json!(["Ink"]));
    assert_eq!(super::params(&d)["names"], json!(["Ink"]));
}
