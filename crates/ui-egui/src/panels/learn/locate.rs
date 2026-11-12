//! `learn.locateTool`, which the link on a tool's name in a tutorial step runs: it marks the tool
//! where the current layout shows it, without choosing it. A tool shown on its toolbar button has
//! the button marked; a tool in a collapsed group opens the group's flyout with the tool's row
//! marked; a group torn off into a strip is raised with the tool marked in it; a tool on no shown
//! toolbar is marked in the All Tools drawer. The mark lasts [`LIFETIME`] seconds after it first
//! shows, until Escape, or until the next locate replaces it.

use egui::{Context, CornerRadius, LayerId, Rect, Stroke, StrokeKind};
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::UiState;
use crate::theme::Tokens;

/// Seconds the mark stays on screen after it first shows.
pub const LIFETIME: f64 = 3.0;
/// The width of the mark's outline.
pub const MARK_WIDTH: f32 = 2.0;
/// Passes a locate waits for its mark to draw before it is dropped: the toolbar no longer shows
/// the tool, or the flyout it opened has closed.
const UNSEEN: u8 = 3;

/// Which control shows the located tool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The Tools panel's toolbar: a button, a group's flyout or a strip torn off it.
    Tools,
    /// The toolbar made with New Toolbar called this: a button, a group's flyout or a strip.
    Custom(String),
    /// The All Tools drawer beside the Tools panel.
    AllTools,
}

/// A tool being located and how far its mark has got.
#[derive(Clone, Debug)]
pub struct Locate {
    pub tool: &'static str,
    pub target: Target,
    /// When the mark first drew (the context's time).
    since: Option<f64>,
    /// The target was scrolled into view.
    pub(crate) scrolled: bool,
    /// The group's flyout was opened or its strip raised.
    pub(crate) revealed: bool,
    /// The toolbar made with New Toolbar that holds the tool was brought to the front.
    raised: bool,
    /// The mark drew in this pass.
    drawn: bool,
    /// Passes in a row in which the mark did not draw.
    unseen: u8,
}

/// The located tool when the Tools panel's toolbar (`toolbar` none) or the toolbar made with New
/// Toolbar called `toolbar` shows it.
pub(crate) fn wanted(ui: &UiState, toolbar: Option<&str>) -> Option<&'static str> {
    let l = ui.learn_locate.as_ref()?;
    let here = match (&l.target, toolbar) {
        (Target::Tools, None) => true,
        (Target::Custom(name), Some(t)) => name == t,
        _ => false,
    };
    here.then_some(l.tool)
}

/// The located tool when the All Tools drawer beside the Tools panel shows it.
pub(crate) fn wanted_in_drawer(ui: &UiState) -> Option<&'static str> {
    let l = ui.learn_locate.as_ref()?;
    (l.target == Target::AllTools && ui.tools_drawer && ui.drawer_toolbar.is_none()).then_some(l.tool)
}

/// Whether the target is still to be scrolled into view; true once.
pub(crate) fn scroll_once(ui: &mut UiState) -> bool {
    ui.learn_locate.as_mut().is_some_and(|l| !std::mem::replace(&mut l.scrolled, true))
}

/// Whether the target's flyout is still to be opened or its strip raised; true once.
pub(crate) fn reveal_once(ui: &mut UiState) -> bool {
    ui.learn_locate.as_mut().is_some_and(|l| !std::mem::replace(&mut l.revealed, true))
}

/// Whether the toolbar made with New Toolbar that holds the tool is still to be brought to the
/// front; true once.
pub(crate) fn raise_once(ui: &mut UiState) -> bool {
    ui.learn_locate.as_mut().is_some_and(|l| !std::mem::replace(&mut l.raised, true))
}

/// Whether a locate is under way and its mark has not drawn yet.
pub(crate) fn unmarked(ui: &UiState) -> bool {
    ui.learn_locate.as_ref().is_some_and(|l| l.since.is_none())
}

/// Mark `rect`, the located tool's control as drawn this pass on `layer`, clipped to `clip` and the
/// window. The first mark starts the [`LIFETIME`]; once it has passed, the locate ends.
pub(crate) fn mark(app: &mut VectorcraftApp, ctx: &Context, layer: LayerId, clip: Rect, rect: Rect) {
    let Some(l) = app.ui.learn_locate.as_mut() else { return };
    let now = ctx.input(|i| i.time);
    let since = *l.since.get_or_insert(now);
    let left = LIFETIME - (now - since);
    if left <= 0.0 {
        app.ui.learn_locate = None;
        return;
    }
    l.drawn = true;
    ctx.request_repaint_after_secs(left as f32);
    let t = Tokens::get(ctx);
    let painter = ctx.layer_painter(layer).with_clip_rect(clip.intersect(ctx.content_rect()));
    painter.rect_stroke(rect.shrink(1.0), CornerRadius::same(6), Stroke::new(MARK_WIDTH, t.selection), StrokeKind::Inside);
}

