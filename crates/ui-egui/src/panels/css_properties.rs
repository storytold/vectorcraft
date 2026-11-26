//! CSS Properties panel (Window → CSS Properties): the CSS web pages style the selected objects
//! with (`css.selection`), or after Generate CSS, while nothing is selected, the whole document's
//! (`css.generate`), in a code view. Copy Selected Style copies it (`css.copy`); Export Selected
//! CSS… and Export All… write a `.css` file with the pictures of rasterized art next to it
//! (`css.exportFile`). The panel menu holds the export options (units, absolute position, width
//! and height, unnamed objects, rasterize), which all of these use.

use std::sync::Arc;

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::theme::variant::Tone;
use craft::ui::contract::{Scroll, Show as _, ShowContainer as _};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::Stack;
use craft::ui::menu::PanelRow;
use craft::ui::notices::{Notice, NoticeIcon, NoticeLayout};
use craft::ui::primitives::{IconSpec, Span};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::sections::{FooterAction, FooterBar};
use craft::ui::style::{SurfaceKind, TypeRole};
use craft::ui::text::SelectableText;
use egui::Ui;
use serde_json::{Value, json};
use vectorcraft_svg::{CssOptions, CssUnits};

use crate::{VectorcraftApp, io};

pub const ID: &str = "cssProperties";
/// The most rules the code view shows (Export All writes every one).
const MAX_SHOWN: usize = 300;

const COPY_SELECTED: &str = "ui.cssProperties.copySelected";
const EXPORT_SELECTED: &str = "ui.cssProperties.exportSelected";
const EXPORT_ALL: &str = "ui.cssProperties.exportAll";
const GENERATE: &str = "ui.cssProperties.generate";
const UNITS: &str = "ui.cssProperties.units";
const OPTION: &str = "ui.cssProperties.option";
/// The export options the panel menu switches, as `ui.cssProperties.option` names them.
const FLAGS: [&str; 4] = ["position", "dimensions", "unnamed", "rasterize"];

/// The CSS Properties panel's own state, kept while the app runs.
#[derive(Clone, Debug, Default)]
pub(crate) struct CssPropertiesPanel {
    /// The export options the panel menu sets, which the code view, Copy and Export use.
    pub(crate) options: CssOptions,
    /// Generate CSS: the whole document's CSS while nothing is selected.
    pub(crate) all: bool,
}

/// What [`shown`] was made for: (document uid, revision, whole document, options).
pub(crate) type CssKey = (u64, u64, bool, CssOptions);

/// The CSS the code view shows.
#[derive(Debug, Default)]
pub(crate) struct Shown {
    /// The rules shown (at most [`MAX_SHOWN`]), as the command wrote them.
    // PLAN: the sheet drops `job` and lists no field for the rules' text, which the spans are made from.
    text: Vec<String>,
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

/// The CSS rules as a code view: selectors and braces strong, property names muted, one blank line between rules.
fn spans(rules: &[String]) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    for (i, rule) in rules.iter().enumerate() {
        if i > 0 {
            out.push(Span::new("\n\n"));
        }
        for (j, line) in rule.lines().enumerate() {
            if j > 0 {
                out.push(Span::new("\n"));
            }
            match line.split_once(": ") {
                Some((k, v)) if line.starts_with("  ") => {
                    out.push(Span::new(k).muted());
                    out.push(Span::new(": ").muted());
                    out.push(Span::new(v));
                }
                _ => out.push(Span::new(line).strong()),
            }
        }
    }
    out
}

/// The CSS for the panel, made again only when the document, the scope or the options change.
fn shown(app: &mut VectorcraftApp, opts: &CssOptions, all: bool) -> Option<Arc<Shown>> {
    let st = app.session.active()?;
    let key: CssKey = (st.uid, st.revision, all, opts.clone());
    if let Some((k, v)) = &app.canvas.css_shown
        && *k == key
    {
        return Some(Arc::clone(v));
    }
    let (id, scope) = if all { ("css.generate", "all") } else { ("css.selection", "selection") };
    let out = app.session.execute(id, &params(opts, scope)).unwrap_or_default();
    let rules: Vec<&str> = out["rules"].as_array().into_iter().flatten().filter_map(|r| r["css"].as_str()).collect();
    let shown = Arc::new(Shown {
        text: rules.iter().take(MAX_SHOWN).map(|r| (*r).to_string()).collect(),
        rules: rules.len(),
        more: rules.len().saturating_sub(MAX_SHOWN),
        skipped: out["skipped"].as_u64().unwrap_or(0),
        warnings: io::notes_of(&out),
    });
    app.canvas.css_shown = Some((key, Arc::clone(&shown)));
    Some(shown)
}

