use super::{NodeKind, Sense, Stroke, Tokens, Ui, VectorcraftApp, json, stroke_panel, theme, vec2, widgets};

/// An anchor button: icon, tooltip, command and the convert command's `to`.
type AnchorButton<'a> = (&'a str, &'a str, &'a str, Option<&'a str>);

/// The anchor controls the Control bar and the Properties panel show.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnchorControls {
    None,
    /// Paths or blends selected as a whole with a tool that edits anchors (Direct Selection, the
    /// Pen…), which shows all their anchors selected: "Convert:" only, as removing or cutting at
    /// every anchor isn't what they're for. A blend converts the points of its spine.
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
    let paths =
        st.selection.objects.iter().any(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Blend { .. })));
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
            (tl!("Snap to Glyph"), "view.snapToGlyph", app.ui.view.snap_to_glyph),
        ] {
            if widgets::check(ui, label, on, true) {
                crate::menus::invoke(app, cmd, json!({}));
            }
        }
        ui.separator();
        for (key, label, on) in [
            ("alignmentGuides", tl!("Alignment Guides"), app.session.prefs.alignment_guides),
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
/// pattern is edited (the isolation bar's Done and Cancel end that), and Exit Symbol Editing Mode
/// while a symbol's art is the isolated layer. Otherwise, with one object selected that
/// [`vectorcraft_tools::select::isolates`], it is Isolate Selected Object, which also isolates a
/// path inside a group that Direct Selection selected. An envelope's contents get no button: the
/// envelope's Edit Envelope and Edit Contents buttons show instead.
fn isolation_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(st) = app.session.active() else { return };
    let (icon, tip, id) = if st.isolation.is_some() {
        if st.doc.pattern_edit.is_some() {
            return;
        }
        if st.doc.symbol_edit.as_ref().is_some_and(|e| st.isolation == Some(e.layer)) {
            ("chevron-left", tl!("Exit Symbol Editing Mode"), "symbol.exitEditing")
        } else {
            ("chevron-left", tl!("Exit Isolation Mode"), "object.exitIsolation")
        }
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

/// The size of the Control bar's Global Edit Options button.
const GLOBAL_EDIT_OPTIONS_WIDTH: f32 = 24.0;

/// The Control bar's Global Edit Options dropdown, shown while `select.globalEdit` can run. The help
/// page places the options in the Control panel as well as in Properties
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>).
/// It opens the Properties panel's Global Edit Options, which set the same session options, and is
/// off and closed while Global Edit is on, as that panel's dropdown is. It sits after Recolor
/// Artwork, ahead of the X/Y/W/H fields that the bar leaves out when it runs short of room, and is
/// left out only when no room is left for it.
fn global_edit_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !crate::menus::enabled(app, "select.globalEdit") || ui.available_width() < GLOBAL_EDIT_OPTIONS_WIDTH {
        return;
    }
    let on = app.session.active().is_some_and(|d| d.global_edit.is_some());
    let options = widgets::icon_button_enabled(ui, "chevron-down", tl!("Global Edit Options"), false, !on, GLOBAL_EDIT_OPTIONS_WIDTH);
    egui::Popup::menu(&options)
        .open_memory(crate::panels::properties::global_edit_options_open(&options, on))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| crate::panels::properties::global_edit_options(app, ui));
}

/// The width of each of the Control bar's inline X/Y/W/H fields.
const INLINE_FIELD_WIDTH: f32 = 80.0;

/// The keys and labels of the Control bar's inline X/Y/W/H fields, in their order.
const INLINE_FIELDS: [(&str, &str); 4] = [("x", "X:"), ("y", "Y:"), ("width", "W:"), ("height", "H:")];

/// The label of an inline X/Y/W/H field.
fn inline_label(t: &Tokens, lbl: &str) -> egui::RichText {
    egui::RichText::new(tl!(lbl)).size(12.0).color(t.text_dim)
}

/// The room the Control bar's inline X/Y/W/H fields take: their labels as drawn in the current
/// language, the four fields, the W/H link and the item spacing ahead of each of them. With less,
/// only the Transform link shows.
fn inline_transform_width(ui: &Ui, t: &Tokens) -> f32 {
    let labels: f32 = INLINE_FIELDS.iter().map(|(_, lbl)| widgets::text_width(ui, inline_label(t, lbl))).sum();
    let fields = INLINE_FIELD_WIDTH * INLINE_FIELDS.len() as f32;
    let items = 2 * INLINE_FIELDS.len() + 1;
    labels + fields + crate::panels::transform::CONSTRAIN_LINK_SIZE + ui.spacing().item_spacing.x * items as f32
}

/// The size of the Control bar's Align, Distribute and area type vertical alignment buttons.
const BUTTON: f32 = 24.0;

/// The space ahead of the separator that ends a group of the Control bar's controls.
const GROUP_GAP: f32 = 4.0;

/// The width of the separators of the Control bar's [`group_end`] and ahead of the Transform link.
const SEPARATOR: f32 = 6.0;

