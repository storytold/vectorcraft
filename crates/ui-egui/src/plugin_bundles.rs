//! Shared plug-ins in the UI (`craft-plugin`, RFC-0041): command plug-ins' commands in Object ›
//! Plug-ins, Manage Plug-ins… (the command plug-ins installed, installing from a `.craftplugin`
//! file, a GitHub repository or a catalog, updates and catalogs), the install confirmation and a
//! parameter dialog generated from a command's `params`. Everything goes through the engine's
//! `plugin.*` commands (`vectorcraft_engine::cmd::craftplugin`).
//!
//! The confirmation is VectorCraft's own install approval ([`dialogs::plugin::ask_install`]): it
//! lists the kind, the source, the bundle's SHA-256 and the permissions, says plainly when a
//! repository is in no catalog in use, and only a person's OK installs. Downloads from GitHub run on
//! a thread of their own ([`Downloads`]), so the window never waits for the network; the bundle is
//! then handed to the session ([`vectorcraft_engine::Session::hand_in_plugin_bundle`]) and is not
//! downloaded again for the install.

use serde_json::{Map, Value, json};
use vectorcraft_engine::cmd::craftplugin::{self, craft_plugin};
use vectorcraft_plugins::permissions;

use crate::VectorcraftApp;
use crate::dialogs::{self, DialogAction, DialogSpec};
use crate::menus::Item;
use crate::state::Dialog;
use crate::widgets;

/// Manage Plug-ins.
pub const MANAGE_KIND: &str = "pluginManage";
/// A command plug-in command's parameters.
pub const PARAMS_KIND: &str = "pluginCommand";
/// Runs a command plug-in's command from the menu, asking for its parameters first: `{id, command}`.
pub const RUN_FROM_MENU: &str = "ui.pluginCommand";
/// Opens Manage Plug-ins.
pub const MANAGE_COMMAND: &str = "ui.managePlugins";

/// HTTPS for catalogs and GitHub releases, over the client VectorCraft checks for updates with.
#[cfg(not(target_arch = "wasm32"))]
struct UpdateHttp(craft_update::Http);

#[cfg(not(target_arch = "wasm32"))]
impl craft_plugin::Fetch for UpdateHttp {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> {
        use craft_update::http::Fetched;
        if !url.starts_with("https://api.github.com/") {
            return self.0.get_bytes(url, max);
        }
        match self.0.get_api(url, None)? {
            Fetched::Body { text, .. } if text.len() > max => Err(format!("GitHub's answer is larger than {max} bytes")),
            Fetched::Body { text, .. } => Ok(Some(text.into_bytes())),
            Fetched::NotFound => Ok(None),
            Fetched::RateLimited { .. } => Err("GitHub's API limit for this hour is used up: try again later".into()),
            Fetched::NotModified => Err("GitHub answered 304 Not Modified to a request without a cache".into()),
        }
    }
}

/// An HTTPS client for plug-in downloads, when one can be made (never on the web).
pub fn http_fetch() -> Option<Box<dyn craft_plugin::Fetch + Send>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let agent = format!("VectorCraft/{}", env!("CARGO_PKG_VERSION"));
        craft_update::Http::new(&agent).map_err(|e| log::warn!("plug-in downloads: {e}")).ok().map(|h| Box::new(UpdateHttp(h)) as _)
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// Give `session` the HTTPS client for catalogs and GitHub releases (the desktop app at start).
pub fn attach_fetch(session: &mut vectorcraft_engine::Session) {
    if let Some(f) = http_fetch() {
        session.set_plugin_fetch(f);
    }
}

