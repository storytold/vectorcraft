//! The Properties panel: context-sensitive sections like Illustrator's.

use egui::Ui;
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{LiveCorners, LiveShape, NodeKind, Unit, WidthProfile};
use vectorcraft_engine::cmd::newdoc;

use super::{corner_radius_row, first_selected, pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, dim_label, divider, section_header};
use crate::{VectorcraftApp, icons};

/// The panel for what is selected: the selection's kind as its heading, then its sections (Edit
/// Artboards' with the Artboard tool, the document's with nothing selected).
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
    if n_sel == 1
        && let Some(NodeKind::Group { clip: true, children }) = first.as_ref().map(|n| &n.kind)
    {
        clip_group_buttons(app, ui, children.len() > 1);
        divider(ui);
    }
    transform_section(app, ui);
    corner_rows(app, ui);
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
    // Blend Options for one blend while the Blend tool is in use, Merge Live Paint for a Live
    // Paint group selected with more to merge into it, as the Properties panel overview lists them
    // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>).
    let blend_options = n_sel == 1 && app.session.tool_id() == "blend" && matches!(first.as_ref().map(|n| &n.kind), Some(NodeKind::Blend { .. }));
    let merge_live_paint = n_sel > 1 && can_merge_live_paint(app);
    let mut actions: Vec<(&str, &str)> = vec![];
    if n_sel > 1 {
        actions.push((tl!("Group"), "object.group"));
        actions.push((tl!("Make Clipping Mask"), "object.clippingMask.make"));
    }
    if merge_live_paint {
        actions.push((tl!("Merge Live Paint"), "livePaint.merge"));
    }
    if blend_options {
        actions.push((tl!("Blend Options"), "ui.blendOptions"));
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
    global_edit_row(app, ui);
}

/// Whether Merge Live Paint can take the selection: a Live Paint group among the selected objects
/// and nothing Live Paint refuses (type, images and the rest; `livePaint.merge` refuses them too).
fn can_merge_live_paint(app: &VectorcraftApp) -> bool {
    use vectorcraft_tools::builder::{is_live_paint, live_paint_takes};
    app.session.active().is_some_and(|st| {
        let nodes = || st.selection.objects.iter().filter_map(|id| st.doc.node(*id));
        nodes().any(is_live_paint) && nodes().all(live_paint_takes)
    })
}

/// The Corners of the one path whose corners Direct Selection has selected: their kind and radius,
/// which `object.setLiveShape` sets on those corners. The Properties panel overview shows the
/// control while the Direct Selection tool is chosen and a corner of an object is selected
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>).
/// A live rectangle's and a live polygon's own rows already set the selected corners.
fn corner_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.tool_id() != "directSelection" {
        return;
    }
    let Some(st) = app.session.active() else { return };
    let [id] = st.selection.objects[..] else { return };
    let Some(n) = st.doc.node(id) else { return };
    if matches!(n.live_shape(), Some(LiveShape::Rectangle { .. } | LiveShape::Polygon { .. })) {
        return;
    }
    let Some(lc) = LiveCorners::of(n) else { return };
    if st.selection.partial(id).is_none_or(|a| lc.corners_of(a).is_empty()) {
        return;
    }
    let n = n.clone();
    let kind = super::corner_style(app, &n).1;
    let kinds: Vec<&str> = vectorcraft_geom::shapes::CornerKind::ALL.iter().map(|k| crate::dialogs::corners::kind_label(*k)).collect();
    let fw = super::field_width(ui);
    egui::Grid::new("props-corners").num_columns(2).spacing([6.0, 6.0]).show(ui, |ui| {
        dim_label(ui, tl!("Corner:"));
        let picked = widgets::dropdown_names(ui, ("props-corners", "kind"), kind.map_or("", crate::dialogs::corners::kind_label), &kinds, fw);
        if let Some(k) = picked.and_then(|i| vectorcraft_geom::shapes::CornerKind::ALL.get(i)) {
            app.run("object.setLiveShape", json!({"kind": k})).ok();
        }
        ui.end_row();
        dim_label(ui, tl!("Corner Radius:"));
        super::corner_radius_field(app, ui, &n, ("props-corners", "radius"), fw);
        ui.end_row();
    });
}

