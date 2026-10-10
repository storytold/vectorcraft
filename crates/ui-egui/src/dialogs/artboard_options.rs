//! Artboard Options (double-click with the Artboard tool, or double-click the tool): the
//! artboard's fields, sizes parsed with units.

use serde_json::{Value, json};

use super::{DialogSpec, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Artboard Options").into(), confirm, ..DialogSpec::FORM };

/// The dialog kind of Artboard Options.
pub const KIND: &str = "artboardOptions";

/// Open Artboard Options for the active artboard: the Artboard tool's while it is in use, else the
/// window's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let index = if app.session.tool_id() == "artboard" {
        app.session.tool_options()["active"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0)
    } else {
        app.view().map_or(0, |v| v.artboard)
    };
    let st = app.session.active().ok_or("no document")?;
    let a = st.doc.artboards.get(index).ok_or("no artboard")?;
    let r = a.rect;
    let fields = json!({"index": index, "name": a.name, "x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    Ok(json!({ "dialog": KIND }))
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height"] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    p["moveArt"] = json!(crate::panels::artboards::move_art(app));
    run_and_close(app, "artboard.setProps", p)
}
