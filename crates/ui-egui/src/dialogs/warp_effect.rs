//! Effect > Warp rows and the Style row Envelope Warp shares. The style remains an effect ID; Envelope Warp has a
//! different schema.
use std::sync::LazyLock;

use craft::ui::contract::{Bind, Entry, MemoProps, Show as _};
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamKind};
use craft::ui::pickers::Select;
use craft::ui::primitives::{Glyph, IconSpec};
use craft::ui::runtime::env::{Label, UnitKind};
use craft::ui::style::UiTokens;
use craft::ui::value_drag::SliderScale;
use egui::{Color32, Painter, Pos2, Rect, Stroke};
use vectorcraft_effects::{WARP_STYLES, WarpNet, WarpStyle};

use super::effect_params::{EffectCx, own_choices};

/// The field a Style pick records that the user chose a style (the Warp effect sends `warpStyle` only then).
const STYLE_CHOSEN: &str = "styleChosen";

pub(super) fn is_warp(id: &str) -> bool {
    id.strip_prefix("warp.").and_then(WarpStyle::from_id).is_some()
}

pub(super) fn shown(label: &str) -> String {
    tl!(label).trim_end_matches('…').to_string()
}

/// A row label from its translated text: the form adds the colon.
pub(super) fn row_label(translated: &str) -> Label {
    Label::from(translated.trim_end_matches(':').to_string())
}

/// A -100 to 100 % slider row.
pub(super) fn percent_param(key: &str, label: Label, default: f64) -> Param {
    Param::new(key, label, ParamKind::range(-100.0, 100.0, default, SliderScale::Linear)).unit(UnitKind::Percent)
}

/// An original three-by-three grid, bent by VectorCraft's own warp map. No reference icon artwork.
#[derive(MemoProps)]
pub(super) struct WarpGlyph(&'static str);

impl Glyph for WarpGlyph {
    fn paint(&self, painter: &Painter, _tokens: &UiTokens, rect: Rect, color: Color32, _mirrored: bool) {
        let Some(style) = WarpStyle::from_id(self.0) else { return };
        // The map new warps take, on a box 0.55 times as high as it is wide.
        let net = WarpNet::new(style, 0.45, 0.0, 0.0, 2.0, 1.1, false);
        let mut lines = vec![];
        let mut bounds = Rect::NOTHING;
        for vertical in [false, true] {
            for fixed in [-1.0, 0.0, 1.0] {
                let line: Vec<Pos2> = (0..=16)
                    .map(|i| {
                        let along = -1.0 + f64::from(i) / 8.0;
                        let (x, y) = if vertical { (fixed, along) } else { (along, fixed) };
                        let q = net.at((x + 1.0) / 2.0, (y + 1.0) / 2.0);
                        let p = Pos2::new(q.x as f32, q.y as f32);
                        bounds.extend_with(p);
                        p
                    })
                    .collect();
                lines.push(line);
            }
        }
        let room = rect.shrink(2.0);
        let scale = (room.width() / bounds.width().max(0.001)).min(room.height() / bounds.height().max(0.001));
        let stroke = Stroke::new(1.0, color);
        for line in lines {
            let points = line.into_iter().map(|p| room.center() + (p - bounds.center()) * scale).collect();
            painter.add(egui::Shape::line(points, stroke));
        }
    }
}

/// One glyph per entry of `WARP_STYLES`, in its order.
static WARP_GLYPHS: LazyLock<Vec<WarpGlyph>> = LazyLock::new(|| WARP_STYLES.iter().map(|&(style, _)| WarpGlyph(style)).collect());

/// The entries of a Style select, one per warp style; `icons` adds the grid glyph of the style.
pub(super) fn style_entries(icons: bool) -> Vec<Entry<&'static str>> {
    WARP_STYLES
        .iter()
        .zip(WARP_GLYPHS.iter())
        .map(|(&(style, label), glyph)| {
            let entry = Entry::new(style, Label::from(shown(label)));
            if icons { entry.icon(IconSpec::glyph(glyph)) } else { entry }
        })
        .collect()
}

/// The `WARP_STYLES` suffix `id` names; the first style for an id that names none.
pub(super) fn static_style(id: &str) -> &'static str {
    WARP_STYLES.iter().find(|(style, _)| *style == id).or(WARP_STYLES.first()).map_or("arc", |(style, _)| *style)
}

/// The Style row: a select over the warp styles, each with its grid glyph (the closed face shows the held one). A pick
/// writes the style suffix under `key` and `true` under `styleChosen`, so a mixed selection changes style only once one
/// was chosen. `current` is the style the row opens on.
pub(crate) fn style_param(key: &str, current: &str) -> Param {
    let field = key.to_owned();
    Param::custom(key, row_label(tl!("Style:")), move |ui, fields, _| {
        let held = static_style(fields.text(&field).unwrap_or_default());
        let entries = style_entries(true);
        let out = Select::new(row_label(tl!("Style:")), Bind::given(held), &entries).show(ui);
        if let Some(&style) = out.events.committed().filter(|&&style| style != held) {
            fields.set(field.as_str(), style);
            fields.set(STYLE_CHOSEN, true);
        }
        out.response
    })
    .companions([STYLE_CHOSEN])
    .opens_with(static_style(current))
}

/// The rows of the Warp effect dialog.
pub(super) fn params(cx: &EffectCx, fields: &DialogFields) -> Vec<Param> {
    let current = fields.text("style").unwrap_or_else(|| cx.id.strip_prefix("warp.").unwrap_or_default());
    let orientation = own_choices(&cx.id, "orientation", false)
        .unwrap_or_default()
        .into_iter()
        .map(|(label, value)| (value.to_owned(), Label::from(tl!(label).to_string())))
        .collect();
    vec![
        style_param("style", current),
        Param::new("orientation", Label::Empty, ParamKind::radio(orientation, None)),
        percent_param("bend", row_label(tl!("Bend:")), 50.0),
        Param::section(row_label(tl!("Distortion"))),
        percent_param("horizontal", row_label(tl!("Horizontal:")), 0.0),
        percent_param("vertical", row_label(tl!("Vertical:")), 0.0),
    ]
}
