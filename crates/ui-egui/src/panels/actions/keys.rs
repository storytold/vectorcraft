use super::*;

/// The highest function key an action can play with: F1 to F15.
pub const FUNCTION_KEYS: u8 = 15;

/// A function key that plays an action: F1 to F15, alone or with Ctrl (Command on macOS), Shift,
/// or both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionKey {
    /// 1 to [`FUNCTION_KEYS`]: F1 to F15.
    pub key: u8,
    /// Held with Ctrl, Command on macOS.
    #[serde(default)]
    pub cmd: bool,
    #[serde(default)]
    pub shift: bool,
}

impl FunctionKey {
    /// The key as shortcut text (`Cmd+Shift+F5`).
    pub(super) fn shortcut(self) -> String {
        format!("{}{}F{}", if self.cmd { "Cmd+" } else { "" }, if self.shift { "Shift+" } else { "" }, self.key)
    }

    /// The chord the key is pressed as; none when `key` is not 1 to [`FUNCTION_KEYS`].
    pub fn chord(self) -> Option<craft_keymap::Chord> {
        (1..=FUNCTION_KEYS).contains(&self.key).then(|| crate::keymap::vc_chord(&self.shortcut())).flatten()
    }

    /// The key as the menus show shortcuts on this platform (`Ctrl+Shift+F5`, `⇧⌘F5`).
    pub fn text(self) -> String {
        crate::menus::pretty_shortcut(&self.shortcut())
    }

    /// The function key that params `p` give: `key` (1 to 15, F1 to F15; 0 or null for none), `cmd`
    /// and `shift`. `Ok(None)` for no key; an error for a `key` that is not one of them.
    pub fn from_params(p: &Value) -> Result<Option<Self>, String> {
        match p.get("key") {
            None | Some(Value::Null) => Ok(None),
            Some(v) => match v.as_u64() {
                Some(0) => Ok(None),
                Some(n) if n <= u64::from(FUNCTION_KEYS) => {
                    let flag = |k: &str| p.get(k).and_then(Value::as_bool).unwrap_or(false);
                    Ok(Some(Self { key: n as u8, cmd: flag("cmd"), shift: flag("shift") }))
                }
                _ => Err("no such function key".into()),
            },
        }
    }
}

/// The function returns an error message if the function key is invalid or reserved on this platform.
pub fn key_error(key: FunctionKey) -> Option<String> {
    let Some(chord) = key.chord() else { return Some("no such function key".into()) };
    if crate::keymap::platform() == craft_keymap::Platform::Windows && (key.key == 1 || key.cmd && !key.shift && matches!(key.key, 4 | 6)) {
        return Some(format!("{} cannot be used on Windows", key.text()));
    }
    if crate::keymap::template().is_reserved(&chord) {
        return Some(format!("{} is reserved by the system", key.text()));
    }
    None
}

pub(super) fn playable_chord(key: FunctionKey) -> Option<craft_keymap::Chord> {
    key_error(key).is_none().then(|| key.chord()).flatten()
}

crate::shortcut_editor::ui_mirror!(fn function_key_chords() -> Vec<craft_keymap::Chord>);

/// Mirrors the actions' function keys for code without app access (cheap when unchanged).
pub fn sync_function_keys(ui: &crate::UiState) {
    let chords: Vec<craft_keymap::Chord> = ui.action_sets.iter().flat_map(|s| s.actions.iter().filter_map(|a| playable_chord(a.key?))).collect();
    if function_key_chords().read().is_ok_and(|v| *v != chords)
        && let Ok(mut v) = function_key_chords().write()
    {
        *v = chords;
    }
}

/// Whether `chord` plays an action, by the function keys [`sync_function_keys`] last mirrored.
pub(crate) fn plays_an_action(chord: &craft_keymap::Chord) -> bool {
    let platform = crate::keymap::platform();
    let chord = chord.effective(platform);
    function_key_chords().read().is_ok_and(|v| v.iter().any(|c| c.effective(platform) == chord))
}

/// Plays each action whose function key was pressed this frame, in the order of the presses and
/// before any command with the same shortcut, and takes those key presses. The frame's other
/// events stay for the command, panel and tool shortcuts.
pub fn play_function_key(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let keys: Vec<(craft_keymap::Chord, (usize, usize))> = app
        .ui
        .action_sets
        .iter()
        .enumerate()
        .flat_map(|(si, s)| s.actions.iter().enumerate().filter_map(move |(ai, a)| Some((playable_chord(a.key?)?, (si, ai)))))
        .collect();
    if keys.is_empty() {
        return;
    }
    let platform = crate::keymap::platform();
    let hits: Vec<(usize, usize)> = ctx.input_mut(|i| {
        let mut hits = Vec::new();
        i.events.retain(|e| {
            let hit = match e {
                egui::Event::Key { key, pressed: true, modifiers, .. } => craft_keymap::Key::from_egui(*key)
                    .map(|k| craft_keymap::Chord::new(craft_keymap::Mods::from_egui(modifiers, platform), k))
                    .and_then(|c| keys.iter().find(|(k, _)| *k == c).map(|(_, at)| *at)),
                _ => None,
            };
            if !matches!(e, egui::Event::Key { repeat: true, .. }) {
                hits.extend(hit);
            }
            hit.is_none()
        });
        hits
    });
    for (set, action) in hits {
        if let Err(e) = play(app, set, action) {
            app.status(e);
        }
    }
}
