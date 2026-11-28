use super::{ROTATE_VIEW_ANGLES, Ui, VectorcraftApp, json};
use craft::behavior::IconId;
use craft::behavior::kurbo::Size;
use craft::ui::bars::{Bar, BarKind, StatusMessage, StatusProgress};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, BodyCx, Entry, Gap, LabelKind, Outcome as _, Props as _, Show as _, ShowContainer as _, SideLabel, Trigger};
use craft::ui::display::{Divider, Elide, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::Menu;
use craft::ui::pickers::{Select, SelectLook};
use craft::ui::runtime::env::Env;
use craft::ui::style::TypeRole;
use craft::ui::text::ZoomField;

/// Shows the status bar: zoom, rotation, artboard navigator, the info field with its Show menu, the readouts, Stop,
/// the background-work progress and last the message, so a narrow bar elides the message before anything else.
pub fn status_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let held = crate::canvas::held_mods(ui.ctx());
    let (info, next) = info_text(app, &held);
    if let Some(secs) = next {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs));
    }
    let print = app.print_ppi();
    let nab = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
    let cur = crate::panels::artboards::selected(app, nab);
    // PLAN: assumed Artboard::rect has `width()` and `height()`.
    let document =
        app.session.active().and_then(|d| d.doc.artboards.get(cur)).map_or(Size::new(1.0, 1.0), |a| Size::new(a.rect.width(), a.rect.height()));
    let boards: Vec<Entry<usize>> = app
        .session
        .active()
        .map(|d| d.doc.artboards.iter().enumerate().map(|(i, a)| Entry::new(i, format!("{}  {}", i + 1, a.name))).collect())
        .unwrap_or_default();
    let angles: Vec<Entry<usize>> = ROTATE_VIEW_ANGLES.iter().enumerate().map(|(i, (label, _))| Entry::new(i, *label)).collect();
    let view = app.view();
    let rotation = view.map_or(0.0, |v| v.rotation);
    let turned = ROTATE_VIEW_ANGLES.iter().position(|(_, a)| (rotation - a).abs() < 1e-9).unwrap_or(usize::MAX);
    // An angle between the presets, from the Rotate View tool or `view.rotate`, shows in whole degrees; adding 0.0
    // turns the -0 of a small negative angle into 0.
    let angle = format!("{}°", rotation.round() + 0.0);
    let shows: Vec<Entry<StatusBarShow>> =
        StatusBarShow::ALL.into_iter().map(|s| Entry::new(s, s.label()).checked(app.ui.status_bar_show == s)).collect();
    let unit = app.session.general_unit();
    let lang = crate::i18n::current();
    let render = crate::i18n::tr_ctx(lang, "diagnostics", "render");
    let ui_name = crate::i18n::tr_ctx(lang, "diagnostics", "ui");
    let busy = busy_labels(app);

    let (mut zoom_to, mut rotate_to, mut go, mut show_pick, mut stop) = (None, None, None, None, false);
    let mut invalid_zoom = None;
    let mut snapping = None;
    // Convenience controls leave first: direct toggles, then the menu. The existing timings and pointer position
    // retain their priority, and all trailing groups leave before the navigation, info field or message narrows.
    let readouts = |ui: &mut Ui, _: &mut BodyCx<'_>| {
        // With no hover point the axes stay empty and keep their width.
        let origin = app.ruler_origin();
        Row::new().gap(Gap::Item).fit_rank(4).show(ui, |ui, _| {
            for (axis, value) in [("X", app.hover_doc.map(|p| p.x - origin.x)), ("Y", app.hover_doc.map(|p| p.y - origin.y))] {
                let text = value.map(|v| format!("{axis}: {}", unit.readout(v))).unwrap_or_default();
                Text::new(text).role(TypeRole::Mono).shade(Shade::Muted).sample(format!("{axis}: -8888.88 {}", unit.suffix())).show(ui);
            }
        });
        let readout = |text: String, sample: String| Text::new(text).shade(Shade::Faint).sample(sample);
        Row::new().gap(Gap::Item).fit_rank(3).show(ui, |ui, _| {
            readout(format!("{render} {:.1} ms", app.perf.render_ms), format!("{render} 8888.8 ms")).show(ui);
            readout(format!("{ui_name} {:.1} ms", app.perf.frame_ms), format!("{ui_name} 8888.8 ms")).show(ui);
            // No rate until the window has drawn continuously for a moment (`frame_rate`).
            let fps = if app.perf.fps > 0.0 { format!("{:.0} fps", app.perf.fps) } else { "\u{2013} fps".into() };
            readout(fps, "8888 fps".into()).show(ui);
        });
        Row::new().gap(Gap::Item).fit_rank(1).show(ui, |ui, _| {
            for (icon, label, command, on) in [
                ("sparkles", tl!("Smart Guides"), "view.smartGuides", app.ui.view.smart_guides),
                ("dc-reference-point", tl!("Snap to Point"), "view.snapToPoint", app.ui.view.snap_to_point),
                ("grid-3x3", tl!("Snap to Grid"), "view.snapToGrid", app.ui.view.snap_to_grid),
            ] {
                if IconButton::new(IconId::new(icon)).label(label).selected(on).show(ui).clicked() {
                    snapping = Some((command, json!({})));
                }
            }
        });
        Row::new().fit_rank(2).show(ui, |ui, _| {
            let label = tl!("Snapping");
            let button = Button::new(label).trailing_icon(IconId::new("chevron-down")).show(ui);
            Menu::new(label).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
                snapping = super::control_bar::snapping_menu_contents(app, ui).or(snapping.take());
            });
        });
    };
    // PLAN: assumed VectorcraftApp::canvas_view() -> Option<CanvasView> (cv.S4) and the label msgid "Status Bar".
    Bar::new(tl!("Status Bar"), BarKind::Status).trailing(readouts).show(ui, |ui, _| {
        let zoom = Field::new(tl!("Zoom"))
            .side(SideLabel::new().kind(LabelKind::Hidden))
            .show(ui, |ui, _| ZoomField::new(tl!("Zoom"), app.canvas_view(), document).scrub(true).show(ui));
        // The field edits the canvas view, whose zoom is divided by UI Scaling; `view.setZoom` takes the view's zoom.
        if let Some(zoom) = zoom.inner {
            if let Some(view) = zoom.view.committed() {
                zoom_to = Some(crate::state::zoom_display_percent(crate::state::View::zoom_of_canvas(view, app.ui_scaling()), print));
            }
            invalid_zoom = zoom.invalid_zoom.map(|invalid| invalid.message(&Env::of(ui)));
        }
        Field::new(tl!("Rotation")).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            // Entries are keyed by index: an angle is not a hashable value.
            rotate_to = Select::new(tl!("Rotation"), Bind::given(turned), &angles)
                .placeholder(angle.clone())
                .look(SelectLook::Filled)
                .enabled(view.is_some())
                .show(ui)
                .events
                .committed()
                .copied();
        });
        Divider::vertical().show(ui);
        for (icon, to, tip, enabled) in
            [("chevrons_left", "first", tl!("First Artboard"), cur > 0), ("chevron_left", "previous", tl!("Previous Artboard"), cur > 0)]
        {
            if IconButton::new(IconId::new(icon)).label(tip).enabled(enabled).show(ui).clicked() {
                go = Some(json!(to));
            }
        }
        Field::new(tl!("Artboard")).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            if let Some(i) =
                Select::new(tl!("Artboard"), Bind::given(cur), &boards).look(SelectLook::Filled).enabled(nab > 0).show(ui).events.committed().copied()
            {
                go = Some(json!(i));
            }
        });
        for (icon, to, tip, enabled) in
            [("chevron_right", "next", tl!("Next Artboard"), cur + 1 < nab), ("chevrons_right", "last", tl!("Last Artboard"), cur + 1 < nab)]
        {
            if IconButton::new(IconId::new(icon)).label(tip).enabled(enabled).show(ui).clicked() {
                go = Some(json!(to));
            }
        }
        Divider::vertical().show(ui);
        // An empty readout shows a dash.
        let shown = if info.is_empty() { "\u{2013}" } else { info.as_str() };
        Text::new(shown.to_owned()).shade(Shade::Muted).elide(Elide::End).show(ui);
        let more = IconButton::new(IconId::new("chevron_down")).label(tl!("Show")).show(ui);
        show_pick = Menu::new(tl!("Show")).values().opened_by(more.response(), Trigger::Press).pick(ui, &shows).picked;
        if app.session.script_running() {
            stop = Button::new(tl!("Stop")).show(ui).response().clicked();
        }
        if let Some(job) = busy.first() {
            let label = crate::i18n::msg(job);
            let text = match busy.len() - 1 {
                0 => format!("{label}\u{2026}"),
                more => format!("{label}\u{2026} (+{more})"),
            };
            // The message's rule, so the work shows where the message was.
            Divider::vertical().show(ui);
            StatusProgress::new(text, None).show(ui);
        } else {
            let mut message = StatusMessage::new(status_message(app));
            let notes = status_notes(&app.ui);
            if !notes.is_empty() {
                message = message.tooltip(listed_notes(notes, MAX_LISTED_NOTES));
            }
            message.show(ui);
        }
    });

    if stop {
        app.session.script_interrupt().set();
    }
    if let Some((id, params)) = snapping {
        crate::menus::invoke(app, id, params);
    }
    // It fails only without a document, when there is no view to turn or zoom.
    if let Some(zoom) = zoom_to {
        app.run("view.setZoom", json!({"zoom": zoom})).ok();
    }
    if let Some(&(_, angle)) = rotate_to.and_then(|i| ROTATE_VIEW_ANGLES.get(i)) {
        app.run("view.rotate", json!({"angle": angle})).ok();
    }
    if let Some(to) = go
        && let Err(e) = app.run("view.goToArtboard", json!({ "index": to }))
    {
        app.status(e);
    }
    if let Some(show) = show_pick
        && let Err(e) = app.run("view.statusBarShow", json!({ "show": show.param() }))
    {
        app.status(e);
    }
    if let Some(message) = invalid_zoom {
        app.status(message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::Role;

    #[test]
    fn canceling_artboard_choices_keeps_the_active_artboard_and_view() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.run("artboard.new", json!({"x": 400, "y": 50, "width": 100, "height": 80, "name": "Back"})).unwrap();
        app.run("view.goToArtboard", json!({"index": 0})).unwrap();
        app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        let original = {
            let view = app.view().unwrap();
            (view.artboard, view.center, view.zoom, view.rotation)
        };
        let document = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, status_bar);
        panel.room(egui::vec2(1200.0, 300.0));
        panel.click_role("Artboard", Role::COMBO_BOX);
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(panel.app().view().unwrap().artboard, 0, "highlighting another board does not navigate");
        panel.key(egui::Key::Escape, egui::Modifiers::NONE);
        {
            let app = panel.app();
            let view = app.view().unwrap();
            assert_eq!((view.artboard, view.center, view.zoom, view.rotation), original);
            assert_eq!(app.session.active().unwrap().doc, document);
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        }
        panel.click_role("Artboard", Role::COMBO_BOX);
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        panel.key(egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(panel.app().view().unwrap().artboard, 1, "an accepted choice navigates to the board");
        assert_eq!(panel.app().session.active().unwrap().history.undo.len(), undo);
    }

    #[test]
    fn canceling_rotation_choices_keeps_an_unlisted_view_angle() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        app.run("view.rotate", json!({"angle": 17})).unwrap();
        let original = {
            let view = app.view().unwrap();
            (view.rotation, view.center, view.zoom)
        };
        let document = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let mut panel = Panel::new(app, status_bar);
        panel.room(egui::vec2(1200.0, 300.0));
        panel.click_role("Rotation", Role::COMBO_BOX);
        panel.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(panel.app().view().unwrap().rotation, 17.0, "highlighting a preset does not rotate the canvas");
        panel.key(egui::Key::Escape, egui::Modifiers::NONE);
        {
            let app = panel.app();
            let view = app.view().unwrap();
            assert_eq!((view.rotation, view.center, view.zoom), original, "Escape keeps the arbitrary opening view");
            assert_eq!(app.session.active().unwrap().doc, document);
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        }
        panel.click_role("Rotation", Role::COMBO_BOX);
        panel.click("30°");
        {
            let app = panel.app();
            let view = app.view().unwrap();
            assert_eq!(view.rotation, 30.0, "a deliberate choice rotates the view");
            assert_eq!((view.center, view.zoom), (original.1, original.2));
            assert_eq!(app.session.active().unwrap().doc, document);
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        }
    }
}

