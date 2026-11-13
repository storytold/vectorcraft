//! Align panel: Align Objects, Distribute Objects and Distribute Spacing (both with the spacing
//! value when aligning to a key object or a key guide) and Align To (selection / key object /
//! artboard). With the Artboard tool chosen, the buttons align and distribute the artboards the
//! tool has selected.

use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

use super::{pstate, selection_len, set_pstate};
use crate::VectorcraftApp;
use crate::widgets::{self, menu_item};

/// What Align aligns to. The Align To choice kept with the UI state ([`crate::UiState::align_to`])
/// is Selection or Artboard: Align to Key Object stores no choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AlignTo {
    #[default]
    Selection,
    Key,
    Artboard,
}

impl AlignTo {
    pub fn param(self) -> &'static str {
        match self {
            AlignTo::Selection => "selection",
            AlignTo::Key => "key",
            AlignTo::Artboard => "artboard",
        }
    }
}

/// Serde reader for the saved Align To choice ([`crate::UiState::align_to`]): `"artboard"` is
/// Artboard, and any other value, `"key"` included, is Selection.
pub fn deserialize_align_to<'de, D: serde::Deserializer<'de>>(d: D) -> Result<AlignTo, D::Error> {
    let v = <Value as serde::Deserialize>::deserialize(d)?;
    Ok(if v == "artboard" { AlignTo::Artboard } else { AlignTo::Selection })
}

/// What Align aligns to with `n_selected` objects selected: the artboard for a single object
/// (unless a key guide is the key: #2143), else `to`.
fn aligns_to(to: AlignTo, n_selected: usize) -> AlignTo {
    if n_selected == 1 && to != AlignTo::Key { AlignTo::Artboard } else { to }
}

/// `object.align` params: a single object aligns to the artboard (Illustrator does the same).
pub fn align_params(base: Value, to: AlignTo, n_selected: usize) -> Value {
    let mut p = base;
    p["to"] = json!(aligns_to(to, n_selected).param());
    p
}

/// `object.align` params `base` with the `to` that the align buttons of the Align panel, the Control
/// bar and the Properties panel send: the key while there is one (a key object, or a key guide:
/// #2143), else the artboard for a single selected object, else the Align To choice. Object → Align
/// sends it too while every selected object is selected whole ([`crate::menus::invoke`]).
pub fn with_align_to(app: &VectorcraftApp, base: Value) -> Value {
    align_params(base, align_to(app), objects_aligned(app))
}

/// The selected objects that Align aligns as objects: none while anchor points are selected
/// (Direct Selection), since the points align and a single path's points must not go to the
/// artboard as a single object does (#1636).
fn objects_aligned(app: &VectorcraftApp) -> usize {
    if has_anchors(app) { 0 } else { selection_len(app) }
}

/// Whether anchor points are selected (Direct Selection): Align and Distribute then act on the points.
fn has_anchors(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| !st.selection.anchors.is_empty())
}

pub const ALIGN: [(&str, &str, &str, &str); 6] = [
    ("dc-al-left", "Horizontal Align Left", "horizontal", "left"),
    ("dc-al-hcenter", "Horizontal Align Center", "horizontal", "center"),
    ("dc-al-right", "Horizontal Align Right", "horizontal", "right"),
    ("dc-al-top", "Vertical Align Top", "vertical", "top"),
    ("dc-al-vcenter", "Vertical Align Center", "vertical", "center"),
    ("dc-al-bottom", "Vertical Align Bottom", "vertical", "bottom"),
];

pub const DISTRIBUTE: [(&str, &str, &str, &str); 6] = [
    ("dc-dist-top", "Vertical Distribute Top", "vertical", "top"),
    ("dc-dist-vcenter", "Vertical Distribute Center", "vertical", "center"),
    ("dc-dist-bottom", "Vertical Distribute Bottom", "vertical", "bottom"),
    ("dc-dist-left", "Horizontal Distribute Left", "horizontal", "left"),
    ("dc-dist-hcenter", "Horizontal Distribute Center", "horizontal", "center"),
    ("dc-dist-right", "Horizontal Distribute Right", "horizontal", "right"),
];

