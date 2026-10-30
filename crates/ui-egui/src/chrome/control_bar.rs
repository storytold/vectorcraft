use super::{NodeKind, Sense, Stroke, Tokens, Ui, VectorcraftApp, json, stroke_panel, theme, vec2, widgets};

/// An anchor button: icon, tooltip, command and the convert command's `to`.
type AnchorButton<'a> = (&'a str, &'a str, &'a str, Option<&'a str>);

/// The anchor controls the Control bar and the Properties panel show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnchorControls {
    None,
    /// Paths selected as a whole with a tool that edits anchors (Direct Selection, the Pen…),
    /// which shows all their anchors selected: "Convert:" only, as removing or cutting at every
    /// anchor isn't what they're for.
    Convert,
    /// Anchors direct-selected: "Convert:" and "Anchors:".
    All,
}

/// Which anchor controls show for the selection and the active tool.
pub fn anchor_controls(app: &VectorcraftApp) -> AnchorControls {
    let Some(st) = app.session.active() else {
        return AnchorControls::None;
    };
    if !st.selection.anchors.is_empty() {
        return AnchorControls::All;
    }
    let paths = st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. })));
    if paths && vectorcraft_tools::catalog::edits_anchors(app.session.tool_id()) { AnchorControls::Convert } else { AnchorControls::None }
}

/// What the Control bar and the Properties panel offer for selected anchors ([`anchor_controls`]):
/// "Convert:" corner or smooth, then "Anchors:" remove, connect (Join) and cut. Each group is a
/// row of its own: inline in the Control bar, one under the other in a panel.
pub fn anchor_buttons(app: &mut VectorcraftApp, ui: &mut Ui, controls: AnchorControls) {
    let t = Tokens::get(ui.ctx());
    let mut run = None;
    let groups: [(&str, &[AnchorButton]); 2] = [
        (
            tl!("Convert:"),
            &[
                ("dc-anchor", tl!("Convert Selected Anchor Points to Corner"), "path.convertAnchors", Some("corner")),
                ("dc-anchor-smooth", tl!("Convert Selected Anchor Points to Smooth"), "path.convertAnchors", Some("smooth")),
            ],
        ),
        (
            tl!("Anchors:"),
            &[
                ("pen-tool-delete", tl!("Remove Anchor Points"), "path.removeAnchors", None),
                ("dc-join", tl!("Connect Selected End Points"), "path.join", None),
                ("scissors", tl!("Cut Path at Selected Anchor Points"), "path.cutAtAnchors", None),
            ],
        ),
    ];
    let shown = match controls {
        AnchorControls::None => 0,
        AnchorControls::Convert => 1,
        AnchorControls::All => groups.len(),
    };
    for (label, buttons) in groups.into_iter().take(shown) {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).size(12.0).color(t.text));
            for &(icon, tip, id, to) in buttons {
                if widgets::icon_button(ui, icon, tip, false, 24.0).clicked() {
                    run = Some((id, to.map_or_else(|| json!({}), |to| json!({ "to": to }))));
                }
            }
        });
    }
    if let Some((id, p)) = run {
        crate::menus::invoke(app, id, p);
    }
}

/// The id of the Control bar's isolation button.
pub(super) const ISOLATION_BUTTON: &str = "cb-isolation";

/// The room the Control bar needs for the Snapping menu next to the Artboard tool's controls; with
/// less, the menu is left out so those controls still fit the narrowest window.
const ARTBOARD_SNAPPING_WIDTH: f32 = 900.0;

/// The Control bar's snapping quick access; every change uses its existing command.
fn snapping_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    ui.menu_button(tl!("Snapping"), |ui| {
        for (label, cmd, on) in [
            (tl!("Smart Guides"), "view.smartGuides", app.ui.view.smart_guides),
            (tl!("Snap to Point"), "view.snapToPoint", app.ui.view.snap_to_point),
            (tl!("Snap to Grid"), "view.snapToGrid", app.ui.view.snap_to_grid),
            (tl!("Snap to Pixel"), "view.snapToPixel", app.ui.view.snap_to_pixel),
        ] {
            if widgets::check(ui, label, on, true) {
                crate::menus::invoke(app, cmd, json!({}));
            }
        }
        ui.separator();
        for (key, label, on) in [
            ("snapTangent", tl!("Snap to Tangent"), app.session.prefs.snap_tangent),
            ("snapPerpendicular", tl!("Snap to Perpendicular"), app.session.prefs.snap_perpendicular),
        ] {
            if widgets::check(ui, label, on, true) {
                crate::menus::invoke(app, "prefs.set", json!({"key": key, "value": !on}));
            }
        }
    });
}

