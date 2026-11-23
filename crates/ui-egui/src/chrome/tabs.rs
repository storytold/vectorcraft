use super::{Ui, VectorcraftApp, json, zoom_label};
use crate::doc_windows::{self, GRAB, Win};
use crate::panel_drag::PanelDrag;
use craft::ui::bars::{Bar, BarKind, BreadcrumbBar};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::layout_components::Row;
use craft::ui::tabs::{DocumentTab, DocumentTabs, DocumentTabsEvent};

/// [`view_tab_title`] for a document with one view.
#[cfg(test)]
pub(super) fn tab_title(d: &vectorcraft_engine::DocState, zoom: f64, outline: bool, display_print_size: bool) -> String {
    view_tab_title(d, None, zoom, outline, display_print_size)
}

/// A document tab's title: "Name* @ 66.67 % (RGB/Preview)", or while an opacity mask is edited
/// "Name* @ 66.67 % (<Opacity Mask>/Opacity Mask)". For a view of a document with more than one
/// view, the view's `number` follows the name: "Name:2* @ 66.67 % (RGB/Preview)".
fn view_tab_title(d: &vectorcraft_engine::DocState, number: Option<u32>, zoom: f64, outline: bool, display_print_size: bool) -> String {
    let mode = if d.doc.mask_edit.is_some() {
        format!("<{0}>/{0}", tl!("Opacity Mask"))
    } else {
        let color = if d.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk { "CMYK" } else { "RGB" };
        format!("{color}/{}", if outline { tl!("Outline") } else { tl!("Preview") })
    };
    format!("{} @ {} ({mode})", view_tab_name(d, number), zoom_label(zoom, display_print_size).replace('%', " %"))
}

/// The start of [`view_tab_title`], which an inactive document tab shows alone (#8582): "Name*",
/// or for a view of a document with more than one view, "Name:2*".
fn view_tab_name(d: &vectorcraft_engine::DocState, number: Option<u32>) -> String {
    let number = number.map(|n| format!(":{n}")).unwrap_or_default();
    format!("{}{number}{}", d.title(), if d.is_dirty() { "*" } else { "" })
}

/// The title of window `w`, a view of document `d`: with the view's zoom and Outline or Preview,
/// and the view's number while the document has more than one view.
fn window_tab_title(app: &VectorcraftApp, w: crate::doc_windows::Win, d: &vectorcraft_engine::DocState) -> String {
    let view = crate::doc_windows::view(app, w);
    let zoom = view.map_or(1.0, |v| v.zoom);
    view_tab_title(d, crate::doc_windows::label(app, w), zoom, view.is_some_and(|v| v.outline), app.session.prefs.display_print_size)
}

/// The title of window `w`'s tab and floating window ([`crate::doc_windows`]).
pub(crate) fn document_window_title(app: &VectorcraftApp, w: crate::doc_windows::Win) -> Option<String> {
    let d = app.session.documents().get(w.0)?;
    Some(window_tab_title(app, w, d))
}

/// Runs command `id`, reporting its failure in the status line.
fn run_reporting(app: &mut VectorcraftApp, id: &str, params: serde_json::Value) {
    if let Err(e) = app.run(id, params) {
        app.status(e);
    }
}

/// The index among all open documents that dropping document `from` before the document of `before`
/// (after the last shown one when `None`) puts it at, counting the documents the strip does not show.
/// A document with several views moves as one.
pub(super) fn reorder_target(app: &VectorcraftApp, from: usize, before: Option<Win>) -> usize {
    let gap = match before {
        Some(w) => w.0,
        None => doc_windows::docked_windows(app).last().map_or(from, |w| w.0 + 1),
    };
    // Taking `from` out first shifts every later document left by one.
    if from < gap { gap - 1 } else { gap }
}

