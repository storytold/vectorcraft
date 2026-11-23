//! The Type tool's text part: the framework's editor part (`Input::Editor`) over the engine tool's caret.
//!
//! The framework's IME engine owns the route, the composition guard and the reset; this component feeds what it
//! delivers (typing, marked text, deletions around the caret, moves, edits) to the engine tool one input at a time, in
//! arrival order, and reports whether the tool still shows marked text so the engine interrupts a composition the tool
//! ended. The caret itself is still drawn by the tool's overlay.

// PLAN: assumed `craft = { workspace = true }` in this crate's Cargo.toml (owner: hub-w1); `craft_ui` does not re-export
// the editor input types, so they come through the `craft::behavior` facade.
use craft::behavior::{EditCommand, EditorInput, Granularity, Metrics, MoveMods, NavMove, PartKey, Sense, SurroundingUnit as ImeUnit, TextPos};
use craft::keymap::{Binding, Chord, CommandDef, CommandRef, KeyInput, Layout, Overrides};
use craft::ui::contract::{
    CommandSet, Commands as _, Common, Component, Cx, EditorStep, Focus as _, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Props as _,
    Remember as _, Role, Show as _,
};
use craft::ui::primitives::{EditorPurpose, EditorState};
use craft::ui::runtime::focus::{self, FocusTarget, KeyUse};
use craft::ui::runtime::pending::Pending;
use craft::ui::runtime::windows::WindowHost;
use craft::ui::style::{ComponentStyle, StyleCx};
use egui::{Rect, Response, Vec2};
use serde_json::json;
use vectorcraft_engine::ViewInfo;
use vectorcraft_tools::{Mods, SurroundingUnit, ToolKey};

use crate::VectorcraftApp;
// PLAN: assumed `toggle_face(app, Face) -> Result<Value, String>` and `Face` from `panels::character` (ty.S10).
use crate::panels::character::{Face, text_copy, text_key, text_paste, toggle_face};

/// The part key of the editor part.
const EDITOR: PartKey = PartKey::new("editor");

