//! Align panel: Align Objects, Distribute Objects and Distribute Spacing (both with the spacing
//! value when aligning to a key object or a key guide) and Align To (selection / key object /
//! artboard). With the Artboard tool chosen, the buttons align and distribute the artboards the
//! tool has selected.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::ui::bars::{BarItem, BarItems};
use craft::ui::buttons::{IconButton, PopupMark, ToggleButton};
use craft::ui::contract::{Bind, Gap, PopupKind, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Divider;
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, MenuItem, PanelRow};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::text::NumberField;
use egui::Ui;
use serde::Serialize;
use serde_json::{Value, json};

// PLAN: assumed `part_hidden(app, key)` and `set_part_hidden(app, key, on)` (op.A5, hub-w2) live in `panels.rs`.
use super::{part_hidden, selection_len, set_part_hidden};
use crate::VectorcraftApp;

/// The `hidden_parts` key of the Align panel's Hide Options state; `panel_docking/sizes.rs` reads it too.
const HIDE_OPTIONS: &str = "align-hide-options";

/// What Align aligns to. The Align To choice kept with the UI state ([`crate::UiState::align_to`])
/// is Selection or Artboard: Align to Key Object stores no choice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
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

/// The params that an Object → Align menu item with params `p` sends: [`with_align_to`] unless `p`
/// names `to` or anchor points are selected (Direct Selection), when `p` goes unchanged and the
/// points align to their own bounds. Insert Menu Item stores the same params as the item's step.
pub fn menu_align_params(app: &VectorcraftApp, p: Value) -> Value {
    if p.as_object().is_some_and(|o| !o.contains_key("to")) && !has_anchors(app) { with_align_to(app, p) } else { p }
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

/// The glyphs of [`ALIGN`]'s buttons, in its order, in the Control bar and the Properties panel: the reference app drew
/// them there with Lucide's align glyphs, and only the Align panel with its own.
const BAR_ALIGN: [&str; 6] = [
    "align-start-vertical",
    "align-center-vertical",
    "align-end-vertical",
    "align-start-horizontal",
    "align-center-horizontal",
    "align-end-horizontal",
];

/// The glyph of align button `b` in the Control bar and the Properties panel ([`BAR_ALIGN`]); `None` for the other
/// buttons, which draw the panel's glyph there too.
pub(crate) fn bar_glyph(b: Button) -> Option<&'static str> {
    let Button::Align(axis, v) = b else { return None };
    ALIGN.iter().position(|r| r.2 == axis && r.3 == v).map(|i| BAR_ALIGN[i])
}

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

/// The command id of button `b`: [`COMMANDS`] is the one table of them.
fn command_id(b: Button) -> Option<&'static str> {
    COMMANDS.iter().find(|c| c.1 == b).map(|c| c.0)
}

/// The Align Objects buttons in the panel's order, the Horizontal & Vertical Align Center button
/// after the third: (icon, button).
fn align_icons() -> impl Iterator<Item = (&'static str, Button)> {
    ALIGN.iter().enumerate().flat_map(|(i, (icon, _, axis, v))| {
        let centers = (i == 3).then_some(("dc-al-center", Button::AlignCenters));
        centers.into_iter().chain([(*icon, Button::Align(axis, v))])
    })
}

/// The Distribute Objects buttons in the panel's order: (icon, button).
fn distribute_icons() -> impl Iterator<Item = (&'static str, Button)> {
    DISTRIBUTE.iter().map(|(icon, _, axis, v)| (*icon, Button::Distribute(axis, v)))
}

/// An icon button that runs the command of `b`; its label, state and click come from the command.
fn command_button(ui: &mut Ui, icon: &'static str, b: Button) {
    if let Some(id) = command_id(b) {
        IconButton::command(IconId::new(icon), CommandRef::new(id)).show(ui);
    }
}

/// A strip of the commands of `buttons`.
fn bar_items(buttons: impl Iterator<Item = Button>) -> BarItems {
    BarItems::new(buttons.filter_map(command_id).map(|id| BarItem::new(CommandRef::new(id))).collect())
}

/// The seven align buttons of the Control bar and the Properties panel. They align to what the
/// Align panel's Align To says (the key object while there is one), as its own buttons do (#789),
/// and the selected artboards with the Artboard tool chosen.
pub(crate) fn align_buttons(_app: &mut VectorcraftApp, ui: &mut Ui) {
    Row::new().wrap(true).show(ui, |ui, _| {
        for (icon, b) in align_icons() {
            command_button(ui, bar_glyph(b).unwrap_or(icon), b);
        }
    });
}

