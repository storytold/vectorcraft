//! Separations Preview panel: process plates C/M/Y/K plus spot plates with eye toggles (one visible
//! plate renders as greyscale ink coverage; Alt-click an eye to show only that plate), and the
//! colour-management controls behind Edit → Color Settings… / Assign Profile… and View → Proof
//! Setup (working spaces, intent, black-point compensation, proof target, gamut check).

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, ProfileKind};
use vectorcraft_render::proof;

use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, dim_label, menu_item};

const PROOF_TARGETS: [(&str, &str); 6] = [
    ("workingCmyk", "Working CMYK"),
    ("legacyMacRgb", "Legacy Macintosh RGB (Gamma 1.8)"),
    ("srgb", "Internet Standard RGB (sRGB)"),
    ("monitorRgb", "Monitor RGB"),
    ("protanopia", "Color blindness – Protanopia-type"),
    ("deuteranopia", "Color blindness – Deuteranopia-type"),
];

fn run(app: &mut VectorcraftApp, id: &str, p: Value) {
    if let Err(e) = app.run(id, p) {
        app.ui.status = e;
    }
}

fn plates_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let v = proof::view();
    let on = v.separations.is_some();
    if widgets::check(ui, tl!("Overprint Preview (separations)"), on, app.session.active().is_some()) {
        run(app, "view.separationsPreview", json!({"on": !on}));
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else {
        dim_label(ui, tl!("No document open."));
        return;
    };
    let plates = proof::plates(&doc);
    let visible = |n: &str| v.separations.as_ref().is_none_or(|s| s.iter().any(|x| x == n));
    let alt = ui.input(|i| i.modifiers.alt);
    widgets::list_box(ui, |ui| {
        let all_on = plates.iter().all(|p| visible(&p.name));
        // CMYK composite row.
        ui.horizontal(|ui| {
            ui.add_enabled_ui(on, |ui| {
                if widgets::toggle_icon(ui, "eye", all_on, 18.0, tl!("Show all plates")) && on {
                    run(app, "view.separationsPreview", json!({"plates": plates.iter().map(|p| p.name.clone()).collect::<Vec<_>>()}));
                }
            });
            ui.label(egui::RichText::new(tl!("CMYK")).size(12.0).color(if on { t.text } else { t.text_dim }));
        });
        for p in &plates {
            ui.horizontal(|ui| {
                let vis = visible(&p.name);
                let mut clicked = false;
                ui.add_enabled_ui(on, |ui| {
                    clicked = widgets::toggle_icon(ui, "eye", vis, 18.0, tl!("Show/hide plate (Alt-click: only this plate)"));
                });
                if clicked && on {
                    let key = if alt { "only" } else { "toggle" };
                    run(app, "view.separationsPreview", json!({ key: p.name }));
                }
                let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
                let q = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
                ui.painter().rect_filled(r, 2.0, egui::Color32::from_rgb(q(p.rgb[0]), q(p.rgb[1]), q(p.rgb[2])));
                let label = if p.spot { crate::i18n::fmt(tl!("{name} (spot)"), &[("name", &p.name)]) } else { p.name.clone() };
                ui.label(egui::RichText::new(label).size(12.0).color(if on { t.text } else { t.text_dim }));
            });
        }
    });
}

/// A dropdown of the `kind` profiles after `extra` (a built-in entry, shown in the UI language);
/// profile names are shown as they are. Returns the chosen name.
fn profile_dropdown(ui: &mut Ui, id: &str, current: &str, kind: ProfileKind, extra: Option<&str>) -> Option<String> {
    let names: Vec<String> =
        extra.map(str::to_string).into_iter().chain(cms::profiles().into_iter().filter(|p| p.kind == kind).map(|p| p.name)).collect();
    let refs: Vec<&str> = names.iter().map(|n| super::label_or_name(n, extra == Some(n.as_str()))).collect();
    widgets::dropdown_names(ui, id, super::label_or_name(current, extra == Some(current)), &refs, 210.0).and_then(|i| names.get(i).cloned())
}