/// The widths of the Document Setup and Preferences buttons, shown with nothing selected.
const DOCUMENT_SETUP_WIDTH: f32 = 112.0;
const PREFERENCES_WIDTH: f32 = 90.0;

/// The room `n` of the bar's buttons take, with the spacing after each.
fn buttons_width(ui: &Ui, n: usize) -> f32 {
    (BUTTON + ui.spacing().item_spacing.x) * n as f32
}

/// The separator that ends a group of the Control bar's controls, as [`group_end_width`] measures it.
fn group_end(ui: &mut Ui) {
    ui.add_space(GROUP_GAP);
    separator(ui);
}

fn group_end_width(ui: &Ui) -> f32 {
    GROUP_GAP + separator_width(ui)
}

/// A separator [`SEPARATOR`] wide.
fn separator(ui: &mut Ui) {
    ui.add(egui::Separator::default().spacing(SEPARATOR));
}

fn separator_width(ui: &Ui) -> f32 {
    SEPARATOR + ui.spacing().item_spacing.x
}

/// The room the controls at the end of the Control bar need: the Transform link, or Document Setup
/// and Preferences with nothing selected. The groups ahead of them (the Paragraph section, area
/// type's vertical alignment, the Align and Distribute buttons) show only while it is left, so
/// the bar trims them from its right end (#5372), after the inline X/Y/W/H fields.
fn end_width(ui: &Ui, nothing_selected: bool) -> f32 {
    if nothing_selected { DOCUMENT_SETUP_WIDTH + ui.spacing().item_spacing.x + PREFERENCES_WIDTH } else { crate::panels::transform::link_width(ui) }
}

/// Draws the Control bar's groups between its Character controls and the Transform link, and
/// returns whether the Transform link and X/Y/W/H follow (false: nothing is selected, and Document
/// Setup and Preferences, drawn here, end the bar). The groups are the Paragraph section and area
/// type's vertical alignment (#3759), then the Align buttons and, for two or more objects, the
/// Distribute buttons (a single object keeps the bar as it was); each shows only while the end of
/// the bar keeps its room.
fn groups_before_transform(app: &mut VectorcraftApp, ui: &mut Ui, objects: usize) -> bool {
    let nothing = objects == 0;
    if crate::panels::paragraph::control_bar(app, ui, BUTTON, |ui| group_end_width(ui) + end_width(ui, nothing)) {
        group_end(ui);
    }
    if crate::panels::properties::vertical_align::shown(app).is_some()
        && ui.available_width() >= buttons_width(ui, 4) + group_end_width(ui) + end_width(ui, nothing)
        && crate::panels::properties::vertical_align::buttons(app, ui, BUTTON)
    {
        group_end(ui);
    }
    if nothing {
        if widgets::flat_button(ui, tl!("Document Setup"), DOCUMENT_SETUP_WIDTH).clicked() {
            app.run("file.documentSetup", json!({})).ok();
        }
        if widgets::flat_button(ui, tl!("Preferences"), PREFERENCES_WIDTH).clicked() {
            app.run("edit.preferences", json!({})).ok();
        }
        return false;
    }
    let end = separator_width(ui) + end_width(ui, false);
    if ui.available_width() >= buttons_width(ui, 7) + end {
        crate::panels::align::align_buttons(app, ui, BUTTON);
        if objects > 1 && ui.available_width() >= GROUP_GAP + buttons_width(ui, 6) + end {
            ui.add_space(GROUP_GAP);
            crate::panels::align::distribute_buttons(app, ui, BUTTON);
        }
        separator(ui);
    }
    true
}

/// The edges of the window the Control bar docks to (`window.control.dock {edge}`): the top, then
/// the bottom.
pub const CONTROL_BAR_EDGES: [&str; 2] = ["top", "bottom"];

/// The side of the Control bar's panel menu button, at the bar's right end.
const MENU_BUTTON: f32 = 16.0;

/// The height of the band drawn along the edge of the window that a gripper drag docks the bar to.
const DOCK_BAND: f32 = 4.0;

/// Docks the Control bar to `edge` of [`CONTROL_BAR_EDGES`], through its command.
fn dock(app: &mut VectorcraftApp, edge: &str) {
    crate::menus::invoke(app, "window.control.dock", json!({ "edge": edge }));
}

/// The Control bar's panel menu: Dock to Top and Dock to Bottom, the edge the bar is docked to
/// checked.
fn panel_menu(app: &mut VectorcraftApp, ui: &mut Ui, rect: egui::Rect) {
    let resp = crate::panels::menu_button(ui, "control", rect);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(160.0);
        for (i, label) in [tl!("Dock to Top"), tl!("Dock to Bottom")].into_iter().enumerate() {
            if widgets::menu_item(ui, label, true, usize::from(app.ui.control_bar_bottom) == i) {
                dock(app, CONTROL_BAR_EDGES[i]);
            }
        }
    });
}

