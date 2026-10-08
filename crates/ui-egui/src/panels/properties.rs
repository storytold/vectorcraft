//! The Properties panel: context-sensitive sections like Illustrator's.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{NodeKind, Unit};

use super::{first_selected, pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, dim_label, divider, section_header};
use crate::{VectorcraftApp, icons};

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        dim_label(ui, tl!("No document open"));
        return;
    };
    let n_sel = st.selection.len();
    let first = first_selected(app);
    let label = match (&first, n_sel) {
        (None, _) => tl!("Document").to_string(),
        (_, n) if n > 1 => crate::i18n::tn(n as u64, "{n} Object", "{n} Objects"),
        (Some(n), _) => tl!(n.kind_label()).to_string(),
    };
    ui.label(egui::RichText::new(label).size(11.5).color(t.text_dim));
    ui.add_space(4.0);
    if first.is_none() {
        document_sections(app, ui);
        return;
    }
    transform_section(app, ui);
    divider(ui);
    if n_sel == 1 && matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Image(_))) {
        image_section(app, ui);
        divider(ui);
    }
    if matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Text(_))) {
        type_sections(app, ui);
        divider(ui);
    }
    appearance_section(app, ui);
    divider(ui);
    section_header(ui, tl!("Align"));
    ui.horizontal(|ui| {
        for (icon, tip, p) in [
            ("align-start-vertical", tl!("Horizontal Align Left"), json!({"horizontal": "left"})),
            ("align-center-vertical", tl!("Horizontal Align Center"), json!({"horizontal": "center"})),
            ("align-end-vertical", tl!("Horizontal Align Right"), json!({"horizontal": "right"})),
            ("align-start-horizontal", tl!("Vertical Align Top"), json!({"vertical": "top"})),
            ("align-center-horizontal", tl!("Vertical Align Center"), json!({"vertical": "center"})),
            ("align-end-horizontal", tl!("Vertical Align Bottom"), json!({"vertical": "bottom"})),
        ] {
            if widgets::icon_button(ui, icon, tip, false, 26.0).clicked() {
                let mut p = p;
                if n_sel == 1 {
                    p["to"] = json!("artboard");
                }
                app.run("object.align", p).ok();
            }
        }
    });
    if n_sel > 1 {
        divider(ui);
        section_header(ui, tl!("Pathfinder"));
        ui.horizontal(|ui| {
            for (icon, tip, op) in [
                ("squares-unite", tl!("Unite"), "unite"),
                ("squares-subtract", tl!("Minus Front"), "minusFront"),
                ("squares-intersect", tl!("Intersect"), "intersect"),
                ("squares-exclude", tl!("Exclude"), "exclude"),
            ] {
                if widgets::icon_button(ui, icon, tip, false, 28.0).clicked() {
                    app.run(&format!("object.pathfinder.{op}"), json!({})).ok();
                }
            }
            if widgets::icon_button(ui, "ellipsis", tl!("More Pathfinder options"), false, 28.0).clicked() {
                app.ui.open_panel = Some("pathfinder".into());
            }
        });
    }
    divider(ui);
    section_header(ui, tl!("Quick Actions"));
    let is_group = matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Group { .. }));
    let is_live = matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Path { live: Some(_), .. }));
    let mut actions: Vec<(&str, &str)> = vec![];
    if n_sel > 1 {
        actions.push((tl!("Group"), "object.group"));
        actions.push((tl!("Make Clipping Mask"), "object.clippingMask.make"));
    }
    if is_group {
        actions.push((tl!("Ungroup"), "object.ungroup"));
        actions.push((tl!("Isolate Group"), "object.isolate"));
    }
    if is_live {
        actions.push((tl!("Expand Shape"), "object.expandShape"));
    }
    if multi_color(app, ui.ctx()) {
        actions.push((tl!("Recolor"), "ui.recolorDialog"));
    }
    actions.push((tl!("Offset Path"), "object.path.offsetPath"));
    actions.push((tl!("Simplify"), "object.path.simplify"));
    actions.push((tl!("Arrange: Bring to Front"), "object.arrange.bringToFront"));
    actions.push((tl!("Lock"), "object.lock"));
    let w = (ui.available_width() - 6.0) / 2.0;
    for pair in actions.chunks(2) {
        ui.horizontal(|ui| {
            for (label, id) in pair {
                if widgets::flat_button(ui, label, w).clicked() {
                    crate::menus::invoke(app, id, json!({}));
                }
            }
        });
    }
}

