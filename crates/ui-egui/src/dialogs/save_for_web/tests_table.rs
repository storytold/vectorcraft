//! The Save for Web colour table model: the edits over the engine's `colorTable` value.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::webexport;

use craft::ui::patterns::form_dialogs::{ColorTableEdit, TableOrder};

use super::color_table::{apply, marks, restore, shift};
use super::tests::{app, frame, host, id, save, set};
use crate::VectorcraftApp;

const A: [u8; 3] = [0x00, 0x00, 0x00];
const B: [u8; 3] = [0x11, 0x11, 0x11];
const C: [u8; 3] = [0x22, 0x22, 0x22];
const D: [u8; 3] = [0x33, 0x33, 0x33];

fn run(table: &mut Value, edit: ColorTableEdit, colors: &[[u8; 3]], shown: &[[u8; 3]]) -> Result<(), String> {
    apply(table, &edit, colors, shown)
}

#[test]
fn every_table_edit_changes_the_lists_it_names() {
    let shown = [A, B, C, D];
    let mut t = json!({});
    // Lock and Transparent: on when any selected colour is not in the list, off when all are.
    run(&mut t, ColorTableEdit::Lock, &[B], &shown).unwrap();
    run(&mut t, ColorTableEdit::Lock, &[B, C], &shown).unwrap();
    assert_eq!(t["locked"], json!(["#111111", "#222222"]));
    run(&mut t, ColorTableEdit::Lock, &[B, C], &shown).unwrap();
    assert_eq!(t["locked"], json!([]));
    run(&mut t, ColorTableEdit::Transparent, &[D], &shown).unwrap();
    assert_eq!(t["transparent"], json!(["#333333"]));
    // A hand-shifted colour stays locked: Lock does nothing to it.
    shift(&mut t, B, [255, 136, 0]).unwrap();
    run(&mut t, ColorTableEdit::Lock, &[B], &shown).unwrap();
    assert_eq!(t["locked"], json!([]));
    // Web Shift takes the hand shift away; Unshift takes colours out of webShift only.
    run(&mut t, ColorTableEdit::WebShift, &[B, C], &shown).unwrap();
    assert_eq!((&t["webShift"], &t["shifted"]), (&json!(["#111111", "#222222"]), &json!({})));
    run(&mut t, ColorTableEdit::Unshift, &[B], &shown).unwrap();
    assert_eq!(t["webShift"], json!(["#222222"]));
    // A shift takes the colour out of webShift; one to the source colour takes it away.
    shift(&mut t, C, [0, 255, 0]).unwrap();
    assert_eq!((&t["webShift"], &t["shifted"]), (&json!([]), &json!({"#222222": "#00ff00"})));
    shift(&mut t, C, C).unwrap();
    assert_eq!(t["shifted"], json!({}));
    // The Shift edit is a shift of the first selected colour.
    run(&mut t, ColorTableEdit::Shift([1, 2, 3]), &[A], &shown).unwrap();
    assert_eq!(t["shifted"], json!({"#000000": "#010203"}));
    // Unshift All and Sort act on the whole table, whatever is selected.
    run(&mut t, ColorTableEdit::WebShift, &[D], &shown).unwrap();
    run(&mut t, ColorTableEdit::UnshiftAll, &[], &shown).unwrap();
    assert_eq!((&t["shifted"], &t["webShift"]), (&json!({}), &json!([])));
    for (order, id) in
        [(TableOrder::Hue, "hue"), (TableOrder::Luminance, "luminance"), (TableOrder::Popularity, "popularity"), (TableOrder::Unsorted, "none")]
    {
        run(&mut t, ColorTableEdit::Sort(order), &[], &shown).unwrap();
        assert_eq!(t["sort"], json!(id));
    }
    // The marks read the same lists, and skip entries that are not colours.
    let mut t =
        json!({"locked": ["#000000", "nope", 7], "transparent": ["#ABCDEF"], "shifted": {"#111111": "#222222", "x": "#000000", "#333333": 5}});
    let m = marks(&t);
    assert_eq!((m.locked, m.transparent, m.shifted), (vec![A], vec![[0xab, 0xcd, 0xef]], vec![(B, C)]));
    t = json!(7);
    assert_eq!(marks(&t), Default::default());
}

