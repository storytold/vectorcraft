use super::tabs::artboard_list;
use super::{Arc, ROTATE_VIEW_ANGLES, Sense, Stroke, Tokens, Ui, VectorcraftApp, ZOOM_STOPS, json, theme, vec2, widgets, zoom_label};

/// Draw the status message, bounded readouts and artboard navigation controls.
pub fn status_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let panel = egui::Panel::bottom("status_bar")
        .exact_size(24.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(8, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let zoom = app.view().map(|v| v.zoom).unwrap_or(1.0);
                let print = app.session.prefs.display_print_size;
                let shown = crate::state::zoom_display_percent(zoom, print);
                ui.name_inputs(tl!("Zoom"), |ui| {
                    craft_ui::ComboBox::from_id_salt("zoom-combo")
                        .selected_text(egui::RichText::new(zoom_label(zoom, print)).size(11.5))
                        .width(76.0)
                        .show_ui(ui, |ui| {
                            for z in ZOOM_STOPS.iter().rev() {
                                if ui
                                    .selectable_label((z - shown).abs() < 0.01, zoom_label(crate::state::zoom_from_display_percent(*z, print), print))
                                    .clicked()
                                {
                                    app.run("view.setZoom", json!({"zoom": z})).ok();
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(false, tl!("Fit on Screen")).clicked() {
                                app.run("view.fitArtboard", json!({})).ok();
                            }
                            if ui.selectable_label(false, tl!("Fit All")).clicked() {
                                app.run("view.fitAll", json!({})).ok();
                            }
                        });
                });
                let rot = app.view().map(|v| v.rotation).unwrap_or(0.0);
                ui.name_inputs(tl!("Rotation"), |ui| {
                    craft_ui::ComboBox::from_id_salt("rot-combo")
                        .selected_text(egui::RichText::new(format!("{rot:.0}°")).size(11.5))
                        .width(52.0)
                        .show_ui(ui, |ui| {
                            for (label, a) in ROTATE_VIEW_ANGLES {
                                if ui.selectable_label((rot - a).abs() < 1e-9, label).clicked() {
                                    // It fails only without a document, when there is no view to turn.
                                    app.run("view.rotate", json!({"angle": a})).ok();
                                }
                            }
                        });
                });
                ui.separator();
                // The artboard navigator has First and Previous, a field with the current artboard's
                // number and name and a list of every artboard, then Next and Last.
                let nab = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
                let cur = crate::panels::artboards::selected(app, nab);
                let mut go = None;
                for (icon, to, tip) in [("chevrons-left", "first", tl!("First Artboard")), ("chevron-left", "previous", tl!("Previous Artboard"))] {
                    if widgets::icon_button_enabled(ui, icon, tip, false, cur > 0, 18.0).clicked() {
                        go = Some(json!(to));
                    }
                }
                if let Some(i) = artboard_list(app, ui, cur) {
                    go = Some(json!(i));
                }
                for (icon, to, tip) in [("chevron-right", "next", tl!("Next Artboard")), ("chevrons-right", "last", tl!("Last Artboard"))] {
                    if widgets::icon_button_enabled(ui, icon, tip, false, cur + 1 < nab, 18.0).clicked() {
                        go = Some(json!(to));
                    }
                }
                if let Some(to) = go
                    && let Err(e) = app.run("view.goToArtboard", json!({ "index": to }))
                {
                    app.status(e);
                }
                ui.separator();
                let (info, next) = info_text(app);
                if let Some(secs) = next {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs));
                }
                // The info field leaves the status message its least width and ends in an ellipsis first.
                // An empty readout shows a dash, so the field can still be clicked.
                let room = (ui.available_width() - MESSAGE_MIN - ui.spacing().item_spacing.x).max(0.0);
                let shown = if info.is_empty() { "–" } else { info.as_str() };
                let field = ui
                    .scope(|ui| {
                        ui.set_max_width(room);
                        clipped_label(ui, egui::RichText::new(shown).size(11.5).color(t.text_dim))
                    })
                    .inner
                    .interact(Sense::click());
                let mut pick = None;
                egui::Popup::menu(&field).show(|ui| {
                    ui.menu_button(tl!("Show"), |ui| {
                        for show in StatusBarShow::ALL {
                            if widgets::menu_item_name(ui, show.label(), true, app.ui.status_bar_show == show) {
                                pick = Some(show);
                                ui.close();
                            }
                        }
                    });
                });
                if let Some(show) = pick
                    && let Err(e) = app.run("view.statusBarShow", json!({ "show": show.param() }))
                {
                    app.status(e);
                }
                status_readouts(app, ui, &t);
            });
        });
    widgets::set_menu_floor(ui.ctx(), panel.response.rect.top());
}