/// The one selected image: its file (or Embedded), colour mode and resolution, and the Links
/// panel's actions for it.
fn image_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(i) = crate::place::selected_image_info(app) else { return };
    let (name, details) = crate::place::image_summary(i);
    let (Some(id), linked) = (i["id"].as_u64(), i["linked"] == true) else { return };
    section_header(ui, if linked { tl!("Linked File") } else { tl!("Embedded Image") });
    ui.add(egui::Label::new(egui::RichText::new(&name).size(12.0).color(t.text_strong)).truncate());
    dim_label(ui, &details);
    ui.add_space(4.0);
    let w = (ui.available_width() - 6.0) / 2.0;
    let links = crate::panels::links::ID;
    ui.horizontal(|ui| {
        if linked {
            if widgets::flat_button(ui, tl!("Embed"), w).clicked() {
                app.run("links.embed", json!({ "ids": [id] })).ok();
            }
            if widgets::flat_button(ui, tl!("Edit Original"), w).clicked() {
                crate::menus::invoke(app, "links.editOriginal", json!({ "id": id }));
            }
        } else {
            if widgets::flat_button(ui, tl!("Unembed…"), w).clicked() {
                crate::panels::links::unembed(app, id, &name);
            }
            if widgets::flat_button(ui, tl!("Image Trace"), w).clicked() {
                app.ui.open_panel = Some("imageTrace".into());
            }
        }
    });
    ui.horizontal(|ui| {
        if app.services.pick_open.is_some() && widgets::flat_button(ui, tl!("Relink…"), w).clicked() {
            crate::panels::links::relink(app, vec![id]);
        }
        if widgets::flat_button(ui, tl!("Links"), w).clicked() {
            app.ui.open_panel = Some(links.into());
        }
    });
}

/// Whether the selection uses more than one colour (`recolor.colors`), which offers the Recolor
/// quick action; cached per document revision.
fn multi_color(app: &mut VectorcraftApp, ctx: &egui::Context) -> bool {
    let Some(st) = app.session.active() else { return false };
    let key = (st.uid, st.revision);
    match pstate::<Option<((u64, u64), bool)>>(ctx, "props-multi-color") {
        Some((k, multi)) if k == key => multi,
        _ => {
            let r = app.session.execute("recolor.colors", &json!({})).unwrap_or_default();
            let multi = r["colors"].as_array().is_some_and(|c| c.len() > 1);
            set_pstate(ctx, "props-multi-color", Some((key, multi)));
            multi
        }
    }
}

