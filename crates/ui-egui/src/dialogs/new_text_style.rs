//! New Character Style and New Paragraph Style: the new style's name and the sections of Character
//! or Paragraph Style Options, filled in from the text that was selected when the dialog opened.
//! OK makes the style with those attributes (`charStyle.new`, `paraStyle.new`).
//!
//! The dialog's `target` is the `id`, `start` and `end` of the Type tool's selection (empty for the
//! selected objects) and the `attrs` that text had when the dialog opened.

use std::sync::Arc;

use craft::ui::patterns::form_dialogs::HostedPropertiesDialog;
use serde_json::{Map, Value, json};

use super::char_style_options::{self, reason, style_dialog};
use super::para_style_options;
use crate::VectorcraftApp;

/// The dialog kind of New Character Style and New Paragraph Style.
pub const KIND: &str = "newTextStyle";

/// What the dialog opened with: the character or the paragraph side's.
enum Opening {
    Char(Arc<char_style_options::Opening>),
    Para(Arc<para_style_options::Opening>),
}

/// The open New Character Style or New Paragraph Style dialog.
pub(crate) struct NewTextStyle {
    inner: HostedPropertiesDialog,
    opening: Opening,
    /// Whether the dialog makes a paragraph style.
    pub(crate) para: bool,
    target: Value,
}

impl NewTextStyle {
    /// The new style's attributes as the dialog's values set them, or why they are refused.
    pub(crate) fn edited(&self) -> Result<Map<String, Value>, String> {
        match &self.opening {
            Opening::Char(opening) => char_style_options::edited_attrs(self.inner.fields(), opening),
            Opening::Para(opening) => para_style_options::edited_attrs(self.inner.fields(), opening),
        }
    }

    /// The parameters of `charStyle.new` or `paraStyle.new` for the text the dialog opened on and `attrs`.
    fn command(&self, attrs: Map<String, Value>) -> Value {
        let mut params = self.target.clone();
        params["name"] = json!(self.inner.fields().text("name").unwrap_or_default());
        params["attrs"] = Value::Object(attrs);
        params
    }
}
style_dialog!(NewTextStyle, KIND);

/// Open New Character Style (`para`: New Paragraph Style) with the name `charStyle.new` would give
/// the style, to make it from the text `target` names.
pub fn open(app: &mut VectorcraftApp, para: bool, target: Value) -> Result<Value, String> {
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let name = vectorcraft_engine::cmd::textstyles::new_style_name(doc, para);
    let mut target = if target.is_object() { target } else { json!({}) };
    let selected_attrs = vectorcraft_engine::cmd::textstyles::target_style_attrs(&app.session, &target, para);
    let attrs = selected_attrs.unwrap_or_else(|| vectorcraft_engine::cmd::textstyles::default_new_style_attrs(&app.session, para));
    target["attrs"] = Value::Object(attrs.clone());
    let (opening, inner) = if para {
        let opening = Arc::new(para_style_options::Opening::new("", attrs, app.session.general_unit(), para_style_options::vertical_text(app)));
        let inner = para_style_options::hosted(KIND, tl!("New Paragraph Style"), &opening, &name);
        (Opening::Para(opening), inner)
    } else {
        let opening = Arc::new(char_style_options::Opening::new("", attrs, &doc.setup, char_style_options::swatch_choices(doc)));
        let inner = char_style_options::hosted(KIND, tl!("New Character Style"), &opening, &name);
        (Opening::Char(opening), inner)
    };
    app.dialogs().ok_or("no window")?.with(|host| host.open(NewTextStyle { inner, opening, para, target }));
    Ok(json!({ "dialog": KIND }))
}

/// OK: make the style with the name. A name another style has, or an empty one, keeps the dialog
/// open with the engine's reason under the rows. The panel highlights the new style and scrolls it
/// into view when it next draws.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some((para, frame)) = dialogs.with(|host| host.get_mut::<NewTextStyle>().map(|d| (d.para, d.frame()))) else {
        // The dialog closed: its answer or cancel has nothing left for the app to do.
        let _ = dialogs.with(|host| host.take::<NewTextStyle>());
        return;
    };
    if !frame.ok {
        return;
    }
    let made = frame.params.and_then(|params| app.run(if para { "paraStyle.new" } else { "charStyle.new" }, params));
    let answer = match &made {
        Ok(v) => {
            if let Some((uid, name)) = app.session.active().and_then(|doc| v["name"].as_str().map(|name| (doc.uid, name.to_string()))) {
                app.ui.text_styles.made.retain(|(pending_uid, pending_para, _)| *pending_uid != uid || *pending_para != para);
                app.ui.text_styles.made.push((uid, para, name));
            }
            Ok(())
        }
        Err(why) => Err(reason(why)),
    };
    dialogs.with(|host| {
        if let Some(d) = host.get_mut::<NewTextStyle>() {
            d.hosted_mut().answer_ok(answer);
        }
    });
}
