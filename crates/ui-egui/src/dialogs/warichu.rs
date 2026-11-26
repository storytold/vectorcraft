//! Character panel menu › Warichu Settings… (or `ui.warichuSettings`): the Lines, Line Gap,
//! Scale and Alignment of the characters the Character panel acts on, which set how a block of
//! warichu characters is stacked inside its line. The dialog opens on the values the selected
//! characters share (a blank field where they differ). OK runs `text.setFormat`
//! (`text.setRangeStyle` while the Type tool edits text) with the values that changed, as one undo
//! step; characters that differ keep a value left blank. It does not turn Warichu on or off.
//!
//! Fields: `warichuLines` (1..6), `warichuGap` (pt, 0..100), `warichuScale` (%, 10..100) and
//! `warichuAlign`, as `text.setFormat` takes them.
//!
//! Behavior reference (public documentation):
//! <https://helpx.adobe.com/illustrator/using/formatting-asian-characters.html>.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind};
// PLAN: assumed path of `UnitKind`
use craft::ui::runtime::env::UnitKind;
use serde_json::{Map, Value, json};
use vectorcraft_doc::WarichuAlign;
use vectorcraft_doc::text::{MAX_WARICHU_GAP, WARICHU_LINES, WARICHU_SCALE};

use crate::VectorcraftApp;
// PLAN: assumed `LengthUnits` path
use craft::ui::runtime::env::LengthUnits;

/// The dialog kind of Warichu Settings.
pub const KIND: &str = "warichuSettings";

/// The fields, as `text.setFormat` names them.
const KEYS: [&str; 4] = ["warichuLines", "warichuGap", "warichuScale", "warichuAlign"];

/// The Alignment choices, by the value `text.setFormat` takes and their label.
fn aligns() -> [(&'static str, &'static str); 8] {
    WarichuAlign::ALL.map(|a| {
        let label = match a {
            WarichuAlign::Auto => tl!("Auto"),
            WarichuAlign::Left => tl!("Align left"),
            WarichuAlign::Center => tl!("Align center"),
            WarichuAlign::Right => tl!("Align right"),
            WarichuAlign::JustifyLeft => tl!("Justify with last line aligned left"),
            WarichuAlign::JustifyCenter => tl!("Justify with last line aligned center"),
            WarichuAlign::JustifyRight => tl!("Justify with last line aligned right"),
            WarichuAlign::JustifyAll => tl!("Justify all lines"),
        };
        (a.id(), label)
    })
}

/// Sets `key` to the value the characters share, or marks it mixed where they differ. The one rule the
/// East Asian and paragraph dialogs open their values by.
pub(super) fn set_shared(fields: &mut DialogFields, key: &str, shared: Option<impl Into<FieldValue>>) {
    match shared {
        Some(v) => fields.set(key, v),
        None => fields.set_mixed(key),
    }
}

/// The rows of the dialog; Line Gap is shown in `lengths`.
fn rows(lengths: LengthUnits) -> Vec<Param> {
    let options = aligns().into_iter().map(|(id, label)| (id.to_owned(), label.into())).collect();
    vec![
        Param::new(KEYS[0], tl!("Lines:"), ParamKind::int(Some(i64::from(*WARICHU_LINES.start())), Some(i64::from(*WARICHU_LINES.end())), 2)),
        Param::number(KEYS[1], tl!("Line Gap:"), 0.0, 1.0, 2).length(lengths).bounds(Some(0.0), Some(MAX_WARICHU_GAP)),
        Param::number(KEYS[2], tl!("Scale:"), 50.0, 1.0, 1).bounds(Some(*WARICHU_SCALE.start()), Some(*WARICHU_SCALE.end())).unit(UnitKind::Percent),
        Param::new(KEYS[3], tl!("Alignment:"), ParamKind::choice(options, Some(WarichuAlign::Auto.id().to_owned()))),
    ]
}

