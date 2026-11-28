use craft::behavior::CanvasView;
use craft::behavior::{IconId, Mixed};
use craft::color_ui::ui::{PaintChip, PaintChipSize};
use craft::theme::{Srgba, Variant};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::canvas::CanvasSpace;
use craft::ui::canvas_controls::{TaskBar, TaskBarGrip, TaskBarHang};
use craft::ui::contract::{Bind, Gap, Props as _, Show as _, ShowContainer as _, Trigger};
use craft::ui::display::Text;
use craft::ui::menu::PanelRow;
use craft::ui::popover::Popover;
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::commands::{CommandRef, CommandState, CommandStates};
use craft::ui::runtime::env::Label;
use vectorcraft_doc::NodeKind;
use vectorcraft_doc::text::TextKind;
use vectorcraft_geom::{Point, Rect};

use super::{Ui, VectorcraftApp, json};

/// A memo of a check on the selection, kept for the document's `(uid, revision)`.
type Memo = Option<((u64, u64), bool)>;

/// An action of the bar: its label, icon, command and the command's params.
type Action = (&'static str, &'static str, &'static str, serde_json::Value);

/// What the Contextual Task Bar offers for the tool and selection in use.
pub(crate) struct Offer {
    /// The actions that show, after Bar Actions left any out.
    pub actions: Vec<Action>,
    /// Every action and chip offered, before Bar Actions left any out, for More › Bar Actions: each one's name,
    /// its id in [`TASK_BAR_ACTIONS`] or [`TASK_BAR_CHIPS`] (none: Bar Actions cannot turn it off) and whether it shows.
    pub listed: Vec<(&'static str, Option<&'static str>, bool)>,
    /// The chips that show, after the action chips Bar Actions left out.
    pub chips: Vec<Chip>,
    /// Whether the bar shows the font family, font style and Font Size controls.
    pub fonts: bool,
    /// The Smooth tool's Fidelity.
    pub fidelity: Option<f64>,
    /// The document points the bar hangs under: the corners of the selection, the picked artboards or the active
    /// artboard, or the canvas's center on a blank canvas out of view.
    pub anchor: Vec<Point>,
    /// The live shape the Lock chip's properties popover shows.
    pub shape: Option<std::sync::Arc<vectorcraft_doc::Node>>,
}

/// Whether the objects `ids` (or anything inside them) are painted with a colour: a solid colour,
/// a gradient's stops or a mesh's points, which Recolor Artwork can change.
pub(crate) fn has_colors(doc: &vectorcraft_doc::Document, ids: &[vectorcraft_doc::NodeId]) -> bool {
    ids.iter().filter_map(|id| doc.node(*id)).any(vectorcraft_doc::swatches::node_has_colors)
}

/// The bounds of the artboards the Artboard tool has selected ([`crate::panels::artboards::picked`]).
fn picked_artboards_bounds(app: &VectorcraftApp) -> Option<Rect> {
    let st = app.session.active()?;
    let picked = crate::panels::artboards::picked(app, st.doc.artboards.len());
    picked.iter().filter_map(|i| st.doc.artboards.get(*i)).map(|a| a.rect).reduce(|a, b| a.union(b))
}

/// The Type tools whose Contextual Task Bar shows the text formatting controls. The Touch Type
/// tool has no bar: its own handles take the place of one.
const TYPE_TOOLS: [&str; 6] = ["type", "areaType", "typeOnPath", "verticalType", "verticalAreaType", "verticalTypeOnPath"];

/// Whether every selected object is a type object, with no anchors picked out.
fn only_type_selected(app: &VectorcraftApp) -> bool {
    app.session.active().is_some_and(|st| {
        !st.selection.is_empty()
            && st.selection.anchors.is_empty()
            && st.selection.objects.iter().all(|id| st.doc.node(*id).is_some_and(|n| matches!(n.kind, NodeKind::Text(_))))
    })
}

/// Whether a selection tool is in use with something selected.
fn selection_tool_with_selection(app: &VectorcraftApp) -> bool {
    matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection")
        && app.session.active().is_some_and(|st| !st.selection.is_empty())
}

/// The basic shape tools whose Contextual Task Bar shows Edit Path, Fill, Stroke and the selected
/// shape's properties. The Line Segment tool is not one of them.
const SHAPE_TOOLS: [&str; 5] = ["rectangle", "roundedRectangle", "ellipse", "polygon", "star"];

/// Whether a basic shape tool is in use with one closed path selected whole, as each of those tools
/// leaves the shape it draws.
fn shape_tool_on_shape(app: &VectorcraftApp) -> bool {
    SHAPE_TOOLS.contains(&app.session.tool_id())
        && app.session.active().is_some_and(|st| {
            st.selection.objects.len() == 1
                && st.selection.anchors.is_empty()
                && st
                    .selection
                    .objects
                    .first()
                    .and_then(|id| st.doc.node(*id))
                    .is_some_and(|n| matches!(&n.kind, NodeKind::Path { path, .. } if path.is_closed()))
        })
}

/// Whether the Smooth tool is in use with a selected path it can smooth
/// ([`vectorcraft_tools::draw2::can_smooth`]). The answer is kept in `memo` for the document's
/// revision, which moves with every change of the document or the selection, so an unchanged
/// selection is not walked again each frame.
fn smooth_tool_on_paths(app: &VectorcraftApp, memo: &mut Memo) -> bool {
    if app.session.tool_id() != "smooth" {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let key = (st.uid, st.revision);
    if let Some((k, found)) = *memo
        && k == key
    {
        return found;
    }
    let found = vectorcraft_tools::draw2::can_smooth(&st.doc, &st.selection);
    *memo = Some((key, found));
    found
}

/// Whether the selection holds images together with other objects, for which the bar offers Make
/// Clipping Mask: at least one selected object is an image and at least one is not (a selected id
/// with no object counts as not an image). The walk stops once it has seen both. The answer is kept
/// in `memo` for the document's revision, as [`smooth_tool_on_paths`] keeps its own.
pub(crate) fn images_with_other_art(st: &vectorcraft_engine::DocState, memo: &mut Memo) -> bool {
    let key = (st.uid, st.revision);
    if let Some((k, found)) = *memo
        && k == key
    {
        return found;
    }
    let (mut image, mut other) = (false, false);
    let found = st.selection.objects.iter().any(|id| {
        if st.doc.node(*id).is_some_and(|o| matches!(o.kind, NodeKind::Image(_))) {
            image = true;
        } else {
            other = true;
        }
        image && other
    });
    *memo = Some((key, found));
    found
}

/// Whether the bar shows its blank canvas actions, Draw (Pencil) and Place Image: a selection tool is
/// in use and the active document has no artwork, only layers and guides.
fn blank_canvas(app: &VectorcraftApp) -> bool {
    fn no_art(nodes: &[std::sync::Arc<vectorcraft_doc::Node>]) -> bool {
        nodes.iter().all(|n| n.is_guide() || (n.is_layer() && n.children().is_none_or(|c| no_art(c))))
    }
    matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection")
        && app.session.active().is_some_and(|st| st.selection.is_empty() && no_art(&st.doc.layers))
}

/// The four corners of `r`, in document points.
fn corners(r: Rect) -> Vec<Point> {
    vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// The document points the bar hangs under (the framework hangs it under the lowest of their corners on screen,
/// centred on them, so a rotated view keeps it below): the selected artboards or the selection; on a blank canvas,
/// which has no selection, the active artboard when part of it is in view, else the canvas's center. Empty with
/// nothing to sit under.
fn anchor(app: &mut VectorcraftApp, view: &CanvasView, on_artboards: bool, blank: bool) -> Vec<Point> {
    if blank {
        let board = app.session.active().and_then(|st| {
            let last = st.doc.artboards.len().checked_sub(1)?;
            st.doc.artboards.get(app.view().map_or(0, |v| v.artboard).min(last)).map(|a| a.rect)
        });
        let shown = board.filter(|b| egui::Rect::from_points(&view.quad(*b)).intersects(view.rect));
        return shown.map_or_else(|| vec![view.to_doc(view.rect.center())], corners);
    }
    let bounds = if on_artboards { picked_artboards_bounds(app) } else { app.selection_bounds() };
    bounds.map(corners).unwrap_or_default()
}

/// Whether a Type tool edits type, or has type selected: its bar then shows only the text
/// formatting controls.
fn type_tool_on_type(app: &VectorcraftApp) -> bool {
    TYPE_TOOLS.contains(&app.session.tool_id()) && (crate::panels::character::text_editing(app).is_some() || only_type_selected(app))
}

/// Whether the Contextual Task Bar shows the font family, font style and Font Size controls: a
/// Type tool edits or has selected type, or a selection tool has only type selected.
fn task_bar_shows_type(app: &VectorcraftApp) -> bool {
    type_tool_on_type(app) || (selection_tool_with_selection(app) && only_type_selected(app))
}

/// What the Contextual Task Bar offers for the tool and selection in use, without the place it hangs at. The
/// per-tool rules:
/// With the Artboard tool it offers Resize (Artboard Options), Duplicate, Rearrange (the selected artboards)
/// and Export (Export for Screens).
/// A basic shape selected with the Selection tool shows a Shape Properties button in place of Lock.
/// While a basic shape tool (Rectangle, Rounded Rectangle, Ellipse, Polygon or Star) has one closed
/// path selected, as after it draws one, the bar offers Edit Path, Fill and Stroke, and Shape
/// Properties for a live rectangle, rounded rectangle, ellipse or polygon; with nothing selected, or
/// any other selection, those tools show no bar. While the Smooth tool has a path it can smooth
/// selected, the bar shows the tool's Fidelity field and Tool Options…; with no such path it shows
/// no bar. Selected type also shows the font family, font style and Font Size controls before its Fill.
/// While a Type tool edits type, or has type selected, the bar shows only those controls and the Fill.
/// On a blank canvas, a document with no artwork and a selection tool in use, it offers Draw (Pencil)
/// and Place Image. More › Bar Actions turns each action and chip off; the bar leaves out the ones
/// that are off. `None` where the bar is not shown.
fn offer(app: &VectorcraftApp, images_memo: &mut Memo, smooth_memo: &mut Memo) -> Option<Offer> {
    let on_artboards = app.session.tool_id() == "artboard";
    // Text added with a Type tool shows its formatting controls, as the help page describes
    // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
    let typing = type_tool_on_type(app);
    // A blank canvas offers Draw (Pencil) and Place Image, as the same help page describes.
    let blank = !on_artboards && blank_canvas(app);
    // A basic shape tool with its shape selected offers Edit Path, Fill, Stroke and the shape's
    // properties, as the same help page describes.
    let shaping = shape_tool_on_shape(app);
    // The Smooth tool with a path to smooth offers its option, Fidelity, as the same help page
    // describes for the Smooth tool, and Tool Options… for its dialog.
    let smoothing = smooth_tool_on_paths(app, smooth_memo);
    if !(on_artboards || typing || blank || shaping || smoothing || selection_tool_with_selection(app)) {
        return None;
    }
    // Selected type has the font controls too, as the help page describes
    // (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/font-options-overview.html>).
    let fonts = task_bar_shows_type(app);
    let st = app.session.active()?;
    let n = st.selection.len();
    let first = st.selection.objects.first().and_then(|id| st.doc.node(*id));
    let is_group = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Group { .. }));
    let is_image = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Image(_)));
    let is_clip_group = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Group { clip: true, .. }));
    let is_compound = first.as_ref().is_some_and(|f| matches!(f.kind, NodeKind::Compound { .. }));
    let open_path = first.as_ref().is_some_and(|f| matches!(&f.kind, NodeKind::Path { path, .. } if !path.is_closed()));
    let text = first.as_ref().and_then(|f| match &f.kind {
        NodeKind::Text(t) => Some(&t.kind),
        _ => None,
    });
    // Images and vector art selected together: a clipping mask is the likely next step.
    let mixed = n > 1 && images_with_other_art(st, images_memo);
    // An Image Trace object: the group the engine names "Image Trace", its source image first.
    let is_trace = is_group
        && first.as_ref().is_some_and(|f| {
            f.name.as_deref() == Some("Image Trace") && f.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
        });
    // A basic shape (a live rectangle, rounded rectangle, ellipse or polygon) selected whole with the
    // Selection tool or a basic shape tool: its properties take the place of Lock, as the help page
    // describes (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
    use vectorcraft_doc::LiveShape;
    let shape = first
        .filter(|f| {
            n == 1
                && st.selection.anchors.is_empty()
                && (app.session.tool_id() == "selection" || shaping)
                && matches!(&f.kind, NodeKind::Path { live: Some(l), .. }
                    if matches!(**l, LiveShape::Rectangle { .. } | LiveShape::Ellipse { .. } | LiveShape::Polygon { .. }))
        })
        .cloned()
        .map(std::sync::Arc::new);
    let mut items: Vec<Action> = vec![];
    let none = || json!({});
    // Edit Path: the Direct Selection tool, to edit the path's anchors.
    let edit_path = || (tl!("Edit Path"), "dc-direct", "tool.select", json!({"tool": "directSelection"}));
    if on_artboards {
        artboard_items(&mut items);
    } else if blank {
        // Draw (Pencil) switches to the Pencil tool; Place Image picks files for the Place dialog.
        items.push((tl!("Draw (Pencil)"), "pencil", "tool.select", json!({"tool": "pencil"})));
        items.push((tl!("Place Image"), "image", "file.place", none()));
    } else if shaping {
        // A basic shape tool's one action; its Fill, Stroke and Shape Properties follow as chips.
        items.push(edit_path());
    } else if smoothing {
        // The Fidelity field, then Tool Options… for the dialog that shows it with its slider.
        items.push((tl!("Fidelity:"), "", TASK_BAR_FIDELITY, none()));
        items.push((tl!("Tool Options…"), "settings", "tool.options", json!({"tool": "smooth"})));
    } else if !typing {
        // The selection tools' object actions. A Type tool's bar leaves them out, Create Outlines
        // and the type conversions included.
        // As in the reference app, an image offers Image Trace, Crop Image and Remove Background, and
        // traced art offers Expand (<https://community.adobe.com/announcements-651/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>,
        // <https://community.adobe.com/announcements-651/ai-powered-image-editing-features-in-illustrator-1624061>).
        if n == 1 && is_image {
            items.push((tl!("Image Trace"), "image", TASK_BAR_IMAGE_TRACE, none()));
            items.push((tl!("Crop Image"), "dc-pf-crop", "ui.cropImage", none()));
            items.push((tl!("Remove Background"), "eraser", crate::addons::image::REMOVE_BACKGROUND, none()));
        }
        // Direct-selected anchors take the place of a path's Offset Path and Simplify. For them the bar
        // offers Remove, Connect, Cut and Convert to corner or smooth, as the reference app does
        // (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
        // These run the same commands as the Control bar's anchor buttons.
        let anchors = !st.selection.anchors.is_empty();
        // With the Direct Selection tool, a path selected whole offers its anchor points' actions
        // too, as the reference app's bar does for any path that tool selects (same page), and Edit
        // Path, which only switches to that tool, is left out (#4469).
        let direct = app.session.tool_id() == "directSelection";
        if anchors {
            items.push((tl!("Remove Anchor Points"), "pen-tool-delete", "path.removeAnchors", none()));
            items.push((tl!("Connect"), "dc-join", "path.join", none()));
            items.push((tl!("Cut Path"), "scissors", "path.cutAtAnchors", none()));
            items.push((tl!("Convert Selected Anchor Points to Corner"), "dc-anchor", TASK_BAR_TO_CORNER, none()));
            items.push((tl!("Convert Selected Anchor Points to Smooth"), "dc-anchor-smooth", TASK_BAR_TO_SMOOTH, none()));
        }
        // As in the reference app, open paths offer Join and Edit Path, compound paths and clipping
        // masks their Release, images selected with vector art Make Clipping Mask, type converts
        // between point and area type, and type on a path offers its options (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
        if n > 1 {
            items.push((tl!("Group"), "group", "object.group", none()));
            if mixed {
                items.push((tl!("Make Clipping Mask"), "dc-pf-crop", "object.clippingMask.make", none()));
            } else {
                items.push((tl!("Unite"), "squares-unite", TASK_BAR_UNITE, none()));
            }
        } else if is_trace {
            items.push((tl!("Expand"), "maximize-2", "imageTrace.expand", none()));
        } else if is_clip_group {
            items.push((tl!("Edit Clipping Path"), "dc-direct", "object.clippingMask.editMask", none()));
            items.push((tl!("Release Clipping Mask"), "ungroup", "object.clippingMask.release", none()));
        } else if is_group {
            items.push((tl!("Ungroup"), "ungroup", "object.ungroup", none()));
            // Isolate is left out while Enter Isolation Mode is off, as it is while an opacity mask is edited.
            if crate::menus::enabled(app, "object.isolate") {
                items.push((tl!("Isolate"), "square-dashed", "object.isolate", none()));
            }
        } else if is_compound {
            if !direct {
                items.push(edit_path());
            }
            items.push((tl!("Release Compound Path"), "ungroup", "object.compoundPath.release", none()));
        } else if let Some(kind) = text {
            items.push((tl!("Create Outlines"), "type", "type.createOutlines", none()));
            // Point type converts to area type and back. The overview above gives type on a path an item to add
            // an effect and rearrange the text; here it opens Type on a Path Options.
            match kind {
                TextKind::Area { .. } => items.push((tl!("Convert To Point Type"), "type", "type.convertToPointType", none())),
                TextKind::Point => items.push((tl!("Convert To Area Type"), "dc-type-area", "type.convertToAreaType", none())),
                TextKind::OnPath { .. } => {
                    items.push((tl!("Type on a Path Options"), "dc-type-path", "type.pathOptions", none()));
                    center_on_path_items(app, &mut items);
                }
            }
            // Dynamic Text turns on or off for type on a path and area type (Block), and turns point type into area
            // type with Block on, as the Type menu's item does; left out for type in a thread
            // (<https://helpx.adobe.com/photoshop/desktop/text-typography/work-with-dynamic-text/adjust-formatting-and-resize-text-with-dynamic-text.html>).
            if crate::menus::enabled(app, "type.dynamicText") {
                items.push((tl!("Dynamic Text"), "maximize-2", "type.dynamicText", none()));
            }
        } else if is_image {
            // Offset Path and Simplify don't apply to an image.
        } else if !anchors && direct {
            // The two convert buttons convert every anchor of a path selected whole; Remove and Cut
            // need anchors picked out, so they wait for them. Simplify and Smooth open their
            // sliders, as Object › Path › Simplify… and Smooth… do; the page above names Smooth as
            // adjusting the path's curvature, apart from converting its anchor points.
            if open_path {
                items.push((tl!("Connect"), "dc-join", "path.join", none()));
            }
            items.push((tl!("Convert Selected Anchor Points to Corner"), "dc-anchor", TASK_BAR_TO_CORNER, none()));
            items.push((tl!("Convert Selected Anchor Points to Smooth"), "dc-anchor-smooth", TASK_BAR_TO_SMOOTH, none()));
            items.push((tl!("Simplify"), "spline", "object.path.simplify", none()));
            items.push((tl!("Smooth"), "dc-smooth", "object.path.smooth", none()));
        } else if !anchors {
            if open_path {
                items.push((tl!("Join"), "dc-join", "path.join", none()));
            }
            items.push(edit_path());
            items.push((tl!("Offset Path"), "square-dashed", "object.path.offsetPath", none()));
            items.push((tl!("Simplify"), "spline", "object.path.simplify", none()));
        }
        items.push((tl!("Duplicate"), "copy", "edit.duplicate", none()));
    }
    let colored = has_colors(&st.doc, &st.selection.objects);
    // The selected objects' Fill, Stroke, Recolor Artwork (when they have colours to recolour) and
    // Lock (a basic shape's properties instead), left out with the Artboard tool and on a blank
    // canvas, which have no selected objects. A Type tool's bar has the Fill alone. A basic shape
    // tool's bar has the Fill, the Stroke and a live shape's Shape Properties. The Smooth tool's bar
    // has none of them.
    let objects = !on_artboards && !blank && !smoothing;
    let chips: Vec<Chip> = [
        (objects, Chip::Fill),
        (objects && !typing, Chip::Stroke),
        (objects && !typing && !shaping && colored, Chip::Recolor),
        (objects && !typing && (!shaping || shape.is_some()), Chip::Lock),
    ]
    .into_iter()
    .filter_map(|(shown, chip)| shown.then_some(chip))
    .collect();
    let hidden = &app.ui.task_bar_hidden_actions;
    let is_shown = |id: Option<&str>| id.is_none_or(|a| !hidden.contains(a));
    let mut listed: Vec<(&'static str, Option<&'static str>, bool)> = items
        .iter()
        .map(|(label, _, cmd, params)| {
            let id = action_id(cmd, params);
            (*label, id, is_shown(id))
        })
        .collect();
    items.retain(|(_, _, cmd, params)| is_shown(action_id(cmd, params)));
    listed.extend(chips.iter().map(|c| {
        let id = c.id(shape.is_some());
        (c.label(shape.is_some()), Some(id), is_shown(Some(id)))
    }));
    let chips = chips.into_iter().filter(|c| is_shown(Some(c.id(shape.is_some())))).collect();
    let fidelity = if smoothing { app.session.tool_options()["fidelity"].as_f64() } else { None };
    Some(Offer { actions: items, listed, chips, fonts, fidelity, anchor: vec![], shape })
}