/// Document tab strip: the current window's tab shows "Name* @ 66.67 % (RGB/Preview)", each other
/// tab "Name*" alone with the whole title as its tooltip (#8582). Each view Window › New Window
/// opened has a tab of its own; floating windows have none ([`crate::doc_windows`]). A tab dragged
/// along the strip moves its document (`document.move`, #5663); one released over no strip floats
/// its view in a window; one from a floating window's strip docks here. A middle click closes a tab
/// as its × does (#6506). On the Home screen no tab is the current one.
pub fn doc_tabs(app: &mut VectorcraftApp, ui: &mut Ui) {
    let shown = doc_windows::tab_window(app).filter(|_| app.ui.home.is_none());
    let tabs: Vec<DocumentTab<Win>> = doc_windows::docked_windows(app)
        .into_iter()
        .filter_map(|w| {
            let d = app.session.documents().get(w.0)?;
            let whole = window_tab_title(app, w, d);
            let tab = if Some(w) == shown {
                DocumentTab::new(w, whole)
            } else {
                DocumentTab::new(w, view_tab_name(d, doc_windows::label(app, w))).tooltip(whole)
            };
            Some(tab.modified(d.is_dirty()))
        })
        .collect();
    let mut strip = DocumentTabs::new(tl!("Documents"), &tabs).reorderable(true).tear_off(true).large(app.session.prefs.large_tabs);
    if let Some(w) = shown {
        strip = strip.selected(w);
    }
    // The strip selects a tab on the press; the tab is shown once the press ends without moving it, as a drag along
    // the strip or off it is not a switch.
    let pressed = egui::Id::unique("vectorcraft.tabPressed");
    // Art carried off the canvas that rests over a tab shows its document at once, for the art to drop in.
    let carrying = craft::ui::runtime::drag::active::<PanelDrag>(ui.ctx()).is_some_and(|d| matches!(*d, PanelDrag::Art { .. }));
    if !carrying {
        app.canvas.carry_target = None;
    }
    let held = !carrying && ui.input(|i| i.pointer.primary_down());
    for event in strip.show(ui).events {
        if matches!(event, DocumentTabsEvent::Reordered { .. } | DocumentTabsEvent::TornOff { .. }) {
            ui.data_mut(|d| d.remove::<Win>(pressed));
        }
        match event {
            DocumentTabsEvent::Selected(w) if held => {
                ui.data_mut(|d| d.insert_temp(pressed, w));
            }
            DocumentTabsEvent::Selected(w) | DocumentTabsEvent::Picked(w) => {
                doc_windows::show_window(app, w);
                if carrying {
                    app.canvas.carry_target = app.session.active().map(|d| d.uid);
                }
            }
            DocumentTabsEvent::CloseRequested(w) => {
                if let Err(e) = doc_windows::close_window(app, w) {
                    app.status(e);
                }
            }
            DocumentTabsEvent::Reordered { documents, before, .. } => {
                if let Some(&(from, _)) = documents.first() {
                    let to = reorder_target(app, from, before);
                    if to != from {
                        run_reporting(app, "document.move", json!({"from": from, "to": to}));
                    }
                }
            }
            DocumentTabsEvent::TornOff { documents, at, .. } => {
                let at = at - GRAB;
                for (index, view) in documents {
                    run_reporting(app, "window.arrange.floatInWindow", json!({"index": index, "view": view, "at": [at.x, at.y]}));
                }
            }
            DocumentTabsEvent::DroppedInto { documents, .. } => {
                for (index, view) in documents {
                    run_reporting(app, "window.arrange.consolidateAllWindows", json!({"index": index, "view": view}));
                }
            }
            // This strip has no context menu, activation, OS file drop or empty-room double click.
            _ => {}
        }
    }
    if !held && let Some(w) = ui.data_mut(|d| d.remove_temp::<Win>(pressed)) {
        doc_windows::show_window(app, w);
    }
}

/// The tab at the top of window `w`'s floating OS window ([`crate::doc_windows`]): a strip with the
/// window's tab, without its ×. Dragged onto the main strip it docks there ([`doc_tabs`]); the
/// window moves by its title bar, so a tear-off is ignored. → the strip's response.
pub(crate) fn window_tab(app: &VectorcraftApp, ui: &mut Ui, w: Win) -> egui::Response {
    let tabs = [DocumentTab::new(w, document_window_title(app, w).unwrap_or_default()).closable(false)];
    DocumentTabs::new(tl!("Window"), &tabs).selected(w).reorderable(true).tear_off(true).large(app.session.prefs.large_tabs).show(ui).response
}

/// Makes document `i` the current one, as a click on its tab does: the Home screen closes, and
/// with the Artboard tool the artboard the document shows becomes the tool's target.
pub(crate) fn show_document(app: &mut VectorcraftApp, i: usize) {
    app.ui.home = None;
    let changed = app.session.active_index() != Some(i);
    if app.session.set_active(i)
        && changed
        && app.session.tool_id() == "artboard"
        && let Some(n) = app.session.active().map(|d| d.doc.artboards.len())
    {
        // A document switch recreates the tool; restore the displayed board as its target.
        // Clicking the current tab keeps its existing multiselection and tool options.
        let selected = crate::panels::artboards::selected(app, n);
        crate::panels::artboards::select(app, selected);
    }
}

/// `window.nextDocument` (`forward`) and `window.previousDocument`: makes the open document after
/// the current one in tab order (before it) the current one, as a click on its tab does. After the
/// last document comes the first, and before the first document comes the last. On the Home
/// screen, where no tab is the current one, the next document is the first and the previous
/// document is the last. Each view Window › New Window opened is a step of its own. Returns
/// `{index}`, with the view's number as `view` while the document has more than one view.
pub fn step_document(app: &mut VectorcraftApp, forward: bool) -> Result<serde_json::Value, String> {
    let all = crate::doc_windows::windows(app);
    let n = all.len();
    if n == 0 {
        return Err("no document open".into());
    }
    let current = crate::doc_windows::current(app).filter(|_| app.ui.home.is_none()).and_then(|c| all.iter().position(|w| *w == c));
    let k = match (current, forward) {
        (Some(a), true) => (a + 1) % n,
        (Some(a), false) => (a + n - 1) % n,
        (None, true) => 0,
        (None, false) => n - 1,
    };
    let Some(&w) = all.get(k) else { return Err("no document open".into()) };
    crate::doc_windows::show_window(app, w);
    Ok(match crate::doc_windows::label(app, w) {
        Some(view) => json!({ "index": w.0, "view": view }),
        None => json!({ "index": w.0 }),
    })
}

