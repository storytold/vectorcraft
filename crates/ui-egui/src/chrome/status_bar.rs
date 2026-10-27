use super::tabs::artboard_list;
use super::*;

/// Draw the status message, bounded readouts and artboard navigation controls.
pub fn status_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("status_bar")
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
                let tool = vectorcraft_tools::tool_info(app.session.tool_id())
                    .map(|t| {
                        // The catalog keys carry the full label ("Selection Tool"); only English drops the suffix.
                        let shown = tl!(t.label);
                        if crate::i18n::current() == crate::i18n::Lang::EN { shown.trim_end_matches(" Tool") } else { shown }
                    })
                    .unwrap_or("");
                ui.label(egui::RichText::new(tool).size(11.5).color(t.text_dim));
                status_readouts(app, ui, &t);
            });
        });
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
    const MESSAGE_MIN: f32 = 180.0;
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
    let busy: Vec<&str> = app
        .background
        .jobs
        .iter()
        .filter(|j| !j.label.is_empty() && !j.cancelled())
        .map(|j| j.label.as_str())
        .chain(app.opening.pending.iter().map(|p| p.label.as_str()))
        .chain(app.placing.pending.iter().map(|p| p.label.as_str()))
        .collect();
    // Reserve the spinner's space while idle too: starting a background job changes the text,
    // not its origin. Messages take priority over optional readouts in a narrow window.
    let message = if let Some(job) = busy.first() {
        let spinner = egui::Rect::from_center_size(egui::pos2(area.left() + 6.0, area.center().y), vec2(12.0, 12.0));
        ui.put(spinner, egui::Spinner::new().size(12.0).color(t.accent));
        let label = crate::i18n::msg(job);
        let more = busy.len() - 1;
        if more > 0 { format!("{label}… (+{more})") } else { format!("{label}…") }
    } else {
        crate::i18n::msg(&app.ui.status)
    };
    if !message.is_empty() {
        let rect = egui::Rect::from_min_max(egui::pos2((area.left() + 18.0).min(right), area.top()), egui::pos2(right, area.bottom()));
        status_field(ui, "message", rect, message, egui::FontId::proportional(11.0), t.text);
    }
}

/// A field never grows with its text. Clipping also bounds over-wide glyphs; hover and access
/// information retain the complete value when its on-screen label is truncated.
fn status_field(ui: &mut Ui, id: &str, rect: egui::Rect, text: String, font: egui::FontId, color: egui::Color32) {
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
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &text));
    if elided {
        response.on_hover_text(text);
    }
}

/// Contextual hint for the active tool: segments of (text, bold). Keys in bold segments are
/// written as shortcuts are ("Alt+Click"); [`hint_segments`] names them for the platform.
/// The hint of the Zoom tool with Performance › Animated Zoom on.
pub(super) const ANIMATED_ZOOM_HINT: &str = "zoom.animated";
