//! Edit → Edit Colors → Convert to Indexed Color: the palette (reduction and colors) and the
//! dither of the selection's embedded images, previewed live on the canvas; OK keeps the preview
//! as one undo step (`edit.colors.toIndexed`).
//!
//! Fields: `reduction`, `colors`, `dither`, `ditherAmount` and `preview`.

use std::sync::Arc;

use craft::ui::contract::Bind;
use craft::ui::dialog::dialog_id;
use craft::ui::patterns::form_dialogs::{DialogFields, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::UnitKind;
use serde_json::{Value, json};
use vectorcraft_render::encode::quantize::{Dither, Reduction};

use super::host::{HeldOptions, open_held_with};
use crate::VectorcraftApp;
use crate::dialogs::preview::LivePreview;

/// The dialog kind of Convert to Indexed Color.
pub const KIND: &str = "indexedColor";

const CMD: &str = "edit.colors.toIndexed";
/// The undo step a preview and OK make.
const PREVIEW_LABEL: &str = "Convert to Indexed Color";

pub(crate) struct Open {
    document: u64,
    reverts: u64,
    opening: Arc<()>,
    preview: LivePreview,
}

impl Open {
    fn on_original_drawing(&self, app: &VectorcraftApp) -> bool {
        self.check_drawing(app).is_ok()
    }

    fn check_drawing(&self, app: &VectorcraftApp) -> Result<(), &'static str> {
        let st = app.session.active().filter(|st| st.uid == self.document).ok_or(tl!("Return to the original drawing to convert its images"))?;
        if st.reverts != self.reverts {
            return Err(tl!("Reopen Convert to Indexed Color after reverting the drawing"));
        }
        Ok(())
    }

    fn cancel_preview(self, app: &mut VectorcraftApp) {
        if self.on_original_drawing(app) && !other_preview(app) {
            self.preview.cancel(app);
        }
    }
}

fn other_preview(app: &VectorcraftApp) -> bool {
    app.session.active().and_then(|st| st.interaction.as_ref()).is_some_and(|p| p.label != PREVIEW_LABEL)
}

/// The reductions besides those of a PNG-8 export, as (value, label).
const MORE_REDUCTIONS: [(&str, &str); 2] = [("uniform", "Uniform"), ("document", "Document Swatches")];

/// The dialog's rows: colors only for a reduction that takes them, the amount only while dithering.
fn rows() -> Vec<Param> {
    let reductions = Reduction::ALL.iter().map(|r| (r.id(), r.label())).chain(MORE_REDUCTIONS).collect::<Vec<_>>();
    let with_colors: Vec<String> = reductions.iter().filter(|(id, _)| takes_colors(id)).map(|(id, _)| id.to_string()).collect();
    let dithers = Dither::ALL.iter().filter(|d| **d != Dither::None).map(|d| d.id().to_string()).collect::<Vec<_>>();
    vec![
        Param::new(
            "reduction",
            tl!("Reduction"),
            ParamKind::choice(reductions.iter().map(|(id, l)| (id.to_string(), (*l).into())).collect(), Some("selective".into())),
        ),
        Param::new("colors", tl!("Colors"), ParamKind::int(Some(least_colors("") as i64), Some(256), 256)).visible_when_in("reduction", with_colors),
        Param::new(
            "dither",
            tl!("Dither"),
            ParamKind::choice(Dither::ALL.iter().map(|d| (d.id().to_string(), d.label().into())).collect(), Some("diffusion".into())),
        ),
        Param::new("ditherAmount", tl!("Amount"), ParamKind::int(Some(0), Some(100), 100)).unit(UnitKind::Percent).visible_when_in("dither", dithers),
    ]
}

/// The dialog over its values; the colors show what the command gets: at least the reduction's fewest.
fn build(fields: &mut DialogFields) -> ParamDialog<'_> {
    if let (Some(reduction), Some(n)) = (fields.text("reduction"), fields.int("colors")) {
        let clamped = n.clamp(least_colors(reduction) as i64, 256);
        if clamped != n {
            fields.set("colors", clamped);
        }
    }
    ParamDialog::new(KIND, tl!("Convert to Indexed Color"), rows()).values(Bind::bound(fields)).previews(true)
}

