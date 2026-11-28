//! The Tools panel. Default: Illustrator 2026's categorized single-column toolbar (Select, Shapes,
//! Draw, Modify, Type, Navigate, Color). Window → Toolbars → Advanced shows every tool group; the
//! double arrow at the top (or Window → Toolbars → Double Column, `window.toolbarColumns`) lays the
//! tools out in one or two columns.
//! Bottom: fill/stroke proxy, colour/gradient/none, drawing modes, screen mode, Search and Edit
//! Toolbar. Edit opens the native All Tools drawer (`edit_toolbar`).
//!
//! The dock draws the rail ([`rail_view`]) for the Tools panel, for each toolbar made with New
//! Toolbar and for each group torn off either of them (a `strip:` panel).

use craft::behavior::dock::{Action, DockEdge};
use craft::behavior::geometry::BoxState;
use craft::behavior::overlay::Side;
use craft::behavior::{EditEvent, IconId, Mixed};
use craft::color_ui::ui::{PaintChip, PaintChipSize};
use craft::draw::ui::transform::{SizeUnit, TransformField, TransformFields, TransformGroups, TransformRow, TransformSource};
use craft::theme::ControlSize;
use craft::ui::buttons::{ActiveLook, Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, BodyCx, Entry, Gap, Props as _, Show as _, ShowContainer as _, Tracked};
use craft::ui::display::Divider;
use craft::ui::docking::{DockGrid, GroupAt};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::pickers::{Anchor9, ReferencePointLook, Select};
use craft::ui::rails::{LastUsed, RailDensity, RailFooter, RailTool, SlotId, ToolRail, ToolSlot};
use craft::ui::runtime::commands::{self, CommandRef};
use craft::ui::text::NumberField;
use egui::{Pos2, Ui, Vec2};
use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, TOOL_GROUPS, ToolInfo, ToolKey, tool_info};

use crate::VectorcraftApp;
use crate::paint_preview::well_paint;
use crate::panels::learn::locate::{self, Home};
use crate::state::{CustomToolbar, UiState};

/// The Tools panel's key in the dock layout.
pub(crate) const TOOLS: &str = "tools";

/// What a rail draws the tools of.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    /// The Tools panel.
    Tools,
    /// The toolbar made with New Toolbar with this id.
    Toolbar(u32),
    /// The group of the owner whose first tool is the string, torn off as its own panel.
    Strip(Box<Owner>, String),
}

impl Owner {
    /// The part of a `strip:` key that names the toolbar the strip was torn off.
    fn part(&self) -> String {
        match self {
            Owner::Tools => TOOLS.to_string(),
            Owner::Toolbar(id) => id.to_string(),
            Owner::Strip(parent, _) => parent.part(),
        }
    }

    /// The owner's panel key in the dock layout.
    pub(crate) fn panel(&self) -> String {
        match self {
            Owner::Tools => TOOLS.to_string(),
            Owner::Toolbar(id) => format!("toolbar:{id}"),
            Owner::Strip(parent, first) => strip_id(parent, first),
        }
    }
}

/// The panel key of the group of `owner` whose first tool is `first`, torn off.
pub(crate) fn strip_id(owner: &Owner, first: &str) -> String {
    format!("strip:{}:{first}", owner.part())
}

/// Whether the Tools panel is open (Window › Tools): the layout is the one store of it.
pub(crate) fn shown(ui: &UiState) -> bool {
    crate::panel_docking::layout_of(ui).contains(&TOOLS.to_owned())
}

/// Opens (back where it was) or closes the Tools panel; a refusal goes to the status bar.
pub(crate) fn set_shown(app: &mut VectorcraftApp, on: bool) {
    if !on {
        strips_follow(app, &Owner::Tools, false);
    }
    let done = if on {
        crate::panel_docking::reveal_panel(app, TOOLS).map(drop)
    } else {
        let ctx = app.ui_ctx.clone();
        crate::panel_docking::apply(app, ctx.as_ref(), vec![Action::close(TOOLS.to_owned())]).map(drop)
    };
    if let Err(e) = done {
        app.status(e);
    }
    if on {
        strips_follow(app, &Owner::Tools, true);
    }
}

/// The strips torn off the rail `owner` hide with it, and those come back when it shows again.
pub(crate) fn strips_follow(app: &mut VectorcraftApp, owner: &Owner, shown: bool) {
    let rail = owner.panel();
    if shown {
        for strip in app.ui.strips_hidden_with.remove(&rail).unwrap_or_default() {
            // A strip whose group has gone meanwhile stays closed.
            app.run("window.panel.dock", json!({ "panel": strip })).ok();
        }
        return;
    }
    let layout = crate::panel_docking::layout_of(&app.ui);
    let open: Vec<String> =
        groups_of(app, owner).iter().filter_map(|g| g.first()).map(|first| strip_id(owner, first)).filter(|id| layout.contains(id)).collect();
    if open.is_empty() {
        return;
    }
    let ctx = app.ui_ctx.clone();
    match crate::panel_docking::apply(app, ctx.as_ref(), open.iter().cloned().map(Action::close).collect()) {
        Ok(_) => {
            app.ui.strips_hidden_with.insert(rail, open);
        }
        Err(e) => app.status(e),
    }
}