#[test]
fn delete_color_fixes_the_palette_and_always_keeps_a_color() {
    let shown = [A, B, C, D];
    let mut t = json!({"locked": ["#222222", "#000000"], "webShift": ["#333333"], "shifted": {"#222222": "#ffffff"}});
    run(&mut t, ColorTableEdit::Delete, &[C, D], &shown).unwrap();
    assert_eq!(
        t,
        json!({
            "locked": ["#000000"], "webShift": [], "shifted": {},
            "customPalette": ["#000000", "#111111", "#222222", "#333333"],
            "deleted": ["#222222", "#333333"],
        })
    );
    // No selection: nothing changes.
    let before = t.clone();
    run(&mut t, ColorTableEdit::Delete, &[], &shown).unwrap();
    assert_eq!(t, before);
    // The colours left can't all go.
    assert_eq!(run(&mut t, ColorTableEdit::Delete, &[A, B], &shown), Err(webexport::KEEP_ONE.to_string()));
    assert_eq!(t, before);
    // Nor can the last colour that is not mapped to transparent.
    run(&mut t, ColorTableEdit::Transparent, &[B], &shown).unwrap();
    let before = t.clone();
    assert_eq!(run(&mut t, ColorTableEdit::Delete, &[A], &shown), Err(webexport::KEEP_ONE.to_string()));
    assert_eq!(t, before);
}

#[test]
fn an_edit_skips_selected_colors_the_table_does_not_show() {
    let shown = [A, B];
    let gone = [0xab, 0xcd, 0xef];
    for edit in [ColorTableEdit::Lock, ColorTableEdit::Transparent, ColorTableEdit::WebShift, ColorTableEdit::Unshift, ColorTableEdit::Delete] {
        let mut t = json!({});
        run(&mut t, edit.clone(), &[gone], &shown).unwrap();
        assert_eq!(t, json!({}), "{edit:?}");
    }
    let mut t = json!({});
    run(&mut t, ColorTableEdit::Shift([1, 2, 3]), &[gone], &shown).unwrap();
    assert_eq!(t, json!({}));
    // A selection partly shown edits only the shown part.
    run(&mut t, ColorTableEdit::Delete, &[gone, B], &shown).unwrap();
    assert_eq!((&t["customPalette"], &t["deleted"]), (&json!(["#000000", "#111111"]), &json!(["#111111"])));
}

/// With 256 colours shifted, shifting another colour is refused with the engine's limit error and
/// changes nothing; a shifted colour can still be shifted again or have its shift taken away.
#[test]
fn a_shift_holds_at_most_256_colors() {
    let full: serde_json::Map<String, Value> = (0..256).map(|i| (format!("#{i:06x}"), json!("#ffffff"))).collect();
    let mut t = json!({"shifted": full});
    let before = t.clone();
    let e = shift(&mut t, [0x12, 0x34, 0x56], A).unwrap_err();
    assert_eq!((e, &t), (webexport::too_many("shifted"), &before));
    // The Shift edit reports the same error.
    let new = [0x12, 0x34, 0x56];
    assert_eq!(run(&mut t, ColorTableEdit::Shift(A), &[new], &[new]), Err(webexport::too_many("shifted")));
    assert_eq!(t, before);
    shift(&mut t, [0, 0, 5], A).unwrap();
    assert_eq!((t["shifted"]["#000005"].as_str(), t["shifted"].as_object().map(|m| m.len())), (Some("#000000"), Some(256)));
    shift(&mut t, [0, 0, 5], [0, 0, 5]).unwrap();
    shift(&mut t, new, A).unwrap();
    assert_eq!(t["shifted"]["#123456"], "#000000");
}

