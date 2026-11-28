//! The Properties panel's Appearance section (Fill, Stroke, Variable Width Profile, Opacity and
//! the effect controls) and Global Edit, whose options the Control bar shows too.

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Gap, LabelScheme, Props as _, Show as _, ShowContainer as _, Sizing, Trigger, clicked};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::menu::Menu;
use craft::ui::popover::Popover;
use craft::ui::style::TypeRole;
use craft::ui::text::{NumberField, TextField};
use egui::{Response, Ui};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, WidthProfile};
use vectorcraft_engine::cmd::globaledit;

use crate::VectorcraftApp;
use crate::edits::{EditPath, apply_edits};
use crate::panels::{self, appearance, first_selected, stroke};

/// The Appearance section for the first selected object: its Fill and Stroke, the stroke's width
/// profile, Opacity and the effect controls.
pub(super) fn appearance_section(app: &mut VectorcraftApp, ui: &mut Ui) {
    let Some(n) = first_selected(app) else { return };
    super::section(tl!("Appearance")).show(ui, |ui, _| {
        fill_stroke_rows(app, ui);
        width_profile_row(app, ui);
        opacity_row(app, ui, &n);
        effects_row(app, ui, &n);
    });
}

/// The command button that shows `panel` in the dock, for the More Options icons.
fn panel_button<'a>(icon: &'static str, panel: &str, label: &'static str) -> IconButton<'a> {
    IconButton::command(IconId::new(icon), CommandRef::with_params("window.panel.activate", json!({ "panel": panel }))).label(label)
}

/// The Opacity field, and the icon that shows the Transparency panel.
fn opacity_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    Row::new().labels(LabelScheme::Inspector).show(ui, |ui, _| {
        let shown = Some(n.opacity as f64 * 100.0);
        let out = Field::new(tl!("Opacity"))
            .sizing(Sizing::Content)
            .show(ui, |ui, _| NumberField::new(tl!("Opacity"), Bind::given(shown)).range(0.0..=100.0).unit("%").fixed_decimals(0).show(ui))
            .inner;
        if let Some(out) = out {
            apply_edits(app, &out.edit, EditPath::Scope, |app, value, _| {
                if let Some(opacity) = value {
                    app.run("object.setProps", json!({ "opacity": opacity })).ok();
                }
            });
        }
        panel_button("ellipsis", "transparency", tl!("Transparency")).show(ui);
    });
}

/// The fx button that opens the effect menu, as the Appearance panel's does, the Delete icon and
/// the icon that shows the Appearance panel.
fn effects_row(app: &mut VectorcraftApp, ui: &mut Ui, n: &Node) {
    let editable = app.session.active().is_some_and(|st| st.selection.symbol_part.is_none());
    // The effects of one selected object: the fill's and stroke's as well as its own.
    let effects: Vec<(Option<usize>, usize, &str)> = if panels::selection_len(app) == 1 {
        let own = n.appearance.effects.iter().enumerate().map(|(k, e)| (None, k, e.id.as_str()));
        let items =
            n.appearance.items.iter().enumerate().flat_map(|(i, it)| it.effects().iter().enumerate().map(move |(k, e)| (Some(i), k, e.id.as_str())));
        own.chain(items).collect()
    } else {
        vec![]
    };
    Row::new().show(ui, |ui, _| {
        let fx = Button::new("fx").tooltip(tl!("Add New Effect")).enabled(editable).show(ui);
        if editable {
            Menu::new(tl!("Add New Effect")).opened_by(&fx.response, Trigger::Click).show(ui, |ui, _| appearance::fx_menu(app, ui));
        }
        effect_delete(app, ui, n.id, &effects, editable);
        panel_button("ellipsis", "appearance", tl!("Appearance panel")).show(ui);
    });
}

