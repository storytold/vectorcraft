//! Character Style Options fields shared by editing and creating a character style.

use serde_json::{Map, Value, json};

use super::{DialogSpec, form};
use crate::VectorcraftApp;
use crate::state::Dialog;

pub const KIND: &str = "charStyleOptions";
const BASIC: &[&str] = &[
    "font_family",
    "font_style",
    "size",
    "leading",
    "kerning",
    "tracking",
    "all_caps",
    "small_caps",
    "position",
    "underline",
    "strikethrough",
    "standardVerticalRoman",
    "charAlign",
];
const NUMBERS: &[&str] = &["size", "leading", "kerning", "tracking"];
/// The on, off or unset checkboxes.
const SWITCHES: &[&str] = &["underline", "strikethrough", "standardVerticalRoman"];

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Character Style Options").into(),
    body,
    confirm,
    preview: true,
    inputs: |_| {
        &[
            "name",
            "font_family",
            "font_style",
            "size",
            "leading",
            "kerning",
            "tracking",
            "case",
            "position",
            "underline",
            "strikethrough",
            "standardVerticalRoman",
            "charAlign",
            "section",
        ]
    },
    min_width: 650.0,
    ..DialogSpec::FORM
};

pub fn open(app: &mut VectorcraftApp, name: &str) -> Result<Value, String> {
    crate::panels::character::end_typing(app);
    let doc = &app.session.active().ok_or("no document open")?.doc;
    let def = doc.char_styles.iter().find(|s| s.name == name).ok_or_else(|| format!("no character style `{name}`"))?;
    let mut fields = fields_from_attrs(&def.attrs);
    fields.insert("name".into(), json!(name));
    fields.insert("__style".into(), json!(name));
    fields.insert("__attrs".into(), json!(def.attrs));
    fields.insert("__smallCapsSize".into(), json!(doc.setup.small_caps_size));
    fields.insert("__superscript".into(), json!(doc.setup.superscript));
    fields.insert("__subscript".into(), json!(doc.setup.subscript));
    fields.insert("section".into(), json!("general"));
    fields.insert("preview".into(), json!(true));
    app.ui.dialog = Some(Dialog { kind: KIND.into(), fields });
    Ok(json!({"dialog": KIND}))
}

pub(crate) fn fields_from_attrs(attrs: &Map<String, Value>) -> Map<String, Value> {
    let mut fields = Map::new();
    for key in
        ["font_family", "font_style", "size", "leading", "kerning", "tracking", "underline", "strikethrough", "standardVerticalRoman", "charAlign"]
    {
        let value = attrs.get(key).cloned().unwrap_or(Value::Null);
        let shown = if SWITCHES.contains(&key) {
            value
        } else if value.is_null() {
            if attrs.contains_key(key) && ["leading", "kerning"].contains(&key) { json!("Auto") } else { json!("") }
        } else {
            value
        };
        fields.insert(key.into(), shown);
    }
    fields.insert("case".into(), case_from_attrs(attrs).map_or(Value::Null, Value::from));
    fields.insert("position".into(), attrs.get("position").and_then(|v| v.get("kind")).cloned().unwrap_or(Value::Null));
    fields
}

fn case_from_attrs(attrs: &Map<String, Value>) -> Option<&'static str> {
    if attrs.get("all_caps") == Some(&json!(true)) {
        Some("allCaps")
    } else if attrs.get("small_caps").is_some_and(|v| !v.is_null()) {
        Some("smallCaps")
    } else if attrs.contains_key("all_caps") || attrs.contains_key("small_caps") {
        Some("normal")
    } else {
        None
    }
}