/// The Control bar's isolation button. In isolation mode it is Exit Isolation Mode, except while a
/// pattern is edited (the isolation bar's Done and Cancel end that). Otherwise, with one object
/// selected that [`vectorcraft_tools::select::isolates`], it is Isolate Selected Object, which
/// also isolates a path inside a group that Direct Selection selected. An envelope's contents
/// get no button: the envelope's Edit Envelope and Edit Contents buttons show instead.
fn isolation_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let (icon, tip, id) = if st.isolation.is_some() {
        if st.doc.pattern_edit.is_some() {
            return;
        }
        ("chevron-left", tl!("Exit Isolation Mode"), "object.exitIsolation")
    } else if let [one] = st.selection.objects[..]
        && st.doc.node(one).is_some_and(|n| vectorcraft_tools::select::isolates(&n.kind))
        && vectorcraft_doc::live::envelope_of(&st.doc, one).is_none()
    {
        ("square-dashed", tl!("Isolate Selected Object"), "object.isolate")
    } else {
        return;
    };
    let (rect, _) = ui.allocate_exact_size(vec2(24.0, 24.0), Sense::hover());
    let resp = ui.interact(rect, egui::Id::unique(ISOLATION_BUTTON), Sense::click());
    if widgets::paint_icon_button(ui, resp, icon, tip, false).clicked() {
        crate::menus::invoke(app, id, json!({}));
    }
}

/// The room the Control bar's inline X/Y/W/H fields need; with less, only the Transform link shows.
const INLINE_TRANSFORM_WIDTH: f32 = 440.0;

