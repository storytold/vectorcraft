//! Paragraph panel menu › Justification… (or `ui.justification`): Word Spacing, Letter Spacing and
//! Glyph Scaling (each Minimum, Desired and Maximum, in percent), Auto Leading and Single Word
//! Justification of the paragraphs the Paragraph panel acts on. The dialog opens on the values the
//! Paragraph panel shows (blank fields where paragraphs in scope differ) and previews them on the
//! canvas once Preview is on. OK runs `text.setFormat` with the values that changed, as one undo
//! step; paragraphs that differ keep the values left alone. OK is disabled while a row's Minimum
//! exceeds its Desired value or Desired its Maximum.
//!
//! Fields: `wordMin`, `wordDesired`, `wordMax` (0..1000), `letterMin`, `letterDesired`,
//! `letterMax` (-100..500), `glyphMin`, `glyphDesired`, `glyphMax` (50..200), `autoLeading`
//! (0..500), `singleWord` (`fullJustify`, `alignLeft`, `alignCenter`, `alignRight`) and `preview`.
//! The dialog acts on the type and range it opened on; with the Selection tool, every frame of each
//! story selected when it opened, locked and hidden frames included; after Select All in threaded
//! text, the story of the edited frame as the document holds it at OK.
//!
//! Behaviour reference (public documentation):
//! <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/adjust-word-and-letterspacing-in-justified-text.html>.

use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind};
// PLAN: assumed paths of `UnitKind` and `DialogId`
use craft::ui::dialog::DialogId;
use craft::ui::runtime::env::UnitKind;
use serde_json::{Map, Value, json};
use vectorcraft_doc::{JustificationOptions, SingleWordJustify};

use super::preview::LivePreview;
use super::warichu::set_shared;
use crate::VectorcraftApp;

/// The dialog kind of Justification.
pub const KIND: &str = "justification";
const CMD: &str = "text.setFormat";

/// The open Justification dialog: what it acts on and the preview it runs, kept on the app between
/// frames.
pub(crate) struct Open {
    /// The dialog on the host.
    pub id: DialogId,
    /// The drawing that owns the text IDs and the preview.
    document: u64,
    /// The preview on the canvas.
    pub preview: LivePreview,
    /// The `ids`, `start` and `end` the dialog's `text.setFormat` acts on.
    targets: Value,
    /// The frame the Type tool edits when the dialog opened after Select All in threaded text: the
    /// dialog then acts on that frame's story as the document holds it.
    story: Option<u64>,
    /// The values the dialog opened with.
    initial: DialogFields,
}

impl Open {
    fn in_opening_document(&self, app: &VectorcraftApp) -> bool {
        app.session.active().is_some_and(|st| st.uid == self.document)
    }
}

/// The three spacing rows: (`text.setFormat` key, field prefix, range).
const ROWS: [(&str, &str, (f64, f64)); 3] = [
    ("wordSpacing", "word", JustificationOptions::WORD_SPACING),
    ("letterSpacing", "letter", JustificationOptions::LETTER_SPACING),
    ("glyphScaling", "glyph", JustificationOptions::GLYPH_SCALING),
];
/// The columns of a row: (field suffix, `text.setFormat` key).
const COLUMNS: [(&str, &str); 3] = [("Min", "min"), ("Desired", "desired"), ("Max", "max")];

/// Single Word Justification's choices: (`text.setFormat` value, label).
fn single_word_choices() -> [(&'static str, &'static str); 4] {
    [("fullJustify", tl!("Full Justify")), ("alignLeft", tl!("Align left")), ("alignCenter", tl!("Align center")), ("alignRight", tl!("Align right"))]
}

fn single_word_name(s: SingleWordJustify) -> &'static str {
    match s {
        SingleWordJustify::FullJustify => "fullJustify",
        SingleWordJustify::AlignLeft => "alignLeft",
        SingleWordJustify::AlignCenter => "alignCenter",
        SingleWordJustify::AlignRight => "alignRight",
    }
}