/// The least width the status message keeps when the window is narrow.
const MESSAGE_MIN: f32 = 180.0;

/// What the status bar's info field shows, chosen from the Show menu that a click on the field
/// opens (`view.statusBarShow`). Kept with the UI state ([`crate::UiState::status_bar_show`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusBarShow {
    /// The name of the artboard the navigator shows.
    ArtboardName,
    /// The name of the tool in use.
    #[default]
    CurrentTool,
    /// The date and time.
    DateTime,
    /// How many steps the active document can undo and redo.
    Undos,
    /// The profile the active document's colors use.
    ColorProfile,
}

impl StatusBarShow {
    /// Every choice, in the Show menu's order.
    pub const ALL: [StatusBarShow; 5] =
        [StatusBarShow::ArtboardName, StatusBarShow::CurrentTool, StatusBarShow::DateTime, StatusBarShow::Undos, StatusBarShow::ColorProfile];

    /// The `show` param of `view.statusBarShow`.
    pub fn param(self) -> &'static str {
        match self {
            StatusBarShow::ArtboardName => "artboardName",
            StatusBarShow::CurrentTool => "currentTool",
            StatusBarShow::DateTime => "dateTime",
            StatusBarShow::Undos => "undos",
            StatusBarShow::ColorProfile => "colorProfile",
        }
    }

    /// The choice whose param is `param`.
    pub fn from_param(param: &str) -> Option<StatusBarShow> {
        StatusBarShow::ALL.into_iter().find(|s| s.param() == param)
    }

    /// The Show menu's label, in the interface language.
    pub fn label(self) -> &'static str {
        match self {
            StatusBarShow::ArtboardName => tl!("Artboard Name"),
            StatusBarShow::CurrentTool => tl!("Current Tool"),
            StatusBarShow::DateTime => tl!("Date and Time"),
            StatusBarShow::Undos => tl!("Number of Undos"),
            StatusBarShow::ColorProfile => tl!("Document Color Profile"),
        }
    }
}

/// Serde reader for the saved choice ([`crate::UiState::status_bar_show`]): a value this version
/// does not know reads as Current Tool.
pub fn deserialize_status_bar_show<'de, D: serde::Deserializer<'de>>(d: D) -> Result<StatusBarShow, D::Error> {
    let v = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
    Ok(v.as_str().and_then(StatusBarShow::from_param).unwrap_or_default())
}

/// `view.statusBarShow {show}`: the info field shows that readout from now on → the choice.
pub fn set_status_bar_show(app: &mut VectorcraftApp, p: &serde_json::Value) -> Result<serde_json::Value, String> {
    let key = "show";
    let Some(show) = p.get(key).and_then(serde_json::Value::as_str).and_then(StatusBarShow::from_param) else {
        let all: Vec<&str> = StatusBarShow::ALL.iter().map(|s| s.param()).collect();
        return Err(format!("`{key}` must be one of {}", all.join(", ")));
    };
    app.ui.status_bar_show = show;
    Ok(json!(show.param()))
}

