//! Objects on Path Tool Options (double-click the Objects on Path tool): the Pivot and Rotate of
//! the objects the tool attaches next. Groups attached before keep theirs. Save sets them with
//! `tool.setOption {tool: "objectsOnPath", values}`; they last across tool switches and are saved
//! with the preferences. Reset puts the center and 0 back.
//!
//! Fields: `pivot` (0-8, row by row from the top left; 4, the center) and `rotate` (degrees
//! counter-clockwise from upright on the path).
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Outcome as _, Show as _, clicked};
use craft::ui::patterns::form_dialogs::{HostedParamDialog, Param, ParamDialog, ParamResult};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_doc::objects_on_path::CENTER;

use super::{form, host, tool_options};
use crate::VectorcraftApp;

/// The dialog kind of Objects on Path Tool Options.
pub const KIND: &str = "objectsOnPathOptions";

/// The tool whose options the dialog edits.
const TOOL: &str = "objectsOnPath";

/// The rows: Pivot, Rotate, and Reset, which puts the center and 0 back in the fields.
fn params() -> Vec<Param> {
    vec![
        tool_options::reference_param("pivot", tl!("Pivot:")),
        Param::number("rotate", tl!("Rotate:"), 0.0, 1.0, 1).unit(UnitKind::Degrees).bounds(Some(-360.0), Some(360.0)),
        Param::custom("reset", "", |ui, fields, _| {
            let out = Button::new(tl!("Reset")).show(ui);
            if clicked(out.response()) {
                fields.set("pivot", i64::from(CENTER));
                fields.set("rotate", 0.0);
            }
            out.response().clone()
        }),
    ]
}

/// Open the tool's options with their current values.
pub fn open(app: &mut VectorcraftApp) -> Value {
    let params = params();
    let options = app.session.tool_options_of(TOOL);
    let fields = options.as_object().map(|m| form::fields_of(&params, m)).unwrap_or_default();
    let (title, shown) = (tool_options::title(TOOL), params.clone());
    let ok = move |app: &mut VectorcraftApp, result: &ParamResult| tool_options::set_options(app, TOOL, form::json_of(&params, &result.fields));
    let opened = app.ui_ctx.clone().and_then(|ctx| {
        let dialog = HostedParamDialog::new(&ctx, KIND, fields, move |f| {
            ParamDialog::new(KIND, title.clone(), shown.clone()).ok_label(tl!("Save").into()).values(Bind::bound(f))
        });
        host::open_held(app, dialog, ok)
    });
    if opened.is_none() {
        app.status(tl!("No window to show the dialog in.").to_string());
    }
    json!({ "dialog": KIND })
}

