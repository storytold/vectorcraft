//! Save PDF › Output and Advanced: the colour conversion, profiles, output intent and real text
//! chosen in the dialog reach the written file, with nothing reported as not applied.

use serde_json::json;
use vectorcraft_color::cms::GENERIC_CMYK;

use super::pathtype::{confirm, frame, set_field, shown};
use crate::tests_svg::app;

#[test]
fn output_and_advanced_choices_reach_the_pdf() {
    let (mut app, written) = app("/tmp/unused.pdf");
    app.run("text.create", json!({"x": 10, "y": 70, "text": "Proof"})).unwrap();
    let ctx = shown(&mut app);
    app.run("ui.savePdfDialog", json!({"path": "/tmp/press.pdf"})).unwrap();
    frame(&mut app, &ctx);
    // The section ids are the framework dialog's `json_fields` ones.
    set_field(&app, &ctx, "section", json!("output"));
    set_field(
        &app,
        &ctx,
        "output",
        json!({"conversion": "destination", "destination": GENERIC_CMYK, "profiles": "destination", "outputIntent": GENERIC_CMYK, "outputConditionId": "Proofing", "trapped": true}),
    );
    set_field(&app, &ctx, "advanced", json!({"outlineText": false}));
    set_field(&app, &ctx, "compression", json!({"compressText": false}));
    set_field(&app, &ctx, "standard", json!("none"));
    set_field(&app, &ctx, "section", json!("advanced"));
    frame(&mut app, &ctx);
    confirm(&mut app, &ctx);
    assert!(app.ui.status.starts_with("Saved /tmp/press.pdf") && !app.ui.status.contains("note"), "{}", app.ui.status);
    let pdf = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert!(pdf.contains("/S/GTS_PDFX") && pdf.contains("/Trapped/True") && pdf.contains("/ICCBased"));
    assert!(pdf.contains("/ToUnicode"), "real text");
}