fn document_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    let units = app.session.general_unit();
    section_header(ui, tl!("Document"));
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Units"));
        let labels: Vec<&str> = Unit::ALL.iter().map(|u| u.label()).collect();
        if let Some(i) = widgets::dropdown(ui, "units", units.label(), &labels, 140.0) {
            app.run("document.setUnits", json!({"units": labels[i]})).ok();
        }
    });
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Document Setup"), w).clicked() {
            app.run("file.documentSetup", json!({})).ok();
        }
        if widgets::flat_button(ui, tl!("Edit Artboards"), w).clicked() {
            app.select_tool("artboard");
        }
    });
    divider(ui);
    section_header(ui, tl!("Rulers & Grids"));
    ui.horizontal(|ui| {
        let v = app.ui.view.clone();
        if widgets::icon_button(ui, "ruler", tl!("Show Rulers (⌘R)"), v.rulers, 26.0).clicked() {
            app.run("view.rulers", json!({})).ok();
        }
        if widgets::icon_button(ui, "grid-3x3", tl!("Show Grid (⌘')"), v.grid, 26.0).clicked() {
            app.run("view.grid", json!({})).ok();
        }
        let transparency_grid = app.session.active().is_some_and(|d| d.transparency_grid);
        if widgets::icon_button(ui, "square-dashed", tl!("Show Transparency Grid"), transparency_grid, 26.0).clicked() {
            app.run("view.transparencyGrid", json!({})).ok();
        }
    });
    divider(ui);
    section_header(ui, tl!("Guides"));
    ui.horizontal(|ui| {
        let v = app.ui.view.clone();
        if widgets::icon_button(ui, "layout-grid", tl!("Show Guides"), v.guides, 26.0).clicked() {
            app.run("view.guides", json!({})).ok();
        }
        if widgets::icon_button(ui, "sparkles", tl!("Smart Guides (⌘U)"), v.smart_guides, 26.0).clicked() {
            app.run("view.smartGuides", json!({})).ok();
        }
    });
    divider(ui);
    section_header(ui, tl!("Snap Options"));
    let mut sp = app.ui.view.snap_to_point;
    if ui.checkbox(&mut sp, tl!("Snap to Point")).changed() {
        app.ui.view.snap_to_point = sp;
    }
    let mut sg = app.ui.view.snap_to_grid;
    if ui.checkbox(&mut sg, tl!("Snap to Grid")).changed() {
        app.ui.view.snap_to_grid = sg;
    }
    divider(ui);
    section_header(ui, tl!("Preferences"));
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Keyboard Increment"));
        if let Some(v) = widgets::num_field(ui, "kbinc", Some(app.session.prefs.keyboard_increment), units, 80.0) {
            app.session.prefs.keyboard_increment = v.max(0.001);
        }
    });
    let mut ss = app.session.prefs.scale_strokes;
    if ui.checkbox(&mut ss, tl!("Scale Strokes & Effects")).changed() {
        super::transform::set_pref(app, "scaleStrokes", ss);
    }
    divider(ui);
    section_header(ui, tl!("Quick Actions"));
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Document Setup"), w).clicked() {
            app.run("file.documentSetup", json!({})).ok();
        }
        if widgets::flat_button(ui, tl!("Preferences"), w).clicked() {
            app.run("edit.preferences", json!({})).ok();
        }
    });
}

pub fn transform_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        return;
    }
    let units = app.session.general_unit();
    // The bounding box, rotated with rotated objects (as in the Transform panel).
    let Some(bx) = app.selection_box() else {
        dim_label(ui, tl!("No Selection"));
        return;
    };
    let b = bx.rect;
    let refi: usize = ui.data(|d| d.get_temp(egui::Id::new("refpt"))).unwrap_or(4);
    let rp = bx.reference_point(refi);
    section_header(ui, tl!("Transform"));
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("refpt"), i));
        }
        ui.add_space(6.0);
        let link = app.session.prefs.constrain_proportions;
        // Room for the labels and the W/H link.
        let fw = ((ui.available_width() - 70.0) / 2.0).clamp(60.0, 110.0);
        egui::Grid::new("xf-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            dim_label(ui, "X:");
            if let Some(v) = widgets::num_field(ui, "tx", Some(rp.x), units, fw) {
                app.run("object.setBounds", json!({"x": v, "reference": refi})).ok();
            }
            dim_label(ui, "W:");
            if let Some(v) = widgets::num_field(ui, "tw", Some(b.width()), units, fw) {
                app.run("object.setBounds", json!({"width": v, "reference": refi, "proportional": link})).ok();
            }
            ui.end_row();
            dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field(ui, "ty", Some(rp.y), units, fw) {
                app.run("object.setBounds", json!({"y": v, "reference": refi})).ok();
            }
            dim_label(ui, "H:");
            if let Some(v) = widgets::num_field(ui, "th", Some(b.height()), units, fw) {
                app.run("object.setBounds", json!({"height": v, "reference": refi, "proportional": link})).ok();
            }
            ui.end_row();
        });
        super::transform::constrain_link(app, ui);
    });
    ui.horizontal(|ui| {
        icons::icon(ui, "rotate-ccw", 16.0, Tokens::get(ui.ctx()).icon);
        // The bounding box's angle: a new value turns the selection to it.
        if let Some(a) = widgets::plain_field(ui, "rot", bx.angle, "°", 2, 70.0) {
            app.run("object.rotate", json!({"angle": a, "absolute": true})).ok();
        }
        ui.add_space(10.0);
        if widgets::icon_button(ui, "flip-horizontal-2", tl!("Flip Along Horizontal Axis"), false, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "vertical"})).ok();
        }
        if widgets::icon_button(ui, "flip-vertical-2", tl!("Flip Along Vertical Axis"), false, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "horizontal"})).ok();
        }
    });
    // Live shape properties.
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
    {
        match live {
            vectorcraft_doc::LiveShape::Rectangle { radii, .. } => {
                dim_label(ui, tl!("Corner Radius:"));
                super::transform::corner_fields(app, ui, "radius", *radii, units);
            }
            vectorcraft_doc::LiveShape::Polygon { sides, .. } => {
                ui.horizontal(|ui| {
                    dim_label(ui, tl!("Sides:"));
                    if let Some(s) = widgets::plain_field(ui, "sides", *sides as f64, "", 0, 60.0) {
                        app.run("object.setLiveShape", json!({"sides": s as u32})).ok();
                    }
                });
            }
            _ => {}
        }
    }
}

