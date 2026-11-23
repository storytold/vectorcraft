use super::{
    Color32, CornerRadius, NodeKind, Pos2, Rect, Sense, Stroke, StrokeKind, TextKind, Tokens, Ui, Vec2, VectorcraftApp, Xf, json, pos2, vec2, widgets,
};

/// The Contextual Task Bar's area.
fn task_bar_id() -> egui::Id {
    egui::Id::unique("task-bar")
}

/// Where the Contextual Task Bar was last drawn (none while it isn't shown).
pub(crate) fn task_bar_rect(ctx: &egui::Context) -> Option<egui::Rect> {
    let id = task_bar_id();
    ctx.memory(|m| m.areas().is_visible(&egui::LayerId::new(egui::Order::Middle, id)).then(|| m.area_rect(id)).flatten())
}

/// Whether the Contextual Task Bar, where it was last drawn, held the font controls: on a canvas
/// too narrow for them it leaves them out.
pub(crate) fn task_bar_shows_fonts(ctx: &egui::Context) -> bool {
    task_bar_rect(ctx).is_some() && ctx.data(|d| d.get_temp::<bool>(task_bar_id().with("fonts")).unwrap_or(false))
}

/// The actions and chips the Contextual Task Bar offered where it was last drawn, before Bar
/// Actions left any out, as More Options › Bar Actions lists them: each one's name, its id in
/// [`TASK_BAR_ACTIONS`] or [`TASK_BAR_CHIPS`] (none: Bar Actions cannot turn it off) and whether it
/// shows.
pub(crate) fn task_bar_offered(ctx: &egui::Context) -> Vec<(&'static str, Option<&'static str>, bool)> {
    ctx.data(|d| d.get_temp(task_bar_id().with("offered")).unwrap_or_default())
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
/// ([`vectorcraft_tools::draw2::can_smooth`]). The answer is kept in `ctx` for the document's
/// revision, which moves with every change of the document or the selection, so an unchanged
/// selection is not walked again each frame.
fn smooth_tool_on_paths(app: &VectorcraftApp, ctx: &egui::Context) -> bool {
    if app.session.tool_id() != "smooth" {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let id = task_bar_id().with("smooth");
    let key = (st.uid, st.revision);
    if let Some((k, found)) = ctx.data(|d| d.get_temp::<((u64, u64), bool)>(id))
        && k == key
    {
        return found;
    }
    let found = vectorcraft_tools::draw2::can_smooth(&st.doc, &st.selection);
    ctx.data_mut(|d| d.insert_temp(id, (key, found)));
    found
}

/// Whether the selection holds images together with other objects, for which the bar offers Make
/// Clipping Mask: at least one selected object is an image and at least one is not (a selected id
/// with no object counts as not an image). The walk stops once it has seen both. The answer is kept
/// in `ctx` for the document's revision, as [`smooth_tool_on_paths`] keeps its own.
pub(crate) fn images_with_other_art(st: &vectorcraft_engine::DocState, ctx: &egui::Context) -> bool {
    let id = task_bar_id().with("mixed");
    let key = (st.uid, st.revision);
    if let Some((k, found)) = ctx.data(|d| d.get_temp::<((u64, u64), bool)>(id))
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
    ctx.data_mut(|d| d.insert_temp(id, (key, found)));
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

/// Where the bar's top edge centers, on screen: under the lowest corner on screen of the selected
/// artboards or the selection, centered on them (the view may be rotated); on a blank canvas, which
/// has no selection, [`blank_canvas_anchor`]. None with nothing to sit under.
fn bar_anchor(app: &mut VectorcraftApp, xf: &Xf, on_artboards: bool, blank: bool) -> Option<Pos2> {
    if blank {
        return Some(blank_canvas_anchor(app, xf));
    }
    let b = if on_artboards { picked_artboards_bounds(app) } else { app.selection_bounds() }?;
    let lowest = xf.quad(b).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
    Some(pos2(xf.to_screen(b.center()).x, lowest))
}

/// Where the blank canvas bar's top edge centers, on screen: under the bottom of the active artboard
/// when part of it is in view, else the canvas's center.
fn blank_canvas_anchor(app: &VectorcraftApp, xf: &Xf) -> Pos2 {
    let board = app.session.active().and_then(|st| {
        let last = st.doc.artboards.len().checked_sub(1)?;
        st.doc.artboards.get(app.view().map_or(0, |v| v.artboard).min(last)).map(|a| a.rect)
    });
    let shown = board.map(|b| (b, xf.quad(b))).filter(|(_, q)| egui::Rect::from_points(q).intersects(xf.rect));
    match shown {
        Some((b, q)) => pos2(xf.to_screen(b.center()).x, q.iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max)),
        None => xf.rect.center(),
    }
}

/// Whether a Type tool edits type, or has type selected: its bar then shows only the text
/// formatting controls.
fn type_tool_on_type(app: &VectorcraftApp) -> bool {
    TYPE_TOOLS.contains(&app.session.tool_id()) && (crate::panels::character::text_editing(app).is_some() || only_type_selected(app))
}

/// Whether the Contextual Task Bar shows the font family, font style and Font Size controls: a
/// Type tool edits or has selected type, or a selection tool has only type selected.
pub(crate) fn task_bar_shows_type(app: &VectorcraftApp) -> bool {
    type_tool_on_type(app) || (selection_tool_with_selection(app) && only_type_selected(app))
}

/// The Contextual Task Bar: a floating pill under the selection with the most likely next actions.
/// With the Artboard tool it sits under the selected artboards and offers Resize (Artboard
/// Options), Duplicate, Rearrange All (with two or more artboards) and Export (Export for Screens).
/// A basic shape selected with the Selection tool shows a Shape Properties button in place of Lock.
/// While a basic shape tool (Rectangle, Rounded Rectangle, Ellipse, Polygon or Star) has one closed
/// path selected, as after it draws one, the bar offers Edit Path, Fill and Stroke, and Shape
/// Properties for a live rectangle, rounded rectangle, ellipse or polygon; with nothing selected, or
/// any other selection, those tools show no bar. While the Smooth tool has a path it can smooth
/// selected, the bar shows the tool's Fidelity field and Tool Options…; with no such path it shows
/// no bar. Selected type also shows the font family, font
/// style and Font Size controls before its Fill.
/// While a Type tool edits type, or has type selected, the bar shows only those controls and the
/// Fill. When one row would be wider than the canvas, the font controls take a row of their own,
/// narrowed to fit it, and the actions wrap onto rows under the handle's ([`bar_layout`]).
/// On a blank canvas, a document with no artwork and a selection tool in use, it sits under the
/// active artboard and offers Draw (Pencil) and Place Image.
/// Its handle drags it anywhere on the canvas; unpinned it keeps that offset as it follows the
/// selection, pinned it stays put ([`crate::state::TaskBarPlace`]). More Options › Bar Actions
/// turns each action and chip the bar offers on or off ([`TASK_BAR_ACTIONS`], [`TASK_BAR_CHIPS`]);
/// the bar leaves out the ones that are off, and Reset Bar Actions shows them all again. With every
/// one of them off, the bar still shows its handle and More Options, where they turn back on.
pub(super) fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let t = Tokens::get(ui.ctx());
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
    let smoothing = smooth_tool_on_paths(app, ui.ctx());
    if !(on_artboards || typing || blank || shaping || smoothing || selection_tool_with_selection(app)) {
        return;
    }
    // Selected type has the font controls too, as the help page describes
    // (<https://helpx.adobe.com/illustrator/desktop/design-with-text/fonts-and-scripts/font-options-overview.html>).
    let fonts = task_bar_shows_type(app);
    let Some(anchor) = bar_anchor(app, xf, on_artboards, blank) else { return };
    let Some(st) = app.session.active() else { return };
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
    let mixed = n > 1 && images_with_other_art(st, ui.ctx());
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
        .cloned();
    let mut items: Vec<(&str, &str, &str, serde_json::Value)> = vec![]; // (label, icon, command, params)
    let none = || json!({});
    // Edit Path: the Direct Selection tool, to edit the path's anchors.
    let edit_path = || (tl!("Edit Path"), "dc-direct", "tool.select", json!({"tool": "directSelection"}));
    if on_artboards {
        artboard_items(&mut items, st.doc.artboards.len());
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
                items.push((tl!("Unite"), "squares-unite", "object.pathfinder.unite", none()));
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
    let paints = chip_paints(app, ui.ctx());
    let colored = has_colors(&st.doc, &st.selection.objects);
    let doc = st.uid;
    // Every action and chip the bar offers here, with its name and its id in TASK_BAR_ACTIONS or
    // TASK_BAR_CHIPS (none: Bar Actions cannot turn it off), for More Options › Bar Actions; the
    // ones that are off there are left out of the bar.
    let hidden = &app.ui.task_bar_hidden_actions;
    let mut offered: Vec<(&'static str, Option<&'static str>)> =
        items.iter().map(|(label, _, cmd, params)| (*label, action_id(cmd, params))).collect();
    items.retain(|(_, _, cmd, params)| action_id(cmd, params).is_none_or(|a| !hidden.contains(a)));
    let bounds = xf.rect.shrink(8.0);
    // The canvas's width less the frame's margins and stroke.
    let room = bounds.width() - BAR_FRAME;
    // On a canvas too narrow for the type controls at their narrowest, the bar leaves them out.
    let fonts = fonts && room >= crate::panels::character::task_bar_min_room();
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
    offered.extend(chips.iter().map(|c| (c.label(shape.is_some()), Some(c.id(shape.is_some())))));
    let chips: Vec<Chip> = chips.into_iter().filter(|c| !hidden.contains(c.id(shape.is_some()))).collect();
    // Bar Actions reads this list through task_bar_offered while its menu is open.
    let offered: Vec<(&'static str, Option<&'static str>, bool)> =
        offered.into_iter().map(|(label, action)| (label, action, action.is_none_or(|a| !hidden.contains(a)))).collect();
    ui.ctx().data_mut(|d| d.insert_temp(task_bar_id().with("offered"), offered));
    let any_hidden = !hidden.is_empty();
    let widths: Vec<f32> = items.iter().map(|(label, _, cmd, _)| item_width(ui, label, cmd)).collect();
    let fidelity = app.session.tool_options()["fidelity"].as_f64();
    let font_w = fonts.then(|| {
        let w = crate::panels::character::TASK_BAR_WIDTHS;
        w.iter().sum::<f32>() + BAR_GAP * (w.len() - 1) as f32
    });
    let spacing = ui.spacing().item_spacing.y;
    let chip_widths: Vec<f32> = chips.iter().map(|c| c.width()).collect();
    let layout = bar_layout(&widths, &chip_widths, font_w, room, bar_max_rows(bounds.height(), spacing));
    // On a canvas too small for its handle and More Options: no bar, rather than one that runs off
    // the canvas.
    let fonts = layout.as_ref().is_some_and(|l| l.fonts);
    ui.ctx().data_mut(|d| d.insert_temp(task_bar_id().with("fonts"), fonts));
    let Some(layout) = layout else { return };
    let est = layout.size(spacing);
    // Its place under the selection, then where the handle moved it, kept on the canvas (with the
    // bar's size last frame, the layout's size before it first shows and whenever the layout
    // changed since, as after the canvas or the selection changed).
    let under = pos2(anchor.x - est.x / 2.0, anchor.y + 28.0);
    let id = task_bar_id();
    let unchanged = ui.ctx().data_mut(|d| {
        let last = d.get_temp::<Vec2>(id.with("layout"));
        d.insert_temp(id.with("layout"), est);
        last == Some(est)
    });
    let size = egui::AreaState::load(ui.ctx(), id).and_then(|s| s.size).filter(|_| unchanged).unwrap_or(est);
    let keep_in = |p: Pos2| p.clamp(bounds.min, (bounds.max - size).max(bounds.min));
    let place = &mut app.ui.task_bar_place;
    // Just unpinned: follow the selection from the pinned spot.
    if !place.pinned
        && let Some(at) = place.pin_at.take()
    {
        place.offset = Some((doc, xf.rect.min + at - under));
    }
    let pos = keep_in(match place.pin_at {
        Some(at) => xf.rect.min + at,
        None => under + place.offset.filter(|(d, _)| *d == doc).map_or(Vec2::ZERO, |(_, o)| o),
    });
    place.shown_at = Some(pos - xf.rect.min);
    // Pinned before it ever showed: hold it here.
    if place.pinned && place.pin_at.is_none() {
        place.pin_at = place.shown_at;
    }
    let pinned = place.pinned;
    let mut moved = Vec2::ZERO;
    let mut run: Option<(&str, serde_json::Value)> = None;
    let mut show_swatches = false;
    egui::Area::new(id).order(egui::Order::Middle).fixed_pos(pos).show(ui.ctx(), |ui| {
        egui::Frame::NONE
            .fill(t.panel)
            .stroke(Stroke::new(1.0, t.tool_active))
            .corner_radius(CornerRadius::same(5))
            .inner_margin(egui::Margin::symmetric(8, 6))
            .shadow(egui::epaint::Shadow { offset: [0, 3], blur: 10, spread: 0, color: Color32::from_black_alpha(70) })
            .show(ui, |ui| {
                // The first row holds the handle and More Options; actions that don't fit beside
                // them go on the rows after it, in order.
                for row in 0..layout.rows {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = BAR_GAP;
                        if row == 0 {
                            let (g, grip) = ui.allocate_exact_size(vec2(BAR_HANDLE, 28.0), Sense::drag());
                            let active = grip.hovered() || grip.dragged();
                            ui.painter().rect_filled(
                                g.shrink2(vec2(0.5, 4.0)),
                                CornerRadius::same(2),
                                if active { t.text_dim } else { t.button_border },
                            );
                            if grip.dragged() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                                moved = grip.drag_delta();
                            } else if grip.hovered() {
                                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                            }
                        }
                        for ((label, icon, cmd, params), at) in items.iter().zip(&layout.actions) {
                            if *at != Some(row) {
                                continue;
                            }
                            run = bar_item(ui, &t, (*label, *icon, *cmd, params), fidelity).or(run.take());
                        }
                        // Type's font family, font style and Font Size, which act as the Character
                        // panel's do: on the characters a Type tool selects, else on the selected type.
                        if row == 0 && fonts && !layout.font_row {
                            crate::panels::character::task_bar(app, ui, f32::INFINITY);
                        }
                        for (chip, at) in chips.iter().zip(&layout.chips) {
                            if *at != row {
                                continue;
                            }
                            match paint_or_object_chip(app, ui, &t, *chip, &paints, shape.as_ref()) {
                                Some(ChipClick::Fill(fill)) => {
                                    app.session.fill_active = fill;
                                    show_swatches = true;
                                }
                                Some(ChipClick::Run(cmd)) => run = Some((cmd, json!({}))),
                                None => {}
                            }
                        }
                        if row != 0 {
                            return;
                        }
                        let more = widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, BAR_MORE);
                        egui::Popup::menu(&more).show(|ui| {
                            ui.set_min_width(170.0);
                            // The actions too wide for any row of the bar.
                            let listed = items.iter().zip(&layout.actions).filter(|(_, at)| at.is_none());
                            let mut any = false;
                            for ((label, _, cmd, params), _) in listed {
                                any = true;
                                if widgets::menu_item_name(ui, label, true, false) {
                                    run = Some((*cmd, params.clone()));
                                }
                            }
                            if any {
                                ui.separator();
                            }
                            let bar = [
                                (tl!("Hide Bar"), "window.taskBar", false),
                                (tl!("Pin Bar Position"), "window.taskBar.pin", pinned),
                                (tl!("Reset Bar Position"), "window.taskBar.reset", false),
                            ];
                            for (label, cmd, checked) in bar {
                                if widgets::menu_item(ui, label, true, checked) {
                                    run = Some((cmd, json!({})));
                                }
                            }
                            // The actions and chips the bar offers for this selection or tool, each
                            // checked while it shows, which turns it on or off; then Reset Bar
                            // Actions, which shows every action again.
                            ui.menu_button(format!("   {}", tl!("Bar Actions")), |ui| {
                                ui.set_min_width(170.0);
                                for (label, action, shown) in task_bar_offered(ui.ctx()) {
                                    let Some(action) = action else { continue };
                                    if widgets::menu_item_name(ui, bar_action_name(label), true, shown) {
                                        run = Some(("window.taskBar.actions", json!({ "action": action })));
                                    }
                                }
                            });
                            if widgets::menu_item(ui, tl!("Reset Bar Actions"), any_hidden, false) {
                                run = Some(("window.taskBar.resetActions", json!({})));
                            }
                            ui.separator();
                            if widgets::menu_item(ui, tl!("Show Properties Panel"), true, false) {
                                run = Some(("window.panel", json!({"panel": "properties"})));
                            }
                        });
                    });
                }
                if layout.font_row {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = BAR_GAP;
                        crate::panels::character::task_bar(app, ui, room);
                    });
                }
            });
    });
    if moved != Vec2::ZERO {
        let to = keep_in(pos + moved);
        let place = &mut app.ui.task_bar_place;
        if place.pinned {
            place.pin_at = Some(to - xf.rect.min);
        } else {
            place.offset = Some((doc, to - under));
        }
        ui.ctx().request_repaint();
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
        // icon opens it; Duplicate for every selected artboard; Rearrange All Artboards.
        Some((TASK_BAR_ARTBOARD_OPTIONS, _)) => crate::menus::invoke(app, "tool.options", json!({"tool": "artboard"})),
        Some((TASK_BAR_DUPLICATE_ARTBOARDS, _)) => crate::panels::artboards::duplicate_picked(app),
        Some((TASK_BAR_REARRANGE_ARTBOARDS, _)) => crate::panels::artboards::open_rearrange(app),
        // The convert buttons convert the direct-selected anchors, as the Control bar's buttons do.
        Some((TASK_BAR_TO_CORNER, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "corner"})),
        Some((TASK_BAR_TO_SMOOTH, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "smooth"})),
        // Fidelity listed in More Options, on a canvas too narrow for its field: the Smooth tool's
        // Tool Options, which show it.
        Some((TASK_BAR_FIDELITY, _)) => crate::menus::invoke(app, "tool.options", json!({"tool": "smooth"})),
        Some((c, p)) => crate::menus::invoke(app, c, p),
        None => {}
    }
}

