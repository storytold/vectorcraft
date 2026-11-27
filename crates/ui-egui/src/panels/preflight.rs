//! Window › Production Preflight. Results follow the active document/revision and chosen output;
//! file and font changes outside the document are rechecked periodically while the panel is open.

use std::borrow::Cow;
use std::cell::RefCell;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _, WordSetCx as _};
use craft::ui::display::{Divider, Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::list::{Finding, FindingsList, FindingsWords, ListRow, Severity};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::pickers::Select;
use craft::ui::runtime::frame::clock;
use craft::ui::text::NumberField;
use egui::Ui;
use serde_json::{Value, json};

use crate::VectorcraftApp;

pub const ID: &str = "preflight";
const RECHECK: f64 = 2.0;

/// The Preflight panel's run-only state: the chosen checks and the last report.
#[derive(Clone, Debug, Default)]
pub(crate) struct PreflightPanel {
    pub(crate) options: Options,
    pub(crate) cache: Cache,
}

/// What the user chose to check.
#[derive(Clone, Debug)]
pub(crate) struct Options {
    output: usize,
    active_only: bool,
    min_ppi: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self { output: 0, active_only: false, min_ppi: 300.0 }
    }
}

/// The last report and what it was made for.
#[derive(Clone, Debug, Default)]
pub(crate) struct Cache {
    /// Document identity, revision, output, artboard scope and resolution threshold.
    key: Option<(u64, u64, usize, Option<usize>, u64)>,
    at: f64,
    /// Shared so a frame clones a pointer, not the report.
    report: Arc<Value>,
    error: Option<String>,
}

fn cached(app: &mut VectorcraftApp, ui: &Ui, force: bool) -> Cache {
    let o = app.ui.preflight_panel.options.clone();
    let Some(st) = app.session.active() else { return Cache::default() };
    let (uid, revision) = (st.uid, st.revision);
    let board = o.active_only.then(|| app.view().map_or(0, |v| v.artboard));
    let key = Some((uid, revision, o.output, board, o.min_ppi.to_bits()));
    let now = clock(ui.ctx());
    let c = &app.ui.preflight_panel.cache;
    if force || c.key != key || now - c.at >= RECHECK {
        let output = ["pdf", "print", "screen"].get(o.output).copied().unwrap_or("pdf");
        let mut params = json!({ "output": output, "minPpi": o.min_ppi });
        if let Some(i) = board {
            params["artboards"] = json!([i]);
        }
        let (report, error) = match app.session.execute("document.preflight", &params) {
            Ok(v) => (v, None),
            Err(e) => (Value::Null, Some(e.to_string())),
        };
        app.ui.preflight_panel.cache = Cache { key, at: now, report: Arc::new(report), error };
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RECHECK));
    app.ui.preflight_panel.cache.clone()
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "fonts" => tl!("Fonts"),
        "text" => tl!("Text Overflow"),
        "links" => tl!("Linked Files"),
        _ => tl!("Image Resolution"),
    }
}

fn reason(code: &str) -> &str {
    match code {
        "missingFont" => tl!("Missing font; fallback will be used"),
        "substitutedFont" => tl!("Missing font style; closest style will be used"),
        "overflow" => tl!("Text past the frame will be left out"),
        "missingLink" => tl!("Linked file is missing"),
        "modifiedLink" => tl!("Linked file changed; update it before delivery"),
        _ => tl!("Image resolution is below the minimum"),
    }
}

/// All locating actions go through commands. Resource findings name a visible canvas owner.
fn locate(app: &mut VectorcraftApp, finding: &Value) {
    let ids = finding["ids"].as_array().cloned().unwrap_or_default();
    if ids.is_empty() {
        return;
    }
    let selected = match app.run("select.set", json!({ "ids": ids })) {
        Ok(result) => result,
        Err(error) => {
            app.ui.status = error;
            return;
        }
    };
    // Selection can succeed while dropping artwork protected by a locked artboard.
    let selected: std::collections::HashSet<_> = selected["ids"].as_array().into_iter().flatten().filter_map(Value::as_u64).collect();
    let found = ids.iter().filter_map(Value::as_u64).filter(|id| selected.contains(id)).count();
    if found > 0
        && let Err(error) = app.run(crate::addons::arrange::ZOOM, json!({}))
    {
        app.ui.status = error;
        return;
    }
    let warning = tl!("Some affected artwork could not be selected. Unlock it or its artboard, or leave isolation mode, then try again.");
    if found < ids.len() {
        app.ui.status = warning.to_string();
    } else if app.ui.status == warning {
        app.ui.status.clear();
    }
}