/// What the bar offers and where it hangs, for this canvas view; `None` where it shows no bar.
pub(crate) fn offered(app: &mut VectorcraftApp) -> Option<Offer> {
    let view = app.canvas_view()?;
    let (mut images, mut smooth) = (app.canvas.images_with_other_art, app.canvas.smooth_paths);
    let offer = offer(app, &mut images, &mut smooth);
    app.canvas.images_with_other_art = images;
    app.canvas.smooth_paths = smooth;
    let mut offer = offer?;
    let on_artboards = app.session.tool_id() == "artboard";
    let blank = !on_artboards && blank_canvas(app);
    offer.anchor = anchor(app, &view, on_artboards, blank);
    (!offer.anchor.is_empty()).then_some(offer)
}

/// Publishes the states of More › Bar Actions' rows: each action and chip the bar offers is named and checked while
/// it shows, and Reset Bar Actions is enabled while any is off. The commands are the ones `window.taskBar.actions`
/// and `window.taskBar.resetActions` run.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let (mut images, mut smooth) = (app.canvas.images_with_other_art, app.canvas.smooth_paths);
    for (label, id, shown) in offer(app, &mut images, &mut smooth).into_iter().flat_map(|o| o.listed) {
        if let Some(id) = id {
            let command = CommandRef::with_params("window.taskBar.actions", json!({ "action": id }));
            states.set(command, CommandState::new().title(bar_action_name(label)).checked(shown));
        }
    }
    states.set(CommandRef::new("window.taskBar.resetActions"), CommandState::new().enabled(!app.ui.task_bar_hidden_actions.is_empty()));
}

