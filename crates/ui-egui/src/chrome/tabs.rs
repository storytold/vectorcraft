use super::{Sense, Stroke, Tokens, Ui, VectorcraftApp, icons, json, theme, vec2, widgets, zoom_label};

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
    let number = number.map(|n| format!(":{n}")).unwrap_or_default();
    format!("{}{number}{} @ {} ({mode})", d.title(), if d.is_dirty() { "*" } else { "" }, zoom_label(zoom, display_print_size).replace('%', " %"))
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

/// The height of the tab strip and the size of a tab's title, in points: larger with User
/// Interface › Large Tabs.
fn tab_size(app: &VectorcraftApp) -> (f32, f32) {
    if app.session.prefs.large_tabs { (44.0, 14.0) } else { (35.0, 12.5) }
}

/// The tab at the top of window `w`'s floating OS window ([`crate::doc_windows`]): a strip with the
/// window's tab, as the main window's strip draws it but without its ×. → the tab's response,
/// which senses clicks and drags.
pub(crate) fn window_tab(app: &VectorcraftApp, ui: &mut Ui, w: crate::doc_windows::Win) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (height, title_size) = tab_size(app);
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    let title = document_window_title(app, w).unwrap_or_default();
    let galley = ui.painter().layout_no_wrap(title, theme::semibold(title_size), t.text_strong);
    let r = egui::Rect::from_min_size(strip.min, vec2((galley.size().x + 28.0).min(strip.width()), strip.height() - 1.0));
    ui.painter().rect_filled(r, 0.0, t.panel);
    ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.5, t.border));
    ui.painter().galley(egui::pos2(r.left() + 14.0, r.center().y - galley.size().y / 2.0), galley, t.text);
    ui.interact(r, ui.scope_id().with("window-tab"), Sense::click_and_drag())
}

/// While document tab `r` is dragged off the tab `strip` far enough to float its document, an
/// outline under the pointer where its window will open; → the window's top-left corner when the
/// drag ends there.
fn tear_off(ui: &Ui, resp: &egui::Response, strip: egui::Rect) -> Option<egui::Pos2> {
    use crate::doc_windows::{GRAB, torn_off};
    if resp.dragged()
        && let Some(p) = resp.interact_pointer_pos().filter(|p| torn_off(strip, *p))
    {
        let t = Tokens::get(ui.ctx());
        let ghost = egui::Rect::from_min_size(p - GRAB, vec2(240.0, 160.0));
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Tooltip, ui.scope_id().with("tab-tear-off")));
        painter.rect_filled(ghost, 0.0, t.accent.gamma_multiply(0.12));
        painter.rect_stroke(ghost, 0.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    if !resp.drag_stopped() {
        return None;
    }
    let p = resp.interact_pointer_pos().or_else(|| ui.input(|i| i.pointer.latest_pos()))?;
    torn_off(strip, p).then_some(p - GRAB)
}

/// The documents of the tabs the strip shows, from `slots` (each tab's document index and rect,
/// left to right): one entry per document, spanning the tabs of all its views (Window › New
/// Window), which sit side by side and move together.
pub(super) fn tab_groups(slots: &[(usize, egui::Rect)]) -> Vec<(usize, egui::Rangef)> {
    let mut groups: Vec<(usize, egui::Rangef)> = vec![];
    for &(i, r) in slots {
        match groups.last_mut() {
            Some((d, span)) if *d == i => span.max = span.max.max(r.right()),
            _ => groups.push((i, egui::Rangef::new(r.left(), r.right()))),
        }
    }
    groups
}

/// Where document `from`'s tabs, dragged along the strip to `x`, go among the other documents'
/// tabs in `groups` ([`tab_groups`]): before the first whose middle is right of `x`, else after
/// the last. → the document's new index among all the open documents (floating ones and those
/// whose tabs don't fit included) and the x of the gap the drop goes in; `None` when the drop
/// leaves the document where it is, or there is no other tab to drop beside.
pub(super) fn tab_drop(groups: &[(usize, egui::Rangef)], from: usize, x: f32) -> Option<(usize, f32)> {
    let others: Vec<&(usize, egui::Rangef)> = groups.iter().filter(|(d, _)| *d != from).collect();
    let slot = others.iter().filter(|(_, span)| span.center() < x).count();
    let (to, gap) = match others.get(slot) {
        // Before document `d`: it shifts left by one when `from` was before it.
        Some(&&(d, span)) => (if from < d { d - 1 } else { d }, span.min),
        // After the last tab shown.
        None => {
            let &&(d, span) = others.last()?;
            (if from < d { d } else { d + 1 }, span.max)
        }
    };
    (to != from).then_some((to, gap))
}

