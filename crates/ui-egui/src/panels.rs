//! Panels. Each panel reads engine state and acts only through `app.run(...)` (slider drags
//! preview through the session's interaction so one drag is one undo step).
//!
//! Every icon panel has a body (`show_icon_panel`); its panel (≡) menu rows come from `panel_rows`.

pub mod actions;
pub mod align;
pub mod appearance;
pub mod artboards;
pub mod asset_export;
pub mod attributes;
pub mod blend;
pub mod brushes;
pub mod character;
pub mod color;
pub mod color_guide;
pub mod color_themes;
pub mod color_wheel;
pub mod css_properties;
pub mod discover;
pub mod doc_info;
pub mod flattener_preview;
pub mod glyphs;
pub mod gradient;
pub mod graphic_styles;
pub mod history;
pub mod image_trace;
pub mod info;
pub mod layers;
pub mod learn;
pub mod libraries;
pub mod library_panel;
pub mod links;
pub mod magic_wand;
pub(crate) mod memo;
pub mod navigator;
pub mod opentype;
pub mod paragraph;
pub mod pathfinder;
pub mod pattern_options;
pub mod preflight;
pub mod properties;
pub mod retype;
pub mod script_console;
pub mod separations;
pub mod stroke;
pub mod svg_interactivity;
pub mod swatches;
pub mod symbols;
pub mod tabs;
pub mod text_styles;
pub mod three_d;
pub mod transform;
pub mod transparency;
pub mod variables;

use std::sync::Arc;

use craft::behavior::{IconId, Mixed};
use craft::color_ui::ui::{ChannelRule, HarmonyCommand};
use craft::color_ui::ui::{PaintChip, PairWell, SwatchPair, SwatchPairOutput, SwatchPalette, Well};
use craft::keymap::CommandRef;
use craft::theme::Srgba;
use craft::theme::variant::Tone;
use craft::ui::buttons::IconButton;
use craft::ui::choice::SwatchSpacing;
use craft::ui::contract::{Bind, CommandSet as _, Entry, ListDrag, PopupKind, Props as _, Show as _, ShowContainer as _, Sizing, Trigger};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row;
use craft::ui::menu::{Menu, PanelMenu, PanelRow};
use craft::ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::popover::Popover;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::CommandStates;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{HeaderStrength, Section};
use craft::ui::text::{NumberField, link_toggle};
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{LiveCorners, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_engine::inspect::StrokeMixed;
use vectorcraft_geom::shapes::CornerKind;

use crate::VectorcraftApp;
use crate::canvas::rulers::general_lengths;
use crate::edits::{EditPath, apply_edits};
use crate::paint_preview::well_paint;
use crate::panel_drag::PanelDrag;

/// The first selected node, as the `Arc` the document holds, if any. It does not change with later
/// edits: read it again after one.
pub fn first_selected(app: &VectorcraftApp) -> Option<Arc<Node>> {
    let st = app.session.active()?;
    st.selection.objects.first().and_then(|id| layers::node_arc(&st.doc, *id))
}

/// The radius the Live Corners of path `n` show in the panels: that of the corners the panels set
/// (the Direct-Selected ones, else every corner), blank when they differ.
pub(crate) fn corner_radius(app: &VectorcraftApp, n: &Node) -> Option<f64> {
    corner_style(app, n).0
}

/// The radius and kind of the corners [`corner_radius`] reads, each None when they differ.
pub(crate) fn corner_style(app: &VectorcraftApp, n: &Node) -> (Option<f64>, Option<CornerKind>) {
    let partial = app.session.active().and_then(|d| d.selection.partial(n.id));
    LiveCorners::of(n).map_or((None, None), |c| c.style(&c.picked(partial)))
}

/// One entry per corner kind, labelled as the Corners dialog labels them.
pub(crate) fn corner_entries() -> Vec<Entry<CornerKind>> {
    CornerKind::ALL.iter().map(|k| Entry::new(*k, crate::dialogs::corners::kind_label(*k))).collect()
}

/// The Corner Radius row of a live rectangle's properties: [`corner_radius_field`] and the link
/// behind it, the `singleCornerRadius` preference. On, a Corner row follows that sets the kind of
/// all four corners; off, the field goes and a radius field and a Corner select follow for each of
/// the four corners, each setting its own corner.
pub(crate) fn corner_radius_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let single = app.session.prefs.single_corner_radius;
    let mut toggled = None;
    Field::new(tl!("Corner Radius")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if single {
                corner_radius_field(app, ui, n);
            }
            toggled = link_toggle(tl!("Single Corner Radius"), Bind::given(single)).show(ui).value().copied();
        });
    });
    if let Some(on) = toggled {
        transform::set_pref(app, "singleCornerRadius", on);
    }
    if single {
        corner_kind_row(app, ui, n);
    } else {
        corner_fields(app, ui, n);
    }
}

/// The Corner select of a live rectangle whose corners share one radius field: the kind of all
/// four corners, also while some of them are Direct-Selected, blank when they differ. A pick sets
/// it on every corner of each selected live rectangle ([`set_rectangle_kinds`]).
fn corner_kind_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let Some(lc) = LiveCorners::of(n) else { return };
    let entries = corner_entries();
    let current = lc.style(&lc.all()).1;
    let picked = Field::new(tl!("Corner")).show(ui, |ui, _| corner_select(ui, tl!("Corner"), current, &entries)).inner.flatten();
    if let Some(kind) = picked {
        set_rectangle_kinds(app, kind, None);
    }
}

/// A select of the corner kinds named `name`, showing `current` (blank for None), as wide as its
/// widest kind, so it fits beside a corner's radius field. Returns the kind picked.
fn corner_select(ui: &mut Ui, name: &'static str, current: Option<CornerKind>, entries: &[Entry<CornerKind>]) -> Option<CornerKind> {
    Select::new(name, Bind::given(current.map_or(Mixed::Mixed, Mixed::Same)), entries).sizing(Sizing::Content).show(ui).value().copied()
}