/// The Basic toolbar: (category, slots); each slot is a flyout group (first = default).
pub const BASIC: &[(&str, &[&[&str]])] = &[
    ("Select", &[&["selection"], &["directSelection", "groupSelection"], &["lasso", "enclosingLasso", "magicWand"]]),
    (
        "Shapes",
        &[
            &["rectangle", "roundedRectangle", "star", "lineSegment", "arc", "threePointArc", "spiral", "rectangularGrid", "polarGrid", "flare"],
            &["ellipse", "circle", "curvatureCircle"],
            &["polygon"],
            &["shaper"],
        ],
    ),
    (
        "Draw",
        &[
            &["pencil", "smooth", "pathEraser", "join"],
            &["eraser", "scissors", "knife", "trimAtCrossings", "mirrorCut", "lineCut", "rectCut"],
            &["paintbrush", "blobBrush"],
            &["pen", "addAnchor", "deleteAnchor", "anchorPoint", "quickPen", "smartPen"],
            &["curvature"],
        ],
    ),
    (
        "Modify",
        &[
            &["width", "warp", "twirl", "pucker", "bloat", "scallop", "crystallize", "wrinkle"],
            &["rotate", "reflect", "scale", "shear", "reshape", "freeTransform", "puppetWarp"],
            &["shapeBuilder", "livePaintBucket", "fillArea", "livePaintSelection", "blend"],
            &["dimension"],
        ],
    ),
    ("Type", &[&["areaType", "typeOnPath", "verticalAreaType", "verticalTypeOnPath"], &["type", "verticalType", "touchType"]]),
    ("Navigate", &[&["zoom"], &["hand", "printTiling"], &["rotateView"]]),
    ("Color", &[&["gradient", "mesh"], &["eyedropper", "measure"]]),
];

/// A tool's name with the key that picks it in the window of `ctx`, as its button's tip shows it.
pub(crate) fn tip(ctx: &egui::Context, t: &ToolInfo) -> String {
    match commands::chord(ctx, &CommandRef::new(format!("tool:{}", t.id))) {
        Some(chord) => format!("{} ({})", tl!(t.label), chord.display(crate::keymap::platform())),
        None => tl!(t.label).to_string(),
    }
}

/// Slots of the current layout, as Edit Toolbar left it: (category label for the first slot of a
/// category, tool ids).
pub(crate) fn slots(app: &VectorcraftApp) -> Vec<(Option<&'static str>, Vec<&'static str>)> {
    if let Some(edited) = crate::edit_toolbar::edited_slots(app) {
        edited
    } else if app.ui.toolbar_advanced {
        TOOL_GROUPS.iter().map(|g| (None, g.iter().map(|t| t.id).collect())).collect()
    } else {
        BASIC.iter().flat_map(|(cat, ss)| ss.iter().enumerate().map(move |(i, s)| (if i == 0 { Some(*cat) } else { None }, s.to_vec()))).collect()
    }
}

/// The slot a group of tools is in the rail's last-used memory: the index of its first tool in the
/// catalog, which stays put while the user edits the toolbar.
pub(crate) fn slot_id(first: &str) -> SlotId {
    vectorcraft_tools::catalog::all_tools().position(|t| t.id == first).and_then(|i| SlotId::try_from(i).ok()).unwrap_or(SlotId::MAX)
}

/// A rail tool that selects `id`: the tool's command in the catalog, which gives the button its name, its icon and
/// its shortcut.
pub(crate) fn rail_tool(id: &str) -> RailTool<String> {
    RailTool::new(id.to_string(), crate::keymap::command_of(&format!("tool:{id}")))
}

/// The rail's slots for `slots`: a category's first slot carries its caption, and its last ends the
/// group.
pub(crate) fn rail_slots(slots: &[(Option<&'static str>, Vec<&'static str>)]) -> Vec<ToolSlot<String>> {
    slots
        .iter()
        .enumerate()
        .map(|(i, (cat, tools))| {
            let id = slot_id(tools.first().copied().unwrap_or_default());
            let tools = tools.iter().map(|tool| rail_tool(tool)).collect();
            let ends = slots.get(i + 1).is_none_or(|next| next.0.is_some());
            let slot = if ends { ToolSlot::ending_group(id, tools) } else { ToolSlot::new(id, tools) };
            match cat {
                Some(cat) => slot.captioned(tl!(cat)),
                None => slot,
            }
        })
        .collect()
}

/// The tool a slot's button shows: the active tool when the slot holds it, else the one chosen
/// last from the slot's group, else its first.
pub(crate) fn shown_tool(app: &VectorcraftApp, slot: &[&'static str]) -> &'static str {
    let active = app.session.tool_id();
    if slot.contains(&active) {
        return active;
    }
    let first = slot.first().copied().unwrap_or_default();
    app.ui.tools_last_used.get(slot_id(first)).and_then(|id| slot.iter().copied().find(|s| *s == id.as_str())).unwrap_or(first)
}

/// The toolbar made with New Toolbar with this id.
fn bar_of(ui: &UiState, id: u32) -> Option<&CustomToolbar> {
    ui.custom_toolbars.iter().find(|bar| bar.id == id)
}

/// The owner of the toolbar named `toolbar` (none: the Tools panel's); none when there is no such
/// toolbar.
fn owner_of(ui: &UiState, toolbar: Option<&str>) -> Option<Owner> {
    match toolbar {
        None => Some(Owner::Tools),
        Some(name) => crate::custom_toolbars::find(ui, name).map(|bar| Owner::Toolbar(bar.id)),
    }
}

/// The tool groups of `owner`'s toolbar (a strip's are those of the toolbar it was torn off).
fn groups_of(app: &VectorcraftApp, owner: &Owner) -> Vec<Vec<&'static str>> {
    match owner {
        Owner::Tools => slots(app).into_iter().map(|(_, s)| s).collect(),
        Owner::Toolbar(id) => bar_of(&app.ui, *id)
            .map(|bar| bar.slots.iter().map(|s| s.iter().filter_map(|id| tool_info(id).map(|t| t.id)).collect()).collect())
            .unwrap_or_default(),
        Owner::Strip(parent, _) => groups_of(app, parent),
    }
}

/// The tools of the group of `parent` whose first tool is `first`, a strip's; none once the group has gone.
fn strip_tools(app: &VectorcraftApp, parent: &Owner, first: &str) -> Vec<&'static str> {
    groups_of(app, parent).into_iter().find(|g| g.first().is_some_and(|f| *f == first)).unwrap_or_default()
}