/// The info field's text for the choice in the Show menu, and for Date and Time the seconds until
/// the minute it shows ends (when the bar draws again).
pub(super) fn info_text(app: &VectorcraftApp) -> (String, Option<u64>) {
    let st = app.session.active();
    match app.ui.status_bar_show {
        StatusBarShow::CurrentTool => {
            let tool = vectorcraft_tools::tool_info(app.session.tool_id())
                .map(|t| {
                    // The catalog keys carry the full label ("Selection Tool"); only English drops the suffix.
                    let shown = tl!(t.label);
                    if crate::i18n::current() == crate::i18n::Lang::EN { shown.trim_end_matches(" Tool") } else { shown }
                })
                .unwrap_or("");
            (tool.to_string(), None)
        }
        StatusBarShow::ArtboardName => {
            let name = st.and_then(|d| {
                let n = d.doc.artboards.len();
                d.doc.artboards.get(crate::panels::artboards::selected(app, n)).map(|a| a.name.clone())
            });
            (name.unwrap_or_default(), None)
        }
        StatusBarShow::DateTime => {
            let (text, next) = date_time_now();
            (text, Some(next))
        }
        StatusBarShow::Undos => {
            let text = st.map(|d| format!("{}: {}  {}: {}", tl!("Undos"), d.history.undo.len(), tl!("Redos"), d.history.redo.len()));
            (text.unwrap_or_default(), None)
        }
        StatusBarShow::ColorProfile => (st.map(|d| color_profile_name(&d.doc)).unwrap_or_default(), None),
    }
}

/// The profile `doc`'s colors use for its color mode, as the context it draws through has it
/// ([`vectorcraft_engine::cmd::colormgmt::doc_cms`]): the one assigned (Edit › Assign Profile), else
/// the working space of that mode (Edit › Color Settings), also when the assigned one is not
/// available.
pub(super) fn color_profile_name(doc: &vectorcraft_doc::Document) -> String {
    let cms = vectorcraft_engine::cmd::colormgmt::doc_cms(doc);
    let settings = cms.settings();
    let name = match doc.color_mode {
        vectorcraft_doc::ColorMode::Rgb => &settings.rgb,
        vectorcraft_doc::ColorMode::Cmyk => &settings.cmyk,
    };
    vectorcraft_color::cms::canonical_name(name).to_string()
}

/// `[year, month, day, hour, minute, second]` as the info field shows it: `2026-11-15 14:03`.
pub(super) fn date_time_label([y, mo, d, h, mi, _]: [i64; 6]) -> String {
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{mi:02}")
}

/// The local date and time ([`date_time_label`]) and the seconds until the minute ends.
#[cfg(not(target_arch = "wasm32"))]
fn date_time_now() -> (String, u64) {
    use chrono::{Datelike, Timelike};
    let now = chrono::Local::now();
    let parts = [
        i64::from(now.year()),
        i64::from(now.month()),
        i64::from(now.day()),
        i64::from(now.hour()),
        i64::from(now.minute()),
        i64::from(now.second()),
    ];
    (date_time_label(parts), 60 - u64::from(now.second().min(59)))
}

