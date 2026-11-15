//! Separations Preview panel: process plates C/M/Y/K plus spot plates with eye toggles (one visible
//! plate renders as greyscale ink coverage; Alt-click an eye to show only that plate), and the
//! colour-management controls behind Edit → Color Settings… / Assign Profile… and View → Proof
//! Setup (working spaces, color management policies, intent, black-point compensation, proof
//! target, gamut check).

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_color::cms::{self, Intent, Policy, ProfileKind};
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

/// The section's rows: the RGB and CMYK working spaces, intent, black point compensation, the RGB
/// and CMYK color management policies, Load Profile… and Gamut Check, and the profile the active
/// document is assigned.
fn settings_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    let st = cms::working_settings();
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
    // Color management policies: what opening a document does with a profile it is tagged with
    // that is not the working space.
    let policies: [(&str, &str, Policy, &[Policy]); 2] =
        [("rgbPolicy", tl!("RGB Policy:"), st.rgb_policy, &Policy::RGB), ("cmykPolicy", tl!("CMYK Policy:"), st.cmyk_policy, &Policy::CMYK)];
    for (key, label, current, choices) in policies {
        ui.horizontal(|ui| {
            ui.label(label);
            let labels: Vec<&str> = choices.iter().map(|p| policy_label(*p)).collect();
            if let Some(i) = widgets::dropdown_names(ui, key, policy_label(current), &labels, 210.0)
                && let Some(p) = choices.get(i)
            {
                run(app, "edit.colorSettings", json!({ key: p.id() }));
            }
        });
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
    if let Some(row) = app.session.active().map(|d| AssignRow::of(&d.doc)) {
        assign_rows(app, ui, &row);
    }
}

/// Edit › Assign Profile…'s three choices for the active document's color mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AssignChoice {
    /// Don't Color Manage This Document: no profile, so the colors show through the working space.
    None,
    /// The working space of the mode, assigned to the document as its profile.
    Working,
    /// Another profile of the mode's kind.
    Profile,
}

/// What the Assign rows show for a document: the profile of its color mode (`key` is the
/// `edit.assignProfile` param), the working space of that mode, and the other profiles of that kind.
#[derive(Debug)]
struct AssignRow {
    key: &'static str,
    mode: vectorcraft_doc::ColorMode,
    assigned: Option<String>,
    working: String,
    others: Vec<String>,
}

impl AssignRow {
    fn of(doc: &vectorcraft_doc::Document) -> Self {
        let (rgb, cmyk) = vectorcraft_engine::cmd::colormgmt::doc_profiles(doc);
        let st = cms::working_settings();
        let (key, kind, assigned, working) = match doc.color_mode {
            vectorcraft_doc::ColorMode::Rgb => ("rgb", ProfileKind::Rgb, rgb, st.rgb),
            vectorcraft_doc::ColorMode::Cmyk => ("cmyk", ProfileKind::Cmyk, cmyk, st.cmyk),
        };
        let working = cms::canonical_name(&working).to_string();
        let others = cms::profiles().into_iter().filter(|p| p.kind == kind && p.name != working).map(|p| p.name).collect();
        AssignRow { key, mode: doc.color_mode, assigned, working, others }
    }

    fn choice(&self) -> AssignChoice {
        match &self.assigned {
            None => AssignChoice::None,
            Some(n) if *n == self.working => AssignChoice::Working,
            Some(_) => AssignChoice::Profile,
        }
    }

    /// The `edit.assignProfile` params for `choice` (with `profile`, the profile picked for
    /// [`AssignChoice::Profile`], else the first of the list), or `None` when the document already
    /// has it.
    fn params(&self, choice: AssignChoice, profile: Option<&str>) -> Option<Value> {
        let v = match choice {
            AssignChoice::None => Value::Null,
            AssignChoice::Working => json!(self.working),
            AssignChoice::Profile => json!(profile.or(self.others.first().map(String::as_str))?),
        };
        (v != json!(self.assigned)).then(|| json!({ self.key: v }))
    }
}

/// The Assign rows: Don't Color Manage This Document, the working space of the document's color
/// mode, and Profile with a list of the other profiles of that kind. A pick runs `edit.assignProfile`.
fn assign_rows(app: &mut VectorcraftApp, ui: &mut Ui, row: &AssignRow) {
    let choice = row.choice();
    let mut pick = None;
    ui.label(tl!("Assign:"));
    if widgets::radio(ui, tl!("Don't Color Manage This Document"), choice == AssignChoice::None, true) {
        pick = row.params(AssignChoice::None, None);
    }
    let working = match row.mode {
        vectorcraft_doc::ColorMode::Rgb => tl!("Working RGB: {name}"),
        vectorcraft_doc::ColorMode::Cmyk => tl!("Working CMYK: {name}"),
    };
    let working = crate::i18n::fmt(working, &[("name", &row.working)]);
    if widgets::radio(ui, &working, choice == AssignChoice::Working, true) {
        pick = row.params(AssignChoice::Working, None);
    }
    ui.horizontal(|ui| {
        let on = choice == AssignChoice::Profile;
        let shown = if on { row.assigned.as_deref() } else { row.others.first().map(String::as_str) };
        if widgets::radio(ui, tl!("Profile:"), on, !row.others.is_empty()) {
            pick = row.params(AssignChoice::Profile, shown);
        }
        let refs: Vec<&str> = row.others.iter().map(String::as_str).collect();
        ui.add_enabled_ui(!refs.is_empty(), |ui| {
            if let Some(i) = widgets::dropdown_names(ui, "cms-assign", shown.unwrap_or_default(), &refs, 210.0) {
                pick = row.params(AssignChoice::Profile, refs.get(i).copied());
            }
        });
    });
    if let Some(p) = pick {
        run(app, "edit.assignProfile", p);
    }
}