/// Open the dialog with the defaults of `edit.colors.toIndexed`.
pub fn open(app: &mut VectorcraftApp) {
    let Some(ctx) = app.ui_ctx.clone() else {
        app.status("no window".to_string());
        return;
    };
    let Some((document, reverts)) = app.session.active().map(|st| (st.uid, st.reverts)) else {
        app.status(tl!("Open a drawing to convert its images"));
        return;
    };
    if let Some(old) = app.object_dialogs.indexed.take() {
        old.cancel_preview(app);
    }
    let dialog = HostedParamDialog::new(&ctx, KIND, DialogFields::new(), build);
    let opening = Arc::new(());
    let cancelled = Arc::clone(&opening);
    let ok = |app: &mut VectorcraftApp, r: &ParamResult| {
        let open = app.object_dialogs.indexed.as_ref().ok_or("the Indexed Color dialog is gone")?;
        open.check_drawing(app)?;
        if other_preview(app) {
            app.keep_typing()?;
            if other_preview(app) {
                return Err(tl!("Finish the current edit before converting its images").into());
            }
        }
        let open = app.object_dialogs.indexed.take().ok_or("the Indexed Color dialog is gone")?;
        let done = open.preview.commit(app, CMD, params(&r.fields)).map(|_| ());
        if done.is_err() {
            // The dialog stays open and previews again.
            app.object_dialogs.indexed = Some(Open { preview: LivePreview::default(), ..open });
        }
        done
    };
    let cancel = Box::new(move |app: &mut VectorcraftApp| {
        if app.object_dialogs.indexed.as_ref().is_some_and(|open| Arc::ptr_eq(&open.opening, &cancelled))
            && let Some(open) = app.object_dialogs.indexed.take()
        {
            open.cancel_preview(app);
        }
    });
    if open_held_with(app, dialog, ok, HeldOptions { parent: None, cancel: Some(cancel) }).is_some() {
        app.object_dialogs.indexed = Some(Open { document, reverts, opening, preview: LivePreview::default() });
    } else {
        app.status("no window".to_string());
    }
}

/// Each frame: show the dialog's values on the canvas while its Preview box is on.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.object_dialogs.indexed.take() else { return };
    if !open.on_original_drawing(app) {
        app.object_dialogs.indexed = Some(open);
        return;
    }
    let shown = app
        .dialogs()
        .and_then(|h| h.with(|host| host.get_by_id::<HostedParamDialog>(dialog_id(KIND)).map(|d| (d.is_previewing(), params(d.fields())))));
    if let Some((on, p)) = &shown {
        if *on
            && other_preview(app)
            && let Err(e) = app.keep_typing()
        {
            app.status(e);
        }
        if !other_preview(app) {
            let _ = open.preview.run(app, *on, PREVIEW_LABEL, CMD, p);
        }
    }
    app.object_dialogs.indexed = Some(open);
}

/// Whether reduction `r` takes a number of colors.
fn takes_colors(r: &str) -> bool {
    !matches!(r, "blackWhite" | "document")
}

/// The fewest colors reduction `r` takes.
fn least_colors(r: &str) -> f64 {
    if r == "uniform" { 9.0 } else { 2.0 }
}

