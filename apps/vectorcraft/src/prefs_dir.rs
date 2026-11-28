//! Where VectorCraft keeps its settings (`config.toml`, see `prefs.rs`), and next to it folders of the user's own
//! (Swatches, Graphic Styles, Fonts, Data Recovery, logs). The desktop app and `vectorcraft-cli`
//! (which shares this file) read the same Fonts folder.

use std::path::PathBuf;

// `prefs.rs` asks it for DrawCraft's folder, which only the app reads.
#[path = "profile.rs"]
pub(crate) mod profile;

/// The settings folder: `VectorCraft` in ~/Library/Application Support (macOS) or %APPDATA%
/// (Windows), or `vectorcraft` in $XDG_CONFIG_HOME or ~/.config (Linux and BSD), unless
/// `profile.rs` chooses another.
pub fn prefs_dir() -> Option<PathBuf> {
    profile::prefs_dir(false)
}

/// Have the font scans read VectorCraft's own Fonts folder, next to the settings, which
/// `text.addFontFiles` copies font files into. The folder is set whether or not
/// `VECTORCRAFT_NO_PREFS` is set, as the Swatches and Graphic Styles folders are.
pub fn install_fonts() {
    if let Some(dir) = prefs_dir().map(|dir| dir.join("Fonts")) {
        vectorcraft_text::set_app_font_dir(dir);
    }
}

/// The folder `name` next to the settings, for what the user keeps: none in a run without
/// preferences (`VECTORCRAFT_NO_PREFS`), which keeps nothing.
fn store_dir(name: &str) -> Option<PathBuf> {
    if std::env::var_os("VECTORCRAFT_NO_PREFS").is_some() {
        return None;
    }
    Some(prefs_dir()?.join(name))
}

/// Register the ICC profiles `color.loadProfile` kept in the `Color Profiles` folder (#2174), and
/// keep the ones loaded from now on there. Before anything names a profile.
pub fn attach_profile_store(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Color Profiles") {
        session.attach_profile_store(dir);
    }
}

/// Make Edit › Color Settings kept in `Color Settings.json` the active ones (#6335), and keep them
/// there when they change. After [`attach_profile_store`]: they may name a loaded profile.
pub fn attach_color_settings(session: &mut vectorcraft_engine::Session) {
    if let Some(path) = store_dir("Color Settings.json") {
        session.attach_color_settings(path);
    }
}

/// Install the plug-ins Install Plug-in kept in the `Plug-ins` folder (#2541), and keep the ones
/// installed from now on there. The key of the grants kept there is in the system's key store.
pub fn attach_plugin_store(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Plug-ins") {
        vectorcraft_engine::store::grant_keys::use_os_grant_keys();
        session.attach_plugin_store(dir);
    }
}

/// Have File › Scripts and `script.listInstalled` list the scripts of the `Scripts` folder. The
/// folder is only read: the user makes it.
pub fn attach_scripts_dir(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Scripts") {
        session.attach_scripts_dir(dir);
    }
}

/// Have `script.startup.run` run the scripts of the `Startup Scripts` folder. The folder is only
/// read: the user makes it.
pub fn attach_startup_scripts_dir(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Startup Scripts") {
        session.attach_startup_scripts_dir(dir);
    }
}

/// Keep File Info's metadata templates in the `Metadata Templates` folder, which a saved template
/// makes.
pub fn attach_metadata_templates_dir(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Metadata Templates") {
        session.attach_metadata_templates_dir(dir);
    }
}

/// Have File › New start each category's documents from its profile in the `New Document
/// Profiles` folder, which `file.newProfiles.edit` makes.
pub fn attach_new_doc_profiles(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("New Document Profiles") {
        session.attach_new_doc_profiles(dir);
    }
}

/// Have View › Proof Setup list the proof setups saved in the `Proof Setups` folder, which
/// `view.proofSetup.save` makes when it saves a setup by name.
pub fn attach_proof_setups(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Proof Setups") {
        session.attach_proof_setups(dir);
    }
}

/// Keep each script's `Folder.userData` in the `Script Data` folder, and the file access people
/// allow scripts with Always in `Script Grants.json` beside it. Both are made when a script first
/// needs them.
pub fn attach_script_data(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("Script Data") {
        session.attach_script_data(dir);
    }
}

/// Take the saved PDF presets from the `PDF Presets` folder (#2168), and keep the ones saved from
/// now on there. After the preferences were restored: the folder is the one place the presets live.
pub fn attach_pdf_preset_store(session: &mut vectorcraft_engine::Session) {
    if let Some(dir) = store_dir("PDF Presets") {
        session.attach_pdf_preset_store(dir);
    }
}

/// Keep the custom dictionary (Edit › Spelling › Add, Edit Custom Dictionary) in the
/// `Custom Dictionary.txt` file next to the settings, one word per line, and take in its words.
pub fn attach_custom_dictionary(session: &mut vectorcraft_engine::Session) {
    if let Some(path) = store_dir("Custom Dictionary.txt") {
        session.attach_custom_dictionary(path);
    }
}

/// Keep the custom arrowheads (Stroke panel menu › Edit Arrowheads…) in the `Arrowheads.vectorcraft`
/// file next to the settings, and list the heads it holds.
pub fn attach_arrowheads(session: &mut vectorcraft_engine::Session) {
    if let Some(path) = store_dir(vectorcraft_engine::cmd::arrowheads::FILE_NAME) {
        session.attach_arrowheads_file(path.to_string_lossy().to_string());
    }
}

/// Keep the custom bevels (`threeD.bevels.edit`) in the `Bevels.vectorcraft` file next to the
/// settings, and list the bevels it holds.
pub fn attach_bevels(session: &mut vectorcraft_engine::Session) {
    if let Some(path) = store_dir(vectorcraft_engine::cmd::bevels::FILE_NAME) {
        session.attach_bevels_file(path.to_string_lossy().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_scans_read_the_fonts_folder_next_to_the_settings() {
        // No test installs a launch's root in this process, so the settings folder stays the same here.
        assert_eq!(profile::data_root(), None, "a test installed a launch's root beside the others");
        install_fonts();
        let fonts = prefs_dir().map(|dir| dir.join("Fonts"));
        assert!(fonts.is_some(), "a home folder to keep the settings in");
        assert_eq!(vectorcraft_text::app_font_dir().map(std::path::Path::to_path_buf), fonts);
        assert!(vectorcraft_text::system_font_dirs().contains(&fonts.unwrap_or_default()));
    }
}