fn appearance_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(n) = first_selected(app) else { return };
    section_header(ui, tl!("Appearance"));
    let weight = super::stroke::shown_weight(app, super::current_stroke(app).as_ref(), &super::stroke_mixed(app, ui.ctx()));
    for (label, is_fill) in [(tl!("Fill"), true), (tl!("Stroke"), false)] {
        ui.horizontal(|ui| {
            super::paint_chip(app, ui, !is_fill, 22.0, false);
            ui.label(egui::RichText::new(label).size(12.0));
            if !is_fill {
                ui.add_space(8.0);
                if let Some(w) = widgets::num_field(ui, "ap-w", weight, app.session.stroke_unit(), 70.0) {
                    app.run("stroke.set", json!({"weight": w})).ok();
                }
                let more = widgets::icon_button(ui, "ellipsis", tl!("Stroke options"), false, 22.0);
                super::stroke::popover(app, &more);
            }
        });
    }
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Opacity")).size(12.0));
        ui.add_space(8.0);
        if let Some(o) = widgets::plain_field(ui, "ap-op", n.opacity as f64 * 100.0, "%", 0, 64.0) {
            app.run("object.setProps", json!({"opacity": o.clamp(0.0, 100.0)})).ok();
        }
        if widgets::icon_button(ui, "ellipsis", tl!("Transparency"), false, 22.0).clicked() {
            app.ui.open_panel = Some("transparency".into());
        }
    });
    ui.horizontal(|ui| {
        // The fx button opens the effect menu, as the Appearance panel's does.
        let r = widgets::flat_button(ui, "fx", 34.0).on_hover_text(tl!("Add New Effect"));
        egui::Popup::menu(&r).show(|ui| super::appearance::fx_menu(app, ui));
        if widgets::icon_button(ui, "ellipsis", tl!("Appearance panel"), false, 22.0).clicked() {
            app.ui.open_panel = Some("appearance".into());
        }
    });
}

pub fn type_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(n) = first_selected(app) else {
        dim_label(ui, tl!("Select a text object"));
        return;
    };
    let NodeKind::Text(tx) = &n.kind else {
        dim_label(ui, tl!("Select a text object"));
        return;
    };
    let s = tx.first_style();
    section_header(ui, tl!("Character"));
    if let Some(f) = widgets::font_dropdown(ui, "font", &s.font_family, ui.available_width() - 4.0) {
        app.run("text.setStyle", json!({ "font": f })).ok();
    }
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Size"));
        let type_unit = app.session.type_unit();
        if let Some(v) = widgets::num_field(ui, "fsize", Some(s.size), type_unit, 70.0) {
            app.run("text.setStyle", json!({"size": v})).ok();
        }
        dim_label(ui, tl!("Leading"));
        if let Some(v) = widgets::num_field(ui, "lead", Some(s.effective_leading()), type_unit, 70.0) {
            app.run("text.setStyle", json!({"leading": v})).ok();
        }
    });
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Tracking"));
        if let Some(v) = widgets::plain_field(ui, "track", s.tracking, "", 0, 60.0) {
            app.run("text.setStyle", json!({"tracking": v})).ok();
        }
    });
    section_header(ui, tl!("Paragraph"));
    ui.horizontal(|ui| {
        for (icon, j) in [("align-start-vertical", "left"), ("align-center-vertical", "center"), ("align-end-vertical", "right")] {
            if widgets::icon_button(ui, icon, j, false, 24.0).clicked() {
                app.run("text.setStyle", json!({"justify": j})).ok();
            }
        }
    });
}
