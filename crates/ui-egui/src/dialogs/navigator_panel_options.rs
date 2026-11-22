//! Navigator panel › Panel Options…: the color the view box is drawn in, a preset or a custom
//! color, and Draw Dashed Lines as Solid Lines for the thumbnail.
//!
//! Fields: `color` (`#rrggbb`; a preset's name such as "Light Red" is read too) and `solidDashes`. OK runs
//! `ui.navigatorPanelOptions` with them (kept with the UI state).

use std::cell::RefCell;

use craft::color_ui::ui::{ChosenColor, ColorChoice, NamedColor};
use craft::theme::Srgba;
use craft::ui::contract::{Bind, Props as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::requests::ColorRequest;
use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::layer_options::{check, rgb8, srgb};
use crate::VectorcraftApp;
use crate::panels::navigator::{VIEW_BOX_RED, resolve, view_box_colors};

/// The key of the Navigator Panel Options dialog.
const KEY: &str = "ui.navigatorPanelOptions";

/// The options the dialog edits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NavigatorOptionsValue {
    /// The view box's colour: a preset's, or any other the Custom row holds.
    rgb: [u8; 3],
    solid_dashes: bool,
}

impl NavigatorOptionsValue {
    /// The colour as `ui.navigatorPanelOptions` takes it.
    fn color_text(&self) -> String {
        let [r, g, b] = self.rgb;
        Color::rgb8(r, g, b).to_hex()
    }
}

impl Default for NavigatorOptionsValue {
    fn default() -> Self {
        NavigatorOptionsValue { rgb: VIEW_BOX_RED, solid_dashes: false }
    }
}

/// `ui.navigatorPanelOptions` without options: open the dialog on the panel's options.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let o = &app.ui.navigator_panel;
    let value = NavigatorOptionsValue { rgb: resolve(&o.color).unwrap_or(VIEW_BOX_RED), solid_dashes: o.solid_dashes };
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(NavigatorOptionsDialog::new(value)));
    Ok(Value::Null)
}

/// The Navigator Panel Options dialog; its answer is the options OK confirmed, which [`pump`] applies.
pub(crate) struct NavigatorOptionsDialog {
    id: DialogId,
    open: bool,
    value: NavigatorOptionsValue,
    /// The Custom… press, until the app takes it.
    color_request: Option<ColorRequest>,
}

impl NavigatorOptionsDialog {
    /// The dialog over `value`.
    fn new(value: NavigatorOptionsValue) -> Self {
        NavigatorOptionsDialog { id: dialog_id(KEY), open: true, value, color_request: None }
    }
}

impl StackDialog for NavigatorOptionsDialog {
    type Result = NavigatorOptionsValue;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<NavigatorOptionsValue>> {
        let requested = RefCell::new(None);
        let shown =
            FormDialog::new(tl!("Navigator Panel Options"), Bind::bound(&mut self.value)).opened(Bind::bound(&mut self.open)).show(ui, |ui, _, v| {
                Field::new(tl!("Color:")).id_salt("color").show(ui, |ui, _| {
                    let presets: Vec<(&str, [u8; 3])> = view_box_colors().collect();
                    let preset = presets.iter().find(|(_, c)| *c == v.rgb);
                    let chosen = preset.map_or(ChosenColor::Custom, |(name, _)| ChosenColor::Named((*name).to_owned()));
                    let named = presets.iter().map(|&(name, c)| NamedColor::new(name, name.to_string(), Some(srgb(c))));
                    let custom = preset.is_none().then(|| srgb(v.rgb));
                    let out = ColorChoice::new(tl!("Color:"), &chosen).named(named).custom(custom).show(ui);
                    if let Some(ChosenColor::Named(picked)) = out.chosen
                        && let Some((_, c)) = presets.iter().find(|(name, _)| *name == picked)
                    {
                        v.rgb = *c;
                    }
                    // A second pass of the frame has no press: only a request keeps what the first pass asked for.
                    if let Some(request) = out.pick_color {
                        *requested.borrow_mut() = Some(request);
                    }
                });
                if let Some(on) = check(ui, tl!("Draw Dashed Lines as Solid Lines"), v.solid_dashes, true) {
                    v.solid_dashes = on;
                }
                true
            });
        if let Some(request) = requested.into_inner() {
            self.color_request = Some(request);
        }
        match (shown.accepted, shown.closed) {
            (Some(value), _) => Some(DialogResult::Ok(value)),
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<NavigatorOptionsValue> {
        Some(self.value.clone())
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "color": self.value.color_text(), "solidDashes": self.value.solid_dashes }))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "color" => self.value.rgb = value.as_str().and_then(resolve).ok_or_else(rejects)?,
            "solidDashes" => self.value.solid_dashes = value.as_bool().ok_or_else(rejects)?,
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }

    fn take_color_request(&mut self) -> Option<ColorRequest> {
        self.color_request.take()
    }

    fn answer_color(&mut self, key: &str, color: Srgba) {
        if key == ColorChoice::CUSTOM_KEY {
            self.value.rgb = rgb8(color);
        }
    }
}