/// The dock grid of the rail panel `owner`: a cell for each tool slot and a custom toolbar's Edit Toolbar button.
/// The Tools panel keeps its actions in the host footer. The rail draws at the chrome's density.
pub(crate) fn dock_grid(app: &VectorcraftApp, owner: &Owner) -> DockGrid {
    let cells = match owner {
        Owner::Strip(parent, first) => strip_tools(app, parent, first).len(),
        Owner::Tools => groups_of(app, owner).len(),
        Owner::Toolbar(_) => groups_of(app, owner).len() + 1,
    };
    DockGrid::rail(cells).density(rail_density(app, owner))
}

/// How tightly the rail of `owner` packs its buttons: the Tools panel's packs them tight under Window › Toolbars ›
/// Compact.
fn rail_density(app: &VectorcraftApp, owner: &Owner) -> RailDensity {
    if matches!(owner, Owner::Tools) && app.ui.toolbar_compact { RailDensity::Dense } else { RailDensity::Regular }
}

/// The groups of several tools of the Tools panel's toolbar (`toolbar` none) or of the toolbar made
/// with New Toolbar called `toolbar` that float as their own panel.
pub(crate) fn strips(app: &VectorcraftApp, toolbar: Option<&str>) -> Vec<Vec<&'static str>> {
    let Some(owner) = owner_of(&app.ui, toolbar) else { return vec![] };
    let layout = crate::panel_docking::layout_of(&app.ui);
    groups_of(app, &owner).into_iter().filter(|g| g.len() > 1 && g.first().is_some_and(|first| layout.contains(&strip_id(&owner, first)))).collect()
}

/// Whether the group whose first tool is `key`, of the Tools panel's toolbar (`toolbar` none) or of
/// the toolbar made with New Toolbar called `toolbar`, floats as its own panel.
pub(crate) fn is_floating(app: &VectorcraftApp, toolbar: Option<&str>, key: &str) -> bool {
    strips(app, toolbar).iter().any(|g| g.first() == Some(&key))
}

/// Float the group `first` of `owner` as a strip at `at` (none: the canvas's corner).
fn place_strip(app: &mut VectorcraftApp, owner: &Owner, first: &str, at: Option<Pos2>) -> Result<(), String> {
    let at = at.unwrap_or_else(|| app.canvas_view().map_or(Pos2::ZERO, |v| v.rect.min));
    // A width of 0 leaves the window's width to the panel's grid.
    crate::panel_docking::place_floating(app, &strip_id(owner, first), at, 0.0)
}

/// Float the toolbar group (of the current layout) holding `tool` as a strip of tool buttons, put
/// it back in the toolbar, or toggle (`floating` omitted), as a flyout's tear-off bar and the
/// strip's × do (`window.floatTools`; `toolbar`: the group of the toolbar made with New Toolbar
/// called that). Returns the new state.
pub fn float_group(app: &mut VectorcraftApp, tool: &str, floating: Option<bool>, toolbar: Option<&str>) -> Result<bool, String> {
    let owner = owner_of(&app.ui, toolbar).ok_or_else(|| format!("no toolbar named `{}`", toolbar.unwrap_or_default()))?;
    let Some(group) = groups_of(app, &owner).into_iter().find(|s| s.len() > 1 && s.contains(&tool)) else {
        return Err(format!("{tool} isn't in a toolbar group of several tools"));
    };
    let first = group.first().copied().unwrap_or_default();
    let was = is_floating(app, toolbar, first);
    let on = floating.unwrap_or(!was);
    if on && !was {
        place_strip(app, &owner, first, None)?;
    } else if !on && was {
        let ctx = app.ui_ctx.clone();
        crate::panel_docking::apply(app, ctx.as_ref(), vec![Action::close(strip_id(&owner, first))])?;
    }
    Ok(on)
}

/// Whether `owner`'s Edit Toolbar drawer is open on it.
fn editing(app: &VectorcraftApp, owner: &Owner) -> bool {
    let target = crate::custom_toolbars::drawer_target(&app.ui);
    app.ui.tools_drawer
        && match owner {
            Owner::Tools => target.is_none(),
            Owner::Toolbar(id) => {
                target.zip(bar_of(&app.ui, *id)).is_some_and(|(t, bar)| crate::custom_toolbars::key(t) == crate::custom_toolbars::key(&bar.name))
            }
            Owner::Strip(..) => false,
        }
}

/// The side flyouts and the drawer open on: toward the canvas.
fn opens(app: &VectorcraftApp, owner: &Owner) -> Side {
    let layout = crate::panel_docking::layout_of(&app.ui);
    match crate::panel_docking::column_of(layout, &owner.panel()) {
        Some((Some((DockEdge::End, _)), _)) => Side::Start,
        _ => Side::End,
    }
}