/// Why OK can't apply the dialog: a row whose values are out of order.
fn problem(fields: &DialogFields) -> Option<&'static str> {
    let out_of_order = ROWS.iter().any(|(_, prefix, _)| {
        let [min, desired, max] = COLUMNS.map(|(suffix, _)| fields.number(&format!("{prefix}{suffix}")));
        match (min, desired, max) {
            (Some(min), Some(desired), Some(max)) => !(min <= desired && desired <= max),
            _ => false,
        }
    });
    out_of_order.then(|| tl!("Minimum must not exceed Desired, nor Desired Maximum."))
}

/// The dialog over its values: three titled groups of Minimum, Desired and Maximum, Auto Leading and
/// Single Word Justification. Preview starts off ([`HostedParamDialog::preview_off`]).
fn form(fields: &mut DialogFields) -> ParamDialog<'_> {
    let titles = [tl!("Word Spacing"), tl!("Letter Spacing"), tl!("Glyph Scaling")];
    let columns = [tl!("Minimum"), tl!("Desired"), tl!("Maximum")];
    let mut rows = Vec::new();
    for ((_, prefix, (lo, hi)), title) in ROWS.iter().zip(titles) {
        rows.push(Param::section(title));
        for ((suffix, _), label) in COLUMNS.iter().zip(columns) {
            rows.push(Param::number(format!("{prefix}{suffix}"), label, 100.0, 1.0, 0).bounds(Some(*lo), Some(*hi)).unit(UnitKind::Percent));
        }
    }
    let (lo, hi) = JustificationOptions::AUTO_LEADING;
    rows.push(Param::number("autoLeading", tl!("Auto Leading:"), 120.0, 1.0, 0).bounds(Some(lo), Some(hi)).unit(UnitKind::Percent));
    let options = single_word_choices().into_iter().map(|(id, label)| (id.to_owned(), label.into())).collect();
    rows.push(Param::new("singleWord", tl!("Single Word Justification:"), ParamKind::choice(options, Some("fullJustify".to_owned()))));
    let reason = problem(fields);
    let dialog = ParamDialog::new(KIND, tl!("Justification"), rows).previews(true).values(Bind::bound(fields));
    match reason {
        Some(why) => dialog.ok_enabled(false).ok_reason(why.into()),
        None => dialog,
    }
}

