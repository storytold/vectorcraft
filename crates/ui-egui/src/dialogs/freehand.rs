//! Tool Options of the freehand tools (double-click Pencil, Paintbrush, Smooth, Blob Brush or
//! Eraser): the options each tool keeps, which last across tool switches and are saved with the
//! preferences. OK sets them with `tool.setOption {tool, values}`.
//!
//! Fields: `tool`; `fidelity` (points; Pencil, Paintbrush and Smooth), `fill` (Pencil and
//! Paintbrush), `keepSelected` (Pencil, Paintbrush and Blob Brush), `mergeOnlySelection` (Blob
//! Brush), `closeWithin` and `editWithin` (screen pixels, 0 turns it off; Pencil and Paintbrush) and
//! `size` (points; Blob Brush and Eraser).

use serde_json::{Map, Value, json};

use super::document_setup::check;
use super::{DialogSpec, form, run_and_close};
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the freehand tools' Tool Options.
pub const KIND: &str = "freehandOptions";

pub(super) const SPEC: DialogSpec = DialogSpec { heading, body, confirm, min_width: 420.0, ..DialogSpec::FORM };

/// The tools the dialog serves.
pub const TOOLS: [&str; 5] = ["pencil", "paintbrush", "smooth", "blobBrush", "eraser"];

/// The options the dialog edits, in the order it shows them (a tool has those it reports).
const KEYS: [&str; 8] = ["fidelity", "size", "fill", "altSmooth", "keepSelected", "mergeOnlySelection", "closeWithin", "editWithin"];

/// Width of the label column: wide enough for the tolerances' labels when there are any.
fn label_w(d: &Dialog) -> f32 {
    if d.fields.contains_key("closeWithin") { 230.0 } else { 90.0 }
}

/// The range of the Fidelity slider (points); the field takes what the tool allows.
const FIDELITY: (f64, f64) = (0.5, 20.0);

fn heading(d: &Dialog) -> String {
    let label = vectorcraft_tools::tool_info(&d.str("tool")).map_or("", |t| t.label);
    crate::i18n::fmt(tl!("{tool} Options"), &[("tool", tl!(label))])
}

/// Open the options of freehand tool `tool` with their current values.
pub fn open(app: &mut VectorcraftApp, tool: &str) -> Result<Value, String> {
    if !TOOLS.contains(&tool) {
        return Err(format!("`{tool}` isn't a freehand tool"));
    }
    let o = app.session.tool_options_of(tool);
    let mut fields: Map<String, Value> = KEYS.iter().filter_map(|key| Some((key.to_string(), o.get(*key)?.clone()))).collect();
    fields.insert("tool".into(), json!(tool));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({ "dialog": KIND }))
}

/// A row with a number field for `d.fields[key]` (`suffix` is its unit), kept within `range`.
fn number(ui: &mut egui::Ui, d: &mut Dialog, (key, label): (&str, &str), suffix: &str, decimals: usize, range: (f64, f64)) {
    let label_w = label_w(d);
    let v = d.f64(key, 0.0);
    widgets::label_row(ui, label, label_w, |ui| {
        if let Some(x) = widgets::plain_field(ui, ("freehand-field", key), v, suffix, decimals, 72.0) {
            d.fields.insert(key.into(), json!(x.clamp(range.0, range.1)));
        }
    });
}

/// The Fidelity row: a slider over [`FIDELITY`] in tenths, and a field.
fn fidelity(ui: &mut egui::Ui, d: &mut Dialog) {
    let label_w = label_w(d);
    let t = Tokens::get(ui.ctx());
    let v = d.f64("fidelity", 1.5);
    let (min, max) = FIDELITY;
    let mut new = None;
    widgets::label_row(ui, tl!("Fidelity:"), label_w, |ui| {
        let at = ((v.clamp(min, max) - min) / (max - min)) as f32;
        let shown = widgets::SliderValue::field(v, min..=max, "pt", 1);
        if let (Some(x), _) =
            widgets::color_slider_valued(ui, "freehand-fidelity", tl!("Fidelity:"), Some(shown), at, form::SLIDER_WIDTH, &|_| t.input_border)
        {
            new = Some(((min + x as f64 * (max - min)) * 10.0).round() / 10.0);
        }
        if let Some(x) = widgets::plain_field(ui, "freehand-fidelity-field", v, "pt", 1, 64.0) {
            new = Some(x.clamp(0.05, 100.0));
        }
    });
    if let Some(n) = new.filter(|n| *n != v) {
        d.fields.insert("fidelity".into(), json!(n));
    }
    form::slider_ends(ui, label_w, (tl!("Accurate"), tl!("Smooth")));
    ui.label(
        egui::RichText::new(tl!("A lower value follows the stroke closely; a higher one gives fewer points and smoother curves."))
            .size(11.5)
            .color(t.text_dim),
    );
}