/// `w` as a sentence: capitalised, with a full stop.
fn sentence(w: &str) -> String {
    let mut c = w.chars();
    c.next().map(|f| f.to_uppercase().chain(c).chain(['.']).collect()).unwrap_or_default()
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(selected) = app.session.active().map(|st| !st.selection.is_empty()) else {
        Text::new(tl!("No document")).shade(Shade::Muted).show(ui);
        return;
    };
    let opts = app.ui.css_properties_panel.options.clone();
    let all = !selected && app.ui.css_properties_panel.all;
    let Some(shown) = shown(app, &opts, all) else { return };
    Stack::new().fill().surface(SurfaceKind::Well).scroll(Scroll::both()).show(ui, |ui, _| {
        let globe = NoticeIcon::Custom(IconSpec::svg(IconId::new("globe")));
        if shown.rules > 0 {
            SelectableText::spans(tl!("CSS code"), spans(&shown.text)).mono().multiline().show(ui);
        } else if selected {
            let hint = tl!("The selected objects have no CSS of their own (see below).");
            Notice::new(Tone::Neutral, hint).title(tl!("No CSS")).layout(NoticeLayout::Centred).icon(globe).show(ui);
        } else {
            let hint = tl!("Select objects to see their CSS, or Generate CSS for the whole document.");
            Notice::new(Tone::Neutral, hint).title(tl!("No selection")).layout(NoticeLayout::Centred).icon(globe).show(ui);
        }
    });
    let notes = (shown.more > 0)
        .then(|| crate::i18n::tn(shown.more as u64, "{n} more rule: Export writes it.", "{n} more rules: Export writes every one."))
        .into_iter()
        .chain((shown.skipped > 0).then(|| {
            crate::i18n::tn(
                shown.skipped,
                "{n} unnamed object left out: name it, or turn on Unnamed Objects in the panel menu.",
                "{n} unnamed objects left out: name them, or turn on Unnamed Objects in the panel menu.",
            )
        }))
        .chain(shown.warnings.iter().map(|w| sentence(w)));
    for n in notes {
        Text::new(n).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    }
    let has_css = shown.rules > 0;
    let scope = if all { "all" } else { "selection" };
    // Generate at the start edge; Export at the end edge with Copy before it.
    let footer = FooterBar::actions([
        FooterAction::new(IconId::new("sparkles"), tl!("Generate CSS: the whole document's CSS while nothing is selected"))
            .pressed(all)
            .enabled(!selected),
        FooterAction::new(IconId::new("save"), if all { tl!("Export All…") } else { tl!("Export Selected CSS…") }).enabled(has_css),
        FooterAction::new(IconId::new("copy"), if all { tl!("Copy All Styles") } else { tl!("Copy Selected Style") }).enabled(has_css),
    ])
    .at_start(1)
    .show_footer(ui);
    match footer.picked {
        Some(0) => {
            crate::menus::invoke(app, GENERATE, json!({}));
        }
        Some(1) => {
            crate::menus::invoke(app, "css.exportFile", params(&opts, scope));
        }
        Some(2) => {
            crate::menus::invoke(app, "css.copy", params(&opts, scope));
        }
        _ => {}
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
    app.status(crate::i18n::msg_n(rules as u64, "Copied the CSS of {n} object", "Copied the CSS of {n} objects", &[]));
    Ok(json!({ "css": css, "rules": rules }))
}

/// `css.exportFile {path?, scope?, …css.selection options}`: `css.export` written to `path`, else
/// a picked file (the web downloads it), its pictures next to it → `{path, rules, images,
/// warnings}`.
pub(crate) fn export_file(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut q = p.clone();
    let path = q.as_object_mut().and_then(|o| o.remove("path")).and_then(|v| v.as_str().map(str::to_string));
    let r = app.session.execute("css.export", &q).map_err(|e| e.to_string())?;
    // PLAN: assumed `io::target_path(app, path, ext, then)` as files-w9 (decide files-2) gives it.
    io::target_path(app, path, "css", move |app, path| write_export(app, &r, &path))
}

/// The sheet and the pictures of `css.export`'s reply `r`, written at `path` and beside it.
fn write_export(app: &mut VectorcraftApp, r: &Value, path: &str) -> Result<Value, String> {
    io::write_to(&mut app.services, path, r["data"].as_str().unwrap_or_default().as_bytes())?;
    let folder = std::path::Path::new(path).parent().map(std::path::Path::to_path_buf);
    let mut images = Vec::new();
    for im in r["images"].as_array().into_iter().flatten() {
        let (Some(name), Some(data)) = (im["name"].as_str(), im["dataBase64"].as_str()) else { continue };
        let bytes = vectorcraft_format::base64_decode(data).ok_or("the export returned unreadable data")?;
        let at = folder.as_ref().map_or_else(|| name.to_string(), |f| f.join(name).to_string_lossy().into_owned());
        io::write_to(&mut app.services, &at, &bytes)?;
        images.push(at);
    }
    // The status bar shows the warnings as Export As shows them.
    let warnings = io::notes_of(r);
    let status = match warnings.first() {
        Some(first) => io::exported_with_notes(path, warnings.len(), first),
        None => format!("Exported {path}"),
    };
    io::show_with_notes(app, status, warnings);
    Ok(json!({ "path": path, "rules": r["rules"], "images": images, "warnings": r["warnings"] }))
}

/// The option `name` of `opts`, as `ui.cssProperties.option` names them.
fn flag_mut<'a>(opts: &'a mut CssOptions, name: &str) -> Option<&'a mut bool> {
    match name {
        "position" => Some(&mut opts.position),
        "dimensions" => Some(&mut opts.dimensions),
        "unnamed" => Some(&mut opts.unnamed),
        "rasterize" => Some(&mut opts.rasterize),
        _ => None,
    }
}