/// The rows More Options shows after the framework's own: Bar Actions, Reset Bar Actions and Show Properties Panel.
fn more_rows(offer: &Offer) -> Vec<PanelRow> {
    let actions = offer
        .listed
        .iter()
        .filter_map(|(_, id, _)| *id)
        .map(|id| PanelRow::Command(CommandRef::with_params("window.taskBar.actions", json!({ "action": id }))))
        .collect();
    vec![
        PanelRow::Submenu(Label::from(tl!("Bar Actions")), actions),
        PanelRow::Command(CommandRef::new("window.taskBar.resetActions")),
        PanelRow::Separator,
        PanelRow::Command(CommandRef::with_params("window.panel", json!({"panel": "properties"}))),
    ]
}

/// The Contextual Task Bar: a floating pill under the selection with the most likely next actions
/// ([`offered`]). Its grip drags it anywhere on the canvas; unpinned it keeps that offset as it follows the
/// selection, pinned it stays put ([`super::bar_place::KeyedPlace`]). More Options › Bar Actions turns each action
/// and chip the bar offers on or off ([`TASK_BAR_ACTIONS`], [`TASK_BAR_CHIPS`]); with every one of them off, the
/// bar still shows its grip and More Options, where they turn back on. Returns the bar's rect on screen.
pub(super) fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) -> Option<egui::Rect> {
    let offer = offered(app)?;
    let key = (app.session.active()?.uid, false);
    let mut local = app.ui.task_bar_place.for_key(key);
    let before = local;
    let paints = chip_paints(app, ui.ctx());
    let has_shape = offer.shape.is_some();
    let mut run: Option<(&'static str, serde_json::Value)> = None;
    // Whether Alt was held for the click that set `run`.
    let mut alt = false;
    let mut show_swatches = false;
    let shown = TaskBar::new(tl!("Contextual Task Bar"), &offer.anchor, Bind::bound(&mut local))
        .grip(TaskBarGrip::Bar)
        .member_gap(Gap::Item)
        .hang_gap(TaskBarHang::ClearOfHandles)
        .more_rows(more_rows(&offer))
        .show(ui, space.view.rect, |ui, _| {
            for (label, icon, cmd, params) in &offer.actions {
                if *cmd == TASK_BAR_FIDELITY {
                    // The Smooth tool's Fidelity, labeled, sets the tool's option as it is typed or stepped.
                    if let Some(v) = offer.fidelity {
                        Text::new(*label).show(ui);
                        if let Some(x) = crate::dialogs::freehand::fidelity_field(ui, "task-bar-fidelity", v, "smooth").filter(|x| *x != v) {
                            run = Some(("tool.setOption", json!({"tool": "smooth", "values": {"fidelity": x}})));
                        }
                    }
                } else if icon_only(cmd) {
                    // Convert to Corner and to Smooth show their icon alone, named by their label, as production's did.
                    if IconButton::new(IconSpec::svg(IconId::new(icon))).label(*label).show(ui).clicked() {
                        run = Some((*cmd, params.clone()));
                    }
                } else {
                    let button = Button::new(*label).variant(Variant::SECONDARY).icon(IconSpec::svg(IconId::new(icon))).show(ui);
                    if button.clicked() {
                        run = Some((*cmd, params.clone()));
                        alt = button.mods.alt;
                    }
                }
            }
            // Type's font family, font style and Font Size, which act as the Character panel's do: on the
            // characters a Type tool selects, else on the selected type.
            if offer.fonts {
                crate::panels::character::task_bar(app, ui);
            }
            for chip in &offer.chips {
                match chip {
                    Chip::Fill | Chip::Stroke => {
                        let stroke = *chip == Chip::Stroke;
                        let (paint, mixed) = if stroke { (&paints.1, paints.3) } else { (&paints.0, paints.2) };
                        let value = if mixed { Mixed::Mixed } else { Mixed::Same(crate::paint_preview::well_paint(paint, &|_| None)) };
                        // The fallback is unused: without the popover a click never switches the paint to Solid.
                        let chip = PaintChip::new(chip.label(has_shape), Bind::given(value), Srgba::BLACK)
                            .popover(false)
                            .ring(stroke)
                            .chip_size(PaintChipSize::Compact)
                            .selected(app.session.fill_active != stroke)
                            .show(ui);
                        if chip.clicked.is_some() {
                            app.session.fill_active = !stroke;
                            show_swatches = true;
                        }
                    }
                    Chip::Recolor => {
                        if IconButton::new(IconSpec::svg(IconId::new("palette"))).label(tl!("Recolor Artwork")).show(ui).clicked() {
                            run = Some(("ui.recolorDialog", json!({})));
                        }
                    }
                    Chip::Lock => {
                        if let Some(shape) = &offer.shape
                            && let NodeKind::Path { live: Some(live), .. } = &shape.kind
                        {
                            // The shape's properties in a popover that stays open while its fields and its
                            // Corner dropdown are used; the popover keeps its own open state.
                            let props = IconButton::new(IconSpec::svg(IconId::new("shapes"))).label(tl!("Shape Properties")).show(ui);
                            let _ = Popover::new(tl!("Shape Properties"), Bind::kept())
                                .overlay(|overlay| overlay.opened_by(&props.response, Trigger::Click))
                                .show(ui, |ui, _| crate::panels::transform::live_shape_rows(app, ui, shape, live, true));
                        } else {
                            // The Lock button is off in isolation mode, as Object › Lock › Selection is.
                            let lock = IconButton::new(IconSpec::svg(IconId::new("lock")))
                                .label(tl!("Lock (⌘2)"))
                                .enabled(crate::menus::enabled(app, "object.lock"))
                                .show(ui);
                            if lock.clicked() {
                                run = Some(("object.lock", json!({})));
                            }
                        }
                    }
                }
            }
        });
    let (rect, hide) = (shown.output.rect, shown.output.hide);
    if local != before {
        app.ui.task_bar_place.set(key, local);
    }
    if hide {
        run = Some(("window.taskBar", json!({})));
    }
    // The Swatches panel shows where it is docked, floating or closed. A panel in an icon column
    // toggles when activated, so an open flyout stays as it is.
    if show_swatches
        && app.ui.open_panel.as_deref() != Some("swatches")
        && let Err(e) = app.run("window.panel.activate", json!({ "panel": "swatches" }))
    {
        app.ui.status = e;
    }
    match run {
        // Image Trace traces the image at once with the Default preset. As a quick trace from
        // Properties or the Control bar, it leaves the Image Trace panel as it was.
        Some((TASK_BAR_IMAGE_TRACE, _)) => crate::menus::invoke(app, "imageTrace.make", json!({"preset": "Default"})),
        // The Artboard tool's buttons: Artboard Options for the active artboard, as Properties'
        // icon opens it; Duplicate and Rearrange for every selected artboard.
        Some((TASK_BAR_ARTBOARD_OPTIONS, _)) => crate::menus::invoke(app, "tool.options", json!({"tool": "artboard"})),
        Some((TASK_BAR_DUPLICATE_ARTBOARDS, _)) => crate::panels::artboards::duplicate_picked(app),
        Some((TASK_BAR_REARRANGE_ARTBOARDS, _)) => crate::panels::artboards::open_rearrange_selected(app),
        // The convert buttons convert the direct-selected anchors, as the Control bar's buttons do.
        Some((TASK_BAR_TO_CORNER, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "corner"})),
        Some((TASK_BAR_TO_SMOOTH, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "smooth"})),
        // Unite is the Pathfinder panel's Shape Mode: Alt, read from the click, and the Compound
        // Shapes setting make a compound shape, and selected members take the mode (#6333).
        Some((TASK_BAR_UNITE, _)) => {
            let members = !vectorcraft_engine::cmd::selected_member_modes(&app.session).is_empty();
            crate::panels::pathfinder::shape_mode(app, "unite", vectorcraft_doc::ShapeMode::Add, members, alt);
        }
        Some((c, p)) => crate::menus::invoke(app, c, p),
        None => {}
    }
    Some(rect)
}

