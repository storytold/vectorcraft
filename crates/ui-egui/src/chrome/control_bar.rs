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

/// The Control bar's snapping quick access; every change uses its existing command, and Pixel
/// Snapping Options… opens the dialog of `view.snapToPixel.options`.
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
        if ui.button(tl!("Pixel Snapping Options…")).clicked() {
            ui.close();
            crate::menus::invoke(app, "view.snapToPixel.options", json!({}));
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

/// The size of the Control bar's Align, Distribute and area type vertical alignment buttons, and
/// the height of its Align To menu.
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
/// and Preferences with nothing selected, or none while the panel menu has that group off. The
/// groups ahead of them (the Paragraph section, area type's vertical alignment, the Align buttons,
/// the Align To menu and the Distribute buttons) show only while it is left, so the bar trims them
/// from its right end (#5372), after the inline X/Y/W/H fields.
fn end_width(app: &VectorcraftApp, ui: &Ui, nothing_selected: bool) -> f32 {
    if nothing_selected {
        if group_on(app, "documentOptions") { DOCUMENT_SETUP_WIDTH + ui.spacing().item_spacing.x + PREFERENCES_WIDTH } else { 0.0 }
    } else if group_on(app, "transform") {
        crate::panels::transform::link_width(ui)
    } else {
        0.0
    }
}

/// Draws the Control bar's groups between its Character controls and the Transform link, and
/// returns whether the Transform link and X/Y/W/H follow (false: nothing is selected, and Document
/// Setup and Preferences, drawn here, end the bar, or the Transform group is off). The groups are
/// the Paragraph section and area type's vertical alignment (#3759), then the Align buttons, the
/// Align To menu and, for two or more objects, the Distribute buttons (a single object keeps the bar
/// as it was); each shows only while the end of the bar keeps its room. Groups the panel menu has
/// off are left out, and take no room.
fn groups_before_transform(app: &mut VectorcraftApp, ui: &mut Ui, objects: usize) -> bool {
    let nothing = objects == 0;
    let end = end_width(app, ui, nothing);
    if group_on(app, "paragraph") && crate::panels::paragraph::control_bar(app, ui, BUTTON, |ui| group_end_width(ui) + end) {
        group_end(ui);
    }
    if group_on(app, "verticalAlignment")
        && crate::panels::properties::vertical_align::shown(app).is_some()
        && ui.available_width() >= buttons_width(ui, 4) + group_end_width(ui) + end
        && crate::panels::properties::vertical_align::buttons(app, ui, BUTTON)
    {
        group_end(ui);
    }
    if nothing {
        if group_on(app, "documentOptions") {
            if widgets::flat_button(ui, tl!("Document Setup"), DOCUMENT_SETUP_WIDTH).clicked() {
                app.run("file.documentSetup", json!({})).ok();
            }
            if widgets::flat_button(ui, tl!("Preferences"), PREFERENCES_WIDTH).clicked() {
                app.run("edit.preferences", json!({})).ok();
            }
        }
        return false;
    }
    let transform = group_on(app, "transform");
    let end = if transform { separator_width(ui) + end } else { 0.0 };
    if group_on(app, "alignDistribute") && ui.available_width() >= buttons_width(ui, 7) + end {
        crate::panels::align::align_buttons(app, ui, BUTTON);
        if ui.available_width() >= crate::panels::align::align_to_menu_width(BUTTON) + ui.spacing().item_spacing.x + end {
            crate::panels::align::align_to_menu(app, ui, BUTTON);
            if objects > 1 && ui.available_width() >= GROUP_GAP + buttons_width(ui, 6) + end {
                ui.add_space(GROUP_GAP);
                crate::panels::align::distribute_buttons(app, ui, BUTTON);
            }
        }
        if transform {
            separator(ui);
        }
    }
    transform
}

/// Draws the Control bar's Align group after the artboard controls, with the Artboard tool in use:
/// the Align buttons, the Align To menu and, while they can space the selected artboards
/// ([`crate::panels::align::artboards_distribute`]), the Distribute buttons. They align and
/// distribute the selected artboards
/// ([`crate::panels::align::run_button`]). Each shows only while the bar has room for it, and none
/// while the panel menu has the group off.
fn artboard_align_group(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !group_on(app, "alignDistribute") || app.session.active().is_none() || ui.available_width() < separator_width(ui) + buttons_width(ui, 7) {
        return;
    }
    separator(ui);
    crate::panels::align::align_buttons(app, ui, BUTTON);
    if ui.available_width() >= crate::panels::align::align_to_menu_width(BUTTON) + ui.spacing().item_spacing.x {
        crate::panels::align::align_to_menu(app, ui, BUTTON);
        if crate::panels::align::artboards_distribute(app) && ui.available_width() >= GROUP_GAP + buttons_width(ui, 6) {
            ui.add_space(GROUP_GAP);
            crate::panels::align::distribute_buttons(app, ui, BUTTON);
        }
    }
}

/// The places the Control bar goes to (`window.control.dock {edge}`): docked to the top or the
/// bottom of the window, or floating over it.
pub const CONTROL_BAR_EDGES: [&str; 3] = ["top", "bottom", "float"];

/// Where `window.control.dock {edge: "float"}` floats the Control bar the first time: its top-left
/// corner in window points.
pub const CONTROL_BAR_FLOAT_AT: [f32; 2] = [60.0, 80.0];

/// The entry of [`CONTROL_BAR_EDGES`] the Control bar is at in `ui`.
pub fn control_bar_placement(ui: &crate::state::UiState) -> &'static str {
    CONTROL_BAR_EDGES[if ui.control_bar_floating { 2 } else { usize::from(ui.control_bar_bottom) }]
}

