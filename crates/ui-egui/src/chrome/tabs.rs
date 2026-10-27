use super::*;

/// A document tab's title: "Name* @ 66.67 % (RGB/Preview)", or while an opacity mask is edited
/// "Name* @ 66.67 % (<Opacity Mask>/Opacity Mask)".
pub(super) fn tab_title(d: &vectorcraft_engine::DocState, zoom: f64, outline: bool, display_print_size: bool) -> String {
    let mode = if d.doc.mask_edit.is_some() {
        format!("<{0}>/{0}", tl!("Opacity Mask"))
    } else {
        let color = if d.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk { "CMYK" } else { "RGB" };
        format!("{color}/{}", if outline { tl!("Outline") } else { tl!("Preview") })
    };
    format!("{}{} @ {} ({mode})", d.title(), if d.is_dirty() { "*" } else { "" }, zoom_label(zoom, display_print_size).replace('%', " %"))
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
pub fn doc_tabs(app: &mut VectorcraftApp, ui: &mut Ui) {
    /// The width of the » button.
    const MORE: f32 = 30.0;
    let t = Tokens::get(ui.ctx());
    let (height, title_size) = if app.session.prefs.large_tabs { (44.0, 14.0) } else { (35.0, 12.5) };
    let (strip, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    ui.painter().rect_filled(strip, 0.0, t.tab_strip);
    ui.painter().line_segment([strip.left_bottom(), strip.right_bottom()], Stroke::new(1.0, t.border));
    // On the Home screen no tab is the current one.
    let active = app.session.active_index().filter(|_| app.ui.home.is_none());
    let tabs: Vec<(String, std::sync::Arc<egui::Galley>)> = app
        .session
        .documents()
        .iter()
        .enumerate()
        .map(|(i, d)| {
            let zoom = app.views.get(i).map(|v| v.zoom).unwrap_or(1.0);
            let title = tab_title(d, zoom, app.ui.view.outline, app.session.prefs.display_print_size);
            let color = if Some(i) == active { t.text_strong } else { t.text_dim };
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
    let mut hovering_art = false;
    for (i, ((_, galley), w)) in tabs.iter().zip(&widths).enumerate().skip(first) {
        if x + w > strip.left() + room && Some(i) != active {
            break;
        }
        let is_active = Some(i) == active;
        let r = egui::Rect::from_min_size(egui::pos2(x, strip.top()), vec2(*w, strip.height() - 1.0));
        let resp = ui.interact(r, ui.scope_id().with(("tab", i)), Sense::click());
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
        let xresp = ui.interact(xr.expand(3.0), ui.scope_id().with(("tabx", i)), Sense::click());
        icons::paint(ui, "x", xr, if xresp.hovered() { t.text_strong } else { t.text });
        ui.painter().galley(egui::pos2(title_left, r.center().y - galley.size().y / 2.0), galley.clone(), t.text);
        hovering_art |= crate::document_drag::hover_tab(app, &resp, i, xr.expand(3.0));
        if xresp.clicked() {
            close = Some(i);
        } else if resp.clicked() {
            activate = Some(i);
        }
        x += w;
    }
    crate::document_drag::finish_tabs(ui.ctx(), hovering_art);
    // The » button: every open document, the active one checked. The list scrolls once it
    // reaches the bottom of the window, so every document of a large session stays reachable
    // (#4017).
    if overflow {
        let more = egui::Rect::from_min_max(egui::pos2(strip.right() - MORE, strip.top()), egui::pos2(strip.right(), strip.bottom() - 1.0));
        let resp = ui.interact(more, ui.scope_id().with("tabs-more"), Sense::click());
        ui.painter().rect_filled(more, 0.0, if resp.hovered() { t.hover.gamma_multiply(0.4) } else { t.tab_strip });
        icons::paint(ui, "chevrons-right", egui::Rect::from_center_size(more.center(), vec2(14.0, 14.0)), t.text);
        let resp = resp.on_hover_text(tl!("Show all open documents"));
        egui::Popup::menu(&resp).show(|ui| {
            widgets::menu_scroll(ui, |ui| {
                for (i, (title, _)) in tabs.iter().enumerate() {
                    if widgets::menu_item_name(ui, title, true, Some(i) == active) {
                        activate = Some(i);
                        ui.close();
                    }
                }
            });
        });
    }
    if let Some(i) = close {
        if let Err(e) = crate::unsaved::close(app, i) {
            app.status(e);
        }
    } else if let Some(i) = activate {
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
}

/// The isolation mode breadcrumb bar. It is drawn over the top of `area` (the canvas), after the
/// canvas, so entering isolation mode does not change the canvas's size (#3126).
pub fn isolation_bar(app: &mut VectorcraftApp, ui: &mut Ui, area: egui::Rect) {
    let t = Tokens::get(ui.ctx());
    if let Some(st) = app.session.active()
        && let Some(iso) = st.isolation
    {
        let mut crumbs: Vec<String> =
            st.doc.ancestry(iso).unwrap_or_default().iter().filter_map(|id| st.doc.node(*id)).map(|n| n.display_name()).collect();
        // Pattern editing mode: the pattern's name, and Save a Copy / Done / Cancel at the right.
        let pattern_edit = st.doc.pattern_edit.as_ref().map(|e| e.pattern.clone());
        if let Some(pn) = &pattern_edit {
            crumbs.push(pn.clone());
        }
        let bar = egui::Rect::from_min_size(area.min, vec2(area.width(), 24.0));
        ui.interact(bar, ui.scope_id().with("iso-bar"), Sense::click_and_drag());
        ui.painter().rect_filled(bar, 0.0, t.panel);
        let back = egui::Rect::from_min_size(bar.min + vec2(6.0, 3.0), vec2(18.0, 18.0));
        let bresp = ui.interact(back, ui.scope_id().with("iso-back"), Sense::click());
        icons::paint(ui, "chevron-left", back, if bresp.hovered() { t.text } else { t.icon });
        ui.painter().text(
            bar.left_center() + vec2(30.0, 0.0),
            egui::Align2::LEFT_CENTER,
            crumbs.join("  ›  "),
            egui::FontId::proportional(12.0),
            t.text,
        );
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
                app.run("object.exitIsolation", json!({})).ok();
            }
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
            egui::ComboBox::from_id_salt("artboard-list")
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
