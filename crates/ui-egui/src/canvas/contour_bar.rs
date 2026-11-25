//! The Contour bar: a floating bar above the selection while the Contour tool is active. It has the
//! Contour's distance in a number box, its corner style (Sharp, Round or Square) and Bake, which
//! turns the live Contour into plain paths. It acts through `addon.contour.set` and
//! `effect.expandAppearance`, so everything it does is a command with its own undo step.

use craft::behavior::overlay::{Align, Side};
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::Button;
use craft::ui::canvas::CanvasSpace;
use craft::ui::choice::Segmented;
use craft::ui::contract::overlay::{Anchor, Dismiss, Outside, Overlay, Place};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Env;
use craft::ui::style::SurfaceKind;
use craft::ui::text::NumberField;
use egui::{Ui, vec2};
use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};

use crate::VectorcraftApp;

/// The live effect a Contour is made of.
const EFFECT: &str = "path.offsetPath";
/// The corner styles, as the Offset Path effect and the tool's `corners` option name them.
const SHARP: &str = "sharp";
const ROUND: &str = "round";
const SQUARE: &str = "square";

/// A selected Contour's distance and corner style, read from its Offset Path effect.
struct Standing {
    offset: f64,
    corners: &'static str,
}

fn standing(n: &Node) -> Option<Standing> {
    let e = n.appearance.effects.iter().rev().find(|e| e.id == EFFECT)?;
    let offset = e.params.get("offset").and_then(Value::as_f64).unwrap_or(0.0);
    let corners = match e.params.get("joins").and_then(Value::as_str) {
        Some("round") => ROUND,
        Some("bevel") => SQUARE,
        _ => SHARP,
    };
    Some(Standing { offset, corners })
}

/// The Contour bar, over the selection's top edge (under it when the top is off the canvas).
pub(super) fn contour_bar(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) {
    if app.session.tool_id() != "contour" {
        return;
    }
    let Some(b) = app.selection_bounds() else { return };
    let Some(st) = app.session.active() else { return };
    let shapes: Vec<&Node> = st
        .selection
        .objects
        .iter()
        .filter_map(|id| st.doc.node(*id))
        .filter(|n| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }))
        .collect();
    if shapes.is_empty() {
        return;
    }
    let stand = shapes.first().and_then(|n| standing(n));
    drop(shapes);
    let tool_corners = app.session.tool_options().get("corners").and_then(Value::as_str).unwrap_or(SHARP).to_string();
    let corners = stand.as_ref().map_or_else(|| [SHARP, ROUND, SQUARE].into_iter().find(|c| *c == tool_corners).unwrap_or(SHARP), |s| s.corners);
    let shown = stand.as_ref().map_or(0.0, |s| s.offset);
    // A grown Contour reaches past the outline it was made from: keep the bar clear of it.
    let b = b.inflate(shown.max(0.0), shown.max(0.0));

    let gap = Env::of(ui).tokens.space.gap.section;
    let selection = egui::Rect::from_points(&space.view.quad(b));
    // The gap is part of the anchor, so the bar keeps it above the selection and below it after a flip.
    let hung = selection.expand2(vec2(0.0, gap));
    let area = space.view.rect.shrink(gap);
    let corner_entries = [Entry::new(SHARP, tl!("Sharp")), Entry::new(ROUND, tl!("Round")), Entry::new(SQUARE, tl!("Square"))];

    let mut run: Option<(&str, Value)> = None;
    let mut corner_pick: Option<&'static str> = None;
    let _ = Popover::new("Contour", Bind::given(true))
        .content_surface()
        .overlay(|_| {
            Overlay::floating().modeless().bounds(area).place(Place::anchored(Anchor::Rect(hung), Side::Above, Align::Center).flipping_across())
        })
        .dismiss(Dismiss::new().outside(Outside::Keep))
        .escape(false)
        .show(ui, |ui, _| {
            let _ = Bar::new("Contour", BarKind::Tool).surface(SurfaceKind::Floating).show(ui, |ui, _| {
                let _ = Text::new(tl!("Contour")).show(ui);
                // The distance applies when its edit ends, as one step.
                let distance = NumberField::new(tl!("Distance"), Bind::given(Some(shown))).unit("pt").range(-1e5..=1e5).step(0.5).show(ui);
                if let Some(Some(v)) = distance.edit.events.committed()
                    && (v - shown).abs() > 1e-9
                {
                    run = Some(("addon.contour.set", json!({"offset": v, "corners": corners})));
                }
                let pick = Segmented::new(tl!("Corners"), Bind::given(corners), &corner_entries).show(ui);
                corner_pick = pick.events.committed().copied();
                let bake = Button::new(tl!("Bake")).enabled(stand.is_some()).tooltip(tl!("Turn the Contour into plain paths")).show(ui);
                if bake.clicked() {
                    run = Some(("effect.expandAppearance", json!({})));
                }
            });
        });
    if let Some(id) = corner_pick {
        // The tool keeps the style for the next Contour; a selected Contour changes now.
        let mut result = app.run("tool.setOption", json!({"key": "corners", "value": id}));
        if result.is_ok()
            && let Some(s) = &stand
        {
            result = app.run("addon.contour.set", json!({"offset": s.offset, "corners": id}));
        }
        if let Err(e) = result {
            app.ui.status = e;
        }
    }
    if let Some((cmd, params)) = run
        && let Err(e) = app.run(cmd, params)
    {
        app.ui.status = e;
    }
}