/// The Control bar's gripper. Dragging it shows a band along the top or the bottom edge of the
/// window, whichever the pointer is nearer, and docks the bar there on release.
fn gripper(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens) {
    let (g, resp) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::drag());
    for i in 0..5 {
        ui.painter().circle_filled(g.center_top() + vec2(0.0, 2.0 + i as f32 * 4.0), 0.9, t.text_disabled);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
    if !resp.dragged() && !resp.drag_stopped() {
        return;
    }
    let Some(pos) = resp.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos()) else { return };
    let window = ui.ctx().content_rect();
    let bottom = pos.y > window.center().y;
    if resp.drag_stopped() {
        if bottom != app.ui.control_bar_bottom {
            dock(app, CONTROL_BAR_EDGES[usize::from(bottom)]);
        }
        return;
    }
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let band = if bottom {
        egui::Rect::from_min_max(egui::pos2(window.left(), window.bottom() - DOCK_BAND), window.max)
    } else {
        egui::Rect::from_min_max(window.min, egui::pos2(window.right(), window.top() + DOCK_BAND))
    };
    let layer = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("control_bar_dock_band"));
    ui.ctx().layer_painter(layer).rect_filled(band, 0.0, t.accent);
}

/// The Control bar (Window → Control), context-sensitive like Illustrator's, docked to the top or
/// the bottom of the window.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let bottom = app.ui.control_bar_bottom;
    let panel = if bottom { egui::Panel::bottom("control_bar") } else { egui::Panel::top("control_bar") };
    let bar = panel
        .exact_size(34.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            // The panel menu button takes the right end, and the controls trim to the room left of it:
            // they are laid out in that room, so every room check below leaves the button's width.
            let full = ui.max_rect();
            let menu = egui::Rect::from_center_size(egui::pos2(full.right() - MENU_BUTTON / 2.0, full.center().y), vec2(MENU_BUTTON, MENU_BUTTON));
            let room = egui::Rect::from_min_max(full.min, egui::pos2(menu.left() - ui.spacing().item_spacing.x, full.bottom()));
            let layout = egui::Layout::left_to_right(egui::Align::Center);
            let controls = ui.scope_builder(egui::UiBuilder::new().max_rect(room).layout(layout), |ui| {
                gripper(app, ui, &t);
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
                let first = sel.first().and_then(|id| st.doc.node(*id));
                let anchors = anchor_controls(app);
                let label = match first {
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
                let has_first = first.is_some();
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
                let opacity = if has_first { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
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
                // A click opens the Transparency panel; a drag scrubs the field (#4088).
                if crate::scrub::link_label(ui, egui::RichText::new(tl!("Opacity:")).size(12.0).color(t.text).underline()).clicked() {
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
                    global_edit_options(app, ui);
                }
                ui.separator();
                crate::panels::character::control_bar(app, ui);
                if !groups_before_transform(app, ui, sel.len()) {
                    return;
                }
                // The Transform link opens the whole Transform panel (reference point, rotate,
                // shear, options) in a popover; X/Y/W/H follow inline while the bar has room.
                crate::panels::transform::link(app, ui);
                // The bounding box, rotated with rotated objects: its centre and its own sides.
                if let Some(b) = app.selection_box()
                    && ui.available_width() >= inline_transform_width(ui, &t)
                {
                    let c = b.center();
                    let origin = app.ruler_origin();
                    let link = crate::panels::transform::proportional(app, ui);
                    let copy = crate::panels::transform::alt_enter_copies(app, ui);
                    let values = [(c.x, origin.x), (c.y, origin.y), (b.rect.width(), 0.0), (b.rect.height(), 0.0)];
                    // The W/H link sits between W and H, but is made after H so Tab goes from W
                    // straight to H and finds the link after it.
                    let mut link_room = None;
                    for ((k, lbl), (v, offset)) in INLINE_FIELDS.into_iter().zip(values) {
                        if k == "height" {
                            link_room = Some(crate::panels::transform::constrain_link_room(ui));
                        }
                        widgets::field_label(ui, inline_label(&t, lbl));
                        let (id, shown) = (("cb", k), Some(v - offset));
                        let set = if matches!(k, "width" | "height") {
                            widgets::size_field(ui, id, shown, units, INLINE_FIELD_WIDTH)
                        } else {
                            widgets::num_field(ui, id, shown, units, INLINE_FIELD_WIDTH)
                        };
                        if let Some(nv) = set {
                            app.run("object.setBounds", json!({k: nv + offset, "reference": 4, "proportional": link, "copy": copy})).ok();
                        }
                    }
                    if let Some(room) = link_room {
                        let id = ui.scope_id().with("cb-constrain-link");
                        crate::panels::transform::constrain_link_at(app, ui, ui.interact(room, id, Sense::click()));
                    }
                }
            });
            // Controls that keep their size can still run into the button's room in a narrow window;
            // the button is left out then.
            if controls.response.rect.right() <= menu.left() {
                panel_menu(app, ui, menu);
            }
        });
    // Docked at the bottom, the bar is a bottom bar that menus stop above.
    if bottom {
        widgets::set_menu_floor(ui.ctx(), bar.response.rect.top());
    }
}
