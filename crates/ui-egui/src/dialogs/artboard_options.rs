//! Artboard Options (double-click with the Artboard tool, double-click the tool, or the Artboards
//! panel's options icon or menu): the artboard's fields, sizes parsed with units. A new size also
//! goes to the other selected artboards.

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

/// OK sets the artboard's fields. A Width or Height that differs from the artboard's also goes to
/// the other selected artboards (when this one is among them), each keeping its top-left corner,
/// all in one undo step.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height"] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    let index = p["index"].as_u64().and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
    let was = app.session.active().and_then(|st| st.doc.artboards.get(index)).map(|a| a.rect);
    let mut size = serde_json::Map::new();
    for (k, old) in [("width", was.map(|r| r.width())), ("height", was.map(|r| r.height()))] {
        if let (Some(v), Some(old)) = (p[k].as_f64(), old)
            && (v - old).abs() > 1e-6
        {
            size.insert(k.into(), json!(v));
        }
    }
    let mut entries = vec![p];
    if !size.is_empty() {
        for i in crate::panels::artboards::resized_with(app, index).into_iter().filter(|i| *i != index) {
            let mut q = size.clone();
            q.insert("index".into(), json!(i));
            entries.push(Value::Object(q));
        }
    }
    let common = json!({"moveArt": crate::panels::artboards::move_art(app)});
    let p = crate::panels::artboards::set_props_params(common, entries).ok_or("no artboard")?;
    run_and_close(app, "artboard.setProps", p)
}
