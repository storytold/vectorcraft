//! Pathfinder panel: Shape Modes (+ Expand) and Pathfinders, with icons drawn for VectorCraft.
//! Alt-clicking a Shape Mode makes a compound shape; with members of one selected, a Shape Mode
//! sets theirs.

use craft::behavior::{IconId, Mixed};
use craft::keymap::CommandRef;
use craft::theme::{ColorToken, Srgba};
use craft::ui::buttons::{Button, IconButton, ToggleButton};
use craft::ui::contract::{Bind, Gap, MemoProps, Props as _, Show as _, ShowContainer as _, TooltipSpec};
use craft::ui::layout_components::Row;
use craft::ui::menu::PanelRow;
use craft::ui::primitives::{Glyph, IconSpec, paint_icon};
use craft::ui::runtime::commands::{CommandState, CommandStates};
use craft::ui::runtime::env::Direction;
use craft::ui::style::UiTokens;
use egui::{Align2, Color32, FontId, Painter, Rect, Stroke, Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::doc::ShapeMode;

use super::selection_len;
use crate::VectorcraftApp;

pub const SHAPE_MODES: [(&str, &str, &str); 4] = [
    ("dc-pf-unite", "Unite", "unite"),
    ("dc-pf-minus-front", "Minus Front", "minusFront"),
    ("dc-pf-intersect", "Intersect", "intersect"),
    ("dc-pf-exclude", "Exclude", "exclude"),
];

pub const PATHFINDERS: [(&str, &str, &str); 6] = [
    ("dc-pf-divide", "Divide", "divide"),
    ("dc-pf-trim", "Trim", "trim"),
    ("dc-pf-merge", "Merge", "merge"),
    ("dc-pf-crop", "Crop", "crop"),
    ("dc-pf-outline", "Outline", "outline"),
    ("dc-pf-minus-back", "Minus Back", "minusBack"),
];

/// Returns the minimum number of operands required by a Pathfinder operation.
pub(crate) fn minimum_operands(op: &str) -> usize {
    if matches!(op, "outline" | "divide") { 1 } else { 2 }
}

/// The panel name for one of the ten repeatable Shape Mode or Pathfinder operations.
pub fn operation_label(op: &str) -> Option<&'static str> {
    SHAPE_MODES.into_iter().chain(PATHFINDERS).find(|(_, _, id)| *id == op).map(|(_, label, _)| label)
}

fn run_op(app: &mut VectorcraftApp, op: &str) {
    // The command path reports a failed operation in the status bar.
    let _ = app.run(&format!("object.pathfinder.{op}"), json!({}));
}

/// Repeat the last successful Shape Mode or Pathfinder operation on the current selection.
pub fn repeat(app: &mut VectorcraftApp) -> Result<Value, String> {
    match app.last_pathfinder.clone() {
        Some(op) => {
            let min = minimum_operands(&op);
            if operands(app) < min {
                return Err("Select two or more objects, or a group".into());
            }
            app.run(&format!("object.pathfinder.{op}"), json!({}))
        }
        None => Err("no Pathfinder operation applied yet".into()),
    }
}

/// How many objects the selection gives an operation: a group selected alone counts as the shapes
/// the operation takes from it, nested groups included (#1450, #1635).
pub(crate) fn operands(app: &VectorcraftApp) -> usize {
    let alone = app.session.active().filter(|d| d.selection.len() == 1).and_then(|d| d.doc.node(*d.selection.objects.first()?));
    match alone.filter(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Group { .. })) {
        Some(group) => vectorcraft_engine::cmd::shape_count(group),
        None => selection_len(app),
    }
}

/// A Shape Mode's icon. While `mixed` (the selected members' modes differ) a question mark sits on a
/// disc of the panel color in its lower right corner, so it reads over the icon.
#[derive(MemoProps)]
struct ShapeModeGlyph {
    icon: IconId,
    mixed: bool,
}

impl Glyph for ShapeModeGlyph {
    fn paint(&self, painter: &Painter, tokens: &UiTokens, rect: Rect, _color: Color32, _mirrored: bool) {
        // The icon has no reading direction, so left to right draws it as it is.
        paint_icon(painter, tokens, Direction::Ltr, rect, &IconSpec::svg(self.icon.clone()));
        if !self.mixed {
            return;
        }
        let token = |t: ColorToken| Color32::from(Srgba::from(tokens.tokens().color(t)));
        let side = (rect.width() * 0.42).round();
        let mark = Rect::from_min_size(rect.right_bottom() - vec2(side, side), vec2(side, side));
        let painter = painter.with_clip_rect(rect);
        painter.circle_filled(mark.center(), side * 0.5, token(ColorToken::SurfacePanel));
        painter.circle_stroke(mark.center(), side * 0.5, Stroke::new(1.0, token(ColorToken::LineBorder)));
        painter.text(mark.center(), Align2::CENTER_CENTER, "?", FontId::proportional(side * 0.8), token(ColorToken::TextNormal));
    }
}