/// The commands of a Type session: the commit chord, the formatting toggles, the characters Alt chords type on
/// Windows and Linux, and the Type tool's app commands. They sit in a typing scope so they reach the editor before an
/// app shortcut on the same chord and run while it types, where no other shortcut does. Option types these characters
/// natively on macOS, so they have no default there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "typeEditor", scope = "typeEditor", typing)]
#[non_exhaustive]
pub(crate) enum TypeEditorCommand {
    /// Commit the text and keep it.
    #[command(label = "Commit Text", default = "Cmd+Enter")]
    Commit,
    /// Toggle the bold face.
    #[command(label = "Bold", default = "Cmd+Shift+B")]
    Bold,
    /// Toggle the italic face.
    #[command(label = "Italic", default = "Cmd+Shift+I")]
    Italic,
    /// Type a bullet.
    #[command(label = "Type Bullet", default = "Alt+8", no_default_on = "mac")]
    Bullet,
    /// Type a copyright sign.
    #[command(label = "Type Copyright Sign", default = "Alt+G", no_default_on = "mac")]
    Copyright,
    /// Type an ellipsis.
    #[command(label = "Type Ellipsis", default = "Alt+;", no_default_on = "mac")]
    Ellipsis,
    /// Type a paragraph sign.
    #[command(label = "Type Paragraph Sign", default = "Alt+7", no_default_on = "mac")]
    Paragraph,
    /// Type a section sign.
    #[command(label = "Type Section Sign", default = "Alt+6", no_default_on = "mac")]
    Section,
    /// Type a trademark sign.
    #[command(label = "Type Trademark Sign", default = "Alt+2", no_default_on = "mac")]
    Trademark,
    /// Type a registered sign.
    #[command(label = "Type Registered Sign", default = "Alt+R", no_default_on = "mac")]
    Registered,
    /// Type an en dash.
    #[command(label = "Type En Dash", default = "Alt+-", no_default_on = "mac")]
    EnDash,
    /// Type an em dash.
    #[command(label = "Type Em Dash", default = "Alt+Shift+-", no_default_on = "mac")]
    EmDash,
    /// Type an opening double quote.
    #[command(label = "Type Opening Double Quote", default = "Alt+[", no_default_on = "mac")]
    OpenDoubleQuote,
    /// Type a closing double quote.
    #[command(label = "Type Closing Double Quote", default = "Alt+Shift+[", no_default_on = "mac")]
    CloseDoubleQuote,
    /// Type an opening single quote.
    #[command(label = "Type Opening Single Quote", default = "Alt+]", no_default_on = "mac")]
    OpenSingleQuote,
    /// Type a closing single quote.
    #[command(label = "Type Closing Single Quote", default = "Alt+Shift+]", no_default_on = "mac")]
    CloseSingleQuote,
    // The Type tool's app commands ([`APP_COMMANDS`]), whose keys work while it types; their chords are the app
    // command's ([`app_keys`]).
    #[command(label = "Show Hidden Characters")]
    HiddenCharacters,
    #[command(label = "Align left")]
    AlignLeft,
    #[command(label = "Align center")]
    AlignCenter,
    #[command(label = "Align right")]
    AlignRight,
    #[command(label = "Justify with last line aligned left")]
    JustifyLeft,
    #[command(label = "Justify all lines")]
    JustifyAll,
    #[command(label = "Highlight Font Name")]
    HighlightFontName,
    #[command(label = "Highlight Kerning")]
    HighlightKerning,
    #[command(label = "Underline")]
    Underline,
    #[command(label = "Superscript")]
    Superscript,
    #[command(label = "Subscript")]
    Subscript,
    #[command(label = "Toggle Line Composer")]
    LineComposer,
    #[command(label = "Hyphenate")]
    Hyphenate,
    #[command(label = "Increase Font Size")]
    SizeIncrease,
    #[command(label = "Decrease Font Size")]
    SizeDecrease,
    #[command(label = "Increase Font Size by Five Steps")]
    SizeIncreaseByFive,
    #[command(label = "Decrease Font Size by Five Steps")]
    SizeDecreaseByFive,
    #[command(label = "Reset Tracking and Kerning")]
    ResetTracking,
    #[command(label = "Reset Horizontal Scale")]
    ResetHorizontalScale,
    #[command(label = "Create Outlines")]
    CreateOutlines,
}

/// The app command each of the Type tool's commands in [`TypeEditorCommand`] runs: production ran their chords while
/// the tool typed, where a text field runs no shortcut (`TypingRule::AllWait`).
// shortcut: only the Type tool's commands with a default chord are here; a chord the user gives another type command
// runs only outside typing, add it here if that is asked for.
const APP_COMMANDS: [(TypeEditorCommand, &str); 20] = [
    (TypeEditorCommand::HiddenCharacters, "type.hiddenCharacters"),
    (TypeEditorCommand::AlignLeft, "type.alignLeft"),
    (TypeEditorCommand::AlignCenter, "type.alignCenter"),
    (TypeEditorCommand::AlignRight, "type.alignRight"),
    (TypeEditorCommand::JustifyLeft, "type.justifyLeft"),
    (TypeEditorCommand::JustifyAll, "type.justifyAll"),
    (TypeEditorCommand::HighlightFontName, "type.highlightFontName"),
    (TypeEditorCommand::HighlightKerning, "type.highlightKerning"),
    (TypeEditorCommand::Underline, "type.underline"),
    (TypeEditorCommand::Superscript, "type.superscript"),
    (TypeEditorCommand::Subscript, "type.subscript"),
    (TypeEditorCommand::LineComposer, "type.lineComposer"),
    (TypeEditorCommand::Hyphenate, "type.hyphenate"),
    (TypeEditorCommand::SizeIncrease, "type.size.increase"),
    (TypeEditorCommand::SizeDecrease, "type.size.decrease"),
    (TypeEditorCommand::SizeIncreaseByFive, "type.size.increaseByFive"),
    (TypeEditorCommand::SizeDecreaseByFive, "type.size.decreaseByFive"),
    (TypeEditorCommand::ResetTracking, "type.resetTracking"),
    (TypeEditorCommand::ResetHorizontalScale, "type.resetHorizontalScale"),
    (TypeEditorCommand::CreateOutlines, "type.createOutlines"),
];