/// The rail of `owner`, drawn by the dock: measured at a width, or shown in a rect. Returns the
/// height it takes.
pub(crate) fn rail_view(app: &mut VectorcraftApp, ui: &mut Ui, owner: Owner, at: GroupAt, _cx: &mut BodyCx<'_>) -> f32 {
    let panel = owner.panel();
    // The model leaves the app for the pass, which lends the app to the footer.
    let mut model = app.dock_rt.rail_models.remove(&panel).unwrap_or_default();
    let height = show_rail(app, ui, owner, at, &mut model);
    app.dock_rt.rail_models.insert(panel, model);
    height
}

/// What a rail reads across passes: its slots and the tools they showed last, each edited only when
/// it changed, so a pass where neither changed reads neither.
#[derive(Default)]
pub(crate) struct RailModel {
    slots: Tracked<Vec<ToolSlot<String>>>,
    last_used: Tracked<LastUsed<String>>,
}

impl RailModel {
    /// Brings the model up to `slots` and `last_used`.
    fn refresh(&mut self, slots: Vec<ToolSlot<String>>, last_used: LastUsed<String>) {
        if *self.slots != slots {
            self.slots.edit(|kept| *kept = slots);
        }
        if *self.last_used != last_used {
            self.last_used.edit(|kept| *kept = last_used);
        }
    }
}

/// [`rail_view`] over `model`, the rail's own.
fn show_rail(app: &mut VectorcraftApp, ui: &mut Ui, owner: Owner, at: GroupAt, model: &mut RailModel) -> f32 {
    let (slots, last_used) = match &owner {
        Owner::Tools => (rail_slots(&slots(app)), app.ui.tools_last_used.clone()),
        Owner::Toolbar(id) => {
            let Some(bar) = bar_of(&app.ui, *id) else { return 0.0 };
            let groups: Vec<_> = groups_of(app, &owner).into_iter().map(|g| (None, g)).collect();
            (rail_slots(&groups), bar.last_used.clone())
        }
        // One slot per tool, so a strip has no flyouts. A group no longer in the toolbar draws
        // nothing; `keep_strips` closes its panel.
        Owner::Strip(parent, first) => {
            let tools = strip_tools(app, parent, first);
            (tools.iter().map(|tool| ToolSlot::new(slot_id(tool), vec![rail_tool(tool)])).collect(), Default::default())
        }
    };
    model.refresh(slots, last_used);
    let current = app.session.tool_id().to_string();
    // PLAN: the sheet's `Cards<'a> { app }` cannot be held beside the footer's `&mut app`; Cards copies the two prefs.
    let cards = card::Cards::new(app);
    let tools_panel = owner == Owner::Tools;
    let own_bar = !matches!(owner, Owner::Strip(..));
    let opens = opens(app, &owner);
    // A Learn link's tool on this rail wears the mark; the pass that shows the rail scrolls its slot into view and opens
    // its flyout, once.
    let home = Home::Rail(&owner);
    let located = locate::wanted(&app.ui, &home);
    let holds = |tool: &str| model.slots.iter().any(|slot| slot.tools.iter().any(|t| t.id == tool));
    let reveal = matches!(at, GroupAt::Show(_)).then(|| locate::take_reveal(&mut app.ui, &home, holds)).flatten();
    // The current tool in production's pressed fill, with no accent edge.
    let mut rail = ToolRail::new(tl!("Tools"), &model.slots, &current, &model.last_used)
        .opens(opens)
        .tips(&cards)
        .active_look(ActiveLook::Pressed)
        .density(rail_density(app, &owner));
    if let Some(tool) = &located {
        rail = rail.mark(tool);
    }
    if let Some(tool) = &reveal {
        rail = rail.reveal(tool);
    }
    if app.session.prefs.tool_group_labels {
        rail = rail.captions();
    }
    if own_bar {
        rail = rail.tear_off();
        if !tools_panel {
            rail = rail.edit_toolbar(tl!("Edit Toolbar"));
        }
    }
    if editing(app, &owner) {
        rail = rail.editing();
    }
    let bar = match &owner {
        Owner::Toolbar(id) => Some(*id),
        _ => None,
    };
    let measured = matches!(at, GroupAt::Measure(_));
    let mut tools_edit = None;
    let footer = |ui: &mut Ui, footer: RailFooter| {
        if tools_panel {
            bottom_controls(app, ui, footer);
            tools_edit = search_and_edit_buttons(app, ui, footer);
        } else if let Some(id) = bar {
            crate::custom_toolbars::add_menu(app, ui, id, footer, measured);
        }
    };
    let rect = match at {
        GroupAt::Measure(width) => return rail.measure_rail(ui, width, footer).y,
        GroupAt::Show(rect) => rect,
    };
    let mut out = rail.show_rail(ui, footer);
    if tools_panel {
        // The native footer opener follows the same drawer, focus and customization path.
        out.edit_toolbar = tools_edit;
    }
    if let Some(tool) = out.select.as_deref().filter(|tool| *tool != app.session.tool_id()) {
        // Not an error to report: a tool the command refuses stays unselected.
        app.run("tool.select", json!({ "tool": tool })).ok();
    }
    if let Some(tool) = &out.double_clicked {
        // Tools without options answer with an error; a double click on one does nothing more.
        app.run("tool.options", json!({ "tool": tool })).ok();
    }
    if out.edit_toolbar.as_ref().is_some_and(|button| button.clicked()) {
        let params = match &owner {
            Owner::Toolbar(id) => json!({ "toolbar": bar_of(&app.ui, *id).map(|bar| bar.name.clone()) }),
            _ => json!({}),
        };
        app.run("window.editToolbar", params).ok();
    }
    if let Some(torn) = &out.torn_off
        && let Some(first) = model.slots.iter().find(|slot| slot.id == torn.grabbed).and_then(|slot| slot.tools.first())
    {
        // The strip floats inside this window: an OS window would never see the held press move.
        match place_strip(app, &owner, &first.id, Some(torn.pointer)) {
            // The window shows at the pointer, so the press holds it by its corner.
            Ok(()) => crate::panel_docking::continue_move(app, &strip_id(&owner, &first.id), Vec2::ZERO),
            Err(e) => app.status(e),
        }
    }
    if let Some(next) = out.last_used.clone() {
        match &owner {
            Owner::Tools => app.ui.tools_last_used = next,
            Owner::Toolbar(id) => {
                if let Some(bar) = app.ui.custom_toolbars.iter_mut().find(|bar| bar.id == *id) {
                    bar.last_used = next;
                }
            }
            Owner::Strip(..) => {}
        }
    }
    if editing(app, &owner) {
        crate::edit_toolbar::drawer(app, ui, &out, opens);
    }
    rect.height()
}