/// Bundles downloading on threads of their own: the `plugin.install` parameters that named each,
/// and where its bundle (or why there is none) arrives.
#[derive(Default)]
pub struct Downloads {
    running: Vec<(Value, std::sync::mpsc::Receiver<Result<craftplugin::Candidate, String>>)>,
    /// A command a Manage Plug-ins button asked for, run by the next [`poll`] (the dialog's body
    /// can't open another dialog while it draws).
    queued: Option<(&'static str, Value)>,
}

impl Downloads {
    /// Whether a download is running.
    pub fn busy(&self) -> bool {
        !self.running.is_empty()
    }
}

/// Whether `p` names a shared bundle (a `.craftplugin` file, `data`, `repo` or `catalog`).
pub fn names_bundle(p: &Value) -> bool {
    craftplugin::names_bundle(p)
}

/// Installs the bundle `p` names, asking first ([`confirm`]). A repository or a catalog's package
/// downloads on a thread of its own first (on the desktop) → `{pending: "download"}` meanwhile.
pub fn install_bundle(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let network = ["repo", "catalog"].iter().any(|k| p.get(*k).and_then(Value::as_str).is_some_and(|v| !v.trim().is_empty()));
    #[cfg(not(target_arch = "wasm32"))]
    if network && let Some(mut fetch) = http_fetch() {
        let request = p.clone();
        let catalogs = app.session.command_plugins().catalogs().to_vec();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("vectorcraft-plugin-download".into())
            .spawn(move || {
                let got = craftplugin::obtain(&mut *fetch, &catalogs, "plugin.install", &request).map_err(|e| e.to_string());
                // The app may have quit meanwhile: nothing to hand it to.
                let _ = tx.send(got);
            })
            .map_err(|e| e.to_string())?;
        app.plugin_downloads.running.push((p.clone(), rx));
        app.status(tl!("Downloading plug-in…").to_string());
        return Ok(json!({"pending": "download"}));
    }
    let _ = network;
    confirm(app, p)
}

/// Hands in the bundles downloaded (each frame) and asks whether to install each. Not while a
/// dialog other than Manage Plug-ins is open: the question replaces it.
pub fn poll(app: &mut VectorcraftApp) {
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind != MANAGE_KIND) {
        return;
    }
    if let Some((id, p)) = app.plugin_downloads.queued.take() {
        run_queued(app, id, p);
    }
    let mut i = 0;
    while let Some((_, rx)) = app.plugin_downloads.running.get(i) {
        let got = match rx.try_recv() {
            Ok(got) => got,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                i += 1;
                continue;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err("the download stopped".to_string()),
        };
        let (request, _) = app.plugin_downloads.running.remove(i);
        let asked = got.and_then(|c| {
            app.session.hand_in_plugin_bundle(c);
            confirm(app, &request)
        });
        if let Err(e) = asked {
            app.status(crate::i18n::fmt(tl!("Couldn't install the plug-in: {e}"), &[("e", &e)]));
        }
        return;
    }
}

