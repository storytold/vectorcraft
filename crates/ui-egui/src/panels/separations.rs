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

const PROOF_TARGETS: [(&str, &str); 7] = [
    ("workingCmyk", "Working CMYK"),
    ("legacyMacRgb", "Legacy Macintosh RGB (Gamma 1.8)"),
    ("srgb", "Internet Standard RGB (sRGB)"),
    ("monitorRgb", "Monitor RGB"),
    ("protanopia", "Color blindness – Protanopia-type"),
    ("deuteranopia", "Color blindness – Deuteranopia-type"),
    ("tritanopia", "Color blindness – Tritanopia-type"),
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
pub(crate) fn profile_dropdown(ui: &mut Ui, id: &str, current: &str, kind: ProfileKind, extra: Option<&str>) -> Option<String> {
    let names: Vec<String> =
        extra.map(str::to_string).into_iter().chain(cms::profiles().into_iter().filter(|p| p.kind == kind).map(|p| p.name)).collect();
    let refs: Vec<&str> = names.iter().map(|n| super::label_or_name(n, extra == Some(n.as_str()))).collect();
    widgets::dropdown_names(ui, id, super::label_or_name(current, extra == Some(current)), &refs, 210.0).and_then(|i| names.get(i).cloned())
}

/// The Color Settings section (Edit › Color Settings… / Assign Profile…), scrolled into view once
/// when one of those commands opened the panel ([`crate::state::UiState::color_settings_scroll`]).
fn settings_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let top = ui.cursor().top();
    settings_rows(app, ui);
    // Edit › Color Settings… / Assign Profile… opened the panel for this section: scroll it into
    // view once, as the plates and Proof Setup above it fill a short window (#6419). At once, not
    // animated: the panel has just opened. egui moves the scroll area on a later frame, so ask for
    // one rather than wait for the pointer to move.
    if app.ui.color_settings_scroll && !ui.is_sizing_pass() {
        let rect = egui::Rect::from_x_y_ranges(ui.max_rect().x_range(), top..=ui.min_rect().bottom().max(top));
        ui.scroll_to_rect_animation(rect, Some(egui::Align::TOP), egui::style::ScrollAnimation::none());
        ui.ctx().request_repaint();
        app.ui.color_settings_scroll = false;
    }
}

/// The section's rows: the RGB and CMYK working spaces, intent, black point compensation, Load
/// Profile… and Gamut Check, and the profile the active document is assigned.
fn settings_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
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
        if widgets::flat_button(ui, tl!("Load Profile…"), 110.0).clicked() {
            // Its file dialog, shown off the UI thread, loads the profile when it answers.
            crate::picks::button(app, |app| {
                let pick = crate::FilePick { filters: vec![crate::FilePick::filter("ICC Profiles", &["icc", "icm"])], ..Default::default() };
                if let Some(path) = crate::picks::open(app, &pick) {
                    run(app, "color.loadProfile", json!({ "path": path }));
                }
            });
        }
        if widgets::flat_button(ui, tl!("Gamut Check"), 100.0).clicked() {
            match app.run("color.gamutCheck", json!({})) {
                Ok(r) => {
                    let n = r["outOfGamut"].as_array().map_or(0, Vec::len);
                    app.ui.status = gamut_check_status(n);
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

/// The status bar's line after Gamut Check finds `n` colors outside the CMYK gamut.
pub(crate) fn gamut_check_status(n: usize) -> String {
    if n == 0 {
        "All colors are within the CMYK gamut".into()
    } else {
        crate::i18n::msg_n(n as u64, "{n} color is out of the CMYK gamut", "{n} colors are out of the CMYK gamut", &[])
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

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    /// The panel's Proof Setup list offers the targets of View › Proof Setup, in the same order and
    /// with the same labels, and each one is a target `view.proofSetup` accepts.
    #[test]
    fn proof_targets_match_the_proof_setup_menu() {
        let app = VectorcraftApp::new(Session::new(), Default::default());
        let menu: Vec<(String, String)> = crate::menus::menu_entries(&app)
            .into_iter()
            .filter(|e| e.path == ["View", "Proof Setup"] && e.command.as_deref() == Some("view.proofSetup"))
            .map(|e| (e.params["target"].as_str().unwrap_or_default().to_string(), e.label))
            .collect();
        let panel: Vec<(String, String)> = PROOF_TARGETS.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
        assert_eq!(menu, panel);
        assert!(panel.iter().any(|(id, _)| id == "tritanopia"));
        assert!(panel.iter().all(|(id, _)| cms::ProofTarget::parse(id).is_some_and(|t| t.id() == *id)));
    }

    /// #6419: Edit › Assign Profile… and Color Settings… show the panel scrolled to its Color
    /// Settings section, which a window shorter than the panel (a floated panel stops at 320
    /// points) otherwise shows only after scrolling past the plates and Proof Setup. Once: a
    /// scroll away afterwards stays.
    #[test]
    fn color_settings_and_assign_profile_scroll_the_color_settings_into_view() {
        const VIEW: f32 = 120.0;
        fn find(s: &egui::Shape, text: &str) -> Option<f32> {
            match s {
                egui::Shape::Text(t) if t.galley.text() == text => Some(t.pos.y),
                egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
                _ => None,
            }
        }
        // One frame of the panel in a scroll area VIEW points high, scrolled to `offset` when
        // given: where the section's Black Point Compensation box shows in the view.
        fn frame(ctx: &egui::Context, app: &mut VectorcraftApp, offset: Option<f32>) -> f32 {
            let mut top = 0.0;
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                top = ui.cursor().top();
                let mut area = egui::ScrollArea::vertical().id_salt("separations").max_height(VIEW);
                if let Some(y) = offset {
                    area = area.vertical_scroll_offset(y);
                }
                area.show(ui, |ui| show(app, ui));
            });
            out.textures_delta.clear();
            out.shapes.iter().find_map(|c| find(&c.shape, "Use Black Point Compensation")).unwrap() - top
        }
        let visible = |y: f32| (0.0..VIEW).contains(&y);
        for id in ["edit.assignProfile", "edit.colorSettings"] {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({})).unwrap();
            let ctx = egui::Context::default();
            let before = frame(&ctx, &mut app, None);
            assert!(!visible(before), "the section starts below the view: {before}");
            app.run(id, json!({})).unwrap();
            frame(&ctx, &mut app, None);
            assert!(!app.ui.color_settings_scroll);
            // egui moves a scroll area on the frame after the request and paints it moved on the
            // one after that; both ask for a repaint.
            assert!(ctx.has_requested_repaint());
            frame(&ctx, &mut app, None);
            assert!(ctx.has_requested_repaint());
            let shown = frame(&ctx, &mut app, None);
            assert!(visible(shown), "{id} scrolls the section into the view: {shown}");
            frame(&ctx, &mut app, Some(0.0));
            assert_eq!(frame(&ctx, &mut app, None), before, "scrolled back to the top, it stays there");
        }
    }
}
