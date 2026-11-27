//! Paragraph panel menu › Hyphenation… (or `ui.hyphenation`): the Hyphenation checkbox and the
//! limits automatic hyphenation keeps to in the paragraphs the Paragraph panel acts on: Words
//! Longer Than, After First, Before Last, Hyphen Limit, Hyphenation Zone and Hyphenate Capitalized
//! Words. The dialog opens on the values the Paragraph panel shows (blank fields and dashed
//! checkboxes where paragraphs in scope disagree). OK runs `text.setFormat` with the values that
//! changed, as one undo step, and paragraphs that differ keep the values left alone.
//!
//! Fields: `hyphenate`, `wordsLongerThan` (letters, 1..25), `afterFirst` and `beforeLast`
//! (letters, 1..15), `hyphenLimit` (hyphens, 0..25, 0: no limit), `hyphenationZone` (pt) and
//! `hyphenateCapitalized`, as `text.setFormat` takes them.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
// PLAN: assumed paths of `UnitKind` and `LengthUnits`
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Map, Value, json};

use super::warichu::set_shared;
use crate::VectorcraftApp;

/// The dialog kind of Hyphenation.
pub const KIND: &str = "hyphenation";

/// The check boxes.
const FLAGS: [&str; 2] = ["hyphenate", "hyphenateCapitalized"];
/// The numbers: (field, range).
const NUMBERS: [(&str, i64, i64); 5] =
    [("wordsLongerThan", 1, 25), ("afterFirst", 1, 15), ("beforeLast", 1, 15), ("hyphenLimit", 0, 25), ("hyphenationZone", 0, 0)];

/// The rows of the dialog; the zone is shown in `lengths`.
fn rows(lengths: LengthUnits) -> Vec<Param> {
    let count =
        |(key, lo, hi): (&str, i64, i64), label, default, unit| Param::new(key, label, ParamKind::int(Some(lo), Some(hi), default)).unit(unit);
    vec![
        Param::new(FLAGS[0], tl!("Hyphenation"), ParamKind::bool(false)),
        count(NUMBERS[0], tl!("Words Longer Than:"), 5, UnitKind::Letters),
        count(NUMBERS[1], tl!("After First:"), 3, UnitKind::Letters),
        count(NUMBERS[2], tl!("Before Last:"), 3, UnitKind::Letters),
        count(NUMBERS[3], tl!("Hyphen Limit:"), 0, UnitKind::Hyphens),
        Param::number(NUMBERS[4].0, tl!("Hyphenation Zone:"), 0.0, 1.0, 2).length(lengths).bounds(Some(0.0), None),
        Param::new(FLAGS[1], tl!("Hyphenate Capitalized Words"), ParamKind::bool(true)),
    ]
}

