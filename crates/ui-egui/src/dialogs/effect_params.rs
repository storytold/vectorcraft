//! The rows of an effect's dialog: [`effect_params`] dispatches to the effect's own builder or to
//! [`generic`], and the tables of the values the rows pick from live here.

use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use craft::ui::runtime::env::{LengthUnits, UnitKind};
use serde_json::{Map, Value};
use vectorcraft_color::Color;

use super::{duotone, effect_adjust, effect_rows, form, free_distort, halftone, raster_effects, warp_effect};
use crate::VectorcraftApp;
use crate::canvas::general_lengths;

/// What an effect's rows need from the app, built once for the dialog, the gallery catalog and
/// the Appearance panel's inline editor.
#[derive(Clone)]
pub(crate) struct EffectCx {
    /// The effect's id.
    pub id: String,
    /// The document's general unit and the unit the user sees.
    pub lengths: LengthUnits,
    /// Whether the document is in CMYK (Rasterize's Color Model names it).
    pub cmyk: bool,
    /// Whether the rows edit an applied effect rather than add one.
    pub editing: bool,
    /// The width over the height of the selection (Free Distort's box).
    pub aspect: f64,
    /// The global and spot swatches a Duotone tone can be: (name, colour).
    pub swatches: Vec<(String, Color)>,
}

impl EffectCx {
    /// The context of effect `id` in the active document.
    pub(crate) fn new(app: &VectorcraftApp, id: &str, editing: bool) -> EffectCx {
        let swatches = app
            .session
            .active()
            .map(|st| st.doc.swatches_iter().filter_map(|w| Some((w.name.clone(), st.doc.global_color(&w.name)?))).collect())
            .unwrap_or_default();
        EffectCx {
            id: id.to_string(),
            lengths: general_lengths(app),
            cmyk: app.session.active().is_some_and(|s| s.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk),
            editing,
            aspect: f64::from(free_distort::aspect(app)),
            swatches,
        }
    }
}

/// The Glass effect (Effect › Distort › Glass…).
pub(super) const GLASS: &str = "distort.glass";
/// The Texturizer effect (Effect › Texture › Texturizer…).
pub(super) const TEXTURIZER: &str = "texture.texturizer";
/// The Rough Pastels effect (Effect › Artistic › Rough Pastels…).
pub(super) const ROUGH_PASTELS: &str = "artistic.roughPastels";
/// The Transform effect (Effect › Distort & Transform › Transform…).
pub(super) const TRANSFORM: &str = "distort.transform";
/// Scribble (Effect › Stylize › Scribble…).
pub(super) const SCRIBBLE: &str = "stylize.scribble";
/// The Zig Zag effect (Effect › Distort & Transform › Zig Zag…).
pub(super) const ZIG_ZAG: &str = "distort.zigZag";
/// The Roughen effect (Effect › Distort & Transform › Roughen…).
pub(super) const ROUGHEN: &str = "distort.roughen";
/// The Pucker & Bloat effect (Effect › Distort & Transform › Pucker & Bloat…).
pub(super) const PUCKER_BLOAT: &str = "distort.puckerBloat";
/// The Tweak effect (Effect › Distort & Transform › Tweak…).
pub(super) const TWEAK: &str = "distort.tweak";
/// Drop Shadow (Effect › Stylize › Drop Shadow…).
pub(super) const DROP_SHADOW: &str = "stylize.dropShadow";
/// The Radial Blur effect (Effect › Blur › Radial Blur…).
pub(super) const RADIAL_BLUR: &str = "blur.radial";
/// Film Grain (Effect › Artistic › Film Grain…).
const FILM_GRAIN: &str = "artistic.filmGrain";

