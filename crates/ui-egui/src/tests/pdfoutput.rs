//! Save PDF › Output and Advanced: the colour conversion, profiles, output intent and real text
//! chosen in the dialog reach the written file, with nothing reported as not applied.

use serde_json::json;
use vectorcraft_color::cms::GENERIC_CMYK;

use crate::tests_svg::app;
use crate::{dialogs, theme};

/// One headless frame of the dialog layer.
fn frame(app: &mut crate::VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn output_and_advanced_choices_reach_the_pdf() {
    let (mut app, written) = app("/tmp/unused.pdf");
    app.run("text.create", json!({"x": 10, "y": 70, "text": "Proof"})).unwrap();
    app.run("ui.savePdfDialog", json!({"path": "/tmp/press.pdf"})).unwrap();
    let fields = &mut app.ui.dialog.as_mut().unwrap().fields;
    fields.insert("__section".into(), json!("Output"));
    fields.insert(
        "output".into(),
        json!({"conversion": "destination", "destination": GENERIC_CMYK, "profiles": "destination", "outputIntent": GENERIC_CMYK, "outputConditionId": "Proofing", "trapped": true}),
    );
    fields.insert("advanced".into(), json!({"outlineText": false}));
    fields.insert("compression".into(), json!({"compressText": false}));
    frame(&mut app);
    // PDF/A greys the output intent out (it carries its own); a name that isn't a profile shows.
    for (standard, intent) in [("pdfA2b", GENERIC_CMYK), ("none", "Registered Press")] {
        let d = app.ui.dialog.as_mut().unwrap();
        d.fields.insert("standard".into(), json!(standard));
        d.fields["output"]["outputIntent"] = json!(intent);
        frame(&mut app);
    }
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("standard".into(), json!("none"));
    d.fields["output"]["outputIntent"] = json!(GENERIC_CMYK);
    d.fields.insert("__section".into(), json!("Advanced"));
    frame(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.status.starts_with("Saved /tmp/press.pdf") && !app.ui.status.contains("note"), "{}", app.ui.status);
    let pdf = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert!(pdf.contains("/S/GTS_PDFX") && pdf.contains("/Trapped/True") && pdf.contains("/ICCBased"));
    assert!(pdf.contains("/ToUnicode"), "real text");
}