/// The name of color management policy `p` in the UI language.
fn policy_label(p: Policy) -> &'static str {
    match p {
        Policy::Off => tl!("Off"),
        Policy::PreserveEmbedded => tl!("Preserve Embedded Profiles"),
        Policy::Convert => tl!("Convert to Working Space"),
        Policy::PreserveNumbers => tl!("Preserve Numbers"),
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

    /// Assign Profile's three choices: no profile, the working space assigned as the profile, or
    /// another profile; a choice the document already has runs nothing.
    #[test]
    fn assign_choices_give_the_profile_params_of_the_color_mode() {
        let row = |assigned: Option<&str>| AssignRow {
            key: "rgb",
            mode: vectorcraft_doc::ColorMode::Rgb,
            assigned: assigned.map(str::to_string),
            working: "W".into(),
            others: vec!["A".into(), "B".into()],
        };
        assert_eq!(row(None).choice(), AssignChoice::None);
        assert_eq!(row(Some("W")).choice(), AssignChoice::Working);
        assert_eq!(row(Some("B")).choice(), AssignChoice::Profile);
        assert_eq!(row(None).params(AssignChoice::None, None), None);
        assert_eq!(row(Some("B")).params(AssignChoice::None, None), Some(json!({"rgb": null})));
        assert_eq!(row(None).params(AssignChoice::Working, None), Some(json!({"rgb": "W"})));
        assert_eq!(row(Some("W")).params(AssignChoice::Working, None), None);
        assert_eq!(row(None).params(AssignChoice::Profile, None), Some(json!({"rgb": "A"})));
        assert_eq!(row(Some("W")).params(AssignChoice::Profile, Some("B")), Some(json!({"rgb": "B"})));
        assert_eq!(row(Some("B")).params(AssignChoice::Profile, Some("B")), None);
        let none = AssignRow { others: vec![], ..row(None) };
        assert_eq!(none.params(AssignChoice::Profile, None), None, "no other profile to pick");
    }

    /// The Assign rows follow the document's color mode: an RGB document lists the RGB working space
    /// and the RGB profiles and assigns its RGB profile, a CMYK document the CMYK ones, and the other
    /// model's profile stays as it was.
    #[test]
    fn assign_rows_follow_the_document_color_mode() {
        fn texts(s: &egui::Shape, out: &mut Vec<String>) {
            match s {
                egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({})).unwrap();
        for (mode, key, kind, other) in [("rgb", "rgb", ProfileKind::Rgb, "cmyk"), ("cmyk", "cmyk", ProfileKind::Cmyk, "rgb")] {
            app.run("edit.assignProfile", json!({"rgb": null, "cmyk": null})).unwrap();
            app.run("file.documentColorMode", json!({"mode": mode, "convert": false})).unwrap();
            let row = AssignRow::of(&app.session.active().unwrap().doc);
            assert_eq!((row.key, row.choice()), (key, AssignChoice::None));
            assert!(!row.others.is_empty() && !row.others.contains(&row.working));
            assert!(row.others.iter().chain([&row.working]).all(|n| cms::profile(n).is_some_and(|p| p.kind == kind)), "{row:?}");
            let ctx = egui::Context::default();
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| settings_rows(&mut app, ui));
            out.textures_delta.clear();
            let mut shown = vec![];
            out.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
            let working = format!("Working {}: {}", mode.to_uppercase(), row.working);
            assert!(shown.contains(&working) && shown.iter().any(|t| t == "Don't Color Manage This Document"), "{shown:?}");
            app.run("edit.assignProfile", row.params(AssignChoice::Profile, None).unwrap()).unwrap();
            let doc = &app.session.active().unwrap().doc;
            let (rgb, cmyk) = vectorcraft_engine::cmd::colormgmt::doc_profiles(doc);
            let (mine, theirs) = if key == "rgb" { (rgb, cmyk) } else { (cmyk, rgb) };
            assert_eq!((mine.as_deref(), theirs), (row.others.first().map(String::as_str), None), "{other} stays untagged");
        }
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