/// Whether the Align or Distribute button `b` can run now: the one rule of the panel, the Control
/// bar and Properties (`menus/state.rs` asks it for the command of each button). With the
/// Artboard tool chosen the buttons act on the artboards it has selected.
pub(crate) fn button_enabled(app: &VectorcraftApp, b: &Button) -> bool {
    let boards = on_artboards(app).then(|| artboards_selected(app));
    let n = selection_len(app) + selected_guides(app);
    match b {
        Button::Align(..) | Button::AlignCenters => boards.map_or(n >= 1, |b| b >= 1),
        // Two or more objects and ruler guides (#1597), or anchor points, which may all be on one
        // path; or the artboards [`artboards_distribute`] asks for.
        Button::Distribute(..) | Button::Spacing(_) => boards.map_or(n >= 2 || has_anchors(app), |_| artboards_distribute(app)),
    }
}

/// The Control bar's Align content: the align buttons, the Align To menu button showing the choice
/// of [`shown_align_to`], and the Distribute buttons while they can act (the artboards the Artboard
/// tool has selected, else two or more objects). The bar's row decides what leaves when it is
/// narrow.
pub(crate) fn control_bar_strip(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (icon, name) = align_to_icon_name(shown_align_to(app), key_is_anchor(app));
    let distributes = if on_artboards(app) { artboards_distribute(app) } else { objects_aligned(app) > 1 };
    Row::new().show(ui, |ui, _| {
        bar_items(align_icons().map(|(_, b)| b)).show(ui);
        let button = IconButton::new(IconId::new(icon))
            .label(tl!("Align To:"))
            .choice(name)
            .tooltip(name)
            .popup(PopupKind::Menu)
            .popup_mark(PopupMark::Menu)
            .show(ui);
        Menu::new(tl!("Align To")).opened_by(&button.response, Trigger::Press).show(ui, |ui, _| {
            for (id, _) in ALIGN_TO_COMMANDS {
                MenuItem::command(CommandRef::new(id)).show(ui);
            }
        });
        if distributes {
            bar_items(distribute_icons().map(|(_, b)| b)).show(ui);
        }
    });
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

/// What Align aligns to: the key object, the key guide (#2143) or the key anchor while there is one
/// (a click on an object of the selection makes it the key, and Align to Key Object makes the
/// topmost one the key, or a guide selected with a single object; a Shift-click with Direct
/// Selection makes the point it adds the key anchor), else the stored Align To choice, Selection or
/// Artboard. The Distribute Objects and Distribute Spacing buttons send it as their `to`.
fn align_to(app: &VectorcraftApp) -> AlignTo {
    if has_key(app) { AlignTo::Key } else { app.ui.align_to }
}

/// The icon and the translated name of Align To choice `v`; `anchor`: the key is an anchor point
/// (Direct Selection), and the key choice is named Align to Key Anchor.
fn align_to_icon_name(v: AlignTo, anchor: bool) -> (&'static str, &'static str) {
    match v {
        AlignTo::Selection => ("dc-alignto-selection", tl!("Align to Selection")),
        AlignTo::Key if anchor => ("dc-alignto-key", tl!("Align to Key Anchor")),
        AlignTo::Key => ("dc-alignto-key", tl!("Align to Key Object")),
        AlignTo::Artboard => ("dc-alignto-artboard", tl!("Align to Artboard")),
    }
}

/// Whether the key is an anchor point: the last of two or more anchor points that a Shift-click
/// with Direct Selection added ([`vectorcraft_doc::Selection::key_anchor`]).
fn key_is_anchor(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| st.selection.key_anchor.is_some())
}

/// The Align To choice that the Align panel and the Control bar show chosen: Artboard while one
/// object is selected (the buttons then align it to the artboard), else [`align_to`]. With the
/// Artboard tool chosen, it is the stored choice, Selection or Artboard.
pub(crate) fn shown_align_to(app: &VectorcraftApp) -> AlignTo {
    if on_artboards(app) {
        return app.ui.align_to;
    }
    aligns_to(align_to(app), objects_aligned(app))
}

/// What Align to Key Object makes the key while there is none, as `select.key` params: the topmost
/// of two or more selected objects when no other selected object holds it, or with one object, a
/// ruler guide selected with it. `None`: nothing can be made the key.
fn key_pick(app: &VectorcraftApp) -> Option<Value> {
    if selection_len(app) < 2 {
        return guide_for_key(app).map(|g| json!({"guide": g}));
    }
    let id = topmost(app)?;
    let st = app.session.active()?;
    let inside = st.doc.ancestry(id).unwrap_or_default().into_iter().any(|a| a != id && st.selection.contains(a));
    if inside { None } else { Some(json!({"id": id.0})) }
}