/// Open a tool's options (`tool.options`, a double-click on its button): the Gradient tool's are
/// the Gradient panel, the Eyedropper's the Eyedropper Options dialog, a Liquify tool's its Tool
/// Options dialog, the Blend tool's Blend
/// Options; the Print Tiling tool's resets the print tiling. As in the reference app, the Hand
/// tool's fits the artboard in the window, the Zoom tool's shows it at 100%, the Rotate, Scale,
/// Reflect and Shear tools' are their Object › Transform dialogs, the selection tools' are the
/// Move dialog, and the Pencil, Paintbrush, Smooth, Blob Brush, Eraser, Shape Builder and Objects
/// on Path tools' are their Tool Options, as are the Line Segment, Arc, Rectangular Grid and Polar Grid tools'
/// ([`crate::dialogs::shapes::open_options`]), the Circle tool's ([`crate::dialogs::circle_options::open`]) and the
/// Curvature Circle tool's ([`crate::dialogs::curvature_circle_options::open`]). The Type on a Path and Vertical Type on a Path
/// tools' are Type on a Path Options for the selected type on a path.
pub fn open_options(app: &mut VectorcraftApp, tool: &str) -> Result<serde_json::Value, String> {
    match tool {
        "dimension" => crate::dialogs::dimension::open(app),
        "hand" => app.run("view.fitArtboard", json!({})),
        "zoom" => app.run("view.actualSize", json!({})),
        "rotate" | "scale" | "reflect" | "shear" | "selection" | "directSelection" | "groupSelection" => {
            let dialog = if vectorcraft_tools::catalog::is_selection_tool(tool) { "move" } else { tool };
            let id = format!("object.{dialog}");
            if let Some(c) = vectorcraft_engine::find_command(&id) {
                (c.enabled)(&app.session)?;
            }
            crate::menus::invoke(app, &id, json!({}));
            Ok(json!({ "dialog": dialog }))
        }
        // The Gradient and Magic Wand tools: their panels.
        "gradient" | "magicWand" => crate::panel_docking::reveal_panel(app, tool),
        "artboard" => crate::dialogs::artboard_options::open(app),
        "eyedropper" => {
            crate::dialogs::eyedropper::open(app);
            Ok(json!({ "dialog": crate::dialogs::eyedropper::KIND }))
        }
        "blend" => {
            crate::dialogs::blend_options::open(app)?;
            Ok(json!({ "dialog": crate::dialogs::blend_options::KIND }))
        }
        "perspectiveGrid" => {
            crate::dialogs::perspective_options::open(app);
            Ok(json!({ "dialog": crate::dialogs::perspective_options::KIND }))
        }
        // Fill Area uses the Shape Builder's Gap Detection and open-filled-path options.
        "shapeBuilder" | "fillArea" => Ok(crate::dialogs::shape_builder_options::open(app)),
        // The Objects on Path tool: the Pivot and Rotate of the objects it attaches next.
        "objectsOnPath" => Ok(crate::dialogs::objects_on_path_options::open(app)),
        // The Line Segment, Arc and grid tools: their Tool Options, the options of their next
        // drag and click.
        _ if crate::dialogs::shapes::OPTION_TOOLS.contains(&tool) => crate::dialogs::shapes::open_options(app, tool),
        // The Circle tool: what it shows while a circle is drawn, where its anchors go and how
        // the third point is placed.
        vectorcraft_tools::circle::ID => Ok(crate::dialogs::circle_options::open(app)),
        // The Curvature Circle tool: how near a path pulls the pointer, the radius label, where
        // the circle's anchors go and whether a circular arc gives its own radius.
        vectorcraft_tools::curvature_circle::ID => crate::dialogs::curvature_circle_options::open(app),
        // The Three-Point Arc tool: whether the third point gives a direction.
        vectorcraft_tools::three_point_arc::ID => Ok(crate::dialogs::three_point_arc_options::open(app)),
        // The Flare tool: its options, which draw the next flare (OK draws none); with flares
        // selected, the first one's settings, and OK draws the selected flares again.
        "flare" => Ok(crate::dialogs::flare_options::open(app, None)),
        // A double click on the Print Tiling tool puts the pages back where the placement puts them.
        "printTiling" => app.run("print.tiling.set", json!({ "reset": true })),
        // The Liquify tools: their Tool Options (the Global Brush Dimensions and the tool's own).
        _ if vectorcraft_tools::settings::LIQUIFY.contains(&tool) => crate::dialogs::liquify::open(app, tool),
        // The freehand tools: their Tool Options (Fidelity, fill, the tolerances, the brush size).
        _ if crate::dialogs::freehand::TOOLS.contains(&tool) => crate::dialogs::freehand::open(app, tool),
        // The Live Paint Selection tool: its Tool Options (what a click selects, the highlight).
        "livePaintSelection" => crate::dialogs::live_paint_selection::open(app),
        // The Live Paint Bucket: its options (what a click paints, the highlight).
        "livePaintBucket" => crate::dialogs::live_paint_bucket::open(app),
        // The Symbolism tools: the brush they share.
        _ if vectorcraft_tools::settings::SYMBOLISM.contains(&tool) => Ok(crate::dialogs::symbolism_options::open(app, tool)),
        // The graph tools: Graph Type for the selected graph.
        _ if vectorcraft_tools::extra::is_graph_tool(tool) => crate::menus::graph_dialog(app, "graph.setType"),
        // The Type on a Path tools: Type on a Path Options for the selected type on a path, as
        // Type › Type on a Path › Type on a Path Options… opens it.
        "typeOnPath" | "verticalTypeOnPath" => {
            let id = "type.pathOptions";
            if let Some(c) = vectorcraft_engine::find_command(id) {
                (c.enabled)(&app.session)?;
            }
            app.session.execute(id, &json!({})).map_err(|e| e.to_string())?;
            crate::menus::invoke(app, id, json!({}));
            Ok(json!({ "dialog": "command" }))
        }
        // The add-ons' tools.
        _ if crate::addons::pathtools::has_options(tool) => crate::addons::pathtools::open_options(app, tool),
        _ if vectorcraft_tools::tool_info(tool).is_none() => Err(format!("unknown tool `{tool}`")),
        _ => Err(format!("the {tool} tool has no options")),
    }
}