/// Shows what the bundle `p` names is (`plugin.install {review: true}`) and asks a person whether
/// to install it ([`dialogs::plugin::ask_install`]); OK installs that very bundle (its SHA-256).
fn confirm(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let origin = if app.automation_asked() { dialogs::plugin::InstallOrigin::Automation } else { dialogs::plugin::InstallOrigin::Person };
    let mut request = craftplugin::request_key(p);
    request["review"] = json!(true);
    let info = app.run("plugin.install", request.clone())?;
    let shown = |v: &Value, max: usize| permissions::display_text(v.as_str().unwrap_or_default(), max);
    let name = shown(&info["name"], permissions::MAX_SHOWN_NAME);
    let mut lines = vec![
        crate::i18n::fmt(tl!("ID: {id}"), &[("id", &shown(&info["id"], permissions::MAX_SHOWN_FIELD))]),
        crate::i18n::fmt(tl!("Version: {version}"), &[("version", &shown(&info["version"], permissions::MAX_SHOWN_FIELD))]),
        crate::i18n::fmt(tl!("Kind: {kind}"), &[("kind", &shown(&info["kind"], permissions::MAX_SHOWN_FIELD))]),
        crate::i18n::fmt(tl!("Source: {source}"), &[("source", &source_words(&info["source"]))]),
        crate::i18n::fmt(tl!("Bundle SHA-256: {hash}"), &[("hash", &shown(&info["sha256"], 64))]),
    ];
    let mut install = request;
    install["review"] = json!(false);
    install["sha256"] = info["sha256"].clone();
    let mut approval = vectorcraft_engine::cmd::plugin::InstallApproval { id: String::new(), hash: String::new(), permissions: vec![], heavy: false };
    let mut paragraphs = vec![];
    if info["kind"] == craft_plugin::KIND_COMMAND {
        let p = &info["permissions"];
        lines.push(String::new());
        lines.push(if p["access"] == "write" {
            tl!("• Read and change the document through VectorCraft's commands").to_string()
        } else {
            tl!("• Read the document through VectorCraft's commands").to_string()
        });
        let commands: Vec<String> = p["commands"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|c| permissions::display_text(c, permissions::MAX_SHOWN_FIELD))
            .collect();
        lines.push(if commands.is_empty() {
            tl!("• Any command a plug-in may call").to_string()
        } else {
            crate::i18n::fmt(tl!("• Only these commands: {ids}"), &[("ids", &commands.join(", "))])
        });
        if info["permissionsChanged"] == true {
            paragraphs.push(tl!("It asks for other permissions than the version installed.").to_string());
        }
    } else {
        // One of VectorCraft's own kinds: what its module asks for, granted by this OK.
        let module = &info["module"];
        let asked: Vec<String> = module["permissions"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        let allowlist: Vec<String> =
            module["commandAllowlist"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
        if !asked.is_empty() {
            lines.push(String::new());
            lines.extend(asked.iter().map(|p| format!("• {}", crate::io::permission_words(p, &allowlist))));
            install["grant"] = json!(asked);
        }
        let heavy = module["budget"] == "heavy";
        if heavy {
            install["allowHeavy"] = json!(true);
        }
        install["replace"] = json!(true);
        approval = vectorcraft_engine::cmd::plugin::InstallApproval {
            id: module["id"].as_str().unwrap_or_default().to_string(),
            hash: module["hash"].as_str().unwrap_or_default().to_string(),
            permissions: asked,
            heavy,
        };
    }
    if let Some(v) = info["replaces"].as_str() {
        paragraphs.push(crate::i18n::fmt(tl!("It replaces version {version}, installed now."), &[("version", &permissions::display_text(v, 40))]));
    }
    if info["inCatalog"] == false {
        paragraphs.push(tl!("This repository is not in any catalog you use. Install it only if you trust its author.").to_string());
    }
    paragraphs.insert(0, lines.join("\n"));
    let message = crate::i18n::fmt(tl!("Install the plug-in “{name}”?"), &[("name", &name)]);
    let pending =
        dialogs::plugin::PendingInstall { message, detail: paragraphs.join("\n\n"), params: install, approval, origin, shown: Default::default() };
    Ok(dialogs::plugin::ask_install(app, pending))
}

/// Where a bundle came from, in words (`plugin.install`'s `source`).
fn source_words(source: &Value) -> String {
    let t = |k: &str| permissions::display_text(source.get(k).and_then(Value::as_str).unwrap_or_default(), permissions::MAX_SHOWN_FIELD);
    match source.get("type").and_then(Value::as_str) {
        Some("github") => crate::i18n::fmt(tl!("GitHub repository {repo}, release {tag}"), &[("repo", &t("repo")), ("tag", &t("tag"))]),
        Some("catalog") => crate::i18n::fmt(tl!("Catalog {catalog}, package {package}"), &[("catalog", &t("catalog")), ("package", &t("package"))]),
        _ => crate::i18n::fmt(tl!("File {name}"), &[("name", &t("path"))]),
    }
}

// ---------- menus ----------

/// Object › Plug-ins' items for the command plug-ins: each one's command (or a submenu of its
/// commands), and Manage Plug-ins…. Names are the plug-ins' own, never translated.
pub fn menu_items() -> (Vec<Item>, Item) {
    let mut items = vec![];
    for p in craftplugin::menu_entries() {
        let name = permissions::display_text(p.name.trim(), permissions::MAX_SHOWN_NAME);
        let item = |label: &str, command: &str, asks: bool| {
            let dots = if asks && !label.ends_with('…') { "…" } else { "" };
            Item::Cmd(vectorcraft_plugins::registry::intern(&format!("{label}{dots}")), RUN_FROM_MENU, json!({"id": p.id, "command": command}))
        };
        match p.commands.as_slice() {
            [(command, _, asks)] => items.push(item(&name, command, *asks)),
            many => {
                let sub = many
                    .iter()
                    .map(|(command, label, asks)| item(&permissions::display_text(label, permissions::MAX_SHOWN_NAME), command, *asks))
                    .collect();
                items.push(Item::Sub(vectorcraft_plugins::registry::intern(&name), sub));
            }
        }
    }
    (items, Item::Cmd("Manage Plug-ins…", MANAGE_COMMAND, Value::Null))
}

/// `ui.pluginCommand {id, command}`: runs the command at once when it takes no parameters (or
/// `params` are given), else opens its parameter dialog with the defaults.
pub fn run_from_menu(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let id = p.get("id").and_then(Value::as_str).ok_or("missing `id` (see plugin.list)")?.to_string();
    let command = p.get("command").and_then(Value::as_str).map(str::to_string);
    let plugin = app.session.command_plugins().get(&id).cloned().ok_or_else(|| format!("no command plug-in {id:?} is installed"))?;
    let c = plugin.manifest.command(command.as_deref()).ok_or_else(|| format!("{} has no command {command:?}", plugin.manifest.name))?;
    let fields: Vec<&craft_plugin::Param> = c.params.iter().filter(|x| x.is_field()).collect();
    if fields.is_empty() || p.get("params").is_some_and(|v| v.as_object().is_some_and(|o| !o.is_empty())) {
        let params = p.get("params").cloned().unwrap_or_else(|| json!({}));
        return app.run("plugin.run", json!({"id": id, "command": c.id, "params": params}));
    }
    let mut values = Map::new();
    for f in &fields {
        values.insert(f.name.clone(), f.default_value());
    }
    values.insert("__plugin".into(), json!(id));
    values.insert("__command".into(), json!(c.id));
    values.insert("__label".into(), json!(permissions::display_text(c.label.trim_end_matches('…'), permissions::MAX_SHOWN_NAME)));
    app.ui.dialog = Some(Dialog::new(PARAMS_KIND, Value::Object(values)));
    Ok(json!({"dialog": PARAMS_KIND}))
}

/// The parameter dialog, headed by the command's label as the plug-in wrote it.
pub(crate) const PARAMS_SPEC: DialogSpec =
    DialogSpec { heading: |d| d.str("__label"), body: params_body, confirm: params_confirm, max_width: Some(420.0), ..DialogSpec::FORM };

/// One row per parameter of the command the dialog is for.
fn params_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let (id, command) = (d.str("__plugin"), d.str("__command"));
    let Some(plugin) = app.session.command_plugins().get(&id).cloned() else { return true };
    let Some(c) = plugin.manifest.command(Some(&command)) else { return true };
    egui::Grid::new("plugin-command-params").num_columns(2).spacing([10.0, 8.0]).show(ui, |ui| {
        for f in c.params.iter().filter(|x| x.is_field()) {
            let title = permissions::display_text(f.title(), permissions::MAX_SHOWN_NAME);
            ui.label(&title);
            let v = d.fields.get(&f.name).cloned().unwrap_or_else(|| f.default_value());
            let new = match f.kind {
                craft_plugin::ParamKind::Boolean => {
                    let mut on = v.as_bool().unwrap_or(false);
                    ui.checkbox(&mut on, "").changed().then(|| json!(on))
                }
                craft_plugin::ParamKind::Number | craft_plugin::ParamKind::Slider => {
                    let mut n = v.as_f64().filter(|n| n.is_finite()).unwrap_or(0.0);
                    let range = f.min.filter(|m| m.is_finite()).unwrap_or(f64::MIN)..=f.max.filter(|m| m.is_finite()).unwrap_or(f64::MAX);
                    ui.add(egui::DragValue::new(&mut n).range(range)).changed().then(|| json!(n))
                }
                craft_plugin::ParamKind::Select | craft_plugin::ParamKind::List => {
                    let mut chosen = v.as_str().unwrap_or_default().to_string();
                    let before = chosen.clone();
                    craft_ui::ComboBox::from_id_salt(("plugin-param", &f.name)).selected_text(permissions::display_text(&chosen, 60)).show_ui(
                        ui,
                        |ui| {
                            for o in &f.options {
                                ui.selectable_value(&mut chosen, o.clone(), permissions::display_text(o, 60));
                            }
                        },
                    );
                    (chosen != before).then(|| json!(chosen))
                }
                _ => widgets::text_field_named(ui, ("plugin-param", &f.name), Some(&title), v.as_str(), 200.0, 1).map(|t| json!(t)),
            };
            if let Some(new) = new {
                d.fields.insert(f.name.clone(), new);
            }
            ui.end_row();
        }
    });
    false
}

/// OK: runs the command with the dialog's values (`plugin.run`), one undo step.
fn params_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params: Map<String, Value> = d.fields.iter().filter(|(k, _)| !k.starts_with("__")).map(|(k, v)| (k.clone(), v.clone())).collect();
    let r = app.run("plugin.run", json!({"id": d.str("__plugin"), "command": d.str("__command"), "params": params}))?;
    app.ui.dialog = None;
    for note in r["notes"].as_array().into_iter().flatten().filter_map(Value::as_str).take(1) {
        app.status(permissions::display_text(note, 200));
    }
    Ok(r)
}

