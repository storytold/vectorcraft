//! Save As, with the user's cloud account offered beside this computer.
//!
//! Save As on the desktop is the operating system's own file panel, which cannot hold a page of
//! somebody's cloud files. So when a cloud service is wired, Save As asks where first. This
//! computer leads to that same panel. The account leads to the shared Save page
//! ([`craft_ui::cloud::save_page`]), which picks a folder and names the file.
//!
//! Nothing here is offered in a build with no service configured. File › Save As then behaves as
//! it always has, and this dialog never opens.

use egui::{Align2, Rect, Ui, pos2, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::fileio::SaveMode;

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

pub const KIND: &str = "cloudSave";

/// Which destination was chosen. Absent until one is.
const TO: &str = "to";
const COMPUTER: &str = "computer";
const CLOUD: &str = "cloud";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Save As…").into(),
    body,
    // The Save page carries its own Save button, and the file panel this computer opens carries
    // another. A third one beside them would be a second control for the same thing.
    ok: None,
    min_width: 620.0,
    max_width: Some(760.0),
    ..DialogSpec::FORM
};

/// Open the dialog. `to_cloud` skips the question, for a save that already knows where it is
/// going (Save as a copy, offered after a refused save).
pub fn open(app: &mut VectorcraftApp, to_cloud: bool) {
    let fields = if to_cloud { json!({TO: CLOUD}) } else { json!({}) };
    app.ui.dialog = Some(Dialog::new(KIND, fields));
    crate::cloud::start(app);
}

/// File › Save As with a cloud service wired: ask where first.
///
/// Without a service this is the ordinary Save As and nothing is asked. A path or a format in `p`
/// is an answer already, so an agent or a script never meets the question either.
pub fn save_as(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let answered = p.get("path").is_some() || p.get("format").is_some();
    if answered || !crate::cloud::available(app) {
        return crate::io::save(app, SaveMode::SaveAs, p, true);
    }
    if app.session.active().is_none() {
        return Err("no drawing open".into());
    }
    open(app, crate::cloud::prefers_cloud(app));
    Ok(json!({"pending": KIND}))
}

/// This computer, chosen inside the dialog and acted on a frame later.
///
/// The body cannot run the save itself: [`super::show`] writes the dialog back after the body
/// returns, so a file panel opened from inside it would be lost. Called once a frame, after the
/// dialogs are drawn.
pub(crate) fn settle(app: &mut VectorcraftApp) {
    let local = app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.str(TO) == COMPUTER);
    if !local {
        return;
    }
    app.ui.dialog = None;
    if let Err(e) = crate::io::save(app, SaveMode::SaveAs, &json!({}), true) {
        app.status(e);
    }
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    if !crate::cloud::available(app) {
        widgets::dim_label(ui, &crate::i18n::msg(crate::cloud::UNAVAILABLE));
        return false;
    }
    // Asked once a frame while the dialog is open, so the badge and the folder fill themselves in
    // without a gesture. A question already answered is not asked again.
    crate::cloud::start(app);
    let to = d.str(TO);
    if to.is_empty() {
        widgets::section_header(ui, tl!("Where"));
        ui.add_space(6.0);
        if place_row(ui, "save", tl!("This computer"), tl!("A folder on this computer")).clicked() {
            // Recorded, not run: see `settle`.
            d.fields.insert(TO.into(), json!(COMPUTER));
        }
        // Named as the account's own, so the destination reads as the user's storage rather than
        // as some generic place.
        let account = crate::cloud::destination_name(app);
        if place_row(ui, "cloud", &account, tl!("Your cloud account")).clicked() {
            d.fields.insert(TO.into(), json!(CLOUD));
        }
        return false;
    }
    if to == CLOUD {
        ui.add_space(4.0);
        // True once a save has started: the drawing is on its way, so the dialog is done.
        return crate::cloud::save_page(app, ui);
    }
    false
}

/// One destination: an icon, its name, and a line saying what it is.
fn place_row(ui: &mut Ui, icon: &str, name: &str, detail: &str) -> egui::Response {
    let t = Tokens::get(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), egui::Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, name));
    if resp.hovered() {
        ui.painter().rect_filled(rect, t.cr(), t.hover);
    }
    let painter = ui.painter();
    crate::icons::paint_with(painter, icon, Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(20.0, 20.0)), t.icon);
    painter.text(pos2(rect.left() + 48.0, rect.center().y - 8.0), Align2::LEFT_CENTER, name, theme::semibold(13.0), t.text);
    painter.text(pos2(rect.left() + 48.0, rect.center().y + 9.0), Align2::LEFT_CENTER, detail, egui::FontId::proportional(11.5), t.text_dim);
    resp
}