/// The active tool's options in the Control bar: Mirror & Cut's axis and the side it keeps, Puppet
/// Warp's mesh and pins, the Circle tool's From Center (set through `tool.setOption`). The Artboard
/// tool's options are drawn by [`crate::panels::artboards::control_bar`]. The bar's rows drop the
/// controls that do not fit.
pub fn control_bar_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    // PLAN: assumed `addons::shapes::control_bar(app, ui)` and `dimension::controls(app, ui)` lose their `reserve` (chrome-w16 ranks).
    if crate::addons::shapes::control_bar(app, ui) {
        return;
    }
    if app.session.tool_id() == "dimension" {
        crate::dialogs::dimension::controls(app, ui);
        return;
    }
    /// (value, label) of each choice.
    type Choices = &'static [(&'static str, &'static str)];
    const MIRROR: [(&str, &str, Choices); 2] = [
        ("axis", "Axis:", &[("free", "Free"), ("vertical", "Vertical"), ("horizontal", "Horizontal")]),
        ("keep", "Keep:", &[("left", "Left"), ("right", "Right"), ("top", "Top"), ("bottom", "Bottom")]),
    ];
    if app.session.tool_id() == "puppetWarp" {
        return puppet_warp_options(app, ui);
    }
    if app.session.tool_id() == vectorcraft_tools::cropimage::ID {
        return crop_options(app, ui);
    }
    if app.session.tool_id() == vectorcraft_tools::circle::ID {
        return circle_options(app, ui);
    }
    if app.session.tool_id() != "mirrorCut" {
        return;
    }
    let opts = app.session.tool_options();
    for (key, label, choices) in MIRROR {
        let entries: Vec<Entry<&'static str>> = choices.iter().map(|&(value, name)| Entry::new(value, tl!(name))).collect();
        let current = opts[key].as_str().and_then(|v| choices.iter().find(|(value, _)| *value == v)).map(|(value, _)| *value);
        let picked = Field::new(tl!(label))
            .show(ui, |ui, _| Select::new(tl!(label), Bind::given(current.map_or(Mixed::Mixed, Mixed::Same)), &entries).show(ui).value().copied())
            .inner
            .flatten();
        if let Some(value) = picked {
            app.run("tool.setOption", json!({ "key": key, "value": value })).ok();
        }
    }
}

/// The Circle tool's Control bar: From Center (the first point is the circle's center and the
/// outline passes through the second), set through `tool.setOption`.
fn circle_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let on = app.session.tool_options()["fromCenter"].as_bool().unwrap_or(false);
    let picked = Checkbox::new(Bind::given(on)).label(tl!("From Center")).id_salt("fromCenter").show(ui).value().copied();
    if let Some(value) = picked.filter(|value| *value != on) {
        app.run("tool.setOption", json!({ "key": "fromCenter", "value": value })).ok();
    }
}

