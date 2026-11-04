//! Selection Tools in the interface: its submenu (Object › Plug-ins › Selection Tools) and the
//! Select by Artboards dialog (#5548), which lists the artboards with their number, name and size,
//! filters them by name, picks several, fits one in the window on double-click (#5549), and on OK
//! runs `addon.select.byArtboards` with the relation chosen.

use egui::{Sense, Ui, vec2};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::{DialogSpec, form};
use crate::menus::Item;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::widgets;

/// Select by Artboards, and its dialog's kind.
pub const COMMAND: &str = "addon.select.byArtboards";
pub const KIND: &str = "selectByArtboards";
/// The relations the dialog offers: (value, label).
const RELATIONS: [(&str, &str); 3] = [("overlap", "Overlapping"), ("inside", "Fully Inside"), ("outside", "Fully Outside")];
const LABEL_W: f32 = 70.0;
const ROW_H: f32 = 22.0;
const NUMBER_W: f32 = 28.0;
const LIST_H: f32 = 220.0;

/// Object › Plug-ins › Selection Tools.
pub fn menu() -> Vec<Item> {
    vec![Item::Cmd("Select by Artboards…", COMMAND, Value::Null)]
}

/// The dialog's fields to start with: the active artboard chosen, Overlapping.
pub(crate) fn fields(app: &VectorcraftApp) -> Value {
    json!({ "artboards": [app.view().map_or(0, |v| v.artboard)], "relation": "overlap" })
}

pub(crate) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Select by Artboards").to_string(),
    body,
    confirm,
    min_width: 360.0,
    ok_enabled: Some(|_, d| !chosen(d).is_empty()),
    ..DialogSpec::FORM
};

/// The artboards chosen in dialog `d`, in order.
fn chosen(d: &Dialog) -> Vec<usize> {
    let mut v: Vec<usize> =
        d.fields.get("artboards").and_then(Value::as_array).into_iter().flatten().filter_map(|i| usize::try_from(i.as_u64()?).ok()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// The indices of `names` whose name holds `query`, ignoring case (all of them for an empty query).
pub(crate) fn shown(names: &[String], query: &str) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    names.iter().enumerate().filter(|(_, n)| q.is_empty() || n.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

/// A click on row `i` of the rows `shown`: alone it chooses only that artboard of those shown, with Cmd/Ctrl it
/// toggles it, with Shift it adds the rows from the last one clicked (`anchor`) to it. Returns the
/// new anchor.
pub(crate) fn click(chosen: &mut Vec<usize>, anchor: Option<usize>, i: usize, shown: &[usize], shift: bool, command: bool) -> Option<usize> {
    let at = |j: usize| shown.iter().position(|k| *k == j);
    match (shift, anchor.and_then(at), at(i)) {
        (true, Some(a), Some(b)) => {
            chosen.extend(shown.iter().skip(a.min(b)).take(a.abs_diff(b) + 1));
            chosen.sort_unstable();
            chosen.dedup();
            return anchor;
        }
        _ if command => match chosen.iter().position(|j| *j == i) {
            Some(p) => {
                chosen.remove(p);
            }
            None => chosen.push(i),
        },
        // Of the rows shown, only this one; rows the search hides keep their state.
        _ => {
            chosen.retain(|j| !shown.contains(j));
            chosen.push(i);
        }
    }
    chosen.sort_unstable();
    Some(i)
}

fn body(app: &mut VectorcraftApp, ui: &mut Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    let unit = app.session.general_unit();
    let boards: Vec<(String, f64, f64)> = app
        .session
        .active()
        .map(|st| st.doc.artboards.iter().map(|a| (a.name.clone(), a.rect.width(), a.rect.height())).collect())
        .unwrap_or_default();
    let names: Vec<String> = boards.iter().map(|b| b.0.clone()).collect();
    let query = widgets::search_field(ui, ui.scope_id().with("select-by-artboards-search"), "Search artboards");
    d.fields.insert("__search".into(), json!(query));
    let rows = shown(&names, &query);
    let picked = chosen(d);
    let (mut clicked, mut double) = (None, None);
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(LIST_H).auto_shrink([false, true]).show(ui, |ui| {
        if rows.is_empty() {
            ui.label(egui::RichText::new(tl!("No artboards match.")).color(t.text_dim));
        }
        for &i in &rows {
            let Some((name, w, h)) = boards.get(i) else { continue };
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
            let on = picked.contains(&i);
            if on {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover);
            }
            let font = egui::FontId::proportional(12.0);
            let mid = r.left_center();
            let p = ui.painter();
            p.text(mid + vec2(6.0, 0.0), egui::Align2::LEFT_CENTER, (i + 1).to_string(), font.clone(), t.text_dim);
            let size = format!("{} × {}", unit.number(*w), unit.format(*h));
            let size_at = r.right_center() - vec2(6.0, 0.0);
            p.text(size_at, egui::Align2::RIGHT_CENTER, &size, font.clone(), t.text_dim);
            p.with_clip_rect(r.with_max_x(size_at.x - 80.0)).text(mid + vec2(NUMBER_W, 0.0), egui::Align2::LEFT_CENTER, name, font, t.text);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::ListItem, true, on, format!("{} {name} {size}", i + 1)));
            if resp.double_clicked() {
                double = Some(i);
            } else if resp.clicked() {
                clicked = Some((i, ui.input(|inp| inp.modifiers)));
            }
        }
    });
    if let Some((i, m)) = clicked {
        row_clicked(d, i, &rows, m);
    }
    if let Some(i) = double {
        row_double_clicked(app, d, i);
    }
    let mut picked = chosen(d);
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if widgets::flat_button_fit(ui, "Select All", 70.0).clicked() {
            choose_shown(&mut picked, &rows, true);
        }
        if widgets::flat_button_fit(ui, "None", 50.0).clicked() {
            choose_shown(&mut picked, &rows, false);
        }
        // Rows hidden by the search stay chosen: the count says how many are.
        let count = crate::i18n::fmt(tl!("Chosen: {n} of {total}"), &[("n", &picked.len().to_string()), ("total", &boards.len().to_string())]);
        ui.label(egui::RichText::new(count).color(t.text_dim));
    });
    d.fields.insert("artboards".into(), json!(picked));
    ui.add_space(6.0);
    form::choice(ui, d, "relation", tl!("Select:"), (LABEL_W, 160.0), &RELATIONS);
    false
}