fn settings_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let st = cms::active_settings();
    widgets::section_header(ui, tl!("Color Settings"));
    ui.horizontal(|ui| {
        ui.label(tl!("RGB:"));
        if let Some(n) = profile_dropdown(ui, "cms-rgb", &st.rgb, ProfileKind::Rgb, None) {
            run(app, "edit.colorSettings", json!({"rgb": n}));
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("CMYK:"));
        if let Some(n) = profile_dropdown(ui, "cms-cmyk", &st.cmyk, ProfileKind::Cmyk, None) {
            run(app, "edit.colorSettings", json!({"cmyk": n}));
        }
    });
    ui.horizontal(|ui| {
        ui.label(tl!("Intent:"));
        let labels: Vec<&str> = Intent::ALL.iter().map(|i| i.label()).collect();
        if let Some(i) = widgets::dropdown(ui, "cms-intent", st.intent.label(), &labels, 170.0) {
            run(app, "edit.colorSettings", json!({"intent": Intent::ALL[i].id()}));
        }
    });
    if widgets::check(ui, tl!("Use Black Point Compensation"), st.bpc, true) {
        run(app, "edit.colorSettings", json!({"bpc": !st.bpc}));
    }
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Load Profile…"), 110.0).clicked()
            && let Some(path) = app
                .services
                .pick_open
                .as_mut()
                .and_then(|f| f(&crate::FilePick { filters: vec![("ICC Profiles", &["icc", "icm"])], ..Default::default() }))
        {
            run(app, "color.loadProfile", json!({ "path": path }));
        }
        if widgets::flat_button(ui, tl!("Gamut Check"), 100.0).clicked() {
            match app.run("color.gamutCheck", json!({})) {
                Ok(r) => {
                    let n = r["outOfGamut"].as_array().map_or(0, Vec::len);
                    app.ui.status =
                        if n == 0 { "All colours are within the CMYK gamut".into() } else { format!("{n} colour(s) are out of the CMYK gamut") };
                }
                Err(e) => app.ui.status = e,
            }
        }
    });
    if let Some(d) = app.session.active() {
        let (_, cmyk) = vectorcraft_engine::cmd::colormgmt::doc_profiles(&d.doc);
        const WORKING: &str = "Working CMYK (don't tag)";
        ui.horizontal(|ui| {
            ui.label(tl!("Assign:"));
            if let Some(n) = profile_dropdown(ui, "cms-assign", cmyk.as_deref().unwrap_or(WORKING), ProfileKind::Cmyk, Some(WORKING)) {
                let v = if n == WORKING { Value::Null } else { Value::String(n) };
                run(app, "edit.assignProfile", json!({ "cmyk": v }));
            }
        });
    }
}

fn proof_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let v = proof::view();
    widgets::section_header(ui, tl!("Proof Setup"));
    let cur = v.setup.target.id();
    // A target that isn't one of ours (a loaded profile) shows its name as it is.
    let label = PROOF_TARGETS.iter().find(|(id, _)| *id == cur).map_or(cur.as_str(), |(_, l)| tl!(*l));
    let labels: Vec<&str> = PROOF_TARGETS.iter().map(|(_, l)| tl!(*l)).collect();
    if let Some(i) = widgets::dropdown_names(ui, "proof-target", label, &labels, 230.0) {
        run(app, "view.proofSetup", json!({"target": PROOF_TARGETS[i].0}));
    }
    ui.horizontal(|ui| {
        if widgets::check(ui, tl!("Proof Colors"), v.proof_colors, true) {
            run(app, "view.proofColors", json!({"on": !v.proof_colors}));
        }
        if widgets::check(ui, tl!("Simulate Paper"), v.setup.simulate_paper, v.setup.target.is_cmyk()) {
            run(app, "view.proofSetup", json!({"simulatePaper": !v.setup.simulate_paper}));
        }
    });
    if widgets::check(ui, tl!("Overprint Preview"), v.overprint, true) {
        run(app, "view.overprintPreview", json!({"on": !v.overprint}));
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    plates_section(app, ui);
    widgets::divider(ui);
    proof_section(app, ui);
    widgets::divider(ui);
    settings_section(app, ui);
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let v = proof::view();
    if menu_item(ui, tl!("Overprint Preview"), true, v.overprint) {
        run(app, "view.overprintPreview", json!({}));
    }
    if menu_item(ui, tl!("Proof Colors"), true, v.proof_colors) {
        run(app, "view.proofColors", json!({}));
    }
    if menu_item(ui, tl!("Show All Plates"), v.separations.is_some(), false) {
        run(app, "view.separationsPreview", json!({"on": false}));
        run(app, "view.separationsPreview", json!({"on": true}));
    }
}
