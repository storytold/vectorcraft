//! The Actions panel: record commands from the engine journal, play them back as one undo step.
//! Every recorded step is an ordinary command invocation, so actions are also plain JSON that
//! agents can author and replay (`command.batch`).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

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

pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let selected_id = egui::Id::new("actions-selected");
    let mut selected: Option<(usize, usize)> = ui.data(|d| d.get_temp(selected_id));
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
    ui.data_mut(|d| d.insert_temp(selected_id, selected));
    widgets::divider(ui);
    // Bottom bar: stop, record, play, new, delete.
    ui.horizontal(|ui| {
        let recording = app.ui.recording.is_some();
        if widgets::icon_button(ui, "square", tl!("Stop Playing/Recording"), false, 24.0).clicked()
            && let Some((set, name, start)) = app.ui.recording.take()
        {
            let steps = app.session.journal_for_action(start);
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
            app.ui.recording = Some((set, format!("Action {n}"), app.session.journal.len()));
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
            ui.data_mut(|d| d.insert_temp::<Option<(usize, usize)>>(selected_id, None));
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
}
