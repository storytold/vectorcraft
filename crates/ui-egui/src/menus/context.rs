use super::state::enabled;
use super::tree::{arrange_items, transform_items};
use super::{Sep, Value, VectorcraftApp, c, cp, heading, json, sub};
use craft::menu::{Node, tidy};

/// The canvas context menu (right-click): what applies to the selection, or to the view when
/// nothing is selected. Commands that can't run now are left out rather than greyed out; a plug-in
/// command whose `when` fails for the selection is greyed out in the Plug-ins submenu.
pub fn context_items(app: &VectorcraftApp) -> Vec<Node> {
    use vectorcraft_doc::NodeKind;
    let Some(st) = app.session.active() else { return vec![] };
    if app.session.tool_id() == "artboard" {
        return prune(app, artboard_items(app));
    }
    let roots: Vec<&vectorcraft_doc::Node> = st.selection.objects.iter().filter_map(|id| st.doc.node(*id)).collect();
    let any = |f: fn(&NodeKind) -> bool| roots.iter().any(|n| f(&n.kind));
    let several = roots.len() >= 2;
    // The spelling rows stay out of `prune`: tidy drops a heading with no item after it, and No Suggestions is alone.
    let spelling = spelling_items(app);
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
        // With one group or compound path selected the menu has Isolate Selected Group, and with
        // one path selected (a member of a group or a compound path too) it has Isolate Selected
        // Path, as the help page
        // (https://web.archive.org/web/20210601162814/https://helpx.adobe.com/pdf/illustrator_reference.pdf#page=324)
        // lists them. Neither shows for an object that `object.isolate` refuses.
        if let [one] = roots.as_slice()
            && isolatable(app, &st.doc, one)
        {
            if one.is_container() {
                v.push(c("Isolate Selected Group", "object.isolate"));
            } else if matches!(one.kind, NodeKind::Path { .. }) {
                v.push(c("Isolate Selected Path", "object.isolate"));
            }
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
        // Object › Path › Simplify… and Smooth…, in that menu's order (#8344). Each opens what the
        // menu item opens (`invoke`): the Simplify slider, or the Simplify dialog when its "Open
        // this dialog with these settings next time" was ticked, and the Smooth slider. `prune`
        // leaves each out unless its command can run: the selection holds a path, in a group or a
        // compound path too.
        v.extend([c("Simplify…", "object.path.simplify"), c("Smooth…", "object.path.smooth")]);
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
            // A right-click on type turns Auto Spell Check on or off and opens Check Spelling…, as
            // Edit › Spelling does, per the public [Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)
            // page. The submenu shows Auto Spell Check's check mark.
            v.push(sub(
                "Spelling",
                vec![c("Auto Spell Check", "edit.autoSpellCheck"), c("Check Spelling…", crate::dialogs::check_spelling::COMMAND)],
            ));
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
        // The commands plug-in manifests list here (`menu.context`), each run on the selection.
        let ids: Vec<u64> = st.selection.objects.iter().map(|id| id.0).collect();
        if let Some(plugins) = crate::dialogs::plugin::canvas_submenu(&st.doc, &ids) {
            v.extend([Sep, plugins]);
        }
    }
    [spelling, prune(app, v)].concat()
}