/// Crop Image's cropping options: the cropped image's resolution (PPI: a list, with the
/// resolutions above what the image allows disabled, or a typed value), the reference point, the
/// box's X and Y (that point) and size (W, H) in the general unit with the link that keeps their
/// proportions, all set through the tool's options; then Apply (Enter) and Cancel (Escape).
fn crop_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    use vectorcraft_tools::cropimage::{self as crop, PPI_CHOICES};
    let opts = app.session.tool_options();
    let set = opts["rect"].as_array().and_then(|v| match v.iter().filter_map(Value::as_f64).collect::<Vec<_>>()[..] {
        [x, y, w, h] => Some(vectorcraft_geom::Rect::new(x, y, x + w, y + h)),
        _ => None,
    });
    let Some((r, image, own, max)) = app.session.active().and_then(|st| {
        let (id, image, r) = crop::crop_box(&st.doc, &st.selection, set)?;
        let vectorcraft_doc::NodeKind::Image(im) = &st.doc.node(id)?.kind else { return None };
        let (own, max) = crop::image_ppi(im);
        Some((r, image, own, max.min(2400.0)))
    }) else {
        return;
    };
    let reference = opts["reference"].as_u64().filter(|v| *v <= 8).map_or(crop::CENTER, |v| v as usize);
    let linked = opts["linked"].as_bool().unwrap_or(false);
    let mut edits = Vec::new();
    // Resolution: the list's choices above the image's limit are disabled; a typed value is held to it.
    let ppi = opts["ppi"].as_f64().unwrap_or(own);
    let label = |v: f64| crate::i18n::fmt(tl!("{n} ppi"), &[("n", &format!("{}", (v * 100.0).round() / 100.0))]);
    let choices: Vec<Entry<f64>> = PPI_CHOICES.iter().map(|&v| Entry::new(v, label(v)).enabled(v <= max + 1e-6)).collect();
    Field::new(tl!("PPI")).show(ui, |ui, _| {
        if let Some(v) = Select::new(tl!("PPI"), Bind::given(ppi), &choices).placeholder(label(ppi)).show(ui).value().copied() {
            edits.push(("ppi", json!(v)));
        }
        // An image's own resolution above the limit (a linked image's, or one past 2400 ppi) still
        // shows as it is; an edit there sets at most the limit.
        let field = NumberField::new(tl!("Resolution"), Bind::given(Some(ppi))).range(1.0..=max.max(ppi).max(1.0)).fixed_decimals(2);
        if let Some(v) = field.show(ui).value().copied().flatten() {
            edits.push(("ppi", json!(v.min(max.max(1.0)))));
        }
    });
    Divider::vertical().show(ui);
    let rulers = crate::canvas::rulers::general_rulers(app);
    let pivot = vectorcraft_geom::reference_point(r, reference);
    let state = BoxState::new([Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], pivot);
    let mut groups = TransformGroups::NONE;
    (groups.reference, groups.position, groups.size) = (true, true, true);
    let out = TransformFields::new(tl!("Crop Box"), Bind::given(TransformRow::new(state, 0.0)), r, Bind::given(linked))
        .rulers(rulers)
        .size_unit(SizeUnit::length(rulers.x, rulers.y))
        .reference_look(ReferencePointLook::Squares)
        .groups(groups)
        .bar()
        .show(ui);
    if let Some(on) = out.linked {
        edits.push(("linked", json!(on)));
    }
    let as_option = |b: vectorcraft_geom::Rect| json!([b.x0, b.y0, b.width(), b.height()]);
    for (event, source) in out.events.iter().zip(&out.sources) {
        // A cancelled gesture puts back the box it began from.
        if let EditEvent::Cancel { before: Some(Mixed::Same(row)), .. } = event {
            edits.push(("rect", as_option(quad_bounds(&row.state.quad))));
            continue;
        }
        let Some(row) = event.value() else { continue };
        let (key, v) = match source {
            TransformSource::Reference => {
                if let Some(i) = row.state.anchor_of_pivot().and_then(|a| Anchor9::ALL.iter().position(|b| *b == a)) {
                    edits.push(("reference", json!(i)));
                }
                continue;
            }
            TransformSource::Field(TransformField::X) => ("x", row.state.pivot.x),
            TransformSource::Field(TransformField::Y) => ("y", row.state.pivot.y),
            TransformSource::Field(TransformField::W) => ("width", quad_bounds(&row.state.quad).width()),
            TransformSource::Field(TransformField::H) => ("height", quad_bounds(&row.state.quad).height()),
            _ => continue,
        };
        if let Some(b) = crop::edited_box(r, image, reference, linked, key, v) {
            edits.push(("rect", as_option(b)));
        }
    }
    for (key, value) in edits {
        app.run("tool.setOption", json!({ "key": key, "value": value })).ok();
    }
    Divider::vertical().show(ui);
    for (label, key) in [(tl!("Apply"), ToolKey::Enter), (tl!("Cancel"), ToolKey::Escape)] {
        if Button::new(label).show(ui).clicked() {
            let view = app.view_info();
            let r = app.session.tool_key(key, Mods::default(), view);
            crate::canvas::apply_requests(app, r);
        }
    }
    Divider::vertical().show(ui);
}

/// The axis-aligned bounds of a box's corners.
fn quad_bounds(quad: &[Point; 4]) -> vectorcraft_geom::Rect {
    quad[1..].iter().fold(vectorcraft_geom::Rect::from_points(quad[0], quad[0]), |b, p| b.union_pt(*p))
}