/// The groups of the Control bar's options that its panel menu turns on or off
/// (`window.control.options {group, visible?}`), by id, in the menu's order. Every group is on
/// until it is turned off. A group that is off stays out of the bar even while its tool or object
/// is selected; one that is on still shows only while the selection or the tool calls for it.
pub const CONTROL_BAR_GROUPS: [&str; 15] = [
    "snapping",
    "anchors",
    "toolOptions",
    "objectOptions",
    "imageOptions",
    "fillStroke",
    "opacity",
    "graphicStyle",
    "character",
    "paragraph",
    "verticalAlignment",
    "alignDistribute",
    "transform",
    "artboardOptions",
    "documentOptions",
];

/// The panel menu's label for group `group` of [`CONTROL_BAR_GROUPS`], translated.
fn group_label(group: &str) -> &'static str {
    match group {
        "snapping" => tl!("Snapping"),
        "anchors" => tl!("Anchor Points"),
        "toolOptions" => tl!("Tool Options"),
        "objectOptions" => tl!("Object Options"),
        "imageOptions" => tl!("Image Options"),
        "fillStroke" => tl!("Fill and Stroke"),
        "opacity" => tl!("Opacity"),
        "graphicStyle" => tl!("Graphic Style"),
        "character" => tl!("Character"),
        "paragraph" => tl!("Paragraph"),
        "verticalAlignment" => tl!("Vertical Alignment"),
        "alignDistribute" => tl!("Align and Distribute"),
        "transform" => tl!("Transform"),
        "artboardOptions" => tl!("Artboard Options"),
        _ => tl!("Document Options"),
    }
}

/// Whether group `group` of [`CONTROL_BAR_GROUPS`] is on, so its controls may show in the bar.
pub(crate) fn group_on(app: &VectorcraftApp, group: &str) -> bool {
    !app.ui.control_bar_hidden_groups.contains(group)
}

/// Reads the groups of [`CONTROL_BAR_GROUPS`] that are off, as kept in the preferences and in a
/// workspace. Ids the list does not hold are dropped, and a value that is not a list of strings
/// reads as no group off, so every group shows.
pub fn read_hidden_groups<'de, D: serde::Deserializer<'de>>(d: D) -> Result<std::collections::BTreeSet<String>, D::Error> {
    let value = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
    let ids = value.as_array().into_iter().flatten().filter_map(serde_json::Value::as_str);
    Ok(ids.filter(|id| CONTROL_BAR_GROUPS.contains(id)).map(str::to_string).collect())
}