fn optional_number(d: &Dialog, key: &str) -> Result<Option<Value>, String> {
    let label = match key {
        "size" => tl!("Size"),
        "leading" => tl!("Leading"),
        "kerning" => tl!("Kerning"),
        "tracking" => tl!("Tracking"),
        _ => return Err("unknown character style number field".into()),
    };
    match d.fields.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("auto") && ["leading", "kerning"].contains(&key) => Ok(Some(Value::Null)),
        Some(v) => {
            let n = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()))
                .filter(|n| n.is_finite())
                .ok_or_else(|| format!("{label} must be a number"))?;
            if ["size", "leading"].contains(&key) && n < 0.1 {
                return Err(format!("{label} must be a number of at least 0.1"));
            }
            Ok(Some(json!(n)))
        }
    }
}

pub(crate) fn edited_attrs(d: &Dialog) -> Result<Map<String, Value>, String> {
    let original = d.fields.get("__attrs").and_then(Value::as_object).cloned().unwrap_or_default();
    let original_fields = fields_from_attrs(&original);
    let reset = d.bool("__resetBasic");
    let mut attrs = original.clone();
    for key in BASIC {
        attrs.remove(*key);
    }
    for key in ["font_family", "font_style"] {
        if let Some(v) = d.fields.get(key).and_then(Value::as_str).filter(|v| !v.trim().is_empty()) {
            attrs.insert(key.into(), json!(v.trim()));
        }
    }
    if reset || ["font_family", "font_style"].iter().any(|key| d.fields.get(*key) != original_fields.get(*key)) {
        attrs.remove("font_version");
        attrs.remove("fontAxes");
    }
    for key in NUMBERS {
        if let Some(v) = optional_number(d, key)? {
            attrs.insert((*key).into(), v);
        }
    }
    for key in SWITCHES {
        if let Some(v) = d.fields.get(*key).and_then(Value::as_bool) {
            attrs.insert((*key).into(), json!(v));
        }
    }
    if reset || d.fields.get("case").and_then(Value::as_str) != case_from_attrs(&original) {
        match d.fields.get("case").and_then(Value::as_str) {
            Some("allCaps") => {
                attrs.insert("all_caps".into(), json!(true));
                attrs.insert("small_caps".into(), Value::Null);
            }
            Some("smallCaps") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert(
                    "small_caps".into(),
                    original
                        .get("small_caps")
                        .filter(|v| v.is_number())
                        .cloned()
                        .unwrap_or_else(|| d.fields.get("__smallCapsSize").cloned().unwrap_or_else(|| json!(70.0))),
                );
            }
            Some("normal") => {
                attrs.insert("all_caps".into(), json!(false));
                attrs.insert("small_caps".into(), Value::Null);
            }
            _ => {}
        }
    }
    if let Some(position) = d.fields.get("position").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        let value = if position == "normal" {
            json!({"kind": "normal"})
        } else {
            original.get("position").filter(|v| v.get("kind").and_then(Value::as_str) == Some(position)).cloned().unwrap_or_else(|| {
                let metrics = d.fields.get(if position == "superscript" { "__superscript" } else { "__subscript" });
                json!({"kind": position, "size": metrics.and_then(|m| m.get("size")).cloned().unwrap_or(json!(58.3)),
                        "position": metrics.and_then(|m| m.get("position")).cloned().unwrap_or(json!(33.3))})
            })
        };
        attrs.insert("position".into(), value);
    }
    if let Some(align) = d.fields.get("charAlign").and_then(Value::as_str).filter(|v| !v.is_empty()) {
        attrs.insert("charAlign".into(), json!(align));
    }
    if !reset {
        for key in BASIC {
            let field = if ["all_caps", "small_caps"].contains(key) { "case" } else { key };
            if d.fields.get(field) == original_fields.get(field) {
                attrs.remove(*key);
                if let Some(value) = original.get(*key) {
                    attrs.insert((*key).into(), value.clone());
                }
            }
        }
    }
    Ok(attrs)
}

pub(crate) fn reset_basic(d: &mut Dialog) {
    d.fields.insert("__resetBasic".into(), json!(true));
    for key in [
        "font_family",
        "font_style",
        "size",
        "leading",
        "kerning",
        "tracking",
        "case",
        "position",
        "underline",
        "strikethrough",
        "standardVerticalRoman",
        "charAlign",
    ] {
        d.fields.insert(
            key.into(),
            if ["font_family", "font_style", "size", "leading", "kerning", "tracking"].contains(&key) { json!("") } else { Value::Null },
        );
    }
}