/// Start Global Edit (Stop Global Edit while it is on) with the Global Edit Options beside it, in
/// Quick Actions as the help page places them
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>).
fn global_edit_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let on = app.session.active().is_some_and(|d| d.global_edit.is_some());
    if !crate::menus::enabled(app, "select.globalEdit") {
        return;
    }
    ui.horizontal(|ui| {
        let label = if on { tl!("Stop Global Edit") } else { tl!("Start Global Edit") };
        if widgets::flat_button(ui, label, ui.available_width() - 26.0).clicked() {
            crate::menus::invoke(app, "select.globalEdit", json!({}));
        }
        let options = widgets::icon_button_enabled(ui, "chevron-down", tl!("Global Edit Options"), false, !on, 20.0);
        egui::Popup::menu(&options).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| global_edit_options(app, ui));
    });
}

/// The Global Edit Options: what else similar objects must share (Match), where they may lie (On
/// Artboards) and how many objects that takes now.
fn global_edit_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.set_min_width(220.0);
    let o = app.session.global_edit_options.clone();
    let set = |app: &mut VectorcraftApp, p: serde_json::Value| crate::menus::invoke(app, "select.globalEditOptions", p);
    widgets::subheader(ui, tl!("Match"));
    if widgets::check(ui, tl!("Appearance"), o.appearance, true) {
        set(app, json!({ "appearance": !o.appearance }));
    }
    if widgets::check(ui, tl!("Size"), o.size, true) {
        set(app, json!({ "size": !o.size }));
    }
    widgets::subheader(ui, tl!("On Artboards"));
    if widgets::check(ui, tl!("All Artboards"), o.artboards.is_none(), true) {
        set(app, json!({ "artboards": if o.artboards.is_some() { "all" } else { "1" } }));
    }
    if let Some(range) = &o.artboards {
        ui.horizontal(|ui| {
            ui.label(tl!("Range:"));
            if let Some(r) = widgets::text_field(ui, "global-edit-range", Some(range), 120.0, 1) {
                set(app, json!({ "artboards": r }));
            }
        });
    }
    if widgets::check(ui, tl!("Include Objects On Canvas"), o.include_canvas, true) {
        set(app, json!({ "includeCanvas": !o.include_canvas }));
    }
    let count = global_edit_count(app, ui.ctx()).to_string();
    dim_label(ui, &crate::i18n::fmt(tl!("Result(s): {count}"), &[("count", &count)]));
}

/// The cached Result(s) count: the (document, revision, options, one selected object) it was
/// found for, and the count.
type GlobalEditCount = Option<((u64, u64, vectorcraft_engine::cmd::globaledit::Options, Option<u64>), usize)>;

/// How many objects Start Global Edit would take now (the options' Result(s) line), cached per
/// document revision, options and selected object, as finding them walks the whole document.
fn global_edit_count(app: &VectorcraftApp, ctx: &egui::Context) -> usize {
    let Some(st) = app.session.active() else { return 0 };
    let selected = match st.selection.objects.as_slice() {
        [id] => Some(id.0),
        _ => None,
    };
    let key = (st.uid, st.revision, app.session.global_edit_options.clone(), selected);
    match pstate::<GlobalEditCount>(ctx, "props-global-edit-count") {
        Some((k, count)) if k == key => count,
        _ => {
            let count = vectorcraft_engine::cmd::globaledit::match_count(&app.session);
            set_pstate::<GlobalEditCount>(ctx, "props-global-edit-count", Some((key, count)));
            count
        }
    }
}

