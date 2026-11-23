//! Tool cards (General › Show Tool Cards, off by default): with tool tips on, resting the pointer on
//! a tool button shows a small card in place of the one-line tool tip, with the tool's icon, name
//! and shortcut, a sentence on what it does ([`vectorcraft_tools::catalog::tool_description`]) and
//! the keys the hint bar lists for it. A tool without a description keeps its one-line tool tip.
//!
//! The card only shows: it holds no widget that takes a click, so the pointer and the keyboard go to
//! the toolbar under it, and it sits beside its button, never over it. It goes away on any press,
//! drag, key, modifier key, scroll or pinch, and comes back only after the pointer leaves the button
//! and rests on one again.

use egui::{Context, Event, Id, RectAlign, Response, RichText, Sense, Ui, vec2};
use vectorcraft_tools::ToolInfo;
use vectorcraft_tools::catalog::tool_description;

use crate::icons;
use crate::theme::Tokens;

/// The card's width.
pub(super) const WIDTH: f32 = 260.0;
/// The size of the tool's icon on the card.
pub(super) const ICON: f32 = 32.0;
/// The space between a card and its button.
const GAP: f32 = 4.0;
/// The room a card needs beside its button: its width with the frame's margins and border.
const ROOM: f32 = WIDTH + 20.0;
/// The height of a line of the card's text.
const LINE: f32 = 16.0;

/// The preferences the cards follow, pushed in each frame by `prefs_dialog::apply_runtime`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct CardPrefs {
    /// General › Show Tool Cards.
    pub on: bool,
    /// The Zoom tool zooms with a drag (its hint differs).
    pub animated_zoom: bool,
}

fn prefs_id() -> Id {
    Id::unique("tool-card-prefs")
}

/// The button whose card a press, key or scroll put away, until the pointer leaves it.
fn quiet_id() -> Id {
    Id::unique("tool-card-quiet")
}

pub(crate) fn set_prefs(ctx: &Context, prefs: CardPrefs) {
    ctx.data_mut(|d| d.insert_temp(prefs_id(), prefs));
}

fn prefs(ctx: &Context) -> CardPrefs {
    ctx.data(|d| d.get_temp(prefs_id())).unwrap_or_default()
}

/// Where a card goes beside its button: right of it (left when there is no room), or below it
/// (above when there is no room) for a row of buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Side {
    Beside,
    Below,
    /// Right of a flyout's row, clear of the grab bar down the flyout's right side.
    Row,
}

/// The hover help of the button `resp` for `tool`: its card with Show Tool Cards on when the tool
/// has a description and the window has room for the card beside the button, else `tip` as a
/// one-line tool tip (none: no tool tip). While a group's flyout
/// or the All Tools drawer is open, only the flyout's own rows show cards: a card beside another
/// button could sit over the flyout.
pub(crate) fn hover(resp: Response, tool: &ToolInfo, side: Side, tip: Option<String>) -> Response {
    let ctx = resp.ctx.clone();
    let prefs = prefs(&ctx);
    let allowed = side == Side::Row || !flyout_drawn(&ctx);
    let description = if prefs.on && allowed { tool_description(tool.id) } else { None };
    // A button the pointer is not on shows neither a card nor a tool tip. Its quiet mark goes, with
    // the cards off too, so the next rest on it shows its card; and its card is not measured.
    if !pointer_on(&ctx, &resp) {
        forget_quiet(&ctx, &resp);
        if description.is_some() {
            return resp;
        }
    }
    let place = description.and_then(|d| placement(&ctx, resp.rect, side, height(tool.id, prefs.animated_zoom)).map(|p| (d, p)));
    let Some((description, (align, others, gap))) = place else {
        return match tip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        };
    };
    if quiet(&ctx, &resp) {
        return resp;
    }
    let mut tooltip = egui::Tooltip::for_enabled(&resp).gap(gap);
    tooltip.popup = tooltip.popup.align(align).align_alternatives(others).width(WIDTH);
    tooltip.show(|ui| card(ui, tool, description, prefs.animated_zoom));
    resp
}

/// Where the card of the button at `rect` goes: its alignment, the alignments to try when it does
/// not fit, and its gap from the button. None when the window has no room for it beside the button,
/// or above or below a row of buttons for a card about `height` tall (a card pushed back into the
/// window would cover the button or its flyout).
fn placement(ctx: &Context, rect: egui::Rect, side: Side, height: f32) -> Option<(RectAlign, &'static [RectAlign], f32)> {
    let screen = ctx.content_rect();
    let right = |gap: f32| screen.right() - rect.right() - gap >= ROOM;
    let left = |gap: f32| rect.left() - gap - screen.left() >= ROOM;
    let below = screen.bottom() - rect.bottom() - GAP >= height;
    let above = rect.top() - GAP - screen.top() >= height;
    match side {
        Side::Beside if right(GAP) => Some((RectAlign::RIGHT_START, &[RectAlign::RIGHT_END], GAP)),
        Side::Beside if left(GAP) => Some((RectAlign::LEFT_START, &[RectAlign::LEFT_END], GAP)),
        Side::Beside => None,
        // On the side the flyout opened toward, never back over the button that opened it.
        Side::Row if flyout_opens_left(ctx) => left(GAP).then_some((RectAlign::LEFT_START, &[RectAlign::LEFT_END], GAP)),
        Side::Row => right(GAP + super::BAR).then_some((RectAlign::RIGHT_START, &[RectAlign::RIGHT_END], GAP + super::BAR)),
        Side::Below if below => Some((RectAlign::BOTTOM_START, &[RectAlign::BOTTOM_END], GAP)),
        Side::Below if above => Some((RectAlign::TOP_START, &[RectAlign::TOP_END], GAP)),
        Side::Below => None,
    }
}

