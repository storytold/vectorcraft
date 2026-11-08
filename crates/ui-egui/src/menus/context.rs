use super::bar::render_items;
use super::state::enabled;
use super::tree::{arrange_items, transform_items};
use super::{Item, Sep, Value, VectorcraftApp, c, cp, json, sub, widgets};

/// The canvas context menu (right-click): what applies to the selection, or to the view when
/// nothing is selected. Commands that can't run now are left out rather than greyed out.
pub fn context_items(app: &VectorcraftApp) -> Vec<Item> {
    use vectorcraft_doc::NodeKind;
    let Some(st) = app.session.active() else { return vec![] };
    if app.session.tool_id() == "artboard" {
        return prune(app, artboard_items(app));
    }
    let roots: Vec<&vectorcraft_doc::Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let any = |f: fn(&NodeKind) -> bool| roots.iter().any(|n| f(&n.kind));
    let several = roots.len() >= 2;
    let mut v = vec![c("Undo", "edit.undo"), c("Redo", "edit.redo"), Sep];
    if st.isolation.is_some() {
        v.extend([c("Exit Isolation Mode", "object.exitIsolation"), Sep]);
    }
    // With the Intertwine tool, where three or more objects overlap: the object nearest to the
    // right-click moves in that overlap area only (`object.intertwine.arrange`), in place of the
    // Arrange submenu, which moves objects in the whole group.
    let intertwine = app.ui.context_intertwine.filter(|(uid, ..)| *uid == st.uid && app.session.tool_id() == "intertwine");
    if let Some((_, group, at, member)) = intertwine {
        let item =
            |label, order| cp(label, "object.intertwine.arrange", json!({"id": group.0, "at": [at.x, at.y], "order": order, "member": member.0}));
        v.extend([
            item("Bring to Front", "front"),
            item("Bring Forward", "forward"),
            item("Send Backward", "backward"),
            item("Send to Back", "back"),
            Sep,
        ]);
    }
    let unlock = unlock_items(app);
    if !unlock.is_empty() {
        v.extend([sub("Unlock", unlock), Sep]);
    }
    if roots.is_empty() {
        v.extend([
            c("Paste", "edit.paste"),
            Sep,
            c("Zoom In", "view.zoomIn"),
            c("Zoom Out", "view.zoomOut"),
            c("Fit Artboard in Window", "view.fitArtboard"),
            Sep,
            c("Show Rulers", "view.rulers"),
            c("Show Grid", "view.grid"),
            c("Hide Guides", "view.guides"),
            c("Lock Guides", "view.guides.lock"),
            Sep,
            c("Select All", "select.all"),
        ]);
    } else {
        v.extend([c("Cut", "edit.cut"), c("Copy", "edit.copy"), c("Paste", "edit.paste"), Sep]);
        if let [one] = roots.as_slice()
            && one.is_container()
        {
            v.push(c("Isolate Selected Group", "object.isolate"));
        }
        if several {
            v.push(c("Group", "object.group"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: false, .. })) {
            v.push(c("Ungroup", "object.ungroup"));
        }
        let paths = any(|k| matches!(k, NodeKind::Path { .. }));
        if paths {
            v.extend([c("Join", "path.join"), c("Average…", "path.average")]);
        }
        if !st.selection.anchors.is_empty() {
            v.extend([c("Remove Anchor Points", "path.removeAnchors"), c("Cut Path at Selected Anchor Points", "path.cutAtAnchors")]);
        }
        if several {
            v.push(c("Make Clipping Mask", "object.clippingMask.make"));
        }
        if any(|k| matches!(k, NodeKind::Group { clip: true, .. })) {
            v.push(c("Release Clipping Mask", "object.clippingMask.release"));
        }
        if several && paths {
            v.push(c("Make Compound Path", "object.compoundPath.make"));
        }
        // Chooses the Objects on Path tool, whose click on a path attaches the selection to it.
        if several {
            v.push(c("Objects on Path", "object.objectsOnPath.attach"));
        }
        if any(|k| matches!(k, NodeKind::Compound { .. })) {
            v.push(c("Release Compound Path", "object.compoundPath.release"));
        }
        // The paragraphs of the selected type, or those the Type tool's selection touches.
        if crate::panels::character::text_style(app).is_some() {
            v.push(sub("Bullets and Numbering", vec![c("Apply Bullets", "type.applyBullets"), c("Apply Numbers", "type.applyNumbers")]));
        }
        v.extend([c("Make Guides", "view.guides.make"), Sep, sub("Transform", transform_items())]);
        if intertwine.is_none() {
            v.push(sub("Arrange", arrange_items()));
        }
        v.extend([
            sub(
                "Select",
                vec![c("Next Object Above", "select.nextAbove"), c("Next Object Below", "select.nextBelow"), Sep, c("Deselect", "select.none")],
            ),
            Sep,
            c("Export Selection…", "file.exportSelection"),
        ]);
    }
    prune(app, v)
}

