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
    pub name: String,
    pub actions: Vec<Action>,
}

/// Built-in actions (our own, handy defaults).
pub fn default_sets() -> Vec<ActionSet> {
    let step = |c: &str, p: Value| (c.to_string(), p);
    vec![ActionSet {
        name: "Default Actions".into(),
        actions: vec![
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
    }]
}

/// Run an action as a single undoable batch.
pub fn play(app: &mut VectorcraftApp, set: usize, idx: usize) -> Result<Value, String> {
    let Some(a) = app.ui.action_sets.get(set).and_then(|s| s.actions.get(idx)).cloned() else { return Err("no such action".into()) };
    let commands: Vec<Value> = a.steps.iter().map(|(c, p)| json!({"command": c, "params": p})).collect();
    app.run("command.batch", json!({"label": a.name, "commands": commands}))
}

/// The panel's (≡) menu.
pub fn menu(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let selected: Option<(usize, usize)> = pstate(ui.ctx(), "actions-selected");
    let has = selected.and_then(|(s, a)| app.ui.action_sets.get(s).and_then(|set| set.actions.get(a))).is_some();
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
    let mut play_req = None;
    egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
        for (si, set) in app.ui.action_sets.iter().enumerate() {
            ui.label(egui::RichText::new(format!("▾ {}", set.name)).color(t.text));
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
                }
                if resp.double_clicked() {
                    play_req = Some((si, ai));
                }
            }
        }
    });
    set_pstate(ui.ctx(), SELECTED, selected);
    widgets::divider(ui);
    // Bottom bar: stop, record, play, new, delete.
    ui.horizontal(|ui| {
        let recording = app.ui.recording.is_some();
        if widgets::icon_button(ui, "square", tl!("Stop Playing/Recording"), false, 24.0).clicked()
            && let Some((set, name, start)) = app.ui.recording.take()
        {
            let steps = app.session.journal_for_action(start);
            app.session.journal.hold(None);
            if let Some(s) = app.ui.action_sets.get_mut(set) {
                s.actions.push(Action { name, steps });
            }
        }
        let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
        ui.painter().circle_filled(r.center(), 6.0, if recording { egui::Color32::from_rgb(0xe0, 0x30, 0x30) } else { t.icon });
        if resp.on_hover_text(tl!("Begin Recording")).clicked() && !recording {
            let n = app.ui.action_sets.first().map(|s| s.actions.len()).unwrap_or(0) + 1;
            if app.ui.action_sets.is_empty() {
                app.ui.action_sets.push(ActionSet { name: "Set 1".into(), actions: vec![] });
            }
            let set = selected.map(|s| s.0).unwrap_or(0);
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
        if widgets::icon_button(ui, "trash-2", tl!("Delete Selection"), false, 24.0).clicked()
            && let Some((si, ai)) = selected
            && let Some(s) = app.ui.action_sets.get_mut(si)
            && ai < s.actions.len()
        {
            s.actions.remove(ai);
            set_pstate::<Option<(usize, usize)>>(ui.ctx(), SELECTED, None);
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
        // The bottom bar's 24-point buttons, left to right: stop, record, play, delete.
        let mut bar = crate::toolbar::tests::widget_rects(&ctx, egui::vec2(24.0, 24.0));
        bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
        let [_, _, play, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
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