/// The Control bar (Window → Control), context-sensitive like Illustrator's.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::top("control_bar")
        .exact_size(34.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                // Gripper
                let (g, _) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::hover());
                for i in 0..5 {
                    ui.painter().circle_filled(g.center_top() + vec2(0.0, 2.0 + i as f32 * 4.0), 0.9, t.text_disabled);
                }
                if app.session.active().is_none() {
                    ui.label(egui::RichText::new(tl!("No Document")).color(t.text_dim));
                    return;
                }
                // The Artboard tool's controls fill the narrowest window, so they keep it to themselves there.
                if app.session.tool_id() != "artboard" || ui.available_width() >= ARTBOARD_SNAPPING_WIDTH {
                    snapping_menu(app, ui);
                }
                let Some(st) = app.session.active() else { return };
                let sel = st.selection.objects.clone();
                let units = app.session.general_unit();
                let first = sel.first().and_then(|id| st.doc.node(*id)).cloned();
                let anchors = anchor_controls(app);
                let label = match &first {
                    _ if app.session.tool_id() == "artboard" => tl!("Artboard"),
                    Some(_) if sel.len() == 1 && anchors != AnchorControls::None => {
                        tl!("Anchor Point")
                    }
                    Some(vectorcraft_doc::Node { kind: NodeKind::Image(im), .. }) if sel.len() == 1 => {
                        if im.link.is_some() {
                            tl!("Linked File")
                        } else {
                            tl!("Embedded")
                        }
                    }
                    Some(n) if sel.len() == 1 && crate::panels::image_trace::is_trace(n) => {
                        tl!("Image Tracing")
                    }
                    _ => tl!(crate::panels::appearance::object_label(app)),
                };
                ui.label(egui::RichText::new(label).font(theme::semibold(12.0)).color(t.text));
                // With the Artboard tool in use, the bar shows the active artboard's controls in place
                // of the object controls.
                if crate::panels::artboards::control_bar(app, ui) {
                    return;
                }
                if anchors != AnchorControls::None {
                    anchor_buttons(app, ui, anchors);
                }
                isolation_button(app, ui);
                ui.add_space(6.0);
                // An image or an Image Trace object shows its own controls in place of Fill and Stroke.
                let image = crate::place::control_bar_details(app, ui) || crate::panels::image_trace::control_bar(app, ui);
                crate::toolbar::control_bar_options(app, ui);
                crate::dialogs::envelope::control_bar(app, ui);
                crate::panels::symbols::control_bar(app, ui);
                let opacity = if first.is_some() { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
                if !image {
                    let shown_stroke = crate::panels::current_stroke(app);
                    let mixed = crate::panels::stroke_mixed(app, ui.ctx());
                    let weight = stroke_panel::shown_weight(app, shown_stroke.as_ref(), &mixed);
                    crate::panels::paint_chip(app, ui, false, 22.0, true);
                    crate::panels::paint_chip(app, ui, true, 22.0, true);
                    // The link opens the Stroke panel as a popover under it; then the weight spinner
                    // (with presets) and the width profile.
                    stroke_panel::link(app, ui, tl!("Stroke:"));
                    stroke_panel::weight_field(app, ui, "cb-stroke", weight, 100.0);
                    if let Some(id) = stroke_panel::profile_dropdown(app, ui, shown_stroke.as_ref().and_then(|s| s.profile.as_ref())) {
                        app.run("stroke.set", json!({"profile": id})).ok();
                    }
                    ui.add_space(4.0);
                    ui.separator();
                }
                if ui.link(egui::RichText::new(tl!("Opacity:")).size(12.0).color(t.text).underline()).clicked() {
                    app.ui.open_panel = Some("transparency".into());
                }
                if let Some(o) = widgets::plain_field(ui, "cb-opacity", opacity as f64 * 100.0, "%", 0, 56.0)
                    && !sel.is_empty()
                {
                    app.run("transparency.set", json!({"opacity": o.clamp(0.0, 100.0)})).ok();
                }
                if !sel.is_empty() {
                    // Style picker: the selection's graphic style; its menu applies another.
                    ui.add_space(4.0);
                    if ui.link(egui::RichText::new(tl!("Style:")).size(12.0).color(t.text).underline()).clicked() {
                        app.ui.open_panel = Some("graphicStyles".into());
                    }
                    let resp = widgets::chip_button_named(ui, 22.0, true, tl!("Graphic Style"), |ui, r| {
                        crate::panels::graphic_styles::paint_linked(app, ui, r)
                    });
                    egui::Popup::menu(&resp).show(|ui| crate::panels::graphic_styles::picker(app, ui));
                    // Recolor Artwork, when the selection has colours to recolour.
                    let colored = app.session.active().is_some_and(|st| crate::canvas::has_colors(&st.doc, &st.selection.objects));
                    if colored && widgets::icon_button(ui, "palette", tl!("Recolor Artwork"), false, 24.0).clicked() {
                        crate::menus::invoke(app, "ui.recolorDialog", json!({}));
                    }
                }
                ui.separator();
                crate::panels::character::control_bar(app, ui);
                if sel.is_empty() {
                    if widgets::flat_button(ui, tl!("Document Setup"), 112.0).clicked() {
                        app.run("file.documentSetup", json!({})).ok();
                    }
                    if widgets::flat_button(ui, tl!("Preferences"), 90.0).clicked() {
                        app.run("edit.preferences", json!({})).ok();
                    }
                    return;
                }
                crate::panels::align::align_buttons(app, ui, 24.0);
                // Two or more objects get the six Distribute Objects buttons too; a single
                // object keeps the bar as it was.
                if sel.len() > 1 {
                    ui.add_space(4.0);
                    crate::panels::align::distribute_buttons(app, ui, 24.0);
                }
                ui.separator();
                // The Transform link opens the whole Transform panel (reference point, rotate,
                // shear, options) in a popover; X/Y/W/H follow inline while the bar has room.
                crate::panels::transform::link(app, ui);
                // The bounding box, rotated with rotated objects: its centre and its own sides.
                if let Some(b) = app.selection_box()
                    && ui.available_width() >= INLINE_TRANSFORM_WIDTH
                {
                    let c = b.center();
                    let origin = app.ruler_origin();
                    let link = crate::panels::transform::proportional(app, ui);
                    let copy = crate::panels::transform::alt_enter_copies(app, ui);
                    for (k, lbl, v, offset) in [
                        ("x", "X:", c.x, origin.x),
                        ("y", "Y:", c.y, origin.y),
                        ("width", "W:", b.rect.width(), 0.0),
                        ("height", "H:", b.rect.height(), 0.0),
                    ] {
                        // The W/H link sits between W and H.
                        if k == "height" {
                            crate::panels::transform::constrain_link(app, ui);
                        }
                        widgets::field_label(ui, egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim));
                        let (id, shown) = (("cb", k), Some(v - offset));
                        let set = if matches!(k, "width" | "height") {
                            widgets::size_field(ui, id, shown, units, 80.0)
                        } else {
                            widgets::num_field(ui, id, shown, units, 80.0)
                        };
                        if let Some(nv) = set {
                            app.run("object.setBounds", json!({k: nv + offset, "reference": 4, "proportional": link, "copy": copy})).ok();
                        }
                    }
                }
            });
        });
}