/// Set `kind` on corner `corner` (every corner for None) of each selected live rectangle
/// ([`set_rectangle_corners`]).
fn set_rectangle_kinds(app: &mut VectorcraftApp, kind: CornerKind, corner: Option<usize>) {
    set_rectangle_corners(app, json!({"kind": kind}), corner);
}

/// Run `object.setLiveShape` with `p` (a `kind` or `radius`) on corner `corner` (every corner for
/// None) of each selected live rectangle, leaving other selected objects as they are: one undo
/// step, or none when nothing changes.
fn set_rectangle_corners(app: &mut VectorcraftApp, mut p: Value, corner: Option<usize>) {
    p["corners"] = corner.map_or(json!([0, 1, 2, 3]), |k| json!([k]));
    // The edit the tool computes on another thread comes first: the rectangles are the ones
    // selected after it.
    match app.session.finish_tool_edit() {
        Ok(reqs) => crate::canvas::apply_requests(app, Ok(reqs)),
        Err(e) => {
            app.status(e.to_string());
            return;
        }
    }
    if app.session.active().is_none() {
        return;
    }
    // The live rectangles of the selection when it runs, so an action that records it acts on the
    // rectangles selected when it plays.
    p["rectanglesOnly"] = json!(true);
    if let Err(e) = app.run("object.setLiveShape", p) {
        app.status(e);
    }
}

/// The corners of a live rectangle in the order their fields show, with their labels (which name their radius
/// fields) and the names of their Corner selects.
fn rectangle_corners() -> [(usize, &'static str, &'static str); 4] {
    [
        (0, tl!("Top Left"), tl!("Top Left Corner")),
        (1, tl!("Top Right"), tl!("Top Right Corner")),
        (2, tl!("Bottom Right"), tl!("Bottom Right Corner")),
        (3, tl!("Bottom Left"), tl!("Bottom Left Corner")),
    ]
}

/// One Corner Radius field and one Corner select per corner of a live rectangle, each pair on its
/// own row after the corner's label, the labels in one column; in a panel too narrow for that,
/// each select wraps under its field. A value typed or a kind picked is set on that corner alone,
/// of each selected live rectangle ([`set_rectangle_corners`]).
fn corner_fields(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let Some(lc) = LiveCorners::of(n) else { return };
    let lengths = general_lengths(app);
    let entries = corner_entries();
    let mut radii = Vec::new();
    let mut kinds = Vec::new();
    FieldGroup::new(tl!("Corners")).show(ui, |ui, _| {
        for (k, label, corner) in rectangle_corners() {
            Field::new(label).show(ui, |ui, _| {
                Row::new().wrap(true).show(ui, |ui, _| {
                    radii.push((k, NumberField::length(label, Bind::given(Some(lc.radius(k))), lengths).show(ui).edit));
                    if let Some(kind) = corner_select(ui, corner, Some(lc.kind(k)), &entries) {
                        kinds.push((k, kind));
                    }
                });
            });
        }
    });
    for (k, edit) in &radii {
        apply_edits(app, edit, EditPath::Scope, |app, v: &Option<f64>, _| {
            // A negative radius typed in rounds nothing: the command takes 0 or more.
            if let Some(v) = v {
                set_rectangle_corners(app, json!({"radius": v.max(0.0)}), Some(*k));
            }
        });
    }
    for (k, kind) in kinds {
        set_rectangle_kinds(app, kind, Some(k));
    }
}

/// [`corner_radius_row`]'s field: [`corner_radius`], which a new value sets on those corners.
pub(crate) fn corner_radius_field(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let edit = NumberField::length(tl!("Corner Radius"), Bind::given(corner_radius(app, n)), general_lengths(app)).show(ui).edit;
    apply_edits(app, &edit, EditPath::Scope, |app, v: &Option<f64>, _| {
        if let Some(r) = v {
            app.run("object.setLiveShape", json!({"radius": r.max(0.0)})).ok();
        }
    });
}

