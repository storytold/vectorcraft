//! The Free Transform widget (#598): while the Free Transform tool is active, a small floating box
//! at the canvas's top left with Constrain (as holding Shift: proportional scaling, moves and
//! rotations by 45°, and Free Distort corners kept on their diagonal) over the tool's three modes,
//! Free Transform, Perspective Distort and Free Distort. It sets the tool's options through `tool.setOption` (`constrain`, `mode`), as agents
//! do; the modifier keys keep working while dragging.

use craft::behavior::IconId;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::buttons::ToggleButton;
use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::overlay::Overlay;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::display::Divider;
use craft::ui::popover::Popover;
use craft::ui::runtime::env::Env;
use craft::ui::style::SurfaceKind;
use egui::{Ui, Vec2};
use serde_json::{Value, json};
use std::num::NonZeroU8;

use crate::VectorcraftApp;

/// The tool the widget belongs to.
const TOOL: &str = "freeTransform";

/// The tool's modes as its `mode` option names them, with their icons and tooltips.
fn modes() -> [(&'static str, &'static str, &'static str); 3] {
    [
        ("free", "dc-free-transform", tl!("Free Transform")),
        ("perspective", "dc-perspective", tl!("Perspective Distort")),
        ("distort", "dc-free-distort", tl!("Free Distort")),
    ]
}

/// Show the widget at the top left of the canvas while the Free Transform tool is active.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut Ui, space: &CanvasSpace) {
    if app.session.tool_id() != TOOL {
        return;
    }
    let opts = app.session.tool_options();
    let mode = opts["mode"].as_str().unwrap_or("free").to_string();
    let constrain = opts["constrain"].as_bool().unwrap_or(false);
    let canvas = space.view.rect;
    let at = canvas.min + Vec2::splat(Env::of(ui).tokens.space.gap.section);
    let mut set: Option<(&str, Value)> = None;
    let _ = Popover::new("Free Transform", Bind::given(true))
        .content_surface()
        .overlay(|_| Overlay::floating().modeless().bounds(canvas).placed_at(at))
        .show(ui, |ui, _| {
            let _ =
                Bar::new("Free Transform", BarKind::Tool).surface(SurfaceKind::Floating).rows(NonZeroU8::MIN.saturating_add(3)).show(ui, |ui, _| {
                    // Constrain shapes what the Free Transform and Free Distort modes do.
                    let toggle = ToggleButton::new(tl!("Constrain"), Bind::given(constrain))
                        .icon(IconId::new("link"))
                        .icon_only()
                        .tooltip(tl!("Constrain"))
                        .enabled(mode != "perspective")
                        .show(ui);
                    if let Some(on) = toggle.events.committed() {
                        set = Some(("constrain", json!(on)));
                    }
                    let _ = Divider::horizontal().show(ui);
                    for (name, icon, tip) in modes() {
                        let button = ToggleButton::new(tip, Bind::given(mode == name)).icon(IconId::new(icon)).icon_only().tooltip(tip).show(ui);
                        // A click on the mode that is on turns it off in the button; the option stays.
                        if button.events.committed() == Some(&true) && mode != name {
                            set = Some(("mode", json!(name)));
                        }
                    }
                });
        });
    if let Some((key, value)) = set
        && let Err(e) = app.run("tool.setOption", json!({ "key": key, "value": value }))
    {
        app.status(e);
    }
}