/// The controls the Contextual Task Bar shows after its actions: the Fill and Stroke chips,
/// Recolor Artwork, and Lock or a basic shape's Shape Properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Chip {
    Fill,
    Stroke,
    Recolor,
    Lock,
}

impl Chip {
    /// The chip's id in [`TASK_BAR_CHIPS`]; Lock is `shapeProperties` while it shows a basic
    /// `shape`'s properties.
    pub(crate) fn id(self, shape: bool) -> &'static str {
        match self {
            Chip::Fill => "fill",
            Chip::Stroke => "stroke",
            Chip::Recolor => "recolor",
            Chip::Lock if shape => "shapeProperties",
            Chip::Lock => "lock",
        }
    }

    /// The chip's name in More Options › Bar Actions, translated.
    fn label(self, shape: bool) -> &'static str {
        match self {
            Chip::Fill => tl!("Fill"),
            Chip::Stroke => tl!("Stroke"),
            Chip::Recolor => tl!("Recolor Artwork"),
            Chip::Lock if shape => tl!("Shape Properties"),
            Chip::Lock => tl!("Lock"),
        }
    }
}

/// The Contextual Task Bar's actions that More Options › Bar Actions turns on or off
/// (`window.taskBar.actions {action, visible?}`): each action's id, the command its button runs and
/// the `tool` that command names, if any. An action that is off stays out of the bar, and More
/// Options does not list it either; turning one off changes only the bar. Every action starts on.
pub(crate) const TASK_BAR_ACTIONS: [(&str, &str, Option<&str>); 37] = [
    ("imageTrace", TASK_BAR_IMAGE_TRACE, None),
    ("cropImage", "ui.cropImage", None),
    ("removeBackground", crate::addons::image::REMOVE_BACKGROUND, None),
    ("removeAnchorPoints", "path.removeAnchors", None),
    ("join", "path.join", None),
    ("cutPath", "path.cutAtAnchors", None),
    ("convertToCorner", TASK_BAR_TO_CORNER, None),
    ("convertToSmooth", TASK_BAR_TO_SMOOTH, None),
    ("group", "object.group", None),
    ("makeClippingMask", "object.clippingMask.make", None),
    ("unite", TASK_BAR_UNITE, None),
    ("expand", "imageTrace.expand", None),
    ("editClippingPath", "object.clippingMask.editMask", None),
    ("releaseClippingMask", "object.clippingMask.release", None),
    ("ungroup", "object.ungroup", None),
    ("isolate", "object.isolate", None),
    ("editPath", "tool.select", Some("directSelection")),
    ("releaseCompoundPath", "object.compoundPath.release", None),
    ("createOutlines", "type.createOutlines", None),
    ("convertToPointType", "type.convertToPointType", None),
    ("convertToAreaType", "type.convertToAreaType", None),
    ("typeOnPathOptions", "type.pathOptions", None),
    ("centerOnTop", "type.pathCenter.top", None),
    ("centerOnBottom", "type.pathCenter.bottom", None),
    ("dynamicText", "type.dynamicText", None),
    ("offsetPath", "object.path.offsetPath", None),
    ("simplify", "object.path.simplify", None),
    ("smooth", "object.path.smooth", None),
    ("duplicate", "edit.duplicate", None),
    ("drawPencil", "tool.select", Some("pencil")),
    ("placeImage", "file.place", None),
    ("fidelity", TASK_BAR_FIDELITY, None),
    ("smoothToolOptions", "tool.options", Some("smooth")),
    ("resizeArtboard", TASK_BAR_ARTBOARD_OPTIONS, None),
    ("duplicateArtboards", TASK_BAR_DUPLICATE_ARTBOARDS, None),
    ("rearrangeArtboards", TASK_BAR_REARRANGE_ARTBOARDS, None),
    ("exportArtboards", "file.exportForScreens", None),
];

