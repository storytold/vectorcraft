use super::bar::render_items;
use super::state::enabled;
use super::tree::{arrange_items, transform_items};
use super::*;

/// The canvas context menu (right-click): what applies to the selection, or to the view when
/// nothing is selected. Commands that can't run now are left out rather than greyed out.
pub fn context_items(app: &VectorcraftApp) -> Vec<Item> {
    use vectorcraft_doc::NodeKind;
    let Some(st) = app.session.active() else { return vec![] };
    let roots: Vec<&vectorcraft_doc::Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let any = |f: fn(&NodeKind) -> bool| roots.iter().any(|n| f(&n.kind));
    let several = roots.len() >= 2;
    let mut v = vec![c("Undo", "edit.undo"), c("Redo", "edit.redo"), Sep];
    if st.isolation.is_some() {
        v.extend([c("Exit Isolation Mode", "object.exitIsolation"), Sep]);
    }
    // Managing artboards from the canvas: with the Artboard tool, the menu locks and unlocks them
    // (`artboard.setProps` with `locked` for each artboard named; an entry with none to change is left out).
    if app.session.tool_id() == "artboard" {
        let n = st.doc.artboards.len();
        let sel = crate::panels::artboards::picked(app, n);
        let others: Vec<usize> = (0..n).filter(|i| !sel.contains(i)).collect();
        let entry = |label, idx: Vec<usize>, locked: bool| {
            let list: Vec<Value> = idx.iter().map(|i| json!({ "index": i, "locked": locked })).collect();
            (!list.is_empty()).then(|| cp(label, "artboard.setProps", json!({ "artboards": list })))
        };
        let lock: Vec<Item> =
            [entry("Lock Selected", sel.clone(), true), entry("Lock All Except Selected", others, true), entry("Lock All", (0..n).collect(), true)]
                .into_iter()
                .flatten()
                .collect();
        let unlock: Vec<Item> = [entry("Unlock Selected", sel, false), entry("Unlock All", (0..n).collect(), false)].into_iter().flatten().collect();
        for (label, items) in [("Lock Artboards", lock), ("Unlock Artboards", unlock)] {
            if !items.is_empty() {
                v.push(sub(label, items));
            }
        }
        v.push(Sep);
    }
    // The locked objects under the pointer at the right-click, each unlocked as the Layers panel's
    // lock column does. `unlock_item_label` shows each object's name.
    let unlock: Vec<Item> = app
        .ui
        .context_locked
        .iter()
        .filter(|(uid, _)| *uid == st.uid)
        .flat_map(|(_, ids)| ids)
        .filter(|id| st.doc.node(**id).is_some_and(|n| n.locked))
        .map(|id| cp("Unlock", "layer.setProps", json!({ "ids": [id.0], "locked": false })))
        .collect();
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
        v.extend([
            c("Make Guides", "view.guides.make"),
            Sep,
            sub("Transform", transform_items()),
            sub("Arrange", arrange_items()),
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
