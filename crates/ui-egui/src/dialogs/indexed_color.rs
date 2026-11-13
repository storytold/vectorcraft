//! Edit → Edit Colors → Convert to Indexed Color: the palette (reduction and colors) and the
//! dither of the selection's embedded images, previewed live on the canvas; OK keeps the preview
//! as one undo step (`edit.colors.toIndexed`).
//!
//! Fields: `reduction`, `colors`, `dither`, `ditherAmount` and `preview`.

use serde_json::{Value, json};
use vectorcraft_render::encode::quantize::{Dither, Reduction};

use super::{DialogSpec, form};
use crate::state::Dialog;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Convert to Indexed Color.
pub const KIND: &str = "indexedColor";

const CMD: &str = "edit.colors.toIndexed";

/// The reductions besides those of a PNG-8 export, as (value, label).
const MORE_REDUCTIONS: [(&str, &str); 2] = [("uniform", "Uniform"), ("document", "Document Swatches")];

/// Label column and dropdown widths.
const WIDTHS: (f32, f32) = (110.0, 160.0);

pub(super) const SPEC: DialogSpec =
    DialogSpec { heading: |_| tl!("Convert to Indexed Color").into(), body, confirm, preview: true, min_width: 320.0, ..DialogSpec::FORM };

/// Open the dialog with the defaults of `edit.colors.toIndexed`.
pub fn open(app: &mut VectorcraftApp) {
    let fields = json!({"reduction": "selective", "colors": 256, "dither": "diffusion", "ditherAmount": 100, "preview": true});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// Whether reduction `r` takes a number of colors.
fn takes_colors(r: &str) -> bool {
    !matches!(r, "blackWhite" | "document")
}

/// The fewest colors reduction `r` takes.
fn least_colors(r: &str) -> f64 {
    if r == "uniform" { 9.0 } else { 2.0 }
}

/// `edit.colors.toIndexed` parameters: the reduction, its colors when it takes them, and the dither.
fn params(d: &Dialog) -> Value {
    let reduction = d.str("reduction");
    let dither = d.str("dither");
    let mut p = json!({"reduction": reduction, "dither": dither});
    if takes_colors(&reduction) {
        p["colors"] = json!(d.f64("colors", 256.0).round().clamp(least_colors(&reduction), 256.0) as u16);
    }
    if dither != Dither::None.id() {
        p["ditherAmount"] = json!(d.f64("ditherAmount", 100.0).round().clamp(0.0, 100.0));
    }
    p
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let reductions: Vec<(&str, &str)> = Reduction::ALL.iter().map(|r| (r.id(), r.label())).chain(MORE_REDUCTIONS).collect();
    form::choice(ui, d, "reduction", tl!("Reduction:"), WIDTHS, &reductions);
    let reduction = d.str("reduction");
    if takes_colors(&reduction) {
        // The field shows the number the command gets: at least the reduction's fewest.
        let least = least_colors(&reduction);
        let n = d.f64("colors", 256.0).round().clamp(least, 256.0);
        d.fields.insert("colors".into(), json!(n as u16));
        widgets::label_row(ui, tl!("Colors:"), WIDTHS.0, |ui| {
            if let Some(n) = widgets::range_field(ui, "indexed-colors", n, least..=256.0, "", 0, 60.0) {
                d.fields.insert("colors".into(), json!(n.round() as u16));
            }
        });
    }
    let dithers: Vec<(&str, &str)> = Dither::ALL.iter().map(|x| (x.id(), x.label())).collect();
    form::choice(ui, d, "dither", tl!("Dither:"), WIDTHS, &dithers);
    if d.str("dither") != Dither::None.id() {
        widgets::label_row(ui, tl!("Amount:"), WIDTHS.0, |ui| {
            if let Some(n) = widgets::range_field(ui, "indexed-dither-amount", d.f64("ditherAmount", 100.0), 0.0..=100.0, "%", 0, 60.0) {
                d.fields.insert("ditherAmount".into(), json!(n.round()));
            }
        });
    }
    let p = params(d);
    form::preview(app, ui, d, "Convert to Indexed Color", CMD, p);
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    form::commit_preview(app, CMD, params(d))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn commits_the_chosen_palette_as_one_step() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let mut png = Vec::new();
        let img = image::RgbaImage::from_raw(2, 1, vec![200, 30, 90, 255, 20, 40, 220, 255]).unwrap();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let id = app
            .session
            .edit("Place", |d, sel| {
                d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png));
                let id = d.alloc_id();
                let im = vectorcraft_doc::ImageObject {
                    key: "pic".into(),
                    width: 2,
                    height: 1,
                    xf: Default::default(),
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                };
                d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(im))))?;
                sel.set([id]);
                Ok(id)
            })
            .unwrap();
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("ui.indexedColorDialog", json!({})).unwrap();
        let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
        fields.insert("reduction".into(), json!("blackWhite"));
        fields.insert("dither".into(), json!("none"));
        super::super::confirm(&mut app).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let vectorcraft_doc::NodeKind::Image(im) = &st.doc.node(id).unwrap().kind else { panic!("not an image") };
        let blob = &st.doc.images[&im.key];
        assert!(blob.indexed_png());
        assert_eq!(blob.rgba().unwrap().2, [0, 0, 0, 255, 0, 0, 0, 255], "black and white only");
    }
}