/// The ids of the Contextual Task Bar's chips that More Options › Bar Actions turns on or off,
/// as [`TASK_BAR_ACTIONS`] are.
pub(crate) const TASK_BAR_CHIPS: [&str; 5] = ["fill", "stroke", "recolor", "lock", "shapeProperties"];

/// Whether `id` names one of the Contextual Task Bar's actions or chips.
pub(crate) fn is_task_bar_action(id: &str) -> bool {
    TASK_BAR_ACTIONS.iter().any(|a| a.0 == id) || TASK_BAR_CHIPS.contains(&id)
}

/// The id in [`TASK_BAR_ACTIONS`] of the bar's item that runs `cmd` with `params`.
fn action_id(cmd: &str, params: &serde_json::Value) -> Option<&'static str> {
    let tool = params.get("tool").and_then(serde_json::Value::as_str);
    TASK_BAR_ACTIONS.iter().find(|(_, c, t)| *c == cmd && *t == tool).map(|a| a.0)
}

/// The name More Options › Bar Actions lists for the bar's item `label`: without a field's colon
/// (a full-width one too), the space some languages put before it, or a final ellipsis (`…` or
/// `...`), in whichever order they come.
pub(crate) fn bar_action_name(label: &str) -> &str {
    let mut name = label.trim_end();
    loop {
        let rest = name.trim_end_matches([':', '：', '…']);
        let rest = rest.strip_suffix("...").unwrap_or(rest).trim_end();
        if rest == name {
            return name;
        }
        name = rest;
    }
}

