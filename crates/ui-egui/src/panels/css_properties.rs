//! CSS Properties panel (Window → CSS Properties): the CSS web pages style the selected objects
//! with (`css.selection`), or after Generate CSS, while nothing is selected, the whole document's
//! (`css.generate`), in a code view. Copy Selected Style copies it (`css.copy`); Export Selected
//! CSS… and Export All… write a `.css` file with the pictures of rasterized art next to it
//! (`css.exportFile`). The panel menu holds the export options (units, absolute position, width
//! and height, unnamed objects, rasterize), which all of these use.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat};
use egui::{FontId, Ui};
use serde_json::{Value, json};
use vectorcraft_svg::{CssOptions, CssUnits};

use super::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, menu_item};
use crate::{VectorcraftApp, io};

pub const ID: &str = "cssProperties";
/// Panel state: the export options.
pub(super) const OPTIONS: &str = "css-options";
/// Panel state: Generate CSS is on (the document's CSS shows while nothing is selected).
pub(super) const ALL: &str = "css-all";
/// The most rules the code view shows (Export All writes every one).
const MAX_SHOWN: usize = 300;

/// The CSS shown, and what it was made for: (document uid, revision, whole document, options,
/// dark theme).
#[derive(Clone, Default)]
struct Cache {
    key: Option<(u64, u64, bool, CssOptions, bool)>,
    shown: Arc<Shown>,
}

#[derive(Default)]
struct Shown {
    /// The rules shown, highlighted.
    job: Option<Arc<LayoutJob>>,
    rules: usize,
    /// Rules beyond [`MAX_SHOWN`].
    more: usize,
    skipped: u64,
    warnings: Vec<String>,
}

/// The command params of `opts` for `scope` (`selection` or `all`).
fn params(opts: &CssOptions, scope: &str) -> Value {
    let mut p = serde_json::to_value(opts).unwrap_or_else(|_| json!({}));
    p["scope"] = json!(scope);
    p
}

/// The CSS rules as a code view: selectors and braces strong, property names dim.
fn highlight(rules: &[&str], t: &Tokens, font: FontId) -> LayoutJob {
    let fmt = |color| TextFormat { font_id: font.clone(), color, ..Default::default() };
    let mut job = LayoutJob::default();
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            job.append("\n\n", 0.0, fmt(t.text));
        }
        for (j, line) in rule.lines().enumerate() {
            if j > 0 {
                job.append("\n", 0.0, fmt(t.text));
            }
            match line.split_once(": ") {
                Some((k, v)) if line.starts_with("  ") => {
                    job.append(k, 0.0, fmt(t.text_dim));
                    job.append(": ", 0.0, fmt(t.text_dim));
                    job.append(v, 0.0, fmt(t.text));
                }
                _ => job.append(line, 0.0, fmt(t.text_strong)),
            }
        }
    }
    job
}

/// The CSS for the panel, made again only when the document, the scope or the options change.
fn shown(app: &mut VectorcraftApp, ui: &Ui, opts: &CssOptions, all: bool) -> Option<Arc<Shown>> {
    let st = app.session.active()?;
    let t = Tokens::get(ui.ctx());
    let key = Some((st.uid, st.revision, all, opts.clone(), t.dark));
    let mut c: Cache = pstate(ui.ctx(), "css-cache");
    if c.key != key {
        let (id, scope) = if all { ("css.generate", "all") } else { ("css.selection", "selection") };
        let out = app.session.execute(id, &params(opts, scope)).unwrap_or_default();
        let rules: Vec<&str> = out["rules"].as_array().into_iter().flatten().filter_map(|r| r["css"].as_str()).collect();
        let warnings = out["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let job = (!rules.is_empty())
            .then(|| Arc::new(highlight(rules.get(..MAX_SHOWN).unwrap_or(&rules), &t, egui::TextStyle::Monospace.resolve(ui.style()))));
        let skipped = out["skipped"].as_u64().unwrap_or(0);
        c = Cache { key, shown: Arc::new(Shown { job, rules: rules.len(), more: rules.len().saturating_sub(MAX_SHOWN), skipped, warnings }) };
        set_pstate(ui.ctx(), "css-cache", c.clone());
    }
    Some(c.shown)
}

/// `w` as a sentence: capitalised, with a full stop.
fn sentence(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).chain(['.']).collect()).unwrap_or_default()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(selected) = app.session.active().map(|st| !st.selection.is_empty()) else {
        widgets::dim_label(ui, tl!("No document"));
        return;
    };
    let opts: CssOptions = pstate(&ctx, OPTIONS);
    let all = !selected && pstate::<bool>(&ctx, ALL);
    let Some(shown) = shown(app, ui, &opts, all) else { return };
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::both().id_salt("css-properties-scroll").max_height(300.0).auto_shrink([false, true]).show(ui, |ui| {
            ui.set_min_height(120.0);
            match &shown.job {
                Some(job) => {
                    ui.add(egui::Label::new(egui::WidgetText::LayoutJob(job.clone())).selectable(true).extend());
                }
                None if selected => super::empty_state(ui, "globe", tl!("No CSS"), tl!("The selected objects have no CSS of their own (see below).")),
                None => super::empty_state(
                    ui,
                    "globe",
                    tl!("No selection"),
                    tl!("Select objects to see their CSS, or Generate CSS for the whole document."),
                ),
            }
        });
    });
    let notes = (shown.more > 0)
        .then(|| crate::i18n::fmt(tl!("{count} more rules: Export writes every one."), &[("count", &shown.more.to_string())]))
        .into_iter()
        .chain((shown.skipped > 0).then(|| {
            crate::i18n::fmt(
                tl!("{count} unnamed objects left out: name them, or turn on Unnamed Objects in the panel menu."),
                &[("count", &shown.skipped.to_string())],
            )
        }))
        .chain(shown.warnings.iter().map(|w| sentence(w)));
    let dim = Tokens::get(&ctx).text_dim;
    for n in notes {
        ui.add(egui::Label::new(egui::RichText::new(n).size(11.0).color(dim)).wrap());
    }
    let has_css = shown.rules > 0;
    let scope = if all { "all" } else { "selection" };
    let (mut generate, mut copy, mut export) = (false, false, false);
    widgets::bottom_bar(ui, |ui| {
        generate = widgets::icon_button_enabled(
            ui,
            "sparkles",
            tl!("Generate CSS: the whole document's CSS while nothing is selected"),
            all,
            !selected,
            24.0,
        )
        .clicked();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let tip = if all { tl!("Export All…") } else { tl!("Export Selected CSS…") };
            export = widgets::icon_button_enabled(ui, "save", tip, false, has_css, 24.0).clicked();
            let tip = if all { tl!("Copy All Styles") } else { tl!("Copy Selected Style") };
            copy = widgets::icon_button_enabled(ui, "copy", tip, false, has_css, 24.0).clicked();
        });
    });
    if generate {
        set_pstate(&ctx, ALL, !all);
    }
    if copy {
        crate::menus::invoke(app, "css.copy", params(&opts, scope));
    }
    if export {
        crate::menus::invoke(app, "css.exportFile", params(&opts, scope));
    }
}