/// The icon the command catalog gives command `id`: the Align panel's distribute and Align To
/// buttons' (its align buttons' as the Control bar draws them, [`align::bar_glyph`]) and the
/// Pathfinder panel's, so a bar or menu row of the command shows the panel's glyph, and each
/// tool's, which the tool rail and its flyouts show. `None` for every other command.
pub(crate) fn command_icon(id: &str) -> Option<&'static str> {
    use align::{AlignTo, Button};
    if let Some(tool) = id.strip_prefix("tool:") {
        return vectorcraft_tools::tool_info(tool).map(|t| crate::icons::tool_icon(t.icon));
    }
    if let Some(op) = id.strip_prefix("object.pathfinder.") {
        return pathfinder::SHAPE_MODES.iter().chain(&pathfinder::PATHFINDERS).find(|(_, _, o)| *o == op).map(|(icon, _, _)| *icon);
    }
    if let Some(to) = align::align_to_command(id) {
        return Some(match to {
            AlignTo::Selection => "dc-alignto-selection",
            AlignTo::Key => "dc-alignto-key",
            AlignTo::Artboard => "dc-alignto-artboard",
        });
    }
    let in_table = |table: &[(&'static str, &str, &str, &str)], axis: &str, v: &str| table.iter().find(|r| r.2 == axis && r.3 == v).map(|r| r.0);
    match align::button(id)? {
        b @ Button::Align(..) => align::bar_glyph(b),
        Button::AlignCenters => Some("dc-al-center"),
        Button::Distribute(axis, v) => in_table(&align::DISTRIBUTE, axis, v),
        Button::Spacing("vertical") => Some("dc-dist-vspace"),
        Button::Spacing(_) => Some("dc-dist-hspace"),
    }
}

/// Number of selected objects.
pub(crate) fn selection_len(app: &VectorcraftApp) -> usize {
    app.session.active().map(|d| d.selection.len()).unwrap_or(0)
}

/// Whether the panel part `key` is hidden by its panel's menu.
pub(crate) fn part_hidden(app: &VectorcraftApp, key: &str) -> bool {
    app.ui.hidden_parts.contains(key)
}

/// Hides or shows the panel part `key`.
pub(crate) fn set_part_hidden(app: &mut VectorcraftApp, key: &'static str, on: bool) {
    if on {
        app.ui.hidden_parts.insert(key);
    } else {
        app.ui.hidden_parts.remove(key);
    }
}

pub fn show_icon_panel(app: &mut VectorcraftApp, ui: &mut Ui, id: &str) {
    match id {
        "swatches" => swatches::show(app, ui),
        "color" => color::show(app, ui),
        "colorGuide" => color_guide::show(app, ui),
        "stroke" => stroke::show(app, ui),
        "transparency" => transparency::show(app, ui),
        "appearance" => appearance::show(app, ui),
        "graphicStyles" => graphic_styles::show(app, ui),
        "align" => align::show(app, ui),
        "pathfinder" => pathfinder::show(app, ui),
        blend::ID => blend::show(app, ui),
        "transform" => transform::show(app, ui),
        "history" => history::show(app, ui),
        "actions" => actions::show(app, ui),
        "info" => info::show(app, ui),
        "separations" => separations::show(app, ui),
        "artboards" => artboards::show(app, ui),
        "gradient" => gradient::show(app, ui),
        "character" => character::show(app, ui),
        "paragraph" => paragraph::show(app, ui),
        "glyphs" => glyphs::show(app, ui),
        "navigator" => navigator::show(app, ui),
        "brushes" => brushes::show(app, ui),
        "symbols" => symbols::show(app, ui),
        "patternOptions" => pattern_options::show(app, ui),
        "imageTrace" => image_trace::show(app, ui),
        "openType" => opentype::show(app, ui),
        "docInfo" => doc_info::show(app, ui),
        "charStyles" => text_styles::show(app, ui, text_styles::Kind::Char),
        "paraStyles" => text_styles::show(app, ui, text_styles::Kind::Para),
        "magicWand" => magic_wand::show(app, ui),
        "tabs" => tabs::show(app, ui),
        flattener_preview::ID => flattener_preview::show(app, ui),
        "attributes" => attributes::show(app, ui),
        svg_interactivity::ID => svg_interactivity::show(app, ui),
        variables::ID => variables::show(app, ui),
        "colorThemes" => color_themes::show(app, ui),
        color_wheel::ID => color_wheel::show(app, ui),
        links::ID => links::show(app, ui),
        preflight::ID => preflight::show(app, ui),
        asset_export::ID => asset_export::show(app, ui),
        css_properties::ID => css_properties::show(app, ui),
        three_d::ID => three_d::show(app, ui),
        script_console::ID => script_console::show(app, ui),
        learn::ID => learn::show(app, ui),
        discover::ID => discover::show(app, ui),
        retype::ID => retype::show(app, ui),
        crate::addons::cleanup::ID => crate::addons::cleanup::show(app, ui),
        crate::addons::color::ID => crate::addons::color::show(app, ui),
        crate::addons::packaging::ID => crate::addons::packaging::show(app, ui),
        crate::addons::packaging::preview::ID => crate::addons::packaging::preview::show(app, ui),
        crate::addons::pathtools::ID => crate::addons::pathtools::show(app, ui),
        crate::addons::pathtools::shape::ID => crate::addons::pathtools::shape::show(app, ui),
        crate::addons::pen::ID => crate::addons::pen::show(app, ui),
        crate::addons::units::ID => crate::addons::units::show(app, ui),
        id if library_panel::parse(id).is_some() => library_panel::show(app, ui, id),
        _ => {
            Text::new(tl!("This panel is on the roadmap (see the parity plan).")).shade(Shade::Muted).show(ui);
        }
    }
}

/// Carries out one of a panel menu's commands; `None` for a command the panel does not run itself.
type PanelRun = fn(&mut VectorcraftApp, &str, &Value) -> Option<Result<Value, String>>;

/// A panel's menu: its rows, the states of its commands and the commands it runs itself, all from
/// the panel's module.
struct PanelMenuSource {
    id: &'static str,
    rows: fn(&VectorcraftApp) -> Vec<PanelRow>,
    states: fn(&VectorcraftApp, &mut CommandStates),
    run: PanelRun,
}

/// For a panel whose menu rows are commands that run elsewhere (engine, UI or add-on commands, or
/// the framework's own rows).
fn runs_none(_: &mut VectorcraftApp, _: &str, _: &Value) -> Option<Result<Value, String>> {
    None
}

/// Lists a panel's menu from its module's `menu_rows`, `menu_states` and `run`; a module that serves
/// two panels takes the panel's kind.
macro_rules! menu {
    ($id:expr, $module:ident) => {
        menu!($id, $module, run: $module::run)
    };
    ($id:expr, $module:ident, run: $run:expr) => {
        PanelMenuSource { id: $id, rows: $module::menu_rows, states: $module::menu_states, run: $run }
    };
    ($id:expr, $module:ident, $kind:expr) => {
        PanelMenuSource {
            id: $id,
            rows: |app| $module::menu_rows(app, $kind),
            states: |app, states| $module::menu_states(app, $kind, states),
            run: runs_none,
        }
    };
}

/// Every panel with a menu of its own.
static PANEL_MENUS: &[PanelMenuSource] = &[
    menu!("swatches", swatches),
    menu!("color", color),
    PanelMenuSource {
        id: "colorGuide",
        rows: |_| color_guide::menu_rows(),
        states: |app, states| {
            for (command, state) in color_guide::menu_states(app) {
                states.set(command, state);
            }
        },
        run: |app, id, _| {
            let command = HarmonyCommand::from_ref(&CommandRef::new(id))?;
            color_guide::run(app, command);
            Some(Ok(Value::Null))
        },
    },
    menu!("stroke", stroke),
    menu!("transparency", transparency),
    menu!("appearance", appearance),
    menu!("graphicStyles", graphic_styles, run: |app, id, p| id.starts_with("ui.graphicStyles.").then(|| graphic_styles::run(app, id, p))),
    menu!("align", align),
    menu!("pathfinder", pathfinder),
    menu!("transform", transform),
    menu!("history", history),
    menu!("info", info),
    menu!("separations", separations),
    menu!("artboards", artboards),
    menu!("gradient", gradient),
    menu!("character", character),
    menu!("paragraph", paragraph),
    menu!("glyphs", glyphs, run: |app, id, p| id.starts_with("ui.glyphs.").then(|| glyphs::run(app, id, p).map(|()| Value::Null))),
    menu!("navigator", navigator, run: runs_none),
    menu!("brushes", brushes),
    menu!("symbols", symbols),
    menu!("patternOptions", pattern_options),
    menu!("imageTrace", image_trace),
    menu!("openType", opentype),
    menu!("docInfo", doc_info),
    menu!("charStyles", text_styles, text_styles::Kind::Char),
    menu!("paraStyles", text_styles, text_styles::Kind::Para),
    menu!("magicWand", magic_wand),
    menu!("tabs", tabs),
    menu!("libraries", libraries),
    menu!(flattener_preview::ID, flattener_preview),
    menu!("attributes", attributes),
    menu!(svg_interactivity::ID, svg_interactivity),
    menu!("colorThemes", color_themes),
    menu!(color_wheel::ID, color_wheel),
    menu!("layers", layers),
    menu!(links::ID, links),
    menu!(asset_export::ID, asset_export),
    menu!(css_properties::ID, css_properties),
    menu!(variables::ID, variables),
    menu!(script_console::ID, script_console),
    menu!(retype::ID, retype, run: retype::command),
    menu!("actions", actions_menu),
    menu!(crate::addons::packaging::ID, packaging_menu, run: runs_none),
];

use crate::addons::packaging as packaging_menu;
use actions::menu as actions_menu;

/// The rows of panel `id`'s (≡) menu; none for a panel without a menu of its own.
pub(crate) fn panel_rows(app: &VectorcraftApp, id: &str) -> Vec<PanelRow> {
    if library_panel::parse(id).is_some() {
        return library_panel::menu_rows(app, id);
    }
    PANEL_MENUS.iter().find(|m| m.id == id).map_or_else(Vec::new, |m| (m.rows)(app))
}

/// Sets the states of every panel menu's commands, the open library panels' included, and of the
/// Contextual Task Bar's.
pub(crate) fn panel_menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    for menu in PANEL_MENUS {
        (menu.states)(app, states);
    }
    let layout = crate::panel_docking::layout_of(&app.ui);
    for panel in app.ui.library_panels.iter().map(|l| library_panel::panel_id(&l.kind, &l.id)).filter(|panel| layout.contains(panel)) {
        library_panel::menu_states(app, &panel, states);
    }
    crate::canvas::task_bar::menu_states(app, states);
}