/// The compact tool options expose translated Pivot corners and accept actual edits.
#[cfg(test)]
#[test]
fn compact_objects_on_path_options_pivot_rotate_reset_and_save() {
    use egui::accesskit::{Node, Role, TreeUpdate};
    use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
    use vectorcraft_engine::Session;

    fn bounds(node: &Node) -> Option<Rect> {
        let b = node.bounds()?;
        Some(Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))
    }

    fn plain_label(label: &str) -> &str {
        label.trim_end().trim_end_matches([':', '：']).trim_end()
    }

    const PIVOT_NAMES: [&str; 9] = ["Top Left", "Top", "Top Right", "Left", "Center", "Right", "Bottom Left", "Bottom", "Bottom Right"];
    let screen = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
    let mut untranslated = Vec::new();
    for lang in ["en", "de", "fr", "ja"] {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.session.prefs.interface_language = lang.into();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("tool.setOption", json!({"tool": TOOL, "values": {"pivot": 1,"rotate": 17}})).unwrap();
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
                frame(app, vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed, repeat: false, modifiers }]);
            }
        };
        let draft = |app: &VectorcraftApp| {
            app.dialogs()
                .unwrap()
                .with(|host| host.get::<HostedParamDialog>().map(|d| (d.fields().int("pivot").unwrap(), d.fields().number("rotate").unwrap())))
        };
        let actual = |app: &VectorcraftApp| {
            let options = app.session.tool_options_of(TOOL);
            (options["pivot"].as_i64().unwrap(), options["rotate"].as_f64().unwrap())
        };
        settle(&mut app, &mut frame);
        for save in [false, true] {
            assert_eq!(app.run("tool.options", json!({"tool": TOOL})).unwrap()["dialog"], KIND);
            let tree = settle(&mut app, &mut frame);
            let by_name = |role, label: &str| {
                tree.nodes
                    .iter()
                    .find(|(_, n)| {
                        n.role() == role
                            && n.label().is_some_and(|actual| {
                                plain_label(actual) == plain_label(label)
                                    || (role == Role::RadioGroup && plain_label(actual) == plain_label(crate::i18n::t("Pivot:")))
                            })
                    })
                    .and_then(|(_, n)| bounds(n))
                    .unwrap_or_else(|| {
                        let names: Vec<_> = tree.nodes.iter().filter(|(_, n)| n.role() == role).filter_map(|(_, n)| n.label()).collect();
                        panic!("{lang}: missing {role:?} {label:?} in {names:?}")
                    })
            };
            let title = tool_options::title(TOOL);
            let modal = by_name(Role::Dialog, title.as_str());
            let picker = by_name(Role::RadioGroup, crate::i18n::t("Reference Point"));
            let rotate = by_name(Role::SpinButton, crate::i18n::t("Rotate:"));
            let reset = by_name(Role::Button, crate::i18n::t("Reset"));
            let footer = by_name(Role::Button, crate::i18n::t(if save { "Save" } else { "Cancel" }));
            assert!(screen.expand(0.5).contains_rect(modal), "{lang}: modal {modal:?} off screen");
            for rect in [picker, rotate, reset, footer] {
                assert!(rect.is_positive() && modal.expand(0.5).contains_rect(rect), "{lang}: {rect:?} outside modal {modal:?}");
            }
            let mut cells: Vec<_> = tree
                .nodes
                .iter()
                .filter(|(_, n)| n.role() == Role::RadioButton)
                .filter_map(|(_, n)| Some((n.label()?, bounds(n)?)))
                .filter(|(_, rect)| picker.expand(0.5).contains(rect.center()))
                .collect();
            cells.sort_by(|(_, a), (_, b)| a.center().y.total_cmp(&b.center().y).then(a.center().x.total_cmp(&b.center().x)));
            eprintln!(
                "{lang}, save {save}: modal {modal:?}, pivot {picker:?}, cells {cells:?}, Rotate {rotate:?}, Reset {reset:?}, footer {footer:?}"
            );
            assert_eq!(cells.len(), 9, "{lang}: all Pivot options");
            if save {
                for ((name, _), legacy) in cells.iter().zip(PIVOT_NAMES) {
                    let expected = crate::i18n::t(legacy);
                    if expected != legacy && *name != expected {
                        untranslated.push(format!("{lang}: {name:?} instead of {expected:?}"));
                    }
                }
            }
            assert!(cells.iter().all(|(_, cell)| picker.expand(0.5).contains_rect(*cell)), "{lang}: all cells lie in the picker");
            assert_eq!(draft(&app), Some((1, 17.0)), "{lang}: opens from saved tool preferences");
            click(&mut app, &mut frame, cells[8].1.center());
            assert_eq!(draft(&app), Some((8, 17.0)), "{lang}: bottom-right physical pointer");
            click(&mut app, &mut frame, rotate.center());
            press(&mut app, &mut frame, Key::A, Modifiers::COMMAND);
            frame(&mut app, vec![Event::ModifiersChanged(Modifiers::NONE), Event::Text("42".to_owned())]);
            press(&mut app, &mut frame, Key::Tab, Modifiers::NONE);
            settle(&mut app, &mut frame);
            assert_eq!(draft(&app), Some((8, 42.0)), "{lang}: real Rotate keyboard edit");
            if !save {
                click(&mut app, &mut frame, reset.center());
                assert_eq!(draft(&app), Some((4, 0.0)), "{lang}: pointer Reset keeps the dialog open and resets both settings");
            }
            assert_eq!(actual(&app), (1, 17.0), "{lang}: only a draft before accepting");
            click(&mut app, &mut frame, footer.center());
            settle(&mut app, &mut frame);
            assert_eq!(draft(&app), None, "{lang}: footer closes the dialog");
            assert_eq!(actual(&app), if save { (8, 42.0) } else { (1, 17.0) }, "{lang}: footer persistence");
        }
        app.run("tool.options", json!({"tool": TOOL})).unwrap();
        settle(&mut app, &mut frame);
        assert_eq!(draft(&app), Some((8, 42.0)), "{lang}: reopened saved values");
        press(&mut app, &mut frame, Key::Escape, Modifiers::NONE);
        settle(&mut app, &mut frame);
        assert_eq!(draft(&app), None, "{lang}: Escape dismisses reopened options");
        crate::i18n::set_current(crate::i18n::Lang::EN);
    }
    assert!(untranslated.is_empty(), "localized Pivot names were lost: {untranslated:?}");
}