/// Reads the Contextual Task Bar's actions and chips that are off, as kept in the preferences. Ids
/// the bar does not have are dropped, and a value that is not a list of strings reads as none off,
/// so the bar shows every action.
pub(crate) fn read_hidden_task_bar_actions<'de, D: serde::Deserializer<'de>>(d: D) -> Result<std::collections::BTreeSet<String>, D::Error> {
    let value = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
    let ids = value.as_array().into_iter().flatten().filter_map(serde_json::Value::as_str);
    Ok(ids.filter(|id| is_task_bar_action(id)).map(str::to_string).collect())
}

/// The paints the Fill and Stroke chips show, and whether each is mixed ("?"): the proxies', or
/// while a Type tool edits type, the edited characters' fill, which a swatch picked then paints.
pub(crate) fn chip_paints(app: &VectorcraftApp, ctx: &egui::Context) -> (vectorcraft_color::Paint, vectorcraft_color::Paint, bool, bool) {
    let (fill, stroke) = crate::panels::current_paints(app);
    let (fill_mixed, stroke_mixed) = crate::panels::mixed_paints(app, ctx);
    if crate::panels::character::text_editing(app).is_some()
        && let Some((chars, _)) = crate::panels::character::text_style(app)
    {
        let mixed = crate::panels::character::shared(app, |s| s.fill.clone()).is_none();
        return (chars.fill, stroke, mixed, stroke_mixed);
    }
    (fill, stroke, fill_mixed, stroke_mixed)
}