/// The width of the Contextual Task Bar's frame: its margins and stroke on both sides.
const BAR_FRAME: f32 = 18.0;
/// The gap between the bar's controls.
const BAR_GAP: f32 = 6.0;
/// The width of the bar's handle.
const BAR_HANDLE: f32 = 4.0;
/// The width of the bar's More Options button.
const BAR_MORE: f32 = 30.0;
/// The height of a row of the bar's controls.
const BAR_ROW: f32 = 30.0;

/// The controls the Contextual Task Bar shows after its actions: the Fill and Stroke chips,
/// Recolor Artwork, and Lock or a basic shape's Shape Properties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Chip {
    Fill,
    Stroke,
    Recolor,
    Lock,
}

impl Chip {
    fn width(self) -> f32 {
        match self {
            Chip::Fill | Chip::Stroke => 26.0,
            Chip::Recolor | Chip::Lock => 30.0,
        }
    }

    /// The chip's id in [`TASK_BAR_CHIPS`]; Lock is `shapeProperties` while it shows a basic
    /// `shape`'s properties.
    fn id(self, shape: bool) -> &'static str {
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
    ("unite", "object.pathfinder.unite", None),
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

/// What a click on one of the bar's [`Chip`]s does: make the Fill (`true`) or the Stroke active and
/// bring up Swatches, or run a command.
enum ChipClick {
    Fill(bool),
    Run(&'static str),
}

/// Where each of the Contextual Task Bar's controls goes.
#[derive(Debug, PartialEq)]
struct BarLayout {
    /// Each action's row (0 is the row with the handle and More Options), or `None` for an action
    /// wider than a row or past the rows the canvas's height allows, which More Options lists
    /// instead.
    actions: Vec<Option<usize>>,
    /// Each chip's row.
    chips: Vec<usize>,
    /// Whether the bar shows the font controls it was given.
    fonts: bool,
    /// Whether the font controls take a row of their own, after the others.
    font_row: bool,
    /// The number of rows before the font controls' row.
    rows: usize,
    /// The bar's width, its frame included.
    width: f32,
}

impl BarLayout {
    /// The bar's size, with `spacing` points between its rows.
    fn size(&self, spacing: f32) -> Vec2 {
        let rows = (self.rows + usize::from(self.font_row)) as f32;
        vec2(self.width, rows * BAR_ROW + (rows - 1.0) * spacing + 14.0)
    }
}

/// How many rows of the Contextual Task Bar fit in `height` points, with `spacing` points between
/// them.
fn bar_max_rows(height: f32, spacing: f32) -> usize {
    ((height - 14.0 + spacing) / (BAR_ROW + spacing)).floor().max(0.0) as usize
}

/// The Contextual Task Bar's layout in `room` points (the canvas's width less the frame) and at
/// most `max_rows` rows, for actions and chips the given widths wide, and the font controls `fonts`
/// wide when shown. One row holds everything when it fits. Otherwise the font controls take a row
/// of their own, narrowed to the room, and the actions wrap: the first row keeps the handle, the
/// chips and More Options, and each action goes on the last row when it fits there, else on a new
/// row. When the handle, the chips and More Options don't fit in one row, the chips take rows of
/// their own after the actions. An action wider than a row, or one that would need a row past
/// `max_rows`, goes in More Options; without a row to spare the font controls are left out. `None`
/// when not even the handle and More Options, and the chips' rows, fit.
fn bar_layout(actions: &[f32], chips: &[f32], fonts: Option<f32>, room: f32, max_rows: usize) -> Option<BarLayout> {
    // Puts a control `w` wide on the last row when it fits there, else on a new row.
    fn place(rows: &mut Vec<f32>, w: f32, room: f32) -> usize {
        match rows.last_mut() {
            Some(last) if *last + BAR_GAP + w <= room => *last += BAR_GAP + w,
            _ => rows.push(w),
        }
        rows.len() - 1
    }
    let ends = BAR_HANDLE + BAR_GAP + BAR_MORE;
    if room < ends || max_rows == 0 {
        return None;
    }
    let chips_w: f32 = chips.iter().map(|w| w + BAR_GAP).sum();
    let actions_w: f32 = actions.iter().map(|w| w + BAR_GAP).sum();
    let one_row = ends + chips_w + actions_w + fonts.map_or(0.0, |w| w + BAR_GAP);
    if one_row <= room {
        return Some(BarLayout {
            actions: vec![Some(0); actions.len()],
            chips: vec![0; chips.len()],
            fonts: fonts.is_some(),
            font_row: false,
            rows: 1,
            width: one_row + BAR_FRAME,
        });
    }
    let chips_first = ends + chips_w <= room;
    let mut chip_rows = vec![];
    let chips: Vec<usize> = chips.iter().map(|w| if chips_first { 0 } else { place(&mut chip_rows, *w, room) }).collect();
    if 1 + chip_rows.len() > max_rows {
        return None;
    }
    let font_row = fonts.is_some() && 1 + chip_rows.len() < max_rows;
    let action_rows = max_rows - chip_rows.len() - usize::from(font_row);
    let mut rows = vec![if chips_first { ends + chips_w } else { ends }];
    // Once an action needs a row past the last one, it and the actions after it go in More Options,
    // keeping their order.
    let mut full = false;
    let actions: Vec<Option<usize>> = actions
        .iter()
        .map(|w| {
            if *w > room || full {
                return None;
            }
            let mut next = rows.clone();
            let row = place(&mut next, *w, room);
            full = row >= action_rows;
            (!full).then(|| {
                rows = next;
                row
            })
        })
        .collect();
    let chips = chips.into_iter().map(|row| if chips_first { 0 } else { rows.len() + row }).collect();
    let font_w = if font_row { fonts.map_or(0.0, |w| w.min(room)) } else { 0.0 };
    let width = rows.iter().chain(&chip_rows).fold(font_w, |a, b| a.max(*b)) + BAR_FRAME;
    Some(BarLayout { actions, chips, fonts: font_row, font_row, rows: rows.len() + chip_rows.len(), width })
}

/// The width of the Contextual Task Bar's action button for `label`.
fn action_width(ui: &Ui, label: &str) -> f32 {
    ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), Color32::PLACEHOLDER).size().x + 38.0
}

