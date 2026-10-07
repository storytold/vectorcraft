//! Artboard Options (double-click with the Artboard tool): the artboard's fields, sizes parsed
//! with units.

use serde_json::{Value, json};

use super::{DialogSpec, run_and_close};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Artboard Options").into(), confirm, ..DialogSpec::FORM };

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut p = Value::Object(d.fields.clone());
    for k in ["x", "y", "width", "height"] {
        if d.fields.contains_key(k) {
            p[k] = json!(d.f64(k, 0.0));
        }
    }
    // Background: a colour (hex), or empty for none.
    if d.fields.contains_key("background") && d.str("background").trim().is_empty() {
        p["background"] = Value::Null;
    }
    run_and_close(app, "artboard.setProps", p)
}
