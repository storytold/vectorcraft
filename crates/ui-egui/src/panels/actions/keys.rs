use craft::automation::actions::ActionList;
use craft::keymap::{Chord, Platform};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model;

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
    pub fn chord(self) -> Option<Chord> {
        (1..=FUNCTION_KEYS).contains(&self.key).then(|| Chord::parse(&self.shortcut()).ok()).flatten()
    }

    /// The key as the menus show shortcuts on this platform (`Ctrl+Shift+F5`, `⇧⌘F5`).
    pub fn text(self) -> String {
        crate::keymap::chord_text(&self.shortcut())
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
    let platform = crate::keymap::platform();
    if platform == Platform::Windows && (key.key == 1 || key.cmd && !key.shift && matches!(key.key, 4 | 6)) {
        return Some(format!("{} cannot be used on Windows", key.text()));
    }
    if craft::keymap::reserved::system(platform).contains(&chord) {
        return Some(format!("{} is reserved by the system", key.text()));
    }
    None
}

pub(super) fn playable_chord(key: FunctionKey) -> Option<Chord> {
    key_error(key).is_none().then(|| key.chord()).flatten()
}

/// The actions of `list` that a function key plays, in list order: each action's name and the
/// chord its key is pressed as. The shortcut catalogue makes a row of each; a key that
/// [`key_error`] refuses on this platform gives none, so it never plays.
pub fn function_keys(list: &ActionList) -> Vec<(String, Chord)> {
    list.actions().iter().filter_map(|a| Some((a.name.clone(), playable_chord(model::function_key(a)?)?))).collect()
}
