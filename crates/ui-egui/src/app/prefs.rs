use super::*;

impl VectorcraftApp {
    /// Was Preferences confirmed since the last call? The host then writes the preferences file,
    /// so what the user just set survives a session that doesn't end with a normal quit (#1747).
    pub fn take_prefs_changed(&mut self) -> bool {
        std::mem::take(&mut self.prefs_changed)
    }
}