/// `edit.colors.toIndexed` parameters: the reduction, its colors when it takes them, and the dither.
fn params(fields: &DialogFields) -> Value {
    let reduction = fields.text("reduction").unwrap_or("selective");
    let dither = fields.text("dither").unwrap_or("diffusion");
    let mut p = json!({"reduction": reduction, "dither": dither});
    if takes_colors(reduction) {
        p["colors"] = json!(fields.number("colors").unwrap_or(256.0).round().clamp(least_colors(reduction), 256.0) as u16);
    }
    if dither != Dither::None.id() {
        p["ditherAmount"] = json!(fields.number("ditherAmount").unwrap_or(100.0).round().clamp(0.0, 100.0));
    }
    p
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{panel_harness::Panel, picks::dialogs};
    use craft::ui::contract::Role;
    use std::collections::BTreeSet;
    use vectorcraft_engine::Session;

    fn add_raster(app: &mut VectorcraftApp) -> vectorcraft_doc::NodeId {
        add_raster_row(app, &[[200, 30, 90, 255], [20, 40, 220, 255]])
    }

    fn add_raster_row(app: &mut VectorcraftApp, rgba: &[[u8; 4]]) -> vectorcraft_doc::NodeId {
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        let width = u32::try_from(rgba.len()).unwrap();
        let img = craft_image_codecs::Image::from_raw(
            width,
            1,
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            rgba.as_flattened().to_vec(),
        )
        .unwrap();
        let png = craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
        app.session
            .edit("Place", |d, sel| {
                d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png));
                let id = d.alloc_id();
                let im = vectorcraft_doc::ImageObject {
                    key: "pic".into(),
                    width,
                    height: 1,
                    xf: Default::default(),
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                };
                d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(im))))?;
                sel.set([id]);
                Ok(id)
            })
            .unwrap()
    }

    fn palette(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> &vectorcraft_doc::ImageBlob {
        let st = app.session.doc().unwrap();
        let vectorcraft_doc::NodeKind::Image(im) = &st.doc.node(id).unwrap().kind else { panic!("not an image") };
        &st.doc.images[&im.key]
    }

    #[test]
    fn indexed_color_form_edits_update_the_preview_and_confirmation() {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        let id = add_raster_row(&mut app, &[[200, 30, 90, 255], [20, 40, 220, 255], [220, 240, 70, 255]]);
        let original = palette(&app, id).rgba().unwrap().2;
        let undo = app.session.doc().unwrap().history.undo.len();
        let mut panel = Panel::new(app, dialogs);
        let black_and_white = |panel: &mut Panel| {
            panel.click_role("Reduction", Role::COMBO_BOX);
            panel.click("Black & White");
            panel.click_role("Dither", Role::COMBO_BOX);
            panel.click("No Dither");
        };

        panel.app().run("ui.indexedColorDialog", json!({})).unwrap();
        panel.frame();
        panel.click_at(panel.rect_role("Colors", Role::SPIN_BUTTON).center());
        panel.key(egui::Key::A, egui::Modifiers::COMMAND);
        panel.type_text("2");
        panel.key(egui::Key::Tab, egui::Modifiers::NONE);
        let preview = palette(&panel.app(), id).rgba().unwrap().2;
        assert_eq!(preview.as_chunks::<4>().0.iter().collect::<BTreeSet<_>>().len(), 2);

        black_and_white(&mut panel);
        let preview = palette(&panel.app(), id).rgba().unwrap().2;
        assert_eq!(preview, [0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255, 255]);
        panel.click("Cancel");
        assert_eq!(palette(&panel.app(), id).rgba().unwrap().2, original);
        assert_eq!(panel.app().session.doc().unwrap().history.undo.len(), undo);

        panel.app().run("ui.indexedColorDialog", json!({})).unwrap();
        black_and_white(&mut panel);
        panel.click("OK");
        assert_eq!(palette(&panel.app(), id).rgba().unwrap().2, preview);
        assert_eq!(panel.app().session.doc().unwrap().history.undo.len(), undo + 1);
    }

    #[test]
    fn commits_the_chosen_palette_as_one_step() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let id = add_raster(&mut app);
        let undo = app.session.doc().unwrap().history.undo.len();
        app.run("ui.indexedColorDialog", json!({})).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let dialog = host.with(|h| h.open_dialogs().into_iter().next()).expect("the dialog is open").id;
        for (field, value) in [("reduction", "blackWhite"), ("dither", "none")] {
            host.with(|h| h.set_json_field(&ctx, dialog, field, json!(value))).unwrap();
        }
        // OK files the held answer, which the pump applies.
        host.with(|h| h.confirm_by_id(&ctx, dialog));
        crate::dialogs::host::pump(&mut app, &ctx);
        let st = app.session.doc().unwrap();
        assert_eq!(st.history.undo.len(), undo + 1);
        let blob = palette(&app, id);
        assert!(blob.indexed_png());
        assert_eq!(blob.rgba().unwrap().2, [0, 0, 0, 255, 0, 0, 0, 255], "black and white only");
    }

    #[test]
    fn indexed_color_preview_and_ok_leave_the_other_drawings_pixels_unchanged() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        let source = add_raster(&mut app);
        open(&mut app);
        let ctx = app.ui_ctx.clone().unwrap();
        let host = app.dialogs().unwrap();
        let id = dialog_id(KIND);
        host.with(|h| h.set_json_field(&ctx, id, "reduction", json!("blackWhite"))).unwrap();
        let other = add_raster(&mut app);
        host.with(|h| h.set_json_field(&ctx, id, "dither", json!("none"))).unwrap();
        let undo = app.session.active().unwrap().history.undo.len();
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(!palette(&app, other).indexed_png(), "the other drawing does not show the preview");
        host.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(!palette(&app, other).indexed_png(), "the other drawing does not keep the preview");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);

        app.run("document.activate", json!({"index": 0})).unwrap();
        host.with(|h| h.confirm_by_id(&ctx, id));
        crate::dialogs::host::pump(&mut app, &ctx);
        assert!(palette(&app, source).indexed_png());
    }

    #[test]
    fn indexed_color_preview_and_ok_leave_reloaded_pixels_unchanged() {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.services.read = Some(Box::new(|path| std::fs::read(path).map_err(|e| e.to_string())));
        app.services.write = Some(Box::new(|path, bytes| std::fs::write(path, bytes).map_err(|e| e.to_string())));
        let dir = vectorcraft_testkit::TempDir::new("indexed-color-image-revert");
        let image = add_raster(&mut app);
        app.run("file.saveAs", json!({"path": dir.join("raster.vectorcraft")})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        app.run("select.set", json!({"ids": [image.0]})).unwrap();
        open(&mut app);
        let st = app.session.active().unwrap();
        let version = (st.uid, st.reverts);
        app.run("file.revert", json!({"confirmed": true})).unwrap();
        let st = app.session.active().unwrap();
        assert_eq!(st.uid, version.0);
        assert_ne!(st.reverts, version.1);
        let undo = st.history.undo.len();
        app.run("select.set", json!({"ids": [image.0]})).unwrap();
        let before = palette(&app, image).rgba().unwrap().2;

        let id = dialog_id(KIND);
        crate::tests::picks::answer_dialog(
            &mut app,
            id,
            &[("reduction", json!("blackWhite")), ("dither", json!("none")), ("preview", json!(true))],
            false,
        );
        assert_eq!(palette(&app, image).rgba().unwrap().2, before, "old Preview leaves the saved pixels in place");
        crate::tests::picks::answer_dialog(&mut app, id, &[("preview", json!(false))], true);
        assert_eq!(palette(&app, image).rgba().unwrap().2, before, "old OK leaves the saved pixels in place");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
    }
}