/// Commands the list's buttons ask for; they run once the list has been shown.
type Pending = RefCell<Vec<(&'static str, Value)>>;

/// A line of the findings list.
enum Entry_ {
    /// The check at this index of the report.
    Heading(usize),
    /// The finding at this index of the report.
    Finding(usize),
}

/// The report as list rows: a heading per check, then its findings. The key of a row is its place in the flat list,
/// so a heading and a finding never share one.
struct PreflightRows<'a> {
    checks: &'a [Value],
    findings: &'a [Value],
    entries: Vec<Entry_>,
}

impl<'a> PreflightRows<'a> {
    fn new(report: &'a Value) -> Self {
        let checks = report["checks"].as_array().map_or(&[][..], Vec::as_slice);
        let findings = report["findings"].as_array().map_or(&[][..], Vec::as_slice);
        let mut entries = Vec::new();
        for (c, check) in checks.iter().enumerate() {
            entries.push(Entry_::Heading(c));
            entries.extend(findings.iter().enumerate().filter(|(_, f)| f["kind"] == check["kind"]).map(|(f, _)| Entry_::Finding(f)));
        }
        Self { checks, findings, entries }
    }
}

impl DisplayOrder<u64> for PreflightRows<'_> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn key(&self, at: usize) -> u64 {
        at as u64
    }

    fn index_of(&self, key: &u64) -> Option<usize> {
        usize::try_from(*key).ok().filter(|at| *at < self.entries.len())
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.entries.get(at), Some(Entry_::Heading(_)))
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match self.entries.get(at)? {
            Entry_::Heading(c) => Some(Cow::Borrowed(kind_label(self.checks.get(*c)?["kind"].as_str().unwrap_or_default()))),
            Entry_::Finding(f) => Some(Cow::Borrowed(self.findings.get(*f)?["name"].as_str().unwrap_or_default())),
        }
    }
}

impl ItemSource<u64> for PreflightRows<'_> {}

/// The row that heads a check: its name, its status and count, and Find Font… under the fonts check when it has findings.
fn heading_row<'r>(check: &Value, pending: &'r Pending) -> ListRow<'r> {
    let kind = check["kind"].as_str().unwrap_or_default();
    let count = check["count"].as_u64().unwrap_or(0);
    let status = match check["status"].as_str() {
        Some("passed") => Text::new(tl!("Passed").to_owned()),
        Some("failed") => Text::new(tl!("Needs attention").to_owned()).tone(Tone::Warn),
        _ => Text::new(tl!("Not checked").to_owned()).shade(Shade::Muted),
    };
    let mut row = ListRow::new(kind_label(kind).to_owned()).section_heading().trailing_field(status, |_| {});
    if count > 0 {
        row = row.detail(count.to_string());
    }
    if kind == "fonts" && count > 0 {
        row = row.below(Button::new(tl!("Find Font…").to_owned()), move |out| {
            if out.clicks == 1 {
                pending.borrow_mut().push(("type.findFont", json!({})));
            }
        });
    }
    row
}

/// The row of one finding, with the button that repairs a link under it.
fn finding_row<'r>(key: u64, finding: &Value, words: &FindingsWords, pending: &'r Pending) -> ListRow<'r> {
    let code = finding["code"].as_str().unwrap_or_default();
    let mut detail = reason(code).to_owned();
    if let Some(ppi) = finding["effectivePpi"].as_array() {
        let values = ppi.iter().filter_map(Value::as_f64).map(|n| format!("{n:.0}")).collect::<Vec<_>>();
        detail = format!("{detail}; {}", effective_ppi_line(&values));
    }
    let row = Finding::new(key, Severity::Warning, finding["name"].as_str().unwrap_or_default()).detail(detail).row(words);
    let action = match code {
        "missingLink" => Some((tl!("Relink…"), "ui.relinkDialog")),
        "modifiedLink" => Some((tl!("Update Link"), "links.update")),
        _ => None,
    };
    match (action, finding["sourceId"].as_u64()) {
        (Some((label, command)), Some(id)) => row.below(Button::new(label.to_owned()), move |out| {
            if out.clicks == 1 {
                pending.borrow_mut().push((command, json!({ "ids": [id] })));
            }
        }),
        _ => row,
    }
}