/// Open Warichu Settings on the characters the Character panel acts on.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    crate::panels::character::text_style(app).ok_or("select some type first")?;
    let shared = crate::panels::character::shared_warichu(app);
    let mut fields = DialogFields::new();
    set_shared(&mut fields, KEYS[0], shared.lines.map(i64::from));
    set_shared(&mut fields, KEYS[1], shared.gap);
    set_shared(&mut fields, KEYS[2], shared.scale);
    set_shared(&mut fields, KEYS[3], shared.align.map(|a| a.id()));
    let initial = fields.clone();
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = crate::canvas::rulers::type_lengths(app);
    let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |_| ParamDialog::new(KIND, tl!("Warichu Settings"), rows(lengths)));
    super::host::open_held(app, dialog, move |app, result| {
        let p = params(&result.fields, &initial);
        if p.is_empty() {
            return Ok(());
        }
        crate::panels::character::run_format(app, Value::Object(p)).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

/// `text.setFormat` parameters: the fields that differ from the values the dialog opened with.
/// Lines is rounded to a whole number and kept within 1..6, Line Gap is in points and kept within
/// 0..100, and Scale is kept within 10..100. A mixed field is left alone. A number is compared
/// with the value the dialog opened with before it is rounded or kept within its range, so OK
/// leaves a value outside the range (Line Gap on a scaled object) as it is when it was not edited.
fn params(fields: &DialogFields, initial: &DialogFields) -> Map<String, Value> {
    let opened_with = |key: &str, v: f64| initial.number(key) == Some(v);
    let mut p = Map::new();
    if let Some(n) = fields.number(KEYS[0]).filter(|&n| !opened_with(KEYS[0], n)) {
        let n = n.round().clamp(f64::from(*WARICHU_LINES.start()), f64::from(*WARICHU_LINES.end()));
        if !opened_with(KEYS[0], n) {
            // Within 1..6 after the clamp, so the cast keeps the value.
            p.insert(KEYS[0].into(), json!(n as u8));
        }
    }
    if let Some(v) = fields.number(KEYS[1]).filter(|&v| !opened_with(KEYS[1], v)).map(|v| v.clamp(0.0, MAX_WARICHU_GAP))
        && !opened_with(KEYS[1], v)
    {
        p.insert(KEYS[1].into(), json!(v));
    }
    if let Some(n) = fields.number(KEYS[2]).filter(|&n| !opened_with(KEYS[2], n)) {
        let v = n.clamp(*WARICHU_SCALE.start(), *WARICHU_SCALE.end());
        if !opened_with(KEYS[2], v) {
            p.insert(KEYS[2].into(), json!(v));
        }
    }
    if let Some(id) = fields.text(KEYS[3]).filter(|id| WarichuAlign::from_id(id).is_some() && initial.text(KEYS[3]) != Some(*id)) {
        p.insert(KEYS[3].into(), json!(id));
    }
    p
}

/// What the tests of the held East Asian and paragraph dialogs drive: the host's open dialog, as the
/// control channel and the user's clicks reach it.
#[cfg(test)]
pub(super) mod driver {
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
    // PLAN: assumed paths of `DialogId` and `StackDialog`
    use craft::ui::dialog::{DialogId, StackDialog};
    use serde_json::Value;

    use crate::VectorcraftApp;

    fn ctx(app: &VectorcraftApp) -> egui::Context {
        app.ui_ctx.clone().expect("installed")
    }

    fn id(app: &VectorcraftApp) -> DialogId {
        app.dialogs().expect("window").with(|h| h.get::<HostedParamDialog>().map(|d| d.id())).expect("a hosted dialog is open")
    }

    /// Is a hosted dialog open?
    pub fn is_open(app: &VectorcraftApp) -> bool {
        app.dialogs().is_some_and(|d| d.with(|h| h.get::<HostedParamDialog>().is_some()))
    }

    /// The open dialog's values.
    pub fn fields(app: &VectorcraftApp) -> DialogFields {
        app.dialogs().expect("window").with(|h| h.get::<HostedParamDialog>().map(|d| d.fields().clone())).expect("a hosted dialog is open")
    }

    /// Sets a field as automation does.
    pub fn set(app: &mut VectorcraftApp, key: &str, value: Value) {
        try_set(app, key, value).expect("the field takes the value");
    }

    /// Sets a field as automation does, with the dialog's refusal.
    pub fn try_set(app: &mut VectorcraftApp, key: &str, value: Value) -> Result<(), String> {
        let (ctx, id) = (ctx(app), id(app));
        app.dialogs().expect("window").with(|h| h.set_json_field(&ctx, id, key, value))
    }

    /// Runs the frame's pump.
    pub fn pump(app: &mut VectorcraftApp) {
        let ctx = ctx(app);
        crate::dialogs::host::pump(app, &ctx);
    }

    /// Presses OK: the dialog files its answer, the pump answers it and the dialog finishes.
    pub fn ok(app: &mut VectorcraftApp) {
        let (ctx, id) = (ctx(app), id(app));
        let host = app.dialogs().expect("window");
        host.with(|h| h.confirm_by_id(&ctx, id));
        pump(app);
        host.with(|h| h.confirm_by_id(&ctx, id));
    }

    /// Presses Cancel.
    pub fn cancel(app: &mut VectorcraftApp) {
        let (ctx, id) = (ctx(app), id(app));
        app.dialogs().expect("window").with(|h| h.cancel_by_id(&ctx, id));
        pump(app);
    }
}

#[cfg(test)]
mod tests {
    use super::driver;
    use super::*;
    use vectorcraft_engine::Session;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// A selected point type object with four characters.
    fn app() -> (VectorcraftApp, u64) {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 50, "text": "abcd"})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        (app, id)
    }

    /// (warichu, Lines, Line Gap, Scale, Alignment) of each run.
    fn settings(app: &VectorcraftApp, id: u64) -> Vec<(bool, u8, f64, f64, WarichuAlign)> {
        match &app.session.active().unwrap().doc.node(NodeId(id)).unwrap().kind {
            NodeKind::Text(t) => t
                .runs
                .iter()
                .map(|r| (r.style.warichu, r.style.warichu_lines, r.style.warichu_gap, r.style.warichu_scale, r.style.warichu_align))
                .collect(),
            _ => panic!("not text"),
        }
    }

    /// `ui.warichuSettings` opens on the characters' values; OK sets the changed ones in one undo
    /// step without turning Warichu on. A value past the ends is refused as it is set (the framework's
    /// `FieldRefusal::Rejects`); OK keeps an unedited one to them (the test below). Automation sets the
    /// gap in points, as a number row takes it.
    #[test]
    fn warichu_settings_dialog_sets_lines_gap_scale_and_alignment() {
        let (mut app, id) = app();
        app.run("ui.warichuSettings", json!({})).unwrap();
        let f = driver::fields(&app);
        assert_eq!((f.number("warichuLines"), f.number("warichuGap"), f.number("warichuScale")), (Some(2.0), Some(0.0), Some(50.0)));
        assert_eq!(f.text("warichuAlign"), Some("auto"));
        driver::set(&mut app, "warichuLines", json!(3));
        driver::set(&mut app, "warichuGap", json!(36));
        assert!(driver::try_set(&mut app, "warichuScale", json!(400)).unwrap_err().contains("warichuScale"));
        assert_eq!(driver::fields(&app).number("warichuScale"), Some(50.0), "a refused value leaves the field");
        driver::set(&mut app, "warichuScale", json!(100));
        driver::set(&mut app, "warichuAlign", json!("center"));
        let undo = app.session.active().unwrap().history.undo.len();
        driver::ok(&mut app);
        assert!(!driver::is_open(&app));
        assert_eq!(settings(&app, id), [(false, 3, 36.0, 100.0, WarichuAlign::Center)]);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // It opens on the values set, and OK with nothing changed adds no undo step.
        app.run("ui.warichuSettings", json!({})).unwrap();
        let f = driver::fields(&app);
        assert_eq!((f.number("warichuLines"), f.number("warichuGap"), f.number("warichuScale")), (Some(3.0), Some(36.0), Some(100.0)));
        driver::ok(&mut app);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        // With no type selected the dialog does not open.
        app.run("select.set", json!({"ids": []})).unwrap();
        assert!(app.run("ui.warichuSettings", json!({})).is_err());
        assert!(!driver::is_open(&app));
    }

    /// A value it opened with outside its range (Line Gap of 200 pt on an object scaled by 2) is
    /// left alone when OK is pressed without editing it; an edited one is kept within the range.
    #[test]
    fn warichu_settings_dialog_leaves_an_unedited_value_outside_its_range() {
        let mut initial = DialogFields::new();
        initial.set("warichuLines", 9_i64);
        initial.set("warichuGap", 200.0);
        initial.set("warichuScale", 5.0);
        let mut fields = initial.clone();
        assert_eq!(params(&fields, &initial), Map::new());
        fields.set("warichuGap", 150.0);
        assert_eq!(params(&fields, &initial).get("warichuGap"), Some(&json!(MAX_WARICHU_GAP)));
    }

    /// While the Type tool edits text, OK sets the selected characters.
    #[test]
    fn warichu_settings_dialog_sets_the_range_the_type_tool_selects() {
        let (mut app, id) = app();
        app.session.select_tool("type", Default::default()).unwrap();
        app.session.set_tool_option("textSelection", &json!({"editing": id, "anchor": 0, "caret": 2}));
        app.run("ui.warichuSettings", json!({})).unwrap();
        driver::set(&mut app, "warichuAlign", json!("justifyAll"));
        driver::ok(&mut app);
        assert_eq!(settings(&app, id), [(false, 2, 0.0, 50.0, WarichuAlign::JustifyAll), (false, 2, 0.0, 50.0, WarichuAlign::Auto)]);
    }
}