/// Runs panel menu command `id` when a panel runs it itself; `None` otherwise.
pub(crate) fn run_menu_command(app: &mut VectorcraftApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if id == "ui.libraryPanel.add" {
        return Some(library_panel::run(app, id, params));
    }
    PANEL_MENUS.iter().find_map(|menu| (menu.run)(app, id, params))
}

pub fn libraries(app: &mut VectorcraftApp, ui: &mut Ui) {
    libraries::show(app, ui);
}

// ---------- shared helpers ----------

/// The paint command of the active proxy (Fill or Stroke), or of the inactive one.
pub(crate) fn proxy_cmd(app: &VectorcraftApp, inactive: bool) -> &'static str {
    if app.session.fill_active != inactive { "paint.setFill" } else { "paint.setStroke" }
}

/// The first selected node, borrowed (for per-frame reads that need no copy of it).
fn first_node(app: &VectorcraftApp) -> Option<&Node> {
    let st = app.session.active()?;
    st.selection.objects.first().and_then(|id| st.doc.node(*id))
}

/// Is Alt held? A click on a colour then paints the inactive proxy.
pub(crate) fn alt_held(ui: &Ui) -> bool {
    ui.input(|i| i.modifiers.alt)
}

/// Apply a clicked colour, swatch or None (`params`: `{color}`, `{swatch}` or `{none}`) to the
/// active proxy, or with Alt held to the inactive one, which stays behind.
pub(crate) fn apply_click(app: &mut VectorcraftApp, ui: &Ui, mut params: Value) {
    let alt = alt_held(ui);
    params["focus"] = json!(!alt);
    app.run(proxy_cmd(app, alt), params).ok();
}

/// Fill and stroke as the proxies show them ([`vectorcraft_engine::Session::proxy_paints`]: the
/// Appearance panel's active item for the proxy of its kind, else the first selected object's, a
/// group's first painted object's, else the defaults for new art).
pub(crate) fn current_paints(app: &VectorcraftApp) -> (Paint, Paint) {
    app.session.proxy_paints()
}