/// What a button of the Align panel's Align Objects, Distribute Objects and Distribute Spacing
/// rows runs ([`run_button`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// An Align Objects button: `object.align {axis: value}`.
    Align(&'static str, &'static str),
    /// Horizontal & Vertical Align Center: `object.align {horizontal: "center", vertical: "center"}`.
    AlignCenters,
    /// A Distribute Objects button: `object.distribute {axis: value}`.
    Distribute(&'static str, &'static str),
    /// A Distribute Spacing button: `object.distributeSpacing {axis}`.
    Spacing(&'static str),
}

impl Button {
    /// The engine command the button runs on the selected objects.
    pub(crate) fn command(self) -> &'static str {
        match self {
            Button::Align(..) | Button::AlignCenters => "object.align",
            Button::Distribute(..) => "object.distribute",
            Button::Spacing(_) => "object.distributeSpacing",
        }
    }

    /// The engine command the button runs on the selected artboards ([`on_artboards`]).
    fn artboard_command(self) -> &'static str {
        match self {
            Button::Align(..) | Button::AlignCenters => "artboard.align",
            Button::Distribute(..) => "artboard.distribute",
            Button::Spacing(_) => "artboard.distributeSpacing",
        }
    }

    /// The engine command a click on the button runs now: the artboard command with the Artboard
    /// tool chosen, else the object command.
    pub(crate) fn command_now(self, app: &VectorcraftApp) -> &'static str {
        if on_artboards(app) { self.artboard_command() } else { self.command() }
    }
}

/// Whether the Align panel's buttons, and the Control bar's, align and distribute artboards: the
/// Artboard tool is chosen. They then act on the artboards the tool has selected.
pub(crate) fn on_artboards(app: &VectorcraftApp) -> bool {
    app.session.tool_id() == "artboard"
}

/// The number of artboards the Artboard tool has selected ([`super::artboards::picked`]).
pub(crate) fn artboards_selected(app: &VectorcraftApp) -> usize {
    let n = app.session.active().map_or(0, |d| d.doc.artboards.len());
    if n == 0 { 0 } else { super::artboards::picked(app, n).len() }
}

/// With the Artboard tool chosen, whether the Distribute buttons of the Align panel and the Control
/// bar can space the selected artboards: two or more with Align to Artboard, whose spacing places
/// the second from the active one (the Artboard tool's selection always holds its active artboard),
/// else three or more, since the first and the last stay where they are.
pub(crate) fn artboards_distribute(app: &VectorcraftApp) -> bool {
    artboards_selected(app) >= if app.ui.align_to == AlignTo::Artboard { 2 } else { 3 }
}

/// Runs what a click on button `b` runs with the Artboard tool chosen: the artboard command on
/// the artboards the tool has selected. With Align to Artboard chosen, it sends `to: "artboard"`,
/// which keeps the tool's active artboard in place, and the Distribute buttons send the
/// Distribute Spacing value. With Align to Selection chosen, it sends `to: "selection"`.
fn run_on_artboards(app: &mut VectorcraftApp, b: Button) -> Result<Value, String> {
    let to_artboard = app.ui.align_to == AlignTo::Artboard;
    let mut p = match b {
        Button::Align(axis, v) | Button::Distribute(axis, v) => json!({axis: v}),
        Button::AlignCenters => json!({"horizontal": "center", "vertical": "center"}),
        Button::Spacing(axis) => json!({"axis": axis}),
    };
    p["to"] = json!(if to_artboard { "artboard" } else { "selection" });
    if to_artboard && matches!(b, Button::Distribute(..) | Button::Spacing(_)) {
        p["spacing"] = json!(app.ui.align_spacing);
    }
    app.run(b.artboard_command(), p)
}