/// The Delete icon beside the fx button, as the Properties panel overview has it
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>):
/// with one effect applied, its name and an icon that removes it (`effect.remove`); with several,
/// the icon shows the Appearance panel to remove them there. Nothing without effects.
fn effect_delete(app: &mut VectorcraftApp, ui: &mut Ui, id: NodeId, effects: &[(Option<usize>, usize, &str)], editable: bool) {
    match effects {
        [] => {}
        [(item, k, effect)] => {
            Text::new(appearance::shown_effect_label(effect)).elide(Elide::End).show(ui);
            if IconButton::new(IconId::new("trash-2")).label(tl!("Remove Effect")).enabled(editable).show(ui).clicked() {
                app.run("effect.remove", json!({"ids": [id.0], "item": item, "index": k})).ok();
            }
        }
        _ => {
            panel_button("trash-2", "appearance", tl!("Remove Effect")).show(ui);
        }
    }
}

/// Whether a stroke that `stroke.set` edits has a width profile
/// ([`vectorcraft_engine::Session::edited_strokes_profiled`]), cached per document revision and
/// active Appearance row in `PropertiesUi::edited_profiled`. False while the Type tool edits text or
/// a tool paints the stroke ([`vectorcraft_engine::Session::tool_paints`]), when the list edits the stroke the panel shows.
fn edited_strokes_profiled(app: &mut VectorcraftApp) -> bool {
    if panels::character::text_editing(app).is_some() || app.session.tool_paints(true) {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let key = (st.uid, st.revision, app.session.appearance_item());
    if let Some((k, v)) = app.ui.properties_panel.edited_profiled
        && k == key
    {
        return v;
    }
    let v = app.session.edited_strokes_profiled();
    app.ui.properties_panel.edited_profiled = Some((key, v));
    v
}

/// The stroke's Variable Width Profile (the Control bar's list), shown while the Width tool is in
/// use or the stroke has a variable width, as the Properties panel overview places it
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/properties-panel-overview.html>).
/// The row also shows when the shown stroke is uniform and another stroke the list edits has a
/// variable width ([`edited_strokes_profiled`]), so that Uniform can remove it.
fn width_profile_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    let stroke = panels::current_stroke(app);
    let profile = stroke.as_ref().and_then(|s| s.profile.as_ref());
    if app.session.tool_id() != "width" && WidthProfile::id_of(profile) == "uniform" && !edited_strokes_profiled(app) {
        return;
    }
    Field::new(tl!("Profile")).show(ui, |ui, _| stroke::profile_select(app, ui));
}

/// The Fill and Stroke rows, with the Control bar's widgets: the chips (a click opens the
/// swatches, Shift-click the mixer), the Stroke link that opens the Stroke panel as a popover
/// and the weight field with its presets. With nothing selected they set up the next object.
pub(super) fn fill_stroke_rows(app: &mut VectorcraftApp, ui: &mut Ui) {
    let weight = stroke::shown_weight(app, panels::current_stroke(app).as_ref(), &panels::stroke_mixed(app, ui.ctx()));
    Row::new().show(ui, |ui, _| {
        panels::paint_chip(app, ui, false);
        Text::new(tl!("Fill")).show(ui);
    });
    Row::new().labels(LabelScheme::Inspector).show(ui, |ui, _| {
        panels::paint_chip(app, ui, true);
        stroke::link_field(app, ui, tl!("Stroke"), weight);
    });
}

/// Start Global Edit (Stop Global Edit while it is on) with the Global Edit Options beside it, in
/// Quick Actions as the help page places them
/// (<https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>).
pub(super) fn global_edit_row(app: &mut VectorcraftApp, ui: &mut Ui) {
    if !crate::menus::enabled(app, "select.globalEdit") {
        return;
    }
    let on = app.session.active().is_some_and(|d| d.global_edit.is_some());
    // A copy, so the popover's flag does not borrow `app` while its body runs commands.
    let mut open = app.ui.properties_panel.global_edit_open;
    Row::new().show(ui, |ui, _| {
        // The command's own state gives its title: Start or Stop Global Edit. It takes the row up to the options.
        Button::command("select.globalEdit").sizing(Sizing::FillWidth).show(ui);
        let options = IconButton::new(IconId::new("chevron-down")).label(tl!("Global Edit Options")).enabled(!on).show(ui);
        global_edit_popover(app, ui, &options.response, &mut open);
    });
    app.ui.properties_panel.global_edit_open = open;
}

