//! Three-Point Arc Tool Options (double-click the Three-Point Arc tool, or `tool.options`):
//! Third Point Sets Direction, the tool's `direction` option. With it on, the third point gives the
//! direction the arc leaves an end in; with it off, the arc passes through the third point. OK
//! sets it with `tool.setOption {tool: "threePointArc", values: {direction}}`; it lasts across
//! tool switches and is saved with the preferences. `]` also switches it while an arc is drawn.

use serde_json::{Map, Value, json};
use vectorcraft_tools::three_point_arc::ID as TOOL;

use super::{DialogSpec, run_and_close};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Three-Point Arc Tool Options.
pub const KIND: &str = "threePointArcOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 280.0, ..DialogSpec::FORM };

fn heading(_: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(TOOL).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let on = app.session.tool_options_of(TOOL).get("direction").and_then(Value::as_bool).unwrap_or(false);
    let mut fields = Map::new();
    fields.insert("direction".into(), json!(on));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    json!({ "dialog": KIND })
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let on = d.bool("direction");
    if widgets::check(ui, tl!("Third Point Sets Direction"), on, true) {
        d.fields.insert("direction".into(), json!(!on));
    }
    false
}

/// OK: set the tool's `direction`.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    run_and_close(app, "tool.setOption", json!({ "tool": TOOL, "values": { "direction": d.bool("direction") } }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn double_clicking_the_tool_opens_its_options_and_ok_sets_the_direction() {
        let mut app = app();
        assert_eq!(app.session.tool_options_of(TOOL)["direction"], json!(false));
        let r = app.run("tool.options", json!({"tool": TOOL})).unwrap();
        assert_eq!(r["dialog"], json!(KIND));
        let text = painted(&mut app);
        for label in ["Three-Point Arc Tool Options", "Third Point Sets Direction", "OK", "Cancel"] {
            assert!(text.contains(label), "{label} in {text}");
        }
        let d = app.ui.dialog.as_mut().unwrap();
        assert!(!d.bool("direction"));
        d.fields.insert("direction".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!(app.session.tool_options_of(TOOL)["direction"], json!(true));
        // It lasts while another tool is chosen, and the dialog opens on it again.
        app.select_tool(TOOL);
        app.select_tool("selection");
        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("direction"));
    }
}