/// The rows of effect `cx.id`'s dialog, in the order the effect's own builder gives them. A 3D
/// effect never reaches this from the dialog.
pub(crate) fn effect_params(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let id = cx.id.as_str();
    if warp_effect::is_warp(id) {
        return warp_effect::params(cx, fields);
    }
    if let Some(plugin) = vectorcraft_plugins::effect::installed(id) {
        // With no rows every key passes through: the values the manifest's visibility reads.
        return form::plugin_params(plugin.manifest(), &form::json_of(&[], fields));
    }
    match id {
        _ if vectorcraft_effects::is_adjustment(id) && !vectorcraft_effects::addons::color::is_color_effect(id) => effect_adjust::params(cx, fields),
        vectorcraft_effects::addons::halftone::HALFTONE => halftone::screen_params(true, fields),
        vectorcraft_effects::addons::duotone::DUOTONE => duotone::params(cx, fields),
        GLASS | TEXTURIZER | ROUGH_PASTELS => effect_rows::textured(cx, fields),
        TRANSFORM => effect_rows::transform(cx, fields),
        SCRIBBLE => effect_rows::scribble(cx, fields),
        ZIG_ZAG | ROUGHEN => effect_rows::distort(cx, fields),
        PUCKER_BLOAT => effect_rows::pucker_bloat(cx, fields),
        TWEAK => effect_rows::tweak(cx, fields),
        DROP_SHADOW => effect_rows::drop_shadow(cx, fields),
        FILM_GRAIN => effect_rows::film_grain(cx, fields),
        RADIAL_BLUR => effect_rows::radial_blur(cx, fields),
        free_distort::FREE_DISTORT => free_distort::params(cx, fields),
        vectorcraft_effects::RASTERIZE => raster_effects::params(cx, true),
        _ if vectorcraft_effects::takes_pathfinder_options(id) => effect_rows::pathfinder(cx, fields),
        _ => generic(cx, fields),
    }
}

/// One row per default of a built-in effect without a builder: the JSON type of the default decides
/// the kind, labels use the UI language, and the rows follow the effect's parameter documentation.
pub(super) fn generic(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let id = cx.id.as_str();
    let Some(info) = vectorcraft_effects::effect_info(id) else { return Vec::new() };
    let relative = fields.contains("relative") && effect_rows::is_relative(fields);
    let mut defaults: Vec<(&String, &Value)> = info.defaults.as_object().into_iter().flatten().collect();
    defaults.sort_by_key(|(key, _)| doc_rank(info.params, key));
    defaults
        .into_iter()
        .filter_map(|(key, default)| {
            let mut row = generic_row(cx, key, default, relative)?;
            row.label = tl!(row.label.as_str()).to_owned().into();
            Some(row)
        })
        .collect()
}

/// One row per stored value of an effect that has no rows of its own (a plug-in no longer
/// installed): the JSON type of each value decides the kind, as for [`generic`].
pub(crate) fn stored_params(cx: &EffectCx, values: &Map<String, Value>) -> Vec<Param> {
    let relative = values.get("relative").and_then(Value::as_bool).unwrap_or(false);
    values.iter().filter_map(|(key, value)| generic_row(cx, key, value, relative)).collect()
}

/// The row of parameter `key` whose default is `default`; none for a value no control edits.
fn generic_row(cx: &EffectCx, key: &str, default: &Value, relative: bool) -> Option<Param> {
    let id = cx.id.as_str();
    let label = form::humanize(key);
    let label = label.trim_end_matches(':').to_string();
    let options = || {
        own_choices(id, key, cx.cmyk)
            .map(|own| {
                own.into_iter().map(|(label, value)| (value.to_string(), label.into())).collect::<Vec<(String, craft::ui::runtime::env::Label)>>()
            })
            .or_else(|| {
                choices(id, key).map(|list| list.iter().map(|(label, value)| (value.to_string(), form::choice_label(label).into())).collect())
            })
    };
    Some(match default {
        Value::Bool(on) if key == "relative" => effect_rows::relative_radio(label, *on),
        Value::Bool(on) => Param::new(key, label, ParamKind::bool(*on)),
        Value::Number(n) => {
            let n = n.as_f64()?;
            if vectorcraft_effects::is_length(id, key, relative) {
                Param::number(key, label, n, 1.0, 2).length(cx.lengths)
            } else if let Some(range) = vectorcraft_effects::raster_px_range(id, key) {
                Param::number(key, label, n, 1.0, 0).bounds(Some(*range.start()), Some(*range.end())).unit(UnitKind::Pixels)
            } else if default.is_i64() || default.is_u64() {
                Param::new(key, label, ParamKind::int(None, None, n as i64))
            } else {
                Param::number(key, label, n, 1.0, 2)
            }
        }
        // The blend mode the effect draws in: the mode it draws a name in, Normal for one it doesn't know.
        Value::String(text) if vectorcraft_effects::takes_blend_mode(id, key) => {
            let mode = vectorcraft_effects::blend_mode_id(vectorcraft_effects::blend_mode(text));
            Param::new(key, label, ParamKind::blend(crate::fields::effect_blend_modes(), Some(mode)))
        }
        Value::String(text) => match options() {
            Some(list) => Param::new(key, label, ParamKind::choice(list, Some(text.clone()))),
            None if text.starts_with('#') => Param::new(key, label, ParamKind::color(None)),
            None if text.contains('\n') => Param::new(key, label, ParamKind::text(text.clone())).multiline(6),
            None => Param::new(key, label, ParamKind::text(text.clone())),
        },
        Value::Object(_) => Param::new(key, label, ParamKind::color(None)),
        _ => return None,
    })
}