/// The width of the Smooth tool's Fidelity on the Contextual Task Bar: its label, the gap and the
/// field.
fn fidelity_width(ui: &Ui) -> f32 {
    widgets::text_width(ui, fidelity_label(tl!("Fidelity:"), &Tokens::get(ui.ctx()))) + BAR_GAP + crate::dialogs::freehand::FIDELITY_FIELD_W
}

/// The Fidelity field's label, `label`, as the bar draws it.
fn fidelity_label(label: &str, t: &Tokens) -> egui::RichText {
    egui::RichText::new(label).size(13.0).color(t.text_strong)
}

/// The width of the Contextual Task Bar's item `label` that runs `cmd`.
fn item_width(ui: &Ui, label: &str, cmd: &str) -> f32 {
    match cmd {
        TASK_BAR_FIDELITY => fidelity_width(ui),
        _ if icon_only(cmd) => BAR_MORE,
        _ => action_width(ui, label),
    }
}

/// One of the Contextual Task Bar's items, `label` with `icon`, which runs `cmd` with `params`, and
/// the Smooth tool's `fidelity`. Returns the command to run when it was used.
fn bar_item(
    ui: &mut Ui,
    t: &Tokens,
    (label, icon, cmd, params): (&str, &str, &'static str, &serde_json::Value),
    fidelity: Option<f64>,
) -> Option<(&'static str, serde_json::Value)> {
    // The Smooth tool's Fidelity, labeled, sets the tool's option as it is typed or stepped.
    if cmd == TASK_BAR_FIDELITY {
        return fidelity_slot(ui, t, label, fidelity);
    }
    // The convert buttons show their icon alone, named by their hover text, as the Control bar's do.
    let clicked = if icon_only(cmd) { widgets::icon_button(ui, icon, label, false, BAR_MORE).clicked() } else { action_button(ui, t, label, icon) };
    clicked.then(|| (cmd, params.clone()))
}

/// The Smooth tool's Fidelity on the Contextual Task Bar, `label` and a field showing the tool's
/// `fidelity` (nothing without one). Returns the command that sets a new value typed or stepped to.
fn fidelity_slot(ui: &mut Ui, t: &Tokens, label: &str, fidelity: Option<f64>) -> Option<(&'static str, serde_json::Value)> {
    let v = fidelity?;
    let x = fidelity_field(ui, t, label, v).filter(|x| *x != v)?;
    Some(("tool.setOption", json!({"tool": "smooth", "values": {"fidelity": x}})))
}

/// The Smooth tool's Fidelity on the Contextual Task Bar: `label` and a field showing `v`, centered
/// in a row of the bar. Returns the value typed or stepped to.
fn fidelity_field(ui: &mut Ui, t: &Tokens, label: &str, v: f64) -> Option<f64> {
    ui.allocate_ui_with_layout(vec2(fidelity_width(ui), BAR_ROW), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        widgets::field_label(ui, fidelity_label(label, t));
        crate::dialogs::freehand::fidelity_field(ui, "task-bar-fidelity", v, "smooth")
    })
    .inner
}

