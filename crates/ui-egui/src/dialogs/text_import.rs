//! Text Import Options: placing a text file asks how to read it (platform and character set) and
//! what to clean up (line breaks inside paragraphs, blank lines between them, runs of spaces). OK
//! places it as area type (`file.place` with `text`).
//!
//! Fields: `platform` (`windows` | `mac`), `characterSet` (`unicode` | `ansi`),
//! `removeLineReturns`, `removeParagraphReturns`, `replaceSpaces` (on or off), `spaces` (how many
//! spaces make a tab, 2–100), and `__params` (the `file.place` params naming the file).

use serde_json::{Value, json};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Text Import Options.
pub const KIND: &str = "textImport";

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Text Import Options").into(), body, confirm, min_width: 340.0, ..DialogSpec::FORM };

/// The label column of the Encoding rows.
const LABEL: f32 = 104.0;

/// Is the file `p` names (`path` or `name`) a text file?
pub fn is_text(p: &Value) -> bool {
    let name = p.get("path").or_else(|| p.get("name")).and_then(Value::as_str).unwrap_or_default();
    vectorcraft_engine::cmd::fileio::TEXT_EXTS.contains(&vectorcraft_engine::cmd::fileio::extension(name).as_str())
}

/// Ask how to place the text file `params` (`file.place` params) names.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Value {
    let fields = json!({
        "platform": if cfg!(target_os = "macos") { "mac" } else { "windows" },
        "characterSet": "unicode",
        "removeLineReturns": false,
        "removeParagraphReturns": false,
        "replaceSpaces": false,
        "spaces": 3,
        "__params": params,
    });
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    json!({ "dialog": KIND })
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    widgets::subheader(ui, tl!("Encoding"));
    form::choice(ui, d, "platform", tl!("Platform:"), (LABEL, 180.0), &[("windows", "Windows"), ("mac", "Mac")]);
    form::choice(ui, d, "characterSet", tl!("Character Set:"), (LABEL, 180.0), &[("unicode", "Unicode"), ("ansi", "ANSI")]);
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Line Breaks"));
    for (key, label) in
        [("removeLineReturns", tl!("Join the lines of each paragraph")), ("removeParagraphReturns", tl!("Remove blank lines between paragraphs"))]
    {
        let on = d.bool(key);
        if widgets::check(ui, label, on, true) {
            d.fields.insert(key.into(), json!(!on));
        }
    }
    ui.add_space(10.0);
    widgets::subheader(ui, tl!("Spaces"));
    let on = d.bool("replaceSpaces");
    ui.horizontal(|ui| {
        if widgets::check(ui, tl!("Replace"), on, true) {
            d.fields.insert("replaceSpaces".into(), json!(!on));
        }
        ui.add_enabled_ui(on, |ui| {
            if let Some(n) = widgets::plain_field(ui, "text-import-spaces", d.f64("spaces", 3.0), "", 0, 44.0) {
                d.fields.insert("spaces".into(), json!(n.round().clamp(2.0, 100.0)));
            }
            widgets::dim_label(ui, tl!("or more spaces with a tab"));
        });
    });
    false
}

/// `file.place`'s `text` options from the fields.
fn options(d: &Dialog) -> Value {
    let mut o = json!({
        "platform": d.str("platform"),
        "characterSet": d.str("characterSet"),
        "removeLineReturns": d.bool("removeLineReturns"),
        "removeParagraphReturns": d.bool("removeParagraphReturns"),
    });
    if d.bool("replaceSpaces") {
        o["replaceSpaces"] = json!(d.f64("spaces", 3.0).round().clamp(2.0, 100.0) as u64);
    }
    o
}

/// Places the file with the options; closes first, so an error shows in the status bar.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = d.fields.get("__params").cloned().unwrap_or_else(|| json!({}));
    params["text"] = options(d);
    app.ui.dialog = None;
    crate::place::run_for_user(app, &params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::NodeKind;

    fn frame(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn placing_a_text_file_asks_for_the_import_options_then_places_area_type() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let text = b"one\r\ntwo\r\n\r\nthree\r\n";
        let file = json!({"name": "notes.txt", "dataBase64": vectorcraft_format::base64_encode(text)});
        let r = app.run("file.place", file).unwrap();
        assert_eq!(r["dialog"], KIND);
        let text = frame(&mut app);
        for label in [
            "Text Import Options",
            "Encoding",
            "Platform:",
            "Character Set:",
            "Unicode",
            "Line Breaks",
            "Join the lines of each paragraph",
            "Spaces",
            "OK",
            "Cancel",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("removeLineReturns".into(), json!(true));
        d.fields.insert("removeParagraphReturns".into(), json!(true));
        assert_eq!(options(d)["removeLineReturns"], true);
        let r = super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let st = app.session.doc().unwrap();
        let n = st.doc.node(vectorcraft_engine::doc::NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
        let NodeKind::Text(t) = &n.kind else { panic!("type") };
        assert_eq!(t.plain_text(), "one two\nthree", "one paragraph per block");
        assert_eq!(st.history.undo.len(), 1);
    }
}