/// Each of the Type tool's commands with the chords of its app command among `defs`, the user's own (`overrides`)
/// included, for the catalog's `replace_defaults`: a chord moved or taken away moves with it.
pub(crate) fn app_keys(defs: &[CommandDef], overrides: &Overrides) -> Vec<(&'static str, Vec<Binding>)> {
    APP_COMMANDS
        .iter()
        .map(|(command, id)| {
            let keys = match overrides.get(id) {
                Some(texts) => texts.iter().filter_map(|text| Binding::parse(text).ok()).collect(),
                None => defs.iter().find(|def| def.command.id == *id).map(|def| def.defaults.clone()).unwrap_or_default(),
            };
            (command.id(), keys)
        })
        .collect()
}

// PLAN: `#[command(scope = ..)]` takes a string literal, so the scope is named here once and the set's own constant is
// the source.
/// The scope of [`TypeEditorCommand`].
pub(crate) const TYPE_EDITOR_SCOPE: craft::ui::runtime::commands::CommandScope = <TypeEditorCommand as CommandSet>::SCOPE;

impl TypeEditorCommand {
    /// The character the command types, with the text its key types without Alt; `None` for the other commands.
    fn character(self) -> Option<(char, &'static str)> {
        Some(match self {
            Self::Bullet => ('\u{2022}', "8"),
            Self::Copyright => ('\u{00A9}', "gG"),
            Self::Ellipsis => ('\u{2026}', ";"),
            Self::Paragraph => ('\u{00B6}', "7"),
            Self::Section => ('\u{00A7}', "6"),
            Self::Trademark => ('\u{2122}', "2"),
            Self::Registered => ('\u{00AE}', "rR"),
            Self::EnDash => ('\u{2013}', "-"),
            Self::EmDash => ('\u{2014}', "_"),
            Self::OpenDoubleQuote => ('\u{201C}', "["),
            Self::CloseDoubleQuote => ('\u{201D}', "{"),
            Self::OpenSingleQuote => ('\u{2018}', "]"),
            Self::CloseSingleQuote => ('\u{2019}', "}"),
            _ => return None,
        })
    }

    /// The app command the command runs, `None` for the editor's own.
    pub(crate) fn app_command(self) -> Option<&'static str> {
        APP_COMMANDS.iter().find(|(command, _)| *command == self).map(|(_, id)| *id)
    }
}

/// Before the keys read a window's input while the Type tool types in it: the press of a chord that types a character
/// goes when the text that came with it is other than its key's own (Alt+G typing ğ on a Turkish layout), so that text
/// is typed as it came, as production typed what the layout typed; with its key's own text or none it types its
/// character. An input method's text after the press is typed whatever the press binds.
pub(crate) fn keep_layout_text(app: &VectorcraftApp, ctx: &egui::Context, raw: &mut egui::RawInput) {
    let platform = crate::keymap::platform();
    let host = WindowHost::of(ctx);
    if platform.is_mac() || !app.session.tool_wants_text() || raw.viewport_id != host.key_window() || !host.typing() {
        return;
    }
    let chords: Vec<(Chord, &str)> = TypeEditorCommand::defs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|def| {
            let (_, own) = TypeEditorCommand::from_ref(&def.command)?.character()?;
            Some(def.defaults.into_iter().map(move |b| (b.trigger.chord, own)))
        })
        .flatten()
        .collect();
    let layout = Layout::default();
    let events = &raw.events;
    let own_of = |e: &egui::Event| {
        let input = KeyInput::from_egui(e, platform).filter(|_| matches!(e, egui::Event::Key { pressed: true, .. }))?;
        layout.candidates(&input).iter().find_map(|c| chords.iter().find(|(chord, _)| chord == c).map(|(_, own)| *own))
    };
    let dropped: Vec<bool> = events
        .iter()
        .enumerate()
        .map(|(j, e)| {
            own_of(e).is_some_and(|own| {
                matches!(events.get(j + 1), Some(egui::Event::Text(t)) if !t.is_empty() && !(t.chars().count() == 1 && own.contains(t.as_str())))
            })
        })
        .collect();
    let mut dropped = dropped.into_iter();
    raw.events.retain(|_| !dropped.next().unwrap_or(false));
}