/// Whether a document tab's × comes before its title (macOS) rather than after it (Windows,
/// Linux and the web).
pub(super) const CLOSE_BEFORE_TITLE: bool = cfg!(target_os = "macos");

/// The first of tabs `widths` wide that the strip shows in `room`: the first one, or a later one so
/// that the `active` tab fits.
pub(super) fn first_tab_shown(widths: &[f32], active: Option<usize>, room: f32) -> usize {
    let Some(a) = active else { return 0 };
    let mut first = 0;
    while first < a && widths.get(first..=a).map_or(0.0, |w| w.iter().sum::<f32>()) > room {
        first += 1;
    }
    first
}

/// Document tab strip: "Name* @ 66.67% (RGB/Preview)". User Interface › Large Tabs makes the tabs
/// taller, with larger titles. Tabs that don't fit are reached from a » button at the strip's
/// right end, which lists every open document, and the active tab is always in view (#746).
/// Each view Window › New Window opened has a tab of its own. Windows floating have no tab
/// ([`crate::doc_windows`]); while the current window is a floating one, the tab of the window the
/// strip shows is the highlighted one. A tab dragged more than [`crate::doc_windows::TEAR_OFF`]
/// above or below the strip floats its view in a window under the pointer; one dragged along the
/// strip moves its document among the others (`document.move`, #5663), a line marking where it
/// will go. A middle-click on a tab closes it, as its × does (#6506).
pub fn doc_tabs(app: &mut VectorcraftApp, ui: &mut Ui) {
    /// The width of the » button.
    const MORE: f32 = 30.0;
    let t = Tokens::get(ui.ctx());
    let (height, title_size) = tab_size(app);
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    app.canvas.windows.strip = Some(strip);
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    // On the Home screen no tab is the current one.
    let shown = crate::doc_windows::tab_window(app).filter(|_| app.ui.home.is_none());
    // The windows under the tabs; `active` is the position of the highlighted one.
    let docked = crate::doc_windows::docked_windows(app);
    let active = docked.iter().position(|w| Some(*w) == shown);
    let tabs: Vec<(String, std::sync::Arc<egui::Galley>)> = docked
        .iter()
        .filter_map(|w| Some((*w, app.session.documents().get(w.0)?)))
        .map(|(w, d)| {
            let title = window_tab_title(app, w, d);
            let color = if Some(w) == shown { t.text_strong } else { t.text_dim };
            (title.clone(), ui.painter().layout_no_wrap(title, theme::semibold(title_size), color))
        })
        .collect();
    let widths: Vec<f32> = tabs.iter().map(|(_, g)| g.size().x + 50.0).collect();
    let overflow = widths.iter().sum::<f32>() > strip.width();
    let room = strip.width() - if overflow { MORE } else { 0.0 };
    let first = if overflow { first_tab_shown(&widths, active, room) } else { 0 };
    let mut x = strip.left();
    let mut activate = None;
    let mut close = None;
    let mut float = None;
    let mut hovering_art = false;
    // The tabs shown, by document and rect, and the document whose tab is dragged (and whether
    // the drag just ended), for moving it along the strip.
    let mut slots: Vec<(usize, egui::Rect)> = vec![];
    let mut dragged = None;
    for (k, ((_, galley), w)) in tabs.iter().zip(&widths).enumerate().skip(first) {
        if x + w > strip.left() + room && Some(k) != active {
            break;
        }
        let Some(&win) = docked.get(k) else { break };
        let i = win.0;
        let is_active = Some(k) == active;
        let r = egui::Rect::from_min_size(egui::pos2(x, strip.top()), vec2(*w, strip.height() - 1.0));
        let resp = ui.interact(r, ui.scope_id().with(("tab", i, win.1)), Sense::click_and_drag());
        slots.push((i, r));
        if resp.dragged() || resp.drag_stopped() {
            dragged = Some((i, resp.drag_stopped()));
        }
        if is_active {
            ui.painter().rect_filled(r, 0.0, t.panel);
        } else if resp.hovered() {
            ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.4));
        }
        ui.painter().line_segment([r.right_top(), r.right_bottom()], Stroke::new(1.5, t.border));
        // The × where each platform puts a tab's: before the title on macOS, after it elsewhere
        // (#673).
        let (x_center, title_left) = if CLOSE_BEFORE_TITLE { (r.left() + 16.0, r.left() + 32.0) } else { (r.right() - 16.0, r.left() + 14.0) };
        let xr = egui::Rect::from_center_size(egui::pos2(x_center, r.center().y), vec2(12.0, 12.0));
        let xresp = ui.interact(xr.expand(3.0), ui.scope_id().with(("tabx", i, win.1)), Sense::click());
        icons::paint(ui, "x", xr, if xresp.hovered() { t.text_strong } else { t.text });
        ui.painter().galley(egui::pos2(title_left, r.center().y - galley.size().y / 2.0), galley.clone(), t.text);
        hovering_art |= crate::document_drag::hover_tab(app, &resp, win, xr.expand(3.0));
        // A middle-click anywhere on the tab closes it as the × does (#6506), so closing a
        // modified document's last view still asks to save first.
        if xresp.clicked() || resp.middle_clicked() || xresp.middle_clicked() {
            close = Some(win);
        } else if resp.clicked() {
            activate = Some(win);
        } else if let Some(at) = tear_off(ui, &resp, strip) {
            float = Some((win, at));
        }
        x += w;
    }
    crate::document_drag::finish_tabs(ui.ctx(), hovering_art);
    // A tab dragged along the strip (not off it, which floats it) moves its document: a line marks
    // the gap it goes in, and the release moves it there. Artwork dragged over the tabs never does.
    let mut reorder = None;
    if let Some((from, released)) = dragged
        && !crate::document_drag::carrying_artwork(ui.ctx())
        && let Some(p) = ui.input(|i| i.pointer.interact_pos().or(i.pointer.latest_pos()))
        && !crate::doc_windows::torn_off(strip, p)
    {
        let drop = tab_drop(&tab_groups(&slots), from, p.x);
        if released {
            reorder = drop.map(|(to, _)| (from, to));
        } else {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
            if let Some((_, gap)) = drop {
                let gap = gap.clamp(strip.left() + 1.0, strip.right() - 1.0);
                ui.painter().vline(gap, strip.y_range().shrink(4.0), Stroke::new(2.0, t.accent));
            }
        }
    }
    // The » button: every window, floating ones included, the current one checked. The list
    // scrolls once it reaches the bottom of the window, so every document of a large session stays
    // reachable (#4017).
    if overflow {
        let current = crate::doc_windows::current(app).filter(|_| app.ui.home.is_none());
        let all = crate::doc_windows::windows(app);
        let titles: Vec<(crate::doc_windows::Win, String)> = all.iter().map(|w| (*w, document_window_title(app, *w).unwrap_or_default())).collect();
        let more = egui::Rect::from_min_max(egui::pos2(strip.right() - MORE, strip.top()), egui::pos2(strip.right(), strip.bottom() - 1.0));
        let resp = ui.interact(more, ui.scope_id().with("tabs-more"), Sense::click());
        ui.painter().rect_filled(more, 0.0, if resp.hovered() { t.hover.gamma_multiply(0.4) } else { t.tab_strip });
        icons::paint(ui, "chevrons-right", egui::Rect::from_center_size(more.center(), vec2(14.0, 14.0)), t.text);
        let resp = resp.on_hover_text(tl!("Show all open documents"));
        egui::Popup::menu(&resp).show(|ui| {
            widgets::menu_scroll(ui, |ui| {
                for (w, title) in &titles {
                    if widgets::menu_item_name(ui, title, true, Some(*w) == current) {
                        activate = Some(*w);
                        ui.close();
                    }
                }
            });
        });
    }
    if let Some(w) = close {
        if let Err(e) = crate::doc_windows::close_window(app, w) {
            app.status(e);
        }
    } else if let Some(((i, n), at)) = float {
        if let Err(e) = app.run("window.arrange.floatInWindow", json!({"index": i, "view": n, "at": [at.x, at.y]})) {
            app.status(e);
        }
    } else if let Some((from, to)) = reorder {
        if let Err(e) = app.run("document.move", json!({"from": from, "to": to})) {
            app.status(e);
        }
    } else if let Some(w) = activate {
        crate::doc_windows::show_window(app, w);
    }
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
    let t = Tokens::get(ui.ctx());
    if let Some(st) = app.session.active()
        && let Some(iso) = st.isolation
    {
        let ancestry = st.doc.ancestry(iso).unwrap_or_default();
        struct Crumb {
            id: Option<vectorcraft_doc::NodeId>,
            is_layer: bool,
            name: String,
        }
        let mut crumbs: Vec<Crumb> = ancestry
            .iter()
            .filter_map(|id| st.doc.node(*id).map(|n| Crumb { id: Some(*id), is_layer: n.is_layer(), name: n.display_name() }))
            .collect();
        // Pattern editing mode: the pattern's name, and Save a Copy / Done / Cancel at the right.
        let pattern_edit = st.doc.pattern_edit.as_ref().map(|e| e.pattern.clone());
        if let Some(pn) = &pattern_edit {
            crumbs.push(Crumb { id: None, is_layer: false, name: pn.clone() });
        }
        let bar = egui::Rect::from_min_size(area.min, vec2(area.width(), 24.0));
        // Keeps the pointer off the canvas; not focusable (`click_and_drag` is), so Tab skips it.
        ui.interact(bar, ui.scope_id().with("iso-bar"), Sense::CLICK | Sense::DRAG);
        ui.painter().rect_filled(bar, 0.0, t.panel);
        let back = egui::Rect::from_min_size(bar.min + vec2(6.0, 3.0), vec2(18.0, 18.0));
        let bresp = ui.interact(back, ui.scope_id().with("iso-back"), Sense::click());
        icons::paint(ui, "chevron-left", back, if bresp.hovered() { t.text } else { t.icon });

        let mut x = bar.left() + 30.0;
        let y_center = bar.center().y;
        let font_id = egui::FontId::proportional(12.0);
        let mut nav_cmd = None;
        let total = crumbs.len();
        for (i, crumb) in crumbs.iter().enumerate() {
            let is_leaf = i + 1 == total;
            let galley = ui.painter().layout_no_wrap(crumb.name.clone(), font_id.clone(), t.text);
            let w = galley.size().x;
            let text_pos = egui::pos2(x, y_center - galley.size().y * 0.5);
            if !is_leaf {
                let crumb_rect = egui::Rect::from_min_max(egui::pos2(x - 3.0, bar.top() + 3.0), egui::pos2(x + w + 3.0, bar.bottom() - 3.0));
                let resp = ui.interact(crumb_rect, ui.scope_id().with(("iso-crumb", i)), Sense::click());
                if resp.hovered() {
                    ui.painter().rect_filled(crumb_rect, 3.0, t.hover);
                }
                if resp.clicked() {
                    if crumb.is_layer {
                        nav_cmd = Some(("object.exitIsolation", json!({})));
                    } else if let Some(target) = crumb.id {
                        nav_cmd = Some(("object.isolate", json!({ "id": target.0 })));
                    }
                }
                ui.painter().galley(text_pos, galley, if resp.hovered() { t.text } else { t.text_dim });
                x += w + 6.0;
                let sep = ui.painter().layout_no_wrap("›".to_string(), font_id.clone(), t.text_dim);
                let sw = sep.size().x;
                ui.painter().galley(egui::pos2(x, y_center - sep.size().y * 0.5), sep, t.text_dim);
                x += sw + 6.0;
            } else {
                ui.painter().galley(text_pos, galley, t.text);
                x += w;
            }
        }
        let mut pattern_cmd = None;
        if pattern_edit.is_some() {
            let mut x = bar.right() - 6.0;
            for (label, cmd) in [("Cancel", "object.pattern.cancel"), ("Done", "object.pattern.done"), ("Save a Copy", "object.pattern.saveCopy")] {
                let label = tl!(label);
                let w = 12.0 + 7.0 * label.len() as f32;
                let r = egui::Rect::from_min_max(egui::pos2(x - w, bar.top() + 3.0), egui::pos2(x, bar.bottom() - 3.0));
                let resp = ui.interact(r, ui.scope_id().with(("pat-bar", cmd)), Sense::click());
                ui.painter().rect_filled(r, 3.0, if resp.hovered() { t.hover } else { t.panel });
                ui.painter().rect_stroke(r, 3.0, Stroke::new(1.0, t.button_border), egui::StrokeKind::Inside);
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, egui::FontId::proportional(12.0), t.text);
                if resp.clicked() {
                    pattern_cmd = Some(cmd);
                }
                x -= w + 6.0;
            }
        }
        if bresp.clicked() {
            pattern_cmd = pattern_cmd.or(pattern_edit.as_ref().map(|_| "object.pattern.done"));
            if pattern_cmd.is_none() {
                // Up one level: the isolated object's parent, or out of isolation mode from a layer.
                nav_cmd = Some(match ancestry.iter().rev().nth(1).filter(|p| st.doc.node(**p).is_some_and(|n| !n.is_layer())) {
                    Some(parent) => ("object.isolate", json!({ "id": parent.0 })),
                    None => ("object.exitIsolation", json!({})),
                });
            }
        }
        if let Some((cmd, params)) = nav_cmd {
            app.run(cmd, params).ok();
        }
        if let Some(cmd) = pattern_cmd {
            app.run(cmd, json!({})).ok();
        }
    }
}