/// The one selected clip group's Edit Clipping Path and Edit Contents, at the top of the panel as in
/// the reference app ("With the clip group selected, click the Edit Clipping Path button at the top
/// of the Properties panel to select and edit the mask, or click the Edit Contents button to select
/// and edit the masked content", <https://www.adobe.com/learn/illustrator/web/crop-using-masks>):
/// they select its clipping path or its clipped art for the selection tools, as Object › Clipping
/// Mask does. Edit Contents only when the group clips something.
fn clip_group_buttons(app: &mut VectorcraftApp, ui: &mut Ui, has_contents: bool) {
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("Edit Clipping Path"), w).clicked() {
            crate::menus::invoke(app, "object.clippingMask.editMask", json!({}));
        }
        if has_contents && widgets::flat_button(ui, tl!("Edit Contents"), w).clicked() {
            crate::menus::invoke(app, "object.clippingMask.editContents", json!({}));
        }
    });
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
/// W or H, preset or orientation goes to every selected artboard, in one undo step), its Fill (background color), Move
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
            if let Some(name) = widgets::text_field_named(ui, ("ab", "name"), Some(tl!("Name:")), Some(&ab.name), w, 1).filter(|n| !n.is_empty()) {
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
                    if let Some(v) = widgets::num_field_named(ui, ("ab", key, row), Some(tl!(label)), Some(v - offset), units, fw) {
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
        artboard_fill_row(app, ui, i, ab.color);
        ui.add_space(2.0);
        super::artboards::art_options(app, ui);
    }
    ui.add_space(4.0);
    let w = (ui.available_width() - 6.0) / 2.0;
    ui.horizontal(|ui| {
        if widgets::flat_button(ui, tl!("New Artboard"), w).clicked() && app.run("artboard.new", json!({})).is_ok() {
            super::artboards::select(app, n);
        }
        let deletable = n > 1 && !super::artboards::deletable(app, n).is_empty();
        if ui.add_enabled_ui(deletable, |ui| widgets::flat_button(ui, tl!("Delete Artboard"), w)).inner.clicked() {
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
    let Some(choice) = artboard_size_controls(app, ui, w, h) else { return };
    let scale_art = super::artboards::scale_art(app);
    let indices = super::artboards::resized_with(app, i);
    super::artboards::set_props(app, &indices, json!({"scaleArt": scale_art}), |j, r| {
        let (width, height) = choice.size_for(r.width(), r.height())?;
        Some(json!({"index": j, "width": width, "height": height}))
    })
    .ok();
}

/// A shared picker choice, applied separately to each artboard's orientation.
#[derive(Clone, Copy)]
pub(crate) enum ArtboardSizeChoice {
    Preset(f64, f64),
    Orientation(bool),
}

impl ArtboardSizeChoice {
    pub(crate) fn size_for(self, w: f64, h: f64) -> Option<(f64, f64)> {
        match self {
            Self::Preset(pw, ph) => Some(turned(w > h, pw, ph)),
            Self::Orientation(landscape) if (w > h) != landscape && w != h => Some((h, w)),
            Self::Orientation(_) => None,
        }
    }
}

/// Shared existing presets/orientation UI. Dialogs keep the chosen size in their draft until OK.
pub(crate) fn artboard_size_controls(app: &VectorcraftApp, ui: &mut Ui, w: f64, h: f64) -> Option<ArtboardSizeChoice> {
    artboard_size_controls_inner(app, ui, w, h, None).0
}

/// The dialog's shared preset/orientation picker with optional artwork and selection fits.
/// The second return value is a chosen Fit: false for all artwork, true for selected artwork.
pub(crate) fn artboard_size_controls_with_fit(
    app: &VectorcraftApp,
    ui: &mut Ui,
    w: f64,
    h: f64,
    enabled: [bool; 2],
) -> (Option<ArtboardSizeChoice>, Option<bool>) {
    artboard_size_controls_inner(app, ui, w, h, Some(enabled))
}

/// Properties and Control keep their ordinary picker; Artboard Options adds two Fit entries.
fn artboard_size_controls_inner(
    app: &VectorcraftApp,
    ui: &mut Ui,
    w: f64,
    h: f64,
    fits: Option<[bool; 2]>,
) -> (Option<ArtboardSizeChoice>, Option<bool>) {
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
    let mut fit = None;
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Preset:"));
        preset = widgets::combo(ui, "ab-preset", &current, 140.0, false, |ui| {
            let mut chosen = None;
            if let Some(enabled) = fits {
                for (selection, label) in [(false, tl!("Fit to Artwork Bounds")), (true, tl!("Fit to Selected Art"))] {
                    if widgets::menu_item_name(ui, label, enabled[usize::from(selection)], false) {
                        fit = Some(selection);
                    }
                }
                ui.separator();
            }
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
    (preset.map(|(w, h)| ArtboardSizeChoice::Preset(w, h)).or_else(|| orientation.map(ArtboardSizeChoice::Orientation)), fit)
}

/// Artboard `i`'s Fill, its background `color`: the chip shows it (the None paint when the artboard
/// has none) and a click on it opens the Color Picker for the artboard. The list sets None, White or
/// Black (in the document's color mode) or opens the Color Picker.
fn artboard_fill_row(app: &mut VectorcraftApp, ui: &mut Ui, i: usize, color: Option<Color>) {
    let fills = super::artboards::fill_colors(app);
    let shown = super::artboards::fill_name(app, color);
    let mut options = super::artboards::FILLS.to_vec();
    options.push("Color Picker…");
    let mut set = None;
    let mut pick = false;
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Fill:"));
        let paint = color.map_or(Paint::None, Paint::solid);
        let chip = widgets::chip_button(ui, 22.0, false, |ui, r| {
            widgets::proxy_chip(ui, r, &paint, false, None);
        });
        pick = chip.on_hover_text(tl!("Color Picker…")).clicked();
        match widgets::dropdown_named(ui, "ab-fill", Some(tl!("Fill:")), shown, &options, 110.0) {
            Some(k) if k < fills.len() => set = Some(fills[k]),
            Some(_) => pick = true,
            None => {}
        }
    });
    if let Some(c) = set {
        app.run("artboard.setProps", json!({"index": i, "color": c.as_ref().map(super::color_json)})).ok();
    }
    if pick {
        app.run("ui.colorPicker", json!({"artboard": i})).ok();
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
    // The reference point, or a symbol instance's registration point: X/Y are read and set there
    // and the rotation and flips turn and mirror the selection about it, as in the Transform panel.
    let reference = super::transform::panel_reference(app, ui, &bx);
    let (refi, rp) = (reference.index, reference.point);
    let copy = super::transform::alt_enter_copies(app, ui);
    section_header(ui, tl!("Transform"));
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        if let Some(i) = widgets::reference_point(ui, refi) {
            ui.data_mut(|d| d.insert_temp(egui::Id::unique("refpt"), i));
        }
        ui.add_space(6.0);
        let link = super::transform::proportional(app, ui);
        egui::Grid::new("xf-grid").num_columns(4).spacing([4.0, 6.0]).min_col_width(0.0).show(ui, |ui| {
            dim_label(ui, "X:");
            if let Some(v) = widgets::num_field_named(ui, "tx", Some(crate::i18n::t("X:")), Some(rp.x - origin.x), units, fw) {
                super::transform::run_transform(
                    app,
                    "object.setBounds",
                    json!({"x": reference.bounds_xy(v + origin.x, 0.0).0, "reference": refi, "copy": copy}),
                );
            }
            dim_label(ui, "W:");
            if let Some(v) = widgets::size_field_named(ui, "tw", Some(tl!("W:")), Some(b.width()), units, fw) {
                super::transform::run_transform(app, "object.setBounds", json!({"width": v, "reference": refi, "proportional": link, "copy": copy}));
            }
            ui.end_row();
            dim_label(ui, "Y:");
            if let Some(v) = widgets::num_field_named(ui, "ty", Some(crate::i18n::t("Y:")), Some(rp.y - origin.y), units, fw) {
                super::transform::run_transform(
                    app,
                    "object.setBounds",
                    json!({"y": reference.bounds_xy(0.0, v + origin.y).1, "reference": refi, "copy": copy}),
                );
            }
            dim_label(ui, "H:");
            if let Some(v) = widgets::size_field_named(ui, "th", Some(tl!("H:")), Some(b.height()), units, fw) {
                super::transform::run_transform(app, "object.setBounds", json!({"height": v, "reference": refi, "proportional": link, "copy": copy}));
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
        if let Some(a) = widgets::plain_field_named(ui, "rot", Some(tl!("Rotation")), bx.angle, "°", 2, fw) {
            super::transform::run_transform(app, "object.rotate", json!({"angle": a, "absolute": true, "origin": reference.origin(), "copy": copy}));
        }
        ui.add_space(10.0);
        // Lucide's flip icons are named for their mirror line: flip-vertical-2 draws a vertical
        // line (the left-right flip), flip-horizontal-2 a horizontal one (top-bottom). The labels
        // are the reference app's Properties panel names for the same buttons (#1910).
        if widgets::icon_button(ui, "flip-vertical-2", tl!("Flip Along Horizontal Axis"), false, 24.0).clicked() {
            super::transform::run_transform(app, "object.reflect", json!({"axis": "vertical", "origin": reference.origin()}));
        }
        if widgets::icon_button(ui, "flip-horizontal-2", tl!("Flip Along Vertical Axis"), false, 24.0).clicked() {
            super::transform::run_transform(app, "object.reflect", json!({"axis": "horizontal", "origin": reference.origin()}));
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
            vectorcraft_doc::LiveShape::Ellipse { pie, .. } => {
                widgets::subheader(ui, tl!("Ellipse Properties:"));
                super::transform::pie_rows(app, ui, *pie, "props-pie");
            }
            _ => {}
        }
    }
}

fn appearance_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(n) = first_selected(app) else { return };
    section_header(ui, tl!("Appearance"));
    fill_stroke_rows(app, ui);
    width_profile_row(app, ui);
    let fw = super::field_width(ui);
    ui.horizontal(|ui| {
        widgets::field_label(ui, egui::RichText::new(tl!("Opacity")).size(12.0));
        ui.add_space(8.0);
        if let Some(o) = widgets::range_field(ui, "ap-op", n.opacity as f64 * 100.0, 0.0..=100.0, "%", 0, fw) {
            app.run("object.setProps", json!({"opacity": o})).ok();
        }
        if widgets::icon_button(ui, "ellipsis", tl!("Transparency"), false, 22.0).clicked() {
            app.ui.open_panel = Some("transparency".into());
        }
    });
    // The effects of one selected object: the fill's and stroke's as well as its own.
    let effects: Vec<(Option<usize>, usize, &str)> = if super::selection_len(app) == 1 {
        let own = n.appearance.effects.iter().enumerate().map(|(k, e)| (None, k, e.id.as_str()));
        let items =
            n.appearance.items.iter().enumerate().flat_map(|(i, it)| it.effects().iter().enumerate().map(move |(k, e)| (Some(i), k, e.id.as_str())));
        own.chain(items).collect()
    } else {
        vec![]
    };
    ui.horizontal(|ui| {
        // The fx button opens the effect menu, as the Appearance panel's does.
        let r = widgets::flat_button(ui, "fx", 34.0).on_hover_text(tl!("Add New Effect"));
        egui::Popup::menu(&r).show(|ui| super::appearance::fx_menu(app, ui));
        effect_delete(app, ui, n.id, &effects);
        if widgets::icon_button(ui, "ellipsis", tl!("Appearance panel"), false, 22.0).clicked() {
            app.ui.open_panel = Some("appearance".into());
        }
    });
}

/// The Delete icon beside the fx button, as the Properties panel overview has it
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>):
/// with one effect applied, its name and an icon that removes it (`effect.remove`); with several,
/// the icon opens the Appearance panel to remove them there. Nothing without effects.
fn effect_delete(app: &mut VectorcraftApp, ui: &mut Ui, id: vectorcraft_doc::NodeId, effects: &[(Option<usize>, usize, &str)]) {
    match effects {
        [] => {}
        [(item, k, effect)] => {
            let t = Tokens::get(ui.ctx());
            let name = super::appearance::shown_effect_label(effect);
            // A long name is cut short before the Delete and Appearance panel icons.
            let room = (ui.available_width() - 2.0 * (22.0 + ui.spacing().item_spacing.x)).max(0.0);
            ui.allocate_ui(egui::vec2(room, 22.0), |ui| ui.add(egui::Label::new(egui::RichText::new(name).size(12.0).color(t.text)).truncate()));
            if widgets::icon_button(ui, "trash-2", tl!("Remove Effect"), false, 22.0).clicked() {
                app.run("effect.remove", json!({"ids": [id.0], "item": item, "index": k})).ok();
            }
        }
        _ => {
            if widgets::icon_button(ui, "trash-2", tl!("Remove Effect"), false, 22.0).clicked() {
                app.ui.open_panel = Some("appearance".into());
            }
        }
    }
}

/// The stroke's Variable Width Profile (the Control bar's list), shown while the Width tool is in
/// use or the stroke has a variable width, as the Properties panel overview places it
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>).
fn width_profile_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let stroke = super::current_stroke(app);
    let profile = stroke.as_ref().and_then(|s| s.profile.as_ref());
    if app.session.tool_id() != "width" && WidthProfile::id_of(profile) == "uniform" {
        return;
    }
    ui.horizontal(|ui| {
        dim_label(ui, tl!("Profile:"));
        if let Some(id) = super::stroke::profile_dropdown(app, ui, profile) {
            app.run("stroke.set", json!({"profile": id})).ok();
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
mod tests;
