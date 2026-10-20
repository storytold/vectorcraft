//! The Properties panel: context-sensitive sections like Illustrator's.

use egui::Ui;
use serde_json::json;
use vectorcraft_doc::{LiveShape, NodeKind, Unit};
use vectorcraft_engine::cmd::newdoc;

use super::{corner_radius_row, first_selected, pstate, set_pstate};
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
    let anchors = crate::chrome::anchor_controls(app);
    let first = first_selected(app);
    // Edit Artboards (the Artboard tool): the active artboard, whatever is selected.
    let editing_artboards = app.session.tool_id() == "artboard";
    let label = match (&first, n_sel) {
        _ if editing_artboards => tl!("Artboard").to_string(),
        (None, _) => tl!("Document").to_string(),
        (_, n) if n > 1 => crate::i18n::tn(n as u64, "{n} Object", "{n} Objects"),
        (Some(n), _) if crate::panels::image_trace::is_trace(n) => tl!("Image Tracing").to_string(),
        (Some(n), _) => tl!(n.kind_label()).to_string(),
    };
    ui.label(egui::RichText::new(label).size(11.5).color(t.text_dim));
    ui.add_space(4.0);
    if editing_artboards {
        artboard_sections(app, ui);
        return;
    }
    if first.is_none() {
        document_sections(app, ui);
        return;
    }
    transform_section(app, ui);
    divider(ui);
    // The Gradient tool: the gradient's type and, for a freeform one, how clicks add points (#921).
    if app.session.tool_id() == "gradient" {
        section_header(ui, tl!("Gradient"));
        super::gradient::type_row(app, ui);
        super::gradient::draw_radios(app, ui);
        divider(ui);
    }
    let is_image = n_sel == 1 && matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Image(_)));
    if is_image {
        image_section(app, ui);
        divider(ui);
    }
    if let Some((preset, view)) = crate::panels::image_trace::selected_trace(app) {
        trace_section(app, ui, &preset, view);
        divider(ui);
    }
    if matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Text(_))) {
        type_sections(app, ui);
        divider(ui);
    }
    if crate::dialogs::dimension::selected(app).is_some() {
        crate::dialogs::dimension::properties_section(app, ui);
        divider(ui);
    }
    appearance_section(app, ui);
    divider(ui);
    section_header(ui, tl!("Align"));
    ui.horizontal(|ui| {
        super::align::align_buttons(app, ui, 26.0);
        // The rest (Distribute Objects, Distribute Spacing, Align To) is the Align panel's.
        if widgets::icon_button(ui, "ellipsis", tl!("More Align options"), false, 26.0).clicked() {
            app.ui.open_panel = Some("align".into());
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
    if anchors != crate::chrome::AnchorControls::None {
        divider(ui);
        section_header(ui, tl!("Anchor Point"));
        crate::chrome::anchor_buttons(app, ui, anchors);
    }
    divider(ui);
    section_header(ui, tl!("Quick Actions"));
    let is_group = matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Group { .. }));
    let is_live = matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Path { live: Some(l), .. }) if !matches!(**l, LiveShape::Path { .. }));
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
    if is_image {
        crate::panels::image_trace::trace_button(app, ui, ui.available_width());
        actions.push((tl!("Crop Image"), "ui.cropImage"));
        actions.push((tl!("Mask"), "object.maskImage"));
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
    ui.horizontal(|ui| crate::place::link_buttons(app, ui, id, linked, &name, Some(w)));
    ui.horizontal(|ui| {
        if app.services.pick_open.is_some() && widgets::flat_button(ui, tl!("Replace…"), w).clicked() {
            crate::panels::links::replace(app, vec![id]);
        }
        if widgets::flat_button(ui, tl!("Links"), w).clicked() {
            app.ui.open_panel = Some(links.into());
        }
    });
}

/// The one selected Image Trace object: its preset (choosing another traces it again with that
/// one), view, a button to reveal the tracing controls, Expand and Release.
fn trace_section(app: &mut VectorcraftApp, ui: &mut Ui, preset: &str, view: vectorcraft_doc::TraceView) {
    section_header(ui, tl!("Image Trace"));
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Preset:"));
        crate::panels::image_trace::preset_dropdown(app, ui, preset, ui.available_width());
    });
    ui.horizontal(|ui| crate::panels::image_trace::view_row(app, ui, view, ui.available_width()));
    if widgets::flat_button(ui, tl!("Image Trace"), ui.available_width()).clicked() {
        crate::panels::image_trace::open_settings(app);
    }
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        for (label, id) in [(tl!("Expand"), "imageTrace.expand"), (tl!("Release"), "imageTrace.release")] {
            if widgets::flat_button(ui, label, w).clicked() {
                app.run(id, json!({})).ok();
            }
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

/// Edit Artboards (the Artboard tool): the active artboard's name, which can be edited to rename
/// it, and the Artboard Options icon; its position and size, its preset and
/// orientation (#671) (with Scale Artwork with Artboard on, a new size scales its art, #602; a new
/// W or H, preset or orientation goes to every selected artboard, in one undo step), Move
/// and Scale Artwork with Artboard, New Artboard and Delete Artboard, and Quick Actions: Rearrange All (#681) and
/// Exit back to the Selection tool (#530).
fn artboard_sections(app: &mut VectorcraftApp, ui: &mut Ui) {
    let units = app.session.general_unit();
    let origin = app.ruler_origin();
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    let i = super::artboards::selected(app, n);
    let scale_art = super::artboards::scale_art(app);
    let move_art = super::artboards::move_art(app);
    if let Some(ab) = app.session.active().and_then(|d| d.doc.artboards.get(i)).cloned() {
        // The field shows the artboard's name as it is (names are never translated). Typing a new
        // name and pressing Enter or clicking elsewhere renames the artboard as the Artboards panel's
        // rename does; pressing Escape or leaving the field empty keeps the old name. A click on
        // the icon opens the artboard's Artboard Options, as double-clicking the Artboard tool's
        // button does.
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 26.0 - ui.spacing().item_spacing.x).max(80.0);
            if let Some(name) = widgets::text_field(ui, ("ab", "name"), Some(&ab.name), w, 1).filter(|n| !n.is_empty()) {
                app.run("artboard.setProps", json!({"index": i, "name": name})).ok();
            }
            if widgets::icon_button(ui, "dc-artboard-options", tl!("Artboard Options"), false, 26.0).clicked() {
                app.run("tool.options", json!({"tool": "artboard"})).ok();
            }
        });
        let fw = super::field_width(ui);
        egui::Grid::new("ab-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            let r = ab.rect;
            for (row, [(l1, k1, v1), (l2, k2, v2)]) in
                [[("X:", "x", r.x0), ("W:", "width", r.width())], [("Y:", "y", r.y0), ("H:", "height", r.height())]].into_iter().enumerate()
            {
                for (label, key, v) in [(l1, k1, v1), (l2, k2, v2)] {
                    dim_label(ui, label);
                    let offset = match key {
                        "x" => origin.x,
                        "y" => origin.y,
                        _ => 0.0,
                    };
                    if let Some(v) = widgets::num_field(ui, ("ab", key, row), Some(v - offset), units, fw) {
                        // A new X or Y moves the active artboard; a new W or H goes to every selected one.
                        let indices = if matches!(key, "width" | "height") { super::artboards::resized_with(app, i) } else { vec![i] };
                        let common = json!({"scaleArt": scale_art, "moveArt": move_art});
                        super::artboards::set_props(app, &indices, common, |j, _| Some(json!({"index": j, key: v + offset}))).ok();
                    }
                }
                ui.end_row();
            }
        });
        artboard_size_row(app, ui, i, ab.rect.width(), ab.rect.height());
        ui.add_space(2.0);
        super::artboards::art_options(app, ui);
    }
    ui.add_space(4.0);
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("New Artboard"), w).clicked() && app.run("artboard.new", json!({})).is_ok() {
            super::artboards::select(app, n);
        }
        if ui.add_enabled_ui(n > 1, |ui| widgets::flat_button(ui, tl!("Delete Artboard"), w)).inner.clicked() {
            super::artboards::delete_selected(app);
        }
    });
    divider(ui);
    section_header(ui, tl!("Quick Actions"));
    ui.horizontal(|ui| {
        rearrange_button(app, ui, n, w);
        if widgets::flat_button(ui, tl!("Exit"), w).clicked() {
            app.select_tool("selection");
        }
    });
}

