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

/// Whether the objects `ids` (or anything inside them) are painted with a colour: a solid colour,
/// a gradient's stops or a mesh's points, which Recolor Artwork can change.
pub(crate) fn has_colors(doc: &vectorcraft_doc::Document, ids: &[vectorcraft_doc::NodeId]) -> bool {
    ids.iter().filter_map(|id| doc.node(*id)).any(|n| {
        let mut found = false;
        vectorcraft_doc::swatches::node_colors(n, &mut |_, _| found = true);
        found
    })
}

/// The bounds of the artboards the Artboard tool has selected ([`crate::panels::artboards::picked`]).
fn picked_artboards_bounds(app: &VectorcraftApp) -> Option<Rect> {
    let st = app.session.active()?;
    let picked = crate::panels::artboards::picked(app, st.doc.artboards.len());
    picked.iter().filter_map(|i| st.doc.artboards.get(*i)).map(|a| a.rect).reduce(|a, b| a.union(b))
}

/// The Contextual Task Bar: a floating pill under the selection with the most likely next actions.
/// With the Artboard tool it sits under the selected artboards and offers Resize (Artboard
/// Options), Duplicate, Rearrange All (with two or more artboards) and Export (Export for Screens).
/// Its handle drags it anywhere on the canvas; unpinned it keeps that offset as it follows the
/// selection, pinned it stays put ([`crate::state::TaskBarPlace`]).
pub(super) fn task_bar(app: &mut VectorcraftApp, ui: &mut Ui, xf: &Xf) {
    let t = Tokens::get(ui.ctx());
    let on_artboards = app.session.tool_id() == "artboard";
    if !on_artboards
        && (app.session.active().is_none_or(|st| st.selection.is_empty())
            || !matches!(app.session.tool_id(), "selection" | "directSelection" | "groupSelection"))
    {
        return;
    }
    let Some(b) = (if on_artboards { picked_artboards_bounds(app) } else { app.selection_bounds() }) else { return };
    let Some(st) = app.session.active() else { return };
    let n = st.selection.len();
    let first = st.selection.objects.first().and_then(|id| st.doc.node(*id)).cloned();
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
    let mixed = n > 1 && {
        let images = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).filter(|o| matches!(o.kind, NodeKind::Image(_))).count();
        images > 0 && images < n
    };
    // An Image Trace object: the group the engine names "Image Trace", its source image first.
    let is_trace = is_group
        && first.as_ref().is_some_and(|f| {
            f.name.as_deref() == Some("Image Trace") && f.children().is_some_and(|c| c.first().is_some_and(|i| matches!(i.kind, NodeKind::Image(_))))
        });
    let mut items: Vec<(&str, &str, &str, serde_json::Value)> = vec![]; // (label, icon, command, params)
    let none = || json!({});
    if on_artboards {
        // As in the reference app, the Artboard tool offers to resize, duplicate and export the
        // artboards (<https://community.adobe.com/questions-652/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>),
        // and to rearrange them (<https://creativepro.com/managing-artboards-in-illustrator/>).
        items.push((tl!("Resize"), "move-diagonal-2", TASK_BAR_ARTBOARD_OPTIONS, none()));
        items.push((tl!("Duplicate"), "copy", TASK_BAR_DUPLICATE_ARTBOARDS, none()));
        if st.doc.artboards.len() > 1 {
            items.push((tl!("Rearrange All"), "dc-rearrange", TASK_BAR_REARRANGE_ARTBOARDS, none()));
        }
        items.push((tl!("Export"), "share-2", "file.exportForScreens", none()));
    } else {
        // Edit Path: the Direct Selection tool, to edit the path's anchors.
        let edit_path = || (tl!("Edit Path"), "dc-direct", "tool.select", json!({"tool": "directSelection"}));
        // As in the reference app, an image offers Image Trace and Crop Image, and traced art offers
        // Expand (<https://community.adobe.com/announcements-651/contextual-task-bar-in-illustrator-2025-quick-actions-when-you-need-them-1546580>).
        if n == 1 && is_image {
            items.push((tl!("Image Trace"), "image", TASK_BAR_IMAGE_TRACE, none()));
            items.push((tl!("Crop Image"), "dc-pf-crop", "ui.cropImage", none()));
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
            items.push((tl!("Corner"), "dc-anchor", TASK_BAR_TO_CORNER, none()));
            items.push((tl!("Smooth"), "dc-anchor-smooth", TASK_BAR_TO_SMOOTH, none()));
        }
        // As in the reference app, open paths offer Join and Edit Path, compound paths and clipping
        // masks their Release, images selected with vector art Make Clipping Mask, and type converts
        // between point and area type (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/contextual-task-bar-overview.html>).
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
            // Point type converts to area type and back; type on a path doesn't.
            match kind {
                TextKind::Area { .. } => items.push((tl!("Convert To Point Type"), "type", "type.convertToPointType", none())),
                TextKind::Point => items.push((tl!("Convert To Area Type"), "dc-type-area", "type.convertToAreaType", none())),
                TextKind::OnPath { .. } => {}
            }
        } else if is_image {
            // Offset Path and Simplify don't apply to an image.
        } else if !anchors && direct {
            // Corner and Smooth convert every anchor of a path selected whole; Remove and Cut
            // need anchors picked out, so they wait for them.
            if open_path {
                items.push((tl!("Connect"), "dc-join", "path.join", none()));
            }
            items.push((tl!("Corner"), "dc-anchor", TASK_BAR_TO_CORNER, none()));
            items.push((tl!("Smooth"), "dc-anchor-smooth", TASK_BAR_TO_SMOOTH, none()));
            items.push((tl!("Simplify"), "spline", "object.path.simplify", none()));
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
    // The paints the Fill and Stroke proxies show, "?" when the selected objects' paints differ.
    let (fill, stroke) = crate::panels::current_paints(app);
    let (fill_mixed, stroke_mixed) = crate::panels::mixed_paints(app, ui.ctx());
    let colored = has_colors(&st.doc, &st.selection.objects);
    let doc = st.uid;
    // Under the selection's lowest corner on screen, centered on the selection (the view may be
    // rotated).
    let lowest = xf.quad(b).iter().map(|c| c.y).fold(f32::NEG_INFINITY, f32::max);
    let anchor = pos2(xf.to_screen(b.center()).x, lowest);
    let est_w = 150.0 + items.iter().map(|(l, _, _, _)| l.len() as f32 * 7.2 + 44.0).sum::<f32>();
    // Its place under the selection, then where the handle moved it, kept on the canvas (with the
    // bar's size last frame, the estimate before it first shows).
    let under = pos2(anchor.x - est_w / 2.0, anchor.y + 28.0);
    let id = task_bar_id();
    let size = egui::AreaState::load(ui.ctx(), id).and_then(|s| s.size).unwrap_or(vec2(est_w, 44.0));
    let bounds = xf.rect.shrink(8.0);
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
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    let (g, grip) = ui.allocate_exact_size(vec2(4.0, 28.0), Sense::drag());
                    let active = grip.hovered() || grip.dragged();
                    ui.painter().rect_filled(g.shrink2(vec2(0.5, 4.0)), CornerRadius::same(2), if active { t.text_dim } else { t.button_border });
                    if grip.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                        moved = grip.drag_delta();
                    } else if grip.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    for (label, icon, cmd, params) in &items {
                        let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), t.text_strong);
                        let (r, resp) = ui.allocate_exact_size(vec2(galley.size().x + 38.0, 30.0), Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(3), t.hover);
                        }
                        ui.painter().rect_stroke(r, CornerRadius::same(3), Stroke::new(1.0, t.button_border), StrokeKind::Inside);
                        crate::icons::paint(ui, icon, egui::Rect::from_min_size(r.min + vec2(8.0, 7.0), vec2(16.0, 16.0)), t.icon);
                        ui.painter().galley(pos2(r.left() + 30.0, r.center().y - galley.size().y / 2.0), galley, t.text_strong);
                        if resp.clicked() {
                            run = Some((*cmd, params.clone()));
                        }
                    }
                    // The selected objects' Fill, Stroke, Recolor Artwork and Lock, left out with the Artboard tool.
                    if !on_artboards {
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 30.0), Sense::click());
                        let chip = egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0));
                        widgets::proxy_chip(ui, chip, &fill, fill_mixed, None);
                        ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.button_border), StrokeKind::Outside);
                        if resp.on_hover_text(tl!("Fill")).clicked() {
                            app.session.fill_active = true;
                            show_swatches = true;
                        }
                        // The Stroke chip, drawn as a ring as the Control bar's is.
                        let (r, resp) = ui.allocate_exact_size(vec2(26.0, 30.0), Sense::click());
                        let chip = egui::Rect::from_center_size(r.center(), vec2(16.0, 16.0));
                        widgets::proxy_chip(ui, chip, &stroke, stroke_mixed, Some(4.0));
                        ui.painter().rect_stroke(chip, 0.0, Stroke::new(1.0, t.button_border), StrokeKind::Outside);
                        if resp.on_hover_text(tl!("Stroke")).clicked() {
                            app.session.fill_active = false;
                            show_swatches = true;
                        }
                        // Recolor Artwork, when the selection has colours to recolour.
                        if colored && widgets::icon_button(ui, "palette", tl!("Recolor Artwork"), false, 30.0).clicked() {
                            run = Some(("ui.recolorDialog", json!({})));
                        }
                        // The Lock button is off in isolation mode, as Object › Lock › Selection is.
                        let lockable = crate::menus::enabled(app, "object.lock");
                        if widgets::icon_button_enabled(ui, "lock", tl!("Lock (⌘2)"), false, lockable, 30.0).clicked() {
                            run = Some(("object.lock", json!({})));
                        }
                    }
                    let more = widgets::icon_button(ui, "ellipsis", tl!("More Options"), false, 30.0);
                    egui::Popup::menu(&more).show(|ui| {
                        ui.set_min_width(170.0);
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
                        ui.separator();
                        if widgets::menu_item(ui, tl!("Show Properties Panel"), true, false) {
                            run = Some(("window.panel", json!({"panel": "properties"})));
                        }
                    });
                });
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
        // Corner and Smooth convert the direct-selected anchors, as the Control bar's buttons do.
        Some((TASK_BAR_TO_CORNER, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "corner"})),
        Some((TASK_BAR_TO_SMOOTH, _)) => crate::menus::invoke(app, "path.convertAnchors", json!({"to": "smooth"})),
        Some((c, p)) => crate::menus::invoke(app, c, p),
        None => {}
    }
}

/// The Contextual Task Bar's Image Trace button: a trace with the Default preset.
const TASK_BAR_IMAGE_TRACE: &str = "taskBar.imageTrace";

/// The Contextual Task Bar's Resize button with the Artboard tool: Artboard Options.
const TASK_BAR_ARTBOARD_OPTIONS: &str = "taskBar.artboardOptions";

/// The Contextual Task Bar's Duplicate button with the Artboard tool.
const TASK_BAR_DUPLICATE_ARTBOARDS: &str = "taskBar.duplicateArtboards";

/// The Contextual Task Bar's Rearrange All button with the Artboard tool.
const TASK_BAR_REARRANGE_ARTBOARDS: &str = "taskBar.rearrangeArtboards";

/// The Contextual Task Bar's Corner button for direct-selected anchors.
const TASK_BAR_TO_CORNER: &str = "taskBar.toCorner";

/// The Contextual Task Bar's Smooth button for direct-selected anchors.
const TASK_BAR_TO_SMOOTH: &str = "taskBar.toSmooth";