/// End an IME composition outside the IME (a click away, the editor losing the keys): the marked text stays as typed.
/// The framework interrupts and discards the OS composition once the part reports `composing = false`.
pub(crate) fn keep_marked_text(app: &mut VectorcraftApp) {
    let Some(m) = app.ime_marked.take() else { return };
    if app.session.tool_composing() {
        // Errors are already reported by the engine's interaction.
        let _ = app.session.tool_typed_text(&m, app.view_info());
    }
}

/// What the part did this pass; it reports no value because the engine tool owns the text.
#[derive(Debug)]
pub(crate) struct TypeEditorOutput {
    response: Response,
    /// The part appeared this pass without the keys.
    unfocused_start: bool,
}

impl Outcome for TypeEditorOutput {
    type Value = NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

/// The part draws nothing: the tool's overlay draws the caret, selection and marked text.
#[derive(Clone, Debug, PartialEq, ComponentStyle)]
pub(crate) struct TypeEditorStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The editor part over the engine Type tool's caret rectangle (screen space).
#[derive(craft::ui::contract::Component, craft::ui::contract::Props)]
pub(crate) struct TypeEditor<'a> {
    #[common]
    common: Common,
    app: &'a mut VectorcraftApp,
    caret: Rect,
    /// The window's context, which the app commands run in.
    ctx: egui::Context,
}

impl<'a> TypeEditor<'a> {
    /// A part for the tool's caret at `caret`, the instance of [`TypeEditorCommand`]'s scope while it has the keys.
    fn new(app: &'a mut VectorcraftApp, caret: Rect, ctx: egui::Context) -> Self {
        TypeEditor { common: Common::new(), app, caret, ctx }.label("Type").command_scope(TYPE_EDITOR_SCOPE)
    }

    /// Applies one input of the editor part to the engine tool; a Paste types `pasted`, the text the paste delivered,
    /// or else the clipboard's.
    fn apply(&mut self, input: EditorInput, view: ViewInfo, pasted: Option<String>) {
        let app = &mut *self.app;
        match input {
            EditorInput::Text(text) => {
                let r = app.session.tool_typed_text(&text, view);
                crate::canvas::apply_requests(app, r);
            }
            EditorInput::Preedit { text, cursor, .. } => {
                // The IME's cursor counts bytes of `text`; the tool counts characters.
                let chars = |at: usize| text.get(..at).map_or_else(|| text.chars().count(), |head| head.chars().count());
                let active = cursor.map(|r| chars(r.start)..chars(r.end));
                app.ime_marked = Some(text.to_string()).filter(|t| !t.is_empty());
                let r = app.session.tool_preedit(&text, active, view);
                crate::canvas::apply_requests(app, r);
            }
            EditorInput::ImeCommit => app.ime_marked = None,
            EditorInput::DeleteSurrounding { before, after, unit, .. } => {
                let unit = match unit {
                    ImeUnit::Bytes => SurroundingUnit::Bytes,
                    _ => SurroundingUnit::Chars,
                };
                let r = app.session.tool_ime_delete_surrounding(before, after, unit, view);
                crate::canvas::apply_requests(app, r);
            }
            EditorInput::Move(step, mods) => {
                if let Some((key, mods)) = move_key(step, mods) {
                    text_key(app, key, mods);
                }
            }
            EditorInput::Delete { forward, by, .. } => {
                let key = if forward { ToolKey::Delete } else { ToolKey::Backspace };
                text_key(app, key, Mods { cmd: !matches!(by, Granularity::Unit), ..Mods::default() });
            }
            EditorInput::Newline { shift, .. } => text_key(app, ToolKey::Enter, Mods { shift, ..Mods::default() }),
            EditorInput::Tab => text_key(app, ToolKey::Tab, Mods::default()),
            // The framework routes Escape to the focused part.
            EditorInput::Cancel => end_edit(app, ToolKey::Escape, Mods::default(), view),
            EditorInput::Edit(command) => match command {
                EditCommand::Copy => {
                    text_copy(app, false);
                }
                EditCommand::Cut => {
                    if text_copy(app, true) {
                        text_key(app, ToolKey::Delete, Mods::default());
                    }
                }
                EditCommand::Paste => text_paste(app, pasted, false),
                EditCommand::SelectAll => {
                    if let Err(e) = app.run("select.all", json!({})) {
                        app.status(e);
                    }
                }
                EditCommand::Delete => text_key(app, ToolKey::Delete, Mods::default()),
                // `EditorState::can_undo` stays false: undo and redo fall through to the app's commands.
                _ => {}
            },
            // The press that selects is the tool's (the canvas hosts it).
            _ => {}
        }
    }

