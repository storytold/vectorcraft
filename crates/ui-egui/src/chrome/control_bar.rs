use craft::behavior::IconId;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, PopupKind, Props as _, Show as _, ShowContainer as _, SideLabel, Trigger};
use craft::ui::display::{Divider, Elide, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::shell::{BarGrip, OptionsBarPlace};
use craft::ui::style::SurfaceKind;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;

use crate::VectorcraftApp;
use crate::panels::stroke as stroke_panel;

/// The ranks of the Control bar's members. A narrow bar drops the lowest rank first and, within a rank, the
/// member that comes last; a row with no rank never leaves by rank.
pub(crate) mod rank {
    /// The Select Similar button and its menu: the first to leave.
    pub const SELECT_SIMILAR: u8 = 1;
    /// The inline X/Y/W/H fields and the Snapping menu.
    pub const FIELDS: u8 = 2;
    /// The parts of a group that its other members work without: the width profile, Recolor Artwork, Global Edit
    /// Options, Document Setup, an image's Edit Original.
    pub const DETAIL: u8 = 3;
    /// The Opacity link and field: they leave before the groups, so the narrower Style link and picker after them
    /// can show without them.
    pub const OPACITY: u8 = 4;
    /// Every other member of the bar.
    pub const GROUP: u8 = 5;
}

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

/// A row of the bar that leaves it whole when the bar is too narrow, by `rank`.
fn ranked(ui: &mut Ui, rank: u8, mut members: impl FnMut(&mut Ui)) {
    Row::new().fit_rank(rank).show(ui, |ui, _| members(ui));
}

/// A [`ranked`] row that starts with its own rule, so the rule leaves with the group.
fn group(ui: &mut Ui, rank: u8, mut members: impl FnMut(&mut Ui)) {
    ranked(ui, rank, |ui| {
        Divider::vertical().show(ui);
        members(ui);
    });
}

/// What the Control bar's and the Properties panel's anchor buttons offer for selected anchors
/// ([`anchor_controls`]): "Convert:" corner or smooth, then "Anchors:" remove, connect (Join) and cut.
/// Each group is a row of its own: side by side in the Control bar's row, one under the other in a panel.
pub fn anchor_buttons(app: &mut VectorcraftApp, ui: &mut Ui, controls: AnchorControls) {
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
        Row::new().show(ui, |ui, _| {
            Text::new(label).show(ui);
            for &(icon, tip, id, to) in buttons {
                if IconButton::new(IconId::new(icon)).label(tip).show(ui).clicked() {
                    run = Some((id, to.map_or_else(|| json!({}), |to| json!({ "to": to }))));
                }
            }
        });
    }
    if let Some((id, p)) = run {
        crate::menus::invoke(app, id, p);
    }
}

/// The Corners link and the radius of the selected paths' picked corners
/// ([`crate::panels::picked_corners`]), beside the anchor controls. The link opens the Corners
/// pop-up for those corners; a narrow bar leaves out the link and the field together.
fn corner_radius(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(picked) = crate::panels::picked_corners(app) else { return };
    let label = tl!("Corner Radius:");
    let lengths = crate::canvas::general_lengths(app);
    let mut typed = None;
    group(ui, rank::GROUP, |ui| {
        crate::panels::corners_link(app, ui);
        let number = NumberField::length(label, Bind::given(picked.radius), lengths).show(ui);
        typed = number.edit.events.committed().copied().flatten().or(typed);
    });
    if let Some(radius) = typed {
        crate::panels::set_picked_corners(app, &picked, json!({"radius": radius.max(0.0)}));
    }
}

/// One row of a menu that runs a command: its label, its check (`None`: no mark), the command and its params.
struct CommandRow {
    label: &'static str,
    check: Option<bool>,
    id: &'static str,
    params: Value,
}

/// Shows `rows` in the menu being built; returns the command of the one that ran.
fn command_rows(ui: &mut Ui, rows: &[CommandRow]) -> Option<(&'static str, Value)> {
    let mut ran = None;
    for row in rows {
        let item = MenuItem::new(row.label);
        let item = match row.check {
            Some(on) => item.check(on),
            None => item,
        };
        if item.show(ui).ran {
            ran = Some((row.id, row.params.clone()));
        }
    }
    ran
}