/// The tooltip of an operation button: its name, and what it needs while it is off.
fn tip(label: &str) -> TooltipSpec {
    TooltipSpec::new(tl!(label).to_string()).disabled_text(tl!("Select two or more objects, or a group").to_string())
}

/// A Shape Mode button activated: Alt makes a compound shape of the selection in `mode`; with
/// members of compound shapes selected they take `mode`; otherwise operation `op` runs. With the
/// Compound Shapes add-on's setting on, a click on the selection (not on members) makes the
/// compound shape and Alt gives the permanent result.
fn shape_mode(app: &mut VectorcraftApp, op: &str, mode: ShapeMode, members: bool, alt: bool) {
    if members {
        if alt {
            crate::menus::invoke(app, "object.compoundShape.make", json!({"mode": mode.name()}));
        } else {
            crate::menus::invoke(app, "object.compoundShape.setMode", json!({"mode": mode.name()}));
        }
    } else if alt != app.ui.shape_modes_compound {
        crate::menus::invoke(app, "object.compoundShape.make", json!({"mode": mode.name()}));
    } else {
        run_op(app, op);
    }
}

pub fn show(app: &mut VectorcraftApp, ui: &mut Ui) {
    let n = operands(app);
    // The modes of the compound-shape members selected: their Shape Mode button lights when they
    // share one, and every Shape Mode button shows a question mark when they differ.
    let modes = vectorcraft_engine::cmd::selected_member_modes(&app.session);
    let shared = modes.first().copied().filter(|m| modes.iter().all(|x| x == m));
    let mixed = !modes.is_empty() && shared.is_none();
    // Selected members can always take a mode.
    let count = if modes.is_empty() { n } else { n.max(2) };
    crate::panels::subheading(tl!("Shape Modes:")).show(ui, |ui, _| {
        Row::new().gap(Gap::Item).show(ui, |ui, _| {
            for ((icon, label, op), mode) in SHAPE_MODES.into_iter().zip(ShapeMode::ALL) {
                let glyph = ShapeModeGlyph { icon: IconId::new(icon), mixed };
                let value = if mixed { Mixed::Mixed } else { Mixed::Same(shared == Some(mode)) };
                let out = ToggleButton::new(tl!(label).to_string(), Bind::given(value))
                    .icon(IconSpec::glyph(&glyph))
                    .icon_only()
                    .enabled(count >= 2)
                    .tooltip(tip(label))
                    .show(ui);
                if !out.events.is_empty() {
                    shape_mode(app, op, mode, !modes.is_empty(), out.activation_mods.alt);
                }
            }
            Button::command(CommandRef::new("object.compoundShape.expand"))
                .label(tl!("Expand").to_string())
                .tooltip(TooltipSpec::new(tl!("Expand Compound Shape").to_string()))
                .show(ui);
        });
    });
    crate::panels::subheading(tl!("Pathfinders:")).show(ui, |ui, _| {
        Row::new().gap(Gap::Item).show(ui, |ui, _| {
            for (icon, label, op) in PATHFINDERS {
                IconButton::command(IconSpec::svg(IconId::new(icon)), CommandRef::new(format!("object.pathfinder.{op}")))
                    .tooltip(tip(label))
                    .show(ui);
            }
        });
    });
}

/// Pathfinder panel › Trap…: the Trap dialog (`object.pathfinder.trap`'s options) for two or more
/// objects.
pub fn open_trap(app: &mut VectorcraftApp) {
    let params = vectorcraft_effects::trap::command_defaults();
    let _ = app.run("ui.paramDialog", json!({"command": "object.pathfinder.trap", "label": "Trap", "params": params}));
}