/// The isolation mode breadcrumb bar. It is drawn over the top of `area` (the canvas), after the
/// canvas, so entering isolation mode does not change the canvas's size (#3126).
pub fn isolation_bar(app: &mut VectorcraftApp, ui: &mut Ui, area: egui::Rect) {
    let Some(st) = app.session.active() else { return };
    let Some(iso) = st.isolation else { return };
    let ancestry = st.doc.ancestry(iso).unwrap_or_default();
    // Each crumb's node, whether it is a layer, and its name; a crumb's value in the bar is its index.
    let mut crumbs: Vec<(Option<vectorcraft_doc::NodeId>, bool, String)> =
        ancestry.iter().filter_map(|id| st.doc.node(*id).map(|n| (Some(*id), n.is_layer(), n.display_name()))).collect();
    // Pattern editing mode: the pattern's name, and Save a Copy / Done / Cancel at the right.
    let pattern_edit = st.doc.pattern_edit.as_ref().map(|e| e.pattern.clone());
    if let Some(pn) = &pattern_edit {
        crumbs.push((None, false, pn.clone()));
    }
    // Up one level: the isolated object's parent group, or `object.exitIsolation`, which goes from an
    // isolated sublayer to its parent layer and otherwise leaves isolation mode.
    let up = match ancestry.iter().rev().nth(1).filter(|p| st.doc.node(**p).is_some_and(|n| !n.is_layer())) {
        Some(parent) => ("object.isolate", json!({ "id": parent.0 })),
        None => ("object.exitIsolation", json!({})),
    };
    let entries: Vec<Entry<usize>> = crumbs.iter().enumerate().map(|(i, (_, _, name))| Entry::new(i, name.clone())).collect();
    let mut nav: Option<(&str, serde_json::Value)> = None;
    let mut pattern_cmd: Option<&str> = None;
    let mut surface_clicked = false;
    // A child of `ui` that allocates nothing in it, so the bar takes no layout space from the canvas.
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(area));
    Bar::new(tl!("Isolation mode"), BarKind::Options).show(&mut bar_ui, |ui, _| {
        Row::new().fill().senses_surface().show(ui, |ui, cx| {
            if IconButton::new(craft::behavior::IconId::new("chevron_left")).label(tl!("Back")).show(ui).clicked() {
                if pattern_edit.is_some() {
                    pattern_cmd = Some("object.pattern.done");
                } else {
                    nav = Some(up.clone());
                }
            }
            if let Some(i) = BreadcrumbBar::new(tl!("Isolation path"), &entries).show(ui).picked
                && let Some(&(Some(target), is_layer, _)) = crumbs.get(i)
            {
                // The top-level layer's crumb leaves isolation mode; a sublayer's or a group's crumb
                // isolates that level.
                nav = Some(if is_layer && i == 0 {
                    ("object.exitIsolation", json!({ "all": true }))
                } else {
                    ("object.isolate", json!({ "id": target.0 }))
                });
            }
            if pattern_edit.is_some() {
                for (label, cmd) in [("Save a Copy", "object.pattern.saveCopy"), ("Done", "object.pattern.done"), ("Cancel", "object.pattern.cancel")]
                {
                    if Button::new(tl!(label)).show(ui).clicked() {
                        pattern_cmd = Some(cmd);
                    }
                }
            }
            // A click on the bar's own surface, the current object's name included, is the background's.
            surface_clicked = cx.surface_clicked();
        });
    });
    // A click on the background leaves isolation mode altogether, symbol editing mode included.
    // While a pattern is edited it does nothing: Save a Copy, Done and Cancel end pattern editing.
    let leave = surface_clicked && nav.is_none() && pattern_cmd.is_none() && pattern_edit.is_none();
    if let Some((cmd, params)) = nav {
        app.run(cmd, params).ok();
    }
    if let Some(cmd) = pattern_cmd {
        app.run(cmd, json!({})).ok();
    }
    if leave {
        leave_isolation(app);
    }
}

/// Runs `object.exitIsolation` until the current document is out of isolation mode. Leaving symbol
/// editing mode from a group isolated inside the symbol's art first goes back to the art, and leaving
/// the mode restores the isolation it was entered from; each of these is left in turn. It stops at
/// pattern editing, which only its own buttons end, and when a step fails or changes nothing.
fn leave_isolation(app: &mut VectorcraftApp) {
    // Each step leaves one level; a document has far fewer levels than this.
    const MAX_STEPS: usize = 64;
    let state = |app: &VectorcraftApp| {
        app.session
            .active()
            .map(|d| (d.isolation, d.doc.symbol_edit.as_ref().map(|e| e.layer), d.doc.mask_edit.is_some(), d.doc.pattern_edit.is_some()))
    };
    for _ in 0..MAX_STEPS {
        let Some(before) = state(app) else { return };
        if before.0.is_none() || before.3 || app.run("object.exitIsolation", json!({})).is_err() || state(app) == Some(before) {
            return;
        }
    }
}
