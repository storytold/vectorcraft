//! `learn.locateTool`, which the link on a tool's name in a tutorial step runs: it marks the tool
//! where the current layout shows it, without choosing it. A tool shown on its toolbar button has
//! the button marked; a tool in a collapsed group opens the group's flyout with the tool's row
//! marked; a group torn off into a strip is raised with the tool marked in it; a tool on no shown
//! toolbar is marked in the All Tools drawer. The rails and the drawer draw the mark and reveal the
//! tool themselves ([`wanted`], [`take_reveal`]); it lasts [`LIFETIME`] seconds after the first
//! pass that follows, until Escape, or until the next locate replaces it.

use craft::ui::runtime::keys::{self, Intent};
use egui::Context;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::state::UiState;
use crate::toolbar::Owner;

/// Seconds the mark stays on screen after it first shows.
pub const LIFETIME: f64 = 3.0;

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

/// A tool being located.
#[derive(Clone, Debug)]
pub struct Locate {
    pub tool: &'static str,
    pub target: Target,
    /// When the first pass after the locate ran (the context's time); `start` has no context.
    started: Option<f64>,
    /// The tool is still to be revealed (scrolled into view, its flyout opened) by its home.
    reveal: bool,
}

/// A control that can show the located tool.
pub(crate) enum Home<'a> {
    /// A rail: the Tools panel's, a toolbar made with New Toolbar, or a strip torn off either.
    Rail(&'a Owner),
    /// The All Tools drawer beside the Tools panel.
    Drawer,
}

/// The toolbar a rail belongs to: the owner itself, or the one a strip was torn off.
fn base(owner: &Owner) -> &Owner {
    match owner {
        Owner::Strip(inner, _) => base(inner),
        owner => owner,
    }
}

/// The located tool while a locate lives and `home` belongs to its target. Owned so a caller can
/// hold it while it takes the reveal from the same state.
pub(crate) fn wanted(ui: &UiState, home: &Home) -> Option<String> {
    let l = ui.learn_locate.as_ref()?;
    let here = match (&l.target, home) {
        (Target::Tools, Home::Rail(owner)) => matches!(base(owner), Owner::Tools),
        (Target::Custom(name), Home::Rail(owner)) => {
            matches!(base(owner), Owner::Toolbar(id) if crate::custom_toolbars::find(ui, name).is_some_and(|b| b.id == *id))
        }
        (Target::AllTools, Home::Drawer) => ui.tools_drawer && ui.drawer_toolbar.is_none(),
        _ => false,
    };
    here.then(|| l.tool.to_string())
}

/// The located tool when `home` shows it, once per locate: the request to reveal it. `holds` is
/// the home's own test that it contains the tool, so a strip and its toolbar, which share a
/// target, leave the request to the one that holds it.
pub(crate) fn take_reveal(ui: &mut UiState, home: &Home, holds: impl Fn(&str) -> bool) -> Option<String> {
    let tool = wanted(ui, home).filter(|tool| holds(tool))?;
    let l = ui.learn_locate.as_mut()?;
    std::mem::replace(&mut l.reveal, false).then_some(tool)
}

/// After the toolbars and panels have drawn: an Escape that no component took ends the locate,
/// and so does the closing of the All Tools drawer a locate showed, and so does [`LIFETIME`].
pub(crate) fn tidy(app: &mut VectorcraftApp, ctx: &Context) {
    let Some(l) = app.ui.learn_locate.as_mut() else { return };
    if keys::with_window_intents(ctx, false, |window| window.take(Intent::Cancel)) {
        app.ui.learn_locate = None;
        return;
    }
    if l.target == Target::AllTools && (!app.ui.tools_drawer || app.ui.drawer_toolbar.is_some()) {
        app.ui.learn_locate = None;
        return;
    }
    let now = ctx.input(|i| i.time);
    let left = LIFETIME - (now - *l.started.get_or_insert(now));
    if left <= 0.0 {
        app.ui.learn_locate = None;
    } else {
        ctx.request_repaint_after_secs(left as f32);
    }
}

/// The reply for a tool that no toolbar can show now, with what would show it.
fn unavailable(tool: &str, reason: &str) -> Value {
    json!({"tool": tool, "location": "unavailable", "toolbar": null, "reason": reason})
}