/// The title of effect `cx.id`'s dialog.
pub(crate) fn heading(cx: &EffectCx) -> String {
    if warp_effect::is_warp(&cx.id) {
        tl!("Warp Options").to_string()
    } else if super::effect::has_only_pathfinder_options(&cx.id) {
        tl!("Pathfinder Options").to_string()
    } else {
        label(&cx.id)
    }
}

/// The effect's name in the UI language as the Effect menu shows it ([`crate::i18n::effect_name`]).
/// A plug-in effect's name is shown as it is.
pub(crate) fn label(id: &str) -> String {
    let name = vectorcraft_effects::effect_info(id).map(|i| i.label.trim_end_matches('…').to_string()).unwrap_or_else(|| id.to_string());
    if vectorcraft_plugins::effect::plugin_id(id).is_some() {
        return name;
    }
    crate::i18n::effect_name(crate::i18n::current(), id).unwrap_or(name)
}

/// Film Grain's bounded, whole-number controls, in the same order as its parameter docs.
pub(crate) const FILM_GRAIN_FIELDS: [(&str, &str, f64); 3] =
    [("grain", "Grain", 20.0), ("highlightArea", "Highlight Area", 20.0), ("intensity", "Intensity", 10.0)];

/// Paint Daubs › Brush Type labels ([`vectorcraft_effects::pixel::PAINT_DAUB_BRUSH_TYPES`]); extend
/// [`crate::menus::menu_strings`] with these like [`FILM_GRAIN_FIELDS`].
pub(crate) const PAINT_DAUB_BRUSH_TYPE_LABELS: [&str; 6] = ["Simple", "Light Rough", "Dark Rough", "Wide Sharp", "Wide Blurry", "Sparkle"];

/// What Drop Shadow's shadow takes its color from (`shade`), as (label, value).
pub(crate) const SHADES: form::Choices = &[("Color", "color"), ("Darkness", "darkness")];

/// Glass's Texture menu, ending with Loaded Texture once a texture is `loaded`.
pub(super) fn glass_textures(loaded: bool) -> form::Choices {
    use vectorcraft_effects::pixel::{GLASS_TEXTURES, GLASS_TEXTURES_LOADED};
    if loaded { &GLASS_TEXTURES_LOADED } else { &GLASS_TEXTURES }
}

/// Texturizer's and Rough Pastels' Texture menu, ending with Loaded Texture once a texture is
/// `loaded`.
pub(super) fn texturizer_textures(loaded: bool) -> form::Choices {
    use vectorcraft_effects::pixel::{TEXTURES, TEXTURES_LOADED};
    if loaded { &TEXTURES_LOADED } else { &TEXTURES }
}