/// The Control bar's snapping quick access; every change uses its existing command, and Pixel
/// Snapping Options… opens the dialog of `view.snapToPixel.options`.
fn snapping_menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let toggle = |label, id, on| CommandRow { label, check: Some(on), id, params: json!({}) };
    let preference =
        |label, key: &'static str, on: bool| CommandRow { label, check: Some(on), id: "prefs.set", params: json!({"key": key, "value": !on}) };
    let view = &app.ui.view;
    let prefs = &app.session.prefs;
    let sections = [
        vec![
            toggle(tl!("Smart Guides"), "view.smartGuides", view.smart_guides),
            toggle(tl!("Snap to Point"), "view.snapToPoint", view.snap_to_point),
            toggle(tl!("Snap to Grid"), "view.snapToGrid", view.snap_to_grid),
            toggle(tl!("Snap to Pixel"), "view.snapToPixel", view.snap_to_pixel),
            toggle(tl!("Snap to Glyph"), "view.snapToGlyph", view.snap_to_glyph),
            CommandRow { label: tl!("Pixel Snapping Options…"), check: None, id: "view.snapToPixel.options", params: json!({}) },
        ],
        vec![
            preference(tl!("Alignment Guides"), "alignmentGuides", prefs.alignment_guides),
            preference(tl!("Snap to Tangent"), "snapTangent", prefs.snap_tangent),
            preference(tl!("Snap to Perpendicular"), "snapPerpendicular", prefs.snap_perpendicular),
        ],
    ];
    let label = tl!("Snapping");
    let mut ran = None;
    ranked(ui, rank::FIELDS, |ui| {
        let button = Button::new(label).trailing_icon(IconId::new("chevron-down")).show(ui);
        Menu::new(label).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
            for (i, rows) in sections.iter().enumerate() {
                if i > 0 {
                    Divider::horizontal().show(ui);
                }
                ran = command_rows(ui, rows).or(ran.take());
            }
        });
    });
    if let Some((id, params)) = ran {
        crate::menus::invoke(app, id, params);
    }
}

/// The Control bar's isolation action: icon, name and command. In isolation mode it is Exit Isolation Mode,
/// except while a pattern is edited (the isolation bar's Done and Cancel end that), and Exit Symbol Editing Mode
/// while a symbol's art is the isolated layer. Otherwise, with one object selected that
/// [`vectorcraft_tools::select::isolates`], it is Isolate Selected Object, which also isolates a
/// path inside a group that Direct Selection selected. An envelope's contents get no button: the
/// envelope's Edit Envelope and Edit Contents buttons show instead.
fn isolation_action(app: &VectorcraftApp) -> Option<(&'static str, &'static str, &'static str)> {
    let st = app.session.active()?;
    if st.isolation.is_some() {
        if st.doc.pattern_edit.is_some() {
            return None;
        }
        return Some(if st.doc.symbol_edit.as_ref().is_some_and(|e| st.isolation == Some(e.layer)) {
            ("chevron-left", tl!("Exit Symbol Editing Mode"), "symbol.exitEditing")
        } else {
            ("chevron-left", tl!("Exit Isolation Mode"), "object.exitIsolation")
        });
    }
    let [one] = st.selection.objects[..] else { return None };
    let isolates =
        st.doc.node(one).is_some_and(|n| vectorcraft_tools::select::isolates(&n.kind)) && vectorcraft_doc::live::envelope_of(&st.doc, one).is_none();
    isolates.then_some(("square-dashed", tl!("Isolate Selected Object"), "object.isolate"))
}

/// The Control bar's isolation button ([`isolation_action`]).
fn isolation_button(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some((icon, name, id)) = isolation_action(app) else { return };
    let mut clicked = false;
    group(ui, rank::GROUP, |ui| clicked |= IconButton::new(IconId::new(icon)).label(name).show(ui).clicked());
    if clicked {
        crate::menus::invoke(app, id, json!({}));
    }
}