/// The Contextual Task Bar's items with the Artboard tool: Resize, Duplicate, Rearrange (the
/// selected artboards) and Export.
fn artboard_items(items: &mut Vec<Action>) {
    // As in the reference app, the Artboard tool offers to resize, duplicate and export the
    // artboards (<https://community.adobe.com/questions-652/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>),
    // and to rearrange them (<https://creativepro.com/managing-artboards-in-illustrator/>).
    items.push((tl!("Resize"), "move-diagonal-2", TASK_BAR_ARTBOARD_OPTIONS, json!({})));
    items.push((tl!("Duplicate"), "copy", TASK_BAR_DUPLICATE_ARTBOARDS, json!({})));
    items.push((tl!("Rearrange"), "dc-rearrange", TASK_BAR_REARRANGE_ARTBOARDS, json!({})));
    items.push((tl!("Export"), "share-2", "file.exportForScreens", json!({})));
}

/// The Contextual Task Bar's Center on Top and Center on Bottom, as in the Type menu, for type on a
/// closed path.
fn center_on_path_items(app: &VectorcraftApp, items: &mut Vec<Action>) {
    for (label, icon, id) in
        [(tl!("Center on Top"), "dc-al-top", "type.pathCenter.top"), (tl!("Center on Bottom"), "dc-al-bottom", "type.pathCenter.bottom")]
    {
        if crate::menus::enabled(app, id) {
            items.push((label, icon, id, json!({})));
        }
    }
}