/// The parameters of the effect dialogs that pick one of some values, as (label, value).
pub(super) fn choices(effect: &str, key: &str) -> Option<form::Choices> {
    match (effect, key) {
        ("blur.radial", "method") => Some(&[("Spin", "spin"), ("Zoom", "zoom")]),
        ("blur.radial", "quality") => Some(&[("Draft", "draft"), ("Good", "good"), ("Best", "best")]),
        ("blur.smart", "quality") => Some(&[("Low", "low"), ("Medium", "medium"), ("High", "high")]),
        ("blur.smart", "blurMode") => Some(&vectorcraft_effects::pixel::SMART_BLUR_MODES),
        ("brushStrokes.sprayedStrokes", "strokeDirection") => Some(&vectorcraft_effects::pixel::STROKE_DIRECTIONS),
        ("distort.glass", "texture") => Some(&vectorcraft_effects::pixel::GLASS_TEXTURES),
        ("pixelate.mezzotint", "type") => Some(&vectorcraft_effects::pixel::MEZZOTINT_TYPES),
        ("artistic.paintDaubs", "brushType") => Some(&vectorcraft_effects::pixel::PAINT_DAUB_BRUSH_TYPES),
        ("sketch.graphicPen", "strokeDirection") => Some(&vectorcraft_effects::pixel::STROKE_DIRECTIONS),
        ("sketch.halftonePattern", "patternType") => Some(&vectorcraft_effects::pixel::HALFTONE_TYPES),
        ("sketch.plaster", "lightDirection") => Some(&vectorcraft_effects::pixel::LIGHT_DIRECTIONS),
        ("texture.grain", "grainType") => Some(&vectorcraft_effects::pixel::GRAIN_TYPES),
        ("texture.texturizer" | "artistic.underpainting" | "artistic.roughPastels" | "sketch.conteCrayon", "texture") => {
            Some(&vectorcraft_effects::pixel::TEXTURES)
        }
        ("sketch.basRelief" | "texture.texturizer" | "artistic.underpainting" | "artistic.roughPastels" | "sketch.conteCrayon", "lightDirection") => {
            Some(&vectorcraft_effects::pixel::LIGHT_DIRECTIONS)
        }
        ("video.deinterlace", "eliminate") => Some(&[("Odd Fields", "odd"), ("Even Fields", "even")]),
        ("video.deinterlace", "create") => Some(&[("Duplication", "duplication"), ("Interpolation", "interpolation")]),
        ("stylize.innerGlow", "source") => Some(&[("Center", "center"), ("Edge", "edge")]),
        ("stylize.dropShadow", "shade") => Some(SHADES),
        ("path.offsetPath", "joins") => Some(&[("Miter", "miter"), ("Round", "round"), ("Bevel", "bevel")]),
        ("distort.roughen" | "distort.zigZag", "points") => Some(&[("Smooth", "smooth"), ("Corner", "corner")]),
        _ => crate::addons::pathtools::effect_choices(effect, key),
    }
}

/// The parameters whose dialogs list their values in their own bodies, as (label, value), each
/// label as that dialog passes it to its dropdown or radio buttons, which translate it into the UI
/// language: the edge of both Revolves, the bevel of both Extrude & Bevels (Rounded translated in
/// the "3d" context first), the surface and shading color of the classic 3D effects, every Warp style's
/// orientation, Rasterize's Color Model (its `document` value labeled
/// CMYK when `cmyk`, for a document in CMYK, and RGB otherwise), Background and Anti-aliasing, and
/// the channel of Levels and Curves.
pub(super) fn own_choices(effect: &str, key: &str, cmyk: bool) -> Option<Vec<(&'static str, &'static str)>> {
    let c3 = |s: &'static str| crate::i18n::tr_ctx(crate::i18n::current(), "3d", s);
    Some(match (effect, key) {
        (vectorcraft_effects::REVOLVE | vectorcraft_effects::CLASSIC_REVOLVE, "edge") => vec![(tl!("Left"), "left"), (tl!("Right"), "right")],
        (
            vectorcraft_effects::REVOLVE | vectorcraft_effects::EXTRUDE | vectorcraft_effects::INFLATE | vectorcraft_effects::ROTATE,
            "shadowPosition",
        ) => {
            vec![(tl!("Behind Object"), "behind"), (tl!("Below Object"), "below")]
        }
        (vectorcraft_effects::CLASSIC_EXTRUDE | vectorcraft_effects::CLASSIC_REVOLVE, "surface") => {
            vec![(tl!("Wireframe"), "wireframe"), (tl!("No Shading"), "none"), (tl!("Diffuse"), "diffuse"), (tl!("Plastic"), "plastic")]
        }
        (vectorcraft_effects::CLASSIC_ROTATE, "surface") => vec![(tl!("No Shading"), "none"), (tl!("Diffuse"), "diffuse")],
        (vectorcraft_effects::CLASSIC_EXTRUDE | vectorcraft_effects::CLASSIC_REVOLVE | vectorcraft_effects::CLASSIC_ROTATE, "shadingColor") => {
            vec![(tl!("Black"), "black"), (tl!("Custom"), "custom"), (tl!("None"), "none"), (tl!("Black Overprint"), "blackOverprint")]
        }
        (vectorcraft_effects::EXTRUDE | vectorcraft_effects::CLASSIC_EXTRUDE, "bevel") => vec![
            (tl!("None"), "none"),
            (tl!("Classic"), "classic"),
            (c3("Rounded"), "rounded"),
            (tl!("Cove"), "cove"),
            (tl!("Step"), "step"),
            (tl!("Ogee"), "ogee"),
        ],
        (_, "orientation") if super::warp_effect::is_warp(effect) => vec![(tl!("Horizontal"), "horizontal"), (tl!("Vertical"), "vertical")],
        (vectorcraft_effects::RASTERIZE, "colorModel") => {
            vec![(if cmyk { "CMYK" } else { "RGB" }, "document"), (tl!("Grayscale"), "grayscale"), (tl!("Bitmap"), "bitmap")]
        }
        (vectorcraft_effects::RASTERIZE, "background") => vec![(tl!("White"), "white"), (tl!("Transparent"), "transparent")],
        (vectorcraft_effects::RASTERIZE, "antiAlias") => vectorcraft_render::AntiAlias::ALL.iter().map(|a| (a.label(), a.id())).collect(),
        ("adjust.levels" | "adjust.curves", "channel") => vec![("RGB", "rgb"), (tl!("Red"), "red"), (tl!("Green"), "green"), (tl!("Blue"), "blue")],
        _ => return None,
    })
}