/// Applies the options OK confirmed; an error from the command goes to the status line.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(DialogResult::Ok(value)) = dialogs.with(|host| host.take::<NavigatorOptionsDialog>()) else { return };
    let params = json!({ "color": value.color_text(), "solidDashes": value.solid_dashes });
    if let Err(e) = app.run("ui.navigatorPanelOptions", params) {
        app.status(e);
    }
    ctx.request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()))
    }

    /// The open dialog's id and fields.
    fn open_dialog(app: &VectorcraftApp) -> (DialogId, Value) {
        let open = app.dialogs().unwrap().with(|host| host.open_dialogs().into_iter().find(|d| d.key.as_deref() == Some(KEY))).unwrap();
        (open.id, open.fields.unwrap())
    }

    /// Sets the dialog's `color` and `solidDashes`, presses OK and runs the host pump.
    fn apply(app: &mut VectorcraftApp, color: &str, solid: bool) {
        let ctx = app.ui_ctx.clone().unwrap();
        let (id, _) = open_dialog(app);
        let dialogs = app.dialogs().unwrap();
        dialogs.with(|host| host.set_json_field(&ctx, id, "color", json!(color))).unwrap();
        dialogs.with(|host| host.set_json_field(&ctx, id, "solidDashes", json!(solid))).unwrap();
        dialogs.with(|host| host.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(app, &ctx);
    }

    /// The dialog opens on the panel's options (Light Red and solid dashes off by default), and OK
    /// sets a preset's or a custom color and Draw Dashed Lines as Solid Lines, kept with the UI
    /// state; a value that is not a color is refused and leaves the options as they were.
    #[test]
    fn panel_options_set_the_view_box_color_and_solid_dashes() {
        let mut app = app();
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        let f = open_dialog(&app).1;
        assert_eq!((f["color"].as_str(), f["solidDashes"].as_bool()), (Some("#ff3030"), Some(false)));
        apply(&mut app, "Green", true);
        assert!(!app.any_dialog_open());
        assert_eq!((app.ui.navigator_panel.color.as_str(), app.ui.navigator_panel.solid_dashes), ("#4fff4f", true));
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        apply(&mut app, "#0000FF", true);
        assert_eq!(app.ui.navigator_panel.color, "#0000ff");
        app.run("ui.navigatorPanelOptions", json!({})).unwrap();
        let (ctx, (id, _)) = (app.ui_ctx.clone().unwrap(), open_dialog(&app));
        let refused = app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, "color", json!("nope")));
        assert!(refused.is_err());
        assert_eq!(open_dialog(&app).1["color"], json!("#0000ff"), "a refused value changes nothing");
        let back: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
        assert_eq!(back.navigator_panel, app.ui.navigator_panel, "kept with the UI state");
        let old: crate::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(old.navigator_panel, crate::panels::navigator::PanelOptions::default(), "UI state saved without them");
    }
}
