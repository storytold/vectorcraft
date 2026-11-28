use super::status_bar::ANIMATED_ZOOM_HINT;
use super::{Cow, Ui, VectorcraftApp, menus};
use craft::behavior::IconId;
use craft::theme::ControlSize;
use craft::ui::bars::{Bar, BarKind};
use craft::ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft::ui::display::{Elide, Icon, Text};
use craft::ui::primitives::IconSpec;
use craft::ui::style::Span;

// PLAN: assumed `SlotKey` is a `&'static str` alias exported by the framework's layout module (hub-w1's slots).
/// The key of the bottom slot that holds the hint bar.
pub const HINT_SLOT: craft::ui::runtime::env::SlotKey = "hints";

fn hint_for(tool: &str) -> Option<&'static [(&'static str, bool)]> {
    Some(match tool {
        "dimension" => &[
            ("Drag", true),
            (" out from a highlighted edge to place its dimension  |  ", false),
            ("Click", true),
            (" two points, then move and click to place an aligned, horizontal or vertical dimension  |  ", false),
            ("Alt+Click", true),
            (" a path, then move and click to place its overall width or height  |  ", false),
            ("Esc", true),
            (" cancels  |  Double-click the tool for options", false),
        ],
        "selection" => &[
            ("Click", true),
            (" the object to select  |  ", false),
            ("Shift+Click", true),
            (" to select multiple objects  |  ", false),
            ("Alt+Drag", true),
            (" the object to duplicate", false),
        ],
        "directSelection" => &[
            ("Click", true),
            (" an anchor or path segment to select it  |  ", false),
            ("Drag", true),
            (" to move  |  ", false),
            ("Shift+Click", true),
            (" to add", false),
        ],
        "groupSelection" => &[
            ("Click", true),
            (" an object in a group to select it  |  ", false),
            ("Click again", true),
            (" to add the group it is in  |  ", false),
            ("Shift+Click", true),
            (" to add", false),
        ],
        "pen" => &[
            ("Click", true),
            (" to add a corner point  |  ", false),
            ("Drag", true),
            (" to add a smooth point  |  ", false),
            ("Click the first point", true),
            (" to close  |  ", false),
            ("Enter", true),
            (" to finish", false),
        ],
        "curvature" => &[
            ("Click", true),
            (" to add smooth points  |  ", false),
            ("Double-click", true),
            (" to toggle corner  |  ", false),
            ("Esc", true),
            (" to finish", false),
        ],
        "type" => &[
            ("Click", true),
            (" to create point type  |  ", false),
            ("Drag", true),
            (" to create area type  |  ", false),
            ("Esc", true),
            (" to exit editing", false),
        ],
        "rectangle" | "roundedRectangle" | "ellipse" | "polygon" | "star" => &[
            ("Drag", true),
            (" to draw  |  ", false),
            ("Shift+Drag", true),
            (" to constrain proportions  |  ", false),
            ("Alt+Drag", true),
            (" from center  |  ", false),
            ("Click", true),
            (" for exact size", false),
        ],
        "rotate" | "reflect" | "scale" | "shear" => &[
            ("Click", true),
            (" to set the reference point  |  ", false),
            ("Drag", true),
            (" to transform  |  ", false),
            ("Alt+Click", true),
            (" for exact values", false),
        ],
        "hand" => &[("Drag", true), (" to pan the view", false)],
        "zoom" => &[
            ("Click", true),
            (" to zoom in  |  ", false),
            ("Alt+Click", true),
            (" to zoom out  |  ", false),
            ("Drag", true),
            (" to zoom into an area", false),
        ],
        ANIMATED_ZOOM_HINT => &[
            ("Click", true),
            (" to zoom in  |  ", false),
            ("Alt+Click", true),
            (" to zoom out  |  ", false),
            ("Drag", true),
            (" right or left to zoom in or out  |  ", false),
            ("Hold", true),
            (" to keep zooming", false),
        ],
        "eyedropper" => &[
            ("Click", true),
            (" an object to copy its attributes  |  ", false),
            ("Alt+Click", true),
            (" to apply the selection's to it  |  ", false),
            ("Shift+Click", true),
            (" to sample a color", false),
        ],
        "gradient" => &[("Drag", true), (" across a selected object to set the gradient direction", false)],
        "artboard" => &[
            ("Click", true),
            (" to select an artboard  |  ", false),
            ("Drag", true),
            (" on the canvas to create one (", false),
            ("Shift", true),
            (" selects several, or draws one inside an artboard)", false),
        ],
        "width" => &[
            ("Drag", true),
            (" a stroke to add a width point  |  ", false),
            ("Alt+Drag", true),
            (" to change one side  |  ", false),
            ("Shift+Drag", true),
            (" a point to move the profile, a handle to scale it  |  ", false),
            ("Alt+Shift+Drag", true),
            (" to scale one side of the profile  |  ", false),
            ("Shift+Click", true),
            (" to select more points  |  ", false),
            ("Double-click", true),
            (" a point to edit it  |  ", false),
            ("Delete", true),
            (" to remove points", false),
        ],
        "puppetWarp" => &[
            ("Click", true),
            (" the art to add a pin  |  ", false),
            ("Shift+Click", true),
            (" to select more pins  |  ", false),
            ("Drag", true),
            (" a pin to warp  |  ", false),
            ("Alt+Drag", true),
            (" near a selected pin to rotate  |  ", false),
            ("Delete", true),
            (" to remove pins", false),
        ],
        "warp" => &[
            ("Drag", true),
            (" across paths to push them along  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "twirl" | "pucker" | "bloat" => &[
            ("Click or drag", true),
            (" over paths, and hold still to keep going  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "scallop" | "crystallize" | "wrinkle" => &[
            ("Click or drag", true),
            (" over paths to roughen their outlines  |  ", false),
            ("Alt+Drag", true),
            (" to size the brush (", false),
            ("Shift", true),
            (" keeps its proportions)", false),
        ],
        "perspectiveSelection" => &[
            ("Drag", true),
            (" to move in perspective  |  ", false),
            ("Alt+Drag", true),
            (" to copy  |  ", false),
            ("5", true),
            (" while dragging to move perpendicular to the plane  |  ", false),
            ("Drag a handle", true),
            (" to scale in perspective", false),
        ],
        "blend" => &[
            ("Click", true),
            (" an object, then another to blend them  |  ", false),
            ("Click an anchor point", true),
            (" to blend from it  |  ", false),
            ("Alt+Click", true),
            (" to set spacing and orientation", false),
        ],
        _ => return None,
    })
}

/// The hint bar's segments for `tool`, with keys named as the menus name them (Alt and Ctrl on
/// Windows and Linux, ⌥ and ⌘ on macOS). Tools without a hint point to Search Commands.
/// Every hint-bar fragment (the action after the last `+` of a chord included), so the catalog tests
/// can insist a complete language covers them.
pub fn hint_strings() -> Vec<&'static str> {
    let mut v = Vec::new();
    for tool in vectorcraft_tools::catalog::all_tools().map(|t| t.id).chain([ANIMATED_ZOOM_HINT]) {
        for &(text, bold) in hint_for(tool).unwrap_or(&[]) {
            let text = if bold { text.rsplit_once('+').map_or(text, |(_, k)| k) } else { text };
            if !text.is_empty() && !text.chars().all(|c| c.is_ascii_digit()) && !v.contains(&text) {
                v.push(text);
            }
        }
    }
    v
}

pub(super) fn hint_segments(app: &VectorcraftApp, tool: &str) -> Vec<(Cow<'static, str>, bool)> {
    if let Some(segments) = hint_for(tool) {
        return shown_segments(segments);
    }
    match menus::shortcut_of(app, "help.commandPalette") {
        Some(chord) => vec![
            (Cow::Borrowed(tl!("Press ")), false),
            (Cow::Owned(chord.display(crate::keymap::platform())), true),
            (Cow::Borrowed(tl!(" to search every command")), false),
        ],
        None => shown_segments(&[(tl!("Use "), false), (tl!("Help › Search Commands…"), true), (tl!(" to search every command"), false)]),
    }
}

/// The hint bar's segments for `tool` as [`hint_segments`] shows them, the Zoom tool's for animated
/// zoom when `animated_zoom` is on; none for a tool without a hint of its own.
pub(crate) fn tool_hint(tool: &str, animated_zoom: bool) -> Option<Vec<(Cow<'static, str>, bool)>> {
    let tool = if tool == "zoom" && animated_zoom { ANIMATED_ZOOM_HINT } else { tool };
    hint_for(tool).map(shown_segments)
}

fn shown_segments(segments: &[(&'static str, bool)]) -> Vec<(Cow<'static, str>, bool)> {
    segments
        .iter()
        .map(|&(text, bold)| {
            // Key names ("Click", "Esc") translate; in a chord ("Alt+Drag") the modifiers are named for
            // the platform and only the action after the last `+` translates.
            if !bold {
                return (Cow::Borrowed(tl!(text)), bold);
            }
            match text.rsplit_once('+') {
                Some((_, key)) if !key.is_empty() => {
                    let pretty = crate::keymap::chord_text(text);
                    let shown = pretty.strip_suffix(key).map_or(pretty.clone(), |m| format!("{m}{}", tl!(key)));
                    (Cow::Owned(shown), bold)
                }
                _ => (Cow::Owned(crate::keymap::chord_text(tl!(text))), bold),
            }
        })
        .collect()
}

/// Shows the hint bar: the help icon and one line of the active tool's hint, bold segments in the strong face.
pub fn hint_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let segments: Vec<(Cow<'static, str>, bool)> = match crate::dialogs::three_d::hint(app) {
        Some(hint) => vec![(Cow::Borrowed(hint), false)],
        None => {
            // While Cmd is held, the hints of the selection tool it lends.
            let held = crate::canvas::held_mods(ui.ctx());
            let tool = app.session.shown_tool_id(&held);
            let hint = if tool == "zoom" && crate::canvas::animated_zoom(&app.session.prefs) { ANIMATED_ZOOM_HINT } else { tool };
            hint_segments(app, hint)
        }
    };
    Bar::new(tl!("Help Bar"), BarKind::Status).show(ui, |ui, _| {
        Icon::new(IconSpec::svg(IconId::new("dc-help"))).size(ControlSize::Md).show(ui);
        let spans = segments
            .iter()
            .map(|(text, bold)| {
                let span = Span::new(text.clone());
                if *bold { span.strong() } else { span }
            })
            .collect();
        Text::styled(spans).elide(Elide::End).show(ui);
    });
}
