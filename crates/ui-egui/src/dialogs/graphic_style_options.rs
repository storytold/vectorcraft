//! Graphic Style Options: the style's name. Opened on a style it renames it (`graphicStyle.rename`,
//! linked objects stay linked); opened for New Graphic Style it names the style made from the
//! selection (`graphicStyle.new`); opened for Merge Graphic Styles it names the style merged from
//! the selected ones (`graphicStyle.merge`).
//!
//! The one field is `name`. The style being renamed or the styles to merge are held by the dialog's
//! OK, not by a field, so a name another style has refuses the OK and keeps the dialog open.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use serde_json::{Value, json};

use super::host;
use crate::VectorcraftApp;

/// The dialog kind of Graphic Style Options.
pub const KIND: &str = "graphicStyleOptions";

/// What OK does with the name.
enum Target {
    /// Name the style made from the selection.
    New,
    /// Name the style merged from these styles.
    Merge(Vec<String>),
    /// Rename this style.
    Rename(String),
}

impl Target {
    /// Run the command the target stands for; a name another style has comes back as the error.
    fn run(&self, app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
        match self {
            Target::Merge(names) => app.run("graphicStyle.merge", json!({ "names": names, "name": name })),
            Target::New => app.run("graphicStyle.new", json!({ "name": name })),
            Target::Rename(style) if style == name => Ok(Value::Null),
            Target::Rename(style) => app.run("graphicStyle.rename", json!({ "name": style, "to": name })),
        }
    }
}

/// Open the dialog with `name` in its field; OK runs `target`.
fn open_with(app: &mut VectorcraftApp, name: String, target: Target) -> Result<Value, String> {
    let params = vec![Param::new("name", tl!("Style Name:").to_string(), ParamKind::text(name.clone()))];
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::command(&ctx, KIND, DialogFields::from_pairs([("name", name)]), move |fields| {
        ParamDialog::command(KIND, tl!("Graphic Style Options").to_string(), params.clone()).values(Bind::bound(fields))
    });
    // An `Err` (a name another style has) keeps the dialog open with the reason.
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| target.run(app, result.fields.text("name").unwrap_or_default()).map(|_| ());
    host::open_held(app, dialog, ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// Open Graphic Style Options for style `name`, or (`None`) to name a new style from the selection.
pub fn open(app: &mut VectorcraftApp, name: Option<&str>) -> Result<Value, String> {
    let d = &app.session.active().ok_or("no document open")?.doc;
    match name {
        Some(n) => {
            let style = d.graphic_style(n).ok_or_else(|| format!("no graphic style `{n}`"))?.name.clone();
            open_with(app, style.clone(), Target::Rename(style))
        }
        None => {
            let name = d.new_graphic_style_name();
            open_with(app, name, Target::New)
        }
    }
}

/// Open Graphic Style Options to name the style Merge Graphic Styles makes of `names`.
pub fn open_merge(app: &mut VectorcraftApp, names: Vec<String>) -> Result<Value, String> {
    let d = &app.session.active().ok_or("no document open")?.doc;
    if names.len() < 2 {
        return Err("select two or more graphic styles to merge".into());
    }
    if let Some(n) = names.iter().find(|n| d.graphic_style(n).is_none()) {
        return Err(format!("no graphic style `{n}`"));
    }
    let name = d.new_graphic_style_name();
    open_with(app, name, Target::Merge(names))
}

#[cfg(test)]
mod tests {
    use super::super::tests_warp::{accept, set, windowed};
    use super::*;
    use craft::ui::dialog::DialogId;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})).unwrap();
        app
    }

    fn names(app: &VectorcraftApp) -> Vec<String> {
        app.session.active().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect()
    }

    /// The keys of the open dialogs, the top one last.
    fn open_keys(app: &VectorcraftApp) -> Vec<Option<String>> {
        crate::tests::picks::open_dialogs(app).into_iter().map(|d| d.key).collect()
    }

    /// The open Graphic Style Options.
    fn options(app: &VectorcraftApp) -> DialogId {
        crate::tests::picks::open_dialogs(app)
            .into_iter()
            .find(|d| d.key.as_deref() == Some(KIND))
            .map(|d| d.id)
            .expect("Graphic Style Options is open")
    }

    #[test]
    fn names_a_new_style_then_renames_it() {
        let mut app = app();
        app.run("ui.graphicStyleOptions", json!({})).unwrap();
        assert_eq!(open_keys(&app), [Some(KIND.to_string())]);
        let id = options(&app);
        set(&app, id, "name", json!("Glow"));
        accept(&mut app, id);
        assert!(open_keys(&app).is_empty() && names(&app).last().unwrap() == "Glow");

        app.run("ui.graphicStyleOptions", json!({"name": "Glow"})).unwrap();
        let id = options(&app);
        set(&app, id, "name", json!("Sunshine"));
        accept(&mut app, id);
        assert_eq!(open_keys(&app), [Some(KIND.to_string())], "a taken name keeps the dialog open");
        assert!(!names(&app).iter().any(|n| n == "Glow copy"));
        set(&app, id, "name", json!("Halo"));
        accept(&mut app, id);
        assert_eq!(names(&app).last().unwrap(), "Halo");
        assert!(app.run("ui.graphicStyleOptions", json!({"name": "nope"})).is_err());
    }
}