/// Select All (`on`) or None: choose every row `shown`, or none of them. Rows the search hides
/// keep their state.
pub(crate) fn choose_shown(chosen: &mut Vec<usize>, shown: &[usize], on: bool) {
    if on {
        chosen.extend(shown.iter().copied());
        chosen.sort_unstable();
        chosen.dedup();
    } else {
        chosen.retain(|i| !shown.contains(i));
    }
}

/// A click on row `i` of the rows `shown`, with modifier keys `m`: [`click`] on dialog `d`'s
/// chosen artboards.
pub(crate) fn row_clicked(d: &mut Dialog, i: usize, shown: &[usize], m: egui::Modifiers) {
    let mut picked = chosen(d);
    // What a double-click's first click changed, for its second to put back.
    d.fields.insert("__before".into(), json!(picked));
    let anchor = d.fields.get("__anchor").and_then(Value::as_u64).and_then(|a| usize::try_from(a).ok());
    if let Some(a) = click(&mut picked, anchor, i, shown, m.shift, m.command) {
        d.fields.insert("__anchor".into(), json!(a));
    }
    d.fields.insert("artboards".into(), json!(picked));
}

/// A double-click on row `i`: artboard `i` becomes active and fits in the window, as a
/// double-click on its number in the Artboards panel does (`view.goToArtboard`). The dialog stays
/// open, and the artboards chosen are those chosen before the double-click.
pub(crate) fn row_double_clicked(app: &mut VectorcraftApp, d: &mut Dialog, i: usize) {
    if let Some(before) = d.fields.remove("__before") {
        d.fields.insert("artboards".into(), before);
    }
    // An artboard the list showed a frame ago can't be missing, and a failure only leaves the view.
    app.run("view.goToArtboard", json!({ "index": i })).ok();
}

/// OK: select the objects and close.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    app.run(COMMAND, form::params(d))
}

#[cfg(test)]
mod tests;