/// The locked objects under the pointer at the right-click, each unlocked as the Layers panel's
/// lock column does. `unlock_item_label` shows each object's name.
fn unlock_items(app: &VectorcraftApp) -> Vec<Item> {
    let Some(st) = app.session.active() else { return vec![] };
    app.ui
        .context_locked
        .iter()
        .filter(|(uid, _)| *uid == st.uid)
        .flat_map(|(_, ids)| ids)
        .filter(|id| st.doc.node(**id).is_some_and(|n| n.locked))
        .map(|id| cp("Unlock", "layer.setProps", json!({ "ids": [id.0], "locked": false })))
        .collect()
}

/// The canvas context menu with the Artboard tool, in place of the entries for objects and the view: the list and order of the public
/// [Use the artboard context menu](https://helpx.adobe.com/illustrator/desktop/create-manage-artboards/organize-manage-artboards/use-artboard-context-menu.html)
/// page, with the page's Lock Artboards submenu as two, Lock Artboards and Unlock Artboards. `docs/mcp.md`
/// names the entries that act on the artboards the tool has selected. Exit Isolation Mode and the
/// Unlock submenu for locked objects under the pointer follow them.
fn artboard_items(app: &VectorcraftApp) -> Vec<Item> {
    use crate::panels::artboards as boards;
    let Some(st) = app.session.active() else { return vec![] };
    let all = &st.doc.artboards;
    let n = all.len();
    let is_locked = |i: &usize| all.get(*i).is_some_and(|a| a.locked);
    let sel: Vec<usize> = boards::picked(app, n).into_iter().filter(|i| *i < n).collect();
    let free: Vec<usize> = sel.iter().copied().filter(|i| !is_locked(i)).collect();
    let facts = boards::open_menu_facts(app);
    let mut v = vec![];
    // Cut refuses a locked artboard and every artboard of the document.
    if !sel.is_empty() && free.len() == sel.len() && sel.len() < n {
        v.push(c("Cut", "artboard.cut"));
    }
    if !sel.is_empty() {
        v.push(c("Copy", "artboard.copy"));
    }
    v.push(match facts {
        // Copied artboards go to the right-click's point, and so do copied objects.
        Some(f) => cp("Paste", "edit.paste", json!({ "center": [f.at.x, f.at.y], "artboardsAt": [f.at.x, f.at.y] })),
        None => c("Paste", "edit.paste"),
    });
    v.extend([Sep, c("Undo", "edit.undo"), c("Redo", "edit.redo"), Sep, c("New Artboard", "artboard.new")]);
    if !sel.is_empty() {
        v.extend([c("Duplicate Artboards", "artboard.duplicate"), c("Rename", "artboard.rename")]);
    }
    let deleted = boards::delete_targets(app, n);
    if !deleted.is_empty() {
        v.push(cp("Delete Artboards", "artboard.delete", json!({ "indices": deleted })));
    }
    v.extend([c("Delete Empty Artboards", "artboard.deleteEmpty"), Sep]);
    if sel.len() < n {
        v.push(c("Select All Artboards", "artboard.selectAll"));
    }
    // `artboard.setProps` with `locked` for each artboard it changes; an entry with none is left out.
    let others: Vec<usize> = (0..n).filter(|i| !sel.contains(i)).collect();
    let entry = |label, idx: Vec<usize>, locked: bool| {
        let list: Vec<Value> = idx.iter().filter(|i| is_locked(i) != locked).map(|i| json!({ "index": i, "locked": locked })).collect();
        (!list.is_empty()).then(|| cp(label, "artboard.setProps", json!({ "artboards": list })))
    };
    let lock: Vec<Item> =
        [entry("Lock Selected", sel.clone(), true), entry("Lock All Except Selected", others, true), entry("Lock All", (0..n).collect(), true)]
            .into_iter()
            .flatten()
            .collect();
    let unlock: Vec<Item> =
        [entry("Unlock Selected", sel.clone(), false), entry("Unlock All", (0..n).collect(), false)].into_iter().flatten().collect();
    for (label, items) in [("Lock Artboards", lock), ("Unlock Artboards", unlock)] {
        if !items.is_empty() {
            v.push(sub(label, items));
        }
    }
    v.push(Sep);
    if n > 1 {
        v.push(c("Rearrange", "artboard.rearrange"));
    }
    if facts.map_or_else(|| boards::fit_has_art(app, &free), |f| f.any_art) {
        v.push(c("Fit to Artwork Bounds", "artboard.fitToArt"));
    }
    if free.iter().filter_map(|i| all.get(*i)).any(|a| (a.rect.width() - a.rect.height()).abs() > 1e-6) {
        v.push(c("Switch Orientation", "artboard.switchOrientation"));
    }
    if !sel.is_empty() {
        v.extend([cp("Artboard Options", "tool.options", json!({ "tool": "artboard" })), Sep, c("Export", "file.exportForScreens")]);
    }
    if st.isolation.is_some() {
        v.extend([Sep, c("Exit Isolation Mode", "object.exitIsolation")]);
    }
    let locked_art = unlock_items(app);
    if !locked_art.is_empty() {
        v.extend([Sep, sub("Unlock", locked_art)]);
    }
    v
}