/// The Output, Minimum PPI, Active Artboard Only and Recheck controls; true when Recheck was pressed.
fn options(app: &mut VectorcraftApp, ui: &mut Ui) -> bool {
    let current = app.ui.preflight_panel.options.clone();
    let outputs: Vec<Entry<usize>> =
        [tl!("PDF"), tl!("Print"), tl!("Screen")].into_iter().enumerate().map(|(i, name)| Entry::new(i, name.to_owned())).collect();
    let (mut output, mut ppi, mut active_only, mut force) = (None, None, None, false);
    FieldGroup::new(tl!("Preflight Options").to_owned()).show(ui, |ui, _| {
        Field::new(tl!("Output:").to_owned()).show(ui, |ui, _| {
            output = Select::new(tl!("Output:").to_owned(), Bind::given(current.output), &outputs)
                .show(ui)
                .value()
                .copied()
                .filter(|i| *i != current.output);
        });
        Field::new(tl!("Minimum PPI:").to_owned()).show(ui, |ui, _| {
            ppi = NumberField::new(tl!("Minimum PPI:").to_owned(), Bind::given(Some(current.min_ppi)))
                .range(1.0..=9600.0)
                .show(ui)
                .edit
                .events
                .landed()
                .and_then(|v| *v);
        });
        active_only = Checkbox::new(Bind::given(current.active_only)).label(tl!("Active Artboard Only").to_owned()).show(ui).events.landed().copied();
        force = Button::new(tl!("Recheck").to_owned()).show(ui).clicks == 1;
    });
    let options = &mut app.ui.preflight_panel.options;
    if let Some(i) = output {
        options.output = i;
        options.min_ppi = if i == 2 { 96.0 } else { 300.0 };
    }
    if let Some(ppi) = ppi {
        options.min_ppi = ppi;
    }
    if let Some(on) = active_only {
        options.active_only = on;
    }
    force
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        Notice::new(Tone::Neutral, tl!("Open a document to check it before delivery.").to_owned())
            .title(tl!("Production Preflight").to_owned())
            .layout(NoticeLayout::Centred)
            .show(ui);
        return;
    }
    let force = options(app, ui);
    Divider::horizontal().show(ui);
    if app.session.active().is_some_and(|st| st.interaction.is_some()) || app.session.tool_busy() {
        Notice::new(Tone::Neutral, tl!("Preflight updates when the edit finishes.").to_owned()).layout(NoticeLayout::Centred).show(ui);
        return;
    }
    let cache = cached(app, ui, force);
    if let Some(error) = cache.error {
        Notice::new(Tone::Bad, crate::i18n::msg(&error)).title(tl!("Preflight could not complete").to_owned()).layout(NoticeLayout::Centred).show(ui);
        return;
    }
    results(app, ui, &cache.report);
}

fn results(app: &mut VectorcraftApp, ui: &mut Ui, report: &Value) {
    let rows = PreflightRows::new(report);
    let pending = Pending::default();
    let out = FindingsList::new(tl!("Findings")).show_items(ui, &rows, &mut |key, icx| match rows.entries.get(*key as usize) {
        Some(Entry_::Heading(c)) => heading_row(&rows.checks[*c], &pending),
        Some(Entry_::Finding(f)) => finding_row(*key, &rows.findings[*f], &icx.words::<FindingsWords>(), &pending),
        None => ListRow::new(String::new()),
    });
    let mut located = None;
    for event in out.events {
        if let CollectionEvent::Activate(key) = event
            && let Some(Entry_::Finding(f)) = usize::try_from(key).ok().and_then(|at| rows.entries.get(at))
        {
            located = rows.findings.get(*f);
        }
    }
    if let Some(finding) = located {
        locate(app, finding);
    }
    for (command, params) in pending.into_inner() {
        // A failed command leaves the findings as they were; the next recheck shows the truth.
        app.run(command, params).ok();
    }
    for check in rows.checks.iter().filter(|c| c["notChecked"].as_u64().unwrap_or(0) > 0) {
        let _ = check;
        Text::new(tl!("Resolution inside resource art or placed documents is not checked.").to_owned()).shade(Shade::Muted).show(ui);
    }
    for warning in report["outputWarnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        Text::new(crate::i18n::msg(warning)).tone(Tone::Warn).show(ui);
    }
    Divider::horizontal().show(ui);
    Text::new(tl!("Checks update after edits. Linked files are rechecked every two seconds.").to_owned()).shade(Shade::Muted).show(ui);
    Text::new(tl!("These checks do not certify an exported file.").to_owned()).shade(Shade::Muted).show(ui);
}

/// A low-resolution finding's effective resolution, one value per axis.
fn effective_ppi_line(values: &[String]) -> String {
    format!("{} {} ppi", crate::i18n::colon(tl!("Effective PPI")), values.join(" × "))
}

#[cfg(test)]
mod tests;