/// Open Hyphenation on the paragraph attributes the Paragraph panel shows.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    use crate::panels::character;

    let editing = character::text_editing_kept(app)?;
    character::text_style(app).ok_or("select some type first")?;
    let source = app.session.active().ok_or("no document open")?;
    let document = source.uid;
    let mut target = Map::new();
    let mut story_focus = None;
    let ids = if let Some((id, start, end)) = editing {
        if let Some(story) = character::text_story(app) {
            story_focus = Some(id);
            story
        } else {
            target.insert("start".into(), json!(start));
            target.insert("end".into(), json!(end));
            vec![id]
        }
    } else {
        character::selected_stories(app)
    };
    if ids.is_empty() || source.selection.symbol_part.is_some() {
        return Err("select some type first".into());
    }
    target.insert("ids".into(), json!(ids.iter().map(|id| id.0).collect::<Vec<_>>()));
    let mut fields = DialogFields::new();
    set_shared(&mut fields, FLAGS[0], crate::panels::paragraph::shared_hyphenate(app));
    set_shared(&mut fields, FLAGS[1], crate::panels::paragraph::shared_hyphenation_field(app, |h| h.capitalized));
    set_shared(&mut fields, NUMBERS[0].0, crate::panels::paragraph::shared_hyphenation_field(app, |h| h.words_longer_than).map(i64::from));
    set_shared(&mut fields, NUMBERS[1].0, crate::panels::paragraph::shared_hyphenation_field(app, |h| h.after_first).map(i64::from));
    set_shared(&mut fields, NUMBERS[2].0, crate::panels::paragraph::shared_hyphenation_field(app, |h| h.before_last).map(i64::from));
    set_shared(&mut fields, NUMBERS[3].0, crate::panels::paragraph::shared_hyphenation_field(app, |h| h.hyphen_limit).map(i64::from));
    set_shared(&mut fields, NUMBERS[4].0, crate::panels::paragraph::shared_hyphenation_zone(app));
    let initial = fields.clone();
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = crate::canvas::rulers::general_lengths(app);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |fields| {
        ParamDialog::new(KIND, tl!("Hyphenation"), rows(lengths)).values(Bind::bound(fields))
    });
    super::host::open_held(app, dialog, move |app, result| {
        let mut p = params(&result.fields, &initial);
        if p.is_empty() {
            return Ok(());
        }
        if app.session.active().is_none_or(|st| st.uid != document) {
            return Err(tl!("This dialog was opened in a different document.").to_owned());
        }
        character::text_editing_kept(app)?;
        p.extend(target.clone());
        if let Some(id) = story_focus {
            let story = app.session.active().ok_or("no document open")?.doc.story_of(id);
            p.insert("ids".into(), json!(story.iter().map(|id| id.0).collect::<Vec<_>>()));
        }
        app.run("text.setFormat", Value::Object(p)).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with.
fn params(fields: &DialogFields, initial: &DialogFields) -> Map<String, Value> {
    let mut p = Map::new();
    for key in FLAGS {
        if let Some(v) = fields.bool(key).filter(|v| initial.bool(key) != Some(*v)) {
            p.insert(key.into(), json!(v));
        }
    }
    for (key, ..) in NUMBERS {
        if let Some(v) = fields.number(key).filter(|v| initial.number(key) != Some(*v)) {
            p.insert(key.into(), json!(v));
        }
    }
    p
}

#[cfg(test)]
mod tests {
    use super::super::warichu::driver;
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{HyphenationOptions, NodeId, NodeKind, ParaStyle};

    /// A selected area type object with two paragraphs.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 10, "text": "One\nTwo", "area": {"width": 200, "height": 100}})).unwrap()["id"]
            .as_u64()
            .unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    fn paras(app: &VectorcraftApp, id: u64) -> (ParaStyle, ParaStyle) {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (t.para_at(0).clone(), t.para_at(1).clone()),
            _ => panic!("not text"),
        }
    }

    /// Actual focused Hyphenation edits keep the hosted draft and apply to both selected paragraphs.
    #[test]
    fn hyphenation_keeps_focused_edits_until_cancel_or_ok() {
        use craft::automation::{ControlRequest, Reply};
        use egui::accesskit::{Node, Role, TreeUpdate};
        use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        fn control(tree: &TreeUpdate, source: &str, role: Role, scope: Option<Rect>) -> Rect {
            fn plain(s: &str) -> &str {
                s.trim_end().trim_end_matches([':', '：']).trim_end()
            }
            fn bounds(node: &Node) -> Option<Rect> {
                let b = node.bounds()?;
                Some(Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
            }
            let expected = crate::i18n::t(source);
            tree.nodes
                .iter()
                .map(|(_, n)| n)
                .filter(|n| n.role() == role && n.label().is_some_and(|label| plain(label) == plain(expected)))
                .filter_map(bounds)
                .find(|rect| scope.is_none_or(|scope| scope.contains(rect.center())))
                .unwrap_or_else(|| panic!("missing {role:?} {expected:?} in {scope:?}"))
        }

        fn draft(app: &VectorcraftApp, ctx: &egui::Context) -> Value {
            let ui = crate::control::inspect(app, ctx);
            ui["dialogs"].as_array().unwrap().iter().find(|dialog| dialog["key"] == KIND).expect("open Hyphenation dialog")["fields"].clone()
        }

        let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        let mut failures = Vec::new();
        for lang in ["en", "de", "fr", "ja"] {
            for answer in ["Cancel", "OK", "ui.dialog.confirm"] {
                let (mut app, id) = app();
                app.session.prefs.interface_language = lang.into();
                app.run("text.setFormat", json!({"ids": [id], "afterFirst": 2, "start": 5, "end": 5})).unwrap();
                let ctx = app.ui_ctx.clone().unwrap();
                let mut time = 0.0;
                let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
                    time += 0.1;
                    let raw = RawInput { events, time: Some(time), screen_rect: Some(screen), focused: true, ..Default::default() };
                    crate::testing::app_frame(app, &ctx, raw).platform_output.accesskit_update.unwrap()
                };
                let settle = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate| {
                    for _ in 0..5 {
                        frame(app, vec![]);
                    }
                    frame(app, vec![])
                };
                let click = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate, pos| {
                    frame(app, vec![Event::PointerMoved(pos)]);
                    for pressed in [true, false] {
                        frame(app, vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
                    }
                    frame(app, vec![]);
                };
                let press = |app: &mut VectorcraftApp, frame: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> TreeUpdate, key, modifiers| {
                    for pressed in [true, false] {
                        frame(
                            app,
                            vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }],
                        );
                    }
                };
                let original = paras(&app, id);
                let undo = app.session.active().unwrap().history.undo.len();
                settle(&mut app, &mut frame);
                let apply = answer != "Cancel";
                app.run("ui.hyphenation", json!({})).unwrap();
                let tree = settle(&mut app, &mut frame);
                let modal = control(&tree, "Hyphenation", Role::Dialog, None);
                let count = control(&tree, "Words Longer Than:", Role::SpinButton, Some(modal));
                let enabled = control(&tree, "Hyphenation", Role::CheckBox, Some(modal));
                let footer = control(&tree, if apply { "OK" } else { "Cancel" }, Role::Button, Some(modal));
                assert!(screen.expand(0.5).contains_rect(modal));
                for bounds in [count, enabled, footer] {
                    assert!(bounds.is_positive() && modal.expand(0.5).contains_rect(bounds), "{lang}: {bounds:?} outside {modal:?}");
                }
                let before = driver::fields(&app);
                assert_eq!(before.number("wordsLongerThan"), Some(5.0));
                assert_eq!(before.bool("hyphenate"), Some(false));
                assert!(before.is_mixed("afterFirst"));
                click(&mut app, &mut frame, count.center());
                press(&mut app, &mut frame, Key::A, Modifiers::COMMAND);
                frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text("8".into())]);
                press(&mut app, &mut frame, Key::Tab, Modifiers::NONE);
                settle(&mut app, &mut frame);
                let typed = draft(&app, &ctx)["wordsLongerThan"].as_f64();
                click(&mut app, &mut frame, enabled.center());
                let inspected = draft(&app, &ctx);
                let kept = (inspected["wordsLongerThan"].as_f64(), inspected["hyphenate"].as_bool());
                eprintln!("{lang}, {answer}: {modal:?}; ui.inspect after typing {typed:?}, after checkbox {kept:?}");
                if typed != Some(8.0) || kept != (Some(8.0), Some(true)) || !driver::fields(&app).is_mixed("afterFirst") {
                    failures.push(format!("{lang} {answer}: {typed:?}, {kept:?}"));
                }
                assert_eq!(paras(&app, id), original, "{lang}: only the draft changes while open");
                if answer == "ui.dialog.confirm" {
                    let (request, _) = ControlRequest::new(answer, json!({}));
                    match crate::control::handle(&mut app, &ctx, &request) {
                        Reply::Done(reply) => assert_eq!(reply["ok"], true, "{reply}"),
                        _ => panic!("confirmation should reply immediately"),
                    }
                } else {
                    click(&mut app, &mut frame, footer.center());
                }
                settle(&mut app, &mut frame);
                assert!(!driver::is_open(&app), "{lang}: footer closed the dialog");
                let actual = paras(&app, id);
                let undo_delta = app.session.active().unwrap().history.undo.len() - undo;
                eprintln!(
                    "{lang}, {answer}: committed counts {}/{}, hyphenation {}/{}, after first {}/{}, undo {undo_delta}",
                    actual.0.hyphenation.words_longer_than,
                    actual.1.hyphenation.words_longer_than,
                    actual.0.hyphenate,
                    actual.1.hyphenate,
                    actual.0.hyphenation.after_first,
                    actual.1.hyphenation.after_first
                );
                if apply {
                    let mut expected = original.clone();
                    for p in [&mut expected.0, &mut expected.1] {
                        p.hyphenation.words_longer_than = 8;
                        p.hyphenate = true;
                    }
                    if actual != expected || undo_delta != 1 {
                        failures.push(format!(
                            "{lang} {answer}: left {:?} and {:?}, undo delta {undo_delta}",
                            actual.0.hyphenation, actual.1.hyphenation
                        ));
                    }
                } else {
                    assert_eq!(actual, original, "{lang}: Cancel discards both edits");
                    assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
                }
            }
            crate::i18n::set_current(crate::i18n::Lang::EN);
        }
        assert!(failures.is_empty(), "Hyphenation lost physical edits: {failures:?}");
    }

    /// `ui.hyphenation` opens the dialog on the selected type; OK sets the paragraphs' hyphenation
    /// options in one undo step.
    #[test]
    fn hyphenation_dialog_sets_the_paragraphs_options() {
        let (mut app, id) = app();
        app.run("ui.hyphenation", json!({})).unwrap();
        let f = driver::fields(&app);
        assert_eq!((f.bool("hyphenate"), f.number("wordsLongerThan"), f.number("hyphenLimit")), (Some(false), Some(5.0), Some(0.0)));
        assert_eq!(f.bool("hyphenateCapitalized"), Some(true));
        driver::set(&mut app, "hyphenate", json!(true));
        driver::set(&mut app, "wordsLongerThan", json!(8));
        driver::set(&mut app, "hyphenLimit", json!(2));
        // A length row holds points (1 in), whatever unit its field shows.
        driver::set(&mut app, "hyphenationZone", json!(72));
        driver::set(&mut app, "hyphenateCapitalized", json!(false));
        let undo = app.session.active().unwrap().history.undo.len();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        let want = HyphenationOptions { words_longer_than: 8, hyphen_limit: 2, zone: 72.0, capitalized: false, ..HyphenationOptions::default() };
        for p in [paras(&app, id).0, paras(&app, id).1] {
            assert!(p.hyphenate);
            assert_eq!(p.hyphenation, want);
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    /// OK sends only the values that changed: paragraphs whose other options differ keep them, and
    /// OK with nothing edited adds no undo step.
    #[test]
    fn hyphenation_dialog_leaves_unchanged_values_alone() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "afterFirst": 2, "start": 5, "end": 5})).unwrap();
        app.run("ui.hyphenation", json!({})).unwrap();
        assert!(driver::fields(&app).is_mixed("afterFirst"));
        driver::set(&mut app, "beforeLast", json!(4));
        driver::ok(&mut app);
        let (first, second) = paras(&app, id);
        assert_eq!((first.hyphenation.after_first, first.hyphenation.before_last), (3, 4));
        assert_eq!((second.hyphenation.after_first, second.hyphenation.before_last), (2, 4));
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.hyphenation", json!({})).unwrap();
        driver::ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.hyphenation", json!({})).is_err());
        assert!(!driver::is_open(&app));
    }

    #[test]
    fn hyphenation_confirm_keeps_the_opening_selection() {
        let (mut app, first) = app();
        let second = app.run("text.create", json!({"x": 10, "y": 150, "text": "Other text"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [first]})).unwrap();
        app.run("ui.hyphenation", json!({})).unwrap();
        driver::set(&mut app, "hyphenate", json!(true));
        app.run("select.set", json!({"ids": [second]})).unwrap();
        driver::ok(&mut app);
        assert_eq!((paras(&app, first).0.hyphenate, paras(&app, second).0.hyphenate), (true, false));
    }

    #[test]
    fn hyphenation_confirm_keeps_the_opening_caret_paragraph() {
        let (mut app, id) = app();
        app.session.select_tool("type", Default::default()).unwrap();
        let caret = |at| json!({"editing": id, "anchor": at, "caret": at});
        app.session.set_tool_option("textSelection", &caret(0));
        app.run("ui.hyphenation", json!({})).unwrap();
        driver::set(&mut app, "hyphenate", json!(true));
        app.session.set_tool_option("textSelection", &caret(5));
        driver::ok(&mut app);
        let (first, second) = paras(&app, id);
        assert_eq!((first.hyphenate, second.hyphenate), (true, false));
    }

    #[test]
    fn hyphenation_confirms_the_caret_paragraph_after_buffered_typing() {
        let (mut app, id) = app();
        app.run("prefs.set", json!({"key": "realTimeDrawing", "value": false})).unwrap();
        let view = Default::default();
        app.session.select_tool("type", view).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 7, "caret": 7}));
        app.session.tool_text("\nThree", view).unwrap();
        let text = &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind;
        assert!(matches!(text, NodeKind::Text(t) if t.plain_text() == "One\nTwo"));

        app.run("ui.hyphenation", json!({})).unwrap();
        driver::set(&mut app, "hyphenate", json!(true));
        driver::ok(&mut app);
        let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind else { panic!("not text") };
        assert_eq!(t.plain_text(), "One\nTwo\nThree");
        assert_eq!([t.para_at(0).hyphenate, t.para_at(1).hyphenate, t.para_at(2).hyphenate], [false, false, true]);
    }

    #[test]
    fn hyphenation_confirm_leaves_the_other_drawing_unchanged() {
        let (mut app, id) = app();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let second = app.run("text.create", json!({"x": 10, "y": 10, "text": "Other drawing"})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(id, second, "local text IDs may match between drawings");
        app.run("select.set", json!({"ids": [second]})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.hyphenation", json!({})).unwrap();
        driver::set(&mut app, "hyphenate", json!(true));
        app.run("document.activate", json!({"index": 1})).unwrap();
        driver::ok(&mut app);
        assert!(!paras(&app, second).0.hyphenate);
        assert!(driver::is_open(&app));
        assert_eq!(driver::fields(&app).bool("hyphenate"), Some(true));

        app.run("document.activate", json!({"index": 0})).unwrap();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        assert!(paras(&app, id).0.hyphenate);
    }
}