/// Whether Align To choice `v` can be chosen. Align to Key Object needs a key object, a key guide
/// or a key anchor, or something to make the key ([`key_pick`]). With the Artboard tool chosen it
/// is off: Align to Artboard keeps the active artboard in place.
pub(crate) fn can_choose(app: &VectorcraftApp, v: AlignTo) -> bool {
    if on_artboards(app) {
        return v != AlignTo::Key;
    }
    v != AlignTo::Key || has_key(app) || key_pick(app).is_some()
}

/// Chooses Align To `v`, as the Align panel's Align To buttons and the Control bar's Align To menu
/// do. Align to Key Object makes the key what [`key_pick`] gives, unless there is a key already:
/// the topmost selected object, or with one object, a guide selected with it. The stored choice stays as
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
        if !has_key(app)
            && let Some(p) = key_pick(app)
        {
            app.run("select.key", p).ok();
        }
    } else {
        app.ui.align_to = v;
        if has_key(app) {
            app.run("select.key", json!({})).ok();
        }
    }
}

/// The Align To choices as UI commands ([`crate::menus::UI_COMMANDS`]): (command id, choice). Object
/// › Align lists them after its align items, checked as the Align panel shows its choice, and Edit ›
/// Keyboard Shortcuts lists each under Align with no default shortcut.
pub const ALIGN_TO_COMMANDS: [(&str, AlignTo); 3] =
    [("align.toSelection", AlignTo::Selection), ("align.toKeyObject", AlignTo::Key), ("align.toArtboard", AlignTo::Artboard)];

/// The Align To choice of UI command `id` ([`ALIGN_TO_COMMANDS`]). `None`: `id` is not one of them.
pub(crate) fn align_to_command(id: &str) -> Option<AlignTo> {
    ALIGN_TO_COMMANDS.iter().find(|c| c.0 == id).map(|c| c.1)
}

/// Runs the UI command of Align To choice `v`: chooses it as the Align panel's Align To buttons do
/// ([`choose_align_to`]) → `{alignTo}`, the choice the Align panel then shows. An error when it
/// can't be chosen: Align to Key Object with the Artboard tool chosen, or with no key and nothing
/// to make the key ([`key_pick`]), which includes a topmost selected object inside another
/// selected object.
pub(crate) fn run_align_to(app: &mut VectorcraftApp, v: AlignTo) -> Result<Value, String> {
    if v == AlignTo::Key && on_artboards(app) {
        return Err("Align to Key Object isn't available with the Artboard tool".into());
    }
    if !can_choose(app, v) {
        return Err("no key object".into());
    }
    choose_align_to(app, v);
    if v == AlignTo::Key && !has_key(app) {
        return Err("no key object".into());
    }
    Ok(json!({"alignTo": shown_align_to(app).param()}))
}

/// The Align panel: Align Objects, Distribute Objects and, unless Hide Options is on, Distribute
/// Spacing with its value and Align To. The buttons are commands, so their state and clicks come
/// from the command layer ([`button_enabled`], [`run_button`]).
pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    // The reference app set the buttons apart in groups: left / centre / right of Align Objects
    // around the both-axes button, and the vertical and horizontal halves of Distribute Objects.
    let align: Vec<_> = align_icons().collect();
    crate::panels::subheading(tl!("Align Objects:")).show(ui, |ui, _| button_groups(ui, &[&align[..3], &align[3..4], &align[4..]]));
    Divider::horizontal().show(ui);
    let distribute: Vec<_> = distribute_icons().collect();
    crate::panels::subheading(tl!("Distribute Objects:")).show(ui, |ui, _| button_groups(ui, &[&distribute[..3], &distribute[3..]]));
    if part_hidden(app, HIDE_OPTIONS) {
        return;
    }
    Divider::horizontal().show(ui);
    // The Distribute Spacing value is sent while there is a key object or a key guide (#2143), or
    // for artboards while Align to Artboard is chosen.
    let spacing = app.ui.align_spacing;
    let sent = spacing_sent(app).is_some();
    // PLAN: assumed `crate::canvas::general_lengths(app) -> LengthUnits` (cv.B5b, canvas-w15).
    let lengths = crate::canvas::general_lengths(app);
    let min_spacing = spacing_min(app);
    let mut edited = None;
    // With one object selected the buttons align it to the artboard, and Align To shows Artboard.
    let anchor = key_is_anchor(app);
    // Distribute Spacing and Align To share a line, a rule between them, as the reference app had them.
    Row::new().gap(Gap::Section).show(ui, |ui, _| {
        crate::panels::subheading(tl!("Distribute Spacing:")).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                command_button(ui, "dc-dist-vspace", Button::Spacing("vertical"));
                command_button(ui, "dc-dist-hspace", Button::Spacing("horizontal"));
                // The value is UI state, not a document edit: a landed value is assigned.
                let field = NumberField::length(tl!("Distribute Spacing"), Bind::given(Some(spacing)), lengths)
                    .step(1.0)
                    .range(min_spacing..=f64::INFINITY)
                    .enabled(sent);
                edited = field.show(ui).edit.events.landed().cloned().flatten();
            });
        });
        Divider::vertical().show(ui);
        crate::panels::subheading(tl!("Align To:")).show(ui, |ui, _| {
            Row::new().show(ui, |ui, _| {
                for (id, v) in ALIGN_TO_COMMANDS {
                    let (icon, _) = align_to_icon_name(v, anchor);
                    ToggleButton::command(CommandRef::new(id)).icon(IconId::new(icon)).icon_only().show(ui);
                }
            });
        });
    });
    if let Some(v) = edited {
        app.ui.align_spacing = v;
    }
}