/// One of the Contextual Task Bar's action buttons: `icon` and `label`, named for screen readers.
/// Returns whether it was clicked.
fn action_button(ui: &mut Ui, t: &Tokens, label: &str, icon: &str) -> bool {
    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), t.text_strong);
    let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 38.0, BAR_ROW), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(3), t.hover);
    }
    ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.button_border), StrokeKind::Inside);
    crate::icons::paint(ui, icon, egui::Rect::from_min_size(r.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), t.icon);
    ui.painter().galley(pos2(r.left() + 30.0, r.center().y - galley.size().y / 2.0), galley, t.text_strong);
    resp.clicked()
}

/// One of the Contextual Task Bar's chips, with the Fill and Stroke `paints` ([`chip_paints`]) and
/// the basic `shape` whose properties take Lock's place. Returns what a click on it does.
fn paint_or_object_chip(
    app: &mut VectorcraftApp,
    ui: &mut Ui,
    t: &Tokens,
    chip: Chip,
    paints: &(vectorcraft_color::Paint, vectorcraft_color::Paint, bool, bool),
    shape: Option<&vectorcraft_doc::Node>,
) -> Option<ChipClick> {
    let (fill, stroke, fill_mixed, stroke_mixed) = paints;
    match chip {
        Chip::Fill => paint_chip(ui, t, fill, *fill_mixed, None, tl!("Fill")).then_some(ChipClick::Fill(true)),
        // The Stroke chip, drawn as a ring as the Control bar's is.
        Chip::Stroke => paint_chip(ui, t, stroke, *stroke_mixed, Some(4.0), tl!("Stroke")).then_some(ChipClick::Fill(false)),
        Chip::Recolor => {
            widgets::icon_button(ui, "palette", tl!("Recolor Artwork"), false, 30.0).clicked().then_some(ChipClick::Run("ui.recolorDialog"))
        }
        Chip::Lock => {
            if let Some(shape) = shape
                && let NodeKind::Path { live: Some(live), .. } = &shape.kind
            {
                // The shape's properties in a popover that stays open while its fields and its
                // Corner dropdown are used.
                let props = widgets::icon_button(ui, "shapes", tl!("Shape Properties"), false, 30.0);
                widgets::popover(&props, props.clicked(), |ui| {
                    ui.set_min_width(250.0);
                    crate::panels::transform::live_shape_rows(app, ui, shape, live, "task-bar-shape", true);
                });
                None
            } else {
                // The Lock button is off in isolation mode, as Object › Lock › Selection is.
                let lockable = crate::menus::enabled(app, "object.lock");
                widgets::icon_button_enabled(ui, "lock", tl!("Lock (⌘2)"), false, lockable, 30.0).clicked().then_some(ChipClick::Run("object.lock"))
            }
        }
    }
}

