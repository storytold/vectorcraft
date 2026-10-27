use super::status_bar::ANIMATED_ZOOM_HINT;
use super::*;

fn hint_for(tool: &str) -> Option<&'static [(&'static str, bool)]> {
    Some(match tool {
        "dimension" => &[
            ("Drag", true),
            (" out from a highlighted edge to place its dimension  |  ", false),
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

pub(super) fn hint_segments(tool: &str) -> Vec<(Cow<'static, str>, bool)> {
    let search;
    let segments = match hint_for(tool) {
        Some(s) => s,
        None => {
            search = match menus::shortcut_of("help.commandPalette") {
                Some(sc) => [(tl!("Press "), false), (sc, true), (tl!(" to search every command"), false)],
                None => [(tl!("Use "), false), (tl!("Help › Search Commands…"), true), (tl!(" to search every command"), false)],
            };
            &search
        }
    };
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
                    let pretty = menus::pretty_shortcut(text);
                    let shown = pretty.strip_suffix(key).map_or(pretty.clone(), |m| format!("{m}{}", tl!(key)));
                    (Cow::Owned(shown), bold)
                }
                _ => (Cow::Owned(menus::pretty_shortcut(tl!(text))), bold),
            }
        })
        .collect()
}

pub fn hint_bar(app: &mut VectorcraftApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let panel = egui::Panel::bottom("hint_bar")
        .exact_size(28.0)
        .frame(egui::Frame::NONE.fill(t.panel).inner_margin(egui::Margin::symmetric(10, 0)).stroke(Stroke::new(1.0, t.border)))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                ui.painter().circle_stroke(r.center(), 7.0, Stroke::new(1.2, t.text));
                ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "?", theme::semibold(11.0), t.text);
                ui.add_space(6.0);
                let mut job = egui::text::LayoutJob::default();
                let segments = if let Some(d) = app.ui.dialog.as_ref().filter(|d| crate::dialogs::revolve_gizmo::active(d)) {
                    vec![(crate::dialogs::revolve_gizmo::hint(d).into(), false)]
                } else {
                    let tool = app.session.tool_id();
                    let hint = if tool == "zoom" && crate::canvas::animated_zoom(&app.session.prefs) { ANIMATED_ZOOM_HINT } else { tool };
                    hint_segments(hint)
                };
                for (txt, bold) in segments {
                    let font = if bold { theme::semibold(12.5) } else { egui::FontId::proportional(12.5) };
                    job.append(&txt, 0.0, egui::TextFormat { font_id: font, color: if bold { t.text_strong } else { t.text }, ..Default::default() });
                }
                ui.label(job);
            });
        });
    widgets::set_menu_floor(ui.ctx(), panel.response.rect.top());
}