    /// Runs one command of [`TypeEditorCommand`].
    fn run(&mut self, command: TypeEditorCommand, view: ViewInfo) {
        let app = &mut *self.app;
        if let Some((c, _)) = command.character() {
            let r = app.session.tool_typed_text(&c.to_string(), view);
            crate::canvas::apply_requests(app, r);
            return;
        }
        match command {
            TypeEditorCommand::Commit => end_edit(app, ToolKey::Enter, Mods { cmd: true, ..Mods::default() }, view),
            TypeEditorCommand::Bold | TypeEditorCommand::Italic => {
                let face = if command == TypeEditorCommand::Bold { Face::Bold } else { Face::Italic };
                if let Err(e) = toggle_face(app, face) {
                    app.status(e);
                }
            }
            _ => {}
        }
    }
}

/// Escape or Cmd+Enter, which end the edit: the tool's requests run, so it goes back to the tool used before.
fn end_edit(app: &mut VectorcraftApp, key: ToolKey, mods: Mods, view: ViewInfo) {
    let r = app.session.tool_key(key, mods, view);
    crate::canvas::apply_requests(app, r);
}

/// The tool key and modifiers the engine's key handler reads for a logical move: the command modifier is the tool's
/// word, paragraph and document reach, Shift extends. Page moves have no tool key.
fn move_key(step: NavMove, mods: MoveMods) -> Option<(ToolKey, Mods)> {
    let key = match step {
        NavMove::Back => ToolKey::Left,
        NavMove::Forward => ToolKey::Right,
        NavMove::Up => ToolKey::Up,
        NavMove::Down => ToolKey::Down,
        NavMove::Start => ToolKey::Home,
        NavMove::End => ToolKey::End,
        _ => return None,
    };
    let wide = !matches!(mods.by, Granularity::Unit | Granularity::Line);
    Some((key, Mods { shift: mods.extend, cmd: wide, ..Mods::default() }))
}

