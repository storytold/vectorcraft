//! Tool cards (General › Show Tool Cards, off by default): with tool tips on, resting the pointer on
//! a tool button shows a small card in place of the one-line tool tip, with the tool's icon, name
//! and shortcut, a sentence on what it does ([`vectorcraft_tools::catalog::tool_description`]) and
//! the keys the hint bar lists for it. A tool without a description keeps its one-line tool tip.
//!
//! The rail places and dismisses the card ([`ToolTips`]); this module only says which tools have
//! one and draws it.

use craft::behavior::IconId;
use craft::ui::contract::{Show as _, ShowContainer as _};
use craft::ui::display::{Divider, Icon, Text};
use craft::ui::layout_components::Row;
use craft::ui::primitives::IconSpec;
use craft::ui::rails::ToolTips;
use craft::ui::style::{Span, TypeRole};
use egui::Ui;
use vectorcraft_tools::ToolInfo;
use vectorcraft_tools::catalog::tool_description;
use vectorcraft_tools::tool_info;

use crate::VectorcraftApp;
use crate::icons;

/// The tool cards of one pass: the two preferences they follow.
pub(crate) struct Cards {
    /// General › Show Tool Cards, which shows only with Show Tool Tips on.
    on: bool,
    /// The Zoom tool zooms with a drag (its hint differs).
    animated_zoom: bool,
}

impl Cards {
    /// The cards under `app`'s preferences. Copies them: the rail's footer holds the app mutably while
    /// the rail holds the cards.
    pub(crate) fn new(app: &VectorcraftApp) -> Self {
        let prefs = &app.session.prefs;
        Cards { on: prefs.show_tool_cards && prefs.show_tool_tips, animated_zoom: crate::canvas::animated_zoom(prefs) }
    }
}

impl ToolTips<String> for Cards {
    fn has(&self, tool: &String) -> bool {
        self.on && tool_description(tool).is_some()
    }

    fn show(&self, tool: &String, ui: &mut Ui) {
        if let Some(info) = tool_info(tool)
            && let Some(description) = tool_description(info.id)
        {
            card(ui, info, description, self.animated_zoom);
        }
    }
}

/// The card: icon, name and shortcut; what the tool does; one line per action of its hint, its
/// keys in bold. Every line wraps inside the card's width, long translated names included.
fn card(ui: &mut Ui, tool: &ToolInfo, description: &'static str, animated_zoom: bool) {
    let name = super::tip(ui.ctx(), tool);
    Row::new().show(ui, |ui, _| {
        Icon::new(IconSpec::svg(IconId::new(icons::tool_icon(tool.icon)))).show(ui);
        Text::new(name.clone()).role(TypeRole::BodyStrong).show(ui);
    });
    Text::new(tl!(description)).show(ui);
    let lines = lines(tool.id, animated_zoom);
    if !lines.is_empty() {
        Divider::horizontal().show(ui);
    }
    for line in lines {
        let spans = line
            .into_iter()
            .map(|(text, bold)| {
                let span = Span::new(text);
                if bold { span.strong() } else { span }
            })
            .collect();
        Text::styled(spans).show(ui);
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