/// The Fill/Stroke proxy, Color/Gradient/None, the drawing modes and screen mode under the rail's
/// tools: in two columns the three drawing modes show, in one only the current one, which cycles
/// them. A row the rail is too narrow for goes on in the next line. The All Tools drawer's Show
/// section hides any of them.
fn bottom_controls(app: &mut VectorcraftApp, ui: &mut Ui, footer: RailFooter) {
    let two = footer.columns > 1;
    let show = app.ui.toolbar_show;
    if show.fill_stroke {
        crate::panels::proxy(app, ui);
    }
    // Color (the last solid colour), Gradient (the last gradient) and None, as commands, in small chips.
    if show.coloring {
        let chips = [
            (tl!("Color (, or Shift+,)"), "paint.lastColor", Paint::solid(app.session.last_solid)),
            (tl!("Gradient (. or Shift+.)"), "paint.lastGradient", Paint::Gradient(Box::new(app.session.last_gradient.clone()))),
            (tl!("None (/)"), "paint.none", Paint::None),
        ];
        let fallback = crate::panels::srgba(&app.session.last_solid);
        let mut clicked = None;
        Row::new().wrap(true).show(ui, |ui, _| {
            for (label, cmd, paint) in &chips {
                let shown = Mixed::Same(well_paint(paint, &|_| None));
                let out =
                    PaintChip::new(*label, Bind::given(shown), fallback).popover(false).chip_size(PaintChipSize::Swatch).tooltip(*label).show(ui);
                if let Some(mods) = out.clicked {
                    clicked = Some((*cmd, mods.cmd));
                }
            }
        });
        match clicked {
            // Cmd/Ctrl-click on Gradient resets the gradient to White, Black.
            Some(("paint.lastGradient", true)) => crate::panels::gradient::reset_to_linear_white_black(app),
            Some((cmd, _)) => {
                app.run(cmd, json!({})).ok();
            }
            None => {}
        }
    }
    let modes = [
        (vectorcraft_engine::DrawMode::Normal, "dc-draw-normal", "Draw Normal", "normal"),
        (vectorcraft_engine::DrawMode::Behind, "dc-draw-behind", "Draw Behind", "behind"),
        (vectorcraft_engine::DrawMode::Inside, "dc-draw-inside", "Draw Inside", "inside"),
    ];
    // `on` is the pressed state of one of the three side-by-side buttons; the single one-column button has none.
    let mode_button = |ui: &mut Ui, icon: &'static str, name: &str, on: Option<bool>| {
        let tip = format!("{} (Shift+D)", tl!(name));
        let button = IconButton::new(IconId::new(icon)).label(tip.clone()).tooltip(tip);
        let button = match on {
            Some(on) => button.selected(on),
            None => button,
        };
        // Small in two columns, where the three sit side by side in the rail's width.
        let button = if two { button.size(ControlSize::Sm) } else { button };
        button.show(ui).clicked()
    };
    let mut run = None;
    Row::new().wrap(true).show(ui, |ui, _| {
        if show.draw_mode {
            if two {
                // Side by side, each one a click away, a tight gap apart to fit the rail's width.
                Row::new().gap(Gap::Tight).show(ui, |ui, _| {
                    for (mode, icon, name, id) in modes {
                        if mode_button(ui, icon, name, Some(app.session.draw_mode == mode)) {
                            run = Some(("view.drawMode", json!({ "mode": id })));
                        }
                    }
                });
            } else if let Some((_, icon, name, _)) = modes.into_iter().find(|m| m.0 == app.session.draw_mode)
                && mode_button(ui, icon, name, None)
            {
                // One column: the current mode's button, named for it, which cycles them; it is always the
                // current mode, so it has no pressed state.
                run = Some(("view.drawMode", json!({})));
            }
        }
        if show.screen_mode {
            let tip = tl!("Change Screen Mode (F)");
            if IconButton::new(IconId::new("dc-screen-mode")).label(tip).tooltip(tip).show(ui).clicked() {
                run = Some(("view.screenMode", json!({})));
            }
        }
    });
    if let Some((cmd, params)) = run {
        app.run(cmd, params).ok();
    }
}

/// The Puppet Warp tool's Control bar: Expand (how far the mesh reaches past the art), Show Mesh
/// and Select All Pins.
fn puppet_warp_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let opts = app.session.tool_options();
    let lengths = crate::canvas::rulers::general_lengths(app);
    let mut set = None;
    Field::new(tl!("Expand:")).show(ui, |ui, _| {
        let field = NumberField::length(tl!("Expand"), Bind::given(opts["expand"].as_f64()), lengths).range(0.0..=1000.0);
        if let Some(v) = field.show(ui).value().copied().flatten() {
            set = Some(("expand", json!(v.clamp(0.0, 1000.0))));
        }
    });
    let show = opts["showMesh"].as_bool().unwrap_or(true);
    if let Some(on) = Checkbox::new(Bind::given(show)).label(tl!("Show Mesh")).show(ui).value().copied() {
        set = Some(("showMesh", json!(on)));
    }
    if Button::new(tl!("Select All Pins")).show(ui).clicked() {
        set = Some(("selectAllPins", json!(true)));
    }
    if let Some((key, value)) = set {
        app.run("tool.setOption", json!({ "key": key, "value": value })).ok();
    }
}

pub(crate) mod card;
/// Search and Edit Toolbar share a row in a wide rail; a narrow rail keeps Edit Toolbar last.
fn search_and_edit_buttons(app: &VectorcraftApp, ui: &mut Ui, footer: RailFooter) -> Option<egui::Response> {
    let enabled = !crate::panel_docking::modal_window_up(app);
    let mut edit = None;
    let buttons = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        IconButton::command(IconId::new("search"), "help.commandPalette")
            .label(tl!("Search commands"))
            .tooltip(tl!("Search commands, tools, panels and effects"))
            .selected(app.ui.palette_open)
            .enabled(enabled)
            .show(ui);
        edit = Some(
            IconButton::new(IconId::new("more"))
                .label(tl!("Edit Toolbar"))
                .selected(editing(app, &Owner::Tools))
                .enabled(crate::menus::enabled(app, "window.editToolbar"))
                .show(ui)
                .response,
        );
    };
    if footer.columns > 1 {
        Row::new().gap(Gap::Tight).id_salt("tools-footer-actions").show(ui, buttons);
    } else {
        Stack::new().gap(Gap::Tight).id_salt("tools-footer-actions").show(ui, buttons);
    }
    edit
}

#[cfg(test)]
pub(crate) mod tests;