/// The side of the Control bar's panel menu button, at the bar's right end.
const MENU_BUTTON: f32 = 16.0;

/// The height of the band drawn along the edge of the window that a gripper drag docks the bar to.
const DOCK_BAND: f32 = 4.0;

/// The Control bar's height, docked or floating.
const HEIGHT: f32 = 34.0;

/// The Control bar's inner margin left and right, around its gripper and its panel menu button.
const MARGIN: f32 = 10.0;

/// The width the floating Control bar takes in a window wide enough for it.
const FLOAT_WIDTH: f32 = 800.0;

/// How near the edge the bar docks to a gripper drag is released for the bar to dock there; a
/// release farther from both edges floats the bar.
const DOCK_ZONE: f32 = 24.0;

/// The key of a gripper drag in progress: where the pointer holds the bar from its top-left
/// corner, and the gripper's widget id.
fn move_id() -> egui::Id {
    egui::Id::unique("control_bar_gripper_move")
}

/// Is the Control bar's gripper being dragged, the frame that releases it included? An Escape then
/// cancels the drag, and nothing else. A drag left over from a bar hidden while the button was held
/// ends with the button.
pub fn control_bar_moving(ctx: &egui::Context) -> bool {
    let Some((_, gripper)) = ctx.data(|d| d.get_temp::<(egui::Vec2, egui::Id)>(move_id())) else { return false };
    ctx.input(|i| i.pointer.primary_down()) || ctx.drag_stopped_id() == Some(gripper)
}

/// Where a gripper release puts the Control bar.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Release {
    /// Docked to the bottom (true) or the top (false).
    Edge(bool),
    /// Floating, its top-left corner here.
    Float(egui::Pos2),
}

/// The width of the floating Control bar in a window whose room below the menu bar is `room`.
fn float_width(room: egui::Rect) -> f32 {
    FLOAT_WIDTH.min(room.width()).max(2.0 * MARGIN + MENU_BUTTON + 6.0)
}

/// The floating Control bar's rect for its saved corner `pos`, kept inside `room`, the window below
/// the menu bar, so its gripper and panel menu stay in reach.
fn float_rect(pos: Option<[f32; 2]>, room: egui::Rect) -> egui::Rect {
    let size = vec2(float_width(room), HEIGHT);
    egui::Rect::from_min_size(crate::tools_float::clamp(pos.unwrap_or(CONTROL_BAR_FLOAT_AT), size, room), size)
}

/// Where a gripper drag released at `at` puts the bar, held at `grab` from its top-left corner:
/// within [`DOCK_ZONE`] of the top or the bottom of `room` (the window below the menu bar) docked
/// there, farther from both floating under the pointer. In a window too short for the two zones
/// and a floating bar between them, it docks at the nearer edge. A release outside `window` puts
/// it nowhere, and the bar stays where it was.
fn release_at(at: egui::Pos2, grab: egui::Vec2, room: egui::Rect, window: egui::Rect) -> Option<Release> {
    if !window.contains(at) || !at.x.is_finite() || !at.y.is_finite() {
        return None;
    }
    if room.height() < 2.0 * DOCK_ZONE + HEIGHT {
        return Some(Release::Edge(at.y > room.center().y));
    }
    Some(if at.y < room.top() + DOCK_ZONE {
        Release::Edge(false)
    } else if at.y > room.bottom() - DOCK_ZONE {
        Release::Edge(true)
    } else {
        let corner = at - grab;
        Release::Float(float_rect(Some([corner.x, corner.y]), room).min)
    })
}

/// Docks the Control bar to `edge` of [`CONTROL_BAR_EDGES`], through its command.
fn dock(app: &mut VectorcraftApp, edge: &str) {
    crate::menus::invoke(app, "window.control.dock", json!({ "edge": edge }));
}