/// `items` without the commands that can't run now (Undo and Redo stay, greyed out), submenus
/// left empty, and separators that no longer separate anything.
fn prune(app: &VectorcraftApp, items: Vec<Item>) -> Vec<Item> {
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    for it in items {
        let it = match it {
            Item::Sub(l, children) => Item::Sub(l, prune(app, children)),
            it => it,
        };
        let keep = match &it {
            Item::Cmd(_, id, _) => matches!(*id, "edit.undo" | "edit.redo") || enabled(app, id),
            Item::Sub(_, children) => !children.is_empty(),
            Item::Sep => out.last().is_some_and(|l| !matches!(l, Item::Sep)),
            _ => true,
        };
        if keep {
            out.push(it);
        }
    }
    if matches!(out.last(), Some(Item::Sep)) {
        out.pop();
    }
    out
}

/// The canvas context menu's popup; the item clicked goes in `clicked`, for [`invoke`].
pub fn context_menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(200.0);
        // Its toggles already say what they do (Show Rulers, Hide Guides): no check-mark gutter.
        render_items(app, ui, &context_items(app), false, clicked);
    });
}

/// The document units a right-click on a ruler offers: each runs `document.setUnits`, exactly as
/// Preferences ▸ Units ▸ General does, with the current one checked ([`checked`]).
pub fn ruler_unit_items() -> Vec<Item> {
    vectorcraft_engine::doc::Unit::ALL.iter().map(|u| cp(u.label(), "document.setUnits", json!({ "units": u.label() }))).collect()
}

/// The ruler context menu's popup (right-click a ruler or the origin box); the unit clicked goes in
/// `clicked`, for [`invoke`].
pub fn ruler_menu_body(app: &VectorcraftApp, ui: &mut egui::Ui, clicked: &mut Option<(String, Value)>) {
    widgets::menu_scroll(ui, |ui| {
        ui.set_min_width(160.0);
        render_items(app, ui, &ruler_unit_items(), true, clicked);
    });
}