/// Open Justification on the paragraph attributes the Paragraph panel shows.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let document = app.session.active().ok_or("no document open")?.uid;
    // While typing, the dialog acts on the paragraphs of the text being edited.
    let mut targets = json!({});
    let mut story = None;
    if let Some((id, a, b)) = crate::panels::character::text_editing_kept(app)? {
        targets = json!({"ids": [id.0], "start": a, "end": b});
        // After Select All in threaded text: every paragraph of every frame ([`params`]).
        story = crate::panels::character::text_story(app).map(|_| id.0);
    } else {
        // With the Selection tool: every frame of each selected story.
        crate::panels::character::target_selected_stories(app, &mut targets);
    }
    let mut fields = DialogFields::new();
    for (_, prefix, _) in ROWS {
        for (i, (suffix, _)) in COLUMNS.iter().enumerate() {
            let shared = crate::panels::paragraph::shared_justification_f64(app, |j| match prefix {
                "word" => j.word_spacing[i],
                "letter" => j.letter_spacing[i],
                _ => j.glyph_scaling[i],
            });
            set_shared(&mut fields, &format!("{prefix}{suffix}"), shared);
        }
    }
    set_shared(&mut fields, "autoLeading", crate::panels::paragraph::shared_justification_f64(app, |j| j.auto_leading));
    set_shared(&mut fields, "singleWord", crate::panels::paragraph::shared_single_word_justify(app).map(single_word_name));
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::new(&ctx, KIND, fields.clone(), form).preview_off();
    let id = super::host::open_held(app, dialog, |app, result| {
        let Some(open) = app.justification.take() else { return Ok(()) };
        if !open.in_opening_document(app) {
            app.justification = Some(open);
            return Err(tl!("This dialog was opened in a different document.").to_owned());
        }
        let p = params(app, &result.fields, &open);
        if !changes(&p) {
            // Nothing to apply: end the preview as production's OK did.
            open.preview.cancel(app);
            return Ok(());
        }
        let Open { id, document, preview, targets, story, initial } = open;
        preview.commit(app, CMD, Value::Object(p)).map(|_| ()).inspect_err(|_| {
            // The dialog stays open on the refusal, with a preview of its own again.
            app.justification = Some(Open { id, document, preview: LivePreview::default(), targets, story, initial });
        })
    })
    .ok_or("no window")?;
    app.justification = Some(Open { id, document, preview: LivePreview::default(), targets, story, initial: fields });
    Ok(Value::Null)
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with,
/// on the dialog's type and range. After Select All in threaded text the type is every frame of
/// the edited frame's thread as the document holds it now; when that frame is no longer threaded,
/// every paragraph in that frame.
fn params(app: &VectorcraftApp, fields: &DialogFields, open: &Open) -> Map<String, Value> {
    let was = |key: &str| open.initial.number(key);
    let mut p = open.targets.as_object().cloned().unwrap_or_default();
    if let (Some(id), Some(st)) = (open.story, app.session.active()) {
        let story = st.doc.story_of(vectorcraft_doc::NodeId(id));
        p.remove("start");
        p.remove("end");
        if story.len() >= 2 {
            p.insert("ids".into(), json!(story.iter().map(|f| f.0).collect::<Vec<_>>()));
        }
    }
    for (param, prefix, _) in ROWS {
        let mut changed = Map::new();
        for (suffix, name) in COLUMNS {
            let key = format!("{prefix}{suffix}");
            if let Some(v) = fields.number(&key).filter(|v| was(&key) != Some(*v)) {
                changed.insert(name.into(), json!(v));
            }
        }
        if !changed.is_empty() {
            p.insert(param.into(), Value::Object(changed));
        }
    }
    if let Some(leading) = fields.number("autoLeading").filter(|v| was("autoLeading") != Some(*v)) {
        p.insert("autoLeading".into(), json!(leading));
    }
    if let Some(single) = fields.text("singleWord").filter(|s| open.initial.text("singleWord") != Some(*s)) {
        p.insert("singleWordJustification".into(), json!(single));
    }
    p
}

/// Does `p` change anything (more than the targets)?
fn changes(p: &Map<String, Value>) -> bool {
    p.keys().any(|k| !matches!(k.as_str(), "ids" | "start" | "end"))
}

/// Runs once per frame: previews the dialog's values while Preview is on and they are in order, and
/// rolls the preview back when the dialog went away without OK.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.justification.take() else { return };
    let shown =
        app.dialogs().and_then(|d| d.with(|h| h.get_mut_by_id::<HostedParamDialog>(open.id).map(|d| (d.is_previewing(), d.fields().clone()))));
    // A document switch ends that document's preview. The active drawing may have started a different one.
    if !open.in_opening_document(app) {
        if shown.is_some() {
            open.preview = LivePreview::default();
            app.justification = Some(open);
        }
        return;
    }
    // The host lock is released: a command never runs inside `DialogHost::with`.
    let Some((on, fields)) = shown else {
        open.preview.cancel(app);
        return;
    };
    let p = params(app, &fields, &open);
    let _ = open.preview.run(app, on && problem(&fields).is_none(), tl!("Justification"), CMD, &Value::Object(p));
    app.justification = Some(open);
}