/// `css.copy {scope?, …css.selection options}`: copy the CSS of the selection (or with scope
/// `all` the document's) → `{css, rules}`.
pub(crate) fn copy(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = if p.get("scope").and_then(Value::as_str) == Some("all") { "css.generate" } else { "css.selection" };
    let r = app.session.execute(id, p).map_err(|e| e.to_string())?;
    let css = r["css"].as_str().filter(|c| !c.is_empty()).ok_or("no CSS to copy: select objects with CSS of their own")?.to_string();
    let rules = r["rules"].as_array().map_or(0, Vec::len);
    app.clipboard_out = Some(css.clone());
    app.status(format!("Copied the CSS of {rules} object(s)"));
    Ok(json!({ "css": css, "rules": rules }))
}

/// `css.exportFile {path?, scope?, …css.selection options}`: `css.export` written to `path`, else
/// a picked file (the web downloads it), its pictures next to it → `{path, rules, images,
/// warnings}`.
pub(crate) fn export_file(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut q = p.clone();
    let path = q.as_object_mut().and_then(|o| o.remove("path")).and_then(|v| v.as_str().map(str::to_string));
    let r = app.session.execute("css.export", &q).map_err(|e| e.to_string())?;
    let path = io::target_path(app, path, "css")?;
    io::write_to(&mut app.services, &path, r["data"].as_str().unwrap_or_default().as_bytes())?;
    let folder = std::path::Path::new(&path).parent().map(std::path::Path::to_path_buf);
    let mut images = Vec::new();
    for im in r["images"].as_array().into_iter().flatten() {
        let (Some(name), Some(data)) = (im["name"].as_str(), im["dataBase64"].as_str()) else { continue };
        let bytes = vectorcraft_format::base64_decode(data).ok_or("the export returned unreadable data")?;
        let at = folder.as_ref().map_or_else(|| name.to_string(), |f| f.join(name).to_string_lossy().into_owned());
        io::write_to(&mut app.services, &at, &bytes)?;
        images.push(at);
    }
    // The status bar shows the first warning in Export As's message.
    let warnings: Vec<&str> = r["warnings"].as_array().map(|w| w.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    let what = &path;
    app.status(match warnings.first() {
        Some(first) => format!("Exported {what} with {} note(s): {first}", warnings.len()),
        None => format!("Exported {path}"),
    });
    Ok(json!({ "path": path, "rules": r["rules"], "images": images, "warnings": r["warnings"] }))
}

/// The panel (≡) menu: copy and export, Generate CSS and the export options.
pub fn menu(app: &mut VectorcraftApp, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let Some(selected) = app.session.active().map(|st| !st.selection.is_empty()) else { return };
    let mut opts: CssOptions = pstate(&ctx, OPTIONS);
    let all: bool = pstate(&ctx, ALL);
    if menu_item(ui, tl!("Copy Selected Style"), selected, false) {
        crate::menus::invoke(app, "css.copy", params(&opts, "selection"));
    }
    if menu_item(ui, tl!("Export Selected CSS…"), selected, false) {
        crate::menus::invoke(app, "css.exportFile", params(&opts, "selection"));
    }
    if menu_item(ui, tl!("Export All…"), true, false) {
        crate::menus::invoke(app, "css.exportFile", params(&opts, "all"));
    }
    ui.separator();
    if menu_item(ui, tl!("Generate CSS"), true, all) {
        set_pstate(&ctx, ALL, !all);
    }
    ui.separator();
    let before = opts.clone();
    ui.menu_button(format!("   {}", crate::i18n::fmt(tl!("Units: {units}"), &[("units", opts.units.name())])), |ui| {
        for u in CssUnits::ALL {
            if menu_item(ui, u.name(), true, opts.units == u) {
                opts.units = u;
            }
        }
    });
    for (label, on) in [
        (tl!("Absolute Position"), &mut opts.position),
        (tl!("Width and Height"), &mut opts.dimensions),
        (tl!("Unnamed Objects"), &mut opts.unnamed),
        (tl!("Rasterize Unsupported Art"), &mut opts.rasterize),
    ] {
        if menu_item(ui, label, true, *on) {
            *on = !*on;
        }
    }
    if opts != before {
        set_pstate(&ctx, OPTIONS, opts);
    }
}