/// Rearrange All (#681): opens Rearrange All Artboards; disabled with fewer than two artboards.
fn rearrange_button(app: &mut VectorcraftApp, ui: &mut Ui, artboards: usize, w: f32) {
    if ui.add_enabled_ui(artboards > 1, |ui| widgets::flat_button(ui, tl!("Rearrange All"), w)).inner.clicked() {
        super::artboards::open_rearrange(app);
    }
}

/// A preset `pw` × `ph` turned to an artboard's orientation (`landscape`, or portrait; a square
/// preset as it is).
fn turned(landscape: bool, pw: f64, ph: f64) -> (f64, f64) {
    if (pw > ph) == landscape || pw == ph { (pw, ph) } else { (ph, pw) }
}

/// Artboard `i`'s size preset (New Document's saved and built-in presets, by category, either way
/// round; Custom when its `w` × `h` points match none) and its orientation, here and in the Control
/// bar. A preset sizes each selected artboard in its own orientation; an orientation swaps the
/// width and height of each selected artboard that is not square and not already in it. Each
/// top-left corner stays put.
pub(crate) fn artboard_size_row(app: &mut VectorcraftApp, ui: &mut Ui, i: usize, w: f64, h: f64) {
    let landscape = w > h;
    let same = |s: &newdoc::DocSettings| {
        let (pw, ph) = turned(landscape, s.width, s.height);
        (pw - w).abs() < 0.01 && (ph - h).abs() < 0.01
    };
    // Recent holds documents made, not presets.
    let cats: Vec<(&str, Vec<newdoc::DocSettings>)> = newdoc::category_names()
        .filter(|c| !c.eq_ignore_ascii_case("Recent"))
        .filter_map(|c| Some((c, newdoc::category(&app.session, c)?)))
        .filter(|(_, l)| !l.is_empty())
        .collect();
    let current = cats.iter().flat_map(|(_, l)| l).find(|s| same(s)).map_or(tl!("Custom"), |s| crate::dialogs::preset_name(&s.name)).to_string();
    // The preset's width and height as listed, or the orientation clicked.
    let mut preset = None;
    let mut orientation = None;
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Preset:"));
        preset = widgets::combo(ui, "ab-preset", &current, 140.0, false, |ui| {
            let mut chosen = None;
            for (cat, list) in &cats {
                ui.menu_button(tl!(cat), |ui| {
                    widgets::menu_scroll(ui, |ui| {
                        for s in list {
                            if widgets::menu_item_name(ui, crate::dialogs::preset_name(&s.name), true, same(s)) {
                                chosen = Some((s.width, s.height));
                            }
                        }
                    });
                });
            }
            chosen
        });
        ui.spacing_mut().item_spacing.x = 2.0;
        for (is_landscape, tip) in [(false, tl!("Portrait")), (true, tl!("Landscape"))] {
            if widgets::orientation_button(ui, is_landscape, landscape == is_landscape, tip) {
                orientation = Some(is_landscape);
            }
        }
    });
    if preset.is_none() && orientation.is_none() {
        return;
    }
    let scale_art = super::artboards::scale_art(app);
    let indices = super::artboards::resized_with(app, i);
    super::artboards::set_props(app, &indices, json!({"scaleArt": scale_art}), |j, r| {
        let (w, h) = (r.width(), r.height());
        let (width, height) = match (preset, orientation) {
            (Some((pw, ph)), _) => turned(w > h, pw, ph),
            (None, Some(l)) if (w > h) != l && w != h => (h, w),
            _ => return None,
        };
        Some(json!({"index": j, "width": width, "height": height}))
    })
    .ok();
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
    let artboards = app.session.active().map_or(0, |d| d.doc.artboards.len());
    if artboards > 1 {
        rearrange_button(app, ui, artboards, w);
    }
    divider(ui);
    section_header(ui, tl!("Appearance"));
    fill_stroke_rows(app, ui);
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
    if widgets::check(ui, tl!("Snap to Point"), app.ui.view.snap_to_point, true) {
        app.ui.view.snap_to_point ^= true;
    }
    if widgets::check(ui, tl!("Snap to Grid"), app.ui.view.snap_to_grid, true) {
        app.ui.view.snap_to_grid ^= true;
    }
    divider(ui);
    section_header(ui, tl!("Preferences"));
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Keyboard Increment"));
        if let Some(v) = widgets::num_field(ui, "kbinc", Some(app.session.prefs.keyboard_increment), units, fw) {
            app.session.prefs.keyboard_increment = v.max(0.001);
        }
    });
    let ss = app.session.prefs.scale_strokes;
    if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
        super::transform::set_pref(app, "scaleStrokes", !ss);
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
    let origin = app.ruler_origin();
    // The bounding box, rotated with rotated objects (as in the Transform panel).
    let Some(bx) = app.selection_box() else {
        dim_label(ui, tl!("No Selection"));
        return;
    };
    let b = bx.rect;
    let refi: usize = ui.data(|d| d.get_temp(egui::Id::new("refpt"))).unwrap_or(4);
    let rp = bx.reference_point(refi);
    let copy = super::transform::alt_enter_copies(app, ui);
    section_header(ui, tl!("Transform"));
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::new("refpt"), i));
        }
        ui.add_space(6.0);
        let link = super::transform::proportional(app, ui);
        egui::Grid::new("xf-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            dim_label(ui, "X:");
            if let Some(v) = widgets::num_field(ui, "tx", Some(rp.x - origin.x), units, fw) {
                app.run("object.setBounds", json!({"x": v + origin.x, "reference": refi, "copy": copy})).ok();
            }
            dim_label(ui, "W:");
            if let Some(v) = widgets::size_field(ui, "tw", Some(b.width()), units, fw) {
                app.run("object.setBounds", json!({"width": v, "reference": refi, "proportional": link, "copy": copy})).ok();
            }
            ui.end_row();
            dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field(ui, "ty", Some(rp.y - origin.y), units, fw) {
                app.run("object.setBounds", json!({"y": v + origin.y, "reference": refi, "copy": copy})).ok();
            }
            dim_label(ui, "H:");
            if let Some(v) = widgets::size_field(ui, "th", Some(b.height()), units, fw) {
                app.run("object.setBounds", json!({"height": v, "reference": refi, "proportional": link, "copy": copy})).ok();
            }
            ui.end_row();
        });
        super::transform::constrain_link(app, ui);
    });
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        let icon = icons::icon(ui, "rotate-ccw", 16.0, Tokens::get(ui.ctx()).icon);
        crate::scrub::note_label(ui, icon.rect);
        // The bounding box's angle: a new value turns the selection to it.
        if let Some(a) = widgets::plain_field(ui, "rot", bx.angle, "°", 2, fw) {
            app.run("object.rotate", json!({"angle": a, "absolute": true, "copy": copy})).ok();
        }
        ui.add_space(10.0);
        // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
        // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom). The labels
        // are the reference app's Properties panel names for the same buttons (#1910).
        if widgets::icon_button(ui, "flip-vertical-2", tl!("Flip Along Horizontal Axis"), false, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "vertical"})).ok();
        }
        if widgets::icon_button(ui, "flip-horizontal-2", tl!("Flip Along Vertical Axis"), false, 24.0).clicked() {
            app.run("object.reflect", json!({"axis": "horizontal"})).ok();
        }
        // More Options: the Transform panel's Scale Corners and Scale Strokes & Effects, open
        // while they are toggled.
        let more = widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, 24.0);
        egui::Popup::menu(&more).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
            if widgets::check(ui, tl!("Scale Corners"), sc, true) {
                super::transform::set_pref(app, "scaleCorners", !sc);
            }
            if widgets::check(ui, tl!("Scale Strokes & Effects"), ss, true) {
                super::transform::set_pref(app, "scaleStrokes", !ss);
            }
        });
    });
    // Live shape properties.
    if let Some(n) = first_selected(app)
        && let NodeKind::Path { live: Some(live), .. } = &n.kind
    {
        match &**live {
            vectorcraft_doc::LiveShape::Rectangle { .. } => corner_radius_row(app, ui, &n, "radius"),
            vectorcraft_doc::LiveShape::Polygon { .. } => super::transform::polygon_rows(app, ui, &n, live, "props-polygon"),
            vectorcraft_doc::LiveShape::Ellipse { pie, .. } => super::transform::pie_rows(app, ui, *pie, "props-pie"),
            _ => {}
        }
    }
}