fn units_ref(units: CssUnits) -> CommandRef {
    CommandRef::with_params(UNITS, json!({ "units": units }))
}

fn option_ref(name: &str) -> CommandRef {
    CommandRef::with_params(OPTION, json!({ "option": name }))
}

/// The panel (≡) menu's rows: copy and export, Generate CSS and the export options.
pub(crate) fn menu_rows(app: &VectorcraftApp) -> Vec<PanelRow> {
    let units = crate::i18n::fmt(tl!("Units: {units}"), &[("units", app.ui.css_properties_panel.options.units.name())]);
    let mut rows = vec![
        PanelRow::Command(CommandRef::new(COPY_SELECTED)),
        PanelRow::Command(CommandRef::new(EXPORT_SELECTED)),
        PanelRow::Command(CommandRef::new(EXPORT_ALL)),
        PanelRow::Separator,
        PanelRow::Command(CommandRef::new(GENERATE)),
        PanelRow::Separator,
        PanelRow::Submenu(units.into(), CssUnits::ALL.iter().map(|u| PanelRow::Command(units_ref(*u))).collect()),
    ];
    rows.extend(FLAGS.iter().map(|name| PanelRow::Command(option_ref(name))));
    rows
}

/// The rows' states: the copy and export of the selection need a selection, Export All and Generate CSS a
/// document; Generate CSS, the units and the options show what is set.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    let panel = &app.ui.css_properties_panel;
    let selection = app.session.active().map(|st| !st.selection.is_empty());
    let (has_doc, selected) = (selection.is_some(), selection == Some(true));
    states.set(CommandRef::new(COPY_SELECTED), CommandState::new().enabled(selected));
    states.set(CommandRef::new(EXPORT_SELECTED), CommandState::new().enabled(selected));
    states.set(CommandRef::new(EXPORT_ALL), CommandState::new().enabled(has_doc));
    states.set(CommandRef::new(GENERATE), CommandState::new().enabled(has_doc).checked(panel.all));
    for units in CssUnits::ALL {
        states.set(units_ref(units), CommandState::new().checked(panel.options.units == units));
    }
    let mut options = panel.options.clone();
    for name in FLAGS {
        let on = flag_mut(&mut options, name).is_some_and(|on| *on);
        states.set(option_ref(name), CommandState::new().checked(on));
    }
}

/// The panel menu's `ui.cssProperties.*` commands; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    let name = id.strip_prefix("ui.cssProperties.")?;
    let opts = app.ui.css_properties_panel.options.clone();
    Some(match name {
        "copySelected" => copy(app, &params(&opts, "selection")),
        "exportSelected" => export_file(app, &params(&opts, "selection")),
        "exportAll" => export_file(app, &params(&opts, "all")),
        "generate" => {
            let panel = &mut app.ui.css_properties_panel;
            panel.all = !panel.all;
            Ok(json!({ "all": panel.all }))
        }
        "units" => serde_json::from_value::<CssUnits>(p["units"].clone()).map_err(|_| "units: px, pt, mm, cm or in".to_string()).map(|units| {
            app.ui.css_properties_panel.options.units = units;
            json!({ "units": units })
        }),
        "option" => match p["option"].as_str().and_then(|n| flag_mut(&mut app.ui.css_properties_panel.options, n)) {
            Some(on) => {
                *on = !*on;
                Ok(json!({ "on": *on }))
            }
            None => Err(format!("option: one of {}", FLAGS.join(", "))),
        },
        _ => return None,
    })
}