/// The Control bar's Global Edit Options dropdown, shown while `select.globalEdit` can run. The help
/// page places the options in the Control panel as well as in Properties
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>).
/// It opens the Properties panel's Global Edit Options, which set the same session options, and is
/// off and closed while Global Edit is on, as that panel's dropdown is.
fn global_edit_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !crate::menus::enabled(app, "select.globalEdit") {
        return;
    }
    let on = app.session.active().is_some_and(|d| d.global_edit.is_some());
    let label = tl!("Global Edit Options");
    ranked(ui, rank::DETAIL, |ui| {
        let options = IconButton::new(IconId::new("chevron-down")).label(label).enabled(!on).show(ui);
        // Properties' popover with the bar's own flag: closed while Global Edit is on, and it stays closed after.
        let id = options.response.id.with("open");
        let mut open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        crate::panels::properties::global_edit_popover(app, ui, &options.response, &mut open);
        ui.data_mut(|d| d.insert_temp(id, open));
    });
}

/// The Control bar's Fill and Stroke group: the fill and stroke chips, the Stroke link with the weight, and
/// the width profile. A narrow bar leaves out the width profile first, then the Stroke link with the weight,
/// then the two chips.
fn fill_and_stroke(app: &mut VectorcraftApp, ui: &mut Ui) {
    let shown_stroke = crate::panels::current_stroke(app);
    let mixed = crate::panels::stroke_mixed(app, ui.ctx());
    let weight = stroke_panel::shown_weight(app, shown_stroke.as_ref(), &mixed);
    group(ui, rank::GROUP, |ui| {
        crate::panels::paint_chip(app, ui, false);
        crate::panels::paint_chip(app, ui, true);
    });
    ranked(ui, rank::GROUP, |ui| stroke_panel::link_field(app, ui, tl!("Stroke:"), weight));
    ranked(ui, rank::DETAIL, |ui| stroke_panel::profile_select(app, ui));
}

/// The Control bar's Opacity group: its link, which opens the Transparency panel and scrubs the field, and the
/// field, showing `opacity`. Only a selection takes the edit.
fn opacity_field(app: &mut VectorcraftApp, ui: &mut Ui, opacity: f32, selected: bool) {
    let label = tl!("Opacity:");
    let (mut link, mut typed) = (false, None);
    group(ui, rank::OPACITY, |ui| {
        Field::new(label).side(SideLabel::new().link().scrub(true)).show(ui, |ui, _| {
            let number =
                NumberField::new(label, Bind::given(Some(f64::from(opacity) * 100.0))).range(0.0..=100.0).unit("%").scrub_by_label().show(ui);
            link |= number.link;
            typed = number.edit.events.committed().copied().flatten().or(typed);
        });
    });
    if link {
        app.ui.open_panel = Some("transparency".into());
    }
    if let Some(o) = typed.filter(|_| selected) {
        app.run("transparency.set", json!({"opacity": o.clamp(0.0, 100.0)})).ok();
    }
}

/// The Control bar's Graphic Style group for a selection: the Style link and picker, then Recolor Artwork and
/// Global Edit Options. A narrow bar leaves out Global Edit Options first, then Recolor Artwork, then the Style
/// link with its picker.
fn style_group(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mut open = false;
    ranked(ui, rank::GROUP, |ui| {
        open |= crate::panels::option_link(tl!("Style:")).show(ui).clicked();
        crate::panels::graphic_styles::style_picker(app, ui);
    });
    if open {
        app.ui.open_panel = Some("graphicStyles".into());
    }
    // Recolor Artwork, when the selection has colours to recolour.
    let colored = app.session.active().is_some_and(|st| crate::canvas::has_colors(&st.doc, &st.selection.objects));
    if colored {
        let mut recolor = false;
        ranked(ui, rank::DETAIL, |ui| recolor |= IconButton::new(IconId::new("palette")).label(tl!("Recolor Artwork")).show(ui).clicked());
        if recolor {
            crate::menus::invoke(app, "ui.recolorDialog", json!({}));
        }
    }
    global_edit_options(app, ui);
}