/// Whether the selected objects' fills and strokes differ (the proxies show "?"), cached per
/// document revision, Appearance panel item and picked gradient mesh points (picking points
/// changes no revision).
pub(crate) fn mixed_paints(app: &VectorcraftApp, ctx: &egui::Context) -> (bool, bool) {
    type Key = (u64, u64, Option<usize>, Vec<(NodeId, Vec<usize>)>, bool);
    let Some(st) = app.session.active() else { return (false, false) };
    // Whether the proxies show the Live Paint Bucket's colors too.
    let key: Key = (st.uid, st.revision, app.session.appearance_item(), app.session.fill_mesh_points(), app.session.bucket_paints());
    match pstate::<Option<(Key, (bool, bool))>>(ctx, "proxy-mixed") {
        Some((k, mixed)) if k == key => mixed,
        _ => {
            let mixed = app.session.proxy_mixed();
            set_pstate(ctx, "proxy-mixed", Some((key, mixed)));
            mixed
        }
    }
}

/// The stroke the Stroke panel, Control bar and Properties show: while the Type tool edits text,
/// the selected characters' stroke, else [`vectorcraft_engine::Session::shown_stroke`].
pub(crate) fn current_stroke(app: &VectorcraftApp) -> Option<StrokeLayer> {
    if character::text_editing(app).is_some() {
        return character::text_style(app).map(|(c, _)| c.stroke_layer());
    }
    app.session.shown_stroke()
}

/// Which Stroke panel values the selection doesn't share and whether Align Stroke applies
/// ([`vectorcraft_engine::DocState::stroke_mixed`]), cached per document revision. While the Type
/// tool edits text, the selected characters' stroke shows as it is, and aligns to their glyphs.
/// While the Live Paint Bucket is active, the panel shows the one stroke it paints with, as it does
/// with nothing selected.
pub(crate) fn stroke_mixed(app: &VectorcraftApp, ctx: &egui::Context) -> StrokeMixed {
    if character::text_editing(app).is_some() {
        return StrokeMixed { can_align: true, ..Default::default() };
    }
    if app.session.bucket_paints() {
        return StrokeMixed { can_align: true, ..Default::default() };
    }
    let Some(st) = app.session.active() else { return StrokeMixed::default() };
    let key = (st.uid, st.revision);
    match pstate::<Option<((u64, u64), StrokeMixed)>>(ctx, "stroke-mixed") {
        Some((k, mixed)) if k == key => mixed,
        _ => {
            let mixed = st.stroke_mixed();
            set_pstate(ctx, "stroke-mixed", Some((key, mixed)));
            mixed
        }
    }
}

/// Opacity and blend mode as the Transparency panel and Control bar show them: the Appearance
/// panel's active item's, else the first picked gradient mesh point's opacity (which
/// `transparency.set` changes then) with its object's blend mode, else the first selected object's.
pub(crate) fn current_transparency(app: &VectorcraftApp) -> Option<(f32, BlendMode)> {
    let n = first_node(app)?;
    if let Some(it) = app.session.appearance_item().and_then(|i| n.appearance.items.get(i)) {
        return Some((it.opacity(), it.blend()));
    }
    let picked = app.session.mesh_points_picked().and_then(|points| {
        let (id, v) = points.first()?;
        let d = &app.session.active()?.doc;
        let m = match &d.node(*id)?.kind {
            NodeKind::Mesh(m) => m,
            _ => return None,
        };
        Some((m.points.get(*v.first()?)?.opacity, d.node(*id)?.blend))
    });
    Some(picked.unwrap_or((n.opacity, n.blend)))
}

/// Is the active proxy "?" (the selected objects' paints differ)?
pub(crate) fn active_mixed(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    let (f, s) = mixed_paints(app, ctx);
    if app.session.fill_active { f } else { s }
}

/// The paint behind the active proxy.
pub(crate) fn active_paint(app: &VectorcraftApp) -> Paint {
    let (f, s) = current_paints(app);
    if app.session.fill_active { f } else { s }
}

/// The Fill/Stroke proxy: fill and stroke wells (the stroke a ring), the active one in front, with
/// the Swap and Default buttons. Its clicks run commands ([`proxy_events`]); a paint dropped on a
/// well that is not "?" paints that proxy.
pub(crate) fn proxy(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (fill, stroke) = current_paints(app);
    let mixed = mixed_paints(app, ui.ctx());
    let well = |label: &'static str, paint: &Paint, mixed: bool| {
        PairWell::paint(label, if mixed { Mixed::Mixed } else { Mixed::Same(well_paint(paint, &|_| None)) })
    };
    let wells = [well(tl!("Fill"), &fill, mixed.0), well(tl!("Stroke"), &stroke, mixed.1).ring(true)];
    let active = if app.session.fill_active { Well::Primary } else { Well::Secondary };
    let out = SwatchPair::new(tl!("Fill and Stroke"), wells, Bind::given(active))
        .swap_button(tl!("Swap Fill and Stroke"))
        .default_button(tl!("Default Fill and Stroke"))
        .popover(false)
        .drags([Some(PanelDrag::paint(fill)), Some(PanelDrag::paint(stroke))])
        .accepts(move |well, drag| takes_drop(mixed, well, drag))
        .show(ui);
    proxy_events(app, &out);
    if let Some((well, drag)) = &out.dropped {
        drop_paint(app, *well, drag);
    }
}

/// Runs what a Fill/Stroke proxy reported this pass ([`proxy_commands`]): this proxy's, or the
/// Color panel's, whose wells are the same two.
pub(crate) fn proxy_events<P>(app: &mut VectorcraftApp, out: &SwatchPairOutput<P>) {
    for (cmd, params) in proxy_commands(out.activated.then_some(out.active), out.double_clicked, out.swap, out.defaults) {
        app.run(cmd, params).ok();
    }
}

