//! Exporter plug-ins in File › Export › Export As: listed after the built-in formats, written
//! whole (Use Artboards is not theirs to apply), and their parameters asked for once the file is
//! picked. Each test installs exporters of its own: plug-ins are process-wide.

use serde_json::{Value, json};
use vectorcraft_plugins::wat::{self as tpl, Echo};

use super::tests_export::{app, frame, kind, names, set};
use super::*;

fn install(app: &mut VectorcraftApp, src: &str) {
    let wasm = wat::parse_str(src).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

#[test]
fn export_as_lists_exporters_and_writes_their_file_whole() {
    let (mut app, written) = app(2);
    install(&mut app, &tpl::returning(&tpl::exporter("test.ui.whole", r#"["tuw"]"#, "{}"), r#"{"dataBase64":"SGk=","warnings":["a note"]}"#));
    // After the built-in formats, with the other exporters installed.
    let listed: Vec<&str> = export_as::formats().iter().map(|f| f.id).collect();
    let first = listed.iter().position(|id| id.starts_with("plugin.")).expect("an exporter listed");
    let added = listed.get(first..).unwrap_or_default();
    assert!(added.iter().all(|id| id.starts_with("plugin.")) && added.contains(&"plugin.test.ui.whole"), "{listed:?}");
    // It isn't a filter: Object › Plug-ins' command says where it is.
    let e = app.run("plugin.dialog", json!({"id": "test.ui.whole"})).unwrap_err();
    assert!(e.contains("is an exporter (File › Export › Export As…)"), "{e}");

    app.run("file.exportAs", json!({"format": "plugin.test.ui.whole"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "plugin.test.ui.whole");
    // Use Artboards is greyed out and doesn't apply: one file of the whole document.
    set(&mut app, "useArtboards", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "an exporter without parameters writes at once");
    assert_eq!(names(&written), ["/out/Untitled-1.tuw"]);
    assert_eq!(written.borrow()[0].1, b"Hi");
    assert!(app.ui.status.contains("a note"), "{}", app.ui.status);
}

#[test]
fn an_exporter_with_parameters_asks_for_them_once_the_file_is_picked() {
    let (mut app, written) = app(1);
    let params = r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#;
    install(&mut app, &tpl::echo_exporter(&tpl::exporter("test.ui.params", r#"["tup"]"#, params), Echo::Params));
    // Asked for by its extension, as agents may.
    app.run("file.exportAs", json!({"format": "tup"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "plugin.test.ui.params");
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(plugin::EXPORT_KIND));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("__label"), d.fields.get("density")), ("Test test.ui.params".to_string(), Some(&json!(4.0))));
    assert!(written.borrow().is_empty(), "nothing written before Export");
    frame(&mut app);
    set(&mut app, "density", json!(7));
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&written), ["/out/Untitled-1.tup"]);
    let sent: Value = serde_json::from_slice(&written.borrow()[0].1).unwrap();
    assert_eq!(sent, json!({"density": 7.0, "_context": {"mode": "export", "extension": "tup", "name": "Untitled-1"}}));
}

/// `CRAFT_UI_PLUGIN_EXPORT_SHOT=<folder>` saves the pictures of the dialogs to that folder: Export As
/// with an exporter's format chosen, then the exporter's own dialog. The plug-in is a test one with
/// parameters a plotter's exporter could have, saved beside them (`plotter-test-exporter.wasm`) to
/// install in the app.
#[test]
fn export_as_and_exporter_dialog_pictures() {
    let Some(dir) = std::env::var_os("CRAFT_UI_PLUGIN_EXPORT_SHOT") else { return };
    let dir = std::path::PathBuf::from(dir);
    let (mut app, _) = app(1);
    let params = r#"{"speed":{"type":"number","min":1,"max":100,"default":40},"passes":{"type":"int","min":1,"max":10,"default":1},
        "optimizeTravel":{"type":"bool","default":true},"paper":{"type":"choice","options":["A4","A3","Letter"],"default":"A4"}}"#;
    let manifest = r#"{"id":"org.example.plotter","name":"Plotter (test exporter)","version":"1","kind":"exporter","extensions":["plt"],
        "description":"Writes the document as pen plotter commands.","params":PARAMS}"#
        .replace("PARAMS", params);
    let src = tpl::echo_exporter(&manifest, Echo::Params);
    std::fs::write(dir.join("plotter-test-exporter.wasm"), wat::parse_str(&src).unwrap()).unwrap();
    install(&mut app, &src);
    app.run("file.exportAs", json!({"format": "plugin.org.example.plotter"})).unwrap();
    let shot = |app: VectorcraftApp, name: &str| -> VectorcraftApp {
        let mut ready = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(720.0, 520.0)).wgpu().build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !ready {
                    crate::theme::install_fonts(ui.ctx());
                    crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                    ready = true;
                    return;
                }
                show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(6);
        h.render().unwrap().save(dir.join(name)).unwrap();
        h.into_state()
    };
    app = shot(app, "export-as-exporter.png");
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some(plugin::EXPORT_KIND));
    shot(app, "exporter-options.png");
}

/// A plug-in that declines keeps nothing written and says why.
#[test]
fn a_declining_exporter_writes_nothing() {
    let (mut app, written) = app(1);
    install(&mut app, &tpl::returning(&tpl::exporter("test.ui.declines", r#"["tud"]"#, "{}"), r#"{"error":"Nothing to plot."}"#));
    app.run("file.exportAs", json!({"format": "plugin.test.ui.declines"})).unwrap();
    let r = confirm(&mut app);
    assert!(written.borrow().is_empty());
    let said = r.err().unwrap_or_default() + &app.ui.status;
    assert!(said.contains("Nothing to plot."), "{said}");
}