/// The label of each attribute of [`vectorcraft_engine::cmd::SIMILAR`] in the Select Similar Options
/// menu, translated.
fn similar_label(by: &str) -> &'static str {
    match by {
        "all" => tl!("All"),
        "fillColor" => tl!("Fill Color"),
        "fillAndStroke" => tl!("Fill & Stroke"),
        "strokeColor" => tl!("Stroke Color"),
        "strokeWeight" => tl!("Stroke Weight"),
        "opacity" => tl!("Opacity"),
        "blendingMode" => tl!("Blending Mode"),
        "appearance" => tl!("Appearance"),
        "graphicStyle" => tl!("Graphic Style"),
        "shapeType" => tl!("Shape"),
        _ => tl!("Symbol Instance"),
    }
}

/// The Control bar's Select Similar Objects button and, beside it, the Select Similar Options menu
/// (<https://help.adobe.com/archive/en_US/illustrator/cs5/illustrator_cs5_help.pdf#page=165>). The
/// button runs `select.similar`, which selects the objects like the first selected one by the
/// attribute kept for the session; a menu item runs it by that item's attribute, which is kept from
/// then on, and the kept one is checked. Both are off while `select.similar` cannot run. They show
/// together at the right end of the bar while something is selected and the panel menu has Object
/// Options on, and are the first members a narrow bar leaves out.
fn trailing_select_similar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let selected = app.session.active().is_some_and(|st| !st.selection.objects.is_empty());
    if !selected || !group_on(app, "objectOptions") {
        return;
    }
    let enabled = crate::menus::enabled(app, "select.similar");
    let kept: Vec<(&str, bool)> = vectorcraft_engine::cmd::SIMILAR.iter().map(|entry| (entry.0, app.session.select_similar == entry.0)).collect();
    let mut params = None;
    group(ui, rank::SELECT_SIMILAR, |ui| {
        if IconButton::new(IconId::new("copy")).label(tl!("Select Similar Objects")).enabled(enabled).show(ui).clicked() {
            params = Some(json!({}));
        }
        let label = tl!("Select Similar Options");
        let options = IconButton::new(IconId::new("chevron-down")).label(label).enabled(enabled).popup(PopupKind::Menu).show(ui);
        Menu::new(label).opened_by(&options, Trigger::Click).show(ui, |ui, _| {
            for &(by, on) in &kept {
                // A menu row reads `&` as its mnemonic marker: `&&` is the character.
                if MenuItem::new(similar_label(by).replace('&', "&&")).check(on).show(ui).ran {
                    params = Some(json!({ "by": by }));
                }
            }
        });
    });
    if let Some(params) = params {
        crate::menus::invoke(app, "select.similar", params);
    }
}

/// The places the Control bar goes to (`window.control.dock {edge}`): docked to the top or the
/// bottom of the window, or floating over it.
pub const CONTROL_BAR_EDGES: [&str; 3] = ["top", "bottom", "float"];

/// The entry of [`CONTROL_BAR_EDGES`] the Control bar is at in `ui`.
pub fn control_bar_placement(ui: &crate::state::UiState) -> &'static str {
    CONTROL_BAR_EDGES[if ui.control_bar_floating { 2 } else { usize::from(ui.control_bar_bottom) }]
}

/// Where the Control bar sits, as the shell's options slot takes it.
pub(crate) fn control_bar_place(ui: &crate::state::UiState) -> OptionsBarPlace {
    OptionsBarPlace::new(ui.control_bar_bottom, ui.control_bar_floating, ui.control_bar_float_pos)
}