/// What the bar shows as work in progress: the running script, then background jobs, opens and places. The first
/// names the progress; the rest are counted.
fn busy_labels(app: &VectorcraftApp) -> Vec<&str> {
    app.session
        .script_running()
        .then_some("Running a script")
        .into_iter()
        .chain(app.background.jobs.iter().filter(|j| !j.label.is_empty() && !j.cancelled()).map(|j| j.label.as_str()))
        .chain(app.opening.pending.iter().map(|p| p.label.as_str()))
        .chain(app.placing.pending.iter().map(|p| p.label.as_str()))
        .collect()
}

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
/// the minute it shows ends (when the bar draws again). Current Tool names the selection tool Cmd
/// lends while it is held (`held`).
pub(super) fn info_text(app: &VectorcraftApp, held: &vectorcraft_tools::Mods) -> (String, Option<u64>) {
    let st = app.session.active();
    match app.ui.status_bar_show {
        StatusBarShow::CurrentTool => {
            let tool = vectorcraft_tools::tool_info(app.session.shown_tool_id(held))
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
            let (undos, redos) = (crate::i18n::colon(tl!("Undos")), crate::i18n::colon(tl!("Redos")));
            let text = st.map(|d| format!("{undos} {}  {redos} {}", d.history.undo.len(), d.history.redo.len()));
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

/// The status message in the interface language. While it is the error of the last script run
/// and that error's message is text the script supplied, or a plug-in run's notice
/// ([`crate::state::UiState::status_kept`]), that text is shown as it is and only the words around
/// it are translated.
pub(crate) fn status_message(app: &VectorcraftApp) -> String {
    let status = app.ui.status.as_str();
    let is = |before: &str, kept: &str, after: &str| status.strip_prefix(before).and_then(|rest| rest.strip_suffix(after)) == Some(kept);
    if let Some((before, kept, after)) = app.session.script_authored_failure()
        && is(&before, kept, &after)
    {
        return crate::i18n::msg_keeping(&before, kept, &after);
    }
    if let Some((before, kept, after)) = &app.ui.status_kept
        && is(before, kept, after)
    {
        return crate::i18n::msg_keeping(before, kept, after);
    }
    crate::i18n::msg(status)
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

/// Contextual hint for the active tool: segments of (text, bold). Keys in bold segments are
/// written as shortcuts are ("Alt+Click"); [`hint_segments`] names them for the platform.
/// The hint of the Zoom tool with Performance › Animated Zoom on.
pub(super) const ANIMATED_ZOOM_HINT: &str = "zoom.animated";
