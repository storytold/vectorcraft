//! The Actions panel: record commands from the engine journal, play them back as one undo step.
//! Every recorded step is an ordinary command invocation, so actions are also plain JSON that
//! agents can author and replay (`command.batch`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{pstate, set_pstate};
use crate::theme::Tokens;
use crate::widgets::{self, dim_label};
use crate::{VectorcraftApp, icons};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Action {
    pub name: String,
    pub steps: Vec<(String, Value)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActionSet {
    /// Tells the set apart from every other set in this run, even one with the same name. It is
    /// not saved: each set gets a new id when it is made or loaded.
    #[serde(skip, default = "next_set_id")]
    pub id: u64,
    pub name: String,
    pub actions: Vec<Action>,
}

impl ActionSet {
    /// A set with a new [`ActionSet::id`].
    pub fn new(name: impl Into<String>, actions: Vec<Action>) -> Self {
        Self { id: next_set_id(), name: name.into(), actions }
    }
}

/// A set id no other set in this run has.
fn next_set_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Built-in actions (our own, handy defaults).
pub fn default_sets() -> Vec<ActionSet> {
    let step = |c: &str, p: Value| (c.to_string(), p);
    vec![ActionSet::new(
        "Default Actions",
        vec![
            Action { name: "Duplicate and Offset 10 pt".into(), steps: vec![step("object.move", json!({"dx": 10, "dy": 10, "copy": true}))] },
            Action {
                name: "Unite and Simplify".into(),
                steps: vec![step("object.pathfinder.unite", json!({})), step("object.path.simplify", json!({"tolerance": 0.5}))],
            },
            Action {
                name: "Center on Artboard".into(),
                steps: vec![step("object.align", json!({"horizontal": "center", "vertical": "center", "to": "artboard"}))],
            },
            Action { name: "Outline Text".into(), steps: vec![step("type.createOutlines", json!({}))] },
            Action { name: "Rotate 90° CW".into(), steps: vec![step("object.rotate", json!({"angle": -90}))] },
            Action { name: "Reflect Horizontal".into(), steps: vec![step("object.reflect", json!({"axis": "vertical"}))] },
        ],
    )]
}

/// Run an action as a single undoable batch.
pub fn play(app: &mut VectorcraftApp, set: usize, idx: usize) -> Result<Value, String> {
    let Some(a) = app.ui.action_sets.get(set).and_then(|s| s.actions.get(idx)).cloned() else { return Err("no such action".into()) };
    let commands: Vec<Value> = a.steps.iter().map(|(c, p)| json!({"command": c, "params": p})).collect();
    app.run("command.batch", json!({"label": a.name, "commands": commands}))
}

/// The command that deletes action sets ([`delete_sets`]).
pub const DELETE_SETS: &str = "ui.actionSet.delete";

/// The sets Delete Selection deletes: the selected sets ([`crate::state::UiState::action_sets_selected`])
/// when the current set is one of them, else the current set alone.
fn selected_sets(app: &VectorcraftApp) -> Vec<usize> {
    let len = app.ui.action_sets.len();
    let current = app.ui.action_set.filter(|&s| s < len);
    let sets: Vec<usize> = app.ui.action_sets_selected.iter().copied().filter(|&s| s < len).collect();
    if current.is_some_and(|c| sets.contains(&c)) { sets } else { current.into_iter().collect() }
}

/// Delete Selection, from the bottom bar and the panel menu: deletes the selected action, else asks
/// whether to delete the selected sets ([`DELETE_SETS`]).
fn delete_selection(app: &mut VectorcraftApp, ctx: &egui::Context) {
    const SELECTED: &str = "actions-selected";
    // A selected action counts only while its set is the current one, as the panel shows it.
    if let Some((si, ai)) = pstate::<Option<(usize, usize)>>(ctx, SELECTED).filter(|&(s, _)| app.ui.action_set == Some(s)) {
        if let Some(s) = app.ui.action_sets.get_mut(si)
            && ai < s.actions.len()
        {
            s.actions.remove(ai);
            set_pstate::<Option<(usize, usize)>>(ctx, SELECTED, None);
        }
        return;
    }
    let sets = selected_sets(app);
    let message = match sets[..] {
        [] => return,
        [s] => crate::i18n::fmt(
            tl!("Delete the action set “{name}” and its actions?"),
            &[("name", app.ui.action_sets.get(s).map_or("", |s| s.name.as_str()))],
        ),
        _ => tl!("Delete the selected action sets and their actions?").to_string(),
    };
    let ids: Vec<u64> = sets.iter().filter_map(|&s| app.ui.action_sets.get(s)).map(|s| s.id).collect();
    let params = json!({ "sets": sets, "ids": ids });
    crate::dialogs::confirm::ask(app, &message, "", DELETE_SETS, params);
}

/// `ui.actionSet.delete {sets?: [index], ids?: [id]}`: deletes the action sets at these
/// indexes, with their actions (default: the current set). `ids` holds the panel's own set ids
/// ([`ActionSet::id`]), which Delete Selection's question passes: the sets with these ids are
/// deleted wherever they are now, `sets` is not read, and when one of them no longer exists
/// nothing is deleted. A recording into a deleted set stops, and its steps are dropped; a
/// recording into another set goes on. An open Set Options or Batch dialog for a deleted set
/// closes. When the current set is deleted, no set is current afterwards.
pub fn delete_sets(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let len = app.ui.action_sets.len();
    let mut sets: Vec<usize> = match (p.get("ids"), p.get("sets")) {
        (Some(v), _) if !v.is_null() => v
            .as_array()
            .ok_or("the action set ids are not a list")?
            .iter()
            .map(|id| {
                id.as_u64().and_then(|id| app.ui.action_sets.iter().position(|s| s.id == id)).ok_or("the action sets changed; nothing was deleted")
            })
            .collect::<Result<_, _>>()?,
        (_, None | Some(Value::Null)) => app.ui.action_set.filter(|&s| s < len).into_iter().collect(),
        (_, Some(v)) => v
            .as_array()
            .ok_or("no such action set")?
            .iter()
            .map(|i| i.as_u64().and_then(|i| usize::try_from(i).ok()).filter(|&i| i < len).ok_or("no such action set"))
            .collect::<Result<_, _>>()?,
    };
    sets.sort_unstable();
    sets.dedup();
    if let Some((rec, ..)) = app.ui.recording.as_mut() {
        if sets.contains(rec) {
            app.ui.recording = None;
            app.session.journal.hold(None);
        } else {
            *rec -= sets.iter().filter(|&&s| s < *rec).count();
        }
    }
    // An open Set Options or Batch dialog holds a set index: it closes when its set is deleted,
    // and otherwise follows its set down.
    let holds_set = |d: &crate::state::Dialog| {
        d.kind == crate::dialogs::action_batch::KIND || (d.kind == crate::dialogs::action_set_options::KIND && d.fields.contains_key("set"))
    };
    if let Some(d) = app.ui.dialog.as_mut().filter(|d| holds_set(d))
        && let Some(set) = d.fields.get("set").and_then(Value::as_u64).and_then(|s| usize::try_from(s).ok())
    {
        if sets.contains(&set) {
            app.ui.dialog = None;
        } else {
            d.fields.insert("set".into(), json!(set - sets.iter().filter(|&&s| s < set).count()));
        }
    }
    for &s in sets.iter().rev() {
        app.ui.action_sets.remove(s);
    }
    if !sets.is_empty() {
        app.ui.action_set = app.ui.action_set.filter(|c| !sets.contains(c)).map(|c| c - sets.iter().filter(|&&s| s < c).count());
        app.ui.action_sets_selected.clear();
    }
    Ok(json!({ "deleted": sets.len() }))
}

/// The panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let selected: Option<(usize, usize)> = pstate(ui.ctx(), "actions-selected");
    let has = selected.and_then(|(s, a)| app.ui.action_sets.get(s).and_then(|set| set.actions.get(a))).is_some();
    if widgets::menu_item(ui, tl!("New Set…"), true, false) {
        crate::dialogs::action_set_options::open_new(app);
    }
    if widgets::menu_item(ui, tl!("Delete"), has || !selected_sets(app).is_empty(), false) {
        delete_selection(app, ui.ctx());
    }
    let current = app.ui.action_set.filter(|&s| s < app.ui.action_sets.len());
    if widgets::menu_item(ui, tl!("Set Options…"), current.is_some(), false)
        && let Some(s) = current
    {
        crate::dialogs::action_set_options::open(app, s);
    }
    if widgets::menu_item(ui, tl!("Batch…"), has, false)
        && let Some((s, a)) = selected
        && let Err(e) = crate::dialogs::action_batch::open(app, s, a)
    {
        app.status(e);
    }
    if widgets::menu_item(ui, tl!("Play Current Selection"), has, false)
        && let Some((s, a)) = selected
        && let Err(e) = play(app, s, a)
    {
        app.status(e);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    // The selected action as (set, action). Read as the type it is stored as: egui keeps a
    // value per type, so reading the bare pair never found it and no action stayed selected (#1629).
    const SELECTED: &str = "actions-selected";
    let mut selected: Option<(usize, usize)> = pstate(ui.ctx(), SELECTED);
    // A set made with New Set, or whose name was clicked, is current with none of its actions
    // selected.
    if selected.is_some_and(|(s, _)| app.ui.action_set != Some(s)) {
        selected = None;
    }
    let mut current = app.ui.action_set;
    let mut sets_selected = selected_sets(app);
    let shift = ui.input(|i| i.modifiers.shift);
    let mut play_req = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (si, set) in app.ui.action_sets.iter().enumerate() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
            if sets_selected.contains(&si) && selected.is_none() {
                ui.painter().rect_filled(r, 0.0, t.row_selected);
            } else if resp.hovered() {
                ui.painter().rect_filled(r, 0.0, t.hover);
            }
            ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, format!("▾ {}", set.name), egui::FontId::proportional(12.5), t.text);
            if resp.clicked() {
                // Shift-click selects the sets from the current one to the one clicked; the current
                // set stays as it was.
                match current {
                    Some(c) if shift && sets_selected.contains(&c) => sets_selected = (c.min(si)..=c.max(si)).collect(),
                    _ => {
                        current = Some(si);
                        sets_selected = vec![si];
                    }
                }
                selected = None;
            }
            for (ai, a) in set.actions.iter().enumerate() {
                let sel = selected == Some((si, ai));
                let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 22.0), egui::Sense::click());
                if sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover);
                }
                ui.painter().text(
                    r.left_center() + egui::vec2(18.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    &a.name,
                    egui::FontId::proportional(12.5),
                    t.text_strong,
                );
                ui.painter().text(
                    r.right_center() - egui::vec2(6.0, 0.0),
                    egui::Align2::RIGHT_CENTER,
                    crate::i18n::tn(a.steps.len() as u64, "{n} step", "{n} steps"),
                    egui::FontId::proportional(11.0),
                    t.text_dim,
                );
                if resp.clicked() {
                    selected = Some((si, ai));
                    current = Some(si);
                    sets_selected = vec![si];
                }
                if resp.double_clicked() {
                    play_req = Some((si, ai));
                }
            }
        }
    });
    set_pstate(ui.ctx(), SELECTED, selected);
    app.ui.action_set = current;
    app.ui.action_sets_selected = sets_selected;
    widgets::divider(ui);
    // Bottom bar: stop, record, play, new set, delete.
    ui.horizontal(|ui| {
        let recording = app.ui.recording.is_some();
        if widgets::icon_button(ui, "square", tl!("Stop Playing/Recording"), false, 24.0).clicked()
            && let Some((set, name, start)) = app.ui.recording.take()
        {
            // A canceled undo group numbers the steps down when it drops entries made before them.
            let start = app.session.journal.held().unwrap_or(start);
            let steps = app.session.journal_for_action(start);
            app.session.journal.hold(None);
            if let Some(s) = app.ui.action_sets.get_mut(set) {
                s.actions.push(Action { name, steps });
            }
        }
        let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        ui.painter().circle_filled(r.center(), 6.0, if recording { egui::Color32::from_rgb(0xe0, 0x30, 0x30) } else { t.icon });
        if resp.on_hover_text(tl!("Begin Recording")).clicked() && !recording {
            if app.ui.action_sets.is_empty() {
                app.ui.action_sets.push(ActionSet::new("Set 1", vec![]));
            }
            let set = current.filter(|&s| s < app.ui.action_sets.len()).unwrap_or(0);
            let n = app.ui.action_sets.get(set).map_or(0, |s| s.actions.len()) + 1;
            let start = app.session.journal.len();
            // Keep every step from here on until recording stops, however long it runs.
            app.session.journal.hold(Some(start));
            app.ui.recording = Some((set, format!("Action {n}"), start));
        }
        if widgets::icon_button(ui, "dc-actions", tl!("Play Current Selection"), false, 24.0).clicked()
            && let Some(s) = selected
        {
            play_req = Some(s);
        }
        if widgets::icon_button(ui, "dc-folder", tl!("Create New Set"), false, 24.0).clicked() {
            crate::dialogs::action_set_options::open_new(app);
        }
        if widgets::icon_button(ui, "trash-2", tl!("Delete Selection"), false, 24.0).clicked() {
            delete_selection(app, ui.ctx());
        }
        if recording {
            ui.label(egui::RichText::new(tl!("● Recording")).color(egui::Color32::from_rgb(0xe0, 0x30, 0x30)));
        }
    });
    if let Some((si, ai)) = play_req
        && let Err(e) = play(app, si, ai)
    {
        app.status(e);
    }
    let _ = icons::exists;
    if app.ui.action_sets.is_empty() {
        dim_label(ui, tl!("No actions. Press ● to record."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    #[test]
    fn defaults_are_valid_commands() {
        for set in default_sets() {
            for a in set.actions {
                for (c, _) in a.steps {
                    assert!(vectorcraft_engine::find_command(&c).is_some(), "{c}");
                }
            }
        }
    }

    /// #1629: a clicked action stays selected, so Play Current Selection plays it and Delete
    /// Selection deletes it.
    #[test]
    fn a_clicked_action_stays_selected_for_play_and_delete() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        let id = vectorcraft_doc::NodeId(id);
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Reflect Horizontal").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((0, 5)), "selected on the frames after the click");
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, delete.
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, play, _, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        click(&ctx, &mut app, text_rect(&texts, "Duplicate and Offset 10 pt").center(), show);
        click(&ctx, &mut app, play.center(), show);
        assert_eq!(app.session.active().unwrap().selection.len(), 1);
        assert_ne!(app.session.active().unwrap().selection.objects, [id], "played: the copy is selected");
        let names = |app: &VectorcraftApp| app.ui.action_sets[0].actions.iter().map(|a| a.name.clone()).collect::<Vec<_>>();
        let before = names(&app);
        click(&ctx, &mut app, delete.center(), show);
        assert_eq!(names(&app), before[1..], "the selected action is deleted");
        click(&ctx, &mut app, delete.center(), show);
        assert_eq!(names(&app), before[1..], "nothing selected after a delete: nothing more goes");
    }

    /// #3712: Create New Set asks for a name (Set 1 when free), the new set is the current one, so a
    /// recording goes into it, and a click on another set's name makes that one current. The sets
    /// are saved with the UI preferences.
    #[test]
    fn create_new_set_names_a_set_that_recordings_go_into() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        let ctx = egui::Context::default();
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, delete. A new
        // set or action moves the bar down.
        let bar = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            frame_events(ctx, app, vec![], show);
            let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
            bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
            let [stop, record, _, new_set, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
            (stop, record, new_set)
        };
        let (_, _, new_set) = bar(&ctx, &mut app);
        click(&ctx, &mut app, new_set.center(), show);
        let d = app.ui.dialog.as_ref().expect("New Set asks for a name");
        assert_eq!((d.kind.as_str(), d.str("name")), ("actionSetOptions", "Set 1".to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Print"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let sets = |app: &VectorcraftApp| app.ui.action_sets.iter().map(|s| (s.name.clone(), s.actions.len())).collect::<Vec<_>>();
        assert_eq!(sets(&app), [("Default Actions".to_string(), 6), ("Print".into(), 0)]);
        let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
        let (stop, record, _) = bar(&ctx, &mut app);
        click(&ctx, &mut app, record.center(), show);
        app.run("shape.rectangle", rect.clone()).unwrap();
        click(&ctx, &mut app, stop.center(), show);
        assert_eq!(sets(&app), [("Default Actions".to_string(), 6), ("Print".into(), 1)], "recorded into the new set");
        assert_eq!(app.ui.action_sets[1].actions[0].name, "Action 1", "numbered within its set");
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "▾ Default Actions").center(), show);
        let (stop, record, _) = bar(&ctx, &mut app);
        click(&ctx, &mut app, record.center(), show);
        app.run("shape.rectangle", rect).unwrap();
        click(&ctx, &mut app, stop.center(), show);
        assert_eq!(sets(&app), [("Default Actions".to_string(), 7), ("Print".into(), 1)], "recorded into the set clicked");
        let saved = serde_json::to_value(&app.ui).unwrap();
        let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
        assert_eq!(restored.action_sets.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(), ["Default Actions", "Print"]);
    }

    /// #3712: the panel menu's Set Options… renames the current set and keeps its actions.
    #[test]
    fn set_options_renames_the_current_set() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.action_sets.push(ActionSet::new("Mine", vec![]));
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "▾ Mine").center(), show);
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Set Options…").map(|(_, r)| *r).expect("Set Options… in the menu");
        click(&ctx, &mut app, item.center(), menu);
        let d = app.ui.dialog.as_ref().expect("Set Options opens");
        assert_eq!((d.kind.as_str(), d.str("name")), ("actionSetOptions", "Mine".to_string()));
        app.ui.dialog.as_mut().unwrap().fields.insert("name".into(), json!("Web"));
        crate::dialogs::confirm(&mut app).unwrap();
        let names = app.ui.action_sets.iter().map(|s| (s.name.as_str(), s.actions.len())).collect::<Vec<_>>();
        assert_eq!(names, [("Default Actions", 6), ("Web", 0)], "the clicked set is renamed, the other kept");
    }

    /// An app with the default set and the sets Mine (two actions) and Other.
    fn app_with_sets() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let action = |name: &str| Action { name: name.into(), steps: vec![("object.move".into(), json!({"dx": 1, "dy": 0}))] };
        app.ui.action_sets.push(ActionSet::new("Mine", vec![action("Nudge"), action("Nudge Again")]));
        app.ui.action_sets.push(ActionSet::new("Other", vec![]));
        app
    }

    fn set_names(app: &VectorcraftApp) -> Vec<String> {
        app.ui.action_sets.iter().map(|s| s.name.clone()).collect()
    }

    /// The bottom bar's Delete Selection button (stop, record, play, new set, delete).
    fn trash(ctx: &egui::Context, app: &mut VectorcraftApp) -> egui::Pos2 {
        super::super::tests_appearance::frame_events(ctx, app, vec![], show);
        let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, _, _, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        delete.center()
    }

    /// A click on the name of set `name` with `modifiers` held.
    fn click_set(ctx: &egui::Context, app: &mut VectorcraftApp, name: &str, modifiers: egui::Modifiers) {
        use super::super::tests_appearance::{frame_events, text_rect};
        let pos = text_rect(&frame_events(ctx, app, vec![], show), &format!("▾ {name}")).center();
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
            frame_events(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), e], show);
        }
    }

    /// The question Delete Selection asks about sets, with the sets OK deletes.
    fn asked(app: &VectorcraftApp) -> Option<(String, Value)> {
        let d = app.ui.dialog.as_ref()?;
        (d.kind == crate::dialogs::confirm::KIND && d.str("__command") == DELETE_SETS)
            .then(|| (d.str("message"), d.fields["__params"]["sets"].clone()))
    }

    /// Delete Selection with a set selected asks first: Cancel keeps every set, OK deletes the
    /// selected set with its actions and keeps the others.
    #[test]
    fn delete_selection_asks_and_then_deletes_the_selected_set() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        let (message, sets) = asked(&app).expect("Delete Selection asks before deleting a set");
        assert_eq!((message.as_str(), sets), ("Delete the action set “Mine” and its actions?", json!([1])));
        crate::dialogs::cancel(&mut app);
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "Cancel deletes nothing");
        click(&ctx, &mut app, delete, show);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Other"], "OK deletes the selected set and its actions");
        assert_eq!(app.ui.action_set, None, "no set is current after the current one is deleted");
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert!(app.ui.dialog.is_none(), "nothing selected: nothing to ask about");
    }

    /// The panel menu's Delete asks the same question about the selected set, and deletes a
    /// selected action at once, as the bottom bar's button does.
    #[test]
    fn the_panel_menus_delete_deletes_the_selected_set_or_action() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let delete = |ctx: &egui::Context, app: &mut VectorcraftApp| {
            let texts = frame_events(ctx, app, vec![], menu);
            let item = texts.iter().find(|(s, _)| s.trim() == "Delete").map(|(_, r)| *r).expect("Delete in the menu");
            click(ctx, app, item.center(), menu);
        };
        click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
        delete(&ctx, &mut app);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Mine"]);
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        delete(&ctx, &mut app);
        assert!(app.ui.dialog.is_none(), "an action is deleted without a question");
        let actions = app.ui.action_sets[1].actions.iter().map(|a| a.name.as_str()).collect::<Vec<_>>();
        assert_eq!((set_names(&app).len(), actions), (2, vec!["Nudge Again"]), "the action goes, its set stays");
    }

    /// After New Set makes another set current, the panel menu's Delete asks about that set and
    /// leaves the action selected before it.
    #[test]
    fn delete_after_new_set_targets_the_new_set() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        crate::dialogs::action_set_options::open_new(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        let texts = frame_events(&ctx, &mut app, vec![], menu);
        let item = texts.iter().find(|(s, _)| s.trim() == "Delete").map(|(_, r)| *r).expect("Delete in the menu");
        click(&ctx, &mut app, item.center(), menu);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([3])), "the new set, not the action");
        assert_eq!(app.ui.action_sets[1].actions.len(), 2);
    }

    /// Shift-click on a set's name selects the sets from the current one to it, and one question
    /// deletes them all; the set left out stays.
    #[test]
    fn shift_click_selects_sets_that_one_question_deletes() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Mine", egui::Modifiers::NONE);
        click_set(&ctx, &mut app, "Other", egui::Modifiers::SHIFT);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1, 2]), "the current set stays");
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app), Some(("Delete the selected action sets and their actions?".to_string(), json!([1, 2]))));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions"]);
        // A plain click selects one set again.
        app.ui.action_sets.extend([ActionSet::new("A", vec![]), ActionSet::new("B", vec![])]);
        click_set(&ctx, &mut app, "A", egui::Modifiers::NONE);
        click_set(&ctx, &mut app, "Default Actions", egui::Modifiers::SHIFT);
        click_set(&ctx, &mut app, "B", egui::Modifiers::NONE);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(2), vec![2]));
    }

    /// The question holds the ids of the sets it asks about: when a set before the asked one is
    /// deleted while it is open, OK deletes the asked set at its new index, not the set that moved
    /// into the asked index.
    #[test]
    fn ok_deletes_the_asked_set_after_an_earlier_set_is_deleted_under_the_question() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        app.ui.action_sets.push(ActionSet::new("Third", vec![]));
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
        let id = app.ui.action_sets[2].id;
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.fields["__params"]["ids"].clone()), Some(json!([id])));
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Other", "Third"]);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Third"], "OK deletes Other, the asked set");
        let p = json!({"sets": [0], "ids": [id]});
        assert_eq!(app.run(DELETE_SETS, p).unwrap_err(), "the action sets changed; nothing was deleted");
        assert_eq!(set_names(&app), ["Default Actions", "Third"], "an id that is gone deletes nothing, sets is not read");
    }

    /// Two sets with the same name: when the asked one is deleted elsewhere while the question is
    /// open, OK does not delete the other one, which now has the asked index and name; when the
    /// two trade places, OK deletes the asked one at its new index.
    #[test]
    fn ok_deletes_only_the_asked_one_of_two_sets_with_the_same_name() {
        use super::super::tests_appearance::click;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let x = |n: &str| ActionSet::new("X", vec![Action { name: n.into(), steps: vec![] }]);
        app.ui.action_sets = vec![x("first"), x("second")];
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "X", egui::Modifiers::NONE);
        assert_eq!(app.ui.action_set, Some(0));
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([0])));
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert_eq!(app.ui.action_sets.iter().map(|s| s.actions[0].name.as_str()).collect::<Vec<_>>(), ["second"], "OK deletes nothing");
        app.ui.action_sets = vec![x("first"), x("second")];
        app.ui.action_set = Some(0);
        app.ui.action_sets_selected = vec![0];
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([0])));
        app.ui.action_sets.swap(0, 1);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(app.ui.action_sets.iter().map(|s| s.actions[0].name.as_str()).collect::<Vec<_>>(), ["second"], "the asked set goes");
    }

    /// When the asked set is deleted while the question is open and a new set with the same name
    /// takes its place, OK does not delete the new set.
    #[test]
    fn ok_does_not_delete_a_new_set_made_in_the_asked_sets_place() {
        use super::super::tests_appearance::click;
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Other", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        assert_eq!(asked(&app).map(|(_, sets)| sets), Some(json!([2])));
        app.run(DELETE_SETS, json!({"sets": [2]})).unwrap();
        app.ui.action_sets.push(ActionSet::new("Other", vec![]));
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"]);
        assert!(crate::dialogs::confirm(&mut app).is_err());
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "the new Other stays");
    }

    /// Every set has its own id, and sets read back from the saved preferences get new ids that
    /// no other set has.
    #[test]
    fn action_sets_get_ids_of_their_own_when_made_and_loaded() {
        let app = app_with_sets();
        let saved = serde_json::to_value(&app.ui).unwrap();
        assert!(saved["action_sets"][0].get("id").is_none(), "ids are not saved");
        let restored = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
        let mut ids: Vec<u64> = app.ui.action_sets.iter().chain(&restored.action_sets).map(|s| s.id).collect();
        assert_eq!(restored.action_sets.len(), 3);
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 6, "six sets, six ids");
    }

    /// Shift-click on another set's name while an action is selected selects the sets from the
    /// current one to it and clears the action.
    #[test]
    fn shift_click_on_a_set_with_an_action_selected_selects_the_range() {
        use super::super::tests_appearance::{click, frame_events, text_rect};
        let mut app = app_with_sets();
        let ctx = egui::Context::default();
        let texts = frame_events(&ctx, &mut app, vec![], show);
        click(&ctx, &mut app, text_rect(&texts, "Nudge").center(), show);
        frame_events(&ctx, &mut app, vec![], show);
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), Some((1, 0)));
        click_set(&ctx, &mut app, "Other", egui::Modifiers::SHIFT);
        assert_eq!((app.ui.action_set, app.ui.action_sets_selected.clone()), (Some(1), vec![1, 2]));
        assert_eq!(pstate::<Option<(usize, usize)>>(&ctx, "actions-selected"), None, "the action is no longer selected");
    }

    /// The default set can be deleted, and so can the last set: the panel is then empty, no set is
    /// current, and New Set makes a set again.
    #[test]
    fn deleting_the_default_and_the_last_set_leaves_an_empty_panel() {
        use super::super::tests_appearance::click;
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let ctx = egui::Context::default();
        click_set(&ctx, &mut app, "Default Actions", egui::Modifiers::NONE);
        let delete = trash(&ctx, &mut app);
        click(&ctx, &mut app, delete, show);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.action_sets.is_empty());
        assert_eq!(app.ui.action_set, None);
        crate::dialogs::action_set_options::open_new(&mut app);
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((set_names(&app), app.ui.action_set), (vec!["Set 1".to_string()], Some(0)), "New Set works on an empty panel");
    }

    /// `ui.actionSet.delete` keeps a recording into a later set going in the same set, stops one
    /// into a deleted set, and refuses an index past the last set.
    #[test]
    fn deleting_sets_keeps_a_recording_in_its_set() {
        let mut app = app_with_sets();
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.ui.action_set = Some(2);
        let start = app.session.journal.len();
        app.session.journal.hold(Some(start));
        app.ui.recording = Some((2, "Action 1".into(), start));
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [1]})).unwrap(), json!({"deleted": 1}));
        assert_eq!(set_names(&app), ["Default Actions", "Other"]);
        assert_eq!((app.ui.recording.as_ref().map(|r| r.0), app.ui.action_set), (Some(1), Some(1)), "still recording into Other");
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [5]})).unwrap_err(), "no such action set");
        assert_eq!(app.run(DELETE_SETS, json!({"sets": [4_294_967_296u64]})).unwrap_err(), "no such action set", "above u32::MAX");
        assert_eq!(set_names(&app), ["Default Actions", "Other"], "a refused index deletes nothing");
        assert_eq!(app.run(DELETE_SETS, json!({})).unwrap(), json!({"deleted": 1}), "the current set by default");
        assert_eq!(set_names(&app), ["Default Actions"]);
        assert!(app.ui.recording.is_none(), "the recording into the deleted set stops");
        assert_eq!(app.session.journal.held(), None);
    }

    /// `ui.actionSet.delete` while Set Options is open on a later set: OK renames that set, not
    /// the one that moves into its old index.
    #[test]
    fn set_options_renames_its_own_set_after_an_earlier_set_is_deleted() {
        let mut app = app_with_sets();
        crate::dialogs::action_set_options::open(&mut app, 2);
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        app.ui.dialog.as_mut().expect("Set Options stays open").fields.insert("name".into(), json!("Web"));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!(set_names(&app), ["Default Actions", "Web"], "the set that was Other is renamed");
    }

    /// `ui.actionSet.delete` closes a Batch dialog open on an action of a deleted set, and keeps
    /// one open on a later set pointing at that set.
    #[test]
    fn deleting_a_set_closes_its_batch_dialog() {
        let mut app = app_with_sets();
        crate::dialogs::action_batch::open(&mut app, 1, 0).unwrap();
        app.run(DELETE_SETS, json!({"sets": [1]})).unwrap();
        assert!(app.ui.dialog.is_none(), "the Batch dialog of the deleted set closes");
        let mut app = app_with_sets();
        crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();
        app.run(DELETE_SETS, json!({"sets": [0]})).unwrap();
        let d = app.ui.dialog.as_ref().expect("the Batch dialog of a kept set stays open");
        assert_eq!((d.fields["set"].clone(), d.fields["action"].clone()), (json!(0), json!(1)), "it follows Mine to index 0");
    }

    /// A recording keeps its steps when a canceled undo group drops journal entries made before the
    /// recording began.
    #[test]
    fn a_recording_keeps_its_steps_when_a_canceled_undo_group_drops_earlier_entries() {
        use super::super::tests_appearance::{click, frame_events};
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let rect = |x: i32| json!({"x": x, "y": 10, "width": 20, "height": 20});
        for (id, p) in [
            ("file.new", json!({"width": 200, "height": 100})),
            ("file.new", json!({"width": 200, "height": 100})),
            ("document.activate", json!({"index": 0})),
            ("edit.undoGroup.begin", json!({})),
            ("shape.rectangle", rect(10)),
        ] {
            app.run(id, p).unwrap();
        }
        let ctx = egui::Context::default();
        frame_events(&ctx, &mut app, vec![], show);
        // The bottom bar's 24-point buttons, left to right: stop, record, play, new set, delete.
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [stop, record, _, _, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
        click(&ctx, &mut app, record.center(), show);
        for (id, p) in [("document.activate", json!({"index": 1})), ("shape.rectangle", rect(50)), ("edit.undoGroup.end", json!({"cancel": true}))] {
            app.run(id, p).unwrap();
        }
        click(&ctx, &mut app, stop.center(), show);
        let steps = app.ui.action_sets[0].actions.last().map(|a| a.steps.iter().map(|(c, _)| c.clone()).collect::<Vec<_>>());
        assert_eq!(steps, Some(vec!["document.activate".to_string(), "shape.rectangle".into(), "edit.undoGroup.end".into()]));
    }

    /// Actions recorded with Align to Artboard and Paste in Place, and the default Center on
    /// Artboard, play on the artboard active when they play, not on the one active when they were
    /// recorded.
    #[test]
    fn actions_play_on_the_artboard_active_when_they_play() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        let r = app.run("shape.rectangle", json!({"x": boards[1].x0 + 10.0, "y": boards[1].y0 + 15.0, "width": 20, "height": 20})).unwrap();
        let rect = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
        let bounds = |app: &VectorcraftApp, ids: &[vectorcraft_doc::NodeId]| app.session.active().unwrap().doc.bounds_of(ids, false).unwrap();
        // Each recorded as the panel records an action, with artboard 0 active.
        let record = |app: &mut VectorcraftApp, name: &str, id: &str, params: Value| {
            let start = app.session.journal.len();
            app.run(id, params).unwrap();
            let steps = app.session.journal_for_action(start);
            app.ui.action_sets[0].actions.push(Action { name: name.into(), steps });
            app.ui.action_sets[0].actions.len() - 1
        };
        app.run("edit.copy", json!({})).unwrap();
        let align = record(&mut app, "Align Left", "object.align", json!({"horizontal": "left", "to": "artboard"}));
        assert_eq!(bounds(&app, &[rect]).x0, boards[0].x0, "aligned to artboard 0 while recording");
        let paste = record(&mut app, "Paste in Place", "edit.pasteInPlace", json!({}));
        app.view_mut().unwrap().artboard = 2;
        app.run("select.set", json!({"ids": [rect.0]})).unwrap();
        play(&mut app, 0, align).unwrap();
        assert_eq!(bounds(&app, &[rect]).x0, boards[2].x0, "aligned to artboard 2, active when the action plays");
        let ids: Vec<vectorcraft_doc::NodeId> = play(&mut app, 0, paste).unwrap()["results"][0]["ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| vectorcraft_doc::NodeId(v.as_u64().unwrap()))
            .collect();
        let b = bounds(&app, &ids);
        assert_eq!((b.x0, b.y0), (boards[2].x0 + 10.0, boards[2].y0 + 15.0), "pasted onto artboard 2, where it was on artboard 1");
        app.view_mut().unwrap().artboard = 1;
        app.run("select.set", json!({"ids": [rect.0]})).unwrap();
        let center = app.ui.action_sets[0].actions.iter().position(|a| a.name == "Center on Artboard").unwrap();
        play(&mut app, 0, center).unwrap();
        assert_eq!(bounds(&app, &[rect]).center(), boards[1].center(), "centered on artboard 1, the active one");
    }

    /// Actions that record Distribute Objects and Distribute Spacing with Align to Artboard play on
    /// the artboard active when they play, not on the one active when they were recorded.
    #[test]
    fn distribute_actions_play_on_the_artboard_active_when_they_play() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100, "artboards": 3})).unwrap();
        let boards: Vec<_> = app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
        // The rectangles are 20, 40 and 20 pt wide, and they are on artboard 1.
        let ids: Vec<vectorcraft_doc::NodeId> = [(10.0, 20.0), (50.0, 40.0), (120.0, 20.0)]
            .into_iter()
            .map(|(x, w)| {
                let r = app.run("shape.rectangle", json!({"x": boards[1].x0 + x, "y": boards[1].y0 + 15.0, "width": w, "height": 20})).unwrap();
                vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
            })
            .collect();
        app.run("select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
        let lefts =
            |app: &VectorcraftApp| ids.iter().map(|id| app.session.active().unwrap().doc.bounds_of(&[*id], false).unwrap().x0).collect::<Vec<_>>();
        // Each step is recorded as the panel records an action, while artboard 0 is active.
        let record = |app: &mut VectorcraftApp, name: &str, id: &str, params: Value| {
            let start = app.session.journal.len();
            app.run(id, params).unwrap();
            let steps = app.session.journal_for_action(start);
            app.ui.action_sets[0].actions.push(Action { name: name.into(), steps });
            app.ui.action_sets[0].actions.len() - 1
        };
        let (x0, x1) = (boards[0].x0, boards[0].x1);
        let distribute = record(&mut app, "Distribute Left", "object.distribute", json!({"horizontal": "left", "to": "artboard"}));
        assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "distributed across artboard 0 while recording");
        let spacing = record(&mut app, "Distribute Spacing", "object.distributeSpacing", json!({"axis": "horizontal", "to": "artboard"}));
        assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "spaced across artboard 0 while recording");
        app.view_mut().unwrap().artboard = 2;
        let (x0, x1) = (boards[2].x0, boards[2].x1);
        play(&mut app, 0, distribute).unwrap();
        assert_eq!(lefts(&app), [x0, x0 + 90.0, x1 - 20.0], "distributed across artboard 2, active when the action plays");
        play(&mut app, 0, spacing).unwrap();
        assert_eq!(lefts(&app), [x0, x0 + 80.0, x1 - 20.0], "spaced across artboard 2, active when the action plays");
    }
}