/// The Control bar's panel menu: an item for each group of [`CONTROL_BAR_GROUPS`], checked while
/// the group is on, which turns it on or off; then Dock to Top and Dock to Bottom, the edge the bar
/// is docked to checked, and neither while it floats. In a window too short for every item, the
/// list scrolls.
fn panel_menu(app: &mut VectorcraftApp, ui: &mut Ui, rect: egui::Rect) {
    let resp = crate::panels::menu_button(ui, "control", rect);
    egui::Popup::menu(&resp).show(|ui| {
        ui.set_min_width(160.0);
        widgets::menu_scroll(ui, |ui| {
            for group in CONTROL_BAR_GROUPS {
                if widgets::menu_item(ui, group_label(group), true, group_on(app, group)) {
                    crate::menus::invoke(app, "window.control.options", json!({ "group": group }));
                }
            }
            ui.separator();
            for (i, label) in [tl!("Dock to Top"), tl!("Dock to Bottom")].into_iter().enumerate() {
                if widgets::menu_item(ui, label, true, control_bar_placement(&app.ui) == CONTROL_BAR_EDGES[i]) {
                    dock(app, CONTROL_BAR_EDGES[i]);
                }
            }
        });
    });
}

/// The Control bar's Fill and Stroke group: the fill and stroke chips, the Stroke link, the weight
/// and the width profile, and the separator after them.
fn fill_and_stroke(app: &mut VectorcraftApp, ui: &mut Ui) {
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

/// The Control bar's Opacity group (its link and field, showing `opacity`) and, with something
/// `selected`, its Graphic Style group (Style, Recolor Artwork and Global Edit Options), each while
/// the panel menu has it on, and the separator after them.
fn opacity_and_style(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens, opacity: f32, selected: bool) {
    let (opacity_on, style_on) = (group_on(app, "opacity"), group_on(app, "graphicStyle") && selected);
    if opacity_on {
        // A click opens the Transparency panel; a drag scrubs the field (#4088).
        if crate::scrub::link_label(ui, egui::RichText::new(tl!("Opacity:")).size(12.0).color(t.text).underline()).clicked() {
            app.ui.open_panel = Some("transparency".into());
        }
        if let Some(o) = widgets::plain_field(ui, "cb-opacity", opacity as f64 * 100.0, "%", 0, 56.0)
            && selected
        {
            app.run("transparency.set", json!({"opacity": o.clamp(0.0, 100.0)})).ok();
        }
    }
    if style_on {
        // Style picker: the selection's graphic style; its menu applies another.
        ui.add_space(4.0);
        if ui.link(egui::RichText::new(tl!("Style:")).size(12.0).color(t.text).underline()).clicked() {
            app.ui.open_panel = Some("graphicStyles".into());
        }
        let resp = widgets::chip_button_named(ui, 22.0, true, tl!("Graphic Style"), |ui, r| crate::panels::graphic_styles::paint_linked(app, ui, r));
        egui::Popup::menu(&resp).show(|ui| crate::panels::graphic_styles::picker(app, ui));
        // Recolor Artwork, when the selection has colours to recolour.
        let colored = app.session.active().is_some_and(|st| crate::canvas::has_colors(&st.doc, &st.selection.objects));
        if colored && widgets::icon_button(ui, "palette", tl!("Recolor Artwork"), false, 24.0).clicked() {
            crate::menus::invoke(app, "ui.recolorDialog", json!({}));
        }
        global_edit_options(app, ui);
    }
    if opacity_on || style_on {
        ui.separator();
    }
}

/// The Control bar's gripper, at the left end of the bar `bar`. Dragging it shows where a release
/// puts the bar ([`release_at`] in `dock_room`, the window below the menu bar): a band along the
/// edge it docks to, or the outline of the bar floating under the pointer. The bar stays where it is
/// until the release. Escape, or the window losing the focus, cancels the drag, and a release
/// outside the window leaves the bar where it was.
fn gripper(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens, bar: egui::Rect, dock_room: egui::Rect) {
    let (g, resp) = ui.allocate_exact_size(vec2(6.0, 20.0), Sense::drag());
    for i in 0..5 {
        ui.painter().circle_filled(g.center_top() + vec2(0.0, 2.0 + i as f32 * 4.0), 0.9, t.text_disabled);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::Grab);
    let ctx = ui.ctx().clone();
    if resp.drag_started()
        && let Some(press) = ctx.input(|i| i.pointer.press_origin())
    {
        ctx.data_mut(|d| d.insert_temp(move_id(), (press - bar.min, resp.id)));
    }
    let Some((grab, _)) = ctx.data(|d| d.get_temp::<(egui::Vec2, egui::Id)>(move_id())) else { return };
    let canceled = ctx.input(|i| i.key_pressed(egui::Key::Escape) || !i.focused);
    if canceled || !(resp.dragged() || resp.drag_stopped()) {
        ctx.data_mut(|d| d.remove::<(egui::Vec2, egui::Id)>(move_id()));
        return;
    }
    let window = ctx.content_rect();
    let target = ctx.input(|i| i.pointer.latest_pos()).and_then(|at| release_at(at, grab, dock_room, window));
    if resp.drag_stopped() {
        ctx.data_mut(|d| d.remove::<(egui::Vec2, egui::Id)>(move_id()));
        match target {
            Some(Release::Edge(bottom)) if app.ui.control_bar_floating || bottom != app.ui.control_bar_bottom => {
                dock(app, CONTROL_BAR_EDGES[usize::from(bottom)]);
            }
            Some(Release::Float(at)) => {
                app.ui.control_bar_float_pos = Some([at.x, at.y]);
                if !app.ui.control_bar_floating {
                    dock(app, CONTROL_BAR_EDGES[2]);
                }
            }
            _ => {}
        }
        return;
    }
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("control_bar_dock_band")));
    match target {
        Some(Release::Edge(bottom)) => {
            let band = if bottom {
                egui::Rect::from_min_max(egui::pos2(dock_room.left(), dock_room.bottom() - DOCK_BAND), dock_room.max)
            } else {
                egui::Rect::from_min_max(dock_room.min, egui::pos2(dock_room.right(), dock_room.top() + DOCK_BAND))
            };
            painter.rect_filled(band, 0.0, t.accent);
        }
        Some(Release::Float(at)) => {
            let outline = egui::Rect::from_min_size(at, vec2(float_width(dock_room), HEIGHT));
            painter.rect_stroke(outline, 0.0, Stroke::new(2.0, t.accent), egui::StrokeKind::Inside);
        }
        None => {}
    }
}