/// Draws the artboard navigator's field, which shows the active artboard (`cur`) by number and
/// name. The field's list shows every artboard by number and name, as the Artboards panel lists
/// them, and opens with the active artboard's row in view. The list lays out only the rows in view
/// and at most one more. Returns the artboard chosen in the list.
pub(super) fn artboard_list(app: &VectorcraftApp, ui: &mut Ui, cur: usize) -> Option<usize> {
    /// The width of the field.
    const FIELD_W: f32 = 140.0;
    /// The least width of the list.
    const LIST_W: f32 = 220.0;
    /// The height of a row, as in the Artboards panel.
    const ROW_H: f32 = 24.0;
    /// The rows the list shows at once.
    const ROWS: f32 = 10.0;
    let t = Tokens::get(ui.ctx());
    let boards = app.session.active().map(|d| d.doc.artboards.as_slice()).unwrap_or_default();
    let shown = boards.get(cur).map_or_else(|| "–".to_string(), |a| format!("{}  {}", cur + 1, a.name));
    // Under `drawn`, egui's temp data stores the pass the list was last drawn in.
    let drawn = ui.scope_id().with("artboard-list");
    let height = ROW_H * ROWS;
    ui.add_enabled_ui(!boards.is_empty(), |ui| {
        ui.set_max_width(FIELD_W);
        ui.name_inputs(tl!("Artboard"), |ui| {
            craft_ui::ComboBox::from_id_salt("artboard-list")
                .selected_text(egui::RichText::new(shown).size(11.5))
                .width(FIELD_W)
                .wrap_mode(egui::TextWrapMode::Truncate)
                .height(height)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .show_ui(ui, |ui| {
                    let pass = ui.ctx().cumulative_pass_nr();
                    let last = ui.data(|d| d.get_temp::<u64>(drawn)).unwrap_or_default();
                    ui.data_mut(|d| d.insert_temp(drawn, pass));
                    let mut area = egui::ScrollArea::vertical().max_height(height);
                    // As the list opens, it scrolls the active row as close to the middle as the list allows.
                    if last + 1 < pass || last == 0 {
                        area = area.vertical_scroll_offset((cur as f32 * ROW_H - (height - ROW_H) * 0.5).max(0.0));
                    }
                    // Every number ends where the widest one ends.
                    let numbers = ui.painter().layout_no_wrap(boards.len().to_string(), egui::FontId::proportional(12.0), t.text_dim).size().x;
                    let width = ui.available_width().max(LIST_W);
                    let mut chosen = None;
                    ui.spacing_mut().item_spacing.y = 0.0;
                    area.show_rows(ui, ROW_H, boards.len(), |ui, range| {
                        for i in range {
                            let Some(board) = boards.get(i) else { continue };
                            let (r, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
                            if i == cur {
                                ui.painter().rect_filled(r, 0.0, t.row_selected);
                            } else if resp.hovered() {
                                ui.painter().rect_filled(r, 0.0, t.hover);
                            }
                            let number_end = r.left() + 8.0 + numbers;
                            ui.painter().text(
                                egui::pos2(number_end, r.center().y),
                                egui::Align2::RIGHT_CENTER,
                                (i + 1).to_string(),
                                egui::FontId::proportional(12.0),
                                t.text_dim,
                            );
                            let name_start = number_end + 10.0;
                            let name = egui::WidgetText::from(egui::RichText::new(&board.name).size(12.5)).into_galley(
                                ui,
                                Some(egui::TextWrapMode::Truncate),
                                r.right() - 8.0 - name_start,
                                egui::TextStyle::Body,
                            );
                            ui.painter().galley(egui::pos2(name_start, r.center().y - name.size().y / 2.0), name, t.text);
                            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, i == cur, format!("{}  {}", i + 1, board.name)));
                            if resp.clicked() {
                                chosen = Some(i);
                            }
                        }
                    });
                    if chosen.is_some() {
                        ui.close();
                    }
                    chosen
                })
                .inner
                .flatten()
        })
    })
    .inner
}