/// Keeps the place the shell moved the Control bar to. Floating keeps the edge and docking keeps the floating
/// corner, as `window.control.dock` does; a corner that is not finite counts as unset.
pub(crate) fn set_control_bar_place(ui: &mut crate::state::UiState, place: OptionsBarPlace) {
    ui.control_bar_bottom = place.bottom;
    ui.control_bar_floating = place.floating;
    ui.control_bar_float_pos = place.at.filter(|at| at.iter().all(|v| v.is_finite()));
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

/// The rows of the panel menu: an item for each group of [`CONTROL_BAR_GROUPS`], checked while the group is
/// on, then Dock to Top and Dock to Bottom, the edge the bar is docked to checked, and neither while it floats.
fn panel_menu_rows(app: &VectorcraftApp) -> [Vec<CommandRow>; 2] {
    let groups = CONTROL_BAR_GROUPS
        .iter()
        .map(|&group| CommandRow {
            label: group_label(group),
            check: Some(group_on(app, group)),
            id: "window.control.options",
            params: json!({ "group": group }),
        })
        .collect();
    let placed = control_bar_placement(&app.ui);
    let docks = [tl!("Dock to Top"), tl!("Dock to Bottom")]
        .into_iter()
        .zip(CONTROL_BAR_EDGES)
        .map(|(label, edge)| CommandRow { label, check: Some(placed == edge), id: "window.control.dock", params: json!({ "edge": edge }) })
        .collect();
    [groups, docks]
}

/// The ≡ button at the bar's reading end and its menu; `picked` takes the command of the row that ran.
fn panel_menu(ui: &mut Ui, rows: &[Vec<CommandRow>; 2], picked: &mut Option<(&'static str, Value)>) {
    let button = IconButton::new(IconId::new("menu")).label(tl!("Panel menu")).popup(PopupKind::Menu).show(ui);
    Menu::new(tl!("Control")).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
        for (i, section) in rows.iter().enumerate() {
            if i > 0 {
                Divider::horizontal().show(ui);
            }
            if let Some(ran) = command_rows(ui, section) {
                *picked = Some(ran);
            }
        }
    });
}

/// The Control bar's name for the selection when it is not the appearance summary's: the Artboard tool's, the
/// anchor point's, an image's (the flag is true: its label is a link) or an Image Trace object's.
fn object_name(app: &VectorcraftApp, anchors: AnchorControls) -> Option<(&'static str, bool)> {
    if app.session.tool_id() == "artboard" {
        return Some((tl!("Artboard"), false));
    }
    let st = app.session.active()?;
    let [one] = st.selection.objects[..] else { return None };
    let node = st.doc.node(one)?;
    if anchors != AnchorControls::None {
        return Some((tl!("Anchor Point"), false));
    }
    match &node.kind {
        NodeKind::Image(im) => Some(if im.link.is_some() { (tl!("Linked File"), true) } else { (tl!("Embedded"), true) }),
        _ if crate::panels::image_trace::is_trace(node) => Some((tl!("Image Tracing"), false)),
        _ => None,
    }
}

/// The Control bar's object label, `label`. For one selected image (`image`: Linked File or Embedded) it is a
/// link, and a click opens the Links panel on the image with its Link Info.
fn object_label(app: &mut VectorcraftApp, ui: &mut Ui, label: &'static str, image: bool) {
    if !image {
        Text::new(label).elide(Elide::End).show(ui);
    } else if crate::panels::option_link(label).show(ui).clicked() {
        crate::panels::links::show_selected_image(app, ui.ctx());
    }
}