/// Whether the bar's item that runs `cmd` shows its icon alone: the anchor convert buttons.
fn icon_only(cmd: &str) -> bool {
    matches!(cmd, TASK_BAR_TO_CORNER | TASK_BAR_TO_SMOOTH)
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

/// One of the Contextual Task Bar's Fill and Stroke chips, showing `paint` ("?" when `mixed`, a
/// ring of `ring` points for the stroke), named `tip` for screen readers. Returns whether it was
/// clicked.
fn paint_chip(ui: &mut Ui, t: &Tokens, paint: &vectorcraft_color::Paint, mixed: bool, ring: Option<f32>, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, 30.0), Sense::click());
    let chip = egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0));
    widgets::proxy_chip(ui, chip, paint, mixed, ring);
    ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.button_border), StrokeKind::Outside);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip).clicked()
}

/// The Contextual Task Bar's items with the Artboard tool in a document of `artboards` artboards:
/// Resize, Duplicate, Rearrange All (with two or more) and Export.
fn artboard_items(items: &mut Vec<(&'static str, &'static str, &'static str, serde_json::Value)>, artboards: usize) {
    // As in the reference app, the Artboard tool offers to resize, duplicate and export the
    // artboards (<https://community.adobe.com/questions-652/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>),
    // and to rearrange them (<https://creativepro.com/managing-artboards-in-illustrator/>).
    items.push((tl!("Resize"), "move-diagonal-2", TASK_BAR_ARTBOARD_OPTIONS, json!({})));
    items.push((tl!("Duplicate"), "copy", TASK_BAR_DUPLICATE_ARTBOARDS, json!({})));
    if artboards > 1 {
        items.push((tl!("Rearrange All"), "dc-rearrange", TASK_BAR_REARRANGE_ARTBOARDS, json!({})));
    }
    items.push((tl!("Export"), "share-2", "file.exportForScreens", json!({})));
}

/// The Contextual Task Bar's Center on Top and Center on Bottom, as in the Type menu, for type on a
/// closed path.
fn center_on_path_items(app: &VectorcraftApp, items: &mut Vec<(&'static str, &'static str, &'static str, serde_json::Value)>) {
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

/// The Contextual Task Bar's Rearrange All button with the Artboard tool.
const TASK_BAR_REARRANGE_ARTBOARDS: &str = "taskBar.rearrangeArtboards";

/// The Contextual Task Bar's Convert Selected Anchor Points to Corner button, for direct-selected
/// anchors or a path selected whole with Direct Selection.
const TASK_BAR_TO_CORNER: &str = "taskBar.toCorner";

/// The Contextual Task Bar's Convert Selected Anchor Points to Smooth button, for direct-selected
/// anchors or a path selected whole with Direct Selection. The bar's Smooth button runs
/// `object.path.smooth` instead.
const TASK_BAR_TO_SMOOTH: &str = "taskBar.toSmooth";