/// A right-click on a word Auto Spell Check underlined ([`crate::state::UiState::context_spelling`]):
/// the words its dictionary suggests, each replacing it (`text.editRange`, one undo step;
/// `spelling_item_label` shows the word), or No Suggestions; then Ignore All
/// (`text.spelling.ignoreAll`) and Add to Dictionary (`text.spelling.addWord`), as the public
/// [Check and correct spelling](https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/check-and-correct-spelling.html)
/// page lists them. Nothing for another right-click, or after the document or the word changed.
fn spelling_items(app: &VectorcraftApp) -> Vec<Node> {
    let Some(st) = app.session.active() else { return vec![] };
    let Some(w) = app.ui.context_spelling.as_ref().filter(|w| w.uid == st.uid) else { return vec![] };
    let still = match st.doc.node(w.id).map(|n| &n.kind) {
        Some(vectorcraft_doc::NodeKind::Text(t)) => t.plain_text().get(w.start..w.end) == Some(w.word.as_str()),
        _ => false,
    };
    if !still {
        return vec![];
    }
    let mut v: Vec<Node> = w
        .suggestions
        .iter()
        .map(|s| cp("Suggestion", "text.editRange", json!({ "id": w.id.0, "start": w.start, "end": w.end, "insert": s })))
        .collect();
    if v.is_empty() {
        v.push(heading("No Suggestions"));
    }
    v.extend([
        Sep,
        cp("Ignore All", "text.spelling.ignoreAll", json!({ "word": w.word })),
        cp("Add to Dictionary", "text.spelling.addWord", json!({ "word": w.word })),
        Sep,
    ]);
    v
}

/// The locked objects under the pointer at the right-click, each unlocked as the Layers panel's
/// lock column does. `unlock_item_label` shows each object's name.
fn unlock_items(app: &VectorcraftApp) -> Vec<Node> {
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
fn artboard_items(app: &VectorcraftApp) -> Vec<Node> {
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
    let lock: Vec<Node> =
        [entry("Lock Selected", sel.clone(), true), entry("Lock All Except Selected", others, true), entry("Lock All", (0..n).collect(), true)]
            .into_iter()
            .flatten()
            .collect();
    let unlock: Vec<Node> =
        [entry("Unlock Selected", sel.clone(), false), entry("Unlock All", (0..n).collect(), false)].into_iter().flatten().collect();
    for (label, items) in [("Lock Artboards", lock), ("Unlock Artboards", unlock)] {
        if !items.is_empty() {
            v.push(sub(label, items));
        }
    }
    v.push(Sep);
    // Rearrange lays out the selected artboards: a click on the item opens its dialog
    // (`menus::invoke`), whose OK waits for a click on the canvas.
    if !sel.is_empty() {
        v.push(cp("Rearrange", "artboard.rearrange", json!({ "indices": sel })));
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

/// Does `object.isolate` take `n`? It and every group and layer above it are shown and unlocked
/// (for a Shaper Group, its source group, which is the group isolated), it is not on a locked
/// artboard, it is not a guide path while Lock Guides is on, and while a symbol is edited it is on
/// the symbol's editing layer. A selection set by a command can still hold an object that fails
/// this.
fn isolatable(app: &VectorcraftApp, doc: &vectorcraft_doc::Document, n: &vectorcraft_doc::Node) -> bool {
    let target = if n.extras().shaper.is_some() { n.children().and_then(|c| c.first()).map_or(n.id, |c| c.id) } else { n.id };
    doc.is_editable(target)
        && !(app.session.guides_locked() && doc.node(target).is_some_and(|t| t.is_guide()))
        && doc.symbol_edit.as_ref().is_none_or(|e| doc.ancestry(target).is_some_and(|a| a.contains(&e.layer)))
}

/// `items` without the commands that can't run now (Undo and Redo stay, greyed out, and so do Show
/// Grid and Hide Guides while Trim View holds them off: #8363), submenus left empty, and separators
/// that no longer separate anything.
fn prune(app: &VectorcraftApp, mut items: Vec<Node>) -> Vec<Node> {
    retain_runnable(app, &mut items);
    tidy(&mut items);
    items
}

/// Drops the items of `items` (and of its submenus, at every depth) whose command can't run now.
fn retain_runnable(app: &VectorcraftApp, items: &mut Vec<Node>) {
    items.retain_mut(|n| match n {
        Node::Item(i) => matches!(&*i.command.id, "edit.undo" | "edit.redo" | "view.guides" | "view.grid") || enabled(app, &i.command.id),
        Node::Submenu { children, .. } => {
            retain_runnable(app, children);
            true
        }
        _ => true,
    });
}