/// The web build reads no time zone, so it shows the date and time in UTC.
#[cfg(target_arch = "wasm32")]
fn date_time_now() -> (String, u64) {
    let secs = web_time::SystemTime::now().duration_since(web_time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let parts = vectorcraft_doc::metadata::civil(i64::try_from(secs).unwrap_or(i64::MAX));
    (format!("{} UTC", date_time_label(parts)), 60 - u64::try_from(parts[5].clamp(0, 59)).unwrap_or(0))
}

/// A label that ends in an ellipsis when it is wider than the room left in `ui`. Only a cut label
/// shows its full text in a tooltip; screen readers always receive the full text.
pub(super) fn clipped_label(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> egui::Response {
    let galley = text.into().into_galley(ui, Some(egui::TextWrapMode::Truncate), ui.available_width(), egui::FontSelection::Default);
    let full = galley.elided.then(|| egui::text::LayoutJob {
        sections: galley.job.sections.clone(),
        text: galley.job.text.clone(),
        language: galley.job.language.clone(),
        ..Default::default()
    });
    let response = ui.add(egui::Label::new(galley).show_tooltip_when_elided(false));
    match full {
        Some(job) => full_text_tooltip(response, job),
        None => response,
    }
}

/// Show `text` on hover in a tooltip that wraps to fit inside the window.
fn full_text_tooltip(response: egui::Response, text: impl Into<egui::WidgetText>) -> egui::Response {
    let text = text.into();
    response.on_hover_ui(|ui| {
        let margins = ui.spacing().menu_margin.sum().x + 2.0 * ui.spacing().item_spacing.x;
        ui.set_max_width(ui.spacing().tooltip_width.min(ui.ctx().content_rect().width() - margins).max(0.0));
        ui.add(egui::Label::new(text).wrap());
    })
}

/// The most recently measured status widths, scoped to this UI and display settings.
pub(super) struct StatusReadoutWidths {
    pub(super) key: (crate::i18n::Lang, vectorcraft_doc::Unit, f32),
    pub(super) values: [f32; 4],
}

/// Reuse sample-string measurements until language, document units or display scale changes.
/// Keep a single entry per UI so repeated DPI/zoom changes cannot grow the cache indefinitely.
fn status_readout_widths(ui: &Ui, lang: crate::i18n::Lang, unit: vectorcraft_doc::Unit, t: &Tokens) -> Arc<StatusReadoutWidths> {
    let id = ui.scope_id().with("status-readout-widths");
    let key = (lang, unit, ui.ctx().pixels_per_point());
    if let Some(cached) = ui.data(|data| data.get_temp::<Arc<StatusReadoutWidths>>(id))
        && cached.key == key
    {
        return cached;
    }
    // Text layout also accesses egui state, so measure outside the ui.data lock.
    let mono = theme::mono(10.5);
    let proportional = egui::FontId::proportional(10.5);
    let render = crate::i18n::tr_ctx(lang, "diagnostics", "render");
    let ui_name = crate::i18n::tr_ctx(lang, "diagnostics", "ui");
    let measure = |text, font: &egui::FontId| ui.painter().layout_no_wrap(text, font.clone(), t.text).size().x.ceil() + 4.0;
    // Reserve a readable numeric range using every digit in the active font. These samples
    // depend on the language/units, never the live values: timings cannot move field boundaries.
    let timing_width = |label| (0..=9).map(|d| measure(format!("{label} {d}{d}{d}{d}.{d} ms"), &proportional)).fold(0.0_f32, f32::max);
    let render_width = timing_width(render);
    let ui_width = timing_width(ui_name);
    let fps_width = (0..=9).map(|d| measure(format!("{d}{d}{d}{d} fps"), &proportional)).fold(0.0_f32, f32::max);
    // Each axis shows at least -9999.99..9999.99 in the selected unit without elision.
    let axis_width = (0..=9).map(|d| measure(format!("X: -{d}{d}{d}{d}.{d}{d} {}", unit.suffix()), &mono)).fold(100.0_f32, f32::max);
    let widths = Arc::new(StatusReadoutWidths { key, values: [render_width, ui_width, fps_width, axis_width] });
    ui.data_mut(|data| data.insert_temp(id, Arc::clone(&widths)));
    widths
}

/// Allocate the remaining row once, then paint each field inside its own bounded rectangle.
/// Child UIs requested with a desired width can shrink to their contents; these rectangles cannot.
pub(super) fn status_readouts(app: &VectorcraftApp, ui: &mut Ui, t: &Tokens) {
    let mono = theme::mono(10.5);
    let proportional = egui::FontId::proportional(10.5);
    let unit = app.session.general_unit();
    let lang = crate::i18n::current();
    let render = crate::i18n::tr_ctx(lang, "diagnostics", "render");
    let ui_name = crate::i18n::tr_ctx(lang, "diagnostics", "ui");
    let widths = status_readout_widths(ui, lang, unit, t);
    let [render_width, ui_width, fps_width, axis_width] = widths.values;
    let coord_width = 2.0 * axis_width;
    let perf_width = render_width + ui_width + fps_width + 12.0;
    let point_id = ui.scope_id().with(("status-hover-point", app.session.active().map(|st| st.uid)));
    if let Some(p) = app.hover_doc {
        ui.data_mut(|data| data.insert_temp(point_id, p));
    }
    let gap = ui.spacing().item_spacing.x;
    let size = vec2(ui.available_width().max(0.0), ui.available_height().max(0.0));
    let (area, _) = ui.allocate_exact_size(size, Sense::hover());
    let coordinates = area.width() >= MESSAGE_MIN + coord_width + gap;
    let diagnostics = area.width() >= MESSAGE_MIN + coord_width + perf_width + 2.0 * gap;
    let mut right = area.right();
    if diagnostics {
        let perf = egui::Rect::from_min_max(egui::pos2(right - perf_width, area.top()), area.right_bottom());
        let mut x = perf.left();
        for (id, width, text) in [
            ("render", render_width, format!("{render} {:.1} ms", app.perf.render_ms)),
            ("ui", ui_width, format!("{ui_name} {:.1} ms", app.perf.frame_ms)),
            // No rate until the window has drawn continuously for a moment (`frame_rate`).
            ("fps", fps_width, if app.perf.fps > 0.0 { format!("{:.0} fps", app.perf.fps) } else { "– fps".into() }),
        ] {
            let rect = egui::Rect::from_min_size(egui::pos2(x, area.top()), vec2(width, area.height()));
            status_field(ui, id, rect, text, proportional.clone(), t.text_disabled);
            x += width + 6.0;
        }
        right = perf.left() - gap;
    }
    if coordinates {
        let left = right - coord_width;
        // Keep the coordinate slot even without a hover point, so entering/leaving the canvas
        // cannot move or re-truncate the message. X and Y have separate, stable origins.
        let coord_rect = egui::Rect::from_min_max(egui::pos2(left, area.top()), egui::pos2(right, area.bottom()));
        // The canvas clears hover_doc when the pointer leaves. While inspecting a readout,
        // retain its last value so its tooltip can show the complete coordinate.
        let inspecting = ui.input(|i| i.pointer.hover_pos()).is_some_and(|p| coord_rect.contains(p)) && ui.rect_contains_pointer(coord_rect);
        let point = app.hover_doc.or_else(|| inspecting.then(|| ui.data(|data| data.get_temp::<vectorcraft_geom::Point>(point_id))).flatten());
        if let Some(p) = point {
            let origin = app.ruler_origin();
            for (id, x, value) in [("X", left, p.x - origin.x), ("Y", left + axis_width, p.y - origin.y)] {
                let rect = egui::Rect::from_min_size(egui::pos2(x, area.top()), vec2(axis_width, area.height()));
                status_field(ui, id, rect, format!("{id}: {}", unit.readout(value)), mono.clone(), t.text_dim);
            }
        }
        right = left - gap;
    }
    let script = app.session.script_running().then_some("Running a script");
    if script.is_some() {
        let left = (right - 52.0).max(area.left());
        let stop = egui::Rect::from_min_max(egui::pos2(left, area.top() + 1.0), egui::pos2(right, area.bottom() - 1.0));
        if ui.put(stop, egui::Button::new(tl!("Stop")).small()).clicked() {
            app.session.script_interrupt().set();
        }
        right = left - gap;
    }
    let busy: Vec<&str> = script
        .into_iter()
        .chain(app.background.jobs.iter().filter(|j| !j.label.is_empty() && !j.cancelled()).map(|j| j.label.as_str()))
        .chain(app.opening.pending.iter().map(|p| p.label.as_str()))
        .chain(app.placing.pending.iter().map(|p| p.label.as_str()))
        .collect();
    // Reserve the spinner's space while idle too: starting a background job changes the text,
    // not its origin. Messages take priority over optional readouts in a narrow window.
    let (message, notes) = if let Some(job) = busy.first() {
        let spinner = egui::Rect::from_center_size(egui::pos2(area.left() + 6.0, area.center().y), vec2(12.0, 12.0));
        ui.put(spinner, egui::Spinner::new().size(12.0).color(t.accent));
        let label = crate::i18n::msg(job);
        let more = busy.len() - 1;
        (if more > 0 { format!("{label}… (+{more})") } else { format!("{label}…") }, &[][..])
    } else {
        (crate::i18n::msg(&app.ui.status), status_notes(&app.ui))
    };
    if !message.is_empty() {
        let rect = egui::Rect::from_min_max(egui::pos2((area.left() + 18.0).min(right), area.top()), egui::pos2(right, area.bottom()));
        status_field_with_notes(ui, "message", rect, message, egui::FontId::proportional(11.0), t.text, notes);
    }
}

/// Most notes the status message's hover and access information list.
const MAX_LISTED_NOTES: usize = 30;

/// The notes behind the status message (an open's, a save's or an export's,
/// [`crate::state::UiState::status_notes`]) when there are several and the status bar still shows
/// the message they came with, else none.
fn status_notes(ui: &crate::state::UiState) -> &[String] {
    let notes = ui.status_notes.as_ref().filter(|(status, notes)| *status == ui.status && notes.len() > 1);
    notes.map(|(_, notes)| notes.as_slice()).unwrap_or_default()
}

/// `notes` in the interface language, one to a line, the first `most` of them (at most
/// [`MAX_LISTED_NOTES`]) and how many more.
pub(super) fn listed_notes(notes: &[String], most: usize) -> String {
    let lang = crate::i18n::current();
    let most = most.min(MAX_LISTED_NOTES);
    let mut lines: Vec<String> = notes.iter().take(most).map(|n| format!("• {}", crate::i18n::message(lang, n))).collect();
    if notes.len() > most {
        lines.push(format!("… (+{})", notes.len() - most));
    }
    lines.join("\n")
}

/// The hover of the status message `text` with `notes`: the message, then as many of the notes
/// ([`listed_notes`]) as fit in the window above or below the field `field`, wrapped within the
/// window's width.
fn notes_tooltip(response: egui::Response, field: egui::Rect, text: &str, notes: &[String]) -> egui::Response {
    response.on_hover_ui(|ui| {
        let screen = ui.ctx().content_rect();
        let margins = ui.spacing().menu_margin.sum() + 2.0 * ui.spacing().item_spacing;
        let width = ui.spacing().tooltip_width.min(screen.width() - margins.x).max(0.0);
        ui.set_max_width(width);
        let room = (field.top() - screen.top()).max(screen.bottom() - field.bottom()) - 2.0 * margins.y;
        let tip = |most: usize| format!("{text}\n{}", listed_notes(notes, most));
        let height =
            |s: &str| egui::WidgetText::from(s).into_galley(ui, Some(egui::TextWrapMode::Wrap), width, egui::FontSelection::Default).size().y;
        let mut most = notes.len().min(MAX_LISTED_NOTES);
        while most > 0 && height(&tip(most)) > room {
            most -= 1;
        }
        ui.add(egui::Label::new(tip(most)).wrap());
    })
}

/// A field never grows with its text. Clipping also bounds over-wide glyphs; hover and access
/// information retain the complete value when its on-screen label is truncated.
fn status_field(ui: &mut Ui, id: &str, rect: egui::Rect, text: String, font: egui::FontId, color: egui::Color32) {
    status_field_with_notes(ui, id, rect, text, font, color, &[]);
}

/// [`status_field`], whose hover, when there are `notes`, lists them ([`notes_tooltip`]) and whose
/// access information has them after the text.
fn status_field_with_notes(ui: &mut Ui, id: &str, rect: egui::Rect, text: String, font: egui::FontId, color: egui::Color32, notes: &[String]) {
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let galley = egui::WidgetText::from(egui::RichText::new(&text).font(font)).into_galley(
        ui,
        Some(egui::TextWrapMode::Truncate),
        rect.width(),
        egui::TextStyle::Body,
    );
    let at = egui::pos2(rect.left(), rect.center().y - galley.size().y / 2.0);
    let elided = galley.elided;
    ui.painter().with_clip_rect(ui.clip_rect().intersect(rect)).galley(at, galley, color);
    let response = ui.interact(rect, ui.scope_id().with(("status-field", id)), Sense::hover());
    if notes.is_empty() {
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
        if elided {
            full_text_tooltip(response, text);
        }
        return;
    }
    // The list is made only for a hover or a screen reader.
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, format!("{text}\n{}", listed_notes(notes, MAX_LISTED_NOTES))));
    if response.hovered() {
        notes_tooltip(response, rect, &text, notes);
    }
}

/// Contextual hint for the active tool: segments of (text, bold). Keys in bold segments are
/// written as shortcuts are ("Alt+Click"); [`hint_segments`] names them for the platform.
/// The hint of the Zoom tool with Performance › Animated Zoom on.
pub(super) const ANIMATED_ZOOM_HINT: &str = "zoom.animated";