/// The Align panel's buttons as UI commands ([`crate::menus::UI_COMMANDS`]), in the panel's order:
/// (command id, button). Edit › Keyboard Shortcuts lists each under Align with its button's label,
/// with no default shortcut, and a shortcut runs what a click on the button runs.
pub const COMMANDS: [(&str, Button); 15] = [
    ("align.horizontalLeft", Button::Align("horizontal", "left")),
    ("align.horizontalCenter", Button::Align("horizontal", "center")),
    ("align.horizontalRight", Button::Align("horizontal", "right")),
    ("align.horizontalVerticalCenter", Button::AlignCenters),
    ("align.verticalTop", Button::Align("vertical", "top")),
    ("align.verticalCenter", Button::Align("vertical", "center")),
    ("align.verticalBottom", Button::Align("vertical", "bottom")),
    ("align.distributeVerticalTop", Button::Distribute("vertical", "top")),
    ("align.distributeVerticalCenter", Button::Distribute("vertical", "center")),
    ("align.distributeVerticalBottom", Button::Distribute("vertical", "bottom")),
    ("align.distributeHorizontalLeft", Button::Distribute("horizontal", "left")),
    ("align.distributeHorizontalCenter", Button::Distribute("horizontal", "center")),
    ("align.distributeHorizontalRight", Button::Distribute("horizontal", "right")),
    ("align.distributeVerticalSpace", Button::Spacing("vertical")),
    ("align.distributeHorizontalSpace", Button::Spacing("horizontal")),
];

/// The button of Align panel command `id` ([`COMMANDS`]), which [`run_button`] runs and which is
/// enabled while the engine command it runs is. `None`: `id` is not one of them.
pub(crate) fn button(id: &str) -> Option<Button> {
    COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// Runs what a click on button `b` runs. An Align Objects button sends the `to` of
/// [`with_align_to`]. A Distribute Objects or Distribute Spacing button sends the key while there
/// is one, else the Align To choice, and the Distribute Spacing value while there is a key. With
/// the Artboard tool chosen, the button acts on the selected artboards ([`run_on_artboards`]).
pub fn run_button(app: &mut VectorcraftApp, b: Button) -> Result<Value, String> {
    if on_artboards(app) {
        return run_on_artboards(app, b);
    }
    let to = align_to(app).param();
    let p = match b {
        Button::Align(axis, v) => with_align_to(app, json!({axis: v})),
        Button::AlignCenters => with_align_to(app, json!({"horizontal": "center", "vertical": "center"})),
        Button::Distribute(axis, v) => {
            let mut p = json!({axis: v, "to": to});
            if let Some(g) = spacing_sent(app) {
                p["spacing"] = json!(g);
            }
            p
        }
        Button::Spacing(axis) => json!({"axis": axis, "spacing": spacing_sent(app), "to": to}),
    };
    app.run(b.command(), p)
}

/// The Distribute Spacing value ([`crate::UiState::align_spacing`]) while there is a key object
/// or a key guide (#2143), which the Distribute Objects and Distribute Spacing buttons then send;
/// else none. With the Artboard tool chosen, the buttons send it while Align to Artboard is chosen.
fn spacing_sent(app: &VectorcraftApp) -> Option<f64> {
    let sent = if on_artboards(app) { app.ui.align_to == AlignTo::Artboard } else { align_to(app) == AlignTo::Key };
    sent.then_some(app.ui.align_spacing)
}

/// The lowest Distribute Spacing value the Align panel's field takes: none with the Artboard tool
/// chosen, since `artboard.distributeSpacing` overlaps the artboards with a negative spacing, else 0.
fn spacing_min(app: &VectorcraftApp) -> f64 {
    if on_artboards(app) { f64::NEG_INFINITY } else { 0.0 }
}

/// The six align buttons of the Control bar and the Properties panel, `size` points square.
/// They align to what the Align panel's Align To says (the key object while there is one), as
/// its own buttons do (#789), and the selected artboards with the Artboard tool chosen.
pub fn align_buttons(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    for (i, (icon, tip, axis, v)) in [
        ("align-start-vertical", "Horizontal Align Left", "horizontal", "left"),
        ("align-center-vertical", "Horizontal Align Center", "horizontal", "center"),
        ("align-end-vertical", "Horizontal Align Right", "horizontal", "right"),
        ("align-start-horizontal", "Vertical Align Top", "vertical", "top"),
        ("align-center-horizontal", "Vertical Align Center", "vertical", "center"),
        ("align-end-horizontal", "Vertical Align Bottom", "vertical", "bottom"),
    ]
    .into_iter()
    .enumerate()
    {
        if i == 3 && widgets::icon_button(ui, "dc-al-center", tl!("Horizontal & Vertical Align Center"), false, size).clicked() {
            run_button(app, Button::AlignCenters).ok();
        }
        if widgets::icon_button(ui, icon, tl!(tip), false, size).clicked() {
            run_button(app, Button::Align(axis, v)).ok();
        }
    }
}

/// The six Distribute Objects buttons of the Control bar, `size` points square. They distribute
/// what the Align panel's buttons do, to what its Align To says.
pub(crate) fn distribute_buttons(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    for (icon, tip, axis, v) in DISTRIBUTE {
        if widgets::icon_button(ui, icon, tl!(tip), false, size).clicked() {
            run_button(app, Button::Distribute(axis, v)).ok();
        }
    }
}

/// Whether the selection has a key object or a key guide.
fn has_key(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.has_key())
}