/// The Control bar's members, after its grip, in the order of the bar.
fn members(app: &mut VectorcraftApp, ui: &mut Ui) {
    BarGrip::new().show(ui);
    let Some(st) = app.session.active() else {
        Text::new(tl!("No Document")).shade(Shade::Muted).show(ui);
        return;
    };
    let count = st.selection.objects.len();
    let has_first = st.selection.objects.first().is_some_and(|id| st.doc.node(*id).is_some());
    if group_on(app, "snapping") {
        snapping_menu(app, ui);
    }
    let anchors = anchor_controls(app);
    let (label, image) = match object_name(app, anchors) {
        Some(named) => named,
        None => (tl!(crate::panels::appearance::summary(app).map_or("No Selection", |s| s.label)), false),
    };
    object_label(app, ui, label, image);
    // With the Artboard tool in use, the bar shows the active artboard's controls in place
    // of the object controls.
    let (artboard_options, transform) = (group_on(app, "artboardOptions"), group_on(app, "transform"));
    if crate::panels::artboards::control_bar(app, ui, artboard_options, transform) {
        if group_on(app, "alignDistribute") {
            group(ui, rank::GROUP, |ui| crate::panels::align::control_bar_strip(app, ui));
        }
        return;
    }
    if anchors != AnchorControls::None && group_on(app, "anchors") {
        group(ui, rank::GROUP, |ui| anchor_buttons(app, ui, anchors));
        corner_radius(app, ui);
    }
    let object_options = group_on(app, "objectOptions");
    if object_options {
        isolation_button(app, ui);
    }
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
    if !image && group_on(app, "fillStroke") {
        fill_and_stroke(app, ui);
    }
    if group_on(app, "opacity") {
        let opacity = if has_first { crate::panels::current_transparency(app).map_or(1.0, |t| t.0) } else { 1.0 };
        opacity_field(app, ui, opacity, count > 0);
    }
    if group_on(app, "graphicStyle") && count > 0 {
        style_group(app, ui);
    }
    if group_on(app, "character") {
        crate::panels::character::control_bar(app, ui);
    }
    if group_on(app, "paragraph") {
        crate::panels::paragraph::control_bar(app, ui);
    }
    if group_on(app, "verticalAlignment") && crate::panels::properties::vertical_align::shown(app).is_some() {
        group(ui, rank::GROUP, |ui| {
            crate::panels::properties::vertical_align::buttons(app, ui);
        });
    }
    if count == 0 {
        if group_on(app, "documentOptions") {
            document_buttons(app, ui);
        }
        return;
    }
    if group_on(app, "alignDistribute") {
        group(ui, rank::GROUP, |ui| crate::panels::align::control_bar_strip(app, ui));
    }
    if transform {
        // The Transform link opens the whole Transform panel in a popover; X/Y/W/H follow inline while the
        // bar has room.
        Divider::vertical().show(ui);
        crate::panels::transform::link(app, ui);
        if app.selection_box().is_some() {
            // PLAN: object-panels-w9's `object_bar_fields` is assumed to run `object.setBounds` for its
            // events itself and to show X, Y, W and H (position and size groups) only.
            ranked(ui, rank::FIELDS, |ui| {
                crate::panels::transform::object_bar_fields(app, ui);
            });
        }
    }
    // Select Similar Objects comes last: it is the first member a narrow bar leaves out.
    trailing_select_similar(app, ui);
}

/// The Document Setup and Preferences buttons, shown with nothing selected: a narrow bar leaves Document
/// Setup out first.
fn document_buttons(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (mut setup, mut preferences) = (false, false);
    group(ui, rank::DETAIL, |ui| setup |= Button::new(tl!("Document Setup")).show(ui).clicked());
    ranked(ui, rank::GROUP, |ui| preferences |= Button::new(tl!("Preferences")).show(ui).clicked());
    if setup {
        app.run("file.documentSetup", json!({})).ok();
    }
    if preferences {
        app.run("edit.preferences", json!({})).ok();
    }
}

/// The Control bar (Window → Control), context-sensitive: the shell's options slot shows it docked to the top
/// or the bottom of the window, or floating over the body ([`control_bar_place`]). It starts with the grip that
/// moves it and ends with the panel menu; a bar too narrow for its members leaves them out by their [`rank`].
pub fn control_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let floating = control_bar_place(&app.ui).floating;
    let menu_rows = panel_menu_rows(app);
    let mut picked = None;
    // PLAN: Crop Image's Apply and Cancel are `Bar::commit_cancel`, but chrome-w10's `control_bar_options(app, ui)`
    // takes no bar to put them on; chrome-w10 draws them as members.
    let mut bar = Bar::new(tl!("Control"), BarKind::Options).trailing(|ui, _| panel_menu(ui, &menu_rows, &mut picked));
    if floating {
        bar = bar.surface(SurfaceKind::Floating);
    }
    bar.show(ui, |ui, _| members(app, ui));
    if let Some((id, params)) = picked {
        crate::menus::invoke(app, id, params);
    }
}