/// The command buttons of `groups` on one line, a section gap between the groups.
fn button_groups(ui: &mut Ui, groups: &[&[(&'static str, Button)]]) {
    Row::new().gap(Gap::Section).show(ui, |ui, _| {
        for group in groups {
            Row::new().show(ui, |ui, _| {
                for (icon, b) in *group {
                    command_button(ui, icon, *b);
                }
            });
        }
    });
}

/// The rows of the Align panel's menu.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    vec![
        PanelRow::Command(CommandRef::new("ui.align.hideOptions")),
        PanelRow::Command(CommandRef::new("ui.align.usePreviewBounds")),
        // Cancel Key Object clears the key, and Align To shows its stored choice again.
        PanelRow::Command(CommandRef::new("ui.align.cancelKeyObject")),
        PanelRow::Submenu(
            tl!("Align to Glyph Bounds").into(),
            vec![PanelRow::Command(CommandRef::new("ui.align.glyphBoundsPoint")), PanelRow::Command(CommandRef::new("ui.align.glyphBoundsArea"))],
        ),
    ]
}

/// The state of the panel menu's rows: Hide Options reads Show Options while the options are
/// hidden, Use Preview Bounds and the two glyph-bounds rows are checked while their preference is
/// on, and Cancel Key Object is on while there is a key.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let hidden = part_hidden(app, HIDE_OPTIONS);
    let prefs = &app.session.prefs;
    states.set(CommandRef::new("ui.align.hideOptions"), CommandState::new().title(if hidden { tl!("Show Options") } else { tl!("Hide Options") }));
    states.set(CommandRef::new("ui.align.usePreviewBounds"), CommandState::new().checked(prefs.use_preview_bounds));
    states.set(CommandRef::new("ui.align.cancelKeyObject"), CommandState::new().enabled(align_to(app) == AlignTo::Key));
    // The key choice is named for what the key is: Align to Key Anchor while it is an anchor point.
    let key = CommandState::new().enabled(can_choose(app, AlignTo::Key)).checked(shown_align_to(app) == AlignTo::Key);
    states.set(CommandRef::new("align.toKeyObject"), key.title(align_to_icon_name(AlignTo::Key, key_is_anchor(app)).1));
    states.set(CommandRef::new("ui.align.glyphBoundsPoint"), CommandState::new().checked(prefs.align_glyph_bounds_point));
    states.set(CommandRef::new("ui.align.glyphBoundsArea"), CommandState::new().checked(prefs.align_glyph_bounds_area));
}

/// Runs the Align panel menu command `id`; `None` when `id` is not one of them. The glyph-bounds
/// rows make Align and Distribute measure point or area text by the ink of its glyphs.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    let prefs = &app.session.prefs;
    let pref = match id {
        "ui.align.usePreviewBounds" => Some(("usePreviewBounds", prefs.use_preview_bounds)),
        "ui.align.glyphBoundsPoint" => Some(("alignGlyphBoundsPoint", prefs.align_glyph_bounds_point)),
        "ui.align.glyphBoundsArea" => Some(("alignGlyphBoundsArea", prefs.align_glyph_bounds_area)),
        _ => None,
    };
    if let Some((key, on)) = pref {
        super::transform::set_pref(app, key, !on);
        return Some(Ok(Value::Null));
    }
    match id {
        "ui.align.hideOptions" => {
            let hidden = part_hidden(app, HIDE_OPTIONS);
            set_part_hidden(app, HIDE_OPTIONS, !hidden);
        }
        "ui.align.cancelKeyObject" => return Some(app.run("select.key", json!({}))),
        _ => return None,
    }
    Some(Ok(Value::Null))
}

#[cfg(test)]
mod tests;