// ---------- Manage Plug-ins ----------

/// `ui.managePlugins`: opens Manage Plug-ins.
pub fn open_manage(app: &mut VectorcraftApp) -> Result<Value, String> {
    app.ui.dialog = Some(Dialog::new(MANAGE_KIND, json!({"repo": "", "catalog": ""})));
    Ok(json!({"dialog": MANAGE_KIND}))
}

/// Manage Plug-ins has its own buttons and a Close.
pub(crate) const MANAGE_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Manage Plug-ins…").trim_end_matches('…').to_string(),
    body_with_actions: Some(manage_body),
    confirm: |app, _| {
        app.ui.dialog = None;
        Ok(Value::Null)
    },
    ok: None,
    min_width: 460.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

/// The installed command plug-ins (Remove, Check for Update), Install from File…, a repository
/// field, the catalogs and their packages, and Close.
fn manage_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let mut action = DialogAction::None;
    let mut run: Option<(&str, Value)> = None;
    let dim = crate::theme::Tokens::get(ui.ctx()).text_dim;
    ui.label(egui::RichText::new(tl!("Installed command plug-ins")).strong());
    let installed = app.session.command_plugins().list().to_vec();
    if installed.is_empty() {
        ui.label(egui::RichText::new(tl!("None installed yet.")).color(dim));
    }
    for p in &installed {
        ui.horizontal(|ui| {
            let name = permissions::display_text(&p.manifest.name, permissions::MAX_SHOWN_NAME);
            ui.label(format!("{name} {}", permissions::display_text(&p.manifest.version, 40)));
            if widgets::secondary_button(ui, "Remove").clicked() {
                run = Some(("plugin.remove", json!({"id": p.manifest.id})));
            }
            if p.source.get("type").and_then(Value::as_str).is_some_and(|t| t != "file") && widgets::secondary_button(ui, "Update").clicked() {
                run = Some(("plugin.update", json!({"id": p.manifest.id, "install": true})));
            }
        });
    }
    for problem in app.session.command_plugins().problems() {
        ui.label(egui::RichText::new(permissions::display_text(problem, 300)).color(dim));
    }
    ui.add_space(12.0);
    ui.label(egui::RichText::new(tl!("Install")).strong());
    if widgets::secondary_button(ui, "Install from File…").clicked() {
        run = Some(("ui.installPlugin", Value::Null));
    }
    ui.horizontal(|ui| {
        let typed = widgets::text_field_named(ui, "plugin-repo", Some(tl!("GitHub repository (owner/repo):")), Some(&d.str("repo")), 220.0, 1);
        if let Some(t) = typed {
            d.fields.insert("repo".into(), json!(t));
        }
        let repo = d.str("repo");
        if widgets::primary_button_enabled(ui, "Install", !repo.trim().is_empty() && !app.plugin_downloads.busy()).clicked() {
            run = Some(("ui.installPlugin", json!({"repo": repo.trim()})));
        }
    });
    if app.plugin_downloads.busy() {
        ui.label(egui::RichText::new(tl!("Downloading plug-in…")).color(dim));
    }
    ui.add_space(12.0);
    ui.label(egui::RichText::new(tl!("Catalogs")).strong());
    for url in app.session.command_plugins().catalogs().to_vec() {
        ui.horizontal(|ui| {
            ui.label(permissions::display_text(&url, 80));
            if widgets::secondary_button(ui, "Remove").clicked() {
                run = Some(("plugin.catalog.add", json!({"url": url, "remove": true})));
            }
        });
    }
    ui.horizontal(|ui| {
        let typed = widgets::text_field_named(ui, "plugin-catalog", Some(tl!("Catalog URL:")), Some(&d.str("catalog")), 260.0, 1);
        if let Some(t) = typed {
            d.fields.insert("catalog".into(), json!(t));
        }
        let url = d.str("catalog");
        if widgets::secondary_button(ui, "Add").clicked() && !url.trim().is_empty() {
            run = Some(("plugin.catalog.add", json!({"url": url.trim()})));
        }
    });
    if !app.session.command_plugins().catalogs().is_empty() && widgets::secondary_button(ui, "Show Packages").clicked() {
        run = Some(("plugin.catalog.list", json!({})));
    }
    for pkg in d.fields.get("__packages").and_then(Value::as_array).cloned().unwrap_or_default() {
        ui.horizontal(|ui| {
            let t = |k: &str| permissions::display_text(pkg[k].as_str().unwrap_or_default(), permissions::MAX_SHOWN_NAME);
            ui.label(format!("{} {} ({})", t("name"), t("version"), t("kind")));
            if widgets::secondary_button(ui, "Install").clicked() {
                run = Some(("ui.installPlugin", json!({"catalog": pkg["catalog"], "id": pkg["id"]})));
            }
        });
    }
    ui.add_space(16.0);
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::secondary_button(ui, "Close").clicked() {
            action = DialogAction::Cancel;
        }
    });
    if run.is_some() {
        app.plugin_downloads.queued = run;
    }
    action
}

/// Runs what a Manage Plug-ins button asked for, and shows its answer.
fn run_queued(app: &mut VectorcraftApp, id: &str, p: Value) {
    {
        match app.run(id, p) {
            Ok(r) if id == "plugin.catalog.list" => {
                if let Some(d) = app.ui.dialog.as_mut().filter(|d| d.kind == MANAGE_KIND) {
                    d.fields.insert("__packages".into(), r["packages"].clone());
                }
            }
            Ok(r) if id == "plugin.update" => {
                let row = r["plugins"].get(0).cloned().unwrap_or_default();
                let said = if row["updated"] == true {
                    tl!("Updated.")
                } else if row["needsReview"] == true {
                    tl!("A newer version asks for other permissions: install it again to review them.")
                } else if row["upToDate"] == true {
                    tl!("Up to date.")
                } else {
                    tl!("Couldn't check for an update.")
                };
                app.status(said.to_string());
            }
            Ok(_) => {}
            Err(e) => app.status(e),
        }
    }
}