/// The commands a proxy's events run, in order: the other well made active brings that proxy
/// forward, Swap and Default, and a double click opens the Color Picker on that proxy's paint.
fn proxy_commands(activated: Option<Well>, double_clicked: Option<Well>, swap: bool, defaults: bool) -> Vec<(&'static str, Value)> {
    let mut commands = Vec::new();
    if let Some(well) = activated {
        commands.push(("paint.toggleActive", json!({ "fill": well == Well::Primary })));
    }
    if swap {
        commands.push(("paint.swap", json!({})));
    }
    if defaults {
        commands.push(("paint.default", json!({})));
    }
    if let Some(well) = double_clicked {
        commands.push(("ui.colorPicker", json!({ "stroke": well == Well::Secondary })));
    }
    commands
}

/// Whether a proxy well takes `drag`: a paint that paints something, dropped on a well that is not
/// "?" (`mixed`: the fill's and the stroke's).
fn takes_drop(mixed: (bool, bool), well: Well, drag: &PanelDrag) -> bool {
    let mixed = if well == Well::Primary { mixed.0 } else { mixed.1 };
    !mixed && matches!(drag, PanelDrag::Paint { params, .. } if !params.is_null())
}

/// Paints the fill (the primary well) or the stroke with the paint `drag` carries.
fn drop_paint(app: &mut VectorcraftApp, well: Well, drag: &PanelDrag) {
    if let PanelDrag::Paint { params, .. } = drag {
        let cmd = if well == Well::Primary { "paint.setFill" } else { "paint.setStroke" };
        app.run(cmd, params.clone()).ok();
    }
}

/// The open Fill/Stroke chip popover: the chip it hangs from and whether it shows the Color
/// panel's mixer instead of the swatches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaintPopover {
    /// The response id of the chip that opened it.
    chip: egui::Id,
    /// The mixer shows, not the swatches.
    mixer: bool,
}

/// The Fill (or `stroke`) chip of the Control bar and the Properties panel: the proxy's paint, "?"
/// when the selected objects' paints differ, the stroke as a ring. A click brings that proxy
/// forward and opens a popover with the Swatches panel; Shift-click (or the popover's toggle)
/// shows the Color panel's mixer instead.
pub(crate) fn paint_chip(app: &mut VectorcraftApp, ui: &mut Ui, stroke: bool) {
    let (fill, stroke_paint) = current_paints(app);
    let mixed = mixed_paints(app, ui.ctx());
    let (paint, mixed, label, tip) = if stroke {
        (stroke_paint, mixed.1, tl!("Stroke"), tl!("Stroke: click for swatches, Shift-click for the color mixer"))
    } else {
        (fill, mixed.0, tl!("Fill"), tl!("Fill: click for swatches, Shift-click for the color mixer"))
    };
    let shown = if mixed { Mixed::Mixed } else { Mixed::Same(well_paint(&paint, &|_| None)) };
    let fallback = srgba(&paint.color().unwrap_or(Color::BLACK));
    let out = PaintChip::new(label, Bind::given(shown), fallback).popover(false).ring(stroke).tooltip(tip).show(ui);
    let chip = out.edit.response.id;
    if let Some(mods) = out.clicked {
        app.run("paint.toggleActive", json!({ "fill": !stroke })).ok();
        app.ui.paint_popover = Some(PaintPopover { chip, mixer: mods.shift });
    }
    let mut open = app.ui.paint_popover.is_some_and(|p| p.chip == chip);
    if !open {
        return;
    }
    // The popover keeps its own open state: the Swatches body opens menus of its own (Swatch
    // Libraries, Show Swatch Kinds), which close with it (#536).
    // The Swatches body is one collection that scrolls inside the popover's room; the mixer scrolls with the popover.
    let popover = Popover::new(label, Bind::bound(&mut open)).opened_by(&out.edit.response);
    let popover = if app.ui.paint_popover.is_some_and(|p| p.mixer) { popover } else { popover.collection_body() };
    let _ = popover.show(ui, |ui, _| paint_popover(app, ui));
    if !open {
        app.ui.paint_popover = None;
    }
}

/// The body of a Fill/Stroke chip popover: a Swatches / Color Mixer toggle above that panel's body.
/// The mixer is the Color panel, so its menu comes along: the color modes (Grayscale, RGB, HSB,
/// CMYK, Web Safe RGB), Invert, Complement and the rest (#4249).
fn paint_popover(app: &mut VectorcraftApp, ui: &mut Ui) {
    let mixer = app.ui.paint_popover.is_some_and(|p| p.mixer);
    let mut picked = None;
    Row::new().show(ui, |ui, _| {
        for (icon, tip, m) in [("swatch-book", tl!("Swatches"), false), ("palette", tl!("Color Mixer (Shift-click the chip)"), true)] {
            if IconButton::new(IconId::new(icon)).label(tip).tooltip(tip).selected(mixer == m).show(ui).clicked() {
                picked = Some(m);
            }
        }
        if mixer {
            let button = IconButton::new(IconId::new("menu")).label(tl!("Panel menu")).popup(PopupKind::Menu).show(ui).response;
            Menu::new(tl!("Panel menu")).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
                PanelMenu::new(color::menu_rows(app), Vec::new()).show(ui);
            });
        }
    });
    if let (Some(m), Some(open)) = (picked, &mut app.ui.paint_popover) {
        open.mixer = m;
    }
    Divider::horizontal().show(ui);
    if mixer {
        color::show(app, ui);
    } else {
        swatches::popover(app, ui);
    }
}

/// Where a live control's gesture stands this pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Live {
    /// The value is being dragged.
    Dragging,
    /// The drag ended this pass.
    Released,
}