/// Where parameter `key` comes in the effect's parameter documentation `doc` (`{radius: …,
/// threshold: …}`), which lists them in its dialog's order; the end when it isn't there.
pub(super) fn doc_rank(doc: &str, key: &str) -> usize {
    let starts = |at: usize| doc.get(..at).and_then(|s| s.chars().last()).is_some_and(|c| c == '{' || c == ' ');
    doc.match_indices(key)
        .map(|(at, _)| at)
        .find(|at| starts(*at) && doc.get(at + key.len()..).is_some_and(|rest| rest.starts_with(':') || rest.starts_with("?:")))
        .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests {
    use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, pos2, vec2};
    use serde_json::json;
    use vectorcraft_doc::NodeId;
    use vectorcraft_engine::Session;

    use crate::VectorcraftApp;

    #[test]
    fn gallery_cutout_setting_uses_its_localized_label() {
        let viewport = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 500.0));
        let mut shown_labels = Vec::new();
        for (language, expected) in [("en", "Levels"), ("de", "Stufen"), ("ja", "レベル数")] {
            let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
            app.session.prefs.interface_language = language.to_owned();
            let lang = app.ui_language();
            let ctx = app.ui_ctx.clone().unwrap();
            let mut time = 0.0;
            let mut frame = |app: &mut VectorcraftApp, events: Vec<Event>| {
                ctx.enable_accesskit();
                time += 0.1;
                crate::testing::app_frame(app, &ctx, RawInput { events, time: Some(time), screen_rect: Some(viewport), ..Default::default() })
            };
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let square = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
            frame(&mut app, vec![]);
            assert_eq!(ctx.content_rect(), viewport);
            app.run("ui.effectGalleryDialog", json!({})).unwrap();
            frame(&mut app, vec![]);
            let nodes = frame(&mut app, vec![]).platform_output.accesskit_update.unwrap().nodes;
            let filter = crate::i18n::effect_name(lang, "artistic.cutout").unwrap();
            let bounds = nodes.iter().filter(|(_, node)| node.label() == Some(filter.as_str())).find_map(|(_, node)| node.bounds()).unwrap();
            let at = pos2((bounds.x0 + bounds.x1) as f32 * 0.5, (bounds.y0 + bounds.y1) as f32 * 0.5);
            assert!(viewport.contains(at), "{language}: Cutout is reachable");
            frame(&mut app, vec![Event::PointerMoved(at)]);
            for pressed in [true, false] {
                frame(&mut app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
            }
            frame(&mut app, vec![]);
            let output = frame(&mut app, vec![]);
            let state = app.session.active().unwrap();
            let affected = &state.doc.node(NodeId(square)).unwrap().appearance.effects;
            assert_eq!(affected.first().map(|effect| effect.id.as_str()), Some("artistic.cutout"), "{language}: Cutout is previewed");

            let nodes = output.platform_output.accesskit_update.unwrap().nodes;
            let found = [expected, "Levels"].into_iter().find(|label| {
                nodes
                    .iter()
                    .any(|(_, node)| node.label() == Some(*label) && node.bounds().is_some_and(|b| viewport.contains(pos2(b.x0 as f32, b.y0 as f32))))
            });
            eprintln!("{language}: expected {expected:?}, found {found:?}");
            shown_labels.push(found);
        }
        assert_eq!(shown_labels, [Some("Levels"), Some("Stufen"), Some("レベル数")]);
    }
}