/// A generous estimate of a card's height, before it is laid out: the icon row, a description of
/// up to three lines and each hint line wrapped once, with the frame's margins.
fn height(tool: &str, animated_zoom: bool) -> f32 {
    let hints = lines(tool, animated_zoom).len() as f32;
    let hint_rows = if hints > 0.0 { 8.0 + hints * 2.0 * LINE } else { 0.0 };
    ICON + 20.0 + 3.0 * LINE + hint_rows
}

/// Whether a tool group's flyout or the All Tools drawer drew in this pass or the one before. The
/// tool buttons draw before the flyout, so in a pass they read the mark of the pass before.
fn flyout_drawn(ctx: &Context) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data(|d| d.get_temp::<u64>(super::flyout_open_pass_id())).is_some_and(|p| p.saturating_add(1) >= pass)
}

/// Whether the open flyout sits left of the button that opened it (a toolbar docked right).
fn flyout_opens_left(ctx: &Context) -> bool {
    let anchor = ctx.data(|d| d.get_temp::<egui::Rect>(Id::unique("flyout-anchor")));
    let menu = ctx.memory(|m| m.area_rect(Id::unique("tool-flyout")));
    anchor.zip(menu).is_some_and(|(a, m)| m.center().x < a.center().x)
}

/// Whether the card of `resp` stays away: the pointer is not resting on the button, a button or a
/// modifier key is held, or a press, key, modifier, scroll or pinch came since it arrived.
fn quiet(ctx: &Context, resp: &Response) -> bool {
    let quiet_for: Option<Id> = ctx.data(|d| d.get_temp(quiet_id()));
    if !pointer_on(ctx, resp) {
        forget_quiet(ctx, resp);
        return true;
    }
    let busy = ctx.input(|i| {
        i.pointer.any_down()
            || i.pointer.any_pressed()
            || i.pointer.any_released()
            || i.modifiers.any()
            // The frame's raw events: the shortcuts, which run before the toolbar draws, take the
            // keys they handle (such as the arrows that nudge the selection) out of `i.events`.
            || i.raw.events.iter().any(|e| {
                matches!(
                    e,
                    Event::Key { pressed: true, .. }
                        | Event::Text(_)
                        | Event::MouseWheel { .. }
                        | Event::ModifiersChanged(_)
                        | Event::Zoom(_)
                        | Event::Rotate(_)
                )
            })
    });
    if busy {
        ctx.data_mut(|d| d.insert_temp(quiet_id(), resp.id));
        return true;
    }
    quiet_for == Some(resp.id)
}

/// Whether the pointer rests on the button `resp`.
fn pointer_on(ctx: &Context, resp: &Response) -> bool {
    resp.hovered() && ctx.input(|i| i.pointer.hover_pos()).is_some_and(|p| resp.rect.contains(p))
}

/// Drops the quiet mark of `resp`, once the pointer has left it.
fn forget_quiet(ctx: &Context, resp: &Response) {
    if ctx.data(|d| d.get_temp::<Id>(quiet_id())) == Some(resp.id) {
        ctx.data_mut(|d| d.remove::<Id>(quiet_id()));
    }
}

/// The card: icon, name and shortcut; what the tool does; one line per action of its hint.
fn card(ui: &mut Ui, tool: &ToolInfo, description: &str, animated_zoom: bool) {
    let t = Tokens::get(ui.ctx());
    // Every line wraps inside the width `placement` keeps free, long translated names included.
    ui.set_max_width(WIDTH);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(ICON, ICON), Sense::hover());
        icons::paint(ui, icons::tool_icon(tool.icon), r, t.icon);
        ui.add_space(4.0);
        ui.add(egui::Label::new(RichText::new(super::tip(tool)).strong().color(t.text_strong)).wrap());
    });
    ui.add(egui::Label::new(RichText::new(tl!(description)).color(t.text)).wrap());
    let lines = lines(tool.id, animated_zoom);
    if !lines.is_empty() {
        ui.separator();
    }
    for line in lines {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = ui.available_width();
        for (text, bold) in line {
            let font = if bold { crate::theme::semibold(12.0) } else { egui::FontId::proportional(12.0) };
            job.append(&text, 0.0, egui::TextFormat { font_id: font, color: if bold { t.text_strong } else { t.text }, ..Default::default() });
        }
        ui.label(job);
    }
}

/// The hint bar's segments for `tool`, one line per action: a line ends at a `|` in the hint's text.
pub(crate) fn lines(tool: &str, animated_zoom: bool) -> Vec<Vec<(String, bool)>> {
    let mut lines = vec![];
    let mut line: Vec<(String, bool)> = vec![];
    for (text, bold) in crate::chrome::tool_hint(tool, animated_zoom).unwrap_or_default() {
        if bold {
            line.push((text.into_owned(), true));
            continue;
        }
        let mut parts = text.split('|').peekable();
        while let Some(part) = parts.next() {
            let part = if line.is_empty() { part.trim_start() } else { part };
            let part = if parts.peek().is_some() { part.trim_end() } else { part };
            if !part.is_empty() {
                line.push((part.to_string(), false));
            }
            if parts.peek().is_some() && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}