fn appearance_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(n) = first_selected(app) else { return };
    section_header(ui, tl!("Appearance"));
    fill_stroke_rows(app, ui);
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        widgets::field_label(ui, egui::RichText::new(tl!("Opacity")).size(12.0));
        ui.add_space(8.0);
        if let Some(o) = widgets::plain_field(ui, "ap-op", n.opacity as f64 * 100.0, "%", 0, fw) {
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

/// The Fill and Stroke rows, with the Control bar's widgets: the chips (a click opens the
/// swatches, Shift-click the mixer), the Stroke link that opens the Stroke panel as a popover
/// and the weight spinner with its presets. With nothing selected they set up the next object.
fn fill_stroke_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let weight = super::stroke::shown_weight(app, super::current_stroke(app).as_ref(), &super::stroke_mixed(app, ui.ctx()));
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        super::paint_chip(app, ui, false, 22.0, true);
        ui.label(egui::RichText::new(tl!("Fill")).size(12.0).color(t.text));
    });
    ui.horizontal(|ui| {
        super::paint_chip(app, ui, true, 22.0, true);
        super::stroke::link(app, ui, tl!("Stroke"));
        ui.add_space(8.0);
        super::stroke::weight_field(app, ui, "ap-w", weight, fw);
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
    // The font, Font Size, Leading, Kerning and Tracking as in the Character panel.
    let w = ui.available_width();
    super::character::font_pickers(app, ui, &s, ("font", "font-style"), (w - 4.0, w - 4.0));
    super::character::variable_font(app, ui, &s, w - 4.0);
    super::character::metrics_grid(app, ui, "props-char", &s, w, false);
    section_header(ui, tl!("Paragraph"));
    // As in the Paragraph panel: the seven alignments and Hyphenate (#775); the rest is there.
    if let Some((_, para)) = super::character::text_style(app) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            super::paragraph::alignment_buttons(app, ui, &para, 24.0);
            if widgets::icon_button(ui, "ellipsis", tl!("More Paragraph options"), false, 24.0).clicked() {
                app.ui.open_panel = Some("paragraph".into());
            }
        });
        super::paragraph::hyphenate_check(app, ui, &para);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, Pos2, Rect, vec2};
    use vectorcraft_engine::Session;

    use super::*;

    /// One frame of the panel with `events` → the texts painted, with their rects.
    fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        panel_frame(app, ctx, events, show)
    }

    fn panel_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, draw: fn(&mut VectorcraftApp, &mut Ui)) -> Vec<(String, Rect)> {
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), events, ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| draw(app, ui));
        out.textures_delta.clear();
        out.shapes
            .iter()
            .filter_map(|s| {
                if let egui::Shape::Text(t) = &s.shape {
                    Some((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size())))
                } else {
                    None
                }
            })
            .collect()
    }

    fn click(app: &mut VectorcraftApp, ctx: &egui::Context, texts: &[(String, Rect)], label: &str) {
        let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"));
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(app, ctx, vec![Event::PointerMoved(at), b(true)]);
        frame(app, ctx, vec![b(false)]);
    }

    /// The Snap Options and Scale Strokes & Effects checkboxes keep a visible box at rest in every
    /// theme, as the Preferences ones do (#1080), and toggle when clicked.
    #[test]
    fn the_document_checkboxes_show_their_boxes_and_toggle() {
        for brightness in crate::theme::Brightness::ALL {
            let mut app = VectorcraftApp::new(Session::new(), Default::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            crate::theme::apply(&ctx, brightness);
            frame(&mut app, &ctx, vec![]);
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(&mut app, ui));
            out.textures_delta.clear();
            let boxes = out
                .shapes
                .iter()
                .filter(|s| matches!(&s.shape, egui::Shape::Rect(r) if r.stroke.width > 0.0 && (r.rect.width() - 13.0).abs() < 0.5 && (r.rect.height() - 13.0).abs() < 0.5))
                .count();
            assert!(boxes >= 3, "{brightness:?}: {boxes} checkbox outlines");
        }
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let texts = frame(&mut app, &ctx, vec![]);
        let (grid, strokes) = (app.ui.view.snap_to_grid, app.session.prefs.scale_strokes);
        click(&mut app, &ctx, &texts, "Snap to Grid");
        assert_eq!(app.ui.view.snap_to_grid, !grid);
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Scale Strokes & Effects");
        assert_eq!(app.session.prefs.scale_strokes, !strokes);
    }

    /// #921: with the Gradient tool, a Gradient section: the type, and for a freeform gradient
    /// Draw as Points or Lines radio buttons.
    #[test]
    fn the_gradient_tool_shows_the_gradient_type_and_draw() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.run("paint.setFill", json!({"gradient": {}})).unwrap();
        let ctx = egui::Context::default();
        let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(!has(&texts, "Gradient"), "only with the Gradient tool");
        app.select_tool("gradient");
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Gradient") && has(&texts, "Type:") && !has(&texts, "Draw:"), "{texts:?}");
        app.run("paint.editGradient", json!({"kind": "freeform"})).unwrap();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Draw:") && has(&texts, "Points") && has(&texts, "Lines"), "{texts:?}");
        click(&mut app, &ctx, &texts, "Lines");
        let fill = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.fill_paint();
        let vectorcraft_color::Paint::Gradient(g) = fill else { panic!("{fill:?}") };
        assert_eq!(super::super::gradient::shown_points(&g).mode, vectorcraft_color::FreeformMode::Lines);
    }

    /// #696: the number fields are all as wide: the Transform fields, the rotation, the corner
    /// radius, Opacity and the stroke weight.
    #[test]
    fn the_number_fields_are_all_as_wide() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        // The field each value is shown in: the narrowest widget around its text that isn't the
        // text alone.
        let field = |value: &str| {
            let at = texts.iter().find(|(t, _)| t == value).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("{value}: {texts:?}"));
            ctx.viewport(|vp| {
                vp.prev_pass
                    .widgets
                    .layers()
                    .flat_map(|(_, w)| w.iter())
                    .filter(|w| w.rect.contains(at) && w.rect.width() > 30.0 && w.rect.height() < 40.0)
                    .map(|w| w.rect.width())
                    .fold(f32::INFINITY, f32::min)
            })
        };
        let widths = [field("35 pt"), field("0°"), field("0 pt"), field("100%")];
        assert!(widths.iter().all(|w| (w - widths[0]).abs() < 1.0), "X, rotation, corner radius, opacity: {widths:?}");
    }

    /// #1512: the link behind Corner Radius. On, one field sets every corner; off, a field per
    /// corner shows each corner's own radius.
    #[test]
    fn the_corner_radius_link_shows_a_field_per_corner() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
        app.run("object.setLiveShape", json!({"radius": 7, "corners": [2]})).unwrap();
        let ctx = egui::Context::default();
        let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(app.session.prefs.single_corner_radius, "linked at first");
        assert!(has(&texts, "Corner Radius:") && !has(&texts, "Top Left:") && !has(&texts, "7 pt"), "{texts:?}");
        app.run("prefs.set", json!({"key": "singleCornerRadius", "value": false})).unwrap();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        for l in ["Corner Radius:", "Top Left:", "Top Right:", "Bottom Right:", "Bottom Left:"] {
            assert!(has(&texts, l), "{l}: {texts:?}");
        }
        assert_eq!(texts.iter().filter(|(t, _)| t == "7 pt").count(), 1, "the bottom-right corner: {texts:?}");
        // The fields sit in line, in the order top left, top right, bottom right, bottom left.
        let at = |s: &str| texts.iter().find(|(t, _)| t == s).unwrap().1;
        assert!(at("Top Left:").top() < at("Top Right:").top() && at("Top Right:").top() < at("Bottom Right:").top());
        assert!((at("7 pt").top() - at("Bottom Right:").top()).abs() < 6.0, "{texts:?}");
    }

    /// The Transform section's More Options holds Scale Corners and Scale Strokes & Effects, and
    /// stays open while they are toggled.
    #[test]
    fn transform_more_options_toggles_scale_corners_and_strokes() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap();
        let ctx = egui::Context::default();
        let has = |texts: &[(String, Rect)], s: &str| texts.iter().any(|(t, _)| t == s);
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(!has(&texts, "Scale Corners"), "closed at first");
        // The button is an icon at the end of the angle row, under the H: field: sweep that row.
        let h = texts.iter().find(|(t, _)| t == "H:").unwrap().1;
        let mut open = None;
        'sweep: for dy in (12..60).step_by(6) {
            for x in (120..300).step_by(6) {
                let at = egui::pos2(x as f32, h.bottom() + dy as f32);
                let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
                frame(&mut app, &ctx, vec![Event::PointerMoved(at), b(true)]);
                frame(&mut app, &ctx, vec![b(false)]);
                let texts = frame(&mut app, &ctx, vec![]);
                if has(&texts, "Scale Corners") {
                    open = Some(texts);
                    break 'sweep;
                }
            }
        }
        let texts = open.expect("More Options opens the popup");
        let (sc, ss) = (app.session.prefs.scale_corners, app.session.prefs.scale_strokes);
        click(&mut app, &ctx, &texts, "Scale Corners");
        assert_eq!(app.session.prefs.scale_corners, !sc);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(has(&texts, "Scale Strokes & Effects"), "still open after a toggle");
        click(&mut app, &ctx, &texts, "Scale Strokes & Effects");
        assert_eq!(app.session.prefs.scale_strokes, !ss);
    }

    /// Both panels forward the Move Artwork toggle when their actual X field is edited.
    #[test]
    fn artboard_coordinate_fields_move_art_in_properties_and_transform() {
        for draw in [show as fn(&mut VectorcraftApp, &mut Ui), super::super::transform::show] {
            for move_art in [false, true] {
                let mut app = VectorcraftApp::new(Session::new(), Default::default());
                app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
                let id = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
                app.select_tool("artboard");
                app.session.set_tool_option("moveArt", &json!(move_art));
                let ctx = egui::Context::default();
                panel_frame(&mut app, &ctx, vec![], draw);
                let texts = panel_frame(&mut app, &ctx, vec![], draw);
                let label = texts.iter().find(|(t, _)| t == "X:").unwrap().1;
                let at = egui::pos2(label.right() + 40.0, label.center().y);
                let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
                panel_frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)], draw);
                panel_frame(&mut app, &ctx, vec![press(false)], draw);
                panel_frame(
                    &mut app,
                    &ctx,
                    vec![Event::Key { key: egui::Key::ArrowUp, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
                    draw,
                );
                let d = &app.session.doc().unwrap().doc;
                assert_eq!(d.artboards[0].rect.x0, 1.0, "the focused X field steps its position");
                assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, if move_art { 11.0 } else { 10.0 });
                app.run("edit.undo", json!({})).unwrap();
                let d = &app.session.doc().unwrap().doc;
                assert_eq!(d.artboards[0].rect.x0, 0.0);
                assert_eq!(d.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0, 10.0);
            }
        }
    }

    /// Three landscape artboards, with Artboards 1 and 3 selected with the Artboard tool (Artboard
    /// 1 active): 200 × 100, 200 × 100 and 300 × 150.
    fn two_selected_artboards() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.run("artboard.setProps", json!({"index": 2, "width": 300, "height": 150})).unwrap();
        app.select_tool("artboard");
        super::super::artboards::select(&mut app, 0);
        app.session.set_tool_option("selected", &json!([0, 2]));
        app
    }

    /// Each artboard's (x0, y0, width, height).
    fn boards(app: &VectorcraftApp) -> Vec<(f64, f64, f64, f64)> {
        app.session.doc().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.rect.y0, a.rect.width(), a.rect.height())).collect()
    }

    /// #1482: with Artboards 1 and 3 selected, a W typed into Properties or the Transform panel
    /// resizes both in one undo step, and Artboard 2 keeps its size. Properties keeps each top-left
    /// corner; the Transform panel resizes each about its own center, its default reference point.
    #[test]
    fn artboard_width_resizes_every_selected_artboard_in_properties_and_transform() {
        for (draw, centered) in [(show as fn(&mut VectorcraftApp, &mut Ui), false), (super::super::transform::show, true)] {
            let mut app = two_selected_artboards();
            let before = boards(&app);
            let undo = app.session.doc().unwrap().history.undo.len();
            let ctx = egui::Context::default();
            panel_frame(&mut app, &ctx, vec![], draw);
            let texts = panel_frame(&mut app, &ctx, vec![], draw);
            let label = texts.iter().find(|(t, _)| t == "W:").unwrap().1;
            let at = egui::pos2(label.right() + 40.0, label.center().y);
            let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            let key = |key, modifiers| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
            panel_frame(&mut app, &ctx, vec![Event::PointerMoved(at), press(true)], draw);
            panel_frame(&mut app, &ctx, vec![press(false)], draw);
            panel_frame(&mut app, &ctx, vec![key(egui::Key::A, egui::Modifiers::COMMAND)], draw);
            panel_frame(&mut app, &ctx, vec![Event::Text("250".into())], draw);
            panel_frame(&mut app, &ctx, vec![key(egui::Key::Enter, egui::Modifiers::NONE)], draw);
            panel_frame(&mut app, &ctx, vec![], draw);
            let mut want = before.clone();
            for (i, shift) in [(0, 25.0), (2, -25.0)] {
                want[i].2 = 250.0;
                if centered {
                    want[i].0 -= shift;
                }
            }
            assert_eq!(boards(&app), want, "centered: {centered}");
            assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
            app.run("edit.undo", json!({})).unwrap();
            assert_eq!(boards(&app), before);
        }
    }

    /// #530: Edit Artboards shows the active artboard, with New Artboard, Delete Artboard and Exit.
    /// #775: type's Paragraph section has Hyphenate, and its checkbox turns it on.
    #[test]
    fn type_paragraph_section_hyphenates() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("text.create", json!({"x": 10, "y": 50, "text": "Paragraph"})).unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Hyphenate");
        let hyphenate = |app: &VectorcraftApp| crate::panels::character::text_style(app).unwrap().1.hyphenate;
        assert!(hyphenate(&app));
    }

    /// Variable fonts get the same Variable Font button in Properties as in the Character panel.
    #[test]
    fn properties_variable_font_button_for_variable_type() {
        use vectorcraft_text::test_fonts::{TWO_AXIS_FAMILY, two_axis_font};
        vectorcraft_text::FontDb::global().add_font(two_axis_font().unwrap());
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let plain = app.run("text.create", json!({"x": 20, "y": 50, "text": "Static"})).unwrap()["id"].clone();
        let id = app.run("text.create", json!({"x": 20, "y": 120, "text": "lIn", "font": TWO_AXIS_FAMILY})).unwrap()["id"].clone();
        let ctx = egui::Context::default();
        app.run("select.set", json!({"ids": [plain]})).unwrap();
        assert!(!frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Variable Font"));
        app.run("select.set", json!({"ids": [id]})).unwrap();
        assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Variable Font"));
    }

    #[test]
    fn edit_artboards_adds_artboards_and_exits() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Edit Artboards");
        assert_eq!(app.session.tool_id(), "artboard");
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "Artboard 1") && texts.iter().any(|(t, _)| t == "200 pt"), "{texts:?}");
        click(&mut app, &ctx, &texts, "New Artboard");
        let boards = |app: &VectorcraftApp| app.session.active().unwrap().doc.artboards.len();
        assert_eq!(boards(&app), 2);
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "Artboard 2"), "the new artboard is the active one: {texts:?}");
        click(&mut app, &ctx, &texts, "Delete Artboard");
        assert_eq!(boards(&app), 1);
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Exit");
        assert_eq!(app.session.tool_id(), "selection");
    }

    /// Typing a name into Edit Artboards' name field and pressing Enter renames the active artboard
    /// alone, trimmed as the Artboards panel's rename trims it, with one command and one undo step;
    /// a click elsewhere in the panel renames it too. Pressing Escape or leaving the field empty
    /// keeps the name and runs no command.
    #[test]
    fn edit_artboards_renames_the_active_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        super::super::artboards::select(&mut app, 1);
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.artboards.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        let key = |key, modifiers| vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }];
        // Click the field showing Artboard 2's name, select all of it, type `typed`, then send each
        // frame's events of `end` → the number of commands run.
        let rename = |app: &mut VectorcraftApp, typed: Vec<Event>, end: Vec<Vec<Event>>| {
            frame(app, &ctx, vec![]);
            let texts = frame(app, &ctx, vec![]);
            let name = app.session.doc().unwrap().doc.artboards[1].name.clone();
            click(app, &ctx, &texts, &name);
            frame(app, &ctx, key(egui::Key::A, egui::Modifiers::COMMAND));
            let commands = app.run_count;
            frame(app, &ctx, typed);
            for events in end {
                frame(app, &ctx, events);
            }
            frame(app, &ctx, vec![]);
            app.run_count.wrapping_sub(commands)
        };
        let enter = || vec![key(egui::Key::Enter, egui::Modifiers::NONE)];
        let text = |s: &str| vec![Event::Text(s.into())];
        assert_eq!(rename(&mut app, text("Cover"), vec![key(egui::Key::Escape, egui::Modifiers::NONE)]), 0, "Escape");
        assert_eq!(rename(&mut app, key(egui::Key::Backspace, egui::Modifiers::NONE), enter()), 0, "an empty field");
        assert_eq!(app.session.doc().unwrap().doc.artboards, before);
        assert_eq!(rename(&mut app, text(" Cover "), enter()), 1);
        let mut renamed = before.clone();
        renamed[1].name = "Cover".into();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.artboards, renamed, "Artboard 2 alone is renamed");
        assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.artboards, before);
        assert!(!st.is_dirty());
        // The click elsewhere is a press and release on the panel's "Artboard" heading, outside
        // the field.
        let at = frame(&mut app, &ctx, vec![]).iter().find(|(t, _)| t == "Artboard").map(|(_, r)| r.center()).unwrap();
        let b = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        assert_eq!(rename(&mut app, text("Back"), vec![vec![Event::PointerMoved(at), b(true)], vec![b(false)]]), 1, "a click elsewhere");
        renamed[1].name = "Back".into();
        assert_eq!(app.session.doc().unwrap().doc.artboards, renamed);
    }

    /// A click on Edit Artboards' Artboard Options icon opens Artboard Options for the active
    /// artboard, as double-clicking the Artboard tool's button does (`tool.options {tool: "artboard"}`).
    #[test]
    fn edit_artboards_opens_the_active_artboards_options() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        app.select_tool("artboard");
        super::super::artboards::select(&mut app, 1);
        app.run("tool.options", json!({"tool": "artboard"})).unwrap();
        let want = app.ui.dialog.take().unwrap().fields;
        let ctx = egui::Context::default();
        frame(&mut app, &ctx, vec![]);
        let texts = frame(&mut app, &ctx, vec![]);
        // The icon is the rightmost clickable widget on the name's line.
        let name = texts.iter().find(|(t, _)| t == "Artboard 2").map(|(_, r)| *r).unwrap();
        let icon = ctx.viewport(|vp| {
            vp.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(name.center().y))
                .map(|w| w.rect)
                .max_by(|a, b| a.left().total_cmp(&b.left()))
                .unwrap()
                .center()
        });
        let commands = app.run_count;
        let press = |pressed| Event::PointerButton { pos: icon, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![Event::PointerMoved(icon), press(true)]);
        frame(&mut app, &ctx, vec![press(false)]);
        assert_eq!(app.run_count, commands.wrapping_add(1), "one command");
        let d = app.ui.dialog.take().expect("Artboard Options");
        assert_eq!((d.kind.as_str(), d.f64("index", 9.0), d.str("name").as_str()), (crate::dialogs::artboard_options::KIND, 1.0, "Artboard 2"));
        assert_eq!(d.fields, want, "the fields the tool's double-click shows");
        assert_eq!(app.session.tool_id(), "artboard");
    }

    fn artboard_deletion_app(count: usize) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": count})).unwrap();
        for i in 0..count {
            app.run("shape.rectangle", json!({"x": i * 250 + 10, "y": 10, "width": 30, "height": 20})).unwrap();
        }
        app.select_tool("artboard");
        super::super::artboards::select(&mut app, 0);
        app
    }

    fn artboard_deletion_click(
        app: &mut VectorcraftApp,
        ctx: &egui::Context,
        draw: fn(&mut VectorcraftApp, &mut Ui),
        label: Option<&str>,
        modifiers: egui::Modifiers,
    ) {
        let at = if let Some(label) = label {
            panel_frame(app, ctx, vec![], draw);
            let texts = panel_frame(app, ctx, vec![], draw);
            texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
        } else {
            // The trash is the last 24-point footer button. Use the drawn panel bounds, as the Asset Export bin test uses its footer.
            let mut bounds = Rect::NOTHING;
            for _ in 0..2 {
                let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
                let mut out = ctx.run_ui(raw, |ui| {
                    draw(app, ui);
                    bounds = ui.min_rect();
                });
                out.textures_delta.clear();
            }
            assert!(bounds.width() >= 24.0 && bounds.height() >= 24.0);
            bounds.right_bottom() - vec2(12.0, 12.0)
        };
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers };
        panel_frame(app, ctx, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at), button(true)], draw);
        panel_frame(app, ctx, vec![button(false)], draw);
        panel_frame(app, ctx, vec![Event::ModifiersChanged(egui::Modifiers::NONE)], draw);
    }

    fn select_artboard_rows(app: &mut VectorcraftApp, ctx: &egui::Context, indices: &[usize]) {
        for (at, i) in indices.iter().enumerate() {
            let modifiers = if at == 0 { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND };
            artboard_deletion_click(app, ctx, super::super::artboards::show, Some(&format!("Artboard {}", i + 1)), modifiers);
        }
        let count = app.session.doc().unwrap().doc.artboards.len();
        assert_eq!(super::super::artboards::picked(app, count), indices, "the rows are selected before deletion");
    }

    fn assert_artboard_deletion(draw: fn(&mut VectorcraftApp, &mut Ui), label: Option<&str>) {
        for indices in [vec![0, 2], vec![2], vec![3], vec![2, 3], vec![0, 1, 2, 3]] {
            let mut app = artboard_deletion_app(4);
            let ctx = egui::Context::default();
            select_artboard_rows(&mut app, &ctx, &indices);
            app.session.doc_mut().unwrap().mark_saved();
            let before = app.session.doc().unwrap().doc.clone();
            let undo = app.session.doc().unwrap().history.undo.len();
            let all_selected = indices == [0, 1, 2, 3];
            let expected: Vec<_> = if all_selected {
                assert_eq!(app.view().unwrap().artboard, 3, "the active last board must not replace the first-board survivor oracle");
                vec![&before.artboards[0]]
            } else {
                before.artboards.iter().enumerate().filter(|(i, _)| !indices.contains(i)).map(|(_, a)| a).collect()
            };
            let expected = serde_json::to_value(expected).unwrap();
            let artwork = serde_json::to_value(&before.layers).unwrap();

            let commands = app.run_count;
            artboard_deletion_click(&mut app, &ctx, draw, label, egui::Modifiers::NONE);
            let st = app.session.doc().unwrap();
            assert_eq!(
                serde_json::to_value(&st.doc.artboards).unwrap(),
                expected,
                "selected deletion keeps the expected IDs; all-selected keeps original board 1"
            );
            assert_eq!(app.run_count, commands.wrapping_add(1), "one command from the actual deletion control");
            assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork, "artwork IDs, geometry and appearance stay intact");
            assert_eq!(st.history.undo.len(), undo + 1, "one deletion is one undo step");
            assert!(st.is_dirty());
            let active = app.view().unwrap().artboard;
            if all_selected {
                assert_eq!(active, 0);
            }
            assert!(active < st.doc.artboards.len(), "the active artboard remains in range");
            assert_eq!(app.session.tool_options()["active"], json!(active));
            assert_eq!(super::super::artboards::picked(&app, st.doc.artboards.len()), [active]);
            assert_eq!(app.session.tool_options()["selected"], json!([active]), "raw tool selection contains only the remaining active board");

            app.run("edit.undo", json!({})).unwrap();
            let st = app.session.doc().unwrap();
            assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
            assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
            assert_eq!(st.history.undo.len(), undo);
            assert!(!st.is_dirty(), "one undo returns to the saved document");
            app.run("edit.redo", json!({})).unwrap();
            let st = app.session.doc().unwrap();
            assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), expected, "redo removes the same artboard IDs");
            assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), artwork);
            assert_eq!(st.history.undo.len(), undo + 1);
            assert!(st.is_dirty());
        }
    }

    fn assert_artboard_deletion_guard(draw: fn(&mut VectorcraftApp, &mut Ui), label: Option<&str>) {
        let mut app = artboard_deletion_app(1);
        // Leave a redo entry to ensure a disabled action preserves both history branches.
        app.run("artboard.setProps", json!({"index": 0, "name": "Temporary"})).unwrap();
        app.run("edit.undo", json!({})).unwrap();
        let ctx = egui::Context::default();
        select_artboard_rows(&mut app, &ctx, &[0]);
        app.session.doc_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let st = app.session.doc().unwrap();
        let history = (st.history.undo.len(), st.history.redo.len());
        assert!(history.1 > 0);
        let active = app.view().unwrap().artboard;
        let options = app.session.tool_options();

        let commands = app.run_count;
        artboard_deletion_click(&mut app, &ctx, draw, label, egui::Modifiers::NONE);
        assert_eq!(app.run_count, commands, "the disabled last-board control dispatches no command");
        let st = app.session.doc().unwrap();
        assert_eq!(serde_json::to_value(&st.doc.artboards).unwrap(), serde_json::to_value(&before.artboards).unwrap());
        assert_eq!(serde_json::to_value(&st.doc.layers).unwrap(), serde_json::to_value(&before.layers).unwrap());
        assert_eq!((st.history.undo.len(), st.history.redo.len()), history);
        assert!(!st.is_dirty(), "deleting the last artboard is a no-op");
        assert_eq!(app.view().unwrap().artboard, active);
        assert_eq!(app.session.tool_options(), options);
    }

    #[test]
    fn artboard_menu_deletes_selected_artboards_in_one_undo_step() {
        assert_artboard_deletion(super::super::artboards::menu, Some("Delete Artboards"));
    }

    #[test]
    fn properties_deletes_selected_artboards_in_one_undo_step() {
        assert_artboard_deletion(show, Some("Delete Artboard"));
    }

    #[test]
    fn artboard_menu_preserves_last_artboard() {
        assert_artboard_deletion_guard(super::super::artboards::menu, Some("Delete Artboards"));
    }

    #[test]
    fn properties_preserves_last_artboard() {
        assert_artboard_deletion_guard(show, Some("Delete Artboard"));
    }

    #[test]
    fn artboard_trash_deletes_selected_artboards_in_one_undo_step() {
        assert_artboard_deletion(super::super::artboards::show, None);
    }

    #[test]
    fn artboard_trash_preserves_last_artboard() {
        assert_artboard_deletion_guard(super::super::artboards::show, None);
    }

    /// Click the actual panel footer's New button (24 points, with a 4-point gap before Trash).
    fn artboard_history_new(app: &mut VectorcraftApp, ctx: &egui::Context) {
        let mut bounds = Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                super::super::artboards::show(app, ui);
                bounds = ui.min_rect();
            });
            out.textures_delta.clear();
        }
        let count = app.session.doc().unwrap().doc.artboards.len();
        let commands = app.run_count;
        let at = bounds.right_bottom() - vec2(40.0, 12.0);
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        panel_frame(app, ctx, vec![Event::PointerMoved(at), button(true)], super::super::artboards::show);
        panel_frame(app, ctx, vec![button(false)], super::super::artboards::show);
        assert_eq!(app.run_count, commands.wrapping_add(1), "the actual New button dispatches once");
        assert_eq!(app.session.doc().unwrap().doc.artboards.len(), count + 1);
        assert_eq!(app.session.tool_options()["active"], json!(count));
        assert_eq!(app.view().unwrap().artboard, count);
    }

    /// Keep the panel and its natural focus in the same context throughout Undo and deletion.
    fn artboard_history_key(app: &mut VectorcraftApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
        for pressed in [true, false] {
            let events = vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }];
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(300.0, 700.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                crate::shortcuts::handle(app, ui.ctx());
                super::super::artboards::show(app, ui);
            });
            out.textures_delta.clear();
        }
    }

    fn artboard_history_delete_after_undo(key: Option<egui::Key>, duplicate: bool) {
        let mut app = artboard_deletion_app(2);
        app.session.set_tool_option("moveArt", &json!(false));
        app.session.set_tool_option("scaleArt", &json!(true));
        app.session.doc_mut().unwrap().mark_saved();
        let st = app.session.doc().unwrap();
        let before = st.doc.clone();
        let selection = st.selection.clone();
        assert!(!selection.is_empty(), "selected artwork catches a Clear fallthrough");
        let undo = st.history.undo.len();
        let ctx = egui::Context::default();
        if duplicate {
            let commands = app.run_count;
            artboard_deletion_click(&mut app, &ctx, super::super::artboards::menu, Some("Duplicate Artboards"), egui::Modifiers::NONE);
            assert_eq!(app.run_count, commands.wrapping_add(1));
            assert_eq!(app.session.doc().unwrap().doc.artboards.len(), 3);
            assert_eq!(app.session.tool_options()["active"], json!(2));
            assert_eq!(app.view().unwrap().artboard, 2);
        } else {
            artboard_history_new(&mut app, &ctx);
        }
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before, "the real Undo shortcut restores the original document");
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        // Capture the state before another panel or pointer can repair it. Still exercise deletion
        // on RED: keyboard deletion used to do nothing; Properties already clamped its own target.
        let after_undo = (app.view().unwrap().artboard, app.session.tool_options());
        if let Some(key) = key {
            artboard_history_key(&mut app, &ctx, key, egui::Modifiers::NONE);
        } else {
            let texts = panel_frame(&mut app, &ctx, vec![], show);
            assert!(texts.iter().any(|(label, _)| label == &before.artboards[1].name));
            artboard_deletion_click(&mut app, &ctx, show, Some("Delete Artboard"), egui::Modifiers::NONE);
        }
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc.artboards, vec![before.artboards[0].clone()], "deletion after Undo removes the shown second board");
        assert_eq!(st.doc.layers, before.layers, "ordinary artwork survives without geometry or style changes");
        assert_eq!(st.selection, selection, "deletion never falls through to Clear");
        assert_eq!(after_undo.0, 1, "Undo normalizes the navigator before another gesture");
        assert_eq!(after_undo.1, json!({"active": 1, "selected": [1], "moveArt": false, "scaleArt": true}));
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
        assert!(st.is_dirty());
        assert_eq!(app.view().unwrap().artboard, 0);
        assert_eq!(app.session.tool_options(), json!({"active": 0, "selected": [0], "moveArt": false, "scaleArt": true}));
        let deleted = st.doc.clone();

        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        // Valid current selection stays put; history does not restore an earlier artboard selection.
        assert_eq!(app.session.tool_options()["selected"], json!([0]));
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, deleted);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        assert!(st.is_dirty());
    }

    #[test]
    fn artboard_history_new_undo_delete_uses_remaining_board() {
        artboard_history_delete_after_undo(Some(egui::Key::Delete), false);
    }

    #[test]
    fn artboard_history_new_undo_backspace_uses_remaining_board() {
        artboard_history_delete_after_undo(Some(egui::Key::Backspace), false);
    }

    #[test]
    fn artboard_history_new_undo_properties_agrees_with_tool() {
        artboard_history_delete_after_undo(None, false);
    }

    #[test]
    fn artboard_history_duplicate_undo_delete_uses_remaining_board() {
        artboard_history_delete_after_undo(Some(egui::Key::Delete), true);
    }

    #[test]
    fn artboard_history_undo_prunes_only_removed_indices() {
        for order in [[0, 1, 2], [0, 2, 1]] {
            let mut app = artboard_deletion_app(2);
            app.session.set_tool_option("moveArt", &json!(false));
            app.session.set_tool_option("scaleArt", &json!(true));
            app.session.doc_mut().unwrap().mark_saved();
            let st = app.session.doc().unwrap();
            let before = st.doc.clone();
            let selection = st.selection.clone();
            let undo = st.history.undo.len();
            let ctx = egui::Context::default();
            artboard_history_new(&mut app, &ctx);
            let created = app.session.doc().unwrap().doc.clone();
            for (at, i) in order.into_iter().enumerate() {
                let modifiers = if at == 0 { egui::Modifiers::NONE } else { egui::Modifiers::COMMAND };
                artboard_deletion_click(&mut app, &ctx, super::super::artboards::show, Some(&format!("Artboard {}", i + 1)), modifiers);
            }
            let options = app.session.tool_options();
            let expected = if order[2] == 2 { json!([0, 1]) } else { json!([1, 0]) };
            assert_eq!(options["selected"], if order[2] == 2 { json!([2, 0, 1]) } else { json!([1, 0, 2]) });
            artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
            let mut normalized = options;
            normalized["active"] = expected[0].clone();
            normalized["selected"] = expected;
            assert_eq!(app.session.tool_options(), normalized, "only the removed index is pruned, preserving surviving order and options");
            assert_eq!(json!(app.view().unwrap().artboard), normalized["active"]);
            let st = app.session.doc().unwrap();
            assert_eq!(st.doc, before);
            assert_eq!(st.selection, selection);
            assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
            assert!(!st.is_dirty());
            artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
            assert_eq!(app.session.doc().unwrap().doc, created);
            assert_eq!(app.session.doc().unwrap().selection, selection);
            assert_eq!(app.session.tool_options(), normalized, "Redo does not replace an already valid multiselection");
        }
    }

    #[test]
    fn artboard_history_valid_multiselection_survives_undo_redo() {
        let mut app = artboard_deletion_app(3);
        app.session.set_tool_option("moveArt", &json!(false));
        app.session.set_tool_option("scaleArt", &json!(true));
        let ctx = egui::Context::default();
        select_artboard_rows(&mut app, &ctx, &[0, 1]);
        let options = app.session.tool_options();
        assert_eq!(options["selected"], json!([1, 0]));
        let view = app.view().unwrap().artboard;
        app.session.doc_mut().unwrap().mark_saved();
        let st = app.session.doc().unwrap();
        let before = st.doc.clone();
        let selection = st.selection.clone();
        let undo = st.history.undo.len();
        app.run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
        let moved = app.session.doc().unwrap().doc.clone();
        assert_ne!(moved, before);
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND);
        app.sync_views();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, before);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, 1));
        assert!(!st.is_dirty());
        assert_eq!(app.session.tool_options(), options);
        assert_eq!(app.view().unwrap().artboard, view);
        artboard_history_key(&mut app, &ctx, egui::Key::Z, egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
        app.sync_views();
        let st = app.session.doc().unwrap();
        assert_eq!(st.doc, moved);
        assert_eq!(st.selection, selection);
        assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo + 1, 0));
        assert!(st.is_dirty());
        assert_eq!(app.session.tool_options(), options);
        assert_eq!(app.view().unwrap().artboard, view);
    }

    /// #681: Rearrange All opens Rearrange All Artboards in one click, from the Document section
    /// (only with several artboards) and from Edit Artboards' Quick Actions.
    #[test]
    fn rearrange_all_opens_the_dialog() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        assert!(!frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Rearrange All"), "one artboard: nothing to rearrange");
        app.run("artboard.new", json!({})).unwrap();
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Rearrange All");
        let kind = |app: &VectorcraftApp| app.ui.dialog.as_ref().map(|d| d.kind.clone());
        assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::rearrange_artboards::KIND));
        app.ui.dialog = None;
        app.select_tool("artboard");
        let texts = frame(&mut app, &ctx, vec![]);
        click(&mut app, &ctx, &texts, "Rearrange All");
        assert_eq!(kind(&app).as_deref(), Some(crate::dialogs::rearrange_artboards::KIND));
    }

    /// #671: Edit Artboards shows the artboard's preset (Custom when it matches none) and its
    /// orientation; the other orientation swaps its width and height, its corner staying put.
    #[test]
    fn edit_artboards_shows_the_preset_and_flips_the_orientation() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"preset": "Letter"})).unwrap();
        app.run("artboard.setProps", json!({"index": 0, "x": 10, "y": 20})).unwrap();
        app.select_tool("artboard");
        let ctx = egui::Context::default();
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "Preset:") && texts.iter().any(|(t, _)| t == "Letter"), "{texts:?}");
        // The row's clickable widgets, left to right: the preset combo, Portrait, Landscape.
        let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
        let buttons: Vec<Rect> = ctx.viewport(|vp| {
            let mut r: Vec<Rect> = vp
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
                .map(|w| w.rect)
                .collect();
            r.sort_by(|a, b| a.left().total_cmp(&b.left()));
            r
        });
        let landscape = buttons.last().unwrap().center();
        let press = |pressed| Event::PointerButton { pos: landscape, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![Event::PointerMoved(landscape), press(true)]);
        frame(&mut app, &ctx, vec![press(false)]);
        let rect = app.session.active().unwrap().doc.artboards[0].rect;
        assert_eq!((rect.x0, rect.y0, rect.width(), rect.height()), (10.0, 20.0, 792.0, 612.0), "landscape, same corner");
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "Letter"), "still Letter, turned: {texts:?}");
        app.run("artboard.setProps", json!({"index": 0, "width": 333})).unwrap();
        assert!(frame(&mut app, &ctx, vec![]).iter().any(|(t, _)| t == "Custom"));
    }

    /// The center of Edit Artboards' Portrait (`landscape` false) or Landscape button.
    fn orientation_button(ctx: &egui::Context, texts: &[(String, Rect)], landscape: bool) -> Pos2 {
        let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
        ctx.viewport(|vp| {
            let mut r: Vec<Rect> = vp
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
                .map(|w| w.rect)
                .collect();
            r.sort_by(|a, b| a.left().total_cmp(&b.left()));
            r[r.len() - if landscape { 1 } else { 2 }].center()
        })
    }

    /// #1482: with Artboards 1 and 3 selected, Portrait swaps the width and height of both in one
    /// undo step, each keeping its top-left corner; Artboard 2 stays landscape. An artboard already
    /// in that orientation keeps its size.
    #[test]
    fn edit_artboards_orientation_turns_every_selected_artboard() {
        let mut app = two_selected_artboards();
        let before = boards(&app);
        let undo = app.session.doc().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        let portrait = |app: &mut VectorcraftApp| {
            frame(app, &ctx, vec![]);
            let texts = frame(app, &ctx, vec![]);
            let at = orientation_button(&ctx, &texts, false);
            let press = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Default::default() };
            frame(app, &ctx, vec![Event::PointerMoved(at), press(true)]);
            frame(app, &ctx, vec![press(false)]);
        };
        portrait(&mut app);
        let mut want = before.clone();
        for i in [0, 2] {
            (want[i].2, want[i].3) = (before[i].3, before[i].2);
        }
        assert_eq!(boards(&app), want);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(boards(&app), before);
        // Artboard 3 portrait already: Portrait turns Artboard 1 alone.
        app.run("artboard.setProps", json!({"index": 2, "width": 150, "height": 300})).unwrap();
        let before = boards(&app);
        portrait(&mut app);
        let mut want = before.clone();
        (want[0].2, want[0].3) = (100.0, 200.0);
        assert_eq!(boards(&app), want);
    }

    /// #1482: with Artboards 1 and 3 selected, a preset chosen in Edit Artboards sizes both in one
    /// undo step, each in its own orientation and keeping its top-left corner: Letter (612 × 792 pt)
    /// stays portrait for Artboard 1 and turns landscape for Artboard 3. Artboard 2 keeps its size.
    #[test]
    fn edit_artboards_preset_sizes_every_selected_artboard() {
        let mut app = two_selected_artboards();
        app.run("artboard.setProps", json!({"index": 0, "width": 100, "height": 200})).unwrap();
        let before = boards(&app);
        let undo = app.session.doc().unwrap().history.undo.len();
        let ctx = egui::Context::default();
        // A frame on a wide screen, with the texts of nested shapes too (the open list and its
        // submenu), and the center of `label` among them (menu items carry a check-mark column).
        let run = |app: &mut VectorcraftApp, events: Vec<Event>| {
            fn walk(s: &egui::Shape, v: &mut Vec<(String, Rect)>) {
                match s {
                    egui::Shape::Text(t) => v.push((t.galley.text().to_string(), Rect::from_min_size(t.pos, t.galley.size()))),
                    egui::Shape::Vec(s) => s.iter().for_each(|s| walk(s, v)),
                    _ => {}
                }
            }
            let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 700.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| show(app, ui));
            out.textures_delta.clear();
            let mut v = vec![];
            out.shapes.iter().for_each(|c| walk(&c.shape, &mut v));
            v
        };
        let at = |texts: &[(String, Rect)], label: &str| {
            texts.iter().find(|(t, _)| t.trim() == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no {label:?} in {texts:?}"))
        };
        let press = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        run(&mut app, vec![]);
        let custom = at(&run(&mut app, vec![]), "Custom");
        run(&mut app, vec![Event::PointerMoved(custom), press(custom, true)]);
        run(&mut app, vec![press(custom, false)]);
        // Hovering Print opens its submenu.
        let print = at(&run(&mut app, vec![]), "Print");
        run(&mut app, vec![Event::PointerMoved(print)]);
        let letter = at(&run(&mut app, vec![]), "Letter");
        run(&mut app, vec![Event::PointerMoved(letter), press(letter, true)]);
        run(&mut app, vec![press(letter, false)]);
        let mut want = before.clone();
        (want[0].2, want[0].3) = (612.0, 792.0);
        (want[2].2, want[2].3) = (792.0, 612.0);
        assert_eq!(boards(&app), want);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    }

    /// #602: Edit Artboards shows Move and Scale Artwork with Artboard; with Scale on, a new size
    /// (here the other orientation) takes the art on the artboard along.
    #[test]
    fn edit_artboards_scales_the_art_with_scale_artwork_with_artboard() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let r = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
        app.select_tool("artboard");
        let ctx = egui::Context::default();
        let texts = frame(&mut app, &ctx, vec![]);
        assert!(texts.iter().any(|(t, _)| t == "Move Artwork with Artboard"), "{texts:?}");
        assert_eq!(app.session.tool_options()["scaleArt"], false);
        click(&mut app, &ctx, &texts, "Scale Artwork with Artboard");
        assert_eq!(app.session.tool_options()["scaleArt"], true);
        // Portrait: 100 × 200, the art scaled by ½ across and 2 down.
        let texts = frame(&mut app, &ctx, vec![]);
        let row = texts.iter().find(|(t, _)| t == "Preset:").map(|(_, r)| *r).unwrap();
        let portrait = ctx.viewport(|vp| {
            let mut r: Vec<Rect> = vp
                .prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.sense.senses_click() && w.rect.y_range().contains(row.center().y) && w.rect.left() > row.right())
                .map(|w| w.rect)
                .collect();
            r.sort_by(|a, b| a.left().total_cmp(&b.left()));
            r[r.len() - 2].center()
        });
        let press = |pressed| Event::PointerButton { pos: portrait, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        frame(&mut app, &ctx, vec![Event::PointerMoved(portrait), press(true)]);
        frame(&mut app, &ctx, vec![press(false)]);
        let d = &app.session.active().unwrap().doc;
        let ab = d.artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (100.0, 200.0));
        let b = d.node(vectorcraft_doc::NodeId(r)).unwrap().geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.width(), b.height()), (10.0, 40.0, 20.0, 80.0));
    }
}