/// Locate `tool` on `target` from the next pass, replacing an earlier locate; the drawer an
/// earlier one opened closes.
fn start(app: &mut VectorcraftApp, tool: &'static str, target: Target) {
    if let Some(old) = app.ui.learn_locate.take()
        && old.target == Target::AllTools
        && target != Target::AllTools
        && app.ui.drawer_toolbar.is_none()
    {
        app.ui.tools_drawer = false;
    }
    app.ui.learn_locate = Some(Locate { tool, target, started: None, reveal: true });
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
    let found = match find(app, tool, false) {
        Ok(found) => found,
        Err(reason) => return Ok(unavailable(tool, reason)),
    };
    if found.target == Target::Tools {
        crate::toolbar::set_shown(app, true);
    }
    if let Some(panel) = &found.panel {
        crate::panel_docking::reveal_panel(app, panel)?;
    }
    match found.target {
        Target::Tools => {
            start(app, tool, Target::Tools);
            Ok(json!({"tool": tool, "location": found.location, "toolbar": null}))
        }
        Target::Custom(name) => {
            start(app, tool, Target::Custom(name.clone()));
            Ok(json!({"tool": tool, "location": found.location, "toolbar": name}))
        }
        Target::AllTools => {
            app.run("window.editToolbar", json!({"open": true}))?;
            start(app, tool, Target::AllTools);
            Ok(json!({"tool": tool, "location": "allTools", "toolbar": null}))
        }
    }
}

/// Where a toolbar holds a tool: [`find`]'s answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Found {
    /// The control that shows the tool.
    pub target: Target,
    /// `toolbar` (its slot's button shows it), `flyout` (it is hidden in its slot's group),
    /// `strip` (its group is torn off into a strip) or `allTools`.
    pub location: &'static str,
    /// The tool the slot's button shows, for `toolbar` and `flyout`.
    pub shown: Option<&'static str>,
    /// The panel that must be shown for the tool to be seen: the strip, or the toolbar made with
    /// New Toolbar. None for the Tools panel and All Tools.
    pub panel: Option<String>,
}

/// Where `tool` is in the current layout, without showing or opening anything: the Tools panel's
/// toolbar first, also while Window › Tools hides it, then the shown toolbars made with New
/// Toolbar, else the All Tools drawer. With `shown_first`, a hidden Tools panel comes after the
/// toolbars made with New Toolbar. An error names why no toolbar can show the tool now.
pub(crate) fn find(app: &VectorcraftApp, tool: &'static str, shown_first: bool) -> Result<Found, &'static str> {
    if app.ui.screen_mode >= 3 {
        return Err("Presentation Mode hides the toolbars: change the screen mode to show them");
    }
    if crate::panel_docking::tab_hidden(app) {
        return Err("Tab has hidden the toolbars: press Tab to show them");
    }
    let tools_shown = crate::toolbar::shown(&app.ui);
    let main = crate::toolbar::slots(app).into_iter().map(|(_, s)| s).find(|s| s.contains(&tool)).map(|slot| {
        if slot.len() > 1 && crate::toolbar::is_floating(app, None, slot[0]) {
            let panel = crate::toolbar::strip_id(&Owner::Tools, slot[0]);
            Found { target: Target::Tools, location: "strip", shown: None, panel: Some(panel) }
        } else {
            let shown = crate::toolbar::shown_tool(app, &slot);
            Found { target: Target::Tools, location: if shown == tool { "toolbar" } else { "flyout" }, shown: Some(shown), panel: None }
        }
    });
    if let Some(main) = main.clone().filter(|_| !shown_first || tools_shown) {
        return Ok(main);
    }
    let active = app.session.tool_id();
    let custom =
        app.ui.custom_toolbars.iter().filter(|b| crate::panel_docking::visible(app, &crate::custom_toolbars::panel_id(b.id))).find_map(|bar| {
            let slot = bar.slots.iter().find(|s| s.iter().any(|t| t == tool))?;
            let tools: Vec<&'static str> = slot.iter().filter_map(|id| vectorcraft_tools::tool_info(id).map(|t| t.id)).collect();
            let first = *tools.first()?;
            let target = Target::Custom(bar.name.clone());
            if tools.len() > 1 && crate::toolbar::is_floating(app, Some(&bar.name), first) {
                let panel = crate::toolbar::strip_id(&Owner::Toolbar(bar.id), first);
                return Some(Found { target, location: "strip", shown: None, panel: Some(panel) });
            }
            let shown = crate::custom_toolbars::shown_tool(bar, active, &tools);
            let panel = Some(crate::custom_toolbars::panel_id(bar.id));
            Some(Found { target, location: if shown == Some(tool) { "toolbar" } else { "flyout" }, shown, panel })
        });
    Ok(custom.or(main).unwrap_or(Found { target: Target::AllTools, location: "allTools", shown: None, panel: None }))
}
