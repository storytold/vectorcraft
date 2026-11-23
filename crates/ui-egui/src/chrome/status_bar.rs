use super::{ROTATE_VIEW_ANGLES, Ui, VectorcraftApp, json};
use craft::behavior::IconId;
use craft::behavior::kurbo::Size;
use craft::ui::bars::{Bar, BarKind, StatusMessage, StatusProgress};
use craft::ui::buttons::{Button, IconButton};
use craft::ui::contract::{Bind, Entry, Gap, LabelKind, Outcome as _, Props as _, Show as _, ShowContainer as _, SideLabel, Trigger};
use craft::ui::display::{Divider, Elide, Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::menu::Menu;
use craft::ui::pickers::Select;
use craft::ui::style::TypeRole;
use craft::ui::text::ZoomField;

/// Shows the status bar: zoom, rotation, artboard navigator, the info field with its Show menu, the readouts, Stop,
/// the background-work progress and last the message, so a narrow bar elides the message before anything else.
pub fn status_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let (info, next) = info_text(app);
    if let Some(secs) = next {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(secs));
    }
    let print = app.session.prefs.display_print_size;
    let nab = app.session.active().map(|d| d.doc.artboards.len()).unwrap_or(0);
    let cur = crate::panels::artboards::selected(app, nab);
    // PLAN: assumed Artboard::rect has `width()` and `height()`.
    let document = app
        .session
        .active()
        .and_then(|d| d.doc.artboards.get(cur))
        .map_or(Size::new(1.0, 1.0), |a| Size::new(f64::from(a.rect.width()), f64::from(a.rect.height())));
    let boards: Vec<Entry<usize>> = app
        .session
        .active()
        .map(|d| d.doc.artboards.iter().enumerate().map(|(i, a)| Entry::new(i, format!("{}  {}", i + 1, a.name))).collect())
        .unwrap_or_default();
    let angles: Vec<Entry<usize>> = ROTATE_VIEW_ANGLES.iter().enumerate().map(|(i, (label, _))| Entry::new(i, *label)).collect();
    let rotation = app.view().map_or(0.0, |v| v.rotation);
    let turned = ROTATE_VIEW_ANGLES.iter().position(|(_, a)| (rotation - a).abs() < 1e-9).unwrap_or(usize::MAX);
    let shows: Vec<Entry<StatusBarShow>> =
        StatusBarShow::ALL.into_iter().map(|s| Entry::new(s, s.label()).checked(app.ui.status_bar_show == s)).collect();
    let unit = app.session.general_unit();
    let lang = crate::i18n::current();
    let render = crate::i18n::tr_ctx(lang, "diagnostics", "render");
    let ui_name = crate::i18n::tr_ctx(lang, "diagnostics", "ui");
    let busy = busy_labels(app);

    let (mut zoom_to, mut rotate_to, mut go, mut show_pick, mut stop) = (None, None, None, None, false);
    // PLAN: assumed VectorcraftApp::canvas_view() -> Option<CanvasView> (cv.S4) and the label msgid "Status Bar".
    Bar::new(tl!("Status Bar"), BarKind::Status).show(ui, |ui, _| {
        let zoom = Field::new(tl!("Zoom"))
            .side(SideLabel::new().kind(LabelKind::Hidden))
            .show(ui, |ui, _| ZoomField::new(tl!("Zoom"), app.canvas_view(), document).scrub(true).show(ui));
        // PLAN: assumed CanvasView::zoom and crate::state::zoom_display_percent(zoom, print) through the scheme (cv.S4).
        if let Some(view) = zoom.inner.and_then(|z| z.view.committed().copied()) {
            zoom_to = Some(crate::state::zoom_display_percent(view.zoom, print));
        }
        Field::new(tl!("Rotation")).side(SideLabel::new().kind(LabelKind::Hidden)).show(ui, |ui, _| {
            // Entries are keyed by index: an angle is not a hashable value.
            rotate_to = Select::new(tl!("Rotation"), Bind::given(turned), &angles).show(ui).value().copied();
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
            if let Some(i) = Select::new(tl!("Artboard"), Bind::given(cur), &boards).enabled(nab > 0).show(ui).value().copied() {
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
        // A narrow bar leaves out the timings first, then the pointer's position, before the message.
        let readout = |text: String, sample: String| Text::new(text).shade(Shade::Faint).sample(sample);
        Row::new().gap(Gap::Item).fit_rank(1).show(ui, |ui, _| {
            readout(format!("{render} {:.1} ms", app.perf.render_ms), format!("{render} 8888.8 ms")).show(ui);
            readout(format!("{ui_name} {:.1} ms", app.perf.frame_ms), format!("{ui_name} 8888.8 ms")).show(ui);
            // No rate until the window has drawn continuously for a moment (`frame_rate`).
            let fps = if app.perf.fps > 0.0 { format!("{:.0} fps", app.perf.fps) } else { "\u{2013} fps".into() };
            readout(fps, "8888 fps".into()).show(ui);
        });
        // With no hover point the axes stay empty and keep their width.
        let origin = app.ruler_origin();
        Row::new().gap(Gap::Item).fit_rank(2).show(ui, |ui, _| {
            for (axis, value) in [("X", app.hover_doc.map(|p| p.x - origin.x)), ("Y", app.hover_doc.map(|p| p.y - origin.y))] {
                let text = value.map(|v| format!("{axis}: {}", unit.readout(v))).unwrap_or_default();
                Text::new(text).role(TypeRole::Mono).shade(Shade::Muted).sample(format!("{axis}: -8888.88 {}", unit.suffix())).show(ui);
            }
        });
        if app.session.script_running() {
            stop = Button::new(tl!("Stop")).show(ui).response().clicked();
        }
        if let Some(job) = busy.first() {
            let label = crate::i18n::msg(job);
            let text = match busy.len() - 1 {
                0 => format!("{label}\u{2026}"),
                more => format!("{label}\u{2026} (+{more})"),
            };
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
/// and that error's message is text the script supplied, that text is shown as it is and only the
/// words around it are translated.
pub(crate) fn status_message(app: &VectorcraftApp) -> String {
    let status = app.ui.status.as_str();
    if let Some((before, kept, after)) = app.session.script_authored_failure()
        && status.strip_prefix(before.as_str()).and_then(|rest| rest.strip_suffix(after.as_str())) == Some(kept)
    {
        return crate::i18n::msg_keeping(&before, kept, &after);
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