/// Run `cmd` live: while dragging, preview on top of an interaction snapshot; on release, commit
/// it as one undo step. Falls back to a plain run when no document is open.
pub(crate) fn live_run(app: &mut VectorcraftApp, label: &str, cmd: &str, params: Value, phase: Live) {
    // The canvas press that took the field's focus committed this gesture already (`committed_preview`).
    if phase == Live::Released && std::mem::take(&mut app.canvas.committed_preview) {
        return;
    }
    if app.begin_interaction(label).is_ok() {
        // As the scrub's undo step was, the gesture's previews stay in their document when another becomes active.
        if let Some(it) = app.session.active_mut().and_then(|st| st.interaction.as_mut()) {
            it.keep_on_switch = true;
        }
        if let Err(e) = app.session.preview(cmd, &params) {
            app.ui.status = e.to_string();
        }
        if phase == Live::Released
            && let Err(e) = app.session.commit_interaction()
        {
            app.ui.status = e.to_string();
        }
        app.sync_views();
    } else if phase == Live::Released {
        app.run(cmd, params).ok();
    }
}

/// Per-panel UI state kept in egui memory (not document state).
pub(crate) fn pstate<T: Clone + Default + Send + Sync + 'static>(ctx: &egui::Context, key: &str) -> T {
    ctx.data(|d| d.get_temp::<T>(egui::Id::unique(("panel-state", key)))).unwrap_or_default()
}
/// Store per-panel UI state in egui temp memory under the `panel-state` namespace.
pub(crate) fn set_pstate<T: Clone + Send + Sync + 'static>(ctx: &egui::Context, key: &str, v: T) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("panel-state", key)), v));
}

/// `ui.panelView {panel, view?}`: set the view of the Swatches, Graphic Styles or Symbols panel
/// (`panel`: swatches, graphicStyles or symbols), as the view items of its panel menu do; without
/// `view`, answer the current one. An unknown panel or view is an error, and then nothing changes.
/// → {panel, view}
pub(crate) fn set_view(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    fn given<T: serde::de::DeserializeOwned>(p: &Value, err: &str) -> Result<Option<T>, String> {
        match p.get("view") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|_| err.to_string()),
        }
    }
    let ui = &mut app.ui;
    let (panel, view) = match p.get("panel").and_then(Value::as_str) {
        Some("swatches") => {
            if let Some(v) = given(p, "`view` is smallThumb, mediumThumb, largeThumb, smallList or largeList")? {
                ui.swatches_view = v;
            }
            ("swatches", json!(ui.swatches_view))
        }
        Some("graphicStyles") => {
            if let Some(v) = given(p, "`view` is thumbnail, smallList or largeList")? {
                ui.graphic_styles_view = v;
            }
            ("graphicStyles", json!(ui.graphic_styles_view))
        }
        Some("symbols") => {
            if let Some(v) = given(p, "`view` is thumbnail, smallList or largeList")? {
                ui.symbols_view = v;
            }
            ("symbols", json!(ui.symbols_view))
        }
        _ => return Err("`panel` is swatches, graphicStyles or symbols".to_string()),
    };
    Ok(json!({ "panel": panel, "view": view }))
}

/// [`crate::i18n::label_or_name`] in the UI language.
pub(crate) fn label_or_name(s: &str, built_in: bool) -> &str {
    crate::i18n::label_or_name(crate::i18n::current(), s, built_in)
}

/// The Session's recent colors (which every paint command feeds) as a wrapping palette named
/// "Recent Colors"; returns the one clicked, for the caller to apply. A cell drags its color as a
/// solid paint, which the Swatches panel makes a swatch of.
pub(crate) fn recent_colors(app: &VectorcraftApp, ui: &mut Ui) -> Option<Color> {
    let stored: Vec<Color> = app.session.recent_colors.to_vec();
    let cells: Vec<(Label, Srgba)> = stored.iter().map(|c| (Label::from(c.to_hex()), srgba(c))).collect();
    // A colour no cell holds, so no cell shows as picked and every click is a pick.
    let mut picked = Srgba::BLACK.with_alpha(0.0);
    let dragged = stored.clone();
    let out = SwatchPalette::new(tl!("Recent Colors"), Bind::bound(&mut picked), &cells)
        .spacing(SwatchSpacing::Spaced)
        .drag_payload(move |drag: &ListDrag<usize>| dragged.get(drag.grabbed).map(|c| PanelDrag::paint(Paint::solid(*c))))
        .show(ui);
    // The palette reports the cell's sRGB; the stored colour keeps its own model.
    let rgba = out.edit?.value()?.to_rgba();
    stored.into_iter().zip(&cells).find(|(_, (_, cell))| cell.to_rgba() == rgba).map(|(c, _)| c)
}

/// A colour as command JSON, keeping its model.
pub(crate) use vectorcraft_tools::params::color_json;

/// Paint as command params (`{color}`, `{swatch, tint?}`, `{none}`, `{gradient}` (lossless) or
/// `{pattern}`).
pub(crate) fn paint_params(p: &Paint) -> Value {
    match p {
        Paint::None => json!({"none": true}),
        Paint::Solid { swatch: Some(n), tint, .. } if *tint < 1.0 => json!({"swatch": n, "tint": tint * 100.0}),
        Paint::Solid { swatch: Some(n), .. } => json!({"swatch": n}),
        Paint::Solid { color, .. } => json!({"color": color_json(color)}),
        Paint::Gradient(g) => json!({"gradient": vectorcraft_tools::params::gradient_params(g)}),
        Paint::Pattern { pattern, .. } => json!({"pattern": pattern}),
    }
}

/// [`paint_params`] for a swatch made from paint the app shows: a color and a gradient's stops
/// and points keep their color models (`keepModel`), in a CMYK document too.
pub(crate) fn swatch_paint_params(p: &Paint) -> Value {
    let mut v = paint_params(p);
    if matches!(p, Paint::Solid { swatch: None, .. } | Paint::Gradient(_)) {
        v["keepModel"] = json!(true);
    }
    v
}

