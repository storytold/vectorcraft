//! The Type tool's text part: the framework's editor part (`Input::Editor`) over the engine tool's caret.
//!
//! The framework's IME engine owns the route, the composition guard and the reset; this component feeds what it
//! delivers (typing, marked text, deletions around the caret, moves, edits) to the engine tool one input at a time, in
//! arrival order, and reports whether the tool still shows marked text so the engine interrupts a composition the tool
//! ended. The caret itself is still drawn by the tool's overlay.

// PLAN: assumed `craft = { workspace = true }` in this crate's Cargo.toml (owner: hub-w1); `craft_ui` does not re-export
// the editor input types, so they come through the `craft::behavior` facade.
use craft::behavior::{EditCommand, EditorInput, Granularity, Metrics, MoveMods, NavMove, PartKey, Sense, SurroundingUnit as ImeUnit, TextPos};
use craft::ui::contract::{
    CommandSet, Commands as _, Common, Component, Cx, Focus as _, MeasureCx, NoValue, Outcome, PaintUi, PartsCx, Props as _, Remember as _, Role,
    Show as _,
};
use craft::ui::primitives::{EditorPurpose, EditorState};
use craft::ui::runtime::focus::{self, FocusTarget, KeyUse};
use craft::ui::runtime::pending::Pending;
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

/// The commands of a Type session: the commit chord, the formatting toggles and the characters Alt chords type on
/// Windows and Linux. They sit in a typing scope so they reach the editor before an app shortcut on the same chord and
/// the keymap lets the Alt chords through. Option types these characters natively on macOS, so they have no default
/// there.
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
}

// PLAN: `#[command(scope = ..)]` takes a string literal, so the scope is named here once and the set's own constant is
// the source.
/// The scope of [`TypeEditorCommand`].
pub(crate) const TYPE_EDITOR_SCOPE: craft::ui::runtime::commands::CommandScope = <TypeEditorCommand as CommandSet>::SCOPE;

impl TypeEditorCommand {
    /// The character the command types, `None` for the commit and the formatting toggles.
    fn character(self) -> Option<char> {
        Some(match self {
            Self::Bullet => '\u{2022}',
            Self::Copyright => '\u{00A9}',
            Self::Ellipsis => '\u{2026}',
            Self::Paragraph => '\u{00B6}',
            Self::Section => '\u{00A7}',
            Self::Trademark => '\u{2122}',
            Self::Registered => '\u{00AE}',
            Self::EnDash => '\u{2013}',
            Self::EmDash => '\u{2014}',
            Self::OpenDoubleQuote => '\u{201C}',
            Self::CloseDoubleQuote => '\u{201D}',
            Self::OpenSingleQuote => '\u{2018}',
            Self::CloseSingleQuote => '\u{2019}',
            Self::Commit | Self::Bold | Self::Italic => return None,
        })
    }
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
}

impl<'a> TypeEditor<'a> {
    /// A part for the tool's caret at `caret`, the instance of [`TypeEditorCommand`]'s scope while it has the keys.
    fn new(app: &'a mut VectorcraftApp, caret: Rect) -> Self {
        TypeEditor { common: Common::new(), app, caret }.label("Type").command_scope(TYPE_EDITOR_SCOPE)
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
        if let Some(c) = command.character() {
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
        let inputs: Vec<EditorInput> = cx.editor_input().collect();
        let pasted = match cx.clipboard().paste() {
            Pending::Ready(content) => content.text.as_deref().map(str::to_owned),
            _ => None,
        };
        let mut fired = Vec::new();
        while let Some((command, _)) = cx.take_command::<TypeEditorCommand>() {
            fired.push(command);
        }
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
        for input in inputs {
            self.apply(input, view, pasted.clone());
        }
        for command in fired {
            self.run(command, view);
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
    let out = ui.scope_builder(egui::UiBuilder::new().max_rect(caret), |ui| TypeEditor::new(app, caret).id_salt(target).show(ui)).inner;
    // Typing goes to the edit that starts, as a click on the part would take the keys.
    if out.unfocused_start {
        focus::request(ui.ctx(), FocusTarget::Id(out.response.id));
    }
}