#[cfg(test)]
mod tests {
    use super::super::warichu::driver;
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind, ParaStyle};

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

    /// `ui.justification` opens the dialog on the selected type's values; OK sets the paragraphs'
    /// justification options in one undo step.
    #[test]
    fn justification_dialog_sets_the_paragraphs_options() {
        let (mut app, id) = app();
        app.run("ui.justification", json!({})).unwrap();
        let f = driver::fields(&app);
        assert_eq!((f.number("wordMin"), f.number("wordDesired"), f.number("wordMax")), (Some(80.0), Some(100.0), Some(133.0)));
        assert_eq!((f.number("glyphDesired"), f.number("autoLeading"), f.text("singleWord")), (Some(100.0), Some(120.0), Some("fullJustify")));
        driver::set(&mut app, "wordMax", json!(150));
        driver::set(&mut app, "letterMax", json!(5));
        driver::set(&mut app, "glyphMin", json!(97));
        driver::set(&mut app, "autoLeading", json!(140));
        driver::set(&mut app, "singleWord", json!("alignRight"));
        let undo = app.session.active().unwrap().history.undo.len();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        let want = JustificationOptions {
            word_spacing: [80.0, 100.0, 150.0],
            letter_spacing: [0.0, 0.0, 5.0],
            glyph_scaling: [97.0, 100.0, 100.0],
            auto_leading: 140.0,
            single_word: SingleWordJustify::AlignRight,
        };
        for p in [paras(&app, id).0, paras(&app, id).1] {
            assert_eq!(p.justification, want);
        }
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    }

    /// OK sends only the values that changed: paragraphs whose other values differ keep them. Values
    /// out of order keep OK disabled and change nothing.
    #[test]
    fn justification_dialog_leaves_unchanged_values_alone_and_refuses_values_out_of_order() {
        let (mut app, id) = app();
        app.run("text.setFormat", json!({"ids": [id], "wordSpacing": [70, 100, 200], "start": 5, "end": 5})).unwrap();
        app.run("ui.justification", json!({})).unwrap();
        assert!(driver::fields(&app).is_mixed("wordMin"));
        driver::set(&mut app, "letterMax", json!(20));
        // Desired above Maximum: refused.
        driver::set(&mut app, "glyphDesired", json!(150));
        assert!(problem(&driver::fields(&app)).is_some());
        driver::ok(&mut app);
        assert!(driver::is_open(&app), "the dialog stays open");
        assert_eq!(paras(&app, id).0.justification, JustificationOptions::default());
        driver::set(&mut app, "glyphDesired", json!(100.0));
        driver::ok(&mut app);
        let (first, second) = paras(&app, id);
        assert_eq!((first.justification.word_spacing, first.justification.letter_spacing), ([80.0, 100.0, 133.0], [0.0, 0.0, 20.0]));
        assert_eq!((second.justification.word_spacing, second.justification.letter_spacing), ([70.0, 100.0, 200.0], [0.0, 0.0, 20.0]));
        // Nothing changed: no undo step.
        let undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.justification", json!({})).unwrap();
        driver::ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.justification", json!({})).is_err());
        assert!(!driver::is_open(&app));
    }

    #[test]
    fn justification_minimum_input_updates_validation_and_paragraphs() {
        use egui::accesskit::Role;
        use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};

        for lang in ["en", "de", "fr", "el"] {
            for height in [700.0, 500.0] {
                let (mut app, id) = app();
                app.session.prefs.interface_language = lang.into();
                let ctx = app.ui_ctx.clone().unwrap();
                ctx.enable_accesskit();
                let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, height));
                let undo = app.session.active().unwrap().history.undo.len();
                let mut time = 0.0;
                let mut draw = |app: &mut VectorcraftApp, events| {
                    time += 0.25;
                    crate::testing::app_frame(app, &ctx, RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() })
                };
                let control = |out: egui::FullOutput, role, name: &str| {
                    let nodes = out.platform_output.accesskit_update.unwrap().nodes;
                    let by_id: std::collections::HashMap<_, _> = nodes.iter().map(|(id, node)| (*id, node)).collect();
                    let dialog = nodes.iter().find(|(_, node)| node.role() == Role::Dialog).expect("Justification modal").0;
                    let mut pending = vec![dialog];
                    let mut matches = Vec::new();
                    while let Some(id) = pending.pop() {
                        let Some(node) = by_id.get(&id) else { continue };
                        pending.extend_from_slice(node.children());
                        if node.role() == role
                            && node.label() == Some(name)
                            && let Some(bounds) = node.bounds()
                        {
                            let at = pos2(((bounds.x0 + bounds.x1) * 0.5) as f32, ((bounds.y0 + bounds.y1) * 0.5) as f32);
                            let text = node
                                .children()
                                .iter()
                                .filter_map(|id| by_id.get(id))
                                .find(|child| child.role() == Role::TextInput)
                                .and_then(|child| child.bounds());
                            let target = text.map_or(at, |text| pos2(((text.x0 + text.x1) * 0.5) as f32, ((text.y0 + text.y1) * 0.5) as f32));
                            if screen.contains(target) {
                                matches.push((at, target, node.is_disabled()));
                            }
                        }
                    }
                    // Word Spacing is the first Minimum control in the modal.
                    let (_, target, disabled) = matches
                        .into_iter()
                        .min_by(|a, b| a.0.y.total_cmp(&b.0.y))
                        .unwrap_or_else(|| panic!("{lang} 800x{height}: missing {role:?} {name:?}"));
                    (target, disabled)
                };
                let click = |draw: &mut dyn FnMut(&mut VectorcraftApp, Vec<Event>) -> egui::FullOutput, app: &mut VectorcraftApp, at| {
                    draw(app, vec![Event::PointerMoved(at)]);
                    for pressed in [true, false] {
                        draw(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
                    }
                };
                let key = |key, modifiers| Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers };

                draw(&mut app, vec![]);
                app.run("ui.justification", json!({})).unwrap();
                for _ in 0..8 {
                    draw(&mut app, vec![]);
                }
                for (typed, value, invalid) in [("120", 120.0, true), ("85", 85.0, false)] {
                    let (min, _) = control(draw(&mut app, vec![]), Role::SpinButton, crate::i18n::t("Minimum"));
                    click(&mut draw, &mut app, min);
                    draw(&mut app, vec![key(Key::A, Modifiers::COMMAND)]);
                    draw(&mut app, vec![Event::Text(typed.into())]);
                    draw(&mut app, vec![key(Key::Tab, Modifiers::NONE)]);
                    draw(&mut app, vec![]);
                    assert_eq!(driver::fields(&app).number("wordMin"), Some(value));
                    let (ok, disabled) = control(draw(&mut app, vec![]), Role::Button, crate::i18n::t("OK"));
                    assert_eq!(disabled, invalid);
                    click(&mut draw, &mut app, ok);
                    for _ in 0..3 {
                        draw(&mut app, vec![]);
                    }
                    assert_eq!(driver::is_open(&app), invalid);
                    if invalid {
                        assert_eq!(paras(&app, id).0.justification.word_spacing, [80.0, 100.0, 133.0]);
                    }
                }
                assert_eq!(paras(&app, id).0.justification.word_spacing, [85.0, 100.0, 133.0]);
                assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
                crate::i18n::set_current(crate::i18n::Lang::EN);
            }
        }
    }

    /// Preview shows the values on the canvas; Cancel rolls them back.
    #[test]
    fn justification_dialog_previews_and_cancel_restores() {
        let (mut app, id) = app();
        app.run("ui.justification", json!({})).unwrap();
        driver::set(&mut app, "autoLeading", json!(200));
        driver::set(&mut app, "preview", json!(true));
        driver::pump(&mut app);
        assert!(app.session.in_interaction(), "the preview runs");
        assert_eq!(paras(&app, id).0.justification.auto_leading, 200.0);
        driver::cancel(&mut app);
        assert!(!driver::is_open(&app));
        assert_eq!(paras(&app, id).0.justification.auto_leading, 120.0);
    }

    /// A threaded story of three frames, first selected with the Type tool and Select All; a
    /// frame joined or left while the dialog is open is read at OK.
    fn threaded(frames: usize) -> (VectorcraftApp, Vec<u64>) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog and keeps running far beyond the first frame.\n".repeat(6);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let mut ids = vec![app.run("text.create", area).unwrap()["id"].as_u64().unwrap()];
        for i in 1..frames {
            let rect = json!([200 * i, 10, 150, if i + 1 == frames { 200 } else { 60 }]);
            let from = ids[i - 1];
            ids.push(app.run("text.thread.add", json!({"from": from, "rect": rect})).unwrap()["id"].as_u64().unwrap());
        }
        app.run("select.set", json!({"ids": [ids[0]]})).unwrap();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": ids[0], "anchor": 0, "caret": 0}));
        app.run("select.all", json!({})).unwrap();
        (app, ids)
    }

    /// After Select All in threaded text, OK changes the frames in the edited frame's thread when OK
    /// is pressed: not a frame that left the thread while the dialog was open, and a frame that
    /// joined it.
    #[test]
    fn justification_dialog_follows_the_story_thread_at_ok() {
        let (mut app, ids) = threaded(3);
        let (a, b, c) = (ids[0], ids[1], ids[2]);
        app.run("ui.justification", json!({})).unwrap();
        app.run("text.thread.break", json!({"id": b, "port": "out"})).unwrap();
        let d = app.run("text.thread.add", json!({"from": a, "rect": [600, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        driver::set(&mut app, "autoLeading", json!(140));
        driver::ok(&mut app);
        for id in [a, d, b] {
            assert_eq!(paras(&app, id).0.justification.auto_leading, 140.0, "frame {id} takes the change");
        }
        assert_eq!(paras(&app, c).0.justification.auto_leading, 120.0, "the frame that left keeps its paragraphs");
    }

    /// After Select All in threaded text, a thread broken after the edited frame while the dialog is
    /// open leaves the whole story in that frame, and OK changes every paragraph it now holds.
    #[test]
    fn justification_dialog_takes_the_story_into_a_frame_that_left_its_thread() {
        let (mut app, ids) = threaded(2);
        let a = ids[0];
        app.run("ui.justification", json!({})).unwrap();
        app.run("text.thread.break", json!({"id": a, "port": "out"})).unwrap();
        driver::set(&mut app, "autoLeading", json!(140));
        driver::ok(&mut app);
        let NodeKind::Text(t) = &app.session.active().unwrap().doc.node(NodeId(a)).unwrap().kind else { panic!("not text") };
        assert!(t.paragraph_count() >= 6, "the story flows into the frame: {} paragraphs", t.paragraph_count());
        for i in 0..t.paragraph_count() {
            assert_eq!(t.para_at(i).justification.auto_leading, 140.0, "paragraph {i}");
        }
    }

    /// A threaded story of three frames, its second frame locked and its third hidden, a type object
    /// of its own, and the first frame selected with the Selection tool → the app, the frames and
    /// the other type object.
    fn story_selected_in_its_first_frame() -> (VectorcraftApp, [u64; 3], u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 900, "height": 300})).unwrap();
        let story = "The quick brown fox jumps over the lazy dog.\n".repeat(12);
        let area = json!({"x": 10, "y": 10, "text": story, "size": 12, "area": {"width": 150, "height": 60}});
        let a = app.run("text.create", area).unwrap()["id"].as_u64().unwrap();
        let b = app.run("text.thread.add", json!({"from": a, "rect": [200, 10, 150, 60]})).unwrap()["id"].as_u64().unwrap();
        let c = app.run("text.thread.add", json!({"from": b, "rect": [400, 10, 150, 200]})).unwrap()["id"].as_u64().unwrap();
        let other = app.run("text.create", json!({"x": 10, "y": 250, "text": "Other", "size": 12})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [b]})).unwrap();
        app.run("object.lock", json!({})).unwrap();
        app.run("select.set", json!({"ids": [c]})).unwrap();
        app.run("object.hide", json!({})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        (app, [a, b, c], other)
    }

    /// Every paragraph's Auto Leading in `id`.
    fn auto_leadings(app: &VectorcraftApp, id: u64) -> Vec<f64> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => (0..t.paragraph_count()).map(|i| t.para_at(i).justification.auto_leading).collect(),
            _ => panic!("not text"),
        }
    }

    /// With the Selection tool a selected frame of threaded text stands for its whole story: the
    /// dialog opens blank where a locked or hidden frame differs, previews on every frame, Cancel
    /// restores them, and OK changes every frame of the story as it was when the dialog opened, in
    /// one undo step, leaving other type and a selection made later alone.
    #[test]
    fn selection_tool_justification_reaches_every_frame_of_the_story() {
        let (mut app, frames, other) = story_selected_in_its_first_frame();
        let [a, _, c] = frames;
        for id in frames {
            assert!(!auto_leadings(&app, id).is_empty(), "frame {id} holds text");
        }
        app.run("text.setFormat", json!({"ids": [c], "autoLeading": 150})).unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.run("ui.justification", json!({})).unwrap();
        assert!(driver::fields(&app).is_mixed("autoLeading"), "the hidden frame differs");
        driver::set(&mut app, "autoLeading", json!(200));
        driver::set(&mut app, "preview", json!(true));
        driver::pump(&mut app);
        for id in frames {
            assert!(auto_leadings(&app, id).iter().all(|l| *l == 200.0), "the preview shows on frame {id}");
        }
        driver::cancel(&mut app);
        assert!(auto_leadings(&app, a).iter().all(|l| *l == 120.0));
        assert!(auto_leadings(&app, c).iter().all(|l| *l == 150.0), "Cancel restores the hidden frame");

        app.run("ui.justification", json!({})).unwrap();
        // A selection made while the dialog is open does not change what it acts on.
        app.run("select.set", json!({"ids": [other]})).unwrap();
        driver::set(&mut app, "autoLeading", json!(140));
        let undo = app.session.active().unwrap().history.undo.len();
        driver::ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1, "one undo step");
        for id in frames {
            assert!(auto_leadings(&app, id).iter().all(|l| *l == 140.0), "frame {id} takes the change");
        }
        assert!(auto_leadings(&app, other).iter().all(|l| *l == 120.0), "other type keeps its paragraphs");
        app.run("edit.undo", json!({})).unwrap();
        assert!(auto_leadings(&app, c).iter().all(|l| *l == 150.0), "undo restores the hidden frame");
    }

    fn two_drawings() -> (VectorcraftApp, u64) {
        let (mut app, id) = app();
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let second = app.run("text.create", json!({"x": 10, "y": 10, "text": "Other drawing"})).unwrap()["id"].as_u64().unwrap();
        assert_eq!(id, second, "local text IDs may match between drawings");
        app.run("document.activate", json!({"index": 0})).unwrap();
        (app, id)
    }

    #[test]
    fn justification_approval_leaves_text_in_the_other_drawing_unchanged() {
        let (mut app, id) = two_drawings();
        app.run("ui.justification", json!({})).unwrap();
        driver::set(&mut app, "autoLeading", json!(180));
        app.run("document.activate", json!({"index": 1})).unwrap();
        driver::ok(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 120.0);
        assert!(driver::is_open(&app));
        assert_eq!(driver::fields(&app).number("autoLeading"), Some(180.0));
        app.run("document.activate", json!({"index": 0})).unwrap();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        assert_eq!(paras(&app, id).0.justification.auto_leading, 180.0);
    }

    #[test]
    fn justification_preview_leaves_text_in_the_other_drawing_unchanged() {
        let (mut app, id) = two_drawings();
        app.run("ui.justification", json!({})).unwrap();
        driver::set(&mut app, "autoLeading", json!(180));
        app.run("document.activate", json!({"index": 1})).unwrap();
        driver::set(&mut app, "preview", json!(true));
        driver::pump(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 120.0);
        assert!(!app.session.in_interaction());
        app.run("document.activate", json!({"index": 0})).unwrap();
        driver::pump(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 180.0);
        driver::cancel(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 120.0);
    }

    #[test]
    fn cancelling_justification_keeps_the_other_drawings_new_interaction() {
        let (mut app, id) = two_drawings();
        app.run("ui.justification", json!({})).unwrap();
        driver::set(&mut app, "autoLeading", json!(180));
        driver::set(&mut app, "preview", json!(true));
        driver::pump(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 180.0);
        app.run("document.activate", json!({"index": 1})).unwrap();
        app.begin_interaction("New text edit").unwrap();
        app.session.preview("text.setFormat", &json!({"ids": [id], "autoLeading": 190})).unwrap();
        driver::cancel(&mut app);
        assert_eq!(paras(&app, id).0.justification.auto_leading, 190.0);
        assert!(app.session.in_interaction());
    }
}