pub fn reset_panel(app: &mut VectorcraftApp) -> Result<Value, String> {
    let d = app.ui.dialog.as_mut().ok_or("no dialog open")?;
    let character_new = d.kind == super::new_text_style::KIND && !d.bool("__para");
    if (d.kind != KIND && !character_new) || d.str("section") != "basic" {
        return Err("the open dialog has no Reset".into());
    }
    reset_basic(d);
    Ok(json!({"dialog": d.kind}))
}

fn choice(ui: &mut egui::Ui, d: &mut Dialog, key: &str, options: &[(&str, &str)]) {
    let selected = d.fields.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let shown = options.iter().find(|(value, _)| *value == selected).map_or("", |(_, label)| *label);
    let labels: Vec<&str> = std::iter::once(" ").chain(options.iter().map(|(_, label)| *label)).collect();
    if let Some(i) = crate::widgets::dropdown(ui, (KIND, key), shown, &labels, 190.0) {
        d.fields.insert(key.into(), if i == 0 { Value::Null } else { options.get(i - 1).map_or(Value::Null, |(value, _)| json!(value)) });
    }
}

pub(crate) fn basic_body(ui: &mut egui::Ui, d: &mut Dialog) {
    egui::Grid::new("character-style-basic").num_columns(2).spacing([18.0, 8.0]).show(ui, |ui| {
        for (key, caption) in [
            ("font_family", tl!("Font Family:")),
            ("font_style", tl!("Font Style:")),
            ("size", tl!("Size")),
            ("leading", tl!("Leading")),
            ("kerning", tl!("Kerning")),
            ("tracking", tl!("Tracking")),
        ] {
            ui.label(caption);
            if d.fields.get(key).is_some_and(Value::is_null) {
                d.fields.insert(key.into(), json!(""));
            }
            form::text(ui, d, key, 190.0);
            ui.end_row();
        }
        ui.label(tl!("Case"));
        choice(ui, d, "case", &[("normal", tl!("Normal")), ("allCaps", tl!("All Caps")), ("smallCaps", tl!("Small Caps"))]);
        ui.end_row();
        ui.label(tl!("Position"));
        choice(ui, d, "position", &[("normal", tl!("Normal")), ("superscript", tl!("Superscript")), ("subscript", tl!("Subscript"))]);
        ui.end_row();
        for (key, caption) in [
            ("underline", tl!("Underline")),
            ("strikethrough", tl!("Strikethrough")),
            ("standardVerticalRoman", tl!("Standard Vertical Roman Alignment")),
        ] {
            ui.label("");
            let value = d.fields.get(key).and_then(Value::as_bool);
            if crate::widgets::check3(ui, caption, value, true) {
                d.fields.insert(
                    key.into(),
                    match value {
                        None => json!(true),
                        Some(true) => json!(false),
                        Some(false) => Value::Null,
                    },
                );
            }
            ui.end_row();
        }
        ui.label(tl!("Character Alignment"));
        choice(
            ui,
            d,
            "charAlign",
            &[
                ("romanBaseline", tl!("Roman Baseline")),
                ("emBoxTop", tl!("Em Box Top/Right")),
                ("emBoxCenter", tl!("Em Box Center")),
                ("emBoxBottom", tl!("Em Box Bottom/Left")),
                ("icfTop", tl!("ICF Top/Right")),
                ("icfBottom", tl!("ICF Bottom/Left")),
            ],
        );
        ui.end_row();
    });
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            for (id, label) in [("general", tl!("General")), ("basic", tl!("Basic Character Formats"))] {
                if ui.selectable_label(d.str("section") == id, label).clicked() {
                    d.fields.insert("section".into(), json!(id));
                }
            }
        });
        ui.separator();
        ui.vertical(|ui| {
            if d.str("section") == "basic" {
                basic_body(ui, d);
            } else {
                ui.horizontal(|ui| {
                    ui.label(tl!("Style Name:"));
                    form::text(ui, d, "name", 190.0);
                });
                ui.separator();
                if let Ok(attrs) = edited_attrs(d) {
                    for (key, value) in attrs {
                        ui.label(format!("{}: {}", super::humanize(&key), value));
                    }
                }
            }
        });
    });
    ui.add_enabled_ui(d.str("section") == "basic", |ui| {
        if ui.button(tl!("Reset Panel")).clicked() {
            reset_basic(d);
        }
    });
    match edited_attrs(d) {
        Ok(attrs) => {
            let params = json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs});
            form::preview(app, ui, d, tl!("Character Style Options"), "charStyle.edit", params);
        }
        Err(why) => {
            ui.add_space(6.0);
            ui.colored_label(crate::theme::Tokens::get(ui.ctx()).error, crate::i18n::message(crate::i18n::current(), &why));
        }
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let attrs = edited_attrs(d)?;
    form::commit_preview(app, "charStyle.edit", json!({"name": d.str("__style"), "to": d.str("name"), "attrs": attrs}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_engine::Session;

    fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
        app
    }

    fn frame(app: &mut VectorcraftApp) {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(Default::default(), |ui| super::super::show(app, ui.ctx()));
        out.textures_delta.clear();
    }

    #[test]
    fn unchanged_basic_fields_preserve_original_keys() {
        for attrs in [
            json!({"all_caps": false}),
            json!({"all_caps": true}),
            json!({"small_caps": 70.0}),
            json!({"font_family": "Example", "font_style": "Regular", "font_version": "1", "size": 24,
                "leading": null, "kerning": null, "tracking": 3, "underline": null,
                "strikethrough": false, "position": {"kind": "normal", "size": 80}, "charAlign": null}),
        ] {
            let original = attrs.as_object().unwrap().clone();
            let mut fields = fields_from_attrs(&original);
            fields.insert("__attrs".into(), attrs);
            let d = Dialog { kind: KIND.into(), fields };
            assert_eq!(edited_attrs(&d).unwrap(), original);
        }
        let original = json!({"all_caps": false}).as_object().unwrap().clone();
        let mut fields = fields_from_attrs(&original);
        fields.insert("__attrs".into(), json!(original));
        fields.insert("case".into(), json!("allCaps"));
        let d = Dialog { kind: KIND.into(), fields };
        assert_eq!(edited_attrs(&d).unwrap(), json!({"all_caps": true, "small_caps": null}).as_object().unwrap().clone());
    }

    #[test]
    fn unchanged_options_keep_local_small_caps_and_history() {
        for preview in [false, true] {
            let mut app = app();
            app.run("charStyle.new", json!({"name": "Body", "attrs": {"all_caps": false}})).unwrap();
            let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
            app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
            app.run("text.setRangeStyle", json!({"id": id, "start": 0, "end": 4, "smallCaps": true})).unwrap();
            let state = app.session.doc().unwrap();
            let before = state.doc.clone();
            let history = (state.history.undo.len(), state.history.redo.len());
            open(&mut app, "Body").unwrap();
            app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(preview));
            if preview {
                frame(&mut app);
                assert_eq!(app.session.doc().unwrap().doc, before);
            }
            super::super::confirm(&mut app).unwrap();
            let state = app.session.doc().unwrap();
            assert_eq!(state.doc, before);
            assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
            assert_eq!(
                state.doc.node(vectorcraft_doc::NodeId(id)).and_then(|node| match &node.kind {
                    vectorcraft_doc::NodeKind::Text(text) => text.runs.first().map(|run| run.style.small_caps),
                    _ => None,
                }),
                Some(Some(state.doc.setup.small_caps_size))
            );
        }
    }

    #[test]
    fn options_preview_cancel_reset_and_commit_keep_sparse_definition() {
        let mut app = app();
        app.run("charStyle.new", json!({"name":"Body","attrs":{"size":24.0,"baseline_shift":3.0}})).unwrap();
        let id = app.run("text.create", json!({"x":10,"y":40,"text":"Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name":"Body","id":id})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        open(&mut app, "Body").unwrap();
        assert!(app.ui.dialog.as_ref().unwrap().bool("preview"));
        assert!(app.run("ui.charStyleOptions.resetPanel", json!({})).is_err());
        assert!(app.ui.dialog.as_ref().unwrap().fields["underline"].is_null());
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("kerning"), "");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Revised"));
            d.fields.insert("size".into(), json!(18));
        }
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
        super::super::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, before);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        open(&mut app, "Body").unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("section".into(), json!("basic"));
        }
        app.run("ui.charStyleOptions.resetPanel", json!({})).unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            assert_eq!(edited_attrs(d).unwrap(), serde_json::from_value(json!({"baseline_shift":3.0})).unwrap());
            d.fields.insert("name".into(), json!("Revised"));
        }
        super::super::confirm(&mut app).unwrap();
        let state = app.session.doc().unwrap();
        assert_eq!(state.history.undo.len(), undo + 1);
        assert_eq!(state.doc.char_styles[0].name, "Revised");
        assert_eq!(state.doc.char_styles[0].attrs, serde_json::from_value(json!({"baseline_shift":3.0})).unwrap());
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before);
        app.run("edit.redo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].name, "Revised");
        app.run("select.set", json!({"ids":[id]})).unwrap();
        app.run("ui.newCharStyle", json!({})).unwrap();
        assert_eq!(app.ui.dialog.as_ref().unwrap().str("size"), "12.0");
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("size".into(), json!(36.0));
            d.fields.insert("name".into(), json!("Future"));
        }
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles.iter().find(|def| def.name == "Future").unwrap().attrs["size"], json!(36.0));
    }

    /// Basic Character Formats shows Character Alignment under its own name, and Standard Vertical
    /// Roman Alignment as its own on, off or unset checkbox that the style saves.
    #[test]
    fn standard_vertical_roman_alignment_has_its_own_row() {
        let mut d = Dialog::new(KIND, json!({}));
        d.fields.extend(fields_from_attrs(&Map::new()));
        d.fields.insert("__attrs".into(), json!({}));
        let mut app = app();
        let shown = crate::tests_labels::painted_text(&mut app, |_, ui| basic_body(ui, &mut d));
        assert!(shown.contains("Character Alignment"), "{shown}");
        assert!(shown.contains("Standard Vertical Roman Alignment"), "{shown}");
        assert!(d.fields["standardVerticalRoman"].is_null());
        assert_eq!(edited_attrs(&d).unwrap(), Map::new());
        d.fields.insert("standardVerticalRoman".into(), json!(true));
        d.fields.insert("charAlign".into(), json!("emBoxCenter"));
        assert_eq!(edited_attrs(&d).unwrap(), json!({"standardVerticalRoman": true, "charAlign": "emBoxCenter"}).as_object().unwrap().clone());
        let fields = fields_from_attrs(json!({"standardVerticalRoman": false}).as_object().unwrap());
        assert_eq!(fields["standardVerticalRoman"], json!(false));
        reset_basic(&mut d);
        assert!(d.fields["standardVerticalRoman"].is_null());

        app.run("charStyle.new", json!({"name": "Upright", "attrs": {}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "AB", "vertical": true})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Upright", "id": id})).unwrap();
        open(&mut app, "Upright").unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("standardVerticalRoman".into(), json!(true));
        super::super::confirm(&mut app).unwrap();
        let doc = &app.session.doc().unwrap().doc;
        assert_eq!(doc.char_styles[0].attrs["standardVerticalRoman"], json!(true));
        let vectorcraft_doc::NodeKind::Text(t) = &doc.node(vectorcraft_doc::NodeId(id)).unwrap().kind else { panic!("text") };
        assert!(t.runs.iter().all(|r| r.style.standard_vertical_roman));
    }

    #[test]
    fn invalid_numbers_name_the_visible_field() {
        let mut d = Dialog::new(KIND, json!({}));
        for (key, label) in [("size", "Size"), ("leading", "Leading"), ("kerning", "Kerning"), ("tracking", "Tracking")] {
            d.fields.insert(key.into(), json!("not a number"));
            assert_eq!(optional_number(&d, key).unwrap_err(), format!("{label} must be a number"));
        }
    }

    #[test]
    fn size_and_leading_below_minimum_do_not_preview_or_commit() {
        for (key, value) in [("size", json!(0)), ("size", json!(-2)), ("leading", json!(0.09)), ("leading", json!(-1))] {
            let mut app = app();
            app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0, "leading": 28.0}})).unwrap();
            let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
            app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
            let before = app.session.doc().unwrap().doc.clone();
            open(&mut app, "Body").unwrap();
            {
                let d = app.ui.dialog.as_mut().unwrap();
                d.fields.insert("name".into(), json!("Revised"));
                d.fields.insert(key.into(), value);
                assert!(edited_attrs(d).unwrap_err().contains("at least 0.1"));
            }
            frame(&mut app);
            assert_eq!(app.session.doc().unwrap().doc, before, "{key} preview");
            assert!(super::super::confirm(&mut app).is_err(), "{key} confirm");
            assert_eq!(app.session.doc().unwrap().doc, before, "{key} confirm");
        }
    }

    #[test]
    fn invalid_number_keeps_last_valid_preview() {
        let mut app = app();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {"size": 24.0}})).unwrap();
        let id = app.run("text.create", json!({"x": 10, "y": 40, "text": "Text"})).unwrap()["id"].as_u64().unwrap();
        app.run("charStyle.apply", json!({"name": "Body", "id": id})).unwrap();
        open(&mut app, "Body").unwrap();
        {
            let d = app.ui.dialog.as_mut().unwrap();
            d.fields.insert("name".into(), json!("Revised"));
            d.fields.insert("size".into(), json!(18));
        }
        frame(&mut app);
        let valid = app.session.doc().unwrap().doc.clone();
        assert_eq!(valid.char_styles[0].name, "Revised");
        assert_eq!(valid.char_styles[0].attrs["size"], json!(18.0));
        app.ui.dialog.as_mut().unwrap().fields.insert("size".into(), json!("invalid"));
        frame(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, valid);
    }

    #[test]
    fn script_positions_use_document_setup_in_existing_and_new_style_dialogs() {
        let mut app = app();
        app.run(
            "document.setup",
            json!({"superscript": {"size": 67.0, "position": 22.0},
            "subscript": {"size": 72.0, "position": 18.0}}),
        )
        .unwrap();
        app.run("charStyle.new", json!({"name": "Body", "attrs": {}})).unwrap();
        open(&mut app, "Body").unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("position".into(), json!("superscript"));
        assert_eq!(
            edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap()["position"],
            json!({"kind": "superscript", "size": 67.0, "position": 22.0})
        );
        super::super::confirm(&mut app).unwrap();
        assert_eq!(app.session.doc().unwrap().doc.char_styles[0].attrs["position"], json!({"kind": "superscript", "size": 67.0, "position": 22.0}));
        app.run("ui.newCharStyle", json!({})).unwrap();
        app.ui.dialog.as_mut().unwrap().fields.insert("position".into(), json!("subscript"));
        assert_eq!(edited_attrs(app.ui.dialog.as_ref().unwrap()).unwrap()["position"], json!({"kind": "subscript", "size": 72.0, "position": 18.0}));
        super::super::confirm(&mut app).unwrap();
        assert_eq!(
            app.session.doc().unwrap().doc.char_styles.last().unwrap().attrs["position"],
            json!({"kind": "subscript", "size": 72.0, "position": 18.0})
        );
    }
}