/// Entries the engine drops (a colour shifted to itself, a deleted colour) do not count toward the
/// 256 shifts, and a new shift writes the map without them. A map the engine refuses is kept whole.
#[test]
fn a_shift_skips_entries_the_engine_drops() {
    let mut full: serde_json::Map<String, Value> = (0..255).map(|i| (format!("#{i:06x}"), json!(format!("#{i:06x}")))).collect();
    full.insert("#abcdef".into(), json!("#000000"));
    let mut t = json!({"shifted": full, "deleted": ["#ABCDEF"]});
    shift(&mut t, [0x12, 0x34, 0x56], A).unwrap();
    assert_eq!(t["shifted"], json!({"#123456": "#000000"}));
    for shifted in [json!({"nope": "NOPE"}), json!({"#abcdef": "nope"}), json!({"#f00": "#f00", "#FF0000": "#00ff00"})] {
        let mut t = json!({"shifted": shifted.clone(), "deleted": ["#abcdef"]});
        shift(&mut t, [0x12, 0x34, 0x56], A).unwrap();
        let mut want = shifted.as_object().cloned().unwrap_or_default();
        want.insert("#123456".into(), json!("#000000"));
        assert_eq!(t["shifted"], Value::Object(want), "{shifted}");
    }
}

/// A map with more than 256 entries is one the engine refuses, so none of its entries are dropped:
/// with 257 colours shifted to themselves, a new shift and a new target for a shifted colour are
/// refused with the limit error and nothing changes. Taking a shift away still works.
#[test]
fn a_shift_keeps_a_map_over_the_limit_whole() {
    let over: serde_json::Map<String, Value> = (0..257).map(|i| (format!("#{i:06x}"), json!(format!("#{i:06x}")))).collect();
    let mut t = json!({"shifted": over});
    let before = t.clone();
    let e = shift(&mut t, [0x12, 0x34, 0x56], A).unwrap_err();
    assert_eq!((e, &t), (webexport::too_many("shifted"), &before));
    let e = shift(&mut t, [0, 0, 5], [0x12, 0x34, 0x56]).unwrap_err();
    assert_eq!((e, &t), (webexport::too_many("shifted"), &before));
    shift(&mut t, [0, 0, 5], [0, 0, 5]).unwrap();
    assert_eq!(t["shifted"].as_object().map(|m| m.len()), Some(256));
}

/// A key or list entry written another way (`#F00`, upper case, `blue`) still names its colour:
/// a new shift replaces it, Unshift and the buttons take it out, the marks show it, and a
/// spelling with alpha names no colour.
#[test]
fn table_edits_match_a_color_written_another_way() {
    let (red, blue) = ([0xff, 0, 0], [0, 0, 0xff]);
    let shown = [red, blue];
    let mut t = json!({"shifted": {"#F00": "#00ff00", "nope": "#000000"}, "webShift": ["#0000FF"]});
    shift(&mut t, red, [0x12, 0x34, 0x56]).unwrap();
    assert_eq!(t["shifted"], json!({"#ff0000": "#123456", "nope": "#000000"}));
    shift(&mut t, blue, [0x65, 0x43, 0x21]).unwrap();
    assert_eq!(t["webShift"], json!([]));
    // Lock leaves a colour shifted under another spelling alone.
    let mut t = json!({"shifted": {"#F00": "#00ff00"}});
    run(&mut t, ColorTableEdit::Lock, &[red], &shown).unwrap();
    assert_eq!(t.get("locked"), None);
    // Each list's button takes out an entry written another way, as Unlock, Restore and Unshift do.
    for (list, edit) in [("locked", ColorTableEdit::Lock), ("transparent", ColorTableEdit::Transparent), ("webShift", ColorTableEdit::Unshift)] {
        let mut t = json!({ list: ["#00F", " #0000FF "] });
        let m = marks(&t);
        if list != "webShift" {
            assert_eq!(if list == "locked" { m.locked } else { m.transparent }, vec![blue], "{list}");
        }
        run(&mut t, edit, &[blue], &shown).unwrap();
        assert_eq!(t[list], json!([]), "{list}");
    }
    // Web Shift of a colour in the list under another spelling takes it out.
    let mut t = json!({"webShift": ["#00f"]});
    run(&mut t, ColorTableEdit::WebShift, &[blue], &shown).unwrap();
    assert_eq!(t["webShift"], json!([]));
    // A spelling with alpha names no colour.
    let mut t = json!({"locked": ["#0000ff80"]});
    assert_eq!(marks(&t).locked, Vec::<[u8; 3]>::new());
    run(&mut t, ColorTableEdit::Lock, &[blue], &shown).unwrap();
    assert_eq!(t["locked"], json!(["#0000ff80", "#0000ff"]));
}