/// The Contextual Task Bar's Fidelity field with the Smooth tool.
const TASK_BAR_FIDELITY: &str = "taskBar.fidelity";

/// The Contextual Task Bar's Image Trace button: a trace with the Default preset.
const TASK_BAR_IMAGE_TRACE: &str = "taskBar.imageTrace";

/// The Contextual Task Bar's Resize button with the Artboard tool: Artboard Options.
const TASK_BAR_ARTBOARD_OPTIONS: &str = "taskBar.artboardOptions";

/// The Contextual Task Bar's Duplicate button with the Artboard tool.
const TASK_BAR_DUPLICATE_ARTBOARDS: &str = "taskBar.duplicateArtboards";

/// The Contextual Task Bar's Rearrange button with the Artboard tool.
const TASK_BAR_REARRANGE_ARTBOARDS: &str = "taskBar.rearrangeArtboards";

/// Whether the task bar shows the action `cmd` as its icon alone (its label names it and is its tooltip).
fn icon_only(cmd: &str) -> bool {
    matches!(cmd, TASK_BAR_TO_CORNER | TASK_BAR_TO_SMOOTH)
}

/// The Contextual Task Bar's Convert Selected Anchor Points to Corner button, for direct-selected
/// anchors or a path selected whole with Direct Selection.
const TASK_BAR_TO_CORNER: &str = "taskBar.toCorner";

/// The Contextual Task Bar's Convert Selected Anchor Points to Smooth button, for direct-selected
/// anchors or a path selected whole with Direct Selection. The bar's Smooth button runs
/// `object.path.smooth` instead.
const TASK_BAR_TO_SMOOTH: &str = "taskBar.toSmooth";

/// The Contextual Task Bar's Unite button for two or more selected objects.
const TASK_BAR_UNITE: &str = "taskBar.unite";