impl Component for TypeEditor<'_> {
    const ROLE: Role = Role::MULTILINE_TEXT_INPUT;
    const INPUT: craft::ui::contract::Input = craft::ui::contract::Input::Editor;
    const KEYS: KeyUse = KeyUse::CODE;
    type Style = TypeEditorStyle;
    type Output = TypeEditorOutput;

    fn style(_: &StyleCx<'_>, _: TypeEditorStyleKey) -> TypeEditorStyle {
        TypeEditorStyle { metrics: Metrics::new() }
    }

    fn sense(&self) -> Sense {
        Sense::FOCUS | Sense::TAKE_FOCUS
    }

    fn measure(&mut self, _cx: &MeasureCx<'_, Self>) -> Vec2 {
        self.caret.size()
    }

    fn parts(&mut self, rect: Rect, cx: &mut PartsCx<'_, Self>) {
        cx.editor(EDITOR, rect, EditorPurpose::Text);
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        let steps = cx.editor_steps::<TypeEditorCommand>();
        let pasted = match cx.clipboard().paste() {
            Pending::Ready(content) => content.text.as_deref().map(str::to_owned),
            _ => None,
        };
        let mut fired = Vec::new();
        // Windows' Alt-code entry takes every Alt+digit no focused component claims, top-row digits too; production
        // read codes from the keypad only, so the character commands claim their Alt+digit chords and run them here.
        let digit_chords: Vec<(craft::keymap::Chord, TypeEditorCommand)> = TypeEditorCommand::defs()
            .unwrap_or_default()
            .iter()
            .filter_map(|def| {
                let command = TypeEditorCommand::from_ref(&def.command).filter(|c| c.character().is_some())?;
                let chord = cx.resolve_command(def.command.clone())?.chord?;
                (chord.mods == craft::keymap::Mods::ALT && chord.key.is_digit()).then_some((chord, command))
            })
            .collect();
        let chords: Vec<_> = digit_chords.iter().map(|(chord, _)| *chord).collect();
        for &chord in &chords {
            cx.claim(chord);
        }
        while let Some(chord) = cx.take_first_claimed(&chords) {
            fired.extend(digit_chords.iter().find(|(c, _)| *c == chord).map(|(_, command)| *command));
        }
        // A new edit takes the keys ([`show`]); a click away loses them.
        if !cx.appeared() && !cx.is_focused() {
            keep_marked_text(self.app);
        }
        let view = self.app.view_info();
        // Inputs and commands in the order they came: once a command (Cmd+Enter) ended the edit, the frame's later
        // typing is not the edit's, as production read the frame's keys in order.
        for step in steps {
            match step {
                EditorStep::Input(input) if self.app.session.tool_wants_text() => self.apply(input, view, pasted.clone()),
                EditorStep::Command(command, mut fired) => match command.app_command() {
                    Some(id) => {
                        fired.command = CommandRef::new(id);
                        crate::shortcuts::run_fired(self.app, &self.ctx, &fired);
                    }
                    None => self.run(command, view),
                },
                _ => {}
            }
        }
        for command in fired {
            if self.app.session.tool_wants_text() {
                self.run(command, view);
            }
        }
    }

    fn paint(&mut self, _ui: &mut PaintUi<'_>, cx: &Cx<'_, Self>) {
        let mut state = EditorState::new(self.caret, &[], TextPos::default());
        state.composing = self.app.session.tool_composing();
        cx.editor_state(state);
    }

    fn finish(self, cx: Cx<'_, Self>) -> TypeEditorOutput {
        TypeEditorOutput { response: cx.response.clone(), unfocused_start: cx.appeared() && !cx.is_focused() }
    }
}

/// Shows the editor part over the Type tool's caret while the tool edits text, before the tool host. A new edit target
/// is a new IME owner, so a reloaded document interrupts its composition.
pub(crate) fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if !app.session.tool_wants_text() || app.viewport_hidden(ui.ctx()) {
        return;
    }
    let view = app.view_info();
    let Some((a, b)) = app.session.tool_ime_caret(view) else { return };
    let Some(target) = app.session.tool_ime_target(view) else { return };
    let Some(canvas_view) = app.canvas_view() else { return };
    let caret = Rect::from_two_pos(canvas_view.to_screen(a), canvas_view.to_screen(b)).expand2(egui::vec2(1.0, 0.0));
    let out =
        ui.scope_builder(egui::UiBuilder::new().max_rect(caret), |ui| TypeEditor::new(app, caret, ui.ctx().clone()).id_salt(target).show(ui)).inner;
    // Typing goes to the edit that starts, as a click on the part would take the keys.
    if out.unfocused_start {
        focus::request(ui.ctx(), FocusTarget::Id(out.response.id));
    }
}
