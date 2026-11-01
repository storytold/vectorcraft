//! Object → Vector Halftone: the dot shape, the screen frequency and angle, mono (in one colour)
//! or CMYK screens, Invert, Clip to Art and Keep Original, previewed live on the canvas; OK keeps
//! the preview as one undo step (`object.vectorHalftone`).
//!
//! Fields: `shape`, `frequency`, `angle`, `mode`, `color`, `invert`, `clip`, `keepOriginal` (the
//! command's parameters) and `preview`. The live effect's dialog shows the same fields but Keep
//! Original ([`screen_fields`]).

use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::{DialogSpec, form};
use crate::panels::c32;
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Vector Halftone.
pub const KIND: &str = "vectorHalftone";

const CMD: &str = "object.vectorHalftone";

/// Width of the label column.
const LABEL_W: f32 = 84.0;

pub(super) const SPEC: DialogSpec = DialogSpec { heading: |_| tl!("Vector Halftone").into(), body, confirm, preview: true, ..DialogSpec::FORM };

/// The dialog's fields as it opens: the command's defaults, previewed.
pub fn fields() -> Value {
    json!({"shape": "circle", "frequency": 20, "angle": 45, "mode": "mono", "color": "#000000", "invert": false, "clip": true, "keepOriginal": false, "preview": true})
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    screen_fields(ui, d);
    form::check(ui, d, "keepOriginal", tl!("Keep Original"));
    let p = form::params(d);
    form::preview(app, ui, d, "Vector Halftone", CMD, p);
    false
}

/// The fields Object › Vector Halftone and the live effect (Effect › Color Effects › Vector
/// Halftone…) share: the dot shape, the screen frequency and angle, mono or CMYK screens, the dot
/// colour (or, with the effect's `fromArt`, the art's), Invert and Clip to Art.
pub(super) fn screen_fields(ui: &mut egui::Ui, d: &mut Dialog) {
    let shapes =
        [("circle", tl!("Circle")), ("ellipse", tl!("Ellipse")), ("square", tl!("Square")), ("diamond", tl!("Diamond")), ("line", tl!("Line"))];
    form::choice(ui, d, "shape", tl!("Shape"), (LABEL_W, 140.0), &shapes);
    form::slider_w(ui, d, ("frequency", tl!("Frequency"), LABEL_W), 1.0..=150.0, " lpi", &|x| c32(&Color::gray(1.0 - x)));
    form::slider_w(ui, d, ("angle", tl!("Angle"), LABEL_W), -90.0..=90.0, "°", &|x| c32(&Color::gray(0.2 + 0.6 * x)));
    form::choice(ui, d, "mode", tl!("Screens"), (LABEL_W, 140.0), &[("mono", tl!("Mono")), ("cmyk", tl!("CMYK"))]);
    if d.str("mode") != "cmyk" {
        if d.fields.contains_key("fromArt") {
            form::check(ui, d, "fromArt", tl!("Dot Color from Art"));
        }
        if !d.bool("fromArt") {
            widgets::label_row(ui, tl!("Color"), LABEL_W, |ui| {
                form::text(ui, d, "color", 120.0);
            });
        }
    }
    ui.add_space(4.0);
    for (key, label) in [("invert", tl!("Invert")), ("clip", tl!("Clip to Art"))] {
        form::check(ui, d, key, label);
    }
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, form::params(d))
}