/// Egui colour of a document colour.
pub(crate) fn c32(c: &Color) -> egui::Color32 {
    let [r, g, b, _] = c.to_rgba8(1.0);
    egui::Color32::from_rgb(r, g, b)
}

/// A document colour as the framework's colour type: its sRGB, opaque.
pub(crate) fn srgba(c: &Color) -> Srgba {
    let [r, g, b, _] = c.to_rgba8(1.0);
    Srgba::rgb(f32::from(r) / 255.0, f32::from(g) / 255.0, f32::from(b) / 255.0)
}

/// A colour channel row's rule over `range` in whole steps, as the alpha row steps.
pub(crate) fn whole_steps(range: std::ops::RangeInclusive<f64>) -> ChannelRule {
    let mut rule = ChannelRule::ALPHA;
    rule.range = range;
    rule
}

/// A section of a panel or dialog under its heading in label text: the reference app's section headings are regular
/// weight, not bold.
pub(crate) fn section<'a>(title: impl Into<Label>) -> Section<'a> {
    Section::fixed(title).strength(HeaderStrength::Normal)
}

/// The empty state of a panel with nothing to show: `icon` over the title and the message,
/// centred in the panel.
pub(crate) fn empty_state(ui: &mut Ui, icon: &'static str, title: &str, body: &str) {
    Notice::new(Tone::Neutral, body.to_owned())
        .layout(NoticeLayout::Centred)
        .title(title.to_owned())
        .icon(NoticeIcon::Custom(IconSpec::svg(IconId::new(icon))))
        .show(ui);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_events_run_their_commands() {
        let ran = |activated, double, swap, defaults| -> Vec<(&'static str, Value)> { proxy_commands(activated, double, swap, defaults) };
        assert!(ran(None, None, false, false).is_empty(), "a pass with no event runs nothing");
        assert_eq!(ran(Some(Well::Primary), None, false, false), vec![("paint.toggleActive", json!({"fill": true}))]);
        assert_eq!(ran(Some(Well::Secondary), None, false, false), vec![("paint.toggleActive", json!({"fill": false}))]);
        assert_eq!(ran(None, None, true, false), vec![("paint.swap", json!({}))]);
        assert_eq!(ran(None, None, false, true), vec![("paint.default", json!({}))]);
        assert_eq!(ran(None, Some(Well::Secondary), false, false), vec![("ui.colorPicker", json!({"stroke": true}))]);
        assert_eq!(ran(None, Some(Well::Primary), false, false), vec![("ui.colorPicker", json!({"stroke": false}))]);

        let mut app = crate::VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
        let green = PanelDrag::paint(Paint::solid(Color::from_hex("#00ff00").unwrap()));
        assert!(takes_drop((false, false), Well::Secondary, &green));
        assert!(!takes_drop((false, true), Well::Secondary, &green), "a mixed well takes no drop");
        assert!(takes_drop((false, true), Well::Primary, &green), "the other well still does");
        drop_paint(&mut app, Well::Secondary, &green);
        let (fill, stroke) = current_paints(&app);
        assert_eq!(stroke.color().map(|c| c.to_hex()), Some("#00ff00".into()), "{}", app.ui.status);
        assert_eq!(fill.color().map(|c| c.to_hex()), Some("#ff0000".into()), "a drop on the stroke leaves the fill");
        drop_paint(&mut app, Well::Primary, &green);
        assert_eq!(current_paints(&app).0.color().map(|c| c.to_hex()), Some("#00ff00".into()));
    }

    #[test]
    fn paint_params_roundtrip_shapes() {
        assert_eq!(paint_params(&Paint::None), json!({"none": true}));
        let p = paint_params(&Paint::solid(Color::cmyk(0.1, 0.2, 0.3, 0.4)));
        assert!((p["color"]["k"].as_f64().unwrap() - 0.4).abs() < 1e-6);
        let g = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(Default::default())));
        let p = paint_params(&g);
        assert_eq!(p["gradient"]["kind"], "linear");
        assert_eq!(p["gradient"]["stops"].as_array().unwrap().len(), 2);
        assert_eq!(paint_params(&vectorcraft_doc::pattern::pattern_paint("Dots")), json!({"pattern": "Dots"}));
        assert_eq!(p["gradient"]["stops"][0]["midpoint"], json!(0.5));
        let tint = Paint::Solid { color: Color::gray(0.5), swatch: Some("Ink".into()), tint: 0.5 };
        assert_eq!(paint_params(&tint), json!({"swatch": "Ink", "tint": 50.0}), "a tint keeps its percentage");
    }
}
#[cfg(test)]
mod tests_appearance;
#[cfg(test)]
mod tests_asset_export;
#[cfg(test)]
mod tests_blend;
#[cfg(test)]
mod tests_brush_libraries;
#[cfg(test)]
mod tests_color_wheel;
#[cfg(test)]
mod tests_color_wheel_box;
#[cfg(test)]
mod tests_color_wheel_document_colors;
#[cfg(test)]
mod tests_color_wheel_gamut;
#[cfg(test)]
mod tests_color_wheel_harmony;
#[cfg(test)]
mod tests_color_wheel_vision;
#[cfg(test)]
mod tests_constrain;
#[cfg(test)]
mod tests_css_properties;
#[cfg(test)]
mod tests_effectedit;
#[cfg(test)]
mod tests_freeform;
#[cfg(test)]
mod tests_links;
#[cfg(test)]
mod tests_maskview;
#[cfg(test)]
mod tests_script_console;
#[cfg(test)]
mod tests_stroke;
#[cfg(test)]
mod tests_strokedepth;
#[cfg(test)]
mod tests_strokeux;
#[cfg(test)]
mod tests_style_libraries;
#[cfg(test)]
mod tests_symbol_libraries;
#[cfg(test)]
mod tests_units;