/// The first selected ruler guide while objects are selected too: the one Align to Key Object
/// makes the key when there is no other object to be it.
fn guide_for_key(app: &VectorcraftApp) -> Option<usize> {
    let st = app.session.active().filter(|st| !st.selection.is_empty())?;
    let first = st.selection.guides.first().copied()?;
    vectorcraft_engine::cmd::guide_indexes(&st.doc, &[first]).first().copied()
}

/// The number of selected ruler guides, which Distribute spaces with the objects (#1597).
fn selected_guides(app: &VectorcraftApp) -> usize {
    app.session.active().map_or(0, |st| st.selection.guides.len())
}

/// The topmost selected object, the last in paint order.
fn topmost(app: &VectorcraftApp) -> Option<vectorcraft_doc::NodeId> {
    app.session.active().and_then(|st| st.selection.in_paint_order(&st.doc).last().copied())
}

/// What Align aligns to: the key object or the key guide (#2143) while there is one (a click on
/// an object of the selection makes it the key, and Align to Key Object makes the topmost one the
/// key, or a guide selected with a single object), else the stored Align To choice, Selection or
/// Artboard. The Distribute Objects and Distribute Spacing buttons send it as their `to`.
fn align_to(app: &VectorcraftApp) -> AlignTo {
    if has_key(app) { AlignTo::Key } else { app.ui.align_to }
}

/// The Align To choices of the Align panel and of the Control bar's Align To menu, in their order.
const ALIGN_TO: [AlignTo; 3] = [AlignTo::Selection, AlignTo::Key, AlignTo::Artboard];

/// The icon and the translated name of Align To choice `v`.
fn align_to_icon_name(v: AlignTo) -> (&'static str, &'static str) {
    match v {
        AlignTo::Selection => ("dc-alignto-selection", tl!("Align to Selection")),
        AlignTo::Key => ("dc-alignto-key", tl!("Align to Key Object")),
        AlignTo::Artboard => ("dc-alignto-artboard", tl!("Align to Artboard")),
    }
}

/// The Align To choice that the Align panel and the Control bar show chosen: Artboard while one
/// object is selected (the buttons then align it to the artboard), else [`align_to`]. With the
/// Artboard tool chosen, it is the stored choice, Selection or Artboard.
fn shown_align_to(app: &VectorcraftApp) -> AlignTo {
    if on_artboards(app) {
        return app.ui.align_to;
    }
    aligns_to(align_to(app), objects_aligned(app))
}

/// Whether Align To choice `v` can be chosen. Align to Key Object needs a key object or a key
/// guide, or something to make one of: two or more selected objects, or a guide selected with
/// objects. With the Artboard tool chosen it is off: Align to Artboard keeps the active artboard
/// in place.
fn can_choose(app: &VectorcraftApp, v: AlignTo) -> bool {
    if on_artboards(app) {
        return v != AlignTo::Key;
    }
    v != AlignTo::Key || selection_len(app) >= 2 || has_key(app) || guide_for_key(app).is_some()
}