/// The Control bar (Window → Control), context-sensitive like Illustrator's, docked to the top or
/// the bottom of the window, or floating over the window below the menu bar, where it takes no room
/// from the canvas. A floating bar is [`FLOAT_WIDTH`] wide, or as wide as the window when that is
/// narrower, and is kept inside the window when the window shrinks.
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    // The window below the menu bar: where the bar docks at the top, and where it floats.
    let dock_room = ui.available_rect_before_wrap();
    let frame = egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border));
    if app.ui.control_bar_floating {
        let bar = float_rect(app.ui.control_bar_float_pos, dock_room);
        let ctx = ui.ctx().clone();
        let area = egui::Area::new(egui::Id::unique("control_bar_floating")).order(egui::Order::Middle).fixed_pos(bar.min).constrain(false);
        area.show(&ctx, |ui| {
            frame.show(ui, |ui| {
                // Inside the frame's margins and its 1-point border.
                ui.set_width(bar.width() - 2.0 * MARGIN - 2.0);
                ui.set_height(HEIGHT - 2.0);
                contents(app, ui, &t, bar, dock_room, true);
            });
        });
        return;
    }
    let bottom = app.ui.control_bar_bottom;
    let panel = if bottom { egui::Panel::bottom("control_bar") } else { egui::Panel::top("control_bar") };
    let bar = panel.exact_size(HEIGHT).frame(frame).show(ui, |ui| {
        // Out to the frame's 1-point border.
        let outer = ui.max_rect().expand2(vec2(MARGIN + 1.0, 1.0));
        contents(app, ui, &t, outer, dock_room, false);
    });
    // Docked at the bottom, the bar is a bottom bar that menus stop above.
    if bottom {
        widgets::set_menu_floor(ui.ctx(), bar.response.rect.top());
    }
}

