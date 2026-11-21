//! Tool cards (General › Show Tool Cards, off by default): with tool tips on, resting the pointer on
//! a tool button shows a small card in place of the one-line tool tip, with the tool's icon, name
//! and shortcut, a sentence on what it does ([`vectorcraft_tools::catalog::tool_description`]) and
//! the keys the hint bar lists for it. A tool without a description keeps its one-line tool tip.
//!
//! The rail places and dismisses the card ([`ToolTips`]); this module only says which tools have
//! one and draws it.

use craft::ui::rails::ToolTips;
use egui::{RichText, Sense, Ui, vec2};
use vectorcraft_tools::ToolInfo;
use vectorcraft_tools::catalog::tool_description;
use vectorcraft_tools::tool_info;

use crate::VectorcraftApp;
use crate::icons;
use crate::theme::Tokens;

/// The card's width.
const WIDTH: f32 = 260.0;
/// The size of the tool's icon on the card.
const ICON: f32 = 32.0;

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

/// The card: icon, name and shortcut; what the tool does; one line per action of its hint.
fn card(ui: &mut Ui, tool: &ToolInfo, description: &str, animated_zoom: bool) {
    let t = Tokens::get(ui.ctx());
    // Every line wraps inside the card's width, long translated names included.
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