/// Chooses Align To `v`, as the Align panel's Align To buttons and the Control bar's Align To menu
/// do. Align to Key Object makes the topmost selected object the key object, unless there is one
/// already; with one object, a guide selected with it becomes the key. The stored choice stays as
/// it was, and Align To shows it again once the key is cleared. Align to Selection and Align to
/// Artboard store the choice and let go of the key object. With the Artboard tool chosen, the
/// choice is stored and the selected objects keep their key.
pub(crate) fn choose_align_to(app: &mut VectorcraftApp, v: AlignTo) {
    if !can_choose(app, v) {
        return;
    }
    if on_artboards(app) {
        app.ui.align_to = v;
        return;
    }
    if v == AlignTo::Key {
        if !has_key(app) {
            match (topmost(app).filter(|_| selection_len(app) >= 2), guide_for_key(app)) {
                (Some(id), _) => app.run("select.key", json!({"id": id.0})).ok(),
                (None, Some(g)) => app.run("select.key", json!({"guide": g})).ok(),
                (None, None) => None,
            };
        }
    } else {
        app.ui.align_to = v;
        if has_key(app) {
            app.run("select.key", json!({})).ok();
        }
    }
}

/// The width of the Control bar's Align To menu ([`align_to_menu`]) for buttons `size` points
/// square: the icon of the choice and the arrow after it.
pub(crate) fn align_to_menu_width(size: f32) -> f32 {
    size + ALIGN_TO_ARROW
}

/// The width of the arrow of the Control bar's Align To menu.
const ALIGN_TO_ARROW: f32 = 14.0;

/// The Control bar's Align To menu, next to its Align buttons: the icon of the choice that the
/// Align panel's Align To shows ([`shown_align_to`]) and an arrow, `size` points tall. Its menu
/// lists Align to Selection, Align to Key Object and Align to Artboard with that choice checked,
/// and a pick chooses it as the Align panel does ([`choose_align_to`]). Align to Key Object is off
/// while no key object can be made. Screen readers get a combo box named "Align To:" whose value
/// is the choice.
pub(crate) fn align_to_menu(app: &mut VectorcraftApp, ui: &mut Ui, size: f32) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let shown = shown_align_to(app);
    let (icon, name) = align_to_icon_name(shown);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(align_to_menu_width(size), size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(5), t.hover);
    }
    let square = egui::Rect::from_min_size(rect.min, egui::vec2(size, size));
    crate::icons::paint(ui, icon, square.shrink((size * 0.2).round()), t.icon);
    let arrow = egui::Rect::from_center_size(egui::pos2(square.right() + ALIGN_TO_ARROW / 2.0, rect.center().y), egui::vec2(10.0, 10.0));
    crate::icons::paint(ui, "chevron-down", arrow, t.text_dim);
    resp.widget_info(|| egui::WidgetInfo {
        current_text_value: Some(name.to_owned()),
        ..egui::WidgetInfo::labeled(egui::Role::ComboBox, ui.is_enabled(), tl!("Align To:"))
    });
    let resp = resp.on_hover_text(name);
    let picked = egui::Popup::menu(&resp).show(|ui| {
        let mut picked = None;
        for v in ALIGN_TO {
            if menu_item(ui, align_to_icon_name(v).1, can_choose(app, v), shown == v) {
                picked = Some(v);
            }
        }
        picked
    });
    if let Some(v) = picked.and_then(|r| r.inner) {
        choose_align_to(app, v);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = selection_len(app);
    let guides = selected_guides(app);
    // With the Artboard tool chosen, the buttons act on the artboards it has selected.
    let boards = on_artboards(app).then(|| artboards_selected(app));
    let can_align = boards.map_or(n + guides >= 1, |b| b >= 1);
    // `spacing` is the Distribute Spacing value. The Distribute Objects and Distribute Spacing
    // buttons send it while there is a key object or a key guide (#2143), or for artboards while
    // Align to Artboard is chosen.
    let spacing = app.ui.align_spacing;
    let key = spacing_sent(app).is_some();
    // Two or more objects and ruler guides (#1597), or anchor points, which may all be on one
    // path; or the artboards [`artboards_distribute`] asks for.
    let distributes = boards.map_or(n + selected_guides(app) >= 2 || has_anchors(app), |_| artboards_distribute(app));
    let (unit, min_spacing) = (app.session.general_unit(), spacing_min(app));
    widgets::subheader(ui, tl!("Align Objects:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (i, (icon, tip, axis, v)) in ALIGN.iter().enumerate() {
            if i == 3 {
                ui.add_space(6.0);
                if widgets::icon_button_enabled(ui, "dc-al-center", tl!("Horizontal & Vertical Align Center"), false, can_align, 32.0).clicked() {
                    run_button(app, Button::AlignCenters).ok();
                }
                ui.add_space(6.0);
            }
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, can_align, 32.0).clicked() {
                run_button(app, Button::Align(axis, v)).ok();
            }
        }
    });
    widgets::divider(ui);
    widgets::subheader(ui, tl!("Distribute Objects:"));
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        for (i, (icon, tip, axis, v)) in DISTRIBUTE.iter().enumerate() {
            if i == 3 {
                ui.add_space(6.0);
            }
            if widgets::icon_button_enabled(ui, icon, tl!(tip), false, distributes, 32.0).clicked() {
                run_button(app, Button::Distribute(axis, v)).ok();
            }
        }
    });
    if pstate::<bool>(ui.ctx(), "align-hide-options") {
        return;
    }
    widgets::divider(ui);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Distribute Spacing:"));
            ui.horizontal(|ui| {
                let on = distributes;
                if widgets::icon_button_enabled(ui, "dc-dist-vspace", tl!("Vertical Distribute Space"), false, on, 28.0).clicked() {
                    run_button(app, Button::Spacing("vertical")).ok();
                }
                if widgets::icon_button_enabled(ui, "dc-dist-hspace", tl!("Horizontal Distribute Space"), false, on, 28.0).clicked() {
                    run_button(app, Button::Spacing("horizontal")).ok();
                }
                ui.add_enabled_ui(key, |ui| {
                    if let Some(v) = widgets::spin_field(ui, "align-spacing", Some(spacing), unit, 70.0, 1.0, min_spacing, &[]) {
                        app.ui.align_spacing = v;
                    }
                });
            });
        });
        ui.separator();
        ui.vertical(|ui| {
            widgets::subheader(ui, tl!("Align To:"));
            // With one object selected the buttons align it to the artboard, and Align To shows Artboard.
            let shown = shown_align_to(app);
            ui.horizontal(|ui| {
                for v in ALIGN_TO {
                    let (icon, tip) = align_to_icon_name(v);
                    if widgets::icon_button_enabled(ui, icon, tip, shown == v, can_choose(app, v), 28.0).clicked() {
                        choose_align_to(app, v);
                    }
                }
            });
        });
    });
}

pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let hidden: bool = pstate(ui.ctx(), "align-hide-options");
    if menu_item(ui, if hidden { tl!("Show Options") } else { tl!("Hide Options") }, true, false) {
        set_pstate(ui.ctx(), "align-hide-options", !hidden);
    }
    let pb = app.session.prefs.use_preview_bounds;
    if menu_item(ui, tl!("Use Preview Bounds"), true, pb) {
        super::transform::set_pref(app, "usePreviewBounds", !pb);
    }
    // Cancel Key Object clears the key, and Align To shows its stored choice again.
    if menu_item(ui, tl!("Cancel Key Object"), align_to(app) == AlignTo::Key, false) {
        app.run("select.key", json!({})).ok();
    }
    // Indented like the items beside it (their check column).
    ui.menu_button(format!("   {}", tl!("Align to Glyph Bounds")), |ui| glyph_bounds_items(app, ui));
}

/// The Align to Glyph Bounds submenu: Point Text and Area Text, each checked while Align and
/// Distribute measure that kind of type by the ink of its glyphs.
fn glyph_bounds_items(app: &mut VectorcraftApp, ui: &mut Ui) {
    for (label, key, on) in [
        (tl!("Point Text"), "alignGlyphBoundsPoint", app.session.prefs.align_glyph_bounds_point),
        (tl!("Area Text"), "alignGlyphBoundsArea", app.session.prefs.align_glyph_bounds_area),
    ] {
        if menu_item(ui, label, true, on) {
            super::transform::set_pref(app, key, !on);
        }
    }
}

#[cfg(test)]
mod tests;
