//! Object → Step and Repeat: copies of the selection on a grid or filling a sheet, with the gaps
//! between the cut paths, every other column or row turned by 180° (`object.stepAndRepeat`),
//! previewed live on the canvas.
//!
//! Fields: `rows`, `columns`, `gapX`, `gapY` (pt), `margin` (pt), `fill` (as many as fit the
//! artboard), `rotate` (`none`, `columns` or `rows`), `cutId` (an object of the selection, or
//! empty for the selection itself).

use craft::ui::contract::Bind;
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::{Label, LengthUnits, UnitKind};
use serde_json::{Map, Value, json};

use super::host::{HeldOptions, open_held_with};
use super::preview::LivePreview;
use crate::VectorcraftApp;

pub(crate) const CMD: &str = "object.stepAndRepeat";

pub(crate) struct Open {
    pub id: DialogId,
    pub preview: LivePreview,
}

fn cut_options(app: &VectorcraftApp) -> Vec<(String, Label)> {
    let mut out = vec![(String::new(), Label::from(tl!("Selection")))];
    let Some(st) = app.session.active() else { return out };
    let single = st.selection.objects.len() == 1;
    for id in &st.selection.objects {
        let Some(n) = st.doc.node(*id) else { continue };
        out.push((id.0.to_string(), Label::from(n.display_name())));
        if single {
            for c in n.children().into_iter().flatten() {
                out.push((c.id.0.to_string(), Label::from(c.display_name())));
            }
        }
    }
    out
}

fn rows(lengths: LengthUnits, cuts: Vec<(String, Label)>) -> Vec<Param> {
    let len = |key: &str, label: &'static str| Param::number(key, label, 0.0, 1.0, 2).length(lengths);
    vec![
        Param::new("rows", tl!("Rows"), ParamKind::int(Some(1), Some(500), 2)).enabled_when("fill", false),
        Param::new("columns", tl!("Columns"), ParamKind::int(Some(1), Some(500), 2)).enabled_when("fill", false),
        len("gapX", tl!("Horizontal Gap")),
        len("gapY", tl!("Vertical Gap")),
        Param::new("cutId", tl!("Cut Path"), ParamKind::choice(cuts, Some(String::new()))),
        Param::new("fill", tl!("Fill the Artboard"), ParamKind::bool(false)),
        len("margin", tl!("Margin")).enabled_when("fill", true),
        Param::new(
            "rotate",
            tl!("Rotate 180°"),
            ParamKind::choice(
                vec![
                    ("none".into(), Label::from(tl!("None"))),
                    ("columns".into(), Label::from(tl!("Every Other Column"))),
                    ("rows".into(), Label::from(tl!("Every Other Row"))),
                ],
                Some("none".into()),
            ),
        ),
    ]
}

pub fn fields() -> DialogFields {
    DialogFields::from_pairs([
        ("rows", FieldValue::Int(2)),
        ("columns", FieldValue::Int(2)),
        ("gapX", FieldValue::Number(12.0)),
        ("gapY", FieldValue::Number(12.0)),
        ("margin", FieldValue::Number(0.0)),
        ("fill", FieldValue::Bool(false)),
        ("cutId", FieldValue::Text(String::new())),
        ("rotate", FieldValue::Text("none".into())),
    ])
}

fn count(f: &DialogFields, key: &str) -> u64 {
    f.int(key).or_else(|| f.number(key).map(|n| n as i64)).unwrap_or(1).max(1) as u64
}

fn params(f: &DialogFields) -> Value {
    let mut p = Map::new();
    p.insert("rows".into(), json!(count(f, "rows")));
    p.insert("columns".into(), json!(count(f, "columns")));
    for k in ["gapX", "gapY", "margin"] {
        p.insert(k.into(), json!(f.number(k).unwrap_or(0.0)));
    }
    p.insert("fill".into(), json!(f.bool("fill").unwrap_or(false)));
    p.insert("rotate".into(), json!(f.text("rotate").unwrap_or("none")));
    if let Some(id) = f.text("cutId").and_then(|t| t.parse::<u64>().ok()) {
        p.insert("cutId".into(), json!(id));
    }
    Value::Object(p)
}

pub(crate) fn open(app: &mut VectorcraftApp) {
    let _ = try_open(app);
}

fn try_open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let lengths = LengthUnits::new(UnitKind::Points, super::rearrange_artboards::length_kind(app.session.general_unit()), 72.0);
    let lengths = crate::fields::bare_points(lengths, app.session.prefs.numbers_without_units_are_points);
    let cuts = cut_options(app);
    let dialog = HostedParamDialog::command(&ctx, CMD, fields(), move |fields| {
        ParamDialog::command(CMD, tl!("Step and Repeat"), rows(lengths, cuts.clone())).previews(true).values(Bind::bound(fields))
    });
    let ok = |app: &mut VectorcraftApp, result: &ParamResult| {
        let mut open = app.object_dialogs.step.take().ok_or("the dialog is closed")?;
        let done = std::mem::take(&mut open.preview).commit(app, CMD, params(&result.fields));
        if done.is_err() {
            app.object_dialogs.step = Some(open);
        }
        done.map(|_| ())
    };
    let cancel: Box<dyn FnOnce(&mut VectorcraftApp)> = Box::new(|app| {
        if let Some(open) = app.object_dialogs.step.take() {
            open.preview.cancel(app);
        }
    });
    let id = open_held_with(app, dialog, ok, HeldOptions { cancel: Some(cancel), ..Default::default() }).ok_or("no window")?;
    app.object_dialogs.step = Some(Open { id, preview: LivePreview::default() });
    Ok(Value::Null)
}

pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.object_dialogs.step.take() else { return };
    let shown =
        app.dialogs().and_then(|host| host.with(|h| h.get_by_id::<HostedParamDialog>(open.id).map(|d| (d.is_previewing(), params(d.fields())))));
    if let Some((on, p)) = shown {
        let _ = open.preview.run(app, on, "Step and Repeat", CMD, &p);
    }
    app.object_dialogs.step = Some(open);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cut_path_is_sent_only_when_one_is_chosen() {
        let mut f = fields();
        assert!(params(&f).get("cutId").is_none());
        f.set("cutId", FieldValue::Text("42".into()));
        assert_eq!(params(&f)["cutId"], json!(42));
    }
}
