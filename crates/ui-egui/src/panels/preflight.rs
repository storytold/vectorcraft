//! Window › Production Preflight. Results follow the active document/revision and chosen output;
//! file and font changes outside the document are rechecked periodically while the panel is open.

use egui::Ui;
use serde_json::{Value, json};

use super::{empty_state, pstate, set_pstate};
use crate::VectorcraftApp;
use crate::theme::Tokens;
use crate::widgets::{self, dim_label};

pub const ID: &str = "preflight";
const OPTIONS: &str = "production-preflight-options";
const CACHE: &str = "production-preflight-cache";
const RECHECK: f64 = 2.0;

#[derive(Clone)]
struct Options {
    output: usize,
    active_only: bool,
    min_ppi: f64,
}

impl Default for Options {
    fn default() -> Self {
        Self { output: 0, active_only: false, min_ppi: 300.0 }
    }
}

#[derive(Clone, Default)]
struct Cache {
    /// Document identity, revision, output, artboard scope and resolution threshold.
    key: Option<(u64, u64, usize, Option<usize>, u64)>,
    at: f64,
    report: Value,
    error: Option<String>,
}

fn cached(app: &mut VectorcraftApp, ui: &Ui, o: &Options, force: bool) -> Cache {
    let Some(st) = app.session.active() else { return Cache::default() };
    let board = o.active_only.then(|| app.view().map_or(0, |v| v.artboard));
    let key = Some((st.uid, st.revision, o.output, board, o.min_ppi.to_bits()));
    let now = ui.input(|i| i.time);
    let mut c: Cache = pstate(ui.ctx(), CACHE);
    if force || c.key != key || now - c.at >= RECHECK {
        let output = ["pdf", "print", "screen"].get(o.output).copied().unwrap_or("pdf");
        let mut params = json!({ "output": output, "minPpi": o.min_ppi });
        if let Some(i) = board {
            params["artboards"] = json!([i]);
        }
        match app.session.execute("document.preflight", &params) {
            Ok(v) => {
                c.report = v;
                c.error = None;
            }
            Err(e) => {
                c.report = Value::Null;
                c.error = Some(e.to_string());
            }
        }
        (c.key, c.at) = (key, now);
        set_pstate(ui.ctx(), CACHE, c.clone());
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(RECHECK));
    c
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

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    if app.session.active().is_none() {
        empty_state(ui, "dc-list-view", "Production Preflight", "Open a document to check it before delivery.");
        return;
    }
    let mut options: Options = pstate(ui.ctx(), OPTIONS);
    let labels = [tl!("PDF"), tl!("Print"), tl!("Screen")];
    widgets::label_row(ui, tl!("Output:"), 74.0, |ui| {
        let shown = labels.get(options.output).copied().unwrap_or(labels[0]);
        if let Some(i) = widgets::dropdown(ui, "preflight-output", shown, &labels, ui.available_width().max(90.0)) {
            options.output = i;
            options.min_ppi = if i == 2 { 96.0 } else { 300.0 };
        }
    });
    widgets::label_row(ui, tl!("Minimum PPI:"), 90.0, |ui| {
        if let Some(ppi) = widgets::range_field(ui, "preflight-ppi", options.min_ppi, 1.0..=9600.0, "", 0, 80.0) {
            options.min_ppi = ppi;
        }
    });
    ui.checkbox(&mut options.active_only, tl!("Active Artboard Only"));
    let force = ui.button(tl!("Recheck")).clicked();
    set_pstate(ui.ctx(), OPTIONS, options.clone());
    ui.separator();
    if app.session.active().is_some_and(|st| st.interaction.is_some()) || app.session.tool_busy() {
        dim_label(ui, tl!("Preflight updates when the edit finishes."));
        return;
    }
    let cache = cached(app, ui, &options, force);
    if let Some(error) = cache.error {
        ui.colored_label(Tokens::get(ui.ctx()).error, tl!("Preflight could not complete"));
        ui.label(crate::i18n::msg(&error));
        return;
    }
    let report = &cache.report;
    let height = ui.available_height().clamp(100.0, 600.0);
    widgets::panel_scroll(ui, egui::ScrollArea::vertical().id_salt("preflight-findings").max_height(height).auto_shrink([false, true]), |ui| {
        results(app, ui, report);
    });
}

fn results(app: &mut VectorcraftApp, ui: &mut Ui, report: &Value) {
    let findings = report["findings"].as_array().map_or(&[][..], Vec::as_slice);
    let tokens = Tokens::get(ui.ctx());
    for check in report["checks"].as_array().into_iter().flatten() {
        let kind = check["kind"].as_str().unwrap_or_default();
        let count = check["count"].as_u64().unwrap_or(0);
        let (status, color) = match check["status"].as_str() {
            Some("passed") => (tl!("Passed"), tokens.text),
            Some("failed") => (tl!("Needs attention"), tokens.warning),
            _ => (tl!("Not checked"), tokens.text_dim),
        };
        ui.horizontal_wrapped(|ui| {
            ui.strong(kind_label(kind));
            ui.colored_label(color, status);
            if count > 0 {
                ui.label(count.to_string());
            }
        });
        for (index, finding) in findings.iter().enumerate().filter(|(_, f)| f["kind"] == kind) {
            let name = finding["name"].as_str().unwrap_or_default();
            let code = finding["code"].as_str().unwrap_or_default();
            let text = format!("{name}\n{}", reason(code));
            ui.push_id(index, |ui| {
                if ui
                    .add(egui::Button::new(text).wrap().min_size(egui::vec2(ui.available_width(), 0.0)))
                    .on_hover_text(tl!("Select and zoom to affected artwork"))
                    .clicked()
                {
                    locate(app, finding);
                }
                if let Some(ppi) = finding["effectivePpi"].as_array() {
                    let values = ppi.iter().filter_map(Value::as_f64).map(|n| format!("{n:.0}")).collect::<Vec<_>>();
                    dim_label(ui, &format!("{}: {} ppi", tl!("Effective PPI"), values.join(" × ")));
                }
                let source = finding["sourceId"].as_u64();
                if code == "missingLink"
                    && ui.small_button(tl!("Relink…")).clicked()
                    && let Some(id) = source
                {
                    app.run("ui.relinkDialog", json!({"ids": [id]})).ok();
                }
                if code == "modifiedLink"
                    && ui.small_button(tl!("Update Link")).clicked()
                    && let Some(id) = source
                {
                    app.run("links.update", json!({"ids": [id]})).ok();
                }
            });
        }
        if kind == "fonts" && count > 0 && ui.small_button(tl!("Find Font…")).clicked() {
            app.run("type.findFont", json!({})).ok();
        }
        if check["notChecked"].as_u64().unwrap_or(0) > 0 {
            dim_label(ui, tl!("Resolution inside resource art or placed documents is not checked."));
        }
        ui.add_space(6.0);
    }
    for warning in report["outputWarnings"].as_array().into_iter().flatten().filter_map(Value::as_str) {
        ui.colored_label(tokens.warning, crate::i18n::msg(warning));
    }
    ui.separator();
    dim_label(ui, tl!("Checks update after edits. Linked files are rechecked every two seconds."));
    dim_label(ui, tl!("These checks do not certify an exported file."));
}

#[cfg(test)]
mod tests;