/// After the toolbars and panels have drawn: Escape ends the locate, also an Escape that closed the
/// All Tools drawer or ended something else first, and so does a mark that has not drawn for
/// [`UNSEEN`] passes.
pub(crate) fn tidy(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(l) = app.ui.learn_locate.as_mut() else { return };
    // The raw events keep a key that a handler consumed this pass.
    let escape = ctx.input(|i| i.raw.events.iter().any(|e| matches!(e, egui::Event::Key { key: egui::Key::Escape, pressed: true, .. })));
    if escape {
        app.ui.learn_locate = None;
        return;
    }
    if std::mem::take(&mut l.drawn) {
        l.unseen = 0;
    } else {
        l.unseen += 1;
        if l.unseen > UNSEEN {
            app.ui.learn_locate = None;
        }
    }
}

/// The reply for a tool that no toolbar can show now, with what would show it.
fn unavailable(tool: &str, reason: &str) -> Value {
    json!({"tool": tool, "location": "unavailable", "toolbar": null, "reason": reason})
}

/// Mark `tool` on `target` from the next pass, replacing an earlier locate; the flyout or drawer an
/// earlier one opened closes.
fn start(app: &mut VectorcraftApp, tool: &'static str, target: Target) {
    if let Some(old) = app.ui.learn_locate.take() {
        app.ui.flyout = None;
        if old.target == Target::AllTools && target != Target::AllTools && app.ui.drawer_toolbar.is_none() {
            app.ui.tools_drawer = false;
        }
    }
    app.ui.learn_locate = Some(Locate { tool, target, since: None, scrolled: false, revealed: false, raised: false, drawn: false, unseen: 0 });
}

/// `learn.locateTool {tool}`: mark where `tool` is in the current layout →
/// `{tool, location, toolbar}`, where `location` is `toolbar` (its button), `flyout` (a row of its
/// collapsed group's flyout, which opens), `strip` (its group torn off into a strip, which comes
/// to the front), `allTools` (no shown toolbar holds it: the All Tools drawer opens) or
/// `unavailable` (with `reason`), and `toolbar` names the toolbar made with New Toolbar that
/// holds it (null for the Tools panel's). A Tools panel that Window › Tools hid shows again; the
/// tool is not chosen, and the remembered tool of its group stays. A missing, non-string or unknown
/// id is an error and changes nothing.
pub(crate) fn locate_tool(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "tool";
    let tool = match p.get(key) {
        None | Some(Value::Null) => return Err("tool (a tool id) is required".into()),
        Some(Value::String(tool)) => tool.as_str(),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let tool = vectorcraft_tools::tool_info(tool).ok_or_else(|| format!("unknown tool `{tool}`"))?.id;
    if app.ui.screen_mode >= 3 {
        return Ok(unavailable(tool, "Presentation Mode hides the toolbars: change the screen mode to show them"));
    }
    // Tab hides the toolbars made with New Toolbar until the next Tab, also once Window › Tools
    // shows the Tools panel again.
    let tab_hid = app.ui.tab_hid.is_some();
    if tab_hid && !app.ui.toolbar {
        return Ok(unavailable(tool, "Tab has hidden the toolbars: press Tab to show them"));
    }
    if let Some(slot) = crate::toolbar::slots(app).into_iter().map(|(_, s)| s).find(|s| s.contains(&tool)) {
        let location = if slot.len() > 1 && crate::toolbar::is_floating(app, None, slot[0]) {
            "strip"
        } else if crate::toolbar::shown_tool(app, &slot) == tool {
            "toolbar"
        } else {
            "flyout"
        };
        app.ui.toolbar = true;
        start(app, tool, Target::Tools);
        return Ok(json!({"tool": tool, "location": location, "toolbar": null}));
    }
    let active = app.session.tool_id();
    let custom = app.ui.custom_toolbars.iter().filter(|b| b.shown && !tab_hid).find_map(|bar| {
        let slot = bar.slots.iter().find(|s| s.iter().any(|t| t == tool))?;
        let tools: Vec<&'static str> = slot.iter().filter_map(|id| vectorcraft_tools::tool_info(id).map(|t| t.id)).collect();
        let first = *tools.first()?;
        let location = if tools.len() > 1 && crate::toolbar::is_floating(app, Some(&bar.name), first) {
            "strip"
        } else if crate::custom_toolbars::shown_tool(bar, active, &tools) == Some(tool) {
            "toolbar"
        } else {
            "flyout"
        };
        Some((bar.name.clone(), location))
    });
    if let Some((name, location)) = custom {
        start(app, tool, Target::Custom(name.clone()));
        return Ok(json!({"tool": tool, "location": location, "toolbar": name}));
    }
    app.run("window.editToolbar", json!({"open": true}))?;
    start(app, tool, Target::AllTools);
    Ok(json!({"tool": tool, "location": "allTools", "toolbar": null}))
}
