//! Attributes panel › Panel Options…: how many recent URLs the panel's URL menu lists.
//!
//! Field: `count` (1–30). OK runs `ui.attributesPanelOptions` with it (kept with the UI state).

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::RECENT_URLS;

use crate::VectorcraftApp;
use crate::dialogs::host::open_held;
use crate::panels::attributes::{DEFAULT_RECENT_URLS, side_name};

/// The dialog's key: the command's dialog name and the one its fields are set under.
pub const KEY: &str = "attributesPanelOptions";

/// `ui.attributesPanelOptions` without options: open the dialog on the panel's options. OK runs
/// the command, and a count it refuses keeps the dialog open with the reason.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let mut fields = DialogFields::new();
    fields.set("count", app.ui.attributes_panel.shown() as f64);
    let dialog = HostedParamDialog::new(&ctx, KEY, fields, |fields| {
        let count = Param::number("count", side_name(tl!("Number of Recent URLs:")), f64::from(DEFAULT_RECENT_URLS), 1.0, 0)
            .bounds(Some(1.0), Some(RECENT_URLS as f64));
        ParamDialog::command(KEY, tl!("Attributes Panel Options"), vec![count]).values(Bind::bound(fields))
    });
    open_held(app, dialog, |app, answer| {
        // A missing count goes on as null, which the command refuses by its own rule.
        let count = answer.fields.number("count").map_or(Value::Null, |n| json!(n));
        app.run("ui.attributesPanelOptions", json!({ "count": count })).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::ui::dialog::dialog_id;
    use vectorcraft_engine::Session;

    /// The window's dialogs run and the held answers drain, as in the app's frame.
    fn frame(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
        app.ui_ctx = Some(ui.ctx().clone());
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.run(ui));
        }
        crate::dialogs::host::pump(app, ui.ctx());
    }

    /// The dialog opens on the panel's count (10 by default), OK sets it, kept with the UI state,
    /// and a count the command refuses is refused, leaving the dialog open and the count as it was.
    #[test]
    fn panel_options_set_the_recent_url_count() {
        let mut panel = Panel::new(VectorcraftApp::new(Session::new(), Default::default()), frame);
        panel.frame();
        let (id, ctx) = (dialog_id(KEY), panel.ctx().clone());
        let count = |panel: &Panel| panel.app().ui.attributes_panel.recent_urls;
        let is_open = |panel: &Panel| panel.app().dialogs().is_some_and(|d| d.with(|host| host.get_mut_by_id::<HostedParamDialog>(id).is_some()));
        let set = |panel: &Panel, value: Value| panel.app().dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, "count", value));
        // OK files a held answer, which the next frames apply and close on; Cancel closes at once.
        let press = |panel: &mut Panel, ok: bool| {
            let closed = panel.app().dialogs().unwrap().with(|host| if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) });
            assert!(ok || closed);
            panel.frame();
            panel.frame();
        };
        panel.app().ui.attributes_panel.recent_urls = 25;
        panel.app().run("ui.attributesPanelOptions", json!({})).unwrap();
        panel.frame();
        assert!(is_open(&panel));
        // It opens on the current count: OK with nothing changed keeps 25 (it would set 10 otherwise).
        press(&mut panel, true);
        assert_eq!((count(&panel), is_open(&panel)), (25, false));
        // A count out of range is refused at the field, or by the command with the dialog left open.
        panel.app().run("ui.attributesPanelOptions", json!({})).unwrap();
        panel.frame();
        let refused = set(&panel, json!(31));
        press(&mut panel, true);
        assert_eq!(count(&panel), 25, "a refused count changes nothing");
        assert!(refused.is_err() || is_open(&panel), "a count the command refuses keeps the dialog open");
        panel.app().run("ui.attributesPanelOptions", json!({})).unwrap();
        panel.frame();
        set(&panel, json!(12)).unwrap();
        press(&mut panel, true);
        assert_eq!((count(&panel), is_open(&panel)), (12, false));
        // Cancel closes the dialog without applying what was typed.
        panel.app().run("ui.attributesPanelOptions", json!({})).unwrap();
        panel.frame();
        set(&panel, json!(3)).unwrap();
        press(&mut panel, false);
        assert_eq!((count(&panel), is_open(&panel)), (12, false));
    }
}
