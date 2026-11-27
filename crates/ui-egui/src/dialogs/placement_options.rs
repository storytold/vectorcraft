//! Links panel → Placement Options: how a file read again (Relink, Update Link) takes the place of
//! the selected images: what it preserves, where it aligns in the old bounds, and whether it is
//! clipped to them.
//!
//! Fields: `preserve` (transforms|bounds|fileDimensions|fit|fill), `align` (0-8, row by row from
//! the top left) and `clip`. OK runs `links.placementOptions` with them and `ids` (null: the
//! selected images): one undo step.

// PLAN: assumed craft_ui paths for ParamDialog, ParamKind, Param, DialogFields, FieldValue, HostedParamDialog and Label.
use craft::ui::contract::Bind;
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::tool_options::reference_param;

const KEY: &str = "placementOptions";
const CMD: &str = "links.placementOptions";
/// (value, label) of each Preserve choice.
const PRESERVE: [(&str, &str); 5] = [
    ("transforms", "Transforms"),
    ("bounds", "Bounds"),
    ("fileDimensions", "File Dimensions"),
    ("fit", "Bounds (Fit Proportionally)"),
    ("fill", "Bounds (Fill Proportionally)"),
];
/// The alignment points, row by row from the top left (the reference point widget's order).
pub(crate) const ALIGN: [&str; 9] = ["topLeft", "top", "topRight", "left", "center", "right", "bottomLeft", "bottom", "bottomRight"];
/// The centre, for a name `ALIGN` does not list.
const CENTRE: usize = 4;
/// Bounds stretches the art into the old bounds: with it there is nothing to align or clip.
const FREE: [&str; 4] = ["transforms", "fileDimensions", "fit", "fill"];

fn params() -> Vec<Param> {
    let options = PRESERVE.iter().map(|(value, label)| ((*value).to_owned(), Label::from(tl!(label).to_owned()))).collect();
    vec![
        Param::new("preserve", tl!("Preserve:"), ParamKind::choice(options, Some("transforms".into()))),
        reference_param("align", tl!("Alignment:")).enabled_when_in("preserve", FREE),
        Param::new("clip", tl!("Clip to Bounding Box"), ParamKind::bool(false)).enabled_when_in("preserve", FREE),
    ]
}

/// Open the dialog on the placement options of images `ids` (default: the selected ones).
pub fn open(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let source_document = app.session.active().map(|document| document.uid);
    let ids = p.get("ids").cloned().unwrap_or(Value::Null);
    let r = app.session.execute(CMD, &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let placement = &r["placement"];
    let align = ALIGN.iter().position(|a| Some(*a) == placement["align"].as_str()).unwrap_or(CENTRE);
    let fields = DialogFields::from_pairs([
        ("preserve", FieldValue::Text(placement["preserve"].as_str().unwrap_or("transforms").to_owned())),
        ("align", FieldValue::Int(align as i64)),
        ("clip", FieldValue::Bool(placement["clip"].as_bool().unwrap_or(false))),
    ]);
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let dialog = HostedParamDialog::new(&ctx, KEY, fields, |fields| {
        ParamDialog::new(KEY, tl!("Placement Options"), params())
            .values(Bind::bound(fields))
            .footer_note(tl!("Applies when the linked file is relinked or updated."))
    })
    .held_ok();
    crate::dialogs::host::open_held(app, dialog, move |app, result: &ParamResult| {
        if app.session.active().map(|document| document.uid) != source_document {
            return Err(tl!("Placement Options can only change the document where it was opened.").to_owned());
        }
        let f = &result.fields;
        let align = f.int("align").and_then(|i| usize::try_from(i).ok()).and_then(|i| ALIGN.get(i).copied()).unwrap_or(ALIGN[CENTRE]);
        let params = json!({ "ids": ids, "preserve": f.text("preserve"), "align": align, "clip": f.bool("clip").unwrap_or(false) });
        app.run(CMD, params).map(|_| ())
    })
    .ok_or("no window")?;
    Ok(Value::Null)
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{HeldOk, dialog_id};
    use craft_image_codecs::{ChannelLayout, EncodeOptions, Format, Image, SampleType};
    use vectorcraft_engine::Session;

    use super::*;

    #[test]
    fn held_placement_options_keep_the_link_and_undo_in_the_opening_document() {
        let dir = vectorcraft_testkit::TempDir::new("placement-options-document");
        let path = dir.join("linked.png");
        let image = Image::from_raw(2, 2, ChannelLayout::Rgba, SampleType::U8, vec![255; 16]).unwrap();
        std::fs::write(&path, craft_image_codecs::encode(&image, Format::Png, &EncodeOptions::default()).unwrap()).unwrap();
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let link = |app: &mut VectorcraftApp| {
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.run("file.place", json!({"path": path, "at": [100, 100], "link": true})).unwrap()["ids"][0].as_u64().unwrap()
        };
        let source_link = link(&mut app);
        let other_link = link(&mut app);
        assert_eq!(source_link, other_link, "different documents reuse this object ID");
        let settings = |app: &mut VectorcraftApp| {
            let placement = app.run(CMD, json!({"ids": [source_link]})).unwrap()["placement"].clone();
            (placement, app.session.active().unwrap().history.undo.len())
        };
        let other = settings(&mut app);

        app.run("document.activate", json!({"index": 0})).unwrap();
        let starting = settings(&mut app);
        app.run("ui.placementOptionsDialog", json!({"ids": [source_link]})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let dialogs = app.dialogs().unwrap();
        let id = dialog_id(KEY);
        dialogs.with(|host| {
            host.set_json_field(&ctx, id, "preserve", json!("fit")).unwrap();
            assert!(!host.confirm_by_id(&ctx, id));
            assert_eq!(host.held_state(id), Some(HeldOk::Waiting));
        });

        app.run("document.activate", json!({"index": 1})).unwrap();
        crate::dialogs::host::pump(&mut app, &ctx);
        assert_eq!(settings(&mut app), other, "the other link and its undo history are preserved");
        assert!(matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Refused(_))));

        app.run("document.activate", json!({"index": 0})).unwrap();
        assert_eq!(settings(&mut app), starting);
        assert!(crate::dialogs::tests::confirm(&mut app));
        let (placement, steps) = settings(&mut app);
        assert_eq!(placement["preserve"], json!("fit"));
        assert_eq!(steps, starting.1 + 1);
    }
}