/// The Global Edit Options popover hanging from `opener`, a click on which toggles `open`. It is
/// closed while Global Edit is on, including when Global Edit starts with it open. The Control bar
/// shows the same popover with its own flag.
pub(crate) fn global_edit_popover(app: &mut VectorcraftApp, ui: &mut Ui, opener: &Response, open: &mut bool) {
    if app.session.active().is_some_and(|d| d.global_edit.is_some()) {
        *open = false;
    } else if clicked(opener) {
        *open = !*open;
    }
    Popover::new(tl!("Global Edit Options"), Bind::bound(open)).opened_by(opener).show(ui, |ui, _| global_edit_options(app, ui));
}

/// A checkbox of the Global Edit Options: the value a click gave it.
fn option_check(ui: &mut Ui, label: &'static str, on: bool) -> Option<bool> {
    Checkbox::new(Bind::given(on)).label(label).show(ui).events.committed().copied()
}

/// The Global Edit Options: what else similar objects must share (Match), where they may lie (On
/// Artboards) and how many objects that takes now. The Control bar's Global Edit Options dropdown
/// shows them too.
pub(crate) fn global_edit_options(app: &mut VectorcraftApp, ui: &mut Ui) {
    let o = app.session.global_edit_options.clone();
    let set = |app: &mut VectorcraftApp, p: Value| crate::menus::invoke(app, "select.globalEditOptions", p);
    let caption = |ui: &mut Ui, text: &'static str| {
        Text::new(text).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
    };
    let results = global_edit_results(global_edit_count(app));
    // Each group is its caption over its options a row gap apart; the groups and the Results line are a stack gap
    // apart. A click is kept and run after the stack returns.
    let mut changes = Vec::new();
    Stack::new().gap(Gap::Stack).show(ui, |ui, _| {
        FieldGroup::new(tl!("Match")).show(ui, |ui, _| {
            caption(ui, tl!("Match"));
            if let Some(on) = option_check(ui, tl!("Appearance"), o.appearance) {
                changes.push(json!({ "appearance": on }));
            }
            if let Some(on) = option_check(ui, tl!("Size"), o.size) {
                changes.push(json!({ "size": on }));
            }
        });
        FieldGroup::new(tl!("On Artboards")).show(ui, |ui, _| {
            caption(ui, tl!("On Artboards"));
            if let Some(all) = option_check(ui, tl!("All Artboards"), o.artboards.is_none()) {
                changes.push(json!({ "artboards": if all { "all" } else { "1" } }));
            }
            if let Some(range) = &o.artboards {
                let typed = Field::new(tl!("Range:")).show(ui, |ui, _| TextField::new(tl!("Range:"), Bind::given(range.clone())).show(ui)).inner;
                if let Some(range) = typed.and_then(|t| t.events.committed().cloned()) {
                    changes.push(json!({ "artboards": range }));
                }
            }
            if let Some(on) = option_check(ui, tl!("Include Objects On Canvas"), o.include_canvas) {
                changes.push(json!({ "includeCanvas": on }));
            }
        });
        Text::new(results.clone()).shade(Shade::Muted).show(ui);
    });
    for p in changes {
        set(app, p);
    }
}

/// The options' line with how many objects Start Global Edit would take, in the interface language.
pub(crate) fn global_edit_results(n: usize) -> String {
    crate::i18n::tn(n as u64, "Result: {n}", "Results: {n}")
}

/// What the Results count was found for: the document, its revision, the options and the one
/// selected object.
pub(crate) type GlobalEditKey = (u64, u64, globaledit::Options, Option<u64>);

/// How many objects Start Global Edit would take now (the options' Results line), kept in
/// `app.canvas.global_edit_count` per [`GlobalEditKey`], as finding them walks the whole document.
pub(crate) fn global_edit_count(app: &mut VectorcraftApp) -> usize {
    let Some(st) = app.session.active() else { return 0 };
    let selected = match st.selection.objects.as_slice() {
        [id] => Some(id.0),
        _ => None,
    };
    let key: GlobalEditKey = (st.uid, st.revision, app.session.global_edit_options.clone(), selected);
    if let Some((k, count)) = &app.canvas.global_edit_count
        && *k == key
    {
        return *count;
    }
    let count = globaledit::match_count(&app.session);
    app.canvas.global_edit_count = Some((key, count));
    count
}