/// Delete Color compares the lists as colours: with blue mapped to transparent as `#00f`, deleting
/// red, the only other colour, is refused with nothing changed, and a deleted colour written
/// another way in `locked` (`blue`, ` #0000FF `) is taken out of the list.
#[test]
fn delete_color_compares_lists_written_another_way() {
    let (red, blue) = ([0xff, 0, 0], [0, 0, 0xff]);
    let shown = [blue, red];
    let mut t = json!({"transparent": ["#00f"]});
    let before = t.clone();
    assert_eq!(run(&mut t, ColorTableEdit::Delete, &[red], &shown), Err(webexport::KEEP_ONE.to_string()));
    assert_eq!(t, before);
    let mut t = json!({"locked": ["blue", " #0000FF ", "#ff0000"], "deleted": ["#00F"]});
    run(&mut t, ColorTableEdit::Delete, &[blue], &shown).unwrap();
    assert_eq!((&t["locked"], &t["deleted"]), (&json!(["#ff0000"]), &json!(["#00f"])));
}

/// One app frame with `events`, a moment after the last.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) {
    let time = ctx.input(|i| i.time) + 0.05;
    crate::testing::app_frame(app, ctx, egui::RawInput { time: Some(time), focused: true, events, ..Default::default() });
}

/// Double-clicks the Color Table's cell of `hex` in drawn frames, as a user opens it, and gives the Color Picker the
/// app opens over the dialog for it.
fn open_cell(app: &mut VectorcraftApp, ctx: &egui::Context, hex: &str) -> craft::ui::dialog::DialogId {
    let parent = id(app);
    // The cells are named by their index and colour.
    let find = |app: &mut VectorcraftApp, name: &str| {
        let (find, _) = craft::automation::ControlRequest::new("ui.find", json!({"name": name}));
        match crate::control::handle(app, ctx, &find) {
            craft::automation::Reply::Done(r) if r["ok"] == true => {
                r["result"]["rect"].as_array().map(|r| r.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
            }
            _ => None,
        }
    };
    // The dialog centres itself again as its panes and table fill in: the cell is clicked once it stays put.
    let mut rect = None;
    for _ in 0..20 {
        app_frame(app, ctx, vec![]);
        let now = (0..256).find_map(|i| find(app, &format!("{i} {hex}")));
        if now.is_some() && now == rect {
            break;
        }
        rect = now;
    }
    let rect = rect.unwrap_or_else(|| panic!("the table shows {hex}"));
    let at = egui::pos2((rect[0] + rect[2] / 2.0) as f32, (rect[1] + rect[3] / 2.0) as f32);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    for events in [
        vec![egui::Event::PointerMoved(at)],
        vec![button(true)],
        vec![button(false)],
        vec![button(true)],
        vec![button(false)],
        vec![],
        vec![],
        vec![],
    ] {
        app_frame(app, ctx, events);
    }
    let top = host(app).with(|h| h.open_dialogs().pop()).expect("a dialog is open");
    assert_eq!(top.parent, Some(parent), "the Color Picker opens over the dialog: {top:?}");
    top.id
}

/// Opening a colour of the table asks the Color Picker for it: its OK shifts the colour, its Cancel shifts nothing.
#[test]
fn a_picked_color_becomes_a_shift_and_cancel_shifts_nothing() {
    let dir = vectorcraft_testkit::temp_dir("sfw-shift-color");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    crate::dialogs::host::pump(&mut app, &ctx);
    let picker = open_cell(&mut app, &ctx, "#0000ff");
    host(&app).with(|h| h.cancel_by_id(&ctx, picker));
    for _ in 0..3 {
        app_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.web_export.table.get("shifted").cloned().unwrap_or(json!({})), json!({}));
    let picker = open_cell(&mut app, &ctx, "#0000ff");
    host(&app).with(|h| h.set_json_field(&ctx, picker, "color", json!("#00ff00"))).unwrap();
    host(&app).with(|h| h.confirm_by_id(&ctx, picker));
    for _ in 0..3 {
        app_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.web_export.table["shifted"], json!({"#0000ff": "#00ff00"}));
}

/// Done right after the Color Picker's OK, before the dialog draws again, remembers the picked shift.
#[test]
fn done_right_after_the_color_picker_remembers_the_shift() {
    let dir = vectorcraft_testkit::temp_dir("sfw-shift-done");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    let ctx = app.ui_ctx.clone().expect("the app has a window");
    crate::dialogs::host::pump(&mut app, &ctx);
    let picker = open_cell(&mut app, &ctx, "#0000ff");
    host(&app).with(|h| h.set_json_field(&ctx, picker, "color", json!("#00ff00"))).unwrap();
    host(&app).with(|h| h.confirm_by_id(&ctx, picker));
    // The frame that settles the picker, then Done before the dialog's next pass.
    app_frame(&mut app, &ctx, vec![]);
    set(&mut app, "choice", json!("done"));
    let dialog = id(&app);
    host(&app).with(|h| h.confirm_by_id(&ctx, dialog));
    for _ in 0..3 {
        app_frame(&mut app, &ctx, vec![]);
    }
    assert!(host(&app).with(|h| h.get::<craft::ui::patterns::form_dialogs::HostedSaveForWeb>().is_none()), "Done closes the dialog");
    let remembered = app.session.last_web_settings().color_table.shifted;
    assert_eq!(remembered.get("#0000ff").map(String::as_str), Some("#00ff00"));
}

#[test]
fn restore_brings_back_a_preview_that_failed() {
    let dir = vectorcraft_testkit::temp_dir("sfw-restore-table");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    let mut table = json!({"deleted": ["#cc3300"], "customPalette": ["#cc3300", "#0000ff"], "transparent": ["#0000ff"]});
    let params = |t: &Value| json!({"format": "gif", "antiAlias": "none", "colorTable": t});
    assert!(app.session.execute("document.exportForWeb.preview", &params(&table)).is_err(), "the preview fails");
    restore(&mut table);
    assert_eq!((&table["deleted"], &table["customPalette"]), (&json!([]), &json!([])));
    assert!(app.session.execute("document.exportForWeb.preview", &params(&table)).is_ok(), "the preview is back");
}

/// The saved file lacks the deleted colour, whose pixels take the colour left; the edit is
/// remembered and reopens with the dialog.
#[test]
fn delete_color_reaches_the_saved_file_and_is_remembered() {
    let dir = vectorcraft_testkit::temp_dir("sfw-delete-color");
    let (mut app, _) = app(&dir, false);
    app.run("shape.rectangle", json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    app.run("file.saveForWeb", json!({})).unwrap();
    for (k, v) in [("format", json!("gif")), ("antiAlias", json!("none")), ("dither", json!("none"))] {
        set(&mut app, k, v);
    }
    frame(&mut app);
    let red = [0xcc, 0x33, 0x00];
    apply(&mut app.web_export.table, &ColorTableEdit::Delete, &[red], &[red, [0, 0, 255]]).unwrap();
    frame(&mut app);
    save(&mut app);
    let path =
        std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).find(|p| p.extension().is_some_and(|e| e == "gif")).expect("the GIF is saved");
    let image = image::open(path).unwrap().to_rgba8();
    assert_eq!((image.get_pixel(20, 14).0, image.get_pixel(50, 20).0), ([0, 0, 255, 255], [0, 0, 255, 255]));
    assert_eq!(app.session.last_web_settings().color_table.deleted, ["#cc3300"]);
    app.run("file.saveForWeb", json!({})).unwrap();
    frame(&mut app);
    assert_eq!(app.web_export.table["deleted"], json!(["#cc3300"]));
}