/// The Control bar's gripper, controls and panel menu, in the bar `bar`. A `floating` bar clips its
/// controls to the room left of the panel menu button, and always shows the button.
fn contents(app: &mut VectorcraftApp, ui: &mut Ui, t: &Tokens, bar: egui::Rect, dock_room: egui::Rect, floating: bool) {
    // The panel menu button takes the right end, and the controls trim to the room left of it:
    // they are laid out in that room, so every room check below leaves the button's width.
    let full = ui.max_rect();
    let menu = egui::Rect::from_center_size(egui::pos2(full.right() - MENU_BUTTON / 2.0, full.center().y), vec2(MENU_BUTTON, MENU_BUTTON));
    let room = egui::Rect::from_min_max(full.min, egui::pos2(menu.left() - ui.spacing().item_spacing.x, full.bottom()));
    let layout = egui::Layout::left_to_right(egui::Align::Center);
    let controls = ui.scope_builder(egui::UiBuilder::new().max_rect(room).layout(layout), |ui| {
        if floating {
            ui.set_clip_rect(room.intersect(ui.clip_rect()));
        }
        gripper(app, ui, t, bar, dock_room);
        if app.session.active().is_none() {
            ui.label(egui::RichText::new(tl!("No Document")).color(t.text_dim));
            return;
        }
        // The Artboard tool's controls fill the narrowest window, so they keep it to themselves there.
        if group_on(app, "snapping") && (app.session.tool_id() != "artboard" || ui.available_width() >= ARTBOARD_SNAPPING_WIDTH) {
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
        let (artboard_options, transform) = (group_on(app, "artboardOptions"), group_on(app, "transform"));
        if crate::panels::artboards::control_bar(app, ui, artboard_options, transform) {
            artboard_align_group(app, ui);
            return;
        }
        if anchors != AnchorControls::None && group_on(app, "anchors") {
            anchor_buttons(app, ui, anchors);
        }
        let object_options = group_on(app, "objectOptions");
        if object_options {
            isolation_button(app, ui);
        }
        ui.add_space(6.0);
        // An image or an Image Trace object shows its own controls in place of Fill and Stroke,
        // and leaves Fill and Stroke out while the Image Options group is off too.
        let image = if group_on(app, "imageOptions") {
            crate::place::control_bar_details(app, ui) || crate::panels::image_trace::control_bar(app, ui)
        } else {
            crate::place::selected_image_info(app).is_some() || crate::panels::image_trace::selected_trace(app).is_some()
        };
        if group_on(app, "toolOptions") {
            crate::toolbar::control_bar_options(app, ui);
        }
        if object_options {
            crate::dialogs::envelope::control_bar(app, ui);
            crate::panels::symbols::control_bar(app, ui);
        }
        let opacity = if has_first { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
        if !image && group_on(app, "fillStroke") {
            fill_and_stroke(app, ui);
        }
        opacity_and_style(app, ui, t, opacity, !sel.is_empty());
        if group_on(app, "character") {
            crate::panels::character::control_bar(app, ui);
        }
        if !groups_before_transform(app, ui, sel.len()) {
            return;
        }
        // The Transform link opens the whole Transform panel (reference point, rotate,
        // shear, options) in a popover; X/Y/W/H follow inline while the bar has room.
        crate::panels::transform::link(app, ui);
        // The bounding box, rotated with rotated objects: its centre and its own sides.
        if let Some(b) = app.selection_box()
            && ui.available_width() >= inline_transform_width(ui, t)
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
                widgets::field_label(ui, inline_label(t, lbl));
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
    // the button is left out then. A floating bar clips them, so its button always shows.
    if floating || controls.response.rect.right() <= menu.left() {
        panel_menu(app, ui, menu);
    }
}