fn body(_app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    if d.fields.contains_key("fidelity") {
        fidelity(ui, d);
        ui.add_space(10.0);
    }
    if d.fields.contains_key("size") {
        number(ui, d, ("size", tl!("Size:")), "pt", 1, (0.1, 1000.0));
    }
    if d.fields.contains_key("fill") {
        check(ui, d, "fill", tl!("Fill new strokes"));
    }
    if d.fields.contains_key("altSmooth") {
        check(ui, d, "altSmooth", tl!("Alt key toggles to Smooth Tool"));
    }
    if d.fields.contains_key("keepSelected") {
        check(ui, d, "keepSelected", tl!("Keep selected"));
        if d.fields.contains_key("mergeOnlySelection") {
            check(ui, d, "mergeOnlySelection", tl!("Merge only with selection"));
        }
        ui.add_space(6.0);
    }
    if d.fields.contains_key("closeWithin") {
        number(ui, d, ("closeWithin", tl!("Close paths when ends are within:")), "px", 0, (0.0, 100.0));
    }
    if d.fields.contains_key("editWithin") {
        number(ui, d, ("editWithin", tl!("Edit selected paths within:")), "px", 0, (0.0, 100.0));
    }
    false
}

/// OK: set the tool's options.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let values: Map<String, Value> = KEYS.iter().filter_map(|k| Some((k.to_string(), d.fields.get(*k)?.clone()))).collect();
    run_and_close(app, "tool.setOption", json!({ "tool": d.str("tool"), "values": values }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    /// The labels the open dialog paints.
    fn painted(app: &mut VectorcraftApp) -> String {
        crate::tests_labels::painted_text(app, |app, ui| super::super::show(app, ui.ctx()))
    }

    #[test]
    fn each_tool_shows_its_options_and_ok_sets_them() {
        let mut app = app();
        // Pencil: Fidelity, fill, the two tolerances; no Size.
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        let text = painted(&mut app);
        for label in [
            "Pencil Tool Options",
            "Fidelity:",
            "Accurate",
            "Smooth",
            "Fill new strokes",
            "Alt key toggles to Smooth Tool",
            "Close paths when ends are within:",
            "Edit selected paths within:",
        ] {
            assert!(text.contains(label), "{label} in {text}");
        }
        assert!(!text.contains("Size:"), "{text}");
        let d = app.ui.dialog.as_mut().unwrap();
        assert_eq!(
            (d.f64("fidelity", 0.0), d.bool("fill"), d.bool("altSmooth"), d.f64("closeWithin", 0.0), d.f64("editWithin", 0.0)),
            (1.5, false, false, 15.0, 12.0)
        );
        for (k, v) in
            [("fidelity", json!(6.5)), ("fill", json!(true)), ("altSmooth", json!(true)), ("closeWithin", json!(0)), ("editWithin", json!(20))]
        {
            d.fields.insert(k.into(), v);
        }
        super::super::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        let o = app.session.tool_options_of("pencil");
        assert_eq!(
            (o["fidelity"].as_f64(), o["fill"].as_bool(), o["altSmooth"].as_bool(), o["closeWithin"].as_f64(), o["editWithin"].as_f64()),
            (Some(6.5), Some(true), Some(true), Some(0.0), Some(20.0))
        );
        // The Paintbrush keeps its own: the Pencil's change left it alone.
        app.run("tool.options", json!({"tool": "paintbrush"})).unwrap();
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.f64("fidelity", 0.0), d.bool("fill")), (1.5, false));
        let pb_text = painted(&mut app);
        assert!(pb_text.contains("Paintbrush Tool Options"));
        assert!(!pb_text.contains("Alt key toggles to Smooth Tool"));
        // Smooth has Fidelity only, the Eraser Size only, and the Blob Brush Size, Keep selected and Merge only with selection.
        app.run("tool.options", json!({"tool": "smooth"})).unwrap();
        let text = painted(&mut app);
        assert!(text.contains("Smooth Tool Options") && text.contains("Fidelity:"), "{text}");
        assert!(
            !text.contains("Fill new strokes")
                && !text.contains("Keep selected")
                && !text.contains("Alt key toggles to Smooth Tool")
                && !text.contains("Size:"),
            "{text}"
        );
        for tool in ["blobBrush", "eraser"] {
            app.run("tool.options", json!({"tool": tool})).unwrap();
            let text = painted(&mut app);
            assert!(text.contains("Size:") && !text.contains("Fidelity:"), "{tool}: {text}");
            // The Blob Brush also has Keep selected and Merge only with selection.
            let blob = tool == "blobBrush";
            assert_eq!((text.contains("Keep selected"), text.contains("Merge only with selection")), (blob, blob), "{tool}: {text}");
            app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!(24));
            super::super::confirm(&mut app).unwrap();
            assert_eq!(app.session.tool_options_of(tool)["size"].as_f64(), Some(24.0), "{tool}");
        }
    }

    #[test]
    fn a_new_fidelity_changes_what_the_pencil_draws() {
        let mut app = app();
        // The anchors of a wave drawn around height `y0`, with nothing selected (a stroke starting
        // at the end of a selected path would continue it).
        let anchors = |app: &mut VectorcraftApp, y0: f64| {
            app.run("select.set", json!({"ids": []})).unwrap();
            app.select_tool("pencil");
            let view = app.view_info();
            let pts: Vec<_> = (0..=40).map(|i| (20.0 + i as f64 * 8.0, y0 + 30.0 * (i as f64 * 0.25).sin())).collect();
            let pointer = |kind, (x, y)| vectorcraft_tools::PointerEvent::new(kind, x, y);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, pts[0]), view);
            for p in &pts[1..] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, *p), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, pts[40]), view);
            let doc = &app.session.doc().unwrap().doc;
            doc.layers[0].children().unwrap().last().unwrap().path_data().map_or(0, |p| p.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>())
        };
        let precise = anchors(&mut app, 100.0);
        app.run("tool.options", json!({"tool": "pencil"})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("fidelity".into(), json!(12));
        super::super::confirm(&mut app).unwrap();
        let smooth = anchors(&mut app, 200.0);
        assert!(smooth < precise, "{smooth} anchors at fidelity 12, {precise} at the default");
    }

    #[test]
    fn the_new_path_is_selected_only_with_keep_selected_on() {
        let mut app = app();
        // A stroke of `tool` along height `y0`; the ids selected afterwards and the new path's id.
        let stroke = |app: &mut VectorcraftApp, tool: &str, y0: f64| {
            app.select_tool(tool);
            let view = app.view_info();
            let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, y0);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
            for x in [60.0, 100.0, 140.0] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 180.0), view);
            let st = app.session.doc().unwrap();
            (st.selection.objects.clone(), st.doc.layers[0].children().unwrap().last().unwrap().id)
        };
        for tool in ["pencil", "paintbrush"] {
            app.run("select.set", json!({"ids": []})).unwrap();
            // On by default: the new path is selected.
            app.run("tool.options", json!({"tool": tool})).unwrap();
            assert!(painted(&mut app).contains("Keep selected"), "{tool}");
            let d = app.ui.dialog.as_mut().unwrap();
            assert!(d.bool("keepSelected"), "{tool}");
            super::super::confirm(&mut app).unwrap();
            let (selected, id) = stroke(&mut app, tool, 100.0);
            assert_eq!(selected, vec![id], "{tool}");
            // Off: the next stroke leaves the selection as it was.
            app.run("tool.options", json!({"tool": tool})).unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("keepSelected".into(), json!(false));
            super::super::confirm(&mut app).unwrap();
            // It lasts while another tool is chosen.
            app.select_tool("selection");
            assert_eq!(app.session.tool_options_of(tool)["keepSelected"].as_bool(), Some(false), "{tool}");
            let (selected, new) = stroke(&mut app, tool, 200.0);
            assert_ne!(new, id, "{tool} drew a new path");
            assert_eq!(selected, vec![id], "{tool}");
        }
    }

    #[test]
    fn the_blob_brush_options_set_what_a_stroke_merges_with_and_selects() {
        let mut app = app();
        // A Blob Brush stroke across the canvas at height 100; the ids selected afterwards.
        let stroke = |app: &mut VectorcraftApp| {
            app.select_tool("blobBrush");
            let view = app.view_info();
            let pointer = |kind, x| vectorcraft_tools::PointerEvent::new(kind, x, 100.0);
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Down, 20.0), view);
            for x in [80.0, 140.0, 200.0, 260.0] {
                crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Drag, x), view);
            }
            crate::canvas::dispatch(app, &pointer(vectorcraft_tools::PointerKind::Up, 300.0), view);
            app.session.doc().unwrap().selection.objects.clone()
        };
        let ids = |app: &VectorcraftApp| app.session.doc().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
        // Two blobs the stroke crosses, drawn with the same color.
        let blob = |app: &mut VectorcraftApp, x: f64| {
            let r = app.run("path.blob", json!({"points": [[x, 60], [x, 140]], "size": 20})).unwrap();
            vectorcraft_doc::NodeId(r["id"].as_u64().unwrap())
        };
        let (a, b) = (blob(&mut app, 100.0), blob(&mut app, 240.0));
        app.run("tool.options", json!({"tool": "blobBrush"})).unwrap();
        let d = app.ui.dialog.as_mut().unwrap();
        // Both are off by default.
        assert!(!d.bool("keepSelected") && !d.bool("mergeOnlySelection"));
        d.fields.insert("keepSelected".into(), json!(true));
        d.fields.insert("mergeOnlySelection".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let o = app.session.tool_options_of("blobBrush");
        assert_eq!((o["keepSelected"].as_bool(), o["mergeOnlySelection"].as_bool()), (Some(true), Some(true)));
        // With `b` selected, the stroke merges into `b` alone, and leaves it selected.
        app.run("select.set", json!({"ids": [b.0]})).unwrap();
        assert_eq!(stroke(&mut app), vec![b]);
        assert_eq!(ids(&app), vec![a, b], "merged into b only");
    }

    #[test]
    fn the_other_tools_have_no_freehand_options() {
        let mut app = app();
        assert!(open(&mut app, "pen").is_err());
        assert!(app.ui.dialog.is_none());
    }
}