/// The rows of the panel menu; each is a command, which supplies its label, enabled state and action.
pub(crate) fn menu_rows(_app: &VectorcraftApp) -> Vec<PanelRow> {
    let row = |id: &str| PanelRow::Command(CommandRef::new(id));
    vec![
        row("ui.pathfinder.trap"),
        row("pathfinder.repeat"),
        row("object.pathfinder.options"),
        PanelRow::Separator,
        row("object.compoundShape.make"),
        row("object.compoundShape.release"),
        row("object.compoundShape.expand"),
    ]
}

/// The states of the panel's own menu commands; the others read the commands' own rules.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    states.set(CommandRef::new("ui.pathfinder.trap"), CommandState::new().enabled(operands(app) >= 2));
}

/// Runs the panel's own menu commands; `None` for any other id.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    (id == "ui.pathfinder.trap").then(|| {
        open_trap(app);
        Ok(Value::Null)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::panel_harness::Panel;
    use craft::automation::actions::{Action, ActionStep};
    use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog};
    use craft::ui::runtime::commands::{FirePhase, FireSource, Fired};
    use egui::Modifiers;
    use vectorcraft_engine::doc::{NodeId, NodeKind};

    /// What a Shape Mode toggle reports: off, lit, or mixed.
    const OFF: Option<Mixed<bool>> = Some(Mixed::Same(false));
    const LIT: Option<Mixed<bool>> = Some(Mixed::Same(true));
    const MIXED: Option<Mixed<bool>> = Some(Mixed::Mixed);

    /// #1635: a stroked object expanded inside a group leaves a group holding one group (its fill
    /// and its outlined stroke). The Shape Modes take the group's shapes, so they stay enabled, and
    /// Unite merges the fill and the stroke into one path.
    #[test]
    fn shape_modes_take_the_shapes_of_an_expanded_stroke_in_a_group() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
        app.run("stroke.set", json!({"weight": 10})).unwrap();
        app.run("object.group", json!({})).unwrap();
        app.run("object.expand", json!({})).unwrap();
        let doc = &app.session.active().unwrap().doc;
        let group = doc.node(*app.session.active().unwrap().selection.objects.first().unwrap()).unwrap();
        let members = group.children().unwrap();
        assert_eq!(members.len(), 1, "one member: the expanded rectangle");
        assert!(matches!(members[0].kind, NodeKind::Group { .. }), "the fill and the outlined stroke, grouped");
        assert_eq!(operands(&app), 2, "the fill and the outlined stroke");
        let id = app.run("object.pathfinder.unite", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let united = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        assert!(matches!(united.kind, NodeKind::Path { .. }), "one path, the stroke's inner edge gone: {:?}", united.kind);
        let b = united.geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (5.0, 5.0, 65.0, 65.0), "the stroke's outer edge");
    }

    /// Whether the panel menu's Trap… row is enabled, as the panel menu shows it.
    fn trap_enabled(app: &VectorcraftApp) -> bool {
        let mut states = CommandStates::new();
        menu_states(app, &mut states);
        states.get(&CommandRef::new("ui.pathfinder.trap")).is_some_and(CommandState::is_enabled)
    }

    /// The panel menu's compound shape items: Make with two or more objects selected, Release and
    /// Expand with a compound shape (or a member of one); the Expand button follows Expand, and
    /// with a member selected its Shape Mode button is lit.
    #[test]
    fn compound_shape_items_follow_the_selection() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap()["id"].clone();
        let mut p = Panel::new(app, show);
        let state = |p: &mut Panel| {
            p.frame();
            let app = p.app();
            let on = |id: &str| crate::menus::enabled(&app, id);
            (on("object.compoundShape.make"), on("object.compoundShape.release"), on("object.compoundShape.expand"), p.enabled("Expand"))
        };
        p.app().run("select.set", json!({"ids": [a]})).unwrap();
        assert_eq!(state(&mut p), (false, false, false, false), "one object");
        p.app().run("select.set", json!({"ids": [a, b]})).unwrap();
        assert_eq!(state(&mut p), (true, false, false, false), "two objects");
        p.app().run("object.compoundShape.make", json!({"mode": "subtract"})).unwrap();
        assert_eq!(state(&mut p), (false, true, true, true), "a compound shape");
        p.app().run("select.set", json!({"ids": [b]})).unwrap();
        assert_eq!(state(&mut p), (false, true, true, true), "a member");
        assert_eq!(p.toggled("Minus Front"), LIT, "the member's mode");
        assert!(p.enabled("Unite"), "a member takes another mode");
        assert_ne!(p.toggled("Unite"), LIT);
    }

    /// Two overlapping squares, the first at (`x`, 10), made into a Unite compound shape, which is
    /// left selected; returns the ids of its two members.
    fn compound_of_two(app: &mut VectorcraftApp, x: f64) -> [Value; 2] {
        select_two_squares(app, x);
        let ids = app.session.active().unwrap().selection.objects.clone();
        app.run("object.compoundShape.make", json!({"mode": "add"})).unwrap();
        [json!(ids[0].0), json!(ids[1].0)]
    }

    /// Selects `members` and gives them `mode`.
    fn set_mode(app: &mut VectorcraftApp, members: &[&Value], mode: &str) {
        app.run("select.set", json!({"ids": members})).unwrap();
        app.run("object.compoundShape.setMode", json!({"mode": mode})).unwrap();
    }

    /// The pressed state the toggle of each Shape Mode button reports.
    fn shape_mode_states(p: &mut Panel) -> Vec<Option<Mixed<bool>>> {
        p.frame();
        SHAPE_MODES.into_iter().map(|(_, label, _)| p.toggled(label)).collect()
    }

    /// Selected members in different modes show a mixed state on each of the four Shape Mode
    /// buttons, which stay enabled; the Pathfinder buttons are left as they are. A click gives every
    /// selected member the clicked mode in one undo step, and the mixed state goes.
    #[test]
    fn shape_modes_show_a_question_mark_while_the_selected_members_modes_differ() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let [a, b] = compound_of_two(&mut app, 10.0);
        set_mode(&mut app, &[&b], "subtract");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let mut p = Panel::new(app, show);
        let modes = vectorcraft_engine::cmd::selected_member_modes(&p.app().session);
        assert_eq!(modes.len(), 2);
        assert_ne!(modes[0], modes[1], "the members' modes differ");
        assert_eq!(shape_mode_states(&mut p), [MIXED; 4]);
        for (_, label, _) in SHAPE_MODES {
            assert!(p.enabled(label), "{label}");
        }
        for (_, label, _) in PATHFINDERS {
            assert_eq!(p.toggled(label), None, "{label}");
        }

        let undo = p.app().session.doc().unwrap().history.undo.len();
        p.click("Intersect");
        assert_eq!(vectorcraft_engine::cmd::selected_member_modes(&p.app().session), [ShapeMode::Intersect; 2]);
        assert_eq!(p.app().session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert_eq!(shape_mode_states(&mut p), [OFF, OFF, LIT, OFF]);
    }

    /// Members of two compound shapes are compared together: in different modes the buttons are
    /// mixed, and in one mode that mode's button is lit.
    #[test]
    fn members_of_two_compound_shapes_are_compared_together() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        let [a, _] = compound_of_two(&mut app, 10.0);
        let [c, _] = compound_of_two(&mut app, 110.0);
        set_mode(&mut app, &[&c], "exclude");
        app.run("select.set", json!({"ids": [a, c]})).unwrap();
        let mut p = Panel::new(app, show);
        assert_eq!(shape_mode_states(&mut p), [MIXED; 4]);
        set_mode(&mut p.app(), &[&a, &c], "exclude");
        assert_eq!(shape_mode_states(&mut p), [OFF, OFF, OFF, LIT]);
    }

    /// No mixed state without members in different modes: nothing selected, plain objects, a
    /// whole compound shape, one member, members sharing a mode, and a whole compound shape with
    /// one of its members.
    #[test]
    fn shape_modes_show_no_question_mark_without_mixed_members() {
        let mut p = Panel::new(two_squares(), show);
        assert_eq!(shape_mode_states(&mut p), [OFF; 4], "two plain objects");
        let [a, b] = compound_of_two(&mut p.app(), 110.0);
        let shape = p.app().session.active().unwrap().selection.objects[0];
        set_mode(&mut p.app(), &[&b], "subtract");
        p.app().run("select.set", json!({"ids": [shape.0]})).unwrap();
        assert_eq!(shape_mode_states(&mut p), [OFF; 4], "the whole compound shape");
        p.app().run("select.set", json!({"ids": [b]})).unwrap();
        let lit = [OFF, LIT, OFF, OFF];
        assert_eq!(shape_mode_states(&mut p), lit, "one member");
        set_mode(&mut p.app(), &[&a, &b], "subtract");
        assert_eq!(shape_mode_states(&mut p), lit, "two members in one mode");
        set_mode(&mut p.app(), &[&a], "add");
        p.app().run("select.set", json!({"ids": [shape.0, b]})).unwrap();
        assert_eq!(shape_mode_states(&mut p), lit, "the shape and one member");
        p.app().run("select.none", json!({})).unwrap();
        assert_eq!(shape_mode_states(&mut p), [OFF; 4], "nothing selected");
    }

    /// An app with a window, so its dialogs open, and a 200 pt document.
    fn windowed() -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app
    }

    fn host(app: &VectorcraftApp) -> craft::ui::shell::WindowDialogs {
        app.dialogs().expect("a window")
    }

    /// The id of the dialog on top of the host's stack.
    fn open_dialog(app: &mut VectorcraftApp) -> egui::Id {
        host(app).with(|h| h.stack().top()).expect("an open dialog")
    }

    /// The values of the open parameter dialog `id`.
    fn dialog_fields(app: &VectorcraftApp, id: egui::Id) -> DialogFields {
        host(app).with(|h| h.get_by_id::<HostedParamDialog>(id).map(|d| d.fields().clone())).expect("an open parameter dialog")
    }

    /// Sets `fields` of the open dialog `id`, presses its OK and lets the app apply the answer; the dialog holds its OK
    /// until the app takes the answer.
    fn confirm_dialog(app: &mut VectorcraftApp, id: egui::Id, fields: &[(&str, Value)]) {
        crate::tests::picks::answer_dialog(app, id, fields, true);
    }

    /// The panel menu's Trap… is enabled for two objects and opens the Trap options; OK adds the
    /// trap above them as one selected group in one undo step.
    #[test]
    fn trap_opens_its_options_and_ok_traps_the_selection() {
        let mut app = windowed();
        let mut rect = |x: f64, color: &str| {
            let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"color": color})).unwrap();
            id
        };
        let (a, b) = (rect(10.0, "#ffff00"), rect(60.0, "#0000ff"));
        app.run("select.set", json!({"ids": [a]})).unwrap();
        assert!(!trap_enabled(&app), "one object has nothing to trap with");
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        assert!(trap_enabled(&app));
        app.run("ui.pathfinder.trap", json!({})).unwrap();
        let id = open_dialog(&mut app);
        let fields = dialog_fields(&app, id);
        for key in ["thickness", "heightWidth", "tintReduction", "trapsWithProcessColor", "reverseTraps"] {
            assert!(fields.contains(key), "{key}");
        }
        let undo = app.session.doc().unwrap().history.undo.len();
        confirm_dialog(&mut app, id, &[("thickness", json!(2))]);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let group = st.doc.node(*st.selection.objects.first().unwrap()).unwrap();
        let trap = &group.children().unwrap()[0];
        let bb = trap.geometric_bounds().unwrap();
        assert!((bb.x0 - 60.0).abs() < 0.02 && (bb.x1 - 62.0).abs() < 0.02, "2 pt into the blue: {bb:?}");
        assert!(trap.appearance.fill().is_some_and(|f| f.overprint));
    }

    /// The panel menu's Pathfinder Options… is enabled and opens the options as they are; OK keeps
    /// them, and the next Divide leaves out the pieces of the object with no fill.
    #[test]
    fn pathfinder_options_open_and_ok_sets_what_the_next_divide_uses() {
        let mut app = windowed();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
        let b = app.run("shape.rectangle", json!({"x": 60, "y": 30, "width": 100, "height": 50})).unwrap()["id"].clone();
        app.run("paint.setFill", json!({"none": true})).unwrap();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        assert!(crate::menus::enabled(&app, "object.pathfinder.options"));
        crate::menus::invoke(&mut app, "object.pathfinder.options", json!({}));
        let id = open_dialog(&mut app);
        let fields = dialog_fields(&app, id);
        assert_eq!(fields.number("precision"), Some(0.01));
        assert_eq!(fields.bool("removeRedundantPoints"), Some(false));
        assert_eq!(fields.bool("removeUnpainted"), Some(false));
        confirm_dialog(&mut app, id, &[("removeUnpainted", json!(true)), ("precision", json!(0.1))]);
        assert!(host(&app).with(|h| h.stack().is_empty()));
        let o = app.session.prefs.pathfinder;
        assert!(o.remove_unpainted && !o.remove_redundant_points && o.precision == 0.1, "{o:?}");
        let id = app.run("object.pathfinder.divide", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let group = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        let pieces = group.children().unwrap();
        assert_eq!(pieces.len(), 1, "only the square's piece outside the unfilled rectangle");
        assert!(pieces.iter().all(|p| !p.appearance.fill_paint().is_none()));
    }

    /// A document with two overlapping 50 pt squares, both selected.
    fn two_squares() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        select_two_squares(&mut app, 10.0);
        app
    }

    /// Draw two overlapping 50 pt squares, the first at (`x`, 10), and select only them.
    fn select_two_squares(app: &mut VectorcraftApp, x: f64) {
        let a = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": x + 30.0, "y": 40, "width": 50, "height": 50})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
    }

    /// The kind and geometric bounds of each object on the first layer.
    fn objects(app: &VectorcraftApp) -> Vec<(&'static str, [f64; 4])> {
        let doc = &app.session.active().unwrap().doc;
        let kind = |n: &vectorcraft_engine::doc::Node| if matches!(n.kind, NodeKind::Path { .. }) { "path" } else { "other" };
        let bounds = |n: &vectorcraft_engine::doc::Node| n.geometric_bounds().map_or([0.0; 4], |b| [b.x0, b.y0, b.x1, b.y1]);
        doc.layers[0].children().unwrap().iter().map(|n| (kind(n), bounds(n))).collect()
    }

    /// Repeat Pathfinder as its key's command runs it.
    fn press_repeat(app: &mut VectorcraftApp) {
        crate::menus::invoke(app, "pathfinder.repeat", json!({}));
    }

    /// Repeat Pathfinder as the panel menu's row fires it.
    fn menu_repeat(p: &Panel) {
        let fired = Fired::new(CommandRef::new("pathfinder.repeat"), FireSource::Menu, FirePhase::Press, 0.0);
        crate::shortcuts::run_fired(&mut p.app(), p.ctx(), &fired);
    }

    /// After Unite from the panel, Repeat Pathfinder unites the two squares selected next, and the
    /// panel menu's Repeat Unite row, which runs the same command, unites a third pair.
    #[test]
    fn cmd_4_repeats_the_operation_a_panel_button_ran_last() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        let first = ("path", [10.0, 10.0, 90.0, 90.0]);
        assert_eq!(objects(&p.app()), [first], "the panel's Unite merged the first pair");
        select_two_squares(&mut p.app(), 110.0);
        assert!(crate::menus::enabled(&p.app(), "pathfinder.repeat"));
        press_repeat(&mut p.app());
        let second = ("path", [110.0, 10.0, 190.0, 90.0]);
        assert_eq!(objects(&p.app()), [first, second], "the key's command merged the second pair");
        select_two_squares(&mut p.app(), 210.0);
        menu_repeat(&p);
        assert_eq!(objects(&p.app()), [first, second, ("path", [210.0, 10.0, 290.0, 90.0])], "the menu row merged the third pair");
    }

    /// Before a repeatable operation has run, Repeat Pathfinder is disabled and leaves the selected
    /// squares as they are, with a message in the status bar. After an operation has run, the
    /// command is disabled again while nothing is selected.
    #[test]
    fn cmd_4_does_nothing_before_an_operation_has_run() {
        let mut p = Panel::new(two_squares(), show);
        let before = objects(&p.app());
        assert_eq!(before.len(), 2);
        assert!(!crate::menus::enabled(&p.app(), "pathfinder.repeat"));
        press_repeat(&mut p.app());
        assert_eq!(objects(&p.app()), before, "Repeat changed nothing");
        assert!(!p.app().ui.status.is_empty(), "the status says why");
        p.click("Unite");
        p.app().run("select.none", json!({})).unwrap();
        assert!(!crate::menus::enabled(&p.app(), "pathfinder.repeat"), "nothing selected");
        assert!(p.app().run("pathfinder.repeat", json!({})).is_err());
    }

    /// One selected path cannot supply the operands for Unite; the command reports the failure.
    #[test]
    fn repeat_with_one_path_keeps_art_and_history() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        let before = objects(&p.app());
        assert_eq!(before.len(), 1);
        assert!(!crate::menus::enabled(&p.app(), "pathfinder.repeat"));
        press_repeat(&mut p.app());
        assert_eq!(objects(&p.app()), before);
        assert!(!p.app().ui.status.is_empty(), "the status says why");
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("unite"));
    }

    /// A direct command replaces the panel's previous operation and supplies the menu name.
    #[test]
    fn direct_operation_becomes_the_one_repeated() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        select_two_squares(&mut p.app(), 110.0);
        p.app().run("object.pathfinder.intersect", json!({})).unwrap();
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("intersect"));
        select_two_squares(&mut p.app(), 210.0);
        menu_repeat(&p);
        assert_eq!(objects(&p.app()).last(), Some(&("path", [240.0, 40.0, 260.0, 60.0])));
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("intersect"));
    }

    /// A successful batch records its last repeatable Pathfinder step.
    #[test]
    fn batch_pathfinder_step_becomes_the_one_repeated() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Intersect");
        select_two_squares(&mut p.app(), 110.0);
        p.app()
            .run(
                "command.batch",
                json!({"commands": [
                    {"command": "object.pathfinder.unite", "params": {}},
                    {"command": "select.none", "params": {}}
                ]}),
            )
            .unwrap();
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut p.app(), 210.0);
        press_repeat(&mut p.app());
        assert_eq!(objects(&p.app()).last(), Some(&("path", [210.0, 10.0, 290.0, 90.0])));
    }

    /// A batch rollback leaves the previous Pathfinder operation available.
    #[test]
    fn failed_batch_pathfinder_step_keeps_history() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        select_two_squares(&mut p.app(), 110.0);
        let before = objects(&p.app());
        assert!(
            p.app()
                .run(
                    "command.batch",
                    json!({"commands": [
                        {"command": "object.pathfinder.intersect", "params": {}},
                        {"command": "unknown.command", "params": {}}
                    ]})
                )
                .is_err()
        );
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("unite"));
        assert_eq!(objects(&p.app()), before);
        press_repeat(&mut p.app());
        assert_eq!(objects(&p.app()).last(), Some(&("path", [110.0, 10.0, 190.0, 90.0])));
    }

    /// Playing an action records its Pathfinder step for the repeat command.
    #[test]
    fn actions_panel_play_records_pathfinder_step() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Intersect");
        let mut action = Action::new("Unite", vec![ActionStep::new("object.pathfinder.unite", json!({}))]);
        action.set = Some("Pathfinder".into());
        let (set, idx) = {
            let mut app = p.app();
            let flat = app.ui.actions.push(action).unwrap();
            crate::panels::actions::model::place(&app.ui.actions, flat).unwrap()
        };
        select_two_squares(&mut p.app(), 110.0);
        crate::panels::actions::playback::play(&mut p.app(), set, idx).unwrap();
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut p.app(), 210.0);
        press_repeat(&mut p.app());
        assert_eq!(objects(&p.app()).last(), Some(&("path", [210.0, 10.0, 290.0, 90.0])));
    }

    /// A rejected operation leaves the prior successful operation available to repeat.
    #[test]
    fn failed_operation_keeps_history() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        assert!(p.app().run("object.pathfinder.intersect", json!({"precision": -1.0})).is_err());
        assert_eq!(p.app().last_pathfinder.as_deref(), Some("unite"));
        select_two_squares(&mut p.app(), 110.0);
        menu_repeat(&p);
        assert_eq!(objects(&p.app()).last(), Some(&("path", [110.0, 10.0, 190.0, 90.0])));
    }

    /// A successful Trap keeps the last repeatable operation in history.
    #[test]
    fn successful_trap_keeps_history() {
        let mut p = Panel::new(two_squares(), show);
        p.click("Unite");
        let mut app = p.app();
        let mut rect = |x: f64, color: &str| {
            let id = app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap()["id"].clone();
            app.run("paint.setFill", json!({"color": color})).unwrap();
            id
        };
        let (a, b) = (rect(110.0, "#ffff00"), rect(160.0, "#0000ff"));
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("object.pathfinder.trap", json!({"thickness": 2, "tintReduction": 30})).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("unite"));
    }

    /// Repeat reads the current options instead of replaying the earlier command's settings.
    #[test]
    fn repeat_uses_current_pathfinder_options() {
        let mut app = two_squares();
        app.run("object.pathfinder.divide", json!({})).unwrap();
        assert_eq!(app.last_pathfinder.as_deref(), Some("divide"));
        app.run("object.pathfinder.options", json!({"precision": 1.0})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 0, "y": 99.5, "width": 100, "height": 100})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let id = app.run("pathfinder.repeat", json!({})).unwrap()["ids"][0].as_u64().unwrap();
        let group = app.session.active().unwrap().doc.node(NodeId(id)).unwrap();
        assert_eq!(group.children().unwrap().len(), 2);
        assert_eq!(app.last_pathfinder.as_deref(), Some("divide"));
    }

    /// Compound-shape creation and member edits leave destructive Pathfinder history alone.
    #[test]
    fn compound_shape_commands_do_not_set_history() {
        let mut p = Panel::new(two_squares(), show);
        let a = p.app().session.active().unwrap().selection.objects[0];
        p.click_with("Minus Front", Modifiers::ALT);
        assert_eq!(p.app().last_pathfinder, None);
        p.app().run("select.set", json!({"ids": [a.0]})).unwrap();
        p.click("Unite");
        assert_eq!(p.app().last_pathfinder, None);
        assert!(!crate::menus::enabled(&p.app(), "pathfinder.repeat"));
    }

    /// How many compound shapes the active document holds.
    fn compound_shapes(app: &VectorcraftApp) -> usize {
        let mut n = 0;
        app.session.active().unwrap().doc.walk(|node| n += usize::from(node.is_compound_shape()));
        n
    }

    /// The panel over two selected squares, with the Compound Shapes add-on's setting `on` or off.
    fn panel_with_setting(on: bool) -> Panel {
        let mut app = two_squares();
        if on {
            app.run("window.pathfinder.compoundShapes", json!({})).unwrap();
        }
        Panel::new(app, show)
    }

    /// Off (the default) a Shape Mode click is the permanent Pathfinder result and Alt-click makes a
    /// compound shape; with the Compound Shapes add-on's setting on it is the other way round.
    #[test]
    fn the_compound_shapes_setting_swaps_click_and_alt_on_the_shape_modes() {
        // Off: unchanged.
        let mut p = panel_with_setting(false);
        assert!(!p.app().ui.shape_modes_compound);
        p.click("Unite");
        assert_eq!(
            {
                let app = p.app();
                (objects(&app).len(), compound_shapes(&app))
            },
            (1, 0),
            "a click is permanent"
        );
        let mut p = panel_with_setting(false);
        p.click_with("Unite", Modifiers::ALT);
        assert_eq!(compound_shapes(&p.app()), 1, "Alt makes a compound shape");

        // On: a click makes the compound shape, Alt the permanent result.
        let mut p = panel_with_setting(true);
        assert!(p.app().ui.shape_modes_compound);
        assert_eq!(crate::menus::checked(&p.app(), "window.pathfinder.compoundShapes", &Value::Null), Some(true));
        p.click("Unite");
        assert_eq!(compound_shapes(&p.app()), 1, "a click makes a compound shape");
        let mut p = panel_with_setting(true);
        p.click_with("Unite", Modifiers::ALT);
        assert_eq!(
            {
                let app = p.app();
                (objects(&app).len(), compound_shapes(&app))
            },
            (1, 0),
            "Alt is permanent"
        );

        // Toggled off again, and the check mark follows.
        p.app().run("window.pathfinder.compoundShapes", json!({})).unwrap();
        assert!(!p.app().ui.shape_modes_compound);
        assert_eq!(crate::menus::checked(&p.app(), "window.pathfinder.compoundShapes", &Value::Null), Some(false));
    }

    /// Members of a compound shape that are selected take the clicked mode whatever the setting says.
    #[test]
    fn selected_members_take_the_mode_with_the_setting_on() {
        let mut p = panel_with_setting(true);
        p.click("Unite");
        let shape = p.app().session.active().unwrap().selection.objects[0];
        let front = p.app().session.active().unwrap().doc.node(shape).unwrap().children().unwrap().last().unwrap().id;
        p.app().run("select.set", json!({"ids": [front.0]})).unwrap();
        p.click("Minus Front");
        assert_eq!(compound_shapes(&p.app()), 1, "no second compound shape");
        assert_eq!(vectorcraft_engine::cmd::selected_member_modes(&p.app().session), [ShapeMode::Subtract]);
    }

    /// The setting is kept with the preferences; preferences saved before it read as off.
    #[test]
    fn the_compound_shapes_setting_is_saved_and_older_preferences_read_as_off() {
        let mut app = two_squares();
        app.run("window.pathfinder.compoundShapes", json!({})).unwrap();
        let mut saved = serde_json::to_value(&app.ui).unwrap();
        assert_eq!(saved["shape_modes_compound"], json!(true));
        assert!(serde_json::from_value::<crate::state::UiState>(saved.clone()).unwrap().shape_modes_compound);
        saved.as_object_mut().unwrap().remove("shape_modes_compound");
        assert!(!serde_json::from_value::<crate::state::UiState>(saved).unwrap().shape_modes_compound);
    }
}
