//! Object › Plug-ins › Plug-ins That Didn't Load… ([`open`]): the plug-in files that didn't load in
//! this session, each with why (`plugin.list`'s `failed`), and Retry All, which loads them again
//! (`plugin.retry`), or Close. The status bar then says how many loaded.
//!
//! The list is a prompt on the window's dialog host; [`pump`] reads its answer each frame.

use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};
use vectorcraft_engine::file_access::ScopeHandle;

use crate::VectorcraftApp;

/// The key of the list.
pub const KIND: &str = "pluginsFailed";

/// The UI command that opens the list.
pub const COMMAND: &str = "ui.failedPlugins";

/// The most files the list names; a line counts the others.
const MAX_LINES: usize = 50;

/// The list of the plug-ins that didn't load; `StandardPrompt` has one id and one result type for
/// every custom prompt, so it is a type of its own.
pub struct FailedPluginsList(StandardPrompt);

super::confirm::wrapped_dialog!(FailedPluginsList, StandardPrompt, KIND);

/// The tag of the list: the access Retry All runs under.
struct Opened {
    /// The automation roots in force when the list opened: Retry All runs under them.
    scope: ScopeHandle,
    /// Automation opened it ([`VectorcraftApp::automation_asked`]): Retry All runs as automation.
    automated: bool,
}

/// Opens the list → `{pending, failed: the count}`; an error while every plug-in loaded.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let list = app.run("plugin.list", json!({}))?;
    let failed = list.get("failed").and_then(Value::as_array).cloned().unwrap_or_default();
    if failed.is_empty() {
        return Err(tl!("No plug-in failed to load").to_string());
    }
    let Some(dialogs) = app.dialogs() else { return Err("no window".into()) };
    let mut lines: Vec<Label> = failed.iter().take(MAX_LINES).map(line).collect();
    if failed.len() > MAX_LINES {
        let more = (failed.len() - MAX_LINES).to_string();
        lines.push(Label::from(crate::i18n::fmt(tl!("{count} more"), &[("count", &more)])));
    }
    let count = failed.len().to_string();
    let prompt = lines
        .into_iter()
        .fold(StandardPrompt::custom(crate::i18n::fmt(tl!("Plug-ins that didn't load: {count}"), &[("count", &count)])), StandardPrompt::paragraph)
        .button(PromptChoice::Proceed, tl!("Retry All"))
        .button(PromptChoice::Cancel, tl!("Close"))
        .tag(Opened { scope: ScopeHandle::capture(), automated: app.automation_asked() });
    dialogs.with(|host| host.open(FailedPluginsList(prompt)));
    Ok(json!({ "pending": KIND, "failed": failed.len() }))
}

/// A row of `plugin.list`'s `failed` as the list shows it: the file's name and why it didn't load.
fn line(row: &Value) -> Label {
    let file = row.get("file").and_then(Value::as_str).unwrap_or_default();
    let name = std::path::Path::new(file).file_name().map_or_else(|| file.to_string(), |n| n.to_string_lossy().into_owned());
    let error = row.get("error").and_then(Value::as_str).unwrap_or_default();
    Label::from(crate::i18n::fmt(tl!("{file}: {error}"), &[("file", &name), ("error", error)]))
}

/// Acts on the answer the list got this frame: Retry All loads every listed file again.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(DialogResult::Ok(mut answer)) = dialogs.with(|host| host.take::<FailedPluginsList>())
        && answer.choice == PromptChoice::Proceed
        && let Some(Opened { scope, automated }) = answer.take_tag::<Opened>()
    {
        let retried = scope.run(|| crate::file_asks::as_automated(app, automated, |app| app.run("plugin.retry", json!({}))));
        report_retry(app, retried);
    }
}

/// Says in the status bar how many of the plug-ins Retry All loaded again ([`pump`]).
fn report_retry(app: &mut VectorcraftApp, retried: Result<Value, String>) {
    match retried {
        Ok(r) => {
            let retried = r.get("retried").and_then(Value::as_array).cloned().unwrap_or_default();
            let loaded = retried.iter().filter(|row| row.get("loaded") == Some(&Value::Bool(true))).count().to_string();
            let count = retried.len().to_string();
            app.status(crate::i18n::fmt(tl!("Plug-ins loaded on retry: {loaded} of {count}"), &[("loaded", &loaded), ("count", &count)]));
        }
        Err(e) => app.status(e),
    }
}

#[cfg(test)]
mod tests;
